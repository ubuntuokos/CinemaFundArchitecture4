//! AMV files (`.amv`): `cfa4-amv` finds the chunks and decodes the JPEG-variant video (4:2:0, full
//! range, stored upside down) and the IMA ADPCM sound (CB-32.1).
//!
//! Every picture stands alone, so a seek decodes the picture asked for and no other. The sound
//! chunks stand alone too (each starts with its own predictor); they are decoded whole at open
//! (22.05 kHz mono, small next to the video), a chunk that does not decode is silence of its length.

use std::sync::Arc;

use cfa4_amv::{Amv, Decoder};
use cfa4_color::{ColorInfo, Matrix, Range};
use cfa4_frame::{AudioBuffer, Chroma, PixelData, VideoFrame};
use cfa4_media::{AudioStreamInfo, FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, SharedSource, VideoStreamInfo};
use cfa4_time::{FrameRate, Tick};

use crate::gop::{GopCache, VideoSamples};
use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// Most decoded sound values kept: 2²⁷.
const MAX_AUDIO_VALUES: usize = 1 << 27;

pub fn sniff(head: &[u8]) -> bool {
    cfa4_amv::probe(head)
}

fn color() -> ColorInfo {
    // YUVJ420P as ffmpeg takes it: full range, BT.601 matrix
    ColorInfo { matrix: Matrix::Bt601, range: Range::Full, ..ColorInfo::REC709 }
}

pub struct AmvSource {
    info: MediaInfo,
    bytes: Arc<[u8]>,
    amv: Amv,
    rate: FrameRate,
    cache: GopCache,
    audio: Option<Vec<Vec<f32>>>,
}

impl AmvSource {
    pub fn open(name: &str, bytes: Arc<[u8]>) -> Result<Self> {
        let amv = Amv::parse(&bytes).map_err(|e| CodecError::Container(format!("{name}: {e}")))?;
        Decoder::new(amv.width, amv.height).map_err(|e| CodecError::Unsupported(format!("{name}: {e}")))?;
        let rate = FrameRate::new(i64::from(amv.frame_rate.0), i64::from(amv.frame_rate.1));
        let mut duration = rate.tick_of(amv.video.len() as i64);
        let secs = duration.seconds();
        let total: usize = amv.video.iter().map(|c| c.len).sum();
        let video_info = VideoStreamInfo {
            width: amv.width,
            height: amv.height,
            frame_rate: rate,
            par: (1, 1),
            codec: "AMV Video".into(),
            pixel_format: "YUV 4:2:0 8-bit, full range, progressive".into(),
            color: color(),
            has_alpha: false,
            bitrate: (secs > 0.0).then(|| (total as f64 * 8.0 / secs) as u64),
            hdr: None,
        };
        let mut audio = None;
        let mut audio_streams = Vec::new();
        if !amv.audio.is_empty() && cfa4_media::bounded_audio_dimensions(amv.sample_rate, 1).is_ok() {
            let mut pcm: Vec<i16> = Vec::new();
            for c in &amv.audio {
                if pcm.len() >= MAX_AUDIO_VALUES {
                    break;
                }
                let chunk = bytes.get(c.offset..c.offset + c.len).unwrap_or(&[]);
                match cfa4_amv::decode_audio(chunk) {
                    Ok(p) => pcm.extend(p),
                    Err(_) => pcm.resize(pcm.len() + cfa4_amv::audio_samples(chunk), 0),
                }
            }
            let planes = vec![pcm.iter().map(|&v| f32::from(v) / 32768.0).collect::<Vec<f32>>()];
            duration = duration.max(Tick::from_units(pcm.len() as i64, i64::from(amv.sample_rate)));
            audio_streams.push(AudioStreamInfo {
                sample_rate: amv.sample_rate,
                channels: 1,
                codec: "IMA ADPCM (AMV)".into(),
                bits_per_sample: Some(16),
                ..AudioStreamInfo::default()
            });
            audio = Some(planes);
        }
        let info = MediaInfo {
            name: name.to_string(),
            kind: MediaKind::Movie,
            duration,
            video: Some(video_info),
            audio_streams,
            container: "AMV".into(),
            start_timecode: None,
            file_size: Some(bytes.len() as u64),
        };
        Ok(Self { info, bytes, amv, rate, cache: GopCache::new(Some(color())), audio })
    }
}

struct AmvDecoder {
    dec: Decoder,
    rate: FrameRate,
}

impl VideoDecoder for AmvDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let pic = self.dec.decode(sample).map_err(|e| CodecError::Decode(e.to_string()))?;
        let [y, u, v] = pic.planes;
        let frame = VideoFrame {
            width: pic.width,
            height: pic.height,
            data: PixelData::Yuv8 { planes: [Arc::new(y), Arc::new(u), Arc::new(v)], chroma: Chroma::C420, alpha: None },
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
        "CFA4 AMV"
    }
    fn is_random_access(&self, _sample: &[u8]) -> Option<bool> {
        Some(true)
    }
    fn intra_only(&self) -> bool {
        true
    }
}

struct AmvVideo<'a>(&'a AmvSource);

impl VideoSamples for AmvVideo<'_> {
    fn count(&self) -> usize {
        self.0.amv.video.len()
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
        let c = self.0.amv.video.get(i).ok_or_else(|| CodecError::Container("frame index out of range".into()))?;
        Ok(self.0.bytes.get(c.offset..c.offset + c.len).unwrap_or(&[]).to_vec())
    }
    fn make_decoder(&self) -> Result<Box<dyn VideoDecoder>> {
        let dec = Decoder::new(self.0.amv.width, self.0.amv.height).map_err(|e| CodecError::Unsupported(e.to_string()))?;
        Ok(Box::new(AmvDecoder { dec, rate: self.0.rate }))
    }
}

impl MediaSource for AmvSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video_frame(&self, req: FrameRequest) -> std::result::Result<Arc<VideoFrame>, MediaError> {
        let t = req.time.max(Tick::ZERO);
        let target = self.rate.frame_at(t);
        let late = cfa4_media::cancel::catch_up().map(|m| self.rate.frame_at(t - m));
        Ok(self.cache.frame_late(&AmvVideo(self), target, late)?)
    }

    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> std::result::Result<AudioBuffer, MediaError> {
        let a = self.audio.as_ref().ok_or(MediaError::NoStream("audio"))?;
        cfa4_media::validate_audio_buffer_shape(1, frames)?;
        Ok(crate::audio::read_resampled(a, self.amv.sample_rate, start, frames, sample_rate))
    }
}

pub fn opener(name: &str, bytes: Arc<[u8]>) -> Option<std::result::Result<SharedSource, MediaError>> {
    if !sniff(&bytes) {
        return None;
    }
    Some(AmvSource::open(name, bytes).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_and_garbage() {
        assert!(!sniff(b""));
        assert!(!sniff(b"RIFF\0\0\0\0AVI "));
        assert!(sniff(b"RIFF\0\0\0\0AMV "));
        assert!(opener("x.amv", Arc::from(&b"RIFF\0\0\0\0AMV LIST"[..])).is_some_and(|r| r.is_err()));
    }
}
