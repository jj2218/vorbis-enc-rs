//! The bit packer (`bitwise.c`, libogg's `oggpack_*` writer): values written least significant
//! bit first into bytes filled from bit 0; the final partial byte zero-padded.

/// `oggpack_buffer` (writing).
#[derive(Clone, Debug, Default)]
pub(crate) struct OggPack {
    buf: Vec<u8>,
    /// bits used in the last byte (0: the next write starts a new byte)
    endbit: u32,
}

/// `mask[bits]`.
#[inline]
fn mask(bits: u32) -> u64 {
    if bits >= 64 { !0 } else { (1u64 << bits) - 1 }
}

impl OggPack {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// `oggpack_write`: the low `bits` (≤ 32) bits of `value` (the C masks a `long`, so a
    /// negative value writes its two's complement bits).
    pub(crate) fn write(&mut self, value: i64, bits: u32) {
        debug_assert!(bits <= 32);
        let mut v = value as u64 & mask(bits);
        let mut bits = bits;
        if bits == 0 {
            return;
        }
        if self.endbit != 0 {
            let last = self.buf.last_mut().unwrap();
            *last |= (v << self.endbit) as u8;
            let room = 8 - self.endbit;
            if bits < room {
                self.endbit += bits;
                return;
            }
            v >>= room;
            bits -= room;
            self.endbit = 0;
        }
        while bits >= 8 {
            self.buf.push(v as u8);
            v >>= 8;
            bits -= 8;
        }
        if bits > 0 {
            self.buf.push(v as u8);
            self.endbit = bits;
        }
    }

    /// `oggpack_bytes`: bytes used (a partial byte counts).
    pub(crate) fn bytes(&self) -> usize {
        self.buf.len()
    }

    /// `oggpack_bits`.
    #[cfg(test)]
    pub(crate) fn bits(&self) -> usize {
        if self.endbit == 0 {
            self.buf.len() * 8
        } else {
            (self.buf.len() - 1) * 8 + self.endbit as usize
        }
    }

    /// `oggpack_get_buffer`: the packet so far.
    pub(crate) fn buffer(&self) -> &[u8] {
        &self.buf
    }

    /// `oggpack_writetrunc`: keep the first `bits` (≤ those written; the C's pointer would
    /// leave the buffer below 0, so a negative count keeps none).
    pub(crate) fn writetrunc(&mut self, bits: i64) {
        let bits = bits.max(0) as usize;
        let bytes = bits >> 3;
        let endbit = (bits - bytes * 8) as u32;
        self.buf.truncate(bytes + (endbit != 0) as usize);
        if endbit != 0 {
            self.buf[bytes] &= mask(endbit) as u8;
        }
        self.endbit = endbit;
    }

    /// `_v_writestring`: bytes, 8 bits each.
    pub(crate) fn write_bytes(&mut self, s: &[u8]) {
        for &b in s {
            self.write(b as i64, 8);
        }
    }
}

/// `ov_ilog`: the number of bits needed for `v` (0 for 0).
pub(crate) fn ilog(v: u32) -> u32 {
    32 - v.leading_zeros()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mixed widths reproduce a reference packing (LSb first).
    #[test]
    fn packs_lsb_first() {
        let mut o = OggPack::new();
        o.write(1, 1);
        o.write(0b101, 3);
        o.write(0xABCD, 16);
        o.write(-1, 32);
        o.write(0, 0);
        o.write(0x3, 2);
        // bits: 1 | 101<<1 | 0xABCD<<4 | 0xFFFFFFFF<<20 | 11<<52
        let want: u64 = 1 | 0b101 << 1 | 0xABCD << 4 | 0xFFFF_FFFF << 20 | 0b11 << 52;
        assert_eq!(o.bytes(), 7);
        assert_eq!(o.bits(), 54);
        assert_eq!(o.buffer(), &want.to_le_bytes()[..7]);
        assert_eq!(ilog(0), 0);
        assert_eq!(ilog(1), 1);
        assert_eq!(ilog(255), 8);
        assert_eq!(ilog(256), 9);
    }
}
