//! Smart Cut of a source file (CFA4-SMART-CUT-RENDER-R1, WP-04; CFA4-SCE-001 v1.5 §11): the
//! `export.analyze`, `export.plan` and `export.start` commands with source scope (the timeline
//! scope, Smart Render of a sequence, is [`crate::smart_render`]).
//!
//! - `export.analyze` opens an MP4 / MOV source, indexes its H.264 or HEVC video track (WP-02,
//!   WP-07) and reports
//!   the access points, GOPs, configuration changes and whether the registry allows a Smart Cut.
//! - `export.plan` returns the plan for one source range (WP-03): copied segments, re-encoded
//!   boundaries, and, in GOP-boundary mode, the nearest safe points when the range is not on them.
//!   A plan that re-encodes boundaries also reports whether the boundary re-encoder can run it
//!   (`boundaryReencode`), with the reason when it cannot (an interlaced, 10-bit, variable-frame-
//!   rate or captioned source…).
//! - `export.start` runs the plan and writes the result through the host's atomic file writer only
//!   after the output was read back and checked, so a failed or refused export never leaves a
//!   partial file. A lossless plan copies the samples unchanged (WP-04); a frame-accurate plan
//!   copies the whole GOPs and re-encodes the boundary pictures (WP-06), and runs only with
//!   `"confirm": true`: nothing is re-encoded silently.
//!
//! - An MPEG program or transport stream source (`.mpg`, `.ts`, `.m2ts`: MPEG-2 video) is cut by
//!   [`crate::smart_cut_mpeg`] (WP-09c): GOP copies only, into an `.mpg`, `.ts` or `.m2ts` output.
//!
//! Times are ticks or time objects (`{"seconds": n}`, `{"frame": n}`, `{"timecode": "…"}`) on the
//! source stream's PTS timeline (an MPEG source: from its first picture); frames and timecodes
//! count at the stream's frame rate.

use std::io::Cursor;

use cfa4_export::audio_copy::{AudioPlan, CutInterval, mux_with_audio, plan_audio_copy, verify_audio_copy};
use cfa4_export::boundary_reencode::{
    BoundaryEncoding, check_mp4_boundary_reencode, execute_mp4_boundary_reencode, h264_stream_format, hevc_stream_format, verify_mp4_boundary_reencode,
};
use cfa4_export::smart_cut::{Mp4CopySource, SmartCutError, av1_stream_format, execute_mp4_copy, verify_mp4_copy};
use cfa4_isobmff::{Brand, ByteSource, CodecConfig, Mp4File, TrackKind};
use cfa4_project::{ItemKind, MediaRef};
use cfa4_smart_edit::av1_index::index_av1;
use cfa4_smart_edit::capabilities::{Container, Policy, Registry, StreamFormat};
use cfa4_smart_edit::copy_planner::{CutMode, SmartCutPlan, SmartCutRequest, plan_smart_cut};
use cfa4_smart_edit::h264_index::{NalFormat, index_h264};
use cfa4_smart_edit::hevc_index::index_hevc;
use cfa4_smart_edit::media_index::{MediaIndex, Packet, SourceFingerprint, StreamIndex};
use cfa4_smart_edit::plan::CopyGuarantee;
use cfa4_smart_edit::timestamp::{MediaTime, Rational};
use cfa4_time::{FrameRate, Tick};
use serde_json::{Value, json};

use crate::clip_ops::{SourceView, source_view};
use crate::commands::{CommandSpec, always, bad, bool_p, item_p, str_p, tick_or_time_p};
use crate::{EngineError, Result, Session};

type Run = fn(&mut Session, &Value) -> Result<Value>;

/// Exports use the strict policy: experimental capabilities never count (developer setting off).
pub(crate) const POLICY: Policy = Policy { allow_experimental: false };

/// The host's random-access reader as a container byte source.
struct Reader(cfa4_media::SharedReader);

impl ByteSource for Reader {
    fn len(&self) -> u64 {
        self.0.len()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
        self.0.read_at(offset, buf)
    }
}

/// An opened source: its bytes, container, video track and index.
pub(crate) struct Source {
    pub(crate) path: String,
    pub(crate) bytes: Box<dyn ByteSource>,
    pub(crate) file: Mp4File,
    pub(crate) container: Container,
    pub(crate) track: usize,
    pub(crate) index: MediaIndex,
    /// Profile, chroma format and bit depth, for the boundary re-encode decision.
    pub(crate) format: Option<StreamFormat>,
}

impl Source {
    pub(crate) fn stream(&self) -> Option<&StreamIndex> {
        self.index.streams.first()
    }
    /// How far the stream's PTS timeline lies behind media time (ticks), when the media layer maps
    /// the two without guessing: through the edit list, or without one when the first picture is
    /// at zero.
    fn media_offset(&self) -> Option<Tick> {
        let track = self.file.tracks.get(self.track)?;
        let first = track.samples.iter().map(|s| s.pts).min()?;
        if track.edits.is_empty() && first.checked_add(track.edit_offset)? != 0 {
            return None;
        }
        MediaTime::new(track.edit_offset, Rational::new(1, i64::from(track.timescale))).to_tick()
    }

    /// Where media time `t` lies on the stream's PTS timeline (ticks); see [`Source::media_offset`].
    pub(crate) fn stream_time(&self, t: Tick) -> Option<Tick> {
        t.0.checked_sub(self.media_offset()?.0).map(Tick)
    }

    /// The media time of stream time `t` (the inverse of [`Source::stream_time`]).
    pub(crate) fn media_time(&self, t: Tick) -> Option<Tick> {
        t.0.checked_add(self.media_offset()?.0).map(Tick)
    }

    /// The stream's frame rate (timescale / first sample duration), for frame and timecode times.
    fn rate(&self) -> FrameRate {
        let t = self.file.tracks.get(self.track);
        let duration = t.and_then(|t| t.samples.first()).map(|s| i64::from(s.duration)).filter(|&d| d > 0);
        match (t, duration) {
            (Some(t), Some(d)) => FrameRate { num: i64::from(t.timescale), den: d },
            _ => FrameRate::FPS_25,
        }
    }
}

/// The source named in `p`: a file (`"path"`), or a project media item (`"item"`, the clip of the
/// Source monitor; a subclip names its media). With an item, times are media times (the Source
/// monitor's), its marked range is the default cut, and the item's view comes back.
pub(crate) fn source_p(s: &Session, p: &Value, cmd: &str) -> Result<(String, Option<SourceView>)> {
    if let Some(path) = str_p(p, "path").filter(|s| !s.is_empty()) {
        return Ok((path.to_string(), None));
    }
    let Some(item) = item_p(p, "item") else {
        return Err(bad(cmd, "`path` (the source file) or `item` (a media item of the project) is required"));
    };
    let view = source_view(s, item).ok_or_else(|| bad(cmd, format!("no project item {}", item.0)))?;
    let file = match s.project.item(view.media).map(|i| &i.kind) {
        Some(ItemKind::Media(m)) => match &m.media {
            MediaRef::File { path } if !path.is_empty() => Some(path.clone()),
            _ => None,
        },
        _ => None,
    };
    let path = file.ok_or_else(|| bad(cmd, "Smart Cut needs a clip of a media file (not a sequence, generator or still)"))?;
    Ok((path, Some(view)))
}

fn container_of(path: &str, quicktime: bool) -> Container {
    if quicktime || path.to_ascii_lowercase().ends_with(".mov") { Container::Mov } else { Container::Mp4 }
}

/// Open `path`, find its first video track (H.264, HEVC or AV1) and index it.
pub(crate) fn open_source(s: &Session, path: &str, cmd: &str) -> Result<Source> {
    let fail = |e: String| EngineError::Other(format!("{path}: {e}"));
    let bytes: Box<dyn ByteSource> = match s.services.reader(path) {
        Some(r) => Box::new(Reader(r.map_err(|e| fail(e.to_string()))?)),
        None => Box::new(s.services.read_file(path).map_err(|e| fail(e.to_string()))?),
    };
    let file = cfa4_isobmff::open(&*bytes).map_err(|e| fail(format!("not an MP4 / MOV file Smart Cut can read ({e})")))?;
    let track = file.tracks.iter().position(|t| t.kind == TrackKind::Video).ok_or_else(|| bad(cmd, format!("{path} has no video track")))?;
    let t = file.tracks.get(track).ok_or_else(|| bad(cmd, format!("{path} has no video track")))?;
    let config = t.codec();
    if !matches!(config, Some(CodecConfig::Avc(_) | CodecConfig::Hevc(_) | CodecConfig::Av1(_))) {
        return Err(bad(cmd, format!("{path}: Smart Cut copies H.264, HEVC and AV1 video only in this release")));
    }
    let mut data = Vec::with_capacity(t.samples.len());
    for i in 0..t.samples.len() {
        data.push(file.read_sample(&*bytes, track, i).map_err(|e| fail(format!("sample {i}: {e}")))?);
    }
    let packets = t.samples.iter().zip(&data).map(|(x, d)| Packet { data: d, pts: Some(x.pts), dts: Some(x.dts), keyframe: x.is_sync });
    let tb = Rational::new(1, i64::from(t.timescale));
    let (format, stream) = match config {
        Some(CodecConfig::Avc(avc)) => {
            let ps: Vec<&[u8]> = avc.sps.iter().chain(&avc.pps).map(Vec::as_slice).collect();
            (h264_stream_format(avc), index_h264(t.id, tb, NalFormat::LengthPrefixed(avc.length_size), &ps, packets))
        }
        Some(CodecConfig::Hevc(hvcc)) => {
            let ps: Vec<&[u8]> = hvcc.arrays.iter().flat_map(|a| a.nalus.iter().map(Vec::as_slice)).collect();
            (hevc_stream_format(hvcc), index_hevc(t.id, tb, NalFormat::LengthPrefixed(hvcc.length_size), &ps, packets))
        }
        Some(CodecConfig::Av1(av1c)) => (Some(av1_stream_format(av1c)), index_av1(t.id, tb, &av1c.config_obus, packets)),
        _ => return Err(bad(cmd, format!("{path}: Smart Cut copies H.264, HEVC and AV1 video only in this release"))),
    };
    let stream = stream.map_err(|e| fail(format!("indexing failed: {e:?}")))?;
    let fingerprint = SourceFingerprint { size: bytes.len(), modified_ns: None, content_id: None };
    let container = container_of(path, file.is_quicktime);
    Ok(Source { path: path.to_string(), bytes, file, container, track, index: MediaIndex::new(path, fingerprint, vec![stream]), format })
}

pub(crate) fn ticks(t: Option<i64>, tb: Rational) -> Value {
    t.and_then(|v| MediaTime::new(v, tb).to_tick()).map(|t| json!(t.0)).unwrap_or(Value::Null)
}

fn analyze(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "export.analyze";
    let (path, view) = source_p(s, p, cmd)?;
    if crate::smart_cut_mpeg::is_mpeg(s, &path) {
        return crate::smart_cut_mpeg::analyze(s, p, &path, view.as_ref());
    }
    let src = open_source(s, &path, cmd)?;
    let stream = src.stream().ok_or_else(|| EngineError::Other(format!("{path}: no stream was indexed")))?;
    let tb = stream.timebase;
    let points: Vec<Value> = stream
        .frames
        .iter()
        .filter(|f| f.random_access.is_access_point())
        .map(|f| {
            let mut point =
                json!({"presentation": f.presentation_index, "time": ticks(f.pts, tb), "clean": f.random_access.is_clean(), "kind": f.random_access});
            if view.is_some() {
                // the Source monitor's time of the access point
                let media = f.pts.and_then(|v| MediaTime::new(v, tb).to_tick()).and_then(|t| src.media_time(t));
                point["mediaTime"] = media.map(|t| json!(t.0)).unwrap_or(Value::Null);
            }
            point
        })
        .collect();
    let registry = Registry::native();
    let cut = registry.smart_cut(stream.codec, src.container, src.container, POLICY);
    let rate = src.rate();
    Ok(json!({
        "path": path,
        "item": view.as_ref().map(|v| v.item.0),
        "container": src.container,
        "codec": stream.codec,
        "frames": stream.frames.len(),
        "frameRate": {"num": rate.num, "den": rate.den},
        "duration": stream.duration().and_then(|d| d.to_tick()).map(|t| t.0),
        "gops": stream.gops.len(),
        "accessPoints": points,
        "configChanges": stream.config_changes,
        "timestampProblems": stream.timestamp_problems,
        "smartCut": {"allowed": cut.allowed, "blockers": cut.blockers()},
    }))
}

/// The copy guarantee named in `p` (`"guarantee"`, default bit-exact).
pub(crate) fn guarantee_p(p: &Value, cmd: &str) -> Result<CopyGuarantee> {
    match str_p(p, "guarantee").unwrap_or("bit-exact") {
        "bit-exact" => Ok(CopyGuarantee::BitExact),
        "compatible" => Ok(CopyGuarantee::Compatible),
        g => Err(bad(cmd, format!("unknown `guarantee` {g:?}: bit-exact or compatible"))),
    }
}

/// Whether `p` names the timeline scope (Smart Render of a sequence) rather than one source file.
pub(crate) fn timeline_scope(p: &Value, cmd: &str) -> Result<bool> {
    match str_p(p, "scope").unwrap_or("source") {
        "source" => Ok(false),
        "timeline" => Ok(true),
        x => Err(bad(cmd, format!("unknown `scope` {x:?}: source or timeline"))),
    }
}

/// How a source maps the Source monitor's media times onto its stream timeline.
pub(crate) struct CutSource<'a> {
    pub(crate) path: &'a str,
    pub(crate) view: Option<&'a SourceView>,
    pub(crate) rate: FrameRate,
    pub(crate) stream_time: &'a dyn Fn(Tick) -> Option<Tick>,
}

/// The cut mode named in `p` (`"mode"`, default GOP boundary).
pub(crate) fn mode_p(p: &Value, cmd: &str) -> Result<CutMode> {
    match str_p(p, "mode").unwrap_or("gop-boundary") {
        "gop-boundary" => Ok(CutMode::GopBoundary),
        "frame-accurate" => Ok(CutMode::FrameAccurate),
        m => Err(bad(cmd, format!("unknown `mode` {m:?}: gop-boundary or frame-accurate"))),
    }
}

/// The range named in `p` on the stream's PTS timeline: `in` / `out` stream times, or with an item
/// media times of the Source monitor (default: its marked range).
pub(crate) fn range_p(p: &Value, cmd: &str, src: &CutSource<'_>) -> Result<(Tick, Tick)> {
    let in_point = tick_or_time_p(p, "in", src.rate, false, cmd)?;
    let out_point = tick_or_time_p(p, "out", src.rate, false, cmd)?;
    match src.view {
        None => Ok((in_point.ok_or_else(|| bad(cmd, "`in` is required"))?, out_point.ok_or_else(|| bad(cmd, "`out` is required (exclusive)"))?)),
        Some(view) => {
            let marked = view.selected_range();
            let unmapped = || bad(cmd, format!("{}: its media time does not map to the stream timeline without guessing (edit list)", src.path));
            let in_point = (src.stream_time)(in_point.unwrap_or(marked.start)).ok_or_else(unmapped)?;
            let out_point = (src.stream_time)(out_point.unwrap_or(marked.end())).ok_or_else(unmapped)?;
            Ok((in_point, out_point))
        }
    }
}

/// The plan of the range named in `p`.
fn plan_of(s: &Session, p: &Value, cmd: &str) -> Result<(Source, SmartCutPlan)> {
    let (path, view) = source_p(s, p, cmd)?;
    let src = open_source(s, &path, cmd)?;
    let map = |t: Tick| src.stream_time(t);
    let (in_point, out_point) = range_p(p, cmd, &CutSource { path: &path, view: view.as_ref(), rate: src.rate(), stream_time: &map })?;
    let mode = mode_p(p, cmd)?;
    let guarantee = guarantee_p(p, cmd)?;
    let output_container = match str_p(p, "output") {
        Some(o) if o.to_ascii_lowercase().ends_with(".mov") => Container::Mov,
        Some(o) if o.to_ascii_lowercase().ends_with(".mp4") || o.to_ascii_lowercase().ends_with(".m4v") => Container::Mp4,
        Some(o) => return Err(bad(cmd, format!("{o}: the output must be an .mp4, .m4v or .mov file"))),
        None => src.container,
    };
    let stream_id = src.stream().map(|x| x.stream_id).unwrap_or_default();
    let request =
        SmartCutRequest { stream_id, in_point, out_point, mode, guarantee, source_container: src.container, output_container, format: src.format.clone() };
    let plan = plan_smart_cut(&Registry::native(), &src.index, &request, POLICY).map_err(|e| bad(cmd, e.to_string()))?;
    Ok((src, plan))
}

pub(crate) fn summary(p: &SmartCutPlan) -> Value {
    json!({
        "lossless": p.plan.is_lossless(),
        "requiresConfirmation": p.plan.requires_confirmation(),
        "copiedFrames": p.plan.copied_frames(),
        "reencodedFrames": p.plan.reencoded_frames(),
    })
}

/// The boundary encoding named in `p` (`"crf"`, default 16).
pub(crate) fn encoding_p(p: &Value, cmd: &str) -> Result<BoundaryEncoding> {
    let mut encoding = BoundaryEncoding::default();
    match p.get("crf") {
        None | Some(Value::Null) => {}
        Some(v) => match v.as_f64().filter(|c| (0.0..=51.0).contains(c)) {
            Some(crf) => encoding.crf = crf as f32,
            None => return Err(bad(cmd, "`crf` must be a number from 0 to 51")),
        },
    }
    Ok(encoding)
}

/// The source as the Smart Cut executors read it.
fn copy_source(src: &Source) -> Mp4CopySource<'_, dyn ByteSource> {
    Mp4CopySource { file: &src.file, bytes: &*src.bytes, track: src.track }
}

/// The audio tracks a cut copies with its video (WP-09b): all of them, unless `"audio": false`.
fn audio_of(src: &Source, plan: &SmartCutPlan, p: &Value) -> Option<(CutInterval, AudioPlan)> {
    if !bool_p(p, "audio").unwrap_or(true) {
        return None;
    }
    let track = src.file.tracks.get(src.track)?;
    let first = plan.plan.segments.first()?;
    let duration = u64::try_from(plan.plan.expected_duration.value).ok()?;
    let interval = CutInterval::of_video(track, first.source_in.value, duration);
    Some((interval, plan_audio_copy(&src.file, interval)))
}

fn audio_json(audio: &Option<(CutInterval, AudioPlan)>) -> Value {
    match audio {
        Some((_, a)) => json!({
            "tracks": a.tracks.iter().map(|t| json!({"trackId": t.track_id, "codec": t.codec})).collect::<Vec<_>>(),
            "dropped": a.dropped,
        }),
        None => json!({"tracks": [], "dropped": []}),
    }
}

fn plan(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "export.plan";
    if timeline_scope(p, cmd)? {
        return crate::smart_render::plan(s, p);
    }
    let (path, _) = source_p(s, p, cmd)?;
    if crate::smart_cut_mpeg::is_mpeg(s, &path) {
        return crate::smart_cut_mpeg::plan(s, p);
    }
    let encoding = encoding_p(p, cmd)?;
    let (src, plan) = plan_of(s, p, cmd)?;
    let summary = summary(&plan);
    let audio = audio_json(&audio_of(&src, &plan, p));
    if !plan.plan.requires_confirmation() {
        return Ok(json!({"plan": plan, "summary": summary, "audio": audio}));
    }
    // the boundary re-encoder's refusals, before anything is decoded or written
    let stream = src.stream().ok_or_else(|| EngineError::Other(format!("{}: no stream was indexed", src.path)))?;
    let problem = check_mp4_boundary_reencode(&plan.plan, stream, &copy_source(&src), &encoding).err().map(|e| e.to_string());
    Ok(json!({"plan": plan, "summary": summary, "audio": audio, "boundaryReencode": {"ready": problem.is_none(), "problem": problem}}))
}

fn start(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "export.start";
    if timeline_scope(p, cmd)? {
        return crate::smart_render::start(s, p);
    }
    let (path, _) = source_p(s, p, cmd)?;
    if crate::smart_cut_mpeg::is_mpeg(s, &path) {
        return crate::smart_cut_mpeg::start(s, p);
    }
    let output = str_p(p, "output").filter(|o| !o.is_empty()).map(str::to_string).ok_or_else(|| bad(cmd, "`output` (the file to write) is required"))?;
    let encoding = encoding_p(p, cmd)?;
    let (src, plan) = plan_of(s, p, cmd)?;
    if output == src.path {
        return Err(bad(cmd, "the output must not be the source file"));
    }
    let reencodes = plan.plan.requires_confirmation();
    if reencodes && !bool_p(p, "confirm").unwrap_or(false) {
        return Err(bad(
            cmd,
            format!(
                "the plan re-encodes {} frames at its boundaries; run it with \"confirm\": true, or use \"mode\": \"gop-boundary\" to copy without re-encoding",
                plan.plan.reencoded_frames()
            ),
        ));
    }
    if !plan.executable {
        return Err(EngineError::Other(format!("Smart Cut cannot run: {}", plan.blockers.join("; "))));
    }
    let stream = src.stream().ok_or_else(|| EngineError::Other(format!("{}: no stream was indexed", src.path)))?;
    let source = copy_source(&src);
    let other = |e: SmartCutError| EngineError::Other(e.to_string());
    let (bytes, report) = if reencodes {
        let (out, report) = execute_mp4_boundary_reencode(&plan.plan, stream, &source, &encoding, Cursor::new(Vec::new())).map_err(other)?;
        let bytes = out.into_inner();
        let verified = verify_mp4_boundary_reencode(&plan.plan, stream, &source, &encoding, bytes.as_slice()).map_err(other)?;
        if (verified.samples, verified.copied, verified.reencoded) != (report.samples, report.copied, report.reencoded) {
            return Err(EngineError::Other("Smart Cut verification read back another file than was written".into()));
        }
        // the verified report carries the measured quality of the re-encoded pictures
        (bytes, serde_json::to_value(&verified).map_err(|e| EngineError::Other(e.to_string()))?)
    } else {
        let (out, report) = execute_mp4_copy(&plan.plan, stream, &source, Cursor::new(Vec::new())).map_err(other)?;
        let bytes = out.into_inner();
        let verified = verify_mp4_copy(&plan.plan, stream, &source, bytes.as_slice()).map_err(other)?;
        if verified != report {
            return Err(EngineError::Other("Smart Cut verification read back another file than was written".into()));
        }
        (bytes, serde_json::to_value(&report).map_err(|e| EngineError::Other(e.to_string()))?)
    };
    // the audio tracks, copied next to the verified video and verified in their turn
    let audio = audio_of(&src, &plan, p);
    let mut audio_reports = Vec::new();
    let bytes = match &audio {
        Some((interval, a)) if !a.tracks.is_empty() => {
            let brand = if plan.plan.output_container == Container::Mov { Brand::Mov } else { Brand::Mp4 };
            let with_audio = mux_with_audio(&bytes, &src.file, &*src.bytes, a, *interval, brand).map_err(other)?;
            audio_reports = verify_audio_copy(&with_audio, &bytes, &src.file, &*src.bytes, a, *interval).map_err(other)?;
            with_audio
        }
        _ => bytes,
    };
    s.services.write_file(&output, &bytes).map_err(|e| EngineError::Other(format!("{output}: {e}")))?;
    let summary = summary(&plan);
    let mut audio = audio_json(&audio);
    audio["verified"] = json!(audio_reports);
    Ok(json!({"output": output, "bytes": bytes.len(), "report": report, "verified": true, "summary": summary, "audio": audio}))
}

fn spec(id: &'static str, label: &'static str, params: &'static str, run: Run, journal: bool) -> CommandSpec {
    CommandSpec { id, label, menu: &[], shortcut: None, params, enabled: always, run, journal }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        spec(
            "export.analyze",
            "Analyze Source for Smart Cut",
            r#"{"path":str}|{"item":id} → {container, codec, frames, frameRate, duration, gops, accessPoints:[{presentation,time,clean,kind,mediaTime?}], configChanges, smartCut:{allowed,blockers}}"#,
            analyze,
            false,
        ),
        spec(
            "export.plan",
            "Plan Smart Cut",
            r#"{"path":str,"in":time,"out":time (exclusive)}|{"item":id,"in":media time?,"out":media time? (default: the marked range)},"mode":"gop-boundary"|"frame-accurate"="gop-boundary","guarantee":"bit-exact"|"compatible"="bit-exact","output":str?,"scope":"source"|"timeline"="source","crf":number=16,"audio":bool=true}; timeline: {"scope":"timeline","sequence":id?,"output":str?,"guarantee":…,"crf":…} → {plan, summary, audio?:{tracks:[{trackId,codec}],dropped:[{trackId,reason}]}, boundaryReencode?:{ready,problem}, timelineRender?:{ready,problem}}"#,
            plan,
            false,
        ),
        spec(
            "export.start",
            "Run Smart Cut",
            r#"{"path":str,"in":time,"out":time}|{"item":id,"in":media time?,"out":media time?},"output":str,"mode":…,"guarantee":…,"confirm":bool (required when the plan re-encodes),"crf":number=16,"audio":bool=true}; timeline: {"scope":"timeline","sequence":id?,"output":str,"confirm":bool,…} → {output, bytes, report, verified, audio?:{tracks,dropped,verified:[{trackId,codec,samples,payloadBytes,syncErrorNs}]}}"#,
            start,
            true,
        ),
        spec(
            "export.gopMarks",
            "GOP Marks of the Sequence",
            r#"{"sequence":id?} → {sequence, clips:[{clip, track, codec, in (starts on a clean access point), points:[{time (sequence ticks), clean}]}]}"#,
            crate::smart_render::gop_marks,
            false,
        ),
    ]
}
