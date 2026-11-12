// Derived from opencore-amr 0.1.6 (commit 7dba8c32), amr_wb/dec/src: dec_acelp_2p_in_64.cpp,
// dec_acelp_4p_in_64.cpp, dec_alg_codebook.cpp, dec_gain2_amr_wb.cpp, qpisf_2s.cpp, qisf_ns.cpp,
// median5.cpp, lagconceal.cpp.
// Copyright (C) 1998-2009 PacketVideo, Apache License 2.0; portions (C) 2007 3GPP
// Organizational Partners (3GPP TS 26.173). Ported to Rust for CFA4 (GPL-3.0-or-later).

//! AMR-WB parameter decoding: algebraic codebook pulses, gains, ISF dequantisation and lag
//! concealment.

use super::op::{add, at, dot_product12, l_mac, l_mult, l_shl, log2, mul_32by16, mult, noise, one_ov_sqrt_norm, power_of_2, round, shl, shr_int, sub, to_dpf};
use super::tables::{
    DICO1_ISF, DICO1_ISF_NOISE, DICO2_ISF, DICO2_ISF_NOISE, DICO3_ISF_NOISE, DICO4_ISF_NOISE, DICO5_ISF_NOISE, DICO21_ISF, DICO21_ISF_36B, DICO22_ISF,
    DICO22_ISF_36B, DICO23_ISF, DICO23_ISF_36B, DICO24_ISF, DICO25_ISF, MEAN_ISF, MEAN_ISF_NOISE, T_QUA_GAIN6B, T_QUA_GAIN7B,
};
use super::{L_SUBFR, M};

const NB_POS: i16 = 16;

/// `dec_acelp_2p_in_64` (6.60 kbit/s).
pub(crate) fn dec_acelp_2p(index: i16, code: &mut [i16; L_SUBFR]) {
    *code = [0; L_SUBFR];
    let i = ((index >> 5) & 0x3e) as usize;
    if let Some(c) = code.get_mut(i) {
        *c = if ((index >> 6) & 32) == 0 { 512 } else { -512 };
    }
    let i = (((index & 0x1f) << 1) + 1) as usize;
    if let Some(c) = code.get_mut(i) {
        *c = if (index & 32) == 0 { 512 } else { -512 };
    }
}

fn dec_1p_n1(index: i32, n: i16, offset: i16, pos: &mut [i16]) {
    let mask = (1i32 << n) - 1;
    let mut p1 = ((index & mask) + i32::from(offset)) as i16;
    if ((index >> n) & 1) == 1 {
        p1 = p1.wrapping_add(NB_POS);
    }
    if let Some(p) = pos.get_mut(0) {
        *p = p1;
    }
}

fn dec_2p_2n1(index: i32, n: i16, offset: i16, pos: &mut [i16]) {
    let mask = i32::from(sub(shl(1, n), 1));
    let mut p1 = ((shr_int(index, i32::from(n)) & mask) + i32::from(offset)) as i16;
    let i = (index >> shl(n, 1)) & 1;
    let mut p2 = add((index & mask) as i16, offset);
    if p2 < p1 {
        if i == 1 {
            p1 = p1.wrapping_add(NB_POS);
        } else {
            p2 = p2.wrapping_add(NB_POS);
        }
    } else if i == 1 {
        p1 = p1.wrapping_add(NB_POS);
        p2 = p2.wrapping_add(NB_POS);
    }
    if let [a, b, ..] = pos {
        *a = p1;
        *b = p2;
    }
}

fn dec_3p_3n1(index: i32, n: i16, offset: i16, pos: &mut [i16]) {
    let mask = (1i32 << (2 * n - 1)) - 1;
    let idx = index & mask;
    let mut j = offset;
    if ((index >> ((n << 1) - 1)) & 1) != 0 {
        j = j.wrapping_add(1 << (n - 1));
    }
    dec_2p_2n1(idx, n - 1, j, pos);
    let mask = (1i32 << (n + 1)) - 1;
    let idx = (index >> (n << 1)) & mask;
    dec_1p_n1(idx, n, offset, pos.get_mut(2..).unwrap_or(&mut []));
}

fn dec_4p_4n1(index: i32, n: i16, offset: i16, pos: &mut [i16]) {
    let tmp = (n << 1) - 1;
    let mask = (1i32 << tmp) - 1;
    let idx = index & mask;
    let mut j = offset;
    if ((index >> tmp) & 1) != 0 {
        j = j.wrapping_add(1 << (n - 1));
    }
    dec_2p_2n1(idx, n - 1, j, pos);
    let tmp = (n << 1) + 1;
    let mask = (1i32 << tmp) - 1;
    let idx = (index >> (n << 1)) & mask;
    dec_2p_2n1(idx, n, offset, pos.get_mut(2..).unwrap_or(&mut []));
}

fn dec_4p_4n(index: i32, n: i16, offset: i16, pos: &mut [i16]) {
    let n1 = n - 1;
    let j = offset.wrapping_add(1 << n1);
    match (index >> ((n << 2) - 2)) & 3 {
        0 => {
            if ((index >> ((n1 << 2) + 1)) & 1) != 0 {
                dec_4p_4n1(index, n1, j, pos);
            } else {
                dec_4p_4n1(index, n1, offset, pos);
            }
        }
        1 => {
            dec_1p_n1(index >> (3 * n1 + 1), n1, offset, pos);
            dec_3p_3n1(index, n1, j, pos.get_mut(1..).unwrap_or(&mut []));
        }
        2 => {
            dec_2p_2n1(index >> ((n1 << 1) + 1), n1, offset, pos);
            dec_2p_2n1(index, n1, j, pos.get_mut(2..).unwrap_or(&mut []));
        }
        _ => {
            dec_3p_3n1(index >> (n1 + 1), n1, offset, pos);
            dec_1p_n1(index, n1, j, pos.get_mut(3..).unwrap_or(&mut []));
        }
    }
}

fn dec_5p_5n(index: i32, n: i16, offset: i16, pos: &mut [i16]) {
    let n1 = n - 1;
    let j = add(offset, shl(1, n1));
    let idx = index >> ((n << 1) + 1);
    let off = if ((index >> (5 * n - 1)) & 1) != 0 { j } else { offset };
    dec_3p_3n1(idx, n1, off, pos);
    dec_2p_2n1(index, n, offset, pos.get_mut(3..).unwrap_or(&mut []));
}

fn dec_6p_6n_2(index: i32, n: i16, offset: i16, pos: &mut [i16]) {
    let n1 = n - 1;
    let j = offset.wrapping_add(1 << n1);
    let (mut oa, mut ob) = (j, j);
    if ((index >> (6 * n - 5)) & 1) == 0 {
        oa = offset;
    } else {
        ob = offset;
    }
    match (index >> (6 * n - 4)) & 3 {
        0 => {
            dec_5p_5n(index >> n, n1, oa, pos);
            dec_1p_n1(index, n1, oa, pos.get_mut(5..).unwrap_or(&mut []));
        }
        1 => {
            dec_5p_5n(index >> n, n1, oa, pos);
            dec_1p_n1(index, n1, ob, pos.get_mut(5..).unwrap_or(&mut []));
        }
        2 => {
            dec_4p_4n(index >> (2 * n1 + 1), n1, oa, pos);
            dec_2p_2n1(index, n1, ob, pos.get_mut(4..).unwrap_or(&mut []));
        }
        _ => {
            dec_3p_3n1(index >> (3 * n1 + 1), n1, offset, pos);
            dec_3p_3n1(index, n1, j, pos.get_mut(3..).unwrap_or(&mut []));
        }
    }
}

fn add_pulses(pos: &[i16], track: usize, code: &mut [i16; L_SUBFR]) {
    for &p in pos {
        let i = (((p & (NB_POS - 1)) as usize) << 2) + track;
        if let Some(c) = code.get_mut(i) {
            *c = if (p & NB_POS) == 0 { c.wrapping_add(512) } else { c.wrapping_sub(512) };
        }
    }
}

/// `dec_acelp_4p_in_64` for `nbbits` 20, 36, 44, 52, 64, 72 or 88.
pub(crate) fn dec_acelp_4p(index: &[i16; 8], nbbits: i16, code: &mut [i16; L_SUBFR]) {
    *code = [0; L_SUBFR];
    let mut pos = [0i16; 6];
    let long = |k: usize, sh: i32| (i32::from(index[k]) << sh) + i32::from(index[k + 4]);
    match nbbits {
        20 => {
            for k in 0..4 {
                dec_1p_n1(i32::from(index[k]), 4, 0, &mut pos);
                add_pulses(&pos[..1], k, code);
            }
        }
        36 => {
            for k in 0..4 {
                dec_2p_2n1(i32::from(index[k]), 4, 0, &mut pos);
                add_pulses(&pos[..2], k, code);
            }
        }
        44 => {
            for k in 0..2 {
                dec_3p_3n1(i32::from(index[k]), 4, 0, &mut pos);
                add_pulses(&pos[..3], k, code);
            }
            for k in 2..4 {
                dec_2p_2n1(i32::from(index[k]), 4, 0, &mut pos);
                add_pulses(&pos[..2], k, code);
            }
        }
        52 => {
            for k in 0..4 {
                dec_3p_3n1(i32::from(index[k]), 4, 0, &mut pos);
                add_pulses(&pos[..3], k, code);
            }
        }
        64 => {
            for k in 0..4 {
                dec_4p_4n(long(k, 14), 4, 0, &mut pos);
                add_pulses(&pos[..4], k, code);
            }
        }
        72 => {
            for k in 0..2 {
                dec_5p_5n(long(k, 10), 4, 0, &mut pos);
                add_pulses(&pos[..5], k, code);
            }
            for k in 2..4 {
                dec_4p_4n(long(k, 14), 4, 0, &mut pos);
                add_pulses(&pos[..4], k, code);
            }
        }
        88 => {
            for k in 0..4 {
                dec_6p_6n_2(long(k, 11), 4, 0, &mut pos);
                add_pulses(&pos[..6], k, code);
            }
        }
        _ => {}
    }
}

/// `median5` of `x[0..5]`.
pub(crate) fn median5(x: &[i16]) -> i16 {
    let g = |i: usize| x.get(i).copied().unwrap_or(0);
    let (mut x1, mut x2, mut x3, mut x4, mut x5) = (g(0), g(1), g(2), g(3), g(4));
    if x2 < x1 {
        std::mem::swap(&mut x1, &mut x2);
    }
    if x3 < x1 {
        std::mem::swap(&mut x1, &mut x3);
    }
    if x4 < x1 {
        std::mem::swap(&mut x1, &mut x4);
    }
    if x5 < x1 {
        x5 = x1;
    }
    if x3 < x2 {
        std::mem::swap(&mut x2, &mut x3);
    }
    if x4 < x2 {
        std::mem::swap(&mut x2, &mut x4);
    }
    if x5 < x2 {
        x5 = x2;
    }
    if x4 < x3 {
        x3 = x4;
    }
    if x5 < x3 {
        x3 = x5;
    }
    x3
}

/// The gain decoder memory (`dec_gain` in opencore): past quantised energies, the last pitch
/// and code gains and the five-deep gain histories.
pub(crate) type GainMem = [i16; 23];

pub(crate) fn gain_mem_init() -> GainMem {
    let mut m = [0i16; 23];
    m[..4].copy_from_slice(&[-14336; 4]);
    m[22] = 21845;
    m
}

const PDOWN_UNUSABLE: [i16; 7] = [32767, 31130, 29491, 24576, 7537, 1638, 328];
const CDOWN_UNUSABLE: [i16; 7] = [32767, 16384, 8192, 8192, 8192, 4915, 3277];
const PDOWN_USABLE: [i16; 7] = [32767, 32113, 31457, 24576, 7537, 1638, 328];
const CDOWN_USABLE: [i16; 7] = [32767, 32113, 32113, 32113, 32113, 32113, 22938];
const PRED: [i16; 4] = [4096, 3277, 2458, 1638];

pub(crate) struct GainIn {
    pub(crate) index: i16,
    pub(crate) nbits: i16,
    pub(crate) bfi: bool,
    pub(crate) prev_bfi: bool,
    pub(crate) state: i16,
    pub(crate) unusable: bool,
    pub(crate) vad_hist: i16,
}

/// `dec_gain2_amr_wb`: returns (pitch gain, code gain).
pub(crate) fn dec_gain2(g: &GainIn, code: &[i16; L_SUBFR], mem: &mut GainMem) -> (i16, i32) {
    const PAST_GAIN_PIT: usize = 4;
    const PAST_GAIN_CODE: usize = 5;
    const PREV_GC: usize = 6;
    const PBUF: usize = 7;
    const GBUF: usize = 12;
    const PBUF2: usize = 17;
    let mut exp = 0;
    let mut l = dot_product12(code, code, &mut exp);
    exp -= 24;
    one_ov_sqrt_norm(&mut l, &mut exp);
    let gcode_inov = (l_shl(l, exp - 3) >> 16) as i16;
    let state = usize::try_from(g.state).unwrap_or(0).min(6);
    if g.bfi {
        let mut pgp = median5(&mem[PBUF..PBUF + 5]);
        if pgp > 15565 {
            pgp = 15565;
        }
        mem[PAST_GAIN_PIT] = pgp;
        let gain_pit = mult(if g.unusable { PDOWN_UNUSABLE[state] } else { PDOWN_USABLE[state] }, pgp);
        let tmp = median5(&mem[GBUF..GBUF + 5]);
        mem[PAST_GAIN_CODE] = if g.vad_hist > 2 { tmp } else { mult(if g.unusable { CDOWN_UNUSABLE[state] } else { CDOWN_USABLE[state] }, tmp) };
        let q = &mut mem[0..4];
        let sum = i32::from(q[3]) + i32::from(q[2]) + i32::from(q[1]) + i32::from(q[0]);
        // opencore keeps past_qua_en[3] here (the reference shifts it).
        q[2] = q[1];
        q[1] = q[0];
        let mut qe = ((sum >> 3) as i16).wrapping_sub(3072);
        if qe < -14336 {
            qe = -14336;
        }
        q[0] = qe;
        mem.copy_within(GBUF + 1..GBUF + 5, GBUF);
        mem.copy_within(PBUF + 1..PBUF + 5, PBUF);
        mem[GBUF + 4] = mem[PAST_GAIN_CODE];
        mem[PBUF + 4] = mem[PAST_GAIN_PIT];
        return (gain_pit, l_mult(mem[PAST_GAIN_CODE], gcode_inov));
    }
    let mut l = l_shl(30 << 16, 8);
    for k in 0..4 {
        l = l_mac(l, PRED[k], mem[k]);
    }
    let gcode0 = (l >> 16) as i16;
    let l = (i32::from(gcode0) * 5443) >> 7;
    let (mut exp_gcode0, frac) = to_dpf(l);
    let gcode0 = power_of_2(14, frac) as i16;
    exp_gcode0 = exp_gcode0.wrapping_sub(14);
    let k = i32::from(g.index) << 1;
    let table: &[i16] = if g.nbits == 6 { &T_QUA_GAIN6B } else { &T_QUA_GAIN7B };
    let gain_pit = at(table, k);
    let g_code = at(table, k + 1);
    let mut gain_cod = l_shl(l_mult(g_code, gcode0), exp_gcode0.wrapping_add(4));
    if g.prev_bfi {
        let lim = l_mult(mem[PREV_GC], 5120);
        if gain_cod > lim && gain_cod > 6_553_600 {
            gain_cod = lim;
        }
    }
    mem[PAST_GAIN_CODE] = round(l_shl(gain_cod, 3));
    mem[PAST_GAIN_PIT] = gain_pit;
    mem[PREV_GC] = mem[PAST_GAIN_CODE];
    // opencore's "shift" (a lagging temporary) only copies element 1 to 0; 1..3 keep their
    // values. Concealment depends on it, so it is kept.
    for b in [GBUF, PBUF, PBUF2] {
        mem[b] = mem[b + 1];
    }
    mem[GBUF + 4] = mem[PAST_GAIN_CODE];
    mem[PBUF + 4] = gain_pit;
    mem[PBUF2 + 4] = gain_pit;
    let (e, f) = to_dpf(gain_cod);
    let gain_cod = l_shl(mul_32by16(e, f, gcode_inov), 3);
    mem[3] = mem[2];
    mem[2] = mem[1];
    mem[1] = mem[0];
    let (e, f) = log2(i32::from(g_code));
    let l = mul_32by16(e.wrapping_sub(11), f, 24660);
    mem[0] = (l >> 3) as i16;
    (gain_pit, gain_cod)
}

pub(crate) fn reorder_isf(isf: &mut [i16; M], min_dist: i16) {
    let mut isf_min = min_dist;
    for v in isf.iter_mut().take(M - 1) {
        if *v < isf_min {
            *v = isf_min;
        }
        isf_min = add(*v, min_dist);
    }
}

const MU: i16 = 10923;
const ALPHA: i16 = 29491;
const ONE_ALPHA: i16 = (32768 - 29491) as i16;
pub(crate) const L_MEANBUF: usize = 3;

/// The ISF predictor state shared by the 46- and 36-bit quantisers.
pub(crate) struct IsfState<'a> {
    pub(crate) past_isfq: &'a mut [i16; M],
    pub(crate) isfold: &'a [i16; M],
    pub(crate) isf_buf: &'a mut [[i16; M]; L_MEANBUF],
}

fn isf_conceal(isf_q: &mut [i16; M], st: &mut IsfState<'_>) {
    let mut ref_isf = [0i16; M];
    for i in 0..M {
        let mut l = l_mult(MEAN_ISF[i], 8192);
        for b in st.isf_buf.iter() {
            l = l_mac(l, b[i], 8192);
        }
        ref_isf[i] = round(l);
    }
    for i in 0..M {
        isf_q[i] = add(mult(ALPHA, st.isfold[i]), mult(ONE_ALPHA, ref_isf[i]));
    }
    for i in 0..M {
        let tmp = add(ref_isf[i], mult(st.past_isfq[i], MU));
        st.past_isfq[i] = sub(isf_q[i], tmp) >> 1;
    }
}

fn push_isf_buf(isf_q: &[i16; M], st: &mut IsfState<'_>) {
    st.isf_buf[2] = st.isf_buf[1];
    st.isf_buf[1] = st.isf_buf[0];
    st.isf_buf[0] = *isf_q;
}

/// `Dpisf_2s_46b`.
pub(crate) fn dpisf_2s_46b(ind: &[i16; 8], isf_q: &mut [i16; M], st: &mut IsfState<'_>, bfi: bool) {
    if bfi {
        isf_conceal(isf_q, st);
    } else {
        let i0 = i32::from(ind[0]);
        let i1 = i32::from(ind[1]);
        for i in 0..9 {
            isf_q[i] = at(&DICO1_ISF, (i0 << 3) + i0 + i as i32);
        }
        for i in 0..7 {
            isf_q[i + 9] = at(&DICO2_ISF, (i1 << 3) - i1 + i as i32);
        }
        for i in 0..3 {
            let k = i as i32;
            isf_q[i] = isf_q[i].wrapping_add(at(&DICO21_ISF, i32::from(ind[2]) * 3 + k));
            isf_q[i + 3] = isf_q[i + 3].wrapping_add(at(&DICO22_ISF, i32::from(ind[3]) * 3 + k));
            isf_q[i + 6] = isf_q[i + 6].wrapping_add(at(&DICO23_ISF, i32::from(ind[4]) * 3 + k));
            isf_q[i + 9] = isf_q[i + 9].wrapping_add(at(&DICO24_ISF, i32::from(ind[5]) * 3 + k));
            isf_q[i + 12] = isf_q[i + 12].wrapping_add(at(&DICO25_ISF, (i32::from(ind[6]) << 2) + k));
        }
        isf_q[15] = isf_q[15].wrapping_add(at(&DICO25_ISF, (i32::from(ind[6]) << 2) + 3));
        for i in 0..M {
            let tmp = isf_q[i];
            isf_q[i] = isf_q[i].wrapping_add(MEAN_ISF[i]);
            isf_q[i] = isf_q[i].wrapping_add(((i32::from(MU) * i32::from(st.past_isfq[i])) >> 15) as i16);
            st.past_isfq[i] = tmp;
        }
        push_isf_buf(isf_q, st);
    }
    reorder_isf(isf_q, 128);
}

/// `Dpisf_2s_36b`.
pub(crate) fn dpisf_2s_36b(ind: &[i16; 8], isf_q: &mut [i16; M], st: &mut IsfState<'_>, bfi: bool) {
    if bfi {
        isf_conceal(isf_q, st);
    } else {
        for i in 0..9 {
            isf_q[i] = at(&DICO1_ISF, i32::from(ind[0]) * 9 + i as i32);
        }
        for i in 0..7 {
            let k = i as i32;
            isf_q[i + 9] = add(at(&DICO2_ISF, i32::from(ind[1]) * 7 + k), at(&DICO23_ISF_36B, i32::from(ind[4]) * 7 + k));
        }
        for i in 0..5 {
            isf_q[i] = add(isf_q[i], at(&DICO21_ISF_36B, i32::from(ind[2]) * 5 + i as i32));
        }
        for i in 0..4 {
            isf_q[i + 5] = add(isf_q[i + 5], at(&DICO22_ISF_36B, (i32::from(ind[3]) << 2) + i as i32));
        }
        for i in 0..M {
            let tmp = isf_q[i];
            isf_q[i] = add(tmp, MEAN_ISF[i]);
            isf_q[i] = add(isf_q[i], mult(MU, st.past_isfq[i]));
            st.past_isfq[i] = tmp;
        }
        push_isf_buf(isf_q, st);
    }
    reorder_isf(isf_q, 128);
}

/// `Disf_ns`: the comfort-noise ISFs of a SID frame.
pub(crate) fn disf_ns(ind: &[i16; 5], isf_q: &mut [i16; M]) {
    let i0 = i32::from(ind[0]) << 1;
    isf_q[0] = at(&DICO1_ISF_NOISE, i0);
    isf_q[1] = at(&DICO1_ISF_NOISE, i0 + 1);
    for i in 0..3 {
        let k = i as i32;
        isf_q[i + 2] = at(&DICO2_ISF_NOISE, i32::from(ind[1]) * 3 + k);
        isf_q[i + 5] = at(&DICO3_ISF_NOISE, i32::from(ind[2]) * 3 + k);
    }
    for i in 0..4 {
        let k = i as i32;
        isf_q[i + 8] = at(&DICO4_ISF_NOISE, (i32::from(ind[3]) << 2) + k);
        isf_q[i + 12] = at(&DICO5_ISF_NOISE, (i32::from(ind[4]) << 2) + k);
    }
    for i in 0..M {
        isf_q[i] = add(isf_q[i], MEAN_ISF_NOISE[i]);
    }
    reorder_isf(isf_q, 128);
}

/// `lagconceal`: replaces a lost or suspicious pitch lag.
pub(crate) fn lagconceal(gain_hist: &[i16], lag_hist: &[i16; 5], t0: &mut i16, old_t0: i16, seed: &mut i16, unusable: bool) {
    const ONE_PER_3: i16 = 10923;
    const ONE_PER_LTPHIST: i16 = 6554;
    let gh = |i: usize| gain_hist.get(i).copied().unwrap_or(0);
    let last_gain = gh(4);
    let sec_last_gain = gh(3);
    let last_lag = lag_hist[0];
    let min_lag = lag_hist.iter().copied().min().unwrap_or(0);
    let max_lag = lag_hist.iter().copied().max().unwrap_or(0);
    let min_gain = (0..5).map(gh).min().unwrap_or(0);
    let lag_dif = sub(max_lag, min_lag);
    let random_lag = |seed: &mut i16| {
        let mut h = *lag_hist;
        h.sort_unstable();
        let mut d = sub(h[4], h[2]);
        if d > 40 {
            d = 40;
        }
        let dn = noise(seed);
        let d2 = mult(d >> 1, dn);
        let tmp = add(add(h[2], h[3]), h[4]);
        add(mult(tmp, ONE_PER_3), d2)
    };
    let clamp = |t: &mut i16| {
        if *t > max_lag {
            *t = max_lag;
        }
        if *t < min_lag {
            *t = min_lag;
        }
    };
    if unusable {
        if min_gain > 8192 && lag_dif < 10 {
            *t0 = old_t0;
        } else if last_gain > 8192 && sec_last_gain > 8192 {
            *t0 = lag_hist[0];
        } else {
            *t0 = random_lag(seed);
        }
        clamp(t0);
    } else {
        let mut mean_lag: i16 = 0;
        for &l in lag_hist {
            mean_lag = add(mean_lag, l);
        }
        let mean_lag = mult(mean_lag, ONE_PER_LTPHIST);
        let t = i32::from(*t0);
        let tmp = (t - i32::from(max_lag)) as i16;
        let tmp2 = (t - i32::from(last_lag)) as i16;
        let keep = (lag_dif < 10 && t > i32::from(min_lag) - 5 && tmp < 5)
            || (last_gain > 8192 && sec_last_gain > 8192 && (i32::from(tmp2) + 10) > 0 && tmp2 < 10)
            || (min_gain < 6554 && last_gain == min_gain && *t0 > min_lag && *t0 < max_lag)
            || (lag_dif < 70 && *t0 > min_lag && *t0 < max_lag)
            || (*t0 > mean_lag && *t0 < max_lag);
        if !keep {
            if (min_gain > 8192 && lag_dif < 10) || (last_gain > 8192 && sec_last_gain > 8192) {
                *t0 = lag_hist[0];
            } else {
                *t0 = random_lag(seed);
            }
            clamp(t0);
        }
    }
}
