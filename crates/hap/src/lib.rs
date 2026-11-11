//! Clean-room pure-Rust Hap video decoder.
//!
//! Implemented from the public Hap specification (Vidvox, `HapVideoDRAFT.md`: section headers,
//! top-level section types, multiple-image sections, decode instructions and chunks) and the
//! specifications it references: the Snappy format description, the OpenGL S3TC and RGTC
//! extensions and the scaled YCoCg DXT5 paper (van Waveren and Castaño, 2007). The integer
//! rounding the specifications leave open was matched against ffmpeg, which is used only as an
//! external fixture generator and test oracle. No decoder source code was read.
//!
//! Supported: `Hap1` (RGB DXT1), `Hap5` (RGBA DXT5), `HapY` (scaled YCoCg DXT5), `HapA`
//! (RGTC1 alpha only, decoded as a grey matte) and `HapM` (scaled YCoCg DXT5 plus RGTC1 alpha), with no second-stage
//! compression, Snappy, or chunked decode instructions. `Hap7` (BC7) and `HapH` (BC6H) are
//! rejected with [`Error::Unsupported`].
//!
//! ```no_run
//! let mut dec = cfa4_hap::Decoder::new(*b"Hap1", 1920, 1080).unwrap();
//! let frame: Vec<u8> = Vec::new(); // one QuickTime sample
//! let pic = dec.decode(&frame).unwrap();
//! println!("{}x{} alpha: {}", pic.width, pic.height, pic.alpha);
//! ```

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

mod bc;
mod error;
mod snappy;

pub use error::{Error, Result};

use error::ensure;

/// Largest accepted frame side (pixels), so allocations stay bounded.
const MAX_SIDE: u32 = 16384;

/// Most chunks accepted in one frame.
const MAX_CHUNKS: usize = 4096;

/// Is `fourcc` one of the Hap formats (`Hap1`, `Hap5`, `HapY`, `HapM`, `HapA`, `Hap7`, `HapH`)?
pub fn is_hap(fourcc: &[u8; 4]) -> bool {
    matches!(fourcc, b"Hap1" | b"Hap5" | b"HapY" | b"HapM" | b"HapA" | b"Hap7" | b"HapH")
}

/// A decoded frame: interleaved 8-bit RGBA, `width * height * 4` bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Whether the alpha channel carries information (`Hap5`, `HapM`). A `HapA` (alpha-only)
    /// stream decodes to an opaque grey matte.
    pub alpha: bool,
}

/// The texture format of one image section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Texture {
    /// RGB DXT1 / BC1.
    Rgb,
    /// RGBA DXT5 / BC3.
    Rgba,
    /// Scaled YCoCg in DXT5 / BC3.
    YCoCg,
    /// Alpha in RGTC1 / BC4.
    Alpha,
}

impl Texture {
    fn block_size(self) -> usize {
        match self {
            Texture::Rgb | Texture::Alpha => 8,
            Texture::Rgba | Texture::YCoCg => 16,
        }
    }
}

/// One section: its type and its data.
struct Section<'a> {
    kind: u8,
    data: &'a [u8],
}

/// Split `buf` into the sections it holds, in order.
fn sections(mut buf: &[u8]) -> Result<Vec<Section<'_>>> {
    let mut out = Vec::new();
    while !buf.is_empty() {
        let (sec, rest) = section(buf)?;
        out.push(sec);
        buf = rest;
        ensure!(out.len() <= 64, "too many sections");
    }
    Ok(out)
}

/// Read the section at the start of `buf`; returns it and the bytes after it.
fn section(buf: &[u8]) -> Result<(Section<'_>, &[u8])> {
    let h = buf.get(..4).ok_or_else(|| Error::Invalid("truncated section header".into()))?;
    let kind = h[3];
    let (size, hdr) = if h[..3] == [0, 0, 0] {
        let s = buf.get(4..8).ok_or_else(|| Error::Invalid("truncated section header".into()))?;
        (u32::from_le_bytes([s[0], s[1], s[2], s[3]]), 8usize)
    } else {
        (u32::from_le_bytes([h[0], h[1], h[2], 0]), 4usize)
    };
    let end = hdr.checked_add(size as usize).ok_or_else(|| Error::Invalid("section size".into()))?;
    let data = buf.get(hdr..end).ok_or_else(|| Error::Invalid(format!("section of {size} bytes runs past the frame")))?;
    Ok((Section { kind, data }, buf.get(end..).unwrap_or_default()))
}

/// A Hap decoder for one stream.
pub struct Decoder {
    width: u32,
    height: u32,
}

impl Decoder {
    /// A decoder for a `width` x `height` stream with the given FourCC.
    pub fn new(fourcc: [u8; 4], width: u32, height: u32) -> Result<Self> {
        ensure!(width > 0 && height > 0 && width <= MAX_SIDE && height <= MAX_SIDE, "frame size {width}x{height} out of range");
        match &fourcc {
            b"Hap1" | b"Hap5" | b"HapY" | b"HapM" | b"HapA" => Ok(Self { width, height }),
            b"Hap7" => Err(Error::Unsupported("Hap R (BC7 textures)".into())),
            b"HapH" => Err(Error::Unsupported("Hap HDR (BC6H textures)".into())),
            _ => Err(Error::Unsupported(format!("FourCC {:?} is not Hap", String::from_utf8_lossy(&fourcc)))),
        }
    }

    /// Bytes of a texture of this frame size in `tex`.
    fn texture_len(&self, tex: Texture) -> usize {
        let bw = self.width.div_ceil(4) as usize;
        let bh = self.height.div_ceil(4) as usize;
        bw * bh * tex.block_size()
    }

    /// Decode one frame.
    pub fn decode(&mut self, frame: &[u8]) -> Result<Picture> {
        let (top, _) = section(frame)?;
        let images: Vec<(Texture, Vec<u8>)> = if top.kind == 0x0D {
            let inner = sections(top.data)?;
            ensure!(!inner.is_empty() && inner.len() <= 2, "multiple-image section with {} images", inner.len());
            inner.iter().map(|s| self.image(s)).collect::<Result<_>>()?
        } else {
            vec![self.image(&top)?]
        };
        let n = (self.width as usize) * (self.height as usize);
        let mut rgba = vec![0u8; n * 4];
        let colour = images.iter().any(|(t, _)| *t != Texture::Alpha);
        let mut alpha = false;
        for (tex, data) in &images {
            match tex {
                // an alpha-only stream is a grey matte; with a colour image it is the alpha
                Texture::Alpha if colour => {
                    self.paint(*tex, data, &mut rgba, |px, t| px[3] = t[3]);
                    alpha = true;
                }
                Texture::Alpha => self.paint(*tex, data, &mut rgba, |px, t| px[..3].fill(t[3])),
                Texture::Rgb => self.paint(*tex, data, &mut rgba, |px, t| px[..3].copy_from_slice(&t[..3])),
                Texture::Rgba => {
                    self.paint(*tex, data, &mut rgba, |px, t| *px = *t);
                    alpha = true;
                }
                Texture::YCoCg => self.paint(*tex, data, &mut rgba, |px, t| px[..3].copy_from_slice(&ycocg(t))),
            }
        }
        if !alpha {
            rgba.iter_mut().skip(3).step_by(4).for_each(|a| *a = 255);
        }
        Ok(Picture { width: self.width, height: self.height, rgba, alpha })
    }

    /// Decompress one image section into its texture data.
    fn image(&self, s: &Section<'_>) -> Result<(Texture, Vec<u8>)> {
        let tex = match s.kind & 0x0F {
            0x0B => Texture::Rgb,
            0x0E => Texture::Rgba,
            0x0F => Texture::YCoCg,
            0x01 => Texture::Alpha,
            0x0C => return Err(Error::Unsupported("BC7 textures".into())),
            0x02 | 0x03 => return Err(Error::Unsupported("BC6H textures".into())),
            k => return Err(Error::Invalid(format!("unknown image section type 0x{:02X}", (s.kind & 0xF0) | k))),
        };
        let len = self.texture_len(tex);
        let data = match s.kind >> 4 {
            0x0A => s.data.get(..len).ok_or_else(|| Error::Invalid("texture shorter than the frame".into()))?.to_vec(),
            0x0B => snappy::decompress(s.data, len)?,
            0x0C => chunks(s.data, len)?,
            _ => return Err(Error::Invalid(format!("unknown image section type 0x{:02X}", s.kind))),
        };
        ensure!(data.len() == len, "texture is {} bytes, {len} expected", data.len());
        Ok((tex, data))
    }

    /// Decode every block of `data` and hand each visible texel to `put`.
    fn paint(&self, tex: Texture, data: &[u8], rgba: &mut [u8], put: impl Fn(&mut [u8; 4], &[u8; 4])) {
        let (w, h) = (self.width as usize, self.height as usize);
        let bw = w.div_ceil(4);
        for (i, block) in data.chunks_exact(tex.block_size()).enumerate() {
            let (bx, by) = ((i % bw) * 4, (i / bw) * 4);
            let texels: [[u8; 4]; 16] = match tex {
                Texture::Rgb => bc::bc1(&arr8(block), false),
                Texture::Alpha => bc::bc4(&arr8(block)).map(|a| [0, 0, 0, a]),
                Texture::Rgba | Texture::YCoCg => bc::bc3(&arr16(block)),
            };
            for (j, t) in texels.iter().enumerate() {
                let (x, y) = (bx + j % 4, by + j / 4);
                if x < w && y < h {
                    let o = (y * w + x) * 4;
                    if let Some(px) = rgba.get_mut(o..o + 4).and_then(|p| <&mut [u8; 4]>::try_from(p).ok()) {
                        put(px, t);
                    }
                }
            }
        }
    }
}

fn arr8(b: &[u8]) -> [u8; 8] {
    let mut a = [0u8; 8];
    a.copy_from_slice(b.get(..8).unwrap_or(&[0; 8]));
    a
}

fn arr16(b: &[u8]) -> [u8; 16] {
    let mut a = [0u8; 16];
    a.copy_from_slice(b.get(..16).unwrap_or(&[0; 16]));
    a
}

/// Scaled YCoCg (R = Co, G = Cg, B = (scale - 1) * 8, A = Y) to RGB.
fn ycocg(t: &[u8; 4]) -> [u8; 3] {
    let scale = i32::from(t[2] >> 3) + 1;
    let co = (i32::from(t[0]) - 128) / scale;
    let cg = (i32::from(t[1]) - 128) / scale;
    let y = i32::from(t[3]);
    let c = |v: i32| v.clamp(0, 255) as u8;
    [c(y + co - cg), c(y + cg), c(y - co - cg)]
}

/// Decode a section with decode instructions: the instructions container, then the frame data
/// split into chunks that are decompressed independently and concatenated.
fn chunks(data: &[u8], max: usize) -> Result<Vec<u8>> {
    let (instr, frame) = section(data)?;
    ensure!(instr.kind == 0x01, "decode instructions container expected, found section type 0x{:02X}", instr.kind);
    let (mut comp, mut sizes, mut offsets) = (None, None, None);
    for s in sections(instr.data)? {
        match s.kind {
            0x02 => comp = Some(s.data),
            0x03 => sizes = Some(u32s(s.data)?),
            0x04 => offsets = Some(u32s(s.data)?),
            _ => {} // unknown sections are skipped
        }
    }
    let sizes = sizes.ok_or_else(|| Error::Invalid("decode instructions without a chunk size table".into()))?;
    let comp = comp.ok_or_else(|| Error::Invalid("decode instructions without a compressor table".into()))?;
    ensure!(comp.len() == sizes.len(), "{} compressors for {} chunks", comp.len(), sizes.len());
    ensure!(sizes.len() <= MAX_CHUNKS, "{} chunks", sizes.len());
    if let Some(o) = &offsets {
        ensure!(o.len() == sizes.len(), "{} offsets for {} chunks", o.len(), sizes.len());
    }
    let mut out = Vec::with_capacity(max);
    let mut at = 0usize;
    for (i, (&size, &c)) in sizes.iter().zip(comp).enumerate() {
        let start = offsets.as_ref().and_then(|o| o.get(i).copied()).map_or(at, |o| o as usize);
        let end = start.checked_add(size as usize).ok_or_else(|| Error::Invalid("chunk size".into()))?;
        let chunk = frame.get(start..end).ok_or_else(|| Error::Invalid(format!("chunk {i} runs past the frame")))?;
        at = end;
        let room = max.saturating_sub(out.len());
        match c {
            0x0A => {
                ensure!(chunk.len() <= room, "chunks hold more than the texture");
                out.extend_from_slice(chunk);
            }
            0x0B => out.extend(snappy::decompress(chunk, room)?),
            _ => return Err(Error::Invalid(format!("unknown chunk compressor 0x{c:02X}"))),
        }
    }
    Ok(out)
}

fn u32s(b: &[u8]) -> Result<Vec<u32>> {
    ensure!(b.len().is_multiple_of(4), "table of {} bytes is not a list of 32-bit values", b.len());
    Ok(b.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr(kind: u8, len: usize) -> Vec<u8> {
        let mut v = (len as u32).to_le_bytes()[..3].to_vec();
        v.push(kind);
        v
    }

    #[test]
    fn section_headers() {
        let mut f = hdr(0xAB, 2);
        f.extend([1, 2, 9]);
        let (s, rest) = section(&f).unwrap();
        assert_eq!((s.kind, s.data, rest), (0xAB, &[1u8, 2][..], &[9u8][..]));
        // the long form
        let f = [0, 0, 0, 0xBE, 1, 0, 0, 0, 7];
        let (s, rest) = section(&f).unwrap();
        assert_eq!((s.kind, s.data, rest.len()), (0xBE, &[7u8][..], 0));
        assert!(section(&[5, 0, 0, 0xAB, 1]).is_err());
        assert!(section(&[0, 0, 0]).is_err());
    }

    /// A 6x2 frame (two blocks) of solid red in uncompressed BC1, then in chunks.
    #[test]
    fn plain_and_chunked_bc1() {
        let block = [0x00, 0xF8, 0x00, 0xF8, 0, 0, 0, 0];
        let tex: Vec<u8> = [block, block].concat();
        let mut f = hdr(0xAB, tex.len());
        f.extend(&tex);
        let mut d = Decoder::new(*b"Hap1", 6, 2).unwrap();
        let p = d.decode(&f).unwrap();
        assert_eq!(p.rgba.len(), 6 * 2 * 4);
        assert!(p.rgba.chunks(4).all(|px| px == [255, 0, 0, 255]));
        assert!(!p.alpha);
        // two chunks: one uncompressed, one Snappy literal, with an offset table
        let mut instr = hdr(0x02, 2);
        instr.extend([0x0A, 0x0B]);
        instr.extend(hdr(0x03, 8));
        instr.extend(8u32.to_le_bytes());
        instr.extend(10u32.to_le_bytes());
        instr.extend(hdr(0x04, 8));
        instr.extend(0u32.to_le_bytes());
        instr.extend(8u32.to_le_bytes());
        let mut body = hdr(0x01, instr.len());
        body.extend(instr);
        body.extend(block);
        body.extend([8, 7 << 2]);
        body.extend(block);
        let mut f = hdr(0xCB, body.len());
        f.extend(body);
        assert_eq!(d.decode(&f).unwrap(), p);
    }

    #[test]
    fn hap_m_and_hap_a() {
        // one 4x4 block: YCoCg grey (Co = Cg = 128, scale 1, Y = 100) and alpha 50
        let ycocg_block = [100, 100, 0, 0, 0, 0, 0, 0, 0x10, 0x84, 0x10, 0x84, 0, 0, 0, 0];
        let alpha_block = [50, 50, 0, 0, 0, 0, 0, 0];
        let mut inner = hdr(0xAF, 16);
        inner.extend(ycocg_block);
        inner.extend(hdr(0xA1, 8));
        inner.extend(alpha_block);
        let mut f = hdr(0x0D, inner.len());
        f.extend(inner);
        let p = Decoder::new(*b"HapM", 4, 4).unwrap().decode(&f).unwrap();
        assert!(p.alpha);
        assert_eq!(&p.rgba[..4], &[100, 100, 100, 50]);
        let mut f = hdr(0xA1, 8);
        f.extend(alpha_block);
        let p = Decoder::new(*b"HapA", 4, 4).unwrap().decode(&f).unwrap();
        assert_eq!(&p.rgba[..4], &[50, 50, 50, 255]);
        assert!(!p.alpha);
    }

    #[test]
    fn multiple_images_in_either_order() {
        let ycocg_block = [100, 100, 0, 0, 0, 0, 0, 0, 0x10, 0x84, 0x10, 0x84, 0, 0, 0, 0];
        let mut inner = hdr(0xA1, 8);
        inner.extend([50, 50, 0, 0, 0, 0, 0, 0]);
        inner.extend(hdr(0xAF, 16));
        inner.extend(ycocg_block);
        let mut f = hdr(0x0D, inner.len());
        f.extend(inner);
        let p = Decoder::new(*b"HapM", 3, 3).unwrap().decode(&f).unwrap();
        assert_eq!(p.rgba.len(), 3 * 3 * 4);
        assert!(p.rgba.chunks(4).all(|px| px == [100, 100, 100, 50]));
    }

    #[test]
    fn bad_input_is_an_error() {
        assert!(matches!(Decoder::new(*b"Hap7", 4, 4), Err(Error::Unsupported(_))));
        assert!(matches!(Decoder::new(*b"HapH", 4, 4), Err(Error::Unsupported(_))));
        assert!(Decoder::new(*b"Hap1", 0, 4).is_err());
        let mut d = Decoder::new(*b"Hap5", 8, 8).unwrap();
        for f in [&[][..], &[3, 0, 0, 0xAE, 1][..], &[0, 0, 0, 0xAE, 0xFF, 0xFF, 0xFF, 0xFF][..], &[1, 0, 0, 0x55, 0][..]] {
            assert!(d.decode(f).is_err(), "{f:?}");
        }
        // every truncation and byte flip of a valid chunked frame is handled
        let mut instr = hdr(0x02, 1);
        instr.push(0x0B);
        instr.extend(hdr(0x03, 4));
        instr.extend(3u32.to_le_bytes());
        let mut body = hdr(0x01, instr.len());
        body.extend(instr);
        body.extend([64, 0, 0]);
        let mut f = hdr(0xCE, body.len());
        f.extend(body);
        for n in 0..f.len() {
            let _ = d.decode(&f[..n]);
            for bit in 0..8 {
                let mut g = f.clone();
                g[n] ^= 1 << bit;
                let _ = d.decode(&g);
            }
        }
    }
}
