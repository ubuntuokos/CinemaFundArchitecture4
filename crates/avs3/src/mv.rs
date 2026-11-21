// SPDX-License-Identifier: GPL-3.0-or-later
//! Motion-vector prediction and derivation: AMVP, skip/direct, UMVE, affine, HMVP.
//!
//! Derived from uavs3d 1.2 (commit 0e20d2c2), Copyright (c) 2018-2022 Peking University
//! Shenzhen Graduate School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation
//! Corporation, BSD 3-clause licence (docs/licensing.md).

use crate::core::*;
use crate::header::SliceType;
use crate::tables::LOG2_TAB;

const INT16_MIN: i32 = -32768;
const INT16_MAX: i32 = 32767;
const INT18_MIN: i32 = -(1 << 17);
const INT18_MAX: i32 = (1 << 17) - 1;

fn log2_of(v: i32) -> i32 {
    usize::try_from(v).ok().and_then(|i| LOG2_TAB.get(i)).map_or(0, |&l| i32::from(l))
}

fn clip_i16(v: i64) -> i16 {
    v.clamp(i64::from(INT16_MIN), i64::from(INT16_MAX)) as i16
}

/// Rounds |v| with a shift and restores the sign (the `mask ^ x - mask` idiom).
fn sym_shift(v: i64, add: i64, shift: u32) -> i64 {
    let neg = v < 0;
    let r = (v.abs() + add) >> shift;
    if neg { -r } else { r }
}

pub(crate) fn scaling_mv(dist_cur: i32, dist_neb: i32, mvp: Mv) -> Mv {
    if mvp == [0, 0] || dist_neb == 0 {
        return [0, 0];
    }
    if dist_neb == dist_cur && (1 << MV_SCALE_PREC) % dist_neb == 0 {
        return mvp;
    }
    let offset = 1i64 << (MV_SCALE_PREC - 1);
    let ratio = i64::from(((offset as i32) / dist_neb).wrapping_mul(dist_cur)) << 1;
    let f = |c: i16| -> i16 { if c == 0 { 0 } else { clip_i16(sym_shift(i64::from(c) * ratio, offset, MV_SCALE_PREC as u32)) } };
    [f(mvp[0]), f(mvp[1])]
}

fn scaling_mv_amvr(dist_cur: i32, dist_neb: i32, mvp: Mv, amvr_idx: i32) -> Mv {
    if mvp == [0, 0] || dist_neb == 0 {
        return [0, 0];
    }
    let add = if amvr_idx > 0 { 1i64 << (amvr_idx - 1) } else { 0 };
    let amvr_mask = !((1i64 << amvr_idx) - 1);
    let signed = |v: i64, f: &dyn Fn(i64) -> i64| -> i64 {
        let r = f(v.abs());
        if v < 0 { -r } else { r }
    };
    if dist_neb == dist_cur && (1 << MV_SCALE_PREC) % dist_neb == 0 {
        let g = |c: i16| -> i16 { if c == 0 { 0 } else { clip_i16(signed(i64::from(c), &|a| (a + add) & amvr_mask)) } };
        [g(mvp[0]), g(mvp[1])]
    } else {
        let mut offset = 1i64 << (MV_SCALE_PREC - 1);
        let ratio = i64::from(((offset as i32) / dist_neb).wrapping_mul(dist_cur)) << 1;
        offset += add << MV_SCALE_PREC;
        let g = |c: i16| -> i16 {
            if c == 0 {
                0
            } else {
                let t = i64::from(c) * ratio;
                clip_i16(signed(t, &|a| ((a + offset) >> MV_SCALE_PREC) & amvr_mask))
            }
        };
        [g(mvp[0]), g(mvp[1])]
    }
}

fn mv_rounding_s16(mvp: &mut Mv, shift: i32) {
    let add = if shift != 0 { 1 << (shift - 1) } else { 0 };
    for c in mvp.iter_mut() {
        if *c != 0 {
            let v = i32::from(*c);
            let a = (((v.abs() + add) >> shift) << shift) * v.signum();
            *c = a.clamp(INT16_MIN, INT16_MAX) as i16;
        }
    }
}

fn mv_rounding_s32(mvp: Mv, shift: i32, add: i32) -> [i32; 2] {
    let f = |c: i16| -> i32 {
        if c == 0 {
            0
        } else {
            let v = i32::from(c) << 2;
            let a = (((v.abs() + add) >> shift) << shift) * v.signum();
            a.clamp(CPMV_MIN, CPMV_MAX)
        }
    };
    [f(mvp[0]), f(mvp[1])]
}

fn mv_rounding_shift7(hor: i32, ver: i32) -> (i32, i32) {
    let f = |v: i32| -> i32 {
        if v == 0 {
            0
        } else {
            let a = (v.wrapping_abs().wrapping_add(64)) >> 7;
            if v < 0 { -a } else { a }
        }
    };
    (f(hor), f(ver))
}

/// `(mv for MC at 1/16 pel, mv for storage at 1/4 pel)` of one component.
fn mv_rounding_affine1(v: i32) -> (i32, i16) {
    let add1 = 1 << 6;
    let add2 = 1 << 1;
    if v == 0 {
        (0, 0)
    } else if v > 0 {
        let m1 = ((v + add1) >> 7).min(INT18_MAX);
        let m2 = ((m1 + add2) >> 2).min(INT16_MAX);
        (m1, m2 as i16)
    } else {
        let t = (-v + add1) >> 7;
        let m1 = (-t).max(INT18_MIN);
        let m2 = (-((t + add2) >> 2)).max(INT16_MIN);
        (m1, m2 as i16)
    }
}

pub(crate) fn mv_rounding_affine(hor: i32, ver: i32) -> ([i32; 2], Mv) {
    let (x1, x2) = mv_rounding_affine1(hor);
    let (y1, y2) = mv_rounding_affine1(ver);
    ([x1, y1], [x2, y2])
}

struct Neighbors {
    idx: [i32; 5],
    valid: [bool; 5],
}

impl Core {
    fn colocal_scup(&self, scup: i32) -> (i32, i32) {
        let w = self.seq.w_scu as i32;
        let h = self.seq.h_scu as i32;
        let bx = scup % self.i_scu;
        let by = scup / self.i_scu;
        let mut xpos = (bx & !3) + 2;
        let mut ypos = (by & !3) + 2;
        if ypos >= h {
            ypos = ((by & !3) + h) >> 1;
        }
        if xpos >= w {
            xpos = ((bx & !3) + w) >> 1;
        }
        (ypos * self.i_scu + xpos, ypos)
    }

    fn umve_neighbors(&self) -> Neighbors {
        let scup = self.cu_scup;
        let (wi, hi) = (self.cu_w >> 2, self.cu_h >> 2);
        let idx = [scup + self.i_scu * (hi - 1) - 1, scup - self.i_scu + wi - 1, scup - self.i_scu + wi, scup - 1, scup - self.i_scu - 1];
        let tv: Vec<bool> = idx.iter().map(|&i| self.is_inter(i)).collect();
        let mut valid = [tv[0], tv[1], tv[2], tv[3], tv[4]];
        let m = |k: usize| self.motion_at(idx[k]);
        if tv[0] && tv[1] {
            valid[1] = !m(1).same(&m(0));
        }
        if tv[1] && tv[2] {
            valid[2] = !m(1).same(&m(2));
        }
        if tv[0] && tv[3] {
            valid[3] = !m(3).same(&m(0));
        }
        if tv[4] {
            valid[4] = (!tv[3] || !m(4).same(&m(3))) && (!tv[1] || !m(4).same(&m(1)));
        }
        Neighbors { idx, valid }
    }

    fn mvp_hmvp(&self, motion: &Motion, lidx: usize, cur_refi: i8, amvr_idx: i32) -> Mv {
        let dist_cur = self.dist(cur_refi, lidx);
        let (mut l, mut r) = (lidx, motion.refi[lidx]);
        if r == REFI_INVALID {
            l = 1 - lidx;
            r = motion.refi[l];
        }
        scaling_mv_amvr(dist_cur, self.dist(r, l), motion.mv[l], amvr_idx)
    }

    fn mvp_default(&self, lidx: usize, cur_refi: i8, amvr_idx: i32) -> Mv {
        let scup = self.cu_scup;
        let i = self.i_scu;
        let dist_cur = self.dist(cur_refi, lidx);
        let mut nb = [scup - 1, scup - i, scup - i + (self.cu_w >> 2)];
        let rf = |s: &Core, n: i32| -> i8 { if s.coded(n) { s.refi_at(n)[lidx] } else { REFI_INVALID } };
        let (rl, ru) = (rf(self, nb[0]), rf(self, nb[1]));
        let mut rur = rf(self, nb[2]);
        let mut mvps = [[0i16; 2]; 3];
        let valid = |r: i8| r >= 0;
        if valid(rl) {
            mvps[0] = scaling_mv(dist_cur, self.dist(rl, lidx), self.mv_at(nb[0])[lidx]);
        }
        if valid(ru) {
            mvps[1] = scaling_mv(dist_cur, self.dist(ru, lidx), self.mv_at(nb[1])[lidx]);
        }
        if valid(rur) {
            mvps[2] = scaling_mv(dist_cur, self.dist(rur, lidx), self.mv_at(nb[2])[lidx]);
        } else {
            nb[2] = scup - i - 1;
            rur = rf(self, nb[2]);
            if valid(rur) {
                mvps[2] = scaling_mv(dist_cur, self.dist(rur, lidx), self.mv_at(nb[2])[lidx]);
            }
        }
        let inv = REFI_INVALID;
        let mut mvp = [0i16; 2];
        let pred_type = if rl != inv && ru == inv && rur == inv {
            0
        } else if rl == inv && ru != inv && rur == inv {
            1
        } else if rl == inv && ru == inv && rur != inv {
            2
        } else {
            3
        };
        match pred_type {
            3 => {
                for hv in 0..2 {
                    let (a, b, c) = (i32::from(mvps[0][hv]), i32::from(mvps[1][hv]), i32::from(mvps[2][hv]));
                    let v = if (a < 0 && b > 0 && c > 0) || (a > 0 && b < 0 && c < 0) {
                        (b + c) / 2
                    } else if (b < 0 && a > 0 && c > 0) || (b > 0 && a < 0 && c < 0) {
                        (c + a) / 2
                    } else if (c < 0 && a > 0 && b > 0) || (c > 0 && a < 0 && b < 0) {
                        (a + b) / 2
                    } else {
                        let ea = (a - b).abs();
                        let eb = (b - c).abs();
                        let ec = (c - a).abs();
                        let m = ea.min(eb.min(ec));
                        if m == ea {
                            (a + b) / 2
                        } else if m == eb {
                            (b + c) / 2
                        } else {
                            (c + a) / 2
                        }
                    };
                    mvp[hv] = v as i16;
                }
            }
            t => mvp = mvps[t],
        }
        mv_rounding_s16(&mut mvp, amvr_idx);
        mvp
    }

    pub(crate) fn derive_mvp(&self, list: usize, emvp: bool, mvr_idx: i32) -> Mv {
        let ref_idx = self.refi[list];
        if !emvp {
            return self.mvp_default(list, ref_idx, mvr_idx);
        }
        let cnt = self.hmvp_cnt;
        if cnt == 0 {
            return [0, 0];
        }
        let sel = if (cnt as i32) < mvr_idx + 1 { cnt - 1 } else { cnt - 1 - mvr_idx as usize };
        let m = self.hmvp.get(sel).copied().unwrap_or_default();
        self.mvp_hmvp(&m, list, ref_idx, mvr_idx)
    }

    /// Spatial skip candidates: the three traditional ones (bi, L1-only, L0-only).
    fn mhb_skip_motions(&self) -> [Motion; 3] {
        let scup = self.cu_scup;
        let i = self.i_scu;
        let (wi, hi) = (self.cu_w >> 2, self.cu_h >> 2);
        let mut list = [Motion::default(); 3];
        list[2].refi[1] = REFI_INVALID;
        list[1].refi[0] = REFI_INVALID;
        let nb = [scup + (hi - 1) * i - 1, scup - i + wi - 1, scup - i + wi, scup - 1, scup - i, scup - i - 1];
        let (mut l0f, mut l1f, mut bif) = (false, false, false);
        let mut last_bi = 0usize;
        for (k, &n) in nb.iter().enumerate() {
            if !self.is_inter(n) {
                continue;
            }
            let r = self.refi_at(n);
            let m = self.mv_at(n);
            let r0 = if r[0] >= 0 { r[0] } else { REFI_INVALID };
            let r1 = if r[1] >= 0 { r[1] } else { REFI_INVALID };
            if r0 >= 0 && r1 < 0 && !l0f {
                l0f = true;
                list[2].mv[0] = m[0];
                list[2].refi[0] = r0;
            }
            if r0 < 0 && r1 >= 0 && !l1f {
                l1f = true;
                list[1].mv[1] = m[1];
                list[1].refi[1] = r1;
            }
            if r0 >= 0 && r1 >= 0 {
                if !bif {
                    list[0].mv = m;
                    list[0].refi = [r0, r1];
                }
                bif = true;
                last_bi = k;
            }
        }
        if l0f && l1f && !bif {
            list[0].mv = [list[2].mv[0], list[1].mv[1]];
            list[0].refi = [list[2].refi[0], list[1].refi[1]];
        }
        if !l0f && bif {
            let n = nb[last_bi];
            list[2].mv[0] = self.mv_at(n)[0];
            list[2].refi[0] = self.refi_at(n)[0];
        }
        if !l1f && bif {
            let n = nb[last_bi];
            list[1].mv[1] = self.mv_at(n)[1];
            list[1].refi[1] = self.refi_at(n)[1];
        }
        list
    }

    fn col_mv_from_list0(&self, scup_co: i32) -> Mv {
        let Some(p) = self.refp.first().and_then(|r| r[0].pic.as_ref()) else { return [0, 0] };
        let ix = self.ix(scup_co);
        let refi = p.map_refi.get(ix).map_or(-1, |r| r[0]);
        let dist_col = usize::try_from(refi).ok().and_then(|r| p.list_dist.get(r)).copied().map_or(1, i32::from);
        scaling_mv(self.refp[0][0].dist, dist_col, p.map_mv.get(ix).map_or([0, 0], |m| m[0]))
    }

    fn col_mv(&self, scup_co: i32) -> [Mv; 2] {
        let Some(p) = self.refp.first().and_then(|r| r[1].pic.as_ref()) else { return [[0, 0]; 2] };
        let ix = self.ix(scup_co);
        let mut refi = p.map_refi.get(ix).map_or(-1, |r| r[0]);
        if refi == REFI_INVALID {
            refi = 0;
        }
        let dist_col = usize::try_from(refi).ok().and_then(|r| p.list_dist.get(r)).copied().map_or(1, i32::from);
        let mv = p.map_mv.get(ix).map_or([0, 0], |m| m[0]);
        [scaling_mv(self.refp[0][0].dist, dist_col, mv), scaling_mv(self.refp[0][1].dist, dist_col, mv)]
    }

    /// The temporal (collocated) candidate. Returns `(mv, refi)`.
    pub(crate) fn temporal_motion(&self) -> Motion {
        let (scup_co, _) = self.colocal_scup(self.cu_scup);
        let mut m = Motion::default();
        if self.slice_type == SliceType::P {
            m.refi = [0, -1];
            let valid = self.refp.first().and_then(|r| r[0].pic.as_ref()).and_then(|p| p.map_refi.get(self.ix(scup_co))).is_some_and(|r| r[0] >= 0);
            if valid {
                m.mv[0] = self.col_mv_from_list0(scup_co);
            }
        } else {
            m.refi = [0, 0];
            let valid = self.refp.first().and_then(|r| r[1].pic.as_ref()).and_then(|p| p.map_refi.get(self.ix(scup_co))).is_some_and(|r| r[0] >= 0);
            if !valid {
                m.mv[0] = self.mvp_default(0, 0, 0);
                m.mv[1] = self.mvp_default(1, 0, 0);
            } else {
                m.mv = self.col_mv(scup_co);
            }
        }
        m
    }

    fn umve_base(&self, base_idx: i32) -> (bool, Motion) {
        let base_idx = base_idx.min(1);
        let nbs = self.umve_neighbors();
        let mut out = Motion { mv: [[0; 2]; 2], refi: [REFI_INVALID; 2] };
        let mut cnt = 0;
        for i in 0..5 {
            if cnt > base_idx {
                break;
            }
            if nbs.valid[i] {
                if cnt == base_idx {
                    let n = nbs.idx[i];
                    let (r, m) = (self.refi_at(n), self.mv_at(n));
                    for l in 0..2 {
                        if r[l] >= 0 {
                            out.mv[l] = m[l];
                            out.refi[l] = r[l];
                        }
                    }
                }
                cnt += 1;
            }
        }
        if cnt <= base_idx {
            if cnt == base_idx {
                return (true, out);
            }
            cnt += 1;
        }
        if cnt <= base_idx {
            out.refi = [0, REFI_INVALID];
            out.mv[0] = [0, 0];
        }
        (false, out)
    }

    fn umve_final(&mut self, umve_idx: i32, base: &Motion) {
        let base_idx = umve_idx / 20;
        let step = (umve_idx - base_idx * 20) / 4;
        let direction = umve_idx - base_idx * 20 - step * 4;
        let cands = [1i32, 2, 4, 8, 16];
        let (mut d0, mut d1) = (cands[step.clamp(0, 4) as usize], cands[step.clamp(0, 4) as usize]);
        let (r0, r1) = (base.refi[0], base.refi[1]);
        self.refi = [REFI_INVALID, REFI_INVALID];
        let off = |d: i32| -> [i32; 2] {
            match direction {
                0 => [d, 0],
                1 => [-d, 0],
                2 => [0, d],
                _ => [0, -d],
            }
        };
        let apply = |b: Mv, o: [i32; 2]| -> Mv { [clip_i16(i64::from(b[0]) + i64::from(o[0])), clip_i16(i64::from(b[1]) + i64::from(o[1]))] };
        if r0 >= 0 && r1 >= 0 {
            let dist0 = self.dist(r0, 0) << 1;
            let dist1 = self.dist(r1, 1) << 1;
            let (w0, w1, mut s0, mut s1);
            s0 = 1;
            s1 = 1;
            if dist1.abs() >= dist0.abs() {
                w0 = (1 << MV_SCALE_PREC) / dist1.abs().max(1) * dist0.abs();
                w1 = 1 << MV_SCALE_PREC;
                if dist0 * dist1 < 0 {
                    s0 = -1;
                }
            } else {
                w0 = 1 << MV_SCALE_PREC;
                w1 = (1 << MV_SCALE_PREC) / dist0.abs().max(1) * dist1.abs();
                if dist0 * dist1 < 0 {
                    s1 = -1;
                }
            }
            d0 = (w0 * d0 + (1 << (MV_SCALE_PREC - 1))) >> MV_SCALE_PREC;
            d1 = (w1 * d1 + (1 << (MV_SCALE_PREC - 1))) >> MV_SCALE_PREC;
            d0 = (s0 * d0).clamp(-(1 << 15), (1 << 15) - 1);
            d1 = (s1 * d1).clamp(-(1 << 15), (1 << 15) - 1);
            self.mv[0] = apply(base.mv[0], off(d0));
            self.mv[1] = apply(base.mv[1], off(d1));
            self.refi = [r0, r1];
        } else if r0 >= 0 {
            self.mv[0] = apply(base.mv[0], off(d0));
            self.refi[0] = r0;
        } else if r1 >= 0 {
            self.mv[1] = apply(base.mv[1], off(d1));
            self.refi[1] = r1;
        }
    }

    pub(crate) fn update_hmvp(&mut self) {
        let max = self.seq.num_hmvp_cand as usize;
        let new = self.motion_at(self.cu_scup);
        let n = self.hmvp_cnt;
        for i in (0..n).rev() {
            if self.hmvp[i].same(&new) {
                for k in i..n - 1 {
                    self.hmvp[k] = self.hmvp[k + 1];
                }
                self.hmvp[n - 1] = new;
                return;
            }
        }
        if n < max {
            self.hmvp[n] = new;
            self.hmvp_cnt += 1;
        } else if max > 0 {
            for i in 1..max {
                self.hmvp[i - 1] = self.hmvp[i];
            }
            self.hmvp[max - 1] = new;
        }
    }

    pub(crate) fn derive_skip_mv_umve(&mut self, umve_idx: i32) {
        let (temporal, mut base) = self.umve_base(umve_idx / 20);
        if temporal {
            base = self.temporal_motion();
        }
        self.umve_final(umve_idx, &base);
    }

    pub(crate) fn derive_skip_mv(&mut self, idx: i32) {
        const TRAD: i32 = 4;
        if idx == 0 {
            let m = self.temporal_motion();
            self.mv = m.mv;
            self.refi = m.refi;
        } else if idx < TRAD {
            let l = self.mhb_skip_motions();
            let m = l[(idx - 1) as usize];
            self.mv = m.mv;
            self.refi = m.refi;
        } else {
            let num_hmvp = self.seq.num_hmvp_cand as i32;
            let mut cands = [Motion::default(); 12];
            cands[0] = self.temporal_motion();
            let sp = self.mhb_skip_motions();
            cands[1..4].copy_from_slice(&sp);
            let mut cnt = TRAD; // PRED_DIR_NUM + 1
            let mut k = self.hmvp_cnt as i32 - 1;
            while k >= 0 && cnt <= idx {
                let m = self.hmvp[k as usize];
                let dup = cands[..cnt as usize].iter().any(|c| c.same(&m));
                if !dup && cnt < TRAD + num_hmvp {
                    cands[cnt as usize] = m;
                    cnt += 1;
                }
                k -= 1;
            }
            let m = if idx > cnt - 1 { if self.hmvp_cnt > 0 { self.hmvp[self.hmvp_cnt - 1] } else { cands[(TRAD - 1) as usize] } } else { cands[idx as usize] };
            self.mv = m.mv;
            self.refi = m.refi;
        }
    }

    // ---------------------------------------------------------------- affine

    fn affine_model_mv(&self, scup: i32, scun: i32, lidx: usize, vertex_num: &mut i32) -> [[i32; 2]; 3] {
        let i_scu = self.i_scu;
        let pos = self.pos(scun);
        let neb_lw = (pos & 7) as i32;
        let neb_lh = ((pos >> 3) & 7) as i32;
        let (neb_w, neb_h) = (1 << neb_lw, 1 << neb_lh);
        let (diff_w, diff_h) = (7 - neb_lw, 7 - neb_lh);
        let n0 = (pos >> 6) as i32;
        let neighbor = [n0, n0 + ((neb_w >> 2) - 1), n0 + ((neb_h >> 2) - 1) * i_scu, n0 + ((neb_h >> 2) - 1) * i_scu + ((neb_w >> 2) - 1)];
        let neb_x = (n0 % i_scu) << 2;
        let mut neb_y = (n0 / i_scu) << 2;
        let cur_x = (scup % i_scu) << 2;
        let cur_y = (scup / i_scu) << 2;
        let mut nmv = [[0i32; 2]; 4];
        for (i, &n) in neighbor.iter().enumerate() {
            let m = self.mv_at(n)[lidx];
            nmv[i] = [i32::from(m[0]), i32::from(m[1])];
        }
        let mut top_ctu = false;
        let max_cu = self.seq.max_cu as i32;
        if (neb_y + neb_h) % max_cu == 0 && (neb_y + neb_h) == cur_y {
            top_ctu = true;
            neb_y += neb_h;
            nmv[0] = nmv[2];
            nmv[1] = nmv[3];
        }
        let dhx = (nmv[1][0] - nmv[0][0]) << diff_w;
        let dhy = (nmv[1][1] - nmv[0][1]) << diff_w;
        let (dvx, dvy);
        if *vertex_num == 3 && !top_ctu {
            dvx = (nmv[2][0] - nmv[0][0]) << diff_h;
            dvy = (nmv[2][1] - nmv[0][1]) << diff_h;
        } else {
            dvx = -dhy;
            dvy = dhx;
            *vertex_num = 2;
        }
        let hor_base = nmv[0][0] << 7;
        let ver_base = nmv[0][1] << 7;
        let mut out = [[0i32; 2]; 3];
        let at = |ox: i32, oy: i32| -> [i32; 2] {
            let th = dhx.wrapping_mul(cur_x - neb_x + ox).wrapping_add(dvx.wrapping_mul(cur_y - neb_y + oy)).wrapping_add(hor_base);
            let tv = dhy.wrapping_mul(cur_x - neb_x + ox).wrapping_add(dvy.wrapping_mul(cur_y - neb_y + oy)).wrapping_add(ver_base);
            let (h, v) = mv_rounding_shift7(th, tv);
            [(h << 2).clamp(CPMV_MIN, CPMV_MAX), (v << 2).clamp(CPMV_MIN, CPMV_MAX)]
        };
        out[0] = at(0, 0);
        out[1] = at(self.cu_w, 0);
        if *vertex_num == 3 {
            out[2] = at(0, self.cu_h);
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn affine_constructed(
        &self,
        cp_valid: &[bool; 4],
        cp_mv: &[[Mv; 4]; 2],
        cp_refi: &[[i8; 4]; 2],
        cp_idx: &[usize],
        model_idx: usize,
        ver_num: usize,
        cnt: &mut i32,
        mrg_idx: i32,
        out: &mut AffineCand,
    ) -> bool {
        let mut valid_model = [false; 2];
        if cp_idx.iter().take(ver_num).any(|&i| !cp_valid[i]) {
            return false;
        }
        for l in 0..2 {
            let r0 = cp_refi[l][cp_idx[0]];
            valid_model[l] = r0 >= 0 && cp_idx.iter().take(ver_num).all(|&i| cp_refi[l][i] == r0);
        }
        if !valid_model[0] && !valid_model[1] {
            return false;
        }
        out.cp_num = ver_num as i32;
        if *cnt != mrg_idx {
            *cnt += 1;
            return true;
        }
        let (lw, lh) = (log2_of(self.cu_w), log2_of(self.cu_h));
        for l in 0..2 {
            if valid_model[l] {
                let mut t = [[0i32; 2]; 4];
                for &i in cp_idx.iter().take(ver_num) {
                    t[i] = [i32::from(cp_mv[l][i][0]), i32::from(cp_mv[l][i][1])];
                }
                match model_idx {
                    1 => {
                        t[2][0] = t[3][0] + t[0][0] - t[1][0];
                        t[2][1] = t[3][1] + t[0][1] - t[1][1];
                    }
                    2 => {
                        t[1][0] = t[3][0] + t[0][0] - t[2][0];
                        t[1][1] = t[3][1] + t[0][1] - t[2][1];
                    }
                    3 => {
                        t[0][0] = t[1][0] + t[2][0] - t[3][0];
                        t[0][1] = t[1][1] + t[2][1] - t[3][1];
                    }
                    5 => {
                        let sh = 7 + lw - lh;
                        let th = ((t[2][1] - t[0][1]) << sh) + (t[0][0] << 7);
                        let tv = -((t[2][0] - t[0][0]) << sh) + (t[0][1] << 7);
                        let (h, v) = mv_rounding_shift7(th, tv);
                        t[1] = [h, v];
                    }
                    _ => {}
                }
                out.refi[l] = cp_refi[l][cp_idx[0]];
                for i in 0..ver_num {
                    out.mv[l][i] = [(t[i][0] << 2).clamp(CPMV_MIN, CPMV_MAX), (t[i][1] << 2).clamp(CPMV_MIN, CPMV_MAX)];
                }
            } else {
                out.refi[l] = -1;
            }
        }
        *cnt += 1;
        true
    }

    fn affine_merge_candidate(&self, mrg_idx: i32) -> AffineCand {
        let scup = self.cu_scup;
        let i = self.i_scu;
        let (wi, hi) = (self.cu_w >> 2, self.cu_h >> 2);
        let mut cand = AffineCand::default();
        let mut cnt = 0;
        // model based
        {
            let nb = [scup + i * (hi - 1) - 1, scup - i + wi - 1, scup - i + wi, scup - 1, scup - i - 1];
            let max_cand = (mrg_idx + 1).min(2);
            let mut valid = [false; 5];
            let mut tl = [0i32; 5];
            for k in 0..5 {
                valid[k] = self.affine_of(nb[k]) != 0;
                if valid[k] {
                    tl[k] = (self.pos(nb[k]) >> 6) as i32;
                }
            }
            if valid[2] && valid[1] && tl[1] == tl[2] {
                valid[2] = false;
            }
            let v3 = valid[3];
            if valid[3] && valid[0] && tl[0] == tl[3] {
                valid[3] = false;
            }
            if (valid[4] && v3 && tl[4] == tl[3]) || (valid[4] && valid[1] && tl[4] == tl[1]) {
                valid[4] = false;
            }
            let mut k = 0;
            while k < 5 && cnt < max_cand {
                if valid[k] {
                    if cnt == mrg_idx {
                        cand.cp_num = if self.affine_of(nb[k]) == 1 { 2 } else { 3 };
                        for l in 0..2 {
                            cand.refi[l] = self.refi_at(nb[k])[l];
                            if cand.refi[l] >= 0 {
                                let mut vn = cand.cp_num;
                                let r = self.affine_model_mv(scup, nb[k], l, &mut vn);
                                cand.cp_num = vn;
                                for q in 0..3 {
                                    cand.mv[l][q] = [r[q][0], r[q][1]];
                                }
                            }
                        }
                    }
                    cnt += 1;
                }
                k += 1;
            }
        }
        // control point based
        if cnt <= mrg_idx {
            let max_cand = (mrg_idx + 1).min(5);
            let mut cp_mv = [[[0i16; 2]; 4]; 2];
            let mut cp_refi = [[-1i8; 4]; 2];
            let mut cp_valid = [false; 4];
            for &n in &[scup - 1, scup - i, scup - i - 1] {
                if self.is_inter(n) {
                    for l in 0..2 {
                        cp_refi[l][0] = self.refi_at(n)[l];
                        cp_mv[l][0] = self.mv_at(n)[l];
                    }
                    cp_valid[0] = true;
                    break;
                }
            }
            for &n in &[scup - i + wi - 1, scup - i + wi] {
                if self.is_inter(n) {
                    for l in 0..2 {
                        cp_refi[l][1] = self.refi_at(n)[l];
                        cp_mv[l][1] = self.mv_at(n)[l];
                    }
                    cp_valid[1] = true;
                    break;
                }
            }
            let lb = scup + i * (hi - 1) - 1;
            if self.is_inter(lb) {
                for l in 0..2 {
                    cp_refi[l][2] = self.refi_at(lb)[l];
                    cp_mv[l][2] = self.mv_at(lb)[l];
                }
                cp_valid[2] = true;
            }
            let rb = scup + i * (hi - 1) + (wi - 1);
            let (scup_co, _) = self.colocal_scup(rb);
            if self.ph.slice_type == SliceType::B {
                let ok = self.refp.first().and_then(|r| r[1].pic.as_ref()).and_then(|p| p.map_refi.get(self.ix(scup_co))).is_some_and(|r| r[0] >= 0);
                if ok {
                    let m = self.col_mv(scup_co);
                    for l in 0..2 {
                        cp_refi[l][3] = 0;
                        cp_mv[l][3] = m[l];
                    }
                    cp_valid[3] = true;
                }
            } else {
                let ok = self.refp.first().and_then(|r| r[0].pic.as_ref()).and_then(|p| p.map_refi.get(self.ix(scup_co))).is_some_and(|r| r[0] >= 0);
                if ok {
                    cp_refi[0][3] = 0;
                    cp_mv[0][3] = self.col_mv_from_list0(scup_co);
                    cp_refi[1][3] = REFI_INVALID;
                    cp_valid[3] = true;
                }
            }
            const CP_NUM: [usize; 6] = [3, 3, 3, 3, 2, 2];
            const MODEL: [[usize; 3]; 6] = [[0, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3], [0, 1, 0], [0, 2, 0]];
            for idx in 0..6 {
                if cnt >= max_cand {
                    break;
                }
                self.affine_constructed(&cp_valid, &cp_mv, &cp_refi, &MODEL[idx], idx, CP_NUM[idx], &mut cnt, mrg_idx, &mut cand);
            }
        }
        if cnt <= mrg_idx {
            cand = AffineCand { cp_num: 2, refi: [0, REFI_INVALID], mv: [[[0; 2]; 4]; 2] };
        }
        cand
    }

    pub(crate) fn derive_skip_mv_affine(&mut self, mrg_idx: i32) {
        let c = self.affine_merge_candidate(mrg_idx);
        self.affine_flag = (c.cp_num - 1).clamp(0, 2) as u8;
        for l in 0..2 {
            if c.refi[l] >= 0 {
                self.refi[l] = c.refi[l];
                for cp in 0..(c.cp_num.clamp(0, 4) as usize) {
                    self.affine_mv[l][cp] = c.mv[l][cp];
                }
            } else {
                self.refi[l] = REFI_INVALID;
            }
        }
    }

    /// Affine MVP of the two control points of an AMVP block.
    pub(crate) fn scale_affine_mvp(&self, lidx: usize, curr_mvr: i32) -> [[i32; 2]; 2] {
        let cur_refi = self.refi[lidx];
        let scup = self.cu_scup;
        let i = self.i_scu;
        let wi = self.cu_w >> 2;
        let dist_cur = self.dist(cur_refi, lidx);
        let try_nb = |n: i32| -> Option<Mv> {
            let r = self.refi_at(n)[lidx];
            if self.is_inter(n) && r >= 0 { Some(scaling_mv(dist_cur, self.dist(r, lidx), self.mv_at(n)[lidx])) } else { None }
        };
        let lt = try_nb(scup - 1).or_else(|| try_nb(scup - i)).or_else(|| try_nb(scup - i - 1)).unwrap_or([0, 0]);
        let rt = try_nb(scup - i + wi - 1).or_else(|| try_nb(scup - i + wi)).unwrap_or([0, 0]);
        let shift = match curr_mvr {
            0 => 2,
            1 => 4,
            _ => 0,
        };
        let add = if shift != 0 { 1 << (shift - 1) } else { 0 };
        [mv_rounding_s32(lt, shift, add), mv_rounding_s32(rt, shift, add)]
    }

    /// Stores the sub-block motion field of the current affine CU into the picture's map and
    /// `affine_sb_mv`.
    pub(crate) fn set_affine_mvf(&mut self) {
        let i_scu = self.i_scu;
        let (lw, lh) = (self.log2w as i32, self.log2h as i32);
        let (wi, hi) = ((1 << lw) >> 2, (1 << lh) >> 2);
        let cp_num = i32::from(self.affine_flag) + 1;
        let base = self.ix(self.cu_scup);
        self.affine_sb_mv.clear();
        let bi = self.refi[0] >= 0 && self.refi[1] >= 0;
        let put = |s: &mut Core, dx: i32, dy: i32, lidx: usize, mv: Mv| {
            let k = base + (dy * i_scu + dx) as usize;
            if let Some(e) = s.cur.map_mv.get_mut(k) {
                e[lidx] = mv;
            }
        };
        let params = |ac: &[[i32; 2]; 4]| -> (i32, i32, i32, i32, i32, i32) {
            let hx = ((ac[1][0] - ac[0][0]) << 7) >> lw;
            let hy = ((ac[1][1] - ac[0][1]) << 7) >> lw;
            let (vx, vy) = if cp_num == 3 { (((ac[2][0] - ac[0][0]) << 7) >> lh, ((ac[2][1] - ac[0][1]) << 7) >> lh) } else { (-hy, hx) };
            (ac[0][0] << 7, ac[0][1] << 7, hx, hy, vx, vy)
        };
        let calc = |p: &(i32, i32, i32, i32, i32, i32), w: i32, h: i32, step: i32, last_w: bool, last_h: bool| -> (i32, i32) {
            let (bx, by, hx, hy, vx, vy) = *p;
            if w == 0 && h == 0 {
                (bx, by)
            } else if last_w && h == 0 {
                (bx + (hx << lw), by + (hy << lw))
            } else if w == 0 && last_h && cp_num == 3 {
                (bx + (vx << lh), by + (vy << lh))
            } else {
                let half = if step == 2 { 4 } else { 2 };
                let px = (w << 2) + half;
                let py = (h << 2) + half;
                (bx + hx * px + vx * py, by + hy * px + vy * py)
            }
        };
        if bi {
            let p0 = params(&self.affine_mv[0]);
            let p1 = params(&self.affine_mv[1]);
            let mut h = 0;
            while h < hi {
                let mut w = 0;
                while w < wi {
                    let (a0, a1) = calc(&p0, w, h, 2, w + 2 == wi, h + 2 == hi);
                    let (b0, b1) = calc(&p1, w, h, 2, w + 2 == wi, h + 2 == hi);
                    let (m18_0, m0) = mv_rounding_affine(a0, a1);
                    let (m18_1, m1) = mv_rounding_affine(b0, b1);
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        put(self, w + dx, h + dy, 0, m0);
                        put(self, w + dx, h + dy, 1, m1);
                    }
                    self.affine_sb_mv.push([m18_0, m18_1]);
                    w += 2;
                }
                h += 2;
            }
        } else {
            let lidx = if self.refi[0] >= 0 { 0 } else { 1 };
            let p = params(&self.affine_mv[lidx]);
            let sub = if self.ph.affine_subblock_size_idx == 1 { 8 } else { 4 };
            if sub == 4 {
                for h in 0..hi {
                    for w in 0..wi {
                        let (a, b) = calc(&p, w, h, 1, w == wi - 1, h == hi - 1);
                        let (m18, m) = mv_rounding_affine(a, b);
                        put(self, w, h, lidx, m);
                        self.affine_sb_mv.push([m18, [0, 0]]);
                    }
                }
            } else {
                let mut h = 0;
                while h < hi {
                    let mut w = 0;
                    while w < wi {
                        let (a, b) = calc(&p, w, h, 2, w + 2 == wi, h + 2 == hi);
                        let (m18, m) = mv_rounding_affine(a, b);
                        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                            put(self, w + dx, h + dy, lidx, m);
                        }
                        self.affine_sb_mv.push([m18, [0, 0]]);
                        w += 2;
                    }
                    h += 2;
                }
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct AffineCand {
    pub cp_num: i32,
    pub refi: [i8; 2],
    pub mv: [[[i32; 2]; 4]; 2],
}
