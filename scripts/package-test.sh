#!/usr/bin/env bash
# Each named crate as crates.io serves it: `cargo package`, then `cargo test` inside the unpacked
# `.crate` (offline; no workspace, no dev-only path crates, no downloads): what a distribution's
# packager runs. The C-oracle unit tests in src/ are behind `cfg(oracle)` (scripts/test.sh sets it),
# so they compile out here. Usage: package-test.sh <crate>... One line per crate.
set -euo pipefail
cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
for c in "$@"; do
    cargo package -q -p "$c" --allow-dirty --locked
    v=$(cargo metadata --format-version 1 --no-deps | python3 -I -c \
        "import json,sys; print(next(p['version'] for p in json.load(sys.stdin)['packages'] if p['name'] == sys.argv[1]))" "$c")
    tar -xzf "target/package/$c-$v.crate" -C "$tmp"
    if out=$(cd "$tmp/$c-$v" && CARGO_TARGET_DIR="$OLDPWD/target/package-test" cargo test -q --offline 2>&1); then
        echo "package-test: $c $v ok ($(grep -o '[0-9]* passed' <<<"$out" | awk '{s += $1} END {print s+0}') passed from the .crate)"
    else
        echo "package-test: $c $v FAILED from the .crate:"
        echo "$out" | tail -20
        exit 1
    fi
done
