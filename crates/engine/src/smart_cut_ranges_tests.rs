//! Several cut ranges in one `export.plan` / `export.start` (K1 of CFA4-LOSSLESS-MEDIA-TOOLS-001):
//! one verified file per range, everything planned before anything is written.

use serde_json::{Value, json};

use crate::smart_cut_tests::{frame, session};

fn range(a: i64, b: i64) -> Value {
    json!({"in": frame(a), "out": frame(b)})
}

#[test]
fn plan_reports_every_range_and_the_totals() {
    let (mut s, _) = session();
    let r = s.execute("export.plan", json!({"path": "clip.mp4", "ranges": [range(0, 10), range(20, 40)]})).unwrap();
    let ranges = r["ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0]["range"], 1);
    assert_eq!(ranges[1]["summary"]["copiedFrames"], 20);
    assert_eq!(r["summary"], json!({"ranges": 2, "lossless": true, "requiresConfirmation": false, "copiedFrames": 30, "reencodedFrames": 0}));
    assert_eq!(r["problems"], json!([]));
}

#[test]
fn start_writes_one_verified_file_per_range() {
    let (mut s, files) = session();
    let r = s
        .execute(
            "export.start",
            json!({"path": "clip.mp4", "output": "out/{n}-{name}.mp4", "ranges": [
                {"in": frame(0), "out": frame(10), "name": "intro"},
                {"in": frame(20), "out": frame(30), "name": "../a/b"},
                {"in": frame(30), "out": frame(40), "output": "own.mov"},
            ]}),
        )
        .unwrap();
    assert_eq!(r["verified"], true);
    let outputs: Vec<&str> = r["ranges"].as_array().unwrap().iter().map(|x| x["output"].as_str().unwrap()).collect();
    // a name never leaves its path component
    assert_eq!(outputs, ["out/1-intro.mp4", "out/2-_a_b.mp4", "own.mov"]);
    let written = files.0.lock().unwrap();
    for o in outputs {
        assert!(written.get(o).is_some_and(|b| b.len() > 100), "{o} was not written");
    }
}

#[test]
fn a_range_that_cannot_run_stops_the_batch_before_anything_is_written() {
    let (mut s, files) = session();
    // the second range starts mid-GOP: refused in GOP-boundary mode
    let e = s.execute("export.start", json!({"path": "clip.mp4", "output": "o{n}.mp4", "ranges": [range(0, 10), range(5, 30)]})).unwrap_err().to_string();
    assert!(e.contains("range 2"), "{e}");
    let written = files.0.lock().unwrap();
    assert!(!written.contains_key("o1.mp4"), "nothing may be written when a range is refused");
}

#[test]
fn re_encoding_ranges_need_one_confirmation_for_the_batch() {
    let (mut s, files) = session();
    let mut p = json!({"path": "clip.mp4", "output": "c{n}.mp4", "mode": "frame-accurate", "ranges": [range(0, 10), range(5, 30)]});
    let e = s.execute("export.start", p.clone()).unwrap_err().to_string();
    assert!(e.contains("confirm") && e.contains("ranges 2"), "{e}");
    assert!(!files.0.lock().unwrap().contains_key("c1.mp4"));
    p["confirm"] = json!(true);
    let r = s.execute("export.start", p).unwrap();
    assert_eq!(r["ranges"].as_array().unwrap().len(), 2);
    assert!(r["summary"]["reencodedFrames"].as_u64().unwrap() > 0);
    let written = files.0.lock().unwrap();
    assert!(written.contains_key("c1.mp4") && written.contains_key("c2.mp4"));
}

#[test]
fn bad_range_lists_are_refused() {
    let (mut s, _) = session();
    let cases = [
        json!({"path": "clip.mp4", "output": "x{n}.mp4", "ranges": []}),
        json!({"path": "clip.mp4", "output": "x{n}.mp4", "ranges": "0-10"}),
        json!({"path": "clip.mp4", "output": "x{n}.mp4", "ranges": [42]}),
        json!({"path": "clip.mp4", "output": "x{n}.mp4", "ranges": [{"in": frame(0)}]}),
        json!({"path": "clip.mp4", "output": "x{n}.mp4", "ranges": [{"in": frame(0), "out": frame(10), "name": 7}]}),
        // two ranges, one output without {n}
        json!({"path": "clip.mp4", "output": "same.mp4", "ranges": [range(0, 10), range(10, 20)]}),
        // two ranges written to one file
        json!({"path": "clip.mp4", "ranges": [{"in": frame(0), "out": frame(10), "output": "a.mp4"}, {"in": frame(10), "out": frame(20), "output": "a.mp4"}]}),
        // no output at all
        json!({"path": "clip.mp4", "ranges": [range(0, 10)]}),
        // ranges are for a source, not for the timeline
        json!({"scope": "timeline", "output": "x{n}.mp4", "ranges": [range(0, 10)]}),
    ];
    for p in cases {
        assert!(s.execute("export.start", p.clone()).is_err(), "accepted: {p}");
    }
    let too_many: Vec<Value> = (0..=crate::smart_cut_ranges::MAX_RANGES).map(|_| range(0, 10)).collect();
    let e = s.execute("export.plan", json!({"path": "clip.mp4", "ranges": too_many})).unwrap_err().to_string();
    assert!(e.contains("at most"), "{e}");
}

#[test]
fn hostile_range_values_never_panic() {
    let (mut s, _) = session();
    let values = [json!(null), json!(-1), json!(u64::MAX), json!("x"), json!({"frame": -5}), json!({"seconds": 1e300}), json!([1, 2])];
    for a in &values {
        for b in &values {
            let p = json!({"path": "clip.mp4", "output": "h{n}.mp4", "ranges": [{"in": a, "out": b, "name": "\u{0}/.."}]});
            let _ = s.execute("export.plan", p.clone());
            let _ = s.execute("export.start", p);
        }
    }
}
