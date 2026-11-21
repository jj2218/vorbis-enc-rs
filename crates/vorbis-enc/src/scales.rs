//! The C's scalar helpers (`scales.h`, the `os.h` macros), each with the C's types and
//! operation order: dB and octave/bark scales, `rint`, `min`/`max`.

/// `todB` (`scales.h`, `VORBIS_IEEE_FLOAT32`): the float's bits, scaled.
#[inline]
#[allow(clippy::excessive_precision)] // the C's literals
pub(crate) fn todb(x: f32) -> f32 {
    let i = x.to_bits() & 0x7fff_ffff;
    i as f32 * 7.177_114_38e-7_f32 - 764.616_188_6_f32
}

/// `toOC` (`scales.h`): `log(n)*1.442695f-5.965784f`, in double.
#[inline]
#[allow(clippy::approx_constant)] // the C's literals
pub(crate) fn to_oc(n: f64) -> f64 {
    n.ln() * 1.442_695_f32 as f64 - 5.965_784_f32 as f64
}

/// `fromOC`: `exp(((o)+5.965784f)*.693147f)`, in double.
#[inline]
#[allow(clippy::approx_constant)] // the C's literals
pub(crate) fn from_oc(o: f64) -> f64 {
    ((o + 5.965_784_f32 as f64) * 0.693_147_f32 as f64).exp()
}

/// `toBARK(n)` for an integer `n` (`long`): the C's mix of float and double terms.
#[inline]
pub(crate) fn to_bark(n: i64) -> f64 {
    let a = 13.1_f32 as f64 * ((0.000_74_f32 * n as f32) as f64).atan();
    let b = 2.24_f32 as f64 * (((n * n) as f32 * 1.85e-8_f32) as f64).atan();
    a + b + (1e-4_f32 * n as f32) as f64
}

/// `unitnorm` (`scales.h`, IEEE): ±1 with `x`'s sign bit.
#[inline]
pub(crate) fn unitnorm(x: f32) -> f32 {
    f32::from_bits((x.to_bits() & 0x8000_0000) | 0x3f80_0000)
}

/// `rint` in the default rounding mode (ties to even). Where it is one instruction (aarch64
/// `frintx`, SSE4.1 `roundsd`) `round_ties_even`; else (baseline x86-64: a libm call) without
/// libm: for |x| < 2^52 the addition |x| + 2^52 rounds to an integer itself; larger values
/// are integers already; the sign (and -0) restored; NaN passes through.
#[inline]
pub(crate) fn rint_f64(x: f64) -> f64 {
    if cfg!(any(target_arch = "aarch64", target_feature = "sse4.1")) {
        return x.round_ties_even();
    }
    const M: f64 = 4_503_599_627_370_496.0;
    let a = x.abs();
    if a < M { ((a + M) - M).copysign(x) } else { x }
}

/// C's `rint` of a float to `int` (the default rounding: ties to even).
#[inline]
pub(crate) fn rint(x: f32) -> i32 {
    rint_f64(x as f64) as i32
}

/// The C's `max()` macro (`os.h`: `x < y ? y : x`; ties and NaN as the C).
#[inline]
pub(crate) fn fmax(x: f32, y: f32) -> f32 {
    if x < y { y } else { x }
}

/// The C's `min()` macro (`os.h`: `x > y ? y : x`).
#[inline]
pub(crate) fn fmin(x: f32, y: f32) -> f32 {
    if x > y { y } else { x }
}

#[cfg(test)]
mod tests {
    use super::rint_f64;

    /// Equal to `round_ties_even` (bits, so -0 counts) on ties, near-ties, ±0, the 2^52
    /// boundary, huge values, infinities and NaN, and 1M random doubles.
    #[test]
    fn rint_is_round_ties_even() {
        let mut r = crate::testutil::Rng::new(52);
        let mut xs = vec![
            0.0,
            -0.0,
            0.5,
            -0.5,
            1.5,
            2.5,
            -2.5,
            0.49999999999999994,
            4503599627370495.5,
            4503599627370496.0,
            -4503599627370497.0,
            1e300,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
        ];
        for _ in 0..1_000_000 {
            let e = (r.u64() % 64) as i32 - 10;
            xs.push(r.unit() as f64 * 2f64.powi(e) + (r.u64() % 3) as f64 * 0.5);
        }
        for x in xs {
            assert_eq!(
                rint_f64(x).to_bits(),
                x.round_ties_even().to_bits(),
                "{x:e}"
            );
        }
    }
}
