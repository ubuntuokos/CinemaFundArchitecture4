//! HEIF / HEIC stills decoded by CFA4 itself (#88): the container by `cfa4-heif`, the pictures by
//! `cfa4-hevc`. Grids are assembled from their tiles, the clean aperture, rotation and mirroring
//! are applied in the file's order, and Y'CbCr becomes RGB with the image's `colr` description
//! (or the HEVC VUI, or BT.601 full range like libheif when neither says anything).
//!
//! An auxiliary alpha image (monochrome HEVC, #136) becomes the alpha channel: its luma samples
//! scaled to 8 bits under the alpha item's own transforms, stretched to the transformed master's
//! size if it differs, as libheif does.
//!
//! What this cannot decode yet comes back as [`MediaError::Unsupported`]: other coding formats
//! (AVIF, JPEG, uncompressed items) and HEVC outside Main / Main 10 4:0:0 / 4:2:0. The caller
//! then tries the libheif tools ([`super::heif`]).

use cfa4_color::{ColorInfo, Matrix, Primaries, Range, Transfer};
use cfa4_frame::{Chroma, PixelData, VideoFrame};
use cfa4_heif::{Coding, Colour, Image, Transform};
use std::sync::Arc;

use super::Raster;
use crate::{MediaError, Result};

/// Largest decoded picture (RGBA bytes), like the libheif bridge.
const MAX_RGBA_BYTES: u64 = 512 * 1024 * 1024;

/// Most RGBA bytes held at once while decoding one file: the colour picture, an alpha canvas
/// and the tile being pasted. Nested grids keep every parent canvas alive while their tiles
/// decode, so they are charged here too.
const MAX_LIVE_BYTES: u64 = 3 * MAX_RGBA_BYTES;

/// The RGBA bytes alive during one decode.
#[derive(Default)]
struct Live {
    bytes: u64,
}

impl Live {
    /// Charge `len` bytes before allocating them.
    fn take(&mut self, len: usize) -> Result<()> {
        let total = self.bytes.saturating_add(len as u64);
        if total > MAX_LIVE_BYTES {
            return Err(MediaError::Unsupported("HEIF pictures exceed the 1.5 GiB decoding limit".into()));
        }
        self.bytes = total;
        Ok(())
    }

    /// Return `len` bytes once their picture is dropped.
    fn give(&mut self, len: usize) {
        self.bytes = self.bytes.saturating_sub(len as u64);
    }
}

/// Whether `bytes` is a HEIF image file.
pub fn sniff(bytes: &[u8]) -> bool {
    cfa4_heif::sniff(bytes)
}

/// An RGBA8 picture being assembled.
struct Rgba {
    width: u32,
    height: u32,
    px: Vec<u8>,
}

fn check_size(width: u32, height: u32) -> Result<usize> {
    let bytes = u64::from(width).saturating_mul(u64::from(height)).saturating_mul(4);
    if width == 0 || height == 0 || bytes > MAX_RGBA_BYTES {
        return Err(MediaError::Unsupported(format!("HEIF picture {width}×{height} exceeds the 512 MiB limit")));
    }
    usize::try_from(bytes).map_err(|_| MediaError::Unsupported("HEIF picture too large".into()))
}

/// Decode the primary image of a HEIF file.
pub fn decode(name: &str, bytes: &[u8]) -> Result<Raster> {
    let heif = cfa4_heif::parse(bytes).map_err(|e| match e {
        cfa4_heif::Error::NotHeif(_) => MediaError::Unsupported(format!("{name}: {e}")),
        e => MediaError::Decode(format!("{name}: {e}")),
    })?;
    let image = heif.primary;
    let mut live = Live::default();
    let mut pic = decode_image(name, &image, image.nclx(), Samples::Colour, &mut live)?;
    for t in &image.transforms {
        pic = transform(pic, *t)?;
    }
    let has_alpha = match &image.alpha {
        Some(a) => {
            let mut alpha = decode_image(name, a, None, Samples::Alpha, &mut live)?;
            for t in &a.transforms {
                alpha = transform(alpha, *t)?;
            }
            merge_alpha(&mut pic, &alpha);
            true
        }
        None => false,
    };
    Ok(Raster { width: pic.width, height: pic.height, rgba: pic.px, has_alpha })
}

/// What an image's samples are: a colour picture, or an alpha plane (its luma alone, in every
/// channel of the RGBA being assembled).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Samples {
    Colour,
    Alpha,
}

/// Put the alpha plane `alpha` (in its red channel) into `pic`'s alpha channel, nearest
/// sample when the two sizes differ.
fn merge_alpha(pic: &mut Rgba, alpha: &Rgba) {
    let (w, h) = (pic.width as usize, pic.height as usize);
    let (aw, ah) = (alpha.width as usize, alpha.height as usize);
    for y in 0..h {
        let ay = (y * ah / h.max(1)).min(ah.saturating_sub(1));
        for x in 0..w {
            let ax = (x * aw / w.max(1)).min(aw.saturating_sub(1));
            let a = alpha.px.get((ay * aw + ax) * 4).copied().unwrap_or(255);
            if let Some(p) = pic.px.get_mut((y * w + x) * 4 + 3) {
                *p = a;
            }
        }
    }
}

/// Decode `image`; its RGBA bytes stay charged to `live`.
fn decode_image(name: &str, image: &Image, nclx: Option<&Colour>, samples: Samples, live: &mut Live) -> Result<Rgba> {
    match &image.coding {
        Coding::Hevc { hvcc, data } => decode_hevc(name, hvcc, data, image.nclx().or(nclx), samples, live),
        Coding::Grid { columns, width, height, tiles, .. } => {
            let len = check_size(*width, *height)?;
            live.take(len)?;
            let mut out = Rgba { width: *width, height: *height, px: vec![0; len] };
            let (mut x, mut y, mut row_height) = (0u32, 0u32, 0u32);
            for (i, tile) in tiles.iter().enumerate() {
                // tiles take the grid's colour description when they have none of their own
                let t = decode_image(name, tile, image.nclx().or(nclx), samples, live)?;
                if i > 0 && (i as u32).is_multiple_of(*columns) {
                    (x, y) = (0, y.saturating_add(row_height));
                }
                // tiles are placed by their declared size (`ispe`), as libheif does
                let (tw, th) = tile.size.unwrap_or((t.width, t.height));
                paste(&mut out, &t, x, y, tw, th);
                live.give(t.px.len());
                x = x.saturating_add(tw);
                row_height = th;
            }
            Ok(out)
        }
        Coding::Other(kind) => Err(MediaError::Unsupported(format!("{name}: HEIF image coded as '{}'", String::from_utf8_lossy(kind)))),
    }
}

/// Copy at most `w`×`h` of `tile` into `canvas` at (`x`, `y`), cropped to the canvas.
fn paste(canvas: &mut Rgba, tile: &Rgba, x: u32, y: u32, w: u32, h: u32) {
    let w = tile.width.min(w).min(canvas.width.saturating_sub(x)) as usize * 4;
    let (cw, tw) = (canvas.width as usize * 4, tile.width as usize * 4);
    for ty in 0..tile.height.min(h).min(canvas.height.saturating_sub(y)) as usize {
        let at = (y as usize + ty) * cw + x as usize * 4;
        if let (Some(dst), Some(src)) = (canvas.px.get_mut(at..at + w), tile.px.get(ty * tw..ty * tw + w)) {
            dst.copy_from_slice(src);
        }
    }
}

fn decode_hevc(name: &str, hvcc: &[u8], data: &[u8], nclx: Option<&Colour>, samples: Samples, live: &mut Live) -> Result<Rgba> {
    let hevc_error = |e: cfa4_hevc::Error| match e {
        cfa4_hevc::Error::Unsupported(what) => MediaError::Unsupported(format!("{name}: HEVC {what}")),
        e => MediaError::Decode(format!("{name}: HEVC: {e}")),
    };
    let mut dec = cfa4_hevc::Decoder::from_hvcc(hvcc).map_err(hevc_error)?;
    // an image item holds one picture: whatever follows its access unit is never decoded
    let data = first_access_unit(data, dec.nal_length_size().unwrap_or(4));
    let mut pictures = dec.decode(data, 0).map_err(hevc_error)?;
    pictures.extend(dec.flush());
    if let Some(e) = dec.take_error() {
        return Err(hevc_error(e));
    }
    let p = pictures.into_iter().next().ok_or_else(|| MediaError::Decode(format!("{name}: HEVC image item has no picture")))?;
    let len = check_size(p.width, p.height)?;
    live.take(len)?;
    if samples == Samples::Alpha {
        return Ok(Rgba { width: p.width, height: p.height, px: alpha_rgba(&p, len) });
    }
    let color = colour(nclx, &p);
    let (w, h) = (p.width as usize, p.height as usize);
    let (cw, ch) = (p.chroma_width as usize, p.chroma_height as usize);
    let data = match (p.y, p.u, p.v) {
        (cfa4_hevc::Plane::U8(y), cfa4_hevc::Plane::U8(u), cfa4_hevc::Plane::U8(v)) => PixelData::Yuv8 {
            planes: [Arc::new(tight(&y, p.y_stride, w, h)), Arc::new(tight(&u, p.uv_stride, cw, ch)), Arc::new(tight(&v, p.uv_stride, cw, ch))],
            chroma: Chroma::C420,
            alpha: None,
        },
        (y, u, v) => {
            let wide = |pl: cfa4_hevc::Plane| match pl {
                cfa4_hevc::Plane::U16(v) => v,
                cfa4_hevc::Plane::U8(v) => v.into_iter().map(u16::from).collect(),
            };
            PixelData::Yuv16 {
                planes: [
                    Arc::new(tight(&wide(y), p.y_stride, w, h)),
                    Arc::new(tight(&wide(u), p.uv_stride, cw, ch)),
                    Arc::new(tight(&wide(v), p.uv_stride, cw, ch)),
                ],
                chroma: Chroma::C420,
                bits: p.bit_depth,
                alpha: None,
            }
        }
    };
    let frame = VideoFrame { width: p.width, height: p.height, data, color, par: (1, 1), pts: cfa4_time::Tick::ZERO };
    Ok(Rgba { width: p.width, height: p.height, px: frame.to_rgba8() })
}

/// The NAL units of `data` (each behind an `n`-byte big-endian length) up to the start of a
/// second access unit: a VCL NAL unit starting a new picture (`first_slice_segment_in_pic_flag`),
/// or an access unit delimiter / parameter set / prefix SEI after a picture's slices.
fn first_access_unit(data: &[u8], n: usize) -> &[u8] {
    let (mut at, mut seen_slice) = (0usize, false);
    while let Some(len_bytes) = data.get(at..at.saturating_add(n)) {
        let len = len_bytes.iter().fold(0usize, |v, &b| v.saturating_mul(256).saturating_add(usize::from(b)));
        let start = at.saturating_add(n);
        let Some(&[h0, _, h2]) = data.get(start..start.saturating_add(3)) else { break };
        let kind = (h0 >> 1) & 0x3f;
        let vcl = kind < 32;
        let starts_picture = vcl && h2 & 0x80 != 0;
        if seen_slice && (starts_picture || matches!(kind, 32..=35 | 39)) {
            return data.get(..at).unwrap_or(data);
        }
        seen_slice |= vcl;
        at = start.saturating_add(len);
    }
    data
}

/// The luma of `p` as 8-bit alpha in all four channels (`len` bytes): higher bit depths keep
/// their top 8 bits, as libheif does.
fn alpha_rgba(p: &cfa4_hevc::Picture, len: usize) -> Vec<u8> {
    let (w, h) = (p.width as usize, p.height as usize);
    let shift = p.bit_depth.saturating_sub(8);
    let mut out = Vec::with_capacity(len);
    for y in 0..h {
        for x in 0..w {
            let i = y * p.y_stride + x;
            let v = match &p.y {
                cfa4_hevc::Plane::U8(v) => v.get(i).map(|&s| u16::from(s)),
                cfa4_hevc::Plane::U16(v) => v.get(i).copied(),
            }
            .unwrap_or(0)
                >> shift;
            let a = u8::try_from(v).unwrap_or(u8::MAX);
            out.extend_from_slice(&[a; 4]);
        }
    }
    out
}

/// A `w`×`h` plane from rows `stride` samples apart (missing samples are zero).
fn tight<T: Copy + Default>(plane: &[T], stride: usize, w: usize, h: usize) -> Vec<T> {
    let mut out = vec![T::default(); w * h];
    for (y, row) in out.chunks_exact_mut(w.max(1)).enumerate().take(h) {
        let at = y * stride;
        if let Some(src) = plane.get(at..at + w) {
            row.copy_from_slice(src);
        }
    }
    out
}

/// The colour of an image: its `colr` nclx, else the HEVC VUI when it describes colour, else
/// libheif's default (BT.601 matrix, sRGB, full range). A range the VUI signals holds even
/// when it leaves the colour codes unspecified.
fn colour(nclx: Option<&Colour>, p: &cfa4_hevc::Picture) -> ColorInfo {
    let vui_full = if p.color.range_signalled { p.color.full_range } else { true };
    let (primaries, transfer, matrix, full) = match nclx {
        Some(Colour::Nclx { primaries, transfer, matrix, full_range }) => (*primaries, *transfer, *matrix, *full_range),
        _ if p.color.matrix != 2 || p.color.primaries != 2 => {
            (u16::from(p.color.primaries), u16::from(p.color.transfer), u16::from(p.color.matrix), p.color.full_range)
        }
        _ => (2, 2, 6, vui_full),
    };
    let code = |c: u16| u8::try_from(c).unwrap_or(2);
    ColorInfo {
        matrix: Matrix::from_code(code(matrix)).unwrap_or(Matrix::Bt601),
        transfer: Transfer::from_code(code(transfer)).unwrap_or(Transfer::Srgb),
        primaries: match code(primaries) {
            5 => Primaries::Bt601_625,
            6 => Primaries::Bt601_525,
            9 => Primaries::Bt2020,
            12 => Primaries::P3D65,
            _ => Primaries::Bt709,
        },
        range: if full { Range::Full } else { Range::Limited },
    }
}

fn transform(pic: Rgba, t: Transform) -> Result<Rgba> {
    let (w, h) = (pic.width as usize, pic.height as usize);
    let px = |x: usize, y: usize| -> [u8; 4] { pic.px.get((y * w + x) * 4..(y * w + x) * 4 + 4).and_then(|p| p.first_chunk::<4>().copied()).unwrap_or([0; 4]) };
    let remap = |nw: usize, nh: usize, f: &dyn Fn(usize, usize) -> (usize, usize)| -> Rgba {
        let mut out = Vec::with_capacity(nw * nh * 4);
        for y in 0..nh {
            for x in 0..nw {
                let (sx, sy) = f(x, y);
                out.extend_from_slice(&px(sx, sy));
            }
        }
        Rgba { width: nw as u32, height: nh as u32, px: out }
    };
    Ok(match t {
        Transform::Crop(c) => {
            let Some((x0, y0, cw, ch)) = c.rect(pic.width, pic.height) else { return Ok(pic) };
            let (x0, y0) = (x0 as usize, y0 as usize);
            remap(cw as usize, ch as usize, &|x, y| (x0 + x, y0 + y))
        }
        // anticlockwise quarter turns
        Transform::Rotate(1) => remap(h, w, &|x, y| (w - 1 - y, x)),
        Transform::Rotate(2) => remap(w, h, &|x, y| (w - 1 - x, h - 1 - y)),
        Transform::Rotate(3) => remap(h, w, &|x, y| (y, h - 1 - x)),
        Transform::Rotate(_) => pic,
        Transform::Mirror { left_right: true } => remap(w, h, &|x, y| (w - 1 - x, y)),
        Transform::Mirror { left_right: false } => remap(w, h, &|x, y| (x, h - 1 - y)),
    })
}

#[cfg(test)]
#[path = "heic_tests.rs"]
mod tests;
