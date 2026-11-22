use serde_json::json;

use crate::Session;
use cfa4_time::Tick;

fn demo() -> (Session, u64) {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let it = s.active_sequence().unwrap().video_tracks[0].items[0].clone();
    s.set_playhead(it.start + Tick(it.duration.0 / 2));
    s.state.selection = vec![it.id];
    (s, it.id.0)
}

#[test]
fn create_list_rename_remove_and_undo() {
    let (mut s, _) = demo();
    let r = s.execute("objects.createFromBox", json!({"box": [100, 200, 300, 150], "label": "Car"})).unwrap();
    let id = r["id"].as_u64().unwrap();
    let l = s.execute("objects.list", json!({})).unwrap();
    assert_eq!(l["objects"].as_array().unwrap().len(), 1);
    assert_eq!(l["objects"][0]["label"], "Car");
    assert_eq!(l["objects"][0]["samples"], 1);
    assert_eq!(l["objects"][0]["source"], "manual");
    s.execute("objects.rename", json!({"id": id, "label": "Truck"})).unwrap();
    assert_eq!(s.execute("objects.list", json!({})).unwrap()["objects"][0]["label"], "Truck");
    s.execute("objects.remove", json!({"id": id})).unwrap();
    assert!(s.project.objects.is_empty());
    s.undo();
    assert_eq!(s.project.objects.values().flatten().count(), 1);
    s.undo();
    s.undo();
    assert!(s.project.objects.is_empty());
}

#[test]
fn default_label_and_ids_are_distinct() {
    let (mut s, _) = demo();
    let a = s.execute("objects.createFromBox", json!({"box": {"x": 1, "y": 2, "w": 3, "h": 4}})).unwrap();
    let b = s.execute("objects.createFromBox", json!({"box": [1, 2, 3, 4], "label": "  "})).unwrap();
    assert_ne!(a["id"], b["id"]);
    let l = s.execute("objects.list", json!({})).unwrap();
    assert_eq!(l["objects"][0]["label"], "Object 1");
    assert_eq!(l["objects"][1]["label"], "Object 2");
}

#[test]
fn hostile_parameters_are_refused_and_change_nothing() {
    let (mut s, _) = demo();
    let before = s.project.clone();
    let bad_boxes = [
        json!(null),
        json!("x"),
        json!([1, 2, 3]),
        json!([1, 2, 3, 4, 5]),
        json!([0, 0, 0, 10]),
        json!([0, 0, -5, 10]),
        json!([0, 0, 10, 0]),
        json!([1e300, 0, 10, 10]),
        json!([0, 0, 1e9, 10]),
        json!(["a", 0, 1, 1]),
        json!({"x": 0, "y": 0, "w": 1}),
    ];
    for b in bad_boxes {
        assert!(s.execute("objects.createFromBox", json!({"box": b})).is_err(), "{b}");
    }
    assert!(s.execute("objects.createFromBox", json!({})).is_err());
    assert!(s.execute("objects.createFromBox", json!({"box": [0, 0, 1, 1], "item": 999999})).is_err());
    assert!(s.execute("objects.createFromBox", json!({"box": [0, 0, 1, 1], "clip": 999999})).is_err());
    assert!(s.execute("objects.createFromBox", json!({"box": [0, 0, 1, 1], "mediaTime": "soon"})).is_err());
    for c in ["objects.rename", "objects.remove"] {
        assert!(s.execute(c, json!({})).is_err());
        assert!(s.execute(c, json!({"id": 424242, "label": "x"})).is_err());
        assert!(s.execute(c, json!({"id": -1, "label": "x"})).is_err());
    }
    assert!(s.project.objects.is_empty());
    assert!(std::sync::Arc::ptr_eq(&before, &s.project) || *before == *s.project);
}

#[test]
fn long_labels_are_cut_on_a_character_boundary() {
    let (mut s, _) = demo();
    let label = "é".repeat(1000);
    let r = s.execute("objects.createFromBox", json!({"box": [0, 0, 5, 5], "label": label})).unwrap();
    let l = s.execute("objects.list", json!({})).unwrap();
    assert!(l["objects"][0]["label"].as_str().unwrap().len() <= cfa4_project::object::MAX_TEXT);
    s.execute("objects.rename", json!({"id": r["id"], "label": "ü".repeat(999)})).unwrap();
    assert!(s.project.objects.values().flatten().all(|t| t.validate().is_ok()));
}

#[test]
fn media_time_follows_the_playhead_or_the_parameter() {
    let (mut s, _) = demo();
    s.execute("objects.createFromBox", json!({"box": [0, 0, 5, 5], "mediaTime": 12345})).unwrap();
    let t = s.project.objects.values().flatten().next().unwrap();
    assert_eq!(t.samples()[0].time, Tick(12345));
    s.execute("objects.createFromBox", json!({"box": [0, 0, 5, 5]})).unwrap();
    let t2 = s.project.objects.values().flatten().nth(1).unwrap();
    assert_ne!(t2.samples()[0].time, Tick(12345));
}
