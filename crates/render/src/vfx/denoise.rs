//! Temporal Denoise: averages each pixel with the same pixel in the neighbouring frames, so grain
//! and sensor noise that changes from frame to frame (the flicker of low-light video) settles,
//! while detail that stays put is kept at full resolution.
//!
//! Motion protection: a neighbour counts with weight `exp(-(d / threshold)²)`, where `d` is the
//! largest channel difference to the current frame in display space, averaged over a 3 × 3
//! neighbourhood (so single noisy pixels do not read as motion). Where something moved the
//! neighbours drop out and the pixel stays as it was, so there is no ghosting. Show Motion draws
//! the share of the neighbours that were rejected (white = moving).

use cfa4_project::EffectInstance;
use rayon::prelude::*;

use super::*;
use crate::effects::enc;

/// Frames read on each side at most.
pub(crate) const MAX_SIDE: usize = 4;

/// Display-space difference of two premultiplied linear images (largest channel), 3 × 3 averaged.
fn difference(a: &Image, b: &Image) -> Option<Image> {
    if a.w != b.w || a.h != b.h || a.px.len() != b.px.len() || a.px.len() != a.w.saturating_mul(a.h).saturating_mul(4) {
        return None;
    }
    let mut d = Image::new(a.w, a.h);
    if d.px.len() != a.px.len() {
        return None;
    }
    d.px.par_chunks_mut(4).zip(a.px.par_chunks(4).zip(b.px.par_chunks(4))).for_each(|(o, (p, q))| {
        let (ca, cb) = (enc(Image::unpremul([p[0], p[1], p[2], p[3]])), enc(Image::unpremul([q[0], q[1], q[2], q[3]])));
        let mut v = (p[3] - q[3]).abs();
        for k in 0..3 {
            v = v.max((ca[k] - cb[k]).abs());
        }
        let v = if v.is_finite() { v } else { 1.0 };
        o.copy_from_slice(&[v, v, v, 1.0]);
    });
    crate::effects::box_blur(&mut d, &[1], &[1], true);
    Some(d)
}

/// Temporal Denoise (see the module docs).
pub fn temporal_denoise(img: &mut Image, e: &EffectInstance, cx: &FxCtx<'_>) {
    let Some(env) = cx.env else { return };
    let amount = (fv(e, "amount", cx) / 100.0).clamp(0.0, 1.0);
    let show = bv(e, "show_motion");
    if amount <= 0.0 && !show {
        return;
    }
    let side = fv(e, "frames", cx);
    let side = if side.is_finite() { (side.round().max(1.0) as usize).min(MAX_SIDE) } else { 1 };
    let sigma = (fv(e, "threshold", cx) / 100.0).clamp(0.001, 1.0);
    let fps = env.frame_rate();
    let fps = if fps.is_finite() && fps > 0.0 { fps } else { 30.0 };
    let n = img.px.len();
    if n == 0 || n != img.w.saturating_mul(img.h).saturating_mul(4) {
        return;
    }
    // running weighted sums: colour (premultiplied, alpha included) and weight per pixel
    let mut sum = img.px.clone();
    let mut weight = vec![1.0f32; n / 4];
    let mut neighbours = 0usize;
    for k in 1..=side {
        for dir in [-1.0f64, 1.0] {
            let Some(f) = env.frame(dir * k as f64 / fps, e) else { continue };
            let Some(d) = difference(img, &f) else { continue };
            neighbours += 1;
            sum.par_chunks_mut(4).zip(weight.par_iter_mut()).zip(f.px.par_chunks(4).zip(d.px.par_chunks(4))).for_each(|((s, w), (q, dv))| {
                let r = dv[0] / sigma;
                let wk = (-r * r).exp();
                if wk > 1e-4 {
                    for c in 0..4 {
                        s[c] += q[c] * wk;
                    }
                    *w += wk;
                }
            });
        }
    }
    if neighbours == 0 {
        return;
    }
    let m = neighbours as f32;
    img.px.par_chunks_mut(4).zip(sum.par_chunks(4).zip(weight.par_iter())).for_each(|(p, (s, w))| {
        if show {
            // share of the neighbours rejected as motion
            let v = crate::effects::dec([(1.0 - (w - 1.0) / m).clamp(0.0, 1.0); 3])[0];
            p.copy_from_slice(&[v, v, v, 1.0]);
            return;
        }
        let w = w.max(1.0);
        for c in 0..4 {
            let filtered = s[c] / w;
            p[c] += (filtered - p[c]) * amount;
        }
    });
}
