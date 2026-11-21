//! Our pure-Rust AVS3-P2 (IEEE 1857.10, GB/T 33475.2) Main-8 / Main-10 decoder (`cfa4-avs3`) on raw
//! elementary streams (`.avs3`, as xavs3 and FFmpeg's `avs3` muxer write them).
//!
//! The stream is cut into pictures as davs3 cuts it. A raw stream has no timestamps: the index
//! derives every picture's order count from its header (coding order index plus output delay)
//! and shows the pictures in that order, one per frame. Decoding starts at an I picture: a
//! picture's sync point is the earliest I picture its references lead back to. Pictures the
//! decoder cannot decode from the start of the stream (before the first I picture, or with a
//! reference before it) and picture kinds it does not decode are left out.

use std::collections::HashMap;
use std::sync::Arc;

use cfa4_avs3::{PictureType, SequenceHeader};
use cfa4_frame::{AudioBuffer, Chroma, PixelData, VideoFrame};
use cfa4_media::{FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, SharedSource, VideoStreamInfo};
use cfa4_time::{FrameRate, Tick};

use crate::gop::{GopCache, VideoSamples};
use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// The codec name shown for AVS3 streams.
pub const NAME: &str = "AVS3-P2";

/// The most pictures a raw stream may index (a day at 60 frames per second).
const MAX_UNITS: usize = 60 * 60 * 60 * 24;
/// How much of a stream [`sniff`] looks at.
pub const SNIFF_LEN: usize = 64 << 10;
/// The most of a stream's start the sequence header is looked for in, and its largest size
/// (weighting matrices included it stays far below).
const SEQ_LEN: u64 = 4096;
/// How much of a picture the index reads for its header first (all of it if that is not enough).
const PIC_HEAD: u64 = 4096;
/// The largest picture packet the index reads whole.
const MAX_PACKET: u64 = 1 << 30;
/// Bytes read per step when indexing.
const CHUNK: usize = 1 << 20;

/// Whether `head` starts a raw AVS3 stream whose sequence header the decoder takes.
pub fn sniff(head: &[u8]) -> bool {
    let head = head.get(..SNIFF_LEN).unwrap_or(head);
    cfa4_avs3::probe(head) && cfa4_avs3::stream_info(head).is_ok()
}

/// The pixel aspect ratio of `aspect_ratio_information` (1 square, 2 = 4:3, 3 = 16:9, 4 = 2.21:1
/// display aspect ratios).
pub fn par(code: u32, width: u32, height: u32) -> (u32, u32) {
    crate::cavs::par(code, width, height)
}

/// A decoded picture as a frame.
fn convert(f: cfa4_avs3::Frame, par: (u32, u32)) -> VideoFrame {
    let [y, u, v] = f.planes;
    let data = if f.bit_depth > 8 {
        PixelData::Yuv16 { planes: [Arc::new(y), Arc::new(u), Arc::new(v)], chroma: Chroma::C420, bits: f.bit_depth, alpha: None }
    } else {
        let narrow = |p: Vec<u16>| Arc::new(p.into_iter().map(|s| u8::try_from(s).unwrap_or(u8::MAX)).collect::<Vec<u8>>());
        PixelData::Yuv8 { planes: [narrow(y), narrow(u), narrow(v)], chroma: Chroma::C420, alpha: None }
    };
    let color = crate::stream_color::resolve(f.width, f.height, &[]);
    VideoFrame { width: f.width, height: f.height, data, color, par, pts: Tick::ZERO, layout: Default::default() }
}

fn decode_error(e: cfa4_avs3::Error) -> CodecError {
    match e {
        cfa4_avs3::Error::Unsupported(why) => CodecError::Unsupported(format!("AVS3: {why}")),
        e => CodecError::Decode(e.to_string()),
    }
}

fn is_intra_packet(sample: &[u8]) -> bool {
    sample.windows(4).find(|w| w[0] == 0 && w[1] == 0 && w[2] == 1 && cfa4_avs3::is_picture_start(w[3])).is_some_and(|w| w[3] == 0xB3)
}

/// An AVS3 decoder primed with the stream's first sequence header. Samples are packets as
/// [`cfa4_avs3::split_packets`] cuts them.
pub struct Avs3Decoder {
    dec: cfa4_avs3::Decoder,
    seq: Vec<u8>,
    par: (u32, u32),
}

impl Avs3Decoder {
    pub fn new(seq: Vec<u8>, par: (u32, u32)) -> Self {
        let mut d = Self { dec: cfa4_avs3::Decoder::new(), seq, par };
        d.reset();
        d
    }
}

impl VideoDecoder for Avs3Decoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        if sample.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self
            .dec
            .decode(sample, pts)
            .map_err(decode_error)?
            .into_iter()
            .map(|f| DecodedFrame { pts: f.tag, frame: convert(f, self.par), draft: false })
            .collect())
    }

    fn flush(&mut self) -> Vec<DecodedFrame> {
        self.dec.flush().into_iter().map(|f| DecodedFrame { pts: f.tag, frame: convert(f, self.par), draft: false }).collect()
    }

    fn reset(&mut self) {
        self.dec = cfa4_avs3::Decoder::new();
        if let Err(e) = self.dec.decode(&self.seq, -1) {
            log::warn!("AVS3: unreadable sequence header: {e}");
        }
    }

    fn name(&self) -> &str {
        "CFA4 AVS3"
    }

    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        Some(is_intra_packet(sample))
    }

    fn is_disposable(&self, _sample: &[u8]) -> bool {
        // AVS3 B pictures may be references (hierarchical GOPs): none is skipped.
        false
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

/// Cuts the stream into packets ([`cfa4_avs3::split_packets`] over a reader) and finds the first
/// sequence header.
fn scan(reader: &dyn cfa4_media::reader::ByteReader, len: u64) -> Result<(Vec<Packet>, Option<u64>)> {
    let mut packets = Vec::new();
    let mut seq = None;
    let mut state = u32::MAX;
    let (mut start, mut pic) = (0u64, None);
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
            if b == 0xB0 && seq.is_none() {
                seq = Some(at);
            }
            if cfa4_avs3::is_packet_start(b) && at > start {
                packets.push(Packet { off: start, end: at, pic });
                if packets.len() > MAX_UNITS {
                    return Err(CodecError::Container("AVS3: too many pictures".into()));
                }
                start = at;
                pic = None;
            }
            if cfa4_avs3::is_picture_start(b) && pic.is_none() {
                pic = Some(at);
            }
        }
        pos += chunk.len() as u64;
    }
    if start < len {
        packets.push(Packet { off: start, end: len, pic });
    }
    Ok((packets, seq))
}

/// A raw AVS3 stream.
pub struct Avs3Source {
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

impl Avs3Source {
    pub fn open(name: &str, bytes: Arc<[u8]>) -> Result<Self> {
        Self::open_reader(name, Arc::new(cfa4_media::reader::MemReader(bytes)))
    }

    pub fn open_reader(name: &str, reader: cfa4_media::SharedReader) -> Result<Self> {
        let len = reader.len();
        let read = |off: u64, n: u64| -> Result<Vec<u8>> {
            let n = usize::try_from(n).map_err(|_| CodecError::Container("AVS3: read too large".into()))?;
            cfa4_media::reader::read_range(&*reader, off, n).map_err(|e| CodecError::Container(e.to_string()))
        };
        let (packets, seq_at) = scan(&*reader, len)?;
        let seq_at = seq_at.ok_or_else(|| CodecError::Container("AVS3: no sequence header".into()))?;
        let mut seq = read(seq_at, SEQ_LEN.min(len - seq_at))?;
        // only the header: what follows it (a picture) must not reach the decoder with it
        if let Some(end) = seq.windows(3).skip(4).position(|w| w == [0, 0, 1]) {
            seq.truncate(end + 4);
        }
        let header = SequenceHeader::parse(&seq).map_err(decode_error)?;
        let si = header.info();
        let reorder = i64::from(header.reorder_delay());

        // the pictures the decoder can decode from the start, their sync points and order counts
        let mut units = Vec::new();
        let mut pocs = Vec::new();
        let mut by_doi: HashMap<i64, usize> = HashMap::new();
        let (mut wrap, mut prev_doi) = (0i64, -1i64);
        for p in &packets {
            let Some(at) = p.pic else { continue };
            let head_len = (p.end - at).min(PIC_HEAD);
            let mut pic = header.picture(&read(at, head_len)?);
            if pic.is_none() && head_len < p.end - at && p.end - at <= MAX_PACKET {
                pic = header.picture(&read(at, p.end - at)?);
            }
            let Some(pic) = pic else { continue };
            let doi8 = i64::from(pic.doi);
            if doi8 < prev_doi {
                wrap += 1;
            }
            prev_doi = doi8;
            let doi = doi8 + wrap * 256;
            let sync = match pic.kind {
                PictureType::I => Some(units.len()),
                PictureType::P | PictureType::B if !pic.refs.is_empty() => pic
                    .refs
                    .iter()
                    .map(|&d| by_doi.get(&(doi - i64::from(d))).and_then(|&k| units.get(k)).map(|u: &Unit| u.sync))
                    .collect::<Option<Vec<usize>>>()
                    .and_then(|s| s.into_iter().min()),
                _ => None,
            };
            let Some(sync) = sync else { continue };
            by_doi.insert(doi, units.len());
            pocs.push(if si.low_delay { doi } else { doi + i64::from(pic.output_delay) - reorder });
            units.push(Unit { off: p.off, size: p.end - p.off, sync, shown: 0 });
        }
        if units.is_empty() {
            return Err(CodecError::Container("AVS3: no pictures".into()));
        }
        let mut order: Vec<usize> = (0..units.len()).collect();
        order.sort_by_key(|&i| (pocs.get(i).copied().unwrap_or(i64::MAX), i));
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
                pixel_format: format!("YUV 4:2:0 {}-bit", si.bit_depth),
                color,
                has_alpha: false,
                bitrate,
                hdr: None,
            }),
            audio_streams: Vec::new(),
            container: "AVS3 elementary stream".into(),
            start_timecode: None,
            file_size: Some(len),
        };
        Ok(Self { info, bytes: crate::Src(reader), units, by_frame: order, seq, par, rate, video: GopCache::new(None) })
    }
}

struct Avs3Video<'a>(&'a Avs3Source);

impl VideoSamples for Avs3Video<'_> {
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
        let sz = usize::try_from(u.size).map_err(|_| CodecError::Container("AVS3: picture too large".into()))?;
        cfa4_media::reader::read_range(&*self.0.bytes.0, u.off, sz).map_err(|e| CodecError::Container(e.to_string()))
    }
    fn make_decoder(&self) -> Result<Box<dyn VideoDecoder>> {
        Ok(Box::new(Avs3Decoder::new(self.0.seq.clone(), self.0.par)))
    }
}

impl MediaSource for Avs3Source {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video_frame(&self, req: FrameRequest) -> std::result::Result<Arc<VideoFrame>, MediaError> {
        let i = self.rate.frame_at(req.time.max(Tick::ZERO));
        Ok(self.video.frame(&Avs3Video(self), i)?)
    }

    fn audio(&self, _start: i64, _frames: usize, _sample_rate: u32) -> std::result::Result<AudioBuffer, MediaError> {
        Err(MediaError::NoStream("audio"))
    }
}

pub fn opener(name: &str, bytes: Arc<[u8]>) -> Option<std::result::Result<SharedSource, MediaError>> {
    if !sniff(&bytes) {
        return None;
    }
    Some(Avs3Source::open(name, bytes).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

pub fn reader_opener(name: &str, head: &[u8], reader: &cfa4_media::SharedReader) -> Option<std::result::Result<SharedSource, MediaError>> {
    if !sniff(head) {
        return None;
    }
    Some(Avs3Source::open_reader(name, reader.clone()).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_streams_are_not_sniffed() {
        assert!(!sniff(b""));
        // an AVS1 Jizhun sequence header and picture
        assert!(!sniff(&[0, 0, 1, 0xb0, 0x20, 0x42, 0x81, 0x68, 0x12, 0x02, 0, 0, 1, 0xb3, 0, 0, 0, 0]));
        // an MPEG-4 visual object sequence with a VOP
        assert!(!sniff(&[0, 0, 1, 0xb0, 0xf5, 0, 0, 1, 0xb5, 0x09, 0, 0, 1, 0, 0, 0, 1, 0x20, 0, 0, 1, 0xb6, 0x10]));
    }

    #[test]
    fn intra_packets() {
        assert!(is_intra_packet(&[0, 0, 1, 0xb3, 0, 0]));
        assert!(!is_intra_packet(&[0, 0, 1, 0xb6, 0, 0]));
        assert!(!is_intra_packet(&[0, 0, 1, 0xb0, 0, 0]));
    }
}
