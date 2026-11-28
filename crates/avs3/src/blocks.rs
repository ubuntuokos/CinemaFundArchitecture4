// SPDX-License-Identifier: GPL-3.0-or-later
//! Block-structure helpers: the coding-tree split geometry, partitions and transform blocks.
//!
//! Derived from uavs3d 1.2 (commit 0e20d2c2), Copyright (c) 2018-2022 Peking University
//! Shenzhen Graduate School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation
//! Corporation, BSD 3-clause licence (docs/licensing.md).

use crate::header::{Sequence, SliceType};

pub(crate) const NO_SPLIT: u8 = 0;
pub(crate) const SPLIT_BI_VER: u8 = 1;
pub(crate) const SPLIT_BI_HOR: u8 = 2;
pub(crate) const SPLIT_EQT_VER: u8 = 3;
pub(crate) const SPLIT_EQT_HOR: u8 = 4;
pub(crate) const SPLIT_QUAD: u8 = 5;

pub(crate) const SIZE_2NX2N: u8 = 0;
pub(crate) const SIZE_2NXNU: u8 = 1;
pub(crate) const SIZE_2NXND: u8 = 2;
pub(crate) const SIZE_NLX2N: u8 = 3;
pub(crate) const SIZE_NRX2N: u8 = 4;
pub(crate) const SIZE_NXN: u8 = 5;
pub(crate) const SIZE_2NXHN: u8 = 6;
pub(crate) const SIZE_HNX2N: u8 = 7;

pub(crate) const PART_NUM: [usize; 8] = [1, 2, 2, 2, 2, 4, 4, 4];
pub(crate) const TB_PART: [u8; 8] = [SIZE_2NX2N, SIZE_2NXHN, SIZE_2NXHN, SIZE_HNX2N, SIZE_HNX2N, SIZE_NXN, SIZE_2NXHN, SIZE_HNX2N];

#[derive(Clone, Copy, Default)]
pub(crate) struct Split {
    pub count: usize,
    pub log_w: [u32; 4],
    pub log_h: [u32; 4],
    pub x: [i32; 4],
    pub y: [i32; 4],
}

fn is_vertical(m: u8) -> bool {
    m == SPLIT_BI_VER || m == SPLIT_EQT_VER
}
fn is_eqt(m: u8) -> bool {
    m == SPLIT_EQT_HOR || m == SPLIT_EQT_VER
}
fn is_bt(m: u8) -> bool {
    m == SPLIT_BI_HOR || m == SPLIT_BI_VER
}

fn part_size(mode: u8, n: usize, len: i32) -> i32 {
    match mode {
        SPLIT_QUAD | SPLIT_BI_HOR | SPLIT_BI_VER => len >> 1,
        SPLIT_EQT_HOR | SPLIT_EQT_VER => {
            if n == 1 || n == 2 {
                len >> 1
            } else {
                len >> 2
            }
        }
        _ => len,
    }
}

fn part_size_log(mode: u8, n: usize, l: u32) -> u32 {
    match mode {
        SPLIT_QUAD | SPLIT_BI_HOR | SPLIT_BI_VER => l.saturating_sub(1),
        SPLIT_EQT_HOR | SPLIT_EQT_VER => {
            if n == 1 || n == 2 {
                l.saturating_sub(1)
            } else {
                l.saturating_sub(2)
            }
        }
        _ => l,
    }
}

pub(crate) fn split_struct(mode: u8, x0: i32, y0: i32, w: i32, h: i32) -> Split {
    let lw = w.max(1).trailing_zeros();
    let lh = h.max(1).trailing_zeros();
    let mut s = Split { count: [0, 2, 2, 4, 4, 4][mode.min(5) as usize], ..Split::default() };
    s.x[0] = x0;
    s.y[0] = y0;
    let mut width = [0i32; 4];
    let mut height = [0i32; 4];
    match mode {
        NO_SPLIT => {
            s.log_w[0] = lw;
            s.log_h[0] = lh;
        }
        SPLIT_QUAD => {
            let (hw, hh) = (w >> 1, h >> 1);
            for i in 0..4 {
                width[i] = hw;
                height[i] = hh;
                s.log_w[i] = lw.saturating_sub(1);
                s.log_h[i] = lh.saturating_sub(1);
            }
            s.x[1] = x0 + hw;
            s.y[1] = y0;
            s.x[2] = x0;
            s.y[2] = y0 + hh;
            s.x[3] = s.x[1];
            s.y[3] = s.y[2];
        }
        _ => {
            if is_vertical(mode) {
                for i in 0..s.count {
                    width[i] = part_size(mode, i, w);
                    s.log_w[i] = part_size_log(mode, i, lw);
                    if mode == SPLIT_EQT_VER {
                        if i == 0 || i == 3 {
                            height[i] = h;
                            s.log_h[i] = lh;
                        } else {
                            height[i] = h >> 1;
                            s.log_h[i] = lh.saturating_sub(1);
                        }
                    } else {
                        height[i] = h;
                        s.log_h[i] = lh;
                        if i > 0 {
                            s.x[i] = s.x[i - 1] + width[i - 1];
                            s.y[i] = s.y[i - 1];
                        }
                    }
                }
                if mode == SPLIT_EQT_VER {
                    s.x[1] = s.x[0] + width[0];
                    s.y[1] = s.y[0];
                    s.x[2] = s.x[1];
                    s.y[2] = s.y[1] + height[1];
                    s.x[3] = s.x[1] + width[1];
                    s.y[3] = s.y[1];
                }
            } else {
                for i in 0..s.count {
                    if mode == SPLIT_EQT_HOR {
                        if i == 0 || i == 3 {
                            width[i] = w;
                            s.log_w[i] = lw;
                        } else {
                            width[i] = w >> 1;
                            s.log_w[i] = lw.saturating_sub(1);
                        }
                    } else {
                        width[i] = w;
                        s.log_w[i] = lw;
                        if i > 0 {
                            s.y[i] = s.y[i - 1] + height[i - 1];
                            s.x[i] = s.x[i - 1];
                        }
                    }
                    height[i] = part_size(mode, i, h);
                    s.log_h[i] = part_size_log(mode, i, lh);
                }
                if mode == SPLIT_EQT_HOR {
                    s.y[1] = s.y[0] + height[0];
                    s.x[1] = s.x[0];
                    s.y[2] = s.y[1];
                    s.x[2] = s.x[1] + width[1];
                    s.y[3] = s.y[1] + height[1];
                    s.x[3] = s.x[1];
                }
            }
        }
    }
    s
}

/// Bit mask of the allowed split modes (bit n set = mode n allowed).
pub(crate) fn split_available(s: &Sequence, x: i32, y: i32, w: i32, h: i32, qt_depth: u32, bet_depth: u32, st: SliceType) -> u32 {
    let (pw, ph) = (s.pic_width as i32, s.pic_height as i32);
    let bit = |m: u8| 1u32 << m;
    let mut tab = 0u32;
    if x + w > pw || y + h > ph {
        if (w == 64 && h == 128) || (h == 64 && w == 128) {
            tab = bit(SPLIT_BI_HOR) | bit(SPLIT_BI_VER);
        } else if st == SliceType::I && w == 128 && h == 128 {
            tab = bit(SPLIT_QUAD) | bit(NO_SPLIT);
        } else if y + h <= ph {
            tab = bit(SPLIT_BI_VER);
        } else if x + w <= pw {
            tab = bit(SPLIT_BI_HOR);
        } else {
            tab = bit(SPLIT_QUAD);
        }
    } else if (w == 64 && h == 128) || (h == 64 && w == 128) {
        tab = bit(NO_SPLIT) | bit(SPLIT_BI_HOR) | bit(SPLIT_BI_VER);
    } else if qt_depth + bet_depth < s.max_split_times {
        if st == SliceType::I && w == 128 && h == 128 {
            tab = bit(NO_SPLIT) | bit(SPLIT_QUAD);
        } else {
            let (max_bt, max_eqt) = (s.max_bt_size as i32, s.max_eqt_size as i32);
            let asp = s.max_part_ratio_log2;
            let min = s.min_cu_size as i32;
            tab = bit(NO_SPLIT) | (u32::from(bet_depth == 0 && w > s.min_qt_size as i32) << SPLIT_QUAD);
            if w <= max_bt && h <= max_bt {
                tab |= (u32::from(h > min && i64::from(w) < i64::from(h) << asp) << SPLIT_BI_HOR)
                    | (u32::from(w > min && i64::from(h) < i64::from(w) << asp) << SPLIT_BI_VER);
            }
            if w <= max_eqt && h <= max_eqt && w > min && h > min {
                let a = asp - 1;
                tab |= (u32::from(h > (min << 1) && i64::from(w) < i64::from(h) << a) << SPLIT_EQT_HOR)
                    | (u32::from(w > (min << 1) && i64::from(h) < i64::from(w) << a) << SPLIT_EQT_VER);
            }
        }
    }
    tab
}

pub(crate) fn cons_allow(w: i32, h: i32, split: u8) -> bool {
    let s = w * h;
    (is_eqt(split) && s == 128) || ((is_bt(split) || split == SPLIT_QUAD) && s == 64)
}

/// DT intra allowance: bit 0 horizontal, bit 1 vertical.
pub(crate) fn dt_allow(w: i32, h: i32, max_size: i32) -> u32 {
    if w <= max_size && h <= max_size {
        let hori = h >= 16 && w < (h << 2);
        let vert = w >= 16 && h < (w << 2);
        u32::from(hori) + (u32::from(vert) << 1)
    } else {
        0
    }
}

pub(crate) fn is_separate_tree(w: i32, h: i32, split: u8) -> bool {
    match split {
        SPLIT_QUAD => w == 8,
        SPLIT_EQT_HOR => h == 16 || w == 8,
        SPLIT_EQT_VER => w == 16 || h == 8,
        SPLIT_BI_HOR => h == 8,
        SPLIT_BI_VER => w == 8,
        _ => false,
    }
}

/// Partition rectangles `(x, y, w, h)` of a CU for a partition size.
pub(crate) fn part_info(part: u8, x: i32, y: i32, w: i32, h: i32) -> Vec<(i32, i32, i32, i32)> {
    let (qw, qh) = (w >> 2, h >> 2);
    match part {
        SIZE_2NXHN => (0..4).map(|i| (x, y + qh * i, w, qh)).collect(),
        SIZE_2NXNU => vec![(x, y, w, qh), (x, y + qh, w, qh * 3)],
        SIZE_2NXND => vec![(x, y, w, qh * 3), (x, y + qh * 3, w, qh)],
        SIZE_HNX2N => (0..4).map(|i| (x + qw * i, y, qw, h)).collect(),
        SIZE_NLX2N => vec![(x, y, qw, h), (x + qw, y, qw * 3, h)],
        SIZE_NRX2N => vec![(x, y, qw * 3, h), (x + qw * 3, y, qw, h)],
        SIZE_NXN => (0..4).map(|i| (x + qw * if i % 2 == 0 { 0 } else { 2 }, y + qh * if i < 2 { 0 } else { 2 }, qw * 2, qh * 2)).collect(),
        _ => vec![(x, y, w, h)],
    }
}

pub(crate) fn pb_idx_by_tb(pb_part: u8, idx: usize) -> usize {
    match pb_part {
        SIZE_2NXNU | SIZE_NLX2N => usize::from(idx != 0),
        SIZE_2NXND | SIZE_NRX2N => usize::from(idx == 3),
        _ => idx,
    }
}

pub(crate) fn tb_size(w: i32, h: i32, part: u8) -> (i32, i32) {
    match part {
        SIZE_NXN => (w >> 1, h >> 1),
        SIZE_2NXHN => (w, h >> 2),
        SIZE_HNX2N => (w >> 2, h),
        _ => (w, h),
    }
}

pub(crate) fn tb_start(w: i32, h: i32, part: u8, idx: usize) -> (i32, i32) {
    let i = idx as i32;
    match part {
        SIZE_NXN => ((i % 2) * w / 2, (i / 2) * h / 2),
        SIZE_2NXHN => (0, i * (h / 4)),
        SIZE_HNX2N => (i * (w / 4), 0),
        _ => (0, 0),
    }
}
