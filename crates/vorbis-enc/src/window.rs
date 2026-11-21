//! The block windows (`window.c`): Vorbis's power-sine slopes, tables generated from the C
//! (`tables.rs`), applied over a block's overlaps with its neighbours.

use crate::tables::VWIN;

/// `_vorbis_window_get`: the rising half of window `n` (`ilog2(blocksize) - 6`).
#[cfg(test)]
pub(crate) fn window_get(n: usize) -> &'static [f32] {
    VWIN[n]
}

/// `_vorbis_apply_window`: block `W` (0 short, 1 long) between its previous `lw` and next
/// `nw`; `winno` the window per block size, `blocksizes` the two sizes. Zeroes outside the
/// slopes.
pub(crate) fn apply_window(
    d: &mut [f32],
    winno: &[usize; 2],
    blocksizes: &[usize; 2],
    lw: usize,
    w: usize,
    nw: usize,
) {
    let (lw, nw) = if w != 0 { (lw, nw) } else { (0, 0) };
    let window_lw = VWIN[winno[lw]];
    let window_nw = VWIN[winno[nw]];
    let n = blocksizes[w];
    let ln = blocksizes[lw];
    let rn = blocksizes[nw];
    let leftbegin = n / 4 - ln / 4;
    let leftend = leftbegin + ln / 2;
    let rightbegin = n / 2 + n / 4 - rn / 4;
    let rightend = rightbegin + rn / 2;
    d[..leftbegin].fill(0.0);
    for (x, &c) in d[leftbegin..leftend].iter_mut().zip(window_lw) {
        *x *= c;
    }
    for (x, &c) in d[rightbegin..rightend]
        .iter_mut()
        .zip(window_nw[..rn / 2].iter().rev())
    {
        *x *= c;
    }
    d[rightend..n].fill(0.0);
}

#[cfg(all(test, oracle))]
#[allow(unsafe_code)]
mod vs_c {
    use super::*;
    use crate::testutil::{Rng, bits, signal};
    use aotuv_sys::{_vorbis_apply_window, _vorbis_window_get};
    use std::ffi::{c_int, c_long};

    #[test]
    fn windows_match_c() {
        for k in 0..8 {
            // SAFETY: vwin[k] holds 32 << k floats.
            let c = unsafe { std::slice::from_raw_parts(_vorbis_window_get(k as c_int), 32 << k) };
            assert_eq!(bits(window_get(k)), bits(c), "window {k}");
        }
    }

    /// 200k blocks: every block size pair the setup allows (short ≤ long, 64…8192), every
    /// (lW, W, nW), random contents; the whole block bit for bit.
    #[test]
    fn apply_window_matches_c() {
        let mut r = Rng::new(13);
        let mut pairs = Vec::new();
        for s in 0..8 {
            for l in s..8 {
                pairs.push((s, l));
            }
        }
        for case in 0..200_000u64 {
            let (s, l) = pairs[(r.u64() % pairs.len() as u64) as usize];
            let winno = [s, l];
            let blocksizes = [64usize << s, 64 << l];
            // (lW, W, nW), the class and the long-block thinning all from the generator: no
            // bit of `case` decides two of them, so every combination occurs at every size
            let flags = r.u64();
            let (lw, w, nw) = (
                (flags & 1) as usize,
                (flags >> 1 & 1) as usize,
                (flags >> 2 & 1) as usize,
            );
            let n = blocksizes[w];
            if n > 2048 && !(flags >> 3).is_multiple_of(16) {
                continue; // the long blocks: fewer cases, same coverage
            }
            let class = r.u64();
            let mut x = signal(&mut r, class, n);
            let mut xc = x.clone();
            apply_window(&mut x, &winno, &blocksizes, lw, w, nw);
            let mut cw = [winno[0] as c_int, winno[1] as c_int];
            let mut cb = [blocksizes[0] as c_long, blocksizes[1] as c_long];
            // SAFETY: d holds blocksizes[W] floats; winno/blocksizes two entries each.
            unsafe {
                _vorbis_apply_window(
                    xc.as_mut_ptr(),
                    cw.as_mut_ptr(),
                    cb.as_mut_ptr(),
                    lw as c_int,
                    w as c_int,
                    nw as c_int,
                )
            };
            assert_eq!(
                bits(&x),
                bits(&xc),
                "{blocksizes:?} {lw}{w}{nw} case {case}"
            );
        }
    }
}
