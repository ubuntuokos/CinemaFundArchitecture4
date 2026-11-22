//! Nintendo THP movies (`.thp`): `cfa4-thp` finds the frames and decodes the JPEG pictures
//! (full range YUV) and the ADPCM sound (CB-28.1).
//!
//! Every picture is a JPEG of its own, so any frame is a sync sample. The sound is decoded whole
//! at open (small next to the video).

use std::sync::Arc;

use cfa4_color::{ColorInfo, Matrix, Range};
use cfa4_frame::{AudioBuffer, Chroma, PixelData, VideoFrame};
use cfa4_media::{AudioStreamInfo, FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, SharedSource, VideoStreamInfo};
use cfa4_thp::{AudioDecoder, Movie};
use cfa4_time::{FrameRate, Tick};

use crate::gop::{GopCache, VideoSamples};
use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// Most decoded sound values (all channels) kept: 2²⁷.
const MAX_AUDIO_VALUES: usize = 1 << 27;

pub fn sniff(head: &[u8]) -> bool {
    cfa4_thp::probe(head)
}

fn color() -> ColorInfo {
    // YUVJ4xxP as ffmpeg takes it: full range, BT.601 matrix
    ColorInfo { matrix: Matrix::Bt601, range: Range::Full, ..ColorInfo::REC709 }
}

struct Audio {
    channels: usize,
    rate: u32,
    samples: Vec<Vec<f32>>,
}

pub struct ThpSource {
    info: MediaInfo,
    bytes: Arc<[u8]>,
    movie: Movie,
    rate: FrameRate,
    cache: GopCache,
    audio: Option<Audio>,
}

impl ThpSource {
    pub fn open(name: &str, bytes: Arc<[u8]>) -> Result<Self> {
        let movie = cfa4_thp::parse(&bytes).map_err(|e| CodecError::Container(format!("{name}: {e}")))?;
        let rate = FrameRate::new(movie.fps_num, movie.fps_den).sane();
        let mut duration = rate.tick_of(movie.video.len() as i64);
        let total: usize = movie.video.iter().map(|s| s.len).sum();
        let secs = duration.seconds();
        let video_info = VideoStreamInfo {
            width: movie.width,
            height: movie.height,
            frame_rate: rate,
            par: (1, 1),
            codec: "Nintendo THP (JPEG)".into(),
            pixel_format: "YUV 8-bit, full range, progressive".into(),
            color: color(),
            has_alpha: false,
            bitrate: (secs > 0.0).then(|| (total as f64 * 8.0 / secs) as u64),
            hdr: None,
        };
        let mut audio = None;
        let mut audio_streams = Vec::new();
        if let Some(a) = movie.audio.filter(|a| (1..=2).contains(&a.channels) && a.sample_rate > 0) {
            let ch = a.channels as usize;
            let mut dec = AudioDecoder::new();
            let mut pcm: Vec<i16> = Vec::new();
            for p in &movie.audio_packets {
                if pcm.len() >= MAX_AUDIO_VALUES {
                    break;
                }
                // a packet that does not decode adds nothing, as in ffmpeg
                if let Ok(v) = dec.decode(p.of(&bytes), a.channels) {
                    pcm.extend(v);
                }
            }
            let frames = pcm.len() / ch;
            let samples: Vec<Vec<f32>> = (0..ch).map(|c| pcm.iter().skip(c).step_by(ch).take(frames).map(|&v| f32::from(v) / 32768.0).collect()).collect();
            duration = duration.max(Tick::from_units(frames as i64, i64::from(a.sample_rate)));
            audio_streams.push(AudioStreamInfo {
                sample_rate: a.sample_rate,
                channels: a.channels,
                codec: "THP ADPCM".into(),
                bits_per_sample: Some(16),
                ..AudioStreamInfo::default()
            });
            audio = Some(Audio { channels: ch, rate: a.sample_rate, samples });
        }
        let info = MediaInfo {
            name: name.to_string(),
            kind: MediaKind::Movie,
            duration,
            video: Some(video_info),
            audio_streams,
            container: "Nintendo THP".into(),
            start_timecode: None,
            file_size: Some(bytes.len() as u64),
        };
        if movie.video.is_empty() {
            return Err(CodecError::Container(format!("{name}: no frames")));
        }
        Ok(Self { info, bytes, movie, rate, cache: GopCache::new(Some(color())), audio })
    }
}

struct ThpDecoder {
    rate: FrameRate,
}

fn plane(v: Vec<u8>) -> Arc<Vec<u8>> {
    Arc::new(v)
}

impl VideoDecoder for ThpDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let p = cfa4_thp::jpeg::decode(sample).map_err(|e| CodecError::Decode(e.to_string()))?;
        let (w, h) = (p.width as usize, p.height as usize);
        let [y, u, v] = p.planes;
        // the frame types know 4:2:0, 4:2:2 and 4:4:4: grey gets neutral chroma, 4:4:0 doubles
        // its chroma lines
        let (planes, chroma) = match p.chroma {
            cfa4_thp::Chroma::C420 => ([plane(y), plane(u), plane(v)], Chroma::C420),
            cfa4_thp::Chroma::C422 => ([plane(y), plane(u), plane(v)], Chroma::C422),
            cfa4_thp::Chroma::C444 => ([plane(y), plane(u), plane(v)], Chroma::C444),
            cfa4_thp::Chroma::Gray => {
                let n = w.div_ceil(2) * h.div_ceil(2);
                ([plane(y), plane(vec![128; n]), plane(vec![128; n])], Chroma::C420)
            }
            cfa4_thp::Chroma::C440 => {
                let up = |c: Vec<u8>| -> Vec<u8> { (0..h).flat_map(|r| c[(r / 2) * w..(r / 2) * w + w].to_vec()).collect() };
                ([plane(y), plane(up(u)), plane(up(v))], Chroma::C444)
            }
        };
        let frame = VideoFrame {
            width: p.width,
            height: p.height,
            data: PixelData::Yuv8 { planes, chroma, alpha: None },
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
    fn reset(&mut self) {}
    fn name(&self) -> &str {
        "CFA4 Nintendo THP"
    }
    fn intra_only(&self) -> bool {
        true
    }
}

struct ThpVideo<'a>(&'a ThpSource);

impl VideoSamples for ThpVideo<'_> {
    fn count(&self) -> usize {
        self.0.movie.video.len()
    }
    fn pts(&self, i: usize) -> i64 {
        i as i64
    }
    fn sync_before(&self, i: usize) -> usize {
        i
    }
    fn sample_at(&self, t: i64) -> Option<usize> {
        let n = self.count();
        (n > 0).then(|| t.clamp(0, n as i64 - 1) as usize)
    }
    fn read(&self, i: usize) -> Result<Vec<u8>> {
        let s = self.0.movie.video.get(i).ok_or_else(|| CodecError::Container("frame index out of range".into()))?;
        Ok(s.of(&self.0.bytes).to_vec())
    }
    fn make_decoder(&self) -> Result<Box<dyn VideoDecoder>> {
        Ok(Box::new(ThpDecoder { rate: self.0.rate }))
    }
}

impl MediaSource for ThpSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video_frame(&self, req: FrameRequest) -> std::result::Result<Arc<VideoFrame>, MediaError> {
        let t = req.time.max(Tick::ZERO);
        let target = self.rate.frame_at(t);
        let late = cfa4_media::cancel::catch_up().map(|m| self.rate.frame_at(t - m));
        Ok(self.cache.frame_late(&ThpVideo(self), target, late)?)
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
    Some(ThpSource::open(name, bytes).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_and_garbage() {
        assert!(!sniff(b""));
        assert!(!sniff(b"RIFF\0\0\0\0AVI "));
        assert!(opener("x.thp", Arc::from(&b"THP\0\0\x01\x10\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0"[..])).is_some_and(|r| r.is_err()));
    }
}
