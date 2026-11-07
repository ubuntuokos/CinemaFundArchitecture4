// SPDX-License-Identifier: LGPL-2.1-or-later
// The scan orders, area maps, quantisation step table and class offsets below are ported from
// libdv 1.0.0 (libdv/parse.c, libdv/quant.c, libdv/weighting.c; Copyright (C) Charles 'Buck'
// Krasic, Erik Walthinsen - April 2000; LGPL-2.1-or-later, "either version 2.1, or (at your
// option) any later version"). They are the tables of IEC 61834-2 / SMPTE 314M. Re-expressed
// for CFA4; see crates/dv/README.md and docs/licensing.md §5.

//! Inverse quantisation and inverse weighting of DV DCT blocks.
//!
//! A block carries a 9-bit DC value (weight 1/4, not quantised), a DCT mode (8-8 or 2-4-8), a
//! class number (0-3) and AC amplitudes in zigzag order. The macroblock's quantisation number
//! QNO plus a per-class offset selects a row of quantisation steps (a power of two per area);
//! class 3 doubles every step. The weights `W(h, v)` of the forward transform are undone here,
//! so the IDCT is a plain orthonormal one.

use std::sync::OnceLock;

/// Zigzag scan index → natural position (`row * 8 + column`, row = vertical frequency) for
/// the 8-8 DCT.
pub(crate) const SCAN_88: [u8; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29,
    22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// Zigzag scan index → natural position for the 2-4-8 DCT (rows 0-3: sum of the two fields,
/// rows 4-7: their difference).
pub(crate) const SCAN_248: [u8; 64] = [
    0, 32, 1, 33, 8, 40, 2, 34, 9, 41, 16, 48, 24, 56, 17, 49, 10, 42, 3, 35, 4, 36, 11, 43, 18, 50, 25, 57, 26, 58, 19, 51, 12, 44, 5, 37, 6, 38, 13, 45, 20,
    52, 27, 59, 28, 60, 21, 53, 14, 46, 7, 39, 15, 47, 22, 54, 29, 61, 30, 62, 23, 55, 31, 63,
];

/// Area (0-3) of each natural position, 8-8 DCT (position 0, the DC, is unused).
const AREAS_88: [u8; 64] = [
    0, 0, 0, 1, 1, 1, 2, 2, //
    0, 0, 1, 1, 1, 2, 2, 2, //
    0, 1, 1, 1, 2, 2, 2, 3, //
    1, 1, 1, 2, 2, 2, 3, 3, //
    1, 1, 2, 2, 2, 3, 3, 3, //
    1, 2, 2, 2, 3, 3, 3, 3, //
    2, 2, 2, 3, 3, 3, 3, 3, //
    2, 2, 3, 3, 3, 3, 3, 3,
];

/// Area of each natural position, 2-4-8 DCT.
const AREAS_248: [u8; 64] = [
    0, 0, 1, 1, 1, 2, 2, 3, //
    0, 1, 1, 2, 2, 2, 3, 3, //
    1, 1, 2, 2, 2, 3, 3, 3, //
    1, 2, 2, 2, 3, 3, 3, 3, //
    0, 0, 1, 1, 2, 2, 2, 3, //
    0, 1, 1, 2, 2, 2, 3, 3, //
    1, 1, 2, 2, 2, 3, 3, 3, //
    1, 2, 2, 3, 3, 3, 3, 3,
];

/// log2 of the quantisation step per area, indexed by `QNO + CLASS_OFFSET[class]`.
const STEP_SHIFTS: [[u8; 4]; 22] = [
    [3, 3, 4, 4],
    [3, 3, 4, 4],
    [2, 3, 3, 4],
    [2, 3, 3, 4],
    [2, 2, 3, 3],
    [2, 2, 3, 3],
    [1, 2, 2, 3],
    [1, 2, 2, 3],
    [1, 1, 2, 2],
    [1, 1, 2, 2],
    [0, 1, 1, 2],
    [0, 1, 1, 2],
    [0, 0, 1, 1],
    [0, 0, 1, 1],
    [0, 0, 0, 1],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
];

const CLASS_OFFSET: [u8; 4] = [6, 3, 0, 1];

/// Per-mode factors that turn a quantised amplitude into a DCT coefficient of the orthonormal
/// transform: `step * 1 / W`.
pub(crate) struct Tables {
    /// `[mode][row][position]`: `2^shift / W(position)` for every class/QNO row (class 3's
    /// extra doubling is applied by the caller).
    factor: [[[f32; 64]; 22]; 2],
}

/// The DV weighting constants `w(i)` (IEC 61834-2): the forward transform multiplies
/// coefficient `(h, v)` of an 8-8 block by `w(h) w(v) / 2` (the DC by 1/4), and of a 2-4-8
/// block by `w(h) w(2 v') / 2` with `v'` the row within its half.
fn w() -> [f64; 8] {
    let cs = |m: f64| (m * std::f64::consts::PI / 16.0).cos();
    [
        1.0,
        cs(4.0) / (4.0 * cs(7.0) * cs(2.0)),
        cs(4.0) / (2.0 * cs(6.0)),
        1.0 / (2.0 * cs(5.0)),
        7.0 / 8.0,
        cs(4.0) / cs(3.0),
        cs(4.0) / cs(2.0),
        cs(4.0) / cs(1.0),
    ]
}

fn build() -> Tables {
    let w = w();
    let mut inv = [[0f64; 64]; 2];
    for p in 0..64 {
        let (row, col) = (p / 8, p % 8);
        inv[0][p] = 2.0 / (w[col] * w[row]);
        inv[1][p] = 2.0 / (w[col] * w[2 * (row % 4)]);
    }
    let mut factor = [[[0f32; 64]; 22]; 2];
    for (mode, areas) in [AREAS_88, AREAS_248].iter().enumerate() {
        for (row, shifts) in STEP_SHIFTS.iter().enumerate() {
            for p in 0..64 {
                let shift = shifts[usize::from(areas[p] & 3)];
                factor[mode][row][p] = (f64::from(1u32 << shift) * inv[mode][p]) as f32;
            }
        }
    }
    Tables { factor }
}

pub(crate) fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(build)
}

/// Dequantise one block: `amps` are the AC amplitudes in zigzag order (index 0 unused), `dc`
/// the signed 9-bit DC value. Returns natural-order coefficients of the orthonormal DCT.
pub(crate) fn dequantise(dc: i32, amps: &[i16; 64], mode248: bool, class: u8, qno: u8) -> [f32; 64] {
    let t = tables();
    let mode = usize::from(mode248);
    let row = (usize::from(qno & 15) + usize::from(CLASS_OFFSET[usize::from(class & 3)])).min(21);
    let extra = if class & 3 == 3 { 2.0 } else { 1.0 };
    let scan = if mode248 { &SCAN_248 } else { &SCAN_88 };
    let f = &t.factor[mode][row];
    let mut out = [0f32; 64];
    // DC: weight 1/4 (both modes), no quantisation
    out[0] = (dc * 4) as f32;
    for (zz, &a) in amps.iter().enumerate().skip(1) {
        if a != 0 {
            let p = usize::from(scan[zz] & 63);
            out[p] = f32::from(a) * f[p] * extra;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_are_permutations() {
        for s in [SCAN_88, SCAN_248] {
            let mut seen = [false; 64];
            for p in s {
                assert!(!seen[p as usize]);
                seen[p as usize] = true;
            }
        }
    }

    #[test]
    fn areas_follow_zigzag_bands() {
        // area boundaries in zigzag order: 1-5, 6-20, 21-42, 43-63
        for (scan, areas) in [(SCAN_88, AREAS_88), (SCAN_248, AREAS_248)] {
            for zz in 1..64 {
                let want = match zz {
                    1..=5 => 0,
                    6..=20 => 1,
                    21..=42 => 2,
                    _ => 3,
                };
                assert_eq!(areas[scan[zz] as usize], want, "zz {zz}");
            }
        }
    }

    #[test]
    fn dc_scale() {
        let d = dequantise(-3, &[0; 64], false, 0, 0);
        assert_eq!(d[0], -12.0);
    }
}
