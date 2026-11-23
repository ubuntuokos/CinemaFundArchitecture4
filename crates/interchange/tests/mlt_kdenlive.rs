//! Kdenlive `.kdenlive` projects (K1, shared MLT core with Shotcut SC-5.1): the CC0 test dataset of
//! Kdenlive (`tests/data/kdenlive-mlt/README.md`). Expected values are read off the documents, not off
//! our own output.

mod support;

use cfa4_interchange::{Format, Imported, Report, detect, import};
use cfa4_project::{Project, Sequence, Track};
use cfa4_time::{FrameRate, Tick};
use support::*;

const BASE: &str = "/proj";

fn load(name: &str) -> (Imported, Report) {
    let path = format!("{}/tests/data/kdenlive-mlt/{name}", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    assert_eq!(detect(&bytes, Some("kdenlive")), Some(Format::Mlt), "{name}");
    import(&bytes, Format::Mlt, Some(BASE)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn seq(imp: &Imported) -> &Sequence {
    imp.project.sequence(only_seq(imp)).expect("sequence")
}

fn spans(t: &Track) -> Vec<(i64, i64)> {
    let r = FrameRate::new(25, 1);
    t.items.iter().map(|i| (r.frame_at(i.start), r.frame_at(i.duration))).collect()
}

fn non_empty(tracks: &[Track]) -> Vec<&Track> {
    tracks.iter().filter(|t| !t.items.is_empty()).collect()
}

#[allow(dead_code)]
fn media_of(p: &Project, t: &Track) -> Vec<String> {
    t.items.iter().map(|i| media_key(p, i.item)).collect()
}

#[test]
fn av_project_keeps_the_profile_and_the_track_layout() {
    let (imp, rep) = load("av.kdenlive");
    let s = seq(&imp);
    assert_eq!((s.settings.width, s.settings.height), (1920, 1080));
    assert_eq!(s.settings.frame_rate, FrameRate::new(25, 1));
    // two audio and two video tracks, in document order
    assert_eq!(s.audio_tracks.len(), 2);
    assert_eq!(s.video_tracks.len(), 2);
    // the video clip is 0…19.96 s inclusive = 500 frames
    let v = non_empty(&s.video_tracks);
    assert_eq!(v.len(), 1);
    assert_eq!(spans(v[0]), vec![(0, 500)]);
    // blipflash is not a CFA4 generator: said so, never an error
    assert!(rep.mentions("blipflash"), "{rep}");
    assert!(rep.mentions("Kdenlive project"), "{rep}");
}

#[test]
fn mix_project_places_clips_with_blanks_and_reports_the_mix() {
    let (imp, rep) = load("test-mix.kdenlive");
    let s = seq(&imp);
    assert_eq!((s.settings.width, s.settings.height), (720, 576));
    // video track: blank 5.08 s, 5.44 s + 0.04 s = 137 frames, blank 4 s, 3.96 s + 0.04 s = 100 frames
    let video: Vec<Vec<(i64, i64)>> = s.video_tracks.iter().map(spans).collect();
    assert!(video.iter().any(|t| t == &vec![(127, 137), (364, 100)]), "{video:?}");
    assert!(rep.mentions("luma"), "{rep}");
}

#[test]
fn missing_media_is_kept_as_a_path_not_an_error() {
    let (imp, rep) = load("test-missing.kdenlive");
    let s = seq(&imp);
    // blank 11.2 s (280 frames), then a 5 s clip (0…4.96 s inclusive = 125 frames) of missing media
    let v = non_empty(&s.video_tracks);
    assert_eq!(v.len(), 1);
    assert_eq!(spans(v[0]), vec![(280, 125)]);
    // the clip has a brightness effect and a LUT: not imported yet, and said so
    assert!(rep.mentions("brightness"), "{rep}");
    assert!(rep.mentions("not imported yet"), "{rep}");
}

#[test]
fn keyframes_and_nesting_open_without_panicking() {
    for name in ["test-keyframes.kdenlive", "test-nesting.kdenlive"] {
        let (imp, _rep) = load(name);
        assert!(!imp.sequences.is_empty(), "{name}");
    }
}

#[test]
fn hostile_inputs_never_panic() {
    let cases: &[&str] = &[
        "",
        "<mlt/>",
        "<mlt><profile frame_rate_num=\"0\" frame_rate_den=\"0\"/></mlt>",
        "<mlt><profile frame_rate_num=\"25\" frame_rate_den=\"1\"/></mlt>",
        "<mlt><profile frame_rate_num=\"25\" frame_rate_den=\"1\"/><tractor id=\"t\"><track producer=\"t\"/></tractor></mlt>",
        "<mlt><profile frame_rate_num=\"25\" frame_rate_den=\"1\"/><playlist id=\"p\"><entry producer=\"p\" in=\"x\" out=\"y\"/></playlist><tractor id=\"t\"><track producer=\"p\"/></tractor></mlt>",
        "<!DOCTYPE mlt [<!ENTITY a 'b'>]><mlt/>",
        "<mlt><profile frame_rate_num=\"25\" frame_rate_den=\"1\"/><producer id=\"a\" out=\"99999999999999999999\"/><playlist id=\"p\"><entry producer=\"a\" in=\"-5\" out=\"1e400\"/><blank length=\"99:99:99.999\"/></playlist><tractor id=\"t\"><track producer=\"p\"/></tractor></mlt>",
    ];
    for c in cases {
        let _ = import(c.as_bytes(), Format::Mlt, Some(BASE));
    }
}

#[test]
fn mlt_is_import_only() {
    assert!(!Format::Mlt.can_export());
    let (imp, _) = load("av.kdenlive");
    let seq = only_seq(&imp);
    let err = cfa4_interchange::export(&imp.project, seq, Format::Mlt, &Default::default()).unwrap_err();
    assert!(err.to_string().contains("not exported"), "{err}");
    let _ = Tick::ZERO;
}

#[test]
fn av_group_links_picture_and_sound() {
    // a picture clip and its sound on the first two tracks (audio, video; numbered without the background track), grouped by `AVSplit`; a
    // third clip stays alone
    let doc = r#"<mlt><profile frame_rate_num="25" frame_rate_den="1" width="1280" height="720"/>
<producer id="c" in="0" out="249"><property name="mlt_service">avformat-novalidate</property><property name="resource">/m/a.mp4</property></producer>
<producer id="black" in="0" out="9999"><property name="mlt_service">color</property><property name="resource">black</property></producer>
<playlist id="pa"><entry producer="c" in="0" out="99"/><blank length="25"/><entry producer="c" in="100" out="149"/></playlist>
<playlist id="pv"><entry producer="c" in="0" out="99"/><blank length="25"/><entry producer="c" in="100" out="149"/></playlist>
<tractor id="ta"><property name="kdenlive:audio_track">1</property><track producer="pa"/></tractor>
<tractor id="tv"><track producer="pv"/></tractor>
<tractor id="main"><property name="kdenlive:sequenceproperties.groups">[{"type":"AVSplit","children":[{"data":"0:0","leaf":"clip","type":"Leaf"},{"data":"1:0","leaf":"clip","type":"Leaf"}]}]</property>
<track producer="black"/><track producer="ta"/><track producer="tv"/></tractor></mlt>"#;
    let (imp, _rep) = import(doc.as_bytes(), Format::Mlt, Some(BASE)).unwrap();
    let s = seq(&imp);
    assert_eq!((s.video_tracks.len(), s.audio_tracks.len()), (1, 1));
    let (v, a) = (&s.video_tracks[0].items, &s.audio_tracks[0].items);
    assert_eq!((v.len(), a.len()), (2, 2));
    assert!(v[0].link.is_some() && v[0].link == a[0].link, "{:?} {:?}", v[0].link, a[0].link);
    assert!(v[1].link.is_none() && a[1].link.is_none());
}

#[test]
fn guides_become_sequence_markers() {
    let doc = r#"<mlt><profile frame_rate_num="25" frame_rate_den="1" width="1280" height="720"/>
<playlist id="p0"><property name="kdenlive:audio_track">1</property></playlist>
<tractor id="t0"><property name="kdenlive:audio_track">1</property><track producer="p0"/></tractor>
<tractor id="main"><property name="kdenlive:sequenceproperties.guides">[{"comment":"Intro","pos":50,"type":0},{"comment":"Bad","pos":-3,"type":0},{"comment":"Outro","pos":250,"type":1}]</property><track producer="t0"/></tractor></mlt>"#;
    let (imp, _) = import(doc.as_bytes(), Format::Mlt, Some(BASE)).unwrap();
    let s = seq(&imp);
    let m: Vec<(String, i64)> = s.markers.iter().map(|m| (m.name.clone(), FrameRate::new(25, 1).frame_at(m.start))).collect();
    assert_eq!(m, vec![("Intro".to_string(), 50), ("Outro".to_string(), 250)]);
}
