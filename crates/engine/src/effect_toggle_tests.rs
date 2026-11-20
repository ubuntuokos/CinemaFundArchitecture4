//! SC-4: `effects.toggleEnabled` on several clips / effects in one undo step.

use cfa4_project::TrackItem;
use serde_json::json;

use crate::Session;

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

fn clip(s: &Session, id: u64) -> TrackItem {
    s.active_sequence().unwrap().find_item(cfa4_project::ClipId(id)).unwrap().1.clone()
}

fn two_clips_with_blur(s: &mut Session) -> Vec<u64> {
    let ids: Vec<u64> = s.active_sequence().unwrap().video_tracks[0].items.iter().take(2).map(|i| i.id.0).collect();
    s.execute("effects.apply", json!({"clips": ids, "effect": "gaussian_blur"})).unwrap();
    ids
}

fn blur_idx(s: &Session, id: u64) -> usize {
    clip(s, id).effects.iter().position(|e| e.effect == "gaussian_blur").unwrap()
}

#[test]
fn single_toggle_is_unchanged() {
    let mut s = demo();
    let ids = two_clips_with_blur(&mut s);
    let i = blur_idx(&s, ids[0]);
    s.execute("effects.toggleEnabled", json!({"clip": ids[0], "index": i})).unwrap();
    assert!(!clip(&s, ids[0]).effects[i].enabled);
    assert!(clip(&s, ids[1]).effects[blur_idx(&s, ids[1])].enabled);
    s.execute("effects.toggleEnabled", json!({"clip": ids[0], "index": i})).unwrap();
    assert!(clip(&s, ids[0]).effects[i].enabled);
}

#[test]
fn bulk_by_effect_id_is_one_undo_step() {
    let mut s = demo();
    let ids = two_clips_with_blur(&mut s);
    let before = s.history.undo.len();
    let r = s.execute("effects.toggleEnabled", json!({"clips": ids, "effect": "gaussian_blur"})).unwrap();
    assert_eq!(r["changed"], 2);
    assert_eq!(r["enabled"], false);
    assert_eq!(s.history.undo.len(), before + 1, "one undo step");
    for id in &ids {
        assert!(!clip(&s, *id).effects[blur_idx(&s, *id)].enabled);
    }
    s.execute("edit.undo", json!({})).unwrap();
    for id in &ids {
        assert!(clip(&s, *id).effects[blur_idx(&s, *id)].enabled);
    }
    s.execute("edit.redo", json!({})).unwrap();
    assert!(!clip(&s, ids[0]).effects[blur_idx(&s, ids[0])].enabled);
}

#[test]
fn mixed_state_switches_all_on_and_explicit_enabled_wins() {
    let mut s = demo();
    let ids = two_clips_with_blur(&mut s);
    let i = blur_idx(&s, ids[0]);
    s.execute("effects.toggleEnabled", json!({"clip": ids[0], "index": i})).unwrap(); // mixed now
    let r = s.execute("effects.toggleEnabled", json!({"clips": ids, "effect": "gaussian_blur"})).unwrap();
    assert_eq!(r["enabled"], true);
    s.execute("effects.toggleEnabled", json!({"clips": ids, "effect": "gaussian_blur", "enabled": true})).unwrap();
    assert!(ids.iter().all(|id| clip(&s, *id).effects[blur_idx(&s, *id)].enabled));
}

#[test]
fn uses_the_selection_when_no_clip_is_named() {
    let mut s = demo();
    let ids = two_clips_with_blur(&mut s);
    s.execute("timeline.select", json!({"clips": [ids[1]]})).unwrap();
    s.execute("effects.toggleEnabled", json!({"effect": "gaussian_blur"})).unwrap();
    assert!(clip(&s, ids[0]).effects[blur_idx(&s, ids[0])].enabled);
    assert!(!clip(&s, ids[1]).effects[blur_idx(&s, ids[1])].enabled);
}

#[test]
fn hostile_parameters_error_or_do_nothing() {
    let mut s = demo();
    let ids = two_clips_with_blur(&mut s);
    let before = s.history.undo.len();
    for p in [
        json!({"clips": [], "index": 0}),
        json!({"clips": [u64::MAX], "index": 0}),
        json!({"clip": ids[0], "index": -1}),
        json!({"clip": ids[0], "indices": "x"}),
        json!({"clip": ids[0], "index": 0, "indices": [0]}),
        json!({"clip": ids[0], "enabled": "yes"}),
        json!({"clip": ids[0], "effect": 5}),
    ] {
        assert!(s.execute("effects.toggleEnabled", p.clone()).is_err(), "{p}");
    }
    let r = s.execute("effects.toggleEnabled", json!({"clip": ids[0], "index": 9999})).unwrap();
    assert_eq!(r["changed"], 0);
    assert_eq!(s.history.undo.len(), before, "no empty undo step");
}

#[test]
fn all_switches_only_applied_effects_and_leaves_intrinsics() {
    let mut s = demo();
    let ids = two_clips_with_blur(&mut s);
    let r = s.execute("effects.toggleEnabled", json!({"clips": ids, "all": true})).unwrap();
    assert_eq!(r["enabled"], false);
    for id in &ids {
        for e in &clip(&s, *id).effects {
            assert_eq!(e.enabled, e.def().is_some_and(|d| d.intrinsic), "{}", e.effect);
        }
    }
    assert!(s.execute("effects.toggleEnabled", json!({"clips": ids, "all": 3})).is_err());
}
