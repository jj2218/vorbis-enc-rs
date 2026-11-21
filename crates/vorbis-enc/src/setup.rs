//! The encoder's setup (`vorbisenc.c`): picks the template for the channel count and rate,
//! places the quality (or bitrate) between its steps, and interpolates the psychoacoustic
//! parameters, floors, residues and books into a codec setup. In the C's arithmetic: `double`
//! interpolation weights, stored into the C's `float`/`int`/`long` fields where it stores them.
#![allow(clippy::needless_range_loop)] // index loops mirror the C code 1:1

use crate::setup_tables::{_MODE_TEMPLATE, _PSY_INFO_TEMPLATE, SETUP_LIST};
use crate::templates::*;

/// Why a configuration is rejected (`OV_EINVAL`, `OV_EIMPL`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SetupError {
    /// `OV_EINVAL`: an argument out of range.
    Invalid,
    /// `OV_EIMPL`: no template covers the channel count and rate (or bitrate).
    Unsupported,
}

/// `highlevel_byblocktype`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ByBlock {
    pub tone_mask_setting: f64,
    pub tone_peaklimit_setting: f64,
    pub noise_bias_setting: f64,
    pub noise_compand_setting: f64,
}

/// `highlevel_encode_setup`.
#[derive(Clone, Debug)]
pub(crate) struct Highlevel {
    pub setup: &'static SetupTemplate,
    pub base_setting: f64,
    pub impulse_noisetune: f64,
    pub req: f32,
    pub managed: bool,
    pub bitrate_min: i64,
    pub bitrate_av: i64,
    pub bitrate_av_damp: f64,
    pub bitrate_max: i64,
    pub bitrate_reservoir: i64,
    pub bitrate_reservoir_bias: f64,
    pub impulse_block_p: bool,
    pub noise_normalize_p: bool,
    pub stereo_point_setting: f64,
    pub lowpass_khz: f64,
    pub ath_floating_db: f64,
    pub ath_absolute_db: f64,
    pub amplitude_track_dbpersec: f64,
    pub trigger_setting: f64,
    pub pre_amplitude: f32,
    pub block: [ByBlock; 4],
}

/// `bitrate_manager_info`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BitrateInfo {
    pub avg_rate: i64,
    pub min_rate: i64,
    pub max_rate: i64,
    pub reservoir_bits: i64,
    pub reservoir_bias: f64,
    pub slew_damp: f64,
}

/// `vorbis_info` with its `codec_setup_info`: what the encoder runs on and the headers carry.
#[derive(Clone, Debug)]
pub(crate) struct CodecSetup {
    pub channels: i32,
    pub rate: i64,
    pub bitrate_upper: i64,
    pub bitrate_nominal: i64,
    pub bitrate_lower: i64,
    pub bitrate_window: i64,
    pub blocksizes: [i64; 2],
    pub modes: Vec<InfoMode>,
    pub maps: Vec<InfoMapping0>,
    pub floors: Vec<InfoFloor1>,
    /// (`residue_type`, `residue_param`)
    pub residues: Vec<(i32, InfoResidue0)>,
    pub books: Vec<&'static StaticCodebook>,
    pub psy_param: Vec<InfoPsy>,
    pub psy_g_param: InfoPsyGlobal,
    pub bi: BitrateInfo,
    pub hi: Highlevel,
    pub block_lowpassr: [i32; 2],
}

/// `LFE_FRQLIMIT`.
const LFE_FRQLIMIT: f64 = 250.0;

impl Default for InfoPsyGlobal {
    fn default() -> Self {
        InfoPsyGlobal {
            eighth_octave_lines: 0,
            preecho_thresh: [0.0; VE_BANDS],
            postecho_thresh: [0.0; VE_BANDS],
            stretch_penalty: 0.0,
            preecho_minenergy: 0.0,
            ampmax_att_per_sec: 0.0,
            coupling_pkhz: [0; PACKETBLOBS],
            coupling_pointlimit: [[0; PACKETBLOBS]; 2],
            coupling_prepointamp: [0; PACKETBLOBS],
            coupling_postpointamp: [0; PACKETBLOBS],
            sliding_lowpass: [[0; PACKETBLOBS]; 2],
        }
    }
}

/// `get_setup_template`: the first template for `ch` and `srate` whose quality (or, per
/// channel, bitrate) range holds `req`, and where `req` lies in it (`base_setting`).
fn get_setup_template(
    ch: i64,
    srate: i64,
    mut req: f64,
    q_or_bitrate: bool,
) -> Option<(&'static SetupTemplate, f64)> {
    if q_or_bitrate {
        req /= ch as f64;
    }
    for s in SETUP_LIST.iter() {
        if s.coupling_restriction != -1 && s.coupling_restriction as i64 != ch {
            continue;
        }
        if srate < s.samplerate_min_restriction || srate > s.samplerate_max_restriction {
            continue;
        }
        let mappings = s.mappings as usize;
        let map = if q_or_bitrate {
            s.rate_mapping
        } else {
            s.quality_mapping
        };
        if req < map[0] || req > map[mappings] {
            continue;
        }
        let j = (0..mappings)
            .find(|&j| req >= map[j] && req < map[j + 1])
            .unwrap_or(mappings);
        let base = if j == mappings {
            j as f64 - 0.001
        } else {
            // float low/high/del, and `j+del` in float
            let low = map[j] as f32;
            let high = map[j + 1] as f32;
            let del = ((req - low as f64) / (high - low) as f64) as f32;
            (j as f32 + del) as f64
        };
        return Some((s, base));
    }
    None
}

impl CodecSetup {
    /// `vorbis_bitrate_managed`: a bitrate reservoir set (managed bitrate, not VBR).
    pub(crate) fn is_managed(&self) -> bool {
        self.bi.reservoir_bits > 0
    }

    /// `vorbis_encode_init_vbr`: quality −0.2…1.0.
    pub(crate) fn vbr(channels: i64, rate: i64, quality: f32) -> Result<Self, SetupError> {
        if rate <= 0 {
            return Err(SetupError::Invalid);
        }
        let mut q = (quality as f64 + 0.0000001) as f32;
        if q >= 1.0 {
            q = 0.9999_f64 as f32;
        }
        let (setup, base) =
            get_setup_template(channels, rate, q as f64, false).ok_or(SetupError::Unsupported)?;
        let mut ci = Self::setting(setup, base, channels, rate);
        ci.hi.req = q;
        ci.hi.managed = false;
        ci.setup_init()?;
        Ok(ci)
    }

    /// `vorbis_encode_init`: managed bitrate (bits/s; ≤ 0: unset).
    pub(crate) fn managed(
        channels: i64,
        rate: i64,
        max_bitrate: i64,
        nominal_bitrate: i64,
        min_bitrate: i64,
    ) -> Result<Self, SetupError> {
        if rate <= 0 {
            return Err(SetupError::Invalid);
        }
        let tnominal = nominal_bitrate as f64;
        let mut nominal = nominal_bitrate;
        if nominal <= 0 {
            if max_bitrate > 0 {
                nominal = if min_bitrate > 0 {
                    ((max_bitrate + min_bitrate) as f64 * 0.5) as i64
                } else {
                    (max_bitrate as f64 * 0.875) as i64
                };
            } else if min_bitrate > 0 {
                nominal = min_bitrate;
            } else {
                return Err(SetupError::Invalid);
            }
        }
        let (setup, base) = get_setup_template(channels, rate, nominal as f64, true)
            .ok_or(SetupError::Unsupported)?;
        let mut ci = Self::setting(setup, base, channels, rate);
        let hi = &mut ci.hi;
        hi.req = nominal as f32;
        hi.managed = true;
        hi.bitrate_min = min_bitrate;
        hi.bitrate_max = max_bitrate;
        hi.bitrate_av = tnominal as i64;
        hi.bitrate_av_damp = 1.5_f32 as f64;
        hi.bitrate_reservoir = nominal * 2;
        hi.bitrate_reservoir_bias = 0.1;
        ci.setup_init()?;
        Ok(ci)
    }

    /// `vorbis_encode_setup_setting` (on a fresh `vorbis_info`).
    fn setting(setup: &'static SetupTemplate, base: f64, channels: i64, rate: i64) -> Self {
        let is = base as usize;
        let ds = base - is as f64;
        let lerp = |a: f64, b: f64| a * (1. - ds) + b * ds;
        let hi = Highlevel {
            setup,
            base_setting: base,
            impulse_noisetune: 0.0,
            req: 0.0,
            managed: false,
            bitrate_min: 0,
            bitrate_av: 0,
            bitrate_av_damp: 0.0,
            bitrate_max: 0,
            bitrate_reservoir: 0,
            bitrate_reservoir_bias: 0.0,
            impulse_block_p: true,
            noise_normalize_p: true,
            stereo_point_setting: base,
            lowpass_khz: lerp(setup.psy_lowpass[is], setup.psy_lowpass[is + 1]),
            ath_floating_db: lerp(
                setup.psy_ath_float[is] as f64,
                setup.psy_ath_float[is + 1] as f64,
            ),
            ath_absolute_db: lerp(
                setup.psy_ath_abs[is] as f64,
                setup.psy_ath_abs[is + 1] as f64,
            ),
            amplitude_track_dbpersec: -6.0,
            trigger_setting: base,
            pre_amplitude: lerp(setup.pre_amp[is] as f64, setup.pre_amp[is + 1] as f64) as f32,
            block: [ByBlock {
                tone_mask_setting: base,
                tone_peaklimit_setting: base,
                noise_bias_setting: base,
                noise_compand_setting: base,
            }; 4],
        };
        CodecSetup {
            channels: channels as i32,
            rate,
            bitrate_upper: 0,
            bitrate_nominal: 0,
            bitrate_lower: 0,
            bitrate_window: 0,
            blocksizes: [0; 2],
            modes: Vec::new(),
            maps: Vec::new(),
            floors: Vec::new(),
            residues: Vec::new(),
            books: Vec::new(),
            psy_param: Vec::new(),
            psy_g_param: InfoPsyGlobal::default(),
            bi: BitrateInfo::default(),
            hi,
            block_lowpassr: [0; 2],
        }
    }

    /// `vorbis_encode_setup_init`.
    fn setup_init(&mut self) -> Result<(), SetupError> {
        if self.channels < 1 || self.channels > 255 {
            return Err(SetupError::Invalid);
        }
        let i0 = if self.hi.impulse_block_p { 0 } else { 1 };
        let hi = &mut self.hi;
        hi.ath_floating_db = hi.ath_floating_db.clamp(-200.0, -80.0);
        hi.amplitude_track_dbpersec = hi.amplitude_track_dbpersec.clamp(-99999.0, 0.0);
        let setup = hi.setup;
        let base = hi.base_setting;

        // vorbis_encode_blocksize_setup
        let is = base as usize;
        self.blocksizes = [
            setup.blocksize_short[is] as i64,
            setup.blocksize_long[is] as i64,
        ];
        let singleblock = self.blocksizes[0] == self.blocksizes[1];

        for i in 0..setup.floor_mappings as usize {
            self.floor_setup(base, setup.floor_mapping_list[i]);
        }
        self.global_psych_setup(self.hi.trigger_setting);
        self.global_stereo(setup.stereo_modes);
        let start = setup.psy_noise_normal_start;
        let part = setup.psy_noise_normal_partition;
        self.psyset_setup(base, start[0], part[0], 0);
        self.psyset_setup(base, start[0], part[0], 1);
        if !singleblock {
            self.psyset_setup(base, start[1], part[1], 2);
            self.psyset_setup(base, start[1], part[1], 3);
        }
        let b = self.hi.block;
        self.tonemask_setup(b[i0].tone_mask_setting, 0, setup.psy_tone_adj_impulse);
        self.tonemask_setup(b[1].tone_mask_setting, 1, setup.psy_tone_adj_other);
        if !singleblock {
            self.tonemask_setup(b[2].tone_mask_setting, 2, setup.psy_tone_adj_other);
            self.tonemask_setup(b[3].tone_mask_setting, 3, setup.psy_tone_adj_long);
        }
        let (short, long) = (
            setup.psy_noise_compand_short_mapping,
            setup.psy_noise_compand_long_mapping,
        );
        self.compand_setup(b[i0].noise_compand_setting, 0, short);
        self.compand_setup(b[1].noise_compand_setting, 1, short);
        if !singleblock {
            self.compand_setup(b[2].noise_compand_setting, 2, long);
            self.compand_setup(b[3].noise_compand_setting, 3, long);
        }
        self.peak_setup(b[i0].tone_peaklimit_setting, 0);
        self.peak_setup(b[1].tone_peaklimit_setting, 1);
        if !singleblock {
            self.peak_setup(b[2].tone_peaklimit_setting, 2);
            self.peak_setup(b[3].tone_peaklimit_setting, 3);
        }
        let tune = if i0 == 0 {
            self.hi.impulse_noisetune
        } else {
            0.0
        };
        self.noisebias_setup(
            b[i0].noise_bias_setting,
            0,
            setup.psy_noise_bias_impulse,
            tune,
        );
        self.noisebias_setup(
            b[1].noise_bias_setting,
            1,
            setup.psy_noise_bias_padding,
            0.0,
        );
        if !singleblock {
            self.noisebias_setup(b[2].noise_bias_setting, 2, setup.psy_noise_bias_trans, 0.0);
            self.noisebias_setup(b[3].noise_bias_setting, 3, setup.psy_noise_bias_long, 0.0);
        }
        for block in 0..if singleblock { 2 } else { 4 } {
            // vorbis_encode_ath_setup
            let p = &mut self.psy_param[block];
            p.ath_adjatt = self.hi.ath_floating_db as f32;
            p.ath_maxatt = self.hi.ath_absolute_db as f32;
        }
        self.map_n_res_setup(base, setup.maps);

        let hi = &self.hi;
        self.bitrate_nominal = if hi.bitrate_av > 0 {
            hi.bitrate_av
        } else {
            self.setting_to_approx_bitrate() as i64
        };
        self.bitrate_lower = hi.bitrate_min;
        self.bitrate_upper = hi.bitrate_max;
        self.bitrate_window = if hi.bitrate_av != 0 {
            (hi.bitrate_reservoir as f64 / hi.bitrate_av as f64) as i64
        } else {
            0
        };
        if hi.managed {
            self.bi = BitrateInfo {
                avg_rate: hi.bitrate_av,
                min_rate: hi.bitrate_min,
                max_rate: hi.bitrate_max,
                reservoir_bits: hi.bitrate_reservoir,
                reservoir_bias: hi.bitrate_reservoir_bias,
                slew_damp: hi.bitrate_av_damp,
            };
        }
        Ok(())
    }

    /// `book_dup_or_new`: the index of `book` among the books so far, or a new one.
    fn book_dup_or_new(&mut self, book: &'static StaticCodebook) -> usize {
        match self.books.iter().position(|&b| std::ptr::eq(b, book)) {
            Some(i) => i,
            None => {
                self.books.push(book);
                self.books.len() - 1
            }
        }
    }

    /// `vorbis_encode_floor_setup`: floor `x[s]`, its books appended.
    fn floor_setup(&mut self, s: f64, x: &[i32]) {
        let is = s as usize;
        let setup = self.hi.setup;
        let mut f = setup.floor_params[x[is] as usize].clone();
        let nbooks = self.books.len() as i32;
        let maxclass = f.partitionclass[..f.partitions as usize]
            .iter()
            .fold(-1, |m, &c| m.max(c));
        let mut maxbook = -1;
        for i in 0..(maxclass + 1) as usize {
            maxbook = maxbook.max(f.class_book[i]);
            f.class_book[i] += nbooks;
            for k in 0..1usize << f.class_subs[i] {
                maxbook = maxbook.max(f.class_subbook[i][k]);
                if f.class_subbook[i][k] >= 0 {
                    f.class_subbook[i][k] += nbooks;
                }
            }
        }
        let books = setup.floor_books[x[is] as usize];
        for i in 0..(maxbook + 1) as usize {
            self.books.push(books[i]);
        }
        self.floors.push(f);
    }

    /// `vorbis_encode_global_psych_setup`.
    fn global_psych_setup(&mut self, s: f64) {
        let setup = self.hi.setup;
        let (inp, x) = (setup.global_params, setup.global_mapping);
        let mut is = s as usize;
        let mut ds = s - is as f64;
        let g = &mut self.psy_g_param;
        *g = inp[x[is] as usize].clone();
        ds = x[is] * (1. - ds) + x[is + 1] * ds;
        is = ds as usize;
        ds -= is as f64;
        if ds == 0.0 && is > 0 {
            is -= 1;
            ds = 1.0;
        }
        for i in 0..4 {
            g.preecho_thresh[i] = (inp[is].preecho_thresh[i] as f64 * (1. - ds)
                + inp[is + 1].preecho_thresh[i] as f64 * ds)
                as f32;
            g.postecho_thresh[i] = (inp[is].postecho_thresh[i] as f64 * (1. - ds)
                + inp[is + 1].postecho_thresh[i] as f64 * ds)
                as f32;
        }
        g.ampmax_att_per_sec = self.hi.amplitude_track_dbpersec as f32;
    }

    /// `vorbis_encode_global_stereo`.
    fn global_stereo(&mut self, p: Option<&'static [AdjStereo]>) {
        let hi = &self.hi;
        // `float s`: the setting rounded to float first
        let s = hi.stereo_point_setting as f32;
        let is = s as usize;
        let ds = (s - is as f32) as f64;
        let g = &mut self.psy_g_param;
        let rate = self.rate as f64;
        let bs = self.blocksizes;
        let limit = |khz: f32, b: i64| ((khz as f64 * 1000. / rate) * b as f64) as i32;
        let Some(p) = p else {
            for i in 0..PACKETBLOBS {
                g.sliding_lowpass[0][i] = bs[0] as i32;
                g.sliding_lowpass[1][i] = bs[1] as i32;
            }
            return;
        };
        g.coupling_prepointamp = p[is].pre;
        g.coupling_postpointamp = p[is].post;
        let lerp = |a: f32, b: f32| (a as f64 * (1. - ds) + b as f64 * ds) as f32;
        if hi.managed {
            for i in 0..PACKETBLOBS {
                let khz = lerp(p[is].khz[i], p[is + 1].khz[i]);
                g.coupling_pointlimit[0][i] = limit(khz, bs[0]);
                g.coupling_pointlimit[1][i] = limit(khz, bs[1]);
                g.coupling_pkhz[i] = khz as i32;
                let khz = lerp(p[is].lowpass_khz[i], p[is + 1].lowpass_khz[i]);
                g.sliding_lowpass[0][i] = limit(khz, bs[0]);
                g.sliding_lowpass[1][i] = limit(khz, bs[1]);
            }
        } else {
            let khz = lerp(p[is].khz[PACKETBLOBS / 2], p[is + 1].khz[PACKETBLOBS / 2]);
            for i in 0..PACKETBLOBS {
                g.coupling_pointlimit[0][i] = limit(khz, bs[0]);
                g.coupling_pointlimit[1][i] = limit(khz, bs[1]);
                g.coupling_pkhz[i] = khz as i32;
            }
            let khz = lerp(
                p[is].lowpass_khz[PACKETBLOBS / 2],
                p[is + 1].lowpass_khz[PACKETBLOBS / 2],
            );
            for i in 0..PACKETBLOBS {
                g.sliding_lowpass[0][i] = limit(khz, bs[0]);
                g.sliding_lowpass[1][i] = limit(khz, bs[1]);
            }
        }
    }

    /// `vorbis_encode_psyset_setup`: block `block`'s psy parameters from the template.
    fn psyset_setup(&mut self, s: f64, nn_start: &[i32], nn_partition: &[i32], block: usize) {
        let is = s as usize;
        while self.psy_param.len() <= block {
            self.psy_param.push(_PSY_INFO_TEMPLATE.clone());
        }
        let mut p = _PSY_INFO_TEMPLATE.clone();
        p.blockflag = (block >> 1) as i32;
        if self.hi.noise_normalize_p {
            p.normal_p = 1;
            p.normal_start = nn_start[is];
            p.normal_partition = nn_partition[is];
            p.normal_thresh = self.hi.setup.psy_noise_normal_thresh[is];
        }
        self.psy_param[block] = p;
    }

    /// `vorbis_encode_tonemask_setup`.
    fn tonemask_setup(&mut self, s: f64, block: usize, inp: &[VpAdjblock]) {
        let setup = self.hi.setup;
        let (att, max) = (setup.psy_tone_masteratt, setup.psy_tone_0db);
        let is = s as usize;
        let ds = s - is as f64;
        let lerp = |a: f64, b: f64| (a * (1. - ds) + b * ds) as f32;
        let p = &mut self.psy_param[block];
        for k in 0..3 {
            p.tone_masteratt[k] = lerp(att[is].att[k] as f64, att[is + 1].att[k] as f64);
        }
        p.tone_centerboost = lerp(att[is].boost as f64, att[is + 1].boost as f64);
        p.tone_decay = lerp(att[is].decay as f64, att[is + 1].decay as f64);
        p.max_curve_db = lerp(max[is] as f64, max[is + 1] as f64);
        for i in 0..P_BANDS {
            p.toneatt[i] = lerp(inp[is].block[i] as f64, inp[is + 1].block[i] as f64);
        }
    }

    /// `vorbis_encode_compand_setup`.
    fn compand_setup(&mut self, s: f64, block: usize, x: &[f64]) {
        let setup = self.hi.setup;
        let inp = setup.psy_noise_compand;
        let mappings = setup.mappings as usize;
        let mut is = s as usize;
        let mut ds = s - is as f64;
        let hcm_stop = 5.min(mappings);
        let p = &mut self.psy_param[block];
        p.flacint = ds as f32;
        ds = x[is] * (1. - ds) + x[is + 1] * ds;
        is = ds as usize;
        ds -= is as f64;
        if ds == 0.0 && is > 0 {
            is -= 1;
            ds = 1.0;
        }
        let mut ishcm = is;
        let mut dshcm = ds + 0.3;
        if dshcm > 1.0 {
            ishcm += 1;
            dshcm -= 1.0;
        }
        if x[hcm_stop] < ishcm as f64 + dshcm {
            ishcm = x[hcm_stop] as usize;
            dshcm = x[hcm_stop] - ishcm as f64;
            if (ishcm as f64 + dshcm) < (is as f64 + ds) {
                ishcm = is;
                dshcm = ds;
            }
        }
        if dshcm == 0.0 && ishcm > 0 {
            ishcm -= 1;
            dshcm = 1.0;
        }
        for i in 0..NOISE_COMPAND_LEVELS {
            p.noisecompand[i] =
                (inp[is].data[i] as f64 * (1. - ds) + inp[is + 1].data[i] as f64 * ds) as f32;
        }
        for i in 0..NOISE_COMPAND_LEVELS {
            p.noisecompand_high[i] = (inp[ishcm].data[i] as f64 * (1. - dshcm)
                + inp[ishcm + 1].data[i] as f64 * dshcm)
                as f32;
        }
    }

    /// `vorbis_encode_peak_setup`.
    fn peak_setup(&mut self, s: f64, block: usize) {
        let suppress = self.hi.setup.psy_tone_dbsuppress;
        let is = s as usize;
        let ds = s - is as f64;
        self.psy_param[block].tone_abs_limit =
            (suppress[is] as f64 * (1. - ds) + suppress[is + 1] as f64 * ds) as f32;
    }

    /// `vorbis_encode_noisebias_setup`.
    fn noisebias_setup(&mut self, s: f64, block: usize, inp: &[Noise3], userbias: f64) {
        let setup = self.hi.setup;
        let (suppress, guard) = (setup.psy_noise_dbsuppress, setup.psy_noiseguards);
        let is = s as usize;
        let ds = s - is as f64;
        let p = &mut self.psy_param[block];
        p.noisemaxsupp = (suppress[is] as f64 * (1. - ds) + suppress[is + 1] as f64 * ds) as f32;
        p.noisewindowlomin = guard[block].lo;
        p.noisewindowhimin = guard[block].hi;
        p.noisewindowfixed = guard[block].fixed;
        for j in 0..P_NOISECURVES {
            for i in 0..P_BANDS {
                p.noiseoff[j][i] = (inp[is].data[j][i] as f64 * (1. - ds)
                    + inp[is + 1].data[j][i] as f64 * ds) as f32;
            }
        }
        for j in 0..P_NOISECURVES {
            let min = p.noiseoff[j][0] + 6.0;
            for i in 0..P_BANDS {
                p.noiseoff[j][i] = (p.noiseoff[j][i] as f64 + userbias) as f32;
                if p.noiseoff[j][i] < min {
                    p.noiseoff[j][i] = min;
                }
            }
        }
    }

    /// `vorbis_encode_residue_setup`: residue `number` for block `block`.
    fn residue_setup(&mut self, number: usize, block: usize, res: &'static ResidueTemplate) {
        let mut r = res.res.clone();
        if self.residues.len() <= number {
            self.residues
                .resize_with(number + 1, || (0, res.res.clone()));
        }
        r.grouping = res.grouping;
        let (aux, base) = if self.hi.managed {
            (res.book_aux_managed, res.books_base_managed)
        } else {
            (res.book_aux, res.books_base)
        };
        let base = base.expect("residue template without books");
        let aux = aux.expect("residue template without a group book");
        for i in 0..r.partitions as usize {
            for k in 0..4 {
                if base.books[i][k].is_some() {
                    r.secondstages[i] |= 1 << k;
                }
            }
        }
        r.groupbook = self.book_dup_or_new(aux) as i32;
        let mut booklist = 0;
        for i in 0..r.partitions as usize {
            for k in 0..4 {
                if let Some(b) = base.books[i][k] {
                    r.booklist[booklist] = self.book_dup_or_new(b) as i32;
                    booklist += 1;
                }
            }
        }

        let mut freq = self.hi.lowpass_khz * 1000.;
        let nyq = self.rate as f64 / 2.;
        let blocksize = self.blocksizes[block] >> 1;
        if freq > nyq {
            freq = nyq;
        }
        self.floors[block].n = (freq / nyq * blocksize as f64) as i32;
        match res.limit_type {
            1 => {
                let k = if self.hi.managed {
                    PACKETBLOBS - 1
                } else {
                    PACKETBLOBS / 2
                };
                freq = self.psy_g_param.coupling_pkhz[k] as f64 * 1000.;
                if freq > nyq {
                    freq = nyq;
                }
            }
            2 => freq = LFE_FRQLIMIT,
            _ => {}
        }
        let grouping = r.grouping as i64;
        if res.res_type == 2 {
            let mut ch = 0i64;
            'maps: for mi in &self.maps {
                for j in 0..mi.submaps as usize {
                    if mi.residuesubmap[j] as usize == number {
                        ch += (0..self.channels as usize)
                            .filter(|&k| mi.chmuxlist[k] as usize == j)
                            .count() as i64;
                    }
                    if ch != 0 {
                        break 'maps;
                    }
                }
            }
            r.end = ((freq / nyq * blocksize as f64 * ch as f64) / grouping as f64 + 0.9) as i32
                as i64
                * grouping;
            if r.end > blocksize * ch {
                r.end = blocksize * ch / grouping * grouping;
            }
            if freq != LFE_FRQLIMIT {
                self.block_lowpassr[block] = (r.end / ch) as i32;
            }
        } else {
            r.end =
                ((freq / nyq * blocksize as f64) / grouping as f64 + 0.9) as i32 as i64 * grouping;
            if r.end > blocksize {
                r.end = blocksize / grouping * grouping;
            }
            if freq != LFE_FRQLIMIT {
                self.block_lowpassr[block] = r.end as i32;
            }
        }
        if r.end == 0 {
            r.end = grouping;
        }
        self.residues[number] = (res.res_type, r);
    }

    /// `vorbis_encode_map_n_res_setup`.
    fn map_n_res_setup(&mut self, s: f64, maps: &'static [MappingTemplate]) {
        let is = s as usize;
        let map = maps[is].map;
        let res = maps[is].res;
        let modes = if self.blocksizes[0] == self.blocksizes[1] {
            1
        } else {
            2
        };
        for i in 0..modes {
            self.modes.push(_MODE_TEMPLATE[i].clone());
            self.maps.push(map[i].clone());
            for j in 0..map[i].submaps as usize {
                let number = map[i].residuesubmap[j] as usize;
                self.residue_setup(number, i, &res[number]);
            }
        }
    }

    /// `setting_to_approx_bitrate` (every shipped template has a bitrate mapping: the C's -1
    /// case can't occur).
    fn setting_to_approx_bitrate(&self) -> f64 {
        let hi = &self.hi;
        let is = hi.base_setting as usize;
        let ds = hi.base_setting - is as f64;
        let r = hi.setup.rate_mapping;
        (r[is] * (1. - ds) + r[is + 1] * ds) * self.channels as f64
    }
}

#[cfg(test)]
impl CodecSetup {
    /// The fields `aotuv_setup_dump` (aotuv-sys `shim.c`) flattens, in its order.
    pub(crate) fn dump(&self) -> Vec<f64> {
        let mut o: Vec<f64> = Vec::new();
        let mut f = |v: &[f64]| o.extend_from_slice(v);
        let fl = |v: &[f32]| v.iter().map(|&x| x as f64).collect::<Vec<_>>();
        let il = |v: &[i32]| v.iter().map(|&x| x as f64).collect::<Vec<_>>();
        f(&[self.psy_param.len() as f64]);
        for p in &self.psy_param {
            f(&[p.blockflag as f64, p.ath_adjatt as f64, p.ath_maxatt as f64]);
            f(&fl(&p.tone_masteratt));
            f(&[
                p.tone_centerboost as f64,
                p.tone_decay as f64,
                p.tone_abs_limit as f64,
            ]);
            f(&fl(&p.toneatt));
            f(&[
                p.noisemaskp as f64,
                p.noisemaxsupp as f64,
                p.noisewindowlo as f64,
                p.noisewindowhi as f64,
                p.noisewindowlomin as f64,
                p.noisewindowhimin as f64,
                p.noisewindowfixed as f64,
            ]);
            for j in 0..P_NOISECURVES {
                f(&fl(&p.noiseoff[j]));
            }
            f(&fl(&p.noisecompand));
            f(&fl(&p.noisecompand_high));
            f(&[
                p.flacint as f64,
                p.max_curve_db as f64,
                p.normal_p as f64,
                p.normal_start as f64,
                p.normal_partition as f64,
                p.normal_thresh,
            ]);
        }
        let g = &self.psy_g_param;
        f(&[g.eighth_octave_lines as f64]);
        f(&fl(&g.preecho_thresh));
        f(&fl(&g.postecho_thresh));
        f(&[
            g.stretch_penalty as f64,
            g.preecho_minenergy as f64,
            g.ampmax_att_per_sec as f64,
        ]);
        f(&il(&g.coupling_pkhz));
        f(&il(&g.coupling_pointlimit[0]));
        f(&il(&g.coupling_pointlimit[1]));
        f(&il(&g.coupling_prepointamp));
        f(&il(&g.coupling_postpointamp));
        f(&il(&g.sliding_lowpass[0]));
        f(&il(&g.sliding_lowpass[1]));
        f(&il(&self.block_lowpassr));
        let hi = &self.hi;
        f(&[
            hi.base_setting,
            hi.impulse_noisetune,
            hi.req as f64,
            hi.managed as i32 as f64,
            hi.bitrate_min as f64,
            hi.bitrate_av as f64,
            hi.bitrate_av_damp,
            hi.bitrate_max as f64,
            hi.bitrate_reservoir as f64,
            hi.bitrate_reservoir_bias,
            hi.stereo_point_setting,
            hi.lowpass_khz,
            hi.ath_floating_db,
            hi.ath_absolute_db,
            hi.amplitude_track_dbpersec,
            hi.trigger_setting,
            hi.pre_amplitude as f64,
        ]);
        let bi = &self.bi;
        f(&[
            bi.avg_rate as f64,
            bi.min_rate as f64,
            bi.max_rate as f64,
            bi.reservoir_bits as f64,
            bi.reservoir_bias,
            bi.slew_damp,
            self.bitrate_window as f64,
        ]);
        o
    }
}
