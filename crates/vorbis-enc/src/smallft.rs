//! The real forward FFT (`smallft.c`, FFTPACK's `rfftf`), as mapping0 runs it on each block
//! for the psychoacoustic model. Vorbis block sizes are powers of two, so only the radix-4 and
//! radix-2 passes (`dradf4`, `dradf2`) are ported; the general radix (`dradfg`, factors 3 and 5)
//! is never reached. In the C's operation order.

/// `drft_lookup`: `trigcache` (the first `n` floats are the transform's scratch, then the
/// twiddles `wa`) and `splitcache` (`ifac`: n, factor count, factors).
#[derive(Clone, Debug)]
pub(crate) struct Drft {
    n: usize,
    pub(crate) trigcache: Vec<f32>,
    splitcache: [i32; 32],
}

impl Drft {
    /// `drft_init`: `n` a power of two ≥ 2.
    pub(crate) fn new(n: usize) -> Self {
        assert!(n.is_power_of_two() && n >= 2);
        let mut d = Drft {
            n,
            trigcache: vec![0.0; 3 * n],
            splitcache: [0; 32],
        };
        drfti1(n, &mut d.trigcache[n..], &mut d.splitcache);
        d
    }

    /// `drft_forward`: `data[..n]` in place (FFTPACK's half-complex order).
    pub(crate) fn forward(&mut self, data: &mut [f32]) {
        let n = self.n;
        let (ch, wa) = self.trigcache.split_at_mut(n);
        drftf1(n, &mut data[..n], ch, wa, &self.splitcache);
    }
}

/// `drfti1`: factor `n` (4s first, then 2s, a 2 moved to the front) and the twiddles.
fn drfti1(n: usize, wa: &mut [f32], ifac: &mut [i32; 32]) {
    const NTRYH: [usize; 4] = [4, 2, 3, 5];
    #[allow(clippy::excessive_precision, clippy::approx_constant)] // the C's `static float tpi`
    const TPI: f32 = 6.283_185_307_179_586_48;
    let mut ntry = 0;
    let mut j = 0;
    let mut nl = n;
    let mut nf = 0;
    'next: loop {
        // L101
        ntry = if j < 4 { NTRYH[j] } else { ntry + 2 };
        j += 1;
        loop {
            // L104
            let nq = nl / ntry;
            if nl - ntry * nq != 0 {
                continue 'next;
            }
            nf += 1;
            ifac[nf + 1] = ntry as i32;
            nl = nq;
            if ntry == 2 && nf != 1 {
                for i in 1..nf {
                    let ib = nf - i + 1;
                    ifac[ib + 1] = ifac[ib];
                }
                ifac[2] = 2;
            }
            if nl == 1 {
                break 'next;
            }
        }
    }
    ifac[0] = n as i32;
    ifac[1] = nf as i32;
    let argh = TPI / n as f32;
    let mut is = 0;
    let mut l1 = 1;
    for k1 in 0..nf - 1 {
        let ip = ifac[k1 + 2] as usize;
        let mut ld = 0;
        let l2 = l1 * ip;
        let ido = n / l2;
        for _ in 0..ip - 1 {
            ld += l1;
            let mut i = is;
            let argld = ld as f32 * argh;
            let mut fi = 0f32;
            let mut ii = 2;
            while ii < ido {
                fi += 1.0;
                let arg = fi * argld;
                // the C's double cos/sin of the float argument, rounded to float on store
                wa[i] = (arg as f64).cos() as f32;
                wa[i + 1] = (arg as f64).sin() as f32;
                i += 2;
                ii += 2;
            }
            is += ido;
        }
        l1 = l2;
    }
}

/// `dradf2`: one radix-2 pass, `cc` → `ch`; `wa1` from offset `w`.
fn dradf2(ido: usize, l1: usize, cc: &[f32], ch: &mut [f32], wa: &[f32], w: usize) {
    let t0 = l1 * ido;
    let (mut t1, mut t2) = (0, t0);
    let t3 = ido << 1;
    for _ in 0..l1 {
        ch[t1 << 1] = cc[t1] + cc[t2];
        ch[(t1 << 1) + t3 - 1] = cc[t1] - cc[t2];
        t1 += ido;
        t2 += ido;
    }
    if ido < 2 {
        return;
    }
    if ido != 2 {
        t1 = 0;
        t2 = t0;
        for _ in 0..l1 {
            let mut t3 = t2;
            let mut t4 = (t1 << 1) + (ido << 1);
            let mut t5 = t1;
            let mut t6 = t1 + t1;
            let mut i = 2;
            while i < ido {
                t3 += 2;
                t4 -= 2;
                t5 += 2;
                t6 += 2;
                let (wr, wi) = (wa[w + i - 2], wa[w + i - 1]);
                let tr2 = wr * cc[t3 - 1] + wi * cc[t3];
                let ti2 = wr * cc[t3] - wi * cc[t3 - 1];
                ch[t6] = cc[t5] + ti2;
                ch[t4] = ti2 - cc[t5];
                ch[t6 - 1] = cc[t5 - 1] + tr2;
                ch[t4 - 1] = cc[t5 - 1] - tr2;
                i += 2;
            }
            t1 += ido;
            t2 += ido;
        }
        if ido % 2 == 1 {
            return;
        }
    }
    // L105
    t1 = ido;
    let mut t3 = ido - 1;
    t2 = t3 + t0;
    for _ in 0..l1 {
        ch[t1] = -cc[t2];
        ch[t1 - 1] = cc[t3];
        t1 += ido << 1;
        t2 += ido;
        t3 += ido;
    }
}

/// `dradf4`: one radix-4 pass, `cc` → `ch`; `wa1`, `wa2`, `wa3` from offsets `w1`, `w2`, `w3`.
#[allow(clippy::too_many_arguments)]
fn dradf4(
    ido: usize,
    l1: usize,
    cc: &[f32],
    ch: &mut [f32],
    wa: &[f32],
    w1: usize,
    w2: usize,
    w3: usize,
) {
    #[allow(clippy::excessive_precision, clippy::approx_constant)] // the C's `static float hsqt2`
    const HSQT2: f32 = 0.707_106_781_186_547_52;
    let t0 = l1 * ido;
    let mut t1 = t0;
    let mut t4 = t1 << 1;
    let mut t2 = t1 + (t1 << 1);
    let mut t3 = 0;
    for _ in 0..l1 {
        let tr1 = cc[t1] + cc[t2];
        let tr2 = cc[t3] + cc[t4];
        let mut t5 = t3 << 2;
        ch[t5] = tr1 + tr2;
        ch[(ido << 2) + t5 - 1] = tr2 - tr1;
        t5 += ido << 1;
        ch[t5 - 1] = cc[t3] - cc[t4];
        ch[t5] = cc[t2] - cc[t1];
        t1 += ido;
        t2 += ido;
        t3 += ido;
        t4 += ido;
    }
    if ido < 2 {
        return;
    }
    if ido != 2 {
        // Per k: the four input rows and the four output quarters as slices of length `ido`
        // (the C's t2..t6 walks: q0/q2 ascend from 1, q1/q3 descend to 1), so no index checks.
        let (wa1, wa2, wa3) = (&wa[w1..w1 + ido], &wa[w2..w2 + ido], &wa[w3..w3 + ido]);
        for k in 0..l1 {
            let t1 = k * ido;
            let r0 = &cc[t1..t1 + ido];
            let r1 = &cc[t1 + t0..t1 + t0 + ido];
            let r2 = &cc[t1 + 2 * t0..t1 + 2 * t0 + ido];
            let r3 = &cc[t1 + 3 * t0..t1 + 3 * t0 + ido];
            let (q01, q23) = ch[4 * t1..4 * t1 + 4 * ido].split_at_mut(2 * ido);
            let (q0, q1) = q01.split_at_mut(ido);
            let (q2, q3) = q23.split_at_mut(ido);
            let mut i = 2;
            while i < ido {
                let cr2 = wa1[i - 2] * r1[i - 1] + wa1[i - 1] * r1[i];
                let ci2 = wa1[i - 2] * r1[i] - wa1[i - 1] * r1[i - 1];
                let cr3 = wa2[i - 2] * r2[i - 1] + wa2[i - 1] * r2[i];
                let ci3 = wa2[i - 2] * r2[i] - wa2[i - 1] * r2[i - 1];
                let cr4 = wa3[i - 2] * r3[i - 1] + wa3[i - 1] * r3[i];
                let ci4 = wa3[i - 2] * r3[i] - wa3[i - 1] * r3[i - 1];
                let tr1 = cr2 + cr4;
                let tr4 = cr4 - cr2;
                let ti1 = ci2 + ci4;
                let ti4 = ci2 - ci4;
                let ti2 = r0[i] + ci3;
                let ti3 = r0[i] - ci3;
                let tr2 = r0[i - 1] + cr3;
                let tr3 = r0[i - 1] - cr3;
                let j = ido - i;
                q0[i - 1] = tr1 + tr2;
                q0[i] = ti1 + ti2;
                q1[j - 1] = tr3 - ti4;
                q1[j] = tr4 - ti3;
                q2[i - 1] = ti4 + tr3;
                q2[i] = tr4 + ti3;
                q3[j - 1] = tr2 - tr1;
                q3[j] = ti1 - ti2;
                i += 2;
            }
        }
        if ido & 1 == 1 {
            return;
        }
    }
    // L105
    t1 = t0 + ido - 1;
    t2 = t1 + (t0 << 1);
    let t3 = ido << 2;
    let mut t4 = ido;
    let t5 = ido << 1;
    let mut t6 = ido;
    for _ in 0..l1 {
        let ti1 = -HSQT2 * (cc[t1] + cc[t2]);
        let tr1 = HSQT2 * (cc[t1] - cc[t2]);
        ch[t4 - 1] = tr1 + cc[t6 - 1];
        ch[t4 + t5 - 1] = cc[t6 - 1] - tr1;
        ch[t4] = ti1 - cc[t1 + t0];
        ch[t4 + t5] = ti1 + cc[t1 + t0];
        t1 += ido;
        t2 += ido;
        t4 += t3;
        t6 += ido;
    }
}

/// `drftf1`: the passes, alternating between `c` and the scratch `ch`.
fn drftf1(n: usize, c: &mut [f32], ch: &mut [f32], wa: &[f32], ifac: &[i32; 32]) {
    let nf = ifac[1] as usize;
    let mut na = true;
    let mut l2 = n;
    let mut iw = n;
    for k1 in 0..nf {
        let kh = nf - k1;
        let ip = ifac[kh + 1] as usize;
        let l1 = l2 / ip;
        let ido = n / l2;
        iw -= (ip - 1) * ido;
        na = !na;
        // `wa+iw-1` etc.: offsets into `wa`, the C's 1-based twiddle pointers
        let w = iw - 1;
        match (ip, na) {
            (4, true) => dradf4(ido, l1, ch, c, wa, w, w + ido, w + 2 * ido),
            (4, false) => dradf4(ido, l1, c, ch, wa, w, w + ido, w + 2 * ido),
            (2, true) => dradf2(ido, l1, ch, c, wa, w),
            (2, false) => dradf2(ido, l1, c, ch, wa, w),
            _ => unreachable!("radix {ip}: n is a power of two"),
        }
        l2 = l1;
    }
    if !na {
        c.copy_from_slice(ch);
    }
}

#[cfg(all(test, oracle))]
mod vs_c {
    use super::*;
    use crate::testutil::{Rng, bits, signal};
    use aotuv_sys::Drft as CDrft;

    /// Every power of two 2…8192 (the block sizes are 64…8192): `trigcache`, `splitcache`.
    #[test]
    fn drft_tables_match_c() {
        for k in 1..=13 {
            let n = 1 << k;
            let (r, c) = (Drft::new(n), CDrft::new(n));
            let (ct, cs) = c.tables();
            assert_eq!(bits(&r.trigcache), bits(ct), "trigcache n {n}");
            assert_eq!(&r.splitcache[..], cs, "splitcache n {n}");
        }
    }

    /// 200k transforms over every size, every signal class at every size (the class drawn
    /// from the generator, not from the case number the size comes from); outputs bit for bit.
    #[test]
    fn drft_forward_matches_c() {
        let mut r = Rng::new(12);
        let sizes: Vec<usize> = (1..=13).map(|k| 1 << k).collect();
        let mut rs: Vec<Drft> = sizes.iter().map(|&n| Drft::new(n)).collect();
        let mut cs: Vec<CDrft> = sizes.iter().map(|&n| CDrft::new(n)).collect();
        for case in 0..200_000u64 {
            // half the cases at 2, a quarter at 4, …: every size gets about the same work
            let k = (case % 8192).trailing_zeros().min(12) as usize;
            let n = sizes[k];
            let class = r.u64();
            let mut x = signal(&mut r, class, n);
            let mut xc = x.clone();
            rs[k].forward(&mut x);
            cs[k].forward(&mut xc);
            assert_eq!(bits(&x), bits(&xc), "n {n} case {case}");
        }
    }
}
