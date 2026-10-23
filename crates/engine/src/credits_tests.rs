use serde_json::{Value, json};

use crate::Session;
use crate::media_test_util::tmp_dir;
use filmcraft_project::graphic::LayerContent;
use filmcraft_project::{ClipId, RollMode, TrackItem};
use filmcraft_render::graphic_clip::item_layer_specs;
use filmcraft_time::Tick;

const SHEET: &str = "Head,Body,Tail,Content Style,Page Style,Page Runtime\n\
                     ,THE LONG WAY HOME,,Heading,Card,4s\n\
                     Directed by,Jordan Lee,,Centered,Scroll,\n\
                     ,,,,,\n\
                     Cast,Ann Smith,as Ann,Gutter,,\n\
                     ,Bob Jones,,,,\n";

fn demo(dir: &str) -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s.prefs_path = Some(tmp_dir(dir).join("prefs.json"));
    s.execute("playhead.set", json!({"seconds": 600})).unwrap();
    s
}

fn item(s: &Session, clip: ClipId) -> TrackItem {
    s.active_sequence().unwrap().find_item(clip).unwrap().1.clone()
}

fn texts(s: &Session, clip: ClipId) -> Vec<String> {
    let q = s.active_sequence().unwrap();
    let canvas = (q.settings.width, q.settings.height);
    let it = item(s, clip);
    item_layer_specs(&it, it.source_in, canvas)
        .into_iter()
        .filter_map(|(_, sp)| match sp.content {
            LayerContent::Text(t) => Some(t.text),
            _ => None,
        })
        .collect()
}

fn clips(v: &Value) -> Vec<ClipId> {
    v["clips"].as_array().unwrap().iter().map(|c| ClipId(c["clip"].as_u64().unwrap())).collect()
}

#[test]
fn import_places_a_card_then_a_roll_in_one_undo_step() {
    let mut s = demo("credits-import");
    let undo_before = s.history.undo.len();
    let v = s.execute("graphics.credits.import", json!({"csv": SHEET})).unwrap();
    assert_eq!(v["warnings"].as_array().unwrap().len(), 0, "{v}");
    let c = clips(&v);
    assert_eq!(c.len(), 2);
    assert_eq!(s.history.undo.len(), undo_before + 1);

    let card = item(&s, c[0]);
    assert_eq!(card.graphic.as_ref().map(|g| g.roll.mode).unwrap_or_default(), RollMode::Off);
    assert_eq!(texts(&s, c[0]), vec!["THE LONG WAY HOME"]);
    let start = Tick(v["clips"][0]["start"].as_i64().unwrap());
    let dur = Tick(v["clips"][0]["duration"].as_i64().unwrap());
    assert_eq!(dur, s.sequence_rate().snap_nearest(Tick::from_seconds_f64(4.0)));

    let roll = item(&s, c[1]);
    assert_eq!(roll.graphic.as_ref().unwrap().roll.mode, RollMode::Roll);
    assert_eq!(Tick(v["clips"][1]["start"].as_i64().unwrap()), start + dur, "pages follow each other");
    let t = texts(&s, c[1]);
    assert!(t.contains(&"Directed by".to_string()) && t.contains(&"Ann Smith\nBob Jones".to_string()) && t.contains(&"as Ann".to_string()), "{t:?}");

    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active_sequence().unwrap().find_item(c[0]).is_none());
    assert!(s.active_sequence().unwrap().find_item(c[1]).is_none());
}

#[test]
fn preview_reports_without_changing_the_project() {
    let mut s = demo("credits-preview");
    let undo_before = s.history.undo.len();
    let v = s.execute("graphics.credits.preview", json!({"csv": SHEET})).unwrap();
    assert_eq!(v["pages"].as_array().unwrap().len(), 2);
    assert_eq!(v["pages"][0]["behaviour"], "card");
    assert_eq!(v["pages"][1]["behaviour"], "scroll");
    assert!(v["pages"][1]["contentHeight"].as_f64().unwrap() > 100.0);
    assert_eq!(s.history.undo.len(), undo_before);
}

#[test]
fn reads_files_and_styling_and_reports_errors() {
    let mut s = demo("credits-files");
    let dir = tmp_dir("credits-files");
    let csv = dir.join("credits.tsv");
    std::fs::write(&csv, "Body\tStyle\nNames\tBig\n").unwrap();
    let sty = dir.join("styling.json");
    std::fs::write(&sty, r#"{"contentStyles":[{"name":"Big","body":{"size":90}}]}"#).unwrap();
    let v = s.execute("graphics.credits.import", json!({"path": csv, "styling": sty})).unwrap();
    assert_eq!(v["warnings"].as_array().unwrap().len(), 0, "{v}");

    for bad in [
        json!({}),
        json!({"path": dir.join("missing.csv")}),
        json!({"path": "credits.xlsx"}),
        json!({"csv": "no header here"}),
        json!({"csv": "Body\nA\n", "styling": "{broken"}),
        json!({"csv": "Body\nA\n", "styling": 7}),
    ] {
        assert!(s.execute("graphics.credits.import", bad.clone()).is_err(), "{bad}");
    }
}

#[test]
fn styles_lists_builtins_and_a_starter() {
    let mut s = Session::default();
    let v = s.execute("graphics.credits.styles", json!({})).unwrap();
    let names: Vec<&str> = v["contentStyles"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(names.contains(&"Gutter") && names.contains(&"Grid"));
    let starter: filmcraft_credits::Styling = serde_json::from_value(v["starter"].clone()).unwrap();
    assert!(starter.content_styles.len() >= 5);
}
