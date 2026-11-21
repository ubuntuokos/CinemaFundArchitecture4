// SPDX-License-Identifier: BSD-3-Clause
// Derived from uavs3d (`source/decoder/parser.c`), Copyright (c) 2018-2022 Peking University Shenzhen Graduate
// School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation. See LICENSE.uavs3d.
//! The syntax of the coding units, the SAO parameters, the ALF flags and the delta QP.

use crate::core::{Core, SaoParam, mcu_get_logh, mcu_get_logw};
use crate::cuutil::{part_info, part_num, tb_part_of_pb, update_map_for_intra};
use crate::header::SliceType;
use crate::lbac::get_log2;
use crate::scan::scan;
use crate::types::*;

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

const MAX_NUM_MVR: u32 = 5;
const MAX_NUM_AFFINE_MVR: u32 = 3;
const IPD_CHROMA_CNT: u32 = 5;

fn affine_amvr(x: i32) -> u32 {
    match x {
        0 => 2,
        1 => 4,
        _ => 0,
    }
}

fn ipred_chk_conv(mode: u8) -> bool {
    mode == IPD_VER || mode == IPD_HOR || mode == IPD_DC || mode == IPD_BI
}

impl Core {
    fn scu_at(&self, scup: i32) -> Scu {
        self.maps.scu[self.maps.ix(scup)]
    }

    fn pos_at(&self, scup: i32) -> u32 {
        self.maps.pos[self.maps.ix(scup)]
    }

    fn decode_split_flag(&mut self, cu_width: usize, cu_height: usize) -> u32 {
        let mut ctx = 0usize;
        if self.slice_type == SliceType::I && cu_width == 128 && cu_height == 128 {
            ctx = 3;
        } else {
            let i_scu = self.seq.i_scu as i32;
            let scup = self.tree_scup;
            if self.scu_at(scup - i_scu).coded {
                ctx += usize::from((1usize << mcu_get_logw(self.pos_at(scup - i_scu))) < cu_width);
            }
            if self.scu_at(scup - 1).coded {
                ctx += usize::from((1usize << mcu_get_logh(self.pos_at(scup - 1))) < cu_height);
            }
        }
        self.lbac.bin(&mut self.ctx.split_flag[ctx])
    }

    /// `dec_parse_split_mode`
    pub(crate) fn parse_split_mode(&mut self, split_tab: u32, cu_width: usize, cu_height: usize) -> u8 {
        if split_tab & (1 << SPLIT_QUAD) != 0 {
            if split_tab == (1 << SPLIT_QUAD) || self.decode_split_flag(cu_width, cu_height) != 0 {
                return SPLIT_QUAD;
            }
            if self.slice_type == SliceType::I && cu_width == 128 && cu_height == 128 {
                return SPLIT_QUAD;
            }
        }
        if split_tab & 0x1E != 0 {
            let i_scu = self.seq.i_scu as i32;
            let scup = self.tree_scup;
            let s = cu_width * cu_height;
            let ctx = usize::from(self.scu_at(scup - i_scu).coded && (1usize << mcu_get_logw(self.pos_at(scup - i_scu))) < cu_width)
                + usize::from(self.scu_at(scup - 1).coded && (1usize << mcu_get_logh(self.pos_at(scup - 1))) < cu_height);
            let sz = if s > 1024 {
                0
            } else if s > 256 {
                3
            } else {
                6
            };
            if split_tab & (1 << SPLIT_NONE) != 0 && self.lbac.bin(&mut self.ctx.bt_split_flag[ctx + sz]) == 0 {
                return SPLIT_NONE;
            }
            let enable_bt = split_tab & ((1 << SPLIT_BI_HOR) | (1 << SPLIT_BI_VER)) != 0;
            let enable_eqt = split_tab & ((1 << SPLIT_EQT_HOR) | (1 << SPLIT_EQT_VER)) != 0;
            let mut split_typ = 0;
            if enable_eqt && enable_bt {
                split_typ = self.lbac.bin(&mut self.ctx.split_mode[ctx]);
            } else if enable_eqt {
                split_typ = 1;
            }
            if (cu_width == 128 && cu_height == 64) || (cu_width == 64 && cu_height == 128) {
                split_typ = 0;
            }
            if split_typ == 0 {
                let mut split_dir;
                if (split_tab >> SPLIT_BI_VER) & 3 == 3 {
                    let ctx_dir = if cu_width == 64 && cu_height == 128 {
                        3
                    } else if cu_width == 128 && cu_height == 64 {
                        4
                    } else if cu_width == cu_height {
                        0
                    } else if cu_width > cu_height {
                        1
                    } else {
                        2
                    };
                    split_dir = self.lbac.bin(&mut self.ctx.split_dir[ctx_dir]);
                } else {
                    split_dir = split_tab & (1 << SPLIT_BI_VER);
                }
                if cu_width == 128 && cu_height == 64 {
                    split_dir = 1;
                } else if cu_width == 64 && cu_height == 128 {
                    split_dir = 0;
                }
                return if split_dir != 0 { SPLIT_BI_VER } else { SPLIT_BI_HOR };
            }
            let split_dir = if (split_tab >> SPLIT_EQT_VER) & 3 == 3 {
                let ctx_dir = if cu_width == cu_height {
                    0
                } else if cu_width > cu_height {
                    1
                } else {
                    2
                };
                self.lbac.bin(&mut self.ctx.split_dir[ctx_dir])
            } else {
                split_tab & (1 << SPLIT_EQT_VER)
            };
            return if split_dir != 0 { SPLIT_EQT_VER } else { SPLIT_EQT_HOR };
        }
        SPLIT_NONE
    }

    pub(crate) fn parse_cons_pred_mode_child(&mut self) -> u8 {
        if self.lbac.bin(&mut self.ctx.cons_mode[0]) != 0 { ONLY_INTRA } else { ONLY_INTER }
    }

    fn decode_refidx(&mut self, num_refp: usize) -> i8 {
        if num_refp > 1 { self.lbac.truncate_unary(&mut self.ctx.refi, 3, num_refp as u32) as i8 } else { 0 }
    }

    fn decode_skip_flag(&mut self, scup: i32) -> bool {
        let i_scu = self.seq.i_scu as i32;
        let ctx = if self.cu_log2w + self.cu_log2h < 6 { 3 } else { usize::from(self.scu_at(scup - i_scu).skip) + usize::from(self.scu_at(scup - 1).skip) };
        self.lbac.bin(&mut self.ctx.skip_flag[ctx]) != 0
    }

    fn decode_umve_idx(&mut self) -> i32 {
        let base_idx = i32::from(self.lbac.bin(&mut self.ctx.umve_base_idx[0]) == 0);
        let mut step_idx = 0;
        if self.lbac.bin(&mut self.ctx.umve_step_idx[0]) == 0 {
            step_idx = self.lbac.truncate_unary_ep(UMVE_REFINE_STEP - 1) as i32 + 1;
        }
        let mut dir_idx = (self.lbac.bin(&mut self.ctx.umve_dir_idx[0]) << 1) as i32;
        dir_idx += self.lbac.bin(&mut self.ctx.umve_dir_idx[1]) as i32;
        base_idx * UMVE_MAX_REFINE_NUM + step_idx * 4 + dir_idx
    }

    fn decode_affine_flag(&mut self) -> u8 {
        let ok = self.cu_log2w >= AFF_BITSIZE && self.cu_log2h >= AFF_BITSIZE && self.seq.affine_enable_flag;
        u8::from(ok && self.lbac.bin(&mut self.ctx.affine_flag[0]) != 0)
    }

    fn decode_part_size(&mut self) -> u8 {
        if !self.seq.dt_intra_enable_flag || self.cu_mode != MODE_INTRA {
            return SIZE_2NX2N;
        }
        let allow = crate::cuutil::dt_allow(self.cu_width, self.cu_height, self.seq.max_dt_size as usize);
        if allow == 0 {
            return SIZE_2NX2N;
        }
        let mut part = SIZE_2NX2N;
        if self.lbac.bin(&mut self.ctx.part_size[0]) != 0 {
            let dir = if allow == 3 { self.lbac.bin(&mut self.ctx.part_size[1]) } else { allow & 1 };
            if dir != 0 {
                if self.lbac.bin(&mut self.ctx.part_size[2]) != 0 {
                    part = SIZE_2NXHN;
                } else {
                    part = if self.lbac.bin(&mut self.ctx.part_size[3]) != 0 { SIZE_2NXND } else { SIZE_2NXNU };
                }
            } else if self.lbac.bin(&mut self.ctx.part_size[4]) != 0 {
                part = SIZE_HNX2N;
            } else {
                part = if self.lbac.bin(&mut self.ctx.part_size[5]) != 0 { SIZE_NRX2N } else { SIZE_NLX2N };
            }
        }
        part
    }

    fn decode_pred_mode(&mut self, scup: i32) -> u8 {
        let i_scu = self.seq.i_scu as i32;
        let ctx = if self.cu_log2w > 6 || self.cu_log2h > 6 {
            5
        } else {
            let mut c = usize::from(self.scu_at(scup - i_scu).intra) + usize::from(self.scu_at(scup - 1).intra);
            if c == 0 {
                let sample = 1usize << (self.cu_log2w + self.cu_log2h);
                c = if sample > 256 {
                    0
                } else if sample > 64 {
                    3
                } else {
                    4
                };
            }
            c
        };
        if self.lbac.bin(&mut self.ctx.pred_mode[ctx]) != 0 { MODE_INTRA } else { MODE_INTER }
    }

    fn decode_mvr_idx(&mut self, affine: bool) -> i32 {
        if affine {
            self.lbac.truncate_unary(&mut self.ctx.affine_mvr_idx, MAX_NUM_AFFINE_MVR - 1, MAX_NUM_AFFINE_MVR) as i32
        } else {
            self.lbac.truncate_unary(&mut self.ctx.mvr_idx, MAX_NUM_MVR - 1, MAX_NUM_MVR) as i32
        }
    }

    fn parse_abs_mvd(&mut self, dim: usize) -> u32 {
        if self.lbac.bin(&mut self.ctx.mvd[dim][0]) == 0 {
            0
        } else if self.lbac.bin(&mut self.ctx.mvd[dim][1]) == 0 {
            1
        } else if self.lbac.bin(&mut self.ctx.mvd[dim][2]) == 0 {
            2
        } else {
            let add_one = self.lbac.bin_ep() & 1;
            let golomb_order = self.lbac.unary_ep();
            let sym = (1u32 << golomb_order) - 1 + self.lbac.bins_ep_msb(golomb_order);
            3 + sym * 2 + add_one
        }
    }

    fn decode_mvd(&mut self) -> [i32; 2] {
        let mut mvd = [0i32; 2];
        for (dim, m) in mvd.iter_mut().enumerate() {
            // the C code stores the magnitude in an s16
            *m = i32::from(self.parse_abs_mvd(dim) as i16);
            if *m != 0 && self.lbac.bin_ep() != 0 {
                *m = i32::from((-*m) as i16);
            }
        }
        mvd
    }

    fn decode_intra_dir(&mut self, mpm: [u8; 2]) -> i32 {
        let mut ipm = if self.lbac.bin(&mut self.ctx.intra_dir[0]) != 0 {
            self.lbac.bin(&mut self.ctx.intra_dir[6]) as i32 - 2
        } else {
            self.lbac.bins_msb(&mut self.ctx.intra_dir[1..], 5) as i32
        };
        ipm = if ipm < 0 { i32::from(mpm[(ipm + 2) as usize]) } else { ipm + i32::from(ipm >= i32::from(mpm[0])) + i32::from(ipm + 1 >= i32::from(mpm[1])) };
        ipm
    }

    fn decode_intra_dir_c(&mut self, ipm_l: u8, tscpm_enable: bool) -> u8 {
        let mut ipm = 0u32;
        if self.lbac.bin(&mut self.ctx.intra_dir[7]) == 0 {
            let mut ipm_l = ipm_l;
            let chk_bypass = ipred_chk_conv(ipm_l);
            if chk_bypass {
                ipm_l = if ipm_l == IPD_VER {
                    IPD_VER_C
                } else if ipm_l == IPD_HOR {
                    IPD_HOR_C
                } else if ipm_l == IPD_DC {
                    IPD_DC_C
                } else {
                    IPD_BI_C
                };
            }
            if tscpm_enable && self.lbac.bin(&mut self.ctx.intra_dir[9]) != 0 {
                return IPD_TSCPM_C;
            }
            ipm = 1 + self.lbac.truncate_unary(&mut self.ctx.intra_dir[8..], 1, IPD_CHROMA_CNT - 1);
            if chk_bypass && ipm >= u32::from(ipm_l) {
                ipm += 1;
            }
        }
        ipm as u8
    }

    fn decode_inter_dir(&mut self) -> u8 {
        if self.cu_log2w + self.cu_log2h < 6 {
            self.lbac.bin(&mut self.ctx.inter_dir[2]);
            if self.lbac.bin(&mut self.ctx.inter_dir[1]) != 0 { PRED_L1 } else { PRED_L0 }
        } else if self.lbac.bin(&mut self.ctx.inter_dir[0]) != 0 {
            PRED_BI
        } else if self.lbac.bin(&mut self.ctx.inter_dir[1]) != 0 {
            PRED_L1
        } else {
            PRED_L0
        }
    }

    fn decode_skip_idx(&mut self) -> usize {
        let p = self.slice_type == SliceType::P;
        let max_skip_num = (if p { 2 } else { TRADITIONAL_SKIP_NUM as u32 }) + self.seq.num_of_hmvp_cand;
        let skip_idx = self.lbac.truncate_unary(&mut self.ctx.skip_idx, (MAX_SKIP_NUM - 1) as u32, max_skip_num) as usize;
        skip_idx + (usize::from(p && skip_idx > 0) << 1)
    }

    fn decode_direct_flag(&mut self) -> bool {
        let ctx = usize::from((self.cu_log2w + self.cu_log2h < 6) || self.cu_log2w > 6 || self.cu_log2h > 6);
        self.lbac.bin(&mut self.ctx.direct_flag[ctx]) != 0
    }

    fn parse_affine_mv(&mut self, mvr_idx: i32, inter_dir: u8) {
        let vertex_num = usize::from(self.affine_flag) + 1;
        for lidx in 0..2usize {
            let used = if lidx == 0 { inter_dir == PRED_L0 || inter_dir == PRED_BI } else { inter_dir == PRED_L1 || inter_dir == PRED_BI };
            if used {
                self.refi[lidx] = self.decode_refidx(self.num_refp[lidx]);
                let mvp = self.scale_affine_mvp(lidx, mvr_idx);
                for vertex in 0..vertex_num {
                    let mvd = self.decode_mvd();
                    let sh = affine_amvr(mvr_idx);
                    let mv_x = mvp[vertex][0] + (mvd[0] << sh);
                    let mv_y = mvp[vertex][1] + (mvd[1] << sh);
                    self.affine_mv[lidx][vertex][0] = clip3(CPMV_MIN, CPMV_MAX, mv_x);
                    self.affine_mv[lidx][vertex][1] = clip3(CPMV_MIN, CPMV_MAX, mv_y);
                }
            } else {
                self.refi[lidx] = REFI_INVALID;
            }
        }
    }

    fn get_mpm(&self, scup: i32) -> [u8; 2] {
        let i_scu = self.seq.i_scu as i32;
        let m = &self.maps;
        let ipm_l = if m.scu[m.ix(scup - 1)].intra { m.ipm[m.ix(scup - 1)] as u8 } else { IPD_DC };
        let ipm_u = if m.scu[m.ix(scup - i_scu)].intra { m.ipm[m.ix(scup - i_scu)] as u8 } else { IPD_DC };
        let mut mpm = [ipm_l.min(ipm_u), ipm_l.max(ipm_u)];
        if mpm[0] == mpm[1] {
            mpm[0] = IPD_DC;
            mpm[1] = if mpm[1] == IPD_DC { IPD_BI } else { mpm[1] };
        }
        mpm
    }

    /// `dec_parse_cu_header_chroma`
    pub(crate) fn parse_cu_header_chroma(&mut self) {
        let scu_x = (self.cu_pix_x + self.cu_width - 1) >> 2;
        let scu_y = (self.cu_pix_y + self.cu_height - 1) >> 2;
        let luma_scup = (scu_x + scu_y * self.seq.i_scu) as i32;
        let ix = self.maps.ix(luma_scup);
        if self.maps.scu[ix].intra {
            self.cu_mode = MODE_INTRA;
            self.ipm[0] = self.maps.ipm[ix];
            let tscpm = self.seq.tscpm_enable_flag;
            self.ipm_c = self.decode_intra_dir_c(self.ipm[0] as u8, tscpm);
        } else {
            self.cu_mode = MODE_INTER;
            for lidx in 0..2 {
                self.refi[lidx] = self.maps.refi[ix][lidx];
                self.mv[lidx] = self.maps.mv[ix][lidx];
            }
        }
        if self.cu_mode != MODE_INTRA || self.ipm[0] as u8 != IPD_IPCM || self.ipm_c != IPD_DM_C {
            self.cbfc[0] = self.lbac.bin(&mut self.ctx.cbf[1]) as u8;
            self.cbfc[1] = self.lbac.bin(&mut self.ctx.cbf[2]) as u8;
        }
    }

    /// `dec_parse_cu_header`
    pub(crate) fn parse_cu_header(&mut self) {
        let scup = self.cu_scup;
        self.tb_part = SIZE_2NX2N;
        self.pb_part = SIZE_2NX2N;

        if self.slice_type != SliceType::I {
            let mut skip_flag = false;
            let mut direct_flag = false;
            if self.cons_pred_mode != ONLY_INTRA {
                skip_flag = self.decode_skip_flag(scup);
                if !skip_flag {
                    direct_flag = self.decode_direct_flag();
                }
            }
            if skip_flag || direct_flag {
                self.cu_mode = if skip_flag { MODE_SKIP } else { MODE_DIR };
                let umve_flag = self.seq.umve_enable_flag && self.lbac.bin(&mut self.ctx.umve_flag[0]) != 0;
                if umve_flag {
                    self.affine_flag = 0;
                    let idx = self.decode_umve_idx();
                    self.derive_skip_mv_umve(idx);
                } else {
                    self.affine_flag = self.decode_affine_flag();
                    if self.affine_flag != 0 {
                        let idx = self.lbac.truncate_unary(&mut self.ctx.affine_mrg_idx, (AFF_MAX_NUM_MRG - 1) as u32, AFF_MAX_NUM_MRG as u32);
                        self.derive_skip_mv_affine(idx as i32);
                    } else {
                        let idx = self.decode_skip_idx();
                        self.derive_skip_mv(idx);
                    }
                }
            } else {
                if self.cons_pred_mode == NO_MODE_CONS {
                    self.cu_mode = self.decode_pred_mode(scup);
                    if self.cu_log2w > 6 || self.cu_log2h > 6 {
                        self.cu_mode = MODE_INTER;
                    }
                } else {
                    self.cu_mode = if self.cons_pred_mode == ONLY_INTRA { MODE_INTRA } else { MODE_INTER };
                }
                self.affine_flag = 0;
                if self.cu_mode != MODE_INTRA {
                    let mut inter_dir = PRED_L0;
                    let mut mvp_from_hmvp = false;
                    let mut mvr_idx = 0;
                    self.affine_flag = self.decode_affine_flag();
                    if self.seq.amvr_enable_flag {
                        if self.seq.emvr_enable_flag && self.affine_flag == 0 {
                            mvp_from_hmvp = self.lbac.bin(&mut self.ctx.mvp_from_hmvp_flag[0]) != 0;
                        }
                        mvr_idx = self.decode_mvr_idx(self.affine_flag != 0);
                    }
                    if self.slice_type == SliceType::B {
                        inter_dir = self.decode_inter_dir();
                    }
                    if self.affine_flag != 0 {
                        self.parse_affine_mv(mvr_idx, inter_dir);
                    } else {
                        let mut smvd_flag = false;
                        let mut mvd = [0i32; 2];
                        let same_dist = self.refp.first().is_some_and(|r| match (&r[0], &r[1]) {
                            (Some(a), Some(b)) => a.dist == -b.dist,
                            _ => false,
                        });
                        if self.seq.smvd_enable_flag && inter_dir == PRED_BI && !mvp_from_hmvp && same_dist {
                            smvd_flag = self.lbac.bin(&mut self.ctx.smvd_flag[0]) != 0;
                        }
                        for lidx in 0..2usize {
                            let used = if lidx == 0 { inter_dir == PRED_L0 || inter_dir == PRED_BI } else { inter_dir == PRED_L1 || inter_dir == PRED_BI };
                            if !used {
                                self.refi[lidx] = REFI_INVALID;
                                continue;
                            }
                            self.refi[lidx] = if smvd_flag { 0 } else { self.decode_refidx(self.num_refp[lidx]) };
                            let mvp = self.derive_mvp(lidx, mvp_from_hmvp, mvr_idx);
                            if lidx == 1 && smvd_flag {
                                mvd[0] = 32767.min(-mvd[0]);
                                mvd[1] = 32767.min(-mvd[1]);
                            } else {
                                mvd = self.decode_mvd();
                            }
                            let mv_x = i32::from(mvp[0]) + (mvd[0] << mvr_idx);
                            let mv_y = i32::from(mvp[1]) + (mvd[1] << mvr_idx);
                            self.mv[lidx] = [clip3(-32768, 32767, mv_x) as i16, clip3(-32768, 32767, mv_y) as i16];
                        }
                    }
                }
            }
        } else {
            self.cu_mode = MODE_INTRA;
        }

        if self.cu_mode == MODE_INTRA {
            self.ipf_flag = false;
            self.pb_part = self.decode_part_size();
            let parts = part_info(self.cu_pix_x, self.cu_pix_y, self.cu_width, self.cu_height, self.pb_part);
            let i_scu = self.seq.i_scu;
            for (part_idx, &(pb_x, pb_y, pb_w, pb_h)) in parts.iter().enumerate() {
                let scup = ((pb_y >> 2) * i_scu + (pb_x >> 2)) as i32;
                let mpm = self.get_mpm(scup);
                let mut ipm = self.decode_intra_dir(mpm);
                if self.pb_part != SIZE_2NX2N {
                    ipm = ipm.min(i32::from(IPD_IPCM) - 1);
                }
                self.ipm[part_idx] = ipm as i8;
                self.refi = [-1; 2];
                update_map_for_intra(&mut self.maps, pb_x, pb_y, pb_w, pb_h, ipm as i8);
            }
            if self.tree_status != TREE_L {
                let tscpm = self.seq.tscpm_enable_flag;
                self.ipm_c = self.decode_intra_dir_c(self.ipm[0] as u8, tscpm);
            }
            let ipcm = self.ipm[0] as u8 == IPD_IPCM;
            if !(self.tree_status == TREE_C && ipcm && self.ipm_c == IPD_DM_C)
                && !(self.tree_status != TREE_C && ipcm)
                && self.seq.ipf_enable_flag
                && self.cu_width < MAX_CU_SIZE
                && self.cu_height < MAX_CU_SIZE
                && self.pb_part == SIZE_2NX2N
            {
                self.ipf_flag = self.lbac.bin(&mut self.ctx.ipf_flag[0]) != 0;
            }
            self.refi = [-1; 2];
        }

        self.cbfy = [0; 4];
        self.cbfc = [0; 2];
        if self.cu_mode != MODE_SKIP {
            self.parse_cbf();
        }
    }

    fn parse_cbf(&mut self) {
        let (cu_log2w, cu_log2h) = (self.cu_log2w, self.cu_log2h);
        let tree_status = self.tree_status;
        if self.cu_mode != MODE_INTRA {
            if self.cu_mode != MODE_DIR && tree_status == TREE_LC {
                let c = usize::from(cu_log2w > 6 || cu_log2h > 6);
                if self.lbac.bin(&mut self.ctx.ctp_zero_flag[c]) != 0 {
                    self.tb_part = SIZE_2NX2N;
                    return;
                }
            }
            let tb_avail = matches!((cu_log2w, cu_log2h), (3, 3) | (3, 4) | (4, 3) | (4, 4) | (4, 5) | (5, 4) | (5, 5));
            if self.seq.position_based_transform_enable_flag && tb_avail {
                self.tb_part = if self.lbac.bin(&mut self.ctx.tb_split[0]) != 0 { SIZE_NXN } else { SIZE_2NX2N };
            } else {
                self.tb_part = SIZE_2NX2N;
            }
            if tree_status == TREE_LC {
                self.cbfc[0] = self.lbac.bin(&mut self.ctx.cbf[1]) as u8;
                self.cbfc[1] = self.lbac.bin(&mut self.ctx.cbf[2]) as u8;
            }
            if self.cbfc == [0, 0] && self.tb_part == SIZE_2NX2N && tree_status == TREE_LC {
                self.cbfy[0] = 1;
            } else {
                for i in 0..part_num(self.tb_part) {
                    self.cbfy[i] = self.lbac.bin(&mut self.ctx.cbf[0]) as u8;
                }
            }
            if cu_log2w > 6 || cu_log2h > 6 {
                self.cbfy[0] = 0;
            }
        } else {
            if self.ipm[0] as u8 != IPD_IPCM {
                self.tb_part = tb_part_of_pb(self.pb_part);
                for i in 0..part_num(self.tb_part) {
                    self.cbfy[i] = self.lbac.bin(&mut self.ctx.cbf[0]) as u8;
                }
            }
            if tree_status == TREE_LC && (self.ipm[0] as u8 != IPD_IPCM || self.ipm_c != IPD_DM_C) {
                self.cbfc[0] = self.lbac.bin(&mut self.ctx.cbf[1]) as u8;
                self.cbfc[1] = self.lbac.bin(&mut self.ctx.cbf[2]) as u8;
            }
        }
    }

    fn parse_run(&mut self, off: usize) -> u32 {
        let mut sym = self.lbac.truncate_unary(&mut self.ctx.run[off..], 2, 17);
        if sym == 16 {
            let g = self.lbac.unary_ep();
            sym += (1 << g) - 1 + self.lbac.bins_ep_msb(g);
        }
        sym
    }

    fn parse_level(&mut self, off: usize) -> u32 {
        let mut sym = self.lbac.truncate_unary(&mut self.ctx.level[off..], 2, 9);
        if sym == 8 {
            let g = self.lbac.unary_ep();
            sym += (1 << g) - 1 + self.lbac.bins_ep_msb(g);
        }
        sym
    }

    /// `dec_parse_run_length_cc`: parses and dequantises one transform block into `coef`
    /// (`1 << (log2_w + log2_h)` entries). `ch_type` is 0 (Y), 1 (U) or 2 (V).
    pub(crate) fn parse_run_length_cc(&mut self, coef: &mut [i16], log2_w: u32, log2_h: u32, ch_type: usize) {
        let scanp = scan(log2_w, log2_h);
        let num_coeff = 1usize << (log2_w + log2_h);
        let mut scan_pos_offset = 0usize;
        let mut prev_level = 5usize;
        let chroma = ch_type != 0;
        let off_last1 = if chroma { 6 } else { 0 };
        let off_last2 = if chroma { 12 } else { 0 };
        let off_rl = if chroma { 12 } else { 0 };

        let qp = match ch_type {
            0 => self.lcu_qp_y,
            1 => self.lcu_qp_u,
            _ => self.lcu_qp_v,
        } as usize;
        let log2_size = ((log2_w + log2_h) >> 1) as i32;
        let refix = (log2_w + log2_h) & 1 != 0;
        let scale = i32::from(DQ_SCALE[qp]);
        let shift = i32::from(DQ_SHIFT[qp]) - (15 - self.seq.bit_depth_internal as i32 - log2_size) + 1;
        let offset = if shift == 0 { 0 } else { 1i32 << (shift - 1) };
        let mask = (1usize << log2_w) - 1;
        let weight_quant = self.pichdr.pic_wq_enable;
        let (wq_idx_shift, wq_width) = if log2_w == 2 && log2_h == 2 { (0, 4) } else { (log2_w.max(log2_h) - 3, 8) };
        let small = log2_w == 2 && log2_h == 2;

        coef[..num_coeff].fill(0);
        loop {
            let ctx_add = prev_level << 1;
            let run = self.parse_run(off_rl + ctx_add) as usize;
            let abs_level = self.parse_level(off_rl + ctx_add) as i32 + 1;
            let level: i32 = if self.lbac.bin_ep() != 0 { -abs_level } else { abs_level };
            let level = i32::from(level as i16);

            scan_pos_offset = (scan_pos_offset + run).min(num_coeff);
            let scan_idx = scanp.get(scan_pos_offset).copied().unwrap_or(0) as usize;
            let x = scan_idx & mask;
            let y = scan_idx >> log2_w;

            if (x | y) & 0xE0 == 0 {
                let mut lev = if weight_quant {
                    let w = if small {
                        i64::from(self.pichdr.wq_4x4_matrix[(y >> wq_idx_shift) * wq_width + (x >> wq_idx_shift)])
                    } else {
                        i64::from(self.pichdr.wq_8x8_matrix[(y >> wq_idx_shift) * wq_width + (x >> wq_idx_shift)])
                    };
                    let v = ((((i64::from(level) * w) >> 2) * i64::from(scale)) >> 4) + i64::from(offset);
                    (v >> shift) as i32
                } else {
                    (level.wrapping_mul(scale).wrapping_add(offset)) >> shift
                };
                lev = clip3(-32768, 32767, lev);
                if refix {
                    lev = (lev * 181 + 128) >> 8;
                }
                coef[scan_idx] = lev as i16;
            }

            if scan_pos_offset >= num_coeff - 1 {
                break;
            }
            let l2 = get_log2(scan_pos_offset as i32 + 1) as usize;
            let last_flag = self.lbac.bin_w(&mut self.ctx.last1[off_last1 + prev_level], &mut self.ctx.last2[(off_last2 + l2).min(21)]);
            prev_level = ((abs_level - 1) as usize).min(5);
            scan_pos_offset += 1;
            if last_flag != 0 {
                break;
            }
        }
    }

    // ---- SAO, ALF flags, delta QP ----

    fn parse_sao_mergeflag(&mut self, left_avail: bool, up_avail: bool) -> u32 {
        let act_ctx = u32::from(left_avail) + u32::from(up_avail);
        let mut sym = 0;
        if act_ctx == 1 {
            sym = self.lbac.bin(&mut self.ctx.sao_merge_flag[0]);
        } else if act_ctx == 2 {
            sym = self.lbac.bin(&mut self.ctx.sao_merge_flag[1]);
            if sym != 1 {
                sym += self.lbac.bin(&mut self.ctx.sao_merge_flag[2]) << 1;
            }
        }
        if left_avail && !up_avail {
            return if sym != 0 { 2 } else { 0 };
        }
        if up_avail && !left_avail {
            return if sym != 0 { 1 } else { 0 };
        }
        if sym != 0 { 3 - sym } else { 0 }
    }

    fn parse_sao_offset(&mut self, typ: u32, offset: &mut [i32; 5], _band: &mut [i32; 4]) {
        const THRESH: [i32; 6] = [7, 1, 0, 1, 7, 7];
        const EO_INV: [i32; 8] = [1, 0, 2, -1, 3, 4, 5, 6];
        offset[2] = 0;
        for i in 0..4usize {
            if typ == 4 {
                let mut sym = i32::from(self.lbac.bin(&mut self.ctx.sao_offset[0]) == 0);
                if sym != 0 {
                    sym += self.lbac.truncate_unary_ep(THRESH[5] as u32) as i32;
                    if self.lbac.bin_ep() != 0 {
                        sym = -sym;
                    }
                }
                offset[i] = sym;
            } else {
                let idx = if i >= 2 { i + 1 } else { i };
                let cnt = self.lbac.truncate_unary_ep(THRESH[idx] as u32 + 1) as i32;
                let sym = match idx {
                    0 => EO_INV[cnt as usize & 7],
                    4 => -EO_INV[cnt as usize & 7],
                    3 => -cnt,
                    _ => cnt,
                };
                offset[idx] = sym;
            }
        }
    }

    /// `dec_parse_sao_param`: fills `sao_param_map[lcu_idx]`.
    pub(crate) fn parse_sao_param(&mut self, lcu_idx: usize) {
        let sh = self.pathdr;
        if !sh.slice_sao_enable.iter().any(|&e| e) {
            for c in 0..3 {
                self.sao_param_map[lcu_idx][c].enable = false;
            }
            return;
        }
        let w = self.seq.pic_width_in_lcu;
        let cur_patch = self.maps.patch[lcu_idx];
        let up = self.lcu_y != 0 && cur_patch == self.maps.patch[lcu_idx - w];
        let left = self.lcu_x != 0 && cur_patch == self.maps.patch[lcu_idx - 1];
        let mut mergemode = 0;
        if left || up {
            mergemode = self.parse_sao_mergeflag(left, up);
        }
        if mergemode != 0 {
            self.sao_param_map[lcu_idx] = if mergemode == 2 { self.sao_param_map[lcu_idx - 1] } else { self.sao_param_map[lcu_idx - w] };
            return;
        }
        let mut cur = self.sao_param_map[lcu_idx];
        for c in 0..3 {
            let p: &mut SaoParam = &mut cur[c];
            if !sh.slice_sao_enable[c] {
                p.enable = false;
                continue;
            }
            let mode = if self.lbac.bin(&mut self.ctx.sao_mode[0]) == 0 { 1 + u32::from(self.lbac.bin_ep() == 0) } else { 0 };
            match mode {
                0 => p.enable = false,
                1 => {
                    p.enable = true;
                    p.typ = 4;
                }
                _ => {
                    p.enable = true;
                    p.typ = 0;
                }
            }
            if p.enable {
                let (typ, mut off, mut band) = (p.typ, p.offset, p.band_idx);
                self.parse_sao_offset(typ, &mut off, &mut band);
                p.offset = off;
                if typ == 4 {
                    let start = self.lbac.bins_ep_lsb(5) as i32;
                    let golomb_order = 1 + self.lbac.truncate_unary_ep(4);
                    let act_sym = (1i32 << golomb_order) - 2 + self.lbac.bins_ep_msb(golomb_order % 4) as i32;
                    band = [start, (start + 1) % 32, (start + 2 + act_sym) % 32, (start + 2 + act_sym + 1) % 32];
                    p.band_idx = band;
                } else {
                    p.typ = self.lbac.bins_ep_lsb(2);
                }
            }
        }
        self.sao_param_map[lcu_idx] = cur;
    }

    pub(crate) fn parse_alf_enable(&mut self) -> bool {
        self.lbac.bin(&mut self.ctx.alf_lcu_enable[0]) != 0
    }

    pub(crate) fn parse_lcu_delta_qp(&mut self, last_dqp: i32) -> i32 {
        let mut act_ctx = usize::from(last_dqp != 0);
        let mut act_sym = i32::from(self.lbac.bin(&mut self.ctx.lcu_qp_delta[act_ctx]) == 0);
        if act_sym != 0 {
            act_ctx = 2;
            loop {
                let bin = self.lbac.bin(&mut self.ctx.lcu_qp_delta[act_ctx]);
                act_ctx = 3.min(act_ctx + 1);
                act_sym += i32::from(bin == 0);
                if !(bin == 0 && self.lbac.cur < self.lbac.end) {
                    break;
                }
            }
        }
        let mut dquant = (act_sym + 1) >> 1;
        if act_sym & 1 == 0 {
            dquant = -dquant;
        }
        dquant
    }
}
