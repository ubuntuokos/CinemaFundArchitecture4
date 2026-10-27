//! `effects.inspect`: a clip's effects as structured JSON (definitions, values, keyframes) in the
//! shapes `effects.setParam` / `effects.setKeyframe` / `effects.setInterpolation` take back.

use super::*;
use serde_json::json;

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

fn v1_clip(s: &Session) -> cfa4_project::TrackItem {
    s.active_sequence().unwrap().video_tracks[0].items[1].clone()
}

fn param<'a>(v: &'a Value, effect: &str, param: &str) -> &'a Value {
    let e = v["effects"].as_array().unwrap().iter().find(|e| e["effect"] == effect).unwrap();
    e["params"].as_array().unwrap().iter().find(|p| p["id"] == param).unwrap()
}

#[test]
fn inspects_definitions_values_and_keyframes() {
    let mut s = demo();
    let it = v1_clip(&s);
    // nothing selected and no clip named: a clear error
    s.execute("edit.deselectAll", json!({})).unwrap();
    assert!(s.execute("effects.inspect", json!({})).is_err());
    assert!(s.execute("effects.inspect", json!({"clip": 987_654_321})).is_err());

    s.execute("timeline.select", json!({"clips": [it.id.0]})).unwrap();
    let v = s.execute("effects.inspect", json!({})).unwrap();
    assert_eq!(v["clip"], json!(it.id.0));
    let scale = param(&v, "motion", "scale");
    assert_eq!((scale["type"].as_str(), scale["animated"].as_bool()), (Some("float"), Some(false)));
    assert_eq!(scale["value"], json!(100.0));
    let blend = param(&v, "opacity", "blend");
    assert_eq!(blend["type"], "choice");
    assert!(blend["options"].as_array().unwrap().len() > 10);

    // a static change reads back as the value that setParam takes
    s.execute("effects.setParam", json!({"clip": it.id.0, "effect": "motion", "param": "rotation", "value": 30.0})).unwrap();
    s.execute("effects.setParam", json!({"clip": it.id.0, "effect": "motion", "param": "position", "value": [100.0, 200.0]})).unwrap();
    let v = s.execute("effects.inspect", json!({"clip": it.id.0})).unwrap();
    assert_eq!(param(&v, "motion", "rotation")["value"], json!(30.0));
    assert_eq!(param(&v, "motion", "position")["value"], json!([100.0, 200.0]));

    // keyframes at the clip's start and end: media and timeline times, values, interpolation
    let t0 = it.start;
    let t1 = it.end() - Tick(1);
    for (t, val) in [(t0, 50.0), (t1, 150.0)] {
        s.execute("effects.setParam", json!({"clip": it.id.0, "effect": "motion", "param": "scale", "value": val, "keyframe": true, "time": t.0})).unwrap();
    }
    let mid = Tick((t0.0 + t1.0) / 2);
    s.execute("playhead.set", json!({"time": mid.0})).unwrap();
    let v = s.execute("effects.inspect", json!({"clip": it.id.0})).unwrap();
    assert_eq!(v["playheadInside"], json!(true));
    let scale = param(&v, "motion", "scale");
    assert_eq!(scale["animated"], json!(true));
    let kf = scale["keyframes"].as_array().unwrap();
    assert_eq!(kf.len(), 2);
    assert_eq!(kf[0]["mediaTime"], json!(it.source_time_at(t0).0));
    assert_eq!(kf[0]["time"], json!(t0.0), "timeline time maps back");
    assert_eq!(kf[1]["time"], json!(t1.0));
    assert_eq!(kf[0]["value"], json!(50.0));
    assert_eq!(kf[0]["interpolation"], "linear");
    let mid_value = scale["valueAtPlayhead"].as_f64().unwrap();
    assert!((mid_value - 100.0).abs() < 1.0, "halfway between 50 and 150: {mid_value}");
    // the interpolation id goes straight back into setInterpolation
    s.execute(
        "effects.setInterpolation",
        json!({"clip": it.id.0, "effect": "motion", "param": "scale", "mediaTime": kf[0]["mediaTime"], "interpolation": "hold"}),
    )
    .unwrap();
    let v = s.execute("effects.inspect", json!({"clip": it.id.0})).unwrap();
    assert_eq!(param(&v, "motion", "scale")["keyframes"][0]["interpolation"], "hold");
    assert_eq!(param(&v, "motion", "scale")["valueAtPlayhead"], json!(50.0));
}

#[test]
fn applied_effects_list_every_defined_parameter() {
    let mut s = demo();
    let it = v1_clip(&s);
    s.execute("effects.apply", json!({"clips": [it.id.0], "effect": "gaussian_blur"})).unwrap();
    let v = s.execute("effects.inspect", json!({"clip": it.id.0})).unwrap();
    let blur = v["effects"].as_array().unwrap().iter().find(|e| e["effect"] == "gaussian_blur").unwrap();
    assert_eq!(blur["name"], "Gaussian Blur");
    let def = cfa4_project::effect_defs().iter().find(|d| d.id == "gaussian_blur").unwrap();
    assert_eq!(blur["params"].as_array().unwrap().len(), def.params.len());
    assert!(blur["index"].as_u64().is_some());
}

#[test]
fn reversed_and_slowed_clips_map_keyframes_to_the_timeline() {
    let mut it = v1_clip(&demo());
    it.start = Tick(1000);
    it.duration = Tick(400);
    it.source_in = Tick(5000);
    it.frame_hold = None;
    it.speed = 0.5;
    assert_eq!(crate::commands::timeline_time_of(&it, it.source_time_at(Tick(1200))), Some(Tick(1200)));
    it.reverse = true;
    let m = it.source_time_at(Tick(1100));
    assert_eq!(crate::commands::timeline_time_of(&it, m), Some(Tick(1100)));
    // hostile times from a damaged project: no overflow, no panic
    it.reverse = false;
    it.speed = 1.0;
    it.source_in = Tick(i64::MAX);
    let _ = crate::commands::timeline_time_of(&it, Tick(i64::MIN));
    it.speed = 0.0;
    assert_eq!(crate::commands::timeline_time_of(&it, m), None);
}
