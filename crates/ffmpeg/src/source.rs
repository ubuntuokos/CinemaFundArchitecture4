//! An engine media source decoded by FFmpeg through rsmedia (ADR 0005, step 1).
//!
//! Video is decoded at the stream's **native** layout and bit depth: rsmedia converts every frame
//! to one output format (8-bit 4:2:0 unless told otherwise), so the source asks for the stream's
//! own planar Y'CbCr format and hands the planes to the engine as `Yuv8` / `Yuv16`; any other
//! layout (RGB, NV12, palette, gray…) is converted to RGBA 8-bit. Audio is decoded to planar
//! 32-bit float and resampled by FFmpeg to the rate the engine asks for.
//!
//! Requests near the last one decode forward; others seek back to the key frame at or before the
//! target and decode up to it, so every request returns exactly the frame displayed at its time.
//! Every failure is a `MediaError`; every call into rsmedia runs under `catch_unwind`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use cfa4_color::{ColorInfo, Matrix, Primaries, Range, Transfer};
use cfa4_frame::{AudioBuffer, Chroma, PixelData, VideoFrame};
use cfa4_media::{AudioStreamInfo, FrameRequest, MediaError, MediaInfo as EngineInfo, MediaKind, MediaSource, VideoStreamInfo};
use cfa4_time::{FrameRate, TICKS_PER_SECOND, Tick};
use rsmedia::frame::MediaFrame;
use rsmedia::stream::StreamInfo as FfStream;
use rsmedia::{AVSeekFlag, Decoder, DecoderBuilder, MediaType, PixelFormat, SampleFormat, Seekable, StreamReader};

use crate::Error;
use crate::backend::{self, AV_TIME_BASE, NOPTS};

/// Largest picture accepted (pixels); larger streams are treated as hostile.
const MAX_PIXELS: u64 = 1 << 28;
/// Most audio frames one request may ask for (about 5.8 minutes at 48 kHz).
const MAX_AUDIO_FRAMES: usize = 1 << 24;
/// Most channels decoded per audio stream.
const MAX_CHANNELS: u32 = 32;
/// Frames decoded for one video request before giving up (long-GOP media needs a few hundred).
const MAX_VIDEO_PULLS: usize = 2048;
/// Audio frames (FFmpeg packets' worth) decoded for one request before giving up.
const MAX_AUDIO_PULLS: usize = 16_384;
/// Requests up to this many frame durations past the last decoded frame decode forward.
const SEQUENTIAL_WINDOW: i64 = 8;
/// Audio kept before the requested position, in seconds (bounded memory, cheap small rewinds).
const AUDIO_KEEP_SECONDS: u32 = 2;
/// Image demuxers: stills and image sequences are left to the engine's own decoders.
const STILL_DEMUXERS: &[&str] = &["image2", "image2pipe", "png_pipe", "jpeg_pipe", "webp_pipe", "tiff_pipe", "bmp_pipe", "dpx_pipe", "exr_pipe"];

fn media_err(e: Error) -> MediaError {
    match e {
        Error::InvalidInput(m) => MediaError::Io(m),
        other => MediaError::Decode(other.to_string()),
    }
}

fn rs_err(context: &str, e: impl std::fmt::Display) -> MediaError {
    MediaError::Decode(format!("FFmpeg {context}: {e}"))
}

/// Runs one decoding step, turning a panic inside rsmedia into an error.
fn guarded<T>(f: impl FnOnce() -> cfa4_media::Result<T>) -> cfa4_media::Result<T> {
    crate::guarded(|| f().map_err(|e| Error::Media(e.to_string()))).map_err(media_err)
}

/// Time base of a stream as `(num, den)`, or `None` when invalid.
fn time_base(s: &FfStream) -> Option<(i64, i64)> {
    backend::rational(s.time_base.num, s.time_base.den)
}

/// `ts` (in `tb` units) as engine ticks, saturating.
fn ts_to_tick(ts: i64, tb: (i64, i64)) -> Tick {
    let v = i128::from(ts).saturating_mul(i128::from(tb.0)).saturating_mul(i128::from(TICKS_PER_SECOND)) / i128::from(tb.1.max(1));
    Tick(i64::try_from(v).unwrap_or(if v < 0 { i64::MIN } else { i64::MAX }))
}

/// Engine ticks as a timestamp in `tb` units (rounded down), saturating.
fn tick_to_ts(t: Tick, tb: (i64, i64)) -> i64 {
    let v = i128::from(t.0).saturating_mul(i128::from(tb.1)) / i128::from(tb.0.max(1)).saturating_mul(i128::from(TICKS_PER_SECOND));
    i64::try_from(v).unwrap_or(if v < 0 { i64::MIN } else { i64::MAX })
}

/// Microseconds as a timestamp in `tb` units.
fn us_to_ts(us: i64, tb: (i64, i64)) -> i64 {
    let v = i128::from(us).saturating_mul(i128::from(tb.1)) / i128::from(tb.0.max(1)).saturating_mul(i128::from(AV_TIME_BASE));
    i64::try_from(v).unwrap_or(0)
}

/// How decoded frames reach the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Output {
    Yuv8(Chroma),
    Yuv16(Chroma, u32),
    Rgba,
}

/// The output for an FFmpeg pixel format name: its own planar Y'CbCr layout when the engine has
/// one (`yuv420p`, `yuvj422p`, `yuv422p10le`, `yuv444p12le`…), RGBA 8-bit otherwise.
fn output_for(pix_fmt: &str) -> (Output, String) {
    let name = pix_fmt.strip_prefix("yuvj").map(|r| format!("yuv{r}")).unwrap_or_else(|| pix_fmt.to_string());
    let chroma = if name.starts_with("yuv420p") {
        Some(Chroma::C420)
    } else if name.starts_with("yuv422p") {
        Some(Chroma::C422)
    } else if name.starts_with("yuv444p") {
        Some(Chroma::C444)
    } else {
        None
    };
    if let Some(chroma) = chroma {
        let rest = name.get(7..).unwrap_or("");
        if rest.is_empty() {
            return (Output::Yuv8(chroma), name);
        }
        // only little-endian high-bit-depth layouts: the engine stores native u16 samples
        if let Some(bits) = rest.strip_suffix("le").and_then(|b| b.parse::<u32>().ok()).filter(|b| (9..=16).contains(b)) {
            return (Output::Yuv16(chroma, bits), name);
        }
    }
    (Output::Rgba, "rgba".into())
}

fn chroma_size(w: usize, h: usize, chroma: Chroma) -> (usize, usize) {
    let (sx, sy) = chroma.shifts();
    (w.div_ceil(1 << sx), h.div_ceil(1 << sy))
}

/// Colour metadata from the stream's H.273 codes (FFmpeg's enums use the same numbers).
fn color_info(s: &FfStream, height: u32, full_range_format: bool) -> ColorInfo {
    let code = |v: u32| u8::try_from(v).unwrap_or(2);
    let sd = height <= 576;
    let matrix = Matrix::from_code(code(s.color_space as u32)).unwrap_or(if sd { Matrix::Bt601 } else { Matrix::Bt709 });
    let transfer = Transfer::from_code(code(s.color_transfer as u32)).unwrap_or(Transfer::Bt709);
    let primaries = match code(s.color_primaries as u32) {
        1 => Primaries::Bt709,
        5 => Primaries::Bt601_625,
        6 | 7 => Primaries::Bt601_525,
        9 => Primaries::Bt2020,
        12 => Primaries::P3D65,
        _ if sd && height == 576 => Primaries::Bt601_625,
        _ if sd => Primaries::Bt601_525,
        _ => Primaries::Bt709,
    };
    // AVCOL_RANGE_JPEG = 2 is full range; the yuvj* formats are full range by definition
    let range = if s.color_range as u32 == 2 || full_range_format { Range::Full } else { Range::Limited };
    ColorInfo { matrix, transfer, primaries, range }
}

/// The video stream the source decodes.
#[derive(Clone)]
struct VideoMeta {
    tb: (i64, i64),
    /// Container start time in this stream's time base (media time 0).
    origin: i64,
    output: Output,
    pix_fmt: PixelFormat,
    width: u32,
    height: u32,
    color: ColorInfo,
    par: (u32, u32),
    /// One frame in the stream's time base (at least 1).
    frame_ts: i64,
}

#[derive(Clone)]
struct AudioMeta {
    tb: (i64, i64),
    origin: i64,
    /// Container start time in microseconds (media time 0), for frames in another time base.
    start_us: i64,
    channels: u32,
}

pub(crate) struct FfSource {
    info: EngineInfo,
    path: PathBuf,
    vmeta: Option<VideoMeta>,
    ameta: Option<AudioMeta>,
    video: Mutex<Option<VideoState>>,
    audio: Mutex<Option<AudioState>>,
}

impl FfSource {
    pub(crate) fn open(path: &Path) -> Result<Self, Error> {
        let (_reader, probed) = backend::probe_file(path)?;
        if STILL_DEMUXERS.contains(&probed.container.as_str()) {
            return Err(Error::Media("still images are left to the engine's own decoders".into()));
        }
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();

        let mut video = None;
        let mut vmeta = None;
        if let Some(s) = probed.streams.iter().find(|s| s.media_type == MediaType::VIDEO && s.disposition & 0x400 == 0) {
            // 0x400 = AV_DISPOSITION_ATTACHED_PIC: cover art is not the movie
            let (w, h) = (u32::try_from(s.width).unwrap_or(0), u32::try_from(s.height).unwrap_or(0));
            if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS {
                return Err(Error::Media(format!("video stream of {w}×{h} pixels")));
            }
            let tb = time_base(s).ok_or_else(|| Error::Media("video stream without a time base".into()))?;
            let native = backend::pixel_format_name(s);
            let (output, out_name) = output_for(&native);
            let pix_fmt = s.format.into_pixel().filter(|_| output != Output::Rgba).unwrap_or(PixelFormat::RGBA);
            let rate = backend::rational(s.avg_frame_rate.num, s.avg_frame_rate.den).or_else(|| backend::rational(s.frame_rate.num, s.frame_rate.den));
            let frame_rate = rate.map(|(num, den)| FrameRate { num, den }).unwrap_or_default();
            let frame_ts =
                rate.map_or(1, |(num, den)| i64::try_from(i128::from(den) * i128::from(tb.1) / (i128::from(num) * i128::from(tb.0))).unwrap_or(1).max(1));
            let par = backend::rational(s.sample_aspect_ratio.num, s.sample_aspect_ratio.den)
                .and_then(|(n, d)| Some((u32::try_from(n).ok()?, u32::try_from(d).ok()?)))
                .unwrap_or((1, 1));
            let color = if output == Output::Rgba { ColorInfo::SRGB_FULL } else { color_info(s, h, native.starts_with("yuvj")) };
            let label = match output {
                Output::Yuv8(c) => format!("YUV {} 8-bit ({native}, FFmpeg)", c.label()),
                Output::Yuv16(c, b) => format!("YUV {} {b}-bit ({native}, FFmpeg)", c.label()),
                Output::Rgba => format!("RGBA 8-bit (from {native}, FFmpeg)"),
            };
            log::debug!("{name}: FFmpeg decodes {native} as {out_name}");
            video = Some(VideoStreamInfo {
                width: w,
                height: h,
                frame_rate,
                par,
                codec: format!("{} (FFmpeg)", backend::codec_name(s)),
                pixel_format: label,
                color,
                has_alpha: false,
                bitrate: u64::try_from(s.bit_rate).ok().filter(|b| *b > 0),
                hdr: None,
            });
            vmeta = Some(VideoMeta { tb, origin: us_to_ts(probed.start_us, tb), output, pix_fmt, width: w, height: h, color, par, frame_ts });
        }

        let mut audio_streams = Vec::new();
        let mut ameta = None;
        // rsmedia decodes the container's best audio stream; that one is reported (stream 0)
        if let Some(s) = probed.streams.iter().find(|s| s.media_type == MediaType::AUDIO) {
            let channels = u32::try_from(s.channel_layout.nb_channels).unwrap_or(0);
            let channels = if (1..=MAX_CHANNELS).contains(&channels) { channels } else { 2 };
            let sample_rate = u32::try_from(s.sample_rate).ok().filter(|r| (1..=768_000).contains(r)).unwrap_or(48_000);
            if let Some(tb) = time_base(s) {
                audio_streams.push(AudioStreamInfo {
                    sample_rate,
                    channels,
                    codec: format!("{} (FFmpeg)", backend::codec_name(s)),
                    bits_per_sample: u32::try_from(s.bits_per_sample).ok().filter(|b| *b > 0),
                });
                ameta = Some(AudioMeta { tb, origin: us_to_ts(probed.start_us, tb), start_us: probed.start_us, channels });
            }
        }
        if video.is_none() && audio_streams.is_empty() {
            return Err(Error::Media("no video or audio stream FFmpeg can decode".into()));
        }
        let duration = probed
            .duration
            .or_else(|| {
                let s = probed.streams.iter().find(|s| s.duration > 0 && s.duration != NOPTS)?;
                Some(ts_to_tick(s.duration, time_base(s)?))
            })
            .filter(|d| *d > Tick::ZERO)
            .ok_or_else(|| Error::Media("unknown duration".into()))?;
        let info = EngineInfo {
            name,
            kind: if video.is_some() { MediaKind::Movie } else { MediaKind::AudioOnly },
            duration,
            video,
            audio_streams,
            container: format!("{} (FFmpeg)", probed.container),
            start_timecode: None,
            file_size: std::fs::metadata(path).ok().map(|m| m.len()),
        };
        Ok(Self { info, path: path.to_path_buf(), vmeta, ameta, video: Mutex::new(None), audio: Mutex::new(None) })
    }
}

impl MediaSource for FfSource {
    fn info(&self) -> &EngineInfo {
        &self.info
    }

    fn video_frame(&self, req: FrameRequest) -> cfa4_media::Result<Arc<VideoFrame>> {
        let meta = self.vmeta.as_ref().ok_or(MediaError::NoStream("video"))?;
        // past the end, the last picture: aim inside the last frame
        let last_start = self.info.duration.0.saturating_sub(ts_to_tick(meta.frame_ts, meta.tb).0).max(0);
        let target = meta.origin.saturating_add(tick_to_ts(Tick(req.time.0.clamp(0, last_start)), meta.tb));
        let mut guard = self.video.lock().unwrap_or_else(PoisonError::into_inner);
        if guard.is_none() {
            *guard = Some(guarded(|| VideoState::new(&self.path, meta))?);
        }
        let Some(state) = guard.as_mut() else { return Err(MediaError::Decode("FFmpeg video decoder missing".into())) };
        let result = guarded(|| state.frame_at(target, meta));
        if result.is_err() {
            // a failed decoder is rebuilt on the next request rather than reused
            *guard = None;
        }
        result
    }

    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> cfa4_media::Result<AudioBuffer> {
        let meta = self.ameta.as_ref().ok_or(MediaError::NoStream("audio"))?;
        if sample_rate == 0 || sample_rate > 768_000 {
            return Err(MediaError::Decode(format!("sample rate {sample_rate}")));
        }
        if frames > MAX_AUDIO_FRAMES {
            return Err(MediaError::Decode(format!("{frames} audio frames requested at once (limit {MAX_AUDIO_FRAMES})")));
        }
        let channels = meta.channels as usize;
        if frames == 0 {
            return Ok(AudioBuffer::silence(sample_rate, channels, 0));
        }
        let mut guard = self.audio.lock().unwrap_or_else(PoisonError::into_inner);
        if guard.as_ref().is_none_or(|a| a.rate != sample_rate) {
            *guard = Some(guarded(|| AudioState::new(&self.path, meta, sample_rate))?);
        }
        let Some(state) = guard.as_mut() else { return Err(MediaError::Decode("FFmpeg audio decoder missing".into())) };
        let result = guarded(|| state.read(start, frames, meta));
        if result.is_err() {
            *guard = None;
        }
        result
    }
}

struct VideoState {
    reader: StreamReader,
    decoder: Decoder,
    /// The newest frame at or before the last target, with its timestamp.
    last: Option<(i64, Arc<VideoFrame>)>,
    /// A decoded frame after the last target, kept for the next request.
    pending: Option<(i64, Arc<VideoFrame>)>,
    /// The decoder reached the end of the stream.
    ended: bool,
}

impl VideoState {
    fn new(path: &Path, meta: &VideoMeta) -> cfa4_media::Result<Self> {
        let reader = backend::open_reader(path).map_err(media_err)?;
        let decoder = DecoderBuilder::new(MediaType::VIDEO).with_pix_fmt(meta.pix_fmt).build_from_reader(&reader).map_err(|e| rs_err("video decoder", e))?;
        Ok(Self { reader, decoder, last: None, pending: None, ended: false })
    }

    fn frame_at(&mut self, target: i64, meta: &VideoMeta) -> cfa4_media::Result<Arc<VideoFrame>> {
        let window = meta.frame_ts.saturating_mul(SEQUENTIAL_WINDOW);
        let near = match (&self.last, &self.pending) {
            (Some((lp, _)), _) => target >= *lp && target <= lp.saturating_add(window),
            (None, Some((pp, _))) => target >= *pp && target <= pp.saturating_add(window),
            (None, None) => false,
        };
        if let (Some((lp, f)), Some((pp, _))) = (&self.last, &self.pending) {
            if target >= *lp && target < *pp {
                return Ok(f.clone());
            }
        }
        if !near {
            self.seek(target, meta)?;
        }
        for _ in 0..MAX_VIDEO_PULLS {
            let next = match self.pending.take() {
                Some(p) => Some(p),
                None if self.ended => None,
                None => self.decode_next(meta)?,
            };
            let Some((ts, frame)) = next else {
                self.ended = true;
                break;
            };
            if ts <= target {
                self.last = Some((ts, frame));
            } else {
                self.pending = Some((ts, frame));
                break;
            }
        }
        if let Some((_, f)) = &self.last {
            return Ok(f.clone());
        }
        // a target before the first frame shows the first frame
        if let Some((_, f)) = &self.pending {
            return Ok(f.clone());
        }
        Err(MediaError::Decode("FFmpeg decoded no video frame".into()))
    }

    fn seek(&mut self, target: i64, _meta: &VideoMeta) -> cfa4_media::Result<()> {
        let stream = self.decoder.stream_index();
        self.reader.seek_to_frame(stream, target, AVSeekFlag::BACKWARD).map_err(|e| rs_err("seek", e))?;
        self.decoder.reset().map_err(|e| rs_err("decoder reset", e))?;
        self.last = None;
        self.pending = None;
        self.ended = false;
        Ok(())
    }

    fn decode_next(&mut self, meta: &VideoMeta) -> cfa4_media::Result<Option<(i64, Arc<VideoFrame>)>> {
        let previous = self.last.as_ref().map(|(t, _)| *t);
        let (ts, frame) = match meta.output {
            Output::Yuv16(chroma, bits) => {
                let Some(f) = self.decoder.decode::<u16>(&mut self.reader).map_err(|e| rs_err("video decoding", e))? else { return Ok(None) };
                let ts = frame_ts(&f, previous, meta);
                let planes = yuv_planes(&f, meta, chroma)?;
                (ts, PixelData::Yuv16 { planes, chroma, bits, alpha: None })
            }
            Output::Yuv8(chroma) => {
                let Some(f) = self.decoder.decode::<u8>(&mut self.reader).map_err(|e| rs_err("video decoding", e))? else { return Ok(None) };
                let ts = frame_ts(&f, previous, meta);
                let planes = yuv_planes(&f, meta, chroma)?;
                (ts, PixelData::Yuv8 { planes, chroma, alpha: None })
            }
            Output::Rgba => {
                let Some(f) = self.decoder.decode::<u8>(&mut self.reader).map_err(|e| rs_err("video decoding", e))? else { return Ok(None) };
                let ts = frame_ts(&f, previous, meta);
                let packed = f.data.as_packed().ok_or_else(|| MediaError::Decode("FFmpeg RGBA frame is not interleaved".into()))?;
                let expected = (meta.width as usize).saturating_mul(meta.height as usize).saturating_mul(4);
                let data: Vec<u8> = packed.iter().copied().collect();
                if data.len() != expected {
                    return Err(MediaError::Decode(format!("FFmpeg RGBA frame of {} bytes, expected {expected}", data.len())));
                }
                (ts, PixelData::Rgba8(Arc::new(data)))
            }
        };
        let pts = ts_to_tick(ts.saturating_sub(meta.origin), meta.tb);
        let color = if meta.output == Output::Rgba { ColorInfo::SRGB_FULL } else { meta.color };
        Ok(Some((ts, Arc::new(VideoFrame { width: meta.width, height: meta.height, data: frame, color, par: meta.par, pts }))))
    }
}

/// A frame's timestamp: its best-effort timestamp, else one frame after the previous one.
fn frame_ts<T>(f: &MediaFrame<T>, previous: Option<i64>, meta: &VideoMeta) -> i64 {
    [f.best_effort_timestamp, f.pts].into_iter().find(|t| *t != NOPTS).unwrap_or_else(|| previous.map_or(meta.origin, |p| p.saturating_add(meta.frame_ts)))
}

/// The three planes of a planar Y'CbCr frame, checked against the engine's plane sizes.
fn yuv_planes<T: Copy>(f: &MediaFrame<T>, meta: &VideoMeta, chroma: Chroma) -> cfa4_media::Result<[Arc<Vec<T>>; 3]> {
    let planes = f.data.as_planes().ok_or_else(|| MediaError::Decode("FFmpeg Y'CbCr frame is not planar".into()))?;
    let (w, h) = (meta.width as usize, meta.height as usize);
    let (cw, ch) = chroma_size(w, h, chroma);
    let sizes = [w * h, cw * ch, cw * ch];
    let mut out: Vec<Arc<Vec<T>>> = Vec::with_capacity(3);
    for (i, expected) in sizes.iter().enumerate() {
        let plane = planes.get(i).ok_or_else(|| MediaError::Decode(format!("FFmpeg frame has no plane {i}")))?;
        let samples: Vec<T> = plane.iter().copied().collect();
        if samples.len() != *expected {
            return Err(MediaError::Decode(format!("FFmpeg plane {i} has {} samples, expected {expected} (frame size changed mid-stream?)", samples.len())));
        }
        out.push(Arc::new(samples));
    }
    let mut it = out.into_iter();
    match (it.next(), it.next(), it.next()) {
        (Some(y), Some(u), Some(v)) => Ok([y, u, v]),
        _ => Err(MediaError::Decode("FFmpeg frame planes missing".into())),
    }
}

struct AudioState {
    rate: u32,
    reader: StreamReader,
    decoder: Decoder,
    /// Decoded samples, one Vec per channel, starting at sample index `start` (at `rate`).
    buf: Vec<Vec<f32>>,
    start: Option<i64>,
    ended: bool,
}

impl AudioState {
    fn new(path: &Path, meta: &AudioMeta, rate: u32) -> cfa4_media::Result<Self> {
        let reader = backend::open_reader(path).map_err(media_err)?;
        let decoder = DecoderBuilder::new(MediaType::AUDIO)
            .with_sample_fmt(SampleFormat::FLTP)
            .with_filters(vec![rsmedia::filter::audio::resample(meta.channels, rate, SampleFormat::FLTP)])
            .build_from_reader(&reader)
            .map_err(|e| rs_err("audio decoder", e))?;
        Ok(Self { rate, reader, decoder, buf: vec![Vec::new(); meta.channels as usize], start: None, ended: false })
    }

    fn end(&self) -> Option<i64> {
        let len = i64::try_from(self.buf.first().map_or(0, Vec::len)).unwrap_or(i64::MAX);
        self.start.map(|s| s.saturating_add(len))
    }

    fn read(&mut self, start: i64, frames: usize, meta: &AudioMeta) -> cfa4_media::Result<AudioBuffer> {
        let want_end = start.saturating_add(i64::try_from(frames).unwrap_or(i64::MAX));
        let usable = match (self.start, self.end()) {
            // within what is buffered, or a little past it (sequential playback)
            (Some(s), Some(e)) => start >= s && start <= e.saturating_add(i64::from(self.rate) / 2),
            _ => false,
        };
        if !usable {
            self.seek(start, meta)?;
        }
        let mut pulls = 0;
        while !self.ended && self.end().is_none_or(|e| e < want_end) && pulls < MAX_AUDIO_PULLS {
            pulls += 1;
            self.decode_next(meta)?;
        }
        let channels = self.buf.len();
        let mut out = AudioBuffer::silence(self.rate, channels, frames);
        if let Some(s) = self.start {
            for (dst, src) in out.channels.iter_mut().zip(&self.buf) {
                for (i, d) in dst.iter_mut().enumerate() {
                    let idx = start.saturating_add(i64::try_from(i).unwrap_or(i64::MAX)).saturating_sub(s);
                    if let Some(v) = usize::try_from(idx).ok().and_then(|k| src.get(k)) {
                        *d = *v;
                    }
                }
            }
            // keep a little before the position, drop the rest (bounded memory)
            let keep_from = start.saturating_sub(i64::from(self.rate) * i64::from(AUDIO_KEEP_SECONDS));
            if keep_from > s {
                let drop = usize::try_from(keep_from - s).unwrap_or(0);
                for c in &mut self.buf {
                    c.drain(..drop.min(c.len()));
                }
                self.start = Some(s.saturating_add(i64::try_from(drop).unwrap_or(0)));
            }
        }
        Ok(out)
    }

    fn seek(&mut self, start: i64, meta: &AudioMeta) -> cfa4_media::Result<()> {
        let target = meta.origin.saturating_add(tick_to_ts(Tick::from_units(start.max(0), i64::from(self.rate)), meta.tb));
        let stream = self.decoder.stream_index();
        self.reader.seek_to_frame(stream, target, AVSeekFlag::BACKWARD).map_err(|e| rs_err("audio seek", e))?;
        self.decoder.reset().map_err(|e| rs_err("audio decoder reset", e))?;
        for c in &mut self.buf {
            c.clear();
        }
        self.start = None;
        self.ended = false;
        Ok(())
    }

    fn decode_next(&mut self, meta: &AudioMeta) -> cfa4_media::Result<()> {
        let Some(f) = self.decoder.decode::<f32>(&mut self.reader).map_err(|e| rs_err("audio decoding", e))? else {
            self.ended = true;
            return Ok(());
        };
        let planes = f.data.as_planes().ok_or_else(|| MediaError::Decode("FFmpeg audio frame is not planar".into()))?;
        if self.start.is_none() {
            // the first frame after a seek places the buffer. Its `pts` is in the frame's own
            // time base, the resampler's output (1/rate); `best_effort_timestamp` still counts in
            // the stream's time base from before the filter, so it is the fallback only.
            let (ts, tb) = match backend::rational(f.time_base.num, f.time_base.den) {
                Some(tb) if f.pts != NOPTS => (f.pts, tb),
                _ => ([f.best_effort_timestamp, f.pts].into_iter().find(|t| *t != NOPTS).unwrap_or(meta.origin), meta.tb),
            };
            let at = ts_to_tick(ts, tb).0.saturating_sub(Tick::from_units(meta.start_us, AV_TIME_BASE).0);
            self.start = Some(tick_to_ts(Tick(at), (1, i64::from(self.rate))));
        }
        for (c, dst) in self.buf.iter_mut().enumerate() {
            match planes.get(c) {
                Some(p) => dst.extend(p.iter().copied()),
                // fewer channels than announced: keep the channels aligned with silence
                None => {
                    let n = planes.first().map_or(0, |p| p.len());
                    dst.extend(std::iter::repeat_n(0.0, n));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_planar_formats_stay_native() {
        assert_eq!(output_for("yuv420p").0, Output::Yuv8(Chroma::C420));
        assert_eq!(output_for("yuvj422p").0, Output::Yuv8(Chroma::C422));
        assert_eq!(output_for("yuv422p10le").0, Output::Yuv16(Chroma::C422, 10));
        assert_eq!(output_for("yuv420p10le").0, Output::Yuv16(Chroma::C420, 10));
        assert_eq!(output_for("yuv444p12le").0, Output::Yuv16(Chroma::C444, 12));
        assert_eq!(output_for("yuv444p16le").0, Output::Yuv16(Chroma::C444, 16));
    }

    #[test]
    fn other_layouts_become_rgba() {
        for f in ["nv12", "p010le", "rgb24", "gray", "pal8", "yuv422p10be", "yuva420p", "yuv410p", "", "yuv420p99le"] {
            assert_eq!(output_for(f).0, Output::Rgba, "{f}");
        }
    }

    #[test]
    fn timestamps_round_trip_and_saturate() {
        let tb = (1, 12_800);
        assert_eq!(ts_to_tick(12_800, tb), Tick(TICKS_PER_SECOND));
        assert_eq!(tick_to_ts(Tick(TICKS_PER_SECOND), tb), 12_800);
        let ntsc = (1001, 30_000);
        assert_eq!(tick_to_ts(ts_to_tick(1234, ntsc), ntsc), 1234);
        assert_eq!(ts_to_tick(i64::MAX, (i32::MAX.into(), 1)), Tick(i64::MAX));
        assert_eq!(ts_to_tick(i64::MIN, (i32::MAX.into(), 1)), Tick(i64::MIN));
        assert_eq!(us_to_ts(1_000_000, tb), 12_800);
        // hostile time bases from a file never divide by zero or overflow
        assert_eq!(tick_to_ts(Tick(i64::MAX), (0, 0)), 0);
        let _ = ts_to_tick(i64::MIN, (i64::MAX, 1));
    }

    #[test]
    fn chroma_sizes_round_up_like_ffmpeg() {
        assert_eq!(chroma_size(1281, 721, Chroma::C420), (641, 361));
        assert_eq!(chroma_size(1281, 721, Chroma::C422), (641, 721));
        assert_eq!(chroma_size(1281, 721, Chroma::C444), (1281, 721));
    }
}
