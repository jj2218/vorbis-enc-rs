//! FFI to aoTuV b6.03, used only as a test and benchmark oracle (strict IEEE float, Lancer's
//! SIMD off: `build.rs`). The shim's encoder (`src/shim.c`), the kernels the Rust port is
//! differentially tested against, and on x86-64 the Lancer copy (`lancer`) as `vorbis_rs`
//! builds it. Signatures mirror the C; handles are opaque.
#![allow(unsafe_code, missing_docs)]

use std::ffi::{c_int, c_long, c_void};

unsafe extern "C" {
    pub fn aotuv_enc_new(
        ch: c_int,
        rate: c_long,
        managed: c_int,
        q: f32,
        min: c_long,
        bitrate: c_long,
        max: c_long,
    ) -> *mut c_void;
    pub fn aotuv_enc_write(e: *mut c_void, pcm: *const *const f32, n: c_long);
    pub fn aotuv_enc_out(e: *mut c_void, len: *mut c_long) -> *const u8;
    pub fn aotuv_enc_free(e: *mut c_void);
    pub fn aotuv_enc_packets(e: *mut c_void, len: *mut c_long) -> *const u8;
    pub fn aotuv_enc_blobs(e: *mut c_void, len: *mut c_long) -> *const u8;

    pub fn aotuv_headers(
        ch: c_int,
        rate: c_long,
        managed: c_int,
        q: f32,
        min: c_long,
        bitrate: c_long,
        max: c_long,
        out: *mut u8,
        cap: c_long,
        len: *mut c_long,
    ) -> c_int;

    pub fn aotuv_setup_dump(
        ch: c_int,
        rate: c_long,
        managed: c_int,
        q: f32,
        min: c_long,
        bitrate: c_long,
        max: c_long,
        out: *mut f64,
        cap: c_long,
    ) -> c_long;

    pub fn aotuv_blocks(
        ch: c_int,
        rate: c_long,
        managed: c_int,
        q: f32,
        min: c_long,
        bitrate: c_long,
        max: c_long,
        pcm: *const *const f32,
        n: c_long,
        chunk: *const c_long,
        nchunk: c_int,
        out: *mut f64,
        cap: c_long,
    ) -> c_long;

    pub fn aotuv_psy_look(
        ch: c_int,
        rate: c_long,
        managed: c_int,
        q: f32,
        min: c_long,
        bitrate: c_long,
        max: c_long,
        block: c_int,
        out: *mut f64,
        cap: c_long,
    ) -> c_long;

    pub fn aotuv_psy_new(
        ch: c_int,
        rate: c_long,
        managed: c_int,
        q: f32,
        min: c_long,
        bitrate: c_long,
        max: c_long,
        block: c_int,
    ) -> *mut c_void;
    pub fn aotuv_psy_free(h: *mut c_void);
    pub fn aotuv_psy_n(h: *mut c_void) -> c_int;
    pub fn aotuv_psy_noisemask(
        h: *mut c_void,
        ncl: f32,
        logmdct: *const f32,
        lastmdct: *const f32,
        epeak: *mut f32,
        npeak: *mut f32,
        logmask: *mut f32,
        poste: f32,
        block_mode: c_int,
    );
    pub fn aotuv_psy_tonemask(
        h: *mut c_void,
        logfft: *const f32,
        logmask: *mut f32,
        gmax: f32,
        lmax: f32,
    );
    pub fn aotuv_psy_offset_and_mix(
        h: *mut c_void,
        noise: *const f32,
        tone: *const f32,
        offset_select: c_int,
        bit_managed: c_int,
        logmask: *mut f32,
        mdct: *mut f32,
        logmdct: *mut f32,
        lastmdct: *mut f32,
        tempmdct: *mut f32,
        low_compand: f32,
        npeak: *mut f32,
        end_block: c_int,
        block_mode: c_int,
        nw: c_int,
        lw_block_mode: c_int,
        lw_no: c_int,
        impadnum: c_int,
    );
    pub fn aotuv_psy_loudnoise(
        h: *mut c_void,
        ncl: f32,
        logmdct: *const f32,
        block_mode: c_int,
        lw_block_mode: c_int,
    ) -> f32;
    pub fn aotuv_postnoise(pcm: *mut f32, nn: c_int, mode: c_int, lw_mode: c_int) -> f32;

    pub fn aotuv_ogg_new(serialno: c_int) -> *mut c_void;
    pub fn aotuv_ogg_packetin(
        os: *mut c_void,
        data: *const u8,
        bytes: c_long,
        eos: c_int,
        granulepos: i64,
    );
    pub fn aotuv_ogg_page(os: *mut c_void, flush: c_int, out: *mut u8, cap: c_long) -> c_long;
    pub fn aotuv_ogg_free(os: *mut c_void);

    pub fn aotuv_mdct_new(n: c_int) -> *mut c_void;
    pub fn aotuv_mdct_forward(m: *mut c_void, inp: *mut f32, out: *mut f32);
    pub fn aotuv_mdct_trig(m: *mut c_void) -> *const f32;
    pub fn aotuv_mdct_bitrev(m: *mut c_void) -> *const c_int;
    pub fn aotuv_mdct_free(m: *mut c_void);

    pub fn aotuv_drft_new(n: c_int) -> *mut c_void;
    pub fn aotuv_drft_forward(l: *mut c_void, data: *mut f32);
    pub fn aotuv_drft_trig(l: *mut c_void) -> *const f32;
    pub fn aotuv_drft_split(l: *mut c_void) -> *const c_int;
    pub fn aotuv_drft_free(l: *mut c_void);

    // window.c
    pub fn _vorbis_window_get(n: c_int) -> *const f32;
    pub fn _vorbis_apply_window(
        d: *mut f32,
        winno: *mut c_int,
        blocksizes: *mut c_long,
        lw: c_int,
        w: c_int,
        nw: c_int,
    );
    // lpc.c
    pub fn vorbis_lpc_from_data(data: *mut f32, lpci: *mut f32, n: c_int, m: c_int) -> f32;
    pub fn vorbis_lpc_predict(
        coeff: *mut f32,
        prime: *mut f32,
        m: c_int,
        data: *mut f32,
        n: c_long,
    );
}

/// The Vorbis encoder side of one copy (the strict oracle or Lancer).
#[derive(Clone, Copy, Debug)]
pub struct Api {
    pub new: unsafe extern "C" fn(c_int, c_long, c_int, f32, c_long, c_long, c_long) -> *mut c_void,
    pub write: unsafe extern "C" fn(*mut c_void, *const *const f32, c_long),
    pub out: unsafe extern "C" fn(*mut c_void, *mut c_long) -> *const u8,
    pub free: unsafe extern "C" fn(*mut c_void),
}

/// The strict oracle.
pub const STRICT: Api = Api {
    new: aotuv_enc_new,
    write: aotuv_enc_write,
    out: aotuv_enc_out,
    free: aotuv_enc_free,
};

/// aoTuV-Lancer as `vorbis_rs` builds it (x86-64 with GCC only; `None` elsewhere).
pub mod lancer {
    #[cfg(aotuv_lancer)]
    mod ffi {
        use std::ffi::{c_int, c_long, c_void};
        unsafe extern "C" {
            pub fn lancer_enc_new(
                ch: c_int,
                rate: c_long,
                managed: c_int,
                q: f32,
                min: c_long,
                bitrate: c_long,
                max: c_long,
            ) -> *mut c_void;
            pub fn lancer_enc_write(e: *mut c_void, pcm: *const *const f32, n: c_long);
            pub fn lancer_enc_out(e: *mut c_void, len: *mut c_long) -> *const u8;
            pub fn lancer_enc_free(e: *mut c_void);
        }
    }
    #[cfg(aotuv_lancer)]
    pub const API: Option<super::Api> = Some(super::Api {
        new: ffi::lancer_enc_new,
        write: ffi::lancer_enc_write,
        out: ffi::lancer_enc_out,
        free: ffi::lancer_enc_free,
    });
    #[cfg(not(aotuv_lancer))]
    pub const API: Option<super::Api> = None;
}

/// xiph's libvorbis 1.3.7 as its `configure` builds it (`-O3 -ffast-math`; x86-64 with GCC only).
pub mod xiph {
    #[cfg(aotuv_xiph)]
    mod ffi {
        use std::ffi::{c_int, c_long, c_void};
        unsafe extern "C" {
            pub fn xiph_enc_new(
                ch: c_int,
                rate: c_long,
                managed: c_int,
                q: f32,
                min: c_long,
                bitrate: c_long,
                max: c_long,
            ) -> *mut c_void;
            pub fn xiph_enc_write(e: *mut c_void, pcm: *const *const f32, n: c_long);
            pub fn xiph_enc_out(e: *mut c_void, len: *mut c_long) -> *const u8;
            pub fn xiph_enc_free(e: *mut c_void);
        }
    }
    #[cfg(aotuv_xiph)]
    pub const API: Option<super::Api> = Some(super::Api {
        new: ffi::xiph_enc_new,
        write: ffi::xiph_enc_write,
        out: ffi::xiph_enc_out,
        free: ffi::xiph_enc_free,
    });
    #[cfg(not(aotuv_xiph))]
    pub const API: Option<super::Api> = None;
}

/// How the encoder is configured: VBR quality, or managed bitrate (bits/s, -1 = unset).
#[derive(Clone, Copy, Debug)]
pub enum Mode {
    Vbr(f32),
    Managed { min: i64, nominal: i64, max: i64 },
}

impl Mode {
    /// The shim's (managed, q, min, nominal, max).
    fn args(self) -> (c_int, f32, i64, i64, i64) {
        match self {
            Mode::Vbr(q) => (0, q, -1, -1, -1),
            Mode::Managed { min, nominal, max } => (1, 0.0, min, nominal, max),
        }
    }
}

/// An owned encoder of one copy; `write` returns the Ogg bytes produced by that call.
#[derive(Debug)]
pub struct Encoder {
    api: Api,
    h: *mut c_void,
    ch: usize,
    head: Vec<u8>,
}

impl Encoder {
    /// `None` when the C rejects the configuration (`vorbis_encode_init*` fails).
    pub fn new(api: Api, ch: usize, rate: i64, mode: Mode) -> Option<Self> {
        let (managed, q, min, nom, max) = mode.args();
        // SAFETY: plain values; a null handle is returned on failure.
        let h = unsafe {
            (api.new)(
                ch as c_int,
                rate as c_long,
                managed,
                q,
                min as c_long,
                nom as c_long,
                max as c_long,
            )
        };
        if h.is_null() {
            return None;
        }
        let mut e = Encoder {
            api,
            h,
            ch,
            head: Vec::new(),
        };
        e.head = e.take();
        Some(e)
    }

    /// The three header packets' pages.
    pub fn headers(&self) -> &[u8] {
        &self.head
    }

    fn take(&mut self) -> Vec<u8> {
        let mut len: c_long = 0;
        // SAFETY: the handle is live; the C returns its buffer and length.
        let p = unsafe { (self.api.out)(self.h, &mut len) };
        if len == 0 {
            return Vec::new();
        }
        // SAFETY: `p` holds `len` bytes until the next call on this handle.
        unsafe { std::slice::from_raw_parts(p, len as usize) }.to_vec()
    }

    /// Feeds `pcm[c][..n]` per channel (all the same length); an empty slice ends the stream.
    pub fn write(&mut self, pcm: &[&[f32]]) -> Vec<u8> {
        assert_eq!(pcm.len(), self.ch);
        let n = pcm[0].len();
        assert!(pcm.iter().all(|c| c.len() == n));
        let ptrs: Vec<*const f32> = pcm.iter().map(|c| c.as_ptr()).collect();
        // SAFETY: `ch` channel pointers of `n` samples each; the C copies them.
        unsafe { (self.api.write)(self.h, ptrs.as_ptr(), n as c_long) };
        self.take()
    }

    /// Ends the stream: the final pages.
    pub fn finish(&mut self) -> Vec<u8> {
        let empty: Vec<&[f32]> = vec![&[]; self.ch];
        self.write(&empty)
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: the handle came from `new` and is freed once.
        unsafe { (self.api.free)(self.h) }
    }
}

/// The C's three header packets for `mode`, or its `vorbis_encode_init*` error code.
pub fn headers(ch: usize, rate: i64, mode: Mode) -> Result<[Vec<u8>; 3], i32> {
    let (managed, q, min, nom, max) = mode.args();
    let mut buf = vec![0u8; 1 << 20];
    let mut len: [c_long; 3] = [0; 3];
    // SAFETY: `buf` holds `cap` bytes, `len` three longs.
    let r = unsafe {
        aotuv_headers(
            ch as c_int,
            rate as c_long,
            managed,
            q,
            min as c_long,
            nom as c_long,
            max as c_long,
            buf.as_mut_ptr(),
            buf.len() as c_long,
            len.as_mut_ptr(),
        )
    };
    if r != 0 {
        assert_ne!(r, -1, "header buffer too small");
        return Err(r);
    }
    let (a, b) = (len[0] as usize, len[1] as usize);
    Ok([
        buf[..a].to_vec(),
        buf[a..a + b].to_vec(),
        buf[a + b..a + b + len[2] as usize].to_vec(),
    ])
}

/// The C's setup beyond the headers, flattened to doubles (`aotuv_setup_dump` in `shim.c`),
/// or its `vorbis_encode_init*` error code.
pub fn setup_dump(ch: usize, rate: i64, mode: Mode) -> Result<Vec<f64>, i64> {
    let (managed, q, min, nom, max) = mode.args();
    let mut buf = vec![0f64; 4096];
    // SAFETY: `buf` holds `cap` doubles.
    let n = unsafe {
        aotuv_setup_dump(
            ch as c_int,
            rate as c_long,
            managed,
            q,
            min as c_long,
            nom as c_long,
            max as c_long,
            buf.as_mut_ptr(),
            buf.len() as c_long,
        )
    };
    assert_ne!(n, -1, "setup dump buffer too small");
    if n < 0 {
        return Err(n);
    }
    buf.truncate(n as usize);
    Ok(buf)
}

/// The C's blocking alone (`aotuv_blocks` in `shim.c`): per block lW W nW blocktype sequence
/// granulepos pcmend eofflag ampmax, its pcm, then the envelope's state, as doubles; or the
/// init error code.
pub fn blocks(rate: i64, mode: Mode, pcm: &[Vec<f32>], chunks: &[i64]) -> Result<Vec<f64>, i64> {
    let (managed, q, min, nom, max) = mode.args();
    let n = pcm[0].len();
    let ptrs: Vec<*const f32> = pcm.iter().map(|c| c.as_ptr()).collect();
    let chunk: Vec<c_long> = chunks.iter().map(|&c| c as c_long).collect();
    let mut cap = 1 << 20;
    loop {
        let mut buf = vec![0f64; cap];
        // SAFETY: `pcm.len()` channels of `n` samples, `chunk` sizes, `buf` of `cap` doubles.
        let r = unsafe {
            aotuv_blocks(
                pcm.len() as c_int,
                rate as c_long,
                managed,
                q,
                min as c_long,
                nom as c_long,
                max as c_long,
                ptrs.as_ptr(),
                n as c_long,
                chunk.as_ptr(),
                chunk.len() as c_int,
                buf.as_mut_ptr(),
                cap as c_long,
            )
        };
        if r == -1 {
            cap *= 4;
            continue;
        }
        if r < 0 {
            return Err(r);
        }
        buf.truncate(r as usize);
        return Ok(buf);
    }
}

/// The C's psy look for psy block `block` (`aotuv_psy_look` in `shim.c`), as doubles; `Err`:
/// the init error, or -2 when the setup has no such block.
pub fn psy_look(ch: usize, rate: i64, mode: Mode, block: usize) -> Result<Vec<f64>, i64> {
    let (managed, q, min, nom, max) = mode.args();
    let mut buf = vec![0f64; 1 << 16];
    // SAFETY: `buf` holds `cap` doubles.
    let r = unsafe {
        aotuv_psy_look(
            ch as c_int,
            rate as c_long,
            managed,
            q,
            min as c_long,
            nom as c_long,
            max as c_long,
            block as c_int,
            buf.as_mut_ptr(),
            buf.len() as c_long,
        )
    };
    assert_ne!(r, -1, "psy look buffer too small");
    if r < 0 {
        return Err(r);
    }
    buf.truncate(r as usize);
    Ok(buf)
}

/// psy.c's per-block functions on one configuration's psy block (`aotuv_psy_*` in `shim.c`).
#[derive(Debug)]
pub struct Psy(*mut c_void);

impl Psy {
    /// `None`: no such configuration or psy block.
    pub fn new(ch: usize, rate: i64, mode: Mode, block: usize) -> Option<Self> {
        let (managed, q, min, nom, max) = mode.args();
        // SAFETY: plain values; null on failure.
        let h = unsafe {
            aotuv_psy_new(
                ch as c_int,
                rate as c_long,
                managed,
                q,
                min as c_long,
                nom as c_long,
                max as c_long,
                block as c_int,
            )
        };
        (!h.is_null()).then_some(Psy(h))
    }
    pub fn n(&self) -> usize {
        // SAFETY: a live handle.
        unsafe { aotuv_psy_n(self.0) as usize }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn noisemask(
        &self,
        ncl: f32,
        logmdct: &[f32],
        lastmdct: &[f32],
        epeak: &mut [f32],
        npeak: &mut [f32],
        logmask: &mut [f32],
        poste: f32,
        block_mode: i32,
    ) {
        let n = self.n();
        assert!(
            logmdct.len() >= n && lastmdct.len() >= n && epeak.len() >= n && logmask.len() >= n
        );
        // SAFETY: n-sized arrays (npeak: n/partition, the caller's).
        unsafe {
            aotuv_psy_noisemask(
                self.0,
                ncl,
                logmdct.as_ptr(),
                lastmdct.as_ptr(),
                epeak.as_mut_ptr(),
                npeak.as_mut_ptr(),
                logmask.as_mut_ptr(),
                poste,
                block_mode,
            )
        }
    }
    pub fn tonemask(&self, logfft: &[f32], logmask: &mut [f32], gmax: f32, lmax: f32) {
        assert!(logfft.len() >= self.n() && logmask.len() >= self.n());
        // SAFETY: n-sized arrays.
        unsafe { aotuv_psy_tonemask(self.0, logfft.as_ptr(), logmask.as_mut_ptr(), gmax, lmax) }
    }
    /// `lastmdct`: 2048 floats, `tempmdct`: 256 (the C's per-channel buffers).
    #[allow(clippy::too_many_arguments)]
    pub fn offset_and_mix(
        &self,
        noise: &[f32],
        tone: &[f32],
        offset_select: i32,
        bit_managed: bool,
        logmask: &mut [f32],
        mdct: &mut [f32],
        logmdct: &mut [f32],
        lastmdct: &mut [f32],
        tempmdct: &mut [f32],
        low_compand: f32,
        npeak: &mut [f32],
        end_block: i32,
        block_mode: i32,
        nw: i32,
        lw_block_mode: i32,
        lw_no: i32,
        impadnum: i32,
    ) {
        assert!(lastmdct.len() >= 2048 && tempmdct.len() >= 256);
        // SAFETY: n-sized arrays, the buffers' sizes checked.
        unsafe {
            aotuv_psy_offset_and_mix(
                self.0,
                noise.as_ptr(),
                tone.as_ptr(),
                offset_select,
                bit_managed as c_int,
                logmask.as_mut_ptr(),
                mdct.as_mut_ptr(),
                logmdct.as_mut_ptr(),
                lastmdct.as_mut_ptr(),
                tempmdct.as_mut_ptr(),
                low_compand,
                npeak.as_mut_ptr(),
                end_block,
                block_mode,
                nw,
                lw_block_mode,
                lw_no,
                impadnum,
            )
        }
    }
    pub fn loudnoise(&self, ncl: f32, logmdct: &[f32], block_mode: i32, lw_block_mode: i32) -> f32 {
        // SAFETY: an n-sized array.
        unsafe { aotuv_psy_loudnoise(self.0, ncl, logmdct.as_ptr(), block_mode, lw_block_mode) }
    }
}

impl Drop for Psy {
    fn drop(&mut self) {
        // SAFETY: freed once.
        unsafe { aotuv_psy_free(self.0) }
    }
}

/// `_postnoise_detection`.
pub fn postnoise(pcm: &[f32], mode: i32, lw_mode: i32) -> f32 {
    let mut v = pcm.to_vec();
    // SAFETY: the C reads `nn` floats.
    unsafe { aotuv_postnoise(v.as_mut_ptr(), v.len() as c_int, mode, lw_mode) }
}

/// libogg's stream writer alone.
#[derive(Debug)]
pub struct OggStream(*mut c_void);

impl OggStream {
    pub fn new(serialno: i32) -> Self {
        // SAFETY: allocates and initialises a stream.
        OggStream(unsafe { aotuv_ogg_new(serialno) })
    }
    pub fn packetin(&mut self, data: &[u8], eos: bool, granulepos: i64) {
        // SAFETY: libogg copies `data`.
        unsafe {
            aotuv_ogg_packetin(
                self.0,
                data.as_ptr(),
                data.len() as c_long,
                eos as c_int,
                granulepos,
            )
        }
    }
    /// The next page (`flush`: `ogg_stream_flush`, else `ogg_stream_pageout`).
    pub fn page(&mut self, flush: bool) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; 27 + 255 + 255 * 255];
        // SAFETY: `buf` holds the largest page.
        let n = unsafe {
            aotuv_ogg_page(
                self.0,
                flush as c_int,
                buf.as_mut_ptr(),
                buf.len() as c_long,
            )
        };
        assert!(n >= 0);
        (n > 0).then(|| {
            buf.truncate(n as usize);
            buf
        })
    }
}

impl Drop for OggStream {
    fn drop(&mut self) {
        // SAFETY: freed once.
        unsafe { aotuv_ogg_free(self.0) }
    }
}

/// `mdct_lookup` of size `n` (a power of two ≥ 64).
#[derive(Debug)]
pub struct Mdct(*mut c_void, usize);

impl Mdct {
    pub fn new(n: usize) -> Self {
        // SAFETY: mdct_init allocates its tables.
        Mdct(unsafe { aotuv_mdct_new(n as c_int) }, n)
    }
    /// `n` inputs → `n/2` outputs.
    pub fn forward(&self, inp: &[f32], out: &mut [f32]) {
        assert!(inp.len() >= self.1 && out.len() >= self.1 / 2);
        let mut i = inp[..self.1].to_vec();
        // SAFETY: sizes checked; the C reads n and writes n/2 floats.
        unsafe { aotuv_mdct_forward(self.0, i.as_mut_ptr(), out.as_mut_ptr()) }
    }
    /// `trig` (n + n/4 floats) and `bitrev` (n/4 ints).
    pub fn tables(&self) -> (&[f32], &[i32]) {
        // SAFETY: mdct_init allocated these sizes.
        unsafe {
            (
                std::slice::from_raw_parts(aotuv_mdct_trig(self.0), self.1 + self.1 / 4),
                std::slice::from_raw_parts(aotuv_mdct_bitrev(self.0), self.1 / 4),
            )
        }
    }
}

impl Drop for Mdct {
    fn drop(&mut self) {
        // SAFETY: freed once.
        unsafe { aotuv_mdct_free(self.0) }
    }
}

/// `drft_lookup` of size `n`.
#[derive(Debug)]
pub struct Drft(*mut c_void, usize);

impl Drft {
    pub fn new(n: usize) -> Self {
        // SAFETY: drft_init allocates its caches.
        Drft(unsafe { aotuv_drft_new(n as c_int) }, n)
    }
    /// `&mut self`: `drft_forward` uses `trigcache` as scratch, which `tables` lends out.
    pub fn forward(&mut self, data: &mut [f32]) {
        assert!(data.len() >= self.1);
        // SAFETY: n floats in place.
        unsafe { aotuv_drft_forward(self.0, data.as_mut_ptr()) }
    }
    /// `trigcache` (3n floats) and `splitcache` (32 ints).
    pub fn tables(&self) -> (&[f32], &[i32]) {
        // SAFETY: drft_init allocated these sizes.
        unsafe {
            (
                std::slice::from_raw_parts(aotuv_drft_trig(self.0), 3 * self.1),
                std::slice::from_raw_parts(aotuv_drft_split(self.0), 32),
            )
        }
    }
}

impl Drop for Drft {
    fn drop(&mut self) {
        // SAFETY: freed once.
        unsafe { aotuv_drft_free(self.0) }
    }
}

/// One packet as the C encoder hands it to libogg.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub data: Vec<u8>,
    pub granulepos: i64,
    pub eos: bool,
}

/// What the strict encoder made of a stream: every packet (headers first), the Ogg stream
/// libogg made of them, and per audio block its 15 packet blobs as `mapping0_forward` left
/// them (before bitrate management picks, pads or truncates one; VBR fills only blob 7).
#[derive(Clone, Debug)]
pub struct Trace {
    pub packets: Vec<Packet>,
    pub stream: Vec<u8>,
    pub blobs: Vec<Vec<Vec<u8>>>,
}

/// A log the shim keeps (`enc_packets`, `enc_blobs`), taken.
fn take_log(
    e: &Encoder,
    f: unsafe extern "C" fn(*mut c_void, *mut c_long) -> *const u8,
) -> Vec<u8> {
    let mut len: c_long = 0;
    // SAFETY: the handle is live; the C returns its log and length.
    let p = unsafe { f(e.h, &mut len) };
    if len == 0 {
        return Vec::new();
    }
    // SAFETY: `p` holds `len` bytes until the next call on this handle.
    unsafe { std::slice::from_raw_parts(p, len as usize) }.to_vec()
}

/// The strict encoder over `pcm` (planar) as `encode` drives it (1024-sample writes).
pub fn encode_trace(rate: i64, mode: Mode, pcm: &[Vec<f32>]) -> Option<Trace> {
    let mut e = Encoder::new(STRICT, pcm.len(), rate, mode)?;
    let mut stream = e.headers().to_vec();
    let n = pcm[0].len();
    for k in (0..n).step_by(1024) {
        let m = (n - k).min(1024);
        let chunk: Vec<&[f32]> = pcm.iter().map(|c| &c[k..k + m]).collect();
        stream.extend(e.write(&chunk));
    }
    stream.extend(e.finish());
    let log = take_log(&e, aotuv_enc_packets);
    let mut packets = Vec::new();
    let mut i = 0;
    while i < log.len() {
        let b = i64::from_le_bytes(log[i..i + 8].try_into().unwrap()) as usize;
        let granulepos = i64::from_le_bytes(log[i + 8..i + 16].try_into().unwrap());
        let eos = log[i + 16] != 0;
        packets.push(Packet {
            data: log[i + 17..i + 17 + b].to_vec(),
            granulepos,
            eos,
        });
        i += 17 + b;
    }
    let log = take_log(&e, aotuv_enc_blobs);
    let mut blobs = Vec::new();
    let mut i = 0;
    while i < log.len() {
        let mut block = Vec::with_capacity(15);
        for _ in 0..15 {
            let b = i64::from_le_bytes(log[i..i + 8].try_into().unwrap()) as usize;
            block.push(log[i + 8..i + 8 + b].to_vec());
            i += 8 + b;
        }
        blobs.push(block);
    }
    Some(Trace {
        packets,
        stream,
        blobs,
    })
}

/// [`encode_trace`]'s packets and stream.
pub fn encode_packets(rate: i64, mode: Mode, pcm: &[Vec<f32>]) -> Option<(Vec<Packet>, Vec<u8>)> {
    encode_trace(rate, mode, pcm).map(|t| (t.packets, t.stream))
}

/// The ETSI LC3plus item `thetest<khz>.wav` (`just lc3plus`; oracle input only): (rate, samples).
pub fn thetest(khz: u32) -> (u32, Vec<i16>) {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache/lc3plus/wav");
    let path = dir.join(format!("thetest{khz}.wav"));
    let d = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("{}: {e} (run `just lc3plus`)", path.display()));
    let u32_at = |i: usize| u32::from_le_bytes(d[i..i + 4].try_into().unwrap());
    let u16_at = |i: usize| u16::from_le_bytes(d[i..i + 2].try_into().unwrap());
    assert_eq!(&d[..4], b"RIFF");
    let (mut i, mut rate) = (12, 0);
    while i + 8 <= d.len() {
        let size = u32_at(i + 4) as usize;
        match &d[i..i + 4] {
            b"fmt " => {
                assert_eq!((u16_at(i + 8), u16_at(i + 10), u16_at(i + 22)), (1, 1, 16));
                rate = u32_at(i + 12);
            }
            b"data" => {
                let pcm = d[i + 8..i + 8 + size]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|&b| i16::from_le_bytes(b))
                    .collect();
                return (rate, pcm);
            }
            _ => {}
        }
        i += 8 + size + (size & 1);
    }
    panic!("{}: no data chunk", path.display());
}

/// The input of the measurements made before the port: `s / 32768.f` per channel; channel 2 is
/// channel 1 delayed 331 samples × 0.75 (correlated, not identical: exercises coupling).
pub fn spike_pcm(pcm: &[i16], ch: usize) -> Vec<Vec<f32>> {
    let left: Vec<f32> = pcm.iter().map(|&s| s as f32 / 32768.0).collect();
    let mut out = vec![left];
    if ch == 2 {
        out.push(
            (0..pcm.len())
                .map(|j| {
                    if j < 331 {
                        0.0
                    } else {
                        pcm[j - 331] as f32 / 32768.0 * 0.75
                    }
                })
                .collect(),
        );
    }
    out
}

/// The whole Ogg stream of `pcm` (planar) as the spike harness writes it: 1024-sample writes,
/// then the end of stream.
pub fn encode(api: Api, rate: i64, mode: Mode, pcm: &[Vec<f32>]) -> Option<Vec<u8>> {
    let mut e = Encoder::new(api, pcm.len(), rate, mode)?;
    let mut s = e.headers().to_vec();
    let n = pcm[0].len();
    for k in (0..n).step_by(1024) {
        let m = (n - k).min(1024);
        let chunk: Vec<&[f32]> = pcm.iter().map(|c| &c[k..k + m]).collect();
        s.extend(e.write(&chunk));
    }
    s.extend(e.finish());
    Some(s)
}
