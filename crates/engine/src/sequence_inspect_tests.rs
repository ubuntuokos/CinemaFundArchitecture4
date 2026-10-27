//! `sequence.inspect` is how an agent checks its edits: it has to show where a clip ends in the
//! sequence and in its media, the clip's audio gain, whether it plays backward and what a marker says.

use serde_json::json;

use crate::Session;

#[test]
fn sequence_inspect_reports_clip_ends_gain_and_marker_comments() {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let rate = s.sequence_rate();
    // a clip at 200 %, a clip with gain and a marker with a comment, all made through commands
    s.execute("sequence.linkedSelection", json!({"on": false})).unwrap();
    let (fast, loud) = {
        let q = s.active_sequence().unwrap();
        (q.video_tracks[0].items.last().unwrap().id, q.audio_tracks[0].items[0].id)
    };
    s.execute("timeline.select", json!({"clips": [fast.0]})).unwrap();
    s.execute("clip.speedDuration", json!({"speed": 200})).unwrap();
    s.execute("timeline.select", json!({"clips": [loud.0]})).unwrap();
    s.execute("clip.audioGain", json!({"mode": "set", "db": -6.5})).unwrap();
    let marker = s.execute("markers.add", json!({"time": rate.tick_of(12).0, "name": "Beat", "comment": "cut on the downbeat"})).unwrap()["marker"].clone();

    let q = s.active_sequence().unwrap().clone();
    let js = s.execute("sequence.inspect", json!({})).unwrap();
    let m = js["markers"].as_array().unwrap().iter().find(|m| m["id"] == marker).unwrap();
    assert_eq!((m["name"].as_str(), m["comment"].as_str()), (Some("Beat"), Some("cut on the downbeat")));
    let mut seen = 0;
    for (key, tracks) in [("video", &q.video_tracks), ("audio", &q.audio_tracks)] {
        for (ti, t) in tracks.iter().enumerate() {
            for (ci, c) in t.items.iter().enumerate() {
                let j = &js[key][ti]["items"][ci];
                assert_eq!(j["clip"], c.id.0);
                assert_eq!(j["end"], c.end().0, "{j}");
                assert_eq!(j["end"].as_i64(), j["start"].as_i64().zip(j["duration"].as_i64()).map(|(a, b)| a + b));
                assert_eq!(j["endFrame"], rate.frame_at(c.end()));
                assert_eq!(j["sourceOut"], c.source_out().0, "{j}");
                assert_eq!(j["gainDb"], c.gain_db);
                // the keys that were there before stay
                for old in ["item", "name", "start", "duration", "startFrame", "durationFrames", "sourceIn", "speed", "enabled", "link", "label", "effects"] {
                    assert!(j.get(old).is_some(), "{old} is still reported");
                }
                if c.id == fast {
                    assert_eq!(j["speed"], 2.0);
                    assert_eq!(
                        j["sourceOut"].as_i64().unwrap() - j["sourceIn"].as_i64().unwrap(),
                        2 * j["duration"].as_i64().unwrap(),
                        "at 200% a clip uses twice its length of media"
                    );
                    seen += 1;
                }
                if c.id == loud {
                    assert_eq!(j["gainDb"], -6.5);
                    seen += 1;
                }
            }
        }
    }
    assert_eq!(seen, 2, "both edited clips were found in the readback");
}

/// A clip set to play backward read back exactly like a forward one: same `speed`, same source
/// range, and nothing that said it was reversed.
#[test]
fn sequence_inspect_reports_a_reversed_clip() {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let backward = s.active_sequence().unwrap().video_tracks[0].items[1].id;
    let all = |js: &serde_json::Value| -> Vec<(u64, bool)> {
        ["video", "audio"]
            .iter()
            .flat_map(|k| js[*k].as_array().unwrap().iter())
            .flat_map(|t| t["items"].as_array().unwrap().iter())
            .map(|i| (i["clip"].as_u64().unwrap(), i["reverse"].as_bool().expect("every clip reports `reverse`")))
            .collect()
    };
    let js = s.execute("sequence.inspect", json!({})).unwrap();
    assert!(all(&js).iter().all(|(_, reversed)| !reversed), "nothing plays backward yet");
    s.execute("sequence.linkedSelection", json!({"on": false})).unwrap();
    s.execute("timeline.select", json!({"clips": [backward.0]})).unwrap();
    s.execute("clip.speedDuration", json!({"speed": 100, "reverse": true})).unwrap();
    let js = s.execute("sequence.inspect", json!({})).unwrap();
    let reversed: Vec<u64> = all(&js).into_iter().filter(|(_, r)| *r).map(|(c, _)| c).collect();
    assert_eq!(reversed, vec![backward.0], "only the reversed clip says so");
    s.execute("edit.undo", json!({})).unwrap();
    let js = s.execute("sequence.inspect", json!({})).unwrap();
    assert!(all(&js).iter().all(|(_, reversed)| !reversed));
}

#[test]
fn sequence_inspect_reports_edit_points_and_through_edits() {
    // the Qt timeline's edit point menu: the selected edit point and whether a cut is a through edit
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s.execute("sequence.linkedSelection", json!({"on": false})).unwrap();
    let (clip, mid) = {
        let q = s.active_sequence().unwrap();
        let it = &q.video_tracks[0].items[0];
        (it.id, crate::Tick(it.start.0 + it.duration.0 / 2))
    };
    s.execute("timeline.razor", json!({"time": mid.0, "clip": clip.0})).unwrap();
    let js = s.execute("sequence.inspect", json!({})).unwrap();
    let right = js["video"][0]["items"][1]["clip"].clone();
    assert_eq!(js["editPoints"], json!([]));
    assert!(js["throughEdits"].as_array().unwrap().contains(&json!({"left": clip.0, "right": right})));
    s.execute("trim.selectEditPoint", json!({"clip": clip.0, "edge": "out", "kind": "roll"})).unwrap();
    let js = s.execute("sequence.inspect", json!({})).unwrap();
    assert_eq!(js["editPoints"], json!([{"clip": clip.0, "out": true, "kind": "roll"}]));
}

#[test]
fn sequence_inspect_reports_frame_holds_and_the_timeline_toggles() {
    // the Qt timeline's thumbnails show a held frame; its toolbar shows Snap, Linked Selection, Nest
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s.execute("sequence.linkedSelection", json!({"on": false})).unwrap();
    let (clip, source_in) = {
        let it = &s.active_sequence().unwrap().video_tracks[0].items[0];
        (it.id, it.source_in.0)
    };
    let js = s.execute("sequence.inspect", json!({})).unwrap();
    assert_eq!(js["video"][0]["items"][0]["frameHold"], json!(null));
    assert_eq!(js["toggles"]["linkedSelection"], json!(false));
    let snapping = js["toggles"]["snapping"].as_bool().unwrap();
    s.execute("timeline.select", json!({"clips": [clip.0]})).unwrap();
    s.execute("clip.frameHoldOptions", json!({"holdOn": "in"})).unwrap();
    s.execute("sequence.snap", json!({})).unwrap();
    s.execute("sequence.nestSequences", json!({"on": false})).unwrap();
    s.execute("sequence.showThroughEdits", json!({"on": true})).unwrap();
    let js = s.execute("sequence.inspect", json!({})).unwrap();
    assert_eq!(js["video"][0]["items"][0]["frameHold"], json!(source_in));
    assert_eq!(js["toggles"], json!({"snapping": !snapping, "linkedSelection": false, "nest": false, "showThroughEdits": true}));
}

#[test]
fn sequence_inspect_lists_the_open_sequences_in_tab_order() {
    // the Qt Timeline's sequence tabs: open, reorder, close
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let a = s.state.active_sequence.unwrap();
    s.execute("file.newSequence", json!({"name": "Second"})).unwrap();
    let b = s.state.active_sequence.unwrap();
    assert_ne!(a, b);
    s.execute("sequence.open", json!({"item": b.0})).unwrap();
    let names = |s: &mut Session| -> Vec<u64> {
        let js = s.execute("sequence.inspect", json!({})).unwrap();
        js["openSequences"].as_array().unwrap().iter().map(|t| t["id"].as_u64().unwrap()).collect()
    };
    assert_eq!(names(&mut s), vec![a.0, b.0]);
    let js = s.execute("sequence.inspect", json!({})).unwrap();
    assert_eq!(js["openSequences"][1]["name"], json!("Second"));
    s.execute("sequence.moveTab", json!({"item": b.0, "index": 0})).unwrap();
    assert_eq!(names(&mut s), vec![b.0, a.0]);
    s.execute("sequence.close", json!({"item": a.0})).unwrap();
    assert_eq!(names(&mut s), vec![b.0]);
}

#[test]
fn sequence_inspect_reports_the_mix_fader() {
    // the Qt Timeline's Mix row shows the master fader in dB
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    assert_eq!(s.execute("sequence.inspect", json!({})).unwrap()["masterVolumeDb"], json!(0.0));
    s.execute("mixer.setStrip", json!({"strip": "Mix", "volumeDb": -3.5})).unwrap();
    assert_eq!(s.execute("sequence.inspect", json!({})).unwrap()["masterVolumeDb"], json!(-3.5));
}

#[test]
fn sequence_inspect_reports_the_volume_line_of_audio_clips() {
    // the Qt Timeline's Volume line: the level, the keyframes in media and timeline time, the curve
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let (clip, start, dur, level) = {
        let it = &s.active_sequence().unwrap().audio_tracks[0].items[0];
        (it.id.0, it.start.0, it.duration.0, it.effect("volume").and_then(|e| e.param("level")).unwrap().f64_at(it.source_in))
    };
    let js = s.execute("sequence.inspect", json!({})).unwrap();
    let vol = &js["audio"][0]["items"][0]["volume"];
    assert_eq!((vol["db"].as_f64(), vol["animated"].as_bool()), (Some(level), Some(false)));
    assert_eq!(vol["curve"], json!([]));
    s.execute("effects.setParam", json!({"clip": clip, "effect": "volume", "param": "level", "value": -6.0})).unwrap();
    let js = s.execute("sequence.inspect", json!({})).unwrap();
    assert_eq!(js["audio"][0]["items"][0]["volume"]["db"].as_f64(), Some(-6.0));
    // two keyframes: a ramp, sampled across the clip
    s.execute("effects.addKeyframe", json!({"clip": clip, "effect": "volume", "param": "level", "time": start})).unwrap();
    s.execute("effects.addKeyframe", json!({"clip": clip, "effect": "volume", "param": "level", "time": start + dur / 2})).unwrap();
    let js = s.execute("sequence.inspect", json!({})).unwrap();
    let vol = &js["audio"][0]["items"][0]["volume"];
    assert_eq!(vol["animated"], json!(true));
    let keys = vol["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0]["time"].as_i64(), Some(start));
    assert_eq!(keys[1]["time"].as_i64(), Some(start + dur / 2));
    assert_eq!(vol["curve"].as_array().unwrap().len(), 129);
    // video clips have no Volume line
    assert_eq!(js["video"][0]["items"][0]["volume"], json!(null));
}

#[test]
fn sequence_inspect_lists_an_audio_tracks_automation_lanes() {
    // the Qt Timeline's track keyframes: the lanes a track can show, their range and keyframes
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let a1 = s.active_sequence().unwrap().audio_tracks[0].id.0;
    let js = s.execute("sequence.inspect", json!({})).unwrap();
    let lanes = js["audio"][0]["lanes"].as_array().unwrap();
    let keys: Vec<&str> = lanes.iter().map(|l| l["key"].as_str().unwrap()).collect();
    assert_eq!(&keys[..3], &["volume", "pan", "mute"]);
    assert_eq!((lanes[0]["fader"].as_bool(), lanes[2]["hold"].as_bool()), (Some(true), Some(true)));
    assert_eq!((lanes[1]["min"].as_f64(), lanes[1]["max"].as_f64()), (Some(-100.0), Some(100.0)));
    assert_eq!(lanes[0]["keys"], json!([]));
    s.execute("mixer.setKeyframe", json!({"strip": a1, "lane": "volume", "time": 1000, "value": -6.0})).unwrap();
    s.execute("mixer.setKeyframe", json!({"strip": a1, "lane": "volume", "time": 9000, "value": -12.0})).unwrap();
    let js = s.execute("sequence.inspect", json!({})).unwrap();
    assert_eq!(js["audio"][0]["lanes"][0]["keys"], json!([[1000, -6.0, false], [9000, -12.0, false]]));
    assert!(js["video"][0].get("lanes").is_none(), "video tracks have no automation lanes");
}

#[test]
fn sequence_inspect_reports_the_caption_tracks() {
    // the Qt Timeline's caption rows: header state, each caption's span and plain text, selection
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let js = s.execute("sequence.inspect", json!({})).unwrap();
    assert_eq!((js["captionTracks"].clone(), js["captionSelection"].clone()), (json!([]), json!([])));
    let id = s.execute("captions.add", json!({"text": "<b>Hello</b>", "time": 0})).unwrap()["caption"].as_u64().unwrap();
    s.execute("captions.setTrack", json!({"track": "C1", "locked": true})).unwrap();
    let js = s.execute("sequence.inspect", json!({})).unwrap();
    let t = &js["captionTracks"][0];
    assert_eq!(
        (t["label"].as_str(), t["format"].as_str(), t["locked"].as_bool(), t["enabled"].as_bool()),
        (Some("C1"), Some("Subtitle"), Some(true), Some(true))
    );
    let c = &t["captions"][0];
    assert_eq!((c["id"].as_u64(), c["start"].as_i64(), c["text"].as_str()), (Some(id), Some(0), Some("Hello")));
    assert!(c["end"].as_i64().unwrap() > 0);
    assert_eq!(js["captionSelection"], json!([id]));
}

/// `dialog.defaults` opens a menu command's dialog as egui's `clip_dialogs::defaults` does: the
/// selected clip's speed, the next nested sequence name, Field Options' defaults; a command
/// without a dialog, or one that can't run now, is an error.
#[test]
fn dialog_defaults_start_each_dialog_from_the_selection() {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s.execute("sequence.linkedSelection", json!({"on": false})).unwrap();
    let clip = s.active_sequence().unwrap().video_tracks[0].items[0].id;
    s.execute("timeline.select", json!({"clips": [clip.0]})).unwrap();
    s.execute("clip.speedDuration", json!({"speed": 50, "reverse": true})).unwrap();
    let d = s.execute("dialog.defaults", json!({"command": "clip.speedDuration"})).unwrap();
    assert_eq!(d["params"]["speed"], 50.0, "{d}");
    assert_eq!(d["params"]["reverse"], true, "a reversed clip opens with Reverse Speed on");
    assert_eq!(d["params"]["interpolation"], "frameSampling");
    let dur = s.active_sequence().unwrap().find_item(clip).unwrap().1.duration.0;
    assert_eq!(d["info"]["duration"], dur);
    assert!(d["info"]["fpsNum"].as_i64().is_some_and(|n| n > 0));
    let d = s.execute("dialog.defaults", json!({"command": "clip.nest"})).unwrap();
    assert_eq!(d["params"]["name"], "Nested Sequence 01");
    let d = s.execute("dialog.defaults", json!({"command": "clip.fieldOptions"})).unwrap();
    assert_eq!(d["params"], json!({"reverseFieldDominance": false, "processing": "none"}));
    let d = s.execute("dialog.defaults", json!({"command": "edit.removeAttributes"})).unwrap();
    assert_eq!((d["info"]["video"].as_bool(), d["info"]["audio"].as_bool()), (Some(true), Some(false)));
    assert!(s.execute("dialog.defaults", json!({"command": "file.save"})).is_err());
    assert!(s.execute("dialog.defaults", json!({})).is_err());
    s.execute("timeline.select", json!({"clips": []})).unwrap();
    assert!(s.execute("dialog.defaults", json!({"command": "clip.speedDuration"})).is_err(), "nothing selected: no dialog");
}
