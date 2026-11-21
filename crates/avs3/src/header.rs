// SPDX-License-Identifier: GPL-3.0-or-later
//! Sequence, picture and patch headers.
//!
//! Derived from uavs3d 1.2 (commit 0e20d2c2), Copyright (c) 2018-2022 Peking University
//! Shenzhen Graduate School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation
//! Corporation, BSD 3-clause licence (docs/licensing.md).

use crate::Error;
use crate::bits::Bits;
use crate::tables::{WQ_DEFAULT_4X4, WQ_DEFAULT_8X8, WQ_DEFAULT_PARAM, WQ_MODEL_4X4, WQ_MODEL_8X8};

pub(crate) const MAX_REFS: usize = 17;
const PIC_ALIGN: u32 = 8;
const FRAME_RATES: [(u32, u32); 13] =
    [(24000, 1001), (24, 1), (25, 1), (30000, 1001), (30, 1), (50, 1), (60000, 1001), (60, 1), (100, 1), (120, 1), (200, 1), (240, 1), (300, 1)];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SliceType {
    #[default]
    I,
    P,
    B,
}

/// A reference picture list description.
#[derive(Clone, Debug, Default)]
pub(crate) struct Rpl {
    pub num: usize,
    pub active: usize,
    pub delta_doi: [i32; MAX_REFS],
}

fn parse_rpl(b: &mut Bits<'_>, max_dpb: u32) -> Result<Rpl, Error> {
    let mut r = Rpl { num: b.uer(0, max_dpb)? as usize, ..Rpl::default() };
    if r.num > 0 {
        let v = b.uer(1, 255)? as i32;
        r.delta_doi[0] = if b.flag()? { -v } else { v };
    }
    for i in 1..r.num.min(MAX_REFS) {
        let mut d = b.uer(0, 255)? as i32;
        if d != 0 && b.flag()? {
            d = -d;
        }
        r.delta_doi[i] = r.delta_doi[i - 1].wrapping_add(d);
    }
    Ok(r)
}

fn read_wq(b: &mut Bits<'_>, m4: &mut [u8; 16], m8: &mut Vec<u8>) -> Result<(), Error> {
    m8.resize(64, 0);
    for v in m4.iter_mut() {
        *v = b.uer(1, 255)? as u8;
    }
    for v in m8.iter_mut() {
        *v = b.uer(1, 255)? as u8;
    }
    Ok(())
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Sequence {
    pub profile_id: u32,
    pub level_id: u32,
    pub progressive_sequence: bool,
    pub field_coded_sequence: bool,
    pub horizontal_size: u32,
    pub vertical_size: u32,
    pub display_horizontal_size: u32,
    pub display_vertical_size: u32,
    pub sample_precision: u32,
    pub encoding_precision: u32,
    pub aspect_ratio: u32,
    pub frame_rate_code: u32,
    pub low_delay: bool,
    pub temporal_id_enable: bool,
    pub max_dpb_size: u32,
    pub rpl1_index_exist: bool,
    pub rpl1_same_as_rpl0: bool,
    pub rpls_l0: Vec<Rpl>,
    pub rpls_l1: Vec<Rpl>,
    pub num_ref_default_active_minus1: [u32; 2],
    pub log2_max_cu: u32,
    pub min_cu_size: u32,
    pub max_part_ratio_log2: u32,
    pub max_split_times: u32,
    pub min_qt_size: u32,
    pub max_bt_size: u32,
    pub max_eqt_size: u32,
    pub wq_enable: bool,
    pub wq_4x4: [u8; 16],
    pub wq_8x8: Vec<u8>,
    pub secondary_transform: bool,
    pub sao_enable: bool,
    pub alf_enable: bool,
    pub affine_enable: bool,
    pub smvd_enable: bool,
    pub ipcm_enable: bool,
    pub amvr_enable: bool,
    pub num_hmvp_cand: u32,
    pub umve_enable: bool,
    pub emvr_enable: bool,
    pub ipf_enable: bool,
    pub tscpm_enable: bool,
    pub dt_intra_enable: bool,
    pub max_dt_size: u32,
    pub pbt_enable: bool,
    pub output_reorder_delay: u32,
    pub cross_patch_loop_filter: bool,
    pub patch_ref_colocated: bool,
    pub patch_height: u32,
    // derived
    pub bit_depth: u32,
    pub bit_depth_2_qp_offset: i32,
    pub pic_width: u32,
    pub pic_height: u32,
    pub max_cu: u32,
    pub w_lcu: u32,
    pub h_lcu: u32,
    pub w_scu: u32,
    pub h_scu: u32,
}

impl Sequence {
    /// Parses a sequence header; `data` starts after the `00 00 01 B0` start code and is
    /// already free of emulation-prevention bytes.
    pub(crate) fn parse(data: &[u8]) -> Result<Sequence, Error> {
        let mut b = Bits::new(data);
        let mut s = Sequence { profile_id: b.u(8)?, ..Sequence::default() };
        if s.profile_id != 0x20 && s.profile_id != 0x22 {
            return Err(Error::Invalid("not an AVS3 profile"));
        }
        s.level_id = b.u(8)?;
        s.progressive_sequence = b.flag()?;
        s.field_coded_sequence = b.flag()?;
        b.ur(1, 0, 0)?; // library_stream_flag
        b.ur(1, 0, 0)?; // library_picture_enable_flag
        b.marker()?;
        s.horizontal_size = b.u(14)?;
        b.marker()?;
        s.vertical_size = b.u(14)?;
        if s.horizontal_size == 0 || s.vertical_size == 0 {
            return Err(Error::Invalid("zero picture size"));
        }
        s.display_horizontal_size = s.horizontal_size;
        s.display_vertical_size = s.vertical_size;
        b.ur(2, 1, 1)?; // chroma_format: 4:2:0
        s.sample_precision = b.ur(3, 1, 2)?;
        s.encoding_precision = if s.profile_id == 0x22 { b.ur(3, 1, 2)? } else { 1 };
        b.marker()?;
        s.aspect_ratio = b.ur(4, 1, 4)?;
        s.frame_rate_code = b.ur(4, 1, 13)?;
        b.marker()?;
        b.u(18)?; // bit_rate_lower
        b.marker()?;
        b.u(12)?; // bit_rate_upper
        s.low_delay = b.flag()?;
        s.temporal_id_enable = b.flag()?;
        b.marker()?;
        b.u(18)?; // bbv_buffer_size
        b.marker()?;
        s.max_dpb_size = b.ur(4, 0, 15)? + 1;
        s.rpl1_index_exist = b.flag()?;
        s.rpl1_same_as_rpl0 = b.flag()?;
        b.marker()?;
        let n0 = b.uer(0, 64)?;
        for _ in 0..n0 {
            s.rpls_l0.push(parse_rpl(&mut b, s.max_dpb_size)?);
        }
        if s.rpl1_same_as_rpl0 {
            s.rpls_l1 = s.rpls_l0.clone();
        } else {
            let n1 = b.uer(0, 64)?;
            for _ in 0..n1 {
                s.rpls_l1.push(parse_rpl(&mut b, s.max_dpb_size)?);
            }
        }
        s.num_ref_default_active_minus1 = [b.uer(0, 14)?, b.uer(0, 14)?];
        s.log2_max_cu = b.ur(3, 3, 5)? + 2;
        s.min_cu_size = 1 << (b.ur(2, 0, 0)? + 2);
        s.max_part_ratio_log2 = b.ur(2, 0, 3)? + 2;
        s.max_split_times = b.ur(3, 0, 7)? + 6;
        s.min_qt_size = 1 << (b.ur(3, 0, 5)? + 2);
        s.max_bt_size = 1 << (b.ur(3, 0, 5)? + 2);
        s.max_eqt_size = 1 << (b.ur(2, 0, 3)? + 3);
        b.marker()?;
        s.wq_enable = b.flag()?;
        if s.wq_enable {
            if b.flag()? {
                read_wq(&mut b, &mut s.wq_4x4, &mut s.wq_8x8)?;
            } else {
                s.wq_4x4 = WQ_DEFAULT_4X4;
                s.wq_8x8 = WQ_DEFAULT_8X8.to_vec();
            }
        }
        s.secondary_transform = b.flag()?;
        s.sao_enable = b.flag()?;
        s.alf_enable = b.flag()?;
        s.affine_enable = b.flag()?;
        s.smvd_enable = b.flag()?;
        s.ipcm_enable = b.flag()?;
        s.amvr_enable = b.flag()?;
        s.num_hmvp_cand = b.ur(4, 0, 8)?;
        s.umve_enable = b.flag()?;
        s.emvr_enable = if s.amvr_enable && s.num_hmvp_cand > 0 { b.flag()? } else { false };
        s.ipf_enable = b.flag()?;
        s.tscpm_enable = b.flag()?;
        b.marker()?;
        s.dt_intra_enable = b.flag()?;
        if s.dt_intra_enable {
            s.max_dt_size = 1 << (b.ur(2, 0, 2)? + 4);
        }
        s.pbt_enable = b.flag()?;
        s.output_reorder_delay = if s.low_delay { 0 } else { b.u(5)? };
        s.cross_patch_loop_filter = b.flag()?;
        s.patch_ref_colocated = b.flag()?;
        b.ur(1, 1, 1)?; // patch_stable
        b.ur(1, 1, 1)?; // patch_uniform
        b.marker()?;
        b.uer(0, 255)?; // patch_width: replaced by the picture width in LCUs
        s.patch_height = b.uer(0, 143)? + 1;
        b.u(2)?; // reserved
        b.marker()?;

        s.bit_depth = if s.encoding_precision == 2 { 10 } else { 8 };
        s.bit_depth_2_qp_offset = 8 * (s.bit_depth as i32 - 8);
        s.pic_width = s.horizontal_size.div_ceil(PIC_ALIGN) * PIC_ALIGN;
        s.pic_height = s.vertical_size.div_ceil(PIC_ALIGN) * PIC_ALIGN;
        s.max_cu = 1 << s.log2_max_cu;
        s.w_lcu = s.pic_width.div_ceil(s.max_cu);
        s.h_lcu = s.pic_height.div_ceil(s.max_cu);
        s.w_scu = s.pic_width.div_ceil(4);
        s.h_scu = s.pic_height.div_ceil(4);
        s.patch_height = s.patch_height.min(s.h_lcu);
        Ok(s)
    }

    /// Sample precision of the output (8 or 10).
    pub(crate) fn output_bit_depth(&self) -> u32 {
        if self.sample_precision == 1 { 8 } else { 10 }
    }

    pub(crate) fn frame_rate(&self) -> (u32, u32) {
        FRAME_RATES.get(self.frame_rate_code.saturating_sub(1) as usize).copied().unwrap_or((25, 1))
    }

    /// Sizes of the patch rows in LCUs.
    pub(crate) fn patch_rows(&self) -> Vec<u32> {
        let ph = self.patch_height.max(1);
        let mut v = vec![ph; (self.h_lcu / ph) as usize];
        let rem = self.h_lcu % ph;
        if rem != 0 {
            match v.last_mut() {
                Some(l) => *l += rem,
                None => v.push(rem),
            }
        }
        v
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct AlfParam {
    pub enable: bool,
    pub flag: [bool; 3],
    pub var_ind_tab: [i32; 16],
    pub coeff_luma: [[i32; 9]; 16],
    pub coeff_chroma: [i32; 18],
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Picture {
    pub slice_type: SliceType,
    pub random_access_decodable: bool,
    pub doi: u32,
    pub temporal_id: u32,
    pub output_delay: u32,
    pub progressive_frame: bool,
    pub picture_structure: bool,
    pub rpl_l0: Rpl,
    pub rpl_l1: Rpl,
    pub fixed_picture_qp: bool,
    pub picture_qp: i32,
    pub loop_filter_disable: bool,
    pub alpha_c_offset: i32,
    pub beta_offset: i32,
    pub chroma_qp_delta_cb: i32,
    pub chroma_qp_delta_cr: i32,
    pub pic_wq_enable: bool,
    pub wq_4x4: [u8; 16],
    pub wq_8x8: Vec<u8>,
    pub alf: AlfParam,
    pub affine_subblock_size_idx: u32,
    /// The picture's order count.
    pub ptr: i64,
}

fn parse_alf(b: &mut Bits<'_>, a: &mut AlfParam) -> Result<(), Error> {
    for f in a.flag.iter_mut() {
        *f = b.flag()?;
    }
    a.enable = a.flag.iter().any(|&f| f);
    if a.flag[0] {
        let groups = b.uer(0, 15)? as usize + 1;
        let mut pattern = [false; 16];
        let mut pre = 0usize;
        for g in 0..groups {
            let mut sum = 0i32;
            if g > 0 {
                let sym = if groups != 16 { b.uer(1, 15)? as usize } else { 1 };
                let sym = (sym + pre).min(15);
                pattern[sym] = true;
                pre = sym;
            }
            for i in 0..8 {
                let c = b.ser(-64, 63)?;
                a.coeff_luma[g][i] = c;
                sum += 2 * c;
            }
            let lac = b.ser(-1088, 1071)?;
            a.coeff_luma[g][8] = (1 << 6) - sum + lac;
        }
        if groups > 1 {
            a.var_ind_tab[0] = 0;
            for g in 1..16 {
                a.var_ind_tab[g] = a.var_ind_tab[g - 1] + i32::from(pattern[g]);
            }
        } else {
            a.var_ind_tab = [0; 16];
        }
    }
    for (c, start) in [(1usize, 0usize), (2, 1)] {
        if a.flag[c] {
            let mut sum = 0;
            for k in 0..8 {
                let v = b.ser(-64, 63)?;
                a.coeff_chroma[start + 2 * k] = v;
                sum += 2 * v;
            }
            let lac = b.ser(-1088, 1071)?;
            a.coeff_chroma[16 + start] = (1 << 6) - sum + lac;
        }
    }
    Ok(())
}

/// Per-stream picture-order bookkeeping that header parsing updates.
#[derive(Clone, Debug, Default)]
pub(crate) struct DoiState {
    pub prev_doi: u32,
    pub doi_cycles: i64,
}

impl Picture {
    /// Parses a picture header. `data` starts after the start code byte; `code` is 0xB3 or 0xB6.
    /// Returns the header and whether the DOI counter wrapped (the caller then rebases the
    /// references' DOIs by -256).
    pub(crate) fn parse(data: &[u8], code: u8, seq: &Sequence, st: &mut DoiState) -> Result<(Picture, bool), Error> {
        let mut b = Bits::new(data);
        let mut p = Picture::default();
        if code == 0xB3 {
            p.slice_type = SliceType::I;
        } else {
            p.random_access_decodable = b.flag()?;
        }
        b.u(32)?; // bbv_delay
        if code == 0xB6 {
            p.slice_type = if b.ur(2, 1, 2)? == 1 { SliceType::P } else { SliceType::B };
        }
        if p.slice_type == SliceType::I && b.flag()? {
            b.u(24)?; // time_code
        }
        p.doi = b.u(8)?;
        if seq.temporal_id_enable {
            p.temporal_id = b.u(3)?;
        }
        if !seq.low_delay {
            p.output_delay = b.uer(0, 63)?;
        } else {
            b.ue()?; // bbv_check_times
        }
        p.progressive_frame = b.flag()?;
        p.picture_structure = if p.progressive_frame { true } else { b.flag()? };
        b.flag()?; // top_field_first
        b.flag()?; // repeat_first_field
        if seq.field_coded_sequence {
            b.flag()?;
            b.flag()?;
        }
        let wrapped = p.doi < st.prev_doi;
        if wrapped {
            st.doi_cycles += 1;
        }
        p.ptr = i64::from(p.doi) + 256 * st.doi_cycles + i64::from(p.output_delay) - i64::from(seq.output_reorder_delay);
        st.prev_doi = p.doi;

        let sps0 = b.flag()?;
        let mut l0_idx = 0usize;
        if sps0 {
            if !seq.rpls_l0.is_empty() {
                if seq.rpls_l0.len() > 1 {
                    l0_idx = b.uer(0, seq.rpls_l0.len() as u32 - 1)? as usize;
                }
                p.rpl_l0 = seq.rpls_l0.get(l0_idx).cloned().unwrap_or_default();
            }
        } else {
            p.rpl_l0 = parse_rpl(&mut b, seq.max_dpb_size)?;
        }
        let sps1 = if seq.rpl1_index_exist { b.flag()? } else { sps0 };
        if sps1 {
            let idx = if seq.rpls_l1.len() > 1 && seq.rpl1_index_exist {
                b.uer(0, seq.rpls_l1.len() as u32 - 1)? as usize
            } else if !seq.rpl1_index_exist {
                l0_idx
            } else {
                0
            };
            p.rpl_l1 = seq.rpls_l1.get(idx).cloned().unwrap_or_default();
        } else {
            p.rpl_l1 = parse_rpl(&mut b, seq.max_dpb_size)?;
        }
        if p.slice_type != SliceType::I {
            if b.flag()? {
                p.rpl_l0.active = b.uer(0, 14)? as usize + 1;
                if p.slice_type == SliceType::B {
                    p.rpl_l1.active = b.uer(0, 14)? as usize + 1;
                }
            } else {
                p.rpl_l0.active = seq.num_ref_default_active_minus1[0] as usize + 1;
                p.rpl_l1.active = if p.slice_type == SliceType::P { 0 } else { seq.num_ref_default_active_minus1[1] as usize + 1 };
            }
        } else {
            p.rpl_l0.active = 0;
            p.rpl_l1.active = 0;
        }
        p.fixed_picture_qp = b.flag()?;
        p.picture_qp = b.ur(7, 0, (63 + seq.bit_depth_2_qp_offset) as u32)? as i32;
        if p.slice_type != SliceType::I && !(p.slice_type == SliceType::B && p.picture_structure) {
            b.flag()?;
        }
        p.loop_filter_disable = b.flag()?;
        if !p.loop_filter_disable && b.flag()? {
            p.alpha_c_offset = b.ser(-8, 8)?;
            p.beta_offset = b.ser(-8, 8)?;
        }
        if !b.flag()? {
            p.chroma_qp_delta_cb = b.ser(-16, 16)?;
            p.chroma_qp_delta_cr = b.ser(-16, 16)?;
        }
        p.wq_4x4 = [64; 16];
        p.wq_8x8 = vec![64; 64];
        if seq.wq_enable {
            p.pic_wq_enable = b.flag()?;
            if p.pic_wq_enable {
                match b.ur(2, 0, 2)? {
                    0 => {
                        p.wq_4x4 = seq.wq_4x4;
                        p.wq_8x8 = seq.wq_8x8.clone();
                    }
                    1 => {
                        b.flag()?;
                        let param = b.ur(2, 0, 2)? as usize;
                        let model = b.ur(2, 0, 3)? as usize;
                        let mut vec = [0i32; 6];
                        if param == 0 {
                            vec = WQ_DEFAULT_PARAM[1];
                        } else {
                            for (i, v) in vec.iter_mut().enumerate() {
                                let d = b.ser(i32::MIN, i32::MAX)?;
                                *v = d.wrapping_add(WQ_DEFAULT_PARAM[param - 1][i]);
                            }
                        }
                        for i in 0..16 {
                            p.wq_4x4[i] = vec[WQ_MODEL_4X4[model][i] as usize] as u8;
                        }
                        for i in 0..64 {
                            p.wq_8x8[i] = vec[WQ_MODEL_8X8[model][i] as usize] as u8;
                        }
                    }
                    _ => read_wq(&mut b, &mut p.wq_4x4, &mut p.wq_8x8)?,
                }
            }
        }
        if seq.alf_enable {
            parse_alf(&mut b, &mut p.alf)?;
        }
        if p.slice_type != SliceType::I && seq.affine_enable {
            p.affine_subblock_size_idx = u32::from(b.flag()?);
        }
        b.marker()?;
        Ok((p, wrapped))
    }
}

/// A patch header: the QP and the SAO enables of one patch.
#[derive(Clone, Debug, Default)]
pub(crate) struct PatchHeader {
    pub fixed_slice_qp: bool,
    pub slice_qp: i32,
    pub sao_enable: [bool; 3],
    /// Offset of the first payload byte after the (byte-aligned) header.
    pub len: usize,
}

impl PatchHeader {
    /// `data` starts after the patch start code byte (which is the patch index).
    pub(crate) fn parse(_index: u32, data: &[u8], seq: &Sequence, pic: &Picture) -> Result<PatchHeader, Error> {
        let mut b = Bits::new(data);
        let mut h = PatchHeader::default();
        if !pic.fixed_picture_qp {
            h.fixed_slice_qp = b.flag()?;
            h.slice_qp = b.ur(7, 0, (63 + seq.bit_depth_2_qp_offset) as u32)? as i32;
        } else {
            h.fixed_slice_qp = true;
            h.slice_qp = pic.picture_qp;
        }
        if seq.sao_enable {
            for k in 0..3 {
                h.sao_enable[k] = b.flag()?;
            }
        }
        b.align();
        h.len = b.byte_pos();
        Ok(h)
    }
}
