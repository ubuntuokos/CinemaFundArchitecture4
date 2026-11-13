// Derived from opencore-amr 0.1.6 (commit 7dba8c32), amr_wb/dec/src: isp_isf.cpp, isp_az.cpp,
// interpolate_isp.cpp, weight_amrwb_lpc.cpp, wb_syn_filt.cpp, pred_lt4.cpp, deemphasis_32.cpp,
// highpass_50hz_at_12k8.cpp, highpass_400hz_at_12k8.cpp, oversamp_12k8_to_16k.cpp,
// band_pass_6k_7k.cpp, low_pass_filt_7k.cpp, isf_extrapolation.cpp, agc2_amr_wb.cpp,
// voice_factor.cpp, phase_dispersion.cpp, scale_signal.cpp, preemph_amrwb_dec.cpp, pit_shrp.cpp.
// Copyright (C) 1998-2009 PacketVideo, Apache License 2.0; portions (C) 2007 3GPP
// Organizational Partners (3GPP TS 26.173). Ported to Rust for CFA4 (GPL-3.0-or-later).

//! AMR-WB signal processing: ISP/LPC conversions, synthesis filters, resampling and the
//! high-band filters. Sums that opencore unrolls in groups are computed one output at a time;
//! the arithmetic wraps, so the order does not change the result.

use super::op::{
    add, at, div_16by16, dot_product12, l_add, l_mac, l_msu, l_mult, l_shl, l_shr, l_sub, mac, mpy_dpf_32, mul32_16b, mult, mult_r, negate, norm, norm_s,
    one_ov_sqrt, round, shl, shl_int, shl1_round, shr_int, shr_rnd, sub, to_dpf,
};
use super::{L_SUBFR, L_SUBFR16K, M, M16K};

const ISP_TABLE: [i16; 129] = [
    32767, 32758, 32729, 32679, 32610, 32522, 32413, 32286, 32138, 31972, 31786, 31581, 31357, 31114, 30853, 30572, 30274, 29957, 29622, 29269, 28899, 28511,
    28106, 27684, 27246, 26791, 26320, 25833, 25330, 24812, 24279, 23732, 23170, 22595, 22006, 21403, 20788, 20160, 19520, 18868, 18205, 17531, 16846, 16151,
    15447, 14733, 14010, 13279, 12540, 11793, 11039, 10279, 9512, 8740, 7962, 7180, 6393, 5602, 4808, 4011, 3212, 2411, 1608, 804, 0, -804, -1608, -2411,
    -3212, -4011, -4808, -5602, -6393, -7180, -7962, -8740, -9512, -10279, -11039, -11793, -12540, -13279, -14010, -14733, -15447, -16151, -16846, -17531,
    -18205, -18868, -19520, -20160, -20788, -21403, -22006, -22595, -23170, -23732, -24279, -24812, -25330, -25833, -26320, -26791, -27246, -27684, -28106,
    -28511, -28899, -29269, -29622, -29957, -30274, -30572, -30853, -31114, -31357, -31581, -31786, -31972, -32138, -32286, -32413, -32522, -32610, -32679,
    -32729, -32758, -32768,
];

/// `Isf_isp`, in place (`v` holds the ISFs on entry and the ISPs on return).
pub(crate) fn isf_isp(v: &mut [i16]) {
    if let Some(last) = v.last_mut() {
        *last = shl(*last, 1);
    }
    for x in v.iter_mut() {
        let ind = i32::from(*x >> 7);
        let offset = *x & 0x7f;
        let t0 = at(&ISP_TABLE, ind);
        let diff = at(&ISP_TABLE, ind + 1).wrapping_sub(t0);
        let l = l_mult(diff, offset);
        *x = add(t0, (l >> 8) as i16);
    }
}

/// `Get_isp_pol` / `Get_isp_pol_16kHz` over every other ISP from `isp[start]`.
fn get_isp_pol(isp: &[i16], start: usize, f: &mut [i32], n: usize, wide: bool) {
    let sh = if wide { 7 } else { 9 };
    let x = |k: usize| i32::from(isp.get(start + 2 * k).copied().unwrap_or(0));
    if let Some(v) = f.get_mut(0) {
        *v = if wide { 0x0020_0000 } else { 0x0080_0000 };
    }
    if let Some(v) = f.get_mut(1) {
        *v = (-x(0)).wrapping_shl(sh);
    }
    for i in 2..=n {
        let xi = x(i - 1);
        let xs = xi as i16;
        let prev2 = f.get(i - 2).copied().unwrap_or(0);
        if let Some(v) = f.get_mut(i) {
            *v = prev2;
        }
        let mut p = i;
        for _ in 1..i {
            let fm1 = f.get(p - 1).copied().unwrap_or(0);
            let fm2 = f.get(p - 2).copied().unwrap_or(0);
            let t0 = l_shl(mul32_16b(fm1, i32::from(xs)), 2);
            if let Some(v) = f.get_mut(p) {
                *v = v.wrapping_sub(t0).wrapping_add(fm2);
            }
            p -= 1;
        }
        if let Some(v) = f.get_mut(p) {
            *v = v.wrapping_sub(xi.wrapping_shl(sh));
        }
    }
}

/// `Isp_Az`: ISPs to LPC coefficients `a[0..=m]`.
pub(crate) fn isp_az(isp: &[i16], a: &mut [i16], m: usize, adaptive_scaling: bool) {
    let nc = m >> 1;
    let mut f1 = [0i32; M16K / 2 + 1];
    let mut f2 = [0i32; M16K / 2];
    if nc > 8 {
        get_isp_pol(isp, 0, &mut f1, nc, true);
        for v in f1.iter_mut().take(nc + 1) {
            *v = l_shl(*v, 2);
        }
        get_isp_pol(isp, 1, &mut f2, nc - 1, true);
        for v in f2.iter_mut().take(nc) {
            *v = l_shl(*v, 2);
        }
    } else {
        get_isp_pol(isp, 0, &mut f1, nc, false);
        get_isp_pol(isp, 1, &mut f2, nc - 1, false);
    }
    for i in (2..nc).rev() {
        f2[i] = f2[i].wrapping_sub(f2[i - 2]);
    }
    let last = isp.get(m - 1).copied().unwrap_or(0);
    for i in 0..nc {
        let t0 = mul32_16b(f1[i], i32::from(last)).wrapping_shl(1);
        let t1 = mul32_16b(f2[i], i32::from(last)).wrapping_shl(1);
        f1[i] = f1[i].wrapping_add(t0);
        f2[i] = f2[i].wrapping_sub(t1);
    }
    let mut out = [0i16; M16K + 1];
    out[0] = 4096;
    let mut tmax: i32 = 1;
    let rnd = |t: i32, q: i32| (shr_int(t, q).wrapping_add(shr_int(t, q - 1) & 1)) as i16;
    let absish = |t: i32| {
        let t1 = t - i32::from(t < 0);
        t1 ^ (t1 >> 31)
    };
    let mut j = m - 1;
    for i in 1..nc {
        let t0 = l_add(f1[i], f2[i]);
        tmax |= absish(t0);
        out[i] = rnd(t0, 12);
        let t0 = l_sub(f1[i], f2[i]);
        tmax |= absish(t0);
        out[j] = rnd(t0, 12);
        j -= 1;
    }
    let mut q: i32 = if adaptive_scaling { 4 - i32::from(norm(tmax)) } else { 0 };
    let q_sug;
    if q > 0 {
        q_sug = 12 + q;
        let mut j = m - 1;
        for i in 1..nc {
            out[i] = rnd(l_add(f1[i], f2[i]), q_sug);
            out[j] = rnd(l_sub(f1[i], f2[i]), q_sug);
            j -= 1;
        }
        out[0] >>= q;
    } else {
        q_sug = 12;
        q = 0;
    }
    let t0 = (((i64::from(f1[nc]) * i64::from(last)) >> 16) as i32).wrapping_shl(1);
    let t0 = l_add(f1[nc], t0);
    out[nc] = rnd(t0, q_sug);
    out[m] = shr_rnd(last, (3 + q) as i16);
    for (d, s) in a.iter_mut().zip(out.iter().take(m + 1)) {
        *d = *s;
    }
}

/// `interpolate_isp`: the four subframe LPC sets from the old and new ISPs.
pub(crate) fn interpolate_isp(isp_old: &[i16; M], isp_new: &[i16; M], frac: &[i16; 4], az: &mut [[i16; M + 1]; 4]) {
    for k in 0..3 {
        let fac_new = frac[k];
        let fac_old = add(sub(32767, fac_new), 1);
        let mut isp = [0i16; M];
        for i in 0..M {
            let l = l_mult(isp_old[i], fac_old);
            isp[i] = round(l_mac(l, isp_new[i], fac_new));
        }
        isp_az(&isp, &mut az[k], M, false);
    }
    isp_az(isp_new, &mut az[3], M, false);
}

/// `weight_amrwb_lpc`.
pub(crate) fn weight_lpc(a: &[i16], ap: &mut [i16], gamma: i16, m: usize) {
    let r = |x: i16, y: i16| ((i32::from(x) * i32::from(y) + 0x4000) >> 15) as i16;
    if let (Some(d), Some(&s)) = (ap.first_mut(), a.first()) {
        *d = s;
    }
    let mut fac = gamma;
    for i in 1..=m {
        let ai = a.get(i).copied().unwrap_or(0);
        if let Some(d) = ap.get_mut(i) {
            *d = r(ai, fac);
        }
        fac = r(fac, gamma);
    }
}

/// `Syn_filt_32`: the 32-bit (hi/lo) synthesis filter. `hi`/`lo` hold `M` samples of history
/// followed by the `L_SUBFR` outputs.
pub(crate) fn syn_filt_32(a: &[i16; M + 1], exc: &[i16], q_new: i16, hi: &mut [i16; M + L_SUBFR], lo: &mut [i16; M + L_SUBFR]) {
    let a0 = 9 - i32::from(q_new);
    for n in 0..L_SUBFR {
        let p = n + M;
        let mut lo_sum: i32 = 0;
        let mut hi_sum: i32 = 0;
        for k in 1..=M {
            lo_sum = mac(lo_sum, lo[p - k], a[k]);
            hi_sum = mac(hi_sum, hi[p - k], a[k]);
        }
        let x = i32::from(exc.get(n).copied().unwrap_or(0));
        let mut l = lo_sum.wrapping_neg() >> 11;
        l = l.wrapping_add(shl_int(x, a0));
        l = l.wrapping_sub(hi_sum.wrapping_shl(1));
        l = l_shl(l, 3);
        hi[p] = (l >> 16) as i16;
        lo[p] = ((l >> 4).wrapping_sub((l >> 16) << 12)) as i16;
    }
}

/// `wb_syn_filt` with `update` set: filters `x` in place through `1/A(z)` of order `m`.
pub(crate) fn wb_syn_filt(a: &[i16], m: usize, x: &mut [i16], mem: &mut [i16]) {
    let lg = x.len();
    let mut yy = [0i16; M16K + L_SUBFR16K];
    for (d, s) in yy.iter_mut().zip(mem.iter().take(m)) {
        *d = *s;
    }
    for n in 0..lg.min(L_SUBFR16K) {
        let p = n + m;
        let mut l = (-i32::from(x[n])) << 11;
        for k in 1..=m {
            l = mac(l, yy.get(p - k).copied().unwrap_or(0), a.get(k).copied().unwrap_or(0));
        }
        let l = l_shl(l, 4);
        let y = round(l.wrapping_neg());
        if let Some(d) = yy.get_mut(p) {
            *d = y;
        }
        x[n] = y;
    }
    if lg >= m {
        for (d, s) in mem.iter_mut().zip(&x[lg - m..]) {
            *d = *s;
        }
    }
}

const INTER4_2: [[i16; 32]; 4] = [
    [
        0, -2, 4, -2, -10, 38, -88, 165, -275, 424, -619, 871, -1207, 1699, -2598, 5531, 14031, -2147, 780, -249, -16, 153, -213, 226, -209, 175, -133, 91,
        -55, 28, -10, 2,
    ],
    [
        1, -7, 19, -33, 47, -52, 43, -9, -60, 175, -355, 626, -1044, 1749, -3267, 10359, 10359, -3267, 1749, -1044, 626, -355, 175, -60, -9, 43, -52, 47, -33,
        19, -7, 1,
    ],
    [
        2, -10, 28, -55, 91, -133, 175, -209, 226, -213, 153, -16, -249, 780, -2147, 14031, 5531, -2598, 1699, -1207, 871, -619, 424, -275, 165, -88, 38, -10,
        -2, 4, -2, 0,
    ],
    [
        1, -7, 22, -49, 92, -153, 231, -325, 431, -544, 656, -762, 853, -923, 968, 15401, 968, -923, 853, -762, 656, -544, 431, -325, 231, -153, 92, -49, 22,
        -7, 1, 0,
    ],
];

/// `Pred_lt4`: the adaptive-codebook excitation `exc[pos..pos+len]` at lag `t0` + `frac`/4.
pub(crate) fn pred_lt4(exc: &mut [i16], pos: usize, t0: i16, frac: i16, len: usize) {
    let mut start = pos as i32 - i32::from(t0);
    let mut frac = -i32::from(frac);
    if frac < 0 {
        frac += 4;
        start -= 1;
    }
    start -= 15;
    let Some(table) = usize::try_from(3 - frac).ok().and_then(|i| INTER4_2.get(i)) else {
        return;
    };
    for n in 0..len {
        let mut sum: i32 = 0x2000;
        for (k, &c) in table.iter().enumerate() {
            let idx = start + (n + k) as i32;
            let v = usize::try_from(idx).ok().and_then(|i| exc.get(i)).copied().unwrap_or(0);
            sum = mac(sum, v, c);
        }
        if let Some(d) = exc.get_mut(pos + n) {
            *d = (sum >> 14) as i16;
        }
    }
}

/// `deemphasis_32`.
pub(crate) fn deemphasis_32(x_hi: &[i16], x_lo: &[i16], y: &mut [i16; L_SUBFR], mu: i16, mem: &mut i16) {
    let mut prev = *mem;
    for i in 0..L_SUBFR {
        let mut l = (i32::from(x_hi[i]) << 16).wrapping_add(i32::from(x_lo[i]) << 4);
        l = l_shl(l, 3);
        l = mac(l, prev, mu);
        l = l_shl(l, 1);
        y[i] = round(l);
        prev = y[i];
    }
    *mem = y[L_SUBFR - 1];
}

/// `highpass_50Hz_at_12k8`.
pub(crate) fn highpass_50hz(signal: &mut [i16], mem: &mut [i16; 6]) {
    let [mut y2_hi, mut y2_lo, mut y1_hi, mut y1_lo, mut x0, mut x1] = *mem;
    for s in signal.iter_mut() {
        let mut l1 = mac(8192, y1_lo, 16211);
        l1 = mac(l1, y2_lo, -8021);
        let mut l2 = i32::from(y1_hi) * 32422;
        l2 = mac(l2, y2_hi, -16042);
        let x2 = x1;
        x1 = x0;
        x0 = *s;
        l2 = mac(l2, x2, 8106);
        l2 = mac(l2, x1, -16212);
        l2 = mac(l2, x0, 8106);
        let l1 = ((l1 >> 14).wrapping_add(l2)).wrapping_shl(2);
        y2_hi = y1_hi;
        y2_lo = y1_lo;
        y1_hi = (l1 >> 16) as i16;
        y1_lo = (l1.wrapping_sub(i32::from(y1_hi) << 16) >> 1) as i16;
        *s = shl1_round(l1);
    }
    *mem = [y2_hi, y2_lo, y1_hi, y1_lo, x0, x1];
}

/// `highpass_400Hz_at_12k8`.
pub(crate) fn highpass_400hz(signal: &mut [i16], mem: &mut [i16; 6]) {
    let [mut y2_hi, mut y2_lo, mut y1_hi, mut y1_lo, mut x0, mut x1] = *mem;
    for s in signal.iter_mut() {
        let mut l1 = mac(8192, y1_lo, 29280);
        let mut l2 = i32::from(y1_hi) * 29280;
        l1 = mac(l1, y2_lo, -14160);
        l2 = mac(l2, y2_hi, -14160);
        let x2 = x1;
        x1 = x0;
        x0 = *s;
        l2 = mac(l2, x2, 915);
        l2 = mac(l2, x1, -1830);
        l2 = mac(l2, x0, 915);
        let l1 = (l1 >> 13).wrapping_add(l2.wrapping_shl(2));
        y2_hi = y1_hi;
        y2_lo = y1_lo;
        *s = (l1.wrapping_add(0x8000) >> 16) as i16;
        y1_hi = (l1 >> 16) as i16;
        y1_lo = (l1.wrapping_sub(i32::from(y1_hi) << 16) >> 1) as i16;
    }
    *mem = [y2_hi, y2_lo, y1_hi, y1_lo, x0, x1];
}

const FIR_UP: [[i16; 24]; 4] = [
    [-1, 12, -33, 68, -119, 191, -291, 430, -634, 963, -1616, 3792, 15317, -2496, 1288, -809, 542, -369, 247, -160, 96, -52, 23, -6],
    [-4, 24, -62, 124, -213, 338, -510, 752, -1111, 1708, -2974, 8219, 12368, -3432, 1881, -1204, 812, -552, 368, -235, 139, -73, 30, -7],
    [-7, 30, -73, 139, -235, 368, -552, 812, -1204, 1881, -3432, 12368, 8219, -2974, 1708, -1111, 752, -510, 338, -213, 124, -62, 24, -4],
    [-6, 23, -52, 96, -160, 247, -369, 542, -809, 1288, -2496, 15317, 3792, -1616, 963, -634, 430, -291, 191, -119, 68, -33, 12, -1],
];

/// `oversamp_12k8_to_16k`: 64 samples at 12.8 kHz to 80 at 16 kHz.
pub(crate) fn oversample(sig: &[i16; L_SUBFR], out: &mut [i16], mem: &mut [i16; 24]) {
    let mut signal = [0i16; 24 + L_SUBFR];
    signal[..24].copy_from_slice(mem);
    signal[24..].copy_from_slice(sig);
    let mut frac: i32 = 1;
    for (j, o) in out.iter_mut().enumerate().take(L_SUBFR16K) {
        let i = (j * 6554) >> 13;
        frac -= 1;
        if frac != 0 {
            let fir = &FIR_UP[(4 - frac) as usize];
            let mut sum: i32 = 0x2000;
            for (k, &c) in fir.iter().enumerate() {
                sum = mac(sum, signal.get(i + 1 + k).copied().unwrap_or(0), c);
            }
            *o = (l_shl(sum, 2) >> 16) as i16;
        } else {
            *o = signal[12 + i];
            frac = 5;
        }
    }
    mem.copy_from_slice(&signal[L_SUBFR..]);
}

const FIR_6K_7K: [i16; 30] = [
    -32, 47, 32, -27, -369, 1122, -1421, 0, 3798, -8880, 12349, -10984, 3548, 7766, -18001, 22118, -18001, 7766, 3548, -10984, 12349, -8880, 3798, 0, -1421,
    1122, -369, -27, 32, 47,
];

/// `band_pass_6k_7k`.
pub(crate) fn band_pass_6k_7k(signal: &mut [i16; L_SUBFR16K], mem: &mut [i16; 30]) {
    let mut x = [0i16; 30 + L_SUBFR16K];
    x[..30].copy_from_slice(mem);
    for (d, s) in x[30..].iter_mut().zip(signal.iter()) {
        *d = *s >> 2;
    }
    for (n, s) in signal.iter_mut().enumerate() {
        let mut l: i32 = 0x4000 - (i32::from(x[n + 30]) << 5);
        for (j, &c) in FIR_6K_7K.iter().enumerate() {
            l = mac(l, x[n + j], c);
        }
        *s = (l >> 15) as i16;
    }
    mem.copy_from_slice(&x[L_SUBFR16K..]);
}

const FIR_7K: [i16; 31] = [
    -21, 47, -89, 146, -203, 229, -177, 0, 335, -839, 1485, -2211, 2931, -3542, 3953, 28682, 3953, -3542, 2931, -2211, 1485, -839, 335, 0, -177, 229, -203,
    146, -89, 47, -21,
];

/// `low_pass_filt_7k`.
pub(crate) fn low_pass_7k(signal: &mut [i16; L_SUBFR16K], mem: &mut [i16; 30]) {
    let mut x = [0i16; 30 + L_SUBFR16K];
    x[..30].copy_from_slice(mem);
    x[30..].copy_from_slice(signal);
    for (n, s) in signal.iter_mut().enumerate() {
        let mut l = mac(0x4000, x[n].wrapping_add(x[n + 30]), FIR_7K[0]);
        for j in 1..30 {
            l = mac(l, x[n + j], FIR_7K[j]);
        }
        *s = (l >> 15) as i16;
    }
    mem.copy_from_slice(&x[L_SUBFR16K..]);
}

/// `isf_extrapolation`: extends the 16 ISFs of the 6.60 kbit/s mode to 20 and converts them to
/// ISPs in place.
pub(crate) fn isf_extrapolation(hf: &mut [i16; M16K]) {
    const INV_LENGTH: i16 = 2731;
    let mut diff = [0i16; M - 2];
    hf[M16K - 1] = hf[M - 1];
    for i in 1..M - 1 {
        diff[i - 1] = sub(hf[i], hf[i - 1]);
    }
    let mut l: i32 = 0;
    for i in 3..M - 1 {
        l = l_mac(l, diff[i - 1], INV_LENGTH);
    }
    let mut mean = round(l);
    let mut tmp: i16 = 0;
    for &d in &diff {
        if d > tmp {
            tmp = d;
        }
    }
    let exp = norm_s(i32::from(tmp));
    for d in diff.iter_mut() {
        *d = shl(*d, exp);
    }
    mean = shl(mean, exp);
    let mut corr = [0i32; 3];
    for (c, lag) in corr.iter_mut().zip([2usize, 3, 4]) {
        for i in 7..M - 2 {
            let t2 = sub(diff[i], mean);
            let t3 = sub(diff[i - lag], mean);
            let (hi, lo) = to_dpf(l_mult(t2, t3));
            *c = l_add(*c, mpy_dpf_32(hi, lo, hi, lo));
        }
    }
    let mut max_corr = if corr[0] > corr[1] { 0 } else { 1 };
    if corr[2] > corr[max_corr] {
        max_corr = 2;
    }
    max_corr += 1;
    for i in M - 1..M16K - 1 {
        let t = sub(hf[i - 1 - max_corr], hf[i - 2 - max_corr]);
        hf[i] = add(hf[i - 1], t);
    }
    let mut tmp = add(hf[4], hf[3]);
    tmp = sub(hf[2], tmp);
    tmp = mult(tmp, 5461);
    tmp = tmp.wrapping_add(20390);
    if tmp > 19456 {
        tmp = 19456;
    }
    let mut tmp = sub(tmp, hf[M - 2]);
    let mut tmp2 = sub(hf[M16K - 2], hf[M - 2]);
    let exp2 = norm_s(i32::from(tmp2));
    let mut exp = norm_s(i32::from(tmp)) - 1;
    tmp = shl_int(i32::from(tmp), i32::from(exp)) as i16;
    tmp2 = shl_int(i32::from(tmp2), i32::from(exp2)) as i16;
    let coeff = div_16by16(tmp, tmp2);
    exp = exp2.wrapping_sub(exp);
    let mut d = [0i16; M16K - M];
    for i in M - 1..M16K - 1 {
        let t = mult(sub(hf[i], hf[i - 1]), coeff);
        d[i - (M - 1)] = shl(t, exp);
    }
    for i in M..M16K - 1 {
        let (a, b) = (d[i - (M - 1)], d[i - M]);
        let t = (i32::from(a) + i32::from(b) - 1280) as i16;
        if t < 0 {
            if a > b {
                d[i - M] = (1280 - i32::from(a)) as i16;
            } else {
                d[i - (M - 1)] = (1280 - i32::from(b)) as i16;
            }
        }
    }
    for i in M - 1..M16K - 1 {
        hf[i] = add(hf[i - 1], d[i - (M - 1)]);
    }
    for v in hf.iter_mut().take(M16K - 1) {
        *v = mult(*v, 26214);
    }
    isf_isp(hf);
}

/// `agc2_amr_wb`: scales `sig_out` to the energy of `sig_in`.
pub(crate) fn agc2(sig_in: &[i16; L_SUBFR], sig_out: &mut [i16; L_SUBFR]) {
    let t = sig_out[0] >> 2;
    let mut s = (i32::from(t) * i32::from(t)).wrapping_shl(1);
    for &v in &sig_out[1..] {
        let t = v >> 2;
        s = l_mac(s, t, t);
    }
    if s == 0 {
        return;
    }
    let mut exp = norm(s) - 1;
    let gain_out = round(shl_int(s, i32::from(exp)));
    let t = sig_in[0] >> 2;
    let mut s = l_mult(t, t);
    for &v in &sig_in[1..] {
        let t = v >> 2;
        s = l_mac(s, t, t);
    }
    let g0 = if s == 0 {
        0
    } else {
        let i = norm(s);
        let gain_in = round(shl_int(s, i32::from(i)));
        exp -= i;
        let mut s = i32::from(div_16by16(gain_out, gain_in));
        s = l_shl(s, 7);
        s = l_shr(s, exp);
        s = one_ov_sqrt(s);
        round(l_shl(s, 9))
    };
    for v in sig_out.iter_mut() {
        *v = (l_shl(i32::from(*v) * i32::from(g0), 3) >> 16) as i16;
    }
}

/// `voice_factor`: -1 (unvoiced) .. 1 (voiced) in Q15.
pub(crate) fn voice_factor(exc: &[i16], q_exc: i16, gain_pit: i16, code: &[i16], gain_code: i16) -> i16 {
    let mut exp1 = 0;
    let mut ener1 = (dot_product12(exc, exc, &mut exp1) >> 16) as i16;
    exp1 = sub(exp1, q_exc.wrapping_shl(1));
    let l = l_mult(gain_pit, gain_pit);
    let exp = norm(l);
    let tmp = (shl_int(l, i32::from(exp)) >> 16) as i16;
    ener1 = mult(ener1, tmp);
    exp1 = exp1.wrapping_sub(exp.wrapping_add(10));
    let mut exp2 = 0;
    let mut ener2 = (dot_product12(code, code, &mut exp2) >> 16) as i16;
    let exp = norm_s(i32::from(gain_code));
    let tmp = shl(gain_code, exp);
    let tmp = mult(tmp, tmp);
    ener2 = mult(ener2, tmp);
    exp2 = exp2.wrapping_sub(exp.wrapping_shl(1));
    let i = i32::from(exp1.wrapping_sub(exp2));
    if i >= 0 {
        ener1 >>= 1;
        ener2 = shr_int(i32::from(ener2), i + 1) as i16;
    } else {
        ener1 = shr_int(i32::from(ener1), 1 - i) as i16;
        ener2 >>= 1;
    }
    let tmp = ener1.wrapping_sub(ener2);
    let ener1 = ener1.wrapping_add(ener2).wrapping_add(1);
    if tmp >= 0 { div_16by16(tmp, ener1) } else { negate(div_16by16(negate(tmp), ener1)) }
}

const PH_IMP_LOW: [i16; L_SUBFR] = [
    20182, 9693, 3270, -3437, 2864, -5240, 1589, -1357, 600, 3893, -1497, -698, 1203, -5249, 1199, 5371, -1488, -705, -2887, 1976, 898, 721, -3876, 4227,
    -5112, 6400, -1032, -4725, 4093, -4352, 3205, 2130, -1996, -1835, 2648, -1786, -406, 573, 2484, -3608, 3139, -1363, -2566, 3808, -639, -2051, -541, 2376,
    3932, -6262, 1432, -3601, 4889, 370, 567, -1163, -2854, 1914, 39, -2418, 3454, 2975, -4021, 3431,
];

const PH_IMP_MID: [i16; L_SUBFR] = [
    24098, 10460, -5263, -763, 2048, -927, 1753, -3323, 2212, 652, -2146, 2487, -3539, 4109, -2107, -374, -626, 4270, -5485, 2235, 1858, -2769, 744, 1140,
    -763, -1615, 4060, -4574, 2982, -1163, 731, -1098, 803, 167, -714, 606, -560, 639, 43, -1766, 3228, -2782, 665, 763, 233, -2002, 1291, 1871, -3470, 1032,
    2710, -4040, 3624, -4214, 5292, -4270, 1563, 108, -580, 1642, -2458, 957, 544, 2540,
];

/// `phase_dispersion`. `mem` holds the previous state, the previous code gain and six past
/// pitch gains.
pub(crate) fn phase_dispersion(gain_code: i16, gain_pit: i16, code: &mut [i16; L_SUBFR], mode: i16, mem: &mut [i16; 8]) {
    const PITCH_0_9: i16 = 14746;
    const PITCH_0_6: i16 = 9830;
    let mut state: i16 = if gain_pit < PITCH_0_6 {
        0
    } else if gain_pit < PITCH_0_9 {
        1
    } else {
        2
    };
    for i in (3..8).rev() {
        mem[i] = mem[i - 1];
    }
    mem[2] = gain_pit;
    if sub(gain_code, mem[1]) > shl(mem[1], 1) {
        if state < 2 {
            state += 1;
        }
    } else {
        let j = mem[2..8].iter().filter(|&&g| g < PITCH_0_6).count();
        if j > 2 {
            state = 0;
        }
        if i32::from(state) > i32::from(mem[0]) + 1 {
            state -= 1;
        }
    }
    mem[1] = gain_code;
    mem[0] = state;
    let state = state.wrapping_add(mode);
    let imp = match state {
        0 => &PH_IMP_LOW,
        1 => &PH_IMP_MID,
        _ => return,
    };
    let mut code2 = [0i16; 2 * L_SUBFR];
    for (i, &c) in code.iter().enumerate() {
        if c != 0 {
            for (j, &h) in imp.iter().enumerate() {
                code2[i + j] = add(code2[i + j], mult_r(c, h));
            }
        }
    }
    for (i, c) in code.iter_mut().enumerate() {
        *c = add(code2[i], code2[i + L_SUBFR]);
    }
}

/// `scale_signal`.
pub(crate) fn scale_signal(x: &mut [i16], exp: i16) {
    if exp > 0 {
        for v in x.iter_mut() {
            *v = round(l_shl(i32::from(*v) << 16, exp));
        }
    } else if exp < 0 {
        let e = (-i32::from(exp)) & 0xf;
        let tmp = (0x8000 >> (16 - e)) as i16;
        let n = x.len() & !1;
        for v in x.iter_mut().take(n) {
            *v = add(*v, tmp) >> e;
        }
    }
}

/// `preemph_amrwb_dec`.
pub(crate) fn preemph(x: &mut [i16; L_SUBFR], mu: i16) {
    for i in (1..L_SUBFR).rev() {
        x[i] = round(l_msu(i32::from(x[i]) << 16, x[i - 1], mu));
    }
}

/// `Pit_shrp`.
pub(crate) fn pit_shrp(x: &mut [i16; L_SUBFR], lag: i16, sharp: i16) {
    let Ok(lag) = usize::try_from(lag) else {
        return;
    };
    for i in lag..L_SUBFR {
        x[i] = round(l_mac(i32::from(x[i]) << 16, x[i - lag], sharp));
    }
}
