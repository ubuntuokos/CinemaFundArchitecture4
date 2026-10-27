//! The Premiere audio-effect set through engine commands: listed with folders by
//! `effects.list`, applicable as clip effects (`effects.apply`) and track effects
//! (`mixer.addInsert`), parameters settable by id, and the sequence still renders.

use super::*;
use cfa4_project::ParamValue;
use cfa4_project::effect::PREMIERE_AUDIO_EFFECTS;
use serde_json::json;

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

fn a1_clip(s: &Session) -> u64 {
    s.active_sequence().unwrap().audio_tracks[0].items[0].id.0
}

fn render(s: &Session, frames: usize) -> cfa4_frame::AudioBuffer {
    let provider = s.media.provider(s.project.clone(), s.services.clone());
    cfa4_render::audio::mix_sequence(&s.project, s.active_sequence().unwrap(), 0, frames, &provider)
}

#[test]
fn effects_list_has_every_audio_effect_in_its_folder() {
    let mut s = demo();
    let list = s.execute("effects.list", json!({})).unwrap();
    let arr = list.as_array().unwrap();
    for id in PREMIERE_AUDIO_EFFECTS {
        let e = arr.iter().find(|e| e["id"] == id).unwrap_or_else(|| panic!("{id} not listed"));
        assert_eq!(e["kind"], "Audio", "{id}");
        assert_eq!(e["category"][0], "Audio Effects", "{id}");
    }
    let folder = |id: &str| arr.iter().find(|e| e["id"] == id).unwrap()["category"][1].clone();
    assert_eq!(folder("tube_compressor"), "Amplitude and Compression");
    assert_eq!(folder("chorus_flanger"), "Modulation");
    assert_eq!(folder("binauralizer"), "Special");
    assert!(folder("volume_a").is_null(), "loose item");
}

#[test]
fn every_audio_effect_applies_to_clips_and_tracks_and_renders() {
    let mut s = demo();
    let clip = a1_clip(&s);
    for id in PREMIERE_AUDIO_EFFECTS {
        let n0 = s.active_sequence().unwrap().find_item(ClipId(clip)).unwrap().1.effects.len();
        s.execute("effects.apply", json!({"clips": [clip], "effect": id})).unwrap_or_else(|e| panic!("{id}: {e}"));
        let it = s.active_sequence().unwrap().find_item(ClipId(clip)).unwrap().1.clone();
        assert_eq!(it.effects.len(), n0 + 1, "{id}");
        assert_eq!(it.effects.last().unwrap().effect, id);
        assert!(cfa4_render::audio_fx::supported(id), "{id}");
        s.execute("mixer.addInsert", json!({"strip": "A1", "effect": id})).unwrap_or_else(|e| panic!("{id}: {e}"));
        let b = render(&s, 2400);
        assert_eq!(b.frames(), 2400);
        assert!(b.channels.iter().flatten().all(|v| v.is_finite()), "{id}: non-finite mix");
        s.execute("mixer.removeInsert", json!({"strip": "A1", "slot": 0})).unwrap();
        s.execute("effects.remove", json!({"clip": clip, "index": n0})).unwrap();
    }
}

#[test]
fn parameters_are_set_by_id_and_change_the_sound() {
    let mut s = demo();
    let clip = a1_clip(&s);
    let before = render(&s, 4800);
    let peak = |b: &cfa4_frame::AudioBuffer| b.channels.iter().flatten().fold(0.0f32, |a, v| a.max(v.abs()));
    assert!(peak(&before) > 1e-3, "the demo has audio on A1");
    // Mute (a loose item) silences the clip.
    s.execute("effects.apply", json!({"clips": [clip], "effect": "Mute"})).unwrap();
    let it = s.active_sequence().unwrap().find_item(ClipId(clip)).unwrap().1.clone();
    let idx = it.effects.iter().position(|e| e.effect == "mute").unwrap();
    s.execute("effects.setParam", json!({"clip": clip, "effect": idx, "param": "mute", "value": true})).unwrap();
    let it = s.active_sequence().unwrap().find_item(ClipId(clip)).unwrap().1.clone();
    assert_eq!(it.effects[idx].param("mute").unwrap().value, ParamValue::Bool(true));
    // the clip's own signal (other tracks of the demo keep playing)
    let it = s.active_sequence().unwrap().find_item(ClipId(clip)).unwrap().1.clone();
    let provider = s.media.provider(s.project.clone(), s.services.clone());
    let a0 = it.start.to_units_floor(48000);
    let sig = cfa4_render::audio::clip_signal(&it, a0, 4800, 48000, &provider).unwrap();
    let pk = sig.iter().flatten().fold(0.0f32, |a, v| a.max(v.abs()));
    assert!(pk < 1e-6, "muted clip peak {pk}");
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    // A track-level Graphic Equalizer, its master gain set through mixer.setInsert (A1 soloed so
    // only it is heard).
    s.execute("mixer.setStrip", json!({"strip": "A1", "solo": true})).unwrap();
    let before = render(&s, 4800);
    s.execute("mixer.addInsert", json!({"strip": "A1", "effect": "graphic_eq_30"})).unwrap();
    s.execute("mixer.setInsert", json!({"strip": "A1", "slot": 0, "params": {"gain": -12.0}})).unwrap();
    let quieter = render(&s, 4800);
    let rms =
        |b: &cfa4_frame::AudioBuffer| (b.channels.iter().flatten().map(|v| (*v as f64).powi(2)).sum::<f64>() / (b.frames() * b.channel_count()) as f64).sqrt();
    let d = 20.0 * (rms(&quieter) / rms(&before)).log10();
    assert!((d + 12.0).abs() < 1.0, "master gain −12 dB → {d}");
}

#[test]
fn shared_names_resolve_by_track_kind() {
    let mut s = demo();
    let clip = a1_clip(&s);
    for (name, id) in [("Invert", "invert_a"), ("Volume", "volume_a"), ("Channel Volume", "channel_volume_a"), ("Balance", "balance_a")] {
        s.execute("effects.apply", json!({"clips": [clip], "effect": name})).unwrap();
        let it = s.active_sequence().unwrap().find_item(ClipId(clip)).unwrap().1.clone();
        assert_eq!(it.effects.last().unwrap().effect, id, "{name}");
    }
    // On a video clip "Invert" is still the video effect.
    let v = s.active_sequence().unwrap().video_tracks[0].items[0].id.0;
    s.execute("effects.apply", json!({"clips": [v], "effect": "Invert"})).unwrap();
    let it = s.active_sequence().unwrap().find_item(ClipId(v)).unwrap().1.clone();
    assert!(it.effects.iter().any(|e| e.effect == "invert"));
}

/// #696: applying an audio effect to the video side of an A/V pair lands on the linked audio.
#[test]
fn audio_effect_on_video_clip_applies_to_linked_audio() {
    let mut s = demo();
    let seq = s.active_sequence().unwrap();
    let v = seq.video_tracks[0].items[0].clone();
    let a = seq.audio_tracks[0].items.iter().find(|i| i.link == v.link).expect("linked audio").clone();
    let n0 = a.effects.len();
    s.execute("effects.apply", json!({"clips": [v.id.0], "effect": "parametric_eq"})).unwrap();
    let a = s.active_sequence().unwrap().find_item(a.id).unwrap().1.clone();
    let v = s.active_sequence().unwrap().find_item(v.id).unwrap().1.clone();
    assert_eq!(a.effects.len(), n0 + 1);
    assert_eq!(a.effects.last().unwrap().effect, "parametric_eq");
    assert!(v.effects.iter().all(|e| e.effect != "parametric_eq"), "EQ stays off the picture clip");
}

#[test]
fn fx_editor_reports_the_dsp_curves_and_previews_overrides() {
    let mut s = demo();
    let clip = a1_clip(&s);
    s.execute("effects.apply", json!({"clips": [clip], "effect": "parametric_eq"})).unwrap();
    let idx = s.active_sequence().unwrap().find_item(ClipId(clip)).unwrap().1.effects.len() - 1;
    let v = s.execute("audio.fxEditor", json!({"clip": clip, "effect": idx})).unwrap();
    assert_eq!(v["editor"], "parametric");
    assert_eq!(v["kind"], "clip");
    assert_eq!(v["params"]["mid_gain"]["min"], -30.0);
    let resp = v["response"].as_array().unwrap();
    assert_eq!(resp.len(), 241);
    let at = |v: &Value, f: f64| {
        v["response"].as_array().unwrap().iter().min_by(|a, b| (a[0].as_f64().unwrap() - f).abs().total_cmp(&(b[0].as_f64().unwrap() - f).abs())).unwrap()[1]
            .as_f64()
            .unwrap()
    };
    // a +12 dB bell at 1 kHz, previewed without touching the project (no undo step)
    let undo = s.history.undo.len();
    let o = s
        .execute(
            "audio.fxEditor",
            json!({"clip": clip, "effect": idx, "overrides": {"mid_on": true, "mid_freq": 1000.0, "mid_gain": 12.0, "b1_gain": "x", "nope": 3}}),
        )
        .unwrap();
    assert!((at(&o, 1000.0) - 12.0).abs() < 1.0, "{}", at(&o, 1000.0));
    assert_eq!(o["params"]["mid_gain"]["value"], 12.0);
    assert_eq!(s.history.undo.len(), undo);
    // out-of-range overrides are clamped to the parameter's range
    let o = s.execute("audio.fxEditor", json!({"clip": clip, "effect": idx, "overrides": {"mid_gain": 1e308}})).unwrap();
    assert_eq!(o["params"]["mid_gain"]["value"], 30.0);
    // graphic EQ band labels, multiband and dynamics transfer curves, on mixer inserts
    s.execute("mixer.addInsert", json!({"strip": "A1", "effect": "graphic_eq_20"})).unwrap();
    s.execute("mixer.addInsert", json!({"strip": "A1", "effect": "multiband_compressor"})).unwrap();
    s.execute("mixer.addInsert", json!({"strip": "A1", "effect": "dynamics_rack"})).unwrap();
    s.execute("mixer.addInsert", json!({"strip": "A1", "effect": "delay"})).unwrap();
    let g = s.execute("audio.fxEditor", json!({"strip": "A1", "slot": 0})).unwrap();
    assert_eq!((g["editor"].as_str(), g["kind"].as_str(), g["labels"].as_array().unwrap().len()), (Some("graphic"), Some("insert"), 20));
    let m = s.execute("audio.fxEditor", json!({"strip": "A1", "slot": 1})).unwrap();
    let t = m["transfer"].as_array().unwrap();
    assert_eq!(t.len(), 4);
    assert!(t.iter().all(|b| b.as_array().unwrap().len() == 121));
    let d = s.execute("audio.fxEditor", json!({"strip": "A1", "slot": 2})).unwrap();
    assert_eq!(d["transfer"].as_array().unwrap().len(), 1);
    // hostile or wrong targets are errors, never panics
    assert!(s.execute("audio.fxEditor", json!({"strip": "A1", "slot": 3})).is_err(), "delay has no editor");
    assert!(s.execute("audio.fxEditor", json!({"strip": "A1", "slot": u64::MAX})).is_err());
    assert!(s.execute("audio.fxEditor", json!({"strip": "Z9", "slot": 0})).is_err());
    assert!(s.execute("audio.fxEditor", json!({"clip": clip, "effect": 999})).is_err());
    assert!(s.execute("audio.fxEditor", json!({"clip": -1, "effect": 0})).is_err());
    assert!(s.execute("audio.fxEditor", json!({})).is_err());
}

#[test]
fn fx_editor_curves_use_the_sequence_sample_rate() {
    let mut s = demo();
    let clip = a1_clip(&s);
    s.execute("effects.apply", json!({"clips": [clip], "effect": "parametric_eq"})).unwrap();
    let idx = s.active_sequence().unwrap().find_item(ClipId(clip)).unwrap().1.effects.len() - 1;
    let ask =
        |s: &mut Session| s.execute("audio.fxEditor", json!({"clip": clip, "effect": idx, "overrides": {"high_gain": 12.0, "high_freq": 15000.0}})).unwrap();
    let rate = s.active_sequence().unwrap().settings.sample_rate;
    let a = ask(&mut s);
    assert_eq!(a["sampleRate"], rate);
    let seq = s.state.active_sequence.unwrap();
    std::sync::Arc::make_mut(&mut s.project).sequence_mut(seq).unwrap().settings.sample_rate = if rate == 44_100 { 48_000 } else { 44_100 };
    let b = ask(&mut s);
    assert_ne!(a["sampleRate"], b["sampleRate"]);
    assert_ne!(a["response"], b["response"], "the filter design follows the rate the programme plays at");
}

#[test]
fn fx_editor_reads_the_shown_time_and_lists_the_effect_stack() {
    let mut s = demo();
    let clip = a1_clip(&s);
    s.execute("effects.apply", json!({"clips": [clip], "effect": "parametric_eq"})).unwrap();
    s.execute("effects.apply", json!({"clips": [clip], "effect": "parametric_eq"})).unwrap();
    let it = s.active_sequence().unwrap().find_item(ClipId(clip)).unwrap().1.clone();
    let idx = it.effects.len() - 1;
    let (t0, t1) = (it.start, it.start + Tick(it.duration.0 / 2));
    for (t, v) in [(t0, -6.0), (t1, 6.0)] {
        s.execute("effects.setParam", json!({"clip": clip, "effect": idx, "param": "high_gain", "value": v, "keyframe": true, "time": t.0})).unwrap();
    }
    s.set_playhead(t0);
    let at = |s: &mut Session, t: Option<Tick>| {
        let mut p = json!({"clip": clip, "effect": idx});
        if let Some(t) = t {
            p["time"] = json!(t.0);
        }
        s.execute("audio.fxEditor", p).unwrap()
    };
    assert_eq!(at(&mut s, None)["params"]["high_gain"]["value"], json!(-6.0));
    // a front end playing ahead of the engine's playhead asks for the frame it shows
    assert_eq!(at(&mut s, Some(t1))["params"]["high_gain"]["value"], json!(6.0));
    let stack = at(&mut s, None)["stack"].clone();
    let names: Vec<&str> = stack.as_array().unwrap().iter().filter_map(serde_json::Value::as_str).collect();
    assert_eq!(names.len(), it.effects.len());
    assert_eq!(&names[names.len() - 2..], ["parametric_eq", "parametric_eq"]);
}
