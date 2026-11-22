// Smacker video and audio decoders and demuxer, ported from FFmpeg n9.0.2 (libavcodec/smacker.c
// and libavformat/smacker.c), which follow the public Smacker description at wiki.multimedia.cx.
// Copyright (c) 2006 Konstantin Shishkov
//
// This file is part of FFmpeg.
//
// FFmpeg is free software; you can redistribute it and/or
// modify it under the terms of the GNU Lesser General Public
// License as published by the Free Software Foundation; either
// version 2.1 of the License, or (at your option) any later version.
//
// FFmpeg is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
// Lesser General Public License for more details.
//
// You should have received a copy of the GNU Lesser General Public
// License along with FFmpeg; if not, write to the Free Software
// Foundation, Inc., 51 Franklin Street, Fifth Floor, Boston, MA 02110-1301 USA
//
// CFA4 changes (the Rust re-expression, the frame splitting and palette handling shaped for
// seeking, the size limits, the snapshots) are GPL-3.0-or-later; see docs/licensing.md §5.

//! Pure-Rust decoder for **Smacker** files (`.smk`, RAD Game Tools; SMK2 and SMK4): the
//! container, the paletted 4×4-block video and the packed Huffman audio. The output is the same
//! as ffmpeg's `smk` demuxer with its `smackvid` and `smackaud` decoders, pixel for pixel and
//! sample for sample.
//!
//! - [`Smk::parse`] reads the header and the frame table; [`Smk::split`] cuts one frame into its
//!   palette update, audio chunks and video data.
//! - [`Decoder`] decodes the video, one frame at a time, onto a persistent paletted picture.
//! - [`decode_audio`] decodes one audio chunk (every chunk stands alone).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// Errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("not a Smacker file")]
    NotSmacker,
    #[error("invalid Smacker data: {0}")]
    Invalid(&'static str),
    #[error("unsupported Smacker stream: {0}")]
    Unsupported(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Audio tracks a file can hold.
pub const MAX_TRACKS: usize = 7;
/// Most pixels a picture may have (8192 × 4096; Smacker files are 640 × 480 at most in practice).
pub const MAX_PIXELS: u64 = 1 << 25;
/// Most frames (ffmpeg's limit).
pub const MAX_FRAMES: u32 = 0x00FF_FFFF;

const HEADER_BYTES: usize = 104;
const SMACKER_PAL: u8 = 0x01;
const FLAG_RING_FRAME: u32 = 0x01;
const FLAG_Y_INTERLACE: u32 = 1 << 1;
const FLAG_Y_DOUBLE: u32 = 1 << 2;

const AUD_PACKED: u8 = 0x80;
const AUD_16BITS: u8 = 0x20;
const AUD_STEREO: u8 = 0x10;
const AUD_BINKAUD: u8 = 0x08;
const AUD_USEDCT: u8 = 0x04;

/// 6-bit palette components as 8-bit ones.
const SMK_PAL: [u8; 64] = [
    0x00, 0x04, 0x08, 0x0C, 0x10, 0x14, 0x18, 0x1C, 0x20, 0x24, 0x28, 0x2C, 0x30, 0x34, 0x38, 0x3C, 0x41, 0x45, 0x49, 0x4D, 0x51, 0x55, 0x59, 0x5D, 0x61, 0x65,
    0x69, 0x6D, 0x71, 0x75, 0x79, 0x7D, 0x82, 0x86, 0x8A, 0x8E, 0x92, 0x96, 0x9A, 0x9E, 0xA2, 0xA6, 0xAA, 0xAE, 0xB2, 0xB6, 0xBA, 0xBE, 0xC3, 0xC7, 0xCB, 0xCF,
    0xD3, 0xD7, 0xDB, 0xDF, 0xE3, 0xE7, 0xEB, 0xEF, 0xF3, 0xF7, 0xFB, 0xFF,
];

/// Block runs a type code can ask for.
const BLOCK_RUNS: [usize; 64] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41,
    42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 128, 256, 512, 1024, 2048,
];

const SMK_NODE: u32 = 0x8000_0000;
/// What ffmpeg's block loop goes on with when a color, map or pixel code cannot be read: the
/// error code `AVERROR_INVALIDDATA` (only the block type code is checked; the picture keeps the
/// blocks written from it).
const BAD_CODE: u32 = 0xBEBB_B1B7;
/// Deepest small tree (ffmpeg: `FFMIN(32, 3 * SMKTREE_BITS)`).
const TREE_MAX_DEPTH: usize = 27;
/// Deepest header ("big") tree.
const BIGTREE_MAX_DEPTH: usize = 500;

fn le32(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

// ---------------------------------------------------------------------------------------------
// Bit reader (little-endian bit order, zeros past the end as ffmpeg's padded buffers give)
// ---------------------------------------------------------------------------------------------

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    /// Bits not yet read (negative past the end).
    fn left(&self) -> i64 {
        (self.data.len() as i64).saturating_mul(8) - self.pos as i64
    }
    fn bit(&mut self) -> u32 {
        let b = self.data.get(self.pos >> 3).map_or(0, |&v| u32::from(v >> (self.pos & 7)) & 1);
        self.pos += 1;
        b
    }
    fn get(&mut self, n: u32) -> u32 {
        let mut v = 0u32;
        for i in 0..n {
            v |= self.bit() << i;
        }
        v
    }
}

// ---------------------------------------------------------------------------------------------
// Huffman trees
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Node {
    Leaf(u8),
    /// A branch; the left child follows it, the right child is at this index.
    Branch(usize),
}

/// A byte-valued prefix code stored as the tree the file gives (first bit 0: left).
struct SmallTree {
    nodes: Vec<Node>,
    leaves: usize,
}

impl SmallTree {
    /// Reads a tree (`smacker_decode_tree`).
    fn read(bits: &mut Bits<'_>) -> Result<Self> {
        let mut t = Self { nodes: Vec::new(), leaves: 0 };
        t.build(bits, 0)?;
        Ok(t)
    }
    fn build(&mut self, bits: &mut Bits<'_>, depth: usize) -> Result<()> {
        if depth > TREE_MAX_DEPTH {
            return Err(Error::Invalid("Huffman tree too deep"));
        }
        if bits.bit() == 0 {
            if self.leaves >= 256 {
                return Err(Error::Invalid("Huffman tree has too many leaves"));
            }
            if bits.left() < 8 {
                return Err(Error::Invalid("Huffman tree truncated"));
            }
            self.nodes.push(Node::Leaf(bits.get(8) as u8));
            self.leaves += 1;
            return Ok(());
        }
        let at = self.nodes.len();
        self.nodes.push(Node::Branch(0));
        self.build(bits, depth + 1)?;
        let right = self.nodes.len();
        if let Some(n) = self.nodes.get_mut(at) {
            *n = Node::Branch(right);
        }
        self.build(bits, depth + 1)
    }
    /// A tree of one value (no bits are read).
    fn constant(v: u8) -> Self {
        Self { nodes: vec![Node::Leaf(v)], leaves: 1 }
    }
    fn decode(&self, bits: &mut Bits<'_>) -> u8 {
        let mut i = 0usize;
        loop {
            match self.nodes.get(i) {
                Some(Node::Leaf(v)) => return *v,
                Some(Node::Branch(right)) => {
                    i = if bits.bit() == 1 { *right } else { i + 1 };
                }
                None => return 0,
            }
        }
    }
    /// A one-leaf tree reads no bits.
    fn is_constant(&self) -> bool {
        self.leaves <= 1
    }
}

/// A header tree of 16-bit values (`smacker_decode_header_tree`): the flat table ffmpeg walks,
/// with the three most-recent-value slots.
#[derive(Clone)]
struct Recode {
    tbl: Vec<u32>,
    last: [usize; 3],
}

struct BigCtx<'a> {
    current: usize,
    length: usize,
    values: Vec<u32>,
    v1: &'a SmallTree,
    v2: &'a SmallTree,
    escapes: [u32; 3],
    last: [Option<usize>; 3],
}

impl BigCtx<'_> {
    /// Stores `v` at `at`, growing the table as the tree is read (the declared size is only an
    /// upper bound, so a short tree cannot make a large allocation).
    fn set(&mut self, at: usize, v: u32) -> Result<()> {
        if at >= self.length + 3 {
            return Err(Error::Invalid("header tree larger than declared"));
        }
        if at >= self.values.len() {
            self.values.resize(at + 1, 0);
        }
        if let Some(slot) = self.values.get_mut(at) {
            *slot = v;
        }
        Ok(())
    }

    fn bigtree(&mut self, bits: &mut Bits<'_>, length: usize) -> Result<usize> {
        if length > BIGTREE_MAX_DEPTH {
            return Err(Error::Invalid("header tree too deep"));
        }
        if self.current >= self.length {
            return Err(Error::Invalid("header tree larger than declared"));
        }
        if bits.left() <= 0 {
            return Err(Error::Invalid("header tree truncated"));
        }
        if bits.bit() == 0 {
            let i1 = u32::from(self.v1.decode(bits));
            let i2 = u32::from(self.v2.decode(bits));
            let mut val = i1 | (i2 << 8);
            for k in 0..3 {
                if val == self.escapes[k] {
                    self.last[k] = Some(self.current);
                    val = 0;
                    break;
                }
            }
            self.set(self.current, val)?;
            self.current += 1;
            Ok(1)
        } else {
            let t = self.current;
            self.current += 1;
            let mut r = self.bigtree(bits, length + 1)?;
            self.set(t, SMK_NODE | r as u32)?;
            r += 1;
            let r_new = self.bigtree(bits, length + 1)?;
            Ok(r + r_new)
        }
    }
}

impl Recode {
    /// The "skipped tree" table: always 0.
    fn zero() -> Self {
        Self { tbl: vec![0, 0], last: [1, 1, 1] }
    }

    fn read(bits: &mut Bits<'_>, size: u32) -> Result<Self> {
        if size >= u32::MAX >> 4 {
            return Err(Error::Invalid("header tree size too large"));
        }
        let mut trees = [SmallTree::constant(0), SmallTree::constant(0)];
        for t in &mut trees {
            if bits.bit() == 0 {
                continue;
            }
            *t = SmallTree::read(bits)?;
            bits.bit();
        }
        let escapes = [bits.get(16), bits.get(16), bits.get(16)];
        let length = ((size as usize) + 3) >> 2;
        let mut ctx = BigCtx { current: 0, length, values: Vec::new(), v1: &trees[0], v2: &trees[1], escapes, last: [None; 3] };
        ctx.bigtree(bits, 0)?;
        bits.bit();
        let mut last = [0usize; 3];
        for (k, l) in last.iter_mut().enumerate() {
            *l = match ctx.last[k] {
                Some(v) => v,
                None => {
                    let v = ctx.current;
                    ctx.current += 1;
                    v
                }
            };
        }
        // the three most-recent-value slots live after the tree
        let need = last.iter().copied().max().map_or(0, |m| m + 1).max(ctx.current);
        if ctx.values.len() < need {
            ctx.values.resize(need, 0);
        }
        Ok(Self { tbl: ctx.values, last })
    }

    fn reset_last(&mut self) {
        for k in 0..3 {
            if let Some(s) = self.tbl.get_mut(self.last[k]) {
                *s = 0;
            }
        }
    }

    /// The next value with the history updated (`smk_get_code`); `None` when the data ends.
    fn get(&mut self, bits: &mut Bits<'_>) -> Option<u32> {
        let mut i = 0usize;
        loop {
            let v = *self.tbl.get(i)?;
            if v & SMK_NODE == 0 {
                break;
            }
            if bits.left() < 1 {
                return None;
            }
            if bits.bit() == 1 {
                i = i.checked_add((v & !SMK_NODE) as usize)?;
            }
            i += 1;
        }
        let v = *self.tbl.get(i)?;
        let [l0, l1, l2] = self.last;
        let cur = *self.tbl.get(l0)?;
        if v != cur {
            let prev = *self.tbl.get(l1)?;
            *self.tbl.get_mut(l2)? = prev;
            *self.tbl.get_mut(l1)? = cur;
            *self.tbl.get_mut(l0)? = v;
        }
        Some(v)
    }
}

// ---------------------------------------------------------------------------------------------
// Container
// ---------------------------------------------------------------------------------------------

/// How an audio track is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackKind {
    /// Huffman-packed (`smackaud`).
    Packed,
    /// Uncompressed PCM.
    Pcm,
    /// Bink audio (RDFT or DCT): not decoded here.
    Bink,
}

/// One audio track of a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Track {
    /// Sample rate, Hz.
    pub rate: u32,
    pub channels: u8,
    /// Bits per sample, 8 or 16.
    pub bits: u8,
    pub kind: TrackKind,
}

impl Track {
    /// Bytes of a chunk's leading length field that ffmpeg's demuxer keeps as part of the
    /// packet (the unpacked size; it also keeps it for 8-bit PCM, where it is data).
    fn has_duration_field(&self) -> bool {
        !(self.bits == 16 && self.kind == TrackKind::Pcm)
    }
    /// A sample (8-bit: unsigned code less 128) as a float in -1..1.
    pub fn to_f32(&self, v: i32) -> f32 {
        if self.bits == 16 { v as f32 / 32768.0 } else { v as f32 / 128.0 }
    }
}

/// Where a frame is in the file.
#[derive(Clone, Copy, Debug)]
pub struct FrameLoc {
    /// Offset of the frame's first byte.
    pub offset: usize,
    /// Bytes of the frame (the stored size without its two low flag bits).
    pub len: usize,
    /// The frame is a key frame (a seek point).
    pub key: bool,
    /// The frame flags byte: bit 0 palette update, bit `1 + i` audio track `i`.
    pub flags: u8,
    /// The file ends inside the frame: `len` is what is there, and the video is lost (the
    /// palette and the audio chunks that are whole still count, as in ffmpeg's demuxer).
    pub truncated: bool,
}

/// A parsed Smacker file header and frame table.
#[derive(Clone, Debug)]
pub struct Smk {
    /// SMK4 (three full-block modes) rather than SMK2.
    pub v4: bool,
    pub width: u32,
    pub height: u32,
    /// Frames in the file, with the ring frame.
    pub frames: usize,
    /// The header's frame-duration field.
    pub pts_inc: i32,
    pub flags: u32,
    /// Audio tracks by slot; `None` where the file has none.
    pub tracks: [Option<Track>; MAX_TRACKS],
    pub frame_table: Vec<FrameLoc>,
    tree_sizes: [u32; 4],
    tree_data: Vec<u8>,
}

/// One frame cut into its parts.
#[derive(Clone, Debug, Default)]
pub struct Parts<'a> {
    /// The palette update chunk, from its second byte to the end of the frame (the update is
    /// read from there as it is coded, not bounded by its own size).
    pub palette: Option<&'a [u8]>,
    /// Each track's chunk without its size field.
    pub audio: [Option<&'a [u8]>; MAX_TRACKS],
    /// The video data; `None` when the frame is damaged before it.
    pub video: Option<&'a [u8]>,
    /// The frame is damaged: ffmpeg's demuxer delivers what comes before the damage.
    pub damaged: bool,
}

/// Whether `head` starts a Smacker file (SMK2 or SMK4).
pub fn probe(head: &[u8]) -> bool {
    matches!(head.get(..4), Some(b"SMK2" | b"SMK4"))
}

fn valid_size(width: u32, height: u32) -> bool {
    width != 0 && height != 0 && u64::from(width) * u64::from(height) <= MAX_PIXELS && width <= 65_535 && height <= 65_535
}

impl Smk {
    /// Reads the header and the frame table of `data`.
    pub fn parse(data: &[u8]) -> Result<Self> {
        if !probe(data) {
            return Err(Error::NotSmacker);
        }
        if data.len() < HEADER_BYTES {
            return Err(Error::Invalid("file shorter than its header"));
        }
        let v4 = data.get(3) == Some(&b'4');
        let rd = |at: usize| le32(data, at).ok_or(Error::Invalid("truncated header"));
        let (width, height) = (rd(4)?, rd(8)?);
        let mut frames = rd(12)?;
        let pts_inc = rd(16)? as i32;
        let flags = rd(20)?;
        if pts_inc > i32::MAX / 100 || pts_inc == i32::MIN {
            return Err(Error::Invalid("frame duration out of range"));
        }
        if pts_inc == 0 {
            return Err(Error::Invalid("frame duration is zero"));
        }
        if flags & FLAG_RING_FRAME != 0 {
            frames = frames.saturating_add(1);
        }
        if frames > MAX_FRAMES {
            return Err(Error::Invalid("too many frames"));
        }
        if !valid_size(width, height) {
            return Err(Error::Unsupported("picture size"));
        }
        let tree_bytes = rd(52)?;
        let tree_sizes = [rd(56)?, rd(60)?, rd(64)?, rd(68)?];
        let mut tracks = [None; MAX_TRACKS];
        for (i, slot) in tracks.iter_mut().enumerate() {
            let at = 72 + i * 4;
            let rate = rd(at)? & 0x00FF_FFFF;
            let aflag = data.get(at + 3).copied().ok_or(Error::Invalid("truncated header"))?;
            if rate == 0 {
                continue;
            }
            let kind = if aflag & (AUD_BINKAUD | AUD_USEDCT) != 0 {
                TrackKind::Bink
            } else if aflag & AUD_PACKED != 0 {
                TrackKind::Packed
            } else {
                TrackKind::Pcm
            };
            *slot = Some(Track { rate, channels: if aflag & AUD_STEREO != 0 { 2 } else { 1 }, bits: if aflag & AUD_16BITS != 0 { 16 } else { 8 }, kind });
        }
        let frames = frames as usize;
        let table_end = HEADER_BYTES
            .checked_add(frames.checked_mul(5).ok_or(Error::Invalid("frame table size"))?)
            .and_then(|v| v.checked_add(tree_bytes as usize))
            .ok_or(Error::Invalid("frame table size"))?;
        if table_end > data.len() {
            return Err(Error::Invalid("frame table or trees beyond the end of the file"));
        }
        let sizes_at = HEADER_BYTES;
        let flags_at = sizes_at + frames * 4;
        let trees_at = flags_at + frames;
        let tree_data = data.get(trees_at..table_end).ok_or(Error::Invalid("trees beyond the end of the file"))?.to_vec();
        let mut frame_table = Vec::with_capacity(frames);
        let mut offset = table_end;
        for i in 0..frames {
            let size = rd(sizes_at + i * 4)?;
            let len = (size & !3) as usize;
            let end = offset.checked_add(len).ok_or(Error::Invalid("frame size"))?;
            let key = i == 0 || size & 1 != 0;
            let flags = data.get(flags_at + i).copied().unwrap_or(0);
            // a frame cut short by the end of the file ends the table, keeping what is there
            if end > data.len() {
                if offset < data.len() {
                    frame_table.push(FrameLoc { offset, len: data.len() - offset, key, flags, truncated: true });
                }
                break;
            }
            frame_table.push(FrameLoc { offset, len, key, flags, truncated: false });
            offset = end;
        }
        Ok(Self { v4, width, height, frames: frame_table.len(), pts_inc, flags, tracks, frame_table, tree_sizes, tree_data })
    }

    /// The frames that have a picture: the file's frames, less a last one the file ends inside.
    pub fn video_frames(&self) -> usize {
        self.frames - usize::from(self.frame_table.last().is_some_and(|f| f.truncated))
    }

    /// The frame rate as numerator and denominator (frames per second).
    pub fn frame_rate(&self) -> (i64, i64) {
        // 100 000 ticks a second; a positive field counts milliseconds, a negative one counts
        // ticks
        let ticks = if self.pts_inc < 0 { i64::from(self.pts_inc).abs() } else { i64::from(self.pts_inc) * 100 };
        let g = gcd(100_000, ticks.max(1));
        (100_000 / g, ticks.max(1) / g)
    }

    /// The pixel aspect ratio: 1:2 when the lines are doubled or interlaced.
    pub fn pixel_aspect(&self) -> (u32, u32) {
        (1, 1 + u32::from(self.flags & (FLAG_Y_INTERLACE | FLAG_Y_DOUBLE) != 0))
    }

    /// The bytes of frame `i` in `data` (the file).
    pub fn frame_bytes<'a>(&self, data: &'a [u8], i: usize) -> Option<&'a [u8]> {
        let loc = self.frame_table.get(i)?;
        data.get(loc.offset..loc.offset.saturating_add(loc.len))
    }

    /// Cuts frame `i` of `data` (the file) into its parts (ffmpeg's `smacker_read_packet`).
    pub fn split<'a>(&self, data: &'a [u8], i: usize) -> Parts<'a> {
        match self.frame_bytes(data, i) {
            Some(frame) => self.split_frame(i, frame),
            None => Parts { damaged: true, ..Parts::default() },
        }
    }

    /// Cuts the bytes of frame `i` (as [`frame_bytes`](Self::frame_bytes) gives them) into its parts.
    pub fn split_frame<'a>(&self, i: usize, frame: &'a [u8]) -> Parts<'a> {
        let mut parts = Parts::default();
        let Some(loc) = self.frame_table.get(i) else {
            parts.damaged = true;
            return parts;
        };
        let mut pos = 0usize;
        if loc.flags & SMACKER_PAL != 0 {
            let size = frame.first().map_or(0, |&b| usize::from(b) * 4);
            let pal = frame.get(1..).unwrap_or_default();
            if size == 0 || size > frame.len() {
                parts.damaged = true;
                return parts;
            }
            // what the update changed before it failed stays in the demuxer's palette
            parts.palette = Some(pal);
            if apply_palette(&[0; 768], pal).1.is_err() {
                parts.damaged = true;
                return parts;
            }
            pos = size;
        }
        for t in 0..MAX_TRACKS {
            if loc.flags >> 1 & (1 << t) == 0 {
                continue;
            }
            let dur = self.tracks[t].is_some_and(|k| k.has_duration_field()) as usize * 4;
            let remaining = frame.len() - pos;
            let size = le32(frame, pos).map_or(0, |v| v as usize);
            if size < 4 + dur || size > remaining {
                parts.damaged = true;
                return parts;
            }
            parts.audio[t] = frame.get(pos + 4..pos + size);
            pos += size;
        }
        parts.video = if loc.truncated { None } else { frame.get(pos..) };
        parts
    }
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// Applies a palette update to `old` (the demuxer's palette handling): runs of kept entries, of
/// entries copied from elsewhere in the old palette, and of new 6-bit entries. The palette comes
/// back whole even when the update fails, with the entries changed before the failure.
fn apply_palette(old: &[u8; 768], chunk: &[u8]) -> ([u8; 768], Result<()>) {
    let mut pal = *old;
    let r = apply_palette_into(old, chunk, &mut pal);
    (pal, r)
}

fn apply_palette_into(old: &[u8; 768], chunk: &[u8], pal: &mut [u8; 768]) -> Result<()> {
    let mut src = chunk.iter().copied();
    let mut next = || src.next().ok_or(Error::Invalid("palette update truncated"));
    let (mut sz, mut at) = (0usize, 0usize);
    while sz < 256 {
        let t = next()?;
        if t & 0x80 != 0 {
            let n = usize::from(t & 0x7F) + 1;
            sz += n;
            at += n * 3;
        } else if t & 0x40 != 0 {
            let off = usize::from(next()?);
            let n = usize::from(t & 0x3F) + 1;
            if off + n > 0x100 {
                return Err(Error::Invalid("palette copy beyond the palette"));
            }
            let mut from = off * 3;
            let mut k = n;
            while k > 0 && sz < 256 {
                for c in 0..3 {
                    let v = *old.get(from + c).ok_or(Error::Invalid("palette copy beyond the palette"))?;
                    if let Some(d) = pal.get_mut(at + c) {
                        *d = v;
                    }
                }
                at += 3;
                from += 3;
                sz += 1;
                k -= 1;
            }
        } else {
            let r = SMK_PAL[usize::from(t)];
            let g = SMK_PAL[usize::from(next()? & 0x3F)];
            let b = SMK_PAL[usize::from(next()? & 0x3F)];
            if let Some(d) = pal.get_mut(at..at + 3) {
                d.copy_from_slice(&[r, g, b]);
            }
            at += 3;
            sz += 1;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Video
// ---------------------------------------------------------------------------------------------

/// The picture and palette a decoder carries from frame to frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pic: Vec<u8>,
    pal: [u8; 768],
}

/// The video decoder. Each frame is coded against the previous picture (blocks can be left as
/// they were), so frames are decoded in order; a [`Snapshot`] lets a caller come back to a frame.
#[derive(Clone)]
pub struct Decoder {
    width: usize,
    height: usize,
    v4: bool,
    mmap: Recode,
    mclr: Recode,
    full: Recode,
    kind: Recode,
    pic: Vec<u8>,
    pal: [u8; 768],
}

impl Decoder {
    /// A decoder for the stream of `smk`: reads the four header trees.
    pub fn new(smk: &Smk) -> Result<Self> {
        let mut bits = Bits::new(&smk.tree_data);
        let mut tables = Vec::with_capacity(4);
        let mut skipped = 0;
        for &size in &smk.tree_sizes {
            if bits.bit() == 0 {
                skipped += 1;
                tables.push(Recode::zero());
            } else {
                tables.push(Recode::read(&mut bits, size)?);
            }
        }
        if skipped == 4 || bits.left() < 0 {
            return Err(Error::Invalid("header trees"));
        }
        let mut it = tables.into_iter();
        let (Some(mmap), Some(mclr), Some(full), Some(kind)) = (it.next(), it.next(), it.next(), it.next()) else {
            return Err(Error::Invalid("header trees"));
        };
        if !valid_size(smk.width, smk.height) {
            return Err(Error::Unsupported("picture size"));
        }
        let (width, height) = (smk.width as usize, smk.height as usize);
        Ok(Self { width, height, v4: smk.v4, mmap, mclr, full, kind, pic: vec![0; width * height], pal: [0; 768] })
    }

    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }

    /// The picture and palette now.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot { pic: self.pic.clone(), pal: self.pal }
    }

    /// Goes back to `s` (taken from a decoder of the same file).
    pub fn restore(&mut self, s: &Snapshot) {
        if s.pic.len() == self.pic.len() {
            self.pic.clone_from(&s.pic);
            self.pal = s.pal;
        }
    }

    /// The picture as RGBA, 4 bytes a pixel.
    pub fn rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pic.len() * 4);
        for &i in &self.pic {
            let p = usize::from(i) * 3;
            let c = self.pal.get(p..p + 3).unwrap_or(&[0, 0, 0]);
            out.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
        out
    }

    /// The indexed picture, `width × height` bytes, and the palette (RGB triples).
    pub fn indexed(&self) -> (&[u8], &[u8; 768]) {
        (&self.pic, &self.pal)
    }

    /// Applies a palette update of a frame that has no video to decode (the demuxer keeps the
    /// palette even when the frame is damaged after it).
    pub fn update_palette(&mut self, palette: &[u8]) -> Result<()> {
        let (pal, r) = apply_palette(&self.pal, palette);
        self.pal = pal;
        r
    }

    /// Decodes one frame: `palette` is the frame's palette update, `video` its video data. On
    /// an error the picture keeps what was decoded before it.
    pub fn decode(&mut self, palette: Option<&[u8]>, video: &[u8]) -> Result<()> {
        if let Some(p) = palette {
            let (pal, r) = apply_palette(&self.pal, p);
            self.pal = pal;
            r?;
        }
        if video.is_empty() {
            return Err(Error::Invalid("video data too small"));
        }
        self.mmap.reset_last();
        self.mclr.reset_last();
        self.full.reset_last();
        self.kind.reset_last();
        let mut bits = Bits::new(video);
        let (w, h) = (self.width, self.height);
        let bw = w >> 2;
        let blocks = bw * (h >> 2);
        let mut blk = 0usize;
        let err = || Error::Invalid("video data truncated");
        while blk < blocks {
            let ty = self.kind.get(&mut bits).ok_or_else(err)?;
            let mut run = BLOCK_RUNS[((ty >> 2) & 0x3F) as usize];
            match ty & 3 {
                0 => {
                    while run > 0 && blk < blocks {
                        run -= 1;
                        let clr = self.mclr.get(&mut bits).unwrap_or(BAD_CODE);
                        let map = self.mmap.get(&mut bits).unwrap_or(BAD_CODE);
                        let (hi, lo) = ((clr >> 8) as u8, clr as u8);
                        let at = (blk / bw) * 4 * w + (blk % bw) * 4;
                        let mut map = map;
                        for row in 0..4 {
                            for x in 0..4 {
                                self.put(at + row * w + x, if map & (1 << x) != 0 { hi } else { lo });
                            }
                            map >>= 4;
                        }
                        blk += 1;
                    }
                }
                1 => {
                    let mut mode = 0;
                    if self.v4 {
                        if bits.left() < 1 {
                            return Err(err());
                        }
                        if bits.bit() == 1 {
                            mode = 1;
                        } else if bits.bit() == 1 {
                            mode = 2;
                        }
                    }
                    while run > 0 && blk < blocks {
                        run -= 1;
                        let at = (blk / bw) * 4 * w + (blk % bw) * 4;
                        match mode {
                            0 => {
                                for row in 0..4 {
                                    let p = self.full.get(&mut bits).unwrap_or(BAD_CODE);
                                    self.put(at + row * w + 2, p as u8);
                                    self.put(at + row * w + 3, (p >> 8) as u8);
                                    let p = self.full.get(&mut bits).unwrap_or(BAD_CODE);
                                    self.put(at + row * w, p as u8);
                                    self.put(at + row * w + 1, (p >> 8) as u8);
                                }
                            }
                            1 => {
                                for half in 0..2 {
                                    let p = self.full.get(&mut bits).unwrap_or(BAD_CODE);
                                    for row in 0..2 {
                                        let o = at + (half * 2 + row) * w;
                                        self.put(o, p as u8);
                                        self.put(o + 1, p as u8);
                                        self.put(o + 2, (p >> 8) as u8);
                                        self.put(o + 3, (p >> 8) as u8);
                                    }
                                }
                            }
                            _ => {
                                for half in 0..2 {
                                    let p2 = self.full.get(&mut bits).unwrap_or(BAD_CODE);
                                    let p1 = self.full.get(&mut bits).unwrap_or(BAD_CODE);
                                    for row in 0..2 {
                                        let o = at + (half * 2 + row) * w;
                                        self.put(o, p1 as u8);
                                        self.put(o + 1, (p1 >> 8) as u8);
                                        self.put(o + 2, p2 as u8);
                                        self.put(o + 3, (p2 >> 8) as u8);
                                    }
                                }
                            }
                        }
                        blk += 1;
                    }
                }
                2 => {
                    // skip: the blocks keep the previous picture
                    blk += run.min(blocks - blk);
                }
                _ => {
                    let col = (ty >> 8) as u8;
                    while run > 0 && blk < blocks {
                        run -= 1;
                        let at = (blk / bw) * 4 * w + (blk % bw) * 4;
                        for row in 0..4 {
                            for x in 0..4 {
                                self.put(at + row * w + x, col);
                            }
                        }
                        blk += 1;
                    }
                }
            }
        }
        Ok(())
    }

    fn put(&mut self, at: usize, v: u8) {
        if let Some(p) = self.pic.get_mut(at) {
            *p = v;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Audio
// ---------------------------------------------------------------------------------------------

/// The samples one audio chunk decodes to, interleaved; 8-bit samples are the unsigned code
/// less 128.
pub fn decode_audio(track: &Track, chunk: &[u8]) -> Result<Vec<i32>> {
    decode_audio_checked(track, chunk, true)
}

/// [`decode_audio`] as the ffmpeg releases before n8 do it: without the check that a packed
/// chunk holds its first samples (for the tests against such an ffmpeg).
pub fn decode_audio_lenient(track: &Track, chunk: &[u8]) -> Result<Vec<i32>> {
    decode_audio_checked(track, chunk, false)
}

fn decode_audio_checked(track: &Track, chunk: &[u8], first_samples_check: bool) -> Result<Vec<i32>> {
    let ch = usize::from(track.channels.max(1));
    match track.kind {
        TrackKind::Bink => Err(Error::Unsupported("Bink audio")),
        TrackKind::Pcm if track.bits == 16 => {
            Ok(chunk.chunks_exact(2 * ch).flat_map(|f| f.chunks_exact(2)).map(|b| i32::from(i16::from_le_bytes([b[0], b[1]]))).collect())
        }
        TrackKind::Pcm => Ok(chunk.chunks_exact(ch).flatten().map(|&b| i32::from(b) - 128).collect()),
        TrackKind::Packed => decode_packed(track, chunk, first_samples_check),
    }
}

/// The samples per channel a chunk stands for in the file's timeline.
pub fn chunk_samples(track: &Track, chunk: &[u8]) -> usize {
    let ch = usize::from(track.channels.max(1));
    let bytes = usize::from(track.bits / 8).max(1);
    match track.kind {
        TrackKind::Bink => 0,
        TrackKind::Pcm => chunk.len() / (ch * bytes),
        TrackKind::Packed => {
            // a chunk that says "no data" is skipped one byte at a time, as `decode_packed` does,
            // and the first chunk that has data gives the length (the declared one when none has)
            let declared = le32(chunk, 0).map_or(0, |v| v as usize / (ch * bytes));
            let mut buf = chunk;
            while buf.len() > 4 {
                let Some(unp) = le32(buf, 0).map(|v| v as usize) else { break };
                if unp > MAX_PACKED_BYTES {
                    break;
                }
                if Bits::new(buf.get(4..).unwrap_or_default()).bit() == 1 {
                    return unp / (ch * bytes);
                }
                buf = buf.get(1..).unwrap_or_default();
            }
            declared
        }
    }
}

/// Most bytes one packed audio chunk may unpack to.
pub const MAX_PACKED_BYTES: usize = 1 << 24;

fn decode_packed(track: &Track, buf: &[u8], first_samples_check: bool) -> Result<Vec<i32>> {
    // A chunk that says "no data" is consumed one byte long, and ffmpeg's decoding loop feeds the
    // rest of the packet to the decoder again as a packet of its own (the bytes after the first
    // are then read as an unpacked size and a bitstream).
    let mut buf = buf;
    loop {
        match decode_packed_once(track, buf, first_samples_check)? {
            Some(out) => return Ok(out),
            None => buf = buf.get(1..).unwrap_or_default(),
        }
    }
}

/// One decoder call: `None` for a chunk without data.
fn decode_packed_once(track: &Track, buf: &[u8], first_samples_check: bool) -> Result<Option<Vec<i32>>> {
    if buf.len() <= 4 {
        return Err(Error::Invalid("audio chunk too small"));
    }
    let unp = le32(buf, 0).ok_or(Error::Invalid("audio chunk too small"))? as usize;
    if unp > MAX_PACKED_BYTES {
        return Err(Error::Invalid("audio chunk too big"));
    }
    let mut bits = Bits::new(&buf[4..]);
    if bits.bit() == 0 {
        return Ok(None);
    }
    let stereo = bits.bit() as usize;
    let b16 = bits.bit() as usize;
    let ch = usize::from(track.channels);
    if stereo != usize::from(ch != 1) {
        return Err(Error::Invalid("audio channel count differs from the file header"));
    }
    if (b16 == 1) != (track.bits == 16) {
        return Err(Error::Invalid("audio sample size differs from the file header"));
    }
    if !unp.is_multiple_of(ch * (b16 + 1)) {
        return Err(Error::Invalid("audio chunk is not a whole number of samples"));
    }
    let n_trees = 1usize << (b16 + stereo);
    let mut trees = Vec::with_capacity(n_trees);
    for _ in 0..n_trees {
        bits.bit();
        let t = SmallTree::read(&mut bits)?;
        bits.bit();
        trees.push(t);
    }
    if first_samples_check && bits.left() < (stereo as i64 + 1) * (b16 as i64 + 1) * 8 {
        return Err(Error::Invalid("audio chunk too short for its first samples"));
    }
    let value = |t: &SmallTree, bits: &mut Bits<'_>| i64::from(t.decode(bits));
    let mut out = Vec::new();
    if b16 == 1 {
        let total = unp / 2;
        let mut pred = [0u32; 2];
        for i in (0..=stereo).rev() {
            let v = bits.get(16);
            pred[i] = ((v & 0xFF) << 8) | (v >> 8);
        }
        for p in pred.iter().take(stereo + 1) {
            out.push(i32::from(*p as u16 as i16));
        }
        let any_tree = trees.iter().take(2).any(|t| !t.is_constant()) || trees.iter().skip(2 * stereo).take(2).any(|t| !t.is_constant());
        let consts: Vec<i64> = trees.iter().map(|t| value(t, &mut Bits::new(&[]))).collect();
        let mut i = stereo + 1;
        if any_tree {
            while i < total {
                if bits.left() < 0 {
                    return Err(Error::Invalid("audio data truncated"));
                }
                let idx = 2 * (i & stereo);
                let lo = trees.get(idx).map_or(0, |t| value(t, &mut bits));
                let hi = trees.get(idx + 1).map_or(0, |t| value(t, &mut bits));
                let ch_i = idx / 2;
                pred[ch_i] = pred[ch_i].wrapping_add((lo | (hi << 8)) as u32);
                out.push(i32::from(pred[ch_i] as u16 as i16));
                i += 1;
            }
        } else if stereo == 1 {
            let v0 = (256 * consts.get(1).copied().unwrap_or(0) + consts.first().copied().unwrap_or(0)) as u32;
            let v1 = (256 * consts.get(3).copied().unwrap_or(0) + consts.get(2).copied().unwrap_or(0)) as u32;
            while i < total {
                pred[0] = pred[0].wrapping_add(v0);
                pred[1] = pred[1].wrapping_add(v1);
                out.push(i32::from(pred[0] as u16 as i16));
                out.push(i32::from(pred[1] as u16 as i16));
                i += 2;
            }
        } else {
            let v0 = (256 * consts.get(1).copied().unwrap_or(0) + consts.first().copied().unwrap_or(0)) as u32;
            while i < total {
                pred[0] = pred[0].wrapping_add(v0);
                out.push(i32::from(pred[0] as u16 as i16));
                i += 1;
            }
        }
        out.truncate(total);
    } else {
        let total = unp;
        let mut pred = [0u32; 2];
        for i in (0..=stereo).rev() {
            pred[i] = bits.get(8);
        }
        for p in pred.iter().take(stereo + 1) {
            out.push(i32::from(*p as u8) - 128);
        }
        let mut i = stereo + 1;
        while i < total {
            if bits.left() < 0 {
                return Err(Error::Invalid("audio data truncated"));
            }
            let idx = i & stereo;
            let v = trees.get(idx).map_or(0, |t| value(t, &mut bits)) as u32;
            pred[idx] = pred[idx].wrapping_add(v);
            out.push(i32::from(pred[idx] as u8) - 128);
            i += 1;
        }
        out.truncate(total);
    }
    Ok(Some(out))
}
