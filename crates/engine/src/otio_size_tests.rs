//! OTIO-2.5 (H5): OTIO has no picture size, so a sequence from another program's `.otio` takes the
//! size of its first video clip once the import has opened that media. CFA4's own files keep the
//! settings they store.

use cfa4_media::DemoScene;
use cfa4_project::ItemId;
use serde_json::json;

use crate::Session;
use crate::media_test_util::{make_movie, tmp_dir};

fn rt(v: i64) -> String {
    format!(r#"{{"OTIO_SCHEMA": "RationalTime.1", "rate": 24.0, "value": {v}}}"#)
}

fn clip(name: &str, url: &str, dur: i64) -> String {
    let tr = |s: i64, d: i64| format!(r#"{{"OTIO_SCHEMA": "TimeRange.1", "start_time": {}, "duration": {}}}"#, rt(s), rt(d));
    format!(
        r#"{{"OTIO_SCHEMA": "Clip.2", "name": "{name}", "source_range": {}, "active_media_reference_key": "DEFAULT_MEDIA",
            "media_references": {{"DEFAULT_MEDIA": {{"OTIO_SCHEMA": "ExternalReference.1", "target_url": "{url}", "available_range": {}}}}}}}"#,
        tr(0, dur),
        tr(0, dur)
    )
}

/// V1: a 12-frame gap, then `late.mov`; V2: `first.mov` from frame 0. Another program's file: no
/// `cfa4` metadata.
fn foreign_doc() -> String {
    let gap = format!(r#"{{"OTIO_SCHEMA": "Gap.1", "source_range": {{"OTIO_SCHEMA": "TimeRange.1", "start_time": {}, "duration": {}}}}}"#, rt(0), rt(12));
    format!(
        r#"{{"OTIO_SCHEMA": "Timeline.1", "name": "Foreign", "metadata": {{"Resolve_OTIO": {{}}}}, "tracks": {{"OTIO_SCHEMA": "Stack.1", "children": [
            {{"OTIO_SCHEMA": "Track.1", "name": "V1", "kind": "Video", "children": [{gap}, {}]}},
            {{"OTIO_SCHEMA": "Track.1", "name": "V2", "kind": "Video", "children": [{}]}}]}}}}"#,
        clip("late", "media/late.mov", 12),
        clip("first", "media/first.mov", 12)
    )
}

fn import(s: &mut Session, doc: &std::path::Path) -> (serde_json::Value, ItemId) {
    let r = s.execute("file.import", json!({"paths": [doc.to_string_lossy()]})).unwrap();
    let seq = ItemId(r["sequences"][0].as_u64().unwrap_or_else(|| panic!("no sequence in {r}")));
    (r["documents"][0].clone(), seq)
}

fn size(s: &Session, seq: ItemId) -> (u32, u32) {
    let q = s.project.sequence(seq).unwrap();
    (q.settings.width, q.settings.height)
}

#[test]
fn a_foreign_otio_sequence_takes_its_first_video_clips_size() {
    let d = tmp_dir("otio-size");
    std::fs::create_dir_all(d.join("media")).unwrap();
    make_movie(&d.join("media/first.mov"), DemoScene::OceanSunset, 320, 180, 12);
    make_movie(&d.join("media/late.mov"), DemoScene::OceanSunset, 640, 360, 12);
    let doc = d.join("cut.otio");
    std::fs::write(&doc, foreign_doc()).unwrap();
    let mut s = Session::default();
    let (r, seq) = import(&mut s, &doc);
    assert_eq!(r["linkedMedia"].as_u64(), Some(2), "{r}");
    // the clip that starts first, though it is on V2
    assert_eq!(size(&s, seq), (320, 180));
    let settings = &s.project.sequence(seq).unwrap().settings;
    assert_eq!(settings.frame_rate, cfa4_time::FrameRate::FPS_24, "the rate stays the document's");
    assert!(settings.preset.contains("320x180"), "{}", settings.preset);

    // CFA4's own export stores the settings: they are kept as written, whatever the media
    if let Some(q) = std::sync::Arc::make_mut(&mut s.project).sequence_mut(seq) {
        q.settings.width = 1280;
        q.settings.height = 720;
    }
    let own = d.join("own.otio");
    s.execute("file.exportInterchange", json!({"format": "otio", "path": own.to_string_lossy(), "sequence": seq.0})).unwrap();
    let mut t = Session::default();
    let (_, seq2) = import(&mut t, &own);
    assert_eq!(size(&t, seq2), (1280, 720));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn without_its_media_a_foreign_otio_sequence_keeps_the_default_size() {
    let d = tmp_dir("otio-size-offline");
    let doc = d.join("cut.otio");
    std::fs::write(&doc, foreign_doc()).unwrap();
    let mut s = Session::default();
    let (r, seq) = import(&mut s, &doc);
    assert_eq!(r["linkedMedia"].as_u64(), Some(0), "{r}");
    assert_eq!(size(&s, seq), (1920, 1080));
    let _ = std::fs::remove_dir_all(&d);
}
