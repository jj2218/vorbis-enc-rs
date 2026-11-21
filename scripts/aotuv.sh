#!/usr/bin/env bash
# Fetch aoTuV b6.03 (on libvorbis 1.3.7; BSD-3, © Aoyumi + Xiph.Org) as the sources the crate
# `aotuv_lancer_vorbis_sys` 0.1.6 vendors (what `vorbis_rs` builds; its `ogg_vendor` is libogg
# 1.3.5's framing.c/bitwise.c, byte-identical) into the gitignored .cache/aotuv/: the Vorbis
# port's C oracle (built strict, Lancer's SIMD off) and its speed baseline (Lancer as built);
# and xiph's libvorbis 1.3.7 (upstream's `configure` build, a second speed baseline).
# SHA-256 pinned. Idempotent.
set -euo pipefail
cd "$(dirname "$0")/.."

URL=https://static.crates.io/crates/aotuv_lancer_vorbis_sys/aotuv_lancer_vorbis_sys-0.1.6.crate
SHA=5bc4fd1a61860d2f1198b60bedd30910eaffa978f1ee6214dfb24ac70d589225
DIR=.cache/aotuv
CRATE=$DIR/aotuv_lancer_vorbis_sys-0.1.6.crate

source scripts/fetch.sh
fetch "$URL" "$SHA" "$CRATE"
if [ ! -f "$DIR/.unpacked" ]; then
  rm -rf "$DIR/aotuv_lancer_vorbis_sys-0.1.6"
  tar xzf "$CRATE" -C "$DIR"
  touch "$DIR/.unpacked"
fi
XURL=https://downloads.xiph.org/releases/vorbis/libvorbis-1.3.7.tar.xz
XSHA=b33cc4934322bcbf6efcbacf49e3ca01aadbea4114ec9589d1b1e9d20f72954b
fetch "$XURL" "$XSHA" "$DIR/libvorbis-1.3.7.tar.xz"
if [ ! -f "$DIR/.unpacked-xiph" ]; then
  rm -rf "$DIR/libvorbis-1.3.7"
  tar xJf "$DIR/libvorbis-1.3.7.tar.xz" -C "$DIR"
  touch "$DIR/.unpacked-xiph"
fi
echo "aotuv: ok ($DIR/aotuv_lancer_vorbis_sys-0.1.6, $DIR/libvorbis-1.3.7; sha256 verified)"
