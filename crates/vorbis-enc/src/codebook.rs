//! The encoder's codebooks (`sharedbook.c` `vorbis_book_init_encode`, `_make_words`,
//! `_float32_unpack`; `codebook.c` `vorbis_book_encode`): each entry's codeword, bit-reversed
//! for the LSB-first packer, and the lattice parameters the residue search uses.

use crate::bitpack::OggPack;
use crate::info::book_maptype1_quantvals;
use crate::templates::StaticCodebook;

/// `codebook` (encode side).
#[derive(Clone, Debug)]
pub(crate) struct Codebook {
    pub c: &'static StaticCodebook,
    pub dim: i32,
    pub entries: i32,
    /// per entry, the codeword bit-reversed (`codelist`; unused entries: 0)
    pub codelist: Vec<u32>,
    pub quantvals: i32,
    pub minval: i32,
    pub delta: i32,
}

/// `_float32_unpack`: the header's packed float (21-bit mantissa, biased exponent, sign).
fn float32_unpack(val: i64) -> f32 {
    let mut mant = (val & 0x1fffff) as f64;
    let sign = val & 0x8000_0000;
    let mut exp = (val & 0x7fe0_0000) >> 21;
    if sign != 0 {
        mant = -mant;
    }
    exp = exp - 20 - 768;
    exp = exp.clamp(-63, 63);
    (mant * (2f64).powi(exp as i32)) as f32
}

/// `_make_words`: the canonical Huffman codewords of a length list (bit-reversed), `None`
/// when the lengths over- or under-specify the tree (the C returns NULL).
fn make_words(l: &[u8]) -> Option<Vec<u32>> {
    let mut marker = [0u32; 33];
    let mut r = vec![0u32; l.len()];
    let mut count = 0;
    for &len in l {
        let length = len as usize;
        if length > 0 {
            let mut entry = marker[length];
            if length < 32 && (entry >> length) != 0 {
                return None;
            }
            r[count] = entry;
            count += 1;
            for j in (1..=length).rev() {
                if marker[j] & 1 != 0 {
                    if j == 1 {
                        marker[1] += 1;
                    } else {
                        marker[j] = marker[j - 1] << 1;
                    }
                    break;
                }
                marker[j] += 1;
            }
            for j in length + 1..33 {
                if (marker[j] >> 1) == entry {
                    entry = marker[j];
                    marker[j] = marker[j - 1] << 1;
                } else {
                    break;
                }
            }
        } else {
            count += 1;
        }
    }
    if !(count == 1 && marker[2] == 2) {
        for (i, &m) in marker.iter().enumerate().skip(1) {
            if m & (0xffff_ffffu32 >> (32 - i)) != 0 {
                return None;
            }
        }
    }
    for (i, &len) in l.iter().enumerate() {
        let mut temp = 0u32;
        for j in 0..len as u32 {
            temp <<= 1;
            temp |= (r[i] >> j) & 1;
        }
        r[i] = temp;
    }
    Some(r)
}

impl Codebook {
    /// `vorbis_book_init_encode`. The setup's books all have valid length lists (the headers
    /// test packs them all); a NULL codelist would crash the C's first encode.
    pub(crate) fn new(s: &'static StaticCodebook) -> Self {
        Codebook {
            c: s,
            dim: s.dim as i32,
            entries: s.entries as i32,
            codelist: make_words(s.lengthlist).expect("a setup book's lengths form a tree"),
            quantvals: book_maptype1_quantvals(s) as i32,
            minval: (float32_unpack(s.q_min) as f64).round_ties_even() as i32,
            delta: (float32_unpack(s.q_delta) as f64).round_ties_even() as i32,
        }
    }

    /// `vorbis_book_encode`: entry `a`'s codeword (out of range: nothing written).
    pub(crate) fn encode(&self, a: i32, b: &mut OggPack) {
        if a < 0 || a >= self.entries {
            return;
        }
        let len = self.c.lengthlist[a as usize] as u32;
        b.write(self.codelist[a as usize] as i64, len);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A complete tree: lengths 1, 2, 3, 3 give 0, 10, 110, 111 (bit-reversed: 0, 1, 3, 7);
    /// an over-full one is rejected; one single-entry book (length 1) is allowed.
    #[test]
    fn words() {
        assert_eq!(make_words(&[1, 2, 3, 3]), Some(vec![0, 1, 3, 7]));
        assert_eq!(make_words(&[2, 0, 2, 1]), Some(vec![0, 0, 2, 1]));
        assert_eq!(make_words(&[1, 1, 1]), None);
        assert_eq!(make_words(&[1, 2]), None);
        assert_eq!(make_words(&[1]), Some(vec![0]));
    }

    #[test]
    fn unpack() {
        // 1.0 = mantissa 2^20 at exponent 768 (2^(768 - 20 - 768))
        assert_eq!(float32_unpack((768 << 21) | (1 << 20)), 1.0);
        assert_eq!(float32_unpack(0x8000_0000 | (767 << 21) | (3 << 19)), -0.75);
    }
}
