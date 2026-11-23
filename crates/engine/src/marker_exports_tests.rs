//! `export.queue.addFromMarkers`: point markers become zones, range markers stay one range each,
//! the colour or name filter narrows them, nothing matching is an error.

use serde_json::{Value, json};

use cfa4_project::Label;

use crate::Session;

fn c(i: usize) -> &'static str {
    Label::ALL[i].name()
}

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

fn mark(s: &mut Session, sec: f64, color: &str, name: &str, frames: i64) {
    s.execute("markers.add", json!({"seconds": sec, "color": color, "name": name, "durationFrames": frames})).unwrap();
}

fn queued(r: &Value) -> usize {
    r.get("result").unwrap_or(r)["added"].as_array().map(Vec::len).unwrap_or(0)
}

#[test]
fn point_markers_make_consecutive_zones_to_the_end() {
    let mut s = demo();
    mark(&mut s, 1.0, c(0), "zz", 0);
    mark(&mut s, 2.0, c(0), "zz", 0);
    let r = s.execute("export.queue.addFromMarkers", json!({"format": "h264", "path": "/tmp/zones.mp4", "markers": "zz"})).unwrap();
    assert_eq!(queued(&r), 2, "{r}");
}

#[test]
fn range_markers_and_the_filter() {
    let mut s = demo();
    mark(&mut s, 1.0, c(0), "keep", 25);
    mark(&mut s, 3.0, c(1), "skip", 25);
    let r = s.execute("export.queue.addFromMarkers", json!({"format": "h264", "path": "/tmp/zones.mp4", "markers": "keep"})).unwrap();
    assert_eq!(queued(&r), 1, "{r}");
    let r = s.execute("export.queue.addFromMarkers", json!({"format": "h264", "path": "/tmp/zones2.mp4", "markers": "skip"})).unwrap();
    assert_eq!(queued(&r), 1, "{r}");
}

#[test]
fn nothing_matching_and_explicit_ranges_are_errors() {
    let mut s = demo();
    mark(&mut s, 1.0, c(0), "a", 0);
    assert!(s.execute("export.queue.addFromMarkers", json!({"format": "h264", "markers": "nomatch"})).is_err());
    assert!(s.execute("export.queue.addFromMarkers", json!({"format": "h264", "ranges": [{}]})).is_err());
    assert!(s.execute("export.queue.addFromMarkers", json!({"format": "h264", "markers": 3})).is_err());
}
