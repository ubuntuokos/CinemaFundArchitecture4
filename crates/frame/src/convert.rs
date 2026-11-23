//! Pixel-format conversion matrix of the Frame ABI v0 (CFA4-RENDER-INTEGRATION-PLAN-001 phase C0,
//! `docs/audit/render-c0/C0-contracts-spec.md` §4).
//!
//! One table says which (source format, target format) pairs this crate converts and how; every
//! other pair is an **explicit error**, never a silent fallback. The supported targets are the two
//! the engine already produces from any source: the compositor's premultiplied linear f32
//! ([`VideoFrame::to_linear_f32`]) and straight-alpha display sRGB RGBA8 ([`VideoFrame::to_rgba8`]).
//! Conversions *into* planar Y'CbCr or R'G'B' integer formats are the encoders' business (in
//! `cfa4-export`) and are listed as unsupported here until a shared converter exists.
//!
//! The matrix is CPU only. GPU residency is described in [`crate::side::Residency`]; a GPU
//! conversion row is added with the `crates/gpu` binding.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{PixelData, VideoFrame};

/// The pixel layout of a [`VideoFrame`], without the data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PixelFormat {
    Rgba8,
    RgbaF32,
    Yuv8,
    Yuv16,
    Rgb16,
}

impl PixelFormat {
    pub const ALL: [PixelFormat; 5] = [PixelFormat::Rgba8, PixelFormat::RgbaF32, PixelFormat::Yuv8, PixelFormat::Yuv16, PixelFormat::Rgb16];

    pub fn of(data: &PixelData) -> PixelFormat {
        match data {
            PixelData::Rgba8(_) => PixelFormat::Rgba8,
            PixelData::RgbaF32(_) => PixelFormat::RgbaF32,
            PixelData::Yuv8 { .. } => PixelFormat::Yuv8,
            PixelData::Yuv16 { .. } => PixelFormat::Yuv16,
            PixelData::Rgb16 { .. } => PixelFormat::Rgb16,
        }
    }
}

/// What a supported conversion costs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Cost {
    /// Same format: the data is shared, nothing is computed.
    Share,
    /// Every pixel is recomputed.
    Convert,
}

/// A supported conversion: how it is done and whether it can lose information.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversion {
    pub from: PixelFormat,
    pub to: PixelFormat,
    pub cost: Cost,
    /// The target cannot represent every value of the source (bit depth, alpha premultiplication
    /// or the range of the colour encoding): converting back does not give the original.
    pub lossy: bool,
}

/// A pair this crate does not convert.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Unsupported {
    pub from: PixelFormat,
    pub to: PixelFormat,
}

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "no conversion from {:?} to {:?} in cfa4-frame (planar and integer targets are produced by the encoders)", self.from, self.to)
    }
}

impl std::error::Error for Unsupported {}

/// The conversion from `from` to `to`, or the explicit error.
pub fn plan(from: PixelFormat, to: PixelFormat) -> Result<Conversion, Unsupported> {
    use PixelFormat::*;
    if from == to {
        return Ok(Conversion { from, to, cost: Cost::Share, lossy: false });
    }
    match to {
        // The compositor's working format holds every source value.
        RgbaF32 => Ok(Conversion { from, to, cost: Cost::Convert, lossy: false }),
        // Display RGBA8: 8 bits, straight alpha. Lossless only from the 8-bit display source.
        Rgba8 => Ok(Conversion { from, to, cost: Cost::Convert, lossy: true }),
        Yuv8 | Yuv16 | Rgb16 => Err(Unsupported { from, to }),
    }
}

/// Convert `frame` to `to`. Presentation time, pixel aspect and layout carry over; the colour
/// description of the result is that of the new constructors (`ColorInfo::SRGB_FULL`), which is
/// what `to_rgba8` and `to_linear_f32` produce.
pub fn convert(frame: &VideoFrame, to: PixelFormat) -> Result<VideoFrame, Unsupported> {
    let c = plan(PixelFormat::of(&frame.data), to)?;
    if c.cost == Cost::Share {
        return Ok(frame.clone());
    }
    let mut out = match to {
        PixelFormat::Rgba8 => VideoFrame::rgba8(frame.width, frame.height, frame.to_rgba8()),
        PixelFormat::RgbaF32 => VideoFrame::rgba_f32(frame.width, frame.height, frame.to_linear_f32()),
        PixelFormat::Yuv8 | PixelFormat::Yuv16 | PixelFormat::Rgb16 => return Err(Unsupported { from: c.from, to }),
    };
    out.par = frame.par;
    out.pts = frame.pts;
    out.layout = frame.layout;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Chroma;
    use cfa4_color::ColorInfo;
    use std::sync::Arc;

    fn frame_of(fmt: PixelFormat) -> VideoFrame {
        let (w, h) = (4u32, 4u32);
        let n = (w * h) as usize;
        let data = match fmt {
            PixelFormat::Rgba8 => PixelData::Rgba8(Arc::new(vec![128; n * 4])),
            PixelFormat::RgbaF32 => PixelData::RgbaF32(Arc::new(vec![0.25; n * 4])),
            PixelFormat::Yuv8 => {
                PixelData::Yuv8 { planes: [Arc::new(vec![126; n]), Arc::new(vec![128; n]), Arc::new(vec![128; n])], chroma: Chroma::C444, alpha: None }
            }
            PixelFormat::Yuv16 => PixelData::Yuv16 {
                planes: [Arc::new(vec![504; n]), Arc::new(vec![512; n]), Arc::new(vec![512; n])],
                chroma: Chroma::C444,
                bits: 10,
                alpha: None,
            },
            PixelFormat::Rgb16 => {
                PixelData::Rgb16 { planes: [Arc::new(vec![30000; n]), Arc::new(vec![30000; n]), Arc::new(vec![30000; n])], bits: 16, alpha: None }
            }
        };
        VideoFrame { width: w, height: h, data, color: ColorInfo::SRGB_FULL, par: (1, 1), pts: crate::Tick(7), layout: Default::default() }
    }

    /// The matrix, pinned: every pair of the five formats, so a change is a visible diff.
    #[test]
    fn the_matrix_is_the_documented_one() {
        use PixelFormat::*;
        for from in PixelFormat::ALL {
            for to in PixelFormat::ALL {
                let r = plan(from, to);
                match (from == to, to) {
                    (true, _) => assert_eq!(r, Ok(Conversion { from, to, cost: Cost::Share, lossy: false }), "{from:?}->{to:?}"),
                    (false, RgbaF32) => assert_eq!(r, Ok(Conversion { from, to, cost: Cost::Convert, lossy: false }), "{from:?}->{to:?}"),
                    (false, Rgba8) => assert_eq!(r, Ok(Conversion { from, to, cost: Cost::Convert, lossy: true }), "{from:?}->{to:?}"),
                    (false, Yuv8 | Yuv16 | Rgb16) => assert_eq!(r, Err(Unsupported { from, to }), "{from:?}->{to:?}"),
                }
            }
        }
    }

    /// Every pair the matrix accepts really converts, and every other pair is an error: the
    /// table cannot drift from what `convert` does.
    #[test]
    fn the_matrix_and_the_converter_agree_for_every_pair() {
        for from in PixelFormat::ALL {
            let f = frame_of(from);
            assert_eq!(PixelFormat::of(&f.data), from);
            for to in PixelFormat::ALL {
                match (plan(from, to), convert(&f, to)) {
                    (Ok(_), Ok(out)) => {
                        assert_eq!(PixelFormat::of(&out.data), to, "{from:?}->{to:?}");
                        assert_eq!((out.width, out.height, out.pts, out.par), (f.width, f.height, f.pts, f.par));
                    }
                    (Err(a), Err(b)) => assert_eq!(a, b),
                    (p, c) => panic!("{from:?}->{to:?}: plan {p:?} but convert {:?}", c.map(|_| ())),
                }
            }
        }
    }

    #[test]
    fn an_unsupported_pair_is_an_explicit_named_error() {
        let e = convert(&frame_of(PixelFormat::Rgba8), PixelFormat::Yuv8).unwrap_err();
        assert_eq!(e, Unsupported { from: PixelFormat::Rgba8, to: PixelFormat::Yuv8 });
        assert!(e.to_string().contains("Rgba8") && e.to_string().contains("Yuv8"));
    }

    #[test]
    fn the_same_format_shares_the_data() {
        let f = frame_of(PixelFormat::Rgba8);
        let g = convert(&f, PixelFormat::Rgba8).unwrap();
        match (&f.data, &g.data) {
            (PixelData::Rgba8(a), PixelData::Rgba8(b)) => assert!(Arc::ptr_eq(a, b)),
            _ => panic!("not rgba8"),
        }
    }

    #[test]
    fn an_eight_bit_display_frame_survives_a_round_trip_through_float() {
        let mut px = Vec::new();
        for i in 0..16u8 {
            px.extend_from_slice(&[i * 16, 255 - i * 16, 100, 255]);
        }
        let f = VideoFrame::rgba8(4, 4, px.clone());
        let back = convert(&convert(&f, PixelFormat::RgbaF32).unwrap(), PixelFormat::Rgba8).unwrap();
        match back.data {
            PixelData::Rgba8(d) => {
                let worst = d.iter().zip(&px).map(|(a, b)| (i16::from(*a) - i16::from(*b)).abs()).max().unwrap_or(0);
                assert!(worst <= 1, "round trip moved a value by {worst}");
            }
            _ => panic!("not rgba8"),
        }
    }
}
