//! AC-14 meter (CFA4-RENDER-INTEGRATION-PLAN-001, F0V): the picture the program monitor shows and
//! the picture an export writes for the same frame must differ by no more than ΔE₀₀ ≤ 1.0 (the
//! "just noticeable" limit). Measured here on the CPU path with default export settings; the GPU
//! path is PENDING (no adapter in CI, see `docs/audit/render-f0v/F0V-ac-meter-map.md`), and an
//! unmeasured path is never reported as PASS.
//!
//! The preview is `render_sequence` with `captions: true` and the monitor's `over_black_rgba8`
//! (what `Session::render_program_with` does); the export is `cfa4_export::Pipeline::frame`.
//! A mutation check turns on a legitimate export-only setting (the video limiter) and requires the
//! meter to fail, so a meter that cannot fail cannot pass either.

use std::sync::Arc;

use cfa4_export::{ExportSettings, GpuRendering, Pipeline, VideoLimiter};
use cfa4_media::generators::GeneratorSource;
use cfa4_media::{DemoScene, Generator, MediaSource, SharedSource};
use cfa4_metrics::delta_e_srgb8;
use cfa4_project::{ItemId, ItemKind, Label, MediaClip, MediaRef, Project, SequenceSettings, TrackKind};
use cfa4_render::{RenderOptions, SourceMap, render_sequence};
use cfa4_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};

const W: u32 = 320;
const H: u32 = 180;
const RATE: FrameRate = FrameRate::FPS_24;
/// The AC-14 limit.
const LIMIT_DE: f64 = 1.0;

fn scene(demo: DemoScene) -> (Arc<Project>, ItemId, SourceMap) {
    let mut p = Project::new("ac14");
    let seq = p.new_sequence("ac14", SequenceSettings { width: W, height: H, frame_rate: RATE, ..Default::default() }, 1, 0, None);
    let src = GeneratorSource::new(Generator::Demo(demo), 640, 360, RATE, Tick(10 * TICKS_PER_SECOND));
    let info = src.info().clone();
    let id = p.add_item(
        &info.name.clone(),
        Label::Iris,
        ItemKind::Media(MediaClip {
            media: MediaRef::Generator(src.generator.clone()),
            info,
            interpret: Default::default(),
            mark_in: None,
            mark_out: None,
            markers: vec![],
            offline: false,
            proxy: None,
            identity: None,
        }),
        None,
    );
    let mut ti = p.make_track_item(id, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, RATE.tick_of(48)), RATE).unwrap();
    ti.scale_to_frame = true;
    p.sequence_mut(seq).unwrap().video_tracks[0].items.push(ti);
    let mut map = SourceMap::default();
    map.0.insert(id, Arc::new(src) as SharedSource);
    (Arc::new(p), seq, map)
}

fn settings(scale: f32, limiter: bool) -> ExportSettings {
    let mut s = ExportSettings { scale, gpu_rendering: GpuRendering::Off, ..Default::default() };
    s.effects.video_limiter = VideoLimiter { enabled: limiter, min_percent: 10.0, max_percent: 90.0 };
    s
}

/// ΔE₀₀ (mean, max) between the monitor picture and the exported picture of frame `f`.
fn parity(demo: DemoScene, f: i64, scale: f32, limiter: bool) -> (f64, f64) {
    let (project, seq, map) = scene(demo);
    let preview = render_sequence(&project, seq, RATE.tick_of(f), RenderOptions { scale, captions: true, ..Default::default() }, &map);
    let (w, h) = (preview.w, preview.h);
    let shown = preview.over_black_rgba8();
    let pipe = Pipeline::new(project, seq, &settings(scale, limiter), false).expect("export pipeline");
    let (exported, _) = pipe.frame(f, &map);
    assert_eq!(exported.len(), shown.len(), "the export and the monitor picture must have the same size");
    let d = delta_e_srgb8(&shown, &exported, w, h, 4).expect("ΔE₀₀");
    (d.mean, d.max)
}

#[test]
fn the_exported_frame_matches_the_monitor_frame_within_one_delta_e() {
    let mut worst = 0f64;
    for demo in [DemoScene::OceanSunset, DemoScene::CityNight, DemoScene::Plasma] {
        for (f, scale) in [(0, 1.0), (17, 1.0), (30, 0.5)] {
            let (mean, max) = parity(demo, f, scale, false);
            eprintln!("AC-14 CPU {demo:?} frame {f} scale {scale}: ΔE₀₀ mean {mean:.4} max {max:.4}");
            worst = worst.max(max);
            assert!(max <= LIMIT_DE, "{demo:?} frame {f} scale {scale}: max ΔE₀₀ {max:.3} exceeds {LIMIT_DE}");
        }
    }
    eprintln!("AC-14 CPU worst max ΔE₀₀ {worst:.4} (limit {LIMIT_DE}); GPU path: PENDING (no adapter measured)");
}

#[test]
fn the_meter_fails_when_the_export_picture_really_differs() {
    // The limiter is an export-only setting: with 10–90 % it must move pixels by more than ΔE 1.
    let (_, max) = parity(DemoScene::OceanSunset, 17, 1.0, true);
    assert!(max > LIMIT_DE, "the meter missed a real difference (max ΔE₀₀ {max:.3})");
}
