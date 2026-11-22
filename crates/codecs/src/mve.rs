//! Interplay MVE files (`.mve`): `cfa4-mve` finds the packets and decodes the video (8-bit
//! palette or 16-bit RGB 5:5:5 pictures) and the sound (PCM or Interplay DPCM) (CB-25.2).
//!
//! Every picture is coded against the ones before it, so decoding runs forward from the start.
//! A shown picture is one sample: the packets that only change the decoder's state come before
//! it in the same sample. The decoder keeps a copy of its state at every 64th picture it passes
//! ([`crate::ckpt`]) and the sample table reports those as sync samples: a seek decodes from the
//! nearest one passed. The sound is decoded whole at open.

use std::sync::Arc;

use cfa4_frame::{AudioBuffer, VideoFrame};
use cfa4_media::{AudioStreamInfo, FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, SharedSource, VideoStreamInfo};
use cfa4_mve::{Decoder, Input, Movie};
use cfa4_time::{FrameRate, Tick};

use crate::ckpt::Checkpoints;
use crate::gop::{GopCache, VideoSamples};
use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// Pictures between decoder checkpoints.
const SPACING: usize = 64;
/// Most decoded sound values (all channels) kept: 2²⁷.
const MAX_AUDIO_VALUES: usize = 1 << 27;
/// Rate of a file with no timer (ffmpeg's demuxer leaves it at 0; 15 fps is what the games used).
const DEFAULT_FRAME_US: u64 = 66_667;

pub fn sniff(head: &[u8]) -> bool {
    cfa4_mve::probe(head)
}

struct Inner {
    bytes: Arc<[u8]>,
    movie: Movie,
    /// Per shown picture: the packets `first..=last` it takes (the last one is the shown one).
    shown: Vec<(usize, usize)>,
    rate: FrameRate,
    cache: GopCache,
    checkpoints: Arc<Checkpoints<Decoder>>,
}

struct Audio {
    channels: usize,
    rate: u32,
    samples: Vec<Vec<f32>>,
}

pub struct MveSource {
    info: MediaInfo,
    inner: Option<Arc<Inner>>,
    audio: Option<Audio>,
}

impl MveSource {
    pub fn open(name: &str, bytes: Arc<[u8]>) -> Result<Self> {
        let movie = cfa4_mve::parse(&bytes).map_err(|e| CodecError::Container(format!("{name}: {e}")))?;
        let mut shown = Vec::new();
        let mut first = 0;
        for (i, p) in movie.video.iter().enumerate() {
            if p.send_buffer {
                shown.push((first, i));
                first = i + 1;
            }
        }
        let mut duration = Tick::ZERO;
        let mut video_info = None;
        let mut inner = None;
        if !shown.is_empty() {
            let dec = Decoder::new(movie.width, movie.height, movie.bpp16).map_err(|e| CodecError::Unsupported(format!("{name}: {e}")))?;
            let us = if movie.frame_us == 0 { DEFAULT_FRAME_US } else { movie.frame_us };
            let rate = FrameRate::new(1_000_000, us as i64).sane();
            duration = rate.tick_of(shown.len() as i64);
            let total: usize = movie.video.iter().map(|p| p.video.len + p.decode_map.len + p.skip_map.len).sum();
            let secs = duration.seconds();
            video_info = Some(VideoStreamInfo {
                width: movie.width,
                height: movie.height,
                frame_rate: rate,
                par: (1, 1),
                codec: "Interplay MVE".into(),
                pixel_format: if movie.bpp16 { "RGB 5:5:5 16-bit".into() } else { "Palette 8-bit".into() },
                color: cfa4_color::ColorInfo::SRGB_FULL,
                has_alpha: false,
                bitrate: (secs > 0.0).then(|| (total as f64 * 8.0 / secs) as u64),
                hdr: None,
            });
            let state_bytes = dec.state_bytes();
            inner = Some(Arc::new(Inner {
                bytes: bytes.clone(),
                shown,
                rate,
                cache: GopCache::new(Some(cfa4_color::ColorInfo::SRGB_FULL)),
                checkpoints: Arc::new(Checkpoints::new(SPACING, state_bytes)),
                movie: movie.clone(),
            }));
        }
        let mut audio = None;
        let mut audio_streams = Vec::new();
        if let Some(p) = movie.audio.filter(|p| p.channels >= 1 && p.channels <= 2 && p.sample_rate > 0) {
            let ch = p.channels as usize;
            let mut pcm: Vec<i16> = Vec::new();
            for a in &movie.audio_packets {
                if pcm.len() >= MAX_AUDIO_VALUES {
                    break;
                }
                // a packet that does not decode is silence of its length
                match cfa4_mve::decode_audio(&p, a.of(&bytes)) {
                    Ok(v) => pcm.extend(v),
                    Err(_) => pcm.resize(pcm.len() + cfa4_mve::audio_values(&p, a.len), 0),
                }
            }
            let frames = pcm.len() / ch;
            let samples: Vec<Vec<f32>> = (0..ch).map(|c| pcm.iter().skip(c).step_by(ch).take(frames).map(|&v| f32::from(v) / 32768.0).collect()).collect();
            duration = duration.max(Tick::from_units(frames as i64, i64::from(p.sample_rate)));
            audio_streams.push(AudioStreamInfo {
                sample_rate: p.sample_rate,
                channels: p.channels,
                codec: if p.dpcm { "Interplay DPCM".into() } else { "PCM".into() },
                bits_per_sample: Some(16),
                ..AudioStreamInfo::default()
            });
            audio = Some(Audio { channels: ch, rate: p.sample_rate, samples });
        }
        if inner.is_none() && audio.is_none() {
            return Err(CodecError::Container(format!("{name}: no video or audio")));
        }
        let info = MediaInfo {
            name: name.to_string(),
            kind: if inner.is_some() { MediaKind::Movie } else { MediaKind::AudioOnly },
            duration,
            video: video_info,
            audio_streams,
            container: "Interplay MVE".into(),
            start_timecode: None,
            file_size: Some(bytes.len() as u64),
        };
        Ok(Self { info, inner, audio })
    }
}

/// The video decoder: `cfa4_mve::Decoder`, restarted from a kept state when a sample other than
/// the next one arrives. A sample is the 16 bytes naming the packets it takes.
struct MveDecoder {
    inner: Arc<Inner>,
    dec: Decoder,
    fresh: Decoder,
    expect: Option<i64>,
}

impl VideoDecoder for MveDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let i = usize::try_from(pts).unwrap_or(0);
        let rd = |o: usize| -> usize { sample.get(o..o + 8).and_then(|b| b.try_into().ok()).map_or(0, |b| u64::from_le_bytes(b) as usize) };
        let (first, last) = (rd(0), rd(8));
        if self.expect != Some(pts) {
            self.dec = self.inner.checkpoints.get(i).unwrap_or_else(|| self.fresh.clone());
        }
        self.inner.checkpoints.offer(i, &self.dec);
        self.expect = Some(pts + 1);
        let m = &self.inner.movie;
        let mut shown = None;
        for k in first..=last {
            let Some(p) = m.video.get(k) else { break };
            let b = &self.inner.bytes;
            let input = Input {
                format: p.format,
                send_buffer: p.send_buffer,
                video: p.video.of(b),
                decode_map: p.decode_map.of(b),
                skip_map: p.skip_map.of(b),
                palette: p.palette.as_deref(),
            };
            match self.dec.decode(&input) {
                Ok(f) => shown = f.or(shown),
                // a packet ffmpeg refuses leaves the decoder as it was; the picture before stays
                Err(e) if k == last => return Err(CodecError::Unsupported(e.to_string())),
                Err(_) => {}
            }
        }
        let f = shown.ok_or_else(|| CodecError::Unsupported("no picture in the sample".into()))?;
        let mut frame = VideoFrame::rgba8(f.width, f.height, f.to_rgba());
        frame.pts = Tick::from_rational(pts, self.inner.rate.den, self.inner.rate.num);
        Ok(vec![DecodedFrame { pts, frame, draft: false }])
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        Vec::new()
    }
    fn reset(&mut self) {
        self.expect = None;
    }
    fn name(&self) -> &str {
        "CFA4 Interplay MVE"
    }
}

struct MveVideo<'a>(&'a Arc<Inner>);

impl VideoSamples for MveVideo<'_> {
    fn count(&self) -> usize {
        self.0.shown.len()
    }
    fn pts(&self, i: usize) -> i64 {
        i as i64
    }
    fn sync_before(&self, i: usize) -> usize {
        self.0.checkpoints.sync_before(i)
    }
    fn sample_at(&self, t: i64) -> Option<usize> {
        let n = self.count();
        (n > 0).then(|| t.clamp(0, n as i64 - 1) as usize)
    }
    fn read(&self, i: usize) -> Result<Vec<u8>> {
        let (a, b) = *self.0.shown.get(i).ok_or_else(|| CodecError::Container("frame index out of range".into()))?;
        let mut v = (a as u64).to_le_bytes().to_vec();
        v.extend_from_slice(&(b as u64).to_le_bytes());
        Ok(v)
    }
    fn make_decoder(&self) -> Result<Box<dyn VideoDecoder>> {
        let m = &self.0.movie;
        let fresh = Decoder::new(m.width, m.height, m.bpp16).map_err(|e| CodecError::Unsupported(e.to_string()))?;
        Ok(Box::new(MveDecoder { inner: self.0.clone(), dec: fresh.clone(), fresh, expect: None }))
    }
}

impl MediaSource for MveSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video_frame(&self, req: FrameRequest) -> std::result::Result<Arc<VideoFrame>, MediaError> {
        let v = self.inner.as_ref().ok_or(MediaError::NoStream("video"))?;
        let t = req.time.max(Tick::ZERO);
        let target = v.rate.frame_at(t);
        let late = cfa4_media::cancel::catch_up().map(|m| v.rate.frame_at(t - m));
        Ok(v.cache.frame_late(&MveVideo(v), target, late)?)
    }

    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> std::result::Result<AudioBuffer, MediaError> {
        let a = self.audio.as_ref().ok_or(MediaError::NoStream("audio"))?;
        cfa4_media::validate_audio_buffer_shape(a.channels, frames)?;
        Ok(crate::audio::read_resampled(&a.samples, a.rate, start, frames, sample_rate))
    }
}

pub fn opener(name: &str, bytes: Arc<[u8]>) -> Option<std::result::Result<SharedSource, MediaError>> {
    if !sniff(&bytes) {
        return None;
    }
    Some(MveSource::open(name, bytes).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_and_garbage() {
        assert!(!sniff(b""));
        assert!(!sniff(b"RIFF\0\0\0\0AVI "));
    }
}
