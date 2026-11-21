# Results

Measured results of the Vorbis encoder (`crates/vorbis-enc`). Every number is reproducible with the
command in its heading; the oracle (aoTuV b6.03 as `aotuv_lancer_vorbis_sys` 0.1.6 vendors it)
and the test items (the ETSI LC3plus `thetest*.wav`, as input audio only) are downloaded and
SHA-verified on first use into `.cache/`, never committed.

## Conformance against aoTuV (`just conform`; all tests: `cargo test --release -p vorbis-enc`)
aoTuV built without `-ffast-math`, FMA contraction or Lancer's SSE paths (`crates/aotuv-sys`), so
its operation order is the source's and its output the same at -O0…-O3: the port keeps that order.
- Ogg streams byte-identical: 275 VBR streams (7 rates × 1/2/6 channels × q −0.2…1.0), 201
  managed-bitrate streams (every bitrate-manager path reached) and 89 edge streams (pure tones,
  1 kHz to 192 kHz, 3 to 255 channels, whole streams of 1 to 3,000 samples), every block's 15
  packet candidates compared too; x86-64 and aarch64 (qemu); the tests run with overflow
  checks on.
- Module by module against the C's function (MDCT, FFT, LPC, windows, setup and headers, Ogg
  framing, blocking and envelope, psychoacoustics); floor 1, coupling, residue and bitrate
  through every block's 15 packet candidates.

## Fuzzing (`just fuzz <target> [seconds]`)
| target | checks | findings |
|---|---|---|
| `vorbis_encode_vs_c` | the whole Ogg stream vs aoTuV: rates 1–96 kHz, 1–8 channels, VBR q −0.2…1.0 or managed limits, PCM in random chunks | the C's 6-channel leak (defect 1, LeakSanitizer) |
| `vorbis_encode_api` | any `f32` (NaN, ±inf, huge) and any configuration through the public API: no panic, lewton decodes, equal to the C's while samples stay within ±2^20 | debug-build overflow panics on valid input (fixed before release) |

## Speed (`CRATE=vorbis-enc just bench`)
ms per pass over thetest44 (10.09 s at 44.1 kHz; stereo: the right channel the left delayed and
scaled), VBR q 0.5, the whole Ogg stream into memory, best of 15, one pinned core, x86-64 (no
AVX2: every build baseline x86-64); medians of 3 runs. "Lancer": aoTuV-Lancer as `vorbis_rs`
builds it (`-O3`, its SSE/SSE2 paths; SSE3 off at baseline x86-64, as in `vorbis_rs`); "strict": the oracle build above; "xiph": libvorbis 1.3.7 as
its `configure` builds it (`-O3 -ffast-math`).

| input | Rust ×realtime | Lancer ×realtime | vs Lancer | vs strict | vs xiph |
|---|---:|---:|---:|---:|---:|
| mono | 158× | 252× | 0.63× | 1.03× | 0.82× |
| stereo | 69× | 105× | 0.66× | 1.00× | 0.75× |

Ratios are C time / Rust time (below 1: the C is faster). Lancer's SIMD (hand-written SSE in the
psychoacoustics, MDCT and FFT) gives no single output across CPUs; the exact port runs at the
strict build's speed. Measured 2026-11-21 on a quiet machine, the release code (×realtime vary with the machine's state; the ratios
are interleaved runs).

## Defects found in the C (aoTuV b6.03; each absent in the Rust port)
Each: **the defect**: its effect. Found: how. Rust: how the port avoids it.
1. **A leak per 6-channel encoder**: `vorbis_encode_residue_setup` leaks one
   `vorbis_info_residue0` (2,848 bytes) per 6-channel `vorbis_encode_init*` (1, 2 and 4
   channels are clean), so a process opening many 5.1 encoders grows without bound. Found:
   LeakSanitizer on the fuzz target's C side. Rust: no such allocation (owned values, dropped
   with the encoder).
2. **Reads one past the row** in `_vp_psy_init`'s noise-offset interpolation: at the top band
   (`inthalfoc` = 16, weight `del` = 0) it reads `noiseoff[j][17]` and `ntfix_offset[17]` of
   17 entries, past a static array in the second case (undefined behaviour; the value is
   discarded by `* 0` as long as the memory holds a finite float). Found: porting
   `_vp_psy_init` (Rust's bounds check). Rust: the term past the row is 0 without a read
   (`psy.rs`), giving the same value.

## Improvements over the C (aoTuV b6.03)
1. **One output on every CPU**: aoTuV-Lancer (the speed build `vorbis_rs` ships) gives three
   different streams for its SSE2, SSE3 and FMA builds, and its `rcpps` approximations differ
   per CPU, so its output is not defined by its source. Found: the oracle spike (strict vs
   Lancer builds). Rust: the strict build's streams byte for byte, on x86-64 and aarch64.
2. **One large write is linear**: `vorbis_analysis_blockout` moves all buffered PCM down after
   every block, so one write of a whole file is quadratic (48 kHz stereo in one write: 60 s
   17.7 s, 120 s 56.1 s, 11 min not done after 35 CPU-minutes; 95% in `memmove`). Found: the
   pre-publish review. Rust: the buffer moves only once the consumed offset passes the samples
   left (120 s in one write 7.0 s on the same loaded machine; quiet: 300 s 4.3 s in one write,
   3.6 s in 1024-sample writes), the same streams.
