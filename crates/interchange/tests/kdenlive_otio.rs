//! OpenTimelineIO written by Kdenlive (K1, first step of `.kdenlive` import): the CC0 test dataset of
//! Kdenlive's own OTIO export/import tests (`tests/data/kdenlive/README.md`). Expected values are read
//! off the Kdenlive documents, not off our own output.

mod support;

use cfa4_interchange::{Format, Imported, Report, detect, import};
use cfa4_media::Generator;
use cfa4_project::{ItemKind, MediaRef, Project, Sequence, Track, TrackKind};
use cfa4_time::{FrameRate, Tick};
use support::*;

const BASE: &str = "/proj";

fn load(name: &str) -> (Imported, Report) {
    let path = format!("{}/tests/data/kdenlive/{name}", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    assert_eq!(detect(&bytes, Some("otio")), Some(Format::Otio), "{name}");
    import(&bytes, Format::Otio, Some(BASE)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn seq(imp: &Imported) -> &Sequence {
    imp.project.sequence(only_seq(imp)).expect("sequence")
}

/// `(start, duration, source in, media)` of every clip on a track.
fn clips(p: &Project, t: &Track) -> Vec<(Tick, Tick, Tick, String)> {
    t.items.iter().map(|i| (i.start, i.duration, i.source_in, media_key(p, i.item))).collect()
}

fn track<'a>(s: &'a Sequence, kind: TrackKind, name: &str) -> &'a Track {
    let tracks = if kind == TrackKind::Video { &s.video_tracks } else { &s.audio_tracks };
    tracks.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("no {kind:?} track {name}: {:?}", tracks.iter().map(|t| &t.name).collect::<Vec<_>>()))
}

fn non_empty(tracks: &[Track]) -> Vec<&Track> {
    tracks.iter().filter(|t| !t.items.is_empty()).collect()
}

#[test]
fn tracks_and_clips() {
    let (imp, rep) = load("test-tracks-clips.otio");
    let (p, s) = (&imp.project, seq(&imp));
    let r = s.settings.frame_rate;
    assert_eq!(r, FrameRate::FPS_25);
    assert_eq!((s.video_tracks.len(), s.audio_tracks.len()), (2, 2));
    let f = |n| r.tick_of(n);
    let file = |n: &str| format!("{BASE}/{n}");
    assert_eq!(clips(p, track(s, TrackKind::Video, "V2")), vec![(f(0), f(5), f(0), file("red.mpg")), (f(10), f(5), f(0), file("blue.mpg"))]);
    assert_eq!(clips(p, track(s, TrackKind::Video, "V1")), vec![(f(5), f(5), f(0), file("yellow.mpg"))]);
    assert_eq!(clips(p, track(s, TrackKind::Audio, "A1")), vec![(f(5), f(5), f(0), file("yellow.mpg"))]);
    assert_eq!(clips(p, track(s, TrackKind::Audio, "A2")), vec![(f(0), f(5), f(0), file("red.mpg")), (f(10), f(5), f(0), file("blue.mpg"))]);
    assert!(rep.entries.is_empty(), "{rep}");
}

#[test]
fn guides_and_clip_markers() {
    let (imp, rep) = load("test-guides-markers.otio");
    let (p, s) = (&imp.project, seq(&imp));
    let r = s.settings.frame_rate;
    assert_eq!(r, FrameRate::FPS_24);
    let f = |n| r.tick_of(n);
    // Kdenlive guides are OTIO markers on the stack.
    let guides: Vec<_> = s.markers.iter().map(|m| (m.start, m.comment.as_str())).collect();
    assert_eq!(guides, vec![(f(1), "This is the first guide"), (f(14), "This is the second guide")]);
    assert_ne!(s.markers[0].color, s.markers[1].color, "PURPLE and ORANGE guides keep distinct colours");
    // Clip markers: blue.mpg 2, red.mpg 2, yellow.mpg 1, on both video and audio.
    for kind in [TrackKind::Video, TrackKind::Audio] {
        let tracks = if kind == TrackKind::Video { &s.video_tracks } else { &s.audio_tracks };
        let mut seen: Vec<(String, Vec<Tick>)> = tracks
            .iter()
            .flat_map(|t| &t.items)
            .map(|i| (media_key(p, i.item).rsplit('/').next().unwrap_or_default().to_string(), i.markers.iter().map(|m| m.start).collect()))
            .collect();
        seen.sort();
        assert_eq!(seen, vec![("blue.mpg".into(), vec![f(2), f(3)]), ("red.mpg".into(), vec![f(3), f(5)]), ("yellow.mpg".into(), vec![f(4)])], "{kind:?}");
    }
    assert!(rep.entries.is_empty(), "{rep}");
}

#[test]
fn solid_colour_clips() {
    let (imp, rep) = load("test-color-clips.otio");
    let (p, s) = (&imp.project, seq(&imp));
    let r = s.settings.frame_rate;
    let f = |n| r.tick_of(n);
    let colour = |item| match &p.item(item).expect("item").kind {
        ItemKind::Media(m) => match &m.media {
            MediaRef::Generator(Generator::ColorMatte { color }) => Some(*color),
            _ => None,
        },
        _ => None,
    };
    let mut got: Vec<_> = non_empty(&s.video_tracks).iter().flat_map(|t| &t.items).map(|i| (i.start, i.duration, colour(i.item))).collect();
    got.sort_by_key(|g| g.0);
    assert_eq!(got, vec![(f(0), f(1), Some([1.0, 0.0, 0.0, 1.0])), (f(2), f(1), Some([1.0, 1.0, 0.0, 1.0])), (f(4), f(1), Some([0.0, 0.0, 1.0, 1.0])),]);
    assert!(!rep.mentions("not supported"), "{rep}");
}

#[test]
fn transitions() {
    let (imp, rep) = load("test-transitions.otio");
    let (p, s) = (&imp.project, seq(&imp));
    let r = s.settings.frame_rate;
    let f = |n| r.tick_of(n);
    let v = non_empty(&s.video_tracks);
    assert_eq!(v.len(), 1);
    let t = v[0];
    let file = |n: &str| format!("{BASE}/{n}");
    assert_eq!(clips(p, t), vec![(f(0), f(24), f(0), file("red.png")), (f(24), f(24), f(12), file("yellow.png")), (f(48), f(24), f(4), file("blue.png"))]);
    let ts: Vec<_> = t.transitions.iter().map(|x| (x.start, x.duration, x.effect.effect.as_str())).collect();
    // OTIO in/out offsets 12+4 at the first cut (24), 4+12 at the second (48).
    assert_eq!(ts, vec![(f(12), f(16), "cross_dissolve"), (f(44), f(16), "cross_dissolve")]);
    assert!(rep.entries.is_empty(), "{rep}");
}

#[test]
fn missing_media_is_offline() {
    for name in ["test-missing-media.otio", "test-missing-media2.otio"] {
        let (imp, rep) = load(name);
        let (p, s) = (&imp.project, seq(&imp));
        let r = s.settings.frame_rate;
        let v = non_empty(&s.video_tracks);
        assert_eq!(v.len(), 1, "{name}");
        let items = &v[0].items;
        assert_eq!(
            items.iter().map(|i| (i.start, i.duration)).collect::<Vec<_>>(),
            vec![(r.tick_of(0), r.tick_of(24)), (r.tick_of(24), r.tick_of(24))],
            "{name}"
        );
        // An empty `target_url` cannot be found: the clip stays on the timeline, offline.
        let offline: Vec<bool> = items.iter().map(|i| p.item(i.item).and_then(|it| it.as_media()).is_some_and(|m| m.offline)).collect();
        let expect = if name == "test-missing-media.otio" { vec![false, true] } else { vec![true, true] };
        assert_eq!(offline, expect, "{name}");
        assert!(rep.mentions("without a path"), "{name}: {rep}");
    }
}

#[test]
fn empty_documents() {
    let (imp, rep) = load("test-no-clips.otio");
    let s = seq(&imp);
    assert!(s.video_tracks.iter().chain(&s.audio_tracks).all(|t| t.items.is_empty()));
    assert!(rep.entries.is_empty(), "{rep}");
    let (imp, _) = load("test-no-tracks.otio");
    let s = seq(&imp);
    assert!(s.video_tracks.iter().chain(&s.audio_tracks).all(|t| t.items.is_empty()));
}
