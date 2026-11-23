// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from uavs3d 1.2 (commit 0e20d2c2), Copyright (c) 2018-2022 Peking University Shenzhen Graduate School,
// Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation, BSD 3-clause licence (docs/licensing.md).
//! The deblocking filter: edge marking while the CUs are parsed, then filtering of the picture.

use std::sync::Arc;

use crate::blocks::{SIZE_2NXHN, SIZE_HNX2N, SIZE_NXN};
use crate::core::{Core, Pic, REFI_INVALID, SCU_CBF, SCU_INTRA};
use crate::tables::QP_CHROMA_ADJUST;

const ALPHA: [u8; 64] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 5, 5, 6, 6, 7, 7, 8, 9, 10, 10, 11, 12, 13, 15, 16,
    17, 19, 21, 23, 25, 27, 29, 32, 35, 38, 41, 45, 49, 54, 59,
];
const BETA: [u8; 64] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 5, 5, 5, 6, 6, 7, 8, 8, 9, 10, 11, 12, 13, 14, 15,
    16, 17, 18, 19, 20, 21, 22, 23, 23, 24, 24, 25, 25, 26, 27,
];

const EDGE_LUMA: u8 = 1;
const EDGE_ALL: u8 = 3;

fn same_pic(a: &Option<Arc<Pic>>, b: &Option<Arc<Pic>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => Arc::ptr_eq(x, y),
        _ => false,
    }
}

impl Core {
    fn ref_pic_opt(&self, refi: i8, lidx: usize) -> Option<Arc<Pic>> {
        usize::try_from(refi).ok().and_then(|r| self.refp.get(r)).and_then(|r| r[lidx].pic.clone())
    }

    /// `skip_filter`: true when the edge between `scup` and `scup + offset` needs no filtering.
    fn skip_filter(&self, scup: i32, offset: i32) -> bool {
        let q = scup + offset;
        if self.scu(q) & (SCU_INTRA | SCU_CBF) != 0 {
            return false;
        }
        let rp = self.refi_at(scup);
        let rq = self.refi_at(q);
        let mp = self.mv_at(scup);
        let mq = self.mv_at(q);
        let far = |a: [i16; 2], b: [i16; 2]| (i32::from(a[0]) - i32::from(b[0])).abs() >= 4 || (i32::from(a[1]) - i32::from(b[1])).abs() >= 4;
        if rp == rq {
            for l in 0..2 {
                if rp[l] != REFI_INVALID && far(mp[l], mq[l]) {
                    return false;
                }
            }
            return true;
        }
        let p0 = if rp[0] != REFI_INVALID { self.ref_pic_opt(rp[0], 0) } else { None };
        let p1 = if rp[1] != REFI_INVALID { self.ref_pic_opt(rp[1], 1) } else { None };
        let q0 = if rq[0] != REFI_INVALID { self.ref_pic_opt(rq[0], 0) } else { None };
        let q1 = if rq[1] != REFI_INVALID { self.ref_pic_opt(rq[1], 1) } else { None };
        if same_pic(&p0, &q1) && same_pic(&p1, &q0) {
            if rp[0] != REFI_INVALID && far(mp[0], mq[1]) {
                return false;
            }
            if rp[1] != REFI_INVALID && far(mp[1], mq[0]) {
                return false;
            }
            return true;
        }
        false
    }

    fn or_edge(&mut self, scup: i32, v: u8) {
        let k = self.ix(scup);
        if let Some(e) = self.map_edge.get_mut(k) {
            *e |= v;
        }
    }

    fn edge_hor(&mut self, scu_x: i32, scu_y: i32, w: i32, forced: bool) {
        if scu_y & 1 != 0 {
            return;
        }
        for i in 0..w {
            let s = scu_y * self.i_scu + scu_x + i;
            if forced || !self.skip_filter(s, -self.i_scu) {
                self.or_edge(s, EDGE_LUMA << 2);
            }
        }
    }

    fn edge_ver(&mut self, scu_x: i32, scu_y: i32, h: i32, forced: bool) {
        if scu_x & 1 != 0 {
            return;
        }
        for j in 0..h {
            let s = (scu_y + j) * self.i_scu + scu_x;
            if forced || !self.skip_filter(s, -1) {
                self.or_edge(s, EDGE_LUMA);
            }
        }
    }

    /// `com_deblock_set_edge`: marks the edges of the CU just parsed.
    pub(crate) fn deblock_set_edge(&mut self) {
        let (sw, sh) = (self.cu_w >> 2, self.cu_h >> 2);
        let (sx, sy) = (self.cu_x >> 2, self.cu_y >> 2);
        let forced = self.scu(self.cu_scup) & (SCU_INTRA | SCU_CBF) != 0;
        match self.tb_part {
            SIZE_2NXHN => {
                self.edge_hor(sx, sy + sh / 4, sw, forced);
                self.edge_hor(sx, sy + sh / 2, sw, forced);
                self.edge_hor(sx, sy + sh / 4 * 3, sw, forced);
            }
            SIZE_HNX2N => {
                self.edge_ver(sx + sw / 4, sy, sh, forced);
                self.edge_ver(sx + sw / 2, sy, sh, forced);
                self.edge_ver(sx + sw / 4 * 3, sy, sh, forced);
            }
            SIZE_NXN => {
                self.edge_hor(sx, sy + sh / 2, sw, forced);
                self.edge_ver(sx + sw / 2, sy, sh, forced);
            }
            _ => {}
        }
        let w_lcu = self.seq.w_lcu as usize;
        let lcu_idx = self.lcu_y as usize * w_lcu + self.lcu_x as usize;
        let patch = |s: &Core, i: usize| s.map_patch.get(i).copied();
        let cross = self.seq.cross_patch_loop_filter;
        if sy != 0 && sy & 1 == 0 && (self.cu_y != self.lcu_pix_y || cross || (lcu_idx >= w_lcu && patch(self, lcu_idx) == patch(self, lcu_idx - w_lcu))) {
            for i in 0..sw {
                let s = self.cu_scup + i;
                if forced || !self.skip_filter(s, -self.i_scu) {
                    self.or_edge(s, EDGE_ALL << 2);
                }
            }
        }
        if sx != 0 && sx & 1 == 0 && (self.cu_x != self.lcu_pix_x || cross || (lcu_idx >= 1 && patch(self, lcu_idx) == patch(self, lcu_idx - 1))) {
            for j in 0..sh {
                let s = self.cu_scup + j * self.i_scu;
                if forced || !self.skip_filter(s, -1) {
                    self.or_edge(s, EDGE_ALL);
                }
            }
        }
    }

    fn df_params(&self, qp_p: i32, qp_q: i32, delta: Option<i32>) -> (i32, i32) {
        let off = self.seq.bit_depth_2_qp_offset;
        let shift = self.seq.bit_depth - 8;
        let qp = match delta {
            None => (qp_p + qp_q + 1 - 2 * off) >> 1,
            Some(d) => {
                let adj = |q: i32| {
                    let mut v = q + d - off;
                    if v >= 0 {
                        v = i32::from(QP_CHROMA_ADJUST[v.min(63) as usize]);
                    }
                    (v + off).max(0)
                };
                ((adj(qp_p) + adj(qp_q) + 1) >> 1) - off
            }
        };
        let a = ALPHA[(qp + self.ph.alpha_c_offset).clamp(0, 63) as usize];
        let b = BETA[(qp + self.ph.beta_offset).clamp(0, 63) as usize];
        (i32::from(a) << shift, i32::from(b) << shift)
    }

    fn lcu_qp(&self, px: i32, py: i32) -> i32 {
        let l = self.seq.log2_max_cu;
        let i = (py >> l) as usize * self.seq.w_lcu as usize + (px >> l) as usize;
        self.map_qp.get(i).copied().map_or(0, i32::from)
    }

    /// Filters the whole picture (`com_deblock_lcu_row` for every row).
    pub(crate) fn deblock_picture(&mut self) {
        let (w, h) = (self.seq.pic_width as i32, self.seq.pic_height as i32);
        let fix = self.ph.fixed_picture_qp;
        let (dcb, dcr) = (self.ph.chroma_qp_delta_cb, self.ph.chroma_qp_delta_cr);
        let lmax = self.seq.max_cu as i32;
        let mut y = 0;
        while y < h {
            let mut x = 0;
            while x < w {
                let xv = x + 8;
                // parameters of the vertical edge at xv and the horizontal edge at y
                let qp_p = if fix { self.lcu_qp(0, 0) } else { self.lcu_qp(x, y) };
                let qp_qv = if fix || xv >= w || xv % lmax != 0 { qp_p } else { self.lcu_qp(xv, y) };
                let qp_qh = if fix || y == 0 { qp_p } else { self.lcu_qp(x, y - 8) };
                let ver = (self.df_params(qp_p, qp_qv, None), self.df_params(qp_p, qp_qv, Some(dcb)), self.df_params(qp_p, qp_qv, Some(dcr)));
                let hor = (self.df_params(qp_p, qp_qh, None), self.df_params(qp_p, qp_qh, Some(dcb)), self.df_params(qp_p, qp_qh, Some(dcr)));
                let sx = x >> 2;
                let sy = y >> 2;
                let e = |s: &Core, dx: i32, dy: i32| -> u32 { u32::from(s.map_edge.get(s.ix((sy + dy) * s.i_scu + sx + dx)).copied().unwrap_or(0)) };
                let edge_v = (e(self, 2, 0) | (e(self, 2, 1) << 8)) & 0x0303;
                if edge_v != 0 {
                    self.df_luma(xv, y, true, ver.0, edge_v);
                    if xv % 16 == 0 && edge_v & 0x0202 != 0 {
                        self.df_chroma(xv >> 1, y >> 1, true, [ver.1, ver.2], edge_v);
                    }
                }
                let edge_h = ((e(self, 0, 0) | (e(self, 1, 0) << 8)) >> 2) & 0x0303;
                if edge_h != 0 {
                    self.df_luma(x, y, false, hor.0, edge_h);
                    if y % 16 == 0 && edge_h & 0x0202 != 0 {
                        self.df_chroma(x >> 1, y >> 1, false, [hor.1, hor.2], edge_h);
                    }
                }
                x += 8;
            }
            y += 8;
        }
    }

    fn df_luma(&mut self, x: i32, y: i32, vertical: bool, (alpha, beta): (i32, i32), edge: u32) {
        let mut lines = 8;
        if edge & 0x0101 != 0x0101 {
            lines = 4;
        }
        let skip = if edge & 1 == 0 { 4 } else { 0 };
        let Some(pl) = self.cur.planes.first_mut() else {
            return;
        };
        let stride = pl.w;
        let (step, line_step, base) =
            if vertical { (1usize, stride, (y + skip) as usize * stride + x as usize) } else { (stride, 1usize, y as usize * stride + (x + skip) as usize) };
        for i in 0..lines {
            filter_luma_line(&mut pl.data, base + i * line_step, step, alpha, beta);
        }
    }

    fn df_chroma(&mut self, cx: i32, cy: i32, vertical: bool, ab: [(i32, i32); 2], edge: u32) {
        let mut lines = 4;
        if edge & 0x0202 != 0x0202 {
            lines = 2;
        }
        let skip = if edge & 2 == 0 { 2 } else { 0 };
        for (c, &(alpha, beta)) in ab.iter().enumerate() {
            let Some(pl) = self.cur.planes.get_mut(c + 1) else {
                continue;
            };
            let stride = pl.w;
            let (step, line_step, base) = if vertical {
                (1usize, stride, (cy + skip) as usize * stride + cx as usize)
            } else {
                (stride, 1usize, cy as usize * stride + (cx + skip) as usize)
            };
            for i in 0..lines {
                filter_chroma_line(&mut pl.data, base + i * line_step, step, alpha, beta);
            }
        }
    }
}

fn filter_luma_line(d: &mut [u16], r0: usize, st: usize, alpha: i32, beta: i32) {
    if r0 < 4 * st || r0 + 3 * st >= d.len() {
        return;
    }
    let g = |i: isize| i32::from(d[(r0 as isize + i * st as isize) as usize]);
    let (l3, l2, l1, l0) = (g(-4), g(-3), g(-2), g(-1));
    let (r0v, r1, r2, r3) = (g(0), g(1), g(2), g(3));
    let abs0 = (r0v - l0).abs();
    let mut flat_l = if (l1 - l0).abs() < beta { 2 } else { 0 };
    let mut flat_r = if (r0v - r1).abs() < beta { 2 } else { 0 };
    if (l2 - l0).abs() < beta {
        flat_l += 1;
    }
    if (r0v - r2).abs() < beta {
        flat_r += 1;
    }
    let fs = match flat_l + flat_r {
        6 => {
            if (r0v - r1).abs() <= beta / 4 && (l0 - l1).abs() <= beta / 4 && abs0 < alpha {
                4
            } else {
                3
            }
        }
        5 => {
            if r1 == r0v && l0 == l1 {
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
    let mut set = |i: isize, v: i32| d[(r0 as isize + i * st as isize) as usize] = v as u16;
    match fs {
        4 => {
            set(-3, (((l3 + l2 + l1) << 1) + l0 + r0v + 4) >> 3);
            set(-2, (((l2 + l0 + l1) << 2) + l1 + r0v * 3 + 8) >> 4);
            set(-1, ((l2 + r1) * 3 + ((l1 + l0 + r0v) << 3) + (l0 << 1) + 16) >> 5);
            set(0, ((r2 + l1) * 3 + ((r1 + r0v + l0) << 3) + (r0v << 1) + 16) >> 5);
            set(1, (((r2 + r1 + r0v) << 2) + r1 + l0 * 3 + 8) >> 4);
            set(2, (((r3 + r2 + r1) << 1) + r0v + l0 + 4) >> 3);
        }
        3 => {
            set(-2, (l2 * 3 + l1 * 8 + l0 * 4 + r0v + 8) >> 4);
            set(-1, (l2 + (l1 << 2) + (l0 << 2) + (l0 << 1) + (r0v << 2) + r1 + 8) >> 4);
            set(0, (l1 + (l0 << 2) + (r0v << 2) + (r0v << 1) + (r1 << 2) + r2 + 8) >> 4);
            set(1, (r2 * 3 + r1 * 8 + r0v * 4 + l0 + 8) >> 4);
        }
        2 => {
            set(-1, ((l1 << 1) + l1 + (l0 << 3) + (l0 << 1) + (r0v << 1) + r0v + 8) >> 4);
            set(0, ((l0 << 1) + l0 + (r0v << 3) + (r0v << 1) + (r1 << 1) + r1 + 8) >> 4);
        }
        1 => {
            set(-1, (l0 * 3 + r0v + 2) >> 2);
            set(0, (r0v * 3 + l0 + 2) >> 2);
        }
        _ => {}
    }
}

fn filter_chroma_line(d: &mut [u16], r0: usize, st: usize, alpha: i32, beta: i32) {
    if r0 < 3 * st || r0 + 2 * st >= d.len() {
        return;
    }
    let g = |i: isize| i32::from(d[(r0 as isize + i * st as isize) as usize]);
    let (l2, l1, l0) = (g(-3), g(-2), g(-1));
    let (r0v, r1, r2) = (g(0), g(1), g(2));
    let dm = (r0v - l0).abs();
    let dl = (l1 - l0).abs();
    let dr = (r0v - r1).abs();
    if dl < beta && dr < beta {
        let mut set = |i: isize, v: i32| d[(r0 as isize + i * st as isize) as usize] = v as u16;
        set(-1, (l1 * 3 + l0 * 10 + r0v * 3 + 8) >> 4);
        set(0, (r1 * 3 + r0v * 10 + l0 * 3 + 8) >> 4);
        if (l2 - l0).abs() < beta && (r2 - r0v).abs() < beta && dr <= beta / 4 && dl <= beta / 4 && dm < alpha {
            set(-2, (l2 * 3 + l1 * 8 + l0 * 3 + r0v * 2 + 8) >> 4);
            set(1, (r2 * 3 + r1 * 8 + r0v * 3 + l0 * 2 + 8) >> 4);
        }
    }
}
