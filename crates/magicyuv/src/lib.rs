// Ported from FFmpeg n7.1.1 libavcodec/magicyuv.c (Copyright (c) 2016 Paul B Mahol), with the
// canonical code assignment of libavcodec/vlc.c (ff_vlc_init_from_lengths) and the left,
// gradient and median predictors of libavcodec/lossless_videodsp.c, LGPL-2.1-or-later; see
// docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! Pure-Rust decoder for **MagicYUV** (`MAGY`), the lossless VfW video codec: version 7 frames
//! of every format ffmpeg knows (8-bit RGB / RGBA / YUV 4:4:4 / 4:2:2 / 4:2:0 / YUVA 4:4:4 /
//! grey, 10-bit YUV and grey, 10- / 12- / 14-bit RGB and RGBA), Huffman-coded or raw slices,
//! left, gradient and median prediction, progressive and interlaced. The output is the same as
//! ffmpeg's `magicyuv` decoder, sample for sample.

/// Errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("invalid MagicYUV frame: {0}")]
    Invalid(&'static str),
    #[error("unsupported MagicYUV frame: {0}")]
    Unsupported(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

/// The largest frame decoded, in pixels (8K UHD fits).
pub const MAX_PIXELS: u64 = 1 << 26;

/// The layout of a decoded picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Planes R, G, B.
    Rgb,
    /// Planes R, G, B, A.
    Rgba,
    /// Planes Y, U, V.
    Yuv444,
    /// Planes Y, U, V; chroma half width.
    Yuv422,
    /// Planes Y, U, V; chroma half width and half height.
    Yuv420,
    /// Planes Y, U, V, A.
    Yuva444,
    /// Plane Y.
    Gray,
}

impl Format {
    /// (horizontal, vertical) chroma shift.
    fn shifts(self) -> (u32, u32) {
        match self {
            Format::Yuv422 => (1, 0),
            Format::Yuv420 => (1, 1),
            _ => (0, 0),
        }
    }
    fn planes(self) -> usize {
        match self {
            Format::Gray => 1,
            Format::Rgba | Format::Yuva444 => 4,
            _ => 3,
        }
    }
}

/// The colour matrix a YUV frame names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Matrix {
    Bt601,
    Bt709,
}

/// A decoded frame: planes of `bits`-bit samples, top row first, each `plane_size(i)` large.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub format: Format,
    pub bits: u32,
    pub planes: Vec<Vec<u16>>,
    /// The matrix (YUV frames that name one).
    pub matrix: Option<Matrix>,
    /// Full range (YUV frames).
    pub full_range: bool,
    pub interlaced: bool,
}

impl Picture {
    /// The width and height of plane `i`.
    pub fn plane_size(&self, i: usize) -> (u32, u32) {
        plane_size(self.format, self.width, self.height, i)
    }
}

fn plane_size(format: Format, w: u32, h: u32, i: usize) -> (u32, u32) {
    let (hs, vs) = format.shifts();
    if i == 1 || i == 2 { (ceil_shift(w, hs), ceil_shift(h, vs)) } else { (w, h) }
}

fn ceil_shift(v: u32, s: u32) -> u32 {
    v.div_ceil(1 << s)
}

fn byte(d: &[u8], i: usize) -> u8 {
    d.get(i).copied().unwrap_or(0)
}

fn le32(d: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([byte(d, i), byte(d, i + 1), byte(d, i + 2), byte(d, i + 3)])
}

fn mid_pred(a: i32, b: i32, c: i32) -> i32 {
    a.max(b).min(a.min(b).max(c))
}

/// An MSB-first bit reader over a slice (zeros past the end).
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn left(&self) -> isize {
        (self.data.len() * 8) as isize - self.pos as isize
    }
    fn peek32(&self) -> u32 {
        let (b, s) = (self.pos / 8, self.pos % 8);
        let w =
            u64::from_be_bytes([byte(self.data, b), byte(self.data, b + 1), byte(self.data, b + 2), byte(self.data, b + 3), byte(self.data, b + 4), 0, 0, 0]);
        ((w << s) >> 32) as u32
    }
    fn read(&mut self, n: u32) -> u32 {
        let v = if n == 0 { 0 } else { self.peek32() >> (32 - n) };
        self.pos += n as usize;
        v
    }
}

/// A canonical Huffman code as FFmpeg builds it from code lengths: the longest codes first,
/// equal lengths by symbol, codes counting up from zero.
struct Huffman {
    /// (left-aligned 32-bit code, length, symbol), by code.
    codes: Vec<(u64, u8, u16)>,
}

impl Huffman {
    fn new(lens: &[u8]) -> Result<Huffman> {
        let mut order: Vec<(u8, u16)> = lens.iter().enumerate().map(|(s, &l)| (l, s as u16)).collect();
        order.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let max_len = u32::from(order.first().map_or(0, |e| e.0));
        let len_max = 32.min(3 * max_len.min(12));
        let mut code: u64 = 0;
        let mut codes = Vec::with_capacity(order.len());
        for (len, sym) in order {
            let len = u32::from(len);
            if len > len_max || code & ((1u64 << (32 - len)) - 1) != 0 {
                return Err(Error::Invalid("Huffman code"));
            }
            codes.push((code, len as u8, sym));
            code += 1u64 << (32 - len);
            if code > 1u64 << 32 {
                return Err(Error::Invalid("Huffman code oversubscribed"));
            }
        }
        Ok(Huffman { codes })
    }

    fn decode(&self, br: &mut Bits<'_>) -> Result<u16> {
        let peek = u64::from(br.peek32());
        let i = self.codes.partition_point(|c| c.0 <= peek);
        let &(code, len, sym) = i.checked_sub(1).and_then(|i| self.codes.get(i)).ok_or(Error::Invalid("Huffman code"))?;
        if peek >= code + (1u64 << (32 - u32::from(len))) {
            return Err(Error::Invalid("Huffman code"));
        }
        br.pos += usize::from(len);
        Ok(sym)
    }
}

/// The decoder of a MagicYUV stream (every frame stands alone).
#[derive(Clone, Debug, Default)]
pub struct Decoder;

impl Decoder {
    pub fn new() -> Decoder {
        Decoder
    }

    /// Decode one frame.
    pub fn decode(&mut self, pkt: &[u8]) -> Result<Picture> {
        let size = pkt.len();
        if size < 36 || pkt.get(..4) != Some(b"MAGY") {
            return Err(Error::Invalid("no MAGY header"));
        }
        let header_size = le32(pkt, 4) as usize;
        if header_size < 32 || header_size >= size {
            return Err(Error::Invalid("header size"));
        }
        if pkt[8] != 7 {
            return Err(Error::Unsupported("version other than 7"));
        }
        let (format, bits, decorrelate) = match pkt[9] {
            0x65 => (Format::Rgb, 8, true),
            0x66 => (Format::Rgba, 8, true),
            0x67 => (Format::Yuv444, 8, false),
            0x68 => (Format::Yuv422, 8, false),
            0x69 => (Format::Yuv420, 8, false),
            0x6a => (Format::Yuva444, 8, false),
            0x6b => (Format::Gray, 8, false),
            0x6c => (Format::Yuv422, 10, false),
            0x76 => (Format::Yuv444, 10, false),
            0x6d => (Format::Rgb, 10, true),
            0x6e => (Format::Rgba, 10, true),
            0x6f => (Format::Rgb, 12, true),
            0x70 => (Format::Rgba, 12, true),
            0x71 => (Format::Rgb, 14, true),
            0x72 => (Format::Rgba, 14, true),
            0x73 => (Format::Gray, 10, false),
            0x7b => (Format::Yuv420, 10, false),
            _ => return Err(Error::Unsupported("format")),
        };
        let max = 1usize << bits;
        let planes = format.planes();
        let color_matrix = pkt[11];
        let flags = pkt[12];
        let interlaced = flags & 2 != 0;
        let width = le32(pkt, 16);
        let height = le32(pkt, 20);
        if width == 0 || height == 0 || width > 32768 || height > 32768 || u64::from(width) * u64::from(height) > MAX_PIXELS {
            return Err(Error::Invalid("frame size"));
        }
        if le32(pkt, 24) != width {
            return Err(Error::Unsupported("slice width other than the frame width"));
        }
        let slice_height = le32(pkt, 28);
        if slice_height == 0 || slice_height > i32::MAX as u32 - height {
            return Err(Error::Invalid("slice height"));
        }
        let nb_slices = height.div_ceil(slice_height) as usize;
        let vshift1 = format.shifts().1;
        if interlaced && ((slice_height >> vshift1) < 2 || (!height.is_multiple_of(slice_height) && ((height % slice_height) >> vshift1) < 2)) {
            return Err(Error::Invalid("interlaced slice height"));
        }
        let mut pos = 36usize;
        if size.saturating_sub(pos) <= nb_slices * planes * 5 {
            return Err(Error::Invalid("slice table too short"));
        }
        // slice positions per plane: (start, size)
        let mut slices = vec![Vec::with_capacity(nb_slices); planes];
        let mut first_offset = 0usize;
        let limit = size - header_size;
        for (i, sl) in slices.iter_mut().enumerate() {
            let mut offset = le32(pkt, pos) as usize;
            pos += 4;
            if offset >= limit {
                return Err(Error::Invalid("slice offset"));
            }
            if i == 0 {
                first_offset = offset;
            }
            for _ in 0..nb_slices - 1 {
                let next = le32(pkt, pos) as usize;
                pos += 4;
                if next <= offset || next >= limit || next - offset < 2 {
                    return Err(Error::Invalid("slice offset"));
                }
                sl.push((offset + header_size, next - offset));
                offset = next;
            }
            let start = offset + header_size;
            if size - start < 2 {
                return Err(Error::Invalid("slice size"));
            }
            sl.push((start, size - start));
        }
        if usize::from(byte(pkt, pos)) != planes {
            return Err(Error::Invalid("plane count"));
        }
        pos += 1 + nb_slices * planes;
        let table_end = header_size + first_offset;
        if table_end < pos + 2 {
            return Err(Error::Invalid("Huffman tables too short"));
        }
        let tables = build_huffman(pkt.get(pos..table_end).unwrap_or_default(), max, planes)?;

        let (w, h) = (width as usize, height as usize);
        let mut out: Vec<Vec<u16>> = (0..planes)
            .map(|i| {
                let (pw, ph) = plane_size(format, width, height, i);
                vec![0u16; pw as usize * ph as usize]
            })
            .collect();
        let mask = (max - 1) as u32;
        for j in 0..nb_slices {
            for (i, plane) in out.iter_mut().enumerate() {
                let (hs, vs) = if i == 1 || i == 2 { format.shifts() } else { (0, 0) };
                let rows_in = (slice_height as usize).min(h - j * slice_height as usize);
                let ph = ceil_shift(rows_in as u32, vs) as usize;
                let pw = ceil_shift(width, hs) as usize;
                let sheight = ceil_shift(slice_height, vs) as usize;
                let first = j * sheight;
                let (start, len) = slices[i][j];
                let data = pkt.get(start..start + len).unwrap_or_default();
                // with an odd slice height, 4:2:0 chroma slices can reach past the plane (ffmpeg
                // writes them into its buffer padding): those rows are decoded and dropped
                let mut region = vec![0u16; pw * ph];
                decode_slice(&mut region, pw, ph, data, bits, &tables[i], interlaced, mask)?;
                let start = (first * pw).min(plane.len());
                let end = ((first + ph) * pw).min(plane.len());
                plane[start..end].copy_from_slice(&region[..end - start]);
            }
            if decorrelate {
                // coded planes: B - G, G, R - G
                let rows_in = (slice_height as usize).min(h - j * slice_height as usize);
                let range = j * slice_height as usize * w..(j * slice_height as usize + rows_in) * w;
                let (b, rest) = out.split_at_mut(1);
                let (g, rest) = rest.split_at_mut(1);
                let r = &mut rest[0];
                for k in range {
                    let gv = g[0][k];
                    b[0][k] = ((u32::from(b[0][k]) + u32::from(gv)) & mask) as u16;
                    r[k] = ((u32::from(r[k]) + u32::from(gv)) & mask) as u16;
                }
            }
        }
        let rgb = matches!(format, Format::Rgb | Format::Rgba);
        let out = if rgb {
            let mut it = out.into_iter();
            let (b, g, r) = (it.next().unwrap_or_default(), it.next().unwrap_or_default(), it.next().unwrap_or_default());
            let mut v = vec![r, g, b];
            v.extend(it);
            v
        } else {
            out
        };
        let matrix = match color_matrix {
            1 if !rgb => Some(Matrix::Bt601),
            2 if !rgb => Some(Matrix::Bt709),
            _ => None,
        };
        Ok(Picture { width, height, format, bits, planes: out, matrix, full_range: !rgb && flags & 4 != 0, interlaced })
    }
}

/// `build_huffman`: one code per plane from the run-length coded lengths.
fn build_huffman(table: &[u8], max: usize, planes: usize) -> Result<Vec<Huffman>> {
    let mut out = Vec::with_capacity(planes);
    let mut len = vec![0u8; max];
    let mut j = 0usize;
    let mut p = 0usize;
    while p < table.len() {
        let b = table[p];
        p += 1;
        let x = b & 0x7f;
        let mut l = 1usize;
        if b & 0x80 != 0 {
            let Some(&n) = table.get(p) else { break };
            p += 1;
            l += usize::from(n);
        }
        let k = j + l;
        if k > max || x == 0 || x > 32 {
            return Err(Error::Invalid("Huffman lengths"));
        }
        len[j..k].fill(x);
        j = k;
        if j == max {
            j = 0;
            out.push(Huffman::new(&len)?);
            if out.len() == planes {
                break;
            }
        }
    }
    if out.len() != planes {
        return Err(Error::Invalid("Huffman tables too short"));
    }
    Ok(out)
}

/// One slice of one plane, `pw` x `ph`, from its data (flags, prediction, then the samples).
#[allow(clippy::too_many_arguments)]
fn decode_slice(dst: &mut [u16], pw: usize, ph: usize, data: &[u8], bits: u32, huff: &Huffman, interlaced: bool, mask: u32) -> Result<()> {
    let n = pw * ph;
    let flags = byte(data, 0);
    let pred = byte(data, 1);
    let body = data.get(2..).unwrap_or_default();
    if flags & 1 != 0 {
        if bits == 8 {
            if body.len() < n {
                return Err(Error::Invalid("raw slice too short"));
            }
            for (d, &s) in dst.iter_mut().zip(body) {
                *d = u16::from(s);
            }
        } else {
            let mut br = Bits { data: body, pos: 0 };
            if br.left() < (bits as usize * n) as isize {
                return Err(Error::Invalid("raw slice too short"));
            }
            for d in dst.iter_mut() {
                *d = br.read(bits) as u16;
            }
        }
    } else {
        let mut br = Bits { data: body, pos: 0 };
        'rows: for row in dst.chunks_mut(pw.max(1)).take(ph) {
            for d in row.iter_mut() {
                if br.left() <= 0 {
                    // the rest of the slice keeps its (zero) samples
                    continue 'rows;
                }
                *d = huff.decode(&mut br)?;
            }
        }
    }
    predict(dst, pw, ph, pred, interlaced, mask, bits == 8);
    Ok(())
}

/// Undo the prediction of a slice.
fn predict(p: &mut [u16], w: usize, h: usize, pred: u8, interlaced: bool, mask: u32, eight: bool) {
    if w == 0 || h == 0 {
        return;
    }
    let fake = w * (1 + usize::from(interlaced));
    let left_row = |p: &mut [u16], row: usize, acc: u32| {
        let mut acc = acc;
        for v in &mut p[row * w..(row + 1) * w] {
            acc = (acc + u32::from(*v)) & mask;
            *v = acc as u16;
        }
    };
    let first = 1 + usize::from(interlaced);
    match pred {
        1..=3 => {
            left_row(p, 0, 0);
            if interlaced && h > 1 {
                left_row(p, 1, 0);
            }
        }
        _ => return,
    }
    match pred {
        1 => {
            for k in first..h {
                let acc = u32::from(p[k * w - fake]);
                left_row(p, k, acc);
            }
        }
        2 => {
            for k in first..h {
                let row = k * w;
                let mut left = i64::from(p[row - fake]) + i64::from(p[row]);
                p[row] = (left & i64::from(mask)) as u16;
                for x in 1..w {
                    let top = i64::from(p[row + x - fake]);
                    let lefttop = i64::from(p[row + x - fake - 1]);
                    left += top - lefttop + i64::from(p[row + x]);
                    p[row + x] = (left & i64::from(mask)) as u16;
                }
            }
        }
        _ => {
            // median; the first left / top-left come from the row's own first sample
            if first >= h {
                return;
            }
            let mut left = i32::from(p[first * w]);
            let mut lefttop = left;
            for k in first..h {
                let row = k * w;
                let (mut l, mut lt) = (left, lefttop);
                for x in 0..w {
                    let t = i32::from(p[row + x - fake]);
                    let grad = if eight { (l + t - lt) & 0xff } else { l + t - lt };
                    l = (mid_pred(l, t, grad) + i32::from(p[row + x])) & mask as i32;
                    if !eight {
                        // uint16_t l, then &= max
                        l &= 0xffff;
                    }
                    lt = t;
                    p[row + x] = l as u16;
                }
                left = i32::from(p[row]);
                lefttop = left;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_code() {
        // lengths 2, 1, 3, 3 -> longest first: 2 (000), 3 (001), 0 (01), 1 (1)
        let h = Huffman::new(&[2, 1, 3, 3]).unwrap();
        let got: Vec<(u64, u8, u16)> = h.codes.iter().map(|&(c, l, s)| (c >> 29, l, s)).collect();
        assert_eq!(got, vec![(0, 3, 2), (1, 3, 3), (2, 2, 0), (4, 1, 1)]);
        assert!(Huffman::new(&[1, 1, 1]).is_err());
    }

    #[test]
    fn bad_input_is_an_error() {
        let mut d = Decoder::new();
        assert!(d.decode(&[]).is_err());
        assert!(d.decode(&[0; 64]).is_err());
        let mut p = b"MAGY".to_vec();
        p.extend_from_slice(&[32, 0, 0, 0, 7, 0x65]);
        p.resize(64, 0);
        assert!(d.decode(&p).is_err());
    }
}
