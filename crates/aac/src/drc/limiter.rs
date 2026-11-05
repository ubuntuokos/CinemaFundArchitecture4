//! Look-ahead peak limiter of the MPEG-D DRC decoder.
//!
//! Ported from libxaac `impd_drc_peak_limiter.c` (Apache-2.0, see `super` for the notice).

/// Attack (= look-ahead delay) and release times.
const ATTACK_MS: f32 = 5.0;
const RELEASE_MS: f32 = 50.0;
/// -1 dBFS.
pub const THRESHOLD: f32 = 0.891_250_9;

#[derive(Debug, Clone)]
pub struct PeakLimiter {
    attack: usize,
    attack_const: f32,
    release_const: f32,
    threshold: f32,
    channels: usize,
    /// Sliding window of per-sample channel maxima (`attack + 1` entries, newest first).
    max_buf: Vec<f32>,
    /// Delay line, `attack` frames of interleaved samples.
    delayed: Vec<f32>,
    index: usize,
    gain_modified: f32,
    smoothed: f64,
}

impl PeakLimiter {
    pub fn new(channels: usize, sample_rate: u32) -> PeakLimiter {
        let attack = ((ATTACK_MS * sample_rate as f32 / 1000.0) as usize).max(1);
        let release = RELEASE_MS * sample_rate as f32 / 1000.0;
        PeakLimiter {
            attack,
            attack_const: 0.1f64.powf(1.0 / (attack as f64 + 1.0)) as f32,
            release_const: 0.1f64.powf(1.0 / (release as f64 + 1.0)) as f32,
            threshold: THRESHOLD,
            channels,
            max_buf: vec![0.0; attack + 1],
            delayed: vec![0.0; attack * channels.max(1)],
            index: 0,
            gain_modified: 1.0,
            smoothed: 1.0,
        }
    }

    /// Delay in samples.
    pub fn delay(&self) -> usize {
        self.attack
    }

    /// Limit planar `pcm` in place (all channels the same length).
    pub fn process(&mut self, pcm: &mut [Vec<f32>]) {
        let len = pcm.iter().map(Vec::len).min().unwrap_or(0);
        let ch = self.channels.min(pcm.len());
        for i in 0..len {
            let mut peak = 0.0f32;
            for c in pcm.iter().take(ch) {
                peak = peak.max(c.get(i).copied().unwrap_or(0.0).abs());
            }
            self.max_buf.rotate_right(1);
            if let Some(m) = self.max_buf.first_mut() {
                *m = peak;
            }
            let maximum = self.max_buf.iter().copied().fold(0.0f32, f32::max);
            let gain = if maximum > self.threshold { self.threshold / maximum } else { 1.0 };
            if gain < self.smoothed as f32 {
                self.gain_modified = self.gain_modified.min((gain - 0.1 * self.smoothed as f32) * (10.0 / 9.0));
            } else {
                self.gain_modified = gain;
            }
            let gm = self.gain_modified as f64;
            if gm < self.smoothed {
                self.smoothed = (self.attack_const as f64 * (self.smoothed - gm) + gm).max(gain as f64);
            } else {
                self.smoothed = self.release_const as f64 * (self.smoothed - gm) + gm;
            }
            let g = self.smoothed as f32;
            for (j, c) in pcm.iter_mut().take(ch).enumerate() {
                let k = self.index * self.channels + j;
                if let (Some(d), Some(s)) = (self.delayed.get_mut(k), c.get_mut(i)) {
                    let out = (*d * g).clamp(-self.threshold, self.threshold);
                    *d = *s;
                    *s = out;
                }
            }
            self.index += 1;
            if self.index >= self.attack {
                self.index = 0;
            }
        }
    }
}
