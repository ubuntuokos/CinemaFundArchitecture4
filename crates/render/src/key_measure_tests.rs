//! Z0 measurement (CFA4-GREENSCREEN-MATTING-INTEGRATION-001): how well Ultra Key and Color Key recover a
//! known matte from synthetic green-screen pictures. Run `cargo test -p cfa4-render key_measure -- --nocapture`
//! to print the table that `docs/audit/GREENSCREEN-Z0-KEYER-MEASUREMENT-2026-11-24.md` quotes.
//! The assertions are sanity bounds only (finite, ordered); the numbers are the deliverable.

use cfa4_project::{ParamValue, find_effect};
use cfa4_time::Tick;

use crate::effects::{FxCtx, apply};
use crate::image::Image;
use crate::key_synth::{FPS, H, Scene, W, frame};

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

struct Score {
    mae: f32,
    edge_mae: f32,
    leak: f32,
    hole: f32,
    spill: f32,
}

fn score(out: &Image, truth: &[f32]) -> Score {
    let (mut s, mut e, mut en, mut leak, mut ln, mut hole, mut hn, mut sp, mut sn) = (0.0, 0.0, 0, 0.0, 0, 0.0, 0, 0.0, 0);
    for i in 0..W * H {
        let a = out.px[i * 4 + 3];
        let t = truth[i];
        let err = (a - t).abs();
        s += err;
        if t > 0.02 && t < 0.98 {
            e += err;
            en += 1;
        }
        if t <= 0.02 {
            leak += a;
            ln += 1;
        }
        if t >= 0.98 {
            hole += 1.0 - a;
            hn += 1;
        }
        if t >= 0.98 && a > 0.5 {
            let p = [out.px[i * 4], out.px[i * 4 + 1], out.px[i * 4 + 2]].map(|v| v / a);
            sp += (p[1] - p[0].max(p[2])).max(0.0);
            sn += 1;
        }
    }
    let m = |v: f32, n: usize| if n == 0 { 0.0 } else { v / n as f32 };
    Score { mae: s / (W * H) as f32, edge_mae: m(e, en), leak: m(leak, ln), hole: m(hole, hn), spill: m(sp, sn) }
}

fn set(e: &mut cfa4_project::EffectInstance, k: &str, v: ParamValue) {
    match e.params.get_mut(k) {
        Some(p) => p.value = v,
        None => panic!("no param {k}"),
    }
}

fn run(name: &str, tune: impl Fn(&mut cfa4_project::EffectInstance), img: &Image) -> Image {
    let mut e = find_effect(name).map(|d| d.instance()).unwrap_or_else(|| panic!("{name} is registered"));
    tune(&mut e);
    let mut o = img.clone();
    apply(&mut o, &e, &cx());
    o
}

fn keyers(key: [f32; 4], img: &Image) -> Vec<(&'static str, Image)> {
    vec![
        ("ultra default", run("ultra_key", |e| set(e, "key_color", ParamValue::Color(key)), img)),
        (
            "ultra relaxed",
            run(
                "ultra_key",
                |e| {
                    set(e, "key_color", ParamValue::Color(key));
                    set(e, "setting", ParamValue::Choice(1));
                },
                img,
            ),
        ),
        (
            "ultra aggressive",
            run(
                "ultra_key",
                |e| {
                    set(e, "key_color", ParamValue::Color(key));
                    set(e, "setting", ParamValue::Choice(2));
                },
                img,
            ),
        ),
        (
            "color key tol 40",
            run(
                "color_key",
                |e| {
                    set(e, "color", ParamValue::Color(key));
                    set(e, "tolerance", ParamValue::Float(40.0));
                    set(e, "feather", ParamValue::Float(10.0));
                },
                img,
            ),
        ),
    ]
}

const KEY: [f32; 4] = [0.05, 0.72, 0.18, 1.0];

#[test]
fn key_measure_synthetic_green_screens() {
    let scenes = [(Scene::Disc, "disc"), (Scene::Uneven, "uneven"), (Scene::Noisy, "noisy"), (Scene::Hair, "hair"), (Scene::Smoke, "smoke")];
    println!("scene   | keyer            |   MAE  | edgeMAE |  leak  |  hole  |  spill");
    for (sc, sn) in scenes {
        let (img, truth) = frame(sc, 0);
        for (n, out) in keyers(KEY, &img) {
            let s = score(&out, &truth);
            assert!(s.mae.is_finite() && s.mae <= 1.0, "{sn}/{n}: matte error is not a probability");
            println!("{sn:<7} | {n:<16} | {:.4} | {:.4}  | {:.4} | {:.4} | {:.4}", s.mae, s.edge_mae, s.leak, s.hole, s.spill);
        }
    }
}

/// Moving subject with noise over 12 frames (half a second): mean error per keyer, and temporal flicker of the matte
/// (mean absolute alpha change between consecutive frames, against the change in the truth).
#[test]
fn key_measure_motion_and_flicker() {
    const FRAMES: usize = 12;
    let names = ["ultra default", "ultra relaxed", "ultra aggressive", "color key tol 40"];
    let mut mae = [0.0f32; 4];
    let mut flicker = [0.0f32; 4];
    let mut prev: Option<Vec<Vec<f32>>> = None;
    let mut prev_truth: Option<Vec<f32>> = None;
    let mut truth_change = 0.0f32;
    for f in 0..FRAMES {
        let (img, truth) = frame(Scene::Motion, f);
        let alphas: Vec<Vec<f32>> = keyers(KEY, &img)
            .into_iter()
            .enumerate()
            .map(|(i, (_, out))| {
                mae[i] += score(&out, &truth).mae / FRAMES as f32;
                out.px.chunks(4).map(|p| p[3]).collect()
            })
            .collect();
        if let (Some(pa), Some(pt)) = (&prev, &prev_truth) {
            for i in 0..4 {
                flicker[i] += alphas[i].iter().zip(&pa[i]).map(|(a, b)| (a - b).abs()).sum::<f32>() / (W * H) as f32 / (FRAMES - 1) as f32;
            }
            truth_change += truth.iter().zip(pt).map(|(a, b)| (a - b).abs()).sum::<f32>() / (W * H) as f32 / (FRAMES - 1) as f32;
        }
        prev = Some(alphas);
        prev_truth = Some(truth);
    }
    println!("motion ({FRAMES} frames at {FPS} fps) | keyer            |  MAE  | mean |dAlpha| between frames (truth: {truth_change:.4})");
    for i in 0..4 {
        assert!(mae[i].is_finite() && flicker[i].is_finite());
        println!("motion  | {:<16} | {:.4} | {:.4}", names[i], mae[i], flicker[i]);
    }
}
