//! A/V drift of the engine render of the flash-and-beep fixture (CFA4-RENDER-INTEGRATION-PLAN-001,
//! F0V; AC-05).
//!
//! `sync_flash_beep` (python/cfa4_devtools/render_fixtures.py) shows a white flash on frame 0 of
//! every second and sounds a 1 kHz beep at the same instants. The test decodes the file with our own
//! decoders, renders it through the CPU compositor and the audio mixer, and measures where the
//! flashes and the beeps fall in the *render* with the `sync.av_drift` check of the harness. The
//! source file is the reference: a render that moves the audio against the picture fails.
//!
//! Needs `ffmpeg` to generate the fixture; without it the test prints SKIPPED and passes, unless
//! `CFA4_REQUIRE_ORACLES=1` (CI), which makes the skip a failure.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use cfa4_golden::validation::{Capture, Extras, FrameCapture, Status, SyncSpec, Thresholds, compare_with};
use cfa4_media::SharedSource;
use cfa4_project::{ItemKind, Label, MediaClip, MediaRef, Project, SequenceSettings, TrackKind};
use cfa4_render::audio::mix_sequence;
use cfa4_render::{Image, RenderOptions, SourceMap, render_sequence};
use cfa4_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};

const RATE: FrameRate = FrameRate::FPS_25;
const SECONDS: i64 = 4;
const FRAMES: i64 = RATE_FPS * SECONDS;
const RATE_FPS: i64 = 25;
const SAMPLE_RATE: u32 = 48_000;

fn require_oracles() -> bool {
    std::env::var("CFA4_REQUIRE_ORACLES").is_ok_and(|v| v == "1")
}

/// The fixture path, generating it with the repository's generator when missing.
fn fixture() -> Option<PathBuf> {
    let path = cfa4_testkit::fixtures::fixtures_root().join("render-f0v").join("sync_flash_beep.mkv");
    if !path.is_file() {
        let script = cfa4_testkit::workspace_root().join("python/cfa4_devtools/render_fixtures.py");
        let _ = Command::new("python3").arg(script).args(["generate", "sync_flash_beep"]).output();
    }
    path.is_file().then_some(path)
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

/// Render the fixture through the engine: one linked video + audio clip on a 25 fps sequence.
fn render(path: &Path) -> Capture {
    let src: SharedSource = cfa4_media::open_path(path, &cfa4_codecs::openers()).expect("open the fixture");
    let info = src.info().clone();
    let video = info.video.as_ref().expect("fixture has video");
    let (w, h) = (video.width, video.height);
    let mut p = Project::new("f0v-sync");
    let seq = p.new_sequence("sync", SequenceSettings { width: w, height: h, frame_rate: RATE, sample_rate: SAMPLE_RATE, ..Default::default() }, 1, 1, None);
    let id = p.add_item(
        "sync_flash_beep",
        Label::Iris,
        ItemKind::Media(MediaClip {
            media: MediaRef::File { path: path.to_string_lossy().into_owned() },
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
    let mut sources = SourceMap::default();
    sources.0.insert(id, Arc::clone(&src));
    let dur = TimeRange::new(Tick::ZERO, RATE.tick_of(FRAMES));
    let v = p.make_track_item(id, TrackKind::Video, Tick::ZERO, dur, RATE).expect("video clip");
    let a = p.make_track_item(id, TrackKind::Audio, Tick::ZERO, dur, RATE).expect("audio clip");
    let s = p.sequence_mut(seq).expect("sequence");
    s.video_tracks[0].items.push(v);
    s.audio_tracks[0].items.push(a);

    let frames: Vec<FrameCapture> =
        (0..FRAMES).map(|f| frame_capture(&render_sequence(&p, seq, RATE.tick_of(f), RenderOptions::default(), &sources))).collect();
    let samples = (FRAMES as usize) * SAMPLE_RATE as usize / RATE_FPS as usize;
    let mixed = mix_sequence(&p, p.sequence(seq).expect("sequence"), 0, samples, &sources);
    Capture {
        frames,
        pts: (0..FRAMES).map(|f| RATE.tick_of(f).0).collect(),
        frame_duration: RATE.frame_duration().0,
        audio: mixed.channels,
        sample_rate: SAMPLE_RATE,
    }
}

fn thresholds() -> Thresholds {
    let path = cfa4_testkit::workspace_root().join("docs/audit/render-f0v/F0V-thresholds.json");
    Thresholds::from_json(&std::fs::read_to_string(path).expect("committed thresholds")).expect("valid thresholds")
}

fn spec() -> SyncSpec {
    SyncSpec { ticks_per_second: TICKS_PER_SECOND, expected_events: SECONDS as usize, max_abs_offset_ms: 0.1, max_drift_ms: 0.1 }
}

/// Limits: the render of a file decoded by our own decoders puts audio and picture on the same
/// clock, so the only legitimate error is the rounding of a frame time to a sample (one sample is
/// 0.0208 ms at 48 kHz). 0.1 ms is four samples; the measured offset over ten runs was one sample
/// and the drift zero, identical in every run (docs/audit/render-f0v/F0V-thresholds.md).
fn sync_of(cap: &Capture) -> (Status, String, String) {
    let x = Extras { sync: Some(spec()), audio_bits: None };
    let checks = compare_with(cap, cap, &thresholds(), &x);
    let c = checks.iter().find(|c| c.id == "sync.av_drift").expect("sync check present");
    (c.status, c.value.clone(), c.limit.clone())
}

#[test]
fn the_engine_render_of_the_flash_and_beep_fixture_stays_in_sync() {
    let Some(path) = fixture() else {
        eprintln!("SKIPPED: sync_flash_beep.mkv could not be generated (ffmpeg missing?)");
        assert!(!require_oracles(), "CFA4_REQUIRE_ORACLES=1 and the sync fixture is missing");
        return;
    };
    let first = render(&path);
    let (status, value, limit) = sync_of(&first);
    eprintln!("sync.av_drift: {status:?} value={value} limit={limit}");
    assert_eq!(status, Status::Pass, "A/V sync of the engine render: {value} (limit {limit})");

    // ten runs: no false failure, and the measurement does not move
    for run in 1..10 {
        let (s, v, _) = sync_of(&render(&path));
        assert_eq!((s, &v), (Status::Pass, &value), "run {run} measured {v}");
    }

    // detection power: the same render with the audio two frames late must fail
    let mut late = first.clone();
    let shift = 2 * SAMPLE_RATE as usize / RATE_FPS as usize;
    for ch in &mut late.audio {
        ch.splice(0..0, std::iter::repeat_n(0.0, shift));
        ch.truncate(first.audio[0].len());
    }
    assert_eq!(sync_of(&late).0, Status::Fail, "an 80 ms audio delay must be caught");
}
