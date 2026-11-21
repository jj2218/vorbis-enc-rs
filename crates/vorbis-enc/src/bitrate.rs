//! Bitrate management (`bitrate.c`): for a managed stream, which of a block's packet blobs
//! to send (the average reservoir steers a float choice, the min/max reservoir bounds it),
//! padded or truncated to the limits; a VBR stream sends the middle blob.

use crate::bitpack::OggPack;
use crate::setup::CodecSetup;
use crate::templates::PACKETBLOBS;

#[cfg(test)]
thread_local! {
    /// Per path of [`BitrateManager::choose`], how often the tests reached it (the managed
    /// tests assert each is: avg to blob 0, avg up, the min loop, the max loop, choice < 0,
    /// truncation, padding, the reservoir clamped back to the desired fill). Per thread: tests
    /// running in parallel don't count for each other. (Paths 4 and 5 always count alike:
    /// blob 0 over the bit budget is over the byte limit too.)
    pub(crate) static HITS: [std::cell::Cell<usize>; 8] = Default::default();
}

#[inline]
fn hit(_i: usize) {
    #[cfg(test)]
    HITS.with(|h| h[_i].set(h[_i].get() + 1));
}

/// `bitrate_manager_state`.
#[derive(Clone, Debug, Default)]
pub(crate) struct BitrateManager {
    pub avg_reservoir: i64,
    pub minmax_reservoir: i64,
    pub avg_bitsper: i64,
    pub min_bitsper: i64,
    pub max_bitsper: i64,
    pub short_per_long: i64,
    pub avgfloat: f64,
}

impl BitrateManager {
    /// `vorbis_bitrate_init`.
    pub(crate) fn new(ci: &CodecSetup) -> Self {
        let bi = &ci.bi;
        let mut bm = BitrateManager::default();
        if ci.is_managed() {
            let ratesamples = ci.rate as f64;
            let halfsamples = (ci.blocksizes[0] >> 1) as i32 as f64;
            bm.short_per_long = ci.blocksizes[1] / ci.blocksizes[0];
            bm.avg_bitsper =
                (1.0 * bi.avg_rate as f64 * halfsamples / ratesamples).round_ties_even() as i64;
            bm.min_bitsper =
                (1.0 * bi.min_rate as f64 * halfsamples / ratesamples).round_ties_even() as i64;
            bm.max_bitsper =
                (1.0 * bi.max_rate as f64 * halfsamples / ratesamples).round_ties_even() as i64;
            bm.avgfloat = (PACKETBLOBS / 2) as f64;
            let desired_fill = (bi.reservoir_bits as f64 * bi.reservoir_bias) as i64;
            bm.minmax_reservoir = desired_fill;
            bm.avg_reservoir = desired_fill;
        }
        bm
    }

    /// `vorbis_bitrate_addblock` + `vorbis_bitrate_flushpacket`: the blob to send for a block
    /// of size `w` (its packet as the C leaves it: padded or truncated in place).
    pub(crate) fn choose(&mut self, ci: &CodecSetup, w: usize, blobs: &mut [OggPack]) -> usize {
        if !ci.is_managed() {
            return PACKETBLOBS / 2;
        }
        let bi = &ci.bi;
        let bytes8 = |b: &OggPack| (b.bytes() * 8) as i64;
        let mut choice = self.avgfloat.round_ties_even() as i32;
        let mut this_bits = bytes8(&blobs[choice as usize]);
        let spl = if w != 0 { self.short_per_long } else { 1 };
        let min_target_bits = self.min_bitsper * spl;
        let max_target_bits = self.max_bitsper * spl;
        let samples = (ci.blocksizes[w] >> 1) as i32;
        let desired_fill = (bi.reservoir_bits as f64 * bi.reservoir_bias) as i64;
        if self.avg_bitsper > 0 {
            let avg_target_bits = self.avg_bitsper * spl;
            let slewlimit = 15.0 / bi.slew_damp;
            if self.avg_reservoir + (this_bits - avg_target_bits) > desired_fill {
                while choice > 0
                    && this_bits > avg_target_bits
                    && self.avg_reservoir + (this_bits - avg_target_bits) > desired_fill
                {
                    choice -= 1;
                    this_bits = bytes8(&blobs[choice as usize]);
                    if choice == 0 {
                        hit(0);
                    }
                }
            } else if self.avg_reservoir + (this_bits - avg_target_bits) < desired_fill {
                while choice + 1 < PACKETBLOBS as i32
                    && this_bits < avg_target_bits
                    && self.avg_reservoir + (this_bits - avg_target_bits) < desired_fill
                {
                    choice += 1;
                    this_bits = bytes8(&blobs[choice as usize]);
                    hit(1);
                }
            }
            let mut slew =
                (choice as f64 - self.avgfloat).round_ties_even() / samples as f64 * ci.rate as f64;
            slew = slew.clamp(-slewlimit, slewlimit);
            self.avgfloat += slew / ci.rate as f64 * samples as f64;
            choice = self.avgfloat.round_ties_even() as i32;
            this_bits = bytes8(&blobs[choice as usize]);
        }
        if self.min_bitsper > 0 && this_bits < min_target_bits {
            while self.minmax_reservoir - (min_target_bits - this_bits) < 0 {
                hit(2);
                choice += 1;
                if choice >= PACKETBLOBS as i32 {
                    break;
                }
                this_bits = bytes8(&blobs[choice as usize]);
            }
        }
        if self.max_bitsper > 0 && this_bits > max_target_bits {
            while self.minmax_reservoir + (this_bits - max_target_bits) > bi.reservoir_bits {
                hit(3);
                choice -= 1;
                if choice < 0 {
                    break;
                }
                this_bits = bytes8(&blobs[choice as usize]);
            }
        }
        if choice < 0 {
            let maxsize = (max_target_bits + (bi.reservoir_bits - self.minmax_reservoir)) / 8;
            choice = 0;
            hit(4);
            if blobs[0].bytes() as i64 > maxsize {
                blobs[0].writetrunc(maxsize * 8);
                hit(5);
                this_bits = bytes8(&blobs[0]);
            }
        } else {
            let mut minsize = (min_target_bits - self.minmax_reservoir + 7) / 8;
            if choice >= PACKETBLOBS as i32 {
                choice = PACKETBLOBS as i32 - 1;
            }
            let b = &mut blobs[choice as usize];
            minsize -= b.bytes() as i64;
            while minsize > 0 {
                minsize -= 1;
                b.write(0, 8);
                hit(6);
            }
            this_bits = bytes8(b);
        }
        if self.min_bitsper > 0 || self.max_bitsper > 0 {
            if max_target_bits > 0 && this_bits > max_target_bits {
                self.minmax_reservoir += this_bits - max_target_bits;
            } else if min_target_bits > 0 && this_bits < min_target_bits {
                self.minmax_reservoir += this_bits - min_target_bits;
            } else if self.minmax_reservoir > desired_fill {
                if max_target_bits > 0 {
                    self.minmax_reservoir += this_bits - max_target_bits;
                    if self.minmax_reservoir < desired_fill {
                        hit(7);
                        self.minmax_reservoir = desired_fill;
                    }
                } else {
                    self.minmax_reservoir = desired_fill;
                }
            } else if min_target_bits > 0 {
                self.minmax_reservoir += this_bits - min_target_bits;
                if self.minmax_reservoir > desired_fill {
                    self.minmax_reservoir = desired_fill;
                }
            } else {
                self.minmax_reservoir = desired_fill;
            }
        }
        if self.avg_bitsper > 0 {
            self.avg_reservoir += this_bits - self.avg_bitsper * spl;
        }
        choice as usize
    }
}
