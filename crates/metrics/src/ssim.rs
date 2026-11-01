// Derived from OxiMedia 0.2.1, crates/oximedia-quality/src/ssim.rs
// (https://github.com/cool-japan/oximedia, commit 5263510374761ec8ef02dace5d26d62fb20e405b),
// Copyright 2024-2026 COOLJAPAN OU (Team Kitasan), licensed under the Apache License 2.0
// (crates/metrics/LICENSE-oximedia). Modified for CFA4; CFA4 changes are GPL-3.0-or-later.
//! Structural similarity (Z. Wang, A. C. Bovik, H. R. Sheikh, E. P. Simoncelli, "Image Quality
//! Assessment: From Error Visibility to Structural Similarity", IEEE TIP 13(4), 2004): 11 × 11
//! Gaussian window (σ = 1.5), K1 = 0.01, K2 = 0.03, population (co)variances, mean over the
//! positions where the whole window fits ("valid"). This equals scikit-image's
//! `structural_similarity(..., gaussian_weights=True, sigma=1.5, use_sample_covariance=False)`.

use crate::{MetricError, pixel_count};

/// Window side length.
pub const SSIM_WINDOW: usize = 11;
const RADIUS: usize = SSIM_WINDOW / 2;
const SIGMA: f64 = 1.5;
const K1: f64 = 0.01;
const K2: f64 = 0.03;

/// Mean SSIM of one plane (`width × height` samples, row-major, no padding). `data_range` is the
/// span of the sample values (255 for 8-bit, 1023 for 10-bit, 1.0 for normalised floats).
pub fn ssim(reference: &[f32], distorted: &[f32], width: usize, height: usize, data_range: f64) -> Result<f64, MetricError> {
    if !(data_range.is_finite() && data_range > 0.0) {
        return Err(MetricError::DataRange);
    }
    let n = check(reference.len(), distorted.len(), width, height)?;
    let x: Vec<f64> = reference.iter().take(n).map(|&v| f64::from(v)).collect();
    let y: Vec<f64> = distorted.iter().take(n).map(|&v| f64::from(v)).collect();
    Ok(ssim_f64(&x, &y, width, height, data_range))
}

/// [`ssim`] for 8-bit samples (data range 255).
pub fn ssim_u8(reference: &[u8], distorted: &[u8], width: usize, height: usize) -> Result<f64, MetricError> {
    let n = check(reference.len(), distorted.len(), width, height)?;
    let x: Vec<f64> = reference.iter().take(n).map(|&v| f64::from(v)).collect();
    let y: Vec<f64> = distorted.iter().take(n).map(|&v| f64::from(v)).collect();
    Ok(ssim_f64(&x, &y, width, height, 255.0))
}

/// [`ssim`] for samples of `bit_depth` bits (1..=16) held in `u16`; data range 2^bits − 1.
pub fn ssim_u16(reference: &[u16], distorted: &[u16], width: usize, height: usize, bit_depth: u32) -> Result<f64, MetricError> {
    if !(1..=16).contains(&bit_depth) {
        return Err(MetricError::DataRange);
    }
    let n = check(reference.len(), distorted.len(), width, height)?;
    let x: Vec<f64> = reference.iter().take(n).map(|&v| f64::from(v)).collect();
    let y: Vec<f64> = distorted.iter().take(n).map(|&v| f64::from(v)).collect();
    Ok(ssim_f64(&x, &y, width, height, f64::from((1u32 << bit_depth) - 1)))
}

fn check(ref_len: usize, dist_len: usize, width: usize, height: usize) -> Result<usize, MetricError> {
    if width < SSIM_WINDOW || height < SSIM_WINDOW {
        return Err(MetricError::TooSmall { width, height });
    }
    let n = pixel_count(width, height)?;
    if ref_len != dist_len {
        return Err(MetricError::SizeMismatch { reference: ref_len, distorted: dist_len });
    }
    if ref_len != n {
        return Err(MetricError::BadLength { expected: n, got: ref_len });
    }
    Ok(n)
}

/// Normalised 1-D Gaussian taps; the 2-D window is their outer product.
fn taps() -> [f64; SSIM_WINDOW] {
    let mut t = [0.0; SSIM_WINDOW];
    for (i, v) in t.iter_mut().enumerate() {
        let d = i as f64 - RADIUS as f64;
        *v = (-(d * d) / (2.0 * SIGMA * SIGMA)).exp();
    }
    let s: f64 = t.iter().sum();
    t.iter_mut().for_each(|v| *v /= s);
    t
}

/// Lengths are checked by the callers: `x.len() == y.len() == width * height`, both sides ≥ window.
fn ssim_f64(x: &[f64], y: &[f64], width: usize, height: usize, data_range: f64) -> f64 {
    let w = taps();
    let (ow, oh) = (width - SSIM_WINDOW + 1, height - SSIM_WINDOW + 1);
    // Separable "valid" filtering of x, y, x², y², xy: horizontal pass to (ow × height), then
    // vertical to (ow × oh).
    let mut h = vec![[0.0f64; 5]; ow * height];
    for row in 0..height {
        let base = row * width;
        for ox in 0..ow {
            let mut acc = [0.0f64; 5];
            for (k, wk) in w.iter().enumerate() {
                let i = base + ox + k;
                let (a, b) = (x.get(i).copied().unwrap_or(0.0), y.get(i).copied().unwrap_or(0.0));
                acc[0] += wk * a;
                acc[1] += wk * b;
                acc[2] += wk * a * a;
                acc[3] += wk * b * b;
                acc[4] += wk * a * b;
            }
            if let Some(slot) = h.get_mut(row * ow + ox) {
                *slot = acc;
            }
        }
    }
    let c1 = (K1 * data_range).powi(2);
    let c2 = (K2 * data_range).powi(2);
    let mut total = 0.0;
    for oy in 0..oh {
        for ox in 0..ow {
            let mut m = [0.0f64; 5];
            for (k, wk) in w.iter().enumerate() {
                if let Some(v) = h.get((oy + k) * ow + ox) {
                    for c in 0..5 {
                        m[c] += wk * v[c];
                    }
                }
            }
            let (mx, my) = (m[0], m[1]);
            let vx = m[2] - mx * mx;
            let vy = m[3] - my * my;
            let cxy = m[4] - mx * my;
            total += ((2.0 * mx * my + c1) * (2.0 * cxy + c2)) / ((mx * mx + my * my + c1) * (vx + vy + c2));
        }
    }
    total / (ow * oh) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_is_one() {
        let img: Vec<u8> = (0..32 * 24).map(|i| (i * 7 % 251) as u8).collect();
        let s = ssim_u8(&img, &img, 32, 24).unwrap();
        assert!((s - 1.0).abs() < 1e-12, "{s}");
    }

    #[test]
    fn rejects_bad_input() {
        let a = vec![0u8; 10 * 10];
        assert_eq!(ssim_u8(&a, &a, 10, 10), Err(MetricError::TooSmall { width: 10, height: 10 }));
        let b = vec![0u8; 12 * 12];
        assert!(matches!(ssim_u8(&b, &b[..100], 12, 12), Err(MetricError::SizeMismatch { .. })));
        assert!(matches!(ssim_u8(&b, &b, 12, 13), Err(MetricError::BadLength { .. })));
        assert_eq!(ssim_u8(&b, &b, usize::MAX, 12), Err(MetricError::TooLarge));
        let f = vec![0.0f32; 12 * 12];
        assert_eq!(ssim(&f, &f, 12, 12, 0.0), Err(MetricError::DataRange));
        assert_eq!(ssim(&f, &f, 12, 12, f64::NAN), Err(MetricError::DataRange));
    }
}
