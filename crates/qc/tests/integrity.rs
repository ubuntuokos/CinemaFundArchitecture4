//! Input and integrity checks on synthetic MP4 files (plan §4.1, §4.2, §6 layers 1–2): good
//! files pass, damaged or mismatched ones are flagged, and nothing passes on missing evidence.

mod common;

use std::sync::atomic::AtomicBool;

use cfa4_qc::expect::Rate;
use cfa4_qc::{Expectation, Options, QcProfile, QcReport, Status, Verdict};
use common::{FPS, H, Spec, W};

fn plan(frames: u64) -> Expectation {
    Expectation {
        video: Some(true),
        frames: Some(frames),
        frame_rate: Some(Rate { num: i64::from(FPS), den: 1 }),
        width: Some(W),
        height: Some(H),
        video_codec: Some("H.264".into()),
        audio_tracks: Some(1),
        audio_channels: Some(2),
        sample_rate: Some(48_000),
        audio_codec: None,
        subtitle_streams: Some(0),
    }
}

fn run(bytes: Vec<u8>, expect: &Expectation) -> QcReport {
    let p = QcProfile::builtin("delivery-web").unwrap();
    cfa4_qc::check("/exports/test.mp4", common::reader(bytes), &p, expect, Options::default(), &AtomicBool::new(false)).unwrap()
}

fn status(r: &QcReport, id: &str) -> Status {
    r.checks.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("no result for {id}")).status
}

fn check<'a>(r: &'a QcReport, id: &str) -> &'a cfa4_qc::CheckResult {
    r.checks.iter().find(|c| c.id == id).unwrap()
}

#[test]
fn good_file_passes_input_and_integrity() {
    let r = run(common::mp4(&Spec::default()), &plan(24));
    for c in r.checks.iter().filter(|c| c.id.starts_with("probe.") || c.id.starts_with("integrity.")) {
        assert_eq!(c.status, Status::Pass, "{}: {:?} {:?}", c.id, c.reason, c.events);
    }
    assert_eq!(check(&r, "integrity.frameCount").value, Some(0.0));
    assert_eq!(check(&r, "integrity.decode").detail["streams"][0]["decoded"], 24);
    // audio / picture checks of later stages and the human review keep the verdict open
    assert_eq!(r.summary.verdict, Verdict::Incomplete);
    assert!(r.summary.reasons.iter().all(|s| !s.contains("probe.") && !s.contains("integrity.")), "{:?}", r.summary.reasons);
    assert_eq!(r.output.sha256.len(), 64);
}

#[test]
fn same_input_same_report() {
    let bytes = common::mp4(&Spec::default());
    let a = run(bytes.clone(), &plan(24)).to_json().unwrap();
    let b = run(bytes, &plan(24)).to_json().unwrap();
    assert_eq!(a, b);
}

#[test]
fn without_a_render_plan_the_comparisons_do_not_run() {
    let r = run(common::mp4(&Spec::default()), &Expectation::default());
    for id in ["integrity.frameCount", "integrity.duration", "integrity.streams"] {
        assert_eq!(status(&r, id), Status::NotRun, "{id}");
        assert!(check(&r, id).reason.as_deref().unwrap().contains("no render plan"), "{id}");
    }
    assert_eq!(status(&r, "integrity.decode"), Status::Pass);
}

#[test]
fn frame_count_mismatch_fails() {
    let r = run(common::mp4(&Spec { frames: 20, ..Spec::default() }), &plan(24));
    assert_eq!(status(&r, "integrity.frameCount"), Status::Fail);
    assert_eq!(check(&r, "integrity.frameCount").value, Some(-4.0));
    assert_eq!(status(&r, "integrity.duration"), Status::Fail);
    assert_eq!(r.summary.verdict, Verdict::Fail);
}

#[test]
fn missing_audio_stream_is_flagged() {
    let r = run(common::mp4(&Spec { audio_channels: None, ..Spec::default() }), &plan(24));
    assert_eq!(status(&r, "integrity.streams"), Status::Fail);
    assert_eq!(status(&r, "probe.audio"), Status::Fail);
    assert_eq!(status(&r, "integrity.avOffset"), Status::NotRun);
    assert_eq!(r.summary.verdict, Verdict::Fail);
}

#[test]
fn wrong_channel_count_is_flagged() {
    let r = run(common::mp4(&Spec { audio_channels: Some(1), ..Spec::default() }), &plan(24));
    assert_eq!(status(&r, "probe.audio"), Status::Fail);
    assert_eq!(status(&r, "integrity.streams"), Status::Fail);
}

#[test]
fn truncated_file_never_passes() {
    let mut bytes = common::mp4(&Spec::default());
    for keep in [bytes.len() * 9 / 10, bytes.len() / 2, 100, 0] {
        bytes.truncate(keep);
        let r = run(bytes.clone(), &plan(24));
        assert_eq!(r.summary.verdict, Verdict::Fail, "{keep} bytes: {:?}", r.summary.reasons);
    }
}

#[test]
fn damaged_picture_fails_with_its_frame() {
    let mut bytes = common::mp4(&Spec::default());
    let (off, size) = common::video_sample(&bytes, 13);
    // keep the 4-byte NAL length, break the NAL header (forbidden_zero_bit set) and the payload
    for b in bytes.iter_mut().skip(off + 4).take(size - 4) {
        *b = 0xff;
    }
    let r = run(bytes, &plan(24));
    assert_eq!(status(&r, "integrity.decode"), Status::Fail, "{:?}", check(&r, "integrity.decode"));
    let ev = &check(&r, "integrity.decode").events;
    assert!(ev.iter().any(|e| e.at.frame == Some(13) && e.at.timecode.as_deref() == Some("00:00:00:13")), "{ev:?}");
    assert_eq!(r.summary.verdict, Verdict::Fail);
}

#[test]
fn timeline_gap_and_variable_rate_warn() {
    // frame 10 lasts 3 frames: a 2-frame hole before frame 11
    let r = run(common::mp4(&Spec { stretch: vec![(10, 3)], ..Spec::default() }), &Expectation::default());
    assert_eq!(status(&r, "integrity.frames"), Status::Warn, "{:?}", check(&r, "integrity.frames"));
    let ev = &check(&r, "integrity.frames").events;
    assert!(ev.iter().any(|e| e.at.frame == Some(11) && e.message.contains("gap of 2.0")), "{ev:?}");
    assert_eq!(status(&r, "probe.consistency"), Status::Warn);
}

#[test]
fn unopenable_video_track_fails() {
    // a sample entry the demuxer cannot read: the file fails to open, so nothing passes
    let r = run(common::mp4(&Spec { unknown_codec: true, ..Spec::default() }), &plan(24));
    assert_eq!(status(&r, "probe.open"), Status::Fail);
    assert_eq!(status(&r, "integrity.decode"), Status::NotRun);
    assert_eq!(r.summary.verdict, Verdict::Fail);
}

#[test]
fn non_mp4_container_is_not_run() {
    // a WAV file: CFA4 opens it, but sample-level analysis covers MP4 / MOV only
    let mut wav = Vec::new();
    let pcm: Vec<u8> = (0..4800i16).flat_map(|s| (s % 100).to_le_bytes()).collect();
    for part in [&b"RIFF"[..], &(36 + pcm.len() as u32).to_le_bytes(), b"WAVEfmt ", &16u32.to_le_bytes(), &1u16.to_le_bytes(), &1u16.to_le_bytes()] {
        wav.extend_from_slice(part);
    }
    for part in [&48_000u32.to_le_bytes()[..], &96_000u32.to_le_bytes(), &2u16.to_le_bytes(), &16u16.to_le_bytes(), b"data", &(pcm.len() as u32).to_le_bytes()]
    {
        wav.extend_from_slice(part);
    }
    wav.extend_from_slice(&pcm);
    let r = run(wav, &Expectation { video: Some(false), ..Expectation::default() });
    assert_eq!(status(&r, "probe.open"), Status::Pass);
    assert_eq!(status(&r, "integrity.decode"), Status::NotRun);
    assert!(check(&r, "integrity.decode").reason.as_deref().unwrap().contains("MP4 / MOV only"));
}

#[test]
fn cancelled_run_is_incomplete() {
    let p = QcProfile::builtin("delivery-web").unwrap();
    let r = cfa4_qc::check("/x.mp4", common::reader(common::mp4(&Spec::default())), &p, &plan(24), Options::default(), &AtomicBool::new(true)).unwrap();
    assert!(r.run.cancelled);
    assert_eq!(r.summary.verdict, Verdict::Incomplete);
    assert!(r.checks.iter().filter(|c| c.id.starts_with("integrity.")).all(|c| c.status == Status::NotRun));
}

#[test]
fn unicode_path_with_spaces() {
    let dir = std::env::temp_dir().join(format!("cfa4-qc-it-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("export árvíztűrő tükör.mp4");
    std::fs::write(&path, common::mp4(&Spec::default())).unwrap();
    let p = QcProfile::builtin("delivery-web").unwrap();
    let r = cfa4_qc::check_file(&path, &p, &plan(24), Options::default(), &AtomicBool::new(false)).unwrap();
    assert!(r.output.path.ends_with("export árvíztűrő tükör.mp4"));
    assert_eq!(status(&r, "integrity.decode"), Status::Pass);
    std::fs::remove_dir_all(&dir).unwrap();
}
