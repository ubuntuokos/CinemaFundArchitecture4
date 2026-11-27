//! Tests for Spill Suppress (`effects::spill_suppress`, CFA4-VFX-001 VFX-5 P0-1): alpha is never touched,
//! amount 0 is a no-op, and on the Z0 synthetic green screens it removes the spill that Color Key leaves
//! (the Z0 measurement's `spill` column). Numbers are synthetic; run `cargo test -p cfa4-render spill_ -- --nocapture`.

use cfa4_project::{ParamValue, find_effect};
use cfa4_time::Tick;

use crate::effects::{FxCtx, apply};
use crate::image::Image;
use crate::key_synth::{H, Scene, W, frame};

fn cx() -> FxCtx<'static> {
    FxCtx {
        t: Tick::ZERO,
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

const KEY: [f32; 4] = [0.05, 0.72, 0.18, 1.0];

fn set(e: &mut cfa4_project::EffectInstance, k: &str, v: ParamValue) {
    match e.params.get_mut(k) {
        Some(p) => p.value = v,
        None => panic!("no param {k}"),
    }
}

fn suppress(img: &Image, method: u32, amount: f64, luma: f64) -> Image {
    let mut e = find_effect("spill_suppress").map(|d| d.instance()).unwrap_or_else(|| panic!("spill_suppress is registered"));
    set(&mut e, "key_color", ParamValue::Color(KEY));
    set(&mut e, "method", ParamValue::Choice(method));
    set(&mut e, "amount", ParamValue::Float(amount));
    set(&mut e, "luma", ParamValue::Float(luma));
    let mut o = img.clone();
    apply(&mut o, &e, &cx());
    o
}

fn keyed(scene: Scene) -> Image {
    let (img, _) = frame(scene, 0);
    let mut e = find_effect("color_key").map(|d| d.instance()).unwrap_or_else(|| panic!("color_key is registered"));
    set(&mut e, "color", ParamValue::Color(KEY));
    set(&mut e, "tolerance", ParamValue::Float(40.0));
    set(&mut e, "feather", ParamValue::Float(10.0));
    let mut o = img;
    apply(&mut o, &e, &cx());
    o
}

/// Mean positive green excess `G - max(R, B)` over pixels with alpha above one half (the Z0 `spill` column).
fn spill(img: &Image) -> f32 {
    let (mut s, mut n) = (0.0, 0usize);
    for p in img.px.chunks(4) {
        if p[3] > 0.5 {
            s += (p[1] / p[3] - (p[0] / p[3]).max(p[2] / p[3])).max(0.0);
            n += 1;
        }
    }
    if n == 0 { 0.0 } else { s / n as f32 }
}

#[test]
fn spill_amount_zero_is_a_bit_identical_no_op() {
    let img = keyed(Scene::Disc);
    assert_eq!(suppress(&img, 0, 0.0, 0.0).px, img.px);
}

#[test]
fn spill_never_changes_alpha_and_skips_transparent_pixels() {
    let img = keyed(Scene::Hair);
    let out = suppress(&img, 1, 100.0, 50.0);
    for (a, b) in img.px.chunks(4).zip(out.px.chunks(4)) {
        assert_eq!(a[3].to_bits(), b[3].to_bits(), "alpha moved");
        if a[3] <= 0.0 {
            assert_eq!(a, b, "a transparent pixel changed");
        }
    }
}

#[test]
fn spill_leaves_a_neutral_pixel_alone_and_pulls_a_green_cast_down() {
    let neutral = Image { w: 1, h: 1, px: vec![0.2, 0.2, 0.2, 1.0] };
    let o = suppress(&neutral, 1, 100.0, 0.0);
    for k in 0..3 {
        assert!((o.px[k] - 0.2).abs() < 2e-3, "neutral channel {k} moved to {}", o.px[k]);
    }
    let cast = Image { w: 1, h: 1, px: vec![0.30, 0.45, 0.20, 1.0] };
    let o = suppress(&cast, 1, 100.0, 0.0);
    assert!(o.px[1] <= 0.30 + 2e-3, "green is not limited to max(R,B): {}", o.px[1]);
    assert!((o.px[0] - 0.30).abs() < 2e-3 && (o.px[2] - 0.20).abs() < 2e-3, "R and B must not move");
}

#[test]
fn spill_average_keeps_more_green_than_maximum_and_luma_keep_adds_light() {
    let cast = Image { w: 1, h: 1, px: vec![0.30, 0.45, 0.10, 1.0] };
    let avg = suppress(&cast, 0, 100.0, 0.0);
    let max = suppress(&cast, 1, 100.0, 0.0);
    assert!(avg.px[1] < max.px[1], "Average limits harder than Maximum when R > B");
    let keep = suppress(&cast, 1, 100.0, 100.0);
    assert!(keep.px[0] > max.px[0], "Preserve Luminance must add the removed light back");
}

#[test]
fn spill_measure_on_the_z0_scenes() {
    println!("scene | spill after Color Key | after Spill Suppress (Maximum) | after (Average)");
    for (sc, name) in [(Scene::Disc, "disc"), (Scene::Hair, "hair"), (Scene::Noisy, "noisy")] {
        let img = keyed(sc);
        let before = spill(&img);
        let max = suppress(&img, 1, 100.0, 0.0);
        let avg = suppress(&img, 0, 100.0, 0.0);
        println!("{name:<5} | {before:.4} | {:.4} | {:.4}", spill(&max), spill(&avg));
        assert_eq!(max.px.len(), W * H * 4);
        assert!(spill(&max) <= before * 0.05 + 1e-4, "{name}: Maximum leaves spill {} of {before}", spill(&max));
        assert!(spill(&avg) <= before * 0.05 + 1e-4, "{name}: Average leaves spill {} of {before}", spill(&avg));
    }
}
