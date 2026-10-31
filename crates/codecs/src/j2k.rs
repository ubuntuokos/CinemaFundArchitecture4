//! JPEG 2000 pictures (`cfa4-j2k`) as [`VideoFrame`]s: the picture essence of JPEG 2000 MXF
//! (broadcast OP1a and the DCP picture track, SMPTE ST 422 / ST 429-4).
//!
//! - Y'CbCr components (4:4:4, 4:2:2 or 4:2:0, with an optional alpha component) are passed
//!   through at their precision.
//! - R'G'B' components (an RGBA picture descriptor, or the multiple-component transform) become
//!   Y'CbCr 4:4:4 with the BT.709 matrix, video range.
//! - X'Y'Z' (the digital cinema profiles, SMPTE ST 428-1) is decoded with the DCI transfer
//!   (gamma 2.6, 52.37 cd/m² at code 4095, 48 cd/m² reference white), converted to P3-D65 and
//!   encoded with the sRGB curve the pipeline decodes P3-D65 frames with, then Y'CbCr 4:4:4 like R'G'B'.

use std::sync::Arc;

use cfa4_color::{ColorInfo, Gamut, Matrix, Primaries, Range, Transfer, spaces};
use cfa4_frame::{Chroma, PixelData, VideoFrame};
use cfa4_j2k::{Header, Image, Plane};
use cfa4_time::Tick;

use crate::video::{DecodedFrame, VideoDecoder};
use crate::{CodecError, Result};

/// What the container says about the components.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct J2kHints {
    /// The components are R, G, B (an MXF RGBA picture descriptor, or GBR coding equations).
    /// Without it, three components are Y'CbCr unless the codestream uses the multiple-component
    /// transform.
    pub rgb: bool,
    /// Y'CbCr components use the full code range.
    pub full_range: bool,
    /// Each picture is two field codestreams, first field on top (an MXF frame layout of
    /// separate fields): they are woven into one frame.
    pub separate_fields: bool,
    /// The container's colorimetry (a codestream carries none).
    pub signalled: Signalled,
}

/// Colorimetry a container signals; `None` keeps the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Signalled {
    pub transfer: Option<Transfer>,
    pub primaries: Option<Primaries>,
    pub matrix: Option<Matrix>,
}

/// How the components of a codestream are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Components {
    Ycbcr(Chroma),
    Gray,
    Rgb,
    Xyz,
}

/// The components of a codestream with `header`, given the container's `hints`.
pub fn components(header: &Header, hints: J2kHints) -> Result<Components> {
    let c = &header.components;
    match c.len() {
        0 => Err(CodecError::Decode("JPEG 2000: no components".into())),
        1 | 2 => Ok(Components::Gray),
        _ if header.digital_cinema() => Ok(Components::Xyz),
        _ if hints.rgb || header.mct => Ok(Components::Rgb),
        _ => {
            let (y, cb, cr) = (c[0], c[1], c[2]);
            if cb.dx != cr.dx || cb.dy != cr.dy || y.dx != 1 || y.dy != 1 {
                return Err(CodecError::Unsupported(format!("JPEG 2000 component sub-sampling {}x{}", cb.dx, cb.dy)));
            }
            match (cb.dx, cb.dy) {
                (1, 1) => Ok(Components::Ycbcr(Chroma::C444)),
                (2, 1) => Ok(Components::Ycbcr(Chroma::C422)),
                (2, 2) => Ok(Components::Ycbcr(Chroma::C420)),
                (dx, dy) => Err(CodecError::Unsupported(format!("JPEG 2000 component sub-sampling {dx}x{dy}"))),
            }
        }
    }
}

/// The colour of the frames [`to_video_frame`] gives for `components`.
/// The container's signalled colorimetry holds for the samples passed through: Y'CbCr and gray
/// take all of it, R'G'B' its transfer and primaries (the matrix is the one it is converted
/// with), X'Y'Z' none (its conversion output is fixed).
pub fn color(components: Components, width: u32, height: u32, hints: J2kHints) -> ColorInfo {
    let s = hints.signalled;
    match components {
        Components::Ycbcr(_) | Components::Gray => ColorInfo {
            matrix: s.matrix.unwrap_or_else(|| cfa4_frame::default_matrix(width, height)),
            transfer: s.transfer.unwrap_or(ColorInfo::REC709.transfer),
            primaries: s.primaries.unwrap_or(ColorInfo::REC709.primaries),
            range: if hints.full_range { Range::Full } else { Range::Limited },
        },
        Components::Rgb => ColorInfo {
            matrix: Matrix::Bt709,
            transfer: s.transfer.unwrap_or(ColorInfo::REC709.transfer),
            primaries: s.primaries.unwrap_or(ColorInfo::REC709.primaries),
            ..ColorInfo::REC709
        },
        Components::Xyz => ColorInfo { matrix: Matrix::Bt709, primaries: Primaries::P3D65, ..ColorInfo::REC709 },
    }
}

/// The samples of `p` as unsigned values of `bits` bits on a `w`×`h` plane whose samples are
/// `step` reference-grid positions apart, starting at the image `origin` (XOsiz, YOsiz): each
/// output sample takes the component sample that covers its reference-grid position (nearest
/// neighbour across the component's own `dx`×`dy` sub-sampling; the component's first sample is
/// the one at `ceil(origin / d)`), repeating the edge samples past the component's edges. Y'CbCr samples
/// shift by the precision difference (video levels scale that way); `full_range` samples (R'G'B',
/// X'Y'Z', alpha) scale across the full range instead, so a 1-bit 1 becomes the `bits`-bit maximum.
fn plane(p: &Plane, bits: u32, w: usize, h: usize, step: (usize, usize), origin: (u32, u32), full_range: bool) -> Vec<u16> {
    let prec = u32::from(p.precision).clamp(1, 16);
    let offset = if p.signed { 1i64 << (prec - 1) } else { 0 };
    let max = (1i64 << prec) - 1;
    let out_max = (1i64 << bits) - 1;
    let level = |v: i32| -> u16 {
        let v = (i64::from(v) + offset).clamp(0, max);
        let v = if full_range {
            (v * out_max + max / 2) / max
        } else if bits >= prec {
            v << (bits - prec)
        } else {
            v >> (prec - bits)
        };
        u16::try_from(v).unwrap_or(u16::MAX)
    };
    let (pw, ph) = (p.width as usize, p.height as usize);
    let mut out = vec![0u16; w.saturating_mul(h)];
    if pw == 0 || ph == 0 || p.data.len() < pw.saturating_mul(ph) {
        return out;
    }
    let (dx, dy) = (usize::from(p.dx.max(1)), usize::from(p.dy.max(1)));
    let (ox, oy) = (origin.0 as usize, origin.1 as usize);
    // the component sample covering reference position `o + i·step`, from the component's first
    let index = |i: usize, step: usize, o: usize, d: usize| (o.saturating_add(i.saturating_mul(step)) / d).saturating_sub(o.div_ceil(d));
    for (y, row) in out.chunks_exact_mut(w.max(1)).enumerate() {
        let sy = index(y, step.1, oy, dy).min(ph - 1) * pw;
        for (x, o) in row.iter_mut().enumerate() {
            let sx = index(x, step.0, ox, dx).min(pw - 1);
            *o = p.data.get(sy + sx).map_or(0, |&v| level(v));
        }
    }
    out
}

/// Y'CbCr 4:4:4, video range, BT.709 matrix, from R'G'B' planes of `bits` bits (in place).
fn rgb_to_ycbcr(r: &mut [u16], g: &mut [u16], b: &mut [u16], bits: u32) {
    let (kr, kb) = Matrix::Bt709.kr_kb();
    let kg = 1.0 - kr - kb;
    let max = ((1u32 << bits) - 1) as f32;
    let s = (1u32 << bits.saturating_sub(8)) as f32;
    for ((r, g), b) in r.iter_mut().zip(g.iter_mut()).zip(b.iter_mut()) {
        let (rr, gg, bb) = (f32::from(*r) / max, f32::from(*g) / max, f32::from(*b) / max);
        let y = kr * rr + kg * gg + kb * bb;
        let u = (bb - y) / (2.0 * (1.0 - kb));
        let v = (rr - y) / (2.0 * (1.0 - kr));
        *r = ((16.0 + 219.0 * y) * s + 0.5).clamp(0.0, max) as u16;
        *g = ((128.0 + 224.0 * u) * s + 0.5).clamp(0.0, max) as u16;
        *b = ((128.0 + 224.0 * v) * s + 0.5).clamp(0.0, max) as u16;
    }
}

/// X'Y'Z' planes of `bits` bits to P3-D65 R'G'B' (in place), for [`rgb_to_ycbcr`].
fn xyz_to_rgb(x: &mut [u16], y: &mut [u16], z: &mut [u16], bits: u32) {
    let max = ((1u32 << bits) - 1) as f32;
    // DCI: luminance = 52.37 cd/m² × (code / max)^2.6; reference white (48 cd/m²) is 1.0
    let decode: Vec<f32> = (0..=(1u32 << bits) - 1).map(|v| (v as f32 / max).powf(2.6) * (52.37 / 48.0)).collect();
    let m = spaces::to_f32(&spaces::inverse(&Gamut::P3D65.to_xyz()));
    let lin = |v: u16| decode.get(usize::from(v)).copied().unwrap_or(0.0);
    // the inverse of the curve the pipeline decodes these frames with (`Curve::Sdr`, the sRGB
    // function in `cfa4_color::transform`), so linear light survives the round trip
    let enc = |v: f32| (cfa4_color::linear_to_srgb(v.clamp(0.0, 1.0)) * max + 0.5) as u16;
    for ((x, y), z) in x.iter_mut().zip(y.iter_mut()).zip(z.iter_mut()) {
        let [r, g, b] = spaces::apply3(&m, [lin(*x), lin(*y), lin(*z)]);
        (*x, *y, *z) = (enc(r), enc(g), enc(b));
    }
}

/// A decoded JPEG 2000 image as a [`VideoFrame`] (see the module documentation).
///
/// `origin` is the image area's top-left corner on the reference grid (XOsiz, YOsiz from the
/// [`Header`]): sub-sampled components start at `ceil(origin / d)`.
pub fn to_video_frame(image: &Image, origin: (u32, u32), components: Components, hints: J2kHints) -> Result<VideoFrame> {
    // the output planes are full-size even when the components are sub-sampled, so the codestream's
    // own sample limit does not bound them
    let samples = u64::from(image.width).saturating_mul(u64::from(image.height)).saturating_mul(4);
    if samples > cfa4_j2k::MAX_SAMPLES {
        return Err(CodecError::Unsupported(format!("JPEG 2000 picture of {}×{} is too large", image.width, image.height)));
    }
    let (w, h) = (image.width as usize, image.height as usize);
    let c = &image.components;
    let first = c.first().ok_or_else(|| CodecError::Decode("JPEG 2000: no components".into()))?;
    let colour_planes = if matches!(components, Components::Gray) { 1 } else { 3 };
    let bits = c.iter().take(colour_planes).map(|p| u32::from(p.precision)).max().unwrap_or(8).clamp(8, 16);
    let chroma = match components {
        Components::Ycbcr(ch) => ch,
        _ => Chroma::C444,
    };
    let (sx, sy) = chroma.shifts();
    let (cw, ch) = ((w + (1 << sx) - 1) >> sx, (h + (1 << sy) - 1) >> sy);
    let mid = 1u16 << (bits - 1);
    let (full, sub) = ((1, 1), (1 << sx, 1 << sy));
    let rgb = matches!(components, Components::Rgb | Components::Xyz);
    // full-range luma (gray or Y') spans the whole code range too; chroma keeps the shift that
    // holds its mid level
    let luma_full = rgb || hints.full_range;
    let (mut p0, mut p1, mut p2) = match c.get(1..3) {
        Some([p1, p2]) if colour_planes == 3 => {
            (plane(first, bits, w, h, full, origin, luma_full), plane(p1, bits, cw, ch, sub, origin, rgb), plane(p2, bits, cw, ch, sub, origin, rgb))
        }
        _ => (plane(first, bits, w, h, full, origin, luma_full), vec![mid; cw * ch], vec![mid; cw * ch]),
    };
    match components {
        Components::Rgb => rgb_to_ycbcr(&mut p0, &mut p1, &mut p2, bits),
        Components::Xyz => {
            xyz_to_rgb(&mut p0, &mut p1, &mut p2, bits);
            rgb_to_ycbcr(&mut p0, &mut p1, &mut p2, bits);
        }
        Components::Ycbcr(_) | Components::Gray => {}
    }
    let alpha = c.get(colour_planes).map(|a| plane(a, bits, w, h, full, origin, true));
    let data = if bits == 8 {
        let to8 = |p: Vec<u16>| Arc::new(p.into_iter().map(|v| v as u8).collect::<Vec<u8>>());
        PixelData::Yuv8 { planes: [to8(p0), to8(p1), to8(p2)], chroma, alpha: alpha.map(to8) }
    } else {
        PixelData::Yuv16 { planes: [Arc::new(p0), Arc::new(p1), Arc::new(p2)], chroma, bits, alpha: alpha.map(Arc::new) }
    };
    Ok(VideoFrame { width: image.width, height: image.height, data, color: color(components, image.width, image.height, hints), par: (1, 1), pts: Tick::ZERO })
}

/// [`components`] with the component transform decoding actually applied: tile-part CODs take
/// precedence over the main one and can switch it on or off, which makes the planes R'G'B' or
/// leaves them as coded (the container's R'G'B' and digital-cinema hints still hold).
fn after_decode(header: &Header, hints: J2kHints, image: &Image) -> Result<Components> {
    components(&Header { mct: image.mct, ..header.clone() }, hints)
}

/// The codestream after the first one in `data`: the first is walked marker segment by marker
/// segment and tile-part by tile-part (Psot) to its EOC, so bytes inside a COM or other segment
/// are never taken for a boundary. A last tile-part of unknown length (Psot 0) runs to the EOC
/// followed by SOC and SIZ (coded data never holds an 0xFF followed by a marker code).
fn second_field(data: &[u8]) -> Option<&[u8]> {
    let be16 = |at: usize| Some(usize::from(u16::from_be_bytes([*data.get(at)?, *data.get(at + 1)?])));
    let mut pos = 2; // after SOC
    loop {
        match be16(pos)? {
            0xFFD9 => return data.get(pos + 2..).filter(|rest| rest.starts_with(&[0xFF, 0x4F])),
            0xFF90 => {
                let psot = u32::from_be_bytes([*data.get(pos + 6)?, *data.get(pos + 7)?, *data.get(pos + 8)?, *data.get(pos + 9)?]) as usize;
                if psot == 0 {
                    let rest = data.get(pos..)?;
                    let at = rest.windows(6).position(|w| w == [0xFF, 0xD9, 0xFF, 0x4F, 0xFF, 0x51])?;
                    return rest.get(at + 2..);
                }
                // SOT, Lsot, Isot, Psot, TPsot, TNsot: 12 bytes before any data
                if psot < 12 {
                    return None;
                }
                pos = pos.checked_add(psot)?;
            }
            m if m >> 8 == 0xFF => pos = pos.checked_add(2 + be16(pos + 2)?)?,
            _ => return None,
        }
    }
}

/// `p` of a field `height` rows high whose image starts at reference row `y0` (YOsiz), with
/// every row repeated over the field lines it covers (`dy` 1): field components sub-sampled
/// vertically cannot be woven row by row. Line `r` takes the component row covering `y0 + r`,
/// counted from the component's first row `ceil(y0 / dy)` (as [`plane`] maps them).
fn unsubsample_rows(p: &Plane, height: u32, y0: u32) -> Plane {
    let dy = usize::from(p.dy.max(1));
    if dy == 1 {
        return p.clone();
    }
    let (w, ph, y0) = (p.width as usize, p.height as usize, y0 as usize);
    let data = (0..height as usize)
        .flat_map(|r| {
            let row = (y0.saturating_add(r) / dy).saturating_sub(y0.div_ceil(dy));
            let at = row.min(ph.saturating_sub(1)).saturating_mul(w);
            p.data.get(at..at.saturating_add(w)).unwrap_or(&[]).iter().copied()
        })
        .collect();
    Plane { height, dy: 1, data, ..p.clone() }
}

/// The frame of two fields: `top`'s rows on the even lines, `bottom`'s on the odd ones. Vertically
/// sub-sampled components are expanded first (4:2:0 fields give a 4:2:2 frame); `y0` is the
/// fields' YOsiz.
fn weave(top: &Image, bottom: &Image, y0: u32) -> Result<Image> {
    // the expanded planes are full width × field height each: cap them before allocating
    let woven = u64::from(top.width).saturating_mul(u64::from(top.height)).saturating_mul(2).saturating_mul(top.components.len() as u64);
    if woven > cfa4_j2k::MAX_SAMPLES {
        return Err(CodecError::Unsupported(format!("JPEG 2000: woven {}x{} picture too large", top.width, u64::from(top.height) * 2)));
    }
    let same = top.width == bottom.width
        && top.height == bottom.height
        && top.components.len() == bottom.components.len()
        && top.components.iter().zip(&bottom.components).all(|(a, b)| (a.width, a.height, a.precision, a.signed) == (b.width, b.height, b.precision, b.signed));
    if !same {
        return Err(CodecError::Unsupported("JPEG 2000: the two fields of a picture differ in size or components".into()));
    }
    let components = top
        .components
        .iter()
        .zip(&bottom.components)
        .map(|(a, b)| {
            let (a, b) = (unsubsample_rows(a, top.height, y0), unsubsample_rows(b, bottom.height, y0));
            let w = (a.width as usize).max(1);
            let data = a.data.chunks(w).zip(b.data.chunks(w)).flat_map(|(ra, rb)| ra.iter().chain(rb).copied()).collect();
            Plane { height: a.height.saturating_mul(2), data, ..a }
        })
        .collect();
    Ok(Image { width: top.width, height: top.height.saturating_mul(2), components, mct: top.mct || bottom.mct })
}

/// Our JPEG 2000 decoder: every picture is a codestream of its own.
pub struct J2kDecoder {
    hints: J2kHints,
}

impl J2kDecoder {
    pub fn new(hints: J2kHints) -> Self {
        Self { hints }
    }
}

impl VideoDecoder for J2kDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let err = |e: cfa4_j2k::Error| match e {
            cfa4_j2k::Error::Unsupported(s) => CodecError::Unsupported(format!("JPEG 2000: {s}")),
            e => CodecError::Decode(e.to_string()),
        };
        let cs = cfa4_j2k::codestream(sample).map_err(err)?;
        let mut header = cfa4_j2k::parse_header(cs).map_err(err)?;
        let mut image = cfa4_j2k::decode(cs).map_err(err)?;
        let mut origin = (header.x0, header.y0);
        if self.hints.separate_fields {
            // a missing or damaged second field repeats the first; one we cannot decode is
            // refused like any unsupported picture
            let second = match second_field(cs).map(cfa4_j2k::decode) {
                Some(Ok(f)) => f,
                Some(Err(e @ cfa4_j2k::Error::Unsupported(_))) => return Err(err(e)),
                Some(Err(_)) | None => image.clone(),
            };
            image = weave(&image, &second, header.y0)?;
            // the woven components have every line (see `weave`)
            header.components.iter_mut().for_each(|c| c.dy = 1);
            origin.1 = 0;
        }
        let components = after_decode(&header, self.hints, &image)?;
        Ok(vec![DecodedFrame { pts, frame: to_video_frame(&image, origin, components, self.hints)?, draft: false }])
    }
    fn flush(&mut self) -> Vec<DecodedFrame> {
        Vec::new()
    }
    fn reset(&mut self) {}
    fn name(&self) -> &str {
        "CFA4 JPEG 2000"
    }
    fn intra_only(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfa4_j2k::ComponentInfo;

    fn header(components: &[(u8, u8)], capabilities: u16, mct: bool) -> Header {
        Header {
            x0: 0,
            y0: 0,
            x1: 8,
            y1: 4,
            tile_x0: 0,
            tile_y0: 0,
            tile_w: 8,
            tile_h: 4,
            components: components.iter().map(|&(dx, dy)| ComponentInfo { precision: 12, signed: false, dx, dy }).collect(),
            levels: 0,
            layers: 1,
            reversible: true,
            progression: 0,
            capabilities,
            mct,
        }
    }

    fn plane_of(w: u32, h: u32, precision: u8, signed: bool, v: i32) -> Plane {
        Plane { width: w, height: h, precision, signed, dx: 1, dy: 1, data: vec![v; (w * h) as usize] }
    }

    #[test]
    fn components_follow_the_codestream_and_the_container() {
        let none = J2kHints::default();
        let rgb = J2kHints { rgb: true, ..none };
        assert_eq!(components(&header(&[(1, 1), (2, 1), (2, 1)], 0, false), none).ok(), Some(Components::Ycbcr(Chroma::C422)));
        assert_eq!(components(&header(&[(1, 1), (2, 2), (2, 2)], 0, false), none).ok(), Some(Components::Ycbcr(Chroma::C420)));
        assert_eq!(components(&header(&[(1, 1); 3], 0, false), none).ok(), Some(Components::Ycbcr(Chroma::C444)));
        assert_eq!(components(&header(&[(1, 1); 3], 0, false), rgb).ok(), Some(Components::Rgb));
        assert_eq!(components(&header(&[(1, 1); 3], 0, true), none).ok(), Some(Components::Rgb));
        assert_eq!(components(&header(&[(1, 1); 3], 3, true), none).ok(), Some(Components::Xyz));
        assert_eq!(components(&header(&[(1, 1); 3], 4, false), rgb).ok(), Some(Components::Xyz));
        assert_eq!(components(&header(&[(1, 1)], 0, false), none).ok(), Some(Components::Gray));
        assert!(components(&header(&[(1, 1), (3, 1), (3, 1)], 0, false), none).is_err());
        assert!(components(&header(&[(1, 1), (2, 1), (1, 1)], 0, false), none).is_err());
        assert!(components(&header(&[], 0, false), none).is_err());
    }

    /// DCI X'Y'Z' of D65 white at 48 cd/m² is P3-D65 white; code 0 is black.
    #[test]
    fn xyz_white_and_black_map_to_white_and_black() {
        let code = |v: f64| ((v * 48.0 / 52.37).powf(1.0 / 2.6) * 4095.0).round() as i32;
        let white = [code(0.3127 / 0.3290), code(1.0), code((1.0 - 0.3127 - 0.3290) / 0.3290)];
        for (xyz, y, c) in [(white, 235 * 16, 128 * 16), ([0; 3], 16 * 16, 128 * 16)] {
            let image = Image { width: 8, height: 4, components: xyz.iter().map(|&v| plane_of(8, 4, 12, false, v)).collect(), mct: false };
            let f = to_video_frame(&image, (0, 0), Components::Xyz, J2kHints::default()).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(f.color.primaries, Primaries::P3D65);
            let PixelData::Yuv16 { planes, bits: 12, .. } = &f.data else { panic!("12-bit Y'CbCr expected") };
            assert!(planes[0].iter().all(|&v| v.abs_diff(y) <= 2), "Y {:?} for {xyz:?}", planes[0].first());
            assert!(planes[1].iter().chain(planes[2].iter()).all(|&v| v.abs_diff(c) <= 2), "CbCr for {xyz:?}");
        }
    }

    #[test]
    fn signed_components_are_offset_and_small_planes_are_padded() {
        // a signed 8-bit gray component of 2×2 in a 4×3 image: 0 is mid-grey, edges repeat
        let image = Image { width: 4, height: 3, components: vec![plane_of(2, 2, 8, true, -128)], mct: false };
        let f = to_video_frame(&image, (0, 0), Components::Gray, J2kHints::default()).unwrap_or_else(|e| panic!("{e}"));
        let PixelData::Yuv8 { planes, .. } = &f.data else { panic!("8-bit expected") };
        assert_eq!(planes[0].len(), 12);
        assert!(planes[0].iter().all(|&v| v == 0));
        let image = Image { width: 2, height: 2, components: vec![plane_of(2, 2, 8, true, 0)], mct: false };
        let f = to_video_frame(&image, (0, 0), Components::Gray, J2kHints::default()).unwrap_or_else(|e| panic!("{e}"));
        let PixelData::Yuv8 { planes, .. } = &f.data else { panic!("8-bit expected") };
        assert!(planes[0].iter().all(|&v| v == 128));
    }

    /// Separate-field pictures: the two codestreams of one sample are woven, first field on top.
    #[test]
    fn separate_fields_are_woven_into_one_frame() {
        let field = |v: i32| Image { width: 4, height: 2, components: vec![plane_of(4, 2, 8, false, v)], mct: false };
        let opts = cfa4_j2k::EncodeOptions::lossless();
        let mut sample = cfa4_j2k::encode(&field(16), &opts).unwrap_or_else(|e| panic!("{e}"));
        sample.extend(cfa4_j2k::encode(&field(235), &opts).unwrap_or_else(|e| panic!("{e}")));
        let hints = J2kHints { separate_fields: true, ..J2kHints::default() };
        let out = J2kDecoder::new(hints).decode(&sample, 0).unwrap_or_else(|e| panic!("{e}"));
        let f = &out[0].frame;
        assert_eq!((f.width, f.height), (4, 4));
        let PixelData::Yuv8 { planes, .. } = &f.data else { panic!("8-bit expected") };
        let rows: Vec<u8> = planes[0].chunks(4).map(|r| r[0]).collect();
        assert_eq!(rows, [16, 235, 16, 235]);
        // 4:2:0 fields: each field's chroma rows stay on that field's lines (a 4:2:2 frame)
        let yuv420 = |y: i32, c: i32| Image {
            width: 4,
            height: 4,
            components: vec![
                plane_of(4, 4, 8, false, y),
                Plane { dx: 2, dy: 2, ..plane_of(2, 2, 8, false, c) },
                Plane { dx: 2, dy: 2, ..plane_of(2, 2, 8, false, c) },
            ],
            mct: false,
        };
        let opts420 = cfa4_j2k::EncodeOptions { mct: false, ..cfa4_j2k::EncodeOptions::lossless() };
        let mut sample = cfa4_j2k::encode(&yuv420(100, 60), &opts420).unwrap_or_else(|e| panic!("{e}"));
        sample.extend(cfa4_j2k::encode(&yuv420(100, 200), &opts420).unwrap_or_else(|e| panic!("{e}")));
        let out = J2kDecoder::new(hints).decode(&sample, 0).unwrap_or_else(|e| panic!("{e}"));
        let PixelData::Yuv8 { planes, chroma, .. } = &out[0].frame.data else { panic!("8-bit expected") };
        assert_eq!(*chroma, Chroma::C422);
        let cb_rows: Vec<u8> = planes[1].chunks(2).map(|r| r[0]).collect();
        assert_eq!(cb_rows, [60, 200, 60, 200, 60, 200, 60, 200]);
        // field boundaries come from the marker segments, not from bytes inside a COM segment
        let mut first = cfa4_j2k::encode(&field(16), &opts).unwrap_or_else(|e| panic!("{e}"));
        let com = [0xFF, 0x64, 0x00, 0x0A, 0x00, 0x00, 0xFF, 0xD9, 0xFF, 0x4F, 0xFF, 0x51];
        let siz_end = 4 + usize::from(u16::from_be_bytes([first[4], first[5]]));
        first.splice(siz_end..siz_end, com);
        let second = cfa4_j2k::encode(&field(235), &opts).unwrap_or_else(|e| panic!("{e}"));
        let mut sample = first.clone();
        sample.extend_from_slice(&second);
        assert_eq!(second_field(&sample), Some(&second[..]));
        // an unaligned field origin (YOsiz 1, dy 2): rows [A, B] cover field lines A A A B
        let p = Plane { dy: 2, data: vec![1, 2], ..plane_of(1, 2, 8, false, 0) };
        assert_eq!(unsubsample_rows(&p, 4, 1).data, [1, 1, 1, 2]);
        assert_eq!(unsubsample_rows(&p, 4, 0).data, [1, 1, 2, 2]);
        // huge woven pictures are refused before anything is expanded
        let huge = Image { width: 1_000_000, height: 1_000_000, components: vec![Plane { dx: 255, dy: 255, ..plane_of(4, 4, 8, false, 0) }], mct: false };
        assert!(weave(&huge, &huge, 0).is_err());
        // a lone field is line-doubled rather than refused
        let lone = cfa4_j2k::encode(&field(16), &opts).unwrap_or_else(|e| panic!("{e}"));
        let out = J2kDecoder::new(hints).decode(&lone, 0).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out[0].frame.height, 4);
    }

    /// Signalled colorimetry follows what was decoded: all of it for Y'CbCr, transfer and
    /// primaries for R'G'B', nothing for X'Y'Z'.
    #[test]
    fn signalled_colorimetry_follows_the_decoded_components() {
        let s = Signalled { transfer: Some(Transfer::Pq), primaries: Some(Primaries::Bt2020), matrix: Some(Matrix::Bt2020Ncl) };
        let hints = J2kHints { signalled: s, ..J2kHints::default() };
        let c = color(Components::Ycbcr(Chroma::C444), 3840, 2160, hints);
        assert_eq!((c.transfer, c.primaries, c.matrix), (Transfer::Pq, Primaries::Bt2020, Matrix::Bt2020Ncl));
        let c = color(Components::Rgb, 3840, 2160, hints);
        assert_eq!((c.transfer, c.primaries, c.matrix), (Transfer::Pq, Primaries::Bt2020, Matrix::Bt709));
        assert_eq!(color(Components::Xyz, 3840, 2160, hints).primaries, Primaries::P3D65);
    }

    #[test]
    fn the_component_transform_decoding_applied_decides_rgb() {
        let mut image = Image { width: 2, height: 2, components: vec![plane_of(2, 2, 8, false, 0); 3], mct: true };
        let j2c = cfa4_j2k::encode(&Image { mct: false, ..image.clone() }, &cfa4_j2k::EncodeOptions { mct: false, ..cfa4_j2k::EncodeOptions::lossless() })
            .unwrap_or_else(|e| panic!("{e}"));
        let mut header = cfa4_j2k::parse_header(&j2c).unwrap_or_else(|e| panic!("{e}"));
        let ycbcr = Components::Ycbcr(Chroma::C444);
        let rgb_hint = J2kHints { rgb: true, ..J2kHints::default() };
        // main COD off, a tile COD on: R'G'B'
        header.mct = false;
        assert_eq!(after_decode(&header, J2kHints::default(), &image).ok(), Some(Components::Rgb));
        // main COD on, every tile COD off: the planes stay as coded
        header.mct = true;
        image.mct = false;
        assert_eq!(after_decode(&header, J2kHints::default(), &image).ok(), Some(ycbcr));
        assert_eq!(after_decode(&header, rgb_hint, &image).ok(), Some(Components::Rgb));
        header.mct = false;
        assert_eq!(after_decode(&header, J2kHints::default(), &image).ok(), Some(ycbcr));
    }

    /// A 1-bit alpha component next to 8-bit colour: opaque is 255, not 128.
    #[test]
    fn low_precision_alpha_scales_to_the_full_range() {
        let comps = vec![plane_of(2, 2, 8, false, 7), plane_of(2, 2, 8, false, 7), plane_of(2, 2, 8, false, 7), plane_of(2, 2, 1, false, 1)];
        let f = to_video_frame(&Image { width: 2, height: 2, components: comps, mct: false }, (0, 0), Components::Ycbcr(Chroma::C444), J2kHints::default())
            .unwrap_or_else(|e| panic!("{e}"));
        let PixelData::Yuv8 { alpha: Some(a), .. } = &f.data else { panic!("8-bit with alpha expected") };
        assert!(a.iter().all(|&v| v == 255));
    }

    /// Sub-sampled gray, R'G'B' and alpha components are spread over the reference grid, not
    /// squeezed into its top-left corner with repeated edges.
    #[test]
    fn sub_sampled_components_are_resampled_to_the_reference_grid() {
        let gray = Plane { width: 2, height: 1, precision: 8, signed: false, dx: 2, dy: 1, data: vec![16, 235] };
        let f = to_video_frame(&Image { width: 4, height: 1, components: vec![gray], mct: false }, (0, 0), Components::Gray, J2kHints::default())
            .unwrap_or_else(|e| panic!("{e}"));
        let PixelData::Yuv8 { planes, .. } = &f.data else { panic!("8-bit expected") };
        assert_eq!(planes[0].as_slice(), &[16, 16, 235, 235]);

        // XOsiz 1: the component's samples cover reference x 2-3 and 4-5, so pixels at x 1 (past
        // the edge), 2 and 3 take the first; the old mapping shifted the plane a pixel left
        let gray = Plane { width: 2, height: 1, precision: 8, signed: false, dx: 2, dy: 1, data: vec![16, 235] };
        let f = to_video_frame(&Image { width: 4, height: 1, components: vec![gray], mct: false }, (1, 0), Components::Gray, J2kHints::default())
            .unwrap_or_else(|e| panic!("{e}"));
        let PixelData::Yuv8 { planes, .. } = &f.data else { panic!("8-bit expected") };
        assert_eq!(planes[0].as_slice(), &[16, 16, 16, 235]);

        // 2×2 R'G'B' components (2×2 sub-sampled) and an alpha component (1×2) on a 4×4 grid
        let rgb = |dx, dy, data: Vec<i32>| Plane { width: 4 / u32::from(dx), height: 4 / u32::from(dy), precision: 8, signed: false, dx, dy, data };
        let quad = vec![0, 255, 255, 0];
        let comps = vec![rgb(2, 2, quad.clone()), rgb(2, 2, quad.clone()), rgb(2, 2, quad), rgb(1, 2, vec![255, 255, 255, 255, 0, 0, 0, 0])];
        let f = to_video_frame(&Image { width: 4, height: 4, components: comps, mct: false }, (0, 0), Components::Rgb, J2kHints::default())
            .unwrap_or_else(|e| panic!("{e}"));
        let PixelData::Yuv8 { planes, alpha: Some(a), .. } = &f.data else { panic!("8-bit with alpha expected") };
        let (black, white) = (16, 235);
        #[rustfmt::skip]
        let want = [black, black, white, white, black, black, white, white, white, white, black, black, white, white, black, black];
        assert_eq!(planes[0].as_slice(), &want);
        assert_eq!(a.as_slice(), &[255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 0, 0, 0, 0, 0]);
    }

    /// R'G'B' components below 8 bits scale to the full 8-bit range before the matrix: 1-bit
    /// white is video white (235), not the 126 a bit shift to 128 gave.
    #[test]
    fn low_precision_rgb_scales_to_the_full_range() {
        for (prec, white) in [(1u8, 1), (4, 15)] {
            let image = Image { width: 2, height: 2, components: vec![plane_of(2, 2, prec, false, white); 3], mct: false };
            let f = to_video_frame(&image, (0, 0), Components::Rgb, J2kHints::default()).unwrap_or_else(|e| panic!("{e}"));
            let PixelData::Yuv8 { planes, .. } = &f.data else { panic!("8-bit expected") };
            assert!(planes[0].iter().all(|&v| v == 235), "{prec}-bit white gives Y {:?}", planes[0].first());
        }
    }

    /// Full-range gray below 8 bits keeps its white: a 1-bit 1 is 255, not the 128 a shift gives
    /// (which `Range::Full` would show at half brightness). Limited range still shifts.
    #[test]
    fn low_precision_full_range_gray_keeps_its_white() {
        let image = Image { width: 2, height: 2, components: vec![plane_of(2, 2, 1, false, 1)], mct: false };
        for (full_range, want) in [(true, 255), (false, 128)] {
            let f = to_video_frame(&image, (0, 0), Components::Gray, J2kHints { full_range, ..J2kHints::default() }).unwrap_or_else(|e| panic!("{e}"));
            let PixelData::Yuv8 { planes, .. } = &f.data else { panic!("8-bit expected") };
            assert!(planes[0].iter().all(|&v| v == want), "full range {full_range}: {:?}", planes[0].first());
        }
    }

    /// A huge reference grid with tiny sub-sampled components is refused before the full-size
    /// output planes are allocated.
    #[test]
    fn huge_output_frames_are_refused() {
        let image = Image { width: 1_000_000, height: 1_000_000, components: vec![plane_of(4, 4, 8, false, 0)], mct: false };
        assert!(to_video_frame(&image, (0, 0), Components::Gray, J2kHints::default()).is_err());
    }

    /// Images whose planes disagree with their size (hostile input) give a frame, never a panic.
    #[test]
    fn inconsistent_planes_do_not_panic() {
        let mut short = plane_of(8, 4, 12, false, 100);
        short.data.truncate(3);
        let image = Image {
            width: 8,
            height: 4,
            components: vec![short, plane_of(0, 0, 16, false, 0), plane_of(9, 9, 1, true, 1), plane_of(1, 1, 16, false, 7)],
            mct: false,
        };
        for c in [Components::Xyz, Components::Rgb, Components::Ycbcr(Chroma::C420), Components::Gray] {
            assert!(to_video_frame(&image, (0, 0), c, J2kHints::default()).is_ok());
        }
        assert!(to_video_frame(&Image { width: 0, height: 0, components: vec![], mct: false }, (0, 0), Components::Gray, J2kHints::default()).is_err());
    }
}
