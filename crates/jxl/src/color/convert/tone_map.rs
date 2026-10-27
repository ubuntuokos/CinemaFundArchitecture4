// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-color 0.11.0.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

use super::HdrParams;

#[allow(unreachable_code)]
pub(super) fn tone_map(r: &mut [f32], g: &mut [f32], b: &mut [f32], hdr_params: &HdrParams, target_display_luminance: f32, detect_peak: bool) {
    assert_eq!(r.len(), g.len());
    assert_eq!(g.len(), b.len());

    let luminances = hdr_params.luminances;
    let intensity_target = hdr_params.intensity_target;
    let min_nits = hdr_params.min_nits;
    let detected_peak_luminance = if detect_peak { detect_peak_luminance(r, g, b, luminances) * intensity_target } else { intensity_target };
    let peak_luminance = intensity_target.min(detected_peak_luminance);

    let from_luminance_range = (min_nits, peak_luminance);
    let to_luminance_range = (0.0, target_display_luminance);

    tone_map_generic(r, g, b, luminances, intensity_target, from_luminance_range, to_luminance_range);
}

#[allow(unreachable_code)]
pub(super) fn tone_map_luma(luma: &mut [f32], hdr_params: &HdrParams, target_display_luminance: f32, detect_peak: bool) {
    let intensity_target = hdr_params.intensity_target;
    let min_nits = hdr_params.min_nits;
    let detected_peak_luminance = if detect_peak {
        let max_luma = luma.iter().copied().fold(0f32, |max, v| max.max(v));
        if max_luma == 0.0 { intensity_target } else { max_luma * intensity_target }
    } else {
        intensity_target
    };
    let peak_luminance = intensity_target.min(detected_peak_luminance);

    let from_luminance_range = (min_nits, peak_luminance);
    let to_luminance_range = (0.0, target_display_luminance);

    tone_map_luma_generic(luma, intensity_target, from_luminance_range, to_luminance_range);
}

fn tone_map_generic(
    r: &mut [f32],
    g: &mut [f32],
    b: &mut [f32],
    luminances: [f32; 3],
    intensity_target: f32,
    from_luminance_range: (f32, f32),
    to_luminance_range: (f32, f32),
) {
    assert_eq!(r.len(), g.len());
    assert_eq!(g.len(), b.len());
    let scale = intensity_target / to_luminance_range.1;

    let [lr, lg, lb] = luminances;
    for ((r, g), b) in r.iter_mut().zip(g).zip(b) {
        let y = *r * lr + *g * lg + *b * lb;
        let y_pq = crate::color::tf::pq::linear_to_pq_generic(y, intensity_target);
        let y_mapped = crate::color::tf::rec2408::rec2408_eetf_generic(y_pq, intensity_target, from_luminance_range, to_luminance_range);
        let y_mapped = crate::color::tf::pq::pq_to_linear_generic(y_mapped, intensity_target);
        let ratio = if y.abs() <= 1e-7 { y_mapped * scale } else { y_mapped / y * scale };
        *r *= ratio;
        *g *= ratio;
        *b *= ratio;
    }
}

fn tone_map_luma_generic(luma: &mut [f32], intensity_target: f32, from_luminance_range: (f32, f32), to_luminance_range: (f32, f32)) {
    let scale = intensity_target / to_luminance_range.1;

    for y in luma {
        let y_pq = crate::color::tf::pq::linear_to_pq_generic(*y, intensity_target);
        let y_mapped = crate::color::tf::rec2408::rec2408_eetf_generic(y_pq, intensity_target, from_luminance_range, to_luminance_range);
        let y_mapped = crate::color::tf::pq::pq_to_linear_generic(y_mapped, intensity_target);
        *y = y_mapped * scale;
    }
}

fn detect_peak_luminance(r: &[f32], g: &[f32], b: &[f32], luminances: [f32; 3]) -> f32 {
    detect_peak_luminance_generic(r, g, b, luminances)
}

#[inline]
fn detect_peak_luminance_generic(r: &[f32], g: &[f32], b: &[f32], luminances: [f32; 3]) -> f32 {
    assert_eq!(r.len(), g.len());
    assert_eq!(g.len(), b.len());

    let [lr, lg, lb] = luminances;
    let mut peak_luminance = 0f32;
    for ((r, g), b) in r.iter().zip(g).zip(b) {
        let y = r * lr + g * lg + b * lb;
        peak_luminance = if peak_luminance < y { y } else { peak_luminance };
    }

    if peak_luminance <= 0.0 { 1.0 } else { peak_luminance }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_map_range() {
        let mut samples = [0f32; 10];
        for (idx, v) in samples.iter_mut().enumerate() {
            *v = (idx / 5) as f32 * 0.1;
        }
        let mut r = samples;
        let mut g = samples;
        let mut b = samples;

        let hdr_params = HdrParams { luminances: [0.2126, 0.7152, 0.0722], intensity_target: 10000.0, min_nits: 0.0 };
        tone_map(&mut r, &mut g, &mut b, &hdr_params, 255.0, false);

        for (idx, ((r, g), b)) in r.into_iter().zip(g).zip(b).enumerate() {
            let expected = (idx / 5) as f32 * 0.8714331;
            assert!((r - expected).abs() < 2e-5);
            assert!((g - expected).abs() < 2e-5);
            assert!((b - expected).abs() < 2e-5);
        }
    }

    #[test]
    fn tone_map_range_detect_peak() {
        let mut samples = [0f32; 10];
        for (idx, v) in samples.iter_mut().enumerate() {
            *v = (idx / 5) as f32 * 0.1;
        }
        let mut r = samples;
        let mut g = samples;
        let mut b = samples;

        let hdr_params = HdrParams { luminances: [0.2126, 0.7152, 0.0722], intensity_target: 10000.0, min_nits: 0.0 };
        tone_map(&mut r, &mut g, &mut b, &hdr_params, 255.0, true);

        for (idx, ((r, g), b)) in r.into_iter().zip(g).zip(b).enumerate() {
            let expected = (idx / 5) as f32;
            assert!((r - expected).abs() < 2e-5);
            assert!((g - expected).abs() < 2e-5);
            assert!((b - expected).abs() < 2e-5);
        }
    }

    #[test]
    fn detect_peak() {
        let samples = [0f32, 0.05, 0.075, 0.1];
        let r = samples;
        let g = samples;
        let b = samples;
        let peak = detect_peak_luminance(&r, &g, &b, [0.2126, 0.7152, 0.0722]);
        assert!((peak - 0.1).abs() < 1e-6);
    }

    #[test]
    fn detect_peak_zero() {
        let samples = [0f32, 0.0];
        let r = samples;
        let g = samples;
        let b = samples;
        let peak = detect_peak_luminance(&r, &g, &b, [0.2126, 0.7152, 0.0722]);
        assert!(peak == 1.0);
    }
}
