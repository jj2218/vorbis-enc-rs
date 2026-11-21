# vorbis-enc

A Vorbis encoder in pure Rust: a port of aoTuV, the quality-tuned Vorbis encoder, producing
exactly the same files as aoTuV's reference build.

## Compared to vorbis_rs
[`vorbis_rs`](https://crates.io/crates/vorbis_rs) wraps a C build of the same encoder
(aoTuV, with hand-written SIMD).

| | this crate | `vorbis_rs` |
|---|---|---|
| language | pure Rust, no C compiler | a C library behind a Rust wrapper |
| output | identical on every CPU (tested: x86-64, aarch64) | varies with the build (its SSE2, SSE3 and FMA builds differ) |
| sound quality | aoTuV's | aoTuV's (the same encoder; only float rounding differs) |
| speed | slower ([measurements](https://github.com/jj2218/vorbis-enc-rs/blob/main/docs/RESULTS.md)) | faster |

Pick this crate for a pure-Rust build or reproducible output, `vorbis_rs` for maximum speed.

## Example
```rust
use std::num::{NonZeroU8, NonZeroU32};
use vorbis_enc::{BitrateManagementStrategy, EncoderBuilder};

fn main() -> Result<(), vorbis_enc::Error> {
    let rate = NonZeroU32::new(44100).unwrap();
    let mut enc = EncoderBuilder::new(rate, NonZeroU8::new(2).unwrap(), Vec::new())
        .bitrate_management_strategy(BitrateManagementStrategy::QualityVbr { target_quality: 0.5 })
        .comment_tag("TITLE", "A tone")
        .build()?;
    let tone: Vec<f32> = (0..44100).map(|i| (i as f32 * 0.0627).sin() * 0.5).collect();
    enc.encode_audio_block([&tone[..], &tone[..]])?;
    let ogg: Vec<u8> = enc.finish()?; // an Ogg Vorbis file
    assert_eq!(&ogg[..4], b"OggS");
    Ok(())
}
```

Planar `f32` samples in, an Ogg Vorbis stream out to any `Write`; quality VBR or managed
bitrate (average, minimum and maximum); comments; the API in `vorbis_rs`'s shape (a builder,
`encode_audio_block`, `finish`; the names without its `Vorbis` prefix, consuming setters).
Channels in Vorbis order (5.1: FL C FR RL RR LFE; see `EncoderBuilder::new`). No dependencies, no `unsafe`; no input panics (any `f32`
sample, NaN included).

## Results
Measured, with the recipe that reproduces each number, in
[`docs/RESULTS.md`](https://github.com/jj2218/vorbis-enc-rs/blob/main/docs/RESULTS.md).

- **The same files as aoTuV b6.03** (built without `-ffast-math` or its SIMD, so its output is
  the one its source states): the Ogg streams byte for byte, VBR from quality −0.2 to 1.0 and
  managed bitrate on every path of its bitrate manager, 1 kHz to 192 kHz, 1 to 255 channels,
  on x86-64 and aarch64 (Linux), given the same sequence of block lengths. The stream's vendor string is aoTuV's own
  (`AO; aoTuV [20110424] (based on libvorbis 1.3.7)`), so files match to the byte.
- **Fuzzed against aoTuV**: rates, 1 to 8 channels, VBR or bitrate limits, PCM in random
  chunks; the whole stream compared. A second target feeds any `f32` (NaN, ±inf, huge) and any
  configuration: no panic, and lewton decodes every stream.
- **Two defects of the C absent** (a leak per 6-channel encoder, a read past a table): see
  the results page.

## Scope
Encoding only: decode with libvorbis, [`lewton`](https://crates.io/crates/lewton) or
[`symphonia`](https://crates.io/crates/symphonia). Both Rust decoders (lewton 0.10, symphonia
0.5) return a stream whose end is on its first audio page (under ~10,000 samples at 44.1 kHz)
padded to a block end, and symphonia 0.5 mis-decodes 3 or more channels: libvorbis' own
streams alike. The channel counts and sample rates aoTuV's
setups cover. Needs Rust 1.88 or later. Byte-identical output is tested on Linux (x86-64,
aarch64); it builds anywhere, but on macOS and Windows the platform libm (`cos`, `exp`,
`atan`, as the C calls them) is unverified, so files there may differ from aoTuV's. 32-bit
x86 needs SSE2 (x87 floats are refused at compile time).

## License
BSD-3-Clause, as libvorbis and aoTuV
([LICENSE](https://github.com/jj2218/vorbis-enc-rs/blob/main/crates/vorbis-enc/LICENSE)). Derived
from aoTuV b6.03 (© Aoyumi) on libvorbis 1.3.7 (© Xiph.Org Foundation); see
[NOTICE](https://github.com/jj2218/vorbis-enc-rs/blob/main/crates/vorbis-enc/NOTICE).
