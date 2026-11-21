#!/usr/bin/env bash
# Coverage-guided fuzzing with cargo-fuzz (libFuzzer) on a pinned nightly; the only nightly
# use in the repo (`just check` and CI stay on stable). Targets: fuzz/<product>/fuzz_targets/*.rs
# (one cargo-fuzz crate per product, each its own workspace; target names are unique across them).
# Usage: fuzz.sh [target|all] [seconds per target]. Corpus and crash inputs persist in the
# gitignored .cache/fuzz/{corpus,artifacts}/<target>/. Prints one line per target.
set -euo pipefail
cd "$(dirname "$0")/.."

NIGHTLY=nightly-2026-11-01
CARGO_FUZZ=0.13.2
want=${1:-all}
secs=${2:-60}

rustup toolchain list | grep -q "^$NIGHTLY" || rustup toolchain install "$NIGHTLY" --profile minimal
if [ "$(cargo fuzz --version 2>/dev/null)" != "cargo-fuzz $CARGO_FUZZ" ]; then
  cargo install cargo-fuzz --locked --version "$CARGO_FUZZ"
fi

if [ "$want" = all ]; then
  targets=$(basename -s .rs fuzz/*/fuzz_targets/*.rs)
else
  targets=$want
fi
dir_of() { local f; f=$(ls fuzz/*/fuzz_targets/"$1".rs 2>/dev/null | head -1)
  [ -n "$f" ] || { echo "fuzz: no target $1 (fuzz/*/fuzz_targets/)" >&2; exit 1; }; dirname "$(dirname "$f")"; }
for d in $(for t in $targets; do dir_of "$t"; done | sort -u); do
  cargo +"$NIGHTLY" fuzz build --fuzz-dir "$d" 2>&1 | grep -E "^(error|warning)" || true
done

status=0
for t in $targets; do
  dir=$(dir_of "$t")
  corpus=.cache/fuzz/corpus/$t art=.cache/fuzz/artifacts/$t log=.cache/fuzz/$t.log
  mkdir -p "$corpus" "$art"
  # Known leaks of a C oracle: fuzz/<product>/lsan.supp, one `leak:<function>` per defect
  # (matched by name, so the stack needs a symbolizer).
  if [ -f "$dir/lsan.supp" ]; then
    export LSAN_OPTIONS=suppressions=$PWD/$dir/lsan.supp
    sym=$(command -v llvm-symbolizer || ls /usr/lib/llvm-*/bin/llvm-symbolizer 2>/dev/null | tail -1 || true)
    [ -z "$sym" ] || export ASAN_SYMBOLIZER_PATH=$sym
  else
    unset LSAN_OPTIONS
  fi
  if cargo +"$NIGHTLY" fuzz run --fuzz-dir "$dir" "$t" "$corpus" -- \
      -max_total_time="$secs" -artifact_prefix="$art/" -print_final_stats=1 >"$log" 2>&1; then
    runs=$(grep -oP 'stat::number_of_executed_units:\s*\K\d+' "$log" || echo "?")
    cov=$(grep -oP '#\d+\s+DONE\s+cov: \K\d+' "$log" || grep -oP 'cov: \K\d+' "$log" | tail -1)
    echo "fuzz $t: ok, $runs runs in ${secs}s, cov $cov, corpus $(ls "$corpus" | wc -l)"
  else
    status=1
    echo "fuzz $t: FAILED (log $log)"
    grep -m1 -E "panicked at|ERROR: libFuzzer|SUMMARY" "$log" || true
    grep -A1 -m1 "panicked at" "$log" | tail -1 || true
    grep -oE "$art/[a-z]+-[0-9a-f]+" "$log" | tail -1 | sed 's|^|  reproduce: cargo +'"$NIGHTLY"' fuzz run --fuzz-dir '"$dir"' '"$t"' |' || true
  fi
done
exit $status
