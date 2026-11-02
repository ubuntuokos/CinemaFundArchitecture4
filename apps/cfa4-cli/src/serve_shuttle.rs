//! The sound of shuttle playback for `audio.render {speed}` (J / L at 0.5–4×), as egui's
//! `shuttle.rs`: the timeline is mixed at `device rate / speed` and played at the device rate, so
//! it runs `speed` times as fast. With Preferences ▸ Audio ▸ Maintain pitch while shuttling (the
//! default) the pitch is corrected first (at twice the device rate when slowed down, for the
//! headroom of the upward correction), then the rate is converted.
//!
//! The client asks for consecutive blocks: `start` is a frame of that `device rate / speed`
//! stream (the `mixRate` the reply names), so block after block continues one stream. The pitch
//! shifters, the rate converters and where the stream is up to are kept per target between
//! requests (one server serves one GUI from one thread), and start over on a jump.

use cfa4_audio_dsp::{AudioEffect, effects::PitchShifter, resample::StreamResampler};

const BLOCK: usize = 1024;
/// Two shifters have at most 16384 frames of latency; room for priming and interpolation (egui's).
const FRAME_MARGIN: i128 = 65_536;
/// The shuttle plays stereo, as `audio.render` does.
const CHANNELS: usize = 2;

/// The rate the timeline is mixed at so the device plays it `speed` times as fast; None outside
/// egui's 0.5–4× (faster, slower and backwards shuttling are silent).
pub fn mix_rate(device_rate: u32, speed: f64) -> Option<u32> {
    if device_rate == 0 || !speed.is_finite() || !(0.5..=4.0).contains(&speed) {
        return None;
    }
    let rate = (f64::from(device_rate) / speed).round();
    if !(1.0..=f64::from(u32::MAX)).contains(&rate) {
        return None;
    }
    Some(rate as u32)
}

/// Planar stereo audio: `raw(start, frames, rate)` at `rate`.
pub type Raw<'a> = &'a mut dyn FnMut(i64, usize, u32) -> Vec<Vec<f32>>;

/// One target's shuttle stream.
pub struct Shuttle {
    key: (&'static str, u32, u32, bool),
    /// The rate `raw` gives audio at, when it has one (the sequence's), converted to the rate the
    /// stream needs; None: `raw` gives any rate asked for (a source decodes at any rate).
    native: Option<(u32, u32, StreamResampler)>,
    corrected: Option<Corrected>,
}

struct Corrected {
    source: PitchedStream,
    resampler: StreamResampler,
}

impl Shuttle {
    /// The stream for `target` at `device_rate` and mix `rate`, pitch-corrected or not.
    pub fn new(target: &'static str, device_rate: u32, rate: u32, maintain_pitch: bool) -> Option<Self> {
        let pitch = maintain_pitch && rate != device_rate;
        let corrected = if pitch {
            // slowed down: headroom for the upward pitch correction before the rate conversion
            let internal = if rate > device_rate { device_rate.checked_mul(2)? } else { device_rate };
            let semitones = (12.0 * (f64::from(rate) / f64::from(device_rate)).log2()) as f32;
            let mut source = PitchedStream { rate: internal, semitones, stages: Vec::new(), latency: 0, next: None };
            source.configure();
            Some(Corrected { source, resampler: StreamResampler::new(internal, rate) })
        } else {
            None
        };
        Some(Shuttle { key: (target, device_rate, rate, maintain_pitch), native: None, corrected })
    }

    pub fn is_for(&self, target: &str) -> bool {
        self.key.0 == target
    }

    pub fn matches(&self, target: &str, device_rate: u32, rate: u32, maintain_pitch: bool) -> bool {
        self.key.0 == target && self.key.1 == device_rate && self.key.2 == rate && self.key.3 == maintain_pitch
    }

    /// `frames` frames of the stream from frame `start` (at the mix rate), planar stereo. `raw`
    /// mixes the timeline at its `native` rate (None: at any rate).
    pub fn render(&mut self, start: i64, frames: usize, native: Option<u32>, raw: Raw<'_>) -> Vec<Vec<f32>> {
        // the rate converters work in i64 source frames: far-away blocks are silent (and start over)
        let lo = i128::from(i64::MIN / 8) + FRAME_MARGIN;
        let hi = i128::from(i64::MAX / 8) - FRAME_MARGIN;
        if !(lo..=hi).contains(&i128::from(start)) || !(lo..=hi).contains(&(i128::from(start) + frames as i128)) {
            if let Some(c) = &mut self.corrected {
                c.source.next = None;
                c.resampler = StreamResampler::new(c.source.rate, self.key.2);
            }
            self.native = None;
            return silence(frames);
        }
        let Shuttle { key, native: conv, corrected } = self;
        let in_rate = corrected.as_ref().map_or(key.2, |c| c.source.rate);
        let mut base = |s: i64, n: usize| -> Vec<Vec<f32>> {
            match native {
                Some(nr) if nr != in_rate => {
                    if conv.as_ref().is_none_or(|(f, t, _)| (*f, *t) != (nr, in_rate)) {
                        *conv = Some((nr, in_rate, StreamResampler::new(nr, in_rate)));
                    }
                    let Some((_, _, r)) = conv.as_mut() else { return silence(n) };
                    r.process(s, n, |s0, m| stereo(raw(s0, m, nr), m))
                }
                _ => stereo(raw(s, n, in_rate), n),
            }
        };
        let out = match corrected {
            Some(c) => {
                let Corrected { source, resampler } = c;
                resampler.process(start, frames, |s, n| source.pull(s, n, &mut base))
            }
            None => base(start, frames),
        };
        out.into_iter()
            .map(|mut c| {
                c.resize(frames, 0.0);
                c.iter_mut().for_each(|x| *x = if x.is_finite() { *x } else { 0.0 });
                c
            })
            .collect()
    }
}

fn silence(n: usize) -> Vec<Vec<f32>> {
    vec![vec![0.0; n]; CHANNELS]
}

/// `planes` as `n` frames of stereo (a missing channel repeats the first, or is silent).
fn stereo(mut planes: Vec<Vec<f32>>, n: usize) -> Vec<Vec<f32>> {
    planes.truncate(CHANNELS);
    while planes.len() < CHANNELS {
        let copy = planes.first().cloned().unwrap_or_else(|| vec![0.0; n]);
        planes.push(copy);
    }
    for p in &mut planes {
        p.resize(n, 0.0);
    }
    planes
}

struct PitchedStream {
    rate: u32,
    semitones: f32,
    stages: Vec<PitchShifter>,
    latency: usize,
    /// The next frame this stream is expected to give; the mix is `latency` frames ahead of it.
    next: Option<i64>,
}

impl PitchedStream {
    fn configure(&mut self) {
        self.stages.clear();
        let mut remaining = self.semitones;
        // one shifter covers ±12 semitones; two cover 4× (−24)
        for _ in 0..2 {
            let shift = remaining.clamp(-12.0, 12.0);
            let mut stage = PitchShifter::new(self.rate as f32, CHANNELS);
            stage.set_param("semitones", shift);
            stage.reset();
            self.stages.push(stage);
            remaining -= shift;
            if remaining.abs() < 1e-4 {
                break;
            }
        }
        self.latency = self.stages.iter().map(AudioEffect::latency).sum();
        self.next = None;
    }

    fn process(&mut self, start: i64, n: usize, base: &mut dyn FnMut(i64, usize) -> Vec<Vec<f32>>) -> Vec<Vec<f32>> {
        let mut planar = base(start, n);
        let mut lanes: Vec<_> = planar.iter_mut().map(Vec::as_mut_slice).collect();
        for stage in &mut self.stages {
            stage.process(&mut lanes);
        }
        planar
    }

    fn pull(&mut self, start: i64, n: usize, base: &mut dyn FnMut(i64, usize) -> Vec<Vec<f32>>) -> Vec<Vec<f32>> {
        if self.next != Some(start) {
            for stage in &mut self.stages {
                stage.reset();
            }
            // prime from earlier timeline audio and drop the delay, so the first sound is the
            // sound at `start` (not behind the accelerated picture)
            let pre = self.latency * 2;
            let Some(mut input) = start.checked_sub(pre as i64) else { return silence(n) };
            let mut left = pre + self.latency;
            while left > 0 {
                let count = left.min(BLOCK);
                self.process(input, count, base);
                input += count as i64;
                left -= count;
            }
        }
        let mut result: Vec<Vec<f32>> = (0..CHANNELS).map(|_| Vec::with_capacity(n)).collect();
        let mut done = 0;
        while done < n {
            let count = (n - done).min(BLOCK);
            let block = self.process(start + self.latency as i64 + done as i64, count, base);
            for (out, plane) in result.iter_mut().zip(block) {
                out.extend(plane);
            }
            done += count;
        }
        self.next = Some(start + n as i64);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tone of `hz` (both channels) at whatever rate is asked for, as a source decodes.
    fn tone(hz: f64) -> impl FnMut(i64, usize, u32) -> Vec<Vec<f32>> {
        move |start, n, rate| {
            let c: Vec<f32> = (0..n).map(|i| (0.3 * (std::f64::consts::TAU * hz * (start + i as i64) as f64 / f64::from(rate)).sin()) as f32).collect();
            vec![c.clone(), c]
        }
    }

    fn amplitude(samples: &[f32], hz: f64, sr: u32) -> f64 {
        let (mut re, mut im) = (0.0, 0.0);
        for (i, x) in samples.iter().enumerate() {
            let phase = std::f64::consts::TAU * hz * i as f64 / f64::from(sr);
            re += f64::from(*x) * phase.cos();
            im += f64::from(*x) * phase.sin();
        }
        2.0 * (re * re + im * im).sqrt() / samples.len() as f64
    }

    /// `seconds` of the shuttle stream at `speed`, as consecutive client blocks.
    fn play(speed: f64, pitch: bool, native: Option<u32>, raw: &mut dyn FnMut(i64, usize, u32) -> Vec<Vec<f32>>) -> (Vec<f32>, u32) {
        let sr = 48_000;
        let rate = mix_rate(sr, speed).unwrap();
        let mut sh = Shuttle::new("program", sr, rate, pitch).unwrap();
        let mut left = Vec::new();
        let mut at = 48_000;
        for _ in 0..4 {
            let block = sh.render(at, 4_800, native, raw);
            assert_eq!(block.len(), 2);
            assert!(block.iter().all(|c| c.len() == 4_800));
            left.extend_from_slice(&block[0]);
            at += 4_800;
        }
        (left, sr)
    }

    #[test]
    fn mix_rates_as_egui() {
        assert_eq!(mix_rate(48_000, 2.0), Some(24_000));
        assert_eq!(mix_rate(48_000, 0.5), Some(96_000));
        assert_eq!(mix_rate(48_000, 4.0), Some(12_000));
        for speed in [8.0, 0.25, -1.0, f64::NAN] {
            assert_eq!(mix_rate(48_000, speed), None, "{speed}× is silent");
        }
    }

    #[test]
    fn shuttle_runs_faster_and_keeps_its_pitch_when_asked() {
        let hz = 440.0;
        for speed in [2.0, 4.0, 0.5] {
            // without the correction the pitch moves with the speed
            let (plain, sr) = play(speed, false, None, &mut tone(hz));
            assert!(amplitude(&plain, hz * speed, sr) > 0.2, "{speed}×: {} Hz", hz * speed);
            assert!(amplitude(&plain, hz, sr) < 0.05, "{speed}× uncorrected");
            // with it the tone stays at its pitch
            let (kept, sr) = play(speed, true, None, &mut tone(hz));
            let level = amplitude(&kept[4_800..], hz, sr);
            assert!(level > 0.1, "{speed}×: {hz} Hz level {level}");
            if hz * speed < f64::from(sr) / 2.0 {
                assert!(amplitude(&kept[4_800..], hz * speed, sr) < level * 0.15, "{speed}×: pitch not corrected");
            }
        }
    }

    #[test]
    fn a_sequence_mixed_at_its_own_rate_is_converted() {
        // the program mixes at the sequence's rate (48 kHz here); 2× wants 24 kHz
        let hz = 300.0;
        let (out, sr) = play(2.0, false, Some(48_000), &mut tone(hz));
        assert!(amplitude(&out, hz * 2.0, sr) > 0.2);
        let (out, sr) = play(2.0, true, Some(48_000), &mut tone(hz));
        assert!(amplitude(&out[4_800..], hz, sr) > 0.1);
    }

    #[test]
    fn far_away_and_bad_samples_stay_silent() {
        let mut sh = Shuttle::new("source", 48_000, 24_000, true).unwrap();
        let mut nan = |_s: i64, n: usize, _r: u32| vec![vec![f32::NAN; n]; 2];
        let out = sh.render(0, 256, None, &mut nan);
        assert!(out.iter().flatten().all(|x| *x == 0.0));
        let out = sh.render(i64::MIN / 2, 16, None, &mut tone(100.0));
        assert!(out.iter().all(|c| c.len() == 16));
    }
}
