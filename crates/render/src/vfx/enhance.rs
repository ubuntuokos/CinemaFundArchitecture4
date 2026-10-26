//! Enhance: retouch effects outside Premiere's bin (CFA4's own folder), on top of the same
//! helpers as the rest of `vfx`. Colour maths runs on sRGB-encoded straight colour, like the keys
//! and limiters: the mask thresholds artists set are display-space numbers. Low Light, the
//! folder's other effect, also runs on the GPU, so it lives in `gpufx` (`FxOp::LowLight`).

use cfa4_project::EffectInstance;
use rayon::prelude::*;

use super::*;
use crate::effects::gaussian;

/// Rec. 601 chroma (Cb, Cr) of an sRGB-encoded colour. Skin sits in a small, well-known patch of
/// this plane whatever the exposure, so the mask is a distance here and ignores brightness.
#[inline]
fn chroma(c: [f32; 3]) -> (f32, f32) {
    let y = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    (0.564 * (c[2] - y), 0.713 * (c[0] - y))
}

/// Pack `n` RGB values (plus an unused fourth channel) into an image [`gaussian`] can blur.
fn plane(w: usize, h: usize, f: impl Fn(usize) -> [f32; 3] + Sync) -> Image {
    let mut img = Image::new(w, h);
    img.px.par_chunks_mut(4).enumerate().for_each(|(i, p)| {
        let v = f(i);
        p[..3].copy_from_slice(&v);
    });
    img
}

/// Edge-preserving smoothing of `rgb` (straight sRGB) with the guided filter (He, Sun & Tang
/// 2013): per-window linear models `a · guide + b` of the luma guide, their coefficients averaged.
/// Only means are needed, so a box-Gaussian of any radius costs the same, and edges survive
/// because a window straddling one has a high variance and keeps `a` near 1.
///
/// `sigma` is the window size in pixels and `eps` the luma variance (display 0..1, squared) below
/// which a window counts as flat and is smoothed. `eps` is strictly positive, so the division is
/// safe.
fn guided_smooth(w: usize, h: usize, rgb: &[[f32; 3]], sigma: f32, eps: f32) -> Vec<[f32; 3]> {
    let eps = eps.max(1e-6);
    let guide = |c: [f32; 3]| 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    let g = |i: usize| rgb.get(i).map_or(0.0, |c| guide(*c));
    let c = |i: usize| rgb.get(i).copied().unwrap_or([0.0; 3]);
    // means of the guide and its square (first two channels), of the input, and of guide × input
    let mut m_g = plane(w, h, |i| [g(i), g(i) * g(i), 0.0]);
    let mut m_p = plane(w, h, c);
    let mut m_gp = plane(w, h, |i| {
        let (g, c) = (g(i), c(i));
        [c[0] * g, c[1] * g, c[2] * g]
    });
    for img in [&mut m_g, &mut m_p, &mut m_gp] {
        gaussian(img, sigma, sigma, true);
    }
    let at = |img: &Image, i: usize| -> [f32; 3] {
        let p = img.px.get(i * 4..i * 4 + 3);
        p.map_or([0.0; 3], |p| [p[0], p[1], p[2]])
    };
    // per-pixel linear model, then the averaged coefficients
    let coeffs: Vec<([f32; 3], [f32; 3])> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let mg = at(&m_g, i);
            let var = (mg[1] - mg[0] * mg[0]).max(0.0);
            let (mp, mgp) = (at(&m_p, i), at(&m_gp, i));
            let mut a = [0.0; 3];
            let mut b = [0.0; 3];
            for k in 0..3 {
                a[k] = (mgp[k] - mg[0] * mp[k]) / (var + eps);
                b[k] = mp[k] - a[k] * mg[0];
            }
            (a, b)
        })
        .collect();
    let mut m_a = plane(w, h, |i| coeffs.get(i).map_or([0.0; 3], |c| c.0));
    let mut m_b = plane(w, h, |i| coeffs.get(i).map_or([0.0; 3], |c| c.1));
    for img in [&mut m_a, &mut m_b] {
        gaussian(img, sigma, sigma, true);
    }
    (0..w * h)
        .map(|i| {
            let (a, b) = (at(&m_a, i), at(&m_b, i));
            let g = g(i);
            [a[0] * g + b[0], a[1] * g + b[1], a[2] * g + b[2]]
        })
        .collect()
}

/// Skin Smoothing: an edge-preserving smooth of the skin tones only, with the fine texture mixed
/// back so the result is not plastic.
///
/// The mask is a distance from the Skin Tone colour in the Cb/Cr plane (Tolerance, Softness);
/// Smooth Entire Image drops it and makes this a surface blur. Inside the mask the picture is
/// split into the guided-filter result and what it removed, and Keep Texture decides how much of
/// that detail returns (100 % is a no-op, the classic frequency-separation retouch).
pub fn skin_smoothing(img: &mut Image, e: &EffectInstance, cx: &FxCtx) {
    let amount = (fv(e, "amount", cx) / 100.0).clamp(0.0, 1.0);
    let show = bv(e, "show_mask");
    if (amount <= 0.0 && !show) || img.px.is_empty() {
        return;
    }
    let radius = (fv(e, "radius", cx) * cx.px_scale).clamp(0.0, 500.0);
    let keep = (fv(e, "texture", cx) / 100.0).clamp(0.0, 1.0);
    let smooth = (fv(e, "smoothness", cx) / 100.0).clamp(0.0, 1.0);
    let whole = bv(e, "whole_image");
    let tone = enc(lin(cv(e, "skin_tone", cx)));
    let (cb0, cr0) = chroma(tone);
    let tol = (fv(e, "tolerance", cx) / 100.0).clamp(0.0, 1.0) * 0.3;
    let soft = (fv(e, "softness", cx) / 100.0).clamp(0.0, 1.0) * 0.3;
    let (w, h) = (img.w, img.h);
    // straight sRGB source, and the mask that decides where the smoothing applies
    let src: Vec<[f32; 3]> = img.px.as_chunks::<4>().0.par_iter().map(|p| enc(Image::unpremul(*p))).collect();
    let mask: Vec<f32> = if whole {
        vec![1.0; src.len()]
    } else {
        src.par_iter()
            .map(|c| {
                let (cb, cr) = chroma(*c);
                let d = ((cb - cb0) * (cb - cb0) + (cr - cr0) * (cr - cr0)).sqrt();
                1.0 - smoothstep(tol, tol + soft.max(1e-4), d)
            })
            .collect()
    };
    if show {
        img.px.par_chunks_mut(4).zip(mask.par_iter()).for_each(|(p, m)| {
            let l = dec([*m, *m, *m]);
            for k in 0..3 {
                p[k] = l[k] * p[3];
            }
        });
        return;
    }
    // smoothing radii below half a pixel do nothing, so only the mask would be left to apply
    if radius < 0.5 || keep >= 1.0 {
        return;
    }
    // Smoothness is the luma standard deviation (in display 0..1) still counted as flat skin:
    // 2 % at 0 (only sensor noise), 20 % at 100 (everything but a hard edge).
    let sd = 0.02 + 0.18 * smooth;
    let eps = sd * sd;
    let smoothed = guided_smooth(w, h, &src, radius, eps);
    img.px.par_chunks_mut(4).enumerate().for_each(|(i, p)| {
        let Some(((orig, sm), m)) = src.get(i).zip(smoothed.get(i)).zip(mask.get(i)) else {
            return;
        };
        let k = amount * m;
        if k <= 0.0 {
            return;
        }
        let mut out = [0.0; 3];
        for c in 0..3 {
            // the guided result plus the detail it removed, scaled by Keep Texture
            let retouched = sm[c] + (orig[c] - sm[c]) * keep;
            out[c] = orig[c] + (retouched - orig[c]) * k;
        }
        let l = dec(out);
        for c in 0..3 {
            p[c] = l[c] * p[3];
        }
    });
}
