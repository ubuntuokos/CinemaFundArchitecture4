//! Our pure-Rust Cinepak decoder (`cfa4-cinepak`): `cvid` in AVI, Matroska (VfW FourCC) and QuickTime.

use cfa4_frame::VideoFrame;
use cfa4_isobmff::{CodecConfig, FourCc, SampleEntry};

use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// The sample entry format of a Cinepak stream; its `CodecConfig::Unknown` `raw` is the palette
/// (up to 256 `B, G, R, 0` entries) of an 8-bit stream, and `VideoParams::depth` the bit count.
pub const CVID: FourCc = FourCc(*b"cvid");

/// Whether `fourcc` is Cinepak's.
pub fn is_cinepak(fourcc: &[u8; 4]) -> bool {
    fourcc.eq_ignore_ascii_case(b"cvid")
}

/// A sample entry for a Cinepak stream of `w` x `h` with `BITMAPINFOHEADER` bit count `bit_count`
/// and the (possibly empty) palette `quads`.
pub fn sample_entry(bit_count: u16, quads: &[u8], w: u16, h: u16) -> SampleEntry {
    let mut e = SampleEntry::video(CVID, CodecConfig::Unknown { fourcc: CVID, raw: quads.to_vec() }, w, h);
    if let Some(v) = e.video.as_mut() {
        v.depth = bit_count;
    }
    e
}

pub struct CinepakDecoder {
    dec: cfa4_cinepak::Decoder,
    size: (u32, u32, u16),
}

impl CinepakDecoder {
    pub fn new(width: u32, height: u32, bit_count: u16, quads: &[u8]) -> Result<Self> {
        let mut dec = cfa4_cinepak::Decoder::new(width, height, bit_count).map_err(|e| CodecError::Unsupported(format!("Cinepak: {e}")))?;
        dec.set_palette_bgr0(quads);
        Ok(Self { dec, size: (width, height, bit_count) })
    }
}

impl VideoDecoder for CinepakDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let pic = self.dec.decode(sample).map_err(|e| CodecError::Decode(e.to_string()))?;
        let frame = VideoFrame::rgba8(pic.width, pic.height, pic.to_rgba());
        Ok(vec![DecodedFrame { pts, frame, draft: false }])
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        Vec::new()
    }
    fn reset(&mut self) {
        self.dec.reset();
    }
    fn name(&self) -> &str {
        "CFA4 Cinepak"
    }
    // AVI marks every sample as a sync sample: only a key frame can start a run
    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        let mut fresh = cfa4_cinepak::Decoder::new(self.size.0, self.size.1, self.size.2).ok()?;
        Some(fresh.decode(sample).is_ok_and(|p| p.key))
    }
    // inter frames: a frame depends on the ones before it
    fn intra_only(&self) -> bool {
        false
    }
}

pub fn cinepak_factory(e: &SampleEntry) -> Option<Result<Box<dyn VideoDecoder>>> {
    if !is_cinepak(&e.format.0) {
        return None;
    }
    let (w, h, depth) = e.video.as_ref().map_or((0, 0, 24), |v| (u32::from(v.width), u32::from(v.height), v.depth));
    let quads: &[u8] = match &e.codec {
        CodecConfig::Unknown { raw, .. } if depth == 8 => raw,
        _ => &[],
    };
    Some(CinepakDecoder::new(w, h, depth, quads).map(|d| Box::new(d) as Box<dyn VideoDecoder>))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourcc_and_empty_frame() {
        assert!(is_cinepak(b"cvid") && is_cinepak(b"CVID") && !is_cinepak(b"cvi2"));
        let mut d = cinepak_factory(&sample_entry(24, &[], 8, 4)).unwrap().unwrap();
        // a frame with no strips changes nothing: black
        let f = d.decode(&[0, 0, 0, 10, 0, 8, 0, 4, 0, 0], 0).unwrap();
        assert_eq!(f.len(), 1);
        assert!(cinepak_factory(&sample_entry(24, &[], 0, 4)).unwrap().is_err());
        assert!(d.decode(&[1, 2, 3], 1).is_err());
    }
}
