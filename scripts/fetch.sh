#!/usr/bin/env bash
# Sourced by the download scripts: `fetch URL SHA256 FILE` downloads URL to FILE unless FILE
# already matches the pinned SHA-256, then verifies it (a mismatch fails, the file is kept as
# FILE.bad for inspection). Idempotent. No User-Agent header: 3GPP refuses curl's default one
# and SourceForge a generic browser one (both 403, 2026-11); both accept none.
fetch() {
  local url=$1 sha=$2 file=$3
  mkdir -p "$(dirname "$file")"
  echo "$sha  $file" | sha256sum -c --status 2>/dev/null && return 0
  echo "fetch: downloading $url"
  curl -fsSL --retry 3 -A "" -o "$file.part" "$url"
  if ! echo "$sha  $file.part" | sha256sum -c --status; then
    mv "$file.part" "$file.bad"
    echo "fetch: SHA-256 MISMATCH for $url (kept as $file.bad)" >&2
    return 1
  fi
  mv "$file.part" "$file"
}
