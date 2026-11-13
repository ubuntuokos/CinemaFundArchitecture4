//! Our pure-Rust Canopus decoder (`cfa4-canopus`): AVI and Matroska (VfW FourCC) `CUVC` (HQ and
//! HQA) and `CHQX` (HQX).

use std::sync::Arc;

use cfa4_frame::{Chroma, PixelData, VideoFrame};
use cfa4_isobmff::{CodecConfig, FourCc, SampleEntry};

use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// The codec of a Canopus FourCC (`CUVC`, `CHQX`, any case).
pub fn codec(fourcc: &[u8; 4]) -> Option<cfa4_canopus::Codec> {
    cfa4_canopus::Codec::from_fourcc(fourcc)
}

/// The label of a Canopus FourCC.
pub fn label(codec: cfa4_canopus::Codec) -> &'static str {
    match codec {
        cfa4_canopus::Codec::Hq => "Canopus HQ",
        cfa4_canopus::Codec::Hqx => "Canopus HQX",
    }
}

/// A sample entry for a Canopus stream of `w` x `h`.
pub fn sample_entry(codec: cfa4_canopus::Codec, w: u16, h: u16) -> SampleEntry {
    let fourcc = FourCc(match codec {
        cfa4_canopus::Codec::Hq => *b"CUVC",
        cfa4_canopus::Codec::Hqx => *b"CHQX",
    });
    SampleEntry::video(fourcc, CodecConfig::Unknown { fourcc, raw: Vec::new() }, w, h)
}

pub struct CanopusDecoder {
    dec: cfa4_canopus::Decoder,
}

/// Convert a decoded picture to a frame.
pub(crate) fn convert(p: cfa4_canopus::Picture) -> Result<VideoFrame> {
    let chroma = if p.layout.is_444() { Chroma::C444 } else { Chroma::C422 };
    // Canopus signals no matrix or range: the size-based default, studio swing
    let color = crate::stream_color::resolve(p.width, p.height, &[]);
    let data = match p.planes {
        cfa4_canopus::Planes::U8(planes) => {
            let mut it = planes.into_iter().map(Arc::new);
            let mut next = || it.next().ok_or_else(|| CodecError::Decode("Canopus picture without planes".into()));
            let (y, u, v) = (next()?, next()?, next()?);
            PixelData::Yuv8 { planes: [y, u, v], chroma, alpha: it.next() }
        }
        cfa4_canopus::Planes::U16(planes) => {
            let mut it = planes.into_iter().map(Arc::new);
            let mut next = || it.next().ok_or_else(|| CodecError::Decode("Canopus picture without planes".into()));
            let (y, u, v) = (next()?, next()?, next()?);
            PixelData::Yuv16 { planes: [y, u, v], chroma, bits: 12, alpha: it.next() }
        }
    };
    Ok(VideoFrame { width: p.width, height: p.height, data, color, par: (1, 1), pts: cfa4_time::Tick::ZERO, layout: Default::default() })
}

impl VideoDecoder for CanopusDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let p = self.dec.decode(sample).map_err(|e| CodecError::Decode(format!("{}: {e}", label(self.dec.codec()))))?;
        Ok(vec![DecodedFrame { pts, frame: convert(p)?, draft: false }])
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        Vec::new()
    }
    fn reset(&mut self) {}
    fn name(&self) -> &str {
        "CFA4 Canopus"
    }
    fn intra_only(&self) -> bool {
        true
    }
}

pub fn canopus_factory(e: &SampleEntry) -> Option<Result<Box<dyn VideoDecoder>>> {
    let c = codec(&e.format.0)?;
    Some(Ok(Box::new(CanopusDecoder { dec: cfa4_canopus::Decoder::new(c) })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_claims_canopus_only() {
        assert!(canopus_factory(&sample_entry(cfa4_canopus::Codec::Hq, 64, 32)).is_some_and(|d| d.is_ok()));
        assert!(canopus_factory(&sample_entry(cfa4_canopus::Codec::Hqx, 64, 32)).is_some_and(|d| d.is_ok()));
        assert!(canopus_factory(&crate::hap::sample_entry(*b"Hap1", 64, 32)).is_none());
    }

    #[test]
    fn garbage_is_an_error() {
        let mut d = CanopusDecoder { dec: cfa4_canopus::Decoder::new(cfa4_canopus::Codec::Hqx) };
        assert!(d.decode(&[0u8; 100], 0).is_err());
        let p = cfa4_canopus::Picture { width: 2, height: 2, layout: cfa4_canopus::Layout::Yuv422, planes: cfa4_canopus::Planes::U8(vec![]) };
        assert!(convert(p).is_err());
    }
}
