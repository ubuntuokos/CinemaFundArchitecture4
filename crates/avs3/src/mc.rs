// SPDX-License-Identifier: BSD-3-Clause
// Derived from uavs3d (`source/decore/inter_pred.c`), Copyright (c) 2018-2022 Peking University Shenzhen
// Graduate School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation. See LICENSE.uavs3d.
//! Sub-sample interpolation of one block of one colour component.

use crate::core::Plane;
use crate::mctab::{CHROMA, CHROMA_HP, LUMA, LUMA_HP};
use crate::types::clip3;

/// Fetches the `(w + taps - 1) x (h + taps - 1)` window of `plane` starting at (`x0`, `y0`), the
/// coordinates clamped to the plane (the reference pictures are edge-extended).
fn window(plane: &Plane, x0: i32, y0: i32, ww: usize, wh: usize) -> Vec<i32> {
    let mut v = Vec::with_capacity(ww * wh);
    let (pw, ph) = (plane.w as i32, plane.h as i32);
    for j in 0..wh as i32 {
        let y = clip3(0, ph - 1, y0 + j) as usize;
        let row = &plane.data[y * plane.w..(y + 1) * plane.w];
        for i in 0..ww as i32 {
            v.push(i32::from(row[clip3(0, pw - 1, x0 + i) as usize]));
        }
    }
    v
}

#[allow(clippy::too_many_arguments)]
fn filter(src: &[i32], ww: usize, w: usize, h: usize, taps: usize, cx: &[i32], cy: &[i32], dx: bool, dy: bool, bit_depth: u32, out: &mut [i32]) {
    // `src` is the window whose first sample is `taps / 2 - 1` samples before the block (both directions)
    let off = taps / 2 - 1;
    let max_val = (1i32 << bit_depth) - 1;
    if !dx && !dy {
        for j in 0..h {
            for i in 0..w {
                out[j * w + i] = src[(j + off) * ww + i + off];
            }
        }
    } else if !dy {
        for j in 0..h {
            for i in 0..w {
                let mut sum = 0;
                for t in 0..taps {
                    sum += src[(j + off) * ww + i + t] * cx[t];
                }
                out[j * w + i] = ((sum + 32) >> 6).clamp(0, max_val);
            }
        }
    } else if !dx {
        for j in 0..h {
            for i in 0..w {
                let mut sum = 0;
                for t in 0..taps {
                    sum += src[(j + t) * ww + i + off] * cy[t];
                }
                out[j * w + i] = ((sum + 32) >> 6).clamp(0, max_val);
            }
        }
    } else {
        let (shift1, shift2) = if bit_depth == 8 { (0, 12) } else { (2, 10) };
        let add1 = (1i32 << shift1) >> 1;
        let add2 = 1i32 << (shift2 - 1);
        let wh = h + taps - 1;
        let mut tmp = vec![0i32; w * wh];
        for j in 0..wh {
            for i in 0..w {
                let mut sum = 0;
                for t in 0..taps {
                    sum += src[j * ww + i + t] * cx[t];
                }
                tmp[j * w + i] = i32::from(((sum + add1) >> shift1) as i16);
            }
        }
        for j in 0..h {
            for i in 0..w {
                let mut sum = 0;
                for t in 0..taps {
                    sum += tmp[(j + t) * w + i] * cy[t];
                }
                out[j * w + i] = ((sum + add2) >> shift2).clamp(0, max_val);
            }
        }
    }
}

/// `mc_core_luma`: `x_pos`/`y_pos` in quarter samples (or sixteenth samples with `hp`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn mc_luma(plane: &Plane, x_pos: i32, y_pos: i32, w: usize, h: usize, max_posx: i32, max_posy: i32, bit_depth: u32, hp: bool, out: &mut [i32]) {
    let (dx, dy, xi, yi) = if hp { (x_pos & 15, y_pos & 15, x_pos >> 4, y_pos >> 4) } else { (x_pos & 3, y_pos & 3, x_pos >> 2, y_pos >> 2) };
    let xi = clip3(-132, max_posx, xi);
    let yi = clip3(-132, max_posy, yi);
    let cx: &[i32] = if hp { &LUMA_HP[dx as usize] } else { &LUMA[dx as usize] };
    let cy: &[i32] = if hp { &LUMA_HP[dy as usize] } else { &LUMA[dy as usize] };
    let ww = w + 7;
    let win = window(plane, xi - 3, yi - 3, ww, h + 7);
    filter(&win, ww, w, h, 8, cx, cy, dx != 0, dy != 0, bit_depth, out);
}

/// `mc_core_chroma` for one component: positions in eighth (or thirty-second with `hp`) samples.
#[allow(clippy::too_many_arguments)]
pub(crate) fn mc_chroma(plane: &Plane, x_pos: i32, y_pos: i32, w: usize, h: usize, max_posx: i32, max_posy: i32, bit_depth: u32, hp: bool, out: &mut [i32]) {
    let (dx, dy, xi, yi) = if hp { (x_pos & 31, y_pos & 31, x_pos >> 5, y_pos >> 5) } else { (x_pos & 7, y_pos & 7, x_pos >> 3, y_pos >> 3) };
    let xi = clip3(-66, max_posx, xi);
    let yi = clip3(-66, max_posy, yi);
    let cx: &[i32] = if hp { &CHROMA_HP[dx as usize] } else { &CHROMA[dx as usize] };
    let cy: &[i32] = if hp { &CHROMA_HP[dy as usize] } else { &CHROMA[dy as usize] };
    let ww = w + 3;
    let win = window(plane, xi - 1, yi - 1, ww, h + 3);
    filter(&win, ww, w, h, 4, cx, cy, dx != 0, dy != 0, bit_depth, out);
}
