#!/usr/bin/env bash
# `just conform`: each codec crate present, its conformance tests in release, one summary line per
# run (the tests print them; pinned deviations are counted there, not failed). Exits non-zero if any
# test failed or printed no summary line; a failing run's panic lines are shown.
set -uo pipefail
cd "$(dirname "$0")/.."
rc=0
# run <crate> <grep pattern of its summary lines> <cargo test args...>
run() {
    local crate=$1 pat=$2 out
    shift 2
    [ -d "crates/$crate" ] || return 0
    out=$(cargo test -q --release -p "$crate" "$@" -- --nocapture 2>&1)
    local st=$?
    grep -E "$pat|panicked" <<<"$out" | sed "s/^\.*//"
    if [ $st -ne 0 ] || ! grep -qE "$pat" <<<"$out"; then
        echo "conform: $crate $* FAILED (exit $st)"
        rc=1
    fi
}
run amr-nb "bit-exact|failing|mismatch" --test conformance
run amr-wb "bit-exact" --test conformance
run amr-wb "bit-exact|bit sync" --test enc_conformance
WB_STRESS_STREAMS=200 run amr-wb "bit-exact|differs" --test vs_ref
run amr-wb "bit-exact|differs" --test enc_vs_ref
run lc3 "bit-exact|byte-identical" --test conformance
run vorbis-enc "byte-identical" --lib analysis::vs_c
exit $rc
