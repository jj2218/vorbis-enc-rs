//! Round trip through an independent decoder: streams of the public `Encoder` decode with
//! lewton (pure-Rust Vorbis decoder) to the input's channel count, rate, comments and length,
//! close to the input (SNR per channel), VBR and managed, mono to 6 channels, 8–96 kHz.

use aotuv_sys::{spike_pcm, thetest};
use lewton::inside_ogg::OggStreamReader;
use std::io::Cursor;
use std::num::{NonZeroU8, NonZeroU32};
use vorbis_enc::{BitrateManagementStrategy, EncoderBuilder};

/// `ch` channels of 2 s of the item at `khz` (beyond 2: delayed, quieter copies; the 6th,
/// 5.1's LFE, coded only below 250 Hz: a 60 Hz tone).
fn input(khz: u32, ch: usize) -> (u32, Vec<Vec<f32>>) {
    let (rate, item) = thetest(khz);
    let item = &item[..(rate as usize * 2).min(item.len())];
    let mut pcm = spike_pcm(item, ch.min(2));
    for c in 2..ch {
        if c == 5 {
            let w = 2.0 * std::f32::consts::PI * 60.0 / rate as f32;
            pcm.push(
                (0..item.len())
                    .map(|j| 0.5 * (w * j as f32).sin())
                    .collect(),
            );
            continue;
        }
        let d = 113 * c;
        pcm.push(
            (0..item.len())
                .map(|j| {
                    if j < d {
                        0.0
                    } else {
                        item[j - d] as f32 / 32768.0 / c as f32
                    }
                })
                .collect(),
        );
    }
    (rate, pcm)
}

/// Encodes in 1000-sample blocks; decodes; returns the decoded planar samples (scaled to ±1).
fn round_trip(rate: u32, pcm: &[Vec<f32>], s: BitrateManagementStrategy) -> Vec<Vec<f32>> {
    let b = EncoderBuilder::new(
        NonZeroU32::new(rate).unwrap(),
        NonZeroU8::new(pcm.len() as u8).unwrap(),
        Vec::new(),
    )
    .bitrate_management_strategy(s)
    .comment_tag("TITLE", "round trip");
    let mut e = b.build().unwrap();
    for k in (0..pcm[0].len()).step_by(1000) {
        let m = (pcm[0].len() - k).min(1000);
        e.encode_audio_block(pcm.iter().map(|c| &c[k..k + m]))
            .unwrap();
    }
    let ogg = e.finish().unwrap();
    let mut r = OggStreamReader::new(Cursor::new(ogg)).unwrap();
    assert_eq!(r.ident_hdr.audio_channels as usize, pcm.len());
    assert_eq!(r.ident_hdr.audio_sample_rate, rate);
    assert_eq!(
        r.comment_hdr.comment_list,
        vec![("TITLE".to_string(), "round trip".to_string())]
    );
    let ch = pcm.len();
    let mut out = vec![Vec::new(); ch];
    while let Some(p) = r.read_dec_packet_itl().unwrap() {
        for (i, s) in p.iter().enumerate() {
            out[i % ch].push(*s as f32 / 32768.0);
        }
    }
    out
}

/// SNR (dB) of `y` against `x` over their common length.
fn snr(x: &[f32], y: &[f32]) -> f64 {
    let (mut s, mut n) = (0f64, 0f64);
    for (&a, &b) in x.iter().zip(y) {
        s += (a as f64).powi(2);
        n += (a as f64 - b as f64).powi(2);
    }
    10.0 * (s / n.max(1e-30)).log10()
}

#[test]
fn lewton_decodes_the_encoder_output() {
    let cases: [(u32, usize, BitrateManagementStrategy, f64); 7] = [
        (
            44,
            1,
            BitrateManagementStrategy::QualityVbr {
                target_quality: 0.5,
            },
            26.0,
        ),
        (
            44,
            2,
            BitrateManagementStrategy::QualityVbr {
                target_quality: 1.0,
            },
            38.0,
        ),
        (
            48,
            6,
            BitrateManagementStrategy::QualityVbr {
                target_quality: 0.3,
            },
            10.0,
        ),
        (
            8,
            1,
            BitrateManagementStrategy::QualityVbr {
                target_quality: -0.1,
            },
            11.0,
        ),
        (
            96,
            2,
            BitrateManagementStrategy::QualityVbr {
                target_quality: 0.7,
            },
            27.0,
        ),
        (
            44,
            2,
            BitrateManagementStrategy::Abr {
                average_bitrate: NonZeroU32::new(128_000).unwrap(),
            },
            21.0,
        ),
        (
            16,
            2,
            BitrateManagementStrategy::ConstrainedAbr {
                maximum_bitrate: NonZeroU32::new(48_000).unwrap(),
            },
            10.0,
        ),
    ];
    // minimum SNR per case, ~3 dB under the measured (the 5.1 case: its LFE, 12.5 dB)
    for (khz, ch, s, min_snr) in cases {
        let (rate, pcm) = input(khz, ch);
        let out = round_trip(rate, &pcm, s);
        let n = pcm[0].len();
        for c in 0..ch {
            // the decoder emits whole blocks: at most one long block past the end
            assert!(
                out[c].len() >= n && out[c].len() <= n + 8192,
                "{khz} kHz × {ch} {s:?}: {} samples for {n}",
                out[c].len()
            );
            let q = snr(&pcm[c], &out[c]);
            println!("{khz} kHz × {ch} {s:?} channel {c}: SNR {q:.1} dB");
            assert!(
                q >= min_snr,
                "{khz} kHz × {ch} {s:?} channel {c}: SNR {q:.1} dB"
            );
        }
    }
}
