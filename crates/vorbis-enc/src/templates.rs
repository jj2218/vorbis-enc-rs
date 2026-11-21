//! The encoder's setup templates (`vorbisenc.c`, `modes/`, `books/`), as types: one per C
//! struct, field for field (C names in snake case; `long` → `i64`, pointers → references,
//! nullable ones `Option`). The values are generated from the compiled C (`setup_tables.rs`,
//! `just vorbis-tables`).

/// `P_BANDS`, `P_NOISECURVES`, `NOISE_COMPAND_LEVELS`, `PACKETBLOBS`, `VE_BANDS`.
pub(crate) const P_BANDS: usize = 17;
pub(crate) const P_NOISECURVES: usize = 3;
pub(crate) const NOISE_COMPAND_LEVELS: usize = 40;
pub(crate) const PACKETBLOBS: usize = 15;
pub(crate) const VE_BANDS: usize = 12;
/// `VIF_POSIT`, `VIF_CLASS`, `VIF_PARTS` (floor1 limits).
pub(crate) const VIF_POSIT: usize = 63;
const VIF_CLASS: usize = 16;
const VIF_PARTS: usize = 31;

/// `static_codebook`: a codebook as the setup header carries it.
#[derive(Debug)]
pub(crate) struct StaticCodebook {
    pub dim: i64,
    pub entries: i64,
    /// codeword lengths per entry (0: unused)
    pub lengthlist: &'static [u8],
    pub maptype: i32,
    pub q_min: i64,
    pub q_delta: i64,
    pub q_quant: i32,
    pub q_sequencep: i32,
    /// empty when the C's is NULL (maptype 0)
    pub quantlist: &'static [i64],
}

/// `static_bookblock`: per partition class, up to four second-stage books.
#[derive(Debug)]
pub(crate) struct StaticBookblock {
    pub books: [[Option<&'static StaticCodebook>; 4]; 12],
}

/// `vorbis_info_residue0`.
#[derive(Clone, Debug)]
pub(crate) struct InfoResidue0 {
    pub begin: i64,
    pub end: i64,
    pub grouping: i32,
    pub partitions: i32,
    #[expect(
        dead_code,
        reason = "the C's decoder-side field (the encoder recomputes it)"
    )]
    pub partvals: i32,
    pub groupbook: i32,
    pub secondstages: [i32; 64],
    pub booklist: [i32; 512],
    pub classmetric1: [i32; 64],
    pub classmetric2: [i32; 64],
}

/// `vorbis_residue_template`.
#[derive(Debug)]
pub(crate) struct ResidueTemplate {
    pub res_type: i32,
    pub limit_type: i32,
    pub grouping: i32,
    pub res: &'static InfoResidue0,
    pub book_aux: Option<&'static StaticCodebook>,
    pub book_aux_managed: Option<&'static StaticCodebook>,
    pub books_base: Option<&'static StaticBookblock>,
    pub books_base_managed: Option<&'static StaticBookblock>,
}

/// `vorbis_info_mapping0`.
#[derive(Clone, Debug)]
pub(crate) struct InfoMapping0 {
    pub submaps: i32,
    pub chmuxlist: [i32; 256],
    pub floorsubmap: [i32; 16],
    pub residuesubmap: [i32; 16],
    pub coupling_steps: i32,
    pub coupling_mag: [i32; 256],
    pub coupling_ang: [i32; 256],
}

/// `vorbis_mapping_template`.
#[derive(Debug)]
pub(crate) struct MappingTemplate {
    pub map: &'static [InfoMapping0],
    pub res: &'static [ResidueTemplate],
}

/// `vorbis_info_mode`.
#[derive(Clone, Debug)]
pub(crate) struct InfoMode {
    pub blockflag: i32,
    pub windowtype: i32,
    pub transformtype: i32,
    pub mapping: i32,
}

/// `vorbis_info_floor1`.
#[derive(Clone, Debug)]
pub(crate) struct InfoFloor1 {
    pub partitions: i32,
    pub partitionclass: [i32; VIF_PARTS],
    pub class_dim: [i32; VIF_CLASS],
    pub class_subs: [i32; VIF_CLASS],
    pub class_book: [i32; VIF_CLASS],
    pub class_subbook: [[i32; 8]; VIF_CLASS],
    pub mult: i32,
    pub postlist: [i32; VIF_POSIT + 2],
    pub maxover: f32,
    pub maxunder: f32,
    pub maxerr: f32,
    pub twofitweight: f32,
    pub twofitatten: f32,
    pub n: i32,
}

/// `vorbis_info_psy`.
#[derive(Clone, Debug)]
pub(crate) struct InfoPsy {
    pub blockflag: i32,
    pub ath_adjatt: f32,
    pub ath_maxatt: f32,
    pub tone_masteratt: [f32; P_NOISECURVES],
    pub tone_centerboost: f32,
    pub tone_decay: f32,
    pub tone_abs_limit: f32,
    pub toneatt: [f32; P_BANDS],
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "set by the templates, read by no aoTuV encoder path"
        )
    )]
    pub noisemaskp: i32,
    pub noisemaxsupp: f32,
    pub noisewindowlo: f32,
    pub noisewindowhi: f32,
    pub noisewindowlomin: i32,
    pub noisewindowhimin: i32,
    pub noisewindowfixed: i32,
    pub noiseoff: [[f32; P_BANDS]; P_NOISECURVES],
    pub noisecompand: [f32; NOISE_COMPAND_LEVELS],
    pub noisecompand_high: [f32; NOISE_COMPAND_LEVELS],
    pub flacint: f32,
    pub max_curve_db: f32,
    pub normal_p: i32,
    pub normal_start: i32,
    pub normal_partition: i32,
    pub normal_thresh: f64,
}

impl InfoPsy {
    /// The noise-normalisation partition length (the C: `vi->normal_p ? vi->normal_partition :
    /// 16`; every setup sets `normal_p`).
    pub(crate) fn partition(&self) -> usize {
        if self.normal_p != 0 {
            self.normal_partition as usize
        } else {
            16
        }
    }
}

/// `vorbis_info_psy_global`.
#[derive(Clone, Debug)]
pub(crate) struct InfoPsyGlobal {
    pub eighth_octave_lines: i32,
    pub preecho_thresh: [f32; VE_BANDS],
    pub postecho_thresh: [f32; VE_BANDS],
    pub stretch_penalty: f32,
    pub preecho_minenergy: f32,
    pub ampmax_att_per_sec: f32,
    pub coupling_pkhz: [i32; PACKETBLOBS],
    pub coupling_pointlimit: [[i32; PACKETBLOBS]; 2],
    pub coupling_prepointamp: [i32; PACKETBLOBS],
    pub coupling_postpointamp: [i32; PACKETBLOBS],
    pub sliding_lowpass: [[i32; PACKETBLOBS]; 2],
}

/// `vp_adjblock`.
#[derive(Debug)]
pub(crate) struct VpAdjblock {
    pub block: [i32; P_BANDS],
}

/// `compandblock`.
#[derive(Debug)]
pub(crate) struct Compandblock {
    pub data: [i32; NOISE_COMPAND_LEVELS],
}

/// `att3`.
#[derive(Debug)]
pub(crate) struct Att3 {
    pub att: [i32; P_NOISECURVES],
    pub boost: f32,
    pub decay: f32,
}

/// `adj_stereo`.
#[derive(Debug)]
pub(crate) struct AdjStereo {
    pub pre: [i32; PACKETBLOBS],
    pub post: [i32; PACKETBLOBS],
    pub khz: [f32; PACKETBLOBS],
    pub lowpass_khz: [f32; PACKETBLOBS],
}

/// `noiseguard`.
#[derive(Debug)]
pub(crate) struct Noiseguard {
    pub lo: i32,
    pub hi: i32,
    pub fixed: i32,
}

/// `noise3`.
#[derive(Debug)]
pub(crate) struct Noise3 {
    pub data: [[i32; 17]; P_NOISECURVES],
}

/// `ve_setup_data_template`: one rate × coupling family's settings, per quality step
/// (`mappings` + 1 points; the encoder interpolates between neighbours).
#[derive(Debug)]
pub(crate) struct SetupTemplate {
    pub mappings: i32,
    pub rate_mapping: &'static [f64],
    pub quality_mapping: &'static [f64],
    pub pre_amp: &'static [f32],
    pub coupling_restriction: i32,
    pub samplerate_min_restriction: i64,
    pub samplerate_max_restriction: i64,
    pub blocksize_short: &'static [i32],
    pub blocksize_long: &'static [i32],
    pub psy_tone_masteratt: &'static [Att3],
    pub psy_tone_0db: &'static [i32],
    pub psy_tone_dbsuppress: &'static [i32],
    pub psy_tone_adj_impulse: &'static [VpAdjblock],
    pub psy_tone_adj_long: &'static [VpAdjblock],
    pub psy_tone_adj_other: &'static [VpAdjblock],
    pub psy_noiseguards: &'static [Noiseguard],
    pub psy_noise_bias_impulse: &'static [Noise3],
    pub psy_noise_bias_padding: &'static [Noise3],
    pub psy_noise_bias_trans: &'static [Noise3],
    pub psy_noise_bias_long: &'static [Noise3],
    pub psy_noise_dbsuppress: &'static [i32],
    pub psy_noise_compand: &'static [Compandblock],
    pub psy_noise_compand_short_mapping: &'static [f64],
    pub psy_noise_compand_long_mapping: &'static [f64],
    pub psy_noise_normal_start: [&'static [i32]; 2],
    pub psy_noise_normal_partition: [&'static [i32]; 2],
    pub psy_noise_normal_thresh: &'static [f64],
    pub psy_ath_float: &'static [i32],
    pub psy_ath_abs: &'static [i32],
    pub psy_lowpass: &'static [f64],
    pub global_params: &'static [InfoPsyGlobal],
    pub global_mapping: &'static [f64],
    pub stereo_modes: Option<&'static [AdjStereo]>,
    pub floor_books: &'static [&'static [&'static StaticCodebook]],
    pub floor_params: &'static [InfoFloor1],
    pub floor_mappings: i32,
    pub floor_mapping_list: &'static [&'static [i32]],
    pub maps: &'static [MappingTemplate],
}
