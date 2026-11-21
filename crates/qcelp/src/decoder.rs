//! The QCELP-13K (TIA/EIA/IS-733) frame decoder, a port of FFmpeg n7.1.1 `libavcodec/qcelpdec.c`
//! (Copyright (c) 2007 Reynaldo H. Verdejo Pinochet; LGPL-2.1-or-later). The float operations keep
//! FFmpeg's order and its C promotion rules (`float * double` is computed in double), so the output
//! matches FFmpeg bit for bit.

use crate::celp::{adaptive_gain_control, dot, lp_synthesis, lp_zero_synthesis, lspd2lpc, scale_to_sum_of_squares, tilt_compensation, weighted_sum};
use crate::tables::{
    BITMAP_FULL, BITMAP_HALF, BITMAP_OCTAVE, BITMAP_QUARTER, G12GA_NUM, LSPVQ1, LSPVQ2, LSPVQ3, LSPVQ4, LSPVQ5, RATE_FULL_CODEBOOK, RATE_HALF_CODEBOOK,
};

/// Samples per frame (20 ms at 8 kHz).
pub const FRAME_SAMPLES: usize = 160;

/// Packet rate (`qcelp_packet_rate`); `Ifq` is "insufficient frame quality", an erasure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Rate {
    Ifq = -1,
    Silence = 0,
    Octave = 1,
    Quarter = 2,
    Half = 3,
    Full = 4,
}

const FRAME_BYTES: usize = 71;
const CBSIGN: usize = 0;
const CBGAIN: usize = 16;
const CINDEX: usize = 32;
const PLAG: usize = 48;
const PFRAC: usize = 52;
const PGAIN: usize = 56;
const LSPV: usize = 60;
const RESERVED: usize = 70;

const HAMMSINC: [f32; 4] = [-0.006822, 0.041249, -0.143459, 0.588863];
const RND_FIR_COEFS: [f64; 11] =
    [-1.344519e-1, 1.735384e-2, -6.905826e-2, 2.434368e-2, -8.210701e-2, 3.041388e-2, -9.251384e-2, 3.501983e-2, -9.918777e-2, 3.749518e-2, 8.985137e-1];
const SPREAD: f64 = 0.02;
const OCTAVE_PREDICTOR: f64 = 29.0 / 32.0;
const BANDWIDTH_EXPANSION: f64 = 0.9883;

/// `qcelp_g12ga`: `x.xxx / 8192.` in double, rounded to float.
const G12GA: [f32; 61] = {
    let mut t = [0f32; 61];
    let mut i = 0;
    while i < 61 {
        t[i] = (G12GA_NUM[i] / 8192.0) as f32;
        i += 1;
    }
    t
};

/// A bit reader over a byte slice (MSB first); past the end it reads zeros.
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn get(&mut self, n: u8) -> u32 {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = self.data.get(self.pos >> 3).copied().unwrap_or(0);
            v = (v << 1) | u32::from((byte >> (7 - (self.pos & 7))) & 1);
            self.pos += 1;
        }
        v
    }
}

/// `FFMAX(a, b)` of a float and a double (the comparison and the result are double), back to float.
fn ffmax(a: f32, b: f64) -> f32 {
    let a = f64::from(a);
    (if a > b { a } else { b }) as f32
}

/// `FFMIN(a, b)` of a float and a double.
fn ffmin(a: f32, b: f64) -> f32 {
    let a = f64::from(a);
    (if a > b { b } else { a }) as f32
}

/// The decoder state (`QCELPContext`); all-zero except the previous LSP frequencies.
#[derive(Clone)]
pub struct Decoder {
    bitrate: Rate,
    frame: [u8; FRAME_BYTES],
    erasure_count: u8,
    octave_count: u8,
    prev_lspf: [f32; 10],
    predictor_lspf: [f32; 10],
    pitch_synthesis_filter_mem: [f32; 303],
    pitch_pre_filter_mem: [f32; 303],
    rnd_fir_filter_mem: [f32; 180],
    formant_mem: [f32; 170],
    last_codebook_gain: f32,
    prev_g1: [i32; 2],
    prev_bitrate: Rate,
    pitch_gain: [f32; 4],
    pitch_lag: [u8; 4],
    first16bits: u16,
    postfilter_synth_mem: [f32; 10],
    postfilter_agc_mem: f32,
    postfilter_tilt_mem: f32,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    /// A decoder at the start of a stream (`qcelp_decode_init`).
    pub fn new() -> Self {
        let mut prev_lspf = [0f32; 10];
        for (i, v) in prev_lspf.iter_mut().enumerate() {
            *v = ((i + 1) as f64 / 11.0) as f32;
        }
        Self {
            bitrate: Rate::Silence,
            frame: [0; FRAME_BYTES],
            erasure_count: 0,
            octave_count: 0,
            prev_lspf,
            predictor_lspf: [0.0; 10],
            pitch_synthesis_filter_mem: [0.0; 303],
            pitch_pre_filter_mem: [0.0; 303],
            rnd_fir_filter_mem: [0.0; 180],
            formant_mem: [0.0; 170],
            last_codebook_gain: 0.0,
            prev_g1: [0; 2],
            prev_bitrate: Rate::Silence,
            pitch_gain: [0.0; 4],
            pitch_lag: [0; 4],
            first16bits: 0,
            postfilter_synth_mem: [0.0; 10],
            postfilter_agc_mem: 0.0,
            postfilter_tilt_mem: 0.0,
        }
    }

    /// Decodes one packet and appends its 160 samples (full scale is about ±1) to `out`. A packet
    /// whose rate cannot be determined or whose contents are implausible is an erasure and is
    /// concealed, as FFmpeg does; this never fails. Returns false if the packet was concealed.
    ///
    /// A packet is the rate byte (0 blank, 1 eighth, 2 quarter, 3 half, 4 full) and the frame data,
    /// 1, 4, 8, 17 or 35 bytes in all; a packet one byte short has lost its rate byte, which is
    /// guessed from the size.
    pub fn decode(&mut self, packet: &[u8], out: &mut Vec<f32>) -> bool {
        let mut samples = [0f32; FRAME_SAMPLES];
        let ok = self.decode_frame(packet, &mut samples);
        out.extend_from_slice(&samples);
        ok
    }

    fn decode_frame(&mut self, packet: &[u8], outbuffer: &mut [f32; FRAME_SAMPLES]) -> bool {
        let mut quantized_lspf = [0f32; 10];
        let mut gain = [0f32; 16];
        let mut lpc = [0f32; 10];

        let mut erasure = false;
        let mut concealed = false;
        let (bitrate, data) = determine_bitrate(packet);
        self.bitrate = bitrate;
        if bitrate == Rate::Ifq {
            erasure = true;
        }

        if !erasure && self.bitrate == Rate::Octave {
            self.first16bits = u16::from_be_bytes([data.first().copied().unwrap_or(0), data.get(1).copied().unwrap_or(0)]);
            if self.first16bits == 0xFFFF {
                erasure = true;
            }
        }

        if !erasure && self.bitrate > Rate::Silence {
            let bitmaps: &[(u8, u8, u8)] = match self.bitrate {
                Rate::Octave => &BITMAP_OCTAVE,
                Rate::Quarter => &BITMAP_QUARTER,
                Rate::Half => &BITMAP_HALF,
                _ => &BITMAP_FULL,
            };
            let mut gb = Bits { data, pos: 0 };
            self.frame = [0; FRAME_BYTES];
            for &(index, bitpos, bitlen) in bitmaps {
                self.frame[usize::from(index)] |= (gb.get(bitlen) << bitpos) as u8;
            }
            let reserved = self.frame[RESERVED] != 0;
            let bad_quarter = self.bitrate == Rate::Quarter && !codebook_sanity_check_for_rate_quarter(&self.frame[CBGAIN..CBGAIN + 5]);
            let bad_pitch = self.bitrate >= Rate::Half && (0..4).any(|i| self.frame[PFRAC + i] != 0 && self.frame[PLAG + i] >= 124);
            if reserved || bad_quarter || bad_pitch {
                erasure = true;
            }
        }

        if !erasure {
            self.decode_gain_and_index(&mut gain);
            self.compute_svector(&gain, outbuffer);
            if !self.decode_lspf(&mut quantized_lspf) {
                erasure = true;
            } else {
                self.apply_pitch_filters(outbuffer);
                self.erasure_count = 0;
            }
        }

        if erasure {
            concealed = true;
            self.bitrate = Rate::Ifq;
            self.erasure_count = self.erasure_count.wrapping_add(1);
            self.decode_gain_and_index(&mut gain);
            self.compute_svector(&gain, outbuffer);
            self.decode_lspf(&mut quantized_lspf);
            self.apply_pitch_filters(outbuffer);
        }

        for i in 0..4 {
            self.interpolate_lpc(&quantized_lspf, &mut lpc, i);
            lp_synthesis(&mut self.formant_mem, 10 + 40 * i, &lpc, &outbuffer[i * 40..i * 40 + 40]);
        }

        self.postfilter(outbuffer, &lpc);

        self.formant_mem.copy_within(160..170, 0);
        self.prev_lspf = quantized_lspf;
        self.prev_bitrate = self.bitrate;
        !concealed
    }

    /// `decode_lspf`: false if the packet was badly received.
    fn decode_lspf(&mut self, lspf: &mut [f32; 10]) -> bool {
        let lspv = &self.frame[LSPV..LSPV + 10];
        if self.bitrate == Rate::Octave || self.bitrate == Rate::Ifq {
            let predictors = if self.prev_bitrate != Rate::Octave && self.prev_bitrate != Rate::Ifq { self.prev_lspf } else { self.predictor_lspf };
            let smooth: f32;
            if self.bitrate == Rate::Octave {
                self.octave_count = self.octave_count.wrapping_add(1);
                for i in 0..10 {
                    let spread = if lspv[i] != 0 { SPREAD } else { -SPREAD };
                    let v = ((spread + f64::from(predictors[i]) * 29.0 / 32.0) + (i + 1) as f64 * ((1.0 - OCTAVE_PREDICTOR) / 11.0)) as f32;
                    lspf[i] = v;
                    self.predictor_lspf[i] = v;
                }
                smooth = if self.octave_count < 10 { 0.875 } else { 0.1f64 as f32 };
            } else {
                let mut erasure_coeff = OCTAVE_PREDICTOR as f32;
                if self.erasure_count > 1 {
                    erasure_coeff = (f64::from(erasure_coeff) * if self.erasure_count < 4 { 0.9 } else { 0.7 }) as f32;
                }
                for i in 0..10 {
                    let v = (i + 1) as f32 * (1.0f32 - erasure_coeff) / 11.0f32 + erasure_coeff * predictors[i];
                    lspf[i] = v;
                    self.predictor_lspf[i] = v;
                }
                smooth = 0.125;
            }

            // Check the stability of the LSP frequencies.
            lspf[0] = ffmax(lspf[0], SPREAD);
            for i in 1..10 {
                lspf[i] = ffmax(lspf[i], f64::from(lspf[i - 1]) + SPREAD);
            }
            lspf[9] = ffmin(lspf[9], 1.0 - SPREAD);
            for i in (1..10).rev() {
                lspf[i - 1] = ffmin(lspf[i - 1], f64::from(lspf[i]) - SPREAD);
            }

            // Low-pass filter the LSP frequencies.
            let cur = *lspf;
            weighted_sum(lspf, &cur, &self.prev_lspf, smooth, (1.0 - f64::from(smooth)) as f32);
        } else {
            self.octave_count = 0;
            let tables: [&[[u16; 2]]; 5] = [&LSPVQ1, &LSPVQ2, &LSPVQ3, &LSPVQ4, &LSPVQ5];
            let mut tmp = 0f32;
            for i in 0..5 {
                let v = tables[i].get(usize::from(lspv[i])).copied().unwrap_or([0, 0]);
                tmp = (f64::from(tmp) + f64::from(v[0]) * 0.0001) as f32;
                lspf[2 * i] = tmp;
                tmp = (f64::from(tmp) + f64::from(v[1]) * 0.0001) as f32;
                lspf[2 * i + 1] = tmp;
            }

            // Check for badly received packets.
            if self.bitrate == Rate::Quarter {
                if f64::from(lspf[9]) <= 0.70 || f64::from(lspf[9]) >= 0.97 {
                    return false;
                }
                for i in 3..10 {
                    if f64::from((lspf[i] - lspf[i - 2]).abs()) < 0.08 {
                        return false;
                    }
                }
            } else {
                if f64::from(lspf[9]) <= 0.66 || f64::from(lspf[9]) >= 0.985 {
                    return false;
                }
                for i in 4..10 {
                    if f64::from((lspf[i] - lspf[i - 4]).abs()) < 0.0931 {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// `decode_gain_and_index`: convert the codebook transmission codes to gain and index.
    fn decode_gain_and_index(&mut self, gain: &mut [f32; 16]) {
        let mut g1 = [0i32; 16];
        if self.bitrate >= Rate::Quarter {
            let subframes_count = match self.bitrate {
                Rate::Full => 16,
                Rate::Half => 4,
                _ => 5,
            };
            for i in 0..subframes_count {
                g1[i] = 4 * i32::from(self.frame[CBGAIN + i]);
                if self.bitrate == Rate::Full && (i + 1) & 3 == 0 {
                    g1[i] += ((g1[i - 1] + g1[i - 2] + g1[i - 3]) / 3 - 6).clamp(0, 32);
                }

                gain[i] = G12GA[g1[i].clamp(0, 60) as usize];

                if self.frame[CBSIGN + i] != 0 {
                    gain[i] = -gain[i];
                    self.frame[CINDEX + i] = (i32::from(self.frame[CINDEX + i]) - 89) as u8 & 127;
                }
            }

            let i = subframes_count;
            self.prev_g1 = [g1[i - 2], g1[i - 1]];
            self.last_codebook_gain = G12GA[g1[i - 1].clamp(0, 60) as usize];

            if self.bitrate == Rate::Quarter {
                // Provide smoothing of the unvoiced excitation energy.
                let d = |x: f64| x as f32;
                gain[7] = gain[4];
                gain[6] = d(0.4 * f64::from(gain[3]) + 0.6 * f64::from(gain[4]));
                gain[5] = gain[3];
                gain[4] = d(0.8 * f64::from(gain[2]) + 0.2 * f64::from(gain[3]));
                gain[3] = d(0.2 * f64::from(gain[1]) + 0.8 * f64::from(gain[2]));
                gain[2] = gain[1];
                gain[1] = d(0.6 * f64::from(gain[0]) + 0.4 * f64::from(gain[1]));
            }
        } else if self.bitrate != Rate::Silence {
            let subframes_count;
            let mut g10;
            if self.bitrate == Rate::Octave {
                g10 = 2 * i32::from(self.frame[CBGAIN]) + ((self.prev_g1[0] + self.prev_g1[1]) / 2 - 5).clamp(0, 54);
                subframes_count = 8;
            } else {
                g10 = self.prev_g1[1];
                match self.erasure_count {
                    1 => {}
                    2 => g10 -= 1,
                    3 => g10 -= 2,
                    _ => g10 -= 6,
                }
                if g10 < 0 {
                    g10 = 0;
                }
                subframes_count = 4;
            }
            // This interpolation is done to produce smoother background noise.
            let target = G12GA[g10.clamp(0, 60) as usize];
            let slope = (0.5 * f64::from(target - self.last_codebook_gain) / subframes_count as f64) as f32;
            for i in 1..=subframes_count {
                gain[i - 1] = self.last_codebook_gain + slope * i as f32;
            }

            self.last_codebook_gain = gain[subframes_count - 1];
            self.prev_g1[0] = self.prev_g1[1];
            self.prev_g1[1] = g10;
        }
    }

    /// `compute_svector`: the scaled codebook vector Cdn from index and gain, for all rates.
    fn compute_svector(&mut self, gain: &[f32; 16], cdn: &mut [f32; FRAME_SAMPLES]) {
        let mut at = 0;
        match self.bitrate {
            Rate::Full => {
                for (i, &g) in gain.iter().enumerate() {
                    let tmp_gain = (f64::from(g) * 0.01) as f32;
                    let mut cindex = 0u16.wrapping_sub(u16::from(self.frame[CINDEX + i]));
                    for _ in 0..10 {
                        cdn[at] = tmp_gain * f32::from(RATE_FULL_CODEBOOK[usize::from(cindex & 127)]);
                        cindex = cindex.wrapping_add(1);
                        at += 1;
                    }
                }
            }
            Rate::Half => {
                for (i, &g) in gain.iter().take(4).enumerate() {
                    let tmp_gain = (f64::from(g) * 0.5) as f32;
                    let mut cindex = 0u16.wrapping_sub(u16::from(self.frame[CINDEX + i]));
                    for _ in 0..40 {
                        cdn[at] = tmp_gain * f32::from(RATE_HALF_CODEBOOK[usize::from(cindex & 127)]);
                        cindex = cindex.wrapping_add(1);
                        at += 1;
                    }
                }
            }
            Rate::Quarter => {
                let l = |i: usize| u32::from(self.frame[LSPV + i]);
                let mut cbseed = ((0x0003 & l(4)) << 14 | (0x003F & l(3)) << 8 | (0x0060 & l(2)) << 1 | (0x0007 & l(1)) << 3 | (0x0038 & l(0)) >> 3) as u16;
                let mut rnd = 20;
                for &g in gain.iter().take(8) {
                    let tmp_gain = (f64::from(g) * (1.373681186 / 32768.0)) as f32;
                    for _ in 0..20 {
                        cbseed = (521 * u32::from(cbseed) + 259) as u16;
                        self.rnd_fir_filter_mem[rnd] = f32::from(cbseed as i16);

                        // FIR filter
                        let m = &self.rnd_fir_filter_mem;
                        let mut fir = 0f32;
                        for j in 0..10 {
                            fir = (f64::from(fir) + RND_FIR_COEFS[j] * f64::from(m[rnd - j] + m[rnd - 20 + j])) as f32;
                        }
                        fir = (f64::from(fir) + RND_FIR_COEFS[10] * f64::from(m[rnd - 10])) as f32;
                        cdn[at] = tmp_gain * fir;
                        at += 1;
                        rnd += 1;
                    }
                }
                self.rnd_fir_filter_mem.copy_within(160..180, 0);
            }
            Rate::Octave => {
                let mut cbseed = self.first16bits;
                for &g in gain.iter().take(8) {
                    let tmp_gain = (f64::from(g) * (1.373681186 / 32768.0)) as f32;
                    for _ in 0..20 {
                        cbseed = (521 * u32::from(cbseed) + 259) as u16;
                        cdn[at] = tmp_gain * f32::from(cbseed as i16);
                        at += 1;
                    }
                }
            }
            Rate::Ifq => {
                let mut cbseed = 0u16.wrapping_sub(44); // random codebook index
                for &g in gain.iter().take(4) {
                    let tmp_gain = (f64::from(g) * 0.01) as f32;
                    for _ in 0..40 {
                        cdn[at] = tmp_gain * f32::from(RATE_FULL_CODEBOOK[usize::from(cbseed & 127)]);
                        cbseed = cbseed.wrapping_add(1);
                        at += 1;
                    }
                }
            }
            Rate::Silence => cdn.fill(0.0),
        }
    }

    /// `apply_pitch_filters`: the pitch synthesis filter and the pitch prefilter on the scaled
    /// codebook vector.
    fn apply_pitch_filters(&mut self, cdn: &mut [f32; FRAME_SAMPLES]) {
        if self.bitrate >= Rate::Half || self.bitrate == Rate::Silence || (self.bitrate == Rate::Ifq && self.prev_bitrate >= Rate::Half) {
            if self.bitrate >= Rate::Half {
                // Compute gain & lag for the whole frame.
                for i in 0..4 {
                    self.pitch_gain[i] = if self.frame[PLAG + i] != 0 { (f64::from(i32::from(self.frame[PGAIN + i]) + 1) * 0.25) as f32 } else { 0.0 };
                    self.pitch_lag[i] = self.frame[PLAG + i].wrapping_add(16);
                }
            } else {
                let max_pitch_gain: f32 = if self.bitrate != Rate::Ifq {
                    1.0
                } else if self.erasure_count < 3 {
                    (0.9 - 0.3 * f64::from(i32::from(self.erasure_count) - 1)) as f32
                } else {
                    0.0
                };
                for g in &mut self.pitch_gain {
                    *g = ffmin(*g, f64::from(max_pitch_gain));
                }
                self.frame[PFRAC..PFRAC + 4].fill(0);
            }

            // pitch synthesis filter
            let pfrac = [self.frame[PFRAC], self.frame[PFRAC + 1], self.frame[PFRAC + 2], self.frame[PFRAC + 3]];
            let synth = do_pitchfilter(&mut self.pitch_synthesis_filter_mem, cdn, &self.pitch_gain, &self.pitch_lag, &pfrac);

            // pitch prefilter update
            for g in &mut self.pitch_gain {
                *g = (0.5 * f64::from(ffmin(*g, 1.0))) as f32;
            }
            let pre = do_pitchfilter(&mut self.pitch_pre_filter_mem, &synth, &self.pitch_gain, &self.pitch_lag, &pfrac);

            // apply_gain_ctrl
            for i in (0..FRAME_SAMPLES).step_by(40) {
                let res = dot(&synth[i..i + 40], &synth[i..i + 40]);
                scale_to_sum_of_squares(&mut cdn[i..i + 40], &pre[i..i + 40], res);
            }
        } else {
            self.pitch_synthesis_filter_mem[..143].copy_from_slice(&cdn[17..160]);
            self.pitch_pre_filter_mem[..143].copy_from_slice(&cdn[17..160]);
            self.pitch_gain = [0.0; 4];
            self.pitch_lag = [0; 4];
        }
    }

    /// `interpolate_lpc`: interpolate the LSP frequencies and compute the LPC coefficients for a
    /// pitch subframe.
    fn interpolate_lpc(&self, curr_lspf: &[f32; 10], lpc: &mut [f32; 10], subframe_num: usize) {
        let weight: f32 = if self.bitrate >= Rate::Quarter {
            (0.25 * (subframe_num + 1) as f64) as f32
        } else if self.bitrate == Rate::Octave && subframe_num == 0 {
            0.625
        } else {
            1.0
        };

        if weight != 1.0 {
            let mut interpolated = [0f32; 10];
            weighted_sum(&mut interpolated, curr_lspf, &self.prev_lspf, weight, (1.0 - f64::from(weight)) as f32);
            lspf2lpc(&interpolated, lpc);
        } else if self.bitrate >= Rate::Quarter || (self.bitrate == Rate::Ifq && subframe_num == 0) {
            lspf2lpc(curr_lspf, lpc);
        } else if self.bitrate == Rate::Silence && subframe_num == 0 {
            lspf2lpc(&self.prev_lspf, lpc);
        }
    }

    /// `postfilter`, as per TIA/EIA/IS-733 2.4.8.6.
    fn postfilter(&mut self, samples: &mut [f32; FRAME_SAMPLES], lpc: &[f32; 10]) {
        const POW_0_775: [f32; 10] = [0.775000, 0.600625, 0.465484, 0.360750, 0.279582, 0.216676, 0.167924, 0.130141, 0.100859, 0.078166];
        const POW_0_625: [f32; 10] = [0.625000, 0.390625, 0.244141, 0.152588, 0.095367, 0.059605, 0.037253, 0.023283, 0.014552, 0.009095];
        let mut lpc_s = [0f32; 10];
        let mut lpc_p = [0f32; 10];
        let mut pole_out = [0f32; 170];
        let mut zero_out = [0f32; 160];

        for n in 0..10 {
            lpc_s[n] = lpc[n] * POW_0_625[n];
            lpc_p[n] = lpc[n] * POW_0_775[n];
        }

        lp_zero_synthesis(&mut zero_out, &lpc_s, &self.formant_mem, 10);
        pole_out[..10].copy_from_slice(&self.postfilter_synth_mem);
        lp_synthesis(&mut pole_out, 10, &lpc_p, &zero_out);
        self.postfilter_synth_mem.copy_from_slice(&pole_out[160..170]);

        tilt_compensation(&mut self.postfilter_tilt_mem, 0.3f32, &mut pole_out[10..170]);

        let speech_energ = dot(&self.formant_mem[10..170], &self.formant_mem[10..170]);
        samples.copy_from_slice(&pole_out[10..170]);
        adaptive_gain_control(samples, speech_energ, 0.9375, &mut self.postfilter_agc_mem);
    }
}

/// `codebook_sanity_check_for_rate_quarter`: false if the check fails.
fn codebook_sanity_check_for_rate_quarter(cbgain: &[u8]) -> bool {
    let mut prev_diff = 0i32;
    for i in 1..5 {
        let diff = i32::from(cbgain[i]) - i32::from(cbgain[i - 1]);
        if diff.abs() > 10 || (diff - prev_diff).abs() > 12 {
            return false;
        }
        prev_diff = diff;
    }
    true
}

/// `do_pitchfilter`: the filter in pitch-subframe steps over `memory` (143 samples of history, then
/// 160 of output), returning the output vector.
fn do_pitchfilter(memory: &mut [f32; 303], v_in: &[f32; FRAME_SAMPLES], gain: &[f32; 4], lag: &[u8; 4], pfrac: &[u8; 4]) -> [f32; FRAME_SAMPLES] {
    for i in 0..4 {
        let first = 143 + 40 * i;
        if gain[i] != 0.0 {
            // The lag is at least 16 for any gain that is not zero and the fractional lag is below
            // 140, so every index is in range; the checks only keep damaged state from panicking.
            let base = first as isize - isize::from(lag[i]);
            for k in 0..40usize {
                let v_lag = base + k as isize;
                let at = |d: isize| -> f32 { usize::try_from(v_lag + d).ok().and_then(|p| memory.get(p).copied()).unwrap_or(0.0) };
                let sample = if pfrac[i] != 0 {
                    let mut v = 0f32;
                    for j in 0..4 {
                        v += HAMMSINC[j as usize] * (at(j - 4) + at(3 - j));
                    }
                    v
                } else {
                    at(0)
                };
                memory[first + k] = v_in[40 * i + k] + gain[i] * sample;
            }
        } else {
            memory[first..first + 40].copy_from_slice(&v_in[40 * i..40 * i + 40]);
        }
    }

    let mut out = [0f32; FRAME_SAMPLES];
    out.copy_from_slice(&memory[143..303]);
    memory.copy_within(160..303, 0);
    out
}

/// `lspf2lpc`: reconstruct the LPC coefficients from the LSP frequencies and expand the bandwidth.
fn lspf2lpc(lspf: &[f32; 10], lpc: &mut [f32; 10]) {
    let mut lsp = [0f64; 10];
    let mut coeff = BANDWIDTH_EXPANSION;
    for i in 0..10 {
        lsp[i] = (std::f64::consts::PI * f64::from(lspf[i])).cos();
    }
    lspd2lpc(&lsp, lpc, 5);
    for v in lpc.iter_mut() {
        *v = (f64::from(*v) * coeff) as f32;
        coeff *= BANDWIDTH_EXPANSION;
    }
}

/// `buf_size2bitrate`.
fn size_to_rate(size: usize) -> Rate {
    match size {
        35 => Rate::Full,
        17 => Rate::Half,
        8 => Rate::Quarter,
        4 => Rate::Octave,
        1 => Rate::Silence,
        _ => Rate::Ifq,
    }
}

/// `determine_bitrate`: the rate from the packet size and its first byte, and the frame data after
/// the rate byte (the whole packet when the rate byte is missing).
fn determine_bitrate(packet: &[u8]) -> (Rate, &[u8]) {
    let size = packet.len();
    let rate = size_to_rate(size);
    if rate >= Rate::Silence {
        let first = i32::from(packet[0]);
        let r = rate as i32;
        let claimed = if r > first {
            // "Claimed bitrate and buffer size mismatch": believe the byte.
            match first {
                0 => Rate::Silence,
                1 => Rate::Octave,
                2 => Rate::Quarter,
                _ => Rate::Half,
            }
        } else if r < first {
            // "Buffer is too small for the claimed bitrate."
            return (Rate::Ifq, packet);
        } else {
            rate
        };
        return (claimed, &packet[1..]);
    }
    let rate = size_to_rate(size + 1);
    if rate >= Rate::Silence {
        // "Bitrate byte missing, guessing bitrate from packet size."
        return (rate, packet);
    }
    (Rate::Ifq, packet)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmaps_cover_every_bit_of_a_frame() {
        let bits = |m: &[(u8, u8, u8)]| m.iter().map(|e| usize::from(e.2)).sum::<usize>();
        assert_eq!(bits(&BITMAP_FULL), 266);
        assert_eq!(bits(&BITMAP_HALF), 124);
        assert_eq!(bits(&BITMAP_QUARTER), 54);
        assert_eq!(bits(&BITMAP_OCTAVE), 20);
        for m in [&BITMAP_FULL[..], &BITMAP_HALF, &BITMAP_QUARTER, &BITMAP_OCTAVE] {
            for &(i, p, l) in m {
                assert!(usize::from(i) < FRAME_BYTES && p + l <= 8);
            }
        }
    }

    #[test]
    fn gain_table_ends_at_one_over_scale() {
        assert_eq!(G12GA[0], (1.0f64 / 8192.0) as f32);
        assert_eq!(G12GA[60], (1000.0f64 / 8192.0) as f32);
    }

    #[test]
    fn rate_is_read_from_size_and_first_byte() {
        let mut p = vec![0u8; 35];
        p[0] = 4;
        assert_eq!(determine_bitrate(&p).0, Rate::Full);
        p[0] = 3; // claims half in a full-size packet
        assert_eq!(determine_bitrate(&p).0, Rate::Half);
        let q = vec![4u8; 17]; // claims full in a half-size packet
        assert_eq!(determine_bitrate(&q).0, Rate::Ifq);
        assert_eq!(determine_bitrate(&[0u8; 34]), (Rate::Full, &[0u8; 34][..]));
        assert_eq!(determine_bitrate(&[]).0, Rate::Silence);
        assert_eq!(determine_bitrate(&[0u8; 20]).0, Rate::Ifq);
    }
}
