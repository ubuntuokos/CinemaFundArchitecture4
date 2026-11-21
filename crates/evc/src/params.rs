// SPDX-License-Identifier: GPL-3.0-or-later AND BSD-3-Clause
//! NAL unit header, sequence/picture parameter sets and slice header of the
//! EVC Baseline profile (`xevd_eco.c`).

use crate::Error;
use crate::bits::Bits;

pub(crate) const NUT_IDR: i32 = 1;
pub(crate) const NUT_SPS: i32 = 24;
pub(crate) const NUT_PPS: i32 = 25;
pub(crate) const NUT_SEI: i32 = 28;

pub(crate) const SLICE_B: u32 = 0;
pub(crate) const SLICE_P: u32 = 1;
pub(crate) const SLICE_I: u32 = 2;

/// MaxLumaPs of level 6.2 (Table A.1).
const MAX_LUMA_PS: u64 = 35_651_584;

/// Tile counts above the limits of the reference decoder are hostile.
const MAX_TILES: u64 = 22 * 20;

#[derive(Clone, Copy, Debug)]
pub(crate) struct NalHeader {
    /// `nal_unit_type_plus1 - 1`; -1 is possible and decoded as a slice, as the
    /// reference decoder does.
    pub(crate) nal_type: i32,
    pub(crate) temporal_id: u8,
}

pub(crate) fn nal_header(bs: &mut Bits<'_>) -> Result<NalHeader, Error> {
    if bs.bit() != 0 {
        return Err(Error::Malformed("forbidden_zero_bit is set"));
    }
    let plus1 = bs.bits(6) as i32;
    let tid = bs.bits(3) as u8;
    if bs.bits(5) != 0 {
        return Err(Error::Malformed("nuh_reserved_zero_5bits is not zero"));
    }
    if bs.bit() != 0 {
        return Err(Error::Malformed("nuh_extension_flag is set"));
    }
    if bs.overrun {
        return Err(Error::Truncated);
    }
    Ok(NalHeader { nal_type: plus1 - 1, temporal_id: tid })
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ChromaQpTable {
    pub(crate) present: bool,
    pub(crate) same_for_chroma: bool,
    pub(crate) global_offset: bool,
    pub(crate) num_points_minus1: [u32; 2],
    pub(crate) delta_in_minus1: [Vec<i32>; 2],
    pub(crate) delta_out: [Vec<i32>; 2],
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Sps {
    pub(crate) profile_idc: u32,
    pub(crate) level_idc: u32,
    pub(crate) chroma_format_idc: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) bit_depth_luma: u32,
    pub(crate) bit_depth_chroma: u32,
    pub(crate) log2_sub_gop_length: u32,
    pub(crate) log2_ref_pic_gap_length: u32,
    pub(crate) max_num_ref_pics: u32,
    /// Crop offsets in chroma units (left, right, top, bottom).
    pub(crate) crop: Option<[u32; 4]>,
    pub(crate) chroma_qp: ChromaQpTable,
    pub(crate) sar: Option<(u32, u32)>,
    pub(crate) timing: Option<(u32, u32)>,
    pub(crate) num_reorder_pics: Option<u32>,
}

fn hrd(bs: &mut Bits<'_>) -> Result<(), Error> {
    let cpb_cnt_minus1 = bs.ue();
    if cpb_cnt_minus1 > 31 {
        return Err(Error::Malformed("cpb_cnt_minus1 out of range"));
    }
    bs.bits(8);
    for _ in 0..=cpb_cnt_minus1 {
        bs.ue();
        bs.ue();
        bs.bit();
    }
    bs.bits(20);
    Ok(())
}

/// Table E-1 sample aspect ratios (`aspect_ratio_idc` 1..16).
const SAR: [(u32, u32); 17] = [
    (0, 0),
    (1, 1),
    (12, 11),
    (10, 11),
    (16, 11),
    (40, 33),
    (24, 11),
    (20, 11),
    (32, 11),
    (80, 33),
    (18, 11),
    (15, 11),
    (64, 33),
    (160, 99),
    (4, 3),
    (3, 2),
    (2, 1),
];

fn vui(bs: &mut Bits<'_>, sps: &mut Sps) -> Result<(), Error> {
    if bs.flag() {
        let idc = bs.bits(8);
        if idc == 255 {
            let w = bs.bits(16);
            let h = bs.bits(16);
            sps.sar = Some((w, h));
        } else {
            sps.sar = SAR.get(idc as usize).copied().filter(|&(w, h)| w > 0 && h > 0);
        }
    }
    if bs.flag() {
        bs.bit();
    }
    if bs.flag() {
        bs.bits(4);
        if bs.flag() {
            bs.bits(24);
        }
    }
    if bs.flag() {
        bs.ue();
        bs.ue();
    }
    bs.bit();
    bs.bit();
    if bs.flag() {
        let units = bs.bits(32);
        let scale = bs.bits(32);
        bs.bit();
        sps.timing = Some((units, scale));
    }
    let nal_hrd = bs.flag();
    if nal_hrd {
        hrd(bs)?;
    }
    let vcl_hrd = bs.flag();
    if vcl_hrd {
        hrd(bs)?;
    }
    if nal_hrd || vcl_hrd {
        bs.bit();
    }
    bs.bit();
    if bs.flag() {
        bs.bit();
        for _ in 0..4 {
            bs.ue();
        }
        sps.num_reorder_pics = Some(bs.ue());
        bs.ue();
    }
    Ok(())
}

pub(crate) fn sps(bs: &mut Bits<'_>) -> Result<Sps, Error> {
    let mut s = Sps::default();
    bs.ue();
    s.profile_idc = bs.bits(8);
    if s.profile_idc != 0 && s.profile_idc != 2 {
        return Err(Error::Unsupported("only the Baseline profiles are supported"));
    }
    s.level_idc = bs.bits(8);
    bs.bits(32);
    bs.bits(32);
    s.chroma_format_idc = bs.ue();
    s.width = bs.ue();
    s.height = bs.ue();
    s.bit_depth_luma = bs.ue().saturating_add(8);
    s.bit_depth_chroma = bs.ue().saturating_add(8);
    // sps_btt_flag, sps_suco_flag and the Main-profile tool flags. The
    // reference Baseline decoder reads and ignores them; so does this one.
    let mut tools = [false; 13];
    for t in &mut tools {
        *t = bs.flag();
    }
    let tool_rpl = tools[9];
    let tool_pocs = tools[10];
    if !tool_rpl || !tool_pocs {
        s.log2_sub_gop_length = bs.ue();
        if s.log2_sub_gop_length == 0 {
            s.log2_ref_pic_gap_length = bs.ue();
        }
    }
    if !tool_rpl {
        s.max_num_ref_pics = bs.ue();
    }
    if bs.flag() {
        s.crop = Some([bs.ue(), bs.ue(), bs.ue(), bs.ue()]);
    }
    if s.chroma_format_idc != 0 {
        let t = &mut s.chroma_qp;
        t.present = bs.flag();
        if t.present {
            t.same_for_chroma = bs.flag();
            t.global_offset = bs.flag();
            for i in 0..if t.same_for_chroma { 1 } else { 2 } {
                let n = bs.ue();
                if n > 57 {
                    return Err(Error::Malformed("too many chroma QP table points"));
                }
                t.num_points_minus1[i] = n;
                for _ in 0..=n {
                    t.delta_in_minus1[i].push(bs.bits(6) as i32);
                    t.delta_out[i].push(bs.se());
                }
            }
        }
    }
    if bs.flag() {
        vui(bs, &mut s)?;
    }
    if bs.overrun {
        return Err(Error::Truncated);
    }
    if s.chroma_format_idc > 1 {
        return Err(Error::Unsupported("only 4:0:0 and 4:2:0 are supported"));
    }
    if !(8..=12).contains(&s.bit_depth_luma) || !(8..=12).contains(&s.bit_depth_chroma) {
        return Err(Error::Unsupported("bit depth outside 8..12"));
    }
    if s.width == 0 || s.height == 0 || s.width > 16384 || s.height > 16384 {
        return Err(Error::Malformed("picture size out of range"));
    }
    // MaxLumaPs of the highest level (6.2) caps the picture allocations.
    if u64::from(s.width) * u64::from(s.height) > MAX_LUMA_PS {
        return Err(Error::Unsupported("picture larger than level 6.2 allows"));
    }
    if s.log2_sub_gop_length > 8 || s.log2_ref_pic_gap_length > 16 || s.max_num_ref_pics > 21 {
        return Err(Error::Malformed("reference structure out of range"));
    }
    if let Some(c) = s.crop {
        // Like xevd, luma offsets are twice the coded ones, also in 4:0:0,
        // and apply to the size aligned to 4.
        let horiz = (u64::from(c[0]) + u64::from(c[1])) * 2;
        let vert = (u64::from(c[2]) + u64::from(c[3])) * 2;
        if horiz >= u64::from(s.width.div_ceil(4) * 4) || vert >= u64::from(s.height.div_ceil(4) * 4) {
            return Err(Error::Malformed("cropping removes the whole picture"));
        }
    }
    Ok(s)
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Pps {
    pub(crate) num_ref_idx_default_active_minus1: [u32; 2],
    pub(crate) single_tile_in_pic_flag: bool,
    pub(crate) tile_id_len_minus1: u32,
    pub(crate) arbitrary_slice_present_flag: bool,
    pub(crate) constrained_intra_pred_flag: bool,
    pub(crate) cu_qp_delta_enabled_flag: bool,
}

pub(crate) fn pps(bs: &mut Bits<'_>) -> Result<Pps, Error> {
    let mut p = Pps::default();
    let id = bs.ue();
    if id >= 64 {
        return Err(Error::Malformed("pps_pic_parameter_set_id out of range"));
    }
    bs.ue();
    p.num_ref_idx_default_active_minus1 = [bs.ue(), bs.ue()];
    bs.ue();
    bs.bit();
    p.single_tile_in_pic_flag = bs.flag();
    p.tile_id_len_minus1 = bs.ue();
    bs.bit();
    bs.bit();
    p.arbitrary_slice_present_flag = bs.flag();
    p.constrained_intra_pred_flag = bs.flag();
    p.cu_qp_delta_enabled_flag = bs.flag();
    if p.cu_qp_delta_enabled_flag {
        bs.ue();
    }
    if bs.overrun {
        return Err(Error::Truncated);
    }
    if p.tile_id_len_minus1 > 31 {
        return Err(Error::Malformed("tile_id_len_minus1 out of range"));
    }
    Ok(p)
}

/// Slice header. Like the reference decoder's context, one instance lives for
/// the whole stream, so flags that a slice does not code keep their value.
#[derive(Clone, Debug, Default)]
pub(crate) struct Sh {
    pub(crate) single_tile_in_slice_flag: bool,
    pub(crate) first_tile_id: u32,
    pub(crate) last_tile_id: u32,
    pub(crate) arbitrary_slice_flag: bool,
    pub(crate) num_remaining_tiles_in_slice_minus1: u32,
    pub(crate) slice_type: u32,
    pub(crate) deblocking_filter_on: bool,
    pub(crate) qp: i32,
    pub(crate) qp_u_offset: i32,
    pub(crate) qp_v_offset: i32,
    pub(crate) qp_u: i32,
    pub(crate) qp_v: i32,
}

pub(crate) fn slice_header(bs: &mut Bits<'_>, sps: &Sps, pps: &Pps, sh: &mut Sh, nut: i32) -> Result<(), Error> {
    if bs.ue() >= 64 {
        return Err(Error::Malformed("slice_pic_parameter_set_id out of range"));
    }
    let id_len = pps.tile_id_len_minus1 + 1;
    if !pps.single_tile_in_pic_flag {
        sh.single_tile_in_slice_flag = bs.flag();
        sh.first_tile_id = bs.bits(id_len);
    } else {
        sh.single_tile_in_slice_flag = true;
    }
    if !sh.single_tile_in_slice_flag {
        if pps.arbitrary_slice_present_flag {
            sh.arbitrary_slice_flag = bs.flag();
        }
        if !sh.arbitrary_slice_flag {
            sh.last_tile_id = bs.bits(id_len);
        } else {
            sh.num_remaining_tiles_in_slice_minus1 = bs.ue();
            let n = u64::from(sh.num_remaining_tiles_in_slice_minus1) + 2;
            if n > MAX_TILES {
                return Err(Error::Malformed("too many tiles in the slice"));
            }
            for _ in 0..n - 1 {
                bs.ue();
            }
        }
    }
    sh.slice_type = bs.ue();
    if sh.slice_type > SLICE_I {
        return Err(Error::Malformed("unknown slice type"));
    }
    // The PPS of the Baseline profile has one tile column and one tile row.
    let num_tiles_in_slice: u64 = if !sh.arbitrary_slice_flag {
        let first = i64::from(sh.first_tile_id);
        let last = i64::from(sh.last_tile_id);
        let mut delta = last - first;
        if last < first {
            delta += 1;
        }
        (delta + 1).max(1) as u64
    } else {
        u64::from(sh.num_remaining_tiles_in_slice_minus1) + 2
    };
    if num_tiles_in_slice > MAX_TILES {
        return Err(Error::Malformed("too many tiles in the slice"));
    }
    if nut == NUT_IDR {
        bs.bit();
    }
    if sh.slice_type != SLICE_I && bs.flag() {
        bs.ue();
        if sh.slice_type == SLICE_B {
            bs.ue();
        }
    }
    let _ = pps.num_ref_idx_default_active_minus1;
    sh.deblocking_filter_on = bs.flag();
    sh.qp = bs.bits(6) as i32;
    if sh.qp > 51 {
        return Err(Error::Malformed("slice_qp out of range"));
    }
    sh.qp_u_offset = bs.se();
    sh.qp_v_offset = bs.se();
    let lo = -6 * (sps.bit_depth_luma as i32 - 8);
    sh.qp_u = sh.qp.saturating_add(sh.qp_u_offset).clamp(lo, 57);
    sh.qp_v = sh.qp.saturating_add(sh.qp_v_offset).clamp(lo, 57);
    if !sh.single_tile_in_slice_flag {
        // entry_point_offset_minus1 with tile_offset_lens_minus1 == 0.
        for _ in 1..num_tiles_in_slice {
            bs.bit();
        }
    }
    while !bs.is_byte_aligned() {
        if bs.bit() != 0 {
            return Err(Error::Malformed("slice header alignment bits are not zero"));
        }
    }
    if bs.overrun {
        return Err(Error::Truncated);
    }
    Ok(())
}
