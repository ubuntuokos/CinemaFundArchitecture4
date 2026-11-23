//! Our pure-Rust Sorenson Video 3 decoder (`cfa4-svq3`) in QuickTime / MP4 sample entries
//! (`SVQ3`), AVI and Matroska (VfW FourCC `SVQ3`).
//!
//! The decoder reads its sequence header (`SEQH`: picture size, motion precision, B pictures,
//! watermark) from the extradata: the `SMI ` atom after the QuickTime image description, or what
//! follows the `BITMAPINFOHEADER` in AVI. It is handed the whole sample entry (QuickTime) or the
//! extradata (VfW), it looks for the marker itself.

use std::sync::Arc;

use cfa4_frame::{Chroma, PixelData, VideoFrame};
use cfa4_isobmff::{CodecConfig, FourCc, SampleEntry};
use cfa4_svq3::PicType;

use crate::stream_color::{ColorCodes, resolve};
use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// The sample entry format of an SVQ3 stream.
pub const SVQ3: FourCc = FourCc(*b"SVQ3");

/// Whether `fourcc` is SVQ3's.
pub fn is_svq3(fourcc: &[u8; 4]) -> bool {
    fourcc.eq_ignore_ascii_case(b"SVQ3")
}

/// A sample entry for an SVQ3 stream of `w` x `h` with the decoder's `extradata` (the bytes
/// holding the `SEQH` sequence header; may be empty).
pub fn sample_entry(w: u16, h: u16, extradata: &[u8]) -> SampleEntry {
    SampleEntry::video(SVQ3, CodecConfig::Unknown { fourcc: SVQ3, raw: extradata.to_vec() }, w, h)
}

fn to_frame(o: cfa4_svq3::Output) -> Result<DecodedFrame> {
    let p = o.picture;
    let mut it = p.planes.into_iter();
    let (Some(y), Some(u), Some(v)) = (it.next(), it.next(), it.next()) else {
        return Err(CodecError::Decode("SVQ3 picture without planes".into()));
    };
    // full-range YUV ("YUVJ420P"), BT.601
    let color = resolve(p.width, p.height, &[ColorCodes { primaries: 2, transfer: 2, matrix: 6, full_range: Some(true) }]);
    let data = PixelData::Yuv8 { planes: [Arc::new(y), Arc::new(u), Arc::new(v)], chroma: Chroma::C420, alpha: None };
    let frame = VideoFrame { width: p.width, height: p.height, data, color, par: (1, 1), pts: cfa4_time::Tick::ZERO, layout: Default::default() };
    Ok(DecodedFrame { pts: o.tag, frame, draft: false })
}

pub struct Svq3Decoder {
    dec: cfa4_svq3::Decoder,
}

impl VideoDecoder for Svq3Decoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        match self.dec.decode(sample, pts).map_err(|e| CodecError::Decode(e.to_string()))? {
            Some(o) => Ok(vec![to_frame(o)?]),
            None => Ok(Vec::new()),
        }
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        self.dec.flush().and_then(|o| to_frame(o).ok()).into_iter().collect()
    }
    fn reset(&mut self) {
        self.dec.reset();
    }
    fn name(&self) -> &str {
        "CFA4 SVQ3"
    }
    // AVI marks every sample as a sync sample: only an intra picture can start decoding
    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        self.dec.frame_type(sample).map(|t| t == PicType::I)
    }
    // a B picture no other picture refers to
    fn is_disposable(&self, sample: &[u8]) -> bool {
        self.dec.frame_type(sample) == Some(PicType::B)
    }
}

pub fn svq3_factory(e: &SampleEntry) -> Option<Result<Box<dyn VideoDecoder>>> {
    if !is_svq3(&e.format.0) {
        return None;
    }
    let (w, h) = e.video.as_ref().map_or((0, 0), |v| (u32::from(v.width), u32::from(v.height)));
    let extradata: &[u8] = match &e.codec {
        CodecConfig::Unknown { raw, .. } => raw,
        _ => &[],
    };
    Some(
        cfa4_svq3::Decoder::new(w, h, extradata)
            .map(|dec| Box::new(Svq3Decoder { dec }) as Box<dyn VideoDecoder>)
            .map_err(|e| CodecError::Decode(e.to_string())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourcc_and_short_frame() {
        assert!(is_svq3(b"SVQ3") && is_svq3(b"svq3") && !is_svq3(b"SVQ1"));
        let mut d = svq3_factory(&sample_entry(32, 32, &[])).unwrap().unwrap();
        assert!(d.decode(&[0; 10], 0).is_err());
        assert!(d.flush().is_empty());
        // a sequence header that does not parse is an error, not a panic
        assert!(svq3_factory(&sample_entry(32, 32, b"SMI \0\0\0\x01SEQH\0\0\0\x09\0")).unwrap().is_err());
    }
}
