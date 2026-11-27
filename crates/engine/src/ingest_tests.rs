//! `ingest.*`: the recorder core through the command layer (synthetic backend).

use serde_json::json;

use crate::Session;

fn start(s: &mut Session) -> u64 {
    let r = s.execute("ingest.start", json!({"backend": "synthetic", "path": "/tmp/cfa4-ingest-test.mkv"})).unwrap();
    assert_eq!(r["state"], json!("recording"), "{r}");
    r["handle"].as_u64().unwrap()
}

#[test]
fn record_grow_and_finalize() {
    let mut s = Session::default();
    let before = s.project.clone();
    let h = start(&mut s);
    let r = s.execute("ingest.synthetic.push", json!({"handle": h, "frames": 50, "dropped": 2})).unwrap();
    assert_eq!(r["extent"]["frames"], json!(50), "{r}");
    assert_eq!(r["extent"]["state"], json!("growing"), "{r}");
    let r = s.execute("ingest.stop", json!({"handle": h})).unwrap();
    assert_eq!(r["state"], json!("finalized"), "{r}");
    assert_eq!((r["report"]["frames"].as_u64(), r["report"]["dropped"].as_u64()), (Some(50), Some(2)), "{r}");
    let st = s.execute("ingest.status", json!({"handle": h})).unwrap();
    assert_eq!(st["journal"].as_array().unwrap().len(), 4, "{st}");
    assert_eq!(s.project, before, "recording never touches the project");
}

#[test]
fn interruption_keeps_the_recording_recoverable_and_quarantine_needs_a_reason() {
    let mut s = Session::default();
    let h = start(&mut s);
    s.execute("ingest.synthetic.push", json!({"handle": h, "frames": 10})).unwrap();
    let r = s.execute("ingest.synthetic.fault", json!({"handle": h, "reason": "source lost"})).unwrap();
    assert_eq!(r["state"], json!("recoverable"), "{r}");
    assert_eq!(r["extent"]["frames"], json!(10), "{r}");
    assert!(s.execute("ingest.quarantine", json!({"handle": h})).is_err(), "a reason is required");
    let r = s.execute("ingest.quarantine", json!({"handle": h, "reason": "checksum differs"})).unwrap();
    assert_eq!(r["state"], json!("quarantined"), "{r}");
    let r = s.execute("ingest.release", json!({"handle": h, "reason": "checked by hand"})).unwrap();
    assert_eq!(r["state"], json!("recoverable"), "{r}");
    let ev = s.execute("ingest.events", json!({})).unwrap();
    let names: Vec<_> = ev["events"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap().to_string()).collect();
    assert!(names.contains(&"ingest.recording.interrupted".to_string()) && names.contains(&"ingest.media.quarantined".to_string()), "{names:?}");
}

#[test]
fn bad_parameters_are_refused_not_guessed() {
    let mut s = Session::default();
    assert!(s.execute("ingest.start", json!({"path": "/tmp/x.mkv"})).is_err(), "backend is required");
    assert!(s.execute("ingest.start", json!({"backend": "decklink", "path": "/tmp/x.mkv"})).is_err(), "no such backend yet");
    assert!(s.execute("ingest.start", json!({"backend": "synthetic"})).is_err(), "path is required");
    assert!(s.execute("ingest.start", json!({"backend": "synthetic", "path": "/tmp/x", "container": "avi"})).is_err());
    assert!(s.execute("ingest.start", json!({"backend": "synthetic", "path": "/tmp/x", "width": 0})).is_err());
    assert!(s.execute("ingest.start", json!({"backend": "synthetic", "path": "/tmp/x", "fpsNum": 25})).is_err());
    assert!(s.execute("ingest.start", json!({"backend": "synthetic", "path": "/tmp/x", "fpsNum": 25, "fpsDen": 0})).is_err());
    assert!(s.execute("ingest.stop", json!({})).is_err());
    assert!(s.execute("ingest.stop", json!({"handle": 99})).is_err());
    assert!(s.execute("ingest.status", json!({"handle": 99})).is_err());
    let h = start(&mut s);
    assert!(s.execute("ingest.start", json!({"backend": "synthetic", "path": "/tmp/y.mkv"})).is_err(), "one active recording");
    assert!(s.execute("ingest.synthetic.push", json!({"handle": h})).is_err());
    s.execute("ingest.stop", json!({"handle": h})).unwrap();
    assert!(s.execute("ingest.synthetic.push", json!({"handle": h, "frames": 1})).is_err(), "a finalized recording does not grow");
}
