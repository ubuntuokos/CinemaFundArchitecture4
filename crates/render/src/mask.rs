//! Mask coverage (CPU reference; `cfa4_gpu::GpuMask` runs the same math in WGSL).
//!
//! Each [`Mask`] is flattened to a polygon in working-image pixels (chord error ≤ 0.05 px). For
//! a pixel centre `p` the signed distance `sd` to the polygon (positive inside, nonzero winding)
//! gives the coverage
//!
//! ```text
//! s  = sd + expansion
//! w  = max(feather, 1)
//! u  = clamp(s / w + 0.5, 0, 1)
//! c  = lerp(u, smoothstep(u), min(feather, 1))
//! m  = opacity · (inverted ? 1 − c : c)
//! ```
//!
//! so a hard mask (feather 0) is the analytic box-filtered coverage of the edge (a one-pixel
//! linear ramp across the boundary: exact for straight edges, antialiased everywhere), and a
//! feathered mask falls off smoothly over `feather` pixels centred on the (expanded) edge — a true
//! distance falloff, so corners round off and the width is the same along the whole contour.
//! Masks combine top to bottom with their [`MaskMode`]. All of this runs in linear light on the
//! premultiplied working image: a masked effect is `lerp(original, effected, coverage)` per
//! channel, an opacity mask scales the layer's premultiplied pixels.

use cfa4_project::{EffectInstance, Mask, MaskMode};
use cfa4_time::Tick;
use rayon::prelude::*;

use crate::image::Image;

/// Flattening tolerance in working pixels.
pub const FLATTEN_TOLERANCE: f64 = 0.05;

/// One mask ready for rasterizing, in working-image pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct FlatMask {
    /// Closed polygon (last vertex connects to the first).
    pub pts: Vec<[f32; 2]>,
    pub feather: f32,
    pub expansion: f32,
    /// 0..1.
    pub opacity: f32,
    pub inverted: bool,
    pub mode: MaskMode,
}

/// Flatten the active masks (mode ≠ None) at clip time `t`. `px_scale` = working pixels per clip pixel.
pub fn prepare(masks: &[Mask], t: Tick, px_scale: f32) -> Vec<FlatMask> {
    let s = px_scale as f64;
    masks
        .iter()
        .filter(|m| m.mode != MaskMode::None)
        .map(|m| {
            let path = m.path_at(t);
            let pts = path.flatten(FLATTEN_TOLERANCE / s.max(1e-6)).into_iter().map(|p| [(p.x * s) as f32, (p.y * s) as f32]).collect();
            FlatMask {
                pts,
                feather: (m.feather.f64_at(t).max(0.0) * s) as f32,
                expansion: (m.expansion.f64_at(t) * s) as f32,
                opacity: (m.opacity.f64_at(t) / 100.0).clamp(0.0, 1.0) as f32,
                inverted: m.inverted,
                mode: m.mode,
            }
        })
        .collect()
}

/// The falloff of a signed (expanded) distance for a feather width.
#[inline]
pub fn falloff(s: f32, feather: f32) -> f32 {
    let w = feather.max(1.0);
    let u = (s / w + 0.5).clamp(0.0, 1.0);
    let smooth = u * u * (3.0 - 2.0 * u);
    u + (smooth - u) * feather.min(1.0)
}

/// Distance band beyond which coverage is saturated (0 or 1).
fn band(m: &FlatMask) -> f32 {
    m.expansion.abs() + m.feather.max(1.0) * 0.5 + 1.0
}

#[inline]
fn seg_dist2(px: f32, py: f32, a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let (qx, qy) = (px - a[0], py - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { ((qx * dx + qy * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let (ex, ey) = (qx - dx * t, qy - dy * t);
    ex * ex + ey * ey
}

/// Coverage of one mask for row `y` into `out` (length = width). Reference per-row algorithm:
/// winding from the row's sorted edge crossings, distance only against edges within the band.
fn mask_row(m: &FlatMask, y: usize, out: &mut [f32]) {
    let n = m.pts.len();
    let py = y as f32 + 0.5;
    let b = band(m);
    let value = |sd: f32| {
        let c = falloff(sd + m.expansion, m.feather);
        m.opacity * if m.inverted { 1.0 - c } else { c }
    };
    if n < 3 {
        out.fill(value(-b));
        return;
    }
    // crossings of the scanline with the polygon edges (x, direction)
    let mut cross: Vec<(f32, i32)> = Vec::new();
    // edges within the band of this row: (a, b, min x, max x)
    let mut near: Vec<([f32; 2], [f32; 2], f32, f32)> = Vec::new();
    for i in 0..n {
        let a = m.pts[i];
        let c = m.pts[(i + 1) % n];
        if (a[1] <= py) != (c[1] <= py) {
            let t = (py - a[1]) / (c[1] - a[1]);
            cross.push((a[0] + (c[0] - a[0]) * t, if c[1] > a[1] { 1 } else { -1 }));
        }
        if py >= a[1].min(c[1]) - b && py <= a[1].max(c[1]) + b {
            near.push((a, c, a[0].min(c[0]) - b, a[0].max(c[0]) + b));
        }
    }
    cross.sort_by(|p, q| p.0.total_cmp(&q.0));
    let mut ci = 0;
    let mut wind = 0;
    let b2 = b * b;
    for (x, o) in out.iter_mut().enumerate() {
        let px = x as f32 + 0.5;
        while ci < cross.len() && cross[ci].0 < px {
            wind += cross[ci].1;
            ci += 1;
        }
        let inside = wind != 0;
        let mut d2 = b2;
        for (a, c, x0, x1) in &near {
            if px >= *x0 && px <= *x1 {
                d2 = d2.min(seg_dist2(px, py, *a, *c));
            }
        }
        let d = d2.sqrt();
        *o = value(if inside { d } else { -d });
    }
}

/// Combined coverage of `masks` over a `w`×`h` image, or `None` when there is no active mask
/// (the effect applies everywhere).
pub fn coverage(masks: &[FlatMask], w: usize, h: usize) -> Option<Vec<f32>> {
    let first = masks.iter().find(|m| m.mode != MaskMode::None)?;
    let start = first.mode.start();
    let mut acc = vec![start; w * h];
    if w == 0 {
        return Some(acc);
    }
    acc.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let mut tmp = vec![0.0f32; w];
        for m in masks.iter().filter(|m| m.mode != MaskMode::None) {
            mask_row(m, y, &mut tmp);
            for (a, v) in row.iter_mut().zip(&tmp) {
                *a = m.mode.combine(*a, *v).clamp(0.0, 1.0);
            }
        }
    });
    Some(acc)
}

/// Coverage of an effect's masks on a working image (`px_scale` working px per clip px).
pub fn effect_coverage(masks: &[Mask], t: Tick, px_scale: f32, w: usize, h: usize) -> Option<Vec<f32>> {
    if masks.is_empty() {
        return None;
    }
    coverage(&prepare(masks, t, px_scale), w, h)
}

/// A tracked object's mask for one frame, ready to rasterize (OM-A3).
///
/// `src` is the binary mask (0 = outside, anything else = inside) of `src_w`×`src_h` pixels, as
/// `cfa4_project::object::MaskRle::decode` gives it. `feather`, `expansion` (working pixels,
/// negative shrinks), `opacity` (0..1) and `inverted` mean what they do for a drawn mask.
#[derive(Clone, Copy, Debug)]
pub struct ObjectRaster<'a> {
    pub src: &'a [u8],
    pub src_w: usize,
    pub src_h: usize,
    pub feather: f32,
    pub expansion: f32,
    pub opacity: f32,
    pub inverted: bool,
}

/// Squared Euclidean distance from every cell to the nearest cell where `seed` is true
/// (Felzenszwalb–Huttenlocher; exact). Cells with no seed anywhere get [`EDT_FAR`].
const EDT_FAR: f32 = 1.0e12;

fn edt_1d(f: &[f32], d: &mut [f32], v: &mut [usize], z: &mut [f32]) {
    let n = f.len();
    if n == 0 || v.len() < n || z.len() < n + 1 || d.len() < n {
        return;
    }
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f32::NEG_INFINITY;
    z[1] = f32::INFINITY;
    for q in 1..n {
        let s = loop {
            let r = v[k];
            let s = ((f[q] + (q * q) as f32) - (f[r] + (r * r) as f32)) / (2.0 * (q as f32 - r as f32));
            if k > 0 && s <= z[k] {
                k -= 1;
            } else {
                break s;
            }
        };
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f32::INFINITY;
    }
    k = 0;
    for (q, out) in d.iter_mut().enumerate().take(n) {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let r = v[k];
        let dq = q as f32 - r as f32;
        *out = dq * dq + f[r];
    }
}

fn edt(seed: &[bool], w: usize, h: usize) -> Vec<f32> {
    let mut g: Vec<f32> = seed.iter().map(|&s| if s { 0.0 } else { EDT_FAR }).collect();
    let n = w.max(h);
    let (mut v, mut z) = (vec![0usize; n], vec![0.0f32; n + 1]);
    let mut col = vec![0.0f32; h];
    let mut out = vec![0.0f32; h];
    for x in 0..w {
        for y in 0..h {
            col[y] = g[y * w + x];
        }
        edt_1d(&col, &mut out, &mut v, &mut z);
        for y in 0..h {
            g[y * w + x] = out[y];
        }
    }
    let mut row_out = vec![0.0f32; w];
    for y in 0..h {
        let row = &g[y * w..(y + 1) * w];
        edt_1d(row, &mut row_out, &mut v, &mut z);
        g[y * w..(y + 1) * w].copy_from_slice(&row_out);
    }
    g
}

/// Coverage of an object mask over a `w`×`h` working image, or `None` when the inputs do not
/// describe one (empty size, `src` of the wrong length, sizes whose product overflows or is
/// unreasonably large).
///
/// The mask is resized to `w`×`h` (nearest cell centre; the result is a binary mask), then its
/// signed distance `sd` to the boundary (positive inside, in working pixels, half a pixel at the
/// edge cells) goes through the same falloff as a drawn mask, `falloff(sd + expansion, feather)`,
/// so expansion, feather and inversion behave identically for both kinds of mask.
pub fn object_coverage(o: &ObjectRaster<'_>, w: usize, h: usize) -> Option<Vec<f32>> {
    const MAX_PIXELS: usize = 1 << 27;
    let src_len = o.src_w.checked_mul(o.src_h)?;
    let dst_len = w.checked_mul(h)?;
    if src_len == 0 || dst_len == 0 || src_len != o.src.len() || src_len > MAX_PIXELS || dst_len > MAX_PIXELS {
        return None;
    }
    let (feather, expansion) = (if o.feather.is_finite() { o.feather.max(0.0) } else { 0.0 }, if o.expansion.is_finite() { o.expansion } else { 0.0 });
    let opacity = if o.opacity.is_finite() { o.opacity.clamp(0.0, 1.0) } else { 1.0 };
    let inside: Vec<bool> = (0..dst_len)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let sx = (((x as f64 + 0.5) * o.src_w as f64 / w as f64) as usize).min(o.src_w - 1);
            let sy = (((y as f64 + 0.5) * o.src_h as f64 / h as f64) as usize).min(o.src_h - 1);
            o.src.get(sy * o.src_w + sx).is_some_and(|&b| b != 0)
        })
        .collect();
    let outside: Vec<bool> = inside.iter().map(|&b| !b).collect();
    let d_to_outside = edt(&outside, w, h);
    let d_to_inside = edt(&inside, w, h);
    let value = |c: f32| opacity * if o.inverted { 1.0 - c } else { c };
    let band = expansion.abs() + feather.max(1.0) * 0.5 + 1.0;
    Some(
        inside
            .iter()
            .zip(d_to_outside.iter().zip(&d_to_inside))
            .map(|(&ins, (&dout, &din))| {
                let sd = if ins { dout.sqrt() - 0.5 } else { -(din.sqrt() - 0.5) };
                value(falloff(sd.clamp(-band - 1.0, band + 1.0) + expansion, feather))
            })
            .collect(),
    )
}

/// `img = lerp(original, img, cov)` on premultiplied pixels.
pub fn mix(img: &mut Image, original: &Image, cov: &[f32]) {
    img.px.par_chunks_mut(4).zip(original.px.par_chunks(4)).zip(cov.par_iter()).for_each(|((p, o), &c)| {
        for k in 0..4 {
            p[k] = o[k] + (p[k] - o[k]) * c;
        }
    });
}

/// Scale premultiplied pixels by coverage (opacity masks).
pub fn scale_by(img: &mut Image, cov: &[f32]) {
    img.px.par_chunks_mut(4).zip(cov.par_iter()).for_each(|(p, &c)| {
        for v in p {
            *v *= c;
        }
    });
}

/// Apply an effect limited to its masks (Premiere: a masked effect only applies inside the mask).
pub fn apply_effect(img: &mut Image, e: &EffectInstance, cx: &crate::effects::FxCtx<'_>) {
    if !e.enabled || e.masks.is_empty() {
        crate::effects::apply(img, e, cx);
        return;
    }
    let Some(cov) = effect_coverage(&e.masks, cx.t, cx.px_scale, img.w, img.h) else {
        crate::effects::apply(img, e, cx);
        return;
    };
    let original = img.clone();
    crate::effects::apply(img, e, cx);
    mix(img, &original, &cov);
}

/// Apply the masks of an item's Opacity effect to its layer (before Motion), if any.
pub fn apply_opacity_masks(img: &mut Image, item: &cfa4_project::TrackItem, t: Tick, px_scale: f32) {
    let Some(op) = item.effect("opacity").filter(|e| e.enabled && !e.masks.is_empty()) else { return };
    if let Some(cov) = effect_coverage(&op.masks, t, px_scale, img.w, img.h) {
        scale_by(img, &cov);
    }
}

/// Brute-force coverage of one pixel (every edge; the definition the row algorithm and the WGSL
/// shader implement). For tests and documentation.
pub fn coverage_at(masks: &[FlatMask], x: f32, y: f32) -> Option<f32> {
    let first = masks.iter().find(|m| m.mode != MaskMode::None)?;
    let mut acc = first.mode.start();
    for m in masks.iter().filter(|m| m.mode != MaskMode::None) {
        let n = m.pts.len();
        let mut wind = 0;
        let mut d2 = f32::INFINITY;
        for i in 0..n {
            let a = m.pts[i];
            let c = m.pts[(i + 1) % n];
            if (a[1] <= y) != (c[1] <= y) {
                let t = (y - a[1]) / (c[1] - a[1]);
                if a[0] + (c[0] - a[0]) * t < x {
                    wind += if c[1] > a[1] { 1 } else { -1 };
                }
            }
            d2 = d2.min(seg_dist2(x, y, a, c));
        }
        let d = if n < 3 { band(m) } else { d2.sqrt().min(band(m)) };
        let sd = if wind != 0 && n >= 3 { d } else { -d };
        let c = falloff(sd + m.expansion, m.feather);
        let v = m.opacity * if m.inverted { 1.0 - c } else { c };
        acc = m.mode.combine(acc, v).clamp(0.0, 1.0);
    }
    Some(acc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfa4_geom::Vec2;
    use cfa4_project::MaskPath;

    fn mask(path: MaskPath, feather: f64) -> Mask {
        let mut m = Mask::new("m", path);
        m.feather.value = cfa4_project::ParamValue::Float(feather);
        m
    }

    #[test]
    fn hard_rect_is_exact_box_coverage() {
        // rect edges on pixel boundaries → exactly 1 inside, 0 outside; half-pixel edge → 0.5
        let m = mask(MaskPath::rect(2.0, 2.0, 6.5, 6.0), 0.0);
        let c = effect_coverage(&[m], Tick(0), 1.0, 10, 10).unwrap();
        assert_eq!(c[3 * 10 + 3], 1.0);
        assert_eq!(c[3 * 10 + 1], 0.0);
        assert!((c[3 * 10 + 6] - 0.5).abs() < 1e-6, "{}", c[3 * 10 + 6]);
        assert_eq!(c[8 * 10 + 3], 0.0);
    }

    #[test]
    fn ellipse_area_matches_analytic() {
        let (rx, ry) = (37.3, 21.7);
        let m = mask(MaskPath::ellipse(Vec2::new(50.2, 40.1), Vec2::new(rx, ry)), 0.0);
        let c = effect_coverage(&[m], Tick(0), 1.0, 100, 80).unwrap();
        let area: f64 = c.iter().map(|v| *v as f64).sum();
        let exact = std::f64::consts::PI * rx * ry;
        assert!((area - exact).abs() / exact < 0.002, "{area} vs {exact}");
    }

    #[test]
    fn feather_is_a_symmetric_distance_falloff() {
        let m = mask(MaskPath::rect(20.0, -100.0, 200.0, 200.0), 10.0);
        let c = effect_coverage(&[m], Tick(0), 1.0, 60, 4).unwrap();
        // edge at x = 20: 0.5 there, saturated 5 px away, symmetric
        let at = |x: usize| c[x];
        assert!((at(19) + at(20) - 1.0).abs() < 1e-5, "{} {}", at(19), at(20));
        assert!(at(13) < 1e-6 && at(26) > 1.0 - 1e-6, "{} {}", at(13), at(26));
        for x in 14..26 {
            assert!(at(x + 1) >= at(x), "monotonic");
        }
    }

    #[test]
    fn expansion_inverted_opacity_modes() {
        let mut m = mask(MaskPath::rect(10.0, 10.0, 20.0, 20.0), 0.0);
        m.expansion.value = cfa4_project::ParamValue::Float(5.0);
        let c = effect_coverage(std::slice::from_ref(&m), Tick(0), 1.0, 30, 30).unwrap();
        assert_eq!(c[15 * 30 + 6], 1.0, "expanded by 5");
        assert_eq!(c[15 * 30 + 4], 0.0);
        m.inverted = true;
        m.opacity.value = cfa4_project::ParamValue::Float(50.0);
        let c = effect_coverage(&[m.clone()], Tick(0), 1.0, 30, 30).unwrap();
        assert_eq!(c[15 * 30 + 15], 0.0);
        assert_eq!(c[2], 0.5);
        // subtract a hole from a big rect
        let mut big = mask(MaskPath::rect(0.0, 0.0, 30.0, 30.0), 0.0);
        big.mode = MaskMode::Add;
        let mut hole = mask(MaskPath::rect(10.0, 10.0, 20.0, 20.0), 0.0);
        hole.mode = MaskMode::Subtract;
        let c = effect_coverage(&[big, hole], Tick(0), 1.0, 30, 30).unwrap();
        assert_eq!(c[15 * 30 + 15], 0.0);
        assert_eq!(c[5 * 30 + 5], 1.0);
    }

    #[test]
    fn row_algorithm_matches_brute_force() {
        let mut a = mask(MaskPath::ellipse(Vec2::new(30.0, 25.0), Vec2::new(20.0, 12.0)), 6.0);
        a.expansion.value = cfa4_project::ParamValue::Float(-2.0);
        let mut b = mask(MaskPath::polygon(&[Vec2::new(5.0, 5.0), Vec2::new(50.0, 10.0), Vec2::new(20.0, 45.0)]), 0.0);
        b.mode = MaskMode::Difference;
        b.inverted = true;
        let flat = prepare(&[a, b], Tick(0), 0.8);
        let (w, h) = (48, 40);
        let c = coverage(&flat, w, h).unwrap();
        for y in 0..h {
            for x in 0..w {
                let r = coverage_at(&flat, x as f32 + 0.5, y as f32 + 0.5).unwrap();
                assert!((r - c[y * w + x]).abs() < 1e-5, "({x},{y}) {r} vs {}", c[y * w + x]);
            }
        }
    }

    // ---- OM-A3: object masks ----

    fn rect_src(w: usize, h: usize, x0: usize, y0: usize, x1: usize, y1: usize) -> Vec<u8> {
        (0..w * h).map(|i| u8::from((x0..x1).contains(&(i % w)) && (y0..y1).contains(&(i / w))) * 255).collect()
    }

    fn raster<'a>(src: &'a [u8], sw: usize, sh: usize) -> ObjectRaster<'a> {
        ObjectRaster { src, src_w: sw, src_h: sh, feather: 0.0, expansion: 0.0, opacity: 1.0, inverted: false }
    }

    #[test]
    fn object_mask_same_size_hard_edge_is_binary() {
        let src = rect_src(12, 10, 3, 2, 8, 7);
        let c = object_coverage(&raster(&src, 12, 10), 12, 10).unwrap();
        for (i, v) in c.iter().enumerate() {
            let inside = (3..8).contains(&(i % 12)) && (2..7).contains(&(i / 12));
            assert_eq!(*v, if inside { 1.0 } else { 0.0 }, "pixel {i}");
        }
    }

    #[test]
    fn object_mask_expansion_grows_and_shrinks_by_whole_pixels() {
        let src = rect_src(20, 20, 8, 8, 12, 12);
        let mut o = raster(&src, 20, 20);
        o.expansion = 2.0;
        let c = object_coverage(&o, 20, 20).unwrap();
        // 4x4 box grown by 2 on each side: columns/rows 6..14
        for y in 0..20 {
            for x in 0..20 {
                let inside = (6..14).contains(&x) && (6..14).contains(&y);
                // the corners round off (a distance falloff), so test the straight edges only
                if (6..14).contains(&x) && (8..12).contains(&y) || (8..12).contains(&x) && (6..14).contains(&y) {
                    assert_eq!(c[y * 20 + x], 1.0, "({x},{y})");
                } else if !inside {
                    assert!(c[y * 20 + x] < 1.0);
                }
            }
        }
        assert_eq!(c[10 * 20 + 5], 0.0);
        o.expansion = -1.0;
        let c = object_coverage(&o, 20, 20).unwrap();
        assert_eq!(c[10 * 20 + 8], 0.0, "the edge cell is eroded");
        assert_eq!(c[10 * 20 + 9], 1.0);
    }

    #[test]
    fn object_mask_resizes_to_the_working_image() {
        // left half of an 8x8 mask → left half of a 4x4 image, and of a 16x16 one
        let src = rect_src(8, 8, 0, 0, 4, 8);
        for n in [4usize, 16] {
            let c = object_coverage(&raster(&src, 8, 8), n, n).unwrap();
            for y in 0..n {
                for x in 0..n {
                    assert_eq!(c[y * n + x], if x < n / 2 { 1.0 } else { 0.0 }, "{n}: ({x},{y})");
                }
            }
        }
    }

    #[test]
    fn object_mask_opacity_and_inversion() {
        let src = rect_src(6, 6, 2, 2, 4, 4);
        let mut o = raster(&src, 6, 6);
        o.opacity = 0.5;
        let c = object_coverage(&o, 6, 6).unwrap();
        assert_eq!((c[2 * 6 + 2], c[0]), (0.5, 0.0));
        o.inverted = true;
        let c = object_coverage(&o, 6, 6).unwrap();
        assert_eq!((c[2 * 6 + 2], c[0]), (0.0, 0.5));
    }

    #[test]
    fn object_mask_feather_is_a_monotone_distance_falloff_matching_a_drawn_rect() {
        // the same box as a drawn rectangle: raster and path coverage agree within a fraction of a level
        let (w, h) = (40usize, 30usize);
        let src = rect_src(w, h, 10, 8, 30, 22);
        let mut o = raster(&src, w, h);
        o.feather = 6.0;
        let c = object_coverage(&o, w, h).unwrap();
        let flat = FlatMask {
            pts: vec![[10.0, 8.0], [30.0, 8.0], [30.0, 22.0], [10.0, 22.0]],
            feather: 6.0,
            expansion: 0.0,
            opacity: 1.0,
            inverted: false,
            mode: MaskMode::Add,
        };
        let mut worst = 0.0f32;
        for y in 0..h {
            for x in 0..w {
                let want = coverage_at(std::slice::from_ref(&flat), x as f32 + 0.5, y as f32 + 0.5).unwrap();
                worst = worst.max((c[y * w + x] - want).abs());
            }
        }
        assert!(worst < 0.12, "worst difference {worst}");
        // monotone away from the edge along a row through the middle
        let row = &c[15 * w..16 * w];
        assert!(row[..20].windows(2).all(|p| p[0] <= p[1]));
    }

    #[test]
    fn object_mask_degenerate_inputs_are_refused_or_safe() {
        let src = rect_src(4, 4, 1, 1, 3, 3);
        let o = raster(&src, 4, 4);
        assert!(object_coverage(&o, 0, 4).is_none());
        assert!(object_coverage(&o, 4, 0).is_none());
        assert!(object_coverage(&o, usize::MAX, 2).is_none(), "size overflow");
        assert!(object_coverage(&o, 1 << 20, 1 << 20).is_none(), "too large");
        assert!(object_coverage(&raster(&src, 4, 3), 4, 4).is_none(), "length mismatch");
        assert!(object_coverage(&raster(&[], 0, 0), 4, 4).is_none());
        // all outside, all inside, hostile numbers
        let none = vec![0u8; 16];
        let all = vec![255u8; 16];
        for (src, want) in [(&none, 0.0), (&all, 1.0)] {
            let mut o = raster(src, 4, 4);
            o.feather = 5.0;
            let c = object_coverage(&o, 4, 4).unwrap();
            assert!(c.iter().all(|v| (*v - want).abs() < 1e-6), "{want}: {c:?}");
        }
        let mut o = raster(&src, 4, 4);
        (o.feather, o.expansion, o.opacity) = (f32::NAN, f32::INFINITY, f32::NAN);
        let c = object_coverage(&o, 4, 4).unwrap();
        assert!(c.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
    }

    #[test]
    fn object_mask_random_masks_never_panic_and_stay_in_range() {
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..200 {
            let (sw, sh) = ((next() % 9 + 1) as usize, (next() % 9 + 1) as usize);
            let (w, h) = ((next() % 12 + 1) as usize, (next() % 12 + 1) as usize);
            let src: Vec<u8> = (0..sw * sh).map(|_| (next() % 3 == 0) as u8 * 255).collect();
            let o = ObjectRaster {
                src: &src,
                src_w: sw,
                src_h: sh,
                feather: (next() % 20) as f32 - 3.0,
                expansion: (next() % 21) as f32 - 10.0,
                opacity: (next() % 120) as f32 / 100.0,
                inverted: next() % 2 == 0,
            };
            let c = object_coverage(&o, w, h).unwrap();
            assert_eq!(c.len(), w * h);
            assert!(c.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
        }
    }
}
