//! Halftone and Dither (Video Effects ▸ Creative Looks): screens, palettes, block size and
//! hostile parameters.

use cfa4_color::{linear_to_srgb, srgb_to_linear};
use cfa4_project::{EffectInstance, ParamValue, find_effect};
use cfa4_render::{
    Image,
    effects::{FxCtx, apply},
    gpufx::{DITHER_PALETTES, FxOp, dither_threshold, halftone_ink},
};
use cfa4_time::Tick;

fn cx(px_scale: f32) -> FxCtx<'static> {
    FxCtx {
        t: Tick::ZERO,
        px_scale,
        seconds: 0.0,
        timecode: "00:00:00:00",
        clip_name: "",
        project: None,
        env: None,
        frame_rate: 24.0,
        working: cfa4_color::WorkingSpace::Rec709,
    }
}

fn effect(id: &str, params: &[(&str, ParamValue)]) -> EffectInstance {
    let mut e = find_effect(id).unwrap_or_else(|| panic!("{id}")).instance();
    for (k, v) in params {
        e.param_mut(k).unwrap_or_else(|| panic!("{id}.{k}")).value = v.clone();
    }
    e
}

fn render(e: &EffectInstance, img: &Image) -> Image {
    let mut out = img.clone();
    apply(&mut out, e, &cx(1.0));
    out
}

/// A display-encoded grey as a linear opaque image.
fn grey(v: f32) -> Image {
    let l = srgb_to_linear(v);
    Image::filled(48, 32, [l, l, l, 1.0])
}

/// Display-encoded colour of each pixel (straight alpha).
fn encoded(img: &Image) -> Vec<[f32; 3]> {
    img.px.chunks_exact(4).map(|p| if p[3] > 0.0 { [0, 1, 2].map(|i| linear_to_srgb(p[i] / p[3])) } else { [0.0; 3] }).collect()
}

fn fl(v: f64) -> ParamValue {
    ParamValue::Float(v)
}
fn ch(c: u32) -> ParamValue {
    ParamValue::Choice(c)
}

#[test]
fn halftone_ink_is_empty_at_zero_and_solid_at_one() {
    for lines in [false, true] {
        for (x, y) in [(0.5, 0.5), (3.5, 7.5), (11.5, 2.5)] {
            assert_eq!(halftone_ink(x, y, 0.8, 0.6, 0.125, 0.1, lines, 0.0), 0.0);
            assert_eq!(halftone_ink(x, y, 0.8, 0.6, 0.125, 0.1, lines, 1.0), 1.0);
        }
    }
    // coverage grows with the tone (the mean over a cell tracks it)
    let mean = |v: f32| {
        (0..16).flat_map(|y| (0..16).map(move |x| halftone_ink(x as f32 + 0.5, y as f32 + 0.5, 1.0, 0.0, 1.0 / 16.0, 0.05, false, v))).sum::<f32>() / 256.0
    };
    assert!(mean(0.25) < mean(0.5) && mean(0.5) < mean(0.75));
    assert!((mean(0.5) - 0.5).abs() < 0.1, "{}", mean(0.5));
}

#[test]
fn colour_halftone_prints_white_as_paper_and_black_as_ink() {
    let e = effect("halftone", &[]);
    assert!(encoded(&render(&e, &grey(1.0))).iter().all(|c| c.iter().all(|v| (v - 1.0).abs() < 1e-4)));
    assert!(encoded(&render(&e, &grey(0.0))).iter().all(|c| c.iter().all(|v| v.abs() < 1e-4)));
    // neutral greys use the black screen only: every pixel stays neutral
    for c in encoded(&render(&e, &grey(0.5))) {
        assert!((c[0] - c[1]).abs() < 1e-4 && (c[1] - c[2]).abs() < 1e-4, "{c:?}");
    }
}

#[test]
fn monochrome_halftone_mixes_paper_and_ink() {
    let ink = [0.1, 0.2, 0.6];
    let paper = [1.0, 0.95, 0.8];
    let e = effect(
        "halftone",
        &[("mode", ch(1)), ("ink", ParamValue::Color([ink[0], ink[1], ink[2], 1.0])), ("paper", ParamValue::Color([paper[0], paper[1], paper[2], 1.0]))],
    );
    for c in encoded(&render(&e, &grey(1.0))) {
        assert!(c.iter().zip(paper).all(|(a, b)| (a - b).abs() < 1e-3), "{c:?}");
    }
    for c in encoded(&render(&e, &grey(0.0))) {
        assert!(c.iter().zip(ink).all(|(a, b)| (a - b).abs() < 1e-3), "{c:?}");
    }
    // a mid grey shows both
    let mid = encoded(&render(&e, &grey(0.5)));
    assert!(mid.iter().any(|c| (c[2] - paper[2]).abs() < 1e-3) && mid.iter().any(|c| (c[2] - ink[2]).abs() < 1e-3));
}

#[test]
fn halftone_cell_follows_the_playback_resolution() {
    let e = effect("halftone", &[("cell_size", fl(10.0))]);
    let full = FxOp::eval(&e, &cx(1.0), 64, 64);
    let half = FxOp::eval(&e, &cx(0.5), 32, 32);
    let (Some(FxOp::Halftone { inv_cell: f, .. }), Some(FxOp::Halftone { inv_cell: h, .. })) = (full, half) else {
        panic!("halftone evaluates to Halftone");
    };
    assert!((h - 2.0 * f).abs() < 1e-6);
}

#[test]
fn bayer_matrices_are_the_classic_ones() {
    let m2: Vec<f32> = (0..2).flat_map(|y| (0..2).map(move |x| dither_threshold(x, y, 0))).collect();
    let want = [0.0, 2.0, 3.0, 1.0].map(|v| (v + 0.5) / 4.0 - 0.5);
    assert_eq!(m2, want);
    // dispersed, not clustered: the classic first rows
    let row = |pattern: u32, side: u32| -> Vec<u32> {
        (0..side).map(|x| ((dither_threshold(x, 0, pattern) + 0.5) * (side * side) as f32 - 0.5).round() as u32).collect()
    };
    assert_eq!(row(1, 4), [0, 8, 2, 10]);
    assert_eq!(row(2, 8), [0, 32, 8, 40, 2, 34, 10, 42]);
    for (pattern, side) in [(1u32, 4u32), (2, 8)] {
        let mut v: Vec<f32> = (0..side).flat_map(|y| (0..side).map(move |x| dither_threshold(x, y, pattern))).collect();
        v.sort_by(f32::total_cmp);
        v.dedup();
        assert_eq!(v.len(), (side * side) as usize, "every threshold once");
        // tiles
        assert_eq!(dither_threshold(1, 2, pattern), dither_threshold(1 + side, 2 + 3 * side, pattern));
    }
    for (x, y) in [(0, 0), (5, 9), (1000, 7)] {
        assert!((-0.5..0.5).contains(&dither_threshold(x, y, 3)));
    }
}

#[test]
fn input_palette_dither_uses_only_the_levels() {
    let img = grey(0.37);
    for levels in [2.0, 4.0, 7.0] {
        let out = encoded(&render(&effect("dither", &[("levels", fl(levels))]), &img));
        let n = (levels - 1.0) as f32;
        for c in &out {
            for v in c {
                assert!(((v * n) - (v * n).round()).abs() < 2e-3, "{v} not on a level of {levels}");
            }
        }
        // a flat 0.37 averages back to about 0.37
        let mean = out.iter().map(|c| c[0]).sum::<f32>() / out.len() as f32;
        assert!((mean - 0.37).abs() < 0.06, "levels {levels}: mean {mean}");
    }
}

#[test]
fn fixed_palettes_emit_only_their_colours() {
    let mut img = Image::new(40, 24);
    for (i, p) in img.px.chunks_exact_mut(4).enumerate() {
        let t = i as f32 / 960.0;
        p.copy_from_slice(&[t, (1.0 - t) * 0.6, (t * 7.0).fract(), 1.0]);
    }
    for (k, pal) in DITHER_PALETTES.iter().enumerate() {
        let out = encoded(&render(&effect("dither", &[("palette", ch(k as u32 + 1))]), &img));
        let colours: Vec<[f32; 3]> = pal.iter().map(|c| [(c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF].map(|v| v as f32 / 255.0)).collect();
        for c in out {
            assert!(colours.iter().any(|p| p.iter().zip(c).all(|(a, b)| (a - b).abs() < 2e-3)), "palette {k}: {c:?}");
        }
    }
}

#[test]
fn pixel_size_makes_uniform_blocks() {
    let mut img = Image::new(32, 32);
    for (i, p) in img.px.chunks_exact_mut(4).enumerate() {
        let v = (i % 32) as f32 / 31.0;
        p.copy_from_slice(&[v, v * 0.5, 1.0 - v, 1.0]);
    }
    let out = render(&effect("dither", &[("pixel_size", fl(4.0)), ("palette", ch(7))]), &img);
    for by in 0..8 {
        for bx in 0..8 {
            let first = &out.px[((by * 4) * 32 + bx * 4) * 4..][..4];
            for y in 0..4 {
                for x in 0..4 {
                    let i = ((by * 4 + y) * 32 + bx * 4 + x) * 4;
                    assert_eq!(&out.px[i..i + 4], first, "block {bx},{by}");
                }
            }
        }
    }
}

#[test]
fn full_blend_returns_each_original_pixel_even_with_big_dither_pixels() {
    let mut img = Image::new(16, 16);
    for (i, p) in img.px.chunks_exact_mut(4).enumerate() {
        // a transparent detail inside each 4×4 block, opaque elsewhere
        let a = if i % 16 == 1 { 0.0 } else { 1.0 };
        let v = (i % 7) as f32 / 6.0;
        p.copy_from_slice(&[v * a, 0.3 * a, (1.0 - v) * a, a]);
    }
    let out = render(&effect("dither", &[("pixel_size", fl(4.0)), ("blend", fl(100.0)), ("palette", ch(4))]), &img);
    for (o, i) in out.px.chunks_exact(4).zip(img.px.chunks_exact(4)) {
        assert_eq!(o[3], i[3]);
        for k in 0..3 {
            assert!((o[k] - i[k]).abs() < 1e-4, "{o:?} vs {i:?}");
        }
    }
}

#[test]
fn hostile_parameters_neither_panic_nor_produce_non_finite_values() {
    let img = grey(0.5);
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1e30, 1e30, 0.0] {
        for k in ["cell_size", "sharpness", "cyan_angle", "black_angle", "mono_angle", "blend"] {
            for mode in [0, 1] {
                let out = render(&effect("halftone", &[("mode", ch(mode)), (k, fl(v))]), &img);
                assert!(out.px.iter().all(|x| x.is_finite()), "halftone {k} = {v}");
            }
        }
        for k in ["pixel_size", "threshold", "levels", "blend"] {
            for palette in [0, 3, 8, 99] {
                let out = render(&effect("dither", &[("palette", ch(palette)), ("pattern", ch(palette)), (k, fl(v))]), &img);
                assert!(out.px.iter().all(|x| x.is_finite()), "dither {k} = {v}");
            }
        }
    }
    // tiny and transparent images
    for id in ["halftone", "dither"] {
        let e = effect(id, &[("pixel_size", fl(16.0))].into_iter().filter(|_| id == "dither").collect::<Vec<_>>());
        for (w, h) in [(1, 1), (1, 5), (3, 1)] {
            let out = render(&e, &Image::filled(w, h, [0.2, 0.3, 0.4, 1.0]));
            assert!(out.px.iter().all(|x| x.is_finite()));
        }
        assert_eq!(render(&e, &Image::new(8, 8)).px, Image::new(8, 8).px);
    }
}
