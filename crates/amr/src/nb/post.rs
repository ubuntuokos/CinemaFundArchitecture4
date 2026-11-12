// Derived from opencore-amr 0.1.6 (commit 7dba8c32), amr_nb: pstfilt.cpp, agc.cpp,
// preemph.cpp, post_pro.cpp, ph_disp.cpp, ex_ctrl.cpp, bgnscd.cpp.
// Copyright (C) 1998-2009 PacketVideo, Apache License 2.0; portions (C) 2004 3GPP
// Organizational Partners (3GPP TS 26.073). Ported to Rust for CFA4 (GPL-3.0-or-later).

//! Excitation enhancement (phase dispersion, excitation control, background-noise detection),
//! the formant post-filter and the output high-pass filter.

use super::gain::gmed_n;
use super::lpc::{residu, syn_filt, weight_ai};
use super::math::inv_sqrt;
use super::tables::{PH_IMP_LOW, PH_IMP_LOW_MR795, PH_IMP_MID, PH_IMP_MID_MR795};
use super::{L_FRAME, L_SUBFR, M, MP1, Mode};
use crate::basic::{MAX_16, MAX_32, MIN_16, MIN_32, add, div_s, l_add, l_mac, l_mult, l_shl, l_shr, mult, norm_l, norm_s, round, shl, shl32, sub};

fn energy_old(x: &[i16]) -> i32 {
    let mut s = 0i32;
    for &v in x {
        let t = v >> 2;
        s = l_mac(s, t, t);
    }
    s
}

fn energy_new(x: &[i16]) -> i32 {
    let mut s = 0i32;
    for &v in x {
        s = l_mac(s, v, v);
    }
    if s != MAX_32 { s >> 4 } else { energy_old(x) }
}

/// `agc`: scales `out` to the energy of `input`, with the gain smoothed across calls.
fn agc(past_gain: &mut i16, input: &[i16], out: &mut [i16], agc_fac: i16) {
    let s = energy_new(out);
    if s == 0 {
        *past_gain = 0;
        return;
    }
    let mut exp = norm_l(s) - 1;
    let gain_out = round(l_shl(s, exp));
    let s = energy_new(input);
    let g0 = if s == 0 {
        0
    } else {
        let i = norm_l(s);
        let gain_in = round(shl32(s, i32::from(i)));
        exp -= i;
        let temp = div_s(gain_out, gain_in);
        let s = l_shr(i32::from(temp) << 7, exp);
        let s = inv_sqrt(s);
        let t = shl32(s, 9);
        let i = (t.wrapping_add(0x8000) >> 16) as i16;
        let temp = 32767 - agc_fac;
        ((i32::from(i) * i32::from(temp)) >> 15) as i16
    };
    let mut gain = *past_gain;
    for v in out.iter_mut() {
        gain = ((i32::from(gain) * i32::from(agc_fac)) >> 15) as i16;
        gain = gain.wrapping_add(g0);
        let t = (i32::from(*v) * i32::from(gain)).wrapping_shl(1);
        *v = (t >> 13) as i16;
    }
    *past_gain = gain;
}

/// `agc2`: scales `out` to the energy of `input` (no smoothing).
pub(crate) fn agc2(input: &[i16], out: &mut [i16]) {
    let s = energy_new(out);
    if s == 0 {
        return;
    }
    let mut exp = norm_l(s) - 1;
    let gain_out = round(l_shl(s, exp));
    let s = energy_new(input);
    let g0 = if s == 0 {
        0
    } else {
        let i = norm_l(s);
        let gain_in = round(l_shl(s, i));
        exp -= i;
        let temp = i32::from(div_s(gain_out, gain_in));
        let s = if temp > 0x00ff_ffff {
            MAX_32
        } else if temp < -16_777_216 {
            MIN_32
        } else {
            temp << 7
        };
        let s = inv_sqrt(l_shr(s, exp));
        let t = if s > 0x003f_ffff {
            MAX_32
        } else if s < -4_194_304 {
            MIN_32
        } else {
            s << 9
        };
        round(t)
    };
    for v in out.iter_mut().rev() {
        let t = l_mult(*v, g0);
        *v = if t > 0x0fff_ffff {
            MAX_16
        } else if t < -268_435_456 {
            MIN_16
        } else {
            (t >> 13) as i16
        };
    }
}

const L_H: usize = 22;
const MU: i32 = 26214;
const AGC_FAC: i16 = 29491;
const GAMMA3_MR122: [i16; M] = [22938, 16057, 11240, 7868, 5508, 3856, 2699, 1889, 1322, 925];
const GAMMA3: [i16; M] = [18022, 9912, 5451, 2998, 1649, 907, 499, 274, 151, 83];
const GAMMA4_MR122: [i16; M] = [24576, 18432, 13824, 10368, 7776, 5832, 4374, 3281, 2461, 1846];
const GAMMA4: [i16; M] = [22938, 16057, 11240, 7868, 5508, 3856, 2699, 1889, 1322, 925];

/// The formant post-filter with tilt compensation and gain control (`Post_Filter`).
#[derive(Clone, Debug)]
pub(crate) struct PostFilter {
    mem_syn_pst: [i16; M],
    synth_buf: [i16; L_FRAME + M],
    past_gain: i16,
    mem_pre: i16,
}

impl PostFilter {
    pub(crate) fn new() -> Self {
        PostFilter { mem_syn_pst: [0; M], synth_buf: [0; L_FRAME + M], past_gain: 4096, mem_pre: 0 }
    }

    pub(crate) fn run(&mut self, mode: Mode, syn: &mut [i16; L_FRAME], az4: &[i16; 4 * MP1]) {
        self.synth_buf[M..].copy_from_slice(syn);
        let high = matches!(mode, Mode::Mr122 | Mode::Mr102);
        let (g3, g4) = if high { (&GAMMA3_MR122, &GAMMA4_MR122) } else { (&GAMMA3, &GAMMA4) };
        for (sf, az) in az4.chunks_exact(MP1).enumerate() {
            let i_subfr = sf * L_SUBFR;
            let mut ap3 = [0i16; MP1];
            let mut ap4 = [0i16; MP1];
            weight_ai(az, g3, &mut ap3);
            weight_ai(az, g4, &mut ap4);
            let mut res2 = [0i16; L_SUBFR];
            residu(&ap3, &self.synth_buf[i_subfr..i_subfr + M + L_SUBFR], &mut res2);
            let mut h = [0i16; L_H];
            h[..MP1].copy_from_slice(&ap3);
            let hin = h;
            let mut zero_mem = [0i16; M];
            syn_filt(&ap4, &hin, &mut h, &mut zero_mem, false);
            let mut acc = 0i32;
            for i in (0..L_H).rev() {
                let p = i32::from(h[i]) * i32::from(h[i]);
                if p == 0x4000_0000 {
                    break;
                }
                acc = l_add(acc, p << 1);
            }
            let temp1 = (acc >> 16) as i16;
            let mut acc = 0i32;
            for i in (0..L_H - 1).rev() {
                let p = i32::from(h[i]) * i32::from(h[i + 1]);
                if p == 0x4000_0000 {
                    break;
                }
                acc = l_add(acc, p << 1);
            }
            let mut temp2 = (acc >> 16) as i16;
            if temp2 <= 0 {
                temp2 = 0;
            } else {
                let mut t = (i32::from(temp2) * MU) >> 15;
                if t & 0x0001_0000 != 0 {
                    t |= 0xffff_0000u32 as i32;
                }
                temp2 = div_s(t as i16, temp1);
            }
            // Preemphasis (tilt compensation) of the residual.
            let last = res2[L_SUBFR - 1];
            for i in (1..L_SUBFR).rev() {
                res2[i] = sub(res2[i], mult(temp2, res2[i - 1]));
            }
            res2[0] = sub(res2[0], mult(temp2, self.mem_pre));
            self.mem_pre = last;
            let out = &mut syn[i_subfr..i_subfr + L_SUBFR];
            syn_filt(&ap4, &res2, out, &mut self.mem_syn_pst, true);
            agc(&mut self.past_gain, &self.synth_buf[M + i_subfr..M + i_subfr + L_SUBFR], out, AGC_FAC);
        }
        self.synth_buf.copy_within(L_FRAME..L_FRAME + M, 0);
    }
}

/// The 60 Hz high-pass filter and ×2 output scaling (`Post_Process`).
#[derive(Clone, Debug, Default)]
pub(crate) struct PostProcess {
    y2_hi: i16,
    y2_lo: i16,
    y1_hi: i16,
    y1_lo: i16,
    x0: i16,
    x1: i16,
}

impl PostProcess {
    pub(crate) fn run(&mut self, signal: &mut [i16]) {
        const B: [i32; 3] = [7699, -15398, 7699];
        const A: [i32; 3] = [8192, 15836, -7667];
        for s in signal.iter_mut() {
            let x2 = self.x1;
            self.x1 = self.x0;
            self.x0 = *s;
            let mut t = i32::from(self.y1_hi) * A[1];
            t = t.wrapping_add((i32::from(self.y1_lo) * A[1]) >> 15);
            t = t.wrapping_add(i32::from(self.y2_hi) * A[2]);
            t = t.wrapping_add((i32::from(self.y2_lo) * A[2]) >> 15);
            t = t.wrapping_add(i32::from(self.x0) * B[0]);
            t = t.wrapping_add(i32::from(self.x1) * B[1]);
            t = t.wrapping_add(i32::from(x2) * B[2]);
            t = l_shl(t, 3);
            *s = round(l_shl(t, 1));
            self.y2_hi = self.y1_hi;
            self.y2_lo = self.y1_lo;
            self.y1_hi = (t >> 16) as i16;
            self.y1_lo = ((t >> 1).wrapping_sub(i32::from(self.y1_hi) << 15)) as i16;
        }
    }
}

const PHDTHR1LTP: i16 = 9830;
const PHDTHR2LTP: i16 = 14746;
const ONFACTPLUS1: i32 = 16384;
const ONLENGTH: i16 = 2;

/// Anti-sparseness processing of the innovation (`ph_disp`).
#[derive(Clone, Debug, Default)]
pub(crate) struct PhDisp {
    gain_mem: [i16; 5],
    prev_state: i16,
    prev_cb_gain: i16,
    pub(crate) lock_full: bool,
    onset: i16,
}

impl PhDisp {
    /// Builds the total excitation into `x` (which holds the pitch excitation on entry).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run(&mut self, mode: Mode, x: &mut [i16; L_SUBFR], cb_gain: i16, ltp_gain: i16, inno: &mut [i16; L_SUBFR], pitch_fac: i16, tmp_shift: i16) {
        self.gain_mem.copy_within(0..4, 1);
        self.gain_mem[0] = ltp_gain;
        let mut imp_nr: i16 = if ltp_gain < PHDTHR2LTP { i16::from(ltp_gain > PHDTHR1LTP) } else { 2 };
        let t = (i32::from(self.prev_cb_gain) * ONFACTPLUS1) << 1;
        let t = if t > 0x1fff_ffff {
            MAX_32
        } else if t < -536_870_912 {
            MIN_32
        } else {
            t << 2
        };
        let tmp1 = round(t);
        if cb_gain > tmp1 {
            self.onset = ONLENGTH;
        } else if self.onset > 0 {
            self.onset -= 1;
        }
        if self.onset == 0 {
            let low = self.gain_mem.iter().filter(|&&g| g < PHDTHR1LTP).count();
            if low > 2 {
                imp_nr = 0;
            }
        }
        if i32::from(imp_nr) > i32::from(self.prev_state) + 1 && self.onset == 0 {
            imp_nr -= 1;
        }
        if imp_nr < 2 && self.onset > 0 {
            imp_nr += 1;
        }
        if cb_gain < 10 {
            imp_nr = 2;
        }
        if self.lock_full {
            imp_nr = 0;
        }
        self.prev_state = imp_nr;
        self.prev_cb_gain = cb_gain;
        if !matches!(mode, Mode::Mr122 | Mode::Mr102 | Mode::Mr74) && imp_nr < 2 {
            let sav = *inno;
            *inno = [0; L_SUBFR];
            let ph_imp: &[i16; L_SUBFR] = match (mode == Mode::Mr795, imp_nr == 0) {
                (true, true) => &PH_IMP_LOW_MR795,
                (true, false) => &PH_IMP_MID_MR795,
                (false, true) => &PH_IMP_LOW,
                (false, false) => &PH_IMP_MID,
            };
            for (ppos, &c) in sav.iter().enumerate() {
                if c == 0 {
                    continue;
                }
                for (k, &imp) in ph_imp.iter().enumerate() {
                    let i = (ppos + k) % L_SUBFR;
                    let t = ((i32::from(c) * i32::from(imp)) >> 15) as i16;
                    inno[i] = add(inno[i], t);
                }
            }
        }
        for i in 0..L_SUBFR {
            let mut t = l_mult(x[i], pitch_fac);
            t = l_add(t, (i32::from(inno[i]) * i32::from(cb_gain)) << 1);
            t = l_shl(t, tmp_shift);
            x[i] = round(t);
        }
    }
}

/// `Ex_ctrl`: limits the excitation energy after errors in background noise.
pub(crate) fn ex_ctrl(exc: &mut [i16; L_SUBFR], exc_energy: i16, hist: &[i16; 9], voiced_hangover: i16, prev_bfi: bool, careful: bool) {
    let mut avg = gmed_n(hist);
    let mut prev = ((i32::from(hist[7]) + i32::from(hist[8])) >> 1) as i16;
    if hist[8] < prev {
        prev = hist[8];
    }
    if exc_energy < avg && exc_energy > 5 {
        let mut test = shl(prev, 2);
        if voiced_hangover < 7 || prev_bfi {
            test = sub(test, prev);
        }
        if avg > test {
            avg = test;
        }
        let exp = norm_s(exc_energy);
        let e = shl(exc_energy, exp);
        let e = div_s(16383, e);
        let mut t0 = l_shr(l_mult(avg, e), sub(20, exp));
        if t0 > 32767 {
            t0 = 32767;
        }
        let mut scale = t0 as i16;
        if careful && scale > 3072 {
            scale = 3072;
        }
        for v in exc.iter_mut() {
            *v = l_shr(l_mult(scale, *v), 11) as i16;
        }
    }
}

const L_ENERGYHIST: usize = 60;
const FRAMEENERGYLIMIT: i16 = 17578;
const LOWERNOISELIMIT: i16 = 20;
const UPPERNOISELIMIT: i16 = 1953;

/// Background-noise detection (`Bgn_scd`).
#[derive(Clone, Debug)]
pub(crate) struct BgnScd {
    frame_energy_hist: [i16; L_ENERGYHIST],
    bg_hangover: i16,
}

impl BgnScd {
    pub(crate) fn new() -> Self {
        BgnScd { frame_energy_hist: [0; L_ENERGYHIST], bg_hangover: 0 }
    }

    pub(crate) fn run(&mut self, ltp_gain_hist: &[i16; 9], speech: &[i16; L_FRAME], voiced_hangover: &mut i16) -> bool {
        let mut s = 0i32;
        for &v in speech.iter().rev() {
            let p = i32::from(v) * i32::from(v);
            s = l_add(s, if p != 0x4000_0000 { p << 1 } else { MAX_32 });
        }
        let curr = if s > 0x1fff_ffff { MAX_16 } else { (s >> 14) as i16 };
        let min = self.frame_energy_hist.iter().copied().fold(32767, i16::min);
        let t = i32::from(min) << 4;
        let noise_floor = t.clamp(-32768, 32767) as i16;
        let mut max = self.frame_energy_hist[0];
        for &v in &self.frame_energy_hist[1..=L_ENERGYHIST - 5] {
            if max < v {
                max = v;
            }
        }
        let mut max_last = self.frame_energy_hist[2 * L_ENERGYHIST / 3];
        for &v in &self.frame_energy_hist[2 * L_ENERGYHIST / 3 + 1..] {
            if max_last < v {
                max_last = v;
            }
        }
        if max > LOWERNOISELIMIT && curr < FRAMEENERGYLIMIT && curr > LOWERNOISELIMIT && (curr < noise_floor || max_last < UPPERNOISELIMIT) {
            self.bg_hangover = (self.bg_hangover + 1).min(30);
        } else {
            self.bg_hangover = 0;
        }
        let in_bg = self.bg_hangover > 1;
        self.frame_energy_hist.copy_within(1.., 0);
        self.frame_energy_hist[L_ENERGYHIST - 1] = curr;
        let ltp_limit = if self.bg_hangover > 15 {
            16383
        } else if self.bg_hangover > 8 {
            15565
        } else {
            13926
        };
        let mut prev_voiced = gmed_n(&ltp_gain_hist[4..9]) > ltp_limit;
        if self.bg_hangover > 20 {
            prev_voiced = gmed_n(ltp_gain_hist) > ltp_limit;
        }
        if prev_voiced {
            *voiced_hangover = 0;
        } else {
            *voiced_hangover = (voiced_hangover.wrapping_add(1)).min(10);
        }
        in_bg
    }
}
