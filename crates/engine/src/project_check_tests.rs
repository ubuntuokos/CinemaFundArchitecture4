//! `project.check` (KDN-23): a healthy project is ok, a missing file and an unused item are found,
//! and the check changes nothing.

use cfa4_media::DemoScene;
use serde_json::{Value, json};

use crate::media_test_util::{make_movie, session_with, tmp_dir};

fn kinds(r: &Value) -> Vec<String> {
    r["findings"].as_array().unwrap().iter().map(|f| f["kind"].as_str().unwrap().to_string()).collect()
}

#[test]
fn healthy_project_is_ok_and_a_missing_file_is_reported() {
    let root = tmp_dir("project-check");
    let (a, b) = (root.join("a.mov"), root.join("b.mov"));
    make_movie(&a, DemoScene::Aurora, 160, 90, 12);
    make_movie(&b, DemoScene::CityNight, 160, 90, 12);
    let (mut s, _items, _) = session_with(&[&a, &b]);

    let r = s.execute("project.check", json!({})).unwrap();
    assert_eq!(r["ok"], json!(true), "{r}");
    assert_eq!(r["sequences"].as_array().unwrap().len(), 1, "{r}");
    assert!(r["sequences"][0]["clips"].as_u64().unwrap() >= 1, "{r}");

    let before = s.project.clone();
    std::fs::remove_file(&a).unwrap();
    let r = s.execute("project.check", json!({})).unwrap();
    assert_eq!(r["ok"], json!(false), "{r}");
    assert!(kinds(&r).contains(&"missing".to_string()), "{r}");
    assert_eq!(r["counts"]["missing"], json!(1), "{r}");
    assert_eq!(s.project, before, "the check is read-only");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn unused_media_is_advice_and_an_unknown_effect_is_damage() {
    let root = tmp_dir("project-check-unused");
    let a = root.join("a.mov");
    make_movie(&a, DemoScene::Aurora, 160, 90, 12);
    let (mut s, _items, _) = session_with(&[&a]);
    let b = root.join("b.mov");
    make_movie(&b, DemoScene::CityNight, 160, 90, 12);
    s.execute("file.import", json!({"paths": [b.to_string_lossy()]})).unwrap();
    let r = s.execute("project.check", json!({})).unwrap();
    assert!(kinds(&r).contains(&"unusedMedia".to_string()), "{r}");
    assert_eq!(r["ok"], json!(true), "unused media alone does not fail the check: {r}");

    let mut q = s.project.sequences().find_map(|i| match &i.kind {
        cfa4_project::ItemKind::Sequence(q) => Some((i.id, q.clone())),
        _ => None,
    });
    let (id, seq) = q.as_mut().unwrap();
    let mut seq = (**seq).clone();
    let clip = seq.video_tracks.iter_mut().flat_map(|t| t.items.iter_mut()).next().unwrap();
    let mut fx = clip.effects.first().cloned().unwrap_or_else(|| cfa4_project::effect::EffectInstance {
        effect: String::new(),
        enabled: true,
        params: Default::default(),
        masks: vec![],
        post_fader: false,
        essential: false,
        layer: None,
    });
    fx.effect = "no.such.effect".into();
    clip.effects.push(fx);
    if let Some(it) = std::sync::Arc::make_mut(&mut s.project).items.get_mut(id) {
        it.kind = cfa4_project::ItemKind::Sequence(std::sync::Arc::new(seq));
    }
    let r = s.execute("project.check", json!({})).unwrap();
    assert!(kinds(&r).contains(&"unknownEffect".to_string()), "{r}");
    assert_eq!(r["ok"], json!(false), "{r}");
    let _ = std::fs::remove_dir_all(&root);
}
