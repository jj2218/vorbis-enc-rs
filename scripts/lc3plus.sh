#!/usr/bin/env bash
# Fetch ETSI TS 103 634 V1.7.1 (LC3plus) into the gitignored .cache/lc3plus/ and extract only
# its `thetest*.wav` inputs (realistic music and speech at 8-96 kHz) into .cache/lc3plus/wav/:
# test input audio (the LC3 conformance runs, the Vorbis tests and bench). "Copyright licence solely through ETSI IPR Policy":
# oracle input only, never committed or ported. SHA-256 pinned. Idempotent.
set -euo pipefail
cd "$(dirname "$0")/.."

URL=https://www.etsi.org/deliver/etsi_ts/103600_103699/103634/01.07.01_60/ts_103634v010701p0.zip
SHA=0b9de69247d2f24ae782ada4255e31c044a1ab4f14ec6798ccf6dd3651d77f7e
DIR=.cache/lc3plus
ZIP=$DIR/ts_103634v010701p0.zip

source scripts/fetch.sh
fetch "$URL" "$SHA" "$ZIP"
# the extracted items' presence is the marker (a deleted wav/ is re-extracted)
if ! ls "$DIR"/wav/thetest*.wav >/dev/null 2>&1; then
  python3 -I scripts/lc3plus_wav.py "$(realpath "$ZIP")" "$(realpath "$DIR")/wav"
fi
echo "lc3plus: ok ($DIR/wav: $(ls "$DIR/wav" | wc -l) thetest*.wav; sha256 verified)"
