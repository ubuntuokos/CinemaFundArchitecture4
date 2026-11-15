//! Our pure-Rust MPEG-4 Simple Studio Profile decoder (`cfa4-sstp`; Sony HDCAM SR / SRMASTER):
//! raw elementary streams (`.m4v`, `.sstp`) and MPEG-4 Visual essence in MXF.
//!
//! Every SStP picture is intra-coded, so every access unit is a random access point. The
//! headers before the first picture (visual object sequence, visual object, video object
//! layer) are kept and given to every new decoder, so decoding can start at any picture.

use std::sync::Arc;

use cfa4_frame::{AudioBuffer, Chroma, PixelData, VideoFrame};
use cfa4_media::{FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, SharedSource, VideoStreamInfo};
use cfa4_time::{FrameRate, Tick};

use crate::gop::{GopCache, VideoSamples};
use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// The most access units an elementary stream may index (a day at 60 frames per second).
const MAX_UNITS: usize = 60 * 60 * 60 * 24;
/// The largest access unit (a 4:4:4 8K picture is far below it).
const MAX_UNIT: u64 = 1 << 30;
/// How much of a stream [`sniff`] looks at.
pub const SNIFF_LEN: usize = 64 << 10;

/// The codec name shown for SStP streams.
pub const NAME: &str = "MPEG-4 Simple Studio Profile (Sony SR)";

/// Whether `head` starts an SStP elementary stream.
pub fn sniff(head: &[u8]) -> bool {
    cfa4_sstp::probe(head.get(..SNIFF_LEN).unwrap_or(head)).is_some()
}

/// The bytes before the first picture of `sample` (its headers), if it has a studio video
/// object layer.
pub fn stream_headers(sample: &[u8]) -> Option<(Vec<u8>, cfa4_sstp::StreamInfo)> {
    let info = cfa4_sstp::probe(sample)?;
    let vop = sample.windows(4).position(|w| w == [0, 0, 1, 0xB6])?;
    Some((sample.get(..vop)?.to_vec(), info))
}

/// The pixel format label of a stream.
pub fn pixel_format(info: &cfa4_sstp::StreamInfo) -> String {
    match info.chroma {
        cfa4_sstp::Chroma::Yuv422 => "YUV 4:2:2 10-bit".into(),
        cfa4_sstp::Chroma::Yuv444 => "YUV 4:4:4 10-bit".into(),
        cfa4_sstp::Chroma::Rgb => "RGB 4:4:4 10-bit".into(),
    }
}

/// A decoded picture as a frame.
pub(crate) fn convert(p: cfa4_sstp::Picture) -> Result<VideoFrame> {
    let width = u32::try_from(p.width).map_err(|_| CodecError::Decode("SStP: picture too wide".into()))?;
    let height = u32::try_from(p.height).map_err(|_| CodecError::Decode("SStP: picture too tall".into()))?;
    let [a, b, c] = p.planes;
    let data = match p.chroma {
        // the studio RGB planes are coded G, B, R
        cfa4_sstp::Chroma::Rgb => PixelData::Rgb16 { planes: [Arc::new(c), Arc::new(a), Arc::new(b)], bits: 10, alpha: None },
        cfa4_sstp::Chroma::Yuv422 => PixelData::Yuv16 { planes: [Arc::new(a), Arc::new(b), Arc::new(c)], chroma: Chroma::C422, bits: 10, alpha: None },
        cfa4_sstp::Chroma::Yuv444 => PixelData::Yuv16 { planes: [Arc::new(a), Arc::new(b), Arc::new(c)], chroma: Chroma::C444, bits: 10, alpha: None },
    };
    let color = crate::stream_color::resolve(width, height, &[]);
    Ok(VideoFrame { width, height, data, color, par: (1, 1), pts: Tick::ZERO, layout: Default::default() })
}

/// An SStP decoder primed with the stream's headers.
pub struct SstpDecoder {
    dec: cfa4_sstp::Decoder,
    headers: Vec<u8>,
}

impl SstpDecoder {
    /// A decoder for a stream whose headers (before the first picture) are `headers`.
    pub fn new(headers: Vec<u8>) -> Self {
        let mut d = Self { dec: cfa4_sstp::Decoder::new(), headers };
        d.reset();
        d
    }
}

impl VideoDecoder for SstpDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let p = self.dec.decode(sample).map_err(|e| match e {
            cfa4_sstp::Error::Unsupported(why) => CodecError::Unsupported(format!("SStP: {why}")),
            e => CodecError::Decode(e.to_string()),
        })?;
        Ok(vec![DecodedFrame { pts, frame: convert(p)?, draft: false }])
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        Vec::new()
    }
    fn reset(&mut self) {
        self.dec = cfa4_sstp::Decoder::new();
        if !self.headers.is_empty() && self.dec.parse_headers(&self.headers).is_err() {
            log::warn!("SStP: unreadable stream headers");
        }
    }
    fn name(&self) -> &str {
        "CFA4 SStP"
    }
    fn intra_only(&self) -> bool {
        true
    }
    fn is_random_access(&self, _sample: &[u8]) -> Option<bool> {
        Some(true)
    }
}

/// A raw SStP elementary stream.
pub struct SstpSource {
    info: MediaInfo,
    bytes: crate::Src,
    /// (offset, size) of every access unit.
    units: Vec<(u64, u64)>,
    headers: Vec<u8>,
    rate: FrameRate,
    video: GopCache,
}

impl SstpSource {
    pub fn open(name: &str, bytes: Arc<[u8]>) -> Result<Self> {
        Self::open_reader(name, Arc::new(cfa4_media::reader::MemReader(bytes)))
    }

    pub fn open_reader(name: &str, reader: cfa4_media::SharedReader) -> Result<Self> {
        let len = reader.len();
        let head = cfa4_media::reader::read_range(&*reader, 0, len.min(SNIFF_LEN as u64) as usize).map_err(|e| CodecError::Container(e.to_string()))?;
        let (headers, si) = stream_headers(&head).ok_or_else(|| CodecError::Container("SStP: no studio video object layer before the first picture".into()))?;
        // index the access units
        let mut starts = vec![0u64];
        let mut split = cfa4_sstp::AccessUnits::new();
        let mut buf = vec![0u8; 1 << 20];
        let mut pos = 0u64;
        while pos < len {
            let n = (len - pos).min(buf.len() as u64) as usize;
            let chunk = buf.get_mut(..n).ok_or_else(|| CodecError::Container("SStP: read".into()))?;
            reader.read_at(pos, chunk).map_err(|e| CodecError::Container(e.to_string()))?;
            split.feed(pos, chunk, &mut starts);
            if starts.len() > MAX_UNITS {
                return Err(CodecError::Container("SStP: too many pictures".into()));
            }
            pos += n as u64;
        }
        starts.push(len);
        let units: Vec<(u64, u64)> = starts
            .windows(2)
            .filter_map(|w| match *w {
                [a, b] if b > a => Some((a, b - a)),
                _ => None,
            })
            .collect();
        if units.is_empty() {
            return Err(CodecError::Container("SStP: no pictures".into()));
        }
        if units.iter().any(|u| u.1 > MAX_UNIT) {
            return Err(CodecError::Container("SStP: access unit too large".into()));
        }
        let rate = match si.frame_rate {
            Some((n, d)) => FrameRate::new(i64::from(n), i64::from(d)),
            None => FrameRate::FPS_25,
        };
        let count = i64::try_from(units.len()).unwrap_or(i64::MAX);
        let duration = Tick::from_rational(count.saturating_mul(rate.den), 1, rate.num);
        let total_bits = len.saturating_mul(8);
        let secs = duration.seconds();
        let bitrate = (secs > 0.0).then(|| (total_bits as f64 / secs) as u64);
        let width = u32::try_from(si.width).unwrap_or(0);
        let height = u32::try_from(si.height).unwrap_or(0);
        let color = crate::stream_color::resolve(width, height, &[]);
        let info = MediaInfo {
            name: name.to_string(),
            kind: MediaKind::Movie,
            duration,
            video: Some(VideoStreamInfo {
                width,
                height,
                frame_rate: rate,
                par: si.par,
                codec: NAME.into(),
                pixel_format: pixel_format(&si),
                color,
                has_alpha: false,
                bitrate,
                hdr: None,
            }),
            audio_streams: Vec::new(),
            container: "MPEG-4 Visual elementary stream".into(),
            start_timecode: None,
            file_size: Some(len),
        };
        Ok(Self { info, bytes: crate::Src(reader), units, headers, rate, video: GopCache::new(None) })
    }
}

struct SstpVideo<'a>(&'a SstpSource);

impl VideoSamples for SstpVideo<'_> {
    fn count(&self) -> usize {
        self.0.units.len()
    }
    fn pts(&self, i: usize) -> i64 {
        i64::try_from(i).unwrap_or(i64::MAX)
    }
    fn sync_before(&self, i: usize) -> usize {
        i
    }
    fn sample_at(&self, t: i64) -> Option<usize> {
        let last = self.0.units.len().checked_sub(1)?;
        Some(usize::try_from(t.max(0)).unwrap_or(last).min(last))
    }
    fn read(&self, i: usize) -> Result<Vec<u8>> {
        let (off, sz) = self.0.units.get(i).copied().ok_or_else(|| CodecError::Container("sample index out of range".into()))?;
        let sz = usize::try_from(sz).map_err(|_| CodecError::Container("SStP: access unit too large".into()))?;
        cfa4_media::reader::read_range(&*self.0.bytes.0, off, sz).map_err(|e| CodecError::Container(e.to_string()))
    }
    fn make_decoder(&self) -> Result<Box<dyn VideoDecoder>> {
        Ok(Box::new(SstpDecoder::new(self.0.headers.clone())))
    }
}

impl MediaSource for SstpSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video_frame(&self, req: FrameRequest) -> std::result::Result<Arc<VideoFrame>, MediaError> {
        let i = self.rate.frame_at(req.time.max(Tick::ZERO));
        let mut f = self.video.frame(&SstpVideo(self), i)?;
        if let Some(v) = &self.info.video
            && f.par != v.par
        {
            Arc::make_mut(&mut f).par = v.par;
        }
        Ok(f)
    }

    fn audio(&self, _start: i64, _frames: usize, _sample_rate: u32) -> std::result::Result<AudioBuffer, MediaError> {
        Err(MediaError::NoStream("audio"))
    }
}

pub fn opener(name: &str, bytes: Arc<[u8]>) -> Option<std::result::Result<SharedSource, MediaError>> {
    if !sniff(&bytes) {
        return None;
    }
    Some(SstpSource::open(name, bytes).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

pub fn reader_opener(name: &str, head: &[u8], reader: &cfa4_media::SharedReader) -> Option<std::result::Result<SharedSource, MediaError>> {
    if !sniff(head) {
        return None;
    }
    Some(SstpSource::open_reader(name, reader.clone()).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_streams_are_not_sniffed() {
        assert!(!sniff(b""));
        assert!(!sniff(&[0, 0, 1, 0xB3, 0x14, 0, 0xF0]));
        // a simple-profile visual object sequence
        assert!(!sniff(&[0, 0, 1, 0xB0, 0x01, 0, 0, 1, 0xB5, 0x09, 0, 0, 1, 0xB6, 0]));
    }

    #[test]
    fn a_decoder_without_headers_refuses_pictures() {
        let mut d = SstpDecoder::new(Vec::new());
        assert!(d.decode(&[0, 0, 1, 0xB6, 0, 0, 0, 0], 0).is_err());
    }
}
