//! Audio and picture checks on synthetic MP4 files (plan §4.3, §4.4, §6 layer 3): known
//! loudness, peaks, clipping, silence, black pictures and out-of-range luma are measured and
//! judged by the profile's bounds, and nothing passes on missing evidence.

mod common;

use std::sync::atomic::AtomicBool;

use cfa4_qc::expect::Rate;
use cfa4_qc::{Expectation, Options, QcProfile, QcReport, Status};
use common::{FPS, Spec};

fn plan(frames: u64) -> Expectation {
    Expectation {
        video: Some(true),
        frames: Some(frames),
        frame_rate: Some(Rate { num: i64::from(FPS), den: 1 }),
        audio_tracks: Some(1),
        audio_channels: Some(2),
        ..Expectation::default()
    }
}

fn run(spec: &Spec, expect: &Expectation) -> cfa4_qc::Checked {
    let p = QcProfile::builtin("delivery-web").unwrap();
    cfa4_qc::check_with_outputs("/exports/a.mp4", common::reader(common::mp4(spec)), &p, expect, Options::default(), &AtomicBool::new(false)).unwrap()
}

fn get<'a>(r: &'a QcReport, id: &str) -> &'a cfa4_qc::CheckResult {
    r.checks.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("no result for {id}"))
}

#[test]
fn good_file_measures_audio_and_picture() {
    let c = run(&Spec { frames: 48, ..Spec::default() }, &plan(48));
    let r = &c.report;
    for id in [
        "audio.loudness",
        "audio.truePeak",
        "audio.clipping",
        "audio.silence",
        "audio.layout",
        "video.black",
        "video.clipBlack",
        "video.clipWhite",
        "video.contactSheet",
    ] {
        assert_eq!(get(r, id).status, Status::Pass, "{id}: {:?}", get(r, id));
    }
    let loud = get(r, "audio.loudness").value.unwrap();
    assert!((-15.0..=-13.0).contains(&loud), "{loud} LUFS");
    let tp = get(r, "audio.truePeak").value.unwrap();
    assert!(tp < -1.0, "{tp} dBTP");
    assert_eq!(get(r, "audio.clipping").value, Some(0.0));
    // 2 s is too short for a loudness range: NOT_RUN (optional in this profile), never PASS
    assert_eq!(get(r, "audio.loudnessRange").status, Status::NotRun);
    // the contact sheet is returned, and the report names its frames and hash
    let png = c.contact_sheet.as_deref().unwrap();
    assert_eq!(&png[1..4], b"PNG");
    let sheet = get(r, "video.contactSheet");
    assert_eq!(sheet.detail["frames"].as_array().unwrap().len(), 12);
    assert_eq!(sheet.detail["frames"][11]["frame"], 47);
    assert_eq!(sheet.detail["sha256"], cfa4_qc::hash::sha256_hex(png));
    // the report still round-trips through its own verification
    QcReport::from_json(&r.to_json().unwrap()).unwrap();
}

#[test]
fn loud_and_clipped_audio_fails() {
    let c = run(&Spec { amplitude: 1.3, ..Spec::default() }, &plan(24));
    let r = &c.report;
    assert_eq!(get(r, "audio.truePeak").status, Status::Fail, "{:?}", get(r, "audio.truePeak"));
    assert!(get(r, "audio.truePeak").value.unwrap() > 0.0);
    assert_eq!(get(r, "audio.clipping").status, Status::Fail);
    assert!(get(r, "audio.clipping").value.unwrap() > 10.0);
    assert!(!get(r, "audio.clipping").events.is_empty(), "clipping runs are located");
    assert_eq!(get(r, "audio.loudness").status, Status::Fail);
}

#[test]
fn quiet_audio_warns_or_fails_by_bounds() {
    // about −15.5 LUFS: outside pass (−15…−13), inside accept (−16…−12)
    let r = run(&Spec { amplitude: 0.2 * 10f32.powf(-1.5 / 20.0), ..Spec::default() }, &plan(24)).report;
    assert_eq!(get(&r, "audio.loudness").status, Status::Warn, "{:?}", get(&r, "audio.loudness").value);
    let r = run(&Spec { amplitude: 0.05, ..Spec::default() }, &plan(24)).report;
    assert_eq!(get(&r, "audio.loudness").status, Status::Fail);
}

#[test]
fn silence_is_located_and_silent_track_fails() {
    let r = run(&Spec { frames: 24 * 5, silence: Some((1.0, 3.5)), ..Spec::default() }, &plan(120)).report;
    let s = get(&r, "audio.silence");
    assert_eq!(s.status, Status::Warn, "{s:?}");
    let e = &s.events[0];
    let ms = |p: &cfa4_qc::report::Position| p.tick.unwrap() as f64 / 254_016_000.0;
    assert!((ms(&e.at) - 1000.0).abs() < 30.0 && (ms(e.end.as_ref().unwrap()) - 3500.0).abs() < 30.0, "{e:?}");
    assert_eq!(e.at.timecode.as_deref(), Some("00:00:01:00"));

    let r = run(&Spec { amplitude: 0.0, ..Spec::default() }, &plan(24)).report;
    assert_eq!(get(&r, "audio.silence").status, Status::Fail);
    assert_eq!(get(&r, "audio.loudness").status, Status::Fail, "a silent track has no programme loudness");
    assert_eq!(get(&r, "audio.loudness").value, Some(-70.0));
    QcReport::from_json(&r.to_json().unwrap()).unwrap();
}

#[test]
fn silent_channel_warns_in_layout() {
    let r = run(&Spec { mute_last_channel: true, ..Spec::default() }, &plan(24)).report;
    let l = get(&r, "audio.layout");
    assert_eq!(l.status, Status::Warn, "{l:?}");
    assert!(l.events[0].message.contains("channel 1 is silent"));
}

#[test]
fn black_pictures_inside_the_programme_warn() {
    let r = run(&Spec { black: vec![0, 1, 10, 11, 12], ..Spec::default() }, &plan(24)).report;
    let b = get(&r, "video.black");
    assert_eq!(b.status, Status::Warn, "{b:?}");
    assert_eq!(b.events.len(), 1, "the head is a fade, not a finding: {:?}", b.events);
    assert_eq!(b.events[0].at.frame, Some(10));
    assert!(b.events[0].message.contains("3 black"));
    assert_eq!(b.detail["headBlackFrames"], 2);

    let all: Vec<u32> = (0..24).collect();
    let r = run(&Spec { black: all, ..Spec::default() }, &plan(24)).report;
    assert_eq!(get(&r, "video.black").status, Status::Fail);
}

#[test]
fn luma_above_legal_white_is_measured() {
    // a quarter of the picture at 255 in 2 of 24 frames: about 2 % of all luma samples
    let r = run(&Spec { white: vec![5, 6], ..Spec::default() }, &plan(24)).report;
    let w = get(&r, "video.clipWhite");
    let v = w.value.unwrap();
    assert!((1.5..=2.6).contains(&v), "{v} %");
    assert_eq!(w.status, Status::Warn);
    assert_eq!(w.events[0].at.frame, Some(5));
    assert_eq!(w.detail["range"], "limited");
}

#[test]
fn video_only_plan_skips_audio_checks_as_not_applicable() {
    let e = Expectation { audio_tracks: Some(0), audio_channels: None, ..plan(24) };
    let r = run(&Spec { audio_channels: None, ..Spec::default() }, &e).report;
    for id in ["audio.loudness", "audio.truePeak", "audio.clipping", "audio.silence", "audio.layout"] {
        assert_eq!(get(&r, id).status, Status::Pass, "{id}: {:?}", get(&r, id).reason);
        assert!(get(&r, id).reason.as_deref().unwrap().starts_with("not applicable"), "{id}");
    }
    // without that plan, missing audio is not evidence of anything: NOT_RUN
    let r = run(&Spec { audio_channels: None, ..Spec::default() }, &Expectation::default()).report;
    assert_eq!(get(&r, "audio.loudness").status, Status::NotRun);
    QcReport::from_json(&r.to_json().unwrap()).unwrap();
}

#[test]
fn cancelled_run_reports_no_measurements() {
    let p = QcProfile::builtin("delivery-web").unwrap();
    let c = cfa4_qc::check_with_outputs("/x.mp4", common::reader(common::mp4(&Spec::default())), &p, &plan(24), Options::default(), &AtomicBool::new(true))
        .unwrap();
    assert!(c.contact_sheet.is_none());
    assert!(c.report.checks.iter().filter(|r| r.id.starts_with("audio.") || r.id.starts_with("video.")).all(|r| r.status == Status::NotRun));
}

/// Review of #306: the container's `colr` range is the picture's range (as playback applies it),
/// so luma 250 in a full-range file is legal, and beyond legal white in a limited-range one.
#[test]
fn container_colour_range_sets_the_picture_limits() {
    let spec = Spec { frames: 24, bright: (0..24).collect(), ..Spec::default() };
    let limited = run(&Spec { colr_full_range: Some(false), ..spec.clone() }, &plan(24));
    let full = run(&Spec { colr_full_range: Some(true), ..spec }, &plan(24));
    let white = |c: &cfa4_qc::Checked| get(&c.report, "video.clipWhite").value.unwrap();
    assert!(white(&limited) > 20.0, "{}", white(&limited));
    assert_eq!(white(&full), 0.0, "full range: 250 is a legal code");
}

/// Review of #306: pictures the edit list does not show are decoded but not judged.
#[test]
fn pictures_hidden_by_the_edit_list_are_not_measured() {
    let spec = Spec { frames: 24, white: vec![0, 1], ..Spec::default() };
    let shown = run(&spec, &plan(24));
    assert!(get(&shown.report, "video.clipWhite").value.unwrap() > 0.0);
    let hidden = run(&Spec { video_media_start: Some(2), ..spec.clone() }, &plan(22));
    let r = get(&hidden.report, "video.clipWhite");
    assert_eq!(r.detail["pictures"], 22, "{r:?}");
    // what is left is coding ripple (P pictures predicted from the hidden white ones), far below
    // the white pictures' own share
    let all = get(&shown.report, "video.clipWhite").value.unwrap();
    assert!(r.value.unwrap() < all / 10.0, "{r:?} vs {all}");
    assert!(r.events.iter().all(|e| e.at.frame.is_some_and(|f| f >= 2)), "{r:?}");
}
