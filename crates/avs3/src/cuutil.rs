// SPDX-License-Identifier: BSD-3-Clause
// Derived from uavs3d (`source/decoder/dec_util.c`), Copyright (c) 2018-2022 Peking University Shenzhen Graduate
// School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation. See LICENSE.uavs3d.
//! Block structure helpers: split geometry, partitions and the 4x4 block maps.

use crate::core::{Core, Maps, mcu_set_logh, mcu_set_logw, mcu_set_scup};
use crate::header::{Sequence, SliceType};
use crate::types::*;

pub(crate) struct SplitInfo {
    pub count: usize,
    pub log_w: [u32; 4],
    pub log_h: [u32; 4],
    pub x: [usize; 4],
    pub y: [usize; 4],
    pub w: [usize; 4],
    pub h: [usize; 4],
}

fn part_size(mode: u8, part: usize, len: usize) -> usize {
    match mode {
        SPLIT_QUAD | SPLIT_BI_HOR | SPLIT_BI_VER => len >> 1,
        SPLIT_EQT_HOR | SPLIT_EQT_VER => {
            if part == 1 || part == 2 {
                len >> 1
            } else {
                len >> 2
            }
        }
        _ => len,
    }
}

fn part_size_idx(mode: u8, part: usize, idx: u32) -> u32 {
    match mode {
        SPLIT_QUAD | SPLIT_BI_HOR | SPLIT_BI_VER => idx - 1,
        SPLIT_EQT_HOR | SPLIT_EQT_VER => {
            if part == 1 || part == 2 {
                idx - 1
            } else {
                idx - 2
            }
        }
        _ => idx,
    }
}

/// `dec_get_split_struct`
pub(crate) fn split_struct(mode: u8, x0: usize, y0: usize, cw: usize, ch: usize) -> SplitInfo {
    const COUNT: [usize; 6] = [0, 2, 2, 4, 4, 4];
    let lw = log2_u(cw);
    let lh = log2_u(ch);
    let mut s = SplitInfo { count: COUNT[mode as usize], log_w: [0; 4], log_h: [0; 4], x: [0; 4], y: [0; 4], w: [0; 4], h: [0; 4] };
    s.x[0] = x0;
    s.y[0] = y0;
    match mode {
        SPLIT_NONE => {
            s.w[0] = cw;
            s.h[0] = ch;
            s.log_w[0] = lw;
            s.log_h[0] = lh;
        }
        SPLIT_QUAD => {
            for i in 0..4 {
                s.w[i] = cw >> 1;
                s.h[i] = ch >> 1;
                s.log_w[i] = lw - 1;
                s.log_h[i] = lh - 1;
            }
            s.x[1] = x0 + s.w[0];
            s.y[1] = y0;
            s.x[2] = x0;
            s.y[2] = y0 + s.h[0];
            s.x[3] = s.x[1];
            s.y[3] = s.y[2];
        }
        _ => {
            let vertical = mode == SPLIT_BI_VER || mode == SPLIT_EQT_VER;
            if vertical {
                for i in 0..s.count {
                    s.w[i] = part_size(mode, i, cw);
                    s.log_w[i] = part_size_idx(mode, i, lw);
                    if mode == SPLIT_EQT_VER {
                        if i == 0 || i == 3 {
                            s.h[i] = ch;
                            s.log_h[i] = lh;
                        } else {
                            s.h[i] = ch >> 1;
                            s.log_h[i] = lh - 1;
                        }
                    } else {
                        s.h[i] = ch;
                        s.log_h[i] = lh;
                        if i > 0 {
                            s.x[i] = s.x[i - 1] + s.w[i - 1];
                            s.y[i] = s.y[i - 1];
                        }
                    }
                }
                if mode == SPLIT_EQT_VER {
                    s.x[1] = s.x[0] + s.w[0];
                    s.y[1] = s.y[0];
                    s.x[2] = s.x[1];
                    s.y[2] = s.y[1] + s.h[1];
                    s.x[3] = s.x[1] + s.w[1];
                    s.y[3] = s.y[1];
                }
            } else {
                for i in 0..s.count {
                    if mode == SPLIT_EQT_HOR {
                        if i == 0 || i == 3 {
                            s.w[i] = cw;
                            s.log_w[i] = lw;
                        } else {
                            s.w[i] = cw >> 1;
                            s.log_w[i] = lw - 1;
                        }
                    } else {
                        s.w[i] = cw;
                        s.log_w[i] = lw;
                        if i > 0 {
                            s.y[i] = s.y[i - 1] + s.h[i - 1];
                            s.x[i] = s.x[i - 1];
                        }
                    }
                    s.h[i] = part_size(mode, i, ch);
                    s.log_h[i] = part_size_idx(mode, i, lh);
                }
                if mode == SPLIT_EQT_HOR {
                    s.y[1] = s.y[0] + s.h[0];
                    s.x[1] = s.x[0];
                    s.y[2] = s.y[1];
                    s.x[2] = s.x[1] + s.w[1];
                    s.y[3] = s.y[1] + s.h[1];
                    s.x[3] = s.x[1];
                }
            }
        }
    }
    s
}

/// `dec_get_split_available`: bit mask of the allowed split modes.
pub(crate) fn split_available(seq: &Sequence, x: usize, y: usize, cw: usize, ch: usize, qt_depth: u32, bet_depth: u32, st: SliceType) -> u32 {
    let (pw, ph) = (seq.pic_width, seq.pic_height);
    let mut tab = 0u32;
    if x + cw > pw || y + ch > ph {
        if (cw == 64 && ch == 128) || (ch == 64 && cw == 128) {
            tab = (1 << SPLIT_BI_HOR) | (1 << SPLIT_BI_VER);
        } else if st == SliceType::I && cw == 128 && ch == 128 {
            tab = (1 << SPLIT_QUAD) | (1 << SPLIT_NONE);
        } else if y + ch <= ph {
            tab = 1 << SPLIT_BI_VER;
        } else if x + cw <= pw {
            tab = 1 << SPLIT_BI_HOR;
        } else {
            tab = 1 << SPLIT_QUAD;
        }
    } else if (cw == 64 && ch == 128) || (ch == 64 && cw == 128) {
        tab = (1 << SPLIT_NONE) | (1 << SPLIT_BI_HOR) | (1 << SPLIT_BI_VER);
    } else if qt_depth + bet_depth < seq.max_split_times {
        if st == SliceType::I && cw == 128 && ch == 128 {
            tab = (1 << SPLIT_NONE) | (1 << SPLIT_QUAD);
        } else {
            let max_bt = seq.max_bt_size as usize;
            let max_eqt = seq.max_eqt_size as usize;
            let max_aspect = seq.max_part_ratio_log2;
            let min_eqt = 4usize;
            let min_bt = 4usize;
            tab = (1 << SPLIT_NONE) | (u32::from(bet_depth == 0 && cw > seq.min_qt_size as usize) << SPLIT_QUAD);
            if cw <= max_bt && ch <= max_bt {
                tab |=
                    (u32::from(ch > min_bt && cw < (ch << max_aspect)) << SPLIT_BI_HOR) | (u32::from(cw > min_bt && ch < (cw << max_aspect)) << SPLIT_BI_VER);
            }
            if cw <= max_eqt && ch <= max_eqt && cw > min_eqt && ch > min_eqt {
                let a = max_aspect - 1;
                tab |=
                    (u32::from(ch > (min_eqt << 1) && cw < (ch << a)) << SPLIT_EQT_HOR) | (u32::from(cw > (min_eqt << 1) && ch < (cw << a)) << SPLIT_EQT_VER);
            }
        }
    }
    tab
}

pub(crate) fn cons_allow(w: usize, h: usize, split: u8) -> bool {
    let s = w * h;
    ((split == SPLIT_EQT_HOR || split == SPLIT_EQT_VER) && s == 128) || ((split == SPLIT_BI_HOR || split == SPLIT_BI_VER || split == SPLIT_QUAD) && s == 64)
}

pub(crate) fn is_separate_tree(w: usize, h: usize, split: u8) -> bool {
    match split {
        SPLIT_QUAD => w == 8,
        SPLIT_EQT_HOR => h == 16 || w == 8,
        SPLIT_EQT_VER => w == 16 || h == 8,
        SPLIT_BI_HOR => h == 8,
        SPLIT_BI_VER => w == 8,
        _ => false,
    }
}

/// `dec_dt_allow`: bit 0 horizontal, bit 1 vertical partitions allowed.
pub(crate) fn dt_allow(cw: usize, ch: usize, max_size: usize) -> u32 {
    if cw <= max_size && ch <= max_size {
        let hori = ch >= 16 && cw < (ch << 2);
        let vert = cw >= 16 && ch < (cw << 2);
        u32::from(hori) + (u32::from(vert) << 1)
    } else {
        0
    }
}

pub(crate) fn part_num(size: u8) -> usize {
    const N: [usize; 8] = [1, 2, 2, 2, 2, 4, 4, 4];
    N[size as usize & 7]
}

pub(crate) fn tb_part_of_pb(pb: u8) -> u8 {
    match pb {
        SIZE_2NXNU | SIZE_2NXND | SIZE_2NXHN => SIZE_2NXHN,
        SIZE_NLX2N | SIZE_NRX2N | SIZE_HNX2N => SIZE_HNX2N,
        SIZE_NXN => SIZE_NXN,
        _ => SIZE_2NX2N,
    }
}

pub(crate) fn pb_idx_by_tb(pb: u8, idx: usize) -> usize {
    match pb {
        SIZE_2NXNU | SIZE_NLX2N => usize::from(idx != 0),
        SIZE_2NXND | SIZE_NRX2N => usize::from(idx == 3),
        _ => idx,
    }
}

pub(crate) fn tb_width_height(w: usize, h: usize, part: u8) -> (usize, usize) {
    match part {
        SIZE_NXN => (w >> 1, h >> 1),
        SIZE_2NXHN => (w, h >> 2),
        SIZE_HNX2N => (w >> 2, h),
        _ => (w, h),
    }
}

pub(crate) fn tb_start_pos(w: usize, h: usize, part: u8, idx: usize) -> (usize, usize) {
    match part {
        SIZE_NXN => ((idx % 2) * w / 2, (idx / 2) * h / 2),
        SIZE_2NXHN => (0, idx * (h / 4)),
        SIZE_HNX2N => (idx * (w / 4), 0),
        _ => (0, 0),
    }
}

/// `dec_get_part_info`: x, y, w, h of each prediction block.
pub(crate) fn part_info(x: usize, y: usize, w: usize, h: usize, part: u8) -> Vec<(usize, usize, usize, usize)> {
    let qw = w >> 2;
    let qh = h >> 2;
    match part {
        SIZE_2NXHN => (0..4).map(|i| (x, qh * i + y, w, qh)).collect(),
        SIZE_2NXNU => vec![(x, y, w, qh), (x, qh + y, w, qh * 3)],
        SIZE_2NXND => vec![(x, y, w, qh * 3), (x, qh * 3 + y, w, qh)],
        SIZE_HNX2N => (0..4).map(|i| (qw * i + x, y, qw, h)).collect(),
        SIZE_NLX2N => vec![(x, y, qw, h), (qw + x, y, qw * 3, h)],
        SIZE_NRX2N => vec![(x, y, qw * 3, h), (qw * 3 + x, y, qw, h)],
        SIZE_NXN => (0..4).map(|i| (qw * if i == 0 || i == 2 { 0 } else { 2 } + x, qh * if i == 0 || i == 1 { 0 } else { 2 } + y, qw * 2, qh * 2)).collect(),
        _ => vec![(x, y, w, h)],
    }
}

/// `dec_get_avail_intra`
pub(crate) fn avail_intra(m: &Maps, scup: i32) -> u8 {
    let i = m.i_scu as i32;
    (if m.scu[m.ix(scup - 1)].coded { AVAIL_LE } else { 0 })
        | (if m.scu[m.ix(scup - i)].coded { AVAIL_UP } else { 0 })
        | (if m.scu[m.ix(scup - i - 1)].coded { AVAIL_UL } else { 0 })
}

/// `dec_update_map_for_intra`
pub(crate) fn update_map_for_intra(m: &mut Maps, tb_x: usize, tb_y: usize, tb_w: usize, tb_h: usize, ipm: i8) {
    let i_scu = m.i_scu;
    let scup = ((tb_y >> 2) * i_scu + (tb_x >> 2)) as i32;
    let scu = Scu { coded: true, intra: true, ..Scu::default() };
    for j in 0..(tb_h >> 2) {
        for i in 0..(tb_w >> 2) {
            let ix = m.ix(scup + (j * i_scu + i) as i32);
            m.ipm[ix] = ipm;
            m.scu[ix] = scu;
        }
    }
}

impl Core {
    /// `dec_update_map`
    pub(crate) fn update_map(&mut self) {
        let scup = self.cu_scup;
        let i_scu = self.seq.i_scu;
        let w_cu = 1usize << (self.cu_log2w - 2);
        let h_cu = 1usize << (self.cu_log2h - 2);
        let intra = self.cu_mode == MODE_INTRA;
        let scu = Scu {
            coded: true,
            intra,
            inter: !intra,
            affine: self.affine_flag,
            skip: self.cu_mode == MODE_SKIP,
            cbf: self.cbfy.iter().any(|&c| c != 0) || self.cbfc.iter().any(|&c| c != 0) || intra,
        };
        let mut cu_pos = 0u32;
        mcu_set_logw(&mut cu_pos, self.cu_log2w);
        mcu_set_logh(&mut cu_pos, self.cu_log2h);
        mcu_set_scup(&mut cu_pos, self.cu_scup as u32);
        let base = self.maps.ix(scup);
        let affine = self.affine_flag != 0;
        let refi = self.refi;
        let mv = self.mv;
        let m = &mut self.maps;
        let single_col = !affine && (w_cu & 1) != 0;
        for i in 0..h_cu {
            let row = base + i * i_scu;
            for j in 0..w_cu {
                m.scu[row + j] = scu;
                m.refi[row + j] = refi;
                if !affine {
                    m.mv[row + j] = mv;
                }
            }
            if single_col {
                m.pos[row] = cu_pos;
            } else {
                m.pos[row + w_cu - 1] = cu_pos;
            }
        }
        let last = base + (h_cu - 1) * i_scu;
        for j in 0..w_cu {
            m.pos[last + j] = cu_pos;
        }
        if affine {
            self.set_affine_mvf();
        }
    }
}
