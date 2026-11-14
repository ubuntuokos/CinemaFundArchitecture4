//! Float CELP helpers shared by the RealAudio 2.0 and SIPR decoders: ports of FFmpeg n7.1.1
//! `libavutil/float_dsp.c` (`avpriv_scalarproduct_float_c`), `libavcodec/celp_filters.c`,
//! `acelp_filters.c`, `acelp_vectors.c`, `acelp_pitch_delay.c` and `lsp.c` (Copyright (c) 2007
//! Reynaldo H. Verdejo Pinochet, 2008 Vladimir Voroshilov; LGPL-2.1-or-later), in FFmpeg's
//! operation order so the results match bit for bit.

/// `avpriv_scalarproduct_float_c`.
pub(crate) fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut p = 0f32;
    for (x, y) in a.iter().zip(b) {
        p += x * y;
    }
    p
}

/// `ff_celp_lp_synthesis_filterf` on `buf[start..start + len]`, with `filter.len()` taps of
/// history before `start` (an even order of at least 4).
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

/// `ff_acelp_interpolatef` within one buffer: `buf[out + n]` from `buf[inp + n ± i]`, in order
/// (the source may overlap what is being written, as in FFmpeg).
#[allow(clippy::too_many_arguments)]
pub(crate) fn interpolate(buf: &mut [f32], out: usize, inp: usize, coeffs: &[f32], precision: usize, frac: usize, filter_length: usize, length: usize) {
    for n in 0..length {
        let mut idx = 0;
        let mut v = 0f32;
        let mut i = 0;
        while i < filter_length {
            v += buf[inp + n + i] * coeffs[idx + frac];
            idx += precision;
            i += 1;
            v += buf[inp + n - i] * coeffs[idx - frac];
        }
        buf[out + n] = v;
    }
}

/// `ff_acelp_apply_order_2_transfer_function`.
pub(crate) fn order_2_transfer(out: &mut [f32], input: &[f32], zero: [f32; 2], pole: [f32; 2], gain: f32, mem: &mut [f32; 2]) {
    for (o, &x) in out.iter_mut().zip(input) {
        let tmp = gain * x - pole[0] * mem[0] - pole[1] * mem[1];
        *o = tmp + zero[0] * mem[0] + zero[1] * mem[1];
        mem[1] = mem[0];
        mem[0] = tmp;
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

/// `ff_weighted_vector_sumf` with `out` as the first input.
pub(crate) fn weighted_sum_in_place(out: &mut [f32], b: &[f32], wa: f32, wb: f32) {
    for (o, &y) in out.iter_mut().zip(b) {
        *o = wa * *o + wb * y;
    }
}

/// `ff_adaptive_gain_control` in place.
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

/// `AMRFixed`: a sparse fixed-codebook vector.
#[derive(Clone, Copy, Default)]
pub(crate) struct Fixed {
    pub n: usize,
    pub x: [usize; 10],
    pub y: [f32; 10],
    pub pitch_lag: usize,
    pub pitch_fac: f32,
}

/// `ff_decode_10_pulses_35bits`.
pub(crate) fn decode_10_pulses_35bits(idx: &[i16], f: &mut Fixed, gray: &[u8], half: usize, bits: u32) {
    let mask = (1i32 << bits) - 1;
    f.n = 2 * half;
    for i in 0..half {
        let a = i32::from(idx[2 * i + 1]);
        let b = i32::from(idx[2 * i]);
        let pos1 = usize::from(gray.get((a & mask) as usize).copied().unwrap_or(0)) + i;
        let pos2 = usize::from(gray.get((b & mask) as usize).copied().unwrap_or(0)) + i;
        let sign = if a & (1 << bits) != 0 { -1.0f32 } else { 1.0 };
        f.x[2 * i + 1] = pos1;
        f.x[2 * i] = pos2;
        f.y[2 * i + 1] = sign;
        f.y[2 * i] = if pos2 < pos1 { -sign } else { sign };
    }
}

/// `ff_set_fixed_vector` (every pulse repeats at the pitch lag).
pub(crate) fn set_fixed_vector(out: &mut [f32], f: &Fixed, scale: f32) {
    for i in 0..f.n {
        let mut x = f.x[i];
        let mut y = f.y[i] * scale;
        if f.pitch_lag > 0 {
            while let Some(o) = out.get_mut(x) {
                *o += y;
                y *= f.pitch_fac;
                x += f.pitch_lag;
            }
        }
    }
}

/// `ff_set_min_dist_lsf`.
pub(crate) fn set_min_dist_lsf(lsf: &mut [f32], min_spacing: f64) {
    let mut prev = 0f32;
    for v in lsf {
        let b = f64::from(prev) + min_spacing;
        *v = if f64::from(*v) > b { *v } else { b as f32 };
        prev = *v;
    }
}

/// `ff_sort_nearly_sorted_floats`.
pub(crate) fn sort_nearly_sorted(v: &mut [f32]) {
    for i in 0..v.len().saturating_sub(1) {
        let mut j = i + 1;
        while j > 0 && v[j - 1] > v[j] {
            v.swap(j - 1, j);
            j -= 1;
        }
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

/// `ff_amrwb_lsp2lpc`.
pub(crate) fn amrwb_lsp2lpc(lsp: &[f64], lp: &mut [f32], order: usize) {
    let half = order >> 1;
    let mut pa = [0f64; 11];
    let mut buf = [0f64; 11];
    lsp2polyf(lsp, &mut pa, half);
    lsp2polyf(&lsp[1..], &mut buf[1..], half - 1);
    // qa[k] = buf[k + 1]; qa[-1] = 0
    let last = lsp[order - 1];
    let mut j = order - 1;
    for i in 1..half {
        let paf = pa[i] * (1.0 + last);
        let qaf = (buf[i + 1] - buf[i - 1]) * (1.0 - last);
        lp[i - 1] = ((paf + qaf) * 0.5) as f32;
        lp[j - 1] = ((paf - qaf) * 0.5) as f32;
        j -= 1;
    }
    lp[half - 1] = ((1.0 + last) * pa[half] * 0.5) as f32;
    lp[order - 1] = last as f32;
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

const PITCH_DELAY_MIN: i32 = 20;
const PITCH_DELAY_MAX: i32 = 143;

/// `ff_decode_pitch_lag` for 5- and 6-bit resolution: `(lag_int, lag_frac)`.
pub(crate) fn decode_pitch_lag(mut index: i32, prev_lag_int: i32, subframe: usize, third_as_first: bool, resolution: u32) -> (i32, i32) {
    if subframe == 0 || (subframe == 2 && third_as_first) {
        if index < 197 {
            index += 59;
        } else {
            index = 3 * index - 335;
        }
    } else {
        index -= 1;
        if resolution == 5 {
            index += 3 * (prev_lag_int - 10).clamp(PITCH_DELAY_MIN, PITCH_DELAY_MAX - 19);
        } else {
            index += 3 * (prev_lag_int - 5).clamp(PITCH_DELAY_MIN, PITCH_DELAY_MAX - 9);
        }
    }
    let lag = (index * 10923) >> 15;
    (lag, index - 3 * lag - 1)
}

/// `ff_amr_set_fixed_gain`.
pub(crate) fn amr_set_fixed_gain(factor: f32, mean_energy: f32, history: &mut [f32; 4], energy_mean: f32, pred: &[f32]) -> f32 {
    let e = if mean_energy != 0.0 { mean_energy } else { 1.0 };
    let x = 0.05 * f64::from(dot(pred, history) + energy_mean);
    let val = (f64::from(factor) * (std::f64::consts::LOG2_10 * x).exp2() / f64::from(e.sqrt())) as f32;
    history.copy_within(1..4, 0);
    history[3] = (20.0 * f64::from(factor.log10())) as f32;
    val
}
