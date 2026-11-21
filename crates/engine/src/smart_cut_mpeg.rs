//! Smart Cut of MPEG-2 video in program and transport streams (CFA4-SMART-CUT-RENDER-R1, WP-09c):
//! `export.analyze`, `export.plan` and `export.start` with an `.mpg` / `.ts` / `.m2ts` source.
//! [`crate::smart_cut`] hands a source here when it is an MPEG stream.
//!
//! A GOP-boundary cut is lossless (whole GOPs, copied into a new program or transport stream with
//! the audio frames of the span at their source offset). A frame-accurate cut copies the whole
//! GOPs and re-encodes the boundary pictures with CFA4's MPEG-2 encoder (WP-09e): `export.plan`
//! reports whether it can (`boundaryReencode`), and `export.start` runs it only with
//! `"confirm": true`.
//! Media time 0 of an MPEG source is its first presented picture, as the media layer decodes it,
//! and `in` / `out` are media times with a file path too (MPEG timestamps start anywhere); the
//! access points report both their stream `time` and their `mediaTime`.

use cfa4_export::boundary_reencode::BoundaryEncoding;
use cfa4_export::boundary_reencode_mpeg::{
    check_mpeg_boundary_reencode, execute_mpeg_boundary_reencode, mpeg2_source_format, plan_mpeg_boundary_audio, verify_mpeg_boundary_reencode,
};
use cfa4_export::smart_cut::SmartCutError;
use cfa4_export::smart_cut_mpeg::{MpegCopySource, container_of, execute_mpeg_copy, index_mpeg_video, plan_mpeg_audio, verify_mpeg_copy};
use cfa4_mpegts::{ByteSource, Codec as TsCodec, File, Format, Kind, MuxFormat};
use cfa4_smart_edit::capabilities::{Container, Registry};
use cfa4_smart_edit::copy_planner::{SmartCutPlan, SmartCutRequest, plan_smart_cut};
use cfa4_smart_edit::media_index::{MediaIndex, SourceFingerprint, StreamIndex};
use cfa4_smart_edit::timestamp::{MediaTime, Rational};
use cfa4_time::{FrameRate, Tick};
use serde_json::{Value, json};

use crate::clip_ops::SourceView;
use crate::commands::{bad, bool_p, str_p};
use crate::smart_cut::{CutSource, POLICY, encoding_p, guarantee_p, mode_p, range_p, source_p, summary, ticks};
use crate::{EngineError, Result, Session};

const CLOCK: i64 = 90_000;

/// The host's random-access reader as an MPEG byte source.
struct Reader(cfa4_media::SharedReader);

impl ByteSource for Reader {
    fn len(&self) -> u64 {
        self.0.len()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
        self.0.read_at(offset, buf)
    }
}

/// Whether `path` is an MPEG program or transport stream (MP4 / MOV names are never sniffed).
pub(crate) fn is_mpeg(s: &Session, path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    if [".mp4", ".m4v", ".mov"].iter().any(|e| lower.ends_with(e)) {
        return false;
    }
    let head = match s.services.reader(path) {
        Some(Ok(r)) => {
            let mut head = vec![0u8; r.len().min(4096) as usize];
            if r.read_at(0, &mut head).is_err() {
                return false;
            }
            head
        }
        Some(Err(_)) => return false,
        None => match s.services.read_file(path) {
            Ok(mut b) => {
                b.truncate(4096);
                b
            }
            Err(_) => return false,
        },
    };
    cfa4_mpegts::sniff(&head).is_some()
}

/// An opened MPEG source: its bytes, demuxed streams, MPEG-2 video stream and its index.
struct Source {
    path: String,
    bytes: Box<dyn ByteSource>,
    file: File,
    video: usize,
    index: MediaIndex,
    /// The first presented picture (90 kHz): media time 0.
    origin: i64,
}

impl Source {
    fn stream(&self) -> Option<&StreamIndex> {
        self.index.streams.first()
    }
    fn container(&self) -> Container {
        container_of(&self.file)
    }
    fn origin(&self) -> Option<Tick> {
        MediaTime::new(self.origin, Rational::new(1, CLOCK)).to_tick()
    }
    fn stream_time(&self, t: Tick) -> Option<Tick> {
        t.0.checked_add(self.origin()?.0).map(Tick)
    }
    fn media_time(&self, t: Tick) -> Option<Tick> {
        t.0.checked_sub(self.origin()?.0).map(Tick)
    }
    /// The picture rate: the most common distance between presented pictures (25 fps if unknown).
    fn rate(&self) -> FrameRate {
        let mut pts: Vec<i64> = self.stream().map(|s| s.frames.iter().filter_map(|f| f.pts).collect()).unwrap_or_default();
        pts.sort_unstable();
        let mut gaps: Vec<i64> = pts.windows(2).map(|w| w[1] - w[0]).filter(|&d| d > 0).collect();
        gaps.sort_unstable();
        let mut best = (0usize, 0i64);
        for g in gaps.chunk_by(|a, b| a == b) {
            if g.len() > best.0 {
                best = (g.len(), g[0]);
            }
        }
        if best.1 > 0 { FrameRate { num: CLOCK, den: best.1 } } else { FrameRate::FPS_25 }
    }
    fn copy_source(&self, audio: bool) -> MpegCopySource<'_, dyn ByteSource> {
        MpegCopySource { file: &self.file, bytes: &*self.bytes, video: self.video, audio }
    }
}

/// Open `path` and index its first MPEG-2 video stream.
fn open_source(s: &Session, path: &str, cmd: &str) -> Result<Source> {
    let fail = |e: String| EngineError::Other(format!("{path}: {e}"));
    let bytes: Box<dyn ByteSource> = match s.services.reader(path) {
        Some(r) => Box::new(Reader(r.map_err(|e| fail(e.to_string()))?)),
        None => Box::new(s.services.read_file(path).map_err(|e| fail(e.to_string()))?),
    };
    let file = cfa4_mpegts::open(&*bytes).map_err(|e| fail(format!("not an MPEG program or transport stream Smart Cut can read ({e})")))?;
    let video = file
        .find(Kind::Video, |c| matches!(c, TsCodec::Mpeg2Video | TsCodec::Mpeg1Video))
        .ok_or_else(|| bad(cmd, format!("{path}: Smart Cut copies MPEG-2 video only from program and transport streams")))?;
    let stream = index_mpeg_video(&file, &*bytes, video).map_err(|e| fail(e.to_string()))?;
    let origin = file.streams[video].units.iter().filter_map(|u| u.pts).min().ok_or_else(|| fail("the video has no timestamps".into()))?;
    let fingerprint = SourceFingerprint { size: bytes.len(), modified_ns: None, content_id: None };
    Ok(Source { path: path.to_string(), bytes, file, video, index: MediaIndex::new(path, fingerprint, vec![stream]), origin })
}

/// The stream format an output file name asks for (none: the source's own).
fn output_format(src: &Source, output: Option<&str>, cmd: &str) -> Result<MuxFormat> {
    let own = match src.file.format {
        Format::Ps { .. } => MuxFormat::Ps,
        Format::Ts { packet_size: 192, .. } => MuxFormat::M2ts,
        Format::Ts { .. } => MuxFormat::Ts,
    };
    let Some(o) = output else { return Ok(own) };
    let lower = o.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map_or("", |(_, e)| e);
    match ext {
        "mpg" | "mpeg" | "m2p" | "ps" | "vob" => Ok(MuxFormat::Ps),
        "ts" | "m2t" => Ok(MuxFormat::Ts),
        "m2ts" | "mts" => Ok(MuxFormat::M2ts),
        _ => Err(bad(cmd, format!("{o}: the output of an MPEG source must be an .mpg, .ts or .m2ts file"))),
    }
}

fn container_of_format(format: MuxFormat) -> Container {
    if format == MuxFormat::Ps { Container::MpegPs } else { Container::MpegTs }
}

pub(crate) fn analyze(s: &mut Session, _p: &Value, path: &str, view: Option<&SourceView>) -> Result<Value> {
    let cmd = "export.analyze";
    let src = open_source(s, path, cmd)?;
    let stream = src.stream().ok_or_else(|| EngineError::Other(format!("{path}: no stream was indexed")))?;
    let tb = stream.timebase;
    let points: Vec<Value> = stream
        .frames
        .iter()
        .filter(|f| f.random_access.is_access_point())
        .map(|f| {
            let mut point =
                json!({"presentation": f.presentation_index, "time": ticks(f.pts, tb), "clean": f.random_access.is_clean(), "kind": f.random_access});
            {
                let media = f.pts.and_then(|v| MediaTime::new(v, tb).to_tick()).and_then(|t| src.media_time(t));
                point["mediaTime"] = media.map(|t| json!(t.0)).unwrap_or(Value::Null);
            }
            point
        })
        .collect();
    let cut = Registry::native().smart_cut(stream.codec, src.container(), src.container(), POLICY);
    let rate = src.rate();
    Ok(json!({
        "path": path,
        "item": view.map(|v| v.item.0),
        "container": src.container(),
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

/// The plan of the range named in `p`, and the output stream format.
fn plan_of(s: &Session, p: &Value, cmd: &str) -> Result<(Source, SmartCutPlan, MuxFormat)> {
    let (path, view) = source_p(s, p, cmd)?;
    let src = open_source(s, &path, cmd)?;
    let map = |t: Tick| src.stream_time(t);
    let (mut in_point, mut out_point) = range_p(p, cmd, &CutSource { path: &path, view: view.as_ref(), rate: src.rate(), stream_time: &map })?;
    if view.is_none() {
        // a file's times count from its first picture too: MPEG timestamps start anywhere
        let unmapped = || bad(cmd, format!("{path}: the time does not fit the stream's timestamps"));
        (in_point, out_point) = (src.stream_time(in_point).ok_or_else(unmapped)?, src.stream_time(out_point).ok_or_else(unmapped)?);
    }
    let format = output_format(&src, str_p(p, "output").filter(|o| !o.is_empty()), cmd)?;
    let request = SmartCutRequest {
        stream_id: src.stream().map(|x| x.stream_id).unwrap_or_default(),
        in_point,
        out_point,
        mode: mode_p(p, cmd)?,
        guarantee: guarantee_p(p, cmd)?,
        source_container: src.container(),
        output_container: container_of_format(format),
        // the encoder's limits (profile, chroma) decide whether boundaries can be re-encoded
        format: mpeg2_source_format(&src.file, &*src.bytes, src.video),
    };
    let plan = plan_smart_cut(&Registry::native(), &src.index, &request, POLICY).map_err(|e| bad(cmd, e.to_string()))?;
    Ok((src, plan, format))
}

/// Why the boundary re-encoder refuses `plan` (before anything is decoded or written).
fn reencode_problem(src: &Source, plan: &SmartCutPlan, format: MuxFormat, encoding: &BoundaryEncoding) -> Option<String> {
    let stream = src.stream()?;
    check_mpeg_boundary_reencode(&plan.plan, stream, &src.copy_source(false), format, encoding).err().map(|e| e.to_string())
}

/// The audio the cut takes along, for the plan preview (the keys of the MP4 path's `audio`).
fn audio_json(src: &Source, plan: &SmartCutPlan, format: MuxFormat, audio: bool, encoding: &BoundaryEncoding) -> Value {
    if !plan.executable {
        return json!({"tracks": [], "dropped": []});
    }
    let Some(stream) = src.stream() else { return json!({"tracks": [], "dropped": []}) };
    let source = src.copy_source(audio);
    let planned = if plan.plan.is_lossless() {
        plan_mpeg_audio(&plan.plan, stream, &source, format)
    } else {
        plan_mpeg_boundary_audio(&plan.plan, stream, &source, format, encoding)
    };
    match planned {
        Ok((tracks, dropped)) => json!({
            "tracks": tracks.iter().map(|t| json!({"trackId": t.stream, "codec": t.codec, "units": t.units})).collect::<Vec<_>>(),
            "dropped": dropped.iter().map(|d| json!({"trackId": d.stream, "reason": d.reason})).collect::<Vec<_>>(),
        }),
        Err(e) => json!({"tracks": [], "dropped": [], "problem": e.to_string()}),
    }
}

pub(crate) fn plan(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "export.plan";
    let encoding = encoding_p(p, cmd)?;
    let (src, plan, format) = plan_of(s, p, cmd)?;
    let audio = audio_json(&src, &plan, format, bool_p(p, "audio").unwrap_or(true), &encoding);
    let summary = summary(&plan);
    if !plan.plan.requires_confirmation() {
        return Ok(json!({"plan": plan, "summary": summary, "audio": audio}));
    }
    let problem = reencode_problem(&src, &plan, format, &encoding);
    Ok(json!({"plan": plan, "summary": summary, "audio": audio, "boundaryReencode": {"ready": problem.is_none(), "problem": problem}}))
}

pub(crate) fn start(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "export.start";
    let output = str_p(p, "output").filter(|o| !o.is_empty()).map(str::to_string).ok_or_else(|| bad(cmd, "`output` (the file to write) is required"))?;
    let encoding = encoding_p(p, cmd)?;
    let (src, plan, format) = plan_of(s, p, cmd)?;
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
    let source = src.copy_source(bool_p(p, "audio").unwrap_or(true));
    let other = |e: SmartCutError| EngineError::Other(e.to_string());
    let read_back = || EngineError::Other("Smart Cut verification read back another file than was written".into());
    let (bytes, audio, dropped, report) = if reencodes {
        let (bytes, report) = execute_mpeg_boundary_reencode(&plan.plan, stream, &source, format, &encoding, Vec::new()).map_err(other)?;
        let verified = verify_mpeg_boundary_reencode(&plan.plan, stream, &source, format, &encoding, &bytes).map_err(other)?;
        if (verified.samples, verified.copied, verified.reencoded, &verified.audio) != (report.samples, report.copied, report.reencoded, &report.audio) {
            return Err(read_back());
        }
        // the verified report carries the measured quality of the re-encoded pictures
        let value = serde_json::to_value(&verified).map_err(|e| EngineError::Other(e.to_string()))?;
        (bytes, verified.audio, verified.dropped, value)
    } else {
        let (bytes, report) = execute_mpeg_copy(&plan.plan, stream, &source, format, Vec::new()).map_err(other)?;
        let verified = verify_mpeg_copy(&plan.plan, stream, &source, format, &bytes).map_err(other)?;
        if verified != report {
            return Err(read_back());
        }
        let value = serde_json::to_value(&report).map_err(|e| EngineError::Other(e.to_string()))?;
        (bytes, report.audio, report.dropped, value)
    };
    s.services.write_file(&output, &bytes).map_err(|e| EngineError::Other(format!("{output}: {e}")))?;
    let audio = json!({
        "tracks": audio.iter().map(|a| json!({"trackId": a.stream, "codec": a.codec, "units": a.units})).collect::<Vec<_>>(),
        "dropped": dropped.iter().map(|d| json!({"trackId": d.stream, "reason": d.reason})).collect::<Vec<_>>(),
        "verified": audio,
    });
    Ok(json!({"output": output, "bytes": bytes.len(), "report": report, "verified": true, "summary": summary(&plan), "audio": audio}))
}
