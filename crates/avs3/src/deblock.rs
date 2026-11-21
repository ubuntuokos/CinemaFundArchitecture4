// SPDX-License-Identifier: BSD-3-Clause
// Derived from uavs3d (`source/decore/deblock.c`), Copyright (c) 2018-2022 Peking University Shenzhen
// Graduate School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation. See LICENSE.uavs3d.
//! The deblocking filter: edge marking during the CU decode and the picture-level filter pass.
//!
//! uavs3d filters each LCU row in raster order of the 8x8 cells (vertical edge, then horizontal edge).
//! Every pixel is changed by all vertical edges before any horizontal edge touches it, so a pass over
//! all vertical edges followed by a pass over all horizontal edges gives the same picture.

use std::sync::Arc;

use crate::core::{Core, Picture};
use crate::types::*;

const ALPHA: [u8; 64] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 5, 5, 6, 6, 7, 7, 8, 9, 10, 10, 11, 12, 13, 15, 16,
    17, 19, 21, 23, 25, 27, 29, 32, 35, 38, 41, 45, 49, 54, 59,
];
const BETA: [u8; 64] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 5, 5, 5, 6, 6, 7, 8, 8, 9, 10, 11, 12, 13, 14, 15,
    16, 17, 18, 19, 20, 21, 22, 23, 23, 24, 24, 25, 25, 26, 27,
];
pub(crate) const QP_CHROMA_ADJUST: [u8; 64] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40,
    41, 42, 42, 43, 43, 44, 44, 45, 45, 46, 46, 47, 47, 48, 48, 48, 49, 49, 49, 50, 50, 50, 51,
];

const EDGE_LUMA: u8 = 1;
const EDGE_ALL: u8 = 3;

fn pic_ptr(r: Option<&crate::core::RefPic>) -> Option<*const Picture> {
    r.map(|r| Arc::as_ptr(&r.pic))
}

impl Core {
    fn ref_pic_ptr(&self, refi: i8, lidx: usize) -> Option<*const Picture> {
        if refi < 0 {
            return None;
        }
        pic_ptr(self.refp.get(refi as usize).and_then(|r| r[lidx].as_ref()))
    }

    /// `skip_filter`: true when the edge between `scup` and `scup + offset` needs no filtering.
    fn skip_filter(&self, refi_p: [i8; 2], p0: Option<*const Picture>, p1: Option<*const Picture>, scup: i32, offset: i32) -> bool {
        let m = &self.maps;
        let q = m.scu[m.ix(scup + offset)];
        if q.intra || q.cbf {
            return false;
        }
        let refi_q = m.refi[m.ix(scup + offset)];
        let mv_p = m.mv[m.ix(scup)];
        let mv_q = m.mv[m.ix(scup + offset)];
        let far = |a: Mv, b: Mv| (i32::from(a[0]) - i32::from(b[0])).abs() >= 4 || (i32::from(a[1]) - i32::from(b[1])).abs() >= 4;
        if refi_p == refi_q {
            for l in 0..2 {
                if refi_p[l] >= 0 && far(mv_p[l], mv_q[l]) {
                    return false;
                }
            }
            return true;
        }
        let q0 = self.ref_pic_ptr(refi_q[0], 0);
        let q1 = self.ref_pic_ptr(refi_q[1], 1);
        if p0 == q1 && p1 == q0 {
            if refi_p[0] >= 0 && far(mv_p[0], mv_q[1]) {
                return false;
            }
            if refi_p[1] >= 0 && far(mv_p[1], mv_q[0]) {
                return false;
            }
            return true;
        }
        false
    }

    fn set_edge_ver_inline(&mut self, scu_x: usize, scu_y: usize, h: usize, cond: u8) {
        let i_scu = self.seq.i_scu as i32;
        let mut scup = (scu_y * self.seq.i_scu + scu_x) as i32;
        let refi_p = self.maps.refi[self.maps.ix(scup)];
        let p0 = self.ref_pic_ptr(refi_p[0], 0);
        let p1 = self.ref_pic_ptr(refi_p[1], 1);
        for _ in 0..h {
            let skip = self.skip_filter(refi_p, p0, p1, scup, -1);
            let ix = self.maps.ix(scup);
            self.maps.edge[ix] |= if skip { 0 } else { cond };
            scup += i_scu;
        }
    }

    fn set_edge_hor_inline(&mut self, scu_x: usize, scu_y: usize, w: usize, cond: u8) {
        let i_scu = self.seq.i_scu as i32;
        let first = (scu_y * self.seq.i_scu + scu_x) as i32;
        let refi_p = self.maps.refi[self.maps.ix(first)];
        let p0 = self.ref_pic_ptr(refi_p[0], 0);
        let p1 = self.ref_pic_ptr(refi_p[1], 1);
        for scup in (first..).take(w) {
            let skip = self.skip_filter(refi_p, p0, p1, scup, -i_scu);
            let ix = self.maps.ix(scup);
            self.maps.edge[ix] |= if skip { 0 } else { cond << 2 };
        }
    }

    fn set_edge_ver_force(&mut self, scu_x: usize, scu_y: usize, h: usize, cond: u8) {
        if scu_x & 1 == 0 {
            for j in 0..h {
                let ix = self.maps.ix(((scu_y + j) * self.seq.i_scu + scu_x) as i32);
                self.maps.edge[ix] |= cond;
            }
        }
    }

    fn set_edge_hor_force(&mut self, scu_x: usize, scu_y: usize, w: usize, cond: u8) {
        if scu_y & 1 == 0 {
            for i in 0..w {
                let ix = self.maps.ix((scu_y * self.seq.i_scu + scu_x + i) as i32);
                self.maps.edge[ix] |= cond << 2;
            }
        }
    }

    /// `com_deblock_set_edge`
    pub(crate) fn deblock_set_edge(&mut self) {
        let seq_w = self.seq.pic_width_in_lcu;
        let scup = self.cu_scup;
        let lcu_idx = self.lcu_y * seq_w + self.lcu_x;
        let scu_w = self.cu_width >> 2;
        let scu_h = self.cu_height >> 2;
        let scu_x = self.cu_pix_x >> 2;
        let scu_y = self.cu_pix_y >> 2;
        let i_scu = self.seq.i_scu;
        let scu = self.maps.scu[self.maps.ix(scup)];
        let forced = scu.intra || scu.cbf;
        let tb_part = self.tb_part;
        let patch = |s: &Self, a: usize, b: usize| s.maps.patch[a] == s.maps.patch[b];

        let hor = |s: &mut Self, y: usize| {
            if forced {
                s.set_edge_hor_force(scu_x, y, scu_w, EDGE_LUMA);
            } else if y & 1 == 0 {
                s.set_edge_hor_inline(scu_x, y, scu_w, EDGE_LUMA);
            }
        };
        match tb_part {
            SIZE_2NXHN => {
                hor(self, scu_y + scu_h / 4);
                hor(self, scu_y + scu_h / 2);
                hor(self, scu_y + scu_h / 4 * 3);
            }
            SIZE_HNX2N => {
                for x in [scu_x + scu_w / 4, scu_x + scu_w / 2, scu_x + scu_w / 4 * 3] {
                    if forced {
                        self.set_edge_ver_force(x, scu_y, scu_h, EDGE_LUMA);
                    } else if x & 1 == 0 {
                        self.set_edge_ver_inline(x, scu_y, scu_h, EDGE_LUMA);
                    }
                }
            }
            SIZE_NXN => {
                hor(self, scu_y + scu_h / 2);
                let x = scu_x + scu_w / 2;
                if forced {
                    self.set_edge_ver_force(x, scu_y, scu_h, EDGE_LUMA);
                } else if x & 1 == 0 {
                    self.set_edge_ver_inline(x, scu_y, scu_h, EDGE_LUMA);
                }
            }
            _ => {}
        }
        let cross = self.seq.cross_patch_loop_filter;
        if scu_y != 0 && scu_y & 1 == 0 && (self.cu_pix_y != self.lcu_pix_y || cross || patch(self, lcu_idx, lcu_idx - seq_w)) {
            if forced {
                for i in 0..scu_w {
                    let ix = self.maps.ix(scup + i as i32);
                    self.maps.edge[ix] |= EDGE_ALL << 2;
                }
            } else {
                self.set_edge_hor_inline(scu_x, scu_y, scu_w, EDGE_ALL);
            }
        }
        if scu_x != 0 && scu_x & 1 == 0 && (self.cu_pix_x != self.lcu_pix_x || cross || patch(self, lcu_idx, lcu_idx - 1)) {
            if forced {
                for j in 0..scu_h {
                    let ix = self.maps.ix(scup + (j * i_scu) as i32);
                    self.maps.edge[ix] |= EDGE_ALL;
                }
            } else {
                self.set_edge_ver_inline(scu_x, scu_y, scu_h, EDGE_ALL);
            }
        }
    }

    fn deblock_params(&self, qp_p: i32, qp_q: i32) -> [(i32, i32); 3] {
        let ph = &self.pichdr;
        let shift = self.seq.bit_depth_internal - 8;
        let qo = self.seq.bit_depth_2_qp_offset;
        let ab = |qp: i32| {
            (i32::from(ALPHA[clip3(0, 63, qp + ph.alpha_c_offset) as usize]) << shift, i32::from(BETA[clip3(0, 63, qp + ph.beta_offset) as usize]) << shift)
        };
        let y = ab((qp_p + qp_q + 1 - qo - qo) >> 1);
        let c = |delta: i32| {
            let mut p = qp_p + delta - qo;
            let mut q = qp_q + delta - qo;
            if p >= 0 {
                p = i32::from(QP_CHROMA_ADJUST[p.min(63) as usize]);
            }
            if q >= 0 {
                q = i32::from(QP_CHROMA_ADJUST[q.min(63) as usize]);
            }
            p = (p + qo).max(0);
            q = (q + qo).max(0);
            ab(((p + q + 1) >> 1) - qo)
        };
        [y, c(ph.chroma_quant_param_delta_cb), c(ph.chroma_quant_param_delta_cr)]
    }

    fn lcu_qp_at(&self, x: usize, y: usize) -> i32 {
        let l = self.seq.log2_max_cuwh;
        i32::from(self.maps.qp[(y >> l) * self.seq.pic_width_in_lcu + (x >> l)])
    }

    /// The deblocking filter of the whole picture.
    pub(crate) fn deblock_picture(&mut self) {
        let (w, h) = (self.seq.pic_width, self.seq.pic_height);
        let i_scu = self.seq.i_scu;
        let edge_at = |s: &Self, sx: usize, sy: usize| s.maps.edge[s.maps.ix((sy * i_scu + sx) as i32)];
        // vertical edges
        for y in (0..h).step_by(8) {
            for x in (8..w).step_by(8) {
                let e0 = edge_at(self, x / 4, y / 4);
                let e1 = edge_at(self, x / 4, y / 4 + 1);
                if (e0 | e1) & 3 == 0 {
                    continue;
                }
                let prm = self.deblock_params(self.lcu_qp_at(x - 1, y), self.lcu_qp_at(x, y));
                let st = self.pic.planes[0].w;
                let n = usize::from(e0 & 1 != 0) * 4 + usize::from(e1 & 1 != 0) * 4;
                let start = if e0 & 1 != 0 { y } else { y + 4 };
                let pl = &mut self.pic.planes[0];
                for r in 0..n {
                    luma_line(&mut pl.data, (start + r) * st + x, 1, prm[0], self.seq.bit_depth_internal);
                }
                if x % 16 == 0 && ((e0 | e1) & 2 != 0) {
                    let n = usize::from(e0 & 2 != 0) * 2 + usize::from(e1 & 2 != 0) * 2;
                    let start = if e0 & 2 != 0 { y / 2 } else { y / 2 + 2 };
                    for c in 1..3 {
                        let pl = &mut self.pic.planes[c];
                        let st = pl.w;
                        for r in 0..n {
                            chroma_line(&mut pl.data, (start + r) * st + x / 2, 1, prm[c]);
                        }
                    }
                }
            }
        }
        // horizontal edges
        for y in (8..h).step_by(8) {
            for x in (0..w).step_by(8) {
                let e0 = edge_at(self, x / 4, y / 4) >> 2;
                let e1 = edge_at(self, x / 4 + 1, y / 4) >> 2;
                if (e0 | e1) & 3 == 0 {
                    continue;
                }
                let prm = self.deblock_params(self.lcu_qp_at(x, y - 1), self.lcu_qp_at(x, y));
                let st = self.pic.planes[0].w;
                let n = usize::from(e0 & 1 != 0) * 4 + usize::from(e1 & 1 != 0) * 4;
                let start = if e0 & 1 != 0 { x } else { x + 4 };
                let pl = &mut self.pic.planes[0];
                for c in 0..n {
                    luma_line(&mut pl.data, y * st + start + c, st as isize, prm[0], self.seq.bit_depth_internal);
                }
                if y % 16 == 0 && ((e0 | e1) & 2 != 0) {
                    let n = usize::from(e0 & 2 != 0) * 2 + usize::from(e1 & 2 != 0) * 2;
                    let start = if e0 & 2 != 0 { x / 2 } else { x / 2 + 2 };
                    for c in 1..3 {
                        let pl = &mut self.pic.planes[c];
                        let st = pl.w as isize;
                        for r in 0..n {
                            chroma_line(&mut pl.data, (y / 2) * pl.w + start + r, st, prm[c]);
                        }
                    }
                }
            }
        }
    }
}

/// `deblock_edge_luma_*` for one line across the edge; `pos` is R0, `a` the step across the edge.
fn luma_line(d: &mut [u16], pos: usize, a: isize, (alpha, beta): (i32, i32), _bd: u32) {
    let at = |d: &[u16], k: isize| i32::from(d[(pos as isize + k * a) as usize]);
    let (l3, l2, l1, l0) = (at(d, -4), at(d, -3), at(d, -2), at(d, -1));
    let (r0, r1, r2, r3) = (at(d, 0), at(d, 1), at(d, 2), at(d, 3));
    let abs0 = (r0 - l0).abs();
    let mut flat_l = if (l1 - l0).abs() < beta { 2 } else { 0 };
    let mut flat_r = if (r0 - r1).abs() < beta { 2 } else { 0 };
    if (l2 - l0).abs() < beta {
        flat_l += 1;
    }
    if (r0 - r2).abs() < beta {
        flat_r += 1;
    }
    let fs = match flat_l + flat_r {
        6 => {
            if (r0 - r1).abs() <= beta / 4 && (l0 - l1).abs() <= beta / 4 && abs0 < alpha {
                4
            } else {
                3
            }
        }
        5 => {
            if r1 == r0 && l0 == l1 {
                3
            } else {
                2
            }
        }
        4 => {
            if flat_l == 2 {
                2
            } else {
                1
            }
        }
        3 => i32::from((l1 - r1).abs() < beta),
        _ => 0,
    };
    let mut set = |k: isize, v: i32| d[(pos as isize + k * a) as usize] = v as u16;
    match fs {
        4 => {
            set(-3, (((l3 + l2 + l1) << 1) + l0 + r0 + 4) >> 3);
            set(-2, (((l2 + l0 + l1) << 2) + l1 + r0 * 3 + 8) >> 4);
            set(-1, ((l2 + r1) * 3 + ((l1 + l0 + r0) << 3) + (l0 << 1) + 16) >> 5);
            set(0, ((r2 + l1) * 3 + ((r1 + r0 + l0) << 3) + (r0 << 1) + 16) >> 5);
            set(1, (((r2 + r1 + r0) << 2) + r1 + l0 * 3 + 8) >> 4);
            set(2, (((r3 + r2 + r1) << 1) + r0 + l0 + 4) >> 3);
        }
        3 => {
            set(-2, (l2 * 3 + l1 * 8 + l0 * 4 + r0 + 8) >> 4);
            set(-1, (l2 + (l1 << 2) + (l0 << 2) + (l0 << 1) + (r0 << 2) + r1 + 8) >> 4);
            set(0, (l1 + (l0 << 2) + (r0 << 2) + (r0 << 1) + (r1 << 2) + r2 + 8) >> 4);
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

/// `deblock_edge_chro_*` for one line of one component.
fn chroma_line(d: &mut [u16], pos: usize, a: isize, (alpha, beta): (i32, i32)) {
    let at = |d: &[u16], k: isize| i32::from(d[(pos as isize + k * a) as usize]);
    let (l2, l1, l0) = (at(d, -3), at(d, -2), at(d, -1));
    let (r0, r1, r2) = (at(d, 0), at(d, 1), at(d, 2));
    let delta_m = (r0 - l0).abs();
    let delta_l = (l1 - l0).abs();
    let delta_r = (r0 - r1).abs();
    if delta_l < beta && delta_r < beta {
        let mut set = |k: isize, v: i32| d[(pos as isize + k * a) as usize] = v as u16;
        set(-1, (l1 * 3 + l0 * 10 + r0 * 3 + 8) >> 4);
        set(0, (r1 * 3 + r0 * 10 + l0 * 3 + 8) >> 4);
        if (l2 - l0).abs() < beta && (r2 - r0).abs() < beta && delta_r <= beta / 4 && delta_l <= beta / 4 && delta_m < alpha {
            set(-2, (l2 * 3 + l1 * 8 + l0 * 3 + r0 * 2 + 8) >> 4);
            set(1, (r2 * 3 + r1 * 8 + r0 * 3 + l0 * 2 + 8) >> 4);
        }
    }
}
