//! DM1: the Datamosh transition keeps the transition contract and really moshes.

use cfa4_project::{ParamValue, find_effect};

use super::*;

/// A soft gradient with a grid of bright dots, shifted `dx` pixels right.
fn scene(w: usize, h: usize, dx: usize, hue: [f32; 3]) -> Image {
    let mut img = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let sx = x + w - dx % w;
            let dot = if (sx % 8 < 3) && (y % 8 < 3) { 0.6 } else { 0.0 };
            let g = (x + y) as f32 / (w + h) as f32 * 0.4;
            let c = hue.map(|k| (k * 0.5 + g + dot).min(1.0));
            let i = (y * w + x) * 4;
            img.px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 1.0]);
        }
    }
    img
}

fn mean_abs(a: &Image, b: &Image) -> f32 {
    a.px.iter().zip(&b.px).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.px.len() as f32
}

const W: usize = 64;
const H: usize = 48;

fn b_frame(k: usize) -> Image {
    scene(W, H, k * 2, [0.1, 0.2, 0.9])
}

/// Render picture `n` of a mosh with session key `key`.
fn render(e: &EffectInstance, key: u64, n: usize, p: f32) -> Image {
    let a0 = scene(W, H, 0, [0.9, 0.1, 0.1]);
    let mut b = |k: usize| Some(b_frame(k));
    let mut src = MoshSource { key, a: &a0, n, b: &mut b };
    apply_mosh(e, &a0, &b_frame(n), p, &mut src)
}

fn set(e: &mut EffectInstance, id: &str, v: ParamValue) {
    if let Some(p) = e.params.get_mut(id) {
        p.value = v;
    }
}

fn instance() -> EffectInstance {
    let mut e = find_effect("datamosh").expect("datamosh").instance();
    set(&mut e, "recovery", ParamValue::Float(0.0));
    e
}

#[test]
fn ends_are_exact_and_the_middle_is_a_moved_by_b() {
    let e = instance();
    let a0 = scene(W, H, 0, [0.9, 0.1, 0.1]);
    assert_eq!(render(&e, 1, 3, 0.0), a0);
    assert_eq!(render(&e, 1, 3, 1.0), b_frame(3));
    // the first moshed picture is A moved by B's motion, not B
    let m = render(&e, 2, 1, 0.1);
    assert!(m.px.iter().all(|v| v.is_finite()));
    let (to_a, to_b) = (mean_abs(&m, &a0), mean_abs(&m, &b_frame(1)));
    assert!(to_a < to_b * 0.5, "to A {to_a}, to B {to_b}");
    // and it is not the dissolve the transition falls back to
    assert!(mean_abs(&m, &dissolve(&a0, &b_frame(1), 0.1)) > 1e-3);
}

#[test]
fn playing_forward_matches_starting_fresh_and_is_deterministic() {
    let e = instance();
    let stepped: Vec<Image> = (1..=4).map(|n| render(&e, 10, n, 0.5)).collect();
    // the same picture again (a paused frame) comes from the kept session
    assert_eq!(render(&e, 10, 4, 0.5), stepped[3]);
    // a fresh session that jumps straight to picture 4, and one that goes backwards
    assert_eq!(render(&e, 11, 4, 0.5), stepped[3]);
    assert_eq!(render(&e, 10, 2, 0.5), stepped[1]);
}

#[test]
fn recovery_dissolves_into_b_at_the_end() {
    let mut e = instance();
    set(&mut e, "recovery", ParamValue::Float(50.0));
    let late = render(&e, 20, 5, 0.95);
    let early = render(&e, 21, 5, 0.4);
    assert!(mean_abs(&late, &b_frame(5)) < mean_abs(&early, &b_frame(5)) * 0.5);
}

#[test]
fn hostile_params_and_bad_frames_never_panic() {
    let mut e = find_effect("datamosh").expect("datamosh").instance();
    set(&mut e, "residual", ParamValue::Float(f64::NAN));
    set(&mut e, "recovery", ParamValue::Float(-50.0));
    set(&mut e, "resolution", ParamValue::Choice(99));
    for (n, p) in [(0, 0.5), (1, f32::NAN), (5, 0.5), (100_000, 0.7)] {
        let out = render(&e, 30, n, p);
        assert!(out.px.iter().all(|v| v.is_finite()));
    }
    // every resolution, and a tiny frame
    for r in 0..3 {
        set(&mut e, "resolution", ParamValue::Choice(r));
        let a = scene(3, 1, 0, [0.5; 3]);
        let mut b = |k: usize| Some(scene(3, 1, k, [0.2; 3]));
        let mut src = MoshSource { key: 31, a: &a, n: 2, b: &mut b };
        let out = apply_mosh(&e, &a, &scene(3, 1, 2, [0.2; 3]), 0.5, &mut src);
        assert_eq!((out.w, out.h), (3, 1));
        assert!(out.px.iter().all(|v| v.is_finite()));
    }
    // B pictures of the wrong size or missing, and a reference of the wrong size: still a picture
    let a0 = scene(W, H, 0, [0.9, 0.1, 0.1]);
    let mut b = |k: usize| if k.is_multiple_of(2) { None } else { Some(scene(8, 8, k, [0.2; 3])) };
    let mut src = MoshSource { key: 32, a: &a0, n: 3, b: &mut b };
    assert!(apply_mosh(&e, &a0, &b_frame(3), 0.5, &mut src).px.iter().all(|v| v.is_finite()));
    let small = scene(8, 8, 0, [0.5; 3]);
    let mut b = |_: usize| None;
    let mut src = MoshSource { key: 33, a: &small, n: 3, b: &mut b };
    assert_eq!(apply_mosh(&e, &a0, &b_frame(3), 0.5, &mut src), dissolve(&a0, &b_frame(3), 0.5));
}

#[test]
fn residual_maps_to_the_quantizer_and_long_transitions_are_capped() {
    assert_eq!(qp_for(100.0), 18);
    assert_eq!(qp_for(0.0), 51);
    assert_eq!(qp_for(f32::NAN), 51);
    assert_eq!(picture_index(10, 30, 100.0), (10, 1.0));
    assert_eq!(picture_index(10, 30, 200.0), (10, 2.0));
    let (n, step) = picture_index(10_000, 10_000, 100.0);
    assert_eq!(n, MAX_FRAMES - 1);
    assert!(step > 41.0 && step < 42.0);
    assert_eq!(picture_index(5, 30, f32::NAN).1, 1.0);
}

#[test]
fn every_parameter_changes_the_moving_picture() {
    let base = instance();
    let pic = |e: &EffectInstance, key: u64, p: f32| render(e, key, 5, p);
    let reference = pic(&base, 40, 0.5);
    for (i, (id, v)) in
        [("residual", ParamValue::Float(100.0)), ("resolution", ParamValue::Choice(2)), ("recovery", ParamValue::Float(80.0))].into_iter().enumerate()
    {
        let mut e = base.clone();
        set(&mut e, id, v);
        let got = pic(&e, 41 + i as u64, 0.5);
        assert!(mean_abs(&got, &reference) > 1e-4, "{id} has no visible effect");
    }
}
