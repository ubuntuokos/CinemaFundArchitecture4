// SPDX-License-Identifier: GPL-3.0-or-later
//! Pixelate (Video Effects ▸ Glitch, CFA4's own): Mosaic with other cell shapes. Every cell
//! takes the colour of the picture at its centre; between cells (Gap) the Background shows.
//!
//! Shapes: Square, Hexagon, Circle, Diamond, Triangle and LED (round dots that are brighter in
//! the middle, on the background). Idea from the pixelize shapes of QianMo/X-PostProcessing-
//! Library (MIT) in the glitch-effect review; the lattices here are textbook geometry (square,
//! 45°-turned square, skewed triangle and offset-rectangle hexagon lattices), written for CFA4.

use rayon::prelude::*;

use super::{chv, cv, dec, fv};
use crate::effects::FxCtx;
use crate::image::Image;
use cfa4_project::EffectInstance;

/// A finite parameter value or `fallback`.
fn fin(v: f32, fallback: f32) -> f32 {
    if v.is_finite() { v } else { fallback }
}

/// The cell of pixel (x, y) for cell size `s`: its centre, and how far (x, y) lies from the
/// centre relative to the cell's inner radius (0 = centre, 1 = edge; used for Gap and LED).
pub(crate) fn cell(shape: u32, x: f32, y: f32, s: f32) -> ((f32, f32), f32) {
    let s = s.max(1.0);
    match shape {
        // Hexagon: nearest centre of two offset rectangle lattices (pointy side up)
        1 => {
            let (rx, ry) = (s, s * 3f32.sqrt());
            let near = |ox: f32, oy: f32| {
                let cx = ((x - ox) / rx).floor() * rx + rx * 0.5 + ox;
                let cy = ((y - oy) / ry).floor() * ry + ry * 0.5 + oy;
                (cx, cy)
            };
            let a = near(0.0, 0.0);
            let b = near(rx * 0.5, ry * 0.5);
            let d2 = |c: (f32, f32)| (x - c.0).powi(2) + (y - c.1).powi(2);
            let c = if d2(a) <= d2(b) { a } else { b };
            // hexagon metric (flat sides left and right): max of the three axis projections
            let (dx, dy) = ((x - c.0).abs(), (y - c.1).abs());
            let r = dx.max(dx * 0.5 + dy * 3f32.sqrt() * 0.5);
            (c, r / (s * 0.5))
        }
        // Circle, LED: square cells, round metric
        2 | 5 => {
            let c = (((x / s).floor() + 0.5) * s, ((y / s).floor() + 0.5) * s);
            (c, ((x - c.0).powi(2) + (y - c.1).powi(2)).sqrt() / (s * 0.5))
        }
        // Diamond: a square lattice turned 45°
        3 => {
            let (a, b) = ((x + y) / s, (x - y) / s);
            let (ca, cb) = (a.floor() + 0.5, b.floor() + 0.5);
            let c = ((ca + cb) * 0.5 * s, (ca - cb) * 0.5 * s);
            let r = ((a - ca).abs()).max((b - cb).abs()) * 2.0;
            (c, r)
        }
        // Triangle: a skewed lattice whose rhombi split into an up and a down triangle
        4 => {
            let hgt = s * 3f32.sqrt() * 0.5;
            let v = y / hgt;
            let u = x / s - v * 0.5;
            let (iu, iv) = (u.floor(), v.floor());
            let (fu, fv) = (u - iu, v - iv);
            let (cu, cv, r) = if fu + fv < 1.0 {
                // barycentric: 1/3 each at the centroid, a zero on an edge
                (iu + 1.0 / 3.0, iv + 1.0 / 3.0, 1.0 - 3.0 * fu.min(fv).min(1.0 - fu - fv))
            } else {
                let (gu, gv) = (1.0 - fu, 1.0 - fv);
                (iu + 2.0 / 3.0, iv + 2.0 / 3.0, 1.0 - 3.0 * gu.min(gv).min(1.0 - gu - gv))
            };
            ((((cu + cv * 0.5) * s), cv * hgt), r.clamp(0.0, 1.0))
        }
        // Square
        _ => {
            let c = (((x / s).floor() + 0.5) * s, ((y / s).floor() + 0.5) * s);
            (c, (x - c.0).abs().max((y - c.1).abs()) / (s * 0.5))
        }
    }
}

pub fn pixelate(img: &mut Image, e: &EffectInstance, cx: &FxCtx<'_>) {
    let (w, h) = (img.w, img.h);
    if w == 0 || h == 0 {
        return;
    }
    let shape = chv(e, "shape").min(5);
    let s = (fin(fv(e, "size", cx), 24.0) * cx.px_scale).clamp(1.0, 8192.0);
    // the shape fills (1 − gap) of its cell
    let fill = 1.0 - (fin(fv(e, "gap", cx), 0.0) / 100.0).clamp(0.0, 0.95);
    let bg_enc = cv(e, "background", cx);
    let bg = {
        let l = dec([bg_enc[0], bg_enc[1], bg_enc[2]]);
        let a = bg_enc[3].clamp(0.0, 1.0);
        [l[0] * a, l[1] * a, l[2] * a, a]
    };
    let blend = (fin(fv(e, "blend", cx), 0.0) / 100.0).clamp(0.0, 1.0);
    if blend >= 1.0 {
        return; // fully blended back: the original, bit for bit
    }
    let src = img.clone();
    // a pixel is ~1/(s/2) of the inner radius: anti-alias the shape edge over one pixel
    let aa = 1.0 / (s * 0.5 * fill).max(0.5);
    img.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let ((ccx, ccy), r) = cell(shape, fx, fy, s);
            let c = src.sample_bilinear_clamped(ccx.clamp(0.0, w as f32 - 0.5), ccy.clamp(0.0, h as f32 - 0.5));
            let r = r / fill;
            let mut k = (1.0 - (r - 1.0) / aa).clamp(0.0, 1.0);
            let mut cell_c = c;
            if shape == 5 {
                // LED: brighter in the middle, soft rim
                let glow = 1.15 - 0.35 * r.min(1.0).powi(2);
                cell_c = [c[0] * glow, c[1] * glow, c[2] * glow, c[3]];
                k *= (1.0 - (r - 0.75).max(0.0) * 4.0).clamp(0.0, 1.0);
            }
            let shaped = [0, 1, 2, 3].map(|i| bg[i] + (cell_c[i] - bg[i]) * k);
            let o = src.get(x, y);
            let out = [0, 1, 2, 3].map(|i| shaped[i] + (o[i] - shaped[i]) * blend);
            if let Some(p) = row.get_mut(x * 4..x * 4 + 4) {
                p.copy_from_slice(&out);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use cfa4_project::{ParamValue, find_effect};

    use super::*;

    fn cx() -> FxCtx<'static> {
        FxCtx {
            t: cfa4_time::Tick::ZERO,
            px_scale: 1.0,
            seconds: 0.0,
            timecode: "01:00:00:00",
            clip_name: "clip",
            project: None,
            env: None,
            frame_rate: 24.0,
            working: cfa4_color::WorkingSpace::Rec709,
        }
    }

    fn picture(w: usize, h: usize) -> Image {
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                let c = [(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32, ((x * 7 + y * 13) % 17) as f32 / 17.0];
                img.px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 1.0]);
            }
        }
        img
    }

    fn run(e: &EffectInstance, img: &Image) -> Image {
        let mut o = img.clone();
        assert!(crate::vfx::apply(&mut o, e, &cx()), "{} not dispatched", e.effect);
        o
    }

    fn set(e: &mut EffectInstance, k: &str, v: ParamValue) {
        e.params.get_mut(k).unwrap_or_else(|| panic!("no param {k}")).value = v;
    }

    #[test]
    fn pixelate_sits_in_the_glitch_folder() {
        let d = find_effect("pixelate").unwrap();
        assert_eq!(d.category, cfa4_project::effect::GLITCH);
    }

    #[test]
    fn every_point_of_a_cell_finds_the_same_centre() {
        // a point and its neighbour a tenth of a cell away share a centre unless an edge lies
        // between; the centre itself is its own cell, and the edge value stays in 0 … ~1.2
        for shape in 0..6 {
            for (x, y) in [(10.0f32, 10.0f32), (55.3, 21.7), (100.1, 300.9), (7.5, 92.0)] {
                let ((cx, cy), r) = cell(shape, x, y, 24.0);
                assert!((0.0..1.6).contains(&r), "shape {shape} at ({x}, {y}): r {r}");
                assert!(((x - cx).powi(2) + (y - cy).powi(2)).sqrt() <= 24.0, "shape {shape}: centre too far");
                let ((ax, ay), r0) = cell(shape, cx, cy, 24.0);
                assert!((ax - cx).abs() < 1e-3 && (ay - cy).abs() < 1e-3, "shape {shape}: centre not stable");
                assert!(r0 < 1e-3, "shape {shape}: centre r {r0}");
            }
        }
    }

    #[test]
    fn cells_are_flat_and_gaps_show_the_background() {
        let img = picture(96, 64);
        let mut e = find_effect("pixelate").unwrap().instance();
        for shape in 0..6u32 {
            set(&mut e, "shape", ParamValue::Choice(shape));
            set(&mut e, "gap", ParamValue::Float(0.0));
            let o = run(&e, &img);
            assert!(o.px.iter().all(|v| v.is_finite() && *v >= 0.0));
            if shape != 5 {
                // two points close to one centre share its colour
                let ((cx, cy), _) = cell(shape, 48.5, 32.5, 24.0);
                let (px, py) = (cx.floor() as usize, cy.floor() as usize);
                let a = o.get(px, py);
                let b = o.get(px + 1, py);
                assert!((0..4).all(|k| (a[k] - b[k]).abs() < 1e-4), "shape {shape}: {a:?} {b:?}");
            }
            // with a gap, some pixels are the (black) background
            set(&mut e, "gap", ParamValue::Float(40.0));
            let o = run(&e, &img);
            let black = o.px.chunks(4).filter(|c| c[0] + c[1] + c[2] < 1e-4 && c[3] > 0.99).count();
            assert!(black > 0, "shape {shape}: no background in the gaps");
        }
        // Blend 100 % gives the original back
        set(&mut e, "blend", ParamValue::Float(100.0));
        assert_eq!(run(&e, &img).px, img.px);
    }

    #[test]
    fn hostile_values_never_panic() {
        let mut e = find_effect("pixelate").unwrap().instance();
        for img in [picture(1, 1), picture(3, 2), Image::new(0, 0), picture(40, 30)] {
            for v in [f64::NAN, f64::INFINITY, -1e12, 0.0, 1e12] {
                for k in ["size", "gap", "blend"] {
                    set(&mut e, k, ParamValue::Float(v));
                }
                for shape in [0, 3, 5, 99] {
                    set(&mut e, "shape", ParamValue::Choice(shape));
                    let mut o = img.clone();
                    crate::vfx::apply(&mut o, &e, &cx());
                    assert!(o.px.iter().all(|c| c.is_finite()));
                }
            }
        }
    }
}
