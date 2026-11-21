//! The three header packets (`info.c`): identification, comment and setup (codebooks, floors,
//! residues, mappings, modes; `vorbis_staticbook_pack`, `floor1_pack`, `res0_pack`,
//! `mapping0_pack`).
#![allow(clippy::needless_range_loop)] // index loops mirror the C code 1:1

use crate::bitpack::{OggPack, ilog};
use crate::setup::CodecSetup;
use crate::templates::{InfoFloor1, InfoMapping0, InfoResidue0, StaticCodebook};

/// `ENCODE_VENDOR_STRING` of aoTuV built without Lancer.
const VENDOR: &str = "AO; aoTuV [20110424] (based on libvorbis 1.3.7)";

/// `_vorbis_pack_info`.
fn pack_info(ci: &CodecSetup) -> Vec<u8> {
    let mut o = OggPack::new();
    o.write(0x01, 8);
    o.write_bytes(b"vorbis");
    o.write(0x00, 32);
    o.write(ci.channels as i64, 8);
    o.write(ci.rate, 32);
    o.write(ci.bitrate_upper, 32);
    o.write(ci.bitrate_nominal, 32);
    o.write(ci.bitrate_lower, 32);
    o.write(ilog(ci.blocksizes[0] as u32 - 1) as i64, 4);
    o.write(ilog(ci.blocksizes[1] as u32 - 1) as i64, 4);
    o.write(1, 1);
    o.buffer().to_vec()
}

/// `_vorbis_pack_comment`: the vendor string and the user comments (`TAG=value`).
pub(crate) fn pack_comment(comments: &[&[u8]]) -> Vec<u8> {
    let mut o = OggPack::new();
    o.write(0x03, 8);
    o.write_bytes(b"vorbis");
    o.write(VENDOR.len() as i64, 32);
    o.write_bytes(VENDOR.as_bytes());
    o.write(comments.len() as i64, 32);
    for c in comments {
        o.write(c.len() as i64, 32);
        o.write_bytes(c);
    }
    o.write(1, 1);
    o.buffer().to_vec()
}

/// `_vorbis_pack_books`: the setup header.
fn pack_books(ci: &CodecSetup) -> Vec<u8> {
    let mut o = OggPack::new();
    o.write(0x05, 8);
    o.write_bytes(b"vorbis");
    o.write(ci.books.len() as i64 - 1, 8);
    for b in &ci.books {
        staticbook_pack(b, &mut o);
    }
    // times: one placeholder
    o.write(0, 6);
    o.write(0, 16);
    o.write(ci.floors.len() as i64 - 1, 6);
    for f in &ci.floors {
        o.write(1, 16);
        floor1_pack(f, &mut o);
    }
    o.write(ci.residues.len() as i64 - 1, 6);
    for (t, r) in &ci.residues {
        o.write(*t as i64, 16);
        res0_pack(r, &mut o);
    }
    o.write(ci.maps.len() as i64 - 1, 6);
    for m in &ci.maps {
        o.write(0, 16);
        mapping0_pack(ci.channels, m, &mut o);
    }
    o.write(ci.modes.len() as i64 - 1, 6);
    for m in &ci.modes {
        o.write(m.blockflag as i64, 1);
        o.write(m.windowtype as i64, 16);
        o.write(m.transformtype as i64, 16);
        o.write(m.mapping as i64, 8);
    }
    o.write(1, 1);
    o.buffer().to_vec()
}

/// `_book_maptype1_quantvals`: the largest `vals` with `vals^dim ≤ entries`.
pub(crate) fn book_maptype1_quantvals(b: &StaticCodebook) -> i64 {
    if b.entries < 1 {
        return 0;
    }
    // `floor(pow((float)entries, 1.f/dim))`; the loop below corrects any rounding
    let mut vals = ((b.entries as f32 as f64).powf((1.0f32 / b.dim as f32) as f64)).floor() as i64;
    if vals < 1 {
        vals = 1;
    }
    loop {
        let mut acc: i64 = 1;
        let mut acc1: i64 = 1;
        let mut i = 0;
        while i < b.dim {
            if b.entries / vals < acc {
                break;
            }
            acc *= vals;
            if i64::MAX / (vals + 1) < acc1 {
                acc1 = i64::MAX;
            } else {
                acc1 *= vals + 1;
            }
            i += 1;
        }
        if i >= b.dim && acc <= b.entries && acc1 > b.entries {
            return vals;
        }
        if i < b.dim || acc > b.entries {
            vals -= 1;
        } else {
            vals += 1;
        }
    }
}

/// `vorbis_staticbook_pack`.
fn staticbook_pack(c: &StaticCodebook, o: &mut OggPack) {
    let len = c.lengthlist;
    let entries = c.entries as usize;
    o.write(0x564342, 24);
    o.write(c.dim, 16);
    o.write(c.entries, 24);
    let mut i = 1;
    while i < entries {
        if len[i - 1] == 0 || len[i] < len[i - 1] {
            break;
        }
        i += 1;
    }
    if i == entries {
        // ordered: run lengths of each codeword length
        let mut count = 0usize;
        o.write(1, 1);
        o.write(len[0] as i64 - 1, 5);
        let mut i = 1;
        while i < entries {
            let (this, last) = (len[i], len[i - 1]);
            if this > last {
                for _ in last..this {
                    o.write((i - count) as i64, ilog((entries - count) as u32));
                    count = i;
                }
            }
            i += 1;
        }
        o.write((i - count) as i64, ilog((entries - count) as u32));
    } else {
        o.write(0, 1);
        if len[..entries].iter().all(|&l| l != 0) {
            o.write(0, 1);
            for &l in &len[..entries] {
                o.write(l as i64 - 1, 5);
            }
        } else {
            o.write(1, 1);
            for &l in &len[..entries] {
                if l == 0 {
                    o.write(0, 1);
                } else {
                    o.write(1, 1);
                    o.write(l as i64 - 1, 5);
                }
            }
        }
    }
    o.write(c.maptype as i64, 4);
    match c.maptype {
        0 => {}
        1 | 2 => {
            o.write(c.q_min, 32);
            o.write(c.q_delta, 32);
            o.write(c.q_quant as i64 - 1, 4);
            o.write(c.q_sequencep as i64, 1);
            let quantvals = if c.maptype == 1 {
                book_maptype1_quantvals(c)
            } else {
                c.entries * c.dim
            };
            for &q in &c.quantlist[..quantvals as usize] {
                o.write(q.abs(), c.q_quant as u32);
            }
        }
        _ => unreachable!("maptype {}", c.maptype),
    }
}

/// `floor1_pack`.
fn floor1_pack(info: &InfoFloor1, o: &mut OggPack) {
    let maxposit = info.postlist[1];
    let mut maxclass = -1;
    o.write(info.partitions as i64, 5);
    for &c in &info.partitionclass[..info.partitions as usize] {
        o.write(c as i64, 4);
        maxclass = maxclass.max(c);
    }
    for j in 0..(maxclass + 1) as usize {
        o.write(info.class_dim[j] as i64 - 1, 3);
        o.write(info.class_subs[j] as i64, 2);
        if info.class_subs[j] != 0 {
            o.write(info.class_book[j] as i64, 8);
        }
        for k in 0..1usize << info.class_subs[j] {
            o.write(info.class_subbook[j][k] as i64 + 1, 8);
        }
    }
    o.write(info.mult as i64 - 1, 2);
    let rangebits = ilog((maxposit - 1) as u32);
    o.write(rangebits as i64, 4);
    let (mut count, mut k) = (0, 0);
    for &c in &info.partitionclass[..info.partitions as usize] {
        count += info.class_dim[c as usize] as usize;
        while k < count {
            o.write(info.postlist[k + 2] as i64, rangebits);
            k += 1;
        }
    }
}

/// `res0_pack`.
fn res0_pack(info: &InfoResidue0, o: &mut OggPack) {
    let mut acc = 0;
    o.write(info.begin, 24);
    o.write(info.end, 24);
    o.write(info.grouping as i64 - 1, 24);
    o.write(info.partitions as i64 - 1, 6);
    o.write(info.groupbook as i64, 8);
    for &s in &info.secondstages[..info.partitions as usize] {
        if ilog(s as u32) > 3 {
            o.write(s as i64, 3);
            o.write(1, 1);
            o.write((s >> 3) as i64, 5);
        } else {
            o.write(s as i64, 4);
        }
        acc += (s as u32).count_ones() as usize;
    }
    for &b in &info.booklist[..acc] {
        o.write(b as i64, 8);
    }
}

/// `mapping0_pack`.
fn mapping0_pack(channels: i32, info: &InfoMapping0, o: &mut OggPack) {
    if info.submaps > 1 {
        o.write(1, 1);
        o.write(info.submaps as i64 - 1, 4);
    } else {
        o.write(0, 1);
    }
    let chbits = ilog(channels as u32 - 1);
    if info.coupling_steps > 0 {
        o.write(1, 1);
        o.write(info.coupling_steps as i64 - 1, 8);
        for i in 0..info.coupling_steps as usize {
            o.write(info.coupling_mag[i] as i64, chbits);
            o.write(info.coupling_ang[i] as i64, chbits);
        }
    } else {
        o.write(0, 1);
    }
    o.write(0, 2);
    if info.submaps > 1 {
        for &c in &info.chmuxlist[..channels as usize] {
            o.write(c as i64, 4);
        }
    }
    for i in 0..info.submaps as usize {
        o.write(0, 8);
        o.write(info.floorsubmap[i] as i64, 8);
        o.write(info.residuesubmap[i] as i64, 8);
    }
}

/// `vorbis_analysis_headerout` without user comments: the three header packets.
pub(crate) fn headers(ci: &CodecSetup) -> [Vec<u8>; 3] {
    [pack_info(ci), pack_comment(&[]), pack_books(ci)]
}

#[cfg(all(test, oracle))]
mod vs_c {
    use super::*;
    use crate::setup::SetupError;
    use crate::testutil::Rng;
    use aotuv_sys::Mode;

    /// The C's error codes (`OV_EINVAL`, `OV_EIMPL`).
    fn code(e: SetupError) -> i32 {
        match e {
            SetupError::Invalid => -131,
            SetupError::Unsupported => -130,
        }
    }

    fn check(ch: usize, rate: i64, mode: Mode) -> bool {
        let c = aotuv_sys::headers(ch, rate, mode);
        let r = match mode {
            Mode::Vbr(q) => CodecSetup::vbr(ch as i64, rate, q),
            Mode::Managed { min, nominal, max } => {
                CodecSetup::managed(ch as i64, rate, max, nominal, min)
            }
        };
        match (r, c) {
            (Ok(ci), Ok(h)) => {
                let rh = headers(&ci);
                for k in 0..3 {
                    assert!(rh[k] == h[k], "header {k}: {ch} ch {rate} Hz {mode:?}");
                }
                let d = aotuv_sys::setup_dump(ch, rate, mode).unwrap();
                let rd = ci.dump();
                assert_eq!(
                    rd.len(),
                    d.len(),
                    "setup dump length: {ch} ch {rate} Hz {mode:?}"
                );
                for (k, (a, b)) in rd.iter().zip(&d).enumerate() {
                    assert!(
                        a.to_bits() == b.to_bits(),
                        "setup value {k}: {a} vs C {b}: {ch} ch {rate} Hz {mode:?}"
                    );
                }
                true
            }
            (Err(e), Err(ce)) => {
                assert_eq!(code(e), ce, "{ch} ch {rate} Hz {mode:?}");
                false
            }
            (r, c) => panic!(
                "{ch} ch {rate} Hz {mode:?}: Rust {:?}, C {:?}",
                r.map(|_| ()),
                c.map(|_| ())
            ),
        }
    }

    /// Every template's rate range (and its edges), 1–8 channels, quality −0.2…1.05 in steps of
    /// 0.05 plus random qualities, managed bitrates (nominal, min/max only) per channel: the
    /// three header packets byte-identical and the rest of the setup (psy parameters, lowpass,
    /// bitrate state: `CodecSetup::dump`) bit-identical, or the same rejection.
    #[test]
    fn headers_match_c() {
        let rates = [
            1, 7999, 8000, 9000, 11025, 12000, 14999, 15000, 16000, 19000, 22050, 24000, 26000,
            32000, 38000, 40000, 44100, 45000, 46000, 48000, 50000, 64000, 88200, 96000, 192000,
            200000, 200001,
        ];
        let mut r = Rng::new(15);
        let (mut ok, mut rejected) = (0, 0);
        for &rate in &rates {
            for ch in 1..=8 {
                let mut modes: Vec<Mode> = (-4..=21).map(|k| Mode::Vbr(k as f32 * 0.05)).collect();
                modes.extend((0..8).map(|_| Mode::Vbr(r.unit() * 0.7 + 0.35)));
                for kbps in [8, 16, 24, 32, 45, 64, 96, 128, 160, 192, 256, 320, 500] {
                    let b = kbps * 1000 * ch as i64 / 2;
                    modes.push(Mode::Managed {
                        min: -1,
                        nominal: b,
                        max: -1,
                    });
                    modes.push(Mode::Managed {
                        min: b / 2,
                        nominal: -1,
                        max: b * 2,
                    });
                    modes.push(Mode::Managed {
                        min: -1,
                        nominal: -1,
                        max: b,
                    });
                    modes.push(Mode::Managed {
                        min: b,
                        nominal: -1,
                        max: -1,
                    });
                }
                for m in modes {
                    if check(ch, rate, m) {
                        ok += 1;
                    } else {
                        rejected += 1;
                    }
                }
            }
        }
        assert!(ok > 2000, "only {ok} configurations accepted");
        println!("vorbis headers: {ok} configurations byte-identical, {rejected} rejected alike");
    }

    /// Every template at its own steps: VBR at each quality step (and just below and above
    /// it), managed at each bitrate step × channels (the top step gives `base_setting =
    /// mappings - 0.001`), at the template's lowest, a middle and its highest rate.
    #[test]
    fn headers_at_template_boundaries() {
        let mut n = 0;
        for t in crate::setup_tables::SETUP_LIST.iter() {
            let chs: Vec<usize> = match t.coupling_restriction {
                -1 => vec![1, 3],
                c => vec![c as usize],
            };
            let (lo, hi) = (t.samplerate_min_restriction, t.samplerate_max_restriction);
            for rate in [lo.max(1), (lo + hi) / 2, hi] {
                for &ch in &chs {
                    for k in 0..=t.mappings as usize {
                        let q = t.quality_mapping[k] as f32;
                        for m in [q, q - 1e-6, q + 1e-6] {
                            n += check(ch, rate, Mode::Vbr(m)) as usize;
                        }
                        let b = t.rate_mapping[k];
                        if b > 0.0 {
                            let nominal = (b * ch as f64) as i64;
                            for nom in [nominal - 1, nominal, nominal + 1] {
                                n += check(
                                    ch,
                                    rate,
                                    Mode::Managed {
                                        min: -1,
                                        nominal: nom,
                                        max: -1,
                                    },
                                ) as usize;
                            }
                        }
                    }
                }
            }
        }
        println!("vorbis headers at template steps: {n} configurations byte-identical");
    }
}
