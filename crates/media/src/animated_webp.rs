//! Animated WebP (RFC 9649 §2.7): the container and the canvas, written for CFA4 (#87).
//!
//! Each `ANMF` frame's bitstream (`VP8L`, or `VP8` with an optional `ALPH`) is wrapped as a WebP
//! still and decoded by the still decoder, which equals libwebp's `dwebp`. The frames are then
//! drawn on the canvas here, with libwebp's animation decoder (`anim_decode.c`, BSD-3-Clause) as
//! the oracle: the canvas starts transparent, a disposed frame clears its rectangle to transparent
//! (the background colour in `ANIM` is a hint, §2.7.1.1), and blending uses its integer formula,
//! so `anim_dump` and we give the same pixels. A frame that does not decode or does not fit the
//! canvas ends the animation.

use std::io::Cursor;

use image::{Delay, Frame, ImageFormat, ImageResult, RgbaImage};

/// Largest canvas side (the format allows 2^24).
pub const MAX_SIDE: u32 = 16_384;
/// Largest canvas in bytes of RGBA (8192 × 8192). Decoding keeps three canvases at a time.
pub const MAX_CANVAS_BYTES: u64 = 1 << 28;

/// The parts of the file the animation needs.
pub struct Animation<'a> {
    pub width: u32,
    pub height: u32,
    frames: Vec<&'a [u8]>,
}

#[derive(Clone, Copy)]
struct Rect {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

impl Rect {
    fn covers(self, width: u32, height: u32) -> bool {
        self.x == 0 && self.y == 0 && self.w == width && self.h == height
    }
}

fn u24(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(3)?)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], 0]))
}

/// The chunks of a RIFF body, as (fourcc, payload). Stops at the first chunk that runs past the
/// end (a truncated file keeps the chunks before it).
fn chunks(mut b: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    std::iter::from_fn(move || {
        let (head, rest) = b.split_first_chunk::<8>()?;
        let (fourcc, size) = head.split_first_chunk::<4>()?;
        let size = usize::try_from(u32::from_le_bytes(*size.first_chunk::<4>()?)).ok()?;
        let payload = rest.get(..size)?;
        // payloads are padded to an even length
        b = rest.get(size.saturating_add(size & 1)..).unwrap_or(&[]);
        Some((*fourcc, payload))
    })
}

/// An animated WebP's canvas and frames. `None` when `bytes` is not an animated WebP.
pub fn parse(bytes: &[u8]) -> Option<Animation<'_>> {
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WEBP") {
        return None;
    }
    let riff_size = usize::try_from(u32::from_le_bytes(*bytes.get(4..8)?.first_chunk::<4>()?)).ok()?;
    let body = bytes.get(12..riff_size.saturating_add(8).min(bytes.len()))?;
    let mut it = chunks(body);
    let (fourcc, vp8x) = it.next()?;
    // flags: bit 1 is "animation"
    if &fourcc != b"VP8X" || vp8x.first()? & 0x02 == 0 {
        return None;
    }
    let width = u24(vp8x, 4)?.checked_add(1)?;
    let height = u24(vp8x, 7)?.checked_add(1)?;
    let frames = it.filter(|(f, _)| f == b"ANMF").map(|(_, p)| p).collect();
    Some(Animation { width, height, frames })
}

impl Animation<'_> {
    /// Whether the canvas is small enough to decode (at most [`MAX_SIDE`] wide and high and
    /// [`MAX_CANVAS_BYTES`] large).
    pub fn fits(&self) -> bool {
        let bytes = u64::from(self.width).saturating_mul(u64::from(self.height)).saturating_mul(4);
        self.width <= MAX_SIDE && self.height <= MAX_SIDE && bytes <= MAX_CANVAS_BYTES
    }

    /// The canvas after each frame, with the frame's duration in milliseconds.
    pub fn frames(self) -> impl Iterator<Item = ImageResult<Frame>> {
        let (w, h) = (self.width, self.height);
        let mut canvas = Canvas::new(w, h);
        let mut frames = self.frames.into_iter();
        let mut failed = false;
        std::iter::from_fn(move || {
            if failed {
                return None;
            }
            let anmf = frames.next()?;
            let drawn = canvas.draw(anmf).and_then(|delay| {
                let picture = RgbaImage::from_raw(w, h, canvas.pixels.clone()).ok_or_else(|| error("canvas size"))?;
                Ok(Frame::from_parts(picture, 0, 0, Delay::from_numer_denom_ms(delay, 1)))
            });
            failed = drawn.is_err();
            Some(drawn)
        })
    }
}

fn error(what: &str) -> image::ImageError {
    image::ImageError::Decoding(image::error::DecodingError::new(ImageFormat::WebP.into(), format!("animated WebP: {what}")))
}

/// libwebp's animation canvas: the current picture and the previous one after its disposal.
struct Canvas {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    disposed: Vec<u8>,
    previous: Option<Previous>,
}

#[derive(Clone, Copy)]
struct Previous {
    rect: Rect,
    dispose: bool,
    key: bool,
}

impl Canvas {
    fn new(width: u32, height: u32) -> Self {
        // `fits` has bounded the size; a canvas that cannot be indexed stays empty and every frame
        // then fails to fit
        let len = usize::try_from(u64::from(width) * u64::from(height) * 4).unwrap_or(0);
        Self { width, height, pixels: vec![0; len], disposed: vec![0; len], previous: None }
    }

    /// Draw one `ANMF` payload; its duration in milliseconds.
    fn draw(&mut self, anmf: &[u8]) -> ImageResult<u32> {
        let field = |at| u24(anmf, at).ok_or_else(|| error("short ANMF chunk"));
        let rect = Rect { x: field(0)?.saturating_mul(2), y: field(3)?.saturating_mul(2), w: field(6)?.saturating_add(1), h: field(9)?.saturating_add(1) };
        let delay = field(12)?;
        let flags = *anmf.get(15).ok_or_else(|| error("short ANMF chunk"))?;
        let (blend, dispose) = (flags & 0x02 == 0, flags & 0x01 != 0);
        if rect.x.saturating_add(rect.w) > self.width || rect.y.saturating_add(rect.h) > self.height {
            return Err(error("frame outside the canvas"));
        }
        let frame = decode_frame(anmf.get(16..).unwrap_or(&[]), rect)?;
        let has_alpha = frame.as_chunks::<4>().0.iter().any(|p| p[3] != 255);

        // libwebp's key frame: drawn on a transparent canvas, without blending
        let full = rect.covers(self.width, self.height);
        let key = match self.previous {
            None => true,
            Some(_) if (!has_alpha || !blend) && full => true,
            Some(p) => p.dispose && (p.rect.covers(self.width, self.height) || p.key),
        };
        if key {
            self.pixels.fill(0);
        } else {
            self.pixels.copy_from_slice(&self.disposed);
        }
        let stride = self.width as usize * 4;
        let row = rect.w as usize * 4;
        for (y, src) in frame.chunks_exact(row).enumerate() {
            let at = (rect.y as usize + y) * stride + rect.x as usize * 4;
            let dst = self.pixels.get_mut(at..at + row).ok_or_else(|| error("frame outside the canvas"))?;
            dst.copy_from_slice(src);
        }
        if blend && !key {
            // blend the decoded frame over the previous canvas, except where that was disposed
            // (transparent there, so the frame stays as decoded)
            let cleared = self.previous.filter(|p| p.dispose).map(|p| p.rect);
            for y in rect.y..rect.y + rect.h {
                for x in rect.x..rect.x + rect.w {
                    if cleared.is_some_and(|c| x >= c.x && x < c.x + c.w && y >= c.y && y < c.y + c.h) {
                        continue;
                    }
                    let i = y as usize * stride + x as usize * 4;
                    let (Some(src), Some(dst)) = (self.pixels.get(i..i + 4), self.disposed.get(i..i + 4)) else { continue };
                    let blended = blend_pixel(*src.first_chunk::<4>().unwrap_or(&[0; 4]), *dst.first_chunk::<4>().unwrap_or(&[0; 4]));
                    if let Some(p) = self.pixels.get_mut(i..i + 4) {
                        p.copy_from_slice(&blended);
                    }
                }
            }
        }

        self.disposed.copy_from_slice(&self.pixels);
        if dispose {
            for y in rect.y..rect.y + rect.h {
                let at = y as usize * stride + rect.x as usize * 4;
                if let Some(r) = self.disposed.get_mut(at..at + row) {
                    r.fill(0);
                }
            }
        }
        self.previous = Some(Previous { rect, dispose, key });
        Ok(delay)
    }
}

/// `src` over `dst`, both straight alpha, with libwebp's integer arithmetic (`BlendPixelNonPremult`).
fn blend_pixel(src: [u8; 4], dst: [u8; 4]) -> [u8; 4] {
    let src_a = u32::from(src[3]);
    if src_a == 255 {
        return src;
    }
    if src_a == 0 {
        return dst;
    }
    let dst_a = (u32::from(dst[3]) * (256 - src_a)) >> 8;
    let blend_a = src_a + dst_a;
    let scale = (1u32 << 24) / blend_a;
    let channel = |c: usize| (((u32::from(src[c]) * src_a + u32::from(dst[c]) * dst_a) * scale) >> 24) as u8;
    [channel(0), channel(1), channel(2), blend_a as u8]
}

/// Decode an `ANMF` frame's bitstream chunks as a `rect`-sized RGBA picture.
fn decode_frame(data: &[u8], rect: Rect) -> ImageResult<Vec<u8>> {
    let mut alph = None;
    let mut image = None;
    for (fourcc, payload) in chunks(data) {
        match &fourcc {
            b"ALPH" if alph.is_none() && image.is_none() => alph = Some(payload),
            b"VP8 " | b"VP8L" => {
                image = Some((fourcc, payload));
                break;
            }
            _ => {}
        }
    }
    let (fourcc, payload) = image.ok_or_else(|| error("frame without a bitstream"))?;
    let mut riff = Vec::with_capacity(payload.len().saturating_add(64));
    let chunk = |riff: &mut Vec<u8>, fourcc: &[u8; 4], payload: &[u8]| -> ImageResult<()> {
        let size = u32::try_from(payload.len()).map_err(|_| error("chunk too large"))?;
        riff.extend_from_slice(fourcc);
        riff.extend_from_slice(&size.to_le_bytes());
        riff.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            riff.push(0);
        }
        Ok(())
    };
    riff.extend_from_slice(b"RIFF\0\0\0\0WEBP");
    match alph {
        // the alpha of a lossy frame needs the extended format around it
        Some(alpha) if &fourcc == b"VP8 " => {
            let (w, h) = (rect.w.saturating_sub(1), rect.h.saturating_sub(1));
            let mut vp8x = [0u8; 10];
            vp8x[0] = 0x10;
            vp8x[4..7].copy_from_slice(&w.to_le_bytes()[..3]);
            vp8x[7..10].copy_from_slice(&h.to_le_bytes()[..3]);
            chunk(&mut riff, b"VP8X", &vp8x)?;
            chunk(&mut riff, b"ALPH", alpha)?;
        }
        _ => {}
    }
    chunk(&mut riff, &fourcc, payload)?;
    let size = u32::try_from(riff.len() - 8).map_err(|_| error("frame too large"))?;
    if let Some(s) = riff.get_mut(4..8) {
        s.copy_from_slice(&size.to_le_bytes());
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(riff), ImageFormat::WebP);
    reader.limits(image::Limits::default());
    let picture = reader.decode()?.into_rgba8();
    if picture.dimensions() != (rect.w, rect.h) {
        return Err(error("frame size differs from its bitstream"));
    }
    Ok(picture.into_raw())
}

#[cfg(test)]
#[path = "animated_webp_tests.rs"]
mod tests;
