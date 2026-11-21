#!/usr/bin/env bash
# CRATE=<crate> (default the first codec present): which codec (lc3: vs liblc3's distribution and strict builds). Build its benchmark once and run it pinned to one core (less noise on laptops).
# BENCH=kern: ns per call of the hot encoder kernels, Rust vs C (an ignored lib test).
# AB=<git ref>: that ref (A) vs the working tree (B; B=<git ref>: that ref instead, e.g. two
# `git stash create` snapshots of consecutive steps), 3 runs per side in the order A B, B A, A B,
# of the "all" line (BENCH=enc by default; BENCH_MODE=<mode>: only that mode, e.g. 122), then
# `B/A` of the median Rust/C ratios (> 1: B faster; the C drifts with temperature, the ratio
# doesn't). BENCH_MODE=each: every mode in one run, then that per mode.
# AB=<ref> COUNT=1 (lc3): per mode, instructions and cycles of the Rust side (see below).
# AB=<ref> BENCH=kern: per kernel, min ns over 3 interleaved runs of each side, and B/A.
set -euo pipefail
cd "$(dirname "$0")/.."
# the first codec crate present (a product export has a subset)
first_codec() { for c in amr-nb amr-wb lc3 vorbis-enc; do [ -d crates/$c ] && { echo $c; return; }; done; }
crate=${CRATE:-$(first_codec)}  # CRATE=<codec>: another one present
# BENCH=all: every codec × side, its "all" line only: a sweep's before/after (run on each tree).
if [ "${BENCH:-}" = all ]; then
    [ -z "${AB:-}" ] || { echo "bench: BENCH=all doesn't take AB= (run it on each tree)" >&2; exit 1; }
    for c in amr-nb amr-wb lc3 vorbis-enc; do [ -d crates/$c ] || continue; for b in $([ $c = vorbis-enc ] || echo dec) enc; do
        printf '%-6s %s %s\n' "$c" "$b" "$(env -u BENCH_MODE CRATE=$c BENCH=$b bash "$0" | grep '^all ' || echo failed)"
    done; done
    exit
fi
# BENCH=stages: the encoder's and decoder's µs per frame per stage (`--cfg codec_stages` timers,
# mean of all reps, timer overhead included): time attribution per stage (`just perf`: per function).
if [ "${BENCH:-}" = stages ]; then
    grep -q codec_stages "crates/$crate/benches/codec.rs" \
        || { echo "bench: $crate has no stage timers (BENCH=stages: amr-nb, amr-wb)" >&2; exit 1; }
    export RUSTFLAGS="${RUSTFLAGS:-} --cfg codec_stages" CARGO_TARGET_DIR="$PWD/target/stages"
    unset BENCH
    exe=$(cargo bench -p "$crate" --bench codec --no-run 2>&1 | sed -n 's/.*Executable .*(\(.*\)).*/\1/p' | tail -1)
    taskset -c "${BENCH_CPU:-2}" "$exe" | grep "^stages"
    exit
fi
# BENCH_MODE=all is the default (every mode, the "all" line).
[ "${BENCH_MODE:-}" = all ] && unset BENCH_MODE
# The portable build gets its own target dirs (as in `just test`), so switching paths doesn't
# rebuild the fat-LTO binaries each time.
ab=target/ab
case "${RUSTFLAGS:-}" in *portable*)
    export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target/portable}"; ab=target/ab-portable ;; esac
build() { cargo bench -p "$crate" --bench codec --no-run 2>&1 | sed -n 's/.*Executable .*(\(.*\)).*/\1/p' | tail -1; }
kern() { taskset -c "${BENCH_CPU:-2}" cargo test -q --release -p "$crate" --lib kernel_timing \
    -- --ignored --nocapture 2>&1 | grep -E " ns$|error|panicked"; }
if [ -n "${AB:-}" ]; then
    # best of 3 reps per run: the A/B takes the median of 3 interleaved runs (15: ~50 min)
    export BENCH_REPS=${BENCH_REPS:-3}
    src=target/ab-src
    git worktree remove --force "$src" 2>/dev/null || rm -rf "$src"
    git worktree add -q --detach "$src" "$AB"
    trap 'git worktree remove --force "$src"' EXIT
    ln -s "$PWD/.cache" "$src/.cache"
    if [ -n "${B:-}" ]; then
        bsrc=target/ab-src-b
        git worktree remove --force "$bsrc" 2>/dev/null || rm -rf "$bsrc"
        git worktree add -q --detach "$bsrc" "$B"
        trap 'git worktree remove --force "$src"; git worktree remove --force "$bsrc"' EXIT
        ln -s "$PWD/.cache" "$bsrc/.cache"
    fi
    if [ "${BENCH:-}" = kern ]; then
        [ -z "${B:-}" ] || { echo "bench: B=<ref> is not supported with BENCH=kern (side B is the working tree)" >&2; exit 1; }
        for r in 1 2 3; do
            a=$(cd "$src" && CARGO_TARGET_DIR="$OLDPWD/$ab" kern) \
                || { echo "bench: $AB has no kernel_timing test (or it failed)" >&2; exit 1; }
            b=$(kern) || { echo "bench: kernel_timing failed in the working tree" >&2; exit 1; }
            sed 's/^/A /' <<<"$a"; sed 's/^/B /' <<<"$b"
        done | awk -v kf="${KERN:-}" '/error|panicked/ {print; bad=1; next}
            { side=$1; ns=$(NF-1); name=$0; sub(/^[AB] +/, "", name); sub(/ +[0-9.]+ ns$/, "", name)
              if (kf != "" && index(name, kf) == 0) next
              k=side SUBSEP name; if (!(k in m) || ns < m[k]) m[k]=ns
              if (!(name in seen)) { seen[name]=1; order[++n]=name } }
            END { if (bad) exit 1
              printf "%-22s %9s %9s  %5s\n", "kernel (min of 3)", "A ns", "B ns", "B/A"
              for (i=1; i<=n; i++) { a=m["A" SUBSEP order[i]]; b=m["B" SUBSEP order[i]]
                printf "%-22s %9.1f %9s  %5s\n", order[i], a, (b=="" ? "-" : sprintf("%.1f", b)),
                    (b=="" || a==0 ? "-" : sprintf("%.2f", b/a)) } }'
        exit
    fi
    # sides per run: A B, B A, A B (the first run is slow, a warm-up: not always A's)
    order() { [ "$1" = 2 ] && echo "B A" || echo "A B"; }
    # awk: the median of r[k, 1..n[k]]
    med='function med(k,  i, j, t, a) { for (i=1; i<=n[k]; i++) a[i]=r[k, i]
        for (i=1; i<=n[k]; i++) for (j=i+1; j<=n[k]; j++) if (a[j]<a[i]) { t=a[i]; a[i]=a[j]; a[j]=t }
        return a[int((n[k]+1)/2)] }'
    abs() { case "$1" in /*) echo "$1" ;; *) echo "$2/$1" ;; esac; }
    if [ -n "${B:-}" ]; then
        b=$(abs "$(cd "$bsrc" && CARGO_TARGET_DIR="$OLDPWD/$ab-b" build)" "$PWD/$bsrc")
    else
        b=$(abs "$(build)" "$PWD")
    fi
    a=$(abs "$(cd "$src" && CARGO_TARGET_DIR="$OLDPWD/$ab" build)" "$PWD/$src")
    # COUNT=1 (lc3; both trees with BENCH_IMPL): per mode (default the two points; BENCH_GRID=1:
    # the 20-mode grid; MODES="8k2.5 48k10": those), the Rust side's user instructions and cycles
    # (perf stat, P-core) minus the setup's (BENCH_IMPL=none), min of 3 interleaved runs, and B/A
    # (< 1: B cheaper). Load-proof; noise ±1-2%.
    if [ -n "${COUNT:-}" ]; then
        ev=instructions:u,cycles:u
        [ -d /sys/bus/event_source/devices/cpu_core ] && ev=cpu_core/instructions/u,cpu_core/cycles/u
        grid=$(for d in 2.5 5 7.5 10; do for r in 8 16 24 32 48; do printf '%sk%s ' $r $d; done; done)
        modes=${MODES:-$([ -n "${BENCH_GRID:-}" ] && echo "$grid" || echo "16k7.5 48k10")}
        export BENCH_GRID=1  # every mode name resolves (the two default points are in the grid)
        one() { BENCH_MODE=$2 BENCH_IMPL=$3 taskset -c "${BENCH_CPU:-2}" perf stat -x, -e "$ev" "$1" \
            2>&1 >/dev/null | awk -F, '{v[NR] = $1} END {print v[1], v[2]}'; }
        for m in $modes; do for _ in 1 2 3; do
            echo "$m N $(one "$a" "$m" none)"; echo "$m A $(one "$a" "$m" rust)"; echo "$m B $(one "$b" "$m" rust)"
        done; done | awk '{k = $1 " " $2; if (!(k in i) || $3 < i[k]) i[k] = $3; if (!(k in c) || $4 < c[k]) c[k] = $4
                if (!($1 in s)) {s[$1] = 1; o[++n] = $1}}
            END {printf "%-7s %8s %8s %6s | %8s %8s %6s\n", "mode", "A Minst", "B Minst", "B/A", "A Mcyc", "B Mcyc", "B/A"
                for (j = 1; j <= n; j++) {m = o[j]; ia = i[m " A"] - i[m " N"]; ib = i[m " B"] - i[m " N"]
                    ca = c[m " A"] - c[m " N"]; cb = c[m " B"] - c[m " N"]
                    printf "%-7s %8.1f %8.1f %6.3f | %8.1f %8.1f %6.3f\n", m, ia / 1e6, ib / 1e6, ib / ia, ca / 1e6, cb / 1e6, cb / ca}}'
        exit
    fi
    [ -f "$a" ] && [ -x "$a" ] && [ -f "$b" ] || { echo "bench: AB build failed (does ${B:+$B or }$AB have the codec bench?)" >&2; exit 1; }
    if [ "${BENCH_MODE:-}" = each ]; then
        for r in 1 2 3; do for v in $(order $r); do
            exe=$a; [ $v = B ] && exe=$b
            BENCH=${BENCH:-enc} env -u BENCH_MODE taskset -c "${BENCH_CPU:-2}" "$exe" |
                awk -v v=$v '$4 ~ /x$/ {print v, $1, $2/$3}'
        done; done | awk "$med"'{ k=$1 SUBSEP $2; n[k]++; r[k, n[k]]=$3; if (!($2 in seen)) { seen[$2]=1; o[++m]=$2 } }
            END { printf "%-5s %7s %7s  %6s\n", "mode", "A x", "B x", "B/A"
                for (i=1; i<=m; i++) { ra=med("A" SUBSEP o[i]); rb=med("B" SUBSEP o[i])
                    printf "%-5s %6.2fx %6.2fx  %6.3f\n", o[i], 1/ra, 1/rb, ra/rb } }'
        exit
    fi
    for r in 1 2 3; do for v in $(order $r); do
        exe=$a; [ $v = B ] && exe=$b
        BENCH=${BENCH:-enc} taskset -c "${BENCH_CPU:-2}" "$exe" | { grep -E "^${BENCH_MODE:-all} " || { echo "bench: no '${BENCH_MODE:-all}' line" >&2; exit 1; }; } |
            awk -v v=$v '{printf "%s rust %s  c %s  %s\n", v, $2, $3, $4}'
    done; done | awk "$med"'{ print; k=$1; n[k]++; r[k, n[k]] = $3 / $5 }
        END { if (n["A"] && n["B"]) printf "B/A %.3f (median Rust/C time per side)\n", med("A")/med("B") }'
    exit
fi
if [ "${BENCH:-}" = kern ]; then
    kern
    exit
fi
exe=$(build)
[ -x "$exe" ] || { echo "bench: could not locate the benchmark binary" >&2; exit 1; }
taskset -c "${BENCH_CPU:-2}" "$exe"
