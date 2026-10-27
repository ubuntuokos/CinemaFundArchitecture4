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

#[test]
fn reports_the_panel_shape_kind_linked_audio_fixed_and_selected_effects() {
    let mut s = demo();
    let q = s.active_sequence().unwrap();
    let (video, audio) = q.video_tracks[0]
        .items
        .iter()
        .find_map(|v| {
            let link = v.link?;
            q.audio_tracks.iter().find_map(|t| t.items.iter().find(|a| a.link == Some(link))).map(|a| (v.clone(), a.clone()))
        })
        .expect("the demo has a linked A/V pair");
    // the audio is selected first, but the video one is shown, with its audio under it
    s.execute("timeline.select", json!({"clips": [audio.id.0, video.id.0]})).unwrap();
    let v = s.execute("effects.inspect", json!({})).unwrap();
    assert_eq!(v["clip"], json!(video.id.0));
    assert_eq!(v["kind"], "video");
    assert_eq!(v["linkedAudio"], json!(audio.id.0));
    assert!(v["frame"].as_i64().unwrap() > 0);
    assert!(!v["sequenceName"].as_str().unwrap().is_empty());
    let motion = v["effects"].as_array().unwrap().iter().find(|e| e["effect"] == "motion").unwrap();
    assert_eq!(motion["intrinsic"], json!(true));
    assert_eq!(motion["selected"], json!(false));
    s.execute("effects.select", json!({"clip": video.id.0, "effect": "motion"})).unwrap();
    let v = s.execute("effects.inspect", json!({})).unwrap();
    let motion = v["effects"].as_array().unwrap().iter().find(|e| e["effect"] == "motion").unwrap();
    assert_eq!(motion["selected"], json!(true));
    let a = s.execute("effects.inspect", json!({"clip": audio.id.0})).unwrap();
    assert_eq!((a["kind"].as_str(), a["linkedAudio"].is_null()), (Some("audio"), true));
    // only the video clip, linked selection off: no audio under it
    s.state.linked_selection = false;
    s.execute("timeline.select", json!({"clips": [video.id.0]})).unwrap();
    assert!(s.execute("effects.inspect", json!({})).unwrap()["linkedAudio"].is_null());
}

#[test]
fn graphs_an_animated_parameter_across_its_clip() {
    let mut s = demo();
    let it = v1_clip(&s);
    let c = it.id.0;
    for (t, val) in [(it.start, 50.0), (it.end() - Tick(1), 150.0)] {
        s.execute("effects.setParam", json!({"clip": c, "effect": "motion", "param": "scale", "value": val, "keyframe": true, "time": t.0})).unwrap();
    }
    let g = s.execute("effects.graph", json!({"clip": c, "effect": "motion", "param": "scale", "samples": 10})).unwrap();
    let v = g["values"].as_array().unwrap();
    assert_eq!(v.len(), 11);
    assert!((v[0][0].as_f64().unwrap() - 50.0).abs() < 0.5);
    assert!(v[10][0].as_f64().unwrap() > 140.0);
    assert!(g["seconds"].as_f64().unwrap() > 0.0);
    // a point draws two curves; damaged requests are errors or clamped, never a panic
    let g = s.execute("effects.graph", json!({"clip": c, "effect": "motion", "param": "position", "samples": 1})).unwrap();
    assert_eq!(g["values"].as_array().unwrap().len(), 9);
    assert_eq!(g["values"][0].as_array().unwrap().len(), 2);
    assert!(s.execute("effects.graph", json!({"clip": c, "effect": 999, "param": "scale"})).is_err());
    assert!(s.execute("effects.graph", json!({"clip": c, "effect": "motion", "param": "nope"})).is_err());
    assert!(s.execute("effects.graph", json!({"clip": c, "effect": "motion", "param": "scale", "mask": 7})).is_err());
    assert_eq!(
        s.execute("effects.graph", json!({"clip": c, "effect": "motion", "param": "scale", "samples": u64::MAX})).unwrap()["values"].as_array().unwrap().len(),
        2001
    );
    // a reversed clip: the curve runs in timeline order, like its keyframes (150 first)
    s.execute("timeline.select", json!({"clips": [c]})).unwrap();
    s.execute("clip.speedDuration", json!({"speed": 100.0, "reverse": true})).unwrap();
    assert!(v1_clip(&s).reverse);
    let g = s.execute("effects.graph", json!({"clip": c, "effect": "motion", "param": "scale", "samples": 10})).unwrap();
    assert!(g["values"][0][0].as_f64().unwrap() > 140.0 && (g["values"][10][0].as_f64().unwrap() - 50.0).abs() < 0.5, "{g}");
}

#[test]
fn program_overlay_reports_the_transform_box_and_the_selected_mask() {
    let mut s = demo();
    let it = v1_clip(&s);
    let c = it.id.0;
    let mid = Tick(it.start.0 + it.duration.0 / 2);
    s.execute("playhead.set", json!({"time": mid.0})).unwrap();
    // nothing selected: the pictures at the playhead only
    s.execute("edit.deselectAll", json!({})).unwrap();
    let v = s.execute("program.overlay", json!({})).unwrap();
    assert!(v["transform"].is_null() && v["mask"].is_null());
    assert!(v["clips"].as_array().unwrap().iter().any(|q| q["clip"] == json!(c)));
    assert_eq!(v["frame"].as_array().unwrap().len(), 2);
    // Motion selected on a selected clip: its box, with the values a drag starts from
    s.execute("timeline.select", json!({"clips": [c]})).unwrap();
    s.execute("effects.select", json!({"clip": c, "effect": "motion"})).unwrap();
    let v = s.execute("program.overlay", json!({"clip": c})).unwrap();
    let t = &v["transform"];
    assert_eq!((t["clip"].as_u64(), t["effectId"].as_str()), (Some(c), Some("motion")));
    assert_eq!(t["quad"].as_array().unwrap().len(), 4);
    assert_eq!(t["toFrame"].as_array().unwrap().len(), 6);
    assert_eq!(t["height"], json!(100.0));
    assert!(v["clipToFrame"].is_array());
    // a polygon mask selected: contour, vertices, handles; the rounded preview on request
    let opacity = it.effects.iter().position(|e| e.effect == "opacity").unwrap();
    s.execute("masks.add", json!({"clip": c, "effect": opacity, "shape": "polygon"})).unwrap();
    s.execute("masks.set", json!({"clip": c, "effect": opacity, "mask": 0, "expansion": 12.0})).unwrap();
    s.execute("masks.select", json!({"clip": c, "effect": opacity, "mask": 0})).unwrap();
    let v = s.execute("program.overlay", json!({"roundRadius": 20.0})).unwrap();
    let m = &v["mask"];
    assert_eq!(m["vertices"].as_array().unwrap().len(), 4);
    assert!(m["contour"].as_array().unwrap().len() >= 4);
    assert!(m["expansionContour"].is_array() && m["featherContour"].is_array());
    assert!(m["featherHandle"]["normal"].is_array() && m["expansionHandle"]["dist"] == json!(12.0));
    assert!(m["roundness"]["maxRadius"].as_f64().unwrap() > 0.0);
    assert!(m["roundedContour"].as_array().unwrap().len() > 4);
    // off the clip, or the clip deselected: no mask handles
    s.execute("playhead.set", json!({"time": it.end().0 + 1})).unwrap();
    assert!(s.execute("program.overlay", json!({})).unwrap()["mask"].is_null());
    assert!(s.execute("program.overlay", json!({"clip": 987_654_321u64})).is_err());
    assert!(s.execute("program.overlay", json!({"roundRadius": f64::MAX})).is_ok());
}

#[test]
fn inspects_a_transition_and_drags_it_by_an_end_or_its_middle() {
    let mut s = demo();
    assert!(s.execute("transitions.inspect", json!({})).is_err(), "none selected");
    // the dip to black between shots 3 and 4
    s.execute("timeline.select", json!({"transitions": [900_003]})).unwrap();
    let v = s.execute("transitions.inspect", json!({})).unwrap();
    assert_eq!(v["transition"], 900_003);
    assert_eq!(v["effect"], "dip_to_black");
    assert_eq!(v["kind"], "video");
    assert_eq!(v["align"], "center");
    assert_eq!(v["oneSided"], false);
    assert!(v["a"]["clip"].is_u64() && v["b"]["clip"].is_u64() && v["a"]["name"].is_string());
    assert!(v["maxFrames"].as_i64().unwrap() > v["frames"].as_i64().unwrap());
    assert!(v["params"].as_array().is_some(), "{v}");
    let (start, duration, frame, cut) =
        (v["start"].as_i64().unwrap(), v["duration"].as_i64().unwrap(), v["frame"].as_i64().unwrap(), v["cut"].as_i64().unwrap());

    // its out end, dragged 3 frames on: the in end stays, one undo step for the whole drag
    let undo = s.history.undo.len();
    let drag = |delta: i64, begin: bool| json!({"transition": 900_003, "drag": {"edge": "out", "delta": delta, "start": start, "duration": duration}, "merge": true, "begin": begin});
    s.execute("sequence.setTransition", drag(frame, true)).unwrap();
    let r = s.execute("sequence.setTransition", drag(3 * frame, false)).unwrap();
    assert_eq!((r["start"].as_i64(), r["duration"].as_i64()), (Some(start), Some(duration + 3 * frame)));
    assert_eq!(r["align"], "custom");
    assert_eq!(s.history.undo.len(), undo + 1);
    // the middle slides it, never off the cut; a hostile delta is clamped
    let r = s
        .execute("sequence.setTransition", json!({"transition": 900_003, "drag": {"edge": null, "delta": i64::MAX, "start": start, "duration": duration}}))
        .unwrap();
    assert_eq!(r["start"].as_i64(), Some(cut), "slid as far as it goes: it starts at the cut");
    assert!(
        s.execute("sequence.setTransition", json!({"transition": 900_003, "drag": {"edge": "sideways", "delta": 0, "start": start, "duration": duration}}))
            .is_err()
    );
    assert!(s.execute("sequence.setTransition", json!({"transition": 900_003, "drag": {"delta": 0, "start": i64::MIN, "duration": duration}})).is_err());
    assert!(
        s.execute("sequence.setTransition", json!({"transition": 900_003, "frames": 4, "drag": {"delta": 0, "start": start, "duration": duration}})).is_err()
    );
    // a typed timecode duration (Set Transition Duration)
    let r = s.execute("sequence.setTransition", json!({"transition": 900_003, "timecode": "00:00:00:10"})).unwrap();
    assert_eq!(r["duration"].as_i64(), Some(10 * frame));
    assert!(s.execute("sequence.setTransition", json!({"transition": 900_003, "timecode": "soon"})).is_err());
    // Deselect All (a click on empty Timeline space) lets go of it too
    s.execute("edit.deselectAll", json!({})).unwrap();
    assert!(s.state.transition_selection.is_empty());
}

#[test]
fn the_eyedropper_fills_a_colour_parameter_from_the_program_picture() {
    let mut s = demo();
    let it = v1_clip(&s);
    s.execute("playhead.set", json!({"time": (it.start + Tick(it.duration.0 / 2)).0})).unwrap();
    s.execute("effects.apply", json!({"clips": [it.id.0], "effect": "ultra_key"})).unwrap();
    let index = v1_clip(&s).effects.iter().position(|e| e.effect == "ultra_key").unwrap();
    let undo = s.history.undo.len();
    let r = s.execute("effects.eyedropper", json!({"clip": it.id.0, "effect": index, "param": "key_color", "u": 0.5, "v": 0.5})).unwrap();
    let rgb: Vec<f64> = r["color"].as_array().unwrap().iter().map(|c| c.as_f64().unwrap()).collect();
    assert!(rgb.iter().all(|c| (0.0..=1.0).contains(c)), "{r}");
    let v = s.execute("effects.inspect", json!({"clip": it.id.0})).unwrap();
    let set = param(&v, "ultra_key", "key_color")["value"].clone();
    assert_eq!(set.as_array().map(Vec::len), Some(4));
    assert!((set[0].as_f64().unwrap() - rgb[0]).abs() < 1e-6 && set[3] == 1.0, "{set} vs {r}");
    assert_eq!(s.history.undo.len(), undo + 1, "one undo step");
    // hostile or wrong arguments are refused
    for bad in [
        json!({"clip": it.id.0, "effect": index, "param": "transparency", "u": 0.5, "v": 0.5}),
        json!({"clip": it.id.0, "effect": 999, "param": "key_color", "u": 0.5, "v": 0.5}),
        json!({"clip": it.id.0, "effect": index, "param": "key_color", "u": f64::NAN, "v": 0.5}),
        json!({"clip": it.id.0, "effect": index, "param": "key_color", "u": 0.5}),
    ] {
        assert!(s.execute("effects.eyedropper", bad.clone()).is_err(), "{bad}");
    }
    // far outside the picture is its edge
    assert!(s.execute("effects.eyedropper", json!({"clip": it.id.0, "effect": index, "param": "key_color", "u": 1e300, "v": -5.0})).is_ok());
}

#[test]
fn program_overlay_follows_effects_that_animate_through_a_frame_hold() {
    // Hold Filters off: the picture holds, its Motion keeps animating (as rendered), so the box
    // follows the effect time, not the held media time
    let mut s = demo();
    let it = v1_clip(&s);
    let c = it.id.0;
    for (t, val) in [(it.start, 50.0), (it.end() - Tick(1), 150.0)] {
        s.execute("effects.setParam", json!({"clip": c, "effect": "motion", "param": "scale", "value": val, "keyframe": true, "time": t.0})).unwrap();
    }
    s.execute("timeline.select", json!({"clips": [c]})).unwrap();
    s.execute("clip.frameHoldOptions", json!({"clips": [c], "holdOn": "in", "holdFilters": false})).unwrap();
    assert!(v1_clip(&s).frame_hold.is_some());
    s.execute("effects.select", json!({"clip": c, "effect": "motion"})).unwrap();
    let width = |s: &mut Session, t: Tick| {
        s.execute("playhead.set", json!({"time": t.0})).unwrap();
        let v = s.execute("program.overlay", json!({})).unwrap();
        let q = &v["transform"]["quad"];
        q[1][0].as_f64().unwrap() - q[0][0].as_f64().unwrap()
    };
    let (early, late) = (width(&mut s, it.start + Tick(it.duration.0 / 10)), width(&mut s, it.end() - Tick(it.duration.0 / 10)));
    assert!(late > early * 1.5, "{early} → {late}");
}

#[test]
fn inspect_reports_where_automatic_points_are() {
    let mut s = demo();
    let it = v1_clip(&s);
    s.execute("effects.apply", json!({"clips": [it.id.0], "effect": "corner_pin"})).unwrap();
    let v = s.execute("effects.inspect", json!({"clip": it.id.0})).unwrap();
    let ur = param(&v, "corner_pin", "upper_right");
    let auto = ur["auto"].as_array().unwrap();
    assert!(auto[0].as_f64().unwrap() > 100.0 && auto[1].as_f64().unwrap() == 0.0, "{ur}");
    let (w, h) = (s.active_sequence().unwrap().settings.width, s.active_sequence().unwrap().settings.height);
    assert_eq!(param(&v, "motion", "position")["auto"], json!([f64::from(w) / 2.0, f64::from(h) / 2.0]));
    assert!(param(&v, "motion", "scale").get("auto").is_none(), "points only");
}

#[test]
fn program_overlay_reports_the_graphic_layers_at_the_playhead() {
    // the Qt Program monitor's graphics tools: each layer's box and matrices, the selection and
    // where a new layer goes
    let mut s = demo();
    s.execute("playhead.set", json!({"time": 0})).unwrap();
    let g = &s.execute("program.overlay", json!({})).unwrap()["graphics"];
    assert_eq!(g["canvasTarget"]["clip"], json!(null));
    assert_eq!(g["canvasTarget"]["toFrame"], json!([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]));
    let made = s.execute("graphics.newShape", json!({"shape": "rectangle", "position": [960, 540], "size": [400, 200], "time": 0})).unwrap();
    let clip = made["clip"].as_u64().unwrap();
    s.execute("graphics.newText", json!({"clip": clip, "text": "Hi", "position": [100, 200]})).unwrap();
    s.execute("graphics.selectLayer", json!({"clip": clip, "layers": [1]})).unwrap();
    let g = s.execute("program.overlay", json!({})).unwrap()["graphics"].clone();
    let layers = g["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 2);
    // front-most first: the text added last
    assert_eq!((layers[0]["layer"].as_u64(), layers[0]["text"]["text"].as_str()), (Some(1), Some("Hi")));
    assert_eq!(layers[0]["text"]["paragraph"], json!(false));
    // its caret stops, in characters: before H, between, after i
    let line = &layers[0]["text"]["lines"][0];
    assert_eq!((line["start"].as_u64(), line["end"].as_u64()), (Some(0), Some(2)));
    let stops: Vec<u64> = line["carets"].as_array().unwrap().iter().map(|c| c[0].as_u64().unwrap()).collect();
    assert_eq!(stops, vec![0, 1, 2]);
    assert_eq!(layers[1]["shape"]["shape"], json!(0));
    assert_eq!(layers[1]["shape"]["size"], json!([400.0, 200.0]));
    assert_eq!(layers[1]["position"], json!([960.0, 540.0]));
    assert_eq!(layers[1]["toFrame"].as_array().unwrap().len(), 6);
    assert_eq!(layers[1]["local"].as_array().unwrap().len(), 4);
    assert_eq!((g["clip"].as_u64(), g["selectedLayers"].clone()), (Some(clip), json!([1])));
    assert_eq!(g["canvasTarget"]["clip"].as_u64(), Some(clip));
}
