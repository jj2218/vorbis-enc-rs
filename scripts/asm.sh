#!/usr/bin/env bash
# Disassemble one function of the release lib-test build: instruction count, the most frequent
# instructions, then the listing in target/asm.s. Use it when a kernel is slower than its vector
# work suggests (index math, spills, unaligned reloads, SLP across the wrong loop showed up this
# way). Usage: scripts/asm.sh <symbol regex>, e.g. 'cor_h_tm.*avx2_copy' or 'simd::cor_h_tm>'.
# Portable path: RUSTFLAGS="--cfg portable". CRATE=<crate>: another codec (default amr-nb).
set -euo pipefail
cd "$(dirname "$0")/.."
# the first codec crate present (a product export has a subset)
first_codec() { for c in amr-nb amr-wb lc3 vorbis-enc; do [ -d crates/$c ] && { echo $c; return; }; done; }
crate=${CRATE:-$(first_codec)}  # CRATE=<codec>: another one present
case "${RUSTFLAGS:-}" in *portable*)
    export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target/portable}" ;; esac
[ $# -eq 1 ] || { echo "usage: just asm '<symbol regex>'" >&2; exit 1; }
exe=$(cargo test -q --release -p "$crate" --lib --no-run --message-format=json 2>/dev/null |
    sed -n 's/.*"executable":"\([^"]*\)".*/\1/p' | tail -1)
[ -x "$exe" ] || { echo "asm: could not build the lib tests" >&2; exit 1; }
out=target/asm.s
objdump -d --no-show-raw-insn -C "$exe" |
    RE=$1 awk '/^[0-9a-f]+ <.*>:$/ { p = ($0 ~ ENVIRON["RE"]); if (p) n++ } p && n == 1' > "$out"
[ -s "$out" ] || { echo "asm: no symbol matches '$1'" >&2; exit 1; }
head -1 "$out"
echo "$(grep -c $'^ *[0-9a-f]*:\t' "$out") instructions, $(grep -c '(%rsp' "$out") stack refs; most frequent:"
grep -oP '^\s*[0-9a-f]+:\t\K\S+' "$out" | sort | uniq -c | sort -rn | head -12 | tr -s ' ' | paste -sd' '
echo "listing: $out"
