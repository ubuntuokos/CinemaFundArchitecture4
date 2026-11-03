// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-color 0.11.0.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

#[repr(align(16))]
struct Aligned([u8; 16]);

const SRGB_POWTABLE_UPPER: Aligned = Aligned([0x00, 0x0a, 0x19, 0x26, 0x32, 0x41, 0x4d, 0x5c, 0x68, 0x75, 0x83, 0x8f, 0xa0, 0xaa, 0xb9, 0xc6]);
const SRGB_POWTABLE_LOWER: Aligned = Aligned([0x00, 0xb7, 0x04, 0x0d, 0xcb, 0xe7, 0x41, 0x68, 0x51, 0xd1, 0xeb, 0xf2, 0x00, 0xb7, 0x04, 0x0d]);

/// Converts the linear samples with the sRGB transfer curve.
// Fast linear to sRGB conversion, ported from libjxl.
pub fn linear_to_srgb(samples: &mut [f32]) {
    for s in samples {
        let v = s.to_bits() & 0x7fff_ffff;
        let v_adj = f32::from_bits((v | 0x3e80_0000) & 0x3eff_ffff);
        let pow = 0.059914046f32;
        let pow = pow * v_adj - 0.10889456;
        let pow = pow * v_adj + 0.107963754;
        let pow = pow * v_adj + 0.018092343;

        // `mul` won't be used when `v` is small.
        let idx = (v >> 23).wrapping_sub(118) as usize & 0xf;
        let mul = 0x4000_0000 | (u32::from(SRGB_POWTABLE_UPPER.0[idx]) << 18) | (u32::from(SRGB_POWTABLE_LOWER.0[idx]) << 10);

        let v = f32::from_bits(v);
        let small = v * 12.92;
        let acc = pow * f32::from_bits(mul) - 0.055;

        *s = if v <= 0.0031308 { small } else { acc }.copysign(*s);
    }
}

/// Converts samples in sRGB transfer curve to linear. Inverse of `linear_to_srgb`.
pub fn srgb_to_linear(samples: &mut [f32]) {
    #[allow(clippy::excessive_precision)]
    const P: [f32; 5] = [2.200248328e-4, 1.043637593e-2, 1.624820318e-1, 7.961564959e-1, 8.210152774e-1];
    #[allow(clippy::excessive_precision)]
    const Q: [f32; 5] = [2.631846970e-1, 1.076976492, 4.987528350e-1, -5.512498495e-2, 6.521209011e-3];

    for x in samples {
        let a = x.abs();
        *x = if a <= 0.04045 { a / 12.92 } else { crate::color::fastmath::rational_poly::eval_generic(a, P, Q) }.copysign(*x);
    }
}
