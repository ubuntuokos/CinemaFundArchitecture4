//! Tests against the machine's real GStreamer installation (feature `gstreamer`).
//!
//! A machine without GStreamer, or without the plugins a test needs, skips that test with a
//! message instead of failing: the crate's contract there is `Error::Unavailable`, which the unit
//! tests cover. Fixtures are generated into Cargo's per-test temporary directory, never committed.

#![cfg(feature = "gstreamer")]

use std::path::{Path, PathBuf};
use std::time::Duration;

use cfa4_gstreamer::{Area, DEFAULT_DISCOVER_TIMEOUT, Error, StreamKind, discover, probe};
use cfa4_time::{FrameRate, Tick};

fn status_or_skip() -> Option<cfa4_gstreamer::Status> {
    match probe() {
        Ok(status) => Some(status),
        Err(Error::Unavailable(why)) => {
            eprintln!("skipped: GStreamer unavailable ({why})");
            None
        }
        Err(e) => panic!("probe failed unexpectedly: {e}"),
    }
}

/// Writes a 2-second 320×240 25 fps raw-video Matroska file and a 1-second WAV file with GStreamer
/// itself, or returns `None` when the needed elements are missing.
fn make_fixture(name: &str, pipeline: &str) -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cfa4-gstreamer");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(name);
    let out = std::process::Command::new("gst-launch-1.0").args(["-q"]).args(pipeline.split(' ')).arg(format!("location={}", path.display())).output();
    match out {
        Ok(o) if o.status.success() && path.exists() => Some(path),
        Ok(o) => {
            eprintln!("skipped: fixture pipeline failed: {}", String::from_utf8_lossy(&o.stderr));
            None
        }
        Err(e) => {
            eprintln!("skipped: gst-launch-1.0 not runnable ({e})");
            None
        }
    }
}

#[test]
fn probe_reports_version_and_every_catalogue_element() {
    let Some(status) = status_or_skip() else { return };
    assert!(status.version.starts_with("1."), "{}", status.version);
    assert_eq!(status.elements.len(), cfa4_gstreamer::CATALOGUE.len());
    for e in &status.elements {
        // An available element always knows its plugin and licence; a missing one knows neither.
        assert_eq!(e.available, e.plugin.is_some(), "{}", e.spec.factory);
        assert_eq!(e.available, e.license.is_some(), "{}", e.spec.factory);
    }
    // gst-plugins-base ships uridecodebin3; a GStreamer that initialises without it is broken.
    assert!(status.has(Area::Pipeline));
    eprintln!("GStreamer {}:", status.version);
    for e in &status.elements {
        eprintln!("  {:<14} {:<16} {}", e.spec.factory, if e.available { "available" } else { "missing" }, e.license.as_deref().unwrap_or(""));
    }
}

#[test]
fn discover_reads_a_matroska_video_file() {
    if status_or_skip().is_none() {
        return;
    }
    let Some(path) = make_fixture("bars.mkv", "videotestsrc num-buffers=50 ! video/x-raw,width=320,height=240,framerate=25/1 ! matroskamux ! filesink") else {
        return;
    };
    let info = discover(&path, DEFAULT_DISCOVER_TIMEOUT).unwrap();
    assert!(info.uri.starts_with("file://"));
    assert!(info.container.as_deref().unwrap_or_default().to_lowercase().contains("matroska"), "{:?}", info.container);
    let video: Vec<_> = info.video().collect();
    assert_eq!(video.len(), 1);
    let StreamKind::Video { width, height, frame_rate, is_image, .. } = &video[0].kind else { unreachable!() };
    assert_eq!((*width, *height), (320, 240));
    assert_eq!(*frame_rate, Some(FrameRate::FPS_25));
    assert!(!is_image);
    // 50 frames at 25 fps: 2 s, to within one frame.
    let duration = info.duration.unwrap();
    let two_s = Tick(2 * cfa4_time::TICKS_PER_SECOND);
    assert!((duration.0 - two_s.0).abs() <= Tick::from_units(1, 25).0, "{duration:?}");
}

#[test]
fn discover_reads_a_wav_audio_file() {
    if status_or_skip().is_none() {
        return;
    }
    let Some(path) = make_fixture("tone.wav", "audiotestsrc num-buffers=10 samplesperbuffer=4800 ! audio/x-raw,rate=48000,channels=2 ! wavenc ! filesink")
    else {
        return;
    };
    let info = discover(&path, DEFAULT_DISCOVER_TIMEOUT).unwrap();
    let audio: Vec<_> = info.audio().collect();
    assert_eq!(audio.len(), 1);
    let StreamKind::Audio { channels, sample_rate, .. } = &audio[0].kind else { unreachable!() };
    assert_eq!((*channels, *sample_rate), (2, 48_000));
    assert_eq!(info.video().count(), 0);
}

#[test]
fn discover_reports_garbage_as_an_error_not_a_crash() {
    if status_or_skip().is_none() {
        return;
    }
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cfa4-gstreamer");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("garbage.mkv");
    // Matroska magic followed by noise: a hostile, truncated file.
    let mut bytes = vec![0x1a, 0x45, 0xdf, 0xa3];
    bytes.extend((0u32..4096).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8));
    std::fs::write(&path, bytes).unwrap();
    match discover(&path, Duration::from_secs(5)) {
        Err(Error::Discover { .. }) => {}
        // GStreamer may also accept the header and find no streams; either way nothing crashed.
        Ok(info) => assert!(info.streams.is_empty(), "{info:?}"),
        Err(e) => panic!("unexpected error kind: {e}"),
    }
}

#[test]
fn discover_rejects_missing_paths_before_gstreamer() {
    let err = discover(Path::new("/nonexistent/cfa4.mkv"), DEFAULT_DISCOVER_TIMEOUT).unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)), "{err}");
}
