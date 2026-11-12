// Ported from the SMPTE ST 2042-1 pseudocode of the BBC VC-2 conformance software
// (vc2_conformance 2.0.0, Copyright British Broadcasting Corporation, GPL-3.0-only); see
// docs/licensing.md §5.

//! The sequence header (ST 2042-1 §11): parse parameters, the base video format with its
//! overrides (`source_parameters`), the picture coding mode, and the picture dimensions and
//! depths derived from them (§11.6).

use crate::bits::Bits;
use crate::tables::{BASE_FORMATS, COLOR_SPECS, FRAME_RATES, PIXEL_ASPECT_RATIOS, SIGNAL_RANGES};
use crate::{Error, Result};

/// Largest frame dimension accepted.
const MAX_DIM: u32 = 16384;

/// Colour difference sampling (Table 11.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChromaFormat {
    C444,
    C422,
    C420,
}

/// The video parameters (§11.4) of a sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoParameters {
    pub frame_width: u32,
    pub frame_height: u32,
    pub chroma: ChromaFormat,
    /// The source is interlaced (`source_sampling` 1).
    pub interlaced: bool,
    pub top_field_first: bool,
    /// Frame rate numerator and denominator.
    pub frame_rate: (u64, u64),
    pub pixel_aspect_ratio: (u64, u64),
    /// Clean width, height, left and top offset.
    pub clean_area: (u64, u64, u64, u64),
    pub luma_offset: u64,
    pub luma_excursion: u64,
    pub color_diff_offset: u64,
    pub color_diff_excursion: u64,
    /// Index of Table 11.7 (0 BT.709, 1 BT.601 525, 2 BT.601 625, 3 SMPTE ST 428-1, 4 BT.2020).
    pub color_primaries: u64,
    /// Index of Table 11.8 (0 BT.709, 1 BT.601, 2 YCgCo, 3 identity, 4 BT.2020).
    pub color_matrix: u64,
    /// Index of Table 11.9 (0 TV gamma, 1 extended gamut, 2 linear, 3 D-Cinema, 4 PQ, 5 HLG).
    pub transfer_function: u64,
}

/// A parsed sequence header with the coding parameters derived from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sequence {
    pub major_version: u64,
    pub profile: u64,
    pub level: u64,
    pub video: VideoParameters,
    /// Pictures are fields (`picture_coding_mode` 1), two per frame.
    pub fields: bool,
    pub luma_width: u32,
    pub luma_height: u32,
    pub color_diff_width: u32,
    pub color_diff_height: u32,
    pub luma_depth: u32,
    pub color_diff_depth: u32,
}

fn preset<T: Copy>(table: &[T], index: u64) -> Result<T> {
    usize::try_from(index).ok().filter(|&i| i > 0).and_then(|i| table.get(i)).copied().ok_or(Error::Invalid("unknown preset index"))
}

fn dim(v: u64) -> Result<u32> {
    u32::try_from(v).ok().filter(|&d| (1..=MAX_DIM).contains(&d)).ok_or(Error::Unsupported("frame size"))
}

/// `intlog2(excursion + 1)`: the number of bits of the excursion.
fn depth(excursion: u64) -> Result<u32> {
    let d = 64 - excursion.leading_zeros();
    if (1..=16).contains(&d) { Ok(d) } else { Err(Error::Unsupported("sample depth above 16 bits")) }
}

/// Parse a sequence header from the bytes after its parse info header.
pub(crate) fn parse(data: &[u8]) -> Result<Sequence> {
    let mut b = Bits::new(data);
    let major_version = b.read_uint()?;
    let _minor_version = b.read_uint()?;
    let profile = b.read_uint()?;
    let level = b.read_uint()?;

    let base = usize::try_from(b.read_uint()?).ok().and_then(|i| BASE_FORMATS.get(i)).ok_or(Error::Invalid("unknown base video format"))?;
    let rate = preset(&FRAME_RATES, u64::from(base.rate))?;
    let range = preset(&SIGNAL_RANGES, u64::from(base.range))?;
    let spec = COLOR_SPECS.get(usize::from(base.spec)).copied().ok_or(Error::Invalid("unknown colour spec"))?;
    let mut v = VideoParameters {
        frame_width: base.w,
        frame_height: base.h,
        chroma: ChromaFormat::C444,
        interlaced: base.interlaced,
        top_field_first: base.tff,
        frame_rate: rate,
        pixel_aspect_ratio: preset(&PIXEL_ASPECT_RATIOS, u64::from(base.par))?,
        clean_area: (u64::from(base.clean[0]), u64::from(base.clean[1]), u64::from(base.clean[2]), u64::from(base.clean[3])),
        luma_offset: range[0],
        luma_excursion: range[1],
        color_diff_offset: range[2],
        color_diff_excursion: range[3],
        color_primaries: u64::from(spec.0),
        color_matrix: u64::from(spec.1),
        transfer_function: u64::from(spec.2),
    };
    let mut cdf = u64::from(base.cdf);

    // frame_size
    if b.read_bool()? {
        v.frame_width = dim(b.read_uint()?)?;
        v.frame_height = dim(b.read_uint()?)?;
    }
    // color_diff_sampling_format
    if b.read_bool()? {
        cdf = b.read_uint()?;
    }
    v.chroma = match cdf {
        0 => ChromaFormat::C444,
        1 => ChromaFormat::C422,
        2 => ChromaFormat::C420,
        _ => return Err(Error::Invalid("unknown colour difference format")),
    };
    // scan_format
    if b.read_bool()? {
        v.interlaced = match b.read_uint()? {
            0 => false,
            1 => true,
            _ => return Err(Error::Invalid("unknown source sampling")),
        };
    }
    // frame_rate
    if b.read_bool()? {
        let index = b.read_uint()?;
        if index == 0 {
            let (n, d) = (b.read_uint()?, b.read_uint()?);
            if n == 0 || d == 0 {
                return Err(Error::Invalid("zero frame rate"));
            }
            v.frame_rate = (n, d);
        } else {
            v.frame_rate = preset(&FRAME_RATES, index)?;
        }
    }
    // pixel_aspect_ratio
    if b.read_bool()? {
        let index = b.read_uint()?;
        if index == 0 {
            let (n, d) = (b.read_uint()?, b.read_uint()?);
            if n == 0 || d == 0 {
                return Err(Error::Invalid("zero pixel aspect ratio"));
            }
            v.pixel_aspect_ratio = (n, d);
        } else {
            v.pixel_aspect_ratio = preset(&PIXEL_ASPECT_RATIOS, index)?;
        }
    }
    // clean_area
    if b.read_bool()? {
        v.clean_area = (b.read_uint()?, b.read_uint()?, b.read_uint()?, b.read_uint()?);
    }
    // signal_range
    if b.read_bool()? {
        let index = b.read_uint()?;
        if index == 0 {
            v.luma_offset = b.read_uint()?;
            v.luma_excursion = b.read_uint()?;
            v.color_diff_offset = b.read_uint()?;
            v.color_diff_excursion = b.read_uint()?;
            if v.luma_excursion == 0 || v.color_diff_excursion == 0 {
                return Err(Error::Invalid("zero signal excursion"));
            }
        } else {
            let r = preset(&SIGNAL_RANGES, index)?;
            (v.luma_offset, v.luma_excursion, v.color_diff_offset, v.color_diff_excursion) = (r[0], r[1], r[2], r[3]);
        }
    }
    // color_spec
    if b.read_bool()? {
        let index = b.read_uint()?;
        let s = usize::try_from(index).ok().and_then(|i| COLOR_SPECS.get(i)).copied().ok_or(Error::Invalid("unknown colour spec"))?;
        (v.color_primaries, v.color_matrix, v.transfer_function) = (u64::from(s.0), u64::from(s.1), u64::from(s.2));
        if index == 0 {
            if b.read_bool()? {
                v.color_primaries = b.read_uint()?;
            }
            if b.read_bool()? {
                v.color_matrix = b.read_uint()?;
            }
            if b.read_bool()? {
                v.transfer_function = b.read_uint()?;
            }
        }
    }

    let fields = match b.read_uint()? {
        0 => false,
        1 => true,
        _ => return Err(Error::Invalid("unknown picture coding mode")),
    };
    // picture_dimensions (§11.6.2)
    let (lw, mut lh) = (v.frame_width, v.frame_height);
    let (mut cw, mut ch) = (lw, lh);
    match v.chroma {
        ChromaFormat::C444 => {}
        ChromaFormat::C422 => cw /= 2,
        ChromaFormat::C420 => {
            cw /= 2;
            ch /= 2;
        }
    }
    if fields {
        lh /= 2;
        ch /= 2;
    }
    if lw == 0
        || lh == 0
        || cw == 0
        || ch == 0
        || !v.frame_height.is_multiple_of(lh)
        || !v.frame_width.is_multiple_of(lw)
        || !v.frame_height.is_multiple_of(ch)
        || !v.frame_width.is_multiple_of(cw)
    {
        return Err(Error::Invalid("picture dimensions don't divide the frame"));
    }
    Ok(Sequence {
        major_version,
        profile,
        level,
        fields,
        luma_width: lw,
        luma_height: lh,
        color_diff_width: cw,
        color_diff_height: ch,
        luma_depth: depth(v.luma_excursion)?,
        color_diff_depth: depth(v.color_diff_excursion)?,
        video: v,
    })
}
