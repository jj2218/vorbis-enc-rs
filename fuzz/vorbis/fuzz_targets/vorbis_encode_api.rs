//! Vorbis encoder API robustness: any configuration the public API admits (rates 1 Hz–200 kHz and
//! beyond, 1–8 channels, any VBR quality incl. non-finite, managed min/avg/max in any order) and any
//! `f32` input (raw bit patterns: NaN, ±inf, subnormals, huge; or 16-bit PCM at a gain 2^-15…2^31),
//! in blocks of 0–4095 samples. Must never panic; `build` errors exactly when the C rejects (or the
//! quality is not finite); a stream
//! decodes with lewton to the configured channels and rate; and while every sample is finite and
//! within ±2^20 the stream is byte-identical with strict aoTuV (`aotuv-sys`, serial 1).
//! Input: 4 configuration bytes (rate, channels, strategy, sample mode), then chunks of a 2-byte
//! header (length v % 4096, 0 = an empty block) and that chunk's samples (zero-padded when short).
#![no_main]

use aotuv_sys::{Encoder as CEncoder, Mode, STRICT};
use libfuzzer_sys::fuzz_target;
use std::io::Cursor;
use std::num::{NonZeroU8, NonZeroU32};
use vorbis_enc::{BitrateManagementStrategy as S, EncoderBuilder};

const RATES: [u32; 16] = [
    8000,
    11025,
    16000,
    22050,
    32000,
    44100,
    48000,
    96000,
    1,
    7999,
    9000,
    50000,
    192000,
    200000,
    200001,
    u32::MAX,
];
const QS: [f32; 8] = [
    f32::NAN,
    f32::INFINITY,
    f32::NEG_INFINITY,
    1e30,
    -1.0,
    -0.1,
    1.0,
    1e-40,
];

fuzz_target!(|data: &[u8]| {
    let [r, c, m, mode, rest @ ..] = data else {
        return;
    };
    let (r, c, m, mode, mut rest) = (*r, *c, *m, *mode, rest);
    let rate = if r & 0x80 == 0 {
        RATES[usize::from(r % 16)]
    } else {
        1 + u32::from(r & 0x7f) * 1693
    };
    let ch = usize::from(c % 8) + 1;
    let strategy = match m >> 6 {
        0 => S::QualityVbr {
            target_quality: -0.2 + f32::from(m & 0x3f) / 50.0,
        },
        1 => S::QualityVbr {
            target_quality: QS[usize::from(m & 7)],
        },
        2 => S::Abr {
            average_bitrate: NonZeroU32::new(u32::from(m & 0x3f) * 6000 + 1).unwrap(),
        },
        _ => {
            // each limit absent or one of a few per-channel values, in any order (min > max too)
            let v = |b: u8| {
                NonZeroU32::new([0, 16_000, 48_000, 96_000][usize::from(b & 3)] * ch as u32)
            };
            S::Managed {
                minimum_bitrate: v(m),
                average_bitrate: v(m >> 2),
                maximum_bitrate: v(m >> 4),
            }
        }
    };
    let cmode = match strategy {
        S::QualityVbr { target_quality } => Mode::Vbr(target_quality),
        S::Abr { average_bitrate } => Mode::Managed {
            min: -1,
            nominal: average_bitrate.get().into(),
            max: -1,
        },
        S::Managed {
            minimum_bitrate,
            average_bitrate,
            maximum_bitrate,
        } => {
            let b = |v: Option<NonZeroU32>| v.map_or(-1, |v| i64::from(v.get()));
            Mode::Managed {
                min: b(minimum_bitrate),
                nominal: b(average_bitrate),
                max: b(maximum_bitrate),
            }
        }
        _ => unreachable!("not generated"),
    };
    let b = EncoderBuilder::new(
        NonZeroU32::new(rate).unwrap(),
        NonZeroU8::new(ch as u8).unwrap(),
        Vec::new(),
    )
    .stream_serial(1)
    .bitrate_management_strategy(strategy);
    let mut c_enc = CEncoder::new(STRICT, ch, i64::from(rate), cmode);
    // a non-finite quality: the C takes NaN/+inf as the top quality, the API rejects it
    let non_finite =
        matches!(strategy, S::QualityVbr { target_quality } if !target_quality.is_finite());
    let mut rust = match b.build() {
        Ok(e) => e,
        Err(e) => {
            assert!(
                non_finite || c_enc.is_none(),
                "{rate} Hz × {ch} {strategy:?}: C accepts, Rust {e}"
            );
            return;
        }
    };
    assert!(!non_finite, "{strategy:?}: accepted");
    assert!(
        c_enc.is_some(),
        "{rate} Hz × {ch} {strategy:?}: C rejects, Rust accepts"
    );
    let raw = mode & 1 == 1;
    let gain = 2f32.powi(i32::from(mode >> 1) % 47 - 15);
    let mut c_out = c_enc.as_ref().unwrap().headers().to_vec();
    let mut tame = true; // every sample finite and within ±2^20
    let mut blocks = 0;
    while let [h0, h1, tail @ ..] = rest {
        let len = usize::from(u16::from_le_bytes([*h0, *h1])) % 4096;
        let width = if raw { 4 } else { 2 };
        let take = (width * len * ch).min(tail.len());
        let (bytes, next) = tail.split_at(take);
        rest = next;
        let mut pcm = vec![vec![0f32; len]; ch];
        for (i, s) in bytes.chunks_exact(width).enumerate() {
            pcm[i % ch][i / ch] = if raw {
                f32::from_le_bytes(s.try_into().unwrap())
            } else {
                f32::from(i16::from_le_bytes([s[0], s[1]])) / 32768.0 * gain
            };
        }
        tame &= pcm.iter().flatten().all(|x| x.abs() <= (1 << 20) as f32);
        let refs: Vec<&[f32]> = pcm.iter().map(|v| &v[..]).collect();
        rust.encode_audio_block(&refs).unwrap();
        if let Some(c) = c_enc.as_mut().filter(|_| tame && len > 0) {
            c_out.extend(c.write(&refs));
        }
        blocks += 1;
        if blocks > 48 {
            break;
        }
    }
    let out = rust.finish().unwrap();
    if tame {
        c_out.extend(c_enc.as_mut().unwrap().finish());
        assert!(
            out == c_out,
            "{rate} Hz × {ch} {strategy:?}: {} vs C {} bytes",
            out.len(),
            c_out.len()
        );
    }
    let mut d =
        lewton::inside_ogg::OggStreamReader::new(Cursor::new(&out)).expect("lewton headers");
    assert_eq!(usize::from(d.ident_hdr.audio_channels), ch);
    assert_eq!(d.ident_hdr.audio_sample_rate, rate);
    while d.read_dec_packet_itl().expect("lewton packet").is_some() {}
});
