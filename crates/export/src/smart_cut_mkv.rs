//! Smart Cut into Matroska / WebM (R2-04 of `CFA4-SMART-CUT-RENDER-R2`).
//!
//! The video packets a lossless plan copies are written unchanged into a new Matroska (or WebM)
//! file, with the source's `CodecPrivate`, in decode order, on the source's timestamp scale. The
//! audio tracks come along without re-encoding:
//!
//! - **Where the audio starts:** AAC, AC-3, E-AC-3 and DTS start one packet before the frame that
//!   holds the first picture's time, because their transforms overlap. TrueHD starts at the last
//!   major sync at or before it, up to [`TRUEHD_MAX_RESTART_DISTANCE`] access units back. FLAC
//!   and PCM start at that frame.
//! - **Sync:** every block keeps its source time minus one common origin, the earliest copied
//!   timestamp. So the audio may start up to one access unit before the picture (owner decision 3
//!   of the R2 plan: about 106 ms for TrueHD), and the A/V offset is exactly the source's.
//! - **Checks:** AC-3, E-AC-3, DTS and TrueHD are indexed before anything is written. A torn
//!   frame, a configuration change or a copy that does not start at an entry point stops the
//!   export.
//! - **Dropped tracks:** other audio (Opus, Vorbis, MP3…), compressed or encrypted frames, and
//!   audio WebM cannot carry are named in [`MkvAudio::dropped`] with the reason.
//!
//! [`verify_mkv_copy`] demuxes the written file again and checks every block against the source.
//! The caller writes into a temporary file and keeps it only when that check passes (the common
//! atomic writer).

use std::io::{Seek, Write};

use cfa4_isobmff::CodecConfig;
use cfa4_matroska::{ByteSource, Codec as MkvCodec, MkvFile, MkvWriter, MuxOptions, Track, TrackKind, TrackSpec};
use cfa4_smart_edit::audio_index::{AudioCodec, index_audio};
use cfa4_smart_edit::av1_index::Av1AccessPoints;
use cfa4_smart_edit::capabilities::{Codec, Container};
use cfa4_smart_edit::media_index::StreamIndex;
use cfa4_smart_edit::plan::{ExportExecutionPlan, PLAN_VERSION};
use cfa4_smart_edit::timestamp::Rational;
use serde::{Deserialize, Serialize};

use crate::audio_copy::{AudioDrop, TRUEHD_MAX_RESTART_DISTANCE};
use crate::mkv_source::{MkvVideo, index_mkv_video};
use crate::smart_cut::{SmartCutError, check_access_point, check_av1_access_point, check_hevc_access_point, io};

/// The source of a Matroska copy: the opened file, its bytes and its indexed video track.
pub struct MkvCopySource<'a, S: ByteSource + ?Sized> {
    pub file: &'a MkvFile,
    pub bytes: &'a S,
    pub video: &'a MkvVideo,
}

/// How one audio track is copied.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MkvAudioCopy {
    /// Index into the source's [`MkvFile::tracks`].
    pub track: usize,
    pub track_number: u64,
    /// `aac`, `ac3`, `eac3`, `dts`, `truehd`, `flac` or `pcm`.
    pub codec: String,
    /// The source frames copied, in file order (with the pre-roll frame or from the major sync).
    pub first: usize,
    pub last: usize,
    /// First copied audio time minus the first presented picture's time, in nanoseconds:
    /// negative when the copy starts before the picture (pre-roll, TrueHD restart point).
    pub lead_ns: i64,
}

/// The audio of a Matroska cut.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MkvAudio {
    pub tracks: Vec<MkvAudioCopy>,
    pub dropped: Vec<AudioDrop>,
}

/// What a Matroska copy wrote.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MkvCopyReport {
    /// Video frames written.
    pub frames: u64,
    pub payload_bytes: u64,
    /// Audio blocks written, per copied audio track (in [`MkvAudio::tracks`] order).
    pub audio_frames: Vec<u64>,
    /// The common origin subtracted from every source time, in nanoseconds.
    pub origin_ns: i64,
}

/// The source video frames a lossless plan copies, checked against the index and the track.
pub(crate) struct VideoRange<'a> {
    pub track: &'a Track,
    pub first: usize,
    pub last: usize,
}

/// The output container must carry the codec: WebM only takes AV1 video (of the codecs Smart Cut
/// copies), and Opus / Vorbis audio.
fn output_doc_type(codec: Codec, output: Container) -> Option<&'static str> {
    match (output, codec) {
        (Container::Matroska, Codec::H264 | Codec::Hevc | Codec::Av1 | Codec::Mpeg2Video) => Some("matroska"),
        (Container::WebM, Codec::Av1) => Some("webm"),
        _ => None,
    }
}

fn video_range<'a, S: ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &MkvCopySource<'a, S>,
) -> Result<VideoRange<'a>, SmartCutError> {
    mkv_video_range(plan, index, source, output_doc_type(plan.codec, plan.output_container).is_some())
}

/// [`video_range`] for any output: `output_ok` says whether the executor writes the plan's
/// output container (Matroska / WebM here, MP4 / MOV for the cross-container copy).
pub(crate) fn mkv_video_range<'a, S: ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &MkvCopySource<'a, S>,
    output_ok: bool,
) -> Result<VideoRange<'a>, SmartCutError> {
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
    if !matches!(plan.source_container, Container::Matroska | Container::WebM) || !output_ok || index.codec != plan.codec || source.video.codec != plan.codec {
        return Err(unsupported);
    }
    let track = source.file.tracks.get(source.video.track).ok_or(SmartCutError::NoTrack { track: source.video.track })?;
    let id = u32::try_from(track.number).unwrap_or(u32::MAX);
    if index.stream_id != id {
        return Err(SmartCutError::WrongStream { expected: id, found: index.stream_id });
    }
    if index.frames.len() != track.samples.len() {
        return Err(SmartCutError::StaleIndex { detail: format!("{} indexed pictures, {} frames in track {id}", index.frames.len(), track.samples.len()) });
    }
    let tb = (i64::try_from(track.timebase.0), i64::try_from(track.timebase.1));
    if !matches!(tb, (Ok(n), Ok(d)) if index.timebase == Rational::new(n, d)) {
        return Err(SmartCutError::StaleIndex { detail: format!("the index timebase does not match track {id}") });
    }
    let mut range: Option<(u64, u64)> = None;
    for (i, s) in plan.segments.iter().enumerate() {
        if s.stream_id != id {
            return Err(SmartCutError::WrongStream { expected: id, found: s.stream_id });
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
    let as_index = |d: u64| usize::try_from(d).ok().filter(|&d| d < track.samples.len());
    let (Some(first), Some(last)) = (as_index(first), as_index(last)) else {
        return Err(SmartCutError::StaleIndex { detail: format!("packets {first}–{last} are past the end of track {id}") });
    };
    for d in first..=last {
        let (Some(sample), Some(frame)) = (track.samples.get(d), index.frames.get(d)) else {
            return Err(SmartCutError::StaleIndex { detail: format!("packet {d} is missing") });
        };
        let size = track.sample_size(d).unwrap_or(u64::MAX);
        if size != u64::from(frame.size) || frame.pts != Some(sample.pts) || frame.keyframe != sample.keyframe {
            return Err(SmartCutError::StaleIndex { detail: format!("packet {d} is not the one that was indexed (size, time or key flag)") });
        }
    }
    Ok(VideoRange { track, first, last })
}

/// The audio codec of a Matroska track that is copied, with its entry-point rules.
fn audio_codec(t: &Track) -> Result<(&'static str, Option<AudioCodec>, usize), String> {
    // (name, bitstream codec indexed before writing, pre-roll frames)
    Ok(match &t.codec {
        MkvCodec::Aac { .. } => ("aac", None, 1),
        MkvCodec::Ac3 => ("ac3", Some(AudioCodec::Ac3), 1),
        MkvCodec::Eac3 => ("eac3", Some(AudioCodec::Eac3), 1),
        MkvCodec::Dts => ("dts", Some(AudioCodec::Dts), 1),
        MkvCodec::TrueHd => ("truehd", Some(AudioCodec::TrueHd), 0),
        MkvCodec::Flac { .. } => ("flac", None, 0),
        MkvCodec::Pcm { .. } => ("pcm", None, 0),
        _ => return Err(format!("{} audio is not copied by Smart Cut yet", t.codec_id)),
    })
}

/// Plan the audio of a cut whose presented video runs `[start_ns, end_ns)` (source times).
/// Tracks that cannot be copied are named in [`MkvAudio::dropped`]; reading or indexing errors of
/// a copied track stop the export.
pub fn plan_mkv_audio<S: ByteSource + ?Sized>(file: &MkvFile, bytes: &S, output: Container, start_ns: i64, end_ns: i64) -> Result<MkvAudio, SmartCutError> {
    let mut plan = MkvAudio::default();
    for (i, t) in file.tracks.iter().enumerate().filter(|(_, t)| t.kind == TrackKind::Audio) {
        let id = u32::try_from(t.number).unwrap_or(u32::MAX);
        let drop = |reason: String| AudioDrop { track_id: id, reason };
        let (name, bitstream, pre_roll) = match audio_codec(t) {
            Ok(c) => c,
            Err(reason) => {
                plan.dropped.push(drop(reason));
                continue;
            }
        };
        if output == Container::WebM {
            plan.dropped.push(drop(format!("WebM carries only Opus and Vorbis audio, not {}", t.codec_id)));
            continue;
        }
        // into MP4 / MOV the track needs a sample entry built from the Matroska track (R2-05)
        if matches!(output, Container::Mp4 | Container::Mov)
            && let Err(reason) = crate::smart_cut_cross::mp4_audio_entry(t)
        {
            plan.dropped.push(drop(reason));
            continue;
        }
        if !t.frames_readable() {
            plan.dropped.push(drop("the audio frames are compressed or encrypted in the file".into()));
            continue;
        }
        let ns: Vec<i64> = t.samples.iter().map(|s| t.ticks_to_ns(s.pts)).collect();
        // times are rounded to the timestamp scale, so short frames (TrueHD: 0.83 ms) may share one
        if ns.windows(2).any(|w| w[1] < w[0]) {
            plan.dropped.push(drop("the audio frame times go backwards".into()));
            continue;
        }
        // the frame that holds the first picture's time (or the first frame after it)
        let Some(s) = ns.iter().rposition(|&t| t <= start_ns).or_else(|| ns.iter().position(|&t| t < end_ns)) else {
            plan.dropped.push(drop("the track has no audio in the cut".into()));
            continue;
        };
        let Some(last) = ns.iter().rposition(|&t| t < end_ns).filter(|&l| l >= s) else {
            plan.dropped.push(drop("the track has no audio in the cut".into()));
            continue;
        };
        let first = match bitstream {
            Some(AudioCodec::TrueHd) => {
                let from = s.saturating_sub(TRUEHD_MAX_RESTART_DISTANCE);
                let mut packets = Vec::with_capacity(s - from + 1);
                for k in from..=s {
                    packets.push(file.read_sample(bytes, i, k).map_err(io)?);
                }
                // index from the first major sync of the window
                let index = (0..packets.len()).find_map(|k| index_audio(AudioCodec::TrueHd, packets[k..].iter().map(Vec::as_slice)).ok().map(|ix| (k, ix)));
                match index.and_then(|(k, ix)| ix.entry_at_or_before(s - from - k).map(|e| from + k + e)) {
                    Some(f) => f,
                    None => {
                        plan.dropped.push(drop(format!("no TrueHD major sync within {TRUEHD_MAX_RESTART_DISTANCE} access units before the cut")));
                        continue;
                    }
                }
            }
            _ => s.saturating_sub(pre_roll),
        };
        if let Some(codec) = bitstream {
            let mut packets = Vec::with_capacity(last - first + 1);
            for k in first..=last {
                packets.push(file.read_sample(bytes, i, k).map_err(io)?);
            }
            check_bitstream(codec, id, &packets)?;
        }
        plan.tracks.push(MkvAudioCopy { track: i, track_number: t.number, codec: name.into(), first, last, lead_ns: ns[first] - start_ns });
    }
    Ok(plan)
}

/// The copied packets of a bitstream audio track must index: every frame whole, the first one an
/// entry point, one configuration.
fn check_bitstream(codec: AudioCodec, track_id: u32, packets: &[Vec<u8>]) -> Result<(), SmartCutError> {
    let fail = |detail: String| SmartCutError::Incompatible { detail: format!("{} audio track {track_id}: {detail}", codec.name()) };
    let ix = index_audio(codec, packets.iter().map(Vec::as_slice)).map_err(|e| fail(e.to_string()))?;
    if ix.units.first().is_some_and(|u| !u.entry) {
        return Err(fail("the copy does not start at an entry point".into()));
    }
    if !ix.one_config(0, ix.units.len().saturating_sub(1)) {
        return Err(fail("the audio configuration changes inside the cut".into()));
    }
    Ok(())
}

/// The output track description of a source track: its codec, `CodecPrivate` and properties.
fn spec(t: &Track) -> TrackSpec {
    let mut s = TrackSpec::new(t.kind, t.codec_id.clone());
    s.codec_private = t.codec_private.clone();
    s.default_duration_ns = t.default_duration_ns;
    s.language = Some(t.language.clone());
    s.name = (!t.name.is_empty()).then(|| t.name.clone());
    s.video_size = t.video.as_ref().map(|v| (v.pixel_width, v.pixel_height));
    s.audio = t.audio.as_ref().map(|a| (a.sampling_frequency, a.channels, a.bit_depth));
    s.codec_delay_ns = t.codec_delay_ns;
    s.seek_pre_roll_ns = t.seek_pre_roll_ns;
    s
}

/// The first presented picture's time and the end of the presented video (source nanoseconds).
fn video_span(r: &VideoRange<'_>) -> (i64, i64) {
    let samples = &r.track.samples[r.first..=r.last];
    let start = samples.iter().map(|s| r.track.ticks_to_ns(s.pts)).min().unwrap_or(0);
    let end = samples.iter().map(|s| r.track.ticks_to_ns(s.pts) + r.track.ticks_to_ns(i64::try_from(s.duration).unwrap_or(0))).max().unwrap_or(start);
    (start, end)
}

/// The presented video interval of a lossless `plan` (source nanoseconds), for [`plan_mkv_audio`].
pub fn mkv_cut_interval<S: ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &MkvCopySource<'_, S>,
) -> Result<(i64, i64), SmartCutError> {
    // a Matroska / WebM output, or MP4 / MOV through the cross-container copy (R2-05)
    let output_ok = output_doc_type(plan.codec, plan.output_container).is_some() || crate::smart_cut_cross::mp4_video_output(plan);
    let r = mkv_video_range(plan, index, source, output_ok)?;
    let (start, end) = video_span(&r);
    // a track without frame durations: the plan's duration ends the cut
    let d = plan.expected_duration;
    let planned = (d.timebase.den != 0).then(|| i128::from(d.value) * i128::from(d.timebase.num) * 1_000_000_000 / i128::from(d.timebase.den));
    Ok((start, planned.and_then(|d| i64::try_from(d).ok()).map_or(end, |d| start + d)))
}

/// Write the copy segments of a lossless `plan` and the planned `audio` from `source` into `out`
/// as a new Matroska / WebM file. On an error `out` may hold a partial file: write to a temporary
/// file, check it with [`verify_mkv_copy`] and keep it only then.
pub fn execute_mkv_copy<W: Write + Seek, S: ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &MkvCopySource<'_, S>,
    audio: &MkvAudio,
    out: W,
) -> Result<(W, MkvCopyReport), SmartCutError> {
    let r = video_range(plan, index, source)?;
    let doc_type = output_doc_type(plan.codec, plan.output_container).ok_or(SmartCutError::NotLossless)?;
    let (start, _) = video_span(&r);
    let mut origin = start;
    for a in &audio.tracks {
        let t = source.file.tracks.get(a.track).ok_or(SmartCutError::NoTrack { track: a.track })?;
        let s = t.samples.get(a.first).ok_or(SmartCutError::NoTrack { track: a.track })?;
        origin = origin.min(t.ticks_to_ns(s.pts));
    }
    let mut specs = vec![spec(r.track)];
    specs.extend(audio.tracks.iter().map(|a| spec(&source.file.tracks[a.track])));
    let opts = MuxOptions { doc_type: doc_type.into(), timestamp_scale: source.file.info.timestamp_scale, ..MuxOptions::default() };
    let mut w = MkvWriter::new(out, specs, opts).map_err(io)?;
    let mut report = MkvCopyReport { frames: 0, payload_bytes: 0, audio_frames: vec![0; audio.tracks.len()], origin_ns: origin };

    // every block, as (output time, track, source track, frame); the writer clusters them in time
    // order, the video in decode order with its key frames first in their clusters
    let mut blocks: Vec<(i64, usize, usize, usize)> = Vec::new();
    for d in r.first..=r.last {
        blocks.push((r.track.ticks_to_ns(r.track.samples[d].pts) - origin, 0, source.video.track, d));
    }
    for (k, a) in audio.tracks.iter().enumerate() {
        let t = &source.file.tracks[a.track];
        for f in a.first..=a.last {
            blocks.push((t.ticks_to_ns(t.samples[f].pts) - origin, k + 1, a.track, f));
        }
    }
    // interleave by decode position: the video's decode order is kept, audio goes by time; a
    // video frame is placed at the smallest presentation time from it onwards, so B-frames never
    // move ahead of their references
    let mut floor = i64::MAX;
    for b in blocks.iter_mut().rev().filter(|b| b.1 == 0) {
        floor = floor.min(b.0);
        b.0 = floor;
    }
    let mut order: Vec<(i64, usize, usize, usize, usize)> = blocks.iter().enumerate().map(|(n, b)| (b.0, if b.1 == 0 { 0 } else { 1 }, n, b.2, b.3)).collect();
    order.sort();
    let mut av1 = match &source.video.config {
        CodecConfig::Av1(c) => Some(Av1AccessPoints::new(&c.config_obus)),
        _ => None,
    };
    for (_, kind, n, track, frame) in order {
        let out_track = blocks[n].1;
        let t = &source.file.tracks[track];
        let s = &t.samples[frame];
        let data = source.file.read_sample(source.bytes, track, frame).map_err(io)?;
        let pts = t.ticks_to_ns(s.pts) - origin;
        if kind == 0 {
            match (&source.video.config, index.frames.get(frame)) {
                (CodecConfig::Avc(avc), Some(f)) => check_access_point(frame, &data, avc.length_size, f)?,
                (CodecConfig::Hevc(h), Some(f)) => check_hevc_access_point(frame, &data, h.length_size, f)?,
                (CodecConfig::Av1(_), Some(f)) => {
                    if let Some(av1) = av1.as_mut() {
                        check_av1_access_point(frame, av1.next(&data), f)?;
                    }
                }
                _ => {}
            }
            report.frames += 1;
            report.payload_bytes = report.payload_bytes.saturating_add(data.len() as u64);
        } else {
            report.audio_frames[out_track - 1] += 1;
        }
        // keep each frame's own duration where it differs from the track default
        let duration = (s.duration > 0).then(|| u64::try_from(t.ticks_to_ns(i64::try_from(s.duration).unwrap_or(0))).unwrap_or(0));
        w.write_frame(out_track, pts, s.keyframe, &data, duration).map_err(io)?;
    }
    let out = w.finish().map_err(io)?;
    Ok((out, report))
}

/// Demux a file written by [`execute_mkv_copy`] and check it against the source: the same tracks
/// (codec and `CodecPrivate`), and every block with the source payload, key flag and time minus
/// the report's origin.
pub fn verify_mkv_copy<S: ByteSource + ?Sized, O: ByteSource>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &MkvCopySource<'_, S>,
    audio: &MkvAudio,
    report: &MkvCopyReport,
    output: O,
) -> Result<(), SmartCutError> {
    let fail = |detail: String| SmartCutError::Verification { detail };
    let r = video_range(plan, index, source)?;
    let file = cfa4_matroska::open(&output).map_err(|e| fail(format!("the output does not open: {e}")))?;
    let mut want: Vec<(usize, usize, usize)> = vec![(source.video.track, r.first, r.last)];
    want.extend(audio.tracks.iter().map(|a| (a.track, a.first, a.last)));
    if file.tracks.len() != want.len() {
        return Err(fail(format!("the output has {} tracks, expected {}", file.tracks.len(), want.len())));
    }
    if r.last - r.first + 1 != usize::try_from(plan.copied_frames()).unwrap_or(0) {
        return Err(fail(format!("{} video frames copied, {} planned", r.last - r.first + 1, plan.copied_frames())));
    }
    for (k, (&(src_track, first, last), out)) in want.iter().zip(&file.tracks).enumerate() {
        let src = &source.file.tracks[src_track];
        if out.codec_id != src.codec_id || out.codec_private != src.codec_private {
            return Err(fail(format!("track {k}: the codec or its CodecPrivate changed")));
        }
        if out.samples.len() != last - first + 1 {
            return Err(fail(format!("track {k}: {} frames written, {} copied", out.samples.len(), last - first + 1)));
        }
        for (j, o) in out.samples.iter().enumerate() {
            let d = first + j;
            let s = &src.samples[d];
            let shift = out.ticks_to_ns(o.pts) - (src.ticks_to_ns(s.pts) - report.origin_ns);
            // the output keeps the source timestamp scale: the times differ by rounding at most
            if o.keyframe != s.keyframe || shift.abs() > i64::try_from(source.file.info.timestamp_scale / 2).unwrap_or(0) {
                return Err(fail(format!("track {k} frame {j} (source frame {d}) has another time or key flag")));
            }
            let a = file.read_sample(&output, k, j).map_err(|e| fail(format!("track {k} frame {j}: {e}")))?;
            let b = source.file.read_sample(source.bytes, src_track, d).map_err(io)?;
            if a != b {
                return Err(fail(format!("track {k} frame {j} (source frame {d}) payload differs")));
            }
        }
    }
    Ok(())
}

/// Open and index a Matroska source in one step (for callers that hold only the bytes).
pub fn open_mkv_source<S: ByteSource + ?Sized>(bytes: &S) -> Result<(MkvFile, MkvVideo), SmartCutError> {
    let file = cfa4_matroska::open(bytes).map_err(io)?;
    let video = index_mkv_video(&file, bytes).map_err(|e| SmartCutError::Io { detail: e.to_string() })?;
    Ok((file, video))
}
