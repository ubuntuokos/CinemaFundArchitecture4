// SPDX-License-Identifier: GPL-3.0-or-later
//! Inverse transforms: DCT-2, DCT-8, DST-7 and the secondary transforms.
//!
//! Derived from uavs3d 1.2 (commit 0e20d2c2), Copyright (c) 2018-2022 Peking University
//! Shenzhen Graduate School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation
//! Corporation, BSD 3-clause licence (docs/licensing.md). The partial-butterfly kernels of
//! uavs3d are mathematically the plain matrix products used here (same integer sums, same
//! rounding and clipping between the two stages).

use std::sync::OnceLock;

use crate::tables::{ITRANS_C4, ITRANS_C8};

pub(crate) const DCT2: usize = 0;
pub(crate) const DCT8: usize = 1;
pub(crate) const DST7: usize = 2;

type Mats = [[Vec<i8>; 3]; 6];

/// Transform matrices `[type][log2 size - 1]`, generated as `com_dct_coef_init` does.
fn mats() -> &'static Mats {
    static M: OnceLock<Mats> = OnceLock::new();
    M.get_or_init(|| {
        let pi = std::f64::consts::PI;
        let mut out: Mats = Default::default();
        let mut n = 2usize;
        for i in 0..6 {
            let s = (n as f64).sqrt() * 32.0;
            let nf = n as f64;
            let mut t = [vec![0i8; n * n], vec![0i8; n * n], vec![0i8; n * n]];
            for k in 0..n {
                for j in 0..n {
                    let kf = k as f64;
                    let jf = j as f64;
                    let w0 = if k == 0 { 0.5f64.sqrt() } else { 1.0 };
                    let v = (pi * (jf + 0.5) * kf / nf).cos() * w0 * (2.0 / nf).sqrt();
                    t[DCT2][k * n + j] = (s * v + if v > 0.0 { 0.5 } else { -0.5 }) as i8;
                    let v = (pi * (kf + 0.5) * (jf + 0.5) / (nf + 0.5)).cos() * (2.0 / (nf + 0.5)).sqrt();
                    t[DCT8][k * n + j] = (s * v + if v > 0.0 { 0.5 } else { -0.5 }) as i8;
                    let v = (pi * (kf + 0.5) * (jf + 1.0) / (nf + 0.5)).sin() * (2.0 / (nf + 0.5)).sqrt();
                    t[DST7][k * n + j] = (s * v + if v > 0.0 { 0.5 } else { -0.5 }) as i8;
                }
            }
            let [a, b, c] = t;
            out[i] = [a, b, c];
            n <<= 1;
        }
        out
    })
}

fn clip(v: i32, lo: i32, hi: i32) -> i16 {
    v.clamp(lo, hi) as i16
}

/// One 1-D inverse stage over `lines` lines: `src[k * stride + line]` are the `n` inputs of a
/// line; the `n` outputs go to `dst[line * n + j]`. Only the first `nz` inputs are non-zero.
#[allow(clippy::too_many_arguments)]
fn stage(m: &[i8], n: usize, nz: usize, src: &[i16], stride: usize, lines: usize, dst: &mut [i16], shift: u32, lo: i32, hi: i32) {
    let add = 1i32 << (shift - 1);
    for line in 0..lines {
        for j in 0..n {
            let mut sum = 0i32;
            for k in 0..nz {
                let c = src.get(k * stride + line).copied().unwrap_or(0);
                sum += i32::from(c) * i32::from(m.get(k * n + j).copied().unwrap_or(0));
            }
            if let Some(d) = dst.get_mut(line * n + j) {
                *d = clip(sum.wrapping_add(add) >> shift, lo, hi);
            }
        }
    }
}

/// 4-point DCT-8 / DST-7 special-cased factorisations of uavs3d (`itx_dct8_pb4`, `itx_dst7_pb4`).
fn stage4(dst7: bool, it: &[i8], src: &[i16], stride: usize, lines: usize, dst: &mut [i16], shift: u32, lo: i32, hi: i32) {
    let add = 1i32 << (shift - 1);
    let t = |i: usize| i32::from(it.get(i).copied().unwrap_or(0));
    for line in 0..lines {
        let c = |k: usize| i32::from(src.get(k * stride + line).copied().unwrap_or(0));
        let (o0, o1, o2, o3);
        if dst7 {
            let c0 = c(0) + c(2);
            let c1 = c(2) + c(3);
            let c2 = c(0) - c(3);
            let c3 = t(2) * c(1);
            o0 = t(0) * c0 + t(1) * c1 + c3;
            o1 = t(1) * c2 - t(0) * c1 + c3;
            o2 = t(2) * (c(0) - c(2) + c(3));
            o3 = t(1) * c0 + t(0) * c2 - c3;
        } else {
            let c0 = c(0) + c(3);
            let c1 = c(2) + c(0);
            let c2 = c(3) - c(2);
            let c3 = t(1) * c(1);
            o0 = t(3) * c0 + t(2) * c1 + c3;
            o1 = t(1) * (c(0) - c(2) - c(3));
            o2 = t(3) * c2 + t(2) * c0 - c3;
            o3 = t(3) * c1 - t(2) * c2 - c3;
        }
        for (j, v) in [o0, o1, o2, o3].into_iter().enumerate() {
            if let Some(d) = dst.get_mut(line * 4 + j) {
                *d = clip(v.wrapping_add(add) >> shift, lo, hi);
            }
        }
    }
}

fn xform(kind: usize, log2: usize, src: &[i16], stride: usize, lines: usize, dst: &mut [i16], shift: u32, lo: i32, hi: i32) {
    let n = 1usize << log2;
    let m = mats().get(log2 - 1).map_or(&[][..], |t| t[kind].as_slice());
    if n == 4 && kind != DCT2 {
        stage4(kind == DST7, m, src, stride, lines, dst, shift, lo, hi);
    } else {
        stage(m, n, n.min(32), src, stride, lines, dst, shift, lo, hi);
    }
}

fn sec_2nd_vert(src: &mut [i16], stride: usize) {
    let mut tmp = [[0i32; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            tmp[i][j] = i32::from(src.get(i * stride + j).copied().unwrap_or(0));
        }
    }
    for i in 0..4 {
        for j in 0..4 {
            let mut sum = 1 << 6;
            for k in 0..4 {
                sum += i32::from(ITRANS_C8[i][k]) * tmp[k][j];
            }
            if let Some(d) = src.get_mut(i * stride + j) {
                *d = clip(sum >> 7, -32768, 32767);
            }
        }
    }
}

fn sec_2nd_hor(src: &mut [i16], stride: usize) {
    let mut tmp = [[0i32; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            tmp[i][j] = i32::from(src.get(i * stride + j).copied().unwrap_or(0));
        }
    }
    for i in 0..4 {
        for j in 0..4 {
            let mut sum = 1 << 6;
            for k in 0..4 {
                sum += i32::from(ITRANS_C8[i][k]) * tmp[j][k];
            }
            if let Some(d) = src.get_mut(j * stride + i) {
                *d = clip(sum >> 7, -32768, 32767);
            }
        }
    }
}

fn sec_2nd_hor_ver(src: &mut [i16], stride: usize) {
    let mut tmp = [0i16; 16];
    for i in 0..4 {
        for j in 0..4 {
            let mut sum = 1 << 6;
            for k in 0..4 {
                sum += i32::from(ITRANS_C8[i][k]) * i32::from(src.get(j * stride + k).copied().unwrap_or(0));
            }
            tmp[i * 4 + j] = clip(sum >> 7, -32768, 32767);
        }
    }
    for i in 0..4 {
        for j in 0..4 {
            let mut sum = 1 << 6;
            for k in 0..4 {
                sum += i32::from(ITRANS_C8[i][k]) * i32::from(tmp[j * 4 + k]);
            }
            if let Some(d) = src.get_mut(i * stride + j) {
                *d = clip(sum >> 7, -32768, 32767);
            }
        }
    }
}

fn alt_4x4(src: &[i16], resi: &mut [i16], bd: i32) {
    let mut tmp = [[0i16; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            let mut sum = 1 << 4;
            for k in 0..4 {
                sum += i32::from(ITRANS_C4[i][k]) * i32::from(src.get(k * 4 + j).copied().unwrap_or(0));
            }
            tmp[i][j] = clip(sum >> 5, -32768, 32767);
        }
    }
    let shift = 22 - bd;
    let add = 1 << (shift - 1);
    let (lo, hi) = (-(1 << bd), (1 << bd) - 1);
    for i in 0..4 {
        for j in 0..4 {
            let mut sum = add;
            for k in 0..4 {
                sum += i32::from(ITRANS_C4[i][k]) * i32::from(tmp[j][k]);
            }
            if let Some(d) = resi.get_mut(j * 4 + i) {
                *d = clip(sum >> shift, lo, hi);
            }
        }
    }
}

/// `com_itrans`: inverse-transforms `coef` (modified by the secondary transform) into `resi`.
/// `nxn_luma` is set for a luma block of a position-based (NxN) transform split.
#[allow(clippy::too_many_arguments)]
pub(crate) fn itrans(nxn_luma: bool, blk_idx: usize, coef: &mut [i16], resi: &mut [i16], log2w: u32, log2h: u32, bd: i32, sec_vh: u32, alt4: bool) {
    let (lw, lh) = (log2w as usize, log2h as usize);
    if alt4 && lw == 2 && lh == 2 {
        alt_4x4(coef, resi, bd);
        return;
    }
    let stride = 1usize << lw;
    if sec_vh == 3 {
        sec_2nd_hor_ver(coef, stride);
    } else if sec_vh & 1 != 0 {
        sec_2nd_hor(coef, stride);
    } else if sec_vh >> 1 != 0 {
        sec_2nd_vert(coef, stride);
    }
    let (w, h) = (1usize << lw, 1usize << lh);
    let mut tmp = vec![0i16; w * h];
    let (hi15, lo15) = ((1 << 15) - 1, -(1 << 15));
    let (hib, lob) = ((1 << bd) - 1, -(1 << bd));
    let shift2 = (20 - bd) as u32;
    if nxn_luma {
        let kv = if blk_idx >> 1 != 0 { DST7 } else { DCT8 };
        let kh = if blk_idx & 1 != 0 { DST7 } else { DCT8 };
        xform(kv, lh, coef, w, w, &mut tmp, 5, lo15, hi15);
        xform(kh, lw, &tmp, h, h, resi, shift2, lob, hib);
    } else {
        xform(DCT2, lh, coef, w, w.min(32), &mut tmp, 5, lo15, hi15);
        xform(DCT2, lw, &tmp, h.min(32).max(h), h, resi, shift2, lob, hib);
    }
}
