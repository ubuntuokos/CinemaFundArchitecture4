//! OpenTimelineIO: a hand-written plain OTIO document (from the public OTIO schema docs) with gaps,
//! transitions, markers and legacy `Clip.1`; lossless round trips through `metadata.cfa4`.

mod support;

use cfa4_interchange::{ExportOptions, Format, detect, import};
use cfa4_media::Generator;
use cfa4_project::{Keyframe, Label, Marker, MarkerId, MarkerKind, MediaRef, Param, ParamValue, Project, TrackKind, TransitionAlign};
use cfa4_time::{FrameRate, TICKS_PER_SECOND, Tick};
use proptest::prelude::*;
use support::*;

fn rt(v: f64, rate: f64) -> String {
    format!(r#"{{"OTIO_SCHEMA": "RationalTime.1", "rate": {rate}, "value": {v}}}"#)
}
fn tr(s: f64, d: f64, rate: f64) -> String {
    format!(r#"{{"OTIO_SCHEMA": "TimeRange.1", "start_time": {}, "duration": {}}}"#, rt(s, rate), rt(d, rate))
}

fn doc() -> String {
    let r = 24.0;
    format!(
        r#"{{
  "OTIO_SCHEMA": "Timeline.1",
  "name": "River Story",
  "global_start_time": {gst},
  "metadata": {{}},
  "tracks": {{
    "OTIO_SCHEMA": "Stack.1",
    "name": "tracks",
    "children": [
      {{
        "OTIO_SCHEMA": "Track.1",
        "name": "V1",
        "kind": "Video",
        "children": [
          {{"OTIO_SCHEMA": "Gap.1", "name": "", "source_range": {gap}}},
          {{"OTIO_SCHEMA": "Transition.1", "name": "Fade In", "transition_type": "SMPTE_Dissolve", "in_offset": {z}, "out_offset": {t6}}},
          {{
            "OTIO_SCHEMA": "Clip.2",
            "name": "river_a",
            "source_range": {src_a},
            "media_references": {{"DEFAULT_MEDIA": {{"OTIO_SCHEMA": "ExternalReference.1", "target_url": "file:///footage/river%20a.mov", "available_range": {avail_a}}}}},
            "active_media_reference_key": "DEFAULT_MEDIA",
            "markers": [{{"OTIO_SCHEMA": "Marker.2", "name": "kingfisher", "color": "RED", "comment": "", "marked_range": {mk}}}]
          }},
          {{"OTIO_SCHEMA": "Transition.1", "name": "", "transition_type": "SMPTE_Dissolve", "in_offset": {t6}, "out_offset": {t6}}},
          {{
            "OTIO_SCHEMA": "Clip.1",
            "name": "river_b",
            "source_range": {src_b},
            "media_reference": {{"OTIO_SCHEMA": "ExternalReference.1", "target_url": "clips/river_b.mov", "available_range": null}},
            "effects": [{{"OTIO_SCHEMA": "LinearTimeWarp.1", "effect_name": "LinearTimeWarp", "time_scalar": 2.0}}]
          }},
          {{"OTIO_SCHEMA": "Gap.1", "source_range": {gap2}}},
          {{
            "OTIO_SCHEMA": "Clip.2",
            "name": "lost",
            "source_range": {src_c},
            "media_references": {{"DEFAULT_MEDIA": {{"OTIO_SCHEMA": "MissingReference.1", "name": "lost_shot.mov"}}}},
            "active_media_reference_key": "DEFAULT_MEDIA",
            "enabled": false
          }},
          {{"OTIO_SCHEMA": "Transition.1", "name": "Fade Out", "transition_type": "SMPTE_Dissolve", "in_offset": {t6}, "out_offset": {z}}}
        ]
      }},
      {{
        "OTIO_SCHEMA": "Track.1",
        "name": "A1",
        "kind": "Audio",
        "children": [
          {{
            "OTIO_SCHEMA": "Clip.2",
            "name": "ambience",
            "source_range": {src_amb},
            "media_references": {{"DEFAULT_MEDIA": {{"OTIO_SCHEMA": "ExternalReference.1", "target_url": "file:///footage/amb.wav"}}}},
            "active_media_reference_key": "DEFAULT_MEDIA",
            "effects": [{{"OTIO_SCHEMA": "Effect.1", "effect_name": "Reverb"}}]
          }}
        ]
      }}
    ],
    "markers": [{{"OTIO_SCHEMA": "Marker.2", "name": "act two", "color": "BLUE", "comment": "", "marked_range": {smk}}}]
  }}
}}"#,
        gst = rt(86_400.0, r),
        z = rt(0.0, r),
        t6 = rt(6.0, r),
        gap = tr(0.0, 12.0, r),
        avail_a = tr(86_400.0, 2400.0, r),
        src_a = tr(86_448.0, 48.0, r),
        mk = tr(86_460.0, 0.0, r),
        src_b = tr(10.0, 36.0, r),
        gap2 = tr(0.0, 24.0, r),
        src_c = tr(0.0, 30.0, r),
        src_amb = tr(0.0, 150.0, r),
        smk = tr(60.0, 0.0, r),
    )
}

#[test]
fn golden_gaps_and_transitions() {
    let d = doc();
    assert_eq!(detect(d.as_bytes(), Some("otio")), Some(Format::Otio));
    let (imp, rep) = import(d.as_bytes(), Format::Otio, Some("/proj")).unwrap();
    let p = &imp.project;
    let sid = only_seq(&imp);
    let s = p.sequence(sid).unwrap();
    let r = FrameRate::FPS_24;
    assert_eq!(p.item(sid).unwrap().name, "River Story");
    assert_eq!(s.settings.frame_rate, r);
    assert_eq!(s.start_timecode, 86_400);
    let v1 = &s.video_tracks[0];
    assert_eq!(v1.items.len(), 3);
    let (a, b, c) = (&v1.items[0], &v1.items[1], &v1.items[2]);
    // source times are relative to the reference's available range start
    assert_eq!((a.start, a.duration, a.source_in), (r.tick_of(12), r.tick_of(48), r.tick_of(48)));
    assert_eq!(media_key(p, a.item), "/footage/river a.mov");
    assert_eq!(a.markers[0].name, "kingfisher");
    assert_eq!(a.markers[0].start, r.tick_of(60));
    assert_eq!(a.markers[0].color, Label::Rose);
    assert_eq!((b.start, b.duration, b.source_in, b.speed), (r.tick_of(60), r.tick_of(36), r.tick_of(10), 2.0));
    assert_eq!(media_key(p, b.item), "/proj/clips/river_b.mov");
    assert_eq!(c.start, r.tick_of(120));
    assert!(!c.enabled);
    assert!(p.item(c.item).unwrap().as_media().unwrap().offline);
    let ts = &v1.transitions;
    assert_eq!(ts.len(), 3);
    assert_eq!((ts[0].from, ts[0].to, ts[0].start, ts[0].duration, ts[0].align), (None, Some(a.id), r.tick_of(12), r.tick_of(6), TransitionAlign::StartAtCut));
    assert_eq!(
        (ts[1].from, ts[1].to, ts[1].start, ts[1].duration, ts[1].align),
        (Some(a.id), Some(b.id), r.tick_of(54), r.tick_of(12), TransitionAlign::CenterAtCut)
    );
    assert_eq!((ts[2].from, ts[2].to, ts[2].start, ts[2].align), (Some(c.id), None, r.tick_of(144), TransitionAlign::EndAtCut));
    assert!(ts.iter().all(|t| t.effect.effect == "cross_dissolve"), "{rep}");
    let amb = &s.audio_tracks[0].items[0];
    assert_eq!(amb.duration, r.tick_of(150));
    assert!(rep.mentions("Reverb"), "{rep}");
    assert_eq!(s.markers[0].name, "act two");
    assert_eq!(s.markers[0].start, r.tick_of(60));
}

fn project() -> (Project, cfa4_project::ItemId) {
    let r = FrameRate::FPS_23_976;
    let mut p = Project::new("P");
    let a = media(&mut p, "/m/a.mov", true, true, r);
    if let Some(m) = p.item_mut(a).and_then(|i| i.as_media_mut()) {
        m.info.start_timecode = Some(86_400);
    }
    let b = media(&mut p, "/m/b.mov", true, true, r);
    let g = generator(&mut p, "Matte", Generator::ColorMatte { color: [0.2, 0.4, 0.6, 1.0] });
    let nest = sequence(&mut p, "Inner", r, false);
    clip(&mut p, nest, TrackKind::Video, 0, b, 0, 50, 5);
    let s = sequence(&mut p, "Outer", r, false);
    let v1 = clip(&mut p, s, TrackKind::Video, 0, a, 0, 48, 24);
    let a1 = clip(&mut p, s, TrackKind::Audio, 0, a, 0, 48, 24);
    link(&mut p, s, &[v1, a1]);
    let v2 = clip(&mut p, s, TrackKind::Video, 0, nest, 48, 40, 3);
    transition(&mut p, s, TrackKind::Video, 0, "iris_round", Some(v1), Some(v2), 40, 16, TransitionAlign::CenterAtCut);
    clip(&mut p, s, TrackKind::Video, 1, g, 10, 20, 0);
    {
        let q = p.sequence_mut(s).unwrap();
        // sub-frame audio position
        let mut ai = q.audio_tracks[0].items[0].clone();
        ai.id = cfa4_project::ClipId(9999);
        ai.link = None;
        ai.start = Tick(TICKS_PER_SECOND * 3 + TICKS_PER_SECOND / 48_000 * 7);
        ai.duration = Tick(TICKS_PER_SECOND / 48_000 * 33_333);
        ai.gain_db = 4.5;
        q.audio_tracks[1].items.push(ai);
        let (_, it) = q.find_item_mut(v1).unwrap();
        it.label = Label::Teal;
        let mut op = Param::new(ParamValue::Float(100.0));
        op.keyframes.push(Keyframe::new(r.tick_of(24), ParamValue::Float(0.0)));
        op.keyframes.push(Keyframe::new(r.tick_of(36), ParamValue::Float(100.0)));
        it.effect_mut("opacity").unwrap().params.insert("opacity".into(), op);
        it.effects.push(cfa4_project::find_effect("gaussian_blur").unwrap().instance());
        it.markers.push(Marker {
            id: MarkerId(5),
            start: r.tick_of(30),
            duration: r.tick_of(2),
            name: "m".into(),
            comment: "c".into(),
            kind: MarkerKind::Comment,
            color: Label::Yellow,
        });
        let (_, n) = q.find_item_mut(v2).unwrap();
        n.reverse = true;
        n.speed = 1.5;
        q.video_tracks[1].locked = true;
        q.audio_tracks[1].volume_db = -3.0;
        q.markers.push(Marker {
            id: MarkerId(6),
            start: r.tick_of(70),
            duration: Tick::ZERO,
            name: "chap".into(),
            comment: String::new(),
            kind: MarkerKind::Chapter,
            color: Label::Mango,
        });
    }
    (p, s)
}

#[test]
fn lossless_roundtrip_with_cfa4_metadata() {
    let (p, s) = project();
    let (imp, text, rep) = roundtrip(&p, s, Format::Otio, &ExportOptions::default());
    assert!(!rep.has_warnings(), "{rep}");
    assert!(text.contains("\"OTIO_SCHEMA\": \"Timeline.1\""));
    assert!(text.contains("\"target_url\": \"file:///m/a.mov\""));
    let si = only_seq(&imp);
    let q = &imp.project;
    let (qs, ps) = (q.sequence(si).unwrap(), p.sequence(s).unwrap());
    for kind in [TrackKind::Video, TrackKind::Audio] {
        assert_eq!(structure(q, si, kind), structure(&p, s, kind), "{text}");
    }
    assert_eq!(links(q, si), links(&p, s));
    assert_eq!(qs.settings, ps.settings);
    assert_eq!(
        qs.markers.iter().map(|m| (&m.name, m.start, m.kind, m.color)).collect::<Vec<_>>(),
        ps.markers.iter().map(|m| (&m.name, m.start, m.kind, m.color)).collect::<Vec<_>>()
    );
    assert!(qs.video_tracks[1].locked);
    assert_eq!(qs.audio_tracks[1].volume_db, -3.0);
    // clip-level fields restored exactly
    let (qa, pa) = (&qs.video_tracks[0].items[0], &ps.video_tracks[0].items[0]);
    assert_eq!(format!("{:?}", qa.effects), format!("{:?}", pa.effects)); // NaN auto points compare unequal
    assert_eq!(qa.label, Label::Teal);
    assert_eq!(
        qa.markers.iter().map(|m| (&m.name, m.start, m.duration)).collect::<Vec<_>>(),
        pa.markers.iter().map(|m| (&m.name, m.start, m.duration)).collect::<Vec<_>>()
    );
    assert_eq!(qs.audio_tracks[1].items[0].gain_db, 4.5);
    assert_eq!(qs.audio_tracks[1].items[0].start, ps.audio_tracks[1].items[0].start);
    assert_eq!(qs.video_tracks[0].transitions[0].effect.effect, "iris_round");
    // media info and generator restored
    let qa_media = q.item(qa.item).unwrap().as_media().unwrap();
    assert_eq!(qa_media.info, p.item(pa.item).unwrap().as_media().unwrap().info);
    let gm = q.item(qs.video_tracks[1].items[0].item).unwrap().as_media().unwrap();
    assert_eq!(gm.media, MediaRef::Generator(Generator::ColorMatte { color: [0.2, 0.4, 0.6, 1.0] }));
    // nested sequence
    let nested = qs.video_tracks[0].items[1].item;
    assert_eq!(q.item(nested).unwrap().name, "Inner");
    assert_eq!(q.sequence(nested).unwrap().video_tracks[0].items[0].source_in, FrameRate::FPS_23_976.tick_of(5));
    // relative media paths
    let o = ExportOptions { relative_to: Some("/m/otio".into()), ..Default::default() };
    let (bytes, _) = cfa4_interchange::export(&p, s, Format::Otio, &o).unwrap();
    let t = String::from_utf8(bytes.clone()).unwrap();
    assert!(t.contains("\"target_url\": \"../a.mov\""), "{t}");
    let (imp2, _) = import(&bytes, Format::Otio, Some("/m/otio")).unwrap();
    assert_eq!(structure(&imp2.project, imp2.sequences[0], TrackKind::Video), structure(&p, s, TrackKind::Video));
}

#[test]
fn collections_and_bare_tracks() {
    let one = r#"{"OTIO_SCHEMA": "SerializableCollection.1", "name": "Bundle", "children": [
        {"OTIO_SCHEMA": "Timeline.1", "name": "T1", "tracks": {"OTIO_SCHEMA": "Stack.1", "children": [
            {"OTIO_SCHEMA": "Track.1", "kind": "Video", "children": [
                {"OTIO_SCHEMA": "Clip.2", "name": "x", "source_range": {"OTIO_SCHEMA": "TimeRange.1", "start_time": {"OTIO_SCHEMA": "RationalTime.1", "rate": 25, "value": 0}, "duration": {"OTIO_SCHEMA": "RationalTime.1", "rate": 25, "value": 50}},
                 "media_references": {"DEFAULT_MEDIA": {"OTIO_SCHEMA": "GeneratorReference.1", "generator_kind": "SMPTEBars"}}, "active_media_reference_key": "DEFAULT_MEDIA"}]}]}},
        {"OTIO_SCHEMA": "Timeline.1", "name": "T2", "tracks": {"OTIO_SCHEMA": "Stack.1", "children": []}}
    ]}"#;
    let (imp, _) = import(one.as_bytes(), Format::Otio, None).unwrap();
    assert_eq!(imp.sequences.len(), 2);
    let s = imp.project.sequence(imp.sequences[0]).unwrap();
    assert_eq!(s.settings.frame_rate, FrameRate::FPS_25);
    assert_eq!(s.video_tracks[0].items[0].duration, FrameRate::FPS_25.tick_of(50));
    let m = imp.project.item(s.video_tracks[0].items[0].item).unwrap().as_media().unwrap();
    assert_eq!(m.media, MediaRef::Generator(Generator::BarsAndTone));
    assert!(import(b"{\"OTIO_SCHEMA\": \"Clip.2\"}", Format::Otio, None).is_err());
    assert!(import(b"not json", Format::Otio, None).is_err());
}

#[derive(Clone, Debug)]
struct G {
    track: usize,
    gap: i64,
    dur: i64,
    media: usize,
    src: i64,
    speed: u8,
    enabled: bool,
    tr: u8,
    tr_len: i64,
    audio: bool,
    subframe: i64,
}

fn gen_clip() -> impl Strategy<Value = G> {
    (0usize..3, 0i64..3, 20i64..150, 0usize..3, 0i64..2000, 0u8..5, prop::bool::weighted(0.9), 0u8..6, 1i64..9, any::<bool>(), 0i64..2000).prop_map(
        |(track, gap, dur, media, src, speed, enabled, tr, tr_len, audio, subframe)| G {
            track,
            gap: gap * 5,
            dur,
            media,
            src,
            speed,
            enabled,
            tr,
            tr_len,
            audio,
            subframe,
        },
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    #[test]
    fn otio_roundtrip(clips in prop::collection::vec(gen_clip(), 1..20), ri in 0usize..5) {
        let rate = [FrameRate::FPS_23_976, FrameRate::FPS_25, FrameRate::FPS_29_97, FrameRate::FPS_59_94, FrameRate::FPS_24][ri];
        let mut p = Project::new("P");
        let ms = [media(&mut p, "/x/one.mov", true, true, rate), media(&mut p, "/x/two.mov", true, true, rate), media(&mut p, "/y/three four.mxf", true, true, rate)];
        let s = sequence(&mut p, "S", rate, rate == FrameRate::FPS_29_97);
        let mut ends = [0i64; 3];
        let mut prev: [Option<cfa4_project::ClipId>; 3] = [None; 3];
        let mut aend = [Tick::ZERO; 3];
        for g in &clips {
            let t = ends[g.track] + g.gap;
            let v = clip(&mut p, s, TrackKind::Video, g.track, ms[g.media], t, g.dur, g.src);
            {
                let q = p.sequence_mut(s).unwrap();
                let (_, it) = q.find_item_mut(v).unwrap();
                it.enabled = g.enabled;
                match g.speed {
                    1 => it.speed = 2.0,
                    2 => it.speed = 0.5,
                    3 => it.reverse = true,
                    4 => it.frame_hold = Some(rate.tick_of(g.src + 3)),
                    _ => {}
                }
            }
            let d = g.tr_len;
            let adjacent = g.gap == 0 && prev[g.track].is_some();
            match g.tr {
                0 if adjacent => transition(&mut p, s, TrackKind::Video, g.track, "cross_dissolve", prev[g.track], Some(v), t - d / 2, d, TransitionAlign::CenterAtCut),
                1 if adjacent => transition(&mut p, s, TrackKind::Video, g.track, "dip_to_white", prev[g.track], Some(v), t, d, TransitionAlign::StartAtCut),
                2 if adjacent => transition(&mut p, s, TrackKind::Video, g.track, "push", prev[g.track], Some(v), t - d, d, TransitionAlign::EndAtCut),
                3 if !adjacent => transition(&mut p, s, TrackKind::Video, g.track, "cross_dissolve", None, Some(v), t, d, TransitionAlign::StartAtCut),
                _ => {}
            }
            // Audio at sample (not frame) positions, linked to the video.
            let at = rate.tick_of(t) + Tick(TICKS_PER_SECOND / 48_000 * g.subframe % rate.frame_duration().0.max(1));
            if g.audio && aend[g.track] <= at {
                let a = clip(&mut p, s, TrackKind::Audio, g.track, ms[g.media], t, g.dur, g.src);
                let q = p.sequence_mut(s).unwrap();
                let (_, ai) = q.find_item_mut(a).unwrap();
                ai.start = at;
                aend[g.track] = ai.end();
                link(&mut p, s, &[v, a]);
            }
            ends[g.track] = t + g.dur;
            prev[g.track] = Some(v);
        }
        let (imp, text, _) = roundtrip(&p, s, Format::Otio, &ExportOptions::default());
        let si = only_seq(&imp);
        prop_assert_eq!(structure(&imp.project, si, TrackKind::Video), structure(&p, s, TrackKind::Video), "{}", text);
        prop_assert_eq!(structure(&imp.project, si, TrackKind::Audio), structure(&p, s, TrackKind::Audio));
        prop_assert_eq!(links(&imp.project, si), links(&p, s));
    }
}

// ---------------------------------------------------------------------------------------------
// OTIO-1.1: fixtures for the external OpenTimelineIO test oracle
// (`python -m cfa4_devtools.otio_oracle`, docs/plans/OTIO-INTEGRATION-2026-10-24.md).
//
// `oracle_fixtures` exports every sequence these tests build to OTIO and writes, next to the
// files, a `manifest.json` with what the project holds: per track (in export order: video, then
// audio) every clip's name, timeline start and duration in exact ticks, and its marker count. The
// official `opentimelineio` package then reads the files and checks them against the manifest with
// its own timing rules. Files are only written when `CFA4_OTIO_ORACLE_DIR` is set; without it the
// test still exports everything and checks the manifest is consistent.
// ---------------------------------------------------------------------------------------------

/// A fixed sequence that uses every timing feature the exporter writes: gaps, all three transition
/// alignments, a lone fade-in, speed changes, reverse, a frame hold, a disabled clip and linked
/// audio at a sub-frame (sample) position.
fn mixed(rate: FrameRate) -> (Project, cfa4_project::ItemId) {
    let mut p = Project::new("P");
    let m = [media(&mut p, "/x/one.mov", true, true, rate), media(&mut p, "/x/two.mov", true, true, rate)];
    let s = sequence(&mut p, "Mixed", rate, rate == FrameRate::FPS_29_97);
    let fade = clip(&mut p, s, TrackKind::Video, 0, m[0], 10, 40, 100);
    transition(&mut p, s, TrackKind::Video, 0, "cross_dissolve", None, Some(fade), 10, 8, TransitionAlign::StartAtCut);
    let fast = clip(&mut p, s, TrackKind::Video, 0, m[1], 50, 30, 7);
    transition(&mut p, s, TrackKind::Video, 0, "cross_dissolve", Some(fade), Some(fast), 46, 8, TransitionAlign::CenterAtCut);
    let rev = clip(&mut p, s, TrackKind::Video, 0, m[0], 80, 25, 300);
    transition(&mut p, s, TrackKind::Video, 0, "dip_to_white", Some(fast), Some(rev), 80, 6, TransitionAlign::StartAtCut);
    let hold = clip(&mut p, s, TrackKind::Video, 0, m[1], 105, 20, 40);
    transition(&mut p, s, TrackKind::Video, 0, "push", Some(rev), Some(hold), 101, 4, TransitionAlign::EndAtCut);
    let off = clip(&mut p, s, TrackKind::Video, 1, m[1], 33, 17, 0);
    let a = clip(&mut p, s, TrackKind::Audio, 0, m[0], 10, 40, 100);
    link(&mut p, s, &[fade, a]);
    let q = p.sequence_mut(s).unwrap();
    q.find_item_mut(fast).unwrap().1.speed = 2.0;
    q.find_item_mut(rev).unwrap().1.reverse = true;
    q.find_item_mut(hold).unwrap().1.frame_hold = Some(rate.tick_of(45));
    q.find_item_mut(off).unwrap().1.enabled = false;
    let ai = q.find_item_mut(a).unwrap().1;
    ai.start = rate.tick_of(10) + Tick(TICKS_PER_SECOND / 48_000 * 13);
    (p, s)
}

/// What the oracle expects to find in the exported file of sequence `s`.
fn oracle_expectation(p: &Project, s: cfa4_project::ItemId, file: &str) -> serde_json::Value {
    let seq = p.sequence(s).unwrap();
    let tracks: Vec<serde_json::Value> = [TrackKind::Video, TrackKind::Audio]
        .into_iter()
        .flat_map(|kind| seq.tracks(kind).iter().map(move |t| (kind, t)))
        .map(|(kind, t)| {
            let items: Vec<serde_json::Value> = t
                .items
                .iter()
                .map(|it| serde_json::json!({"name": it.name, "start_ticks": it.start.0, "duration_ticks": it.duration.0, "markers": it.markers.len()}))
                .collect();
            serde_json::json!({"kind": if kind == TrackKind::Video { "Video" } else { "Audio" }, "name": t.name, "items": items})
        })
        .collect();
    serde_json::json!({"file": file, "name": p.item(s).unwrap().name, "markers": seq.markers.len(), "tracks": tracks})
}

#[test]
fn oracle_fixtures() {
    let mut cases: Vec<(String, Project, cfa4_project::ItemId, ExportOptions)> = Vec::new();
    let (p, s) = project();
    cases.push(("outer.otio".into(), p.clone(), s, ExportOptions::default()));
    cases.push(("outer_relative.otio".into(), p, s, ExportOptions { relative_to: Some("/m/otio".into()), ..Default::default() }));
    let (imp, _) = import(doc().as_bytes(), Format::Otio, Some("/proj")).unwrap();
    let sid = only_seq(&imp);
    cases.push(("river_story.otio".into(), imp.project, sid, ExportOptions::default()));
    for (tag, rate) in [
        ("23976", FrameRate::FPS_23_976),
        ("24", FrameRate::FPS_24),
        ("25", FrameRate::FPS_25),
        ("2997df", FrameRate::FPS_29_97),
        ("5994", FrameRate::FPS_59_94),
    ] {
        let (p, s) = mixed(rate);
        cases.push((format!("mixed_{tag}.otio"), p, s, ExportOptions::default()));
    }

    // Cargo runs integration tests from the package directory, so a relative path is taken from the
    // workspace root, where the documented `python -m cfa4_devtools.otio_oracle` command looks.
    let out = std::env::var_os("CFA4_OTIO_ORACLE_DIR").map(|d| {
        let d = std::path::PathBuf::from(d);
        if d.is_absolute() { d } else { std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(d) }
    });
    if let Some(dir) = &out {
        std::fs::create_dir_all(dir).unwrap();
    }
    let mut fixtures = Vec::new();
    for (file, p, s, opts) in &cases {
        let (bytes, rep) = cfa4_interchange::export(p, *s, Format::Otio, opts).unwrap();
        assert!(!rep.has_warnings(), "{file}: {rep}");
        let exp = oracle_expectation(p, *s, file);
        // the manifest must describe what was exported: as many items per track as the project has
        let n: usize = exp["tracks"].as_array().unwrap().iter().map(|t| t["items"].as_array().unwrap().len()).sum();
        assert!(n > 0, "{file}: empty fixture");
        if let Some(dir) = &out {
            std::fs::write(dir.join(file), &bytes).unwrap();
        }
        fixtures.push(exp);
    }
    if let Some(dir) = &out {
        let manifest = serde_json::json!({"version": 1, "ticks_per_second": TICKS_PER_SECOND, "fixtures": fixtures});
        std::fs::write(dir.join("manifest.json"), serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
    }
}

// ---------------------------------------------------------------------------------------------
// OTIO-1.2: the reverse direction. `python -m cfa4_devtools.otio_samples <dir>` has the official
// `opentimelineio` package write sample documents for every schema CFA4 reads, with a manifest of
// what CFA4 must import, timed by OTIO's own rules. `oracle_samples` imports them when
// `CFA4_OTIO_SAMPLES_DIR` is set (relative to the workspace root, like `CFA4_OTIO_ORACLE_DIR`);
// without it the test does nothing.
// ---------------------------------------------------------------------------------------------

/// OTIO stores times as floating-point frame counts: 1 µs is far below one 48 kHz sample.
const SAMPLE_TOLERANCE: i64 = TICKS_PER_SECOND / 1_000_000;

fn ticks_close(got: Tick, want: &serde_json::Value) -> bool {
    want.as_i64().is_some_and(|w| (got.0 - w).abs() <= SAMPLE_TOLERANCE)
}

fn sample_media(p: &Project, ti: &cfa4_project::TrackItem, want: &serde_json::Value, at: &str, problems: &mut Vec<String>) {
    let Some(item) = p.item(ti.item) else {
        problems.push(format!("{at}: points at no project item"));
        return;
    };
    match want["ref"].as_str() {
        Some("sequence") => {
            if item.as_sequence().is_none() {
                problems.push(format!("{at}: expected a nested sequence"));
            }
        }
        Some(kind) => {
            let Some(m) = item.as_media() else {
                problems.push(format!("{at}: expected media, found another item"));
                return;
            };
            if m.offline != want["offline"].as_bool().unwrap_or(false) {
                problems.push(format!("{at}: offline {} != {}", m.offline, want["offline"]));
            }
            match (kind, &m.media) {
                ("file", MediaRef::File { path }) => {
                    if let Some(w) = want["path"].as_str()
                        && path != w
                    {
                        problems.push(format!("{at}: path {path:?} != {w:?}"));
                    }
                }
                ("generator", MediaRef::Generator(g)) => {
                    let v = serde_json::to_value(g).unwrap_or_default();
                    let name = v.as_str().map(str::to_string).or_else(|| v.as_object().and_then(|o| o.keys().next().cloned())).unwrap_or_default();
                    if Some(name.as_str()) != want["generator"].as_str() {
                        problems.push(format!("{at}: generator {name} != {}", want["generator"]));
                    }
                }
                (k, other) => problems.push(format!("{at}: media {other:?}, expected {k}")),
            }
        }
        None => problems.push(format!("{at}: manifest has no media kind")),
    }
}

fn sample_track(p: &Project, t: &cfa4_project::Track, want: &serde_json::Value, at: &str, problems: &mut Vec<String>) {
    if Some(t.name.as_str()) != want["name"].as_str() {
        problems.push(format!("{at}: track name {:?} != {}", t.name, want["name"]));
    }
    let items = want["items"].as_array().cloned().unwrap_or_default();
    if t.items.len() != items.len() {
        problems.push(format!("{at}: {} items, expected {}", t.items.len(), items.len()));
    }
    for (ti, wi) in t.items.iter().zip(&items) {
        let at = format!("{at} item {:?}", wi["name"].as_str().unwrap_or(""));
        if Some(ti.name.as_str()) != wi["name"].as_str() {
            problems.push(format!("{at}: name {:?}", ti.name));
        }
        if !ticks_close(ti.start, &wi["start_ticks"]) {
            problems.push(format!("{at}: start {} != {}", ti.start.0, wi["start_ticks"]));
        }
        if !ticks_close(ti.duration, &wi["duration_ticks"]) {
            problems.push(format!("{at}: duration {} != {}", ti.duration.0, wi["duration_ticks"]));
        }
        if Some(ti.enabled) != wi["enabled"].as_bool() {
            problems.push(format!("{at}: enabled {}", ti.enabled));
        }
        if let Some(speed) = wi["speed"].as_f64()
            && (ti.speed - speed).abs() > 1e-9
        {
            problems.push(format!("{at}: speed {} != {speed}", ti.speed));
        }
        if let Some(r) = wi["reverse"].as_bool()
            && ti.reverse != r
        {
            problems.push(format!("{at}: reverse {}", ti.reverse));
        }
        if let Some(h) = wi["frame_hold"].as_bool()
            && ti.frame_hold.is_some() != h
        {
            problems.push(format!("{at}: frame hold {:?}", ti.frame_hold));
        }
        let markers = wi["markers"].as_array().cloned().unwrap_or_default();
        if ti.markers.len() != markers.len() {
            problems.push(format!("{at}: {} markers, expected {}", ti.markers.len(), markers.len()));
        }
        for (m, wm) in ti.markers.iter().zip(&markers) {
            if Some(m.name.as_str()) != wm["name"].as_str() || !ticks_close(m.start, &wm["start_ticks"]) {
                problems.push(format!("{at}: marker {:?} at {} != {wm}", m.name, m.start.0));
            }
        }
        sample_media(p, ti, &wi["media"], &at, problems);
    }
    let trans = want["transitions"].as_array().cloned().unwrap_or_default();
    if t.transitions.len() != trans.len() {
        problems.push(format!("{at}: {} transitions, expected {}", t.transitions.len(), trans.len()));
    }
    for (i, (tr, wt)) in t.transitions.iter().zip(&trans).enumerate() {
        if !ticks_close(tr.start, &wt["start_ticks"]) || !ticks_close(tr.duration, &wt["duration_ticks"]) {
            problems.push(format!("{at} transition {i}: {}+{} != {wt}", tr.start.0, tr.duration.0));
        }
    }
}

/// Differences between what CFA4 imported from one sample and what the manifest expects.
fn check_sample(dir: &std::path::Path, entry: &serde_json::Value, base: Option<&str>) -> Vec<String> {
    let mut problems = Vec::new();
    let Some(file) = entry["file"].as_str().filter(|f| f.ends_with(".otio") && std::path::Path::new(f).file_name().and_then(|n| n.to_str()) == Some(*f)) else {
        return vec![format!("invalid sample file name {}", entry["file"])];
    };
    let bytes = match std::fs::read(dir.join(file)) {
        Ok(b) => b,
        Err(e) => return vec![format!("cannot read: {e}")],
    };
    let (imp, rep) = match import(&bytes, Format::Otio, base) {
        Ok(r) => r,
        Err(e) => return vec![format!("import failed: {e}")],
    };
    // Every expected warning matches exactly one report warning; nothing else may warn.
    let mut warnings: Vec<String> = rep.warnings().map(|w| w.message.clone()).collect();
    for want in entry["warnings"].as_array().into_iter().flatten().filter_map(|w| w.as_str()) {
        match warnings.iter().position(|w| w.contains(want)) {
            Some(i) => {
                warnings.remove(i);
            }
            None => problems.push(format!("missing warning containing {want:?}")),
        }
    }
    problems.extend(warnings.into_iter().map(|w| format!("unexpected warning: {w}")));

    let seqs = entry["sequences"].as_array().cloned().unwrap_or_default();
    if imp.sequences.len() != seqs.len() {
        problems.push(format!("{} sequences, expected {}", imp.sequences.len(), seqs.len()));
    }
    let p = &imp.project;
    for (&sid, ws) in imp.sequences.iter().zip(&seqs) {
        let at = format!("sequence {}", ws["name"]);
        let (Some(item), Some(seq)) = (p.item(sid), p.sequence(sid)) else {
            problems.push(format!("{at}: missing"));
            continue;
        };
        if Some(item.name.as_str()) != ws["name"].as_str() {
            problems.push(format!("{at}: name {:?}", item.name));
        }
        if let Some(rate) = ws["rate"].as_f64()
            && (seq.settings.frame_rate.as_f64() - rate).abs() > 1e-6
        {
            problems.push(format!("{at}: frame rate {} != {rate}", seq.settings.frame_rate.as_f64()));
        }
        if Some(seq.start_timecode) != ws["start_timecode"].as_i64() {
            problems.push(format!("{at}: start timecode {} != {}", seq.start_timecode, ws["start_timecode"]));
        }
        let markers = ws["markers"].as_array().cloned().unwrap_or_default();
        if seq.markers.len() != markers.len() {
            problems.push(format!("{at}: {} markers, expected {}", seq.markers.len(), markers.len()));
        }
        for (m, wm) in seq.markers.iter().zip(&markers) {
            if Some(m.name.as_str()) != wm["name"].as_str() || !ticks_close(m.start, &wm["start_ticks"]) {
                problems.push(format!("{at}: marker {:?} at {} != {wm}", m.name, m.start.0));
            }
        }
        // OTIO lists tracks in stack order; CFA4 keeps video and audio apart, and adds an empty
        // track of a kind the document lacks.
        for kind in [TrackKind::Video, TrackKind::Audio] {
            let label = if kind == TrackKind::Video { "Video" } else { "Audio" };
            let wanted: Vec<&serde_json::Value> = ws["tracks"].as_array().into_iter().flatten().filter(|t| t["kind"].as_str() == Some(label)).collect();
            let got = seq.tracks(kind);
            if got.len() < wanted.len() {
                problems.push(format!("{at}: {} {label} tracks, expected {}", got.len(), wanted.len()));
            }
            for (i, t) in got.iter().enumerate() {
                match wanted.get(i) {
                    Some(w) => sample_track(p, t, w, &format!("{at} {label} {i}"), &mut problems),
                    None if t.items.is_empty() && t.transitions.is_empty() => {}
                    None => problems.push(format!("{at}: unexpected {label} track {i} with content")),
                }
            }
        }
    }
    problems
}

#[test]
fn oracle_samples() {
    let Some(dir) = std::env::var_os("CFA4_OTIO_SAMPLES_DIR").map(|d| {
        let d = std::path::PathBuf::from(d);
        if d.is_absolute() { d } else { std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(d) }
    }) else {
        return;
    };
    let text = std::fs::read_to_string(dir.join("manifest.json")).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(manifest["version"], 1);
    assert_eq!(manifest["kind"], "otio-samples");
    assert_eq!(manifest["ticks_per_second"].as_i64(), Some(TICKS_PER_SECOND), "sample manifest uses another time base");
    let base = manifest["base_dir"].as_str();
    let samples = manifest["samples"].as_array().unwrap();
    assert!(!samples.is_empty(), "no samples in the manifest");
    let failures: Vec<String> =
        samples.iter().flat_map(|s| check_sample(&dir, s, base).into_iter().map(move |p| format!("{}: {p}", s["file"].as_str().unwrap_or("?")))).collect();
    assert!(failures.is_empty(), "{} OTIO sample problem(s) (oracle {}):\n{}", failures.len(), manifest["oracle_version"], failures.join("\n"));
}
