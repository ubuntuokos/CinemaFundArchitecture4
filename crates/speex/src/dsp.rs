//! Signal-processing helpers of the float build of libspeex 1.2.1 (`filters.c`, `lsp.c`,
//! `quant_lsp.c`, `math_approx.h`, `ltp_sse.h`). Arithmetic follows C's promotion rules exactly:
//! `float` operations stay in `f32`, and wherever the reference mixes in a `double` literal or a
//! libm call the expression is evaluated in `f64` and rounded where C assigns it to a `float`.

use crate::tables;

/// `buf[i]`, or 0 outside the buffer.
pub(crate) fn at(buf: &[f32], i: isize) -> f32 {
    usize::try_from(i).ok().and_then(|i| buf.get(i)).copied().unwrap_or(0.0)
}

/// `buf[start..start + len]`, or an empty slice outside the buffer.
pub(crate) fn seg(buf: &[f32], start: isize, len: usize) -> &[f32] {
    usize::try_from(start).ok().and_then(|s| buf.get(s..s.checked_add(len)?)).unwrap_or(&[])
}

/// `inner_prod` of the SSE build (`ltp_sse.h`, which Linux distributions compile in): four lanes,
/// eight products per step, lanes summed as `(l0 + l2) + (l1 + l3)`.
pub(crate) fn inner_prod(a: &[f32], b: &[f32], len: usize) -> f32 {
    let mut l = [0f32; 4];
    let steps = (len >> 2).div_ceil(2);
    let v = |s: &[f32], i: usize| s.get(i).copied().unwrap_or(0.0);
    for step in 0..steps {
        let base = step * 8;
        for k in 0..4 {
            l[k] += v(a, base + k) * v(b, base + k);
        }
        for k in 0..4 {
            l[k] += v(a, base + 4 + k) * v(b, base + 4 + k);
        }
    }
    (l[0] + l[2]) + (l[1] + l[3])
}

/// `speex_rand`: a linear congruential generator turned into a uniform float of deviation `std`.
pub(crate) fn speex_rand(std: f32, seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    let f = f32::from_bits(0x3f80_0000 | (*seed & 0x007f_ffff));
    let f = (f64::from(f) - 1.5) as f32;
    (3.4642 * f64::from(std) * f64::from(f)) as f32
}

/// `spx_cos`: a polynomial cosine on [0, π].
pub(crate) fn spx_cos(x: f32) -> f32 {
    const C1: f32 = 0.999_993_3;
    const C2: f32 = -0.499_912_44;
    const C3: f32 = 0.041_487_746;
    const C4: f32 = -0.001_271_209_5;
    // The reference compares with 1.5707963268; no f32 lies between that and π/2.
    if f64::from(x) < std::f64::consts::FRAC_PI_2 {
        let x = x * x;
        C1 + x * (C2 + x * (C3 + C4 * x))
    } else {
        let x = (std::f64::consts::PI - f64::from(x)) as f32;
        let x = x * x;
        -(C1 + x * (C2 + x * (C3 + C4 * x)))
    }
}

/// `bw_lpc`: bandwidth expansion of an LPC filter.
pub(crate) fn bw_lpc(gamma: f32, lpc_in: &[f32], lpc_out: &mut [f32]) {
    let mut tmp = gamma;
    for (o, i) in lpc_out.iter_mut().zip(lpc_in) {
        *o = tmp * i;
        tmp *= gamma;
    }
}

/// `sanitize_values32` (NaN becomes 0).
pub(crate) fn sanitize(v: &mut [f32], min: f32, max: f32) {
    for x in v {
        if !(*x >= min && *x <= max) {
            *x = if *x < min {
                min
            } else if *x > max {
                max
            } else {
                0.0
            };
        }
    }
}

/// `compute_rms` / `compute_rms16`.
pub(crate) fn compute_rms(x: &[f32]) -> f32 {
    let mut sum = 0f32;
    for v in x {
        sum += v * v;
    }
    let len = x.len().max(1) as f32;
    (0.1 + f64::from(sum / len)).sqrt() as f32
}

/// `highpass` on the decoder output (`HIGHPASS_OUTPUT`, narrowband or wideband), in place.
pub(crate) fn highpass(x: &mut [f32], wideband: bool, mem: &mut [f32; 2]) {
    const P: [[f32; 3]; 5] =
        [[1.0, -1.911_2, 0.914_98], [1.0, -1.926_83, 0.930_71], [1.0, -1.933_38, 0.935_53], [1.0, -1.972_26, 0.973_32], [1.0, -1.37, 0.399]];
    const Z: [[f32; 3]; 5] = [
        [0.956_54, -1.913_09, 0.956_54],
        [0.964_46, -1.928_79, 0.964_46],
        [0.967_23, -1.934_45, 0.967_23],
        [0.986_45, -1.972_77, 0.986_45],
        [0.88, -1.76, 0.88],
    ];
    let id = if wideband { 3 } else { 1 };
    let (den, num) = (P[id], Z[id]);
    for v in x {
        let xi = *v;
        let vout = num[0] * xi + mem[0];
        mem[0] = (mem[1] + num[1] * xi) + (-den[1] * vout);
        mem[1] = num[2] * xi + (-den[2] * vout);
        *v = vout;
    }
}

/// `iir_mem16` (synthesis filter `1/A(z)`), in place.
pub(crate) fn iir_mem16(x: &mut [f32], den: &[f32], mem: &mut [f32]) {
    let ord = den.len().min(mem.len());
    if ord == 0 {
        return;
    }
    for v in x {
        let yi = *v + mem[0];
        let nyi = -yi;
        for j in 0..ord - 1 {
            mem[j] = mem[j + 1] + den[j] * nyi;
        }
        mem[ord - 1] = den[ord - 1] * nyi;
        *v = yi;
    }
}

/// `qmf_synth` with the 64-tap `h0` filter: `out` holds the low band in its first half and the high
/// band in its second, and receives the full-band signal.
pub(crate) fn qmf_synth(out: &mut [f32], mem1: &mut [f32; 64], mem2: &mut [f32; 64]) {
    const M2: usize = 32;
    let a = &tables::H0;
    let n = out.len();
    let n2 = n / 2;
    let mut xx1 = vec![0f32; M2 + n2];
    let mut xx2 = vec![0f32; M2 + n2];
    for i in 0..n2 {
        xx1[i] = out[n2 - 1 - i];
        xx2[i] = out[n - 1 - i];
    }
    for i in 0..M2 {
        xx1[n2 + i] = mem1[2 * i + 1];
        xx2[n2 + i] = mem2[2 * i + 1];
    }
    let mut i = 0;
    while i + 1 < n2 {
        let (mut y0, mut y1, mut y2, mut y3) = (0f32, 0f32, 0f32, 0f32);
        let mut x10 = xx1[n2 - 2 - i];
        let mut x20 = xx2[n2 - 2 - i];
        let mut j = 0;
        while j < M2 {
            let (a0, a1) = (a[2 * j], a[2 * j + 1]);
            let x11 = xx1[n2 - 1 + j - i];
            let x21 = xx2[n2 - 1 + j - i];
            y0 += a0 * (x11 - x21);
            y1 += a1 * (x11 + x21);
            y2 += a0 * (x10 - x20);
            y3 += a1 * (x10 + x20);
            let (a0, a1) = (a[2 * j + 2], a[2 * j + 3]);
            x10 = xx1[n2 + j - i];
            x20 = xx2[n2 + j - i];
            y0 += a0 * (x10 - x20);
            y1 += a1 * (x10 + x20);
            y2 += a0 * (x11 - x21);
            y3 += a1 * (x11 + x21);
            j += 2;
        }
        out[2 * i] = 2.0 * y0;
        out[2 * i + 1] = 2.0 * y1;
        out[2 * i + 2] = 2.0 * y2;
        out[2 * i + 3] = 2.0 * y3;
        i += 2;
    }
    for i in 0..M2 {
        mem1[2 * i + 1] = xx1[i];
        mem2[2 * i + 1] = xx2[i];
    }
}

const SHIFT_FILT: [[f32; 7]; 3] = [
    [-0.011_915, 0.046_995, -0.152_373, 0.614_108, 0.614_108, -0.152_373, 0.046_995],
    [-0.032_485_5, 0.085_976_8, -0.204_298_6, 0.964_029_7, 0.208_642, -0.030_205_4, -0.006_364_6],
    [-0.006_364_6, -0.030_205_4, 0.208_642, 0.964_029_7, -0.204_298_6, 0.085_976_8, -0.032_485_5],
];

/// `interp_pitch`: the excitation at `pitch` (fractional, from the best of four interpolations).
fn interp_pitch(exc: &[f32], base: isize, interp: &mut [f32], pitch: isize, len: usize) {
    let mut corr = [[0f32; 7]; 4];
    let x = seg(exc, base, len);
    for i in 0..7 {
        corr[0][i] = inner_prod(x, seg(exc, base - pitch - 3 + i as isize, len), len);
    }
    for i in 0..3 {
        for j in 0..7usize {
            let i1 = 3usize.saturating_sub(j);
            let i2 = (10 - j).min(7);
            let mut tmp = 0f32;
            for k in i1..i2 {
                tmp += SHIFT_FILT[i][k] * corr[0][j + k - 3];
            }
            corr[i + 1][j] = tmp;
        }
    }
    let (mut maxi, mut maxj) = (0usize, 0usize);
    let mut maxcorr = corr[0][0];
    for (i, row) in corr.iter().enumerate() {
        for (j, &c) in row.iter().enumerate() {
            if c > maxcorr {
                maxcorr = c;
                maxi = i;
                maxj = j;
            }
        }
    }
    let lag = pitch - maxj as isize + 3;
    for (i, o) in interp.iter_mut().enumerate().take(len) {
        let p = base + i as isize - lag;
        *o = if maxi > 0 {
            let mut tmp = 0f32;
            for k in 0..7 {
                tmp += at(exc, p + k as isize - 3) * SHIFT_FILT[maxi - 1][k];
            }
            tmp
        } else {
            at(exc, p)
        };
    }
}

/// `multicomb`: the comb-filter enhancer on `nsf` samples of excitation at `exc[base..]`.
pub(crate) fn multicomb(exc: &[f32], base: isize, new_exc: &mut [f32], nsf: usize, pitch: i32, max_pitch: i32, comb_gain: f32) {
    let mut iexc = vec![0f32; 2 * nsf];
    let pitch = pitch as isize;
    interp_pitch(exc, base, &mut iexc[..nsf], pitch, 80);
    if pitch > max_pitch as isize {
        interp_pitch(exc, base, &mut iexc[nsf..], 2 * pitch, 80);
    } else {
        interp_pitch(exc, base, &mut iexc[nsf..], -pitch, 80);
    }
    let x = seg(exc, base, nsf);
    let (i0, i1) = iexc.split_at(nsf);
    let iexc0_mag = f64::from(1000.0 + inner_prod(i0, i0, nsf)).sqrt() as f32;
    let iexc1_mag = f64::from(1000.0 + inner_prod(i1, i1, nsf)).sqrt() as f32;
    let exc_mag = f64::from(1.0 + inner_prod(x, x, nsf)).sqrt() as f32;
    let corr0 = inner_prod(i0, x, nsf).max(0.0);
    let corr1 = inner_prod(i1, x, nsf).max(0.0);
    let pgain1 = if corr0 > iexc0_mag * exc_mag { 1.0 } else { (corr0 / exc_mag) / iexc0_mag };
    let pgain2 = if corr1 > iexc1_mag * exc_mag { 1.0 } else { (corr1 / exc_mag) / iexc1_mag };
    let gg1 = exc_mag / iexc0_mag;
    let gg2 = exc_mag / iexc1_mag;
    let (c1, c2) = if comb_gain > 0.0 {
        let c1 = (0.4 * f64::from(comb_gain) + 0.07) as f32;
        let c2 = (0.5 + 1.72 * (f64::from(c1) - 0.07)) as f32;
        (c1, c2)
    } else {
        (0.0, 0.0)
    };
    let mut g1 = 1.0 - c2 * pgain1 * pgain1;
    let mut g2 = 1.0 - c2 * pgain2 * pgain2;
    if g1 < c1 {
        g1 = c1;
    }
    if g2 < c1 {
        g2 = c1;
    }
    let g1 = c1 / g1;
    let g2 = c1 / g2;
    let (gain0, gain1) = if pitch > max_pitch as isize {
        ((0.7 * f64::from(g1 * gg1)) as f32, (0.3 * f64::from(g2 * gg2)) as f32)
    } else {
        ((0.6 * f64::from(g1 * gg1)) as f32, (0.6 * f64::from(g2 * gg2)) as f32)
    };
    for (i, o) in new_exc.iter_mut().enumerate().take(nsf) {
        *o = at(exc, base + i as isize) + (gain0 * i0[i] + gain1 * i1[i]);
    }
    let new_ener = compute_rms(&new_exc[..nsf.min(new_exc.len())]);
    let old_ener = compute_rms(x);
    let mut old_ener = old_ener.max(1.0);
    let new_ener = new_ener.max(1.0);
    if old_ener > new_ener {
        old_ener = new_ener;
    }
    let ngain = old_ener / new_ener;
    for o in new_exc.iter_mut().take(nsf) {
        *o *= ngain;
    }
}

/// `lsp_to_lpc` (float version).
pub(crate) fn lsp_to_lpc(freq: &[f32], ak: &mut [f32]) {
    let order = freq.len();
    let m = order / 2;
    let mut wp = vec![0f32; 4 * m + 2];
    let x: Vec<f32> = freq.iter().map(|&f| spx_cos(f)).collect();
    let (mut xin1, mut xin2) = (1f32, 1f32);
    for j in 0..=order {
        for i in 0..m {
            let n = i * 4;
            let xout1 = xin1 - 2.0 * x[2 * i] * wp[n] + wp[n + 1];
            let xout2 = xin2 - 2.0 * x[2 * i + 1] * wp[n + 2] + wp[n + 3];
            wp[n + 1] = wp[n];
            wp[n + 3] = wp[n + 2];
            wp[n] = xin1;
            wp[n + 2] = xin2;
            xin1 = xout1;
            xin2 = xout2;
        }
        let xout1 = xin1 + wp[4 * m];
        let xout2 = xin2 - wp[4 * m + 1];
        if j > 0
            && let Some(a) = ak.get_mut(j - 1)
        {
            *a = (xout1 + xout2) * 0.5;
        }
        wp[4 * m] = xin1;
        wp[4 * m + 1] = xin2;
        xin1 = 0.0;
        xin2 = 0.0;
    }
}

/// `lsp_interpolate`, with the stability margin.
pub(crate) fn lsp_interpolate(old: &[f32], new: &[f32], lsp: &mut [f32], subframe: usize, nb_subframes: usize, margin: f32) {
    let len = lsp.len();
    if len < 2 {
        return;
    }
    let tmp = (1.0f32 + subframe as f32) / nb_subframes as f32;
    for i in 0..len {
        lsp[i] = (1.0 - tmp) * old[i] + tmp * new[i];
    }
    if lsp[0] < margin {
        lsp[0] = margin;
    }
    let top = std::f64::consts::PI - f64::from(margin);
    if f64::from(lsp[len - 1]) > top {
        lsp[len - 1] = top as f32;
    }
    for i in 1..len - 1 {
        if lsp[i] < lsp[i - 1] + margin {
            lsp[i] = lsp[i - 1] + margin;
        }
        if lsp[i] > lsp[i + 1] - margin {
            lsp[i] = 0.5 * (lsp[i] + lsp[i + 1] - margin);
        }
    }
}

/// `x += k * cb`, evaluated in double as in `quant_lsp.c`.
fn add(x: &mut f32, k: f64, cb: i8) {
    *x = (f64::from(*x) + k * f64::from(cb)) as f32;
}

fn rows(cb: &[i8], id: u32, n: usize) -> &[i8] {
    cb.get(id as usize * n..(id as usize + 1) * n).unwrap_or(&[])
}

/// `lsp_unquant_nb` (submodes 5 to 7).
pub(crate) fn lsp_unquant_nb(lsp: &mut [f32; 10], bits: &mut crate::bits::Bits<'_>) {
    for (i, l) in lsp.iter_mut().enumerate() {
        *l = (0.25 * i as f64 + 0.25) as f32;
    }
    let id = bits.unpack(6);
    for (l, &c) in lsp.iter_mut().zip(rows(&tables::CDBK_NB, id, 10)) {
        add(l, 0.0039062, c);
    }
    let id = bits.unpack(6);
    for (l, &c) in lsp[..5].iter_mut().zip(rows(&tables::CDBK_NB_LOW1, id, 5)) {
        add(l, 0.0019531, c);
    }
    let id = bits.unpack(6);
    for (l, &c) in lsp[..5].iter_mut().zip(rows(&tables::CDBK_NB_LOW2, id, 5)) {
        add(l, 0.00097656, c);
    }
    let id = bits.unpack(6);
    for (l, &c) in lsp[5..].iter_mut().zip(rows(&tables::CDBK_NB_HIGH1, id, 5)) {
        add(l, 0.0019531, c);
    }
    let id = bits.unpack(6);
    for (l, &c) in lsp[5..].iter_mut().zip(rows(&tables::CDBK_NB_HIGH2, id, 5)) {
        add(l, 0.00097656, c);
    }
}

/// `lsp_unquant_lbr` (submodes 1 to 4 and 8).
pub(crate) fn lsp_unquant_lbr(lsp: &mut [f32; 10], bits: &mut crate::bits::Bits<'_>) {
    for (i, l) in lsp.iter_mut().enumerate() {
        *l = (0.25 * i as f64 + 0.25) as f32;
    }
    let id = bits.unpack(6);
    for (l, &c) in lsp.iter_mut().zip(rows(&tables::CDBK_NB, id, 10)) {
        add(l, 0.0039062, c);
    }
    let id = bits.unpack(6);
    for (l, &c) in lsp[..5].iter_mut().zip(rows(&tables::CDBK_NB_LOW1, id, 5)) {
        add(l, 0.0019531, c);
    }
    let id = bits.unpack(6);
    for (l, &c) in lsp[5..].iter_mut().zip(rows(&tables::CDBK_NB_HIGH1, id, 5)) {
        add(l, 0.0019531, c);
    }
}

/// `lsp_unquant_high` (the 8 LSPs of a high band).
pub(crate) fn lsp_unquant_high(lsp: &mut [f32; 8], bits: &mut crate::bits::Bits<'_>) {
    for (i, l) in lsp.iter_mut().enumerate() {
        *l = (0.3125 * i as f64 + 0.75) as f32;
    }
    let id = bits.unpack(6);
    for (l, &c) in lsp.iter_mut().zip(rows(&tables::HIGH_LSP_CDBK, id, 8)) {
        add(l, 0.0039062, c);
    }
    let id = bits.unpack(6);
    for (l, &c) in lsp.iter_mut().zip(rows(&tables::HIGH_LSP_CDBK2, id, 8)) {
        add(l, 0.0019531, c);
    }
}

/// Parameters of a split-VQ innovation codebook (`split_cb_params`).
pub(crate) struct SplitCb {
    pub subvect_size: usize,
    pub nb_subvect: usize,
    pub shape_cb: &'static [i8],
    pub shape_bits: u32,
    pub have_sign: bool,
}

/// `split_cb_shape_sign_unquant`: adds the decoded innovation to `exc`.
pub(crate) fn split_cb_unquant(exc: &mut [f32], p: &SplitCb, bits: &mut crate::bits::Bits<'_>) {
    let mut ind = [0u32; 10];
    let mut signs = [false; 10];
    for i in 0..p.nb_subvect.min(10) {
        signs[i] = p.have_sign && bits.unpack(1) != 0;
        ind[i] = bits.unpack(p.shape_bits);
    }
    for i in 0..p.nb_subvect.min(10) {
        let s: f64 = if signs[i] { -1.0 } else { 1.0 };
        let row = rows(p.shape_cb, ind[i], p.subvect_size);
        for (j, &c) in row.iter().enumerate() {
            if let Some(e) = exc.get_mut(p.subvect_size * i + j) {
                *e = (f64::from(*e) + s * 0.03125 * f64::from(c)) as f32;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_inner_product_order() {
        let a: Vec<f32> = (0..40).map(|i| 1.0 + i as f32 * 1e-3).collect();
        let b: Vec<f32> = (0..40).map(|i| 1e8 * if i % 3 == 0 { 1.0 } else { -1.0 }).collect();
        let mut l = [0f32; 4];
        for i in 0..40 {
            l[i % 4] += a[i] * b[i];
        }
        assert_eq!(inner_prod(&a, &b, 40), (l[0] + l[2]) + (l[1] + l[3]));
    }

    #[test]
    fn rand_and_cos_follow_the_reference() {
        let mut seed = 1000u32;
        let r = speex_rand(1.0, &mut seed);
        assert_eq!(seed, 1000u32.wrapping_mul(1_664_525).wrapping_add(1_013_904_223));
        assert!(r.abs() < 1.8);
        assert!((spx_cos(0.0) - 1.0).abs() < 1e-5);
        assert!((spx_cos(3.0) - 3f32.cos()).abs() < 1e-3);
    }
}
