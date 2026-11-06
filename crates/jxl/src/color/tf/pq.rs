// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-color 0.11.0.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

//! Approximation of perceptual quantizer transfer functions, ported from libjxl.

use crate::color::fastmath::rational_poly;

const EOTF_P: [f32; 5] = [2.6297566e-4, -6.235531e-3, 7.386023e-1, 2.6455317, 5.500349e-1];
const EOTF_Q: [f32; 5] = [4.213501e2, -4.2873682e2, 1.7436467e2, -3.3907887e1, 2.6771877];

const INV_EOTF_P: [f32; 5] = [1.351392e-2, -1.095778, 5.522776e1, 1.492516e2, 4.838434e1];
const INV_EOTF_Q: [f32; 5] = [1.012416, 2.016708e1, 9.26371e1, 1.120607e2, 2.590418e1];
const INV_EOTF_P_SMALL: [f32; 5] = [9.863406e-6, 3.881234e-1, 1.352821e2, 6.889862e4, -2.864824e5];
const INV_EOTF_Q_SMALL: [f32; 5] = [3.371868e1, 1.477719e3, 1.608477e4, -4.389884e4, -2.072546e5];

/// Converts the linear samples with the PQ transfer function, where linear sample value of 1.0
/// represents `intensity_target` nits (PQ inverse EOTF).
#[allow(unused_mut)]
pub fn linear_to_pq(mut samples: &mut [f32], intensity_target: f32) {
    for s in samples {
        *s = linear_to_pq_generic(*s, intensity_target);
    }
}

#[inline]
pub(crate) fn linear_to_pq_generic(s: f32, intensity_target: f32) -> f32 {
    let y_mult = intensity_target / 10000.0;
    let a = s.abs();
    let a_scaled = a * y_mult;
    let a_1_4 = a_scaled.sqrt().sqrt();

    let y = if a < 1e-4 {
        rational_poly::eval_generic(a_1_4, INV_EOTF_P_SMALL, INV_EOTF_Q_SMALL)
    } else {
        rational_poly::eval_generic(a_1_4, INV_EOTF_P, INV_EOTF_Q)
    };

    y.copysign(s)
}

/// Converts non-linear PQ signals to linear display luminance values, where luminance value of 1.0
/// represents `intensity_target` nits (PQ EOTF).
#[allow(unused_mut)]
pub fn pq_to_linear(mut samples: &mut [f32], intensity_target: f32) {
    for s in samples {
        *s = pq_to_linear_generic(*s, intensity_target);
    }
}

#[inline]
pub(crate) fn pq_to_linear_generic(s: f32, intensity_target: f32) -> f32 {
    let y_mult = 10000.0 / intensity_target;
    let a = s.abs();
    let x = a.mul_add(a, a);
    let y = rational_poly::eval_generic(x, EOTF_P, EOTF_Q);
    (y * y_mult).copysign(s)
}

pub(crate) fn pq_table(n: usize) -> Vec<u16> {
    const M1_RECIP_F64: f64 = 8192.0 / 1305.0;
    const M2_RECIP_F64: f64 = 32.0 / 2523.0;
    const C1_F64: f64 = 107.0 / 128.0;
    const C2_F64: f64 = 2413.0 / 128.0;
    const C3_F64: f64 = 2392.0 / 128.0;

    let mut out = vec![0u16; n];
    for (idx, out) in out.iter_mut().enumerate() {
        let e = idx as f64 / (n - 1) as f64;

        let e_pow = e.powf(M2_RECIP_F64);
        let numerator = (e_pow - C1_F64).max(0.0);
        let denominator = e_pow.mul_add(-C3_F64, C2_F64);
        let d = (numerator / denominator).powf(M1_RECIP_F64);
        *out = (d * 65535.0) as u16; // clamped
    }
    out
}

#[cfg(test)]
mod tests {
    const M1: f32 = 1305.0 / 8192.0;
    const M2: f32 = 2523.0 / 32.0;
    const C1: f32 = 107.0 / 128.0;
    const C2: f32 = 2413.0 / 128.0;
    const C3: f32 = 2392.0 / 128.0;

    #[test]
    fn pq_inverse_eotf_100k_generic() {
        let mut input = Vec::with_capacity(100000);
        for (idx, v) in input.iter_mut().enumerate() {
            *v = idx as f32 * 1e-5;
        }

        for v in &mut input {
            *v = super::linear_to_pq_generic(*v, 10000.0);
        }

        for (idx, v) in input.iter().enumerate() {
            let linear = idx as f32 * 1e-5;
            let y_m1 = linear.powf(M1);
            let expected = ((y_m1.mul_add(C2, C1)) / (y_m1.mul_add(C3, 1.0))).powf(M2);
            let diff = (expected - *v).abs();
            assert!(diff < 1e-6);
        }
    }

    #[test]
    fn pq_inverse_eotf_100k() {
        let mut input = Vec::with_capacity(100000);
        for (idx, v) in input.iter_mut().enumerate() {
            *v = idx as f32 * 1e-5;
        }

        super::linear_to_pq(&mut input, 10000.0);

        for (idx, v) in input.iter().enumerate() {
            let linear = idx as f32 * 1e-5;
            let y_m1 = linear.powf(M1);
            let expected = ((y_m1.mul_add(C2, C1)) / (y_m1.mul_add(C3, 1.0))).powf(M2);
            let diff = (expected - *v).abs();
            assert!(diff < 1e-6);
        }
    }

    #[test]
    fn pq_roundtrip_10k_generic() {
        let mut input = Vec::with_capacity(10000);
        for (idx, v) in input.iter_mut().enumerate() {
            *v = idx as f32 * 1e-5;
        }

        for v in &mut input {
            let t = super::linear_to_pq_generic(*v, 10000.0);
            *v = super::pq_to_linear_generic(t, 1000.0);
        }

        for (idx, v) in input.iter().enumerate() {
            let expected = idx as f32 * 1e-4;
            let diff = (expected - *v).abs();
            assert!(diff < 1e-5);
        }
    }

    #[test]
    fn pq_roundtrip_10k() {
        let mut input = Vec::with_capacity(10000);
        for (idx, v) in input.iter_mut().enumerate() {
            *v = idx as f32 * 1e-5;
        }

        super::linear_to_pq(&mut input, 10000.0);
        super::pq_to_linear(&mut input, 1000.0);

        for (idx, v) in input.iter().enumerate() {
            let expected = idx as f32 * 1e-4;
            let diff = (expected - *v).abs();
            assert!(diff < 1e-5);
        }
    }
}
