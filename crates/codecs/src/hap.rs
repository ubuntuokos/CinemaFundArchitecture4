//! Our pure-Rust Hap decoder (`cfa4-hap`): QuickTime / MP4 and AVI `Hap1`, `Hap5`, `HapY`,
//! `HapM` and `HapA`.

use cfa4_frame::VideoFrame;
use cfa4_isobmff::{CodecConfig, FourCc, SampleEntry};

use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

pub use cfa4_hap::is_hap;

/// A sample entry for a Hap stream of `w` x `h` (for containers other than QuickTime).
pub fn sample_entry(fourcc: [u8; 4], w: u16, h: u16) -> SampleEntry {
    SampleEntry::video(FourCc(fourcc), CodecConfig::Unknown { fourcc: FourCc(fourcc), raw: Vec::new() }, w, h)
}

pub struct HapDecoder {
    dec: cfa4_hap::Decoder,
}

impl HapDecoder {
    pub fn new(fourcc: [u8; 4], width: u32, height: u32) -> Result<Self> {
        let dec = cfa4_hap::Decoder::new(fourcc, width, height).map_err(|e| CodecError::Unsupported(format!("Hap: {e}")))?;
        Ok(Self { dec })
    }
}

impl VideoDecoder for HapDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let p = self.dec.decode(sample).map_err(|e| CodecError::Decode(format!("Hap: {e}")))?;
        Ok(vec![DecodedFrame { pts, frame: VideoFrame::rgba8(p.width, p.height, p.rgba), draft: false }])
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        Vec::new()
    }
    fn reset(&mut self) {}
    fn name(&self) -> &str {
        "CFA4 Hap"
    }
    fn intra_only(&self) -> bool {
        true
    }
}

pub fn hap_factory(e: &SampleEntry) -> Option<Result<Box<dyn VideoDecoder>>> {
    if !is_hap(&e.format.0) {
        return None;
    }
    let (w, h) = e.video.as_ref().map_or((0, 0), |v| (u32::from(v.width), u32::from(v.height)));
    Some(HapDecoder::new(e.format.0, w, h).map(|d| Box::new(d) as Box<dyn VideoDecoder>))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_claims_hap_only() {
        assert!(hap_factory(&sample_entry(*b"Hap1", 64, 32)).is_some_and(|d| d.is_ok()));
        assert!(matches!(hap_factory(&sample_entry(*b"Hap7", 64, 32)), Some(Err(CodecError::Unsupported(_)))));
        assert!(hap_factory(&sample_entry(*b"avc1", 64, 32)).is_none());
    }
}
