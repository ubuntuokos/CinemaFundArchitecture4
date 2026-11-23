// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from uavs3d 1.2 (commit 0e20d2c2), Copyright (c) 2018-2022 Peking University Shenzhen Graduate School,
// Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation, BSD 3-clause licence (docs/licensing.md).
//! Sample adaptive offset and the adaptive loop filter, applied to the whole picture after deblocking.

use crate::core::Core;

fn sign(v: i32) -> i32 {
    (v > 0) as i32 - (v < 0) as i32
}

impl Core {
    fn patch_differs(&self, lcu_y: usize) -> bool {
        let w = self.seq.w_lcu as usize;
        lcu_y > 0 && self.map_patch.get(lcu_y * w) != self.map_patch.get((lcu_y - 1) * w)
    }

    /// Sample adaptive offset of every LCU on the deblocked picture. As in uavs3d the block of an
    /// LCU is shifted up and left by 4 samples (of the component), because the deblocking of the
    /// LCU row below and of the LCU to the right is not finished when the LCU is filtered; the
    /// last LCU of a row/column extends to the picture edge. Neighbouring samples are available
    /// inside the picture, except across the top edge of an LCU row in another patch when the
    /// loop filter does not cross patches.
    pub(crate) fn sao_picture(&mut self) {
        let bd = self.seq.bit_depth;
        let max = (1i32 << bd) - 1;
        let lcu = self.seq.max_cu as i32;
        let (wl, hl) = (self.seq.w_lcu as usize, self.seq.h_lcu as usize);
        let (pw, ph) = (self.seq.pic_width as i32, self.seq.pic_height as i32);
        let cross = self.seq.cross_patch_loop_filter;
        for comp in 0..3usize {
            if !(0..hl * wl).any(|i| self.sao_map.get(i).is_some_and(|p| p[comp].enable)) {
                continue;
            }
            let (sh, cw, chh) = if comp == 0 { (0, pw, ph) } else { (1, pw >> 1, ph >> 1) };
            let src = self.cur.planes[comp].data.clone();
            let stride = self.cur.planes[comp].w;
            for ly in 0..hl {
                let up_ok = ly > 0 && (cross || !self.patch_differs(ly));
                let below = ly + 1 < hl;
                for lx in 0..wl {
                    let Some(p) = self.sao_map.get(ly * wl + lx).map(|a| a[comp]) else {
                        continue;
                    };
                    if !p.enable {
                        continue;
                    }
                    let (px0, py0) = (lx as i32 * lcu, ly as i32 * lcu);
                    let (px1, py1) = ((px0 + lcu).min(pw), (py0 + lcu).min(ph));
                    let right = px1 < pw;
                    let (bx0, bx1) = ((px0 >> sh) - if lx > 0 { 4 } else { 0 }, (px1 >> sh) - if right { 4 } else { 0 });
                    let line = py0 >> sh;
                    let (by0, by1) = (line - if ly > 0 { 4 } else { 0 }, (py1 >> sh) - if below { 4 } else { 0 });
                    let at = |x: i32, y: i32| i32::from(src[y as usize * stride + x as usize]);
                    let avail = |y: i32, nx: i32, ny: i32| -> bool {
                        if nx < 0 || ny < 0 || nx >= cw || ny >= chh {
                            return false;
                        }
                        !(ly > 0 && !up_ok && ((y < line) != (ny < line)))
                    };
                    let dst = &mut self.cur.planes[comp].data;
                    for y in by0..by1 {
                        for x in bx0..bx1 {
                            let cur = at(x, y);
                            let v = if p.typ == 4 {
                                let band = cur >> (bd - 5);
                                match p.band_idx.iter().position(|&b| b == band) {
                                    Some(k) => cur + p.offset[k],
                                    None => cur,
                                }
                            } else {
                                let ((ax, ay), (bx, by)) = match p.typ {
                                    0 => ((-1, 0), (1, 0)),
                                    1 => ((0, -1), (0, 1)),
                                    2 => ((-1, -1), (1, 1)),
                                    _ => ((1, -1), (-1, 1)),
                                };
                                let (n1x, n1y, n2x, n2y) = (x + ax, y + ay, x + bx, y + by);
                                if !avail(y, n1x, n1y) || !avail(y, n2x, n2y) {
                                    continue;
                                }
                                let e = sign(cur - at(n1x, n1y)) + sign(cur - at(n2x, n2y));
                                cur + p.offset[(e + 2) as usize]
                            };
                            dst[y as usize * stride + x as usize] = v.clamp(0, max) as u16;
                        }
                    }
                }
            }
        }
    }

    fn alf_region(&self, lx: usize, ly: usize) -> usize {
        const REGION: [usize; 16] = [0, 1, 4, 5, 15, 2, 3, 6, 14, 11, 10, 7, 13, 12, 9, 8];
        let lcu = self.seq.max_cu as usize;
        let (wl, hl) = (self.seq.w_lcu as usize, self.seq.h_lcu as usize);
        let rw = (wl + 1) / 4 * lcu;
        let rh = (hl + 1) / 4 * lcu;
        let oy = (3.min((ly * lcu).checked_div(rh).unwrap_or(3))) << 2;
        let ox = 3.min((lx * lcu).checked_div(rw).unwrap_or(3));
        REGION[oy + ox]
    }

    /// Adaptive loop filter of every LCU on the SAO output.
    pub(crate) fn alf_picture(&mut self) {
        if !self.seq.alf_enable || !self.ph.alf.enable {
            return;
        }
        let bd = self.seq.bit_depth;
        let max = (1i32 << bd) - 1;
        let lcu = self.seq.max_cu as i32;
        let (wl, hl) = (self.seq.w_lcu as usize, self.seq.h_lcu as usize);
        let (pw, ph) = (self.seq.pic_width as i32, self.seq.pic_height as i32);
        let cross = self.seq.cross_patch_loop_filter;
        for comp in 0..3usize {
            let any = (0..hl * wl).any(|i| self.alf_map.get(i).is_some_and(|f| f[comp]));
            if !any {
                continue;
            }
            let sh = i32::from(comp != 0);
            let (cw, chh) = (pw >> sh, ph >> sh);
            let src = self.cur.planes[comp].data.clone();
            let stride = self.cur.planes[comp].w;
            for ly in 0..hl {
                let above = ly > 0 && (cross || !self.patch_differs(ly));
                let below = ly + 1 < hl;
                let y0 = (ly as i32 * lcu) >> sh;
                let lh = ((ly as i32 * lcu + lcu).min(ph) >> sh) - y0;
                let top = y0 - if above { 4 } else { 0 };
                let bottom = y0 + lh - if below { 4 } else { 0 };
                for lx in 0..wl {
                    if !self.alf_map.get(ly * wl + lx).is_some_and(|f| f[comp]) {
                        continue;
                    }
                    let x0 = (lx as i32 * lcu) >> sh;
                    let x1 = ((lx as i32 * lcu + lcu).min(pw)) >> sh;
                    let coef: [i32; 9] = if comp == 0 {
                        let v = self.ph.alf.var_ind_tab[self.alf_region(lx, ly).min(15)].clamp(0, 15) as usize;
                        self.ph.alf.coeff_luma[v]
                    } else {
                        let mut c = [0i32; 9];
                        for (k, v) in c.iter_mut().enumerate() {
                            *v = self.ph.alf.coeff_chroma[2 * k + (comp - 1)];
                        }
                        c
                    };
                    // taps in the luma order: [vert3, vert2, diag(+,+)/(-,-), vert1, diag(+,-)/(-,+), horiz3, horiz2, horiz1, centre]
                    let mut dst_rows = Vec::new();
                    for y in top..bottom {
                        for x in x0..x1 {
                            let first = y == top;
                            let last = y == bottom - 1;
                            let row = |r: i32| -> i32 { r.clamp(top, bottom - 1) };
                            let g = |xx: i32, yy: i32| -> i32 {
                                let xc = xx.clamp(0, cw - 1);
                                i32::from(src[row(yy) as usize * stride + xc as usize])
                            };
                            // diagonal taps; at the block corners the column is not moved (uavs3d `alf_one_blk_fix`)
                            let ul_x = if first && x == x0 { x } else { x - 1 };
                            let ur_x = if first && x == x1 - 1 { x } else { x + 1 };
                            let dl_x = if last && x == x0 { x } else { x - 1 };
                            let dr_x = if last && x == x1 - 1 { x } else { x + 1 };
                            let mut s = coef[0] * (g(x, y + 3) + g(x, y - 3));
                            s += coef[1] * (g(x, y + 2) + g(x, y - 2));
                            s += coef[2] * (g(dr_x, y + 1) + g(ul_x, y - 1));
                            s += coef[3] * (g(x, y + 1) + g(x, y - 1));
                            s += coef[4] * (g(dl_x, y + 1) + g(ur_x, y - 1));
                            s += coef[7] * (g(x + 1, y) + g(x - 1, y));
                            s += coef[6] * (g(x + 2, y) + g(x - 2, y));
                            s += coef[5] * (g(x + 3, y) + g(x - 3, y));
                            s += coef[8] * g(x, y);
                            dst_rows.push((y, x, ((s + 32) >> 6).clamp(0, max) as u16));
                        }
                    }
                    let dst = &mut self.cur.planes[comp].data;
                    for (y, x, v) in dst_rows {
                        if y >= 0 && y < chh {
                            dst[y as usize * stride + x as usize] = v;
                        }
                    }
                }
            }
        }
    }
}
