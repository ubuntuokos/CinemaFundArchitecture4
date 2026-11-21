// SPDX-License-Identifier: BSD-3-Clause
// Derived from uavs3d (`source/decore/alf.c`), Copyright (c) 2018-2022 Peking University Shenzhen Graduate
// School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation. See LICENSE.uavs3d.
//! Adaptive loop filter.
//!
//! The filter of an LCU covers the rows shifted up by four (the virtual boundary): rows `y0 - 4 .. y0 + h - 4`
//! (the first four rows are left out when the LCU row is the first of the picture or of a patch that does not
//! filter across the patch border, the last four rows when it is the last row of the picture). Taps above and below
//! that region repeat its first and last row; the four corner samples use the corner rule of `alf_one_blk_fix`.
//! The source is the picture after SAO.

use crate::core::{Core, Plane};
use crate::types::clip3;

const SHIFT: usize = 4;
const TAB_REGION: [usize; 16] = [0, 1, 4, 5, 15, 2, 3, 6, 14, 11, 10, 7, 13, 12, 9, 8];

struct Region {
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
}

fn alf_region(src: &Plane, dst: &mut Plane, r: &Region, c: &[i32; 9], bit_depth: u32) {
    if r.w == 0 || r.h == 0 {
        return;
    }
    let max = (1i32 << bit_depth) - 1;
    let (w, h) = (r.w as isize, r.h as isize);
    let pw = src.w as isize;
    let t = |x: isize, y: isize| -> i32 {
        let yy = r.y0 + y.clamp(0, h - 1) as usize;
        let xx = (r.x0 as isize + x).clamp(0, pw - 1) as usize;
        src.data[yy * src.w + xx] as i32
    };
    for i in 0..h {
        for j in 0..w {
            // 0: none, 1: top left (c2 second), 2: top right (c4 second), 3: bottom left (c4 first), 4: bottom right (c2 first)
            let corner = match (j == 0, j == w - 1, i == 0, i == h - 1) {
                (true, _, true, _) => 1,
                (_, true, true, _) => 2,
                (true, _, _, true) => 3,
                (_, true, _, true) => 4,
                _ => 0,
            };
            let ctr = t(j, i);
            let mut c2a = t(j + 1, i + 1);
            let mut c2b = t(j - 1, i - 1);
            let mut c4a = t(j - 1, i + 1);
            let mut c4b = t(j + 1, i - 1);
            match corner {
                1 => c2b = ctr,
                2 => c4b = ctr,
                3 => c4a = ctr,
                4 => c2a = ctr,
                _ => {}
            }
            let mut sum = c[0] * (t(j, i + 3) + t(j, i - 3));
            sum += c[1] * (t(j, i + 2) + t(j, i - 2));
            sum += c[2] * (c2a + c2b);
            sum += c[3] * (t(j, i + 1) + t(j, i - 1));
            sum += c[4] * (c4a + c4b);
            sum += c[7] * (t(j + 1, i) + t(j - 1, i));
            sum += c[6] * (t(j + 2, i) + t(j - 2, i));
            sum += c[5] * (t(j + 3, i) + t(j - 3, i));
            sum += c[8] * ctr;
            let v = clip3(0, max, (sum + 32) >> 6);
            dst.data[(r.y0 + i as usize) * dst.w + r.x0 + j as usize] = v as u16;
        }
    }
}

impl Core {
    /// Applies the adaptive loop filter to the whole picture (after SAO).
    pub(crate) fn alf_picture(&mut self) {
        let seq = &self.seq;
        let lcu = seq.max_cuwh;
        let (pw, ph) = (seq.pic_width, seq.pic_height);
        let wl = seq.pic_width_in_lcu;
        let hl = seq.pic_height_in_lcu;
        let bd = seq.bit_depth_internal;
        let cross = seq.cross_patch_loop_filter;
        let alf = self.pichdr.alf_param.clone();
        let regin_w = (wl + 1) / 4 * lcu;
        let regin_h = (hl + 1) / 4 * lcu;
        let src: [Plane; 3] = self.pic.planes.clone();
        for ly in 0..hl {
            let y0 = ly * lcu;
            let lcu_h = lcu.min(ph - y0);
            let mut above = ly > 0;
            let below = ly < hl - 1;
            if !cross && above && self.maps.patch[ly * wl] != self.maps.patch[(ly - 1) * wl] {
                above = false;
            }
            let yo = if above { SHIFT } else { 0 };
            let ho = yo as isize - if below { SHIFT as isize } else { 0 };
            let offset_y = y0.checked_div(regin_h).map_or(3, |v| v.min(3)) << 2;
            for lx in 0..wl {
                let x0 = lx * lcu;
                let lcu_w = lcu.min(pw - x0);
                let idx = ly * wl + lx;
                let en = self.alf_enable_map[idx];
                let offset_x = x0.checked_div(regin_w).map_or(3, |v| v.min(3));
                if en[0] {
                    let bin = TAB_REGION[offset_y + offset_x];
                    let coef = alf.filter_coeff_luma[alf.var_ind_tab[bin]];
                    let r = Region { x0, y0: y0 - yo, w: lcu_w, h: (lcu_h as isize + ho) as usize };
                    alf_region(&src[0], &mut self.pic.planes[0], &r, &coef, bd);
                }
                for comp in 1..3 {
                    if !en[comp] {
                        continue;
                    }
                    let mut coef = [0i32; 9];
                    for (k, v) in coef.iter_mut().enumerate() {
                        *v = alf.filter_coeff_chroma[2 * k + comp - 1];
                    }
                    let r = Region { x0: x0 / 2, y0: y0 / 2 - yo, w: lcu_w / 2, h: ((lcu_h / 2) as isize + ho) as usize };
                    alf_region(&src[comp], &mut self.pic.planes[comp], &r, &coef, bd);
                }
            }
        }
    }
}
