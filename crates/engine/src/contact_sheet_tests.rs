//! `qc.contactSheet`: a PNG of evenly spaced frames, the right grid, bad parameters refused.

use serde_json::json;

use crate::Session;
use crate::media_test_util::tmp_dir;

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

#[test]
fn writes_a_png_with_the_requested_grid() {
    let dir = tmp_dir("contact-sheet");
    let path = dir.join("sheet.png");
    let mut s = demo();
    let before = s.project.clone();
    let r = s.execute("qc.contactSheet", json!({"frames": 6, "columns": 3, "thumb": 64, "path": path.to_string_lossy()})).unwrap();
    assert_eq!(r["frames"].as_array().unwrap().len(), 6, "{r}");
    assert_eq!(r["columns"], json!(3), "{r}");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert!(r["width"].as_u64().unwrap() > 64 * 3, "{r}");
    let times: Vec<i64> = r["frames"].as_array().unwrap().iter().map(|f| f["time"].as_i64().unwrap()).collect();
    assert!(times.windows(2).all(|w| w[0] < w[1]), "frames are in time order: {times:?}");
    assert!(std::sync::Arc::ptr_eq(&before, &s.project), "the project is untouched");
}

#[test]
fn bad_parameters_are_refused() {
    let dir = tmp_dir("contact-sheet-bad");
    let mut s = demo();
    let p = dir.join("x.png").to_string_lossy().into_owned();
    assert!(s.execute("qc.contactSheet", json!({"frames": 0, "path": p})).is_err());
    assert!(s.execute("qc.contactSheet", json!({"frames": 65, "path": p})).is_err());
    assert!(s.execute("qc.contactSheet", json!({"columns": 0, "path": p})).is_err());
    assert!(s.execute("qc.contactSheet", json!({"path": dir.join("x.jpg").to_string_lossy()})).is_err());
    assert!(s.execute("qc.contactSheet", json!({"sequence": 1, "item": 2, "path": p})).is_err());
}
