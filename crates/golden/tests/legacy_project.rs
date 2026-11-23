//! AC-15 (CFA4-RENDER-INTEGRATION-PLAN-001, F0V): a project in an older schema opens, saves and
//! reopens with no field lost and renders the same frames.
//!
//! What this measures, and what it does not. The fixtures are **derived from a project of this build**:
//! the envelope is rewritten to carry each older schema number, the pre-envelope bare form (schema 1,
//! `version` field) and the upstream application's `format` tag (written through
//! `cfa4_project::legacy`, the only file where that name may appear). The migration chain is applied
//! to them as on a real load. Files really saved by old builds are not in the repository (PENDING:
//! the owner supplies some), so a field an old build wrote that this build no longer knows is out of
//! reach of this test.
//!
//! The two checks are exact: the project JSON after open → save → reopen equals the project JSON
//! of the original (0 lost or changed fields), and the rendered frames hash identically.

use std::sync::Arc;

use cfa4_format::{FORMAT_ID, SCHEMA_VERSION, decode, encode};
use cfa4_media::generators::GeneratorSource;
use cfa4_media::{DemoScene, Generator, MediaSource, SharedSource};
use cfa4_project::{ItemKind, Label, MediaClip, MediaRef, ParamValue, Project, SequenceSettings, TrackKind};
use cfa4_render::{RenderOptions, SourceMap, render_sequence};
use cfa4_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};
use serde_json::Value;
use sha2::{Digest, Sha256};

const RATE: FrameRate = FrameRate::FPS_24;

fn add_media(p: &mut Project, g: Generator) -> cfa4_project::ItemId {
    let src = GeneratorSource::new(g, 640, 360, RATE, Tick(10 * TICKS_PER_SECOND));
    let info = src.info().clone();
    p.add_item(
        &info.name.clone(),
        Label::Iris,
        ItemKind::Media(MediaClip {
            media: MediaRef::Generator(src.generator.clone()),
            info,
            interpret: Default::default(),
            mark_in: None,
            mark_out: None,
            markers: vec![],
            offline: false,
            proxy: None,
            identity: None,
        }),
        None,
    )
}

/// One picture (scaled, rotated, half transparent) and one tone, as in the validation scene.
fn project() -> (Project, cfa4_project::ItemId) {
    let mut p = Project::new("legacy");
    let seq = p.new_sequence("legacy", SequenceSettings { width: 320, height: 180, frame_rate: RATE, sample_rate: 48_000, ..Default::default() }, 1, 1, None);
    let picture = add_media(&mut p, Generator::Demo(DemoScene::Aurora));
    let tone = add_media(&mut p, Generator::Tone { hz: 440.0, db: -12.0 });
    let dur = TimeRange::new(Tick::ZERO, RATE.tick_of(12));
    let mut v = p.make_track_item(picture, TrackKind::Video, Tick::ZERO, dur, RATE).expect("video clip");
    v.scale_to_frame = true;
    for (effect, params) in
        [("motion", vec![("scale", ParamValue::Float(45.0)), ("rotation", ParamValue::Float(20.0))]), ("opacity", vec![("opacity", ParamValue::Float(70.0))])]
    {
        let e = v.effect_mut(effect).expect("intrinsic effect");
        for (k, val) in params {
            e.param_mut(k).expect("parameter").value = val;
        }
    }
    let a = p.make_track_item(tone, TrackKind::Audio, Tick::ZERO, dur, RATE).expect("audio clip");
    let s = p.sequence_mut(seq).expect("sequence");
    s.video_tracks[0].items.push(v);
    s.audio_tracks[0].items.push(a);
    (p, seq)
}

/// The sources are generators, so they are rebuilt from the project itself.
fn sources(p: &Project) -> SourceMap {
    let mut map = SourceMap::default();
    for (id, item) in &p.items {
        if let ItemKind::Media(m) = &item.kind
            && let MediaRef::Generator(g) = &m.media
        {
            map.0.insert(*id, Arc::new(GeneratorSource::new(g.clone(), 640, 360, RATE, Tick(10 * TICKS_PER_SECOND))) as SharedSource);
        }
    }
    map
}

/// SHA-256 over every rendered frame (a measurement of identity, not a gate on a platform hash).
fn render_hash(p: &Project, seq: cfa4_project::ItemId) -> String {
    let map = sources(p);
    let mut h = Sha256::new();
    for f in 0..12 {
        let img = render_sequence(p, seq, RATE.tick_of(f), RenderOptions::default(), &map);
        for v in &img.px {
            h.update(v.to_bits().to_le_bytes());
        }
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn envelope(p: &Project) -> Value {
    serde_json::from_slice(&encode(p, false)).expect("the writer's own output is JSON")
}

/// The same project in an older shape: `(label, bytes)`.
fn old_files(p: &Project) -> Vec<(String, Vec<u8>)> {
    let env = envelope(p);
    let mut out = Vec::new();
    for k in 2..SCHEMA_VERSION {
        let mut v = env.clone();
        v["schema_version"] = Value::from(k);
        out.push((format!("envelope, schema {k}"), serde_json::to_vec(&v).expect("json")));
    }
    // before the envelope (schema 1): the bare project with a `version` field
    let mut bare = env["project"].clone();
    bare["version"] = Value::from(1);
    out.push(("bare project, schema 1".into(), serde_json::to_vec(&bare).expect("json")));
    // the upstream application's tag
    let mut up = env;
    up["format"] = Value::from(cfa4_project::legacy::PROJECT_FORMAT);
    out.push(("upstream format tag".into(), serde_json::to_vec(&up).expect("json")));
    out
}

#[test]
fn an_older_schema_project_opens_saves_and_reopens_without_loss_and_renders_the_same() {
    let (original, seq) = project();
    let want_json = original.to_json();
    let want_render = render_hash(&original, seq);
    let files = old_files(&original);
    assert!(files.len() >= 3, "the fixtures cover older schemas, the bare form and the upstream tag");

    for (label, bytes) in files {
        let opened = decode(&bytes).unwrap_or_else(|e| panic!("{label}: does not open: {e}"));
        let saved = encode(&opened.project, true);
        let reopened = decode(&saved).unwrap_or_else(|e| panic!("{label}: the saved file does not reopen: {e}"));

        let saved_env: Value = serde_json::from_slice(&saved).expect("json");
        assert_eq!(saved_env["format"], FORMAT_ID, "{label}: saved with the CFA4 tag");
        assert_eq!(saved_env["schema_version"], SCHEMA_VERSION, "{label}: saved at the current schema");

        assert_eq!(opened.project.to_json(), want_json, "{label}: a field was lost or changed on opening");
        assert_eq!(reopened.project.to_json(), want_json, "{label}: a field was lost or changed after save and reopen");
        assert_eq!(render_hash(&reopened.project, seq), want_render, "{label}: the render differs");
    }
}

/// Detection power: a changed field (here the project name) must make the comparison differ.
#[test]
fn the_check_catches_a_lost_field() {
    let (original, _) = project();
    let mut env = envelope(&original);
    env["project"]["name"] = Value::from("another name");
    let loaded = decode(&serde_json::to_vec(&env).expect("json")).expect("opens");
    assert_ne!(loaded.project.to_json(), original.to_json());
}
