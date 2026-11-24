//! Z0 measurement (CFA4-GREENSCREEN-MATTING-INTEGRATION-001): how well Ultra Key and Color Key recover a
//! known matte from synthetic green-screen pictures. Run `cargo test -p cfa4-render key_measure -- --nocapture`
//! to print the table that `docs/audit/GREENSCREEN-Z0-KEYER-MEASUREMENT-2026-11-24.md` quotes.
//! The assertions are sanity bounds only (finite, ordered); the numbers are the deliverable.

use cfa4_project::{ParamValue, find_effect};
use cfa4_time::Tick;

use crate::effects::{FxCtx, apply, dec};
use crate::image::Image;

const W: usize = 192;
const H: usize = 128;

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

#[derive(Clone, Copy)]
enum Scene {
    /// Soft-edged disc, even screen.
    Disc,
    /// Same, screen brightness varies ±25 % across the frame.
    Uneven,
    /// Same, with 2 % sensor noise on every channel.
    Noisy,
    /// One-pixel vertical stripes (hair) over the screen.
    Hair,
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 40) as f32 / (1u64 << 24) as f32) - 0.5
    }
}

/// Returns the keyed-source picture (display-encoded composite, linear premultiplied) and the true alpha.
fn make(scene: Scene) -> (Image, Vec<f32>) {
    let mut rng = Rng(0x9E3779B97F4A7C15);
    let mut truth = vec![0.0f32; W * H];
    let mut img = Image::new(W, H);
    let fg = [0.45f32, 0.40, 0.38]; // muted subject, so that spill makes green the dominant channel
    for y in 0..H {
        for x in 0..W {
            let (dx, dy) = (x as f32 - 96.0, y as f32 - 64.0);
            let d = (dx * dx + dy * dy).sqrt();
            let mut a = match scene {
                Scene::Hair => {
                    if x % 6 == 3 && (24..104).contains(&y) {
                        1.0
                    } else if x % 6 == 2 || x % 6 == 4 {
                        0.0
                    } else {
                        0.0
                    }
                }
                _ => ((46.0 - d) / 6.0).clamp(0.0, 1.0), // 6 px soft edge
            };
            a = a.clamp(0.0, 1.0);
            let k = match scene {
                Scene::Uneven => 0.75 + 0.5 * (x as f32 / W as f32),
                _ => 1.0,
            };
            let screen = [0.05 * k, 0.72 * k, 0.18 * k];
            // spill: foreground picks up green, strongest near the edge
            let spill = if a > 0.0 { 0.30 * (1.0 - ((46.0 - d) / 30.0).clamp(0.0, 1.0)) } else { 0.0 };
            let mut c = [0.0f32; 3];
            for i in 0..3 {
                let f = if i == 1 { fg[i] + spill } else { fg[i] - spill * 0.2 };
                c[i] = a * f + (1.0 - a) * screen[i];
            }
            if matches!(scene, Scene::Noisy) {
                for v in &mut c {
                    *v += rng.next() * 0.04;
                }
            }
            let l = dec([c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0)]);
            let o = (y * W + x) * 4;
            img.px[o..o + 4].copy_from_slice(&[l[0], l[1], l[2], 1.0]);
            truth[y * W + x] = a;
        }
    }
    (img, truth)
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

#[test]
fn key_measure_synthetic_green_screens() {
    let key = [0.05f32, 0.72, 0.18, 1.0];
    let names = [(Scene::Disc, "disc"), (Scene::Uneven, "uneven"), (Scene::Noisy, "noisy"), (Scene::Hair, "hair")];
    println!("scene   | keyer            |   MAE  | edgeMAE |  leak  |  hole  |  spill");
    for (sc, sn) in names {
        let (img, truth) = make(sc);
        let runs: Vec<(&str, Image)> = vec![
            ("ultra default", run("ultra_key", |e| set(e, "key_color", ParamValue::Color(key)), &img)),
            (
                "ultra relaxed",
                run(
                    "ultra_key",
                    |e| {
                        set(e, "key_color", ParamValue::Color(key));
                        set(e, "setting", ParamValue::Choice(1));
                    },
                    &img,
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
                    &img,
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
                    &img,
                ),
            ),
        ];
        for (n, out) in runs {
            let s = score(&out, &truth);
            assert!(s.mae.is_finite() && s.mae <= 1.0, "{sn}/{n}: matte error is not a probability");
            println!("{sn:<7} | {n:<16} | {:.4} | {:.4}  | {:.4} | {:.4} | {:.4}", s.mae, s.edge_mae, s.leak, s.hole, s.spill);
        }
    }
}
