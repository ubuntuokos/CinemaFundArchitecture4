//! MPEG-2 Smart Cut (WP-09c of `CFA4-SMART-CUT-RENDER-R1`): the copy segments of a plan are
//! written from an MPEG program or transport stream into a new one, access unit for access unit,
//! with the audio of the same span, and the result is read back and checked.
//!
//! - **Video:** every copied picture is written with its source bytes (sequence and GOP headers
//!   included) unchanged; PTS and DTS keep their source distances and start at 0 at the first
//!   presented picture (the muxer adds its own delay, [`Muxer::DELAY`]).
//! - **Audio:** MPEG audio (layers I-III) and AC-3 are copied in either container, AAC (ADTS)
//!   into transport streams and DVD LPCM into program streams. Whole frames whose start lies in
//!   the cut are copied unchanged and keep their source timing against the video, so the A/V
//!   offset of the source is kept exactly; the audio starts at most one frame after the first
//!   picture and ends at most one frame before the cut's end. Other audio (E-AC-3, DTS, LATM,
//!   Blu-ray LPCM, or a codec the output container cannot carry) is named in
//!   [`MpegCopyReport::dropped`]; nothing is dropped silently.
//! - **Validation:** [`verify_mpeg_copy`] reopens the output and compares every video and audio
//!   unit with its source unit: payload bytes and timestamps (SCE-001 §18).

use std::io::Write;

use cfa4_mpegts::{ByteSource, Codec as TsCodec, File, Kind, MuxCodec, MuxFormat, Muxer, StreamId};
use cfa4_smart_edit::capabilities::{Codec, Container};
use cfa4_smart_edit::media_index::{Packet, StreamIndex};
use cfa4_smart_edit::mpeg2_index::index_mpeg2;
use cfa4_smart_edit::plan::{ExportExecutionPlan, PLAN_VERSION};
use cfa4_smart_edit::timestamp::Rational;
use serde::{Deserialize, Serialize};

use crate::smart_cut::{SmartCutError, io};

/// 90 kHz: the clock of every MPEG-2 Systems timestamp.
pub(crate) const CLOCK: i64 = 90_000;

/// An opened MPEG program or transport stream and the video stream a plan cuts.
pub struct MpegCopySource<'a, S: ByteSource + ?Sized> {
    pub file: &'a File,
    pub bytes: &'a S,
    /// Index into [`File::streams`]; the plan's `stream_id` is this index.
    pub video: usize,
}

/// The container of an MPEG file, as the capability registry names it.
pub fn container_of(file: &File) -> Container {
    match file.format {
        cfa4_mpegts::Format::Ps { .. } => Container::MpegPs,
        _ => Container::MpegTs,
    }
}

/// The WP-02 index of MPEG-2 video stream `video` (stream id = the stream's index, 90 kHz).
pub fn index_mpeg_video<S: ByteSource + ?Sized>(file: &File, bytes: &S, video: usize) -> Result<StreamIndex, SmartCutError> {
    let stream = file.streams.get(video).ok_or(SmartCutError::NoTrack { track: video })?;
    if !matches!(stream.codec, TsCodec::Mpeg2Video | TsCodec::Mpeg1Video) {
        return Err(SmartCutError::Unsupported { codec: Codec::Mpeg2Video, source: container_of(file), output: container_of(file) });
    }
    let mut data = Vec::with_capacity(stream.units.len());
    for i in 0..stream.units.len() {
        data.push(file.read_unit(bytes, video, i).map_err(io)?);
    }
    let packets = stream.units.iter().zip(&data).map(|(u, d)| Packet { data: d, pts: u.pts, dts: u.dts.or(u.pts), keyframe: u.key });
    index_mpeg2(video as u32, Rational::new(1, CLOCK), packets).map_err(|e| SmartCutError::StaleIndex { detail: format!("indexing failed: {e:?}") })
}

/// One audio stream of a copy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MpegAudioReport {
    /// Index into the source's [`File::streams`].
    pub stream: usize,
    pub codec: String,
    pub units: u64,
    pub payload_bytes: u64,
    /// First copied audio frame against the first picture, in 90 kHz ticks (the same in the
    /// source and the output).
    pub offset: i64,
}

/// An audio stream the copy leaves out, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MpegAudioDrop {
    /// Index into the source's [`File::streams`].
    pub stream: usize,
    pub reason: String,
}

/// What an MPEG copy wrote.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MpegCopyReport {
    /// Pictures written.
    pub samples: u64,
    pub payload_bytes: u64,
    /// Presented video, 90 kHz.
    pub duration: u64,
    pub timescale: u32,
    pub audio: Vec<MpegAudioReport>,
    pub dropped: Vec<MpegAudioDrop>,
}

/// The output container of `format`.
pub(crate) fn container_of_format(format: MuxFormat) -> Container {
    match format {
        MuxFormat::Ps => Container::MpegPs,
        MuxFormat::Ts | MuxFormat::M2ts => Container::MpegTs,
    }
}

/// An audio stream to copy: its source stream, how the muxer writes it and its units in the cut,
/// with each unit's time (90 kHz, source clock).
pub(crate) struct AudioCopy {
    pub(crate) stream: usize,
    pub(crate) codec: MuxCodec,
    pub(crate) units: Vec<(usize, i64)>,
}

/// The source video units a lossless plan copies (decode order), checked against the index.
fn video_range<S: ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &MpegCopySource<'_, S>,
    format: MuxFormat,
) -> Result<(usize, usize), SmartCutError> {
    if plan.plan_version != PLAN_VERSION {
        return Err(SmartCutError::PlanVersion { found: plan.plan_version });
    }
    if !plan.is_lossless() {
        return Err(SmartCutError::NotLossless);
    }
    if plan.segments.iter().any(|s| s.source.is_some()) {
        return Err(SmartCutError::TimelinePlan);
    }
    let unsupported = SmartCutError::Unsupported { codec: plan.codec, source: plan.source_container, output: plan.output_container };
    if plan.codec != Codec::Mpeg2Video
        || !matches!(plan.source_container, Container::MpegPs | Container::MpegTs)
        || plan.source_container != container_of(source.file)
        || plan.output_container != container_of_format(format)
        || index.codec != Codec::Mpeg2Video
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
    if index.frames.len() != stream.units.len() {
        return Err(SmartCutError::StaleIndex { detail: format!("{} indexed pictures, {} units in the stream", index.frames.len(), stream.units.len()) });
    }
    let mut range: Option<(u64, u64)> = None;
    for (i, s) in plan.segments.iter().enumerate() {
        if s.stream_id != index.stream_id {
            return Err(SmartCutError::WrongStream { expected: index.stream_id, found: s.stream_id });
        }
        if s.decode.last < s.decode.first {
            return Err(SmartCutError::NotContiguous { segment: i });
        }
        range = match range {
            None => Some((s.decode.first, s.decode.last)),
            Some((first, last)) if last.checked_add(1) == Some(s.decode.first) => Some((first, s.decode.last)),
            Some(_) => return Err(SmartCutError::NotContiguous { segment: i }),
        };
    }
    let (first, last) = range.ok_or(SmartCutError::NotLossless)?;
    let as_index = |d: u64| usize::try_from(d).ok().filter(|&d| d < stream.units.len());
    let (Some(first), Some(last)) = (as_index(first), as_index(last)) else {
        return Err(SmartCutError::StaleIndex { detail: format!("units {first}–{last} are past the end of the stream") });
    };
    for d in first..=last {
        let (Some(u), Some(f)) = (stream.units.get(d), index.frames.get(d)) else {
            return Err(SmartCutError::StaleIndex { detail: format!("unit {d} is missing") });
        };
        if u.size != f.size || u.pts != f.pts || u.dts.or(u.pts) != f.dts {
            return Err(SmartCutError::StaleIndex { detail: format!("unit {d} changed (size or timestamps)") });
        }
        if u.pts.is_none() {
            return Err(SmartCutError::Timestamps { decode_index: d as u64 });
        }
    }
    Ok((first, last))
}

/// Duration (90 kHz) of audio unit `data` of a stream, from its frame header (LPCM: its size).
fn audio_duration(stream: &cfa4_mpegts::Stream, data: &[u8]) -> Option<i64> {
    if let Some(l) = stream.lpcm.filter(|_| stream.codec == TsCodec::LpcmDvd) {
        let frame = i64::from(l.channels.max(1)) * i64::from(l.bits.div_ceil(8).max(1));
        return Some(data.len() as i64 / frame * CLOCK / i64::from(l.sample_rate.max(1)));
    }
    let info = cfa4_mpegts::frame_info(&stream.codec, data)?;
    Some(i64::from(info.samples) * CLOCK / i64::from(info.sample_rate.max(1)))
}

/// How `stream` is written to `format`, or why it is not.
fn mux_codec(stream: &cfa4_mpegts::Stream, format: MuxFormat) -> Result<MuxCodec, String> {
    let ps = format == MuxFormat::Ps;
    match (&stream.codec, stream.lpcm) {
        (TsCodec::MpegAudio, _) => Ok(MuxCodec::MpegAudio),
        (TsCodec::Ac3, _) => Ok(MuxCodec::Ac3),
        (TsCodec::AacAdts, _) if !ps => Ok(MuxCodec::AacAdts),
        (TsCodec::LpcmDvd, Some(l)) if ps && l.bits == 16 => Ok(MuxCodec::LpcmDvd { sample_rate: l.sample_rate, channels: l.channels }),
        (TsCodec::AacAdts, _) => Err("AAC is written to transport streams only".into()),
        (TsCodec::LpcmDvd, _) => Err("DVD LPCM is written to 16-bit program streams only".into()),
        (c, _) => Err(format!("{} audio is not copied yet", c.name())),
    }
}

/// The audio streams of the cut `[start, end)` (90 kHz, source clock).
pub(crate) fn audio_copies<S: ByteSource + ?Sized>(
    source: &MpegCopySource<'_, S>,
    format: MuxFormat,
    start: i64,
    end: i64,
) -> Result<(Vec<AudioCopy>, Vec<MpegAudioDrop>), SmartCutError> {
    let (mut copies, mut dropped) = (Vec::new(), Vec::new());
    for (k, stream) in source.file.streams.iter().enumerate() {
        if stream.kind() != Kind::Audio {
            continue;
        }
        let codec = match mux_codec(stream, format) {
            Ok(c) => c,
            Err(reason) => {
                dropped.push(MpegAudioDrop { stream: k, reason });
                continue;
            }
        };
        // every unit's time: its PTS, or the previous one's end
        let mut units = Vec::new();
        let mut next: Option<i64> = None;
        for i in 0..stream.units.len() {
            let u = &stream.units[i];
            let t = match (u.pts, next) {
                (Some(p), _) => p,
                (None, Some(n)) => n,
                (None, None) => continue,
            };
            let data = source.file.read_unit(source.bytes, k, i).map_err(io)?;
            next = audio_duration(stream, &data).map(|d| t + d);
            if t >= start && t < end && next.is_some_and(|n| n <= end) {
                units.push((i, t));
            }
        }
        if units.is_empty() {
            dropped.push(MpegAudioDrop { stream: k, reason: "the audio does not overlap the cut".into() });
        } else {
            copies.push(AudioCopy { stream: k, codec, units });
        }
    }
    Ok((copies, dropped))
}

/// The first presented picture's PTS and the presented length (90 kHz) of the copied run.
fn video_span(plan: &ExportExecutionPlan, stream: &cfa4_mpegts::Stream, first: usize, last: usize) -> Result<(i64, i64), SmartCutError> {
    let base = stream
        .units
        .get(first..=last)
        .unwrap_or_default()
        .iter()
        .filter_map(|u| u.pts)
        .min()
        .ok_or(SmartCutError::Timestamps { decode_index: first as u64 })?;
    if plan.expected_duration.timebase != Rational::new(1, CLOCK) || plan.expected_duration.value <= 0 {
        return Err(SmartCutError::StaleIndex { detail: "the plan's duration is not on the 90 kHz clock".into() });
    }
    Ok((base, plan.expected_duration.value))
}

/// Write the copy segments of a lossless MPEG-2 `plan` from `source` into `out` as a new
/// program or transport stream (`format`), with the audio of the same span.
///
/// `index` is the stream index the plan was built from ([`index_mpeg_video`]). A plan with any
/// other segment is refused before anything is written. On an error `out` may hold a partial
/// file, so write to a temporary file and keep it only on success.
pub fn execute_mpeg_copy<W: Write, S: ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &MpegCopySource<'_, S>,
    format: MuxFormat,
    out: W,
) -> Result<(W, MpegCopyReport), SmartCutError> {
    let (first, last) = video_range(plan, index, source, format)?;
    let stream = source.file.streams.get(source.video).ok_or(SmartCutError::NoTrack { track: source.video })?;
    let (base, duration) = video_span(plan, stream, first, last)?;
    let (audio, dropped) = audio_copies(source, format, base, base + duration)?;
    let mut codecs = vec![MuxCodec::Mpeg2Video];
    codecs.extend(audio.iter().map(|a| a.codec));
    // a multiplex rate well above the source's average keeps the clock ahead of the data
    let span = stream.pts_range().map_or(CLOCK, |(a, b)| (b - a).max(CLOCK));
    let rate = (source.bytes.len().saturating_mul(8 * 2 * CLOCK as u64) / span as u64).max(20_000_000);
    let mux_err = |e: cfa4_mpegts::MuxError| SmartCutError::Io { detail: e.to_string() };
    let mut m = Muxer::new(out, format, &codecs, rate).map_err(mux_err)?;
    let mut report = MpegCopyReport { samples: 0, payload_bytes: 0, duration: duration as u64, timescale: CLOCK as u32, audio: Vec::new(), dropped };
    for d in first..=last {
        let u = stream.units.get(d).ok_or_else(|| SmartCutError::StaleIndex { detail: format!("unit {d} is missing") })?;
        let data = source.file.read_unit(source.bytes, source.video, d).map_err(io)?;
        if data.len() as u64 != u64::from(u.size) {
            return Err(SmartCutError::StaleIndex { detail: format!("unit {d}: read {} bytes, expected {}", data.len(), u.size) });
        }
        let pts = u.pts.ok_or(SmartCutError::Timestamps { decode_index: d as u64 })? - base;
        let dts = u.dts.map(|t| t - base);
        m.write(0, &data, pts, dts, u.key).map_err(|_| SmartCutError::Timestamps { decode_index: d as u64 })?;
        report.samples += 1;
        report.payload_bytes = report.payload_bytes.saturating_add(data.len() as u64);
    }
    report.audio = write_audio(&mut m, source, &audio, base)?;
    let out = m.finish().map_err(mux_err)?;
    Ok((out, report))
}

/// Write the audio units of `audio` as muxer streams 1, 2, … (stream 0 is the video), on the
/// output clock that starts at `base` (90 kHz, source clock).
pub(crate) fn write_audio<W: Write, S: ByteSource + ?Sized>(
    m: &mut Muxer<W>,
    source: &MpegCopySource<'_, S>,
    audio: &[AudioCopy],
    base: i64,
) -> Result<Vec<MpegAudioReport>, SmartCutError> {
    let mut reports = Vec::with_capacity(audio.len());
    for (k, a) in audio.iter().enumerate() {
        let s = source.file.streams.get(a.stream).ok_or(SmartCutError::NoTrack { track: a.stream })?;
        let mut bytes = 0u64;
        for &(i, t) in &a.units {
            let data = source.file.read_unit(source.bytes, a.stream, i).map_err(io)?;
            bytes = bytes.saturating_add(data.len() as u64);
            m.write(k + 1, &data, t - base, None, true).map_err(|e| SmartCutError::Io { detail: format!("audio stream {}: {e}", a.stream) })?;
        }
        reports.push(MpegAudioReport {
            stream: a.stream,
            codec: s.codec.name(),
            units: a.units.len() as u64,
            payload_bytes: bytes,
            offset: a.units.first().map_or(0, |u| u.1 - base),
        });
    }
    Ok(reports)
}

/// The id the muxer gives output stream `k` of `codecs` (transport: PID 0x100 + k; program:
/// stream id and sub-stream id by kind).
pub(crate) fn output_id(format: MuxFormat, codecs: &[MuxCodec], k: usize) -> Option<StreamId> {
    if format != MuxFormat::Ps {
        return Some(StreamId::Pid(0x100 + k as u16));
    }
    let codec = *codecs.get(k)?;
    let nth = codecs.iter().take(k).filter(|c| std::mem::discriminant(*c) == std::mem::discriminant(&codec)).count() as u8;
    Some(match codec {
        MuxCodec::Mpeg2Video => StreamId::Ps { stream_id: 0xE0 + nth, sub_id: None },
        MuxCodec::MpegAudio | MuxCodec::AacAdts => StreamId::Ps { stream_id: 0xC0 + nth, sub_id: None },
        MuxCodec::Ac3 => StreamId::Ps { stream_id: 0xBD, sub_id: Some(0x80 + nth) },
        MuxCodec::LpcmDvd { .. } => StreamId::Ps { stream_id: 0xBD, sub_id: Some(0xA0 + nth) },
    })
}

/// Check the audio streams of `file` (an output of [`write_audio`]): the same frames, byte for
/// byte, with the first one at its source offset against the first picture. `find` maps muxer
/// stream `k` to its stream in `file`.
pub(crate) fn verify_audio<S: ByteSource + ?Sized, O: ByteSource + ?Sized>(
    file: &File,
    output: &O,
    source: &MpegCopySource<'_, S>,
    audio: &[AudioCopy],
    base: i64,
    find: &dyn Fn(usize) -> Result<usize, SmartCutError>,
) -> Result<Vec<MpegAudioReport>, SmartCutError> {
    let fail = |detail: String| SmartCutError::Verification { detail };
    let delay = Muxer::<Vec<u8>>::DELAY;
    let mut reports = Vec::with_capacity(audio.len());
    for (k, a) in audio.iter().enumerate() {
        let oa = find(k + 1)?;
        let out = &file.streams[oa];
        let written: Vec<u8> = (0..out.units.len()).map(|i| file.read_unit(output, oa, i)).collect::<Result<Vec<_>, _>>().map_err(io)?.concat();
        let mut copied = Vec::new();
        for &(i, _) in &a.units {
            copied.extend(source.file.read_unit(source.bytes, a.stream, i).map_err(io)?);
        }
        if written != copied {
            return Err(fail(format!("audio stream {}: the written audio differs from the source", a.stream)));
        }
        // the first frame's time against the first picture, as in the source
        let offset = a.units.first().map_or(0, |u| u.1 - base);
        if out.units.first().and_then(|u| u.pts) != Some(offset + delay) {
            return Err(fail(format!("audio stream {} is out of sync with the video", a.stream)));
        }
        reports.push(MpegAudioReport {
            stream: a.stream,
            codec: source.file.streams[a.stream].codec.name(),
            units: a.units.len() as u64,
            payload_bytes: written.len() as u64,
            offset,
        });
    }
    Ok(reports)
}

/// Read back a stream written by [`execute_mpeg_copy`] and check that it holds exactly the copied
/// source units: the same video pictures and audio frames, byte for byte, at the same times
/// against the first picture. Returns the same report as the copy.
pub fn verify_mpeg_copy<S: ByteSource + ?Sized, O: ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &MpegCopySource<'_, S>,
    format: MuxFormat,
    output: &O,
) -> Result<MpegCopyReport, SmartCutError> {
    let fail = |detail: String| SmartCutError::Verification { detail };
    let (first, last) = video_range(plan, index, source, format)?;
    let stream = source.file.streams.get(source.video).ok_or(SmartCutError::NoTrack { track: source.video })?;
    let (base, duration) = video_span(plan, stream, first, last)?;
    let (audio, dropped) = audio_copies(source, format, base, base + duration)?;
    let file = cfa4_mpegts::open(output).map_err(|e| fail(format!("the output does not open: {e}")))?;
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
    // the output clock: the first picture at the muxer's delay
    let ov = find(0)?;
    let out_video = &file.streams[ov];
    if out_video.units.len() != last - first + 1 {
        return Err(fail(format!("{} pictures written, {} copied", out_video.units.len(), last - first + 1)));
    }
    let mut report = MpegCopyReport { samples: 0, payload_bytes: 0, duration: duration as u64, timescale: CLOCK as u32, audio: Vec::new(), dropped };
    for (k, d) in (first..=last).enumerate() {
        let (src, out) = (&stream.units[d], &out_video.units[k]);
        let want = source.file.read_unit(source.bytes, source.video, d).map_err(io)?;
        let got = file.read_unit(output, ov, k).map_err(io)?;
        if got != want {
            return Err(fail(format!("picture {k} differs from source unit {d}")));
        }
        let shift = |t: Option<i64>| t.map(|t| t - base + delay);
        if out.pts != shift(src.pts) || out.dts.or(out.pts) != shift(src.dts.or(src.pts)) || out.key != src.key {
            return Err(fail(format!("picture {k}: timestamps or key flag changed")));
        }
        report.samples += 1;
        report.payload_bytes = report.payload_bytes.saturating_add(got.len() as u64);
    }
    report.audio = verify_audio(&file, output, source, &audio, base, &find)?;
    Ok(report)
}
