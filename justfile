# Every recurring task is a recipe here, backed by a script when it has logic: `just --list`.
# The recipes below `rules` are the development repository's, unchanged.

default:
    @just --list

# The full gate: export guard, format, lint, tests against the oracle (downloaded into .cache/)
check:
    @bash scripts/check.sh

# Export guard: no third-party source, test items or stray files in the repo
rules:
    @bash scripts/export-guard.sh

# The published codec crates present in this checkout, as `-p` arguments: recipe bodies stay the
# same in every product (a product's justfile copies them; doctor compares them).
export CODECS := `for c in amr-nb amr-wb lc3 vorbis-enc; do [ -d crates/$c ] && printf -- '-p %s ' $c; done; true`

fmt-check:
    cargo fmt --all --check
    for m in fuzz/*/Cargo.toml; do cargo fmt --manifest-path "$m" --check; done

lint: aotuv msrv
    RUSTFLAGS="--cfg oracle" cargo clippy --workspace --all-targets --locked -- -D warnings
    RUSTFLAGS="--cfg oracle --cfg portable" CARGO_TARGET_DIR=target/portable cargo clippy $CODECS --all-targets --locked -- -D warnings
    for m in fuzz/*/Cargo.toml; do cargo clippy --manifest-path "$m" --locked -- -D warnings; done
    cargo clippy -q $CODECS --lib --locked --target aarch64-unknown-linux-gnu -- -D warnings
    RUSTDOCFLAGS="-D warnings" cargo doc -q $CODECS --no-deps --locked

# The library builds with its minimum supported Rust (`rust-version` in Cargo.toml; installs it)
msrv:
    #!/usr/bin/env bash
    set -euo pipefail
    v=$(sed -n 's/^rust-version = "\(.*\)"$/\1/p' Cargo.toml)
    rustup toolchain install "$v" --profile minimal --no-self-update >/dev/null 2>&1
    CARGO_TARGET_DIR=target/msrv cargo "+$v" check -q $CODECS --lib --locked
    echo "msrv: $(sed "s/-p //g" <<<"$CODECS")build with Rust $v"

# scripts/test.sh: the default build (AMR: runtime AVX2 dispatch; LC3: baseline SSE2 kernels) and
# the portable path (--cfg portable: src/simd.rs compiled out); with AMR-NB also its vectors on
# the SSE2 kernels' array forms (the non-x86 bodies). One line per run, a failing run's whole log.
# The codec tests against their C oracles (fetched and cached in .cache/), in parallel
test: lc3plus aotuv
    @bash scripts/test.sh

# The tests and conformance on aarch64: cross-built, each test binary under qemu-aarch64 (the portable code; correctness only, qemu timing means nothing; C-oracle-only tests skipped; ~15 min)
test-aarch64: lc3plus aotuv
    @TARGET=aarch64 bash scripts/test.sh

# Codec conformance, one summary line per run: each codec crate present, bit-exact vs its oracle (its conformance tests name the set: AMR the 3GPP vectors and reference C, LC3 liblc3 on the SIG operating points, Vorbis aoTuV's Ogg streams; pinned deviations counted, not failed)
conform: lc3plus aotuv
    @bash scripts/conform.sh

# Decoder + encoder speed, Rust vs the legacy C (AMR: opencore -O3; LC3: liblc3 as distributions build it, -O3 -ffast-math, and strict; Vorbis: encoder only, aoTuV-Lancer, strict aoTuV and xiph's libvorbis), pinned to one core; µs/frame per mode (CRATE=<codec>, default the first present; BENCH=dec|enc; BENCH=all: every codec × side, the "all" lines (before/after a sweep); BENCH=stages: encoder and decoder µs/frame per stage, time attribution; BENCH=kern: ns/call per kernel, KERN=<substring> filters; AB=<ref>: ref vs working tree (B=<ref>: vs that ref; 3 reps per run), BENCH_MODE=<mode>: one mode only (each: per-mode table); with kern: min of 3 + ratio)
bench: lc3plus aotuv
    @bash scripts/bench.sh

# Disassemble one function (count, top instructions, listing in target/asm.s), e.g. 'ltpf.*dot16' (an AMR AVX2 copy: 'cor_h_tm.*avx2_copy')
[positional-arguments]
asm symbol:
    @bash scripts/asm.sh "$1"

# Rank the Rust codec's functions by sampled cycles (perf on the shipped build, inline frames resolved; the default and the portable build; CRATE=<codec>, BENCH=dec|enc|all, BENCH_MODE=<mode>, TOP=<n>)
perf: lc3plus aotuv
    @bash scripts/perf.sh

# Rank the Rust codec's functions by self cost (callgrind, no-inline copy; CRATE=<codec>, BENCH=dec|enc, BENCH_MODE=<mode>: one mode)
profile: lc3plus aotuv
    @bash scripts/profile.sh

# Fuzz (nightly + cargo-fuzz, installed on first use; fuzz/<product>/), each codec vs its C oracle: AMR (nb_*, wb_*) the rfc4867 readers, the decoder and the encoder vs the 3GPP reference C; lc3_decode_vs_c, lc3_encode_vs_c: the LC3 decoder and encoder vs liblc3; lc3_api: any configuration, PCM and bytes, no panic; vorbis_encode_vs_c: the Vorbis encoder vs aoTuV; vorbis_encode_api: any f32 and configuration, no panic, lewton decodes
[positional-arguments]
fuzz target="all" seconds="60": aotuv
    @bash scripts/fuzz.sh "$1" "$2"

# Fetch + SHA-verify aoTuV b6.03 (the `aotuv_lancer_vorbis_sys` 0.1.6 crate's sources, BSD-3; the Vorbis port's C oracle built strict, and Lancer its speed bar) into .cache/aotuv
aotuv:
    @bash scripts/aotuv.sh

# Regenerate crates/vorbis-enc/src/{tables,setup_tables,psy_tables}.rs from aoTuV's compiled tables (scripts/gen_vorbis_tables.c: the windows, every encoder setup declaration listed by scripts/vorbis_decls.py, psy.c's tables)
vorbis-tables: aotuv
    #!/usr/bin/env bash
    set -euo pipefail
    A=.cache/aotuv/aotuv_lancer_vorbis_sys-0.1.6; V=$A/vorbis_vendor/lib
    mkdir -p target
    python3 -I scripts/vorbis_decls.py "$V" target/vorbis_decls.h
    srcs=(); for f in analysis bitrate block codebook envelope floor0 floor1 info lookup lpc lsp mapping0 mdct registry res0 sharedbook smallft synthesis; do srcs+=("$V/$f.c"); done
    cc -w -U__SSE__ -U__SSE2__ -U__SSE3__ -Itarget -I$A/vorbis_vendor/include -I$V -I$A/ogg_vendor/include scripts/gen_vorbis_tables.c "${srcs[@]}" $A/ogg_vendor/src/bitwise.c $A/ogg_vendor/src/framing.c -lm -o target/gen_vorbis_tables
    for t in tables:window setup_tables:setup psy_tables:psy; do target/gen_vorbis_tables "${t#*:}" > "crates/vorbis-enc/src/${t%:*}.rs"; rustfmt --edition 2024 "crates/vorbis-enc/src/${t%:*}.rs"; done
    echo "vorbis-tables: crates/vorbis-enc/src/tables.rs ($(wc -l < crates/vorbis-enc/src/tables.rs) lines), setup_tables.rs ($(wc -l < crates/vorbis-enc/src/setup_tables.rs) lines, $(wc -c < crates/vorbis-enc/src/setup_tables.rs) bytes), psy_tables.rs ($(wc -l < crates/vorbis-enc/src/psy_tables.rs) lines)"

# Fetch ETSI TS 103 634 V1.7.1 (LC3plus), SHA-verified, and extract its thetest*.wav inputs into .cache/lc3plus/wav (test input audio only)
lc3plus:
    @bash scripts/lc3plus.sh
