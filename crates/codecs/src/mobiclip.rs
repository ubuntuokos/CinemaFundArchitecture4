//! Nintendo MobiClip movies (`.moflex`): `cfa4-mobiclip` demuxes the container and decodes the
//! pictures, the FastAudio / IMA MOFLEX / PCM sound (CB-28.2).
//!
//! A picture is either intra (a sync sample) or predicted from the last few pictures. The sound
//! is decoded whole at open (small next to the video).

use std::sync::Arc;

use cfa4_color::{ColorInfo, Matrix, Range};
use cfa4_frame::{AudioBuffer, Chroma, PixelData, VideoFrame};
use cfa4_media::{AudioStreamInfo, FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, SharedSource, VideoStreamInfo};
use cfa4_mobiclip::{AudioCodec, Decoder, FastAudio, Movie};
use cfa4_time::{FrameRate, Tick};

use crate::gop::{GopCache, VideoSamples};
use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// Most decoded sound values (all channels) kept: 2²⁷.
const MAX_AUDIO_VALUES: usize = 1 << 27;
/// Most sound channels taken.
const MAX_CHANNELS: u32 = 8;

pub fn sniff(head: &[u8]) -> bool {
    cfa4_mobiclip::probe(head)
}

fn color() -> ColorInfo {
    // ffmpeg leaves the colour space of MOFLEX pictures unspecified: BT.601, limited range
    ColorInfo { matrix: Matrix::Bt601, range: Range::Limited, ..ColorInfo::REC709 }
}

struct Audio {
    channels: usize,
    rate: u32,
    samples: Vec<Vec<f32>>,
}

pub struct MobiclipSource {
    info: MediaInfo,
    movie: Movie,
    rate: FrameRate,
    /// For each picture, the nearest intra picture at or before it.
    sync: Vec<usize>,
    cache: GopCache,
    audio: Option<Audio>,
}

fn decode_sound(m: &Movie) -> Option<(AudioStreamInfo, Audio)> {
    let a = m.audio.as_ref().filter(|a| (1..=MAX_CHANNELS).contains(&a.channels) && a.sample_rate > 0)?;
    let ch = a.channels as usize;
    let mut samples: Vec<Vec<f32>> = vec![Vec::new(); ch];
    let mut fast = FastAudio::new(ch);
    for p in &a.packets {
        if samples[0].len() * ch >= MAX_AUDIO_VALUES {
            break;
        }
        let part: Vec<Vec<f32>> = match a.codec {
            AudioCodec::FastAudio => fast.decode(p),
            AudioCodec::ImaMoflex => match cfa4_mobiclip::decode_ima(p, ch) {
                Ok(v) => v.into_iter().map(|c| c.into_iter().map(|s| f32::from(s) / 32768.0).collect()).collect(),
                // a packet that does not decode adds nothing, as in ffmpeg
                Err(_) => continue,
            },
            AudioCodec::PcmS16le => cfa4_mobiclip::decode_pcm(p, ch).into_iter().map(|c| c.into_iter().map(|s| f32::from(s) / 32768.0).collect()).collect(),
        };
        for (dst, src) in samples.iter_mut().zip(part) {
            dst.extend(src);
        }
    }
    let (codec, bits) = match a.codec {
        AudioCodec::FastAudio => ("MobiClip FastAudio", None),
        AudioCodec::ImaMoflex => ("IMA ADPCM (MOFLEX)", Some(16)),
        AudioCodec::PcmS16le => ("PCM signed 16-bit little-endian", Some(16)),
    };
    let info = AudioStreamInfo { sample_rate: a.sample_rate, channels: a.channels, codec: codec.into(), bits_per_sample: bits, ..AudioStreamInfo::default() };
    Some((info, Audio { channels: ch, rate: a.sample_rate, samples }))
}

impl MobiclipSource {
    pub fn open(name: &str, bytes: Arc<[u8]>) -> Result<Self> {
        let movie = cfa4_mobiclip::parse(&bytes).map_err(|e| CodecError::Container(format!("{name}: {e}")))?;
        if movie.video.is_empty() {
            return Err(CodecError::Container(format!("{name}: no frames")));
        }
        // the decoder must accept the picture size
        Decoder::new(movie.width, movie.height).map_err(|e| CodecError::Container(format!("{name}: {e}")))?;
        let rate = FrameRate::new(i64::from(movie.fps_num), i64::from(movie.fps_den)).sane();
        let mut duration = rate.tick_of(movie.video.len() as i64);
        let total: usize = movie.video.iter().map(|p| p.data.len()).sum();
        let secs = duration.seconds();
        let mut last = 0;
        let sync = movie
            .video
            .iter()
            .enumerate()
            .map(|(i, p)| {
                if p.key || i == 0 {
                    last = i;
                }
                last
            })
            .collect();
        let video_info = VideoStreamInfo {
            width: movie.width,
            height: movie.height,
            frame_rate: rate,
            par: (1, 1),
            codec: "Nintendo MobiClip".into(),
            pixel_format: "YUV 4:2:0 8-bit".into(),
            color: color(),
            has_alpha: false,
            bitrate: (secs > 0.0).then(|| (total as f64 * 8.0 / secs) as u64),
            hdr: None,
        };
        let mut audio_streams = Vec::new();
        let mut audio = None;
        if let Some((info, a)) = decode_sound(&movie) {
            let frames = a.samples.first().map_or(0, Vec::len);
            duration = duration.max(Tick::from_units(frames as i64, i64::from(a.rate)));
            audio_streams.push(info);
            audio = Some(a);
        }
        let info = MediaInfo {
            name: name.to_string(),
            kind: MediaKind::Movie,
            duration,
            video: Some(video_info),
            audio_streams,
            container: "Nintendo MobiClip MOFLEX".into(),
            start_timecode: None,
            file_size: Some(bytes.len() as u64),
        };
        Ok(Self { info, movie, rate, sync, cache: GopCache::new(Some(color())), audio })
    }
}

struct MobiDecoder {
    dec: Decoder,
    width: u32,
    height: u32,
    rate: FrameRate,
}

/// The 4:2:0 pictures of a YCgCo stream (the second plane Cg, the third Co) as RGBA, full range.
fn ycgco_rgba(f: &cfa4_mobiclip::Frame) -> Vec<u8> {
    let (w, h) = (f.width as usize, f.height as usize);
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let l = i32::from(f.planes[0][y * w + x]);
            let c = (y / 2) * (w / 2) + x / 2;
            let cg = i32::from(f.planes[1][c]) - 128;
            let co = i32::from(f.planes[2][c]) - 128;
            for v in [l - cg + co, l + cg, l - cg - co] {
                out.push(v.clamp(0, 255) as u8);
            }
            out.push(255);
        }
    }
    out
}

impl VideoDecoder for MobiDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let f = self.dec.decode(sample).map_err(|e| CodecError::Decode(e.to_string()))?;
        let t = Tick::from_rational(pts, self.rate.den, self.rate.num);
        let frame = if f.ycgco {
            VideoFrame::rgba8(f.width, f.height, ycgco_rgba(&f)).with_pts(t)
        } else {
            let [y, u, v] = f.planes;
            VideoFrame {
                width: f.width,
                height: f.height,
                data: PixelData::Yuv8 { planes: [Arc::new(y), Arc::new(u), Arc::new(v)], chroma: Chroma::C420, alpha: None },
                color: color(),
                par: (1, 1),
                pts: t,
                layout: Default::default(),
            }
        };
        Ok(vec![DecodedFrame { pts, frame, draft: false }])
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        Vec::new()
    }
    fn reset(&mut self) {
        if let Ok(d) = Decoder::new(self.width, self.height) {
            self.dec = d;
        }
    }
    fn name(&self) -> &str {
        "CFA4 Nintendo MobiClip"
    }
    fn intra_only(&self) -> bool {
        false
    }
}

struct MobiVideo<'a>(&'a MobiclipSource);

impl VideoSamples for MobiVideo<'_> {
    fn count(&self) -> usize {
        self.0.movie.video.len()
    }
    fn pts(&self, i: usize) -> i64 {
        i as i64
    }
    fn sync_before(&self, i: usize) -> usize {
        self.0.sync.get(i.min(self.0.sync.len().saturating_sub(1))).copied().unwrap_or(0)
    }
    fn sample_at(&self, t: i64) -> Option<usize> {
        let n = self.count();
        (n > 0).then(|| t.clamp(0, n as i64 - 1) as usize)
    }
    fn read(&self, i: usize) -> Result<Vec<u8>> {
        self.0.movie.video.get(i).map(|p| p.data.clone()).ok_or_else(|| CodecError::Container("frame index out of range".into()))
    }
    fn make_decoder(&self) -> Result<Box<dyn VideoDecoder>> {
        let (width, height) = (self.0.movie.width, self.0.movie.height);
        let dec = Decoder::new(width, height).map_err(|e| CodecError::Decode(e.to_string()))?;
        Ok(Box::new(MobiDecoder { dec, width, height, rate: self.0.rate }))
    }
}

impl MediaSource for MobiclipSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video_frame(&self, req: FrameRequest) -> std::result::Result<Arc<VideoFrame>, MediaError> {
        let t = req.time.max(Tick::ZERO);
        let target = self.rate.frame_at(t);
        let late = cfa4_media::cancel::catch_up().map(|m| self.rate.frame_at(t - m));
        Ok(self.cache.frame_late(&MobiVideo(self), target, late)?)
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
    Some(MobiclipSource::open(name, bytes).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_and_garbage() {
        assert!(!sniff(b""));
        assert!(!sniff(b"RIFF\0\0\0\0AVI "));
        assert!(!sniff(b"L2\0\0\0\0\0\0\0\0\0\0\0\0"));
        let mut head = b"L2\0\0\0\0\0\0\0\0\0\0\0\x10".to_vec();
        head.extend_from_slice(&[1, 12]);
        head.extend_from_slice(&[0; 12]);
        assert!(opener("x.moflex", Arc::from(&head[..])).is_some_and(|r| r.is_err()));
    }
}
