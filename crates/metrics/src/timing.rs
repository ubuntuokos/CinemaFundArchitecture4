//! Timing metrics on presentation-time series: frame PTS error, dropped and duplicated frames,
//! audio/video drift and seek accuracy.
//!
//! Times are integers (CFA4 ticks, or any other fixed unit the caller states): no floating-point
//! seconds enter the comparison, so a one-frame shift is exactly one frame duration.

use crate::MetricError;

/// Result of [`pts_diff`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PtsDiff {
    /// Frames in the expected series.
    pub expected: usize,
    /// Frames in the actual series.
    pub actual: usize,
    /// Expected presentation times with no actual frame within half a frame duration.
    pub dropped: usize,
    /// Actual frames beyond the expected series: later frames matching an already used time, or
    /// times matching no expected frame.
    pub extra: usize,
    /// Actual times that do not increase from the previous one.
    pub non_monotonic: usize,
    /// Median of `actual − expected` over the matched frames, in the series' unit.
    pub median_offset: i64,
    /// Largest `|actual − expected|` over the matched frames.
    pub max_abs_error: i64,
    /// Median of `actual[i] − expected[i]` by position over the common prefix of both series.
    /// A whole clip shifted by one frame matches the neighbouring expected frames exactly, so
    /// `median_offset` stays 0 and only this positional figure (and `dropped` / `extra`) show it.
    pub index_median_offset: i64,
    /// Largest `|actual[i] − expected[i]|` by position over the common prefix.
    pub index_max_abs_error: i64,
}

/// Compare an `actual` PTS series with the `expected` one. A frame matches the nearest expected
/// time within half of `frame_duration`; the series are sorted as delivered (not re-sorted), so
/// out-of-order delivery shows up as `non_monotonic`.
pub fn pts_diff(expected: &[i64], actual: &[i64], frame_duration: i64) -> Result<PtsDiff, MetricError> {
    if frame_duration <= 0 {
        return Err(MetricError::DataRange);
    }
    if expected.is_empty() {
        return Err(MetricError::TooSmall { width: 0, height: 0 });
    }
    let half = frame_duration / 2;
    let mut used = vec![false; expected.len()];
    let (mut extra, mut non_monotonic) = (0usize, 0usize);
    let mut errors: Vec<i64> = Vec::with_capacity(actual.len());
    let mut previous: Option<i64> = None;
    for &t in actual {
        if previous.is_some_and(|p| t <= p) {
            non_monotonic += 1;
        }
        previous = Some(t);
        // nearest expected time (the series is ordered, so a binary search finds it)
        let at = expected.partition_point(|&e| e < t);
        let candidates = [at.checked_sub(1), (at < expected.len()).then_some(at)];
        let best = candidates.into_iter().flatten().min_by_key(|&i| (t - expected[i]).abs());
        match best {
            Some(i) if (t - expected[i]).abs() <= half && !used[i] => {
                used[i] = true;
                errors.push(t - expected[i]);
            }
            _ => extra += 1,
        }
    }
    let dropped = used.iter().filter(|u| !**u).count();
    let max_abs_error = errors.iter().map(|e| e.abs()).max().unwrap_or(0);
    errors.sort_unstable();
    let median_offset = errors.get(errors.len() / 2).copied().unwrap_or(0);
    let mut by_index: Vec<i64> = expected.iter().zip(actual).map(|(e, a)| a - e).collect();
    let index_max_abs_error = by_index.iter().map(|e| e.abs()).max().unwrap_or(0);
    by_index.sort_unstable();
    let index_median_offset = by_index.get(by_index.len() / 2).copied().unwrap_or(0);
    Ok(PtsDiff {
        expected: expected.len(),
        actual: actual.len(),
        dropped,
        extra,
        non_monotonic,
        median_offset,
        max_abs_error,
        index_median_offset,
        index_max_abs_error,
    })
}

/// Audio/video drift over a clip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drift {
    /// Offset of the first event, `video − audio`, in milliseconds.
    pub initial_offset_ms: f64,
    /// Largest change of that offset over the clip, in milliseconds (the drift proper).
    pub max_drift_ms: f64,
}

/// Drift between events seen in both streams: `video_ticks[i]` is the picture time of the i-th
/// event (a flash, in ticks of `ticks_per_second`), `audio_samples[i]` the sample position of the
/// same event (a beep) at `sample_rate`.
pub fn av_drift(video_ticks: &[i64], audio_samples: &[i64], ticks_per_second: i64, sample_rate: u32) -> Result<Drift, MetricError> {
    if video_ticks.len() != audio_samples.len() {
        return Err(MetricError::SizeMismatch { reference: video_ticks.len(), distorted: audio_samples.len() });
    }
    if video_ticks.is_empty() {
        return Err(MetricError::TooSmall { width: 0, height: 0 });
    }
    if ticks_per_second <= 0 || sample_rate == 0 {
        return Err(MetricError::DataRange);
    }
    let offsets: Vec<f64> =
        video_ticks.iter().zip(audio_samples).map(|(&v, &a)| 1000.0 * (v as f64 / ticks_per_second as f64 - a as f64 / f64::from(sample_rate))).collect();
    let first = offsets[0];
    let max_drift_ms = offsets.iter().map(|o| (o - first).abs()).fold(0.0, f64::max);
    Ok(Drift { initial_offset_ms: first, max_drift_ms })
}

/// Indices where `values` rises to `high` or above after having been at `low` or below (a
/// hysteresis edge detector). The start counts as low, so a series that begins high gives an event
/// at index 0. Used to find the flash frames of a flash-and-beep fixture from the mean picture level.
pub fn rising_edges(values: &[f64], low: f64, high: f64) -> Vec<usize> {
    let mut out = Vec::new();
    let mut is_high = false;
    for (i, &v) in values.iter().enumerate() {
        if is_high {
            if v <= low {
                is_high = false;
            }
        } else if v >= high {
            is_high = true;
            out.push(i);
        }
    }
    out
}

/// Sample positions where a beep starts: the first sample at or above `level` (linear, full scale
/// 1.0, any channel) that follows at least `quiet_ms` without one. Positions are sample indices.
pub fn beep_onsets(channels: &[Vec<f32>], sample_rate: u32, level: f32, quiet_ms: u32) -> Result<Vec<i64>, MetricError> {
    if sample_rate == 0 || !level.is_finite() || level <= 0.0 {
        return Err(MetricError::DataRange);
    }
    let len = channels.iter().map(Vec::len).max().unwrap_or(0);
    if len == 0 {
        return Err(MetricError::TooSmall { width: 0, height: 0 });
    }
    let quiet = (u64::from(sample_rate) * u64::from(quiet_ms) / 1000).max(1);
    let mut out = Vec::new();
    let mut last_loud: Option<u64> = None;
    for i in 0..len {
        let loud = channels.iter().any(|ch| ch.get(i).is_some_and(|s| s.abs() >= level));
        if loud {
            let i = i as u64;
            if last_loud.is_none_or(|l| i - l > quiet) {
                out.push(i as i64);
            }
            last_loud = Some(i);
        }
    }
    Ok(out)
}

/// Largest seek error in frames: `|delivered − requested| / frame_duration` over all seeks.
pub fn seek_error_frames(requested: &[i64], delivered: &[i64], frame_duration: i64) -> Result<f64, MetricError> {
    if requested.len() != delivered.len() {
        return Err(MetricError::SizeMismatch { reference: requested.len(), distorted: delivered.len() });
    }
    if requested.is_empty() {
        return Err(MetricError::TooSmall { width: 0, height: 0 });
    }
    if frame_duration <= 0 {
        return Err(MetricError::DataRange);
    }
    let worst = requested.iter().zip(delivered).map(|(r, d)| (d - r).abs()).max().unwrap_or(0);
    Ok(worst as f64 / frame_duration as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rising_edges_need_the_signal_to_fall_back_first() {
        let v = [0.0, 0.9, 0.9, 0.4, 0.9, 0.1, 0.0, 0.8];
        // 0.4 is between the thresholds: still high, so no new event at index 4
        assert_eq!(rising_edges(&v, 0.25, 0.5), vec![1, 7]);
        assert_eq!(rising_edges(&[0.9, 0.0, 0.9], 0.25, 0.5), vec![0, 2]);
        assert!(rising_edges(&[], 0.25, 0.5).is_empty());
    }

    fn beeps_at(positions: &[usize], len: usize) -> Vec<f32> {
        let mut a = vec![0.0f32; len];
        for &p in positions {
            for k in 0..480 {
                if let Some(s) = a.get_mut(p + k) {
                    *s = (2.0 * std::f32::consts::PI * 1000.0 * k as f32 / 48_000.0).sin() * 0.8;
                }
            }
        }
        a
    }

    #[test]
    fn beep_onsets_find_each_beep_once() {
        let a = beeps_at(&[0, 48_000, 96_000], 144_000);
        let on = beep_onsets(&[a], 48_000, 0.1, 100).unwrap();
        assert_eq!(on.len(), 3);
        // sin(0) = 0: the first sample at 0.1 comes within two samples of the true start
        for (o, t) in on.iter().zip([0i64, 48_000, 96_000]) {
            assert!((0..3).contains(&(o - t)), "{o} vs {t}");
        }
    }

    #[test]
    fn beep_onsets_reject_bad_input_and_report_silence_as_no_beep() {
        assert_eq!(beep_onsets(&[vec![0.0; 10]], 0, 0.1, 100), Err(MetricError::DataRange));
        assert_eq!(beep_onsets(&[vec![0.0; 10]], 48_000, f32::NAN, 100), Err(MetricError::DataRange));
        assert!(beep_onsets(&[], 48_000, 0.1, 100).is_err());
        assert_eq!(beep_onsets(&[vec![0.0; 1000]], 48_000, 0.1, 100).unwrap(), Vec::<i64>::new());
    }

    const FD: i64 = 1000;

    fn series(n: i64) -> Vec<i64> {
        (0..n).map(|i| i * FD).collect()
    }

    #[test]
    fn exact_series_has_no_error() {
        let s = series(24);
        let d = pts_diff(&s, &s, FD).unwrap();
        assert_eq!((d.dropped, d.extra, d.non_monotonic, d.median_offset, d.max_abs_error), (0, 0, 0, 0, 0));
        assert_eq!((d.index_median_offset, d.index_max_abs_error), (0, 0));
    }

    #[test]
    fn a_dropped_frame_is_counted() {
        let e = series(24);
        let mut a = e.clone();
        a.remove(10);
        let d = pts_diff(&e, &a, FD).unwrap();
        assert_eq!((d.dropped, d.extra, d.actual), (1, 0, 23));
    }

    #[test]
    fn a_duplicated_frame_is_extra_and_not_monotonic() {
        let e = series(24);
        let mut a = e.clone();
        a.insert(10, 9 * FD);
        let d = pts_diff(&e, &a, FD).unwrap();
        assert_eq!((d.dropped, d.extra, d.non_monotonic), (0, 1, 1));
    }

    #[test]
    fn a_one_frame_shift_is_a_one_frame_offset() {
        let e = series(24);
        let a: Vec<i64> = e.iter().map(|t| t + FD).collect();
        let d = pts_diff(&e, &a, FD).unwrap();
        // every frame matches its neighbour exactly, so the matched error is 0 …
        assert_eq!((d.median_offset, d.max_abs_error), (0, 0));
        // … and the shift shows by position, with the first frame missing and one extra at the end
        assert_eq!((d.index_median_offset, d.index_max_abs_error), (FD, FD));
        assert_eq!((d.dropped, d.extra), (1, 1));
    }

    #[test]
    fn drift_is_the_change_of_the_offset() {
        let v = [0, 24_000, 48_000]; // ticks at 24 000 per second: 0 s, 1 s, 2 s
        let a = [0, 48_000, 96_000 + 48]; // 48 kHz: 0 s, 1 s, 2 s + 1 ms
        let d = av_drift(&v, &a, 24_000, 48_000).unwrap();
        assert_eq!(d.initial_offset_ms, 0.0);
        assert!((d.max_drift_ms - 1.0).abs() < 1e-9, "{}", d.max_drift_ms);
    }

    #[test]
    fn seek_error_in_frames() {
        let e = seek_error_frames(&[0, 5_000, 9_000], &[0, 5_000, 11_000], FD).unwrap();
        assert_eq!(e, 2.0);
    }

    #[test]
    fn bad_input_is_an_error() {
        assert_eq!(pts_diff(&[], &[], FD), Err(MetricError::TooSmall { width: 0, height: 0 }));
        assert_eq!(pts_diff(&[0], &[0], 0), Err(MetricError::DataRange));
        assert_eq!(av_drift(&[0], &[], 24_000, 48_000), Err(MetricError::SizeMismatch { reference: 1, distorted: 0 }));
        assert_eq!(seek_error_frames(&[0], &[0], 0), Err(MetricError::DataRange));
    }
}
