//! Uncompressed pictures (ST 384): the stored layout of the samples and unpacking them into planes.
//!
//! The layout comes from the picture essence coding label (ST 377-1:2019 G.2.25, which takes
//! precedence), else from the descriptor: the RGBA descriptor's Pixel Layout (G.2.40) or the CDCI
//! descriptor's Component Depth, Horizontal / Vertical Subsampling, Reversed Byte Order and
//! Padding Bits.
//!
//! G.2.25 describes every layout as big-endian bit-fields. The two packed 10-bit layouts (`v210`
//! and its planar variant) are read the way files carry them in practice: little-endian 32-bit
//! words with the first component in the least significant bits and the 2 pad bits on top, as
//! QuickTime defined `v210` and as every MXF writer we know stores it.

use crate::essence::PictureInfo;
use crate::klv::Ul;
use crate::{Error, Result};

/// The largest picture we unpack (pixels per frame): 8K full-aperture frames fit with room to spare.
pub const MAX_PIXELS: u64 = 1 << 26;

/// The Y'CbCr layouts of ST 377-1:2019 G.2.25, by their picture essence coding labels
/// (`06.0E.2B.34.04.01.01.0A.04.01.02.01.` + the last four bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UncompressedLayout {
    /// `01.01.01.01`: interleaved 4:4:4, 8-bit Cb Y Cr.
    CbYCr444x8,
    /// `01.02.01.01`: interleaved 4:2:2, 8-bit Cb Y Cr Y (`2vuy`, UYVY).
    Uyvy8,
    /// `01.02.01.02`: interleaved 4:2:2, 8-bit Y Cb Y Cr (`yuv2`, YUY2).
    Yuy2x8,
    /// `01.02.01.03`: planar 4:2:2, 8-bit (Y, then Cb, then Cr).
    Planar422x8,
    /// `01.02.02.01`: interleaved 4:2:2, 10-bit `v210` (three components per 32-bit word).
    V210,
    /// `01.02.02.02`: planar 4:2:2, 10-bit (three components per 32-bit word in each plane).
    Planar422x10,
    /// `01.02.03.01`: interleaved 4:2:2, 12-bit Cb Y Cr Y.
    CbYCrY12,
    /// `01.02.04.01`: interleaved 4:2:2, 16-bit Cb Y Cr Y.
    CbYCrY16,
    /// `01.03.01.02`: planar 4:2:0, 8-bit (Y, then Cb, then Cr).
    Planar420x8,
}

const LAYOUT_LABELS: [(UncompressedLayout, [u8; 4]); 9] = [
    (UncompressedLayout::CbYCr444x8, [1, 1, 1, 1]),
    (UncompressedLayout::Uyvy8, [1, 2, 1, 1]),
    (UncompressedLayout::Yuy2x8, [1, 2, 1, 2]),
    (UncompressedLayout::Planar422x8, [1, 2, 1, 3]),
    (UncompressedLayout::V210, [1, 2, 2, 1]),
    (UncompressedLayout::Planar422x10, [1, 2, 2, 2]),
    (UncompressedLayout::CbYCrY12, [1, 2, 3, 1]),
    (UncompressedLayout::CbYCrY16, [1, 2, 4, 1]),
    (UncompressedLayout::Planar420x8, [1, 3, 1, 2]),
];

impl UncompressedLayout {
    /// The picture essence coding label.
    pub fn label(self) -> [u8; 16] {
        let tail = LAYOUT_LABELS.iter().find(|(l, _)| *l == self).map_or([0; 4], |(_, t)| *t);
        [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x0A, 0x04, 0x01, 0x02, 0x01, tail[0], tail[1], tail[2], tail[3]]
    }
    /// The layout of a picture essence coding label (the version byte is ignored).
    pub fn from_label(ul: Ul) -> Option<Self> {
        let b = &ul.0;
        if !ul.is_smpte() || b[8..12] != [0x04, 0x01, 0x02, 0x01] {
            return None;
        }
        LAYOUT_LABELS.iter().find(|(_, t)| b[12..16] == t[..]).map(|(l, _)| *l)
    }
    /// Bits per component.
    pub fn depth(self) -> u32 {
        match self {
            UncompressedLayout::V210 | UncompressedLayout::Planar422x10 => 10,
            UncompressedLayout::CbYCrY12 => 12,
            UncompressedLayout::CbYCrY16 => 16,
            _ => 8,
        }
    }
    /// (horizontal, vertical) chroma subsampling.
    pub fn subsampling(self) -> (u32, u32) {
        match self {
            UncompressedLayout::CbYCr444x8 => (1, 1),
            UncompressedLayout::Planar420x8 => (2, 2),
            _ => (2, 1),
        }
    }
    /// Bytes of one `width`×`height` picture (a frame or one field).
    pub fn picture_bytes(self, width: u32, height: u32) -> u64 {
        Layout::YCbCr(self).picture_bytes(width, height)
    }
    fn short_name(self) -> &'static str {
        match self {
            UncompressedLayout::CbYCr444x8 => "Cb Y Cr",
            UncompressedLayout::Uyvy8 => "UYVY",
            UncompressedLayout::Yuy2x8 => "YUY2",
            UncompressedLayout::Planar422x8 | UncompressedLayout::Planar420x8 | UncompressedLayout::Planar422x10 => "planar",
            UncompressedLayout::V210 => "v210",
            UncompressedLayout::CbYCrY12 | UncompressedLayout::CbYCrY16 => "Cb Y Cr Y",
        }
    }
}

/// A Y'CbCr component of an interleaved pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comp {
    /// Luma.
    Y,
    Cb,
    Cr,
}

/// A component of an RGBA pixel layout (G.2.40).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rgba {
    /// Red, green, blue, alpha; a fill component holds unused bits.
    R,
    G,
    B,
    A,
    Fill,
}

/// The stored layout of an uncompressed picture track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Layout {
    /// One of the labelled Y'CbCr layouts.
    YCbCr(UncompressedLayout),
    /// Interleaved Y'CbCr bit-fields from the CDCI descriptor's properties: `pattern` repeats every
    /// `pixels` pixels (1 for 4:4:4, 2 for 4:2:2), each component `depth` bits, then `padding` bits.
    Interleaved { pattern: &'static [Comp], pixels: u32, depth: u32, padding: u32 },
    /// R'G'B'(A) bit-fields from the RGBA descriptor's Pixel Layout: (component, bits) in stored
    /// order; R, G and B share one depth. `min`, `max`: the code values of black and white.
    Rgba { components: Vec<(Rgba, u32)>, depth: u32, min: u32, max: u32 },
}

const CB_Y_CR: &[Comp] = &[Comp::Cb, Comp::Y, Comp::Cr];
const Y_CB_CR: &[Comp] = &[Comp::Y, Comp::Cb, Comp::Cr];
const CB_Y_CR_Y: &[Comp] = &[Comp::Cb, Comp::Y, Comp::Cr, Comp::Y];
const Y_CB_Y_CR: &[Comp] = &[Comp::Y, Comp::Cb, Comp::Y, Comp::Cr];

impl Layout {
    /// The layout of an uncompressed picture track: the coding label's, else the descriptor's.
    pub fn of(p: &PictureInfo) -> Result<Layout> {
        if let Some(l) = p.picture_coding.and_then(UncompressedLayout::from_label) {
            return Ok(Layout::YCbCr(l));
        }
        if p.rgba {
            return rgba_layout(p);
        }
        let d = p.component_depth;
        let reversed = p.reversed_byte_order;
        let (h, v) = (p.horizontal_subsampling.max(1), p.vertical_subsampling.max(1));
        let unsupported = || Error::Unsupported(format!("uncompressed {d}-bit Y'CbCr with {h}:{v} subsampling"));
        if !(1..=16).contains(&d) {
            return Err(unsupported());
        }
        Ok(match (h, v) {
            (2, 2) if d == 8 => Layout::YCbCr(UncompressedLayout::Planar420x8),
            // 10-bit 4:2:2 has no bit-field equivalent; files store it as v210
            (2, 1) if d == 10 && p.padding_bits == 0 => Layout::YCbCr(UncompressedLayout::V210),
            (2, 1) => Layout::Interleaved { pattern: if reversed { Y_CB_Y_CR } else { CB_Y_CR_Y }, pixels: 2, depth: d, padding: p.padding_bits },
            (1, 1) => Layout::Interleaved { pattern: if reversed { Y_CB_CR } else { CB_Y_CR }, pixels: 1, depth: d, padding: p.padding_bits },
            _ => return Err(unsupported()),
        })
    }

    /// Bits per component of the colour components.
    pub fn depth(&self) -> u32 {
        match self {
            Layout::YCbCr(l) => l.depth(),
            Layout::Interleaved { depth, .. } | Layout::Rgba { depth, .. } => *depth,
        }
    }

    /// (horizontal, vertical) chroma subsampling; (1, 1) for R'G'B'.
    pub fn subsampling(&self) -> (u32, u32) {
        match self {
            Layout::YCbCr(l) => l.subsampling(),
            Layout::Interleaved { pixels, .. } => (*pixels, 1),
            Layout::Rgba { .. } => (1, 1),
        }
    }

    /// The components are R'G'B'.
    pub fn rgb(&self) -> bool {
        matches!(self, Layout::Rgba { .. })
    }

    /// The layout carries an alpha component.
    pub fn alpha(&self) -> bool {
        matches!(self, Layout::Rgba { components, .. } if components.iter().any(|c| c.0 == Rgba::A))
    }

    /// A display name, e.g. "Uncompressed 4:2:2 10-bit (v210)".
    pub fn name(&self) -> String {
        let chroma = match self.subsampling() {
            (2, 2) => "4:2:0",
            (2, 1) => "4:2:2",
            _ => "4:4:4",
        };
        match self {
            Layout::YCbCr(l) => format!("Uncompressed {chroma} {}-bit ({})", l.depth(), l.short_name()),
            Layout::Interleaved { depth, .. } => format!("Uncompressed {chroma} {depth}-bit"),
            Layout::Rgba { depth, .. } => format!("Uncompressed {} {depth}-bit", if self.alpha() { "RGBA" } else { "RGB" }),
        }
    }

    /// Bytes of one line (interleaved layouts), rounded up to whole bytes.
    fn line_bytes(&self, width: u32) -> u64 {
        let w = u64::from(width);
        match self {
            Layout::YCbCr(UncompressedLayout::V210) => w.div_ceil(48) * 128,
            Layout::YCbCr(l @ (UncompressedLayout::Planar422x8 | UncompressedLayout::Planar420x8 | UncompressedLayout::Planar422x10)) => {
                // planar: the luma plane's line (only used for sizes)
                if *l == UncompressedLayout::Planar422x10 { w.div_ceil(3) * 4 } else { w }
            }
            Layout::YCbCr(UncompressedLayout::CbYCr444x8) => w * 3,
            Layout::YCbCr(UncompressedLayout::Uyvy8 | UncompressedLayout::Yuy2x8) => w.div_ceil(2) * 4,
            Layout::YCbCr(UncompressedLayout::CbYCrY12) => w.div_ceil(2) * 6,
            Layout::YCbCr(UncompressedLayout::CbYCrY16) => w.div_ceil(2) * 8,
            Layout::Interleaved { pattern, pixels, depth, padding } => {
                let group = pattern.len() as u64 * u64::from(*depth) + u64::from(*padding);
                (w.div_ceil(u64::from((*pixels).max(1))) * group).div_ceil(8)
            }
            Layout::Rgba { components, .. } => (w * components.iter().map(|c| u64::from(c.1)).sum::<u64>()).div_ceil(8),
        }
    }

    /// Bytes of one `width`×`height` picture (a frame, or one field of a separate-fields track).
    pub fn picture_bytes(&self, width: u32, height: u32) -> u64 {
        let (w, h) = (u64::from(width), u64::from(height));
        match self {
            Layout::YCbCr(UncompressedLayout::Planar422x8) => w * h + 2 * w.div_ceil(2) * h,
            Layout::YCbCr(UncompressedLayout::Planar420x8) => w * h + 2 * w.div_ceil(2) * h.div_ceil(2),
            Layout::YCbCr(UncompressedLayout::Planar422x10) => (w * h).div_ceil(3) * 4 + 2 * (w.div_ceil(2) * h).div_ceil(3) * 4,
            _ => self.line_bytes(width) * h,
        }
    }

    /// Bytes of one edit unit of a track: every field with its start and end fill.
    pub fn edit_unit_bytes(&self, p: &PictureInfo) -> u64 {
        let g = Geometry::of(p);
        let field = self.picture_bytes(p.stored_width, g.field_lines) + u64::from(p.image_start_offset) + u64::from(p.image_end_offset);
        field * u64::from(g.fields)
    }
}

fn rgba_layout(p: &PictureInfo) -> Result<Layout> {
    let mut components = Vec::new();
    for &(code, bits) in &p.pixel_layout {
        if code == 0 || bits == 0 {
            break;
        }
        let c = match code {
            b'R' => Rgba::R,
            b'G' => Rgba::G,
            b'B' => Rgba::B,
            b'A' => Rgba::A,
            b'F' => Rgba::Fill,
            0xD8..=0xDA => return Err(Error::Unsupported("uncompressed X'Y'Z' pictures".into())),
            b'P' => return Err(Error::Unsupported("uncompressed palette pictures".into())),
            _ => return Err(Error::Unsupported(format!("uncompressed RGBA pixel layout component {:?}", code as char))),
        };
        if bits > 16 {
            return Err(Error::Unsupported(format!("uncompressed RGBA components of {bits} bits")));
        }
        components.push((c, u32::from(bits)));
    }
    let depth_of = |want: Rgba| -> Result<u32> {
        let mut found = components.iter().filter(|c| c.0 == want);
        match (found.next(), found.next()) {
            (Some(c), None) => Ok(c.1),
            _ => Err(Error::Unsupported("uncompressed RGBA pixel layout without exactly one R, G and B".into())),
        }
    };
    let depth = depth_of(Rgba::R)?;
    if depth_of(Rgba::G)? != depth || depth_of(Rgba::B)? != depth || components.iter().filter(|c| c.0 == Rgba::A).count() > 1 {
        return Err(Error::Unsupported("uncompressed RGBA components of different depths".into()));
    }
    let full = (1u32 << depth) - 1;
    let min = p.component_min_ref.filter(|&m| m < full).unwrap_or(0);
    let max = p.component_max_ref.filter(|&m| m > min && m <= full).unwrap_or(full);
    Ok(Layout::Rgba { components, depth, min, max })
}

/// How the lines of an edit unit form the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Geometry {
    /// Stored pictures per edit unit (2 for separate fields).
    fields: u32,
    /// Lines of each stored picture.
    field_lines: u32,
}

impl Geometry {
    fn of(p: &PictureInfo) -> Self {
        match p.frame_layout {
            // separate fields: each field is stored as its own picture of `stored_height` lines
            1 => Geometry { fields: 2, field_lines: p.stored_height },
            // mixed fields: one interleaved frame, `stored_height` is the height of a field
            3 => Geometry { fields: 1, field_lines: p.stored_height.saturating_mul(2) },
            _ => Geometry { fields: 1, field_lines: p.stored_height },
        }
    }
}

/// An unpacked picture: planes of `depth`-bit samples, Y' Cb Cr (chroma subsampled by
/// `subsampling`) or full-range R' G' B', plus alpha when the layout has one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub subsampling: (u32, u32),
    pub rgb: bool,
    pub planes: [Vec<u16>; 3],
    pub alpha: Option<Vec<u16>>,
    /// The display rectangle (x, y, width, height) within the picture.
    pub display: (u32, u32, u32, u32),
}

/// Bit-field reader, most significant bit first.
struct Bits<'a> {
    data: &'a [u8],
    pos: u64,
}

impl Bits<'_> {
    fn read(&mut self, n: u32) -> u16 {
        let mut v: u32 = 0;
        for _ in 0..n {
            let byte = self.data.get((self.pos / 8) as usize).copied().unwrap_or(0);
            v = (v << 1) | u32::from((byte >> (7 - (self.pos % 8) as u8)) & 1);
            self.pos += 1;
        }
        v as u16
    }
    fn skip(&mut self, n: u32) {
        self.pos += u64::from(n);
    }
    fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }
}

/// One component of a 10-bit little-endian packed word (`k` 0, 1 or 2 from the least significant bits).
fn word10(data: &[u8], word: usize, k: usize) -> u16 {
    let w = data.get(word * 4..word * 4 + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    ((w >> (10 * k)) & 0x3FF) as u16
}

/// Unpacks one stored picture of `w`×`h` into planes (Y Cb Cr or R G B, and alpha).
fn unpack(layout: &Layout, data: &[u8], w: usize, h: usize) -> ([Vec<u16>; 3], Option<Vec<u16>>) {
    let (sx, sy) = layout.subsampling();
    let (cw, ch) = (w.div_ceil(sx as usize), h.div_ceil(sy as usize));
    let mut y = vec![0u16; w * h];
    let mut cb = vec![0u16; cw * ch];
    let mut cr = vec![0u16; cw * ch];
    let stride = layout.line_bytes(w as u32) as usize;
    match layout {
        Layout::YCbCr(UncompressedLayout::Planar422x8 | UncompressedLayout::Planar420x8) => {
            let (ys, rest) = data.split_at(data.len().min(w * h));
            let (us, vs) = rest.split_at(rest.len().min(cw * ch));
            for (o, i) in y.iter_mut().zip(ys) {
                *o = u16::from(*i);
            }
            for (o, i) in cb.iter_mut().zip(us) {
                *o = u16::from(*i);
            }
            for (o, i) in cr.iter_mut().zip(vs) {
                *o = u16::from(*i);
            }
        }
        Layout::YCbCr(UncompressedLayout::Planar422x10) => {
            let yw = (w * h).div_ceil(3);
            let cwords = (cw * ch).div_ceil(3);
            let planes: [(&mut Vec<u16>, usize); 3] = [(&mut y, 0), (&mut cb, yw), (&mut cr, yw + cwords)];
            for (plane, first) in planes {
                for (i, o) in plane.iter_mut().enumerate() {
                    *o = word10(data, first + i / 3, i % 3);
                }
            }
        }
        Layout::YCbCr(UncompressedLayout::V210) => {
            for row in 0..h {
                let line = data.get(row * stride..).unwrap_or(&[]);
                // each 4-word block holds 6 pixels: Cb Y Cr | Y Cb Y | Cr Y Cb | Y Cr Y
                let (yl, cl) = (&mut y[row * w..row * w + w], row * cw);
                let mut n = 0usize;
                let (mut yi, mut ci) = (0usize, 0usize);
                while yi < w || ci < cw {
                    let v = word10(line, n / 3, n % 3);
                    // component order within a 6-pixel group of 12 samples
                    match n % 4 {
                        0 => {
                            if let Some(o) = cb.get_mut(cl + ci).filter(|_| ci < cw) {
                                *o = v;
                            }
                        }
                        2 => {
                            if let Some(o) = cr.get_mut(cl + ci).filter(|_| ci < cw) {
                                *o = v;
                            }
                            ci += 1;
                        }
                        _ => {
                            if let Some(o) = yl.get_mut(yi) {
                                *o = v;
                            }
                            yi += 1;
                        }
                    }
                    n += 1;
                }
            }
        }
        Layout::Rgba { components, .. } => {
            let mut a = layout.alpha().then(|| vec![0u16; w * h]);
            for row in 0..h {
                let mut bits = Bits { data: data.get(row * stride..).unwrap_or(&[]), pos: 0 };
                for x in 0..w {
                    let i = row * w + x;
                    for &(c, n) in components {
                        let v = bits.read(n);
                        match c {
                            Rgba::R => y[i] = v,
                            Rgba::G => cb[i] = v,
                            Rgba::B => cr[i] = v,
                            Rgba::A => {
                                if let Some(a) = a.as_mut() {
                                    a[i] = v;
                                }
                            }
                            Rgba::Fill => {}
                        }
                    }
                }
            }
            return ([y, cb, cr], a);
        }
        _ => {
            let (pattern, pixels, depth, padding) = match layout {
                Layout::YCbCr(UncompressedLayout::CbYCr444x8) => (CB_Y_CR, 1, 8, 0),
                Layout::YCbCr(UncompressedLayout::Uyvy8) => (CB_Y_CR_Y, 2, 8, 0),
                Layout::YCbCr(UncompressedLayout::Yuy2x8) => (Y_CB_Y_CR, 2, 8, 0),
                Layout::YCbCr(UncompressedLayout::CbYCrY12) => (CB_Y_CR_Y, 2, 12, 0),
                Layout::YCbCr(UncompressedLayout::CbYCrY16) => (CB_Y_CR_Y, 2, 16, 0),
                Layout::Interleaved { pattern, pixels, depth, padding } => (*pattern, *pixels as usize, *depth, *padding),
                _ => (CB_Y_CR, 1, 8, 0),
            };
            for row in 0..h {
                let line = data.get(row * stride..).unwrap_or(&[]);
                if depth == 8 && padding == 0 {
                    // byte per component
                    for (g, group) in line.chunks(pattern.len()).take(w.div_ceil(pixels)).enumerate() {
                        place(pattern, group.iter().map(|&b| u16::from(b)), g, pixels, row, w, cw, &mut y, &mut cb, &mut cr);
                    }
                } else {
                    let mut bits = Bits { data: line, pos: 0 };
                    for g in 0..w.div_ceil(pixels) {
                        let vals: Vec<u16> = pattern.iter().map(|_| bits.read(depth)).collect();
                        bits.skip(padding);
                        place(pattern, vals.into_iter(), g, pixels, row, w, cw, &mut y, &mut cb, &mut cr);
                    }
                    bits.align();
                }
            }
        }
    }
    ([y, cb, cr], None)
}

/// Stores the components of interleaved group `g` of line `row`.
#[allow(clippy::too_many_arguments)]
fn place(
    pattern: &[Comp],
    vals: impl Iterator<Item = u16>,
    g: usize,
    pixels: usize,
    row: usize,
    w: usize,
    cw: usize,
    y: &mut [u16],
    cb: &mut [u16],
    cr: &mut [u16],
) {
    let mut yi = g * pixels;
    for (c, v) in pattern.iter().zip(vals) {
        match c {
            Comp::Y => {
                if yi < w
                    && let Some(o) = y.get_mut(row * w + yi)
                {
                    *o = v;
                }
                yi += 1;
            }
            Comp::Cb => {
                if let Some(o) = cb.get_mut(row * cw + g).filter(|_| g < cw) {
                    *o = v;
                }
            }
            Comp::Cr => {
                if let Some(o) = cr.get_mut(row * cw + g).filter(|_| g < cw) {
                    *o = v;
                }
            }
        }
    }
}

/// Interleaves the lines of two fields (`first` on the even lines).
fn weave(first: Vec<u16>, second: Vec<u16>, w: usize) -> Vec<u16> {
    let mut out = Vec::with_capacity(first.len() + second.len());
    let w = w.max(1);
    let mut a = first.chunks(w);
    let mut b = second.chunks(w);
    loop {
        match (a.next(), b.next()) {
            (None, None) => break,
            (x, y) => {
                out.extend_from_slice(x.unwrap_or(&[]));
                out.extend_from_slice(y.unwrap_or(&[]));
            }
        }
    }
    out
}

/// Scales R'G'B'(A) samples from `[min, max]` codes of `from` bits to full-range `to` bits.
fn rescale(plane: &mut [u16], min: u32, max: u32, to: u32) {
    let span = max.saturating_sub(min).max(1);
    let out = (1u32 << to) - 1;
    for v in plane {
        let x = u32::from(*v).clamp(min, max) - min;
        *v = ((u64::from(x) * u64::from(out) + u64::from(span / 2)) / u64::from(span)) as u16;
    }
}

/// Unpacks one edit unit of an uncompressed track with descriptor `p` and layout `layout`.
pub fn decode(layout: &Layout, p: &PictureInfo, data: &[u8]) -> Result<Picture> {
    let g = Geometry::of(p);
    let (w, field_h) = (p.stored_width, g.field_lines);
    let frame_h = field_h.saturating_mul(g.fields);
    if w == 0 || field_h == 0 {
        return Err(Error::Invalid("uncompressed picture without stored dimensions".into()));
    }
    if u64::from(w) * u64::from(frame_h) > MAX_PIXELS {
        return Err(Error::Unsupported(format!("uncompressed picture of {w}×{frame_h} is too large")));
    }
    let need = layout.edit_unit_bytes(p);
    if (data.len() as u64) < need {
        return Err(Error::Invalid(format!("uncompressed picture of {} bytes, {need} expected", data.len())));
    }
    let (start, end) = (p.image_start_offset as usize, p.image_end_offset as usize);
    let field_bytes = layout.picture_bytes(w, field_h) as usize;
    let mut fields = Vec::new();
    let mut at = 0usize;
    for _ in 0..g.fields {
        let from = at + start;
        let bytes = data.get(from..from + field_bytes).ok_or_else(|| Error::Invalid("uncompressed field past the edit unit".into()))?;
        fields.push(unpack(layout, bytes, w as usize, field_h as usize));
        at = from + field_bytes + end;
    }
    let mut fields = fields.into_iter();
    let Some((mut planes, mut alpha)) = fields.next() else {
        return Err(Error::Invalid("uncompressed edit unit without pictures".into()));
    };
    if let Some((second, second_alpha)) = fields.next() {
        let (sx, _) = layout.subsampling();
        let cw = (w as usize).div_ceil(sx as usize);
        let [y0, b0, r0] = planes;
        let [y1, b1, r1] = second;
        planes = [weave(y0, y1, w as usize), weave(b0, b1, cw), weave(r0, r1, cw)];
        alpha = alpha.zip(second_alpha).map(|(a, b)| weave(a, b, w as usize));
    }
    let mut depth = layout.depth();
    if let Layout::Rgba { min, max, depth: d, .. } = layout {
        // full-range samples of at least 8 bits
        depth = (*d).max(8);
        for plane in &mut planes {
            rescale(plane, *min, *max, depth);
        }
        if let Some(a) = alpha.as_mut() {
            let ad = layout_alpha_depth(layout);
            rescale(a, 0, (1u32 << ad) - 1, depth);
        }
    }
    // the display rectangle: sampled offsets, then display offsets (vertical ones count field lines)
    let lines = if g.fields == 2 || p.frame_layout == 3 { 2 } else { 1 };
    let x0 = p.sampled_x_offset.saturating_add(p.display_x_offset).max(0) as u32;
    let y0 = (p.sampled_y_offset.saturating_add(p.display_y_offset).max(0) as u32).saturating_mul(lines);
    let x0 = x0.min(w.saturating_sub(1));
    let y0 = y0.min(frame_h.saturating_sub(1));
    let dw = p.frame_width().clamp(1, w - x0);
    let dh = p.frame_height().clamp(1, frame_h - y0);
    Ok(Picture { width: w, height: frame_h, depth, subsampling: layout.subsampling(), rgb: layout.rgb(), planes, alpha, display: (x0, y0, dw, dh) })
}

fn layout_alpha_depth(layout: &Layout) -> u32 {
    match layout {
        Layout::Rgba { components, .. } => components.iter().find(|c| c.0 == Rgba::A).map_or(8, |c| c.1.clamp(1, 16)),
        _ => 8,
    }
}

/// Bit-field writer, most significant bit first.
struct BitsOut {
    data: Vec<u8>,
    bits: u32,
    acc: u64,
}

impl BitsOut {
    fn put(&mut self, v: u16, n: u32) {
        self.acc = (self.acc << n) | u64::from(v & ((1u32 << n) - 1) as u16);
        self.bits += n;
        while self.bits >= 8 {
            self.bits -= 8;
            self.data.push((self.acc >> self.bits) as u8);
        }
    }
    fn align(&mut self) {
        if self.bits > 0 {
            self.put(0, 8 - self.bits);
        }
    }
}

impl UncompressedLayout {
    /// Packs a `width`×`height` picture of `depth()`-bit Y', Cb, Cr planes (chroma planes
    /// subsampled by `subsampling()`, sizes rounded up) into this layout. Missing samples are 0.
    pub fn pack(self, width: u32, height: u32, planes: [&[u16]; 3]) -> Vec<u8> {
        let (w, h) = (width as usize, height as usize);
        let (sx, sy) = self.subsampling();
        let (cw, ch) = (w.div_ceil(sx as usize), h.div_ceil(sy as usize));
        let at = |p: usize, i: usize| planes[p].get(i).copied().unwrap_or(0);
        let mut out = Vec::with_capacity(self.picture_bytes(width, height) as usize);
        match self {
            UncompressedLayout::Planar422x8 | UncompressedLayout::Planar420x8 => {
                for (p, n) in [(0, w * h), (1, cw * ch), (2, cw * ch)] {
                    out.extend((0..n).map(|i| at(p, i) as u8));
                }
            }
            UncompressedLayout::Planar422x10 => {
                for (p, n) in [(0, w * h), (1, cw * ch), (2, cw * ch)] {
                    for k in 0..n.div_ceil(3) {
                        let word = (0..3).fold(0u32, |a, j| if 3 * k + j < n { a | u32::from(at(p, 3 * k + j) & 0x3FF) << (10 * j) } else { a });
                        out.extend_from_slice(&word.to_le_bytes());
                    }
                }
            }
            UncompressedLayout::V210 => {
                let stride = Layout::YCbCr(self).line_bytes(width) as usize;
                for row in 0..h {
                    let start = out.len();
                    let mut samples = Vec::with_capacity(2 * w + 2);
                    for g in 0..cw {
                        samples.push(at(1, row * cw + g));
                        samples.push(if 2 * g < w { at(0, row * w + 2 * g) } else { 0 });
                        samples.push(at(2, row * cw + g));
                        samples.push(if 2 * g + 1 < w { at(0, row * w + 2 * g + 1) } else { 0 });
                    }
                    for c in samples.chunks(3) {
                        let word = c.iter().enumerate().fold(0u32, |a, (j, &v)| a | u32::from(v & 0x3FF) << (10 * j));
                        out.extend_from_slice(&word.to_le_bytes());
                    }
                    out.resize(start + stride, 0);
                }
            }
            _ => {
                let (pattern, pixels) = match self {
                    UncompressedLayout::CbYCr444x8 => (CB_Y_CR, 1),
                    UncompressedLayout::Yuy2x8 => (Y_CB_Y_CR, 2),
                    _ => (CB_Y_CR_Y, 2),
                };
                let depth = self.depth();
                let mut bits = BitsOut { data: out, bits: 0, acc: 0 };
                for row in 0..h {
                    for g in 0..w.div_ceil(pixels) {
                        let mut yi = g * pixels;
                        for c in pattern {
                            let v = match c {
                                Comp::Y => {
                                    yi += 1;
                                    if yi - 1 < w { at(0, row * w + yi - 1) } else { 0 }
                                }
                                Comp::Cb => at(1, row * cw + g),
                                Comp::Cr => at(2, row * cw + g),
                            };
                            bits.put(v, depth);
                        }
                    }
                    bits.align();
                }
                out = bits.data;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests;
