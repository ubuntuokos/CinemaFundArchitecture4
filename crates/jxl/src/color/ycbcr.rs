// Ported from jxl-oxide <https://github.com/tirr-c/jxl-oxide>, crate jxl-color 0.11.0.
// Copyright (c) Wonwoo Choi and the jxl-oxide contributors; MIT OR Apache-2.0 (see
// LICENSE-MIT, LICENSE-APACHE). CFA4 changes (no unsafe, errors instead of panics, safe
// subgrids) are GPL-3.0-or-later.

/// Applies transform from YCbCr to RGB.
///
/// Channels are expected to be in CbYCr order.
pub fn ycbcr_to_rgb(fb_cbycr: [&mut [f32]; 3]) {
    let [cb, y, cr] = fb_cbycr;

    run_generic([cb, y, cr])
}

#[inline(always)]
fn run_generic([cb, y, cr]: [&mut [f32]; 3]) {
    if cb.len() != y.len() || y.len() != cr.len() {
        return;
    }
    for ((r, g), b) in cb.iter_mut().zip(y).zip(cr) {
        let cb = *r;
        let y = *g + 128.0 / 255.0;
        let cr = *b;

        *r = cr.mul_add(1.402, y);
        *g = cb.mul_add(-0.114 * 1.772 / 0.587, cr.mul_add(-0.299 * 1.402 / 0.587, y));
        *b = cb.mul_add(1.772, y);
    }
}
