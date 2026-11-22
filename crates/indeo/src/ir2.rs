//! Indeo 2 (`RT21`), a Rust re-expression of FFmpeg n7.1.1 `libavcodec/indeo2.c`
//! (LGPL-2.1-or-later). YUV 4:1:0; intra frames carry absolute first lines and vertical deltas,
//! inter frames add scaled deltas to the previous picture.

use crate::ir2data::{IR2_DELTA, IR2_TAB};
use crate::{Error, Picture, PixelFormat};

const CODE_BITS: u32 = 14;
const IR2_CODES: usize = 143;

/// The code lookup: 14 bits of stream (first bit read is the most significant) to (symbol, length).
fn build_table() -> Vec<(u8, u8)> {
    let mut t = vec![(0u8, 0u8); 1 << CODE_BITS];
    let mut code: u64 = 0;
    for &(sym, len) in IR2_TAB.iter() {
        let hi = (code >> (32 - u32::from(len))) as usize;
        let shift = CODE_BITS - u32::from(len);
        for low in 0..1usize << shift {
            t[(hi << shift) | low] = (sym, len);
        }
        code += 1u64 << (32 - u32::from(len));
    }
    t
}

/// A least-significant-bit-first reader (the stream's bit order); reads past the end give zeros.
struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn left(&self) -> i64 {
        self.d.len() as i64 * 8 - self.pos as i64
    }
    /// The next `CODE_BITS` bits as a code (first bit most significant).
    fn peek(&self) -> usize {
        let mut v = 0usize;
        for i in 0..CODE_BITS as usize {
            let p = self.pos + i;
            let bit = self.d.get(p >> 3).map_or(0, |b| (b >> (p & 7)) & 1);
            v = v << 1 | usize::from(bit);
        }
        v
    }
}

pub struct Decoder {
    width: usize,
    height: usize,
    table: Vec<(u8, u8)>,
    planes: [Vec<u8>; 3],
}

impl Decoder {
    pub fn new(width: u32, height: u32) -> Result<Self, Error> {
        if width == 0 || height == 0 || width > 16384 || height > 16384 {
            return Err(Error::Size(width, height));
        }
        let (w, h) = (width as usize, height as usize);
        let (cw, ch) = (w.div_ceil(4), h.div_ceil(4));
        Ok(Self { width: w, height: h, table: build_table(), planes: [vec![0; w * h], vec![0; cw * ch], vec![0; cw * ch]] })
    }

    fn code(&self, b: &mut Bits<'_>) -> i32 {
        let (sym, len) = self.table[b.peek()];
        if len == 0 {
            return -1;
        }
        b.pos += usize::from(len);
        i32::from(sym)
    }

    fn plane(&self, b: &mut Bits<'_>, dst: &mut [u8], pitch: usize, w: usize, h: usize, table: &[u8; 256], intra: bool) -> Result<(), Error> {
        if w & 1 != 0 {
            return Err(Error::Invalid);
        }
        if intra {
            if (w * h / (2 * (IR2_CODES - 0x7F))) as i64 > b.left() {
                return Err(Error::Invalid);
            }
            let mut out = 0;
            while out < w {
                let c = self.code(b);
                if c >= 0x80 {
                    let n = (c - 0x7F) as usize * 2;
                    if out + n > w {
                        return Err(Error::Invalid);
                    }
                    dst[out..out + n].fill(0x80);
                    out += n;
                } else {
                    if c <= 0 {
                        return Err(Error::Invalid);
                    }
                    dst[out] = table[c as usize * 2];
                    dst[out + 1] = table[c as usize * 2 + 1];
                    out += 2;
                }
            }
            for j in 1..h {
                let mut out = 0;
                while out < w {
                    if b.left() <= 0 {
                        return Err(Error::Invalid);
                    }
                    let c = self.code(b);
                    let (row, prev) = (j * pitch, (j - 1) * pitch);
                    if c >= 0x80 {
                        let n = (c - 0x7F) as usize * 2;
                        if out + n > w {
                            return Err(Error::Invalid);
                        }
                        for _ in 0..n {
                            dst[row + out] = dst[prev + out];
                            out += 1;
                        }
                    } else {
                        if c <= 0 {
                            return Err(Error::Invalid);
                        }
                        for k in 0..2 {
                            let t = i32::from(dst[prev + out]) + (i32::from(table[c as usize * 2 + k]) - 128);
                            dst[row + out] = t.clamp(0, 255) as u8;
                            out += 1;
                        }
                    }
                }
            }
        } else {
            for j in 0..h {
                let mut out = 0;
                while out < w {
                    if b.left() <= 0 {
                        return Err(Error::Invalid);
                    }
                    let c = self.code(b);
                    if c >= 0x80 {
                        out += (c - 0x7F) as usize * 2;
                    } else {
                        if c <= 0 {
                            return Err(Error::Invalid);
                        }
                        for k in 0..2 {
                            let t = i32::from(dst[j * pitch + out]) + (((i32::from(table[c as usize * 2 + k]) - 128) * 3) >> 2);
                            dst[j * pitch + out] = t.clamp(0, 255) as u8;
                            out += 1;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Decode one frame. The picture is returned even when decoding stopped early (`Err` then
    /// leaves the planes partly updated, as ffmpeg does, but returns no picture).
    pub fn decode(&mut self, data: &[u8]) -> Result<Picture, Error> {
        if data.len() <= 48 {
            return Err(Error::Invalid);
        }
        let intra = data[18] != 0;
        let (ltab, ctab) = (usize::from(data[0x22] & 3), usize::from(data[0x22] >> 2));
        if ctab > 3 {
            return Err(Error::Invalid);
        }
        let mut b = Bits { d: &data[48..], pos: 0 };
        let (w, h) = (self.width, self.height);
        let cw = w.div_ceil(4);
        let mut planes = std::mem::take(&mut self.planes);
        // order: Y, then V (plane 2), then U (plane 1): U and V are swapped in the stream
        let r = (|| {
            self.plane(&mut b, &mut planes[0], w, w, h, &IR2_DELTA[ltab], intra)?;
            self.plane(&mut b, &mut planes[2], cw, w >> 2, h >> 2, &IR2_DELTA[ctab], intra)?;
            self.plane(&mut b, &mut planes[1], cw, w >> 2, h >> 2, &IR2_DELTA[ctab], intra)
        })();
        self.planes = planes;
        r?;
        Ok(Picture { width: self.width as u32, height: self.height as u32, format: PixelFormat::Yuv410, planes: self.planes.to_vec(), ..Default::default() })
    }
}
