use cfa4_ingest::*;
use cfa4_time::{FrameRate, Tick};

fn job() -> RecordJob {
    RecordJob {
        target: StagedTarget { path: "/tmp/rec.mkv".into() },
        container: Container::Mkv,
        video: vec![VideoTrackSpec { codec: "h264".into(), width: 1920, height: 1080, frame_rate: FrameRate::FPS_25 }],
        audio: vec![AudioTrackSpec { codec: "pcm".into(), channels: 2, sample_rate: 48_000 }],
        split: None,
        start_timecode: Some(Tick::from_units(36_000, 1)),
    }
}

#[test]
fn normal_recording_grows_then_finalizes() {
    let mut r = testsrc::TestRecorder::new();
    let h = r.start(job()).unwrap();
    assert_eq!(r.state(h), Some(State::Recording));
    r.push_frames(h, 25, 0).unwrap();
    let v = r.extent(h).unwrap();
    assert_eq!((v.frames, v.state), (25, ExtentState::Growing));
    assert_eq!(v.available, FrameRate::FPS_25.tick_of(25));
    r.marker(h, Tick::ZERO, "chapter").unwrap();
    let rep = r.stop(h).unwrap();
    assert_eq!((rep.frames, rep.dropped), (25, 0));
    assert_eq!(rep.preserved, vec![MetadataKind::Timecode, MetadataKind::Markers]);
    assert_eq!(r.state(h), Some(State::Finalized));
    assert_eq!(r.extent(h).unwrap().state, ExtentState::Finalized);
}

#[test]
fn seek_into_unwritten_media_is_refused_not_faked() {
    let mut r = testsrc::TestRecorder::new();
    let h = r.start(job()).unwrap();
    r.push_frames(h, 50, 0).unwrap();
    let v = r.extent(h).unwrap();
    let edge = v.available;
    assert_eq!(v.check_seek(Tick(edge.0 - 1)), SeekCheck::Available);
    assert_eq!(v.check_seek(edge), SeekCheck::NotYetWritten { available: edge });
    assert_eq!(v.check_seek(Tick(-1)), SeekCheck::BeforeStart);
}

#[test]
fn dropped_frames_do_not_extend_the_extent() {
    let mut r = testsrc::TestRecorder::new();
    let h = r.start(job()).unwrap();
    r.push_frames(h, 10, 3).unwrap();
    assert_eq!(r.extent(h).unwrap().frames, 10);
    assert_eq!(r.stop(h).unwrap().dropped, 3);
}

#[test]
fn interruption_keeps_what_was_recorded() {
    let mut r = testsrc::TestRecorder::new();
    let h = r.start(job()).unwrap();
    r.push_frames(h, 40, 0).unwrap();
    r.inject_fault(h, "source lost").unwrap();
    assert_eq!(r.state(h), Some(State::Recoverable));
    let v = r.extent(h).unwrap();
    assert_eq!((v.frames, v.state), (40, ExtentState::Interrupted));
    assert!(r.events().iter().any(|e| e.name() == "ingest.recording.interrupted"));
    // a new recording may start; the interrupted one stays readable
    let h2 = r.start(job()).unwrap();
    assert_ne!(h, h2);
    assert_eq!(r.extent(h).unwrap().frames, 40);
}

#[test]
fn state_machine_rejects_invalid_and_reasonless_transitions() {
    let mut s = Session::new(1);
    assert!(matches!(s.apply(Trigger::Stop, Tick::ZERO, ""), Err(IngestError::InvalidTransition { .. })));
    assert_eq!(s.state(), State::Idle);
    s.apply(Trigger::Begin, Tick::ZERO, "").unwrap();
    s.apply(Trigger::Started, Tick::ZERO, "").unwrap();
    assert_eq!(s.apply(Trigger::Fault, Tick::ZERO, "  "), Err(IngestError::ReasonRequired("fault, quarantine and release must say why")));
    s.apply(Trigger::Fault, Tick::ZERO, "disk full").unwrap();
    s.apply(Trigger::Preserve, Tick::ZERO, "").unwrap();
    assert!(s.apply(Trigger::Release, Tick::ZERO, "x").is_err(), "not quarantined");
    s.apply(Trigger::Quarantine, Tick(5), "bad checksum").unwrap();
    assert!(s.apply(Trigger::Recover, Tick(6), "").is_err(), "quarantined media is not recovered silently");
    s.apply(Trigger::Release, Tick(7), "checked by hand").unwrap();
    assert_eq!(s.state(), State::Recoverable);
    assert_eq!(s.journal().len(), 6);
    assert!(s.journal().windows(2).all(|w| w[0].seq + 1 == w[1].seq));
}

#[test]
fn extent_is_monotonic_and_closes() {
    let mut e = Extent::default();
    e.advance(5, Tick(100)).unwrap();
    assert!(matches!(e.advance(1, Tick(99)), Err(IngestError::ExtentNotMonotonic(_))));
    e.finalize().unwrap();
    assert!(matches!(e.advance(1, Tick(200)), Err(IngestError::ExtentClosed(_))));
    assert!(e.finalize().is_err());
}

#[test]
fn hostile_jobs_are_refused() {
    let mut r = testsrc::TestRecorder::new();
    let mut j = job();
    j.video[0].width = 0;
    assert!(matches!(r.start(j), Err(IngestError::InvalidJob(_))));
    let mut j = job();
    j.video.clear();
    j.audio.clear();
    assert!(r.start(j).is_err());
    let mut j = job();
    j.video[0].frame_rate = FrameRate { num: 25, den: 0 };
    assert!(r.start(j).is_err());
    let mut j = job();
    j.split = Some(SplitRule::BySize(0));
    assert!(r.start(j).is_err());
    let mut j = job();
    j.start_timecode = Some(Tick(-1));
    assert!(r.start(j).is_err());
    let h = r.start(job()).unwrap();
    assert_eq!(r.start(job()), Err(IngestError::AlreadyActive));
    assert!(matches!(r.stop(RecordHandle(999)), Err(IngestError::UnknownHandle(999))));
    r.push_frames(h, 5, 0).unwrap();
    r.push_frames(h, u64::MAX, 0).unwrap_err();
    assert_eq!(r.extent(h).unwrap().frames, 5, "a refused push changes nothing");
}

#[test]
fn events_serialize_with_provisional_names() {
    let mut r = testsrc::TestRecorder::new();
    let h = r.start(job()).unwrap();
    r.stop(h).unwrap();
    let names: Vec<_> = r.events().iter().map(|e| e.name()).collect();
    assert!(names.contains(&"ingest.recording.started") && names.contains(&"ingest.media.finalized"));
    let j = serde_json::to_string(&r.events()[0]).unwrap();
    assert!(j.contains("\"kind\""));
}
