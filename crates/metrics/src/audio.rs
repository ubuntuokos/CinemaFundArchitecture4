//! Audio null test: subtract the test signal from the reference and measure what is left, per
//! channel, in dBFS.
//!
//! A passing null test of a lossless path leaves silence (`−∞ dBFS`); a muted, swapped, shifted or
//! re-quantised channel leaves a measurable residual in exactly the channels it damaged, which is
//! why the result is kept per channel and not as one summed figure.

use crate::MetricError;

/// Level in dBFS of a linear sample magnitude (1.0 = full scale); `−∞` for silence.
pub fn dbfs(linear: f64) -> f64 {
    if linear <= 0.0 { f64::NEG_INFINITY } else { 20.0 * linear.log10() }
}

/// Residual of one channel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChannelNull {
    /// Largest absolute residual sample, in dBFS.
    pub peak_dbfs: f64,
    /// RMS of the residual, in dBFS.
    pub rms_dbfs: f64,
    /// Number of samples whose residual is not exactly zero.
    pub differing: usize,
}

/// Result of [`null_test`].
#[derive(Clone, Debug, PartialEq)]
pub struct AudioNull {
    /// One entry per channel.
    pub channels: Vec<ChannelNull>,
    /// Worst (highest) residual peak over all channels, in dBFS.
    pub worst_peak_dbfs: f64,
    /// `true` when every sample of every channel is identical.
    pub identical: bool,
}

/// Null test of `test` against `reference` (one `Vec` per channel, equal lengths).
pub fn null_test(reference: &[Vec<f32>], test: &[Vec<f32>]) -> Result<AudioNull, MetricError> {
    if reference.len() != test.len() {
        return Err(MetricError::SizeMismatch { reference: reference.len(), distorted: test.len() });
    }
    if reference.is_empty() {
        return Err(MetricError::Channels(0));
    }
    let mut channels = Vec::with_capacity(reference.len());
    for (r, t) in reference.iter().zip(test) {
        if r.len() != t.len() {
            return Err(MetricError::SizeMismatch { reference: r.len(), distorted: t.len() });
        }
        if r.is_empty() {
            return Err(MetricError::TooSmall { width: 0, height: 0 });
        }
        let (mut peak, mut sq, mut differing) = (0.0f64, 0.0f64, 0usize);
        for (a, b) in r.iter().zip(t) {
            let d = (f64::from(*a) - f64::from(*b)).abs();
            if d > 0.0 {
                differing += 1;
                sq += d * d;
                peak = peak.max(d);
            }
        }
        channels.push(ChannelNull { peak_dbfs: dbfs(peak), rms_dbfs: dbfs((sq / r.len() as f64).sqrt()), differing });
    }
    let worst_peak_dbfs = channels.iter().map(|c| c.peak_dbfs).fold(f64::NEG_INFINITY, f64::max);
    let identical = channels.iter().all(|c| c.differing == 0);
    Ok(AudioNull { channels, worst_peak_dbfs, identical })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize) -> Vec<f32> {
        (0..n).map(|i| (i as f32 * 0.05).sin() * 0.5).collect()
    }

    #[test]
    fn identical_signals_null_to_silence() {
        let a = vec![tone(480), tone(480)];
        let r = null_test(&a, &a.clone()).unwrap();
        assert!(r.identical);
        assert_eq!(r.worst_peak_dbfs, f64::NEG_INFINITY);
    }

    #[test]
    fn a_muted_channel_leaves_a_residual_in_that_channel_only() {
        let a = vec![tone(480), tone(480)];
        let mut b = a.clone();
        b[1] = vec![0.0; 480];
        let r = null_test(&a, &b).unwrap();
        assert!(!r.identical);
        assert_eq!(r.channels[0].differing, 0);
        assert!(r.channels[1].differing > 400);
        // the tone peaks at 0.5, i.e. about −6 dBFS
        assert!((r.worst_peak_dbfs + 6.02).abs() < 0.2, "{}", r.worst_peak_dbfs);
    }

    #[test]
    fn dbfs_of_known_levels() {
        assert_eq!(dbfs(1.0), 0.0);
        assert!((dbfs(0.5) + 6.0206).abs() < 1e-3);
        assert_eq!(dbfs(0.0), f64::NEG_INFINITY);
    }

    #[test]
    fn shape_errors_are_reported() {
        let a = vec![tone(10)];
        assert_eq!(null_test(&a, &[]), Err(MetricError::SizeMismatch { reference: 1, distorted: 0 }));
        assert_eq!(null_test(&a, &[tone(9)]), Err(MetricError::SizeMismatch { reference: 10, distorted: 9 }));
        assert_eq!(null_test(&[], &[]), Err(MetricError::Channels(0)));
    }
}
