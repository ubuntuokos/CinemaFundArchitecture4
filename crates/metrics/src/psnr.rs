//! Plane difference statistics: mean squared error, PSNR, largest absolute difference and the
//! number of differing samples, for 8-bit, 9–16-bit and floating-point planes.
//!
//! These are the numbers the render validation harness (CFA4-RENDER-INTEGRATION-PLAN-001, F0V)
//! gates on, plane by plane. Identical planes give `mse = 0` and `psnr = +∞`, so a caller that
//! needs bit-exact output tests `differing == 0` rather than comparing floating-point values.

use crate::MetricError;

/// Difference statistics of one plane or sample buffer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlaneDiff {
    /// Mean squared error in sample units.
    pub mse: f64,
    /// `10·log10(peak² / mse)` in dB; `f64::INFINITY` when the planes are identical.
    pub psnr: f64,
    /// Largest absolute sample difference, in sample units.
    pub max_abs: f64,
    /// Number of samples that differ.
    pub differing: usize,
    /// Number of samples compared.
    pub samples: usize,
}

fn diff_generic<T: Copy + Into<f64>>(reference: &[T], distorted: &[T], peak: f64) -> Result<PlaneDiff, MetricError> {
    if reference.len() != distorted.len() {
        return Err(MetricError::SizeMismatch { reference: reference.len(), distorted: distorted.len() });
    }
    if !(peak.is_finite() && peak > 0.0) {
        return Err(MetricError::DataRange);
    }
    if reference.is_empty() {
        return Err(MetricError::TooSmall { width: 0, height: 0 });
    }
    let (mut sq, mut max_abs, mut differing) = (0.0f64, 0.0f64, 0usize);
    for (a, b) in reference.iter().zip(distorted) {
        let d = (Into::<f64>::into(*a) - Into::<f64>::into(*b)).abs();
        if d > 0.0 {
            differing += 1;
            sq += d * d;
            max_abs = max_abs.max(d);
        }
    }
    let samples = reference.len();
    let mse = sq / samples as f64;
    let psnr = if mse == 0.0 { f64::INFINITY } else { 10.0 * (peak * peak / mse).log10() };
    Ok(PlaneDiff { mse, psnr, max_abs, differing, samples })
}

/// [`PlaneDiff`] of two 8-bit planes (peak 255).
pub fn diff_u8(reference: &[u8], distorted: &[u8]) -> Result<PlaneDiff, MetricError> {
    diff_generic(reference, distorted, 255.0)
}

/// [`PlaneDiff`] of two planes of `bit_depth` bits (1..=16) held in `u16` (peak 2^bits − 1).
pub fn diff_u16(reference: &[u16], distorted: &[u16], bit_depth: u32) -> Result<PlaneDiff, MetricError> {
    if !(1..=16).contains(&bit_depth) {
        return Err(MetricError::DataRange);
    }
    diff_generic(reference, distorted, f64::from((1u32 << bit_depth) - 1))
}

/// [`PlaneDiff`] of two floating-point planes; `data_range` is the span of the sample values
/// (1.0 for normalised samples).
pub fn diff_f32(reference: &[f32], distorted: &[f32], data_range: f64) -> Result<PlaneDiff, MetricError> {
    diff_generic(reference, distorted, data_range)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_planes_are_infinite_psnr_and_zero_differing() {
        let a = [1u8, 2, 3, 4];
        let d = diff_u8(&a, &a).unwrap();
        assert_eq!((d.mse, d.max_abs, d.differing, d.samples), (0.0, 0.0, 0, 4));
        assert!(d.psnr.is_infinite());
    }

    #[test]
    fn known_8_bit_psnr() {
        // every sample off by 1: MSE 1, PSNR = 20·log10(255) = 48.1308 dB
        let a = [10u8; 16];
        let b = [11u8; 16];
        let d = diff_u8(&a, &b).unwrap();
        assert!((d.psnr - 20.0 * 255f64.log10()).abs() < 1e-9);
        assert_eq!((d.max_abs, d.differing), (1.0, 16));
    }

    #[test]
    fn sixteen_bit_one_code_is_far_below_one_8_bit_level() {
        let a = [30000u16; 8];
        let mut b = a;
        b[3] = 30001;
        let d = diff_u16(&a, &b, 16).unwrap();
        assert_eq!((d.max_abs, d.differing), (1.0, 1));
        assert!(d.psnr > 100.0, "{}", d.psnr);
    }

    #[test]
    fn float_planes_use_the_data_range() {
        let a = [0.0f32, 0.5, 1.0, 0.25];
        let b = [0.0f32, 0.5, 1.0, 0.75];
        let d = diff_f32(&a, &b, 1.0).unwrap();
        assert_eq!(d.max_abs, 0.5);
        assert!((d.mse - 0.0625).abs() < 1e-12);
        assert_eq!(diff_f32(&a, &b, 0.0), Err(MetricError::DataRange));
    }

    #[test]
    fn errors_are_reported_not_panicked() {
        assert_eq!(diff_u8(&[1, 2], &[1]), Err(MetricError::SizeMismatch { reference: 2, distorted: 1 }));
        assert_eq!(diff_u8(&[], &[]), Err(MetricError::TooSmall { width: 0, height: 0 }));
        assert_eq!(diff_u16(&[1], &[1], 0), Err(MetricError::DataRange));
        assert_eq!(diff_u16(&[1], &[1], 17), Err(MetricError::DataRange));
    }
}
