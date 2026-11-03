// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-color 0.11.0.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

mod bt709;
pub(crate) mod pq;
pub(crate) mod rec2408;
mod srgb;

pub use bt709::*;
pub use pq::*;
pub use srgb::*;

/// Applies gamma to samples.
pub fn apply_gamma(samples: &mut [f32], gamma: f32) {
    for x in samples {
        let a = *x;
        *x = if a <= 1e-7 { 0.0 } else { crate::color::fastmath::fast_powf_generic(a, gamma) };
    }
}

/// Converts scene luminance values to display luminance values using the hybrid log-gamma
/// transfer function (HLG OOTF).
pub fn hlg_oo([samples_r, samples_g, samples_b]: [&mut [f32]; 3], [lr, lg, lb]: [f32; 3], intensity_target: f32) {
    let gamma = 1.2f32 * 1.111f32.powf((intensity_target / 1e3).log2());
    // 1/g - 1
    let exp = gamma - 1.0;

    for ((r, g), b) in samples_r.iter_mut().zip(samples_g).zip(samples_b) {
        let mixed = r.mul_add(lr, g.mul_add(lg, *b * lb));
        let mult = mixed.powf(exp);
        *r *= mult;
        *g *= mult;
        *b *= mult;
    }
}

/// Converts the display-referred samples to scene-referred signals using the hybrid log-gamma
/// transfer function (HLG inverse OOTF).
pub fn hlg_inverse_oo([samples_r, samples_g, samples_b]: [&mut [f32]; 3], [lr, lg, lb]: [f32; 3], intensity_target: f32) {
    // System gamma results to ~1 in this range.
    if (295.0..=305.0).contains(&intensity_target) {
        return;
    }

    let gamma = 1.2f32 * 1.111f32.powf((intensity_target / 1e3).log2());
    // 1/g - 1
    let exp = (1.0 - gamma) / gamma;

    for ((r, g), b) in samples_r.iter_mut().zip(samples_g).zip(samples_b) {
        let mixed = r.mul_add(lr, g.mul_add(lg, *b * lb));
        let mult = mixed.powf(exp);
        *r *= mult;
        *g *= mult;
        *b *= mult;
    }
}

const HLG_A: f32 = 0.17883277;
const HLG_B: f32 = 0.28466892;
const HLG_C: f32 = 0.5599107;

/// Converts the scene-referred linear samples with the hybrid log-gamma transfer function.
pub fn linear_to_hlg(samples: &mut [f32]) {
    for s in samples {
        let a = s.abs();
        *s = if a <= 1.0 / 12.0 { (3.0 * a).sqrt() } else { HLG_A * a.mul_add(12.0, -HLG_B).ln() + HLG_C }.copysign(*s);
    }
}

/// HLG inverse OETF, maps non-linear HLG signal to scene-referred linear sample.
pub fn hlg_to_linear(samples: &mut [f32]) {
    for s in samples {
        let a = s.abs();
        *s = if a <= 0.5 { a * a / 3.0 } else { (((a - HLG_C) / HLG_A).exp() + HLG_B) / 12.0 }.copysign(*s);
    }
}

pub(crate) fn hlg_table(n: usize) -> Vec<u16> {
    const A: f64 = 0.17883277;
    const B: f64 = 0.28466892;
    const C: f64 = 0.5599107;

    let mut out = vec![0u16; n];
    for (idx, out) in out[..=(n - 1) / 2].iter_mut().enumerate() {
        let d = (idx * idx) as f64 / (3 * (n - 1) * (n - 1)) as f64;
        *out = (d * 65535.0) as u16; // clamped
    }
    for (idx, out) in out[(n - 1) / 2 + 1..].iter_mut().enumerate() {
        let idx = idx + (n - 1) / 2 + 1;
        let e = idx as f64 / (n - 1) as f64;
        let d = (((e - C) / A).exp() + B) / 12.0;
        *out = (d * 65535.0) as u16; // clamped
    }
    out
}
