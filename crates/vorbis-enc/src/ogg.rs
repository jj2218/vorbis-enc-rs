//! Ogg framing (`framing.c`, libogg 1.3.5's stream writer): packets laced into pages, each
//! page with its header (flags, granule position, serial and page numbers, lacing) and CRC.

/// libogg's page fill: a page closes once more than this many body bytes and four packets
/// are buffered (the `_fill` variants, which change it, are not ported).
const NFILL: usize = 4096;

/// libogg's CRC-32 (`_ogg_crc_init`: polynomial 0x04c11db7, MSb first, initial 0), byte-wise.
const CRC: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = (i as u32) << 24;
        let mut j = 0;
        while j < 8 {
            crc = (crc << 1) ^ if crc & (1 << 31) != 0 { 0x04c1_1db7 } else { 0 };
            j += 1;
        }
        t[i] = crc;
        i += 1;
    }
    t
};

/// `_os_update_crc` (the C's slicing-by-8 computes the same value).
fn update_crc(mut crc: u32, buf: &[u8]) -> u32 {
    for &b in buf {
        crc = (crc << 8) ^ CRC[(((crc >> 24) & 0xff) as u8 ^ b) as usize];
    }
    crc
}

/// `ogg_stream_state` (writing).
#[derive(Clone, Debug)]
pub(crate) struct OggStream {
    serialno: i32,
    body: Vec<u8>,
    /// bytes of `body` already returned in pages
    body_returned: usize,
    /// lacing values; 0x100 marks a packet's first segment
    lacing: Vec<u32>,
    granule: Vec<i64>,
    granulepos: i64,
    pageno: i64,
    b_o_s: bool,
    e_o_s: bool,
}

impl OggStream {
    /// `ogg_stream_init`.
    pub(crate) fn new(serialno: i32) -> Self {
        OggStream {
            serialno,
            body: Vec::new(),
            body_returned: 0,
            lacing: Vec::new(),
            granule: Vec::new(),
            granulepos: 0,
            pageno: 0,
            b_o_s: false,
            e_o_s: false,
        }
    }

    /// `ogg_stream_packetin`.
    pub(crate) fn packetin(&mut self, packet: &[u8], e_o_s: bool, granulepos: i64) {
        let bytes = packet.len();
        let lacing_vals = bytes / 255 + 1;
        if self.body_returned != 0 {
            self.body.drain(..self.body_returned);
            self.body_returned = 0;
        }
        self.body.extend_from_slice(packet);
        let first = self.lacing.len();
        for _ in 0..lacing_vals - 1 {
            self.lacing.push(255);
            self.granule.push(self.granulepos);
        }
        self.lacing.push((bytes % 255) as u32);
        self.granule.push(granulepos);
        self.granulepos = granulepos;
        self.lacing[first] |= 0x100;
        if e_o_s {
            self.e_o_s = true;
        }
    }

    /// `ogg_stream_flush_i` (`nfill` [`NFILL`]): the next page (header then body), if one is due.
    fn flush_i(&mut self, mut force: bool) -> Option<Vec<u8>> {
        let maxvals = self.lacing.len().min(255);
        if maxvals == 0 {
            return None;
        }
        let mut vals = 0;
        let mut granule_pos: i64 = -1;
        if !self.b_o_s {
            // the first page holds the first packet only
            granule_pos = 0;
            while vals < maxvals {
                if (self.lacing[vals] & 0x0ff) < 255 {
                    vals += 1;
                    break;
                }
                vals += 1;
            }
        } else {
            let mut acc = 0;
            let mut packets_done = 0;
            let mut packet_just_done = 0;
            while vals < maxvals {
                if acc > NFILL && packet_just_done >= 4 {
                    force = true;
                    break;
                }
                acc += (self.lacing[vals] & 0x0ff) as usize;
                if (self.lacing[vals] & 0xff) < 255 {
                    granule_pos = self.granule[vals];
                    packets_done += 1;
                    packet_just_done = packets_done;
                } else {
                    packet_just_done = 0;
                }
                vals += 1;
            }
            if vals == 255 {
                force = true;
            }
        }
        if !force {
            return None;
        }
        let mut page = Vec::with_capacity(27 + vals + 4096);
        page.extend_from_slice(b"OggS");
        page.push(0);
        let mut flags = 0u8;
        if self.lacing[0] & 0x100 == 0 {
            flags |= 0x01;
        }
        if !self.b_o_s {
            flags |= 0x02;
        }
        if self.e_o_s && self.lacing.len() == vals {
            flags |= 0x04;
        }
        page.push(flags);
        self.b_o_s = true;
        page.extend_from_slice(&granule_pos.to_le_bytes());
        page.extend_from_slice(&self.serialno.to_le_bytes());
        page.extend_from_slice(&(self.pageno as u32).to_le_bytes());
        self.pageno += 1;
        page.extend_from_slice(&[0; 4]);
        page.push(vals as u8);
        let mut bytes = 0;
        for &l in &self.lacing[..vals] {
            page.push((l & 0xff) as u8);
            bytes += (l & 0xff) as usize;
        }
        page.extend_from_slice(&self.body[self.body_returned..self.body_returned + bytes]);
        self.lacing.drain(..vals);
        self.granule.drain(..vals);
        self.body_returned += bytes;
        // ogg_page_checksum_set: over the header (CRC field zero) and the body
        let crc = update_crc(0, &page);
        page[22..26].copy_from_slice(&crc.to_le_bytes());
        Some(page)
    }

    /// `ogg_stream_flush`: a page of whatever is buffered.
    pub(crate) fn flush(&mut self) -> Option<Vec<u8>> {
        self.flush_i(true)
    }

    /// `ogg_stream_pageout`: a page once ~4 kB (or the stream's first or last packet) is buffered.
    pub(crate) fn pageout(&mut self) -> Option<Vec<u8>> {
        let force =
            (self.e_o_s && !self.lacing.is_empty()) || (!self.lacing.is_empty() && !self.b_o_s);
        self.flush_i(force)
    }
}

#[cfg(test)]
mod vs_c {
    use super::*;
    use aotuv_sys::{Mode, encode_packets, spike_pcm, thetest};

    /// The C's packets, framed as the C's harness frames them (headers flushed to their own
    /// pages, a pageout loop per audio packet, a final flush): the C's Ogg stream byte for
    /// byte, over the spike's inputs and other rates, channel counts and modes (packet sizes
    /// from tens of bytes to multi-page).
    #[test]
    fn pages_match_c() {
        let cases: [(u32, usize, Mode); 7] = [
            (16, 1, Mode::Vbr(0.5)),
            (44, 1, Mode::Vbr(0.5)),
            (48, 1, Mode::Vbr(1.0)),
            (44, 2, Mode::Vbr(-0.1)),
            (8, 1, Mode::Vbr(0.2)),
            (96, 2, Mode::Vbr(0.9)),
            (
                48,
                2,
                Mode::Managed {
                    min: -1,
                    nominal: 128_000,
                    max: -1,
                },
            ),
        ];
        let mut pages = 0;
        for (khz, ch, mode) in cases {
            let (rate, pcm) = thetest(khz);
            let (packets, c) = encode_packets(rate as i64, mode, &spike_pcm(&pcm, ch)).unwrap();
            let mut os = OggStream::new(1);
            let mut s = Vec::new();
            for p in &packets[..3] {
                os.packetin(&p.data, p.eos, p.granulepos);
            }
            while let Some(pg) = os.flush() {
                s.extend(pg);
                pages += 1;
            }
            for p in &packets[3..] {
                os.packetin(&p.data, p.eos, p.granulepos);
                while let Some(pg) = os.pageout() {
                    s.extend(pg);
                    pages += 1;
                }
            }
            while let Some(pg) = os.flush() {
                s.extend(pg);
                pages += 1;
            }
            assert!(
                s == c,
                "thetest{khz} × {ch} {mode:?}: {} vs C {} bytes",
                s.len(),
                c.len()
            );
        }
        println!("vorbis ogg: {pages} pages byte-identical over 7 streams");
    }

    /// Random packet sizes (empty, tiny: the 255-segment limit, 255·k ± 1, up to multi-page),
    /// granule positions, call sequences (pageout or flush after each packet) and an end of
    /// stream: every page byte for byte vs libogg.
    #[test]
    fn random_framing_matches_c() {
        let mut r = crate::testutil::Rng::new(16);
        let mut pages = 0;
        for stream in 0..400u64 {
            let mut os = OggStream::new(stream as i32 * 7919 - 3);
            let mut c = aotuv_sys::OggStream::new(stream as i32 * 7919 - 3);
            let n = 1 + r.u64() % 300;
            let mut granule = 0i64;
            for k in 0..n {
                let len = match r.u64() % 6 {
                    0 => 0,
                    1 => (r.u64() % 16) as usize,
                    2 => 255 * (1 + r.u64() % 4) as usize + (r.u64() % 3) as usize - 1,
                    3 => (r.u64() % 600) as usize,
                    4 => (r.u64() % 9000) as usize,
                    _ => (r.u64() % 70000) as usize,
                };
                let data: Vec<u8> = (0..len).map(|_| r.u64() as u8).collect();
                granule += (r.u64() % 4096) as i64;
                let eos = k == n - 1 && stream % 3 != 0;
                os.packetin(&data, eos, granule);
                c.packetin(&data, eos, granule);
                let flush = r.u64().is_multiple_of(8);
                loop {
                    let (a, b) = if flush {
                        (os.flush(), c.page(true))
                    } else {
                        (os.pageout(), c.page(false))
                    };
                    assert_eq!(a, b, "stream {stream} packet {k}");
                    if a.is_none() {
                        break;
                    }
                    pages += 1;
                }
            }
            loop {
                let (a, b) = (os.flush(), c.page(true));
                assert_eq!(a, b, "stream {stream} final flush");
                if a.is_none() {
                    break;
                }
                pages += 1;
            }
        }
        println!("vorbis ogg: {pages} random pages byte-identical");
    }
}
