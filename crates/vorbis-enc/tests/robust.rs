//! Hostile input through the public API (the test profile keeps overflow checks on): samples far
//! out of range (up to f32::MAX), NaN, ±inf and subnormals never panic, and the stream decodes with
//! lewton; while every sample stays within ±2^20 it is still byte-identical with aoTuV.

use aotuv_sys::{Encoder as CEncoder, Mode, STRICT};
use lewton::inside_ogg::OggStreamReader;
use std::cell::RefCell;
use std::io::Cursor;
use std::num::{NonZeroU8, NonZeroU32};
use std::rc::Rc;
use vorbis_enc::{BitrateManagementStrategy, EncoderBuilder};

/// (rate, channels, quality): the configurations whose integer paths overflowed first.
const CONFIGS: [(u32, u8, f32); 4] = [
    (8000, 1, 0.0),
    (22050, 2, -0.1),
    (44100, 2, 1.0),
    (48000, 6, 0.5),
];

/// `ch` channels of `n` samples: xorshift noise at `amp`, every 97th sample replaced by `spike`.
fn signal(ch: usize, n: usize, amp: f32, spike: f32) -> Vec<Vec<f32>> {
    let mut s = 0x9e37_79b9_7f4a_7c15u64;
    (0..ch)
        .map(|_| {
            (0..n)
                .map(|j| {
                    s ^= s << 13;
                    s ^= s >> 7;
                    s ^= s << 17;
                    if j % 97 == 96 {
                        spike
                    } else {
                        amp * ((s >> 40) as f32 / (1 << 23) as f32 - 1.0)
                    }
                })
                .collect()
        })
        .collect()
}

fn encode(rate: u32, ch: u8, q: f32, pcm: &[Vec<f32>]) -> Vec<u8> {
    let b = EncoderBuilder::new(
        NonZeroU32::new(rate).unwrap(),
        NonZeroU8::new(ch).unwrap(),
        Vec::new(),
    )
    .stream_serial(1)
    .bitrate_management_strategy(BitrateManagementStrategy::QualityVbr { target_quality: q });
    let mut e = b.build().unwrap();
    for k in (0..pcm[0].len()).step_by(4096) {
        e.encode_audio_block(pcm.iter().map(|c| &c[k..(k + 4096).min(c.len())]))
            .unwrap();
    }
    e.finish().unwrap()
}

fn decodes(ogg: &[u8], ch: u8, rate: u32) {
    let mut r = OggStreamReader::new(Cursor::new(ogg)).unwrap();
    assert_eq!(
        (r.ident_hdr.audio_channels, r.ident_hdr.audio_sample_rate),
        (ch, rate)
    );
    while r.read_dec_packet_itl().unwrap().is_some() {}
}

#[test]
fn out_of_range_and_non_finite_samples_encode() {
    for (rate, ch, q) in CONFIGS {
        for (amp, spike) in [
            (1e6, 0.0),
            (1e10, 0.0),
            (f32::MAX, f32::MIN),
            (0.5, f32::NAN),
            (0.5, f32::INFINITY),
            (0.5, f32::NEG_INFINITY),
            (1e-40, 1e-45),
            (f32::NAN, f32::NAN),
        ] {
            let pcm = signal(ch.into(), 3 * 4096, amp, spike);
            decodes(&encode(rate, ch, q, &pcm), ch, rate);
        }
    }
}

#[test]
fn far_out_of_range_still_matches_c() {
    for (rate, ch, q) in CONFIGS {
        for amp in [3e5, (1 << 20) as f32] {
            let pcm = signal(ch.into(), 3 * 4096, amp, -amp);
            let mut c = CEncoder::new(STRICT, ch.into(), rate.into(), Mode::Vbr(q)).unwrap();
            let mut want = c.headers().to_vec();
            for k in (0..pcm[0].len()).step_by(4096) {
                let refs: Vec<&[f32]> =
                    pcm.iter().map(|v| &v[k..(k + 4096).min(v.len())]).collect();
                want.extend(c.write(&refs));
            }
            want.extend(c.finish());
            assert!(
                encode(rate, ch, q, &pcm) == want,
                "{rate} Hz × {ch} q {q} amp {amp}"
            );
        }
    }
}

/// A sink that fails its `fail`-th write or its first flush (`fail == 0`), then works again;
/// what it took stays readable after the encoder drops it.
struct Flaky {
    out: Rc<RefCell<Vec<u8>>>,
    writes: usize,
    fail: usize,
    flushed: bool,
}

impl std::io::Write for Flaky {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.writes += 1;
        if self.writes == self.fail {
            return Err(std::io::ErrorKind::WouldBlock.into());
        }
        self.out.borrow_mut().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if self.fail == 0 && !self.flushed {
            self.flushed = true;
            return Err(std::io::Error::other("flush"));
        }
        Ok(())
    }
}

/// The number of end-of-stream pages (header type bit 2).
fn eos_pages(ogg: &[u8]) -> usize {
    let (mut n, mut i) = (0, 0);
    while i + 27 <= ogg.len() {
        assert_eq!(&ogg[i..i + 4], b"OggS");
        let segs = usize::from(ogg[i + 26]);
        let body: usize = ogg[i + 27..i + 27 + segs]
            .iter()
            .map(|&s| usize::from(s))
            .sum();
        n += usize::from(ogg[i + 5] & 4 != 0);
        i += 27 + segs + body;
    }
    assert_eq!(i, ogg.len());
    n
}

fn flaky_encoder(fail: usize) -> (vorbis_enc::Encoder<Flaky>, Rc<RefCell<Vec<u8>>>) {
    let out = Rc::new(RefCell::new(Vec::new()));
    let sink = Flaky {
        out: out.clone(),
        writes: 0,
        fail,
        flushed: false,
    };
    let e = EncoderBuilder::new(
        NonZeroU32::new(44100).unwrap(),
        NonZeroU8::new(2).unwrap(),
        sink,
    )
    .stream_serial(1)
    .build()
    .unwrap();
    (e, out)
}

#[test]
fn failed_finish_ends_the_stream_once() {
    let (mut e, out) = flaky_encoder(0);
    e.encode_audio_block(signal(2, 30000, 0.5, 0.0)).unwrap();
    assert!(matches!(e.finish(), Err(vorbis_enc::Error::Io(_))));
    assert_eq!(eos_pages(&out.borrow()), 1);
}

#[test]
fn dropping_ends_the_stream() {
    let (mut e, out) = flaky_encoder(usize::MAX);
    e.encode_audio_block(signal(2, 30000, 0.5, 0.0)).unwrap();
    drop(e);
    assert_eq!(eos_pages(&out.borrow()), 1);
}

#[test]
fn a_sink_error_is_sticky() {
    let (mut e, out) = flaky_encoder(5);
    let pcm = signal(2, 4096, 0.5, 0.0);
    let mut failed = false;
    for _ in 0..40 {
        let r = e.encode_audio_block(&pcm);
        if failed {
            assert!(
                matches!(r, Err(vorbis_enc::Error::StreamBroken)),
                "after a sink error: {r:?}"
            );
        } else if r.is_err() {
            assert!(matches!(r, Err(vorbis_enc::Error::Io(_))));
            failed = true;
        }
    }
    assert!(failed);
    assert!(matches!(e.finish(), Err(vorbis_enc::Error::StreamBroken)));
    assert_eq!(eos_pages(&out.borrow()), 0, "a broken stream is not ended");
}

#[test]
fn mismatched_blocks_are_rejected() {
    let (mut e, _) = flaky_encoder(usize::MAX);
    let a = [0.25f32; 100];
    assert!(matches!(
        e.encode_audio_block([&a[..]]),
        Err(vorbis_enc::Error::ChannelMismatch {
            expected: 2,
            actual: 1
        })
    ));
    assert!(matches!(
        e.encode_audio_block([&a[..], &a[..50]]),
        Err(vorbis_enc::Error::LengthMismatch)
    ));
    e.encode_audio_block([&a[..], &a[..]]).unwrap();
    e.finish().unwrap();
}

#[test]
fn a_failed_build_writes_nothing_more() {
    // the sink fails on the header pages' first or second write: `build` errs, and the
    // dropped encoder must not end the stream into the sink anyway
    for fail in [1, 2] {
        let out = Rc::new(RefCell::new(Vec::new()));
        let sink = Flaky {
            out: out.clone(),
            writes: 0,
            fail,
            flushed: false,
        };
        let r = EncoderBuilder::new(
            NonZeroU32::new(44100).unwrap(),
            NonZeroU8::new(2).unwrap(),
            sink,
        )
        .build();
        assert!(matches!(r, Err(vorbis_enc::Error::Io(_))));
        assert_eq!(eos_pages(&out.borrow()), 0, "fail at write {fail}");
    }
}

/// A sink that panics on its `n`-th write.
struct Panicky(usize);

impl std::io::Write for Panicky {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0 -= 1;
        assert!(self.0 != 0, "sink panic");
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_sink_panic_breaks_the_stream() {
    let mut e = EncoderBuilder::new(
        NonZeroU32::new(44100).unwrap(),
        NonZeroU8::new(2).unwrap(),
        Panicky(6),
    )
    .build()
    .unwrap();
    let pcm = signal(2, 4096, 0.5, 0.0);
    let mut panicked = false;
    for _ in 0..40 {
        let r =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| e.encode_audio_block(&pcm)));
        panicked |= r.is_err();
        if panicked && let Ok(r) = r {
            assert!(r.is_err(), "Ok after a sink panic: a page is lost");
        }
    }
    assert!(panicked);
    assert!(e.finish().is_err());
}

#[test]
fn invalid_arguments_are_rejected() {
    let b = || {
        EncoderBuilder::new(
            NonZeroU32::new(44100).unwrap(),
            NonZeroU8::new(2).unwrap(),
            Vec::new(),
        )
    };
    let vbr = |q| BitrateManagementStrategy::QualityVbr { target_quality: q };
    for q in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(matches!(
            b().bitrate_management_strategy(vbr(q)).build(),
            Err(vorbis_enc::Error::InvalidQuality)
        ));
    }
    assert!(b().bitrate_management_strategy(vbr(-0.2)).build().is_ok());
    assert!(matches!(
        b().bitrate_management_strategy(vbr(-0.21)).build(),
        Err(vorbis_enc::Error::Unsupported)
    ));
    for tag in ["", "A=B", "ÄRTIST", "NUL\0", "~"] {
        assert!(
            matches!(
                b().comment_tag(tag, "v").build(),
                Err(vorbis_enc::Error::InvalidCommentTag)
            ),
            "{tag:?}"
        );
    }
    assert!(
        b().comment_tag("TITLE", "ä=b")
            .comment_tag(" !}", "")
            .build()
            .is_ok()
    );
    let none = BitrateManagementStrategy::Managed {
        minimum_bitrate: None,
        average_bitrate: None,
        maximum_bitrate: None,
    };
    assert!(matches!(
        b().bitrate_management_strategy(none).build(),
        Err(vorbis_enc::Error::MissingBitrate)
    ));
}

/// `Abr` and `ConstrainedAbr` are `Managed` with only the average or only the maximum set (the
/// C oracle tests take `Managed`), byte for byte.
#[test]
fn abr_strategies_are_managed_ones() {
    let pcm = signal(2, 44100, 0.5, 0.0);
    let enc = |s| {
        let mut e = EncoderBuilder::new(
            NonZeroU32::new(44100).unwrap(),
            NonZeroU8::new(2).unwrap(),
            Vec::new(),
        )
        .stream_serial(1)
        .bitrate_management_strategy(s)
        .build()
        .unwrap();
        e.encode_audio_block(&pcm).unwrap();
        e.finish().unwrap()
    };
    let br = NonZeroU32::new(128_000).unwrap();
    let managed = |avg, max| BitrateManagementStrategy::Managed {
        minimum_bitrate: None,
        average_bitrate: avg,
        maximum_bitrate: max,
    };
    use BitrateManagementStrategy::{Abr, ConstrainedAbr};
    let abr = enc(Abr {
        average_bitrate: br,
    });
    assert_eq!(abr, enc(managed(Some(br), None)));
    let cabr = enc(ConstrainedAbr {
        maximum_bitrate: br,
    });
    assert_eq!(cabr, enc(managed(None, Some(br))));
    assert_ne!(abr, cabr);
}

#[test]
fn drop_while_unwinding_writes_nothing() {
    // a panic in the caller's code with a healthy encoder alive: `Drop` must not end the
    // stream while unwinding (the encoder's state may be mid-update)
    let out = Rc::new(RefCell::new(Vec::new()));
    let sink = Flaky {
        out: out.clone(),
        writes: 0,
        fail: usize::MAX,
        flushed: false,
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut e = EncoderBuilder::new(
            NonZeroU32::new(44100).unwrap(),
            NonZeroU8::new(2).unwrap(),
            sink,
        )
        .build()
        .unwrap();
        e.encode_audio_block(signal(2, 30000, 0.5, 0.0)).unwrap();
        panic!("user code");
    }));
    assert!(r.is_err());
    assert!(!out.borrow().is_empty());
    assert_eq!(eos_pages(&out.borrow()), 0);
}

#[test]
fn errors_display_and_chain() {
    use std::error::Error as _;
    let io = vorbis_enc::Error::from(std::io::Error::other("disk full"));
    assert_eq!(io.to_string(), "writing to the sink failed");
    assert_eq!(io.source().unwrap().to_string(), "disk full");
    let bad = vorbis_enc::Error::ChannelMismatch {
        expected: 2,
        actual: 1,
    };
    assert_eq!(bad.to_string(), "block has 1 channels, the encoder 2");
    assert!(bad.source().is_none());
}

#[test]
fn default_serials_differ() {
    let serial = || {
        let e = EncoderBuilder::new(
            NonZeroU32::new(44100).unwrap(),
            NonZeroU8::new(1).unwrap(),
            Vec::new(),
        )
        .build()
        .unwrap();
        let ogg = e.finish().unwrap();
        u32::from_le_bytes(ogg[14..18].try_into().unwrap())
    };
    let first = serial();
    assert!((0..4).any(|_| serial() != first));
}

#[test]
fn debug_is_short() {
    let e = EncoderBuilder::new(
        NonZeroU32::new(44100).unwrap(),
        NonZeroU8::new(2).unwrap(),
        Vec::new(),
    )
    .build()
    .unwrap();
    assert!(format!("{e:?}").len() < 100);
}
