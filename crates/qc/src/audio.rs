//! Audio analysis (plan §4.3): loudness, loudness range and true peak (ITU-R BS.1770-4 /
//! EBU R128 through `cfa4-audio-dsp`), clipped samples, silence, and per-channel levels for the
//! layout check.
//!
//! The analysis runs on the decoded samples of every audio track while the integrity decode
//! reads them, so the file is read once. Encoder priming and padding are cut off by the track's
//! edit list first: what is measured is what plays.

use cfa4_audio_dsp::loudness::LoudnessMeter;

use crate::inspect::{Findings, Found};

/// A block whose loudest sample stays below this level is silent (dBFS).
pub const SILENCE_DBFS: f64 = -60.0;
/// Silence shorter than this is a pause, not a finding (seconds).
pub const SILENCE_MIN_S: f64 = 2.0;
/// A sample at or above this magnitude is clipped: full scale of 16-bit audio (32767 / 32768).
pub const CLIP_LEVEL: f32 = 32767.0 / 32768.0;
/// Consecutive clipped sample frames reported as one clipping event.
pub const CLIP_RUN: u64 = 3;
/// Integrated loudness needs one 400 ms gating block.
pub const MIN_LOUDNESS_S: f64 = 0.4;
/// Loudness range needs one 3 s short-term window.
pub const MIN_RANGE_S: f64 = 3.0;
/// Silence detection block length (seconds).
const BLOCK_S: f64 = 0.01;

/// What the analysis of one audio track found.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AudioFacts {
    pub track: usize,
    pub channels: usize,
    pub sample_rate: u32,
    /// Sample frames analysed (after the edit list).
    pub frames: u64,
    /// LUFS; −∞ when nothing passed the −70 LUFS gate.
    pub integrated_lufs: f64,
    pub loudness_range_lu: f64,
    /// dBTP; −∞ for digital silence.
    pub true_peak_dbtp: f64,
    pub sample_peak_dbfs: f64,
    pub channel_peak_dbfs: Vec<f64>,
    /// Samples (over all channels) at or above [`CLIP_LEVEL`].
    pub clipped: u64,
    /// Runs of at least [`CLIP_RUN`] clipped sample frames.
    pub clip_runs: Findings,
    /// Silent stretches of at least [`SILENCE_MIN_S`].
    pub silence: Findings,
    /// Every channel carries the same samples (dual mono).
    pub identical_channels: bool,
}

impl AudioFacts {
    pub fn seconds(&self) -> f64 {
        if self.sample_rate == 0 { 0.0 } else { self.frames as f64 / f64::from(self.sample_rate) }
    }
}

fn db(linear: f64) -> f64 {
    20.0 * linear.log10()
}

/// Streaming analysis of one track's decoded audio.
pub(crate) struct AudioAnalyzer {
    track: usize,
    rate: u32,
    /// Sample frames still to drop before the presented range (encoder priming).
    skip: u64,
    /// Sample frames still to analyse (`None`: to the end).
    left: Option<u64>,
    meter: Option<LoudnessMeter>,
    channels: usize,
    pos: u64,
    peaks: Vec<f32>,
    clipped: u64,
    clip_run: u64,
    clip_runs: Findings,
    block_len: u64,
    block_pos: u64,
    block_peak: f32,
    silent_from: Option<u64>,
    silence: Findings,
    identical: bool,
}

impl AudioAnalyzer {
    /// `skip` and `keep` are in sample frames: the edit list's presented range.
    pub(crate) fn new(track: usize, rate: u32, skip: u64, keep: Option<u64>) -> Self {
        AudioAnalyzer {
            track,
            rate,
            skip,
            left: keep,
            meter: None,
            channels: 0,
            pos: 0,
            peaks: Vec::new(),
            clipped: 0,
            clip_run: 0,
            clip_runs: Findings::default(),
            block_len: ((f64::from(rate) * BLOCK_S).round() as u64).max(1),
            block_pos: 0,
            block_peak: 0.0,
            silent_from: None,
            silence: Findings::default(),
            identical: true,
        }
    }

    fn ms(&self, frame: u64) -> Option<f64> {
        (self.rate > 0).then(|| frame as f64 * 1000.0 / f64::from(self.rate))
    }

    /// Feed one decoded packet (planar, one vector per channel).
    pub(crate) fn push(&mut self, planes: &[Vec<f32>]) {
        if planes.is_empty() || self.rate == 0 {
            return;
        }
        let n = planes.iter().map(Vec::len).min().unwrap_or(0) as u64;
        let start = self.skip.min(n);
        self.skip -= start;
        let mut end = n;
        if let Some(left) = self.left {
            end = end.min(start.saturating_add(left));
        }
        if end <= start {
            return;
        }
        if self.meter.is_none() {
            self.channels = planes.len();
            self.peaks = vec![0.0; planes.len()];
            self.meter = Some(LoudnessMeter::new(f64::from(self.rate), planes.len()));
        }
        // a decoder that changes its channel count mid-stream: measure the channels it started with
        let used = self.channels.min(planes.len());
        let (s, e) = (start as usize, end as usize);
        let silent_level = 10f64.powf(SILENCE_DBFS / 20.0) as f32;
        for i in s..e {
            let mut any_clip = false;
            let first = planes.first().and_then(|p| p.get(i)).copied().unwrap_or(0.0);
            for (ch, p) in planes.iter().take(used).enumerate() {
                let x = p.get(i).copied().unwrap_or(0.0);
                let a = if x.is_finite() { x.abs() } else { f32::MAX };
                if let Some(pk) = self.peaks.get_mut(ch) {
                    *pk = pk.max(a);
                }
                if a >= CLIP_LEVEL {
                    any_clip = true;
                    self.clipped = self.clipped.saturating_add(1);
                }
                if x.to_bits() != first.to_bits() {
                    self.identical = false;
                }
                self.block_peak = self.block_peak.max(a);
            }
            let frame = self.pos.saturating_add((i - s) as u64);
            if any_clip {
                self.clip_run = self.clip_run.saturating_add(1);
            } else {
                self.close_clip_run(frame);
            }
            self.block_pos += 1;
            if self.block_pos == self.block_len {
                let block_start = frame.saturating_add(1).saturating_sub(self.block_len);
                self.close_block(block_start, silent_level);
            }
        }
        if let Some(m) = self.meter.as_mut() {
            let slices: Vec<&[f32]> = planes.iter().take(used).filter_map(|p| p.get(s..e)).collect();
            if slices.len() == used {
                m.process(&slices);
            }
        }
        self.pos = self.pos.saturating_add(end - start);
        if let Some(left) = self.left.as_mut() {
            *left = left.saturating_sub(end - start);
        }
    }

    fn close_clip_run(&mut self, frame: u64) {
        if self.clip_run >= CLIP_RUN {
            let from = frame.saturating_sub(self.clip_run);
            let n = self.clip_run;
            self.clip_runs.push(Found { index: from, ms: self.ms(from), end_ms: self.ms(frame), message: format!("{n} clipped sample frames in a row") });
        }
        self.clip_run = 0;
    }

    fn close_block(&mut self, block_start: u64, silent_level: f32) {
        if self.block_peak < silent_level {
            self.silent_from.get_or_insert(block_start);
        } else {
            self.close_silence(block_start);
        }
        self.block_pos = 0;
        self.block_peak = 0.0;
    }

    fn close_silence(&mut self, end: u64) {
        if let Some(from) = self.silent_from.take() {
            let len_s = end.saturating_sub(from) as f64 / f64::from(self.rate.max(1));
            if len_s >= SILENCE_MIN_S {
                self.silence.push(Found {
                    index: from,
                    ms: self.ms(from),
                    end_ms: self.ms(end),
                    message: format!("{len_s:.1} s of silence (below {SILENCE_DBFS} dBFS)"),
                });
            }
        }
    }

    pub(crate) fn finish(mut self) -> AudioFacts {
        let end = self.pos;
        self.close_clip_run(end);
        if self.block_pos > 0 {
            let start = end.saturating_sub(self.block_pos);
            self.close_block(start, 10f64.powf(SILENCE_DBFS / 20.0) as f32);
        }
        self.close_silence(end);
        let mut f = AudioFacts {
            track: self.track,
            channels: self.channels,
            sample_rate: self.rate,
            frames: self.pos,
            integrated_lufs: f64::NEG_INFINITY,
            loudness_range_lu: 0.0,
            true_peak_dbtp: f64::NEG_INFINITY,
            sample_peak_dbfs: f64::NEG_INFINITY,
            channel_peak_dbfs: self.peaks.iter().map(|p| db(f64::from(*p))).collect(),
            clipped: self.clipped,
            clip_runs: self.clip_runs,
            silence: self.silence,
            identical_channels: self.identical && self.channels > 1,
        };
        if let Some(m) = &self.meter {
            let s = m.summary();
            f.integrated_lufs = s.integrated_lufs;
            f.loudness_range_lu = s.loudness_range_lu;
            f.true_peak_dbtp = s.true_peak_dbtp;
            f.sample_peak_dbfs = s.sample_peak_dbfs;
        }
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize, amp: f32, rate: u32) -> Vec<f32> {
        (0..n).map(|k| (k as f32 * 997.0 * std::f32::consts::TAU / rate as f32).sin() * amp).collect()
    }

    #[test]
    fn priming_is_cut_and_silence_found() {
        let rate = 48_000;
        let mut a = AudioAnalyzer::new(0, rate, 1000, Some(48_000 * 5));
        // 1000 priming frames of full-scale noise, then 1 s tone, 3 s silence, 1 s tone, then
        // more than the presented range
        let mut ch = vec![1.0f32; 1000];
        ch.extend(tone(48_000, 0.5, rate));
        ch.extend(vec![0.0; 48_000 * 3]);
        ch.extend(tone(48_000 * 2, 0.5, rate));
        for chunk in ch.chunks(1024) {
            a.push(&[chunk.to_vec(), chunk.to_vec()]);
        }
        let f = a.finish();
        assert_eq!(f.frames, 48_000 * 5);
        assert_eq!(f.clipped, 0, "priming is not measured");
        assert!(f.identical_channels);
        assert_eq!(f.silence.total, 1);
        let s = &f.silence.kept[0];
        assert!((s.ms.unwrap() - 1000.0).abs() <= 10.0 && (s.end_ms.unwrap() - 4000.0).abs() <= 10.0, "{s:?}");
        assert!((f.sample_peak_dbfs - db(0.5)).abs() < 0.01);
    }

    #[test]
    fn clipping_counts_samples_and_runs() {
        let mut a = AudioAnalyzer::new(0, 48_000, 0, None);
        let mut l = tone(4800, 0.25, 48_000);
        l[100..105].iter_mut().for_each(|x| *x = 1.0);
        l[200] = -1.0;
        let r = vec![0.0; 4800];
        a.push(&[l, r]);
        let f = a.finish();
        assert_eq!(f.clipped, 6);
        assert_eq!(f.clip_runs.total, 1, "a single clipped sample is counted, not a run");
        assert_eq!(f.clip_runs.kept[0].index, 100);
        assert!(!f.identical_channels);
        assert_eq!(f.channel_peak_dbfs[1], f64::NEG_INFINITY);
    }

    #[test]
    fn hostile_samples_do_not_crash() {
        let mut a = AudioAnalyzer::new(0, 48_000, 0, None);
        a.push(&[vec![f32::NAN, f32::INFINITY, 0.0], vec![0.0]]);
        a.push(&[]);
        let f = a.finish();
        assert_eq!(f.frames, 1);
        let mut z = AudioAnalyzer::new(0, 0, 0, None);
        z.push(&[vec![0.5; 10]]);
        assert_eq!(z.finish().frames, 0);
    }
}
