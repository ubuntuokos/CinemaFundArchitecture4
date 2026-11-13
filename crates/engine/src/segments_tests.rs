//! `segments.*` (K2 of CFA4-LOSSLESS-MEDIA-TOOLS-001): the range markers of a media item are its
//! segments; they are listed in time order and cut in one batch.

use std::sync::Arc;

use cfa4_project::{ItemId, Label, Marker, MarkerId, MarkerKind};
use cfa4_time::{FrameRate, Tick};
use serde_json::json;

use crate::Session;
use crate::smart_cut_tests::source_mp4;

const RATE: FrameRate = FrameRate { num: 25, den: 1 };

fn marker(id: u64, from: i64, frames: i64, name: &str) -> Marker {
    Marker {
        id: MarkerId(id),
        start: RATE.tick_of(from),
        duration: RATE.tick_of(frames),
        name: name.into(),
        comment: format!("note {id}"),
        kind: MarkerKind::Comment,
        color: Label::Green,
    }
}

/// A session with `clip.mp4` as a media item carrying `markers`.
fn with_markers(tag: &str, markers: Vec<Marker>) -> (Session, ItemId, std::path::PathBuf) {
    let dir = crate::media_test_util::tmp_dir(tag);
    let path = dir.join("clip.mp4");
    std::fs::write(&path, source_mp4()).unwrap();
    let (mut s, items, _) = crate::media_test_util::session_with(&[path.as_path()]);
    let item = items[0];
    let mut p = (*s.project).clone();
    if let Some(cfa4_project::ItemKind::Media(m)) = p.item_mut(item).map(|i| &mut i.kind) {
        m.markers = markers;
    }
    s.project = Arc::new(p);
    (s, item, dir)
}

#[test]
fn range_markers_are_listed_in_time_order_and_point_markers_are_not_segments() {
    let (mut s, item, dir) = with_markers("segments-list", vec![marker(900, 20, 10, "b"), marker(901, 5, 0, "point"), marker(902, 0, 10, "a")]);
    let r = s.execute("segments.list", json!({"item": item.0})).unwrap();
    let names: Vec<&str> = r["segments"].as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["a", "b"]);
    assert_eq!(r["segments"][0]["out"], RATE.tick_of(10).0);
    assert_eq!(r["segments"][1]["comment"], "note 900");
    // the Source monitor's clip is the default item
    s.state.source_item = Some(item);
    assert_eq!(s.execute("segments.list", json!({})).unwrap()["segments"], r["segments"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn segments_are_cut_one_file_each_in_the_chosen_order() {
    let (mut s, item, dir) = with_markers("segments-export", vec![marker(900, 0, 10, "intro"), marker(902, 20, 10, "end")]);
    let pattern = dir.join("{n}-{name}.mp4").to_string_lossy().into_owned();
    let r = s.execute("segments.plan", json!({"item": item.0, "output": pattern})).unwrap();
    assert_eq!(r["summary"]["copiedFrames"], 20, "{r}");
    assert_eq!(r["ranges"][0]["marker"], 900);
    // an explicit list sets the order
    let r = s.execute("segments.export", json!({"item": item.0, "segments": [902, 900], "output": pattern})).unwrap();
    assert_eq!(r["verified"], true);
    assert_eq!(r["ranges"][0]["marker"], 902);
    for name in ["1-end.mp4", "2-intro.mp4"] {
        assert!(dir.join(name).is_file(), "{name} was not written");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_segment_requests_are_refused() {
    let (mut s, item, dir) = with_markers("segments-bad", vec![marker(900, 0, 10, "a")]);
    let out = dir.join("{n}.mp4").to_string_lossy().into_owned();
    for p in [
        json!({"item": item.0, "segments": [], "output": out}),
        json!({"item": item.0, "segments": [12345], "output": out}),
        json!({"item": item.0, "segments": [900, 900], "output": out}),
        json!({"item": item.0, "segments": "900", "output": out}),
        json!({"item": "x", "output": out}),
        json!({"item": 999_999, "output": out}),
        json!({"output": out}), // no item and no Source clip
    ] {
        assert!(s.execute("segments.export", p.clone()).is_err(), "accepted: {p}");
    }
    // an item without segments
    let (mut s2, item2, dir2) = with_markers("segments-none", vec![]);
    let e = s2.execute("segments.plan", json!({"item": item2.0})).unwrap_err().to_string();
    assert!(e.contains("no segments"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
}

#[test]
fn hostile_marker_times_never_panic() {
    let huge = Marker { start: Tick(i64::MAX - 1), duration: Tick(i64::MAX), ..marker(900, 0, 1, "x") };
    let negative = Marker { start: Tick(i64::MIN), duration: Tick(5), ..marker(901, 0, 1, "y") };
    let (mut s, item, dir) = with_markers("segments-hostile", vec![huge, negative]);
    let out = dir.join("{n}.mp4").to_string_lossy().into_owned();
    let _ = s.execute("segments.list", json!({"item": item.0}));
    let _ = s.execute("segments.plan", json!({"item": item.0, "output": out}));
    let _ = s.execute("segments.export", json!({"item": item.0, "output": out}));
    let _ = std::fs::remove_dir_all(&dir);
}
