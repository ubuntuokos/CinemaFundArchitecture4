//! Smart Render planner (WP-05) tests: copied runs, re-encoded boundaries with the extra
//! reference pictures they decode, rendered gaps and changed clips, joined splits, codec
//! configuration seams, frame-rate mismatches, refusals and the registry blockers.

use cfa4_time::Tick;

use crate::capabilities::{Backend, Codec, Container, EncoderLimits, Entry, Key, Operation, Policy, Registry, StreamFormat, SupportLevel};
use crate::copy_planner::PlanRefusal;
use crate::edit_decision::{EditClip, EditContent, EditDecisionList, EditItem, RenderCause};
use crate::media_index::{DependencyStatus, FrameIndexEntry, MediaIndex, PictureType, RandomAccessKind, SourceFingerprint, StreamIndex};
use crate::plan::{CopyGuarantee, PlanSegment, SegmentMode, SegmentReason, Span};
use crate::smart_render::{RenderRefusal, SmartRenderPlan, SmartRenderRequest, TICK_TIMEBASE, plan_smart_render};
use crate::timestamp::{MediaTime, Rational};

const STRICT: Policy = Policy { allow_experimental: false };
/// Ticks per frame at 25 fps.
const FRAME: i64 = 254_016_000_000 / 25;

/// `n` frames at 25 fps.
fn f(n: i64) -> Tick {
    Tick(n * FRAME)
}

/// Closed GOPs of 7 pictures (decode order I P B B P B B; presentation I B B P B B P) at
/// `step` 90 kHz units per picture, with codec configuration 0.
fn closed_gops(count: u64, step: i64) -> Vec<FrameIndexEntry> {
    let mut out = Vec::new();
    let order =
        [(0, PictureType::I), (3, PictureType::P), (1, PictureType::B), (2, PictureType::B), (6, PictureType::P), (4, PictureType::B), (5, PictureType::B)];
    for g in 0..count {
        for (k, (p, ty)) in order.into_iter().enumerate() {
            let (decode, presentation) = (g * 7 + k as u64, g * 7 + p);
            let (ra, dep) = if k == 0 { (RandomAccessKind::Idr, DependencyStatus::Independent) } else { (RandomAccessKind::None, DependencyStatus::WithinGop) };
            out.push(FrameIndexEntry {
                stream_id: 1,
                decode_index: decode,
                presentation_index: Some(presentation),
                pts: Some(presentation as i64 * step),
                dts: Some(decode as i64 * step),
                keyframe: k == 0,
                picture_type: ty,
                reference: ty != PictureType::B,
                random_access: ra,
                dependency_status: dep,
                references: None,
                config_id: 0,
                size: 1000,
            });
        }
    }
    out
}

fn source(id: &str, codec: Codec, frames: Vec<FrameIndexEntry>) -> MediaIndex {
    let stream =
        StreamIndex { stream_id: 1, codec, timebase: Rational::MPEG_90K, frames, gops: Vec::new(), config_changes: Vec::new(), timestamp_problems: Vec::new() };
    MediaIndex::new(id, SourceFingerprint { size: 1, modified_ns: None, content_id: None }, vec![stream])
}

/// Two H.264 sources of 4 GOPs at 25 fps.
fn sources() -> Vec<MediaIndex> {
    vec![source("a.mp4", Codec::H264, closed_gops(4, 3600)), source("b.mp4", Codec::H264, closed_gops(4, 3600))]
}

fn h264_high() -> StreamFormat {
    StreamFormat { profile: "high".into(), chroma: "4:2:0".into(), bit_depth: 8 }
}

fn clip(source_id: &str, source_in: i64, config: &str) -> EditContent {
    EditContent::Clip(EditClip {
        source_id: source_id.into(),
        stream_id: 1,
        container: Container::Mp4,
        source_in: f(source_in),
        configs: vec![config.into()],
        format: Some(h264_high()),
    })
}

/// An item at output frame `at`, `len` frames long.
fn item(at: i64, len: i64, content: EditContent) -> EditItem {
    EditItem { output_in: f(at), duration: f(len), content, render_causes: Vec::new() }
}

fn edl(items: Vec<EditItem>) -> EditDecisionList {
    EditDecisionList { timeline_id: "sequence-1".into(), frame_duration: Rational::new(1, 25), items }
}

fn request() -> SmartRenderRequest {
    SmartRenderRequest { codec: Codec::H264, output_container: Container::Mp4, guarantee: CopyGuarantee::BitExact, output_config: None }
}

/// The built-in registry with H.264 packet copy and boundary re-encode into MP4 allowed.
fn ready_registry() -> Registry {
    let mut r = Registry::native();
    let row = |key: Key| Entry { key, backend: Backend::Native, level: SupportLevel::Supported, evidence: "test".into(), limits: None, routes: Vec::new() };
    for op in [Operation::Demux, Operation::PacketCopy, Operation::Mux, Operation::BoundaryReencode] {
        r.insert(row(Key::in_container(op, Codec::H264, Container::Mp4))).unwrap();
    }
    let limits = EncoderLimits { profiles: vec!["high".into()], chroma: vec!["4:2:0".into()], bit_depths: vec![8] };
    r.insert(Entry { limits: Some(limits), ..row(Key::codec(Operation::Encode, Codec::H264)) }).unwrap();
    r
}

fn plan(items: Vec<EditItem>) -> SmartRenderPlan {
    plan_smart_render(&ready_registry(), &sources(), &edl(items), &request(), STRICT).unwrap()
}

/// (mode, reason, first, last, decode first, decode last) of every segment.
fn shape(p: &SmartRenderPlan) -> Vec<(SegmentMode, SegmentReason, u64, u64, u64, u64)> {
    p.plan.segments.iter().map(|s| (s.mode, s.reason, s.presentation.first, s.presentation.last, s.decode.first, s.decode.last)).collect()
}

fn output_frame(s: &PlanSegment) -> i64 {
    assert_eq!(s.output_in.timebase, TICK_TIMEBASE);
    assert_eq!(s.output_in.value % FRAME, 0, "on the frame grid");
    s.output_in.value / FRAME
}

/// Segments follow each other in the output without gaps or overlaps, and fill the duration.
fn assert_covers(p: &SmartRenderPlan) {
    let mut at = 0;
    for s in &p.plan.segments {
        assert_eq!(output_frame(s), at, "{s:?}");
        at += s.frames() as i64;
    }
    assert_eq!(p.plan.expected_duration, MediaTime::new(at * FRAME, TICK_TIMEBASE));
}

use SegmentMode::{BoundaryReencode as Reencode, Render, StrictCopy as Copy};
use SegmentReason::*;

#[test]
fn a_clip_of_whole_gops_is_copied() {
    let p = plan(vec![item(0, 14, clip("a.mp4", 7, "x"))]);
    assert_eq!(shape(&p), [(Copy, WholeGops, 7, 20, 7, 20)]);
    let s = &p.plan.segments[0];
    assert_eq!(s.source.as_ref().map(|s| (s.source_id.as_str(), s.item)), Some(("a.mp4", 0)));
    assert!(p.plan.is_lossless() && p.executable, "{:?}", p.blockers);
    assert_eq!(p.plan.source_id, "sequence-1");
    assert_covers(&p);
}

#[test]
fn boundaries_decode_the_pictures_they_reference() {
    // source pictures 9–22: 9–13 lie in the GOP from 7, 14–20 is a whole GOP, 21–22 open the next
    // one; the IDR 21 alone still copies (decode and presentation order meet after it)
    let p = plan(vec![item(0, 14, clip("a.mp4", 9, "x"))]);
    // the head decodes from the IDR at 7 (pictures 7 and 8 only as references); the tail decodes
    // the IDR 21 and the P picture 24 (decode index 22) as references, because B picture 22
    // predicts from both
    assert_eq!(shape(&p), [(Reencode, HeadBoundary, 9, 13, 7, 13), (Copy, WholeGops, 14, 21, 14, 21), (Reencode, TailBoundary, 22, 22, 21, 23)]);
    assert!(p.plan.requires_confirmation());
    assert!(p.executable, "{:?}", p.blockers);
    assert_covers(&p);
}

#[test]
fn a_split_without_a_change_is_joined_before_planning() {
    // a razor cut at source picture 10 inside a GOP: planned apart, both sides would be re-encoded
    let p = plan(vec![item(0, 3, clip("a.mp4", 7, "x")), item(3, 11, clip("a.mp4", 10, "x"))]);
    assert_eq!(shape(&p), [(Copy, WholeGops, 7, 20, 7, 20)]);
    assert_covers(&p);
    // not joined: another source, a gap in the source, or a change on one side
    let p = plan(vec![item(0, 7, clip("a.mp4", 7, "x")), item(7, 7, clip("a.mp4", 21, "x"))]);
    assert_eq!(shape(&p).len(), 2);
    let mut changed = item(7, 7, clip("a.mp4", 14, "x"));
    changed.render_causes = vec![RenderCause::Effect];
    let p = plan(vec![item(0, 7, clip("a.mp4", 7, "x")), changed]);
    assert_eq!(shape(&p), [(Copy, WholeGops, 7, 13, 7, 13), (Render, Transformed, 7, 13, 7, 13)]);
}

#[test]
fn gaps_and_changed_clips_are_rendered() {
    let mut fx = item(14, 7, clip("b.mp4", 0, "x"));
    fx.render_causes = vec![RenderCause::Transition, RenderCause::Effect, RenderCause::Effect];
    let p = plan(vec![item(7, 7, clip("a.mp4", 0, "x")), fx, item(21, 7, EditContent::Gap)]);
    assert_eq!(shape(&p), [(Render, Gap, 0, 6, 0, 6), (Copy, WholeGops, 0, 6, 0, 6), (Render, Transformed, 14, 20, 14, 20), (Render, Gap, 21, 27, 21, 27)]);
    let fx = &p.plan.segments[2];
    assert_eq!(fx.render_causes, [RenderCause::Effect, RenderCause::Transition], "sorted, without repeats");
    assert_eq!(fx.source.as_ref().map(|s| s.item), Some(1));
    assert_eq!(fx.source_in, MediaTime::new(0, TICK_TIMEBASE));
    assert!(p.plan.segments[0].source.is_none(), "a gap reads no source");
    assert!(p.plan.requires_confirmation());
    assert_covers(&p);
}

#[test]
fn a_run_with_another_codec_configuration_is_re_encoded_at_the_seam() {
    // the first copied run sets the output stream; b.mp4 has another configuration
    let p = plan(vec![item(0, 7, clip("a.mp4", 0, "avcC-1")), item(7, 7, clip("b.mp4", 7, "avcC-2"))]);
    assert_eq!(shape(&p), [(Copy, WholeGops, 0, 6, 0, 6), (Reencode, IncompatibleStream, 7, 13, 7, 13)]);
    // the export settings fix the output configuration: now a.mp4 is the odd one
    let req = SmartRenderRequest { output_config: Some("avcC-2".into()), ..request() };
    let tl = edl(vec![item(0, 7, clip("a.mp4", 0, "avcC-1")), item(7, 7, clip("b.mp4", 7, "avcC-2"))]);
    let p = plan_smart_render(&ready_registry(), &sources(), &tl, &req, STRICT).unwrap();
    assert_eq!(shape(&p), [(Reencode, IncompatibleStream, 0, 6, 0, 6), (Copy, WholeGops, 7, 13, 7, 13)]);
    // an unknown configuration is never copied
    let EditContent::Clip(mut c) = clip("a.mp4", 0, "x") else { unreachable!() };
    c.configs.clear();
    let p = plan(vec![item(0, 7, EditContent::Clip(c))]);
    assert_eq!(shape(&p), [(Reencode, IncompatibleStream, 0, 6, 0, 6)]);
}

#[test]
fn another_codec_or_frame_rate_is_rendered() {
    let mut indexes = sources();
    indexes.push(source("m2v.ts", Codec::Mpeg2Video, closed_gops(2, 3600)));
    indexes.push(source("50p.mp4", Codec::H264, closed_gops(4, 1800)));
    let tl = edl(vec![item(0, 7, clip("m2v.ts", 0, "x")), item(7, 7, clip("50p.mp4", 0, "x"))]);
    let p = plan_smart_render(&ready_registry(), &indexes, &tl, &request(), STRICT).unwrap();
    assert_eq!(shape(&p), [(Render, IncompatibleStream, 0, 6, 0, 6), (Render, Transformed, 7, 13, 7, 13)]);
    assert_eq!(p.plan.segments[1].render_causes, [RenderCause::FrameRate], "14 source pictures cannot fill 7 output frames");
    assert_covers(&p);
}

#[test]
fn invalid_timelines_are_refused() {
    let run = |items: Vec<EditItem>| plan_smart_render(&ready_registry(), &sources(), &edl(items), &request(), STRICT).unwrap_err();
    assert_eq!(run(vec![]), RenderRefusal::EmptyTimeline);
    let mut odd = edl(vec![item(0, 7, clip("a.mp4", 0, "x"))]);
    odd.frame_duration = Rational::new(1, 11);
    assert!(matches!(plan_smart_render(&ready_registry(), &sources(), &odd, &request(), STRICT), Err(RenderRefusal::InvalidFrameRate { .. })));
    let mut half = item(0, 7, clip("a.mp4", 0, "x"));
    half.duration = Tick(FRAME / 2);
    assert_eq!(run(vec![half]), RenderRefusal::OffGrid { item: 0 });
    assert_eq!(run(vec![item(0, 0, EditContent::Gap)]), RenderRefusal::OffGrid { item: 0 });
    assert_eq!(run(vec![item(0, 7, EditContent::Gap), item(5, 7, EditContent::Gap)]), RenderRefusal::Overlap { item: 1 });
    assert_eq!(run(vec![item(0, 7, clip("c.mp4", 0, "x"))]), RenderRefusal::UnknownSource { item: 0, source_id: "c.mp4".into() });
    let EditContent::Clip(mut c) = clip("a.mp4", 0, "x") else { unreachable!() };
    c.stream_id = 9;
    assert_eq!(run(vec![item(0, 7, EditContent::Clip(c))]), RenderRefusal::UnknownStream { item: 0, stream_id: 9 });
    // past the end of the source (28 pictures)
    assert!(matches!(run(vec![item(0, 7, clip("a.mp4", 25, "x"))]), RenderRefusal::Clip { item: 0, refusal: PlanRefusal::OutOfRange }));
}

#[test]
fn the_registry_decides_whether_the_plan_runs() {
    let tl = edl(vec![item(0, 14, clip("a.mp4", 9, "x")), item(14, 7, EditContent::Gap)]);
    // the built-in registry: H.264 packet copy into MP4 (WP-04), no boundary re-encode yet (WP-06)
    let p = plan_smart_render(&Registry::native(), &sources(), &tl, &request(), STRICT).unwrap();
    assert!(!p.executable);
    assert!(!p.blockers.iter().any(|b| b.contains("packet-copy")), "{:?}", p.blockers);
    assert!(p.blockers.iter().any(|b| b.contains("boundary re-encode")), "{:?}", p.blockers);
    let copy_only = edl(vec![item(0, 14, clip("a.mp4", 7, "x"))]);
    let p = plan_smart_render(&Registry::native(), &sources(), &copy_only, &request(), STRICT).unwrap();
    assert!(p.executable, "{:?}", p.blockers);
    let unique: std::collections::BTreeSet<&String> = p.blockers.iter().collect();
    assert_eq!(unique.len(), p.blockers.len(), "no repeats");
    // a re-encoded boundary needs the stream format for the encoder limits
    let EditContent::Clip(mut c) = clip("a.mp4", 9, "x") else { unreachable!() };
    c.format = None;
    let p = plan_smart_render(&ready_registry(), &sources(), &edl(vec![item(0, 14, EditContent::Clip(c))]), &request(), STRICT).unwrap();
    assert!(!p.executable);
    assert!(p.blockers.iter().any(|b| b.contains("item 0") && b.contains("format")), "{:?}", p.blockers);
    // a bit-exact copy cannot change the packet framing (MP4 into MPEG-TS)
    let mut r = ready_registry();
    for op in [Operation::PacketCopy, Operation::Mux] {
        r.insert(Entry {
            key: Key::in_container(op, Codec::H264, Container::MpegTs),
            backend: Backend::Native,
            level: SupportLevel::Supported,
            evidence: "test".into(),
            limits: None,
            routes: Vec::new(),
        })
        .unwrap();
    }
    let ts = SmartRenderRequest { output_container: Container::MpegTs, ..request() };
    let p = plan_smart_render(&r, &sources(), &edl(vec![item(0, 7, clip("a.mp4", 0, "x"))]), &ts, STRICT).unwrap();
    assert!(!p.executable);
}

#[test]
fn plans_are_deterministic_and_serde() {
    let mut fx = item(14, 7, clip("b.mp4", 0, "y"));
    fx.render_causes = vec![RenderCause::Speed];
    let items = vec![item(0, 14, clip("a.mp4", 9, "x")), fx, item(28, 7, clip("b.mp4", 14, "x"))];
    let (a, b) = (plan(items.clone()), plan(items));
    assert_eq!(a, b);
    let json = serde_json::to_string(&a).unwrap();
    assert_eq!(serde_json::from_str::<SmartRenderPlan>(&json).unwrap(), a);
    assert!(json.contains(r#""render_causes":["speed"]"#), "{json}");
    assert!(json.contains(r#""reason":"gap""#), "{json}");
    // a plan of one clip has no per-segment source, and reads as it did before
    assert!(!serde_json::to_string(&a.plan.segments[0].clone()).unwrap().contains("render_causes"));
    let tl = edl(vec![item(0, 7, clip("a.mp4", 0, "x"))]);
    let json = serde_json::to_string(&tl).unwrap();
    assert_eq!(serde_json::from_str::<EditDecisionList>(&json).unwrap(), tl);
    assert_covers(&a);
    assert_eq!(
        a.plan.segments.iter().filter(|s| s.mode == Render).map(|s| s.presentation).collect::<Vec<_>>(),
        [Span { first: 14, last: 20 }, Span { first: 21, last: 27 }]
    );
}

#[test]
fn random_timelines_cover_the_output_and_decode_every_reference() {
    let indexes = sources();
    let frames = &indexes[0].streams[0].frames;
    let decode_of = |p: u64| frames.iter().find(|f| f.presentation_index == Some(p)).map(|f| f.decode_index).unwrap();
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    let mut planned = 0;
    for _ in 0..500 {
        let mut items = Vec::new();
        let mut at = next(3) as i64;
        for _ in 0..1 + next(4) {
            let len = 1 + next(10) as i64;
            let src = next(28 - len as u64) as i64;
            let mut it = item(at, len, if next(5) == 0 { EditContent::Gap } else { clip(["a.mp4", "b.mp4"][next(2) as usize], src, "x") });
            if next(4) == 0 {
                it.render_causes = vec![RenderCause::Effect];
            }
            items.push(it);
            at += len + next(2) as i64;
        }
        let p = plan(items);
        planned += 1;
        assert_covers(&p);
        for s in p.plan.segments.iter().filter(|s| s.mode != Render) {
            let first = frames.iter().find(|f| f.decode_index == s.decode.first).unwrap();
            assert!(first.random_access.is_clean(), "{s:?} starts decoding at a clean access point");
            let needed = (s.presentation.first..=s.presentation.last).map(decode_of).max().unwrap();
            assert!(s.decode.last >= needed, "{s:?} decodes every picture it outputs");
            if s.mode.is_copy() {
                assert_eq!(s.decode.count(), s.presentation.count(), "a copy outputs exactly its packets");
            }
        }
    }
    assert_eq!(planned, 500);
}
