// Derived from opencore-amr 0.1.6 (commit 7dba8c32), amr_nb: dec_lag3.cpp, dec_lag6.cpp,
// pred_lt.cpp, d2_9pf.cpp, d2_11pf.cpp, d3_14pf.cpp, d4_17pf.cpp, d8_31pf.cpp, d1035pf.cpp,
// b_cn_cod.cpp.
// Copyright (C) 1998-2009 PacketVideo, Apache License 2.0; portions (C) 2004 3GPP
// Organizational Partners (3GPP TS 26.073). Ported to Rust for CFA4 (GPL-3.0-or-later).

//! Pitch lags, the adaptive-codebook interpolation and the algebraic (fixed) codebooks.

use super::L_SUBFR;
use super::tables::{DGRAY, STARTPOS};
use crate::basic::{add, at, l_add, l_mult, l_shr, mult, negate, shl, sub};

/// `Dec_lag3`: the 1/3-resolution lag of every mode but MR122. Returns `(T0, T0_frac)`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dec_lag3(index: i16, t0_min: i16, t0_max: i16, i_subfr: i32, t0_prev: i16, flag4: bool) -> (i16, i16) {
    if i_subfr == 0 {
        if index < 197 {
            let t = mult(index.wrapping_add(2), 10923).wrapping_add(19);
            let i = t.wrapping_mul(3);
            (t, index.wrapping_sub(i).wrapping_add(58))
        } else {
            (index.wrapping_sub(112), 0)
        }
    } else if !flag4 {
        let i = ((i32::from(index.wrapping_add(2)) * 10923) >> 15) as i16 - 1;
        let t0 = i.wrapping_add(t0_min);
        let i = i.wrapping_mul(3);
        (t0, index.wrapping_sub(2).wrapping_sub(i))
    } else {
        let mut tmp_lag = t0_prev;
        if sub(tmp_lag, t0_min) > 5 {
            tmp_lag = t0_min.wrapping_add(5);
        }
        if t0_max.wrapping_sub(tmp_lag) > 4 {
            tmp_lag = t0_max.wrapping_sub(4);
        }
        if index < 4 {
            (tmp_lag.wrapping_sub(5).wrapping_add(index), 0)
        } else if index < 12 {
            let i = ((i32::from(index.wrapping_sub(5)) * 10923) >> 15) as i16 - 1;
            let t0 = i.wrapping_add(tmp_lag);
            let i = i.wrapping_mul(3);
            (t0, index.wrapping_sub(9).wrapping_sub(i))
        } else {
            (index.wrapping_sub(12).wrapping_add(tmp_lag).wrapping_add(1), 0)
        }
    }
}

/// `Dec_lag6`: the 1/6-resolution lag of MR122. `t0` is the previous subframe's lag.
pub(crate) fn dec_lag6(index: i16, pit_min: i16, pit_max: i16, i_subfr: i32, t0: i16) -> (i16, i16) {
    if i_subfr == 0 {
        if index < 463 {
            let t = (((i32::from(index.wrapping_add(5)) * 5462) >> 15) as i16).wrapping_add(17);
            let i = t.wrapping_mul(6);
            (t, index.wrapping_sub(i).wrapping_add(105))
        } else {
            (index.wrapping_sub(368), 0)
        }
    } else {
        let mut t0_min = t0.wrapping_sub(5);
        if t0_min < pit_min {
            t0_min = pit_min;
        }
        let t0_max = t0_min.wrapping_add(9);
        if t0_max > pit_max {
            t0_min = pit_max.wrapping_sub(9);
        }
        let i = ((i32::from(index.wrapping_add(5)) * 5462) >> 15) as i16 - 1;
        let t = i.wrapping_add(t0_min);
        let i = i.wrapping_mul(6);
        (t, index.wrapping_sub(3).wrapping_sub(i))
    }
}

const INTER_6_PRED_LT: [i16; 61] = [
    29443, 28346, 25207, 20449, 14701, 8693, 3143, -1352, -4402, -5865, -5850, -4673, -2783, -672, 1211, 2536, 3130, 2991, 2259, 1170, 0, -1001, -1652, -1868,
    -1666, -1147, -464, 218, 756, 1060, 1099, 904, 550, 135, -245, -514, -634, -602, -451, -231, 0, 191, 308, 340, 296, 198, 78, -36, -120, -163, -165, -132,
    -79, -19, 34, 73, 91, 89, 70, 38, 0,
];

/// `Pred_lt_3or6`: the adaptive-codebook excitation `exc[base..base + L_SUBFR]` interpolated from
/// the past excitation at lag `t0 + frac`. Outputs are produced in pairs, as in opencore, so a
/// pair never reads its own first sample.
pub(crate) fn pred_lt_3or6(exc: &mut [i16], base: usize, t0: i16, frac: i16, flag3: bool) {
    let mut x0 = base as i32 - i32::from(t0);
    let mut frac = -i32::from(frac);
    if flag3 {
        frac <<= 1;
    }
    if frac < 0 {
        frac += 6;
        x0 -= 1;
    }
    let mut c = [0i32; 20];
    for q in 0..10 {
        c[2 * q] = i32::from(at(&INTER_6_PRED_LT, frac + 6 * q as i32));
        c[2 * q + 1] = i32::from(at(&INTER_6_PRED_LT, 6 - frac + 6 * q as i32));
    }
    let x = |exc: &[i16], i: i32| i32::from(at(exc, i));
    for m in 0..(L_SUBFR / 2) as i32 {
        let p = x0 + 1 + 2 * m;
        let mut s1 = 0x4000i32;
        let mut s2 = 0x4000i32;
        for q in 0..10i32 {
            let c1 = c[2 * q as usize];
            let c2 = c[2 * q as usize + 1];
            s2 = s2.wrapping_add(x(exc, p - q).wrapping_mul(c1));
            s1 = s1.wrapping_add(x(exc, p - q - 1).wrapping_mul(c1));
            s1 = s1.wrapping_add(x(exc, p + q).wrapping_mul(c2));
            s2 = s2.wrapping_add(x(exc, p + q + 1).wrapping_mul(c2));
        }
        let o = base + 2 * m as usize;
        if let Some(v) = exc.get_mut(o) {
            *v = (s1 >> 15) as i16;
        }
        if let Some(v) = exc.get_mut(o + 1) {
            *v = (s2 >> 15) as i16;
        }
    }
}

fn set_pulse(cod: &mut [i16; L_SUBFR], pos: i32, v: i16) {
    if let Some(c) = usize::try_from(pos).ok().and_then(|p| cod.get_mut(p)) {
        *c = v;
    }
}

fn place_signed(cod: &mut [i16; L_SUBFR], pos: &[i32], mut sign: i16) {
    *cod = [0; L_SUBFR];
    for &p in pos {
        let i = sign & 1;
        set_pulse(cod, p, i * 16383 - 8192);
        sign >>= 1;
    }
}

/// MR475 / MR515: two pulses, 9 bits.
pub(crate) fn decode_2i40_9bits(sub_nr: i16, sign: i16, index: i16, cod: &mut [i16; L_SUBFR]) {
    let j = i32::from((index & 64) >> 3);
    let k = i32::from(shl(sub_nr, 1)) + j;
    let p0 = i32::from(index & 7) * 5 + i32::from(at(&STARTPOS, k));
    let p1 = i32::from((index >> 3) & 7) * 5 + i32::from(at(&STARTPOS, k + 1));
    place_signed(cod, &[p0, p1], sign);
}

/// MR59: two pulses, 11 bits.
pub(crate) fn decode_2i40_11bits(sign: i16, index: i16, cod: &mut [i16; L_SUBFR]) {
    let index = i32::from(index);
    let j = index & 1;
    let p0 = ((index >> 1) & 7) * 5 + j * 2 + 1;
    let j = (index >> 4) & 3;
    let i = (index >> 6) & 7;
    let p1 = if j == 3 { i * 5 + 4 } else { i * 5 + j };
    place_signed(cod, &[p0, p1], sign);
}

/// MR67: three pulses, 14 bits.
pub(crate) fn decode_3i40_14bits(sign: i16, index: i16, cod: &mut [i16; L_SUBFR]) {
    let index = i32::from(index);
    let p0 = (index & 7) * 5;
    let p1 = ((index >> 4) & 7) * 5 + ((index >> 3) & 1) * 2 + 1;
    let p2 = ((index >> 8) & 7) * 5 + ((index >> 7) & 1) * 2 + 2;
    place_signed(cod, &[p0, p1, p2], sign);
}

/// MR74 / MR795: four pulses, 17 bits.
pub(crate) fn decode_4i40_17bits(sign: i16, index: i16, cod: &mut [i16; L_SUBFR]) {
    let index = i32::from(index);
    let g = |v: i32| i32::from(at(&DGRAY, v & 7));
    let p0 = g(index) * 5;
    let p1 = g(index >> 3) * 5 + 1;
    let p2 = g(index >> 6) * 5 + 2;
    let p3 = g(index >> 10) * 5 + 3 + ((index >> 9) & 1);
    place_signed(cod, &[p0, p1, p2, p3], sign);
}

fn decompress10(msbs: i16, lsbs: i16, i1: usize, i2: usize, i3: usize, pos: &mut [i16; 8]) {
    let msbs = msbs.min(124);
    let ia0 = mult(msbs, 1311);
    let ia = msbs.wrapping_sub((l_mult(ia0, 25) >> 1) as i16);
    let ib = mult(ia, 6554);
    let ib = ia.wrapping_sub((l_mult(ib, 5) >> 1) as i16);
    let ib = shl(ib, 1);
    let ic = lsbs.wrapping_sub((lsbs >> 2) << 2);
    pos[i1] = ib.wrapping_add(ic & 1);
    let ib = shl(mult(ia, 6554), 1);
    pos[i2] = ib.wrapping_add(ic >> 1);
    let ib = lsbs >> 2;
    let ic = shl(mult(msbs, 1311), 1);
    pos[i3] = add(ib, ic);
}

/// MR102: eight pulses, 31 bits (`dec_8i40_31bits`). `index` holds the 7 codebook parameters.
pub(crate) fn dec_8i40_31bits(index: &[i16], cod: &mut [i16; L_SUBFR]) {
    let ix = |i: usize| index.get(i).copied().unwrap_or(0);
    *cod = [0; L_SUBFR];
    let mut pos = [0i16; 8];
    decompress10(ix(4) >> 3, ix(4) & 7, 0, 4, 1, &mut pos);
    decompress10(ix(5) >> 3, ix(5) & 7, 2, 6, 5, &mut pos);
    let msbs = ix(6) >> 2;
    let lsbs = ix(6) & 3;
    let mut ia = (l_shr(l_mult(msbs, 25), 1) as i16).wrapping_add(12);
    let msbs0_24 = ia >> 5;
    ia = mult(msbs0_24, 6554) & 1;
    let ib = mult(msbs0_24, 6554);
    let mut ib = msbs0_24.wrapping_sub((l_mult(ib, 5) >> 1) as i16);
    if ia == 1 {
        ib = 4i16.wrapping_sub(ib);
    }
    let ib = shl(ib, 1);
    pos[3] = add(ib, lsbs & 1);
    let ia = shl(mult(msbs0_24, 6554), 1);
    pos[7] = ia.wrapping_add(lsbs >> 1);
    for j in 0..4usize {
        let p1 = i32::from((i32::from(pos[j]) * 4 + j as i32) as i16);
        let mut sign: i16 = if ix(j) == 0 { 8191 } else { -8191 };
        if (0..L_SUBFR as i32).contains(&p1) {
            set_pulse(cod, p1, sign);
        }
        let p2 = i32::from((i32::from(pos[j + 4]) * 4 + j as i32) as i16);
        if p2 < p1 {
            sign = negate(sign);
        }
        if let Some(c) = usize::try_from(p2).ok().filter(|&p| p < L_SUBFR).and_then(|p| cod.get_mut(p)) {
            *c = c.wrapping_add(sign);
        }
    }
}

/// MR122: ten pulses, 35 bits (`dec_10i40_35bits`). `index` holds the 10 codebook parameters.
pub(crate) fn dec_10i40_35bits(index: &[i16], cod: &mut [i16; L_SUBFR]) {
    let ix = |i: usize| i32::from(index.get(i).copied().unwrap_or(0));
    *cod = [0; L_SUBFR];
    for j in 0..5i32 {
        let tmp = ix(j as usize);
        let p1 = i32::from(at(&DGRAY, tmp & 7)) * 5 + j;
        let mut sign: i16 = if (tmp >> 3) & 1 == 0 { 4096 } else { -4096 };
        set_pulse(cod, p1, sign);
        let p2 = i32::from(at(&DGRAY, ix(j as usize + 5) & 7)) * 5 + j;
        if p2 < p1 {
            sign = negate(sign);
        }
        if let Some(c) = usize::try_from(p2).ok().and_then(|p| cod.get_mut(p)) {
            *c = c.wrapping_add(sign);
        }
    }
}

/// The comfort-noise pseudo-random generator.
pub(crate) fn pseudonoise(seed: &mut i32, bits: i16) -> i16 {
    let mut out: i16 = 0;
    for _ in 0..bits {
        let mut sn = *seed & 1;
        if *seed & 0x1000_0000 != 0 {
            sn ^= 1;
        }
        out = (out << 1) | (*seed & 1) as i16;
        *seed >>= 1;
        if sn & 1 != 0 {
            *seed |= 0x4000_0000;
        }
    }
    out
}

pub(crate) fn build_cn_code(seed: &mut i32, cod: &mut [i16; L_SUBFR]) {
    *cod = [0; L_SUBFR];
    for k in 0..10i16 {
        let i = pseudonoise(seed, 2);
        let temp = l_mult(i, 10) as i16;
        let i = add(temp >> 1, k);
        let j = pseudonoise(seed, 1);
        set_pulse(cod, i32::from(i), if j > 0 { 4096 } else { -4096 });
    }
}

/// `build_CN_param`: random parameters for a frame lost outside DTX.
pub(crate) fn build_cn_param(seed: &mut i16, sizes: &[i16], window: &[i16], parm: &mut [i16]) {
    let t = l_mult(*seed, 31821) >> 1;
    *seed = l_add(t, 13849) as i16;
    let start = i32::from(*seed & 0x7f);
    for (i, (p, &bits)) in parm.iter_mut().zip(sizes).enumerate() {
        let mask = !(0xffffi32.wrapping_shl(bits as u32 & 31)) as i16;
        *p = at(window, start + i as i32) & mask;
    }
}
