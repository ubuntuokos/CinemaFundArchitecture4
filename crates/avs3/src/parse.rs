// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from uavs3d 1.2 (commit 0e20d2c2), Copyright (c) 2018-2022 Peking University Shenzhen Graduate School,
// Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation, BSD 3-clause licence (docs/licensing.md).
//! Coding-unit level syntax: the CU header, coefficient blocks, SAO / ALF flags and the LCU QP delta.

use crate::blocks::{self, NO_SPLIT, SIZE_2NX2N, SIZE_2NXHN, SIZE_2NXND, SIZE_2NXNU, SIZE_HNX2N, SIZE_NLX2N, SIZE_NRX2N, SIZE_NXN};
use crate::blocks::{SPLIT_BI_HOR, SPLIT_BI_VER, SPLIT_EQT_HOR, SPLIT_EQT_VER, SPLIT_QUAD};
use crate::core::{
    CPMV_MAX, CPMV_MIN, Core, MODE_DIR, MODE_INTER, MODE_INTRA, MODE_SKIP, Motion, NO_MODE_CONS, ONLY_INTER, ONLY_INTRA, REFI_INVALID, SCU_AFFINE_SHIFT,
    SCU_CBF, SCU_CODED, SCU_INTER, SCU_INTRA, SCU_SKIP, SaoParam, TREE_C, TREE_L, TREE_LC,
};
use crate::header::SliceType;
use crate::intra::{IPD_BI, IPD_BI_C, IPD_DC, IPD_DC_C, IPD_DM_C, IPD_HOR, IPD_HOR_C, IPD_IPCM, IPD_TSCPM_C, IPD_VER, IPD_VER_C};
use crate::scan::scan;

pub(crate) const PRED_L0: u8 = 0;
pub(crate) const PRED_L1: u8 = 1;
pub(crate) const PRED_BI: u8 = 2;

const DQ_SHIFT: [u8; 80] = [
    14, 14, 14, 14, 14, 14, 14, 14, 13, 13, 13, 13, 13, 13, 13, 13, 13, 12, 12, 12, 12, 12, 12, 12, 11, 11, 11, 11, 11, 11, 11, 11, 11, 10, 10, 10, 10, 10, 10,
    10, 10, 9, 9, 9, 9, 9, 9, 9, 9, 8, 8, 8, 8, 8, 8, 8, 7, 7, 7, 7, 7, 7, 7, 7, 6, 6, 6, 6, 6, 6, 6, 6, 5, 5, 5, 5, 5, 5, 5, 5,
];

const DQ_SCALE: [u16; 80] = [
    32768, 36061, 38968, 42495, 46341, 50535, 55437, 60424, 32932, 35734, 38968, 42495, 46177, 50535, 55109, 59933, 65535, 35734, 38968, 42577, 46341, 50617,
    55027, 60097, 32809, 35734, 38968, 42454, 46382, 50576, 55109, 60056, 65535, 35734, 38968, 42495, 46320, 50515, 55109, 60076, 65535, 35744, 38968, 42495,
    46341, 50535, 55099, 60087, 65535, 35734, 38973, 42500, 46341, 50535, 55109, 60097, 32771, 35734, 38965, 42497, 46341, 50535, 55109, 60099, 32768, 36061,
    38968, 42495, 46341, 50535, 55437, 60424, 32932, 35734, 38968, 42495, 46177, 50535, 55109, 59933,
];

fn is_conv(m: i32) -> bool {
    matches!(m, IPD_VER | IPD_HOR | IPD_DC | IPD_BI)
}

impl Core {
    fn scu_flag(&self, scup: i32, bit: u8) -> u32 {
        u32::from(self.scu(scup) & bit != 0)
    }

    // ------------------------------------------------------------ small elements

    fn dec_refidx(&mut self, num_refp: usize) -> i8 {
        if num_refp > 1 { self.lbac.truncate_unary(&mut self.ctx.refi, 3, num_refp as u32) as i8 } else { 0 }
    }

    pub(crate) fn dec_cons_pred_mode_child(&mut self) -> u8 {
        if self.lbac.bin(&mut self.ctx.cons_mode[0]) != 0 { ONLY_INTRA } else { ONLY_INTER }
    }

    fn dec_umve_idx(&mut self) -> i32 {
        let base = i32::from(self.lbac.bin(&mut self.ctx.umve_base_idx[0]) == 0);
        let mut step = 0;
        if self.lbac.bin(&mut self.ctx.umve_step_idx[0]) == 0 {
            step = self.lbac.truncate_unary_ep(4) as i32 + 1;
        }
        let mut dir = (self.lbac.bin(&mut self.ctx.umve_dir_idx[0]) as i32) << 1;
        dir += self.lbac.bin(&mut self.ctx.umve_dir_idx[1]) as i32;
        base * 20 + step * 4 + dir
    }

    fn split_flag_ctx(&self, w: i32, h: i32) -> usize {
        let s = self.tree_scup;
        let mut ctx = 0;
        if self.coded(s - self.i_scu) {
            ctx += usize::from((1 << (self.pos(s - self.i_scu) & 7)) < w);
        }
        if self.coded(s - 1) {
            ctx += usize::from((1 << ((self.pos(s - 1) >> 3) & 7)) < h);
        }
        ctx
    }

    fn dec_split_flag(&mut self, w: i32, h: i32) -> bool {
        let ctx = if self.slice_type == SliceType::I && w == 128 && h == 128 { 3 } else { self.split_flag_ctx(w, h) };
        self.lbac.bin(&mut self.ctx.split_flag[ctx]) != 0
    }

    /// `dec_parse_split_mode`.
    pub(crate) fn dec_split_mode(&mut self, tab: u32, w: i32, h: i32) -> u8 {
        if tab & (1 << SPLIT_QUAD) != 0 {
            if tab == 1 << SPLIT_QUAD || self.dec_split_flag(w, h) {
                return SPLIT_QUAD;
            }
            if self.slice_type == SliceType::I && w == 128 && h == 128 {
                return SPLIT_QUAD;
            }
        }
        if tab & 0x1E != 0 {
            let s = w * h;
            let sc = self.tree_scup;
            let ctx = usize::from(self.coded(sc - self.i_scu) && (1 << (self.pos(sc - self.i_scu) & 7)) < w)
                + usize::from(self.coded(sc - 1) && (1 << ((self.pos(sc - 1) >> 3) & 7)) < h);
            let no_split_ok = tab & (1 << NO_SPLIT) != 0;
            let idx = ctx
                + if s > 1024 {
                    0
                } else if s > 256 {
                    3
                } else {
                    6
                };
            if no_split_ok && self.lbac.bin(&mut self.ctx.bt_split_flag[idx]) == 0 {
                return NO_SPLIT;
            }
            let en_bt = tab & ((1 << SPLIT_BI_HOR) | (1 << SPLIT_BI_VER)) != 0;
            let en_eqt = tab & ((1 << SPLIT_EQT_HOR) | (1 << SPLIT_EQT_VER)) != 0;
            let mut typ = 0;
            if en_eqt && en_bt {
                typ = self.lbac.bin(&mut self.ctx.split_mode[ctx]);
            } else if en_eqt {
                typ = 1;
            }
            if (w == 128 && h == 64) || (w == 64 && h == 128) {
                typ = 0;
            }
            if typ == 0 {
                let dir = if (tab >> SPLIT_BI_VER) & 3 == 3 {
                    let cd = if w == 64 && h == 128 {
                        3
                    } else if w == 128 && h == 64 {
                        4
                    } else if w == h {
                        0
                    } else if w > h {
                        1
                    } else {
                        2
                    };
                    self.lbac.bin(&mut self.ctx.split_dir[cd])
                } else {
                    tab & (1 << SPLIT_BI_VER)
                };
                let dir = if w == 128 && h == 64 {
                    1
                } else if w == 64 && h == 128 {
                    0
                } else {
                    dir
                };
                if dir != 0 { SPLIT_BI_VER } else { SPLIT_BI_HOR }
            } else {
                let dir = if (tab >> SPLIT_EQT_VER) & 3 == 3 {
                    let cd = if w == h {
                        0
                    } else if w > h {
                        1
                    } else {
                        2
                    };
                    self.lbac.bin(&mut self.ctx.split_dir[cd])
                } else {
                    tab & (1 << SPLIT_EQT_VER)
                };
                if dir != 0 { SPLIT_EQT_VER } else { SPLIT_EQT_HOR }
            }
        } else {
            NO_SPLIT
        }
    }

    fn dec_affine_flag(&mut self) -> bool {
        self.log2w >= 4 && self.log2h >= 4 && self.seq.affine_enable && self.lbac.bin(&mut self.ctx.affine_flag[0]) != 0
    }

    fn dec_part_size(&mut self) -> u8 {
        if !self.seq.dt_intra_enable || self.cu_mode != MODE_INTRA {
            return SIZE_2NX2N;
        }
        let allow = blocks::dt_allow(self.cu_w, self.cu_h, self.seq.max_dt_size as i32);
        if allow == 0 {
            return SIZE_2NX2N;
        }
        let ps = &mut self.ctx.part_size;
        let l = &mut self.lbac;
        if l.bin(&mut ps[0]) != 0 {
            let dir = if allow == 3 { l.bin(&mut ps[1]) != 0 } else { allow & 1 != 0 };
            if dir {
                if l.bin(&mut ps[2]) != 0 {
                    SIZE_2NXHN
                } else if l.bin(&mut ps[3]) != 0 {
                    SIZE_2NXND
                } else {
                    SIZE_2NXNU
                }
            } else if l.bin(&mut ps[4]) != 0 {
                SIZE_HNX2N
            } else if l.bin(&mut ps[5]) != 0 {
                SIZE_NRX2N
            } else {
                SIZE_NLX2N
            }
        } else {
            SIZE_2NX2N
        }
    }

    fn dec_pred_mode(&mut self) -> u8 {
        let (lw, lh) = (self.log2w, self.log2h);
        let sc = self.cu_scup;
        let ctx = if lw > 6 || lh > 6 {
            5
        } else {
            let mut c = self.scu_flag(sc - self.i_scu, SCU_INTRA) + self.scu_flag(sc - 1, SCU_INTRA);
            if c == 0 {
                let sample = 1u32 << (lw + lh);
                c = if sample > 256 {
                    0
                } else if sample > 64 {
                    3
                } else {
                    4
                };
            }
            c as usize
        };
        if self.lbac.bin(&mut self.ctx.pred_mode[ctx]) != 0 { MODE_INTRA } else { MODE_INTER }
    }

    fn dec_abs_mvd(&mut self, comp: usize) -> u32 {
        let m = &mut self.ctx.mvd[comp];
        let l = &mut self.lbac;
        if l.bin(&mut m[0]) == 0 {
            0
        } else if l.bin(&mut m[1]) == 0 {
            1
        } else if l.bin(&mut m[2]) == 0 {
            2
        } else {
            let add_one = l.bin_ep() & 1;
            let order = l.unary_ep().min(31);
            let sym = (1u32 << order) - 1 + l.bins_ep_msb(order);
            3u32.wrapping_add(sym.wrapping_mul(2)).wrapping_add(add_one)
        }
    }

    fn dec_mvd(&mut self) -> [i32; 2] {
        let mut mvd = [0i32; 2];
        for (c, v) in mvd.iter_mut().enumerate() {
            let a = i32::from(self.dec_abs_mvd(c) as i16);
            *v = if a != 0 && self.lbac.bin_ep() != 0 { -a } else { a };
        }
        mvd
    }

    fn dec_intra_dir(&mut self, mpm: [i32; 2]) -> i32 {
        let c = &mut self.ctx.intra_dir;
        let l = &mut self.lbac;
        let ipm = if l.bin(&mut c[0]) != 0 { l.bin(&mut c[6]) as i32 - 2 } else { l.bins_msb(&mut c[1..], 5) as i32 };
        if ipm < 0 { mpm[(ipm + 2) as usize] } else { ipm + i32::from(ipm >= mpm[0]) + i32::from(ipm + 1 >= mpm[1]) }
    }

    fn dec_intra_dir_c(&mut self, ipm_l: i32) -> i32 {
        let tscpm = self.seq.tscpm_enable;
        let c = &mut self.ctx.intra_dir;
        let l = &mut self.lbac;
        if l.bin(&mut c[7]) != 0 {
            return IPD_DM_C;
        }
        let chk = is_conv(ipm_l);
        let ipm_l = if chk {
            match ipm_l {
                IPD_VER => IPD_VER_C,
                IPD_HOR => IPD_HOR_C,
                IPD_DC => IPD_DC_C,
                _ => IPD_BI_C,
            }
        } else {
            ipm_l
        };
        if tscpm && l.bin(&mut c[9]) != 0 {
            return IPD_TSCPM_C;
        }
        let mut ipm = 1 + l.truncate_unary(&mut c[8..], 1, 4) as i32;
        if chk && ipm >= ipm_l {
            ipm += 1;
        }
        ipm
    }

    fn dec_inter_dir(&mut self) -> u8 {
        let c = &mut self.ctx.inter_dir;
        let l = &mut self.lbac;
        if self.log2w + self.log2h < 6 {
            l.bin(&mut c[2]);
            if l.bin(&mut c[1]) != 0 { PRED_L1 } else { PRED_L0 }
        } else if l.bin(&mut c[0]) != 0 {
            PRED_BI
        } else if l.bin(&mut c[1]) != 0 {
            PRED_L1
        } else {
            PRED_L0
        }
    }

    fn dec_skip_idx(&mut self) -> i32 {
        let p = self.slice_type == SliceType::P;
        let max = (if p { 2 } else { 4 }) + self.seq.num_hmvp_cand;
        let idx = self.lbac.truncate_unary(&mut self.ctx.skip_idx, 11, max) as i32;
        idx + if p && idx > 0 { 2 } else { 0 }
    }

    fn mpm(&self, scup: i32) -> [i32; 2] {
        let ipm_at = |s: i32| -> i32 { if self.scu(s) & SCU_INTRA != 0 { self.map_ipm.get(self.ix(s)).copied().map_or(IPD_DC, i32::from) } else { IPD_DC } };
        let l = ipm_at(scup - 1);
        let u = ipm_at(scup - self.i_scu);
        let mut m = [l.min(u), l.max(u)];
        if m[0] == m[1] {
            m[0] = IPD_DC;
            m[1] = if m[1] == IPD_DC { IPD_BI } else { m[1] };
        }
        m
    }

    fn dec_affine_mv(&mut self, mvr_idx: i32, dir: u8) {
        let nv = usize::from(self.affine_flag) + 1;
        for lidx in 0..2usize {
            let used = if lidx == 0 { dir == PRED_L0 || dir == PRED_BI } else { dir == PRED_L1 || dir == PRED_BI };
            if !used {
                self.refi[lidx] = REFI_INVALID;
                continue;
            }
            self.refi[lidx] = self.dec_refidx(self.num_refp[lidx]);
            let mvp = self.scale_affine_mvp(lidx, mvr_idx);
            let shift = match mvr_idx {
                0 => 2,
                1 => 4,
                _ => 0,
            };
            for v in 0..nv.min(2) {
                let mvd = self.dec_mvd();
                for c in 0..2 {
                    let mv = mvp[v][c].wrapping_add(mvd[c].wrapping_shl(shift));
                    self.affine_mv[lidx][v][c] = mv.clamp(CPMV_MIN, CPMV_MAX);
                }
            }
        }
    }

    fn set_scu_ipm(&mut self, x: i32, y: i32, w: i32, h: i32, ipm: i32) {
        let (sx, sy, sw, sh) = (x >> 2, y >> 2, w >> 2, h >> 2);
        for j in 0..sh {
            for i in 0..sw {
                let k = self.ix((sy + j) * self.i_scu + sx + i);
                if let Some(s) = self.map_scu.get_mut(k) {
                    *s = SCU_CODED | SCU_INTRA;
                }
                if let Some(s) = self.map_ipm.get_mut(k) {
                    *s = ipm as i8;
                }
            }
        }
    }

    // ------------------------------------------------------------ cbf and header

    fn dec_cbf(&mut self) {
        let (lw, lh) = (self.log2w, self.log2h);
        if self.cu_mode != MODE_INTRA {
            if self.cu_mode != MODE_DIR && self.tree_status == TREE_LC {
                let ctx = usize::from(lw > 6 || lh > 6);
                if self.lbac.bin(&mut self.ctx.ctp_zero_flag[ctx]) != 0 {
                    self.tb_part = SIZE_2NX2N;
                    return;
                }
            }
            let avail = matches!((lw, lh), (3, 3) | (3, 4) | (4, 3) | (4, 4) | (4, 5) | (5, 4) | (5, 5));
            self.tb_part = if self.seq.pbt_enable && avail && self.lbac.bin(&mut self.ctx.tb_split[0]) != 0 { SIZE_NXN } else { SIZE_2NX2N };
            if self.tree_status == TREE_LC {
                self.cbfc[0] = self.lbac.bin(&mut self.ctx.cbf[1]) as u8;
                self.cbfc[1] = self.lbac.bin(&mut self.ctx.cbf[2]) as u8;
            }
            if self.cbfc == [0, 0] && self.tb_part == SIZE_2NX2N && self.tree_status == TREE_LC {
                self.cbfy[0] = 1;
            } else {
                for i in 0..blocks::PART_NUM[self.tb_part as usize] {
                    self.cbfy[i] = self.lbac.bin(&mut self.ctx.cbf[0]) as u8;
                }
            }
            if lw > 6 || lh > 6 {
                self.cbfy[0] = 0;
            }
        } else {
            if i32::from(self.ipm[0]) != IPD_IPCM {
                self.tb_part = blocks::TB_PART[self.pb_part as usize];
                for i in 0..blocks::PART_NUM[self.tb_part as usize] {
                    self.cbfy[i] = self.lbac.bin(&mut self.ctx.cbf[0]) as u8;
                }
            }
            if self.tree_status == TREE_LC && (i32::from(self.ipm[0]) != IPD_IPCM || i32::from(self.ipm_c) != IPD_DM_C) {
                self.cbfc[0] = self.lbac.bin(&mut self.ctx.cbf[1]) as u8;
                self.cbfc[1] = self.lbac.bin(&mut self.ctx.cbf[2]) as u8;
            }
        }
    }

    /// `dec_parse_cu_header_chroma`: the chroma-only CU of a separate tree.
    pub(crate) fn dec_cu_header_chroma(&mut self) {
        let sx = (self.cu_x + self.cu_w - 1) >> 2;
        let sy = (self.cu_y + self.cu_h - 1) >> 2;
        let luma_scup = sx + sy * self.i_scu;
        if self.scu(luma_scup) & SCU_INTRA != 0 {
            self.cu_mode = MODE_INTRA;
            self.ipm[0] = self.map_ipm.get(self.ix(luma_scup)).copied().unwrap_or(0);
            self.ipm_c = self.dec_intra_dir_c(i32::from(self.ipm[0])) as i8;
        } else {
            self.cu_mode = MODE_INTER;
            self.refi = self.refi_at(luma_scup);
            self.mv = self.mv_at(luma_scup);
        }
        if self.cu_mode != MODE_INTRA || i32::from(self.ipm[0]) != IPD_IPCM || i32::from(self.ipm_c) != IPD_DM_C {
            self.cbfc[0] = self.lbac.bin(&mut self.ctx.cbf[1]) as u8;
            self.cbfc[1] = self.lbac.bin(&mut self.ctx.cbf[2]) as u8;
        }
    }

    /// `dec_parse_cu_header`.
    pub(crate) fn dec_cu_header(&mut self) {
        let (lw, lh) = (self.log2w, self.log2h);
        let sc = self.cu_scup;
        self.tb_part = SIZE_2NX2N;
        self.pb_part = SIZE_2NX2N;
        if self.slice_type != SliceType::I {
            let mut skip = false;
            let mut direct = false;
            if self.cons_pred_mode != ONLY_INTRA {
                let ctx = if lw + lh < 6 { 3 } else { (self.scu_flag(sc - self.i_scu, SCU_SKIP) + self.scu_flag(sc - 1, SCU_SKIP)) as usize };
                skip = self.lbac.bin(&mut self.ctx.skip_flag[ctx]) != 0;
                if !skip {
                    let c = usize::from(lw + lh < 6 || lw > 6 || lh > 6);
                    direct = self.lbac.bin(&mut self.ctx.direct_flag[c]) != 0;
                }
            }
            if skip || direct {
                self.cu_mode = if skip { MODE_SKIP } else { MODE_DIR };
                let umve = self.seq.umve_enable && self.lbac.bin(&mut self.ctx.umve_flag) != 0;
                if umve {
                    self.affine_flag = 0;
                    let idx = self.dec_umve_idx();
                    self.derive_skip_mv_umve(idx);
                } else {
                    let aff = self.dec_affine_flag();
                    self.affine_flag = u8::from(aff);
                    if aff {
                        let idx = self.lbac.truncate_unary(&mut self.ctx.affine_mrg_idx, 4, 5) as i32;
                        self.derive_skip_mv_affine(idx);
                    } else {
                        let idx = self.dec_skip_idx();
                        self.derive_skip_mv(idx);
                    }
                }
            } else {
                if self.cons_pred_mode == NO_MODE_CONS {
                    self.cu_mode = self.dec_pred_mode();
                    if lw > 6 || lh > 6 {
                        self.cu_mode = MODE_INTER;
                    }
                } else {
                    self.cu_mode = if self.cons_pred_mode == ONLY_INTRA { MODE_INTRA } else { MODE_INTER };
                }
                self.affine_flag = 0;
                if self.cu_mode != MODE_INTRA {
                    self.dec_inter_params();
                }
            }
        } else {
            self.cu_mode = MODE_INTRA;
        }

        if self.cu_mode == MODE_INTRA {
            self.ipf_flag = false;
            self.pb_part = self.dec_part_size();
            let parts = blocks::part_info(self.pb_part, self.cu_x, self.cu_y, self.cu_w, self.cu_h);
            for (k, &(px, py, pw, ph)) in parts.iter().enumerate().take(4) {
                let scup = (py >> 2) * self.i_scu + (px >> 2);
                let mpm = self.mpm(scup);
                let mut ipm = self.dec_intra_dir(mpm);
                if self.pb_part != SIZE_2NX2N {
                    ipm = ipm.min(IPD_IPCM - 1);
                }
                self.ipm[k] = ipm as i8;
                self.set_scu_ipm(px, py, pw, ph, ipm);
            }
            self.refi = [-1, -1];
            if self.tree_status != TREE_L {
                self.ipm_c = self.dec_intra_dir_c(i32::from(self.ipm[0])) as i8;
            }
            let ipcm0 = i32::from(self.ipm[0]) == IPD_IPCM;
            let dm = i32::from(self.ipm_c) == IPD_DM_C;
            if !(self.tree_status == TREE_C && ipcm0 && dm)
                && !(self.tree_status != TREE_C && ipcm0)
                && self.seq.ipf_enable
                && self.cu_w < 128
                && self.cu_h < 128
                && self.pb_part == SIZE_2NX2N
            {
                self.ipf_flag = self.lbac.bin(&mut self.ctx.ipf_flag[0]) != 0;
            }
            self.refi = [-1, -1];
        }
        self.cbfy = [0; 4];
        self.cbfc = [0; 2];
        if self.cu_mode != MODE_SKIP {
            self.dec_cbf();
        }
    }

    fn dec_inter_params(&mut self) {
        let mut dir = PRED_L0;
        let mut hmvp_flag = false;
        let mut mvr_idx = 0i32;
        let aff = self.dec_affine_flag();
        self.affine_flag = u8::from(aff);
        if self.seq.amvr_enable {
            if self.seq.emvr_enable && !aff {
                hmvp_flag = self.lbac.bin(&mut self.ctx.mvp_from_hmvp_flag[0]) != 0;
            }
            mvr_idx = if aff {
                self.lbac.truncate_unary(&mut self.ctx.affine_mvr_idx, 2, 3) as i32
            } else {
                self.lbac.truncate_unary(&mut self.ctx.mvr_idx, 4, 5) as i32
            };
        }
        if self.slice_type == SliceType::B {
            dir = self.dec_inter_dir();
        }
        if aff {
            self.dec_affine_mv(mvr_idx, dir);
            return;
        }
        let mut smvd = false;
        let d0 = self.refp.first().map_or(0, |r| r[0].dist);
        let d1 = self.refp.first().map_or(0, |r| r[1].dist);
        if self.seq.smvd_enable && dir == PRED_BI && !hmvp_flag && d0 == -d1 {
            smvd = self.lbac.bin(&mut self.ctx.smvd_flag[0]) != 0;
        }
        let mut mvd = [0i32; 2];
        for lidx in 0..2usize {
            let used = if lidx == 0 { dir == PRED_L0 || dir == PRED_BI } else { dir == PRED_L1 || dir == PRED_BI };
            if !used {
                self.refi[lidx] = REFI_INVALID;
                continue;
            }
            self.refi[lidx] = if smvd { 0 } else { self.dec_refidx(self.num_refp[lidx]) };
            let mvp = self.derive_mvp(lidx, hmvp_flag, mvr_idx);
            if lidx == 1 && smvd {
                mvd = [mvd[0].wrapping_neg().min(32767), mvd[1].wrapping_neg().min(32767)];
            } else {
                mvd = self.dec_mvd();
            }
            for c in 0..2 {
                let v = i32::from(mvp[c]).wrapping_add(mvd[c].wrapping_shl(mvr_idx as u32));
                self.mv[lidx][c] = v.clamp(-32768, 32767) as i16;
            }
        }
    }

    // ------------------------------------------------------------ map update

    /// `dec_update_map`: stores the CU's flags, motion and size into the SCU maps.
    pub(crate) fn update_map(&mut self) {
        let (wi, hi) = (self.cu_w >> 2, self.cu_h >> 2);
        let intra = self.cu_mode == MODE_INTRA;
        let cbf = self.cbfy != [0; 4] || self.cbfc != [0; 2] || intra;
        let mut scu = SCU_CODED;
        if intra {
            scu |= SCU_INTRA;
        } else {
            scu |= SCU_INTER;
        }
        if self.cu_mode == MODE_SKIP {
            scu |= SCU_SKIP;
        }
        if cbf {
            scu |= SCU_CBF;
        }
        scu |= self.affine_flag << SCU_AFFINE_SHIFT;
        let pos = self.log2w | (self.log2h << 3) | ((self.cu_scup as u32) << 6);
        for j in 0..hi {
            for i in 0..wi {
                let k = self.ix(self.cu_scup + j * self.i_scu + i);
                if let Some(s) = self.map_scu.get_mut(k) {
                    *s = scu;
                }
                if let Some(s) = self.map_pos.get_mut(k) {
                    *s = pos;
                }
                if let Some(s) = self.cur.map_refi.get_mut(k) {
                    *s = self.refi;
                }
                if self.affine_flag == 0
                    && let Some(s) = self.cur.map_mv.get_mut(k)
                {
                    *s = self.mv;
                }
            }
        }
        if self.affine_flag != 0 {
            self.set_affine_mvf();
        }
    }

    // ------------------------------------------------------------ coefficients

    /// `dec_parse_run_length_cc`: parses and de-quantises one transform block. `comp` is 0, 1 or 2.
    pub(crate) fn dec_coefs(&mut self, log2w: u32, log2h: u32, comp: usize) -> Vec<i16> {
        let num = 1usize << (log2w + log2h);
        let mut coef = vec![0i16; num];
        let scanp = scan(log2w, log2h);
        let qp = match comp {
            0 => self.qp_y,
            1 => self.qp_u,
            _ => self.qp_v,
        }
        .clamp(0, 79) as usize;
        let log2_size = ((log2w + log2h) >> 1) as i32;
        let refix = (log2w + log2h) & 1 != 0;
        let scale = i64::from(DQ_SCALE[qp]);
        let shift = (i32::from(DQ_SHIFT[qp]) - (15 - self.seq.bit_depth as i32 - log2_size) + 1).max(0);
        let offset = if shift == 0 { 0i64 } else { 1i64 << (shift - 1) };
        let mask = (1usize << log2w) - 1;
        let wq_on = self.ph.pic_wq_enable;
        let (wq_w, idx_shift): (usize, u32) = if log2w == 2 && log2h == 2 { (4, 0) } else { (8, log2w.max(log2h).saturating_sub(3)) };
        let ch_off = usize::from(comp != 0);
        let mut prev_level = 5usize;
        let mut pos_off = 0usize;
        loop {
            let ctx_add = prev_level << 1;
            let run = {
                let m = &mut self.ctx.run[ch_off * 12 + ctx_add..];
                let mut s = self.lbac.truncate_unary(m, 2, 17);
                if s == 16 {
                    let order = self.lbac.unary_ep().min(31);
                    s = s.wrapping_add((1u32 << order) - 1).wrapping_add(self.lbac.bins_ep_msb(order));
                }
                s
            };
            let abs_level = {
                let m = &mut self.ctx.level[ch_off * 12 + ctx_add..];
                let mut s = self.lbac.truncate_unary(m, 2, 9);
                if s == 8 {
                    let order = self.lbac.unary_ep().min(31);
                    s = s.wrapping_add((1u32 << order) - 1).wrapping_add(self.lbac.bins_ep_msb(order));
                }
                s as i32 + 1
            };
            let level = if self.lbac.bin_ep() != 0 { -abs_level } else { abs_level };
            pos_off = (pos_off + run as usize).min(num);
            // uavs3d reads one entry past the scan table when a run lands on `num`: the first entry of the next
            // table, which is always 0.
            let at_pos = if pos_off == num { Some(0u16) } else { scanp.get(pos_off).copied() };
            if let Some(si) = at_pos {
                let si = si as usize;
                let (x, y) = (si & mask, si >> log2w);
                if (x | y) & 0xE0 == 0 {
                    let mut lev = if wq_on {
                        let w = if wq_w == 4 {
                            self.ph.wq_4x4.get((y >> idx_shift) * 4 + (x >> idx_shift)).copied()
                        } else {
                            self.ph.wq_8x8.get((y >> idx_shift) * 8 + (x >> idx_shift)).copied()
                        }
                        .unwrap_or(0);
                        (((((i64::from(level) * i64::from(w)) >> 2) * scale) >> 4) + offset) >> shift
                    } else {
                        (i64::from(level) * scale + offset) >> shift
                    };
                    lev = lev.clamp(-32768, 32767);
                    if refix {
                        lev = (lev * 181 + 128) >> 8;
                    }
                    if let Some(c) = coef.get_mut(si) {
                        *c = lev as i16;
                    }
                }
            }
            if pos_off + 1 >= num {
                break;
            }
            let l1 = &mut self.ctx.last1[ch_off * 6 + prev_level];
            let l2i = ch_off * 11 + (usize::BITS - 1 - (pos_off + 1).leading_zeros()) as usize;
            let l2 = &mut self.ctx.last2[l2i.min(21)];
            let last = self.lbac.bin_w(l1, l2) != 0;
            prev_level = ((abs_level - 1) as usize).min(5);
            pos_off += 1;
            if last {
                break;
            }
        }
        coef
    }

    // ------------------------------------------------------------ SAO, ALF, QP

    pub(crate) fn dec_sao_param(&mut self, lcu_idx: usize) -> [SaoParam; 3] {
        let mut out = [SaoParam::default(); 3];
        if !self.pat.sao_enable.iter().any(|&e| e) {
            return out;
        }
        let w_lcu = self.seq.w_lcu as usize;
        let cur_patch = self.map_patch.get(lcu_idx).copied().unwrap_or(0);
        let up = self.lcu_y != 0 && lcu_idx >= w_lcu && self.map_patch.get(lcu_idx - w_lcu).copied() == Some(cur_patch);
        let left = self.lcu_x != 0 && lcu_idx >= 1 && self.map_patch.get(lcu_idx - 1).copied() == Some(cur_patch);
        let mut merge = 0;
        if left || up {
            let ctx = usize::from(left) + usize::from(up);
            let mut sym = 0u32;
            if ctx == 1 {
                sym = self.lbac.bin(&mut self.ctx.sao_merge_flag[0]);
            } else if ctx == 2 {
                sym = self.lbac.bin(&mut self.ctx.sao_merge_flag[1]);
                if sym != 1 {
                    sym += self.lbac.bin(&mut self.ctx.sao_merge_flag[2]) << 1;
                }
            }
            merge = if left && !up {
                if sym != 0 { 2 } else { 0 }
            } else if up && !left {
                if sym != 0 { 1 } else { 0 }
            } else if sym != 0 {
                3 - sym
            } else {
                0
            };
        }
        if merge != 0 {
            let src = if merge == 2 { lcu_idx - 1 } else { lcu_idx - w_lcu };
            return self.sao_map.get(src).copied().unwrap_or(out);
        }
        for c in 0..3 {
            if !self.pat.sao_enable[c] {
                continue;
            }
            let mode = if self.lbac.bin(&mut self.ctx.sao_mode[0]) == 0 { 1 + u32::from(self.lbac.bin_ep() == 0) } else { 0 };
            if mode == 0 {
                continue;
            }
            let p = &mut out[c];
            p.enable = true;
            p.typ = if mode == 1 { 4 } else { 0 };
            // offsets
            p.offset[2] = 0;
            const INV: [i32; 8] = [1, 0, 2, -1, 3, 4, 5, 6];
            for i in 0..4usize {
                if p.typ == 4 {
                    let mut sym = i32::from(self.lbac.bin(&mut self.ctx.sao_offset[0]) == 0);
                    if sym != 0 {
                        sym += self.lbac.truncate_unary_ep(7) as i32;
                        if self.lbac.bin_ep() != 0 {
                            sym = -sym;
                        }
                    }
                    p.offset[i] = sym;
                } else {
                    let idx = if i >= 2 { i + 1 } else { i };
                    let maxv = match idx {
                        0 | 4 => 7,
                        _ => 1,
                    };
                    let cnt = self.lbac.truncate_unary_ep(maxv + 1) as usize;
                    let inv = INV.get(cnt).copied().unwrap_or(0);
                    p.offset[idx] = match idx {
                        0 => inv,
                        4 => -inv,
                        3 => -(cnt as i32),
                        _ => cnt as i32,
                    };
                }
            }
            if p.typ == 4 {
                let start = self.lbac.bins_ep_lsb(5) as i32;
                let order = 1 + self.lbac.truncate_unary_ep(4);
                let sym = (1i32 << order) - 2 + self.lbac.bins_ep_msb(order % 4) as i32;
                p.band_idx = [start, (start + 1) % 32, (start + 2 + sym) % 32, (start + 3 + sym) % 32];
            } else {
                p.typ = self.lbac.bins_ep_lsb(2) as i32;
            }
        }
        out
    }

    pub(crate) fn dec_lcu_delta_qp(&mut self, last: i32) -> i32 {
        let ctx = &mut self.ctx.lcu_qp_delta;
        let l = &mut self.lbac;
        let mut sym = i32::from(l.bin(&mut ctx[usize::from(last != 0)]) == 0);
        if sym != 0 {
            let mut c = 2usize;
            loop {
                let bin = l.bin(&mut ctx[c]);
                c = (c + 1).min(3);
                sym += i32::from(bin == 0);
                if bin != 0 || l.at_end() {
                    break;
                }
            }
        }
        let d = (sym + 1) >> 1;
        if sym & 1 == 0 { -d } else { d }
    }

    pub(crate) fn dec_alf_enable(&mut self) -> bool {
        self.lbac.bin(&mut self.ctx.alf_lcu_enable[0]) != 0
    }
}

#[allow(dead_code)]
fn _unused(_: Motion) {}
