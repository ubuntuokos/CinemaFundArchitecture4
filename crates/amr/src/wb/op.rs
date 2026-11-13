// Derived from opencore-amr 0.1.6 (commit 7dba8c32), amr_wb/dec/src:
// pvamrwbdecoder_basic_op.h, pvamrwbdecoder_basic_op_cequivalent.h, normalize_amr_wb.cpp,
// pvamrwb_math_op.cpp, get_amr_wb_bits.cpp, noise_gen_amrwb.cpp.
// Copyright (C) 1998-2009 PacketVideo, Apache License 2.0; portions (C) 2007 3GPP
// Organizational Partners (3GPP TS 26.173). Ported to Rust for CFA4 (GPL-3.0-or-later).

//! The AMR-WB fixed-point primitives, exactly as opencore's C equivalents compute them
//! (wrapping arithmetic, shift counts masked as on x86).

pub(crate) const MAX_16: i16 = i16::MAX;
pub(crate) const MAX_32: i32 = i32::MAX;
const MIN_32: i32 = i32::MIN;

#[inline]
fn sat16(x: i32) -> i16 {
    if (x >> 15) != (x >> 31) { ((x >> 31) ^ i32::from(MAX_16)) as i16 } else { x as i16 }
}

#[inline]
pub(crate) fn add(a: i16, b: i16) -> i16 {
    sat16(i32::from(a) + i32::from(b))
}

#[inline]
pub(crate) fn sub(a: i16, b: i16) -> i16 {
    sat16(i32::from(a) - i32::from(b))
}

#[inline]
pub(crate) fn mult(a: i16, b: i16) -> i16 {
    sat16((i32::from(a) * i32::from(b)) >> 15)
}

#[inline]
pub(crate) fn mult_r(a: i16, b: i16) -> i16 {
    sat16((i32::from(a) * i32::from(b) + 0x4000) >> 15)
}

#[inline]
pub(crate) fn negate(a: i16) -> i16 {
    if a == i16::MIN { MAX_16 } else { -a }
}

#[inline]
pub(crate) fn l_add(a: i32, b: i32) -> i32 {
    let out = a.wrapping_add(b);
    if ((a ^ b) & MIN_32) == 0 && ((out ^ a) & MIN_32) != 0 { (a >> 31) ^ MAX_32 } else { out }
}

#[inline]
pub(crate) fn l_sub(a: i32, b: i32) -> i32 {
    let out = a.wrapping_sub(b);
    if ((a ^ b) & MIN_32) != 0 && ((out ^ a) & MIN_32) != 0 { (a >> 31) ^ MAX_32 } else { out }
}

#[inline]
pub(crate) fn l_mult(a: i16, b: i16) -> i32 {
    let p = i32::from(a) * i32::from(b);
    if p != 0x4000_0000 { p << 1 } else { MAX_32 }
}

#[inline]
pub(crate) fn l_mac(acc: i32, a: i16, b: i16) -> i32 {
    let m = l_mult(a, b);
    let out = acc.wrapping_add(m);
    if ((m ^ acc) & MIN_32) == 0 && ((out ^ acc) & MIN_32) != 0 { (acc >> 31) ^ MAX_32 } else { out }
}

#[inline]
pub(crate) fn l_msu(acc: i32, a: i16, b: i16) -> i32 {
    let m = l_mult(a, b);
    let out = acc.wrapping_sub(m);
    if ((m ^ acc) & MIN_32) != 0 && ((out ^ acc) & MIN_32) != 0 { (acc >> 31) ^ MAX_32 } else { out }
}

/// `fxp_mac_16by16`: a plain (wrapping) multiply-accumulate.
#[inline]
pub(crate) fn mac(acc: i32, a: i16, b: i16) -> i32 {
    acc.wrapping_add(i32::from(a) * i32::from(b))
}

/// `fxp_mul32_by_16b`.
#[inline]
pub(crate) fn mul32_16b(l: i32, b: i32) -> i32 {
    ((i64::from(l) * i64::from(b.wrapping_shl(16))) >> 32) as i32
}

/// `mul_32by16`.
#[inline]
pub(crate) fn mul_32by16(hi: i16, lo: i16, n: i16) -> i32 {
    let a = i32::from(hi) * i32::from(n);
    let b = (i32::from(lo) * i32::from(n)) >> 15;
    a.wrapping_add(b).wrapping_shl(1)
}

#[inline]
pub(crate) fn round(l: i32) -> i16 {
    let l = if l != MAX_32 { l.wrapping_add(0x8000) } else { l };
    (l >> 16) as i16
}

#[inline]
pub(crate) fn shl1_round(l: i32) -> i16 {
    if (l.wrapping_shl(1) >> 1) == l { ((l + 0x4000) >> 15) as i16 } else { (((l >> 31) ^ MAX_32) >> 16) as i16 }
}

/// `shl_int16`.
#[inline]
pub(crate) fn shl(a: i16, n: i16) -> i16 {
    if n < 0 {
        let n = (-i32::from(n)) & 0xf;
        a >> n
    } else {
        let n = i32::from(n) & 0xf;
        let out = (i32::from(a) << n) as i16;
        if i32::from(out) >> n != i32::from(a) { (a >> 15) ^ MAX_16 } else { out }
    }
}

/// `shl_int32`.
#[inline]
pub(crate) fn l_shl(a: i32, n: i16) -> i32 {
    if n > 0 {
        let s = (n as u32) & 31;
        let out = a.wrapping_shl(s);
        if out >> s != a { (a >> 31) ^ MAX_32 } else { out }
    } else {
        a >> ((-i32::from(n)) & 0xf)
    }
}

/// `shr_int32`.
#[inline]
pub(crate) fn l_shr(a: i32, n: i16) -> i32 {
    if n >= 0 {
        a >> (i32::from(n) & 0x1f)
    } else {
        let s = ((-i32::from(n)) & 0x1f) as u32;
        let out = a.wrapping_shl(s);
        if out >> s != a { (a >> 31) ^ MAX_32 } else { out }
    }
}

/// An `int` shift as compiled for x86 (count masked to 5 bits).
#[inline]
pub(crate) fn shl_int(a: i32, n: i32) -> i32 {
    a.wrapping_shl(n as u32)
}

#[inline]
pub(crate) fn shr_int(a: i32, n: i32) -> i32 {
    a.wrapping_shr(n as u32)
}

pub(crate) fn shr_rnd(a: i16, n: i16) -> i16 {
    let mut out = (i32::from(a) >> (i32::from(n) & 0xf)) as i16;
    if n != 0 && (i32::from(a) & shl_int(1, i32::from(n) - 1)) != 0 {
        out = out.wrapping_add(1);
    }
    out
}

/// `normalize_amr_wb`.
pub(crate) fn norm(x: i32) -> i16 {
    let mut i: i16 = if x > 0x0FFF_FFFF {
        0
    } else if x > 0x00FF_FFFF {
        3
    } else if x > 0x0000_FFFF {
        if x > 0x000F_FFFF { 7 } else { 11 }
    } else if x > 0xFF {
        if x > 0xFFF { 15 } else { 19 }
    } else if x > 0xF {
        23
    } else {
        27
    };
    match x.wrapping_shl(i as u32) & 0x7800_0000 {
        0x0800_0000 => i += 3,
        0x1800_0000 | 0x1000_0000 => i += 2,
        0x2800_0000 | 0x2000_0000 | 0x3800_0000 | 0x3000_0000 => i += 1,
        _ => {}
    }
    i
}

#[inline]
pub(crate) fn norm_s(x: i32) -> i16 {
    norm(x) - 16
}

/// Reads `t[i]`, or 0 outside the table (the C code would read past it; valid streams never do).
#[inline]
pub(crate) fn at(t: &[i16], i: i32) -> i16 {
    usize::try_from(i).ok().and_then(|i| t.get(i)).copied().unwrap_or(0)
}

pub(crate) fn div_16by16(num: i16, den: i16) -> i16 {
    if num > den || num < 0 {
        return 0;
    }
    if num == 0 {
        return 0;
    }
    if num == den {
        return MAX_16;
    }
    let mut out: i16 = 0;
    let mut n = i32::from(num);
    let d = i32::from(den);
    let (d2, d4) = (d << 1, d << 2);
    for _ in 0..5 {
        out = out.wrapping_shl(3);
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
    out
}

const TABLE_ISQRT: [i16; 49] = [
    32767, 31790, 30894, 30070, 29309, 28602, 27945, 27330, 26755, 26214, 25705, 25225, 24770, 24339, 23930, 23541, 23170, 22817, 22479, 22155, 21845, 21548,
    21263, 20988, 20724, 20470, 20225, 19988, 19760, 19539, 19326, 19119, 18919, 18725, 18536, 18354, 18176, 18004, 17837, 17674, 17515, 17361, 17211, 17064,
    16921, 16782, 16646, 16514, 16384,
];

pub(crate) fn one_ov_sqrt_norm(frac: &mut i32, exp: &mut i16) {
    if *frac <= 0 {
        *exp = 0;
        *frac = MAX_32;
        return;
    }
    if (*exp & 1) == 1 {
        *frac >>= 1;
    }
    *exp = negate(((i32::from(*exp) - 1) >> 1) as i16);
    *frac >>= 9;
    let i = i32::from((*frac >> 16) as i16) - 16;
    *frac >>= 1;
    let a = (*frac as i16) & 0x7fff;
    let t = at(&TABLE_ISQRT, i);
    *frac = i32::from(t) << 16;
    let tmp = t.wrapping_sub(at(&TABLE_ISQRT, i + 1));
    *frac = l_msu(*frac, tmp, a);
}

pub(crate) fn one_ov_sqrt(x: i32) -> i32 {
    let e = norm(x);
    let mut x = x.wrapping_shl(e as u32);
    let mut e = 31 - e;
    one_ov_sqrt_norm(&mut x, &mut e);
    l_shl(x, e)
}

const TABLE_POW2: [i16; 33] = [
    16384, 16743, 17109, 17484, 17867, 18258, 18658, 19066, 19484, 19911, 20347, 20792, 21247, 21713, 22188, 22674, 23170, 23678, 24196, 24726, 25268, 25821,
    26386, 26964, 27554, 28158, 28774, 29405, 30048, 30706, 31379, 32066, 32767,
];

pub(crate) fn power_of_2(exponent: i16, fraction: i16) -> i32 {
    let lx = i32::from(fraction) << 5;
    let i = i32::from(fraction >> 10);
    let a = (lx as i16) & 0x7fff;
    let t = at(&TABLE_POW2, i);
    let mut lx = i32::from(t) << 15;
    let tmp = t.wrapping_sub(at(&TABLE_POW2, i + 1));
    lx = lx.wrapping_sub(i32::from(tmp) * i32::from(a));
    let e = 29 - i32::from(exponent);
    if e != 0 {
        lx = shr_int(lx, e).wrapping_add(shr_int(lx, e - 1) & 1);
    }
    lx
}

/// `Dot_product12`: the normalised energy-style dot product of `x` and `y`.
pub(crate) fn dot_product12(x: &[i16], y: &[i16], exp: &mut i16) -> i32 {
    let mut sum: i32 = 1;
    for (&a, &b) in x.iter().zip(y) {
        sum = l_mac(sum, a, b);
    }
    let s = norm(sum);
    *exp = 30 - s;
    sum.wrapping_shl(s as u32)
}

const LOG2_NORM_TABLE: [i16; 33] = [
    0, 1455, 2866, 4236, 5568, 6863, 8124, 9352, 10549, 11716, 12855, 13967, 15054, 16117, 17156, 18172, 19167, 20142, 21097, 22033, 22951, 23852, 24735,
    25603, 26455, 27291, 28113, 28922, 29716, 30497, 31266, 32023, 32767,
];

fn lg2_normalized(x: i32, exp: i16) -> (i16, i16) {
    if x <= 0 {
        return (0, 0);
    }
    let exponent = 30 - exp;
    let x = x >> 9;
    let i = i32::from((x >> 16) as i16) - 32;
    let x = x >> 1;
    let a = (x as i16) & 0x7fff;
    let t = at(&LOG2_NORM_TABLE, i);
    let y = i32::from(t) << 16;
    let tmp = t.wrapping_sub(at(&LOG2_NORM_TABLE, i + 1));
    let y = l_msu(y, tmp, a);
    (exponent, (y >> 16) as i16)
}

/// `amrwb_log_2`: (exponent, fraction).
pub(crate) fn log2(x: i32) -> (i16, i16) {
    let e = norm(x);
    lg2_normalized(l_shl(x, e), e)
}

/// `int32_to_dpf`: (hi, lo).
pub(crate) fn to_dpf(l: i32) -> (i16, i16) {
    let hi = (l >> 16) as i16;
    let lo = (l.wrapping_sub(i32::from(hi) << 16) >> 1) as i16;
    (hi, lo)
}

pub(crate) fn mpy_dpf_32(hi1: i16, lo1: i16, hi2: i16, lo2: i16) -> i32 {
    let l = l_mult(hi1, hi2);
    let l = l_mac(l, mult(hi1, lo2), 1);
    l_mac(l, mult(lo1, hi2), 1)
}

/// `noise_gen_amrwb`.
#[inline]
pub(crate) fn noise(seed: &mut i16) -> i16 {
    *seed = (i32::from(*seed) * 31821 + 13849) as i16;
    *seed
}

/// The decoder's unpacked parameter bits (`BIT_1` = 127, others 0) and a read position.
pub(crate) struct Bits<'a> {
    pub(crate) bits: &'a [i16],
    pub(crate) pos: usize,
}

pub(crate) const BIT_1: i16 = 127;

impl Bits<'_> {
    fn bit(&mut self) -> i16 {
        let b = self.bits.get(self.pos).copied().unwrap_or(0);
        self.pos += 1;
        i16::from(b == BIT_1)
    }

    /// `Serial_parm`.
    pub(crate) fn parm(&mut self, n: i16) -> i16 {
        let mut v: i16 = 0;
        for _ in 0..n {
            v = v.wrapping_shl(1) | self.bit();
        }
        v
    }

    /// `Serial_parm_1bit`.
    pub(crate) fn bit1(&mut self) -> i16 {
        self.bit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_matches_opencore() {
        assert_eq!(norm(0x4000_0000), 0);
        assert_eq!(norm(1), 30);
        assert_eq!(norm(0), 27);
        assert_eq!(norm_s(0x4000), 0);
        assert_eq!(round(MAX_32), MAX_16);
        assert_eq!(shl(0x4000, 1), MAX_16);
    }
}
