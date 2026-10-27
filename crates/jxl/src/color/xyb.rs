// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-color 0.11.0.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

pub(crate) fn run(xyb: [&mut [f32]; 3], ob: [f32; 3], intensity_target: f32) {
    run_generic(xyb, ob, intensity_target)
}

#[inline(always)]
fn run_generic(xyb: [&mut [f32]; 3], ob: [f32; 3], intensity_target: f32) {
    let itscale = 255.0 / intensity_target;

    let [x, y, b] = xyb;
    if x.len() != y.len() || y.len() != b.len() {
        return;
    }
    let cbrt_ob = ob.map(|v| v.cbrt());

    for ((x, y), b) in x.iter_mut().zip(&mut *y).zip(&mut *b) {
        // matrix: [1, 1, 0, -1, 1, 0, 0, 0, 1]
        let g_l = *y + *x;
        let g_m = *y - *x;
        let g_s = *b;

        // bias: -cbrt_ob
        let g_l = g_l - cbrt_ob[0];
        let g_m = g_m - cbrt_ob[1];
        let g_s = g_s - cbrt_ob[2];

        // inverse tf: gamma3, bias: ob, matrix: id * itscale
        *x = (g_l * g_l).mul_add(g_l, ob[0]) * itscale;
        *y = (g_m * g_m).mul_add(g_m, ob[1]) * itscale;
        *b = (g_s * g_s).mul_add(g_s, ob[2]) * itscale;
    }
}
