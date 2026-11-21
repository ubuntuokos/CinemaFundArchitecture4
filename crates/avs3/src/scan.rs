// SPDX-License-Identifier: BSD-3-Clause
// Derived from uavs3d (`source/decore/com_table.c`, `g_tbl_scan`), Copyright (c) 2018-2022 Peking University
// Shenzhen Graduate School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation.
//! Coefficient scan orders (generated; they equal uavs3d's table for every block size).

use std::sync::OnceLock;

/// The scan of a `w`x`h` block: positions `y * w + x` in coding order.
fn generate(h: usize, w: usize) -> Vec<u16> {
    let mut out = Vec::with_capacity(w * h);
    if h == 2 {
        for x in (0..w).step_by(2) {
            for y in 0..2 {
                for dx in 0..2 {
                    out.push((y * w + x + dx) as u16);
                }
            }
        }
        return out;
    }
    for d in 0..(w + h - 1) {
        if d % 2 == 1 {
            let mut x = d.min(w - 1) as isize;
            let mut y = d as isize - x;
            while x >= 0 && (y as usize) < h {
                out.push((y as usize * w + x as usize) as u16);
                x -= 1;
                y += 1;
            }
        } else {
            let mut y = d.min(h - 1) as isize;
            let mut x = d as isize - y;
            while y >= 0 && (x as usize) < w {
                out.push((y as usize * w + x as usize) as u16);
                x += 1;
                y -= 1;
            }
        }
    }
    out
}

static SCANS: OnceLock<Vec<Vec<u16>>> = OnceLock::new();

/// The scan for `1 << log2_w` x `1 << log2_h` (both 1..=6 here, up to 7 in the tables).
pub(crate) fn scan(log2_w: u32, log2_h: u32) -> &'static [u16] {
    let all = SCANS.get_or_init(|| {
        let mut v = Vec::new();
        for lh in 1..8 {
            for lw in 1..8 {
                v.push(generate(1 << lh, 1 << lw));
            }
        }
        v
    });
    &all[(log2_h as usize - 1) * 7 + (log2_w as usize - 1)]
}
