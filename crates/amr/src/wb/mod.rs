// Derived from opencore-amr 0.1.6 (commit 7dba8c32), amr_wb/dec/src: pvamrwbdecoder.cpp,
// synthesis_amr_wb.cpp, mime_io.cpp, homing_amr_wb_dec.cpp, and amrwb/wrapper.cpp (D_IF_decode).
// Copyright (C) 1998-2009 PacketVideo, Apache License 2.0; portions (C) 2007 3GPP
// Organizational Partners (3GPP TS 26.173); wrapper (C) 2009 Martin Storsjo.
// Ported to Rust for CFA4 (GPL-3.0-or-later).

//! AMR wideband (3GPP TS 26.171, 16 kHz), bit-exact with opencore-amrwb and its `D_IF`
//! interface: decoder homing, comfort noise, frame-loss concealment and the 14-bit output.

mod codebook;
mod dsp;
mod dtx;
mod op;
mod tables;

use crate::Error;
use codebook::{GainIn, IsfState, L_MEANBUF, dec_acelp_2p, dec_acelp_4p, dec_gain2, dpisf_2s_36b, dpisf_2s_46b, gain_mem_init, lagconceal};
use dsp::{
    agc2, band_pass_6k_7k, deemphasis_32, highpass_50hz, highpass_400hz, interpolate_isp, isf_extrapolation, isf_isp, isp_az, low_pass_7k, oversample,
    phase_dispersion, pit_shrp, pred_lt4, preemph, scale_signal, syn_filt_32, voice_factor, wb_syn_filt, weight_lpc,
};
use dtx::{DTX, DTX_MUTE, DtxDec, Rx, SPEECH};
use op::{
    BIT_1, Bits, MAX_16, add, at, div_16by16, dot_product12, l_add, l_mac, l_msu, l_mult, l_shl, mul32_16b, mult, noise, norm, norm_s, one_ov_sqrt_norm, round,
    shl, sub,
};
use tables::*;

const L_FRAME: usize = 256;
const L_SUBFR: usize = 64;
const L_SUBFR16K: usize = 80;
const M: usize = 16;
const M16K: usize = 20;
/// `PIT_MAX + L_INTERPOL`: the excitation history the pitch predictor reads.
const EXC_HIST: usize = 231 + 17;
const PIT_MIN: i16 = 34;
const PIT_MAX: i16 = 231;
const Q_MAX: i16 = 8;
const PIT_SHARP: i16 = 27853;
const PREEMPH_FAC: i16 = 22282;

const NBBITS_7K: i16 = 132;
const NBBITS_9K: i16 = 177;
const NBBITS_12K: i16 = 253;
const NBBITS_14K: i16 = 285;
const NBBITS_16K: i16 = 317;
const NBBITS_18K: i16 = 365;
const NBBITS_20K: i16 = 397;
const NBBITS_24K: i16 = 477;
const NB_BITS: [i16; 16] = [132, 177, 253, 285, 317, 365, 397, 461, 477, 35, 0, 0, 0, 0, 0, 0];
const MAX_BITS: usize = 477;

/// Samples per frame (20 ms at 16 kHz).
pub const FRAME_SAMPLES: usize = 320;
/// Output sample rate.
pub const SAMPLE_RATE: u32 = 16_000;
/// Payload bytes of each frame type (after the table-of-contents byte), RFC 4867 §5.3.
pub const PAYLOAD_BYTES: [usize; 16] = [17, 23, 32, 36, 40, 46, 50, 58, 60, 5, 0, 0, 0, 0, 0, 0];

const INTERPOL_FRAC: [i16; 4] = [14746, 26214, 31457, 32767];
const ISP_INIT: [i16; M] = [32138, 30274, 27246, 23170, 18205, 12540, 6393, 0, -6393, -12540, -18205, -23170, -27246, -30274, -32138, 1475];
const ISF_INIT: [i16; M] = [1024, 2048, 3072, 4096, 5120, 6144, 7168, 8192, 9216, 10240, 11264, 12288, 13312, 14336, 15360, 3840];
const HP_GAIN: [i16; 16] = [3624, 4673, 5597, 6479, 7425, 8378, 9324, 10264, 11210, 12206, 13391, 14844, 16770, 19655, 24289, 32728];

/// `Decoder_State`.
#[derive(Clone)]
struct State {
    old_exc: [i16; EXC_HIST],
    ispold: [i16; M],
    isfold: [i16; M],
    isf_buf: [[i16; M]; L_MEANBUF],
    past_isfq: [i16; M],
    tilt_code: i16,
    q_old: i16,
    qsubfr: [i16; 4],
    l_gc_thres: i32,
    mem_syn_hi: [i16; M],
    mem_syn_lo: [i16; M],
    mem_deemph: i16,
    mem_sig_out: [i16; 6],
    mem_oversamp: [i16; 24],
    mem_syn_hf: [i16; M16K],
    mem_hf: [i16; 30],
    mem_hf3: [i16; 30],
    seed: i16,
    seed2: i16,
    old_t0: i16,
    lag_hist: [i16; 5],
    dec_gain: codebook::GainMem,
    seed3: i16,
    disp_mem: [i16; 8],
    mem_hp400: [i16; 6],
    prev_bfi: bool,
    state: i16,
    first_frame: bool,
    dtx: DtxDec,
    vad_hist: i16,
}

/// What `synthesis_amr_wb` needs besides the state.
struct SynthIn<'a> {
    aq: &'a [i16; M + 1],
    q_new: i16,
    corr_gain: i16,
    nb_bits: i16,
    speech: bool,
    bfi: bool,
}

impl State {
    fn new() -> Self {
        let mut s = State {
            old_exc: [0; EXC_HIST],
            ispold: ISP_INIT,
            isfold: ISF_INIT,
            isf_buf: [ISF_INIT; L_MEANBUF],
            past_isfq: [0; M],
            tilt_code: 0,
            q_old: Q_MAX,
            qsubfr: [Q_MAX; 4],
            l_gc_thres: 0,
            mem_syn_hi: [0; M],
            mem_syn_lo: [0; M],
            mem_deemph: 0,
            mem_sig_out: [0; 6],
            mem_oversamp: [0; 24],
            mem_syn_hf: [0; M16K],
            mem_hf: [0; 30],
            mem_hf3: [0; 30],
            seed: 21845,
            seed2: 21845,
            old_t0: 64,
            lag_hist: [64; 5],
            dec_gain: gain_mem_init(),
            seed3: 21845,
            disp_mem: [0; 8],
            mem_hp400: [0; 6],
            prev_bfi: false,
            state: 0,
            first_frame: true,
            dtx: DtxDec::new(&ISF_INIT),
            vad_hist: 0,
        };
        s.reset(true);
        s
    }

    /// `pvDecoder_AmrWb_Reset`.
    fn reset(&mut self, all: bool) {
        self.old_exc = [0; EXC_HIST];
        self.past_isfq = [0; M];
        self.old_t0 = 64;
        self.first_frame = true;
        self.l_gc_thres = 0;
        self.tilt_code = 0;
        self.disp_mem = [0; 8];
        self.q_old = Q_MAX;
        self.qsubfr = [Q_MAX; 4];
        if all {
            self.dec_gain = gain_mem_init();
            self.mem_oversamp = [0; 24];
            self.mem_hf = [0; 30];
            self.mem_hf3 = [0; 30];
            self.mem_sig_out = [0; 6];
            self.mem_hp400 = [0; 6];
            self.lag_hist = [64; 5];
            self.ispold = ISP_INIT;
            self.isfold = ISF_INIT;
            self.isf_buf = [ISF_INIT; L_MEANBUF];
            self.mem_deemph = 0;
            self.seed = 21845;
            self.seed2 = 21845;
            self.seed3 = 21845;
            self.state = 0;
            self.prev_bfi = false;
            self.mem_syn_hf = [0; M16K];
            self.mem_syn_hi = [0; M];
            self.mem_syn_lo = [0; M];
            self.dtx = DtxDec::new(&ISF_INIT);
            self.vad_hist = 0;
        }
    }

    /// `pvDecoder_AmrWb`: one 20 ms frame from the sorted parameter bits.
    fn decode(&mut self, mode: usize, prms: &[i16], synth16k: &mut [i16; FRAME_SAMPLES], ft: Rx) {
        let nb_bits = NB_BITS.get(mode).copied().unwrap_or(0);
        let mut bits = Bits { bits: prms, pos: 0 };
        let new_dtx = self.dtx.rx_handler(ft);
        let mut exc2 = [0i16; L_FRAME];
        let mut isf = [0i16; M];
        if new_dtx != SPEECH {
            self.dtx.decode(&mut exc2, new_dtx, &mut isf, &mut bits);
        }
        let (bfi, unusable) = if matches!(ft, Rx::NoData | Rx::SpeechLost) { (true, true) } else { (false, false) };
        if bfi {
            self.state = (self.state + 1).min(6);
        } else {
            self.state >>= 1;
        }
        if self.dtx.global_state == DTX {
            self.state = 5;
            self.prev_bfi = false;
        } else if self.dtx.global_state == DTX_MUTE {
            self.state = 5;
            self.prev_bfi = true;
        }
        if new_dtx == SPEECH {
            let vad_flag = bits.bit1();
            if !bfi {
                self.vad_hist = if vad_flag == 0 { add(self.vad_hist, 1) } else { 0 };
            }
        }
        let mut hf_isf = [0i16; M16K];
        if new_dtx != SPEECH {
            let mut ispnew = isf;
            isf_isp(&mut ispnew);
            let mut aq = [0i16; M + 1];
            isp_az(&ispnew, &mut aq, M, true);
            let isf_tmp = self.isfold;
            for (sub_i, (exc, out)) in exc2.chunks_exact_mut(L_SUBFR).zip(synth16k.chunks_exact_mut(L_SUBFR16K)).enumerate() {
                interp_isf(&isf_tmp, &isf, INTERPOL_FRAC[sub_i], &mut hf_isf);
                let si = SynthIn { aq: &aq, q_new: 0, corr_gain: 1, nb_bits, speech: false, bfi };
                self.synthesis(&si, exc, out, &mut hf_isf);
            }
            self.reset(false);
            self.isfold = isf;
            self.prev_bfi = bfi;
            self.dtx.global_state = new_dtx;
            return;
        }

        let mut exc_buf = [0i16; EXC_HIST + L_FRAME + 1];
        exc_buf[..EXC_HIST].copy_from_slice(&self.old_exc);
        let mut ind = [0i16; 8];
        {
            let mut st = IsfState { past_isfq: &mut self.past_isfq, isfold: &self.isfold, isf_buf: &mut self.isf_buf };
            if nb_bits > NBBITS_7K {
                for (k, n) in [8, 8, 6, 7, 7, 5, 5].into_iter().enumerate() {
                    ind[k] = bits.parm(n);
                }
                dpisf_2s_46b(&ind, &mut isf, &mut st, bfi);
            } else {
                ind[0] = bits.parm(8);
                ind[1] = bits.parm(8);
                let t = bits.parm(14);
                ind[3] = t & 0x7f;
                ind[2] = t >> 7;
                ind[4] = bits.parm(6);
                dpisf_2s_36b(&ind, &mut isf, &mut st, bfi);
            }
        }
        let mut ispnew = isf;
        isf_isp(&mut ispnew);
        if self.first_frame {
            self.first_frame = false;
            self.ispold = ispnew;
        }
        let mut aq = [[0i16; M + 1]; 4];
        interpolate_isp(&self.ispold, &ispnew, &INTERPOL_FRAC, &mut aq);
        self.ispold = ispnew;
        let mut l: i32 = 0;
        for i in 0..M - 1 {
            let t = sub(isf[i], self.isfold[i]);
            l = l_mac(l, t, t);
        }
        let tmp = mult((l_shl(l, 8) >> 16) as i16, 26214);
        let stab_fac = shl(20480i16.wrapping_sub(tmp), 1).max(0);
        let isf_tmp = self.isfold;
        self.isfold = isf;

        let mut t0_min: i16 = 0;
        let mut q_new: i16 = 0;
        let mut code = [0i16; L_SUBFR];
        let mut excp = [0i16; L_SUBFR];
        for (sub_i, aq_sub) in aq.iter().enumerate() {
            let i_subfr = sub_i * L_SUBFR;
            let pos = EXC_HIST + i_subfr;
            let first = i_subfr == 0 || (i_subfr == 2 * L_SUBFR && nb_bits > NBBITS_7K);
            let (mut t0, mut t0_frac);
            if first {
                if nb_bits <= NBBITS_9K {
                    let index = bits.parm(8);
                    if index < (92 - PIT_MIN) * 2 {
                        t0 = PIT_MIN + (index >> 1);
                        t0_frac = shl(sub(index, shl(sub(t0, PIT_MIN), 1)), 1);
                    } else {
                        t0 = add(index, 92 - (92 - PIT_MIN) * 2);
                        t0_frac = 0;
                    }
                } else {
                    let mut index = bits.parm(9);
                    if index < (128 - PIT_MIN) * 4 {
                        t0 = PIT_MIN + (index >> 2);
                        t0_frac = sub(index, shl(sub(t0, PIT_MIN), 2));
                    } else if index < ((128 - PIT_MIN) << 2) + ((160 - 128) << 1) {
                        index -= (128 - PIT_MIN) << 2;
                        t0 = 128 + (index >> 1);
                        t0_frac = shl(sub(index, shl(sub(t0, 128), 1)), 1);
                    } else {
                        t0 = add(index, 160 - (128 - PIT_MIN) * 4 - (160 - 128) * 2);
                        t0_frac = 0;
                    }
                }
                t0_min = (t0 - 8).max(PIT_MIN);
                if t0_min + 15 > PIT_MAX {
                    t0_min = PIT_MAX - 15;
                }
            } else if nb_bits <= NBBITS_9K {
                let index = bits.parm(5);
                t0 = t0_min.wrapping_add(index >> 1);
                t0_frac = shl(sub(index, shl(t0.wrapping_sub(t0_min), 1)), 1);
            } else {
                let index = bits.parm(6);
                t0 = t0_min.wrapping_add(index >> 2);
                t0_frac = sub(index, shl(t0.wrapping_sub(t0_min), 2));
            }
            if bfi {
                let gh = self.dec_gain.get(17..22).unwrap_or(&[]);
                lagconceal(gh, &self.lag_hist, &mut t0, self.old_t0, &mut self.seed3, unusable);
                t0_frac = 0;
            }
            pred_lt4(&mut exc_buf, pos, t0, t0_frac, L_SUBFR + 1);
            let select = if unusable {
                1
            } else if nb_bits <= NBBITS_9K {
                0
            } else {
                bits.bit1()
            };
            if select == 0 {
                for i in 0..L_SUBFR {
                    let e = |k: usize| i32::from(exc_buf.get(k).copied().unwrap_or(0));
                    let l = (e(pos + i - 1) + e(pos + i + 1)) * 5898 + e(pos + i) * 20972;
                    code[i] = round(l.wrapping_shl(1));
                }
                exc_buf[pos..pos + L_SUBFR].copy_from_slice(&code);
            }
            self.decode_code(&mut bits, nb_bits, unusable, &mut code);
            preemph(&mut code, self.tilt_code);
            pit_shrp(&mut code, if t0_frac > 2 { t0.wrapping_add(1) } else { t0 }, PIT_SHARP);
            let gi = GainIn {
                index: bits.parm(if nb_bits <= NBBITS_9K { 6 } else { 7 }),
                nbits: if nb_bits <= NBBITS_9K { 6 } else { 7 },
                bfi,
                prev_bfi: self.prev_bfi,
                state: self.state,
                unusable,
                vad_hist: self.vad_hist,
            };
            let (gain_pit, mut l_gain_code) = dec_gain2(&gi, &code, &mut self.dec_gain);
            let qmax = self.qsubfr.iter().copied().min().unwrap_or(Q_MAX).min(Q_MAX);
            q_new = 0;
            let mut l = l_gain_code;
            while l < 0x0800_0000 && q_new < qmax {
                l = l.wrapping_shl(1);
                q_new += 1;
            }
            let mut gain_code = round(l);
            scale_signal(&mut exc_buf[i_subfr..pos + L_SUBFR], q_new.wrapping_sub(self.q_old));
            self.q_old = q_new;
            if !bfi {
                self.lag_hist.copy_within(0..4, 1);
                self.lag_hist[0] = t0;
                self.old_t0 = t0;
            }
            let mut ex2 = [0i16; L_SUBFR];
            for (d, &e) in ex2.iter_mut().zip(&exc_buf[pos..pos + L_SUBFR]) {
                *d = ((i32::from(e) + if e != MAX_16 { 4 } else { 0 }) >> 3) as i16;
            }
            let pit_sharp = if nb_bits <= NBBITS_9K { shl(gain_pit, 1) } else { 0 };
            if nb_bits <= NBBITS_9K && pit_sharp > 16384 {
                for (p, &e) in excp.iter_mut().zip(&ex2) {
                    let t = mult(e, pit_sharp);
                    *p = round(l_mult(t, gain_pit) >> 1);
                }
            }
            let voice_fac = voice_factor(&ex2, -3, gain_pit, &code, gain_code);
            self.tilt_code = (voice_fac >> 2).wrapping_add(8192);
            ex2.copy_from_slice(&exc_buf[pos..pos + L_SUBFR]);
            let mut max: i32 = 1;
            for (i, &c) in code.iter().enumerate() {
                let mut l = l_shl(l_mult(c, gain_code), 5);
                l = l_mac(l, exc_buf[pos + i], gain_pit);
                let t = round(l_shl(l, 1));
                exc_buf[pos + i] = t;
                let t = i32::from(t) - i32::from(t < 0);
                max |= t ^ (t >> 15);
            }
            let tmp = add(norm_s(max), q_new) - 1;
            self.qsubfr.copy_within(0..3, 1);
            self.qsubfr[0] = tmp;
            let j = if nb_bits <= NBBITS_7K {
                0
            } else if nb_bits <= NBBITS_9K {
                1
            } else {
                2
            };
            phase_dispersion((l_gain_code >> 16) as i16, gain_pit, &mut code, j, &mut self.disp_mem);
            let fac = mult(stab_fac, 16384i16.wrapping_sub(voice_fac >> 1));
            let mut l = l_gain_code;
            if l < self.l_gc_thres {
                l = l.wrapping_add(mul32_16b(l_gain_code, 6226).wrapping_shl(1));
                if l > self.l_gc_thres {
                    l = self.l_gc_thres;
                }
            } else {
                l = mul32_16b(l_gain_code, 27536).wrapping_shl(1);
                if l < self.l_gc_thres {
                    l = self.l_gc_thres;
                }
            }
            self.l_gc_thres = l;
            l_gain_code = mul32_16b(l_gain_code, 32767 - i32::from(fac)).wrapping_shl(1);
            l_gain_code = l_add(l_gain_code, mul32_16b(l, i32::from(fac)).wrapping_shl(1));
            let tmp = (voice_fac >> 3).wrapping_add(4096);
            gain_code = round(l_shl(l_gain_code, q_new));
            for i in 0..L_SUBFR {
                let neighbours = match i {
                    0 => code[1],
                    63 => code[62],
                    _ => code[i + 1].wrapping_add(code[i - 1]),
                };
                let mut l = l_msu(i32::from(code[i]) << 16, neighbours, tmp);
                l = l_shl(l_mult(round(l), gain_code), 5);
                l = l_mac(l, ex2[i], gain_pit);
                ex2[i] = round(l_shl(l, 1));
            }
            if nb_bits <= NBBITS_9K && pit_sharp > 16384 {
                for (p, &e) in excp.iter_mut().zip(&ex2) {
                    *p = add(*p, e);
                }
                agc2(&ex2, &mut excp);
                ex2 = excp;
            }
            if nb_bits <= NBBITS_7K {
                interp_isf(&isf_tmp, &isf, INTERPOL_FRAC[sub_i], &mut hf_isf);
            } else {
                self.mem_syn_hf[..M16K - M].fill(0);
            }
            let corr_gain = if nb_bits >= NBBITS_24K { bits.parm(4) } else { 0 };
            let start = i_subfr + (i_subfr >> 2);
            let si = SynthIn { aq: aq_sub, q_new, corr_gain, nb_bits, speech: true, bfi };
            if let Some(out) = synth16k.get_mut(start..start + L_SUBFR16K) {
                self.synthesis(&si, &mut ex2, out, &mut hf_isf);
            }
        }
        self.old_exc.copy_from_slice(&exc_buf[L_FRAME..L_FRAME + EXC_HIST]);
        let exc = &mut exc_buf[EXC_HIST..EXC_HIST + L_FRAME];
        scale_signal(exc, q_new.wrapping_neg());
        self.dtx.activity_update(&isf, exc);
        self.dtx.global_state = new_dtx;
        self.prev_bfi = bfi;
    }

    /// The algebraic codebook vector of one subframe (or noise for an unusable frame).
    fn decode_code(&mut self, bits: &mut Bits<'_>, nb_bits: i16, unusable: bool, code: &mut [i16; L_SUBFR]) {
        let mut ind = [0i16; 8];
        let mut read = |widths: &[i16]| {
            for (k, &w) in widths.iter().enumerate() {
                ind[k] = bits.parm(w);
            }
        };
        if unusable {
            for c in code.iter_mut() {
                *c = noise(&mut self.seed) >> 3;
            }
        } else if nb_bits <= NBBITS_7K {
            read(&[12]);
            dec_acelp_2p(ind[0], code);
        } else {
            let (widths, nb): (&[i16], i16) = if nb_bits <= NBBITS_9K {
                (&[5, 5, 5, 5], 20)
            } else if nb_bits <= NBBITS_12K {
                (&[9, 9, 9, 9], 36)
            } else if nb_bits <= NBBITS_14K {
                (&[13, 13, 9, 9], 44)
            } else if nb_bits <= NBBITS_16K {
                (&[13, 13, 13, 13], 52)
            } else if nb_bits <= NBBITS_18K {
                (&[2, 2, 2, 2, 14, 14, 14, 14], 64)
            } else if nb_bits <= NBBITS_20K {
                (&[10, 10, 2, 2, 10, 10, 14, 14], 72)
            } else {
                (&[11, 11, 11, 11, 11, 11, 11, 11], 88)
            };
            read(widths);
            dec_acelp_4p(&ind, nb, code);
        }
    }

    /// `synthesis_amr_wb`: one 64-sample subframe to 80 samples at 16 kHz, with the 6.4-7 kHz
    /// band rebuilt from shaped noise.
    fn synthesis(&mut self, si: &SynthIn<'_>, exc: &mut [i16], out: &mut [i16], hf_isf: &mut [i16; M16K]) {
        let mut hi = [0i16; M + L_SUBFR];
        let mut lo = [0i16; M + L_SUBFR];
        hi[..M].copy_from_slice(&self.mem_syn_hi);
        lo[..M].copy_from_slice(&self.mem_syn_lo);
        syn_filt_32(si.aq, exc, si.q_new, &mut hi, &mut lo);
        self.mem_syn_hi.copy_from_slice(&hi[L_SUBFR..]);
        self.mem_syn_lo.copy_from_slice(&lo[L_SUBFR..]);
        let mut synth = [0i16; L_SUBFR];
        deemphasis_32(&hi[M..], &lo[M..], &mut synth, PREEMPH_FAC, &mut self.mem_deemph);
        highpass_50hz(&mut synth, &mut self.mem_sig_out);
        oversample(&synth, out, &mut self.mem_oversamp);
        let mut hf = [0i16; L_SUBFR16K];
        for v in hf.iter_mut() {
            *v = noise(&mut self.seed2) >> 3;
        }
        for v in exc.iter_mut() {
            *v = add(*v, 4) >> 3;
        }
        let q_new = si.q_new - 3;
        let mut exp_ener = 0;
        let ener = (dot_product12(exc, exc, &mut exp_ener) >> 16) as i16;
        exp_ener = exp_ener.wrapping_sub(q_new.wrapping_shl(1));
        let mut exp = 0;
        let mut tmp = (dot_product12(&hf, &hf, &mut exp) >> 16) as i16;
        if tmp > ener {
            tmp >>= 1;
            exp += 1;
        }
        let mut l = i32::from(div_16by16(tmp, ener)) << 16;
        exp = exp.wrapping_sub(exp_ener);
        one_ov_sqrt_norm(&mut l, &mut exp);
        let l = l_shl(l, exp.wrapping_add(1));
        let tmp = (l >> 16) as i16;
        for v in hf.iter_mut() {
            *v = ((i32::from(*v) * i32::from(tmp)) >> 15) as i16;
        }
        highpass_400hz(&mut synth, &mut self.mem_hp400);
        let mut l: i32 = l_mac(1, synth[0], synth[0]);
        let mut l2: i32 = 1;
        for i in 1..L_SUBFR {
            l = l_mac(l, synth[i], synth[i]);
            l2 = l_mac(l2, synth[i], synth[i - 1]);
        }
        let e = norm(l);
        let ener = (l.wrapping_shl(e as u32) >> 16) as i16;
        let t = (l2.wrapping_shl(e as u32) >> 16) as i16;
        let fac = if t > 0 { div_16by16(t, ener) } else { 0 };
        let gain1 = 32767 - fac;
        let gain2 = shl(mult(gain1, 20480), 1);
        let mut tmp = if self.vad_hist > 0 { gain2.wrapping_sub(1) } else { gain1.wrapping_sub(1) };
        if tmp != 0 {
            tmp = tmp.wrapping_add(1);
        }
        if tmp < 3277 {
            tmp = 3277;
        }
        if si.nb_bits >= NBBITS_24K && !si.bfi {
            let g = at(&HP_GAIN, i32::from(si.corr_gain));
            for v in hf.iter_mut() {
                *v = mult(*v, g).wrapping_shl(1);
            }
        } else {
            for v in hf.iter_mut() {
                *v = mult(*v, tmp);
            }
        }
        let mut ap = [0i16; M16K + 1];
        if si.nb_bits <= NBBITS_7K && si.speech {
            isf_extrapolation(hf_isf);
            let mut hfa = [0i16; M16K + 1];
            isp_az(hf_isf, &mut hfa, M16K, false);
            weight_lpc(&hfa, &mut ap, 29491, M16K);
            wb_syn_filt(&ap, M16K, &mut hf, &mut self.mem_syn_hf);
        } else {
            weight_lpc(si.aq, &mut ap, 19661, M);
            wb_syn_filt(&ap, M, &mut hf, &mut self.mem_syn_hf[M16K - M..]);
        }
        band_pass_6k_7k(&mut hf, &mut self.mem_hf);
        if si.nb_bits >= NBBITS_24K {
            low_pass_7k(&mut hf, &mut self.mem_hf3);
        }
        for (o, &h) in out.iter_mut().zip(&hf) {
            *o = add(*o, h);
        }
    }
}

/// The high-band ISFs of one subframe: the old and new ISFs interpolated.
fn interp_isf(old: &[i16; M], new: &[i16; M], frac: i16, hf_isf: &mut [i16; M16K]) {
    for i in 0..M {
        let l = l_mult(old[i], sub(32767, frac));
        hf_isf[i] = round(l_mac(l, new[i], frac));
    }
}

fn sort_table(mode: usize) -> &'static [i16] {
    match mode {
        0 => &SORT_660,
        1 => &SORT_885,
        2 => &SORT_1265,
        3 => &SORT_1425,
        4 => &SORT_1585,
        5 => &SORT_1825,
        6 => &SORT_1985,
        7 => &SORT_2305,
        8 => &SORT_2385,
        9 => &SORT_SID,
        _ => &[],
    }
}

fn homing_table(mode: usize) -> &'static [i16] {
    match mode {
        0 => &DFH_M7K,
        1 => &DFH_M9K,
        2 => &DFH_M12K,
        3 => &DFH_M14K,
        4 => &DFH_M16K,
        5 => &DFH_M18K,
        6 => &DFH_M20K,
        7 => &DFH_M23K,
        _ => &DFH_M24K,
    }
}

/// `dhf_test`: whether the first `nparms` bits are those of the decoder homing frame.
fn is_homing(bits: &[i16], mode: usize, nparms: i16) -> bool {
    let mut b = Bits { bits, pos: 0 };
    let mut param = [0i16; 32];
    let (count, shift);
    if mode != 8 {
        let tmp = nparms - 15;
        let (mut i, mut j) = (0usize, 0i16);
        while tmp > j && i < 31 {
            param[i] = b.parm(15);
            j += 15;
            i += 1;
        }
        let rest = nparms - j;
        shift = 15 - rest;
        param[i] = shl(b.parm(rest), shift);
        count = i;
    } else {
        for (i, p) in param.iter_mut().enumerate().take(31) {
            *p = b.parm(15);
            match i {
                10 => *p &= 0x61FF,
                17 => *p &= 0xE0FFu16 as i16,
                24 => *p &= 0x7F0F,
                _ => {}
            }
        }
        param[31] = shl(b.parm(8), 7);
        shift = 0;
        count = 31;
    }
    let table = homing_table(mode);
    let mut j: i16 = 0;
    let mut i = 0usize;
    while i < count {
        j = param[i] ^ table.get(i).copied().unwrap_or(0);
        if j != 0 {
            break;
        }
        i += 1;
    }
    let mask = shl(0x7fff >> shift, shift);
    let t = table.get(i).copied().unwrap_or(0) & mask;
    j |= param.get(i).copied().unwrap_or(0) ^ t;
    j == 0
}

const PRMNOFSF: [i16; 9] = [63, 81, 100, 108, 116, 128, 136, 152, 156];

/// An AMR-WB decoder: RFC 4867 frames (table-of-contents byte plus payload) in, 320 samples of
/// 16 kHz audio out per frame.
#[derive(Clone)]
pub struct Decoder {
    st: Box<State>,
    bits: [i16; MAX_BITS],
    prev_mode: usize,
    mode_old: usize,
    reset_flag: bool,
    reset_flag_old: bool,
}

impl std::fmt::Debug for Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("wb::Decoder").field("prev_mode", &self.prev_mode).finish_non_exhaustive()
    }
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        Decoder { st: Box::new(State::new()), bits: [0; MAX_BITS], prev_mode: 0, mode_old: 0, reset_flag: false, reset_flag_old: true }
    }

    /// Bytes one frame occupies in a storage file or packet, including the table-of-contents
    /// byte, from that byte.
    pub fn frame_size(toc: u8) -> usize {
        1 + PAYLOAD_BYTES[usize::from((toc >> 3) & 15)]
    }

    /// `mime_unsorting`: payload bits into the decoder's parameter order. Returns the frame
    /// type and updates `mode` as opencore does (comfort noise and lost frames keep the
    /// previous speech mode). Bits a frame does not carry keep their previous values.
    fn unsort(&mut self, payload: &[u8], mode: &mut usize) -> Rx {
        let table = sort_table(*mode);
        let n = usize::try_from(NB_BITS.get(*mode).copied().unwrap_or(0)).unwrap_or(0).min(MAX_BITS);
        self.bits[..n].fill(0);
        let bit = |i: usize| payload.get(i >> 3).is_some_and(|b| (b >> (7 - (i & 7))) & 1 != 0);
        for (i, &dst) in table.iter().enumerate().take(n) {
            if bit(i)
                && let Some(s) = usize::try_from(dst).ok().and_then(|d| self.bits.get_mut(d))
            {
                *s = BIT_1;
            }
        }
        let ft = match *mode {
            0..=8 => Rx::SpeechGood,
            9 => {
                *mode = self.prev_mode;
                if bit(35) { Rx::SidUpdate } else { Rx::SidFirst }
            }
            14 => {
                *mode = self.prev_mode;
                Rx::SpeechLost
            }
            _ => {
                *mode = self.prev_mode;
                Rx::NoData
            }
        };
        self.prev_mode = *mode;
        ft
    }

    /// Decodes one frame (`D_IF_decode`). `frame` starts with the table-of-contents byte; a
    /// shorter payload than the frame type needs is padded with zeros.
    pub fn decode_frame(&mut self, frame: &[u8], out: &mut [i16; FRAME_SAMPLES]) -> Result<(), Error> {
        let (&toc, payload) = frame.split_first().ok_or(Error::Truncated("AMR-WB frame"))?;
        let mut mode = usize::from((toc >> 3) & 0x0f);
        let ft = self.unsort(payload, &mut mode);
        if matches!(ft, Rx::NoData | Rx::SpeechLost) {
            mode = self.mode_old;
            self.reset_flag = false;
        } else {
            self.mode_old = mode;
            if self.reset_flag_old {
                self.reset_flag = is_homing(&self.bits, mode, PRMNOFSF.get(mode).copied().unwrap_or(0));
            }
        }
        if self.reset_flag && self.reset_flag_old {
            *out = [8; FRAME_SAMPLES];
        } else {
            self.st.decode(mode, &self.bits, out, ft);
        }
        for v in out.iter_mut() {
            *v &= !3;
        }
        if !self.reset_flag_old {
            self.reset_flag = is_homing(&self.bits, mode, NB_BITS.get(mode).copied().unwrap_or(0));
        }
        if self.reset_flag {
            self.st.reset(true);
        }
        self.reset_flag_old = self.reset_flag;
        Ok(())
    }
}
