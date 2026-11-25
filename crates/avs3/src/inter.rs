// SPDX-License-Identifier: GPL-3.0-or-later
//! Inter prediction: sub-pel interpolation, bi-prediction averaging and affine motion compensation.
//!
//! Derived from uavs3d 1.2 (commit 0e20d2c2), Copyright (c) 2018-2022 Peking University
//! Shenzhen Graduate School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation
//! Corporation, BSD 3-clause licence (docs/licensing.md).

use std::sync::Arc;

use crate::core::*;
use crate::tables::{MC_CHROMA, MC_CHROMA_HP, MC_LUMA, MC_LUMA_HP};

/// Prediction samples of a CU (planar).
#[derive(Default)]
pub(crate) struct Pred {
    pub y: Vec<u16>,
    pub u: Vec<u16>,
    pub v: Vec<u16>,
}

#[derive(Clone, Copy)]
enum Taps<'a> {
    T8(&'a [i8; 8]),
    T4(&'a [i8; 4]),
}

impl Taps<'_> {
    fn slice(&self) -> &[i8] {
        match self {
            Taps::T8(t) => &t[..],
            Taps::T4(t) => &t[..],
        }
    }
}

/// Interpolates a `w` x `h` block whose integer position is `(xi, yi)` with fractional parts
/// selecting the filters. Positions are clamped to the picture like the padded reference of uavs3d.
#[allow(clippy::too_many_arguments)]
fn interp(plane: &Plane, xi: i32, yi: i32, fx: Option<Taps<'_>>, fy: Option<Taps<'_>>, w: usize, h: usize, max_val: i32, taps: usize) -> Vec<u16> {
    let before = (taps / 2 - 1) as i32;
    let after = (taps / 2) as i32;
    let mut out = vec![0u16; w * h];
    if fx.is_none() && fy.is_none() {
        for r in 0..h {
            for c in 0..w {
                out[r * w + c] = plane.get(xi + c as i32, yi + r as i32) as u16;
            }
        }
        return out;
    }
    let (wb, hb) = (w + taps - 1, h + taps - 1);
    let mut win = vec![0i32; wb * hb];
    for r in 0..hb {
        for c in 0..wb {
            win[r * wb + c] = plane.get(xi - before + c as i32, yi - before + r as i32);
        }
    }
    let _ = after;
    let bd10 = max_val != 255;
    match (fx, fy) {
        (Some(cx), None) => {
            let cx = cx.slice();
            for r in 0..h {
                for c in 0..w {
                    let mut s = 0i32;
                    for (k, &co) in cx.iter().enumerate() {
                        s += win[(r + before as usize) * wb + c + k] * i32::from(co);
                    }
                    out[r * w + c] = ((s + 32) >> 6).clamp(0, max_val) as u16;
                }
            }
        }
        (None, Some(cy)) => {
            let cy = cy.slice();
            for r in 0..h {
                for c in 0..w {
                    let mut s = 0i32;
                    for (k, &co) in cy.iter().enumerate() {
                        s += win[(r + k) * wb + c + before as usize] * i32::from(co);
                    }
                    out[r * w + c] = ((s + 32) >> 6).clamp(0, max_val) as u16;
                }
            }
        }
        (Some(cx), Some(cy)) => {
            let (cx, cy) = (cx.slice(), cy.slice());
            let (shift1, shift2) = if bd10 { (2, 10) } else { (0, 12) };
            let add1 = (1 << shift1) >> 1;
            let add2 = 1 << (shift2 - 1);
            let mut tmp = vec![0i32; w * hb];
            for r in 0..hb {
                for c in 0..w {
                    let mut s = 0i32;
                    for (k, &co) in cx.iter().enumerate() {
                        s += win[r * wb + c + k] * i32::from(co);
                    }
                    tmp[r * w + c] = (s + add1) >> shift1;
                }
            }
            for r in 0..h {
                for c in 0..w {
                    let mut s = 0i32;
                    for (k, &co) in cy.iter().enumerate() {
                        s += tmp[(r + k) * w + c] * i32::from(co);
                    }
                    out[r * w + c] = ((s + add2) >> shift2).clamp(0, max_val) as u16;
                }
            }
        }
        (None, None) => {}
    }
    out
}

fn avg(a: &[u16], b: &[u16]) -> Vec<u16> {
    a.iter().zip(b).map(|(&x, &y)| ((u32::from(x) + u32::from(y) + 1) >> 1) as u16).collect()
}

impl Core {
    fn gray(&self, n: usize) -> Vec<u16> {
        vec![1u16 << (self.seq.bit_depth - 1); n]
    }

    #[allow(clippy::too_many_arguments)]
    fn mc_luma(&self, pic: Option<&Arc<Pic>>, pos_x: i32, pos_y: i32, w: usize, h: usize, hp: bool) -> Vec<u16> {
        let Some(pic) = pic else { return self.gray(w * h) };
        let max_val = (1 << self.seq.bit_depth) - 1;
        let (max_px, max_py) = (self.seq.pic_width as i32 + 4, self.seq.pic_height as i32 + 4);
        let (dx, dy, mut x, mut y) = if hp {
            ((pos_x & 15) as usize, (pos_y & 15) as usize, pos_x >> 4, pos_y >> 4)
        } else {
            ((pos_x & 3) as usize, (pos_y & 3) as usize, pos_x >> 2, pos_y >> 2)
        };
        x = x.clamp(-128 - 4, max_px);
        y = y.clamp(-128 - 4, max_py);
        let coef = |d: usize| -> Option<Taps<'static>> {
            if d == 0 {
                None
            } else if hp {
                MC_LUMA_HP.get(d).map(Taps::T8)
            } else {
                MC_LUMA.get(d).map(Taps::T8)
            }
        };
        interp(&pic.planes[0], x, y, coef(dx), coef(dy), w, h, max_val, 8)
    }

    #[allow(clippy::too_many_arguments)]
    fn mc_chroma(&self, pic: Option<&Arc<Pic>>, pos_x: i32, pos_y: i32, w: usize, h: usize, hp: bool, plane: usize) -> Vec<u16> {
        let Some(pic) = pic else { return self.gray(w * h) };
        let max_val = (1 << self.seq.bit_depth) - 1;
        let (max_px, max_py) = ((self.seq.pic_width as i32 + 4) >> 1, (self.seq.pic_height as i32 + 4) >> 1);
        let (dx, dy, mut x, mut y) = if hp {
            ((pos_x & 31) as usize, (pos_y & 31) as usize, pos_x >> 5, pos_y >> 5)
        } else {
            ((pos_x & 7) as usize, (pos_y & 7) as usize, pos_x >> 3, pos_y >> 3)
        };
        x = x.clamp(-64 - 2, max_px);
        y = y.clamp(-64 - 2, max_py);
        let coef = |d: usize| -> Option<Taps<'static>> {
            if d == 0 {
                None
            } else if hp {
                MC_CHROMA_HP.get(d).map(Taps::T4)
            } else {
                MC_CHROMA.get(d).map(Taps::T4)
            }
        };
        interp(&pic.planes[plane], x, y, coef(dx), coef(dy), w, h, max_val, 4)
    }

    fn ref_pic(&self, refi: i8, lidx: usize) -> Option<&Arc<Pic>> {
        usize::try_from(refi).ok().and_then(|r| self.refp.get(r)).and_then(|r| r[lidx].pic.as_ref())
    }

    /// Whole-sample / sub-pel motion compensation of the current (non-affine) CU.
    pub(crate) fn mc(&self) -> Pred {
        let (x, y, w, h) = (self.cu_x, self.cu_y, self.cu_w as usize, self.cu_h as usize);
        let base = if self.refi[0] >= 0 { 0 } else { 1 };
        let bi = self.refi[0] >= 0
            && self.refi[1] >= 0
            && (match (self.ref_pic(self.refi[0], 0), self.ref_pic(self.refi[1], 1)) {
                (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                _ => true,
            } || self.mv[0] != self.mv[1]);
        let mut pred = Pred::default();
        let lists: Vec<usize> = if bi { vec![base, 1 - base] } else { vec![base] };
        if self.tree_status != TREE_C {
            let mut bufs = Vec::new();
            for &l in &lists {
                let p = self.ref_pic(self.refi[l], l);
                bufs.push(self.mc_luma(p, (x << 2) + i32::from(self.mv[l][0]), (y << 2) + i32::from(self.mv[l][1]), w, h, false));
            }
            pred.y = if bi { avg(&bufs[0], &bufs[1]) } else { bufs.swap_remove(0) };
        }
        if self.tree_status != TREE_L {
            for pl in 1..3 {
                let mut bufs = Vec::new();
                for &l in &lists {
                    let p = self.ref_pic(self.refi[l], l);
                    bufs.push(self.mc_chroma(p, (x << 2) + i32::from(self.mv[l][0]), (y << 2) + i32::from(self.mv[l][1]), w / 2, h / 2, false, pl));
                }
                let r = if bi { avg(&bufs[0], &bufs[1]) } else { bufs.swap_remove(0) };
                if pl == 1 {
                    pred.u = r;
                } else {
                    pred.v = r;
                }
            }
        }
        pred
    }

    /// Affine motion compensation of the current CU.
    pub(crate) fn mc_affine(&self) -> Pred {
        let (x, y, w, h) = (self.cu_x, self.cu_y, self.cu_w as usize, self.cu_h as usize);
        let bi = self.refi[0] >= 0 && self.refi[1] >= 0;
        let base = if self.refi[0] >= 0 { 0 } else { 1 };
        let sub = if self.ph.affine_subblock_size_idx == 1 || bi { 8usize } else { 4 };
        let lists: Vec<usize> = if bi { vec![base, 1 - base] } else { vec![base] };
        let mut pred = Pred::default();
        let mut ybufs = Vec::new();
        let mut ubufs = Vec::new();
        let mut vbufs = Vec::new();
        for &l in &lists {
            let pic = self.ref_pic(self.refi[l], l);
            let sb = |idx: usize| -> [i32; 2] { self.affine_sb_mv.get(idx).map_or([0, 0], |m| if bi { m[l] } else { m[0] }) };
            // luma
            let mut yb = vec![0u16; w * h];
            let per_row = w / sub;
            let mut idx = 0usize;
            let mut sy = 0usize;
            while sy < h {
                let mut sx = 0usize;
                while sx < w {
                    let m = sb(idx);
                    idx += 1;
                    let blk = self.mc_luma(pic, ((x + sx as i32) << 4) + m[0], ((y + sy as i32) << 4) + m[1], sub, sub, true);
                    for r in 0..sub {
                        for c in 0..sub {
                            yb[(sy + r) * w + sx + c] = blk[r * sub + c];
                        }
                    }
                    sx += sub;
                }
                sy += sub;
            }
            ybufs.push(yb);
            // chroma: 4x4 blocks per 8x8 luma
            let (cw, ch) = (w / 2, h / 2);
            for pl in 1..3 {
                let mut cb = vec![0u16; cw * ch];
                let mut sy = 0usize;
                while sy < h {
                    let mut sx = 0usize;
                    while sx < w {
                        let (mx, my) = if sub == 4 {
                            let i0 = (sy / 4) * per_row + sx / 4;
                            let a = sb(i0);
                            let b = sb(i0 + 1);
                            let c = sb(i0 + per_row);
                            let d = sb(i0 + per_row + 1);
                            ((a[0] + b[0] + c[0] + d[0] + 2) >> 2, (a[1] + b[1] + c[1] + d[1] + 2) >> 2)
                        } else {
                            let m = sb((sy / 8) * per_row + sx / 8);
                            (m[0], m[1])
                        };
                        let blk = self.mc_chroma(pic, ((x + sx as i32) << 4) + mx, ((y + sy as i32) << 4) + my, 4, 4, true, pl);
                        for r in 0..4 {
                            for c in 0..4 {
                                if let Some(d) = cb.get_mut((sy / 2 + r) * cw + sx / 2 + c) {
                                    *d = blk[r * 4 + c];
                                }
                            }
                        }
                        sx += 8;
                    }
                    sy += 8;
                }
                if pl == 1 {
                    ubufs.push(cb);
                } else {
                    vbufs.push(cb);
                }
            }
        }
        let fin = |mut v: Vec<Vec<u16>>| if bi { avg(&v[0], &v[1]) } else { v.swap_remove(0) };
        pred.y = fin(ybufs);
        pred.u = fin(ubufs);
        pred.v = fin(vbufs);
        pred
    }
}
