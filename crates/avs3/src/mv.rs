// SPDX-License-Identifier: BSD-3-Clause
// Derived from uavs3d (`source/decoder/dec_util.c`), Copyright (c) 2018-2022 Peking University Shenzhen Graduate
// School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation. See LICENSE.uavs3d.
//! Motion vector prediction and merge / skip candidate derivation.

use crate::core::{Core, Picture, RefPic, mcu_get_logh, mcu_get_logw, mcu_get_scup};
use crate::header::SliceType;
use crate::types::*;

fn nz(d: i32) -> i32 {
    if d == 0 { 1 } else { d }
}

fn clip16(v: i64) -> i16 {
    v.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16
}

pub(crate) fn scaling_mv(dist_cur: i32, dist_neb: i32, mvp: Mv) -> Mv {
    let mut mv = [0i16; 2];
    if mvp != [0, 0] {
        let dist_neb = nz(dist_neb);
        if dist_neb == dist_cur && (1 << MV_SCALE_PREC) % dist_neb == 0 {
            mv = mvp;
        } else {
            let offset = 1i64 << (MV_SCALE_PREC - 1);
            let ratio = (offset as i32 / dist_neb).wrapping_mul(dist_cur).wrapping_shl(1);
            for k in 0..2 {
                if mvp[k] != 0 {
                    let tmp = i64::from(mvp[k]) * i64::from(ratio);
                    let mask = tmp >> 63;
                    let tmp = (mask ^ tmp) - mask;
                    let tmp = (mask ^ ((tmp + offset) >> MV_SCALE_PREC)) - mask;
                    mv[k] = clip16(tmp);
                }
            }
        }
    }
    mv
}

fn scaling_mv_amvr(dist_cur: i32, dist_neb: i32, mvp: Mv, amvr_idx: i32) -> Mv {
    let mut mv = [0i16; 2];
    if mvp != [0, 0] {
        let dist_neb = nz(dist_neb);
        let add = if amvr_idx > 0 { 1i64 << (amvr_idx - 1) } else { 0 };
        let amvr_mask = !((1i64 << amvr_idx) - 1);
        if dist_neb == dist_cur && (1 << MV_SCALE_PREC) % dist_neb == 0 {
            for k in 0..2 {
                if mvp[k] != 0 {
                    let tmp = i64::from(mvp[k]);
                    let mask = tmp >> 63;
                    let tmp = (mask ^ tmp) - mask;
                    let tmp = (mask ^ ((tmp + add) & amvr_mask)) - mask;
                    mv[k] = clip16(tmp);
                }
            }
        } else {
            let mut offset = 1i64 << (MV_SCALE_PREC - 1);
            let ratio = (offset as i32 / dist_neb).wrapping_mul(dist_cur).wrapping_shl(1);
            offset += add << MV_SCALE_PREC;
            for k in 0..2 {
                if mvp[k] != 0 {
                    let tmp = i64::from(mvp[k]) * i64::from(ratio);
                    let mask = tmp >> 63;
                    let tmp = (mask ^ tmp) - mask;
                    let tmp = (mask ^ (((tmp + offset) >> MV_SCALE_PREC) & amvr_mask)) - mask;
                    mv[k] = clip16(tmp);
                }
            }
        }
    }
    mv
}

fn mv_rounding_s16(mvp: &mut Mv, shift: i32) {
    let add = if shift != 0 { 1 << (shift - 1) } else { 0 };
    for v in mvp.iter_mut() {
        if *v != 0 {
            let t = i32::from(*v);
            let mask = t >> 31;
            let t = (mask ^ t) - mask;
            let t = ((t + add) >> shift) << shift;
            let t = (mask ^ t) - mask;
            *v = clip16(i64::from(t));
        }
    }
}

fn mv_rounding_s32(mvp: Mv, shift: i32, add: i32) -> [i32; 2] {
    let mut mv = [0i32; 2];
    for k in 0..2 {
        if mvp[k] != 0 {
            let t = i32::from(mvp[k]) << 2;
            let mask = t >> 31;
            let t = (mask ^ t) - mask;
            let t = ((t + add) >> shift) << shift;
            let t = (mask ^ t) - mask;
            mv[k] = clip3(CPMV_MIN, CPMV_MAX, t);
        }
    }
    mv
}

fn round7(v: i32) -> i32 {
    if v == 0 {
        0
    } else {
        let mask = v >> 31;
        let a = (mask ^ v) - mask;
        let a = (a + 64) >> 7;
        (mask ^ a) - mask
    }
}

fn mv_rounding_s32_shift7(hor: i32, ver: i32) -> (i32, i32) {
    (round7(hor), round7(ver))
}

/// `com_mv_rounding_affine`: returns the 1/16 pel vector (18 bit) and the stored 1/4 pel one.
pub(crate) fn mv_rounding_affine(v: i32) -> (i32, i16) {
    const ADD1: i32 = 1 << 6;
    const ADD2: i32 = 1 << 1;
    if v == 0 {
        (0, 0)
    } else if v > 0 {
        let m1 = ((v + ADD1) >> 7).min(131071);
        let val = (m1 + ADD2) >> 2;
        (m1, val.min(i32::from(i16::MAX)) as i16)
    } else {
        let t = (-v + ADD1) >> 7;
        let m1 = (-t).max(-131072);
        let val = -((t + ADD2) >> 2);
        (m1, val.max(i32::from(i16::MIN)) as i16)
    }
}

fn amvr_shift_affine(x: i32) -> i32 {
    match x {
        0 => 2,
        1 => 4,
        _ => 0,
    }
}

impl Picture {
    fn col_ix(&self, scup: i32, i_scu: usize) -> usize {
        (scup + i_scu as i32 + 1) as usize
    }
}

impl Core {
    fn colocal_scup(&self, scup: i32) -> (i32, i32) {
        let i_scu = self.seq.i_scu as i32;
        let (w, h) = (self.seq.pic_width_in_scu as i32, self.seq.pic_height_in_scu as i32);
        let mask = !3;
        let bx = scup % i_scu;
        let by = scup / i_scu;
        let mut xpos = (bx & mask) + 2;
        let mut ypos = (by & mask) + 2;
        if ypos >= h {
            ypos = ((by & mask) + h) >> 1;
        }
        if xpos >= w {
            xpos = ((bx & mask) + w) >> 1;
        }
        (ypos * i_scu + xpos, ypos)
    }

    fn rp(&self, refi: i8, lidx: usize) -> Option<&RefPic> {
        self.refp.get(refi.max(0) as usize).and_then(|r| r[lidx].as_ref())
    }

    fn col_mv_list0(&self, p: &RefPic, scup_co: i32, out: &mut Mv) {
        let ix = p.pic.col_ix(scup_co, self.seq.i_scu);
        let refi = p.pic.map_refi.get(ix).map_or(REFI_INVALID, |r| r[0]);
        let dist_col = i32::from(p.pic.list_dist.get(refi.max(0) as usize).copied().unwrap_or(1));
        let mv = p.pic.map_mv.get(ix).map_or([0, 0], |m| m[0]);
        *out = scaling_mv(p.dist, dist_col, mv);
    }

    fn col_mv(&self, scup_co: i32) -> [Mv; 2] {
        let (Some(p0), Some(p1)) = (self.rp(0, 0), self.rp(0, 1)) else {
            return [[0; 2]; 2];
        };
        let ix = p1.pic.col_ix(scup_co, self.seq.i_scu);
        let mut refi = p1.pic.map_refi.get(ix).map_or(REFI_INVALID, |r| r[0]);
        if refi == REFI_INVALID {
            refi = 0;
        }
        let dist_col = i32::from(p1.pic.list_dist.get(refi as usize).copied().unwrap_or(1));
        let mv = p1.pic.map_mv.get(ix).map_or([0, 0], |m| m[0]);
        [scaling_mv(p0.dist, dist_col, mv), scaling_mv(p1.dist, dist_col, mv)]
    }

    /// `get_mvp_default`
    fn mvp_default(&self, lidx: usize, cur_refi: i8, amvr_idx: i32) -> Mv {
        let m = &self.maps;
        let scup = self.cu_scup;
        let i_scu = self.seq.i_scu as i32;
        let dist_cur = self.refp_dist(cur_refi, lidx);
        let neighbor = [scup - 1, scup - i_scu, scup - i_scu + (self.cu_width >> 2) as i32];
        let mut mvps = [[0i16; 2]; 3];
        let rf = |n: i32| -> i8 { if m.scu[m.ix(n)].coded { m.refi[m.ix(n)][lidx] } else { REFI_INVALID } };
        let r_l = rf(neighbor[0]);
        let r_u = rf(neighbor[1]);
        let mut r_ur = rf(neighbor[2]);
        let mut nb2 = neighbor[2];
        if r_l >= 0 {
            mvps[0] = scaling_mv(dist_cur, self.refp_dist(r_l, lidx), m.mv[m.ix(neighbor[0])][lidx]);
        }
        if r_u >= 0 {
            mvps[1] = scaling_mv(dist_cur, self.refp_dist(r_u, lidx), m.mv[m.ix(neighbor[1])][lidx]);
        }
        if r_ur >= 0 {
            mvps[2] = scaling_mv(dist_cur, self.refp_dist(r_ur, lidx), m.mv[m.ix(nb2)][lidx]);
        } else {
            nb2 = scup - i_scu - 1;
            r_ur = rf(nb2);
            if r_ur >= 0 {
                mvps[2] = scaling_mv(dist_cur, self.refp_dist(r_ur, lidx), m.mv[m.ix(nb2)][lidx]);
            }
        }
        let mut mvp = [0i16; 2];
        if r_l != REFI_INVALID && r_u == REFI_INVALID && r_ur == REFI_INVALID {
            mvp = mvps[0];
        } else if r_l == REFI_INVALID && r_u != REFI_INVALID && r_ur == REFI_INVALID {
            mvp = mvps[1];
        } else if r_l == REFI_INVALID && r_u == REFI_INVALID && r_ur != REFI_INVALID {
            mvp = mvps[2];
        } else {
            for hv in 0..2 {
                let mva = i32::from(mvps[0][hv]);
                let mvb = i32::from(mvps[1][hv]);
                let mvc = i32::from(mvps[2][hv]);
                mvp[hv] = if (mva < 0 && mvb > 0 && mvc > 0) || (mva > 0 && mvb < 0 && mvc < 0) {
                    ((mvb + mvc) / 2) as i16
                } else if (mvb < 0 && mva > 0 && mvc > 0) || (mvb > 0 && mva < 0 && mvc < 0) {
                    ((mvc + mva) / 2) as i16
                } else if (mvc < 0 && mva > 0 && mvb > 0) || (mvc > 0 && mva < 0 && mvb < 0) {
                    ((mva + mvb) / 2) as i16
                } else {
                    let a = (mva - mvb).abs();
                    let b = (mvb - mvc).abs();
                    let c = (mvc - mva).abs();
                    let pred_vec = a.min(b.min(c));
                    if pred_vec == a {
                        ((mva + mvb) / 2) as i16
                    } else if pred_vec == b {
                        ((mvb + mvc) / 2) as i16
                    } else {
                        ((mvc + mva) / 2) as i16
                    }
                };
            }
        }
        mv_rounding_s16(&mut mvp, amvr_idx);
        mvp
    }

    fn mvp_hmvp(&self, motion: &Motion, lidx: usize, cur_refi: i8, amvr_idx: i32) -> Mv {
        let dist_cur = self.refp_dist(cur_refi, lidx);
        let mut l = lidx;
        let mut refi = motion.refi[l];
        if refi == REFI_INVALID {
            l = 1 - l;
            refi = motion.refi[l];
        }
        scaling_mv_amvr(dist_cur, self.refp_dist(refi, l), motion.mv[l], amvr_idx)
    }

    /// `dec_derive_mvp`
    pub(crate) fn derive_mvp(&self, ref_list: usize, emvp_flag: bool, mvr_idx: i32) -> Mv {
        let ref_idx = self.refi[ref_list];
        if !emvp_flag {
            return self.mvp_default(ref_list, ref_idx, mvr_idx);
        }
        let cnt = self.hmvp_cands_cnt;
        if cnt == 0 {
            [0, 0]
        } else if cnt < mvr_idx as usize + 1 {
            self.mvp_hmvp(&self.hmvp_cands_list[cnt - 1], ref_list, ref_idx, mvr_idx)
        } else {
            self.mvp_hmvp(&self.hmvp_cands_list[cnt - 1 - mvr_idx as usize], ref_list, ref_idx, mvr_idx)
        }
    }

    /// `derive_mhb_skip_motions`: fills `list[0..3]`.
    fn derive_mhb_skip_motions(&self, list: &mut [Motion]) {
        let m = &self.maps;
        let scup = self.cu_scup;
        let i_scu = self.seq.i_scu as i32;
        let wi = (self.cu_width >> 2) as i32;
        let hi = (self.cu_height >> 2) as i32;
        for l in list.iter_mut().take(3) {
            *l = Motion::default();
        }
        list[2].refi[1] = REFI_INVALID;
        list[1].refi[0] = REFI_INVALID;
        let neighbor = [scup + (hi - 1) * i_scu - 1, scup - i_scu + wi - 1, scup - i_scu + wi, scup - 1, scup - i_scu, scup - i_scu - 1];
        let (mut l0_found, mut l1_found, mut bi_found) = (false, false, false);
        let mut last_bi_idx = 0usize;
        for (k, &n) in neighbor.iter().enumerate() {
            let ix = m.ix(n);
            if m.scu[ix].inter {
                let refi = m.refi[ix];
                let v0 = refi[0] >= 0;
                let v1 = refi[1] >= 0;
                let r0 = if v0 { refi[0] } else { REFI_INVALID };
                let r1 = if v1 { refi[1] } else { REFI_INVALID };
                if v0 && !v1 && !l0_found {
                    l0_found = true;
                    list[2].mv[0] = m.mv[ix][0];
                    list[2].refi[0] = r0;
                }
                if !v0 && v1 && !l1_found {
                    l1_found = true;
                    list[1].mv[1] = m.mv[ix][1];
                    list[1].refi[1] = r1;
                }
                if v0 && v1 {
                    if !bi_found {
                        list[0].mv[0] = m.mv[ix][0];
                        list[0].mv[1] = m.mv[ix][1];
                        list[0].refi = [r0, r1];
                    }
                    bi_found = true;
                    last_bi_idx = k;
                }
            }
        }
        if l0_found && l1_found && !bi_found {
            list[0].mv[0] = list[2].mv[0];
            list[0].mv[1] = list[1].mv[1];
            list[0].refi[0] = list[2].refi[0];
            list[0].refi[1] = list[1].refi[1];
        }
        let lb = m.ix(neighbor[last_bi_idx]);
        if !l0_found && bi_found {
            list[2].mv[0] = m.mv[lb][0];
            list[2].refi[0] = m.refi[lb][0];
        }
        if !l1_found && bi_found {
            list[1].mv[1] = m.mv[lb][1];
            list[1].refi[1] = m.refi[lb][1];
        }
    }

    /// `derive_temporal_motions`
    fn derive_temporal_motions(&self, mv: &mut [Mv; 2], refi: &mut [i8; 2]) {
        let (scup_co, _) = self.colocal_scup(self.cu_scup);
        *refi = [0, 0];
        if self.slice_type == SliceType::P {
            refi[1] = -1;
            if let Some(p) = self.rp(0, 0) {
                let ix = p.pic.col_ix(scup_co, self.seq.i_scu);
                if p.pic.map_refi.get(ix).is_some_and(|r| r[0] >= 0) {
                    self.col_mv_list0(p, scup_co, &mut mv[0]);
                } else {
                    mv[0] = [0, 0];
                }
            } else {
                mv[0] = [0, 0];
            }
        } else if let Some(p) = self.rp(0, 1) {
            let ix = p.pic.col_ix(scup_co, self.seq.i_scu);
            if !p.pic.map_refi.get(ix).is_some_and(|r| r[0] >= 0) {
                mv[0] = self.mvp_default(0, 0, 0);
                mv[1] = self.mvp_default(1, 0, 0);
            } else {
                *mv = self.col_mv(scup_co);
            }
        } else {
            *mv = [[0; 2]; 2];
        }
    }

    fn umve_availability(&self) -> ([i32; 5], [bool; 5]) {
        let m = &self.maps;
        let scup = self.cu_scup;
        let i_scu = self.seq.i_scu as i32;
        let wi = (self.cu_width >> 2) as i32;
        let hi = (self.cu_height >> 2) as i32;
        let nb = [scup + i_scu * (hi - 1) - 1, scup - i_scu + wi - 1, scup - i_scu + wi, scup - 1, scup - i_scu - 1];
        let tv: Vec<bool> = nb.iter().map(|&n| m.scu[m.ix(n)].inter).collect();
        let mut valid = [tv[0], tv[1], tv[2], tv[3], tv[4]];
        let same = |a: i32, b: i32| same_motion(&m.mv[m.ix(a)], &m.refi[m.ix(a)], &m.mv[m.ix(b)], &m.refi[m.ix(b)]);
        if tv[0] && tv[1] {
            valid[1] = !same(nb[1], nb[0]);
        }
        if tv[1] && tv[2] {
            valid[2] = !same(nb[1], nb[2]);
        }
        if tv[0] && tv[3] {
            valid[3] = !same(nb[3], nb[0]);
        }
        if tv[4] {
            valid[4] = (!tv[3] || !same(nb[4], nb[3])) && (!tv[1] || !same(nb[4], nb[1]));
        }
        (nb, valid)
    }

    fn derive_umve_base_motions(&self, base_idx: i32, pmv: &mut [Mv; 2], refi: &mut [i8; 2]) -> bool {
        let m = &self.maps;
        let base_idx = base_idx.min(UMVE_BASE_NUM - 1);
        *refi = [REFI_INVALID, REFI_INVALID];
        let (nb, valid) = self.umve_availability();
        let mut cnt = 0;
        let mut i = 0;
        while i < 5 && cnt <= base_idx {
            if valid[i] {
                if cnt == base_idx {
                    let ix = m.ix(nb[i]);
                    if m.refi[ix][0] >= 0 {
                        pmv[0] = m.mv[ix][0];
                        refi[0] = m.refi[ix][0];
                    }
                    if m.refi[ix][1] >= 0 {
                        pmv[1] = m.mv[ix][1];
                        refi[1] = m.refi[ix][1];
                    }
                }
                cnt += 1;
            }
            i += 1;
        }
        if cnt <= base_idx {
            if cnt == base_idx {
                return true;
            }
            cnt += 1;
        }
        if cnt <= base_idx {
            *refi = [0, REFI_INVALID];
            pmv[0] = [0, 0];
        }
        false
    }

    fn derive_umve_final_motions(&mut self, umve_idx: i32, pmv: &[Mv; 2], base_refi: &[i8; 2]) {
        let base_idx = umve_idx / UMVE_MAX_REFINE_NUM;
        let step = (umve_idx - base_idx * UMVE_MAX_REFINE_NUM) / 4;
        let direction = umve_idx - base_idx * UMVE_MAX_REFINE_NUM - step * 4;
        const CANDS: [i32; 5] = [1, 2, 4, 8, 16];
        let mut ref_mvd0 = CANDS[(step as usize).min(4)];
        let mut ref_mvd1 = ref_mvd0;
        let (refi0, refi1) = (base_refi[0], base_refi[1]);
        self.refi = [REFI_INVALID, REFI_INVALID];
        let offs = |d: i32, v: i32| -> [i32; 2] {
            match d {
                0 => [v, 0],
                1 => [-v, 0],
                2 => [0, v],
                _ => [0, -v],
            }
        };
        let add = |b: Mv, o: [i32; 2]| -> Mv { [clip16(i64::from(b[0]) + i64::from(o[0])), clip16(i64::from(b[1]) + i64::from(o[1]))] };
        if refi0 >= 0 && refi1 >= 0 {
            let dist0 = self.refp_dist(refi0, 0) << 1;
            let dist1 = self.refp_dist(refi1, 1) << 1;
            let (list0_weight, list1_weight);
            let (mut s0, mut s1) = (1, 1);
            if dist1.abs() >= dist0.abs() {
                list0_weight = (1 << MV_SCALE_PREC) / nz(dist1.abs()) * dist0.abs();
                list1_weight = 1 << MV_SCALE_PREC;
                if dist0 * dist1 < 0 {
                    s0 = -1;
                }
            } else {
                list0_weight = 1 << MV_SCALE_PREC;
                list1_weight = (1 << MV_SCALE_PREC) / nz(dist0.abs()) * dist1.abs();
                if dist0 * dist1 < 0 {
                    s1 = -1;
                }
            }
            ref_mvd0 = (list0_weight * ref_mvd0 + (1 << (MV_SCALE_PREC - 1))) >> MV_SCALE_PREC;
            ref_mvd1 = (list1_weight * ref_mvd1 + (1 << (MV_SCALE_PREC - 1))) >> MV_SCALE_PREC;
            ref_mvd0 = clip3(-(1 << 15), (1 << 15) - 1, s0 * ref_mvd0);
            ref_mvd1 = clip3(-(1 << 15), (1 << 15) - 1, s1 * ref_mvd1);
            self.mv[0] = add(pmv[0], offs(direction, ref_mvd0));
            self.refi[0] = refi0;
            self.mv[1] = add(pmv[1], offs(direction, ref_mvd1));
            self.refi[1] = refi1;
        } else if refi0 >= 0 {
            self.mv[0] = add(pmv[0], offs(direction, ref_mvd0));
            self.refi[0] = refi0;
        } else if refi1 >= 0 {
            self.mv[1] = add(pmv[1], offs(direction, ref_mvd1));
            self.refi[1] = refi1;
        }
    }

    /// `update_hmvp_candidates`
    pub(crate) fn update_hmvp_candidates(&mut self) {
        let ix = self.maps.ix(self.cu_scup);
        let new = Motion { mv: self.maps.mv[ix], refi: self.maps.refi[ix] };
        let num = self.hmvp_cands_cnt;
        let max = self.seq.num_of_hmvp_cand as usize;
        for i in (0..num).rev() {
            let c = self.hmvp_cands_list[i];
            if same_motion(&c.mv, &c.refi, &new.mv, &new.refi) {
                for j in i..num - 1 {
                    self.hmvp_cands_list[j] = self.hmvp_cands_list[j + 1];
                }
                self.hmvp_cands_list[num - 1] = new;
                return;
            }
        }
        if num < max {
            self.hmvp_cands_list[num] = new;
            self.hmvp_cands_cnt += 1;
        } else if max > 0 {
            for i in 1..max {
                self.hmvp_cands_list[i - 1] = self.hmvp_cands_list[i];
            }
            self.hmvp_cands_list[max - 1] = new;
        }
    }

    /// `dec_derive_skip_mv_umve`
    pub(crate) fn derive_skip_mv_umve(&mut self, umve_idx: i32) {
        let mut pmv = [[0i16; 2]; 2];
        let mut refi = [0i8; 2];
        let base_idx = umve_idx / UMVE_MAX_REFINE_NUM;
        if self.derive_umve_base_motions(base_idx, &mut pmv, &mut refi) {
            self.derive_temporal_motions(&mut pmv, &mut refi);
        }
        self.derive_umve_final_motions(umve_idx, &pmv, &refi);
    }

    /// `dec_derive_skip_mv`
    pub(crate) fn derive_skip_mv(&mut self, skip_idx: usize) {
        if skip_idx == 0 {
            let (mut mv, mut refi) = (self.mv, self.refi);
            self.derive_temporal_motions(&mut mv, &mut refi);
            self.mv = mv;
            self.refi = refi;
        } else if skip_idx < TRADITIONAL_SKIP_NUM {
            let mut list = [Motion::default(); MAX_SKIP_NUM];
            self.derive_mhb_skip_motions(&mut list);
            self.mv = list[skip_idx - 1].mv;
            self.refi = list[skip_idx - 1].refi;
        } else {
            let mut list = [Motion::default(); MAX_SKIP_NUM];
            let (mut mv, mut refi) = (list[0].mv, list[0].refi);
            self.derive_temporal_motions(&mut mv, &mut refi);
            list[0].mv = mv;
            list[0].refi = refi;
            self.derive_mhb_skip_motions(&mut list[1..]);
            let num_hmvp = self.seq.num_of_hmvp_cand as usize;
            let mut cnt = 4usize;
            let mut k = self.hmvp_cands_cnt as isize - 1;
            while k >= 0 && cnt <= skip_idx {
                let motion = self.hmvp_cands_list[k as usize];
                // fill_skip_candidates
                let mut dup = false;
                for c in list.iter().take(cnt) {
                    if same_motion(&c.mv, &c.refi, &motion.mv, &motion.refi) {
                        dup = true;
                        break;
                    }
                }
                if !dup && cnt < TRADITIONAL_SKIP_NUM + num_hmvp {
                    list[cnt] = motion;
                    cnt += 1;
                }
                k -= 1;
            }
            let pick = if skip_idx > cnt - 1 {
                if self.hmvp_cands_cnt > 0 { self.hmvp_cands_list[self.hmvp_cands_cnt - 1] } else { list[TRADITIONAL_SKIP_NUM - 1] }
            } else {
                list[skip_idx]
            };
            self.mv = pick.mv;
            self.refi = pick.refi;
        }
    }

    // ---------------------------------------------------------------- affine

    /// `derive_affine_model_mv`
    fn affine_model_mv(&self, scup: i32, scun: i32, lidx: usize, vertex_num: &mut i32, mvp: &mut [[i32; 2]; 4]) {
        let m = &self.maps;
        let i_scu = self.seq.i_scu as i32;
        let pos = m.pos[m.ix(scun)];
        let neb_log_w = mcu_get_logw(pos) as i32;
        let neb_log_h = mcu_get_logh(pos) as i32;
        let neb_w = 1i32 << neb_log_w;
        let neb_h = 1i32 << neb_log_h;
        let diff_w = 7 - neb_log_w;
        let diff_h = 7 - neb_log_h;
        let cu_width = self.cu_width as i32;
        let cu_height = self.cu_height as i32;
        let n0 = mcu_get_scup(pos);
        let n1 = n0 + ((neb_w >> 2) - 1);
        let n2 = n0 + ((neb_h >> 2) - 1) * i_scu;
        let n3 = n2 + ((neb_w >> 2) - 1);
        let neb_x = (n0 % i_scu) << 2;
        let mut neb_y = (n0 / i_scu) << 2;
        let cur_x = (scup % i_scu) << 2;
        let cur_y = (scup / i_scu) << 2;
        let mut neb_mv = [m.mv[m.ix(n0)][lidx], m.mv[m.ix(n1)][lidx], m.mv[m.ix(n2)][lidx], m.mv[m.ix(n3)][lidx]];
        let mut top_ctu_boundary = false;
        if (neb_y + neb_h) % (1 << self.seq.log2_max_cuwh) == 0 && (neb_y + neb_h) == cur_y {
            top_ctu_boundary = true;
            neb_y += neb_h;
            neb_mv[0] = neb_mv[2];
            neb_mv[1] = neb_mv[3];
        }
        let dmv_hor_x = (i32::from(neb_mv[1][0]) - i32::from(neb_mv[0][0])) << diff_w;
        let dmv_hor_y = (i32::from(neb_mv[1][1]) - i32::from(neb_mv[0][1])) << diff_w;
        let (dmv_ver_x, dmv_ver_y);
        if *vertex_num == 3 && !top_ctu_boundary {
            dmv_ver_x = (i32::from(neb_mv[2][0]) - i32::from(neb_mv[0][0])) << diff_h;
            dmv_ver_y = (i32::from(neb_mv[2][1]) - i32::from(neb_mv[0][1])) << diff_h;
        } else {
            dmv_ver_x = -dmv_hor_y;
            dmv_ver_y = dmv_hor_x;
            *vertex_num = 2;
        }
        let hor_base = i32::from(neb_mv[0][0]) << 7;
        let ver_base = i32::from(neb_mv[0][1]) << 7;
        let derive = |dx: i32, dy: i32, out: &mut [i32; 2]| {
            let th = dmv_hor_x.wrapping_mul(dx).wrapping_add(dmv_ver_x.wrapping_mul(dy)).wrapping_add(hor_base);
            let tv = dmv_hor_y.wrapping_mul(dx).wrapping_add(dmv_ver_y.wrapping_mul(dy)).wrapping_add(ver_base);
            let (h, v) = mv_rounding_s32_shift7(th, tv);
            out[0] = clip3(CPMV_MIN, CPMV_MAX, h << 2);
            out[1] = clip3(CPMV_MIN, CPMV_MAX, v << 2);
        };
        derive(cur_x - neb_x, cur_y - neb_y, &mut mvp[0]);
        derive(cur_x - neb_x + cu_width, cur_y - neb_y, &mut mvp[1]);
        if *vertex_num == 3 {
            derive(cur_x - neb_x, cur_y - neb_y + cu_height, &mut mvp[2]);
        }
    }

    /// `derive_affine_constructed_candidate`
    #[allow(clippy::too_many_arguments)]
    fn affine_constructed(
        &self,
        cp_valid: &[bool; 4],
        cp_mv: &[[Mv; 4]; 2],
        cp_refi: &[[i32; 4]; 2],
        cp_idx: &[usize],
        model_idx: usize,
        ver_num: usize,
        mrg_cp_mv: &mut [[[i32; 2]; 4]; 2],
        mrg_refi: &mut [i8; 2],
        mrg_cp_num: &mut i32,
        cnt: &mut i32,
        mrg_idx: i32,
    ) -> bool {
        let mut valid_model = [false; 2];
        if cp_idx.iter().take(ver_num).any(|&i| !cp_valid[i]) {
            return false;
        }
        for lidx in 0..2 {
            let first = cp_refi[lidx][cp_idx[0]];
            if first >= 0 && cp_idx.iter().take(ver_num).all(|&i| cp_refi[lidx][i] == first) {
                valid_model[lidx] = true;
            }
        }
        if !valid_model[0] && !valid_model[1] {
            return false;
        }
        *mrg_cp_num = ver_num as i32;
        if *cnt != mrg_idx {
            *cnt += 1;
            return true;
        }
        let cu_width = self.cu_width;
        let cu_height = self.cu_height;
        for lidx in 0..2 {
            if valid_model[lidx] {
                let mut t = [[0i32; 2]; 4];
                for i in 0..ver_num {
                    let ci = cp_idx[i];
                    t[ci] = [i32::from(cp_mv[lidx][ci][0]), i32::from(cp_mv[lidx][ci][1])];
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
                        let shift_h_to_w = 7 + log2_u(cu_width) as i32 - log2_u(cu_height) as i32;
                        let tmp_hor = ((t[2][1] - t[0][1]) << shift_h_to_w) + (t[0][0] << 7);
                        let tmp_ver = -((t[2][0] - t[0][0]) << shift_h_to_w) + (t[0][1] << 7);
                        let (h, v) = mv_rounding_s32_shift7(tmp_hor, tmp_ver);
                        t[1] = [h, v];
                    }
                    _ => {}
                }
                mrg_refi[lidx] = cp_refi[lidx][cp_idx[0]] as i8;
                for i in 0..ver_num {
                    let x = t[i][0] << 2;
                    let y = t[i][1] << 2;
                    mrg_cp_mv[lidx][i] = [clip3(CPMV_MIN, CPMV_MAX, x), clip3(CPMV_MIN, CPMV_MAX, y)];
                }
            } else {
                mrg_refi[lidx] = -1;
            }
        }
        *cnt += 1;
        true
    }

    /// `get_affine_merge_candidate`
    fn affine_merge_candidate(&self, mrg_refi: &mut [i8; 2], mrg_cpmv: &mut [[[i32; 2]; 4]; 2], mrg_cp_num: &mut i32, mrg_idx: i32) -> i32 {
        let scup = self.cu_scup;
        let m = &self.maps;
        let seq = &self.seq;
        let i_scu = seq.i_scu as i32;
        let wi = (self.cu_width >> 2) as i32;
        let hi = (self.cu_height >> 2) as i32;
        let mut cnt = 0i32;
        {
            let max_candidate = (mrg_idx + 1).min(2);
            let nb = [scup + i_scu * (hi - 1) - 1, scup - i_scu + wi - 1, scup - i_scu + wi, scup - 1, scup - i_scu - 1];
            let mut valid = [false; 5];
            let mut top_left = [0i32; 5];
            for k in 0..5 {
                let ix = m.ix(nb[k]);
                valid[k] = m.scu[ix].affine != 0;
                if valid[k] {
                    top_left[k] = mcu_get_scup(m.pos[ix]);
                }
            }
            if valid[2] && valid[1] && top_left[1] == top_left[2] {
                valid[2] = false;
            }
            let valid3_bak = valid[3];
            if valid[3] && valid[0] && top_left[0] == top_left[3] {
                valid[3] = false;
            }
            if (valid[4] && valid3_bak && top_left[4] == top_left[3]) || (valid[4] && valid[1] && top_left[4] == top_left[1]) {
                valid[4] = false;
            }
            let mut k = 0;
            while k < 5 && cnt < max_candidate {
                if valid[k] {
                    if cnt == mrg_idx {
                        let ix = m.ix(nb[k]);
                        *mrg_cp_num = if m.scu[ix].affine == 1 { 2 } else { 3 };
                        for lidx in 0..2 {
                            mrg_refi[lidx] = m.refi[ix][lidx];
                            if mrg_refi[lidx] >= 0 {
                                let mut vn = *mrg_cp_num;
                                self.affine_model_mv(scup, nb[k], lidx, &mut vn, &mut mrg_cpmv[lidx]);
                                *mrg_cp_num = vn;
                            }
                        }
                    }
                    cnt += 1;
                }
                k += 1;
            }
        }
        if cnt <= mrg_idx {
            let mut cp_mv = [[[0i16; 2]; 4]; 2];
            let mut cp_refi = [[-1i32; 4]; 2];
            let mut cp_valid = [false; 4];
            let max_candidate = (mrg_idx + 1).min(AFF_MAX_NUM_MRG as i32);
            // LT
            for &n in &[scup - 1, scup - i_scu, scup - i_scu - 1] {
                let ix = m.ix(n);
                if m.scu[ix].inter {
                    for lidx in 0..2 {
                        cp_refi[lidx][0] = i32::from(m.refi[ix][lidx]);
                        cp_mv[lidx][0] = m.mv[ix][lidx];
                    }
                    cp_valid[0] = true;
                    break;
                }
            }
            // RT
            for &n in &[scup - i_scu + wi - 1, scup - i_scu + wi] {
                let ix = m.ix(n);
                if m.scu[ix].inter {
                    for lidx in 0..2 {
                        cp_refi[lidx][1] = i32::from(m.refi[ix][lidx]);
                        cp_mv[lidx][1] = m.mv[ix][lidx];
                    }
                    cp_valid[1] = true;
                    break;
                }
            }
            // LB
            let lb = m.ix(scup + i_scu * (hi - 1) - 1);
            if m.scu[lb].inter {
                for lidx in 0..2 {
                    cp_refi[lidx][2] = i32::from(m.refi[lb][lidx]);
                    cp_mv[lidx][2] = m.mv[lb][lidx];
                }
                cp_valid[2] = true;
            }
            // RB (temporal)
            let rb = scup + i_scu * (hi - 1) + (wi - 1);
            let (scup_co, _) = self.colocal_scup(rb);
            if self.pichdr.slice_type == SliceType::B {
                if let Some(p1) = self.rp(0, 1) {
                    let ix = p1.pic.col_ix(scup_co, seq.i_scu);
                    if p1.pic.map_refi.get(ix).is_some_and(|r| r[0] >= 0) {
                        let mv_tmp = self.col_mv(scup_co);
                        for lidx in 0..2 {
                            cp_refi[lidx][3] = 0;
                            cp_mv[lidx][3] = mv_tmp[lidx];
                        }
                        cp_valid[3] = true;
                    }
                }
            } else if let Some(p0) = self.rp(0, 0) {
                let ix = p0.pic.col_ix(scup_co, seq.i_scu);
                if p0.pic.map_refi.get(ix).is_some_and(|r| r[0] >= 0) {
                    let mut mv = [0i16; 2];
                    self.col_mv_list0(p0, scup_co, &mut mv);
                    cp_refi[0][3] = 0;
                    cp_mv[0][3] = mv;
                    cp_refi[1][3] = i32::from(REFI_INVALID);
                    cp_valid[3] = true;
                }
            }
            const CP_NUM: [usize; 6] = [3, 3, 3, 3, 2, 2];
            const CONST_MODEL: [[usize; 3]; 6] = [[0, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3], [0, 1, 0], [0, 2, 0]];
            let mut idx = 0;
            while idx < 6 && cnt < max_candidate {
                self.affine_constructed(&cp_valid, &cp_mv, &cp_refi, &CONST_MODEL[idx], idx, CP_NUM[idx], mrg_cpmv, mrg_refi, mrg_cp_num, &mut cnt, mrg_idx);
                idx += 1;
            }
        }
        if cnt <= mrg_idx {
            *mrg_cp_num = 2;
            mrg_refi[0] = 0;
            mrg_refi[1] = REFI_INVALID;
            *mrg_cpmv = [[[0; 2]; 4]; 2];
        }
        cnt
    }

    /// `dec_derive_skip_mv_affine`
    pub(crate) fn derive_skip_mv_affine(&mut self, mrg_idx: i32) {
        let mut refi = [0i8; 2];
        let mut cpmv = [[[0i32; 2]; 4]; 2];
        let mut cp_num = 0;
        self.affine_merge_candidate(&mut refi, &mut cpmv, &mut cp_num, mrg_idx);
        self.affine_flag = (cp_num - 1).clamp(0, 2) as u8;
        for lidx in 0..2 {
            if refi[lidx] >= 0 {
                self.refi[lidx] = refi[lidx];
                for cp in 0..cp_num.clamp(0, 4) as usize {
                    self.affine_mv[lidx][cp] = cpmv[lidx][cp];
                }
            } else {
                self.refi[lidx] = REFI_INVALID;
            }
        }
    }

    /// `dec_scale_affine_mvp`
    pub(crate) fn scale_affine_mvp(&self, lidx: usize, curr_mvr: i32) -> [[i32; 2]; 4] {
        let cur_refi = self.refi[lidx];
        let m = &self.maps;
        let scup = self.cu_scup;
        let i_scu = self.seq.i_scu as i32;
        let wi = (self.cu_width >> 2) as i32;
        let dist_cur = self.refp_dist(cur_refi, lidx);
        let try_one = |idx: i32| -> Option<Mv> {
            let ix = m.ix(idx);
            let refi = m.refi[ix][lidx];
            if m.scu[ix].inter && refi >= 0 { Some(scaling_mv(dist_cur, self.refp_dist(refi, lidx), m.mv[ix][lidx])) } else { None }
        };
        let mvp_lt = try_one(scup - 1).or_else(|| try_one(scup - i_scu)).or_else(|| try_one(scup - i_scu - 1)).unwrap_or([0, 0]);
        let mvp_rt = try_one(scup - i_scu + wi - 1).or_else(|| try_one(scup - i_scu + wi)).unwrap_or([0, 0]);
        let shift = amvr_shift_affine(curr_mvr);
        let add = if shift != 0 { 1 << (shift - 1) } else { 0 };
        let mut out = [[0i32; 2]; 4];
        out[0] = mv_rounding_s32(mvp_lt, shift, add);
        out[1] = mv_rounding_s32(mvp_rt, shift, add);
        out
    }

    /// `set_affine_mvf`: the motion field of an affine CU (4x4 or 8x8 sub blocks).
    pub(crate) fn set_affine_mvf(&mut self) {
        let scup = self.cu_scup;
        let i_scu = self.seq.i_scu as i32;
        let log2_cuw = self.cu_log2w as i32;
        let log2_cuh = self.cu_log2h as i32;
        let cu_w_in_scu = (1usize << log2_cuw) >> 2;
        let cu_h_in_scu = (1usize << log2_cuh) >> 2;
        let cp_num = self.affine_flag as i32 + 1;
        let base = self.maps.ix(scup);
        let both = self.refi[0] >= 0 && self.refi[1] >= 0;
        if both {
            struct L {
                scale_hor: i32,
                scale_ver: i32,
                dhx: i32,
                dhy: i32,
                dvx: i32,
                dvy: i32,
            }
            let mk = |ac: &[[i32; 2]; 4]| -> L {
                let dhx = ((ac[1][0] - ac[0][0]) << 7) >> log2_cuw;
                let dhy = ((ac[1][1] - ac[0][1]) << 7) >> log2_cuw;
                let (dvx, dvy) = if cp_num == 3 { (((ac[2][0] - ac[0][0]) << 7) >> log2_cuh, ((ac[2][1] - ac[0][1]) << 7) >> log2_cuh) } else { (-dhy, dhx) };
                L { scale_hor: ac[0][0] << 7, scale_ver: ac[0][1] << 7, dhx, dhy, dvx, dvy }
            };
            let l = [mk(&self.affine_mv[0]), mk(&self.affine_mv[1])];
            let mut asb = 0usize;
            let mut h = 0usize;
            while h < cu_h_in_scu {
                let mut w = 0usize;
                while w < cu_w_in_scu {
                    let mut mvs = [[0i16; 2]; 2];
                    let mut mv18 = [[0i32; 2]; 2];
                    for k in 0..2 {
                        let lk = &l[k];
                        let (th, tv) = if w == 0 && h == 0 {
                            (lk.scale_hor, lk.scale_ver)
                        } else if w + 2 == cu_w_in_scu && h == 0 {
                            (lk.scale_hor + (lk.dhx << log2_cuw), lk.scale_ver + (lk.dhy << log2_cuw))
                        } else if w == 0 && h + 2 == cu_h_in_scu && cp_num == 3 {
                            (lk.scale_hor + (lk.dvx << log2_cuh), lk.scale_ver + (lk.dvy << log2_cuh))
                        } else {
                            let px = ((w as i32) << 2) + 4;
                            let py = ((h as i32) << 2) + 4;
                            (lk.scale_hor + lk.dhx * px + lk.dvx * py, lk.scale_ver + lk.dhy * px + lk.dvy * py)
                        };
                        let (x18, x16) = mv_rounding_affine(th);
                        let (y18, y16) = mv_rounding_affine(tv);
                        mv18[k] = [x18, y18];
                        mvs[k] = [x16, y16];
                    }
                    for (dy, dx) in [(0i32, 0i32), (0, 1), (1, 0), (1, 1)] {
                        let ix = (base as i32 + (h as i32 + dy) * i_scu + w as i32 + dx) as usize;
                        self.maps.mv[ix] = mvs;
                    }
                    self.affine_sb_mv[0][asb] = mv18[0];
                    self.affine_sb_mv[1][asb] = mv18[1];
                    asb += 1;
                    w += 2;
                }
                h += 2;
            }
        } else {
            let lidx = if self.refi[0] >= 0 { 0 } else { 1 };
            let ac = self.affine_mv[lidx];
            let scale_hor = ac[0][0] << 7;
            let scale_ver = ac[0][1] << 7;
            let dhx = ((ac[1][0] - ac[0][0]) << 7) >> log2_cuw;
            let dhy = ((ac[1][1] - ac[0][1]) << 7) >> log2_cuw;
            let sub = if self.pichdr.affine_subblock_size_idx == 1 { 8 } else { 4 };
            let (dvx, dvy) = if cp_num == 3 { (((ac[2][0] - ac[0][0]) << 7) >> log2_cuh, ((ac[2][1] - ac[0][1]) << 7) >> log2_cuh) } else { (-dhy, dhx) };
            let mut asb = 0usize;
            let step = if sub == 4 { 1 } else { 2 };
            let half = if sub == 4 { 2 } else { 4 };
            let mut h = 0usize;
            while h < cu_h_in_scu {
                let mut w = 0usize;
                while w < cu_w_in_scu {
                    let (th, tv) = if w == 0 && h == 0 {
                        (scale_hor, scale_ver)
                    } else if w + step == cu_w_in_scu && h == 0 {
                        (scale_hor + (dhx << log2_cuw), scale_ver + (dhy << log2_cuw))
                    } else if w == 0 && h + step == cu_h_in_scu && cp_num == 3 {
                        (scale_hor + (dvx << log2_cuh), scale_ver + (dvy << log2_cuh))
                    } else {
                        let px = ((w as i32) << 2) + half;
                        let py = ((h as i32) << 2) + half;
                        (scale_hor + dhx * px + dvx * py, scale_ver + dhy * px + dvy * py)
                    };
                    let (x18, x16) = mv_rounding_affine(th);
                    let (y18, y16) = mv_rounding_affine(tv);
                    let blocks: &[(i32, i32)] = if sub == 4 { &[(0, 0)] } else { &[(0, 0), (0, 1), (1, 0), (1, 1)] };
                    for &(dy, dx) in blocks {
                        let ix = (base as i32 + (h as i32 + dy) * i_scu + w as i32 + dx) as usize;
                        self.maps.mv[ix][lidx] = [x16, y16];
                    }
                    if let Some(e) = self.affine_sb_mv[lidx].get_mut(asb) {
                        *e = [x18, y18];
                    }
                    asb += 1;
                    w += step;
                }
                h += step;
            }
        }
    }
}
