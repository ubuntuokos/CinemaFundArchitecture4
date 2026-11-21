// SPDX-License-Identifier: BSD-3-Clause
// Derived from uavs3d (`source/decore/sao.c`), Copyright (c) 2018-2022 Peking University Shenzhen Graduate
// School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation. See LICENSE.uavs3d.
//! Sample adaptive offset.
//!
//! uavs3d filters one LCU row at a time and delays the filter by four rows and four columns (the deblocking of
//! the neighbouring block has to be finished first), so the parameters of an LCU apply to the region shifted by
//! four samples up and to the left (the same in the chroma planes). This module applies the same regions to the
//! deblocked picture as a whole. Neighbours are always read from the deblocked, not yet offset, samples.

use crate::core::{Core, Plane, SaoParam};
use crate::types::clip3;

const SHIFT: usize = 4;

fn sgn(v: i32) -> i32 {
    v.signum()
}

/// One filtered region of a plane (`sao_on_lcu`): `(x0, y0)` is its top left corner in the plane.
struct Region {
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    left: bool,
    right: bool,
    up: bool,
    down: bool,
}

fn sao_region(src: &Plane, dst: &mut Plane, r: &Region, p: &SaoParam, bit_depth: u32) {
    let max = (1i32 << bit_depth) - 1;
    let (w, h) = (r.w, r.h);
    if w == 0 || h == 0 {
        return;
    }
    let stride = src.w;
    let at = |x: usize, y: usize, dx: isize, dy: isize| -> i32 {
        let xx = (r.x0 + x) as isize + dx;
        let yy = (r.y0 + y) as isize + dy;
        src.data[yy as usize * stride + xx as usize] as i32
    };
    let mut write = |x: usize, y: usize, v: i32| {
        dst.data[(r.y0 + y) * stride + r.x0 + x] = v as u16;
    };
    let eo = |x: usize, y: usize, off_idx: i32| -> i32 {
        let c = src.data[(r.y0 + y) * stride + r.x0 + x] as i32;
        clip3(0, max, c + p.offset[(off_idx + 2) as usize])
    };
    let lo = |c: bool| if c { 0 } else { 1 };
    let hi = |c: bool| if c { w } else { w - 1 };
    match p.typ {
        0 => {
            for y in 0..h {
                for x in lo(r.left)..hi(r.right) {
                    let c = at(x, y, 0, 0);
                    let e = sgn(c - at(x, y, -1, 0)) + sgn(c - at(x, y, 1, 0));
                    write(x, y, eo(x, y, e));
                }
            }
        }
        1 => {
            let ys = lo(r.up);
            let ye = if r.down { h } else { h - 1 };
            for y in ys..ye {
                for x in 0..w {
                    let c = at(x, y, 0, 0);
                    let e = sgn(c - at(x, y, 0, -1)) + sgn(c - at(x, y, 0, 1));
                    write(x, y, eo(x, y, e));
                }
            }
        }
        2 | 3 => {
            let d135 = p.typ == 2;
            // (dx, dy) of the two neighbours
            let (n0, n1): ((isize, isize), (isize, isize)) = if d135 { ((-1, -1), (1, 1)) } else { ((1, -1), (-1, 1)) };
            for y in 0..h {
                let (xs, xe) = if y == 0 {
                    if d135 {
                        (if r.up && r.left { 0 } else { 1 }, if r.up { hi(r.right) } else { 1 })
                    } else {
                        (if r.up { lo(r.left) } else { w - 1 }, if r.up && r.right { w } else { w - 1 })
                    }
                } else if y == h - 1 {
                    if d135 {
                        (if r.down { lo(r.left) } else { w - 1 }, if r.right && r.down { w } else { w - 1 })
                    } else {
                        (if r.left && r.down { 0 } else { 1 }, if r.down { hi(r.right) } else { 1 })
                    }
                } else {
                    (lo(r.left), hi(r.right))
                };
                for x in xs..xe {
                    let c = at(x, y, 0, 0);
                    let e = sgn(c - at(x, y, n0.0, n0.1)) + sgn(c - at(x, y, n1.0, n1.1));
                    write(x, y, eo(x, y, e));
                }
            }
        }
        _ => {
            let shift = bit_depth - 5;
            for y in 0..h {
                for x in 0..w {
                    let v = at(x, y, 0, 0);
                    let band = v >> shift;
                    for k in 0..4 {
                        if band == p.band_idx[k] {
                            write(x, y, clip3(0, max, v + p.offset[k]));
                            break;
                        }
                    }
                }
            }
        }
    }
}

impl Core {
    /// Applies SAO to the whole (deblocked) picture.
    pub(crate) fn sao_picture(&mut self) {
        let seq = &self.seq;
        let lcu = seq.max_cuwh;
        let (pw, ph) = (seq.pic_width, seq.pic_height);
        let wl = seq.pic_width_in_lcu;
        let hl = seq.pic_height_in_lcu;
        let bd = seq.bit_depth_internal;
        let cross = seq.cross_patch_loop_filter;
        let src: [Plane; 3] = self.pic.planes.clone();
        for ly in 0..hl {
            let y0 = ly * lcu;
            let lcu_h = lcu.min(ph - y0);
            let above = ly > 0;
            let below = y0 + lcu < ph;
            let mut flt_border = above;
            if !cross && above && self.maps.patch[ly * wl] != self.maps.patch[(ly - 1) * wl] {
                flt_border = false;
            }
            for lx in 0..wl {
                let x0 = lx * lcu;
                let lcu_w = lcu.min(pw - x0);
                let params = self.sao_param_map[ly * wl + lx];
                let left = lx > 0;
                let right = x0 + lcu_w < pw;
                for c in 0..3 {
                    let p = &params[c];
                    if !p.enable {
                        continue;
                    }
                    let (sh, lw, lh, bx, by) = if c == 0 { (1, lcu_w, lcu_h, x0, y0) } else { (2, lcu_w / 2, lcu_h / 2, x0 / 2, y0 / 2) };
                    let _ = sh;
                    let xo = if left { SHIFT } else { 0 };
                    let yo = if above { SHIFT } else { 0 };
                    let wo = (if left { SHIFT } else { 0 }) as isize - (if right { SHIFT } else { 0 }) as isize;
                    let ho = -(if below { SHIFT } else { 0 } as isize);
                    let rx = bx - xo;
                    let rw = (lw as isize + wo) as usize;
                    let ry = by - yo;
                    let rh = (lh as isize + ho + yo as isize) as usize;
                    let (a, b) = {
                        let planes = &mut self.pic.planes;
                        (&src[c], &mut planes[c])
                    };
                    if p.typ == 4 {
                        sao_region(a, b, &Region { x0: rx, y0: ry, w: rw, h: rh, left, right, up: flt_border, down: below }, p, bd);
                    } else {
                        if above {
                            sao_region(a, b, &Region { x0: rx, y0: ry, w: rw, h: SHIFT, left, right, up: true, down: flt_border }, p, bd);
                        }
                        sao_region(a, b, &Region { x0: rx, y0: by, w: rw, h: (lh as isize + ho) as usize, left, right, up: flt_border, down: below }, p, bd);
                    }
                }
            }
        }
    }
}
