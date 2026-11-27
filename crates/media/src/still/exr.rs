//! OpenEXR stills (and the frames of EXR image sequences), read from the public OpenEXR file layout:
//! single-part scan-line files with no, RLE, ZIPS (one line per chunk), ZIP or PXR24 (16 lines)
//! or PIZ (32 lines, [`piz`]) compression,
//! half, float or uint channels. R, G, B and A become the frame's channels (missing colour channels
//! are 0, missing alpha is opaque); a luminance-only file (Y) is shown grey. EXR values are
//! scene-linear with premultiplied alpha, which is exactly the compositor's working format, so the
//! frame keeps them as 32-bit floats (values above 1.0 survive into effects and colour work).
//!
//! The `framesPerSecond` and `timeCode` attributes give a sequence its frame rate and start time
//! code (a still's too). A `chromaticities` attribute other than Rec.709 / D65 (ACES AP0 or AP1,
//! P3, Rec.2020, …) is honoured: the decoder converts the pixels to the linear Rec.709 working
//! primaries (normalised primary matrix per SMPTE RP 177, Bradford white adaptation), keeping
//! values outside the Rec.709 gamut as negative or above-1.0 floats.
//!
//! The display window is the frame; pixels of the data window outside it are cut, and the parts of
//! it the data window does not cover are transparent. Tiled, deep and multi-part files, chroma
//! subsampled channels and the B44 and DWA compressions are refused with a clear error.

pub mod inflate;
pub mod piz;

use std::sync::Arc;

use cfa4_color::{ColorInfo, Transfer};
use cfa4_frame::VideoFrame;
use cfa4_time::FrameRate;
use rayon::prelude::*;

use super::{DEFAULT_STILL_DURATION, StillSource};
use crate::{MediaError, MediaInfo, MediaKind, Result, VideoStreamInfo};

/// Magic number at the start of every OpenEXR file (20000630 little-endian).
const MAGIC: [u8; 4] = [0x76, 0x2f, 0x31, 0x01];

/// Largest picture we decode (each pixel takes 16 bytes as RGBA floats).
const MAX_PIXELS: u64 = 1 << 26;
/// Maximum memory retained while decoding the data window and, when different,
/// the display window. Both buffers coexist during placement.
/// 2 GiB: an 8K RGBA f32 plate is 566 MB and the data and display windows may both be held.
const MAX_DECODED_BYTES: usize = 2 << 30;
/// Most attributes / channels we read from a header.
const MAX_ATTRIBUTES: usize = 4096;
const MAX_CHANNELS: usize = 1024;
/// Largest uncompressed chunk (scan lines × every channel) accepted. A real chunk is a few MB
/// (16 lines of a 16K-wide, 8-channel float image is 8 MB); a hostile header naming a thousand
/// channels on a 64M-pixel-wide line would otherwise ask for hundreds of GB.
const MAX_CHUNK_BYTES: usize = 256 << 20;

/// Whether `b` starts like an OpenEXR file.
pub fn sniff(b: &[u8]) -> bool {
    b.starts_with(&MAGIC)
}

/// The colour of EXR frames: scene-linear light, Rec.709 primaries.
const LINEAR_709: ColorInfo = ColorInfo { transfer: Transfer::Linear, ..ColorInfo::SRGB_FULL };

/// A decoded EXR picture.
pub struct Decoded {
    /// Premultiplied linear RGBA floats, display-window sized.
    pub frame: VideoFrame,
    /// Whether any pixel is not fully opaque.
    pub has_alpha: bool,
    /// For the media info, e.g. "RGBA 16-bit float".
    pub pixel_format: String,
    /// The `framesPerSecond` and `timeCode` attributes.
    pub timing: super::Timing,
}

/// Open an EXR file as a still.
pub fn still(name: &str, bytes: &[u8]) -> Result<StillSource> {
    let d = decode(name, bytes)?;
    let (frame_rate, start_timecode) = d.timing.resolve(FrameRate::FPS_29_97);
    let info = MediaInfo {
        name: name.to_string(),
        kind: MediaKind::Still,
        duration: DEFAULT_STILL_DURATION,
        video: Some(VideoStreamInfo {
            width: d.frame.width,
            height: d.frame.height,
            frame_rate,
            par: (1, 1),
            codec: "EXR".into(),
            pixel_format: d.pixel_format,
            color: LINEAR_709,
            has_alpha: d.has_alpha,
            bitrate: None,
            hdr: None,
        }),
        audio_streams: Vec::new(),
        container: "Image".into(),
        start_timecode,
        file_size: Some(bytes.len() as u64),
    };
    Ok(StillSource { info, frame: Arc::new(d.frame) })
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PixelType {
    Half,
    Uint,
    Float,
}

impl PixelType {
    fn size(self) -> usize {
        match self {
            PixelType::Half => 2,
            PixelType::Uint | PixelType::Float => 4,
        }
    }
}

/// Where a channel's samples go.
#[derive(Clone, Copy)]
enum Target {
    Slot(usize),
    /// Luminance: R, G and B.
    Grey,
    Skip,
}

struct Channel {
    name: String,
    ty: PixelType,
}

/// An inclusive pixel rectangle (`box2i`).
#[derive(Clone, Copy, PartialEq, Eq)]
struct Window {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl Window {
    fn size(self) -> Option<(usize, usize)> {
        let w = i64::from(self.x1) - i64::from(self.x0) + 1;
        let h = i64::from(self.y1) - i64::from(self.y0) + 1;
        if w <= 0 || h <= 0 || (w as u64).saturating_mul(h as u64) > MAX_PIXELS {
            return None;
        }
        Some((w as usize, h as usize))
    }
}

#[derive(Clone, Copy)]
enum Compression {
    None,
    Rle,
    Zip,
    /// Lossy 24-bit floats: deflated byte planes of per-line differences.
    Pxr24,
    /// Wavelet + Huffman, 32 scan lines per chunk.
    Piz,
}

impl Compression {
    fn lines_per_chunk(self, code: u8) -> usize {
        match self {
            Compression::Zip if code == 3 => 16,
            Compression::Pxr24 => 16,
            Compression::Piz => 32,
            _ => 1,
        }
    }
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// A NUL-terminated string of at most 255 bytes at `*pos`; advances past the NUL.
fn cstr<'a>(b: &'a [u8], pos: &mut usize) -> Option<&'a str> {
    let rest = b.get(*pos..)?;
    let n = rest.iter().take(256).position(|&c| c == 0)?;
    *pos += n + 1;
    std::str::from_utf8(rest.get(..n)?).ok()
}

fn window(v: &[u8]) -> Option<Window> {
    let g = |i: usize| le32(v, i * 4).map(|x| x as i32);
    Some(Window { x0: g(0)?, y0: g(1)?, x1: g(2)?, y1: g(3)? })
}

fn channels(v: &[u8]) -> Result<Vec<Channel>> {
    let mut out = Vec::new();
    let mut pos = 0;
    loop {
        let name = cstr(v, &mut pos).ok_or_else(|| MediaError::Decode("EXR channel list".into()))?;
        if name.is_empty() {
            return Ok(out);
        }
        let (Some(ty), Some(xs), Some(ys)) = (le32(v, pos), le32(v, pos + 8), le32(v, pos + 12)) else {
            return Err(MediaError::Decode("EXR channel list".into()));
        };
        pos += 16;
        let ty = match ty {
            0 => PixelType::Uint,
            1 => PixelType::Half,
            2 => PixelType::Float,
            _ => return Err(MediaError::Decode(format!("EXR channel {name}: pixel type {ty}"))),
        };
        if (xs, ys) != (1, 1) {
            return Err(MediaError::Unsupported(format!("EXR channel {name} is subsampled ({xs}×{ys})")));
        }
        if out.len() >= MAX_CHANNELS {
            return Err(MediaError::Decode("EXR has too many channels".into()));
        }
        out.push(Channel { name: name.to_string(), ty });
    }
}

struct Header {
    channels: Vec<Channel>,
    compression: Compression,
    code: u8,
    data: Window,
    display: Window,
    /// Offset of the line offset table.
    end: usize,
    timing: super::Timing,
    /// The `chromaticities` attribute: R, G, B and white (x, y).
    chromaticities: Option<[f64; 8]>,
}

/// Chromaticities of the common EXR colour spaces, for the media info (OpenEXR's default is
/// Rec.709 / D65; ACES from SMPTE ST 2065-1 and S-2014-004).
const NAMED_CHROMATICITIES: [(&str, [f64; 8]); 6] = [
    ("Rec.709", [0.64, 0.33, 0.30, 0.60, 0.15, 0.06, 0.3127, 0.3290]),
    ("ACES AP0", [0.7347, 0.2653, 0.0, 1.0, 0.0001, -0.0770, 0.32168, 0.33767]),
    ("ACES AP1", [0.713, 0.293, 0.165, 0.830, 0.128, 0.044, 0.32168, 0.33767]),
    ("P3-D65", [0.680, 0.320, 0.265, 0.690, 0.150, 0.060, 0.3127, 0.3290]),
    ("DCI-P3", [0.680, 0.320, 0.265, 0.690, 0.150, 0.060, 0.314, 0.351]),
    ("Rec.2020", [0.708, 0.292, 0.170, 0.797, 0.131, 0.046, 0.3127, 0.3290]),
];

fn same_chromaticities(a: &[f64; 8], b: &[f64; 8]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 5e-4)
}

/// The `chromaticities` attribute value (eight 32-bit floats), when every number is finite.
fn chromaticities(v: &[u8]) -> Option<[f64; 8]> {
    let mut c = [0.0; 8];
    for (i, o) in c.iter_mut().enumerate() {
        let f = f32::from_le_bytes(v.get(i * 4..i * 4 + 4)?.try_into().ok()?);
        if !f.is_finite() {
            return None;
        }
        *o = f64::from(f);
    }
    Some(c)
}

/// Linear RGB with chromaticities `c` → linear Rec.709 (D65 white). `None` when `c` is Rec.709 /
/// D65 already, or does not describe a usable RGB space (the pixels are then taken as Rec.709).
fn to_rec709(c: &[f64; 8]) -> Option<[[f32; 3]; 3]> {
    use cfa4_color::spaces::{self, Gamut};
    if same_chromaticities(c, &NAMED_CHROMATICITIES[0].1) || [c[1], c[3], c[5], c[7]].iter().any(|y| y.abs() < 1e-6) {
        return None;
    }
    // normalised primary matrix (SMPTE RP 177); a primary may lie below the spectral locus with a
    // negative y (ACES AP0 blue), which it handles
    let to_xyz = spaces::npm([[c[0], c[1]], [c[2], c[3]], [c[4], c[5]]], [c[6], c[7]]);
    let xyz = |x: f64, y: f64| [x / y, 1.0, (1.0 - x - y) / y];
    let adapt = cfa4_dng::color::bradford(&xyz(c[6], c[7]), &xyz(spaces::D65[0], spaces::D65[1]))?;
    let m = spaces::mul(&spaces::inverse(&Gamut::Bt709.to_xyz()), &spaces::mul(&adapt, &to_xyz));
    m.iter().flatten().all(|v| v.is_finite()).then(|| m.map(|r| r.map(|v| v as f32)))
}

fn header(name: &str, b: &[u8]) -> Result<Header> {
    let bad = |what: &str| MediaError::Decode(format!("{name}: EXR {what}"));
    if !sniff(b) {
        return Err(bad("magic number"));
    }
    let version = le32(b, 4).ok_or_else(|| bad("version"))?;
    if version & 0xFF != 2 {
        return Err(MediaError::Unsupported(format!("{name}: OpenEXR version {}", version & 0xFF)));
    }
    for (bit, what) in [(0x200, "tiled"), (0x800, "deep"), (0x1000, "multi-part")] {
        if version & bit != 0 {
            return Err(MediaError::Unsupported(format!("{name}: {what} OpenEXR files are not supported yet")));
        }
    }
    let (mut chans, mut code, mut data, mut display) = (None, None, None, None);
    let mut timing = super::Timing::default();
    let mut chroma = None;
    let mut pos = 8;
    for _ in 0..MAX_ATTRIBUTES {
        let attr = cstr(b, &mut pos).ok_or_else(|| bad("header"))?;
        if attr.is_empty() {
            let (Some(channels), Some(code), Some(data)) = (chans, code, data) else {
                return Err(bad("header lacks channels, compression or dataWindow"));
            };
            let compression = match code {
                0 => Compression::None,
                1 => Compression::Rle,
                2 | 3 => Compression::Zip,
                4 => Compression::Piz,
                5 => Compression::Pxr24,
                _ => {
                    let what = ["PIZ", "PXR24", "B44", "B44A", "DWAA", "DWAB"].get(usize::from(code).wrapping_sub(4)).copied().unwrap_or("unknown");
                    return Err(MediaError::Unsupported(format!("{name}: {what} compressed OpenEXR is not supported yet")));
                }
            };
            return Ok(Header { channels, compression, code, data, display: display.unwrap_or(data), end: pos, timing, chromaticities: chroma });
        }
        let ty = cstr(b, &mut pos).ok_or_else(|| bad("header"))?;
        let size = le32(b, pos).ok_or_else(|| bad("header"))? as usize;
        pos += 4;
        let value = b.get(pos..pos.saturating_add(size)).ok_or_else(|| bad("header"))?;
        pos += size;
        match (attr, ty) {
            ("channels", "chlist") => chans = Some(channels(value).map_err(|e| bad(&e.to_string()))?),
            ("compression", "compression") => code = value.first().copied(),
            ("dataWindow", "box2i") => data = Some(window(value).ok_or_else(|| bad("dataWindow"))?),
            ("displayWindow", "box2i") => display = Some(window(value).ok_or_else(|| bad("displayWindow"))?),
            // a damaged optional attribute is ignored, not an error
            // OpenEXR names it framesPerSecond; OpenImageIO 2.4 and older write FramesPerSecond
            ("framesPerSecond" | "FramesPerSecond", "rational") => {
                timing.frame_rate = le32(value, 0).zip(le32(value, 4)).map(|(n, d)| (i64::from(n as i32), i64::from(d))).filter(|&(n, d)| n > 0 && d > 0);
            }
            // SMPTE ST 12-1 time and flags, packed as OpenEXR stores them (the same byte layout as
            // the CinemaDNG TimeCodes tag); the user data word that follows is not used
            ("timeCode", "timecode") => timing.timecode = value.get(..4).and_then(cfa4_dng::Timecode::from_bytes),
            ("chromaticities", "chromaticities") => chroma = chromaticities(value),
            _ => {}
        }
    }
    Err(bad("header has too many attributes"))
}

/// IEEE half → f32.
fn half(h: u16) -> f32 {
    let sign = u32::from(h >> 15) << 31;
    let exp = u32::from((h >> 10) & 0x1F);
    let man = u32::from(h & 0x3FF);
    let bits = match (exp, man) {
        (0, 0) => sign,
        (0, _) => {
            // subnormal: normalise the mantissa
            let mut e = 113u32;
            let mut m = man;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | e << 23 | (m & 0x3FF) << 13
        }
        (31, _) => sign | 0x7F80_0000 | man << 13,
        _ => sign | (exp + 112) << 23 | man << 13,
    };
    f32::from_bits(bits)
}

/// A sample as a finite float (NaN → 0, infinities → the half range).
fn sample(ty: PixelType, s: &[u8]) -> f32 {
    let v = match (ty, s) {
        (PixelType::Half, [a, b, ..]) => half(u16::from_le_bytes([*a, *b])),
        (PixelType::Float, [a, b, c, d, ..]) => f32::from_le_bytes([*a, *b, *c, *d]),
        (PixelType::Uint, [a, b, c, d, ..]) => u32::from_le_bytes([*a, *b, *c, *d]) as f32,
        _ => 0.0,
    };
    if v.is_nan() { 0.0 } else { v.clamp(-65504.0, 65504.0) }
}

/// Undo the RLE / ZIP preprocessing: byte differences, then the split into even and odd bytes.
fn unpredict(mut t: Vec<u8>) -> Vec<u8> {
    let mut prev = 0u8;
    for (i, b) in t.iter_mut().enumerate() {
        if i > 0 {
            *b = prev.wrapping_add(*b).wrapping_sub(128);
        }
        prev = *b;
    }
    let (even, odd) = t.split_at(t.len().div_ceil(2));
    let mut out = Vec::with_capacity(t.len());
    for (i, &e) in even.iter().enumerate() {
        out.push(e);
        if let Some(&o) = odd.get(i) {
            out.push(o);
        }
    }
    out
}

/// OpenEXR run-length decoding into exactly `expected` bytes.
fn unrle(src: &[u8], expected: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    out.try_reserve_exact(expected).ok()?;
    let mut i = 0;
    while let Some(&c) = src.get(i) {
        let c = c as i8;
        i += 1;
        if c < 0 {
            let n = usize::from(c.unsigned_abs());
            out.extend_from_slice(src.get(i..i + n)?);
            i += n;
        } else {
            let n = c as usize + 1;
            let b = *src.get(i)?;
            i += 1;
            out.resize(out.len() + n, b);
        }
        if out.len() > expected {
            return None;
        }
    }
    (out.len() == expected).then_some(out)
}

/// Bytes of one sample in PXR24's byte planes (a float keeps its top 24 bits only).
fn plane_count(ty: PixelType) -> usize {
    match ty {
        PixelType::Half => 2,
        PixelType::Float => 3,
        PixelType::Uint => 4,
    }
}

/// Undo PXR24: per scan line and channel, the samples' differences (to the previous sample of the
/// line, modulo 2^16 / 2^24 / 2^32) are stored as byte planes, most significant first. Returns the
/// lines in the plain layout (little-endian samples).
fn unpxr24(planes: &[u8], channels: &[Channel], width: usize, lines: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(planes.len().checked_mul(2)?);
    let mut at = 0usize;
    for _ in 0..lines {
        for c in channels {
            let n = plane_count(c.ty);
            let len = n.checked_mul(width)?;
            let plane = planes.get(at..at.checked_add(len)?)?;
            at += len;
            let mut acc = 0u32;
            for x in 0..width {
                let mut d = 0u32;
                for k in 0..n {
                    d = d << 8 | u32::from(*plane.get(k * width + x)?);
                }
                acc = acc.wrapping_add(d);
                match c.ty {
                    PixelType::Half => out.extend_from_slice(&(acc as u16).to_le_bytes()),
                    PixelType::Float => out.extend_from_slice(&((acc & 0xFF_FFFF) << 8).to_le_bytes()),
                    PixelType::Uint => out.extend_from_slice(&acc.to_le_bytes()),
                }
            }
        }
    }
    (at == planes.len()).then_some(out)
}

/// Decode an EXR file into premultiplied linear RGBA floats.
pub fn decode(name: &str, b: &[u8]) -> Result<Decoded> {
    let h = header(name, b)?;
    let bad = |what: String| MediaError::Decode(format!("{name}: EXR {what}"));
    let (dw, dh) = h.data.size().ok_or_else(|| bad("data window size".into()))?;
    let (fw, fh) = h.display.size().ok_or_else(|| bad("display window size".into()))?;
    let (Ok(fw32), Ok(fh32)) = (u32::try_from(fw), u32::try_from(fh)) else {
        return Err(bad("display window size".into()));
    };
    // channel → frame component
    let has = |n: &str| h.channels.iter().any(|c| c.name == n);
    if has("RY") || has("BY") {
        return Err(MediaError::Unsupported(format!("{name}: luminance / chroma OpenEXR is not supported yet")));
    }
    let rgb = has("R") || has("G") || has("B");
    let targets: Vec<Target> = h
        .channels
        .iter()
        .map(|c| match c.name.as_str() {
            "R" => Target::Slot(0),
            "G" => Target::Slot(1),
            "B" => Target::Slot(2),
            "A" => Target::Slot(3),
            "Y" if !rgb => Target::Grey,
            _ => Target::Skip,
        })
        .collect();
    let used: Vec<PixelType> = h.channels.iter().zip(&targets).filter(|(_, t)| !matches!(t, Target::Skip)).map(|(c, _)| c.ty).collect();
    if !targets.iter().any(|t| matches!(t, Target::Slot(0..=2) | Target::Grey)) {
        return Err(MediaError::Unsupported(format!("{name}: EXR has no R, G, B or Y channel")));
    }
    let alpha = has("A");
    // byte offset of each channel within a scan line, and the line size
    let mut line = 0usize;
    let mut offs = Vec::with_capacity(h.channels.len());
    for c in &h.channels {
        offs.push(line);
        line = c.ty.size().checked_mul(dw).and_then(|n| line.checked_add(n)).ok_or_else(|| bad("line size".into()))?;
    }
    let lpc = h.compression.lines_per_chunk(h.code);
    if line.checked_mul(lpc).is_none_or(|n| n > MAX_CHUNK_BYTES) {
        return Err(MediaError::Unsupported(format!("{name}: EXR scan lines too large ({} channels, {dw} wide)", h.channels.len())));
    }
    let table = h.end;
    let px_len = dw.checked_mul(dh).and_then(|n| n.checked_mul(4)).ok_or_else(|| bad("size".into()))?;
    let frame_len = fw.checked_mul(fh).and_then(|n| n.checked_mul(4)).ok_or_else(|| bad("display window size".into()))?;
    let decoded_bytes = px_len
        .checked_mul(std::mem::size_of::<f32>())
        .and_then(|n| if h.data == h.display { Some(n) } else { frame_len.checked_mul(std::mem::size_of::<f32>()).and_then(|m| n.checked_add(m)) })
        .ok_or_else(|| bad("decoded size".into()))?;
    if decoded_bytes > MAX_DECODED_BYTES {
        return Err(MediaError::Unsupported(format!("{name}: EXR decoded buffers exceed the {MAX_DECODED_BYTES}-byte limit")));
    }
    let chunks = dh.div_ceil(lpc);
    let table_end = table.checked_add(chunks.checked_mul(8).ok_or_else(|| bad("offset table size".into()))?).ok_or_else(|| bad("offset table size".into()))?;
    if table_end > b.len() {
        return Err(bad("offset table is truncated".into()));
    }
    for i in 0..chunks {
        let at = table + i * 8;
        let off = le32(b, at).zip(le32(b, at + 4)).map(|(lo, hi)| u64::from(lo) | u64::from(hi) << 32);
        let off = off.and_then(|o| usize::try_from(o).ok()).ok_or_else(|| bad(format!("offset of chunk {i}")))?;
        let size = le32(b, off.checked_add(4).ok_or_else(|| bad(format!("chunk {i}")))?).ok_or_else(|| bad(format!("chunk {i}")))? as usize;
        let end = off.checked_add(8).and_then(|n| n.checked_add(size)).ok_or_else(|| bad(format!("chunk {i}")))?;
        if end > b.len() {
            return Err(bad(format!("chunk {i} is truncated")));
        }
        let y = le32(b, off).map(|y| y as i32);
        let want = i64::from(h.data.y0) + (i * lpc) as i64;
        if y.map(i64::from) != Some(want) {
            return Err(bad(format!("chunk {i} is not at line {want}")));
        }
    }
    let mut px = vec![0f32; px_len];
    if !alpha {
        px.par_chunks_mut(4).for_each(|p| {
            if let Some(a) = p.get_mut(3) {
                *a = 1.0;
            }
        });
    }
    let row = dw * 4;
    px.par_chunks_mut(row * lpc).enumerate().try_for_each(|(i, rows)| -> Result<()> {
        let n = rows.len() / row;
        let expected = line.checked_mul(n).ok_or_else(|| bad("chunk size".into()))?;
        let off = le32(b, table + i * 8).zip(le32(b, table + i * 8 + 4)).map(|(lo, hi)| u64::from(lo) | u64::from(hi) << 32);
        let off = off.and_then(|o| usize::try_from(o).ok()).ok_or_else(|| bad(format!("offset of chunk {i}")))?;
        let y = le32(b, off).map(|y| y as i32);
        let want = i64::from(h.data.y0) + (i * lpc) as i64;
        if y.map(i64::from) != Some(want) {
            return Err(bad(format!("chunk {i} is not at line {want}")));
        }
        let size = le32(b, off + 4).ok_or_else(|| bad(format!("chunk {i}")))? as usize;
        let data = b.get(off + 8..(off + 8).saturating_add(size)).ok_or_else(|| bad(format!("chunk {i} is truncated")))?;
        let raw: std::borrow::Cow<'_, [u8]> = if data.len() == expected {
            // stored raw when compression would not shrink it
            data.into()
        } else {
            match h.compression {
                Compression::None => return Err(bad(format!("chunk {i} has the wrong size"))),
                Compression::Rle => unpredict(unrle(data, expected).ok_or_else(|| bad(format!("chunk {i}: bad RLE data")))?).into(),
                Compression::Zip => unpredict(inflate::zlib(data, expected).map_err(|e| bad(format!("chunk {i}: {e}")))?).into(),
                Compression::Pxr24 => {
                    let per_line = h.channels.iter().try_fold(0usize, |t, c| t.checked_add(plane_count(c.ty).checked_mul(dw)?));
                    let planes = per_line.and_then(|l| l.checked_mul(n)).ok_or_else(|| bad("chunk size".into()))?;
                    let inflated = inflate::zlib(data, planes).map_err(|e| bad(format!("chunk {i}: {e}")))?;
                    unpxr24(&inflated, &h.channels, dw, n).ok_or_else(|| bad(format!("chunk {i}: bad PXR24 data")))?.into()
                }
                Compression::Piz => {
                    let chans: Vec<piz::PizChannel> = h.channels.iter().map(|c| piz::PizChannel { width: dw, lines: n, bytes: c.ty.size() }).collect();
                    piz::decompress(data, &chans, expected).map_err(|e| bad(format!("chunk {i}: {e}")))?.into()
                }
            }
        };
        for (l, out) in rows.chunks_exact_mut(row).enumerate() {
            let src = raw.get(l * line..(l + 1) * line).unwrap_or_default();
            for ((c, &t), &o) in h.channels.iter().zip(&targets).zip(&offs) {
                if matches!(t, Target::Skip) {
                    continue;
                }
                let sz = c.ty.size();
                let samples = src.get(o..o + sz * dw).unwrap_or_default();
                for (p, s) in out.as_chunks_mut::<4>().0.iter_mut().zip(samples.chunks_exact(sz)) {
                    let v = sample(c.ty, s);
                    match t {
                        Target::Slot(3) => p[3] = v.clamp(0.0, 1.0),
                        Target::Slot(k) => {
                            if let Some(q) = p.get_mut(k) {
                                *q = v;
                            }
                        }
                        Target::Grey => {
                            p[0] = v;
                            p[1] = v;
                            p[2] = v;
                        }
                        Target::Skip => {}
                    }
                }
            }
        }
        Ok(())
    })?;
    // place the data window into the display window
    let px = if h.data == h.display {
        px
    } else {
        let mut f = vec![0f32; fw * fh * 4];
        let dx = i64::from(h.data.x0) - i64::from(h.display.x0);
        let dy = i64::from(h.data.y0) - i64::from(h.display.y0);
        let (x0, x1) = (dx.max(0), (dx + dw as i64).min(fw as i64));
        if x0 < x1 {
            for (y, out) in f.chunks_exact_mut(fw * 4).enumerate() {
                let sy = y as i64 - dy;
                if sy < 0 || sy >= dh as i64 {
                    continue;
                }
                let src = (sy as usize * dw + (x0 - dx) as usize) * 4;
                let len = (x1 - x0) as usize * 4;
                if let (Some(d), Some(s)) = (out.get_mut(x0 as usize * 4..x0 as usize * 4 + len), px.get(src..src + len)) {
                    d.copy_from_slice(s);
                }
            }
        }
        f
    };
    // other primaries / white → the Rec.709 working primaries (alpha is untouched; the matrix is
    // linear, so premultiplied values convert as they are)
    let mut px = px;
    let gamut = if rgb { h.chromaticities.as_ref().and_then(to_rec709) } else { None };
    if let Some(m) = gamut {
        px.par_chunks_mut(4).for_each(|p| {
            if let [r, g, b, _] = p {
                [*r, *g, *b] = cfa4_color::spaces::apply3(&m, [*r, *g, *b]);
            }
        });
    }
    let has_alpha = px.par_chunks(4).any(|p| p.get(3).is_some_and(|&a| a < 1.0));
    let depth = match used.iter().max() {
        Some(PixelType::Float) => "32-bit float",
        Some(PixelType::Uint) => "32-bit integer",
        _ => "16-bit float",
    };
    let layout = match (alpha, rgb) {
        (true, _) => "RGBA",
        (false, true) => "RGB",
        (false, false) => "Y",
    };
    let space = match (&h.chromaticities, gamut) {
        (Some(c), Some(_)) => {
            let named = NAMED_CHROMATICITIES.iter().find(|(_, n)| same_chromaticities(c, n));
            format!(", {}", named.map_or("custom primaries", |(n, _)| n))
        }
        _ => String::new(),
    };
    let pixel_format = format!("{layout} {depth}{space}");
    let mut frame = VideoFrame::rgba_f32(fw32, fh32, px);
    frame.color = LINEAR_709;
    Ok(Decoded { frame, has_alpha, pixel_format, timing: h.timing })
}

#[cfg(test)]
mod tests;
