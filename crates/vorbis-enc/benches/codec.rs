//! Encoder speed: Rust vs aoTuV b6.03 + Lancer as `vorbis_rs` builds it ("lancer": the cc
//! crate's `-O3`, Lancer's SSE paths; the speed bar), the strict oracle build ("strict") and
//! xiph's libvorbis 1.3.7 as its `configure` builds it ("xiph": `-O3 -ffast-math`), on
//! the ETSI LC3plus item thetest44 (`just lc3plus`; 44.1 kHz): `44m` mono, `44s` stereo (the
//! right channel the left delayed 331 samples × 0.75, so coupling works), VBR q 0.5, the whole
//! Ogg stream into memory, written in 1024-sample blocks.
//! Columns after Rust/Lancer (which `just bench`'s `AB=` reads) only append. `BENCH_MODE=44m|44s`: one input.
//! Run via `CRATE=vorbis-enc just bench` (pins one core). Prints ms per pass (best of BENCH_REPS,
//! default 15) and ×realtime.

use aotuv_sys::{Encoder as CEncoder, Mode, STRICT, lancer, spike_pcm, thetest, xiph};
use std::hint::black_box;
use std::num::{NonZeroU8, NonZeroU32};
use std::time::Instant;
use vorbis_enc::{BitrateManagementStrategy, EncoderBuilder};

const Q: f32 = 0.5;
const MODES: [(&str, usize); 2] = [("44m", 1), ("44s", 2)];

fn reps() -> usize {
    std::env::var("BENCH_REPS")
        .ok()
        .and_then(|r| r.parse().ok())
        .unwrap_or(15)
}

fn wanted(name: &str) -> bool {
    std::env::var("BENCH_MODE").map_or(true, |m| m == name)
}

fn best<F: FnMut()>(mut f: F) -> f64 {
    (0..reps())
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed().as_secs_f64()
        })
        .fold(f64::INFINITY, f64::min)
}

fn rust(rate: u32, pcm: &[Vec<f32>]) -> Vec<u8> {
    let b = EncoderBuilder::new(
        NonZeroU32::new(rate).unwrap(),
        NonZeroU8::new(pcm.len() as u8).unwrap(),
        Vec::new(),
    )
    .stream_serial(1)
    .bitrate_management_strategy(BitrateManagementStrategy::QualityVbr { target_quality: Q });
    let mut e = b.build().unwrap();
    for k in (0..pcm[0].len()).step_by(1024) {
        let m = (pcm[0].len() - k).min(1024);
        e.encode_audio_block(pcm.iter().map(|c| &c[k..k + m]))
            .unwrap();
    }
    e.finish().unwrap()
}

fn c(api: aotuv_sys::Api, rate: u32, pcm: &[Vec<f32>]) -> Vec<u8> {
    let mut e = CEncoder::new(api, pcm.len(), rate as i64, Mode::Vbr(Q)).unwrap();
    let mut s = e.headers().to_vec();
    for k in (0..pcm[0].len()).step_by(1024) {
        let m = (pcm[0].len() - k).min(1024);
        let chunk: Vec<&[f32]> = pcm.iter().map(|c| &c[k..k + m]).collect();
        s.extend(e.write(&chunk));
    }
    s.extend(e.finish());
    s
}

fn main() {
    if let Ok(m) = std::env::var("BENCH_MODE")
        && !MODES.iter().any(|p| p.0 == m)
    {
        eprintln!("bench: unknown BENCH_MODE '{m}' (44m 44s)");
        std::process::exit(2);
    }
    let which = std::env::var("BENCH").unwrap_or_default();
    if !["", "enc"].contains(&which.as_str()) {
        eprintln!("bench: unknown BENCH '{which}' (enc; the crate has no decoder)");
        std::process::exit(2);
    }
    let (rate, item) = thetest(44);
    let secs = item.len() as f64 / rate as f64;
    println!(
        "encoder {:>9} {:>9} {:>8} {:>9} {:>8} {:>8} {:>8} {:>9} {:>8}",
        "rust ms",
        "lancer ms",
        "speedup",
        "strict ms",
        "speedup",
        "rust ×rt",
        "lanc ×rt",
        "xiph ms",
        "speedup"
    );
    let (mut tot, mut n) = ([0.0; 4], 0.0);
    for (name, ch) in MODES.iter().filter(|m| wanted(m.0)) {
        let pcm = spike_pcm(&item, *ch);
        assert!(
            rust(rate, &pcm) == c(STRICT, rate, &pcm),
            "{name}: Rust differs from strict aoTuV"
        );
        let t = [
            best(|| {
                black_box(rust(rate, black_box(&pcm)));
            }),
            lancer::API.map_or(f64::NAN, |api| {
                best(|| {
                    black_box(c(api, rate, black_box(&pcm)));
                })
            }),
            best(|| {
                black_box(c(STRICT, rate, black_box(&pcm)));
            }),
            xiph::API.map_or(f64::NAN, |api| {
                best(|| {
                    black_box(c(api, rate, black_box(&pcm)));
                })
            }),
        ];
        row(name, t, secs);
        for i in 0..4 {
            tot[i] += t[i];
        }
        n += secs;
    }
    if std::env::var("BENCH_MODE").is_err() {
        row("all", tot, n);
        println!("({secs:.2} s of audio per input, best of {})", reps());
    }
}

/// One row: ms per pass of Rust, Lancer and strict, the speedups (C time / Rust time),
/// ×realtime of Rust and Lancer, then xiph's ms and speedup.
fn row(name: &str, t: [f64; 4], secs: f64) {
    let ms = t.map(|v| v * 1e3);
    println!(
        "{name:<7} {:>9.2} {:>9.2} {:>7.2}x {:>9.2} {:>7.2}x {:>7.1}x {:>7.1}x {:>9.2} {:>7.2}x",
        ms[0],
        ms[1],
        t[1] / t[0],
        ms[2],
        t[2] / t[0],
        secs / t[0],
        secs / t[1],
        ms[3],
        t[3] / t[0]
    );
}
