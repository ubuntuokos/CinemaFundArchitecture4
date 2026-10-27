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

#[test]
fn thumbnail_renders_a_page_without_changing_the_project() {
    let mut s = demo("credits-thumb");
    let undo = s.history.undo.len();
    let v = s.execute("graphics.credits.thumbnail", json!({"csv": SHEET, "page": 0, "width": 320})).unwrap();
    assert_eq!((v["width"].as_u64(), v["height"].as_u64(), v["pages"].as_u64()), (Some(320), Some(180), Some(2)));
    let png = cfa4_project::gtemplate::base64_decode(v["pngBase64"].as_str().unwrap()).unwrap();
    let img = image::load_from_memory(&png).unwrap().to_rgba8();
    // the title card shows white text in the middle
    assert!(img.pixels().any(|p| p[0] > 200 && p[3] > 200));
    assert_eq!(s.history.undo.len(), undo);
    assert!(s.execute("graphics.credits.thumbnail", json!({"csv": SHEET, "page": 9})).is_err());
}

// ----- linked credits: save, diff, refresh -----

fn group_pages(s: &Session, group: u64) -> Vec<TrackItem> {
    let q = s.active_sequence().unwrap();
    let mut v: Vec<TrackItem> = q
        .video_tracks
        .iter()
        .flat_map(|t| t.items.iter())
        .filter(|it| it.graphic.as_ref().and_then(|g| g.credits.as_ref()).is_some_and(|l| l.group == group))
        .cloned()
        .collect();
    v.sort_by_key(|it| it.start);
    v
}

/// Set the text of the layer whose text is `from` on clip `clip`, as a user would by hand.
fn hand_edit(s: &mut Session, clip: ClipId, from: &str, to: &str) {
    let (from, to) = (from.to_string(), to.to_string());
    s.edit_sequence("Edit Text", move |q, _, _| {
        let (_, it) = q.find_item_mut(clip).unwrap();
        let e = it.effects.iter_mut().find(|e| matches!(e.params.get("text").map(|p| &p.value), Some(ParamValue::Text(t)) if *t == from)).unwrap();
        e.params.insert("text".into(), cfa4_project::Param::new(ParamValue::Text(to)));
        Ok(())
    })
    .unwrap();
}

#[test]
fn linked_credits_diff_and_refresh_keep_hand_edits() {
    let mut s = demo("credits-linked");
    let dir = tmp_dir("credits-linked");
    let sheet = dir.join("credits.csv");
    std::fs::write(&sheet, SHEET).unwrap();
    let v = s.execute("graphics.credits.import", json!({"path": sheet})).unwrap();
    let group = v["group"].as_u64().unwrap();
    let roll = ClipId(v["clips"][1]["clip"].as_u64().unwrap());
    let link = item(&s, roll).graphic.unwrap().credits.unwrap();
    assert_eq!((link.group, link.page), (group, 1));
    assert!(!link.layers.is_empty() && link.doc.contains("cfa4.credits"));

    // nothing changed yet
    s.execute("timeline.select", json!({"clips": [roll.0]})).unwrap();
    let d = s.execute("graphics.credits.diff", json!({})).unwrap();
    assert_eq!(d["changes"].as_array().unwrap().len(), 0, "{d}");
    assert_eq!(d["manual"].as_array().unwrap().len(), 0);

    // a hand edit on the Cast block, then the sheet changes: the director is renamed, Bob leaves,
    // Cyd joins
    hand_edit(&mut s, roll, "Cast", "Szereplők");
    let new_sheet = SHEET.replace("Jordan Lee", "Jordan Lee-Park").replace(",Bob Jones,,,,\n", ",Cyd Park,,,,\n");
    std::fs::write(&sheet, &new_sheet).unwrap();
    let d = s.execute("graphics.credits.diff", json!({"clip": roll.0})).unwrap();
    let changes = d["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 2, "{d}");
    assert_eq!(changes[0]["kind"], "changed");
    assert_eq!(changes[0]["oldRows"], json!([3, 3]));
    assert_eq!(changes[0]["after"][0][1], "Jordan Lee-Park");
    let manual = d["manual"].as_array().unwrap();
    assert_eq!(manual.len(), 1, "{d}");
    assert_eq!(manual[0]["text"], "Szereplők");
    // the Cast block changes when Bob is replaced: the hand edit would not survive accepting all
    assert_eq!(manual[0]["survivesAll"], false);

    // accept only the director's new name: the Cast block stays, and so does the hand edit
    let undo = s.history.undo.len();
    let r = s.execute("graphics.credits.refresh", json!({"clip": roll.0, "accept": [0]})).unwrap();
    assert_eq!((r["accepted"].as_u64(), r["keptEdits"].as_u64(), r["lostEdits"].as_u64()), (Some(1), Some(1), Some(0)), "{r}");
    assert_eq!(s.history.undo.len(), undo + 1, "one undo step");
    let pages = group_pages(&s, r["clips"][0]["clip"].as_u64().unwrap());
    assert_eq!(pages.len(), 2);
    assert!(s.active_sequence().unwrap().find_item(roll).is_none(), "the old pages are replaced");
    let t = texts(&s, pages[1].id);
    assert!(t.contains(&"Jordan Lee-Park".to_string()) && t.contains(&"Szereplők".to_string()) && t.contains(&"Ann Smith\nBob Jones".to_string()), "{t:?}");
    // the same place on the timeline
    assert_eq!(pages[0].start.0, v["clips"][0]["start"].as_i64().unwrap());

    // the rejected change is still offered; the kept hand edit is still recognised as one
    let new_roll = pages[1].id;
    let d = s.execute("graphics.credits.diff", json!({"clip": new_roll.0})).unwrap();
    assert_eq!(d["changes"].as_array().unwrap().len(), 1, "{d}");
    assert_eq!(d["manual"].as_array().unwrap().len(), 1);

    // undo brings the earlier pages back
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active_sequence().unwrap().find_item(roll).is_some());
}

#[test]
fn credits_file_saves_reopens_without_the_sheet_and_reports_a_missing_source() {
    let mut s = demo("credits-file");
    let dir = tmp_dir("credits-file");
    let sheet = dir.join("credits.csv");
    std::fs::write(&sheet, SHEET).unwrap();
    // save straight from the spreadsheet, then from placed credits
    let a = dir.join("from-sheet.cfa4credits");
    s.execute("graphics.credits.save", json!({"to": a, "from": {"path": sheet}})).unwrap();
    let v = s.execute("graphics.credits.import", json!({"path": sheet})).unwrap();
    let page = v["clips"][0]["clip"].as_u64().unwrap();
    let b = dir.join("placed");
    let r = s.execute("graphics.credits.save", json!({"to": b, "clip": page})).unwrap();
    assert!(r["path"].as_str().unwrap().ends_with(".cfa4credits"));
    let text = std::fs::read_to_string(dir.join("placed.cfa4credits")).unwrap();
    assert!(text.contains("\"format\": \"cfa4.credits\"") && text.contains("Jordan Lee"));

    // the file opens on another machine: the table is embedded, the sheet is not needed
    std::fs::remove_file(&sheet).unwrap();
    let p = s.execute("graphics.credits.preview", json!({"path": a})).unwrap();
    assert_eq!(p["pages"].as_array().unwrap().len(), 2);
    let v2 = s.execute("graphics.credits.import", json!({"path": a, "time": 0})).unwrap();
    // but refreshing needs the spreadsheet: a clear error, and its new place can be given
    let page2 = v2["clips"][0]["clip"].as_u64().unwrap();
    let e = s.execute("graphics.credits.diff", json!({"clip": page2})).unwrap_err().to_string();
    assert!(e.contains("gone"), "{e}");
    let moved = dir.join("moved.csv");
    std::fs::write(&moved, SHEET.replace("Ann Smith", "Ann Smyth")).unwrap();
    let d = s.execute("graphics.credits.diff", json!({"clip": page2, "source": moved})).unwrap();
    assert_eq!(d["changes"].as_array().unwrap().len(), 1);

    // hostile documents are errors, not crashes
    for bad in [r#"{"format":"cfa4.credits","version":99}"#, r#"{"format":"cfa4.credits","table":"x"}"#, "{\"format\":\"cfa4.credits\""] {
        let f = dir.join("bad.cfa4credits");
        std::fs::write(&f, bad).unwrap();
        assert!(s.execute("graphics.credits.import", json!({"path": f})).is_err(), "{bad}");
    }
    // diff and refresh need a credits page
    assert!(s.execute("graphics.credits.diff", json!({"clip": 999_999})).is_err());
}

// ----- review fixes (C1) -----

fn layer_with_text(s: &Session, clip: ClipId, text: &str) -> cfa4_project::EffectInstance {
    item(s, clip).effects.into_iter().find(|e| matches!(e.params.get("text").map(|p| &p.value), Some(ParamValue::Text(t)) if t == text)).unwrap()
}

fn position(e: &cfa4_project::EffectInstance) -> (f64, f64) {
    match e.params.get("position").map(|p| &p.value) {
        Some(ParamValue::Vec2(v)) => (v.x, v.y),
        _ => (f64::NAN, f64::NAN),
    }
}

#[test]
fn a_full_undo_history_still_makes_import_and_refresh_one_step() {
    let mut s = demo("credits-undo-limit");
    let dir = tmp_dir("credits-undo-limit");
    let sheet = dir.join("credits.csv");
    std::fs::write(&sheet, SHEET).unwrap();
    // the history is at its limit: every new step pushes the oldest out
    s.history.limit = s.history.undo.len().max(1);
    let v = s.execute("graphics.credits.import", json!({"path": sheet})).unwrap();
    let first = clips(&v)[0];
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active_sequence().unwrap().find_item(first).is_none(), "one undo removes every page");
    s.execute("edit.redo", json!({})).unwrap();
    std::fs::write(&sheet, SHEET.replace("Jordan Lee", "Jordan Lee-Park")).unwrap();
    let r = s.execute("graphics.credits.refresh", json!({"clip": first.0})).unwrap();
    assert!(s.active_sequence().unwrap().find_item(first).is_none());
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active_sequence().unwrap().find_item(first).is_some(), "one undo brings the old pages back");
    assert!(s.active_sequence().unwrap().find_item(clips(&r)[0]).is_none());
    // a refresh that cannot read its spreadsheet changes nothing
    std::fs::remove_file(&sheet).unwrap();
    let n = s.active_sequence().unwrap().video_tracks.iter().map(|t| t.items.len()).sum::<usize>();
    assert!(s.execute("graphics.credits.refresh", json!({"clip": first.0})).is_err());
    assert_eq!(s.active_sequence().unwrap().video_tracks.iter().map(|t| t.items.len()).sum::<usize>(), n);
}

#[test]
fn a_hand_edit_moves_with_the_new_layout() {
    let mut s = demo("credits-rebase");
    let dir = tmp_dir("credits-rebase");
    let sheet = dir.join("credits.csv");
    std::fs::write(&sheet, SHEET).unwrap();
    let v = s.execute("graphics.credits.import", json!({"path": sheet})).unwrap();
    let roll = clips(&v)[1];
    hand_edit(&mut s, roll, "Cast", "Szereplők");
    let before = position(&layer_with_text(&s, roll, "Szereplők"));
    // a new block above the Cast block pushes it down
    std::fs::write(&sheet, SHEET.replace(",,,,,\n", ",,,,,\nProduced by,Pat Doe,,Centered,,\n,,,,,\n")).unwrap();
    let r = s.execute("graphics.credits.refresh", json!({"clip": roll.0})).unwrap();
    assert_eq!(r["keptEdits"].as_u64(), Some(1), "{r}");
    let new_roll = clips(&r)[1];
    let after = position(&layer_with_text(&s, new_roll, "Szereplők"));
    assert!(after.1 > before.1 + 10.0, "the edited head moved down with its block: {before:?} → {after:?}");
    // the names next to it moved by as much
    let names = position(&layer_with_text(&s, new_roll, "Ann Smith\nBob Jones"));
    let old_names = {
        let e = s.execute("edit.undo", json!({})).map(|_| layer_with_text(&s, roll, "Ann Smith\nBob Jones")).unwrap();
        position(&e)
    };
    assert!(((after.1 - before.1) - (names.1 - old_names.1)).abs() < 1e-6);
}

#[test]
fn repeated_blocks_and_pasted_copies_are_their_own() {
    let mut s = demo("credits-repeat");
    let dir = tmp_dir("credits-repeat");
    let sheet = dir.join("credits.csv");
    std::fs::write(&sheet, "Body,Page Style\nTHANK YOU,Card\nTHANK YOU,Card\n").unwrap();
    let v = s.execute("graphics.credits.import", json!({"path": sheet})).unwrap();
    let (a, b) = (clips(&v)[0], clips(&v)[1]);
    hand_edit(&mut s, a, "THANK YOU", "KÖSZÖNJÜK");
    let r = s.execute("graphics.credits.refresh", json!({"clip": a.0})).unwrap();
    let t: Vec<Vec<String>> = clips(&r).into_iter().map(|c| texts(&s, c)).collect();
    assert_eq!(t, vec![vec!["KÖSZÖNJÜK".to_string()], vec!["THANK YOU".to_string()]], "only the edited occurrence changed");
    let _ = b;

    // copy the pages and paste them later: refreshing the copy leaves the original alone
    let pages: Vec<u64> = clips(&r).iter().map(|c| c.0).collect();
    s.execute("timeline.select", json!({"clips": pages})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("playhead.set", json!({"seconds": 1200})).unwrap();
    s.execute("edit.paste", json!({})).unwrap();
    let pasted: Vec<ClipId> = s.state.selection.clone();
    assert_eq!(pasted.len(), 2);
    let link = |c: ClipId| item(&s, c).graphic.unwrap().credits.unwrap();
    assert_ne!(link(pasted[0]).group, link(clips(&r)[0]).group);
    assert_eq!(link(pasted[0]).group, link(pasted[1]).group);
    s.execute("graphics.credits.refresh", json!({"clip": pasted[0].0})).unwrap();
    for c in clips(&r) {
        assert!(s.active_sequence().unwrap().find_item(c).is_some(), "the original pages stay");
    }
}

#[test]
fn a_credits_file_elsewhere_keeps_its_picture_folder_and_big_sheets_save() {
    let mut s = demo("credits-docdir");
    let dir = tmp_dir("credits-docdir");
    let (sheets, docs) = (dir.join("sheets"), dir.join("docs"));
    std::fs::create_dir_all(&sheets).unwrap();
    std::fs::create_dir_all(&docs).unwrap();
    image::RgbaImage::from_pixel(40, 20, image::Rgba([1, 2, 3, 255])).save(docs.join("logo.png")).unwrap();
    let sheet = sheets.join("credits.csv");
    std::fs::write(&sheet, "Body\n{{Pic logo.png}}\nAnna\n").unwrap();
    let doc = docs.join("credits.cfa4credits");
    s.execute("graphics.credits.save", json!({"to": doc, "from": {"path": sheet}})).unwrap();
    let v = s.execute("graphics.credits.import", json!({"path": doc})).unwrap();
    assert_eq!(v["clips"][0]["pictures"].as_array().unwrap().len(), 1, "{v}");
    let r = s.execute("graphics.credits.refresh", json!({"clip": v["clips"][0]["clip"]})).unwrap();
    assert_eq!(r["clips"][0]["pictures"].as_array().unwrap().len(), 1, "the refresh finds the pictures too: {r}");

    // a sheet with many empty columns saves small enough to open again
    let mut wide = String::from("Body");
    wide.push_str(&",".repeat(250));
    wide.push('\n');
    for i in 0..2000 {
        wide.push_str(&format!("Name {i}{}\n", ",".repeat(250)));
    }
    std::fs::write(&sheet, wide).unwrap();
    let big = dir.join("big.cfa4credits");
    s.execute("graphics.credits.save", json!({"to": big, "from": {"path": sheet}})).unwrap();
    assert!(std::fs::metadata(&big).unwrap().len() < 1 << 20);
    assert!(s.execute("graphics.credits.preview", json!({"path": big})).is_ok());
}

#[test]
fn validate_reports_issues_for_sheets_and_placed_credits() {
    let mut s = demo("credits-validate");
    let sheet = "Head,Body,Content Style,Page Style\nCast,Kiss Anna,Gutter,Scroll\n,Kiss Ánna,,\n,{{Blank}},,\n,Bob,,\n";
    let v = s
        .execute("graphics.credits.validate", json!({"csv": sheet, "styling": {"contentStyles": [{"name": "Gutter", "body": {"font": "No Such Font"}}]}}))
        .unwrap();
    let codes: Vec<&str> = v["issues"].as_array().unwrap().iter().filter_map(|i| i["code"].as_str()).collect();
    assert!(codes.contains(&"missing-font") && codes.contains(&"name-spelling"), "{v}");
    assert!(v["errors"].as_u64().unwrap() >= 1);
    // the bundled font has Hungarian letters: no missing-glyphs for ő/ű in the default style
    let v = s.execute("graphics.credits.preview", json!({"csv": "Head,Body\nZeneszerző,Hegedűs Hanna\n"})).unwrap();
    let codes: Vec<&str> = v["issues"].as_array().unwrap().iter().filter_map(|i| i["code"].as_str()).collect();
    assert!(!codes.contains(&"missing-glyphs") && !codes.contains(&"missing-font"), "{v}");
    // placed credits: checked as accepted
    let placed = s.execute("graphics.credits.import", json!({"csv": SHEET})).unwrap();
    let page = placed["clips"][0]["clip"].as_u64().unwrap();
    let v = s.execute("graphics.credits.validate", json!({"clip": page})).unwrap();
    assert!(v["issues"].is_array(), "{v}");
    assert!(s.execute("graphics.credits.validate", json!({"clip": 999_999})).is_err());
}

#[test]
fn the_example_credits_pass_the_checks() {
    let mut s = demo("credits-validate-example");
    let v = s.execute("graphics.credits.validate", json!({"csv": include_str!("../../credits/examples/credits.csv")})).unwrap();
    let serious: Vec<&Value> = v["issues"].as_array().unwrap().iter().filter(|i| i["severity"] != "info").collect();
    assert!(serious.is_empty(), "{serious:#?}");
}

// ----- fit to music -----

/// A sequence with 32 s of generated 120 BPM music on A1 (the last 4 s silent), at 0 s.
fn with_music() -> (Session, ClipId) {
    use cfa4_audio_dsp::remix as dsp;
    const SR: u32 = 48_000;
    let (mut m, _) = dsp::test_music(SR, 120.0, 16, 3);
    // silence the last 4 seconds: the sound ends at 28 s
    for ch in &mut m {
        let n = ch.len();
        for v in ch.iter_mut().skip(n - 4 * SR as usize) {
            *v = 0.0;
        }
    }
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "credits", "audio": 1, "video": 2})).unwrap();
    let inter: Vec<f32> = (0..m[0].len()).flat_map(|i| [m[0][i], m[1][i]]).collect();
    let bytes: std::sync::Arc<[u8]> = crate::previews::write_wav_f32(&inter, SR).into();
    let item = crate::commands::import_bytes(&mut s, "/music.wav", bytes, None).unwrap();
    let r = s.execute("timeline.place", json!({"item": item.0, "audioTrack": "A1", "seconds": 0.0})).unwrap();
    (s, ClipId(r["clips"][0].as_u64().unwrap()))
}

fn total(s: &Session, group: u64) -> Tick {
    let p = group_pages(s, group);
    let (first, last) = (p.first().unwrap(), p.last().unwrap());
    Tick(last.start.0 + last.duration.0 - first.start.0)
}

#[test]
fn music_analysis_offers_lengths_and_changes_nothing() {
    let (mut s, music) = with_music();
    let undo = s.history.undo.len();
    let v = s.execute("graphics.credits.music", json!({"audio": music.0})).unwrap();
    let sec = |k: &str| v[k].as_i64().unwrap() as f64 / cfa4_time::TICKS_PER_SECOND as f64;
    assert!((sec("duration") - 32.0).abs() < 0.01, "{v}");
    assert!((sec("soundEnd") - 28.0).abs() < 0.1, "{v}");
    assert!((v["bpm"].as_f64().unwrap() - 120.0).abs() < 2.0, "{v}");
    let ids: Vec<&str> = v["suggestions"].as_array().unwrap().iter().filter_map(|x| x["id"].as_str()).collect();
    assert_eq!(&ids[..2], &["musicLength", "soundEnd"], "{v}");
    assert_eq!(s.history.undo.len(), undo, "analysis changes nothing");
}

#[test]
fn fit_to_music_in_place_keeps_cards_and_survives_a_refresh() {
    let (mut s, music) = with_music();
    let dir = tmp_dir("credits-fit");
    let sheet = dir.join("credits.csv");
    std::fs::write(&sheet, "Body,Page Style,Vertical Gap\nTITLE,Card,\nA,Scroll,\nB,,4\nC,,4\nD,,4\nEND,Card,\n").unwrap();
    s.execute("playhead.set", json!({"seconds": 3})).unwrap();
    let v = s.execute("graphics.credits.import", json!({"path": sheet})).unwrap();
    let group = v["group"].as_u64().unwrap();
    let card = Tick(v["clips"][0]["duration"].as_i64().unwrap());

    // fit to where the sound ends, with the credits moved to the start of the music
    let undo = s.history.undo.len();
    let r = s.execute("graphics.credits.fit", json!({"clip": group, "audio": music.0, "use": "soundEnd", "alignToAudio": true})).unwrap();
    assert_eq!(s.history.undo.len(), undo + 1, "one undo step");
    let g = r["clips"][0]["clip"].as_u64().unwrap();
    let want = r["report"]["target"].as_i64().unwrap();
    assert_eq!(total(&s, g).0, want, "exact to the frame: {r}");
    assert!((want as f64 / cfa4_time::TICKS_PER_SECOND as f64 - 28.0).abs() < 0.1);
    let pages = group_pages(&s, g);
    assert_eq!(pages[0].start, Tick::ZERO, "moved to the music's start");
    assert_eq!(pages[0].duration, card, "cards keep their length");

    // cards on the beat: whole beats of 0.5 s
    let r = s.execute("graphics.credits.fit", json!({"clip": g, "audio": music.0, "use": "musicLength", "cardsOnBeat": true})).unwrap();
    let g = r["clips"][0]["clip"].as_u64().unwrap();
    let pages = group_pages(&s, g);
    let beat = r["fit"]["cardBeat"].as_i64().unwrap() as f64 / cfa4_time::TICKS_PER_SECOND as f64;
    assert!((beat - 0.5).abs() < 0.02, "{r}");
    let card_beats = pages[0].duration.0 as f64 / r["fit"]["cardBeat"].as_f64().unwrap();
    assert!((card_beats - card_beats.round()).abs() < 0.05, "{card_beats}");
    assert_eq!(total(&s, g).0, r["report"]["target"].as_i64().unwrap());

    // a refresh from the sheet keeps the fit
    std::fs::write(&sheet, "Body,Page Style,Vertical Gap\nTITLE,Card,\nA,Scroll,\nB,,4\nC,,4\nD,,4\nE,,4\nEND,Card,\n").unwrap();
    let rr = s.execute("graphics.credits.refresh", json!({"clip": g})).unwrap();
    let g2 = rr["clips"][0]["clip"].as_u64().unwrap();
    assert_eq!(total(&s, g2).0, r["report"]["target"].as_i64().unwrap(), "{rr}");

    // and it can be undone, or switched off
    let off = s.execute("graphics.credits.fit", json!({"clip": g2, "off": true})).unwrap();
    assert!(off["fit"].is_null());
    assert!(s.execute("graphics.credits.fit", json!({"clip": g2, "seconds": -5})).is_err());
}

#[test]
fn the_music_under_the_credits_is_found_even_a_frame_off() {
    let (mut s, music) = with_music();
    // the credits start a little before the music
    s.execute("timeline.move", json!({"clips": [music.0], "delta": cfa4_time::TICKS_PER_SECOND / 50})).ok();
    let v = s.execute("graphics.credits.import", json!({"csv": "Body\nA\n", "time": 0})).unwrap();
    let page = v["clips"][0]["clip"].as_u64().unwrap();
    s.execute("timeline.select", json!({"clips": [page]})).unwrap();
    let m = s.execute("graphics.credits.music", json!({})).unwrap();
    assert_eq!(m["audio"].as_u64(), Some(music.0), "{m}");
    assert_eq!(m["credits"].as_u64(), Some(v["group"].as_u64().unwrap()));
}

#[test]
fn music_is_heard_as_the_timeline_plays_it_and_fitting_works_without_music() {
    let (mut s, audio) = with_music();
    let v = s.execute("graphics.credits.import", json!({"csv": SHEET, "time": 0})).unwrap();
    let credits = clips(&v)[0];
    let normal = s.execute("graphics.credits.music", json!({"clip": credits.0, "audio": audio.0})).unwrap();
    // twice as fast: the sound ends twice as early on the timeline
    s.execute("timeline.select", json!({"clips": [audio.0]})).unwrap();
    s.execute("clip.speedDuration", json!({"speed": 200.0})).unwrap();
    let fast = s.execute("graphics.credits.music", json!({"clip": credits.0, "audio": audio.0})).unwrap();
    let (a, b) = (normal["soundEnd"].as_i64().unwrap(), fast["soundEnd"].as_i64().unwrap());
    assert!((b * 2 - a).abs() < cfa4_time::TICKS_PER_SECOND / 5, "{a} vs {b}");
    // no music at all: the credits are still offered for a typed length
    s.execute("edit.clear", json!({"clips": [audio.0]})).unwrap();
    let none = s.execute("graphics.credits.music", json!({"clip": credits.0})).unwrap();
    assert!(none["audio"].is_null() && none["natural"].as_i64().unwrap() > 0, "{none}");
    s.execute("timeline.select", json!({"clips": [credits.0]})).unwrap();
    let r = s.execute("graphics.credits.fit", json!({"clip": none["credits"], "seconds": 30.0})).unwrap();
    // the asked length to the frame
    assert_eq!(r["report"]["result"], r["report"]["target"], "{r}");
    assert!((r["report"]["result"].as_i64().unwrap() - 30 * cfa4_time::TICKS_PER_SECOND).abs() < cfa4_time::TICKS_PER_SECOND / 20);
}

// ----- C4: backgrounds, transitions, crawls, guides -----

const LOOK: &str = r##"{"pageStyles":[
    {"name":"Night","behaviour":"card","transition":"dissolve","transitionFrames":10,
     "background":{"kind":"gradient","color":"#101030","color2":"#000000","opacity":80,"blur":4}},
    {"name":"Sky","behaviour":"card","background":{"kind":"image","file":"sky.png","fit":"fill"}}
]}"##;

fn track_of(s: &Session, c: ClipId) -> usize {
    s.active_sequence().unwrap().video_tracks.iter().position(|t| t.items.iter().any(|i| i.id == c)).unwrap()
}

#[test]
fn backgrounds_sit_below_their_pages_and_transitions_join_the_pages() {
    let mut s = demo("credits-look");
    let dir = tmp_dir("credits-look");
    image::RgbaImage::from_pixel(960, 400, image::Rgba([40, 90, 200, 255])).save(dir.join("sky.png")).unwrap();
    let undo = s.history.undo.len();
    std::fs::write(dir.join("look.csv"), "Body,Page Style\nFIRST,Night\nSECOND,Night\nTHIRD,Sky\n").unwrap();
    let v = s.execute("graphics.credits.import", json!({"path": dir.join("look.csv"), "styling": LOOK})).unwrap();
    assert_eq!(v["warnings"].as_array().unwrap().len(), 0, "{v}");
    assert_eq!(s.history.undo.len(), undo + 1, "one undo step");
    let pages = clips(&v);
    let bgs: Vec<ClipId> = v["clips"].as_array().unwrap().iter().map(|c| ClipId(c["background"].as_u64().unwrap())).collect();
    for (p, b) in pages.iter().zip(&bgs) {
        assert_eq!(track_of(&s, *b) + 1, track_of(&s, *p), "the background is just below its page");
        let (pi, bi) = (item(&s, *p), item(&s, *b));
        assert_eq!((pi.start, pi.duration), (bi.start, bi.duration));
    }
    // a gradient behind the first page, at 80 % and blurred
    let b0 = item(&s, bgs[0]);
    let shape = b0.effects.iter().find(|e| e.effect == "graphic_shape").unwrap();
    assert_eq!(shape.params["fill_kind"].value, ParamValue::Choice(1));
    assert_eq!(b0.effects.iter().find(|e| e.effect == "opacity").unwrap().params["opacity"].value, ParamValue::Float(80.0));
    assert!(b0.effects.iter().any(|e| e.effect == "gaussian_blur"));
    // the picture fills the 16:9 frame: 960×400 → scale by the height, 1080 / 400
    let q = s.active_sequence().unwrap();
    let fh = f64::from(q.settings.height);
    let b2 = item(&s, bgs[2]);
    let ParamValue::Float(scale) = b2.effects.iter().find(|e| e.effect == "motion").unwrap().params["scale"].value else { panic!() };
    assert!((scale - fh / 400.0 * 100.0).abs() < 1e-6, "{scale}");
    // a dissolve on the cuts out of the Night pages, and between their backgrounds; none out of
    // the last page
    let on = |c: ClipId| {
        let q = s.active_sequence().unwrap();
        q.video_tracks.iter().flat_map(|t| t.transitions.iter()).filter(|x| x.from == Some(c)).map(|x| x.effect.effect.clone()).collect::<Vec<_>>()
    };
    assert_eq!(on(pages[0]), vec!["cross_dissolve"]);
    assert_eq!(on(bgs[0]), vec!["cross_dissolve"]);
    assert_eq!(on(pages[1]), vec!["cross_dissolve"]);
    assert_eq!(on(bgs[1]), vec!["cross_dissolve"]);
    assert!(on(pages[2]).is_empty());
    assert_eq!(v["clips"][0]["transition"], json!(true));
    // the dissolve replaces the fades at its cut
    let fades = |c: ClipId| {
        let it = item(&s, c);
        let e = it.effects.iter().find(|e| e.effect == "graphic_text").unwrap();
        e.params["opacity"].keyframes.iter().map(|k| (k.time, k.value.clone())).collect::<Vec<_>>()
    };
    let f0 = fades(pages[0]);
    assert_eq!(f0.last().map(|k| k.1.clone()), Some(ParamValue::Float(100.0)), "no fade out into the dissolve: {f0:?}");

    // a refresh with nothing changed puts everything back on the same tracks
    let tracks: Vec<usize> = pages.iter().chain(&bgs).map(|c| track_of(&s, *c)).collect();
    let r = s.execute("graphics.credits.refresh", json!({"clip": pages[0].0})).unwrap();
    let pages2 = clips(&r);
    let bgs2: Vec<ClipId> = r["clips"].as_array().unwrap().iter().map(|c| ClipId(c["background"].as_u64().unwrap())).collect();
    let tracks2: Vec<usize> = pages2.iter().chain(&bgs2).map(|c| track_of(&s, *c)).collect();
    assert_eq!(tracks, tracks2);
    for c in &bgs {
        assert!(s.active_sequence().unwrap().find_item(*c).is_none(), "old backgrounds are gone");
    }
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active_sequence().unwrap().find_item(bgs[0]).is_none());
}

#[test]
fn a_crawl_runs_left_and_its_pictures_go_with_it() {
    let mut s = demo("credits-crawl");
    let dir = tmp_dir("credits-crawl");
    image::RgbaImage::from_pixel(200, 100, image::Rgba([200, 200, 30, 255])).save(dir.join("logo.png")).unwrap();
    let csv = "Head,Body,Page Style\nMusic,Kiss Anna,Crawl\n,{{Pic logo.png 80}},\nThanks,Everyone,\n";
    let v = s.execute("graphics.credits.import", json!({"csv": csv, "pictureDir": dir})).unwrap();
    assert_eq!(v["warnings"].as_array().unwrap().len(), 0, "{v}");
    assert_eq!(v["clips"][0]["behaviour"], "crawl");
    let page = item(&s, clips(&v)[0]);
    assert_eq!(page.graphic.as_ref().unwrap().roll.mode, RollMode::CrawlLeft);
    let pic = item(&s, ClipId(v["clips"][0]["pictures"][0].as_u64().unwrap()));
    let motion = pic.effects.iter().find(|e| e.effect == "motion").unwrap();
    let q = s.active_sequence().unwrap();
    let canvas = (q.settings.width, q.settings.height);
    for k in [0i64, 3, 7, 10] {
        let u = Tick(page.duration.0 * k / 10);
        let specs = item_layer_specs(&page, page.source_in + u, canvas);
        let holder = specs.iter().find(|(_, sp)| matches!(sp.content, LayerContent::Shape(_))).unwrap();
        let ParamValue::Vec2(at) = motion.params["position"].value_at(u) else { panic!() };
        let want = holder.1.transform.position;
        assert!((at.x - want.x).abs() < 1e-6 && (at.y - want.y).abs() < 1e-6, "t={k}/10: {at:?} vs {want:?}");
    }
}

#[test]
fn thumbnail_shows_the_background_and_guides() {
    let mut s = demo("credits-thumb-look");
    let styling = r##"{"pageStyles":[{"name":"Red","behaviour":"card","background":{"kind":"color","color":"#c00000"}}]}"##;
    let csv = "Body,Page Style,Spine Position\nA,Red,\nB,,-400\n";
    let shot = |s: &mut Session, extra: Value| {
        let mut p = json!({"csv": csv, "styling": styling, "width": 384});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        let v = s.execute("graphics.credits.thumbnail", p).unwrap();
        let png = cfa4_project::gtemplate::base64_decode(v["pngBase64"].as_str().unwrap()).unwrap();
        image::load_from_memory(&png).unwrap().to_rgba8()
    };
    let plain = shot(&mut s, json!({}));
    // red in the corner: the background is behind the whole frame
    let c = plain.get_pixel(2, 2);
    assert!(c[0] > 150 && c[1] < 40 && c[3] > 250, "{c:?}");
    let guided = shot(&mut s, json!({"guides": true, "grid": true}));
    // the title-safe line is 5 % in (yellow over red), the centre line in the middle
    let (w, h) = guided.dimensions();
    let y = (f64::from(h) * 0.05).round() as u32;
    let g = guided.get_pixel(w / 2 + 20, y);
    assert!(g[1] > 150, "title safe at y={y}: {g:?}");
    assert_ne!(plain.get_pixel(w / 2, h / 4), guided.get_pixel(w / 2, h / 4), "centre line");
    // a spine 400 px left of the centre (on a 1080 frame) is drawn too
    let x = ((960.0 - 400.0 * f64::from(h) / 1080.0 * 1080.0 / f64::from(h)) * f64::from(w) / 1920.0).round() as u32;
    assert_ne!(plain.get_pixel(x, h / 3), guided.get_pixel(x, h / 3), "spine at x={x}");
}

#[test]
fn transitions_join_pages_with_and_without_backgrounds_and_take_their_pictures_along() {
    let mut s = demo("credits-look-mixed");
    let dir = tmp_dir("credits-look-mixed");
    image::RgbaImage::from_pixel(200, 100, image::Rgba([200, 30, 30, 255])).save(dir.join("logo.png")).unwrap();
    let styling = r##"{"pageStyles":[
        {"name":"Lit","behaviour":"card","transition":"dissolve","transitionFrames":10,"background":{"kind":"color","color":"#202040"}},
        {"name":"Plain","behaviour":"card","transition":"dissolve","transitionFrames":10}
    ]}"##;
    let csv = "Body,Page Style\nFIRST,Lit\n{{Pic logo.png}},Plain\nTHIRD,Lit\n";
    let v = s.execute("graphics.credits.import", json!({"csv": csv, "styling": styling, "pictureDir": dir, "track": 0})).unwrap();
    assert_eq!(v["warnings"].as_array().unwrap().len(), 0, "{v}");
    let pages = clips(&v);
    // all pages on one track, the backgrounds on the one below
    let tracks: Vec<usize> = pages.iter().map(|c| track_of(&s, *c)).collect();
    assert!(tracks.iter().all(|t| *t == tracks[0]), "{tracks:?}");
    let bg = ClipId(v["clips"][0]["background"].as_u64().unwrap());
    assert_eq!(track_of(&s, bg) + 1, tracks[0]);
    for c in &pages[..2] {
        let q = s.active_sequence().unwrap();
        assert!(q.video_tracks.iter().flat_map(|t| t.transitions.iter()).any(|x| x.from == Some(*c)), "a transition out of every page but the last");
    }
    // the middle page's picture fades in and out across half of each transition
    let pic = item(&s, ClipId(v["clips"][1]["pictures"][0].as_u64().unwrap()));
    let op = &pic.effects.iter().find(|e| e.effect == "opacity").unwrap().params["opacity"];
    assert_eq!(op.value_at(Tick::ZERO), ParamValue::Float(0.0));
    assert_eq!(op.value_at(pic.duration), ParamValue::Float(0.0));
    assert_eq!(op.value_at(Tick(pic.duration.0 / 2)), ParamValue::Float(100.0));
}
