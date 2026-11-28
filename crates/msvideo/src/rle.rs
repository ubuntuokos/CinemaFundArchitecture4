//! Microsoft RLE: the run-length coding of `BI_RLE8` and `BI_RLE4` (and the 24-bit variant FFmpeg
//! also decodes), plus the uncompressed pictures some files carry under the same codec tag.
//! Rows are stored bottom-up; the picture is returned top-down.

use crate::{Error, Palette, Picture, Result, check_size};

/// Rows are padded to this many bytes, as FFmpeg's frame buffers are: a run that overshoots the
/// visible width into the padding is accepted there and its pixels are dropped.
const ROW_ALIGN: usize = 64;

/// A Microsoft RLE decoder for one stream.
#[derive(Clone)]
pub struct Decoder {
    width: usize,
    height: usize,
    /// Bits per pixel of the stream: 1, 4, 8 or 24.
    bits: u16,
    pal: Palette,
    /// Bytes per row of `frame`.
    stride: usize,
    /// One byte per pixel (1, 4, 8 bits: palette index, or the bit) or BGR triples (24 bits),
    /// top row first. Persists between packets.
    frame: Vec<u8>,
}

/// A byte reader that, like FFmpeg's `GetByteContext`, yields zeros past the end.
struct Gb<'a> {
    s: &'a [u8],
    p: usize,
}

impl Gb<'_> {
    fn left(&self) -> usize {
        self.s.len() - self.p
    }

    fn get(&mut self) -> usize {
        let v = self.s.get(self.p).copied().unwrap_or(0);
        self.p = (self.p + 1).min(self.s.len());
        usize::from(v)
    }

    fn be16(&mut self) -> usize {
        let hi = self.get();
        (hi << 8) | self.get()
    }

    fn skip(&mut self, n: usize) {
        self.p = self.p.saturating_add(n).min(self.s.len());
    }

    /// The next `n` bytes (the caller has checked that they exist).
    fn take(&mut self, n: usize) -> &[u8] {
        let r = self.s.get(self.p..self.p + n).unwrap_or(&[]);
        self.p += r.len();
        r
    }
}

impl Decoder {
    /// `bits_per_coded_sample` is the `BITMAPINFOHEADER` bit count (1, 4, 8 or 24), `extra` the
    /// bytes after the header: the palette, four bytes (blue first) per entry.
    pub fn new(width: u32, height: u32, bits_per_coded_sample: u16, extra: &[u8]) -> Result<Self> {
        check_size(width, height)?;
        if !matches!(bits_per_coded_sample, 1 | 4 | 8 | 24) {
            return Err(Error::Unsupported(format!("{bits_per_coded_sample}-bit Microsoft RLE")));
        }
        let (width, height) = (width as usize, height as usize);
        let bytes = if bits_per_coded_sample == 24 { 3 } else { 1 };
        let stride = (width * bytes).div_ceil(ROW_ALIGN) * ROW_ALIGN;
        Ok(Self { width, height, bits: bits_per_coded_sample, pal: Palette::from_extra(extra), stride, frame: vec![0; stride * height] })
    }

    /// Forget the previous picture.
    pub fn reset(&mut self) {
        self.frame.fill(0);
    }

    /// Decode one packet on top of the previous picture. A damaged packet leaves the rest of the
    /// picture as it was; only a packet too short to hold an end-of-picture code is an error.
    pub fn decode(&mut self, pkt: &[u8]) -> Result<Picture> {
        if pkt.len() < 2 {
            return Err(Error::Invalid("packet too small"));
        }
        // FFmpeg: a packet of exactly the size of an uncompressed picture is one
        let istride = (self.width * usize::from(self.bits)).div_ceil(32) * 4;
        if self.height * istride == pkt.len() {
            self.uncompressed(pkt, istride);
        } else {
            let mut gb = Gb { s: pkt, p: 0 };
            match self.bits {
                4 => self.pal4(&mut gb),
                8 => self.rle_8_24(&mut gb, 1),
                24 => self.rle_8_24(&mut gb, 3),
                // FFmpeg has no RLE decoder for 1-bit pictures: they stay as they were
                _ => {}
            }
        }
        Ok(self.picture())
    }

    fn picture(&self) -> Picture {
        let mut rgba = Vec::with_capacity(self.width * self.height * 4);
        for row in self.frame.chunks_exact(self.stride).take(self.height) {
            match self.bits {
                // monochrome as FFmpeg's `monowhite`: 0 is white
                1 => row.iter().take(self.width).for_each(|&b| {
                    let v = if b == 0 { 255 } else { 0 };
                    rgba.extend_from_slice(&[v, v, v, 255]);
                }),
                24 => row.chunks_exact(3).take(self.width).for_each(|p| rgba.extend_from_slice(&[p[2], p[1], p[0], 255])),
                _ => row.iter().take(self.width).for_each(|&i| {
                    let [r, g, b] = self.pal.rgb(i);
                    rgba.extend_from_slice(&[r, g, b, 255]);
                }),
            }
        }
        Picture { width: self.width as u32, height: self.height as u32, rgba }
    }

    fn uncompressed(&mut self, pkt: &[u8], istride: usize) {
        for y in 0..self.height {
            // the file's first row is the bottom one
            let Some(src) = pkt.get((self.height - 1 - y) * istride..(self.height - y) * istride) else { return };
            let Some(dst) = self.frame.get_mut(y * self.stride..y * self.stride + self.stride) else { return };
            match self.bits {
                1 => {
                    for (x, d) in dst.iter_mut().take(self.width).enumerate() {
                        *d = (src[x / 8] >> (7 - x % 8)) & 1;
                    }
                }
                4 => {
                    for (x, d) in dst.iter_mut().take(self.width).enumerate() {
                        let b = src[x / 2];
                        *d = if x % 2 == 0 { b >> 4 } else { b & 0xF };
                    }
                }
                _ => {
                    let n = self.width * if self.bits == 24 { 3 } else { 1 };
                    dst[..n].copy_from_slice(&src[..n]);
                }
            }
        }
    }

    fn set(&mut self, x: usize, line: isize, v: u8) {
        if let Ok(y) = usize::try_from(line)
            && let Some(p) = self.frame.get_mut(y * self.stride + x)
        {
            *p = v;
        }
    }

    /// 4-bit runs and literals: two pixels per byte.
    fn pal4(&mut self, gb: &mut Gb<'_>) {
        let w = self.width;
        let mut line = self.height as isize - 1;
        let mut px = 0usize;
        while line >= 0 && px <= w {
            if gb.left() == 0 {
                return;
            }
            let code = gb.get();
            if code == 0 {
                match gb.get() {
                    0 => {
                        line -= 1;
                        px = 0;
                    }
                    1 => return,
                    2 => {
                        px += gb.get();
                        line -= gb.get() as isize;
                    }
                    n => {
                        // copy pixels from the stream
                        let odd = n & 1;
                        let bytes = n.div_ceil(2);
                        if px + 2 * bytes - odd > w || gb.left() < bytes {
                            return;
                        }
                        for i in 0..bytes {
                            if px >= w {
                                break;
                            }
                            let b = gb.get();
                            self.set(px, line, (b >> 4) as u8);
                            px += 1;
                            if i + 1 == bytes && odd == 1 || px >= w {
                                break;
                            }
                            self.set(px, line, (b & 0xF) as u8);
                            px += 1;
                        }
                        // an odd byte count is padded to a word
                        if bytes & 1 == 1 {
                            gb.skip(1);
                        }
                    }
                }
            } else {
                // a run of two alternating pixels
                if px + code > w + 1 {
                    return;
                }
                let b = gb.get();
                for i in 0..code {
                    if px >= w {
                        break;
                    }
                    self.set(px, line, if i & 1 == 0 { (b >> 4) as u8 } else { (b & 0xF) as u8 });
                    px += 1;
                }
            }
        }
    }

    /// 8- and 24-bit runs and literals; `bpp` is the bytes per pixel.
    fn rle_8_24(&mut self, gb: &mut Gb<'_>, bpp: usize) {
        let stride = self.stride;
        let width_px = stride / bpp;
        let mut line = self.height as isize - 1;
        let row = |line: isize| usize::try_from(line).map_or(0, |l| l * stride);
        let mut out = row(line);
        let mut out_end = out + stride;
        let mut pos = 0usize;
        while gb.left() > 0 {
            let mut p1 = gb.get();
            if p1 == 0 {
                // escape code
                let mut p2 = gb.get();
                match p2 {
                    0 => {
                        // end of line
                        line -= 1;
                        if line < 0 {
                            // the end of the picture is a 0, 1 escape after the last line; a
                            // damaged stream stops here either way
                            let _ = gb.be16();
                            return;
                        }
                        out = row(line);
                        out_end = out + stride;
                        pos = 0;
                        continue;
                    }
                    1 => return,
                    2 => {
                        // skip right and up
                        p1 = gb.get();
                        p2 = gb.get();
                        line -= p2 as isize;
                        pos += p1;
                        if line < 0 || pos >= width_px {
                            return;
                        }
                        out = row(line) + pos * bpp;
                        out_end = row(line) + stride;
                        continue;
                    }
                    _ => {}
                }
                // copy data
                let n = p2 * bpp;
                if out + n > out_end {
                    gb.skip(2 * bpp);
                    continue;
                } else if gb.left() < n {
                    return;
                }
                let src = gb.take(n);
                if let Some(d) = self.frame.get_mut(out..out + n) {
                    d.copy_from_slice(src);
                }
                out += n;
                // 8-bit literals are padded to a word, runs are not
                if bpp == 1 && p2 & 1 == 1 {
                    gb.skip(1);
                }
                pos += p2;
            } else {
                // a run of one pixel
                let n = p1 * bpp;
                if out + n > out_end {
                    continue;
                }
                let mut pix = [0u8; 3];
                for p in pix.iter_mut().take(bpp) {
                    *p = gb.get() as u8;
                }
                if let Some(d) = self.frame.get_mut(out..out + n) {
                    for c in d.chunks_exact_mut(bpp) {
                        c.copy_from_slice(&pix[..bpp]);
                    }
                }
                out += n;
                pos += p1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pal() -> Vec<u8> {
        (0..256u32).flat_map(|i| [i as u8, 255 - i as u8, (i * 7) as u8, 0]).collect()
    }

    #[test]
    fn rejects_unknown_depths() {
        assert!(Decoder::new(8, 8, 16, &[]).is_err());
        assert!(Decoder::new(8, 8, 32, &[]).is_err());
        assert!(Decoder::new(0, 8, 8, &[]).is_err());
        assert!(Decoder::new(8, 8, 8, &[]).is_ok());
    }

    #[test]
    fn eight_bit_runs_and_literals() {
        let mut d = Decoder::new(4, 2, 8, &pal()).expect("decoder");
        // bottom row (first in the stream): a run of 4 of index 5; top row: literal 1, 2, 3, 4; end
        let p = d.decode(&[4, 5, 0, 0, 0, 4, 1, 2, 3, 4, 0, 1]).expect("picture");
        let px = |i: usize| p.rgba[i * 4..i * 4 + 3].to_vec();
        // index i is [blue i, green 255 - i, red 7i, 0] in the file
        assert_eq!(px(0), vec![7, 254, 1]);
        assert_eq!(px(3), vec![28, 251, 4]);
        assert_eq!(px(4), vec![35, 250, 5]);
        assert_eq!(px(7), vec![35, 250, 5]);
    }

    #[test]
    fn damaged_packets_do_not_panic() {
        let mut x = 3u32;
        for bits in [1u16, 4, 8, 24] {
            let mut d = Decoder::new(21, 13, bits, &pal()).expect("decoder");
            for len in 0..300 {
                let pkt: Vec<u8> = (0..len)
                    .map(|_| {
                        x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                        (x >> 16) as u8
                    })
                    .collect();
                let _ = d.decode(&pkt);
            }
        }
    }
}
