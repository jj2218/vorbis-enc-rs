//! A Vorbis encoder in pure Rust: a port of aoTuV b6.03, the quality-tuned Vorbis encoder
//! (libvorbis 1.3.7 with Aoyumi's tuning; BSD-3, see NOTICE), producing exactly the same files
//! as aoTuV's reference build: aoTuV built without its SIMD and without `-ffast-math` (the
//! operation order its source states), so the output is the same on every CPU (tested: x86-64
//! and aarch64 Linux; elsewhere it also needs a libm that rounds `cos`/`exp`/`atan`/… as glibc
//! and musl do, unverified on macOS and Windows; x87 floats are refused at compile time). The same Ogg stream byte for byte given the same sequence of block
//! lengths, VBR and managed bitrate, every channel count and rate aoTuV's setups cover (tested
//! against the C: 1–255 channels, 1–192 kHz); the comment header carries aoTuV's vendor string
//! (`AO; aoTuV [20110424] (based on libvorbis 1.3.7)`). No input panics.
//!
//! ```
//! use std::num::{NonZeroU8, NonZeroU32};
//! use vorbis_enc::{BitrateManagementStrategy, EncoderBuilder};
//!
//! let rate = NonZeroU32::new(44100).unwrap();
//! let mut enc = EncoderBuilder::new(rate, NonZeroU8::new(2).unwrap(), Vec::new())
//!     .bitrate_management_strategy(BitrateManagementStrategy::QualityVbr { target_quality: 0.5 })
//!     .comment_tag("TITLE", "A tone")
//!     .build()?;
//! let tone: Vec<f32> = (0..44100).map(|i| (i as f32 * 0.0627).sin() * 0.5).collect();
//! enc.encode_audio_block([&tone[..], &tone[..]])?;
//! let ogg: Vec<u8> = enc.finish()?;
//! assert_eq!(&ogg[..4], b"OggS");
//! # Ok::<(), vorbis_enc::Error>(())
//! ```

#![cfg_attr(not(test), forbid(unsafe_code))]

// x87 floats (32-bit x86 without SSE2) round every operation to 80 bits first: the output is
// then not aoTuV's, and the rounding helpers' exactness tricks fail (measured: degraded audio).
#[cfg(all(target_arch = "x86", not(target_feature = "sse2")))]
compile_error!(
    "vorbis-enc needs IEEE single/double float arithmetic: on 32-bit x86 enable SSE2 \
     (e.g. target i686, or -C target-feature=+sse2); x87 floats change the output"
);

mod analysis;
mod bitpack;
mod bitrate;
mod block;
mod codebook;
mod encoder;
mod envelope;
mod floor1;
mod info;
mod lpc;
mod mapping0;
mod mdct;
mod ogg;
mod psy;
mod psy_tables;
mod res0;
mod scales;
mod setup;
mod setup_tables;
mod smallft;
mod tables;
mod templates;
mod window;

pub use encoder::{BitrateManagementStrategy, Encoder, EncoderBuilder, Error};

/// The README's example compiles and runs.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctest;

#[cfg(test)]
mod libm_pin;
#[cfg(test)]
mod testutil;
