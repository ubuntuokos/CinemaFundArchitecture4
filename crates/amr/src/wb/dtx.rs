// Derived from opencore-amr 0.1.6 (commit 7dba8c32), amr_wb/dec/src: dtx_decoder_amr_wb.cpp.
// Copyright (C) 1998-2009 PacketVideo, Apache License 2.0; portions (C) 2007 3GPP
// Organizational Partners (3GPP TS 26.173). Ported to Rust for CFA4 (GPL-3.0-or-later).

//! AMR-WB discontinuous transmission: comfort-noise generation between SID frames.

use super::codebook::disf_ns;
use super::op::{Bits, add, div_16by16, dot_product12, l_add, l_mac, l_mult, l_sub, log2, mult, mult_r, noise, norm, one_ov_sqrt_norm, power_of_2, shl, sub};
use super::{L_FRAME, M};

pub(crate) const SPEECH: i16 = 0;
pub(crate) const DTX: i16 = 1;
pub(crate) const DTX_MUTE: i16 = 2;

/// Receive frame types (`RX_*`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rx {
    SpeechGood,
    SpeechLost,
    SidFirst,
    SidUpdate,
    NoData,
}

const HIST: usize = 8;
const RANDOM_INITSEED: i16 = 21845;

#[derive(Clone)]
pub(crate) struct DtxDec {
    since_last_sid: i16,
    true_sid_period_inv: i16,
    log_en: i16,
    old_log_en: i16,
    isf: [i16; M],
    isf_old: [i16; M],
    cng_seed: i16,
    isf_hist: [[i16; M]; HIST],
    log_en_hist: [i16; HIST],
    hist_ptr: usize,
    hangover_count: i16,
    elapsed_count: i16,
    sid_frame: bool,
    valid_data: bool,
    hangover_added: bool,
    pub(crate) global_state: i16,
    data_updated: bool,
    dither_seed: i16,
    cn_dith: bool,
}

impl DtxDec {
    pub(crate) fn new(isf_init: &[i16; M]) -> Self {
        Self {
            since_last_sid: 0,
            true_sid_period_inv: 1 << 13,
            log_en: 3500,
            old_log_en: 3500,
            isf: *isf_init,
            isf_old: *isf_init,
            cng_seed: RANDOM_INITSEED,
            isf_hist: [*isf_init; HIST],
            log_en_hist: [3500; HIST],
            hist_ptr: 0,
            hangover_count: 7,
            elapsed_count: 32767,
            sid_frame: false,
            valid_data: false,
            hangover_added: false,
            global_state: SPEECH,
            data_updated: false,
            dither_seed: RANDOM_INITSEED,
            cn_dith: false,
        }
    }

    /// `dtx_dec_amr_wb`: comfort-noise ISFs into `isf` and excitation into `exc2`.
    pub(crate) fn decode(&mut self, exc2: &mut [i16; L_FRAME], new_state: i16, isf: &mut [i16; M], prms: &mut Bits<'_>) {
        if self.hangover_added && self.sid_frame {
            let ptr = (self.hist_ptr + 1) % HIST;
            self.isf_hist[ptr] = self.isf_hist[self.hist_ptr];
            self.log_en_hist[ptr] = self.log_en_hist[self.hist_ptr];
            let mut log_en: i16 = 0;
            let mut l_isf = [0i32; M];
            for i in 0..HIST {
                log_en = add(log_en, self.log_en_hist[i]);
                for j in 0..M {
                    l_isf[j] = l_add(l_isf[j], i32::from(self.isf_hist[i][j]));
                }
            }
            log_en = (log_en >> 1).wrapping_add(1024);
            if log_en < 0 {
                log_en = 0;
            }
            self.log_en = log_en;
            for j in 0..M {
                self.isf[j] = (l_isf[j] >> 3) as i16;
            }
        }
        if self.sid_frame {
            self.isf_old = self.isf;
            self.old_log_en = self.log_en;
            if self.valid_data {
                let len = self.since_last_sid.min(32);
                self.true_sid_period_inv = if len >= 2 { div_16by16(1 << 10, shl(len, 10)) } else { 1 << 14 };
                let mut ind = [0i16; 5];
                for (k, n) in [6, 6, 6, 5, 5].into_iter().enumerate() {
                    ind[k] = prms.parm(n);
                }
                disf_ns(&ind, &mut self.isf);
                let log_en_index = prms.parm(6);
                self.cn_dith = prms.bit1() != 0;
                self.log_en = mult(shl(log_en_index, 15 - 6), 12483);
                if !self.data_updated || self.global_state == SPEECH {
                    self.isf_old = self.isf;
                    self.old_log_en = self.log_en;
                }
            }
        }
        if self.sid_frame && self.valid_data {
            self.since_last_sid = 0;
        }
        let mut int_fac = shl(self.since_last_sid, 10);
        int_fac = mult(int_fac, self.true_sid_period_inv);
        if int_fac > 1024 {
            int_fac = 1024;
        }
        int_fac = shl(int_fac, 4);
        let mut l_log_en_int = l_mult(int_fac, self.log_en);
        for i in 0..M {
            isf[i] = mult(int_fac, self.isf[i]);
        }
        let int_fac = 16384 - int_fac;
        l_log_en_int = l_mac(l_log_en_int, int_fac, self.old_log_en);
        for i in 0..M {
            isf[i] = add(isf[i], mult(int_fac, self.isf_old[i]));
            isf[i] = shl(isf[i], 1);
        }
        if self.cn_dith {
            cn_dithering(isf, &mut l_log_en_int, &mut self.dither_seed);
        }
        l_log_en_int >>= 9;
        let e = (l_log_en_int >> 16) as i16;
        let m = (l_sub(l_log_en_int, i32::from(e) << 16) >> 1) as i16;
        let level32 = power_of_2(e.wrapping_add(15), m);
        let exp0 = norm(level32);
        let level = (level32.wrapping_shl(exp0 as u32) >> 16) as i16;
        let exp0 = 15 - exp0;
        for v in exc2.iter_mut() {
            *v = noise(&mut self.cng_seed) >> 4;
        }
        let mut exp = 0;
        let mut ener32 = dot_product12(exc2, exc2, &mut exp);
        one_ov_sqrt_norm(&mut ener32, &mut exp);
        let gain = mult(level, (ener32 >> 16) as i16);
        let exp = exp.wrapping_add(exp0).wrapping_add(4);
        for v in exc2.iter_mut() {
            *v = shl(mult(*v, gain), exp);
        }
        if new_state == DTX_MUTE {
            let len = self.since_last_sid.min(32);
            self.true_sid_period_inv = div_16by16(1 << 10, shl(len, 10));
            self.since_last_sid = 0;
            self.old_log_en = self.log_en;
            self.log_en = self.log_en.wrapping_sub(64);
            if self.log_en < 0 {
                self.log_en = 0;
            }
        }
        if self.sid_frame && (self.valid_data || self.hangover_added) {
            self.since_last_sid = 0;
            self.data_updated = true;
        }
    }

    /// `dtx_dec_amr_wb_activity_update`.
    pub(crate) fn activity_update(&mut self, isf: &[i16; M], exc: &[i16]) {
        self.hist_ptr = (self.hist_ptr + 1) % HIST;
        self.isf_hist[self.hist_ptr] = *isf;
        let mut en: i32 = 0;
        for &v in exc.iter().take(L_FRAME) {
            en = l_mac(en, v, v);
        }
        let (e, m) = log2(en >> 1);
        let log_en = shl(e, 7).wrapping_add(m >> 8).wrapping_sub(1024);
        self.log_en_hist[self.hist_ptr] = log_en;
    }

    /// `rx_amr_wb_dtx_handler`: the new DTX state for a received frame type.
    pub(crate) fn rx_handler(&mut self, ft: Rx) -> i16 {
        let in_dtx = self.global_state == DTX || self.global_state == DTX_MUTE;
        let sid = matches!(ft, Rx::SidFirst | Rx::SidUpdate);
        let new_state = if sid || (in_dtx && matches!(ft, Rx::NoData | Rx::SpeechLost)) {
            let mut s = DTX;
            if self.global_state == DTX_MUTE && matches!(ft, Rx::SidFirst | Rx::SpeechLost | Rx::NoData) {
                s = DTX_MUTE;
            }
            self.since_last_sid = add(self.since_last_sid, 1);
            if self.since_last_sid > 50 {
                s = DTX_MUTE;
            }
            s
        } else {
            self.since_last_sid = 0;
            SPEECH
        };
        if !self.data_updated && ft == Rx::SidUpdate {
            self.elapsed_count = 0;
        }
        self.elapsed_count = add(self.elapsed_count, 1);
        self.hangover_added = false;
        let enc_dtx = matches!(ft, Rx::SidFirst | Rx::SidUpdate | Rx::NoData);
        if !enc_dtx {
            self.hangover_count = 7;
        } else if self.elapsed_count > 30 {
            self.hangover_added = true;
            self.elapsed_count = 0;
            self.hangover_count = 0;
        } else if self.hangover_count == 0 {
            self.elapsed_count = 0;
        } else {
            self.hangover_count -= 1;
        }
        if new_state != SPEECH {
            self.sid_frame = false;
            self.valid_data = false;
            match ft {
                Rx::SidFirst => self.sid_frame = true,
                Rx::SidUpdate => {
                    self.sid_frame = true;
                    self.valid_data = true;
                }
                _ => {}
            }
        }
        new_state
    }
}

fn cn_dithering(isf: &mut [i16; M], l_log_en_int: &mut i32, seed: &mut i16) {
    const GAIN_FACTOR: i16 = 75;
    const ISF_GAP: i16 = 128;
    const ISF_DITH_GAP: i16 = 448;
    let rand = |seed: &mut i16| (noise(seed) >> 1).wrapping_add(noise(seed) >> 1);
    let r = rand(seed);
    *l_log_en_int = l_add(*l_log_en_int, l_mult(r, GAIN_FACTOR));
    if *l_log_en_int < 0 {
        *l_log_en_int = 0;
    }
    let mut dither_fac: i16 = 256;
    let r = rand(seed);
    let temp = add(isf[0], mult_r(r, dither_fac));
    isf[0] = if temp < ISF_GAP { ISF_GAP } else { temp };
    for i in 1..M - 1 {
        dither_fac = add(dither_fac, 2);
        let r = rand(seed);
        let temp = add(isf[i], mult_r(r, dither_fac));
        let temp1 = sub(temp, isf[i - 1]);
        isf[i] = if temp1 < ISF_DITH_GAP { isf[i - 1].wrapping_add(ISF_DITH_GAP) } else { temp };
    }
    if isf[M - 2] > 16384 {
        isf[M - 2] = 16384;
    }
}
