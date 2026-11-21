// SPDX-License-Identifier: BSD-3-Clause
// Derived from uavs3d (`source/decoder/parser.c`), Copyright (c) 2018-2022 Peking University Shenzhen Graduate
// School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation. See LICENSE.uavs3d.
//! Sequence, picture and patch headers, extension and user data.

use crate::Error;
use crate::bits::Bits;

pub(crate) const MAX_REFS: usize = 17;
pub(crate) const MAX_RPLS: usize = 32;
pub(crate) const NO_VAR_BINS: usize = 16;
pub(crate) const ALF_MAX_NUM_COEF: usize = 9;
pub(crate) const MAX_QUANT_BASE: i32 = 63;

/// The 8x8 weighting matrix of a picture (an array that has a `Default`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Wq8(pub [u8; 64]);

impl Default for Wq8 {
    fn default() -> Self {
        Wq8([0; 64])
    }
}

impl std::ops::Deref for Wq8 {
    type Target = [u8; 64];
    fn deref(&self) -> &[u8; 64] {
        &self.0
    }
}

impl std::ops::DerefMut for Wq8 {
    fn deref_mut(&mut self) -> &mut [u8; 64] {
        &mut self.0
    }
}

/// Picture coding type (`SLICE_I` = 1, `SLICE_P` = 2, `SLICE_B` = 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SliceType {
    #[default]
    I,
    P,
    B,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Rpl {
    pub num: usize,
    pub active: usize,
    pub delta_doi: [i32; MAX_REFS],
}

#[derive(Clone, Debug)]
#[allow(dead_code)] // syntax elements that are parsed (and range checked) but not needed for decoding
pub(crate) struct Sequence {
    pub profile_id: u32,
    pub level_id: u32,
    pub progressive_sequence: bool,
    pub field_coded_sequence: bool,
    pub chroma_format: u32,
    pub encoding_precision: u32,
    pub output_reorder_delay: u32,
    pub sample_precision: u32,
    pub aspect_ratio: u32,
    pub frame_rate_code: u32,
    pub bit_rate_lower: u32,
    pub bit_rate_upper: u32,
    pub low_delay: bool,
    pub temporal_id_enable_flag: bool,
    pub bbv_buffer_size: u32,
    pub horizontal_size: usize,
    pub vertical_size: usize,
    pub log2_max_cu_width_height: u32,
    pub max_part_ratio_log2: u32,
    pub max_split_times: u32,
    pub min_qt_size: u32,
    pub max_bt_size: u32,
    pub max_eqt_size: u32,
    pub max_dt_size: u32,
    pub rpl1_index_exist_flag: bool,
    pub rpl1_same_as_rpl0_flag: bool,
    pub rpls_l0: Vec<Rpl>,
    pub rpls_l1: Vec<Rpl>,
    pub num_ref_default_active_minus1: [u32; 2],
    pub max_dpb_size: u32,
    pub ipcm_enable_flag: bool,
    pub amvr_enable_flag: bool,
    pub umve_enable_flag: bool,
    pub ipf_enable_flag: bool,
    pub emvr_enable_flag: bool,
    pub affine_enable_flag: bool,
    pub smvd_enable_flag: bool,
    pub dt_intra_enable_flag: bool,
    pub num_of_hmvp_cand: u32,
    pub tscpm_enable_flag: bool,
    pub sao_enable_flag: bool,
    pub alf_enable_flag: bool,
    pub secondary_transform_enable_flag: bool,
    pub position_based_transform_enable_flag: bool,
    pub wq_enable: bool,
    pub seq_wq_mode: bool,
    pub wq_4x4_matrix: [u8; 16],
    pub wq_8x8_matrix: [u8; 64],
    pub patch_stable: bool,
    pub cross_patch_loop_filter: bool,
    pub patch_ref_colocated: bool,
    pub patch_uniform: bool,
    pub patch_width: usize,
    pub patch_height: usize,

    // derived (`update_seqhdr`)
    pub bit_depth_internal: u32,
    pub bit_depth_input: u32,
    pub bit_depth_2_qp_offset: i32,
    pub pic_width: usize,
    pub pic_height: usize,
    pub max_cuwh: usize,
    pub log2_max_cuwh: u32,
    pub pic_width_in_lcu: usize,
    pub pic_height_in_lcu: usize,
    pub f_lcu: usize,
    pub pic_width_in_scu: usize,
    pub pic_height_in_scu: usize,
    pub i_scu: usize,
    pub f_scu: usize,
    pub a_scu: usize,
    pub patch_columns: usize,
    pub patch_rows: usize,
    pub patch_column_width: Vec<usize>,
    pub patch_row_height: Vec<usize>,
}

const WQ_DEFAULT_4X4: [u8; 16] = [64, 64, 64, 68, 64, 64, 68, 72, 64, 68, 76, 80, 72, 76, 84, 96];
const WQ_DEFAULT_8X8: [u8; 64] = [
    64, 64, 64, 64, 68, 68, 72, 76, 64, 64, 64, 68, 72, 76, 84, 92, 64, 64, 68, 72, 76, 80, 88, 100, 64, 68, 72, 80, 84, 92, 100, 112, 68, 72, 80, 84, 92, 104,
    112, 128, 76, 80, 84, 92, 104, 116, 132, 152, 96, 100, 104, 116, 124, 140, 164, 188, 104, 108, 116, 128, 152, 172, 192, 216,
];
const WQ_DEFAULT_PARAM: [[i32; 6]; 2] = [[67, 71, 71, 80, 80, 106], [64, 49, 53, 58, 58, 64]];
const WQ_MODEL_4X4: [[u8; 16]; 4] = [
    [0, 4, 3, 5, 4, 2, 1, 5, 3, 1, 1, 5, 5, 5, 5, 5],
    [0, 4, 4, 5, 3, 2, 2, 5, 3, 2, 1, 5, 5, 5, 5, 5],
    [0, 4, 3, 5, 4, 3, 2, 5, 3, 2, 1, 5, 5, 5, 5, 5],
    [0, 3, 1, 5, 3, 4, 2, 5, 1, 2, 2, 5, 5, 5, 5, 5],
];
include!("wq_model8.rs");

fn parse_rpl(b: &mut Bits<'_>, max_dpb_size: u32) -> Result<Rpl, Error> {
    let mut rpl = Rpl { num: b.ue_range(0, max_dpb_size)? as usize, ..Rpl::default() };
    if rpl.num > MAX_REFS {
        return Err(Error::Invalid("reference picture list too long"));
    }
    if rpl.num > 0 {
        let d = b.ue_range(1, 255)? as i32;
        rpl.delta_doi[0] = if b.read1()? != 0 { -d } else { d };
    }
    for i in 1..rpl.num {
        let mut d = b.ue_range(0, 255)? as i32;
        if d != 0 && b.read1()? != 0 {
            d = -d;
        }
        rpl.delta_doi[i] = rpl.delta_doi[i - 1] + d;
    }
    Ok(rpl)
}

fn read_wq_matrix(b: &mut Bits<'_>, m4: &mut [u8; 16], m8: &mut [u8; 64]) -> Result<(), Error> {
    for v in m4.iter_mut() {
        *v = b.ue_range(1, 255)? as u8;
    }
    for v in m8.iter_mut() {
        *v = b.ue_range(1, 255)? as u8;
    }
    Ok(())
}

impl Sequence {
    /// `dec_parse_sqh` + `update_seqhdr`; `b` is positioned at the start code.
    pub(crate) fn parse(b: &mut Bits<'_>) -> Result<Self, Error> {
        b.read_range(24, 1, 1)?;
        b.read_range(8, 0xB0, 0xB0)?;
        let profile_id = b.read(8)?;
        if profile_id != 0x20 && profile_id != 0x22 {
            return Err(Error::Unsupported("AVS3 profile"));
        }
        let level_id = b.read(8)?;
        let progressive_sequence = b.flag()?;
        let field_coded_sequence = b.flag()?;
        b.read1()?; // library_stream_flag
        b.read1()?; // library_picture_enable_flag
        b.read1()?;
        let horizontal_size = b.read(14)? as usize;
        b.read1()?;
        let vertical_size = b.read(14)? as usize;
        let chroma_format = b.read_range(2, 1, 1)?;
        let sample_precision = b.read_range(3, 1, 2)?;
        let encoding_precision = if profile_id == 0x22 { b.read_range(3, 1, 2)? } else { 1 };
        b.read1()?;
        let aspect_ratio = b.read_range(4, 1, 4)?;
        let frame_rate_code = b.read_range(4, 1, 13)?;
        b.read1()?;
        let bit_rate_lower = b.read(18)?;
        b.read1()?;
        let bit_rate_upper = b.read(12)?;
        let low_delay = b.flag()?;
        let temporal_id_enable_flag = b.flag()?;
        b.read1()?;
        let bbv_buffer_size = b.read(18)?;
        b.read1()?;
        let max_dpb_size = b.read_range(4, 0, 15)? + 1;
        let rpl1_index_exist_flag = b.flag()?;
        let rpl1_same_as_rpl0_flag = b.flag()?;
        b.read1()?;
        let n0 = b.ue_range(0, 64)? as usize;
        if n0 > MAX_RPLS {
            return Err(Error::Invalid("too many reference picture lists"));
        }
        let mut rpls_l0 = Vec::with_capacity(n0);
        for _ in 0..n0 {
            rpls_l0.push(parse_rpl(b, max_dpb_size)?);
        }
        let rpls_l1 = if rpl1_same_as_rpl0_flag {
            rpls_l0.clone()
        } else {
            let n1 = b.ue_range(0, 64)? as usize;
            if n1 > MAX_RPLS {
                return Err(Error::Invalid("too many reference picture lists"));
            }
            let mut v = Vec::with_capacity(n1);
            for _ in 0..n1 {
                v.push(parse_rpl(b, max_dpb_size)?);
            }
            v
        };
        let num_ref_default_active_minus1 = [b.ue_range(0, 14)?, b.ue_range(0, 14)?];
        let log2_max_cu_width_height = b.read_range(3, 3, 5)? + 2;
        b.read_range(2, 0, 0)?; // min_cu_size (4)
        let max_part_ratio_log2 = b.read(2)? + 2;
        let max_split_times = b.read(3)? + 6;
        let min_qt_size = 1 << (b.read_range(3, 0, 5)? + 2);
        let max_bt_size = 1 << (b.read_range(3, 0, 5)? + 2);
        let max_eqt_size = 1 << (b.read(2)? + 3);
        b.read1()?;
        let wq_enable = b.flag()?;
        let mut seq_wq_mode = false;
        let mut wq_4x4_matrix = [0u8; 16];
        let mut wq_8x8_matrix = [0u8; 64];
        if wq_enable {
            seq_wq_mode = b.flag()?;
            if seq_wq_mode {
                read_wq_matrix(b, &mut wq_4x4_matrix, &mut wq_8x8_matrix)?;
            } else {
                wq_4x4_matrix = WQ_DEFAULT_4X4;
                wq_8x8_matrix = WQ_DEFAULT_8X8;
            }
        }
        let secondary_transform_enable_flag = b.flag()?;
        let sao_enable_flag = b.flag()?;
        let alf_enable_flag = b.flag()?;
        let affine_enable_flag = b.flag()?;
        let smvd_enable_flag = b.flag()?;
        let ipcm_enable_flag = b.flag()?;
        let amvr_enable_flag = b.flag()?;
        let num_of_hmvp_cand = b.read_range(4, 0, 8)?;
        let umve_enable_flag = b.flag()?;
        let emvr_enable_flag = if amvr_enable_flag && num_of_hmvp_cand != 0 { b.flag()? } else { false };
        let ipf_enable_flag = b.flag()?;
        let tscpm_enable_flag = b.flag()?;
        b.read1()?;
        let dt_intra_enable_flag = b.flag()?;
        let mut max_dt_size = 0;
        if dt_intra_enable_flag {
            max_dt_size = 1 << (b.read_range(2, 0, 2)? + 4);
        }
        let position_based_transform_enable_flag = b.flag()?;
        let output_reorder_delay = if low_delay { 0 } else { b.read(5)? };
        let cross_patch_loop_filter = b.flag()?;
        let patch_ref_colocated = b.flag()?;
        let patch_stable = b.flag()?;
        let mut patch_uniform = false;
        let (mut patch_width, mut patch_height) = (0usize, 0usize);
        if patch_stable {
            patch_uniform = b.flag()?;
            if patch_uniform {
                b.read1()?;
                patch_width = b.ue_range(0, 255)? as usize + 1;
                patch_height = b.ue_range(0, 143)? as usize + 1;
            }
        }
        b.read(2)?;
        b.read1()?;
        b.align();

        let bit_depth_internal = if encoding_precision == 2 { 10 } else { 8 };
        let bit_depth_input = if sample_precision == 1 { 8 } else { 10 };
        if horizontal_size == 0 || vertical_size == 0 {
            return Err(Error::Invalid("empty picture"));
        }
        let pic_width = horizontal_size.div_ceil(8) * 8;
        let pic_height = vertical_size.div_ceil(8) * 8;
        let max_cuwh = 1usize << log2_max_cu_width_height;
        let pic_width_in_lcu = pic_width.div_ceil(max_cuwh);
        let pic_height_in_lcu = pic_height.div_ceil(max_cuwh);
        let pic_width_in_scu = pic_width.div_ceil(4);
        let pic_height_in_scu = pic_height.div_ceil(4);
        let i_scu = pic_width_in_scu + 2;
        let mut s = Self {
            profile_id,
            level_id,
            progressive_sequence,
            field_coded_sequence,
            chroma_format,
            encoding_precision,
            output_reorder_delay,
            sample_precision,
            aspect_ratio,
            frame_rate_code,
            bit_rate_lower,
            bit_rate_upper,
            low_delay,
            temporal_id_enable_flag,
            bbv_buffer_size,
            horizontal_size,
            vertical_size,
            log2_max_cu_width_height,
            max_part_ratio_log2,
            max_split_times,
            min_qt_size,
            max_bt_size,
            max_eqt_size,
            max_dt_size,
            rpl1_index_exist_flag,
            rpl1_same_as_rpl0_flag,
            rpls_l0,
            rpls_l1,
            num_ref_default_active_minus1,
            max_dpb_size,
            ipcm_enable_flag,
            amvr_enable_flag,
            umve_enable_flag,
            ipf_enable_flag,
            emvr_enable_flag,
            affine_enable_flag,
            smvd_enable_flag,
            dt_intra_enable_flag,
            num_of_hmvp_cand,
            tscpm_enable_flag,
            sao_enable_flag,
            alf_enable_flag,
            secondary_transform_enable_flag,
            position_based_transform_enable_flag,
            wq_enable,
            seq_wq_mode,
            wq_4x4_matrix,
            wq_8x8_matrix,
            patch_stable,
            cross_patch_loop_filter,
            patch_ref_colocated,
            patch_uniform,
            patch_width,
            patch_height,
            bit_depth_internal,
            bit_depth_input,
            bit_depth_2_qp_offset: 8 * (bit_depth_internal as i32 - 8),
            pic_width,
            pic_height,
            max_cuwh,
            log2_max_cuwh: log2_max_cu_width_height,
            pic_width_in_lcu,
            pic_height_in_lcu,
            f_lcu: pic_width_in_lcu * pic_height_in_lcu,
            pic_width_in_scu,
            pic_height_in_scu,
            i_scu,
            f_scu: i_scu * (pic_height_in_scu + 2),
            a_scu: i_scu * pic_height_in_scu,
            patch_columns: 0,
            patch_rows: 0,
            patch_column_width: Vec::new(),
            patch_row_height: Vec::new(),
        };
        s.update_patches()?;
        Ok(s)
    }

    /// The patch grid of `update_seqhdr`.
    fn update_patches(&mut self) -> Result<(), Error> {
        self.patch_width = self.pic_width_in_lcu;
        self.patch_height = self.patch_height.min(self.pic_height_in_lcu);
        if !self.patch_stable || !self.patch_uniform {
            return Err(Error::Unsupported("non-uniform AVS3 patches"));
        }
        let split = |total: usize, unit: usize| -> Vec<usize> {
            let mut v = vec![unit; total / unit];
            if !total.is_multiple_of(unit) {
                match v.last_mut() {
                    Some(last) => *last += total - unit * (total / unit),
                    None => v.push(total),
                }
            }
            v
        };
        if self.patch_width == 0 || self.patch_height == 0 {
            return Err(Error::Invalid("empty patch"));
        }
        self.patch_column_width = split(self.pic_width_in_lcu, self.patch_width);
        self.patch_row_height = split(self.pic_height_in_lcu, self.patch_height);
        self.patch_columns = self.patch_column_width.len();
        self.patch_rows = self.patch_row_height.len();
        Ok(())
    }

    /// Frame rate as a fraction.
    pub(crate) fn frame_rate(&self) -> (u32, u32) {
        match self.frame_rate_code {
            1 => (24000, 1001),
            2 => (24, 1),
            3 => (25, 1),
            4 => (30000, 1001),
            5 => (30, 1),
            6 => (50, 1),
            7 => (60000, 1001),
            8 => (60, 1),
            9 => (100, 1),
            10 => (120, 1),
            11 => (200, 1),
            12 => (240, 1),
            13 => (300, 1),
            _ => (25, 1),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct AlfParam {
    pub pic_alf_enable: bool,
    pub alf_flag: [bool; 3],
    pub var_ind_tab: [usize; NO_VAR_BINS],
    pub filter_coeff_luma: [[i32; ALF_MAX_NUM_COEF]; NO_VAR_BINS],
    pub filter_coeff_chroma: [i32; ALF_MAX_NUM_COEF * 2],
}

#[derive(Clone, Debug, Default)]
pub(crate) struct PicHeader {
    pub rpl_l0: Rpl,
    pub rpl_l1: Rpl,
    pub slice_type: SliceType,
    pub temporal_id: u32,
    pub loop_filter_disable_flag: bool,
    pub alpha_c_offset: i32,
    pub beta_offset: i32,
    pub chroma_quant_param_delta_cb: i32,
    pub chroma_quant_param_delta_cr: i32,
    pub alf_param: AlfParam,
    pub fixed_picture_qp_flag: bool,
    pub random_access_decodable_flag: bool,
    pub decode_order_index: u32,
    pub output_delay: u32,
    pub progressive_frame: bool,
    pub picture_structure: u32,
    pub top_field_first: bool,
    pub repeat_first_field: bool,
    pub picture_qp: i32,
    pub affine_subblock_size_idx: u32,
    pub pic_wq_enable: bool,
    pub wq_4x4_matrix: [u8; 16],
    pub wq_8x8_matrix: Wq8,
    pub pic_md5: [u8; 16],
    pub pic_md5_exist: bool,
    /// Picture order (`ptr`): decode order index + cycles, plus the output delay.
    pub ptr: i64,
}

/// The state of the decode order index cycle that the picture header parse reads and updates.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DoiState {
    pub prev_doi: u32,
    pub doi_cycles: i64,
    /// Set when the doi wrapped in this header (the caller shifts the doi of the stored pictures).
    pub wrapped: bool,
}

const DOI_CYCLE_LENGTH: i64 = 256;

fn parse_alf(b: &mut Bits<'_>) -> Result<AlfParam, Error> {
    let mut p = AlfParam::default();
    for f in p.alf_flag.iter_mut() {
        *f = b.flag()?;
    }
    p.pic_alf_enable = p.alf_flag.iter().any(|&f| f);
    if p.alf_flag[0] {
        let groups = b.ue_range(0, 15)? as usize + 1;
        let mut pattern = [false; 16];
        let mut pre = 0usize;
        for g in 0..groups {
            let mut sum = 0i32;
            if g > 0 {
                let symbol = if groups != 16 { b.ue_range(1, 15)? as usize } else { 1 };
                let symbol = (symbol + pre).min(15);
                pattern[symbol] = true;
                pre = symbol;
            }
            for i in 0..ALF_MAX_NUM_COEF - 1 {
                let c = b.se_range(-64, 63)?;
                p.filter_coeff_luma[g][i] = c;
                sum += 2 * c;
            }
            let lac = b.se_range(-1088, 1071)?;
            p.filter_coeff_luma[g][ALF_MAX_NUM_COEF - 1] = (1 << 6) - sum + lac;
        }
        if groups > 1 {
            p.var_ind_tab[0] = 0;
            for g in 1..NO_VAR_BINS {
                p.var_ind_tab[g] = p.var_ind_tab[g - 1] + usize::from(pattern[g]);
            }
        } else {
            p.var_ind_tab = [0; NO_VAR_BINS];
        }
    }
    for (c, start) in [(1usize, 0usize), (2, 1)] {
        if p.alf_flag[c] {
            let mut sum = 0i32;
            for k in 0..ALF_MAX_NUM_COEF - 1 {
                let v = b.se_range(-64, 63)?;
                p.filter_coeff_chroma[start + 2 * k] = v;
                sum += 2 * v;
            }
            let lac = b.se_range(-1088, 1071)?;
            p.filter_coeff_chroma[start + 2 * (ALF_MAX_NUM_COEF - 1)] = (1 << 6) - sum + lac;
        }
    }
    Ok(p)
}

impl PicHeader {
    /// `dec_parse_pic_header`; `b` is positioned at the `00 00 01 B3/B6` start code.
    pub(crate) fn parse(b: &mut Bits<'_>, seq: &Sequence, doi: &mut DoiState) -> Result<Self, Error> {
        let mut h = PicHeader::default();
        b.read_range(24, 1, 1)?;
        let start_code = b.read_range(8, 0xB3, 0xB6)?;
        if start_code != 0xB3 && start_code != 0xB6 {
            return Err(Error::Invalid("not a picture start code"));
        }
        if start_code == 0xB3 {
            h.slice_type = SliceType::I;
        } else {
            h.random_access_decodable_flag = b.flag()?;
        }
        b.read(32)?; // bbv_delay
        if start_code == 0xB6 {
            h.slice_type = if b.read_range(2, 1, 2)? == 1 { SliceType::P } else { SliceType::B };
        }
        if h.slice_type == SliceType::I && b.flag()? {
            b.read(24)?; // time_code
        }
        h.decode_order_index = b.read(8)?;
        if seq.temporal_id_enable_flag {
            h.temporal_id = b.read(3)?;
        }
        if !seq.low_delay {
            h.output_delay = b.ue_range(0, 63)?;
        } else {
            h.output_delay = 0;
            b.ue()?; // bbv_check_times
        }
        h.progressive_frame = b.flag()?;
        h.picture_structure = if !h.progressive_frame { b.read1()? } else { 1 };
        h.top_field_first = b.flag()?;
        h.repeat_first_field = b.flag()?;
        if seq.field_coded_sequence {
            b.read1()?; // top_field_picture_flag
            b.read1()?;
        }

        doi.wrapped = false;
        if h.decode_order_index < doi.prev_doi {
            doi.doi_cycles += 1;
            doi.wrapped = true;
        }
        h.ptr = i64::from(h.decode_order_index) + DOI_CYCLE_LENGTH * doi.doi_cycles + i64::from(h.output_delay) - i64::from(seq.output_reorder_delay);
        doi.prev_doi = h.decode_order_index;

        let sps0 = b.flag()?;
        let mut idx0 = None;
        if sps0 {
            if !seq.rpls_l0.is_empty() {
                let i = if seq.rpls_l0.len() > 1 { b.ue_range(0, seq.rpls_l0.len() as u32 - 1)? as usize } else { 0 };
                h.rpl_l0 = seq.rpls_l0[i];
                idx0 = Some(i);
            }
        } else {
            h.rpl_l0 = parse_rpl(b, seq.max_dpb_size)?;
        }
        let sps1 = if seq.rpl1_index_exist_flag { b.flag()? } else { sps0 };
        if sps1 {
            let i = if seq.rpls_l1.len() > 1 && seq.rpl1_index_exist_flag {
                b.ue_range(0, seq.rpls_l1.len() as u32 - 1)? as usize
            } else if !seq.rpl1_index_exist_flag {
                idx0.unwrap_or(0)
            } else {
                0
            };
            if let Some(r) = seq.rpls_l1.get(i) {
                h.rpl_l1 = *r;
            }
        } else {
            h.rpl_l1 = parse_rpl(b, seq.max_dpb_size)?;
        }

        if h.slice_type != SliceType::I {
            if b.flag()? {
                h.rpl_l0.active = b.ue_range(0, 14)? as usize + 1;
                if h.slice_type == SliceType::B {
                    h.rpl_l1.active = b.ue_range(0, 14)? as usize + 1;
                }
            } else {
                h.rpl_l0.active = seq.num_ref_default_active_minus1[0] as usize + 1;
                h.rpl_l1.active = if h.slice_type == SliceType::P { 0 } else { seq.num_ref_default_active_minus1[1] as usize + 1 };
            }
        } else {
            h.rpl_l0.active = 0;
            h.rpl_l1.active = 0;
        }

        h.fixed_picture_qp_flag = b.flag()?;
        h.picture_qp = b.read_range(7, 0, (MAX_QUANT_BASE + seq.bit_depth_2_qp_offset) as u32)? as i32;
        if h.slice_type != SliceType::I && !(h.slice_type == SliceType::B && h.picture_structure == 1) {
            b.read1()?;
        }
        h.loop_filter_disable_flag = b.flag()?;
        if !h.loop_filter_disable_flag && b.flag()? {
            h.alpha_c_offset = b.se_range(-8, 8)?;
            h.beta_offset = b.se_range(-8, 8)?;
        }
        if !b.flag()? {
            h.chroma_quant_param_delta_cb = b.se_range(-16, 16)?;
            h.chroma_quant_param_delta_cr = b.se_range(-16, 16)?;
        }

        h.wq_4x4_matrix = [64; 16];
        h.wq_8x8_matrix = Wq8([64; 64]);
        if seq.wq_enable {
            h.pic_wq_enable = b.flag()?;
            if h.pic_wq_enable {
                let data_idx = b.read_range(2, 0, 2)?;
                if data_idx == 0 {
                    h.wq_4x4_matrix = seq.wq_4x4_matrix;
                    h.wq_8x8_matrix = Wq8(seq.wq_8x8_matrix);
                } else if data_idx == 1 {
                    b.read1()?;
                    let wq_param = b.read_range(2, 0, 2)?;
                    let wq_model = b.read_range(2, 0, 3)? as usize;
                    let mut vec = [0i32; 6];
                    if wq_param == 0 {
                        vec = WQ_DEFAULT_PARAM[1];
                    } else {
                        let base = WQ_DEFAULT_PARAM[(wq_param - 1) as usize];
                        for (i, v) in vec.iter_mut().enumerate() {
                            *v = b.se()?.wrapping_add(base[i]);
                        }
                    }
                    for i in 0..16 {
                        h.wq_4x4_matrix[i] = vec[usize::from(WQ_MODEL_4X4[wq_model][i])] as u8;
                    }
                    for i in 0..64 {
                        h.wq_8x8_matrix[i] = vec[usize::from(WQ_MODEL_8X8[wq_model][i])] as u8;
                    }
                } else {
                    read_wq_matrix(b, &mut h.wq_4x4_matrix, &mut h.wq_8x8_matrix.0)?;
                }
            }
        }
        if seq.alf_enable_flag {
            h.alf_param = parse_alf(b)?;
        }
        if h.slice_type != SliceType::I && seq.affine_enable_flag {
            h.affine_subblock_size_idx = b.read1()?;
        }
        b.read1()?;
        b.align();
        Ok(h)
    }
}

/// Parses the extension and user data after a header; the picture MD5 signature (user data
/// 0x10) lands in `pic`.
pub(crate) fn parse_ext_and_user_data(b: &mut Bits<'_>, pic: Option<&mut PicHeader>) {
    let mut pic = pic;
    // The data is optional and only informative: parse errors end it quietly.
    while !b.at_end() {
        let next = b.peek(32);
        if next == 0x1B2 {
            if user_data(b, pic.as_deref_mut()).is_err() {
                return;
            }
        } else if next == 0x1B5 {
            // skip extension data: it holds nothing the decoder needs
            b.skip_bytes(4);
            while !b.at_end() && b.peek(24) != 1 {
                b.skip_bytes(1);
            }
        } else {
            return;
        }
    }
}

fn user_data(b: &mut Bits<'_>, pic: Option<&mut PicHeader>) -> Result<(), Error> {
    let mut pic = pic;
    b.read_range(24, 1, 1)?;
    b.read_range(8, 0xB2, 0xB2)?;
    while !b.at_end() && b.peek(24) != 1 {
        let mut code = b.read(8)?;
        while code != 0xFF {
            if code != 0x10 {
                return Err(Error::Invalid("unknown user data"));
            }
            let mut md5 = [0u8; 16];
            for (i, m) in md5.iter_mut().enumerate() {
                *m = b.read(8)? as u8;
                if i % 2 == 1 {
                    b.read1()?;
                }
            }
            if let Some(p) = pic.as_deref_mut() {
                p.pic_md5 = md5;
                p.pic_md5_exist = true;
            }
            code = b.read(8)?;
        }
    }
    Ok(())
}

/// A patch (slice) header.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PatchHeader {
    pub index: usize,
    pub fixed_slice_qp_flag: bool,
    pub slice_qp: i32,
    pub slice_sao_enable: [bool; 3],
}

impl PatchHeader {
    /// `dec_parse_patch_header`; `b` is positioned at the `00 00 01 idx` start code and ends at the
    /// first byte of the slice data.
    pub(crate) fn parse(b: &mut Bits<'_>, seq: &Sequence, ph: &PicHeader) -> Result<Self, Error> {
        let mut h = PatchHeader::default();
        b.read_range(24, 1, 1)?;
        h.index = b.read_range(8, 0, 0x8E)? as usize;
        if !ph.fixed_picture_qp_flag {
            h.fixed_slice_qp_flag = b.flag()?;
            h.slice_qp = b.read_range(7, 0, (MAX_QUANT_BASE + seq.bit_depth_2_qp_offset) as u32)? as i32;
        } else {
            h.fixed_slice_qp_flag = true;
            h.slice_qp = ph.picture_qp;
        }
        if seq.sao_enable_flag {
            for f in h.slice_sao_enable.iter_mut() {
                *f = b.flag()?;
            }
        }
        b.align();
        Ok(h)
    }
}
