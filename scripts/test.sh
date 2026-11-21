#!/usr/bin/env bash
# `just test`: the codec tests in three configurations: runtime AVX2 dispatch (the whole
# workspace), the portable path (src/simd.rs compiled out), and the vectors with the SSE2
# kernels' array forms. Each configuration builds once (`--no-run`, the three builds in
# parallel, separate target dirs); then every test binary of every configuration runs at once,
# from its package directory as cargo runs it, plus the doc tests. `cargo test` runs a
# configuration's binaries one after another (277 + 260 + 27 s serially; the longest single
# test, the reference encoder on the WB vectors, is serial by design). One line per binary run;
# a failing run's whole log. The C-oracle tests (C_ONLY) run once per change of their inputs.
# TARGET=aarch64 (`just test-aarch64`): one cross-built configuration (the portable code, as on
# any non-x86 CPU; optimised, overflow checks and debug assertions on), every binary under
# qemu-aarch64. Correctness only: qemu's timing means nothing. No doc tests, no C_ONLY tests.
set -uo pipefail
cd "$(dirname "$0")/.."
logs=$(mktemp -d)
trap 'rm -rf "$logs"' EXIT

# every codec crate present (a product export has a subset), not the *-sys oracles
pkgs=$(for c in crates/*/; do c=${c%/}; c=${c#crates/}; [[ $c == *-sys ]] || printf -- '-p %s ' "$c"; done)
# --cfg portable only changes crates with SIMD kernels (src/simd.rs): the rest would run twice
simd=$(for c in crates/*/src/simd.rs; do [ -e "$c" ] || continue; c=${c#crates/}; printf -- '-p %s ' "${c%%/*}"; done)
# --cfg oracle: the C-oracle unit tests in src/ (out of a published `.crate`: package-test.sh)
configs=("dispatch|--cfg oracle|target|--workspace")
[ -z "$simd" ] || configs+=("portable|--cfg oracle --cfg portable|target/portable|$simd")
[ ! -d crates/amr-nb ] ||
    configs+=("arrays|--cfg oracle --cfg portable --cfg amr_nb_arrays|target/arrays|--release -p amr-nb --test conformance")
runner=()
if [ "${TARGET:-}" = aarch64 ]; then
    t=aarch64-unknown-linux-gnu
    rustup target list --installed | grep -qx $t || rustup target add $t
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
    export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
    export CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
    export CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS=true
    configs=("aarch64|--cfg oracle|target/aarch64|--release --target $t $pkgs")
    runner=(qemu-aarch64 -L /usr/aarch64-linux-gnu)
fi
# build: every config's test binaries as "<config> <package dir> <executable>" lines
build() {
    local name=$1 flags=$2 dir=$3 args=$4
    # shellcheck disable=SC2086 # args are words
    RUSTFLAGS="$flags" CARGO_TARGET_DIR="$dir" cargo test --locked --no-run \
        --message-format=json $args 2>"$logs/build-$name" | grep '"test":true' |
        sed -n 's/.*"manifest_path":"\([^"]*\)\/Cargo.toml".*"executable":"\([^"]*\)".*/\1 \2/p' |
        sed "s|^|$name |" >"$logs/list-$name"
    [ "${PIPESTATUS[0]}" -eq 0 ]
}
# Tests of the C oracles and the vectors alone: their results can't depend on the Rust
# configuration, so only the dispatch run has them.
C_ONLY=(c_oracle_with_homing_is_bit_exact c_encoder_oracle_with_homing_is_bit_exact
    c_encoder_oracle_sync c_decoders_on_vectors reference_encoder_on_vectors
    vo_amrwbenc_on_vectors no_vector_has_no_data_mid_speech spike_hashes)
# Their result depends only on the inputs below (C sources and vectors are SHA-pinned by the
# fetch scripts; the C build: the compiler's version, the cc crate's version, every CC/CFLAGS
# environment variable): a passing dispatch run stamps the inputs' hash, and later runs skip
# them while it is unchanged (the TS 26.173 reference encoder alone is ~125 s of every check).
c_key=$({
    find crates/*-sys crates/*/tests $(ls scripts/{fetch,vectors,opencore,liblc3,aotuv}.sh scripts/strict_c.rs 2>/dev/null) \
        -type f | sort | xargs sha256sum
    ${CC:-cc} --version 2>&1
    grep -A1 '^name = "cc"$' Cargo.lock
    env | grep -E '^(HOST_|TARGET_)?(CC|CFLAGS|CRATE_CC_NO_DEFAULTS)[_=]' | sort
} | sha256sum | cut -c1-32)
c_stamp=.cache/checked-c/$c_key
c_cached=false
[ -e "$c_stamp" ] && c_cached=true
skips() {
    [ "$1" = dispatch ] && ! $c_cached && return
    printf -- '--exact\n'
    printf -- '--skip\n%s\n' "${C_ONLY[@]}"
}
rc=0
pids=()
for c in "${configs[@]}"; do
    IFS='|' read -r name flags dir args <<<"$c"
    build "$name" "$flags" "$dir" "$args" &
    pids+=($!)
done
for i in "${!pids[@]}"; do
    if ! wait "${pids[$i]}"; then
        name=${configs[$i]%%|*}
        echo "test: $name build FAILED:"
        cat "$logs/build-$name"
        rc=1
    fi
done
[ $rc -eq 0 ] || exit 1

# run: one test binary (or the doc tests) into its log; a summary line, the log on failure
run() {
    local label=$1 log=$2 start=$SECONDS
    shift 2
    if "$@" >"$log" 2>&1; then
        echo "test: $label ok ($(grep -o '[0-9]* passed' "$log" | awk '{s += $1} END {print s+0}') passed, $((SECONDS - start))s)"
    else
        echo "test: $label FAILED ($((SECONDS - start))s):"
        cat "$log"
        return 1
    fi
}
pids=()
n=0
while read -r name pkg exe; do
    n=$((n + 1))
    label="$name $(basename "$pkg") $(basename "$exe" | sed 's/-[0-9a-f]*$//')"
    mapfile -t skip < <(skips "$name")
    (cd "$pkg" && run "$label" "$logs/run-$n" ${runner[@]+"${runner[@]}"} "$exe" -q ${skip[@]+"${skip[@]}"}) &
    pids+=($!)
done < <(cat "$logs"/list-*)
if [ ${#runner[@]} -gt 0 ]; then
    for p in "${pids[@]}"; do wait "$p" || rc=1; done
    exit $rc
fi
RUSTFLAGS="--cfg oracle" run "dispatch doc tests" "$logs/doc" cargo test --locked --workspace --doc -q &
pids+=($!)
for p in "${pids[@]}"; do wait "$p" || rc=1; done
if $c_cached; then
    echo "test: C-oracle harness tests skipped (their inputs passed before: $c_stamp)"
elif [ $rc -eq 0 ]; then
    mkdir -p .cache/checked-c && touch "$c_stamp"
fi
exit $rc
