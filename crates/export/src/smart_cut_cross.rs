//! Cross-container Smart Cut (R2-05 of `CFA4-SMART-CUT-RENDER-R2`): a lossless plan from a
//! Matroska source into MP4 / MOV, and from an MP4 / MOV source into Matroska / WebM.
//!
//! The video payloads are copied unchanged: H.264 and HEVC are length-prefixed and AV1 is OBU
//! temporal units in both container families, so only the description around them changes. Every
//! such change is listed in the [`CrossReport::log`]:
//!
//! - **Codec configuration:** the Matroska `CodecPrivate` of H.264, HEVC and AV1 *is* the
//!   ISOBMFF configuration record (`avcC`, `hvcC`, `av1C`), so the sample entry is built from it,
//!   and the other way round the record is written as `CodecPrivate`. Verification compares the
//!   parsed records, field for field.
//! - **Timing:** Matroska stores presentation times only. Into MP4 the decode times are rebuilt
//!   from them: decode time *k* is the *k*-th smallest presentation time, moved back by the
//!   smallest shift that keeps every decode time at or before its presentation time, and an edit
//!   list hides that shift. The pictures keep their source presentation times exactly (the MP4
//!   timescale is the Matroska timebase). Into Matroska the times are written on a timestamp
//!   scale that holds the MP4 timescale exactly where one exists (1 ms otherwise); the largest
//!   rounding is reported.
//! - **Audio:** into MP4, AAC, FLAC and PCM tracks of the Matroska source get a sample entry from
//!   their `CodecPrivate` and audio element (the bitstream codecs follow in later steps and are
//!   named as dropped until then). Into Matroska, every track the MP4 Smart Cut copies (AAC, PCM,
//!   AC-3, E-AC-3, DTS, TrueHD, see [`crate::audio_copy`]) is written with its codec ID and
//!   `CodecPrivate`. As in the same-container copies, the audio keeps its source offset against
//!   the picture and may start one entry unit before it.

use std::io::{Cursor, Seek, Write};

use cfa4_isobmff::{
    AacConfig, Av1Config, AvcConfig, CodecConfig, HevcConfig, Mp4File, Mp4Writer, PcmConfig, SampleEntry, TrackConfig, TrackKind, WriteSample, WriterOptions,
};
use cfa4_matroska::{Codec as MkvCodec, MkvFile, MkvWriter, MuxOptions, TrackSpec};
use cfa4_smart_edit::av1_index::Av1AccessPoints;
use cfa4_smart_edit::capabilities::{Codec, Container};
use cfa4_smart_edit::media_index::StreamIndex;
use cfa4_smart_edit::plan::ExportExecutionPlan;
use serde::{Deserialize, Serialize};

use crate::audio_copy::{AudioPlan, AudioTrackInput, CutInterval, copied_tracks, mux_audio_tracks, verify_audio_tracks};
use crate::smart_cut::{Mp4CopySource, SmartCutError, brand, check_access_point, check_av1_access_point, check_hevc_access_point, io, mp4_copy_range};
use crate::smart_cut_mkv::{MkvAudio, MkvCopySource, mkv_video_range};

/// What a cross-container copy wrote, and every representation change it made.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossReport {
    /// Video frames written.
    pub frames: u64,
    pub payload_bytes: u64,
    /// Copied audio tracks: source track id, codec and frames written.
    pub audio: Vec<(u32, String, u64)>,
    /// The representation changes, one line each (the export log of a Compatible copy).
    pub log: Vec<String>,
    /// The largest difference between a written time and its exact source time, in nanoseconds.
    pub max_rounding_ns: i64,
}

/// The plan's output is MP4 / MOV and its codec is one the cross-container copy carries there.
pub fn mp4_video_output(plan: &ExportExecutionPlan) -> bool {
    brand(plan.output_container).is_some() && matches!(plan.codec, Codec::H264 | Codec::Hevc | Codec::Av1)
}

/// The plan's output is Matroska (H.264, HEVC, AV1) or WebM (AV1).
fn mkv_output(plan: &ExportExecutionPlan) -> Option<&'static str> {
    match (plan.output_container, plan.codec) {
        (Container::Matroska, Codec::H264 | Codec::Hevc | Codec::Av1) => Some("matroska"),
        (Container::WebM, Codec::Av1) => Some("webm"),
        _ => None,
    }
}

fn rescale(v: i128, num: i128, den: i128) -> i128 {
    if den == 0 {
        return 0;
    }
    let p = v * num;
    let half = den / 2;
    if p >= 0 { (p + half) / den } else { (p - half) / den }
}

// --- Matroska → MP4 / MOV -----------------------------------------------------------------

/// The MP4 sample entry and timescale of a Matroska audio track, or why it has none yet.
pub fn mp4_audio_entry(t: &cfa4_matroska::Track) -> Result<(SampleEntry, u32), String> {
    let a = t.audio.as_ref().ok_or_else(|| format!("track {} has no audio element", t.number))?;
    let channels = u32::try_from(a.channels).unwrap_or(0);
    let rate = a.sampling_frequency;
    let whole_rate = (rate.fract() == 0.0 && rate >= 1.0 && rate <= f64::from(u32::MAX)).then_some(rate as u32);
    match &t.codec {
        MkvCodec::Aac { asc } => {
            let cfg = AacConfig::from_asc(asc.clone());
            let rate = if cfg.sample_rate > 0 { cfg.sample_rate } else { whole_rate.ok_or("the AAC sample rate is unknown")? };
            Ok((SampleEntry::aac(asc.clone(), channels, rate), rate))
        }
        MkvCodec::Flac { private } => {
            // `fLaC`, then STREAMINFO (4-byte block header, 34-byte body) first
            let info: &[u8; 34] =
                private.get(8..42).and_then(|b| b.try_into().ok()).filter(|_| private.starts_with(b"fLaC")).ok_or("the FLAC CodecPrivate has no STREAMINFO")?;
            let rate = (u32::from(info[10]) << 12) | (u32::from(info[11]) << 4) | (u32::from(info[12]) >> 4);
            if rate == 0 {
                return Err("the FLAC sample rate is unknown".into());
            }
            Ok((SampleEntry::flac(info), rate))
        }
        MkvCodec::Pcm { float, big_endian, bits } => {
            let bits = if *bits > 0 { *bits } else { a.bit_depth.and_then(|b| u16::try_from(b).ok()).unwrap_or(0) };
            let rate = whole_rate.ok_or("the PCM sample rate is not a whole number")?;
            if bits == 0 || bits % 8 != 0 || channels == 0 {
                return Err(format!("PCM with {bits} bits and {channels} channels is not copied"));
            }
            let cfg = PcmConfig { bits, float: *float, big_endian: *big_endian, signed: !*float && bits > 8, channels, sample_rate: f64::from(rate) };
            Ok((SampleEntry::pcm(cfg), rate))
        }
        MkvCodec::Ac3 | MkvCodec::Eac3 | MkvCodec::Dts | MkvCodec::TrueHd => {
            Err(format!("{} audio is not copied from Matroska into MP4 yet (its sample entry is not built)", t.codec_id))
        }
        _ => Err(format!("{} audio is not copied by Smart Cut yet", t.codec_id)),
    }
}

/// Samples per channel of a FLAC frame, from its header.
fn flac_block_size(frame: &[u8]) -> Option<u32> {
    if frame.len() < 5 || frame[0] != 0xFF || frame[1] & 0xFE != 0xF8 {
        return None;
    }
    let code = frame[2] >> 4;
    // the coded frame / sample number: UTF-8 style, 1 to 7 bytes, after the 4 fixed bytes
    let lead = frame[4];
    let n = match lead.leading_ones() {
        0 => 1,
        l @ 2..=7 => l as usize,
        _ => return None,
    };
    let at = 4 + n;
    Some(match code {
        1 => 192,
        2..=5 => 576 << (code - 2),
        6 => u32::from(*frame.get(at)?) + 1,
        7 => (u32::from(*frame.get(at)?) << 8 | u32::from(*frame.get(at + 1)?)) + 1,
        8..=15 => 256 << (code - 8),
        _ => return None,
    })
}

/// Samples per channel of one Matroska audio frame, for its MP4 sample duration.
fn frame_samples(entry: &SampleEntry, data: &[u8]) -> Option<u32> {
    match &entry.codec {
        CodecConfig::Aac(_) => Some(1024),
        CodecConfig::Flac(_) => flac_block_size(data),
        CodecConfig::Pcm(p) => {
            let fb = p.bytes_per_frame() as usize;
            (fb > 0 && data.len().is_multiple_of(fb)).then(|| u32::try_from(data.len() / fb).ok()).flatten()
        }
        _ => None,
    }
}

/// The audio tracks of a Matroska cut as MP4 tracks: payloads, durations and their place against
/// the picture. `video_ns` is the presented video length.
fn mkv_audio_inputs<S: cfa4_matroska::ByteSource + ?Sized>(
    file: &MkvFile,
    bytes: &S,
    audio: &MkvAudio,
    video_ns: i64,
) -> Result<Vec<AudioTrackInput>, SmartCutError> {
    let mut out = Vec::new();
    for a in &audio.tracks {
        let t = file.tracks.get(a.track).ok_or(SmartCutError::NoTrack { track: a.track })?;
        let id = u32::try_from(t.number).unwrap_or(u32::MAX);
        let (entry, rate) = mp4_audio_entry(t).map_err(|detail| SmartCutError::Incompatible { detail: format!("audio track {id}: {detail}") })?;
        let mut samples = Vec::with_capacity(a.last - a.first + 1);
        for f in a.first..=a.last {
            let data = file.read_sample(bytes, a.track, f).map_err(io)?;
            let n = frame_samples(&entry, &data)
                .ok_or_else(|| SmartCutError::Incompatible { detail: format!("audio track {id}: frame {f} has no known length") })?;
            samples.push((data, n));
        }
        let total: u64 = samples.iter().map(|s| u64::from(s.1)).sum();
        let r = i128::from(rate);
        let at = |ns: i64| i64::try_from(rescale(i128::from(ns), r, 1_000_000_000)).unwrap_or(0);
        // before the picture: hidden by the edit list; after it: silence first
        let (lead_in, media_time) = if a.lead_ns < 0 { (0u64, at(-a.lead_ns)) } else { (u64::try_from(at(a.lead_ns)).unwrap_or(0), 0) };
        let video = u64::try_from(at(video_ns)).unwrap_or(0);
        let presented = total.saturating_sub(u64::try_from(media_time).unwrap_or(0)).min(video.saturating_sub(lead_in));
        out.push(AudioTrackInput {
            track_id: id,
            codec: a.codec.clone(),
            entry,
            timescale: rate,
            language: t.language.clone(),
            lead_in,
            media_time,
            presented,
            samples,
            pcm: false,
            first_at: (i128::from(a.lead_ns), 1_000_000_000),
        });
    }
    Ok(out)
}

/// The rebuilt MP4 timing of the copied Matroska pictures.
struct Mp4Timing {
    timescale: u32,
    /// Composition offsets and durations, in decode order.
    offsets: Vec<i32>,
    durations: Vec<u32>,
    /// Media time of the first presented picture (the edit list start).
    shift: i64,
    /// Presented duration (the plan's), in the timescale.
    presented: u64,
}

fn mp4_timing(track: &cfa4_matroska::Track, first: usize, last: usize, plan: &ExportExecutionPlan) -> Result<Mp4Timing, SmartCutError> {
    let (num, den) = track.timebase;
    let timescale = u32::try_from(den).ok().filter(|_| num == 1).ok_or(SmartCutError::Timestamps { decode_index: first as u64 })?;
    let pts: Vec<i64> = track.samples[first..=last].iter().map(|s| s.pts).collect();
    let mut sorted = pts.clone();
    sorted.sort_unstable();
    if sorted.windows(2).any(|w| w[0] == w[1]) {
        // two pictures at one time: the Matroska timestamp scale is too coarse for the frame rate
        return Err(SmartCutError::Timestamps { decode_index: first as u64 });
    }
    let shift = sorted.iter().zip(&pts).map(|(s, p)| s - p).max().unwrap_or(0).max(0);
    let base = sorted[0] - shift;
    let dts: Vec<i64> = sorted.iter().map(|s| s - shift - base).collect();
    let mut offsets = Vec::with_capacity(pts.len());
    for (k, (p, d)) in pts.iter().zip(&dts).enumerate() {
        offsets.push(i32::try_from(p - base - d).map_err(|_| SmartCutError::Timestamps { decode_index: (first + k) as u64 })?);
    }
    // the plan's duration in the timescale (the index runs on the track timebase)
    let d = plan.expected_duration;
    let presented = rescale(i128::from(d.value) * i128::from(d.timebase.num), i128::from(timescale), i128::from(d.timebase.den));
    let presented = u64::try_from(presented).map_err(|_| SmartCutError::Timestamps { decode_index: last as u64 })?;
    let mut durations = Vec::with_capacity(dts.len());
    for k in 0..dts.len() {
        let next = match dts.get(k + 1) {
            Some(n) => *n,
            // the last picture lasts until the presentation ends
            None => i64::try_from(presented).unwrap_or(0) + shift,
        };
        durations.push(u32::try_from(next - dts[k]).ok().filter(|&n| n > 0).ok_or(SmartCutError::Timestamps { decode_index: (first + k) as u64 })?);
    }
    Ok(Mp4Timing { timescale, offsets, durations, shift, presented })
}

/// The MP4 sample entry of the copied Matroska video.
fn mp4_video_entry(config: &CodecConfig, track: &cfa4_matroska::Track) -> Result<SampleEntry, SmartCutError> {
    let v = track.video.as_ref();
    let size = |x: Option<u32>| u16::try_from(x.unwrap_or(0)).unwrap_or(0);
    let (w, h) = (size(v.map(|v| v.pixel_width)), size(v.map(|v| v.pixel_height)));
    let mut entry = match config {
        CodecConfig::Avc(c) => SampleEntry::avc(c.clone(), w, h),
        CodecConfig::Hevc(c) => SampleEntry::hevc(c.clone(), w, h),
        CodecConfig::Av1(c) => SampleEntry::av1(c.clone(), w, h),
        _ => return Err(SmartCutError::Unsupported { codec: Codec::Mpeg2Video, source: Container::Matroska, output: Container::Mp4 }),
    };
    if let (Some(vp), Some(info)) = (entry.video.as_mut(), v) {
        let par = info.pixel_aspect();
        if par != (1, 1) {
            vp.pixel_aspect = Some(par);
        }
    }
    Ok(entry)
}

/// Write the copy segments of a lossless `plan` and the planned `audio` from a Matroska `source`
/// into a new MP4 / MOV file. Write to a temporary file, check it with [`verify_mkv_to_mp4`] and
/// keep it only then.
pub fn execute_mkv_to_mp4<S: cfa4_matroska::ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &MkvCopySource<'_, S>,
    audio: &MkvAudio,
) -> Result<(Vec<u8>, CrossReport), SmartCutError> {
    let (video_only, tracks, mut report) = mkv_to_mp4_parts(plan, index, source, audio)?;
    let brand = brand(plan.output_container).ok_or(SmartCutError::NotLossless)?;
    let r = mkv_video_range(plan, index, source, true)?;
    let timing = mp4_timing(r.track, r.first, r.last, plan)?;
    let out = if tracks.is_empty() { video_only } else { mux_audio_tracks(&video_only, &tracks, (timing.presented, timing.timescale), brand)? };
    report.audio = tracks.iter().map(|t| (t.track_id, t.codec.clone(), t.samples.len() as u64)).collect();
    Ok((out, report))
}

/// The video-only MP4 file and the audio tracks to add to it.
fn mkv_to_mp4_parts<S: cfa4_matroska::ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &MkvCopySource<'_, S>,
    audio: &MkvAudio,
) -> Result<(Vec<u8>, Vec<AudioTrackInput>, CrossReport), SmartCutError> {
    let r = mkv_video_range(plan, index, source, mp4_video_output(plan))?;
    let brand = brand(plan.output_container).ok_or(SmartCutError::NotLossless)?;
    let timing = mp4_timing(r.track, r.first, r.last, plan)?;
    let entry = mp4_video_entry(&source.video.config, r.track)?;
    let mut report = CrossReport::default();
    report.log.push(format!("sample entry {} written from the Matroska CodecPrivate of {}", String::from_utf8_lossy(&entry.format.0), r.track.codec_id));
    report.log.push(format!(
        "decode times rebuilt from the Matroska presentation times on a 1/{} timescale (reorder delay {} ticks, hidden by the edit list)",
        timing.timescale, timing.shift
    ));
    if r.track.video.as_ref().is_some_and(|v| v.colour.is_some()) {
        report.log.push("the Matroska Colour element is not written as colr / mdcv / clli boxes; the bitstream's own colour signalling is unchanged".into());
    }
    let mut w = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(brand)).map_err(io)?;
    let mut cfg = TrackConfig::new(entry, timing.timescale);
    cfg.language = r.track.language.clone();
    cfg.media_start = (timing.shift > 0).then_some(timing.shift);
    let t = w.add_track(cfg).map_err(io)?;
    let mut av1 = match &source.video.config {
        CodecConfig::Av1(c) => Some(Av1AccessPoints::new(&c.config_obus)),
        _ => None,
    };
    for k in 0..=(r.last - r.first) {
        let d = r.first + k;
        let s = &r.track.samples[d];
        let data = source.file.read_sample(source.bytes, source.video.track, d).map_err(io)?;
        check_video_access_point(&source.video.config, index, d, &data, av1.as_mut())?;
        let ws = WriteSample { data: &data, duration: timing.durations[k], composition_offset: timing.offsets[k], is_sync: s.keyframe };
        w.write_sample(t, ws).map_err(io)?;
        report.frames += 1;
        report.payload_bytes = report.payload_bytes.saturating_add(data.len() as u64);
    }
    let video_only = w.finish().map_err(io)?.into_inner();
    let video_ns = i64::try_from(rescale(i128::from(timing.presented), 1_000_000_000, i128::from(timing.timescale))).unwrap_or(i64::MAX);
    let tracks = mkv_audio_inputs(source.file, source.bytes, audio, video_ns)?;
    for t in &tracks {
        report.log.push(format!(
            "audio track {}: sample entry {} written from the Matroska {} track",
            t.track_id,
            String::from_utf8_lossy(&t.entry.format.0),
            t.codec
        ));
    }
    // the audio offsets are Matroska times, rounded to its timestamp scale
    report.max_rounding_ns = i64::try_from(source.file.info.timestamp_scale / 2).unwrap_or(0);
    Ok((video_only, tracks, report))
}

fn check_video_access_point(config: &CodecConfig, index: &StreamIndex, d: usize, data: &[u8], av1: Option<&mut Av1AccessPoints>) -> Result<(), SmartCutError> {
    match (config, index.frames.get(d)) {
        (CodecConfig::Avc(avc), Some(f)) => check_access_point(d, data, avc.length_size, f),
        (CodecConfig::Hevc(h), Some(f)) => check_hevc_access_point(d, data, h.length_size, f),
        (CodecConfig::Av1(_), Some(f)) => match av1 {
            Some(av1) => check_av1_access_point(d, av1.next(data), f),
            None => Ok(()),
        },
        _ => Ok(()),
    }
}

/// Read back a file written by [`execute_mkv_to_mp4`]: the video track holds every copied frame
/// with its source payload, key flag and presentation time (exactly, on the same timebase), its
/// configuration record is the source `CodecPrivate`'s, and every audio track holds its payloads
/// at the source offset against the picture.
pub fn verify_mkv_to_mp4<S: cfa4_matroska::ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &MkvCopySource<'_, S>,
    audio: &MkvAudio,
    output: &[u8],
) -> Result<(), SmartCutError> {
    let fail = |detail: String| SmartCutError::Verification { detail };
    let (video_only, tracks, _) = mkv_to_mp4_parts(plan, index, source, audio)?;
    let r = mkv_video_range(plan, index, source, true)?;
    let file = cfa4_isobmff::open(output).map_err(|e| fail(format!("the output does not open: {e}")))?;
    let v = file.tracks.iter().position(|t| t.kind == TrackKind::Video).ok_or_else(|| fail("the output has no video track".into()))?;
    let vt = &file.tracks[v];
    if vt.samples.len() != r.last - r.first + 1 || plan.copied_frames() != vt.samples.len() as u64 {
        return Err(fail(format!("{} video samples written, {} copied, {} planned", vt.samples.len(), r.last - r.first + 1, plan.copied_frames())));
    }
    if vt.entries.len() != 1 || vt.entries.first().map(|e| &e.codec) != Some(&source.video.config) {
        return Err(fail("the video configuration record is not the source CodecPrivate".into()));
    }
    let min_pts = r.track.samples[r.first..=r.last].iter().map(|s| s.pts).min().unwrap_or(0);
    if u64::from(vt.timescale) != r.track.timebase.1 {
        return Err(fail(format!("timescale {} instead of 1/{}", vt.timescale, r.track.timebase.1)));
    }
    for (k, o) in vt.samples.iter().enumerate() {
        let d = r.first + k;
        let s = &r.track.samples[d];
        if o.is_sync != s.keyframe || o.pts + vt.edit_offset != s.pts - min_pts {
            return Err(fail(format!("video sample {k} (source frame {d}) has another time or key flag")));
        }
        let a = file.read_sample(output, v, k).map_err(|e| fail(format!("video sample {k}: {e}")))?;
        let b = source.file.read_sample(source.bytes, source.video.track, d).map_err(io)?;
        if a != b {
            return Err(fail(format!("video sample {k} (source frame {d}) payload differs")));
        }
    }
    if tracks.is_empty() {
        if file.tracks.len() != 1 {
            return Err(fail(format!("the output has {} tracks, expected 1", file.tracks.len())));
        }
        return Ok(());
    }
    verify_audio_tracks(output, &video_only, &tracks).map(|_| ())
}

// --- MP4 / MOV → Matroska / WebM ------------------------------------------------------------

/// The Matroska codec ID and `CodecPrivate` of an MP4 track the cut copies.
fn mkv_codec(entry: &SampleEntry) -> Option<(&'static str, Vec<u8>)> {
    Some(match &entry.codec {
        CodecConfig::Avc(c) => ("V_MPEG4/ISO/AVC", c.to_bytes()),
        CodecConfig::Hevc(c) => ("V_MPEGH/ISO/HEVC", c.to_bytes()),
        CodecConfig::Av1(c) => ("V_AV1", c.to_bytes()),
        CodecConfig::Aac(c) => ("A_AAC", c.asc.clone()),
        CodecConfig::Ac3 { .. } => ("A_AC3", Vec::new()),
        CodecConfig::Eac3 { .. } => ("A_EAC3", Vec::new()),
        CodecConfig::Dts { .. } => ("A_DTS", Vec::new()),
        CodecConfig::TrueHd { .. } => ("A_TRUEHD", Vec::new()),
        CodecConfig::Flac(f) => ("A_FLAC", [b"fLaC".as_slice(), &f.metadata_blocks].concat()),
        CodecConfig::Pcm(p) if p.float => ("A_PCM/FLOAT/IEEE", Vec::new()),
        CodecConfig::Pcm(p) if p.big_endian => ("A_PCM/INT/BIG", Vec::new()),
        CodecConfig::Pcm(_) => ("A_PCM/INT/LIT", Vec::new()),
        _ => return None,
    })
}

/// A Matroska timestamp scale that holds `timescale` ticks exactly, if one keeps clusters of a
/// second or more within the 16-bit block offsets; 1 ms otherwise.
fn timestamp_scale(timescale: u32) -> u64 {
    let ts = u64::from(timescale.max(1));
    if 1_000_000_000 % ts == 0 && 1_000_000_000 / ts >= 30_518 { 1_000_000_000 / ts } else { 1_000_000 }
}

/// One block of the Matroska output: exact source time (ns from the cut), track, payload source.
struct Block {
    exact_ns: i128,
    track: usize,
    key: bool,
    /// Video: the source sample; audio: (input, payload).
    src: (usize, usize),
}

/// The blocks of an MP4 → Matroska copy, in source order per track, plus the audio inputs.
struct MkvPlan {
    specs: Vec<TrackSpec>,
    blocks: Vec<Block>,
    tracks: Vec<AudioTrackInput>,
    scale: u64,
    origin_ns: i128,
    doc_type: &'static str,
}

fn mp4_to_mkv_plan<S: cfa4_isobmff::ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &Mp4CopySource<'_, S>,
    audio: &AudioPlan,
) -> Result<MkvPlan, SmartCutError> {
    let doc_type = mkv_output(plan);
    let r = mp4_copy_range(plan, index, source, doc_type.is_some())?;
    let doc_type = doc_type.ok_or(SmartCutError::NotLossless)?;
    let samples = &r.track.samples[r.first..=r.last];
    let ts = i128::from(r.track.timescale.max(1));
    let first_pts = samples.iter().map(|s| s.pts).min().unwrap_or(0);
    let (codec_id, private) =
        mkv_codec(r.entry).ok_or(SmartCutError::Unsupported { codec: plan.codec, source: plan.source_container, output: plan.output_container })?;
    let mut video = TrackSpec::new(cfa4_matroska::TrackKind::Video, codec_id);
    video.codec_private = private;
    video.video_size = r.entry.video.as_ref().map(|v| (u32::from(v.width), u32::from(v.height)));
    video.language = Some(r.track.language.clone());
    let constant = samples.windows(2).all(|w| w[0].duration == w[1].duration);
    if constant && let Some(s) = samples.first() {
        video.default_duration_ns = u64::try_from(rescale(i128::from(s.duration), 1_000_000_000, ts)).ok();
    }
    let mut specs = vec![video];
    let mut blocks = Vec::new();
    for (k, s) in samples.iter().enumerate() {
        blocks.push(Block { exact_ns: rescale(i128::from(s.pts - first_pts), 1_000_000_000, ts), track: 0, key: s.is_sync, src: (r.first + k, 0) });
    }
    // the audio, placed against the first presented picture as the MP4 copy places it
    let cut = CutInterval::of_video(r.track, first_pts, u64::try_from(plan.expected_duration.value).unwrap_or(0));
    let tracks = copied_tracks(source.file, source.bytes, audio, cut)?;
    for (n, t) in tracks.iter().enumerate() {
        let (codec_id, private) = mkv_codec(&t.entry)
            .ok_or_else(|| SmartCutError::Incompatible { detail: format!("audio track {} ({}) has no Matroska mapping", t.track_id, t.codec) })?;
        if doc_type == "webm" {
            return Err(SmartCutError::Incompatible { detail: format!("WebM carries only Opus and Vorbis audio, not {codec_id}") });
        }
        let mut spec = TrackSpec::new(cfa4_matroska::TrackKind::Audio, codec_id);
        spec.codec_private = private;
        spec.language = Some(t.language.clone());
        if let Some(a) = &t.entry.audio {
            let bits = match &t.entry.codec {
                CodecConfig::Pcm(p) => Some(u64::from(p.bits)),
                _ => None,
            };
            spec.audio = Some((a.sample_rate, u64::from(a.channels), bits));
        }
        specs.push(spec);
        let ats = i128::from(t.timescale.max(1));
        let (num, den) = t.first_at;
        let mut at = 0i128;
        for (j, (_, dur)) in t.samples.iter().enumerate() {
            // first_at is in seconds as num / den; the payload starts `at` audio ticks later
            let exact = rescale(num * ats + at * den, 1_000_000_000, den.max(1) * ats);
            blocks.push(Block { exact_ns: exact, track: n + 1, key: true, src: (n, j) });
            at += i128::from(*dur);
        }
    }
    let origin_ns = blocks.iter().map(|b| b.exact_ns).min().unwrap_or(0);
    Ok(MkvPlan { specs, blocks, tracks, scale: timestamp_scale(r.track.timescale), origin_ns, doc_type })
}

/// The written time of a block: its exact time from the origin, rounded to the timestamp scale.
fn block_ns(b: &Block, origin: i128, scale: u64) -> i64 {
    let s = i128::from(scale.max(1));
    i64::try_from(rescale(b.exact_ns - origin, 1, s) * s).unwrap_or(i64::MAX)
}

/// Write the copy segments of a lossless `plan` and the planned `audio` from an MP4 / MOV
/// `source` into `out` as a new Matroska / WebM file. Check it with [`verify_mp4_to_mkv`].
pub fn execute_mp4_to_mkv<W: Write + Seek, S: cfa4_isobmff::ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &Mp4CopySource<'_, S>,
    audio: &AudioPlan,
    out: W,
) -> Result<(W, CrossReport), SmartCutError> {
    let p = mp4_to_mkv_plan(plan, index, source, audio)?;
    let r = mp4_copy_range(plan, index, source, true)?;
    let opts = MuxOptions { doc_type: p.doc_type.into(), timestamp_scale: p.scale, ..MuxOptions::default() };
    let mut w = MkvWriter::new(out, p.specs.clone(), opts).map_err(io)?;
    let mut report = CrossReport::default();
    report.log.push(format!("CodecPrivate of {} written from the {} record of the sample entry", p.specs[0].codec_id, r.entry.codec.name()));
    report.log.push(format!("times written on a {} ns timestamp scale", p.scale));
    // interleave as the Matroska copy does: video in decode order at the smallest time from it on
    let mut keys: Vec<i128> = p.blocks.iter().map(|b| b.exact_ns).collect();
    let mut floor = i128::MAX;
    for (k, b) in p.blocks.iter().enumerate().rev() {
        if b.track == 0 {
            floor = floor.min(b.exact_ns);
            keys[k] = floor;
        }
    }
    let mut order: Vec<usize> = (0..p.blocks.len()).collect();
    order.sort_by_key(|&k| (keys[k], p.blocks[k].track != 0, k));
    let mut av1 = match &r.entry.codec {
        CodecConfig::Av1(c) => Some(Av1AccessPoints::new(&c.config_obus)),
        _ => None,
    };
    let mut audio_frames = vec![0u64; p.tracks.len()];
    let scale = p.scale;
    for k in order {
        let b = &p.blocks[k];
        let ns = block_ns(b, p.origin_ns, scale);
        report.max_rounding_ns = report.max_rounding_ns.max(i64::try_from((i128::from(ns) - (b.exact_ns - p.origin_ns)).abs()).unwrap_or(i64::MAX));
        if b.track == 0 {
            let d = b.src.0;
            let data = source.file.read_sample(source.bytes, source.track, d).map_err(io)?;
            check_video_access_point(&r.entry.codec, index, d, &data, av1.as_mut())?;
            w.write_frame(0, ns, b.key, &data, None).map_err(io)?;
            report.frames += 1;
            report.payload_bytes = report.payload_bytes.saturating_add(data.len() as u64);
        } else {
            let (data, _) = &p.tracks[b.src.0].samples[b.src.1];
            w.write_frame(b.track, ns, true, data, None).map_err(io)?;
            audio_frames[b.src.0] += 1;
        }
    }
    report.audio = p.tracks.iter().zip(audio_frames).map(|(t, n)| (t.track_id, t.codec.clone(), n)).collect();
    for t in &p.tracks {
        report.log.push(format!("audio track {}: {} written as {}", t.track_id, t.codec, mkv_codec(&t.entry).map_or("?", |c| c.0)));
    }
    let out = w.finish().map_err(io)?;
    Ok((out, report))
}

/// Demux a file written by [`execute_mp4_to_mkv`]: every track has its Matroska codec, the video
/// `CodecPrivate` parses to the source configuration record, and every block holds its source
/// payload at its source time (rounded to the timestamp scale).
pub fn verify_mp4_to_mkv<S: cfa4_isobmff::ByteSource + ?Sized, O: cfa4_matroska::ByteSource>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &Mp4CopySource<'_, S>,
    audio: &AudioPlan,
    output: O,
) -> Result<(), SmartCutError> {
    let fail = |detail: String| SmartCutError::Verification { detail };
    let p = mp4_to_mkv_plan(plan, index, source, audio)?;
    let r = mp4_copy_range(plan, index, source, true)?;
    let file = cfa4_matroska::open(&output).map_err(|e| fail(format!("the output does not open: {e}")))?;
    if file.tracks.len() != p.specs.len() || file.info.timestamp_scale != p.scale {
        return Err(fail(format!("{} tracks on a {} ns scale, expected {} on {}", file.tracks.len(), file.info.timestamp_scale, p.specs.len(), p.scale)));
    }
    for (k, (t, spec)) in file.tracks.iter().zip(&p.specs).enumerate() {
        if t.codec_id != spec.codec_id || t.codec_private != spec.codec_private {
            return Err(fail(format!("track {k}: the codec or its CodecPrivate is not the one planned")));
        }
    }
    let parsed = match &file.tracks[0].codec {
        MkvCodec::Avc { avcc } => AvcConfig::parse(avcc).ok().map(CodecConfig::Avc),
        MkvCodec::Hevc { hvcc } => HevcConfig::parse(hvcc).ok().map(CodecConfig::Hevc),
        MkvCodec::Av1 { av1c } => Av1Config::parse(av1c).ok().map(CodecConfig::Av1),
        _ => None,
    };
    if parsed.as_ref() != Some(&r.entry.codec) {
        return Err(fail("the video CodecPrivate does not parse to the source configuration record".into()));
    }
    let mut seen = vec![0usize; p.specs.len()];
    let mut want: Vec<Vec<&Block>> = vec![Vec::new(); p.specs.len()];
    for b in &p.blocks {
        want[b.track].push(b);
    }
    for (k, (t, blocks)) in file.tracks.iter().zip(&want).enumerate() {
        if t.samples.len() != blocks.len() {
            return Err(fail(format!("track {k}: {} blocks written, {} copied", t.samples.len(), blocks.len())));
        }
        for (j, (o, b)) in t.samples.iter().zip(blocks).enumerate() {
            if t.ticks_to_ns(o.pts) != block_ns(b, p.origin_ns, p.scale) || o.keyframe != b.key {
                return Err(fail(format!("track {k} block {j} has another time or key flag")));
            }
            let a = file.read_sample(&output, k, j).map_err(|e| fail(format!("track {k} block {j}: {e}")))?;
            let same = if b.track == 0 {
                a == source.file.read_sample(source.bytes, source.track, b.src.0).map_err(io)?
            } else {
                a == p.tracks[b.src.0].samples[b.src.1].0
            };
            if !same {
                return Err(fail(format!("track {k} block {j} payload differs")));
            }
            seen[k] += 1;
        }
    }
    if seen[0] as u64 != plan.copied_frames() {
        return Err(fail(format!("{} video blocks, {} planned", seen[0], plan.copied_frames())));
    }
    Ok(())
}

/// The audio plan of an MP4 → Matroska cut: the MP4 Smart Cut's audio plan; WebM takes none of
/// those codecs, so every track is named as dropped there.
pub fn plan_mp4_to_mkv_audio<S: cfa4_isobmff::ByteSource + ?Sized>(
    plan: &ExportExecutionPlan,
    index: &StreamIndex,
    source: &Mp4CopySource<'_, S>,
) -> Result<AudioPlan, SmartCutError> {
    let r = mp4_copy_range(plan, index, source, mkv_output(plan).is_some())?;
    let first_pts = r.track.samples[r.first..=r.last].iter().map(|s| s.pts).min().unwrap_or(0);
    let cut = CutInterval::of_video(r.track, first_pts, u64::try_from(plan.expected_duration.value).unwrap_or(0));
    let mut audio = crate::audio_copy::plan_audio_copy(source.file, source.bytes, cut);
    if plan.output_container == Container::WebM {
        for t in std::mem::take(&mut audio.tracks) {
            audio
                .dropped
                .push(crate::audio_copy::AudioDrop { track_id: t.track_id, reason: format!("WebM carries only Opus and Vorbis audio, not {}", t.codec) });
        }
    }
    Ok(audio)
}

/// Whether `file`'s video track is the one a plan from `index` cuts (for callers choosing the
/// executor by the source's container).
pub fn mp4_track_of(file: &Mp4File, index: &StreamIndex) -> Option<usize> {
    file.tracks.iter().position(|t| t.kind == TrackKind::Video && t.id == index.stream_id)
}
