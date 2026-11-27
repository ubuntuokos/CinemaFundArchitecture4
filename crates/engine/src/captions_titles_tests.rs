//! `captions.importAsTitles`: every cue becomes a timed text clip, in one undo step; markup is
//! stripped; bad input is refused.

use serde_json::json;

use crate::Session;
use crate::media_test_util::tmp_dir;

const SRT: &str = "1\n00:00:01,000 --> 00:00:03,000\n<i>Hello</i>\nworld\n\n2\n00:00:04,000 --> 00:00:05,500\nSecond\n";

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

fn graphic_clips(s: &Session) -> usize {
    s.active_sequence().unwrap().video_tracks.iter().flat_map(|t| t.items.iter()).filter(|i| i.name == "Hello" || i.name == "Second").count()
}

#[test]
fn cues_become_timed_title_clips_in_one_undo_step() {
    let dir = tmp_dir("cap-titles");
    let path = dir.join("a.srt");
    std::fs::write(&path, SRT).unwrap();
    let mut s = demo();
    let before = graphic_clips(&s);
    let r = s.execute("captions.importAsTitles", json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(r["cues"], json!(2), "{r}");
    assert_eq!(graphic_clips(&s), before + 2);
    let seq = s.active_sequence().unwrap();
    let rate = seq.frame_rate();
    let ids: Vec<u64> = r["clips"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    let first = seq.find_item(cfa4_project::ClipId(ids[0])).unwrap().1;
    assert_eq!(rate.frame_at(first.start), rate.frame_at(cfa4_time::Tick::from_seconds_f64(1.0)));
    assert!((first.duration.seconds() - 2.0).abs() < 0.05, "{}", first.duration.seconds());
    assert!(s.active_sequence().unwrap().caption_tracks.iter().all(|t| t.captions.is_empty()), "no captions are made");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(graphic_clips(&s), before, "one undo removes every title");
}

#[test]
fn missing_files_and_non_caption_files_are_refused() {
    let dir = tmp_dir("cap-titles-bad");
    let junk = dir.join("junk.txt");
    std::fs::write(&junk, "not captions at all").unwrap();
    let mut s = demo();
    assert!(s.execute("captions.importAsTitles", json!({})).is_err());
    assert!(s.execute("captions.importAsTitles", json!({"path": dir.join("nope.srt").to_string_lossy()})).is_err());
    assert!(s.execute("captions.importAsTitles", json!({"path": junk.to_string_lossy()})).is_err());
}
