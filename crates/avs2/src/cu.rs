// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from davs2 1.7 (`source/common/cu.cc`, `aec.cc`, `predict.cc`, `block_info.cc`,
// `quant.h`, `predict.h`), Copyright (C) 2018~ VCL, NELVT, Peking University
// (Falei Luo et al.), GPL-2.0-or-later.
//! Coding-unit parsing: the AEC syntax elements, motion-vector prediction, skip/direct
//! derivation and the run-level coefficients (dequantised on the fly, as in davs2).

use crate::Error;
use crate::aec::*;
use crate::header::FrameType;

pub(crate) const MIN_CU_SIZE_IN_BIT: i32 = 3;
pub(crate) const MIN_PU_SIZE_IN_BIT: i32 = 2;
pub(crate) const B8X8_IN_BIT: i32 = 3;
pub(crate) const B16X16_IN_BIT: i32 = 4;
pub(crate) const B32X32_IN_BIT: i32 = 5;
pub(crate) const B64X64_IN_BIT: i32 = 6;

// cu types
pub(crate) const PRED_SKIP: u8 = 0;
pub(crate) const PRED_2NX2N: u8 = 1;
pub(crate) const PRED_2NXN: u8 = 2;
pub(crate) const PRED_NX2N: u8 = 3;
pub(crate) const PRED_2NXNU: u8 = 4;
pub(crate) const PRED_2NXND: u8 = 5;
pub(crate) const PRED_NLX2N: u8 = 6;
pub(crate) const PRED_NRX2N: u8 = 7;
pub(crate) const PRED_I_2NX2N: u8 = 8;
pub(crate) const PRED_I_NXN: u8 = 9;
pub(crate) const PRED_I_2NXN: u8 = 10;
pub(crate) const PRED_I_NX2N: u8 = 11;

const MASK_HOR_PU_MODES: u32 = 0x0434;
const MASK_VER_PU_MODES: u32 = 0x08C8;
const MASK_INTER_MODES: u32 = 0x00FF;
const MASK_INTER_NOSKIP: u32 = 0x00FE;
const MASK_INTRA_MODES: u32 = 0x0F00;

pub(crate) fn is_hor_pu_part(m: u8) -> bool {
    (1u32 << m) & MASK_HOR_PU_MODES != 0
}
pub(crate) fn is_ver_pu_part(m: u8) -> bool {
    (1u32 << m) & MASK_VER_PU_MODES != 0
}
pub(crate) fn is_intra_mode(m: u8) -> bool {
    (1u32 << m) & MASK_INTRA_MODES != 0
}
pub(crate) fn is_inter_mode(m: u8) -> bool {
    (1u32 << m) & MASK_INTER_MODES != 0
}
fn is_noskip_inter_mode(m: u8) -> bool {
    (1u32 << m) & MASK_INTER_NOSKIP != 0
}

// TU split types
pub(crate) const TU_SPLIT_NON: u8 = 0;
pub(crate) const TU_SPLIT_HOR: u8 = 1;
pub(crate) const TU_SPLIT_VER: u8 = 2;
pub(crate) const TU_SPLIT_CROSS: u8 = 3;
const TU_SPLIT_INVALID: u8 = 0xFF;

// prediction directions
pub(crate) const PDIR_FWD: i8 = 0;
pub(crate) const PDIR_BWD: i8 = 1;
pub(crate) const PDIR_SYM: i8 = 2;
pub(crate) const PDIR_BID: i8 = 3;
pub(crate) const PDIR_DUAL: i8 = 4;
pub(crate) const PDIR_INVALID: i8 = -1;

pub(crate) const INVALID_REF: i8 = -1;
pub(crate) const B_BWD: i8 = 0;
pub(crate) const B_FWD: i8 = 1;

// direct/skip modes
const DS_NONE: u8 = 0;
const DS_B_BID: usize = 1;
const DS_B_BWD: usize = 2;
const DS_B_SYM: usize = 3;
const DS_B_FWD: usize = 4;
const DS_DUAL_1ST: u8 = 1;
const DS_DUAL_2ND: u8 = 2;
const DS_SINGLE_1ST: usize = 3;
const DS_SINGLE_2ND: usize = 4;

pub(crate) const DC_PRED: i8 = 0;
const BI_PRED: i8 = 2;
pub(crate) const NUM_INTRA_MODE: i8 = 33;
pub(crate) const DC_PRED_C: u8 = 1;

// neighbour indices (`neighbor_block_pos_e`)
const BLK_TOPLEFT: usize = 0;
const BLK_TOP: usize = 1;
const BLK_LEFT: usize = 2;
const BLK_TOPRIGHT: usize = 3;
const BLK_TOP2: usize = 4;
const BLK_LEFT2: usize = 5;

// DCT patterns
const DCT_DEFAULT: u8 = 0;
const DCT_HALF: u8 = 1;
const DCT_QUAD: u8 = 2;

const INTRA_PRED_HOR: i32 = 1;
const INTRA_PRED_DC_DIAG: i32 = 2;

const MULTI: i32 = 16384;
const HALF_MULTI: i32 = 8192;
const OFFSET: i32 = 14;
const THRESHOLD_PMVR: i32 = 2;
const LIMIT_BIT: i32 = 16;

/// A motion vector in quarter (luma) samples.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Mv {
    pub x: i16,
    pub y: i16,
}

/// Reference indices of both directions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RefIdx {
    pub r: [i8; 2],
}

/// A block inside a CU, in luma samples (`cb_t`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Cb {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Cb {
    const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Cb { x, y, w, h }
    }
    pub(crate) fn shl(self, s: i32) -> Self {
        Cb { x: self.x << s, y: self.y << s, w: self.w << s, h: self.h << s }
    }
    pub(crate) fn shr(self, s: i32) -> Self {
        Cb { x: self.x >> s, y: self.y >> s, w: self.w >> s, h: self.h >> s }
    }
}

/// The data of one coding unit, copied to every 8×8 SCU it covers (`cu_t`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cu {
    pub level: i32,
    pub cu_type: u8,
    pub qp: i32,
    pub cbp: u8,
    pub c_ipred_mode: u8,
    pub dmh_mode: u8,
    pub dct_pattern: [u8; 6],
    pub trans_size: u8,
    pub intra_pred_modes: [i8; 4],
    pub b8pdir: [i8; 4],
    pub ref_idx: [RefIdx; 4],
    pub mv: [[Mv; 2]; 4],
    pub num_pu: usize,
    pub pu: [Cb; 4],
    pub weighted_skipmode: u8,
    pub md_directskip_mode: u8,
    pub slice_nr: i32,
}

impl Default for Cu {
    fn default() -> Self {
        Cu {
            level: 3,
            cu_type: PRED_SKIP,
            qp: 0,
            cbp: 0,
            c_ipred_mode: DC_PRED_C,
            dmh_mode: 0,
            dct_pattern: [0; 6],
            trans_size: TU_SPLIT_NON,
            intra_pred_modes: [0; 4],
            b8pdir: [0; 4],
            ref_idx: [RefIdx::default(); 4],
            mv: [[Mv::default(); 2]; 4],
            num_pu: 1,
            pu: [Cb::default(); 4],
            weighted_skipmode: 0,
            md_directskip_mode: 0,
            slice_nr: 0,
        }
    }
}

impl Cu {
    pub(crate) fn is_intra(&self) -> bool {
        is_intra_mode(self.cu_type)
    }
}

/// A spatial / temporal neighbour for MV prediction (`neighbor_inter_t`).
#[derive(Clone, Copy, Debug, Default)]
struct Neighbor {
    available: bool,
    dir_pred: i8,
    ref_idx: RefIdx,
    mv: [Mv; 2],
}

/// Motion data of a reference picture kept for temporal prediction.
#[derive(Clone, Debug, Default)]
pub(crate) struct ColInfo {
    pub mvbuf: Vec<Mv>,
    pub refbuf: Vec<i8>,
    pub dist_scale_refs: [i32; 4],
}

// ---------------------------------------------------------------------------------------------
// tables

pub(crate) const QP_SCALE_CR: [u8; 64] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40,
    41, 42, 42, 43, 43, 44, 44, 45, 45, 46, 46, 47, 47, 48, 48, 48, 49, 49, 49, 50, 50, 50, 51,
];

const IQ_SHIFT: [i32; 80] = [
    15, 15, 15, 15, 15, 15, 15, 15, 14, 14, 14, 14, 14, 14, 14, 14, 14, 13, 13, 13, 13, 13, 13, 13, 12, 12, 12, 12, 12, 12, 12, 12, 12, 11, 11, 11, 11, 11, 11,
    11, 11, 10, 10, 10, 10, 10, 10, 10, 10, 9, 9, 9, 9, 9, 9, 9, 8, 8, 8, 8, 8, 8, 8, 8, 7, 7, 7, 7, 7, 7, 7, 7, 6, 6, 6, 6, 6, 6, 6, 6,
];

const IQ_TAB: [i32; 80] = [
    32768, 36061, 38968, 42495, 46341, 50535, 55437, 60424, 32932, 35734, 38968, 42495, 46177, 50535, 55109, 59933, 65535, 35734, 38968, 42577, 46341, 50617,
    55027, 60097, 32809, 35734, 38968, 42454, 46382, 50576, 55109, 60056, 65535, 35734, 38968, 42495, 46320, 50515, 55109, 60076, 65535, 35744, 38968, 42495,
    46341, 50535, 55099, 60087, 65535, 35734, 38973, 42500, 46341, 50535, 55109, 60097, 32771, 35734, 38965, 42497, 46341, 50535, 55109, 60099, 32768, 36061,
    38968, 42495, 46341, 50535, 55437, 60424, 32932, 35734, 38968, 42495, 46177, 50535, 55109, 59933,
];

pub(crate) const TAB_B8XY_TO_ZIGZAG: [[usize; 8]; 8] = [
    [0, 1, 4, 5, 16, 17, 20, 21],
    [2, 3, 6, 7, 18, 19, 22, 23],
    [8, 9, 12, 13, 24, 25, 28, 29],
    [10, 11, 14, 15, 26, 27, 30, 31],
    [32, 33, 36, 37, 48, 49, 52, 53],
    [34, 35, 38, 39, 50, 51, 54, 55],
    [40, 41, 44, 45, 56, 57, 60, 61],
    [42, 43, 46, 47, 58, 59, 62, 63],
];

const TAB_INTRA_MODE_SCAN_TYPE: [i32; 33] = [2, 2, 2, 1, 1, 2, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 0];

/// `tab_intra_mode_luma2chroma`: chroma mode equal to the luma mode (DM duplicates).
const TAB_INTRA_MODE_LUMA2CHROMA: [i32; 33] =
    [1, -1, 4, -1, -1, -1, -1, -1, -1, -1, -1, -1, 3, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, 2, -1, -1, -1, -1, -1, -1, -1, -1];

const EO_OFFSET_INV_MAP: [i32; 8] = [1, 0, 2, -1, 3, 4, 5, 6];
const T_CHR: [i32; 5] = [0, 1, 2, 4, 3000];
const TAB_RANK: [i32; 6] = [0, 1, 2, 3, 3, 4];

const RASTER2ZZ_4X4: [u8; 16] = [0, 1, 5, 6, 2, 4, 7, 12, 3, 8, 11, 13, 9, 10, 14, 15];
const RASTER2ZZ_8X8: [u8; 64] = [
    0, 1, 5, 6, 14, 15, 27, 28, 2, 4, 7, 13, 16, 26, 29, 42, 3, 8, 12, 17, 25, 30, 41, 43, 9, 11, 18, 24, 31, 40, 44, 53, 10, 19, 23, 32, 39, 45, 52, 54, 20,
    22, 33, 38, 46, 51, 55, 60, 21, 34, 37, 47, 50, 56, 59, 61, 35, 36, 48, 49, 57, 58, 62, 63,
];
const RASTER2ZZ_2X8: [u8; 16] = [0, 1, 4, 5, 8, 9, 12, 13, 2, 3, 6, 7, 10, 11, 14, 15];
const RASTER2ZZ_8X2: [u8; 16] = [0, 1, 2, 4, 3, 5, 6, 8, 7, 9, 10, 12, 11, 13, 14, 15];
const TAB_SCAN_COEFF_POS_IN_CG: [[usize; 4]; 4] = [[0, 1, 5, 6], [2, 4, 7, 12], [3, 8, 11, 13], [9, 10, 14, 15]];

/// `tab_scan_4x4`: zig-zag positions (x, y) in a 4×4 coefficient group.
pub(crate) const TAB_SCAN_4X4: [[usize; 2]; 16] =
    [[0, 0], [1, 0], [0, 1], [0, 2], [1, 1], [2, 0], [3, 0], [2, 1], [1, 2], [0, 3], [1, 3], [2, 2], [3, 1], [3, 2], [2, 3], [3, 3]];

pub(crate) const SAOCLIP: [[i32; 3]; 6] = [[-1, 6, 7], [0, 1, 1], [0, 0, 0], [-1, 0, 1], [-6, 1, 7], [-7, 7, 7]];

const DMH_POS: [[[i32; 2]; 2]; 9] = [
    [[0, 0], [0, 0]],
    [[-1, 0], [1, 0]],
    [[0, -1], [0, 1]],
    [[-1, 1], [1, -1]],
    [[-1, -1], [1, 1]],
    [[-2, 0], [2, 0]],
    [[0, -2], [0, 2]],
    [[-2, 2], [2, -2]],
    [[-2, -2], [2, 2]],
];

pub(crate) fn dmh_pos(mode: u8) -> [[i32; 2]; 2] {
    DMH_POS.get(usize::from(mode)).copied().unwrap_or([[0, 0], [0, 0]])
}

const SCAN_2X2: [[usize; 2]; 4] = [[0, 0], [1, 0], [0, 1], [1, 1]];
const SCAN_1X4: [[usize; 2]; 4] = [[0, 0], [1, 0], [2, 0], [3, 0]];
const SCAN_4X1: [[usize; 2]; 4] = [[0, 0], [0, 1], [0, 2], [0, 3]];
const SCAN_2X8: [[usize; 2]; 16] =
    [[0, 0], [1, 0], [0, 1], [1, 1], [2, 0], [3, 0], [2, 1], [3, 1], [4, 0], [5, 0], [4, 1], [5, 1], [6, 0], [7, 0], [6, 1], [7, 1]];
const SCAN_8X2: [[usize; 2]; 16] =
    [[0, 0], [1, 0], [0, 1], [0, 2], [1, 1], [1, 2], [0, 3], [0, 4], [1, 3], [1, 4], [0, 5], [0, 6], [1, 5], [1, 6], [0, 7], [1, 7]];
const SCAN_8X8: [[usize; 2]; 64] = [
    [0, 0],
    [1, 0],
    [0, 1],
    [0, 2],
    [1, 1],
    [2, 0],
    [3, 0],
    [2, 1],
    [1, 2],
    [0, 3],
    [0, 4],
    [1, 3],
    [2, 2],
    [3, 1],
    [4, 0],
    [5, 0],
    [4, 1],
    [3, 2],
    [2, 3],
    [1, 4],
    [0, 5],
    [0, 6],
    [1, 5],
    [2, 4],
    [3, 3],
    [4, 2],
    [5, 1],
    [6, 0],
    [7, 0],
    [6, 1],
    [5, 2],
    [4, 3],
    [3, 4],
    [2, 5],
    [1, 6],
    [0, 7],
    [1, 7],
    [2, 6],
    [3, 5],
    [4, 4],
    [5, 3],
    [6, 2],
    [7, 1],
    [7, 2],
    [6, 3],
    [5, 4],
    [4, 5],
    [3, 6],
    [2, 7],
    [3, 7],
    [4, 6],
    [5, 5],
    [6, 4],
    [7, 3],
    [7, 4],
    [6, 5],
    [5, 6],
    [4, 7],
    [5, 7],
    [6, 6],
    [7, 5],
    [7, 6],
    [6, 7],
    [7, 7],
];

/// `tab_scan_cg`: the scan of 4×4 coefficient groups for a block of `w_cg`×`h_cg` groups.
fn cg_scan(w_cg: usize, h_cg: usize) -> &'static [[usize; 2]] {
    match (w_cg, h_cg) {
        (1, 1) => &SCAN_2X2[..1],
        (2, 2) => &SCAN_2X2,
        (4, 1) => &SCAN_1X4,
        (1, 4) => &SCAN_4X1,
        (4, 4) => &TAB_SCAN_4X4,
        (8, 2) => &SCAN_2X8,
        (2, 8) => &SCAN_8X2,
        _ => &SCAN_8X8,
    }
}

// ---------------------------------------------------------------------------------------------
// MV scaling (frame coding only)

fn clip_i16(v: i32) -> i16 {
    v.clamp(-32768, 32767) as i16
}

fn sign3(v: i32) -> i32 {
    v.signum()
}

fn scale_mv_default(mv: i32, dist_dst: i32, dist_src: i32) -> i16 {
    let v = sign3(mv) * ((mv.abs().wrapping_mul(dist_dst).wrapping_mul(dist_src).wrapping_add(HALF_MULTI)) >> OFFSET);
    clip_i16(v)
}

pub(crate) fn scale_mv_skip(mv: i32, dist_dst: i32, dist_src: i32) -> i16 {
    let v = ((mv.wrapping_mul(dist_dst).wrapping_mul(dist_src).wrapping_add(HALF_MULTI)) >> OFFSET) as i16;
    clip_i16(i32::from(v))
}

fn scale_mv_biskip(mv: i32, dist_dst: i32, dist_src: i32) -> i16 {
    let v = (sign3(mv) * ((dist_src.wrapping_mul(1 + mv.abs().wrapping_mul(dist_dst)) - 1) >> OFFSET)) as i16;
    clip_i16(i32::from(v))
}

// ---------------------------------------------------------------------------------------------
// the per-picture parsing state (`davs2_t` subset)

/// Frame-level geometry and parsing state shared by the CU parser and the reconstruction.
pub(crate) struct PicState {
    pub width: i32,
    pub height: i32,
    pub w_scu: i32,
    pub h_scu: i32,
    pub w_spu: i32,
    pub h_spu: i32,
    pub lcu_level: i32,
    pub bit_depth: i32,
    pub frame_type: FrameType,
    pub num_refs: usize,
    pub dist_refs: [i32; 4],
    pub dist_scale_refs: [i32; 4],
    pub chroma_delta_u: i32,
    pub chroma_delta_v: i32,
    pub mono: bool,

    // sequence tools
    pub amp: bool,
    pub mhp_skip: bool,
    pub wsm: bool,
    pub dhp: bool,
    pub nsqt: bool,
    pub sdip: bool,
    pub pmvr: bool,

    pub scu: Vec<Cu>,
    ipm: Vec<i8>,
    ipm_stride: i32,
    pub dirpred: Vec<i8>,
    pub ref_idx: Vec<RefIdx>,
    pub tmv: [Vec<Mv>; 2],

    // slice / LCU state
    pub qp: i32,
    pub dqp: bool,
    pub slice_index: i32,
    last_dquant: i32,
    left_cu_qp: i32,
    c_ipred_mode_ctx: usize,
    pub lcu_scu_x: i32,
    pub lcu_scu_y: i32,
    lcu_spu_x: i32,
    lcu_spu_y: i32,
    idx_cu_zscan: usize,
    neighbors: [Neighbor; 7],
    pub coeff_y: Vec<i16>,
    pub coeff_uv: [Vec<i16>; 2],
}

impl PicState {
    pub(crate) fn new() -> Self {
        PicState {
            width: 0,
            height: 0,
            w_scu: 0,
            h_scu: 0,
            w_spu: 0,
            h_spu: 0,
            lcu_level: 6,
            bit_depth: 8,
            frame_type: FrameType::I,
            num_refs: 0,
            dist_refs: [-1; 4],
            dist_scale_refs: [-1; 4],
            chroma_delta_u: 0,
            chroma_delta_v: 0,
            mono: false,
            amp: false,
            mhp_skip: false,
            wsm: false,
            dhp: false,
            nsqt: false,
            sdip: false,
            pmvr: false,
            scu: Vec::new(),
            ipm: Vec::new(),
            ipm_stride: 0,
            dirpred: Vec::new(),
            ref_idx: Vec::new(),
            tmv: [Vec::new(), Vec::new()],
            qp: 0,
            dqp: false,
            slice_index: -1,
            last_dquant: 0,
            left_cu_qp: 0,
            c_ipred_mode_ctx: 0,
            lcu_scu_x: 0,
            lcu_scu_y: 0,
            lcu_spu_x: 0,
            lcu_spu_y: 0,
            idx_cu_zscan: 0,
            neighbors: [Neighbor::default(); 7],
            coeff_y: vec![0; 64 * 64],
            coeff_uv: [vec![0; 32 * 32], vec![0; 32 * 32]],
        }
    }

    /// Sizes the buffers for a `width`×`height` (multiples of 8) picture; keeps them when the
    /// size is unchanged, as davs2 keeps its SCU data between pictures.
    pub(crate) fn resize(&mut self, width: i32, height: i32, lcu_level: i32) {
        if self.width == width && self.height == height && self.lcu_level == lcu_level && !self.scu.is_empty() {
            return;
        }
        self.width = width;
        self.height = height;
        self.lcu_level = lcu_level;
        self.w_scu = width >> MIN_CU_SIZE_IN_BIT;
        self.h_scu = height >> MIN_CU_SIZE_IN_BIT;
        self.w_spu = width >> MIN_PU_SIZE_IN_BIT;
        self.h_spu = height >> MIN_PU_SIZE_IN_BIT;
        let n_scu = (self.w_scu * self.h_scu).max(0) as usize;
        let n_spu = (self.w_spu * self.h_spu).max(0) as usize;
        self.scu = vec![Cu::default(); n_scu];
        self.ipm_stride = self.w_spu + 16;
        self.ipm = vec![DC_PRED; (self.ipm_stride * (self.h_spu + 1)).max(0) as usize];
        self.dirpred = vec![PDIR_INVALID; n_spu];
        self.ref_idx = vec![RefIdx::default(); n_spu];
        self.tmv = [vec![Mv::default(); n_spu], vec![Mv::default(); n_spu]];
    }

    /// `init_frame`.
    pub(crate) fn init_frame(&mut self) {
        self.slice_index = -1;
        self.ipm.fill(DC_PRED);
        self.dirpred.fill(PDIR_INVALID);
        self.ref_idx.fill(RefIdx::default());
    }

    fn ipm_index(&self, x4: i32, y4: i32) -> usize {
        (self.ipm_stride * (y4 + 1) + 16 + x4).max(0) as usize
    }

    fn ipm_get(&self, x4: i32, y4: i32) -> i8 {
        self.ipm.get(self.ipm_index(x4, y4)).copied().unwrap_or(DC_PRED)
    }

    /// Start of a new slice whose first LCU is in LCU row `lcu_y` (`check_slice_header`).
    pub(crate) fn new_slice(&mut self, qp: i32, dqp: bool, lcu_y: i32) {
        self.slice_index += 1;
        self.qp = qp;
        self.dqp = dqp;
        self.last_dquant = 0;
        let row = (lcu_y << (self.lcu_level - MIN_PU_SIZE_IN_BIT)) - 1;
        let start = (row * self.ipm_stride + self.ipm_stride).max(0) as usize; // row index `row`, column -16
        let end = (start + self.ipm_stride.max(0) as usize).min(self.ipm.len());
        if let Some(s) = self.ipm.get_mut(start..end) {
            s.fill(DC_PRED);
        }
    }

    /// `decode_lcu_init`.
    pub(crate) fn lcu_init(&mut self, lcu_x: i32, lcu_y: i32) {
        let num_in_scu = 1 << (self.lcu_level - MIN_CU_SIZE_IN_BIT);
        self.lcu_scu_x = lcu_x << (self.lcu_level - MIN_CU_SIZE_IN_BIT);
        self.lcu_scu_y = lcu_y << (self.lcu_level - MIN_CU_SIZE_IN_BIT);
        self.lcu_spu_x = self.lcu_scu_x * 2;
        self.lcu_spu_y = self.lcu_scu_y * 2;
        let w = ((self.width - (self.lcu_scu_x << 3)) >> 3).min(num_in_scu);
        let h = ((self.height - (self.lcu_scu_y << 3)) >> 3).min(num_in_scu);
        for j in 0..h {
            for i in 0..w {
                let idx = ((self.lcu_scu_y + j) * self.w_scu + self.lcu_scu_x + i) as usize;
                if let Some(c) = self.scu.get_mut(idx) {
                    c.slice_nr = self.slice_index;
                }
            }
        }
    }

    fn scu_at(&self, idx: i32) -> Option<&Cu> {
        if idx < 0 {
            return None;
        }
        self.scu.get(idx as usize)
    }

    fn spu(&self, x4: i32, y4: i32) -> usize {
        (y4 * self.w_spu + x4).max(0) as usize
    }

    // -----------------------------------------------------------------------------------------
    // neighbours

    fn neighbor_spatial(&self, slice: i32, x4: i32, y4: i32) -> Neighbor {
        let outside = y4 < 0 || y4 >= self.h_spu || x4 < 0 || x4 >= self.w_spu;
        let scu_xy = (y4 >> 1) * self.w_scu + (x4 >> 1);
        if outside || self.scu_at(scu_xy).map(|c| c.slice_nr) != Some(slice) {
            return Neighbor { available: false, dir_pred: PDIR_INVALID, ref_idx: RefIdx { r: [-1, -1] }, mv: [Mv::default(); 2] };
        }
        let pos = self.spu(x4, y4);
        Neighbor {
            available: true,
            dir_pred: self.dirpred.get(pos).copied().unwrap_or(PDIR_INVALID),
            ref_idx: self.ref_idx.get(pos).copied().unwrap_or(RefIdx { r: [-1, -1] }),
            mv: [self.tmv[0].get(pos).copied().unwrap_or_default(), self.tmv[1].get(pos).copied().unwrap_or_default()],
        }
    }

    fn tr_avail(&self, x4_in_lcu: i32, y4_in_lcu: i32) -> bool {
        tr_avail(self.lcu_level, x4_in_lcu, y4_in_lcu)
    }

    /// `cu_get_neighbors`.
    fn get_neighbors(&mut self, slice: i32, pix_x: i32, pix_y: i32, bsx: i32, bsy: i32) {
        let x0 = pix_x >> 2;
        let y0 = pix_y >> 2;
        let x1 = (bsx >> 2) + x0 - 1;
        let y1 = (bsy >> 2) + y0 - 1;
        let tr = self.tr_avail(x1 - self.lcu_spu_x, y0 - self.lcu_spu_y);
        self.neighbors[BLK_LEFT] = self.neighbor_spatial(slice, x0 - 1, y0);
        self.neighbors[BLK_TOP] = self.neighbor_spatial(slice, x0, y0 - 1);
        self.neighbors[BLK_TOP2] = self.neighbor_spatial(slice, x1, y0 - 1);
        self.neighbors[BLK_TOPLEFT] = self.neighbor_spatial(slice, x0 - 1, y0 - 1);
        self.neighbors[BLK_LEFT2] = self.neighbor_spatial(slice, x0 - 1, y1);
        self.neighbors[BLK_TOPRIGHT] = self.neighbor_spatial(slice, if tr { x1 + 1 } else { -1 }, y0 - 1);
    }

    /// `cu_get_neighbors_default_mvp`.
    fn get_neighbors_default_mvp(&mut self, slice: i32, pix_x: i32, pix_y: i32, bsx: i32) {
        let x0 = pix_x >> 2;
        let y0 = pix_y >> 2;
        let x1 = (bsx >> 2) + x0 - 1;
        let tr = self.tr_avail(x1 - self.lcu_spu_x, y0 - self.lcu_spu_y);
        self.neighbors[BLK_LEFT] = self.neighbor_spatial(slice, x0 - 1, y0);
        self.neighbors[BLK_TOP] = self.neighbor_spatial(slice, x0, y0 - 1);
        self.neighbors[BLK_TOPLEFT] = self.neighbor_spatial(slice, x0 - 1, y0 - 1);
        self.neighbors[BLK_TOPRIGHT] = self.neighbor_spatial(slice, if tr { x1 + 1 } else { -1 }, y0 - 1);
    }

    fn dist_p(&self, r: i32) -> i32 {
        self.dist_refs.get(r.max(0) as usize).copied().unwrap_or(1)
    }

    fn dist_p_scale(&self, r: i32) -> i32 {
        self.dist_scale_refs.get(r.max(0) as usize).copied().unwrap_or(MULTI)
    }

    /// `get_mvp_default`.
    fn get_mvp_default(&mut self, slice: i32, pix_x: i32, pix_y: i32, bwd_2nd: usize, ref_frame: i32, bsx: i32, pu_type: i32) -> Mv {
        self.get_neighbors_default_mvp(slice, pix_x, pix_y, bsx);
        let n = &self.neighbors;
        let avail_ur = n[BLK_TOPRIGHT].available;
        let r_l = i32::from(n[BLK_LEFT].ref_idx.r[bwd_2nd]);
        let r_u = i32::from(n[BLK_TOP].ref_idx.r[bwd_2nd]);
        let r_ul = i32::from(n[BLK_TOPLEFT].ref_idx.r[bwd_2nd]);
        let r_ur = if avail_ur { i32::from(n[BLK_TOPRIGHT].ref_idx.r[bwd_2nd]) } else { r_ul };
        let mut mva = n[BLK_LEFT].mv[bwd_2nd];
        let mut mvb = n[BLK_TOP].mv[bwd_2nd];
        let mvd = n[BLK_TOPLEFT].mv[bwd_2nd];
        let mut mvc = if avail_ur { n[BLK_TOPRIGHT].mv[bwd_2nd] } else { mvd };

        let s_frame = self.frame_type == FrameType::S;
        let recheck = |r: i32| if r != -1 && s_frame { -1 } else { r };
        let r_l = recheck(r_l);
        let r_u = recheck(r_u);
        let r_ur = recheck(r_ur);

        let mvp_type = derive_mv_pred_type(ref_frame, r_l, r_u, r_ur, pu_type);

        if self.frame_type == FrameType::B {
            let which = if bwd_2nd != 0 { B_BWD } else { B_FWD } as i32;
            let mult = self.dist_p(which);
            let mult_src = self.dist_p_scale(which);
            for (mv, r) in [(&mut mva, r_l), (&mut mvb, r_u), (&mut mvc, r_ur)] {
                if r >= 0 {
                    mv.y = scale_mv_default(i32::from(mv.y), mult, mult_src);
                    mv.x = scale_mv_default(i32::from(mv.x), mult, mult_src);
                } else {
                    *mv = Mv::default();
                }
            }
        } else {
            let mult = self.dist_p(ref_frame);
            for (mv, r) in [(&mut mva, r_l), (&mut mvb, r_u), (&mut mvc, r_ur)] {
                if r >= 0 {
                    let src = self.dist_p_scale(r);
                    mv.y = scale_mv_default(i32::from(mv.y), mult, src);
                    mv.x = scale_mv_default(i32::from(mv.x), mult, src);
                } else {
                    *mv = Mv::default();
                }
            }
        }

        match mvp_type {
            0 => Mv {
                x: derive_median_mv(i32::from(mva.x), i32::from(mvb.x), i32::from(mvc.x)),
                y: derive_median_mv(i32::from(mva.y), i32::from(mvb.y), i32::from(mvc.y)),
            },
            1 => mva,
            2 => mvb,
            _ => mvc,
        }
    }

    fn pmvr(&self, mvd: Mv, mvp: Mv) -> Mv {
        let (mvdx, mvdy) = (i32::from(mvd.x), i32::from(mvd.y));
        let (mvpx, mvpy) = (i32::from(mvp.x), i32::from(mvp.y));
        let (mvx, mvy);
        if self.pmvr {
            let ctr_x = ((mvpx >> 1) << 1) - mvpx;
            let ctr_y = ((mvpy >> 1) << 1) - mvpy;
            if (mvdx - ctr_x).abs() > THRESHOLD_PMVR {
                mvx = mvpx + (mvdx << 1) - ctr_x - sign2(mvdx - ctr_x) * THRESHOLD_PMVR;
                mvy = mvpy + (mvdy << 1) + ctr_y;
            } else if (mvdy - ctr_y).abs() > THRESHOLD_PMVR {
                mvx = mvpx + (mvdx << 1) + ctr_x;
                mvy = mvpy + (mvdy << 1) - ctr_y - sign2(mvdy - ctr_y) * THRESHOLD_PMVR;
            } else {
                mvx = mvdx + mvpx;
                mvy = mvdy + mvpy;
            }
        } else {
            mvx = mvdx + mvpx;
            mvy = mvdy + mvpy;
        }
        Mv { x: clip_i16(mvx), y: clip_i16(mvy) }
    }

    // -----------------------------------------------------------------------------------------
    // CU parsing

    /// `decode_lcu_parse`: parses the CU quadtree of one LCU (or sub-tree).
    pub(crate) fn parse_lcu(&mut self, aec: &mut Aec<'_>, col: Option<&ColInfo>, level: i32, pix_x: i32, pix_y: i32) -> Result<(), Error> {
        let inside = pix_x + (1 << level) <= self.width && pix_y + (1 << level) <= self.height;
        let mut split = level != MIN_CU_SIZE_IN_BIT;
        if level > MIN_CU_SIZE_IN_BIT && inside {
            split = aec.symbol(CTX_CU_SPLIT + (level - MIN_CU_SIZE_IN_BIT - 1) as usize) != 0;
        }
        if split {
            let next = level - 1;
            for i in 0..4 {
                let sx = pix_x + ((i & 1) << next);
                let sy = pix_y + ((i >> 1) << next);
                if sx < self.width && sy < self.height {
                    self.parse_lcu(aec, col, next, sx, sy)?;
                }
            }
            Ok(())
        } else {
            let cx = pix_x >> 3;
            let cy = pix_y >> 3;
            let zy = (cy - self.lcu_scu_y).clamp(0, 7) as usize;
            let zx = (cx - self.lcu_scu_x).clamp(0, 7) as usize;
            self.idx_cu_zscan = TAB_B8XY_TO_ZIGZAG[zy][zx];
            let xy = cy * self.w_scu + cx;
            let res = self.read_cu(aec, col, level, xy, pix_x, pix_y);
            if res.is_err() || aec.bit_error {
                if let Some(c) = self.scu.get_mut(xy.max(0) as usize) {
                    c.slice_nr = -1;
                }
                return Err(res.err().unwrap_or(Error::Invalid("arithmetic decoding error")));
            }
            Ok(())
        }
    }

    fn cu_mut(&mut self, xy: i32) -> Result<&mut Cu, Error> {
        self.scu.get_mut(xy.max(0) as usize).ok_or(Error::Invalid("CU position"))
    }

    fn cu(&self, xy: i32) -> Result<Cu, Error> {
        self.scu.get(xy.max(0) as usize).copied().ok_or(Error::Invalid("CU position"))
    }

    /// `cu_read_info`.
    fn read_cu(&mut self, aec: &mut Aec<'_>, col: Option<&ColInfo>, level: i32, xy: i32, pix_x: i32, pix_y: i32) -> Result<(), Error> {
        let size_in_scu = 1 << (level - MIN_CU_SIZE_IN_BIT);
        // cu_init
        let slice_qp = self.qp;
        {
            let c = self.cu_mut(xy)?;
            c.level = level;
            c.qp = slice_qp;
            c.cu_type = PRED_SKIP;
            c.cbp = 0;
            c.c_ipred_mode = DC_PRED_C;
            c.dmh_mode = 0;
            c.dct_pattern = [0; 6];
        }
        self.left_cu_qp = self.qp;
        self.c_ipred_mode_ctx = 0;
        if pix_x > 0 {
            let cur_slice = self.cu(xy)?.slice_nr;
            if let Some(left) = self.scu_at(xy - 1).copied()
                && left.slice_nr == cur_slice
            {
                self.c_ipred_mode_ctx = usize::from(left.c_ipred_mode != 0);
                self.left_cu_qp = left.qp;
            }
        }

        let mut cu = self.cu(xy)?;
        let real_cu_type = self.read_cu_header(aec, &mut cu, pix_x, pix_y)?;
        init_prediction_units(&mut cu, self.frame_type);

        if cu.is_intra() {
            for i in 0..4 {
                cu.ref_idx[i] = RefIdx { r: [INVALID_REF, INVALID_REF] };
                cu.b8pdir[i] = PDIR_INVALID;
            }
            self.store_references(&cu, pix_x, pix_y);
        } else if cu.cu_type == PRED_SKIP {
            self.get_neighbors(cu.slice_nr, pix_x, pix_y, 1 << level, 1 << level);
            self.fill_mv_and_ref_for_skip(&mut cu, col, pix_x, pix_y, size_in_scu)?;
        } else {
            self.store_references(&cu, pix_x, pix_y);
            self.read_mv(aec, &mut cu, pix_x, pix_y)?;
        }

        if real_cu_type < 0 {
            cu.qp = self.left_cu_qp;
            cu.trans_size = TU_SPLIT_NON;
        } else {
            self.read_cbp(aec, &mut cu, pix_x >> 3, pix_y >> 3)?;
            if cu.cbp != 0 {
                self.read_all_coeffs(aec, &mut cu)?;
            }
        }
        if aec.bit_error {
            return Err(Error::Invalid("arithmetic decoding error"));
        }

        // cu_read_end
        for j in 0..size_in_scu {
            for i in 0..size_in_scu {
                let idx = xy + j * self.w_scu + i;
                if let Some(c) = self.scu.get_mut(idx.max(0) as usize) {
                    *c = cu;
                }
            }
        }
        Ok(())
    }

    /// `cu_read_header`: CU type, prediction directions, intra modes.
    fn read_cu_header(&mut self, aec: &mut Aec<'_>, cu: &mut Cu, pix_x: i32, pix_y: i32) -> Result<i32, Error> {
        cu.md_directskip_mode = 0;
        let real = if self.frame_type == FrameType::S { read_cu_type_sframe(aec) } else { self.read_cu_type(aec, cu) };
        if aec.bit_error {
            return Err(Error::Invalid("CU type"));
        }
        cu.cu_type = real.max(0) as u8;

        if !self.frame_type.is_intra() && is_inter_mode(cu.cu_type) {
            self.read_inter_pred_dir(aec, cu);
        }

        if cu.is_intra() {
            let size_8x8 = 1 << (cu.level - B8X8_IN_BIT);
            let size_16x16 = 1 << (cu.level - B16X16_IN_BIT).max(0);
            let y4 = pix_y >> 2;
            let x4 = pix_x >> 2;
            let t = self.read_intra_cu_type(aec, cu)?;
            cu.cu_type = t;
            self.read_intrapred_mode_luma(aec, cu, 0, x4, y4)?;
            match t {
                PRED_I_2NXN => {
                    for k in 1..4 {
                        self.read_intrapred_mode_luma(aec, cu, k, x4, y4 + k as i32 * size_16x16)?;
                    }
                }
                PRED_I_NX2N => {
                    for k in 1..4 {
                        self.read_intrapred_mode_luma(aec, cu, k, x4 + k as i32 * size_16x16, y4)?;
                    }
                }
                PRED_I_NXN => {
                    self.read_intrapred_mode_luma(aec, cu, 1, x4 + size_8x8, y4)?;
                    self.read_intrapred_mode_luma(aec, cu, 2, x4, y4 + size_8x8)?;
                    self.read_intrapred_mode_luma(aec, cu, 3, x4 + size_8x8, y4 + size_8x8)?;
                }
                _ => {}
            }
            if !self.mono {
                cu.c_ipred_mode = self.read_intra_pmode_c(aec, cu.intra_pred_modes[0])?;
            } else {
                cu.c_ipred_mode = 0;
            }
            if aec.bit_error {
                return Err(Error::Invalid("intra modes"));
            }
        }
        Ok(real)
    }

    /// `aec_read_cu_type`.
    fn read_cu_type(&mut self, aec: &mut Aec<'_>, cu: &mut Cu) -> i32 {
        const MAP: [[i32; 7]; 2] = [[-1, 0, 1, 2, 3, -1, PRED_I_NXN as i32], [-1, 0, 1, 2, 3, PRED_I_NXN as i32, 0]];
        let is8 = cu.level == B8X8_IN_BIT;
        let mut real;
        if !self.frame_type.is_intra() {
            let mut bin_idx = 0;
            let mut act_ctx = 0usize;
            let mut act_sym = 0usize;
            let max_bit = 6 - usize::from(is8);
            while act_sym < max_bit {
                let symbol = if bin_idx == 5 && cu.level != MIN_CU_SIZE_IN_BIT { aec.final_bit() } else { aec.symbol(CTX_CU_TYPE + act_ctx) };
                if aec.bit_error {
                    return -1;
                }
                bin_idx += 1;
                if symbol == 0 {
                    act_sym += 1;
                    act_ctx = (act_ctx + 1).min(5);
                } else {
                    break;
                }
            }
            real = MAP[usize::from(is8)].get(act_sym).copied().unwrap_or(0);
            if cu.level >= B16X16_IN_BIT && self.amp && (real == 2 || real == 3) && aec.symbol(CTX_AMP) == 0 {
                real = real * 2 + i32::from(aec.symbol(CTX_AMP + 1) == 0);
            }
        } else {
            real = i32::from(PRED_I_NXN);
        }

        if real <= 0 {
            let mut wsm = 0u32;
            let mut ds = 0u32;
            if self.frame_type == FrameType::F && self.wsm && self.num_refs > 1 {
                wsm = aec.continue0_ext(CTX_WPM, 2, self.num_refs as u32 - 1);
            }
            cu.weighted_skipmode = wsm as u8;
            if wsm == 0 && ((self.mhp_skip && self.frame_type == FrameType::F) || self.frame_type == FrameType::B) {
                ds = aec.continue0_ext(CTX_CU_SUBTYPE, 32768, 3);
                if ds == 3 {
                    ds += u32::from(aec.symbol(CTX_CU_SUBTYPE + 3) == 0);
                }
            }
            cu.md_directskip_mode = ds as u8;
        }
        real
    }

    /// `aec_read_intra_cu_type` + `cu_set_tu_split_type`.
    fn read_intra_cu_type(&mut self, aec: &mut Aec<'_>, cu: &mut Cu) -> Result<u8, Error> {
        let mut cu_type = PRED_I_NXN;
        let sdip = (cu.level == B32X32_IN_BIT || cu.level == B16X16_IN_BIT) && self.sdip;
        let mut tu_split = false;
        if cu.level == B8X8_IN_BIT || sdip {
            tu_split = aec.symbol(CTX_TU_SPLIT + 1 + usize::from(sdip)) != 0;
        }
        if !tu_split {
            cu_type = PRED_I_2NX2N;
        } else if sdip {
            cu_type = if aec.symbol(CTX_INTRA_PU_TYPE) != 0 { PRED_I_2NXN } else { PRED_I_NX2N };
        }
        cu.cu_type = cu_type;
        self.set_tu_split_type(cu, tu_split)?;
        Ok(cu_type)
    }

    fn set_tu_split_type(&self, cu: &mut Cu, split: bool) -> Result<(), Error> {
        const TU_SPLIT_TYPE: [[u8; 2]; 12] = [
            [TU_SPLIT_CROSS, TU_SPLIT_CROSS],
            [TU_SPLIT_CROSS, TU_SPLIT_CROSS],
            [TU_SPLIT_CROSS, TU_SPLIT_HOR],
            [TU_SPLIT_CROSS, TU_SPLIT_VER],
            [TU_SPLIT_CROSS, TU_SPLIT_HOR],
            [TU_SPLIT_CROSS, TU_SPLIT_HOR],
            [TU_SPLIT_CROSS, TU_SPLIT_VER],
            [TU_SPLIT_CROSS, TU_SPLIT_VER],
            [TU_SPLIT_NON, TU_SPLIT_INVALID],
            [TU_SPLIT_CROSS, TU_SPLIT_CROSS],
            [TU_SPLIT_INVALID, TU_SPLIT_HOR],
            [TU_SPLIT_INVALID, TU_SPLIT_VER],
        ];
        let en = if is_intra_mode(cu.cu_type) { self.sdip } else { self.nsqt } && cu.level > B8X8_IN_BIT;
        cu.trans_size = if split { TU_SPLIT_TYPE.get(usize::from(cu.cu_type)).map_or(TU_SPLIT_INVALID, |r| r[usize::from(en)]) } else { TU_SPLIT_NON };
        if cu.trans_size == TU_SPLIT_INVALID {
            return Err(Error::Invalid("transform split"));
        }
        Ok(())
    }

    /// `aec_read_inter_pred_dir`.
    fn read_inter_pred_dir(&mut self, aec: &mut Aec<'_>, cu: &mut Cu) {
        let mut pdir = PDIR_FWD as usize;
        let t = cu.cu_type;
        if self.frame_type == FrameType::B {
            if (PRED_2NX2N..=PRED_NRX2N).contains(&t) {
                pdir = read_b_pdir(aec, cu);
            }
            cu_set_pdir_bframe(cu, pdir);
        } else {
            if t == PRED_SKIP {
                let mut p = PDIR_FWD;
                if cu.weighted_skipmode != 0 || cu.md_directskip_mode == DS_DUAL_1ST || cu.md_directskip_mode == DS_DUAL_2ND {
                    p = PDIR_DUAL;
                }
                cu.b8pdir = [p; 4];
            } else {
                if self.frame_type == FrameType::F && self.num_refs > 1 && self.dhp && !(cu.level == B8X8_IN_BIT && (PRED_2NXN..=PRED_NRX2N).contains(&t)) {
                    pdir = read_pdir_dhp(aec, cu);
                }
                cu_set_pdir_pfframe(cu, pdir);
            }
            if self.frame_type != FrameType::S && cu.cu_type != PRED_SKIP {
                // cu_read_references
                let num_pu = if cu.cu_type == PRED_2NX2N { 1 } else { 2 };
                for k in 0..num_pu {
                    let r1 = if self.num_refs > 1 { read_ref_frame(aec, self.num_refs) as i8 } else { 0 };
                    let r2 = if cu.b8pdir[k] == PDIR_DUAL { i8::from(r1 == 0) } else { INVALID_REF };
                    cu.ref_idx[k] = RefIdx { r: [r1, r2] };
                }
            }
        }
    }

    /// `cu_read_intrapred_mode_luma`.
    fn read_intrapred_mode_luma(&mut self, aec: &mut Aec<'_>, cu: &mut Cu, b8: usize, bi: i32, bj: i32) -> Result<(), Error> {
        let size_in_scu = 1 << (cu.level - MIN_CU_SIZE_IN_BIT);
        let top = i32::from(self.ipm_get(bi, bj - 1));
        let left = i32::from(self.ipm_get(bi - 1, bj));
        let luma_mode = read_intra_pmode(aec);
        if aec.bit_error {
            return Err(Error::Invalid("intra luma mode"));
        }
        let mut mpm = [top.min(left), top.max(left)];
        if mpm[0] == mpm[1] {
            mpm[0] = i32::from(DC_PRED);
            mpm[1] = if mpm[1] == i32::from(DC_PRED) { i32::from(BI_PRED) } else { mpm[1] };
        }
        let real = if luma_mode < 0 {
            mpm[(luma_mode + 2).clamp(0, 1) as usize]
        } else {
            luma_mode + i32::from(luma_mode >= mpm[0]) + i32::from(luma_mode + 1 >= mpm[1])
        };
        let real = real.clamp(0, i32::from(NUM_INTRA_MODE) - 1) as i8;
        if let Some(m) = cu.intra_pred_modes.get_mut(b8) {
            *m = real;
        }
        let mut w4 = size_in_scu << 1;
        let mut h4 = size_in_scu << 1;
        match cu.trans_size {
            TU_SPLIT_HOR => h4 >>= 2,
            TU_SPLIT_VER => w4 >>= 2,
            TU_SPLIT_CROSS => {
                w4 >>= 1;
                h4 >>= 1;
            }
            _ => {}
        }
        for j in 0..h4 {
            let start = if j == h4 - 1 { 0 } else { w4 - 1 };
            for i in start..w4 {
                let idx = self.ipm_index(bi + i, bj + j);
                if let Some(p) = self.ipm.get_mut(idx) {
                    *p = real;
                }
            }
        }
        Ok(())
    }

    /// `aec_read_intra_pmode_c`.
    fn read_intra_pmode_c(&mut self, aec: &mut Aec<'_>, luma_mode: i8) -> Result<u8, Error> {
        let lmode = TAB_INTRA_MODE_LUMA2CHROMA.get(luma_mode.max(0) as usize).copied().unwrap_or(-1);
        let redundant = lmode >= 0;
        let mut act = u32::from(aec.symbol(CTX_INTRA_CHROMA + self.c_ipred_mode_ctx) == 0);
        if act != 0 {
            act = aec.unary_max(CTX_INTRA_CHROMA + 2, 0, 3) + 1;
            if redundant && act as i32 >= lmode {
                if act == 4 {
                    return Err(Error::Invalid("intra chroma mode"));
                }
                act += 1;
            }
        }
        Ok(act as u8)
    }

    /// `cu_store_references`.
    fn store_references(&mut self, cu: &Cu, pix_x: i32, pix_y: i32) {
        let bx = pix_x >> 2;
        let by = pix_y >> 2;
        for k in 0..cu.num_pu.min(4) {
            let pu = cu.pu[k].shr(2);
            let dir = cu.b8pdir[k];
            let r = cu.ref_idx[k];
            for y in 0..pu.h {
                for x in 0..pu.w {
                    let pos = self.spu(bx + pu.x + x, by + pu.y + y);
                    if let Some(p) = self.ref_idx.get_mut(pos) {
                        *p = r;
                    }
                    if let Some(p) = self.dirpred.get_mut(pos) {
                        *p = dir;
                    }
                }
            }
        }
    }

    fn set_mv(&mut self, which: usize, x4: i32, y4: i32, mv: Mv) {
        let pos = self.spu(x4, y4);
        if let Some(p) = self.tmv[which].get_mut(pos) {
            *p = mv;
        }
    }

    /// `cu_read_mv`.
    fn read_mv(&mut self, aec: &mut Aec<'_>, cu: &mut Cu, pix_x: i32, pix_y: i32) -> Result<(), Error> {
        let bframe = self.frame_type == FrameType::B;
        let b8x = pix_x >> 2;
        let b8y = pix_y >> 2;
        if self.frame_type == FrameType::F && cu.b8pdir.iter().all(|&d| d == PDIR_FWD) {
            if !(cu.level == B8X8_IN_BIT && (PRED_2NXN..=PRED_NRX2N).contains(&cu.cu_type)) {
                cu.dmh_mode = read_dmh_mode(aec, cu.level);
            } else {
                cu.dmh_mode = 0;
            }
        }
        for k in 0..cu.num_pu.min(4) {
            if cu.b8pdir[k] == PDIR_BWD {
                continue;
            }
            let pu = cu.pu[k];
            let i8_ = b8x + (pu.x >> 2);
            let j8 = b8y + (pu.y >> 2);
            let refframe = i32::from(self.ref_idx.get(self.spu(i8_, j8)).map_or(0, |r| r.r[0]));
            let pu_type = get_pu_type_for_mvp(pu.w, pu.h, pu.x, pu.y);
            let mvp = self.get_mvp_default(cu.slice_nr, pix_x + pu.x, pix_y + pu.y, 0, refframe, pu.w, pu_type);
            let mv = if self.frame_type != FrameType::S {
                let mvd = read_mvds(aec);
                self.pmvr(mvd, mvp)
            } else {
                mvp
            };
            if aec.bit_error {
                return Err(Error::Invalid("motion vector"));
            }
            let (bw, bh) = (pu.w >> 2, pu.h >> 2);
            if bframe {
                for y in 0..bh {
                    for x in 0..bw {
                        self.set_mv(0, i8_ + x, j8 + y, mv);
                    }
                }
                cu.mv[k][0] = mv;
            } else {
                let mv2 = if cu.b8pdir[k] == PDIR_DUAL {
                    let d1 = self.dist_p(refframe);
                    let d1s = self.dist_p_scale(refframe);
                    let d2 = self.dist_p(i32::from(refframe == 0));
                    let _ = d1;
                    Mv { x: scale_mv_skip(i32::from(mv.x), d2, d1s), y: scale_mv_skip(i32::from(mv.y), d2, d1s) }
                } else {
                    Mv::default()
                };
                cu.mv[k][0] = mv;
                cu.mv[k][1] = mv2;
                for y in 0..bh {
                    for x in 0..bw {
                        self.set_mv(0, i8_ + x, j8 + y, mv);
                        self.set_mv(1, i8_ + x, j8 + y, mv2);
                    }
                }
            }
        }
        if !bframe {
            return Ok(());
        }
        let d_fwd = self.dist_p(i32::from(B_FWD));
        let d_fwd_src = self.dist_p_scale(i32::from(B_FWD));
        let d_bwd = self.dist_p(i32::from(B_BWD));
        for k in 0..cu.num_pu.min(4) {
            if cu.b8pdir[k] == PDIR_FWD {
                continue;
            }
            let pu = cu.pu[k];
            let i8_ = b8x + (pu.x >> 2);
            let j8 = b8y + (pu.y >> 2);
            let refframe = i32::from(self.ref_idx.get(self.spu(i8_, j8)).map_or(0, |r| r.r[1]));
            let pu_type = get_pu_type_for_mvp(pu.w, pu.h, pu.x, pu.y);
            let mvp = self.get_mvp_default(cu.slice_nr, pix_x + pu.x, pix_y + pu.y, 1, refframe, pu.w, pu_type);
            let mv = if cu.b8pdir[k] == PDIR_SYM {
                let mv1 = self.tmv[0].get(self.spu(i8_, j8)).copied().unwrap_or_default();
                Mv { x: scale_mv_skip(i32::from(mv1.x), d_bwd, d_fwd_src).wrapping_neg(), y: scale_mv_skip(i32::from(mv1.y), d_bwd, d_fwd_src).wrapping_neg() }
            } else {
                let mvd = read_mvds(aec);
                self.pmvr(mvd, mvp)
            };
            let _ = d_fwd;
            if aec.bit_error {
                return Err(Error::Invalid("motion vector"));
            }
            cu.mv[k][1] = mv;
            for y in 0..(pu.h >> 2) {
                for x in 0..(pu.w >> 2) {
                    self.set_mv(1, i8_ + x, j8 + y, mv);
                }
            }
        }
        Ok(())
    }

    /// `fill_mv_and_ref_for_skip`.
    fn fill_mv_and_ref_for_skip(&mut self, cu: &mut Cu, col: Option<&ColInfo>, pix_x: i32, pix_y: i32, size_in_scu: i32) -> Result<(), Error> {
        match self.frame_type {
            FrameType::B => self.fill_mv_bskip(cu, col, pix_x, pix_y, size_in_scu),
            FrameType::F | FrameType::P => {
                if cu.md_directskip_mode == 0 {
                    self.fill_mv_pf_skip_temporal(cu, col, pix_x, pix_y, size_in_scu << 3)
                } else {
                    let ds = usize::from(cu.md_directskip_mode);
                    let (refs1, refs2, mv1s, mv2s) = self.get_mv_fskip_spatial();
                    let mv1 = mv1s.get(ds).copied().unwrap_or_default();
                    let mv2 = mv2s.get(ds).copied().unwrap_or_default();
                    let r = RefIdx { r: [refs1.get(ds).copied().unwrap_or(0), refs2.get(ds).copied().unwrap_or(-1)] };
                    for i in 0..4 {
                        cu.mv[i] = [mv1, mv2];
                        cu.ref_idx[i] = r;
                    }
                    let n = size_in_scu << 1;
                    let (x4, y4) = (pix_x >> 2, pix_y >> 2);
                    for j in 0..n {
                        for i in 0..n {
                            let pos = self.spu(x4 + i, y4 + j);
                            if let Some(p) = self.ref_idx.get_mut(pos) {
                                *p = r;
                            }
                            self.set_mv(0, x4 + i, y4 + j, mv1);
                            self.set_mv(1, x4 + i, y4 + j, mv2);
                        }
                    }
                    Ok(())
                }
            }
            _ => Ok(()),
        }
    }

    fn fill_mv_pf_skip_temporal(&mut self, cu: &mut Cu, col: Option<&ColInfo>, pix_x: i32, pix_y: i32, cu_size: i32) -> Result<(), Error> {
        let col = col.ok_or(Error::Invalid("missing reference picture"))?;
        let spu_x = pix_x >> 2;
        let spu_y = pix_y >> 2;
        let mut size_in_spu = cu_size >> 2;
        let r = RefIdx { r: [0, if cu.weighted_skipmode != 0 { cu.weighted_skipmode as i8 } else { INVALID_REF }] };
        let delta: Vec<i32> = (0..self.num_refs as i32).map(|i| self.dist_p(i)).collect();
        let delta_src0 = self.dist_p_scale(0);
        if cu_size != 8 {
            size_in_spu >>= 1;
        }
        for i in 0..cu.num_pu.min(4) {
            let bx = spu_x + size_in_spu * (i as i32 & 1);
            let by = spu_y + size_in_spu * (i as i32 >> 1);
            let off = self.spu(bx, by);
            let d0 = delta.first().copied().unwrap_or(1);
            let rf = col.refbuf.get(off).copied().unwrap_or(-1);
            let mv1 = if rf >= 0 {
                let t = col.mvbuf.get(off).copied().unwrap_or_default();
                let cd = col.dist_scale_refs.get(rf as usize).copied().unwrap_or(MULTI);
                Mv { x: scale_mv_skip(i32::from(t.x), d0, cd), y: scale_mv_skip(i32::from(t.y), d0, cd) }
            } else {
                Mv::default()
            };
            let mv2 = if r.r[1] != INVALID_REF {
                let dd = delta.get(r.r[1] as usize).copied().unwrap_or(1);
                Mv { x: scale_mv_skip(i32::from(mv1.x), dd, delta_src0), y: scale_mv_skip(i32::from(mv1.y), dd, delta_src0) }
            } else {
                Mv::default()
            };
            cu.mv[i] = [mv1, mv2];
            cu.ref_idx[i] = r;
            for m in 0..size_in_spu {
                for l in 0..size_in_spu {
                    let pos = self.spu(bx + l, by + m);
                    if let Some(p) = self.ref_idx.get_mut(pos) {
                        *p = r;
                    }
                    self.set_mv(0, bx + l, by + m, mv1);
                    self.set_mv(1, bx + l, by + m, mv2);
                }
            }
        }
        Ok(())
    }

    /// `get_mv_fskip_spatial`: (ref_1st, ref_2nd, mv_1st, mv_2nd) per direct/skip mode.
    #[allow(clippy::type_complexity)]
    fn get_mv_fskip_spatial(&self) -> ([i8; 5], [i8; 5], [Mv; 5], [Mv; 5]) {
        let n = &self.neighbors;
        let mut r1 = [0i8; 5];
        let mut r2 = [0i8; 5];
        let mut m1 = [Mv::default(); 5];
        let mut m2 = [Mv::default(); 5];
        let (mut bid_flag, mut fw_flag, mut bid2, mut fw2) = (0, 0, 0usize, 0usize);
        let d1 = usize::from(DS_DUAL_1ST);
        let d2 = usize::from(DS_DUAL_2ND);
        for (j, nb) in n.iter().enumerate().take(6) {
            if nb.ref_idx.r[0] != -1 && nb.ref_idx.r[1] != -1 {
                r1[d1] = nb.ref_idx.r[0];
                r2[d1] = nb.ref_idx.r[1];
                m1[d1] = nb.mv[0];
                m2[d1] = nb.mv[1];
                bid_flag += 1;
                if bid_flag == 1 {
                    bid2 = j;
                }
            } else if nb.ref_idx.r[0] != -1 && nb.ref_idx.r[1] == -1 {
                r1[DS_SINGLE_1ST] = nb.ref_idx.r[0];
                m1[DS_SINGLE_1ST] = nb.mv[0];
                fw_flag += 1;
                if fw_flag == 1 {
                    fw2 = j;
                }
            }
        }
        if bid_flag == 0 && fw_flag > 1 {
            r1[d1] = r1[DS_SINGLE_1ST];
            r2[d1] = n[fw2].ref_idx.r[0];
            m1[d1] = m1[DS_SINGLE_1ST];
            m2[d1] = n[fw2].mv[0];
        }
        if bid_flag > 1 {
            r1[d2] = n[bid2].ref_idx.r[0];
            r2[d2] = n[bid2].ref_idx.r[1];
            m1[d2] = n[bid2].mv[0];
            m2[d2] = n[bid2].mv[1];
        } else if bid_flag == 1 && fw_flag > 1 {
            r1[d2] = r1[DS_SINGLE_1ST];
            r2[d2] = n[fw2].ref_idx.r[0];
            m1[d2] = m1[DS_SINGLE_1ST];
            m2[d2] = n[fw2].mv[0];
        }
        r2[DS_SINGLE_1ST] = INVALID_REF;
        m2[DS_SINGLE_1ST] = Mv::default();
        if fw_flag == 0 && bid_flag > 1 {
            r1[DS_SINGLE_1ST] = n[bid2].ref_idx.r[0];
            m1[DS_SINGLE_1ST] = n[bid2].mv[0];
        } else if fw_flag == 0 && bid_flag == 1 {
            r1[DS_SINGLE_1ST] = r1[d1];
            m1[DS_SINGLE_1ST] = m1[d1];
        }
        r2[DS_SINGLE_2ND] = INVALID_REF;
        m2[DS_SINGLE_2ND] = Mv::default();
        if fw_flag > 1 {
            r1[DS_SINGLE_2ND] = n[fw2].ref_idx.r[0];
            m1[DS_SINGLE_2ND] = n[fw2].mv[0];
        } else if bid_flag > 1 {
            r1[DS_SINGLE_2ND] = n[bid2].ref_idx.r[1];
            m1[DS_SINGLE_2ND] = n[bid2].mv[1];
        } else if bid_flag == 1 {
            r1[DS_SINGLE_2ND] = r2[d1];
            m1[DS_SINGLE_2ND] = m2[d1];
        }
        (r1, r2, m1, m2)
    }

    /// `get_mv_bskip_spatial`.
    fn get_mv_bskip_spatial(&self, mode: usize) -> (Mv, Mv) {
        let n = &self.neighbors;
        let mut m1 = [Mv::default(); 5];
        let mut m2 = [Mv::default(); 5];
        let (mut bid_flag, mut bw_flag, mut fw_flag, mut sym_flag, mut bid2) = (0, 0, 0, 0, 0usize);
        for (j, nb) in n.iter().enumerate().take(6) {
            match nb.dir_pred {
                PDIR_BID => {
                    m2[DS_B_BID] = nb.mv[1];
                    m1[DS_B_BID] = nb.mv[0];
                    bid_flag += 1;
                    if bid_flag == 1 {
                        bid2 = j;
                    }
                }
                PDIR_SYM => {
                    m2[DS_B_SYM] = nb.mv[1];
                    m1[DS_B_SYM] = nb.mv[0];
                    sym_flag += 1;
                }
                PDIR_BWD => {
                    m2[DS_B_BWD] = nb.mv[1];
                    bw_flag += 1;
                }
                PDIR_FWD => {
                    m1[DS_B_FWD] = nb.mv[0];
                    fw_flag += 1;
                }
                _ => {}
            }
        }
        if bid_flag == 0 && fw_flag != 0 && bw_flag != 0 {
            m2[DS_B_BID] = m2[DS_B_BWD];
            m1[DS_B_BID] = m1[DS_B_FWD];
        }
        if sym_flag == 0 && bid_flag > 1 {
            m2[DS_B_SYM] = n[bid2].mv[1];
            m1[DS_B_SYM] = n[bid2].mv[0];
        } else if sym_flag == 0 && bw_flag != 0 {
            m2[DS_B_SYM] = m2[DS_B_BWD];
            m1[DS_B_SYM] = Mv { x: m2[DS_B_BWD].x.wrapping_neg(), y: m2[DS_B_BWD].y.wrapping_neg() };
        } else if sym_flag == 0 && fw_flag != 0 {
            m2[DS_B_SYM] = Mv { x: m1[DS_B_FWD].x.wrapping_neg(), y: m1[DS_B_FWD].y.wrapping_neg() };
            m1[DS_B_SYM] = m1[DS_B_FWD];
        }
        if bw_flag == 0 && bid_flag > 1 {
            m2[DS_B_BWD] = n[bid2].mv[1];
        } else if bw_flag == 0 && bid_flag != 0 {
            m2[DS_B_BWD] = m2[DS_B_BID];
        }
        if fw_flag == 0 && bid_flag > 1 {
            m1[DS_B_FWD] = n[bid2].mv[0];
        } else if fw_flag == 0 && bid_flag != 0 {
            m1[DS_B_FWD] = m1[DS_B_BID];
        }
        (m1.get(mode).copied().unwrap_or_default(), m2.get(mode).copied().unwrap_or_default())
    }

    /// `fill_mv_bskip`.
    fn fill_mv_bskip(&mut self, cu: &mut Cu, col: Option<&ColInfo>, pix_x: i32, pix_y: i32, size_in_scu: i32) -> Result<(), Error> {
        let i8_1st = pix_x >> 2;
        let j8_1st = pix_y >> 2;
        let ds = cu.md_directskip_mode;
        if ds != DS_NONE {
            let n = size_in_scu << 1;
            let r = match usize::from(ds) {
                DS_B_SYM | DS_B_BID => RefIdx { r: [B_FWD, B_BWD] },
                DS_B_BWD => RefIdx { r: [INVALID_REF, B_BWD] },
                _ => RefIdx { r: [B_FWD, INVALID_REF] },
            };
            let (mv1, mv2) = self.get_mv_bskip_spatial(usize::from(ds));
            cu.mv[0] = [mv1, mv2];
            cu.ref_idx[0] = r;
            let dir = cu.b8pdir[0];
            for y in 0..n {
                for x in 0..n {
                    let pos = self.spu(i8_1st + x, j8_1st + y);
                    if let Some(p) = self.ref_idx.get_mut(pos) {
                        *p = r;
                    }
                    if let Some(p) = self.dirpred.get_mut(pos) {
                        *p = dir;
                    }
                    self.set_mv(0, i8_1st + x, j8_1st + y, mv1);
                    self.set_mv(1, i8_1st + x, j8_1st + y, mv2);
                }
            }
        } else {
            let col = col.ok_or(Error::Invalid("missing reference picture"))?;
            let size_cu = size_in_scu << 3;
            let size_pu = size_cu >> i32::from(cu.num_pu == 4);
            let size_pu_spu = size_pu >> 2;
            let r = RefIdx { r: [B_FWD, B_BWD] };
            for i in 0..cu.num_pu.min(4) {
                let i8_ = i8_1st + (i as i32 & 1) * size_in_scu;
                let j8 = j8_1st + (i as i32 >> 1) * size_in_scu;
                let off = self.spu(i8_, j8);
                let rf = col.refbuf.get(off).copied().unwrap_or(-1);
                let (mv1, mv2);
                if rf == -1 {
                    mv1 = self.get_mvp_default(cu.slice_nr, pix_x, pix_y, 0, 0, size_cu, 0);
                    mv2 = self.get_mvp_default(cu.slice_nr, pix_x, pix_y, 1, 0, size_cu, 0);
                } else {
                    let rfu = rf.max(0) as usize;
                    let trp_src = col.dist_scale_refs.get(rfu).copied().unwrap_or(MULTI);
                    let trd = self.dist_p(i32::from(B_BWD));
                    let trb = self.dist_p(i32::from(B_FWD));
                    let t = col.mvbuf.get(off).copied().unwrap_or_default();
                    mv1 = Mv { x: scale_mv_biskip(i32::from(t.x), trb, trp_src), y: scale_mv_biskip(i32::from(t.y), trb, trp_src) };
                    mv2 =
                        Mv { x: scale_mv_biskip(i32::from(t.x), trd, trp_src).wrapping_neg(), y: scale_mv_biskip(i32::from(t.y), trd, trp_src).wrapping_neg() };
                }
                cu.mv[i] = [mv1, mv2];
                cu.ref_idx[i] = r;
                for y in 0..size_pu_spu {
                    for x in 0..size_pu_spu {
                        let pos = self.spu(i8_ + x, j8 + y);
                        if let Some(p) = self.ref_idx.get_mut(pos) {
                            *p = r;
                        }
                        if let Some(p) = self.dirpred.get_mut(pos) {
                            *p = PDIR_SYM;
                        }
                        self.set_mv(0, i8_ + x, j8 + y, mv1);
                        self.set_mv(1, i8_ + x, j8 + y, mv2);
                    }
                }
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------------------------
    // CBP and coefficients

    fn neighbor_cbp_y(&self, x4: i32, y4: i32, scu_x: i32, scu_y: i32, cur: &Cu) -> u32 {
        // get_neighbor_cu_in_slice
        let nb: Cu = if x4 < 0 || y4 < 0 || x4 >= self.w_spu || y4 >= self.h_spu {
            return 0;
        } else if (scu_x << 1) <= x4 && (scu_y << 1) <= y4 {
            *cur
        } else {
            match self.scu_at((y4 >> 1) * self.w_scu + (x4 >> 1)) {
                Some(c) if c.slice_nr == cur.slice_nr => *c,
                _ => return 0,
            }
        };
        let cbp = u32::from(nb.cbp);
        if nb.trans_size == TU_SPLIT_NON {
            return cbp & 1;
        }
        let level = nb.level - MIN_PU_SIZE_IN_BIT;
        let mask = (1 << level) - 1;
        let (mut x, mut y) = (x4 & mask, y4 & mask);
        if nb.trans_size == TU_SPLIT_VER {
            x >>= (level - 2).max(0);
            (cbp >> x.clamp(0, 31)) & 1
        } else if nb.trans_size == TU_SPLIT_HOR {
            y >>= (level - 2).max(0);
            (cbp >> y.clamp(0, 31)) & 1
        } else {
            x >>= (level - 1).max(0);
            y >>= (level - 1).max(0);
            (cbp >> (x + (y << 1)).clamp(0, 31)) & 1
        }
    }

    /// `aec_read_ctp_y`.
    fn read_ctp_y(&self, aec: &mut Aec<'_>, b8: i32, cu: &Cu, scu_x: i32, scu_y: i32) -> u8 {
        let hor = cu.trans_size == TU_SPLIT_HOR;
        let ver = cu.trans_size == TU_SPLIT_VER;
        let cu_size = 1 << cu.level;
        let (mut x, mut y) = if hor {
            (0, (cu_size * b8) >> 2)
        } else if ver {
            ((cu_size * b8) >> 2, 0)
        } else {
            ((cu_size * (b8 & 1)) >> 1, (cu_size * (b8 >> 1)) >> 1)
        };
        x += scu_x << 3;
        y += scu_y << 3;
        x >>= 2;
        y >>= 2;
        let a = if ver && b8 > 0 { (u32::from(cu.cbp) >> (b8 - 1)) & 1 } else { self.neighbor_cbp_y(x - 1, y, scu_x, scu_y, cu) };
        let b = if hor && b8 > 0 { (u32::from(cu.cbp) >> (b8 - 1)) & 1 } else { self.neighbor_cbp_y(x, y - 1, scu_x, scu_y, cu) };
        aec.symbol(CTX_CBP + (a + 2 * b) as usize) as u8
    }

    /// `cu_read_cbp` + `aec_read_cbp`.
    fn read_cbp(&mut self, aec: &mut Aec<'_>, cu: &mut Cu, scu_x: i32, scu_y: i32) -> Result<(), Error> {
        let mut cbp: u8 = 0;
        if is_inter_mode(cu.cu_type) {
            let mut bit = 0;
            if is_noskip_inter_mode(cu.cu_type) {
                bit = aec.symbol(CTX_CBP + 8);
            }
            if bit == 0 {
                let tu_split = aec.symbol(CTX_TU_SPLIT) != 0;
                self.set_tu_split_type(cu, tu_split)?;
                if !self.mono && aec.symbol(CTX_CBP + 4) != 0 {
                    if aec.symbol(CTX_CBP + 5) != 0 {
                        cbp += 48;
                    } else {
                        cbp += if aec.symbol(CTX_CBP + 5) == 1 { 32 } else { 16 };
                    }
                }
                if !tu_split {
                    if cbp == 0 {
                        cbp = 1;
                    } else {
                        cu.cbp = cbp;
                        cbp += self.read_ctp_y(aec, 0, cu, scu_x, scu_y);
                    }
                } else {
                    for b8 in 0..4 {
                        cu.cbp = cbp;
                        cbp += self.read_ctp_y(aec, b8, cu, scu_x, scu_y) << b8;
                    }
                }
                cu.cbp = cbp;
            } else {
                self.set_tu_split_type(cu, true)?;
                cu.cbp = 0;
                cbp = 0;
            }
        } else {
            if cu.cu_type == PRED_I_2NX2N {
                cu.cbp = 0;
                cbp = self.read_ctp_y(aec, 0, cu, scu_x, scu_y);
            } else {
                for b8 in 0..4 {
                    cu.cbp = cbp;
                    cbp += self.read_ctp_y(aec, b8, cu, scu_x, scu_y) << b8;
                }
            }
            if !self.mono && aec.symbol(CTX_CBP + 6) != 0 {
                if aec.symbol(CTX_CBP + 7) != 0 {
                    cbp += 48;
                } else {
                    cbp += 16 << aec.symbol(CTX_CBP + 7);
                }
            }
        }
        if cbp == 0 {
            self.last_dquant = 0;
        }
        cu.cbp = cbp;

        if self.dqp {
            let mut dqp = 0;
            if cbp != 0 {
                let max = 32 + 4 * (self.bit_depth - 8);
                // aec_read_cu_delta_qp
                let mut act = 1 - aec.symbol(CTX_DELTA_QP + usize::from(self.last_dquant != 0)) as i32;
                if act != 0 {
                    act = aec.unary_max(CTX_DELTA_QP + 2, 1, 256) as i32 + 1;
                }
                let mut d = (act + 1) >> 1;
                if act & 1 == 0 {
                    d = -d;
                }
                dqp = i32::from(d as i8).clamp(-max, max);
            }
            self.last_dquant = dqp;
            cu.qp = i32::from((dqp + self.left_cu_qp) as i8);
        } else {
            cu.qp = self.qp;
        }
        if aec.bit_error {
            return Err(Error::Invalid("coded block pattern"));
        }
        Ok(())
    }

    fn quant_params(&self, qp: i32, bit_size: i32) -> Result<(i32, i32), Error> {
        let q = usize::try_from(qp).map_err(|_| Error::Invalid("QP"))?;
        let s = *IQ_SHIFT.get(q).ok_or(Error::Invalid("QP"))?;
        let scale = *IQ_TAB.get(q).ok_or(Error::Invalid("QP"))?;
        let shift = s + (self.bit_depth + 1) + bit_size - LIMIT_BIT;
        if shift < 1 {
            return Err(Error::Invalid("QP"));
        }
        Ok((shift, scale))
    }

    pub(crate) fn chroma_qp(&self, luma_qp: i32, uv: usize) -> i32 {
        let qp = luma_qp + if uv == 0 { self.chroma_delta_u } else { self.chroma_delta_v };
        let off = (self.bit_depth - 8) << 3;
        let qp = qp - off;
        let qp = if qp < 0 { qp } else { i32::from(QP_SCALE_CR[qp.clamp(0, 63) as usize]) };
        (qp + off).clamp(0, 63 + off)
    }

    /// `cu_read_all_coeffs`.
    fn read_all_coeffs(&mut self, aec: &mut Aec<'_>, cu: &mut Cu) -> Result<(), Error> {
        let z = self.idx_cu_zscan;
        let bit_size = cu.level;
        let intra = cu.is_intra();
        let mut ctx = RunLevelCtx::luma();
        if cu.trans_size == TU_SPLIT_NON {
            if cu.cbp & 0x0F != 0 {
                let tu_level = (bit_size - 2).min(3);
                let cls = if intra { scan_type(cu.intra_pred_modes[0]) } else { INTRA_PRED_DC_DIAG };
                let swap = intra && cls == INTRA_PRED_HOR;
                let bs = 1usize << (tu_level + 2);
                let (shift, scale) = self.quant_params(cu.qp, bit_size)?;
                let base = z << 6;
                let mut buf = std::mem::take(&mut self.coeff_y);
                let res = (|| {
                    let blk = buf.get_mut(base..base + bs * bs).ok_or(Error::Invalid("coefficient buffer"))?;
                    blk.fill(0);
                    read_block_coeffs(aec, &mut ctx, cu, blk, bs, bs, true, cls, swap, scale, shift)
                })();
                self.coeff_y = buf;
                cu.dct_pattern[0] = res?;
            }
        } else {
            let wavelet = bit_size == B64X64_IN_BIT && cu.trans_size != TU_SPLIT_CROSS;
            let mut tus = init_transform_units(cu);
            if wavelet {
                for t in tus.iter_mut() {
                    *t = t.shr(1);
                }
            }
            let (shift, scale) = self.quant_params(cu.qp, cu.level - 1)?;
            for b8 in 0..4usize {
                if cu.cbp & (1 << b8) == 0 {
                    continue;
                }
                let bsx = tus[b8].w.max(0) as usize;
                let bsy = tus[b8].h.max(0) as usize;
                let cls = if intra { scan_type(cu.intra_pred_modes[b8]) } else { INTRA_PRED_DC_DIAG };
                let swap = intra && cls == INTRA_PRED_HOR && cu.cu_type != PRED_I_2NXN && cu.cu_type != PRED_I_NX2N;
                let base = (z << 6) + (b8 << ((bit_size - 1) << 1));
                let mut buf = std::mem::take(&mut self.coeff_y);
                let res = (|| {
                    let blk = buf.get_mut(base..base + bsx * bsy).ok_or(Error::Invalid("coefficient buffer"))?;
                    blk.fill(0);
                    read_block_coeffs(aec, &mut ctx, cu, blk, bsx, bsy, true, cls, swap, scale, shift)
                })();
                self.coeff_y = buf;
                cu.dct_pattern[b8] = res?;
            }
        }
        if !self.mono {
            let mut ctx = RunLevelCtx::chroma();
            let size_id = cu.level - 1;
            for uv in 0..2usize {
                if (cu.cbp >> (uv + 4)) & 1 == 0 {
                    continue;
                }
                let bs = 1usize << size_id;
                let (shift, scale) = self.quant_params(self.chroma_qp(cu.qp, uv), size_id)?;
                let base = z << 4;
                let mut buf = std::mem::take(&mut self.coeff_uv[uv]);
                let res = (|| {
                    let blk = buf.get_mut(base..base + bs * bs).ok_or(Error::Invalid("coefficient buffer"))?;
                    blk.fill(0);
                    read_block_coeffs(aec, &mut ctx, cu, blk, bs, bs, false, INTRA_PRED_DC_DIAG, false, scale, shift)
                })();
                self.coeff_uv[uv] = buf;
                cu.dct_pattern[4 + uv] = res?;
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------------------------
    // LCU-level syntax

    /// `aec_read_sao_mergeflag` etc.: returns the SAO parameters of one LCU, given those of the
    /// left and upper LCUs.
    pub(crate) fn read_sao(&self, aec: &mut Aec<'_>, on: [bool; 3], left: Option<&SaoParam>, up: Option<&SaoParam>) -> SaoParam {
        let scu_xy = self.lcu_scu_y * self.w_scu + self.lcu_scu_x;
        let cur = self.scu_at(scu_xy).map_or(-2, |c| c.slice_nr);
        let up_avail = self.lcu_scu_y != 0 && self.scu_at(scu_xy - self.w_scu).map(|c| c.slice_nr) == Some(cur);
        let left_avail = self.lcu_scu_x != 0 && self.scu_at(scu_xy - 1).map(|c| c.slice_nr) == Some(cur);
        let mut merge = 0;
        if left_avail || up_avail {
            // aec_read_sao_mergeflag
            let ctx = usize::from(left_avail) + usize::from(up_avail);
            let mut idx = 0u32;
            if ctx == 1 {
                idx = aec.symbol(CTX_SAO_MERGE);
            } else if ctx == 2 {
                idx = aec.symbol(CTX_SAO_MERGE + 1);
                if idx != 1 {
                    idx += aec.symbol(CTX_SAO_MERGE + 2) << 1;
                }
            }
            let mut ml = 0;
            let mut mt = 0;
            if left_avail {
                ml = idx & 1;
                idx >>= 1;
            }
            if up_avail && ml == 0 {
                mt = idx & 1;
            }
            merge = (ml << 1) + mt;
        }
        if merge == 2 {
            return left.copied().unwrap_or_default();
        } else if merge == 1 {
            return up.copied().unwrap_or_default();
        }
        let mut p = SaoParam::default();
        for (i, plane) in p.planes.iter_mut().enumerate() {
            if !on[i] {
                plane.mode = SaoMode::Off;
                continue;
            }
            // aec_read_sao_mode
            let t2 = u32::from(aec.symbol(CTX_SAO_MODE) == 0);
            let mode = if t2 != 0 { t2 + (u32::from(aec.eq_prob() == 0) << 1) } else { 0 };
            match mode {
                1 => {
                    plane.mode = SaoMode::New;
                    plane.type_idc = SAO_TYPE_BO;
                }
                3 => {
                    plane.mode = SaoMode::New;
                    plane.type_idc = SAO_TYPE_EO_0;
                }
                _ => plane.mode = SaoMode::Off,
            }
            if plane.mode == SaoMode::New {
                let mut offset = [0i32; 4];
                for (k, o) in offset.iter_mut().enumerate() {
                    let cls = if plane.type_idc == SAO_TYPE_BO {
                        5
                    } else if k >= 2 {
                        k + 1
                    } else {
                        k
                    };
                    *o = read_sao_offset(aec, cls);
                }
                if plane.type_idc == SAO_TYPE_BO {
                    let st0 = read_sao_type(aec, 1);
                    let st1 = read_sao_type(aec, 2) + 2;
                    let sao_type = st0 + (st1 << 5);
                    plane.offset = [0; 32];
                    let db = sao_type >> 5;
                    let b0 = sao_type - (db << 5);
                    let b1 = (b0 + db) % 32;
                    let set = |o: &mut [i32; 32], i: i32, v: i32| {
                        if let Some(x) = o.get_mut(i.rem_euclid(32) as usize) {
                            *x = v;
                        }
                    };
                    set(&mut plane.offset, b0, offset[0]);
                    set(&mut plane.offset, (b0 + 1) % 32, offset[1]);
                    set(&mut plane.offset, b1, offset[2]);
                    set(&mut plane.offset, (b1 + 1) % 32, offset[3]);
                } else {
                    plane.type_idc = read_sao_type(aec, 0);
                    plane.offset[0] = offset[0];
                    plane.offset[1] = offset[1];
                    plane.offset[2] = 0;
                    plane.offset[3] = offset[2];
                    plane.offset[4] = offset[3];
                }
            }
        }
        p
    }
}

/// SAO parameters of one LCU.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SaoParam {
    pub planes: [SaoPlane; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum SaoMode {
    #[default]
    Off,
    New,
}

pub(crate) const SAO_TYPE_EO_0: i32 = 0;
pub(crate) const SAO_TYPE_BO: i32 = 4;

#[derive(Clone, Copy, Debug)]
pub(crate) struct SaoPlane {
    pub mode: SaoMode,
    pub type_idc: i32,
    pub offset: [i32; 32],
}

impl Default for SaoPlane {
    fn default() -> Self {
        SaoPlane { mode: SaoMode::Off, type_idc: -1, offset: [0; 32] }
    }
}

fn read_sao_offset(aec: &mut Aec<'_>, cls: usize) -> i32 {
    let max = SAOCLIP.get(cls).map_or(0, |c| c[2]);
    let mut cnt = 0;
    let mut sym = if cls == 5 { aec.symbol(CTX_SAO_OFFSET) == 0 } else { aec.eq_prob() == 0 };
    while sym {
        cnt += 1;
        if cnt == max {
            break;
        }
        sym = aec.eq_prob() == 0;
        if aec.bit_error {
            break;
        }
    }
    let inv = EO_OFFSET_INV_MAP.get(cnt.clamp(0, 7) as usize).copied().unwrap_or(0);
    let mut act = match cls {
        0 => inv,
        4 => -inv,
        3 => -cnt,
        _ => cnt,
    };
    if cls == 5 && act != 0 && aec.eq_prob() != 0 {
        act = -act;
    }
    act
}

fn read_sao_type(aec: &mut Aec<'_>, act_ctx: i32) -> i32 {
    let mut act = 0;
    if act_ctx == 2 {
        let mut golomb = 1;
        loop {
            let mut temp = aec.eq_prob();
            if aec.bit_error {
                return 0;
            }
            if temp == 0 {
                act += 1 << golomb;
                golomb += 1;
            }
            if golomb == 4 {
                golomb = 0;
                temp = 1;
            }
            if temp == 1 {
                break;
            }
        }
        let mut rest = 0;
        while golomb > 0 {
            golomb -= 1;
            if aec.eq_prob() == 1 {
                rest |= 1 << golomb;
            }
        }
        act += rest;
    } else {
        let len = if act_ctx == 0 { 2 } else { 5 };
        for i in 0..len {
            act += (aec.eq_prob() as i32) << i;
        }
    }
    act
}

fn sign2(v: i32) -> i32 {
    if v >= 0 { 1 } else { -1 }
}

fn derive_mv_pred_type(ref_frame: i32, l: i32, u: i32, ur: i32, pu_type: i32) -> i32 {
    let inv = -1;
    if l != inv && u == inv && ur == inv {
        1
    } else if l == inv && u != inv && ur == inv {
        2
    } else if l == inv && u == inv && ur != inv {
        3
    } else {
        match pu_type {
            1 | 4 if l == ref_frame => 1,
            2 if ur == ref_frame => 3,
            3 if u == ref_frame => 2,
            _ => 0,
        }
    }
}

fn derive_median_mv(a: i32, b: i32, c: i32) -> i16 {
    let mvp = if (a < 0 && b > 0 && c > 0) || (a > 0 && b < 0 && c < 0) {
        (b + c) / 2
    } else if (b < 0 && a > 0 && c > 0) || (b > 0 && a < 0 && c < 0) {
        (c + a) / 2
    } else if (c < 0 && a > 0 && b > 0) || (c > 0 && a < 0 && b < 0) {
        (a + b) / 2
    } else {
        let dab = (a - b).abs();
        let dbc = (b - c).abs();
        let dca = (c - a).abs();
        let m = dab.min(dbc.min(dca));
        if m == dab {
            (a + b) / 2
        } else if m == dbc {
            (b + c) / 2
        } else {
            (c + a) / 2
        }
    };
    mvp as i16
}

fn get_pu_type_for_mvp(bsx: i32, bsy: i32, x: i32, y: i32) -> i32 {
    if bsx < bsy {
        if x == 0 { 1 } else { 2 }
    } else if bsx > bsy {
        if y == 0 { 3 } else { 4 }
    } else {
        0
    }
}

/// The top-right availability of the 4×4 block at (`x4`, `y4`) inside an LCU (`tab_TR_Avails`).
pub(crate) fn tr_avail(lcu_level: i32, x4: i32, y4: i32) -> bool {
    let n = 1 << (lcu_level - 2);
    if x4 < 0 || y4 < 0 || x4 >= n || y4 >= n {
        return false;
    }
    if y4 == 0 {
        return true;
    }
    // the TR tables: a block is available when the 4x4 block up-right of it was decoded before
    // it in z-order (the generated pattern of tab_TR_Avail8/16/32/64)
    let xr = x4 + 1;
    let yu = y4 - 1;
    if xr >= n {
        return false;
    }
    zorder(xr as u32, yu as u32) < zorder(x4 as u32, y4 as u32)
}

/// The down-left availability (`tab_DL_Avails`).
pub(crate) fn dl_avail(lcu_level: i32, x4: i32, y4: i32) -> bool {
    let n = 1 << (lcu_level - 2);
    if x4 < 0 || y4 < 0 || x4 >= n || y4 >= n {
        return false;
    }
    if x4 == 0 {
        return y4 + 1 < n;
    }
    let xl = x4 - 1;
    let yd = y4 + 1;
    if yd >= n {
        return false;
    }
    zorder(xl as u32, yd as u32) < zorder(x4 as u32, y4 as u32)
}

fn zorder(x: u32, y: u32) -> u32 {
    let mut z = 0;
    for b in 0..8 {
        z |= ((x >> b) & 1) << (2 * b);
        z |= ((y >> b) & 1) << (2 * b + 1);
    }
    z
}

fn scan_type(mode: i8) -> i32 {
    TAB_INTRA_MODE_SCAN_TYPE.get(mode.max(0) as usize).copied().unwrap_or(INTRA_PRED_DC_DIAG)
}

/// `cu_init_prediction_units`.
fn init_prediction_units(cu: &mut Cu, ft: FrameType) {
    const NUM_PU: [usize; 12] = [1, 1, 2, 2, 2, 2, 2, 2, 1, 4, 4, 4];
    const INFO: [[Cb; 4]; 13] = [
        [Cb::new(0, 0, 8, 8), Cb::new(0, 0, 0, 0), Cb::new(0, 0, 0, 0), Cb::new(0, 0, 0, 0)],
        [Cb::new(0, 0, 8, 8), Cb::new(0, 0, 0, 0), Cb::new(0, 0, 0, 0), Cb::new(0, 0, 0, 0)],
        [Cb::new(0, 0, 8, 4), Cb::new(0, 4, 8, 4), Cb::new(0, 0, 0, 0), Cb::new(0, 0, 0, 0)],
        [Cb::new(0, 0, 4, 8), Cb::new(4, 0, 4, 8), Cb::new(0, 0, 0, 0), Cb::new(0, 0, 0, 0)],
        [Cb::new(0, 0, 8, 2), Cb::new(0, 2, 8, 6), Cb::new(0, 0, 0, 0), Cb::new(0, 0, 0, 0)],
        [Cb::new(0, 0, 8, 6), Cb::new(0, 6, 8, 2), Cb::new(0, 0, 0, 0), Cb::new(0, 0, 0, 0)],
        [Cb::new(0, 0, 2, 8), Cb::new(2, 0, 6, 8), Cb::new(0, 0, 0, 0), Cb::new(0, 0, 0, 0)],
        [Cb::new(0, 0, 6, 8), Cb::new(6, 0, 2, 8), Cb::new(0, 0, 0, 0), Cb::new(0, 0, 0, 0)],
        [Cb::new(0, 0, 8, 8), Cb::new(0, 0, 0, 0), Cb::new(0, 0, 0, 0), Cb::new(0, 0, 0, 0)],
        [Cb::new(0, 0, 4, 4), Cb::new(4, 0, 4, 4), Cb::new(0, 4, 4, 4), Cb::new(4, 4, 4, 4)],
        [Cb::new(0, 0, 8, 2), Cb::new(0, 2, 8, 2), Cb::new(0, 4, 8, 2), Cb::new(0, 6, 8, 2)],
        [Cb::new(0, 0, 2, 8), Cb::new(2, 0, 2, 8), Cb::new(4, 0, 2, 8), Cb::new(6, 0, 2, 8)],
        [Cb::new(0, 0, 4, 4), Cb::new(4, 0, 4, 4), Cb::new(0, 4, 4, 4), Cb::new(4, 4, 4, 4)],
    ];
    let shift = cu.level - MIN_CU_SIZE_IN_BIT;
    let mode = usize::from(cu.cu_type).min(11);
    let ds = cu.md_directskip_mode;
    if cu.cu_type == PRED_SKIP {
        if cu.level > 3 && (ft == FrameType::P || (ft == FrameType::F && ds == DS_NONE) || (ft == FrameType::B && ds == DS_NONE)) {
            cu.num_pu = 4;
            for i in 0..4 {
                cu.pu[i] = INFO[12][i].shl(shift);
            }
        } else {
            cu.num_pu = 1;
            cu.pu[0] = INFO[0][0].shl(shift);
        }
    } else {
        cu.num_pu = NUM_PU[mode];
        for i in 0..cu.num_pu {
            cu.pu[i] = INFO[mode][i].shl(shift);
        }
    }
}

/// `cu_init_transform_units`.
pub(crate) fn init_transform_units(cu: &Cu) -> [Cb; 4] {
    const INFO: [[Cb; 4]; 4] = [
        [Cb::new(0, 0, 8, 8), Cb::new(0, 0, 0, 0), Cb::new(0, 0, 0, 0), Cb::new(0, 0, 0, 0)],
        [Cb::new(0, 0, 8, 2), Cb::new(0, 2, 8, 2), Cb::new(0, 4, 8, 2), Cb::new(0, 6, 8, 2)],
        [Cb::new(0, 0, 2, 8), Cb::new(2, 0, 2, 8), Cb::new(4, 0, 2, 8), Cb::new(6, 0, 2, 8)],
        [Cb::new(0, 0, 4, 4), Cb::new(4, 0, 4, 4), Cb::new(0, 4, 4, 4), Cb::new(4, 4, 4, 4)],
    ];
    let shift = cu.level - MIN_CU_SIZE_IN_BIT;
    let t = usize::from(cu.trans_size).min(3);
    [INFO[t][0].shl(shift), INFO[t][1].shl(shift), INFO[t][2].shl(shift), INFO[t][3].shl(shift)]
}

fn cu_set_pdir_pfframe(cu: &mut Cu, pdir: usize) {
    const P0: [i8; 4] = [PDIR_FWD, PDIR_FWD, PDIR_DUAL, PDIR_DUAL];
    const P1: [i8; 4] = [PDIR_FWD, PDIR_DUAL, PDIR_FWD, PDIR_DUAL];
    let t = cu.cu_type;
    let pdir = pdir.min(3);
    if t == PRED_2NX2N {
        let p = if pdir == PDIR_FWD as usize { PDIR_FWD } else { PDIR_DUAL };
        cu.b8pdir = [p; 4];
    } else if is_hor_pu_part(t) || is_ver_pu_part(t) {
        cu.b8pdir = [P0[pdir], P1[pdir], P0[pdir], P1[pdir]];
    } else {
        cu.b8pdir = [PDIR_INVALID; 4];
    }
}

fn cu_set_pdir_bframe(cu: &mut Cu, pdir: usize) {
    const P0: [i8; 16] = [
        PDIR_FWD, PDIR_BWD, PDIR_FWD, PDIR_BWD, PDIR_FWD, PDIR_BWD, PDIR_SYM, PDIR_SYM, PDIR_SYM, PDIR_FWD, PDIR_BWD, PDIR_SYM, PDIR_BID, PDIR_BID, PDIR_BID,
        PDIR_BID,
    ];
    const P1: [i8; 16] = [
        PDIR_FWD, PDIR_BWD, PDIR_BWD, PDIR_FWD, PDIR_SYM, PDIR_SYM, PDIR_FWD, PDIR_BWD, PDIR_SYM, PDIR_BID, PDIR_BID, PDIR_BID, PDIR_FWD, PDIR_BWD, PDIR_SYM,
        PDIR_BID,
    ];
    const BSKIP: [i8; 5] = [PDIR_SYM, PDIR_BID, PDIR_BWD, PDIR_SYM, PDIR_FWD];
    const REFIDX: [[i8; 2]; 4] = [[B_FWD, INVALID_REF], [INVALID_REF, B_BWD], [B_FWD, B_BWD], [B_FWD, B_BWD]];
    let t = cu.cu_type;
    let pdir = pdir.min(15);
    if t == PRED_SKIP {
        let p = BSKIP.get(usize::from(cu.md_directskip_mode)).copied().unwrap_or(PDIR_SYM);
        cu.b8pdir = [p; 4];
    } else if t == PRED_2NX2N {
        cu.b8pdir = [pdir as i8; 4];
    } else if is_hor_pu_part(t) || is_ver_pu_part(t) {
        cu.b8pdir = [P0[pdir], P1[pdir], P0[pdir], P1[pdir]];
    } else {
        cu.b8pdir = [PDIR_INVALID; 4];
    }
    for i in 0..4 {
        let r = usize::try_from(cu.b8pdir[i]).ok().and_then(|d| REFIDX.get(d)).copied().unwrap_or([INVALID_REF, INVALID_REF]);
        cu.ref_idx[i] = RefIdx { r };
    }
}

// ---------------------------------------------------------------------------------------------
// syntax elements without picture state

fn read_cu_type_sframe(aec: &mut Aec<'_>) -> i32 {
    const MAP: [i32; 3] = [-1, PRED_SKIP as i32, PRED_I_NXN as i32];
    let mut act_ctx = 0;
    let mut t = 0;
    loop {
        if aec.symbol(CTX_CU_TYPE + act_ctx) == 0 {
            t += 1;
            act_ctx += 1;
        } else {
            break;
        }
        if t >= 2 {
            break;
        }
    }
    MAP[t]
}

fn read_b_pdir(aec: &mut Aec<'_>, cu: &Cu) -> usize {
    const DIR2OFFSET: [[usize; 4]; 4] = [[0, 2, 4, 9], [3, 1, 5, 10], [6, 7, 8, 11], [12, 13, 14, 15]];
    const NEW_PDIR: [usize; 4] = [3, 1, 0, 2];
    let c = CTX_PU_TYPE_INDEX;
    let t = cu.cu_type;
    if t == PRED_2NX2N {
        let mut s = aec.continue0_ext(c, 32768, 2) as usize;
        if s == 2 {
            s += usize::from(aec.symbol(c + 2) == 0);
        }
        s
    } else if (PRED_2NXN..=PRED_NRX2N).contains(&t) && cu.level == B8X8_IN_BIT {
        let cm = CTX_B_PU_TYPE_MIN;
        let p0 = usize::from(aec.symbol(cm) == 0);
        let p1 = if aec.symbol(cm + 1) != 0 { p0 } else { 1 - p0 };
        DIR2OFFSET[p0][p1]
    } else {
        let mut s = aec.continue0_ext(c + 3, 32768, 2) as usize;
        if s == 2 {
            s += usize::from(aec.symbol(c + 5) == 0);
        }
        let p0 = s.min(3);
        let p1 = if aec.symbol(c + 6) != 0 {
            p0
        } else {
            match p0 {
                0 => {
                    if aec.symbol(c + 7) != 0 {
                        1
                    } else if aec.symbol(c + 8) != 0 {
                        2
                    } else {
                        3
                    }
                }
                1 => {
                    if aec.symbol(c + 9) != 0 {
                        0
                    } else if aec.symbol(c + 10) != 0 {
                        2
                    } else {
                        3
                    }
                }
                2 => {
                    if aec.symbol(c + 11) != 0 {
                        0
                    } else if aec.symbol(c + 12) != 0 {
                        1
                    } else {
                        3
                    }
                }
                _ => {
                    if aec.symbol(c + 13) != 0 {
                        0
                    } else if aec.symbol(c + 14) != 0 {
                        1
                    } else {
                        2
                    }
                }
            }
        };
        DIR2OFFSET[NEW_PDIR[p0]][NEW_PDIR[p1]]
    }
}

fn read_pdir_dhp(aec: &mut Aec<'_>, cu: &Cu) -> usize {
    let c = CTX_PU_TYPE_INDEX;
    if cu.cu_type == PRED_2NX2N {
        aec.symbol(c) as usize
    } else {
        let p0 = aec.symbol(c + 1) as usize;
        let p1 = if aec.symbol(c + 2) != 0 { p0 } else { 1 - p0.min(1) };
        p0.min(1) * 2 + p1.min(1)
    }
}

fn read_ref_frame(aec: &mut Aec<'_>, num_refs: usize) -> usize {
    let c = CTX_REF_IDX;
    if aec.symbol(c) != 0 {
        0
    } else {
        let mut act_ctx = 1;
        let mut s = 1;
        while s != num_refs - 1 && aec.symbol(c + act_ctx) == 0 {
            s += 1;
            act_ctx = (act_ctx + 1).min(2);
            if aec.bit_error {
                break;
            }
        }
        s
    }
}

fn read_dmh_mode(aec: &mut Aec<'_>, level: i32) -> u8 {
    let c = CTX_PU_TYPE_INDEX + ((level - 3).max(0) as usize) * 3 + 3;
    if aec.symbol(c) == 0 {
        0
    } else if aec.symbol(c + 1) == 0 {
        (3 + aec.eq_prob()) as u8
    } else if aec.symbol(c + 2) == 0 {
        (7 + aec.eq_prob()) as u8
    } else {
        let b3 = aec.eq_prob();
        let b4 = aec.eq_prob();
        (1 + (b3 << 2) + b4) as u8
    }
}

fn read_mvd(aec: &mut Aec<'_>, c: usize) -> i32 {
    let mut act: i32;
    if aec.symbol(c) == 0 {
        act = 0;
    } else if aec.symbol(c + 1) == 0 {
        act = 1;
    } else if aec.symbol(c + 2) == 0 {
        act = 2;
    } else {
        let add = aec.eq_prob() as i32;
        act = 0;
        let mut golomb = 0;
        loop {
            let l = aec.eq_prob();
            if aec.bit_error || golomb > 30 {
                return 0;
            }
            if l == 0 {
                act += 1 << golomb;
                golomb += 1;
            } else {
                break;
            }
        }
        let mut bin = 0;
        while golomb > 0 {
            golomb -= 1;
            if aec.eq_prob() != 0 {
                bin |= 1 << golomb;
            }
        }
        act = act.wrapping_add(bin);
        act = act.wrapping_shl(1).wrapping_add(3 + add);
    }
    if act != 0 && aec.eq_prob() != 0 {
        act = -act;
    }
    act
}

fn read_mvds(aec: &mut Aec<'_>) -> Mv {
    let x = read_mvd(aec, CTX_MVD) as i16;
    let y = read_mvd(aec, CTX_MVD + 3) as i16;
    Mv { x, y }
}

fn read_intra_pmode(aec: &mut Aec<'_>) -> i32 {
    let c = CTX_INTRA_LUMA;
    if aec.symbol(c) == 1 {
        aec.symbol(c + 6) as i32 - 2
    } else {
        let mut s = (aec.symbol(c + 1) << 4) as i32;
        s += (aec.symbol(c + 2) << 3) as i32;
        s += (aec.symbol(c + 3) << 2) as i32;
        s += (aec.symbol(c + 4) << 1) as i32;
        s += aec.symbol(c + 5) as i32;
        s
    }
}

// ---------------------------------------------------------------------------------------------
// run-level coefficients

/// The contexts used for one component's coefficients (`runlevel_t` context pointers).
struct RunLevelCtx {
    run: usize,
    level: usize,
    sig_cg: usize,
    last_cg: usize,
    last_pos: usize,
}

impl RunLevelCtx {
    fn luma() -> Self {
        RunLevelCtx { run: CTX_COEFF_RUN, level: CTX_COEFF_LEVEL, sig_cg: CTX_SIG_CG, last_cg: CTX_LAST_CG, last_pos: CTX_LAST_POS }
    }
    fn chroma() -> Self {
        RunLevelCtx { run: CTX_COEFF_RUN + 33, level: CTX_COEFF_LEVEL + 20, sig_cg: CTX_SIG_CG + 2, last_cg: CTX_LAST_CG + 6, last_pos: CTX_LAST_POS + 48 }
    }
}

#[allow(clippy::too_many_arguments)]
fn read_last_cg_pos(aec: &mut Aec<'_>, c: usize, cu: &Cu, luma: bool, num_cg: usize, dc_diag: bool, mut nx1: u32, mut ny1: u32) -> (usize, usize, usize) {
    if luma && dc_diag {
        std::mem::swap(&mut nx1, &mut ny1);
    }
    let (mut x, mut y);
    let idx;
    if num_cg == 4 {
        let i = aec.continue0_ext(c, 2, 3) as usize;
        if luma && cu.trans_size == TU_SPLIT_HOR {
            x = i;
            y = 0;
        } else if luma && cu.trans_size == TU_SPLIT_VER {
            x = 0;
            y = i;
        } else {
            x = i & 1;
            y = i >> 1;
        }
        idx = i;
    } else {
        let bit = aec.symbol(c + 3);
        if bit == 0 {
            x = 0;
            y = 0;
        } else {
            x = aec.continue0(c + 4, nx1) as usize;
            if x == 0 {
                y = 0;
                if ny1 != 1 {
                    y = aec.continue0(c + 5, ny1.saturating_sub(1)) as usize;
                }
                y += 1;
            } else {
                y = aec.continue0(c + 5, ny1) as usize;
            }
        }
        if luma && dc_diag {
            std::mem::swap(&mut x, &mut y);
        }
        let get = |t: &[u8], i: usize| usize::from(t.get(i).copied().unwrap_or(0));
        idx = if luma && cu.trans_size == TU_SPLIT_HOR {
            get(&RASTER2ZZ_2X8, y * 8 + x)
        } else if luma && cu.trans_size == TU_SPLIT_VER {
            get(&RASTER2ZZ_8X2, y * 2 + x)
        } else if num_cg == 16 {
            get(&RASTER2ZZ_4X4, y * 4 + x)
        } else {
            get(&RASTER2ZZ_8X8, y * 8 + x)
        };
    }
    (idx, x, y)
}

#[allow(clippy::too_many_arguments)]
fn read_last_coeff_pos_in_cg(aec: &mut Aec<'_>, c: usize, rank: i32, cg_x: usize, cg_y: usize, luma: bool, one_cg: bool, dc_diag: bool) -> usize {
    let offset = if !luma {
        if one_cg { 0 } else { 4 + usize::from(rank == 0) * 4 }
    } else if one_cg {
        40 + usize::from(dc_diag) * 4
    } else if cg_x != 0 && cg_y != 0 {
        32 + usize::from(rank == 0) * 4
    } else {
        (4 * usize::from(rank == 0) + 2 * usize::from(cg_x == 0 && cg_y == 0) + usize::from(dc_diag)) * 4
    };
    let c = c + offset;
    let mut xx = aec.continue0_ext(c, 1, 3) as usize;
    let mut yy = aec.continue0_ext(c + 2, 1, 3) as usize;
    if cg_x == 0 && cg_y > 0 && dc_diag {
        std::mem::swap(&mut xx, &mut yy);
    }
    if rank != 0 {
        xx = 3usize.saturating_sub(xx);
        if dc_diag {
            yy = 3usize.saturating_sub(yy);
        }
    }
    TAB_SCAN_COEFF_POS_IN_CG[yy.min(3)][xx.min(3)]
}

fn read_run(aec: &mut Aec<'_>, c: usize, pos: i32, one_cg: bool, first_cg: bool, luma: bool, dc_diag: bool) -> i32 {
    let mut run = 0;
    let mut offset = 0usize;
    let mut ctxpos = 0;
    while run != pos {
        if ctxpos < pos {
            if luma {
                let one = if one_cg { 0 } else { 4 };
                let moddiv = if !dc_diag {
                    let i = (pos - 1 - ctxpos).clamp(0, 15) as usize;
                    (TAB_SCAN_4X4[i][1] as i32 + 1) >> 1
                } else if pos < ctxpos + 4 {
                    0
                } else if pos < ctxpos + 11 {
                    1
                } else {
                    2
                };
                offset = ((if first_cg { if pos == ctxpos + 1 { 0 } else { 1 + moddiv } } else { 4 + moddiv }) + one) as usize;
            } else {
                let one = if one_cg { 0 } else { 3 };
                let moddiv = i32::from(pos >= 6 + ctxpos);
                offset = ((if first_cg { if pos == ctxpos + 1 { 0 } else { 1 + moddiv } } else { 3 + moddiv }) + one) as usize;
            }
        }
        if aec.symbol(c + offset.min(10)) != 0 || aec.bit_error {
            break;
        }
        run += 1;
        ctxpos += 1;
    }
    run
}

/// `cu_get_block_coeffs` / `aec_read_run_level`: reads one transform block's coefficients into
/// `res` (`w`×`h`, row stride `w`), dequantised; returns the DCT pattern.
#[allow(clippy::too_many_arguments)]
fn read_block_coeffs(
    aec: &mut Aec<'_>,
    ctx: &mut RunLevelCtx,
    cu: &Cu,
    res: &mut [i16],
    w: usize,
    h: usize,
    luma: bool,
    cls: i32,
    swap: bool,
    scale: i32,
    shift: i32,
) -> Result<u8, Error> {
    let dc_diag = cls == INTRA_PRED_DC_DIAG;
    let num_cg_total = (w * h) >> 4;
    let add = 1i64 << (shift - 1);
    let cgs = cg_scan(w >> 2, h >> 2);
    let one_cg = num_cg_total == 1;
    let mut dct_pattern = DCT_QUAD;
    let (mut wh, mut hh, mut wq, mut hq) = if w == h {
        (w >> 1, h >> 1, w >> 2, h >> 2)
    } else if w > h {
        (w >> 1, h, w >> 2, h)
    } else {
        (w, h >> 1, w, h >> 2)
    };
    wh >>= 2;
    hh >>= 2;
    wq >>= 2;
    hq >>= 2;

    let (mut cgx, mut cgy) = (0usize, 0usize);
    let mut cg_pos = 0usize;
    if num_cg_total > 1 {
        let last = cgs.get(num_cg_total - 1).copied().unwrap_or([0, 0]);
        let (i, x, y) = read_last_cg_pos(aec, ctx.last_cg, cu, luma, num_cg_total, dc_diag, last[0] as u32, last[1] as u32);
        cg_pos = i;
        cgx = x;
        cgy = y;
    }
    if cg_pos >= num_cg_total {
        return Err(Error::Invalid("last coefficient group"));
    }
    let num_cg = cg_pos + 1;
    let mut rank = 0i32;
    let mut runlevel = [(0i32, 0i32); 16];
    for _ in 0..num_cg {
        let first_cg = cg_pos == 0;
        let mut nonzero = true;
        if rank > 0 {
            let c = usize::from(luma && cg_pos != 0);
            let p = cgs.get(cg_pos).copied().unwrap_or([0, 0]);
            cgx = p[0];
            cgy = p[1];
            nonzero = aec.symbol(ctx.sig_cg + c) != 0;
        }
        if nonzero {
            let mut n_pairs = 0usize;
            let mut pos = read_last_coeff_pos_in_cg(aec, ctx.last_pos, rank, cgx, cgy, luma, one_cg, dc_diag) as i32;
            for _ in 0..16 {
                let mut level: i32 = 1;
                if aec.final_bit() != 0 {
                    let mut golomb = 0;
                    loop {
                        let l = aec.eq_prob();
                        if aec.bit_error || golomb > 30 {
                            return Err(Error::Invalid("coefficient level"));
                        }
                        if l != 0 {
                            break;
                        }
                        level += 1 << golomb;
                        golomb += 1;
                    }
                    let mut bin = 0i32;
                    while golomb > 0 {
                        golomb -= 1;
                        bin |= (aec.eq_prob() as i32) << golomb;
                    }
                    level = level.wrapping_add(bin).wrapping_add(32);
                } else {
                    let idx = ((n_pairs + 1) >> 1).min(2) as i32;
                    let c = ctx.level + 10 * usize::from(first_cg && pos < 3) + rank.min(idx + 2) as usize + ((5 * idx) >> 1) as usize;
                    level += aec.continue0(c, 31) as i32;
                }
                if aec.bit_error {
                    return Err(Error::Invalid("coefficient level"));
                }
                // get_abssum_of_n_last_coeffs
                let mut abs5 = 0i32;
                let mut n = 0i32;
                for k in (0..n_pairs).rev() {
                    n += runlevel[k].1;
                    if n >= 6 {
                        break;
                    }
                    abs5 += runlevel[k].0.abs();
                    n += 1;
                }
                abs5 = (abs5 + level) >> 1;
                let run_ctx = ctx.run + abs5.clamp(0, 2) as usize * 11;
                let run = if pos > 0 { read_run(aec, run_ctx, pos, one_cg, first_cg, luma, dc_diag) } else { 0 };
                if aec.bit_error {
                    return Err(Error::Invalid("coefficient run"));
                }
                if let Some(rl) = runlevel.get_mut(n_pairs) {
                    *rl = (level, run);
                }
                n_pairs += 1;
                if level > T_CHR[rank.clamp(0, 4) as usize] {
                    rank = TAB_RANK[level.clamp(0, 5) as usize];
                }
                if run == pos {
                    break;
                }
                pos -= run + 1;
            }
            for rl in runlevel.iter_mut().take(n_pairs) {
                if aec.eq_prob() != 0 {
                    rl.0 = -rl.0;
                }
            }
            // run-level to coefficients
            let (mut gx, mut gy) = (cgx, cgy);
            if swap {
                std::mem::swap(&mut gx, &mut gy);
            }
            let base = w * (gy << 2) + (gx << 2);
            let mut ctr: i32 = -1;
            for k in (0..n_pairs).rev() {
                let (lv, run) = runlevel[k];
                if !(0..16).contains(&run) {
                    return Err(Error::Invalid("coefficient run"));
                }
                ctr += run + 1;
                let s = TAB_SCAN_4X4.get(ctr as usize).ok_or(Error::Invalid("coefficient run"))?;
                let (xi, yi) = if swap { (s[1], s[0]) } else { (s[0], s[1]) };
                let v = (i64::from(lv) * i64::from(scale) + add) >> shift;
                let p = res.get_mut(base + yi * w + xi).ok_or(Error::Invalid("coefficient position"))?;
                *p = v.clamp(-32768, 32767) as i16;
            }
            if gy >= hh || gx >= wh {
                dct_pattern = DCT_DEFAULT;
            } else if (gy >= hq || gx >= wq) && dct_pattern != DCT_DEFAULT {
                dct_pattern = DCT_HALF;
            }
        }
        cg_pos = cg_pos.wrapping_sub(1);
    }
    Ok(dct_pattern)
}
