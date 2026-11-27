use serde_json::{Value, json};

use super::{LayerPicture, picture_key, program_layers, render_source};
use crate::Session;
use crate::media_test_util::tmp_dir;
use cfa4_project::SourceKind;

fn ok(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn scene(s: &mut Session, name: &str) -> u64 {
    ok(s, "live.studio.scene.add", json!({"name": name}))["id"].as_u64().unwrap()
}

fn source(s: &mut Session, p: Value) -> u64 {
    ok(s, "live.studio.source.add", p)["source"]["id"].as_u64().unwrap()
}

#[test]
fn scenes_and_sources_are_undoable_project_edits() {
    let mut s = Session::default();
    assert!(s.execute("live.studio.source.add", json!({"type": "color"})).is_err(), "no scene yet");
    let a = scene(&mut s, "Main");
    let b = scene(&mut s, "");
    let st = ok(&mut s, "live.studio.scenes", json!({}));
    assert_eq!(st["current"], a, "the first scene is the current one");
    assert_eq!(st["scenes"][1]["name"], "Scene 2");

    let bg = source(&mut s, json!({"type": "color", "color": "#102030", "width": 640, "height": 360}));
    let title = source(&mut s, json!({"type": "text", "text": "Hello\nWorld", "size": 48, "color": [255, 200, 0], "transform": {"x": 100, "y": 50}}));
    let st = ok(&mut s, "live.studio.scenes", json!({}));
    let srcs = &st["scenes"][0]["sources"];
    assert_eq!(srcs[0]["kind"], json!({"type": "color", "color": [16, 32, 48, 255], "width": 640, "height": 360}));
    assert_eq!(srcs[1]["type"], "text");
    assert_eq!(srcs[1]["name"], "Hello");
    assert_eq!(srcs[1]["transform"]["x"], 100.0);
    assert_eq!(srcs[1]["transform"]["scaleX"], 1.0);

    // change, move, hide: each one undo step
    ok(&mut s, "live.studio.source.set", json!({"id": title, "transform": {"rotation": -90, "scaleX": 2}, "text": "Hi"}));
    let t = &s.project.studio.scenes[0].sources[1];
    assert_eq!((t.transform.rotation, t.transform.scale_x, t.transform.x), (270.0, 2.0, 100.0));
    assert!(matches!(&t.kind, SourceKind::Text { text, size, .. } if text == "Hi" && *size == 48.0));
    ok(&mut s, "live.studio.source.move", json!({"id": title, "index": 0}));
    assert_eq!(ok(&mut s, "live.studio.scenes", json!({}))["scenes"][0]["sources"][0]["id"], title);
    ok(&mut s, "live.studio.source.set", json!({"id": bg, "visible": false}));
    ok(&mut s, "live.studio.scene.select", json!({"id": b}));
    assert_eq!(s.project.studio.current, Some(b));
    ok(&mut s, "live.studio.scene.rename", json!({"id": b, "name": "Interview"}));
    ok(&mut s, "live.studio.scene.move", json!({"id": b, "index": 0}));
    assert_eq!(s.project.studio.scenes[0].name, "Interview");

    let history = ok(&mut s, "history.list", json!({}));
    let undo: Vec<&str> = history["undo"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(
        undo.ends_with(&[
            "Change Studio Source",
            "Move Studio Source",
            "Change Studio Source",
            "Select Studio Scene",
            "Rename Studio Scene",
            "Move Studio Scene"
        ]),
        "{undo:?}"
    );
    for _ in 0..6 {
        s.undo();
    }
    let st = &s.project.studio;
    assert_eq!(st.current, Some(a));
    assert_eq!(st.scenes[0].name, "Main");
    assert_eq!(st.scenes[0].sources[1].id, title);
    assert_eq!(st.scenes[0].sources[1].transform.rotation, 0.0);
    assert!(st.scenes[0].sources[0].visible);
    s.redo();
    assert_eq!(s.project.studio.scenes[0].sources[1].transform.rotation, 270.0);

    // a drag sends many changes with one merge key: one undo step
    let before = ok(&mut s, "history.list", json!({}))["undo"].as_array().unwrap().len();
    for x in [10, 20, 30] {
        ok(&mut s, "live.studio.source.set", json!({"id": bg, "transform": {"x": x}, "merge": "drag"}));
    }
    assert_eq!(ok(&mut s, "history.list", json!({}))["undo"].as_array().unwrap().len(), before + 1);
    assert_eq!(s.project.studio.scenes[0].sources[0].transform.x, 30.0);
    let unchanged = ok(&mut s, "live.studio.source.set", json!({"id": bg, "transform": {"x": 30}}));
    assert_eq!(unchanged["changed"], false);

    // removing the current scene selects a neighbour
    ok(&mut s, "live.studio.source.remove", json!({"id": title}));
    assert_eq!(s.project.studio.find_source(title), None);
    ok(&mut s, "live.studio.scene.remove", json!({"id": a}));
    assert_eq!(s.project.studio.current, Some(b));
    s.undo();
    assert_eq!(s.project.studio.current, Some(a));
}

#[test]
fn hostile_parameters_are_errors_not_edits() {
    let mut s = Session::default();
    let a = scene(&mut s, "A");
    let c = source(&mut s, json!({"type": "color"}));
    let undo = ok(&mut s, "history.list", json!({}))["undo"].as_array().unwrap().len();
    for (cmd, p) in [
        ("live.studio.source.add", json!({"type": "ndi"})),
        ("live.studio.source.add", json!({})),
        ("live.studio.source.add", json!({"type": "image"})),
        ("live.studio.source.add", json!({"type": "image", "path": "  "})),
        ("live.studio.source.add", json!({"type": "color", "color": "#12345"})),
        ("live.studio.source.add", json!({"type": "color", "color": [300, 0, 0]})),
        ("live.studio.source.add", json!({"type": "color", "width": 0})),
        ("live.studio.source.add", json!({"type": "color", "height": 100_000})),
        ("live.studio.source.add", json!({"type": "text", "size": 0})),
        ("live.studio.source.add", json!({"type": "text", "size": 1e9})),
        ("live.studio.source.add", json!({"type": "text", "text": "x".repeat(20_000)})),
        ("live.studio.source.add", json!({"type": "color", "scene": 999})),
        ("live.studio.source.add", json!({"type": "color", "transform": {"x": 1e300}})),
        ("live.studio.source.add", json!({"type": "color", "transform": {"scaleX": "big"}})),
        ("live.studio.source.add", json!({"type": "color", "transform": {"crop": [1, 2, 3]}})),
        ("live.studio.source.add", json!({"type": "color", "transform": 5})),
        ("live.studio.source.add", json!({"type": "color", "name": "n".repeat(500)})),
        ("live.studio.source.set", json!({"id": c, "type": "text"})),
        ("live.studio.source.set", json!({"id": c, "visible": "yes"})),
        ("live.studio.source.set", json!({"id": 999})),
        ("live.studio.source.move", json!({"id": c, "index": 5})),
        ("live.studio.source.remove", json!({})),
        ("live.studio.scene.remove", json!({"id": 999})),
        ("live.studio.scene.rename", json!({"id": a, "name": " "})),
        ("live.studio.scene.move", json!({"id": a, "index": 1})),
        ("live.studio.scene.select", json!({"id": c})),
    ] {
        assert!(s.execute(cmd, p.clone()).is_err(), "{cmd} {p}");
    }
    assert_eq!(ok(&mut s, "history.list", json!({}))["undo"].as_array().unwrap().len(), undo, "nothing was edited");
}

#[test]
fn scenes_are_saved_and_loaded_with_the_project() {
    let dir = tmp_dir("studio-save");
    let path = dir.join("studio.fcproj");
    let mut s = Session::default();
    scene(&mut s, "Main");
    let t = source(&mut s, json!({"type": "text", "text": "On air", "transform": {"x": 12.5, "crop": [1, 2, 3, 4]}}));
    ok(&mut s, "file.saveAs", json!({"path": path.to_string_lossy()}));
    let saved = s.project.studio.clone();
    let mut t2 = Session::default();
    ok(&mut t2, "file.open", json!({"path": path.to_string_lossy()}));
    assert_eq!(t2.project.studio, saved);
    assert_eq!(t2.project.studio.find_source(t), Some((saved.scenes[0].id, 0)));
    // new ids do not collide with the studio's
    let n = scene(&mut t2, "Next");
    assert!(n > t);
}

#[test]
fn program_layers_render_colour_text_and_pictures() {
    let dir = tmp_dir("studio-layers");
    // a 3x2 uncompressed TGA (32-bit BGRA, top-left origin): CFA4's own still decoder reads it
    let tga = dir.join("dot.tga");
    let mut data = vec![0u8, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 2, 0, 32, 0x28];
    for i in 0..6u8 {
        data.extend_from_slice(&[255 - i * 40, 0, i * 40, 255]);
    }
    std::fs::write(&tga, &data).unwrap();
    let mut s = Session::default();
    scene(&mut s, "Main");
    source(&mut s, json!({"type": "color", "color": [1, 2, 3, 255], "width": 64, "height": 32}));
    let txt = source(&mut s, json!({"type": "text", "text": "Ag", "size": 40}));
    let pic = source(&mut s, json!({"type": "image", "path": tga.to_string_lossy()}));
    let missing = source(&mut s, json!({"type": "image", "path": dir.join("gone.png").to_string_lossy()}));
    let hidden = source(&mut s, json!({"type": "color", "visible": false}));
    let (layers, problems) = program_layers(&s.project.studio, |src| render_source(&src.kind));
    assert_eq!(layers.len(), 3, "colour, text, picture; the missing file and the hidden source are not drawn");
    assert_eq!(layers[0].picture, LayerPicture::Color { rgba: [1, 2, 3, 255], width: 64, height: 32 });
    match &layers[1].picture {
        LayerPicture::Pixels { width, height, rgba } => {
            assert_eq!(layers[1].source, txt);
            assert!((20..80).contains(width) && (25..70).contains(height), "cut to the ink: {width}x{height}");
            assert!(rgba.chunks_exact(4).any(|p| p == [255, 255, 255, 255]), "white text");
        }
        other => panic!("text is pixels: {other:?}"),
    }
    match &layers[2].picture {
        LayerPicture::Pixels { width: 3, height: 2, rgba } => assert_eq!(rgba[..4], [0, 0, 255, 255]),
        other => panic!("picture: {other:?}"),
    }
    assert_eq!(layers[2].source, pic);
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].0, missing);
    assert!(!problems.iter().any(|p| p.0 == hidden));
    // a changed picture file changes the key
    let kind = SourceKind::Image { path: tga.to_string_lossy().into_owned() };
    let k1 = picture_key(&kind);
    std::thread::sleep(std::time::Duration::from_millis(20));
    data.push(0);
    std::fs::write(&tga, &data).unwrap();
    assert_ne!(picture_key(&kind), k1);
    assert!(render_source(&SourceKind::text("   ")).is_err());
    // no current scene: nothing to draw
    assert!(program_layers(&cfa4_project::LiveStudio::default(), |src| render_source(&src.kind)).0.is_empty());
}

#[test]
fn media_and_live_graphics_are_moving_sources() {
    let mut s = Session::default();
    scene(&mut s, "Main");
    let m = source(&mut s, json!({"type": "media", "path": "/videos/opener.mp4", "loop": false}));
    let g = source(&mut s, json!({"type": "liveGraphics", "transform": {"x": 0, "y": 0}}));
    let st = ok(&mut s, "live.studio.scenes", json!({}));
    let srcs = &st["scenes"][0]["sources"];
    assert_eq!(srcs[0]["kind"], json!({"type": "media", "path": "/videos/opener.mp4", "loop": false}));
    assert_eq!(srcs[0]["name"], "opener.mp4");
    assert_eq!(srcs[1]["kind"], json!({"type": "liveGraphics"}));
    assert_eq!(srcs[1]["name"], "Live Graphics");
    ok(&mut s, "live.studio.source.set", json!({"id": m, "loop": true, "path": "/videos/loop.mkv"}));
    assert_eq!(s.project.studio.scenes[0].sources[0].kind, SourceKind::Media { path: "/videos/loop.mkv".into(), looping: true });
    for p in [
        json!({"type": "media"}),
        json!({"type": "media", "path": ""}),
        json!({"type": "media", "path": "/v.mp4", "loop": "yes"}),
        json!({"type": "media", "path": "a\u{0}b"}),
    ] {
        assert!(s.execute("live.studio.source.add", p.clone()).is_err(), "{p}");
    }
    assert!(s.execute("live.studio.source.set", json!({"id": m, "loop": 1})).is_err());
    // moving sources are fed by the front end: render_source refuses them, program_layers asks
    let kinds: Vec<SourceKind> = s.project.studio.scenes[0].sources.iter().map(|x| x.kind.clone()).collect();
    assert!(kinds.iter().all(|k| k.is_moving() && render_source(k).is_err()));
    let (layers, problems) =
        program_layers(&s.project.studio, |src| Ok(LayerPicture::Feed { width: 32, height: 18, ring: std::path::PathBuf::from(format!("/feeds/{}", src.id)) }));
    assert!(problems.is_empty());
    assert_eq!(layers.iter().map(|l| l.source).collect::<Vec<_>>(), [m, g]);
}

#[test]
fn the_live_program_is_a_picture_for_the_studio() {
    let mut s = Session::default();
    ok(&mut s, "file.openDemoProject", json!({}));
    s.prefs_path = Some(tmp_dir("studio-live-program").join("prefs.json"));
    assert_eq!(crate::live::program_picture(&mut s, 320).expect("draw"), None, "nothing on air");
    let (w, h) = crate::live::program_size(&s, 320);
    let seq = s.active_sequence().map(|q| (q.settings.width, q.settings.height)).expect("sequence");
    assert_eq!((w, h), (320, (320.0 * f64::from(seq.1) / f64::from(seq.0)).round() as u32));
    ok(&mut s, "live.cue.add", json!({"csv": "Head,Body\nDirected by,Jordan Lee\n"}));
    ok(&mut s, "live.take", json!({}));
    let (rgba, pw, ph) = crate::live::program_picture(&mut s, 320).expect("draw").expect("on air");
    assert_eq!((pw, ph), (w, h));
    assert_eq!(rgba.len(), (w * h * 4) as usize);
    ok(&mut s, "live.out", json!({}));
    assert_eq!(crate::live::program_picture(&mut s, 320).expect("draw"), None, "OUT takes the program off");
}
