//! Markers from CSV and EDL (K6 of CFA4-LOSSLESS-MEDIA-TOOLS-001): what `markers.exportCsv` and
//! `markers.exportEdl` write comes back unchanged, and a different time base is refused.

use cfa4_project::{Label, Marker, MarkerKind};
use cfa4_time::Tick;
use serde_json::{Value, json};

use crate::segments_tests::{RATE, marker, with_markers};

fn path(dir: &std::path::Path, name: &str) -> String {
    dir.join(name).to_string_lossy().into_owned()
}

fn media_markers(s: &crate::Session, item: cfa4_project::ItemId) -> Vec<Marker> {
    let view = crate::clip_ops::source_view(s, item).unwrap();
    s.project.item(view.media).unwrap().as_media().unwrap().markers.clone()
}

/// The fields a round trip must keep (ids are new).
fn fields(m: &[Marker]) -> Vec<(Tick, Tick, String, String, MarkerKind, Label)> {
    let mut v: Vec<_> = m.iter().map(|m| (m.start, m.duration, m.name.clone(), m.comment.clone(), m.kind, m.color)).collect();
    v.sort_by_key(|x| (x.0, x.1));
    v
}

fn clear(s: &mut crate::Session, item: cfa4_project::ItemId) {
    let view = crate::clip_ops::source_view(s, item).unwrap();
    s.edit("clear", |pr, _| {
        pr.item_mut(view.media).unwrap().as_media_mut().unwrap().markers.clear();
        Ok(())
    })
    .unwrap();
}

#[test]
fn segments_go_out_as_csv_and_come_back_unchanged() {
    let tricky = Marker { comment: "line 1\r\nline 2, \"quoted\"".into(), kind: MarkerKind::Chapter, color: Label::Rose, ..marker(901, 20, 10, "=SUM(A1)") };
    // text that starts with an apostrophe comes back as it was, guarded or not
    let quoted = Marker { comment: "'=not a formula".into(), ..marker(903, 30, 2, "'tis") };
    let (mut s, item, dir) =
        with_markers("mimport-csv", vec![marker(900, 0, 10, "intro, 日本語"), tricky, marker(902, 35, 0, "point"), quoted, marker(904, 32, 1, "''@x")]);
    let before = fields(&media_markers(&s, item));
    let csv = path(&dir, "seg.csv");
    s.execute("markers.exportCsv", json!({"path": csv, "item": item.0})).unwrap();
    clear(&mut s, item);
    let undo = s.history.undo.len();
    let r = s.execute("markers.importCsv", json!({"path": csv, "item": item.0})).unwrap();
    assert_eq!(r["markers"].as_array().unwrap().len(), 5, "{r}");
    assert_eq!(fields(&media_markers(&s, item)), before);
    assert_eq!(s.history.undo.len(), undo + 1, "one undo step");
    // importing the same file again adds nothing
    let again = s.execute("markers.importCsv", json!({"path": csv, "item": item.0})).unwrap();
    assert_eq!(again["skipped"], 5, "{again}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_csv_with_timecodes_only_is_read_at_the_targets_rate() {
    let (mut s, item, dir) = with_markers("mimport-tc", vec![]);
    let csv = path(&dir, "tc.csv");
    std::fs::write(&csv, "Name,Start Timecode,End Timecode,Color\r\nA,00:00:00:05,00:00:00:15,Blue\r\nB,00:00:01:00,,\r\n").unwrap();
    s.execute("markers.importCsv", json!({"path": csv, "item": item.0})).unwrap();
    let got = fields(&media_markers(&s, item));
    assert_eq!(got[0].0, RATE.tick_of(5));
    assert_eq!(got[0].1, RATE.tick_of(10));
    assert_eq!(got[0].5, Label::Blue);
    assert_eq!((got[1].0, got[1].1), (RATE.tick_of(25), Tick::ZERO));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_different_frame_rate_is_refused_unless_allowed() {
    let (mut s, item, dir) = with_markers("mimport-rate", vec![]);
    let csv = path(&dir, "rate.csv");
    // written at 24 fps; the clip runs at 25
    std::fs::write(&csv, "Name,Start Timecode,Duration Frames,Frame Rate\r\nA,00:00:01:00,12,24/1\r\n").unwrap();
    let e = s.execute("markers.importCsv", json!({"path": csv, "item": item.0})).unwrap_err().to_string();
    assert!(e.contains("24") && e.contains("allowRateChange"), "{e}");
    assert!(media_markers(&s, item).is_empty(), "nothing is imported");
    // allowed: the times are read at the file's own rate (one second, half a second)
    s.execute("markers.importCsv", json!({"path": csv, "item": item.0, "allowRateChange": true})).unwrap();
    let got = fields(&media_markers(&s, item));
    assert_eq!((got[0].0, got[0].1), (Tick::from_seconds_f64(1.0), Tick::from_seconds_f64(0.5)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn segments_go_out_as_an_edl_and_come_back_unchanged() {
    let (mut s, item, dir) = with_markers("mimport-edl", vec![marker(900, 0, 10, "intro"), marker(901, 20, 15, "end part"), marker(902, 12, 0, "a point")]);
    let edl = path(&dir, "seg.edl");
    let r = s.execute("markers.exportEdl", json!({"path": edl, "item": item.0})).unwrap();
    assert_eq!(r["events"], 2, "point markers are not segments");
    let text = std::fs::read_to_string(&edl).unwrap();
    assert!(text.contains("FCM: NON-DROP FRAME") && text.contains("* SEGMENT NAME: end part"), "{text}");
    clear(&mut s, item);
    s.execute("markers.importEdl", json!({"path": edl, "item": item.0})).unwrap();
    let got: Vec<(Tick, Tick, String)> = fields(&media_markers(&s, item)).into_iter().map(|m| (m.0, m.1, m.2)).collect();
    assert_eq!(got, [(Tick::ZERO, RATE.tick_of(10), "intro".to_string()), (RATE.tick_of(20), RATE.tick_of(15), "end part".to_string())]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_edl_of_another_rate_or_outside_the_clip_is_refused() {
    let (mut s, item, dir) = with_markers("mimport-edl-bad", vec![]);
    let edl = path(&dir, "bad.edl");
    // frame 29 cannot exist at 25 fps
    std::fs::write(&edl, "TITLE: x\nFCM: NON-DROP FRAME\n\n001  AX       V     C        00:00:00:00 00:00:00:29 01:00:00:00 01:00:00:29\n").unwrap();
    let e = s.execute("markers.importEdl", json!({"path": edl, "item": item.0})).unwrap_err().to_string();
    assert!(e.contains("frame rate"), "{e}");
    // drop-frame timecode cannot be 25 fps either
    std::fs::write(&edl, "TITLE: x\nFCM: DROP FRAME\n\n001  AX       V     C        00;00;00;00 00;00;00;10 01;00;00;00 01;00;00;10\n").unwrap();
    assert!(s.execute("markers.importEdl", json!({"path": edl, "item": item.0})).is_err());
    // the clip holds 40 frames
    std::fs::write(&edl, "TITLE: x\nFCM: NON-DROP FRAME\n\n001  AX       V     C        00:00:01:00 00:00:02:00 01:00:00:00 01:00:01:00\n").unwrap();
    let e = s.execute("markers.importEdl", json!({"path": edl, "item": item.0})).unwrap_err().to_string();
    assert!(e.contains("outside"), "{e}");
    assert!(media_markers(&s, item).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sequence_markers_round_trip_through_csv() {
    let (mut s, _item, dir) = with_markers("mimport-seq", vec![]);
    for (t, name) in [(0, "a"), (48, "b")] {
        s.execute("markers.add", json!({"time": s.sequence_rate().tick_of(t).0, "name": name, "durationFrames": 3, "color": "Mango"})).unwrap();
    }
    let before: Vec<_> = s.active_sequence().unwrap().markers.iter().map(|m| (m.start, m.duration, m.name.clone(), m.color)).collect();
    let csv = path(&dir, "seq.csv");
    s.execute("markers.exportCsv", json!({"path": csv})).unwrap();
    s.execute("markers.clearAll", json!({})).unwrap();
    s.execute("markers.importCsv", json!({"path": csv})).unwrap();
    let after: Vec<_> = s.active_sequence().unwrap().markers.iter().map(|m| (m.start, m.duration, m.name.clone(), m.color)).collect();
    assert_eq!(after, before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_files_and_hostile_values_are_refused_without_panicking() {
    let (mut s, item, dir) = with_markers("mimport-bad", vec![]);
    let csv = path(&dir, "bad.csv");
    let cases = [
        "",
        "Name,Comment\r\nx,y\r\n",
        "Start Ticks\r\nabc\r\n",
        "Start Ticks,Duration Ticks\r\n-5,1\r\n",
        "Start Ticks,Duration Ticks\r\n0,-1\r\n",
        "Start Ticks,Duration Ticks\r\n9223372036854775807,9223372036854775807\r\n",
        "Start Ticks,Kind\r\n0,Nonsense\r\n",
        "Start Ticks,Color\r\n0,Plaid\r\n",
        "Start Ticks,Frame Rate\r\n0,0/0\r\n",
        "Start Timecode\r\n99:99\r\n",
        "Start Ticks\r\n\"0\r\n",
    ];
    for c in cases {
        std::fs::write(&csv, c).unwrap();
        assert!(s.execute("markers.importCsv", json!({"path": csv, "item": item.0})).is_err(), "accepted {c:?}");
    }
    for p in [json!({}), json!({"path": ""}), json!({"path": "\u{0}"}), json!({"path": csv, "item": "x"}), json!({"path": csv, "target": "elsewhere"})] {
        assert!(s.execute("markers.importCsv", p.clone()).is_err(), "accepted {p}");
    }
    let missing = path(&dir, "missing.edl");
    assert!(s.execute("markers.importEdl", json!({"path": missing, "item": item.0})).is_err());
    assert!(s.execute("markers.exportEdl", json!({"path": path(&dir, "x.edl"), "item": item.0})).is_err(), "no segments to write");
    let hostile = [json!(null), json!(-1), json!(i64::MAX), json!("x"), json!([1])];
    for v in &hostile {
        let _ = s.execute("markers.importCsv", json!({"path": v, "item": v, "allowRateChange": v}));
        let _ = s.execute("markers.importEdl", json!({"path": v, "item": v}));
        let _ = s.execute("markers.exportEdl", json!({"path": v, "item": v}));
    }
    assert!(media_markers(&s, item).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
    let _: Option<Value> = None;
}

#[test]
fn csv_limits_apply_while_the_file_is_read() {
    let (mut s, item, dir) = with_markers("mimport-limits", vec![]);
    let csv = path(&dir, "big.csv");
    let wide = format!("Name,Start Ticks{}\r\nA,0\r\n", ",x".repeat(300));
    std::fs::write(&csv, wide).unwrap();
    let e = s.execute("markers.importCsv", json!({"path": csv, "item": item.0})).unwrap_err().to_string();
    assert!(e.contains("fields"), "{e}");
    let long = format!("Name,Start Ticks\r\n\"{}\",0\r\n", "a".repeat(70 * 1024));
    std::fs::write(&csv, long).unwrap();
    let e = s.execute("markers.importCsv", json!({"path": csv, "item": item.0})).unwrap_err().to_string();
    assert!(e.contains("longer than"), "{e}");
    let rows = format!("Name,Start Ticks\n{}", "A,0\n".repeat(100_001));
    std::fs::write(&csv, rows).unwrap();
    let e = s.execute("markers.importCsv", json!({"path": csv, "item": item.0})).unwrap_err().to_string();
    assert!(e.contains("rows"), "{e}");
    assert!(media_markers(&s, item).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn csv_export_of_a_named_item_is_enabled_without_an_open_clip() {
    let (mut s, item, dir) = with_markers("mimport-enabled", vec![marker(900, 0, 10, "a")]);
    s.state.source_item = None;
    s.state.active_sequence = None;
    assert!(crate::marker_import::has_target(&s).is_ok(), "a media item can be named with `item`");
    let csv = path(&dir, "named.csv");
    s.execute("markers.exportCsv", json!({"path": csv, "item": item.0})).unwrap();
    assert!(std::fs::read_to_string(&csv).unwrap().contains("\"a\""));
    let _ = std::fs::remove_dir_all(&dir);
}
