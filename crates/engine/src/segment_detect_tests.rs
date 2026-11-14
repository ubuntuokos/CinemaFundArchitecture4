//! Segment proposals (K4, K5 of CFA4-LOSSLESS-MEDIA-TOOLS-001) on a generated movie with known
//! content: a demo scene with sound, one second of black without sound, another demo scene with
//! sound (ProRes + PCM, written with our own encoder).

use std::path::Path;
use std::sync::Arc;

use cfa4_media::generators::GeneratorSource;
use cfa4_media::{DemoScene, Generator, MediaSource};
use cfa4_project::{ItemId, ItemKind, Label, MediaClip, MediaRef, Project, SequenceSettings, TrackKind};
use cfa4_time::{FrameRate, Tick, TimeRange};
use serde_json::{Value, json};

use crate::Session;
use crate::media_test_util::{session_with, tmp_dir};

const RATE: FrameRate = FrameRate::FPS_24;

/// A 24 fps ProRes movie of `shots` (generator, frames, with its sound) one after the other.
fn make_movie(path: &Path, shots: &[(Generator, i64, bool)]) {
    let (w, h) = (160, 90);
    let mut p = Project::new("fixture");
    let seq = p.new_sequence("s", SequenceSettings { width: w, height: h, frame_rate: RATE, ..Default::default() }, 1, 1, None);
    let mut sources: Vec<(ItemId, cfa4_media::SharedSource)> = Vec::new();
    let mut at = Tick::ZERO;
    for (k, (g, frames, sound)) in shots.iter().enumerate() {
        let dur = RATE.tick_of(*frames);
        let src = GeneratorSource::new(g.clone(), w, h, RATE, RATE.tick_of(200));
        let clip = MediaClip {
            media: MediaRef::Generator(g.clone()),
            info: src.info().clone(),
            interpret: Default::default(),
            mark_in: None,
            mark_out: None,
            markers: vec![],
            offline: false,
            proxy: None,
            identity: None,
        };
        let item = p.add_item(&format!("shot{k}"), Label::Iris, ItemKind::Media(clip), None);
        let r = TimeRange::new(RATE.tick_of(10 * k as i64), dur);
        let mut v = p.make_track_item(item, TrackKind::Video, at, r, RATE).unwrap();
        for e in &mut v.effects {
            cfa4_project::resolve_auto_points(e, (w, h), (w, h));
        }
        let q = p.sequence_mut(seq).unwrap();
        q.video_tracks[0].items.push(v);
        if *sound {
            let a = p.make_track_item(item, TrackKind::Audio, at, r, RATE).unwrap();
            p.sequence_mut(seq).unwrap().audio_tracks[0].items.push(a);
        }
        sources.push((item, Arc::new(src)));
        at += dur;
    }
    let settings = cfa4_export::ExportSettings { format: cfa4_export::Format::ProRes, path: path.to_string_lossy().into_owned(), ..Default::default() };
    let provider = move |id: ItemId| sources.iter().find(|(i, _)| *i == id).map(|(_, s)| s.clone());
    cfa4_export::export(&Arc::new(p), seq, &settings, &provider, &Default::default()).unwrap();
}

/// Ocean Sunset with sound (frames 0–24), black without sound (24–48), Forest with sound (48–72).
fn session(tag: &str) -> (Session, ItemId, std::path::PathBuf) {
    let dir = tmp_dir(tag);
    let path = dir.join("clip.mov");
    make_movie(
        &path,
        &[(Generator::Demo(DemoScene::OceanSunset), 24, true), (Generator::BlackVideo, 24, false), (Generator::Demo(DemoScene::Forest), 24, true)],
    );
    let (s, items, _) = session_with(&[path.as_path()]);
    (s, items[0], dir)
}

fn spans(r: &Value) -> Vec<(i64, i64)> {
    r["segments"].as_array().unwrap_or_else(|| panic!("no segments: {r}")).iter().map(|x| (x["in"].as_i64().unwrap(), x["out"].as_i64().unwrap())).collect()
}

fn frames(a: i64, b: i64) -> (i64, i64) {
    (RATE.tick_of(a).0, RATE.tick_of(b).0)
}

#[test]
fn scenes_black_and_silence_are_proposed_without_changing_the_project() {
    let (mut s, item, dir) = session("segdetect-all");
    let undo = s.history.undo.len();
    let r = s.execute("segments.detectScenes", json!({"item": item.0, "wait": true})).unwrap();
    assert_eq!(r["state"], "done", "{r}");
    assert_eq!(spans(&r), [frames(0, 24), frames(24, 48), frames(48, 72)], "{r}");
    assert_eq!(r["segments"][0]["name"], "Scene 01");
    let r = s.execute("segments.detectBlack", json!({"item": item.0, "wait": true})).unwrap();
    assert_eq!(spans(&r), [frames(24, 48)], "{r}");
    let r = s.execute("segments.detectSilence", json!({"item": item.0, "minSeconds": 0.5, "wait": true})).unwrap();
    let silent = spans(&r);
    assert_eq!(silent.len(), 1, "{r}");
    // the silent second, to within one 10 ms block on each side
    let block = Tick::from_seconds_f64(0.01).0;
    assert!((silent[0].0 - frames(24, 48).0).abs() <= block && (silent[0].1 - frames(24, 48).1).abs() <= block, "{r}");
    // the analyses never change the project
    assert_eq!(s.history.undo.len(), undo);
    // the proposal can be read again by its job
    let job = r["job"].as_u64().unwrap();
    assert_eq!(s.execute("segments.proposal", json!({"job": job})).unwrap()["segments"], r["segments"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_shorter_minimum_or_a_narrower_span_changes_the_proposal() {
    let (mut s, item, dir) = session("segdetect-span");
    // black shorter than the minimum is not proposed
    let r = s.execute("segments.detectBlack", json!({"item": item.0, "minSeconds": 2.0, "wait": true})).unwrap();
    assert_eq!(spans(&r), []);
    // only the span asked for is analysed
    let r = s.execute("segments.detectScenes", json!({"item": item.0, "in": {"frame": 30}, "out": {"frame": 60}, "wait": true})).unwrap();
    assert_eq!(spans(&r), [frames(30, 48), frames(48, 60)], "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_accepted_proposal_becomes_segments_in_one_undo_step() {
    let (mut s, item, dir) = session("segdetect-add");
    let r = s.execute("segments.detectScenes", json!({"item": item.0, "wait": true})).unwrap();
    let mut list = r["segments"].as_array().unwrap().clone();
    // the user corrects the proposal: renames the first shot and drops the last one
    list[0]["name"] = json!("Sunset");
    list.pop();
    let undo = s.history.undo.len();
    let added = s.execute("segments.add", json!({"item": item.0, "segments": list})).unwrap();
    assert_eq!(added["markers"].as_array().unwrap().len(), 2, "{added}");
    assert_eq!(s.history.undo.len(), undo + 1);
    assert_eq!(s.history.undo.last().unwrap().0, "Add Segments");
    let segs = s.execute("segments.list", json!({"item": item.0})).unwrap();
    let names: Vec<&str> = segs["segments"].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Sunset", "Scene 02"]);
    // adding the same segments again changes nothing
    let again = s.execute("segments.add", json!({"item": item.0, "segments": list})).unwrap();
    assert_eq!(again["skipped"], 2, "{again}");
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.execute("segments.list", json!({"item": item.0})).unwrap()["segments"], json!([]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_requests_are_refused() {
    let (mut s, item, dir) = session("segdetect-bad");
    let i = item.0;
    for (cmd, p) in [
        ("segments.detectScenes", json!({"item": i, "sensitivity": 101})),
        ("segments.detectScenes", json!({"item": i, "in": {"frame": 50}, "out": {"frame": 10}})),
        ("segments.detectBlack", json!({"item": i, "pixelThreshold": -0.1})),
        ("segments.detectSilence", json!({"item": i, "thresholdDb": 3})),
        ("segments.detectSilence", json!({"item": i, "minSeconds": "x"})),
        ("segments.detectScenes", json!({"item": 999_999})),
        ("segments.detectScenes", json!({})),
        ("segments.proposal", json!({"job": 424_242})),
        ("segments.proposal", json!({})),
        ("segments.add", json!({"item": i, "segments": []})),
        ("segments.add", json!({"item": i, "segments": [{"in": 0}]})),
        ("segments.add", json!({"item": i, "segments": [{"in": 10, "out": 5}]})),
        ("segments.add", json!({"item": i, "segments": [{"in": 0, "out": RATE.tick_of(5).0, "color": "Plaid"}]})),
        ("segments.add", json!({"item": i, "segments": [{"in": 0, "out": RATE.tick_of(5).0, "name": 7}]})),
        ("segments.add", json!({"item": i, "segments": "all"})),
    ] {
        assert!(s.execute(cmd, p.clone()).is_err(), "{cmd} accepted {p}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hostile_values_never_panic() {
    let (mut s, item, dir) = session("segdetect-hostile");
    let values = [json!(null), json!(-1), json!(i64::MAX), json!(i64::MIN), json!(u64::MAX), json!(1e300), json!("x"), json!({"frame": i64::MIN}), json!([1])];
    for v in &values {
        for cmd in ["segments.detectScenes", "segments.detectBlack", "segments.detectSilence"] {
            let _ = s.execute(cmd, json!({"item": item.0, "in": v, "out": {"frame": 2}, "minSeconds": v, "wait": true}));
        }
        let _ = s.execute("segments.add", json!({"item": item.0, "segments": [{"in": v, "out": v, "name": "\u{0}"}]}));
        let _ = s.execute("segments.proposal", json!({"job": v}));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_running_analysis_reports_its_state_and_can_be_stopped() {
    let (mut s, item, dir) = session("segdetect-job");
    let r = s.execute("segments.detectBlack", json!({"item": item.0})).unwrap();
    let job = r["job"].as_u64().unwrap();
    s.execute("jobs.cancel", json!({"job": job})).unwrap();
    // it ends either stopped or (when it finished first) done
    let t0 = std::time::Instant::now();
    loop {
        let p = s.execute("segments.proposal", json!({"job": job})).unwrap();
        match p["state"].as_str().unwrap() {
            "running" if t0.elapsed().as_secs() < 60 => std::thread::sleep(std::time::Duration::from_millis(20)),
            st => {
                assert!(st == "stopped" || st == "done", "{p}");
                break;
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_subclip_is_analysed_and_segmented_only_within_its_range() {
    let (mut s, item, dir) = session("segdetect-subclip");
    // a subclip of frames 30–60 that does not restrict trims (the Source monitor shows all media)
    let range = TimeRange::from_bounds(RATE.tick_of(30), RATE.tick_of(60));
    let sub = s.edit("sub", |pr, _| Ok(pr.add_item("sub", Label::Iris, ItemKind::Subclip { parent: item, range, restrict_trims: false }, None))).unwrap();
    let r = s.execute("segments.detectScenes", json!({"item": sub.0, "wait": true})).unwrap();
    assert_eq!(spans(&r), [frames(30, 48), frames(48, 60)], "{r}");
    let added = s.execute("segments.add", json!({"item": sub.0, "segments": [{"in": 0, "out": RATE.tick_of(72).0}]})).unwrap();
    assert_eq!(added["markers"].as_array().unwrap().len(), 1, "{added}");
    let segs = s.execute("segments.list", json!({"item": item.0})).unwrap();
    assert_eq!(spans(&segs), [frames(30, 60)], "kept inside the subclip: {segs}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_short_last_audio_block_counts_only_its_own_samples() {
    use crate::segment_detect::silent_spans;
    // 48 kHz, blocks of 480 samples: one 24 fps frame (2 000 samples) is four full blocks and
    // one of 80 samples, which is shorter than a 0.05 s (2 400 sample) minimum
    let flags = [true; 5];
    let ends = [480, 960, 1440, 1920, 2000];
    assert!(silent_spans(&flags, &ends, 0, 480, 2400).is_empty());
    assert_eq!(silent_spans(&flags, &ends, 0, 480, 2000), vec![(0, 2000)]);
    assert_eq!(silent_spans(&[false, true, true, false], &[10, 20, 30, 40], 0, 10, 20), vec![(10, 30)]);
    assert!(silent_spans(&[true], &[], 0, 10, 1).is_empty());
}
