// SPDX-License-Identifier: BSD-3-Clause
// Derived from uavs3d (`source/decore/com_def.h`, `com_type.h`), Copyright (c) 2018-2022 Peking University
// Shenzhen Graduate School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation.
//! Shared constants and plain data types.

pub(crate) const MAX_CU_SIZE: usize = 128;
pub(crate) const REFI_INVALID: i8 = -1;
pub(crate) const MAX_SKIP_NUM: usize = 4 + ALLOWED_HMVP_NUM;
pub(crate) const TRADITIONAL_SKIP_NUM: usize = 4;
pub(crate) const ALLOWED_HMVP_NUM: usize = 8;
pub(crate) const UMVE_BASE_NUM: i32 = 2;
pub(crate) const UMVE_REFINE_STEP: u32 = 5;
pub(crate) const UMVE_MAX_REFINE_NUM: i32 = 20;
pub(crate) const MV_SCALE_PREC: i32 = 14;
pub(crate) const AFF_MAX_NUM_MRG: usize = 5;
pub(crate) const AFF_BITSIZE: u32 = 4;
pub(crate) const CPMV_MAX: i32 = (1 << 17) - 1;
pub(crate) const CPMV_MIN: i32 = -(1 << 17);

pub(crate) const PRED_L0: u8 = 0;
pub(crate) const PRED_L1: u8 = 1;
pub(crate) const PRED_BI: u8 = 2;

pub(crate) const MODE_INTRA: u8 = 0;
pub(crate) const MODE_INTER: u8 = 1;
pub(crate) const MODE_SKIP: u8 = 2;
pub(crate) const MODE_DIR: u8 = 3;

pub(crate) const IPD_DC: u8 = 0;
pub(crate) const IPD_PLN: u8 = 1;
pub(crate) const IPD_BI: u8 = 2;
pub(crate) const IPD_DM_C: u8 = 0;
pub(crate) const IPD_DC_C: u8 = 1;
pub(crate) const IPD_HOR_C: u8 = 2;
pub(crate) const IPD_VER_C: u8 = 3;
pub(crate) const IPD_BI_C: u8 = 4;
pub(crate) const IPD_TSCPM_C: u8 = 5;
pub(crate) const IPD_HOR: u8 = 24;
pub(crate) const IPD_VER: u8 = 12;
pub(crate) const IPD_CNT: u8 = 33;
pub(crate) const IPD_IPCM: u8 = 33;
pub(crate) const IPD_DIA_R: u8 = 18;
pub(crate) const IPD_DIA_L: u8 = 3;

pub(crate) const SPLIT_NONE: u8 = 0;
pub(crate) const SPLIT_BI_VER: u8 = 1;
pub(crate) const SPLIT_BI_HOR: u8 = 2;
pub(crate) const SPLIT_EQT_VER: u8 = 3;
pub(crate) const SPLIT_EQT_HOR: u8 = 4;
pub(crate) const SPLIT_QUAD: u8 = 5;

pub(crate) const NO_MODE_CONS: u8 = 0;
pub(crate) const ONLY_INTER: u8 = 1;
pub(crate) const ONLY_INTRA: u8 = 2;

pub(crate) const TREE_LC: u8 = 0;
pub(crate) const TREE_L: u8 = 1;
pub(crate) const TREE_C: u8 = 2;

pub(crate) const SIZE_2NX2N: u8 = 0;
pub(crate) const SIZE_2NXNU: u8 = 1;
pub(crate) const SIZE_2NXND: u8 = 2;
pub(crate) const SIZE_NLX2N: u8 = 3;
pub(crate) const SIZE_NRX2N: u8 = 4;
pub(crate) const SIZE_NXN: u8 = 5;
pub(crate) const SIZE_2NXHN: u8 = 6;
pub(crate) const SIZE_HNX2N: u8 = 7;

pub(crate) const AVAIL_UP: u8 = 1;
pub(crate) const AVAIL_LE: u8 = 2;
pub(crate) const AVAIL_UL: u8 = 4;

pub(crate) type Mv = [i16; 2];
/// Motion of both lists: vectors and reference indices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Motion {
    pub mv: [Mv; 2],
    pub refi: [i8; 2],
}

/// Per 4x4 block flags (`com_scu_t`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Scu {
    pub coded: bool,
    pub intra: bool,
    pub inter: bool,
    pub skip: bool,
    pub cbf: bool,
    pub affine: u8,
}

/// `same_motion`: equal reference indices, and equal vectors for the lists in use.
pub(crate) fn same_motion(mv1: &[Mv; 2], r1: &[i8; 2], mv2: &[Mv; 2], r2: &[i8; 2]) -> bool {
    r1 == r2 && (r1[0] == REFI_INVALID || mv1[0] == mv2[0]) && (r1[1] == REFI_INVALID || mv1[1] == mv2[1])
}

pub(crate) fn clip3(lo: i32, hi: i32, v: i32) -> i32 {
    v.max(lo).min(hi)
}

pub(crate) fn log2_u(v: usize) -> u32 {
    v.max(1).ilog2()
}
