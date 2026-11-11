// Derived from OxiMedia 0.2.1, crates/oximedia-quality/src/ciede2000.rs
// (https://github.com/cool-japan/oximedia, commit 5263510374761ec8ef02dace5d26d62fb20e405b),
// Copyright 2024-2026 COOLJAPAN OU (Team Kitasan), licensed under the Apache License 2.0
// (crates/metrics/LICENSE-oximedia). Modified for CFA4; CFA4 changes are GPL-3.0-or-later.
//! CIEDE2000 colour difference (CIE 142-2001; implementation notes: G. Sharma, W. Wu, E. N. Dalal,
//! "The CIEDE2000 Color-Difference Formula: Implementation Notes, Supplementary Test Data, and
//! Mathematical Observations", Color Research & Application 30(1), 2005).

use crate::{Lab, MetricError, pixel_count, srgb8_to_lab};

/// Parametric weighting factors kL, kC, kH (all 1 for the reference conditions).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeltaEParams {
    pub k_l: f64,
    pub k_c: f64,
    pub k_h: f64,
}

impl Default for DeltaEParams {
    fn default() -> Self {
        DeltaEParams { k_l: 1.0, k_c: 1.0, k_h: 1.0 }
    }
}

/// 25⁷.
const POW25_7: f64 = 6_103_515_625.0;

/// ΔE₀₀ between two Lab colours under the reference conditions (kL = kC = kH = 1).
pub fn ciede2000(x: &Lab, y: &Lab) -> f64 {
    delta_e(x, y, DeltaEParams::default())
}

/// ΔE₀₀ with explicit weighting factors, which must be positive and finite
/// ([`MetricError::Weights`] otherwise). Symmetric and never negative; NaN input gives NaN.
pub fn ciede2000_with(x: &Lab, y: &Lab, p: DeltaEParams) -> Result<f64, MetricError> {
    if ![p.k_l, p.k_c, p.k_h].iter().all(|k| k.is_finite() && *k > 0.0) {
        return Err(MetricError::Weights);
    }
    Ok(delta_e(x, y, p))
}

fn delta_e(x: &Lab, y: &Lab, p: DeltaEParams) -> f64 {
    // Sharma et al. (2005) eqs. (2)–(7): C*, G, a', C', h'.
    let c_bar = (x.a.hypot(x.b) + y.a.hypot(y.b)) / 2.0;
    let c_bar7 = c_bar.powi(7);
    let g = 0.5 * (1.0 - (c_bar7 / (c_bar7 + POW25_7)).sqrt());
    let (ap1, ap2) = (x.a * (1.0 + g), y.a * (1.0 + g));
    let (cp1, cp2) = (ap1.hypot(x.b), ap2.hypot(y.b));
    let (hp1, hp2) = (hue_deg(x.b, ap1), hue_deg(y.b, ap2));

    // Eqs. (8)–(11): ΔL', ΔC', Δh', ΔH'.
    let dl = y.l - x.l;
    let dc = cp2 - cp1;
    let chroma_zero = cp1 * cp2 == 0.0;
    let dh = if chroma_zero {
        0.0
    } else if (hp2 - hp1).abs() <= 180.0 {
        hp2 - hp1
    } else if hp2 - hp1 > 180.0 {
        hp2 - hp1 - 360.0
    } else {
        hp2 - hp1 + 360.0
    };
    let d_big_h = 2.0 * (cp1 * cp2).sqrt() * (dh.to_radians() / 2.0).sin();

    // Eqs. (12)–(22): means, weighting functions and the rotation term.
    let l_bar = (x.l + y.l) / 2.0;
    let cp_bar = (cp1 + cp2) / 2.0;
    let hp_bar = if chroma_zero {
        hp1 + hp2
    } else if (hp1 - hp2).abs() <= 180.0 {
        (hp1 + hp2) / 2.0
    } else if hp1 + hp2 < 360.0 {
        (hp1 + hp2 + 360.0) / 2.0
    } else {
        (hp1 + hp2 - 360.0) / 2.0
    };
    let t = 1.0 - 0.17 * (hp_bar - 30.0).to_radians().cos() + 0.24 * (2.0 * hp_bar).to_radians().cos() + 0.32 * (3.0 * hp_bar + 6.0).to_radians().cos()
        - 0.20 * (4.0 * hp_bar - 63.0).to_radians().cos();
    let l50 = (l_bar - 50.0).powi(2);
    let s_l = 1.0 + 0.015 * l50 / (20.0 + l50).sqrt();
    let s_c = 1.0 + 0.045 * cp_bar;
    let s_h = 1.0 + 0.015 * cp_bar * t;
    let cp_bar7 = cp_bar.powi(7);
    let r_c = 2.0 * (cp_bar7 / (cp_bar7 + POW25_7)).sqrt();
    let d_theta = 30.0 * (-((hp_bar - 275.0) / 25.0).powi(2)).exp();
    let r_t = -(2.0 * d_theta).to_radians().sin() * r_c;

    let (tl, tc, th) = (dl / (p.k_l * s_l), dc / (p.k_c * s_c), d_big_h / (p.k_h * s_h));
    // Rounding can push the sum a hair below zero; NaN must stay NaN (`f64::max` would hide it).
    let sum = tl * tl + tc * tc + th * th + r_t * tc * th;
    if sum < 0.0 { 0.0 } else { sum.sqrt() }
}

/// h' in degrees, 0..360; 0 when a' = b = 0 (Sharma eq. 7).
fn hue_deg(b: f64, ap: f64) -> f64 {
    if b == 0.0 && ap == 0.0 {
        return 0.0;
    }
    let h = b.atan2(ap).to_degrees();
    if h < 0.0 { h + 360.0 } else { h }
}

/// Per-image ΔE₀₀ summary.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DeltaEStats {
    pub mean: f64,
    pub max: f64,
    /// Share of pixels with ΔE₀₀ above 1 (the usual "just noticeable" threshold), 0..1.
    pub above_jnd: f64,
}

/// ΔE₀₀ between two 8-bit sRGB images, interleaved RGB (`channels` = 3) or RGBA (4; alpha is
/// ignored).
pub fn delta_e_srgb8(reference: &[u8], distorted: &[u8], width: usize, height: usize, channels: usize) -> Result<DeltaEStats, MetricError> {
    if channels != 3 && channels != 4 {
        return Err(MetricError::Channels(channels));
    }
    let n = pixel_count(width, height)?;
    if n == 0 {
        return Err(MetricError::TooSmall { width, height });
    }
    if reference.len() != distorted.len() {
        return Err(MetricError::SizeMismatch { reference: reference.len(), distorted: distorted.len() });
    }
    let expected = n.checked_mul(channels).ok_or(MetricError::TooLarge)?;
    if reference.len() != expected {
        return Err(MetricError::BadLength { expected, got: reference.len() });
    }
    let (mut sum, mut max, mut above) = (0.0, 0.0_f64, 0usize);
    for (r, d) in reference.chunks_exact(channels).zip(distorted.chunks_exact(channels)) {
        let (Some(&[r0, r1, r2]), Some(&[d0, d1, d2])) = (r.first_chunk::<3>(), d.first_chunk::<3>()) else {
            continue;
        };
        let e = if r0 == d0 && r1 == d1 && r2 == d2 { 0.0 } else { ciede2000(&srgb8_to_lab(r0, r1, r2), &srgb8_to_lab(d0, d1, d2)) };
        sum += e;
        max = max.max(e);
        if e > 1.0 {
            above += 1;
        }
    }
    Ok(DeltaEStats { mean: sum / n as f64, max, above_jnd: above as f64 / n as f64 })
}
