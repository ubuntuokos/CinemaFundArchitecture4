//! RealAudio SIPR (ACELP.NET, `sipr`): the 5.0, 6.5 and 8.5 kbit/s ACELP modes at 8 kHz and the
//! 16 kbit/s mode at 16 kHz. A port of FFmpeg n7.1.1 `libavcodec/sipr.c` and `sipr16k.c`
//! (Copyright (c) 2008 Vladimir Voroshilov, 2009 Vitor Sessak; LGPL-2.1-or-later), keeping its
//! `f32`/`f64` operation order so the samples match FFmpeg's bit for bit.

use std::f64::consts::{LN_2, LN_10, PI};

use crate::celp::{
    Fixed, adaptive_gain_control, amr_set_fixed_gain, amrwb_lsp2lpc, decode_10_pulses_35bits, decode_pitch_lag, dot, interpolate, lp_synthesis,
    lp_zero_synthesis, lspd2lpc, order_2_transfer, set_fixed_vector, set_min_dist_lsf, sort_nearly_sorted, tilt_compensation, weighted_sum_in_place,
};
use crate::tables_sipr::*;
use crate::{Error, Result};

const LP_ORDER: usize = 10;
const LP_ORDER_16K: usize = 16;
const L_SUBFR_16K: usize = 80;
const PITCH_MIN: i32 = 30;
const PITCH_MAX: usize = 281;
const L_INTERPOL: usize = LP_ORDER + 1;
const SUBFR: usize = 48;
const PITCH_DELAY_MAX: usize = 143;
const LSFQ_DIFF_MIN: f64 = 0.0125 * PI;

/// `ff_fc_4pulses_8bits_tracks_13`.
const TRACKS_13: [u8; 16] = [0, 5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55, 60, 65, 70, 75];

/// The four SIPR modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// 16 kbit/s, 16 kHz, 20-byte frames.
    K16,
    /// 8.5 kbit/s, 19-byte frames.
    K8_5,
    /// 6.5 kbit/s, 29-byte frames.
    K6_5,
    /// 5.0 kbit/s, 37-byte frames.
    K5_0,
}

struct ModeParam {
    bits_per_frame: usize,
    subframe_count: usize,
    frames_per_packet: usize,
    pitch_sharp_factor: f32,
    number_of_fc_indexes: usize,
    ma_predictor_bits: u32,
    vq_indexes_bits: [u32; 5],
    pitch_delay_bits: [u32; 5],
    gp_index_bits: u32,
    fc_index_bits: [u32; 10],
    gc_index_bits: u32,
}

const fn mode_param(m: Mode) -> ModeParam {
    match m {
        Mode::K16 => ModeParam {
            bits_per_frame: 160,
            subframe_count: 2,
            frames_per_packet: 1,
            pitch_sharp_factor: 0.0,
            number_of_fc_indexes: 10,
            ma_predictor_bits: 1,
            vq_indexes_bits: [7, 8, 7, 7, 7],
            pitch_delay_bits: [9, 6, 0, 0, 0],
            gp_index_bits: 4,
            fc_index_bits: [4, 5, 4, 5, 4, 5, 4, 5, 4, 5],
            gc_index_bits: 5,
        },
        Mode::K8_5 => ModeParam {
            bits_per_frame: 152,
            subframe_count: 3,
            frames_per_packet: 1,
            pitch_sharp_factor: 0.8,
            number_of_fc_indexes: 3,
            ma_predictor_bits: 0,
            vq_indexes_bits: [6, 7, 7, 7, 5],
            pitch_delay_bits: [8, 5, 5, 0, 0],
            gp_index_bits: 0,
            fc_index_bits: [9, 9, 9, 0, 0, 0, 0, 0, 0, 0],
            gc_index_bits: 7,
        },
        Mode::K6_5 => ModeParam {
            bits_per_frame: 232,
            subframe_count: 3,
            frames_per_packet: 2,
            pitch_sharp_factor: 0.8,
            number_of_fc_indexes: 3,
            ma_predictor_bits: 0,
            vq_indexes_bits: [6, 7, 7, 7, 5],
            pitch_delay_bits: [8, 5, 5, 0, 0],
            gp_index_bits: 0,
            fc_index_bits: [5, 5, 5, 0, 0, 0, 0, 0, 0, 0],
            gc_index_bits: 7,
        },
        Mode::K5_0 => ModeParam {
            bits_per_frame: 296,
            subframe_count: 5,
            frames_per_packet: 2,
            pitch_sharp_factor: 0.85,
            number_of_fc_indexes: 1,
            ma_predictor_bits: 0,
            vq_indexes_bits: [6, 7, 7, 7, 5],
            pitch_delay_bits: [8, 5, 8, 5, 5],
            gp_index_bits: 0,
            fc_index_bits: [10, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            gc_index_bits: 7,
        },
    }
}

/// `SiprParameters`.
#[derive(Default)]
struct Params {
    ma_pred_switch: usize,
    vq_indexes: [usize; 5],
    pitch_delay: [i32; 5],
    gp_index: [usize; 5],
    fc_indexes: [[i16; 10]; 5],
    gc_index: [usize; 5],
}

/// A little-endian bit reader (`BITSTREAM_READER_LE`) over the first `limit` bits; reads past it
/// give zeros.
struct LeBits<'a> {
    data: &'a [u8],
    pos: usize,
    limit: usize,
}

impl LeBits<'_> {
    fn get(&mut self, n: u32) -> u32 {
        let mut v = 0u32;
        for j in 0..n {
            let bit = if self.pos < self.limit { self.data.get(self.pos >> 3).map_or(0, |b| (b >> (self.pos & 7)) & 1) } else { 0 };
            v |= u32::from(bit) << j;
            self.pos += 1;
        }
        v
    }
}

fn decode_parameters(b: &mut LeBits<'_>, p: &ModeParam) -> Params {
    let mut q = Params::default();
    if p.ma_predictor_bits != 0 {
        q.ma_pred_switch = b.get(p.ma_predictor_bits) as usize;
    }
    for i in 0..5 {
        q.vq_indexes[i] = b.get(p.vq_indexes_bits[i]) as usize;
    }
    for i in 0..p.subframe_count {
        q.pitch_delay[i] = b.get(p.pitch_delay_bits[i]) as i32;
        if p.gp_index_bits != 0 {
            q.gp_index[i] = b.get(p.gp_index_bits) as usize;
        }
        for j in 0..p.number_of_fc_indexes {
            q.fc_indexes[i][j] = b.get(p.fc_index_bits[j]) as i16;
        }
        q.gc_index[i] = b.get(p.gc_index_bits) as usize;
    }
    q
}

/// Copies `stride` floats of entry `idx` of `cb` into `out` (zeros past the table's end).
fn cb_entry(out: &mut [f32], cb: &[f32], idx: usize, stride: usize) {
    for (k, o) in out.iter_mut().enumerate().take(stride) {
        *o = cb.get(idx * stride + k).copied().unwrap_or(0.0);
    }
}

/// Decoder state (`SiprContext`).
#[derive(Clone)]
pub struct Decoder {
    mode: Mode,
    past_pitch_gain: f32,
    lsf_history: [f32; LP_ORDER_16K],
    excitation: [f32; L_INTERPOL + PITCH_MAX + 2 * L_SUBFR_16K],
    synth_buf: [f32; LP_ORDER + 5 * SUBFR + 6],
    lsp_history: [f32; LP_ORDER],
    gain_mem: f32,
    energy_history: [f32; 4],
    highpass_filt_mem: [f32; 2],
    postfilter_mem: [f32; LP_ORDER],
    tilt_mem: f32,
    postfilter_agc: f32,
    postfilter_mem5k0: [f32; LP_ORDER],
    postfilter_syn5k0: [f32; LP_ORDER + SUBFR * 5],
    pitch_lag_prev: i32,
    iir_mem: [f32; LP_ORDER_16K + 1],
    filt_buf: [[f32; LP_ORDER_16K + 1]; 2],
    /// Which of `filt_buf` is FFmpeg's `filt_mem[0]`.
    filt_first: usize,
    mem_preemph: [f32; LP_ORDER_16K],
    synth: [f32; LP_ORDER_16K],
    lsp_history_16k: [f64; 16],
}

impl Decoder {
    /// The mode follows the frame size (`block_align`: 20, 19, 29 or 37 bytes).
    pub fn new(block_align: usize) -> Result<Self> {
        let mode = match block_align {
            20 => Mode::K16,
            19 => Mode::K8_5,
            29 => Mode::K6_5,
            37 => Mode::K5_0,
            _ => return Err(Error::Unsupported("SIPR frame size")),
        };
        let mut lsp_history = [0f32; LP_ORDER];
        for (i, v) in lsp_history.iter_mut().enumerate() {
            *v = ((i + 1) as f64 * PI / (LP_ORDER + 1) as f64).cos() as f32;
        }
        let mut lsp_history_16k = [0f64; 16];
        for (i, v) in lsp_history_16k.iter_mut().enumerate() {
            *v = ((i + 1) as f64 * PI / (LP_ORDER_16K + 1) as f64).cos();
        }
        Ok(Self {
            mode,
            past_pitch_gain: 0.0,
            lsf_history: [0.0; LP_ORDER_16K],
            excitation: [0.0; L_INTERPOL + PITCH_MAX + 2 * L_SUBFR_16K],
            synth_buf: [0.0; LP_ORDER + 5 * SUBFR + 6],
            lsp_history,
            gain_mem: 0.0,
            energy_history: [-14.0; 4],
            highpass_filt_mem: [0.0; 2],
            postfilter_mem: [0.0; LP_ORDER],
            tilt_mem: 0.0,
            postfilter_agc: 0.0,
            postfilter_mem5k0: [0.0; LP_ORDER],
            postfilter_syn5k0: [0.0; LP_ORDER + SUBFR * 5],
            pitch_lag_prev: 180,
            iir_mem: [0.0; LP_ORDER_16K + 1],
            filt_buf: [[0.0; LP_ORDER_16K + 1]; 2],
            filt_first: 0,
            mem_preemph: [0.0; LP_ORDER_16K],
            synth: [0.0; LP_ORDER_16K],
            lsp_history_16k,
        })
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Bytes per frame.
    pub fn frame_size(&self) -> usize {
        mode_param(self.mode).bits_per_frame >> 3
    }

    /// Samples one frame decodes to (16 kHz for [`Mode::K16`], 8 kHz otherwise).
    pub fn frame_samples(&self) -> usize {
        let p = mode_param(self.mode);
        let sub = if self.mode == Mode::K16 { L_SUBFR_16K } else { SUBFR };
        p.frames_per_packet * sub * p.subframe_count
    }

    /// Decodes one frame of [`Self::frame_size`] bytes into [`Self::frame_samples`] `f32` samples.
    pub fn decode(&mut self, frame: &[u8]) -> Result<Vec<f32>> {
        let p = mode_param(self.mode);
        if frame.len() < p.bits_per_frame >> 3 {
            return Err(Error::Invalid("SIPR frame too short"));
        }
        let mut out = vec![0f32; self.frame_samples()];
        let mut b = LeBits { data: frame, pos: 0, limit: p.bits_per_frame };
        let per = out.len() / p.frames_per_packet;
        for chunk in out.chunks_exact_mut(per) {
            let parm = decode_parameters(&mut b, &p);
            if self.mode == Mode::K16 {
                self.frame_16k(&parm, chunk);
            } else {
                self.frame(&parm, &p, chunk);
            }
        }
        Ok(out)
    }

    /// `lsf_decode_fp`.
    fn lsf_decode(&mut self, parm: &Params) -> [f32; LP_ORDER] {
        let mut tmp = [0f32; LP_ORDER];
        let cbs: [&[f32]; 5] = [&LSF_CB1, &LSF_CB2, &LSF_CB3, &LSF_CB4, &LSF_CB5];
        for i in 0..5 {
            cb_entry(&mut tmp[2 * i..2 * i + 2], cbs[i], parm.vq_indexes[i], 2);
        }
        let mut new = [0f32; LP_ORDER];
        for i in 0..LP_ORDER {
            new[i] = (f64::from(self.lsf_history[i]) * 0.33 + f64::from(tmp[i]) + f64::from(MEAN_LSF[i])) as f32;
        }
        sort_nearly_sorted(&mut new[..LP_ORDER - 1]);
        set_min_dist_lsf(&mut new[..LP_ORDER - 1], LSFQ_DIFF_MIN);
        let cap = 1.3 * PI;
        new[9] = if f64::from(new[9]) > cap { cap as f32 } else { new[9] };
        self.lsf_history[..LP_ORDER].copy_from_slice(&tmp);
        for v in &mut new[..LP_ORDER - 1] {
            *v = f64::from(*v).cos() as f32;
        }
        new[9] = (f64::from(new[9]) * (6.153848 / PI)) as f32;
        new
    }

    /// `decode_frame` (5.0, 6.5 and 8.5 kbit/s).
    fn frame(&mut self, parm: &Params, p: &ModeParam, out: &mut [f32]) {
        let n_sub = p.subframe_count;
        let frame_size = n_sub * SUBFR;
        let lsf_new = self.lsf_decode(parm);

        // sipr_decode_lp
        let mut az = [0f32; LP_ORDER * 5];
        let t0 = (1.0 / n_sub as f64) as f32;
        let mut t = t0 * 0.5;
        for i in 0..n_sub {
            let mut lsfint = [0f64; LP_ORDER];
            for j in 0..LP_ORDER {
                lsfint[j] = f64::from(self.lsp_history[j] * (1.0 - t) + t * lsf_new[j]);
            }
            amrwb_lsp2lpc(&lsfint, &mut az[i * LP_ORDER..(i + 1) * LP_ORDER], LP_ORDER);
            t += t0;
        }
        self.lsp_history = lsf_new;

        let five = self.mode == Mode::K5_0;
        let synth0 = 16;
        let mut exc = PITCH_DELAY_MAX + L_INTERPOL;
        let mut t0_first = 0;
        let mut ir = [0f32; SUBFR + LP_ORDER];
        for i in 0..n_sub {
            let paz = &az[i * LP_ORDER..(i + 1) * LP_ORDER];
            let (lag, frac) = decode_pitch_lag(parm.pitch_delay[i], t0_first, i, five, 6);
            if i == 0 || (i == 2 && five) {
                t0_first = lag;
            }
            let lag_u = lag.max(0) as usize;
            let src = (exc + usize::from(frac <= 0)).saturating_sub(lag_u).max(LP_ORDER);
            let fpos = (2 * ((2 + frac).rem_euclid(3) + 1)) as usize;
            interpolate(&mut self.excitation, exc, src, &B60_SINC, 6, fpos, LP_ORDER, SUBFR);

            let fixed = self.fixed_sparse(&parm.fc_indexes[i], f64::from(self.past_pitch_gain) < 0.8);

            // eval_ir
            let mut tmp1 = [0f32; SUBFR];
            let mut tmp2 = [0f32; LP_ORDER];
            tmp1[0] = 1.0;
            for k in 0..LP_ORDER {
                tmp1[k + 1] = paz[k] * POW_0_55[k];
                tmp2[k] = paz[k] * POW_0_7[k];
            }
            lp_synthesis(&mut ir, LP_ORDER, &tmp2, &tmp1);
            for k in lag_u..SUBFR {
                let v = p.pitch_sharp_factor * ir[LP_ORDER + k - lag_u];
                ir[LP_ORDER + k] += v;
            }

            // convolute_with_sparse
            let mut fv = [0f32; SUBFR];
            for k in 0..fixed.n {
                for j in fixed.x[k]..SUBFR {
                    fv[j] += fixed.y[k] * ir[LP_ORDER + j - fixed.x[k]];
                }
            }
            let avg_energy = ((0.01 + f64::from(dot(&fv, &fv))) / SUBFR as f64) as f32;
            let gc = GAIN_CB.get(parm.gc_index[i] * 2..parm.gc_index[i] * 2 + 2).unwrap_or(&[0.0, 0.0]);
            let mut pitch_gain = gc[0];
            self.past_pitch_gain = pitch_gain;
            let mean = (34.0 - 15.0 / (0.05 * LN_10 / LN_2)) as f32;
            let mut gain_code = amr_set_fixed_gain(gc[1], avg_energy, &mut self.energy_history, mean, &PRED);
            weighted_sum_in_place(&mut self.excitation[exc..exc + SUBFR], &fv, pitch_gain, gain_code);

            pitch_gain = (f64::from(pitch_gain) * (0.5 * f64::from(pitch_gain))) as f32;
            if f64::from(pitch_gain) > 0.4 {
                pitch_gain = 0.4f64 as f32;
            }
            self.gain_mem = (0.7 * f64::from(self.gain_mem) + 0.3 * f64::from(pitch_gain)) as f32;
            if self.gain_mem > pitch_gain {
                self.gain_mem = pitch_gain;
            }
            gain_code *= self.gain_mem;
            for j in 0..SUBFR {
                fv[j] = self.excitation[exc + j] - gain_code * fv[j];
            }
            if five {
                self.postfilter_5k0(paz, &mut fv);
                let e: Vec<f32> = self.excitation[exc..exc + SUBFR].to_vec();
                lp_synthesis(&mut self.postfilter_syn5k0, LP_ORDER + i * SUBFR, paz, &e);
            }
            lp_synthesis(&mut self.synth_buf, synth0 + i * SUBFR, paz, &fv);
            exc += SUBFR;
        }
        self.synth_buf.copy_within(synth0 + frame_size - LP_ORDER..synth0 + frame_size, synth0 - LP_ORDER);
        if five {
            for i in 0..n_sub {
                let s = &self.postfilter_syn5k0[LP_ORDER + i * SUBFR..LP_ORDER + (i + 1) * SUBFR];
                let energy = dot(s, s);
                let a = synth0 + i * SUBFR;
                adaptive_gain_control(&mut self.synth_buf[a..a + SUBFR], energy, 0.9, &mut self.postfilter_agc);
            }
            self.postfilter_syn5k0.copy_within(frame_size..frame_size + LP_ORDER, 0);
        }
        let keep = PITCH_DELAY_MAX + L_INTERPOL;
        self.excitation.copy_within(exc - keep..exc, 0);
        order_2_transfer(
            out,
            &self.synth_buf[synth0..synth0 + frame_size],
            [-1.99997f64 as f32, 1.0],
            [-1.93307352f64 as f32, 0.935891986f64 as f32],
            0.939805806f64 as f32,
            &mut self.highpass_filt_mem,
        );
    }

    /// `decode_fixed_sparse`.
    fn fixed_sparse(&self, pulses: &[i16; 10], low_gain: bool) -> Fixed {
        let mut f = Fixed::default();
        let p = |i: usize| i32::from(pulses[i]);
        match self.mode {
            Mode::K6_5 => {
                for i in 0..3 {
                    f.x[i] = (3 * (p(i) & 0xf)) as usize + i;
                    f.y[i] = if p(i) & 0x10 != 0 { -1.0 } else { 1.0 };
                }
                f.n = 3;
            }
            Mode::K8_5 => {
                for i in 0..3 {
                    f.x[2 * i] = (3 * ((p(i) >> 4) & 0xf)) as usize + i;
                    f.x[2 * i + 1] = (3 * (p(i) & 0xf)) as usize + i;
                    f.y[2 * i] = if p(i) & 0x100 != 0 { -1.0 } else { 1.0 };
                    f.y[2 * i + 1] = if f.x[2 * i + 1] < f.x[2 * i] { -f.y[2 * i] } else { f.y[2 * i] };
                }
                f.n = 6;
            }
            _ => {
                if low_gain {
                    let offset = if p(0) & 0x200 != 0 { 2 } else { 0 };
                    let mut val = p(0);
                    for i in 0..3 {
                        let index = (val & 7) * 6 + 4 - (i as i32) * 2;
                        f.y[i] = if (offset + index) & 3 != 0 { -1.0 } else { 1.0 };
                        f.x[i] = index as usize;
                        val >>= 3;
                    }
                    f.n = 3;
                } else {
                    let subset = ((p(0) >> 8) & 1) as usize;
                    f.x[0] = ((p(0) >> 4) & 15) as usize * 3 + subset;
                    f.x[1] = (p(0) & 15) as usize * 3 + subset + 1;
                    f.y[0] = if p(0) & 0x200 != 0 { -1.0 } else { 1.0 };
                    f.y[1] = -f.y[0];
                    f.n = 2;
                }
            }
        }
        f
    }

    /// `postfilter_5k0`.
    fn postfilter_5k0(&mut self, lpc: &[f32], samples: &mut [f32; SUBFR]) {
        let mut lpc_n = [0f32; LP_ORDER];
        let mut lpc_d = [0f32; LP_ORDER];
        for i in 0..LP_ORDER {
            lpc_d[i] = lpc[i] * POW_0_75[i];
            lpc_n[i] = lpc[i] * (1.0 / f64::from(1u32 << (i + 1))) as f32;
        }
        let mut buf = [0f32; SUBFR + LP_ORDER];
        buf[..LP_ORDER].copy_from_slice(&self.postfilter_mem);
        lp_synthesis(&mut buf, LP_ORDER, &lpc_d, samples);
        self.postfilter_mem.copy_from_slice(&buf[SUBFR..SUBFR + LP_ORDER]);
        tilt_compensation(&mut self.tilt_mem, 0.4f64 as f32, &mut buf[LP_ORDER..]);
        buf[..LP_ORDER].copy_from_slice(&self.postfilter_mem5k0);
        self.postfilter_mem5k0.copy_from_slice(&buf[SUBFR..SUBFR + LP_ORDER]);
        lp_zero_synthesis(samples, &lpc_n, &buf, LP_ORDER);
    }

    /// `ff_sipr_decode_frame_16k`.
    fn frame_16k(&mut self, parm: &Params, out: &mut [f32]) {
        let frame_size = 2 * L_SUBFR_16K;
        let synth0 = LP_ORDER_16K;

        // lsf_decode_fp_16k
        let mut isp_q = [0f32; LP_ORDER_16K];
        let cbs: [&[f32]; 4] = [&LSF_CB1_16K, &LSF_CB2_16K, &LSF_CB3_16K, &LSF_CB4_16K];
        for i in 0..4 {
            cb_entry(&mut isp_q[3 * i..3 * i + 3], cbs[i], parm.vq_indexes[i], 3);
        }
        cb_entry(&mut isp_q[12..16], &LSF_CB5_16K, parm.vq_indexes[4], 4);
        let m = parm.ma_pred_switch & 1;
        let mut lsf_new = [0f32; LP_ORDER_16K];
        for i in 0..LP_ORDER_16K {
            lsf_new[i] = (1.0 - QU[m]) * isp_q[i] + QU[m] * self.lsf_history[i] + MEAN_LSF_16K[i];
        }
        self.lsf_history = isp_q;
        set_min_dist_lsf(&mut lsf_new, LSFQ_DIFF_MIN / 2.0);
        let mut lsp_new = [0f64; LP_ORDER_16K];
        for (d, s) in lsp_new.iter_mut().zip(&lsf_new) {
            *d = f64::from(s.cos());
        }
        // acelp_lp_decodef
        let mut az = [[0f32; LP_ORDER_16K]; 2];
        let mut lsp_1st = [0f64; LP_ORDER_16K];
        for k in 0..LP_ORDER_16K {
            lsp_1st[k] = (lsp_new[k] + self.lsp_history_16k[k]) * 0.5;
        }
        lspd2lpc(&lsp_1st, &mut az[0], LP_ORDER_16K >> 1);
        lspd2lpc(&lsp_new, &mut az[1], LP_ORDER_16K >> 1);
        self.lsp_history_16k = lsp_new;
        self.synth_buf[..LP_ORDER_16K].copy_from_slice(&self.synth);

        let exc0 = 292;
        let div3 = |x: i32| (x * 10923) >> 15;
        for i in 0..2 {
            let sub = exc0 + i * L_SUBFR_16K;
            let pd = parm.pitch_delay[i];
            let p3 = if i == 0 {
                if pd < 390 { pd + 88 } else { 3 * pd - 690 }
            } else if pd < 62 {
                3 * (self.pitch_lag_prev - 10).clamp(PITCH_MIN, PITCH_MAX as i32 - 19) + pd - 2
            } else {
                3 * self.pitch_lag_prev
            };
            let pitch_fac = GAIN_PITCH_CB_16K.get(parm.gp_index[i]).copied().unwrap_or(0.0);
            let mut f =
                Fixed { pitch_fac: if f64::from(pitch_fac) > 1.0 { 1.0 } else { pitch_fac }, pitch_lag: div3(p3 + 1).max(0) as usize, ..Fixed::default() };
            self.pitch_lag_prev = div3(p3 + 1);
            let pdi = div3(p3 + 2);
            let pdf = (p3 + 2 - 3 * pdi) as usize;
            let src = (sub + 1).saturating_sub(pdi.max(0) as usize).max(LP_ORDER);
            interpolate(&mut self.excitation, sub, src, &SINC_WIN, 3, pdf + 1, LP_ORDER, L_SUBFR_16K);

            let mut fv = [0f32; L_SUBFR_16K];
            decode_10_pulses_35bits(&parm.fc_indexes[i], &mut f, &TRACKS_13, 5, 4);
            set_fixed_vector(&mut fv, &f, 1.0);

            let gcf = GAIN_CB_16K.get(parm.gc_index[i]).copied().unwrap_or(0.0);
            // acelp_decode_gain_codef
            let mut mr = (19.0 - 15.0 / (0.05 * LN_10 / LN_2)) as f32;
            mr += dot(&PRED_16K, &self.energy_history[..2]);
            let g = (f64::from((L_SUBFR_16K as f64).sqrt() as f32) * (LN_10 / 20.0 * f64::from(mr)).exp() / (0.01 + f64::from(dot(&fv, &fv))).sqrt()) as f32;
            let gain_code = gcf * g;
            self.energy_history[1] = self.energy_history[0];
            self.energy_history[0] = (20.0 * f64::from(gcf.log10())) as f32;

            weighted_sum_in_place(&mut self.excitation[sub..sub + L_SUBFR_16K], &fv, pitch_fac, gain_code);
            let e: Vec<f32> = self.excitation[sub..sub + L_SUBFR_16K].to_vec();
            lp_synthesis(&mut self.synth_buf, synth0 + i * L_SUBFR_16K, &az[i], &e);
        }
        self.synth.copy_from_slice(&self.synth_buf[synth0 + frame_size - LP_ORDER_16K..synth0 + frame_size]);
        self.excitation.copy_within(frame_size..frame_size + L_INTERPOL + PITCH_MAX, 0);
        self.postfilter_16k(out);
        self.iir_mem[..LP_ORDER_16K].copy_from_slice(&az[1]);
    }

    /// `postfilter` (16 kbit/s).
    fn postfilter_16k(&mut self, out: &mut [f32]) {
        let (f0, f1) = (self.filt_first, 1 - self.filt_first);
        for i in 0..LP_ORDER_16K {
            self.filt_buf[f0][i] = self.iir_mem[i] * (1.0 / f64::from(1u32 << (i + 1))) as f32;
        }
        let s0 = LP_ORDER_16K;
        let mut tmp = [0f32; 30 + LP_ORDER_16K];
        tmp[..LP_ORDER_16K].copy_from_slice(&self.mem_preemph);
        let head: Vec<f32> = self.synth_buf[s0..s0 + 30].to_vec();
        lp_synthesis(&mut tmp, LP_ORDER_16K, &self.filt_buf[f1][..LP_ORDER_16K], &head);
        self.synth_buf[..LP_ORDER_16K].copy_from_slice(&self.mem_preemph);
        let c0: [f32; LP_ORDER_16K] = self.filt_buf[f0][..LP_ORDER_16K].try_into().unwrap_or([0.0; LP_ORDER_16K]);
        lp_synthesis(&mut self.synth_buf, s0, &c0, &head);
        // out needs 16 samples of history before index 30.
        let mut o = vec![0f32; LP_ORDER_16K + 2 * L_SUBFR_16K];
        let base = LP_ORDER_16K;
        o[base + 30 - LP_ORDER_16K..base + 30].copy_from_slice(&self.synth_buf[s0 + 30 - LP_ORDER_16K..s0 + 30]);
        let rest: Vec<f32> = self.synth_buf[s0 + 30..s0 + 2 * L_SUBFR_16K].to_vec();
        lp_synthesis(&mut o, base + 30, &c0, &rest);
        self.mem_preemph.copy_from_slice(&o[base + 2 * L_SUBFR_16K - LP_ORDER_16K..base + 2 * L_SUBFR_16K]);
        self.filt_first = f1;
        let mut s = 0f32;
        for i in 0..30 {
            o[base + i] = tmp[LP_ORDER_16K + i] + s * (self.synth_buf[s0 + i] - tmp[LP_ORDER_16K + i]);
            s = (f64::from(s) + 1.0 / 30.0) as f32;
        }
        out.copy_from_slice(&o[base..base + 2 * L_SUBFR_16K]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_survives_noise() {
        for ba in [20, 19, 29, 37] {
            let mut d = Decoder::new(ba).unwrap();
            assert!(d.decode(&vec![0; ba - 1]).is_err());
            let mut x = 0x2545_F491u32 ^ ba as u32;
            for _ in 0..3000 {
                let f: Vec<u8> = (0..ba)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 17;
                        x ^= x << 5;
                        x as u8
                    })
                    .collect();
                assert_eq!(d.decode(&f).unwrap().len(), d.frame_samples());
            }
        }
        assert!(Decoder::new(21).is_err());
    }
}
