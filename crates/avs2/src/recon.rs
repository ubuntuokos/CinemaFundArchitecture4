// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from davs2 1.7 (`source/common/intra.cc`, `transform.cc`, `mc.cc`, `cu.cc`), Copyright (C) 2018~ VCL, NELVT, Peking
// University (Falei Luo et al.), GPL-2.0-or-later.
//! CU reconstruction: intra prediction, inverse transforms and motion compensation
//! (davs2 `intra.cc`, `transform.cc`, `mc.cc`, `cu_recon`).

use crate::Error;
use crate::cu::*;
use crate::header::FrameType;

/// One picture plane (samples of `bit_depth` bits).
#[derive(Clone, Debug)]
pub(crate) struct Plane {
    pub w: i32,
    pub h: i32,
    pub data: Vec<u16>,
}

impl Plane {
    pub(crate) fn new(w: i32, h: i32, val: u16) -> Self {
        Plane { w, h, data: vec![val; (w.max(0) as usize) * (h.max(0) as usize)] }
    }

    #[inline]
    fn idx(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return None;
        }
        Some((y * self.w + x) as usize)
    }

    #[inline]
    pub(crate) fn get(&self, x: i32, y: i32) -> i32 {
        self.idx(x, y).and_then(|i| self.data.get(i)).map_or(0, |&v| i32::from(v))
    }

    /// The sample at (`x`, `y`) with the picture edges replicated (davs2's padded planes).
    #[inline]
    pub(crate) fn get_clamped(&self, x: i32, y: i32) -> i32 {
        self.get(x.clamp(0, self.w - 1), y.clamp(0, self.h - 1))
    }

    #[inline]
    pub(crate) fn set(&mut self, x: i32, y: i32, v: i32) {
        if let Some(p) = self.idx(x, y).and_then(|i| self.data.get_mut(i)) {
            *p = v as u16;
        }
    }
}

/// A decoded picture: Y, U, V at the coded size (multiples of 8).
#[derive(Clone, Debug)]
pub(crate) struct Frame {
    pub planes: [Plane; 3],
}

impl Frame {
    pub(crate) fn new(w: i32, h: i32, bit_depth: i32) -> Self {
        let mid = 1u16 << (bit_depth - 1);
        Frame { planes: [Plane::new(w, h, mid), Plane::new(w / 2, h / 2, mid), Plane::new(w / 2, h / 2, mid)] }
    }
}

// ---------------------------------------------------------------------------------------------
// intra prediction

const DIR_DXDY: [[[i32; 2]; 33]; 2] = [
    [
        [0, 0],
        [0, 0],
        [0, 0],
        [11, 2],
        [2, 0],
        [11, 3],
        [1, 0],
        [93, 7],
        [1, 1],
        [93, 8],
        [1, 2],
        [1, 3],
        [0, 0],
        [1, 3],
        [1, 2],
        [93, 8],
        [1, 1],
        [93, 7],
        [1, 0],
        [11, 3],
        [2, 0],
        [11, 2],
        [4, 0],
        [8, 0],
        [0, 0],
        [8, 0],
        [4, 0],
        [11, 2],
        [2, 0],
        [11, 3],
        [1, 0],
        [93, 7],
        [1, 1],
    ],
    [
        [0, 0],
        [0, 0],
        [0, 0],
        [93, 8],
        [1, 1],
        [93, 7],
        [1, 0],
        [11, 3],
        [2, 0],
        [11, 2],
        [4, 0],
        [8, 0],
        [0, 0],
        [8, 0],
        [4, 0],
        [11, 2],
        [2, 0],
        [11, 3],
        [1, 0],
        [93, 7],
        [1, 1],
        [93, 8],
        [1, 2],
        [1, 3],
        [0, 0],
        [1, 3],
        [1, 2],
        [93, 8],
        [1, 1],
        [93, 7],
        [1, 0],
        [11, 3],
        [2, 0],
    ],
];

const AVAIL_LEFT: u32 = 1;
const AVAIL_TOP: u32 = 2;
const AVAIL_TOP_LEFT: u32 = 4;
const AVAIL_TOP_RIGHT: u32 = 8;
const AVAIL_LEFT_DOWN: u32 = 16;

/// The reference sample line around a block (`EP`): index 0 is the top-left sample, positive
/// indices run along the top row, negative ones down the left column.
struct Edge {
    buf: [i32; EDGE_LEN],
}

const EDGE_CENTER: i32 = 320;
const EDGE_LEN: usize = 640;

impl Edge {
    #[inline]
    fn get(&self, i: i32) -> i32 {
        self.buf.get((EDGE_CENTER + i).max(0) as usize).copied().unwrap_or(0)
    }

    #[inline]
    fn set(&mut self, i: i32, v: i32) {
        if let Some(p) = self.buf.get_mut((EDGE_CENTER + i).max(0) as usize) {
            *p = v;
        }
    }

    fn repeat(&mut self, from: i32, val: i32, n: i32) {
        for k in 0..n.max(0) {
            self.set(from + k, val);
        }
    }
}

fn log2(v: i32) -> i32 {
    31 - (v.max(1) as u32).leading_zeros() as i32
}

fn context_pixel(mode: usize, xy: usize, d: i32) -> (i32, i32) {
    let [mult, shift] = DIR_DXDY.get(xy).and_then(|t| t.get(mode)).copied().unwrap_or([0, 0]);
    let dn = (d * mult) >> shift;
    let off = ((d * mult * 32) >> shift) - dn * 32;
    (dn, off)
}

/// `intra_pred`: predicts a `bsx`×`bsy` block from the edge samples into `dst` (row-major).
fn intra_pred(ep: &Edge, dst: &mut [i32], mode: i32, bsx: i32, bsy: i32, avail: u32, bit_depth: i32) {
    let max = (1 << bit_depth) - 1;
    let w = bsx as usize;
    let mut put = |x: i32, y: i32, v: i32| {
        if let Some(p) = dst.get_mut(y as usize * w + x as usize) {
            *p = v;
        }
    };
    match mode {
        0 => {
            let b_top = avail & AVAIL_TOP != 0;
            let b_left = avail & AVAIL_LEFT != 0;
            let mut dc = 0;
            if b_left {
                for y in 0..bsy {
                    dc += ep.get(-1 - y);
                }
                if b_top {
                    for x in 0..bsx {
                        dc += ep.get(1 + x);
                    }
                    dc += (bsx + bsy) >> 1;
                    dc = (dc * (512 / (bsx + bsy))) >> 9;
                } else {
                    dc += bsy / 2;
                    dc /= bsy;
                }
            } else if b_top {
                for x in 0..bsx {
                    dc += ep.get(1 + x);
                }
                dc += bsx / 2;
                dc /= bsx;
            } else {
                dc = 1 << (bit_depth - 1);
            }
            let dc = dc.clamp(0, max);
            for y in 0..bsy {
                for x in 0..bsx {
                    put(x, y, dc);
                }
            }
        }
        1 => {
            const MULT: [i32; 5] = [13, 17, 5, 11, 23];
            const SHIFT: [i32; 5] = [7, 10, 11, 15, 19];
            let lw = (log2(bsx) - 2).clamp(0, 4) as usize;
            let lh = (log2(bsy) - 2).clamp(0, 4) as usize;
            let (im_h, is_h) = (MULT[lw], SHIFT[lw]);
            let (im_v, is_v) = (MULT[lh], SHIFT[lh]);
            let w2 = bsx >> 1;
            let h2 = bsy >> 1;
            let mut ih = 0;
            let mut iv = 0;
            // p_src = src + 1 + (w2 - 1)
            let base = w2;
            for x in 1..=w2 {
                ih += x * (ep.get(base + x) - ep.get(base - x));
            }
            // p_src = src - 1 - (h2 - 1)
            let base = -h2;
            for y in 1..=h2 {
                iv += y * (ep.get(base - y) - ep.get(base + y));
            }
            let ia = (ep.get(-bsy) + ep.get(bsx)) << 4;
            let ib = ((ih << 5) * im_h + (1 << (is_h - 1))) >> is_h;
            let ic = ((iv << 5) * im_v + (1 << (is_v - 1))) >> is_v;
            let mut t = ia - (h2 - 1) * ic - (w2 - 1) * ib + 16;
            for y in 0..bsy {
                let mut t2 = t;
                for x in 0..bsx {
                    put(x, y, (t2 >> 5).clamp(0, max));
                    t2 += ib;
                }
                t += ic;
            }
        }
        2 => {
            let sx = log2(bsx);
            let sy = log2(bsy);
            let s = sx.min(sy);
            let sxy = sx + sy + 1;
            let offset = 1 << (sx + sy);
            let mut top: Vec<i32> = (0..bsx).map(|x| ep.get(1 + x)).collect();
            let mut left: Vec<i32> = (0..bsy).map(|y| ep.get(-1 - y)).collect();
            let a = top.last().copied().unwrap_or(0);
            let b = left.last().copied().unwrap_or(0);
            let c = if bsx == bsy { (a + b + 1) >> 1 } else { (((a << sx) + (b << sy)) * 13 + (1 << (s + 5))) >> (s + 6) };
            let w = (c << 1) - a - b;
            let mut pt = vec![0i32; bsx as usize];
            for (t, p) in top.iter_mut().zip(pt.iter_mut()) {
                *p = b - *t;
                *t <<= sy;
            }
            let mut pl = vec![0i32; bsy as usize];
            let mut wy = vec![0i32; bsy as usize];
            let mut acc = 0;
            for ((l, p), wv) in left.iter_mut().zip(pl.iter_mut()).zip(wy.iter_mut()) {
                *p = a - *l;
                *l <<= sx;
                *wv = acc;
                acc += w;
            }
            for y in 0..bsy as usize {
                let mut predx = left.get(y).copied().unwrap_or(0);
                let ply = pl.get(y).copied().unwrap_or(0);
                let wyy = wy.get(y).copied().unwrap_or(0);
                let mut wxy = -wyy;
                for x in 0..bsx as usize {
                    predx += ply;
                    wxy += wyy;
                    let (Some(t), Some(d)) = (top.get_mut(x), pt.get(x)) else { continue };
                    *t += *d;
                    let v = ((predx << sy) + (*t << sx) + wxy + offset) >> sxy;
                    put(x as i32, y as i32, v.clamp(0, max));
                }
            }
        }
        12 => {
            for y in 0..bsy {
                for x in 0..bsx {
                    put(x, y, ep.get(1 + x));
                }
            }
        }
        24 => {
            for y in 0..bsy {
                for x in 0..bsx {
                    put(x, y, ep.get(-1 - y));
                }
            }
        }
        3..=11 => {
            let m = mode as usize;
            for j in 0..bsy {
                let (mut ix, c4) = context_pixel(m, 0, j + 1);
                let (c1, c2, c3) = (32 - c4, 64 - c4, 32 + c4);
                for i in 0..bsx {
                    let v = (ep.get(ix) * c1 + ep.get(ix + 1) * c2 + ep.get(ix + 2) * c3 + ep.get(ix + 3) * c4 + 64) >> 7;
                    put(i, j, v);
                    ix += 1;
                }
            }
        }
        25..=32 => {
            let m = mode as usize;
            let steps: Vec<(i32, i32)> = (0..bsx).map(|i| context_pixel(m, 1, i + 1)).collect();
            for j in 0..bsy {
                for (i, &(xs, off)) in steps.iter().enumerate() {
                    let idx = -(j + xs);
                    let v = (ep.get(idx) * (32 - off) + ep.get(idx - 1) * (64 - off) + ep.get(idx - 2) * (32 + off) + ep.get(idx - 3) * off + 64) >> 7;
                    put(i as i32, j, v);
                }
            }
        }
        13..=23 => {
            let m = mode as usize;
            let steps: Vec<(i32, i32)> = (0..bsx).map(|i| context_pixel(m, 1, i + 1)).collect();
            for j in 0..bsy {
                let (dn, offx) = context_pixel(m, 0, j + 1);
                for (i, &(xs, offy)) in steps.iter().enumerate() {
                    let ixx = i as i32 - dn;
                    let iyy = j - xs;
                    let v = if iyy <= -1 {
                        (ep.get(ixx + 2) * (32 - offx) + ep.get(ixx + 1) * (64 - offx) + ep.get(ixx) * (32 + offx) + ep.get(ixx - 1) * offx + 64) >> 7
                    } else {
                        (ep.get(-iyy - 2) * (32 - offy) + ep.get(-iyy - 1) * (64 - offy) + ep.get(-iyy) * (32 + offy) + ep.get(-iyy + 1) * offy + 64) >> 7
                    };
                    put(i as i32, j, v);
                }
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------------------------
// inverse transforms

/// `g_T32`: the 32-point integer DCT basis (row i = frequency).
const T32: [[i16; 32]; 32] = [
    [32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32],
    [45, 45, 44, 43, 41, 39, 36, 34, 30, 27, 23, 19, 15, 11, 7, 2, -2, -7, -11, -15, -19, -23, -27, -30, -34, -36, -39, -41, -43, -44, -45, -45],
    [45, 43, 40, 35, 29, 21, 13, 4, -4, -13, -21, -29, -35, -40, -43, -45, -45, -43, -40, -35, -29, -21, -13, -4, 4, 13, 21, 29, 35, 40, 43, 45],
    [45, 41, 34, 23, 11, -2, -15, -27, -36, -43, -45, -44, -39, -30, -19, -7, 7, 19, 30, 39, 44, 45, 43, 36, 27, 15, 2, -11, -23, -34, -41, -45],
    [44, 38, 25, 9, -9, -25, -38, -44, -44, -38, -25, -9, 9, 25, 38, 44, 44, 38, 25, 9, -9, -25, -38, -44, -44, -38, -25, -9, 9, 25, 38, 44],
    [44, 34, 15, -7, -27, -41, -45, -39, -23, -2, 19, 36, 45, 43, 30, 11, -11, -30, -43, -45, -36, -19, 2, 23, 39, 45, 41, 27, 7, -15, -34, -44],
    [43, 29, 4, -21, -40, -45, -35, -13, 13, 35, 45, 40, 21, -4, -29, -43, -43, -29, -4, 21, 40, 45, 35, 13, -13, -35, -45, -40, -21, 4, 29, 43],
    [43, 23, -7, -34, -45, -36, -11, 19, 41, 44, 27, -2, -30, -45, -39, -15, 15, 39, 45, 30, 2, -27, -44, -41, -19, 11, 36, 45, 34, 7, -23, -43],
    [42, 17, -17, -42, -42, -17, 17, 42, 42, 17, -17, -42, -42, -17, 17, 42, 42, 17, -17, -42, -42, -17, 17, 42, 42, 17, -17, -42, -42, -17, 17, 42],
    [41, 11, -27, -45, -30, 7, 39, 43, 15, -23, -45, -34, 2, 36, 44, 19, -19, -44, -36, -2, 34, 45, 23, -15, -43, -39, -7, 30, 45, 27, -11, -41],
    [40, 4, -35, -43, -13, 29, 45, 21, -21, -45, -29, 13, 43, 35, -4, -40, -40, -4, 35, 43, 13, -29, -45, -21, 21, 45, 29, -13, -43, -35, 4, 40],
    [39, -2, -41, -36, 7, 43, 34, -11, -44, -30, 15, 45, 27, -19, -45, -23, 23, 45, 19, -27, -45, -15, 30, 44, 11, -34, -43, -7, 36, 41, 2, -39],
    [38, -9, -44, -25, 25, 44, 9, -38, -38, 9, 44, 25, -25, -44, -9, 38, 38, -9, -44, -25, 25, 44, 9, -38, -38, 9, 44, 25, -25, -44, -9, 38],
    [36, -15, -45, -11, 39, 34, -19, -45, -7, 41, 30, -23, -44, -2, 43, 27, -27, -43, 2, 44, 23, -30, -41, 7, 45, 19, -34, -39, 11, 45, 15, -36],
    [35, -21, -43, 4, 45, 13, -40, -29, 29, 40, -13, -45, -4, 43, 21, -35, -35, 21, 43, -4, -45, -13, 40, 29, -29, -40, 13, 45, 4, -43, -21, 35],
    [34, -27, -39, 19, 43, -11, -45, 2, 45, 7, -44, -15, 41, 23, -36, -30, 30, 36, -23, -41, 15, 44, -7, -45, -2, 45, 11, -43, -19, 39, 27, -34],
    [32, -32, -32, 32, 32, -32, -32, 32, 32, -32, -32, 32, 32, -32, -32, 32, 32, -32, -32, 32, 32, -32, -32, 32, 32, -32, -32, 32, 32, -32, -32, 32],
    [30, -36, -23, 41, 15, -44, -7, 45, -2, -45, 11, 43, -19, -39, 27, 34, -34, -27, 39, 19, -43, -11, 45, 2, -45, 7, 44, -15, -41, 23, 36, -30],
    [29, -40, -13, 45, -4, -43, 21, 35, -35, -21, 43, 4, -45, 13, 40, -29, -29, 40, 13, -45, 4, 43, -21, -35, 35, 21, -43, -4, 45, -13, -40, 29],
    [27, -43, -2, 44, -23, -30, 41, 7, -45, 19, 34, -39, -11, 45, -15, -36, 36, 15, -45, 11, 39, -34, -19, 45, -7, -41, 30, 23, -44, 2, 43, -27],
    [25, -44, 9, 38, -38, -9, 44, -25, -25, 44, -9, -38, 38, 9, -44, 25, 25, -44, 9, 38, -38, -9, 44, -25, -25, 44, -9, -38, 38, 9, -44, 25],
    [23, -45, 19, 27, -45, 15, 30, -44, 11, 34, -43, 7, 36, -41, 2, 39, -39, -2, 41, -36, -7, 43, -34, -11, 44, -30, -15, 45, -27, -19, 45, -23],
    [21, -45, 29, 13, -43, 35, 4, -40, 40, -4, -35, 43, -13, -29, 45, -21, -21, 45, -29, -13, 43, -35, -4, 40, -40, 4, 35, -43, 13, 29, -45, 21],
    [19, -44, 36, -2, -34, 45, -23, -15, 43, -39, 7, 30, -45, 27, 11, -41, 41, -11, -27, 45, -30, -7, 39, -43, 15, 23, -45, 34, 2, -36, 44, -19],
    [17, -42, 42, -17, -17, 42, -42, 17, 17, -42, 42, -17, -17, 42, -42, 17, 17, -42, 42, -17, -17, 42, -42, 17, 17, -42, 42, -17, -17, 42, -42, 17],
    [15, -39, 45, -30, 2, 27, -44, 41, -19, -11, 36, -45, 34, -7, -23, 43, -43, 23, 7, -34, 45, -36, 11, 19, -41, 44, -27, -2, 30, -45, 39, -15],
    [13, -35, 45, -40, 21, 4, -29, 43, -43, 29, -4, -21, 40, -45, 35, -13, -13, 35, -45, 40, -21, -4, 29, -43, 43, -29, 4, 21, -40, 45, -35, 13],
    [11, -30, 43, -45, 36, -19, -2, 23, -39, 45, -41, 27, -7, -15, 34, -44, 44, -34, 15, 7, -27, 41, -45, 39, -23, 2, 19, -36, 45, -43, 30, -11],
    [9, -25, 38, -44, 44, -38, 25, -9, -9, 25, -38, 44, -44, 38, -25, 9, 9, -25, 38, -44, 44, -38, 25, -9, -9, 25, -38, 44, -44, 38, -25, 9],
    [7, -19, 30, -39, 44, -45, 43, -36, 27, -15, 2, 11, -23, 34, -41, 45, -45, 41, -34, 23, -11, -2, 15, -27, 36, -43, 45, -44, 39, -30, 19, -7],
    [4, -13, 21, -29, 35, -40, 43, -45, 45, -43, 40, -35, 29, -21, 13, -4, -4, 13, -21, 29, -35, 40, -43, 45, -45, 43, -40, 35, -29, 21, -13, 4],
    [2, -7, 11, -15, 19, -23, 27, -30, 34, -36, 39, -41, 43, -44, 45, -45, 45, -45, 44, -43, 41, -39, 36, -34, 30, -27, 23, -19, 15, -11, 7, -2],
];

const T2: [i32; 16] = [123, -35, -8, -3, -32, -120, 30, 10, 14, 25, 123, -22, 8, 13, 19, 126];
const T2_C: [i32; 16] = [34, 58, 72, 81, 77, 69, -7, -75, 79, -33, -75, 58, 55, -84, 73, -28];

#[inline]
fn basis(n: usize, i: usize, k: usize) -> i32 {
    let step = 32 / n.max(1);
    T32.get(i * step).and_then(|r| r.get(k)).map_or(0, |&v| i32::from(v))
}

/// `partialButterflyInverseN`: `src` holds `n`-point columns spaced `line` apart; writes `line`
/// rows of `n` samples to `dst`.
fn butterfly(src: &[i32], dst: &mut [i32], n: usize, line: usize, shift: i32, clip_depth: i32) {
    let max = (1i32 << (clip_depth - 1)) - 1;
    let min = -max - 1;
    let add = 1i32 << (shift - 1);
    for j in 0..line {
        for k in 0..n {
            let mut sum = 0i32;
            for i in 0..n {
                sum = sum.wrapping_add(basis(n, i, k).wrapping_mul(src.get(i * line + j).copied().unwrap_or(0)));
            }
            if let Some(d) = dst.get_mut(j * n + k) {
                *d = (sum.wrapping_add(add) >> shift).clamp(min, max);
            }
        }
    }
}

/// The inverse DCT of a `w`×`h` block (row-major `src`) into `dst`; `a_flag` marks the half-size
/// transform inside a wavelet (one more bit of range).
fn idct(src: &[i32], dst: &mut [i32], w: usize, h: usize, bit_depth: i32, a_flag: bool) {
    let a = i32::from(a_flag);
    let mut tmp = vec![0i32; w * h];
    // stage 1: vertical (columns of height h, `w` lines)
    butterfly(src, &mut tmp, h, w, 5, 16);
    // stage 2: horizontal
    butterfly(&tmp, dst, w, h, 20 - bit_depth - a, bit_depth + 1 + a);
}

/// The 5/3 wavelet synthesis that doubles a `w`×`h` block to `2w`×`2h`.
fn wavelet(src: &[i32], w: usize, h: usize) -> Vec<i32> {
    let (w2, h2) = (w * 2, h * 2);
    let mut out = vec![0i32; w2 * h2];
    let mut ext = vec![0i32; w2.max(h2) + 1];
    // vertical
    for x in 0..w {
        for y in 0..h {
            if let Some(e) = ext.get_mut(y << 1) {
                *e = src.get(y * w + x).copied().unwrap_or(0);
            }
        }
        let refl = ext.get(h2 - 2).copied().unwrap_or(0);
        if let Some(e) = ext.get_mut(h2) {
            *e = refl;
        }
        for y in (0..=h2).step_by(2) {
            if let Some(e) = ext.get_mut(y) {
                *e >>= 1;
            }
        }
        for y in (1..h2).step_by(2) {
            let v = (ext.get(y - 1).copied().unwrap_or(0) + ext.get(y + 1).copied().unwrap_or(0)) >> 1;
            if let Some(e) = ext.get_mut(y) {
                *e = v;
            }
        }
        for y in 0..h2 {
            if let Some(o) = out.get_mut(y * w2 + x) {
                *o = i32::from(ext.get(y).copied().unwrap_or(0) as i16);
            }
        }
    }
    // horizontal
    for y in 0..h2 {
        for x in 0..w {
            let v = out.get(y * w2 + x).copied().unwrap_or(0);
            if let Some(e) = ext.get_mut(x << 1) {
                *e = v;
            }
        }
        let refl = ext.get(w2 - 2).copied().unwrap_or(0);
        if let Some(e) = ext.get_mut(w2) {
            *e = refl;
        }
        for x in (1..w2).step_by(2) {
            let v = (ext.get(x - 1).copied().unwrap_or(0) + ext.get(x + 1).copied().unwrap_or(0)) >> 1;
            if let Some(e) = ext.get_mut(x) {
                *e = v;
            }
        }
        for x in 0..w2 {
            if let Some(o) = out.get_mut(y * w2 + x) {
                *o = i32::from(ext.get(x).copied().unwrap_or(0) as i16);
            }
        }
    }
    out
}

/// `xTr2nd_4_1d_Inv_Ver` on the top-left 4×4 of a block with row stride `stride`.
fn tr2nd_ver(c: &mut [i32], stride: usize, shift: i32, tc: &[i32; 16]) {
    let mut t = [0i32; 16];
    for i in 0..4 {
        for j in 0..4 {
            t[i * 4 + j] = c.get(i * stride + j).copied().unwrap_or(0);
        }
    }
    let add = 1 << (shift - 1);
    for i in 0..4 {
        for j in 0..4 {
            let mut sum = add;
            for k in 0..4 {
                sum += tc[k * 4 + i] * t[k * 4 + j];
            }
            if let Some(p) = c.get_mut(i * stride + j) {
                *p = (sum >> shift).clamp(-32768, 32767);
            }
        }
    }
}

/// `xTr2nd_4_1d_Inv_Hor`.
fn tr2nd_hor(c: &mut [i32], stride: usize, shift: i32, clip_depth: i32, tc: &[i32; 16]) {
    let mut t = [0i32; 16];
    for i in 0..4 {
        for j in 0..4 {
            t[i * 4 + j] = c.get(i * stride + j).copied().unwrap_or(0);
        }
    }
    let max = (1 << (clip_depth - 1)) - 1;
    let min = -max - 1;
    let add = 1 << (shift - 1);
    for i in 0..4 {
        for j in 0..4 {
            let mut sum = add;
            for k in 0..4 {
                sum += tc[k * 4 + i] * t[j * 4 + k];
            }
            if let Some(p) = c.get_mut(j * stride + i) {
                *p = (sum >> shift).clamp(min, max);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// motion compensation

const LUMA_FILTER: [[i32; 8]; 4] =
    [[0, 0, 0, 64, 0, 0, 0, 0], [-1, 4, -10, 57, 19, -7, 3, -1], [-1, 4, -11, 40, 40, -11, 4, -1], [-1, 3, -7, 19, 57, -10, 4, -1]];
const CHROMA_FILTER: [[i32; 4]; 8] =
    [[0, 64, 0, 0], [-4, 62, 6, 0], [-6, 56, 15, -1], [-5, 47, 25, -3], [-4, 36, 36, -4], [-3, 25, 47, -5], [-1, 15, 56, -6], [0, 6, 62, -4]];

/// Interpolates a `w`×`h` block at the sub-sample position (`posx`, `posy`) in units of
/// 1/`1 << frac_bits` sample. Luma: 8 taps, quarter samples; chroma: 4 taps, eighth samples.
fn interpolate(rf: &Plane, out: &mut [i32], posx: i32, posy: i32, w: i32, h: i32, luma: bool, bit_depth: i32) {
    let max = (1 << bit_depth) - 1;
    let (frac_bits, taps, first): (i32, usize, i32) = if luma { (2, 8, -3) } else { (3, 4, -1) };
    let mask = (1 << frac_bits) - 1;
    let dx = (posx & mask) as usize;
    let dy = (posy & mask) as usize;
    let ix = posx >> frac_bits;
    let iy = posy >> frac_bits;
    let coef = |d: usize, k: usize| -> i32 {
        if luma { LUMA_FILTER.get(d).and_then(|f| f.get(k)).copied().unwrap_or(0) } else { CHROMA_FILTER.get(d).and_then(|f| f.get(k)).copied().unwrap_or(0) }
    };
    let wu = w as usize;
    let mut put = |x: i32, y: i32, v: i32| {
        if let Some(p) = out.get_mut(y as usize * wu + x as usize) {
            *p = v;
        }
    };
    if dx == 0 && dy == 0 {
        for y in 0..h {
            for x in 0..w {
                put(x, y, rf.get_clamped(ix + x, iy + y));
            }
        }
    } else if dx == 0 {
        for y in 0..h {
            for x in 0..w {
                let mut s = 0;
                for k in 0..taps {
                    s += coef(dy, k) * rf.get_clamped(ix + x, iy + y + first + k as i32);
                }
                put(x, y, ((s + 32) >> 6).clamp(0, max));
            }
        }
    } else if dy == 0 {
        for y in 0..h {
            for x in 0..w {
                let mut s = 0;
                for k in 0..taps {
                    s += coef(dx, k) * rf.get_clamped(ix + x + first + k as i32, iy + y);
                }
                put(x, y, ((s + 32) >> 6).clamp(0, max));
            }
        }
    } else {
        let shift1 = bit_depth - 8;
        let add1 = (1 << shift1) >> 1;
        let shift2 = 20 - bit_depth;
        let add2 = 1 << (shift2 - 1);
        let rows = h + taps as i32 - 1;
        let mut tmp = vec![0i32; (rows.max(0) as usize) * wu];
        for r in 0..rows {
            let y = iy + first + r;
            for x in 0..w {
                let mut s = 0;
                for k in 0..taps {
                    s += coef(dx, k) * rf.get_clamped(ix + x + first + k as i32, y);
                }
                let mut v = (s + add1) >> shift1;
                if luma {
                    v = i32::from(v as i16); // mct_t
                }
                if let Some(p) = tmp.get_mut(r as usize * wu + x as usize) {
                    *p = v;
                }
            }
        }
        for y in 0..h {
            for x in 0..w {
                let mut s = 0;
                for k in 0..taps {
                    s += coef(dy, k) * tmp.get((y as usize + k) * wu + x as usize).copied().unwrap_or(0);
                }
                put(x, y, ((s + add2) >> shift2).clamp(0, max));
            }
        }
    }
}

/// `cu_get_mc_pos`: the clamped quarter-sample position of a block.
fn mc_pos(img: i32, blk: i32, pos: i32, mv: i32) -> i32 {
    let imv = mv >> 2;
    let fmv = mv & 7;
    let v = if pos + imv < -blk - 8 {
        ((-blk - 8) << 2) + fmv
    } else if pos + imv > img + 4 {
        ((img + 4) << 2) + fmv
    } else {
        (pos << 2) + mv
    };
    i32::from(v as i16)
}

// ---------------------------------------------------------------------------------------------
// CU reconstruction

/// The reconstruction context of one picture.
pub(crate) struct Recon<'a> {
    pub st: &'a PicState,
    pub frame: &'a mut Frame,
    /// The reference pictures by reference index (B pictures: 0 = backward, 1 = forward).
    pub refs: &'a [&'a Frame],
    pub second_transform: bool,
    pub coeff_y: &'a mut [i16],
    pub coeff_uv: [&'a mut [i16]; 2],
    avail_top: bool,
    avail_left: bool,
}

impl<'a> Recon<'a> {
    pub(crate) fn new(
        st: &'a PicState,
        frame: &'a mut Frame,
        refs: &'a [&'a Frame],
        second_transform: bool,
        coeff_y: &'a mut [i16],
        coeff_uv: [&'a mut [i16]; 2],
    ) -> Self {
        Recon { st, frame, refs, second_transform, coeff_y, coeff_uv, avail_top: false, avail_left: false }
    }

    /// `decode_lcu_recon`.
    pub(crate) fn recon_lcu(&mut self, level: i32, pix_x: i32, pix_y: i32) -> Result<(), Error> {
        let st = self.st;
        let cx = pix_x >> 3;
        let cy = pix_y >> 3;
        let cu = *st.scu.get((cy * st.w_scu + cx).max(0) as usize).ok_or(Error::Invalid("CU position"))?;
        if cu.level < level {
            let next = level - 1;
            for i in 0..4 {
                let sx = pix_x + ((i & 1) << next);
                let sy = pix_y + ((i >> 1) << next);
                if sx < st.width && sy < st.height {
                    self.recon_lcu(next, sx, sy)?;
                }
            }
            return Ok(());
        }
        if cu.slice_nr == -1 {
            return Err(Error::Invalid("undecodable CU"));
        }
        let mask = (1 << (st.lcu_level - 3)) - 1;
        let z = TAB_B8XY_TO_ZIGZAG.get((cy & mask) as usize).and_then(|r| r.get((cx & mask) as usize)).copied().unwrap_or(0);
        self.cu_recon(&cu, z, pix_x, pix_y)
    }

    /// `get_intra_neighbors`.
    fn intra_neighbors(&self, x4: i32, y4: i32, bsx: i32, bsy: i32, slice: i32) -> u32 {
        let st = self.st;
        let avail = |dx: i32, dy: i32| -> bool {
            let x2 = x4 + dx;
            let y2 = y4 + dy;
            if x2 < 0 || y2 < 0 || x2 >= st.w_spu || y2 >= st.h_spu {
                return false;
            }
            st.scu.get(((y2 >> 1) * st.w_scu + (x2 >> 1)) as usize).is_some_and(|c| c.slice_nr == slice)
        };
        let left = avail(-1, 0);
        let top = avail(0, -1);
        let top_left = avail(-1, -1);
        let mut left_down = avail(-1, (bsy >> 1) - 1);
        let mut top_right = avail((bsx >> 1) - 1, -1);
        let lmask = (1 << (st.lcu_level - 2)) - 1;
        let xl = x4 & lmask;
        let yl = y4 & lmask;
        left_down = left_down && dl_avail(st.lcu_level, xl, yl + (bsy >> 2) - 1);
        top_right = top_right && tr_avail(st.lcu_level, xl + (bsx >> 2) - 1, yl);
        let mut a = 0;
        if left {
            a |= AVAIL_LEFT;
        }
        if top {
            a |= AVAIL_TOP;
        }
        if top_left {
            a |= AVAIL_TOP_LEFT;
        }
        if top_right {
            a |= AVAIL_TOP_RIGHT;
        }
        if left_down {
            a |= AVAIL_LEFT_DOWN;
        }
        a
    }

    /// `fill_reference_samples_*`: the edge of a block at (`px`, `py`) of `plane`.
    fn fill_edge(&self, plane: usize, px: i32, py: i32, avail: u32, bsx: i32, bsy: i32) -> Edge {
        let p = &self.frame.planes[plane.min(2)];
        let dc = 1 << (self.st.bit_depth - 1);
        let mut ep = Edge { buf: [0; EDGE_LEN] };
        ep.repeat(-(bsy << 1), dc, ((bsy + bsx) << 1) + 1);
        if avail & AVAIL_TOP != 0 {
            for x in 0..bsx {
                ep.set(1 + x, p.get(px + x, py - 1));
            }
        }
        if avail & AVAIL_TOP_RIGHT != 0 {
            for x in 0..bsx {
                ep.set(bsx + 1 + x, p.get(px + bsx + x, py - 1));
            }
        } else {
            let v = ep.get(bsx);
            ep.repeat(bsx + 1, v, bsx);
        }
        let pad = bsy * 11 / 4 - bsx + 4;
        if pad > 0 {
            let v = ep.get(2 * bsx);
            ep.repeat(2 * bsx + 1, v, pad);
        }
        if avail & AVAIL_LEFT != 0 {
            for y in 0..bsy {
                ep.set(-1 - y, p.get(px - 1, py + y));
            }
        }
        if avail & AVAIL_LEFT_DOWN != 0 {
            for y in 0..bsy {
                ep.set(-bsy - 1 - y, p.get(px - 1, py + bsy + y));
            }
        } else {
            let v = ep.get(-bsy);
            ep.repeat(-(bsy << 1), v, bsy);
        }
        if avail & AVAIL_TOP_LEFT != 0 {
            ep.set(0, p.get(px - 1, py - 1));
        } else if avail & AVAIL_TOP != 0 {
            ep.set(0, p.get(px, py - 1));
        } else if avail & AVAIL_LEFT != 0 {
            ep.set(0, p.get(px - 1, py));
        }
        let pad = bsx * 11 / 4 - bsy + 4;
        if pad > 0 {
            let v = ep.get(-2 * bsy);
            ep.repeat(-2 * bsy - pad, v, pad);
        }
        ep
    }

    fn write_block(&mut self, plane: usize, px: i32, py: i32, w: i32, h: i32, v: &[i32]) {
        let p = &mut self.frame.planes[plane.min(2)];
        for y in 0..h {
            for x in 0..w {
                if let Some(&s) = v.get((y * w + x) as usize) {
                    p.set(px + x, py + y, s);
                }
            }
        }
    }

    /// `davs2_get_intra_pred` (luma).
    fn intra_pred_luma(&mut self, cu: &Cu, mode: i8, px: i32, py: i32, bsx: i32, bsy: i32) {
        let avail = self.intra_neighbors(px >> 2, py >> 2, bsx, bsy, cu.slice_nr);
        self.avail_top = avail & AVAIL_TOP != 0;
        self.avail_left = avail & AVAIL_LEFT != 0;
        let ep = self.fill_edge(0, px, py, avail, bsx, bsy);
        let mut pred = vec![0i32; (bsx * bsy).max(0) as usize];
        intra_pred(&ep, &mut pred, i32::from(mode), bsx, bsy, avail, self.st.bit_depth);
        self.write_block(0, px, py, bsx, bsy, &pred);
    }

    /// `davs2_get_intra_pred_chroma`.
    fn intra_pred_chroma(&mut self, cu: &Cu, cpx: i32, cpy: i32) {
        const REAL: [i32; 5] = [0, 0, 24, 12, 2];
        let bs = 1 << (cu.level - 1);
        let luma_mode = i32::from(cu.intra_pred_modes[0]);
        let real = if cu.c_ipred_mode == 0 { luma_mode } else { REAL.get(usize::from(cu.c_ipred_mode)).copied().unwrap_or(0) };
        let avail = self.intra_neighbors((cpx << 1) >> 2, (cpy << 1) >> 2, bs << 1, bs << 1, cu.slice_nr);
        for plane in 1..3 {
            let ep = self.fill_edge(plane, cpx, cpy, avail, bs, bs);
            let mut pred = vec![0i32; (bs * bs) as usize];
            intra_pred(&ep, &mut pred, real, bs, bs, avail, self.st.bit_depth);
            self.write_block(plane, cpx, cpy, bs, bs, &pred);
        }
    }

    /// `davs2_get_recons`: inverse transform of block `blk` and addition to the prediction.
    fn recons(&mut self, cu: &Cu, z: usize, blk: usize, tu: Cb, px: i32, py: i32) -> Result<(), Error> {
        let luma = blk < 4;
        let wavelet = luma && cu.level == B64X64_IN_BIT && cu.trans_size != TU_SPLIT_CROSS;
        let (bsx, bsy) = (tu.w, tu.h);
        let (cw, ch) = if wavelet { (bsx >> 1, bsy >> 1) } else { (bsx, bsy) };
        let (cw, ch) = (cw.max(0) as usize, ch.max(0) as usize);
        let src: &[i16] = if luma {
            let off = (z << 6) + (blk << ((cu.level - 1) << 1));
            self.coeff_y.get(off..off + cw * ch).ok_or(Error::Invalid("coefficient buffer"))?
        } else {
            let off = z << 4;
            self.coeff_uv.get(blk - 4).and_then(|b| b.get(off..off + cw * ch)).ok_or(Error::Invalid("coefficient buffer"))?
        };
        let mut c: Vec<i32> = src.iter().map(|&v| i32::from(v)).collect();
        let bd = self.st.bit_depth;
        let sec = self.second_transform && cu.is_intra() && luma;
        let res = if bsx == 4 && bsy == 4 {
            if sec {
                tr2nd_ver(&mut c, 4, 5, &T2_C);
                tr2nd_hor(&mut c, 4, 20 - bd + 2, bd + 1, &T2_C);
                c
            } else {
                let mut out = vec![0i32; 16];
                idct(&c, &mut out, 4, 4, bd, false);
                out
            }
        } else {
            if sec {
                let mode = i32::from(cu.intra_pred_modes.get(blk).copied().unwrap_or(0));
                let vt = (0..=23).contains(&mode);
                let ht = (13..=32).contains(&mode) || (0..=2).contains(&mode);
                if ht && self.avail_left {
                    tr2nd_hor(&mut c, cw, 7, 16, &T2);
                }
                if vt && self.avail_top {
                    tr2nd_ver(&mut c, cw, 7, &T2);
                }
            }
            let mut out = vec![0i32; cw * ch];
            idct(&c, &mut out, cw, ch, bd, wavelet);
            if wavelet { wavelet_out(&out, cw, ch) } else { out }
        };
        let (plane, x0, y0) = if luma { (0, px + tu.x, py + tu.y) } else { (blk - 3, px >> 1, py >> 1) };
        let max = (1 << bd) - 1;
        let p = &mut self.frame.planes[plane.min(2)];
        for y in 0..bsy {
            for x in 0..bsx {
                let r = res.get((y * bsx + x) as usize).copied().unwrap_or(0);
                let v = (p.get(x0 + x, y0 + y) + r).clamp(0, max);
                p.set(x0 + x, y0 + y, v);
            }
        }
        Ok(())
    }

    /// `davs2_get_inter_pred`.
    fn inter_pred(&mut self, cu: &Cu, px: i32, py: i32) -> Result<(), Error> {
        let st = self.st;
        let b = st.frame_type == FrameType::B;
        for pu_idx in 0..cu.num_pu.min(4) {
            let pu = cu.pu[pu_idx];
            let (x, y, w, h) = (px + pu.x, py + pu.y, pu.w, pu.h);
            let pdir = cu.b8pdir[pu_idx];
            let mut mv1;
            let mut mv2 = Mv::default();
            let (r1, r2): (Option<usize>, Option<usize>);
            if pdir == PDIR_BWD {
                mv1 = cu.mv[pu_idx][1];
                r1 = Some(B_BWD as usize);
                r2 = None;
            } else if pdir == PDIR_SYM || pdir == PDIR_BID {
                mv1 = cu.mv[pu_idx][0];
                mv2 = cu.mv[pu_idx][1];
                r1 = Some(B_FWD as usize);
                r2 = Some(B_BWD as usize);
            } else {
                let ref1 = cu.ref_idx[pu_idx].r[0];
                mv1 = cu.mv[pu_idx][0];
                if b {
                    r1 = Some(B_FWD as usize);
                    r2 = None;
                } else if pdir == PDIR_DUAL {
                    mv2 = cu.mv[pu_idx][1];
                    r1 = usize::try_from(ref1).ok();
                    r2 = usize::try_from(cu.ref_idx[pu_idx].r[1]).ok();
                    if r2.is_none() {
                        return Err(Error::Invalid("missing reference"));
                    }
                } else if cu.dmh_mode != 0 {
                    let d = dmh_pos(cu.dmh_mode);
                    mv2.x = (i32::from(mv1.x) + d[1][0]) as i16;
                    mv2.y = (i32::from(mv1.y) + d[1][1]) as i16;
                    mv1.x = (i32::from(mv1.x) + d[0][0]) as i16;
                    mv1.y = (i32::from(mv1.y) + d[0][1]) as i16;
                    r1 = usize::try_from(ref1).ok();
                    r2 = r1;
                } else {
                    r1 = usize::try_from(ref1).ok();
                    r2 = None;
                }
            }
            let v1x = mc_pos(st.width, w, x, i32::from(mv1.x));
            let v1y = mc_pos(st.height, h, y, i32::from(mv1.y));
            let v2x = mc_pos(st.width, w, x, i32::from(mv2.x));
            let v2y = mc_pos(st.height, h, y, i32::from(mv2.y));
            let f1 = r1.and_then(|r| self.refs.get(r)).copied().ok_or(Error::Invalid("missing reference picture"))?;
            let f2 = match r2 {
                Some(r) => Some(self.refs.get(r).copied().ok_or(Error::Invalid("missing reference picture"))?),
                None => None,
            };
            let bd = st.bit_depth;
            let n = (w * h).max(0) as usize;
            let mut p1 = vec![0i32; n];
            interpolate(&f1.planes[0], &mut p1, v1x, v1y, w, h, true, bd);
            if let Some(f2) = f2 {
                let mut p2 = vec![0i32; n];
                interpolate(&f2.planes[0], &mut p2, v2x, v2y, w, h, true, bd);
                for (a, b) in p1.iter_mut().zip(p2.iter()) {
                    *a = (*a + *b + 1) >> 1;
                }
            }
            self.write_block(0, x, y, w, h, &p1);
            if !st.mono {
                let (cx, cy, cw, ch) = (x >> 1, y >> 1, w >> 1, h >> 1);
                let n = (cw * ch).max(0) as usize;
                for plane in 1..3 {
                    let mut p1 = vec![0i32; n];
                    interpolate(&f1.planes[plane], &mut p1, v1x, v1y, cw, ch, false, bd);
                    if let Some(f2) = f2 {
                        let mut p2 = vec![0i32; n];
                        interpolate(&f2.planes[plane], &mut p2, v2x, v2y, cw, ch, false, bd);
                        for (a, b) in p1.iter_mut().zip(p2.iter()) {
                            *a = (*a + *b + 1) >> 1;
                        }
                    }
                    self.write_block(plane, cx, cy, cw, ch, &p1);
                }
            }
        }
        Ok(())
    }

    /// `cu_recon`.
    fn cu_recon(&mut self, cu: &Cu, z: usize, px: i32, py: i32) -> Result<(), Error> {
        let tus = init_transform_units(cu);
        if cu.is_intra() {
            if cu.trans_size == TU_SPLIT_NON {
                self.intra_pred_luma(cu, cu.intra_pred_modes[0], px, py, tus[0].w, tus[0].h);
                if cu.cbp & 0x0F != 0 {
                    self.recons(cu, z, 0, tus[0], px, py)?;
                }
            } else {
                for (b, tu) in tus.iter().enumerate() {
                    self.intra_pred_luma(cu, cu.intra_pred_modes[b], px + tu.x, py + tu.y, tu.w, tu.h);
                    if cu.cbp & (1 << b) != 0 {
                        self.recons(cu, z, b, *tu, px, py)?;
                    }
                }
            }
            if !self.st.mono {
                self.intra_pred_chroma(cu, px >> 1, py >> 1);
            }
        } else {
            self.inter_pred(cu, px, py)?;
            if cu.trans_size == TU_SPLIT_NON {
                if cu.cbp & 0x0F != 0 {
                    self.recons(cu, z, 0, tus[0], px, py)?;
                }
            } else {
                for (b, tu) in tus.iter().enumerate() {
                    if cu.cbp & (1 << b) != 0 {
                        self.recons(cu, z, b, *tu, px, py)?;
                    }
                }
            }
        }
        if !self.st.mono {
            let s = 1 << (cu.level - 1);
            let cb = Cb { x: 0, y: 1 << cu.level, w: s, h: s };
            if cu.cbp & (1 << 4) != 0 {
                self.recons(cu, z, 4, cb, px, py)?;
            }
            if cu.cbp & (1 << 5) != 0 {
                self.recons(cu, z, 5, Cb { x: s, ..cb }, px, py)?;
            }
        }
        Ok(())
    }
}

fn wavelet_out(v: &[i32], w: usize, h: usize) -> Vec<i32> {
    wavelet(v, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dc_only_idct_is_flat() {
        let mut c = vec![0i32; 64];
        c[0] = 64;
        let mut out = vec![0i32; 64];
        idct(&c, &mut out, 8, 8, 8, false);
        assert!(out.iter().all(|&v| v == out[0]));
    }

    #[test]
    fn mc_pos_clamps() {
        assert_eq!(mc_pos(64, 8, 0, 0), 0);
        assert_eq!(mc_pos(64, 8, 0, -400), -64); // clamped to -blk-8 = -16 samples
    }
}
