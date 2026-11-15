// Ported from bcg729 1.1.2 src/fixedPointMacros.h, src/g729FixedPointMath.h and src/utils.h,
// GPL-3.0-or-later, Copyright (c) 2011-2019 Belledonne Communications SARL.

//! bcg729's fixed-point macros and its log2 / exp2 / sqrt / cos approximations.
//!
//! C's `int` arithmetic is modelled with wrapping `i32` operations (bcg729 relies on two's
//! complement), and every store to a `word16_t` is an `as i16` truncation at the call site.

pub(crate) const MAXINT16: i32 = 0x7fff;
pub(crate) const MAXINT29: i32 = 0x0fff_ffff;

/// `MULT16_16`: a 32-bit product.
#[inline]
pub(crate) fn mul(a: i32, b: i32) -> i32 {
    a.wrapping_mul(b)
}

/// `MAC16_16`.
#[inline]
pub(crate) fn mac(c: i32, a: i32, b: i32) -> i32 {
    c.wrapping_add(mul(a, b))
}

/// `MSU16_16`.
#[inline]
pub(crate) fn msu(c: i32, a: i32, b: i32) -> i32 {
    c.wrapping_sub(mul(a, b))
}

/// `ADD16`: both terms and the sum truncated to 16 bits.
#[inline]
pub(crate) fn add16(a: i32, b: i32) -> i16 {
    (a as i16).wrapping_add(b as i16)
}

/// `SUB16`: both terms truncated to 16 bits, the difference kept in an `int`.
#[inline]
pub(crate) fn sub16(a: i32, b: i32) -> i32 {
    i32::from(a as i16) - i32::from(b as i16)
}

/// `SHR`/`SHL` with the shift capped below the type width (the C shifts never exceed it).
#[inline]
pub(crate) fn shr(a: i32, s: u32) -> i32 {
    a >> s.min(31)
}

#[inline]
pub(crate) fn shl(a: i32, s: u32) -> i32 {
    a.wrapping_shl(s.min(31))
}

/// `PSHR`: shift right with rounding.
#[inline]
pub(crate) fn pshr(a: i32, s: u32) -> i32 {
    let s = s.min(31);
    a.wrapping_add((1i32 << s) >> 1) >> s
}

/// `SSHL`: a left shift of the magnitude that keeps the sign.
#[inline]
pub(crate) fn sshl(a: i32, s: u32) -> i32 {
    if a < 0 { a.wrapping_neg().wrapping_shl(s.min(31)).wrapping_neg() } else { a.wrapping_shl(s.min(31)) }
}

/// `VSHR32`: a right shift by `s`, or a left shift by `-s`.
#[inline]
pub(crate) fn vshr(a: i32, s: i32) -> i32 {
    if s > 0 { shr(a, s.unsigned_abs()) } else { shl(a, s.unsigned_abs()) }
}

/// `VSHR32` on an unsigned value.
#[inline]
pub(crate) fn uvshr(a: u32, s: i32) -> u32 {
    if s > 0 { a >> s.unsigned_abs().min(31) } else { a.wrapping_shl(s.unsigned_abs().min(31)) }
}

/// `SVSHR32`.
#[inline]
pub(crate) fn svshr(a: i32, s: i32) -> i32 {
    if s > 0 { shr(a, s.unsigned_abs()) } else { sshl(a, s.unsigned_abs()) }
}

/// `SATURATE(x, a)`: clamps to `[-(a + 1), a]`.
#[inline]
pub(crate) fn sat(x: i32, a: i32) -> i32 {
    x.clamp(-a - 1, a)
}

/// `SATURATE(x, MAXINT16)` stored to 16 bits.
#[inline]
pub(crate) fn sat16(x: i32) -> i16 {
    sat(x, MAXINT16) as i16
}

/// `MULT16_16_Qn`.
#[inline]
pub(crate) fn mul_q(a: i32, b: i32, n: u32) -> i32 {
    shr(mul(a, b), n)
}

/// `MULT16_16_Pn`: rounded.
#[inline]
pub(crate) fn mul_p(a: i32, b: i32, n: u32) -> i32 {
    mul(a, b).wrapping_add(1 << (n - 1)) >> n
}

/// `MULT16_32_Qn`.
#[inline]
pub(crate) fn mul32_q(a: i32, b: i32, n: u32) -> i32 {
    mul(a, shr(b, n)).wrapping_add(shr(mul(a, b & ((1 << n) - 1)), n))
}

/// `MULT16_32_Pn`.
#[inline]
pub(crate) fn mul32_p(a: i32, b: i32, n: u32) -> i32 {
    mul(a, shr(b, n)).wrapping_add(pshr(mul(a, b & ((1 << n) - 1)), n))
}

/// `countLeadingZeros`: the leading zeros after the sign bit (31 for 0).
#[inline]
pub(crate) fn clz(x: i32) -> u32 {
    if x <= 0 { 31 } else { x.leading_zeros() - 1 }
}

/// `g729Log2_Q0Q16`.
pub(crate) fn log2_q0q16(x: i32) -> i32 {
    const KL0: i32 = -135003;
    const KL1: i32 = 378194;
    const KL2: i32 = -252129;
    const KL3: i32 = 74705;
    let lz = clz(x) as i32;
    let ret = shl(sub16(30, lz), 16);
    let acc = i32::from(vshr(x, 16 - lz) as i16);
    let acc32 = KL0.wrapping_add(mul32_q(acc, KL1.wrapping_add(mul32_q(acc, KL2.wrapping_add(mul32_q(acc, KL3, 15)), 15)), 15));
    ret.wrapping_add(acc32)
}

/// `g729Exp2_Q11Q16`.
pub(crate) fn exp2_q11q16(x: i16) -> i32 {
    const E0: i32 = 16384;
    const E1: i32 = 11356;
    const E2: i32 = 3726;
    const E3: i32 = 1301;
    let integer = i32::from(x) >> 11;
    if integer > 14 {
        return 0x7fff_ffff;
    }
    if integer < -15 {
        return 0;
    }
    let frac = i32::from(((i32::from(x) - (integer << 11)) << 3) as i16);
    let frac = add16(E0, mul_q(frac, i32::from(add16(E1, mul_q(frac, i32::from(add16(E2, mul_q(E3, frac, 14))), 14))), 14));
    vshr(i32::from(frac), -integer - 2)
}

/// `g729Sqrt_Q0Q7`.
pub(crate) fn sqrt_q0q7(x: u32) -> i32 {
    const C0: i32 = 3634;
    const C1: i32 = 21173;
    const C2: i32 = -12627;
    const C3: i32 = 4204;
    if x == 0 {
        return 0;
    }
    let k = (19 - x.leading_zeros() as i32) >> 1;
    let x = uvshr(x, k * 2) as i32;
    let rt = add16(C0, mul_q(x, i32::from(add16(C1, mul_q(x, i32::from(add16(C2, mul_q(x, C3, 14))), 14))), 14));
    vshr(i32::from(rt), -k)
}

/// `g729InvSqrt_Q0Q31` (`x` at least 1; anything lower gives the maximum).
pub(crate) fn inv_sqrt_q0q31(x: i32) -> i32 {
    if x <= 1 {
        return i32::MAX;
    }
    ((i64::from(sqrt_q0q7(x as u32)) << 24) / i64::from(x)) as i32
}

/// `g729Cos_Q13Q15`: cosine of `x` in Q13 (0…π) in Q15.
pub(crate) fn cos_q13q15(x: i16) -> i16 {
    const KCOS1: i32 = 32768;
    const KCOS2: i32 = -16384;
    const KCOS3: i32 = 1365;
    const KCOS4: i32 = -46;
    const KSIN1: i32 = 32768;
    const KSIN2: i32 = -5461;
    const KSIN3: i32 = 273;
    const KSIN4: i32 = -7;
    let x = i32::from(x);
    let sin_poly = |x2: i32| KSIN1.wrapping_add(mul_p(x2, KSIN2.wrapping_add(mul_p(x2, KSIN3.wrapping_add(mul_p(KSIN4, x2, 15)), 15)), 15));
    let cos_poly = |x2: i32| mul_p(x2, KCOS2.wrapping_add(mul_p(x2, KCOS3.wrapping_add(mul_p(KCOS4, x2, 15)), 15)), 15);
    if x < 12868 {
        if x < 6434 {
            let x2 = i32::from(mul_p(x, x, 11) as i16);
            sat16(KCOS1.wrapping_add(cos_poly(x2)))
        } else {
            let x = sub16(12868, x);
            let x2 = i32::from(mul_p(x, x, 11) as i16);
            mul_p(x, sin_poly(x2), 13) as i16
        }
    } else {
        let xs = sub16(25736, x);
        if x < 19302 {
            let xs = sub16(12868, xs);
            let x2 = i32::from(mul_p(xs, xs, 11) as i16);
            mul_p(-xs, sin_poly(x2), 13) as i16
        } else {
            let x2 = i32::from(mul_p(xs, xs, 11) as i16);
            (-KCOS1).wrapping_sub(cos_poly(x2)) as i16
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approximations_track_the_functions() {
        for v in [1i32, 7, 1000, 1 << 20, i32::MAX] {
            let l = f64::from(log2_q0q16(v)) / 65536.0;
            assert!((l - f64::from(v).log2()).abs() < 0.01, "log2 {v}: {l}");
        }
        for v in [-20000i16, -2048, 0, 2048, 20000] {
            let e = f64::from(exp2_q11q16(v)) / 65536.0;
            let want = 2f64.powf(f64::from(v) / 2048.0);
            assert!((e - want).abs() <= want * 0.01 + 1.0 / 32768.0, "exp2 {v}: {e} {want}");
        }
        for v in [1u32, 2, 100, 65536, u32::MAX] {
            let s = f64::from(sqrt_q0q7(v)) / 128.0;
            assert!((s - f64::from(v).sqrt()).abs() <= f64::from(v).sqrt() * 0.01 + 0.01, "sqrt {v}: {s}");
        }
        for v in (0..25736).step_by(997) {
            let c = f64::from(cos_q13q15(v as i16)) / 32768.0;
            assert!((c - (f64::from(v) / 8192.0).cos()).abs() < 0.001, "cos {v}: {c}");
        }
    }
}
