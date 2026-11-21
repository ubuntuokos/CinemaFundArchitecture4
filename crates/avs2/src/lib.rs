// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from davs2 1.7 (`source/common/davs2.cc`, `bitstream.cc`), Copyright (C) 2018~ VCL, NELVT, Peking
// University (Falei Luo et al.), GPL-2.0-or-later.
//! AVS2-P2 (IEEE 1857.4 / GB/T 33475.2) video decoder: a Rust port of davs2 1.7.

mod aec;
mod bits;
mod cu;
mod decoder;
mod filter;
mod header;
mod recon;

pub use header::FrameType;

/// Decoding errors. Damaged input never panics; it yields one of these.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The bitstream ended inside a syntax element.
    #[error("AVS2 bitstream truncated")]
    Truncated,
    /// A syntax element has a value the standard forbids.
    #[error("invalid AVS2 bitstream: {0}")]
    Invalid(&'static str),
    /// A valid feature this decoder does not implement.
    #[error("unsupported AVS2 feature: {0}")]
    Unsupported(&'static str),
}

/// A decoded picture, cropped to the picture size, in output order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Bits per sample of the planes (8 or 10).
    pub bit_depth: u32,
    /// Y, Cb, Cr (4:2:0; chroma is `ceil(w/2) × ceil(h/2)`), one `u16` per sample.
    pub planes: [Vec<u16>; 3],
    /// Picture order count.
    pub poc: i32,
    /// The tag passed with the packet that held the picture.
    pub tag: i64,
}

/// Stream parameters from the first sequence header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamInfo {
    pub profile: u32,
    pub level: u32,
    pub width: u32,
    pub height: u32,
    /// Bits per output sample (8 or 10).
    pub bit_depth: u32,
    /// `aspect_ratio` code (1 = square samples, 2 = 4:3, 3 = 16:9, 4 = 2.21:1).
    pub aspect_ratio: u32,
    /// Frame rate as a fraction.
    pub frame_rate: (u32, u32),
    /// Pictures are output in decoding order.
    pub low_delay: bool,
}

fn info_of(s: &header::Sequence) -> StreamInfo {
    StreamInfo {
        profile: s.profile_id,
        level: s.level_id,
        width: s.width,
        height: s.height,
        bit_depth: s.sample_bit_depth,
        aspect_ratio: s.aspect_ratio,
        frame_rate: s.frame_rate(),
        low_delay: s.low_delay,
    }
}

/// Iterates the start codes of `data`: yields the code byte and the offset of its `00 00 01`.
fn codes(data: &[u8]) -> impl Iterator<Item = (u8, usize)> + '_ {
    data.windows(4).enumerate().filter_map(|(i, w)| (w[0] == 0 && w[1] == 0 && w[2] == 1).then_some((w[3], i)))
}

/// Whether `code` (the byte after `00 00 01`) ends a packet before it ([`split_packets`]).
pub fn is_packet_start(code: u8) -> bool {
    matches!(code, 0xB0 | 0xB1 | 0xB3 | 0xB6 | 0xB7)
}

/// Whether `code` starts a picture.
pub fn is_picture_start(code: u8) -> bool {
    matches!(code, 0xB3 | 0xB6)
}

/// True when `data` looks like an AVS2 video elementary stream: a sequence header of an AVS2
/// profile (Main 0x20, Main 10 0x22, Main picture 0x12) followed by pictures.
pub fn probe(data: &[u8]) -> bool {
    let mut seq = 0u32;
    let mut pic = 0u32;
    for (c, at) in codes(data) {
        match c {
            0xB0 => {
                let profile = data.get(at + 4).copied().unwrap_or(0);
                if !matches!(profile, 0x20 | 0x22 | 0x12) {
                    return false;
                }
                // the header up to the next start code (or the end of what was given)
                let body = data.get(at + 4..).unwrap_or_default();
                let end = codes(body).next().map_or(body.len(), |(_, k)| k);
                if end < body.len() && !header::Sequence::plausible(body.get(..end).unwrap_or_default()) {
                    return false;
                }
                seq += 1;
            }
            0xB3 | 0xB6 => pic += 1,
            0xB4 | 0xB8..=0xFF => return false,
            _ => {}
        }
    }
    seq > 0 && pic > 0
}

/// Reads the first sequence header of a stream.
pub fn stream_info(data: &[u8]) -> Result<StreamInfo, Error> {
    let (_, at) = codes(data).find(|&(c, _)| c == 0xB0).ok_or(Error::Invalid("no sequence header"))?;
    let s = header::Sequence::parse(data.get(at + 4..).unwrap_or_default())?;
    Ok(info_of(&s))
}

/// Splits an elementary stream into packets as davs2 does: a packet runs from a picture start code
/// to the next picture, sequence header, sequence end or video edit code. Data before the first
/// picture (the sequence header) forms its own packet.
pub fn split_packets(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (c, at) in codes(data) {
        if is_packet_start(c) && at > start {
            if let Some(p) = data.get(start..at) {
                out.push(p);
            }
            start = at;
        }
    }
    if let Some(p) = data.get(start..)
        && !p.is_empty()
    {
        out.push(p);
    }
    out
}

/// What indexing a stream needs from one picture header ([`SequenceHeader::picture`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PictureInfo {
    pub kind: FrameType,
    /// `coding_order_index`, modulo 256.
    pub coi: u32,
    /// `picture_output_delay` (0 in low-delay streams).
    pub display_delay: u32,
    /// The reference pictures, as distances back in coding order.
    pub refs: Vec<u32>,
}

/// A parsed sequence header, for reading picture headers without decoding.
#[derive(Clone, Debug)]
pub struct SequenceHeader {
    seq: header::Sequence,
}

impl SequenceHeader {
    /// Reads the first sequence header of `data`.
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        let (_, at) = codes(data).find(|&(c, _)| c == 0xB0).ok_or(Error::Invalid("no sequence header"))?;
        Ok(Self { seq: header::Sequence::parse(data.get(at + 4..).unwrap_or_default())? })
    }

    pub fn info(&self) -> StreamInfo {
        info_of(&self.seq)
    }

    /// `picture_reorder_delay`: how far the display order lags the coding order.
    pub fn reorder_delay(&self) -> u32 {
        self.seq.picture_reorder_delay
    }

    /// The header of the picture `packet` starts with (or holds), if it has one that parses.
    pub fn picture(&self, packet: &[u8]) -> Option<PictureInfo> {
        let (code, at) = codes(packet).find(|&(c, _)| is_picture_start(c))?;
        let body = packet.get(at + 4..)?;
        let (pic, _) = header::Picture::parse(body, code == 0xB3, &self.seq, &Default::default()).ok()?;
        let refs = pic.rps.ref_pic.iter().take(pic.rps.num_of_ref).copied().collect();
        Some(PictureInfo { kind: pic.frame_type, coi: pic.coi, display_delay: pic.display_delay, refs })
    }
}

/// An AVS2 decoder fed packets (as [`split_packets`] makes them).
#[derive(Default)]
pub struct Decoder {
    ctx: decoder::Ctx,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// The current sequence parameters (after the first sequence header).
    pub fn info(&self) -> Option<StreamInfo> {
        self.ctx.seq.as_ref().map(info_of)
    }

    /// Decodes one packet; returns the pictures that are due for output, in POC order.
    pub fn decode(&mut self, packet: &[u8], tag: i64) -> Result<Vec<Frame>, Error> {
        let res = self.ctx.decode_packet(packet, tag);
        let mut out = Vec::new();
        while let Some(p) = self.ctx.next_output(false) {
            out.push(self.frame(&p));
        }
        res.map(|()| out)
    }

    /// Drains the pictures still waiting for output at the end of the stream.
    pub fn flush(&mut self) -> Vec<Frame> {
        let mut out = Vec::new();
        while let Some(p) = self.ctx.next_output(true) {
            out.push(self.frame(&p));
        }
        out
    }

    /// Forgets the reference pictures before a seek: decoding resumes at the next I picture.
    pub fn reset(&mut self) {
        self.ctx.reset();
    }

    fn frame(&self, p: &decoder::OutPic) -> Frame {
        let Some(s) = self.ctx.seq.as_ref() else {
            return Frame { width: 0, height: 0, bit_depth: 8, planes: Default::default(), poc: p.poc, tag: p.tag };
        };
        let (w, h) = (s.width as usize, s.height as usize);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let out_depth = s.sample_bit_depth;
        let shift = p.bit_depth.saturating_sub(out_depth);
        let max = (1u32 << out_depth) - 1;
        let conv = |v: u16| -> u16 { if shift == 0 { v } else { ((u32::from(v) + (1 << (shift - 1))) >> shift).min(max) as u16 } };
        let crop = |pl: &recon::Plane, w: usize, h: usize| -> Vec<u16> {
            let stride = pl.w.max(1) as usize;
            let mut v = Vec::with_capacity(w * h);
            for row in pl.data.chunks(stride).take(h) {
                v.extend(row.iter().take(w).map(|&x| conv(x)));
            }
            v
        };
        let f = &p.frame.planes;
        Frame {
            width: s.width,
            height: s.height,
            bit_depth: out_depth,
            planes: [crop(&f[0], w, h), crop(&f[1], cw, ch), crop(&f[2], cw, ch)],
            poc: p.poc,
            tag: p.tag,
        }
    }
}
