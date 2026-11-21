//! The encoder's input side (`block.c`): the PCM buffer (pre-amplified, extrapolated before the
//! first and after the last sample by LPC), and `vorbis_analysis_blockout`'s choice of the next
//! block size from the envelope's marks.

use crate::envelope::Envelope;
use crate::lpc::{lpc_from_data, lpc_predict};
use crate::setup::CodecSetup;

/// `BLOCKTYPE_*` (`codec_internal.h`): short blocks are impulse or padding, long ones
/// transition or long.
const BLOCKTYPE_IMPULSE: i32 = 0;
const BLOCKTYPE_PADDING: i32 = 1;
const BLOCKTYPE_TRANSITION: i32 = 0;
const BLOCKTYPE_LONG: i32 = 1;

/// One block to analyse (`vorbis_block` as blockout fills it).
#[derive(Clone, Debug)]
pub(crate) struct Block {
    pub lw: usize,
    pub w: usize,
    pub nw: usize,
    pub blocktype: i32,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the C's packetno: no packet carries it; the test reads it"
        )
    )]
    pub sequence: i64,
    pub granulepos: i64,
    pub pcmend: usize,
    pub eofflag: bool,
    /// `vbi->ampmax`: the decayed running peak (dB) the psy model starts from
    pub ampmax: f32,
    /// `pcmend` samples per channel
    pub pcm: Vec<Vec<f32>>,
}

/// `vorbis_dsp_state` (encoder side) with the parts of `private_state` blocking uses.
#[derive(Clone, Debug)]
pub(crate) struct Dsp {
    blocksizes: [i64; 2],
    pre_amplitude: f32,
    /// per channel; the C's sample `i` at `pcm[c][off + i]` (the C moves the buffer down by
    /// each block's advance: here `off` grows, and the buffer moves only once `off` passes the
    /// samples left, so a long write costs linear, not quadratic, time)
    pcm: Vec<Vec<f32>>,
    off: usize,
    pcm_current: i64,
    center_w: i64,
    lw: usize,
    w: usize,
    nw: usize,
    eofflag: i64,
    granulepos: i64,
    sequence: i64,
    preextrapolate: bool,
    ve: Envelope,
    /// `g->ampmax` (`vorbis_look_psy_global`)
    g_ampmax: f32,
    /// `vbi->ampmax` of the block in flight (the analysis may raise it)
    pub vbi_ampmax: f32,
}

impl Dsp {
    /// `vorbis_analysis_init` (the blocking state).
    pub(crate) fn new(ci: &CodecSetup) -> Self {
        let bs1 = ci.blocksizes[1];
        let ch = ci.channels as usize;
        Dsp {
            blocksizes: ci.blocksizes,
            pre_amplitude: ci.hi.pre_amplitude,
            pcm: vec![vec![0.0; bs1 as usize]; ch],
            off: 0,
            pcm_current: bs1 / 2,
            center_w: bs1 / 2,
            lw: 0,
            w: 0,
            nw: 0,
            eofflag: 0,
            granulepos: 0,
            sequence: 3,
            preextrapolate: false,
            ve: Envelope::new(ch, bs1, &ci.psy_g_param),
            g_ampmax: -9999.0,
            vbi_ampmax: -9999.0,
        }
    }

    /// Room for `n` more samples per channel from `pcm_current` (`vorbis_analysis_buffer`).
    fn reserve(&mut self, n: i64) {
        let need = self.off + (self.pcm_current + n) as usize;
        for c in &mut self.pcm {
            if c.len() < need {
                c.resize(need, 0.0);
            }
        }
    }

    /// `vorbis_analysis_buffer` + `vorbis_analysis_wrote(vals > 0)`: `pcm[c]` per channel.
    pub(crate) fn write(&mut self, pcm: &[&[f32]]) {
        let vals = pcm[0].len() as i64;
        debug_assert!(vals > 0, "the end of the stream is write_end");
        self.reserve(vals);
        let at = self.off + self.pcm_current as usize;
        for (dst, src) in self.pcm.iter_mut().zip(pcm) {
            for (d, &s) in dst[at..at + vals as usize].iter_mut().zip(src.iter()) {
                *d = s * self.pre_amplitude;
            }
        }
        self.pcm_current += vals;
        if !self.preextrapolate && self.pcm_current - self.center_w > self.blocksizes[1] {
            self.preextrapolate_helper();
        }
    }

    /// `vorbis_analysis_wrote(0)`: the end of the stream, extrapolated by LPC (order 32).
    pub(crate) fn write_end(&mut self) {
        const ORDER: usize = 32;
        if !self.preextrapolate {
            self.preextrapolate_helper();
        }
        let bs1 = self.blocksizes[1];
        self.reserve(bs1 * 3);
        self.eofflag = self.pcm_current;
        self.pcm_current += bs1 * 3;
        let eof = self.eofflag as usize;
        let cur = self.pcm_current as usize;
        for c in &mut self.pcm {
            let c = &mut c[self.off..];
            if eof > ORDER * 2 {
                let n = eof.min(bs1 as usize);
                let mut lpc = [0f32; ORDER];
                lpc_from_data(&c[eof - n..eof], &mut lpc, n, ORDER);
                let prime = c[eof - ORDER..eof].to_vec();
                lpc_predict(&lpc, Some(&prime), ORDER, &mut c[eof..cur], cur - eof);
            } else {
                c[eof..cur].fill(0.0);
            }
        }
    }

    /// `_preextrapolate_helper`: the samples before the first one, predicted backwards
    /// (order 16), so the first block sees no step.
    fn preextrapolate_helper(&mut self) {
        const ORDER: usize = 16;
        self.preextrapolate = true;
        let cur = self.pcm_current as usize;
        let cw = self.center_w as usize;
        if cur - cw > ORDER * 2 {
            for c in &mut self.pcm {
                let c = &mut c[self.off..];
                let mut work: Vec<f32> = (0..cur).map(|j| c[cur - j - 1]).collect();
                let mut lpc = [0f32; ORDER];
                lpc_from_data(&work, &mut lpc, cur - cw, ORDER);
                let (head, tail) = work.split_at_mut(cur - cw);
                lpc_predict(&lpc, Some(&head[cur - cw - ORDER..]), ORDER, tail, cw);
                for j in 0..cur {
                    c[cur - j - 1] = work[j];
                }
            }
        }
    }

    /// `vorbis_analysis_blockout`: the next block, once enough input decides its size.
    pub(crate) fn blockout(&mut self, ci: &CodecSetup) -> Option<Block> {
        let bs = self.blocksizes;
        let begin_w = self.center_w - bs[self.w] / 2;
        if !self.preextrapolate || self.eofflag == -1 {
            return None;
        }
        let pcm: Vec<&[f32]> = self.pcm.iter().map(|c| &c[self.off..]).collect();
        let bp = self.ve.search(
            &pcm,
            self.pcm_current,
            self.center_w,
            bs[self.w],
            bs,
            &ci.psy_g_param,
        );
        match bp {
            None if self.eofflag == 0 => return None,
            None => self.nw = 0,
            Some(_) if bs[0] == bs[1] => self.nw = 0,
            Some(nw) => self.nw = nw,
        }
        let center_next = self.center_w + bs[self.w] / 4 + bs[self.nw] / 4;
        if self.pcm_current < center_next + bs[self.nw] / 2 {
            return None;
        }
        let blocktype = if self.w != 0 {
            if self.lw == 0 || self.nw == 0 {
                BLOCKTYPE_TRANSITION
            } else {
                BLOCKTYPE_LONG
            }
        } else if self.ve.marked(self.center_w, bs, self.lw, self.w, self.nw) {
            BLOCKTYPE_IMPULSE
        } else {
            BLOCKTYPE_PADDING
        };
        let sequence = self.sequence;
        self.sequence += 1;
        let pcmend = bs[self.w] as usize;
        // _vp_ampmax_decay
        if self.vbi_ampmax > self.g_ampmax {
            self.g_ampmax = self.vbi_ampmax;
        }
        let secs = (bs[self.w] / 2) as f32 / ci.rate as f32;
        self.g_ampmax += secs * ci.psy_g_param.ampmax_att_per_sec;
        if self.g_ampmax < -9999.0 {
            self.g_ampmax = -9999.0;
        }
        self.vbi_ampmax = self.g_ampmax;
        let b0 = begin_w as usize;
        let mut block = Block {
            lw: self.lw,
            w: self.w,
            nw: self.nw,
            blocktype,
            sequence,
            granulepos: self.granulepos,
            pcmend,
            eofflag: false,
            ampmax: self.vbi_ampmax,
            pcm: self
                .pcm
                .iter()
                .map(|c| c[self.off + b0..self.off + b0 + pcmend].to_vec())
                .collect(),
        };
        if self.eofflag != 0 && self.center_w >= self.eofflag {
            self.eofflag = -1;
            block.eofflag = true;
            return Some(block);
        }
        let new_center_next = bs[1] / 2;
        let movement_w = center_next - new_center_next;
        if movement_w > 0 {
            self.ve.shift(movement_w);
            self.pcm_current -= movement_w;
            let (m, cur) = (movement_w as usize, self.pcm_current as usize);
            self.off += m;
            if self.off >= cur {
                for c in &mut self.pcm {
                    c.copy_within(self.off..self.off + cur, 0);
                }
                self.off = 0;
            }
            self.lw = self.w;
            self.w = self.nw;
            self.center_w = new_center_next;
            if self.eofflag != 0 {
                self.eofflag -= movement_w;
                if self.eofflag <= 0 {
                    self.eofflag = -1;
                }
                if self.center_w >= self.eofflag {
                    self.granulepos += movement_w - (self.center_w - self.eofflag);
                } else {
                    self.granulepos += movement_w;
                }
            } else {
                self.granulepos += movement_w;
            }
        }
        Some(block)
    }
}

#[cfg(all(test, oracle))]
mod vs_c {
    use super::*;
    use crate::testutil::Rng;
    use aotuv_sys::{Mode, spike_pcm, thetest};

    /// The Rust blocking over `pcm` in `chunks`, flattened as `aotuv_sys::blocks` logs it.
    fn blocks(ci: &CodecSetup, pcm: &[Vec<f32>], chunks: &[i64]) -> Vec<f64> {
        let mut d = Dsp::new(ci);
        let mut out = Vec::new();
        let n = pcm[0].len();
        let (mut at, mut k) = (0, 0);
        loop {
            let m = (n - at).min(chunks[k % chunks.len()] as usize);
            k += 1;
            let chunk: Vec<&[f32]> = pcm.iter().map(|c| &c[at..at + m]).collect();
            if m == 0 {
                d.write_end();
            } else {
                d.write(&chunk);
            }
            at += m;
            while let Some(b) = d.blockout(ci) {
                out.extend([
                    b.lw as f64,
                    b.w as f64,
                    b.nw as f64,
                    b.blocktype as f64,
                    b.sequence as f64,
                    b.granulepos as f64,
                    b.pcmend as f64,
                    b.eofflag as i32 as f64,
                    b.ampmax as f64,
                ]);
                for c in &b.pcm {
                    out.extend(c.iter().map(|&x| x as f64));
                }
                d.ve.dump(&mut out);
            }
            if m == 0 {
                break;
            }
        }
        out
    }

    fn check(rate: i64, mode: Mode, pcm: &[Vec<f32>], chunks: &[i64], what: &str) -> usize {
        let ci = match mode {
            Mode::Vbr(q) => CodecSetup::vbr(pcm.len() as i64, rate, q),
            Mode::Managed { min, nominal, max } => {
                CodecSetup::managed(pcm.len() as i64, rate, max, nominal, min)
            }
        }
        .unwrap();
        let c = aotuv_sys::blocks(rate, mode, pcm, chunks).unwrap();
        let r = blocks(&ci, pcm, chunks);
        if let Some(k) = r
            .iter()
            .zip(&c)
            .position(|(a, b)| a.to_bits() != b.to_bits())
        {
            panic!("{what}: value {k} differs: {} vs C {}", r[k], c[k]);
        }
        assert_eq!(r.len(), c.len(), "{what}: lengths");
        // count blocks: walk the records (header, pcm, envelope)
        let (mut i, mut n) = (0, 0);
        while i < r.len() {
            i += 9 + pcm.len() * r[i + 6] as usize;
            i += 5 + r[i + 4] as usize + pcm.len() * crate::templates::VE_BANDS * (17 + 15 + 4);
            n += 1;
        }
        n
    }

    /// The spike's inputs and other rates/channels/qualities, written in 1024-sample chunks,
    /// in random chunk sizes and in one write; plus short streams (shorter than a block, empty), impulses
    /// in silence (attacks: short blocks) and noise: every block's sizes, type, sequence,
    /// granule position, end flag, ampmax and samples bit for bit, and the envelope's state
    /// after each (marks, cursors, every band filter's buffers).
    #[test]
    fn blocks_match_c() {
        let mut r = Rng::new(17);
        let mut n = 0;
        let rand_chunks: Vec<i64> = (0..37).map(|_| 1 + (r.u64() % 3000) as i64).collect();
        for (khz, ch, q) in [
            (16, 1, 0.5),
            (44, 1, 0.5),
            (48, 1, 0.0),
            (44, 2, 0.5),
            (8, 2, 0.3),
            (96, 1, 0.9),
        ] {
            let (rate, pcm) = thetest(khz);
            let pcm = spike_pcm(&pcm, ch);
            n += check(
                rate as i64,
                Mode::Vbr(q),
                &pcm,
                &[1024],
                &format!("thetest{khz}×{ch}"),
            );
            n += check(
                rate as i64,
                Mode::Vbr(q),
                &pcm,
                &rand_chunks,
                &format!("thetest{khz}×{ch} random chunks"),
            );
            // the whole item in one write: the buffer moves rarely, not once per block
            n += check(
                rate as i64,
                Mode::Vbr(q),
                &pcm,
                &[pcm[0].len().max(1) as i64],
                &format!("thetest{khz}×{ch} one write"),
            );
        }
        // synthetic: clicks in silence, a burst, noise; short and empty streams
        for (len, kind) in [
            (0usize, 0),
            (17, 2),
            (100, 2),
            (3000, 2),
            (200_000, 0),
            (200_000, 1),
            (100_000, 2),
        ] {
            for ch in [1usize, 2, 6] {
                let pcm: Vec<Vec<f32>> = (0..ch)
                    .map(|c| {
                        (0..len)
                            .map(|i| match kind {
                                0 => {
                                    if i % (7919 + c * 13) < 3 {
                                        0.9
                                    } else {
                                        0.0
                                    }
                                }
                                1 => {
                                    if (i / 20000) % 2 == 1 {
                                        r.unit()
                                    } else {
                                        r.unit() * 1e-4
                                    }
                                }
                                _ => r.unit() * 0.5,
                            })
                            .collect()
                    })
                    .collect();
                let rate = [8000i64, 44100, 48000][ch % 3];
                n += check(
                    rate,
                    Mode::Vbr(0.4),
                    &pcm,
                    &rand_chunks,
                    &format!("synthetic {kind} len {len} × {ch}"),
                );
            }
        }
        println!("vorbis blocking: {n} blocks bit-identical");
    }
}
