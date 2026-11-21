//! The platform libm, pinned. The encoder calls f64 `cos`/`sin`/`exp`/`ln`/`atan`/`powf` from
//! the platform's libm at run time, as the C does (the MDCT and FFT twiddles, the envelope's and
//! psy's windows, psy's `toOC`/`fromOC`/`toBARK` look tables, codebook sizes). IEEE 754 does not
//! require those to be correctly rounded, so "the same output on every CPU" holds only for a
//! libm that rounds them as glibc does, where the vs_c tests prove it. These tests compare with no C
//! output (but build with the crate's tests, whose C oracle builds on x86-64 and aarch64 only): they hash the twiddle tables and a whole encoded
//! stream (every look table and every per-block libm call) against values pinned on glibc, so
//! a libm that rounds one value differently fails here, loudly, instead of silently changing
//! the files. A failure on a new platform means its libm differs: the output is then not
//! aoTuV's, and the fix is a libm-free (correctly rounded) implementation, not a new pin.

use crate::mdct::Mdct;
use crate::smallft::Drft;
use crate::testutil::Rng;
use crate::{BitrateManagementStrategy, EncoderBuilder};
use std::num::{NonZeroU8, NonZeroU32};

/// FNV-1a, 64 bits.
fn fnv(h: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        *h = (*h ^ b as u64).wrapping_mul(0x100_0000_01b3);
    }
}

const FNV0: u64 = 0xcbf2_9ce4_8422_2325;

fn hash_f32(h: &mut u64, v: &[f32]) {
    for x in v {
        fnv(h, &x.to_bits().to_le_bytes());
    }
}

/// `mdct_init`'s `trig` for every size the encoder uses (64…8192) and `drft_init`'s twiddles
/// for every block size (the psy FFT runs on whole blocks, 64…8192).
#[test]
fn twiddles_are_pinned() {
    let mut m = FNV0;
    let mut d = FNV0;
    for k in 6..=13 {
        hash_f32(&mut m, &Mdct::new(1 << k).trig);
        hash_f32(&mut d, &Drft::new(1 << k).trigcache);
    }
    assert_eq!(
        (m, d),
        (0x67f2_41b7_bafe_090d, 0x803c_b867_c9ae_5ab1),
        "MDCT/FFT twiddles differ from glibc's (see this module's doc)"
    );
}

/// A whole stream, mono 44.1 kHz and stereo 22.05 kHz VBR, silence then noise bursts (long
/// and short blocks, both setups' psy looks); input built from integers only (no libm).
#[test]
fn stream_is_pinned() {
    let mut h = FNV0;
    for (rate, ch, q) in [(44100, 1, 0.5), (22050, 2, 0.1)] {
        let mut r = Rng::new(rate as u64);
        let n = rate as usize / 2;
        let pcm: Vec<Vec<f32>> = (0..ch)
            .map(|_| {
                (0..n)
                    .map(|i| {
                        if (i / 2048) % 3 == 2 {
                            r.unit() * 0.5
                        } else {
                            0.0
                        }
                    })
                    .collect()
            })
            .collect();
        let mut enc = EncoderBuilder::new(
            NonZeroU32::new(rate).unwrap(),
            NonZeroU8::new(ch as u8).unwrap(),
            Vec::new(),
        )
        .bitrate_management_strategy(BitrateManagementStrategy::QualityVbr { target_quality: q })
        .stream_serial(1)
        .build()
        .unwrap();
        enc.encode_audio_block(&pcm).unwrap();
        fnv(&mut h, &enc.finish().unwrap());
    }
    assert_eq!(
        h, 0xb711_7eca_1017_573f,
        "encoded stream differs from glibc's (see this module's doc)"
    );
}
