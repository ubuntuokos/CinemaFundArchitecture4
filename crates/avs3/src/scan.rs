// SPDX-License-Identifier: GPL-3.0-or-later
//! Coefficient scan orders (zig-zag over every block shape from 2x2 to 128x128).
//!
//! Derived from uavs3d 1.2 (commit 0e20d2c2), Copyright (c) 2018-2022 Peking University
//! Shenzhen Graduate School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation
//! Corporation, BSD 3-clause licence (docs/licensing.md). uavs3d stores these as a 64516-entry
//! table; the table is exactly the zig-zag order generated here.

use std::sync::OnceLock;

fn zigzag(w: usize, h: usize) -> Vec<u16> {
    let mut out = Vec::with_capacity(w * h);
    for d in 0..w + h - 1 {
        let lo = d.saturating_sub(h - 1);
        let hi = d.min(w - 1);
        if d % 2 == 1 {
            // top-right to bottom-left: x decreasing
            for x in (lo..=hi).rev() {
                out.push(((d - x) * w + x) as u16);
            }
        } else {
            for x in lo..=hi {
                out.push(((d - x) * w + x) as u16);
            }
        }
    }
    out
}

/// The scan for a block of `1 << log2w` by `1 << log2h` coefficients (each in `1..=7`).
pub(crate) fn scan(log2w: u32, log2h: u32) -> &'static [u16] {
    static T: OnceLock<Vec<Vec<u16>>> = OnceLock::new();
    let t = T.get_or_init(|| {
        let mut v = Vec::new();
        for lh in 1..=7usize {
            for lw in 1..=7usize {
                v.push(zigzag(1 << lw, 1 << lh));
            }
        }
        v
    });
    let (w, h) = (log2w.clamp(1, 7) as usize, log2h.clamp(1, 7) as usize);
    t.get((h - 1) * 7 + (w - 1)).map_or(&[], |v| v.as_slice())
}
