// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-color 0.11.0.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

#[inline]
pub fn eval_generic<const P: usize, const Q: usize>(x: f32, p: [f32; P], q: [f32; Q]) -> f32 {
    let yp = p.into_iter().rev().reduce(|yp, p| yp * x + p).unwrap_or(0.0);
    let yq = q.into_iter().rev().reduce(|yq, q| yq * x + q).unwrap_or(1.0);
    yp / yq
}
