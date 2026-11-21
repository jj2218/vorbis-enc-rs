//! Floor 1, the encoder side (`floor1.c`): the look (posts sorted, neighbours), the fit of a
//! piecewise-linear floor to the masking curve (`floor1_fit`), the fits between two blobs
//! (`floor1_interpolate_fit`), and the floor packet (`floor1_encode`) with the integer floor
//! the residue is normalised by. Integer C arithmetic in `int` (wrapping where the C's sums
//! of squares may), the line fit in `double`.
#![allow(clippy::needless_range_loop)] // index loops mirror the C code 1:1

use crate::bitpack::{OggPack, ilog};
use crate::codebook::Codebook;
use crate::scales::rint_f64;
use crate::templates::{InfoFloor1, VIF_POSIT};

/// `vorbis_look_floor1` (the encoder's part; the C's bit statistics are never read).
#[derive(Clone, Debug)]
pub(crate) struct Floor1Look {
    pub vi: InfoFloor1,
    pub sorted_index: [i32; VIF_POSIT + 2],
    pub forward_index: [i32; VIF_POSIT + 2],
    pub reverse_index: [i32; VIF_POSIT + 2],
    pub hineighbor: [i32; VIF_POSIT],
    pub loneighbor: [i32; VIF_POSIT],
    pub posts: usize,
    pub n: i32,
    pub quant_q: i32,
}

/// `lsfit_acc`.
#[derive(Clone, Copy, Debug, Default)]
struct LsfitAcc {
    x0: i32,
    x1: i32,
    xa: i32,
    ya: i32,
    x2a: i32,
    xya: i32,
    an: i32,
    xb: i32,
    yb: i32,
    x2b: i32,
    xyb: i32,
    bn: i32,
}

impl Floor1Look {
    /// `floor1_look`.
    pub(crate) fn new(info: &InfoFloor1) -> Self {
        let mut n = 0usize;
        for i in 0..info.partitions as usize {
            n += info.class_dim[info.partitionclass[i] as usize] as usize;
        }
        n += 2;
        let mut look = Floor1Look {
            vi: info.clone(),
            sorted_index: [0; VIF_POSIT + 2],
            forward_index: [0; VIF_POSIT + 2],
            reverse_index: [0; VIF_POSIT + 2],
            hineighbor: [0; VIF_POSIT],
            loneighbor: [0; VIF_POSIT],
            posts: n,
            n: info.postlist[1],
            quant_q: 0,
        };
        // qsort by x: the posts' x are distinct, so any sort gives the C's order
        let mut sorted: Vec<usize> = (0..n).collect();
        sorted.sort_by_key(|&i| info.postlist[i]);
        for i in 0..n {
            look.forward_index[i] = sorted[i] as i32;
        }
        for i in 0..n {
            look.reverse_index[look.forward_index[i] as usize] = i as i32;
        }
        for i in 0..n {
            look.sorted_index[i] = info.postlist[look.forward_index[i] as usize];
        }
        look.quant_q = match info.mult {
            1 => 256,
            2 => 128,
            3 => 86,
            4 => 64,
            _ => 0,
        };
        for i in 0..n - 2 {
            let (mut lo, mut hi, mut lx, mut hx) = (0, 1, 0, look.n);
            let currentx = info.postlist[i + 2];
            for j in 0..i + 2 {
                let x = info.postlist[j];
                if x > lx && x < currentx {
                    lo = j;
                    lx = x;
                }
                if x < hx && x > currentx {
                    hi = j;
                    hx = x;
                }
            }
            look.loneighbor[i] = lo as i32;
            look.hineighbor[i] = hi as i32;
        }
        look
    }
}

/// `render_point`: the line's y at `x` (flag bits masked off).
fn render_point(x0: i32, x1: i32, y0: i32, y1: i32, x: i32) -> i32 {
    let y0 = y0 & 0x7fff;
    let y1 = y1 & 0x7fff;
    let dy = y1 - y0;
    let adx = x1 - x0;
    let ady = dy.abs();
    let err = ady * (x - x0);
    let off = err / adx;
    if dy < 0 { y0 - off } else { y0 + off }
}

/// `vorbis_dBquant`: dB to the 0..1023 floor scale (float math, truncated as the C's
/// conversion; inputs are dB values, far inside `int`).
fn vorbis_dbquant(x: f32) -> i32 {
    let i = (x * 7.314_285_7_f32 + 1023.5f32) as i32;
    i.clamp(0, 1023)
}

/// `render_line0`: the integer line from (x0, y0) to (x1, y1) into `d[x0..min(n, x1)]`.
fn render_line0(n: i32, x0: i32, x1: i32, y0: i32, y1: i32, d: &mut [i32]) {
    let dy = y1 - y0;
    let adx = x1 - x0;
    let mut ady = dy.abs();
    let base = dy / adx;
    let sy = if dy < 0 { base - 1 } else { base + 1 };
    let mut x = x0;
    let mut y = y0;
    let mut err = 0;
    let n = n.min(x1);
    ady -= (base * adx).abs();
    if x < n {
        d[x as usize] = y;
    }
    loop {
        x += 1;
        if x >= n {
            break;
        }
        err += ady;
        if err >= adx {
            err -= adx;
            y += sy;
        } else {
            y += base;
        }
        d[x as usize] = y;
    }
}

/// `accumulate_fit`: the least-squares sums over `[x0, x1]` of the quantised mask, split by
/// whether the MDCT reaches the mask (`a`) or not (`b`); returns the `a` count.
fn accumulate_fit(q: &[i32], reach: &[bool], x0: i32, x1: i32, a: &mut LsfitAcc, n: i32) -> i32 {
    // (the C also sums y² per side, never read)
    let (mut xa, mut ya, mut x2a, mut xya, mut na) = (0i32, 0i32, 0i32, 0i32, 0);
    let (mut xb, mut yb, mut x2b, mut xyb, mut nb) = (0i32, 0i32, 0i32, 0i32, 0);
    let end = if x1 >= n { n - 1 } else { x1 };
    for i in x0..=end {
        let iu = i as usize;
        let quantized = q[iu];
        if quantized != 0 {
            // the C's `long i` products, stored back into `int` sums
            if reach[iu] {
                xa = xa.wrapping_add(i);
                ya = ya.wrapping_add(quantized);
                x2a = x2a.wrapping_add(i.wrapping_mul(i));
                xya = xya.wrapping_add(i.wrapping_mul(quantized));
                na += 1;
            } else {
                xb = xb.wrapping_add(i);
                yb = yb.wrapping_add(quantized);
                x2b = x2b.wrapping_add(i.wrapping_mul(i));
                xyb = xyb.wrapping_add(i.wrapping_mul(quantized));
                nb += 1;
            }
        }
    }
    // x1 as given, before the clamp (as the C stores it)
    *a = LsfitAcc {
        x0,
        x1,
        xa,
        ya,
        x2a,
        xya,
        an: na,
        xb,
        yb,
        x2b,
        xyb,
        bn: nb,
    };
    na
}

/// `fit_line`: the weighted least-squares line over `a`, through the given ends when they are
/// set (≥ 0); its ends (rounded, clamped to 0..1023) into `y0`, `y1`; false when degenerate.
fn fit_line(a: &[LsfitAcc], y0: &mut i32, y1: &mut i32, info: &InfoFloor1) -> bool {
    // (the C also sums y², never read)
    let (mut xb, mut yb, mut x2b, mut xyb, mut bn) = (0f64, 0f64, 0f64, 0f64, 0f64);
    let x0 = a[0].x0;
    let x1 = a[a.len() - 1].x1;
    for f in a {
        // (int * float) / int in float, + 1. in double
        let weight = ((f.bn + f.an) as f32 * info.twofitweight / (f.an + 1) as f32) as f64 + 1.0;
        xb += f.xb as f64 + f.xa as f64 * weight;
        yb += f.yb as f64 + f.ya as f64 * weight;
        x2b += f.x2b as f64 + f.x2a as f64 * weight;
        xyb += f.xyb as f64 + f.xya as f64 * weight;
        bn += f.bn as f64 + f.an as f64 * weight;
    }
    if *y0 >= 0 {
        xb += x0 as f64;
        yb += *y0 as f64;
        x2b += (x0 * x0) as f64;
        xyb += (*y0 * x0) as f64;
        bn += 1.0;
    }
    if *y1 >= 0 {
        xb += x1 as f64;
        yb += *y1 as f64;
        x2b += (x1 * x1) as f64;
        xyb += (*y1 * x1) as f64;
        bn += 1.0;
    }
    let denom = bn * x2b - xb * xb;
    if denom > 0.0 {
        let a = (yb * x2b - xyb * xb) / denom;
        let b = (bn * xyb - xb * yb) / denom;
        *y0 = rint_f64(a + b * x0 as f64) as i32;
        *y1 = rint_f64(a + b * x1 as f64) as i32;
        *y0 = (*y0).clamp(0, 1023);
        *y1 = (*y1).clamp(0, 1023);
        true
    } else {
        *y0 = 0;
        *y1 = 0;
        false
    }
}

/// `inspect_error`: whether the line from (x0, y0) to (x1, y1) strays too far from the mask.
fn inspect_error(
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
    q: &[i32],
    reach: &[bool],
    info: &InfoFloor1,
) -> bool {
    let dy = y1 - y0;
    let adx = x1 - x0;
    let mut ady = dy.abs();
    let base = dy / adx;
    let sy = if dy < 0 { base - 1 } else { base + 1 };
    let mut x = x0;
    let mut y = y0;
    let mut err = 0;
    let mut val = q[x as usize];
    let mut n = 0;
    ady -= (base * adx).abs();
    let mut mse = y - val;
    mse *= mse;
    n += 1;
    if reach[x as usize] {
        if (y as f32 + info.maxover) < val as f32 {
            return true;
        }
        if (y as f32 - info.maxunder) > val as f32 {
            return true;
        }
    }
    loop {
        x += 1;
        if x >= x1 {
            break;
        }
        err += ady;
        if err >= adx {
            err -= adx;
            y += sy;
        } else {
            y += base;
        }
        val = q[x as usize];
        mse += (y - val) * (y - val);
        n += 1;
        if reach[x as usize] && val != 0 {
            if (y as f32 + info.maxover) < val as f32 {
                return true;
            }
            if (y as f32 - info.maxunder) > val as f32 {
                return true;
            }
        }
    }
    if info.maxover * info.maxover / n as f32 > info.maxerr {
        return false;
    }
    if info.maxunder * info.maxunder / n as f32 > info.maxerr {
        return false;
    }
    if (mse / n) as f32 > info.maxerr {
        return true;
    }
    false
}

/// `post_Y`.
fn post_y(a: &[i32], b: &[i32], pos: usize) -> i32 {
    if a[pos] < 0 {
        return b[pos];
    }
    if b[pos] < 0 {
        return a[pos];
    }
    (a[pos] + b[pos]) >> 1
}

/// `floor1_fit`: the posts (0..1023, bit 15: predicted, not coded) of the floor fitted to
/// `logmask`; `None` when the mask is silent throughout.
pub(crate) fn floor1_fit(look: &Floor1Look, logmdct: &[f32], logmask: &[f32]) -> Option<Vec<i32>> {
    let info = &look.vi;
    let n = look.n;
    let posts = look.posts;
    let mut nonzero = 0;
    let mut fits = [LsfitAcc::default(); VIF_POSIT + 1];
    let mut fit_value_a = [-200i32; VIF_POSIT + 2];
    let mut fit_value_b = [-200i32; VIF_POSIT + 2];
    let mut loneighbor = [0i32; VIF_POSIT + 2];
    let mut hineighbor = [1i32; VIF_POSIT + 2];
    let mut memo = [-1i32; VIF_POSIT + 2];
    // per bin, once: the quantised mask and whether the MDCT reaches it (the C recomputes both
    // in every segment it inspects)
    let q: Vec<i32> = logmask[..n as usize]
        .iter()
        .map(|&m| vorbis_dbquant(m))
        .collect();
    let reach: Vec<bool> = logmdct[..n as usize]
        .iter()
        .zip(&logmask[..n as usize])
        .map(|(&d, &m)| d + info.twofitatten >= m)
        .collect();
    for i in 0..posts - 1 {
        nonzero += accumulate_fit(
            &q,
            &reach,
            look.sorted_index[i],
            look.sorted_index[i + 1],
            &mut fits[i],
            n,
        );
    }
    if nonzero == 0 {
        return None;
    }
    let (mut y0, mut y1) = (-200, -200);
    fit_line(&fits[..posts - 1], &mut y0, &mut y1, info);
    fit_value_a[0] = y0;
    fit_value_b[0] = y0;
    fit_value_b[1] = y1;
    fit_value_a[1] = y1;
    for i in 2..posts {
        let sortpos = look.reverse_index[i] as usize;
        let ln = loneighbor[sortpos] as usize;
        let hn = hineighbor[sortpos] as usize;
        if memo[ln] == hn as i32 {
            continue;
        }
        let lsortpos = look.reverse_index[ln] as usize;
        let hsortpos = look.reverse_index[hn] as usize;
        memo[ln] = hn as i32;
        let lx = info.postlist[ln];
        let hx = info.postlist[hn];
        let ly = post_y(&fit_value_a, &fit_value_b, ln);
        let hy = post_y(&fit_value_a, &fit_value_b, hn);
        // the C exits on ly or hy == -1: post_Y gives -200 or a mean of two values ≥ 0
        if inspect_error(lx, hx, ly, hy, &q, &reach, info) {
            let (mut ly0, mut ly1, mut hy0, mut hy1) = (-200, -200, -200, -200);
            let ok0 = fit_line(&fits[lsortpos..sortpos], &mut ly0, &mut ly1, info);
            let ok1 = fit_line(&fits[sortpos..hsortpos], &mut hy0, &mut hy1, info);
            if !ok0 {
                ly0 = ly;
                ly1 = hy0;
            }
            if !ok1 {
                hy0 = ly1;
                hy1 = hy;
            }
            if !ok0 && !ok1 {
                fit_value_a[i] = -200;
                fit_value_b[i] = -200;
            } else {
                fit_value_b[ln] = ly0;
                if ln == 0 {
                    fit_value_a[ln] = ly0;
                }
                fit_value_a[i] = ly1;
                fit_value_b[i] = hy0;
                fit_value_a[hn] = hy1;
                if hn == 1 {
                    fit_value_b[hn] = hy1;
                }
                if ly1 >= 0 || hy0 >= 0 {
                    for j in (0..sortpos).rev() {
                        if hineighbor[j] == hn as i32 {
                            hineighbor[j] = i as i32;
                        } else {
                            break;
                        }
                    }
                    for j in sortpos + 1..posts {
                        if loneighbor[j] == ln as i32 {
                            loneighbor[j] = i as i32;
                        } else {
                            break;
                        }
                    }
                }
            }
        } else {
            fit_value_a[i] = -200;
            fit_value_b[i] = -200;
        }
    }
    let mut output = vec![0i32; posts];
    output[0] = post_y(&fit_value_a, &fit_value_b, 0);
    output[1] = post_y(&fit_value_a, &fit_value_b, 1);
    for i in 2..posts {
        let ln = look.loneighbor[i - 2] as usize;
        let hn = look.hineighbor[i - 2] as usize;
        let x0 = info.postlist[ln];
        let x1 = info.postlist[hn];
        let y0 = output[ln];
        let y1 = output[hn];
        let predicted = render_point(x0, x1, y0, y1, info.postlist[i]);
        let vx = post_y(&fit_value_a, &fit_value_b, i);
        output[i] = if vx >= 0 && predicted != vx {
            vx
        } else {
            predicted | 0x8000
        };
    }
    Some(output)
}

/// `floor1_interpolate_fit`: the posts `del`/65536 of the way from `a` to `b` (both needed).
pub(crate) fn floor1_interpolate_fit(
    look: &Floor1Look,
    a: Option<&[i32]>,
    b: Option<&[i32]>,
    del: i32,
) -> Option<Vec<i32>> {
    let (a, b) = (a?, b?);
    Some(
        (0..look.posts)
            .map(|i| {
                let mut o = ((65536 - del) * (a[i] & 0x7fff) + del * (b[i] & 0x7fff) + 32768) >> 16;
                if a[i] & 0x8000 != 0 && b[i] & 0x8000 != 0 {
                    o |= 0x8000;
                }
                o
            })
            .collect(),
    )
}

/// `floor1_encode`: the floor's bits into `opb` and the integer floor (dB steps × mult) into
/// `ilogmask[..pcmend/2]`; `post` is quantised to the floor's resolution in place, as the C
/// leaves it. Returns whether the floor is nonzero.
pub(crate) fn floor1_encode(
    opb: &mut OggPack,
    look: &Floor1Look,
    books: &[Codebook],
    post: Option<&mut [i32]>,
    ilogmask: &mut [i32],
    n: usize,
) -> bool {
    let info = &look.vi;
    let posts = look.posts;
    let Some(post) = post else {
        opb.write(0, 1);
        ilogmask[..n].fill(0);
        return false;
    };
    let mut out = [0i32; VIF_POSIT + 2];
    for i in 0..posts {
        let mut val = post[i] & 0x7fff;
        match info.mult {
            1 => val >>= 2,
            2 => val >>= 3,
            3 => val /= 12,
            4 => val >>= 4,
            _ => {}
        }
        post[i] = val | (post[i] & 0x8000);
    }
    out[0] = post[0];
    out[1] = post[1];
    for i in 2..posts {
        let ln = look.loneighbor[i - 2] as usize;
        let hn = look.hineighbor[i - 2] as usize;
        let x0 = info.postlist[ln];
        let x1 = info.postlist[hn];
        let y0 = post[ln];
        let y1 = post[hn];
        let predicted = render_point(x0, x1, y0, y1, info.postlist[i]);
        if post[i] & 0x8000 != 0 || predicted == post[i] {
            post[i] = predicted | 0x8000;
            out[i] = 0;
        } else {
            let headroom = if look.quant_q - predicted < predicted {
                look.quant_q - predicted
            } else {
                predicted
            };
            let mut val = post[i] - predicted;
            if val < 0 {
                if val < -headroom {
                    val = headroom - val - 1;
                } else {
                    val = -1 - (val << 1);
                }
            } else if val >= headroom {
                val += headroom;
            } else {
                val <<= 1;
            }
            out[i] = val;
            post[ln] &= 0x7fff;
            post[hn] &= 0x7fff;
        }
    }
    opb.write(1, 1);
    let qbits = ilog((look.quant_q - 1) as u32);
    opb.write(out[0] as i64, qbits);
    opb.write(out[1] as i64, qbits);
    let mut j = 2;
    for i in 0..info.partitions as usize {
        let class = info.partitionclass[i] as usize;
        let cdim = info.class_dim[class] as usize;
        let csubbits = info.class_subs[class];
        let csub = 1usize << csubbits;
        let mut bookas = [0usize; 8];
        if csubbits != 0 {
            let mut maxval = [0i32; 8];
            for k in 0..csub {
                let booknum = info.class_subbook[class][k];
                maxval[k] = if booknum < 0 {
                    1
                } else {
                    books[booknum as usize].entries
                };
            }
            let mut cval = 0;
            let mut cshift = 0;
            for k in 0..cdim {
                for l in 0..csub {
                    if out[j + k] < maxval[l] {
                        bookas[k] = l;
                        break;
                    }
                }
                cval |= (bookas[k] as i32) << cshift;
                cshift += csubbits;
            }
            books[info.class_book[class] as usize].encode(cval, opb);
        }
        for k in 0..cdim {
            let book = info.class_subbook[class][bookas[k]];
            if book >= 0 && out[j + k] < books[book as usize].entries {
                books[book as usize].encode(out[j + k], opb);
            }
        }
        j += cdim;
    }
    let mut hx = 0;
    let mut lx = 0;
    let mut ly = post[0] * info.mult;
    for j in 1..posts {
        let current = look.forward_index[j] as usize;
        let mut hy = post[current] & 0x7fff;
        if hy == post[current] {
            hy *= info.mult;
            hx = info.postlist[current];
            render_line0(n as i32, lx, hx, ly, hy, ilogmask);
            lx = hx;
            ly = hy;
        }
    }
    ilogmask[hx as usize..n].fill(ly);
    true
}
