//! Smart Cut of a source file (CFA4-SMART-CUT-RENDER-R1, WP-04; CFA4-SCE-001 v1.5 §11): the
//! `export.analyze`, `export.plan` and `export.start` commands with source scope.
//!
//! - `export.analyze` opens an MP4 / MOV source, indexes its H.264 video track (WP-02) and reports
//!   the access points, GOPs, configuration changes and whether the registry allows a Smart Cut.
//! - `export.plan` returns the plan for one source range (WP-03): copied segments, re-encoded
//!   boundaries, and, in GOP-boundary mode, the nearest safe points when the range is not on them.
//! - `export.start` runs a lossless plan (WP-04): the samples are copied unchanged, the output is
//!   read back and checked sample by sample, and only then written through the host's atomic file
//!   writer, so a failed or refused export never leaves a partial file. A plan that re-encodes is
//!   refused until the Smart Render executor exists (WP-06); nothing is re-encoded silently.
//!
//! Times are ticks or time objects (`{"seconds": n}`, `{"frame": n}`, `{"timecode": "…"}`) on the
//! source stream's PTS timeline; frames and timecodes count at the stream's frame rate.

use std::io::Cursor;

use cfa4_isobmff::{ByteSource, CodecConfig, Mp4File, TrackKind};
use cfa4_smart_edit::capabilities::{Codec, Container, Policy, Registry};
use cfa4_smart_edit::copy_planner::{CutMode, SmartCutPlan, SmartCutRequest, plan_smart_cut};
use cfa4_smart_edit::h264_index::{NalFormat, index_h264};
use cfa4_smart_edit::media_index::{MediaIndex, Packet, SourceFingerprint, StreamIndex};
use cfa4_smart_edit::plan::CopyGuarantee;
use cfa4_smart_edit::timestamp::{MediaTime, Rational};
use cfa4_time::{FrameRate, Tick};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad, bool_p, str_p, tick_or_time_p};
use crate::{EngineError, Result, Session};

type Run = fn(&mut Session, &Value) -> Result<Value>;

/// Exports use the strict policy: experimental capabilities never count (developer setting off).
const POLICY: Policy = Policy { allow_experimental: false };

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
struct Source {
    path: String,
    bytes: Box<dyn ByteSource>,
    file: Mp4File,
    container: Container,
    track: usize,
    index: MediaIndex,
}

impl Source {
    fn stream(&self) -> Option<&StreamIndex> {
        self.index.streams.first()
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

fn path_p(p: &Value, cmd: &str) -> Result<String> {
    str_p(p, "path").filter(|s| !s.is_empty()).map(str::to_string).ok_or_else(|| bad(cmd, "`path` (the source file) is required"))
}

fn container_of(path: &str, quicktime: bool) -> Container {
    if quicktime || path.to_ascii_lowercase().ends_with(".mov") { Container::Mov } else { Container::Mp4 }
}

/// Open `path`, find its first H.264 video track and index it.
fn open_source(s: &Session, path: &str, cmd: &str) -> Result<Source> {
    let fail = |e: String| EngineError::Other(format!("{path}: {e}"));
    let bytes: Box<dyn ByteSource> = match s.services.reader(path) {
        Some(r) => Box::new(Reader(r.map_err(|e| fail(e.to_string()))?)),
        None => Box::new(s.services.read_file(path).map_err(|e| fail(e.to_string()))?),
    };
    let file = cfa4_isobmff::open(&*bytes).map_err(|e| fail(format!("not an MP4 / MOV file Smart Cut can read ({e})")))?;
    let track = file.tracks.iter().position(|t| t.kind == TrackKind::Video).ok_or_else(|| bad(cmd, format!("{path} has no video track")))?;
    let t = file.tracks.get(track).ok_or_else(|| bad(cmd, format!("{path} has no video track")))?;
    let Some(CodecConfig::Avc(avc)) = t.codec() else {
        return Err(bad(cmd, format!("{path}: Smart Cut copies H.264 video only in this release")));
    };
    let ps: Vec<&[u8]> = avc.sps.iter().chain(&avc.pps).map(Vec::as_slice).collect();
    let mut data = Vec::with_capacity(t.samples.len());
    for i in 0..t.samples.len() {
        data.push(file.read_sample(&*bytes, track, i).map_err(|e| fail(format!("sample {i}: {e}")))?);
    }
    let packets = t.samples.iter().zip(&data).map(|(x, d)| Packet { data: d, pts: Some(x.pts), dts: Some(x.dts), keyframe: x.is_sync });
    let stream = index_h264(t.id, Rational::new(1, i64::from(t.timescale)), NalFormat::LengthPrefixed(avc.length_size), &ps, packets)
        .map_err(|e| fail(format!("indexing failed: {e:?}")))?;
    let fingerprint = SourceFingerprint { size: bytes.len(), modified_ns: None, content_id: None };
    let container = container_of(path, file.is_quicktime);
    Ok(Source { path: path.to_string(), bytes, file, container, track, index: MediaIndex::new(path, fingerprint, vec![stream]) })
}

fn ticks(t: Option<i64>, tb: Rational) -> Value {
    t.and_then(|v| MediaTime::new(v, tb).to_tick()).map(|t| json!(t.0)).unwrap_or(Value::Null)
}

fn analyze(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "export.analyze";
    let path = path_p(p, cmd)?;
    let src = open_source(s, &path, cmd)?;
    let stream = src.stream().ok_or_else(|| EngineError::Other(format!("{path}: no stream was indexed")))?;
    let tb = stream.timebase;
    let points: Vec<Value> = stream
        .frames
        .iter()
        .filter(|f| f.random_access.is_access_point())
        .map(|f| json!({"presentation": f.presentation_index, "time": ticks(f.pts, tb), "clean": f.random_access.is_clean(), "kind": f.random_access}))
        .collect();
    let registry = Registry::native();
    let cut = registry.smart_cut(Codec::H264, src.container, src.container, POLICY);
    let rate = src.rate();
    Ok(json!({
        "path": path,
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

/// The plan of the range named in `p`.
fn plan_of(s: &Session, p: &Value, cmd: &str) -> Result<(Source, SmartCutPlan)> {
    let path = path_p(p, cmd)?;
    let src = open_source(s, &path, cmd)?;
    let rate = src.rate();
    let in_point = tick_or_time_p(p, "in", rate, false, cmd)?.ok_or_else(|| bad(cmd, "`in` is required"))?;
    let out_point = tick_or_time_p(p, "out", rate, false, cmd)?.ok_or_else(|| bad(cmd, "`out` is required (exclusive)"))?;
    let mode = match str_p(p, "mode").unwrap_or("gop-boundary") {
        "gop-boundary" => CutMode::GopBoundary,
        "frame-accurate" => CutMode::FrameAccurate,
        m => return Err(bad(cmd, format!("unknown `mode` {m:?}: gop-boundary or frame-accurate"))),
    };
    let guarantee = match str_p(p, "guarantee").unwrap_or("bit-exact") {
        "bit-exact" => CopyGuarantee::BitExact,
        "compatible" => CopyGuarantee::Compatible,
        g => return Err(bad(cmd, format!("unknown `guarantee` {g:?}: bit-exact or compatible"))),
    };
    match str_p(p, "scope").unwrap_or("source") {
        "source" => {}
        "timeline" => return Err(bad(cmd, "timeline scope (Smart Render) runs in the Smart Render executor, which is not implemented yet (WP-06)")),
        x => return Err(bad(cmd, format!("unknown `scope` {x:?}: source"))),
    }
    let output_container = match str_p(p, "output") {
        Some(o) if o.to_ascii_lowercase().ends_with(".mov") => Container::Mov,
        Some(o) if o.to_ascii_lowercase().ends_with(".mp4") || o.to_ascii_lowercase().ends_with(".m4v") => Container::Mp4,
        Some(o) => return Err(bad(cmd, format!("{o}: the output must be an .mp4, .m4v or .mov file"))),
        None => src.container,
    };
    let stream_id = src.stream().map(|x| x.stream_id).unwrap_or_default();
    let request = SmartCutRequest {
        stream_id,
        in_point: Tick(in_point.0),
        out_point: Tick(out_point.0),
        mode,
        guarantee,
        source_container: src.container,
        output_container,
        format: None,
    };
    let plan = plan_smart_cut(&Registry::native(), &src.index, &request, POLICY).map_err(|e| bad(cmd, e.to_string()))?;
    Ok((src, plan))
}

fn summary(p: &SmartCutPlan) -> Value {
    json!({
        "lossless": p.plan.is_lossless(),
        "requiresConfirmation": p.plan.requires_confirmation(),
        "copiedFrames": p.plan.copied_frames(),
        "reencodedFrames": p.plan.reencoded_frames(),
    })
}

fn plan(s: &mut Session, p: &Value) -> Result<Value> {
    let (_, plan) = plan_of(s, p, "export.plan")?;
    let summary = summary(&plan);
    Ok(json!({"plan": plan, "summary": summary}))
}

fn start(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "export.start";
    let output = str_p(p, "output").filter(|o| !o.is_empty()).map(str::to_string).ok_or_else(|| bad(cmd, "`output` (the file to write) is required"))?;
    let (src, plan) = plan_of(s, p, cmd)?;
    if output == src.path {
        return Err(bad(cmd, "the output must not be the source file"));
    }
    if plan.plan.requires_confirmation() {
        // even with "confirm": true, there is no boundary re-encoder yet
        let confirmed = bool_p(p, "confirm").unwrap_or(false);
        return Err(bad(
            cmd,
            format!(
                "the plan re-encodes {} frames at its boundaries ({}); the Smart Render executor is not implemented yet (WP-06). Use \"mode\": \"gop-boundary\" or cut at the safe points",
                plan.plan.reencoded_frames(),
                if confirmed { "confirmed" } else { "not confirmed" }
            ),
        ));
    }
    if !plan.executable {
        return Err(EngineError::Other(format!("Smart Cut cannot run: {}", plan.blockers.join("; "))));
    }
    let stream = src.stream().ok_or_else(|| EngineError::Other(format!("{}: no stream was indexed", src.path)))?;
    let source = cfa4_export::smart_cut::Mp4CopySource { file: &src.file, bytes: &*src.bytes, track: src.track };
    let (out, report) =
        cfa4_export::smart_cut::execute_mp4_copy(&plan.plan, stream, &source, Cursor::new(Vec::new())).map_err(|e| EngineError::Other(e.to_string()))?;
    let bytes = out.into_inner();
    let verified = cfa4_export::smart_cut::verify_mp4_copy(&plan.plan, stream, &source, bytes.as_slice()).map_err(|e| EngineError::Other(e.to_string()))?;
    if verified != report {
        return Err(EngineError::Other("Smart Cut verification read back another file than was written".into()));
    }
    s.services.write_file(&output, &bytes).map_err(|e| EngineError::Other(format!("{output}: {e}")))?;
    let summary = summary(&plan);
    Ok(json!({"output": output, "bytes": bytes.len(), "report": report, "verified": true, "summary": summary}))
}

fn spec(id: &'static str, label: &'static str, params: &'static str, run: Run, journal: bool) -> CommandSpec {
    CommandSpec { id, label, menu: &[], shortcut: None, params, enabled: always, run, journal }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        spec(
            "export.analyze",
            "Analyze Source for Smart Cut",
            r#"{"path":str} → {container, codec, frames, frameRate, duration, gops, accessPoints:[{presentation,time,clean,kind}], configChanges, smartCut:{allowed,blockers}}"#,
            analyze,
            false,
        ),
        spec(
            "export.plan",
            "Plan Smart Cut",
            r#"{"path":str,"in":time,"out":time (exclusive),"mode":"gop-boundary"|"frame-accurate"="gop-boundary","guarantee":"bit-exact"|"compatible"="bit-exact","output":str?,"scope":"source"="source"} → {plan, summary}"#,
            plan,
            false,
        ),
        spec(
            "export.start",
            "Run Smart Cut",
            r#"{"path":str,"in":time,"out":time,"output":str,"mode":…,"guarantee":…,"confirm":bool?} → {output, bytes, report, verified}"#,
            start,
            true,
        ),
    ]
}
