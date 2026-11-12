// Derived from opencore-amr 0.1.6 (commit 7dba8c32), amr_nb/common: basic_op_c_equivalent.h,
// basic_op.h, add.cpp, sub.cpp, shr.cpp, shr_r.cpp, mult_r.cpp, l_shr_r.cpp, round.cpp,
// norm_l.cpp, norm_s.cpp, div_s.cpp, div_32.cpp, l_abs.cpp.
// Copyright (C) 1998-2009 PacketVideo, Apache License 2.0; portions (C) 2004 3GPP
// Organizational Partners (3GPP TS 26.073). Ported to Rust for CFA4 (GPL-3.0-or-later).

//! The fixed-point basic operators of the AMR reference code, with opencore's exact behaviour.
//!
//! The C code computes in 32-bit `int` and wraps where it overflows; these functions do the same
//! with explicit wrapping arithmetic. Shift counts are masked to 5 bits as x86 does. The
//! reference's overflow flag never changes the decoder output, so it is not kept.

pub(crate) const MAX_16: i16 = i16::MAX;
pub(crate) const MIN_16: i16 = i16::MIN;
pub(crate) const MAX_32: i32 = i32::MAX;
pub(crate) const MIN_32: i32 = i32::MIN;

/// `x << n` on a 32-bit int, wrapping, with the shift count masked as the x86 `shl` does.
#[inline]
pub(crate) fn shl32(x: i32, n: i32) -> i32 {
    x.wrapping_shl(n as u32 & 31)
}

/// `x >> n` (arithmetic) on a 32-bit int, with the shift count masked.
#[inline]
pub(crate) fn shr32(x: i32, n: i32) -> i32 {
    x.wrapping_shr(n as u32 & 31)
}

#[inline]
pub(crate) fn l_add(a: i32, b: i32) -> i32 {
    let sum = a.wrapping_add(b);
    if (a ^ b) >= 0 && (sum ^ a) < 0 { if a < 0 { MIN_32 } else { MAX_32 } } else { sum }
}

#[inline]
pub(crate) fn l_sub(a: i32, b: i32) -> i32 {
    let diff = a.wrapping_sub(b);
    if (a ^ b) < 0 && (diff ^ a) < 0 { if a < 0 { MIN_32 } else { MAX_32 } } else { diff }
}

#[inline]
pub(crate) fn l_mult(a: i16, b: i16) -> i32 {
    let p = i32::from(a) * i32::from(b);
    if p != 0x4000_0000 { p << 1 } else { MAX_32 }
}

/// opencore's `L_mac`: the overflow test compares the sign of the accumulator with the
/// *undoubled* product and is strict (`> 0`), so some sums wrap instead of saturating.
#[inline]
pub(crate) fn l_mac(acc: i32, a: i16, b: i16) -> i32 {
    let result = i32::from(a) * i32::from(b);
    if result == 0x4000_0000 {
        return MAX_32;
    }
    let sum = (result << 1).wrapping_add(acc);
    if (acc ^ result) > 0 && (sum ^ acc) < 0 { if acc < 0 { MIN_32 } else { MAX_32 } } else { sum }
}

#[inline]
pub(crate) fn l_msu(acc: i32, a: i16, b: i16) -> i32 {
    l_sub(acc, l_mult(a, b))
}

#[inline]
pub(crate) fn mult(a: i16, b: i16) -> i16 {
    let p = (i32::from(a) * i32::from(b)) >> 15;
    if p > 0x7fff { MAX_16 } else { p as i16 }
}

#[inline]
pub(crate) fn add(a: i16, b: i16) -> i16 {
    (i32::from(a) + i32::from(b)).clamp(-32768, 32767) as i16
}

#[inline]
pub(crate) fn sub(a: i16, b: i16) -> i16 {
    (i32::from(a) - i32::from(b)).clamp(-32768, 32767) as i16
}

#[inline]
pub(crate) fn negate(a: i16) -> i16 {
    if a == MIN_16 { MAX_16 } else { -a }
}

#[inline]
pub(crate) fn abs_s(a: i16) -> i16 {
    let y = a.wrapping_sub(i16::from(a < 0));
    y ^ (y >> 15)
}

#[inline]
pub(crate) fn l_abs(a: i32) -> i32 {
    let y = a.wrapping_sub(i32::from(a < 0));
    y ^ (y >> 31)
}

/// opencore's `shl`: a negative count shifts right (0 from 15 on), a positive one saturates.
#[inline]
pub(crate) fn shl(a: i16, n: i16) -> i16 {
    if n < 0 {
        let n = -i32::from(n);
        if n < 15 { a >> n } else { 0 }
    } else {
        let n = i32::from(n);
        let out = shl32(i32::from(a), n) as i16;
        if shr32(i32::from(out), n) != i32::from(a) { (a >> 15) ^ MAX_16 } else { out }
    }
}

/// opencore's `shr`: the count is clamped to ±15; a left shift saturates by the sign of `a`.
#[inline]
pub(crate) fn shr(a: i16, n: i16) -> i16 {
    if n == 0 {
        a
    } else if n > 0 {
        a >> n.min(15)
    } else {
        let n = i32::from((-i32::from(n)).min(15) as i16);
        let out = (i32::from(a) << n) as i16;
        if (i32::from(out) >> n) != i32::from(a) { if a > 0 { MAX_16 } else { MIN_16 } } else { out }
    }
}

#[inline]
pub(crate) fn shr_r(a: i16, n: i16) -> i16 {
    if n > 15 {
        return 0;
    }
    let mut out = shr(a, n);
    if n > 0 && (i32::from(a) & (1 << (n - 1))) != 0 {
        out = out.wrapping_add(1);
    }
    out
}

/// opencore's `L_shl`: a count of 31 or more to the right gives 0, even for negative values.
#[inline]
pub(crate) fn l_shl(a: i32, n: i16) -> i32 {
    if n > 0 {
        let n = i32::from(n);
        let out = shl32(a, n);
        if shr32(out, n) != a { (a >> 31) ^ MAX_32 } else { out }
    } else {
        let n = -i32::from(n);
        if n < 31 { a >> n } else { 0 }
    }
}

/// opencore's `L_shr`: counts of 31 or more give 0; a negative count saturates.
#[inline]
pub(crate) fn l_shr(a: i32, n: i16) -> i32 {
    if n > 0 {
        if n < 31 { a >> n } else { 0 }
    } else {
        let n = -i32::from(n);
        let out = shl32(a, n);
        if shr32(out, n) != a { (a >> 31) ^ MAX_32 } else { out }
    }
}

#[inline]
pub(crate) fn l_shr_r(a: i32, n: i16) -> i32 {
    if n > 31 {
        return 0;
    }
    let mut out = l_shr(a, n);
    if n > 0 && (a & shl32(1, i32::from(n) - 1)) != 0 {
        out = out.wrapping_add(1);
    }
    out
}

/// `pv_round`: saturating add of 0x8000, then the high half.
#[inline]
pub(crate) fn round(a: i32) -> i16 {
    (l_add(a, 0x8000) >> 16) as i16
}

/// opencore's `norm_l`, which normalises |x| (not `~x`) for negative values.
#[inline]
pub(crate) fn norm_l(a: i32) -> i16 {
    if a == 0 {
        return 0;
    }
    let v = l_abs(a);
    v.leading_zeros() as i16 - 1
}

#[inline]
pub(crate) fn norm_s(a: i16) -> i16 {
    if a == 0 {
        return 0;
    }
    let v = abs_s(a);
    (v as u16).leading_zeros() as i16 - 1
}

pub(crate) fn div_s(num: i16, den: i16) -> i16 {
    if num > den || num < 0 {
        return 0;
    }
    if num == 0 {
        return 0;
    }
    if num == den {
        return MAX_16;
    }
    let mut out: i32 = 0;
    let mut n = i32::from(num);
    let d = i32::from(den);
    let d2 = d << 1;
    let d4 = d << 2;
    for _ in 0..5 {
        out <<= 3;
        n <<= 3;
        if n >= d4 {
            n -= d4;
            out |= 4;
        }
        if n >= d2 {
            n -= d2;
            out |= 2;
        }
        if n >= d {
            n -= d;
            out |= 1;
        }
    }
    out as i16
}

pub(crate) fn mpy_32_16(hi: i16, lo: i16, n: i16) -> i32 {
    let mut prod = i32::from(hi) * i32::from(n);
    prod = if prod != 0x4000_0000 { prod << 1 } else { MAX_32 };
    let r = (i32::from(lo) * i32::from(n)) >> 15;
    let sum = prod.wrapping_add(r << 1);
    if (prod ^ r) > 0 && (sum ^ prod) < 0 { if prod < 0 { MIN_32 } else { MAX_32 } } else { sum }
}

/// Reads `t[i]`, or 0 outside the table (the C code would read past it; valid streams never do).
#[inline]
pub(crate) fn at(t: &[i16], i: i32) -> i16 {
    usize::try_from(i).ok().and_then(|i| t.get(i)).copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencore_quirks() {
        // norm_l normalises |x|: -1 and 1 both give 30 (the ETSI operator gives 31 for -1).
        assert_eq!(norm_l(-1), 30);
        assert_eq!(norm_l(1), 30);
        assert_eq!(norm_l(MIN_32), 0);
        assert_eq!(norm_s(-1), 14);
        assert_eq!(norm_s(MIN_16), 0);
        // L_mac with an accumulator equal to the product does not saturate: it wraps.
        assert_eq!(l_mac(1_073_676_289, 32767, 32767), -1_073_938_429);
        assert_eq!(l_mult(MIN_16, MIN_16), MAX_32);
        assert_eq!(l_shr(-5, 31), 0);
        assert_eq!(shl(-5, -15), 0);
        assert_eq!(shr(-5, 20), -1);
        assert_eq!(div_s(1, 2), 16384);
        assert_eq!(abs_s(MIN_16), MAX_16);
        assert_eq!(mult(MIN_16, MIN_16), MAX_16);
        assert_eq!(round(MAX_32), MAX_16);
    }
}
