// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Sirac Ozmen
//
// Ported to Rust for CFA4 from Maul Audio (https://github.com/Rawframe-Project/maul-audio, commit
// aeb383458250759f92a3e724a5bd006505e45b4d, src/hrtf_resample.{h,c}).

//! Impulse responses moved to another sample rate, as an HRTF set loaded for a renderer at another
//! rate needs: band-limited interpolation (Smith, "Digital Audio Resampling Home Page") with a
//! Blackman-windowed sinc at the lower of the two Nyquist frequencies, 32 input samples wide on
//! each side (wider when going down), scaled by `from / to` so the filter's gain stays the same.
//! The output starts 24 input samples early: a minimum-phase response begins at full height, and
//! its band-limited form rings before it. The lead is the same for every response, so interaural
//! delays stay as they were. Computed in f64.

use std::f64::consts::PI;

const HALF_WIDTH: f64 = 32.0;
const LEAD: f64 = 24.0;

fn lead(from: u32, to: u32) -> u32 {
    (LEAD * f64::from(to) / f64::from(from)).ceil() as u32
}

fn cutoff(from: u32, to: u32) -> f64 {
    if to < from { f64::from(to) / f64::from(from) } else { 1.0 }
}

/// The taps a response of `taps` at `from` takes at `to`, its lead included; `taps` itself at the
/// same rate.
pub(crate) fn resampled_taps(taps: u32, from: u32, to: u32) -> u32 {
    if from == to {
        return taps;
    }
    ((u64::from(taps) * u64::from(to)).div_ceil(u64::from(from))) as u32 + lead(from, to)
}

fn kernel(x: f64, cutoff: f64) -> f64 {
    let half = HALF_WIDTH / cutoff;
    if x.abs() >= half {
        return 0.0;
    }
    let window = 0.42 + 0.5 * (PI * x / half).cos() + 0.08 * (2.0 * PI * x / half).cos();
    let phase = PI * cutoff * x;
    window * cutoff * if phase == 0.0 { 1.0 } else { phase.sin() / phase }
}

/// The input taps output tap `n` reads (start..=end, none when end < start) and its time in input
/// samples.
fn reach(n: u32, in_taps: u32, from: u32, to: u32) -> (f64, f64, f64) {
    let step = f64::from(from) / f64::from(to);
    let span = HALF_WIDTH / cutoff(from, to);
    let t = (f64::from(n) - f64::from(lead(from, to))) * step;
    let first = (t - span).ceil().max(0.0);
    let last = (t + span).floor().min(f64::from(in_taps) - 1.0);
    (t, first, last)
}

/// The precomputed kernel weights of resampling responses of `in_taps` at `from` into `out_taps`
/// at `to`: the same for every response of a set.
pub(crate) struct Resampler {
    from: u32,
    to: u32,
    in_taps: u32,
    out_taps: u32,
    width: usize,
    weights: Vec<f64>,
}

impl Resampler {
    pub fn new(in_taps: u32, from: u32, to: u32) -> Self {
        let out_taps = resampled_taps(in_taps, from, to);
        let c = cutoff(from, to);
        let width = (2.0 * HALF_WIDTH / c) as usize + 2;
        let mut weights = vec![0.0; out_taps as usize * width];
        for n in 0..out_taps {
            let (t, start, end) = reach(n, in_taps, from, to);
            let row = &mut weights[n as usize * width..(n as usize + 1) * width];
            if end >= start {
                let count = ((end - start) as usize + 1).min(width);
                for (j, w) in row.iter_mut().take(count).enumerate() {
                    *w = kernel(t - (start + j as f64), c);
                }
            }
        }
        Resampler { from, to, in_taps, out_taps, width, weights }
    }

    pub fn out_taps(&self) -> u32 {
        self.out_taps
    }

    /// Resamples `input` (`in_taps` long) into `out` (`out_taps` long).
    pub fn run(&self, input: &[f32], out: &mut [f32]) {
        let step = f64::from(self.from) / f64::from(self.to);
        for (n, o) in out.iter_mut().take(self.out_taps as usize).enumerate() {
            let (_, start, end) = reach(n as u32, self.in_taps, self.from, self.to);
            let row = &self.weights[n * self.width..(n + 1) * self.width];
            let mut sum = 0.0f64;
            if end >= start {
                let count = ((end - start) as usize + 1).min(self.width);
                let first = start as usize;
                for (j, w) in row.iter().take(count).enumerate() {
                    sum += f64::from(input.get(first + j).copied().unwrap_or(0.0)) * w;
                }
            }
            *o = (sum * step) as f32;
        }
    }
}
