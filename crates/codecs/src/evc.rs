//! Our pure-Rust MPEG-5 EVC Baseline decoder (`cfa4-evc`): raw `.evc` streams (a 4-byte
//! big-endian length before every NAL unit, as xeve writes them) and `evc1` tracks in MP4.
//!
//! A raw stream has no timestamps: the index derives every picture's order count from the
//! headers and shows the pictures in that order, one per frame. Decoding starts at an IDR
//! picture or at an intra picture (a clean random access point; the decoder drops its leading
//! pictures that cannot be decoded).

use std::sync::Arc;

use cfa4_frame::{AudioBuffer, Chroma, PixelData, VideoFrame};
use cfa4_isobmff::{CodecConfig, FourCc, SampleEntry};
use cfa4_media::{FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, SharedSource, VideoStreamInfo};
use cfa4_time::{FrameRate, Tick};

use crate::gop::{GopCache, VideoSamples};
use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// MP4 sample entry of EVC (ISO/IEC 14496-15 §12).
pub const EVC1: FourCc = FourCc(*b"evc1");

/// The most pictures a raw stream may index (a day at 60 frames per second).
const MAX_UNITS: usize = 60 * 60 * 60 * 24;
/// The largest NAL unit (far above an 8K intra picture).
const MAX_NAL: u64 = 1 << 30;
/// How much of a slice NAL unit the index reads for its header.
const SLICE_HEAD: u64 = 4096;
/// How much of a stream [`sniff`] looks at.
pub const SNIFF_LEN: usize = 64 << 10;

/// The codec name shown for EVC streams.
pub const NAME: &str = "MPEG-5 EVC (Baseline)";

/// Whether `head` starts a raw EVC stream: its first NAL unit is a sequence parameter set the
/// decoder accepts.
pub fn sniff(head: &[u8]) -> bool {
    let head = head.get(..SNIFF_LEN).unwrap_or(head);
    let first = cfa4_evc::split_nal_units(head).next();
    first.and_then(cfa4_evc::nal_type).is_some_and(|(t, _)| t == cfa4_evc::NUT_SPS) && cfa4_evc::probe(head).is_some()
}

/// The pixel format label of a stream.
pub fn pixel_format(info: &cfa4_evc::StreamInfo) -> String {
    match info.chroma {
        cfa4_evc::Chroma::Yuv420 => format!("YUV 4:2:0 {}-bit", info.bit_depth),
        cfa4_evc::Chroma::Mono => format!("Gray {}-bit", info.bit_depth),
    }
}

/// A decoded picture as a frame (4:0:0 gets neutral chroma).
pub(crate) fn convert(f: cfa4_evc::Frame) -> Result<VideoFrame> {
    let (w, h) = (f.width as usize, f.height as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let bits = f.bit_depth;
    let mut planes = f.planes.into_iter();
    let y = planes.next().ok_or_else(|| CodecError::Decode("EVC: picture without samples".into()))?;
    let mid = 1u16.checked_shl(bits.saturating_sub(1)).unwrap_or(0);
    let u = planes.next().unwrap_or_else(|| vec![mid; cw * ch]);
    let v = planes.next().unwrap_or_else(|| vec![mid; cw * ch]);
    let data = if bits == 8 {
        let narrow = |p: Vec<u16>| Arc::new(p.into_iter().map(|s| s.min(255) as u8).collect::<Vec<u8>>());
        PixelData::Yuv8 { planes: [narrow(y), narrow(u), narrow(v)], chroma: Chroma::C420, alpha: None }
    } else {
        PixelData::Yuv16 { planes: [Arc::new(y), Arc::new(u), Arc::new(v)], chroma: Chroma::C420, bits, alpha: None }
    };
    let color = crate::stream_color::resolve(f.width, f.height, &[]);
    Ok(VideoFrame { width: f.width, height: f.height, data, color, par: (1, 1), pts: Tick::ZERO, layout: Default::default() })
}

fn decode_error(e: cfa4_evc::Error) -> CodecError {
    match e {
        cfa4_evc::Error::Unsupported(why) => CodecError::Unsupported(format!("EVC: {why}")),
        e => CodecError::Decode(e.to_string()),
    }
}

/// An EVC decoder primed with the stream's parameter sets. Samples are NAL units, each after
/// its length in `length_size` bytes (4 in raw streams).
pub struct EvcDecoder {
    dec: cfa4_evc::Decoder,
    /// Parameter set NAL units (without lengths), given to every new decoder.
    params: Vec<Vec<u8>>,
    length_size: usize,
}

impl EvcDecoder {
    pub fn new(params: Vec<Vec<u8>>, length_size: usize) -> Self {
        let mut d = Self { dec: cfa4_evc::Decoder::new(), params, length_size };
        d.reset();
        d
    }

    fn nal_units<'a>(&self, sample: &'a [u8]) -> impl Iterator<Item = &'a [u8]> + 'a {
        let ls = self.length_size;
        let mut rest = sample;
        std::iter::from_fn(move || {
            if !(1..=4).contains(&ls) {
                return None;
            }
            let len = rest.get(..ls)?.iter().fold(0usize, |a, &b| (a << 8) | usize::from(b));
            let nal = rest.get(ls..ls.checked_add(len)?)?;
            rest = rest.get(ls + len..).unwrap_or(&[]);
            Some(nal)
        })
    }
}

impl VideoDecoder for EvcDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let mut out = Vec::new();
        let nals: Vec<&[u8]> = self.nal_units(sample).collect();
        for nal in nals {
            if let Some(f) = self.dec.decode_tagged(nal, pts).map_err(decode_error)? {
                let pts = f.tag;
                out.push(DecodedFrame { pts, frame: convert(f)?, draft: false });
            }
        }
        Ok(out)
    }

    fn flush(&mut self) -> Vec<DecodedFrame> {
        let mut out = Vec::new();
        for f in self.dec.flush() {
            let pts = f.tag;
            match convert(f) {
                Ok(frame) => out.push(DecodedFrame { pts, frame, draft: false }),
                Err(e) => log::warn!("EVC: {e}"),
            }
        }
        out
    }

    fn reset(&mut self) {
        self.dec = cfa4_evc::Decoder::new();
        for p in &self.params {
            if let Err(e) = self.dec.decode_nal(p) {
                log::warn!("EVC: unreadable parameter set: {e}");
            }
        }
    }

    fn name(&self) -> &str {
        "CFA4 EVC"
    }

    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        // An IDR picture always is; whether another picture is an intra one takes its slice
        // header (the index or the container's sync flags know).
        self.nal_units(sample).any(|n| cfa4_evc::nal_type(n).is_some_and(|(t, _)| t == cfa4_evc::NUT_IDR)).then_some(true)
    }
}

/// The parameter sets and length size of an `evc1` sample entry (its `evcC` box: ISO/IEC
/// 14496-15 §12.3.3, `EVCDecoderConfigurationRecord`).
pub fn evcc_params(entry_payload: &[u8]) -> Option<(Vec<Vec<u8>>, usize)> {
    // the visual sample entry fields come first (78 bytes), then the boxes
    let mut boxes = entry_payload.get(78..)?;
    while boxes.len() >= 8 {
        let size = u32::from_be_bytes(boxes.get(..4)?.try_into().ok()?) as usize;
        let kind = boxes.get(4..8)?;
        if size < 8 {
            return None;
        }
        let payload = boxes.get(8..size)?;
        if kind == b"evcC" {
            return parse_evcc(payload);
        }
        boxes = boxes.get(size..)?;
    }
    None
}

/// The NAL units and length size of an `EVCDecoderConfigurationRecord`.
pub fn parse_evcc(rec: &[u8]) -> Option<(Vec<Vec<u8>>, usize)> {
    // configurationVersion, profile_idc, level_idc, toolset_idc_h / _l, chroma format and bit
    // depths in one byte, width, height: 16 bytes
    let length_size = usize::from(rec.get(16)? & 3) + 1;
    let arrays = usize::from(*rec.get(17)?);
    let mut pos = 18usize;
    let mut out = Vec::new();
    for _ in 0..arrays {
        let n = usize::from(u16::from_be_bytes(rec.get(pos + 1..pos + 3)?.try_into().ok()?));
        pos += 3;
        for _ in 0..n {
            let len = usize::from(u16::from_be_bytes(rec.get(pos..pos + 2)?.try_into().ok()?));
            out.push(rec.get(pos + 2..pos + 2 + len)?.to_vec());
            pos += 2 + len;
        }
    }
    Some((out, length_size))
}

/// Decoder factory for MP4 `evc1` tracks.
pub fn evc_factory(e: &SampleEntry) -> Option<Result<Box<dyn VideoDecoder>>> {
    if e.format != EVC1 {
        return None;
    }
    let (params, length_size) = match &e.codec {
        CodecConfig::Unknown { raw, .. } => evcc_params(raw).unwrap_or((Vec::new(), 4)),
        _ => (Vec::new(), 4),
    };
    Some(Ok(Box::new(EvcDecoder::new(params, length_size))))
}

/// One picture of a raw stream.
#[derive(Clone, Copy)]
struct Unit {
    /// Byte range: the NAL units after the previous picture's slice up to and including this
    /// picture's slice (parameter sets ride with the picture that follows them).
    off: u64,
    size: u64,
    /// Decoding can start here.
    sync: bool,
    /// Display position (frame index).
    shown: i64,
}

/// A raw EVC stream.
pub struct EvcSource {
    info: MediaInfo,
    bytes: crate::Src,
    units: Vec<Unit>,
    /// Decode-order index of the picture shown as frame `i`.
    by_frame: Vec<usize>,
    params: Vec<Vec<u8>>,
    rate: FrameRate,
    video: GopCache,
}

impl EvcSource {
    pub fn open(name: &str, bytes: Arc<[u8]>) -> Result<Self> {
        Self::open_reader(name, Arc::new(cfa4_media::reader::MemReader(bytes)))
    }

    pub fn open_reader(name: &str, reader: cfa4_media::SharedReader) -> Result<Self> {
        let len = reader.len();
        let read = |off: u64, n: u64| -> Result<Vec<u8>> {
            let n = usize::try_from(n).map_err(|_| CodecError::Container("EVC: NAL unit too large".into()))?;
            cfa4_media::reader::read_range(&*reader, off, n).map_err(|e| CodecError::Container(e.to_string()))
        };
        let mut scan = cfa4_evc::Scanner::new();
        let mut params: Vec<Vec<u8>> = Vec::new();
        let mut si = None;
        // (start, end, sync, poc) per picture, plus the IDR period it belongs to
        let mut pics: Vec<(u64, u64, bool, i32, usize)> = Vec::new();
        let mut period = 0usize;
        let mut start = 0u64;
        let mut pos = 0u64;
        while pos.saturating_add(4) <= len {
            let head = read(pos, 4)?;
            let n = u64::from(u32::from_be_bytes(head.as_slice().try_into().map_err(|_| CodecError::Container("EVC: read".into()))?));
            let body = pos + 4;
            if n == 0 || n > MAX_NAL || body.saturating_add(n) > len {
                // a damaged or cut tail: index what came before
                break;
            }
            let nal = read(body, n.min(2))?;
            let Some((t, _)) = cfa4_evc::nal_type(&nal) else { break };
            let nal = if t < cfa4_evc::NUT_SPS { read(body, n.min(SLICE_HEAD))? } else { read(body, n)? };
            if t == cfa4_evc::NUT_SPS && si.is_none() {
                si = cfa4_evc::probe(&[&head[..], &nal[..]].concat());
            }
            if (cfa4_evc::NUT_SPS..=cfa4_evc::NUT_SPS + 1).contains(&t) && pics.is_empty() {
                params.push(nal.clone());
            }
            match scan.push(&nal) {
                Ok(Some(p)) => {
                    if p.idr && !pics.is_empty() {
                        period += 1;
                    }
                    pics.push((start, body + n, p.idr || p.intra, p.poc, period));
                    start = body + n;
                    if pics.len() > MAX_UNITS {
                        return Err(CodecError::Container("EVC: too many pictures".into()));
                    }
                }
                Ok(None) => {}
                // an undecodable header: the decoder will report it when the picture is asked for
                Err(e) => log::warn!("EVC: picture at byte {pos}: {e}"),
            }
            pos = body + n;
        }
        let si = si.ok_or_else(|| CodecError::Container("EVC: no sequence parameter set".into()))?;
        if pics.is_empty() {
            return Err(CodecError::Container("EVC: no pictures".into()));
        }
        // display order: by period, then by order count
        let mut order: Vec<usize> = (0..pics.len()).collect();
        order.sort_by_key(|&i| pics.get(i).map(|p| (p.4, p.3, i)));
        let mut units: Vec<Unit> = pics.iter().map(|&(a, b, sync, _, _)| Unit { off: a, size: b - a, sync, shown: 0 }).collect();
        for (frame, &i) in order.iter().enumerate() {
            if let Some(u) = units.get_mut(i) {
                u.shown = i64::try_from(frame).unwrap_or(i64::MAX);
            }
        }
        let rate = match si.timing {
            Some((units, scale)) if units > 0 && scale > 0 => FrameRate::new(i64::from(scale), i64::from(units)),
            _ => FrameRate::FPS_25,
        };
        let count = i64::try_from(units.len()).unwrap_or(i64::MAX);
        let duration = Tick::from_rational(count.saturating_mul(rate.den), 1, rate.num);
        let secs = duration.seconds();
        let bitrate = (secs > 0.0).then(|| (len.saturating_mul(8) as f64 / secs) as u64);
        let color = crate::stream_color::resolve(si.width, si.height, &[]);
        let info = MediaInfo {
            name: name.to_string(),
            kind: MediaKind::Movie,
            duration,
            video: Some(VideoStreamInfo {
                width: si.width,
                height: si.height,
                frame_rate: rate,
                par: si.sar.unwrap_or((1, 1)),
                codec: NAME.into(),
                pixel_format: pixel_format(&si),
                color,
                has_alpha: false,
                bitrate,
                hdr: None,
            }),
            audio_streams: Vec::new(),
            container: "EVC elementary stream".into(),
            start_timecode: None,
            file_size: Some(len),
        };
        Ok(Self { info, bytes: crate::Src(reader), units, by_frame: order, params, rate, video: GopCache::new(None) })
    }
}

struct EvcVideo<'a>(&'a EvcSource);

impl VideoSamples for EvcVideo<'_> {
    fn count(&self) -> usize {
        self.0.units.len()
    }
    fn pts(&self, i: usize) -> i64 {
        self.0.units.get(i).map_or(i64::MAX, |u| u.shown)
    }
    fn sync_before(&self, i: usize) -> usize {
        let i = i.min(self.0.units.len().saturating_sub(1));
        (0..=i).rev().find(|&k| self.0.units.get(k).is_some_and(|u| u.sync)).unwrap_or(0)
    }
    fn sample_at(&self, t: i64) -> Option<usize> {
        let last = self.0.by_frame.len().checked_sub(1)?;
        let f = usize::try_from(t.max(0)).unwrap_or(last).min(last);
        self.0.by_frame.get(f).copied()
    }
    fn read(&self, i: usize) -> Result<Vec<u8>> {
        let u = self.0.units.get(i).ok_or_else(|| CodecError::Container("sample index out of range".into()))?;
        let sz = usize::try_from(u.size).map_err(|_| CodecError::Container("EVC: picture too large".into()))?;
        cfa4_media::reader::read_range(&*self.0.bytes.0, u.off, sz).map_err(|e| CodecError::Container(e.to_string()))
    }
    fn make_decoder(&self) -> Result<Box<dyn VideoDecoder>> {
        Ok(Box::new(EvcDecoder::new(self.0.params.clone(), 4)))
    }
}

impl MediaSource for EvcSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video_frame(&self, req: FrameRequest) -> std::result::Result<Arc<VideoFrame>, MediaError> {
        let i = self.rate.frame_at(req.time.max(Tick::ZERO));
        let mut f = self.video.frame(&EvcVideo(self), i)?;
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
    Some(EvcSource::open(name, bytes).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

pub fn reader_opener(name: &str, head: &[u8], reader: &cfa4_media::SharedReader) -> Option<std::result::Result<SharedSource, MediaError>> {
    if !sniff(head) {
        return None;
    }
    Some(EvcSource::open_reader(name, reader.clone()).map(|s| Arc::new(s) as SharedSource).map_err(Into::into))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_streams_are_not_sniffed() {
        assert!(!sniff(b""));
        assert!(!sniff(&[0, 0, 0, 1, 0x00]));
        // an MP4 file type box
        assert!(!sniff(b"\0\0\0\x18ftypisom\0\0\x02\0isomiso2"));
    }

    #[test]
    fn evcc_records_give_their_nal_units() {
        let mut rec = vec![1, 0, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0x40, 0, 64, 0, 64, 3, 2];
        rec.extend_from_slice(&[0x80 | 24, 0, 1, 0, 3, 0x32, 0, 1]);
        rec.extend_from_slice(&[0x80 | 25, 0, 2, 0, 1, 0x34, 0, 2, 0x34, 0]);
        let (nals, ls) = parse_evcc(&rec).unwrap();
        assert_eq!(ls, 4);
        assert_eq!(nals, vec![vec![0x32, 0, 1], vec![0x34], vec![0x34, 0]]);
        assert!(parse_evcc(&rec[..rec.len() - 1]).is_none());
        let mut entry = vec![0u8; 78];
        entry.extend_from_slice(&((rec.len() + 8) as u32).to_be_bytes());
        entry.extend_from_slice(b"evcC");
        entry.extend_from_slice(&rec);
        assert_eq!(evcc_params(&entry).map(|p| p.0.len()), Some(3));
    }

    #[test]
    fn a_decoder_without_parameter_sets_refuses_pictures() {
        let mut d = EvcDecoder::new(Vec::new(), 4);
        assert!(d.decode(&[0, 0, 0, 3, 0x04, 0x00, 0x00], 0).is_err());
    }
}
