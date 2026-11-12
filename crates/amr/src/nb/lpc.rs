// Derived from opencore-amr 0.1.6 (commit 7dba8c32), amr_nb/common/src: lsp_lsf.cpp,
// lsp_az.cpp, int_lpc.cpp, int_lsf.cpp, reorder.cpp, syn_filt.cpp, residu.cpp, weight_a.cpp,
// a_refl.cpp, lsp_avg.cpp.
// Copyright (C) 1998-2009 PacketVideo, Apache License 2.0; portions (C) 2004 3GPP
// Organizational Partners (3GPP TS 26.073). Ported to Rust for CFA4 (GPL-3.0-or-later).

//! Linear-prediction helpers: LSF/LSP/LPC conversions, interpolation and filters.

use super::tables::{SLOPE, TABLE};
use super::{M, MP1};
use crate::basic::{MAX_16, MAX_32, MIN_16, abs_s, add, at, div_s, l_abs, l_mac, l_msu, l_mult, l_shl, l_shr_r, l_sub, norm_l, round, shl};

pub(crate) fn lsf_lsp(lsf: &[i16; M], lsp: &mut [i16; M]) {
    for (q, &f) in lsp.iter_mut().zip(lsf) {
        let ind = i32::from(f >> 8);
        let offset = i32::from(f & 0xff);
        let t0 = at(&TABLE, ind);
        let t1 = at(&TABLE, ind + 1);
        let tmp = ((i32::from(t1) - i32::from(t0)) * offset) >> 8;
        *q = t0.wrapping_add(tmp as i16);
    }
}

pub(crate) fn lsp_lsf(lsp: &[i16; M], lsf: &mut [i16; M]) {
    let mut ind: i32 = 63;
    for i in (0..M).rev() {
        let temp = lsp[i];
        while ind >= 0 && at(&TABLE, ind) < temp {
            ind -= 1;
        }
        let t = (i32::from(temp) - i32::from(at(&TABLE, ind))) * i32::from(at(&SLOPE, ind));
        let t = t.wrapping_add(0x800) >> 12;
        lsf[i] = (t as i16).wrapping_add((ind << 8) as i16);
    }
}

pub(crate) fn reorder_lsf(lsf: &mut [i16; M], min_dist: i16) {
    let mut lsf_min = min_dist;
    for v in lsf.iter_mut() {
        if *v < lsf_min {
            *v = lsf_min;
            lsf_min = lsf_min.wrapping_add(min_dist);
        } else {
            lsf_min = v.wrapping_add(min_dist);
        }
    }
}

/// `Get_lsp_pol`: the polynomial of every other LSP, starting at `lsp[start]`.
fn get_lsp_pol(lsp: &[i16; M], start: usize) -> [i32; 6] {
    let l = |k: usize| i32::from(lsp.get(start + 2 * k).copied().unwrap_or(0));
    let mut f = [0i32; 6];
    f[0] = 0x0100_0000;
    f[1] = (-l(0)).wrapping_shl(10);
    for i in 2..=5usize {
        let lv = l(i - 1);
        f[i] = f[i - 2];
        let mut p = i;
        for _ in 1..i {
            let (hi, lo) = ((f[p - 1] >> 16) as i16, ((f[p - 1] >> 1).wrapping_sub(i32::from((f[p - 1] >> 16) as i16) << 15)) as i16);
            let mut t0 = i32::from(hi).wrapping_mul(lv);
            t0 = t0.wrapping_add(i32::from(lo).wrapping_mul(lv) >> 15);
            f[p] = f[p].wrapping_add(f[p - 2]);
            f[p] = f[p].wrapping_sub(t0.wrapping_shl(2));
            p -= 1;
        }
        f[p] = f[p].wrapping_sub(lv.wrapping_shl(10));
    }
    f
}

pub(crate) fn lsp_az(lsp: &[i16; M], a: &mut [i16]) {
    let mut f1 = get_lsp_pol(lsp, 0);
    let mut f2 = get_lsp_pol(lsp, 1);
    for i in (1..=5).rev() {
        f1[i] = f1[i].wrapping_add(f1[i - 1]);
        f2[i] = f2[i].wrapping_sub(f2[i - 1]);
    }
    let mut out = [0i16; MP1];
    out[0] = 4096;
    for i in 1..=5 {
        let t0 = f1[i].wrapping_add(f2[i]).wrapping_add(1 << 12);
        let t1 = f1[i].wrapping_sub(f2[i]).wrapping_add(1 << 12);
        out[i] = (t0 >> 13) as i16;
        out[11 - i] = (t1 >> 13) as i16;
    }
    for (d, s) in a.iter_mut().zip(out) {
        *d = s;
    }
}

/// `Int_lpc_1to3`: four subframe filters from the old and new LSPs.
pub(crate) fn int_lpc_1to3(old: &[i16; M], new: &[i16; M], az: &mut [i16; 4 * MP1]) {
    let mut lsp = [0i16; M];
    for i in 0..M {
        lsp[i] = (old[i] - (old[i] >> 2)).wrapping_add(new[i] >> 2);
    }
    lsp_az(&lsp, &mut az[0..MP1]);
    for i in 0..M {
        lsp[i] = (new[i] >> 1).wrapping_add(old[i] >> 1);
    }
    lsp_az(&lsp, &mut az[MP1..2 * MP1]);
    for i in 0..M {
        lsp[i] = (new[i] - (new[i] >> 2)).wrapping_add(old[i] >> 2);
    }
    lsp_az(&lsp, &mut az[2 * MP1..3 * MP1]);
    lsp_az(new, &mut az[3 * MP1..4 * MP1]);
}

/// `Int_lpc_1and3`: MR122 with a transmitted mid-frame LSP set.
pub(crate) fn int_lpc_1and3(old: &[i16; M], mid: &[i16; M], new: &[i16; M], az: &mut [i16; 4 * MP1]) {
    let mut lsp = [0i16; M];
    for i in 0..M {
        lsp[i] = (old[i] >> 1).wrapping_add(mid[i] >> 1);
    }
    lsp_az(&lsp, &mut az[0..MP1]);
    lsp_az(mid, &mut az[MP1..2 * MP1]);
    for i in 0..M {
        lsp[i] = (mid[i] >> 1).wrapping_add(new[i] >> 1);
    }
    lsp_az(&lsp, &mut az[2 * MP1..3 * MP1]);
    lsp_az(new, &mut az[3 * MP1..4 * MP1]);
}

pub(crate) fn int_lsf(old: &[i16; M], new: &[i16; M], i_subfr: usize, out: &mut [i16; M]) {
    for i in 0..M {
        out[i] = match i_subfr {
            0 => add(old[i].wrapping_sub(old[i] >> 2), new[i] >> 2),
            40 => (old[i] >> 1).wrapping_add(new[i] >> 1),
            80 => add(old[i] >> 2, new[i].wrapping_sub(new[i] >> 2)),
            _ => new[i],
        };
    }
}

/// Saturates a `Syn_filt` accumulator. opencore's range test sends exactly `0x07ff_ffff` to
/// `MIN_16`, which is kept.
#[inline]
fn syn_sat(s: i32) -> i16 {
    if (s.wrapping_add(134_217_728) as u32) < 0x0fff_ffff {
        (s >> 12) as i16
    } else if s > 0x07ff_ffff {
        MAX_16
    } else {
        MIN_16
    }
}

/// `Syn_filt`: 1/A(z) over `x` into `y`, starting from the filter memory `mem`; with `update`
/// the memory takes the last outputs. `x` and `y` may hold the same samples (pass a copy).
pub(crate) fn syn_filt(a: &[i16], x: &[i16], y: &mut [i16], mem: &mut [i16; M], update: bool) {
    let lg = x.len().min(y.len());
    let mut hist = [0i16; M + 64];
    hist[..M].copy_from_slice(mem);
    let a = |k: usize| i32::from(a.get(k).copied().unwrap_or(0));
    for n in 0..lg.min(64) {
        let mut s = 0x800i32.wrapping_add(i32::from(x[n]) * a(0));
        for k in 1..=M {
            s = s.wrapping_sub(a(k) * i32::from(hist[M + n - k]));
        }
        let v = syn_sat(s);
        hist[M + n] = v;
        y[n] = v;
    }
    if update && lg >= M {
        mem.copy_from_slice(&y[lg - M..lg]);
    }
}

/// `Residu`: A(z) applied to `x[M..]`, with `x[..M]` the history.
pub(crate) fn residu(a: &[i16; MP1], x: &[i16], y: &mut [i16]) {
    for (n, out) in y.iter_mut().enumerate() {
        let mut s = 0x800i32;
        for k in 0..=M {
            let xv = x.get(M + n - k).copied().unwrap_or(0);
            s = s.wrapping_add(i32::from(a[k]) * i32::from(xv));
        }
        *out = (s >> 12) as i16;
    }
}

pub(crate) fn weight_ai(a: &[i16], fac: &[i16; M], out: &mut [i16; MP1]) {
    out[0] = a.first().copied().unwrap_or(0);
    for i in 1..=M {
        let av = i32::from(a.get(i).copied().unwrap_or(0));
        out[i] = ((av * i32::from(fac[i - 1]) + 0x4000) >> 15) as i16;
    }
}

/// `A_Refl`: reflection coefficients from `a[1..=M]`.
///
/// When an intermediate value overflows, opencore zeroes the coefficients but only leaves the
/// inner loop and carries on with a partly uninitialised scratch array; here that array starts
/// at zero.
pub(crate) fn a_refl(a: &[i16], refl: &mut [i16; M]) {
    let mut a_state = [0i16; M];
    for (i, s) in a_state.iter_mut().enumerate() {
        *s = a.get(i).copied().unwrap_or(0);
    }
    let mut b_state = [0i16; M];
    for i in (0..M).rev() {
        if abs_s(a_state[i]) >= 4096 {
            *refl = [0; M];
            break;
        }
        refl[i] = shl(a_state[i], 3);
        let t = l_mult(refl[i], refl[i]);
        let mut acc = l_sub(MAX_32, t);
        let norm = norm_l(acc);
        let scale = 15 - norm;
        acc = l_shl(acc, norm);
        let norm_prod = round(acc);
        let m = div_s(16384, norm_prod);
        for j in 0..i {
            let mut acc = i32::from(a_state[j]) << 16;
            acc = l_msu(acc, refl[i], a_state[i - j - 1]);
            let temp = round(acc);
            let mut t = l_mult(m, temp);
            t = l_shr_r(t, scale);
            if l_abs(t) > 32767 {
                *refl = [0; M];
                break;
            }
            b_state[j] = t as i16;
        }
        a_state[..i].copy_from_slice(&b_state[..i]);
    }
}

pub(crate) const EXPCONST: i16 = 5243;

/// `lsp_avg`: the running mean of the LSF vector (despite the name).
pub(crate) fn lsp_avg(mean: &mut [i16; M], lsf: &[i16; M]) {
    for i in 0..M {
        let mut t = i32::from(mean[i]) << 16;
        t = l_msu(t, EXPCONST, mean[i]);
        t = l_mac(t, EXPCONST, lsf[i]);
        mean[i] = round(t);
    }
}
