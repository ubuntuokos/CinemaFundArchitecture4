//! Tests of [`crate::beat_markers`]: Markers ▸ Add Beat Markers on generated 120 BPM music.

use std::sync::Arc;

use cfa4_audio_dsp::remix as dsp;
use cfa4_project::ClipId;
use serde_json::json;

use super::*;

const SR: u32 = 48_000;

/// A session with 16 bars of generated 120 BPM music on A1 at `at` s, selected; returns the
/// true beat times (s, relative to the music).
fn session(at: f64) -> (Session, ClipId, Vec<f64>) {
    let (m, truth) = dsp::test_music(SR, 120.0, 16, 3);
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "beats", "audio": 2, "video": 1})).unwrap();
    let inter: Vec<f32> = (0..m[0].len()).flat_map(|i| [m[0][i], m[1][i]]).collect();
    let bytes: Arc<[u8]> = crate::previews::write_wav_f32(&inter, SR).into();
    let item = crate::commands::import_bytes(&mut s, "/music.wav", bytes, None).unwrap();
    let r = s.execute("timeline.place", json!({"item": item.0, "audioTrack": "A1", "seconds": at})).unwrap();
    let c = ClipId(r["clips"][0].as_u64().unwrap());
    s.execute("timeline.select", json!({"clips": [c.0]})).unwrap();
    (s, c, truth.iter().map(|&b| b as f64 / SR as f64).collect())
}

fn marker_times(s: &Session) -> Vec<f64> {
    s.active_sequence().unwrap().markers.iter().filter(|m| m.comment.starts_with(crate::beat_markers::BEAT_COMMENT)).map(|m| m.start.seconds()).collect()
}

#[test]
fn beat_markers_land_on_the_beats_and_replace_each_other() {
    let (mut s, _, truth) = session(2.0);
    let spec = crate::commands::find("markers.addBeats").unwrap();
    assert_eq!(spec.menu, &["Markers"], "reachable from the Markers menu");
    assert_eq!(crate::commands::find("markers.clearBeats").unwrap().menu, &["Markers"]);
    s.execute("markers.add", json!({"seconds": 1.0, "name": "mine"})).unwrap();
    let r = s.execute("markers.addBeats", json!({})).unwrap();
    assert!((r["bpm"].as_f64().unwrap() - 120.0).abs() < 1.0, "{r}");
    let got = marker_times(&s);
    assert!(got.len() >= 50, "{} beat markers", got.len());
    // every marker is within 25 ms of a true beat (the clip starts at 2 s)
    for t in &got {
        let d = truth.iter().map(|b| (b + 2.0 - t).abs()).fold(f64::MAX, f64::min);
        assert!(d < 0.025, "marker at {t:.3} s is {d:.3} s off the nearest beat");
    }
    // again, every 4th beat: replaces (one undo step), keeps the user's marker
    let n0 = s.history.undo.len();
    s.execute("markers.addBeats", json!({"every": 4})).unwrap();
    assert_eq!(s.history.undo.len(), n0 + 1);
    let bars = marker_times(&s);
    assert!(bars.len() * 4 >= got.len() - 3 && bars.len() * 4 <= got.len() + 3, "{} bars for {} beats", bars.len(), got.len());
    assert!(s.active_sequence().unwrap().markers.iter().any(|m| m.name == "mine"));
    assert!(s.active_sequence().unwrap().markers.iter().any(|m| m.name == "Bar 1"));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(marker_times(&s), got);
    // clear removes only beat markers
    let r = s.execute("markers.clearBeats", json!({})).unwrap();
    assert_eq!(r["removed"].as_u64().unwrap() as usize, got.len());
    assert!(marker_times(&s).is_empty());
    assert_eq!(s.active_sequence().unwrap().markers.len(), 1);
    assert!((crate::commands::find("markers.clearBeats").unwrap().enabled)(&s).is_err());
}

#[test]
fn speed_changed_clip_maps_beats_through_its_speed() {
    let (mut s, c, truth) = session(0.0);
    s.execute("clip.speedDuration", json!({"clips": [c.0], "speed": 200.0})).unwrap();
    s.execute("markers.addBeats", json!({"clip": c.0})).unwrap();
    let got = marker_times(&s);
    assert!(got.len() >= 50, "{}", got.len());
    for t in &got {
        let d = truth.iter().map(|b| (b / 2.0 - t).abs()).fold(f64::MAX, f64::min);
        assert!(d < 0.02, "marker at {t:.3} s is {d:.3} s off");
    }
    let end = s.active_sequence().unwrap().find_item(c).unwrap().1.duration.seconds();
    assert!(got.iter().all(|t| *t < end));
}

#[test]
fn refuses_what_has_no_beats() {
    let (mut s, c, _) = session(0.0);
    s.execute("edit.deselectAll", json!({})).unwrap();
    let why = (crate::commands::find("markers.addBeats").unwrap().enabled)(&s).unwrap_err();
    assert!(why.contains("music clip"), "{why}");
    assert!(s.execute("markers.addBeats", json!({"clip": c.0, "every": 3})).is_err());
    assert!(s.execute("markers.addBeats", json!({"clip": 999_999})).is_err());
    // silence has no beat: an error, not a panic, and nothing changes
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "quiet", "audio": 1, "video": 1})).unwrap();
    let bytes: Arc<[u8]> = crate::previews::write_wav_f32(&vec![0.0f32; SR as usize * 40], SR).into();
    let item = crate::commands::import_bytes(&mut s, "/quiet.wav", bytes, None).unwrap();
    let r = s.execute("timeline.place", json!({"item": item.0, "audioTrack": "A1", "seconds": 0.0})).unwrap();
    let c = r["clips"][0].as_u64().unwrap();
    assert!(s.execute("markers.addBeats", json!({"clip": c})).is_err());
    assert!(s.active_sequence().unwrap().markers.is_empty());
}
