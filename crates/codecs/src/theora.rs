//! Our pure-Rust Theora decoder (`cfa4-theora`): Matroska `V_THEORA` (CodecPrivate: the Xiph-laced
//! identification, comment and setup headers).

use std::sync::Arc;

use cfa4_frame::{Chroma, PixelData, VideoFrame};
use cfa4_isobmff::{CodecConfig, FourCc, SampleEntry};

use crate::stream_color::ColorCodes;
use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// The sample entry format of a Theora stream; its `CodecConfig::Unknown` `raw` is the
/// Xiph-laced header block of the Matroska `CodecPrivate`.
pub const THEO: FourCc = FourCc(*b"theo");

/// A sample entry for a Theora stream of `w` x `h` whose Xiph-laced headers are `private`.
pub fn sample_entry(private: &[u8], w: u16, h: u16) -> SampleEntry {
    SampleEntry::video(THEO, CodecConfig::Unknown { fourcc: THEO, raw: private.to_vec() }, w, h)
}

/// The three headers of a Xiph-laced block (one count byte of 2, two lace sizes, the data).
fn split_xiph(p: &[u8]) -> Option<[&[u8]; 3]> {
    if *p.first()? != 2 {
        return None;
    }
    let mut pos = 1;
    let mut size = || -> Option<usize> {
        let mut s = 0usize;
        loop {
            let b = *p.get(pos)?;
            pos += 1;
            s = s.checked_add(usize::from(b))?;
            if b != 255 {
                return Some(s);
            }
        }
    };
    let a = size()?;
    let b = size()?;
    let start = pos;
    let first = p.get(start..start.checked_add(a)?)?;
    let second = p.get(start + a..(start + a).checked_add(b)?)?;
    let third = p.get(start + a + b..)?;
    Some([first, second, third])
}

pub struct TheoraDecoder {
    dec: cfa4_theora::Decoder,
    color: ColorCodes,
    par: (u32, u32),
    /// A key packet was decoded since the start or the last reset.
    started: bool,
}

impl TheoraDecoder {
    pub fn new(private: &[u8]) -> Result<Self> {
        let [id, comment, setup] = split_xiph(private).ok_or_else(|| CodecError::Unsupported("Theora: unreadable CodecPrivate".into()))?;
        let dec = cfa4_theora::Decoder::new(id, comment, setup).map_err(|e| CodecError::Unsupported(format!("Theora: {e}")))?;
        let i = dec.info();
        // identification header §6.2: 1 Rec. 470M (NTSC), 2 Rec. 470BG (PAL), studio range
        let (primaries, matrix) = match i.colorspace {
            1 => (6, 6),
            2 => (5, 5),
            _ => (2, 2),
        };
        let color = ColorCodes { primaries, transfer: if primaries == 2 { 2 } else { 6 }, matrix, full_range: Some(false) };
        let par = if i.aspect.0 == 0 || i.aspect.1 == 0 { (1, 1) } else { i.aspect };
        Ok(Self { dec, color, par, started: false })
    }

    fn convert(&self, p: cfa4_theora::Picture) -> VideoFrame {
        let chroma = match p.chroma {
            cfa4_theora::Chroma::C420 => Chroma::C420,
            cfa4_theora::Chroma::C422 => Chroma::C422,
            cfa4_theora::Chroma::C444 => Chroma::C444,
        };
        let [y, u, v] = p.planes;
        let color = crate::stream_color::resolve(p.width, p.height, &[self.color]);
        VideoFrame {
            width: p.width,
            height: p.height,
            data: PixelData::Yuv8 { planes: [Arc::new(y), Arc::new(u), Arc::new(v)], chroma, alpha: None },
            color,
            par: self.par,
            pts: cfa4_time::Tick::ZERO,
            layout: Default::default(),
        }
    }
}

impl VideoDecoder for TheoraDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        if !self.started {
            if !cfa4_theora::Decoder::is_key(sample) || sample.is_empty() {
                // after a seek: nothing to predict from until a key frame
                return Ok(Vec::new());
            }
            self.started = true;
        }
        let p = self.dec.decode(sample).map_err(|e| CodecError::Decode(format!("Theora: {e}")))?;
        Ok(vec![DecodedFrame { pts, frame: self.convert(p), draft: false }])
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        Vec::new()
    }
    fn reset(&mut self) {
        self.dec.reset();
        self.started = false;
    }
    fn name(&self) -> &str {
        "CFA4 Theora"
    }
    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        Some(cfa4_theora::Decoder::is_key(sample) && !sample.is_empty())
    }
}

pub fn theora_factory(e: &SampleEntry) -> Option<Result<Box<dyn VideoDecoder>>> {
    if e.format != THEO {
        return None;
    }
    let CodecConfig::Unknown { raw, .. } = &e.codec else {
        return Some(Err(CodecError::Unsupported("Theora: no headers".into())));
    };
    Some(TheoraDecoder::new(raw.as_slice()).map(|d| Box::new(d) as Box<dyn VideoDecoder>))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xiph_lacing() {
        let mut p = vec![2, 3, 2];
        p.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(split_xiph(&p), Some([&[1u8, 2, 3][..], &[4, 5][..], &[6, 7][..]]));
        assert!(split_xiph(&[2, 9, 9, 1]).is_none());
        assert!(split_xiph(&[]).is_none());
        assert!(split_xiph(&[1, 0]).is_none());
        assert!(split_xiph(&[2, 255]).is_none());
    }

    #[test]
    fn factory_claims_theora_only() {
        assert!(theora_factory(&crate::dirac::sample_entry(64, 32)).is_none());
        assert!(theora_factory(&sample_entry(&[], 64, 32)).is_some_and(|d| d.is_err()));
    }
}
