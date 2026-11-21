// SPDX-License-Identifier: BSD-3-Clause
// Derived from uavs3d (`source/decoder/uavs3d.c`), Copyright (c) 2018-2022 Peking University Shenzhen Graduate
// School, Peng Cheng Laboratory and Guangdong Bohua UHD Innovation Corporation. See LICENSE.uavs3d.
//! AVS3-P2 (IEEE 1857.10 / GB/T 33475.2) video decoder: a Rust port of uavs3d 1.1.
//!
//! Every input is hostile: no panics, no `unsafe`, capped picture size.

mod alf;
mod bits;
mod core;
mod cuutil;
mod deblock;
mod decoder;
mod header;
mod intra;
mod itrans;
mod lbac;
mod mc;
mod mctab;
mod mv;
mod parse;
mod plps;
mod recon;
mod sao;
mod scan;
mod types;

pub use header::SliceType as PictureType;

/// The largest picture the decoder accepts, in samples.
const MAX_SAMPLES: usize = 8192 * 4320;

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
    /// Y, Cb, Cr (4:2:0; chroma is `ceil(w/2) × ceil(h/2)`), one `u16` per sample.
    pub planes: [Vec<u16>; 3],
    /// Picture order (decode order index plus output delay minus reorder delay).
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
    /// Bits per coded sample (8 or 10).
    pub bit_depth: u32,
    /// `aspect_ratio` code (1 = square samples, 2 = 4:3, 3 = 16:9, 4 = 2.21:1).
    pub aspect_ratio: u32,
    /// Frame rate as a fraction.
    pub frame_rate: (u32, u32),
    /// Pictures are output in decoding order.
    pub low_delay: bool,
}

fn info_of(s: &header::Sequence) -> StreamInfo {
    let (n, d) = s.frame_rate();
    StreamInfo {
        profile: s.profile_id,
        level: s.level_id,
        width: s.horizontal_size as u32,
        height: s.vertical_size as u32,
        bit_depth: s.bit_depth_internal,
        aspect_ratio: s.aspect_ratio,
        frame_rate: (n, d),
        low_delay: s.low_delay,
    }
}

/// The start codes of `data`: the code byte and the offset of its `00 00 01`.
fn codes(data: &[u8]) -> impl Iterator<Item = (u8, usize)> + '_ {
    data.windows(4).enumerate().filter_map(|(i, w)| (w[0] == 0 && w[1] == 0 && w[2] == 1).then_some((w[3], i)))
}

/// Parses the first sequence header of `data` (everything from its start code to the next one).
fn parse_sequence(data: &[u8]) -> Result<header::Sequence, Error> {
    let (_, at) = codes(data).find(|&(c, _)| c == 0xB0).ok_or(Error::Invalid("no sequence header"))?;
    let body = data.get(at..).unwrap_or_default();
    let end = codes(body.get(4..).unwrap_or_default()).next().map_or(body.len(), |(_, k)| k + 4);
    let d = bits::demulate(body.get(..end).unwrap_or_default());
    let mut b = bits::Bits::new(&d);
    let s = header::Sequence::parse(&mut b)?;
    if s.pic_width.saturating_mul(s.pic_height) > MAX_SAMPLES {
        return Err(Error::Unsupported("picture too large"));
    }
    Ok(s)
}

/// Whether `code` (the byte after `00 00 01`) ends a packet before it ([`split_packets`]).
pub fn is_packet_start(code: u8) -> bool {
    matches!(code, 0xB0 | 0xB1 | 0xB3 | 0xB6 | 0xB7)
}

/// Whether `code` starts a picture.
pub fn is_picture_start(code: u8) -> bool {
    matches!(code, 0xB3 | 0xB6)
}

/// True when `data` looks like an AVS3 video elementary stream: a sequence header of an AVS3
/// profile (Main-8 0x20, Main-10 0x22) that parses, followed by a picture.
pub fn probe(data: &[u8]) -> bool {
    let mut seq = 0u32;
    let mut pic = 0u32;
    for (c, at) in codes(data) {
        match c {
            0xB0 => {
                let profile = data.get(at + 4).copied().unwrap_or(0);
                if !matches!(profile, 0x20 | 0x22) {
                    return false;
                }
                let body = data.get(at + 4..).unwrap_or_default();
                // a header cut off by the end of `data` is not judged
                if codes(body).next().is_some() && parse_sequence(data.get(at..).unwrap_or_default()).is_err() {
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
    parse_sequence(data).map(|s| info_of(&s))
}

/// Splits an elementary stream into packets: a packet runs from a picture start code to the next
/// picture, sequence header, sequence end or video edit code. Data before the first picture (the
/// sequence header) forms its own packet.
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
    pub kind: PictureType,
    /// `decode_order_index`, modulo 256.
    pub doi: u32,
    /// `picture_output_delay` (0 in low-delay streams).
    pub output_delay: u32,
    /// The reference pictures of both lists, as distances back in decode order.
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
        Ok(Self { seq: parse_sequence(data)? })
    }

    pub fn info(&self) -> StreamInfo {
        info_of(&self.seq)
    }

    /// `output_reorder_delay`: how far the output order lags the decode order.
    pub fn reorder_delay(&self) -> u32 {
        self.seq.output_reorder_delay
    }

    /// The header of the picture `packet` starts with (or holds), if it has one that parses.
    pub fn picture(&self, packet: &[u8]) -> Option<PictureInfo> {
        let (_, at) = codes(packet).find(|&(c, _)| is_picture_start(c))?;
        let body = packet.get(at..)?;
        let end = codes(body.get(4..)?).find(|&(c, _)| c <= 0x8F).map_or(body.len(), |(_, k)| k + 4);
        let d = bits::demulate(body.get(..end)?);
        let mut b = bits::Bits::new(&d);
        let ph = header::PicHeader::parse(&mut b, &self.seq, &mut header::DoiState::default()).ok()?;
        let mut refs: Vec<u32> = Vec::new();
        for rpl in [&ph.rpl_l0, &ph.rpl_l1] {
            for &d in rpl.delta_doi.iter().take(rpl.num) {
                if let Ok(d) = u32::try_from(d)
                    && !refs.contains(&d)
                {
                    refs.push(d);
                }
            }
        }
        Some(PictureInfo { kind: ph.slice_type, doi: ph.decode_order_index, output_delay: ph.output_delay, refs })
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

    /// Decodes one packet; returns the picture that is due for output, if any (like uavs3d, at most
    /// one per decoded picture).
    pub fn decode(&mut self, packet: &[u8], tag: i64) -> Result<Vec<Frame>, Error> {
        let res = catch(|| self.ctx.decode_packet(packet, tag));
        let mut out = Vec::new();
        if res.is_ok()
            && let Some(p) = self.ctx.next_output(false)
        {
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
        let (w, h) = (s.horizontal_size, s.vertical_size);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let crop = |pl: &core::Plane, w: usize, h: usize| -> Vec<u16> {
            let stride = pl.w.max(1);
            let mut v = Vec::with_capacity(w * h);
            for row in pl.data.chunks(stride).take(h) {
                v.extend(row.iter().take(w));
            }
            v
        };
        let f = &p.pic.planes;
        Frame {
            width: w as u32,
            height: h as u32,
            bit_depth: s.bit_depth_internal,
            planes: [crop(&f[0], w, h), crop(&f[1], cw, ch), crop(&f[2], cw, ch)],
            poc: p.poc,
            tag: p.tag,
        }
    }
}

/// Runs `f`, turning a panic of the decoder into an error (a hostile stream must never take the host down).
fn catch(f: impl FnOnce() -> Result<(), Error>) -> Result<(), Error> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        // for debugging the decoder itself
        Err(p) if std::env::var_os("CFA4_AVS3_PANIC").is_some() => std::panic::resume_unwind(p),
        Err(_) => Err(Error::Invalid("internal error")),
    }
}
