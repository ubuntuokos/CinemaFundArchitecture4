//! `edit.deleteShortClips`: reports by default, deletes and closes the gap on request, skips locked tracks.

use cfa4_project::ClipId;
use cfa4_time::Tick;
use serde_json::json;

use crate::Session;

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

/// Cut V1's first clip so its tail is `frames` frames long; returns (tail clip id, V1 clip count).
fn cut_tail(s: &mut Session, frames: i64) -> (u64, usize) {
    let seq = s.active_sequence().unwrap();
    let rate = seq.frame_rate();
    let first = seq.video_tracks[0].items[0].clone();
    let at = Tick(first.end().0 - rate.tick_of(frames).0);
    s.execute("timeline.razor", json!({"time": at.0, "clip": first.id.0})).unwrap();
    let v1 = &s.active_sequence().unwrap().video_tracks[0].items;
    let tail = v1.iter().find(|i| i.start == at).unwrap().id.0;
    (tail, v1.len())
}

#[test]
fn default_is_a_dry_run() {
    let mut s = demo();
    let (tail, n) = cut_tail(&mut s, 3);
    let r = s.execute("edit.deleteShortClips", json!({"minFrames": 5})).unwrap();
    assert_eq!(r["dryRun"], json!(true), "{r}");
    assert_eq!(r["deleted"], json!(false), "{r}");
    assert!(r["found"].as_array().unwrap().iter().any(|f| f["clip"] == json!(tail) && f["frames"] == json!(3)), "{r}");
    assert_eq!(s.active_sequence().unwrap().video_tracks[0].items.len(), n, "a dry run deletes nothing");
}

#[test]
fn deletes_in_one_undo_step() {
    let mut s = demo();
    let (tail, n) = cut_tail(&mut s, 3);
    let r = s.execute("edit.deleteShortClips", json!({"minFrames": 5, "dryRun": false, "ripple": false})).unwrap();
    assert_eq!(r["deleted"], json!(true), "{r}");
    let seq = s.active_sequence().unwrap();
    assert!(seq.find_item(ClipId(tail)).is_none());
    assert!(seq.video_tracks[0].items.len() < n);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active_sequence().unwrap().find_item(ClipId(tail)).is_some(), "one undo restores it");
}

#[test]
fn rejects_bad_parameters_and_unknown_tracks() {
    let mut s = demo();
    assert!(s.execute("edit.deleteShortClips", json!({})).is_err());
    assert!(s.execute("edit.deleteShortClips", json!({"minFrames": 0})).is_err());
    assert!(s.execute("edit.deleteShortClips", json!({"minFrames": 5, "tracks": [999_999]})).is_err());
}
