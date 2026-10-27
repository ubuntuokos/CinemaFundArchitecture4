//! CELT layer encoder (fullband, 20 ms frames, constant bit rate).
//!
//! The bitstream side mirrors [`super::CeltDecoder::decode_frame`] symbol for symbol: band shapes,
//! allocation and TF flags run through the same generic code as the decoder (see
//! [`crate::range::Coder`]), so the encoder's energy state and resynthesis match the decoder by
//! construction. The analysis (transient detection, intensity/dual stereo decisions, PVQ search)
//! follows the structure of the RFC 6716 reference encoder.

use super::bands::quant_all_bands;
use super::energy::{quant_coarse_energy, quant_energy_finalise, quant_fine_energy};
use super::mdct::ImdctScratch;
use super::rate::{AllocHints, Mode, compute_allocation};
use super::tables::*;
use super::{mode, tf_code};
use crate::range::{BITRES, RangeEncoder};

/// Samples per channel in one encoded frame (20 ms at 48 kHz).
pub const FRAME_SIZE: usize = 960;
const LM: usize = 3;
const SPREAD_NORMAL: usize = 2;
const ALLOC_TRIM: usize = 5;

/// Intensity stereo start band by bit rate (kbit/s) with hysteresis (reference encoder tuning).
const INTENSITY_THRESHOLDS: [f32; 21] = [1., 2., 3., 4., 5., 6., 7., 8., 16., 24., 36., 44., 50., 56., 62., 67., 72., 79., 88., 106., 134.];
const INTENSITY_HYSTERESIS: [f32; 21] = [1., 1., 1., 1., 1., 1., 1., 2., 2., 2., 2., 2., 2., 2., 3., 3., 4., 5., 6., 8., 8.];

/// CELT encoder state for one mono or stereo stream.
pub struct CeltEncoder {
    m: &'static Mode,
    channels: usize,
    /// Pre-emphasised history: the last `OVERLAP` samples of each channel.
    in_mem: Vec<f32>,
    preemph_mem: [f32; 2],
    old_band_e: [f32; 2 * NB_EBANDS],
    rng: u32,
    intensity: usize,
    last_coded_bands: usize,
    consec_transient: u32,
    frames: u64,
    // scratch
    buf: Vec<f32>,
    freq: Vec<f32>,
    x: Vec<f32>,
    mdct: ImdctScratch,
}

impl CeltEncoder {
    /// `channels` is 1 or 2.
    pub fn new(channels: usize) -> CeltEncoder {
        let channels = channels.clamp(1, 2);
        CeltEncoder {
            m: mode(),
            channels,
            in_mem: vec![0.0; channels * OVERLAP],
            preemph_mem: [0.0; 2],
            old_band_e: [0.0; 2 * NB_EBANDS],
            rng: 0,
            intensity: 0,
            last_coded_bands: 0,
            consec_transient: 0,
            frames: 0,
            buf: Vec::new(),
            freq: Vec::new(),
            x: Vec::new(),
            mdct: ImdctScratch::default(),
        }
    }

    /// Final range of the last frame (what a decoder reports after decoding it).
    pub fn final_range(&self) -> u32 {
        self.rng
    }

    /// Encodes one frame of [`FRAME_SIZE`] interleaved samples (nominal range ±1.0) into exactly
    /// `bytes` bytes (2..=1275) of CELT data. `bitrate` (bit/s) steers the stereo decisions.
    pub fn encode(&mut self, pcm: &[f32], bytes: usize, bitrate: u32) -> Vec<u8> {
        let c = self.channels;
        let n = FRAME_SIZE;
        let bytes = bytes.clamp(2, 1275);
        let total_bits = bytes as i32 * 8;
        let mm = 1usize << LM;
        let start = 0usize;
        let end = NB_EBANDS;

        // Pre-emphasis into [history | new samples] per channel.
        let len = n + OVERLAP;
        let mut buf = std::mem::take(&mut self.buf);
        buf.clear();
        buf.resize(c * len, 0.0);
        let mut silent = true;
        for ch in 0..c {
            let b = &mut buf[ch * len..(ch + 1) * len];
            b[..OVERLAP].copy_from_slice(&self.in_mem[ch * OVERLAP..(ch + 1) * OVERLAP]);
            let mut mem = self.preemph_mem[ch];
            for i in 0..n {
                let s = pcm.get(i * c + ch).copied().unwrap_or(0.0);
                let x = if s.is_finite() { s.clamp(-2.0, 2.0) * 32768.0 } else { 0.0 };
                silent &= x == 0.0;
                b[OVERLAP + i] = x - mem;
                mem = PREEMPH * x;
            }
            self.preemph_mem[ch] = mem;
            self.in_mem[ch * OVERLAP..(ch + 1) * OVERLAP].copy_from_slice(&b[n..]);
        }

        let mut enc = RangeEncoder::new(bytes);
        enc.bit_logp(silent, 15);
        if silent {
            // The decoder treats the rest of the frame as already read.
            enc.skip_to_end(total_bits);
            for v in self.old_band_e.iter_mut() {
                *v = -28.0;
            }
            self.buf = buf;
            self.consec_transient = 0;
            self.frames += 1;
            let out = enc.done();
            self.rng = 0;
            return out;
        }
        // No pitch post-filter.
        if enc.tell() + 16 <= total_bits {
            enc.bit_logp(false, 1);
        }
        let mut is_transient = transient_analysis(&buf, len, c);
        if enc.tell() + 3 <= total_bits {
            enc.bit_logp(is_transient, 3);
        } else {
            is_transient = false;
        }

        // MDCT: one long block or eight short ones (interleaved).
        let mut freq = std::mem::take(&mut self.freq);
        freq.clear();
        freq.resize(c * n, 0.0);
        let (bcount, nb, shift) = if is_transient { (mm, SHORT_MDCT, MAX_LM) } else { (1, n, 0) };
        for ch in 0..c {
            for b in 0..bcount {
                let input = &buf[ch * len + b * nb..ch * len + b * nb + nb + OVERLAP];
                self.m.mdct[shift].forward(input, &mut freq[ch * n + b..], bcount, &self.m.window, OVERLAP, &mut self.mdct);
            }
        }
        self.buf = buf;

        // Band energies and normalised shapes.
        let mut band_e = [0f32; 2 * NB_EBANDS];
        let mut band_log_e = [0f32; 2 * NB_EBANDS];
        let mut x = std::mem::take(&mut self.x);
        x.clear();
        x.resize(c * n, 0.0);
        for ch in 0..c {
            for i in start..end {
                let lo = ch * n + mm * EBANDS[i] as usize;
                let hi = ch * n + mm * EBANDS[i + 1] as usize;
                let e = (1e-27 + freq[lo..hi].iter().map(|v| v * v).sum::<f32>()).sqrt();
                band_e[ch * NB_EBANDS + i] = e;
                band_log_e[ch * NB_EBANDS + i] = e.log2() - E_MEANS[i];
                let g = 1.0 / (1e-27 + e);
                for j in lo..hi {
                    x[j] = freq[j] * g;
                }
            }
        }
        self.freq = freq;

        let mut dual_stereo = false;
        if c == 2 {
            dual_stereo = stereo_analysis(&x, n);
            let kbps = bitrate as f32 / 1000.0;
            self.intensity = hysteresis_decision(kbps, &INTENSITY_THRESHOLDS, &INTENSITY_HYSTERESIS, self.intensity).clamp(start, end);
        }

        // Coarse energy: try intra and inter prediction, keep the cheaper one.
        let max_decay = if end - start > 10 { (0.125 * bytes as f32).min(16.0) } else { 16.0 };
        let can_flag = enc.tell() + 3 <= total_bits;
        let force_intra = self.frames == 0 || !can_flag;
        let code = |intra: bool, enc: &RangeEncoder, old: &[f32; 2 * NB_EBANDS]| {
            let mut e = enc.clone();
            let mut o = *old;
            let mut err = [0f32; 2 * NB_EBANDS];
            if can_flag {
                e.bit_logp(intra, 3);
            }
            quant_coarse_energy(start, end, &band_log_e, &mut o, intra, &mut e, c, LM, max_decay, &mut err);
            (e, o, err)
        };
        let (e_intra, o_intra, err_intra) = code(true, &enc, &self.old_band_e);
        let mut error;
        if force_intra {
            (enc, self.old_band_e, error) = (e_intra, o_intra, err_intra);
        } else {
            let (e_inter, o_inter, err_inter) = code(false, &enc, &self.old_band_e);
            if e_intra.tell_frac() < e_inter.tell_frac() {
                (enc, self.old_band_e, error) = (e_intra, o_intra, err_intra);
            } else {
                (enc, self.old_band_e, error) = (e_inter, o_inter, err_inter);
            }
        }

        let mut tf_res = [is_transient as i32; NB_EBANDS];
        tf_code(start, end, is_transient, &mut tf_res, 0, LM, &mut enc);
        let mut spread = SPREAD_NORMAL;
        if enc.tell() + 4 <= total_bits {
            enc.icdf(SPREAD_NORMAL, &SPREAD_ICDF, 5);
        } else {
            spread = SPREAD_NORMAL;
        }

        // No dynamic allocation boosts: one "no boost" flag per band where the decoder reads one.
        let cap = self.m.init_caps(LM, c);
        let offsets = [0i32; NB_EBANDS];
        let total_frac = total_bits << BITRES;
        let mut tellf = enc.tell_frac() as i32;
        for &cp in cap.iter().take(end).skip(start) {
            if tellf + (6 << BITRES) < total_frac && 0 < cp {
                enc.bit_logp(false, 6);
                tellf = enc.tell_frac() as i32;
            }
        }
        let alloc_trim = if tellf + (6 << BITRES) <= total_frac {
            enc.icdf(ALLOC_TRIM, &TRIM_ICDF, 7);
            ALLOC_TRIM as i32
        } else {
            5
        };
        let mut bits = total_frac - enc.tell_frac() as i32 - 1;
        let anti_collapse_rsv = if is_transient && bits >= ((LM as i32 + 2) << BITRES) { 1 << BITRES } else { 0 };
        bits -= anti_collapse_rsv;
        let hints = AllocHints { prev_coded_bands: self.last_coded_bands, signal_bandwidth: end - 1, intensity: self.intensity, dual_stereo };
        let alloc = compute_allocation(self.m, start, end, &offsets, &cap, alloc_trim, bits, c, LM, &mut enc, &hints);
        self.last_coded_bands = alloc.coded_bands;
        quant_fine_energy(start, end, &mut self.old_band_e, &mut error, &alloc.fine_quant, &mut enc, c);

        let mut collapse_masks = vec![0u32; c * NB_EBANDS];
        {
            let (xa, xb) = x.split_at_mut(n);
            let yb = if c == 2 { Some(xb) } else { None };
            let mut seed = self.rng;
            quant_all_bands(
                self.m,
                start,
                end,
                xa,
                yb,
                &mut collapse_masks,
                &alloc.pulses,
                is_transient,
                spread,
                alloc.dual_stereo,
                alloc.intensity,
                &tf_res,
                total_frac - anti_collapse_rsv,
                alloc.balance,
                &mut enc,
                &band_e,
                LM,
                alloc.coded_bands,
                &mut seed,
                false,
            );
        }
        self.x = x;
        if anti_collapse_rsv > 0 {
            enc.bits((self.consec_transient < 2) as u32, 1);
        }
        let bits_left = total_bits - enc.tell();
        quant_energy_finalise(start, end, &mut self.old_band_e, &mut error, &alloc.fine_quant, &alloc.fine_priority, bits_left, &mut enc, c);
        if c == 1 {
            let (a, b) = self.old_band_e.split_at_mut(NB_EBANDS);
            b.copy_from_slice(a);
        }
        self.consec_transient = if is_transient { self.consec_transient + 1 } else { 0 };
        self.frames += 1;
        self.rng = enc.rng();
        enc.done()
    }
}

/// Time-domain transient detector on the pre-emphasised frame (reference encoder structure):
/// high-pass, forward/backward masking envelopes, then the ratio of the mean energy to the
/// harmonic mean of the envelope.
fn transient_analysis(buf: &[f32], len: usize, c: usize) -> bool {
    let len2 = len / 2;
    if len2 < 24 {
        return false;
    }
    let mut tmp = vec![0f32; len];
    let mut mask_metric = 0f32;
    for ch in 0..c {
        let input = &buf[ch * len..(ch + 1) * len];
        let (mut mem0, mut mem1) = (0f32, 0f32);
        // High-pass: (1 - 2 z^-1 + z^-2) / (1 - z^-1 + 0.5 z^-2).
        for (t, &x) in tmp.iter_mut().zip(input) {
            let y = mem0 + x;
            mem0 = mem1 + y - 2.0 * x;
            mem1 = x - 0.5 * y;
            *t = y;
        }
        tmp[..12].fill(0.0);
        // Forward pass: post-echo threshold.
        let mut mean = 0f32;
        mem0 = 0.0;
        for i in 0..len2 {
            let x2 = tmp[2 * i] * tmp[2 * i] + tmp[2 * i + 1] * tmp[2 * i + 1];
            mean += x2;
            tmp[i] = mem0 + 0.0625 * (x2 - mem0);
            mem0 = tmp[i];
        }
        // Backward pass: pre-echo threshold (13.9 dB/ms masking).
        mem0 = 0.0;
        let mut max_e = 0f32;
        for i in (0..len2).rev() {
            tmp[i] = mem0 + 0.125 * (tmp[i] - mem0);
            mem0 = tmp[i];
            max_e = max_e.max(mem0);
        }
        let mean = (mean * max_e * 0.5 * len2 as f32).sqrt();
        let norm = len2 as f32 / (1e-15 + mean);
        let mut unmask = 0f32;
        let mut i = 12;
        while i + 5 < len2 {
            let id = (64.0 * norm * (tmp[i] + 1e-15)).floor().clamp(0.0, 127.0);
            // Inverse table of the reference encoder: ~6*64/(id+0.5), saturating at 255.
            unmask += (384.0 / (id + 0.5)).floor().min(255.0);
            i += 4;
        }
        let unmask = 64.0 * unmask * 4.0 / (6.0 * (len2 - 17) as f32);
        mask_metric = mask_metric.max(unmask);
    }
    mask_metric > 200.0
}

/// Chooses dual (L/R) stereo when its L1 cost beats M/S on the lower 13 bands.
fn stereo_analysis(x: &[f32], n: usize) -> bool {
    let (mut sum_lr, mut sum_ms) = (1e-15f32, 1e-15f32);
    for j in 0..(EBANDS[13] as usize) << LM {
        let l = x.get(j).copied().unwrap_or(0.0);
        let r = x.get(n + j).copied().unwrap_or(0.0);
        sum_lr += l.abs() + r.abs();
        sum_ms += (l + r).abs() + (l - r).abs();
    }
    let sum_ms = std::f32::consts::FRAC_1_SQRT_2 * sum_ms;
    let thetas = 13.0;
    let width = (EBANDS[13] << (LM + 1)) as f32;
    (width + thetas) * sum_ms > width * sum_lr
}

fn hysteresis_decision(val: f32, thresholds: &[f32], hysteresis: &[f32], prev: usize) -> usize {
    let n = thresholds.len();
    let mut i = thresholds.iter().position(|&t| val < t).unwrap_or(n);
    let prev = prev.min(n);
    if i > prev && prev < n && val < thresholds[prev] + hysteresis[prev] {
        i = prev;
    }
    if i < prev && prev > 0 && val > thresholds[prev - 1] - hysteresis[prev - 1] {
        i = prev;
    }
    i
}
