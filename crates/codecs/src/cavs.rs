//! Our pure-Rust AVS1-P2 (Chinese AVS, Jizhun profile) decoder (`cfa4-cavs`) on raw elementary
//! streams (`.avs`, `.cavs`, as FFmpeg's `cavsvideo` muxer writes them).
//!
//! The stream is cut into pictures as FFmpeg's parser cuts it. A raw stream has no timestamps:
//! the index shows the pictures in the order the decoder outputs them (B pictures at once, a
//! reference picture when the next one arrives, everything in decoding order with `low_delay`),
//! one per frame. Decoding starts at an I picture (as FFmpeg seeks); a B picture right after one
//! also needs the reference picture before it, so its sync point is the I picture before that.

use std::sync::Arc;

use cfa4_cavs::PictureType;
use cfa4_frame::{AudioBuffer, Chroma, PixelData, VideoFrame};
use cfa4_media::{FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, SharedSource, VideoStreamInfo};
use cfa4_time::{FrameRate, Tick};

use crate::gop::{GopCache, VideoSamples};
use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// The codec name shown for AVS1 streams.
pub const NAME: &str = "AVS1-P2 (Jizhun)";

/// The most pictures a raw stream may index (a day at 60 frames per second).
const MAX_UNITS: usize = 60 * 60 * 60 * 24;
/// How much of a stream [`sniff`] looks at.
pub const SNIFF_LEN: usize = 64 << 10;
/// The largest sequence header the index keeps (its fields take 18 bytes).
const SEQ_LEN: u64 = 64;
/// Bytes read per step when indexing.
const CHUNK: usize = 1 << 20;

/// Whether `head` starts a raw AVS1 stream (FFmpeg's `cavsvideo` probe) whose sequence header
/// the decoder takes.
pub fn sniff(head: &[u8]) -> bool {
    let head = head.get(..SNIFF_LEN).unwrap_or(head);
    cfa4_cavs::probe(head) && cfa4_cavs::stream_info(head).is_ok()
}

/// The pixel aspect ratio of `aspect_ratio_information` (1 square, 2 = 4:3, 3 = 16:9, 4 = 2.21:1
/// display aspect ratios).
pub fn par(code: u32, width: u32, height: u32) -> (u32, u32) {
    let (dw, dh) = match code {
        2 => (4u64, 3u64),
        3 => (16, 9),
        4 => (221, 100),
        _ => return (1, 1),
    };
    let (n, d) = (dw * u64::from(height), dh * u64::from(width));
    let g = gcd(n, d);
    if g == 0 {
        return (1, 1);
    }
    (u32::try_from(n / g).unwrap_or(1), u32::try_from(d / g).unwrap_or(1))
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// A decoded picture as a frame.
pub(crate) fn convert(f: cfa4_cavs::Frame, par: (u32, u32)) -> VideoFrame {
    let [y, u, v] = f.planes;
    let data = PixelData::Yuv8 { planes: [Arc::new(y), Arc::new(u), Arc::new(v)], chroma: Chroma::C420, alpha: None };
    let color = crate::stream_color::resolve(f.width, f.height, &[]);
    VideoFrame { width: f.width, height: f.height, data, color, par, pts: Tick::ZERO, layout: Default::default() }
}

fn decode_error(e: cfa4_cavs::Error) -> CodecError {
    match e {
        cfa4_cavs::Error::Unsupported(why) => CodecError::Unsupported(format!("AVS1: {why}")),
        e => CodecError::Decode(e.to_string()),
    }
}

/// An AVS1 decoder primed with the stream's first sequence header. Samples are packets as
/// [`cfa4_cavs::split_packets`] cuts them.
pub struct CavsDecoder {
    dec: cfa4_cavs::Decoder,
    seq: Vec<u8>,
    par: (u32, u32),
}

impl CavsDecoder {
    pub fn new(seq: Vec<u8>, par: (u32, u32)) -> Self {
        let mut d = Self { dec: cfa4_cavs::Decoder::new(), seq, par };
        d.reset();
        d
    }
}

impl VideoDecoder for CavsDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        if sample.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self
            .dec
            .decode_tagged(sample, pts)
            .map_err(decode_error)?
            .into_iter()
            .map(|f| DecodedFrame { pts: f.tag, frame: convert(f, self.par), draft: false })
            .collect())
    }

    fn flush(&mut self) -> Vec<DecodedFrame> {
        self.dec.flush().into_iter().map(|f| DecodedFrame { pts: f.tag, frame: convert(f, self.par), draft: false }).collect()
    }

    fn reset(&mut self) {
        self.dec = cfa4_cavs::Decoder::new();
        if let Err(e) = self.dec.decode(&self.seq) {
            log::warn!("AVS1: unreadable sequence header: {e}");
        }
    }

    fn name(&self) -> &str {
        "CFA4 AVS1"
    }

    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        Some(cfa4_cavs::picture_type(sample) == Some(PictureType::I))
    }

    fn is_disposable(&self, sample: &[u8]) -> bool {
        cfa4_cavs::picture_type(sample) == Some(PictureType::B)
    }
}

/// One picture of a raw stream.
#[derive(Clone, Copy)]
struct Unit {
    off: u64,
    size: u64,
    /// Decoding order index of the I picture to start from.
    sync: usize,
    /// Display position (frame index).
    shown: i64,
}

/// A packet found by the index: its byte range and the offset of its picture start code.
struct Packet {
    off: u64,
    end: u64,
    pic: Option<u64>,
}

/// Cuts the stream into packets ([`cfa4_cavs::split_packets`] over a reader) and finds the first
/// sequence header.
fn scan(reader: &dyn cfa4_media::reader::ByteReader, len: u64) -> Result<(Vec<Packet>, Option<u64>)> {
    let mut packets = Vec::new();
    let mut seq = None;
    let mut state = u32::MAX;
    let (mut start, mut pic, mut pic_found) = (0u64, None, false);
    let mut pos = 0u64;
    while pos < len {
        let n = usize::try_from((len - pos).min(CHUNK as u64)).unwrap_or(CHUNK);
        let chunk = cfa4_media::reader::read_range(reader, pos, n).map_err(|e| CodecError::Container(e.to_string()))?;
        if chunk.is_empty() {
            break;
        }
        for (k, &b) in chunk.iter().enumerate() {
            state = (state << 8) | u32::from(b);
            if state & 0xffff_ff00 != 0x100 {
                continue;
            }
            let at = (pos + k as u64).saturating_sub(3);
            if b == 0xb0 && seq.is_none() {
                seq = Some(at);
            }
            if !pic_found {
                if cfa4_cavs::is_picture_start(b) {
                    pic_found = true;
                    pic = Some(at);
                }
            } else if cfa4_cavs::is_packet_start(b) {
                packets.push(Packet { off: start, end: at, pic });
                if packets.len() > MAX_UNITS {
                    return Err(CodecError::Container("AVS1: too many pictures".into()));
                }
                start = at;
                pic_found = cfa4_cavs::is_picture_start(b);
                pic = pic_found.then_some(at);
            }
        }
        pos += chunk.len() as u64;
    }
    if start < len {
        packets.push(Packet { off: start, end: len, pic });
    }
    Ok((packets, seq))
}

/// A raw AVS1 stream.
pub struct CavsSource {
    info: MediaInfo,
    bytes: crate::Src,
    units: Vec<Unit>,
    /// Decoding order index of the picture shown as frame `i`.
    by_frame: Vec<usize>,
    seq: Vec<u8>,
    par: (u32, u32),
    rate: FrameRate,
    video: GopCache,
}

impl CavsSource {
    pub fn open(name: &str, bytes: Arc<[u8]>) -> Result<Self> {
        Self::open_reader(name, Arc::new(cfa4_media::reader::MemReader(bytes)))
    }

    pub fn open_reader(name: &str, reader: cfa4_media::SharedReader) -> Result<Self> {
        let len = reader.len();
        let read = |off: u64, n: u64| -> Result<Vec<u8>> {
            let n = usize::try_from(n).map_err(|_| CodecError::Container("AVS1: read too large".into()))?;
            cfa4_media::reader::read_range(&*reader, off, n).map_err(|e| CodecError::Container(e.to_string()))
        };
        let (packets, seq_at) = scan(&*reader, len)?;
        let seq_at = seq_at.ok_or_else(|| CodecError::Container("AVS1: no sequence header".into()))?;
        let mut seq = read(seq_at, SEQ_LEN.min(len - seq_at))?;
        // only the header: what follows it (a picture) must not reach the decoder with it
        if let Some(end) = seq.windows(3).skip(4).position(|w| w == [0, 0, 1]) {
            seq.truncate(end + 4);
        }
        let si = cfa4_cavs::stream_info(&seq).map_err(decode_error)?;

        // the pictures, their types and the order the decoder outputs them in
        // (pictures the decoder cannot decode in a run from the start are left out: those before
        // the first I picture, and the B pictures after it that still lack a reference)
        let mut kinds = Vec::new();
        let mut ranges = Vec::new();
        let mut refs = None::<u8>;
        for p in &packets {
            let Some(at) = p.pic else { continue };
            let head = read(at, 7.min(len - at))?;
            let Some(kind) = cfa4_cavs::picture_type(&head) else { continue };
            let decodable = match (kind, refs) {
                (PictureType::I, _) => true,
                (_, None) => false,
                (PictureType::P, Some(n)) => n >= 1,
                (PictureType::B, Some(n)) => n >= 2,
            };
            if !decodable {
                continue;
            }
            if kind != PictureType::B {
                refs = Some(refs.unwrap_or(0).saturating_add(1).min(2));
            }
            kinds.push(kind);
            ranges.push((p.off, p.end));
        }
        if kinds.is_empty() {
            return Err(CodecError::Container("AVS1: no pictures".into()));
        }
        let mut order = Vec::with_capacity(kinds.len());
        let mut held: Option<usize> = None;
        for (i, &k) in kinds.iter().enumerate() {
            if si.low_delay || k == PictureType::B {
                order.push(i);
            } else if let Some(prev) = held.replace(i) {
                order.push(prev);
            }
        }
        order.extend(held);
        // sync points: an I picture itself; for a P picture the I picture at or before its
        // reference (P pictures do not refer back across an I picture, as FFmpeg assumes when it
        // seeks); for a B picture the one at or before the older of its two references (a B
        // picture right after an I picture still uses the reference before it)
        let mut units = Vec::with_capacity(kinds.len());
        let (mut last_i, mut refs) = (0usize, [None::<usize>; 2]);
        let mut i_before = Vec::with_capacity(kinds.len());
        for (i, &k) in kinds.iter().enumerate() {
            let sync = match k {
                PictureType::I => i,
                PictureType::P => i_before.get(refs[0].unwrap_or(0)).copied().unwrap_or(0),
                PictureType::B => i_before.get(refs[1].or(refs[0]).unwrap_or(0)).copied().unwrap_or(0),
            };
            if k == PictureType::I {
                last_i = i;
            }
            i_before.push(last_i);
            if k != PictureType::B {
                refs = [Some(i), refs[0]];
            }
            let (off, end) = ranges.get(i).copied().unwrap_or((0, 0));
            units.push(Unit { off, size: end - off, sync, shown: 0 });
        }
        for (frame, &i) in order.iter().enumerate() {
            if let Some(u) = units.get_mut(i) {
                u.shown = i64::try_from(frame).unwrap_or(i64::MAX);
            }
        }

        let rate =
            if si.frame_rate.0 > 0 && si.frame_rate.1 > 0 { FrameRate::new(i64::from(si.frame_rate.0), i64::from(si.frame_rate.1)) } else { FrameRate::FPS_25 };
        let count = i64::try_from(units.len()).unwrap_or(i64::MAX);
        let duration = Tick::from_rational(count.saturating_mul(rate.den), 1, rate.num);
        let secs = duration.seconds();
        let bitrate = (secs > 0.0).then(|| (len.saturating_mul(8) as f64 / secs) as u64);
        let color = crate::stream_color::resolve(si.width, si.height, &[]);
        let par = par(si.aspect_ratio, si.width, si.height);
        let info = MediaInfo {
            name: name.to_string(),
            kind: MediaKind::Movie,
            duration,
            video: Some(VideoStreamInfo {
                width: si.width,
                height: si.height,
                frame_rate: rate,
                par,
                codec: NAME.into(),
                pixel_format: "YUV 4:2:0 8-bit".into(),
                color,
                has_alpha: false,
                bitrate,
                hdr: None,
            }),
            audio_streams: Vec::new(),
            container: "AVS1 elementary stream".into(),
            start_timecode: None,
            file_size: Some(len),
        };
        Ok(Self { info, bytes: crate::Src(reader), units, by_frame: order, seq, par, rate, video: GopCache::new(None) })
    }
}

struct CavsVideo<'a>(&'a CavsSource);

impl VideoSamples for CavsVideo<'_> {
    fn count(&self) -> usize {
        self.0.units.len()
    }
    fn pts(&self, i: usize) -> i64 {
        self.0.units.get(i).map_or(i64::MAX, |u| u.shown)
    }
    fn sync_before(&self, i: usize) -> usize {
        let i = i.min(self.0.units.len().saturating_sub(1));
        self.0.units.get(i).map_or(0, |u| u.sync)
    }
    fn sample_at(&self, t: i64) -> Option<usize> {
        let last = self.0.by_frame.len().checked_sub(1)?;
        let f = usize::try_from(t.max(0)).unwrap_or(last).min(last);
        self.0.by_frame.get(f).copied()
    }
    fn read(&self, i: usize) -> Result<Vec<u8>> {
        let u = self.0.units.get(i).ok_or_else(|| CodecError::Container("sample index out of range".into()))?;
        let sz = usize::try_from(u.size).map_err(|_| CodecError::Container("AVS1: picture too large".into()))?;
        cfa4_media::reader::read_range(&*self.0.bytes.0, u.off, sz).map_err(|e| CodecError::Container(e.to_string()))
    }
    fn make_decoder(&self) -> Result<Box<dyn VideoDecoder>> {
        Ok(Box::new(CavsDecoder::new(self.0.seq.clone(), self.0.par)))
    }
}

impl MediaSource for CavsSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video_frame(&self, req: FrameRequest) -> std::result::Result<Arc<VideoFrame>, MediaError> {
        let i = self.rate.frame_at(req.time.max(Tick::ZERO));
        Ok(self.video.frame(&CavsVideo(self), i)?)
    }

    fn audio(&self, _start: i64, _frames: usize, _sample_rate: u32) -> std::result::Result<AudioBuffer, MediaError> {
        Err(MediaError::NoStream("audio"))
    }
}

pub fn opener(name: &str, bytes: Arc<[u8]>) -> Option<std::result::Result<SharedSource, MediaError>> {
    if !sniff(&bytes) {
        return None;
    }
    Some(CavsSource::open(name, bytes).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

pub fn reader_opener(name: &str, head: &[u8], reader: &cfa4_media::SharedReader) -> Option<std::result::Result<SharedSource, MediaError>> {
    if !sniff(head) {
        return None;
    }
    Some(CavsSource::open_reader(name, reader.clone()).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_streams_are_not_sniffed() {
        assert!(!sniff(b""));
        // an MPEG-2 sequence header and picture
        assert!(!sniff(&[0, 0, 1, 0xb3, 0x16, 0, 0xf0, 0x15, 0, 0, 1, 0xb8, 0, 0, 1, 0, 0, 0x0f]));
        // an MPEG-4 visual object sequence with a VOP
        assert!(!sniff(&[0, 0, 1, 0xb0, 0xf5, 0, 0, 1, 0xb5, 0x09, 0, 0, 1, 0, 0, 0, 1, 0x20, 0, 0, 1, 0xb6, 0x10]));
    }

    #[test]
    fn aspect_ratios() {
        assert_eq!(par(1, 720, 576), (1, 1));
        assert_eq!(par(2, 720, 576), (16, 15));
        assert_eq!(par(3, 1920, 1080), (1, 1));
        assert_eq!(par(9, 720, 576), (1, 1));
        assert_eq!(par(3, 0, 0), (1, 1));
    }
}
