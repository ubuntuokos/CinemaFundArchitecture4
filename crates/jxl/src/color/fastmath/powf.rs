// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-color 0.11.0.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

const POW2F_NUMER_COEFFS: [f32; 3] = [1.017_490_7e1, 4.886_878e1, 9.855_066e1];
const POW2F_DENOM_COEFFS: [f32; 4] = [2.102_429_6e-1, -2.223_288_6e-2, -1.944_15e1, 9.855_067e1];

#[inline]
fn fast_pow2f_generic(x: f32) -> f32 {
    let x_floor = x.floor();
    let exp = f32::from_bits(((x_floor as i32 + 127) as u32) << 23);
    let frac = x - x_floor;

    let num = frac + POW2F_NUMER_COEFFS[0];
    let num = num * frac + POW2F_NUMER_COEFFS[1];
    let num = num * frac + POW2F_NUMER_COEFFS[2];
    let num = num * exp;

    let den = POW2F_DENOM_COEFFS[0] * frac + POW2F_DENOM_COEFFS[1];
    let den = den * frac + POW2F_DENOM_COEFFS[2];
    let den = den * frac + POW2F_DENOM_COEFFS[3];

    num / den
}

const LOG2F_P: [f32; 3] = [-1.850_383_3e-6, 1.428_716_1, 7.424_587_6e-1];
const LOG2F_Q: [f32; 3] = [9.903_281_3e-1, 1.009_671_8, 1.740_934_3e-1];

#[inline]
fn fast_log2f_generic(x: f32) -> f32 {
    let x_bits = x.to_bits() as i32;
    let exp_bits = x_bits - 0x3f2aaaab;
    let exp_shifted = exp_bits >> 23;
    let mantissa = f32::from_bits((x_bits - (exp_shifted << 23)) as u32);
    let exp_val = exp_shifted as f32;

    let x = mantissa - 1.0;
    super::rational_poly::eval_generic(x, LOG2F_P, LOG2F_Q) + exp_val
}

#[inline]
pub fn fast_powf_generic(base: f32, exp: f32) -> f32 {
    fast_pow2f_generic(fast_log2f_generic(base) * exp)
}
