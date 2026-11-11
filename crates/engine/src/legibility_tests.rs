use serde_json::{Value, json};

use crate::Session;
use crate::legibility::{CHECK_COMMENT, contrast_ratio};
use cfa4_project::{ClipId, Param, ParamValue};

/// A 320×180 sequence with a full-frame matte of `matte` on V1 and a text title on V2.
fn titled(matte: &str, text_params: Value) -> (Session, ClipId) {
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "Titles", "fps": 24, "width": 320, "height": 180, "video": 2, "audio": 1})).unwrap();
    let m = s.execute("file.newColorMatte", json!({"color": matte, "seconds": 4.0})).unwrap()["item"].as_u64().unwrap();
    s.execute("timeline.place", json!({"item": m, "track": "V1", "seconds": 0.0})).unwrap();
    let mut p = json!({"text": "Hello", "size": 30, "seconds": 4.0, "track": 1, "time": 0});
    if let (Some(o), Some(extra)) = (p.as_object_mut(), text_params.as_object()) {
        o.extend(extra.clone());
    }
    let clip = ClipId(s.execute("graphics.newText", p).unwrap()["clip"].as_u64().unwrap());
    let q = s.active_sequence().unwrap();
    assert_eq!(q.video_tracks[1].items.len(), 1, "the title is on V2");
    (s, clip)
}

/// Set appearance parameters of the clip's first layer.
fn style(s: &mut Session, clip: ClipId, params: &[(&str, ParamValue)]) {
    s.edit_sequence("Style", |q, _, _| {
        let (_, it) = q.find_item_mut(clip).unwrap();
        let i = cfa4_project::graphic::layer_indices(&it.effects)[0];
        for (k, v) in params {
            it.effects[i].params.insert((*k).into(), Param::new(v.clone()));
        }
        Ok(())
    })
    .unwrap();
}

fn kinds(r: &Value) -> Vec<String> {
    r["issues"].as_array().unwrap().iter().map(|i| i["kind"].as_str().unwrap().to_string()).collect()
}

#[test]
fn contrast_ratio_matches_wcag() {
    assert!((contrast_ratio(1.0, 0.0) - 21.0).abs() < 1e-9);
    assert!((contrast_ratio(0.0, 1.0) - 21.0).abs() < 1e-9);
    assert!((contrast_ratio(0.5, 0.5) - 1.0).abs() < 1e-9);
}

#[test]
fn white_text_on_white_is_flagged_and_black_text_is_not() {
    let (mut s, clip) = titled("#ffffff", json!({}));
    let r = s.execute("graphics.checkLegibility", json!({})).unwrap();
    assert_eq!(r["checked"], 1, "{r}");
    assert_eq!(kinds(&r), ["lowContrast"], "{r}");
    let c = r["issues"][0]["contrast"].as_f64().unwrap();
    assert!(c < 1.5, "{r}");
    assert_eq!(r["issues"][0]["clip"], clip.0);
    // the problem is marked on the timeline
    let q = s.active_sequence().unwrap();
    assert_eq!(q.markers.iter().filter(|m| m.comment.starts_with(CHECK_COMMENT)).count(), 1);

    style(&mut s, clip, &[("fill_color", ParamValue::Color([0.0, 0.0, 0.0, 1.0]))]);
    let r = s.execute("graphics.checkLegibility", json!({})).unwrap();
    assert_eq!(r["ok"], true, "{r}");
    // running again replaces the markers
    assert!(!s.active_sequence().unwrap().markers.iter().any(|m| m.comment.starts_with(CHECK_COMMENT)));
}

#[test]
fn a_background_box_or_an_outline_rescues_the_text() {
    let (mut s, clip) = titled("#ffffff", json!({}));
    style(
        &mut s,
        clip,
        &[
            ("background", ParamValue::Bool(true)),
            ("background_color", ParamValue::Color([0.0, 0.0, 0.0, 1.0])),
            ("background_opacity", ParamValue::Float(100.0)),
        ],
    );
    assert_eq!(s.execute("graphics.checkLegibility", json!({})).unwrap()["ok"], true);
    // a faint box is not enough
    style(&mut s, clip, &[("background_opacity", ParamValue::Float(10.0))]);
    assert_eq!(kinds(&s.execute("graphics.checkLegibility", json!({})).unwrap()), ["lowContrast"]);
    // a black outer stroke 4 px wide on 30 px text
    style(&mut s, clip, &[("background", ParamValue::Bool(false)), ("stroke", ParamValue::Bool(true)), ("stroke_width", ParamValue::Float(4.0))]);
    assert_eq!(s.execute("graphics.checkLegibility", json!({})).unwrap()["ok"], true);
    // a hairline does not count
    style(&mut s, clip, &[("stroke_width", ParamValue::Float(0.5))]);
    assert_eq!(kinds(&s.execute("graphics.checkLegibility", json!({})).unwrap()), ["lowContrast"]);
}

#[test]
fn the_shape_behind_the_text_counts() {
    // white text on a white frame, but over a black bar in the same graphic
    let (mut s, clip) = titled("#ffffff", json!({}));
    let r = s.execute("graphics.newShape", json!({"clip": clip.0})).unwrap();
    assert!(r["layer"].is_number(), "{r}");
    s.edit_sequence("Bar", |q, _, _| {
        let (_, it) = q.find_item_mut(clip).unwrap();
        let idx = cfa4_project::graphic::layer_indices(&it.effects);
        let (text, shape) = (idx[0], idx[1]);
        let e = &mut it.effects[shape];
        e.params.insert("fill_color".into(), Param::new(ParamValue::Color([0.0, 0.0, 0.0, 1.0])));
        e.params.insert("size".into(), Param::new(ParamValue::Vec2(cfa4_geom::Vec2::new(1000.0, 1000.0))));
        e.params.insert("position".into(), Param::new(ParamValue::Vec2(cfa4_geom::Vec2::new(0.0, 0.0))));
        // the bar goes behind the text
        it.effects.swap(text, shape);
        Ok(())
    })
    .unwrap();
    let r = s.execute("graphics.checkLegibility", json!({})).unwrap();
    assert_eq!((r["ok"].clone(), r["checked"].clone()), (json!(true), json!(1)), "{r}");
}

#[test]
fn small_and_unsafe_text_are_reported() {
    let (mut s, _) = titled("#000000", json!({"size": 3, "position": [4, 20]}));
    let r = s.execute("graphics.checkLegibility", json!({"markers": false})).unwrap();
    let k = kinds(&r);
    assert!(k.contains(&"smallText".to_string()) && k.contains(&"titleSafe".to_string()), "{r}");
    assert!(!s.active_sequence().unwrap().markers.iter().any(|m| m.comment.starts_with(CHECK_COMMENT)), "markers: false");
    let r = s.execute("graphics.checkLegibility", json!({"titleSafe": false, "minSize": 0})).unwrap();
    assert_eq!(r["ok"], true, "{r}");
}

#[test]
fn clear_removes_only_legibility_markers() {
    let (mut s, _) = titled("#ffffff", json!({}));
    s.execute("markers.add", json!({"name": "Mine"})).unwrap();
    assert!(s.execute("markers.clearLegibility", json!({})).is_err(), "nothing to clear yet");
    s.execute("graphics.checkLegibility", json!({})).unwrap();
    let r = s.execute("markers.clearLegibility", json!({})).unwrap();
    assert_eq!(r["removed"], 1);
    let q = s.active_sequence().unwrap();
    assert_eq!(q.markers.len(), 1);
    assert_eq!(q.markers[0].name, "Mine");
}

#[test]
fn hostile_parameters_are_refused() {
    let (mut s, _) = titled("#ffffff", json!({}));
    for p in [
        json!({"minContrast": 0}),
        json!({"minContrast": 99}),
        json!({"largeContrast": -1}),
        json!({"minSize": 1000}),
        json!({"minSize": -1}),
        json!({"sequence": 999_999}),
    ] {
        assert!(s.execute("graphics.checkLegibility", p.clone()).is_err(), "{p}");
    }
    // a sequence without graphics is fine
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "Empty", "fps": 24, "width": 64, "height": 36, "video": 1, "audio": 1})).unwrap();
    let r = s.execute("graphics.checkLegibility", json!({})).unwrap();
    assert_eq!((r["ok"].clone(), r["checked"].clone()), (json!(true), json!(0)));
}
