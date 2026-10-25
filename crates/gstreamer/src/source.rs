//! An engine media source decoded by GStreamer (ADR 0002, step 2).
//!
//! One `playbin` pipeline decodes video into an RGBA `appsink`; another, created on the first
//! audio request, decodes the requested audio stream into interleaved 32-bit float at the sample
//! rate the engine asks for. Both run unsynchronised (as fast as the engine pulls) and are seeked
//! with flushing, accurate seeks; sequential requests just pull the next buffer, so playback does
//! not seek on every frame. Everything is safe code and every failure is a `MediaError`.

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use cfa4_frame::{AudioBuffer, VideoFrame};
use cfa4_media::{AudioStreamInfo, FrameRequest, MediaError, MediaInfo as EngineInfo, MediaKind, MediaSource, VideoStreamInfo};
use cfa4_time::{FrameRate, Tick};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;

use crate::{DEFAULT_DISCOVER_TIMEOUT, Error, StreamKind, ticks_from_nanos};

/// How long one buffer pull or state change may take before the request fails.
const WAIT: gst::ClockTime = gst::ClockTime::from_seconds(10);
/// Largest decoded picture accepted (bytes of RGBA); larger caps are treated as hostile.
const MAX_FRAME_BYTES: usize = 1 << 30;
/// Most audio frames one request may ask for (about 5.8 minutes at 48 kHz).
const MAX_AUDIO_FRAMES: usize = 1 << 24;
/// Most channels decoded per audio stream.
const MAX_CHANNELS: u32 = 32;
/// Pulls allowed for one video request before giving up (a seek lands on the target at once).
const MAX_VIDEO_PULLS: usize = 512;
/// Requests up to this many frame durations past the last decoded one are served by pulling
/// instead of seeking.
const SEQUENTIAL_WINDOW: u64 = 4;

pub(crate) struct GstSource {
    info: EngineInfo,
    uri: String,
    video: Mutex<Option<VideoDecoder>>,
    audio: Mutex<Option<AudioDecoder>>,
}

impl GstSource {
    pub(crate) fn open(path: &Path) -> Result<Self, Error> {
        let found = crate::backend::discover(path, DEFAULT_DISCOVER_TIMEOUT)?;
        crate::backend::require_element("playbin")?;
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();

        let mut video = None;
        let mut audio_streams = Vec::new();
        for s in &found.streams {
            match &s.kind {
                StreamKind::Video { width, height, frame_rate, is_image, par, .. } if video.is_none() => {
                    if *is_image {
                        return Err(Error::Discover { message: "still images are left to the engine's own decoders".into(), missing_plugins: Vec::new() });
                    }
                    if *width == 0 || *height == 0 {
                        return Err(Error::Discover { message: "video stream without a size".into(), missing_plugins: Vec::new() });
                    }
                    video = Some(VideoStreamInfo {
                        width: *width,
                        height: *height,
                        frame_rate: frame_rate.unwrap_or_default(),
                        par: par.unwrap_or((1, 1)),
                        codec: format!("{} (GStreamer)", s.codec.as_deref().unwrap_or("video")),
                        pixel_format: "RGBA 8-bit (GStreamer)".into(),
                        color: cfa4_color::ColorInfo::SRGB_FULL,
                        has_alpha: false,
                        bitrate: None,
                        hdr: None,
                    });
                }
                StreamKind::Audio { channels, sample_rate, .. } if audio_streams.len() < cfa4_media::MAX_AUDIO_STREAMS => {
                    audio_streams.push(AudioStreamInfo {
                        sample_rate: if *sample_rate == 0 { 48_000 } else { *sample_rate },
                        channels: if (1..=MAX_CHANNELS).contains(channels) { *channels } else { 2 },
                        codec: format!("{} (GStreamer)", s.codec.as_deref().unwrap_or("audio")),
                        bits_per_sample: None,
                    });
                }
                _ => {}
            }
        }
        if video.is_none() && audio_streams.is_empty() {
            return Err(Error::Discover { message: "no video or audio stream".into(), missing_plugins: Vec::new() });
        }
        let duration =
            found.duration.filter(|d| *d > Tick::ZERO).ok_or_else(|| Error::Discover { message: "unknown duration".into(), missing_plugins: Vec::new() })?;
        let info = EngineInfo {
            name,
            kind: if video.is_some() { MediaKind::Movie } else { MediaKind::AudioOnly },
            duration,
            video,
            audio_streams,
            container: format!("{} (GStreamer)", found.container.as_deref().unwrap_or("stream")),
            start_timecode: None,
            file_size: std::fs::metadata(path).ok().map(|m| m.len()),
        };
        Ok(Self { info, uri: found.uri, video: Mutex::new(None), audio: Mutex::new(None) })
    }
}

impl MediaSource for GstSource {
    fn info(&self) -> &EngineInfo {
        &self.info
    }

    fn video_frame(&self, req: FrameRequest) -> cfa4_media::Result<Arc<VideoFrame>> {
        let v = self.info.video.as_ref().ok_or(MediaError::NoStream("video"))?;
        let frame_ns = frame_duration_ns(v.frame_rate);
        // Past the end, the last picture: aim inside the last frame instead of beyond the stream.
        let last_frame_start = nanos_of(self.info.duration).saturating_sub(frame_ns);
        let target = nanos_of(req.time).min(last_frame_start);
        let mut guard = self.video.lock().unwrap_or_else(PoisonError::into_inner);
        if guard.is_none() {
            *guard = Some(VideoDecoder::new(&self.uri)?);
        }
        let Some(dec) = guard.as_mut() else { return Err(MediaError::Decode("GStreamer video decoder missing".into())) };
        let result = dec.frame_at(target, frame_ns, v.par);
        if result.is_err() {
            // A failed pipeline is rebuilt on the next request rather than reused.
            *guard = None;
        }
        result
    }

    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> cfa4_media::Result<AudioBuffer> {
        self.audio_stream(0, start, frames, sample_rate)
    }

    fn audio_stream(&self, stream: usize, start: i64, frames: usize, sample_rate: u32) -> cfa4_media::Result<AudioBuffer> {
        let a = self.info.audio_streams.get(stream).ok_or(MediaError::NoStream("audio"))?;
        if sample_rate == 0 {
            return Err(MediaError::Decode("sample rate 0".into()));
        }
        if frames > MAX_AUDIO_FRAMES {
            return Err(MediaError::Decode(format!("{frames} audio frames requested at once (limit {MAX_AUDIO_FRAMES})")));
        }
        let channels = a.channels;
        let mut guard = self.audio.lock().unwrap_or_else(PoisonError::into_inner);
        if guard.as_ref().is_none_or(|d| d.stream != stream || d.rate != sample_rate) {
            *guard = None; // drop (and stop) the old pipeline first
            *guard = Some(AudioDecoder::new(&self.uri, stream, sample_rate, channels)?);
        }
        let Some(dec) = guard.as_mut() else { return Err(MediaError::Decode("GStreamer audio decoder missing".into())) };
        let result = dec.read(start, frames);
        if result.is_err() {
            *guard = None;
        }
        result
    }
}

/// Engine media time as GStreamer nanoseconds (negative times are clamped to 0).
fn nanos_of(t: Tick) -> u64 {
    u64::try_from(t.to_units_floor(1_000_000_000)).unwrap_or(0)
}

fn frame_duration_ns(rate: FrameRate) -> u64 {
    if rate.num <= 0 || rate.den <= 0 {
        return 40_000_000;
    }
    let ns = (i128::from(rate.den) * 1_000_000_000) / i128::from(rate.num);
    u64::try_from(ns).ok().filter(|n| *n > 0).unwrap_or(40_000_000)
}

/// Builds a `playbin` for `uri` that only plays `flags` (`video` or `audio`) into `sink`.
fn playbin(uri: &str, flags: &str, sink_property: &str, sink: &gst_app::AppSink) -> Result<gst::Element, MediaError> {
    let pipeline = gst::ElementFactory::make("playbin").property("uri", uri).build().map_err(|e| MediaError::Unsupported(format!("GStreamer playbin: {e}")))?;
    pipeline.set_property_from_str("flags", flags);
    pipeline.set_property(sink_property, sink);
    // playbin creates automatic sinks for the stream types it is not asked to play, and probing
    // those (sound cards, display back ends) is slow and can crash on machines without them.
    // Every other sink is a `fakesink`, so only our appsink is ever used.
    for other in ["video-sink", "audio-sink", "text-sink"] {
        if other != sink_property {
            let fake = gst::ElementFactory::make("fakesink")
                .property("sync", false)
                .build()
                .map_err(|e| MediaError::Unsupported(format!("GStreamer fakesink: {e}")))?;
            pipeline.set_property(other, &fake);
        }
    }
    Ok(pipeline)
}

/// Starts `pipeline` and waits until it runs, reporting the pipeline's own error on failure.
fn start(pipeline: &gst::Element) -> Result<(), MediaError> {
    pipeline.set_state(gst::State::Playing).map_err(|_| pipeline_error(pipeline, "cannot start the GStreamer pipeline"))?;
    let (result, _, _) = pipeline.state(WAIT);
    result.map(|_| ()).map_err(|_| pipeline_error(pipeline, "the GStreamer pipeline did not start"))
}

/// Flushing, accurate seek to `ns`, waiting for it to complete.
fn seek(pipeline: &gst::Element, ns: u64) -> Result<(), MediaError> {
    pipeline
        .seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE, gst::ClockTime::from_nseconds(ns))
        .map_err(|_| pipeline_error(pipeline, "GStreamer seek failed"))?;
    let (result, _, _) = pipeline.state(WAIT);
    result.map(|_| ()).map_err(|_| pipeline_error(pipeline, "GStreamer seek did not complete"))
}

/// The first error message on the pipeline's bus, or `fallback`.
fn pipeline_error(pipeline: &gst::Element, fallback: &str) -> MediaError {
    let detail = pipeline.bus().and_then(|bus| bus.pop_filtered(&[gst::MessageType::Error])).and_then(|msg| match msg.view() {
        gst::MessageView::Error(err) => Some(err.error().to_string()),
        _ => None,
    });
    MediaError::Decode(match detail {
        Some(d) => format!("{fallback}: {d}"),
        None => fallback.to_string(),
    })
}

/// Pulls the next sample; `Ok(None)` at the end of the stream.
fn pull(pipeline: &gst::Element, sink: &gst_app::AppSink) -> Result<Option<gst::Sample>, MediaError> {
    match sink.try_pull_sample(WAIT) {
        Some(s) => Ok(Some(s)),
        None if sink.is_eos() => Ok(None),
        None => Err(pipeline_error(pipeline, "GStreamer produced no data in time")),
    }
}

/// Stream time (media time from the file's start) of `sample`'s buffer, in nanoseconds.
fn stream_time(sample: &gst::Sample) -> Option<u64> {
    let pts = sample.buffer()?.pts()?;
    let seg = sample.segment().and_then(|s| s.downcast_ref::<gst::ClockTime>());
    Some(seg.and_then(|s| s.to_stream_time(pts)).unwrap_or(pts).nseconds())
}

struct Decoded {
    start: u64,
    end: u64,
    frame: Arc<VideoFrame>,
}

struct VideoDecoder {
    pipeline: gst::Element,
    sink: gst_app::AppSink,
    last: Option<Decoded>,
}

impl VideoDecoder {
    fn new(uri: &str) -> Result<Self, MediaError> {
        let caps = gst::Caps::builder("video/x-raw").field("format", "RGBA").build();
        let sink = gst_app::AppSink::builder().caps(&caps).sync(false).max_buffers(2).drop(false).build();
        let pipeline = playbin(uri, "video", "video-sink", &sink)?;
        start(&pipeline)?;
        Ok(Self { pipeline, sink, last: None })
    }

    fn frame_at(&mut self, target: u64, frame_ns: u64, par: (u32, u32)) -> cfa4_media::Result<Arc<VideoFrame>> {
        if let Some(l) = &self.last {
            if l.start <= target && target < l.end {
                return Ok(l.frame.clone());
            }
            let window = l.end.saturating_add(frame_ns.saturating_mul(SEQUENTIAL_WINDOW));
            if !(target >= l.end && target < window) {
                seek(&self.pipeline, target)?;
                self.last = None;
            }
        } else if target > 0 {
            seek(&self.pipeline, target)?;
        }
        let mut backed_off = false;
        for _ in 0..MAX_VIDEO_PULLS {
            let Some(sample) = pull(&self.pipeline, &self.sink)? else {
                // Past the end: the last picture stays on screen. When the stream ended before
                // any picture (the container's duration was longer than its video), go back a
                // second once and keep the last picture found.
                if self.last.is_none() && !backed_off && target > 0 {
                    backed_off = true;
                    seek(&self.pipeline, target.saturating_sub(1_000_000_000))?;
                    continue;
                }
                return self.last.as_ref().map(|l| l.frame.clone()).ok_or_else(|| MediaError::Decode("no video frame before the end of the stream".into()));
            };
            let decoded = convert_video(&sample, frame_ns, par)?;
            let done = decoded.end > target;
            let frame = decoded.frame.clone();
            self.last = Some(decoded);
            if done {
                return Ok(frame);
            }
        }
        Err(MediaError::Decode(format!("GStreamer did not reach {target} ns")))
    }
}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

fn convert_video(sample: &gst::Sample, frame_ns: u64, par: (u32, u32)) -> cfa4_media::Result<Decoded> {
    let bad = |m: &str| MediaError::Decode(format!("GStreamer video: {m}"));
    let caps = sample.caps().ok_or_else(|| bad("sample without caps"))?;
    let vinfo = gst_video::VideoInfo::from_caps(caps).map_err(|e| bad(&e.to_string()))?;
    let (w, h) = (vinfo.width(), vinfo.height());
    let row = usize::try_from(w).ok().and_then(|w| w.checked_mul(4)).ok_or_else(|| bad("width"))?;
    let total =
        usize::try_from(h).ok().and_then(|h| h.checked_mul(row)).filter(|n| *n > 0 && *n <= MAX_FRAME_BYTES).ok_or_else(|| bad("picture size out of range"))?;
    let stride = vinfo.stride().first().and_then(|s| usize::try_from(*s).ok()).ok_or_else(|| bad("stride"))?;
    let offset = vinfo.offset().first().copied().ok_or_else(|| bad("plane offset"))?;
    let buffer = sample.buffer().ok_or_else(|| bad("sample without buffer"))?;
    let map = buffer.map_readable().map_err(|e| bad(&e.to_string()))?;
    let data = map.as_slice();
    let mut px = Vec::with_capacity(total);
    for y in 0..usize::try_from(h).unwrap_or(0) {
        let from = y.checked_mul(stride).and_then(|o| o.checked_add(offset)).ok_or_else(|| bad("row offset"))?;
        let to = from.checked_add(row).ok_or_else(|| bad("row end"))?;
        px.extend_from_slice(data.get(from..to).ok_or_else(|| bad("buffer shorter than the picture"))?);
    }
    let start = stream_time(sample).unwrap_or(0);
    let dur = buffer.duration().map(|d| d.nseconds()).filter(|d| *d > 0).unwrap_or(frame_ns);
    let mut frame = VideoFrame::rgba8(w, h, px).with_pts(ticks_from_nanos(start));
    frame.par = par;
    Ok(Decoded { start, end: start.saturating_add(dur), frame: Arc::new(frame) })
}

struct AudioDecoder {
    stream: usize,
    rate: u32,
    channels: usize,
    pipeline: gst::Element,
    sink: gst_app::AppSink,
    /// Decoded interleaved samples not yet consumed, starting at sample index `carry_start`.
    carry: Vec<f32>,
    carry_start: Option<i64>,
    eos: bool,
}

impl AudioDecoder {
    fn new(uri: &str, stream: usize, rate: u32, channels: u32) -> Result<Self, MediaError> {
        let rate_i = i32::try_from(rate).map_err(|_| MediaError::Decode(format!("sample rate {rate}")))?;
        let ch_i = i32::try_from(channels.clamp(1, MAX_CHANNELS)).unwrap_or(2);
        let caps =
            gst::Caps::builder("audio/x-raw").field("format", "F32LE").field("layout", "interleaved").field("rate", rate_i).field("channels", ch_i).build();
        let sink = gst_app::AppSink::builder().caps(&caps).sync(false).max_buffers(4).drop(false).build();
        let pipeline = playbin(uri, "audio", "audio-sink", &sink)?;
        pipeline.set_property("current-audio", i32::try_from(stream).unwrap_or(0));
        start(&pipeline)?;
        Ok(Self { stream, rate, channels: usize::try_from(ch_i).unwrap_or(2), pipeline, sink, carry: Vec::new(), carry_start: None, eos: false })
    }

    fn carry_frames(&self) -> i64 {
        i64::try_from(self.carry.len() / self.channels.max(1)).unwrap_or(i64::MAX)
    }

    fn read(&mut self, start: i64, frames: usize) -> cfa4_media::Result<AudioBuffer> {
        let start = start.max(0);
        let want_end = start.saturating_add(i64::try_from(frames).unwrap_or(i64::MAX));
        // Continue the current position when the request starts inside (or right after) what is
        // buffered; otherwise seek.
        let contiguous = self.carry_start.is_some_and(|cs| cs <= start && start <= cs.saturating_add(self.carry_frames()));
        if !contiguous {
            let ns = u64::try_from(i128::from(start) * 1_000_000_000 / i128::from(self.rate)).unwrap_or(0);
            seek(&self.pipeline, ns)?;
            self.carry.clear();
            self.carry_start = None;
            self.eos = false;
        }
        let max_pulls = frames / 64 + 64;
        let mut pulls = 0;
        while !self.eos && self.carry_start.is_none_or(|cs| cs.saturating_add(self.carry_frames()) < want_end) && pulls < max_pulls {
            pulls += 1;
            let Some(sample) = pull(&self.pipeline, &self.sink)? else {
                self.eos = true;
                break;
            };
            self.append(&sample, start)?;
        }
        let mut out = AudioBuffer::silence(self.rate, self.channels, frames);
        if let Some(cs) = self.carry_start {
            for (i, f) in (start..want_end).enumerate() {
                let Some(rel) = f.checked_sub(cs).and_then(|r| usize::try_from(r).ok()) else { continue };
                for (c, ch) in out.channels.iter_mut().enumerate() {
                    if let (Some(dst), Some(src)) = (ch.get_mut(i), self.carry.get(rel * self.channels + c)) {
                        *dst = *src;
                    }
                }
            }
        }
        // Keep only what a sequential next request can use.
        self.drop_before(want_end);
        Ok(out)
    }

    fn append(&mut self, sample: &gst::Sample, want_start: i64) -> cfa4_media::Result<()> {
        let buffer = sample.buffer().ok_or_else(|| MediaError::Decode("GStreamer audio: sample without buffer".into()))?;
        let map = buffer.map_readable().map_err(|e| MediaError::Decode(format!("GStreamer audio: {e}")))?;
        let mut samples: Vec<f32> = map.as_slice().chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        let ch = self.channels.max(1);
        samples.truncate(samples.len() - samples.len() % ch);
        let first = stream_time(sample).map(|ns| i64::try_from(i128::from(ns) * i128::from(self.rate) / 1_000_000_000).unwrap_or(i64::MAX));
        let Some(cs) = self.carry_start else {
            self.carry_start = Some(first.unwrap_or(want_start));
            self.carry = samples;
            self.drop_before(want_start);
            return Ok(());
        };
        let expected = cs.saturating_add(self.carry_frames());
        match first {
            // A gap in the stream: fill it with silence (bounded).
            Some(f) if f > expected => {
                let gap = usize::try_from(f - expected).unwrap_or(0).min(MAX_AUDIO_FRAMES);
                self.carry.resize(self.carry.len() + gap * ch, 0.0);
            }
            // Overlap with what is buffered: skip the repeated part.
            Some(f) if f < expected => {
                let skip = usize::try_from(expected - f).unwrap_or(usize::MAX).saturating_mul(ch).min(samples.len());
                samples.drain(..skip);
            }
            _ => {}
        }
        self.carry.extend_from_slice(&samples);
        Ok(())
    }

    fn drop_before(&mut self, frame: i64) {
        let Some(cs) = self.carry_start else { return };
        if frame <= cs {
            return;
        }
        let n = usize::try_from(frame - cs).unwrap_or(usize::MAX).saturating_mul(self.channels).min(self.carry.len());
        self.carry.drain(..n);
        self.carry_start = Some(cs.saturating_add(i64::try_from(n / self.channels.max(1)).unwrap_or(0)));
    }
}

impl Drop for AudioDecoder {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}
