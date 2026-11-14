// SPDX-License-Identifier: GPL-3.0-or-later AND BSD-3-Clause
//! Sample-level tools of the EVC Baseline decoder: dequantisation and inverse
//! transform (`xevd_itdq.c`), intra prediction (`xevd_ipred.c`), motion
//! compensation (`xevd_mc.c`), reconstruction (`xevd_recon.c`) and the
//! deblocking filter kernels (`xevd_df.c`).

use crate::tables::{DQ_SCALE_B, TM2, TM4, TM8, TM16, TM32, TM64};

/// Transform matrix entry `tm[row][col]` of the `n`-point DCT.
fn tm(n: usize, row: usize, col: usize) -> i64 {
    let v = match n {
        2 => TM2.get(row).and_then(|r| r.get(col)),
        4 => TM4.get(row).and_then(|r| r.get(col)),
        8 => TM8.get(row).and_then(|r| r.get(col)),
        16 => TM16.get(row).and_then(|r| r.get(col)),
        32 => TM32.get(row).and_then(|r| r.get(col)),
        _ => TM64.get(row).and_then(|r| r.get(col)),
    };
    i64::from(v.copied().unwrap_or(0))
}

/// Partial butterfly of `xevd_itx_pb*b`: even/odd decomposition with exact
/// 64-bit sums. `s[i]` is the coefficient of matrix row `i * stride`.
fn butterfly(n: usize, size: usize, stride: usize, s: &[i64], out: &mut [i64]) {
    if n == 2 {
        let (s0, s1) = (s.first().copied().unwrap_or(0), s.get(1).copied().unwrap_or(0));
        for (j, o) in out.iter_mut().enumerate().take(2) {
            *o = tm(size, 0, j) * s0 + tm(size, stride, j) * s1;
        }
        return;
    }
    let half = n / 2;
    let even: Vec<i64> = s.iter().step_by(2).copied().collect();
    let mut e = vec![0i64; half];
    butterfly(half, size, stride * 2, &even, &mut e);
    for k in 0..half {
        let mut o = 0i64;
        for i in (1..n).step_by(2) {
            o += tm(size, i * stride, k) * s.get(i).copied().unwrap_or(0);
        }
        let ek = e.get(k).copied().unwrap_or(0);
        if let Some(d) = out.get_mut(k) {
            *d = ek + o;
        }
        if let Some(d) = out.get_mut(n - 1 - k) {
            *d = ek - o;
        }
    }
}

/// `xevd_itrans`: column transform without rounding, then the row transform
/// with the final shift and 16-bit clipping. `coef` is `w × h`, row-major.
fn itrans(coef: &mut [i16], w: usize, h: usize, bit_depth: u32) {
    let shift = 7 + 12 - (bit_depth as i64 - 8);
    let add = 1i64 << (shift - 1);
    let mut tb = vec![0i32; w * h];
    let mut s = vec![0i64; h.max(w)];
    let mut out = vec![0i64; h.max(w)];
    for j in 0..w {
        for k in 0..h {
            s[k] = i64::from(coef.get(k * w + j).copied().unwrap_or(0));
        }
        butterfly(h, h, 1, &s[..h], &mut out[..h]);
        for k in 0..h {
            if let Some(t) = tb.get_mut(j * h + k) {
                *t = out[k] as i32;
            }
        }
    }
    for j in 0..h {
        for k in 0..w {
            s[k] = i64::from(tb.get(k * h + j).copied().unwrap_or(0));
        }
        butterfly(w, w, 1, &s[..w], &mut out[..w]);
        for k in 0..w {
            if let Some(c) = coef.get_mut(j * w + k) {
                *c = ((out[k] + add) >> shift).clamp(-32768, 32767) as i16;
            }
        }
    }
}

/// `xevd_itdq`: dequantise and inverse-transform one `w × h` block in place.
pub(crate) fn itdq(coef: &mut [i16], log2_w: u32, log2_h: u32, qp: i32, bit_depth: u32) {
    let qp = qp.max(0);
    let scale = i64::from(DQ_SCALE_B[(qp % 6) as usize]) << (qp / 6).min(20);
    let log2_size = ((log2_w + log2_h) >> 1) as i64;
    let odd = (log2_w + log2_h) & 1 == 1;
    let tr_shift = 15 - bit_depth as i64 - log2_size;
    let shift = 20 - 14 - tr_shift + if odd { 8 } else { 0 };
    let ns_scale: i64 = if odd { 181 } else { 1 };
    let offset = if shift <= 0 { 0 } else { 1i64 << (shift - 1) };
    let n = 1usize << (log2_w + log2_h);
    for c in coef.iter_mut().take(n) {
        let lev = (i64::from(*c) * (scale * ns_scale) + offset) >> shift.max(0);
        *c = lev.clamp(-32768, 32767) as i16;
    }
    itrans(coef, 1 << log2_w, 1 << log2_h, bit_depth);
}

/// `xevd_recon`: adds the residual (16-bit wrap as in the reference) and clips.
pub(crate) fn recon(coef: Option<&[i16]>, pred: &[i16], w: usize, h: usize, dst: &mut [u16], stride: usize, bit_depth: u32) {
    let max = (1i32 << bit_depth) - 1;
    for i in 0..h {
        for j in 0..w {
            let p = pred.get(i * w + j).copied().unwrap_or(0);
            let v = match coef {
                Some(c) => i32::from(c.get(i * w + j).copied().unwrap_or(0).wrapping_add(p)),
                None => i32::from(p),
            };
            if let Some(d) = dst.get_mut(i * stride + j) {
                *d = v.clamp(0, max) as u16;
            }
        }
    }
}

// ---------------------------------------------------------------- intra

pub(crate) const IPD_DC: u8 = 0;
pub(crate) const IPD_HOR: u8 = 1;
pub(crate) const IPD_VER: u8 = 2;
pub(crate) const IPD_UL: u8 = 3;
pub(crate) const IPD_UR: u8 = 4;

/// Neighbouring samples of a block: `left[k]` is the sample at row `k - 1`
/// left of the block (so `left[0]` is the top-left corner), `up[k]` the sample
/// at column `k - 1` above it.
pub(crate) struct Nbr {
    pub(crate) left: Vec<i32>,
    pub(crate) up: Vec<i32>,
}

/// `xevd_ipred_b` / `xevd_ipred_uv_b` for the five Baseline modes.
pub(crate) fn ipred(nb: &Nbr, mode: u8, w: usize, h: usize, dst: &mut [i16]) {
    let le = |k: isize| nb.left.get((k + 1) as usize).copied().unwrap_or(0);
    let up = |k: isize| nb.up.get((k + 1) as usize).copied().unwrap_or(0);
    let mut put = |i: usize, j: usize, v: i32| {
        if let Some(d) = dst.get_mut(i * w + j) {
            *d = v as i16;
        }
    };
    match mode {
        IPD_VER => {
            for i in 0..h {
                for j in 0..w {
                    put(i, j, up(j as isize));
                }
            }
        }
        IPD_HOR => {
            for i in 0..h {
                for j in 0..w {
                    put(i, j, le(i as isize));
                }
            }
        }
        IPD_DC => {
            let mut dc: i32 = 0;
            for i in 0..h {
                dc += le(i as isize);
            }
            for j in 0..w {
                dc += up(j as isize);
            }
            let dc = (dc + w as i32) >> (w.trailing_zeros() + 1);
            for i in 0..h {
                for j in 0..w {
                    put(i, j, dc);
                }
            }
        }
        IPD_UL => {
            for i in 0..h {
                for j in 0..w {
                    let diag = i as isize - j as isize;
                    let v = if diag > 0 {
                        le(diag - 1)
                    } else if diag == 0 {
                        up(-1)
                    } else {
                        up(-diag - 1)
                    };
                    put(i, j, v);
                }
            }
        }
        IPD_UR => {
            for i in 0..h {
                for j in 0..w {
                    let k = (i + j + 1) as isize;
                    put(i, j, (up(k) + le(k)) >> 1);
                }
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------- motion

const MC_L: [[i32; 8]; 4] = [[0, 0, 0, 64, 0, 0, 0, 0], [0, 1, -5, 52, 20, -5, 1, 0], [0, 2, -10, 40, 40, -10, 2, 0], [0, 1, -5, 20, 52, -5, 1, 0]];
const MC_C: [[i32; 4]; 8] =
    [[0, 64, 0, 0], [-2, 58, 10, -2], [-4, 52, 20, -4], [-6, 46, 30, -6], [-8, 40, 40, -8], [-6, 30, 46, -6], [-4, 20, 52, -4], [-2, 10, 58, -2]];

/// A reference plane: samples outside `w × h` repeat the edge, which is what
/// the padded pictures of the reference decoder contain.
pub(crate) struct RefPlane<'a> {
    pub(crate) data: &'a [u16],
    pub(crate) stride: usize,
    pub(crate) w: usize,
    pub(crate) h: usize,
}

impl RefPlane<'_> {
    fn at(&self, x: i64, y: i64) -> i32 {
        let x = x.clamp(0, self.w as i64 - 1) as usize;
        let y = y.clamp(0, self.h as i64 - 1) as usize;
        i32::from(self.data.get(y * self.stride + x).copied().unwrap_or(0))
    }
}

/// Interpolates a `w × h` block. `gx`, `gy` are positions in 1/`1 << frac_bits`
/// sample units; `fx`, `fy` select the filtered paths as the reference decoder
/// does (from the unclipped motion vector).
#[allow(clippy::too_many_arguments)]
fn mc(rp: &RefPlane<'_>, gx: i64, gy: i64, frac_bits: u32, fx: bool, fy: bool, w: usize, h: usize, bit_depth: u32, pred: &mut [i16], luma: bool) {
    let max = (1i32 << bit_depth) - 1;
    let mask = (1i64 << frac_bits) - 1;
    let (ix, iy) = (gx >> frac_bits, gy >> frac_bits);
    let (dx, dy) = ((gx & mask) as usize, (gy & mask) as usize);
    let taps: usize = if luma { 8 } else { 4 };
    let back = if luma { 3 } else { 1 };
    let coef = |d: usize| -> [i32; 8] {
        let mut c = [0i32; 8];
        if luma {
            if let Some(r) = MC_L.get(d / 4) {
                c = *r;
            }
        } else if let Some(r) = MC_C.get(d / 4) {
            c[..4].copy_from_slice(r);
        }
        c
    };
    let cx = coef(dx);
    let cy = coef(dy);
    let mut put = |i: usize, j: usize, v: i32| {
        if let Some(p) = pred.get_mut(i * w + j) {
            *p = v.clamp(0, max) as i16;
        }
    };
    match (fx, fy) {
        (false, false) => {
            for i in 0..h {
                for j in 0..w {
                    put(i, j, rp.at(ix + j as i64, iy + i as i64));
                }
            }
        }
        (true, false) => {
            for i in 0..h {
                for j in 0..w {
                    let mut s = 0i32;
                    for (t, c) in cx.iter().enumerate().take(taps) {
                        s += c * rp.at(ix - back + (j + t) as i64, iy + i as i64);
                    }
                    put(i, j, s >> 6);
                }
            }
        }
        (false, true) => {
            for i in 0..h {
                for j in 0..w {
                    let mut s = 0i32;
                    for (t, c) in cy.iter().enumerate().take(taps) {
                        s += c * rp.at(ix + j as i64, iy - back + (i + t) as i64);
                    }
                    put(i, j, s >> 6);
                }
            }
        }
        (true, true) => {
            let shift1 = (bit_depth as i32 - 8).min(4);
            let shift2 = (20 - bit_depth as i32).max(8);
            let offset2 = 1i32 << (shift2 - 1);
            let rows = h + taps - 1;
            let mut buf = vec![0i16; rows * w];
            for i in 0..rows {
                for j in 0..w {
                    let mut s = 0i32;
                    for (t, c) in cx.iter().enumerate().take(taps) {
                        s += c * rp.at(ix - back + (j + t) as i64, iy - back + i as i64);
                    }
                    if let Some(b) = buf.get_mut(i * w + j) {
                        *b = (s >> shift1) as i16;
                    }
                }
            }
            for i in 0..h {
                for j in 0..w {
                    let mut s = 0i32;
                    for (t, c) in cy.iter().enumerate().take(taps) {
                        s += c * i32::from(buf.get((i + t) * w + j).copied().unwrap_or(0));
                    }
                    put(i, j, (s + offset2) >> shift2);
                }
            }
        }
    }
}

/// Luma motion compensation (`xevd_mc_l`): `qx`, `qy` is the clipped block
/// position in quarter samples, `mv` the unclipped motion vector.
#[allow(clippy::too_many_arguments)]
pub(crate) fn mc_luma(rp: &RefPlane<'_>, qx: i64, qy: i64, mv: [i16; 2], w: usize, h: usize, bit_depth: u32, pred: &mut [i16]) {
    let fx = mv[0] & 3 != 0;
    let fy = mv[1] & 3 != 0;
    mc(rp, qx << 2, qy << 2, 4, fx, fy, w, h, bit_depth, pred, true);
}

/// 4:2:0 chroma motion compensation (`xevd_mc_c`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn mc_chroma(rp: &RefPlane<'_>, qx: i64, qy: i64, mv: [i16; 2], w: usize, h: usize, bit_depth: u32, pred: &mut [i16]) {
    let fx = mv[0] & 7 != 0;
    let fy = mv[1] & 7 != 0;
    mc(rp, qx << 2, qy << 2, 5, fx, fy, w, h, bit_depth, pred, false);
}

// ---------------------------------------------------------------- deblocking

/// One luma edge segment of four samples (`deblock_scu_hor/ver`). `at(k, i)`
/// addresses sample `k` (−2..=1 across the edge) of line `i`.
pub(crate) fn deblock_luma(plane: &mut [u16], idx: impl Fn(isize, usize) -> Option<usize>, st: i32, bd: u32) {
    let max = (1i32 << bd) - 1;
    for i in 0..4 {
        let (Some(ia), Some(ib), Some(ic), Some(id)) = (idx(-2, i), idx(-1, i), idx(0, i), idx(1, i)) else {
            continue;
        };
        let get = |k: usize| i32::from(plane.get(k).copied().unwrap_or(0));
        let (a, b, c, d) = (get(ia), get(ib), get(ic), get(id));
        let dd = i32::from(((a - (b << 2) + (c << 2) - d) / 8) as i16);
        let abs_d = dd.abs();
        let t16 = ((abs_d - st) << 1).max(0);
        let clip = (abs_d - t16).max(0);
        let d1 = if dd < 0 { -clip } else { clip };
        let clip2 = clip >> 1;
        let d2 = ((a - d) / 4).clamp(-clip2, clip2);
        let vals = [(ia, a - d2), (ib, b + d1), (ic, c - d1), (id, d + d2)];
        for (k, v) in vals {
            if let Some(p) = plane.get_mut(k) {
                *p = (i32::from(v as i16)).clamp(0, max) as u16;
            }
        }
    }
}

/// One chroma edge segment of `n` samples: only the two samples next to the
/// edge change.
pub(crate) fn deblock_chroma(plane: &mut [u16], idx: impl Fn(isize, usize) -> Option<usize>, n: usize, st: i32, bd: u32) {
    let max = (1i32 << bd) - 1;
    for i in 0..n {
        let (Some(ia), Some(ib), Some(ic), Some(id)) = (idx(-2, i), idx(-1, i), idx(0, i), idx(1, i)) else {
            continue;
        };
        let get = |k: usize| i32::from(plane.get(k).copied().unwrap_or(0));
        let (a, b, c, d) = (get(ia), get(ib), get(ic), get(id));
        let dd = i32::from(((a - (b << 2) + (c << 2) - d) / 8) as i16);
        let abs_d = dd.abs();
        let t16 = ((abs_d - st) << 1).max(0);
        let clip = (abs_d - t16).max(0);
        let d1 = if dd < 0 { -clip } else { clip };
        for (k, v) in [(ib, b + d1), (ic, c - d1)] {
            if let Some(p) = plane.get_mut(k) {
                *p = (i32::from(v as i16)).clamp(0, max) as u16;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The butterfly equals the plain matrix product, which is what the
    /// Baseline transform is defined as.
    #[test]
    fn butterfly_matches_matrix_product() {
        for n in [2usize, 4, 8, 16, 32, 64] {
            let s: Vec<i64> = (0..n as i64).map(|i| (i * 37 % 23) - 11).collect();
            let mut out = vec![0i64; n];
            butterfly(n, n, 1, &s, &mut out);
            for (j, &o) in out.iter().enumerate() {
                let direct: i64 = (0..n).map(|i| tm(n, i, j) * s[i]).sum();
                assert_eq!(o, direct, "n={n} j={j}");
            }
        }
    }

    #[test]
    fn dc_only_block_is_flat() {
        let mut c = vec![0i16; 16];
        c[0] = 64;
        itdq(&mut c, 2, 2, 22, 8);
        assert!(c.iter().all(|&v| v == c[0]));
    }
}
