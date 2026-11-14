//! Early AV2 decoding through an external helper process (ADR 0010, A track; plan X/6).
//!
//! Behind the `av2-external` cargo feature and the `CFA4_AV2_DECODER` environment variable
//! (the path of `cfa4-av2-decode`, built by `scripts/dev/av2-decoder.py`). Without either, AV2
//! stays `Unsupported` ([`crate::av2`]). This is AV2-LAB level: the router registration (plan
//! X/7) waits for G1.
//!
//! The helper (`packaging/av2-decode/cfa4-av2-decode.c`) wraps dav2d. It runs in its own
//! process, so a crash, hang or garbage output of the C decoder becomes a [`CodecError`] here,
//! never a crash of CFA4 (AGENTS.md § 0). Every reply is checked: message sizes are capped, a
//! frame header must describe exactly the bytes that follow, and each request has a time limit.
//!
//! The protocol and the process handling are shared with the VVC helper: see
//! [`crate::external_decoder`].

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use cfa4_av2::obu::{ObuType, annexb_obus};
use cfa4_frame::VideoFrame;

use crate::external_decoder::{ExternalDecoder, HelperKind};
pub use crate::external_decoder::{FRAME_HEADER_LEN, PROTOCOL_VERSION, RawPicture};
use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// The environment variable naming the helper executable.
pub const DECODER_ENV: &str = "CFA4_AV2_DECODER";
/// The AV2 helper (`cfa4-av2-decode`).
pub static HELPER: HelperKind = HelperKind { magic: *b"CFA4AV2D", label: "AV2", decoder: "dav2d", thread_name: "cfa4-av2-helper" };

/// The helper configured in [`DECODER_ENV`], if any.
pub fn configured_helper() -> Option<PathBuf> {
    std::env::var_os(DECODER_ENV).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// AV2 decoding through `cfa4-av2-decode`. Samples are temporal units (Annex B OBUs).
pub struct ExternalAv2Decoder(ExternalDecoder);

impl ExternalAv2Decoder {
    /// Start the helper at `path` (`threads` 0 = the decoder's choice).
    pub fn new(path: &Path, threads: usize) -> Result<ExternalAv2Decoder> {
        Ok(ExternalAv2Decoder(ExternalDecoder::new(path, threads, &HELPER)?))
    }

    /// Time the helper gets to answer one request (default one minute).
    pub fn set_request_timeout(&mut self, timeout: Duration) {
        self.0.set_request_timeout(timeout);
    }

    /// The helper configured in [`DECODER_ENV`].
    pub fn from_env() -> Result<ExternalAv2Decoder> {
        let path = configured_helper().ok_or_else(|| CodecError::Unsupported(format!("AV2: {DECODER_ENV} is not set")))?;
        Self::new(&path, 0)
    }
}

impl VideoDecoder for ExternalAv2Decoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        self.0.decode(sample, pts)
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        self.0.flush()
    }
    fn reset(&mut self) {
        self.0.reset();
    }
    fn name(&self) -> &str {
        self.0.name()
    }
    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        Some(is_closed_loop_key_unit(sample))
    }
}

/// Whether a temporal unit holds a closed-loop key frame (decoding can start there).
pub fn is_closed_loop_key_unit(tu: &[u8]) -> bool {
    annexb_obus(tu).map_while(|o| o.ok()).any(|o| o.header.obu_type == ObuType::ClosedLoopKey)
}

/// Split an Annex B stream into temporal units (each starts at a temporal delimiter):
/// `(offset, length)` pairs. Stops at the first malformed OBU.
pub fn temporal_units(data: &[u8]) -> Vec<(usize, usize)> {
    let mut units: Vec<(usize, usize)> = Vec::new();
    let mut end = 0usize;
    for obu in annexb_obus(data) {
        let Ok(obu) = obu else { break };
        let obu_end = obu.payload.as_ptr() as usize - data.as_ptr() as usize + obu.payload.len();
        if obu.header.obu_type == ObuType::TemporalDelimiter || units.is_empty() {
            if let Some(last) = units.last_mut() {
                last.1 = obu.offset - last.0;
            }
            units.push((obu.offset, 0));
        }
        end = obu_end;
    }
    if let Some(last) = units.last_mut() {
        last.1 = end - last.0;
    }
    units
}

/// One temporal unit of an Annex B file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Unit {
    offset: u64,
    len: usize,
    key: bool,
}

/// Longest temporal unit read from a file (the helper refuses more than 256 MiB too).
const MAX_UNIT: usize = 256 * 1024 * 1024;

/// Index the temporal units of an Annex B file by reading only the OBU headers.
fn index_units(reader: &dyn cfa4_media::reader::ByteReader) -> Vec<Unit> {
    let len = reader.len();
    let mut units: Vec<Unit> = Vec::new();
    let mut pos = 0u64;
    let mut head = [0u8; 10]; // the longest leb128 (8) and OBU header (2)
    while pos < len {
        let n = usize::try_from((len - pos).min(head.len() as u64)).unwrap_or(0);
        let Some(buf) = head.get_mut(..n) else { break };
        if reader.read_at(pos, buf).is_err() {
            break;
        }
        let Ok((size, leb)) = cfa4_av2::obu::leb128(buf) else { break };
        let Ok((header, _)) = cfa4_av2::obu::ObuHeader::parse(buf.get(leb..).unwrap_or(&[])) else { break };
        let obu_len = leb as u64 + u64::from(size);
        if size == 0 || pos + obu_len > len {
            break;
        }
        let starts_unit = header.obu_type == ObuType::TemporalDelimiter || units.is_empty();
        if starts_unit {
            units.push(Unit { offset: pos, len: 0, key: false });
        }
        let Some(u) = units.last_mut() else { break };
        u.len = usize::try_from(pos + obu_len - u.offset).unwrap_or(usize::MAX);
        u.key |= header.obu_type == ObuType::ClosedLoopKey;
        if u.len > MAX_UNIT {
            units.pop();
            break;
        }
        pos += obu_len;
    }
    units
}

/// An AV2 Annex B file (`.obu`) decoded through the helper. Frame rate: Annex B carries no timing,
/// so 24 fps like the other raw elementary streams.
pub struct Av2AnnexBSource {
    info: cfa4_media::MediaInfo,
    bytes: crate::Src,
    units: Vec<Unit>,
    helper: PathBuf,
    rate: cfa4_time::FrameRate,
    video: crate::gop::GopCache,
}

impl Av2AnnexBSource {
    pub fn open(name: &str, reader: cfa4_media::SharedReader, helper: PathBuf) -> Result<Self> {
        use cfa4_media::{MediaInfo, MediaKind, VideoStreamInfo};
        let units = index_units(&*reader);
        let first = units.first().ok_or_else(|| CodecError::Container("AV2: no temporal units".into()))?;
        let bytes = cfa4_media::reader::read_range(&*reader, first.offset, first.len).map_err(|e| CodecError::Container(e.to_string()))?;
        let stream = cfa4_av2::probe_annexb(&bytes).map_err(|e| CodecError::Unsupported(format!("AV2: {e}")))?;
        let sh = stream.sequence_header;
        let (width, height) = sh.cropped_size().ok_or_else(|| CodecError::Container("AV2: cropping window larger than the frame".into()))?;
        let chroma = match sh.chroma_format {
            cfa4_av2::seq::ChromaFormat::Monochrome => "Monochrome",
            cfa4_av2::seq::ChromaFormat::Yuv420 => "YUV 4:2:0",
            cfa4_av2::seq::ChromaFormat::Yuv422 => "YUV 4:2:2",
            cfa4_av2::seq::ChromaFormat::Yuv444 => "YUV 4:4:4",
        };
        let rate = cfa4_time::FrameRate::FPS_24;
        let duration = cfa4_time::Tick::from_rational(units.len() as i64 * rate.den, 1, rate.num);
        let color = crate::hw_frame::av1_color(width, height, 2, 2, 2, false);
        let info = MediaInfo {
            name: name.to_string(),
            kind: MediaKind::Movie,
            duration,
            video: Some(VideoStreamInfo {
                width,
                height,
                frame_rate: rate,
                par: (1, 1),
                codec: format!("AV2 {} (external decoder)", sh.profile.label()),
                pixel_format: format!("{chroma} {}-bit", sh.bit_depth),
                color,
                has_alpha: false,
                bitrate: None,
                hdr: None,
            }),
            audio_streams: Vec::new(),
            container: "AV2 Annex B".into(),
            start_timecode: None,
            file_size: Some(reader.len()),
        };
        Ok(Av2AnnexBSource { info, bytes: crate::Src(reader), units, helper, rate, video: crate::gop::GopCache::new(None) })
    }
}

struct Av2Video<'a>(&'a Av2AnnexBSource);

impl crate::gop::VideoSamples for Av2Video<'_> {
    fn count(&self) -> usize {
        self.0.units.len()
    }
    fn pts(&self, i: usize) -> i64 {
        i as i64
    }
    fn sync_before(&self, i: usize) -> usize {
        let i = i.min(self.0.units.len().saturating_sub(1));
        self.0.units.get(..=i).and_then(|u| u.iter().rposition(|u| u.key)).unwrap_or(0)
    }
    fn sample_at(&self, t: i64) -> Option<usize> {
        let last = self.0.units.len().checked_sub(1)?;
        Some(usize::try_from(t.max(0)).unwrap_or(usize::MAX).min(last))
    }
    fn read(&self, i: usize) -> Result<Vec<u8>> {
        let u = self.0.units.get(i).ok_or_else(|| CodecError::Container("sample index out of range".into()))?;
        cfa4_media::reader::read_range(&*self.0.bytes.0, u.offset, u.len).map_err(|e| CodecError::Container(e.to_string()))
    }
    fn make_decoder(&self) -> Result<Box<dyn VideoDecoder>> {
        Ok(Box::new(ExternalAv2Decoder::new(&self.0.helper, 0)?))
    }
}

impl cfa4_media::MediaSource for Av2AnnexBSource {
    fn info(&self) -> &cfa4_media::MediaInfo {
        &self.info
    }

    fn video_frame(&self, req: cfa4_media::FrameRequest) -> std::result::Result<Arc<VideoFrame>, cfa4_media::MediaError> {
        let frame_idx = self.rate.frame_at(req.time.max(cfa4_time::Tick::ZERO));
        Ok(self.video.frame(&Av2Video(self), frame_idx)?)
    }

    fn audio(&self, _start: i64, _frames: usize, _sample_rate: u32) -> std::result::Result<cfa4_frame::AudioBuffer, cfa4_media::MediaError> {
        Err(cfa4_media::MediaError::NoStream("audio"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporal_units_start_at_delimiters() {
        // TD, seq header (2 bytes), tile group | TD, tile group
        let s = [0x01, 0x08, 0x03, 0x04, 0xaa, 0xbb, 0x02, 0x1c, 0x01, 0x01, 0x08, 0x02, 0x10, 0x00];
        assert_eq!(temporal_units(&s), vec![(0, 9), (9, 5)]);
        // closed-loop key (type 4) in the second unit
        assert!(!is_closed_loop_key_unit(&s[..9]));
        assert!(is_closed_loop_key_unit(&s[9..]));
        assert!(temporal_units(&[]).is_empty());
        // a broken tail ends the last unit at the last good OBU
        assert_eq!(temporal_units(&s[..13]), vec![(0, 9), (9, 2)]);
    }

    #[test]
    fn a_missing_or_wrong_helper_is_an_error() {
        assert!(ExternalAv2Decoder::new(Path::new("/nonexistent/cfa4-av2-decode"), 0).is_err());
        // a program that is not the helper: no hello
        assert!(ExternalAv2Decoder::new(Path::new("/bin/true"), 0).is_err());
    }
}
