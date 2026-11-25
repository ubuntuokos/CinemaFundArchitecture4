// Ported from FFmpeg n7.1.1 libavcodec/rv34dsp.c, rv30dsp.c, rv40dsp.c, rv30.c, h264chroma_template.c
// and h264qpel_template.c (Copyright (c) 2007 Konstantin Shishkov, 2003-2010 Michael Niedermayer),
// LGPL-2.1-or-later; see docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! Transforms, motion compensation and loop filters of RealVideo 3 / 4.

use crate::Plane;

#[inline]
fn clip8(v: i32) -> i32 {
    v.clamp(0, 255)
}

// ---------------------------------------------------------------------------------------------
// Transforms (rv34dsp.c)

fn row_transform(block: &[i16; 16]) -> [i32; 16] {
    let mut t = [0i32; 16];
    for i in 0..4 {
        let b = |k: usize| i32::from(block[i + 4 * k]);
        let z0 = 13 * (b(0) + b(2));
        let z1 = 13 * (b(0) - b(2));
        let z2 = 7 * b(1) - 17 * b(3);
        let z3 = 17 * b(1) + 7 * b(3);
        t[4 * i] = z0 + z3;
        t[4 * i + 1] = z1 + z2;
        t[4 * i + 2] = z1 - z2;
        t[4 * i + 3] = z0 - z3;
    }
    t
}

/// `rv34_idct_add`: adds the inverse transform of `block` to the 4×4 block at (`x`, `y`) and clears
/// `block`.
pub(super) fn idct_add(p: &mut Plane, x: usize, y: usize, block: &mut [i16; 16]) {
    let t = row_transform(block);
    *block = [0; 16];
    for i in 0..4 {
        let z0 = 13 * (t[i] + t[8 + i]) + 0x200;
        let z1 = 13 * (t[i] - t[8 + i]) + 0x200;
        let z2 = 7 * t[4 + i] - 17 * t[12 + i];
        let z3 = 17 * t[4 + i] + 7 * t[12 + i];
        let v = [(z0 + z3) >> 10, (z1 + z2) >> 10, (z1 - z2) >> 10, (z0 - z3) >> 10];
        let o = (y + i) * p.w + x;
        for (k, d) in v.iter().enumerate() {
            if let Some(px) = p.d.get_mut(o + k) {
                *px = clip8(i32::from(*px) + d) as u8;
            }
        }
    }
}

/// `rv34_idct_dc_add`.
pub(super) fn idct_dc_add(p: &mut Plane, x: usize, y: usize, dc: i32) {
    let dc = (13 * 13 * dc + 0x200) >> 10;
    for j in 0..4 {
        let o = (y + j) * p.w + x;
        for k in 0..4 {
            if let Some(px) = p.d.get_mut(o + k) {
                *px = clip8(i32::from(*px) + dc) as u8;
            }
        }
    }
}

/// `rv34_inv_transform_noround` (the 16×16 DC block).
pub(super) fn inv_transform(block: &mut [i16; 16]) {
    let t = row_transform(block);
    for i in 0..4 {
        let z0 = 39 * (t[i] + t[8 + i]);
        let z1 = 39 * (t[i] - t[8 + i]);
        let z2 = 21 * t[4 + i] - 51 * t[12 + i];
        let z3 = 51 * t[4 + i] + 21 * t[12 + i];
        block[i * 4] = ((z0 + z3) >> 11) as i16;
        block[i * 4 + 1] = ((z1 + z2) >> 11) as i16;
        block[i * 4 + 2] = ((z1 - z2) >> 11) as i16;
        block[i * 4 + 3] = ((z0 - z3) >> 11) as i16;
    }
}

/// `rv34_inv_transform_dc_noround`.
pub(super) fn inv_transform_dc(block: &mut [i16; 16]) {
    let dc = ((13 * 13 * 3 * i32::from(block[0])) >> 11) as i16;
    *block = [dc; 16];
}

// ---------------------------------------------------------------------------------------------
// Motion compensation

/// A `w`×`h` window of `p` at (`x`, `y`) with the edges replicated (as ffmpeg's padded,
/// edge-emulated references).
struct Window {
    d: Vec<i32>,
    w: usize,
    /// Window coordinates of the block origin.
    ox: usize,
    oy: usize,
}

impl Window {
    fn new(p: &Plane, x: isize, y: isize, bw: usize, bh: usize, margin: usize) -> Self {
        let w = bw + 2 * margin + 1;
        let h = bh + 2 * margin + 1;
        let mut d = Vec::with_capacity(w * h);
        for j in 0..h {
            for i in 0..w {
                d.push(p.at(x + i as isize - margin as isize, y + j as isize - margin as isize));
            }
        }
        Self { d, w, ox: margin, oy: margin }
    }
    #[inline]
    fn s(&self, i: isize, j: isize) -> i32 {
        let x = (self.ox as isize + i) as usize;
        let y = (self.oy as isize + j) as usize;
        self.d.get(y * self.w + x).copied().unwrap_or(0)
    }
}

/// RV40 six-tap filter coefficients for a quarter-pel phase (1, 2 or 3).
fn rv40_taps(phase: i32) -> (i32, i32, i32) {
    match phase {
        1 => (52, 20, 6),
        2 => (20, 20, 5),
        _ => (20, 52, 6),
    }
}

#[inline]
fn six_tap(a: [i32; 6], (c1, c2, sh): (i32, i32, i32)) -> i32 {
    clip8((a[0] + a[5] - 5 * (a[1] + a[4]) + a[2] * c1 + a[3] * c2 + (1 << (sh - 1))) >> sh)
}

/// RV40 luma prediction of a `bw`×`bh` block whose integer source position is (`sx`, `sy`) and
/// quarter-pel phase (`lx`, `ly`).
fn rv40_luma(src: &Plane, sx: isize, sy: isize, bw: usize, bh: usize, lx: i32, ly: i32, out: &mut [i32]) {
    let win = Window::new(src, sx, sy, bw, bh, 3);
    let h_at = |i: isize, j: isize, t| six_tap([win.s(i - 2, j), win.s(i - 1, j), win.s(i, j), win.s(i + 1, j), win.s(i + 2, j), win.s(i + 3, j)], t);
    if lx == 3 && ly == 3 {
        // `pixels_xy2`: the half-pel average with rounding 2.
        for j in 0..bh {
            for i in 0..bw {
                let (i, jj) = (i as isize, j as isize);
                let v = win.s(i, jj) + win.s(i + 1, jj) + win.s(i, jj + 1) + win.s(i + 1, jj + 1) + 2;
                out[j * bw + i as usize] = v >> 2;
            }
        }
        return;
    }
    match (lx, ly) {
        (0, 0) => {
            for j in 0..bh {
                for i in 0..bw {
                    out[j * bw + i] = win.s(i as isize, j as isize);
                }
            }
        }
        (_, 0) => {
            let t = rv40_taps(lx);
            for j in 0..bh {
                for i in 0..bw {
                    out[j * bw + i] = h_at(i as isize, j as isize, t);
                }
            }
        }
        (0, _) => {
            let t = rv40_taps(ly);
            for j in 0..bh {
                for i in 0..bw {
                    let (i, jj) = (i as isize, j as isize);
                    out[j * bw + i as usize] =
                        six_tap([win.s(i, jj - 2), win.s(i, jj - 1), win.s(i, jj), win.s(i, jj + 1), win.s(i, jj + 2), win.s(i, jj + 3)], t);
                }
            }
        }
        _ => {
            let (th, tv) = (rv40_taps(lx), rv40_taps(ly));
            // Rows −2 … bh + 2 of the horizontally filtered block.
            let rows = bh + 5;
            let mut full = vec![0i32; rows * bw];
            for r in 0..rows {
                for i in 0..bw {
                    full[r * bw + i] = h_at(i as isize, r as isize - 2, th);
                }
            }
            for j in 0..bh {
                for i in 0..bw {
                    let f = |k: usize| full[(j + k) * bw + i];
                    out[j * bw + i] = six_tap([f(0), f(1), f(2), f(3), f(4), f(5)], tv);
                }
            }
        }
    }
}

/// RV30 third-pel luma prediction (`rv30_tpel*`); phases 0 … 2.
fn rv30_luma(src: &Plane, sx: isize, sy: isize, bw: usize, bh: usize, lx: i32, ly: i32, out: &mut [i32]) {
    let win = Window::new(src, sx, sy, bw, bh, 2);
    let s = |i: isize, j: isize| win.s(i, j);
    let four = |a: i32, b: i32, c: i32, d: i32, c1: i32, c2: i32| clip8((-(a + d) + b * c1 + c * c2 + 8) >> 4);
    for j in 0..bh {
        for i in 0..bw {
            let (x, y) = (i as isize, j as isize);
            let v = match (lx, ly) {
                (0, 0) => s(x, y),
                (1, 0) => four(s(x - 1, y), s(x, y), s(x + 1, y), s(x + 2, y), 12, 6),
                (2, 0) => four(s(x - 1, y), s(x, y), s(x + 1, y), s(x + 2, y), 6, 12),
                (0, 1) => four(s(x, y - 1), s(x, y), s(x, y + 1), s(x, y + 2), 12, 6),
                (0, 2) => four(s(x, y - 1), s(x, y), s(x, y + 1), s(x, y + 2), 6, 12),
                (1, 1) => clip8(
                    (s(x - 1, y - 1) - 12 * s(x, y - 1) - 6 * s(x + 1, y - 1) + s(x + 2, y - 1) - 12 * s(x - 1, y) + 144 * s(x, y) + 72 * s(x + 1, y)
                        - 12 * s(x + 2, y)
                        - 6 * s(x - 1, y + 1)
                        + 72 * s(x, y + 1)
                        + 36 * s(x + 1, y + 1)
                        - 6 * s(x + 2, y + 1)
                        + s(x - 1, y + 2)
                        - 12 * s(x, y + 2)
                        - 6 * s(x + 1, y + 2)
                        + s(x + 2, y + 2)
                        + 128)
                        >> 8,
                ),
                (2, 1) => clip8(
                    (s(x - 1, y - 1) - 12 * s(x + 1, y - 1) - 6 * s(x, y - 1) + s(x + 2, y - 1) - 12 * s(x - 1, y) + 144 * s(x + 1, y) + 72 * s(x, y)
                        - 12 * s(x + 2, y)
                        - 6 * s(x - 1, y + 1)
                        + 72 * s(x + 1, y + 1)
                        + 36 * s(x, y + 1)
                        - 6 * s(x + 2, y + 1)
                        + s(x - 1, y + 2)
                        - 12 * s(x + 1, y + 2)
                        - 6 * s(x, y + 2)
                        + s(x + 2, y + 2)
                        + 128)
                        >> 8,
                ),
                (1, 2) => clip8(
                    (s(x - 1, y - 1) - 12 * s(x, y - 1) - 6 * s(x + 1, y - 1) + s(x + 2, y - 1) - 6 * s(x - 1, y) + 72 * s(x, y) + 36 * s(x + 1, y)
                        - 6 * s(x + 2, y)
                        - 12 * s(x - 1, y + 1)
                        + 144 * s(x, y + 1)
                        + 72 * s(x + 1, y + 1)
                        - 12 * s(x + 2, y + 1)
                        + s(x - 1, y + 2)
                        - 12 * s(x, y + 2)
                        - 6 * s(x + 1, y + 2)
                        + s(x + 2, y + 2)
                        + 128)
                        >> 8,
                ),
                _ => clip8(
                    (36 * s(x, y)
                        + 54 * s(x + 1, y)
                        + 6 * s(x + 2, y)
                        + 54 * s(x, y + 1)
                        + 81 * s(x + 1, y + 1)
                        + 9 * s(x + 2, y + 1)
                        + 6 * s(x, y + 2)
                        + 9 * s(x + 1, y + 2)
                        + s(x + 2, y + 2)
                        + 128)
                        >> 8,
                ),
            };
            out[j * bw + i] = v;
        }
    }
}

/// Writes (`put`) or averages (`avg`) `v` into the `bw`×`bh` block of `dst` at (`dx`, `dy`).
fn store(dst: &mut Plane, dx: usize, dy: usize, bw: usize, bh: usize, v: &[i32], avg: bool) {
    for j in 0..bh {
        let o = (dy + j) * dst.w + dx;
        for i in 0..bw {
            let (Some(d), Some(&p)) = (dst.d.get_mut(o + i), v.get(j * bw + i)) else { continue };
            *d = if avg { ((i32::from(*d) + p + 1) >> 1) as u8 } else { p as u8 };
        }
    }
}

/// Luma motion compensation of a `bw`×`bh` block.
#[allow(clippy::too_many_arguments)]
pub(super) fn luma_mc(rv30: bool, dst: &mut Plane, dx: usize, dy: usize, bw: usize, bh: usize, src: &Plane, sx: isize, sy: isize, lx: i32, ly: i32, avg: bool) {
    let mut v = vec![0i32; bw * bh];
    if rv30 {
        rv30_luma(src, sx, sy, bw, bh, lx, ly, &mut v);
    } else {
        rv40_luma(src, sx, sy, bw, bh, lx, ly, &mut v);
    }
    store(dst, dx, dy, bw, bh, &v, avg);
}

const RV40_BIAS: [[i32; 4]; 4] = [[0, 16, 32, 16], [32, 28, 32, 28], [0, 32, 16, 32], [32, 28, 32, 28]];

/// Chroma motion compensation (`h264_chroma_mc` for RV30, `rv40_chroma_mc` for RV40); `x`, `y` are
/// eighth-pel phases.
#[allow(clippy::too_many_arguments)]
pub(super) fn chroma_mc(rv30: bool, dst: &mut Plane, dx: usize, dy: usize, bw: usize, bh: usize, src: &Plane, sx: isize, sy: isize, x: i32, y: i32, avg: bool) {
    let a = (8 - x) * (8 - y);
    let b = x * (8 - y);
    let c = (8 - x) * y;
    let d = x * y;
    let bias = if rv30 { 32 } else { RV40_BIAS[((y >> 1) & 3) as usize][((x >> 1) & 3) as usize] };
    let mut v = vec![0i32; bw * bh];
    for j in 0..bh {
        for i in 0..bw {
            let (px, py) = (sx + i as isize, sy + j as isize);
            let s = a * src.at(px, py) + b * src.at(px + 1, py) + c * src.at(px, py + 1) + d * src.at(px + 1, py + 1);
            v[j * bw + i] = (s + bias) >> 6;
        }
    }
    store(dst, dx, dy, bw, bh, &v, avg);
}

/// `rv40_weight_func_{rnd,nornd}`: `dst = w2 * src1 + w1 * src2` of two `n`×`n` predictions
/// (at the origin of `src1` and `src2`).
#[allow(clippy::too_many_arguments)]
pub(super) fn weight(dst: &mut Plane, dx: usize, dy: usize, n: usize, src1: &Plane, src2: &Plane, w1: i32, w2: i32, scaled: bool) {
    let (w1, w2) = (w1 as u32, w2 as u32);
    for j in 0..n {
        for i in 0..n {
            let s1 = u32::from(src1.d.get(j * src1.w + i).copied().unwrap_or(0));
            let s2 = u32::from(src2.d.get(j * src2.w + i).copied().unwrap_or(0));
            let v = if scaled {
                w2.wrapping_mul(s1).wrapping_add(w1.wrapping_mul(s2)).wrapping_add(0x10) >> 5
            } else {
                ((w2.wrapping_mul(s1) >> 9).wrapping_add(w1.wrapping_mul(s2) >> 9).wrapping_add(0x10)) >> 5
            };
            if let Some(d) = dst.d.get_mut((dy + j) * dst.w + dx + i) {
                *d = v as u8;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Loop filters

/// Four lines of pixels across an edge: `get(k, i)` is the pixel `k` steps across the edge (`k = 0`
/// the first pixel after it) on line `i`.
struct Edge<'a> {
    d: &'a mut [u8],
    base: isize,
    step: isize,
    along: isize,
}

impl Edge<'_> {
    /// The edge at (`x`, `y`) of `p`; `vertical` edges have their pixels side by side. `None` when
    /// pixels `-reach … reach - 1` across it fall outside the plane.
    fn new(p: &mut Plane, x: usize, y: usize, vertical: bool, reach: usize) -> Option<Edge<'_>> {
        let (w, h) = (p.w, p.h);
        if vertical {
            if x < reach || x + reach > w || y + 4 > h {
                return None;
            }
        } else if y < reach || y + reach > h || x + 4 > w {
            return None;
        }
        let (step, along) = if vertical { (1, w as isize) } else { (w as isize, 1) };
        Some(Edge { d: &mut p.d, base: (y * w + x) as isize, step, along })
    }
    #[inline]
    fn get(&self, k: isize, i: isize) -> i32 {
        i32::from(self.d.get((self.base + k * self.step + i * self.along) as usize).copied().unwrap_or(0))
    }
    #[inline]
    fn set(&mut self, k: isize, i: isize, v: i32) {
        if let Some(d) = self.d.get_mut((self.base + k * self.step + i * self.along) as usize) {
            *d = v as u8;
        }
    }
}

/// `rv30_weak_loop_filter` across the edge at (`x`, `y`).
pub(super) fn rv30_filter(p: &mut Plane, x: usize, y: usize, vertical: bool, lim: i32) {
    let Some(mut e) = Edge::new(p, x, y, vertical, 2) else { return };
    for i in 0..4 {
        let diff = ((e.get(-2, i) - e.get(1, i)) - (e.get(-1, i) - e.get(0, i)) * 4) >> 3;
        let diff = diff.clamp(-lim, lim);
        let (a, b) = (e.get(-1, i), e.get(0, i));
        e.set(-1, i, clip8(a + diff));
        e.set(0, i, clip8(b - diff));
    }
}

const DITHER_L: [i32; 16] = [0x40, 0x50, 0x20, 0x60, 0x30, 0x50, 0x40, 0x30, 0x50, 0x40, 0x50, 0x30, 0x60, 0x20, 0x50, 0x40];
const DITHER_R: [i32; 16] = [0x40, 0x30, 0x60, 0x20, 0x50, 0x30, 0x30, 0x40, 0x40, 0x40, 0x50, 0x30, 0x20, 0x60, 0x30, 0x40];

#[allow(clippy::too_many_arguments)]
fn rv40_weak(e: &mut Edge<'_>, filter_p1: bool, filter_q1: bool, alpha: i32, beta: i32, lim_p0q0: i32, lim_q1: i32, lim_p1: i32) {
    for i in 0..4 {
        let diff_p1p0 = e.get(-2, i) - e.get(-1, i);
        let diff_q1q0 = e.get(1, i) - e.get(0, i);
        let diff_p1p2 = e.get(-2, i) - e.get(-3, i);
        let diff_q1q2 = e.get(1, i) - e.get(2, i);
        let mut t = e.get(0, i) - e.get(-1, i);
        if t == 0 {
            continue;
        }
        let u = (alpha * t.abs()) >> 7;
        if u > 3 - i32::from(filter_p1 && filter_q1) {
            continue;
        }
        t <<= 2;
        if filter_p1 && filter_q1 {
            t += e.get(-2, i) - e.get(1, i);
        }
        let diff = ((t + 4) >> 3).clamp(-lim_p0q0, lim_p0q0);
        let (p0, q0) = (e.get(-1, i), e.get(0, i));
        e.set(-1, i, clip8(p0 + diff));
        e.set(0, i, clip8(q0 - diff));
        if filter_p1 && diff_p1p2.abs() <= beta {
            let t = (diff_p1p0 + diff_p1p2 - diff) >> 1;
            let p1 = e.get(-2, i);
            e.set(-2, i, clip8(p1 - t.clamp(-lim_p1, lim_p1)));
        }
        if filter_q1 && diff_q1q2.abs() <= beta {
            let t = (diff_q1q0 + diff_q1q2 + diff) >> 1;
            let q1 = e.get(1, i);
            e.set(1, i, clip8(q1 - t.clamp(-lim_q1, lim_q1)));
        }
    }
}

fn rv40_strong(e: &mut Edge<'_>, alpha: i32, lims: i32, dmode: usize, chroma: bool) {
    for i in 0..4isize {
        let t = e.get(0, i) - e.get(-1, i);
        if t == 0 {
            continue;
        }
        let sflag = (alpha * t.abs()) >> 7;
        if sflag > 1 {
            continue;
        }
        let dl = DITHER_L.get(dmode + i as usize).copied().unwrap_or(0);
        let dr = DITHER_R.get(dmode + i as usize).copied().unwrap_or(0);
        let s = |k: isize| e.get(k, i);
        let mut p0 = (25 * s(-3) + 26 * s(-2) + 26 * s(-1) + 26 * s(0) + 25 * s(1) + dl) >> 7;
        let mut q0 = (25 * s(-2) + 26 * s(-1) + 26 * s(0) + 26 * s(1) + 25 * s(2) + dr) >> 7;
        if sflag != 0 {
            p0 = p0.clamp(s(-1) - lims, s(-1) + lims);
            q0 = q0.clamp(s(0) - lims, s(0) + lims);
        }
        let mut p1 = (25 * s(-4) + 26 * s(-3) + 26 * s(-2) + 26 * p0 + 25 * s(0) + dl) >> 7;
        let mut q1 = (25 * s(-1) + 26 * q0 + 26 * s(1) + 26 * s(2) + 25 * s(3) + dr) >> 7;
        if sflag != 0 {
            p1 = p1.clamp(s(-2) - lims, s(-2) + lims);
            q1 = q1.clamp(s(1) - lims, s(1) + lims);
        }
        e.set(-2, i, p1);
        e.set(-1, i, p0);
        e.set(0, i, q0);
        e.set(1, i, q1);
        if !chroma {
            let v = (25 * e.get(-1, i) + 26 * e.get(-2, i) + 51 * e.get(-3, i) + 26 * e.get(-4, i) + 64) >> 7;
            e.set(-3, i, v);
            let v = (25 * e.get(0, i) + 26 * e.get(1, i) + 51 * e.get(2, i) + 26 * e.get(3, i) + 64) >> 7;
            e.set(2, i, v);
        }
    }
}

/// `rv40_loop_filter_strength`: (strong, filter_p1, filter_q1).
fn rv40_strength(e: &Edge<'_>, beta: i32, beta2: i32, edge: bool) -> (bool, bool, bool) {
    let (mut sp, mut sq) = (0, 0);
    for i in 0..4 {
        sp += e.get(-2, i) - e.get(-1, i);
        sq += e.get(1, i) - e.get(0, i);
    }
    let p1 = sp.abs() < (beta << 2);
    let q1 = sq.abs() < (beta << 2);
    if (!p1 && !q1) || !edge {
        return (false, p1, q1);
    }
    let (mut sp2, mut sq2) = (0, 0);
    for i in 0..4 {
        sp2 += e.get(-2, i) - e.get(-3, i);
        sq2 += e.get(1, i) - e.get(2, i);
    }
    (p1 && sp2.abs() < beta2 && q1 && sq2.abs() < beta2, p1, q1)
}

/// `rv40_adaptive_loop_filter` across the edge at (`x`, `y`): `dir` 0 is a horizontal edge, 1 a
/// vertical one.
#[allow(clippy::too_many_arguments)]
pub(super) fn rv40_adaptive(
    p: &mut Plane,
    x: usize,
    y: usize,
    dmode: usize,
    lim_q1: i32,
    lim_p1: i32,
    alpha: i32,
    beta: i32,
    beta2: i32,
    chroma: bool,
    edge: bool,
    dir: usize,
) {
    let Some(mut e) = Edge::new(p, x, y, dir == 1, 4) else { return };
    let (strong, fp1, fq1) = rv40_strength(&e, beta, beta2, edge);
    let lims = i32::from(fp1) + i32::from(fq1) + ((lim_q1 + lim_p1) >> 1) + 1;
    if strong {
        rv40_strong(&mut e, alpha, lims, dmode, chroma);
    } else if fp1 && fq1 {
        rv40_weak(&mut e, true, true, alpha, beta, lims, lim_q1, lim_p1);
    } else if fp1 || fq1 {
        rv40_weak(&mut e, fp1, fq1, alpha, beta, lims >> 1, lim_q1 >> 1, lim_p1 >> 1);
    }
}
