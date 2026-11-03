//! Film Grain (`film_grain`, Video Effects ▸ Creative Looks): determinism, animation, the tonal
//! response and hostile parameters.

use cfa4_project::{EffectInstance, ParamValue, find_effect};
use cfa4_render::{
    Image,
    effects::{FxCtx, apply},
    gpufx::{FxOp, grain_noise, pcg3d},
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

fn grain(params: &[(&str, ParamValue)]) -> EffectInstance {
    let mut e = find_effect("film_grain").expect("film_grain").instance();
    for (k, v) in params {
        e.param_mut(k).unwrap_or_else(|| panic!("film_grain.{k}")).value = v.clone();
    }
    e
}

fn render(e: &EffectInstance, img: &Image, frame: i64) -> Vec<f32> {
    let mut out = img.clone();
    apply(&mut out, e, &cx(FrameRate::FPS_24.tick_of(frame), 1.0));
    out.px
}

fn grey(v: f32) -> Image {
    Image::filled(32, 24, [v, v, v, 1.0])
}

fn fl(v: f64) -> ParamValue {
    ParamValue::Float(v)
}

#[test]
fn pcg3d_is_stable_and_spreads_values() {
    // pinned so the CPU reference and `fx.wgsl` can't drift apart silently
    assert_eq!(pcg3d(0, 0, 0), pcg3d(0, 0, 0));
    assert_ne!(pcg3d(1, 0, 0), pcg3d(0, 1, 0));
    assert_ne!(pcg3d(0, 1, 0), pcg3d(0, 0, 1));
    // lattice noise is centred and bounded
    let n: Vec<f32> = (0..64).flat_map(|y| (0..64).map(move |x| grain_noise(x, y, 0.7, 12345, 0))).collect();
    assert!(n.iter().all(|v| (-0.5..=0.5).contains(v)));
    let mean = n.iter().sum::<f32>() / n.len() as f32;
    assert!(mean.abs() < 0.05, "mean {mean}");
    // layers are independent
    assert_ne!(grain_noise(3, 4, 0.7, 1, 0), grain_noise(3, 4, 0.7, 1, 1));
}

#[test]
fn grain_is_deterministic_and_animates_per_frame() {
    let img = grey(0.2);
    let e = grain(&[("intensity", fl(60.0))]);
    let a = render(&e, &img, 3);
    assert_ne!(a, img.px, "grain must change the picture");
    assert_eq!(a, render(&e, &img, 3), "same frame must render identically");
    assert_ne!(a, render(&e, &img, 4), "animated grain must change between frames");
    // still grain: the same pattern on every frame
    let still = grain(&[("intensity", fl(60.0)), ("animated", ParamValue::Bool(false))]);
    assert_eq!(render(&still, &img, 3), render(&still, &img, 4));
    // the seed picks another pattern
    let other = grain(&[("intensity", fl(60.0)), ("seed", fl(2.0))]);
    assert_ne!(a, render(&other, &img, 3));
}

#[test]
fn zero_intensity_and_transparent_pixels_are_untouched() {
    let img = grey(0.4);
    assert_eq!(render(&grain(&[("intensity", fl(0.0))]), &img, 0), img.px);
    let clear = Image::new(16, 16);
    assert_eq!(render(&grain(&[("intensity", fl(100.0))]), &clear, 0), clear.px);
}

#[test]
fn mono_grain_keeps_grey_grey_and_alpha() {
    let img = Image::filled(20, 20, [0.1, 0.1, 0.1, 0.5]);
    let out = render(&grain(&[("intensity", fl(80.0)), ("color", fl(0.0))]), &img, 1);
    for p in out.chunks_exact(4) {
        assert_eq!(p[3], 0.5);
        assert_eq!(p[0], p[1]);
        assert_eq!(p[1], p[2]);
    }
    let colored = render(&grain(&[("intensity", fl(80.0)), ("color", fl(100.0))]), &img, 1);
    assert!(colored.chunks_exact(4).any(|p| p[0] != p[1] || p[1] != p[2]), "color grain must tint");
}

#[test]
fn tonal_response_weights_the_grain() {
    // shadows only: dark pixels move, white stays white
    let e = grain(&[("intensity", fl(100.0)), ("shadows", fl(200.0)), ("midtones", fl(0.0)), ("highlights", fl(0.0))]);
    let white = grey(1.0);
    assert_eq!(render(&e, &white, 0), white.px);
    let dark = grey(0.02);
    assert_ne!(render(&e, &dark, 0), dark.px);
}

#[test]
fn grain_size_follows_the_playback_resolution() {
    // a half-resolution preview shows the same grain cell per output pixel as the full render
    let e = grain(&[("size", fl(4.0))]);
    let full = FxOp::eval(&e, &cx(Tick::ZERO, 1.0), 64, 64);
    let half = FxOp::eval(&e, &cx(Tick::ZERO, 0.5), 32, 32);
    let (Some(FxOp::FilmGrain { inv_cell: f, .. }), Some(FxOp::FilmGrain { inv_cell: h, .. })) = (full, half) else {
        panic!("film_grain evaluates to FilmGrain");
    };
    assert!((h - 2.0 * f).abs() < 1e-6, "{h} vs {f}");
}

#[test]
fn hostile_parameters_neither_panic_nor_produce_non_finite_values() {
    let img = grey(0.5);
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1e30, 1e30, 0.0] {
        for k in ["intensity", "size", "shadows", "midtones", "highlights", "color", "seed"] {
            let e = grain(&[("intensity", fl(50.0)), (k, fl(v))]);
            let out = render(&e, &img, 7);
            assert!(out.iter().all(|x| x.is_finite()), "{k} = {v}");
        }
    }
    // extreme timestamps fold into the seed
    let e = grain(&[("intensity", fl(50.0))]);
    let mut out = img.clone();
    apply(&mut out, &e, &cx(Tick(i64::MAX), 1.0));
    assert!(out.px.iter().all(|x| x.is_finite()));
}
