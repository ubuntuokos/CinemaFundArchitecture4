// Ported from bcg729 1.1.2 src/decoder.c, decodeLSP.c, interpolateqLSP.c, qLSP2LP.c,
// decodeAdaptativeCodeVector.c, decodeFixedCodeVector.c, decodeGains.c, LPSynthesisFilter.c,
// postFilter.c, postProcessing.c, cng.c and utils.c, GPL-3.0-or-later,
// Copyright (c) 2011-2019 Belledonne Communications SARL.
// CFA4 changes GPL-3.0-or-later.

//! The G.729 / G.729 Annex B decoder (bcg729's `bcg729Decoder`).

use crate::math::*;
use crate::tables::*;

/// Samples per frame (10 ms at 8 kHz).
pub const FRAME_LENGTH: usize = 80;
const L_SUBFRAME: usize = 40;
const NB_LSP: usize = 10;
const MAX_PITCH: usize = 143;
const L_PAST_EXCITATION: usize = 154;
const BOUNDED_PITCH_GAIN_MIN: i16 = 3277;
const BOUNDED_PITCH_GAIN_MAX: i16 = 13107;
const CNG_SEED_INIT: u16 = 11111;
const GAP1: i16 = 10;
const GAP2: i16 = 5;
const QLSF_MIN: i16 = 40;
const QLSF_MAX: i16 = 25681;
const MIN_QLSF_DISTANCE: i32 = 321;
const GAMMA_N: [i32; 10] = [18022, 9912, 5452, 2998, 1649, 907, 499, 274, 151, 83];
const GAMMA_D: [i32; 10] = [22938, 16056, 11239, 7868, 5507, 3855, 2699, 1889, 1322, 926];
const GAMMA_T: i32 = 26214;

const PREVIOUS_QLSP_INIT: [i16; 10] = [30000, 26000, 21000, 15000, 8000, 0, -8000, -15000, -21000, -26000];
const PREVIOUS_L_CODE_WORD_INIT: [i16; 10] = [2339, 4679, 7018, 9358, 11698, 14037, 16377, 18717, 21056, 23396];
const SID_QLSP_INIT: [i16; 10] = [31441, 27566, 21458, 13612, 4663, -4663, -13612, -21458, -27566, -31441];

/// One frame of input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame<'a> {
    /// A 10-byte speech frame (80 bits).
    Speech(&'a [u8; 10]),
    /// A 2-byte Annex B SID frame (15 bits, the last bit unused).
    Sid(&'a [u8; 2]),
    /// A lost frame, or one the sender did not transmit during silence.
    Erased,
}

/// A G.729 decoder channel.
#[derive(Clone, Debug)]
pub struct Decoder {
    previous_qlsp: [i16; NB_LSP],
    excitation: [i16; L_PAST_EXCITATION + FRAME_LENGTH],
    bounded_pitch_gain: i16,
    adaptive_gain: i16,
    fixed_gain: i16,
    speech: [i16; NB_LSP + FRAME_LENGTH],
    seed: u16,
    cng_seed: u16,
    last_qlsf: [i16; NB_LSP],
    previous_l_code_word: [[i16; NB_LSP]; 4],
    last_valid_l0: usize,
    previous_int_pitch_delay: u16,
    previous_gain_prediction_error: [i16; 4],
    residual: [i16; MAX_PITCH + FRAME_LENGTH],
    scaled_residual: [i16; MAX_PITCH + FRAME_LENGTH],
    long_term: [i16; 1 + L_SUBFRAME],
    short_term: [i16; NB_LSP + L_SUBFRAME],
    previous_adaptive_gain: i16,
    x0: i16,
    x1: i16,
    y2: i32,
    y1: i32,
    cng: Cng,
    previous_active: bool,
}

#[derive(Clone, Debug)]
struct Cng {
    received_gain: i16,
    smoothed_gain: i16,
    qlsp: [i16; NB_LSP],
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    /// `initBcg729DecoderChannel`.
    pub fn new() -> Self {
        Self {
            previous_qlsp: PREVIOUS_QLSP_INIT,
            excitation: [0; L_PAST_EXCITATION + FRAME_LENGTH],
            bounded_pitch_gain: BOUNDED_PITCH_GAIN_MIN,
            adaptive_gain: 0,
            fixed_gain: 0,
            speech: [0; NB_LSP + FRAME_LENGTH],
            seed: 21845,
            cng_seed: CNG_SEED_INIT,
            last_qlsf: PREVIOUS_L_CODE_WORD_INIT,
            previous_l_code_word: [PREVIOUS_L_CODE_WORD_INIT; 4],
            last_valid_l0: 0,
            previous_int_pitch_delay: 60,
            previous_gain_prediction_error: [-14336; 4],
            residual: [0; MAX_PITCH + FRAME_LENGTH],
            scaled_residual: [0; MAX_PITCH + FRAME_LENGTH],
            long_term: [0; 1 + L_SUBFRAME],
            short_term: [0; NB_LSP + L_SUBFRAME],
            previous_adaptive_gain: 4096,
            x0: 0,
            x1: 0,
            y2: 0,
            y1: 0,
            cng: Cng { received_gain: 0, smoothed_gain: 0, qlsp: SID_QLSP_INIT },
            previous_active: true,
        }
    }

    /// `bcg729Decoder`: decodes one frame into 80 samples.
    pub fn decode(&mut self, frame: Frame<'_>) -> [i16; FRAME_LENGTH] {
        let mut out = [0i16; FRAME_LENGTH];
        let erased = matches!(frame, Frame::Erased);
        let sid = match frame {
            Frame::Sid(_) => true,
            Frame::Erased => !self.previous_active,
            Frame::Speech(_) => false,
        };
        let mut lp = [0i16; 2 * NB_LSP];
        if sid {
            let bits = if let Frame::Sid(b) = frame { Some(*b) } else { None };
            self.decode_sid(bits, &mut lp);
            self.previous_active = false;
            for (sub, chunk) in out.chunks_exact_mut(L_SUBFRAME).enumerate() {
                let s = sub * L_SUBFRAME;
                let coeffs = sub_lp(&lp, sub);
                lp_synthesis(&self.excitation, L_PAST_EXCITATION + s, &coeffs, &mut self.speech, NB_LSP + s);
                let delay = self.previous_int_pitch_delay as i16;
                let filtered = self.post_filter(&coeffs, NB_LSP + s, delay, s);
                chunk.copy_from_slice(&self.post_processing(filtered));
            }
            self.bounded_pitch_gain = BOUNDED_PITCH_GAIN_MIN;
            self.end_frame();
            return out;
        }
        self.previous_active = true;
        self.cng_seed = CNG_SEED_INIT;
        let mut p = [0u16; 15];
        if let Frame::Speech(b) = frame {
            p = parameters(b);
        }
        let qlsp = self.decode_lsp(&p, erased);
        let mut interpolated = [0i16; NB_LSP];
        for i in 0..NB_LSP {
            interpolated[i] = pshr(i32::from(self.previous_qlsp[i]) + i32::from(qlsp[i]), 1) as i16;
        }
        self.previous_qlsp = qlsp;
        lp[..NB_LSP].copy_from_slice(&qlsp_to_lp(&interpolated));
        lp[NB_LSP..].copy_from_slice(&qlsp_to_lp(&qlsp));
        let parity_error = (parity(p[4]) ^ p[5]) != 0;
        let mut pi = 4usize;
        let mut int_delay = 0i16;
        for (sub, chunk) in out.chunks_exact_mut(L_SUBFRAME).enumerate() {
            let s = sub * L_SUBFRAME;
            int_delay = self.decode_adaptive(s, p[pi], parity_error, erased, int_delay);
            pi += if sub == 0 { 2 } else { 1 };
            if erased {
                p[pi] = pseudo_random(&mut self.seed) & 0x1fff;
                p[pi + 1] = pseudo_random(&mut self.seed) & 0x000f;
            }
            let fixed = fixed_code_vector(p[pi + 1], p[pi], int_delay, self.bounded_pitch_gain);
            pi += 2;
            self.decode_gains(p[pi], p[pi + 1], &fixed, erased);
            pi += 2;
            self.bounded_pitch_gain = self.adaptive_gain.clamp(BOUNDED_PITCH_GAIN_MIN, BOUNDED_PITCH_GAIN_MAX);
            for i in 0..L_SUBFRAME {
                let e = &mut self.excitation[L_PAST_EXCITATION + s + i];
                let acc = mul(i32::from(*e), i32::from(self.adaptive_gain)).wrapping_add(mul(i32::from(fixed[i]), i32::from(self.fixed_gain)));
                *e = sat16(pshr(acc, 14));
            }
            let coeffs = sub_lp(&lp, sub);
            lp_synthesis(&self.excitation, L_PAST_EXCITATION + s, &coeffs, &mut self.speech, NB_LSP + s);
            let filtered = self.post_filter(&coeffs, NB_LSP + s, int_delay, s);
            chunk.copy_from_slice(&self.post_processing(filtered));
        }
        self.end_frame();
        out
    }

    fn end_frame(&mut self) {
        self.excitation.copy_within(FRAME_LENGTH.., 0);
        self.speech.copy_within(FRAME_LENGTH.., 0);
    }

    /// `decodeLSP`.
    fn decode_lsp(&mut self, l: &[u16; 15], erased: bool) -> [i16; NB_LSP] {
        let mut qlsf = [0i16; NB_LSP];
        if !erased {
            let l0 = usize::from(l[0] & 1);
            let l1 = &L1[usize::from(l[1] & 127)];
            let l2 = &L2L3[usize::from(l[2] & 31)];
            let l3 = &L2L3[usize::from(l[3] & 31)];
            for i in 0..NB_LSP / 2 {
                qlsf[i] = add16(i32::from(l1[i]), i32::from(l2[i]));
            }
            for i in NB_LSP / 2..NB_LSP {
                qlsf[i] = add16(i32::from(l1[i]), i32::from(l3[i]));
            }
            compute_qlsf(&mut qlsf, &mut self.previous_l_code_word, l0, &MA_PREDICTOR, &MA_PREDICTOR_SUM);
            self.last_qlsf = qlsf;
            self.last_valid_l0 = l0;
        } else {
            qlsf = self.last_qlsf;
            let l0 = self.last_valid_l0 & 1;
            for i in 0..NB_LSP {
                let mut acc = shl(i32::from(self.last_qlsf[i]), 15);
                for j in 0..4 {
                    acc = msu(acc, i32::from(MA_PREDICTOR[l0][j][i]), i32::from(self.previous_l_code_word[j][i]));
                }
                acc = mul32_q(i32::from(INV_MA_PREDICTOR_SUM[l0][i]), acc, 12);
                for j in (1..4).rev() {
                    self.previous_l_code_word[j][i] = self.previous_l_code_word[j - 1][i];
                }
                self.previous_l_code_word[0][i] = pshr(acc, 15) as i16;
            }
        }
        qlsf.map(cos_q13q15)
    }

    /// `decodeAdaptativeCodeVector`: returns the integer pitch delay.
    fn decode_adaptive(&mut self, sub: usize, index: u16, parity_error: bool, erased: bool, first_delay: i16) -> i16 {
        let index = i32::from(index);
        let (int_delay, frac) = if (sub == 0 && (parity_error || erased)) || (sub != 0 && erased) {
            let d = self.previous_int_pitch_delay as i16;
            self.previous_int_pitch_delay = (self.previous_int_pitch_delay + 1).min(MAX_PITCH as u16);
            (d, 0)
        } else if sub == 0 {
            let (d, f) = if index < 197 {
                let d = add16(mul_q(i32::from(add16(index, 2)), 10923, 15), 19);
                (d, add16(sub16(index, mul(i32::from(d), 3)), 58))
            } else {
                (sub16(index, 112) as i16, 0)
            };
            self.previous_int_pitch_delay = d as u16;
            (d, f)
        } else {
            let t_min = sub16(i32::from(first_delay), 5).clamp(20, 134);
            let d = sub16(mul_q(i32::from(add16(index, 2)), 10923, 15), 1);
            let f = sub16(sub16(index, mul(d, 3)), 2) as i16;
            let d = add16(d, t_min);
            self.previous_int_pitch_delay = d as u16;
            (d, f)
        };
        adaptive_codebook_vector(&mut self.excitation, L_PAST_EXCITATION + sub, frac, int_delay);
        int_delay
    }

    /// `decodeGains`.
    fn decode_gains(&mut self, ga: u16, gb: u16, fixed: &[i16; L_SUBFRAME], erased: bool) {
        if erased {
            self.adaptive_gain = if self.adaptive_gain < 16384 { mul_q(i32::from(self.adaptive_gain), 29491, 15) as i16 } else { 14746 };
            self.fixed_gain = mul_q(i32::from(self.fixed_gain), 32113, 15) as i16;
            let mut cur = 0i32;
            for e in self.previous_gain_prediction_error {
                cur = cur.wrapping_add(i32::from(e));
            }
            cur = pshr(cur, 2);
            cur = if cur < -10240 { -14336 } else { cur - 4096 };
            self.previous_gain_prediction_error.copy_within(0..3, 1);
            self.previous_gain_prediction_error[0] = cur as i16;
            return;
        }
        let ga = usize::from(REVERSE_INDEX_MAPPING_GA[usize::from(ga & 7)]) & 7;
        let gb = usize::from(REVERSE_INDEX_MAPPING_GB[usize::from(gb & 15)]) & 15;
        self.adaptive_gain = add16(i32::from(GA_CODEBOOK[ga][0]), i32::from(GB_CODEBOOK[gb][0]));
        let predicted = ma_code_gain_prediction(&self.previous_gain_prediction_error, fixed);
        let correction = add16(i32::from(GA_CODEBOOK[ga][1]), i32::from(GB_CODEBOOK[gb][1]));
        self.fixed_gain = pshr(mul32_q(i32::from(correction), predicted, 12), 15) as i16;
        compute_gain_prediction_error(correction, &mut self.previous_gain_prediction_error);
    }

    /// `postFilter`: the speech at `speech[at..at + 40]`, with ten samples of history before it.
    fn post_filter(&mut self, lp: &[i16; NB_LSP], at: usize, int_delay: i16, sub: usize) -> [i16; L_SUBFRAME] {
        let mut gn = [0i16; NB_LSP];
        let mut gd = [0i16; NB_LSP];
        for i in 0..NB_LSP {
            gn[i] = mul_p(i32::from(lp[i]), GAMMA_N[i], 15) as i16;
            gd[i] = mul_p(i32::from(lp[i]), GAMMA_D[i], 15) as i16;
        }
        // residual signal (eq79)
        let r0 = MAX_PITCH + sub;
        for i in 0..L_SUBFRAME {
            let mut acc = sshl(i32::from(self.speech[at + i]), 12);
            for j in 0..NB_LSP {
                acc = mac(acc, i32::from(gn[j]), i32::from(self.speech[at + i - j - 1]));
            }
            let r = sat16(pshr(acc, 12));
            self.residual[r0 + i] = r;
            self.scaled_residual[r0 + i] = pshr(i32::from(r), 2) as i16;
        }
        // long-term post filter (eq80-83)
        let int_delay = i32::from(int_delay).clamp(3, MAX_PITCH as i32 - 3);
        let mut correlation_max = i32::MIN;
        let mut best = 0usize;
        for d in (int_delay - 3)..=(int_delay + 3) {
            let d = d as usize;
            let mut c = 0i32;
            for j in 0..L_SUBFRAME {
                c = mac(c, i32::from(self.scaled_residual[r0 + j - d]), i32::from(self.scaled_residual[r0 + j]));
            }
            if c > correlation_max {
                correlation_max = c;
                best = d;
            }
        }
        correlation_max = correlation_max.max(0);
        let mut energy = 0i32;
        let mut delayed_energy = 0i32;
        for i in 0..L_SUBFRAME {
            let s = i32::from(self.scaled_residual[r0 + i]);
            let ds = i32::from(self.scaled_residual[r0 + i - best]);
            energy = mac(energy, s, s);
            delayed_energy = mac(delayed_energy, ds, ds);
        }
        let maximum = correlation_max.max(energy).max(delayed_energy);
        let (mut c16, mut e16, mut d16) = (0i32, 0i32, 0i32);
        if maximum > 0 {
            let lz = clz(maximum);
            if lz < 16 {
                c16 = i32::from(shr(correlation_max, 16 - lz) as i16);
                e16 = i32::from(shr(energy, 16 - lz) as i16);
                d16 = i32::from(shr(delayed_energy, 16 - lz) as i16);
            } else {
                c16 = i32::from(correlation_max as i16);
                e16 = i32::from(energy as i16);
                d16 = i32::from(delayed_energy as i16);
            }
        }
        if mul(c16, c16) < shr(mul(e16, d16), 1) || (c16 == 0 && d16 == 0) {
            for i in 0..L_SUBFRAME {
                self.long_term[1 + i] = self.residual[r0 + i];
            }
        } else {
            let (g0, g1) = if correlation_max > delayed_energy {
                (21845i32, 10923i32)
            } else {
                let g1 = i32::from(shl(c16, 15).checked_div(shl(d16, 1).wrapping_add(c16)).unwrap_or(0) as i16);
                (i32::from(sub16(32767, g1) as i16), g1)
            };
            for i in 0..L_SUBFRAME {
                let acc = mul(g0, i32::from(self.residual[r0 + i])).wrapping_add(mul(g1, i32::from(self.residual[r0 + i - best])));
                self.long_term[1 + i] = sat16(pshr(acc, 15));
            }
        }
        // tilt compensation (eq84-87)
        let mut hf = [0i16; 22];
        hf[0] = 4096;
        for i in 1..22 {
            let mut acc = if i < 11 { sshl(i32::from(gn[i - 1]), 12) } else { 0 };
            for j in 0..NB_LSP.min(i) {
                acc = msu(acc, i32::from(gd[j]), i32::from(hf[i - j - 1]));
            }
            hf[i] = sat16(pshr(acc, 12));
        }
        let mut rh1 = mul(i32::from(hf[0]), i32::from(hf[1]));
        for i in 1..21 {
            rh1 = mac(rh1, i32::from(hf[i]), i32::from(hf[i + 1]));
        }
        let mut tilted = [0i16; L_SUBFRAME];
        if rh1 < 0 {
            tilted.copy_from_slice(&self.long_term[1..]);
        } else {
            let mut rh0 = mul(i32::from(hf[0]), i32::from(hf[0]));
            for h in &hf[1..] {
                rh0 = mac(rh0, i32::from(*h), i32::from(*h));
            }
            let rh1 = mul32_q(GAMMA_T, rh1, 15);
            let gain = i32::from(sat16(rh1.checked_div(pshr(rh0, 12)).unwrap_or(0)));
            for i in 0..L_SUBFRAME {
                tilted[i] = i32::from(self.long_term[1 + i]).wrapping_sub(mul_q(gain, i32::from(self.long_term[i]), 12)) as i16;
            }
        }
        self.long_term[0] = self.long_term[L_SUBFRAME];
        // synthesis filter 1/A(z/γd)
        synthesis_filter(&tilted, &gd, &mut self.short_term);
        self.short_term.copy_within(L_SUBFRAME.., 0);
        let st = &self.short_term[NB_LSP..];
        // adaptive gain control (A.4.2.4)
        let mut out = [0i16; L_SUBFRAME];
        let mut st_sum = 0u32;
        for &v in st {
            st_sum = st_sum.wrapping_add((mul(i32::from(v), i32::from(v)) as u32) >> 4);
        }
        if st_sum == 0 {
            self.previous_adaptive_gain = 0;
            out.copy_from_slice(st);
        } else {
            let mut sp_sum = 0u32;
            for &v in &self.speech[at..at + L_SUBFRAME] {
                sp_sum = sp_sum.wrapping_add((mul(i32::from(v), i32::from(v)) as u32) >> 4);
            }
            let scale = if sp_sum == 0 {
                0
            } else {
                let shift = sp_sum.leading_zeros() as i32;
                let num = sp_sum.wrapping_shl(shift as u32);
                let scaled = uvshr(st_sum, 10 - shift) as i32;
                let fraction = if scaled == 0 { uvshr(num / st_sum, shift - 10) } else { num / (scaled as u32) };
                let g = i32::from(sat16(sqrt_q0q7(fraction)));
                mul_p(g, 3277, 15) as i16
            };
            let mut current = self.previous_adaptive_gain;
            for i in 0..L_SUBFRAME {
                current = add16(i32::from(scale), mul_p(i32::from(current), 29491, 15));
                out[i] = mul_q(i32::from(current), i32::from(st[i]), 12) as i16;
            }
            self.previous_adaptive_gain = current;
        }
        if sub > 0 {
            self.residual.copy_within(FRAME_LENGTH.., 0);
            self.scaled_residual.copy_within(FRAME_LENGTH.., 0);
        }
        out
    }

    /// `postProcessing`: the high-pass filter and ×2 upscaling.
    fn post_processing(&mut self, mut signal: [i16; L_SUBFRAME]) -> [i16; L_SUBFRAME] {
        const A1: i32 = 15836;
        const A2: i32 = -7667;
        const B0: i32 = 7699;
        const B1: i32 = -15398;
        const B2: i32 = 7699;
        for s in &mut signal {
            let x2 = self.x1;
            self.x1 = self.x0;
            self.x0 = *s;
            let mut acc = mul32_q(A1, self.y1, 13);
            acc = acc.wrapping_add(mul32_q(A2, self.y2, 13));
            acc = mac(acc, i32::from(self.x0), B0);
            acc = mac(acc, i32::from(self.x1), B1);
            acc = sat(mac(acc, i32::from(x2), B2), MAXINT29);
            *s = sat16(pshr(acc, 12));
            self.y2 = self.y1;
            self.y1 = acc;
        }
        signal
    }

    /// `decodeSIDframe` (the two-byte G.729 Annex B payload; `None` for a frame not sent).
    fn decode_sid(&mut self, bits: Option<[u8; 2]>, lp: &mut [i16; 2 * NB_LSP]) {
        if let Some(b) = bits {
            let l0 = usize::from(b[0] >> 7);
            let l1 = usize::from((b[0] >> 2) & 0x1f);
            let l2 = usize::from(((b[0] & 3) << 2) | (b[1] >> 6));
            self.cng.received_gain = SID_GAIN_CODEBOOK[usize::from((b[1] >> 1) & 0x1f)];
            let c1 = &L1[usize::from(L1_SUBSET_INDEX[l1]) & 127];
            let c2 = &L2L3[usize::from(L2_SUBSET_INDEX[l2]) & 31];
            let c3 = &L2L3[usize::from(L3_SUBSET_INDEX[l2]) & 31];
            let mut qlsf = [0i16; NB_LSP];
            for i in 0..NB_LSP / 2 {
                qlsf[i] = add16(i32::from(c1[i]), i32::from(c2[i]));
            }
            for i in NB_LSP / 2..NB_LSP {
                qlsf[i] = add16(i32::from(c1[i]), i32::from(c3[i]));
            }
            compute_qlsf(&mut qlsf, &mut self.previous_l_code_word, l0, &NOISE_MA_PREDICTOR, &NOISE_MA_PREDICTOR_SUM);
            self.cng.qlsp = qlsf.map(cos_q13q15);
        }
        let mut interpolated = [0i16; NB_LSP];
        for i in 0..NB_LSP {
            interpolated[i] = pshr(i32::from(self.previous_qlsp[i]) + i32::from(self.cng.qlsp[i]), 1) as i16;
        }
        self.previous_qlsp = self.cng.qlsp;
        lp[..NB_LSP].copy_from_slice(&qlsp_to_lp(&interpolated));
        lp[NB_LSP..].copy_from_slice(&qlsp_to_lp(&self.cng.qlsp));
        if self.previous_active {
            self.cng.smoothed_gain = self.cng.received_gain;
        } else {
            let s = self.cng.smoothed_gain;
            let s = sub16(i32::from(s), i32::from(s >> 3)) as i16;
            self.cng.smoothed_gain = add16(i32::from(s), i32::from(self.cng.received_gain >> 3));
        }
        comfort_noise_excitation(self.cng.smoothed_gain, &mut self.cng_seed, &mut self.excitation);
    }
}

fn sub_lp(lp: &[i16; 2 * NB_LSP], sub: usize) -> [i16; NB_LSP] {
    let mut c = [0i16; NB_LSP];
    c.copy_from_slice(&lp[sub * NB_LSP..(sub + 1) * NB_LSP]);
    c
}

/// `parametersBitStream2Array`.
fn parameters(b: &[u8; 10]) -> [u16; 15] {
    let b = b.map(u16::from);
    [
        (b[0] >> 7) & 1,
        b[0] & 0x7f,
        (b[1] >> 3) & 0x1f,
        ((b[1] & 7) << 2) | ((b[2] >> 6) & 3),
        ((b[2] & 0x3f) << 2) | ((b[3] >> 6) & 3),
        (b[3] >> 5) & 1,
        ((b[3] & 0x1f) << 8) | b[4],
        (b[5] >> 4) & 0xf,
        (b[5] >> 1) & 7,
        ((b[5] & 1) << 3) | ((b[6] >> 5) & 7),
        b[6] & 0x1f,
        (b[7] << 5) | ((b[8] >> 3) & 0x1f),
        ((b[8] & 7) << 1) | ((b[9] >> 7) & 1),
        (b[9] >> 4) & 7,
        b[9] & 0xf,
    ]
}

/// `computeParity`: the parity of the six most significant bits of P1.
fn parity(index: u16) -> u16 {
    let mut p = 1u16;
    let mut i = index >> 2;
    for _ in 0..6 {
        p ^= i & 1;
        i >>= 1;
    }
    p
}

/// `pseudoRandom` (spec 4.4.4 eq96).
fn pseudo_random(seed: &mut u16) -> u16 {
    *seed = 13849u16.wrapping_add(seed.wrapping_mul(31821));
    *seed
}

/// `rearrangeCoefficients`.
fn rearrange(q: &mut [i16; NB_LSP], gap: i16) {
    for i in 1..NB_LSP {
        let delta = (i32::from(add16(sub16(i32::from(q[i - 1]), i32::from(q[i])), i32::from(gap))) / 2) as i16;
        if delta > 0 {
            q[i - 1] = sub16(i32::from(q[i - 1]), i32::from(delta)) as i16;
            q[i] = add16(i32::from(q[i]), i32::from(delta));
        }
    }
}

/// `computeqLSF`.
fn compute_qlsf(q: &mut [i16; NB_LSP], previous: &mut [[i16; NB_LSP]; 4], l0: usize, predictor: &[[[i16; NB_LSP]; 4]; 2], sum: &[[i16; NB_LSP]; 2]) {
    let l0 = l0 & 1;
    rearrange(q, GAP1);
    rearrange(q, GAP2);
    for i in 0..NB_LSP {
        let mut acc = mul(i32::from(sum[l0][i]), i32::from(q[i]));
        for j in (0..4).rev() {
            acc = mac(acc, i32::from(predictor[l0][j][i]), i32::from(previous[j][i]));
            previous[j][i] = if j > 0 { previous[j - 1][i] } else { q[i] };
        }
        q[i] = pshr(acc, 15) as i16;
    }
    // insertion sort, as bcg729
    for i in 1..NB_LSP {
        let v = q[i];
        let mut j = i;
        while j > 0 && q[j - 1] > v {
            q[j] = q[j - 1];
            j -= 1;
        }
        q[j] = v;
    }
    if q[0] < QLSF_MIN {
        q[0] = QLSF_MIN;
    }
    for i in 0..NB_LSP - 1 {
        if sub16(i32::from(q[i + 1]), i32::from(q[i])) < MIN_QLSF_DISTANCE {
            q[i + 1] = (i32::from(q[i]) + MIN_QLSF_DISTANCE) as i16;
        }
    }
    if q[NB_LSP - 1] > QLSF_MAX {
        q[NB_LSP - 1] = QLSF_MAX;
    }
}

/// `computePolynomialCoefficients`.
fn polynomial(q: &[i16], f: &mut [i32; 6]) {
    let qv = |k: usize| i32::from(q.get(k).copied().unwrap_or(0));
    f[0] = 16_777_216;
    f[1] = mul(qv(0), -1024);
    for i in 2..6 {
        let c = qv(2 * i - 2);
        f[i] = sshl(f[i - 2].wrapping_sub(mul32_p(c, f[i - 1], 15)), 1);
        for j in (2..i).rev() {
            f[j] = f[j].wrapping_add(f[j - 2].wrapping_sub(mul32_p(c, f[j - 1], 14)));
        }
        f[1] = f[1].wrapping_sub(sshl(c, 10));
    }
}

/// `qLSP2LP`: ten LSPs (Q15) to ten LP coefficients (Q12).
fn qlsp_to_lp(q: &[i16; NB_LSP]) -> [i16; NB_LSP] {
    let mut f1 = [0i32; 6];
    let mut f2 = [0i32; 6];
    polynomial(q, &mut f1);
    polynomial(&q[1..], &mut f2);
    for i in (1..6).rev() {
        f1[i] = f1[i].wrapping_add(f1[i - 1]);
        f2[i] = f2[i].wrapping_sub(f2[i - 1]);
    }
    let mut lp = [0i16; NB_LSP];
    for i in 0..5 {
        lp[i] = pshr(f1[i + 1].wrapping_add(f2[i + 1]), 13) as i16;
        lp[9 - i] = pshr(f1[i + 1].wrapping_sub(f2[i + 1]), 13) as i16;
    }
    lp
}

/// `computeAdaptativeCodebookVector`: writes `exc[at..at + 40]` from the past excitation.
fn adaptive_codebook_vector(exc: &mut [i16; L_PAST_EXCITATION + FRAME_LENGTH], at: usize, frac: i16, int_delay: i16) {
    // the decoder only produces delays in 19..=144 and fractions in -1..=1
    let int_delay = usize::from(int_delay.clamp(19, 144) as u16);
    let (start, t) = if frac == 1 { (at - (int_delay + 1), 2usize) } else { (at - int_delay, usize::from((-frac.clamp(-1, 1)) as u16)) };
    for n in 0..L_SUBFRAME {
        let mut acc = 0i32;
        for i in 0..10 {
            let j = 3 * i;
            acc = mac(acc, i32::from(exc[start + n - i]), i32::from(B30[t + j]));
            acc = mac(acc, i32::from(exc[start + n + 1 + i]), i32::from(B30[3 - t + j]));
        }
        exc[at + n] = sat16(pshr(acc, 15));
    }
}

/// `decodeFixedCodeVector`.
fn fixed_code_vector(signs: u16, positions: u16, int_delay: i16, beta: i16) -> [i16; L_SUBFRAME] {
    let mut p = positions;
    let mut pos = [0usize; 4];
    pos[0] = usize::from(p & 7) * 5;
    p >>= 3;
    pos[1] = usize::from(p & 7) * 5 + 1;
    p >>= 3;
    pos[2] = usize::from(p & 7) * 5 + 2;
    p >>= 3;
    let jx = usize::from(p & 1);
    p >>= 1;
    pos[3] = usize::from(p & 7) * 5 + 3 + jx;
    let mut v = [0i16; L_SUBFRAME];
    let mut s = signs;
    for &q in &pos {
        v[q] = if s & 1 != 0 { 8192 } else { -8192 };
        s >>= 1;
    }
    let d = usize::from(int_delay.max(1) as u16);
    for i in d..L_SUBFRAME {
        v[i] = add16(i32::from(v[i]), mul_p(i32::from(v[i - d]), i32::from(beta), 14));
    }
    v
}

/// `MACodeGainPrediction`: the predicted fixed codebook gain in Q16.
fn ma_code_gain_prediction(previous: &[i16; 4], fixed: &[i16; L_SUBFRAME]) -> i32 {
    let mut sum = 0i32;
    for &v in fixed {
        if v != 0 {
            sum = mac(sum, i32::from(v), i32::from(v));
        }
    }
    let mut acc = 8_145_364i32.wrapping_add(mul32_q(-24660, log2_q0q16(sum), 13));
    acc = shl(acc, 8);
    for i in 0..4 {
        acc = mac(acc, i32::from(previous[i]), i32::from(MA_PREDICTION_COEFFICIENTS[i]));
    }
    acc = shr(acc, 2);
    acc = mul32_q(5442, acc, 15);
    acc = pshr(acc, 11);
    exp2_q11q16(acc as i16)
}

/// `computeGainPredictionError`.
fn compute_gain_prediction_error(correction: i16, previous: &mut [i16; 4]) {
    let cur = log2_q0q16(i32::from(correction)).wrapping_sub(786_432);
    let cur = pshr(mul32_q(24660, cur, 12), 6);
    previous.copy_within(0..3, 1);
    previous[0] = cur as i16;
}

/// `LPSynthesisFilter`: `speech[out..out + 40]` from `exc[at..at + 40]`.
fn lp_synthesis(exc: &[i16; L_PAST_EXCITATION + FRAME_LENGTH], at: usize, lp: &[i16; NB_LSP], speech: &mut [i16; NB_LSP + FRAME_LENGTH], out: usize) {
    for i in 0..L_SUBFRAME {
        let mut acc = sshl(i32::from(exc[at + i]), 12);
        for j in 0..NB_LSP {
            acc = msu(acc, i32::from(lp[j]), i32::from(speech[out + i - j - 1]));
        }
        speech[out + i] = sat16(pshr(acc, 12));
    }
}

/// `synthesisFilter`: `filtered[10..50]` from `input`, `filtered[0..10]` being the memory.
fn synthesis_filter(input: &[i16; L_SUBFRAME], lp: &[i16; NB_LSP], filtered: &mut [i16; NB_LSP + L_SUBFRAME]) {
    for i in 0..L_SUBFRAME {
        let mut acc = sshl(i32::from(input[i]), 12);
        for j in 0..NB_LSP {
            acc = msu(acc, i32::from(lp[j]), i32::from(filtered[NB_LSP + i - j - 1]));
        }
        filtered[NB_LSP + i] = sat16(pshr(acc, 12));
    }
}

/// `computeComfortNoiseExcitationVector` (spec B.4.4, B.4.5) on `exc[154..234]`.
fn comfort_noise_excitation(target_gain: i16, seed: &mut u16, exc: &mut [i16; L_PAST_EXCITATION + FRAME_LENGTH]) {
    let target = i32::from(target_gain);
    for s in (0..FRAME_LENGTH).step_by(L_SUBFRAME) {
        let base = L_PAST_EXCITATION + s;
        let mut r = pseudo_random(seed);
        let mut frac = (r & 3) as i16 - 1;
        if frac == 2 {
            frac = 0;
        }
        r >>= 2;
        let int_delay = (r & 0x3f) as i16 + 40;
        r >>= 6;
        let mut position = [0usize; 4];
        let mut sign = [0u16; 4];
        position[0] = usize::from(r & 7) * 5;
        r >>= 3;
        sign[0] = r & 1;
        r >>= 1;
        position[1] = usize::from(r & 7) * 5 + 1;
        r >>= 3;
        sign[1] = r & 1;
        r = pseudo_random(seed);
        position[2] = usize::from(r & 7) * 5 + 2;
        r >>= 3;
        sign[2] = r & 1;
        r >>= 1;
        position[3] = usize::from(r & 1) + 3;
        r >>= 1;
        position[3] += usize::from(r & 7) * 5;
        r >>= 3;
        sign[3] = r & 1;
        let ga = i32::from(((pseudo_random(seed) & 0x1fff) << 1) as i16);
        let mut gaussian = [0i16; L_SUBFRAME];
        let mut eg = 0i32;
        for g in &mut gaussian {
            let mut t = 0i32;
            for _ in 0..12 {
                t = t.wrapping_add(i32::from(pseudo_random(seed) as i16));
            }
            *g = shr(t, 7) as i16;
            eg = mac(eg, i32::from(*g), i32::from(*g));
        }
        let gg = mul32_q(25905, inv_sqrt_q0q31(eg), 15);
        let gg = mul32_q(target, gg, 15);
        for g in &mut gaussian {
            let v = i32::from(*g);
            *g = if v < 0 { (-sat(pshr(mul32_q(-v, gg, 15), 2), MAXINT16)) as i16 } else { pshr(mul32_q(v, gg, 15), 2) as i16 };
        }
        adaptive_codebook_vector(exc, base, frac, int_delay);
        for i in 0..L_SUBFRAME {
            exc[base + i] = sat16(mul_p(i32::from(exc[base + i]), ga, 15));
        }
        for i in 0..L_SUBFRAME {
            exc[base + i] = sat16(i32::from(exc[base + i]) + i32::from(gaussian[i]));
        }
        let mut ea = 0i32;
        for i in 0..L_SUBFRAME {
            ea = mac(ea, i32::from(exc[base + i]), i32::from(exc[base + i]));
        }
        let pulses = |exc: &[i16; L_PAST_EXCITATION + FRAME_LENGTH], mut ei: i32| {
            for i in 0..4 {
                let v = i32::from(exc[position[i] + base]);
                ei = if sign[i] == 0 { ei.wrapping_sub(v) } else { ei.wrapping_add(v) };
            }
            ei
        };
        let mut ei = pulses(exc, 0);
        let k = mul(target, shr(mul(L_SUBFRAME as i32, target), 3));
        let mut delta = i64::from(ei) * i64::from(ei) + ((i64::from(k) - i64::from((i64::from(ea) << 3) as i32)) >> 1);
        if delta < 0 {
            exc[base..base + L_SUBFRAME].copy_from_slice(&gaussian);
            // bcg729 (after the ITU code) adds to the previous sum here
            ei = pulses(exc, ei);
            delta = i64::from(ei) * i64::from(ei) + i64::from(mul32_p(24576, k, 15));
        }
        let mut scale = 0i32;
        while delta >= 0x8000_0000 {
            delta >>= 1;
            scale += 1;
        }
        if scale % 2 == 1 {
            delta >>= 1;
            scale += 1;
        }
        let root = sqrt_q0q7(delta as i32 as u32);
        let ei = svshr(ei, scale / 2 - 7);
        let mut gf = root.wrapping_sub(ei);
        let x2 = root.wrapping_add(ei).wrapping_neg();
        if i64::from(x2).abs() < i64::from(gf).abs() {
            gf = x2;
        }
        let gf = vshr(gf, 2 + 7 - scale / 2);
        for i in 0..4 {
            let e = &mut exc[position[i] + base];
            *e = if sign[i] == 0 { i32::from(*e).wrapping_sub(gf) } else { i32::from(*e).wrapping_add(gf) } as i16;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameters_unpack_in_table_8_order() {
        let p = parameters(&[0xff; 10]);
        assert_eq!(p, [1, 127, 31, 31, 255, 1, 8191, 15, 7, 15, 31, 8191, 15, 7, 15]);
        assert_eq!(parameters(&[0; 10]), [0; 15]);
    }

    #[test]
    fn parity_covers_the_six_high_bits() {
        assert_eq!(parity(0), 1);
        assert_eq!(parity(0b0000_0111), 0b1 ^ 0b1);
        assert_eq!(parity(0b0000_0100), 0);
        assert_eq!(parity(0b1111_1100), 1);
    }

    #[test]
    fn any_frame_sequence_decodes() {
        let mut d = Decoder::new();
        let mut x = 0x1234_5678u32;
        for n in 0..2000 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let b = x.to_le_bytes();
            let speech = [b[0], b[1], b[2], b[3], b[0] ^ 0x55, b[1], b[2] ^ 0xaa, b[3], b[0], b[1]];
            let frame = match n % 7 {
                3 => Frame::Erased,
                5 => Frame::Sid(&[b[2], b[3]]),
                _ => Frame::Speech(&speech),
            };
            let _ = d.decode(frame);
        }
        for f in [[0u8; 10], [0xff; 10]] {
            for _ in 0..50 {
                let _ = d.decode(Frame::Speech(&f));
            }
        }
    }
}
