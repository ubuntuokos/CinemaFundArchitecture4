//! RealMedia files (`.rm`, `.rmvb`) with RealVideo 1.0 / 2.0 / 3.0 / 4.0 video: `cfa4-realmedia`
//! demuxes, our `cfa4-realvideo` decodes (CB-13.1, CB-13.2).
//!
//! RealMedia stamps packets in decode order, so the timeline is the frame count at the stream's
//! frame rate: at open every video frame is located (file offset of its first data packet) and
//! its display position worked out from the picture types (a B-frame is shown before the
//! reference picture decoded just before it). Audio streams (Cook, ATRAC3, RealAudio 1.0/2.0, …)
//! are not decoded yet.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use cfa4_frame::{Chroma, PixelData, VideoFrame};
use cfa4_media::{FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, SharedSource, VideoStreamInfo};
use cfa4_realmedia::{Demuxer, Header, StreamKind};
use cfa4_realvideo::{PictType, Picture, rv34};
use cfa4_time::{FrameRate, Tick};

use crate::gop::{GopCache, VideoSamples};
use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// Most video frames indexed (a day at 30 fps).
const MAX_FRAMES: usize = 30 * 60 * 60 * 24;

pub fn sniff(head: &[u8]) -> bool {
    cfa4_realmedia::probe(head)
}

/// The RealVideo versions CFA4 decodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Codec {
    /// RealVideo 1.0 / 2.0 (H.263 based).
    Rv12(cfa4_realvideo::Codec),
    /// RealVideo 3.0 / 4.0 (H.264 like).
    Rv34(rv34::Codec),
}

fn codec_of(fourcc: &[u8; 4]) -> Option<Codec> {
    match fourcc {
        b"RV10" => Some(Codec::Rv12(cfa4_realvideo::Codec::Rv10)),
        b"RV20" => Some(Codec::Rv12(cfa4_realvideo::Codec::Rv20)),
        b"RV30" => Some(Codec::Rv34(rv34::Codec::Rv30)),
        b"RV40" => Some(Codec::Rv34(rv34::Codec::Rv40)),
        _ => None,
    }
}

fn label(codec: Codec) -> &'static str {
    match codec {
        Codec::Rv12(cfa4_realvideo::Codec::Rv10) => "RealVideo 1.0",
        Codec::Rv12(cfa4_realvideo::Codec::Rv20) => "RealVideo 2.0",
        Codec::Rv34(rv34::Codec::Rv30) => "RealVideo 3.0",
        Codec::Rv34(rv34::Codec::Rv40) => "RealVideo 4.0",
    }
}

fn picture_type(codec: Codec, pkt: &[u8]) -> Option<PictType> {
    match codec {
        Codec::Rv12(c) => cfa4_realvideo::picture_type(c, pkt),
        Codec::Rv34(c) => rv34::picture_type(c, pkt),
    }
}

/// Either decoder.
enum AnyDecoder {
    Rv12(Box<cfa4_realvideo::Decoder>),
    Rv34(Box<rv34::Decoder>),
}

impl AnyDecoder {
    fn new(codec: Codec, width: u32, height: u32, extradata: &[u8]) -> std::result::Result<Self, cfa4_realvideo::Error> {
        Ok(match codec {
            Codec::Rv12(c) => Self::Rv12(Box::new(cfa4_realvideo::Decoder::new(c, width, height, extradata)?)),
            Codec::Rv34(c) => Self::Rv34(Box::new(rv34::Decoder::new(c, width, height, extradata)?)),
        })
    }
    fn decode(&mut self, pkt: &[u8]) -> std::result::Result<Option<Picture>, cfa4_realvideo::Error> {
        match self {
            Self::Rv12(d) => d.decode(pkt),
            Self::Rv34(d) => d.decode(pkt),
        }
    }
    fn flush(&mut self) -> Option<Picture> {
        match self {
            Self::Rv12(d) => d.flush(),
            Self::Rv34(d) => d.flush(),
        }
    }
    fn frames_started(&self) -> u64 {
        match self {
            Self::Rv12(d) => d.frames_started(),
            Self::Rv34(d) => d.frames_started(),
        }
    }
}

/// One video frame of the file, in decode order.
#[derive(Clone, Copy, Debug)]
struct Sample {
    /// File offset of the data packet the frame starts in.
    pos: u64,
    /// Which video frame of the stream starting in that packet this is (several small frames can
    /// share a packet).
    nth: u32,
    /// Display position (frame number).
    pts: i64,
    pict: PictType,
}

pub struct RmSource {
    info: MediaInfo,
    bytes: Arc<[u8]>,
    header: Header,
    stream: usize,
    codec: Codec,
    width: u32,
    height: u32,
    extradata: Vec<u8>,
    rate: FrameRate,
    samples: Vec<Sample>,
    /// Sample index of each display position.
    by_pts: Vec<usize>,
    video: GopCache,
}

fn container(e: impl std::fmt::Display) -> CodecError {
    CodecError::Container(format!("RealMedia: {e}"))
}

/// Display positions for pictures in decode order: a B-frame is shown at once, a reference
/// picture when the next reference picture arrives (or at the end).
fn display_order(types: &[PictType]) -> Vec<i64> {
    let mut pts = vec![0i64; types.len()];
    let mut next = 0i64;
    let mut held: Option<usize> = None;
    for (i, t) in types.iter().enumerate() {
        if *t == PictType::B {
            if let Some(p) = pts.get_mut(i) {
                *p = next;
            }
            next += 1;
        } else {
            if let Some(h) = held.and_then(|h| pts.get_mut(h)) {
                *h = next;
                next += 1;
            }
            held = Some(i);
        }
    }
    if let Some(h) = held.and_then(|h| pts.get_mut(h)) {
        *h = next;
    }
    pts
}

impl RmSource {
    pub fn open(name: &str, bytes: Arc<[u8]>) -> Result<Self> {
        let mut dm = Demuxer::new(Cursor::new(bytes.clone())).map_err(container)?;
        let header = dm.header().clone();
        let found = header.streams.iter().enumerate().find_map(|(i, s)| match &s.kind {
            StreamKind::Video { fourcc, width, height, fps_16_16, extradata } => Some((i, *fourcc, *width, *height, *fps_16_16, extradata.clone())),
            _ => None,
        });
        let Some((stream, fourcc, width, height, fps, extradata)) = found else {
            return Err(CodecError::Unsupported(format!("{name}: RealMedia without a video stream (RealAudio is not decoded yet)")));
        };
        let Some(codec) = codec_of(&fourcc) else {
            return Err(CodecError::Unsupported(format!("{name}: RealMedia video {} (CFA4 decodes RealVideo 1.0 to 4.0)", String::from_utf8_lossy(&fourcc))));
        };
        // the decoder checks the stream parameters
        AnyDecoder::new(codec, u32::from(width), u32::from(height), &extradata)
            .map_err(|e| CodecError::Unsupported(format!("{name}: {}: {e}", label(codec))))?;
        let mut types = Vec::new();
        let mut places = Vec::new();
        let mut last_pos = u64::MAX;
        let mut nth = 0u32;
        while places.len() < MAX_FRAMES {
            let p = match dm.next_packet() {
                Ok(Some(p)) => p,
                // a damaged tail ends the index
                Ok(None) | Err(_) => break,
            };
            if p.stream != stream || p.partial {
                continue;
            }
            let Some(t) = picture_type(codec, &p.data) else { continue };
            nth = if p.pos == last_pos { nth.saturating_add(1) } else { 0 };
            last_pos = p.pos;
            places.push((p.pos, nth));
            types.push(t);
        }
        if places.is_empty() {
            return Err(container(format!("{name}: no video frames")));
        }
        let pts = display_order(&types);
        let samples: Vec<Sample> = places.iter().zip(&types).zip(&pts).map(|((&(pos, nth), &pict), &pts)| Sample { pos, nth, pts, pict }).collect();
        let mut by_pts = vec![0usize; samples.len()];
        for (i, s) in samples.iter().enumerate() {
            if let Some(slot) = usize::try_from(s.pts).ok().and_then(|p| by_pts.get_mut(p)) {
                *slot = i;
            }
        }
        // 16.16 fixed point; RealProducer writes whole rates, keep it exact when it is one, and
        // take a standard rate the field approximates (29.97 is 0x1DF852) as that rate
        let rate = if fps == 0 {
            FrameRate::new(25, 1)
        } else if fps & 0xFFFF == 0 {
            FrameRate::new(i64::from(fps >> 16), 1)
        } else {
            let exact = FrameRate::new(i64::from(fps), 65536);
            FrameRate::COMMON.into_iter().find(|r| (r.as_f64() - exact.as_f64()).abs() < 0.005).unwrap_or(exact)
        };
        let frames = samples.len() as i64;
        // the start of the frame after the last, so every frame lies inside the duration
        let duration = rate.tick_of(frames);
        let color = crate::stream_color::resolve(u32::from(width), u32::from(height), &[]);
        let bitrate = u64::try_from(duration.to_rational_round(1, 1)).ok().filter(|&s| s > 0).map(|s| bytes.len() as u64 * 8 / s);
        let info = MediaInfo {
            name: name.to_string(),
            kind: MediaKind::Movie,
            duration,
            video: Some(VideoStreamInfo {
                width: u32::from(width),
                height: u32::from(height),
                frame_rate: rate,
                par: (1, 1),
                codec: label(codec).into(),
                pixel_format: "YUV 4:2:0 8-bit, progressive".into(),
                color,
                has_alpha: false,
                bitrate,
                hdr: None,
            }),
            audio_streams: Vec::new(),
            container: "RealMedia".into(),
            start_timecode: None,
            file_size: Some(bytes.len() as u64),
        };
        Ok(Self {
            info,
            bytes,
            header,
            stream,
            codec,
            width: u32::from(width),
            height: u32::from(height),
            extradata,
            rate,
            samples,
            by_pts,
            video: GopCache::new(None),
        })
    }

    fn read(&self, i: usize) -> Result<Vec<u8>> {
        let s = self.samples.get(i).ok_or_else(|| container("frame index out of range"))?;
        let mut dm = Demuxer::from_header(Cursor::new(self.bytes.clone()), self.header.clone()).map_err(container)?;
        dm.seek(s.pos).map_err(container)?;
        let mut seen = 0u32;
        // bounded: the frame starts in the packet at `pos`
        for _ in 0..4096 {
            let Some(p) = dm.next_packet().map_err(container)? else { break };
            if p.stream != self.stream || p.partial || p.pos != s.pos {
                continue;
            }
            if seen == s.nth {
                return Ok(p.data);
            }
            seen += 1;
        }
        Err(container("video frame not found"))
    }
}

/// [`cfa4_realvideo::Decoder`] or [`rv34::Decoder`] as a [`VideoDecoder`]: pictures come out in display order and
/// are matched to their samples by decode index.
pub struct RealVideoDecoder {
    dec: AnyDecoder,
    codec: Codec,
    width: u32,
    height: u32,
    extradata: Vec<u8>,
    rate: FrameRate,
    pts: HashMap<u64, i64>,
}

impl RealVideoDecoder {
    fn new(codec: Codec, width: u32, height: u32, extradata: &[u8], rate: FrameRate) -> Result<Self> {
        let dec = AnyDecoder::new(codec, width, height, extradata).map_err(|e| CodecError::Unsupported(format!("{}: {e}", label(codec))))?;
        Ok(Self { dec, codec, width, height, extradata: extradata.to_vec(), rate, pts: HashMap::new() })
    }

    fn frame(&mut self, p: Picture) -> Option<DecodedFrame> {
        let pts = self.pts.remove(&p.decode_index)?;
        let color = crate::stream_color::resolve(p.width, p.height, &[]);
        let data = PixelData::Yuv8 { planes: [Arc::new(p.y), Arc::new(p.u), Arc::new(p.v)], chroma: Chroma::C420, alpha: None };
        let frame = VideoFrame {
            width: p.width,
            height: p.height,
            data,
            color,
            par: (1, 1),
            pts: Tick::from_rational(pts, self.rate.den, self.rate.num),
            layout: Default::default(),
        };
        Some(DecodedFrame { pts, frame, draft: false })
    }
}

impl VideoDecoder for RealVideoDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let started = self.dec.frames_started();
        let r = self.dec.decode(sample);
        if self.dec.frames_started() > started {
            self.pts.insert(started, pts);
        }
        // a stale entry (a frame that never came out) must not grow the map
        if self.pts.len() > 16 {
            let keep = self.dec.frames_started().saturating_sub(16);
            self.pts.retain(|&k, _| k >= keep);
        }
        match r {
            Ok(Some(p)) => Ok(self.frame(p).into_iter().collect()),
            Ok(None) => Ok(Vec::new()),
            // a B-frame decoded right after the I-frame a decode started at refers to a picture
            // before it: nothing references a B-frame, so it is left out (as ffmpeg does)
            Err(_) if self.is_disposable(sample) => Ok(Vec::new()),
            Err(e) => Err(CodecError::Decode(format!("{}: {e}", label(self.codec)))),
        }
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        self.dec.flush().and_then(|p| self.frame(p)).into_iter().collect()
    }
    fn reset(&mut self) {
        if let Ok(d) = AnyDecoder::new(self.codec, self.width, self.height, &self.extradata) {
            self.dec = d;
        }
        self.pts.clear();
    }
    fn name(&self) -> &str {
        "CFA4 RealVideo"
    }
    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        picture_type(self.codec, sample).map(|t| t == PictType::I)
    }
    fn is_disposable(&self, sample: &[u8]) -> bool {
        picture_type(self.codec, sample) == Some(PictType::B)
    }
}

struct RmVideo<'a>(&'a RmSource);

impl VideoSamples for RmVideo<'_> {
    fn count(&self) -> usize {
        self.0.samples.len()
    }
    fn pts(&self, i: usize) -> i64 {
        self.0.samples.get(i).map_or(0, |s| s.pts)
    }
    /// The I-frame at or before `i`; for a B-frame decoded right after that I-frame (it also
    /// needs the reference picture before it), the I-frame before that.
    fn sync_before(&self, i: usize) -> usize {
        let s = &self.0.samples;
        let is_i = |k: usize| s.get(k).is_some_and(|x| x.pict == PictType::I);
        let prev_i = |from: usize| (0..=from).rev().find(|&k| is_i(k)).unwrap_or(0);
        let sync = prev_i(i.min(s.len().saturating_sub(1)));
        let leading = s.get(i).is_some_and(|x| x.pict == PictType::B) && (sync + 1..i).all(|k| s.get(k).is_some_and(|x| x.pict == PictType::B));
        if leading && sync > 0 { prev_i(sync - 1) } else { sync }
    }
    fn sample_at(&self, t: i64) -> Option<usize> {
        let last = self.0.by_pts.len().checked_sub(1)?;
        let t = usize::try_from(t.max(0)).ok()?.min(last);
        self.0.by_pts.get(t).copied()
    }
    fn read(&self, i: usize) -> Result<Vec<u8>> {
        self.0.read(i)
    }
    fn make_decoder(&self) -> Result<Box<dyn VideoDecoder>> {
        let s = self.0;
        Ok(Box::new(RealVideoDecoder::new(s.codec, s.width, s.height, &s.extradata, s.rate)?))
    }
}

impl MediaSource for RmSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video_frame(&self, req: FrameRequest) -> std::result::Result<Arc<VideoFrame>, MediaError> {
        let t = req.time.max(Tick::ZERO);
        let target = self.rate.frame_at(t);
        let late = cfa4_media::cancel::catch_up().map(|m| self.rate.frame_at(t - m));
        Ok(self.video.frame_late(&RmVideo(self), target, late)?)
    }

    fn audio(&self, _start: i64, _frames: usize, _sample_rate: u32) -> std::result::Result<cfa4_frame::AudioBuffer, MediaError> {
        Err(MediaError::NoStream("audio"))
    }
}

pub fn opener(name: &str, bytes: Arc<[u8]>) -> Option<std::result::Result<SharedSource, MediaError>> {
    if !sniff(&bytes) {
        return None;
    }
    Some(RmSource::open(name, bytes).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_order_moves_b_frames_before_their_reference() {
        use PictType::{B, I, P};
        assert_eq!(display_order(&[I, P, B, P, B, P]), vec![0, 2, 1, 4, 3, 5]);
        assert_eq!(display_order(&[I, P, P]), vec![0, 1, 2]);
        assert_eq!(display_order(&[]), Vec::<i64>::new());
    }

    #[test]
    fn sniff_and_garbage() {
        assert!(!sniff(b""));
        assert!(!sniff(b"RIFF\0\0\0\0AVI "));
        assert!(opener("x.rm", Arc::from(&b".RMF\0\0\0\x12\0\0"[..])).is_some_and(|r| r.is_err()));
    }
}
