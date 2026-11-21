//! The forward MDCT (`mdct.c`): `n` windowed samples to `n/2` coefficients, a split-radix
//! butterfly network over `n/2` points. Ported in the C's operation order (no contraction).

use core::f64::consts::PI;

/// `cPI3_8`, `cPI2_8`, `cPI1_8` (float literals in the C).
#[allow(clippy::excessive_precision)] // the C's literals as written
const PI3_8: f32 = 0.382_683_432_365_089_771_75;
#[allow(clippy::excessive_precision, clippy::approx_constant)] // the C's literal
const PI2_8: f32 = 0.707_106_781_186_547_524_41;
#[allow(clippy::excessive_precision)] // the C's literal
const PI1_8: f32 = 0.923_879_532_511_286_756_13;

/// `mdct_lookup`: the twiddles (`trig`: n + n/4), the bit reversal (`bitrev`: n/4) and the
/// output scale `4/n`.
#[derive(Clone, Debug)]
pub(crate) struct Mdct {
    n: usize,
    log2n: u32,
    pub(crate) trig: Vec<f32>,
    bitrev: Vec<usize>,
    scale: f32,
}

impl Mdct {
    /// `mdct_init`: `n` a power of two, 64…8192 (Vorbis block sizes; the envelope's 128).
    pub(crate) fn new(n: usize) -> Self {
        assert!(n.is_power_of_two() && (64..=8192).contains(&n));
        let n2 = n >> 1;
        // `rint(log((float)n)/log(2.f))`: exact for a power of two
        let log2n = n.trailing_zeros();
        let mut t = vec![0f32; n + n / 4];
        // the C's double expressions, rounded to float on store (`FLOAT_CONV`)
        for i in 0..n / 4 {
            let a = (PI / n as f64) * (4 * i) as f64;
            let b = (PI / (2 * n) as f64) * (2 * i + 1) as f64;
            t[i * 2] = a.cos() as f32;
            t[i * 2 + 1] = (-a.sin()) as f32;
            t[n2 + i * 2] = b.cos() as f32;
            t[n2 + i * 2 + 1] = b.sin() as f32;
        }
        for i in 0..n / 8 {
            let a = (PI / n as f64) * (4 * i + 2) as f64;
            t[n + i * 2] = (a.cos() * 0.5) as f32;
            t[n + i * 2 + 1] = (-a.sin() * 0.5) as f32;
        }
        let mask = (1usize << (log2n - 1)) - 1;
        let msb = 1usize << (log2n - 2);
        let mut bitrev = vec![0usize; n / 4];
        for i in 0..n / 8 {
            let mut acc = 0;
            let mut j = 0;
            while msb >> j != 0 {
                if (msb >> j) & i != 0 {
                    acc |= 1 << j;
                }
                j += 1;
            }
            // acc < mask (bit 0 is never set: i < n/8), so this is ≥ 0
            bitrev[i * 2] = ((!acc) & mask) - 1;
            bitrev[i * 2 + 1] = acc;
        }
        Mdct {
            n,
            log2n,
            trig: t,
            bitrev,
            scale: 4.0 / n as f32,
        }
    }

    /// `mdct_forward`: `inp[..n]` → `out[..n/2]`.
    pub(crate) fn forward(&self, inp: &[f32], out: &mut [f32]) {
        let n = self.n;
        let (n2, n4, n8) = (n >> 1, n >> 2, n >> 3);
        let inp = &inp[..n];
        let out = &mut out[..n2];
        let tr = &self.trig;
        let mut w = vec![0f32; n];
        {
            let w2 = &mut w[n2..];
            let mut x0 = n2 + n4;
            let mut x1 = x0 + 1;
            let mut t = n2;
            let mut i = 0;
            while i < n8 {
                x0 -= 4;
                t -= 2;
                let r0 = inp[x0 + 2] + inp[x1];
                let r1 = inp[x0] + inp[x1 + 2];
                w2[i] = r1 * tr[t + 1] + r0 * tr[t];
                w2[i + 1] = r1 * tr[t] - r0 * tr[t + 1];
                x1 += 4;
                i += 2;
            }
            x1 = 1;
            while i < n2 - n8 {
                t -= 2;
                x0 -= 4;
                let r0 = inp[x0 + 2] - inp[x1];
                let r1 = inp[x0] - inp[x1 + 2];
                w2[i] = r1 * tr[t + 1] + r0 * tr[t];
                w2[i + 1] = r1 * tr[t] - r0 * tr[t + 1];
                x1 += 4;
                i += 2;
            }
            x0 = n;
            while i < n2 {
                t -= 2;
                x0 -= 4;
                let r0 = -inp[x0 + 2] - inp[x1];
                let r1 = -inp[x0] - inp[x1 + 2];
                w2[i] = r1 * tr[t + 1] + r0 * tr[t];
                w2[i + 1] = r1 * tr[t] - r0 * tr[t + 1];
                x1 += 4;
                i += 2;
            }
            self.butterflies(w2, n2);
        }
        self.bitreverse(&mut w);
        let mut t = n2;
        let mut x0 = n2;
        for i in 0..n4 {
            x0 -= 1;
            let (a, b) = (w[2 * i], w[2 * i + 1]);
            out[i] = (a * tr[t] + b * tr[t + 1]) * self.scale;
            out[x0] = (a * tr[t + 1] - b * tr[t]) * self.scale;
            t += 2;
        }
    }

    /// `mdct_butterflies`: the radix stages over `x[..points]`.
    fn butterflies(&self, x: &mut [f32], points: usize) {
        let tr = &self.trig;
        let mut stages = self.log2n as i32 - 5;
        stages -= 1;
        if stages > 0 {
            butterfly_first(tr, x, points);
        }
        let mut i = 1;
        loop {
            stages -= 1;
            if stages <= 0 {
                break;
            }
            for j in 0..1usize << i {
                butterfly_generic(tr, &mut x[(points >> i) * j..], points >> i, 4 << i);
            }
            i += 1;
        }
        for j in (0..points).step_by(32) {
            butterfly_32(&mut x[j..j + 32]);
        }
    }

    /// `mdct_bitreverse`: reads `x[n/2..]`, writes `x[..n/2]`.
    fn bitreverse(&self, x: &mut [f32]) {
        let n2 = self.n >> 1;
        let tr = &self.trig;
        let bit = &self.bitrev;
        let (mut w0, mut w1, mut t, mut b) = (0, n2, self.n, 0);
        loop {
            let (x0, x1) = (n2 + bit[b], n2 + bit[b + 1]);
            let r0 = x[x0 + 1] - x[x1 + 1];
            let r1 = x[x0] + x[x1];
            let r2 = r1 * tr[t] + r0 * tr[t + 1];
            let r3 = r1 * tr[t + 1] - r0 * tr[t];
            w1 -= 4;
            let r0 = (x[x0 + 1] + x[x1 + 1]) * 0.5;
            let r1 = (x[x0] - x[x1]) * 0.5;
            x[w0] = r0 + r2;
            x[w1 + 2] = r0 - r2;
            x[w0 + 1] = r1 + r3;
            x[w1 + 3] = r3 - r1;
            let (x0, x1) = (n2 + bit[b + 2], n2 + bit[b + 3]);
            let r0 = x[x0 + 1] - x[x1 + 1];
            let r1 = x[x0] + x[x1];
            let r2 = r1 * tr[t + 2] + r0 * tr[t + 3];
            let r3 = r1 * tr[t + 3] - r0 * tr[t + 2];
            let r0 = (x[x0 + 1] + x[x1 + 1]) * 0.5;
            let r1 = (x[x0] - x[x1]) * 0.5;
            x[w0 + 2] = r0 + r2;
            x[w1] = r0 - r2;
            x[w0 + 3] = r1 + r3;
            x[w1 + 1] = r3 - r1;
            t += 4;
            b += 4;
            w0 += 4;
            if w0 >= w1 {
                break;
            }
        }
    }
}

/// One butterfly pair of a radix stage: `x1 += x2`, `x2 = (x1 - x2)` rotated by (t0, t1).
#[inline(always)]
fn rotate(x: &mut [f32], a: usize, b: usize, t0: f32, t1: f32) {
    let r0 = x[a] - x[b];
    let r1 = x[a + 1] - x[b + 1];
    x[a] += x[b];
    x[a + 1] += x[b + 1];
    x[b] = r1 * t1 + r0 * t0;
    x[b + 1] = r1 * t0 - r0 * t1;
}

/// `mdct_butterfly_first`.
fn butterfly_first(tr: &[f32], x: &mut [f32], points: usize) {
    let (mut x1, mut x2, mut t) = (points - 8, (points >> 1) - 8, 0);
    loop {
        rotate(x, x1 + 6, x2 + 6, tr[t], tr[t + 1]);
        rotate(x, x1 + 4, x2 + 4, tr[t + 4], tr[t + 5]);
        rotate(x, x1 + 2, x2 + 2, tr[t + 8], tr[t + 9]);
        rotate(x, x1, x2, tr[t + 12], tr[t + 13]);
        if x2 < 8 {
            break;
        }
        x1 -= 8;
        x2 -= 8;
        t += 16;
    }
}

/// `mdct_butterfly_generic`.
fn butterfly_generic(tr: &[f32], x: &mut [f32], points: usize, trigint: usize) {
    let (mut x1, mut x2, mut t) = (points - 8, (points >> 1) - 8, 0);
    loop {
        for k in [6, 4, 2, 0] {
            rotate(x, x1 + k, x2 + k, tr[t], tr[t + 1]);
            t += trigint;
        }
        if x2 < 8 {
            break;
        }
        x1 -= 8;
        x2 -= 8;
    }
}

/// `mdct_butterfly_8`.
#[inline(always)]
fn butterfly_8(x: &mut [f32]) {
    let r0 = x[6] + x[2];
    let r1 = x[6] - x[2];
    let r2 = x[4] + x[0];
    let r3 = x[4] - x[0];
    x[6] = r0 + r2;
    x[4] = r0 - r2;
    let r0 = x[5] - x[1];
    let r2 = x[7] - x[3];
    x[0] = r1 + r0;
    x[2] = r1 - r0;
    let r0 = x[5] + x[1];
    let r1 = x[7] + x[3];
    x[3] = r2 + r3;
    x[1] = r2 - r3;
    x[7] = r1 + r0;
    x[5] = r1 - r0;
}

/// `mdct_butterfly_16`.
#[inline(always)]
fn butterfly_16(x: &mut [f32]) {
    let r0 = x[1] - x[9];
    let r1 = x[0] - x[8];
    x[8] += x[0];
    x[9] += x[1];
    x[0] = (r0 + r1) * PI2_8;
    x[1] = (r0 - r1) * PI2_8;
    let r0 = x[3] - x[11];
    let r1 = x[10] - x[2];
    x[10] += x[2];
    x[11] += x[3];
    x[2] = r0;
    x[3] = r1;
    let r0 = x[12] - x[4];
    let r1 = x[13] - x[5];
    x[12] += x[4];
    x[13] += x[5];
    x[4] = (r0 - r1) * PI2_8;
    x[5] = (r0 + r1) * PI2_8;
    let r0 = x[14] - x[6];
    let r1 = x[15] - x[7];
    x[14] += x[6];
    x[15] += x[7];
    x[6] = r0;
    x[7] = r1;
    butterfly_8(x);
    butterfly_8(&mut x[8..]);
}

/// `mdct_butterfly_32`.
fn butterfly_32(x: &mut [f32]) {
    let r0 = x[30] - x[14];
    let r1 = x[31] - x[15];
    x[30] += x[14];
    x[31] += x[15];
    x[14] = r0;
    x[15] = r1;
    let r0 = x[28] - x[12];
    let r1 = x[29] - x[13];
    x[28] += x[12];
    x[29] += x[13];
    x[12] = r0 * PI1_8 - r1 * PI3_8;
    x[13] = r0 * PI3_8 + r1 * PI1_8;
    let r0 = x[26] - x[10];
    let r1 = x[27] - x[11];
    x[26] += x[10];
    x[27] += x[11];
    x[10] = (r0 - r1) * PI2_8;
    x[11] = (r0 + r1) * PI2_8;
    let r0 = x[24] - x[8];
    let r1 = x[25] - x[9];
    x[24] += x[8];
    x[25] += x[9];
    x[8] = r0 * PI3_8 - r1 * PI1_8;
    x[9] = r1 * PI3_8 + r0 * PI1_8;
    let r0 = x[22] - x[6];
    let r1 = x[7] - x[23];
    x[22] += x[6];
    x[23] += x[7];
    x[6] = r1;
    x[7] = r0;
    let r0 = x[4] - x[20];
    let r1 = x[5] - x[21];
    x[20] += x[4];
    x[21] += x[5];
    x[4] = r1 * PI1_8 + r0 * PI3_8;
    x[5] = r1 * PI3_8 - r0 * PI1_8;
    let r0 = x[2] - x[18];
    let r1 = x[3] - x[19];
    x[18] += x[2];
    x[19] += x[3];
    x[2] = (r1 + r0) * PI2_8;
    x[3] = (r1 - r0) * PI2_8;
    let r0 = x[0] - x[16];
    let r1 = x[1] - x[17];
    x[16] += x[0];
    x[17] += x[1];
    x[0] = r1 * PI3_8 + r0 * PI1_8;
    x[1] = r1 * PI1_8 - r0 * PI3_8;
    butterfly_16(x);
    butterfly_16(&mut x[16..]);
}

#[cfg(all(test, oracle))]
mod vs_c {
    use super::*;
    use crate::testutil::{Rng, bits, signal};
    use aotuv_sys::Mdct as CMdct;

    /// Every size the encoder can ask for (block sizes 64…8192, the envelope's 128): `trig`
    /// and `bitrev` equal the C's (its libm `cos`/`sin` vs Rust's: the same double, rounded).
    #[test]
    fn mdct_tables_match_c() {
        for k in 6..=13 {
            let n = 1 << k;
            let (r, c) = (Mdct::new(n), CMdct::new(n));
            let (ct, cb) = c.tables();
            assert_eq!(bits(&r.trig), bits(ct), "trig n {n}");
            let cb: Vec<usize> = cb.iter().map(|&b| b as usize).collect();
            assert_eq!(r.bitrev, cb, "bitrev n {n}");
        }
    }

    /// 200k transforms over every size, every signal class at every size (the class drawn
    /// from the generator, not from the case number the size comes from); outputs bit for bit.
    #[test]
    fn mdct_forward_matches_c() {
        let mut r = Rng::new(11);
        let sizes: Vec<usize> = (6..=13).map(|k| 1 << k).collect();
        let (rs, cs): (Vec<_>, Vec<_>) =
            sizes.iter().map(|&n| (Mdct::new(n), CMdct::new(n))).unzip();
        for case in 0..200_000u64 {
            // half the cases at 64, a quarter at 128, …: every size gets the same work
            let k = (case % 128).trailing_zeros().min(7) as usize;
            let n = sizes[k];
            let class = r.u64();
            let x = signal(&mut r, class, n);
            let (mut y, mut yc) = (vec![f32::NAN; n / 2], vec![f32::NAN; n / 2]);
            rs[k].forward(&x, &mut y);
            cs[k].forward(&x, &mut yc);
            assert_eq!(bits(&y), bits(&yc), "n {n} case {case}");
        }
    }
}
