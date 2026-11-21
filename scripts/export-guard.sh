#!/usr/bin/env bash
# Export guard: `just rules`, CI, and every release before its commit. This repository is
# exported from a private development repository; the guard fails on anything that must never
# be in it: development-only files, 3GPP or opencore source, test vectors or archives,
# references to files that aren't here, and `just` recipes the docs mention but don't exist.
# Checks tracked and untracked (not ignored) files, so it runs before the first commit too.
set -euo pipefail
cd "$(dirname "$0")/.."
self=scripts/export-guard.sh
files=$(git ls-files -co --exclude-standard)
# Text to scan: everything but this script and the licence/attribution files.
text=$(grep -vE "^($self|(.*/)?(LICENSE|NOTICE))$" <<<"$files" || true)
scan() { tr '\n' '\0' <<<"$text" | xargs -0 grep -nE "$1" -- 2>/dev/null | head -5 || true; }

out=$(
  # Development-only paths and downloads.
  grep -E '^(CLAUDE\.md|\.claude/|\.devcontainer/|docs/(FLAGS|ideas|structure|porting)\.md|docs/spikes/|spikes/|ship/|\.cache/|fuzz/([^/]+/)?(artifacts|corpus)/|scripts/(doctor|ship|commit|pr)\.sh)' <<<"$files" \
    | sed 's/^/development-only path: /' || true
  # Vectors, archives, C++ (opencore's sources are .cpp).
  grep -iE '\.(inp|cod|out|zip|tar|gz|tgz|xz|cpp|cc|wav|pdf)$' <<<"$files" | sed 's/^/vector, audio item, archive, spec or C++ file: /' || true
  # C files: only our own FFI shims and tools, each saying so on its first line.
  for f in $(grep -E '\.[ch]$' <<<"$files" || true); do
    head -1 "$f" | grep -qE 'Our own (shim|tool)' || echo "C file that isn't one of our shims or tools: $f"
  done
  scan 'CLAUDE\.md|\.claude/|docs/(FLAGS|ideas|structure|porting)\.md|spikes/|just doctor|doctor\.sh' \
    | sed 's/^/reference to an unexported file: /'
  scan 'PacketVideo|Organizational Partners|Google LLC' | sed 's/^/third-party copyright outside NOTICE: /'
  # Every `just <recipe>` the docs and sources mention exists.
  recipes=" $(just --summary) "
  for r in $(tr '\n' '\0' <<<"$text" | xargs -0 grep -ohE '`just [a-z][a-z0-9-]*' -- 2>/dev/null | awk '{print $2}' | sort -u); do
    [[ "$recipes" == *" $r "* ]] || echo "mentions 'just $r', the justfile has no such recipe"
  done
)
if [ -n "$out" ]; then
  sed 's/^/export-guard: /' <<<"$out"
  exit 1
fi
echo "export-guard: ok ($(wc -l <<<"$files") files)"
