// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from uavs3d 1.2 (commit 0e20d2c2), Copyright (c) 2018-2022 Peking University Shenzhen Graduate School,
// Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation, BSD 3-clause licence.
//! AVS3-P2 (IEEE 1857.10 / GB/T 33475.2) video decoder: a Rust port of uavs3d 1.2.

mod bits;
mod blocks;
mod core;
mod deblock;
mod decoder;
mod header;
mod inter;
mod intra;
mod lbac;
mod loopfilter;
mod mv;
mod parse;
mod recon;
mod scan;
mod tables;
mod transform;

/// Decoding errors. Damaged input never panics; it yields one of these.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The bitstream ended inside a syntax element.
    #[error("AVS3 bitstream truncated")]
    Truncated,
    /// A syntax element has a value the standard forbids.
    #[error("invalid AVS3 bitstream: {0}")]
    Invalid(&'static str),
    /// A valid feature this decoder does not implement.
    #[error("unsupported AVS3 feature: {0}")]
    Unsupported(&'static str),
}

/// A decoded picture, cropped to the picture size, in output order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Bits per sample of the planes (8 or 10).
    pub bit_depth: u32,
    /// Y, Cb, Cr (4:2:0; chroma is `ceil(w/2) x ceil(h/2)`), one `u16` per sample.
    pub planes: [Vec<u16>; 3],
    /// Picture order count.
    pub poc: i64,
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
        width: s.horizontal_size,
        height: s.vertical_size,
        bit_depth: s.output_bit_depth(),
        aspect_ratio: s.aspect_ratio,
        frame_rate: s.frame_rate(),
        low_delay: s.low_delay,
    }
}

/// Whether `code` (the byte after `00 00 01`) ends a packet before it ([`split_packets`]).
pub fn is_packet_start(code: u8) -> bool {
    matches!(code, 0xB0 | 0xB1 | 0xB3 | 0xB6 | 0xB7)
}

/// Whether `code` starts a picture.
pub fn is_picture_start(code: u8) -> bool {
    matches!(code, 0xB3 | 0xB6)
}

fn parse_seq_at(data: &[u8], at: usize) -> Result<header::Sequence, Error> {
    let end = bits::start_codes(data.get(at + 3..).unwrap_or_default()).next().map_or(data.len(), |(_, k)| at + 3 + k);
    let dem = bits::demulate(data.get(at + 3..end).unwrap_or_default());
    header::Sequence::parse(dem.get(1..).unwrap_or_default())
}

/// True when `data` looks like an AVS3 video elementary stream: a sequence header of an AVS3
/// profile (0x20, 0x22) that parses strictly (marker bits), followed by pictures.
pub fn probe(data: &[u8]) -> bool {
    let mut seq = 0u32;
    let mut pic = 0u32;
    for (c, at) in bits::start_codes(data) {
        match c {
            0xB0 => {
                if parse_seq_at(data, at).is_err() {
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
    let (_, at) = bits::start_codes(data).find(|&(c, _)| c == 0xB0).ok_or(Error::Invalid("no sequence header"))?;
    Ok(info_of(&parse_seq_at(data, at)?))
}

/// Splits an elementary stream into packets: a packet runs from a sequence header, a picture start
/// code, a sequence end or a video edit code to the next of them (a picture keeps its patches).
pub fn split_packets(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (c, at) in bits::start_codes(data) {
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
    /// 'I', 'P' or 'B'.
    pub kind: char,
    /// `decode_order_index`, modulo 256.
    pub doi: u32,
    pub output_delay: u32,
    /// The reference pictures of list 0 and list 1, as distances back in decoding order.
    pub refs: Vec<i32>,
}

/// A parsed sequence header, for reading picture headers without decoding.
#[derive(Clone, Debug)]
pub struct SequenceHeader {
    seq: header::Sequence,
}

impl SequenceHeader {
    /// Reads the first sequence header of `data`.
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        let (_, at) = bits::start_codes(data).find(|&(c, _)| c == 0xB0).ok_or(Error::Invalid("no sequence header"))?;
        Ok(Self { seq: parse_seq_at(data, at)? })
    }

    pub fn info(&self) -> StreamInfo {
        info_of(&self.seq)
    }

    /// `output_reorder_delay`: how far the display order lags the decoding order.
    pub fn reorder_delay(&self) -> u32 {
        self.seq.output_reorder_delay
    }

    /// The header of the picture `packet` starts with (or holds), if it has one that parses.
    pub fn picture(&self, packet: &[u8]) -> Option<PictureInfo> {
        let (code, at) = bits::start_codes(packet).find(|&(c, _)| is_picture_start(c))?;
        let end = bits::start_codes(packet.get(at + 3..)?).next().map_or(packet.len(), |(_, k)| at + 3 + k);
        let dem = bits::demulate(packet.get(at + 3..end)?);
        let (p, _) = header::Picture::parse(dem.get(1..)?, code, &self.seq, &mut header::DoiState::default()).ok()?;
        let mut refs: Vec<i32> = p.rpl_l0.delta_doi.iter().take(p.rpl_l0.active).copied().collect();
        refs.extend(p.rpl_l1.delta_doi.iter().take(p.rpl_l1.active));
        let kind = match p.slice_type {
            header::SliceType::I => 'I',
            header::SliceType::P => 'P',
            header::SliceType::B => 'B',
        };
        Some(PictureInfo { kind, doi: p.doi, output_delay: p.output_delay, refs })
    }
}

/// An AVS3 decoder fed packets (as [`split_packets`] makes them).
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

    /// Decodes one packet; returns the pictures that are due for output.
    pub fn decode(&mut self, packet: &[u8], tag: i64) -> Result<Vec<Frame>, Error> {
        let res = self.ctx.decode_packet(packet, tag);
        let mut out = Vec::new();
        while let Some(p) = self.ctx.next_output(false) {
            out.extend(self.frame(&p));
        }
        res.map(|()| out)
    }

    /// Drains the pictures still waiting for output at the end of the stream.
    pub fn flush(&mut self) -> Vec<Frame> {
        let mut out = Vec::new();
        while let Some(p) = self.ctx.next_output(true) {
            out.extend(self.frame(&p));
        }
        out
    }

    /// Forgets the reference pictures before a seek: decoding resumes at the next I picture.
    pub fn reset(&mut self) {
        self.ctx.reset();
    }

    fn frame(&self, p: &decoder::OutPic) -> Option<Frame> {
        let s = self.ctx.seq.as_ref()?;
        let (w, h) = (s.horizontal_size as usize, s.vertical_size as usize);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let out_depth = s.output_bit_depth();
        let src_depth = p.pic.bit_depth;
        let shift = src_depth.saturating_sub(out_depth);
        let up = out_depth.saturating_sub(src_depth);
        let max = (1u32 << out_depth) - 1;
        let conv =
            |v: u16| -> u16 { if shift > 0 { ((u32::from(v) + (1 << (shift - 1))) >> shift).min(max) as u16 } else { (u32::from(v) << up).min(max) as u16 } };
        let crop = |pl: &core::Plane, w: usize, h: usize| -> Vec<u16> {
            let stride = pl.w.max(1);
            let mut v = Vec::with_capacity(w * h);
            for row in pl.data.chunks(stride).take(h) {
                v.extend(row.iter().take(w).map(|&x| conv(x)));
            }
            v
        };
        let f = &p.pic.planes;
        Some(Frame {
            width: s.horizontal_size,
            height: s.vertical_size,
            bit_depth: out_depth,
            planes: [crop(&f[0], w, h), crop(&f[1], cw, ch), crop(&f[2], cw, ch)],
            poc: p.poc,
            tag: p.tag,
        })
    }
}
