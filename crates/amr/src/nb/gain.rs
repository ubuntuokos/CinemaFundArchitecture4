// Derived from opencore-amr 0.1.6 (commit 7dba8c32), amr_nb: gmed_n.cpp, gc_pred.cpp,
// dec_gain.cpp, d_gain_c.cpp, d_gain_p.cpp, ec_gains.cpp, c_g_aver.cpp.
// Copyright (C) 1998-2009 PacketVideo, Apache License 2.0; portions (C) 2004 3GPP
// Organizational Partners (3GPP TS 26.073). Ported to Rust for CFA4 (GPL-3.0-or-later).

//! Gain decoding, prediction, concealment and smoothing.

use super::math::{log2, log2_norm, pow2};
use super::tables::{QUA_GAIN_CODE, QUA_GAIN_PITCH, TABLE_GAIN_HIGHRATES, TABLE_GAIN_LOWRATES, TABLE_GAIN_MR475};
use super::{L_SUBFR, M, Mode};
use crate::basic::{
    MAX_32, MIN_32, abs_s, add, at, div_s, l_add, l_mac, l_msu, l_mult, l_shl, l_shr, l_sub, mpy_32_16, mult, negate, norm_l, norm_s, round, shl, shr, shr_r,
    sub,
};

/// `gmed_n`: the median of `ind` (n odd, at most 9).
pub(crate) fn gmed_n(ind: &[i16]) -> i16 {
    let n = ind.len().min(9);
    let mut tmp2 = [0i16; 9];
    tmp2[..n].copy_from_slice(&ind[..n]);
    let mut order = [0usize; 9];
    for slot in order.iter_mut().take(n) {
        let mut max = -32767i16;
        let mut ix = 0usize;
        for (j, &v) in tmp2.iter().enumerate().take(n) {
            if v >= max {
                max = v;
                ix = j;
            }
        }
        tmp2[ix] = -32768;
        *slot = ix;
    }
    ind.get(order[n >> 1]).copied().unwrap_or(0)
}

const MEAN_ENER_MR122: i32 = 783_741;
const MIN_ENERGY: i16 = -14336;
const MIN_ENERGY_MR122: i16 = -2381;
const PRED: [i16; 4] = [5571, 4751, 2785, 1556];
const PRED_MR122: [i16; 4] = [44, 37, 22, 12];

/// MA predictor state of the fixed-codebook gain.
#[derive(Clone, Debug)]
pub(crate) struct GcPred {
    pub(crate) past_qua_en: [i16; 4],
    pub(crate) past_qua_en_mr122: [i16; 4],
}

impl GcPred {
    pub(crate) fn new() -> Self {
        GcPred { past_qua_en: [MIN_ENERGY; 4], past_qua_en_mr122: [MIN_ENERGY_MR122; 4] }
    }

    pub(crate) fn update(&mut self, qua_ener_mr122: i16, qua_ener: i16) {
        for i in (1..4).rev() {
            self.past_qua_en[i] = self.past_qua_en[i - 1];
            self.past_qua_en_mr122[i] = self.past_qua_en_mr122[i - 1];
        }
        self.past_qua_en_mr122[0] = qua_ener_mr122;
        self.past_qua_en[0] = qua_ener;
    }

    pub(crate) fn average_limited(&self) -> (i16, i16) {
        let avg = |v: &[i16; 4], min: i16| {
            let mut s: i16 = 0;
            for &e in v {
                s = add(s, e);
            }
            (s >> 2).max(min)
        };
        (avg(&self.past_qua_en_mr122, MIN_ENERGY_MR122), avg(&self.past_qua_en, MIN_ENERGY))
    }

    /// `gc_pred`: the predicted gain as `(exp, frac)`, plus the innovation energy `(exp, frac)`
    /// used by MR795.
    pub(crate) fn predict(&self, mode: Mode, code: &[i16; L_SUBFR]) -> (i16, i16, i16, i16) {
        let mut ener_code: i32 = 0;
        for &c in code {
            ener_code = ener_code.wrapping_add((i32::from(c) * i32::from(c)) >> 3);
        }
        ener_code = ener_code.wrapping_shl(4);
        if ener_code < 0 {
            ener_code = MAX_32;
        }
        if mode == Mode::Mr122 {
            let ener_code = (i32::from(round(ener_code)) * 26214) << 1;
            let (exp, frac) = log2(ener_code);
            let ener_code = ((i32::from(exp) - 30) << 16).wrapping_add(i32::from(frac) << 1);
            let mut ener = MEAN_ENER_MR122;
            for i in 0..4 {
                ener = l_add(ener, (i32::from(self.past_qua_en_mr122[i]) * i32::from(PRED_MR122[i])) << 1);
            }
            let t = l_sub(ener, ener_code);
            let exp_g = (t >> 17) as i16;
            let frac_g = ((t >> 2).wrapping_sub(i32::from(exp_g) << 15)) as i16;
            return (exp_g, frac_g, 0, 0);
        }
        let exp_code = norm_l(ener_code);
        let ener_code = l_shl(ener_code, exp_code);
        let (exp, frac) = log2_norm(ener_code, exp_code);
        let t2 = (i32::from(exp) * -24660) << 1;
        let mut tmp = (i32::from(frac) * -24660) >> 15;
        if tmp & 0x0001_0000 != 0 {
            tmp |= 0xffff_0000u32 as i32;
        }
        tmp <<= 1;
        tmp = l_add(tmp, t2);
        let (mut exp_en, mut frac_en) = (0, 0);
        let add_c = match mode {
            Mode::Mr102 => 16678 << 7,
            Mode::Mr795 => {
                frac_en = (ener_code >> 16) as i16;
                exp_en = -11 - exp_code;
                17062 << 7
            }
            Mode::Mr74 => 32588 << 6,
            Mode::Mr67 => 32268 << 6,
            _ => 16678 << 7,
        };
        tmp = l_add(tmp, add_c);
        tmp = if tmp > 0x001f_ffff {
            MAX_32
        } else if tmp < -2_097_152 {
            MIN_32
        } else {
            tmp << 10
        };
        for i in 0..4 {
            tmp = l_add(tmp, (i32::from(PRED[i]) * i32::from(self.past_qua_en[i])) << 1);
        }
        let gcode0 = (tmp >> 16) as i16;
        let mut t = if mode == Mode::Mr74 { (i32::from(gcode0) * 5439) << 1 } else { (i32::from(gcode0) * 5443) << 1 };
        t >>= 8;
        let exp_g = (t >> 16) as i16;
        let frac_g = l_sub(t >> 1, i32::from(exp_g) << 15) as i16;
        (exp_g, frac_g, exp_en, frac_en)
    }
}

/// `Dec_gain`: the jointly quantised pitch and code gains of MR475, MR515, MR59, MR67, MR74
/// and MR102. Returns `(gain_pit, gain_code)`.
pub(crate) fn dec_gain(pred: &mut GcPred, mode: Mode, index: i16, code: &[i16; L_SUBFR], even_subfr: bool) -> (i16, i16) {
    let mut index = i32::from(shl(index, 2));
    let (gain_pit, g_code, qua_ener_mr122, qua_ener);
    if matches!(mode, Mode::Mr102 | Mode::Mr74 | Mode::Mr67) {
        let t = &TABLE_GAIN_HIGHRATES[..];
        gain_pit = at(t, index);
        g_code = at(t, index + 1);
        qua_ener_mr122 = at(t, index + 2);
        qua_ener = at(t, index + 3);
    } else if mode == Mode::Mr475 {
        index += i32::from(1 ^ i16::from(even_subfr)) << 1;
        index = index.min(256 * 4 - 2);
        gain_pit = at(&TABLE_GAIN_MR475, index);
        g_code = at(&TABLE_GAIN_MR475, index + 1);
        let (exp, frac) = log2(i32::from(g_code));
        let exp = exp - 12;
        qua_ener_mr122 = add(shr_r(frac, 5), shl(exp, 10));
        let t = l_shl(mpy_32_16(exp, frac, 24660), 13);
        qua_ener = round(t);
    } else {
        let t = &TABLE_GAIN_LOWRATES[..];
        gain_pit = at(t, index);
        g_code = at(t, index + 1);
        qua_ener_mr122 = at(t, index + 2);
        qua_ener = at(t, index + 3);
    }
    let (exp, frac, _, _) = pred.predict(mode, code);
    let gcode0 = pow2(14, frac) as i16;
    let t = l_shr(l_mult(g_code, gcode0), 10i16.wrapping_sub(exp));
    pred.update(qua_ener_mr122, qua_ener);
    (gain_pit, (t >> 16) as i16)
}

/// `d_gain_code`: the scalar code gain of MR795 and MR122.
pub(crate) fn d_gain_code(pred: &mut GcPred, mode: Mode, index: i16, code: &[i16; L_SUBFR]) -> i16 {
    let (exp, frac, _, _) = pred.predict(mode, code);
    let p = i32::from(index & 31) * 3;
    let g = at(&QUA_GAIN_CODE, p);
    let gain = if mode == Mode::Mr122 {
        let gcode0 = shl(pow2(exp, frac) as i16, 4);
        shl(mult(gcode0, g), 1)
    } else {
        let gcode0 = pow2(14, frac) as i16;
        (l_shr(l_mult(g, gcode0), sub(9, exp)) >> 16) as i16
    };
    pred.update(at(&QUA_GAIN_CODE, p + 1), at(&QUA_GAIN_CODE, p + 2));
    gain
}

pub(crate) fn d_gain_pitch(mode: Mode, index: i16) -> i16 {
    let g = at(&QUA_GAIN_PITCH, i32::from(index));
    if mode == Mode::Mr122 { g & !3 } else { g }
}

/// Concealment state of the pitch gain.
#[derive(Clone, Debug)]
pub(crate) struct EcGainPitch {
    pbuf: [i16; 5],
    past_gain_pit: i16,
    prev_gp: i16,
}

impl EcGainPitch {
    pub(crate) fn new() -> Self {
        EcGainPitch { pbuf: [1640; 5], past_gain_pit: 0, prev_gp: 16384 }
    }

    pub(crate) fn conceal(&self, state: i16) -> i16 {
        const PDOWN: [i16; 7] = [32767, 32112, 32112, 26214, 9830, 6553, 6553];
        let mut tmp = gmed_n(&self.pbuf);
        if sub(tmp, self.past_gain_pit) > 0 {
            tmp = self.past_gain_pit;
        }
        mult(tmp, at(&PDOWN, i32::from(state)))
    }

    pub(crate) fn update(&mut self, bfi: bool, prev_bf: bool, gain: &mut i16) {
        if !bfi {
            if prev_bf && sub(*gain, self.prev_gp) > 0 {
                *gain = self.prev_gp;
            }
            self.prev_gp = *gain;
        }
        self.past_gain_pit = (*gain).min(16384);
        self.pbuf.copy_within(1.., 0);
        self.pbuf[4] = self.past_gain_pit;
    }
}

/// Concealment state of the code gain.
#[derive(Clone, Debug)]
pub(crate) struct EcGainCode {
    gbuf: [i16; 5],
    past_gain_code: i16,
    prev_gc: i16,
}

impl EcGainCode {
    pub(crate) fn new() -> Self {
        EcGainCode { gbuf: [1; 5], past_gain_code: 0, prev_gc: 1 }
    }

    pub(crate) fn conceal(&self, pred: &mut GcPred, state: i16) -> i16 {
        const CDOWN: [i16; 7] = [32767, 32112, 32112, 32112, 32112, 32112, 22937];
        let mut tmp = gmed_n(&self.gbuf);
        if sub(tmp, self.past_gain_code) > 0 {
            tmp = self.past_gain_code;
        }
        let g = mult(tmp, at(&CDOWN, i32::from(state)));
        let (a, b) = pred.average_limited();
        pred.update(a, b);
        g
    }

    pub(crate) fn update(&mut self, bfi: bool, prev_bf: bool, gain: &mut i16) {
        if !bfi {
            if prev_bf && sub(*gain, self.prev_gc) > 0 {
                *gain = self.prev_gc;
            }
            self.prev_gc = *gain;
        }
        self.past_gain_code = *gain;
        self.gbuf.copy_within(1.., 0);
        self.gbuf[4] = *gain;
    }
}

const L_CBGAINHIST: usize = 7;

/// `Cb_gain_average`: code-gain smoothing in stationary background noise.
#[derive(Clone, Debug)]
pub(crate) struct CbGainAverage {
    hist: [i16; L_CBGAINHIST],
    pub(crate) hang_var: i16,
    pub(crate) hang_count: i16,
}

/// Frame flags `Cb_gain_average` looks at.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Flags {
    pub(crate) bfi: bool,
    pub(crate) prev_bf: bool,
    pub(crate) pdfi: bool,
    pub(crate) prev_pdf: bool,
    pub(crate) in_background_noise: bool,
    pub(crate) voiced_hangover: i16,
}

impl CbGainAverage {
    pub(crate) fn new() -> Self {
        CbGainAverage { hist: [0; L_CBGAINHIST], hang_var: 0, hang_count: 0 }
    }

    pub(crate) fn average(&mut self, mode: Mode, gain_code: i16, lsp: &[i16; M], lsp_aver: &[i16; M], f: Flags) -> i16 {
        let mut mix = gain_code;
        self.hist.copy_within(1.., 0);
        self.hist[L_CBGAINHIST - 1] = gain_code;
        let mut diff: i16 = 0;
        for i in 0..M {
            let mut t1 = abs_s(sub(lsp_aver[i], lsp[i]));
            let shift1 = norm_s(t1) - 1;
            t1 = shl(t1, shift1);
            let shift2 = norm_s(lsp_aver[i]);
            let t2 = shl(lsp_aver[i], shift2);
            let mut t = div_s(t1, t2);
            let shift = 2 + shift1 - shift2;
            t = if shift >= 0 { shr(t, shift) } else { shl(t, negate(shift)) };
            diff = add(diff, t);
        }
        if diff > 5325 {
            self.hang_var = self.hang_var.wrapping_add(1);
        } else {
            self.hang_var = 0;
        }
        if self.hang_var > 10 {
            self.hang_count = 0;
        }
        let low_rate = matches!(mode, Mode::Mr475 | Mode::Mr515 | Mode::Mr59);
        if mode <= Mode::Mr67 || mode == Mode::Mr102 {
            let tmp_diff = if ((f.pdfi && f.prev_pdf) || f.bfi || f.prev_bf) && f.voiced_hangover > 1 && f.in_background_noise && low_rate {
                diff.wrapping_sub(4506)
            } else {
                diff.wrapping_sub(3277)
            };
            let t1 = tmp_diff.max(0);
            let mut bg_mix = if 2048 < t1 { 8192 } else { shl(t1, 2) };
            if self.hang_count < 40 || diff > 5325 {
                bg_mix = 8192;
            }
            let mut sum = l_mult(6554, self.hist[2]);
            for &h in &self.hist[3..] {
                sum = l_mac(sum, 6554, h);
            }
            let mut mean = round(sum);
            if (f.bfi || f.prev_bf) && f.in_background_noise && low_rate {
                let mut sum = l_mult(4681, self.hist[0]);
                for &h in &self.hist[1..] {
                    sum = l_mac(sum, 4681, h);
                }
                mean = round(sum);
            }
            let mut sum = l_mult(bg_mix, mix);
            sum = l_mac(sum, 8192, mean);
            sum = l_msu(sum, bg_mix, mean);
            mix = round(l_shl(sum, 2));
        }
        self.hang_count = self.hang_count.wrapping_add(1);
        mix
    }
}
