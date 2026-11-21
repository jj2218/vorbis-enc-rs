#!/usr/bin/env bash
# Rank the Rust codec's functions by sampled CPU cycles: time, where `just profile` counts
# instructions. perf records the shipped bench binary (fat LTO) pinned to BENCH_CPU (a
# performance core on hybrid CPUs); each sample's inline chain (addr2line -i, debug info) is
# charged to its innermost codec function outside basic_op, so code inlined into the AVX2
# copies and Decoder::decode still ranks as its own function (operators count to their caller).
# CRATE= (the first codec present), BENCH=dec|enc (enc; all: both codecs × sides), BENCH_MODE=<mode>, TOP= (10),
# PERF_REPS= (3); C=1 (lc3): also liblc3's distribution build ranked, per stage against the Rust;
# FN=<name substring>: that function's cost per source line instead (inlined
# callees' lines included). Two builds: runtime AVX2 dispatch and portable (--cfg portable).
# The header gives the Rust codec's share of the samples (the C baseline runs in the same binary);
# a last line the shared libraries' share (libm, libc: setup's exp/atan, malloc), uncharged.
# Detail: perf report -i target/perf.data (the last run); perf annotate -i target/perf.data <sym>.
set -euo pipefail
cd "$(dirname "$0")/.."
# the first codec crate present (a product export has a subset)
first_codec() { for c in amr-nb amr-wb lc3 vorbis-enc; do [ -d crates/$c ] && { echo $c; return; }; done; }
if [ "${BENCH:-}" = all ]; then
    for c in amr-nb amr-wb lc3 vorbis-enc; do [ -d crates/$c ] || continue; for b in $([ $c = vorbis-enc ] || echo dec) enc; do CRATE=$c BENCH=$b bash "$0"; done; done
    exit
fi
crate=${CRATE:-$(first_codec)} side=${BENCH:-enc}
case $side in dec | enc) ;; *) echo "perf: BENCH=dec|enc|all" >&2; exit 1 ;; esac
# Debian's `perf` wrapper refuses a host kernel it has no package for; perf_<ver> works then.
perf=
for p in perf $(compgen -c perf_ | sort -uV); do
    "$p" stat -e task-clock true >/dev/null 2>&1 && { perf=$p; break; }
done
[ -n "$perf" ] || { echo "perf: no working perf (container: --cap-add=PERFMON, linux-perf; host: perf_event_paranoid <= 2)" >&2; exit 1; }
ev=cycles:u; [ -d /sys/bus/event_source/devices/cpu_core ] && ev=cpu_core/cycles/u; ev=${PERF_EV:-$ev}  # hybrid: P-core PMU
for build in ${PERF_BUILDS:-avx2 portable}; do
    flags=${RUSTFLAGS:-} dir=$PWD/target
    [ $build = portable ] && flags="$flags --cfg portable" dir=$PWD/target/portable  # as just test/bench
    exe=$(RUSTFLAGS=$flags CARGO_TARGET_DIR=$dir cargo bench -p "$crate" --bench codec --no-run 2>&1 |
        sed -n 's/.*Executable .*(\(.*\)).*/\1/p' | tail -1)
    case $exe in /*) ;; *) exe=$PWD/$exe ;; esac
    [ -x "$exe" ] || { echo "perf: could not build the $build benchmark" >&2; exit 1; }
    BENCH=$side BENCH_REPS=${PERF_REPS:-3} taskset -c "${BENCH_CPU:-2}" \
        "$perf" record -q -e "$ev" -F 4999 -o target/perf.data "$exe" >/dev/null 2>&1
    "$perf" script -i target/perf.data -F ip,sym,symoff,dso 2>/dev/null |
        python3 -I -c '
import collections, re, subprocess, sys
exe, mod, head, top, fn = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4]), sys.argv[5]
base = {}
for line in subprocess.run(["nm", "-C", "--defined-only", exe], capture_output=True, text=True).stdout.splitlines():
    a, _, name = line.split(" ", 2)
    base.setdefault(name, int(a, 16))
hits, samples, shared = collections.Counter(), 0, collections.Counter()
for line in sys.stdin:
    samples += 1
    m = re.match(r"\s*[0-9a-f]+ (.*)\+0x([0-9a-f]+) \((.*)\)$", line)
    if m and m.group(3) == exe and m.group(1) in base:
        hits[base[m.group(1)] + int(m.group(2), 16)] += 1
    elif m and m.group(3) != exe:
        shared[m.group(1)] += 1  # libm/libc: not charged to a Rust function (no call graph)
addrs = sorted(hits)
out = subprocess.run(["addr2line", "-a", "-i", "-f", "-C", "-e", exe], capture_output=True, text=True,
                     input="\n".join(f"{a:x}" for a in addrs)).stdout.splitlines()
cost, i = collections.Counter(), 0
while i < len(out):  # per address: "0x…", then (function, file:line) pairs, innermost first
    addr, i, fns = int(out[i], 16), i + 1, []
    while i < len(out) and not out[i].startswith("0x"):
        fns.append((re.sub(r"::\{closure#\d+\}", "", out[i]), out[i + 1])); i += 2
    own = [(f, loc) for f, loc in fns if f.startswith(mod + "::") and "::basic_op::" not in f]
    if own:
        f, loc = own[0]
        if fn and any(fn in g for g, _ in own):  # FN=: per source line (inlined callees too)
            loc = re.sub(r" \(discriminator \d+\)", "", loc)
            f = re.split(r"/crates/|/library/", loc)[-1]
        elif fn:
            continue
        cost[f] += hits[addr]
total = sum(cost.values())
what = f"in {fn}*, self cycles per source line" if fn else "in the Rust codec, self cycles"
print(f"{head}: {100 * total / max(samples, 1):.1f}% of {samples} samples {what}:")
for f, n in cost.most_common(top):
    print(f"{100 * n / max(total, 1):5.1f}%  {f}")
if not fn and shared:  # Rust and the C baseline both call them: a hint, not a ranking
    top3 = ", ".join(f"{f} {100 * n / max(samples, 1):.1f}%" for f, n in shared.most_common(3))
    print(f"  (+ {100 * sum(shared.values()) / max(samples, 1):.1f}% of samples in shared libraries, Rust and C callers: {top3})")
' "$exe" "${crate//-/_}" "$crate $side $build${BENCH_MODE:+ mode $BENCH_MODE}" "${TOP:-10}" "${FN:-}"
    # C=1 (lc3): the same ranking for liblc3's distribution build (its renamed copy, by address
    # range: the local copies of the oracle's global lc3_* symbols, and the copy's instance of
    # every static both copies have; LTO places some statics, e.g. its FFT, before the lc3_* ones).
    [ "${C:-}" = 1 ] && [ $build = avx2 ] || continue
    [ "$crate" = lc3 ] || { echo "perf: C=1 needs CRATE=lc3 (a renamed C copy in the bench)" >&2; exit 1; }
    read -r lo hi < <(nm "$exe" | awk '$2 == "T" && $3 ~ /^lc3_/ {g[$3] = 1; if ($1 > gmax) gmax = $1}
        $2 == "t" {n[$3]++; a[$3, n[$3]] = $1}
        END {lo = "z"; hi = ""
             for (k in n) if (k in g || n[k] == 2) {
                 x = (k in g) ? a[k, 1] : (a[k, 1] > a[k, 2] ? a[k, 1] : a[k, 2])
                 if (!(k in g) && x < gmax) continue  # the copy sits above the oracle
                 if (x < lo) lo = x; if (x > hi) hi = x }
             print lo, hi}')
    [ "$lo" != z ] || { echo "perf: no fast liblc3 copy in the bench (see liblc3-sys build warnings)" >&2; exit 1; }
    "$perf" report -i target/perf.data --no-children --sort sym -v 2>/dev/null | grep -v '^#' |
        python3 -I -c '
import sys
lo, hi, head, top = int(sys.argv[1], 16) - 0x1000, int(sys.argv[2], 16) + 0x3000, sys.argv[3], int(sys.argv[4])
rows = []
for l in sys.stdin:
    p = l.split()
    if len(p) >= 5 and lo <= int(p[1], 16) < hi:
        rows.append((float(p[0].rstrip("%")), " ".join(p[4:])))
t = sum(x for x, _ in rows)
print(f"{head}: {t:.1f}% of samples in liblc3 -O3 -ffast-math (C, self, inlined callees in their caller):")
for x, f in rows[:top]:
    print(f"{100 * x / max(t, 1e-9):5.1f}%  {f}")
' "$lo" "$hi" "$crate $side c${BENCH_MODE:+ mode $BENCH_MODE}" "${TOP:-10}"
done
