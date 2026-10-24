use serde_json::{Value, json};

use crate::Session;
use crate::media_test_util::tmp_dir;
use cfa4_project::graphic::LayerContent;
use cfa4_project::{ClipId, ParamValue, RollMode, TrackItem};
use cfa4_render::graphic_clip::item_layer_specs;
use cfa4_time::Tick;

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
        json!({"path": "credits.numbers"}),
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
    let starter: cfa4_credits::Styling = serde_json::from_value(v["starter"].clone()).unwrap();
    assert!(starter.content_styles.len() >= 5);
}

#[test]
fn pictures_become_clips_that_roll_with_the_page() {
    let mut s = demo("credits-pictures");
    let dir = tmp_dir("credits-pictures");
    image::RgbaImage::from_pixel(400, 200, image::Rgba([200, 30, 30, 255])).save(dir.join("logo.png")).unwrap();
    let csv = "Head,Body,Page Style\nSupported by,{{Pic logo.png 100}},Scroll\n,Names,\n,{{Pic nope.png}},\nTHE END,{{Logo logo.png}},Card\n";
    let v = s.execute("graphics.credits.import", json!({"csv": csv, "pictureDir": dir})).unwrap();
    assert_eq!(v["warnings"].as_array().unwrap().len(), 1, "{v}");
    let roll_page = ClipId(v["clips"][0]["clip"].as_u64().unwrap());
    let pic = ClipId(v["clips"][0]["pictures"][0].as_u64().unwrap());
    let card_pic = ClipId(v["clips"][1]["pictures"][0].as_u64().unwrap());

    // the picture sits on a track above its page and lasts as long
    let q = s.active_sequence().unwrap();
    let track_of = |c: ClipId| q.video_tracks.iter().position(|t| t.items.iter().any(|i| i.id == c)).unwrap();
    assert!(track_of(pic) > track_of(roll_page));
    let (page, p) = (item(&s, roll_page), item(&s, pic));
    assert_eq!(p.start, page.start);
    assert_eq!(p.duration, page.duration);

    // 100 px high: a 400×200 picture is scaled to 50 %
    let motion = p.effects.iter().find(|e| e.effect == "motion").unwrap();
    assert_eq!(motion.params["scale"].value, ParamValue::Float(50.0));

    // at any time the picture's centre is where the page's placeholder layer has rolled to
    let canvas = (q.settings.width, q.settings.height);
    for k in [0i64, 1, 5, 9, 10] {
        let u = Tick(page.duration.0 * k / 10);
        let specs = item_layer_specs(&page, page.source_in + u, canvas);
        let holder = specs.iter().find(|(_, sp)| matches!(sp.content, LayerContent::Shape(_))).unwrap();
        let ParamValue::Vec2(at) = motion.params["position"].value_at(u) else { panic!() };
        let want = holder.1.transform.position;
        assert!((at.x - want.x).abs() < 1e-6 && (at.y - want.y).abs() < 1e-6, "t={k}/10: {at:?} vs {want:?}");
    }

    // the card's picture fades with the card
    let c = item(&s, card_pic);
    let op = c.effects.iter().find(|e| e.effect == "opacity").unwrap();
    assert_eq!(op.params["opacity"].keyframes.len(), 4);
    assert!(!c.effects.iter().find(|e| e.effect == "motion").unwrap().params["position"].is_animated());

    // one undo removes pages and pictures
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active_sequence().unwrap().find_item(pic).is_none());
}
