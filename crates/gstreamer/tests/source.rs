//! The GStreamer media source and the engine fallback (ADR 0002 step 2), against the machine's
//! real GStreamer. Fixtures are encoded by GStreamer itself (Theora / Vorbis in Ogg, formats the
//! engine's own openers cannot fully read) into Cargo's temporary directory; a machine without
//! GStreamer or the needed plugins skips with a message.

#![cfg(feature = "gstreamer")]

use std::path::{Path, PathBuf};

use cfa4_media::{FrameRequest, MediaKind, SharedSource};
use cfa4_time::{FrameRate, TICKS_PER_SECOND, Tick};

fn fixture(name: &str, args: &str) -> Option<PathBuf> {
    if let Err(e) = cfa4_gstreamer::probe() {
        eprintln!("skipped: {e}");
        return None;
    }
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cfa4-gstreamer-source");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(name);
    if path.exists() {
        return Some(path);
    }
    let tmp = dir.join(format!("{name}.part"));
    let args = args.replace("OUT", &tmp.display().to_string());
    match std::process::Command::new("gst-launch-1.0").arg("-q").args(args.split(' ')).output() {
        Ok(o) if o.status.success() && tmp.exists() => {
            std::fs::rename(&tmp, &path).ok()?;
            Some(path)
        }
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

/// 2 s of red 320×240 Theora at 25 fps, no audio. The engine cannot open it at all.
fn theora() -> Option<PathBuf> {
    fixture("red.ogv", "videotestsrc pattern=red num-buffers=50 ! video/x-raw,width=320,height=240,framerate=25/1 ! theoraenc ! oggmux ! filesink location=OUT")
}

/// 2 s of Theora video plus a 440 Hz Vorbis tone. The engine opens it as audio only.
fn theora_vorbis() -> Option<PathBuf> {
    fixture(
        "av.ogv",
        "videotestsrc num-buffers=50 ! video/x-raw,width=320,height=240,framerate=25/1 ! theoraenc ! queue ! oggmux name=m ! filesink location=OUT audiotestsrc freq=440 volume=0.5 num-buffers=100 samplesperbuffer=960 ! audio/x-raw,rate=48000,channels=2 ! audioconvert ! vorbisenc ! queue ! m.",
    )
}

fn ticks(seconds_num: i64, den: i64) -> Tick {
    Tick::from_units(seconds_num, den)
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

#[test]
fn theora_video_opens_with_correct_info() {
    let Some(path) = theora() else { return };
    let src = cfa4_gstreamer::open(&path).unwrap();
    let info = src.info();
    assert_eq!(info.kind, MediaKind::Movie);
    let v = info.video.as_ref().unwrap();
    assert_eq!((v.width, v.height), (320, 240));
    assert_eq!(v.frame_rate, FrameRate::FPS_25);
    assert!(v.codec.contains("GStreamer"), "{}", v.codec);
    assert!(info.audio_streams.is_empty());
    let two_s = 2 * TICKS_PER_SECOND;
    assert!((info.duration.0 - two_s).abs() <= ticks(1, 25).0, "{:?}", info.duration);
}

#[test]
fn video_frames_are_rgba_and_land_on_the_requested_time() {
    let Some(path) = theora() else { return };
    let src = cfa4_gstreamer::open(&path).unwrap();
    let frame_dur = ticks(1, 25);

    // Random access, forwards and backwards.
    for &(n, d) in &[(0i64, 1i64), (1, 1), (1, 5), (37, 25), (3, 25), (0, 1)] {
        let t = ticks(n, d);
        let f = src.video_frame(FrameRequest::full(t)).unwrap();
        assert_eq!((f.width, f.height), (320, 240));
        assert!(f.pts <= t && t < Tick(f.pts.0 + frame_dur.0), "requested {t:?}, got frame at {:?}", f.pts);
        let cfa4_frame::PixelData::Rgba8(px) = &f.data else { panic!("expected RGBA 8-bit") };
        assert_eq!(px.len(), 320 * 240 * 4);
        // Red after lossy Theora 4:2:0 coding.
        let (r, g, b) = (px[0], px[1], px[2]);
        assert!(r > 200 && g < 70 && b < 70, "pixel {r},{g},{b}");
    }

    // Sequential playback: every frame in order, without gaps or repeats.
    let mut last = None;
    for i in 0..25 {
        let f = src.video_frame(FrameRequest::full(ticks(i, 25))).unwrap();
        if let Some(prev) = last {
            assert!(f.pts > prev, "frame {i} did not advance");
        }
        last = Some(f.pts);
    }

    // Past the end: the last picture, not an error.
    let end = src.video_frame(FrameRequest::full(ticks(10, 1))).unwrap();
    assert!(end.pts >= ticks(48, 25), "{:?}", end.pts);
    // And back to the start after reaching the end.
    assert_eq!(src.video_frame(FrameRequest::full(Tick::ZERO)).unwrap().pts, Tick::ZERO);
}

#[test]
fn vorbis_audio_decodes_sequentially_and_after_seeks() {
    let Some(path) = theora_vorbis() else { return };
    let src = cfa4_gstreamer::open(&path).unwrap();
    let a = src.info().audio().unwrap().clone();
    assert_eq!((a.sample_rate, a.channels), (48_000, 2));

    let first = src.audio(0, 4800, 48_000).unwrap();
    assert_eq!((first.channel_count(), first.frames()), (2, 4800));
    assert!(rms(&first.channels[0][1000..]) > 0.1, "tone expected, rms {}", rms(&first.channels[0]));

    // The next block continues the tone without a gap.
    let next = src.audio(4800, 4800, 48_000).unwrap();
    assert!(rms(&next.channels[0]) > 0.1);

    // Seek into the middle, and resample on request.
    let mid = src.audio(48_000, 2205, 44_100).unwrap();
    assert_eq!((mid.sample_rate, mid.frames()), (44_100, 2205));
    assert!(rms(&mid.channels[1]) > 0.1);

    // Past the end: silence, not an error.
    let after = src.audio(48_000 * 30, 4800, 48_000).unwrap();
    assert!(rms(&after.channels[0]) < 1e-6);

    // Hostile requests are errors.
    assert!(src.audio(0, 10, 0).is_err());
    assert!(src.audio(0, usize::MAX, 48_000).is_err());
    assert!(src.audio_stream(5, 0, 10, 48_000).is_err());
}

#[test]
fn audio_only_files_open_without_video() {
    let Some(path) = fixture(
        "tone.oga",
        "audiotestsrc num-buffers=50 samplesperbuffer=960 ! audio/x-raw,rate=48000,channels=1 ! audioconvert ! vorbisenc ! oggmux ! filesink location=OUT",
    ) else {
        return;
    };
    let src = cfa4_gstreamer::open(&path).unwrap();
    assert_eq!(src.info().kind, MediaKind::AudioOnly);
    assert!(src.video_frame(FrameRequest::full(Tick::ZERO)).is_err());
    assert_eq!(src.audio(0, 960, 48_000).unwrap().channel_count(), 1);
}

#[test]
fn garbage_and_missing_files_are_errors() {
    if cfa4_gstreamer::probe().is_err() {
        return;
    }
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cfa4-gstreamer-source");
    std::fs::create_dir_all(&dir).unwrap();
    let junk = dir.join("junk.ogv");
    std::fs::write(&junk, (0u32..8192).map(|i| (i.wrapping_mul(2_654_435_761) >> 11) as u8).collect::<Vec<_>>()).unwrap();
    assert!(cfa4_gstreamer::open(&junk).is_err());
    assert!(cfa4_gstreamer::open(Path::new("/nonexistent/x.ogv")).is_err());
}

fn engine_open(path: &Path) -> SharedSource {
    cfa4_media::open_path(path, &cfa4_codecs::openers()).unwrap()
}

#[test]
fn registered_fallback_fills_the_engine_gaps_and_leaves_the_rest_alone() {
    let (Some(video_only), Some(av)) = (theora(), theora_vorbis()) else { return };
    let mjpeg = fixture("mjpeg.avi", "videotestsrc num-buffers=10 ! video/x-raw,width=64,height=64,framerate=25/1 ! jpegenc ! avimux ! filesink location=OUT");
    cfa4_gstreamer::register().unwrap();

    assert!(cfa4_media::is_importable(Path::new("clip.ogv")));
    assert!(cfa4_media::is_importable(Path::new("song.WMA")));

    // The engine cannot open Theora at all: GStreamer does.
    let s = engine_open(&video_only);
    assert!(s.info().video.as_ref().unwrap().codec.contains("GStreamer"));

    // The engine opens Theora + Vorbis as audio only: GStreamer adds the video.
    let s = engine_open(&av);
    assert_eq!(s.info().kind, MediaKind::Movie);
    assert!(s.info().has_audio());
    assert!(s.video_frame(FrameRequest::full(Tick::ZERO)).is_ok());

    // What the engine opens itself stays with the engine.
    if let Some(mjpeg) = mjpeg {
        let s = engine_open(&mjpeg);
        assert!(!s.info().video.as_ref().unwrap().codec.contains("GStreamer"), "{}", s.info().video.as_ref().unwrap().codec);
    }
}

/// The engine's own entry points (media pool import, Media Browser properties and thumbnails)
/// all reach the fallback: thumbnails are frames of the source `probe_file` / `open_file` return.
#[test]
fn engine_import_browser_and_thumbnails_use_the_fallback() {
    let Some(path) = theora() else { return };
    cfa4_gstreamer::register().unwrap();
    let p = path.to_string_lossy().to_string();
    let pool = cfa4_engine::media_pool::MediaPool::default();
    let services = cfa4_engine::FsServices;

    // Import (MediaPool::open_file) and Media Browser probing (MediaPool::probe_file).
    let imported = pool.open_file(&p, &services).unwrap();
    let probed = pool.probe_file(&p, &services).unwrap();
    for src in [&imported, &probed] {
        assert_eq!(src.info().kind, MediaKind::Movie);
        // A thumbnail is a decoded frame at a reduced scale.
        let thumb = src.video_frame(FrameRequest { time: ticks(1, 2), scale: 0.25 }).unwrap();
        assert!(thumb.width > 0 && thumb.height > 0);
    }
    assert_eq!(cfa4_engine::media_browser::kind_of("clip.ogv"), Some("video"));
    assert!(cfa4_engine::media_browser::extensions().contains(&"ogv"));
}
