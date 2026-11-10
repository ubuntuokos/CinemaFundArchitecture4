//! WP-06: the timeline Smart Render executor copies the whole GOPs of several sources into one
//! H.264 stream, re-encodes the boundaries and the incompatible runs, and encodes the rendered
//! segments (here: a gap as black pictures).

use std::collections::BTreeMap;
use std::io::Cursor;
use std::process::Command;

use cfa4_isobmff::{Brand, ByteSource, Mp4File};
use cfa4_smart_edit::capabilities::{Codec, Container, Policy, Registry, StreamFormat};
use cfa4_smart_edit::edit_decision::{EditClip, EditContent, EditDecisionList, EditItem};
use cfa4_smart_edit::media_index::{MediaIndex, SourceFingerprint, StreamIndex};
use cfa4_smart_edit::plan::{CopyGuarantee, ExportExecutionPlan, PlanSegment, SegmentMode};
use cfa4_smart_edit::smart_render::{SmartRenderRequest, plan_smart_render};
use cfa4_smart_edit::timestamp::Rational;

use crate::boundary_reencode::BoundaryEncoding;
use crate::smart_cut::{Mp4CopySource, SmartCutError};
use crate::smart_cut_tests::{index_of, source_mp4_seeded, tick};
use crate::timeline_render::{
    RenderedFrames, RenderedPicture, TimelineOutput, TimelineReport, TimelineSource, check_mp4_timeline, execute_mp4_timeline, verify_mp4_timeline,
};

const STRICT: Policy = Policy { allow_experimental: false };

struct Src {
    bytes: Vec<u8>,
    file: Mp4File,
    index: StreamIndex,
}

impl Src {
    fn new(seed: usize, bframes: u8) -> Src {
        let bytes = source_mp4_seeded(40, 10, bframes, Brand::Mp4, seed);
        let file = cfa4_isobmff::open(bytes.as_slice()).unwrap();
        let index = index_of(&file, &bytes);
        Src { bytes, file, index }
    }

    fn media(&self, id: &str) -> MediaIndex {
        MediaIndex::new(id, SourceFingerprint { size: self.bytes.len() as u64, modified_ns: None, content_id: None }, vec![self.index.clone()])
    }

    fn timeline(&self) -> TimelineSource<'_> {
        let bytes: &dyn ByteSource = &self.bytes;
        TimelineSource { index: &self.index, source: Mp4CopySource { file: &self.file, bytes, track: 0 } }
    }

    /// The first presentation time (frames).
    fn pts0(&self) -> i64 {
        self.file.tracks[0].samples.iter().map(|s| s.pts).min().unwrap()
    }

    fn packet(&self, d: usize) -> Vec<u8> {
        self.file.read_sample(self.bytes.as_slice(), 0, d).unwrap()
    }
}

/// Black 64×48 pictures; counts the calls.
#[derive(Default)]
struct Black {
    calls: u64,
    fail: bool,
    size: (u32, u32),
}

impl RenderedFrames for Black {
    fn picture(&mut self, segment: &PlanSegment, output_frame: u64) -> Result<RenderedPicture, String> {
        assert!((segment.presentation.first..=segment.presentation.last).contains(&output_frame));
        self.calls += 1;
        if self.fail {
            return Err("no renderer".into());
        }
        let (w, h) = if self.size == (0, 0) { (64, 48) } else { self.size };
        let (luma, chroma) = ((w * h) as usize, (w * h / 4) as usize);
        Ok(RenderedPicture { width: w, height: h, y: vec![16; luma], u: vec![128; chroma], v: vec![128; chroma] })
    }
}

fn output() -> TimelineOutput {
    TimelineOutput { width: 64, height: 48, frame_duration: Rational::new(1, 25), encoding: BoundaryEncoding { threads: 1, ..BoundaryEncoding::default() } }
}

fn clip(src: &Src, id: &str, at: i64, from: i64, len: i64, config: &str) -> EditItem {
    EditItem {
        output_in: tick(at),
        duration: tick(len),
        content: EditContent::Clip(EditClip {
            source_id: id.into(),
            stream_id: src.index.stream_id,
            container: Container::Mp4,
            source_in: tick(src.pts0() + from),
            configs: vec![config.into()],
            format: Some(StreamFormat { profile: "high".into(), chroma: "4:2:0".into(), bit_depth: 8 }),
        }),
        render_causes: Vec::new(),
    }
}

fn plan_of(indexes: &[MediaIndex], items: Vec<EditItem>) -> ExportExecutionPlan {
    let edl = EditDecisionList { timeline_id: "sequence-1".into(), frame_duration: Rational::new(1, 25), items };
    let request = SmartRenderRequest { codec: Codec::H264, output_container: Container::Mp4, guarantee: CopyGuarantee::BitExact, output_config: None };
    plan_smart_render(&Registry::native(), indexes, &edl, &request, STRICT).unwrap().plan
}

/// A 0–20, B 10–30, a 5-frame gap, A 5–17 (a head boundary), B 20–30: 67 frames.
fn timeline(a: &Src, b: &Src, b_config: &str) -> ExportExecutionPlan {
    plan_of(
        &[a.media("a.mp4"), b.media("b.mp4")],
        vec![
            clip(a, "a.mp4", 0, 0, 20, "avc"),
            clip(b, "b.mp4", 20, 10, 20, b_config),
            clip(a, "a.mp4", 45, 5, 12, "avc"),
            clip(b, "b.mp4", 57, 20, 10, b_config),
        ],
    )
}

fn sources<'a>(a: &'a Src, b: &'a Src) -> BTreeMap<String, TimelineSource<'a>> {
    BTreeMap::from([("a.mp4".to_string(), a.timeline()), ("b.mp4".to_string(), b.timeline())])
}

/// Run and verify the plan; the verified report agrees with the written one.
fn run(plan: &ExportExecutionPlan, sources: &BTreeMap<String, TimelineSource<'_>>) -> (Vec<u8>, TimelineReport) {
    check_mp4_timeline(plan, sources, &output()).unwrap();
    let mut black = Black::default();
    let (out, report) = execute_mp4_timeline(plan, sources, &mut black, &output(), Cursor::new(Vec::new())).unwrap();
    assert_eq!(black.calls, report.rendered);
    let out = out.into_inner();
    let verified = verify_mp4_timeline(plan, sources, &output(), out.as_slice()).unwrap();
    assert_eq!((verified.samples, verified.copied, verified.reencoded, verified.rendered), (report.samples, report.copied, report.reencoded, report.rendered));
    assert_eq!((verified.duration, verified.payload_bytes, verified.timescale), (report.duration, report.payload_bytes, report.timescale));
    (out, verified)
}

#[test]
fn two_sources_a_gap_and_boundaries_make_one_stream() {
    for bframes in [0, 2] {
        let (a, b) = (Src::new(0, bframes), Src::new(1, bframes));
        let plan = timeline(&a, &b, "avc");
        let modes: Vec<SegmentMode> = plan.segments.iter().map(|s| s.mode).collect();
        assert!(modes.contains(&SegmentMode::Render) && modes.contains(&SegmentMode::BoundaryReencode), "{modes:?}");
        let (out, report) = run(&plan, &sources(&a, &b));
        assert_eq!(report.samples, 67, "bframes {bframes}");
        assert_eq!((report.copied, report.reencoded + report.rendered), (plan.copied_frames(), plan.reencoded_frames()), "bframes {bframes}");
        assert_eq!(report.rendered, 5);
        assert!(report.reencoded > 0);
        if bframes == 0 {
            // without B-frames A 10–17 copies up to its last picture: only A 5–10 is re-encoded
            assert_eq!((report.copied, report.reencoded), (57, 5));
        }
        assert_eq!(report.duration, 67);
        assert!(report.min_psnr.is_some_and(|p| p >= 30.0));
        // the copied samples are the packets of their own source
        let back = cfa4_isobmff::open(out.as_slice()).unwrap();
        let sample = |k: usize| back.read_sample(out.as_slice(), 0, k).unwrap();
        assert_eq!(sample(0), a.packet(0));
        assert_eq!(sample(20), b.packet(10));
        assert_eq!(sample(57), b.packet(20));
        assert_ne!(a.packet(0), b.packet(0), "the two sources differ");
    }
}

#[test]
fn a_stream_with_another_configuration_is_re_encoded() {
    let (a, b) = (Src::new(0, 0), Src::new(1, 0));
    let plan = timeline(&a, &b, "other");
    let (_, report) = run(&plan, &sources(&a, &b));
    assert_eq!((report.copied, report.reencoded, report.rendered), (27, 35, 5));
}

#[test]
fn a_timeline_of_only_a_gap_is_rendered() {
    let a = Src::new(0, 0);
    let mut gap = clip(&a, "a.mp4", 0, 0, 8, "avc");
    gap.content = EditContent::Gap;
    let plan = plan_of(&[], vec![gap]);
    let (_, report) = run(&plan, &BTreeMap::new());
    assert_eq!((report.samples, report.rendered, report.timescale, report.duration), (8, 8, 25, 8));
    assert_eq!(report.min_psnr, None);
}

#[test]
fn incompatible_plans_are_refused_before_writing() {
    let (a, b) = (Src::new(0, 0), Src::new(1, 0));
    let plan = timeline(&a, &b, "avc");
    let all = sources(&a, &b);
    let refuse = |p: &ExportExecutionPlan, s: &BTreeMap<String, TimelineSource<'_>>, o: &TimelineOutput| {
        let checked = check_mp4_timeline(p, s, o).unwrap_err();
        let mut out = Cursor::new(Vec::new());
        let mut black = Black::default();
        let e = execute_mp4_timeline(p, s, &mut black, o, &mut out).unwrap_err();
        assert!(out.get_ref().is_empty() && black.calls == 0, "{e}: nothing is written or rendered");
        assert_eq!(checked, e);
        e
    };
    let only_a = BTreeMap::from([("a.mp4".to_string(), a.timeline())]);
    assert_eq!(refuse(&plan, &only_a, &output()), SmartCutError::MissingSource { source_id: "b.mp4".into() });
    let small = TimelineOutput { width: 32, ..output() };
    let e = refuse(&plan, &all, &small);
    assert!(e.to_string().contains("32×48"), "{e}");
    let mut swapped = plan.clone();
    swapped.segments.swap(0, 1);
    assert!(matches!(refuse(&swapped, &all, &output()), SmartCutError::NotContiguous { .. }));
    let mut longer = plan.clone();
    longer.expected_duration.value += 1;
    assert!(matches!(refuse(&longer, &all, &output()), SmartCutError::StaleIndex { .. }));
    let mut mkv = plan.clone();
    mkv.output_container = Container::Matroska;
    assert!(matches!(refuse(&mkv, &all, &output()), SmartCutError::Unsupported { .. }));
    let fps30 = TimelineOutput { frame_duration: Rational::new(1, 30), ..output() };
    assert!(check_mp4_timeline(&plan, &all, &fps30).is_err(), "another frame rate is refused");
}

#[test]
fn renderer_failures_stop_the_export() {
    let (a, b) = (Src::new(0, 0), Src::new(1, 0));
    let plan = timeline(&a, &b, "avc");
    let gap = plan.segments.iter().position(|s| s.mode == SegmentMode::Render).unwrap();
    for (mut black, wanted) in [(Black { fail: true, ..Black::default() }, "no renderer"), (Black { size: (32, 24), ..Black::default() }, "4:2:0")] {
        let e = execute_mp4_timeline(&plan, &sources(&a, &b), &mut black, &output(), Cursor::new(Vec::new())).unwrap_err();
        assert!(matches!(&e, SmartCutError::Render { segment, detail } if *segment == gap && detail.contains(wanted)), "{e}");
    }
}

#[test]
fn verification_catches_a_changed_or_poor_output() {
    let (a, b) = (Src::new(0, 0), Src::new(1, 0));
    let plan = timeline(&a, &b, "avc");
    let all = sources(&a, &b);
    let (out, _) = run(&plan, &all);
    let strict = TimelineOutput { encoding: BoundaryEncoding { min_psnr: 99.5, ..output().encoding }, ..output() };
    let e = verify_mp4_timeline(&plan, &all, &strict, out.as_slice()).unwrap_err();
    assert!(matches!(&e, SmartCutError::Verification { detail } if detail.contains("dB")), "{e}");
    // a copied sample of b.mp4 (output sample 20) changed
    let back = cfa4_isobmff::open(out.as_slice()).unwrap();
    let s = back.tracks[0].samples[20];
    let mut damaged = out.clone();
    damaged[(s.offset + u64::from(s.size) - 1) as usize] ^= 0x55;
    let e = verify_mp4_timeline(&plan, &all, &output(), damaged.as_slice()).unwrap_err();
    assert!(matches!(e, SmartCutError::Verification { .. }), "{e}");
    // the output of another plan
    let shorter = plan_of(&[a.media("a.mp4"), b.media("b.mp4")], vec![clip(&a, "a.mp4", 0, 0, 20, "avc"), clip(&b, "b.mp4", 20, 10, 20, "avc")]);
    let e = verify_mp4_timeline(&shorter, &all, &output(), out.as_slice()).unwrap_err();
    assert!(e.to_string().contains("planned"), "{e}");
}

/// FFmpeg as the oracle (SCE-001 §18): the output decodes without errors to 67 pictures, the
/// copied pictures are the source pictures and the gap is one black picture repeated.
#[test]
fn ffmpeg_decodes_the_timeline() {
    let Some(ffmpeg) = cfa4_testkit::ffmpeg_or_skip("timeline Smart Render decode") else { return };
    for bframes in [0, 2] {
        let (a, b) = (Src::new(0, bframes), Src::new(1, bframes));
        let (out, _) = run(&timeline(&a, &b, "avc"), &sources(&a, &b));
        let dir = std::env::temp_dir().join(format!("cfa4-timeline-{}-{bframes}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = [dir.join("a.mp4"), dir.join("b.mp4"), dir.join("timeline.mp4")];
        for (path, bytes) in paths.iter().zip([&a.bytes, &b.bytes, &out]) {
            std::fs::write(path, bytes).unwrap();
        }
        let hashes = |path: &std::path::Path| -> Vec<String> {
            let o = Command::new(&ffmpeg).args(["-v", "error", "-xerror", "-i"]).arg(path).args(["-map", "0:v", "-f", "framemd5", "-"]).output().unwrap();
            assert!(o.status.success() && o.stderr.is_empty(), "{}", String::from_utf8_lossy(&o.stderr));
            String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.starts_with('#')).map(|l| l.rsplit(',').next().unwrap().trim().to_string()).collect()
        };
        let [ha, hb, ht] = paths.each_ref().map(|p| hashes(p));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(ht.len(), 67, "bframes {bframes}");
        assert_eq!(ht[0..20], ha[0..20], "bframes {bframes}: a.mp4 copied");
        assert_eq!(ht[20..40], hb[10..30], "bframes {bframes}: b.mp4 copied");
        assert_eq!(ht[57..67], hb[20..30], "bframes {bframes}: b.mp4 copied after a boundary");
        assert!(ht[40..45].iter().all(|h| *h == ht[40]), "bframes {bframes}: the gap is one picture");
    }
}
