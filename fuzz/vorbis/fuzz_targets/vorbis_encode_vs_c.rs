//! Vorbis encoder differential: `vorbis_enc::Encoder` against strict aoTuV (`aotuv-sys`), the whole
//! Ogg stream compared byte for byte (serial 1, as the C harness pages it).
//! Input: three configuration bytes (rate: one of 8–96 kHz, 8000 + v·679 Hz or 1000 + v·111 Hz;
//! channels 1–8; VBR quality −0.2 + v/105.8, or a managed min/average/max from bits of the
//! third), then
//! chunks: a 3-byte header (length 1 + v % 2048 samples per channel; repeat count 1–8, so long
//! states are reached; gain 2^-(0…15)) and that chunk's 16-bit PCM (little-endian, interleaved;
//! short data is zero-padded), written to both encoders in the same chunks.
#![no_main]

use aotuv_sys::{Encoder as CEncoder, Mode, STRICT};
use libfuzzer_sys::fuzz_target;
use std::num::{NonZeroU8, NonZeroU32};
use vorbis_enc::{BitrateManagementStrategy, EncoderBuilder};

const RATES: [u32; 8] = [8000, 11025, 16000, 22050, 32000, 44100, 48000, 96000];

fuzz_target!(|data: &[u8]| {
    let [r, c, m, rest @ ..] = data else {
        return;
    };
    let (r, c, m, mut rest) = (*r, *c, *m, rest);
    let rate = if r & 0x80 == 0 {
        RATES[usize::from(r % 8)]
    } else if r & 0x40 == 0 {
        8000 + u32::from(r & 0x3f) * 679
    } else {
        1000 + u32::from(r & 0x3f) * 111
    };
    let ch = usize::from(c % 8) + 1;
    let mode = if m & 0x80 == 0 {
        Mode::Vbr(-0.2 + f32::from(m & 0x7f) / 105.8)
    } else {
        // per channel 8–64 kbit/s steps; each limit present or not
        let k = |b: u8| i64::from(8 + (b & 7) * 8) * 1000 * ch as i64;
        let on = |bit: u8, v: i64| if m & bit != 0 { v } else { -1 };
        let (min, nominal, max) = (
            on(0x40, k(m >> 3) / 2),
            on(0x20, k(m)),
            on(0x10, k(m) * 3 / 2),
        );
        if min < 0 && nominal < 0 && max < 0 {
            return;
        }
        Mode::Managed { min, nominal, max }
    };
    let c_enc = CEncoder::new(STRICT, ch, i64::from(rate), mode);
    let o = |b: i64| NonZeroU32::new(b.max(0) as u32);
    let strategy = match mode {
        Mode::Vbr(q) => BitrateManagementStrategy::QualityVbr { target_quality: q },
        Mode::Managed { min, nominal, max } => BitrateManagementStrategy::Managed {
            minimum_bitrate: o(min),
            average_bitrate: o(nominal),
            maximum_bitrate: o(max),
        },
    };
    let b = EncoderBuilder::new(
        NonZeroU32::new(rate).unwrap(),
        NonZeroU8::new(ch as u8).unwrap(),
        Vec::new(),
    )
    .stream_serial(1)
    .bitrate_management_strategy(strategy);
    let rust = b.build();
    let (mut c_enc, mut rust) = match (c_enc, rust) {
        (Some(c), Ok(r)) => (c, r),
        (None, Err(_)) => return,
        (c, r) => panic!(
            "{rate} Hz × {ch} {mode:?}: C accepts {}, Rust {}",
            c.is_some(),
            r.is_ok()
        ),
    };
    let mut c_out = c_enc.headers().to_vec();
    let mut chunks = 0;
    while let [h0, h1, h2, tail @ ..] = rest {
        let len = 1 + usize::from(u16::from_le_bytes([*h0, *h1])) % 2048;
        let reps = 1 + usize::from(h2 & 7);
        let gain = 1.0 / f32::from(1u16 << ((h2 >> 3) % 16));
        let take = (2 * len * ch).min(tail.len());
        let (bytes, next) = tail.split_at(take);
        rest = next;
        let mut pcm = vec![vec![0f32; len]; ch];
        for (i, &s) in bytes.as_chunks::<2>().0.iter().enumerate() {
            pcm[i % ch][i / ch] = f32::from(i16::from_le_bytes(s)) / 32768.0 * gain;
        }
        let refs: Vec<&[f32]> = pcm.iter().map(|v| &v[..]).collect();
        for _ in 0..reps {
            c_out.extend(c_enc.write(&refs));
            rust.encode_audio_block(&refs).unwrap();
            chunks += 1;
            if chunks > 64 {
                break;
            }
        }
        if chunks > 64 {
            break;
        }
    }
    c_out.extend(c_enc.finish());
    let r_out = rust.finish().unwrap();
    assert!(
        r_out == c_out,
        "{rate} Hz × {ch} {mode:?}: {} vs C {} bytes",
        r_out.len(),
        c_out.len()
    );
});
