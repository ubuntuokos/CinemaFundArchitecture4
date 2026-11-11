//! The audio of a Smart Cut (WP-09b of `CFA4-SMART-CUT-RENDER-R1`, SCE-001 §9).
//!
//! The video executors ([`crate::smart_cut`], [`crate::boundary_reencode`]) write one video
//! track. This module adds the source's audio tracks to that file without re-encoding them:
//!
//! - **AAC:** whole packets are copied unchanged, with one extra packet before the cut as decoder
//!   pre-roll (the MDCT overlap of AAC needs the previous frame). An edit list starts and ends the
//!   presentation on the cut, so the encoder delay (priming) of the source and the pre-roll packet
//!   are hidden exactly as the source's own edit list hid its priming.
//! - **PCM:** the copied audio frames are cut at the exact sample; the bytes of every kept frame
//!   are unchanged.
//! - **A/V sync:** the audio keeps its source timing relative to the video: output audio time 0
//!   is the source audio at the first presented video picture, both with their edit lists
//!   applied. The only error is rounding that instant to the audio timescale (at most half an
//!   audio tick), and [`verify_audio_copy`] measures it in the written file.
//! - **Other audio codecs** (MP3, Opus, AC-3, ALAC…), tracks with several sample entries in the
//!   cut, and PCM whose timescale is not its sample rate are not copied: they are named in
//!   [`AudioPlan::dropped`] with the reason, and the caller shows that before writing (nothing is
//!   dropped silently).

use cfa4_isobmff::{Brand, ByteSource, CodecConfig, Edit, Mp4File, Mp4Writer, SampleEntry, Track, TrackConfig, TrackKind, WriteSample, WriterOptions};
use serde::{Deserialize, Serialize};
use std::io::Cursor;

use crate::smart_cut::{SmartCutError, io};

/// The presented video of a cut, in source presentation time: the video track's timescale with
/// its edit list applied (`pts + edit_offset`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutInterval {
    pub start: i64,
    pub duration: u64,
    pub timescale: u32,
}

impl CutInterval {
    /// The interval of a plan whose first presented picture has the raw source PTS `first_pts`
    /// and which lasts `duration` (both in the video track's timescale).
    pub fn of_video(track: &Track, first_pts: i64, duration: u64) -> Self {
        CutInterval { start: first_pts.saturating_add(track.edit_offset), duration, timescale: track.timescale }
    }
}

/// How one audio track is copied.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioTrackCopy {
    /// Index into the source's [`Mp4File::tracks`].
    pub track: usize,
    pub track_id: u32,
    /// `aac` or `pcm`.
    pub codec: String,
    /// The source samples read, in decode order (AAC: with the pre-roll packet).
    pub first: usize,
    pub last: usize,
    /// PCM: audio frames skipped at the start of sample `first` and frames written in all.
    pub pcm: Option<(u64, u64)>,
    /// Silence before the audio starts (audio later than the cut), in audio ticks.
    pub lead_in: u64,
    /// Media time of the output track where the presentation starts (audio ticks).
    pub media_time: i64,
    /// Presented audio, in audio ticks.
    pub presented: u64,
    /// The audio timescale.
    pub timescale: u32,
}

/// An audio track that is not copied, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDrop {
    pub track_id: u32,
    pub reason: String,
}

/// The audio of a cut.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioPlan {
    pub tracks: Vec<AudioTrackCopy>,
    pub dropped: Vec<AudioDrop>,
}

/// What [`verify_audio_copy`] measured for one track.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioTrackReport {
    pub track_id: u32,
    pub codec: String,
    pub samples: u64,
    pub payload_bytes: u64,
    /// Offset of the audio against the video at the cut, compared with the source, in
    /// nanoseconds (rounded); never more than half an audio tick.
    pub sync_error_ns: i64,
}

/// Round `v · num / den` to the nearest integer (halves away from zero).
fn rescale(v: i128, num: i128, den: i128) -> i128 {
    if den == 0 {
        return 0;
    }
    let p = v * num;
    let half = den / 2;
    if p >= 0 { (p + half) / den } else { (p - half) / den }
}

fn ticks(v: i128) -> Option<i64> {
    i64::try_from(v).ok()
}

/// Presentation time of sample `s` of `track` (its edit list applied).
fn pres(track: &Track, s: &cfa4_isobmff::Sample) -> i128 {
    i128::from(s.dts) + i128::from(track.edit_offset)
}

fn not_copied(track: &Track, reason: impl Into<String>) -> AudioDrop {
    AudioDrop { track_id: track.id, reason: reason.into() }
}

/// Plan the copy of every audio track of `file` for the cut `cut`.
pub fn plan_audio_copy(file: &Mp4File, cut: CutInterval) -> AudioPlan {
    let mut plan = AudioPlan::default();
    for (i, track) in file.tracks.iter().enumerate() {
        if track.kind != TrackKind::Audio {
            continue;
        }
        match plan_track(i, track, cut) {
            Ok(Some(t)) => plan.tracks.push(t),
            Ok(None) => plan.dropped.push(not_copied(track, "the audio does not overlap the cut")),
            Err(d) => plan.dropped.push(d),
        }
    }
    plan
}

fn plan_track(index: usize, track: &Track, cut: CutInterval) -> Result<Option<AudioTrackCopy>, AudioDrop> {
    let ats = i128::from(track.timescale);
    let vts = i128::from(cut.timescale);
    if ats == 0 || vts == 0 {
        return Err(not_copied(track, "the track has no timescale"));
    }
    let codec = match track.entries.first().map(|e| &e.codec) {
        Some(CodecConfig::Aac(_)) => "aac",
        Some(CodecConfig::Pcm(p)) => {
            if p.bytes_per_frame() == 0 || (p.sample_rate - f64::from(track.timescale)).abs() > 0.5 {
                return Err(not_copied(track, "PCM whose timescale is not its sample rate is not copied"));
            }
            "pcm"
        }
        Some(other) => return Err(not_copied(track, format!("{} audio is not copied yet", codec_name(other)))),
        None => return Err(not_copied(track, "the track has no sample entry")),
    };
    let samples = &track.samples;
    let (Some(head), Some(tail)) = (samples.first(), samples.last()) else { return Ok(None) };
    let a0 = rescale(i128::from(cut.start), ats, vts);
    let a1 = rescale(i128::from(cut.start) + i128::from(cut.duration), ats, vts);
    let end = |s: &cfa4_isobmff::Sample| pres(track, s) + i128::from(s.duration);
    if a1 <= a0 || a1 <= pres(track, head) || a0 >= end(tail) {
        return Ok(None);
    }
    // the first sample still sounding at the cut, and the last one starting before its end
    let s = samples.partition_point(|x| end(x) <= a0);
    let e = samples.partition_point(|x| pres(track, x) < a1).checked_sub(1).ok_or_else(|| not_copied(track, "no audio before the end of the cut"))?;
    let (Some(first_kept), Some(last_kept)) = (samples.get(s), samples.get(e)) else { return Ok(None) };
    let start_present = a0.max(pres(track, first_kept));
    let end_present = a1.min(end(last_kept));
    let lead_in = start_present - a0;
    let presented = end_present - start_present;
    // AAC decodes a packet with the one before it; PCM frames stand alone
    let first = if codec == "aac" { s.saturating_sub(1) } else { s };
    let entry = samples.get(first).map(|x| x.description_index);
    if samples.get(first..=e).is_none_or(|run| run.iter().any(|x| Some(x.description_index) != entry)) {
        return Err(not_copied(track, "the audio changes its codec configuration inside the cut"));
    }
    let base = samples.get(first).map_or(0, |x| i128::from(x.dts));
    let (media_time, pcm) = if codec == "pcm" {
        let skip = start_present - pres(track, first_kept);
        (0, Some((u64::try_from(skip).unwrap_or(0), u64::try_from(presented).unwrap_or(0))))
    } else {
        (start_present - i128::from(track.edit_offset) - base, None)
    };
    let bad = || not_copied(track, "the audio timestamps do not fit the output");
    Ok(Some(AudioTrackCopy {
        track: index,
        track_id: track.id,
        codec: codec.into(),
        first,
        last: e,
        pcm,
        lead_in: u64::try_from(lead_in).map_err(|_| bad())?,
        media_time: ticks(media_time).filter(|&m| m >= 0).ok_or_else(bad)?,
        presented: u64::try_from(presented).map_err(|_| bad())?,
        timescale: track.timescale,
    }))
}

fn codec_name(c: &CodecConfig) -> &'static str {
    match c {
        CodecConfig::Mp3 => "MP3 / MP2",
        CodecConfig::Pcm(_) => "PCM",
        CodecConfig::Aac(_) => "AAC",
        _ => "This",
    }
}

/// A movie timescale in which the video and every audio timescale count exactly (their least
/// common multiple), or the video timescale when that does not fit.
fn movie_timescale(video: u32, audio: impl IntoIterator<Item = u32>) -> u32 {
    fn gcd(a: u64, b: u64) -> u64 {
        if b == 0 { a } else { gcd(b, a % b) }
    }
    let video = video.max(1);
    let mut m = u64::from(video);
    for a in audio {
        let a = u64::from(a.max(1));
        m = m / gcd(m, a) * a;
        if m > u64::from(u32::MAX) {
            return video;
        }
    }
    u32::try_from(m).unwrap_or(video)
}

/// An audio track to add next to a video track: copied from a source ([`copied_tracks`]) or
/// encoded from a mix ([`crate::timeline_audio`]).
#[derive(Clone, Debug)]
pub struct AudioTrackInput {
    /// The source track id (0 for a mix) and the codec name, for the report.
    pub track_id: u32,
    pub codec: String,
    pub entry: SampleEntry,
    pub timescale: u32,
    pub language: String,
    /// Silence before the audio starts, in audio ticks.
    pub lead_in: u64,
    /// Media time where the presentation starts (audio ticks; AAC priming and pre-roll).
    pub media_time: i64,
    /// Presented audio, in audio ticks.
    pub presented: u64,
    /// Payloads and their durations, in decode order (PCM: whole frames, any count).
    pub samples: Vec<(Vec<u8>, u32)>,
    /// PCM: the payloads may be merged into other samples; only bytes and total length count.
    pub pcm: bool,
    /// When the first payload should be presented, relative to the first video picture, in
    /// seconds as `num / den` (negative: before it, hidden by the edit list).
    pub first_at: (i128, i128),
}

impl AudioTrackInput {
    fn edits(&self, movie: u32) -> Vec<Edit> {
        let to_movie = |v: u64| u64::try_from(rescale(i128::from(v), i128::from(movie), i128::from(self.timescale.max(1)))).unwrap_or(0);
        let mut edits = Vec::new();
        if self.lead_in > 0 {
            edits.push(Edit { segment_duration: to_movie(self.lead_in), media_time: -1, media_rate: 0x10000 });
        }
        edits.push(Edit { segment_duration: to_movie(self.presented), media_time: self.media_time, media_rate: 0x10000 });
        edits
    }
}

/// The source audio a copy plan writes, read and checked against the source.
pub fn copied_tracks<S: ByteSource + ?Sized>(source: &Mp4File, bytes: &S, plan: &AudioPlan, cut: CutInterval) -> Result<Vec<AudioTrackInput>, SmartCutError> {
    let mut out = Vec::new();
    for t in &plan.tracks {
        let track = source.tracks.get(t.track).ok_or(SmartCutError::NoTrack { track: t.track })?;
        let entry = track
            .samples
            .get(t.first)
            .and_then(|s| track.entries.get(usize::try_from(s.description_index.checked_sub(1)?).ok()?))
            .ok_or_else(|| SmartCutError::StaleIndex { detail: format!("audio track {} changed", t.track_id) })?;
        let samples = audio_samples(source, bytes, t)?;
        let first = samples.first().map_or(0, |x| x.1);
        let (ats, vts) = (i128::from(track.timescale.max(1)), i128::from(cut.timescale.max(1)));
        // the source audio at the first copied payload against the source video at the cut
        let a_src = first + i128::from(track.edit_offset);
        out.push(AudioTrackInput {
            track_id: t.track_id,
            codec: t.codec.clone(),
            entry: entry.clone(),
            timescale: track.timescale,
            language: track.language.clone(),
            lead_in: t.lead_in,
            media_time: t.media_time,
            presented: t.presented,
            samples: samples.into_iter().map(|(d, _, n)| (d, n)).collect(),
            pcm: t.pcm.is_some(),
            first_at: (a_src * vts - i128::from(cut.start) * ats, ats * vts),
        });
    }
    Ok(out)
}

/// One sample to write: its track in the output, data and timing.
struct Pending {
    time: i128,
    out_track: usize,
    data: Vec<u8>,
    sample: WriteSample<'static>,
}

/// Write `video_only` (a file of one video track from a video executor, presenting `video`
/// = (duration, timescale)) again with the audio `tracks`. The video samples, their timing and
/// sample entry are unchanged. Returns the new file.
pub fn mux_audio_tracks(video_only: &[u8], tracks: &[AudioTrackInput], video: (u64, u32), brand: Brand) -> Result<Vec<u8>, SmartCutError> {
    let v = cfa4_isobmff::open(video_only).map_err(io)?;
    let [vt] = v.tracks.as_slice() else {
        return Err(SmartCutError::Io { detail: format!("the video file has {} tracks, expected 1", v.tracks.len()) });
    };
    let [ventry] = vt.entries.as_slice() else {
        return Err(SmartCutError::MixedSampleEntries);
    };
    let movie = movie_timescale(video.1, tracks.iter().map(|t| t.timescale));
    let mut opts = WriterOptions::new(brand);
    opts.movie_timescale = movie;
    let mut w = Mp4Writer::new(Cursor::new(Vec::new()), opts).map_err(io)?;
    let mut vcfg = TrackConfig::new(ventry.clone(), vt.timescale);
    vcfg.language = vt.language.clone();
    vcfg.matrix = Some(vt.matrix);
    // one edit (the usual case) presents exactly the video; others are rescaled from the old movie
    let single = vt.edits.len() == 1;
    vcfg.edits = vt
        .edits
        .iter()
        .map(|e| {
            let (d, from) = if single { video } else { (e.segment_duration, v.timescale) };
            Edit { segment_duration: u64::try_from(rescale(i128::from(d), i128::from(movie), i128::from(from.max(1)))).unwrap_or(0), ..*e }
        })
        .collect();
    let out_video = w.add_track(vcfg).map_err(io)?;
    let mut pending = Vec::new();
    for (i, s) in vt.samples.iter().enumerate() {
        let data = v.read_sample(video_only, 0, i).map_err(io)?;
        let offset = i32::try_from(s.pts - s.dts).map_err(|_| SmartCutError::Timestamps { decode_index: i as u64 })?;
        let time = i128::from(s.dts) * 1_000_000_000 / i128::from(vt.timescale.max(1));
        let sample = WriteSample { data: &[], duration: s.duration, composition_offset: offset, is_sync: s.is_sync };
        pending.push(Pending { time, out_track: out_video, data, sample });
    }
    for t in tracks {
        let mut cfg = TrackConfig::new(t.entry.clone(), t.timescale);
        cfg.language = t.language.clone();
        cfg.edits = t.edits(movie);
        let out = w.add_track(cfg).map_err(io)?;
        let mut dts = 0i128;
        for (data, duration) in &t.samples {
            let time = dts * 1_000_000_000 / i128::from(t.timescale.max(1));
            let sample = WriteSample { data: &[], duration: *duration, composition_offset: 0, is_sync: true };
            pending.push(Pending { time, out_track: out, data: data.clone(), sample });
            dts += i128::from(*duration);
        }
    }
    // interleave the tracks by decode time (stable: equal times keep the video first)
    pending.sort_by_key(|p| p.time);
    for p in &pending {
        w.write_sample(p.out_track, WriteSample { data: &p.data, ..p.sample }).map_err(io)?;
    }
    Ok(w.finish().map_err(io)?.into_inner())
}

/// Write `video_only` again with the audio tracks of `plan` copied from `source` (see
/// [`mux_audio_tracks`]).
pub fn mux_with_audio<S: ByteSource + ?Sized>(
    video_only: &[u8],
    source: &Mp4File,
    bytes: &S,
    plan: &AudioPlan,
    cut: CutInterval,
    brand: Brand,
) -> Result<Vec<u8>, SmartCutError> {
    let tracks = copied_tracks(source, bytes, plan, cut)?;
    mux_audio_tracks(video_only, &tracks, (cut.duration, cut.timescale), brand)
}

/// The payloads a track copy writes: (bytes, source decode time of the first byte, duration).
fn audio_samples<S: ByteSource + ?Sized>(source: &Mp4File, bytes: &S, t: &AudioTrackCopy) -> Result<Vec<(Vec<u8>, i128, u32)>, SmartCutError> {
    let track = source.tracks.get(t.track).ok_or(SmartCutError::NoTrack { track: t.track })?;
    let stale = |d: usize| SmartCutError::StaleIndex { detail: format!("audio track {} sample {d} changed", t.track_id) };
    let mut out = Vec::new();
    match (t.pcm, track.entries.first().map(|e| &e.codec)) {
        (Some((skip, frames)), Some(CodecConfig::Pcm(p))) => {
            let fb = u64::from(p.bytes_per_frame());
            let mut skip = skip;
            let mut left = frames;
            for d in t.first..=t.last {
                let s = track.samples.get(d).ok_or_else(|| stale(d))?;
                let data = source.read_sample(bytes, t.track, d).map_err(io)?;
                let have = data.len() as u64 / fb.max(1);
                if data.len() as u64 != have * fb || have != u64::from(s.duration) {
                    return Err(stale(d));
                }
                let from = skip.min(have);
                let take = (have - from).min(left);
                let (a, b) = (usize::try_from(from * fb).map_err(|_| stale(d))?, usize::try_from((from + take) * fb).map_err(|_| stale(d))?);
                if take > 0 {
                    out.push((data[a..b].to_vec(), i128::from(s.dts) + i128::from(from), u32::try_from(take).map_err(|_| stale(d))?));
                }
                skip -= from;
                left -= take;
            }
            if left != 0 {
                return Err(stale(t.last));
            }
        }
        (None, Some(CodecConfig::Aac(_))) => {
            for d in t.first..=t.last {
                let s = track.samples.get(d).ok_or_else(|| stale(d))?;
                let data = source.read_sample(bytes, t.track, d).map_err(io)?;
                if data.len() as u64 != u64::from(s.size) {
                    return Err(stale(d));
                }
                out.push((data, i128::from(s.dts), s.duration));
            }
        }
        _ => return Err(SmartCutError::StaleIndex { detail: format!("audio track {} changed its codec", t.track_id) }),
    }
    Ok(out)
}

/// Read back a file written by [`mux_audio_tracks`]: the video track must be the one of
/// `video_only` unchanged, every audio track must hold its payloads unchanged, and its first
/// payload must be presented where [`AudioTrackInput::first_at`] says, within half an audio tick.
pub fn verify_audio_tracks(output: &[u8], video_only: &[u8], tracks: &[AudioTrackInput]) -> Result<Vec<AudioTrackReport>, SmartCutError> {
    let fail = |detail: String| SmartCutError::Verification { detail };
    let file = cfa4_isobmff::open(output).map_err(|e| fail(format!("the output does not open: {e}")))?;
    let v = cfa4_isobmff::open(video_only).map_err(io)?;
    if file.tracks.len() != 1 + tracks.len() {
        return Err(fail(format!("the output has {} tracks, expected {}", file.tracks.len(), 1 + tracks.len())));
    }
    let (Some(vt), Some(ot)) = (v.tracks.first(), file.tracks.first()) else { return Err(fail("no video track".into())) };
    if ot.samples.len() != vt.samples.len() || ot.entries != vt.entries || ot.edit_offset != vt.edit_offset || ot.timescale != vt.timescale {
        return Err(fail("the video track changed when the audio was added".into()));
    }
    for (i, (a, b)) in vt.samples.iter().zip(&ot.samples).enumerate() {
        let same = (a.dts, a.pts, a.duration, a.is_sync) == (b.dts, b.pts, b.duration, b.is_sync)
            && v.read_sample(video_only, 0, i).map_err(io)? == file.read_sample(output, 0, i).map_err(io)?;
        if !same {
            return Err(fail(format!("video sample {i} changed when the audio was added")));
        }
    }
    // the first video picture of the output (edit list applied), in video ticks
    let v_out = ot.samples.iter().map(|s| i128::from(s.pts)).min().unwrap_or(0) + i128::from(ot.edit_offset);
    let vts = i128::from(ot.timescale.max(1));
    let mut reports = Vec::new();
    for (k, t) in tracks.iter().enumerate() {
        let out = file.tracks.get(k + 1).ok_or_else(|| fail(format!("audio track {} is missing", t.track_id)))?;
        let written: Vec<u8> = (0..out.samples.len()).map(|i| file.read_sample(output, k + 1, i)).collect::<Result<Vec<_>, _>>().map_err(io)?.concat();
        let expected: Vec<u8> = t.samples.iter().flat_map(|e| e.0.iter().copied()).collect();
        if written != expected {
            return Err(fail(format!("audio track {}: the written audio differs from the source", t.track_id)));
        }
        let durations = |it: &mut dyn Iterator<Item = u32>| it.map(u64::from).sum::<u64>();
        let same_packets = out.samples.len() == t.samples.len() && out.samples.iter().zip(&t.samples).all(|(s, e)| s.duration == e.1);
        let same_length = durations(&mut out.samples.iter().map(|s| s.duration)) == durations(&mut t.samples.iter().map(|e| e.1));
        if !(same_length && (t.pcm || same_packets)) {
            return Err(fail(format!("audio track {}: the packets or their durations changed", t.track_id)));
        }
        if t.samples.is_empty() {
            return Err(fail(format!("audio track {} is empty", t.track_id)));
        }
        // output audio (first payload) − output video, against `first_at`; scaled by ats·vts·den
        let ats = i128::from(out.timescale.max(1));
        let (num, den) = (t.first_at.0, t.first_at.1.max(1));
        let a_out = i128::from(out.edit_offset);
        let err = a_out * vts * den - v_out * ats * den - num * ats * vts;
        if 2 * err.abs() > vts * den {
            return Err(fail(format!("audio track {} is out of sync by {} audio ticks", t.track_id, err as f64 / (vts * den) as f64)));
        }
        reports.push(AudioTrackReport {
            track_id: t.track_id,
            codec: t.codec.clone(),
            samples: out.samples.len() as u64,
            payload_bytes: written.len() as u64,
            sync_error_ns: ticks(rescale(err, 1_000_000_000, ats * vts * den)).unwrap_or(i64::MAX),
        });
    }
    Ok(reports)
}

/// [`verify_audio_tracks`] for the audio of `plan` copied from `source`.
pub fn verify_audio_copy<S: ByteSource + ?Sized>(
    output: &[u8],
    video_only: &[u8],
    source: &Mp4File,
    bytes: &S,
    plan: &AudioPlan,
    cut: CutInterval,
) -> Result<Vec<AudioTrackReport>, SmartCutError> {
    let tracks = copied_tracks(source, bytes, plan, cut)?;
    verify_audio_tracks(output, video_only, &tracks)
}
