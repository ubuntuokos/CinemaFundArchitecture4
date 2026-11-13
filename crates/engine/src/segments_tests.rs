//! `segments.*` (K2 of CFA4-LOSSLESS-MEDIA-TOOLS-001): the range markers of a media item are its
//! segments; they are listed in time order and cut in one batch.

use std::sync::Arc;

use cfa4_project::{ItemId, Label, Marker, MarkerId, MarkerKind};
use cfa4_time::{FrameRate, Tick};
use serde_json::json;

use crate::Session;
use crate::smart_cut_tests::source_mp4;

pub(crate) const RATE: FrameRate = FrameRate { num: 25, den: 1 };

pub(crate) fn marker(id: u64, from: i64, frames: i64, name: &str) -> Marker {
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
pub(crate) fn with_markers(tag: &str, markers: Vec<Marker>) -> (Session, ItemId, std::path::PathBuf) {
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

#[test]
fn segments_become_named_subclips_in_one_undo_step() {
    let (mut s, item, dir) = with_markers("segments-subclips", vec![marker(900, 0, 10, "intro"), marker(902, 20, 10, "")]);
    let before = s.project.items.len();
    let r = s.execute("segments.toSubclips", json!({"item": item.0})).unwrap();
    let made = r["subclips"].as_array().unwrap();
    assert_eq!(made.len(), 2);
    assert_eq!(s.project.items.len(), before + 2);
    let first = cfa4_project::ItemId(made[0]["item"].as_u64().unwrap());
    let it = s.project.item(first).unwrap();
    assert_eq!(it.name, "intro");
    match &it.kind {
        cfa4_project::ItemKind::Subclip { parent, range, .. } => {
            assert_eq!(*parent, item);
            assert_eq!((range.start, range.end()), (RATE.tick_of(0), RATE.tick_of(10)));
        }
        k => panic!("not a subclip: {k:?}"),
    }
    // an unnamed segment is named after the clip and its number
    let second = s.project.item(cfa4_project::ItemId(made[1]["item"].as_u64().unwrap())).unwrap();
    assert!(second.name.ends_with(" 2"), "{}", second.name);
    // one undo step removes both
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project.items.len(), before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn segments_are_placed_back_to_back_on_the_timeline() {
    let (mut s, item, dir) = with_markers("segments-timeline", vec![marker(900, 0, 10, "a"), marker(902, 20, 5, "b")]);
    let r = s.execute("segments.toTimeline", json!({"item": item.0, "segments": [902, 900], "time": 0})).unwrap();
    assert_eq!(r["subclips"][0]["marker"], 902);
    let made: Vec<u64> = r["subclips"].as_array().unwrap().iter().map(|x| x["item"].as_u64().unwrap()).collect();
    let seq = s.active_sequence().unwrap();
    // only the placed subclips (the test project already holds a clip of its own)
    let mut clips: Vec<(Tick, Tick)> = seq
        .tracks(cfa4_project::TrackKind::Video)
        .iter()
        .flat_map(|t| t.items.iter())
        .filter(|c| made.contains(&c.item.0))
        .map(|c| (c.start, c.duration))
        .collect();
    clips.sort();
    assert_eq!(clips, [(Tick(0), RATE.tick_of(5)), (RATE.tick_of(5), RATE.tick_of(10))]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn listed_segments_are_removed_renamed_and_labelled_in_undo_steps() {
    let (mut s, item, dir) = with_markers("segments-remove", vec![marker(900, 0, 10, "a"), marker(901, 20, 10, "b"), marker(902, 40, 5, "c")]);
    let names = |s: &mut crate::Session| -> Vec<String> {
        s.execute("segments.list", json!({"item": item.0})).unwrap()["segments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["name"].as_str().unwrap().to_string())
            .collect()
    };
    // nothing is removed without a list
    assert!(s.execute("segments.remove", json!({"item": item.0})).is_err());
    assert!(s.execute("segments.remove", json!({"item": item.0, "segments": [999]})).is_err());
    let r = s.execute("segments.remove", json!({"item": item.0, "segments": [901, 902]})).unwrap();
    assert_eq!(r["removed"], json!([901, 902]));
    assert_eq!(names(&mut s), ["a"]);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(names(&mut s), ["a", "b", "c"]);
    // renamed, commented and labelled through the Source monitor's marker editing
    s.execute("source.open", json!({"item": item.0})).unwrap();
    s.execute("markers.edit", json!({"target": "source", "marker": 900, "name": "Eleje", "comment": "jó", "color": "Rose"})).unwrap();
    let first = s.execute("segments.list", json!({"item": item.0})).unwrap()["segments"][0].clone();
    assert_eq!((first["name"].as_str(), first["comment"].as_str()), (Some("Eleje"), Some("jó")));
    let _ = std::fs::remove_dir_all(&dir);
}
