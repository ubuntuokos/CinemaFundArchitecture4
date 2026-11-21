// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from uavs3d 1.2 (commit 0e20d2c2), Copyright (c) 2018-2022 Peking University Shenzhen Graduate School,
// Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation, BSD 3-clause licence (docs/licensing.md).
//! Intra prediction: reference sample construction, DC / planar / bilinear / angular modes, the
//! intra prediction filter (IPF) and TSCPM. Chroma is predicted one component at a time; every
//! mode works the same on U and V as on luma.

use crate::core::{Core, SCU_CODED};
use crate::tables::TSCPM_DIV;

pub(crate) const IPD_DC: i32 = 0;
pub(crate) const IPD_PLN: i32 = 1;
pub(crate) const IPD_BI: i32 = 2;
pub(crate) const IPD_VER: i32 = 12;
pub(crate) const IPD_HOR: i32 = 24;
pub(crate) const IPD_CNT: i32 = 33;
pub(crate) const IPD_IPCM: i32 = 33;
pub(crate) const IPD_DM_C: i32 = 0;
pub(crate) const IPD_DC_C: i32 = 1;
pub(crate) const IPD_HOR_C: i32 = 2;
pub(crate) const IPD_VER_C: i32 = 3;
pub(crate) const IPD_BI_C: i32 = 4;
pub(crate) const IPD_TSCPM_C: i32 = 5;

pub(crate) const AVAIL_UP: u8 = 1;
pub(crate) const AVAIL_LE: u8 = 2;
pub(crate) const AVAIL_UL: u8 = 4;

/// Reference samples: index 0 is the top-left sample, `1 + k` the k-th sample above (and above
/// right), `-1 - k` the k-th sample to the left (and below left).
pub(crate) struct Nb {
    buf: Vec<i32>,
    c: i32,
}

impl Nb {
    fn new(range: usize, fill: i32) -> Nb {
        let n = range + 16;
        Nb { buf: vec![fill; 2 * n + 1], c: n as i32 }
    }
    #[inline]
    pub(crate) fn at(&self, i: i32) -> i32 {
        usize::try_from(self.c + i).ok().and_then(|k| self.buf.get(k)).copied().unwrap_or(0)
    }
    #[inline]
    fn set(&mut self, i: i32, v: i32) {
        if let Some(s) = usize::try_from(self.c + i).ok().and_then(|k| self.buf.get_mut(k)) {
            *s = v;
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

/// `tab_auc_dir_dxdy[xy][mode]` = (multiplier, shift).
const DIR_DXDY: [[(i32, i32); 33]; 2] = [
    [
        (0, 0),
        (0, 0),
        (0, 0),
        (11, 2),
        (2, 0),
        (11, 3),
        (1, 0),
        (93, 7),
        (1, 1),
        (93, 8),
        (1, 2),
        (1, 3),
        (0, 0),
        (1, 3),
        (1, 2),
        (93, 8),
        (1, 1),
        (93, 7),
        (1, 0),
        (11, 3),
        (2, 0),
        (11, 2),
        (4, 0),
        (8, 0),
        (0, 0),
        (8, 0),
        (4, 0),
        (11, 2),
        (2, 0),
        (11, 3),
        (1, 0),
        (93, 7),
        (1, 1),
    ],
    [
        (0, 0),
        (0, 0),
        (0, 0),
        (93, 8),
        (1, 1),
        (93, 7),
        (1, 0),
        (11, 3),
        (2, 0),
        (11, 2),
        (4, 0),
        (8, 0),
        (0, 0),
        (8, 0),
        (4, 0),
        (11, 2),
        (2, 0),
        (11, 3),
        (1, 0),
        (93, 7),
        (1, 1),
        (93, 8),
        (1, 2),
        (1, 3),
        (0, 0),
        (1, 3),
        (1, 2),
        (93, 8),
        (1, 1),
        (93, 7),
        (1, 0),
        (11, 3),
        (2, 0),
    ],
];

fn ctx_pixel(mode: i32, xy: usize, d: i32) -> (i32, i32) {
    let (m, s) = usize::try_from(mode).ok().and_then(|k| DIR_DXDY[xy].get(k)).copied().unwrap_or((0, 0));
    let n = (d * m) >> s;
    let off = ((d * m * 32) >> s) - n * 32;
    (n, off)
}

fn log2(v: i32) -> i32 {
    31 - v.max(1).leading_zeros() as i32
}

fn clip(v: i32, bd: u32) -> i32 {
    v.clamp(0, (1 << bd) - 1)
}

fn pred_dc(nb: &Nb, w: i32, h: i32, avail: u8, bd: u32) -> i32 {
    let le = avail & AVAIL_LE != 0;
    let up = avail & AVAIL_UP != 0;
    if le {
        let mut dc: i32 = (0..h).map(|i| nb.at(-i - 1)).sum();
        if up {
            dc += (0..w).map(|j| nb.at(j + 1)).sum::<i32>();
            ((dc + ((w + h) >> 1)) * (4096 / (w + h))) >> 12
        } else {
            (dc + (h >> 1)) >> log2(h)
        }
    } else if up {
        let dc: i32 = (0..w).map(|j| nb.at(j + 1)).sum();
        (dc + (w >> 1)) >> log2(w)
    } else {
        1 << (bd - 1)
    }
}

fn pred_plane(nb: &Nb, w: i32, h: i32, bd: u32, clipped: bool) -> Vec<i32> {
    const MULT: [i32; 5] = [13, 17, 5, 11, 23];
    const SHIFT: [i32; 5] = [7, 10, 11, 15, 19];
    let (w2, h2) = (w >> 1, h >> 1);
    let iw = usize::try_from(log2(w) - 2).unwrap_or(0).min(4);
    let ih = usize::try_from(log2(h) - 2).unwrap_or(0).min(4);
    let (im_h, is_h, im_v, is_v) = (MULT[iw], SHIFT[iw], MULT[ih], SHIFT[ih]);
    let mut coef_h = 0i32;
    for x in 1..=w2 {
        coef_h += x * (nb.at(w2 + x) - nb.at(w2 - x));
    }
    let mut coef_v = 0i32;
    for y in 1..=h2 {
        coef_v += y * (nb.at(-h2 - y) - nb.at(-h2 + y));
    }
    let a = (nb.at(-h) + nb.at(w)) << 4;
    let b = ((coef_h << 5) * im_h + (1 << (is_h - 1))) >> is_h;
    let c = ((coef_v << 5) * im_v + (1 << (is_v - 1))) >> is_v;
    let mut temp = a - (h2 - 1) * c - (w2 - 1) * b + 16;
    let mut out = Vec::with_capacity((w * h) as usize);
    for _ in 0..h {
        let mut t2 = temp;
        for _ in 0..w {
            let v = t2 >> 5;
            out.push(if clipped { clip(v, bd) } else { i32::from(v as i16) });
            t2 += b;
        }
        temp += c;
    }
    out
}

fn pred_bi(nb: &Nb, w: i32, h: i32, bd: u32, clipped: bool) -> Vec<i32> {
    const WC: [i32; 6] = [-1, 21, 13, 7, 4, 2];
    let (sx, sy) = (log2(w), log2(h));
    let shift = sx.min(sy);
    let shift_xy = sx + sy + 1;
    let offset = 1 << (sx + sy);
    let wc = WC[usize::try_from((sx - sy).abs()).unwrap_or(0).min(5)];
    let mut ref_up: Vec<i32> = (0..w).map(|x| nb.at(x + 1)).collect();
    let mut ref_le: Vec<i32> = (0..h).map(|y| nb.at(-y - 1)).collect();
    let a = nb.at(w);
    let b = nb.at(-h);
    let c = if w == h { (a + b + 1) >> 1 } else { (((a << sx) + (b << sy)) * wc + (1 << (shift + 5))) >> (shift + 6) };
    let wt = (c << 1) - a - b;
    let up: Vec<i32> = ref_up.iter().map(|&r| b - r).collect();
    for r in &mut ref_up {
        *r <<= sy;
    }
    let mut le = vec![0; h as usize];
    let mut wy = vec![0; h as usize];
    let mut tmp = 0;
    for y in 0..h as usize {
        le[y] = a - ref_le[y];
        ref_le[y] <<= sx;
        wy[y] = tmp;
        tmp += wt;
    }
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h as usize {
        let mut predx = ref_le[y];
        let mut wxy = 0;
        for x in 0..w as usize {
            predx += le[y];
            ref_up[x] += up[x];
            let v = ((predx << sy) + (ref_up[x] << sx) + wxy + offset) >> shift_xy;
            out.push(if clipped { clip(v, bd) } else { i32::from(v as i16) });
            wxy += wy[y];
        }
    }
    out
}

fn ang(nb: &Nb, w: i32, h: i32, mode: i32) -> Vec<i32> {
    let mut out = vec![0i32; (w * h) as usize];
    let f = |p: [i32; 4], c: [i32; 4]| (p[0] * c[0] + p[1] * c[1] + p[2] * c[2] + p[3] * c[3] + 64) >> 7;
    if mode > IPD_BI && mode < IPD_VER {
        let w2 = w << 1;
        for j in 0..h {
            let (mut idx, off) = ctx_pixel(mode, 0, j + 1);
            let c = [32 - off, 64 - off, 32 + off, off];
            let mut pw = w.min(w2 - idx + 1);
            let row = (j * w) as usize;
            let mut i = 0;
            while i < pw {
                let p = [nb.at(idx), nb.at(1 + idx), nb.at(2 + idx), nb.at(3 + idx)];
                out[row + i as usize] = f(p, c);
                i += 1;
                idx += 1;
            }
            if pw <= 0 {
                let p = [nb.at(w2), nb.at(1 + w2), nb.at(2 + w2), nb.at(3 + w2)];
                out[row] = f(p, c);
                pw = 1;
                i = 1;
            }
            let last = out[row + (pw - 1) as usize];
            while i < w {
                out[row + i as usize] = last;
                i += 1;
            }
        }
    } else if mode > IPD_HOR {
        let h2 = h << 1;
        let steps: Vec<(i32, i32)> = (0..w).map(|i| ctx_pixel(mode, 1, i + 1)).collect();
        for j in 0..h {
            for i in 0..w {
                let (st, off) = steps[i as usize];
                let idx = (-h2).max(-(j + st));
                // src[idx], src[idx-1], src[idx-2], src[idx-3] in the left column
                let p = [nb.at(idx), nb.at(idx - 1), nb.at(idx - 2), nb.at(idx - 3)];
                out[(j * w + i) as usize] = f(p, [32 - off, 64 - off, 32 + off, off]);
            }
        }
    } else {
        let steps: Vec<(i32, i32)> = (0..w).map(|i| ctx_pixel(mode, 1, i + 1)).collect();
        for j in 0..h {
            let (sx, offx) = ctx_pixel(mode, 0, j + 1);
            for i in 0..w {
                let (st, offy) = steps[i as usize];
                let yy = j - st;
                let v = if yy <= -1 {
                    let px = -sx + i;
                    f([nb.at(px + 2), nb.at(px + 1), nb.at(px), nb.at(px - 1)], [32 - offx, 64 - offx, 32 + offx, offx])
                } else {
                    f([nb.at(-yy - 2), nb.at(-yy - 1), nb.at(-yy), nb.at(-yy + 1)], [32 - offy, 64 - offy, 32 + offy, offy])
                };
                out[(j * w + i) as usize] = v;
            }
        }
    }
    out
}

fn ipf(nb: &Nb, pred: &mut [i32], w: i32, h: i32, ipm: i32, bd: u32) {
    let mut fh = log2(w) - 2;
    let mut fv = log2(h) - 2;
    let mut rv = h.min(10);
    let mut rh = w.min(10);
    if fh > 4 {
        fh = 4;
        rh = 0;
    }
    if fv > 4 {
        fv = 4;
        rv = 0;
    }
    let ph = IPF_PARAM[usize::try_from(fh).unwrap_or(0).min(4)];
    let pv = IPF_PARAM[usize::try_from(fv).unwrap_or(0).min(4)];
    if (3..=18).contains(&ipm) {
        rv = 0;
    }
    if ipm > 18 {
        rh = 0;
    }
    for row in 0..h {
        let ct = if row < rv { pv[row as usize] } else { 0 };
        for col in 0..w {
            let cl = if col < rh { ph[col as usize] } else { 0 };
            let cc = 64 - cl - ct;
            let k = (row * w + col) as usize;
            let v = (cl * nb.at(-row - 1) + ct * nb.at(col + 1) + cc * pred[k] + 32) >> 6;
            pred[k] = clip(v, bd);
        }
    }
}

fn to_u16(v: Vec<i32>) -> Vec<u16> {
    v.into_iter().map(|x| x as u16).collect()
}

impl Core {
    /// `dec_get_avail_intra`: left, up and up-left availability of the block at `scup`.
    pub(crate) fn avail_intra(&self, scup: i32) -> u8 {
        let c = |s: i32| u8::from(self.scu(s) & SCU_CODED != 0);
        (c(scup - self.i_scu) * AVAIL_UP) | (c(scup - 1) * AVAIL_LE) | (c(scup - self.i_scu - 1) * AVAIL_UL)
    }

    /// `com_get_nbr_l` / `com_get_nbr_c` for one component; `x`, `y`, `w`, `h` are in samples of
    /// that component and `scup` is the SCU of the block's luma position.
    pub(crate) fn nbr(&self, comp: usize, x: i32, y: i32, w: i32, h: i32, avail: u8, scup: i32) -> Nb {
        let bd = self.seq.bit_depth;
        let unit = if comp == 0 { 4 } else { 2 };
        let w_scu = w / unit;
        let h_scu = h / unit;
        let range = w.max(h) * 2 + 4;
        let def = 1i32 << (bd - 1);
        let mut nb = Nb::new(range as usize, def);
        let Some(pl) = self.cur.planes.get(comp) else {
            return nb;
        };
        if avail & AVAIL_UP != 0 {
            let mut n = w;
            for k in 0..w {
                nb.set(1 + k, pl.get(x + k, y - 1));
            }
            for i in 0..w_scu {
                if !self.coded(scup - self.i_scu + w_scu + i) {
                    break;
                }
                for k in 0..unit {
                    nb.set(1 + n + k, pl.get(x + n + k, y - 1));
                }
                n += unit;
            }
            let last = nb.at(n);
            for k in n..range {
                nb.set(1 + k, last);
            }
        }
        if avail & AVAIL_LE != 0 {
            let mut n = h;
            for k in 0..h {
                nb.set(-1 - k, pl.get(x - 1, y + k));
            }
            for i in 0..h_scu {
                if !self.coded(scup - 1 + (h_scu + i) * self.i_scu) {
                    break;
                }
                for k in 0..unit {
                    nb.set(-1 - n - k, pl.get(x - 1, y + n + k));
                }
                n += unit;
            }
            let last = nb.at(-n);
            for k in n..range {
                nb.set(-1 - k, last);
            }
        }
        let tl = if avail & AVAIL_UL != 0 {
            pl.get(x - 1, y - 1)
        } else if avail & AVAIL_UP != 0 {
            pl.get(x, y - 1)
        } else if avail & AVAIL_LE != 0 {
            pl.get(x - 1, y)
        } else {
            def
        };
        nb.set(0, tl);
        nb
    }

    /// Predicts a luma block (`com_ipred_l`).
    pub(crate) fn ipred_luma(&self, x: i32, y: i32, w: i32, h: i32, ipm: i32, ipf_on: bool, scup: i32) -> Vec<u16> {
        let bd = self.seq.bit_depth;
        let avail = self.avail_intra(scup);
        let nb = self.nbr(0, x, y, w, h, avail, scup);
        self.ipred_comp(&nb, w, h, ipm, ipf_on, avail, bd)
    }

    fn ipred_comp(&self, nb: &Nb, w: i32, h: i32, ipm: i32, ipf_on: bool, avail: u8, bd: u32) -> Vec<u16> {
        let ipf_on = ipf_on && w < 128 && h < 128;
        let mut p = if ipm == IPD_PLN || ipm == IPD_BI {
            let v = if ipm == IPD_PLN { pred_plane(nb, w, h, bd, !ipf_on) } else { pred_bi(nb, w, h, bd, !ipf_on) };
            if ipf_on {
                let mut v = v;
                ipf(nb, &mut v, w, h, ipm, bd);
                return to_u16(v);
            }
            v
        } else {
            let v = match ipm {
                IPD_VER => (0..h).flat_map(|_| (0..w).map(|i| nb.at(1 + i))).collect(),
                IPD_HOR => (0..h).flat_map(|j| (0..w).map(move |_| j)).map(|j| nb.at(-1 - j)).collect(),
                IPD_DC => vec![pred_dc(nb, w, h, avail, bd); (w * h) as usize],
                _ => ang(nb, w, h, ipm),
            };
            v.into_iter().map(|a: i32| clip(a, bd)).collect()
        };
        if ipf_on {
            ipf(nb, &mut p, w, h, ipm, bd);
        }
        to_u16(p)
    }

    /// Predicts the two chroma blocks of the CU at luma position (`x`, `y`) of size `cw` × `ch`
    /// luma samples (`com_ipred_c`).
    pub(crate) fn ipred_chroma(&self, x: i32, y: i32, cw: i32, ch: i32, ipm_c: i32, ipm: i32, scup: i32) -> [Vec<u16>; 2] {
        let bd = self.seq.bit_depth;
        let avail = self.avail_intra(scup);
        let (w, h) = (cw >> 1, ch >> 1);
        let (cx, cy) = (x >> 1, y >> 1);
        let mut mode = ipm_c;
        if ipm_c == IPD_DM_C && matches!(ipm, IPD_VER | IPD_HOR | IPD_DC | IPD_BI) {
            mode = match ipm {
                IPD_VER => IPD_VER_C,
                IPD_HOR => IPD_HOR_C,
                IPD_DC => IPD_DC_C,
                _ => IPD_BI_C,
            };
        }
        let nbs = [self.nbr(1, cx, cy, w, h, avail, scup), self.nbr(2, cx, cy, w, h, avail, scup)];
        if mode == IPD_TSCPM_C {
            return self.tscpm(x, y, cw, ch, &nbs, avail, scup);
        }
        let one = |nb: &Nb| -> Vec<u16> {
            let m = match mode {
                IPD_DM_C => ipm,
                IPD_DC_C => IPD_DC,
                IPD_HOR_C => IPD_HOR,
                IPD_VER_C => IPD_VER,
                _ => IPD_BI,
            };
            self.ipred_comp(nb, w, h, m, false, avail, bd)
        };
        [one(&nbs[0]), one(&nbs[1])]
    }

    fn tscpm(&self, x: i32, y: i32, cw: i32, ch: i32, nbs: &[Nb; 2], avail: u8, scup: i32) -> [Vec<u16>; 2] {
        let bd = self.seq.bit_depth;
        let (w, h) = (cw >> 1, ch >> 1);
        let up = avail & AVAIL_UP != 0;
        let le = avail & AVAIL_LE != 0;
        let ln = self.nbr(0, x, y, cw, ch, avail, scup);
        // luma border samples, filtered
        let luma_up =
            |i: i32| -> i32 { if i == 0 && !le { (3 * ln.at(1) + ln.at(2) + 2) >> 2 } else { (2 * ln.at(1 + i) + ln.at(i) + ln.at(2 + i) + 2) >> 2 } };
        let luma_le = |j: i32| -> i32 { (ln.at(-1 - j) + ln.at(-2 - j) + 1) >> 1 };
        let min_dim = if le && up {
            h.min(w)
        } else if le {
            h
        } else {
            w
        };
        let steps = min_dim;
        let mut rl = [-1i32; 4];
        let mut rc = [[-1i32; 4]; 2];
        if up {
            let idx = (((steps - 1) * w) / min_dim) << 1;
            rl[0] = luma_up(0);
            rl[1] = luma_up(idx);
            for (c, nb) in nbs.iter().enumerate() {
                rc[c][0] = nb.at(1);
                rc[c][1] = nb.at(1 + (idx >> 1));
            }
            if !le && w >= 4 {
                let st = w >> 1;
                for i in 0..4 {
                    rl[i as usize] = luma_up(i * st);
                    for (c, nb) in nbs.iter().enumerate() {
                        rc[c][i as usize] = nb.at(1 + ((i * st) >> 1));
                    }
                }
            }
        }
        if le {
            let idx = (((steps - 1) * h) / min_dim) << 1;
            rl[2] = luma_le(0);
            rl[3] = luma_le(idx);
            for (c, nb) in nbs.iter().enumerate() {
                rc[c][2] = nb.at(-1);
                rc[c][3] = nb.at(-1 - (idx >> 1));
            }
            if !up && h >= 4 {
                let st = h >> 1;
                for i in 0..4 {
                    rl[i as usize] = luma_le(i * st);
                    for (c, nb) in nbs.iter().enumerate() {
                        rc[c][i as usize] = nb.at(-1 - ((i * st) >> 1));
                    }
                }
            }
        }
        let mut x_max = i32::MAX.wrapping_neg();
        let mut x_min = i32::MAX;
        let mut y_max = [0i32; 2];
        let mut y_min = [0i32; 2];
        if (up && le) || (up && !le && w >= 4) || (le && !up && h >= 4) {
            let mut mn = [0usize, 2];
            let mut mx = [1usize, 3];
            if rl[mn[0]] > rl[mn[1]] {
                mn.swap(0, 1);
            }
            if rl[mx[0]] > rl[mx[1]] {
                mx.swap(0, 1);
            }
            if rl[mn[0]] > rl[mx[1]] {
                std::mem::swap(&mut mn, &mut mx);
            }
            if rl[mn[1]] > rl[mx[0]] {
                std::mem::swap(&mut mn[1], &mut mx[0]);
            }
            x_min = (rl[mn[0]] + rl[mn[1]] + 1) >> 1;
            x_max = (rl[mx[0]] + rl[mx[1]] + 1) >> 1;
            for c in 0..2 {
                y_min[c] = (rc[c][mn[0]] + rc[c][mn[1]] + 1) >> 1;
                y_max[c] = (rc[c][mx[0]] + rc[c][mx[1]] + 1) >> 1;
            }
        } else if up || le {
            let ks = if up { 0..2 } else { 2..4 };
            for k in ks {
                if rl[k] > x_max {
                    x_max = rl[k];
                    y_max = [rc[0][k], rc[1][k]];
                }
                if rl[k] < x_min {
                    x_min = rl[k];
                    y_min = [rc[0][k], rc[1][k]];
                }
            }
        }
        let mut a = [0i64; 2];
        let mut b = [0i64; 2];
        let mut shift = 16i32;
        if up || le {
            let mut diff = x_max - x_min;
            let mut add = 0;
            let mut sh = 0;
            if diff > 64 {
                sh = if bd > 8 { bd as i32 - 6 } else { 2 };
                add = if sh > 0 { 1 << (sh - 1) } else { 0 };
                diff = (diff + add) >> sh;
            }
            if diff > 0 {
                let d = i64::from(TSCPM_DIV.get((diff - 1) as usize).copied().unwrap_or(0));
                for c in 0..2 {
                    a[c] = ((i64::from(y_max[c] - y_min[c]) * d) + i64::from(add)) >> sh;
                }
            }
            for c in 0..2 {
                b[c] = i64::from(y_min[c]) - ((a[c] * i64::from(x_min)) >> shift);
            }
        } else {
            b = [1i64 << (bd - 1); 2];
            shift = 0;
        }
        let maxv = (1i64 << bd) - 1;
        let pl = &self.cur.planes[0];
        let mut out = [Vec::new(), Vec::new()];
        for c in 0..2 {
            // linear model on the reconstructed luma, then 6-tap downsampling
            let t = |i: i32, j: i32| -> i64 {
                let l = i64::from(pl.get(x + i, y + j));
                (((a[c] * l) >> shift) + b[c]).clamp(0, maxv)
            };
            let mut o = Vec::with_capacity((w * h) as usize);
            for j in 0..h {
                for i in 0..w {
                    let v = if i == 0 {
                        (t(0, 2 * j) + t(0, 2 * j + 1) + 1) >> 1
                    } else {
                        (t(2 * i, 2 * j) * 2
                            + t(2 * i + 1, 2 * j)
                            + t(2 * i - 1, 2 * j)
                            + t(2 * i, 2 * j + 1) * 2
                            + t(2 * i + 1, 2 * j + 1)
                            + t(2 * i - 1, 2 * j + 1)
                            + 4)
                            >> 3
                    };
                    o.push(v as u16);
                }
            }
            out[c] = o;
        }
        out
    }
}
