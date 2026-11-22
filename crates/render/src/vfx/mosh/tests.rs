//! DM2: Datamosh Bloom repeats one P picture of the clip, and leaves the rest of the clip alone.

use cfa4_project::{ParamValue, find_effect};

use super::*;

const W: usize = 64;
const H: usize = 48;
const FPS: f64 = 24.0;

/// A gradient with a grid of bright dots moving 2 px right per frame.
fn scene(frame: i64) -> Image {
    let mut img = Image::new(W, H);
    let dx = (frame.rem_euclid(1000) * 2) as usize;
    for y in 0..H {
        for x in 0..W {
            let sx = x + 4 * W - dx % W;
            let dot = if (sx % 8 < 3) && (y % 8 < 3) { 0.6 } else { 0.0 };
            let g = (x + y) as f32 / (W + H) as f32 * 0.4;
            let c = [0.45 + g + dot, 0.1 + g, 0.05 + dot * 0.5].map(|v: f32| v.min(1.0));
            let i = (y * W + x) * 4;
            img.px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 1.0]);
        }
    }
    img
}

/// The moving clip at `now` seconds; `missing` drops every other frame.
struct Clip {
    now: f64,
    missing: bool,
}

impl FxEnv for Clip {
    fn frame(&self, dt: f64, _: &EffectInstance) -> Option<Image> {
        let f = ((self.now + dt) * FPS).round() as i64;
        (!self.missing || f % 2 == 0).then(|| scene(f))
    }
    fn source_frame(&self, dt: f64, _: f32) -> Option<Image> {
        self.frame(dt, &find_effect("datamosh_bloom")?.instance())
    }
    fn track(&self, _: usize) -> Option<Image> {
        None
    }
    fn layer_to_output(&self) -> Affine {
        Affine::IDENTITY
    }
    fn clip_seconds(&self) -> f64 {
        10.0
    }
    fn clip_offset(&self) -> f64 {
        self.now
    }
    fn frame_rate(&self) -> f64 {
        FPS
    }
    fn media_timecode(&self) -> String {
        String::new()
    }
    fn file_name(&self) -> String {
        String::new()
    }
    fn sequence_name(&self) -> String {
        String::new()
    }
    fn sequence_size(&self) -> (u32, u32) {
        (W as u32, H as u32)
    }
    fn source_size(&self) -> (u32, u32) {
        (W as u32, H as u32)
    }
    fn clip_key(&self) -> u64 {
        7
    }
}

fn effect(params: &[(&str, f64)]) -> EffectInstance {
    let mut e = find_effect("datamosh_bloom").expect("datamosh_bloom").instance();
    for (k, v) in params {
        if let Some(p) = e.params.get_mut(*k) {
            p.value = ParamValue::Float(*v);
        }
    }
    e
}

/// Frame `f` of the clip with the effect.
fn render(e: &EffectInstance, f: i64, missing: bool) -> Image {
    let env = Clip { now: f as f64 / FPS, missing };
    let cx = FxCtx {
        t: Tick::ZERO,
        px_scale: 1.0,
        seconds: f as f64 / FPS,
        timecode: "",
        clip_name: "clip",
        project: None,
        env: Some(&env),
        frame_rate: FPS,
        working: cfa4_color::WorkingSpace::Rec709,
    };
    let mut img = scene(f);
    datamosh_bloom(&mut img, e, &cx);
    img
}

fn mean_abs(a: &Image, b: &Image) -> f32 {
    a.px.iter().zip(&b.px).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.px.len() as f32
}

#[test]
fn the_bloom_runs_for_repeats_frames_after_start_and_leaves_the_rest() {
    // start at frame 24 (1 s), 6 repeats: frames 25..=30 bloom
    let e = effect(&[("start", 1.0), ("repeats", 6.0), ("dissolve", 0.0)]);
    for f in [0, 23, 24, 31, 60] {
        assert_eq!(render(&e, f, false), scene(f), "frame {f} is untouched");
    }
    let start = scene(24);
    let mut prev_to_start = 0.0;
    for f in 25..=30 {
        let img = render(&e, f, false);
        assert!(img.px.iter().all(|v| v.is_finite()), "frame {f}");
        assert!(mean_abs(&img, &scene(f)) > 1e-3, "frame {f} blooms");
        // the bloom grows away from the start picture, repeat by repeat
        let d = mean_abs(&img, &start);
        assert!(d + 1e-3 >= prev_to_start, "frame {f}: {d} < {prev_to_start}");
        prev_to_start = d;
    }
}

#[test]
fn the_same_frame_is_the_same_picture_in_any_order() {
    let e = effect(&[("start", 0.5), ("repeats", 8.0)]);
    let forward: Vec<Image> = (13..=20).map(|f| render(&e, f, false)).collect();
    assert_eq!(render(&e, 20, false), forward[7]);
    assert_eq!(render(&e, 15, false), forward[2]);
    assert_eq!(render(&e, 18, false), forward[5]);
}

#[test]
fn dissolve_brings_the_clip_back_and_residual_changes_the_picture() {
    let plain = effect(&[("start", 1.0), ("repeats", 10.0), ("dissolve", 0.0)]);
    let fading = effect(&[("start", 1.0), ("repeats", 10.0), ("dissolve", 100.0)]);
    // at the last repeat a full dissolve is the clip itself
    assert!(mean_abs(&render(&fading, 34, false), &scene(34)) < 1e-6);
    assert!(mean_abs(&render(&plain, 34, false), &scene(34)) > 1e-3);
    let detailed = effect(&[("start", 1.0), ("repeats", 10.0), ("dissolve", 0.0), ("residual", 100.0)]);
    assert!(mean_abs(&render(&detailed, 30, false), &render(&plain, 30, false)) > 1e-4);
}

#[test]
fn hostile_params_and_missing_frames_never_panic() {
    for params in [
        vec![("start", f64::NAN)],
        vec![("start", -5.0), ("repeats", 1e9)],
        vec![("repeats", f64::NAN), ("residual", f64::INFINITY), ("dissolve", -1.0)],
        vec![("start", 1e12)],
    ] {
        let e = effect(&params);
        for f in [0, 1, 25, 30] {
            for missing in [false, true] {
                assert!(render(&e, f, missing).px.iter().all(|v| v.is_finite()));
            }
        }
    }
    // without the clip's other frames (adjustment layer, isolated test) nothing changes
    let mut img = scene(30);
    let cx = FxCtx {
        t: Tick::ZERO,
        px_scale: 1.0,
        seconds: 30.0 / FPS,
        timecode: "",
        clip_name: "clip",
        project: None,
        env: None,
        frame_rate: FPS,
        working: cfa4_color::WorkingSpace::Rec709,
    };
    datamosh_bloom(&mut img, &effect(&[]), &cx);
    assert_eq!(img, scene(30));
    assert_eq!(repeat_at(1.0, 1.0, 0.0, 5), None);
    assert_eq!(repeat_at(1.0 + 3.0 / FPS, 1.0, FPS, 5), Some(3));
}
