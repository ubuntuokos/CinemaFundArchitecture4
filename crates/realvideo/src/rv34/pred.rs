// Ported from FFmpeg n7.1.1 libavcodec/h264pred.c and h264pred_template.c (the predictors
// `ff_h264_pred_init` selects for AV_CODEC_ID_RV40) (Copyright (c) 2003 Michael Niedermayer),
// LGPL-2.1-or-later; see docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! Intra prediction of RealVideo 3 / 4.

use crate::Plane;

pub(super) const VERT: i32 = 0;
pub(super) const HOR: i32 = 1;
pub(super) const DC: i32 = 2;
pub(super) const DIAG_DOWN_LEFT: i32 = 3;
pub(super) const DIAG_DOWN_RIGHT: i32 = 4;
pub(super) const VERT_RIGHT: i32 = 5;
pub(super) const HOR_DOWN: i32 = 6;
pub(super) const VERT_LEFT: i32 = 7;
pub(super) const HOR_UP: i32 = 8;
pub(super) const LEFT_DC: i32 = 9;
pub(super) const TOP_DC: i32 = 10;
pub(super) const DC_128: i32 = 11;
pub(super) const DIAG_DOWN_LEFT_NODOWN: i32 = 12;
pub(super) const HOR_UP_NODOWN: i32 = 13;
pub(super) const VERT_LEFT_NODOWN: i32 = 14;

pub(super) const DC8: i32 = 0;
pub(super) const HOR8: i32 = 1;
pub(super) const VERT8: i32 = 2;
pub(super) const PLANE8: i32 = 3;
pub(super) const LEFT_DC8: i32 = 4;
pub(super) const TOP_DC8: i32 = 5;
pub(super) const DC_128_8: i32 = 6;

fn put(p: &mut Plane, x: usize, y: usize, i: usize, j: usize, v: i32) {
    let w = p.w;
    if let Some(d) = p.d.get_mut((y + j) * w + x + i) {
        *d = v as u8;
    }
}

/// 4×4 prediction of the block at (`x`, `y`) with the four pixels right of its top edge `tr`.
pub(super) fn pred4x4(p: &mut Plane, x: usize, y: usize, mode: i32, tr: [i32; 4]) {
    let (xi, yi) = (x as isize, y as isize);
    let t: [i32; 8] = std::array::from_fn(|k| if k < 4 { p.at(xi + k as isize, yi - 1) } else { tr[k - 4] });
    let l: [i32; 8] = std::array::from_fn(|k| p.at(xi - 1, yi + k as isize));
    let lt = p.at(xi - 1, yi - 1);
    let mut b = [[0i32; 4]; 4]; // b[row][col]
    let fill = |b: &mut [[i32; 4]; 4], v: i32| *b = [[v; 4]; 4];
    match mode {
        VERT => b = [[t[0], t[1], t[2], t[3]]; 4],
        HOR => {
            for (j, r) in b.iter_mut().enumerate() {
                *r = [l[j]; 4];
            }
        }
        DC => fill(&mut b, (t[0] + t[1] + t[2] + t[3] + l[0] + l[1] + l[2] + l[3] + 4) >> 3),
        LEFT_DC => fill(&mut b, (l[0] + l[1] + l[2] + l[3] + 2) >> 2),
        TOP_DC => fill(&mut b, (t[0] + t[1] + t[2] + t[3] + 2) >> 2),
        DC_128 => fill(&mut b, 128),
        DIAG_DOWN_RIGHT => {
            b[3][0] = (l[3] + 2 * l[2] + l[1] + 2) >> 2;
            b[2][0] = (l[2] + 2 * l[1] + l[0] + 2) >> 2;
            b[3][1] = b[2][0];
            b[1][0] = (l[1] + 2 * l[0] + lt + 2) >> 2;
            b[2][1] = b[1][0];
            b[3][2] = b[1][0];
            b[0][0] = (l[0] + 2 * lt + t[0] + 2) >> 2;
            b[1][1] = b[0][0];
            b[2][2] = b[0][0];
            b[3][3] = b[0][0];
            b[0][1] = (lt + 2 * t[0] + t[1] + 2) >> 2;
            b[1][2] = b[0][1];
            b[2][3] = b[0][1];
            b[0][2] = (t[0] + 2 * t[1] + t[2] + 2) >> 2;
            b[1][3] = b[0][2];
            b[0][3] = (t[1] + 2 * t[2] + t[3] + 2) >> 2;
        }
        VERT_RIGHT => {
            b[0][0] = (lt + t[0] + 1) >> 1;
            b[2][1] = b[0][0];
            b[0][1] = (t[0] + t[1] + 1) >> 1;
            b[2][2] = b[0][1];
            b[0][2] = (t[1] + t[2] + 1) >> 1;
            b[2][3] = b[0][2];
            b[0][3] = (t[2] + t[3] + 1) >> 1;
            b[1][0] = (l[0] + 2 * lt + t[0] + 2) >> 2;
            b[3][1] = b[1][0];
            b[1][1] = (lt + 2 * t[0] + t[1] + 2) >> 2;
            b[3][2] = b[1][1];
            b[1][2] = (t[0] + 2 * t[1] + t[2] + 2) >> 2;
            b[3][3] = b[1][2];
            b[1][3] = (t[1] + 2 * t[2] + t[3] + 2) >> 2;
            b[2][0] = (lt + 2 * l[0] + l[1] + 2) >> 2;
            b[3][0] = (l[0] + 2 * l[1] + l[2] + 2) >> 2;
        }
        HOR_DOWN => {
            b[0][0] = (lt + l[0] + 1) >> 1;
            b[1][2] = b[0][0];
            b[0][1] = (l[0] + 2 * lt + t[0] + 2) >> 2;
            b[1][3] = b[0][1];
            b[0][2] = (lt + 2 * t[0] + t[1] + 2) >> 2;
            b[0][3] = (t[0] + 2 * t[1] + t[2] + 2) >> 2;
            b[1][0] = (l[0] + l[1] + 1) >> 1;
            b[2][2] = b[1][0];
            b[1][1] = (lt + 2 * l[0] + l[1] + 2) >> 2;
            b[2][3] = b[1][1];
            b[2][0] = (l[1] + l[2] + 1) >> 1;
            b[3][2] = b[2][0];
            b[2][1] = (l[0] + 2 * l[1] + l[2] + 2) >> 2;
            b[3][3] = b[2][1];
            b[3][0] = (l[2] + l[3] + 1) >> 1;
            b[3][1] = (l[1] + 2 * l[2] + l[3] + 2) >> 2;
        }
        DIAG_DOWN_LEFT | DIAG_DOWN_LEFT_NODOWN => {
            let nd = mode == DIAG_DOWN_LEFT_NODOWN;
            let ll = |k: usize| if nd && k > 3 { l[3] } else { l[k] };
            // Each anti-diagonal d = row + col.
            let v = |d: usize| -> i32 {
                if d == 6 {
                    (t[6] + t[7] + 1 + ll(6) + ll(7) + 1) >> 2
                } else if nd && d == 2 {
                    (t[2] + t[4] + 2 * t[3] + 2 + l[2] + 3 * l[3] + 2) >> 3
                } else if nd && d >= 3 {
                    (t[d] + t[d + 2] + 2 * t[d + 1] + 2 + l[3] * 4 + 2) >> 3
                } else {
                    (t[d] + t[d + 2] + 2 * t[d + 1] + 2 + ll(d) + ll(d + 2) + 2 * ll(d + 1) + 2) >> 3
                }
            };
            for (j, r) in b.iter_mut().enumerate() {
                for (i, c) in r.iter_mut().enumerate() {
                    *c = v(i + j);
                }
            }
            if nd {
                b[3][3] = (t[6] + t[7] + 1 + 2 * l[3] + 1) >> 2;
            }
        }
        VERT_LEFT | VERT_LEFT_NODOWN => {
            let l4 = if mode == VERT_LEFT_NODOWN { l[3] } else { l[4] };
            b[0][0] = (2 * t[0] + 2 * t[1] + l[1] + 2 * l[2] + l[3] + 4) >> 3;
            b[0][1] = (t[1] + t[2] + 1) >> 1;
            b[2][0] = b[0][1];
            b[0][2] = (t[2] + t[3] + 1) >> 1;
            b[2][1] = b[0][2];
            b[0][3] = (t[3] + t[4] + 1) >> 1;
            b[2][2] = b[0][3];
            b[2][3] = (t[4] + t[5] + 1) >> 1;
            b[1][0] = (t[0] + 2 * t[1] + t[2] + l[2] + 2 * l[3] + l4 + 4) >> 3;
            b[1][1] = (t[1] + 2 * t[2] + t[3] + 2) >> 2;
            b[3][0] = b[1][1];
            b[1][2] = (t[2] + 2 * t[3] + t[4] + 2) >> 2;
            b[3][1] = b[1][2];
            b[1][3] = (t[3] + 2 * t[4] + t[5] + 2) >> 2;
            b[3][2] = b[1][3];
            b[3][3] = (t[4] + 2 * t[5] + t[6] + 2) >> 2;
        }
        HOR_UP => {
            b[0][0] = (t[1] + 2 * t[2] + t[3] + 2 * l[0] + 2 * l[1] + 4) >> 3;
            b[0][1] = (t[2] + 2 * t[3] + t[4] + l[0] + 2 * l[1] + l[2] + 4) >> 3;
            b[0][2] = (t[3] + 2 * t[4] + t[5] + 2 * l[1] + 2 * l[2] + 4) >> 3;
            b[1][0] = b[0][2];
            b[0][3] = (t[4] + 2 * t[5] + t[6] + l[1] + 2 * l[2] + l[3] + 4) >> 3;
            b[1][1] = b[0][3];
            b[1][2] = (t[5] + 2 * t[6] + t[7] + 2 * l[2] + 2 * l[3] + 4) >> 3;
            b[2][0] = b[1][2];
            b[1][3] = (t[6] + 3 * t[7] + l[2] + 3 * l[3] + 4) >> 3;
            b[2][1] = b[1][3];
            b[2][3] = (l[3] + 2 * l[4] + l[5] + 2) >> 2;
            b[3][1] = b[2][3];
            b[3][0] = (t[6] + t[7] + l[3] + l[4] + 2) >> 2;
            b[2][2] = b[3][0];
            b[3][2] = (l[4] + l[5] + 1) >> 1;
            b[3][3] = (l[4] + 2 * l[5] + l[6] + 2) >> 2;
        }
        HOR_UP_NODOWN => {
            b[0][0] = (t[1] + 2 * t[2] + t[3] + 2 * l[0] + 2 * l[1] + 4) >> 3;
            b[0][1] = (t[2] + 2 * t[3] + t[4] + l[0] + 2 * l[1] + l[2] + 4) >> 3;
            b[0][2] = (t[3] + 2 * t[4] + t[5] + 2 * l[1] + 2 * l[2] + 4) >> 3;
            b[1][0] = b[0][2];
            b[0][3] = (t[4] + 2 * t[5] + t[6] + l[1] + 2 * l[2] + l[3] + 4) >> 3;
            b[1][1] = b[0][3];
            b[1][2] = (t[5] + 2 * t[6] + t[7] + 2 * l[2] + 2 * l[3] + 4) >> 3;
            b[2][0] = b[1][2];
            b[1][3] = (t[6] + 3 * t[7] + l[2] + 3 * l[3] + 4) >> 3;
            b[2][1] = b[1][3];
            b[2][3] = l[3];
            b[3][1] = l[3];
            b[3][0] = (t[6] + t[7] + 2 * l[3] + 2) >> 2;
            b[2][2] = b[3][0];
            b[3][2] = l[3];
            b[3][3] = l[3];
        }
        _ => fill(&mut b, 128),
    }
    for (j, r) in b.iter().enumerate() {
        for (i, &v) in r.iter().enumerate() {
            put(p, x, y, i, j, v);
        }
    }
}

/// 16×16 (`n = 16`, luma) or 8×8 (`n = 8`, chroma) prediction at (`x`, `y`).
pub(super) fn pred_block(p: &mut Plane, x: usize, y: usize, n: usize, mode: i32) {
    let (xi, yi) = (x as isize, y as isize);
    let top = |k: usize| p.at(xi + k as isize, yi - 1);
    let left = |k: usize| p.at(xi - 1, yi + k as isize);
    let mut out = vec![0i32; n * n];
    let fill = |out: &mut Vec<i32>, v: i32| out.iter_mut().for_each(|o| *o = v);
    match mode {
        VERT8 => {
            for (k, o) in out.iter_mut().enumerate() {
                *o = top(k % n);
            }
        }
        HOR8 => {
            for (k, o) in out.iter_mut().enumerate() {
                *o = left(k / n);
            }
        }
        DC_128_8 => fill(&mut out, 128),
        DC8 if n == 16 => {
            let s: i32 = (0..16).map(|k| left(k) + top(k)).sum();
            fill(&mut out, (s + 16) >> 5);
        }
        LEFT_DC8 if n == 16 => {
            let s: i32 = (0..16).map(left).sum();
            fill(&mut out, (s + 8) >> 4);
        }
        TOP_DC8 if n == 16 => {
            let s: i32 = (0..16).map(top).sum();
            fill(&mut out, (s + 8) >> 4);
        }
        // The RV40 chroma DC predictors (`pred8x8_*dc_rv40`).
        DC8 => {
            let s: i32 = (0..8).map(|k| left(k) + top(k)).sum();
            fill(&mut out, (s + 8) >> 4);
        }
        LEFT_DC8 => {
            let s: i32 = (0..8).map(left).sum();
            fill(&mut out, (s + 4) >> 3);
        }
        TOP_DC8 => {
            let s: i32 = (0..8).map(top).sum();
            fill(&mut out, (s + 4) >> 3);
        }
        PLANE8 if n == 16 => {
            // `pred16x16_plane_compat` with the RV40 rounding.
            let px = |i: isize, j: isize| p.at(xi + i, yi + j);
            let mut h = px(8, -1) - px(6, -1);
            let mut v = px(-1, 8) - px(-1, 6);
            for k in 2..=8isize {
                h += k as i32 * (px(7 + k, -1) - px(7 - k, -1));
                v += k as i32 * (px(-1, 7 + k) - px(-1, 7 - k));
            }
            let h = (h + (h >> 2)) >> 4;
            let v = (v + (v >> 2)) >> 4;
            let mut a = 16 * (px(-1, 15) + px(15, -1) + 1) - 7 * (v + h);
            for j in 0..16 {
                let mut b = a;
                a += v;
                for i in 0..16 {
                    out[j * 16 + i] = (b >> 5).clamp(0, 255);
                    b += h;
                }
            }
        }
        _ => fill(&mut out, 128),
    }
    for j in 0..n {
        for i in 0..n {
            put(p, x, y, i, j, out[j * n + i]);
        }
    }
}
