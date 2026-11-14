//! Oil Paint (`oil_paint`, Video Effects ▸ Creative Looks): the Kuwahara sectors, flat areas,
//! edges, styles, textures and hostile parameters.

use cfa4_project::{EffectInstance, ParamValue, find_effect};
use cfa4_render::{
    Image,
    effects::{FxCtx, apply},
    gpufx::{FxOp, oil_sectors, oil_texture},
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

fn oil(params: &[(&str, ParamValue)]) -> EffectInstance {
    let mut e = find_effect("oil_paint").expect("oil_paint").instance();
    for (k, v) in params {
        e.param_mut(k).unwrap_or_else(|| panic!("oil_paint.{k}")).value = v.clone();
    }
    e
}

fn render(e: &EffectInstance, img: &Image) -> Image {
    let mut out = img.clone();
    apply(&mut out, e, &cx(1.0));
    out
}

fn fl(v: f64) -> ParamValue {
    ParamValue::Float(v)
}
fn ch(c: u32) -> ParamValue {
    ParamValue::Choice(c)
}

/// Pseudo-random detail on two flat halves (left dark, right light).
fn two_halves(w: usize, h: usize) -> Image {
    let mut img = Image::new(w, h);
    for (i, p) in img.px.chunks_exact_mut(4).enumerate() {
        let (x, y) = (i % w, i / w);
        let base = if x < w / 2 { 0.1 } else { 0.8 };
        let n = (((x * 7 + y * 13) % 11) as f32 / 11.0 - 0.5) * 0.08;
        p.copy_from_slice(&[base + n, base * 0.8 + n, base * 0.5, 1.0]);
    }
    img
}

fn spread(img: &Image, x0: usize, x1: usize) -> f32 {
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for y in 0..img.h {
        for x in x0..x1 {
            let v = img.get(x, y)[0];
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    hi - lo
}

#[test]
fn sectors_cover_the_brush_and_meet_at_the_centre() {
    for style in 0..6u32 {
        let r = 5;
        assert_eq!(oil_sectors(style, 0, 0, r).count_ones(), if style == 0 || style == 4 { 4 } else { 8 });
        for dy in -r..=r {
            for dx in -r..=r {
                let m = oil_sectors(style, dx, dy, r);
                let inside = style == 0 || dx * dx + dy * dy <= r * r;
                assert_eq!(m != 0, inside, "style {style} ({dx}, {dy})");
                // a tap is in at most two neighbouring sectors, apart from the centre
                if (dx, dy) != (0, 0) {
                    assert!(m.count_ones() <= 2, "style {style} ({dx}, {dy}): {m:b}");
                }
            }
        }
    }
    // mirrored taps land in mirrored octants
    assert_eq!(oil_sectors(1, 3, 1, 5), 1);
    assert_eq!(oil_sectors(1, -3, -1, 5), 1 << 4);
}

#[test]
fn flat_colour_stays_flat_without_texture() {
    let img = Image::filled(24, 20, [0.3, 0.2, 0.1, 1.0]);
    for style in 0..6 {
        let out = render(&oil(&[("style", ch(style)), ("texture", fl(0.0))]), &img);
        for (a, b) in out.px.iter().zip(&img.px) {
            assert!((a - b).abs() < 1e-5, "style {style}: {a} vs {b}");
        }
    }
}

#[test]
fn flattens_detail_but_keeps_the_edge() {
    let img = two_halves(48, 24);
    let out = render(&oil(&[("texture", fl(0.0)), ("detail", fl(100.0))]), &img);
    // the noise inside each half is smoothed
    assert!(spread(&out, 6, 18) < 0.5 * spread(&img, 6, 18), "{} vs {}", spread(&out, 6, 18), spread(&img, 6, 18));
    assert!(spread(&out, 30, 42) < 0.5 * spread(&img, 30, 42));
    // and the edge between the halves stays sharp: the two pixels next to it keep their sides
    for y in 0..24 {
        assert!(out.get(23, y)[0] < 0.3, "row {y}: {}", out.get(23, y)[0]);
        assert!(out.get(24, y)[0] > 0.6, "row {y}: {}", out.get(24, y)[0]);
    }
}

#[test]
fn styles_and_seeds_differ() {
    let img = two_halves(40, 30);
    let outs: Vec<Vec<f32>> = (0..6).map(|s| render(&oil(&[("style", ch(s)), ("texture", fl(80.0))]), &img).px).collect();
    for i in 0..6 {
        for j in i + 1..6 {
            assert_ne!(outs[i], outs[j], "styles {i} and {j}");
        }
    }
    // the seed moves the noise textures (Fresco), not the weave (Daubs)
    let fresco = |seed: f64| render(&oil(&[("style", ch(3)), ("texture", fl(80.0)), ("seed", fl(seed))]), &img).px;
    assert_ne!(fresco(1.0), fresco(2.0));
    let daubs = |seed: f64| render(&oil(&[("style", ch(1)), ("texture", fl(80.0)), ("seed", fl(seed))]), &img).px;
    assert_eq!(daubs(1.0), daubs(2.0));
    for style in 0..6 {
        for (x, y) in [(0, 0), (17, 3), (999, 1234)] {
            assert!(oil_texture(style, x, y, 6, 1).abs() <= 0.6 + 1e-6);
        }
    }
}

#[test]
fn brush_follows_the_playback_resolution() {
    let e = oil(&[("brush_size", fl(8.0))]);
    let full = FxOp::eval(&e, &cx(1.0), 64, 64);
    let half = FxOp::eval(&e, &cx(0.5), 32, 32);
    let (Some(FxOp::OilPaint { radius: f, .. }), Some(FxOp::OilPaint { radius: h, .. })) = (full, half) else {
        panic!("oil_paint evaluates to OilPaint");
    };
    assert_eq!((f, h), (8, 4));
}

#[test]
fn transparency_is_kept() {
    let img = Image::new(10, 10);
    assert_eq!(render(&oil(&[]), &img).px, img.px);
    let half = Image::filled(10, 10, [0.2, 0.1, 0.05, 0.5]);
    let out = render(&oil(&[("texture", fl(50.0))]), &half);
    assert!(out.px.chunks_exact(4).all(|p| (p[3] - 0.5).abs() < 1e-6));
}

#[test]
fn hostile_parameters_neither_panic_nor_produce_non_finite_values() {
    let img = two_halves(20, 14);
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1e30, 1e30, 0.0] {
        for k in ["brush_size", "detail", "texture", "seed"] {
            for style in [0, 1, 2, 3, 4, 5, 99] {
                let out = render(&oil(&[("style", ch(style)), (k, fl(v))]), &img);
                assert!(out.px.iter().all(|x| x.is_finite()), "{k} = {v}, style {style}");
            }
        }
    }
    for (w, h) in [(1, 1), (1, 5), (3, 1)] {
        let out = render(&oil(&[("brush_size", fl(12.0))]), &Image::filled(w, h, [0.2, 0.3, 0.4, 1.0]));
        assert!(out.px.iter().all(|x| x.is_finite()));
    }
}
