//! MQ-1 meter (CFA4-RENDER-INTEGRATION-PLAN-001, phase F2-0): does a render keep out-of-range
//! values between layers?
//!
//! MQ-1 asks for at least 16-bit floating intermediate precision. The observable consequence is
//! that a value above 1.0 or below 0.0 (a highlight, an undershoot) survives a layer stack. The
//! meter checks exactly that, on material that is generated here and has a known answer:
//!
//! * [`fixture`] builds three premultiplied linear RGBA layers whose values run from −0.2 to 4.0;
//! * [`truth`] composites them with the Normal (over) operator in `f64`;
//! * [`measure`] compares what a renderer produced, an `f32` [`RangeFrame`], with that truth and
//!   reports the error separately for pixels whose true value is inside and outside [0, 1].
//!
//! The harness [`crate::validation`] stores display-encoded `u16`, which cannot hold such values;
//! [`RangeFrame`] is the `f32` capture that can. All numbers made with this meter are synthetic.

use std::fmt::Write as _;

/// A rendered frame in premultiplied linear light, `f32` RGBA, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct RangeFrame {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<f32>,
}

impl RangeFrame {
    /// A frame; `None` when `rgba` is not `width × height × 4` samples.
    pub fn new(width: usize, height: usize, rgba: Vec<f32>) -> Option<RangeFrame> {
        (width.checked_mul(height)?.checked_mul(4)? == rgba.len()).then_some(RangeFrame { width, height, rgba })
    }
}

/// One layer of the fixture: premultiplied linear RGBA and an opacity applied when compositing.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub rgba: Vec<f32>,
    pub opacity: f32,
}

/// The fixture: three layers over a `width × height` frame, built from closed formulas only (no
/// random numbers, no files). Layer 0 is opaque and its colour runs from −0.2 to 4.0 across the
/// frame; layer 1 is half transparent with highlights up to 2.0; layer 2 has opacity 0.5 and dips
/// below zero. Returns `None` for a zero or too large size.
pub fn fixture(width: usize, height: usize) -> Option<Vec<Layer>> {
    let n = width.checked_mul(height).filter(|&n| n > 0 && n <= 1 << 24)?;
    let mut l0 = Vec::with_capacity(n * 4);
    let mut l1 = Vec::with_capacity(n * 4);
    let mut l2 = Vec::with_capacity(n * 4);
    for y in 0..height {
        for x in 0..width {
            let u = x as f32 / (width.max(2) - 1) as f32;
            let v = y as f32 / (height.max(2) - 1) as f32;
            // layer 0: opaque ramp from -0.2 to 4.0 in R, G ramps the other way, B a diagonal
            l0.extend_from_slice(&[-0.2 + 4.2 * u, -0.2 + 4.2 * (1.0 - u), -0.2 + 4.2 * (u + v) / 2.0, 1.0]);
            // layer 1: alpha ramp down the frame, premultiplied colour reaching 2.0
            let a1 = 0.25 + 0.5 * v;
            l1.extend_from_slice(&[2.0 * a1 * u, 2.0 * a1 * (1.0 - u), a1, a1]);
            // layer 2: alpha 0.5, colour that dips to -0.2 (an undershoot, e.g. a sharpening ring)
            l2.extend_from_slice(&[0.5 * (-0.2 + 1.2 * v), 0.5 * (-0.2 + 1.2 * u), 0.5 * (1.0 - u) * v, 0.5]);
        }
    }
    Some(vec![Layer { rgba: l0, opacity: 1.0 }, Layer { rgba: l1, opacity: 1.0 }, Layer { rgba: l2, opacity: 0.5 }])
}

/// The Normal-over composite of `layers`, bottom first, in `f64`. Empty when the layers disagree
/// in size or hold a non-finite value.
pub fn truth(layers: &[Layer]) -> Vec<f64> {
    let Some(first) = layers.first() else { return Vec::new() };
    if layers.iter().any(|l| l.rgba.len() != first.rgba.len() || l.rgba.iter().any(|v| !v.is_finite()) || !l.opacity.is_finite()) {
        return Vec::new();
    }
    let mut out = vec![0.0f64; first.rgba.len()];
    for l in layers {
        let o = f64::from(l.opacity);
        for (d, s) in out.chunks_exact_mut(4).zip(l.rgba.chunks_exact(4)) {
            let sa = f64::from(s[3]) * o;
            let k = 1.0 - sa;
            for c in 0..4 {
                d[c] = f64::from(s[c]) * o + d[c] * k;
            }
        }
    }
    out
}

/// What [`measure`] found.
#[derive(Clone, Debug, PartialEq)]
pub struct RangeReport {
    pub samples: usize,
    /// RGB samples whose true value lies outside [0, 1].
    pub out_of_range_samples: usize,
    /// Largest |error| over samples whose true value is inside [0, 1].
    pub max_err_in_range: f64,
    /// Largest |error| over samples whose true value is outside [0, 1].
    pub max_err_out_of_range: f64,
    /// Mean |error| over all RGB samples.
    pub mean_err: f64,
    /// Out-of-range samples the test frame has clamped into [0, 1] (the signature of a quantising path).
    pub clamped_samples: usize,
    /// Largest |error| allowed for the sample that produced `max_err_out_of_range` (the rounding bound).
    pub tolerance_at_worst: f64,
    /// MQ-1 on this material: every out-of-range sample within its rounding bound.
    pub pass: bool,
}

/// The rounding bound for one `f32` sample: 8 ulps of the larger of 1 and the true value. Three
/// layers of one multiply and one add each stay far inside it; it is not a quality threshold.
pub fn tolerance(truth: f64) -> f64 {
    8.0 * f64::from(f32::EPSILON) * truth.abs().max(1.0)
}

/// Compare `test` with `truth` (as [`truth`] returns it). `None` when the sizes disagree or
/// there is nothing to compare, so that a missing measurement is never a pass.
pub fn measure(truth: &[f64], test: &RangeFrame) -> Option<RangeReport> {
    if truth.is_empty() || truth.len() != test.rgba.len() || !truth.len().is_multiple_of(4) {
        return None;
    }
    let mut r = RangeReport {
        samples: 0,
        out_of_range_samples: 0,
        max_err_in_range: 0.0,
        max_err_out_of_range: 0.0,
        mean_err: 0.0,
        clamped_samples: 0,
        tolerance_at_worst: 0.0,
        pass: true,
    };
    let mut sum = 0.0;
    for (t, p) in truth.chunks_exact(4).zip(test.rgba.chunks_exact(4)) {
        for c in 0..3 {
            let (tv, pv) = (t[c], f64::from(p[c]));
            let err = if pv.is_finite() { (pv - tv).abs() } else { f64::INFINITY };
            r.samples += 1;
            sum += err.min(1e9);
            if (0.0..=1.0).contains(&tv) {
                r.max_err_in_range = r.max_err_in_range.max(err);
            } else {
                r.out_of_range_samples += 1;
                if err > r.max_err_out_of_range {
                    r.max_err_out_of_range = err;
                    r.tolerance_at_worst = tolerance(tv);
                }
                if (0.0..=1.0).contains(&pv) {
                    r.clamped_samples += 1;
                }
                if err > tolerance(tv) {
                    r.pass = false;
                }
            }
        }
    }
    // a fixture without any out-of-range sample proves nothing about MQ-1
    if r.out_of_range_samples == 0 {
        r.pass = false;
    }
    r.mean_err = sum / r.samples.max(1) as f64;
    Some(r)
}

impl RangeReport {
    pub fn to_markdown(&self, label: &str) -> String {
        let mut s = String::new();
        let _ = write!(
            s,
            "| {label} | {} | {} | {:.3e} | {:.3e} | {} | {} |",
            self.samples,
            self.out_of_range_samples,
            self.max_err_in_range,
            self.max_err_out_of_range,
            self.clamped_samples,
            if self.pass { "PASS" } else { "FAIL" }
        );
        s
    }
}

/// Quantise like a pipeline that stores each layer result in integers: `scale` codes per 1.0 and
/// `max_code` the largest code. An emulation used to prove the meter can fail, not a model of any candidate.
pub fn quantise(frame: &mut [f32], scale: f32, max_code: f32) {
    for v in frame {
        *v = (*v * scale).round().clamp(0.0, max_code) / scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exact(layers: &[Layer]) -> RangeFrame {
        let t = truth(layers);
        let (w, h) = (16, 8);
        RangeFrame { width: w, height: h, rgba: t.iter().map(|&v| v as f32).collect() }
    }

    #[test]
    fn the_fixture_reaches_far_outside_the_unit_range() {
        let t = truth(&fixture(16, 8).unwrap());
        let (lo, hi) = t.chunks_exact(4).flat_map(|p| p[..3].iter().copied()).fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(v), b.max(v)));
        assert!(lo < -0.1 && hi > 2.0, "{lo} .. {hi}");
    }

    #[test]
    fn an_f32_copy_of_the_truth_passes() {
        let layers = fixture(16, 8).unwrap();
        let r = measure(&truth(&layers), &exact(&layers)).unwrap();
        assert!(r.pass && r.out_of_range_samples > 100 && r.clamped_samples == 0, "{r:?}");
    }

    /// F0V rule: the harness proves it can fail. Pipelines that store each layer in integers lose the
    /// out-of-range values and must be reported as failures.
    #[test]
    fn a_quantising_pipeline_fails() {
        let layers = fixture(16, 8).unwrap();
        for (name, scale, max) in [("8 bit", 255.0, 255.0), ("16 bit, 1.0 = 32768", 32768.0, 65535.0), ("16 bit, 1.0 = 65535", 65535.0, 65535.0)] {
            let mut stored: Vec<Layer> = layers.clone();
            for l in &mut stored {
                quantise(&mut l.rgba, scale, max);
            }
            let r = measure(&truth(&layers), &RangeFrame { width: 16, height: 8, rgba: truth(&stored).iter().map(|&v| v as f32).collect() }).unwrap();
            assert!(!r.pass, "{name}: {r:?}");
        }
    }

    #[test]
    fn a_missing_or_broken_measurement_is_never_a_pass() {
        let layers = fixture(16, 8).unwrap();
        let good = exact(&layers);
        assert!(measure(&[], &good).is_none());
        assert!(measure(&truth(&layers), &RangeFrame { width: 1, height: 1, rgba: vec![0.0; 4] }).is_none());
        let mut nan = good.clone();
        nan.rgba[0] = f32::NAN;
        assert!(!measure(&truth(&layers), &nan).unwrap().pass);
        // an all-in-range fixture proves nothing about MQ-1
        let flat = vec![Layer { rgba: vec![0.5; 16 * 8 * 4], opacity: 1.0 }];
        assert!(!measure(&truth(&flat), &RangeFrame { width: 16, height: 8, rgba: vec![0.5; 16 * 8 * 4] }).unwrap().pass);
        assert!(truth(&[Layer { rgba: vec![f32::NAN; 4], opacity: 1.0 }]).is_empty());
        assert!(fixture(0, 4).is_none());
    }
}
