//! Facts about a media file, gathered with CFA4's own demuxers and software decoders: what the
//! file says about itself (streams, timing, colour) and what a full decode finds. No judgement
//! here; [`crate::checks`] turns facts into check results.
//!
//! Sample-level analysis (timestamps, full decode) covers MP4 / MOV, the containers CFA4 exports
//! H.264, HEVC and AV1 to. For other containers the facts say so, and the checks that need them
//! report NOT_RUN instead of passing on what playback would conceal.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};

use cfa4_frame::{PixelData, VideoFrame};
use cfa4_isobmff::{CodecConfig, Mp4File, TrackKind};
use cfa4_media::{MediaInfo, SharedReader};

use crate::audio::{AudioAnalyzer, AudioFacts};
use crate::picture::{PictureAnalyzer, PictureFacts};

/// Most decode errors and timing anomalies kept per stream (the rest are counted).
pub const MAX_KEPT: usize = 200;

/// A media reader as the MP4 demuxer's byte source.
struct Src(SharedReader);

impl cfa4_isobmff::ByteSource for Src {
    fn len(&self) -> u64 {
        self.0.len()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
        self.0.read_at(offset, buf)
    }
}

/// Something found at a frame (video) or packet (audio) index, in presentation order.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub index: u64,
    /// Media time of the frame / packet, in milliseconds (when known).
    pub ms: Option<f64>,
    /// Where it ends (a run of samples or pictures), in milliseconds.
    pub end_ms: Option<f64>,
    pub message: String,
}

/// A bounded list of findings plus how many there were in all.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Findings {
    pub kept: Vec<Found>,
    pub total: u64,
}

impl Findings {
    pub(crate) fn push(&mut self, f: Found) {
        self.total = self.total.saturating_add(1);
        if self.kept.len() < MAX_KEPT {
            self.kept.push(f);
        }
    }
    pub fn is_empty(&self) -> bool {
        self.total == 0
    }
}

/// Container facts of one MP4 track.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrackFacts {
    pub track: usize,
    pub codec: String,
    pub timescale: u32,
    pub samples: usize,
    /// Presented start and end (edit list applied), in milliseconds.
    pub start_ms: f64,
    pub end_ms: f64,
    /// Sample descriptions (more than one: the codec setup changes mid-stream).
    pub entries: usize,
    /// The most common sample duration, in the track timescale.
    pub nominal_duration: u32,
    /// Samples whose duration differs from the nominal one by more than rounding.
    pub odd_durations: usize,
    pub dts_backwards: Findings,
    pub duplicate_pts: Findings,
    /// Steps between pictures longer than one and a half nominal frame periods.
    pub gaps: Findings,
    pub channels: u32,
    pub sample_rate: u32,
}

/// What the MP4 / MOV index says.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Mp4Facts {
    pub quicktime: bool,
    pub fragmented: bool,
    pub video: Option<TrackFacts>,
    pub audio: Vec<TrackFacts>,
    pub subtitles: usize,
}

/// A full decode of one stream.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StreamDecode {
    pub track: usize,
    pub decoder: String,
    pub samples: usize,
    /// Frames out of the video decoder; packets decoded for audio.
    pub decoded: u64,
    pub errors: Findings,
    /// Why this stream could not be decoded at all (no decoder for the codec).
    pub unsupported: Option<String>,
}

/// A full decode of the file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DecodeFacts {
    pub video: Option<StreamDecode>,
    pub audio: Vec<StreamDecode>,
    /// Longest run of identical consecutive decoded pictures, and how many pictures repeated the
    /// one before. Reported, not judged: still images repeat by design.
    pub longest_repeat: u64,
    pub repeated: u64,
    pub decoded_size: Option<(u32, u32)>,
    pub size_changes: u64,
    /// Picture analysis of the decoded video (black, clipping, contact sheet).
    pub picture: Option<PictureFacts>,
    /// Audio analysis of every decoded audio track, in track order.
    pub audio_facts: Vec<AudioFacts>,
}

/// Everything known about the file.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub open_error: Option<String>,
    pub info: Option<MediaInfo>,
    pub mp4: Option<Mp4Facts>,
    /// The file looks like MP4 / MOV but its index could not be read.
    pub mp4_error: Option<String>,
    pub decode: Option<DecodeFacts>,
    pub cancelled: bool,
}

fn is_mp4(head: &[u8]) -> bool {
    head.get(4..8).is_some_and(|t| matches!(t, b"ftyp" | b"moov" | b"mdat" | b"free" | b"wide" | b"skip" | b"styp"))
}

fn ms(units: i128, timescale: u32) -> f64 {
    if timescale == 0 { 0.0 } else { units as f64 * 1000.0 / f64::from(timescale) }
}

/// Container facts of a track: presented span, sample durations, timestamp order.
fn track_facts(file: &Mp4File, ti: usize) -> Option<TrackFacts> {
    let t = file.tracks.get(ti)?;
    let mut f = TrackFacts {
        track: ti,
        codec: t.codec().map(|c| c.name().to_string()).unwrap_or_default(),
        timescale: t.timescale,
        samples: t.samples.len(),
        entries: t.entries.len(),
        ..TrackFacts::default()
    };
    if let Some(a) = t.audio() {
        f.channels = a.channels;
        f.sample_rate = a.sample_rate as u32;
    }
    let mut durs: Vec<u32> = t.samples.iter().map(|s| s.duration).collect();
    durs.sort_unstable();
    f.nominal_duration = durs.get(durs.len() / 2).copied().unwrap_or(0);
    let nominal = i64::from(f.nominal_duration);
    f.odd_durations = t.samples.iter().take(t.samples.len().saturating_sub(1)).filter(|s| (i64::from(s.duration) - nominal).abs() > 1).count();
    for (i, w) in t.samples.windows(2).enumerate() {
        if let [a, b] = w
            && b.dts < a.dts
        {
            f.dts_backwards.push(Found {
                index: i as u64 + 1,
                ms: Some(ms(i128::from(b.dts), t.timescale)),
                end_ms: None,
                message: "decode timestamp goes backwards".into(),
            });
        }
    }
    // presentation order
    let mut order: Vec<(i64, usize)> = t.samples.iter().enumerate().map(|(i, s)| (s.pts, i)).collect();
    order.sort_unstable();
    for (k, w) in order.windows(2).enumerate() {
        let [(pa, _), (pb, _)] = w else { continue };
        let at = Some(ms(i128::from(*pb) + i128::from(t.edit_offset), t.timescale));
        if pa == pb {
            f.duplicate_pts.push(Found { index: k as u64 + 1, ms: at, end_ms: None, message: "two pictures share a presentation time".into() });
            continue;
        }
        // against the nominal frame duration: a picture held for several frame periods leaves the
        // frames of a constant-rate plan missing, whatever its own sample duration says
        let step = i128::from(*pb) - i128::from(*pa);
        if t.kind == TrackKind::Video && nominal > 0 && step * 2 > i128::from(nominal) * 3 {
            let missing = (step as f64 / nominal as f64) - 1.0;
            f.gaps.push(Found { index: k as u64 + 1, ms: at, end_ms: None, message: format!("gap of {missing:.1} frame(s) before this picture") });
        }
    }
    let first_pts = order.first().map_or(0, |(p, _)| *p);
    let last_end = t.samples.iter().map(|s| i128::from(s.pts) + i128::from(s.duration)).max().unwrap_or(0);
    f.start_ms = ms(i128::from(first_pts) + i128::from(t.edit_offset), t.timescale);
    f.end_ms = ms(last_end + i128::from(t.edit_offset), t.timescale);
    // an edit list says how long the track is presented (encoder priming and padding cut off)
    if !t.edits.is_empty() && file.timescale > 0 {
        let presented = t.edits.iter().fold(0u128, |a, e| a.saturating_add(u128::from(e.segment_duration)));
        let first_media = t.edits.iter().find(|e| e.media_time >= 0);
        let empty_lead = t.edits.iter().take_while(|e| e.media_time < 0).fold(0u128, |a, e| a.saturating_add(u128::from(e.segment_duration)));
        if first_media.is_some() && presented < (1u128 << 100) {
            f.start_ms = empty_lead as f64 * 1000.0 / f64::from(file.timescale);
            f.end_ms = presented as f64 * 1000.0 / f64::from(file.timescale);
        }
    }
    Some(f)
}

fn mp4_facts(file: &Mp4File) -> Mp4Facts {
    let video = file.track_of_kind(TrackKind::Video).and_then(|i| track_facts(file, i));
    let audio = file.tracks.iter().enumerate().filter(|(_, t)| t.kind == TrackKind::Audio).filter_map(|(i, _)| track_facts(file, i)).collect();
    let subtitles = file.tracks.iter().filter(|t| t.kind == TrackKind::Subtitle).count();
    Mp4Facts { quicktime: file.is_quicktime, fragmented: file.fragmented, video, audio, subtitles }
}

fn frame_hash(f: &VideoFrame) -> u64 {
    let mut h = DefaultHasher::new();
    (f.width, f.height).hash(&mut h);
    match &f.data {
        PixelData::Rgba8(d) => d.hash(&mut h),
        PixelData::RgbaF32(d) => d.iter().for_each(|v| v.to_bits().hash(&mut h)),
        PixelData::Yuv8 { planes, alpha, .. } => {
            planes.iter().for_each(|p| p.hash(&mut h));
            alpha.hash(&mut h);
        }
        PixelData::Yuv16 { planes, alpha, .. } => {
            planes.iter().for_each(|p| p.hash(&mut h));
            alpha.hash(&mut h);
        }
    }
    h.finish()
}

/// Run `f`, turning a panic in a decoder into an error (a hostile file must not take QC down).
fn guarded<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| Err("the decoder crashed on this sample".into()))
}

struct Pictures {
    decoded: u64,
    last: Option<u64>,
    run: u64,
    longest: u64,
    repeated: u64,
    size: Option<(u32, u32)>,
    size_changes: u64,
    analyzer: Option<PictureAnalyzer>,
    analysis: Option<PictureFacts>,
}

impl Pictures {
    fn new(expected: usize) -> Self {
        Pictures {
            decoded: 0,
            last: None,
            run: 0,
            longest: 0,
            repeated: 0,
            size: None,
            size_changes: 0,
            analyzer: Some(PictureAnalyzer::new(expected)),
            analysis: None,
        }
    }

    /// A decoded picture shown at `ms`; `presented`: inside the edit list's range, so it is
    /// measured (pictures the edit list hides are decoded and counted, never judged).
    fn take_at(&mut self, f: &VideoFrame, ms: Option<f64>, presented: bool) {
        let index = self.decoded;
        self.take(f);
        if presented && let Some(a) = self.analyzer.as_mut() {
            a.take(f, index, ms);
        }
    }

    fn take(&mut self, f: &VideoFrame) {
        self.decoded = self.decoded.saturating_add(1);
        let h = frame_hash(f);
        if self.last == Some(h) {
            self.run = self.run.saturating_add(1);
            self.repeated = self.repeated.saturating_add(1);
        } else {
            self.run = 1;
        }
        self.longest = self.longest.max(self.run);
        self.last = Some(h);
        let size = (f.width, f.height);
        if self.size.is_some_and(|s| s != size) {
            self.size_changes = self.size_changes.saturating_add(1);
        }
        self.size = Some(size);
    }
}

fn decode_video(file: &Mp4File, src: &Src, ti: usize, cancel: &AtomicBool) -> (StreamDecode, Pictures, bool) {
    let mut pics = Pictures::new(file.tracks.get(ti).map_or(0, |t| t.samples.len()));
    let mut out = StreamDecode { track: ti, ..StreamDecode::default() };
    let Some(t) = file.tracks.get(ti) else { return (out, pics, false) };
    let at_ms = |pts: i64| Some(ms(i128::from(pts) + i128::from(t.edit_offset), t.timescale));
    out.samples = t.samples.len();
    let Some(entry) = t.entries.first() else {
        out.unsupported = Some("the video track has no sample description".into());
        return (out, pics, false);
    };
    let mut dec = match cfa4_codecs::software_video_decoder(entry) {
        Ok(d) => d,
        Err(e) => {
            out.unsupported = Some(e.to_string());
            return (out, pics, false);
        }
    };
    out.decoder = dec.name().to_string();
    // the container's colour wins over the bitstream's for YUV pictures, as in playback
    let (w, h) = entry.video.as_ref().map_or((0, 0), |v| (u32::from(v.width), u32::from(v.height)));
    let color = cfa4_codecs::mp4::container_color(entry, w, h);
    let range = presented_range(file, t);
    let take = |pics: &mut Pictures, f: &VideoFrame, pts: i64| {
        let shown = pts.saturating_add(t.edit_offset);
        let presented = range.is_none_or(|(a, b)| shown >= a && shown < b);
        match color {
            Some(c) if c != f.color && !matches!(f.data, cfa4_frame::PixelData::Rgba8(_) | cfa4_frame::PixelData::RgbaF32(_)) => {
                let mut g = f.clone();
                g.color = c;
                pics.take_at(&g, at_ms(pts), presented);
            }
            _ => pics.take_at(f, at_ms(pts), presented),
        }
    };
    // presentation index of every sample, for messages
    let mut order: Vec<(i64, usize)> = t.samples.iter().enumerate().map(|(i, s)| (s.pts, i)).collect();
    order.sort_unstable();
    let mut frame_of = vec![0u64; t.samples.len()];
    for (k, (_, i)) in order.iter().enumerate() {
        if let Some(slot) = frame_of.get_mut(*i) {
            *slot = k as u64;
        }
    }
    for (i, s) in t.samples.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return (out, pics, true);
        }
        let index = frame_of.get(i).copied().unwrap_or(i as u64);
        let at = Some(ms(i128::from(s.pts) + i128::from(t.edit_offset), t.timescale));
        let r = guarded(|| {
            let data = file.read_sample(src, ti, i).map_err(|e| format!("unreadable sample: {e}"))?;
            dec.decode(&data, s.pts).map_err(|e| format!("decode error: {e}"))
        });
        match r {
            Ok(frames) => frames.iter().for_each(|f| take(&mut pics, &f.frame, f.pts)),
            Err(message) => out.errors.push(Found { index, ms: at, end_ms: None, message }),
        }
    }
    match guarded(|| Ok(dec.flush())) {
        Ok(frames) => frames.iter().for_each(|f| take(&mut pics, &f.frame, f.pts)),
        Err(message) => out.errors.push(Found { index: t.samples.len() as u64, ms: None, end_ms: None, message }),
    }
    out.decoded = pics.decoded;
    let end = track_facts(file, ti).map(|f| f.end_ms);
    pics.analysis = pics.analyzer.take().map(|a| a.finish(end));
    (out, pics, false)
}

/// The presentation times (track timescale, edit list applied) a track shows: from the end of
/// its leading empty edits for the edits' total duration. `None` without an edit list.
fn presented_range(file: &Mp4File, t: &cfa4_isobmff::Track) -> Option<(i64, i64)> {
    let first = t.edits.iter().find(|e| e.media_time >= 0)?;
    if t.timescale == 0 || file.timescale == 0 {
        return None;
    }
    let start = first.media_time.saturating_add(t.edit_offset);
    let presented = t.edits.iter().filter(|e| e.media_time >= 0).fold(0i128, |a, e| a.saturating_add(i128::from(e.segment_duration)));
    // rounded up: the movie timescale is often coarser than the track's, and a duration rounded
    // down there would hide the last picture
    let fts = i128::from(file.timescale);
    let len = presented.saturating_mul(i128::from(t.timescale)).saturating_add(fts - 1) / fts;
    (len > 0).then(|| (start, start.saturating_add(i64::try_from(len).unwrap_or(i64::MAX))))
}

/// The presented range of an audio track in sample frames at `rate`: frames to skip (encoder
/// priming, the first edit's media time), frames to keep (the edits' duration), and where the
/// kept audio starts on the file's timeline, ms (after leading empty edits).
fn presented_frames(file: &Mp4File, t: &cfa4_isobmff::Track, rate: u32) -> (u64, Option<u64>, f64) {
    let Some(first) = t.edits.iter().find(|e| e.media_time >= 0) else { return (0, None, 0.0) };
    if t.timescale == 0 || file.timescale == 0 {
        return (0, None, 0.0);
    }
    let start_ms = ms(i128::from(first.media_time) + i128::from(t.edit_offset), t.timescale).max(0.0);
    let skip = u128::try_from(first.media_time).unwrap_or(0).saturating_mul(u128::from(rate)) / u128::from(t.timescale);
    let presented = t.edits.iter().filter(|e| e.media_time >= 0).fold(0u128, |a, e| a.saturating_add(u128::from(e.segment_duration)));
    let keep = presented.saturating_mul(u128::from(rate)) / u128::from(file.timescale);
    (u64::try_from(skip).unwrap_or(u64::MAX), (presented > 0).then(|| u64::try_from(keep).unwrap_or(u64::MAX)), start_ms)
}

fn decode_audio(file: &Mp4File, src: &Src, ti: usize, cancel: &AtomicBool) -> (StreamDecode, Option<AudioFacts>, bool) {
    let mut out = StreamDecode { track: ti, ..StreamDecode::default() };
    let Some(t) = file.tracks.get(ti) else { return (out, None, false) };
    out.samples = t.samples.len();
    let Some(codec) = t.codec() else {
        out.unsupported = Some("the audio track has no sample description".into());
        return (out, None, false);
    };
    let rate = t.audio().map_or(0, |a| a.sample_rate as u32);
    let pcm = matches!(codec, CodecConfig::Pcm(_));
    let mut dec = if pcm {
        None
    } else {
        match cfa4_codecs::audio::PacketDecoder::for_isobmff(codec, rate) {
            Ok(d) => Some(d),
            Err(e) => {
                out.unsupported = Some(e.to_string());
                return (out, None, false);
            }
        }
    };
    out.decoder = if pcm { "PCM".into() } else { codec.name().to_string() };
    let (skip, keep, start_ms) = presented_frames(file, t, rate);
    let mut analyzer = AudioAnalyzer::new(ti, rate, skip, keep, start_ms);
    let mut ts: u64 = 0;
    for (i, s) in t.samples.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return (out, None, true);
        }
        let at = Some(ms(i128::from(s.pts) + i128::from(t.edit_offset), t.timescale));
        let r = guarded(|| {
            let data = file.read_sample(src, ti, i).map_err(|e| format!("unreadable packet: {e}"))?;
            match (dec.as_mut(), codec) {
                (Some(d), _) => d.decode(&data, ts).map_err(|e| format!("decode error: {e}")),
                (None, CodecConfig::Pcm(cfg)) => Ok(cfa4_codecs::audio::decode_pcm(&data, cfg)),
                (None, _) => Ok(Vec::new()),
            }
        });
        match r {
            Ok(planes) => {
                out.decoded = out.decoded.saturating_add(1);
                analyzer.push(&planes);
            }
            Err(message) => out.errors.push(Found { index: i as u64, ms: at, end_ms: None, message }),
        }
        ts = ts.saturating_add(u64::from(s.duration));
    }
    (out, Some(analyzer.finish()), false)
}

/// Gather the facts. `decode`: also decode every sample (the expensive part).
pub fn inspect(name: &str, reader: SharedReader, decode: bool, cancel: &AtomicBool) -> Facts {
    let mut facts = Facts::default();
    let opened =
        catch_unwind(AssertUnwindSafe(|| cfa4_media::reader::open_reader(name, reader.clone(), &cfa4_codecs::reader_openers(), &cfa4_codecs::openers())));
    match opened {
        Ok(Ok(src)) => facts.info = Some(src.info().clone()),
        Ok(Err(e)) => facts.open_error = Some(e.to_string()),
        Err(_) => facts.open_error = Some("the demuxer crashed on this file".into()),
    }
    let head = cfa4_media::reader::read_range(&*reader, 0, 16).unwrap_or_default();
    if !is_mp4(&head) {
        return facts;
    }
    let src = Src(reader);
    let parsed = catch_unwind(AssertUnwindSafe(|| cfa4_isobmff::open(Src(src.0.clone()))));
    let file = match parsed {
        Ok(Ok(f)) => f,
        Ok(Err(e)) => {
            facts.mp4_error = Some(e.to_string());
            return facts;
        }
        Err(_) => {
            facts.mp4_error = Some("the MP4 demuxer crashed on this file".into());
            return facts;
        }
    };
    facts.mp4 = Some(mp4_facts(&file));
    if !decode || facts.open_error.is_some() {
        return facts;
    }
    let mut d = DecodeFacts::default();
    if let Some(vi) = file.track_of_kind(TrackKind::Video) {
        let (v, pics, cancelled) = decode_video(&file, &src, vi, cancel);
        d.video = Some(v);
        d.longest_repeat = pics.longest;
        d.repeated = pics.repeated;
        d.decoded_size = pics.size;
        d.size_changes = pics.size_changes;
        d.picture = pics.analysis;
        if cancelled {
            facts.cancelled = true;
            facts.decode = Some(d);
            return facts;
        }
    }
    for (ai, t) in file.tracks.iter().enumerate() {
        if t.kind != TrackKind::Audio {
            continue;
        }
        let (a, analysis, cancelled) = decode_audio(&file, &src, ai, cancel);
        d.audio.push(a);
        d.audio_facts.extend(analysis);
        if cancelled {
            facts.cancelled = true;
            break;
        }
    }
    facts.decode = Some(d);
    facts
}
