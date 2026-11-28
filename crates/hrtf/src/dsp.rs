// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Sirac Ozmen
//
// Ported to Rust for CFA4 from Maul Audio (https://github.com/Rawframe-Project/maul-audio, commit
// aeb383458250759f92a3e724a5bd006505e45b4d, src/binaural_dsp.{h,c}). CFA4 changes: slices and
// offsets instead of pointers; reads outside a buffer give 0 instead of undefined behaviour.

//! The binaural effect's sample loops: a direct-form FIR, a fractional-delay read and a
//! first-order filter. They work in caller memory and allocate nothing.

/// `out[n] += sum_k h[k] x[at + n - k]`: `x` holds `h.len() - 1` samples of history before `at`.
/// Written tap by tap across the frames, which the compiler vectorizes; each output still adds
/// its taps in order.
pub(crate) fn fir_add(x: &[f32], at: usize, h: &[f32], out: &mut [f32]) {
    let frames = out.len();
    for (k, &c) in h.iter().enumerate() {
        let Some(start) = at.checked_sub(k) else { continue };
        let Some(input) = x.get(start..start + frames) else { continue };
        for (o, i) in out.iter_mut().zip(input) {
            *o += c * i;
        }
    }
}

/// `out[n] = sum_k h[k] x[at + n - k]`.
pub(crate) fn fir(x: &[f32], at: usize, h: &[f32], out: &mut [f32]) {
    out.fill(0.0);
    fir_add(x, at, h, out);
}

/// The cubic Lagrange weights of the points at offsets -1, 0, 1 and 2 samples from the read's
/// whole delay, for its fraction f.
fn weights(f: f32) -> [f32; 4] {
    [-f * (f - 1.0) * (f - 2.0) / 6.0, (f + 1.0) * (f - 1.0) * (f - 2.0) / 2.0, -(f + 1.0) * f * (f - 2.0) / 2.0, (f + 1.0) * f * (f - 1.0) / 6.0]
}

fn sample(x: &[f32], i: isize) -> f32 {
    if i < 0 { 0.0 } else { x.get(i as usize).copied().unwrap_or(0.0) }
}

/// `out[n]` = `x` read `start + step * n` samples before `x[at + n]`, by cubic Lagrange
/// interpolation. Every delay is at least 1, so the read never needs a sample after `x[at + n]`.
pub(crate) fn read_delayed(x: &[f32], at: usize, start: f32, step: f32, out: &mut [f32]) {
    for (n, o) in out.iter_mut().enumerate() {
        let delay = (start + step * n as f32).max(1.0);
        let whole = delay as usize;
        let c = weights(delay - whole as f32);
        let p = (at + n) as isize - whole as isize;
        *o = c[0] * sample(x, p + 1) + c[1] * sample(x, p) + c[2] * sample(x, p - 1) + c[3] * sample(x, p - 2);
    }
}

/// Keeps a recursive state out of the subnormal range.
pub(crate) fn flush(v: f32) -> f32 {
    if v.abs() < 1.0e-30 { 0.0 } else { v }
}

/// Filters `samples` in place through `y[n] = b0 x[n] + b1 x[n - 1] - a1 y[n - 1]`, the
/// coefficients {b0, b1, a1} being `start + step * (n + 0)` at sample n. `state` holds x[n - 1] and
/// y[n - 1]. Coefficients moving between two stable filters stay stable.
pub(crate) fn shelve(samples: &mut [f32], start: [f32; 3], step: [f32; 3], state: &mut [f32; 2]) {
    let [mut x1, mut y1] = *state;
    for (n, s) in samples.iter_mut().enumerate() {
        let t = n as f32;
        let b0 = start[0] + step[0] * t;
        let b1 = start[1] + step[1] * t;
        let a1 = start[2] + step[2] * t;
        let x = *s;
        let y = b0 * x + b1 * x1 - a1 * y1;
        *s = y;
        x1 = x;
        y1 = y;
    }
    *state = [x1, flush(y1)];
}
