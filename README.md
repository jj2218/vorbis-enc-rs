# vorbis-enc-rs

A Vorbis encoder in pure Rust: a port of aoTuV, the quality-tuned Vorbis encoder, producing
exactly the same files as aoTuV's reference build.

```sh
cargo add vorbis-enc
```

| crate | |
|---|---|
| [`vorbis-enc`](crates/vorbis-enc) | Vorbis encoder (aoTuV b6.03 on libvorbis 1.3.7): planar `f32` PCM in, Ogg Vorbis out; quality VBR and managed bitrate. Byte-identical with aoTuV's reference build on every CPU. [README: comparison with `vorbis_rs`, example, scope](crates/vorbis-enc/README.md) |
| `aotuv-sys` | test-only (not published): aoTuV b6.03 as the oracle (strict IEEE, no SIMD), plus the speed baselines (aoTuV with its SIMD as `vorbis_rs` builds it, xiph's libvorbis 1.3.7), compiled from pinned downloads |

No dependencies. Measured results, each with the command that reproduces it:
[`docs/RESULTS.md`](docs/RESULTS.md).

## Development
Needs a stable Rust toolchain (pinned in `rust-toolchain.toml`, installed by rustup), [`just`](https://github.com/casey/just),
a C compiler, `nm`/`objcopy` (binutils), `curl`, `unzip`, `xz` and `python3`.

```sh
just check     # the full gate: guard, fmt, clippy, docs, tests against aoTuV
just conform   # byte-identity vs aoTuV over the rate × channel × quality grid, one summary line each
just bench     # ×realtime, Rust vs aoTuV (with and without its SIMD) and xiph's libvorbis
just --list    # everything else (fuzz, perf, ...)
```

**Oracles.** The tests compare against aoTuV b6.03 (BSD-3-Clause; the sources the
`aotuv_lancer_vorbis_sys` 0.1.6 crate vendors), built without `-ffast-math` or its SIMD so its
output is the same on every CPU, and use the ETSI LC3plus test items (`thetest*.wav`,
TS 103 634) as input audio. `just check` downloads both on first use into the gitignored
`.cache/` and verifies each against a pinned SHA-256. The ETSI items are used as test input
only, never committed. The encoder is ported from aoTuV.

**CI** runs what builds without the downloads; run `just check` locally before a pull request.

**Releases** are published from a private development repository, one commit per release.
Pull requests are welcome: an accepted change is applied there with the same paths, credited
with `Co-authored-by`, and ships with the next release; the pull request is then closed with a
link to it.

## License
BSD-3-Clause ([LICENSE](LICENSE)), as libvorbis and aoTuV; attribution in
[`crates/vorbis-enc/NOTICE`](crates/vorbis-enc/NOTICE).
