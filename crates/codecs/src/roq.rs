//! id RoQ files (`.roq`): `cfa4-roq` finds the packets and decodes the video (full range
//! YUV 4:4:4) and the DPCM sound (CB-25.1).
//!
//! Every picture is coded against the one before it (and the blocks it leaves alone keep the
//! pixels of the picture before that), so decoding runs forward from the start. The decoder
//! keeps a copy of its state at every 128th picture it passes ([`crate::ckpt`]) and the sample
//! table reports those as sync samples: a seek decodes from the nearest one passed. The sound is
//! decoded whole at open (22.05 kHz mono or stereo, small next to the video).

use std::sync::Arc;

use cfa4_color::{ColorInfo, Matrix, Range};
use cfa4_frame::{AudioBuffer, Chroma, PixelData, VideoFrame};
use cfa4_media::{AudioStreamInfo, FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, SharedSource, VideoStreamInfo};
use cfa4_roq::{Decoder, Stream};
use cfa4_time::{FrameRate, Tick};

use crate::ckpt::Checkpoints;
use crate::gop::{GopCache, VideoSamples};
use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// Pictures between decoder checkpoints.
const SPACING: usize = 128;
/// Most decoded sound values (all channels) kept: 2²⁷.
const MAX_AUDIO_VALUES: usize = 1 << 27;

pub fn sniff(head: &[u8]) -> bool {
    cfa4_roq::probe(head)
}

struct Video {
    width: u32,
    height: u32,
    rate: FrameRate,
    cache: GopCache,
    checkpoints: Arc<Checkpoints<Decoder>>,
}

struct Audio {
    channels: usize,
    samples: Vec<Vec<f32>>,
}

pub struct RoqSource {
    info: MediaInfo,
    bytes: Arc<[u8]>,
    stream: Stream,
    video: Option<Video>,
    audio: Option<Audio>,
}

fn color() -> ColorInfo {
    // YUVJ444P as ffmpeg takes it: full range, BT.601 matrix
    ColorInfo { matrix: Matrix::Bt601, range: Range::Full, ..ColorInfo::REC709 }
}

impl RoqSource {
    pub fn open(name: &str, bytes: Arc<[u8]>) -> Result<Self> {
        let stream = cfa4_roq::parse(&bytes).map_err(|e| CodecError::Container(format!("{name}: {e}")))?;
        let mut duration = Tick::ZERO;
        let mut video = None;
        let mut video_info = None;
        if !stream.video.is_empty() {
            let dec = Decoder::new(stream.width, stream.height).map_err(|e| CodecError::Unsupported(format!("{name}: {e}")))?;
            let rate = FrameRate::new(i64::from(if stream.fps == 0 { 30 } else { stream.fps }), 1);
            duration = rate.tick_of(stream.video.len() as i64);
            let secs = duration.seconds();
            let total: usize = stream.video.iter().map(|s| s.len).sum();
            video_info = Some(VideoStreamInfo {
                width: stream.width,
                height: stream.height,
                frame_rate: rate,
                par: (1, 1),
                codec: "id RoQ".into(),
                pixel_format: "YUV 4:4:4 8-bit, full range, progressive".into(),
                color: color(),
                has_alpha: false,
                bitrate: (secs > 0.0).then(|| (total as f64 * 8.0 / secs) as u64),
                hdr: None,
            });
            let state_bytes = dec.state_bytes().max(stream.width as usize * stream.height as usize * 6);
            video = Some(Video { width: stream.width, height: stream.height, rate, cache: GopCache::new(Some(color())), checkpoints: Arc::new(Checkpoints::new(SPACING, state_bytes)) });
        }
        let mut audio = None;
        let mut audio_streams = Vec::new();
        if let Some(ch) = stream.audio_channels {
            let ch = ch as usize;
            let mut pcm: Vec<i16> = Vec::new();
            for a in &stream.audio {
                if pcm.len() >= MAX_AUDIO_VALUES {
                    break;
                }
                // a chunk that does not decode is silence of its length
                match cfa4_roq::decode_audio(a.of(&bytes), ch as u32) {
                    Ok(p) => pcm.extend(p),
                    Err(_) => pcm.resize(pcm.len() + cfa4_roq::audio_samples(a.len, ch as u32) * ch, 0),
                }
            }
            let frames = pcm.len() / ch;
            let samples: Vec<Vec<f32>> = (0..ch).map(|c| pcm.iter().skip(c).step_by(ch).take(frames).map(|&v| f32::from(v) / 32768.0).collect()).collect();
            duration = duration.max(Tick::from_units(frames as i64, i64::from(cfa4_roq::AUDIO_RATE)));
            audio_streams.push(AudioStreamInfo {
                sample_rate: cfa4_roq::AUDIO_RATE,
                channels: ch as u32,
                codec: "id RoQ DPCM".into(),
                bits_per_sample: Some(16),
                ..AudioStreamInfo::default()
            });
            audio = Some(Audio { channels: ch, samples });
        }
        if video.is_none() && audio.is_none() {
            return Err(CodecError::Container(format!("{name}: no video or audio")));
        }
        let info = MediaInfo {
            name: name.to_string(),
            kind: if video.is_some() { MediaKind::Movie } else { MediaKind::AudioOnly },
            duration,
            video: video_info,
            audio_streams,
            container: "id RoQ".into(),
            start_timecode: None,
            file_size: Some(bytes.len() as u64),
        };
        Ok(Self { info, bytes, stream, video, audio })
    }
}

/// The video decoder: `cfa4_roq::Decoder`, restarted from a kept state when a sample other than
/// the next one arrives.
struct RoqDecoder {
    dec: Decoder,
    fresh: Decoder,
    store: Arc<Checkpoints<Decoder>>,
    rate: FrameRate,
    expect: Option<i64>,
}

impl VideoDecoder for RoqDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let i = usize::try_from(pts).unwrap_or(0);
        if self.expect != Some(pts) {
            self.dec = self.store.get(i).unwrap_or_else(|| self.fresh.clone());
        }
        self.store.offer(i, &self.dec);
        let f = self.dec.decode(sample);
        self.expect = Some(pts + 1);
        let [y, u, v] = f.planes;
        let frame = VideoFrame {
            width: f.width,
            height: f.height,
            data: PixelData::Yuv8 { planes: [Arc::new(y), Arc::new(u), Arc::new(v)], chroma: Chroma::C444, alpha: None },
            color: color(),
            par: (1, 1),
            pts: Tick::from_rational(pts, self.rate.den, self.rate.num),
            layout: Default::default(),
        };
        Ok(vec![DecodedFrame { pts, frame, draft: false }])
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        Vec::new()
    }
    fn reset(&mut self) {
        self.expect = None;
    }
    fn name(&self) -> &str {
        "CFA4 id RoQ"
    }
}

struct RoqVideo<'a>(&'a RoqSource, &'a Video);

impl VideoSamples for RoqVideo<'_> {
    fn count(&self) -> usize {
        self.0.stream.video.len()
    }
    fn pts(&self, i: usize) -> i64 {
        i as i64
    }
    fn sync_before(&self, i: usize) -> usize {
        self.1.checkpoints.sync_before(i)
    }
    fn sample_at(&self, t: i64) -> Option<usize> {
        let n = self.count();
        (n > 0).then(|| t.clamp(0, n as i64 - 1) as usize)
    }
    fn read(&self, i: usize) -> Result<Vec<u8>> {
        let s = self.0.stream.video.get(i).ok_or_else(|| CodecError::Container("frame index out of range".into()))?;
        Ok(s.of(&self.0.bytes).to_vec())
    }
    fn make_decoder(&self) -> Result<Box<dyn VideoDecoder>> {
        let fresh = Decoder::new(self.1.width, self.1.height).map_err(|e| CodecError::Unsupported(e.to_string()))?;
        Ok(Box::new(RoqDecoder { dec: fresh.clone(), fresh, store: self.1.checkpoints.clone(), rate: self.1.rate, expect: None }))
    }
}

impl MediaSource for RoqSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video_frame(&self, req: FrameRequest) -> std::result::Result<Arc<VideoFrame>, MediaError> {
        let v = self.video.as_ref().ok_or(MediaError::NoStream("video"))?;
        let t = req.time.max(Tick::ZERO);
        let target = v.rate.frame_at(t);
        let late = cfa4_media::cancel::catch_up().map(|m| v.rate.frame_at(t - m));
        Ok(v.cache.frame_late(&RoqVideo(self, v), target, late)?)
    }

    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> std::result::Result<AudioBuffer, MediaError> {
        let a = self.audio.as_ref().ok_or(MediaError::NoStream("audio"))?;
        cfa4_media::validate_audio_buffer_shape(a.channels, frames)?;
        Ok(crate::audio::read_resampled(&a.samples, cfa4_roq::AUDIO_RATE, start, frames, sample_rate))
    }
}

pub fn opener(name: &str, bytes: Arc<[u8]>) -> Option<std::result::Result<SharedSource, MediaError>> {
    if !sniff(&bytes) {
        return None;
    }
    Some(RoqSource::open(name, bytes).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_and_garbage() {
        assert!(!sniff(b""));
        assert!(!sniff(b"RIFF\0\0\0\0AVI "));
        assert!(opener("x.roq", Arc::from(&b"\x84\x10\xff\xff\xff\xff\x1e\x00"[..])).is_some_and(|r| r.is_err()));
    }
}
