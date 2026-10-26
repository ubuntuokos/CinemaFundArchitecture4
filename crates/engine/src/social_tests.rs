//! Tests of [`crate::social`] (safe zones, Auto Reframe Sequence, Social Deliverables) and
//! [`crate::delivery_check`] (Check Before Export).

use cfa4_project::{ClipId, ItemId};
use serde_json::{Value, json};

use crate::Session;
use crate::social::{SafeZone, aspect_of, reframed_size, strictest_safe_rect};

/// A 1920 × 1080 / 24 fps sequence with `clips` colour mattes of the given lengths (s) placed
/// back to back on V1 from `first` s.
fn session(first: f64, clips: &[f64]) -> (Session, Vec<ClipId>) {
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "Promo", "width": 1920, "height": 1080, "fps": 24, "video": 2, "audio": 1})).unwrap();
    let mut at = first;
    let mut ids = Vec::new();
    for &secs in clips {
        let r = s.execute("file.newColorMatte", json!({"color": "#3366aa", "seconds": secs})).unwrap();
        let item = r["item"].as_u64().unwrap();
        let r = s.execute("timeline.place", json!({"item": item, "track": "V1", "seconds": at})).unwrap();
        ids.push(ClipId(r["clips"][0].as_u64().unwrap()));
        at += secs;
    }
    (s, ids)
}

fn issues(r: &Value, kind: &str) -> usize {
    r["issues"].as_array().unwrap().iter().filter(|i| i["kind"] == kind).count()
}

#[test]
fn safe_zones_and_sizes() {
    for z in SafeZone::ALL {
        assert_eq!(SafeZone::from_id(z.id()), Some(z));
        let r = z.safe_rect(1080.0, 1920.0);
        assert!(r[0] > 0.0 && r[1] > 0.0 && r[2] < 1080.0 && r[3] < 1920.0 && r[0] < r[2] && r[1] < r[3], "{z:?} {r:?}");
        let all = strictest_safe_rect(1080.0, 1920.0);
        assert!(all[0] >= r[0] && all[1] >= r[1] && all[2] <= r[2] && all[3] <= r[3]);
    }
    assert_eq!(SafeZone::from_id("snapchat"), None);
    assert_eq!(reframed_size(1920, 1080, 9, 16), (1080, 1920));
    assert_eq!(reframed_size(1920, 1080, 1, 1), (1080, 1080));
    assert_eq!(reframed_size(1920, 1080, 4, 5), (1080, 1350));
    assert_eq!(reframed_size(1080, 1920, 16, 9), (1920, 1080));
    // hostile sizes never panic and stay even and bounded
    for (w, h) in [(0, 0), (1, 1), (u32::MAX, 3), (7, u32::MAX)] {
        for (_, a, b) in crate::social::ASPECTS {
            let (x, y) = reframed_size(w, h, a, b);
            assert!(x >= 2 && y >= 2 && x % 2 == 0 && y % 2 == 0 && x <= 16_384 && y <= 16_384, "{w}x{h} → {x}x{y}");
        }
    }
    assert_eq!(aspect_of(1080, 1920), Some("9:16"));
    assert_eq!(aspect_of(1440, 1080), None);
}

#[test]
fn auto_reframe_sequence_copies_with_new_ids_and_reframe() {
    let (mut s, clips) = session(0.0, &[2.0, 2.0]);
    s.execute("graphics.newText", json!({"text": "Title"})).unwrap();
    let orig = s.state.active_sequence.unwrap();
    let spec = crate::commands::find("sequence.autoReframe").unwrap();
    assert_eq!(spec.menu, &["Sequence"]);
    let r = s.execute("sequence.autoReframe", json!({"aspect": "9:16"})).unwrap();
    let new = ItemId(r["sequence"].as_u64().unwrap());
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(1080), Some(1920)));
    assert_eq!(s.state.active_sequence, Some(new), "opened");
    assert_eq!(s.project.item(new).unwrap().name, "Promo (9×16)");
    let q = s.project.sequence(new).unwrap();
    q.check().unwrap();
    let copied: Vec<_> = q.video_tracks.iter().flat_map(|t| &t.items).collect();
    assert_eq!(copied.len(), 3);
    for it in &copied {
        assert!(!clips.contains(&it.id), "copies get new clip ids");
        let graphic = matches!(s.project.item(it.item).unwrap().kind, cfa4_project::ItemKind::Graphic { .. });
        assert_eq!(it.effects.iter().any(|e| e.effect == "auto_reframe"), !graphic, "pictures get Auto Reframe, graphics do not");
    }
    // the original is untouched; one undo step removes the copy
    assert!(s.project.sequence(orig).unwrap().video_tracks.iter().flat_map(|t| &t.items).all(|i| i.effects.iter().all(|e| e.effect != "auto_reframe")));
    assert!(s.execute("sequence.autoReframe", json!({"aspect": "3:2"})).is_err());
}

#[test]
fn social_deliverables_queue_one_export_per_aspect() {
    let (mut s, _) = session(0.0, &[2.0]);
    let dir = crate::media_test_util::tmp_dir("deliverables");
    let spec = crate::commands::find("export.socialDeliverables").unwrap();
    assert_eq!(spec.menu, &["File", "Export"]);
    let r = s.execute("export.socialDeliverables", json!({"path": dir.to_string_lossy()})).unwrap();
    let d = r["deliverables"].as_array().unwrap();
    assert_eq!(d.len(), 4);
    assert_eq!(r["reframed"].as_array().unwrap().len(), 3, "16:9 is the sequence itself");
    assert_eq!(r["queued"].as_array().unwrap().len(), 4);
    assert_eq!(r["loudnessLufs"], -14.0);
    let q = s.execute("export.queue.list", json!({})).unwrap();
    let paths: Vec<String> = q["items"].as_array().unwrap().iter().map(|i| i["path"].as_str().unwrap_or("").to_string()).collect();
    assert_eq!(paths.len(), 4);
    let mut uniq = paths.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), 4, "one file per aspect: {paths:?}");
    // a second run reuses the reframed copies
    let r = s.execute("export.socialDeliverables", json!({"aspects": ["9:16"], "loudnessLufs": false, "path": dir.to_string_lossy()})).unwrap();
    assert!(r["reframed"].as_array().unwrap().is_empty());
    assert_eq!(r["loudnessLufs"], Value::Null);
    assert!(s.execute("export.socialDeliverables", json!({"aspects": ["2:1"]})).is_err());
    assert!(s.execute("export.socialDeliverables", json!({"loudnessLufs": 12.0})).is_err());
}

#[test]
fn check_before_export_finds_and_marks_problems() {
    // a gap from 2 to 3 s, a one-frame clip at 5 s
    let (mut s, _) = session(0.0, &[2.0]);
    let r = s.execute("file.newColorMatte", json!({"color": "#ffffff", "seconds": 3.0})).unwrap();
    s.execute("timeline.place", json!({"item": r["item"], "track": "V1", "seconds": 3.0})).unwrap();
    let r = s.execute("file.newColorMatte", json!({"color": "#ff0000", "seconds": 1.0 / 24.0})).unwrap();
    s.execute("timeline.place", json!({"item": r["item"], "track": "V2", "seconds": 4.0})).unwrap();
    s.execute("captions.newTrack", json!({})).unwrap();
    s.execute("captions.add", json!({"text": "This caption is far too long to read in such a short time", "seconds": 0.5, "durationSeconds": 1.0})).unwrap();
    s.execute("captions.add", json!({"text": "Hi", "seconds": 3.0, "durationSeconds": 0.3})).unwrap();
    s.execute("playhead.set", json!({"seconds": 3.5})).unwrap();
    s.execute("graphics.newText", json!({"text": "Way off", "position": [3000, 500]})).unwrap();
    let spec = crate::commands::find("sequence.checkDelivery").unwrap();
    assert_eq!(spec.menu, &["File", "Export"]);
    let r = s.execute("sequence.checkDelivery", json!({})).unwrap();
    assert_eq!(r["ok"], false);
    assert_eq!(issues(&r, "blackGap"), 1, "{r:#}");
    assert_eq!(issues(&r, "flashFrame"), 1, "{r:#}");
    assert_eq!(issues(&r, "captionFast"), 1, "{r:#}");
    assert_eq!(issues(&r, "captionShort"), 1, "{r:#}");
    assert_eq!(issues(&r, "offFrame"), 1, "{r:#}");
    assert_eq!(r["zone"], Value::Null, "horizontal frames check no platform zone by default");
    let gap = r["issues"].as_array().unwrap().iter().find(|i| i["kind"] == "blackGap").unwrap();
    assert!((gap["seconds"].as_f64().unwrap() - 2.0).abs() < 1e-6);
    let marks = |s: &Session| s.active_sequence().unwrap().markers.iter().filter(|m| m.comment.starts_with(crate::delivery_check::CHECK_COMMENT)).count();
    assert_eq!(marks(&s), r["count"].as_u64().unwrap() as usize);
    // again: replaced, not doubled
    s.execute("sequence.checkDelivery", json!({})).unwrap();
    assert_eq!(marks(&s), r["count"].as_u64().unwrap() as usize);
    s.execute("markers.clearChecks", json!({})).unwrap();
    assert_eq!(marks(&s), 0);
    assert!(s.execute("sequence.checkDelivery", json!({"zone": "myspace"})).is_err());
    assert!(s.execute("sequence.checkDelivery", json!({"maxCps": 0.0})).is_err());
}

#[test]
fn vertical_text_under_platform_buttons_is_flagged() {
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "Reel", "width": 1080, "height": 1920, "fps": 30, "video": 2, "audio": 1})).unwrap();
    let r = s.execute("file.newColorMatte", json!({"color": "#202020", "seconds": 5.0})).unwrap();
    s.execute("timeline.place", json!({"item": r["item"], "track": "V1", "seconds": 0.0})).unwrap();
    s.execute("playhead.set", json!({"seconds": 1.0})).unwrap();
    // centred: safe everywhere
    s.execute("graphics.newText", json!({"text": "Safe", "position": [540, 900]})).unwrap();
    let r = s.execute("sequence.checkDelivery", json!({})).unwrap();
    assert_eq!(r["zone"], "all platforms");
    assert_eq!(issues(&r, "safeZone"), 0, "{r:#}");
    // near the bottom: under the caption and button area of every platform
    s.execute("graphics.newText", json!({"text": "Buy now", "position": [540, 1800]})).unwrap();
    let r = s.execute("sequence.checkDelivery", json!({"zone": "reels"})).unwrap();
    assert_eq!(r["zone"], "Instagram Reels");
    assert_eq!(issues(&r, "safeZone"), 1, "{r:#}");
    let r = s.execute("sequence.checkDelivery", json!({"zone": "none", "markers": false})).unwrap();
    assert_eq!(issues(&r, "safeZone"), 0);
}

#[test]
fn a_vertical_deliverable_renders_at_its_own_size() {
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "Tiny", "width": 320, "height": 180, "fps": 24, "video": 1, "audio": 1})).unwrap();
    let r = s.execute("file.newBarsAndTone", json!({"seconds": 0.5})).unwrap();
    s.execute("timeline.place", json!({"item": r["item"], "track": "V1", "seconds": 0.0})).unwrap();
    let dir = crate::media_test_util::tmp_dir("deliverable-render");
    let r = s.execute("export.socialDeliverables", json!({"aspects": ["9:16"], "path": dir.to_string_lossy(), "start": true, "wait": true})).unwrap();
    assert_eq!(r["queued"].as_array().unwrap().len(), 1);
    let q = s.execute("export.queue.list", json!({})).unwrap();
    let item = &q["items"][0];
    assert_eq!(item["status"], "done", "{item:#}");
    let path = item["path"].as_str().unwrap();
    assert!(std::fs::metadata(path).unwrap().len() > 0);
    let r = s.execute("file.import", json!({"paths": [path]})).unwrap();
    let id = ItemId(r["items"][0].as_u64().unwrap_or_else(|| panic!("{r}")));
    let cfa4_project::ItemKind::Media(m) = &s.project.item(id).unwrap().kind else { panic!("not media") };
    let v = m.info.video.as_ref().unwrap();
    assert_eq!((v.width, v.height), (180, 320), "9:16 of a 320 × 180 sequence");
}

#[test]
fn placeholders_fill_from_a_row() {
    let row = vec![("Name".to_string(), "Anna".to_string()), ("Price".to_string(), "9 €".to_string())];
    let mut miss = Vec::new();
    assert_eq!(crate::variants::fill("Hi {{name}}, only {{ PRICE }}!", &row, &mut miss), "Hi Anna, only 9 €!");
    assert!(miss.is_empty());
    assert_eq!(crate::variants::fill("{{city}} {{ broken", &row, &mut miss), "{{city}} {{ broken");
    assert_eq!(miss, vec!["city".to_string()]);
    assert_eq!(crate::variants::fill("{{}}x}}", &row, &mut miss), "{{}}x}}");
}

#[test]
fn batch_variants_make_one_sequence_per_row() {
    let (mut s, _) = session(0.0, &[3.0]);
    s.execute("graphics.newText", json!({"text": "{{Headline}} – {{Price}}"})).unwrap();
    let r = s.execute("graphics.newText", json!({"text": "placeholder"})).unwrap();
    let clip = ClipId(r["clip"].as_u64().unwrap());
    let layers = s.execute("graphics.list", json!({"clip": clip.0})).unwrap();
    let layer = layers["layers"][0]["layer"].clone();
    s.execute("graphics.set", json!({"clip": clip.0, "layer": layer, "props": {"name": "Tagline"}})).unwrap();
    let orig = s.state.active_sequence.unwrap();
    let undo0 = s.history.undo.len();
    let spec = crate::commands::find("export.batchVariants").unwrap();
    assert_eq!(spec.menu, &["File", "Export"]);
    let dir = crate::media_test_util::tmp_dir("variants");
    let csv = "Name,Headline,Price,Tagline\nSpring sale,Fresh looks,19 €,Only this week\nVIP,Members first,free,For you\n";
    let r = s.execute("export.batchVariants", json!({"csv": csv, "exportPath": dir.to_string_lossy()})).unwrap();
    let v = r["variants"].as_array().unwrap();
    assert_eq!(v.len(), 2, "{r:#}");
    assert_eq!(r["queued"].as_array().unwrap().len(), 2);
    assert_eq!(s.history.undo.len(), undo0 + 1, "one undo step");
    let texts = |s: &Session, id: ItemId| -> Vec<String> {
        let q = s.project.sequence(id).unwrap();
        let mut out = Vec::new();
        for it in q.video_tracks.iter().flat_map(|t| &t.items) {
            for e in it.effects.iter().filter(|e| e.effect == cfa4_project::graphic::TEXT_LAYER) {
                if let Some(cfa4_project::ParamValue::Text(t)) = e.params.get("text").map(|p| &p.value) {
                    out.push(t.clone());
                }
            }
        }
        out.sort();
        out
    };
    let first = ItemId(v[0]["sequence"].as_u64().unwrap());
    assert_eq!(s.project.item(first).unwrap().name, "Promo – Spring sale");
    assert_eq!(texts(&s, first), vec!["Fresh looks – 19 €".to_string(), "Only this week".to_string()]);
    let second = ItemId(v[1]["sequence"].as_u64().unwrap());
    assert_eq!(texts(&s, second), vec!["For you".to_string(), "Members first – free".to_string()]);
    assert_eq!(texts(&s, orig), vec!["placeholder".to_string(), "{{Headline}} – {{Price}}".to_string()], "the template is untouched");
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.project.sequence(first).is_none() && s.project.sequence(second).is_none());
    // unknown placeholders are reported; bad tables are errors
    let r = s.execute("export.batchVariants", json!({"csv": "Headline\nHello\n", "export": false})).unwrap();
    assert_eq!(r["missing"], json!(["Price"]));
    assert_eq!(r["queued"], json!([]));
    assert!(s.execute("export.batchVariants", json!({"csv": ""})).is_err());
    assert!(s.execute("export.batchVariants", json!({"csv": "Headline\n"})).is_err());
    assert!(s.execute("export.batchVariants", json!({})).is_err());
    let many = format!("Headline\n{}", "x\n".repeat(crate::variants::MAX_ROWS + 1));
    assert!(s.execute("export.batchVariants", json!({"csv": many})).is_err());
}

#[test]
fn text_animators_change_what_a_title_shows_over_time() {
    let (mut s, _) = session(0.0, &[5.0]);
    let r = s.execute("graphics.newText", json!({"text": "432,014,696 views"})).unwrap();
    let clip = ClipId(r["clip"].as_u64().unwrap());
    let shown = |s: &Session, secs: f64| -> String {
        let q = s.active_sequence().unwrap();
        let (_, it) = q.find_item(clip).unwrap();
        let mt = it.source_in + cfa4_time::Tick::from_seconds_f64(secs);
        let specs = cfa4_render::graphic_clip::item_layer_specs(it, mt, (1920, 1080));
        match &specs[0].1.content {
            cfa4_project::graphic::LayerContent::Text(t) => t.text.clone(),
            _ => panic!("not text"),
        }
    };
    for (id, label) in [
        ("graphics.textAnimator.counter", "Counter"),
        ("graphics.textAnimator.typewriter", "Typewriter"),
        ("graphics.textAnimator.wordByWord", "Word by Word"),
        ("graphics.textAnimator.none", "None"),
    ] {
        let spec = crate::commands::find(id).unwrap();
        assert_eq!(spec.menu, &["Graphics and Titles", "Text Animator"]);
        assert_eq!(spec.label, label);
        assert!((spec.enabled)(&s).is_ok(), "{id}");
    }
    s.execute("graphics.textAnimator.counter", json!({"duration": 2.0})).unwrap();
    assert_eq!(shown(&s, 0.0), "0 views");
    let mid = shown(&s, 1.0);
    assert!(mid != "0 views" && mid != "432,014,696 views" && mid.ends_with(" views"), "{mid}");
    assert_eq!(shown(&s, 2.5), "432,014,696 views");
    s.execute("graphics.textAnimator.typewriter", json!({"duration": 1.0, "delay": 1.0})).unwrap();
    assert_eq!(shown(&s, 0.5), "");
    assert_eq!(shown(&s, 1.5).chars().count(), 8);
    s.execute("graphics.set", json!({"clip": clip.0, "props": {"anim": "Word by Word"}})).unwrap();
    assert_eq!(shown(&s, 1.6), "432,014,696");
    s.execute("graphics.textAnimator.none", json!({})).unwrap();
    assert_eq!(shown(&s, 0.0), "432,014,696 views");
    // the Source Text never changes; one undo step per change
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(shown(&s, 1.6), "432,014,696");
    assert!(s.execute("graphics.textAnimator.counter", json!({"duration": -1.0})).is_err());
    // a shape layer has no animator
    s.execute("graphics.newShape", json!({"clip": clip.0})).unwrap();
    assert!(s.execute("graphics.textAnimator.counter", json!({"clip": clip.0, "layer": 1})).is_err());
}

#[test]
fn reframe_moves_motion_positions_with_the_frame_centre() {
    let (mut s, clips) = session(0.0, &[2.0]);
    let c = clips[0].0;
    // explicitly centred, and animated off-centre
    s.execute("effects.setParam", json!({"clip": c, "effect": "motion", "param": "position", "value": [960.0, 540.0]})).unwrap();
    s.execute("effects.setParam", json!({"clip": c, "effect": "motion", "param": "scale", "value": 90.0})).unwrap();
    let r = s.execute("sequence.autoReframe", json!({"aspect": "9:16"})).unwrap();
    let q = s.project.sequence(ItemId(r["sequence"].as_u64().unwrap())).unwrap();
    let it = q.video_tracks.iter().flat_map(|t| &t.items).next().unwrap();
    let p = it.effect("motion").unwrap().vec2_at("position", it.source_in);
    assert_eq!((p.x, p.y), (540.0, 960.0), "the centre of 1920×1080 becomes the centre of 1080×1920");
    // keyframes move too
    let (mut s, clips) = session(0.0, &[2.0]);
    let c = clips[0].0;
    for (t, x) in [(0i64, 100.0), (1, 1800.0)] {
        let time = cfa4_time::Tick::from_seconds_f64(t as f64).0;
        s.execute("effects.setParam", json!({"clip": c, "effect": "motion", "param": "position", "value": [x, 540.0], "keyframe": true, "time": time}))
            .unwrap();
    }
    let r = s.execute("sequence.autoReframe", json!({"aspect": "1:1"})).unwrap();
    let q = s.project.sequence(ItemId(r["sequence"].as_u64().unwrap())).unwrap();
    let it = q.video_tracks.iter().flat_map(|t| &t.items).next().unwrap();
    let pos = it.effect("motion").unwrap().params.get("position").unwrap();
    let xs: Vec<(f64, f64)> = pos
        .keyframes
        .iter()
        .filter_map(|k| match k.value {
            cfa4_project::ParamValue::Vec2(v) => Some((v.x, v.y)),
            _ => None,
        })
        .collect();
    // 1920×1080 → 1080×1080: the centre moves by (−420, 0)
    assert_eq!(xs, vec![(-320.0, 540.0), (1380.0, 540.0)]);
}

#[test]
fn check_measures_graphics_in_the_reframed_frame_and_audio_offline() {
    let (mut s, _) = session(0.0, &[3.0]);
    s.execute("playhead.set", json!({"seconds": 1.0})).unwrap();
    // inside 1920 wide; the copy keeps it 790 px right of the centre, outside 1080 wide
    s.execute("graphics.newText", json!({"text": "Hi", "position": [1750, 540]})).unwrap();
    let r = s.execute("sequence.checkDelivery", json!({"markers": false})).unwrap();
    assert_eq!(issues(&r, "offFrame"), 0, "{r:#}");
    s.execute("sequence.autoReframe", json!({"aspect": "9:16"})).unwrap();
    let r = s.execute("sequence.checkDelivery", json!({"markers": false})).unwrap();
    assert_eq!(issues(&r, "offFrame"), 1, "{r:#}");
    // an offline music clip on an audio track is reported
    let (mut s, _) = session(0.0, &[3.0]);
    let r = s.execute("file.newOfflineFile", json!({"fileName": "music.wav", "video": false, "audio": true, "seconds": 3.0})).unwrap();
    s.execute("timeline.place", json!({"item": r["item"], "track": "A1", "seconds": 0.0})).unwrap();
    let r = s.execute("sequence.checkDelivery", json!({"markers": false})).unwrap();
    assert_eq!(issues(&r, "offline"), 1, "{r:#}");
}

#[test]
fn batch_variants_keep_names_unique_undo_whole_and_detach_source_graphics() {
    let (mut s, _) = session(0.0, &[3.0]);
    let r = s.execute("graphics.newText", json!({"text": "{{Headline}}"})).unwrap();
    let clip = r["clip"].as_u64().unwrap();
    let r = s.execute("graphics.upgradeToSourceGraphic", json!({"clip": clip})).unwrap();
    let source = ItemId(r["item"].as_u64().unwrap());
    let orig = s.state.active_sequence.unwrap();
    // a history limit smaller than the batch must still give one undo step
    let undo0 = s.history.undo.len();
    s.history.limit = undo0 + 2;
    let csv = "Name,Headline\nA/B,One\nA:B,Two\na/b,Three\nOther,Four\nLast,Five\n";
    let r = s.execute("export.batchVariants", json!({"csv": csv, "export": false})).unwrap();
    let names: Vec<&str> = r["variants"].as_array().unwrap().iter().map(|v| v["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["A_B", "A_B (2)", "a_b (3)", "Other", "Last"]);
    assert_eq!(s.history.undo.len(), undo0 + 1);
    let text_of = |s: &Session, seq: ItemId| -> (ItemId, String) {
        let q = s.project.sequence(seq).unwrap();
        let it = q.video_tracks.iter().flat_map(|t| &t.items).find(|i| i.effects.iter().any(|e| e.effect == cfa4_project::graphic::TEXT_LAYER)).unwrap();
        let e = it.effects.iter().find(|e| e.effect == cfa4_project::graphic::TEXT_LAYER).unwrap();
        match &e.params["text"].value {
            cfa4_project::ParamValue::Text(t) => (it.item, t.clone()),
            _ => panic!(),
        }
    };
    let seqs: Vec<ItemId> = r["variants"].as_array().unwrap().iter().map(|v| ItemId(v["sequence"].as_u64().unwrap())).collect();
    for (i, seq) in seqs.iter().enumerate() {
        let (item, _) = text_of(&s, *seq);
        assert_ne!(item, source, "variant {i} has its own graphic item");
    }
    // an unrelated graphics edit afterwards does not sync one variant's text everywhere
    s.execute("sequence.open", json!({"item": orig.0})).unwrap();
    s.execute("playhead.set", json!({"seconds": 2.0})).unwrap();
    s.execute("graphics.newText", json!({"text": "Unrelated"})).unwrap();
    assert_eq!(text_of(&s, orig).1, "{{Headline}}");
    assert_eq!(text_of(&s, seqs[0]).1, "One");
    assert_eq!(text_of(&s, seqs[4]).1, "Five");
    // one undo removes the whole batch even though it was longer than the history limit
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert!(seqs.iter().all(|q| s.project.sequence(*q).is_none()));
}
