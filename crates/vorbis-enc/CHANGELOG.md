# Changelog

One section per release, newest first. The release commit in the repository carries the
section of each crate it releases.

## 0.1.1 (2026-11-21)
Fixed
- `cargo test` from the published crate: the tests against aoTuV (dev-only, not published)
  are compiled out there, so the crate's own tests, doc tests included, pass (0.1.0's failed to
  compile). No change to the encoder.

## 0.1.0 (2026-11-20)
First release: a Vorbis encoder, ported from aoTuV b6.03 (on libvorbis 1.3.7).
- `EncoderBuilder` and `Encoder`: planar `f32` PCM in, an Ogg Vorbis stream out to any
  `Write`; quality VBR (−0.2 to 1.0) and managed bitrate (average, minimum, maximum); user
  comments; the channel counts and sample rates aoTuV's setups cover. No input panics; a
  non-finite quality or an invalid comment tag is rejected (aoTuV takes them silently).
  `Error` names the cause (`InvalidQuality`, `InvalidCommentTag`, `MissingBitrate`,
  `ChannelMismatch { expected, actual }`, …); after a sink error every call returns
  `StreamBroken`. 32-bit x86 needs SSE2 (x87 floats change the output: a compile error).
- The same files as aoTuV built without `-ffast-math` or its SIMD, byte for byte (aoTuV's
  vendor string included), on x86-64 and aarch64: VBR over 8 to 96 kHz, 1, 2 and 6 channels,
  every quality step, and edge cases from 1 kHz to 192 kHz and 1 to 255 channels; managed
  bitrate on every bitrate-manager path; fuzzing with 1 to 8 channels.
- Without two defects of the C: a leak per 6-channel encoder, a read past a table.
