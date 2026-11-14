// Ported from WebRTC's iLBC (libilbc v3.0.4), BSD-3-Clause,
// Copyright (c) 2011-2013 The WebRTC project authors (`LICENSE.webrtc`, `PATENTS.webrtc`):
// modules/audio_coding/codecs/ilbc/decode.c, init_decode.c, unpack_bits.c, index_conv_dec.c,
// simple_lsf_dequant.c, lsf_check.c, decoder_interpolate_lsf.c, lsf_interpolate_to_poly_dec.c,
// interpolate.c, lsf_to_poly.c, lsf_to_lsp.c, get_lsp_poly.c, decode_residual.c,
// state_construct.c, cb_construct.c, get_cd_vec.c, create_augmented_vec.c, gain_dequant.c,
// do_plc.c, comp_corr.c, xcorr_coef.c, hp_output.c; common_audio/signal_processing
// filter_ar_fast_q12.c, filter_ma_fast_q12.c, dot_product_with_scale.cc, min_max_operations.c,
// division_operations.c, ilbc_specific_functions.c, include/spl_inl.h.
// CFA4 changes GPL-3.0-or-later.

//! The iLBC decoder (`WebRtcIlbcfix_DecodeImpl` without the enhancer).

use crate::tables::*;

const SUBL: usize = 40;
const STATE_LEN: usize = 80;
const ORDER: usize = 10;
const BLOCKL_MAX: usize = 240;
const NSUB_MAX: usize = 6;
const CB_NSTAGES: usize = 3;
const CB_MEML: usize = 147;
const CB_HALFFILTERLEN: usize = 4;
const CB_FILTERLEN: usize = 8;
const ST_MEM_L_TBL: usize = 85;
const MEM_LF_TBL: usize = 147;
const ENH_BLOCKL: usize = 80;
const LSF_DIM: [usize; 3] = [3, 3, 4];
const LSF_SIZE: [usize; 3] = [64, 128, 128];

/// The frame length: 20 ms (38 bytes, 160 samples) or 30 ms (50 bytes, 240 samples).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Ms20,
    Ms30,
}

impl Mode {
    /// Bytes per frame.
    pub fn frame_bytes(self) -> usize {
        match self {
            Mode::Ms20 => 38,
            Mode::Ms30 => 50,
        }
    }

    /// Samples per frame (8 kHz).
    pub fn frame_samples(self) -> usize {
        match self {
            Mode::Ms20 => 160,
            Mode::Ms30 => 240,
        }
    }
}

/// The parameters of one frame (`iLBC_bits`).
struct Bits {
    lsf: [i16; 6],
    cb_index: [i16; 15],
    gain_index: [i16; 15],
    idx_for_max: usize,
    state_first: bool,
    idx_vec: [i16; 58],
    start_idx: usize,
}

impl Default for Bits {
    fn default() -> Self {
        Self { lsf: [0; 6], cb_index: [0; 15], gain_index: [0; 15], idx_for_max: 0, state_first: false, idx_vec: [0; 58], start_idx: 0 }
    }
}

/// An iLBC decoder (WebRTC's fixed-point decoder with the enhancer off, as ffmpeg decodes).
#[derive(Clone, Debug)]
pub struct Decoder {
    mode: Mode,
    blockl: usize,
    nsub: usize,
    lpc_n: usize,
    state_short_len: usize,
    synt_mem: [i16; ORDER],
    pub(crate) lsfdeqold: [i16; ORDER],
    last_lag: usize,
    cons_pli_count: usize,
    prev_pli: bool,
    per_square: i16,
    prev_lag: usize,
    prev_lpc: [i16; ORDER + 1],
    prev_residual: [i16; BLOCKL_MAX],
    seed: i16,
    hpimemx: [i16; 2],
    hpimemy: [i16; 4],
}

impl Decoder {
    /// `WebRtcIlbcfix_InitDecode`.
    pub fn new(mode: Mode) -> Self {
        let (nsub, lpc_n, state_short_len) = match mode {
            Mode::Ms20 => (4, 1, 57),
            Mode::Ms30 => (6, 2, 58),
        };
        let mut prev_lpc = [0i16; ORDER + 1];
        prev_lpc[0] = 4096;
        Self {
            mode,
            blockl: mode.frame_samples(),
            nsub,
            lpc_n,
            state_short_len,
            synt_mem: [0; ORDER],
            lsfdeqold: LSF_MEAN,
            last_lag: 20,
            cons_pli_count: 0,
            prev_pli: false,
            per_square: 0,
            prev_lag: 120,
            prev_lpc,
            prev_residual: [0; BLOCKL_MAX],
            seed: 777,
            hpimemx: [0; 2],
            hpimemy: [0; 4],
        }
    }

    /// The frame mode.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Decodes one frame (`None` for a lost one) into `out`; true when the frame decoded
    /// normally, false when it was concealed (lost, wrong size, or a bit error detected).
    pub fn decode(&mut self, frame: Option<&[u8]>, out: &mut Vec<i16>) -> bool {
        let blockl = self.blockl;
        let mut decresidual = [0i16; BLOCKL_MAX];
        let mut syntdenum = [0i16; NSUB_MAX * (ORDER + 1)];
        let mut normal = false;
        if let Some(bytes) = frame.filter(|b| b.len() == self.mode.frame_bytes()) {
            let words: Vec<u16> = bytes.chunks(2).map(|c| u16::from_be_bytes([c[0], c.get(1).copied().unwrap_or(0)])).collect();
            let (mut bits, last_bit) = unpack(&words, self.mode);
            let max_start = if self.mode == Mode::Ms20 { 3 } else { 5 };
            normal = bits.start_idx >= 1 && bits.start_idx <= max_start && !last_bit;
            if normal {
                index_conv_dec(&mut bits.cb_index);
                let mut lsfdeq = simple_lsf_deq(&bits.lsf, self.lpc_n);
                lsf_check(&mut lsfdeq, self.lpc_n);
                self.interpolate_lsp(&mut syntdenum, &lsfdeq);
                if !self.decode_residual(&bits, &mut decresidual, &syntdenum) {
                    // `goto error`: WebRTC resets the decoder and outputs nothing; a silent
                    // frame keeps the timeline
                    *self = Self::new(self.mode);
                    out.extend(std::iter::repeat_n(0, blockl));
                    return false;
                }
                let last = (self.nsub - 1) * (ORDER + 1);
                let mut lpc = [0i16; ORDER + 1];
                lpc.copy_from_slice(&syntdenum[last..last + ORDER + 1]);
                let mut plc = [0i16; BLOCKL_MAX];
                self.do_plc(&mut plc, false, &decresidual, &lpc);
                decresidual = plc;
            }
        }
        if !normal {
            let mut plc = [0i16; BLOCKL_MAX];
            let lpc = self.do_plc(&mut plc, true, &decresidual, &[0; ORDER + 1]);
            decresidual = plc;
            for s in syntdenum.chunks_exact_mut(ORDER + 1).take(self.nsub) {
                s.copy_from_slice(&lpc);
            }
        }
        // the enhancer is off: find the last lag for a future loss
        self.last_lag = match self.mode {
            Mode::Ms20 => xcorr_coef(&decresidual, blockl - 60, blockl - 80, 60, 80, 20),
            Mode::Ms30 => xcorr_coef(&decresidual, blockl - ENH_BLOCKL, blockl - ENH_BLOCKL - 20, ENH_BLOCKL, 100, 20),
        };
        let mut data = [0i16; ORDER + BLOCKL_MAX];
        data[..ORDER].copy_from_slice(&self.synt_mem);
        data[ORDER..ORDER + blockl].copy_from_slice(&decresidual[..blockl]);
        for i in 0..self.nsub {
            let start = ORDER + i * SUBL;
            let mut coef = [0i16; ORDER + 1];
            coef.copy_from_slice(&syntdenum[i * (ORDER + 1)..(i + 1) * (ORDER + 1)]);
            filter_ar_q12_in_place(&mut data, start, &coef, SUBL);
        }
        self.synt_mem.copy_from_slice(&data[blockl..blockl + ORDER]);
        let signal = &mut data[ORDER..ORDER + blockl];
        hp_output(signal, &mut self.hpimemy, &mut self.hpimemx);
        out.extend_from_slice(signal);
        normal
    }

    /// `WebRtcIlbcfix_DecoderInterpolateLsp` (the weighting filters are not needed).
    fn interpolate_lsp(&mut self, syntdenum: &mut [i16; NSUB_MAX * (ORDER + 1)], lsfdeq: &[i16; 2 * ORDER]) {
        let mut cur = [0i16; ORDER];
        cur.copy_from_slice(&lsfdeq[..ORDER]);
        let mut second = [0i16; ORDER];
        second.copy_from_slice(&lsfdeq[ORDER..]);
        let lp_len = ORDER + 1;
        match self.mode {
            Mode::Ms30 => {
                syntdenum[..lp_len].copy_from_slice(&lsf_interpolate_to_poly(&self.lsfdeqold, &cur, LSF_WEIGHT_30_MS[0]));
                for i in 1..6 {
                    syntdenum[i * lp_len..(i + 1) * lp_len].copy_from_slice(&lsf_interpolate_to_poly(&cur, &second, LSF_WEIGHT_30_MS[i]));
                }
                self.lsfdeqold = second;
            }
            Mode::Ms20 => {
                for i in 0..self.nsub {
                    syntdenum[i * lp_len..(i + 1) * lp_len].copy_from_slice(&lsf_interpolate_to_poly(&self.lsfdeqold, &cur, LSF_WEIGHT_20_MS[i]));
                }
                self.lsfdeqold = cur;
            }
        }
    }

    /// `WebRtcIlbcfix_DecodeResidual`.
    fn decode_residual(&self, bits: &Bits, decresidual: &mut [i16; BLOCKL_MAX], syntdenum: &[i16; NSUB_MAX * (ORDER + 1)]) -> bool {
        let ssl = self.state_short_len;
        let diff = STATE_LEN - ssl;
        let start = bits.start_idx - 1;
        let start_pos = if bits.state_first { start * SUBL } else { start * SUBL + diff };
        let mut coef = [0i16; ORDER + 1];
        coef.copy_from_slice(&syntdenum[start * (ORDER + 1)..(start + 1) * (ORDER + 1)]);
        let state = state_construct(bits.idx_for_max, &bits.idx_vec, &coef, ssl);
        decresidual[start_pos..start_pos + ssl].copy_from_slice(&state[..ssl]);
        // the codebook memory, with CB_HALFFILTERLEN samples of room on each side
        let mut mem_vec = [0i16; CB_HALFFILTERLEN + CB_MEML + CB_HALFFILTERLEN];
        const M: usize = CB_HALFFILTERLEN;
        let mut reverse = [0i16; BLOCKL_MAX];
        let stage = |n: usize| -> ([i16; CB_NSTAGES], [i16; CB_NSTAGES]) {
            let mut c = [0i16; CB_NSTAGES];
            let mut g = [0i16; CB_NSTAGES];
            c.copy_from_slice(&bits.cb_index[n * CB_NSTAGES..(n + 1) * CB_NSTAGES]);
            g.copy_from_slice(&bits.gain_index[n * CB_NSTAGES..(n + 1) * CB_NSTAGES]);
            (c, g)
        };
        let (c, g) = stage(0);
        if bits.state_first {
            mem_vec[M..M + CB_MEML - ssl].fill(0);
            mem_vec[M + CB_MEML - ssl..M + CB_MEML].copy_from_slice(&decresidual[start_pos..start_pos + ssl]);
            let mut v = [0i16; SUBL];
            if !cb_construct(&mut v[..diff], &c, &g, &mut mem_vec, M + CB_MEML - ST_MEM_L_TBL, ST_MEM_L_TBL) {
                return false;
            }
            decresidual[start_pos + ssl..start_pos + ssl + diff].copy_from_slice(&v[..diff]);
        } else {
            for k in 0..ssl {
                mem_vec[M + CB_MEML - 1 - k] = decresidual[start_pos + k];
            }
            mem_vec[M..M + CB_MEML - ssl].fill(0);
            if !cb_construct(&mut reverse[..diff], &c, &g, &mut mem_vec, M + CB_MEML - ST_MEM_L_TBL, ST_MEM_L_TBL) {
                return false;
            }
            for k in 0..diff {
                decresidual[start_pos - 1 - k] = reverse[k];
            }
        }
        let mut subcount = 1;
        if self.nsub > bits.start_idx + 1 {
            mem_vec[M..M + CB_MEML - STATE_LEN].fill(0);
            mem_vec[M + CB_MEML - STATE_LEN..M + CB_MEML].copy_from_slice(&decresidual[start * SUBL..start * SUBL + STATE_LEN]);
            let nfor = self.nsub - bits.start_idx - 1;
            for sub in 0..nfor {
                let (c, g) = stage(subcount);
                let at = (bits.start_idx + 1 + sub) * SUBL;
                let mut v = [0i16; SUBL];
                if !cb_construct(&mut v, &c, &g, &mut mem_vec, M, MEM_LF_TBL) {
                    return false;
                }
                decresidual[at..at + SUBL].copy_from_slice(&v);
                mem_vec.copy_within(M + SUBL..M + CB_MEML, M);
                mem_vec[M + CB_MEML - SUBL..M + CB_MEML].copy_from_slice(&v);
                subcount += 1;
            }
        }
        if bits.start_idx > 1 {
            let gotten = (SUBL * (self.nsub + 1 - bits.start_idx)).min(CB_MEML);
            for k in 0..gotten {
                mem_vec[M + CB_MEML - 1 - k] = decresidual[start * SUBL + k];
            }
            mem_vec[M..M + CB_MEML - gotten].fill(0);
            let nback = bits.start_idx - 1;
            for sub in 0..nback {
                let (c, g) = stage(subcount);
                let mut v = [0i16; SUBL];
                if !cb_construct(&mut v, &c, &g, &mut mem_vec, M, MEM_LF_TBL) {
                    return false;
                }
                reverse[sub * SUBL..(sub + 1) * SUBL].copy_from_slice(&v);
                mem_vec.copy_within(M + SUBL..M + CB_MEML, M);
                mem_vec[M + CB_MEML - SUBL..M + CB_MEML].copy_from_slice(&v);
                subcount += 1;
            }
            for k in 0..SUBL * nback {
                decresidual[SUBL * nback - 1 - k] = reverse[k];
            }
        }
        true
    }

    /// `WebRtcIlbcfix_DoThePlc`: fills `plc` and returns the LPC filter to use.
    fn do_plc(&mut self, plc: &mut [i16; BLOCKL_MAX], pli: bool, decresidual: &[i16; BLOCKL_MAX], lpc: &[i16; ORDER + 1]) -> [i16; ORDER + 1] {
        let blockl = self.blockl;
        let out_lpc;
        if pli {
            self.cons_pli_count += 1;
            let lag;
            let max_per_square;
            if !self.prev_pli {
                let prev = &self.prev_residual[..blockl];
                let max = max_abs(prev);
                let scale3 = ((size_in_bits(max as u32) << 1) - 25).max(0);
                let inlag = self.last_lag;
                let mut best = inlag.saturating_sub(3);
                let corr_len = 60.min(blockl.saturating_sub(inlag + 3));
                let (mut cross, mut ener) = comp_corr(prev, best, corr_len, scale3);
                let mut shift_max = size_in_bits(cross.wrapping_abs() as u32) - 15;
                let c = shift_w32(cross, -shift_max) as i16;
                let mut cross_square_max = ((i32::from(c) * i32::from(c)) >> 15) as i16;
                let mut ener_comp = 0;
                for j in inlag.saturating_sub(2)..=inlag + 3 {
                    let (cross_comp, e) = comp_corr(prev, j, corr_len, scale3);
                    ener_comp = e;
                    let shift1 = size_in_bits(cross_comp.wrapping_abs() as u32) - 15;
                    let c = shift_w32(cross_comp, -shift1) as i16;
                    let cross_square = ((i32::from(c) * i32::from(c)) >> 15) as i16;
                    let shift2 = size_in_bits(ener as u32) - 15;
                    let measure = i32::from(shift_w32(ener, -shift2) as i16) * i32::from(cross_square);
                    let shift3 = size_in_bits(ener_comp as u32) - 15;
                    let max_measure = i32::from(shift_w32(ener_comp, -shift3) as i16) * i32::from(cross_square_max);
                    let (t1, t2) = if 2 * shift_max + shift3 > 2 * shift1 + shift2 {
                        ((2 * shift_max + shift3 - 2 * shift1 - shift2).min(31), 0)
                    } else {
                        (0, (2 * shift1 + shift2 - 2 * shift_max - shift3).min(31))
                    };
                    if (measure >> t1) > (max_measure >> t2) {
                        best = j;
                        cross_square_max = cross_square;
                        cross = cross_comp;
                        shift_max = shift1;
                        ener = ener_comp;
                    }
                }
                lag = best;
                let tail = &prev[blockl - corr_len..];
                let t = dot(tail, tail, scale3);
                max_per_square = if t > 0 && ener_comp > 0 {
                    let scale1 = norm_w32(t) - 16;
                    let t1 = shift_w32(t, scale1) as i16;
                    let scale2 = norm_w32(ener) - 16;
                    let t2 = shift_w32(ener, scale2) as i16;
                    let denom = ((i32::from(t1) * i32::from(t2)) >> 16) as i16;
                    let totscale = scale1 + scale2 - 1;
                    let a = shift_w32(cross, totscale >> 1) as i16;
                    let b = shift_w32(cross, totscale - (totscale >> 1)) as i16;
                    let nom = i32::from(a) * i32::from(b);
                    (if denom != 0 { nom.wrapping_div(i32::from(denom)) } else { i32::MAX }) as i16
                } else {
                    0
                };
            } else {
                lag = self.prev_lag;
                max_per_square = self.per_square;
            }
            let lost = self.cons_pli_count.saturating_mul(blockl);
            // WebRTC's chain: only the first test can hold
            let use_gain: i32 = if lost > 320 { 29491 } else { 32767 };
            let pitchfact: i32 = if max_per_square > 7868 {
                32767
            } else if max_per_square > 839 {
                let mut ind = 5;
                while ind > 0 && max_per_square < PLC_PER_SQR[ind] {
                    ind -= 1;
                }
                let t = i32::from(PLC_PITCH_FACT[ind]) + ((i32::from(PLC_PF_SLOPE[ind]) * (i32::from(max_per_square) - i32::from(PLC_PER_SQR[ind]))) >> 11);
                t.min(32767)
            } else {
                0
            };
            let use_lag = if lag < 80 { 2 * lag } else { lag };
            let threshold = blockl as i32 * 900;
            let mut energy = 0i32;
            let mut randvec = [0i16; BLOCKL_MAX];
            let prev = self.prev_residual;
            let at = |k: isize| prev.get(usize::try_from(k).unwrap_or(usize::MAX)).copied().unwrap_or(0);
            for i in 0..blockl {
                self.seed = (i32::from(self.seed).wrapping_mul(31821).wrapping_add(13849)) as i16;
                let randlag = 53 + (self.seed & 63) as usize;
                randvec[i] = if randlag > i { at((blockl + i) as isize - randlag as isize) } else { at(i as isize - randlag as isize) };
                let p = if use_lag > i { at((blockl + i) as isize - use_lag as isize) } else { plc.get(i - use_lag).copied().unwrap_or(0) };
                let tot_gain = if i < 80 {
                    use_gain
                } else if i < 160 {
                    i32::from(((31130 * use_gain) >> 15) as i16)
                } else {
                    i32::from(((29491 * use_gain) >> 15) as i16)
                };
                let mixed = (pitchfact * i32::from(p)).wrapping_add((32767 - pitchfact) * i32::from(randvec[i])).wrapping_add(16384) >> 15;
                plc[i] = (tot_gain.wrapping_mul(mixed) >> 15) as i16;
                if energy < threshold {
                    energy = energy.wrapping_add(i32::from(plc[i]) * i32::from(plc[i]));
                }
            }
            if energy < threshold {
                plc[..blockl].copy_from_slice(&randvec[..blockl]);
            }
            out_lpc = self.prev_lpc;
            self.prev_lag = lag;
            self.per_square = max_per_square;
        } else {
            plc[..blockl].copy_from_slice(&decresidual[..blockl]);
            out_lpc = *lpc;
            self.cons_pli_count = 0;
        }
        self.prev_pli = pli;
        self.prev_lpc = out_lpc;
        self.prev_residual[..blockl].copy_from_slice(&plc[..blockl]);
        out_lpc
    }
}

/// `WebRtcIlbcfix_UnpackBits` on the big-endian 16-bit words; returns the parameters and the
/// last (must-be-zero) bit.
fn unpack(words: &[u16], mode: Mode) -> (Bits, bool) {
    let mut b = Bits::default();
    let mut p = 0usize;
    let w = |p: usize| i32::from(words.get(p).copied().unwrap_or(0));
    let mut cb = [0i32; 15];
    let mut gi = [0i32; 15];
    let mut lsf = [0i32; 6];
    let mut idx = [0i32; 58];
    let start;
    let first;
    let mut ifm;
    lsf[0] = w(p) >> 10;
    lsf[1] = (w(p) >> 3) & 0x7f;
    lsf[2] = (w(p) & 7) << 4;
    p += 1;
    lsf[2] |= (w(p) >> 12) & 0xf;
    if mode == Mode::Ms20 {
        start = (w(p) >> 10) & 3;
        first = (w(p) >> 9) & 1;
        ifm = (w(p) >> 3) & 0x3f;
        cb[0] = (w(p) & 7) << 4;
        p += 1;
        cb[0] |= (w(p) >> 12) & 0xe;
        gi[0] = (w(p) >> 8) & 0x18;
        gi[1] = (w(p) >> 7) & 0x8;
        cb[3] = (w(p) >> 2) & 0xfe;
        gi[3] = (w(p) << 2) & 0x10;
        gi[4] = (w(p) << 2) & 0x8;
        gi[6] = (w(p) << 4) & 0x10;
    } else {
        lsf[3] = (w(p) >> 6) & 0x3f;
        lsf[4] = (w(p) << 1) & 0x7e;
        p += 1;
        lsf[4] |= (w(p) >> 15) & 1;
        lsf[5] = (w(p) >> 8) & 0x7f;
        start = (w(p) >> 5) & 7;
        first = (w(p) >> 4) & 1;
        ifm = (w(p) << 2) & 0x3c;
        p += 1;
        ifm |= (w(p) >> 14) & 3;
        cb[0] = (w(p) >> 7) & 0x78;
        gi[0] = (w(p) >> 5) & 0x10;
        gi[1] = (w(p) >> 5) & 0x8;
        cb[3] = w(p) & 0xfc;
        gi[3] = (w(p) << 3) & 0x10;
        gi[4] = (w(p) << 3) & 0x8;
    }
    p += 1;
    let mut t = 0usize;
    for _ in 0..3 {
        for i in (0..16).rev() {
            idx[t] = ((w(p) >> i) << 2) & 4;
            t += 1;
        }
        p += 1;
    }
    if mode == Mode::Ms20 {
        for i in (7..16).rev() {
            idx[t] = ((w(p) >> i) << 2) & 4;
            t += 1;
        }
        gi[1] |= (w(p) >> 4) & 4;
        gi[3] |= (w(p) >> 2) & 0xc;
        gi[4] |= (w(p) >> 1) & 4;
        gi[6] |= (w(p) << 1) & 8;
        gi[7] = (w(p) << 2) & 0xc;
    } else {
        for i in (6..16).rev() {
            idx[t] = ((w(p) >> i) << 2) & 4;
            t += 1;
        }
        cb[0] |= (w(p) >> 3) & 6;
        gi[0] |= w(p) & 8;
        gi[1] |= w(p) & 4;
        cb[3] |= w(p) & 2;
        cb[6] = (w(p) << 7) & 0x80;
        p += 1;
        cb[6] |= (w(p) >> 9) & 0x7e;
        cb[9] = (w(p) >> 2) & 0xfe;
        cb[12] = (w(p) << 5) & 0xe0;
        p += 1;
        cb[12] |= (w(p) >> 11) & 0x1e;
        gi[3] |= (w(p) >> 8) & 0xc;
        gi[4] |= (w(p) >> 7) & 6;
        gi[6] = (w(p) >> 3) & 0x18;
        gi[7] = (w(p) >> 2) & 0xc;
        gi[9] = (w(p) << 1) & 0x10;
        gi[10] = (w(p) << 1) & 8;
        gi[12] = (w(p) << 3) & 0x10;
        gi[13] = (w(p) << 3) & 8;
    }
    p += 1;
    t = 0;
    for _ in 0..7 {
        for i in (0..=14).rev().step_by(2) {
            idx[t] |= (w(p) >> i) & 3;
            t += 1;
        }
        p += 1;
    }
    if mode == Mode::Ms20 {
        idx[56] |= (w(p) >> 14) & 3;
        cb[0] |= (w(p) >> 13) & 1;
        cb[1] = (w(p) >> 6) & 0x7f;
        cb[2] = (w(p) << 1) & 0x7e;
        p += 1;
        cb[2] |= (w(p) >> 15) & 1;
        gi[0] |= (w(p) >> 12) & 7;
        gi[1] |= (w(p) >> 10) & 3;
        gi[2] = (w(p) >> 7) & 7;
        cb[3] |= (w(p) >> 6) & 1;
        cb[4] = (w(p) << 1) & 0x7e;
        p += 1;
        cb[4] |= (w(p) >> 15) & 1;
        cb[5] = (w(p) >> 8) & 0x7f;
        cb[6] = w(p) & 0xff;
        p += 1;
        cb[7] = w(p) >> 8;
        cb[8] = w(p) & 0xff;
        p += 1;
        gi[3] |= (w(p) >> 14) & 3;
        gi[4] |= (w(p) >> 12) & 3;
        gi[5] = (w(p) >> 9) & 7;
        gi[6] |= (w(p) >> 6) & 7;
        gi[7] |= (w(p) >> 4) & 3;
        gi[8] = (w(p) >> 1) & 7;
    } else {
        idx[56] |= (w(p) >> 14) & 3;
        idx[57] |= (w(p) >> 12) & 3;
        cb[0] |= (w(p) >> 11) & 1;
        cb[1] = (w(p) >> 4) & 0x7f;
        cb[2] = (w(p) << 3) & 0x78;
        p += 1;
        cb[2] |= (w(p) >> 13) & 7;
        gi[0] |= (w(p) >> 10) & 7;
        gi[1] |= (w(p) >> 8) & 3;
        gi[2] = (w(p) >> 5) & 7;
        cb[3] |= (w(p) >> 4) & 1;
        cb[4] = (w(p) << 3) & 0x78;
        p += 1;
        cb[4] |= (w(p) >> 13) & 7;
        cb[5] = (w(p) >> 6) & 0x7f;
        cb[6] |= (w(p) >> 5) & 1;
        cb[7] = (w(p) << 3) & 0xf8;
        p += 1;
        cb[7] |= (w(p) >> 13) & 7;
        cb[8] = (w(p) >> 5) & 0xff;
        cb[9] |= (w(p) >> 4) & 1;
        cb[10] = (w(p) << 4) & 0xf0;
        p += 1;
        cb[10] |= (w(p) >> 12) & 0xf;
        cb[11] = (w(p) >> 4) & 0xff;
        cb[12] |= (w(p) >> 3) & 1;
        cb[13] = (w(p) << 5) & 0xe0;
        p += 1;
        cb[13] |= (w(p) >> 11) & 0x1f;
        cb[14] = (w(p) >> 3) & 0xff;
        gi[3] |= (w(p) >> 1) & 3;
        gi[4] |= w(p) & 1;
        p += 1;
        gi[5] = (w(p) >> 13) & 7;
        gi[6] |= (w(p) >> 10) & 7;
        gi[7] |= (w(p) >> 8) & 3;
        gi[8] = (w(p) >> 5) & 7;
        gi[9] |= (w(p) >> 1) & 0xf;
        gi[10] |= (w(p) << 2) & 4;
        p += 1;
        gi[10] |= (w(p) >> 14) & 3;
        gi[11] = (w(p) >> 11) & 7;
        gi[12] |= (w(p) >> 7) & 0xf;
        gi[13] |= (w(p) >> 4) & 7;
        gi[14] = (w(p) >> 1) & 7;
    }
    let last_bit = w(p) & 1 == 1;
    b.lsf = lsf.map(|v| v as i16);
    b.cb_index = cb.map(|v| v as i16);
    b.gain_index = gi.map(|v| v as i16);
    b.idx_vec = idx.map(|v| v as i16);
    b.start_idx = start as usize;
    b.state_first = first == 1;
    b.idx_for_max = ifm as usize;
    (b, last_bit)
}

/// `WebRtcIlbcfix_IndexConvDec`.
fn index_conv_dec(index: &mut [i16; 15]) {
    for v in &mut index[4..6] {
        if (44..108).contains(v) {
            *v += 64;
        } else if (108..128).contains(v) {
            *v += 128;
        }
    }
}

/// `WebRtcIlbcfix_SimpleLsfDeQ`.
fn simple_lsf_deq(index: &[i16; 6], lpc_n: usize) -> [i16; 2 * ORDER] {
    let mut out = [0i16; 2 * ORDER];
    for n in 0..lpc_n.min(2) {
        let mut pos = 0;
        let mut cb_pos = 0;
        for i in 0..3 {
            let idx = usize::try_from(index[n * 3 + i]).unwrap_or(0);
            for j in 0..LSF_DIM[i] {
                out[n * ORDER + pos + j] = LSF_CB.get(cb_pos + idx * LSF_DIM[i] + j).copied().unwrap_or(0);
            }
            pos += LSF_DIM[i];
            cb_pos += LSF_SIZE[i] * LSF_DIM[i];
        }
    }
    out
}

/// `WebRtcIlbcfix_LsfCheck`.
fn lsf_check(lsf: &mut [i16; 2 * ORDER], n_an: usize) {
    const EPS: i32 = 319;
    const EPS2: i16 = 160;
    const MAXLSF: i16 = 25723;
    const MINLSF: i16 = 82;
    for _ in 0..2 {
        for m in 0..n_an.min(2) {
            for k in 0..ORDER - 1 {
                let pos = m * ORDER + k;
                if i32::from(lsf[pos + 1]) - i32::from(lsf[pos]) < EPS {
                    if lsf[pos + 1] < lsf[pos] {
                        lsf[pos + 1] = lsf[pos].wrapping_add(EPS2);
                        lsf[pos] = lsf[pos + 1].wrapping_sub(EPS2);
                    } else {
                        lsf[pos] = lsf[pos].wrapping_sub(EPS2);
                        lsf[pos + 1] = lsf[pos + 1].wrapping_add(EPS2);
                    }
                }
                lsf[pos] = lsf[pos].clamp(MINLSF, MAXLSF);
            }
        }
    }
}

/// `WebRtcIlbcfix_LspInterpolate2PolyDec`: interpolates two LSF vectors and converts to LPC.
fn lsf_interpolate_to_poly(lsf1: &[i16; ORDER], lsf2: &[i16; ORDER], coef: i16) -> [i16; ORDER + 1] {
    let coef = i32::from(coef);
    let inv = 16384 - coef;
    let mut lsf = [0i16; ORDER];
    for i in 0..ORDER {
        lsf[i] = ((coef * i32::from(lsf1[i]) + inv * i32::from(lsf2[i]) + 8192) >> 14) as i16;
    }
    lsf_to_poly(&lsf)
}

/// `WebRtcIlbcfix_Lsf2Poly`.
fn lsf_to_poly(lsf: &[i16; ORDER]) -> [i16; ORDER + 1] {
    let mut lsp = [0i16; ORDER];
    for i in 0..ORDER {
        let freq = ((i32::from(lsf[i]) * 20861) >> 15) as i16;
        let k = usize::try_from(freq >> 8).unwrap_or(0).min(63);
        let diff = i32::from(freq & 0xff);
        let t = i32::from(COS_DERIVATIVE[k]) * diff;
        lsp[i] = COS[k].wrapping_add((t >> 12) as i16);
    }
    let mut f1 = lsp_poly(&lsp, 0);
    let mut f2 = lsp_poly(&lsp, 1);
    for i in (1..6).rev() {
        f1[i] = f1[i].wrapping_add(f1[i - 1]);
        f2[i] = f2[i].wrapping_sub(f2[i - 1]);
    }
    let mut a = [0i16; ORDER + 1];
    a[0] = 4096;
    for i in 0..5 {
        a[1 + i] = (f1[1 + i].wrapping_add(f2[1 + i]).wrapping_add(4096) >> 13) as i16;
        a[10 - i] = (f1[1 + i].wrapping_sub(f2[1 + i]).wrapping_add(4096) >> 13) as i16;
    }
    a
}

/// `WebRtcIlbcfix_GetLspPoly` on `lsp[first]`, `lsp[first + 2]`, …
fn lsp_poly(lsp: &[i16; ORDER], first: usize) -> [i32; 6] {
    let l = |k: usize| i32::from(lsp[(first + 2 * k).min(ORDER - 1)]);
    let mut f = [0i32; 6];
    f[0] = 16_777_216;
    f[1] = l(0) * -1024;
    for i in 2..=5 {
        let c = l(i - 1);
        f[i] = f[i - 2];
        for j in (2..=i).rev() {
            let high = i32::from((f[j - 1] >> 16) as i16);
            let low = i32::from(((f[j - 1] & 0xffff) >> 1) as i16);
            let t = (high * c).wrapping_mul(4).wrapping_add(4 * ((low * c) >> 15));
            f[j] = f[j].wrapping_add(f[j - 2]).wrapping_sub(t);
        }
        f[1] = f[1].wrapping_sub(c * 1024);
    }
    f
}

/// `WebRtcIlbcfix_StateConstruct`.
fn state_construct(idx_for_max: usize, idx_vec: &[i16; 58], synt: &[i16; ORDER + 1], len: usize) -> [i16; 58] {
    let len = len.min(58);
    let mut numerator = [0i16; ORDER + 1];
    for k in 0..=ORDER {
        numerator[k] = synt[ORDER - k];
    }
    let max_val = i32::from(FRG_QUANT_MOD[idx_for_max & 63]);
    let (round, shift) = if idx_for_max < 37 {
        (2_097_152, 22)
    } else if idx_for_max < 59 {
        (262_144, 19)
    } else {
        (65536, 17)
    };
    // sample_val with ORDER zeros of history in front
    let mut val = [0i16; ORDER + 2 * 58];
    for k in 0..len {
        let q = usize::try_from(idx_vec[len - 1 - k]).unwrap_or(0) & 7;
        val[ORDER + k] = ((max_val * i32::from(STATE_SQ_3[q]) + round) >> shift) as i16;
    }
    let mut ma = [0i16; ORDER + 2 * 58];
    filter_ma_q12(&val, ORDER, &mut ma[ORDER..], &numerator, len + ORDER);
    // the AR filter writes over `val` (as WebRTC's `sampleAr`), whose history is still zero
    for i in 0..2 * len {
        let mut sum = 0i32;
        for j in 1..=ORDER {
            sum = sum.wrapping_add(i32::from(synt[j]) * i32::from(val[ORDER + i - j]));
        }
        let o = (i32::from(synt[0]) * i32::from(ma[ORDER + i])).wrapping_sub(sum).clamp(-134_217_728, 134_215_679);
        val[ORDER + i] = ((o + 2048) >> 12) as i16;
    }
    let mut out = [0i16; 58];
    for k in 0..len {
        out[k] = val[ORDER + len - 1 - k].wrapping_add(val[ORDER + 2 * len - 1 - k]);
    }
    out
}

/// `WebRtcIlbcfix_GainDequant`.
fn gain_dequant(index: i16, max_in: i16, stage: usize) -> i16 {
    let scale = if max_in >= 0 { max_in } else { (-i32::from(max_in)) as i16 };
    let scale = i32::from(scale.max(1638));
    let table: &[i16] = match stage {
        0 => &GAIN_SQ_5,
        1 => &GAIN_SQ_4,
        _ => &GAIN_SQ_3,
    };
    let g = i32::from(table.get(usize::try_from(index).unwrap_or(0)).copied().unwrap_or(0));
    ((scale * g + 8192) >> 14) as i16
}

/// `WebRtcIlbcfix_CbConstruct`: `mem` is the codebook memory at `mem_vec[base..base + l_mem]`.
fn cb_construct(out: &mut [i16], index: &[i16; CB_NSTAGES], gain_index: &[i16; CB_NSTAGES], mem_vec: &mut [i16], base: usize, l_mem: usize) -> bool {
    let veclen = out.len();
    let g0 = gain_dequant(gain_index[0], 16384, 0);
    let g1 = gain_dequant(gain_index[1], g0, 1);
    let g2 = gain_dequant(gain_index[2], g1, 2);
    let mut v = [[0i16; SUBL]; CB_NSTAGES];
    for s in 0..CB_NSTAGES {
        let idx = usize::try_from(index[s]).unwrap_or(0);
        if !get_cb_vec(&mut v[s][..veclen], mem_vec, base, idx, l_mem) {
            return false;
        }
    }
    for j in 0..veclen {
        let a = i32::from(g0) * i32::from(v[0][j]) + i32::from(g1) * i32::from(v[1][j]);
        let a = a.wrapping_add(i32::from(g2) * i32::from(v[2][j]));
        out[j] = (a.wrapping_add(8192) >> 14) as i16;
    }
    true
}

/// `WebRtcIlbcfix_GetCbVec`.
fn get_cb_vec(cbvec: &mut [i16], mem_vec: &mut [i16], base: usize, index: usize, l_mem: usize) -> bool {
    let len = cbvec.len();
    let first = l_mem - len + 1;
    let base_size = if len == SUBL { first + len / 2 } else { first };
    if index < first {
        let k = index + len;
        cbvec.copy_from_slice(&mem_vec[base + l_mem - k..base + l_mem - k + len]);
    } else if index < base_size {
        let lag = (2 * (index - first) + len) / 2;
        let end = base + l_mem;
        augmented_vec(lag, &mem_vec[..end], cbvec);
    } else if index - base_size < first {
        let mem_ind = l_mem - (index - base_size + len);
        mem_vec[base - CB_HALFFILTERLEN..base].fill(0);
        mem_vec[base + l_mem..base + l_mem + CB_HALFFILTERLEN].fill(0);
        filter_ma_q12(mem_vec, base + mem_ind + 4, cbvec, &CB_FILTERS_REV, len);
    } else {
        if len < SUBL {
            return false;
        }
        let mem_ind = l_mem - len - CB_FILTERLEN;
        mem_vec[base + l_mem..base + l_mem + CB_HALFFILTERLEN].fill(0);
        let mut temp = [0i16; SUBL + 5];
        filter_ma_q12(mem_vec, base + mem_ind + 7, &mut temp, &CB_FILTERS_REV, len + 5);
        let lag = (len << 1) + index - 20 - base_size - l_mem - 1;
        augmented_vec(lag, &temp, cbvec);
    }
    true
}

/// `WebRtcIlbcfix_CreateAugmentedVec`: `buffer` ends where WebRTC's `buffer` pointer points.
fn augmented_vec(index: usize, buffer: &[i16], cbvec: &mut [i16]) {
    let end = buffer.len();
    let index = index.min(end.saturating_sub(4)).min(SUBL);
    let interp = index.min(4);
    let ilow = index - interp;
    let n = index.min(cbvec.len());
    cbvec[..n].copy_from_slice(&buffer[end - index..end - index + n]);
    for i in 0..interp {
        let a = (i32::from(buffer[end - index - interp + i]) * i32::from(ALPHA[i])) >> 15;
        let b = (i32::from(buffer[end - interp + i]) * i32::from(ALPHA[interp - 1 - i])) >> 15;
        if let Some(c) = cbvec.get_mut(ilow + i) {
            *c = (i32::from(a as i16) + i32::from(b as i16)) as i16;
        }
    }
    let rest = (SUBL - index).min(index).min(cbvec.len().saturating_sub(index));
    cbvec[index..index + rest].copy_from_slice(&buffer[end - index..end - index + rest]);
}

/// `WebRtcSpl_FilterMAFastQ12` reading `input[at + i - j]`.
fn filter_ma_q12(input: &[i16], at: usize, out: &mut [i16], b: &[i16], len: usize) {
    for i in 0..len.min(out.len()) {
        let mut o = 0i32;
        for (j, &c) in b.iter().enumerate() {
            let v = (at + i).checked_sub(j).and_then(|k| input.get(k)).copied().unwrap_or(0);
            o = o.wrapping_add(i32::from(c) * i32::from(v));
        }
        let o = o.clamp(-134_217_728, 134_215_679);
        out[i] = ((o + 2048) >> 12) as i16;
    }
}

/// `WebRtcSpl_FilterARFastQ12` with input and output both `data[at..at + len]` and the history
/// before `at`.
fn filter_ar_q12_in_place(data: &mut [i16], at: usize, coef: &[i16; ORDER + 1], len: usize) {
    for i in at..at + len {
        // 32-bit accumulation that wraps, as the C code does
        let mut sum = 0i32;
        for j in 1..=ORDER {
            sum = sum.wrapping_add(i32::from(coef[j]) * i32::from(data[i - j]));
        }
        let o = (i32::from(coef[0]) * i32::from(data[i])).wrapping_sub(sum).clamp(-134_217_728, 134_215_679);
        data[i] = ((o + 2048) >> 12) as i16;
    }
}

/// `WebRtcIlbcfix_HpOutput`: the output high-pass filter with ×2 upscaling.
fn hp_output(signal: &mut [i16], y: &mut [i16; 4], x: &mut [i16; 2]) {
    let ba = HP_OUT_COEFS.map(i32::from);
    for s in signal {
        let mut t = i32::from(y[1]) * ba[3] + i32::from(y[3]) * ba[4];
        t >>= 15;
        t += i32::from(y[0]) * ba[3] + i32::from(y[2]) * ba[4];
        t = t.wrapping_mul(2);
        t = t.wrapping_add(i32::from(*s) * ba[0]).wrapping_add(i32::from(x[0]) * ba[1]).wrapping_add(i32::from(x[1]) * ba[2]);
        x[1] = x[0];
        x[0] = *s;
        let b = t.wrapping_add(1024).clamp(-67_108_864, 67_108_863);
        *s = (b >> 11) as i16;
        y[2] = y[0];
        y[3] = y[1];
        let t = if t > 268_435_455 {
            i32::MAX
        } else if t < -268_435_456 {
            i32::MIN
        } else {
            t * 8
        };
        y[0] = (t >> 16) as i16;
        y[1] = ((t & 0xffff) >> 1) as i16;
    }
}

/// `WebRtcIlbcfix_XcorrCoef` with step −1: `target` at `decres[t..t + subl]`, the regressor at
/// `decres[r..]`, searched backwards.
fn xcorr_coef(decres: &[i16; BLOCKL_MAX], t: usize, r: usize, subl: usize, search_len: usize, offset: usize) -> usize {
    let at = |k: isize| decres.get(usize::try_from(k).unwrap_or(usize::MAX)).copied().unwrap_or(0);
    let r = r as isize;
    let lo = (r - search_len as isize).max(0) as usize;
    let max = max_abs(&decres[lo..(lo + subl + search_len - 1).min(BLOCKL_MAX)]);
    let shifts = if max > 5000 { 2 } else { 0 };
    let slice = |s: isize| -> [i16; ENH_BLOCKL] {
        let mut v = [0i16; ENH_BLOCKL];
        for (i, x) in v.iter_mut().take(subl).enumerate() {
            *x = at(s + i as isize);
        }
        v
    };
    let target = slice(t as isize);
    let reg0 = slice(r);
    let mut energy = dot(&reg0[..subl], &reg0[..subl], shifts);
    let mut cross_sq_max: i16 = 0;
    let mut energy_mod_max: i16 = i16::MAX;
    let mut totscale_max: i32 = -500;
    let mut maxlag = 0usize;
    let mut pos = 0isize;
    let mut rp_beg = r - 1;
    let mut rp_end = r + subl as isize - 1;
    for k in 0..search_len {
        let reg = slice(r + pos);
        let cross = dot(&target[..subl], &reg[..subl], shifts);
        if energy > 0 && cross > 0 {
            let cross_scale = norm_w32(cross) - 16;
            let cross_mod = shift_w32(cross, cross_scale) as i16;
            let energy_scale = norm_w32(energy) - 16;
            let energy_mod = shift_w32(energy, energy_scale) as i16;
            let cross_sq = ((i32::from(cross_mod) * i32::from(cross_mod)) >> 16) as i16;
            let totscale = energy_scale - (cross_scale << 1);
            let scalediff = (totscale - totscale_max).clamp(-31, 31);
            let (new_crit, max_crit) = if scalediff < 0 {
                ((i32::from(cross_sq) * i32::from(energy_mod_max)) >> -scalediff, i32::from(cross_sq_max) * i32::from(energy_mod))
            } else {
                (i32::from(cross_sq) * i32::from(energy_mod_max), (i32::from(cross_sq_max) * i32::from(energy_mod)) >> scalediff)
            };
            if new_crit > max_crit {
                cross_sq_max = cross_sq;
                energy_mod_max = energy_mod;
                totscale_max = totscale;
                maxlag = k;
            }
        }
        pos -= 1;
        let e = i32::from(at(rp_end)) * i32::from(at(rp_end)) - i32::from(at(rp_beg)) * i32::from(at(rp_beg));
        energy = energy.wrapping_sub(e >> shifts);
        rp_beg -= 1;
        rp_end -= 1;
    }
    maxlag + offset
}

/// `WebRtcIlbcfix_CompCorr` on `buffer[..b_len]`.
fn comp_corr(buffer: &[i16], lag: usize, s_range: usize, scale: i32) -> (i32, i32) {
    let b_len = buffer.len();
    let Some(w) = b_len.checked_sub(s_range + lag) else { return (0, 1) };
    let a = &buffer[b_len - s_range..];
    let b = &buffer[w..w + s_range];
    let corr = dot(a, b, scale);
    let ener = dot(b, b, scale);
    if ener == 0 { (0, 1) } else { (corr, ener) }
}

/// `WebRtcSpl_DotProductWithScale`.
fn dot(a: &[i16], b: &[i16], scale: i32) -> i32 {
    let s = scale.clamp(0, 31);
    let sum: i64 = a.iter().zip(b).map(|(&x, &y)| i64::from((i32::from(x) * i32::from(y)) >> s)).sum();
    sum.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// `WebRtcSpl_MaxAbsValueW16`.
fn max_abs(v: &[i16]) -> i16 {
    v.iter().map(|&x| i32::from(x).abs()).max().unwrap_or(0).min(32767) as i16
}

/// `WebRtcSpl_GetSizeInBits`.
fn size_in_bits(n: u32) -> i32 {
    32 - n.leading_zeros() as i32
}

/// `WebRtcSpl_NormW32`.
fn norm_w32(a: i32) -> i32 {
    if a == 0 { 0 } else { (if a < 0 { !a } else { a }).leading_zeros() as i32 - 1 }
}

/// `WEBRTC_SPL_SHIFT_W32`: left by `c`, or right by `-c`.
fn shift_w32(x: i32, c: i32) -> i32 {
    if c >= 0 { x.wrapping_shl(c.min(31) as u32) } else { x >> (-c).min(31) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsf_mean_gives_a_stable_filter() {
        let a = lsf_to_poly(&LSF_MEAN);
        assert_eq!(a[0], 4096);
        assert!(a.iter().any(|&v| v != 0));
    }

    #[test]
    fn any_bytes_decode_without_panicking() {
        for mode in [Mode::Ms20, Mode::Ms30] {
            let mut d = Decoder::new(mode);
            let mut x = 0x1234_5678_9abc_def0u64;
            let mut out = Vec::new();
            for n in 0..3000 {
                let frame: Vec<u8> = (0..mode.frame_bytes())
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        x as u8
                    })
                    .collect();
                let f = match n % 11 {
                    3 => None,
                    7 => Some(&frame[..5]),
                    _ => Some(&frame[..]),
                };
                d.decode(f, &mut out);
            }
            for f in [vec![0u8; mode.frame_bytes()], vec![0xff; mode.frame_bytes()]] {
                for _ in 0..20 {
                    d.decode(Some(&f), &mut out);
                }
            }
            // a long loss: WebRTC conceals it with low-level noise, never silence
            for _ in 0..100 {
                d.decode(None, &mut out);
            }
            assert_eq!(out.len(), (3000 + 40 + 100) * mode.frame_samples());
        }
    }
}
