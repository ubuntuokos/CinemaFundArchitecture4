//! `file.import` of a Kdenlive project (`.kdenlive`, KDN-1): detected by content and extension, merged
//! as a sequence with its clips, the report lists what is not imported yet.

use serde_json::json;

use crate::Session;
use crate::media_test_util::tmp_dir;

#[test]
fn file_import_opens_a_kdenlive_project() {
    let root = tmp_dir("kdenlive-import");
    let path = root.join("mix.kdenlive");
    std::fs::write(&path, include_str!("../../interchange/tests/data/kdenlive-mlt/test-mix.kdenlive")).unwrap();
    let mut s = Session::default();
    let r = s.execute("file.import", json!({"paths": [path.to_string_lossy()]})).unwrap();
    assert!(r.get("errors").is_none_or(|e| e.as_array().is_none_or(|a| a.is_empty())), "{r}");
    let seqs: Vec<_> = s.project.items.values().filter(|i| i.as_sequence().is_some()).collect();
    assert_eq!(seqs.len(), 1, "{r}");
    let q = seqs[0].as_sequence().unwrap();
    assert_eq!((q.settings.width, q.settings.height), (720, 576));
    assert!(q.video_tracks.iter().any(|t| !t.items.is_empty()));
}
