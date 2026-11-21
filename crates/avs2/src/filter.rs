// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from davs2 1.7 (`source/common/deblock.cc`, `sao.cc`, `alf.cc`), Copyright (C) 2018~ VCL, NELVT, Peking
// University (Falei Luo et al.), GPL-2.0-or-later.
//! In-loop filters: deblocking, SAO and ALF (davs2 `deblock.cc`, `sao.cc`, `alf.cc`).
//!
//! davs2 runs them row by row interleaved with reconstruction; every read stays inside a window
//! that the interleaving has already finished (intra prediction uses unfiltered borders, SAO and
//! ALF read copies limited to their LCU windows), so running each filter over the whole picture
//! in the same LCU order gives identical samples.

use crate::cu::*;
use crate::header::{AlfParam, FrameType};
use crate::recon::{Frame, Plane};

const ALPHA_TABLE: [i32; 64] = [
    0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 2, 2, 2, 3, 3, 4, 4, 5, 5, 6, 7, 8, 9, 10, 11, 12, 13, 15, 16, 18, 20, 22, 24, 26, 28, 30, 33, 33, 35, 35, 36, 37, 37, 39,
    39, 42, 44, 46, 48, 50, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64,
];
const BETA_TABLE: [i32; 64] = [
    0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 6, 6, 6, 7, 7, 7, 8, 8, 8, 9, 9, 10, 10, 11, 11, 12, 13, 14, 15,
    16, 17, 18, 19, 20, 21, 22, 23, 23, 24, 24, 25, 25, 26, 27,
];

const EDGE_VER: usize = 0;
const EDGE_HOR: usize = 1;
const EDGE_NOFILTER: u8 = 0;
const EDGE_ONLY_LUMA: u8 = 1;
const EDGE_BOTH: u8 = 2;

/// Deblocking state of one picture.
pub(crate) struct Deblock<'a> {
    pub st: &'a PicState,
    pub cross_loop_filter: bool,
    pub nsqt: bool,
    pub alpha_offset: i32,
    pub beta_offset: i32,
    flags: [Vec<u8>; 2],
}

impl<'a> Deblock<'a> {
    pub(crate) fn new(st: &'a PicState, cross_loop_filter: bool, nsqt: bool, alpha_offset: i32, beta_offset: i32) -> Self {
        let n = (st.w_scu * st.h_scu).max(0) as usize;
        Deblock { st, cross_loop_filter, nsqt, alpha_offset, beta_offset, flags: [vec![0; n], vec![0; n]] }
    }

    fn scu(&self, xy: i32) -> Option<&Cu> {
        if xy < 0 {
            return None;
        }
        self.st.scu.get(xy as usize)
    }

    /// `lf_set_edge_filter_param`.
    fn set_edge(&mut self, level: i32, scu_x: i32, scu_y: i32, dir: usize, edge_type: u8) {
        let w = self.st.w_scu;
        let n = 1 << (level - MIN_CU_SIZE_IN_BIT);
        let xy = scu_y * w + scu_x;
        let cur = self.scu(xy).map(|c| c.slice_nr);
        if dir == EDGE_VER {
            if scu_x == 0 {
                return;
            }
            if !self.cross_loop_filter && cur != self.scu(xy - 1).map(|c| c.slice_nr) {
                return;
            }
            for i in 0..n {
                let idx = ((scu_y + i) * w + scu_x).max(0) as usize;
                let Some(f) = self.flags[EDGE_VER].get_mut(idx) else { break };
                if *f != EDGE_NOFILTER {
                    break;
                }
                *f = edge_type;
            }
        } else {
            if scu_y == 0 {
                return;
            }
            if !self.cross_loop_filter && cur != self.scu(xy - w).map(|c| c.slice_nr) {
                return;
            }
            for i in 0..n {
                let idx = (scu_y * w + scu_x + i).max(0) as usize;
                let Some(f) = self.flags[EDGE_HOR].get_mut(idx) else { break };
                if *f != EDGE_NOFILTER {
                    break;
                }
                *f = edge_type;
            }
        }
    }

    /// `lf_lcu_set_edge_filter`.
    fn set_lcu_edges(&mut self, level: i32, scu_x: i32, scu_y: i32) {
        let st = self.st;
        let Some(cu) = self.scu(scu_y * st.w_scu + scu_x).copied() else { return };
        if cu.level < level {
            for i in 0..4 {
                let sx = scu_x + ((i & 1) << (level - MIN_CU_SIZE_IN_BIT - 1));
                let sy = scu_y + ((i >> 1) << (level - MIN_CU_SIZE_IN_BIT - 1));
                if sx >= st.w_scu || sy >= st.h_scu {
                    continue;
                }
                self.set_lcu_edges(level - 1, sx, sy);
            }
            return;
        }
        self.set_edge(level, scu_x, scu_y, EDGE_VER, EDGE_BOTH);
        self.set_edge(level, scu_x, scu_y, EDGE_HOR, EDGE_BOTH);
        if cu.level <= B8X8_IN_BIT {
            return;
        }
        let i = level - MIN_CU_SIZE_IN_BIT - 1;
        let h = 1 << (i - 1).max(0);
        match cu.cu_type {
            PRED_2NXN => self.set_edge(level, scu_x, scu_y + (1 << i), EDGE_HOR, EDGE_BOTH),
            PRED_NX2N => self.set_edge(level, scu_x + (1 << i), scu_y, EDGE_VER, EDGE_BOTH),
            PRED_I_NXN => {
                self.set_edge(level, scu_x + (1 << i), scu_y, EDGE_VER, EDGE_BOTH);
                self.set_edge(level, scu_x, scu_y + (1 << i), EDGE_HOR, EDGE_BOTH);
            }
            PRED_I_2NXN => {
                if i > 0 {
                    for k in 1..4 {
                        self.set_edge(level, scu_x, scu_y + h * k, EDGE_HOR, EDGE_ONLY_LUMA);
                    }
                } else {
                    self.set_edge(level, scu_x, scu_y + (1 << i), EDGE_HOR, EDGE_ONLY_LUMA);
                }
            }
            PRED_I_NX2N => {
                if i > 0 {
                    for k in 1..4 {
                        self.set_edge(level, scu_x + h * k, scu_y, EDGE_VER, EDGE_ONLY_LUMA);
                    }
                } else {
                    self.set_edge(level, scu_x + (1 << i), scu_y, EDGE_VER, EDGE_ONLY_LUMA);
                }
            }
            PRED_2NXNU if i > 0 => self.set_edge(level, scu_x, scu_y + h, EDGE_HOR, EDGE_BOTH),
            PRED_2NXND if i > 0 => self.set_edge(level, scu_x, scu_y + h * 3, EDGE_HOR, EDGE_BOTH),
            PRED_NLX2N if i > 0 => self.set_edge(level, scu_x + h, scu_y, EDGE_VER, EDGE_BOTH),
            PRED_NRX2N if i > 0 => self.set_edge(level, scu_x + h * 3, scu_y, EDGE_VER, EDGE_BOTH),
            _ => {}
        }
        if cu.cu_type != PRED_I_NXN && cu.trans_size != TU_SPLIT_NON && cu.cbp != 0 {
            if self.nsqt && is_hor_pu_part(cu.cu_type) {
                if cu.level == B16X16_IN_BIT {
                    self.set_edge(level, scu_x, scu_y + (1 << i), EDGE_HOR, EDGE_ONLY_LUMA);
                } else {
                    self.set_edge(level, scu_x, scu_y + h, EDGE_HOR, EDGE_ONLY_LUMA);
                    self.set_edge(level, scu_x, scu_y + (1 << i), EDGE_HOR, EDGE_ONLY_LUMA);
                    self.set_edge(level, scu_x, scu_y + (1 << i) + h, EDGE_HOR, EDGE_ONLY_LUMA);
                }
            } else if self.nsqt && is_ver_pu_part(cu.cu_type) {
                if cu.level == B16X16_IN_BIT {
                    self.set_edge(level, scu_x + (1 << i), scu_y, EDGE_VER, EDGE_ONLY_LUMA);
                } else {
                    self.set_edge(level, scu_x + h, scu_y, EDGE_VER, EDGE_ONLY_LUMA);
                    self.set_edge(level, scu_x + (1 << i), scu_y, EDGE_VER, EDGE_ONLY_LUMA);
                    self.set_edge(level, scu_x + (1 << i) + h, scu_y, EDGE_VER, EDGE_ONLY_LUMA);
                }
            } else {
                self.set_edge(level, scu_x + (1 << i), scu_y, EDGE_VER, EDGE_ONLY_LUMA);
                self.set_edge(level, scu_x, scu_y + (1 << i), EDGE_HOR, EDGE_ONLY_LUMA);
            }
        }
    }

    /// `lf_skip_filter`: 0 when the 4-sample edge segment needs no filtering.
    fn skip_filter(&self, p: &Cu, q: &Cu, dir: usize, bx: i32, by: i32) -> bool {
        let st = self.st;
        if st.frame_type == FrameType::P || st.frame_type == FrameType::F {
            let w = st.w_spu;
            let d = dir as i32;
            let pos1 = (by * w + bx).max(0) as usize;
            let pos2 = ((by - d) * w + (bx - (1 - d))).max(0) as usize;
            let r1 = st.ref_idx.get(pos1).map_or(INVALID_REF, |r| r.r[0]);
            let r2 = st.ref_idx.get(pos2).map_or(INVALID_REF, |r| r.r[0]);
            let m1 = st.tmv[0].get(pos1).copied().unwrap_or_default();
            let m2 = st.tmv[0].get(pos2).copied().unwrap_or_default();
            if p.cbp == 0
                && q.cbp == 0
                && (i32::from(m1.x) - i32::from(m2.x)).abs() < 4
                && (i32::from(m1.y) - i32::from(m2.y)).abs() < 4
                && r1 != INVALID_REF
                && r1 == r2
            {
                return false;
            }
        }
        true
    }

    /// `lf_scu_deblock`.
    fn scu_deblock(&self, f: &mut Frame, scu_x: i32, scu_y: i32, dir: usize) {
        let st = self.st;
        let xy = scu_y * st.w_scu + scu_x;
        let cond = self.flags[dir].get(xy.max(0) as usize).copied().unwrap_or(EDGE_NOFILTER);
        if cond == EDGE_NOFILTER {
            return;
        }
        let Some(q) = self.scu(xy) else { return };
        let Some(p) = self.scu(if dir == EDGE_HOR { xy - st.w_scu } else { xy - 1 }) else { return };
        let d = dir as i32;
        let flags = [self.skip_filter(p, q, dir, scu_x << 1, scu_y << 1), self.skip_filter(p, q, dir, (scu_x << 1) + d, (scu_y << 1) + (1 - d))];
        if !flags[0] && !flags[1] {
            return;
        }
        let shift = st.bit_depth - 8;
        let qp = (p.qp + q.qp + 1) >> 1;
        let alpha = ALPHA_TABLE[(qp - (shift << 3) + self.alpha_offset).clamp(0, 63) as usize] << shift;
        let beta = BETA_TABLE[(qp - (shift << 3) + self.beta_offset).clamp(0, 63) as usize] << shift;
        edge_core(&mut f.planes[0], scu_x << 3, scu_y << 3, dir, false, alpha, beta, flags);
        if cond == EDGE_BOTH && !st.mono && (((scu_y & 1) == 0 && dir == EDGE_HOR) || ((scu_x & 1) == 0 && dir == EDGE_VER)) {
            let cqp = st.chroma_qp(qp, 0) - (shift << 3);
            let alpha = ALPHA_TABLE[(cqp + self.alpha_offset).clamp(0, 63) as usize] << shift;
            let beta = BETA_TABLE[(cqp + self.beta_offset).clamp(0, 63) as usize] << shift;
            for plane in 1..3 {
                edge_core(&mut f.planes[plane], scu_x << 2, scu_y << 2, dir, true, alpha, beta, flags);
            }
        }
    }

    /// `davs2_lcu_deblock`.
    pub(crate) fn lcu(&mut self, f: &mut Frame, lcu_x: i32, lcu_y: i32) {
        let st = self.st;
        let n = 1 << (st.lcu_level - MIN_CU_SIZE_IN_BIT);
        let mut scu_x = lcu_x << (st.lcu_level - MIN_CU_SIZE_IN_BIT);
        let scu_y = lcu_y << (st.lcu_level - MIN_CU_SIZE_IN_BIT);
        let mut nh = (st.w_scu - scu_x).min(n);
        let nv = (st.h_scu - scu_y).min(n);
        self.set_lcu_edges(st.lcu_level, scu_x, scu_y);
        for j in 0..nv {
            for i in 0..nh {
                self.scu_deblock(f, scu_x + i, scu_y + j, EDGE_VER);
            }
        }
        if scu_x == 0 {
            nh -= 1;
        } else {
            if scu_x + nh == st.w_scu {
                nh += 1;
            }
            scu_x -= 1;
        }
        for j in 0..nv {
            for i in 0..nh {
                self.scu_deblock(f, scu_x + i, scu_y + j, EDGE_HOR);
            }
        }
    }
}

/// `lf_edge_core` on the edge at (`x`, `y`) of `plane` (8 luma or 4 chroma samples long).
#[allow(clippy::too_many_arguments)]
fn edge_core(p: &mut Plane, x: i32, y: i32, dir: usize, chroma: bool, alpha: i32, beta: i32, flt: [bool; 2]) {
    // dir VER: samples across x, step along y; dir HOR: samples across y, step along x.
    let at = |k: i32, off: i32| -> (i32, i32) { if dir == EDGE_VER { (x + off, y + k) } else { (x + k, y + off) } };
    let mut pel = 0;
    let mut k = 0;
    while pel < 8 {
        let g = |p: &Plane, off: i32| {
            let (px, py) = at(k, off);
            p.get(px, py)
        };
        let l2 = g(p, -3);
        let l1 = g(p, -2);
        let l0 = g(p, -1);
        let r0 = g(p, 0);
        let r1 = g(p, 1);
        let r2 = g(p, 2);
        let delta = (r0 - l0).abs();
        let flag = if pel < 4 { flt[0] } else { flt[1] };
        if flag && delta < alpha && delta > 1 {
            let mut fl = if (l1 - l0).abs() < beta { 2 } else { 0 };
            fl += i32::from((l2 - l0).abs() < beta);
            let mut fr = if (r0 - r1).abs() < beta { 2 } else { 0 };
            fr += i32::from((r0 - r2).abs() < beta);
            let mut fs = match fl + fr {
                6 => 3 + i32::from(r1 == r0 && l0 == l1),
                5 => 2 + i32::from(r1 == r0 && l0 == l1),
                4 => 1 + i32::from(fl == 2),
                3 => i32::from((l1 - r1).abs() < beta),
                _ => 0,
            };
            if chroma && fs > 0 {
                fs -= 1;
            }
            let mut set = |off: i32, v: i32| {
                let (px, py) = at(k, off);
                p.set(px, py, v);
            };
            match fs {
                4 => {
                    set(-1, (l0 + ((l0 + l2) << 3) + l2 + (r0 << 3) + (r2 << 2) + (r2 << 1) + 16) >> 5);
                    set(-2, ((l0 << 3) - l0 + (l2 << 2) + (l2 << 1) + r0 + (r0 << 1) + 8) >> 4);
                    set(-3, ((l0 << 2) + l2 + (l2 << 1) + r0 + 4) >> 3);
                    set(0, (r0 + ((r0 + r2) << 3) + r2 + (l0 << 3) + (l2 << 2) + (l2 << 1) + 16) >> 5);
                    set(1, ((r0 << 3) - r0 + (r2 << 2) + (r2 << 1) + l0 + (l0 << 1) + 8) >> 4);
                    set(2, ((r0 << 2) + r2 + (r2 << 1) + l0 + 4) >> 3);
                }
                3 => {
                    set(-1, (l2 + (l1 << 2) + (l0 << 2) + (l0 << 1) + (r0 << 2) + r1 + 8) >> 4);
                    set(0, (l1 + (l0 << 2) + (r0 << 2) + (r0 << 1) + (r1 << 2) + r2 + 8) >> 4);
                    set(-2, (l2 * 3 + l1 * 8 + l0 * 4 + r0 + 8) >> 4);
                    set(1, (r2 * 3 + r1 * 8 + r0 * 4 + l0 + 8) >> 4);
                }
                2 => {
                    set(-1, ((l1 << 1) + l1 + (l0 << 3) + (l0 << 1) + (r0 << 1) + r0 + 8) >> 4);
                    set(0, ((l0 << 1) + l0 + (r0 << 3) + (r0 << 1) + (r1 << 1) + r1 + 8) >> 4);
                }
                1 => {
                    set(-1, (l0 * 3 + r0 + 2) >> 2);
                    set(0, (r0 * 3 + l0 + 2) >> 2);
                }
                _ => {}
            }
        }
        k += 1;
        pel += 1 + i32::from(chroma);
    }
}

// ---------------------------------------------------------------------------------------------
// SAO

const SAO_SHIFT: i32 = 4;

#[derive(Clone, Copy, Default)]
struct Region {
    top: bool,
    down: bool,
    left: bool,
    right: bool,
    top_left: bool,
    top_right: bool,
    down_left: bool,
    down_right: bool,
    x: [i32; 3],
    y: [i32; 3],
    w: [i32; 3],
    h: [i32; 3],
}

/// `sao_get_neighbor_avail`.
fn sao_region(st: &PicState, cross_loop_filter: bool, lcu_x: i32, lcu_y: i32) -> Region {
    let lv = st.lcu_level;
    let w_lcu = (st.width + (1 << lv) - 1) >> lv;
    let h_lcu = (st.height + (1 << lv) - 1) >> lv;
    let mut px = lcu_x << lv;
    let mut py = lcu_y << lv;
    let mut w = (1 << lv).min(st.width - px);
    let mut h = (1 << lv).min(st.height - py);
    let mut pxc = px >> 1;
    let mut pyc = py >> 1;
    let mut wc = w >> 1;
    let mut hc = h >> 1;
    let mut r = Region { left: lcu_x != 0, top: lcu_y != 0, right: lcu_x < w_lcu - 1, down: lcu_y < h_lcu - 1, ..Default::default() };
    if !cross_loop_filter {
        let sx = lcu_x << (lv - 3);
        let mut sy = lcu_y << (lv - 3);
        let slice = |x: i32, y: i32| st.scu.get((y * st.w_scu + x).max(0) as usize).map(|c| c.slice_nr);
        if r.top {
            r.top = slice(sx, sy) == slice(sx, sy - 1);
        }
        if r.down {
            sy += 1 << (lv - 3);
            r.down = slice(sx, sy) == slice(sx, sy - 1);
        }
    }
    r.top_left = r.top && r.left;
    r.top_right = r.top && r.right;
    r.down_left = r.down && r.left;
    r.down_right = r.down && r.right;
    if !r.right {
        w += SAO_SHIFT;
        wc += SAO_SHIFT;
    }
    if !r.down {
        h += SAO_SHIFT;
        hc += SAO_SHIFT;
    }
    if r.left {
        px -= SAO_SHIFT;
        pxc -= SAO_SHIFT;
    } else {
        w -= SAO_SHIFT;
        wc -= SAO_SHIFT;
    }
    if r.top {
        py -= SAO_SHIFT;
        pyc -= SAO_SHIFT;
    } else {
        h -= SAO_SHIFT;
        hc -= SAO_SHIFT;
    }
    w = w.min(st.width - px);
    wc = wc.min((st.width >> 1) - pxc);
    h = h.min(st.height - py);
    hc = hc.min((st.height >> 1) - pyc);
    r.x = [px, pxc, pxc];
    r.y = [py, pyc, pyc];
    r.w = [w, wc, wc];
    r.h = [h, hc, hc];
    r
}

fn sign(v: i32) -> i32 {
    v.signum()
}

/// One SAO block: reads `src` (the deblocked picture), writes `dst`.
#[allow(clippy::too_many_arguments)]
fn sao_block(dst: &mut Plane, src: &Plane, x0: i32, y0: i32, bw: i32, bh: i32, bd: i32, r: &Region, p: &SaoPlane) {
    let max = (1 << bd) - 1;
    let s = |x: i32, y: i32| src.get(x0 + x, y0 + y);
    let off = |t: i32| p.offset.get(t.clamp(0, 31) as usize).copied().unwrap_or(0);
    let mut put = |x: i32, y: i32, e: i32| {
        let v = (s(x, y) + off(e)).clamp(0, max);
        dst.set(x0 + x, y0 + y, v);
    };
    match p.type_idc {
        4 => {
            let shift = bd - 5;
            for y in 0..bh {
                for x in 0..bw {
                    put(x, y, s(x, y) >> shift);
                }
            }
        }
        0 => {
            let sx = if r.left { 0 } else { 1 };
            let ex = if r.right { bw } else { bw - 1 };
            for y in 0..bh {
                let mut ls = sign(s(sx, y) - s(sx - 1, y));
                for x in sx..ex {
                    let rs = sign(s(x, y) - s(x + 1, y));
                    put(x, y, ls + rs + 2);
                    ls = -rs;
                }
            }
        }
        1 => {
            let sy = if r.top { 0 } else { 1 };
            let ey = if r.down { bh } else { bh - 1 };
            for x in 0..bw {
                let mut ts = sign(s(x, sy) - s(x, sy - 1));
                for y in sy..ey {
                    let ds = sign(s(x, y) - s(x, y + 1));
                    put(x, y, ds + ts + 2);
                    ts = -ds;
                }
            }
        }
        2 => {
            // sao_block_eo_135_c, including its `reg` carry
            let mut up = [0i32; 64 + 32];
            let ui = |x: i32| (x + 16).clamp(0, 95) as usize;
            let mut reg = 0;
            let sx = if r.left { 0 } else { 1 };
            let ex = if r.right { bw } else { bw - 1 };
            for x in sx..ex {
                up[ui(x + 1)] = sign(s(x + 1, 1) - s(x, 0));
            }
            let sx0 = if r.top_left { 0 } else { 1 };
            let ex0 = if r.top { if r.right { bw } else { bw - 1 } } else { 1 };
            for x in sx0..ex0 {
                let ts = sign(s(x, 0) - s(x - 1, -1));
                put(x, 0, ts - up[ui(x + 1)] + 2);
            }
            for y in 1..bh - 1 {
                for x in sx..ex {
                    if x == sx {
                        up[ui(x)] = sign(s(x, y) - s(x - 1, y - 1));
                    }
                    let ds = sign(s(x, y) - s(x + 1, y + 1));
                    put(x, y, ds + up[ui(x)] + 2);
                    up[ui(x)] = reg;
                    reg = -ds;
                }
            }
            let y = bh - 1;
            let sxn = if r.down { if r.left { 0 } else { 1 } } else { bw - 1 };
            let exn = if r.down_right { bw } else { bw - 1 };
            for x in sxn..exn {
                if x == sx {
                    up[ui(x)] = sign(s(x, y) - s(x - 1, y - 1));
                }
                let ds = sign(s(x, y) - s(x + 1, y + 1));
                put(x, y, ds + up[ui(x)] + 2);
            }
        }
        3 => {
            let mut up = [0i32; 64 + 32];
            let ui = |x: i32| (x + 16).clamp(0, 95) as usize;
            let sx = if r.left { 0 } else { 1 };
            let ex = if r.right { bw } else { bw - 1 };
            for x in sx..ex {
                up[ui(x - 1)] = sign(s(x - 1, 1) - s(x, 0));
            }
            let sx0 = if r.top { if r.left { 0 } else { 1 } } else { bw - 1 };
            let ex0 = if r.top_right { bw } else { bw - 1 };
            for x in sx0..ex0 {
                let ts = sign(s(x, 0) - s(x + 1, -1));
                put(x, 0, ts - up[ui(x - 1)] + 2);
            }
            for y in 1..bh - 1 {
                for x in sx..ex {
                    if x == ex - 1 {
                        up[ui(x)] = sign(s(x, y) - s(x + 1, y - 1));
                    }
                    let ds = sign(s(x, y) - s(x - 1, y + 1));
                    put(x, y, ds + up[ui(x)] + 2);
                    up[ui(x - 1)] = -ds;
                }
            }
            let y = bh - 1;
            let sxn = if r.down_left { 0 } else { 1 };
            let exn = if r.down { if r.right { bw } else { bw - 1 } } else { 1 };
            for x in sxn..exn {
                if x == ex - 1 {
                    up[ui(x)] = sign(s(x, y) - s(x + 1, y - 1));
                }
                let ds = sign(s(x, y) - s(x - 1, y + 1));
                put(x, y, ds + up[ui(x)] + 2);
            }
        }
        _ => {}
    }
}

/// SAO over the whole picture. `params` holds one entry per LCU (raster order), `slice_on[row]`
/// the slice SAO flags in effect when that LCU row was filtered.
pub(crate) fn sao(st: &PicState, cross_loop_filter: bool, f: &mut Frame, params: &[SaoParam], slice_on: &[[bool; 3]]) {
    let src = f.clone();
    let lv = st.lcu_level;
    let w_lcu = (st.width + (1 << lv) - 1) >> lv;
    let h_lcu = (st.height + (1 << lv) - 1) >> lv;
    for ly in 0..h_lcu {
        let on = slice_on.get(ly as usize).copied().unwrap_or([false; 3]);
        for lx in 0..w_lcu {
            let Some(param) = params.get((ly * w_lcu + lx) as usize) else { continue };
            let r = sao_region(st, cross_loop_filter, lx, ly);
            for c in 0..3 {
                if st.mono && c > 0 {
                    continue;
                }
                let pl = &param.planes[c];
                if !on[c] || pl.mode == SaoMode::Off {
                    continue;
                }
                sao_block(&mut f.planes[c], &src.planes[c], r.x[c], r.y[c], r.w[c], r.h[c], st.bit_depth, &r, pl);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// ALF

const REGION_TABLE: [usize; 16] = [0, 1, 4, 5, 15, 2, 3, 6, 14, 11, 10, 7, 13, 12, 9, 8];

/// `alf_recon_coefficients` for filter `j`.
fn alf_coeffs(p: &AlfParam, j: usize) -> [i32; 9] {
    let mut c = [0i32; 9];
    let Some(m) = p.coeffmulti.get(j) else { return c };
    let mut sum = 0;
    for i in 0..8 {
        sum += 2 * m[i];
        c[i] = m[i];
    }
    c[8] = (64 - sum) + m[8];
    c
}

/// `alf_filter_block1` + `alf_filter_block2` on one LCU of `plane`.
#[allow(clippy::too_many_arguments)]
fn alf_block(dst: &mut Plane, src: &Plane, px: i32, py: i32, w: i32, h: i32, c: &[i32; 9], top: bool, down: bool, bd: i32) {
    let max = (1 << bd) - 1;
    let start = if top { py - 4 } else { py };
    let end = if down { py + h - 4 } else { py + h };
    let last = end - start - 1;
    let s = |x: i32, y: i32| src.get_clamped(px + x, start + y);
    for y in 0..=last {
        let y1u = (y - 1).clamp(0, last);
        let y1d = (y + 1).clamp(0, last);
        let y2u = (y - 2).clamp(0, last);
        let y2d = (y + 2).clamp(0, last);
        let y3u = (y - 3).clamp(0, last);
        let y3d = (y + 3).clamp(0, last);
        for x in 0..w {
            let mut v = c[0] * (s(x, y3d) + s(x, y3u));
            v += c[1] * (s(x, y2d) + s(x, y2u));
            v += c[2] * (s(x + 1, y1d) + s(x - 1, y1u));
            v += c[3] * (s(x, y1d) + s(x, y1u));
            v += c[4] * (s(x - 1, y1d) + s(x + 1, y1u));
            v += c[7] * (s(x + 1, y) + s(x - 1, y));
            v += c[6] * (s(x + 2, y) + s(x - 2, y));
            v += c[5] * (s(x + 3, y) + s(x - 3, y));
            v += c[8] * s(x, y);
            dst.set(px + x, start + y, ((v + 32) >> 6).clamp(0, max));
        }
    }
    // corner fix-ups (alf_filter_block2)
    let corner = |x: i32, y: i32, first: bool, left: bool| -> Option<i32> {
        let p0 = s(x, y);
        let nb = if left { s(x - 1, y) } else { s(x + 1, y) };
        if p0 == nb {
            return None;
        }
        // rows: first line -> "down" rows real, "up" rows = current; last line -> the reverse
        let (r1, r2, r3, r4, r5, r6) = if first { (y + 1, y, y + 2, y, y + 3, y) } else { (y, y - 1, y, y - 2, y, y - 3) };
        let mut v = c[0] * (s(x, r5) + s(x, r6));
        v += c[1] * (s(x, r3) + s(x, r4));
        // coefficient 2 and 4 taps per corner, exactly as in davs2
        let (c2a, c2b, c4a, c4b) = match (first, left) {
            (true, true) => (s(x + 1, r1), s(x, r2), s(x - 1, r1), s(x + 1, r2)),
            (true, false) => (s(x + 1, r1), s(x - 1, r2), s(x - 1, r1), s(x, r2)),
            (false, true) => (s(x + 1, r1), s(x - 1, r2), s(x, r1), s(x + 1, r2)),
            (false, false) => (s(x, r1), s(x - 1, r2), s(x - 1, r1), s(x + 1, r2)),
        };
        v += c[2] * (c2a + c2b);
        v += c[3] * (s(x, r1) + s(x, r2));
        v += c[4] * (c4a + c4b);
        v += c[7] * (s(x + 1, y) + s(x - 1, y));
        v += c[6] * (s(x + 2, y) + s(x - 2, y));
        v += c[5] * (s(x + 3, y) + s(x - 3, y));
        v += c[8] * p0;
        Some(((v + 32) >> 6).clamp(0, max))
    };
    let pts = [(0, 0, true, true), (w - 1, 0, true, false), (0, last, false, true), (w - 1, last, false, false)];
    for (x, y, first, left) in pts {
        if let Some(v) = corner(x, y, first, left) {
            dst.set(px + x, start + y, v);
        }
    }
}

/// ALF over the whole picture (on a copy of the SAO output).
pub(crate) fn alf(st: &PicState, cross_loop_filter: bool, f: &mut Frame, params: &[AlfParam; 3], pic_on: [bool; 3], lcu_on: &[[bool; 3]]) {
    if !pic_on.iter().any(|&b| b) {
        return;
    }
    let src = f.clone();
    let lv = st.lcu_level;
    let size = 1 << lv;
    let w_lcu = (st.width + size - 1) >> lv;
    let h_lcu = (st.height + size - 1) >> lv;
    let qw = (w_lcu + 1) >> 2;
    let qh = (h_lcu + 1) >> 2;
    // luma filter index per variance region
    let p0 = &params[0];
    let mut var_tab = [0usize; 16];
    if p0.filters_per_group > 1 {
        for i in 1..16 {
            var_tab[i] = var_tab[i - 1] + usize::from(p0.filter_pattern.get(i).copied().unwrap_or(false));
        }
    }
    for ly in 0..h_lcu {
        let ry = if qh == 0 { 3 } else { (ly / qh).min(3) };
        for lx in 0..w_lcu {
            let rx = if qw == 0 { 3 } else { (lx / qw).min(3) };
            let region = REGION_TABLE[(ry * 4 + rx) as usize];
            let lcu_xy = ly * w_lcu + lx;
            let on = lcu_on.get(lcu_xy as usize).copied().unwrap_or([false; 3]);
            let px = lx << lv;
            let py = ly << lv;
            let w = if px + size > st.width { st.width - px } else { size };
            let h = if py + size > st.height { st.height - py } else { size };
            let mut top = ly > 0;
            let down = ly < h_lcu - 1;
            if !cross_loop_filter {
                let sxy = (py >> 3) * st.w_scu + (px >> 3);
                let cur = st.scu.get(sxy.max(0) as usize).map(|c| c.slice_nr);
                let above = if top { st.scu.get((sxy - st.w_scu).max(0) as usize).map(|c| c.slice_nr) } else { Some(-1) };
                top = cur == above;
            }
            if on[0] {
                let c = alf_coeffs(p0, var_tab[region]);
                alf_block(&mut f.planes[0], &src.planes[0], px, py, w, h, &c, top, down, st.bit_depth);
            }
            if !st.mono {
                for comp in 1..3 {
                    if on[comp] {
                        let c = alf_coeffs(&params[comp], 0);
                        alf_block(&mut f.planes[comp], &src.planes[comp], px >> 1, py >> 1, w >> 1, h >> 1, &c, top, down, st.bit_depth);
                    }
                }
            }
        }
    }
}
