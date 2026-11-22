//! The inverse DCT, the loop filter and the half-pel averages of FFmpeg n7.1.1
//! `libavcodec/vp3dsp.c` (LGPL-2.1-or-later), in integer arithmetic.
//!
//! A block holds its coefficients transposed (`block[8 * column + row]`), as FFmpeg's does.

const C1S7: i32 = 64277;
const C2S6: i32 = 60547;
const C3S5: i32 = 54491;
const C4S4: i32 = 46341;
const C5S3: i32 = 36410;
const C6S2: i32 = 25080;
const C7S1: i32 = 12785;

/// `M(a, b)`: a 16-bit fixed-point product (wrapping, as C's does).
fn m(a: i32, b: i32) -> i32 {
    a.wrapping_mul(b) >> 16
}

fn clip8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// One 8-point inverse transform: inputs `x[0..8]`, outputs in order 0..8.
fn idct8(x: [i32; 8], add: i32) -> [i32; 8] {
    let a = m(C1S7, x[1]) + m(C7S1, x[7]);
    let b = m(C7S1, x[1]) - m(C1S7, x[7]);
    let c = m(C3S5, x[3]) + m(C5S3, x[5]);
    let d = m(C3S5, x[5]) - m(C5S3, x[3]);
    let ad = m(C4S4, a - c);
    let bd = m(C4S4, b - d);
    let cd = a + c;
    let dd = b + d;
    let e = m(C4S4, x[0] + x[4]) + add;
    let f = m(C4S4, x[0] - x[4]) + add;
    let g = m(C2S6, x[2]) + m(C6S2, x[6]);
    let h = m(C6S2, x[2]) - m(C2S6, x[6]);
    let ed = e - g;
    let gd = e + g;
    let add_ = f + ad;
    let bdd = bd - h;
    let fd = f - ad;
    let hd = bd + h;
    [gd + cd, add_ + hd, add_ - hd, ed + dd, ed - dd, fd + bdd, fd - bdd, gd - cd]
}

/// The 2-D transform of `block`: `out[8 * k + i]` is the residual of output column `i`, row `k`.
fn idct(block: &[i16; 64], put: bool) -> [i32; 64] {
    let mut t = [0i16; 64];
    // the first pass runs down the columns of the transposed block (stride 8) and stores 16 bits
    for i in 0..8 {
        let col: [i32; 8] = std::array::from_fn(|k| i32::from(block[k * 8 + i]));
        let o = idct8(col, 0);
        for k in 0..8 {
            t[k * 8 + i] = o[k] as i16;
        }
    }
    let mut out = [0i32; 64];
    for i in 0..8 {
        let row: [i32; 8] = std::array::from_fn(|k| i32::from(t[i * 8 + k]));
        let o = idct8(row, if put { 8 + 16 * 128 } else { 8 });
        for k in 0..8 {
            out[k * 8 + i] = o[k] >> 4;
        }
    }
    out
}

/// Intra block: the transform plus 128, into `dst` (an 8 x 8 area of `stride`-wide rows).
pub(crate) fn idct_put(dst: &mut [u8], stride: usize, block: &[i16; 64]) {
    let o = idct(block, true);
    for k in 0..8 {
        for i in 0..8 {
            if let Some(p) = dst.get_mut(k * stride + i) {
                *p = clip8(o[k * 8 + i]);
            }
        }
    }
}

/// Inter block: the transform added to the prediction already in `dst`.
pub(crate) fn idct_add(dst: &mut [u8], stride: usize, block: &[i16; 64]) {
    let o = idct(block, false);
    for k in 0..8 {
        for i in 0..8 {
            if let Some(p) = dst.get_mut(k * stride + i) {
                *p = clip8(i32::from(*p) + o[k * 8 + i]);
            }
        }
    }
}

/// Inter block with a DC coefficient only.
pub(crate) fn idct_dc_add(dst: &mut [u8], stride: usize, dc: i16) {
    let v = (i32::from(dc) + 15) >> 5;
    for k in 0..8 {
        for i in 0..8 {
            if let Some(p) = dst.get_mut(k * stride + i) {
                *p = clip8(i32::from(*p) + v);
            }
        }
    }
}

/// `ff_vp3dsp_set_bounding_values`: the filter's clamp table for indexes -127..=128 (stored from 0).
pub(crate) fn bounding_values(limit: i32) -> [i32; 256] {
    let mut t = [0i32; 256];
    let at = |x: i32| (x + 127) as usize;
    for x in 0..limit {
        t[at(-x)] = -x;
        t[at(x)] = x;
    }
    let mut value = limit;
    let mut x = limit;
    while x < 128 && value != 0 {
        t[at(x)] = value;
        t[at(-x)] = -value;
        x += 1;
        value -= 1;
    }
    if value != 0 {
        t[at(128)] = value;
    }
    t
}

fn filter_value(bv: &[i32; 256], fv: i32) -> i32 {
    bv.get((((fv + 4) >> 3) + 127) as usize).copied().unwrap_or(0)
}

/// Filters the horizontal edge above row `y0` of the plane, over `x0..x0 + 8`.
pub(crate) fn v_loop_filter(plane: &mut [u8], stride: usize, x0: usize, y0: usize, bv: &[i32; 256]) {
    if y0 < 2 {
        return;
    }
    for x in x0..x0 + 8 {
        let px = |dy: isize| plane.get((y0 as isize + dy) as usize * stride + x).copied().map_or(0, i32::from);
        let fv = (px(-2) - px(1)) + (px(0) - px(-1)) * 3;
        let f = filter_value(bv, fv);
        let (a, b) = ((y0 - 1) * stride + x, y0 * stride + x);
        if let Some(p) = plane.get_mut(a) {
            *p = clip8(i32::from(*p) + f);
        }
        if let Some(p) = plane.get_mut(b) {
            *p = clip8(i32::from(*p) - f);
        }
    }
}

/// Filters the vertical edge left of column `x0` of the plane, over `y0..y0 + 8`.
pub(crate) fn h_loop_filter(plane: &mut [u8], stride: usize, x0: usize, y0: usize, bv: &[i32; 256]) {
    if x0 < 2 {
        return;
    }
    for y in y0..y0 + 8 {
        let base = y * stride + x0;
        let px = |dx: isize| plane.get((base as isize + dx) as usize).copied().map_or(0, i32::from);
        let fv = (px(-2) - px(1)) + (px(0) - px(-1)) * 3;
        let f = filter_value(bv, fv);
        if let Some(p) = plane.get_mut(base - 1) {
            *p = clip8(i32::from(*p) + f);
        }
        if let Some(p) = plane.get_mut(base) {
            *p = clip8(i32::from(*p) - f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dc_only_block_is_flat() {
        let mut b = [0i16; 64];
        b[0] = 800;
        let o = idct(&b, true);
        assert!(o.iter().all(|&v| v == o[0]));
        let mut dst = [0u8; 64];
        idct_put(&mut dst, 8, &b);
        assert!(dst.iter().all(|&v| v == dst[0]) && dst[0] > 128);
    }

    #[test]
    fn bounding_table_shape() {
        let t = bounding_values(3);
        let at = |x: i32| t[(x + 127) as usize];
        assert_eq!((at(0), at(1), at(2), at(3), at(4), at(5), at(6)), (0, 1, 2, 3, 2, 1, 0));
        assert_eq!(at(-2), -2);
    }
}
