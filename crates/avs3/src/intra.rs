// SPDX-License-Identifier: BSD-3-Clause
// Derived from uavs3d (`source/decore/intra_pred.c`), Copyright (c) 2018-2022 Peking University Shenzhen Graduate
// School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation. See LICENSE.uavs3d.
//! Intra prediction of one block of one colour component.
//!
//! uavs3d keeps U and V interleaved; here every component has its own plane, so the interleaved
//! variants reduce to the luma formulas applied per component.

use crate::core::{Maps, Plane};
use crate::types::*;

const NB: usize = 160;

/// Reference samples of a block: index 0 is the corner, `1 + i` the above row, `-1 - j` the left column.
pub(crate) struct Nbr {
    b: [i32; 2 * NB + 1],
}

impl Nbr {
    #[inline]
    pub(crate) fn at(&self, i: isize) -> i32 {
        self.b[(i + NB as isize) as usize]
    }
    #[inline]
    fn set(&mut self, i: isize, v: i32) {
        self.b[(i + NB as isize) as usize] = v;
    }
    /// above sample `i`
    #[inline]
    pub(crate) fn up(&self, i: usize) -> i32 {
        self.at(1 + i as isize)
    }
    /// left sample `j`
    #[inline]
    pub(crate) fn left(&self, j: usize) -> i32 {
        self.at(-1 - j as isize)
    }
}

/// `com_get_nbr_l` / `com_get_nbr_c`: collects the reference samples of the `w`x`h` block at sample
/// position (`x`, `y`) of `plane`. `unit` is the number of samples per 4x4 luma block (4 or 2) and
/// `scup` the position of the block in the 4x4 map.
#[allow(clippy::too_many_arguments)]
pub(crate) fn get_nbr(plane: &Plane, x: usize, y: usize, w: usize, h: usize, unit: usize, avail: u8, scup: i32, maps: &Maps, bit_depth: u32) -> Nbr {
    let mut n = Nbr { b: [0; 2 * NB + 1] };
    let i_scu = maps.i_scu as i32;
    let pad_range = w.max(h) * 2 + 4;
    let default_val = 1i32 << (bit_depth - 1);
    let stride = plane.w;
    let px = |xx: usize, yy: usize| i32::from(plane.data[yy * stride + xx]);
    let w_scu = w / unit;
    let h_scu = h / unit;

    if avail & AVAIL_UP != 0 {
        for i in 0..w {
            n.set(1 + i as isize, px(x + i, y - 1));
        }
        let mut cnt = w;
        let mut map = scup - i_scu + w_scu as i32;
        for _ in 0..w_scu {
            if maps.scu[maps.ix(map)].coded {
                for k in 0..unit {
                    n.set(1 + (cnt + k) as isize, px(x + cnt + k, y - 1));
                }
                cnt += unit;
                map += 1;
            } else {
                break;
            }
        }
        let last = n.at(cnt as isize);
        for i in cnt..pad_range {
            n.set(1 + i as isize, last);
        }
    } else {
        for i in 0..pad_range {
            n.set(1 + i as isize, default_val);
        }
    }

    if avail & AVAIL_LE != 0 {
        for j in 0..h {
            n.set(-1 - j as isize, px(x - 1, y + j));
        }
        let mut cnt = h;
        let mut map = scup - 1 + h_scu as i32 * i_scu;
        for _ in 0..h_scu {
            if maps.scu[maps.ix(map)].coded {
                for k in 0..unit {
                    n.set(-1 - (cnt + k) as isize, px(x - 1, y + cnt + k));
                }
                cnt += unit;
                map += i_scu;
            } else {
                break;
            }
        }
        let last = n.at(-(cnt as isize));
        for j in cnt..pad_range {
            n.set(-1 - j as isize, last);
        }
    } else {
        for j in 0..pad_range {
            n.set(-1 - j as isize, default_val);
        }
    }

    let corner = if avail & AVAIL_UL != 0 {
        px(x - 1, y - 1)
    } else if avail & AVAIL_UP != 0 {
        px(x, y - 1)
    } else if avail & AVAIL_LE != 0 {
        px(x - 1, y)
    } else {
        default_val
    };
    n.set(0, corner);
    n
}

fn clip_pel(v: i32, bit_depth: u32) -> i32 {
    clip3(0, (1 << bit_depth) - 1, v)
}

fn dc(n: &Nbr, w: usize, h: usize, avail: u8, bit_depth: u32) -> i32 {
    let (lw, lh) = (log2_u(w) as i32, log2_u(h) as i32);
    let mut dc = 0i32;
    if avail & AVAIL_LE != 0 {
        for i in 0..h {
            dc += n.left(i);
        }
        if avail & AVAIL_UP != 0 {
            for j in 0..w {
                dc += n.up(j);
            }
            ((dc + ((w + h) >> 1) as i32) * (4096 / (w + h)) as i32) >> 12
        } else {
            (dc + (h >> 1) as i32) >> lh
        }
    } else if avail & AVAIL_UP != 0 {
        for j in 0..w {
            dc += n.up(j);
        }
        (dc + (w >> 1) as i32) >> lw
    } else {
        1 << (bit_depth - 1)
    }
}

const IB_MULT: [i32; 5] = [13, 17, 5, 11, 23];
const IB_SHIFT: [i32; 5] = [7, 10, 11, 15, 19];

/// Plane prediction, unclipped.
fn plane_pred(n: &Nbr, w: usize, h: usize, out: &mut [i32]) {
    let (w2, h2) = ((w >> 1) as isize, (h >> 1) as isize);
    let idx_w = log2_u(w) as usize - 2;
    let idx_h = log2_u(h) as usize - 2;
    let (im_h, is_h, im_v, is_v) = (IB_MULT[idx_w], IB_SHIFT[idx_w], IB_MULT[idx_h], IB_SHIFT[idx_h]);
    // C `src[k]`: k = 0 corner, k > 0 above k-1, k < 0 left -k-1
    let src = |k: isize| n.at(k);
    let mut coef_h = 0i32;
    let mut coef_v = 0i32;
    let rs = w2;
    for x in 1..=w2 {
        coef_h += x as i32 * (src(rs + x) - src(rs - x));
    }
    let rs = -h2;
    for y in 1..=h2 {
        coef_v += y as i32 * (src(rs - y) - src(rs + y));
    }
    let a = (src(-(h as isize)) + src(w as isize)) << 4;
    let b = (((coef_h << 5) * im_h) + (1 << (is_h - 1))) >> is_h;
    let c = (((coef_v << 5) * im_v) + (1 << (is_v - 1))) >> is_v;
    let mut temp = a - (h2 as i32 - 1) * c - (w2 as i32 - 1) * b + 16;
    for y in 0..h {
        let mut t2 = temp;
        for x in 0..w {
            out[y * w + x] = t2 >> 5;
            t2 += b;
        }
        temp += c;
    }
}

/// Bi-linear prediction, unclipped.
fn bi_pred(n: &Nbr, w: usize, h: usize, out: &mut [i32]) {
    let ishift_x = log2_u(w) as i32;
    let ishift_y = log2_u(h) as i32;
    let ishift = ishift_x.min(ishift_y);
    let ishift_xy = ishift_x + ishift_y + 1;
    let offset = 1i32 << (ishift_x + ishift_y);
    const TBL_WC: [i32; 6] = [-1, 21, 13, 7, 4, 2];
    let wc = TBL_WC[(ishift_x - ishift_y).unsigned_abs() as usize];
    let mut ref_up: Vec<i32> = (0..w).map(|x| n.up(x)).collect();
    let mut ref_le: Vec<i32> = (0..h).map(|y| n.left(y)).collect();
    let a = n.at(w as isize);
    let b = n.at(-(h as isize));
    let c = if w == h { (a + b + 1) >> 1 } else { (((a << ishift_x) + (b << ishift_y)) * wc + (1 << (ishift + 5))) >> (ishift + 6) };
    let wt = (c << 1) - a - b;
    let mut up = vec![0i32; w];
    for x in 0..w {
        up[x] = b - ref_up[x];
        ref_up[x] <<= ishift_y;
    }
    let mut le = vec![0i32; h];
    let mut wy = vec![0i32; h];
    let mut tmp = 0;
    for y in 0..h {
        le[y] = a - ref_le[y];
        ref_le[y] <<= ishift_x;
        wy[y] = tmp;
        tmp += wt;
    }
    for y in 0..h {
        let mut predx = ref_le[y];
        let mut wxy = 0;
        for x in 0..w {
            predx += le[y];
            ref_up[x] += up[x];
            out[y * w + x] = ((predx << ishift_y) + (ref_up[x] << ishift_x) + wxy + offset) >> ishift_xy;
            wxy += wy[y];
        }
    }
}

const DXDY: [[[i32; 2]; 33]; 2] = [
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

/// `getContextPixel`: (step, offset)
fn ctx_pixel(mode: usize, xy: usize, d: i32) -> (i32, i32) {
    let imult = DXDY[xy][mode][0];
    let ishift = DXDY[xy][mode][1];
    let dn = (d * imult) >> ishift;
    let offset = ((d * imult * 32) >> ishift) - dn * 32;
    (dn, offset)
}

fn ang_x(n: &Nbr, mode: usize, w: usize, h: usize, out: &mut [i32]) {
    let w2 = (w << 1) as i32;
    for j in 0..h {
        let (mut idx, offset) = ctx_pixel(mode, 0, j as i32 + 1);
        let mut pred_width = (w as i32).min(w2 - idx + 1);
        let (c1, c2, c3, c4) = (32 - offset, 64 - offset, 32 + offset, offset);
        let s = |k: i32| n.at(k as isize);
        let mut i = 0usize;
        while (i as i32) < pred_width {
            out[j * w + i] = (s(idx) * c1 + s(idx + 1) * c2 + s(idx + 2) * c3 + s(idx + 3) * c4 + 64) >> 7;
            i += 1;
            idx += 1;
        }
        if pred_width <= 0 {
            out[j * w] = (s(w2) * c1 + s(w2 + 1) * c2 + s(w2 + 2) * c3 + s(w2 + 3) * c4 + 64) >> 7;
            pred_width = 1;
            i = 1;
        }
        while i < w {
            out[j * w + i] = out[j * w + pred_width as usize - 1];
            i += 1;
        }
    }
}

fn ang_y(n: &Nbr, mode: usize, w: usize, h: usize, out: &mut [i32]) {
    let h2 = (h << 1) as i32;
    let mut offsets = [0i32; 64];
    let mut xsteps = [0i32; 64];
    for i in 0..w {
        let (s, o) = ctx_pixel(mode, 1, i as i32 + 1);
        xsteps[i] = s;
        offsets[i] = o;
    }
    for j in 0..h {
        for i in 0..w {
            let iy = j as i32 + xsteps[i];
            let idx = (-h2).max(-iy);
            let o = offsets[i];
            let s = |k: i32| n.at(k as isize);
            out[j * w + i] = (s(idx) * (32 - o) + s(idx - 1) * (64 - o) + s(idx - 2) * (32 + o) + s(idx - 3) * o + 64) >> 7;
        }
    }
}

fn ang_xy(n: &Nbr, mode: usize, w: usize, h: usize, out: &mut [i32]) {
    let mut xoffsets = [0i32; 64];
    let mut xsteps = [0i32; 64];
    for i in 0..w {
        let (s, o) = ctx_pixel(mode, 1, i as i32 + 1);
        xsteps[i] = s;
        xoffsets[i] = o;
    }
    let s = |k: i32| n.at(k as isize);
    for j in 0..h {
        let (step, offsetx) = ctx_pixel(mode, 0, j as i32 + 1);
        for (i, px) in (-step..).take(w).enumerate() {
            let iyy = j as i32 - xsteps[i];
            if iyy <= -1 {
                out[j * w + i] = (s(px + 2) * (32 - offsetx) + s(px + 1) * (64 - offsetx) + s(px) * (32 + offsetx) + s(px - 1) * offsetx + 64) >> 7;
            } else {
                let o = xoffsets[i];
                out[j * w + i] = (s(-iyy - 2) * (32 - o) + s(-iyy - 1) * (64 - o) + s(-iyy) * (32 + o) + s(-iyy + 1) * o + 64) >> 7;
            }
        }
    }
}

const IPF_PARAM: [[i32; 16]; 5] = [
    [24, 6, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [44, 25, 14, 8, 4, 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
    [40, 27, 19, 13, 9, 6, 4, 3, 2, 1, 0, 0, 0, 0, 0, 0],
    [36, 27, 21, 16, 12, 9, 7, 5, 4, 3, 0, 0, 0, 0, 0, 0],
    [52, 44, 37, 31, 26, 22, 18, 15, 13, 11, 0, 0, 0, 0, 0, 0],
];

/// `ipf_core` / `ipf_core_s16`: filters `pred` in place with the neighbouring samples.
fn ipf(n: &Nbr, pred: &mut [i32], ipm: u8, w: usize, h: usize, bit_depth: u32) {
    let mut idx_h = log2_u(w) as usize - 2;
    let mut idx_v = log2_u(h) as usize - 2;
    let mut ver_range = h.min(10);
    let mut hor_range = w.min(10);
    if idx_h > 4 {
        idx_h = 4;
        hor_range = 0;
    }
    if idx_v > 4 {
        idx_v = 4;
        ver_range = 0;
    }
    if (IPD_DIA_L..=IPD_DIA_R).contains(&ipm) {
        ver_range = 0;
    }
    if IPD_DIA_R < ipm {
        hor_range = 0;
    }
    let (hp, vp) = (&IPF_PARAM[idx_h], &IPF_PARAM[idx_v]);
    for row in 0..h {
        let coeff_top = if row < ver_range { vp[row] } else { 0 };
        for col in 0..w {
            let coeff_left = if col < hor_range { hp[col] } else { 0 };
            if coeff_left == 0 && coeff_top == 0 {
                pred[row * w + col] = clip_pel(pred[row * w + col], bit_depth);
                continue;
            }
            let coeff_cur = 64 - coeff_left - coeff_top;
            let v = (coeff_left * n.left(row) + coeff_top * n.up(col) + coeff_cur * pred[row * w + col] + 32) >> 6;
            pred[row * w + col] = clip_pel(v, bit_depth);
        }
    }
}

/// `com_ipred_l`: prediction of a luma block (or of a chroma block with a luma-derived mode) into `out`
/// (`w * h` samples, row by row).
#[allow(clippy::too_many_arguments)]
pub(crate) fn predict(n: &Nbr, ipm: u8, w: usize, h: usize, bit_depth: u32, avail: u8, use_ipf: bool, out: &mut [i32]) {
    let out = &mut out[..w * h];
    match ipm {
        IPD_PLN | IPD_BI => {
            if ipm == IPD_PLN {
                plane_pred(n, w, h, out);
            } else {
                bi_pred(n, w, h, out);
            }
            if use_ipf {
                for v in out.iter_mut() {
                    *v = i32::from(*v as i16);
                }
                ipf(n, out, ipm, w, h, bit_depth);
            } else {
                for v in out.iter_mut() {
                    *v = clip_pel(*v, bit_depth);
                }
            }
        }
        _ => {
            match ipm {
                IPD_VER => {
                    for y in 0..h {
                        for x in 0..w {
                            out[y * w + x] = n.up(x);
                        }
                    }
                }
                IPD_HOR => {
                    for y in 0..h {
                        let v = n.left(y);
                        out[y * w..(y + 1) * w].fill(v);
                    }
                }
                IPD_DC => out.fill(dc(n, w, h, avail, bit_depth)),
                m if m > IPD_BI && m < IPD_VER => ang_x(n, m as usize, w, h, out),
                m if m > IPD_VER && m < IPD_HOR => ang_xy(n, m as usize, w, h, out),
                m => ang_y(n, m as usize, w, h, out),
            }
            if use_ipf {
                ipf(n, out, ipm, w, h, bit_depth);
            }
        }
    }
}

/// `com_ipred_c` for one component (everything but TSCPM).
pub(crate) fn predict_chroma(n: &Nbr, ipm_c: u8, ipm: u8, w: usize, h: usize, bit_depth: u32, avail: u8, out: &mut [i32]) {
    let mut ipm_c = ipm_c;
    if ipm_c == IPD_DM_C && matches!(ipm, IPD_VER | IPD_HOR | IPD_DC | IPD_BI) {
        ipm_c = match ipm {
            IPD_VER => IPD_VER_C,
            IPD_HOR => IPD_HOR_C,
            IPD_DC => IPD_DC_C,
            _ => IPD_BI_C,
        };
    }
    match ipm_c {
        IPD_DM_C => predict(n, ipm, w, h, bit_depth, avail, false, out),
        IPD_DC_C => predict(n, IPD_DC, w, h, bit_depth, avail, false, out),
        IPD_HOR_C => predict(n, IPD_HOR, w, h, bit_depth, avail, false, out),
        IPD_VER_C => predict(n, IPD_VER, w, h, bit_depth, avail, false, out),
        IPD_BI_C => predict(n, IPD_BI, w, h, bit_depth, avail, false, out),
        _ => {}
    }
}

const TSCPM_DIV: [i32; 64] = [
    65536, 32768, 21845, 16384, 13107, 10922, 9362, 8192, 7281, 6553, 5957, 5461, 5041, 4681, 4369, 4096, 3855, 3640, 3449, 3276, 3120, 2978, 2849, 2730, 2621,
    2520, 2427, 2340, 2259, 2184, 2114, 2048, 1985, 1927, 1872, 1820, 1771, 1724, 1680, 1638, 1598, 1560, 1524, 1489, 1456, 1424, 1394, 1365, 1337, 1310, 1285,
    1260, 1236, 1213, 1191, 1170, 1149, 1129, 1110, 1092, 1074, 1057, 1040, 1024,
];

fn luma_border(idx: usize, above: bool, cw: usize, ch: usize, above_avail: bool, left_avail: bool, luma: &Nbr) -> i32 {
    if above {
        if above_avail && idx < (cw << 1) {
            let p = |i: isize| luma.at(1 + i);
            let i = idx as isize;
            if idx == 0 && !left_avail {
                return (3 * p(i) + p(i + 1) + 2) >> 2;
            }
            return (2 * p(i) + p(i - 1) + p(i + 1) + 2) >> 2;
        }
    } else if left_avail && idx < (ch << 1) {
        let j = idx as isize;
        let p = |k: isize| luma.at(-1 + k);
        return (p(-j) + p(-j - 1) + 1) >> 1;
    }
    -1
}

/// `ipred_tscpm`: the U and V predictions from the reconstructed luma. `luma_nb` are the luma reference
/// samples of the (2w x 2h) luma block, `luma_rec` its samples (stride `2 * w`), `nu`/`nv` the chroma ones.
#[allow(clippy::too_many_arguments)]
pub(crate) fn tscpm(
    nu: &Nbr,
    nv: &Nbr,
    luma_nb: &Nbr,
    luma_rec: &[i32],
    w: usize,
    h: usize,
    above: bool,
    left: bool,
    bit_depth: u32,
    out_u: &mut [i32],
    out_v: &mut [i32],
) {
    let min_dim = if left && above {
        h.min(w)
    } else if left {
        h
    } else {
        w
    };
    let num_steps = min_dim;
    let mut y_max = [0i32; 2];
    let mut x_max = i32::MIN;
    let mut y_min = [0i32; 2];
    let mut x_min = i32::MAX;
    let mut ref_luma = [-1i32; 4];
    let mut ref_c = [[-1i32; 4]; 2];
    let cu = |k: usize, i: isize| if k == 0 { nu.at(i) } else { nv.at(i) };

    if above {
        let idx = (((num_steps - 1) * w) / min_dim) << 1;
        ref_luma[0] = luma_border(0, true, w, h, above, left, luma_nb);
        ref_c[0][0] = cu(0, 1);
        ref_c[1][0] = cu(1, 1);
        ref_luma[1] = luma_border(idx, true, w, h, above, left, luma_nb);
        ref_c[0][1] = cu(0, 1 + (idx >> 1) as isize);
        ref_c[1][1] = cu(1, 1 + (idx >> 1) as isize);
        if !left && w >= 4 {
            let step = w >> 1;
            for i in 0..4 {
                ref_luma[i] = luma_border(i * step, true, w, h, above, left, luma_nb);
                ref_c[0][i] = cu(0, 1 + ((i * step) >> 1) as isize);
                ref_c[1][i] = cu(1, 1 + ((i * step) >> 1) as isize);
            }
        }
    }
    if left {
        let idx = (((num_steps - 1) * h) / min_dim) << 1;
        ref_luma[2] = luma_border(0, false, w, h, above, left, luma_nb);
        ref_c[0][2] = cu(0, -1);
        ref_c[1][2] = cu(1, -1);
        ref_luma[3] = luma_border(idx, false, w, h, above, left, luma_nb);
        ref_c[0][3] = cu(0, -1 - (idx >> 1) as isize);
        ref_c[1][3] = cu(1, -1 - (idx >> 1) as isize);
        if !above && h >= 4 {
            let step = h >> 1;
            for i in 0..4 {
                ref_luma[i] = luma_border(i * step, false, w, h, above, left, luma_nb);
                ref_c[0][i] = cu(0, -1 - ((i * step) >> 1) as isize);
                ref_c[1][i] = cu(1, -1 - ((i * step) >> 1) as isize);
            }
        }
    }

    if (above && left) || (above && !left && w >= 4) || (left && !above && h >= 4) {
        let mut min_grp = [0usize, 2];
        let mut max_grp = [1usize, 3];
        if ref_luma[min_grp[0]] > ref_luma[min_grp[1]] {
            min_grp.swap(0, 1);
        }
        if ref_luma[max_grp[0]] > ref_luma[max_grp[1]] {
            max_grp.swap(0, 1);
        }
        if ref_luma[min_grp[0]] > ref_luma[max_grp[1]] {
            std::mem::swap(&mut min_grp, &mut max_grp);
        }
        if ref_luma[min_grp[1]] > ref_luma[max_grp[0]] {
            std::mem::swap(&mut min_grp[1], &mut max_grp[0]);
        }
        x_min = (ref_luma[min_grp[0]] + ref_luma[min_grp[1]] + 1) >> 1;
        x_max = (ref_luma[max_grp[0]] + ref_luma[max_grp[1]] + 1) >> 1;
        for k in 0..2 {
            y_min[k] = (ref_c[k][min_grp[0]] + ref_c[k][min_grp[1]] + 1) >> 1;
            y_max[k] = (ref_c[k][max_grp[0]] + ref_c[k][max_grp[1]] + 1) >> 1;
        }
    } else if above || left {
        let range = if above { 0..2 } else { 2..4 };
        for k in range {
            if ref_luma[k] > x_max {
                x_max = ref_luma[k];
                y_max = [ref_c[0][k], ref_c[1][k]];
            }
            if ref_luma[k] < x_min {
                x_min = ref_luma[k];
                y_min = [ref_c[0][k], ref_c[1][k]];
            }
        }
    }

    let (mut a, mut b) = ([0i32; 2], [0i32; 2]);
    let mut shift_out = 16;
    if left || above {
        let mut diff = x_max - x_min;
        let mut add = 0;
        let mut shift = 0;
        if diff > 64 {
            shift = if bit_depth > 8 { bit_depth as i32 - 6 } else { 2 };
            add = if shift != 0 { 1 << (shift - 1) } else { 0 };
            diff = (diff + add) >> shift;
        }
        if diff > 0 {
            for k in 0..2 {
                a[k] = ((y_max[k] - y_min[k]) * TSCPM_DIV[diff as usize - 1] + add) >> shift;
            }
        }
        for k in 0..2 {
            b[k] = y_min[k] - ((i64::from(a[k]) * i64::from(x_min)) >> shift_out) as i32;
        }
    } else {
        a = [0, 0];
        b = [1 << (bit_depth - 1), 1 << (bit_depth - 1)];
        shift_out = 0;
    }

    let max_result = (1 << bit_depth) - 1;
    let (lw, lh) = (w << 1, h << 1);
    let shift_out = shift_out.max(0);
    for (k, out) in [out_u, out_v].into_iter().enumerate() {
        let mut t = vec![0i32; lw * lh];
        for (i, v) in t.iter_mut().enumerate() {
            *v = clip3(0, max_result, ((i64::from(a[k]) * i64::from(luma_rec[i])) >> shift_out) as i32 + b[k]);
        }
        for j in 0..h {
            let r0 = 2 * j * lw;
            let r1 = r0 + lw;
            for i in 0..w {
                let i2 = i << 1;
                out[j * w + i] = if i == 0 {
                    (t[r0] + t[r1] + 1) >> 1
                } else {
                    (t[r0 + i2] * 2 + t[r0 + i2 + 1] + t[r0 + i2 - 1] + t[r1 + i2] * 2 + t[r1 + i2 + 1] + t[r1 + i2 - 1] + 4) >> 3
                };
            }
        }
    }
}
