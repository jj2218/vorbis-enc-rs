//! Residue 1 and 2, the encoder side (`res0.c`; residue 0 has no encoder in libvorbis): the
//! look (stage books per partition class), the classification of each partition by its peak
//! and sum (`_01class`, `_2class`), and the partitioned VQ encode (`_01forward`) with the
//! lattice search (`local_book_besterror`). Integer arithmetic throughout.
#![allow(clippy::needless_range_loop)] // index loops mirror the C code 1:1

use crate::bitpack::{OggPack, ilog};
use crate::codebook::Codebook;
use crate::templates::InfoResidue0;

/// `vorbis_look_residue0` (the encoder's part; the C's bit statistics are never read).
#[derive(Clone, Debug)]
pub(crate) struct ResidueLook {
    pub info: InfoResidue0,
    pub stages: usize,
    /// the phrase (classification) book's index into the setup's books
    pub phrasebook: usize,
    /// per partition class and stage, the stage book's index (`None`: that stage skips it)
    pub partbooks: Vec<Vec<Option<usize>>>,
}

impl ResidueLook {
    /// `res0_look`.
    pub(crate) fn new(info: &InfoResidue0) -> Self {
        let parts = info.partitions as usize;
        let mut acc = 0;
        let mut maxstage = 0;
        let mut partbooks = vec![Vec::new(); parts];
        for j in 0..parts {
            let stages = ilog(info.secondstages[j] as u32) as usize;
            if stages > 0 {
                maxstage = maxstage.max(stages);
                partbooks[j] = vec![None; stages];
                for k in 0..stages {
                    if info.secondstages[j] & (1 << k) != 0 {
                        partbooks[j][k] = Some(info.booklist[acc] as usize);
                        acc += 1;
                    }
                }
            }
        }
        ResidueLook {
            info: info.clone(),
            stages: maxstage,
            phrasebook: info.groupbook as usize,
            partbooks,
        }
    }
}

/// `local_book_besterror`: the entry nearest `a` (the lattice point, or by brute force when
/// that entry is unused), subtracted from `a`; returns the entry.
fn local_book_besterror(book: &Codebook, a: &mut [i32]) -> i32 {
    let dim = book.dim as usize;
    let minval = book.minval;
    let del = book.delta;
    let qv = book.quantvals;
    let ze = qv >> 1;
    let mut index = 0i32;
    let mut p = [0i32; 8];
    let mut o = dim;
    for _ in 0..dim {
        o -= 1;
        let v = if del != 1 {
            a[o].wrapping_sub(minval).wrapping_add(del >> 1) / del
        } else {
            a[o].wrapping_sub(minval)
        };
        let m = if v < ze {
            (ze.wrapping_sub(v) << 1).wrapping_sub(1)
        } else {
            v.wrapping_sub(ze) << 1
        };
        index = index * qv
            + if m < 0 {
                0
            } else if m >= qv {
                qv - 1
            } else {
                m
            };
        p[o] = v.wrapping_mul(del).wrapping_add(minval);
    }
    if book.c.lengthlist[index as usize] == 0 {
        let lengthlist = book.c.lengthlist;
        let mut best = -1;
        // one spare: the counter's carry runs one past the last dimension after the last entry
        let mut e = [0i32; 9];
        let maxval = book.minval + book.delta * (book.quantvals - 1);
        for i in 0..book.entries as usize {
            if lengthlist[i] > 0 {
                let mut this = 0;
                for j in 0..dim {
                    let val = e[j].wrapping_sub(a[j]);
                    this = val.wrapping_mul(val).wrapping_add(this);
                }
                if best == -1 || this < best {
                    p.copy_from_slice(&e[..8]);
                    best = this;
                    index = i as i32;
                }
            }
            let mut j = 0;
            while e[j] >= maxval {
                e[j] = 0;
                j += 1;
            }
            if e[j] >= 0 {
                e[j] += book.delta;
            }
            e[j] = -e[j];
        }
    }
    for i in 0..dim {
        a[i] = a[i].wrapping_sub(p[i]);
    }
    index
}

/// `_encodepart`: `vec` as `len/dim` entries of `book` (the C's bit count feeds only its
/// training statistics, not ported).
fn encodepart(opb: &mut OggPack, vec: &mut [i32], book: &Codebook) {
    for v in vec.chunks_exact_mut(book.dim as usize) {
        let entry = local_book_besterror(book, v);
        book.encode(entry, opb);
    }
}

/// `_01class`: per channel and partition, the first class whose peak and (scaled) sum
/// bounds hold.
fn class01(look: &ResidueLook, input: &[&mut [i32]]) -> Vec<Vec<i64>> {
    let info = &look.info;
    let samples_per_partition = info.grouping as usize;
    let possible_partitions = info.partitions as usize;
    let n = (info.end - info.begin) as usize;
    let partvals = n / samples_per_partition;
    // `float scale = 100./samples_per_partition`
    let scale = (100.0 / samples_per_partition as f64) as f32;
    let mut partword = vec![vec![0i64; partvals]; input.len()];
    for i in 0..partvals {
        let offset = i * samples_per_partition + info.begin as usize;
        for (j, inj) in input.iter().enumerate() {
            let mut max = 0;
            let mut ent = 0i32;
            for k in 0..samples_per_partition {
                max = max.max(inj[offset + k].wrapping_abs());
                ent = ent.wrapping_add(inj[offset + k].wrapping_abs());
            }
            // `ent *= scale`: int to float, product, truncated back
            ent = (ent as f32 * scale) as i32;
            let mut k = 0;
            while k < possible_partitions - 1 {
                if max <= info.classmetric1[k]
                    && (info.classmetric2[k] < 0 || ent < info.classmetric2[k])
                {
                    break;
                }
                k += 1;
            }
            partword[j][i] = k as i64;
        }
    }
    partword
}

/// `_2class`: per partition of the interleaved channels, the first class whose bounds on the
/// first channel's peak and the others' hold.
fn class2(look: &ResidueLook, input: &[&mut [i32]]) -> Vec<Vec<i64>> {
    let info = &look.info;
    let ch = input.len();
    let samples_per_partition = info.grouping as usize;
    let possible_partitions = info.partitions as usize;
    let n = (info.end - info.begin) as usize;
    let partvals = n / samples_per_partition;
    let mut partword = vec![vec![0i64; partvals]];
    let mut l = info.begin as usize / ch;
    for i in 0..partvals {
        let mut magmax = 0;
        let mut angmax = 0;
        let mut j = 0;
        while j < samples_per_partition {
            magmax = magmax.max(input[0][l].wrapping_abs());
            for inp in &input[1..] {
                angmax = angmax.max(inp[l].wrapping_abs());
            }
            l += 1;
            j += ch;
        }
        let mut j = 0;
        while j < possible_partitions - 1 {
            if magmax <= info.classmetric1[j] && angmax <= info.classmetric2[j] {
                break;
            }
            j += 1;
        }
        partword[0][i] = j as i64;
    }
    partword
}

/// `_01forward`: the classes (phrase book, stage 0) and each stage's VQ of the partitions;
/// `input` is consumed (each encoded value subtracted).
fn forward01(
    opb: &mut OggPack,
    look: &ResidueLook,
    books: &[Codebook],
    input: &mut [&mut [i32]],
    partword: &[Vec<i64>],
) {
    let info = &look.info;
    let samples_per_partition = info.grouping as usize;
    let possible_partitions = info.partitions as i64;
    let phrasebook = &books[look.phrasebook];
    let partitions_per_word = phrasebook.dim as usize;
    let n = (info.end - info.begin) as usize;
    let partvals = n / samples_per_partition;
    for s in 0..look.stages {
        let mut i = 0;
        while i < partvals {
            if s == 0 {
                for pw in partword.iter().take(input.len()) {
                    let mut val = pw[i];
                    for k in 1..partitions_per_word {
                        val *= possible_partitions;
                        if i + k < partvals {
                            val += pw[i + k];
                        }
                    }
                    if val < phrasebook.entries as i64 {
                        phrasebook.encode(val as i32, opb);
                    }
                }
            }
            let mut k = 0;
            while k < partitions_per_word && i < partvals {
                let offset = i * samples_per_partition + info.begin as usize;
                for (j, inj) in input.iter_mut().enumerate() {
                    let c = partword[j][i] as usize;
                    // (a book exactly where `secondstages[c]` has bit `s`)
                    if let Some(b) = look.partbooks[c].get(s).copied().flatten() {
                        encodepart(
                            opb,
                            &mut inj[offset..offset + samples_per_partition],
                            &books[b],
                        );
                    }
                }
                k += 1;
                i += 1;
            }
        }
    }
}

/// `res1_class` + `res1_forward`: the nonzero channels, each coded on its own.
pub(crate) fn res1_encode(
    opb: &mut OggPack,
    look: &ResidueLook,
    books: &[Codebook],
    input: &mut [&mut [i32]],
    nonzero: &[bool],
) {
    let mut used: Vec<&mut [i32]> = Vec::new();
    for (inp, &nz) in input.iter_mut().zip(nonzero) {
        if nz {
            used.push(inp);
        }
    }
    if used.is_empty() {
        return;
    }
    let partword = class01(look, &used);
    forward01(opb, look, books, &mut used, &partword);
}

/// `res2_class` + `res2_forward`: all channels interleaved into one vector (when any is
/// nonzero), classified and coded as one.
pub(crate) fn res2_encode(
    opb: &mut OggPack,
    look: &ResidueLook,
    books: &[Codebook],
    input: &mut [&mut [i32]],
    nonzero: &[bool],
) {
    if !nonzero.iter().any(|&nz| nz) {
        return;
    }
    let partword = class2(look, input);
    let ch = input.len();
    let n = input[0].len();
    let mut work = vec![0i32; ch * n];
    for (i, inp) in input.iter().enumerate() {
        for j in 0..n {
            work[j * ch + i] = inp[j];
        }
    }
    forward01(opb, look, books, &mut [&mut work[..]], &partword);
}
