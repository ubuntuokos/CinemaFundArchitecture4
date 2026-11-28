//! Tests of [`crate::silence`]: Remove Silence on generated speech-and-silence audio.

use std::sync::Arc;

use cfa4_project::ClipId;
use cfa4_time::{TICKS_PER_SECOND, Tick};
use serde_json::{Value, json};

use super::*;
use crate::silence::SILENCE_COMMENT;

const SR: u32 = 48_000;

/// Audio (both channels alike) of `seconds` with a tone over each `(from, to)`.
fn wav(seconds: f64, tones: &[(f64, f64)]) -> Arc<[u8]> {
    let n = (seconds * SR as f64) as usize;
    let mut a: Vec<f32> = (0..n).map(|i| if i % 2 == 0 { 0.0001 } else { 0.0 }).collect();
    for &(from, to) in tones {
        for i in (from * SR as f64) as usize..((to * SR as f64) as usize).min(n) {
            a[i] = 0.3 * (i as f32 * 0.05).sin();
        }
    }
    // the writer takes interleaved stereo
    let st: Vec<f32> = a.iter().flat_map(|v| [*v, *v]).collect();
    crate::previews::write_wav_f32(&st, SR).into()
}

fn new_session() -> Session {
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "silence", "audio": 2, "video": 1})).unwrap();
    s
}

/// Place `bytes` on `track` at 0 s; returns the clip.
fn place(s: &mut Session, name: &str, bytes: Arc<[u8]>, track: &str) -> ClipId {
    let item = crate::commands::import_bytes(s, name, bytes, None).unwrap();
    let r = s.execute("timeline.place", json!({"item": item.0, "audioTrack": track, "seconds": 0.0})).unwrap();
    ClipId(r["clips"][0].as_u64().unwrap())
}

/// Speech 0.5–1.5 and 3.0–4.0 in 5.0 s: silences 0–0.5, 1.5–3.0, 4.0–5.0.
fn speech() -> Arc<[u8]> {
    wav(5.0, &[(0.5, 1.5), (3.0, 4.0)])
}

fn one_clip() -> (Session, ClipId) {
    let mut s = new_session();
    let c = place(&mut s, "/speech.wav", speech(), "A1");
    s.execute("timeline.select", json!({"clips": [c.0]})).unwrap();
    (s, c)
}

fn secs(v: &Value) -> (f64, f64) {
    (v["start"].as_i64().unwrap() as f64 / TICKS_PER_SECOND as f64, v["duration"].as_i64().unwrap() as f64 / TICKS_PER_SECOND as f64)
}

fn saved(s: &Session) -> String {
    serde_json::to_string(&*s.project).unwrap()
}

fn seq_len(s: &Session) -> i64 {
    s.active_sequence().unwrap().duration().0
}

fn silence_markers(s: &Session) -> Vec<(u64, i64, i64)> {
    s.active_sequence().unwrap().markers.iter().filter(|m| m.comment.starts_with(SILENCE_COMMENT)).map(|m| (m.id.0, m.start.0, m.duration.0)).collect()
}

const PARAMS: fn() -> Value = || json!({"minSilence": TICKS_PER_SECOND / 2, "padding": TICKS_PER_SECOND / 10});

fn with_mode(mode: &str) -> Value {
    let mut p = PARAMS();
    p["mode"] = json!(mode);
    p
}

#[test]
fn detection_finds_the_known_silences_and_a_query_changes_nothing() {
    let (mut s, _) = one_clip();
    let before = saved(&s);
    let n0 = s.history.undo.len();
    let r = s.execute("audio.detectSpeech", with_mode("query")).unwrap();
    let regions = r["regions"].as_array().unwrap();
    assert_eq!(regions.len(), 3, "{r}");
    // padding 0.1 s comes off the sides next to speech (not at the clip's own edges), ±1 window
    let near = |a: f64, b: f64| (a - b).abs() <= 0.05;
    let (s0, d0) = secs(&regions[0]);
    let (s1, d1) = secs(&regions[1]);
    let (s2, d2) = secs(&regions[2]);
    assert!(near(s0, 0.0) && near(s0 + d0, 0.4), "{r}");
    assert!(near(s1, 1.6) && near(s1 + d1, 2.9), "{r}");
    assert!(near(s2, 4.1) && near(s2 + d2, 5.0), "{r}");
    assert_eq!(saved(&s), before, "a query changes nothing");
    assert_eq!(s.history.undo.len(), n0, "and is no undo step");
    assert!(regions.iter().all(|x| x["keep"] == json!(false)));
}

#[test]
fn preview_only_adds_markers_and_reanalysis_replaces_them_but_not_other_markers() {
    let (mut s, c) = one_clip();
    s.execute("markers.add", json!({"seconds": 1.0, "name": "mine"})).unwrap();
    let before = s.active_sequence().unwrap().clone();
    let n0 = s.history.undo.len();
    let r = s.execute("audio.detectSpeech", PARAMS()).unwrap();
    assert_eq!(r["regions"].as_array().unwrap().len(), 3);
    assert_eq!(s.history.undo.len(), n0 + 1, "one undo step");
    let after = s.active_sequence().unwrap();
    assert_eq!(after.duration(), before.duration(), "the timeline is unchanged");
    assert_eq!(after.audio_tracks, before.audio_tracks);
    assert_eq!(silence_markers(&s).len(), 3);
    assert!(after.markers.iter().any(|m| m.name == "mine"));
    // another analysis with a longer minimum replaces the markers
    s.execute("audio.detectSpeech", json!({"clips": [c.0], "minSilence": 2 * TICKS_PER_SECOND})).unwrap();
    assert!(silence_markers(&s).is_empty());
    assert!(s.active_sequence().unwrap().markers.iter().any(|m| m.name == "mine"));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(silence_markers(&s).len(), 3);
}

#[test]
fn removal_ripples_in_one_undo_step_and_undo_redo_are_exact() {
    let (mut s, _) = one_clip();
    s.execute("audio.detectSpeech", PARAMS()).unwrap();
    let original = saved(&s);
    let len = seq_len(&s);
    let ticks: i64 = silence_markers(&s).iter().map(|m| m.2).sum();
    let n0 = s.history.undo.len();
    let r = s.execute("clip.removeSilence", json!({})).unwrap();
    assert_eq!(r["removed"], json!(3));
    assert_eq!(r["ticks"], json!(ticks));
    assert_eq!(s.history.undo.len(), n0 + 1, "one undo step");
    assert_eq!(seq_len(&s), len - ticks, "the sequence is shorter by exactly the removed silence");
    assert!(silence_markers(&s).is_empty(), "the finished regions' markers are gone");
    let removed = saved(&s);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(saved(&s), original, "undo is byte-identical");
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(saved(&s), removed, "redo is byte-identical");
}

#[test]
fn lift_leaves_the_gaps_and_the_length() {
    let (mut s, _) = one_clip();
    s.execute("audio.detectSpeech", PARAMS()).unwrap();
    let len = seq_len(&s);
    s.execute("clip.removeSilence", json!({"ripple": false})).unwrap();
    let seq = s.active_sequence().unwrap();
    assert!(seq.duration().0 <= len && seq.duration().0 >= len - TICKS_PER_SECOND, "the last silence at the end is gone, the rest stays");
    let clips = &seq.audio_tracks[0].items;
    assert!(clips.len() >= 2, "{} clips: the gaps split the clip", clips.len());
}

#[test]
fn kept_regions_are_skipped_and_mark_mode_deletes_nothing() {
    let (mut s, _) = one_clip();
    s.execute("audio.detectSpeech", PARAMS()).unwrap();
    let ms = silence_markers(&s);
    s.execute("silence.keep", json!({"ids": [ms[1].0], "keep": true})).unwrap();
    let list = s.execute("silence.regions", json!({})).unwrap();
    assert_eq!(list["regions"][1]["keep"], json!(true));
    assert_eq!(list["regions"][0]["keep"], json!(false));
    let before = saved(&s);
    let r = s.execute("clip.removeSilence", json!({"mode": "mark"})).unwrap();
    assert_eq!(r["removed"], json!(0));
    assert_eq!(saved(&s), before, "mark mode deletes nothing");
    let len = seq_len(&s);
    s.execute("clip.removeSilence", json!({})).unwrap();
    let gone = ms[0].2 + ms[2].2;
    assert_eq!(seq_len(&s), len - gone, "the kept region stays");
    assert_eq!(silence_markers(&s).len(), 1, "the kept marker stays");
    // not kept again: removable
    s.execute("silence.keep", json!({"ids": [silence_markers(&s)[0].0], "keep": false})).unwrap();
    s.execute("clip.removeSilence", json!({})).unwrap();
    assert!(silence_markers(&s).is_empty());
}

#[test]
fn a_moved_or_resized_marker_removes_the_changed_range() {
    let (mut s, _) = one_clip();
    s.execute("audio.detectSpeech", json!({"minSilence": TICKS_PER_SECOND, "padding": TICKS_PER_SECOND / 10})).unwrap();
    let ms = silence_markers(&s);
    assert_eq!(ms.len(), 2, "with a 1 s minimum the 1.5 s middle and the 1 s end remain, the 0.5 s start does not");
    let len = seq_len(&s);
    // shrink the middle region by half a second through the sequence markers
    let target = ms[0].0;
    {
        let seq = s.active_sequence().unwrap();
        assert!(seq.markers.iter().any(|m| m.id.0 == target));
    }
    s.execute("markers.edit", json!({"marker": target, "durationFrames": 12})).unwrap();
    let ms2 = silence_markers(&s);
    let before_total: i64 = ms2.iter().map(|m| m.2).sum();
    s.execute("clip.removeSilence", json!({})).unwrap();
    assert_eq!(seq_len(&s), len - before_total, "the removal follows the markers as they are now");
}

#[test]
fn removal_without_silence_markers_refuses_and_deletes_nothing() {
    let (mut s, _) = one_clip();
    let before = saved(&s);
    let why = (crate::commands::find("clip.removeSilence").unwrap().enabled)(&s).unwrap_err();
    assert!(why.contains("analyse"), "{why}");
    assert!(s.execute("clip.removeSilence", json!({})).is_err());
    assert!(s.execute("silence.keep", json!({"ids": [1], "keep": true})).is_err());
    assert_eq!(saved(&s), before);
}

#[test]
fn clearing_removes_only_the_silence_markers() {
    let (mut s, _) = one_clip();
    s.execute("markers.add", json!({"seconds": 1.0, "name": "mine"})).unwrap();
    s.execute("audio.detectSpeech", PARAMS()).unwrap();
    let len = seq_len(&s);
    let r = s.execute("silence.clear", json!({})).unwrap();
    assert_eq!(r["removed"], json!(3));
    assert!(silence_markers(&s).is_empty());
    assert!(s.active_sequence().unwrap().markers.iter().any(|m| m.name == "mine"));
    assert_eq!(seq_len(&s), len);
    assert!(s.execute("silence.clear", json!({})).is_err(), "nothing to clear");
}

#[test]
fn detection_never_deletes() {
    let (mut s, _) = one_clip();
    let before = s.active_sequence().unwrap().audio_tracks.clone();
    s.execute("audio.detectSpeech", PARAMS()).unwrap();
    assert_eq!(s.active_sequence().unwrap().audio_tracks, before);
}

#[test]
fn several_clips_give_only_the_time_where_all_are_silent() {
    let mut s = new_session();
    // A1 speaks 0.5–1.5 and 3.0–4.0; A2 speaks 1.5–3.0: nothing but the ends is silent in both
    let a = place(&mut s, "/a.wav", speech(), "A1");
    let b = place(&mut s, "/b.wav", wav(5.0, &[(1.5, 3.0)]), "A2");
    let r = s.execute("audio.detectSpeech", json!({"clips": [a.0, b.0], "mode": "query", "minSilence": TICKS_PER_SECOND / 4, "padding": 0})).unwrap();
    let regions: Vec<(f64, f64)> = r["regions"].as_array().unwrap().iter().map(secs).collect();
    // 0–0.5 (A1 silent, A2 silent), 4.0–5.0 (both silent); 1.5–3.0 is A2's speech
    assert_eq!(regions.len(), 2, "{r}");
    assert!(regions[0].0 < 0.05 && (regions[0].0 + regions[0].1 - 0.5).abs() < 0.05, "{regions:?}");
    assert!((regions[1].0 - 4.0).abs() < 0.05 && (regions[1].0 + regions[1].1 - 5.0).abs() < 0.05, "{regions:?}");
    // by scope
    let r2 = s.execute("audio.detectSpeech", json!({"scope": "sequence", "mode": "query", "minSilence": TICKS_PER_SECOND / 4, "padding": 0})).unwrap();
    assert_eq!(r2["regions"], r["regions"]);
    assert_eq!(r2["clips"], json!(2));
}

#[test]
fn a_batch_removal_is_one_undo_step_and_keeps_every_track_in_sync() {
    let mut s = new_session();
    let a = place(&mut s, "/a.wav", speech(), "A1");
    let b = place(&mut s, "/b.wav", speech(), "A2");
    let original = saved(&s);
    s.execute("audio.detectSpeech", json!({"clips": [a.0, b.0], "minSilence": TICKS_PER_SECOND / 2, "padding": TICKS_PER_SECOND / 10})).unwrap();
    let n0 = s.history.undo.len();
    s.execute("clip.removeSilence", json!({})).unwrap();
    assert_eq!(s.history.undo.len(), n0 + 1);
    let seq = s.active_sequence().unwrap();
    let ends: Vec<_> = seq.audio_tracks.iter().take(2).map(|t| t.end()).collect();
    assert_eq!(ends[0], ends[1], "both tracks shrink alike");
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(saved(&s), original);
}

#[test]
fn unsupported_clips_and_hostile_parameters_are_errors_not_panics() {
    let (mut s, c) = one_clip();
    let before = saved(&s);
    // speed-changed
    s.execute("clip.speedDuration", json!({"clips": [c.0], "speed": 200})).unwrap();
    let spd = saved(&s);
    assert!(s.execute("audio.detectSpeech", json!({"clips": [c.0]})).is_err());
    assert!(s.execute("audio.detectSpeech", json!({})).is_err(), "selection has only unsupported clips");
    s.execute("edit.undo", json!({})).unwrap();
    assert_ne!(spd, saved(&s));
    assert_eq!(saved(&s), before);
    for bad in [
        json!({"detector": "nope"}),
        json!({"mode": "nope"}),
        json!({"scope": "nope"}),
        json!({"clips": "x"}),
        json!({"clips": [999_999]}),
        json!({"thresholdDb": "loud"}),
        json!({"thresholdDb": 5.0}),
        json!({"minSilence": -1}),
        json!({"padding": 1.5}),
    ] {
        assert!(s.execute("audio.detectSpeech", bad.clone()).is_err(), "{bad}");
    }
    assert_eq!(saved(&s), before, "nothing changed");
    // a track scope with a good id works; a clip-less sequence does not
    // without a track the selected clips' tracks are used; with nothing selected it is an error
    s.execute("timeline.select", json!({"clips": []})).unwrap();
    assert!(s.execute("audio.detectSpeech", json!({"scope": "track"})).is_err());
    let tid = s.active_sequence().unwrap().audio_tracks[0].id.0;
    assert!(s.execute("audio.detectSpeech", json!({"scope": "track", "track": tid, "mode": "query"})).is_ok());
    // the selected clip's own track
    s.execute("timeline.select", json!({"clips": [c.0]})).unwrap();
    let r = s.execute("audio.detectSpeech", json!({"scope": "track", "mode": "query"})).unwrap();
    assert_eq!(r["clips"], json!(1));
    let mut empty = new_session();
    assert!(empty.execute("audio.detectSpeech", json!({"scope": "sequence"})).is_err());
    assert!(empty.execute("clip.removeSilence", json!({})).is_err());
}

#[test]
fn all_speech_and_all_silence_give_no_regions() {
    let mut s = new_session();
    let c = place(&mut s, "/talk.wav", wav(3.0, &[(0.0, 3.0)]), "A1");
    let r = s.execute("audio.detectSpeech", json!({"clips": [c.0], "mode": "query"})).unwrap();
    assert!(r["regions"].as_array().unwrap().is_empty(), "{r}");
    let mut s = new_session();
    let c = place(&mut s, "/quiet.wav", wav(3.0, &[]), "A1");
    let r = s.execute("audio.detectSpeech", json!({"clips": [c.0], "mode": "query"})).unwrap();
    assert!(r["regions"].as_array().unwrap().is_empty(), "{r}");
    // preview of nothing adds no marker
    let r = s.execute("audio.detectSpeech", json!({"clips": [c.0]})).unwrap();
    assert!(r["regions"].as_array().unwrap().is_empty());
    assert!(silence_markers(&s).is_empty());
}

#[test]
fn markers_and_the_kept_flag_survive_save_and_reload() {
    let (mut s, _) = one_clip();
    s.execute("audio.detectSpeech", PARAMS()).unwrap();
    let ms = silence_markers(&s);
    s.execute("silence.keep", json!({"ids": [ms[0].0], "keep": true})).unwrap();
    let text = saved(&s);
    let back: cfa4_project::Project = serde_json::from_str(&text).unwrap();
    let seq = back.sequences().find_map(|i| match &i.kind {
        cfa4_project::ItemKind::Sequence(q) => Some(q.clone()),
        _ => None,
    });
    let seq = seq.expect("a sequence");
    let got: Vec<_> = seq.markers.iter().filter(|m| m.comment.starts_with(SILENCE_COMMENT)).collect();
    assert_eq!(got.len(), 3);
    assert!(got.iter().any(|m| m.comment.contains("[silence:keep]")));
}

/// The whole sequence's audio mix (left channel) through the real mixer.
fn mix_left(s: &Session) -> Vec<f32> {
    let seq = s.active_sequence().unwrap();
    let sr = i64::from(seq.settings.sample_rate);
    let frames = seq.duration().to_units_floor(sr).max(0) as usize;
    let provider = s.media.full_res_provider(s.project.clone(), s.services.clone());
    cfa4_render::audio::mix_sequence(&s.project, seq, 0, frames, &provider).channels[0].clone()
}

/// The acceptance test of plan CFA4-V1-SILENCE-REMOVAL-001 (6.1): (a) picture and sound stay in
/// sync and the sequence shrinks by exactly the removed silence, (b) removal, undo and redo are
/// single steps and byte-exact, (c) the mix after the removal is the mix before with exactly the
/// removed samples taken out, and what was taken out was silent.
#[test]
fn v1_silence_removal_acceptance() {
    let detect = PARAMS();
    acceptance(speech(), 5, detect, |regions| assert_eq!(regions, 3), 0.001);
}

/// The acceptance checks (a), (b), (c) on `audio` (`seconds` long, a whole number) with the given
/// `audio.detectSpeech` parameters; `regions` checks how many regions were found, and what is
/// removed must stay below `silent_peak` in level.
fn acceptance(audio_bytes: Arc<[u8]>, seconds: i64, detect: Value, regions: impl Fn(usize), silent_peak: f32) {
    use cfa4_media::{Generator, generators::GeneratorSource};
    use cfa4_project::Label;
    use cfa4_time::FrameRate;

    // 25 fps at 48 kHz is 1920 samples per frame: every cut falls on a whole sample
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "acceptance", "width": 64, "height": 36, "fps": 25.0, "audio": 2, "video": 1})).unwrap();
    let audio = place(&mut s, "/speech.wav", audio_bytes, "A1");
    let mut p = (*s.project).clone();
    let matte = crate::demo::add_generator(
        &mut p,
        &s.media,
        GeneratorSource::new(Generator::ColorMatte { color: [0.2, 0.5, 0.8, 1.0] }, 64, 36, FrameRate::FPS_25, Tick(seconds * TICKS_PER_SECOND)),
        "Matte",
        Label::Iris,
        None,
    );
    s.project = Arc::new(p);
    s.execute("timeline.place", json!({"item": matte.0, "videoTrack": "V1", "seconds": 0.0})).unwrap();
    s.execute("timeline.select", json!({"clips": [audio.0]})).unwrap();
    assert_eq!(seq_len(&s), seconds * TICKS_PER_SECOND);

    s.execute("audio.detectSpeech", detect).unwrap();
    let regions_found = silence_markers(&s);
    regions(regions_found.len());
    let regions = regions_found;
    let removed_ticks: i64 = regions.iter().map(|m| m.2).sum();
    let sr = 48_000i64;
    let ranges: Vec<(usize, usize)> = regions.iter().map(|m| (Tick(m.1).to_units_floor(sr) as usize, Tick(m.1 + m.2).to_units_floor(sr) as usize)).collect();
    let before_mix = mix_left(&s);
    let before_state = saved(&s);
    let steps = s.history.undo.len();

    s.execute("clip.removeSilence", json!({})).unwrap();

    // (a) sync: the picture was cut with the sound, the length is exact
    assert_eq!(s.history.undo.len(), steps + 1, "one undo step");
    assert_eq!(seq_len(&s), seconds * TICKS_PER_SECOND - removed_ticks);
    let seq = s.active_sequence().unwrap();
    let span = |t: &cfa4_project::Track| t.items.iter().map(|i| (i.start.0, i.duration.0)).collect::<Vec<_>>();
    assert!(!seq.video_tracks[0].items.is_empty());
    assert_eq!(span(&seq.video_tracks[0]), span(&seq.audio_tracks[0]), "the picture is cut exactly where the sound is");
    let after_state = saved(&s);

    // (c) the mix: before, with the removed samples taken out, is the mix after
    let after_mix = mix_left(&s);
    let mut expected = Vec::with_capacity(before_mix.len());
    let mut at = 0usize;
    for &(a, b) in &ranges {
        expected.extend_from_slice(&before_mix[at..a]);
        assert!(before_mix[a..b].iter().all(|v| v.abs() < silent_peak), "what is removed was silent");
        at = b;
    }
    expected.extend_from_slice(&before_mix[at..]);
    assert_eq!(after_mix.len(), expected.len());
    assert_eq!(after_mix, expected, "the kept audio is untouched, sample for sample");

    // (b) undo and redo are byte-exact single steps
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(saved(&s), before_state);
    assert_eq!(mix_left(&s), before_mix);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(saved(&s), after_state);
}

/// The built-in voice at 48 kHz (linear resampling) for the Silero tests.
#[cfg(feature = "silero")]
fn voice_48k() -> Vec<f32> {
    let voice = cfa4_tts::voice(cfa4_tts::default_voice_id()).expect("built-in voice");
    let a = voice.synthesize(cfa4_tts::SAMPLE_SENTENCE, &cfa4_tts::Params::default()).expect("synthesis");
    let ratio = f64::from(a.sample_rate) / f64::from(SR);
    let n = (a.samples.len() as f64 / ratio) as usize;
    (0..n)
        .map(|i| {
            let x = i as f64 * ratio;
            let (i0, f) = (x as usize, (x - x.floor()) as f32);
            let (a0, a1) = (a.samples.get(i0).copied().unwrap_or(0.0), a.samples.get(i0 + 1).copied().unwrap_or(0.0));
            a0 + (a1 - a0) * f
        })
        .collect()
}

/// `1 s` silence, the voice, `2 s` silence, the voice, silence up to a whole number of seconds.
#[cfg(feature = "silero")]
fn voice_and_silence() -> (Arc<[u8]>, i64) {
    let v = voice_48k();
    let mut a = vec![0.0f32; SR as usize];
    a.extend(&v);
    a.extend(vec![0.0f32; 2 * SR as usize]);
    a.extend(&v);
    let seconds = (a.len() as i64 + 2 * SR as i64 + SR as i64 - 1) / SR as i64; // at least 2 s of tail
    a.resize(seconds as usize * SR as usize, 0.0);
    let st: Vec<f32> = a.iter().flat_map(|x| [*x, *x]).collect();
    (crate::previews::write_wav_f32(&st, SR).into(), seconds)
}

/// V1-B acceptance (plan CFA4-V1-SILENCE-REMOVAL-001, 12.4): the same checks (a), (b), (c) with
/// the Silero detector on the built-in voice. The model is never downloaded by a test: it runs
/// when the model is in `<data dir>/models` (`CFA4_DATA_DIR=<dir>`, see `docs/testing.md`) and
/// prints SKIPPED otherwise.
#[cfg(feature = "silero")]
#[test]
fn v1_silence_removal_acceptance_with_the_speech_model() {
    let installed = crate::transcript::models_dir().is_some_and(|d| cfa4_speech::vad::silero::installed(&d));
    if !installed {
        eprintln!("SKIPPED: the Silero VAD model is not in the model store (set CFA4_DATA_DIR to a directory with models/silero-vad/)");
        return;
    }
    let (bytes, seconds) = voice_and_silence();
    let detect = json!({"detector": "silero", "minSilence": TICKS_PER_SECOND / 2, "padding": TICKS_PER_SECOND / 10});
    // the leading second, the 2 s gap and the tail are silence to the model; the voice's own
    // breathy fade-out may count as speech or not, so what is removed only has to be quiet
    acceptance(bytes, seconds, detect, |n| assert!((2..=4).contains(&n), "{n} silent regions"), 0.05);
}

/// Wait for a job to finish; its `result` as JSON.
fn finished(s: &mut Session, job: u64) -> Value {
    for _ in 0..400 {
        let list = s.execute("jobs.list", json!({})).unwrap();
        if let Some(j) = list.as_array().unwrap().iter().find(|j| j["id"] == json!(job))
            && j["finished"] == json!(true)
        {
            return j["result"].clone();
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("the job did not finish");
}

#[test]
fn a_background_analysis_is_a_job_and_marks_exactly_what_the_direct_run_does() {
    let mut s = new_session();
    let a = place(&mut s, "/a.wav", speech(), "A1");
    let b = place(&mut s, "/b.wav", wav(5.0, &[(1.5, 3.0)]), "A2");
    let p = json!({"clips": [a.0, b.0], "minSilence": TICKS_PER_SECOND / 4, "padding": 0});
    let direct = s.execute("audio.detectSpeech", p.clone()).unwrap();
    let want = silence_markers(&s);
    s.execute("silence.clear", json!({})).unwrap();
    let before = saved(&s);
    let mut bg = p.clone();
    bg["background"] = json!(true);
    let r = s.execute("audio.detectSpeech", bg).unwrap();
    let job = r["job"].as_u64().unwrap();
    assert_eq!(saved(&s), before, "starting the job changes nothing");
    assert!(finished(&mut s, job)["error"].is_null());
    assert_eq!(saved(&s), before, "neither does finishing it");
    let marked = s.execute("silence.mark", json!({"job": job})).unwrap();
    assert_eq!(marked["regions"].as_array().unwrap().len(), direct["regions"].as_array().unwrap().len());
    let got: Vec<(i64, i64)> = silence_markers(&s).iter().map(|m| (m.1, m.2)).collect();
    assert_eq!(got, want.iter().map(|m| (m.1, m.2)).collect::<Vec<_>>());
    assert!(s.execute("silence.mark", json!({"job": job})).is_err(), "taken once");
    assert!(s.execute("silence.mark", json!({"job": 99_999})).is_err());
    assert!(s.execute("silence.mark", json!({})).is_err());
}

#[test]
fn a_cancelled_analysis_leaves_the_sequence_alone_and_cannot_be_marked() {
    let mut s = new_session();
    // eight pieces of audio to read: the cancel arrives long before the end
    let c = place(&mut s, "/long.wav", wav(240.0, &[(1.0, 2.0)]), "A1");
    let before = saved(&s);
    let r = s.execute("audio.detectSpeech", json!({"clips": [c.0], "background": true})).unwrap();
    let job = r["job"].as_u64().unwrap();
    s.execute("jobs.cancel", json!({"job": job})).unwrap();
    let result = finished(&mut s, job);
    assert!(result["error"].as_str().is_some_and(|e| e.contains("cancel")), "{result}");
    assert!(s.execute("silence.mark", json!({"job": job})).is_err());
    assert_eq!(saved(&s), before);
    assert!(silence_markers(&s).is_empty());
}

#[test]
fn silero_model_is_shown_first_and_downloaded_only_after_acceptance() {
    let mut s = new_session();
    let info = s.execute("speech.vadModel", json!({})).unwrap();
    assert_eq!(info["id"], "silero-vad");
    assert!(info["license"].as_str().unwrap().starts_with("MIT"));
    assert!(info["size"].as_u64().unwrap() > 1_000_000);
    assert!(info["source"].as_str().unwrap().starts_with("https://github.com/snakers4/silero-vad"));
    // the licence is not accepted: no download, whatever the build, and nothing else downloads
    for p in [json!({}), json!({"licenseAccepted": false}), json!({"licenseAccepted": "yes"})] {
        let e = s.execute("speech.downloadVadModel", p.clone()).unwrap_err().to_string();
        assert!(e.contains("licence") || e.contains("not available"), "{p}: {e}");
    }
}

/// Without the model (or the feature) `silero` is an error that says what to do; it never falls
/// back to the energy detector silently.
#[cfg(not(feature = "silero"))]
#[test]
fn silero_detector_without_the_feature_is_an_error_not_a_fallback() {
    let (mut s, c) = one_clip();
    let before = saved(&s);
    let e = s.execute("audio.detectSpeech", json!({"clips": [c.0], "detector": "silero"})).unwrap_err().to_string();
    assert!(e.contains("silero"), "{e}");
    assert_eq!(saved(&s), before);
}
