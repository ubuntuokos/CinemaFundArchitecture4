//! Microsoft Video 1: 4x4 blocks of one, two or eight colours (8-bit palette or RGB555), with
//! skip codes. Rows are stored bottom-up in the stream; the picture is returned top-down.

use crate::{Error, Palette, Picture, Result, check_size};

/// A Video 1 decoder for one stream.
#[derive(Clone)]
pub struct Decoder {
    width: usize,
    height: usize,
    /// 8-bit palette mode (otherwise RGB555).
    mode8: bool,
    pal: Palette,
    /// Palette indices (8-bit) or RGB555 words, top row first. Persists between packets.
    frame: Vec<u16>,
}

/// A cursor over a packet; every read is bounds-checked and a short packet ends the picture.
struct Cursor<'a> {
    s: &'a [u8],
    p: usize,
}

impl Cursor<'_> {
    fn bytes<const N: usize>(&mut self) -> Option<[u8; N]> {
        let r = <[u8; N]>::try_from(self.s.get(self.p..self.p.checked_add(N)?)?).ok()?;
        self.p += N;
        Some(r)
    }

    fn u16le(&mut self) -> Option<u16> {
        self.bytes::<2>().map(u16::from_le_bytes)
    }
}

impl Decoder {
    /// `bits_per_coded_sample` is the `BITMAPINFOHEADER` bit count (8: palette, anything else:
    /// 16-bit RGB555), `extra` the bytes after the header (the 256-entry palette of 8-bit streams).
    pub fn new(width: u32, height: u32, bits_per_coded_sample: u16, extra: &[u8]) -> Result<Self> {
        check_size(width, height)?;
        if width < 4 || height < 4 {
            return Err(Error::Invalid("frame size"));
        }
        let mode8 = bits_per_coded_sample == 8;
        // FFmpeg's AVI demuxer hands the decoder the palette entries the header has (up to 256,
        // the rest black)
        let pal = Palette::from_extra(if mode8 { extra } else { &[] });
        let (width, height) = (width as usize, height as usize);
        Ok(Self { width, height, mode8, pal, frame: vec![0; width * height] })
    }

    /// Forget the previous picture.
    pub fn reset(&mut self) {
        self.frame.fill(0);
    }

    /// Decode one packet on top of the previous picture. A packet that ends early leaves the rest
    /// of the picture as it was (as FFmpeg does).
    pub fn decode(&mut self, pkt: &[u8]) -> Result<Picture> {
        // FFmpeg discards a packet smaller than the minimum frame size
        if pkt.len() < (self.width / 4) * (self.height / 4) / 512 {
            return Err(Error::Invalid("packet too small"));
        }
        self.blocks(pkt);
        Ok(self.picture())
    }

    fn picture(&self) -> Picture {
        let mut rgba = Vec::with_capacity(self.frame.len() * 4);
        for &v in &self.frame {
            if self.mode8 {
                let [r, g, b] = self.pal.rgb(v as u8);
                rgba.extend_from_slice(&[r, g, b, 255]);
            } else {
                let (r, g, b) = ((v >> 10) & 31, (v >> 5) & 31, v & 31);
                rgba.extend_from_slice(&[expand5(r), expand5(g), expand5(b), 255]);
            }
        }
        Picture { width: self.width as u32, height: self.height as u32, rgba }
    }

    /// Write one 4x4 block at block column `bx`, block row `by` counted from the bottom (1-based,
    /// FFmpeg's `block_y`). `pick(px, py)` is the value of the pixel `px` across and `py` rows up
    /// from the block's bottom row, the order the stream lists them in.
    fn put(&mut self, bx: usize, by: usize, mut pick: impl FnMut(usize, usize) -> u16) {
        for py in 0..4 {
            let y = by * 4 - 1 - py;
            let row = y * self.width + bx * 4;
            for px in 0..4 {
                self.frame[row + px] = pick(px, py);
            }
        }
    }

    fn blocks(&mut self, s: &[u8]) {
        let (bw, bh) = (self.width / 4, self.height / 4);
        let mut c = Cursor { s, p: 0 };
        let mut skip = 0i32;
        let mut total = (bw * bh) as i64;
        for by in (1..=bh).rev() {
            for bx in 0..bw {
                if skip != 0 {
                    skip -= 1;
                    total -= 1;
                    continue;
                }
                let Some([a, b]) = c.bytes::<2>() else { return };
                let (a, b) = (u16::from(a), u16::from(b));
                if a == 0 && b == 0 && total == 0 {
                    return;
                } else if b & 0xFC == 0x84 {
                    // skip code, not counting the current block (a count of 0 makes -1: skip to the end)
                    skip = (i32::from(b) - 0x84) * 256 + i32::from(a) - 1;
                } else if self.mode8 {
                    if !self.block8(&mut c, bx, by, a, b) {
                        return;
                    }
                } else if !self.block16(&mut c, bx, by, a, b) {
                    return;
                }
                total -= 1;
            }
        }
    }

    /// One block of an 8-bit picture; `false` when the packet ends inside it.
    fn block8(&mut self, c: &mut Cursor<'_>, bx: usize, by: usize, a: u16, b: u16) -> bool {
        if b < 0x80 {
            // 2-colour encoding
            let flags = (b << 8) | a;
            let Some(colors) = c.bytes::<2>() else { return false };
            let mut f = flags;
            self.put(bx, by, |_, _| {
                let v = u16::from(colors[usize::from((f & 1) ^ 1)]);
                f >>= 1;
                v
            });
        } else if b >= 0x90 {
            // 8-colour encoding
            let flags = (b << 8) | a;
            let Some(colors) = c.bytes::<8>() else { return false };
            let mut f = flags;
            self.put(bx, by, |px, py| {
                let v = u16::from(colors[((py & 2) << 1) + (px & 2) + usize::from((f & 1) ^ 1)]);
                f >>= 1;
                v
            });
        } else {
            // 1-colour encoding
            self.put(bx, by, |_, _| a);
        }
        true
    }

    /// One block of an RGB555 picture; `false` when the packet ends inside it.
    fn block16(&mut self, c: &mut Cursor<'_>, bx: usize, by: usize, a: u16, b: u16) -> bool {
        if b < 0x80 {
            // 2- or 8-colour encoding, told apart by the top bit of the first colour
            let flags = (b << 8) | a;
            let Some(c0) = c.u16le() else { return false };
            let Some(c1) = c.u16le() else { return false };
            let mut f = flags;
            if c0 & 0x8000 != 0 {
                let mut colors = [c0, c1, 0, 0, 0, 0, 0, 0];
                for slot in colors.iter_mut().skip(2) {
                    let Some(v) = c.u16le() else { return false };
                    *slot = v;
                }
                self.put(bx, by, |px, py| {
                    let v = colors[((py & 2) << 1) + (px & 2) + usize::from((f & 1) ^ 1)];
                    f >>= 1;
                    v
                });
            } else {
                let colors = [c0, c1];
                self.put(bx, by, |_, _| {
                    let v = colors[usize::from((f & 1) ^ 1)];
                    f >>= 1;
                    v
                });
            }
        } else {
            // 1-colour encoding
            let v = (b << 8) | a;
            self.put(bx, by, |_, _| v);
        }
        true
    }
}

/// Five bits widened to eight by repeating the top bits into the low ones, as FFmpeg's RGB555 to
/// RGB24 conversion does.
fn expand5(v: u16) -> u8 {
    ((v << 3) | (v >> 2)) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_small_frames() {
        assert!(Decoder::new(3, 8, 16, &[]).is_err());
        assert!(Decoder::new(8, 3, 16, &[]).is_err());
        assert!(Decoder::new(0, 0, 16, &[]).is_err());
    }

    #[test]
    fn one_colour_blocks_fill_bottom_up() {
        // 8x4 picture, 8-bit: two 1-colour blocks (b in 0x88..0x8F), the left one first
        let mut pal = vec![0u8; 1024];
        pal[4..8].copy_from_slice(&[10, 20, 30, 0]); // index 1: B=10 G=20 R=30
        pal[8..12].copy_from_slice(&[1, 2, 3, 0]);
        let mut d = Decoder::new(8, 4, 8, &pal).expect("decoder");
        let p = d.decode(&[1, 0x88, 2, 0x88]).expect("picture");
        assert_eq!(&p.rgba[..4], &[30, 20, 10, 255]);
        assert_eq!(&p.rgba[4 * 4..4 * 4 + 4], &[3, 2, 1, 255]);
    }

    #[test]
    fn damaged_packets_do_not_panic() {
        let mut d = Decoder::new(16, 16, 16, &[]).expect("decoder");
        let mut x = 1u32;
        for len in 0..200 {
            let pkt: Vec<u8> = (0..len)
                .map(|_| {
                    x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    (x >> 16) as u8
                })
                .collect();
            let _ = d.decode(&pkt);
        }
        let mut d8 = Decoder::new(16, 16, 8, &[]).expect("decoder");
        for len in 0..200 {
            let pkt = vec![0x93u8; len];
            let _ = d8.decode(&pkt);
        }
    }
}
