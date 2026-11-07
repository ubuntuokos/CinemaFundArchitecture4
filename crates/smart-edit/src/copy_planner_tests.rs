//! Smart Cut planner (WP-03) tests: synthetic GOP structures, the edge cases of CFA4-SCE-001
//! §13.4 (first / last picture, GOP boundary, mid-GOP, open GOP, leading pictures, missing and
//! repeated timestamps, empty, negative and too long ranges, unsupported combinations) and
//! property checks on random indexes.

use cfa4_time::Tick;

use crate::capabilities::{Backend, Codec, Container, EncoderLimits, Entry, Key, Operation, Policy, Registry, StreamFormat, SupportLevel};
use crate::copy_planner::{CutMode, PlanRefusal, SmartCutPlan, SmartCutRequest, plan_smart_cut};
use crate::media_index::{DependencyStatus, FrameIndexEntry, MediaIndex, PictureType, RandomAccessKind, SourceFingerprint, StreamIndex};
use crate::plan::{CopyGuarantee, SegmentMode, SegmentReason, Span};
use crate::timestamp::Rational;

const STRICT: Policy = Policy { allow_experimental: false };
/// 25 fps on a 90 kHz timebase.
const STEP: i64 = 3600;
/// Ticks per 90 kHz unit.
const TICKS_PER_UNIT: i64 = 254_016_000_000 / 90_000;

/// The source time of presentation position `p`.
fn t(p: i64) -> Tick {
    Tick(p * STEP * TICKS_PER_UNIT)
}

fn frame(decode: u64, presentation: u64, picture_type: PictureType, random_access: RandomAccessKind, dependency_status: DependencyStatus) -> FrameIndexEntry {
    FrameIndexEntry {
        stream_id: 1,
        decode_index: decode,
        presentation_index: Some(presentation),
        pts: Some(presentation as i64 * STEP),
        dts: Some(decode as i64 * STEP),
        keyframe: random_access.is_access_point(),
        picture_type,
        reference: picture_type != PictureType::B,
        random_access,
        dependency_status,
        references: None,
        config_id: 0,
        size: 1000,
    }
}

/// Closed GOPs of 7 pictures, I B B P B B P in presentation order (decode: I P B B P B B).
fn closed_gops(count: u64) -> Vec<FrameIndexEntry> {
    let mut out = Vec::new();
    for g in 0..count {
        let base = g * 7;
        let order =
            [(0, PictureType::I), (3, PictureType::P), (1, PictureType::B), (2, PictureType::B), (6, PictureType::P), (4, PictureType::B), (5, PictureType::B)];
        for (k, (p, ty)) in order.into_iter().enumerate() {
            let (ra, dep) = if k == 0 { (RandomAccessKind::Idr, DependencyStatus::Independent) } else { (RandomAccessKind::None, DependencyStatus::WithinGop) };
            out.push(frame(base + k as u64, base + p, ty, ra, dep));
        }
    }
    out
}

fn stream(frames: Vec<FrameIndexEntry>) -> StreamIndex {
    StreamIndex {
        stream_id: 1,
        codec: Codec::H264,
        timebase: Rational::MPEG_90K,
        frames,
        gops: Vec::new(),
        config_changes: Vec::new(),
        timestamp_problems: Vec::new(),
    }
}

fn index(s: StreamIndex) -> MediaIndex {
    MediaIndex::new("clip", SourceFingerprint { size: 1, modified_ns: None, content_id: None }, vec![s])
}

fn request(in_p: i64, out_p: i64, mode: CutMode) -> SmartCutRequest {
    SmartCutRequest {
        stream_id: 1,
        in_point: t(in_p),
        out_point: t(out_p),
        mode,
        guarantee: CopyGuarantee::BitExact,
        source_container: Container::Mp4,
        output_container: Container::Mp4,
        format: Some(StreamFormat { profile: "high".into(), chroma: "4:2:0".into(), bit_depth: 8 }),
    }
}

/// A registry in which everything a Smart Cut / Smart Render of H.264 MP4 needs is supported (the
/// executor of WP-04 does not exist yet, so the built-in one refuses packet copy).
fn ready_registry() -> Registry {
    let mut r = Registry::native();
    let row = |key: Key| Entry { key, backend: Backend::Native, level: SupportLevel::Supported, evidence: "test".into(), limits: None, routes: Vec::new() };
    for op in [Operation::Demux, Operation::PacketCopy, Operation::Mux, Operation::BoundaryReencode] {
        r.insert(row(Key::in_container(op, Codec::H264, Container::Mp4))).unwrap();
    }
    for op in [Operation::GopIndex, Operation::Decode] {
        r.insert(row(Key::codec(op, Codec::H264))).unwrap();
    }
    let limits = EncoderLimits { profiles: vec!["high".into()], chroma: vec!["4:2:0".into()], bit_depths: vec![8] };
    r.insert(Entry { limits: Some(limits), ..row(Key::codec(Operation::Encode, Codec::H264)) }).unwrap();
    r
}

fn plan(frames: Vec<FrameIndexEntry>, req: &SmartCutRequest) -> Result<SmartCutPlan, PlanRefusal> {
    plan_smart_cut(&ready_registry(), &index(stream(frames)), req, STRICT)
}

fn modes(p: &SmartCutPlan) -> Vec<(SegmentMode, SegmentReason, u64, u64)> {
    p.plan.segments.iter().map(|s| (s.mode, s.reason, s.presentation.first, s.presentation.last)).collect()
}

#[test]
fn a_whole_gop_is_copied_unchanged() {
    let p = plan(closed_gops(3), &request(7, 14, CutMode::GopBoundary)).unwrap();
    assert_eq!(modes(&p), [(SegmentMode::StrictCopy, SegmentReason::WholeGops, 7, 13)]);
    let s = &p.plan.segments[0];
    assert_eq!(s.decode, Span { first: 7, last: 13 });
    assert_eq!((s.source_in.value, s.source_out.value, s.output_in.value), (7 * STEP, 14 * STEP, 0));
    assert_eq!(p.plan.expected_duration.value, 7 * STEP);
    assert!(p.plan.is_lossless() && !p.plan.requires_confirmation() && p.plan.requires_validation);
    assert!(p.executable, "{:?}", p.blockers);
    assert_eq!((p.plan.copied_frames(), p.plan.reencoded_frames()), (7, 0));
}

#[test]
fn first_and_last_picture_cuts() {
    // the whole stream, from the first picture to the end of the last one
    let p = plan(closed_gops(3), &request(0, 21, CutMode::GopBoundary)).unwrap();
    assert_eq!(modes(&p), [(SegmentMode::StrictCopy, SegmentReason::WholeGops, 0, 20)]);
    // only the first picture: an I picture alone is a copy span
    let p = plan(closed_gops(3), &request(0, 1, CutMode::GopBoundary)).unwrap();
    assert_eq!(modes(&p), [(SegmentMode::StrictCopy, SegmentReason::WholeGops, 0, 0)]);
    // only the last picture: a B picture, re-encoded from its GOP's access point
    let p = plan(closed_gops(3), &request(20, 21, CutMode::FrameAccurate)).unwrap();
    assert_eq!(modes(&p), [(SegmentMode::BoundaryReencode, SegmentReason::NoCopyableRange, 20, 20)]);
    assert_eq!(p.plan.segments[0].decode, Span { first: 14, last: 18 });
}

#[test]
fn a_mid_gop_cut_is_refused_in_gop_boundary_mode_with_the_nearest_safe_points() {
    let err = plan(closed_gops(3), &request(9, 17, CutMode::GopBoundary)).unwrap_err();
    let PlanRefusal::NotAtSafePoints { first, last, safe } = err else { panic!("{err:?}") };
    assert_eq!((first, last), (9, 16));
    assert_eq!(safe.in_before.map(|s| s.presentation), Some(7));
    assert_eq!(safe.in_after.map(|s| s.presentation), Some(14));
    assert_eq!(safe.in_after.map(|s| s.time.value), Some(14 * STEP));
    // copy ends where decode and presentation order meet: after P picture 14 and after 17
    assert_eq!(safe.out_before.map(|s| s.presentation), Some(14));
    assert_eq!(safe.out_after.map(|s| (s.presentation, s.time.value)), Some((17, 18 * STEP)));
}

#[test]
fn a_frame_accurate_cut_copies_whole_gops_and_reencodes_the_head() {
    let p = plan(closed_gops(3), &request(9, 21, CutMode::FrameAccurate)).unwrap();
    assert_eq!(modes(&p), [(SegmentMode::BoundaryReencode, SegmentReason::HeadBoundary, 9, 13), (SegmentMode::StrictCopy, SegmentReason::WholeGops, 14, 20)]);
    let head = &p.plan.segments[0];
    // decoded from the GOP's IDR picture; the pictures it needs come up to decode position 13
    assert_eq!(head.decode, Span { first: 7, last: 13 });
    assert_eq!(p.plan.segments[1].output_in.value, 5 * STEP);
    assert_eq!(p.plan.expected_duration.value, 12 * STEP);
    assert!(!p.plan.is_lossless() && p.plan.requires_confirmation());
    assert_eq!((p.plan.copied_frames(), p.plan.reencoded_frames()), (7, 5));
    assert!(p.executable, "{:?}", p.blockers);
}

#[test]
fn a_frame_accurate_cut_reencodes_head_and_tail() {
    let p = plan(closed_gops(4), &request(3, 26, CutMode::FrameAccurate)).unwrap();
    assert_eq!(
        modes(&p),
        [
            (SegmentMode::BoundaryReencode, SegmentReason::HeadBoundary, 3, 6),
            (SegmentMode::StrictCopy, SegmentReason::WholeGops, 7, 24),
            (SegmentMode::BoundaryReencode, SegmentReason::TailBoundary, 25, 25),
        ]
    );
    // the segments cover the range without gaps, in output order
    let s = &p.plan.segments;
    assert!(s.windows(2).all(|w| w[0].presentation.last + 1 == w[1].presentation.first && w[0].source_out == w[1].source_in));
}

#[test]
fn a_range_inside_one_gop_is_reencoded_as_a_whole() {
    let p = plan(closed_gops(2), &request(8, 10, CutMode::FrameAccurate)).unwrap();
    assert_eq!(modes(&p), [(SegmentMode::BoundaryReencode, SegmentReason::NoCopyableRange, 8, 9)]);
    assert_eq!(p.plan.segments[0].decode, Span { first: 7, last: 10 });
}

#[test]
fn a_cut_point_between_two_pictures_keeps_the_picture_on_screen() {
    let req = SmartCutRequest { in_point: Tick(t(7).0 + 1), out_point: Tick(t(14).0 - 1), ..request(0, 0, CutMode::FrameAccurate) };
    let p = plan(closed_gops(3), &req).unwrap();
    assert_eq!(modes(&p), [(SegmentMode::StrictCopy, SegmentReason::WholeGops, 7, 13)]);
}

#[test]
fn leading_pictures_of_a_closed_gop_are_copied_with_it() {
    // IDR shown third: decode I(2) B(0) B(1) P(5) B(3) B(4), then the next GOP
    let mut frames = vec![
        frame(0, 2, PictureType::I, RandomAccessKind::Idr, DependencyStatus::Independent),
        frame(1, 0, PictureType::B, RandomAccessKind::None, DependencyStatus::WithinGop),
        frame(2, 1, PictureType::B, RandomAccessKind::None, DependencyStatus::WithinGop),
        frame(3, 5, PictureType::P, RandomAccessKind::None, DependencyStatus::WithinGop),
        frame(4, 3, PictureType::B, RandomAccessKind::None, DependencyStatus::WithinGop),
        frame(5, 4, PictureType::B, RandomAccessKind::None, DependencyStatus::WithinGop),
    ];
    frames.extend(closed_gops(2).into_iter().map(|mut f| {
        f.decode_index += 6;
        f.presentation_index = f.presentation_index.map(|p| p + 6);
        f.pts = f.pts.map(|v| v + 6 * STEP);
        f
    }));
    let p = plan(frames.clone(), &request(0, 6, CutMode::GopBoundary)).unwrap();
    assert_eq!(modes(&p), [(SegmentMode::StrictCopy, SegmentReason::WholeGops, 0, 5)]);
    assert_eq!(p.plan.segments[0].decode, Span { first: 0, last: 5 });
    // a cut-in at the IDR itself would drop its leading pictures: not a GOP boundary
    let err = plan(frames, &request(2, 6, CutMode::GopBoundary)).unwrap_err();
    let PlanRefusal::NotAtSafePoints { safe, .. } = err else { panic!("{err:?}") };
    assert_eq!(safe.in_before.map(|s| s.presentation), Some(0));
    assert_eq!(safe.in_after.map(|s| s.presentation), Some(6));
}

#[test]
fn an_open_gop_is_no_copy_start() {
    // second GOP opens with an open-GOP I picture whose leading B pictures refer to GOP 1
    let mut frames = closed_gops(3);
    for f in frames.iter_mut().filter(|f| (7..14).contains(&f.decode_index)) {
        if f.decode_index == 7 {
            f.random_access = RandomAccessKind::OpenGop { broken_link: false };
        }
    }
    let err = plan(frames.clone(), &request(7, 14, CutMode::GopBoundary)).unwrap_err();
    let PlanRefusal::NotAtSafePoints { safe, .. } = err else { panic!("{err:?}") };
    assert_eq!(safe.in_before.map(|s| s.presentation), Some(0));
    assert_eq!(safe.in_after.map(|s| s.presentation), Some(14));
    // frame-accurate: the open GOP is re-encoded from the closed GOP before it
    let p = plan(frames, &request(7, 21, CutMode::FrameAccurate)).unwrap();
    assert_eq!(modes(&p), [(SegmentMode::BoundaryReencode, SegmentReason::HeadBoundary, 7, 13), (SegmentMode::StrictCopy, SegmentReason::WholeGops, 14, 20)]);
    assert_eq!(p.plan.segments[0].decode.first, 0);
}

#[test]
fn pictures_crossing_an_access_point_end_a_copy_span() {
    let mut frames = closed_gops(2);
    // a leading picture of GOP 2 that needs GOP 1
    if let Some(f) = frames.iter_mut().find(|f| f.decode_index == 9) {
        f.dependency_status = DependencyStatus::CrossesAccessPoint;
    }
    assert!(matches!(plan(frames, &request(7, 14, CutMode::GopBoundary)), Err(PlanRefusal::NotAtSafePoints { .. })));
}

#[test]
fn a_stream_starting_mid_gop_has_undecodable_head_pictures() {
    let mut frames = vec![
        frame(0, 0, PictureType::P, RandomAccessKind::None, DependencyStatus::Undecodable),
        frame(1, 1, PictureType::P, RandomAccessKind::None, DependencyStatus::Undecodable),
    ];
    frames.extend(closed_gops(1).into_iter().map(|mut f| {
        f.decode_index += 2;
        f.presentation_index = f.presentation_index.map(|p| p + 2);
        f.pts = f.pts.map(|v| v + 2 * STEP);
        f
    }));
    assert_eq!(plan(frames.clone(), &request(0, 9, CutMode::FrameAccurate)), Err(PlanRefusal::UndecodablePictures { first: 0, last: 1 }));
    // from the first IDR it copies
    let p = plan(frames, &request(2, 9, CutMode::GopBoundary)).unwrap();
    assert_eq!(modes(&p), [(SegmentMode::StrictCopy, SegmentReason::WholeGops, 2, 8)]);
}

#[test]
fn a_configuration_change_ends_a_copy_span() {
    let mut frames = closed_gops(2);
    for f in frames.iter_mut().filter(|f| f.decode_index >= 7) {
        f.config_id = 1;
    }
    assert!(matches!(plan(frames.clone(), &request(0, 14, CutMode::GopBoundary)), Err(PlanRefusal::NotAtSafePoints { .. })));
    // each configuration on its own copies
    let p = plan(frames.clone(), &request(7, 14, CutMode::GopBoundary)).unwrap();
    assert!(p.plan.is_lossless());
    // frame-accurate: the first configuration's GOP is copied, nothing is joined across the change
    let p = plan(frames, &request(0, 14, CutMode::FrameAccurate)).unwrap();
    assert_eq!(modes(&p), [(SegmentMode::StrictCopy, SegmentReason::WholeGops, 0, 6), (SegmentMode::BoundaryReencode, SegmentReason::TailBoundary, 7, 13)]);
}

#[test]
fn invalid_requests_are_refused() {
    let f = closed_gops(2);
    assert_eq!(plan(f.clone(), &request(5, 5, CutMode::FrameAccurate)), Err(PlanRefusal::EmptyRange));
    assert_eq!(plan(f.clone(), &request(5, 3, CutMode::FrameAccurate)), Err(PlanRefusal::EmptyRange));
    assert_eq!(plan(f.clone(), &request(-1, 3, CutMode::FrameAccurate)), Err(PlanRefusal::OutOfRange));
    assert_eq!(plan(f.clone(), &request(0, 15, CutMode::FrameAccurate)), Err(PlanRefusal::OutOfRange));
    let req = SmartCutRequest { stream_id: 9, ..request(0, 7, CutMode::FrameAccurate) };
    assert_eq!(plan(f.clone(), &req), Err(PlanRefusal::UnknownStream { stream_id: 9 }));
    assert_eq!(plan(Vec::new(), &request(0, 7, CutMode::FrameAccurate)), Err(PlanRefusal::EmptyStream));
    let req = SmartCutRequest { in_point: Tick::MIN, out_point: Tick::MAX, ..request(0, 0, CutMode::FrameAccurate) };
    assert_eq!(plan(f, &req), Err(PlanRefusal::OutOfRange));
}

#[test]
fn missing_or_repeated_timestamps_are_refused() {
    let mut f = closed_gops(2);
    f[3].pts = None;
    assert_eq!(plan(f, &request(0, 7, CutMode::FrameAccurate)), Err(PlanRefusal::MissingTimestamps { decode_index: 3 }));
    let mut f = closed_gops(2);
    f[4].presentation_index = None;
    assert_eq!(plan(f, &request(0, 7, CutMode::FrameAccurate)), Err(PlanRefusal::MissingTimestamps { decode_index: 4 }));
    let mut f = closed_gops(2);
    f[2].pts = f[3].pts;
    assert!(matches!(plan(f, &request(0, 7, CutMode::FrameAccurate)), Err(PlanRefusal::InvalidTimestamps { .. })));
    let mut f = closed_gops(2);
    f[2].presentation_index = f[3].presentation_index;
    assert!(matches!(plan(f, &request(0, 7, CutMode::FrameAccurate)), Err(PlanRefusal::InvalidTimestamps { .. })));
    let mut f = closed_gops(2);
    f[5].pts = Some(i64::MAX);
    assert!(matches!(plan(f, &request(0, 7, CutMode::FrameAccurate)), Err(PlanRefusal::InvalidTimestamps { .. })));
}

#[test]
fn nothing_is_executable_before_the_executor_exists() {
    // the built-in registry: packet copy and boundary re-encode are not implemented (WP-04, WP-06)
    let ix = index(stream(closed_gops(3)));
    let p = plan_smart_cut(&Registry::native(), &ix, &request(7, 14, CutMode::GopBoundary), STRICT).unwrap();
    assert!(!p.executable);
    assert!(p.blockers.iter().any(|b| b.contains("packet-copy")), "{:?}", p.blockers);
    let p = plan_smart_cut(&Registry::native(), &ix, &request(9, 21, CutMode::FrameAccurate), STRICT).unwrap();
    assert!(!p.executable && p.plan.requires_confirmation());
}

#[test]
fn the_copy_guarantee_decides_whether_a_framing_change_is_allowed() {
    // H.264 from MP4 (length-prefixed) into MPEG-TS (Annex B)
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
    let ix = index(stream(closed_gops(2)));
    let req = SmartCutRequest { output_container: Container::MpegTs, ..request(0, 7, CutMode::GopBoundary) };
    let p = plan_smart_cut(&r, &ix, &req, STRICT).unwrap();
    assert!(!p.executable);
    assert!(p.blockers.iter().any(|b| b.starts_with("bit-exact copy") && b.contains("framing")), "{:?}", p.blockers);
    assert_eq!(p.plan.segments[0].mode, SegmentMode::StrictCopy, "never switched to the compatible guarantee");
    let req = SmartCutRequest { guarantee: CopyGuarantee::Compatible, ..req };
    let p = plan_smart_cut(&r, &ix, &req, STRICT).unwrap();
    assert_eq!(p.plan.segments[0].mode, SegmentMode::CompatibleCopy);
    assert!(p.blockers.iter().any(|b| b.starts_with("compatible copy")), "{:?}", p.blockers);
    // same framing: no blocker from the guarantee
    let p = plan(closed_gops(2), &request(0, 7, CutMode::GopBoundary)).unwrap();
    assert!(p.executable, "{:?}", p.blockers);
}

#[test]
fn a_boundary_reencode_needs_a_known_stream_format_that_the_encoder_fits() {
    let req = SmartCutRequest { format: None, ..request(9, 21, CutMode::FrameAccurate) };
    let p = plan(closed_gops(3), &req).unwrap();
    assert!(!p.executable);
    assert!(p.blockers.iter().any(|b| b.contains("stream format")), "{:?}", p.blockers);
    let req = SmartCutRequest { format: Some(StreamFormat { profile: "high 4:2:2".into(), chroma: "4:2:2".into(), bit_depth: 10 }), ..req };
    let p = plan(closed_gops(3), &req).unwrap();
    assert!(!p.executable, "the encoder does not take 4:2:2 10-bit");
    // a lossless plan needs no format
    let req = SmartCutRequest { format: None, ..request(7, 14, CutMode::GopBoundary) };
    assert!(plan(closed_gops(3), &req).unwrap().executable);
}

#[test]
fn plans_are_deterministic_and_round_trip_as_json() {
    let req = request(3, 26, CutMode::FrameAccurate);
    let a = plan(closed_gops(4), &req).unwrap();
    let b = plan(closed_gops(4), &req).unwrap();
    assert_eq!(a, b);
    let json = serde_json::to_string(&a).unwrap();
    assert_eq!(serde_json::from_str::<SmartCutPlan>(&json).unwrap(), a);
    let req_json = serde_json::to_string(&req).unwrap();
    assert_eq!(serde_json::from_str::<SmartCutRequest>(&req_json).unwrap(), req);
    let err = plan(closed_gops(3), &request(9, 17, CutMode::GopBoundary)).unwrap_err();
    assert_eq!(serde_json::from_str::<PlanRefusal>(&serde_json::to_string(&err).unwrap()).unwrap(), err);
    assert!(!err.to_string().is_empty());
}

#[test]
fn random_indexes_and_requests_never_panic_and_plans_cover_the_range() {
    let mut seed = 0x1234_5678_9ABC_DEF1u64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let kinds = [
        RandomAccessKind::Idr,
        RandomAccessKind::None,
        RandomAccessKind::None,
        RandomAccessKind::None,
        RandomAccessKind::None,
        RandomAccessKind::IntraOnly,
        RandomAccessKind::OpenGop { broken_link: false },
    ];
    let rare = [DependencyStatus::Independent, DependencyStatus::CrossesAccessPoint, DependencyStatus::Undecodable, DependencyStatus::Unknown];
    let mut planned = 0;
    for _ in 0..2000 {
        let n = 1 + (rnd() % 40) as usize;
        // a random presentation order: decode order with nearby pictures swapped
        let mut order: Vec<u64> = (0..n as u64).collect();
        for i in 1..n {
            if rnd() % 3 == 0 {
                order.swap(i - 1, i);
            }
        }
        let frames: Vec<FrameIndexEntry> = (0..n)
            .map(|d| {
                let kind = if d == 0 { RandomAccessKind::Idr } else { kinds[(rnd() % 7) as usize] };
                let dep = match kind {
                    RandomAccessKind::Idr => DependencyStatus::Independent,
                    _ if rnd() % 8 == 0 => rare[(rnd() % 4) as usize],
                    _ => DependencyStatus::WithinGop,
                };
                let mut f = frame(d as u64, order[d], PictureType::P, kind, dep);
                f.config_id = u32::from(rnd() % 40 == 0);
                if rnd() % 300 == 0 {
                    f.pts = None;
                }
                f
            })
            .collect();
        let a = (rnd() % (n as u64 + 2)) as i64 - 1;
        let b = a + (rnd() % 25) as i64 - 2;
        let mode = if rnd() % 2 == 0 { CutMode::FrameAccurate } else { CutMode::GopBoundary };
        let req = request(a, b, mode);
        let Ok(p) = plan(frames.clone(), &req) else { continue };
        planned += 1;
        // the segments cover the requested pictures exactly, without gaps
        let first = p.plan.segments.first().unwrap().presentation.first;
        let last = p.plan.segments.last().unwrap().presentation.last;
        assert_eq!((first as i64, last as i64), (a.max(0), b - 1), "{req:?} {:?}", p.plan.segments);
        assert!(p.plan.segments.windows(2).all(|w| w[0].presentation.last + 1 == w[1].presentation.first));
        for s in &p.plan.segments {
            let start = &frames[s.decode.first as usize];
            assert!(start.random_access.is_clean(), "every segment is read from a clean access point: {s:?}");
            if s.mode.is_copy() {
                // the copied packets output exactly the segment's pictures
                let mut shown: Vec<u64> = frames[s.decode.first as usize..=s.decode.last as usize].iter().map(|f| f.presentation_index.unwrap()).collect();
                shown.sort_unstable();
                assert_eq!(shown, (s.presentation.first..=s.presentation.last).collect::<Vec<_>>());
            }
        }
        if mode == CutMode::GopBoundary {
            assert!(p.plan.is_lossless());
        }
    }
    assert!(planned >= 150, "only {planned} random requests gave a plan");
}

#[test]
fn an_encoded_h264_stream_is_planned_from_its_index() {
    use crate::h264_index::{NalFormat, index_h264};
    use crate::media_index::Packet;
    // IDR every 10 pictures, two B pictures between anchors, 25 fps on a 1/25 timebase
    let packets = crate::index_tests::encode_h264(30, 10, 2);
    let s = index_h264(
        1,
        Rational::new(1, 25),
        NalFormat::AnnexB,
        &[],
        packets.iter().map(|p| Packet { data: &p.data, pts: Some(p.pts), dts: Some(p.dts), keyframe: p.keyframe }),
    )
    .unwrap();
    let ix = index(s);
    let at = |frame: i64| Tick(frame * cfa4_time::TICKS_PER_SECOND / 25);
    let req = |a: i64, b: i64, mode: CutMode| SmartCutRequest { in_point: at(a), out_point: at(b), ..request(0, 0, mode) };
    let p = plan_smart_cut(&ready_registry(), &ix, &req(10, 20, CutMode::GopBoundary), STRICT).unwrap();
    assert_eq!(modes(&p), [(SegmentMode::StrictCopy, SegmentReason::WholeGops, 10, 19)]);
    assert!(p.executable, "{:?}", p.blockers);
    let p = plan_smart_cut(&ready_registry(), &ix, &req(5, 25, CutMode::FrameAccurate), STRICT).unwrap();
    assert_eq!(
        modes(&p),
        [
            (SegmentMode::BoundaryReencode, SegmentReason::HeadBoundary, 5, 9),
            // the copy runs on into the next GOP up to where decode and presentation order meet
            (SegmentMode::StrictCopy, SegmentReason::WholeGops, 10, 23),
            (SegmentMode::BoundaryReencode, SegmentReason::TailBoundary, 24, 24),
        ]
    );
    assert_eq!(p.plan.segments[0].decode.first, 0);
    assert_eq!(p.plan.segments[2].decode.first, 20);
    assert!(matches!(plan_smart_cut(&ready_registry(), &ix, &req(5, 25, CutMode::GopBoundary), STRICT), Err(PlanRefusal::NotAtSafePoints { .. })));
}
