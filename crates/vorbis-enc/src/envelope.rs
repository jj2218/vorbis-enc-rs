//! The pre-echo envelope (`envelope.c`, aoTuV's 12 bands): a 128-point MDCT every 64 samples
//! per channel, band energies against their recent maxima and minima, marks where an attack
//! (or release) needs short blocks. In the C's arithmetic (its `double` constants included).

use crate::mdct::Mdct;
use crate::scales::{fmax, fmin, todb};
use crate::templates::{InfoPsyGlobal, VE_BANDS};

const VE_PRE: usize = 16;
const VE_WIN: i64 = 4;
const VE_POST: i64 = 2;
const VE_AMP: usize = VE_PRE + VE_POST as usize - 1;
const VE_NEARDC: usize = 15;
const VE_MINSTRETCH: i32 = 2;
const VE_MAXSTRETCH: i32 = 12;

const BAND_BEGIN: [usize; VE_BANDS] = [2, 4, 6, 9, 13, 17, 22, 12, 8, 3, 2, 1];
const BAND_END: [usize; VE_BANDS] = [4, 5, 6, 8, 8, 8, 8, 4, 4, 3, 2, 4];

/// `envelope_filter_state`.
#[derive(Clone, Debug, Default)]
struct FilterState {
    ampbuf: [f32; VE_AMP],
    ampptr: usize,
    near_dc: [f32; VE_NEARDC],
    near_dc_acc: f32,
    near_dc_partialacc: f32,
    nearptr: usize,
}

/// `envelope_band`.
#[derive(Clone, Debug)]
struct Band {
    begin: usize,
    window: Vec<f32>,
    total: f32,
}

/// `envelope_lookup`.
#[derive(Clone, Debug)]
pub(crate) struct Envelope {
    ch: usize,
    winlength: usize,
    searchstep: i64,
    minenergy: f32,
    mdct: Mdct,
    mdct_win: Vec<f32>,
    band: Vec<Band>,
    filter: Vec<FilterState>,
    stretch: i32,
    mark: Vec<i32>,
    current: i64,
    curmark: i64,
    cursor: i64,
}

impl Envelope {
    /// `_ve_envelope_init`.
    pub(crate) fn new(ch: usize, blocksize1: i64, gi: &InfoPsyGlobal) -> Self {
        let n = 128;
        let mdct_win = (0..n)
            .map(|i| {
                let t = (i as f64 / (n as f64 - 1.) * std::f64::consts::PI).sin() as f32;
                t * t
            })
            .collect();
        let band = (0..VE_BANDS)
            .map(|j| {
                let n = BAND_END[j];
                let window: Vec<f32> = (0..n)
                    .map(|i| ((i as f64 + 0.5) / n as f64 * std::f64::consts::PI).sin() as f32)
                    .collect();
                let mut total = 0f32;
                for &w in &window {
                    total += w;
                }
                Band {
                    begin: BAND_BEGIN[j],
                    window,
                    total: (1. / total as f64) as f32,
                }
            })
            .collect();
        Envelope {
            ch,
            winlength: n,
            searchstep: 64,
            minenergy: gi.preecho_minenergy,
            mdct: Mdct::new(n),
            mdct_win,
            band,
            filter: vec![FilterState::default(); VE_BANDS * ch],
            stretch: 0,
            mark: vec![0; 128],
            current: 0,
            curmark: 0,
            cursor: blocksize1 / 2,
        }
    }

    /// `_ve_amp`: one 128-sample window of one channel through the band filters `f0..`;
    /// bit 0: an attack, bit 1: a release, bit 2: reset the stretch.
    fn amp(&mut self, gi: &InfoPsyGlobal, data: &[f32], f0: usize) -> i32 {
        let n = self.winlength;
        let mut ret = 0;
        let min_v = self.minenergy;
        let stretch = VE_MINSTRETCH.max(self.stretch / 2) as usize;
        let mut penalty = gi.stretch_penalty - (self.stretch / 2 - VE_MINSTRETCH) as f32;
        if penalty < 0.0 {
            penalty = 0.0;
        }
        if penalty > gi.stretch_penalty {
            penalty = gi.stretch_penalty;
        }
        let mut vec = [0f32; 128];
        for i in 0..n {
            vec[i] = data[i] * self.mdct_win[i];
        }
        let inp = vec;
        self.mdct.forward(&inp, &mut vec);

        let mut decay;
        {
            let f = &mut self.filter[f0];
            // float + double terms: the sum in double, stored to float
            let temp = ((vec[0] * vec[0]) as f64
                + 0.7 * vec[1] as f64 * vec[1] as f64
                + 0.2 * vec[2] as f64 * vec[2] as f64) as f32;
            let ptr = f.nearptr;
            if ptr == 0 {
                f.near_dc_acc = f.near_dc_partialacc + temp;
                decay = f.near_dc_acc;
                f.near_dc_partialacc = temp;
            } else {
                f.near_dc_acc += temp;
                decay = f.near_dc_acc;
                f.near_dc_partialacc += temp;
            }
            f.near_dc_acc -= f.near_dc[ptr];
            f.near_dc[ptr] = temp;
            decay = (decay as f64 * (1. / (VE_NEARDC + 1) as f64)) as f32;
            f.nearptr += 1;
            if f.nearptr >= VE_NEARDC {
                f.nearptr = 0;
            }
            decay = (todb(decay) as f64 * 0.5 - 15.0) as f32;
        }
        let mut i = 0;
        while i < n / 2 {
            let mut val = vec[i] * vec[i] + vec[i + 1] * vec[i + 1];
            val = todb(val) * 0.5;
            if val < decay {
                val = decay;
            }
            if val < min_v {
                val = min_v;
            }
            vec[i >> 1] = val;
            decay = (decay as f64 - 8.0) as f32;
            i += 2;
        }
        for j in 0..VE_BANDS {
            let b = &self.band[j];
            let mut acc = 0f32;
            for (i, &w) in b.window.iter().enumerate() {
                acc += vec[i + b.begin] * w;
            }
            acc *= b.total;
            let f = &mut self.filter[f0 + j];
            let this = f.ampptr;
            let (mut premax, mut premin) = (-99999f32, 99999f32);
            let mut p = (this + VE_AMP - 1) % VE_AMP;
            let postmax = fmax(acc, f.ampbuf[p]);
            let postmin = fmin(acc, f.ampbuf[p]);
            for _ in 0..stretch {
                p = (p + VE_AMP - 1) % VE_AMP;
                premax = fmax(premax, f.ampbuf[p]);
                premin = fmin(premin, f.ampbuf[p]);
            }
            let valmin = postmin - premin;
            let valmax = postmax - premax;
            f.ampbuf[this] = acc;
            f.ampptr += 1;
            if f.ampptr >= VE_AMP {
                f.ampptr = 0;
            }
            if valmax > gi.preecho_thresh[j] + penalty {
                ret |= 1;
                ret |= 4;
            }
            if valmin < gi.postecho_thresh[j] - penalty {
                ret |= 2;
            }
        }
        ret
    }

    /// `_ve_envelope_search`: marks up to `pcm_current`; the next block's size (1: long, 0:
    /// short: a mark between the centres), `None` while not decided yet.
    pub(crate) fn search(
        &mut self,
        pcm: &[&[f32]],
        pcm_current: i64,
        center_w: i64,
        bs_w: i64,
        blocksizes: [i64; 2],
        gi: &InfoPsyGlobal,
    ) -> Option<usize> {
        let step = self.searchstep;
        let first = (self.current / step).max(0);
        let last = pcm_current / step - VE_WIN;
        if (last + VE_WIN + VE_POST) as usize > self.mark.len() {
            self.mark.resize((last + VE_WIN + VE_POST) as usize, 0);
        }
        let mut j = first;
        while j < last {
            let mut ret = 0;
            self.stretch += 1;
            if self.stretch > VE_MAXSTRETCH * 2 {
                self.stretch = VE_MAXSTRETCH * 2;
            }
            for (i, c) in pcm.iter().enumerate().take(self.ch) {
                let at = (step * j) as usize;
                ret |= self.amp(gi, &c[at..at + self.winlength], i * VE_BANDS);
            }
            let ju = j as usize;
            self.mark[ju + VE_POST as usize] = 0;
            if ret & 1 != 0 {
                self.mark[ju] = 1;
                self.mark[ju + 1] = 1;
            }
            if ret & 2 != 0 {
                self.mark[ju] = 1;
                if j > 0 {
                    self.mark[ju - 1] = 1;
                }
            }
            if ret & 4 != 0 {
                self.stretch = -1;
            }
            j += 1;
        }
        self.current = last * step;
        let test_w = center_w + bs_w / 4 + blocksizes[1] / 2 + blocksizes[0] / 4;
        let mut j = self.cursor;
        while j < self.current - step {
            if j >= test_w {
                return Some(1);
            }
            self.cursor = j;
            if self.mark[(j / step) as usize] != 0 && j > center_w {
                self.curmark = j;
                return Some(0); // (the C retests j >= test_w: false here)
            }
            j += step;
        }
        None
    }

    /// `_ve_envelope_mark`: whether a mark falls within block `w` (a short block: impulse).
    pub(crate) fn marked(
        &self,
        center_w: i64,
        blocksizes: [i64; 2],
        lw: usize,
        w: usize,
        nw: usize,
    ) -> bool {
        let mut begin_w = center_w - blocksizes[w] / 4;
        let mut end_w = center_w + blocksizes[w] / 4;
        if w != 0 {
            begin_w -= blocksizes[lw] / 4;
            end_w += blocksizes[nw] / 4;
        } else {
            begin_w -= blocksizes[0] / 4;
            end_w += blocksizes[0] / 4;
        }
        if self.curmark >= begin_w && self.curmark < end_w {
            return true;
        }
        let (first, last) = (begin_w / self.searchstep, end_w / self.searchstep);
        (first..last).any(|i| self.mark[i as usize] != 0)
    }

    /// `_ve_envelope_shift`: the buffer moved `shift` samples down.
    pub(crate) fn shift(&mut self, shift: i64) {
        let smallsize = (self.current / self.searchstep + VE_POST) as usize;
        let smallshift = (shift / self.searchstep) as usize;
        self.mark.copy_within(smallshift..smallsize, 0);
        self.current -= shift;
        if self.curmark >= 0 {
            self.curmark -= shift;
        }
        self.cursor -= shift;
    }
}

#[cfg(test)]
impl Envelope {
    /// The state `aotuv_blocks` (aotuv-sys `shim.c`) logs after each blockout, in its order.
    pub(crate) fn dump(&self, out: &mut Vec<f64>) {
        let marks = (self.current / self.searchstep + VE_POST) as usize;
        out.extend([
            self.stretch as f64,
            self.current as f64,
            self.curmark as f64,
            self.cursor as f64,
            marks as f64,
        ]);
        out.extend(self.mark[..marks].iter().map(|&m| m as f64));
        for f in &self.filter {
            out.extend(f.ampbuf.iter().map(|&x| x as f64));
            out.extend(f.near_dc.iter().map(|&x| x as f64));
            out.extend([
                f.ampptr as f64,
                f.near_dc_acc as f64,
                f.near_dc_partialacc as f64,
                f.nearptr as f64,
            ]);
        }
    }
}
