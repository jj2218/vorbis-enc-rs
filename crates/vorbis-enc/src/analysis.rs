//! The encode loop as `vorbis_analysis_buffer`/`_wrote`, `vorbis_analysis_blockout`,
//! `vorbis_analysis`, `vorbis_bitrate_addblock`/`_flushpacket` run it: PCM in, Vorbis packets
//! out (the headers, then one audio packet per block); [`crate::Encoder`] pages them into Ogg.

use crate::bitrate::BitrateManager;
use crate::block::Dsp;
use crate::info::headers;
use crate::mapping0::Backend;
use crate::setup::CodecSetup;

/// One Vorbis packet with the granule position and end flag the C gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Packet {
    pub data: Vec<u8>,
    pub granulepos: i64,
    pub eos: bool,
}

/// `vorbis_dsp_state` + `vorbis_block` for encoding.
#[derive(Clone, Debug)]
pub(crate) struct PacketEncoder {
    ci: CodecSetup,
    dsp: Dsp,
    backend: Backend,
    bm: BitrateManager,
    /// per block, the packet blobs before bitrate management (`Some`: recorded; the tests)
    #[cfg(test)]
    pub trace: Option<Vec<Vec<Vec<u8>>>>,
}

impl PacketEncoder {
    /// From a setup (`vorbis_analysis_init`).
    pub(crate) fn new(ci: CodecSetup) -> Self {
        PacketEncoder {
            dsp: Dsp::new(&ci),
            backend: Backend::new(&ci),
            bm: BitrateManager::new(&ci),
            ci,
            #[cfg(test)]
            trace: None,
        }
    }

    /// The three header packets (no user comments).
    pub(crate) fn headers(&self) -> [Vec<u8>; 3] {
        headers(&self.ci)
    }

    /// `pcm[c]` per channel (all one length, not empty): the packets completed.
    pub(crate) fn write(&mut self, pcm: &[&[f32]]) -> Vec<Packet> {
        self.dsp.write(pcm);
        self.packets()
    }

    /// The end of the stream (`vorbis_analysis_wrote(0)`): the last packets.
    pub(crate) fn finish(&mut self) -> Vec<Packet> {
        self.dsp.write_end();
        self.packets()
    }

    /// The packets of every block the buffered PCM completes.
    fn packets(&mut self) -> Vec<Packet> {
        let mut out = Vec::new();
        while let Some(mut vb) = self.dsp.blockout(&self.ci) {
            let (mut blobs, ampmax) = self.backend.mapping0_forward(&self.ci, &mut vb);
            self.dsp.vbi_ampmax = ampmax;
            #[cfg(test)]
            if let Some(t) = &mut self.trace {
                t.push(blobs.iter().map(|b| b.buffer().to_vec()).collect());
            }
            let choice = self.bm.choose(&self.ci, vb.w, &mut blobs);
            out.push(Packet {
                data: blobs[choice].buffer().to_vec(),
                granulepos: vb.granulepos,
                eos: vb.eofflag,
            });
        }
        out
    }
}

#[cfg(all(test, oracle))]
mod vs_c {
    use super::*;
    use crate::encoder::{BitrateManagementStrategy, EncoderBuilder};
    use crate::testutil::Rng;
    use aotuv_sys::{Mode, encode_trace, spike_pcm, thetest};
    use std::num::{NonZeroU8, NonZeroU32};

    /// The Rust packets for `pcm` written as `encode_trace` writes (1024-sample chunks).
    fn packets(enc: &mut PacketEncoder, pcm: &[Vec<f32>]) -> Vec<Packet> {
        let mut out: Vec<Packet> = enc
            .headers()
            .into_iter()
            .map(|data| Packet {
                data,
                granulepos: 0,
                eos: false,
            })
            .collect();
        let n = pcm[0].len();
        for k in (0..n).step_by(1024) {
            let m = (n - k).min(1024);
            let chunk: Vec<&[f32]> = pcm.iter().map(|c| &c[k..k + m]).collect();
            out.extend(enc.write(&chunk));
        }
        out.extend(enc.finish());
        out
    }

    /// The first packet that differs (index, Rust and C lengths), if any.
    fn first_diff(r: &[Packet], c: &[aotuv_sys::Packet]) -> Option<(usize, usize, usize)> {
        for (i, (a, b)) in r.iter().zip(c).enumerate() {
            if a.data != b.data || a.granulepos != b.granulepos || a.eos != b.eos {
                return Some((i, a.data.len(), b.data.len()));
            }
        }
        (r.len() != c.len()).then_some((r.len().min(c.len()), r.len(), c.len()))
    }

    /// `ch` channels from a mono item: the spike's stereo pair (left; left delayed and
    /// scaled), then per further channel a delay and gain, every third one time-reversed (so
    /// coupled pairs see correlated and uncorrelated content), `secs` seconds long.
    fn multichannel(pcm: &[i16], rate: u32, ch: usize, secs: f32) -> Vec<Vec<f32>> {
        let len = ((rate as f32 * secs) as usize).min(pcm.len());
        let pcm = &pcm[..len];
        let mut out = spike_pcm(pcm, ch.min(2));
        for c in 2..ch {
            let d = 97 * c;
            let g = 1.0 / c as f32;
            out.push(
                (0..len)
                    .map(|j| {
                        let s = if c % 3 == 0 {
                            pcm[len - 1 - j]
                        } else if j < d {
                            0
                        } else {
                            pcm[j - d]
                        };
                        s as f32 / 32768.0 * g
                    })
                    .collect(),
            );
        }
        out
    }

    /// A stress stream for the bitrate manager, 0.3 s per segment: digital silence (the
    /// smallest packets: padding to a minimum), full-scale white noise (the largest:
    /// truncation under a maximum), a quiet sine, clicks, the item; per channel its own noise.
    fn stress(item: &[i16], rate: u32, ch: usize) -> Vec<Vec<f32>> {
        let seg = (rate as f32 * 0.3) as usize;
        (0..ch)
            .map(|c| {
                let mut r = Rng::new(c as u64 + 1);
                let mut v = Vec::with_capacity(seg * 6);
                v.extend(std::iter::repeat_n(0.0, seg));
                v.extend((0..seg).map(|_| r.unit()));
                v.extend((0..seg).map(|j| 1e-3 * (j as f32 * 0.07 * (c + 1) as f32).sin()));
                v.extend((0..seg).map(|j| if j % 997 == 0 { 0.9 } else { 0.0 }));
                v.extend(item[..seg].iter().map(|&s| s as f32 / 32768.0));
                v.extend((0..seg).map(|_| r.unit() * 0.5));
                v
            })
            .collect()
    }

    /// The public encoder's whole Ogg stream (serial 1, as the shim's), 1024-sample blocks.
    fn stream(rate: i64, mode: Mode, pcm: &[Vec<f32>]) -> Vec<u8> {
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
            NonZeroU32::new(rate as u32).unwrap(),
            NonZeroU8::new(pcm.len() as u8).unwrap(),
            Vec::new(),
        )
        .stream_serial(1)
        .bitrate_management_strategy(strategy);
        let mut e = b.build().unwrap();
        let n = pcm[0].len();
        for k in (0..n).step_by(1024) {
            let m = (n - k).min(1024);
            e.encode_audio_block(pcm.iter().map(|c| &c[k..k + m]))
                .unwrap();
        }
        e.finish().unwrap()
    }

    /// One configuration: every block's packet blobs (before bitrate management), the
    /// packets and the public encoder's Ogg stream equal the C's; (audio packets, bytes).
    /// `None` when the C rejects the configuration (and Rust must too).
    fn check(rate: i64, mode: Mode, pcm: &[Vec<f32>], what: &str) -> Option<(usize, usize)> {
        let ch = pcm.len() as i64;
        let ci = match mode {
            Mode::Vbr(q) => CodecSetup::vbr(ch, rate, q),
            Mode::Managed { min, nominal, max } => CodecSetup::managed(ch, rate, max, nominal, min),
        };
        let Some(t) = encode_trace(rate, mode, pcm) else {
            assert!(
                ci.is_err(),
                "{what} {mode:?}: the C rejects it, Rust accepts"
            );
            return None;
        };
        let mut enc = PacketEncoder::new(ci.unwrap());
        enc.trace = Some(Vec::new());
        let r = packets(&mut enc, pcm);
        let rb = enc.trace.take().unwrap();
        for (b, (x, y)) in rb.iter().zip(&t.blobs).enumerate() {
            for k in 0..x.len() {
                assert!(x[k] == y[k], "{what} {mode:?}: block {b} blob {k}");
            }
        }
        assert_eq!(rb.len(), t.blobs.len(), "{what} {mode:?}: blocks");
        assert_eq!(
            first_diff(&r, &t.packets),
            None,
            "{what} {mode:?}: (packet, Rust bytes, C bytes)"
        );
        assert!(
            stream(rate, mode, pcm) == t.stream,
            "{what} {mode:?}: Ogg stream"
        );
        Some((r.len() - 3, r[3..].iter().map(|p| p.data.len()).sum()))
    }

    /// The rates of the grid and the item each is read from (22.05 kHz: the 24 kHz item).
    const RATES: [(i64, u32); 7] = [
        (8000, 8),
        (16000, 16),
        (22050, 24),
        (32000, 32),
        (44100, 44),
        (48000, 48),
        (96000, 96),
    ];

    /// VBR: blobs, packets and Ogg streams byte-identical with the C's over the grid: 7 rates
    /// × 1, 2, 6 channels × q −0.2…1.0 in steps of 0.1, 2 s of each item (the whole 44.1 kHz
    /// item mono and stereo at q 0.5).
    #[test]
    fn vbr_streams_match_c() {
        let (mut packets_n, mut bytes, mut streams) = (0, 0, 0);
        let mut add = |r: Option<(usize, usize)>| {
            let (p, b) = r.unwrap();
            packets_n += p;
            bytes += b;
            streams += 1;
        };
        for (rate, khz) in RATES {
            let (_, item) = thetest(khz);
            for ch in [1, 2, 6] {
                let pcm = multichannel(&item, rate as u32, ch, 2.0);
                for qi in -2..=10 {
                    let q = qi as f32 / 10.0;
                    add(check(
                        rate,
                        Mode::Vbr(q),
                        &pcm,
                        &format!("{rate} Hz × {ch}"),
                    ));
                }
            }
        }
        let (_, item) = thetest(44);
        for ch in [1, 2] {
            add(check(
                44100,
                Mode::Vbr(0.5),
                &spike_pcm(&item, ch),
                "thetest44",
            ));
        }
        println!(
            "vorbis vbr: {streams} streams, {packets_n} audio packets ({bytes} bytes) byte-identical"
        );
    }

    /// The corners the grid misses, run with overflow checks (where the C's `int` maths
    /// wraps): pure tones at 8 and 11.025 kHz, loud noise at 4 kHz, rates below 8 kHz and up
    /// to 192 kHz, 3, 4, 5, 8, 9, 128 and 255 channels, q −0.15 and −0.2.
    #[test]
    fn edge_streams_match_c() {
        let tone = |rate: u32, ch: usize, w: f32, secs: f32| -> Vec<Vec<f32>> {
            let n = (rate as f32 * secs) as usize;
            (0..ch)
                .map(|c| {
                    let w = w * (1.0 + c as f32 / 7.0);
                    (0..n).map(|j| 0.5 * (j as f32 * w).sin()).collect()
                })
                .collect()
        };
        let noise = |rate: u32, ch: usize, amp: f32, secs: f32| -> Vec<Vec<f32>> {
            let n = (rate as f32 * secs) as usize;
            (0..ch)
                .map(|c| {
                    let mut r = Rng::new(11 + c as u64);
                    (0..n).map(|_| amp * r.unit()).collect()
                })
                .collect()
        };
        let mut streams = 0;
        let mut run = |rate: i64, mode: Mode, pcm: &[Vec<f32>], what: &str| {
            check(rate, mode, pcm, what).unwrap();
            streams += 1;
        };
        // pure tones: the noise normaliser's `int` square overflows (1.16…2.56 rad/sample)
        for (rate, ch) in [(8000u32, 1usize), (8000, 2), (11025, 1)] {
            for w in [1.16f32, 1.32, 2.4] {
                for q in [0.5, 1.0] {
                    run(
                        rate.into(),
                        Mode::Vbr(q),
                        &tone(rate, ch, w, 2.0),
                        &format!("{rate} Hz × {ch} tone {w}"),
                    );
                }
            }
        }
        for rate in [1000u32, 4000, 7999, 12000, 192000] {
            for ch in [1, 2, 4] {
                run(
                    rate.into(),
                    Mode::Vbr(1.0),
                    &noise(rate, ch, 0.7, 1.5),
                    &format!("{rate} Hz × {ch} noise"),
                );
            }
        }
        let (_, item) = thetest(44);
        for ch in [3, 4, 5, 8] {
            let pcm = multichannel(&item, 44100, ch, 1.0);
            for q in [-0.2, -0.15, 0.4, 1.0] {
                run(44100, Mode::Vbr(q), &pcm, &format!("44100 Hz × {ch}"));
            }
        }
        let pcm = multichannel(&item, 44100, 3, 1.0);
        let mode = Mode::Managed {
            min: -1,
            nominal: 160_000,
            max: -1,
        };
        run(44100, mode, &pcm, "44100 Hz × 3");
        for (rate, ch) in [(8000u32, 128usize), (16000, 9), (16000, 255)] {
            run(
                rate.into(),
                Mode::Vbr(0.4),
                &tone(rate, ch, 0.3, 0.1),
                &format!("{rate} Hz × {ch}"),
            );
        }
        // whole streams of a few samples: the start-of-stream extrapolation's threshold (19–32
        // samples), a stream ending inside the first long block
        for len in [1usize, 2, 16, 32, 33, 40, 64, 65, 1057, 2047, 2049, 3000] {
            for (rate, ch) in [(44100u32, 1usize), (44100, 2), (8000, 1)] {
                let pcm: Vec<Vec<f32>> = (0..ch)
                    .map(|c| {
                        let mut r = Rng::new(5 + c as u64);
                        (0..len)
                            .map(|j| 0.5 * (j as f32 * 0.05).sin() + 0.1 * r.unit())
                            .collect()
                    })
                    .collect();
                run(
                    rate.into(),
                    Mode::Vbr(0.4),
                    &pcm,
                    &format!("{len} samples @ {rate} Hz × {ch}"),
                );
            }
        }
        println!("vorbis edges: {streams} streams byte-identical");
    }

    /// Managed bitrate: all 15 blobs of every block (three floor fits, twelve interpolated),
    /// the manager's choice, padding and truncation, the Ogg stream; average-only,
    /// min/max-bounded, maximum-only (to truncation), minimum-only (to padding) and
    /// hard-limited streams over the item and the stress stream.
    #[test]
    fn managed_streams_match_c() {
        let (mut packets_n, mut bytes, mut streams, mut rejected) = (0, 0, 0, 0);
        // (min, nominal, max) in kbit/s for two channels at ≥ 44.1 kHz
        let modes: [(i64, i64, i64); 7] = [
            (-1, 128, -1),
            (64, 96, 160),
            (-1, -1, 48),
            (112, -1, -1),
            (96, 96, 96),
            (-1, -1, 24),
            (192, -1, -1),
        ];
        for (rate, khz) in RATES {
            let (_, item) = thetest(khz);
            let k = if rate >= 44100 {
                1
            } else if rate >= 22050 {
                2
            } else {
                4
            };
            for ch in [1, 2, 6] {
                let inputs = [
                    multichannel(&item, rate as u32, ch, 2.0),
                    stress(&item, rate as u32, ch),
                ];
                for (min, nominal, max) in modes {
                    let s = |b: i64| {
                        if b > 0 {
                            b * 1000 * ch as i64 / 2 / k
                        } else {
                            b
                        }
                    };
                    let mode = Mode::Managed {
                        min: s(min),
                        nominal: s(nominal),
                        max: s(max),
                    };
                    for pcm in &inputs {
                        match check(rate, mode, pcm, &format!("{rate} Hz × {ch}")) {
                            Some((p, b)) => {
                                packets_n += p;
                                bytes += b;
                                streams += 1;
                            }
                            None => rejected += 1,
                        }
                    }
                }
            }
        }
        println!(
            "vorbis managed: {streams} streams, {packets_n} audio packets ({bytes} bytes) byte-identical ({rejected} configurations rejected by both)"
        );
    }

    /// The bitrate manager's limits: 4 s of full-scale noise (beyond the reservoir) against
    /// low average, maximum and hard limits, then silence against a high minimum, mono and
    /// stereo at 44.1 and 8 kHz; every manager path is reached (counted in `bitrate::HITS`).
    #[test]
    fn managed_limits_match_c() {
        use crate::bitrate::HITS;
        let (mut streams, mut packets_n) = (0, 0);
        for rate in [44100i64, 8000] {
            for ch in [1usize, 2] {
                let n = rate as usize * 4;
                let pcm: Vec<Vec<f32>> = (0..ch)
                    .map(|c| {
                        let mut r = Rng::new(7 + c as u64);
                        let mut v: Vec<f32> = (0..n).map(|_| r.unit()).collect();
                        v.extend(std::iter::repeat_n(0.0, n / 2));
                        v.extend((0..n / 2).map(|_| r.unit() * 0.7));
                        v
                    })
                    .collect();
                // (min, nominal, max) kbit/s: the lowest limits aoTuV accepts here
                let lo = match (rate, ch) {
                    (44100, 1) => 28,
                    (44100, _) => 32,
                    (_, 1) => 8,
                    _ => 12,
                };
                for (min, nominal, max) in [
                    (-1, lo, -1),
                    (-1, -1, lo + 4),
                    (-1, -1, lo + 8),
                    (lo, lo, lo),
                    (-1, lo + 8, lo + 16),
                    (lo + 8, -1, -1),
                    // the template of a high nominal under a low maximum: blob 0 too big
                    (-1, 4 * lo, lo),
                    (-1, 4 * lo, 2 * lo),
                ] {
                    let s = |b: i64| if b > 0 { b * 1000 } else { b };
                    let mode = Mode::Managed {
                        min: s(min),
                        nominal: s(nominal),
                        max: s(max),
                    };
                    if let Some((p, _)) =
                        check(rate, mode, &pcm, &format!("{rate} Hz × {ch} noise"))
                    {
                        streams += 1;
                        packets_n += p;
                    }
                }
            }
        }
        let hits: Vec<usize> = HITS.with(|h| h.iter().map(|c| c.get()).collect());
        println!(
            "vorbis managed limits: {streams} streams, {packets_n} audio packets byte-identical; manager paths {hits:?}"
        );
        assert!(
            hits.iter().all(|&h| h > 0),
            "a bitrate manager path unreached: {hits:?}"
        );
    }
}
