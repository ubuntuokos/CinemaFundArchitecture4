// SPDX-License-Identifier: GPL-3.0-or-later
//! The per-picture decoding state shared by the parsing and reconstruction code.
//!
//! Derived from uavs3d 1.2 (commit 0e20d2c2), Copyright (c) 2018-2022 Peking University
//! Shenzhen Graduate School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation
//! Corporation, BSD 3-clause licence (docs/licensing.md).

use std::sync::Arc;

use crate::header::{MAX_REFS, PatchHeader, Picture, Sequence, SliceType};
use crate::lbac::{Ctx, Lbac};

pub(crate) const MODE_INTRA: u8 = 0;
pub(crate) const MODE_INTER: u8 = 1;
pub(crate) const MODE_SKIP: u8 = 2;
pub(crate) const MODE_DIR: u8 = 3;

pub(crate) const TREE_LC: u8 = 0;
pub(crate) const TREE_L: u8 = 1;
pub(crate) const TREE_C: u8 = 2;

pub(crate) const NO_MODE_CONS: u8 = 0;
pub(crate) const ONLY_INTER: u8 = 1;
pub(crate) const ONLY_INTRA: u8 = 2;

pub(crate) const REFI_INVALID: i8 = -1;
pub(crate) const MV_SCALE_PREC: i32 = 14;
pub(crate) const CPMV_MAX: i32 = (1 << 17) - 1;
pub(crate) const CPMV_MIN: i32 = -(1 << 17);

// SCU flag bits
pub(crate) const SCU_CODED: u8 = 1;
pub(crate) const SCU_INTRA: u8 = 2;
pub(crate) const SCU_INTER: u8 = 4;
pub(crate) const SCU_SKIP: u8 = 8;
pub(crate) const SCU_CBF: u8 = 16;
pub(crate) const SCU_AFFINE_SHIFT: u8 = 5;

pub(crate) type Mv = [i16; 2];

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct Motion {
    pub mv: [Mv; 2],
    pub refi: [i8; 2],
}

impl Motion {
    pub(crate) fn same(&self, o: &Motion) -> bool {
        self.refi == o.refi && (self.refi[0] == REFI_INVALID || self.mv[0] == o.mv[0]) && (self.refi[1] == REFI_INVALID || self.mv[1] == o.mv[1])
    }
}

#[derive(Clone)]
pub(crate) struct Plane {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u16>,
}

impl Plane {
    pub(crate) fn new(w: usize, h: usize) -> Plane {
        Plane { w, h, data: vec![0; w * h] }
    }

    #[inline]
    pub(crate) fn get(&self, x: i32, y: i32) -> i32 {
        let xc = x.clamp(0, self.w as i32 - 1) as usize;
        let yc = y.clamp(0, self.h as i32 - 1) as usize;
        self.data.get(yc * self.w + xc).copied().map_or(0, i32::from)
    }
}

/// A decoded picture with the motion field the following pictures use as collocated data.
pub(crate) struct Pic {
    pub planes: [Plane; 3],
    pub map_mv: Vec<[Mv; 2]>,
    pub map_refi: Vec<[i8; 2]>,
    pub list_dist: [i16; MAX_REFS],
    pub doi: i32,
    pub ptr: i64,
    pub tag: i64,
    pub bit_depth: u32,
}

#[derive(Clone, Default)]
pub(crate) struct RefInfo {
    pub pic: Option<Arc<Pic>>,
    pub dist: i32,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct SaoParam {
    pub enable: bool,
    pub typ: i32,
    pub band_idx: [i32; 4],
    pub offset: [i32; 5],
}

pub(crate) struct Core {
    pub seq: Sequence,
    pub ph: Picture,
    pub pat: PatchHeader,
    pub lbac: Lbac,
    pub ctx: Ctx,
    pub cur: Pic,
    pub refp: Vec<[RefInfo; 2]>,
    pub num_refp: [usize; 2],
    pub slice_type: SliceType,
    pub i_scu: i32,
    // maps over the padded SCU grid
    pub map_scu: Vec<u8>,
    pub map_pos: Vec<u32>,
    pub map_ipm: Vec<i8>,
    pub map_edge: Vec<u8>,
    pub map_patch: Vec<i8>,
    pub map_qp: Vec<i8>,
    pub sao_map: Vec<[SaoParam; 3]>,
    pub alf_map: Vec<[bool; 3]>,
    // current CU
    pub cu_mode: u8,
    pub pb_part: u8,
    pub tb_part: u8,
    pub cbfy: [u8; 4],
    pub cbfc: [u8; 2],
    pub refi: [i8; 2],
    pub ipm: [i8; 4],
    pub ipm_c: i8,
    pub ipf_flag: bool,
    pub affine_flag: u8,
    pub mv: [Mv; 2],
    pub affine_mv: [[[i32; 2]; 4]; 2],
    pub affine_sb_mv: Vec<[[i32; 2]; 2]>,
    pub cu_scup: i32,
    pub cu_x: i32,
    pub cu_y: i32,
    pub log2w: u32,
    pub log2h: u32,
    pub cu_w: i32,
    pub cu_h: i32,
    pub tree_scup: i32,
    pub tree_status: u8,
    pub cons_pred_mode: u8,
    // current LCU
    pub lcu_x: i32,
    pub lcu_y: i32,
    pub lcu_pix_x: i32,
    pub lcu_pix_y: i32,
    pub qp_y: i32,
    pub qp_u: i32,
    pub qp_v: i32,
    pub hmvp_cnt: usize,
    pub hmvp: [Motion; 8],
}

impl Core {
    #[inline]
    pub(crate) fn ix(&self, scup: i32) -> usize {
        usize::try_from(scup + self.i_scu + 1).unwrap_or(0)
    }
    #[inline]
    pub(crate) fn scu(&self, scup: i32) -> u8 {
        self.map_scu.get(self.ix(scup)).copied().unwrap_or(0)
    }
    #[inline]
    pub(crate) fn coded(&self, scup: i32) -> bool {
        self.scu(scup) & SCU_CODED != 0
    }
    #[inline]
    pub(crate) fn is_inter(&self, scup: i32) -> bool {
        self.scu(scup) & SCU_INTER != 0
    }
    #[inline]
    pub(crate) fn affine_of(&self, scup: i32) -> u8 {
        self.scu(scup) >> SCU_AFFINE_SHIFT & 3
    }
    #[inline]
    pub(crate) fn pos(&self, scup: i32) -> u32 {
        self.map_pos.get(self.ix(scup)).copied().unwrap_or(0)
    }
    #[inline]
    pub(crate) fn mv_at(&self, scup: i32) -> [Mv; 2] {
        self.cur.map_mv.get(self.ix(scup)).copied().unwrap_or_default()
    }
    #[inline]
    pub(crate) fn refi_at(&self, scup: i32) -> [i8; 2] {
        self.cur.map_refi.get(self.ix(scup)).copied().unwrap_or([-1, -1])
    }
    pub(crate) fn motion_at(&self, scup: i32) -> Motion {
        Motion { mv: self.mv_at(scup), refi: self.refi_at(scup) }
    }
    pub(crate) fn dist(&self, refi: i8, lidx: usize) -> i32 {
        usize::try_from(refi).ok().and_then(|r| self.refp.get(r)).map_or(1, |r| r[lidx].dist)
    }
}

impl Core {
    pub(crate) fn new(seq: &Sequence) -> Core {
        let i_scu = seq.w_scu as i32 + 2;
        // padded grid: one row above, rows below for the below-left reference look-ups
        let n = (i_scu as usize) * (seq.h_scu as usize + 24) + 2;
        let lcus = (seq.w_lcu * seq.h_lcu) as usize;
        Core {
            seq: seq.clone(),
            ph: Picture::default(),
            pat: PatchHeader::default(),
            lbac: Lbac::new(Vec::new()),
            ctx: Ctx::new(),
            cur: Self::new_pic(seq),
            refp: vec![[RefInfo::default(), RefInfo::default()]; MAX_REFS],
            num_refp: [0, 0],
            slice_type: SliceType::I,
            i_scu,
            map_scu: vec![0; n],
            map_pos: vec![0; n],
            map_ipm: vec![0; n],
            map_edge: vec![0; n],
            map_patch: vec![0; lcus],
            map_qp: vec![0; lcus],
            sao_map: vec![[SaoParam::default(); 3]; lcus],
            alf_map: vec![[false; 3]; lcus],
            cu_mode: MODE_INTRA,
            pb_part: 0,
            tb_part: 0,
            cbfy: [0; 4],
            cbfc: [0; 2],
            refi: [REFI_INVALID; 2],
            ipm: [0; 4],
            ipm_c: 0,
            ipf_flag: false,
            affine_flag: 0,
            mv: [[0; 2]; 2],
            affine_mv: [[[0; 2]; 4]; 2],
            affine_sb_mv: Vec::new(),
            cu_scup: 0,
            cu_x: 0,
            cu_y: 0,
            log2w: 0,
            log2h: 0,
            cu_w: 0,
            cu_h: 0,
            tree_scup: 0,
            tree_status: TREE_LC,
            cons_pred_mode: NO_MODE_CONS,
            lcu_x: 0,
            lcu_y: 0,
            lcu_pix_x: 0,
            lcu_pix_y: 0,
            qp_y: 0,
            qp_u: 0,
            qp_v: 0,
            hmvp_cnt: 0,
            hmvp: [Motion::default(); 8],
        }
    }

    pub(crate) fn new_pic(seq: &Sequence) -> Pic {
        let (w, h) = (seq.pic_width as usize, seq.pic_height as usize);
        let i_scu = seq.w_scu as usize + 2;
        let n = i_scu * (seq.h_scu as usize + 24) + 2;
        Pic {
            planes: [Plane::new(w, h), Plane::new(w / 2, h / 2), Plane::new(w / 2, h / 2)],
            map_mv: vec![[[0; 2]; 2]; n],
            map_refi: vec![[REFI_INVALID; 2]; n],
            list_dist: [0; MAX_REFS],
            doi: 0,
            ptr: 0,
            tag: 0,
            bit_depth: seq.bit_depth,
        }
    }
}
