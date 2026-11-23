//! Tests for Color Key (`effects::key`): Edge Thin, hostile parameters, and that the unthinned matte
//! is bit-identical to the keyer before Edge Thin existed (CFA4-GREENSCREEN-MATTING-INTEGRATION-001, Z1a).

use cfa4_project::{ParamValue, find_effect};

use crate::effects::{FxCtx, apply, dec, enc};
use crate::image::Image;
use cfa4_time::Tick;

fn cx(px_scale: f32) -> FxCtx<'static> {
    FxCtx {
        t: Tick::ZERO,
        px_scale,
        seconds: 0.0,
        timecode: "01:00:00:00",
        clip_name: "clip",
        project: None,
        env: None,
        frame_rate: 24.0,
        working: cfa4_color::WorkingSpace::Rec709,
    }
}

fn color_key(thin: f64) -> cfa4_project::EffectInstance {
    let mut e = find_effect("color_key").map(|d| d.instance()).unwrap_or_else(|| panic!("color_key is registered"));
    for (k, v) in [
        ("color", ParamValue::Color([0.0, 0.8, 0.2, 1.0])),
        ("tolerance", ParamValue::Float(40.0)),
        ("feather", ParamValue::Float(0.0)),
        ("thin", ParamValue::Float(thin)),
    ] {
        match e.params.get_mut(k) {
            Some(p) => p.value = v,
            None => panic!("color_key has no param {k}"),
        }
    }
    e
}

/// `w`×`h` picture: green screen on the left `split` columns, grey subject on the rest.
fn screen_and_subject(w: usize, h: usize, split: usize) -> Image {
    let (g, s) = (dec([0.0, 0.8, 0.2]), dec([0.5, 0.45, 0.42]));
    let mut img = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let c = if x < split { g } else { s };
            img.px[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], 1.0]);
        }
    }
    img
}

fn run(thin: f64, img: &Image, px_scale: f32) -> Image {
    let mut o = img.clone();
    apply(&mut o, &color_key(thin), &cx(px_scale));
    o
}

fn opaque_columns(img: &Image, row: usize) -> usize {
    (0..img.w).filter(|&x| img.px[(row * img.w + x) * 4 + 3] > 0.5).count()
}

/// The keyer as it was before Edge Thin (colour part, spill 0, composite output).
fn reference_key(img: &mut Image, kc: [f32; 3], tolerance: f32, feather: f32) {
    let kycc = cfa4_color::rgb_to_ycbcr(kc[0], kc[1], kc[2], cfa4_color::Matrix::Bt709);
    let (tol, soft) = (tolerance / 255.0 * 0.6 + 0.01, feather / 50.0 * 0.2 + 0.01);
    for p in img.px.chunks_mut(4) {
        if p[3] <= 0.0 {
            continue;
        }
        let c = enc([p[0] / p[3], p[1] / p[3], p[2] / p[3]]);
        let ycc = cfa4_color::rgb_to_ycbcr(c[0], c[1], c[2], cfa4_color::Matrix::Bt709);
        let d = ((ycc[1] - kycc[1]).powi(2) + (ycc[2] - kycc[2]).powi(2)).sqrt() + (ycc[0] - kycc[0]).abs() * 0.15;
        let alpha = ((d - tol) / soft).clamp(0.0, 1.0);
        let a = p[3] * alpha;
        let lo = dec(c);
        p.copy_from_slice(&[lo[0] * a, lo[1] * a, lo[2] * a, a]);
    }
}

#[test]
fn edge_thin_zero_is_bit_identical_to_the_keyer_before_it() {
    let img = screen_and_subject(24, 6, 10);
    let mut want = img.clone();
    reference_key(&mut want, [0.0, 0.8, 0.2], 40.0, 0.0);
    let got = run(0.0, &img, 1.0);
    assert_eq!(got.px, want.px);
}

#[test]
fn edge_thin_thins_and_fattens_the_foreground() {
    let img = screen_and_subject(32, 5, 12);
    let base = opaque_columns(&run(0.0, &img, 1.0), 2);
    assert_eq!(base, 20, "20 subject columns stay opaque without thinning");
    let thinner = opaque_columns(&run(2.0, &img, 1.0), 2);
    let fatter = opaque_columns(&run(-2.0, &img, 1.0), 2);
    assert_eq!(thinner, 18, "Edge Thin +2 erodes the foreground edge by two pixels (the frame border stays)");
    assert_eq!(fatter, 22, "Edge Thin -2 grows the foreground two pixels into the screen");
}

#[test]
fn edge_thin_scales_with_the_working_resolution() {
    let img = screen_and_subject(64, 4, 24);
    // 2 source pixels at half resolution is 1 working pixel
    assert_eq!(opaque_columns(&run(2.0, &img, 0.5), 1), 40 - 1);
}

#[test]
fn edge_thin_does_not_spread_into_source_transparency() {
    let mut img = screen_and_subject(16, 3, 6);
    for p in img.px.chunks_mut(4).take(3) {
        p.copy_from_slice(&[0.0, 0.0, 0.0, 0.0]); // three transparent pixels at the left of row 0
    }
    let o = run(-3.0, &img, 1.0);
    for x in 0..3 {
        assert_eq!(o.px[x * 4 + 3], 0.0, "transparent source pixel {x} stays transparent");
    }
}

#[test]
fn hostile_edge_thin_values_and_sizes_stay_finite() {
    let img = screen_and_subject(9, 7, 3);
    for thin in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e30, -1e30, 5.0, -5.0] {
        for scale in [0.0f32, 1.0, 1e9, f32::NAN, f32::INFINITY, -3.0] {
            let o = run(thin, &img, scale);
            assert_eq!(o.px.len(), img.px.len());
            assert!(o.px.iter().all(|v| v.is_finite() && *v >= 0.0 && *v <= 1.0 + 1e-4), "thin {thin} scale {scale}");
        }
    }
    for (w, h) in [(0, 0), (1, 1), (1, 9), (9, 1)] {
        let tiny = screen_and_subject(w, h, 0);
        let _ = run(5.0, &tiny, 4.0);
        let _ = run(-5.0, &tiny, 4.0);
    }
}

#[test]
fn ultra_key_is_still_keyed_by_the_vfx_keyer() {
    let mut e = find_effect("ultra_key").map(|d| d.instance()).unwrap_or_else(|| panic!("ultra_key is registered"));
    if let Some(p) = e.params.get_mut("key_color") {
        p.value = ParamValue::Color([0.0, 0.8, 0.2, 1.0]);
    }
    let img = screen_and_subject(8, 2, 4);
    let mut o = img.clone();
    apply(&mut o, &e, &cx(1.0));
    assert!(o.px[3] < 0.05, "screen pixel keyed out: {:?}", &o.px[0..4]);
    assert!(o.px[(5) * 4 + 3] > 0.95, "subject pixel kept: {:?}", &o.px[20..24]);
}
