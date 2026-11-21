//! Test helpers: a deterministic random generator for reproducible inputs, signal classes, bits.

/// Deterministic xorshift64* generator: reproducible inputs without extra dependencies.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in [-1, 1).
    pub fn unit(&mut self) -> f32 {
        (self.u64() >> 40) as f32 / (1u64 << 23) as f32 - 1.0
    }
}

/// `n` samples of signal class `k`: noise at full scale and at 16-bit PCM scale, zeros, tiny
/// values, a sine, impulses, a clipped square, magnitudes down to subnormal results; so sums
/// see cancellation and range.
pub fn signal(r: &mut Rng, k: u64, n: usize) -> Vec<f32> {
    let f = 0.01 + (r.u64() % 1000) as f32 / 2000.0;
    let a = [1.0, 1e-3, 32768.0, 1e-30, 1e-37][(r.u64() % 5) as usize];
    (0..n)
        .map(|i| match k % 8 {
            0 => r.unit(),
            1 => (r.unit() * 32768.0).round(),
            2 => 0.0,
            3 => r.unit() * 1e-30,
            4 => (f * i as f32).sin(),
            5 => {
                if r.u64().is_multiple_of(17) {
                    r.unit() * 32768.0
                } else {
                    0.0
                }
            }
            6 => {
                if (f * i as f32).sin() >= 0.0 {
                    1.0
                } else {
                    -1.0
                }
            }
            _ => r.unit() * a,
        })
        .collect()
}

/// The bit patterns, so NaN and -0.0 compare exactly.
pub fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|x| x.to_bits()).collect()
}
