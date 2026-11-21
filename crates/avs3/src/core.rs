// SPDX-License-Identifier: BSD-3-Clause
// Derived from uavs3d (`source/decore/com_type.h`), Copyright (c) 2018-2022 Peking University Shenzhen Graduate
// School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation. See LICENSE.uavs3d.
//! The picture buffers and the state of the picture being decoded.

use std::sync::Arc;

use crate::header::{PatchHeader, PicHeader, Sequence, SliceType};
use crate::lbac::{Ctx, Lbac};
use crate::types::*;

/// A plane of samples.
#[derive(Clone, Debug, Default)]
pub(crate) struct Plane {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u16>,
}

impl Plane {
    pub(crate) fn new(w: usize, h: usize) -> Self {
        Self { w, h, data: vec![0; w * h] }
    }
}

/// A decoded (or being decoded) picture with the motion kept for later pictures.
#[derive(Clone, Debug, Default)]
#[allow(dead_code)] // bookkeeping fields kept for the DPB in uavs3d order
pub(crate) struct Picture {
    pub planes: [Plane; 3],
    /// Motion of the 4x4 blocks, with a one block border (`i_scu` stride, offset `i_scu + 1`).
    pub map_mv: Vec<[Mv; 2]>,
    pub map_refi: Vec<[i8; 2]>,
    /// Distance to each reference picture of list 0 (for temporal scaling).
    pub list_dist: [i16; 17],
    pub ptr: i64,
    pub doi: i64,
    pub output_delay: i64,
    pub is_ref: bool,
    pub is_output: bool,
    pub tag: i64,
    pub slice_type: SliceType,
}

/// A reference picture of the current picture.
#[derive(Clone)]
pub(crate) struct RefPic {
    pub pic: Arc<Picture>,
    pub dist: i32,
}

/// SAO parameters of one LCU and component.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SaoParam {
    pub enable: bool,
    pub typ: u32,
    pub band_idx: [i32; 4],
    pub offset: [i32; 5],
}

/// The maps over the 4x4 blocks of the picture being decoded.
pub(crate) struct Maps {
    pub i_scu: usize,
    pub scu: Vec<Scu>,
    pub pos: Vec<u32>,
    pub edge: Vec<u8>,
    pub ipm: Vec<i8>,
    pub patch: Vec<i8>,
    pub qp: Vec<i8>,
    pub mv: Vec<[Mv; 2]>,
    pub refi: Vec<[i8; 2]>,
}

impl Maps {
    pub(crate) fn new(seq: &Sequence) -> Self {
        Self {
            i_scu: seq.i_scu,
            scu: vec![Scu::default(); seq.f_scu],
            pos: vec![0; seq.f_scu],
            edge: vec![0; seq.f_scu],
            ipm: vec![0; seq.f_scu],
            patch: vec![0; seq.f_lcu],
            qp: vec![0; seq.f_lcu],
            mv: vec![[[0; 2]; 2]; seq.f_scu],
            refi: vec![[REFI_INVALID; 2]; seq.f_scu],
        }
    }

    /// Index of the block at `scup` (position `y * i_scu + x`, which may point into the border).
    #[inline]
    pub(crate) fn ix(&self, scup: i32) -> usize {
        (scup + self.i_scu as i32 + 1) as usize
    }
}

pub(crate) fn mcu_set_scup(m: &mut u32, v: u32) {
    *m = (*m & 0x3F) | (v << 6);
}
pub(crate) fn mcu_get_scup(m: u32) -> i32 {
    (m >> 6) as i32
}
pub(crate) fn mcu_set_logw(m: &mut u32, v: u32) {
    *m = (*m & !0x07) | (v & 0x07);
}
pub(crate) fn mcu_set_logh(m: &mut u32, v: u32) {
    *m = (*m & !0x38) | ((v & 0x07) << 3);
}
pub(crate) fn mcu_get_logw(m: u32) -> u32 {
    m & 0x07
}
pub(crate) fn mcu_get_logh(m: u32) -> u32 {
    (m >> 3) & 0x07
}

/// The state of one picture decode (`com_core_t`).
pub(crate) struct Core {
    pub seq: Sequence,
    pub pichdr: PicHeader,
    pub pathdr: PatchHeader,
    pub lbac: Lbac,
    pub ctx: Ctx,
    pub pic: Picture,
    pub maps: Maps,
    pub slice_type: SliceType,
    pub num_refp: [usize; 2],
    pub refp: Vec<[Option<RefPic>; 2]>,

    // current CU
    pub cu_mode: u8,
    pub pb_part: u8,
    pub tb_part: u8,
    pub cbfy: [u8; 4],
    pub cbfc: [u8; 2],
    pub refi: [i8; 2],
    pub ipm: [i8; 4],
    pub ipm_c: u8,
    pub ipf_flag: bool,
    pub affine_flag: u8,
    pub mv: [Mv; 2],
    pub affine_mv: [[[i32; 2]; 4]; 2],
    pub affine_sb_mv: [[[i32; 2]; 256]; 2],
    pub cu_scup: i32,
    pub cu_pix_x: usize,
    pub cu_pix_y: usize,
    pub cu_log2w: u32,
    pub cu_log2h: u32,
    pub cu_width: usize,
    pub cu_height: usize,
    pub tree_scup: i32,
    pub tree_status: u8,
    pub cons_pred_mode: u8,

    // current LCU
    pub lcu_x: usize,
    pub lcu_y: usize,
    pub lcu_pix_x: usize,
    pub lcu_pix_y: usize,
    pub lcu_qp_y: i32,
    pub lcu_qp_u: i32,
    pub lcu_qp_v: i32,

    pub hmvp_cands_cnt: usize,
    pub hmvp_cands_list: [Motion; ALLOWED_HMVP_NUM],

    pub sao_param_map: Vec<[SaoParam; 3]>,
    pub alf_enable_map: Vec<[bool; 3]>,
}

impl Core {
    pub(crate) fn refp_dist(&self, refi: i8, lidx: usize) -> i32 {
        self.refp.get(refi.max(0) as usize).and_then(|r| r[lidx].as_ref()).map_or(1, |r| r.dist)
    }
}
