//! MPEG-2 boundary re-encode (WP-09e of `CFA4-SMART-CUT-RENDER-R1`): a frame-accurate Smart Cut
//! of MPEG-2 video in a program or transport stream. The whole GOPs inside the cut are copied
//! access unit for access unit ([`crate::smart_cut_mpeg`]); the pictures before the first and
//! after the last copied GOP are decoded with the CFA4 MPEG-2 decoder and re-encoded with the
//! CFA4 MPEG-2 encoder, each run as one closed GOP (I and P pictures, a sequence header of its
//! own), so decoding never reaches across a splice.
//!
//! - **Stream:** the re-encoded runs keep the source's picture size, chroma format, frame rate,
//!   aspect ratio and colour description. Interlaced sources, MPEG-1 and 4:4:4 are refused before
//!   anything is written ([`check_mpeg_boundary_reencode`]), as are re-encoded pictures that do not
//!   all last one frame. A copied GOP that follows a re-encoded run must start with its own
//!   sequence header (its quantiser matrices must not come from the re-encoded run's).
//! - **Timing:** every picture keeps its source presentation time (from 0 at the first presented
//!   picture); the re-encoded pictures get decode times that fit between the copied runs.
//! - **Audio:** as for a copy: the whole audio frames of the cut at their source offset.
//! - **Validation:** [`verify_mpeg_boundary_reencode`] reopens the output, compares every copied
//!   unit with its source bytes and timestamps, decodes the whole video and measures the luma PSNR
//!   of every re-encoded picture against the source picture (SCE-001 §18).

use std::io::Write;

use cfa4_mpeg2v::encode::{AspectRatio, Config, Encoder, Input, Rate};
use cfa4_mpeg2v::{ChromaFormat, Decoder, Picture, SequenceInfo};
use cfa4_mpegts::{ByteSource, File, MuxCodec, MuxFormat, Muxer, Stream};
use cfa4_smart_edit::capabilities::{Codec, StreamFormat};
use cfa4_smart_edit::media_index::StreamIndex;
use cfa4_smart_edit::plan::{ExportExecutionPlan, PLAN_VERSION, SegmentMode};
use cfa4_smart_edit::timestamp::Rational;
use serde::{Deserialize, Serialize};

use crate::boundary_reencode::{BoundaryEncoding, incompatible};
use crate::smart_cut::{SmartCutError, io};
use crate::smart_cut_mpeg::{
    CLOCK, MpegAudioDrop, MpegAudioReport, MpegCopySource, audio_copies, container_of, container_of_format, output_id, verify_audio, write_audio,
};

/// Profile, chroma format and bit depth of an MPEG-2 video stream, for the registry's encoder
/// limits (the CFA4 encoder: Main and 4:2:2 profiles, 8 bits).
pub fn mpeg2_stream_format(info: &SequenceInfo) -> StreamFormat {
    let (profile, chroma) = match info.chroma {
        ChromaFormat::Yuv420 => ("main", "4:2:0"),
        ChromaFormat::Yuv422 => ("422", "4:2:2"),
        ChromaFormat::Yuv444 => ("high", "4:4:4"),
    };
    StreamFormat { profile: profile.into(), chroma: chroma.into(), bit_depth: 8 }
}

/// What an MPEG-2 boundary re-encode wrote.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MpegBoundaryReport {
    /// Pictures written.
    pub samples: u64,
    /// Of those, copied unchanged from the source.
    pub copied: u64,
    /// Of those, decoded and re-encoded.
    pub reencoded: u64,
    /// Video payload bytes written.
    pub payload_bytes: u64,
    /// Presented video, 90 kHz.
    pub duration: u64,
    pub timescale: u32,
    /// Lowest luma PSNR of a re-encoded picture (dB; `None` before verification or when nothing
    /// was re-encoded). Identical pictures count as 99 dB.
    pub min_psnr: Option<f64>,
    pub audio: Vec<MpegAudioReport>,
    pub dropped: Vec<MpegAudioDrop>,
}

/// One planned segment, resolved against the source video stream.
enum Seg {
    /// Units `first..=last` (decode order), copied.
    Copy { first: usize, last: usize },
    /// Units `decode_first..=decode_last` decoded; `pictures` (unit indices, in presentation
    /// order) re-encoded.
    Reencode { decode_first: usize, decode_last: usize, pictures: Vec<usize> },
}

struct Prepared<'a> {
    stream: &'a Stream,
    segs: Vec<Seg>,
    /// The first presented picture's PTS: output time 0.
    base: i64,
    /// Presented length (90 kHz).
    duration: i64,
    /// One frame (90 kHz): the most common distance between presented pictures.
    frame: i64,
}

fn pts_of(stream: &Stream, d: usize) -> Result<i64, SmartCutError> {
    stream.units.get(d).and_then(|u| u.pts).ok_or(SmartCutError::Timestamps { decode_index: d as u64 })
}

fn dts_of(stream: &Stream, d: usize) -> Result<i64, SmartCutError> {
    stream.units.get(d).and_then(|u| u.dts.or(u.pts)).ok_or(SmartCutError::Timestamps { decode_index: d as u64 })
}

/// Resolve `plan` against the source video stream, after checking that `index` still describes it.
fn prepare<'a, S: ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &MpegCopySource<'a, S>,
    format: MuxFormat,
) -> Result<Prepared<'a>, SmartCutError> {
    if plan.plan_version != PLAN_VERSION {
        return Err(SmartCutError::PlanVersion { found: plan.plan_version });
    }
    if plan.segments.iter().any(|s| s.source.is_some()) {
        return Err(SmartCutError::TimelinePlan);
    }
    let unsupported = SmartCutError::Unsupported { codec: plan.codec, source: plan.source_container, output: plan.output_container };
    if plan.codec != Codec::Mpeg2Video
        || index.codec != Codec::Mpeg2Video
        || plan.source_container != container_of(source.file)
        || plan.output_container != container_of_format(format)
    {
        return Err(unsupported);
    }
    let stream = source.file.streams.get(source.video).ok_or(SmartCutError::NoTrack { track: source.video })?;
    if index.stream_id != source.video as u32 {
        return Err(SmartCutError::WrongStream { expected: source.video as u32, found: index.stream_id });
    }
    if index.timebase != Rational::new(1, CLOCK) {
        return Err(SmartCutError::StaleIndex { detail: "the index is not on the 90 kHz clock".into() });
    }
    let n = stream.units.len();
    if index.frames.len() != n {
        return Err(SmartCutError::StaleIndex { detail: format!("{} indexed pictures, {n} units in the stream", index.frames.len()) });
    }
    for (d, (u, f)) in stream.units.iter().zip(&index.frames).enumerate() {
        if u.size != f.size || u.pts != f.pts || u.dts.or(u.pts) != f.dts {
            return Err(SmartCutError::StaleIndex { detail: format!("unit {d} changed (size or timestamps)") });
        }
        if u.pts.is_none() {
            return Err(SmartCutError::Timestamps { decode_index: d as u64 });
        }
    }
    // presentation order and position of every unit
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&d| (stream.units[d].pts, d));
    let mut position = vec![0u64; n];
    for (p, &d) in order.iter().enumerate() {
        position[d] = p as u64;
    }
    let mut gaps: Vec<i64> = order.windows(2).filter_map(|w| Some(stream.units[w[1]].pts? - stream.units[w[0]].pts?)).filter(|&g| g > 0).collect();
    gaps.sort_unstable();
    let frame = gaps.chunk_by(|a, b| a == b).max_by_key(|g| g.len()).map_or(0, |g| g[0]);
    if frame <= 0 {
        return Err(SmartCutError::Timestamps { decode_index: 0 });
    }
    let shown_for = |d: usize| -> i64 {
        let p = position[d] as usize;
        match order.get(p + 1) {
            Some(&next) => stream.units[next].pts.unwrap_or(0) - stream.units[d].pts.unwrap_or(0),
            None => frame,
        }
    };
    let mut segs = Vec::with_capacity(plan.segments.len());
    let mut next_position: Option<u64> = None;
    let mut duration = 0i64;
    let mut base: Option<i64> = None;
    for (i, s) in plan.segments.iter().enumerate() {
        if s.stream_id != index.stream_id {
            return Err(SmartCutError::WrongStream { expected: index.stream_id, found: s.stream_id });
        }
        if s.decode.last < s.decode.first || s.presentation.last < s.presentation.first || next_position.is_some_and(|p| p != s.presentation.first) {
            return Err(SmartCutError::NotContiguous { segment: i });
        }
        next_position = s.presentation.last.checked_add(1);
        let as_index = |d: u64| usize::try_from(d).ok().filter(|&d| d < n);
        let (Some(first), Some(last)) = (as_index(s.decode.first), as_index(s.decode.last)) else {
            return Err(SmartCutError::StaleIndex { detail: format!("units {}–{} are past the end of the stream", s.decode.first, s.decode.last) });
        };
        let mut shown = Vec::new();
        for p in s.presentation.first..=s.presentation.last {
            match usize::try_from(p).ok().and_then(|p| order.get(p)).copied() {
                Some(d) if (first..=last).contains(&d) => shown.push(d),
                _ => return Err(SmartCutError::StaleIndex { detail: format!("picture {p} of segment {i} is not decoded by it") }),
            }
        }
        for &d in &shown {
            duration += shown_for(d);
        }
        if base.is_none() {
            base = shown.first().map(|&d| stream.units[d].pts.unwrap_or(0));
        }
        match s.mode {
            SegmentMode::StrictCopy | SegmentMode::CompatibleCopy => {
                if shown.len() != last - first + 1 {
                    return Err(SmartCutError::NotContiguous { segment: i });
                }
                segs.push(Seg::Copy { first, last });
            }
            SegmentMode::BoundaryReencode => {
                if !stream.units[first].key {
                    return Err(SmartCutError::StaleIndex { detail: format!("segment {i} does not decode from an access point") });
                }
                segs.push(Seg::Reencode { decode_first: first, decode_last: last, pictures: shown });
            }
            SegmentMode::Render => return Err(SmartCutError::RenderSegment { segment: i }),
        }
    }
    let base = base.ok_or(SmartCutError::NotLossless)?;
    if plan.expected_duration.timebase != Rational::new(1, CLOCK) || plan.expected_duration.value != duration {
        return Err(SmartCutError::StaleIndex {
            detail: format!("the planned pictures last {duration} ticks, the plan says {}", plan.expected_duration.value),
        });
    }
    Ok(Prepared { stream, segs, base, duration, frame })
}

/// Decode units `first..=last` and return the pictures shown at the PTS of each of `pictures`.
fn decode_run<S: ByteSource + ?Sized>(
    source: &MpegCopySource<'_, S>,
    stream: &Stream,
    first: usize,
    last: usize,
    pictures: &[usize],
) -> Result<(Vec<Picture>, SequenceInfo), SmartCutError> {
    let mut dec = Decoder::new();
    dec.set_threads(false);
    let mut out = Vec::new();
    for d in first..=last {
        let data = source.file.read_unit(source.bytes, source.video, d).map_err(io)?;
        let got = dec.decode(&data, pts_of(stream, d)?).map_err(|e| incompatible(format!("unit {d} does not decode: {e}")))?;
        out.extend(got);
    }
    out.extend(dec.flush());
    let info = dec.sequence().cloned().ok_or_else(|| incompatible(format!("unit {first} has no sequence header before it")))?;
    let mut shown = Vec::with_capacity(pictures.len());
    for &d in pictures {
        let pts = pts_of(stream, d)?;
        let k = out.iter().position(|p| p.pts == pts).ok_or_else(|| incompatible(format!("picture of unit {d} was not decoded")))?;
        shown.push(out.swap_remove(k));
    }
    Ok((shown, info))
}

/// The encoder configuration that reproduces the source stream's format, or why there is none.
fn encoder_config(info: &SequenceInfo, pictures: usize, encoding: &BoundaryEncoding) -> Result<Config, SmartCutError> {
    if !info.mpeg2 {
        return Err(incompatible("MPEG-1 video is not re-encoded"));
    }
    if !info.progressive_sequence {
        return Err(incompatible("interlaced MPEG-2 video is not re-encoded yet"));
    }
    if info.chroma == ChromaFormat::Yuv444 {
        return Err(incompatible("4:4:4 MPEG-2 video is not re-encoded"));
    }
    let (num, den) = info.frame_rate.ok_or_else(|| incompatible("the stream signals no frame rate"))?;
    let aspect = match (info.dar, info.sar) {
        (Some((4, 3)), _) => AspectRatio::Dar4x3,
        (Some((16, 9)), _) => AspectRatio::Dar16x9,
        (Some((221, 100)), _) => AspectRatio::Dar221x100,
        (None, (a, b)) if a == b => AspectRatio::Square,
        (dar, sar) => return Err(incompatible(format!("aspect ratio {dar:?} / {sar:?} cannot be signalled again"))),
    };
    let mut cfg = Config::new(info.width, info.height, num, den);
    cfg.chroma = info.chroma;
    // the CRF scale (0–51, 16 visually lossless) onto quantiser_scale_code (1–31)
    cfg.rate = Rate::Quantizer((encoding.crf / 4.0).round().clamp(1.0, 31.0) as u8);
    cfg.gop = u32::try_from(pictures).unwrap_or(u32::MAX).saturating_add(1);
    cfg.b_frames = 0;
    // the source's flag: a decoder that switches delay mode mid-stream drops or repeats a picture
    cfg.low_delay = Some(info.low_delay);
    cfg.closed_gop = true;
    cfg.aspect = aspect;
    cfg.colour = info.colour;
    cfg.threads = encoding.threads != 1;
    Ok(cfg)
}

/// The refusals the executor makes before it writes: the plan, the source and the re-encoded
/// runs' format (decoding the first access unit of each).
struct Setup<'a> {
    p: Prepared<'a>,
}

fn setup<'a, S: ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &MpegCopySource<'a, S>,
    format: MuxFormat,
    encoding: &BoundaryEncoding,
) -> Result<Setup<'a>, SmartCutError> {
    let p = prepare(plan, index, source, format)?;
    for (i, seg) in p.segs.iter().enumerate() {
        match seg {
            Seg::Reencode { decode_first, pictures, .. } => {
                // one frame each: the encoder signals one rate
                for &d in pictures {
                    let next = p.stream.units.iter().filter_map(|u| u.pts).filter(|&t| t > pts_of(p.stream, d).unwrap_or(i64::MAX)).min();
                    if next.is_some_and(|n| n - pts_of(p.stream, d).unwrap_or(0) != p.frame) {
                        return Err(incompatible("the re-encoded pictures have a variable frame rate"));
                    }
                }
                let data = source.file.read_unit(source.bytes, source.video, *decode_first).map_err(io)?;
                let mut dec = Decoder::new();
                let _ = dec.decode(&data, 0);
                let info = dec.sequence().cloned().ok_or_else(|| incompatible(format!("unit {decode_first} does not start with a sequence header")))?;
                let cfg = encoder_config(&info, pictures.len(), encoding)?;
                let frame = i64::from(cfg.fps_den) * CLOCK;
                if frame % i64::from(cfg.fps_num) != 0 || frame / i64::from(cfg.fps_num) != p.frame {
                    return Err(incompatible(format!("the stream's frame rate {}/{} does not match its timestamps", cfg.fps_num, cfg.fps_den)));
                }
                Encoder::new(cfg).map_err(|e| incompatible(e.to_string()))?;
            }
            Seg::Copy { first, .. } => {
                // a copied GOP after a re-encoded run brings its own sequence header
                if i > 0 {
                    let data = source.file.read_unit(source.bytes, source.video, *first).map_err(io)?;
                    if !cfa4_mpeg2v::scan_access_unit(&data).sequence_header {
                        return Err(incompatible(format!("the copied GOP at unit {first} has no sequence header of its own")));
                    }
                }
            }
        }
    }
    Ok(Setup { p })
}

/// The refusals [`execute_mpeg_boundary_reencode`] makes before it writes, without decoding more
/// than one access unit per re-encoded run.
pub fn check_mpeg_boundary_reencode<S: ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &MpegCopySource<'_, S>,
    format: MuxFormat,
    encoding: &BoundaryEncoding,
) -> Result<(), SmartCutError> {
    setup(plan, index, source, format, encoding).map(|_| ())
}

/// The planar 8-bit input of a decoded picture.
fn input(p: &Picture) -> Input<'_> {
    Input { y: &p.y, cb: &p.cb, cr: &p.cr, y_stride: p.width as usize, c_stride: p.chroma_width() as usize }
}

/// Luma PSNR of `b` against `a` (99 dB when identical).
fn luma_psnr(a: &Picture, b: &Picture) -> Option<f64> {
    if (a.width, a.height) != (b.width, b.height) || a.y.len() != b.y.len() || a.y.is_empty() {
        return None;
    }
    let se: u64 = a.y.iter().zip(&b.y).map(|(&x, &y)| u64::from(x.abs_diff(y)).pow(2)).sum();
    if se == 0 {
        return Some(99.0);
    }
    let mse = se as f64 / a.y.len() as f64;
    Some(10.0 * (255.0f64 * 255.0 / mse).log10())
}

/// The output times (PTS, DTS) of every written picture, in write order, with what it is:
/// `Ok(d)` copied unit `d`, `Err(d)` the re-encoded picture of unit `d`.
type Timeline = Vec<(Result<usize, usize>, i64, i64)>;

fn timeline(p: &Prepared<'_>) -> Result<Timeline, SmartCutError> {
    let mut out: Timeline = Vec::new();
    let mut last_dts: Option<i64> = None;
    for (i, seg) in p.segs.iter().enumerate() {
        match seg {
            Seg::Copy { first, last } => {
                for d in *first..=*last {
                    let (pts, dts) = (pts_of(p.stream, d)? - p.base, dts_of(p.stream, d)? - p.base);
                    if last_dts.is_some_and(|l| dts <= l) {
                        return Err(SmartCutError::Timestamps { decode_index: d as u64 });
                    }
                    last_dts = Some(dts);
                    out.push((Ok(d), pts, dts));
                }
            }
            Seg::Reencode { pictures, .. } => {
                // decode times that end before the next copied run starts decoding
                let next = match p.segs.get(i + 1) {
                    Some(Seg::Copy { first, .. }) => Some(dts_of(p.stream, *first)? - p.base),
                    _ => None,
                };
                let n = pictures.len() as i64;
                for (k, &d) in pictures.iter().enumerate() {
                    let pts = pts_of(p.stream, d)? - p.base;
                    let mut dts = pts;
                    if let Some(nd) = next {
                        dts = dts.min(nd - (n - k as i64) * p.frame);
                    }
                    if let Some(l) = last_dts {
                        dts = dts.max(l + 1);
                    }
                    if dts > pts {
                        return Err(SmartCutError::Timestamps { decode_index: d as u64 });
                    }
                    last_dts = Some(dts);
                    out.push((Err(d), pts, dts));
                }
            }
        }
    }
    Ok(out)
}

/// Write `plan` (copied GOPs and re-encoded boundaries of MPEG-2 video) from `source` into `out`
/// as a new program or transport stream (`format`), with the audio of the same span. Everything
/// [`check_mpeg_boundary_reencode`] refuses is refused before anything is written; on a later
/// error `out` may hold a partial file, so write to a temporary file and keep it only after
/// [`verify_mpeg_boundary_reencode`].
pub fn execute_mpeg_boundary_reencode<W: Write, S: ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &MpegCopySource<'_, S>,
    format: MuxFormat,
    encoding: &BoundaryEncoding,
    out: W,
) -> Result<(W, MpegBoundaryReport), SmartCutError> {
    let Setup { p } = setup(plan, index, source, format, encoding)?;
    let times = timeline(&p)?;
    let (audio, dropped) = audio_copies(source, format, p.base, p.base + p.duration)?;
    let mut codecs = vec![MuxCodec::Mpeg2Video];
    codecs.extend(audio.iter().map(|a| a.codec));
    let span = p.stream.pts_range().map_or(CLOCK, |(a, b)| (b - a).max(CLOCK));
    let rate = (source.bytes.len().saturating_mul(8 * 2 * CLOCK as u64) / span as u64).max(20_000_000);
    let mux_err = |e: cfa4_mpegts::MuxError| SmartCutError::Io { detail: e.to_string() };
    let mut m = Muxer::new(out, format, &codecs, rate).map_err(mux_err)?;
    let mut report = MpegBoundaryReport {
        samples: 0,
        copied: 0,
        reencoded: 0,
        payload_bytes: 0,
        duration: p.duration as u64,
        timescale: CLOCK as u32,
        min_psnr: None,
        audio: Vec::new(),
        dropped,
    };
    let mut at = 0usize;
    for seg in &p.segs {
        let written: Vec<(Vec<u8>, bool)> = match seg {
            Seg::Copy { first, last } => {
                let mut v = Vec::with_capacity(last - first + 1);
                for d in *first..=*last {
                    let data = source.file.read_unit(source.bytes, source.video, d).map_err(io)?;
                    if data.len() as u64 != u64::from(p.stream.units[d].size) {
                        return Err(SmartCutError::StaleIndex { detail: format!("unit {d}: read {} bytes, expected {}", data.len(), p.stream.units[d].size) });
                    }
                    v.push((data, p.stream.units[d].key));
                }
                report.copied += v.len() as u64;
                v
            }
            Seg::Reencode { decode_first, decode_last, pictures } => {
                let (shown, info) = decode_run(source, p.stream, *decode_first, *decode_last, pictures)?;
                let mut enc = Encoder::new(encoder_config(&info, pictures.len(), encoding)?).map_err(|e| incompatible(e.to_string()))?;
                let mut packets = Vec::with_capacity(shown.len());
                for pic in &shown {
                    packets.extend(enc.encode(&input(pic)).map_err(|e| incompatible(e.to_string()))?);
                }
                packets.extend(enc.flush().map_err(|e| incompatible(e.to_string()))?);
                if packets.len() != pictures.len() {
                    return Err(incompatible(format!("{} pictures re-encoded into {} packets", pictures.len(), packets.len())));
                }
                report.reencoded += packets.len() as u64;
                packets.into_iter().map(|k| (k.data, k.keyframe)).collect()
            }
        };
        for (data, key) in written {
            let (_, pts, dts) = times.get(at).copied().ok_or(SmartCutError::Timestamps { decode_index: at as u64 })?;
            m.write(0, &data, pts, (dts != pts).then_some(dts), key).map_err(|_| SmartCutError::Timestamps { decode_index: at as u64 })?;
            report.samples += 1;
            report.payload_bytes = report.payload_bytes.saturating_add(data.len() as u64);
            at += 1;
        }
    }
    report.audio = write_audio(&mut m, source, &audio, p.base)?;
    let out = m.finish().map_err(mux_err)?;
    Ok((out, report))
}

/// Read back a stream written by [`execute_mpeg_boundary_reencode`] and check it: the copied
/// units byte for byte with their timestamps, every picture at its source presentation time, the
/// whole video decoding without errors, every re-encoded picture at least
/// [`BoundaryEncoding::min_psnr`] against its source picture, and the audio as for a copy.
pub fn verify_mpeg_boundary_reencode<S: ByteSource + ?Sized, O: ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &MpegCopySource<'_, S>,
    format: MuxFormat,
    encoding: &BoundaryEncoding,
    output: &O,
) -> Result<MpegBoundaryReport, SmartCutError> {
    let fail = |detail: String| SmartCutError::Verification { detail };
    let Setup { p } = setup(plan, index, source, format, encoding)?;
    let times = timeline(&p)?;
    let (audio, dropped) = audio_copies(source, format, p.base, p.base + p.duration)?;
    let file: File = cfa4_mpegts::open(output).map_err(|e| fail(format!("the output does not open: {e}")))?;
    if container_of(&file) != container_of_format(format) {
        return Err(fail("the output is another container".into()));
    }
    let mut codecs = vec![MuxCodec::Mpeg2Video];
    codecs.extend(audio.iter().map(|a| a.codec));
    let find = |k: usize| -> Result<usize, SmartCutError> {
        let id = output_id(format, &codecs, k).ok_or_else(|| fail(format!("no output stream {k}")))?;
        file.streams.iter().position(|s| s.id == id).ok_or_else(|| fail(format!("output stream {id:?} is missing")))
    };
    let delay = Muxer::<Vec<u8>>::DELAY;
    let ov = find(0)?;
    let out = &file.streams[ov];
    if out.units.len() != times.len() {
        return Err(fail(format!("{} pictures written, {} planned", out.units.len(), times.len())));
    }
    let mut report = MpegBoundaryReport {
        samples: 0,
        copied: 0,
        reencoded: 0,
        payload_bytes: 0,
        duration: p.duration as u64,
        timescale: CLOCK as u32,
        min_psnr: None,
        audio: Vec::new(),
        dropped,
    };
    // timestamps and copied bytes; the whole video decodes
    let mut dec = Decoder::new();
    dec.set_threads(false);
    let mut decoded = Vec::new();
    for (k, &(what, pts, dts)) in times.iter().enumerate() {
        let u = &out.units[k];
        if u.pts != Some(pts + delay) || u.dts.or(u.pts) != Some(dts + delay) {
            return Err(fail(format!("picture {k}: timestamps changed")));
        }
        let got = file.read_unit(output, ov, k).map_err(io)?;
        match what {
            Ok(d) => {
                if got != source.file.read_unit(source.bytes, source.video, d).map_err(io)? || u.key != p.stream.units[d].key {
                    return Err(fail(format!("picture {k} differs from source unit {d}")));
                }
                report.copied += 1;
            }
            Err(_) => report.reencoded += 1,
        }
        decoded.extend(dec.decode(&got, pts).map_err(|e| fail(format!("picture {k} does not decode: {e}")))?);
        report.samples += 1;
        report.payload_bytes = report.payload_bytes.saturating_add(got.len() as u64);
    }
    decoded.extend(dec.flush());
    if dec.errors() > 0 || decoded.len() != times.len() {
        return Err(fail(format!("the video decodes to {} pictures with {} errors, {} were written", decoded.len(), dec.errors(), times.len())));
    }
    // the re-encoded pictures against their source pictures
    for seg in &p.segs {
        let Seg::Reencode { decode_first, decode_last, pictures } = seg else { continue };
        let (want, _) = decode_run(source, p.stream, *decode_first, *decode_last, pictures)?;
        for (pic, &d) in want.iter().zip(pictures) {
            let pts = pts_of(p.stream, d)? - p.base;
            let got = decoded.iter().find(|x| x.pts == pts).ok_or_else(|| fail(format!("the re-encoded picture of unit {d} is missing")))?;
            let psnr = luma_psnr(pic, got).ok_or_else(|| fail(format!("the re-encoded picture of unit {d} has another size")))?;
            if psnr < encoding.min_psnr {
                return Err(fail(format!("the re-encoded picture of unit {d} measures {psnr:.1} dB, below {:.1} dB", encoding.min_psnr)));
            }
            report.min_psnr = Some(report.min_psnr.map_or(psnr, |m: f64| m.min(psnr)));
        }
    }
    report.audio = verify_audio(&file, output, source, &audio, p.base, &find)?;
    Ok(report)
}
