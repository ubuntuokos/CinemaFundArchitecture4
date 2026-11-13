// Derived from opencore-amr 0.1.6 (commit 7dba8c32), amr_nb/dec/src/dtx_dec.cpp.
// Copyright (C) 1998-2009 PacketVideo, Apache License 2.0; portions (C) 2004 3GPP
// Organizational Partners (3GPP TS 26.073). Ported to Rust for CFA4 (GPL-3.0-or-later).

//! Discontinuous transmission: comfort-noise generation from SID frames.

use super::codebook::{build_cn_code, pseudonoise};
use super::gain::{CbGainAverage, GcPred};
use super::lpc::{a_refl, lsf_lsp, lsp_az, lsp_lsf, reorder_lsf, syn_filt};
use super::math::{log2, pow2};
use super::{DPlsf, L_FRAME, L_SUBFR, LSF_GAP, M, MP1, Mode, RxType};
use crate::basic::{MAX_16, MIN_16, abs_s, add, div_s, l_add, l_mac, l_mult, l_shl, l_shr, l_sub, mult, shl, shr, sub};

const DTX_HIST_SIZE: usize = 8;
const DTX_MAX_EMPTY_THRESH: i16 = 50;
const DTX_ELAPSED_FRAMES_THRESH: i16 = 24 + 7 - 1;
const DTX_HANG_CONST: i16 = 7;
const PN_INITIAL_SEED: i32 = 0x7081_6958;
const LSF_HIST_MEAN_SCALE: [i16; M] = [20000, 20000, 20000, 20000, 20000, 18000, 16384, 8192, 0, 0];
const DTX_LOG_EN_ADJUST: [i16; 9] = [-1023, -878, -732, -586, -440, -294, -148, 0, 0];
const LSP_INIT: [i16; M] = [30000, 26000, 21000, 15000, 8000, 0, -8000, -15000, -21000, -26000];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DtxState {
    Speech,
    Dtx,
    DtxMute,
}

#[derive(Clone, Debug)]
pub(crate) struct DtxDec {
    since_last_sid: i16,
    true_sid_period_inv: i16,
    log_en: i16,
    old_log_en: i16,
    l_pn_seed_rx: i32,
    lsp: [i16; M],
    lsp_old: [i16; M],
    lsf_hist: [i16; M * DTX_HIST_SIZE],
    lsf_hist_ptr: usize,
    lsf_hist_mean: [i16; M * DTX_HIST_SIZE],
    log_pg_mean: i16,
    log_en_hist: [i16; DTX_HIST_SIZE],
    log_en_hist_ptr: usize,
    log_en_adjust: i16,
    dtx_hangover_count: i16,
    dec_ana_elapsed_count: i16,
    sid_frame: bool,
    valid_data: bool,
    dtx_hangover_added: bool,
    pub(crate) global_state: DtxState,
    data_updated: bool,
}

/// The SID parameters and synthesis state `dtx_dec` updates.
pub(crate) struct DtxTarget<'a> {
    pub(crate) mem_syn: &'a mut [i16; M],
    pub(crate) lsf_state: &'a mut DPlsf,
    pub(crate) pred: &'a mut GcPred,
    pub(crate) aver: &'a mut CbGainAverage,
}

impl DtxDec {
    pub(crate) fn new() -> Self {
        const HIST0: [i16; M] = [1384, 2077, 3420, 5108, 6742, 8122, 9863, 11092, 12714, 13701];
        let mut lsf_hist = [0i16; M * DTX_HIST_SIZE];
        for c in lsf_hist.chunks_exact_mut(M) {
            c.copy_from_slice(&HIST0);
        }
        DtxDec {
            since_last_sid: 0,
            true_sid_period_inv: 1 << 13,
            log_en: 3500,
            old_log_en: 3500,
            l_pn_seed_rx: PN_INITIAL_SEED,
            lsp: LSP_INIT,
            lsp_old: LSP_INIT,
            lsf_hist,
            lsf_hist_ptr: 0,
            lsf_hist_mean: [0; M * DTX_HIST_SIZE],
            log_pg_mean: 0,
            log_en_hist: [3500; DTX_HIST_SIZE],
            log_en_hist_ptr: 0,
            log_en_adjust: 0,
            dtx_hangover_count: DTX_HANG_CONST,
            dec_ana_elapsed_count: 32767,
            sid_frame: false,
            valid_data: false,
            dtx_hangover_added: false,
            global_state: DtxState::Dtx,
            data_updated: false,
        }
    }

    /// `dtx_dec`: one frame of comfort noise into `synth`, with the filters in `a_t`.
    pub(crate) fn decode(&mut self, t: DtxTarget<'_>, new_state: DtxState, mode: Mode, parm: &[i16], synth: &mut [i16; L_FRAME], a_t: &mut [i16; 4 * MP1]) {
        let p = |i: usize| parm.get(i).copied().unwrap_or(0);
        let mode_ix = (mode as usize).min(8);
        if self.dtx_hangover_added && self.sid_frame {
            self.log_en_adjust = DTX_LOG_EN_ADJUST[mode_ix];
            let mut ptr = self.lsf_hist_ptr + M;
            if ptr == 80 {
                ptr = 0;
            }
            self.lsf_hist.copy_within(self.lsf_hist_ptr..self.lsf_hist_ptr + M, ptr);
            let mut ptr = self.log_en_hist_ptr + 1;
            if ptr == DTX_HIST_SIZE {
                ptr = 0;
            }
            self.log_en_hist[ptr] = self.log_en_hist[self.log_en_hist_ptr];
            self.log_en = 0;
            let mut l_lsf = [0i32; M];
            for i in (0..DTX_HIST_SIZE).rev() {
                self.log_en = add(self.log_en, self.log_en_hist[i] >> 3);
                for j in (0..M).rev() {
                    l_lsf[j] = l_add(l_lsf[j], i32::from(self.lsf_hist[i * M + j]));
                }
            }
            let mut lsf = [0i16; M];
            for j in 0..M {
                lsf[j] = (l_lsf[j] >> 3) as i16;
            }
            lsf_lsp(&lsf, &mut self.lsp);
            self.log_en = sub(self.log_en, self.log_en_adjust);
            self.lsf_hist_mean = self.lsf_hist;
            for i in (0..M).rev() {
                let mut sum = 0i32;
                for j in (0..8).rev() {
                    sum = l_add(sum, i32::from(self.lsf_hist_mean[i + j * M]));
                }
                let mean = (sum >> 3) as i16;
                for j in (0..8).rev() {
                    let v = &mut self.lsf_hist_mean[i + j * M];
                    *v = sub(*v, mean);
                    *v = mult(*v, LSF_HIST_MEAN_SCALE[i]);
                    let negative = *v < 0;
                    *v = abs_s(*v);
                    if *v > 655 {
                        *v = 655 + ((*v - 655) >> 2);
                    }
                    if *v > 1310 {
                        *v = 1310;
                    }
                    if negative {
                        *v = v.wrapping_neg();
                    }
                }
            }
        }
        if self.sid_frame {
            self.lsp_old = self.lsp;
            self.old_log_en = self.log_en;
            if self.valid_data {
                let tmp_int_length = self.since_last_sid.min(32);
                self.since_last_sid = 0;
                let l = i32::from(tmp_int_length) << 10;
                let temp = if l != i32::from(l as i16) { if tmp_int_length > 0 { MAX_16 } else { MIN_16 } } else { l as i16 };
                self.true_sid_period_inv = if tmp_int_length >= 2 { div_s(1 << 10, temp) } else { 1 << 14 };
                t.lsf_state.init_3(p(0));
                t.lsf_state.decode_3(Mode::MrDtx, false, &[p(1), p(2), p(3)], &mut self.lsp);
                t.lsf_state.past_r_q = [0; M];
                let log_en_index = p(4);
                self.log_en = if !(-64..=63).contains(&log_en_index) { if log_en_index > 0 { MAX_16 } else { MIN_16 } } else { log_en_index << (11 - 2) };
                self.log_en = self.log_en.wrapping_sub(2560 * 2);
                if log_en_index == 0 {
                    self.log_en = MIN_16;
                }
                if !self.data_updated || self.global_state == DtxState::Speech {
                    self.lsp_old = self.lsp;
                    self.old_log_en = self.log_en;
                }
            }
            let mut ma = (self.log_en >> 1).wrapping_sub(9000);
            ma = ma.clamp(-14436, 0);
            t.pred.past_qua_en = [ma; 4];
            let ma = ((i32::from(ma) * 5443) >> 15) as i16;
            t.pred.past_qua_en_mr122 = [ma; 4];
        }
        let adj = DTX_LOG_EN_ADJUST[mode_ix];
        let temp = (((i32::from(adj) << 5) * 3277) >> 15) as i16;
        let temp = temp >> 5;
        self.log_en_adjust = add(((i32::from(self.log_en_adjust) * 29491) >> 15) as i16, temp);

        let mut int_fac = shl(self.since_last_sid.wrapping_add(1), 10);
        int_fac = mult(int_fac, self.true_sid_period_inv);
        int_fac = if int_fac > 1024 {
            16384
        } else if int_fac < -2048 {
            MIN_16
        } else {
            int_fac << 4
        };
        let mut l_log_en_int = l_mult(int_fac, self.log_en);
        let mut lsp_int = [0i16; M];
        for i in (0..M).rev() {
            lsp_int[i] = mult(int_fac, self.lsp[i]);
        }
        int_fac = 16384i16.wrapping_sub(int_fac);
        l_log_en_int = l_mac(l_log_en_int, int_fac, self.old_log_en);
        for i in (0..M).rev() {
            lsp_int[i] = add(lsp_int[i], mult(int_fac, self.lsp_old[i]));
            let l = i32::from(lsp_int[i]) << 1;
            lsp_int[i] = if l != i32::from(l as i16) { if lsp_int[i] > 0 { MAX_16 } else { MIN_16 } } else { l as i16 };
        }
        let mut lsf_variab_factor = self.log_pg_mean.wrapping_sub(2457);
        lsf_variab_factor = 4096i16.wrapping_sub(mult(lsf_variab_factor, 9830));
        lsf_variab_factor = if lsf_variab_factor > 4095 {
            MAX_16
        } else if lsf_variab_factor < 0 {
            0
        } else {
            lsf_variab_factor << 3
        };
        let lsf_variab_index = pseudonoise(&mut self.l_pn_seed_rx, 3) as usize;
        let mut lsf_int = [0i16; M];
        lsp_lsf(&lsp_int, &mut lsf_int);
        let mut lsf_int_variab = lsf_int;
        for i in (0..M).rev() {
            let h = self.lsf_hist_mean.get(i + lsf_variab_index * M).copied().unwrap_or(0);
            lsf_int_variab[i] = add(lsf_int_variab[i], mult(lsf_variab_factor, h));
        }
        reorder_lsf(&mut lsf_int, LSF_GAP);
        reorder_lsf(&mut lsf_int_variab, LSF_GAP);
        t.lsf_state.past_lsf_q = lsf_int;
        let mut lsp_int_variab = [0i16; M];
        lsf_lsp(&lsf_int, &mut lsp_int);
        lsf_lsp(&lsf_int_variab, &mut lsp_int_variab);
        let mut acoeff = [0i16; MP1];
        let mut acoeff_variab = [0i16; MP1];
        lsp_az(&lsp_int, &mut acoeff);
        lsp_az(&lsp_int_variab, &mut acoeff_variab);
        for c in a_t.chunks_exact_mut(MP1) {
            c.copy_from_slice(&acoeff);
        }
        let mut refl = [0i16; M];
        a_refl(&acoeff[1..], &mut refl);
        let mut pred_err = MAX_16;
        for &r in &refl {
            let l = (i32::from(r) * i32::from(r)) >> 15;
            let temp = if l <= 0x7fff { MAX_16.wrapping_sub(l as i16) } else { 0 };
            pred_err = mult(pred_err, temp);
        }
        let (log_pg_e, log_pg_m) = log2(i32::from(pred_err));
        let mut log_pg = shl(log_pg_e.wrapping_sub(15), 12);
        log_pg = shr(sub(0, add(log_pg, shr(log_pg_m, 15 - 12))), 1);
        self.log_pg_mean = add(mult(29491, self.log_pg_mean), mult(3277, log_pg));
        l_log_en_int = l_shr(l_log_en_int, 10);
        l_log_en_int = l_add(l_log_en_int, 4 * 65536);
        l_log_en_int = l_sub(l_log_en_int, l_shl(i32::from(log_pg), 4));
        l_log_en_int = l_add(l_log_en_int, l_shl(i32::from(self.log_en_adjust), 5));
        let e = (l_log_en_int >> 16) as i16;
        let m = l_shr(l_sub(l_log_en_int, i32::from(e) << 16), 1) as i16;
        let level = pow2(e, m) as i16;
        for i in 0..4 {
            let mut ex = [0i16; L_SUBFR];
            build_cn_code(&mut self.l_pn_seed_rx, &mut ex);
            for v in ex.iter_mut() {
                *v = mult(level, *v);
            }
            syn_filt(&acoeff_variab, &ex, &mut synth[i * L_SUBFR..(i + 1) * L_SUBFR], t.mem_syn, true);
        }
        t.aver.hang_var = 20;
        t.aver.hang_count = 0;
        if new_state == DtxState::DtxMute {
            let mut len = self.since_last_sid;
            if len > 32 {
                len = 32;
            } else if len <= 0 {
                len = 8;
            }
            let l = i32::from(len) << 10;
            let temp = if l != i32::from(l as i16) { MAX_16 } else { l as i16 };
            self.true_sid_period_inv = div_s(1 << 10, temp);
            self.since_last_sid = 0;
            self.lsp_old = self.lsp;
            self.old_log_en = self.log_en;
            self.log_en = self.log_en.wrapping_sub(256).max(0);
        }
        if self.sid_frame && (self.valid_data || self.dtx_hangover_added) {
            self.since_last_sid = 0;
            self.data_updated = true;
        }
    }

    /// `dtx_dec_activity_update`: records the LSFs and energy of a speech frame.
    pub(crate) fn activity_update(&mut self, lsf: &[i16; M], frame: &[i16; L_FRAME]) {
        self.lsf_hist_ptr += M;
        if self.lsf_hist_ptr == 80 {
            self.lsf_hist_ptr = 0;
        }
        self.lsf_hist[self.lsf_hist_ptr..self.lsf_hist_ptr + M].copy_from_slice(lsf);
        let mut en = 0i32;
        for &v in frame.iter().rev() {
            let p = i32::from(v) * i32::from(v);
            en = l_add(en, if p != 0x4000_0000 { p << 1 } else { i32::MAX });
        }
        let (e, m) = log2(en);
        let l = i32::from(e) << 10;
        let e = if l != i32::from(l as i16) { if e > 0 { MAX_16 } else { MIN_16 } } else { l as i16 };
        let log_en = e.wrapping_add(m >> 5).wrapping_sub(7497 + 1024);
        self.log_en_hist_ptr += 1;
        if self.log_en_hist_ptr == DTX_HIST_SIZE {
            self.log_en_hist_ptr = 0;
        }
        self.log_en_hist[self.log_en_hist_ptr] = log_en;
    }

    /// `rx_dtx_handler`: the DTX state of this frame.
    pub(crate) fn rx_handler(&mut self, ft: RxType) -> DtxState {
        use RxType::*;
        let in_dtx = matches!(self.global_state, DtxState::Dtx | DtxState::DtxMute);
        let new_state = if matches!(ft, SidFirst | SidUpdate | SidBad) || (in_dtx && matches!(ft, NoData | SpeechBad | Onset)) {
            let mut s = DtxState::Dtx;
            if self.global_state == DtxState::DtxMute && matches!(ft, SidBad | SidFirst | Onset | NoData) {
                s = DtxState::DtxMute;
            }
            self.since_last_sid = self.since_last_sid.wrapping_add(1);
            if ft != SidUpdate && self.since_last_sid > DTX_MAX_EMPTY_THRESH {
                s = DtxState::DtxMute;
            }
            s
        } else {
            self.since_last_sid = 0;
            DtxState::Speech
        };
        if !self.data_updated && ft == SidUpdate {
            self.dec_ana_elapsed_count = 0;
        }
        self.dec_ana_elapsed_count = add(self.dec_ana_elapsed_count, 1);
        self.dtx_hangover_added = false;
        let enc_dtx = matches!(ft, SidFirst | SidUpdate | SidBad | Onset | NoData) && !(ft == NoData && new_state == DtxState::Speech);
        if !enc_dtx {
            self.dtx_hangover_count = DTX_HANG_CONST;
        } else if self.dec_ana_elapsed_count > DTX_ELAPSED_FRAMES_THRESH {
            self.dtx_hangover_added = true;
            self.dec_ana_elapsed_count = 0;
            self.dtx_hangover_count = 0;
        } else if self.dtx_hangover_count == 0 {
            self.dec_ana_elapsed_count = 0;
        } else {
            self.dtx_hangover_count -= 1;
        }
        if new_state != DtxState::Speech {
            self.sid_frame = false;
            self.valid_data = false;
            match ft {
                SidFirst => self.sid_frame = true,
                SidUpdate => {
                    self.sid_frame = true;
                    self.valid_data = true;
                }
                SidBad => {
                    self.sid_frame = true;
                    self.dtx_hangover_added = false;
                }
                _ => {}
            }
        }
        new_state
    }
}
