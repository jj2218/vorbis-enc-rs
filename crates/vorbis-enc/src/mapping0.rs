//! The encoder's per-block analysis (`mapping0.c` `mapping0_forward`, with the backend state
//! `block.c`'s `_vds_shared_init`/`vorbis_analysis_init` set up): per channel the window, MDCT
//! and FFT, the psy masks, the floor fits (one, or three plus interpolations per packet blob
//! when the bitrate is managed); per blob the floors, coupling/quantisation and residues into
//! that blob's packet. In the C's arithmetic (`todB(x) + .345` in double).
#![allow(clippy::needless_range_loop)] // index loops mirror the C code 1:1

use crate::bitpack::{OggPack, ilog};
use crate::block::Block;
use crate::codebook::Codebook;
use crate::floor1::{Floor1Look, floor1_encode, floor1_fit, floor1_interpolate_fit};
use crate::mdct::Mdct;
use crate::psy::{PsyLook, postnoise_detection};
use crate::res0::{ResidueLook, res1_encode, res2_encode};
use crate::scales::todb;
use crate::setup::CodecSetup;
use crate::smallft::Drft;
use crate::templates::PACKETBLOBS;
use crate::window::apply_window;

/// `private_state`'s analysis side (the looks, and aoTuV's history across blocks).
#[derive(Clone, Debug)]
pub(crate) struct Backend {
    pub modebits: u32,
    pub transform: [Mdct; 2],
    pub fft_look: [Drft; 2],
    pub window: [usize; 2],
    pub psy: Vec<PsyLook>,
    pub flr: Vec<Floor1Look>,
    pub residue: Vec<ResidueLook>,
    pub books: Vec<Codebook>,
    /// `mblock`: per channel 2048 lines of the previous MDCT (dB)
    pub mblock: Vec<f32>,
    /// `tblock`: per channel 256 lines
    pub tblock: Vec<f32>,
    pub lownoise_compand_level: Vec<f32>,
    pub lw_block_mode: i32,
    pub lw_no: i32,
    pub impadnum: i32,
}

impl Backend {
    /// The analysis looks of `_vds_shared_init(encp = 1)` and `vorbis_analysis_init`.
    pub(crate) fn new(ci: &CodecSetup) -> Self {
        let ch = ci.channels as usize;
        let bs = [ci.blocksizes[0] as usize, ci.blocksizes[1] as usize];
        Backend {
            modebits: ilog(ci.modes.len() as u32 - 1),
            transform: [Mdct::new(bs[0]), Mdct::new(bs[1])],
            fft_look: [Drft::new(bs[0]), Drft::new(bs[1])],
            window: [
                ilog(bs[0] as u32) as usize - 7,
                ilog(bs[1] as u32) as usize - 7,
            ],
            psy: ci
                .psy_param
                .iter()
                .map(|p| PsyLook::new(p, &ci.psy_g_param, bs[p.blockflag as usize] / 2, ci.rate))
                .collect(),
            flr: ci.floors.iter().map(Floor1Look::new).collect(),
            residue: ci
                .residues
                .iter()
                .map(|(_, r)| ResidueLook::new(r))
                .collect(),
            books: ci.books.iter().map(|&b| Codebook::new(b)).collect(),
            mblock: vec![0.0; 2048 * ch],
            tblock: vec![0.0; 256 * ch],
            lownoise_compand_level: vec![0.0; ch],
            lw_block_mode: 0,
            lw_no: 0,
            impadnum: 0,
        }
    }

    /// `mapping0_forward`: the block's packet per blob (`vbi->packetblob`; VBR fills only
    /// `PACKETBLOBS/2`) and the block's raised peak (`vbi->ampmax`). `vb.pcm` is consumed as
    /// the C's scratch.
    pub(crate) fn mapping0_forward(
        &mut self,
        ci: &CodecSetup,
        vb: &mut Block,
    ) -> ([OggPack; PACKETBLOBS], f32) {
        let channels = ci.channels as usize;
        let n = vb.pcmend;
        let blocktype = vb.blocktype;
        let modenumber = vb.w;
        let info = &ci.maps[ci.modes[modenumber].mapping as usize];
        let vif_n = ci.floors[vb.w].n;
        let psy_index = blocktype as usize + if vb.w != 0 { 2 } else { 0 };
        let managed = ci.is_managed();
        let mut global_ampmax = vb.ampmax;
        let mut local_ampmax = vec![0f32; channels];
        let mut poste = vec![0f32; channels];
        let mut gmdct = vec![vec![0f32; n / 2]; channels];
        let mut epeak = vec![vec![0f32; n / 2]; channels];
        let mut iwork = vec![vec![0i32; n / 2]; channels];
        let mut floor_posts: Vec<[Option<Vec<i32>>; PACKETBLOBS]> = Vec::with_capacity(channels);
        let block_mode = blocktype | ((modenumber as i32) << 1);
        let pvi = &self.psy[psy_index].vi;
        let partition = pvi.partition();
        let mut npeak = vec![vec![0f32; n / 2 / partition]; channels];
        let mut lowpass_residue = ci.block_lowpassr[if modenumber != 0 { 1 } else { 0 }];
        if lowpass_residue % pvi.normal_partition != 0 {
            lowpass_residue = (lowpass_residue / pvi.normal_partition + 1) * pvi.normal_partition;
        }
        let bs = [ci.blocksizes[0] as usize, ci.blocksizes[1] as usize];

        for i in 0..channels {
            let scale = 4.0f32 / n as f32;
            let scale_db = (todb(scale) as f64 + 0.345) as f32;
            let pcm = &mut vb.pcm[i];
            poste[i] = postnoise_detection(pcm, n, block_mode, self.lw_block_mode);
            apply_window(pcm, &self.window, &bs, vb.lw, vb.w, vb.nw);
            self.transform[vb.w].forward(pcm, &mut gmdct[i]);
            self.fft_look[vb.w].forward(pcm);
            // logfft overwrites pcm
            pcm[0] = ((scale_db + todb(pcm[0])) as f64 + 0.345) as f32;
            local_ampmax[i] = pcm[0];
            let mut j = 1;
            while j < n - 1 {
                let temp = pcm[j] * pcm[j] + pcm[j + 1] * pcm[j + 1];
                let temp = ((scale_db + 0.5f32 * todb(temp)) as f64 + 0.345) as f32;
                pcm[(j + 1) >> 1] = temp;
                if temp > local_ampmax[i] {
                    local_ampmax[i] = temp;
                }
                j += 2;
            }
            if local_ampmax[i] > 0.0 {
                local_ampmax[i] = 0.0;
            }
            if local_ampmax[i] > global_ampmax {
                global_ampmax = local_ampmax[i];
            }
        }

        let mut noise = vec![0f32; n / 2];
        let mut tone = vec![0f32; n / 2];
        for i in 0..channels {
            let submap = info.chmuxlist[i] as usize;
            let flr = &self.flr[info.floorsubmap[submap] as usize];
            let psy_look = &self.psy[psy_index];
            let mdct = &mut gmdct[i];
            let (logfft, logmdct) = vb.pcm[i].split_at_mut(n / 2);
            let lastmdct = &mut self.mblock[i * 2048..(i + 1) * 2048];
            let tempmdct = &mut self.tblock[i * 256..(i + 1) * 256];
            let mut posts: [Option<Vec<i32>>; PACKETBLOBS] = Default::default();
            for j in 0..n / 2 {
                logmdct[j] = (todb(mdct[j]) as f64 + 0.345) as f32;
            }
            let lowcomp = psy_look.lb_loudnoise_fix(
                self.lownoise_compand_level[i],
                logmdct,
                block_mode,
                self.lw_block_mode,
            );
            self.lownoise_compand_level[i] = lowcomp;
            psy_look.noisemask(
                lowcomp,
                logmdct,
                lastmdct,
                &mut epeak[i],
                &mut npeak[i],
                &mut noise,
                poste[i],
                block_mode,
            );
            psy_look.tonemask(logfft, &mut tone, global_ampmax, local_ampmax[i]);
            // logmask overwrites logfft
            let logmask = logfft;
            let mut mix = |sel: usize, logmask: &mut [f32], mdct: &mut [f32], npeak: &mut [f32]| {
                psy_look.offset_and_mix(
                    &noise,
                    &tone,
                    sel,
                    managed,
                    logmask,
                    mdct,
                    logmdct,
                    lastmdct,
                    tempmdct,
                    lowcomp,
                    npeak,
                    vif_n,
                    block_mode,
                    vb.nw,
                    self.lw_block_mode,
                    self.lw_no,
                    self.impadnum,
                )
            };
            mix(1, logmask, mdct, &mut npeak[i]);
            posts[PACKETBLOBS / 2] = floor1_fit(flr, logmdct, logmask);
            if managed && posts[PACKETBLOBS / 2].is_some() {
                mix(2, logmask, mdct, &mut npeak[i]);
                posts[PACKETBLOBS - 1] = floor1_fit(flr, logmdct, logmask);
                mix(0, logmask, mdct, &mut npeak[i]);
                posts[0] = floor1_fit(flr, logmdct, logmask);
                for k in 1..PACKETBLOBS / 2 {
                    posts[k] = floor1_interpolate_fit(
                        flr,
                        posts[0].as_deref(),
                        posts[PACKETBLOBS / 2].as_deref(),
                        (k * 65536 / (PACKETBLOBS / 2)) as i32,
                    );
                }
                for k in PACKETBLOBS / 2 + 1..PACKETBLOBS - 1 {
                    posts[k] = floor1_interpolate_fit(
                        flr,
                        posts[PACKETBLOBS / 2].as_deref(),
                        posts[PACKETBLOBS - 1].as_deref(),
                        ((k - PACKETBLOBS / 2) * 65536 / (PACKETBLOBS / 2)) as i32,
                    );
                }
            }
            floor_posts.push(posts);
        }

        let mut blobs: [OggPack; PACKETBLOBS] = Default::default();
        let (k0, k1) = if managed {
            (0, PACKETBLOBS - 1)
        } else {
            (PACKETBLOBS / 2, PACKETBLOBS / 2)
        };
        let mut nonzero = vec![false; channels];
        for k in k0..=k1 {
            let opb = &mut blobs[k];
            opb.write(0, 1);
            opb.write(modenumber as i64, self.modebits);
            if vb.w != 0 {
                opb.write(vb.lw as i64, 1);
                opb.write(vb.nw as i64, 1);
            }
            for i in 0..channels {
                let submap = info.chmuxlist[i] as usize;
                nonzero[i] = floor1_encode(
                    opb,
                    &self.flr[info.floorsubmap[submap] as usize],
                    &self.books,
                    floor_posts[i][k].as_deref_mut(),
                    &mut iwork[i],
                    n / 2,
                );
            }
            self.psy[psy_index].couple_quantize_normalize(
                k,
                &ci.psy_g_param,
                info,
                &gmdct,
                &epeak,
                &mut npeak,
                &mut iwork,
                &mut nonzero,
                ci.psy_g_param.sliding_lowpass[vb.w][k],
                channels,
                lowpass_residue,
            );
            for i in 0..info.submaps as usize {
                let resnum = info.residuesubmap[i] as usize;
                let look = &self.residue[resnum];
                let mut zerobundle = Vec::new();
                let mut bundle: Vec<&mut [i32]> = Vec::new();
                for (j, w) in iwork.iter_mut().enumerate() {
                    if info.chmuxlist[j] as usize == i {
                        zerobundle.push(nonzero[j]);
                        bundle.push(&mut w[..]);
                    }
                }
                match ci.residues[resnum].0 {
                    1 => res1_encode(opb, look, &self.books, &mut bundle, &zerobundle),
                    2 => res2_encode(opb, look, &self.books, &mut bundle, &zerobundle),
                    t => unreachable!("residue type {t} has no encoder"),
                }
            }
            // the history advances inside the blob loop, as in aoTuV: a managed block (15
            // blobs) moves it 15 times, a VBR block once; the next block's offsets read it
            if block_mode >= 2 {
                self.impadnum = 0;
            }
            if self.lw_block_mode == 0 && block_mode == 1 {
                self.impadnum = 1;
            } else if self.impadnum != 0 && self.impadnum < 8 {
                self.impadnum += 1;
            }
            if self.lw_block_mode == block_mode {
                self.lw_no += 1;
            } else {
                self.lw_no = 1;
            }
            self.lw_block_mode = block_mode;
        }
        (blobs, global_ampmax)
    }
}
