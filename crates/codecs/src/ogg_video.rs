//! Theora video in Ogg (`.ogv`, `.ogg`): the first Theora stream of the file through our
//! pure-Rust decoder (`cfa4-theora`), plus the file's first audio stream (Vorbis, Opus, Speex)
//! through [`crate::ogg::OggSource`].
//!
//! Theora has no reordering: packet `i` is frame `i` at the nominal frame rate (an empty packet
//! repeats the previous frame). A key frame is a packet whose first byte has the data bit and the
//! inter bit clear; decoding for a seek starts at the key frame before the requested frame.

use std::sync::Arc;

use cfa4_frame::{AudioBuffer, VideoFrame};
use cfa4_media::{FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, SharedSource, VideoStreamInfo};
use cfa4_ogg::{Codec, OggFile};
use cfa4_time::{FrameRate, Tick};

use crate::gop::{GopCache, VideoSamples};
use crate::ogg::OggSource;
use crate::video::VideoDecoder;
use crate::{CodecError, Result};

/// The largest sample count an Ogg Theora stream may index (a day at 60 frames per second).
const MAX_PACKETS: usize = 60 * 60 * 60 * 24;

/// The Xiph-laced block of the three Theora headers (the Matroska `CodecPrivate` form).
fn lace(headers: &[Vec<u8>]) -> Option<Vec<u8>> {
    let [a, b, c] = headers else { return None };
    let mut out = vec![2u8];
    for n in [a.len(), b.len()] {
        out.extend(std::iter::repeat_n(255u8, n / 255));
        out.push((n % 255) as u8);
    }
    for h in [a, b, c] {
        out.extend_from_slice(h);
    }
    Some(out)
}

/// A Theora video stream (with its audio) in an Ogg file.
pub struct OggVideoSource {
    info: MediaInfo,
    bytes: crate::Src,
    file: OggFile,
    stream: usize,
    private: Vec<u8>,
    /// Key frame at or before each packet.
    sync: Vec<usize>,
    rate: FrameRate,
    video: GopCache,
    audio: Option<OggSource>,
}

impl OggVideoSource {
    pub fn open(name: &str, bytes: Arc<[u8]>) -> Result<Self> {
        Self::open_reader(name, Arc::new(cfa4_media::reader::MemReader(bytes)))
    }

    pub fn open_reader(name: &str, reader: cfa4_media::SharedReader) -> Result<Self> {
        let bytes = crate::Src(reader.clone());
        let file = cfa4_ogg::open(&bytes).map_err(|e| CodecError::Container(e.to_string()))?;
        for w in &file.warnings {
            log::warn!("{name}: {w}");
        }
        let stream = file
            .streams
            .iter()
            .position(|s| s.codec == Codec::Theora && !s.packets.is_empty())
            .ok_or_else(|| CodecError::Unsupported("Ogg: no Theora stream".into()))?;
        let st = &file.streams[stream];
        if st.packets.len() > MAX_PACKETS {
            return Err(CodecError::Container("Theora: too many frames".into()));
        }
        let private = lace(&st.headers).ok_or_else(|| CodecError::Container("Theora: missing header packets".into()))?;
        let [id, comment, setup] = [&st.headers[0], &st.headers[1], &st.headers[2]];
        let dec = cfa4_theora::Decoder::new(id, comment, setup).map_err(|e| CodecError::Unsupported(format!("Theora: {e}")))?;
        let ti = dec.info();
        let rate = if ti.fps.0 > 0 && ti.fps.1 > 0 { FrameRate::new(i64::from(ti.fps.0), i64::from(ti.fps.1)) } else { FrameRate::FPS_25 };
        let mut sync = Vec::with_capacity(st.packets.len());
        let mut last = 0usize;
        for (i, p) in st.packets.iter().enumerate() {
            if p.size > 0 && p.prefix[0] & 0xc0 == 0 {
                last = i;
            }
            sync.push(last);
        }
        let count = i64::try_from(st.packets.len()).unwrap_or(i64::MAX);
        let duration = Tick::from_rational(count.saturating_mul(rate.den), 1, rate.num);
        let secs = duration.seconds();
        let len = bytes.0.len();
        let bitrate = (secs > 0.0).then(|| (len.saturating_mul(8) as f64 / secs) as u64);
        let pixel_format = match ti.chroma {
            cfa4_theora::Chroma::C420 => "YUV 4:2:0 8-bit",
            cfa4_theora::Chroma::C422 => "YUV 4:2:2 8-bit",
            cfa4_theora::Chroma::C444 => "YUV 4:4:4 8-bit",
        };
        let (primaries, matrix) = match ti.colorspace {
            1 => (6, 6),
            2 => (5, 5),
            _ => (2, 2),
        };
        let codes = crate::stream_color::ColorCodes { primaries, transfer: if primaries == 2 { 2 } else { 6 }, matrix, full_range: Some(false) };
        let par = if ti.aspect.0 == 0 || ti.aspect.1 == 0 { (1, 1) } else { ti.aspect };
        let (w, h) = (ti.pic_width, ti.pic_height);
        let audio = OggSource::open_reader(name, reader).ok();
        let audio_streams = audio.as_ref().map(|a| a.info().audio_streams.clone()).unwrap_or_default();
        let info = MediaInfo {
            name: name.to_string(),
            kind: MediaKind::Movie,
            duration,
            video: Some(VideoStreamInfo {
                width: w,
                height: h,
                frame_rate: rate,
                par,
                codec: "Theora".into(),
                pixel_format: pixel_format.into(),
                color: crate::stream_color::resolve(w, h, &[codes]),
                has_alpha: false,
                bitrate,
                hdr: None,
            }),
            audio_streams,
            container: "Ogg".into(),
            start_timecode: None,
            file_size: Some(len),
        };
        Ok(Self { info, bytes, file, stream, private, sync, rate, video: GopCache::new(None), audio })
    }
}

struct Samples<'a>(&'a OggVideoSource);

impl VideoSamples for Samples<'_> {
    fn count(&self) -> usize {
        self.0.sync.len()
    }
    fn pts(&self, i: usize) -> i64 {
        i64::try_from(i).unwrap_or(i64::MAX)
    }
    fn sync_before(&self, i: usize) -> usize {
        self.0.sync.get(i).copied().unwrap_or(0)
    }
    fn sample_at(&self, t: i64) -> Option<usize> {
        let last = self.0.sync.len().checked_sub(1)?;
        Some(usize::try_from(t.max(0)).unwrap_or(last).min(last))
    }
    fn read(&self, i: usize) -> Result<Vec<u8>> {
        self.0.file.read_packet(&self.0.bytes, self.0.stream, i).map_err(|e| CodecError::Container(e.to_string()))
    }
    fn make_decoder(&self) -> Result<Box<dyn VideoDecoder>> {
        Ok(Box::new(crate::theora::TheoraDecoder::new(&self.0.private)?))
    }
}

impl MediaSource for OggVideoSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video_frame(&self, req: FrameRequest) -> std::result::Result<Arc<VideoFrame>, MediaError> {
        let i = self.rate.frame_at(req.time.max(Tick::ZERO));
        Ok(self.video.frame(&Samples(self), i)?)
    }

    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> std::result::Result<AudioBuffer, MediaError> {
        match &self.audio {
            Some(a) => a.audio(start, frames, sample_rate),
            None => Err(MediaError::NoStream("audio")),
        }
    }
}

/// Whether `head` is an Ogg file with a Theora stream.
fn handles(head: &[u8]) -> bool {
    cfa4_ogg::sniff(head) && head.windows(7).any(|w| w == b"\x80theora")
}

pub fn opener(name: &str, bytes: Arc<[u8]>) -> Option<std::result::Result<SharedSource, MediaError>> {
    if !handles(&bytes[..bytes.len().min(4096)]) {
        return None;
    }
    Some(OggVideoSource::open(name, bytes).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

pub fn reader_opener(name: &str, head: &[u8], reader: &cfa4_media::SharedReader) -> Option<std::result::Result<SharedSource, MediaError>> {
    if !handles(&head[..head.len().min(4096)]) {
        return None;
    }
    Some(OggVideoSource::open_reader(name, reader.clone()).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lacing_round_trips_through_the_matroska_split() {
        let h = vec![vec![1u8; 300], vec![2u8; 255], vec![3u8; 7]];
        let p = lace(&h).unwrap_or_default();
        assert_eq!(&p[..5], &[2, 255, 45, 255, 0]);
        assert_eq!(p.len(), 5 + 300 + 255 + 7);
        assert!(lace(&h[..2]).is_none());
    }

    #[test]
    fn other_ogg_is_not_claimed() {
        assert!(!handles(b""));
        assert!(!handles(b"OggS\0\x02 OpusHead"));
    }
}
