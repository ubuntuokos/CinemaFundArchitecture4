//! HEIF / HEIC stills decoded by CFA4 itself (#88): the container by `cfa4-heif`, the pictures by
//! `cfa4-hevc`. Grids are assembled from their tiles, the clean aperture, rotation and mirroring
//! are applied in the file's order, and Y'CbCr becomes RGB with the image's `colr` description
//! (or the HEVC VUI, or BT.601 full range like libheif when neither says anything).
//!
//! What this cannot decode yet comes back as [`MediaError::Unsupported`]: other coding formats
//! (AVIF, JPEG, uncompressed items), HEVC outside Main / Main 10 4:2:0, and images with an alpha
//! plane (monochrome HEVC, #136). The caller then tries the libheif tools ([`super::heif`]).

use cfa4_color::{ColorInfo, Matrix, Primaries, Range, Transfer};
use cfa4_frame::{Chroma, PixelData, VideoFrame};
use cfa4_heif::{Coding, Colour, Image, Transform};
use std::sync::Arc;

use super::Raster;
use crate::{MediaError, Result};

/// Largest decoded picture (RGBA bytes), like the libheif bridge.
const MAX_RGBA_BYTES: u64 = 512 * 1024 * 1024;

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
    if image.alpha.is_some() {
        return Err(MediaError::Unsupported(format!("{name}: HEIF alpha planes need monochrome HEVC (#136)")));
    }
    let mut pic = decode_image(name, &image, image.nclx())?;
    for t in &image.transforms {
        pic = transform(pic, *t)?;
    }
    Ok(Raster { width: pic.width, height: pic.height, rgba: pic.px, has_alpha: false })
}

fn decode_image(name: &str, image: &Image, nclx: Option<&Colour>) -> Result<Rgba> {
    match &image.coding {
        Coding::Hevc { hvcc, data } => decode_hevc(name, hvcc, data, image.nclx().or(nclx)),
        Coding::Grid { columns, width, height, tiles, .. } => {
            let len = check_size(*width, *height)?;
            let mut out = Rgba { width: *width, height: *height, px: vec![0; len] };
            let (mut x, mut y, mut row_height) = (0u32, 0u32, 0u32);
            for (i, tile) in tiles.iter().enumerate() {
                // tiles take the grid's colour description when they have none of their own
                let t = decode_image(name, tile, image.nclx().or(nclx))?;
                if i > 0 && (i as u32).is_multiple_of(*columns) {
                    (x, y) = (0, y.saturating_add(row_height));
                }
                // tiles are placed by their declared size (`ispe`), as libheif does
                let (tw, th) = tile.size.unwrap_or((t.width, t.height));
                paste(&mut out, &t, x, y, tw, th);
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

fn decode_hevc(name: &str, hvcc: &[u8], data: &[u8], nclx: Option<&Colour>) -> Result<Rgba> {
    let hevc_error = |e: cfa4_hevc::Error| match e {
        cfa4_hevc::Error::Unsupported(what) => MediaError::Unsupported(format!("{name}: HEVC {what}")),
        e => MediaError::Decode(format!("{name}: HEVC: {e}")),
    };
    let mut dec = cfa4_hevc::Decoder::from_hvcc(hvcc).map_err(hevc_error)?;
    let mut pictures = dec.decode(data, 0).map_err(hevc_error)?;
    pictures.extend(dec.flush());
    if let Some(e) = dec.take_error() {
        return Err(hevc_error(e));
    }
    let p = pictures.into_iter().next().ok_or_else(|| MediaError::Decode(format!("{name}: HEVC image item has no picture")))?;
    check_size(p.width, p.height)?;
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
/// libheif's default (BT.601 matrix, sRGB, full range).
fn colour(nclx: Option<&Colour>, p: &cfa4_hevc::Picture) -> ColorInfo {
    let (primaries, transfer, matrix, full) = match nclx {
        Some(Colour::Nclx { primaries, transfer, matrix, full_range }) => (*primaries, *transfer, *matrix, *full_range),
        _ if p.color.matrix != 2 || p.color.primaries != 2 => {
            (u16::from(p.color.primaries), u16::from(p.color.transfer), u16::from(p.color.matrix), p.color.full_range)
        }
        _ => (2, 2, 6, true),
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
