//! RealAudio 2.0 (28.8, `28_8`): a G.728-style backward-adaptive CELP coder, 38-byte frames of
//! 160 samples at 8 kHz. A port of FFmpeg n7.1.1 `libavcodec/ra288.c` (with the helpers it uses
//! from `celp_filters.c`, `lpc_functions.h` and `float_dsp.c`; LGPL-2.1-or-later), keeping its
//! `f32`/`f64` operation order so the samples match FFmpeg's bit for bit.

use crate::celp::{dot, lp_synthesis};
use crate::tables288::{AMPTABLE, CODETABLE, GAIN_BW_TAB, GAIN_WINDOW, SYN_BW_TAB, SYN_WINDOW};
use crate::{Error, Result};

/// Bytes per frame (the stream's `block_align`).
pub const FRAME_SIZE: usize = 38;
/// Samples per frame.
pub const FRAME_SAMPLES: usize = BLOCK_SIZE * BLOCKS_PER_FRAME;

const BLOCK_SIZE: usize = 5;
const BLOCKS_PER_FRAME: usize = 32;

/// Decoder state (`RA288Context`).
#[derive(Clone)]
pub struct Decoder {
    sp_lpc: [f32; 36],
    gain_lpc: [f32; 10],
    sp_hist: [f32; 111],
    sp_rec: [f32; 37],
    gain_hist: [f32; 38],
    gain_rec: [f32; 11],
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

/// `compute_lpc_coefs` (float, normalised, failing on a degenerate autocorrelation); `autoc`
/// holds `order + 1` values and `lpc` `order`.
fn compute_lpc_coefs(autoc: &[f32], order: usize, lpc: &mut [f32]) -> bool {
    let Some((&first, autoc)) = autoc.split_first() else { return false };
    let mut err = first;
    if order == 0 || autoc.len() < order || lpc.len() < order || autoc[order - 1] == 0.0 || err <= 0.0 {
        return false;
    }
    for i in 0..order {
        let mut r = -autoc[i];
        for j in 0..i {
            r -= lpc[j] * autoc[i - j - 1];
        }
        if err != 0.0 {
            r /= err;
        }
        err *= 1.0 - r * r;
        lpc[i] = r;
        for j in 0..(i + 1) >> 1 {
            let f = lpc[j];
            let b = lpc[i - 1 - j];
            lpc[j] = f + r * b;
            lpc[i - 1 - j] = b + r * f;
        }
        if err < 0.0 {
            return false;
        }
    }
    true
}

/// `do_hybrid_window` + `backward_filter`: new LPC coefficients from the history, then the
/// history moves down by `n`.
#[allow(clippy::too_many_arguments)]
fn backward_filter(hist: &mut [f32], rec: &mut [f32], window: &[f32], lpc: &mut [f32], tab: &[f32], order: usize, n: usize, non_rec: usize, move_size: usize) {
    let total = order + n + non_rec;
    let work: Vec<f32> = window.iter().zip(hist.iter()).take(total).map(|(w, h)| w * h).collect();
    let convolve = |src: usize, len: usize| -> Vec<f32> { (0..=order).map(|k| dot(&work[src..src + len], &work[src - k..src - k + len])).collect() };
    let b1 = convolve(order, n);
    let b2 = convolve(order + n, non_rec);
    let mut temp = vec![0f32; order + 1];
    for i in 0..=order {
        rec[i] = (f64::from(rec[i]) * 0.5625 + f64::from(b1[i])) as f32;
        temp[i] = rec[i] + b2[i];
    }
    temp[0] = (f64::from(temp[0]) * (257.0 / 256.0)) as f32;
    // In place, as FFmpeg does: a recursion that fails part way leaves what it wrote.
    if compute_lpc_coefs(&temp, order, &mut lpc[..order]) {
        for (l, t) in lpc.iter_mut().zip(tab) {
            *l *= t;
        }
    }
    hist.copy_within(n..n + move_size, 0);
}

impl Decoder {
    pub fn new() -> Self {
        Self { sp_lpc: [0.0; 36], gain_lpc: [0.0; 10], sp_hist: [0.0; 111], sp_rec: [0.0; 37], gain_hist: [0.0; 38], gain_rec: [0.0; 11] }
    }

    /// `decode`: one 5-sample block.
    fn block(&mut self, gain: f32, cb: usize) {
        self.sp_hist.copy_within(75..111, 70);
        let gb = 28;
        let mut sum = 32.0f32;
        for i in 0..10 {
            sum -= self.gain_hist[gb + 9 - i] * self.gain_lpc[i];
        }
        let sum = sum.clamp(0.0, 60.0);
        let sumsum = (f64::from(sum) * 0.1151292546497).exp() * f64::from(gain) * (1.0 / f64::from(1u32 << 23));
        let code = CODETABLE.get(cb).copied().unwrap_or_default();
        let mut buffer = [0f32; 5];
        for (b, &c) in buffer.iter_mut().zip(&code) {
            *b = (f64::from(c) * sumsum) as f32;
        }
        let e = dot(&buffer, &buffer);
        let floor = 5.0 / f64::from(1u32 << 24);
        let e = if f64::from(e) > floor { e } else { floor as f32 };
        self.gain_hist.copy_within(gb + 1..gb + 10, gb);
        self.gain_hist[gb + 9] = (10.0 * f64::from(e).log10() + (10.0 * (f64::from(1u32 << 24) / 5.0).log10() - 32.0)) as f32;
        lp_synthesis(&mut self.sp_hist, 106, &self.sp_lpc, &buffer);
    }

    /// Decodes one 38-byte frame into 160 samples in `[-1, 1]`-ish `f32` (FFmpeg's float output).
    pub fn decode(&mut self, frame: &[u8]) -> Result<[f32; FRAME_SAMPLES]> {
        let frame = frame.get(..FRAME_SIZE).ok_or(Error::Invalid("RealAudio 2.0 frame too short"))?;
        let mut pos = 0usize;
        let mut get = |n: usize| -> usize {
            let mut v = 0usize;
            for j in 0..n {
                let bit = frame.get(pos >> 3).map_or(0, |b| (b >> (pos & 7)) & 1);
                v |= usize::from(bit) << j;
                pos += 1;
            }
            v
        };
        let mut out = [0f32; FRAME_SAMPLES];
        for i in 0..BLOCKS_PER_FRAME {
            let gain = AMPTABLE[get(3) & 7];
            let cb = get(6 + (i & 1));
            self.block(gain, cb);
            out[i * BLOCK_SIZE..(i + 1) * BLOCK_SIZE].copy_from_slice(&self.sp_hist[106..111]);
            if i & 7 == 3 {
                backward_filter(&mut self.sp_hist, &mut self.sp_rec, &SYN_WINDOW, &mut self.sp_lpc, &SYN_BW_TAB, 36, 40, 35, 70);
                backward_filter(&mut self.gain_hist, &mut self.gain_rec, &GAIN_WINDOW, &mut self.gain_lpc, &GAIN_BW_TAB, 10, 8, 20, 28);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_frames_are_errors_and_noise_does_not_panic() {
        let mut d = Decoder::new();
        assert!(d.decode(&[0; 37]).is_err());
        let mut x = 0x9E37_79B9u32;
        for _ in 0..5000 {
            let mut f = [0u8; FRAME_SIZE];
            for b in &mut f {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                *b = x as u8;
            }
            let s = d.decode(&f).unwrap();
            assert_eq!(s.len(), FRAME_SAMPLES);
        }
    }
}
