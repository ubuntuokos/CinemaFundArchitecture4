//! CRT (`crt`, Video Effects ▸ Creative Looks): the neutral settings, the tube shape, scanlines,
//! phosphor masks, flicker and hostile parameters.

use cfa4_color::{linear_to_srgb, srgb_to_linear};
use cfa4_project::{EffectInstance, ParamValue, find_effect};
use cfa4_render::{
    Image,
    effects::{FxCtx, apply},
    gpufx::{FxOp, crt_shade, crt_warp},
};
use cfa4_time::{FrameRate, Tick};

fn cx(t: Tick, px_scale: f32) -> FxCtx<'static> {
    FxCtx {
        t,
        px_scale,
        seconds: t.seconds(),
        timecode: "00:00:00:00",
        clip_name: "",
        project: None,
        env: None,
        frame_rate: 24.0,
        working: cfa4_color::WorkingSpace::Rec709,
    }
}

fn crt(params: &[(&str, ParamValue)]) -> EffectInstance {
    let mut e = find_effect("crt").expect("crt").instance();
    for (k, v) in params {
        e.param_mut(k).unwrap_or_else(|| panic!("crt.{k}")).value = v.clone();
    }
    e
}

fn render(e: &EffectInstance, img: &Image, frame: i64) -> Image {
    let mut out = img.clone();
    apply(&mut out, e, &cx(FrameRate::FPS_24.tick_of(frame), 1.0));
    out
}

fn fl(v: f64) -> ParamValue {
    ParamValue::Float(v)
}
fn ch(c: u32) -> ParamValue {
    ParamValue::Choice(c)
}

/// Only one of the looks, the others off.
fn only(params: &[(&str, ParamValue)]) -> EffectInstance {
    let mut all = vec![("scanlines", fl(0.0)), ("mask_strength", fl(0.0)), ("curvature", fl(0.0)), ("corner_darkening", fl(0.0)), ("flicker", fl(0.0))];
    all.extend(params.iter().cloned());
    crt(&all)
}

fn gradient(w: usize, h: usize) -> Image {
    let mut img = Image::new(w, h);
    for (i, p) in img.px.chunks_exact_mut(4).enumerate() {
        let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
        p.copy_from_slice(&[x, y * 0.7, 1.0 - x, 1.0]);
    }
    img
}

/// Display-encoded red of pixel (`x`, `y`).
fn red(img: &Image, x: usize, y: usize) -> f32 {
    let p = img.get(x, y);
    linear_to_srgb(p[0] / p[3])
}

#[test]
fn neutral_settings_leave_the_picture_untouched() {
    let img = gradient(40, 30);
    let e = only(&[]);
    assert_eq!(render(&e, &img, 0).px, img.px);
    let op = FxOp::eval(&e, &cx(Tick::ZERO, 1.0), 40, 30);
    assert!(op.as_ref().is_some_and(FxOp::is_identity), "{op:?}");
    // the mask choice "None" ignores Mask Strength
    assert_eq!(render(&only(&[("mask", ch(0)), ("mask_strength", fl(100.0))]), &img, 0).px, img.px);
}

#[test]
fn defaults_change_the_picture_and_stay_finite() {
    let img = gradient(64, 48);
    let out = render(&crt(&[]), &img, 3);
    assert_ne!(out.px, img.px);
    assert!(out.px.iter().all(|v| v.is_finite()));
}

#[test]
fn curvature_bends_the_screen_and_blacks_out_the_corners() {
    // flat: every pixel is on the screen, at its own coordinates
    assert_eq!(crt_warp(0.5, 0.5, 10.0, 10.0, 0.0), Some((-0.9, -0.9)));
    // curved: the corner pixel falls outside, the centre stays put
    assert_eq!(crt_warp(0.5, 0.5, 10.0, 10.0, 0.25), None);
    let c = crt_warp(5.0, 5.0, 10.0, 10.0, 0.25);
    assert_eq!(c, Some((0.0, 0.0)));
    let img = Image::filled(40, 30, [0.5, 0.5, 0.5, 1.0]);
    let out = render(&only(&[("curvature", fl(100.0))]), &img, 0);
    assert_eq!(out.get(0, 0), [0.0; 4], "corner outside the tube");
    assert_eq!(out.get(20, 15), img.get(20, 15), "a flat colour stays flat inside");
}

#[test]
fn scanlines_darken_between_lines() {
    let img = Image::filled(16, 60, [0.6, 0.6, 0.6, 1.0]);
    // 60 lines on 60 rows: each row is the middle of its line, at full brightness
    let out = render(&only(&[("scanlines", fl(100.0)), ("line_count", fl(60.0))]), &img, 0);
    for y in 0..60 {
        assert!((red(&out, 3, y) - red(&img, 3, 0)).abs() < 1e-4, "row {y}");
    }
    // 60 lines on 180 rows: the middle row of each line is brighter than the rows at its edges
    let thin = render(&only(&[("scanlines", fl(100.0)), ("line_count", fl(60.0))]), &Image::filled(16, 180, [0.6, 0.6, 0.6, 1.0]), 0);
    let line: Vec<f32> = (0..3).map(|y| red(&thin, 3, y)).collect();
    assert!(line[1] > line[0] && line[1] > line[2], "{line:?}");
    let shade = |cv: f32| crt_shade(0.0, cv, 0.5, 0.5, 1.0, 20.0, 0, 0.0, 3.0, 0.0)[0];
    // the middle of line 10 (of 20) and the boundary between lines 9 and 10
    assert!((shade(0.05) - 1.0).abs() < 1e-5);
    assert!(shade(0.0) < 1e-5);
}

#[test]
fn masks_light_one_phosphor_per_stripe() {
    for mask in 1..=3u32 {
        let mut seen = [false; 3];
        for x in 0..12 {
            let k = crt_shade(0.0, 0.05, x as f32 + 0.5, 7.5, 0.0, 240.0, mask, 1.0, 3.0, 0.0);
            let lit: Vec<usize> = (0..3).filter(|&i| k[i] == 1.0).collect();
            assert!(lit.len() <= 1, "mask {mask}: {k:?}");
            assert!(k.iter().all(|&v| v == 0.0 || v == 1.0), "mask {mask}: {k:?}");
            if let Some(&i) = lit.first() {
                seen[i] = true;
            }
        }
        assert_eq!(seen, [true; 3], "mask {mask} lights every phosphor somewhere");
    }
    // the shadow mask shifts every other row of triads
    let row = |y: f32| (0..6).map(|x| crt_shade(0.0, 0.05, x as f32 + 0.5, y, 0.0, 240.0, 2, 1.0, 3.0, 0.0)).collect::<Vec<_>>();
    assert_ne!(row(0.5), row(2.5));
    // the slot mask has dark gaps
    let gaps = (0..12).filter(|&y| crt_shade(0.0, 0.05, 1.5, y as f32 + 0.5, 0.0, 240.0, 3, 1.0, 3.0, 0.0) == [0.0; 3]).count();
    assert!(gaps > 0);
}

#[test]
fn mask_pitch_follows_the_playback_resolution() {
    let e = crt(&[]);
    let full = FxOp::eval(&e, &cx(Tick::ZERO, 1.0), 64, 64);
    let half = FxOp::eval(&e, &cx(Tick::ZERO, 0.5), 32, 32);
    let (Some(FxOp::Crt { pitch: f, lines: lf, .. }), Some(FxOp::Crt { pitch: h, lines: lh, .. })) = (full, half) else {
        panic!("crt evaluates to Crt");
    };
    assert!((f - 2.0 * h).abs() < 1e-6, "{f} vs {h}");
    // the line count belongs to the frame, not to its pixels
    assert_eq!(lf, lh);
}

#[test]
fn flicker_varies_by_frame_and_seed_and_only_darkens() {
    let img = Image::filled(8, 8, [0.4, 0.4, 0.4, 1.0]);
    let e = only(&[("flicker", fl(100.0))]);
    let frames: Vec<f32> = (0..12).map(|f| red(&render(&e, &img, f), 2, 2)).collect();
    assert!(frames.iter().all(|&v| v <= red(&img, 2, 2) + 1e-5 && v >= red(&img, 2, 2) * 0.69));
    assert!(frames.windows(2).any(|w| w[0] != w[1]), "flicker must change between frames");
    assert_eq!(render(&e, &img, 5).px, render(&e, &img, 5).px);
    let other = only(&[("flicker", fl(100.0)), ("seed", fl(9.0))]);
    assert!((0..12).any(|f| render(&e, &img, f).px != render(&other, &img, f).px));
}

#[test]
fn corner_darkening_keeps_the_centre_and_darkens_the_rim() {
    let img = Image::filled(41, 41, [0.7, 0.7, 0.7, 1.0]);
    let out = render(&only(&[("corner_darkening", fl(100.0))]), &img, 0);
    assert!((red(&out, 20, 20) - red(&img, 20, 20)).abs() < 1e-3);
    assert!(red(&out, 0, 0) < 0.25 * red(&img, 0, 0), "{}", red(&out, 0, 0));
}

#[test]
fn transparent_pixels_stay_transparent_and_alpha_is_kept() {
    let mut img = Image::filled(20, 20, [0.2, 0.1, 0.05, 0.5]);
    img.px[0..4].copy_from_slice(&[0.0; 4]);
    let out = render(&crt(&[("curvature", fl(0.0))]), &img, 1);
    assert_eq!(&out.px[0..4], &[0.0; 4]);
    assert!(out.px.chunks_exact(4).skip(1).all(|p| p[3] == 0.5));
}

#[test]
fn hostile_parameters_neither_panic_nor_produce_non_finite_values() {
    let img = gradient(24, 18);
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1e30, 1e30, 0.0] {
        for k in ["scanlines", "line_count", "mask_strength", "curvature", "corner_darkening", "flicker", "seed"] {
            for mask in [0, 1, 2, 3, 99] {
                let out = render(&crt(&[("mask", ch(mask)), (k, fl(v))]), &img, 7);
                assert!(out.px.iter().all(|x| x.is_finite()), "{k} = {v}, mask {mask}");
            }
        }
    }
    // tiny, transparent and extreme-time frames
    let e = crt(&[("curvature", fl(100.0))]);
    for (w, h) in [(1, 1), (1, 5), (3, 1)] {
        let out = render(&e, &Image::filled(w, h, [0.2, 0.3, 0.4, 1.0]), 0);
        assert!(out.px.iter().all(|x| x.is_finite()));
    }
    assert_eq!(render(&e, &Image::new(8, 8), 0).px, Image::new(8, 8).px);
    let mut out = img.clone();
    apply(&mut out, &e, &cx(Tick(i64::MAX), 1.0));
    assert!(out.px.iter().all(|x| x.is_finite()));
    // the gamma helpers used above agree with each other
    assert!((linear_to_srgb(srgb_to_linear(0.5)) - 0.5).abs() < 1e-5);
}
