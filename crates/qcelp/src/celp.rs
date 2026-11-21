//! Float CELP helpers: ports of FFmpeg n7.1.1 `libavutil/float_dsp.c` (`avpriv_scalarproduct_float_c`),
//! `libavcodec/celp_filters.c`, `acelp_filters.c`, `acelp_vectors.c` and `lsp.c` (Copyright (c) 2007
//! Reynaldo H. Verdejo Pinochet, 2008 Vladimir Voroshilov; LGPL-2.1-or-later), in FFmpeg's operation
//! order so the results match bit for bit.

/// `avpriv_scalarproduct_float_c`.
pub(crate) fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut p = 0f32;
    for (x, y) in a.iter().zip(b) {
        p += x * y;
    }
    p
}

/// `ff_celp_lp_synthesis_filterf` on `buf[start..start + input.len()]`, with `c.len()` taps of
/// history before `start` (an even order of at least 4), FFmpeg's unrolled form.
pub(crate) fn lp_synthesis(buf: &mut [f32], start: usize, c: &[f32], input: &[f32]) {
    let order = c.len();
    let len = input.len();
    let a = c[0];
    let mut b = c[1];
    let mut cc = c[2];
    b -= c[0] * c[0];
    cc -= c[1] * c[0];
    cc -= c[0] * b;
    let mut old0 = buf[start - 4];
    let mut old1 = buf[start - 3];
    let mut old2 = buf[start - 2];
    let mut old3 = buf[start - 1];
    let mut n = 0;
    while n + 4 <= len {
        let o = start + n;
        let mut out0 = input[n];
        let mut out1 = input[n + 1];
        let mut out2 = input[n + 2];
        let mut out3 = input[n + 3];
        out0 -= c[2] * old1;
        out1 -= c[2] * old2;
        out2 -= c[2] * old3;
        out0 -= c[1] * old2;
        out1 -= c[1] * old3;
        out0 -= c[0] * old3;
        let mut val = c[3];
        out0 -= val * old0;
        out1 -= val * old1;
        out2 -= val * old2;
        out3 -= val * old3;
        let mut i = 5;
        while i < order {
            old3 = buf[o - i];
            val = c[i - 1];
            out0 -= val * old3;
            out1 -= val * old0;
            out2 -= val * old1;
            out3 -= val * old2;
            old2 = buf[o - i - 1];
            val = c[i];
            out0 -= val * old2;
            out1 -= val * old3;
            out2 -= val * old0;
            out3 -= val * old1;
            std::mem::swap(&mut old0, &mut old2);
            old1 = old3;
            i += 2;
        }
        let (t0, t1, t2) = (out0, out1, out2);
        out3 -= a * t2;
        out2 -= a * t1;
        out1 -= a * t0;
        out3 -= b * t1;
        out2 -= b * t0;
        out3 -= cc * t0;
        buf[o] = out0;
        buf[o + 1] = out1;
        buf[o + 2] = out2;
        buf[o + 3] = out3;
        old0 = out0;
        old1 = out1;
        old2 = out2;
        old3 = out3;
        n += 4;
    }
    while n < len {
        let o = start + n;
        let mut v = input[n];
        for i in 1..=order {
            v -= c[i - 1] * buf[o - i];
        }
        buf[o] = v;
        n += 1;
    }
}

/// `ff_celp_lp_zero_synthesis_filterf`: `out[n] = input[start + n] + Σ c[i-1]·input[start + n - i]`.
pub(crate) fn lp_zero_synthesis(out: &mut [f32], c: &[f32], input: &[f32], start: usize) {
    for (n, o) in out.iter_mut().enumerate() {
        let mut v = input[start + n];
        for i in 1..=c.len() {
            v += c[i - 1] * input[start + n - i];
        }
        *o = v;
    }
}

/// `ff_tilt_compensation`.
pub(crate) fn tilt_compensation(mem: &mut f32, tilt: f32, s: &mut [f32]) {
    let Some(&last) = s.last() else { return };
    for i in (1..s.len()).rev() {
        s[i] -= tilt * s[i - 1];
    }
    s[0] -= tilt * *mem;
    *mem = last;
}

/// `ff_weighted_vector_sumf`.
pub(crate) fn weighted_sum(out: &mut [f32], a: &[f32], b: &[f32], wa: f32, wb: f32) {
    for ((o, &x), &y) in out.iter_mut().zip(a).zip(b) {
        *o = wa * x + wb * y;
    }
}

/// `ff_adaptive_gain_control` in place (`out` and `in` are the same vector).
pub(crate) fn adaptive_gain_control(s: &mut [f32], speech_energ: f32, alpha: f32, gain_mem: &mut f32) {
    let post = dot(s, s);
    let mut scale = 1.0f32;
    if post != 0.0 {
        scale = f64::from(speech_energ / post).sqrt() as f32;
    }
    scale = (f64::from(scale) * (1.0 - f64::from(alpha))) as f32;
    let mut mem = *gain_mem;
    for v in s {
        mem = alpha * mem + scale;
        *v *= mem;
    }
    *gain_mem = mem;
}

/// `ff_scale_vector_to_given_sum_of_squares`.
pub(crate) fn scale_to_sum_of_squares(out: &mut [f32], input: &[f32], sum_of_squares: f32) {
    let mut scale = dot(input, input);
    if scale != 0.0 {
        scale = f64::from(sum_of_squares / scale).sqrt() as f32;
    }
    for (o, &x) in out.iter_mut().zip(input) {
        *o = x * scale;
    }
}

/// `lsp2polyf`: `f[0..=half]` from every second LSP starting at `lsp[0]`.
fn lsp2polyf(lsp: &[f64], f: &mut [f64], half: usize) {
    f[0] = 1.0;
    f[1] = -2.0 * lsp[0];
    for i in 2..=half {
        let val = -2.0 * lsp[2 * i - 2];
        f[i] = val * f[i - 1] + 2.0 * f[i - 2];
        for j in (2..i).rev() {
            f[j] += f[j - 1] * val + f[j - 2];
        }
        f[1] += val;
    }
}

/// `ff_acelp_lspd2lpc`.
pub(crate) fn lspd2lpc(lsp: &[f64], lpc: &mut [f32], half: usize) {
    let mut pa = [0f64; 11];
    let mut qa = [0f64; 11];
    lsp2polyf(lsp, &mut pa, half);
    lsp2polyf(&lsp[1..], &mut qa, half);
    let top = 2 * half - 1;
    for k in (0..half).rev() {
        let paf = pa[k + 1] + pa[k];
        let qaf = qa[k + 1] - qa[k];
        lpc[k] = (0.5 * (paf + qaf)) as f32;
        lpc[top - k] = (0.5 * (paf - qaf)) as f32;
    }
}
