//! Render validation harness on the real CPU renderer (CFA4-RENDER-INTEGRATION-PLAN-001, F0V).
//!
//! A procedural project (a scaled, half-transparent picture and a tone, no media files) is rendered
//! through the CPU compositor and the audio mixer into a [`Capture`]. The tests then
//!
//! 1. render it ten times and measure the run-to-run noise ([`measure_noise`]);
//! 2. derive the thresholds from that noise and compare them with the committed
//!    `docs/audit/render-f0v/F0V-thresholds.json` (`CFA4_BLESS=1` rewrites it);
//! 3. check that an unchanged build gives no false failure in ten runs against the first one;
//! 4. damage the capture in the six ways of the plan and require every one to be caught.
//!
//! The committed thresholds are the ones of the **CPU path, compared on one machine**: the noise of
//! a render repeated in one process. Comparing across machines or against the GPU needs the noise of
//! those pairs, measured on the reference machine (docs/audit/RENDER-F0V-*.md, PENDING there).
//!
//! Every run writes its report to `<workspace>/target/f0v-report/` (JSON and Markdown).

use std::path::PathBuf;
use std::sync::Arc;

use cfa4_golden::validation::{Capture, FrameCapture, Metadata, Report, Thresholds, compare, derive_thresholds, measure_noise, measured_failure, mutation};
use cfa4_media::generators::GeneratorSource;
use cfa4_media::{DemoScene, Generator, MediaSource, SharedSource};
use cfa4_project::{ItemKind, Label, MediaClip, MediaRef, ParamValue, Project, SequenceSettings, TrackKind};
use cfa4_render::audio::mix_sequence;
use cfa4_render::{Image, RenderOptions, SourceMap, render_sequence};
use cfa4_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};

const W: u32 = 320;
const H: u32 = 180;
const RATE: FrameRate = FrameRate::FPS_24;
const FRAMES: i64 = 12;
const SAMPLE_RATE: u32 = 48_000;

struct Scene {
    project: Project,
    seq: cfa4_project::ItemId,
    sources: SourceMap,
}

fn add_media(p: &mut Project, map: &mut SourceMap, g: Generator) -> cfa4_project::ItemId {
    let src = GeneratorSource::new(g, 640, 360, RATE, Tick(10 * TICKS_PER_SECOND));
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
    map.0.insert(id, Arc::new(src) as SharedSource);
    id
}

/// One video track (a picture at 45 % scale, 70 % opacity: transparent border, partly transparent
/// picture) and one audio track (a 440 Hz tone), 48 frames long.
fn scene() -> Scene {
    let mut p = Project::new("f0v");
    let seq = p.new_sequence("f0v", SequenceSettings { width: W, height: H, frame_rate: RATE, sample_rate: SAMPLE_RATE, ..Default::default() }, 1, 1, None);
    let mut sources = SourceMap::default();
    let picture = add_media(&mut p, &mut sources, Generator::Demo(DemoScene::Aurora));
    let tone = add_media(&mut p, &mut sources, Generator::Tone { hz: 440.0, db: -12.0 });
    let dur = TimeRange::new(Tick::ZERO, RATE.tick_of(48));

    let mut v = p.make_track_item(picture, TrackKind::Video, Tick::ZERO, dur, RATE).expect("video clip");
    v.scale_to_frame = true;
    for (effect, params) in
        [("motion", vec![("scale", ParamValue::Float(45.0)), ("rotation", ParamValue::Float(20.0))]), ("opacity", vec![("opacity", ParamValue::Float(70.0))])]
    {
        let e = v.effect_mut(effect).expect("intrinsic effect");
        for (k, val) in params {
            e.param_mut(k).expect("parameter").value = val;
        }
    }
    let a = p.make_track_item(tone, TrackKind::Audio, Tick::ZERO, dur, RATE).expect("audio clip");
    let s = p.sequence_mut(seq).expect("sequence");
    s.video_tracks[0].items.push(v);
    s.audio_tracks[0].items.push(a);
    Scene { project: p, seq, sources }
}

fn frame_capture(img: &Image) -> FrameCapture {
    let enc = |v: f32| (cfa4_color::linear_to_srgb(v.clamp(0.0, 1.0)) * 65_535.0).round() as u16;
    let mut px = Vec::with_capacity(img.w * img.h * 4);
    for p in img.px.chunks_exact(4) {
        let a = p[3];
        let (r, g, b) = if a > 0.0 { (p[0] / a, p[1] / a, p[2] / a) } else { (0.0, 0.0, 0.0) };
        px.extend([enc(r), enc(g), enc(b), (a.clamp(0.0, 1.0) * 65_535.0).round() as u16]);
    }
    FrameCapture::new(img.w, img.h, px).expect("rendered frame size")
}

/// One complete run: build the project, render the frames and the audio.
fn run() -> Capture {
    let s = scene();
    let frames: Vec<FrameCapture> =
        (0..FRAMES).map(|f| frame_capture(&render_sequence(&s.project, s.seq, RATE.tick_of(f), RenderOptions::default(), &s.sources))).collect();
    let seq = s.project.sequence(s.seq).expect("sequence");
    let samples = (FRAMES as usize) * SAMPLE_RATE as usize / 24;
    let mixed = mix_sequence(&s.project, seq, 0, samples, &s.sources);
    Capture {
        frames,
        pts: (0..FRAMES).map(|f| RATE.tick_of(f).0).collect(),
        frame_duration: RATE.frame_duration().0,
        audio: mixed.channels,
        sample_rate: SAMPLE_RATE,
    }
}

fn thresholds_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/audit/render-f0v/F0V-thresholds.json")
}

fn bless() -> bool {
    std::env::var("CFA4_BLESS").is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
}

fn ten_runs() -> Vec<Capture> {
    (0..10).map(|_| run()).collect()
}

fn write_report(name: &str, r: &Report) {
    let dir = cfa4_testkit::workspace_root().join("target/f0v-report");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(format!("{name}.json")), r.to_json());
    let _ = std::fs::write(dir.join(format!("{name}.md")), r.to_markdown());
}

#[test]
fn the_scene_exercises_what_the_mutations_damage() {
    let c = run();
    assert_eq!(c.frames.len(), FRAMES as usize);
    let alpha = c.frames[0].plane(3);
    assert!(alpha.iter().any(|&a| a < u16::MAX), "no transparent or partly transparent pixel: alpha loss would be invisible");
    assert!(alpha.iter().any(|&a| a > 0), "an empty frame proves nothing");
    assert_eq!(c.audio.len(), 2, "stereo mix");
    assert!(c.audio.iter().all(|ch| ch.iter().any(|&s| s.abs() > 1e-3)), "silent audio: a muted channel would be invisible");
    assert!(c.frames.windows(2).any(|w| w[0] != w[1]), "the picture never changes: a dropped frame would be invisible");
}

#[test]
fn ten_cpu_runs_are_bit_identical_and_give_the_committed_thresholds() {
    let runs = ten_runs();
    let noise = measure_noise(&runs).expect("noise");
    eprintln!("noise over {} runs: {noise:?}", noise.runs);
    assert!(noise.bit_identical, "the CPU render is not bit-identical between runs (MQ-6): {noise:?}");
    let derived = derive_thresholds(&noise);
    let path = thresholds_path();
    if bless() {
        std::fs::write(&path, derived.to_json()).expect("write thresholds");
        eprintln!("blessed {}", path.display());
        return;
    }
    let committed = Thresholds::from_json(&std::fs::read_to_string(&path).expect("docs/audit/render-f0v/F0V-thresholds.json (CFA4_BLESS=1 writes it)"))
        .expect("thresholds file");
    assert_eq!(committed, derived, "the measured noise now gives other thresholds: review, then CFA4_BLESS=1");
}

#[test]
fn an_unchanged_build_gives_no_false_failure_in_ten_runs() {
    let runs = ten_runs();
    let t = Thresholds::from_json(&std::fs::read_to_string(thresholds_path()).expect("thresholds file")).expect("thresholds");
    let (first, rest) = runs.split_first().expect("runs");
    for (i, r) in rest.iter().enumerate() {
        let checks = compare(first, r, &t);
        assert!(!measured_failure(&checks), "false FAIL in run {}: {checks:#?}", i + 2);
    }
    let checks = compare(first, &runs[1], &t);
    write_report("unchanged-head", &Report::new("unchanged HEAD, run 2 against run 1", Metadata::collect(first, &t), checks));
}

#[test]
fn every_mutation_of_the_real_render_is_caught() {
    let c = run();
    let t = Thresholds::from_json(&std::fs::read_to_string(thresholds_path()).expect("thresholds file")).expect("thresholds");
    let outcomes = mutation::self_test(&c, &t);
    let mut md = String::from("| Mutation | Changed the capture | Caught | Failed checks |\n|---|---|---|---|\n");
    for o in &outcomes {
        md.push_str(&format!("| `{}` | {} | {} | {} |\n", o.mutation.name(), o.changed, o.caught, o.failed_checks.join(", ")));
    }
    let dir = cfa4_testkit::workspace_root().join("target/f0v-report");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("mutation-self-test.md"), &md);
    eprintln!("{md}");
    let missed: Vec<_> = outcomes.iter().filter(|o| !o.proved()).map(|o| o.mutation.name()).collect();
    assert!(missed.is_empty(), "mutations not proved: {missed:?}");
}
