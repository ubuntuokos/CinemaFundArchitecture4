// Derived from opencore-amr 0.1.6 (commit 7dba8c32), amr_nb: dec/src/amrdecode.cpp,
// dec/src/sp_dec.cpp, dec/src/dec_amr.cpp, dec/src/d_plsf.cpp, dec/src/d_plsf_3.cpp,
// dec/src/d_plsf_5.cpp, common/src/bits2prm.cpp, dec/src/wmf_to_ets.cpp, amrnb/wrapper.cpp.
// Copyright (C) 1998-2009 PacketVideo, Apache License 2.0; portions (C) 2004 3GPP
// Organizational Partners (3GPP TS 26.073). Ported to Rust for CFA4 (GPL-3.0-or-later).

//! AMR narrowband (3GPP TS 26.071/26.073/26.090), bit-exact with opencore-amrnb.

mod codebook;
mod dtx;
mod gain;
mod lpc;
mod math;
mod post;
mod tables;

use codebook::{
    build_cn_param, dec_8i40_31bits, dec_10i40_35bits, dec_lag3, dec_lag6, decode_2i40_9bits, decode_2i40_11bits, decode_3i40_14bits, decode_4i40_17bits,
    pred_lt_3or6,
};
use dtx::{DtxDec, DtxState, DtxTarget};
use gain::{CbGainAverage, EcGainCode, EcGainPitch, Flags, GcPred, d_gain_code, d_gain_pitch, dec_gain};
use lpc::{int_lpc_1and3, int_lpc_1to3, int_lsf, lsf_lsp, lsp_avg, reorder_lsf, syn_filt};
use post::{BgnScd, PhDisp, PostFilter, PostProcess, agc2, ex_ctrl};
use tables::{
    BITNO_MR59, BITNO_MR67, BITNO_MR74, BITNO_MR102, BITNO_MR122, BITNO_MR475, BITNO_MR515, BITNO_MR795, BITNO_MRDTX, DICO1_LSF_3, DICO1_LSF_5, DICO2_LSF_3,
    DICO2_LSF_5, DICO3_LSF_3, DICO3_LSF_5, DICO4_LSF_5, DICO5_LSF_5, MEAN_LSF_3, MEAN_LSF_5, MR515_3_LSF, MR795_1_LSF, NUMOFBITS, PAST_RQ_INIT, PRED_FAC_3,
    REORDERBITS_MR59, REORDERBITS_MR67, REORDERBITS_MR74, REORDERBITS_MR102, REORDERBITS_MR122, REORDERBITS_MR475, REORDERBITS_MR515, REORDERBITS_MR795,
    WINDOW_200_40,
};

use crate::Error;
use crate::basic::{MAX_16, MIN_16, add, at, l_mac, l_mult, l_shl, l_shr, mult, negate, round, shl, sub};

/// Samples per frame (20 ms at 8 kHz).
pub const FRAME_SAMPLES: usize = 160;
/// Output sample rate.
pub const SAMPLE_RATE: u32 = 8000;

pub(crate) const M: usize = 10;
pub(crate) const MP1: usize = M + 1;
pub(crate) const L_FRAME: usize = 160;
pub(crate) const L_SUBFR: usize = 40;
pub(crate) const LSF_GAP: i16 = 205;
const PIT_MIN: i16 = 20;
const PIT_MIN_MR122: i16 = 18;
const PIT_MAX: i16 = 143;
const L_INTERPOL: usize = 11;
const EXC_OFF: usize = PIT_MAX as usize + L_INTERPOL;
const SHARPMAX: i16 = 13017;
const LSP_PRED_FAC_MR122: i16 = 21299;
const MAX_PRM_SIZE: usize = 57;
const MAX_SERIAL_SIZE: usize = 244;

/// Codec modes, in bit-rate order (the RFC 4867 frame types 0–7) plus the SID parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Mode {
    Mr475 = 0,
    Mr515,
    Mr59,
    Mr67,
    Mr74,
    Mr795,
    Mr102,
    Mr122,
    MrDtx,
}

impl Mode {
    fn from_index(i: i32) -> Mode {
        match i {
            0 => Mode::Mr475,
            1 => Mode::Mr515,
            2 => Mode::Mr59,
            3 => Mode::Mr67,
            4 => Mode::Mr74,
            5 => Mode::Mr795,
            6 => Mode::Mr102,
            7 => Mode::Mr122,
            _ => Mode::MrDtx,
        }
    }

    fn bitno(self) -> &'static [i16] {
        match self {
            Mode::Mr475 => &BITNO_MR475,
            Mode::Mr515 => &BITNO_MR515,
            Mode::Mr59 => &BITNO_MR59,
            Mode::Mr67 => &BITNO_MR67,
            Mode::Mr74 => &BITNO_MR74,
            Mode::Mr795 => &BITNO_MR795,
            Mode::Mr102 => &BITNO_MR102,
            Mode::Mr122 => &BITNO_MR122,
            Mode::MrDtx => &BITNO_MRDTX,
        }
    }

    fn reorder(self) -> &'static [i16] {
        match self {
            Mode::Mr475 => &REORDERBITS_MR475,
            Mode::Mr515 => &REORDERBITS_MR515,
            Mode::Mr59 => &REORDERBITS_MR59,
            Mode::Mr67 => &REORDERBITS_MR67,
            Mode::Mr74 => &REORDERBITS_MR74,
            Mode::Mr795 => &REORDERBITS_MR795,
            Mode::Mr102 => &REORDERBITS_MR102,
            Mode::Mr122 | Mode::MrDtx => &REORDERBITS_MR122,
        }
    }
}

/// Receive frame classification (`RXFrameType`). RFC 4867 input yields only good speech, SID
/// and no-data frames; the other classes are kept so the ported logic stays whole.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RxType {
    SpeechGood,
    SpeechDegraded,
    Onset,
    SpeechBad,
    SidFirst,
    SidUpdate,
    SidBad,
    NoData,
}

/// Bytes of payload after the table-of-contents byte, by RFC 4867 frame type.
pub const PAYLOAD_BYTES: [usize; 16] = [12, 13, 15, 17, 19, 20, 26, 31, 5, 0, 0, 0, 0, 0, 0, 0];

/// LSF dequantiser state (`D_plsfState`).
#[derive(Clone, Debug)]
pub(crate) struct DPlsf {
    pub(crate) past_r_q: [i16; M],
    pub(crate) past_lsf_q: [i16; M],
}

impl DPlsf {
    fn new() -> Self {
        DPlsf { past_r_q: [0; M], past_lsf_q: MEAN_LSF_5 }
    }

    pub(crate) fn init_3(&mut self, index: i16) {
        let base = i32::from(index) * M as i32;
        for (i, v) in self.past_r_q.iter_mut().enumerate() {
            *v = at(&PAST_RQ_INIT, base + i as i32);
        }
    }

    /// `D_plsf_3`: split-VQ of the residual LSFs with MA prediction.
    pub(crate) fn decode_3(&mut self, mode: Mode, bfi: bool, indice: &[i16], lsp1_q: &mut [i16; M]) {
        const ALPHA: i16 = 29491;
        const ONE_ALPHA: i16 = 3277;
        let ix = |i: usize| i32::from(indice.get(i).copied().unwrap_or(0));
        let mut lsf1_q = [0i16; M];
        if bfi {
            for i in 0..M {
                lsf1_q[i] = add(mult(MEAN_LSF_3[i], ONE_ALPHA), mult(self.past_lsf_q[i], ALPHA));
            }
            for i in 0..M {
                let temp = if mode != Mode::MrDtx { add(MEAN_LSF_3[i], mult(self.past_r_q[i], PRED_FAC_3[i])) } else { add(MEAN_LSF_3[i], self.past_r_q[i]) };
                self.past_r_q[i] = sub(lsf1_q[i], temp);
            }
        } else {
            let (cb1, lim1, cb3, lim3): (&[i16], i32, &[i16], i32) = match mode {
                Mode::Mr475 | Mode::Mr515 => (&DICO1_LSF_3, 255 * 3, &MR515_3_LSF, 127 * 4),
                Mode::Mr795 => (&MR795_1_LSF, 511 * 3, &DICO3_LSF_3, 511 * 4),
                _ => (&DICO1_LSF_3, 255 * 3, &DICO3_LSF_3, 511 * 4),
            };
            let mut lsf1_r = [0i16; M];
            let t = (ix(0) * 3).min(lim1);
            for k in 0..3 {
                lsf1_r[k] = at(cb1, t + k as i32);
            }
            let mut index = ix(1);
            if matches!(mode, Mode::Mr475 | Mode::Mr515) {
                index <<= 1;
            }
            let t = (index * 3).min(511 * 3);
            for k in 0..3 {
                lsf1_r[3 + k] = at(&DICO2_LSF_3, t + k as i32);
            }
            let t = (ix(2) << 2).min(lim3);
            for k in 0..4 {
                lsf1_r[6 + k] = at(cb3, t + k as i32);
            }
            for i in 0..M {
                let temp = if mode != Mode::MrDtx { add(MEAN_LSF_3[i], mult(self.past_r_q[i], PRED_FAC_3[i])) } else { add(MEAN_LSF_3[i], self.past_r_q[i]) };
                lsf1_q[i] = add(lsf1_r[i], temp);
                self.past_r_q[i] = lsf1_r[i];
            }
        }
        reorder_lsf(&mut lsf1_q, LSF_GAP);
        self.past_lsf_q = lsf1_q;
        lsf_lsp(&lsf1_q, lsp1_q);
    }

    /// `D_plsf_5`: the two LSF sets of MR122.
    fn decode_5(&mut self, bfi: bool, indice: &[i16], lsp1_q: &mut [i16; M], lsp2_q: &mut [i16; M]) {
        const ALPHA: i32 = 31128;
        const ONE_ALPHA: i32 = 1639;
        let ix = |i: usize| indice.get(i).copied().unwrap_or(0);
        let mut lsf1_q = [0i16; M];
        let mut lsf2_q = [0i16; M];
        if bfi {
            for i in 0..M {
                let temp = ((i32::from(self.past_lsf_q[i]) * ALPHA) >> 15) as i16;
                let sign = ((i32::from(MEAN_LSF_5[i]) * ONE_ALPHA) >> 15) as i16;
                lsf1_q[i] = add(sign, temp);
                lsf2_q[i] = lsf1_q[i];
                let temp = ((i32::from(self.past_r_q[i]) * i32::from(LSP_PRED_FAC_MR122)) >> 15) as i16;
                let temp = add(MEAN_LSF_5[i], temp);
                self.past_r_q[i] = sub(lsf2_q[i], temp);
            }
        } else {
            let mut lsf1_r = [0i16; M];
            let mut lsf2_r = [0i16; M];
            let mut take = |dico: &[i16], idx: i16, k: usize, neg: bool| {
                let t = i32::from(shl(idx, 2));
                let f = |v: i16| if neg { negate(v) } else { v };
                lsf1_r[k] = f(at(dico, t));
                lsf1_r[k + 1] = f(at(dico, t + 1));
                lsf2_r[k] = f(at(dico, t + 2));
                lsf2_r[k + 1] = f(at(dico, t + 3));
            };
            take(&DICO1_LSF_5, ix(0), 0, false);
            take(&DICO2_LSF_5, ix(1), 2, false);
            take(&DICO3_LSF_5, ix(2) >> 1, 4, ix(2) & 1 != 0);
            take(&DICO4_LSF_5, ix(3), 6, false);
            take(&DICO5_LSF_5, ix(4), 8, false);
            for i in 0..M {
                let temp = add(MEAN_LSF_5[i], mult(self.past_r_q[i], LSP_PRED_FAC_MR122));
                lsf1_q[i] = add(lsf1_r[i], temp);
                lsf2_q[i] = add(lsf2_r[i], temp);
                self.past_r_q[i] = lsf2_r[i];
            }
        }
        reorder_lsf(&mut lsf1_q, LSF_GAP);
        reorder_lsf(&mut lsf2_q, LSF_GAP);
        self.past_lsf_q = lsf2_q;
        lsf_lsp(&lsf1_q, lsp1_q);
        lsf_lsp(&lsf2_q, lsp2_q);
    }
}

const LSP_INIT: [i16; M] = [30000, 26000, 21000, 15000, 8000, 0, -8000, -15000, -21000, -26000];

/// The speech decoder proper (`Decoder_amrState`).
#[derive(Clone, Debug)]
struct DecoderAmr {
    old_exc: [i16; L_SUBFR + EXC_OFF],
    lsp_old: [i16; M],
    mem_syn: [i16; M],
    sharp: i16,
    old_t0: i16,
    prev_bf: bool,
    prev_pdf: bool,
    state: i16,
    exc_energy_hist: [i16; 9],
    t0_lag_buff: i16,
    in_background_noise: bool,
    voiced_hangover: i16,
    ltp_gain_history: [i16; 9],
    background: BgnScd,
    nodata_seed: i16,
    cb_gain_aver: CbGainAverage,
    lsp_mean_save: [i16; M],
    lsf_state: DPlsf,
    ec_gain_p: EcGainPitch,
    ec_gain_c: EcGainCode,
    pred: GcPred,
    ph_disp: PhDisp,
    dtx: DtxDec,
}

impl DecoderAmr {
    fn new() -> Self {
        DecoderAmr {
            old_exc: [0; L_SUBFR + EXC_OFF],
            lsp_old: LSP_INIT,
            mem_syn: [0; M],
            sharp: 0,
            old_t0: 40,
            prev_bf: false,
            prev_pdf: false,
            state: 0,
            exc_energy_hist: [0; 9],
            t0_lag_buff: 40,
            in_background_noise: false,
            voiced_hangover: 0,
            ltp_gain_history: [0; 9],
            background: BgnScd::new(),
            nodata_seed: 21845,
            cb_gain_aver: CbGainAverage::new(),
            lsp_mean_save: MEAN_LSF_5,
            lsf_state: DPlsf::new(),
            ec_gain_p: EcGainPitch::new(),
            ec_gain_c: EcGainCode::new(),
            pred: GcPred::new(),
            ph_disp: PhDisp::default(),
            dtx: DtxDec::new(),
        }
    }

    /// `Decoder_amr_reset(st, MRDTX)`: the partial reset before comfort noise.
    fn reset_for_dtx(&mut self) {
        self.old_exc[..EXC_OFF].fill(0);
        self.sharp = 0;
        self.old_t0 = 40;
        self.prev_bf = false;
        self.prev_pdf = false;
        self.state = 0;
        self.t0_lag_buff = 40;
        self.in_background_noise = false;
        self.voiced_hangover = 0;
        self.ltp_gain_history = [0; 9];
        self.cb_gain_aver = CbGainAverage::new();
        self.lsf_state = DPlsf::new();
        self.ec_gain_p = EcGainPitch::new();
        self.ec_gain_c = EcGainCode::new();
        self.background = BgnScd::new();
        self.nodata_seed = 21845;
        self.ph_disp = PhDisp::default();
    }

    /// `Decoder_amr`: one frame of synthesis (before post-filtering) and its LPC filters.
    fn decode(&mut self, mode: Mode, parm_in: &[i16], frame_type: RxType, synth: &mut [i16; L_FRAME], a_t: &mut [i16; 4 * MP1]) {
        let mut parm = [0i16; MAX_PRM_SIZE + 1];
        for (d, s) in parm.iter_mut().zip(parm_in) {
            *d = *s;
        }
        let new_state = self.dtx.rx_handler(frame_type);
        if new_state != DtxState::Speech {
            self.reset_for_dtx();
            let target = DtxTarget { mem_syn: &mut self.mem_syn, lsf_state: &mut self.lsf_state, pred: &mut self.pred, aver: &mut self.cb_gain_aver };
            self.dtx.decode(target, new_state, mode, &parm, synth, a_t);
            lsf_lsp(&self.lsf_state.past_lsf_q, &mut self.lsp_old);
            lsp_avg(&mut self.lsp_mean_save, &self.lsf_state.past_lsf_q);
            self.dtx.global_state = new_state;
            return;
        }
        let mut bfi = false;
        let mut pdfi = false;
        match frame_type {
            RxType::SpeechBad | RxType::NoData | RxType::Onset => {
                bfi = true;
                if matches!(frame_type, RxType::NoData | RxType::Onset) {
                    let sizes = mode.bitno();
                    build_cn_param(&mut self.nodata_seed, sizes, &WINDOW_200_40, &mut parm[..sizes.len()]);
                }
            }
            RxType::SpeechDegraded => pdfi = true,
            _ => {}
        }
        if bfi {
            self.state += 1;
        } else if self.state == 6 {
            self.state = 5;
        } else {
            self.state = 0;
        }
        if self.state > 6 {
            self.state = 6;
        }
        match self.dtx.global_state {
            DtxState::Dtx => {
                self.state = 5;
                self.prev_bf = false;
            }
            DtxState::DtxMute => {
                self.state = 5;
                self.prev_bf = true;
            }
            DtxState::Speech => {}
        }
        let prev_lsf = self.lsf_state.past_lsf_q;
        let mut lsp_new = [0i16; M];
        let mut pi: usize;
        if mode != Mode::Mr122 {
            self.lsf_state.decode_3(mode, bfi, &parm[0..3], &mut lsp_new);
            pi = 3;
            int_lpc_1to3(&self.lsp_old, &lsp_new, a_t);
        } else {
            let mut lsp_mid = [0i16; M];
            self.lsf_state.decode_5(bfi, &parm[0..5], &mut lsp_mid, &mut lsp_new);
            pi = 5;
            int_lpc_1and3(&self.lsp_old, &lsp_mid, &lsp_new, a_t);
        }
        self.lsp_old = lsp_new;

        let low_rate = matches!(mode, Mode::Mr475 | Mode::Mr515 | Mode::Mr59);
        let next = |pi: &mut usize| {
            let v = parm.get(*pi).copied().unwrap_or(0);
            *pi += 1;
            v
        };
        let mut even_subfr = false;
        let mut index_mr475: i16 = 0;
        let mut t0: i16 = 0;
        for (sub_nr, i_subfr) in (0..L_FRAME).step_by(L_SUBFR).enumerate() {
            even_subfr = !even_subfr;
            let mut pit_flag = i_subfr as i32;
            if i_subfr == L_FRAME / 2 && !matches!(mode, Mode::Mr475 | Mode::Mr515) {
                pit_flag = 0;
            }
            let index = next(&mut pi);
            let t0_frac;
            if mode != Mode::Mr122 {
                let flag4 = matches!(mode, Mode::Mr475 | Mode::Mr515 | Mode::Mr59 | Mode::Mr67);
                let (dl, dr) = if mode == Mode::Mr795 { (10, 19) } else { (5, 9) };
                let mut t0_min = self.old_t0.wrapping_sub(dl).max(PIT_MIN);
                let mut t0_max = t0_min + dr;
                if t0_max > PIT_MAX {
                    t0_max = PIT_MAX;
                    t0_min = t0_max - dr;
                }
                let (t, f) = dec_lag3(index, t0_min, t0_max, pit_flag, self.old_t0, flag4);
                t0 = t;
                let mut frac = f;
                self.t0_lag_buff = t0;
                if bfi {
                    if self.old_t0 < PIT_MAX {
                        self.old_t0 += 1;
                    }
                    t0 = self.old_t0;
                    frac = 0;
                    if self.in_background_noise && self.voiced_hangover > 4 && low_rate {
                        t0 = self.t0_lag_buff;
                    }
                }
                t0_frac = frac;
                pred_lt_3or6(&mut self.old_exc, EXC_OFF, t0, t0_frac, true);
            } else {
                let (t, f) = dec_lag6(index, PIT_MIN_MR122, PIT_MAX, pit_flag, t0);
                t0 = t;
                let mut frac = f;
                if !(!bfi && (pit_flag == 0 || index < 61)) {
                    self.t0_lag_buff = t0;
                    t0 = self.old_t0;
                    frac = 0;
                }
                t0_frac = frac;
                pred_lt_3or6(&mut self.old_exc, EXC_OFF, t0, t0_frac, false);
            }

            let mut code = [0i16; L_SUBFR];
            let mut gain_pit: i16 = 0;
            let sharp2 = |v: i16| -> i16 {
                let l = i32::from(v) << 1;
                if l != i32::from(l as i16) { if v > 0 { MAX_16 } else { MIN_16 } } else { l as i16 }
            };
            let mut pit_sharp: i16;
            match mode {
                Mode::Mr475 | Mode::Mr515 => {
                    let index = next(&mut pi);
                    let sign = next(&mut pi);
                    decode_2i40_9bits(sub_nr as i16, sign, index, &mut code);
                    pit_sharp = sharp2(self.sharp);
                }
                Mode::Mr59 => {
                    let index = next(&mut pi);
                    let sign = next(&mut pi);
                    decode_2i40_11bits(sign, index, &mut code);
                    pit_sharp = sharp2(self.sharp);
                }
                Mode::Mr67 => {
                    let index = next(&mut pi);
                    let sign = next(&mut pi);
                    decode_3i40_14bits(sign, index, &mut code);
                    pit_sharp = sharp2(self.sharp);
                }
                Mode::Mr74 | Mode::Mr795 => {
                    let index = next(&mut pi);
                    let sign = next(&mut pi);
                    decode_4i40_17bits(sign, index, &mut code);
                    pit_sharp = sharp2(self.sharp);
                }
                Mode::Mr102 => {
                    dec_8i40_31bits(parm.get(pi..pi + 7).unwrap_or(&[]), &mut code);
                    pi += 7;
                    pit_sharp = sharp2(self.sharp);
                }
                _ => {
                    let index = next(&mut pi);
                    gain_pit = if bfi { self.ec_gain_p.conceal(self.state) } else { d_gain_pitch(mode, index) };
                    self.ec_gain_p.update(bfi, self.prev_bf, &mut gain_pit);
                    dec_10i40_35bits(parm.get(pi..pi + 10).unwrap_or(&[]), &mut code);
                    pi += 10;
                    pit_sharp = sharp2(gain_pit);
                }
            }
            let t0u = usize::try_from(t0).unwrap_or(0);
            for i in t0u..L_SUBFR {
                let temp = mult(code[i - t0u], pit_sharp);
                code[i] = add(code[i], temp);
            }

            let mut gain_code: i16;
            if mode == Mode::Mr475 {
                if even_subfr {
                    index_mr475 = next(&mut pi);
                }
                if !bfi {
                    let (gp, gc) = dec_gain(&mut self.pred, mode, index_mr475, &code, even_subfr);
                    gain_pit = gp;
                    gain_code = gc;
                } else {
                    gain_pit = self.ec_gain_p.conceal(self.state);
                    gain_code = self.ec_gain_c.conceal(&mut self.pred, self.state);
                }
                self.ec_gain_p.update(bfi, self.prev_bf, &mut gain_pit);
                self.ec_gain_c.update(bfi, self.prev_bf, &mut gain_code);
                pit_sharp = gain_pit.min(SHARPMAX);
            } else if mode <= Mode::Mr74 || mode == Mode::Mr102 {
                let index = next(&mut pi);
                if !bfi {
                    let (gp, gc) = dec_gain(&mut self.pred, mode, index, &code, even_subfr);
                    gain_pit = gp;
                    gain_code = gc;
                } else {
                    gain_pit = self.ec_gain_p.conceal(self.state);
                    gain_code = self.ec_gain_c.conceal(&mut self.pred, self.state);
                }
                self.ec_gain_p.update(bfi, self.prev_bf, &mut gain_pit);
                self.ec_gain_c.update(bfi, self.prev_bf, &mut gain_code);
                pit_sharp = gain_pit.min(SHARPMAX);
                if mode == Mode::Mr102 && self.old_t0 > (L_SUBFR as i16 + 5) {
                    pit_sharp >>= 2;
                }
            } else {
                let index = next(&mut pi);
                if mode == Mode::Mr795 {
                    gain_pit = if bfi { self.ec_gain_p.conceal(self.state) } else { d_gain_pitch(mode, index) };
                    self.ec_gain_p.update(bfi, self.prev_bf, &mut gain_pit);
                    let index = next(&mut pi);
                    gain_code = if !bfi { d_gain_code(&mut self.pred, mode, index, &code) } else { self.ec_gain_c.conceal(&mut self.pred, self.state) };
                    self.ec_gain_c.update(bfi, self.prev_bf, &mut gain_code);
                    pit_sharp = gain_pit.min(SHARPMAX);
                } else {
                    gain_code = if !bfi { d_gain_code(&mut self.pred, mode, index, &code) } else { self.ec_gain_c.conceal(&mut self.pred, self.state) };
                    self.ec_gain_c.update(bfi, self.prev_bf, &mut gain_code);
                    pit_sharp = gain_pit;
                }
            }
            if mode != Mode::Mr475 || !even_subfr {
                self.sharp = gain_pit.min(SHARPMAX);
            }
            pit_sharp = shl(pit_sharp, 1);
            let mut excp = [0i16; L_SUBFR];
            if pit_sharp > 16384 {
                for i in 0..L_SUBFR {
                    let temp = mult(self.old_exc[EXC_OFF + i], pit_sharp);
                    let mut l = l_mult(temp, gain_pit);
                    if mode == Mode::Mr122 {
                        l >>= 1;
                    }
                    excp[i] = round(l);
                }
            }
            if !bfi {
                self.ltp_gain_history.copy_within(1.., 0);
                self.ltp_gain_history[8] = gain_pit;
            }
            if (self.prev_bf || bfi) && self.in_background_noise && low_rate {
                if gain_pit > 12288 {
                    gain_pit = ((gain_pit - 12288) >> 1) + 12288;
                }
                if gain_pit > 14745 {
                    gain_pit = 14745;
                }
            }
            let mut lsf_i = [0i16; M];
            int_lsf(&prev_lsf, &self.lsf_state.past_lsf_q, i_subfr, &mut lsf_i);
            let flags = Flags {
                bfi,
                prev_bf: self.prev_bf,
                pdfi,
                prev_pdf: self.prev_pdf,
                in_background_noise: self.in_background_noise,
                voiced_hangover: self.voiced_hangover,
            };
            let mut gain_code_mix = self.cb_gain_aver.average(mode, gain_code, &lsf_i, &self.lsp_mean_save, flags);
            if mode > Mode::Mr67 && mode != Mode::Mr102 {
                gain_code_mix = gain_code;
            }
            let (pitch_fac, tmp_shift) = if mode <= Mode::Mr102 { (gain_pit, 1) } else { (gain_pit >> 1, 2) };
            let mut exc_enhanced = [0i16; L_SUBFR];
            for i in 0..L_SUBFR {
                let e = self.old_exc[EXC_OFF + i];
                exc_enhanced[i] = e;
                let mut l = l_mult(e, pitch_fac);
                l = l_mac(l, code[i], gain_code);
                l = l_shl(l, tmp_shift);
                self.old_exc[EXC_OFF + i] = round(l);
            }
            self.ph_disp.lock_full = false;
            if low_rate && self.voiced_hangover > 3 && self.in_background_noise && bfi {
                self.ph_disp.lock_full = true;
            }
            self.ph_disp.run(mode, &mut exc_enhanced, gain_code_mix, gain_pit, &mut code, pitch_fac, tmp_shift);
            let mut l = 0i32;
            for &e in &exc_enhanced {
                l = l_mac(l, e, e);
            }
            l >>= 1;
            let (root, exp) = post_sqrt(l);
            let l = l_shr(root, (exp >> 1) + 15);
            let exc_energy = (l >> 2) as i16;
            if low_rate && self.voiced_hangover > 5 && self.in_background_noise && self.state < 4 && ((pdfi && self.prev_pdf) || bfi || self.prev_bf) {
                let careful = pdfi && !bfi;
                ex_ctrl(&mut exc_enhanced, exc_energy, &self.exc_energy_hist, self.voiced_hangover, self.prev_bf, careful);
            }
            if !(self.in_background_noise && (bfi || self.prev_bf) && self.state < 4) {
                self.exc_energy_hist.copy_within(1.., 0);
                self.exc_energy_hist[8] = exc_energy;
            }
            let az = &a_t[sub_nr * MP1..(sub_nr + 1) * MP1];
            let out = &mut synth[i_subfr..i_subfr + L_SUBFR];
            if pit_sharp > 16384 {
                for i in 0..L_SUBFR {
                    excp[i] = add(excp[i], exc_enhanced[i]);
                }
                agc2(&exc_enhanced, &mut excp);
                syn_filt(az, &excp, out, &mut self.mem_syn, false);
            } else {
                syn_filt(az, &exc_enhanced, out, &mut self.mem_syn, false);
            }
            self.mem_syn.copy_from_slice(&synth[i_subfr + L_SUBFR - M..i_subfr + L_SUBFR]);
            self.old_exc.copy_within(L_SUBFR.., 0);
            self.old_t0 = t0;
        }
        self.in_background_noise = self.background.run(&self.ltp_gain_history, synth, &mut self.voiced_hangover);
        self.dtx.activity_update(&self.lsf_state.past_lsf_q, synth);
        self.prev_bf = bfi;
        self.prev_pdf = pdfi;
        lsp_avg(&mut self.lsp_mean_save, &self.lsf_state.past_lsf_q);
        self.dtx.global_state = new_state;
    }
}

fn post_sqrt(l: i32) -> (i32, i16) {
    math::sqrt_l_exp(l)
}

/// An AMR-NB decoder: RFC 4867 frames (table-of-contents byte plus payload) in, 160 samples of
/// 8 kHz audio out per frame.
#[derive(Clone, Debug)]
pub struct Decoder {
    dec: DecoderAmr,
    post: PostFilter,
    post_hp: PostProcess,
    prev_mode: Mode,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        Decoder { dec: DecoderAmr::new(), post: PostFilter::new(), post_hp: PostProcess::default(), prev_mode: Mode::Mr475 }
    }

    /// Bytes one frame occupies in a storage file or packet, including the table-of-contents
    /// byte, from that byte.
    pub fn frame_size(toc: u8) -> usize {
        1 + PAYLOAD_BYTES[usize::from((toc >> 3) & 15)]
    }

    /// Decodes one frame. `frame` starts with the table-of-contents byte; a shorter payload
    /// than the frame type needs is padded with zeros. Frame types 9–14 decode to silence
    /// without touching the decoder state, as in opencore.
    pub fn decode_frame(&mut self, frame: &[u8], out: &mut [i16; FRAME_SAMPLES]) -> Result<(), Error> {
        let toc = *frame.first().ok_or(Error::Truncated("AMR frame"))?;
        let ft = usize::from((toc >> 3) & 15);
        let payload = &frame[1..];
        let mut serial = [0i16; MAX_SERIAL_SIZE];
        let nbits = usize::try_from(at(&NUMOFBITS, ft as i32)).unwrap_or(0).min(MAX_SERIAL_SIZE);
        let bit = |i: usize| -> i16 { payload.get(i >> 3).map_or(0, |b| i16::from((b >> (7 - (i & 7))) & 1)) };
        let (mode, rx) = if ft <= 7 {
            let mode = Mode::from_index(ft as i32);
            let reorder = mode.reorder();
            for i in 0..nbits {
                if let Some(s) = reorder.get(i).and_then(|&dst| usize::try_from(dst).ok()).and_then(|d| serial.get_mut(d)) {
                    *s = bit(i);
                }
            }
            (mode, RxType::SpeechGood)
        } else if ft == 8 {
            for (i, s) in serial.iter_mut().enumerate().take(nbits) {
                *s = bit(i);
            }
            let mut m = 0i32;
            for i in 0..3 {
                m |= i32::from(serial[36 + i]) << i;
            }
            let rx = if serial[35] == 0 { RxType::SidFirst } else { RxType::SidUpdate };
            (Mode::from_index(m), rx)
        } else if ft < 15 {
            *out = [0; FRAME_SAMPLES];
            return Ok(());
        } else {
            (self.prev_mode, RxType::NoData)
        };
        self.decode_serial(mode, rx, &serial, out);
        self.prev_mode = mode;
        Ok(())
    }

    /// `GSMFrameDecode`: parameters, synthesis, post-filter, high-pass, 13-bit output.
    fn decode_serial(&mut self, mode: Mode, rx: RxType, serial: &[i16; MAX_SERIAL_SIZE], out: &mut [i16; FRAME_SAMPLES]) {
        let pmode = if matches!(rx, RxType::SidBad | RxType::SidUpdate) { Mode::MrDtx } else { mode };
        let mut parm = [0i16; MAX_PRM_SIZE + 1];
        let mut pos = 0usize;
        for (p, &bits) in parm.iter_mut().zip(pmode.bitno()) {
            let mut v: i16 = 0;
            for _ in 0..bits {
                v = (v << 1) | serial.get(pos).copied().unwrap_or(0);
                pos += 1;
            }
            *p = v;
        }
        let mut a_t = [0i16; 4 * MP1];
        self.dec.decode(mode, &parm, rx, out, &mut a_t);
        self.post.run(mode, out, &a_t);
        self.post_hp.run(out);
        for s in out.iter_mut() {
            *s &= !7;
        }
    }
}
