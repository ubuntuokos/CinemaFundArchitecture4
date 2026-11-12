// Derived from opencore-amr 0.1.6 (commit 7dba8c32), amr_nb/common/src: log2.cpp,
// log2_norm.cpp, pow2.cpp, sqrt_l.cpp, inv_sqrt.cpp.
// Copyright (C) 1998-2009 PacketVideo, Apache License 2.0; portions (C) 2004 3GPP
// Organizational Partners (3GPP TS 26.073). Ported to Rust for CFA4 (GPL-3.0-or-later).

//! Table-driven logarithm, power, and square-root approximations.

use super::tables::{INV_SQRT_TBL, LOG2_TBL, POW2_TBL, SQRT_L_TBL};
use crate::basic::{at, l_msu, l_mult, l_shl, l_shr_r, norm_l, shl32};

pub(crate) fn log2_norm(x: i32, exp: i16) -> (i16, i16) {
    if x <= 0 {
        return (0, 0);
    }
    let x = x >> 10;
    let i = i32::from((x >> 15) as i16) - 32;
    let a = x & 0x7fff;
    let mut y = i32::from(at(&LOG2_TBL, i)) << 16;
    let tmp = at(&LOG2_TBL, i).wrapping_sub(at(&LOG2_TBL, i + 1));
    y = y.wrapping_sub((i32::from(tmp) * a) << 1);
    (30 - exp, (y >> 16) as i16)
}

pub(crate) fn log2(x: i32) -> (i16, i16) {
    let exp = norm_l(x);
    log2_norm(shl32(x, i32::from(exp)), exp)
}

pub(crate) fn pow2(exponent: i16, fraction: i16) -> i32 {
    let x = l_mult(fraction, 32);
    let i = i32::from((x >> 16) as i16 & 31);
    let a = ((x >> 1) & 0x7fff) as i16;
    let mut x = i32::from(at(&POW2_TBL, i)) << 16;
    let tmp = at(&POW2_TBL, i).wrapping_sub(at(&POW2_TBL, i + 1));
    x = l_msu(x, tmp, a);
    l_shr_r(x, 30i16.wrapping_sub(exponent))
}

/// `sqrt_l_exp`: returns the root and the (even) normalisation shift.
pub(crate) fn sqrt_l_exp(x: i32) -> (i32, i16) {
    if x <= 0 {
        return (0, 0);
    }
    let e = norm_l(x) & !1;
    let x = l_shl(x, e) >> 10;
    let mut i = i32::from((x >> 15) as i16 & 63);
    let a = (x as i16) & 0x7fff;
    if i > 15 {
        i -= 16;
    }
    let y = i32::from(at(&SQRT_L_TBL, i)) << 16;
    let tmp = at(&SQRT_L_TBL, i).wrapping_sub(at(&SQRT_L_TBL, i + 1));
    (l_msu(y, tmp, a), e)
}

pub(crate) fn inv_sqrt(x: i32) -> i32 {
    if x <= 0 {
        return 0x3fff_ffff;
    }
    let e = norm_l(x);
    let mut x = shl32(x, i32::from(e));
    let mut exp = 30 - e;
    if exp & 1 == 0 {
        x >>= 1;
    }
    exp = (exp >> 1) + 1;
    x >>= 9;
    let i = i32::from((x >> 16) as i16) - 16;
    let a = ((x >> 1) as i16) & 0x7fff;
    let mut y = i32::from(at(&INV_SQRT_TBL, i)) << 16;
    let tmp = at(&INV_SQRT_TBL, i).wrapping_sub(at(&INV_SQRT_TBL, i + 1));
    y = y.wrapping_sub((i32::from(tmp) * i32::from(a)) << 1);
    y >> exp
}
