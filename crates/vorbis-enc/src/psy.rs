//! aoTuV's psychoacoustic model (`psy.c`): per block size and type, the look tables
//! (`_vp_psy_init`: absolute threshold, octave and bark maps, tone masking curves, noise
//! offsets), then per block the noise and tone masks and their mix. In the C's arithmetic:
//! the scale macros (`toOC`, `fromOC`, `toBARK`) in `double` through the platform libm, as the
//! C calls it.
#![allow(clippy::needless_range_loop)] // index loops mirror the C code 1:1

use crate::psy_tables::*;
use crate::scales::{fmax, fmin, from_oc, rint, rint_f64, to_bark, to_oc, unitnorm};
use crate::templates::{
    InfoMapping0, InfoPsy, InfoPsyGlobal, NOISE_COMPAND_LEVELS, P_BANDS, P_NOISECURVES,
};

const P_LEVELS: usize = 8;
/// `P_LEVEL_0` (`psy.h`).
const P_LEVEL_0: f64 = 30.0;
const MAX_ATH: usize = ATH.len();
const EHMER_OFFSET: usize = 16;
const EHMER_MAX: usize = 56;
const M3C: usize = 3;

/// `vorbis_look_psy`.
#[derive(Clone, Debug)]
pub(crate) struct PsyLook {
    pub n: usize,
    pub vi: InfoPsy,
    /// `[P_BANDS][P_LEVELS]` curves of `EHMER_MAX + 2` (first two: the used range)
    pub tonecurves: Vec<Vec<[f32; EHMER_MAX + 2]>>,
    pub noiseoffset: [Vec<f32>; P_NOISECURVES],
    pub ath: Vec<f32>,
    pub octave: Vec<i64>,
    pub bark: Vec<i64>,
    pub firstoc: i64,
    pub shiftoc: i64,
    pub eighth_octave_lines: i32,
    pub total_octave_lines: i32,
    pub rate: i64,
    pub m3n: [i32; 4],
    pub m_val: f32,
    pub tonecomp_endp: i32,
    pub tonecomp_thres: f32,
    pub ntfix_noiseoffset: Vec<f32>,
    pub min_nn_lp: i32,
    // (the C look also holds endeg_start/end, always 0, and nn75pt/nn50pt/nn25pt, quarters of
    // the partition; nothing reads them: the dump test derives them)
    pub tonefix_end: i32,
    pub n25p: i32,
    pub n33p: i32,
    pub n75p: i32,
}

/// `min_curve`.
fn min_curve(c: &mut [f32; EHMER_MAX], c2: &[f32; EHMER_MAX]) {
    for i in 0..EHMER_MAX {
        if c2[i] < c[i] {
            c[i] = c2[i];
        }
    }
}

/// `max_curve`.
fn max_curve(c: &mut [f32; EHMER_MAX], c2: &[f32; EHMER_MAX]) {
    for i in 0..EHMER_MAX {
        if c2[i] > c[i] {
            c[i] = c2[i];
        }
    }
}

/// `attenuate_curve`: `c[i] += att` (a float `att`).
fn attenuate_curve(c: &mut [f32; EHMER_MAX], att: f32) {
    for v in c.iter_mut() {
        *v += att;
    }
}

/// `tonemasks[band][level]`.
fn tonemask_row(band: usize, level: usize) -> [f32; EHMER_MAX] {
    let at = (band * 6 + level) * EHMER_MAX;
    TONEMASKS[at..at + EHMER_MAX].try_into().unwrap()
}

/// `setup_tone_curves`.
fn setup_tone_curves(
    curveatt_db: &[f32; P_BANDS],
    bin_hz: f32,
    n: usize,
    center_boost: f32,
    center_decay_rate: f32,
) -> Vec<Vec<[f32; EHMER_MAX + 2]>> {
    let mut workc = vec![[[0f32; EHMER_MAX]; P_LEVELS]; P_BANDS];
    let mut athc = [[0f32; EHMER_MAX]; P_LEVELS];
    let mut brute = vec![0f32; n];
    for i in 0..P_BANDS {
        let ath_offset = i * 4;
        let mut ath = [0f32; EHMER_MAX];
        for j in 0..EHMER_MAX {
            let mut min = 999f32;
            for k in 0..4 {
                let a = if j + k + ath_offset < MAX_ATH {
                    ATH[j + k + ath_offset]
                } else {
                    ATH[MAX_ATH - 1]
                };
                if min > a {
                    min = a;
                }
            }
            ath[j] = min;
        }
        for j in 0..6 {
            workc[i][j + 2] = tonemask_row(i, j);
        }
        workc[i][0] = tonemask_row(i, 0);
        workc[i][1] = tonemask_row(i, 0);
        for j in 0..P_LEVELS {
            for k in 0..EHMER_MAX {
                // float + int * float
                let mut adj = center_boost
                    + (EHMER_OFFSET as i32 - k as i32).abs() as f32 * center_decay_rate;
                if (adj as f64) < 0. && center_boost > 0.0 {
                    adj = 0.0;
                }
                if (adj as f64) > 0. && center_boost < 0.0 {
                    adj = 0.0;
                }
                workc[i][j][k] += adj;
            }
        }
        for j in 0..P_LEVELS {
            let lv = if j < 2 { 2 } else { j } as f64;
            attenuate_curve(
                &mut workc[i][j],
                (curveatt_db[i] as f64 + 100. - lv * 10. - P_LEVEL_0) as f32,
            );
            athc[j] = ath;
            attenuate_curve(
                &mut athc[j],
                (100. - (j as f32 * 10.0) as f64 - P_LEVEL_0) as f32,
            );
            max_curve(&mut athc[j], &workc[i][j]);
        }
        for j in 1..P_LEVELS {
            let prev = athc[j - 1];
            min_curve(&mut athc[j], &prev);
            min_curve(&mut workc[i][j], &athc[j]);
        }
    }
    let bin_hz64 = bin_hz as f64;
    // the C's `(int)(fromOC(...)/binHz)` and its bounds
    let bin_at = |o: f64| (from_oc(o) / bin_hz64) as i32;
    let mut ret = Vec::with_capacity(P_BANDS);
    for i in 0..P_BANDS {
        let bin = (from_oc(i as f64 * 0.5) / bin_hz64).floor() as i32;
        let mut lo_curve = (to_oc(bin as f64 * bin_hz64 + 1.) * 2.).ceil() as i32;
        let mut hi_curve = (to_oc((bin + 1) as f64 * bin_hz64) * 2.).floor() as i32;
        if lo_curve > i as i32 {
            lo_curve = i as i32;
        }
        if lo_curve < 0 {
            lo_curve = 0;
        }
        if hi_curve >= P_BANDS as i32 {
            hi_curve = P_BANDS as i32 - 1;
        }
        // The fills' bin ranges and the sampled bins depend on the band only (not the level):
        // computed once. Only bins below `cap` (past the last sampled bin) are ever read, and
        // the walk below `cap` is the C's (its cursor capped at `cap`: same writes there).
        let mut fills: Vec<(usize, [i32; EHMER_MAX], [i32; EHMER_MAX])> = Vec::new();
        let mut add = |k: usize, i_oc: f64| {
            let mut lo = [0i32; EHMER_MAX];
            let mut hi = [0i32; EHMER_MAX];
            for j in 0..EHMER_MAX {
                lo[j] = bin_at(j as f64 * 0.125 + i_oc * 0.5 - 2.0625).clamp(0, n as i32);
                hi[j] = (bin_at(j as f64 * 0.125 + i_oc * 0.5 - 1.9375) + 1).clamp(0, n as i32);
            }
            fills.push((k, lo, hi));
        };
        for k in lo_curve..=hi_curve {
            add(k as usize, k as f64);
        }
        if i + 1 < P_BANDS {
            add(i + 1, i as f64);
        }
        let mut sampled = [-1i32; EHMER_MAX];
        let mut cap = 0usize;
        for j in 0..EHMER_MAX {
            let bin = bin_at(j as f64 * 0.125 + i as f64 * 0.5 - 2.);
            if bin >= 0 && (bin as usize) < n {
                sampled[j] = bin;
                cap = cap.max(bin as usize + 1);
            }
        }
        let mut levels = Vec::with_capacity(P_LEVELS);
        for m in 0..P_LEVELS {
            let mut r = [0f32; EHMER_MAX + 2];
            brute[..cap].fill(999.0);
            for (k, lo, hi) in &fills {
                let w = &workc[*k][m];
                let mut l = 0usize;
                for j in 0..EHMER_MAX {
                    if (lo[j] as usize) < l {
                        l = lo[j] as usize;
                    }
                    let end = (hi[j] as usize).min(cap);
                    while l < end {
                        if brute[l] > w[j] {
                            brute[l] = w[j];
                        }
                        l += 1;
                    }
                }
                while l < cap {
                    if brute[l] > w[EHMER_MAX - 1] {
                        brute[l] = w[EHMER_MAX - 1];
                    }
                    l += 1;
                }
            }
            for j in 0..EHMER_MAX {
                r[j + 2] = if sampled[j] < 0 {
                    -999.0
                } else {
                    brute[sampled[j] as usize]
                };
            }
            let mut j = 0;
            while j < EHMER_OFFSET {
                if r[j + 2] > -200.0 {
                    break;
                }
                j += 1;
            }
            r[0] = j as f32;
            let mut j = EHMER_MAX - 1;
            while j > EHMER_OFFSET + 1 {
                if r[j + 2] > -200.0 {
                    break;
                }
                j -= 1;
            }
            r[1] = j as f32;
            levels.push(r);
        }
        ret.push(levels);
    }
    ret
}

impl PsyLook {
    /// `_vp_psy_init`: the look of psy setting `vi` for `n` = blocksize/2 bins at `rate`.
    pub(crate) fn new(vi: &InfoPsy, gi: &InfoPsyGlobal, n: usize, rate: i64) -> Self {
        let (mut lo, mut hi): (i64, i64) = (-99, 1);
        let eighth = gi.eighth_octave_lines;
        let shiftoc =
            (((eighth as f32 * 8.0) as f64).ln() / (2f32 as f64).ln()).round_ties_even() as i64 - 1;
        let sh = (1i64 << (shiftoc + 1)) as f64;
        let nn = n as i64;
        let firstoc =
            (to_oc((0.25_f32 * rate as f32) as f64 * 0.5 / n as f64) * sh - eighth as f64) as i64;
        let maxoc = (to_oc(((n as f32 + 0.25) * rate as f32) as f64 * 0.5 / n as f64) * sh
            + 0.5_f32 as f64) as i64;
        let (m_val, select, m3n): (f32, i32, [i32; 4]) = if rate < 26000 {
            (0.0, -1, [0; 4])
        } else {
            let (mv, base, tabs) = if rate < 38000 {
                (0.93_f64 as f32, 0, (&M3N32, &M3N32X2))
            } else if rate > 46000 {
                (1.205_f64 as f32, 4, (&M3N48, &M3N48X2))
            } else {
                (1.0, 8, (&M3N44, &M3N44X2))
            };
            let mut m3 = [0; 4];
            let sel = match n {
                128 => {
                    m3[..M3C].copy_from_slice(tabs.0);
                    base
                }
                256 => {
                    m3[..M3C].copy_from_slice(tabs.1);
                    base + 1
                }
                1024 => base + 2,
                2048 => base + 3,
                _ => -1,
            };
            (mv, sel, m3)
        };
        let (tonecomp_endp, tonecomp_thres, min_nn_lp, tonefix_end) = if select < 0 {
            (0, 0.25_f64 as f32, 0, 0)
        } else {
            SET_AOTUV_PSY[select as usize]
        };

        // the absolute threshold: ATH interpolated per bin, +100 dB, extended past its end
        let mut ath = vec![0f32; n];
        let mut j = 0usize;
        for i in 0..MAX_ATH - 1 {
            let endpos = (from_oc((i + 1) as f64 * 0.125 - 2.) * 2. * n as f64 / rate as f64)
                .round_ties_even() as i64;
            let mut base = ATH[i];
            if (j as i64) < endpos {
                let delta = (ATH[i + 1] - base) / (endpos - j as i64) as f32;
                while (j as i64) < endpos && j < n {
                    ath[j] = (base as f64 + 100.) as f32;
                    base += delta;
                    j += 1;
                }
            }
        }
        {
            let mut cs = ath[j - 1];
            let ds = ath[j - 1] - ath[j - 2];
            for v in &mut ath[j..] {
                *v = cs;
                cs += ds;
            }
        }

        let step = rate / (2 * nn);
        let mut bark = vec![0i64; n];
        for i in 0..n {
            let b = to_bark(step * i as i64) as f32;
            while lo + (vi.noisewindowlomin as i64) < i as i64
                && to_bark(step * lo) < (b - vi.noisewindowlo) as f64
            {
                lo += 1;
            }
            while hi <= nn
                && (hi < i as i64 + vi.noisewindowhimin as i64
                    || to_bark(step * hi) < (b + vi.noisewindowhi) as f64)
            {
                hi += 1;
            }
            bark[i] = ((lo - 1) << 16) + (hi - 1);
        }
        let octave = (0..n)
            .map(|i| {
                (to_oc((i as f32 + 0.25) as f64 * 0.5 * rate as f64 / n as f64) * sh
                    + 0.5_f32 as f64) as i64
            })
            .collect();
        let tonecurves = setup_tone_curves(
            &vi.toneatt,
            (rate as f64 * 0.5 / n as f64) as f32,
            n,
            vi.tone_centerboost,
            vi.tone_decay,
        );

        let mut noiseoffset: [Vec<f32>; P_NOISECURVES] = Default::default();
        for v in &mut noiseoffset {
            *v = vec![0f32; n];
        }
        let mut ntfix_noiseoffset = vec![0f32; n];
        for i in 0..n {
            let mut halfoc = (to_oc((i as f64 + 0.5) * rate as f64 / (2. * n as f64)) * 2.) as f32;
            if halfoc < 0.0 {
                halfoc = 0.0;
            }
            if halfoc >= (P_BANDS - 1) as f32 {
                halfoc = (P_BANDS - 1) as f32;
            }
            let inthalfoc = halfoc as usize;
            let del = halfoc - inthalfoc as f32;
            // `x[k]*(1.-del)` is double, `x[k+1]*del` a float product (both float); at the top
            // band the C reads one past the row with weight 0 (the results page's defect 2):
            // here that term is `+ 0.` without the read
            let next = |row: &[f32]| (row.get(inthalfoc + 1).copied().unwrap_or(0.0) * del) as f64;
            for k in 0..P_NOISECURVES {
                let row = &vi.noiseoff[k];
                noiseoffset[k][i] = (row[inthalfoc] as f64 * (1. - del as f64) + next(row)) as f32;
            }
            ntfix_noiseoffset[i] =
                (NTFIX_OFFSET[inthalfoc] as f64 * (1. - del as f64) + next(&NTFIX_OFFSET)) as f32;
        }
        let n25p = (n / 4) as i32;
        PsyLook {
            n,
            vi: vi.clone(),
            tonecurves,
            noiseoffset,
            ath,
            octave,
            bark,
            firstoc,
            shiftoc,
            eighth_octave_lines: eighth,
            total_octave_lines: (maxoc - firstoc + 1) as i32,
            rate,
            m3n,
            m_val,
            tonecomp_endp,
            tonecomp_thres,
            ntfix_noiseoffset,
            min_nn_lp,
            tonefix_end,
            n25p,
            n33p: (n / 3) as i32,
            n75p: n25p * 3,
        }
    }
}

/// `NEGINF`.
const NEGINF: f32 = -9999.0;

/// `seed_curve`: raise `seed` along tone curve `curves[choice]` placed at octave line `oc`.
fn seed_curve(
    seed: &mut [f32],
    curves: &[[f32; EHMER_MAX + 2]],
    amp: f32,
    oc: i64,
    n: i64,
    linesper: i32,
    db_offset: f32,
) {
    let mut choice = (((amp + db_offset) as f64 - P_LEVEL_0) * 0.1_f32 as f64) as i32;
    choice = choice.max(0).min(P_LEVELS as i32 - 1);
    let posts = &curves[choice as usize];
    let curve = &posts[2..];
    let post1 = posts[1] as i32;
    // all float: `oc+(posts[0]-EHMER_OFFSET)*linesper-(linesper>>1)`
    let seedptr = (oc as f32 + (posts[0] - EHMER_OFFSET as f32) * linesper as f32
        - (linesper >> 1) as f32) as i64;
    // The C's loop as a range: point k (curve index posts[0] + k) at seedptr + k·linesper, raised
    // where seedptr > 0, for k below the curve's end, and up to the first point at or past n
    // (the C breaks after writing the last below n; seedptr < n: posts[0] ≤ EHMER_OFFSET and
    // oc < n).
    let l = linesper as i64;
    let p0 = posts[0] as i32;
    let count = (post1 - p0).max(0) as i64;
    debug_assert!(seedptr < n);
    let below_n = (n - seedptr + l - 1) / l;
    let kmax = count.min(below_n);
    let kstart = if seedptr > 0 { 0 } else { -seedptr / l + 1 };
    if kstart >= kmax {
        return;
    }
    let first = (seedptr + kstart * l) as usize;
    let lines = &curve[(p0 as i64 + kstart) as usize..(p0 as i64 + kmax) as usize];
    for (s, &c) in seed[first..].iter_mut().step_by(l as usize).zip(lines) {
        let lin = amp + c;
        *s = if *s < lin { lin } else { *s };
    }
}

/// `seed_chase`: the running maxima of `seeds` over `linesper` lines.
fn seed_chase(seeds: &mut [f32], linesper: i32, n: usize) {
    let mut posstack = vec![0i64; n];
    let mut ampstack = vec![0f32; n];
    let mut stack = 0usize;
    for i in 0..n {
        if stack < 2 {
            posstack[stack] = i as i64;
            ampstack[stack] = seeds[i];
            stack += 1;
        } else {
            loop {
                if seeds[i] < ampstack[stack - 1] {
                    posstack[stack] = i as i64;
                    ampstack[stack] = seeds[i];
                    stack += 1;
                    break;
                }
                if (i as i64) < posstack[stack - 1] + linesper as i64
                    && stack > 1
                    && ampstack[stack - 1] <= ampstack[stack - 2]
                    && (i as i64) < posstack[stack - 2] + linesper as i64
                {
                    stack -= 1;
                    continue;
                }
                posstack[stack] = i as i64;
                ampstack[stack] = seeds[i];
                stack += 1;
                break;
            }
        }
    }
    let mut pos = 0usize;
    for i in 0..stack {
        let mut endpos = if i < stack - 1 && ampstack[i + 1] > ampstack[i] {
            posstack[i + 1]
        } else {
            posstack[i] + linesper as i64 + 1
        };
        if endpos > n as i64 {
            endpos = n as i64;
        }
        while (pos as i64) < endpos {
            seeds[pos] = ampstack[i];
            pos += 1;
        }
    }
}

/// `bark_noise_hybridmp`: a sliding weighted line fit of `f + offset` over each bin's bark
/// window (`b`), then (with `fixed` > 0) the same over a fixed window, the lower kept.
fn bark_noise_hybridmp(n: usize, b: &[i64], f: &[f32], noise: &mut [f32], offset: f32, fixed: i32) {
    let mut sums = vec![0f32; 5 * n];
    let (nn, r) = sums.split_at_mut(n);
    let (xa, r) = r.split_at_mut(n);
    let (xx, r) = r.split_at_mut(n);
    let (ya, xy) = r.split_at_mut(n);
    let f = &f[..n];
    let noise = &mut noise[..n];
    let (mut t_n, mut t_x, mut t_xx, mut t_y, mut t_xy) = (0f32, 0f32, 0f32, 0f32, 0f32);
    let (mut a, mut bb, mut d) = (0f32, 0f32, 1f32);
    let mut y = f[0] + offset;
    if y < 1.0 {
        y = 1.0;
    }
    let mut w = ((y * y) as f64 * 0.5) as f32;
    t_n += w;
    t_x += w;
    t_y += w * y;
    nn[0] = t_n;
    xa[0] = t_x;
    xx[0] = t_xx;
    ya[0] = t_y;
    xy[0] = t_xy;
    let mut x = 1f32;
    for i in 1..n {
        y = f[i] + offset;
        if y < 1.0 {
            y = 1.0;
        }
        w = y * y;
        t_n += w;
        t_x += w * x;
        t_xx += w * x * x;
        t_y += w * y;
        t_xy += w * x * y;
        nn[i] = t_n;
        xa[i] = t_x;
        xx[i] = t_xx;
        ya[i] = t_y;
        xy[i] = t_xy;
        x += 1.0;
    }
    // the window (lo, hi]'s (A, B, D); `mirror`: it folds at bin 0, the sums to -lo added
    let fit = |lo: usize, hi: usize, mirror: bool| fit_window(nn, xa, xx, ya, xy, lo, hi, mirror);
    let mut i = 0usize;
    x = 0.0;
    let ni = n as i64;
    while i < n {
        let lo = b[i] >> 16;
        let hi = b[i] & 0xffff;
        if lo >= 0 || -lo >= ni || hi >= ni {
            break;
        }
        let (lo, hi) = ((-lo) as usize, hi as usize);
        (a, bb, d) = fit(lo, hi, true);
        let mut r = (a + x * bb) / d;
        if r < 0.0 {
            r = 0.0;
        }
        noise[i] = r - offset;
        i += 1;
        x += 1.0;
    }
    while i < n {
        let lo = b[i] >> 16;
        let hi = b[i] & 0xffff;
        if lo < 0 || lo >= ni || hi >= ni {
            break;
        }
        let (lo, hi) = (lo as usize, hi as usize);
        (a, bb, d) = fit(lo, hi, false);
        let mut r = (a + x * bb) / d;
        if r < 0.0 {
            r = 0.0;
        }
        noise[i] = r - offset;
        i += 1;
        x += 1.0;
    }
    while i < n {
        let mut r = (a + x * bb) / d;
        if r < 0.0 {
            r = 0.0;
        }
        noise[i] = r - offset;
        i += 1;
        x += 1.0;
    }
    if fixed <= 0 {
        return;
    }
    i = 0;
    x = 0.0;
    while i < n {
        let hi = i as i64 + (fixed / 2) as i64;
        let lo = hi - fixed as i64;
        if hi >= ni || lo >= 0 {
            break;
        }
        let (lo, hi) = ((-lo) as usize, hi as usize);
        (a, bb, d) = fit(lo, hi, true);
        let r = (a + x * bb) / d;
        if r - offset < noise[i] {
            noise[i] = r - offset;
        }
        i += 1;
        x += 1.0;
    }
    while i < n {
        let hi = i as i64 + (fixed / 2) as i64;
        let lo = hi - fixed as i64;
        if hi >= ni || lo < 0 {
            break;
        }
        let (lo, hi) = (lo as usize, hi as usize);
        (a, bb, d) = fit(lo, hi, false);
        let r = (a + x * bb) / d;
        if r - offset < noise[i] {
            noise[i] = r - offset;
        }
        i += 1;
        x += 1.0;
    }
    while i < n {
        let r = (a + x * bb) / d;
        if r - offset < noise[i] {
            noise[i] = r - offset;
        }
        i += 1;
        x += 1.0;
    }
}

/// One window's (A, B, D) from the prefix sums (`bark_noise_hybridmp`'s per-bin fit).
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn fit_window(
    nn: &[f32],
    xa: &[f32],
    xx: &[f32],
    ya: &[f32],
    xy: &[f32],
    lo: usize,
    hi: usize,
    mirror: bool,
) -> (f32, f32, f32) {
    let t_n = if mirror {
        nn[hi] + nn[lo]
    } else {
        nn[hi] - nn[lo]
    };
    let t_x = xa[hi] - xa[lo];
    let t_xx = if mirror {
        xx[hi] + xx[lo]
    } else {
        xx[hi] - xx[lo]
    };
    let t_y = if mirror {
        ya[hi] + ya[lo]
    } else {
        ya[hi] - ya[lo]
    };
    let t_xy = xy[hi] - xy[lo];
    (
        t_y * t_xx - t_x * t_xy,
        t_n * t_xy - t_x * t_y,
        t_n * t_xx - t_x * t_x,
    )
}

/// `local_mod3_psy`.
#[derive(Clone, Copy, Debug, Default)]
struct Mod3 {
    sw: bool,
    mdctbuf_flag: bool,
    noise_rate: f32,
    noise_rate_low: f32,
    noise_center: f32,
    tone_rate: f32,
}

/// `_postnoise_detection`: the energy jump into the second half of a long transition block
/// after a short one (aoTuV's post-echo noise), or -1.
pub(crate) fn postnoise_detection(pcm: &[f32], nn: usize, mode: i32, lw_mode: i32) -> f32 {
    let sn = nn >> 2;
    let mn = sn + sn;
    let en = sn + (nn >> 1);
    let mut ret = -1f32;
    if mode != 2 || lw_mode != 0 || nn < 2048 {
        return ret;
    }
    let mut upt = 0f64;
    let mut unt = 0f64;
    for &v in &pcm[sn..mn] {
        upt += (v as f64).abs();
    }
    for &v in &pcm[mn..en] {
        unt += (v as f64).abs();
    }
    if unt / sn as f64 > 0.01 {
        return ret;
    }
    upt *= upt;
    unt *= unt;
    unt *= 15.;
    if upt > unt {
        ret = (upt - unt) as f32;
        if (ret as f64) < 0.1 {
            ret = -1.0;
        }
    }
    ret
}

#[allow(clippy::neg_cmp_op_on_partial_ord)] // the C's `continue` conditions, NaN included
impl PsyLook {
    /// `seed_loop`.
    fn seed_loop(&self, f: &[f32], flr: &[f32], seed: &mut [f32], specmax: f32) {
        let n = self.n;
        let db_offset = self.vi.max_curve_db - specmax;
        let mut i = 0usize;
        while i < n {
            let mut max = f[i];
            let mut oc = self.octave[i];
            while i + 1 < n && self.octave[i + 1] == oc {
                i += 1;
                if f[i] > max {
                    max = f[i];
                }
            }
            if max + 6.0 > flr[i] {
                oc >>= self.shiftoc;
                oc = oc.clamp(0, P_BANDS as i64 - 1);
                seed_curve(
                    seed,
                    &self.tonecurves[oc as usize],
                    max,
                    self.octave[i] - self.firstoc,
                    self.total_octave_lines as i64,
                    self.eighth_octave_lines,
                    db_offset,
                );
            }
            i += 1;
        }
    }

    /// `max_seeds`.
    fn max_seeds(&self, seed: &mut [f32], flr: &mut [f32]) {
        let n = self.total_octave_lines as usize;
        let linesper = self.eighth_octave_lines;
        seed_chase(seed, linesper, n);
        let mut linpos = 0usize;
        let mut pos = (self.octave[0] - self.firstoc - (linesper >> 1) as i64) as usize;
        while linpos + 1 < self.n {
            let mut min_v = seed[pos];
            let mut end = ((self.octave[linpos] + self.octave[linpos + 1]) >> 1) - self.firstoc;
            if min_v > self.vi.tone_abs_limit {
                min_v = self.vi.tone_abs_limit;
            }
            while (pos + 1) as i64 <= end {
                pos += 1;
                if (seed[pos] > NEGINF && seed[pos] < min_v) || min_v == NEGINF {
                    min_v = seed[pos];
                }
            }
            end = pos as i64 + self.firstoc;
            while linpos < self.n && self.octave[linpos] <= end {
                if flr[linpos] < min_v {
                    flr[linpos] = min_v;
                }
                linpos += 1;
            }
        }
        let min_v = seed[n - 1];
        for v in &mut flr[linpos..self.n] {
            if *v < min_v {
                *v = min_v;
            }
        }
    }

    /// `ntfix`: aoTuV's tonal peak correction of the noise estimate.
    fn ntfix(&self, spectral: &[f32], noise: &mut [f32], block_mode: i32) {
        let n = self.n;
        let mut nx = self.tonefix_end as usize;
        let limit = self.noiseoffset[1][0].abs();
        if nx == 0 {
            return;
        }
        let mut temp = [0f32; 256];
        let mut inmod = [0f32; 256];
        if block_mode <= 1 {
            const FREQ_UPC: usize = 3;
            const FREQ_UNC: usize = 4;
            let mut nxplus = nx + FREQ_UNC;
            let mut tolerance = 9f32;
            let strength = 0.6f32;
            if n == 256 {
                tolerance = 15.0;
            }
            if nxplus > n {
                nx = n;
                nxplus = n - FREQ_UNC;
            }
            for i in 0..nxplus {
                inmod[i] = if spectral[i] < -70.0 {
                    (-70. + (spectral[i] + 70.0) as f64 * 0.1) as f32
                } else {
                    spectral[i]
                };
            }
            let mut i = FREQ_UNC;
            while i < nx {
                if spectral[i] > spectral[i - 1] && spectral[i] > spectral[i + 1] {
                    let mut ps = i - 1;
                    let mut pe = i + 1;
                    let upper = i as i64 - FREQ_UPC as i64;
                    let under = i + FREQ_UNC;
                    let mut j = ps as i64;
                    while j > upper {
                        let ju = j as usize;
                        if spectral[ju + 1] < spectral[ju] {
                            break;
                        }
                        ps = ju;
                        j -= 1;
                    }
                    let mut j = pe;
                    while j < under {
                        if spectral[j - 1] < spectral[j] {
                            break;
                        }
                        pe = j;
                        j += 1;
                    }
                    let mut ss = inmod[i] - inmod[ps];
                    ss = fmax(ss, inmod[i] - inmod[pe]);
                    if ss > tolerance {
                        if spectral[i] > noise[i] {
                            ss -= tolerance;
                            ss *= strength;
                        }
                        for t in &mut temp[ps..=pe] {
                            *t = fmax(ss, *t);
                            if *t < 0.0 {
                                *t = 0.0;
                            }
                        }
                    }
                    i = pe;
                }
                i += 1;
            }
            for i in FREQ_UNC - 1..nx {
                let test = fmin(self.ntfix_noiseoffset[i], self.noiseoffset[1][i] + limit);
                if temp[i] > test {
                    temp[i] = test;
                }
                noise[i] -= temp[i];
            }
        } else if block_mode == 2 {
            let mut k = 0;
            let mut i = 0;
            while i < nx {
                let mut na = 0f64;
                for j in 0..8 {
                    na += noise[i + j] as f64;
                }
                na /= 8.;
                temp[k] = na as f32;
                i += 8;
                k += 1;
            }
            nx /= 8;
            for i in 3..nx {
                if temp[i] > temp[i - 1] && temp[i] > temp[i + 1] {
                    let (thres0, mut a) = if temp[i - 1] > temp[i - 2] {
                        (temp[i - 2], i - 3)
                    } else {
                        (temp[i - 1], i - 2)
                    };
                    let mut b = i + 3;
                    let mut thres = temp[i] - thres0;
                    if thres as f64 > 2. {
                        let e = i * 8;
                        let test = fmin(self.ntfix_noiseoffset[e], self.noiseoffset[1][e] + limit);
                        thres = fmin(thres - 2.0, test);
                        a *= 8;
                        b *= 8;
                        for v in &mut noise[a..=b] {
                            *v -= thres;
                        }
                    }
                }
            }
        }
    }

    /// `_vp_noisemask`: the noise masking curve `logmask`, the compand peaks `epeak` and the
    /// noise-normalisation partitions `npeak`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn noisemask(
        &self,
        noise_compand_level: f32,
        logmdct: &[f32],
        lastmdct: &[f32],
        epeak: &mut [f32],
        npeak: &mut [f32],
        logmask: &mut [f32],
        poste: f32,
        block_mode: i32,
    ) {
        let n = self.n;
        let partition = self.vi.partition();
        let mut work = vec![0f32; n];
        bark_noise_hybridmp(n, &self.bark, logmdct, logmask, 140.0, -1);
        for i in 0..n {
            work[i] = logmdct[i] - logmask[i];
        }
        bark_noise_hybridmp(n, &self.bark, &work, logmask, 0.0, self.vi.noisewindowfixed);
        for i in 0..n {
            work[i] = logmdct[i] - work[i];
        }
        self.ntfix(logmdct, &mut work, block_mode);
        let db_of =
            |m: f32| ((m as f64 + 0.5) as i32).clamp(0, NOISE_COMPAND_LEVELS as i32 - 1) as usize;
        let mut i = 0usize;
        if noise_compand_level > 0.0 {
            let thter = self.n33p as usize;
            while i < thter {
                let db = db_of(logmask[i]);
                epeak[i] = work[i] + STN_COMPAND[db] as f32;
                logmask[i] = work[i] + self.vi.noisecompand[db]
                    - (self.vi.noisecompand[db] - self.vi.noisecompand_high[db])
                        * noise_compand_level;
                i += 1;
            }
        }
        while i < n {
            let db = db_of(logmask[i]);
            epeak[i] = work[i] + STN_COMPAND[db] as f32;
            logmask[i] = work[i] + self.vi.noisecompand[db];
            i += 1;
        }
        let min_nn_lp = self.min_nn_lp as usize;
        let mut k = 0;
        let mut i = 0;
        while i < n {
            npeak[k] = 0.0;
            i += partition;
            k += 1;
        }
        if poste > 0.0 {
            let (mut i, mut k) = (0, 0);
            while i < min_nn_lp {
                let temp = fmin(fmin(poste, 30.0), self.noiseoffset[1][i] + 30.0);
                if !(temp <= 0.0) {
                    npeak[k] = -1.0;
                    for v in &mut logmask[i..i + partition] {
                        *v -= temp;
                    }
                }
                i += partition;
                k += 1;
            }
        }
        let (mut i, mut k) = (0, 0);
        while i < min_nn_lp {
            let nt = 4f32;
            let o = self.noiseoffset[1][i + partition - 1] + 6.0;
            let mut me = 0f32;
            let mut avge = 0f32;
            // the C's `continue`s, NaN included
            if !(o <= 0.0) && !(npeak[k] < -0.5) {
                for j in 0..partition {
                    let temp = logmdct[i + j] - logmask[i + j];
                    if me < temp {
                        me = temp;
                    }
                    avge += logmdct[i + j];
                }
                if !(avge < (-95 * partition as i32) as f32) && me < nt {
                    npeak[k] = fmin(o, nt - me) / nt;
                }
            }
            i += partition;
            k += 1;
        }
        let mut i = 0;
        if block_mode > 1 {
            while i < self.tonecomp_endp as usize {
                let temp = logmdct[i] - epeak[i];
                epeak[i] = 0.0;
                if temp >= 12.0 {
                    let mi = logmdct[i] - lastmdct[i];
                    if mi >= 1.0 {
                        epeak[i] = mi;
                    }
                }
                i += 1;
            }
        }
        epeak[i..n].fill(0.0);
    }

    /// `_vp_tonemask`: the tone masking curve `logmask` (above the absolute threshold).
    pub(crate) fn tonemask(
        &self,
        logfft: &[f32],
        logmask: &mut [f32],
        global_specmax: f32,
        local_specmax: f32,
    ) {
        let n = self.n;
        let mut seed = vec![NEGINF; self.total_octave_lines as usize];
        let mut att = local_specmax + self.vi.ath_adjatt;
        if att < self.vi.ath_maxatt {
            att = self.vi.ath_maxatt;
        }
        for i in 0..n {
            logmask[i] = self.ath[i] + att;
        }
        self.seed_loop(logfft, logmask, &mut seed, global_specmax);
        self.max_seeds(&mut seed, logmask);
    }

    /// `set_m3p`: aoTuV's short-block noise/tone tuning (M3), and the `tempmdct` history.
    #[allow(clippy::too_many_arguments)]
    fn set_m3p(
        &self,
        mp: &mut Mod3,
        lw_no: i32,
        impadnum: i32,
        hs_rate: bool,
        toneatt: f32,
        logmdct: &[f32],
        lastmdct: &[f32],
        tempmdct: &mut [f32],
        block_mode: i32,
        lw_block_mode: i32,
        bit_managed: bool,
        offset_select: usize,
    ) {
        let n = self.n;
        if !hs_rate {
            mp.sw = false;
            mp.mdctbuf_flag = false;
            return;
        }
        if !bit_managed || offset_select == 2 {
            mp.mdctbuf_flag = true;
        } else {
            mp.mdctbuf_flag = false;
            if offset_select == 0 {
                mp.sw = false;
                return;
            }
        }
        if block_mode != 0 {
            mp.sw = false;
            return;
        }
        let (bfn, sub, imp, add): (&[i32], f32, f64, f64) = match n {
            128 => (&FREQ_BFN128, 5.0, 0.125, 5.),
            256 => (&FREQ_BFN256, 10.0, 0.0625, 10.),
            _ => {
                mp.sw = false;
                return;
            }
        };
        if n == 128 {
            let count = if toneatt < 3.0 { 2 } else { 3 };
            if lw_block_mode == 0 {
                if lw_no < 8 {
                    mp.noise_rate = (0.7 - ((lw_no - 1) as f32 / 17.0) as f64) as f32;
                    mp.noise_center = (lw_no * count) as f32;
                    mp.tone_rate = (8 - lw_no) as f32;
                } else {
                    mp.noise_rate = 0.3_f64 as f32;
                    mp.noise_center = 25.0;
                    mp.tone_rate = 0.0;
                    if lw_no * count < 24 {
                        mp.noise_center = (lw_no * count) as f32;
                    }
                }
            } else {
                mp.noise_rate = 0.7_f64 as f32;
                mp.noise_center = 0.0;
                mp.tone_rate = 8.0;
            }
        } else if lw_block_mode == 0 {
            let count = 6;
            if lw_no < 4 {
                mp.noise_rate = (0.4 - ((lw_no - 1) as f32 / 11.0) as f64) as f32;
                mp.noise_center = (lw_no * count + 12) as f32;
                mp.tone_rate = (8 - lw_no * 2) as f32;
            } else {
                mp.noise_rate = 0.2_f64 as f32;
                mp.noise_center = 30.0;
                mp.tone_rate = 0.0;
            }
        } else {
            mp.noise_rate = 0.6_f64 as f32;
            mp.noise_center = 12.0;
            mp.tone_rate = 8.0;
        }
        if mp.mdctbuf_flag {
            if lw_block_mode == 0 {
                for v in &mut tempmdct[..n] {
                    *v -= sub;
                }
            } else {
                for i in 0..n {
                    tempmdct[i] = lastmdct[i] - sub;
                }
            }
        }
        mp.noise_rate_low = 0.0;
        mp.sw = true;
        if impadnum != 0 {
            mp.noise_rate = (mp.noise_rate as f64 * (impadnum as f64 * imp)) as f32;
        }
        for i in 0..n {
            let cell = 75.0 / bfn[i] as f32;
            for j in 1..bfn[i] as usize {
                let freqbuf = logmdct[i] - cell * j as f32;
                if tempmdct[i + j] < freqbuf && mp.mdctbuf_flag {
                    tempmdct[i + j] =
                        (tempmdct[i + j] as f64 + add / bfn[i + j] as f32 as f64) as f32;
                }
            }
        }
    }

    /// `_vp_offset_and_mix`: noise and tone masks offset per `offset_select` and mixed into
    /// `logmask`; aoTuV's M1 (MDCT scaling, select 1), M3 (short blocks after long ones), M4
    /// (tone compensation) and the `lastmdct` history.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn offset_and_mix(
        &self,
        noise: &[f32],
        tone: &[f32],
        offset_select: usize,
        bit_managed: bool,
        logmask: &mut [f32],
        mdct: &mut [f32],
        logmdct: &[f32],
        lastmdct: &mut [f32],
        tempmdct: &mut [f32],
        mut low_compand: f32,
        npeak: &mut [f32],
        end_block: i32,
        block_mode: i32,
        nw_modenumber: usize,
        lw_block_mode: i32,
        lw_no: i32,
        impadnum: i32,
    ) {
        let n = self.n;
        let hsrate = self.rate >= 26000;
        let partition = self.vi.partition();
        let toneatt = self.vi.tone_masteratt[offset_select];
        let mut mp3 = Mod3::default();
        let mut mp4_start = self.vi.normal_start;
        let mut mp4_end = self.tonecomp_endp;
        let mp4_thres = self.tonecomp_thres;
        let mut mp4_end_block = end_block;
        if low_compand < 0.0 || (toneatt as f64) < 25. {
            low_compand = 0.0;
        } else {
            low_compand = (low_compand as f64 * (toneatt as f64 - 25.)) as f32;
        }
        self.set_m3p(
            &mut mp3,
            lw_no,
            impadnum,
            hsrate,
            toneatt,
            logmdct,
            lastmdct,
            tempmdct,
            block_mode,
            lw_block_mode,
            bit_managed,
            offset_select,
        );
        mp4_end_block += self.vi.normal_partition;
        if mp4_end_block > n as i32 {
            mp4_end_block = n as i32;
        }
        if !hsrate {
            mp4_end = mp4_end_block;
        } else if self.vi.normal_thresh > 1. {
            mp4_start = 9999;
        }
        // (`lp_pos` is set here in the C but never read)
        let m1_coeffi = -17.2_f64 as f32;
        for i in 0..n {
            let ii = i as i32;
            let mut val = noise[i] + self.noiseoffset[offset_select][i];
            let mut tval = tone[i] + toneatt;
            if ii <= mp4_start {
                tval -= low_compand;
            }
            if val > self.vi.noisemaxsupp {
                val = self.vi.noisemaxsupp;
            }
            if mp3.sw
                && val > tval
                && val > lastmdct[i]
                && logmdct[i] > tempmdct[i] + mp3.noise_center
            {
                let mut toneac = false;
                if mp3.mdctbuf_flag {
                    tempmdct[i] = logmdct[i];
                }
                let mut rate_mod = if logmdct[i] > lastmdct[i] {
                    mp3.noise_rate
                } else {
                    mp3.noise_rate_low
                };
                if impadnum == 0 && ii < self.tonecomp_endp && (val - lastmdct[i]) > 20.0 {
                    let db_sub = logmdct[i] - lastmdct[i];
                    if db_sub > 25.0 {
                        toneac = true;
                        if tval > -100.0 && (logmdct[i] - tval) < 48.0 {
                            let mut tr_cur = mp3.tone_rate;
                            if db_sub < 35.0 {
                                tr_cur *= (35.0 - db_sub) * 0.1;
                            }
                            tval -= tr_cur;
                            if tval < -100.0 {
                                tval = -100.0;
                            }
                            if (logmdct[i] - tval) > 48.0 {
                                tval = logmdct[i] - 48.0;
                            }
                        }
                    }
                }
                let mainth;
                if ii > self.m3n[0] {
                    mainth = 30f32;
                } else if ii > self.m3n[1] {
                    mainth = 20.0;
                } else if ii > self.m3n[2] {
                    mainth = 10.0;
                    rate_mod *= 0.5;
                } else {
                    mainth = 10.0;
                    rate_mod *= 0.3;
                }
                let valmask = if (val - tval) > mainth {
                    ((val - tval - mainth) * 0.1 + mainth) * rate_mod
                } else {
                    (val - tval) * rate_mod
                };
                if (val - valmask) > lastmdct[i] {
                    val -= valmask;
                } else {
                    val = lastmdct[i];
                }
                if toneac {
                    // `max(lastmdct[i], -140)`: an int and a float, the result float
                    let temp = val
                        - if lastmdct[i] < -140.0 {
                            -140.0
                        } else {
                            lastmdct[i]
                        };
                    if temp > 20.0 {
                        val -= (temp - 20.0) * 0.2;
                    }
                }
                let k = i / partition;
                if toneac {
                    npeak[k] = -1.0;
                } else if npeak[k] > 0.0 {
                    npeak[k] = 0.0;
                }
            }
            if val > tval {
                logmask[i] = val;
            } else if ii > mp4_start && ii < mp4_end {
                if logmdct[i] < tval {
                    if logmdct[i] < val {
                        tval -= (tval - val) * mp4_thres;
                    } else {
                        tval = logmdct[i];
                    }
                }
                logmask[i] = tval;
            } else {
                logmask[i] = tval;
            }
            if offset_select == 1 {
                val -= logmdct[i];
                let mut m1_de;
                if val > m1_coeffi {
                    m1_de = (1.0 - (val - m1_coeffi) as f64 * 0.005 * self.m_val as f64) as f32;
                    if m1_de < 0.0 {
                        m1_de = 0.0001_f64 as f32;
                    }
                } else {
                    m1_de = (1.0 - (val - m1_coeffi) as f64 * 0.0003 * self.m_val as f64) as f32;
                }
                mdct[i] *= m1_de;
            }
        }
        if mp3.mdctbuf_flag {
            const MAG: usize = 8;
            match block_mode {
                0 | 1 => {
                    if nw_modenumber != 0 {
                        for i in 0..n {
                            lastmdct[i * MAG..i * MAG + MAG].fill(logmdct[i]);
                        }
                    } else {
                        lastmdct[..n].copy_from_slice(&logmdct[..n]);
                    }
                }
                2 => {
                    if nw_modenumber == 0 {
                        for i in 0..n >> 3 {
                            let ni = i * MAG;
                            lastmdct[i] = logmdct[ni];
                            for j in 1..MAG {
                                if lastmdct[i] > logmdct[ni + j] {
                                    lastmdct[i] = logmdct[ni + j];
                                }
                            }
                        }
                    } else {
                        lastmdct[..n].copy_from_slice(&logmdct[..n]);
                    }
                }
                3 => lastmdct[..n].copy_from_slice(&logmdct[..n]),
                _ => {}
            }
        }
    }

    /// `lb_loudnoise_fix`: the noise compand level of a long block next to a transition.
    pub(crate) fn lb_loudnoise_fix(
        &self,
        noise_compand_level: f32,
        logmdct: &[f32],
        block_mode: i32,
        lw_block_mode: i32,
    ) -> f32 {
        if (self.m_val as f64) < 0.5 || self.vi.normal_thresh > 0.45 {
            return -1.0;
        }
        if !((block_mode == 2 && lw_block_mode == 3) || (block_mode == 3 && lw_block_mode == 2)) {
            return noise_compand_level;
        }
        let mut hi_th = 0f64;
        for &v in &logmdct[self.n25p as usize..self.n75p as usize] {
            hi_th += if v > -130.0 { v as f64 } else { -130. };
        }
        hi_th /= self.n as f64;
        if hi_th > -40. {
            -1.0
        } else if hi_th < -50. {
            1.0
        } else {
            (1. - ((hi_th + 50.) / 10.)) as f32
        }
    }
}

// ---- channel coupling, quantisation and noise normalisation (`_vp_couple_quantize_normalize`)

/// `existe(x, y)`: |x| ≥ y, in double (the C macro's promotion).
#[inline]
fn existe(x: f32, y: f64) -> bool {
    (x as f64) < -y || x as f64 >= y
}

/// `refer_phase(a, b)`: opposite signs.
#[inline]
fn refer_phase(a: f32, b: f32) -> bool {
    (a > 0.0 && b < 0.0) || (b > 0.0 && a < 0.0)
}

/// `flag_lossless`: per line of a partition starting at `i`, the residue `mdct/floor` and its
/// flag: 1 coded losslessly (above the point), -1 between the two thresholds, 0 below.
#[allow(clippy::too_many_arguments)]
fn flag_lossless(
    limit: i32,
    prepoint: f32,
    postpoint: f32,
    prepoint_r: f32,
    postpoint_r: f32,
    res: &mut [f32],
    mdct: &[f32],
    enpeak: &[f32],
    floor: &[f32],
    flag: &mut [i32],
    i: i32,
    jn: usize,
) {
    let pointlimit = limit - i;
    let (mut point1, mut point2);
    let (mut ps1, mut ps2) = (0f32, 0f32);
    let mut ps = false;
    if pointlimit > 0 {
        point1 = prepoint;
        point2 = prepoint_r;
        if pointlimit - jn as i32 <= 0 {
            ps1 = (postpoint - prepoint) / jn as f32;
            ps2 = (postpoint_r - prepoint_r) / jn as f32;
            ps = true;
        }
    } else {
        point1 = postpoint;
        point2 = postpoint_r;
    }
    for j in 0..jn {
        if ps {
            point1 += ps1;
            point2 += ps2;
        }
        let bakp1 = point1;
        res[j] = mdct[j] / floor[j];
        let r = res[j].abs();
        point1 -= enpeak[j];
        if point1 < prepoint {
            point1 = prepoint;
        }
        flag[j] = if r < point1 {
            if r < point2 { 0 } else { -1 }
        } else {
            1
        };
        point1 = bakp1;
    }
}

/// `lossless_coupling`: (magnitude, angle) of the integer pair.
fn lossless_coupling(a: i32, b: i32) -> (i32, i32) {
    let (mut mag, mut ang) = if a.wrapping_abs() > b.wrapping_abs() {
        (
            a,
            if a > 0 {
                a.wrapping_sub(b)
            } else {
                b.wrapping_sub(a)
            },
        )
    } else {
        (
            b,
            if b > 0 {
                a.wrapping_sub(b)
            } else {
                b.wrapping_sub(a)
            },
        )
    };
    if ang >= mag.wrapping_abs().wrapping_mul(2) {
        ang = ang.wrapping_neg();
        mag = mag.wrapping_neg();
    }
    (mag, ang)
}

/// `lossless_couplingf`: the same on floats (`fabs` in double).
fn lossless_couplingf(a: f32, b: f32) -> (f32, f32) {
    let (mut mag, mut ang) = if (a as f64).abs() > (b as f64).abs() {
        (a, if a > 0.0 { a - b } else { b - a })
    } else {
        (b, if b > 0.0 { a - b } else { b - a })
    };
    if ang as f64 >= (mag as f64).abs() * 2.0 {
        ang = -ang;
        mag = -mag;
    }
    (mag, ang)
}

/// `min_indemnity_dipole_hypot`: the coupled magnitude of a point-stereo pair.
fn min_indemnity_dipole_hypot(a: f32, b: f32, threv: f32) -> f32 {
    let thnor = 0.94f32;
    let a2 = (a * thnor).abs();
    let b2 = (b * thnor).abs();
    if a as f64 > 0.0 {
        if b as f64 > 0.0 {
            return a2 + b2;
        }
        if a > -b {
            return a2 - b2 * threv;
        }
        return -(b2 - a2 * threv);
    }
    if (b as f64) < 0.0 {
        return -(a2 + b2);
    }
    if -a > b {
        return -(a2 - b2 * threv);
    }
    b2 - a2 * threv
}

/// `ssort`: the first `min(range, bthresh)` places of `sort` by descending `q` (selection
/// sort, the first of equals kept).
fn ssort(range: usize, bthresh: usize, sort: &mut [usize], q: &[f32]) {
    let bthresh = bthresh.min(range);
    for i in 0..bthresh {
        let mut large = i;
        for j in i + 1..range {
            if q[sort[large]] < q[sort[j]] {
                large = j;
            }
        }
        sort.swap(i, large);
    }
}

impl PsyLook {
    /// `noise_normalize`: quantise one partition's residue (`out`), the lines under ½ pooled
    /// and re-spent as ±1 by energy while the pool lasts; returns the pool left.
    #[allow(clippy::too_many_arguments)]
    fn noise_normalize(
        &self,
        limit: i32,
        r: &[f32],
        q: &mut [f32],
        f: &[f32],
        res: &mut [f32],
        flags: Option<&[i32]>,
        nepeak: f32,
        i: i32,
        n: usize,
        out: &mut [i32],
    ) -> f32 {
        let vi = &self.vi;
        let mut sort = Vec::with_capacity(n);
        let mut start = if vi.normal_p != 0 {
            (vi.normal_start - i) as i64
        } else {
            n as i64
        };
        if start > n as i64 || (nepeak as f64) < -0.5 {
            start = n as i64;
        }
        let mut acc = 0f32;
        let mut j = 0usize;
        // `j < start` with a signed start (it may be negative past normal_start)
        let below = |j: usize| (j as i64) < start;
        match flags {
            None => {
                while below(j) {
                    out[j] = rint(res[j]);
                    j += 1;
                }
            }
            Some(flags) => {
                while below(j) {
                    if flags[j] != 1 {
                        let ve = ((q[j] / f[j]) as f64).sqrt() as f32;
                        if r[j] < 0.0 {
                            out[j] = -(rint_f64(ve as f64) as i32);
                            res[j] = -ve;
                        } else {
                            out[j] = rint(ve);
                            res[j] = ve;
                        }
                    }
                    j += 1;
                }
            }
        }
        match flags {
            Some(flags) => {
                while j < n {
                    if flags[j] == 1 {
                        j += 1;
                        continue;
                    }
                    let mut ve = q[j] / f[j];
                    if ve < 0.25 && j as i32 >= limit - i {
                        acc += ve;
                        sort.push(j);
                        let s = (ve as f64).sqrt() as f32;
                        res[j] = if r[j] < 0.0 { -s } else { s };
                    } else {
                        ve = (ve as f64).sqrt() as f32;
                        if r[j] < 0.0 {
                            out[j] = -(rint_f64(ve as f64) as i32);
                            res[j] = -ve;
                        } else {
                            out[j] = rint(ve);
                            res[j] = ve;
                        }
                        q[j] = out[j].wrapping_mul(out[j]) as f32 * f[j];
                    }
                    j += 1;
                }
            }
            None => {
                while j < n {
                    let ve = res[j] * res[j];
                    if ve < 0.25 {
                        acc += ve;
                        sort.push(j);
                    } else {
                        out[j] = rint(res[j]);
                        q[j] = out[j].wrapping_mul(out[j]) as f32 * f[j];
                    }
                    j += 1;
                }
            }
        }
        acc += acc * nepeak * nepeak;
        let count = sort.len();
        if count > 0 {
            let iacc = ((acc as i32) + 1).min(n as i32);
            ssort(count, iacc.max(0) as usize, &mut sort, q);
            for &e in &sort {
                if acc as f64 >= vi.normal_thresh {
                    out[e] = unitnorm(r[e]) as i32;
                    acc -= 1.0;
                    q[e] = f[e];
                } else {
                    out[e] = 0;
                    q[e] = 0.0;
                }
            }
        }
        acc
    }

    /// `_vp_couple_quantize_normalize`: for packet blob `blobno`, the residue of each channel
    /// (MDCT over the integer floor `iwork`), quantised and noise-normalised per partition
    /// below `lowpassr`, the coupled pairs as magnitude/angle (lossless) or point stereo;
    /// `iwork` becomes the integer residue, `nonzero` is spread over coupled pairs.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn couple_quantize_normalize(
        &self,
        blobno: usize,
        g: &InfoPsyGlobal,
        vi: &InfoMapping0,
        mdct: &[Vec<f32>],
        enpeak: &[Vec<f32>],
        nepeak: &mut [Vec<f32>],
        iwork: &mut [Vec<i32>],
        nonzero: &mut [bool],
        sliding_lowpass: i32,
        ch: usize,
        lowpassr: i32,
    ) {
        let n = self.n as i32;
        let partition = self.vi.partition();
        let limit = g.coupling_pointlimit[self.vi.blockflag as usize][blobno];
        let prepoint = STEREO_THRESHHOLDS[g.coupling_prepointamp[blobno] as usize] as f32;
        let postpoint = STEREO_THRESHHOLDS[g.coupling_postpointamp[blobno] as usize] as f32;
        let mut prepoint_x = STEREO_THRESHHOLDS_X[g.coupling_prepointamp[blobno] as usize] as f32;
        let mut postpoint_x = STEREO_THRESHHOLDS_X[g.coupling_postpointamp[blobno] as usize] as f32;
        let steps = vi.coupling_steps as usize;
        let mut raw = vec![0f32; ch * partition];
        let mut quant = vec![0f32; ch * partition];
        let mut floor = vec![0f32; ch * partition];
        let mut res = vec![0f32; ch * partition];
        let mut flag = vec![0i32; ch * partition];
        let mut nz = vec![false; ch];
        let mut side_resdef = vec![-1f32; steps];
        if prepoint_x < prepoint {
            prepoint_x = prepoint;
        }
        if postpoint_x < prepoint {
            postpoint_x = prepoint;
        }
        let prae = if steps == 1 {
            0.34f64 as f32
        } else {
            0.825f64 as f32
        };
        let mut i = 0i32;
        let mut pi = 0usize;
        while i < lowpassr {
            let iu = i as usize;
            let jn = partition.min((n - i) as usize);
            nz.copy_from_slice(nonzero);
            flag.fill(0);
            for k in 0..ch {
                let o = k * partition;
                let iout = &mut iwork[k][iu..];
                if nz[k] {
                    for j in 0..jn {
                        floor[o + j] = FLOOR1_FROMDB_LOOKUP[iout[j] as usize];
                    }
                    flag_lossless(
                        limit,
                        prepoint,
                        postpoint,
                        prepoint_x,
                        postpoint_x,
                        &mut res[o..o + jn],
                        &mdct[k][iu..iu + jn],
                        &enpeak[k][iu..iu + jn],
                        &floor[o..o + jn],
                        &mut flag[o..o + jn],
                        i,
                        jn,
                    );
                    for j in 0..jn {
                        let m = mdct[k][iu + j];
                        raw[o + j] = m * m;
                        quant[o + j] = raw[o + j];
                        if m < 0.0 {
                            raw[o + j] *= -1.0;
                        }
                        floor[o + j] *= floor[o + j];
                    }
                    self.noise_normalize(
                        limit,
                        &raw[o..o + jn],
                        &mut quant[o..o + jn],
                        &floor[o..o + jn],
                        &mut res[o..o + jn],
                        None,
                        nepeak[k][pi],
                        i,
                        jn,
                        iout,
                    );
                } else {
                    for j in 0..jn {
                        floor[o + j] = 1e-10;
                        raw[o + j] = 0.0;
                        quant[o + j] = 0.0;
                        res[o + j] = 0.0;
                        flag[o + j] = 0;
                        iout[j] = 0;
                    }
                }
            }
            for step in 0..steps {
                let mi = vi.coupling_mag[step] as usize;
                let ai = vi.coupling_ang[step] as usize;
                let (om, oa) = (mi * partition, ai * partition);
                let mut pointflag = false;
                if !(nz[mi] || nz[ai]) {
                    continue;
                }
                nz[mi] = true;
                nz[ai] = true;
                if self.tonefix_end > i {
                    let (mut rp, mut pp) = (0i32, 0i32);
                    let mut residue_def = 0f32;
                    for j in 0..jn {
                        if existe(res[om + j], 0.5) || existe(res[oa + j], 0.5) {
                            if refer_phase(raw[om + j], raw[oa + j]) {
                                rp += 1;
                            } else {
                                pp += 1;
                            }
                            residue_def = (residue_def as f64
                                + ((res[om + j].abs() as f64) - (res[oa + j].abs() as f64)).abs())
                                as f32;
                        }
                    }
                    let ap = rp + pp;
                    if ap != 0 {
                        residue_def /= ap as f32;
                        let temp_def = residue_def;
                        if side_resdef[step] > 0.0 {
                            residue_def =
                                (temp_def as f64 * 0.5 + side_resdef[step] as f64 * 0.5) as f32;
                        }
                        side_resdef[step] = temp_def;
                        if residue_def > 1.0 {
                            for j in 0..jn {
                                if flag[om + j] == -1 || flag[oa + j] == -1 {
                                    flag[om + j] = 1;
                                }
                            }
                        }
                        if rp as f32 / ap as f32 >= prae {
                            for j in 0..jn {
                                if (flag[om + j] == -1 || flag[oa + j] == -1)
                                    && refer_phase(raw[om + j], raw[oa + j])
                                {
                                    flag[om + j] = 1;
                                }
                            }
                        }
                    } else {
                        side_resdef[step] = -1.0;
                    }
                }
                for j in 0..jn {
                    if (j as i32) < sliding_lowpass - i {
                        if flag[om + j] == 1 || flag[oa + j] == 1 {
                            raw[om + j] = raw[om + j].abs() + raw[oa + j].abs();
                            quant[om + j] += quant[oa + j];
                            flag[om + j] = 1;
                            flag[oa + j] = 1;
                            (res[om + j], res[oa + j]) =
                                lossless_couplingf(res[om + j], res[oa + j]);
                            let (m, a) = lossless_coupling(iwork[mi][iu + j], iwork[ai][iu + j]);
                            iwork[mi][iu + j] = m;
                            iwork[ai][iu + j] = a;
                        } else {
                            let (hpl, hph) = if steps == 1 || step == 3 {
                                (0.18f32, 0.12f32)
                            } else {
                                (0.18, 0.04)
                            };
                            let threv = if (j as i32) < limit - i { hpl } else { hph };
                            raw[om + j] =
                                min_indemnity_dipole_hypot(raw[om + j], raw[oa + j], threv);
                            quant[om + j] = raw[om + j].abs();
                            raw[oa + j] = 0.0;
                            quant[oa + j] = 0.0;
                            flag[oa + j] = 1;
                            iwork[ai][iu + j] = 0;
                            res[oa + j] = 0.0;
                            if (nepeak[mi][pi] as f64) < -0.5 || (nepeak[ai][pi] as f64) < -0.5 {
                                nepeak[mi][pi] = -1.0;
                            } else {
                                // `min(x, y)` (os.h): x > y ? y : x
                                let (x, y) = (nepeak[mi][pi], nepeak[ai][pi]);
                                nepeak[mi][pi] = if x > y { y } else { x };
                            }
                            pointflag = true;
                        }
                    }
                    let s = floor[om + j] + floor[oa + j];
                    floor[om + j] = s;
                    floor[oa + j] = s;
                }
                if pointflag {
                    self.noise_normalize(
                        limit,
                        &raw[om..om + jn],
                        &mut quant[om..om + jn],
                        &floor[om..om + jn],
                        &mut res[om..om + jn],
                        Some(&flag[om..om + jn]),
                        nepeak[mi][pi],
                        i,
                        jn,
                        &mut iwork[mi][iu..],
                    );
                }
            }
            i += partition as i32;
            pi += 1;
        }
        if lowpassr < n {
            for w in iwork.iter_mut().take(ch) {
                w[lowpassr as usize..n as usize].fill(0);
            }
        }
        for s in 0..steps {
            let (m, a) = (vi.coupling_mag[s] as usize, vi.coupling_ang[s] as usize);
            if nonzero[m] || nonzero[a] {
                nonzero[m] = true;
                nonzero[a] = true;
            }
        }
    }
}

#[cfg(all(test, oracle))]
mod vs_c {
    use super::*;
    use crate::setup::CodecSetup;
    use crate::testutil::bits;
    use aotuv_sys::Mode;

    /// The C's dump order (`aotuv_psy_look` in aotuv-sys `shim.c`).
    fn dump(p: &PsyLook) -> Vec<f64> {
        let nn25pt = p.vi.normal_partition / 4;
        let mut o = vec![
            p.n as f64,
            p.firstoc as f64,
            p.shiftoc as f64,
            p.eighth_octave_lines as f64,
            p.total_octave_lines as f64,
            p.rate as f64,
            p.m3n[0] as f64,
            p.m3n[1] as f64,
            p.m3n[2] as f64,
            p.m_val as f64,
            p.tonecomp_endp as f64,
            p.tonecomp_thres as f64,
            p.min_nn_lp as f64,
            0.0, // endeg_start
            0.0, // endeg_end
            p.tonefix_end as f64,
            p.n25p as f64,
            p.n33p as f64,
            p.n75p as f64,
            (nn25pt * 3) as f64,
            (nn25pt + nn25pt) as f64,
            nn25pt as f64,
            0.,
            0.,
            0.,
        ];
        o.extend(p.ath.iter().map(|&x| x as f64));
        o.extend(p.octave.iter().map(|&x| x as f64));
        o.extend(p.bark.iter().map(|&x| x as f64));
        o.extend(p.ntfix_noiseoffset.iter().map(|&x| x as f64));
        for v in &p.noiseoffset {
            o.extend(v.iter().map(|&x| x as f64));
        }
        for band in &p.tonecurves {
            for c in band {
                o.extend(c.iter().map(|&x| x as f64));
            }
        }
        o
    }

    /// Every psy block of every template family (rates at and between the edges, 1/2/6
    /// channels, qualities across the range, managed): the whole look bit for bit.
    #[test]
    fn psy_look_matches_c() {
        let mut n = 0;
        for rate in [
            8000i64, 11025, 16000, 22050, 26000, 32000, 38000, 44100, 46000, 48000, 96000, 192000,
        ] {
            for ch in [1usize, 2, 6] {
                let mut modes: Vec<Mode> =
                    [-0.1f32, 0.0, 0.25, 0.5, 0.77, 1.0].map(Mode::Vbr).to_vec();
                modes.push(Mode::Managed {
                    min: -1,
                    nominal: 64_000 * ch as i64,
                    max: -1,
                });
                for mode in modes {
                    let ci = match mode {
                        Mode::Vbr(q) => CodecSetup::vbr(ch as i64, rate, q),
                        Mode::Managed { min, nominal, max } => {
                            CodecSetup::managed(ch as i64, rate, max, nominal, min)
                        }
                    };
                    let Ok(ci) = ci else { continue };
                    for (b, vi) in ci.psy_param.iter().enumerate() {
                        let c = aotuv_sys::psy_look(ch, rate, mode, b).unwrap();
                        let bs = ci.blocksizes[vi.blockflag as usize] as usize / 2;
                        let r = dump(&PsyLook::new(vi, &ci.psy_g_param, bs, rate));
                        if let Some(k) = r
                            .iter()
                            .zip(&c)
                            .position(|(a, b)| a.to_bits() != b.to_bits())
                        {
                            panic!(
                                "{rate} Hz {ch} ch {mode:?} block {b} (n {}): value {k}: {} vs C {}",
                                r[0], r[k], c[k]
                            );
                        }
                        assert_eq!(r.len(), c.len());
                        n += 1;
                    }
                }
            }
        }
        println!("vorbis psy look: {n} looks bit-identical");
    }

    /// Realistic spectra: `thetest` audio at `rate`, 2n-sample windows from `at`, through the
    /// (verified) MDCT and FFT, in the dB forms mapping0 gives the psy model.
    fn spectra(pcm: &[f32], at: usize, n: usize) -> (Vec<f32>, Vec<f32>, Vec<f32>, f32) {
        use crate::scales::todb;
        let m = crate::mdct::Mdct::new(2 * n);
        let mut x: Vec<f32> = (0..2 * n)
            .map(|i| {
                let w = ((i as f64 + 0.5) / (2 * n) as f64 * std::f64::consts::PI).sin() as f32;
                pcm.get(at + i).copied().unwrap_or(0.0) * w
            })
            .collect();
        let mut mdct = vec![0f32; n];
        m.forward(&x, &mut mdct);
        let logmdct: Vec<f32> = mdct
            .iter()
            .map(|&v| (todb(v) as f64 + 0.345) as f32)
            .collect();
        crate::smallft::Drft::new(2 * n).forward(&mut x);
        let mut logfft = vec![0f32; n];
        logfft[0] = (todb(x[0]) as f64 + 0.345) as f32;
        let mut lmax = logfft[0];
        let mut j = 1;
        while j < 2 * n - 1 {
            let t = x[j] * x[j] + x[j + 1] * x[j + 1];
            let v = ((0.5 * todb(t)) as f64 + 0.345) as f32;
            logfft[(j + 1) >> 1] = v;
            if v > lmax {
                lmax = v;
            }
            j += 2;
        }
        (mdct, logmdct, logfft, lmax.min(0.0))
    }

    /// The per-block functions chained as mapping0 chains them, on block sequences per
    /// configuration and psy block (12 rates... 1/2 ch, qualities, managed; real audio, its
    /// silences and random states): `lb_loudnoise_fix`, `_vp_noisemask`, `_vp_tonemask`,
    /// `_vp_offset_and_mix` (selects 1, 2, 0), every output and the carried `lastmdct` /
    /// `tempmdct` bit for bit.
    #[test]
    fn psy_block_functions_match_c() {
        use crate::testutil::Rng;
        let mut r = Rng::new(18);
        let (_, a16) = aotuv_sys::thetest(48);
        let audio: Vec<f32> = a16.iter().map(|&s| s as f32 / 32768.0).collect();
        let mut calls = 0;
        for rate in [8000i64, 16000, 22050, 32000, 44100, 48000, 96000] {
            for (ch, mode) in [
                (1usize, Mode::Vbr(0.1)),
                (2, Mode::Vbr(0.6)),
                (1, Mode::Vbr(1.0)),
                (
                    2,
                    Mode::Managed {
                        min: -1,
                        nominal: 96_000,
                        max: -1,
                    },
                ),
            ] {
                let Ok(ci) = (match mode {
                    Mode::Vbr(q) => CodecSetup::vbr(ch as i64, rate, q),
                    Mode::Managed { min, nominal, max } => {
                        CodecSetup::managed(ch as i64, rate, max, nominal, min)
                    }
                }) else {
                    continue;
                };
                for (b, vi) in ci.psy_param.iter().enumerate() {
                    let c = aotuv_sys::Psy::new(ch, rate, mode, b).unwrap();
                    let n = ci.blocksizes[vi.blockflag as usize] as usize / 2;
                    let p = PsyLook::new(vi, &ci.psy_g_param, n, rate);
                    assert_eq!(c.n(), n);
                    let partition = vi.partition();
                    let np = n.div_ceil(partition);
                    let (mut last, mut lastc) = (vec![-140f32; 2048], vec![-140f32; 2048]);
                    let (mut temp, mut tempc) = (vec![-140f32; 256], vec![-140f32; 256]);
                    let (mut ncl, mut nclc) = (0f32, 0f32);
                    let block_mode = b as i32;
                    for f in 0..40 {
                        let at = (r.u64() % (audio.len() as u64 - 4 * n as u64)) as usize;
                        let (mut mdct, mut logmdct, logfft, lmax) = if f % 9 == 4 {
                            // silence: the dB floor
                            (
                                vec![0f32; n],
                                vec![(crate::scales::todb(0.0) as f64 + 0.345) as f32; n],
                                vec![-700f32; n],
                                -700f32,
                            )
                        } else {
                            spectra(&audio, at, n)
                        };
                        let gmax = lmax.max(-20.0 + (r.u64() % 20) as f32);
                        let lw_block_mode = (r.u64() % 4) as i32;
                        let lw_no = 1 + (r.u64() % 10) as i32;
                        let impadnum = (r.u64() % 4) as i32 * (f % 3 == 0) as i32;
                        let nw = (r.u64() % 2) as i32;
                        let end_block = (r.u64() % (n as u64 + 1)) as i32;
                        let poste = if f % 4 == 1 {
                            (r.u64() % 50) as f32
                        } else {
                            -1.0
                        };
                        let managed = f % 2 == 1;

                        ncl = p.lb_loudnoise_fix(ncl, &logmdct, block_mode, lw_block_mode);
                        nclc = c.loudnoise(nclc, &logmdct, block_mode, lw_block_mode);
                        assert_eq!(
                            ncl.to_bits(),
                            nclc.to_bits(),
                            "loudnoise {rate} {mode:?} b{b} f{f}"
                        );

                        let (mut ep, mut epc) = (vec![0f32; n], vec![0f32; n]);
                        let (mut npk, mut npkc) = (vec![0f32; np], vec![0f32; np]);
                        let (mut noise, mut noisec) = (vec![0f32; n], vec![0f32; n]);
                        p.noisemask(
                            ncl, &logmdct, &last, &mut ep, &mut npk, &mut noise, poste, block_mode,
                        );
                        c.noisemask(
                            nclc,
                            &logmdct,
                            &lastc,
                            &mut epc,
                            &mut npkc,
                            &mut noisec,
                            poste,
                            block_mode,
                        );
                        let at_ = format!("{rate} Hz {ch} ch {mode:?} block {b} frame {f}");
                        assert_eq!(bits(&noise), bits(&noisec), "noisemask logmask {at_}");
                        assert_eq!(bits(&ep), bits(&epc), "noisemask epeak {at_}");
                        assert_eq!(bits(&npk), bits(&npkc), "noisemask npeak {at_}");

                        let (mut tone, mut tonec) = (vec![0f32; n], vec![0f32; n]);
                        p.tonemask(&logfft, &mut tone, gmax, lmax);
                        c.tonemask(&logfft, &mut tonec, gmax, lmax);
                        assert_eq!(bits(&tone), bits(&tonec), "tonemask {at_}");

                        let selects: &[usize] = if managed { &[1, 2, 0] } else { &[1] };
                        let mut mdctc = mdct.clone();
                        let mut logmdctc = logmdct.clone();
                        for &sel in selects {
                            let (mut m, mut mc) = (vec![0f32; n], vec![0f32; n]);
                            p.offset_and_mix(
                                &noise,
                                &tone,
                                sel,
                                managed,
                                &mut m,
                                &mut mdct,
                                &logmdct,
                                &mut last,
                                &mut temp,
                                ncl,
                                &mut npk,
                                end_block,
                                block_mode,
                                nw as usize,
                                lw_block_mode,
                                lw_no,
                                impadnum,
                            );
                            c.offset_and_mix(
                                &noisec,
                                &tonec,
                                sel as i32,
                                managed,
                                &mut mc,
                                &mut mdctc,
                                &mut logmdctc,
                                &mut lastc,
                                &mut tempc,
                                nclc,
                                &mut npkc,
                                end_block,
                                block_mode,
                                nw,
                                lw_block_mode,
                                lw_no,
                                impadnum,
                            );
                            let at2 = format!("{at_} select {sel}");
                            assert_eq!(bits(&m), bits(&mc), "offset_and_mix logmask {at2}");
                            assert_eq!(bits(&mdct), bits(&mdctc), "offset_and_mix mdct {at2}");
                            assert_eq!(bits(&npk), bits(&npkc), "offset_and_mix npeak {at2}");
                            assert_eq!(bits(&last), bits(&lastc), "offset_and_mix lastmdct {at2}");
                            assert_eq!(bits(&temp), bits(&tempc), "offset_and_mix tempmdct {at2}");
                            calls += 1;
                        }
                        let _ = &mut logmdct;
                    }
                }
            }
        }
        // _postnoise_detection: quiet second halves after loud first ones, every mode
        for k in 0..2000u64 {
            let nn = [1024usize, 2048, 4096][(k % 3) as usize];
            let loud = [0.0f32, 1e-3, 0.02, 0.5][(k % 4) as usize];
            let quiet = [0.0f32, 1e-5, 1e-3, 0.01][((k / 4) % 4) as usize];
            let pcm: Vec<f32> = (0..nn)
                .map(|i| r.unit() * if i < nn / 2 { loud } else { quiet })
                .collect();
            let (mode, lw) = ((k % 5) as i32 % 4, ((k / 5) % 2) as i32);
            let a = postnoise_detection(&pcm, nn, mode, lw);
            let b = aotuv_sys::postnoise(&pcm, mode, lw);
            assert_eq!(a.to_bits(), b.to_bits(), "postnoise {k}");
        }
        println!("vorbis psy blocks: {calls} chained calls bit-identical");
    }

    /// ns per call of the per-block psy functions, Rust vs the C's own (the strict build), on
    /// 64 long blocks of 44.1 kHz audio (q 0.5 stereo, the long-block look): `BENCH=kern
    /// CRATE=vorbis-enc just bench`. Ignored: timing, not a test.
    #[test]
    #[ignore]
    fn kernel_timing() {
        use std::hint::black_box;
        use std::time::Instant;
        let mode = Mode::Vbr(0.5);
        let ci = CodecSetup::vbr(2, 44100, 0.5).unwrap();
        let b = 3;
        let vi = &ci.psy_param[b];
        let n = ci.blocksizes[1] as usize / 2;
        let p = PsyLook::new(vi, &ci.psy_g_param, n, 44100);
        let c = aotuv_sys::Psy::new(2, 44100, mode, b).unwrap();
        let (_, a16) = aotuv_sys::thetest(44);
        let audio: Vec<f32> = a16.iter().map(|&s| s as f32 / 32768.0).collect();
        let frames: Vec<_> = (0..64)
            .map(|f| spectra(&audio, 4096 + f * 1777, n))
            .collect();
        let np = n.div_ceil(vi.normal_partition as usize);
        let last = vec![-140f32; 2048];
        let time = |name: &str, f: &mut dyn FnMut()| {
            let best = (0..9)
                .map(|_| {
                    let t = Instant::now();
                    f();
                    t.elapsed().as_secs_f64()
                })
                .fold(f64::INFINITY, f64::min);
            println!("{name:<22} {:9.1} ns", best / frames.len() as f64 * 1e9);
        };
        let (mut ep, mut npk, mut noise) = (vec![0f32; n], vec![0f32; np], vec![0f32; n]);
        time("noisemask rust", &mut || {
            for fr in &frames {
                p.noisemask(1.0, &fr.1, &last, &mut ep, &mut npk, &mut noise, -1.0, 3);
                black_box(&noise);
            }
        });
        time("noisemask c", &mut || {
            for fr in &frames {
                c.noisemask(1.0, &fr.1, &last, &mut ep, &mut npk, &mut noise, -1.0, 3);
                black_box(&noise);
            }
        });
        let mut tone = vec![0f32; n];
        time("tonemask rust", &mut || {
            for fr in &frames {
                p.tonemask(&fr.2, &mut tone, fr.3, fr.3);
                black_box(&tone);
            }
        });
        time("tonemask c", &mut || {
            for fr in &frames {
                c.tonemask(&fr.2, &mut tone, fr.3, fr.3);
                black_box(&tone);
            }
        });
        let (mut m, mut mdct, mut lastm, mut temp) = (
            vec![0f32; n],
            vec![0f32; n],
            vec![-140f32; 2048],
            vec![-140f32; 256],
        );
        let mut lm = vec![0f32; n];
        time("offset_and_mix rust", &mut || {
            for fr in &frames {
                mdct.copy_from_slice(&fr.0);
                lm.copy_from_slice(&fr.1);
                p.offset_and_mix(
                    &noise, &tone, 1, false, &mut m, &mut mdct, &fr.1, &mut lastm, &mut temp, 1.0,
                    &mut npk, n as i32, 3, 1, 3, 2, 0,
                );
                black_box(&m);
            }
        });
        time("offset_and_mix c", &mut || {
            for fr in &frames {
                mdct.copy_from_slice(&fr.0);
                lm.copy_from_slice(&fr.1);
                c.offset_and_mix(
                    &noise, &tone, 1, false, &mut m, &mut mdct, &mut lm, &mut lastm, &mut temp,
                    1.0, &mut npk, n as i32, 3, 1, 3, 2, 0,
                );
                black_box(&m);
            }
        });
    }
}
