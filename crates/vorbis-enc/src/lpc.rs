//! Linear prediction (`lpc.c`): the encoder extrapolates the signal with it before the first
//! and after the last sample (`block.c`), so the edge blocks see no step. In the C's order:
//! autocorrelation and Levinson-Durbin in double, the prediction filter in float.

/// `vorbis_lpc_from_data`: `m` damped LPC coefficients of `data[..n]` into `lpci[..m]`;
/// returns the prediction error (double, rounded to float as the C returns it).
pub(crate) fn lpc_from_data(data: &[f32], lpci: &mut [f32], n: usize, m: usize) -> f32 {
    let mut aut = vec![0f64; m + 1];
    let mut lpc = vec![0f64; m];
    for j in (0..=m).rev() {
        let mut d = 0f64;
        for i in j..n {
            d += data[i] as f64 * data[i - j] as f64;
        }
        aut[j] = d;
    }
    let mut error = aut[0] * (1. + 1e-10);
    let epsilon = 1e-9 * aut[0] + 1e-10;
    for i in 0..m {
        let mut r = -aut[i + 1];
        if error < epsilon {
            lpc[i..].fill(0.0);
            break;
        }
        for j in 0..i {
            r -= lpc[j] * aut[i - j];
        }
        r /= error;
        lpc[i] = r;
        let mut j = 0;
        while j < i / 2 {
            let tmp = lpc[j];
            lpc[j] += r * lpc[i - 1 - j];
            lpc[i - 1 - j] += r * tmp;
            j += 1;
        }
        if i & 1 == 1 {
            lpc[j] += lpc[j] * r;
        }
        error *= 1. - r * r;
    }
    let g = 0.99;
    let mut damp = g;
    for v in lpc.iter_mut() {
        *v *= damp;
        damp *= g;
    }
    for (o, &v) in lpci[..m].iter_mut().zip(&lpc) {
        *o = v as f32;
    }
    error as f32
}

/// `vorbis_lpc_predict`: `data[..n]` predicted from `m` coefficients, primed with the `m`
/// samples before it (`None`: zeros).
pub(crate) fn lpc_predict(
    coeff: &[f32],
    prime: Option<&[f32]>,
    m: usize,
    data: &mut [f32],
    n: usize,
) {
    let mut work = vec![0f32; m + n];
    if let Some(p) = prime {
        work[..m].copy_from_slice(&p[..m]);
    }
    for (i, out) in data[..n].iter_mut().enumerate() {
        let hist = &work[i..i + m];
        // The end-of-stream extrapolation decays through subnormals (thousands of samples),
        // where every f32 operation is a microcode assist: emulate them exactly instead.
        let y = if hist.iter().all(|v| v.abs() < TINY) {
            predict_tiny(coeff, hist)
        } else {
            let mut y = 0f32;
            for (w, c) in hist.iter().zip(coeff[..m].iter().rev()) {
                y -= w * c;
            }
            y
        };
        work[i + m] = y;
        *out = y;
    }
}

/// Below this (2^-120) the history's products and sums come near the subnormal range.
const TINY: f32 = f32::from_bits((127 - 120) << 23);

/// `y -= w·c` over the history in f32, exactly, computed on values scaled by 2^149 in f64
/// (f32's subnormals become integers there): each product is exact in f64 (24-bit operands),
/// then rounded to the f32 grid by [`round_f32`]. A sum is exact below 2^24 (an integer
/// difference); one that rounds in f64 is ≥ 2^24, a normal f32 there, where rounding f64 then
/// f32 equals rounding once (53 ≥ 2·24 + 2: double rounding is innocuous for a sum).
fn predict_tiny(coeff: &[f32], hist: &[f32]) -> f32 {
    let mut y = 0f64;
    for (&w, &c) in hist.iter().zip(coeff[..hist.len()].iter().rev()) {
        let p = round_f32(w as f64 * SCALE * c as f64);
        y = round_f32(y - p);
    }
    (y * (1.0 / SCALE)) as f32
}

/// 2^149: f32's smallest subnormal becomes 1.
const SCALE: f64 = f64::from_bits((1023 + 149) << 52);

/// The f32 rounding (ties to even) of `x / 2^149`, times 2^149; `x` exact. Below 2^24 (f32's
/// subnormals and its lowest binade, grid spacing 1 here) an integer rounding; above it a
/// conversion of a normal value (no assist).
#[inline]
fn round_f32(x: f64) -> f64 {
    if x.abs() < 16_777_216.0 {
        crate::scales::rint_f64(x)
    } else {
        ((x * (1.0 / SCALE)) as f32) as f64 * SCALE
    }
}

#[cfg(all(test, oracle))]
#[allow(unsafe_code)]
mod vs_c {
    use super::*;
    use crate::testutil::{Rng, bits, signal};
    use aotuv_sys::{vorbis_lpc_from_data, vorbis_lpc_predict};
    use std::ffi::{c_int, c_long};

    /// 100k cases: block.c's own shapes (1 in 32: `write_end`, order 32 over up to a long
    /// block, 3 long blocks predicted; `_preextrapolate_helper`, order 16, half a long block
    /// predicted; long blocks 256…8192) and beyond (orders 1…40, short data, silence: the
    /// `error < epsilon` exit); coefficients, error and the predicted run bit for bit, with and
    /// without priming. Every choice is drawn from the generator, none from `case`, so they
    /// combine freely.
    #[test]
    fn lpc_matches_c() {
        let mut r = Rng::new(14);
        for case in 0..100_000u64 {
            let (m, n, np) = if r.u64().is_multiple_of(32) {
                let bs1 = 256usize << (r.u64() % 6);
                if r.u64().is_multiple_of(2) {
                    (32, 65 + (r.u64() % bs1 as u64) as usize, 3 * bs1)
                } else {
                    (16, 33 + (r.u64() % (bs1 as u64 + 1024)) as usize, bs1 / 2)
                }
            } else {
                let m = match r.u64() % 4 {
                    0 => 16,
                    1 => 32,
                    _ => 1 + (r.u64() % 40) as usize,
                };
                let n = match r.u64() % 5 {
                    0 => m + (r.u64() % 8) as usize,
                    1 | 2 => 64 + (r.u64() % 512) as usize,
                    _ => 1024 + (r.u64() % 3072) as usize,
                };
                if n > 1024 && !r.u64().is_multiple_of(8) {
                    continue; // the long runs: fewer cases
                }
                (m, n, 1 + (r.u64() % 1024) as usize)
            };
            let class = r.u64();
            let mut x = signal(&mut r, class, n);
            if r.u64().is_multiple_of(13) {
                x.iter_mut().for_each(|v| *v = 0.0);
            }
            let (mut a, mut ac) = (vec![f32::NAN; m], vec![f32::NAN; m]);
            let e = lpc_from_data(&x, &mut a, n, m);
            // SAFETY: data n floats, lpci m floats.
            let ec = unsafe {
                vorbis_lpc_from_data(x.as_mut_ptr(), ac.as_mut_ptr(), n as c_int, m as c_int)
            };
            assert_eq!(bits(&a), bits(&ac), "coeffs m {m} n {n} case {case}");
            assert_eq!(e.to_bits(), ec.to_bits(), "error m {m} n {n} case {case}");

            let primed = !r.u64().is_multiple_of(3);
            let mut prime = x[n - m..].to_vec();
            let (mut y, mut yc) = (vec![f32::NAN; np], vec![f32::NAN; np]);
            lpc_predict(&a, primed.then_some(&prime[..]), m, &mut y, np);
            let pp = if primed {
                prime.as_mut_ptr()
            } else {
                std::ptr::null_mut()
            };
            // SAFETY: coeff m floats, prime m floats or null, data np floats.
            unsafe {
                vorbis_lpc_predict(
                    ac.as_mut_ptr(),
                    pp,
                    m as c_int,
                    yc.as_mut_ptr(),
                    np as c_long,
                )
            };
            assert_eq!(bits(&y), bits(&yc), "predict m {m} np {np} case {case}");
        }
    }
}

#[cfg(test)]
mod tiny {
    use super::*;
    use crate::testutil::Rng;

    /// 2^e for e in -149..=127, subnormals included (`powi` underflows below -126).
    fn pow2(e: i32) -> f32 {
        if e >= -126 {
            f32::from_bits(((e + 127) as u32) << 23)
        } else {
            f32::from_bits(1 << (e + 149))
        }
    }

    /// `predict_tiny` against the hardware's own f32 (IEEE subnormals, no flush) on histories
    /// below `TINY`: subnormals, zeros of both signs, the lowest normal binades; coefficients
    /// over f32's whole exponent range (subnormal to 2^127: sums and products crossing into
    /// normal values), raw bit patterns, ±inf and NaN (a NaN result compares as NaN, its
    /// payload is the hardware's).
    #[test]
    fn predict_tiny_is_f32() {
        let mut r = Rng::new(26);
        let mut buf = vec![0f32; 64];
        for case in 0..400_000u32 {
            let m = [16, 32, 1, 7][case as usize % 4];
            let mut coeff = [0f32; 32];
            for c in &mut coeff[..m] {
                *c = match r.u64() % 64 {
                    0 => f32::INFINITY,
                    1 => f32::NEG_INFINITY,
                    2 => f32::NAN,
                    3..=10 => f32::from_bits(r.u64() as u32),
                    _ => r.unit() * pow2((r.u64() % 277) as i32 - 149),
                };
            }
            for v in &mut buf[..m] {
                *v = match r.u64() % 8 {
                    0 => 0.0,
                    1 => -0.0,
                    2 => f32::from_bits((r.u64() % 4) as u32 | ((r.u64() & 1) as u32) << 31),
                    3 => f32::from_bits((r.u64() as u32) & 0x807f_ffff),
                    _ => r.unit() * 2f32.powi(-120 - (r.u64() % 30) as i32),
                };
            }
            let hist = &buf[..m];
            let mut want = 0f32;
            for (w, c) in hist.iter().zip(coeff[..m].iter().rev()) {
                want -= w * c;
            }
            let got = predict_tiny(&coeff, hist);
            assert!(
                got.to_bits() == want.to_bits() || got.is_nan() && want.is_nan(),
                "case {case}: {got:?} vs {want:?}: {hist:?} {coeff:?}"
            );
        }
    }
}
