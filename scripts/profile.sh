#!/usr/bin/env bash
# Rank the Rust codec's functions by self cost (callgrind instructions, one bench rep;
# BENCH=dec or BENCH=enc profiles one side, BENCH_MODE=<mode> one mode, e.g. 102).
# Fat LTO inlines nearly everything into Decoder::decode, so the ranking build is a copy of
# the workspace with #[inline(never)] on every function of the crate (CRATE=, default the first codec present) (basic_op stays inlined, so
# operator cost lands on its caller). Instructions are a guide, not time: confirm with
# `just bench`. Line detail: callgrind_annotate --auto=yes target/callgrind.out
# Each line also counts the function's scalar `imul`s and vector multiplies (pmaddwd/pmul*):
# many scalar imuls in a function with sums of products usually means a loop that didn't
# vectorise (syn_filt, norm_corr, cor_h_x, ph_disp were found that way); `just asm` to look.
set -euo pipefail
cd "$(dirname "$0")/.."
# the first codec crate present (a product export has a subset)
first_codec() { for c in amr-nb amr-wb lc3 vorbis-enc; do [ -d crates/$c ] && { echo $c; return; }; done; }
crate=${CRATE:-$(first_codec)}  # CRATE=<codec>: another one present
mod=${crate//-/_}
src=target/profile-src
rm -rf "$src" && mkdir -p "$src"
tar --exclude=./target --exclude=./.git --exclude=./.cache -cf - . | tar -xf - -C "$src"
ln -s "$PWD/.cache" "$src/.cache"
find "$src/crates/$crate/src" -name '*.rs' | while read -r f; do
    case "$f" in *basic_op.rs | *tables.rs | *testutil.rs | *cnst.rs | *lib.rs) continue ;; esac
    perl -0pi -e 's/^((?:    )?(?:pub(?:\([a-z]+\))? )?fn )/#[inline(never)]\n$1/mg;
        s/(#\[inline[^\n]*\]\n)#\[inline\(never\)\]\n/$1/g' "$f"
done
exe=$(cd "$src" && CARGO_TARGET_DIR="$PWD/target/profile" cargo bench -p "$crate" --bench codec --no-run 2>&1 |
    sed -n 's/.*Executable .*(\(.*\)).*/\1/p' | tail -1)
exe=$src/$exe
[ -x "$exe" ] || { echo "profile: could not build the benchmark" >&2; exit 1; }
out=target/callgrind.out
objdump -d --no-show-raw-insn -C "$exe" > target/profile.asm
BENCH_REPS=1 valgrind --tool=callgrind --callgrind-out-file="$out" "$exe" >/dev/null 2>&1
callgrind_annotate --auto=no --threshold=99.9 "$out" 2>/dev/null | grep -E '^ *[0-9,]+ +\(' |
    MOD=$mod python3 -I -c '
import collections, os, re, sys
mod = os.environ["MOD"]
muls = collections.defaultdict(lambda: [0, 0])  # function -> [scalar imul, vector mul]
fn = None
for line in open(sys.argv[2]):
    m = re.match(r"^[0-9a-f]+ <(.*)>:$", line)
    if m:
        fn = m.group(1)
    elif fn and "\t" in line:  # address, tab, instruction (may be empty)
        ins = line.split("\t")[1].split()
        op = ins[0] if ins else ""
        if op.startswith("imul"):
            muls[fn][0] += 1
        elif re.match(r"v?pm(addwd|ul)", op):
            muls[fn][1] += 1
cost = collections.Counter()
for line in sys.stdin:
    n, rest = line.split("(", 1)
    fn = rest.split(")", 1)[1].split(" [")[0].strip()
    name = fn.split(":", 1)[-1]  # strip the source file
    if mod + "::" in name:  # Rust symbols only (the C baseline runs in the same binary)
        cost[name] += int(n.replace(",", ""))
total = sum(cost.values())
print(f"{mod} self cost by function ({total / 1e6:.0f}M instructions; scalar imul / vector mul):")
for fn, n in cost.most_common(int(sys.argv[1])):
    i, v = muls.get(fn, ("?", "?"))
    print(f"{100 * n / total:5.1f}%  {i:>4} / {v:<4} {fn}")
' "${TOP:-15}" target/profile.asm
