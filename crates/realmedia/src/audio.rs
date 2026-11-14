// Ported from FFmpeg n7.1.1 libavformat/rmdec.c (rm_read_audio_stream_info, ff_rm_parse_packet,
// ff_rm_retrieve_cache) (Copyright (c) 2000, 2001 Fabrice Bellard), LGPL-2.1-or-later; see
// docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! RealAudio stream headers (`.ra\xfd` type-specific data) and the deinterleaving of audio data
//! packets into codec frames.

use crate::{Error, Packet, Result};

/// Largest codec data we take (FFmpeg caps extradata at 16 MiB; RealAudio needs a few bytes).
const MAX_EXTRADATA: usize = 1 << 16;
/// Largest deinterleaving superblock (`audio_framesize × sub_packet_h`).
const MAX_SUPERBLOCK: usize = 1 << 24;

/// How the packets of a stream are interleaved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interleaver {
    /// `Int0`: no interleaving, one packet is one frame.
    Int0,
    /// `Int4`: RealAudio 28.8.
    Int4,
    /// `genr`: Cook and ATRAC3.
    Genr,
    /// `sipr`: Sipro.
    Sipr,
    /// `vbrs` / `vbrf`: AAC, a length-prefixed list of frames per packet.
    Vbr,
}

/// A parsed RealAudio header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioInfo {
    /// Header version (3, 4 or 5).
    pub version: u16,
    /// Codec FourCC as stored (`cook`, `atrc`, `sipr`, `28_8`, `raac`, …; `lpcJ` for version 3).
    pub fourcc: [u8; 4],
    pub flavor: u16,
    pub sample_rate: u32,
    pub channels: u16,
    /// Bytes per minute from the header (0 when unknown).
    pub bytes_per_minute: u32,
    pub coded_framesize: u32,
    pub sub_packet_h: u16,
    /// FFmpeg's `audio_framesize`: the bytes of one interleaving row.
    pub audio_framesize: u32,
    pub sub_packet_size: u16,
    /// Bytes of one codec frame as the demuxer hands them out (FFmpeg's `block_align`).
    pub block_align: u32,
    pub interleaver: Interleaver,
    pub extradata: Vec<u8>,
}

/// SIPR sub-packet sizes per flavour (`ff_sipr_subpk_size`).
const SIPR_SUBPK_SIZE: [u32; 4] = [29, 19, 37, 20];

struct Rd<'a> {
    d: &'a [u8],
    p: usize,
}

impl Rd<'_> {
    fn bytes(&mut self, n: usize) -> Result<&[u8]> {
        let s = self.d.get(self.p..self.p + n).ok_or(Error::Invalid("RealAudio header too short"))?;
        self.p += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(*self.bytes(1)?.first().ok_or(Error::Invalid("RealAudio header too short"))?)
    }
    fn be16(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn be32(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn skip(&mut self, n: usize) -> Result<()> {
        self.bytes(n).map(|_| ())
    }
    /// `get_str8`: a length byte and that many bytes.
    fn str8(&mut self) -> Result<Vec<u8>> {
        let n = usize::from(self.u8()?);
        Ok(self.bytes(n)?.to_vec())
    }
}

fn fourcc_of(b: &[u8]) -> [u8; 4] {
    let mut f = [0u8; 4];
    for (d, s) in f.iter_mut().zip(b) {
        *d = *s;
    }
    f
}

/// `rm_read_audio_stream_info` on a stream's type-specific data (starting with `.ra\xfd`).
pub fn parse_audio_info(codec_data: &[u8]) -> Result<AudioInfo> {
    let mut r = Rd { d: codec_data, p: 0 };
    if r.bytes(4)? != b".ra\xfd" {
        return Err(Error::Invalid("not a RealAudio header"));
    }
    let version = r.be16()?;
    if version == 3 {
        // RealAudio 1.0 (14.4): 8 kHz mono, frames of 20 bytes, not interleaved
        let header_size = usize::from(r.be16()?);
        let start = r.p;
        r.skip(8)?;
        let bpm = u32::from(r.be16()?);
        let _ = (start, header_size);
        return Ok(AudioInfo {
            version,
            fourcc: *b"lpcJ",
            flavor: 0,
            sample_rate: 8000,
            channels: 1,
            bytes_per_minute: bpm,
            coded_framesize: 0,
            sub_packet_h: 0,
            audio_framesize: 0,
            sub_packet_size: 0,
            block_align: 0,
            interleaver: Interleaver::Int0,
            extradata: Vec::new(),
        });
    }
    r.skip(2)?;
    r.be32()?; // .ra4
    r.be32()?; // data size
    r.be16()?; // version2
    r.be32()?; // header size
    let flavor = r.be16()?;
    let coded_framesize = r.be32()?;
    if coded_framesize > i32::MAX as u32 {
        return Err(Error::Invalid("coded frame size"));
    }
    r.be32()?;
    let bytes_per_minute = r.be32()?;
    r.be32()?;
    let sub_packet_h = r.be16()?;
    let frame_size = u32::from(r.be16()?);
    let sub_packet_size = r.be16()?;
    r.be16()?;
    if version == 5 {
        r.skip(6)?;
    }
    let sample_rate = u32::from(r.be16()?);
    r.be32()?;
    let channels = r.be16()?;
    let (deint, fourcc) = if version == 5 {
        let d = fourcc_of(r.bytes(4)?);
        let f = fourcc_of(r.bytes(4)?);
        (d, f)
    } else {
        let d = fourcc_of(&r.str8()?);
        let f = fourcc_of(&r.str8()?);
        (d, f)
    };
    let mut audio_framesize = 0u32;
    let mut block_align = frame_size;
    let mut extradata = Vec::new();
    match &fourcc {
        b"28_8" => {
            audio_framesize = frame_size;
            block_align = coded_framesize;
        }
        b"cook" | b"atrc" | b"sipr" => {
            r.be16()?;
            r.u8()?;
            if version == 5 {
                r.u8()?;
            }
            let len = r.be32()? as usize;
            if len > MAX_EXTRADATA {
                return Err(Error::Invalid("RealAudio codec data too large"));
            }
            audio_framesize = frame_size;
            if &fourcc == b"sipr" {
                block_align = *SIPR_SUBPK_SIZE.get(usize::from(flavor)).ok_or(Error::Invalid("SIPR flavour"))?;
            } else {
                if sub_packet_size == 0 {
                    return Err(Error::Invalid("sub_packet_size"));
                }
                block_align = u32::from(sub_packet_size);
            }
            extradata = r.bytes(len)?.to_vec();
        }
        b"raac" | b"racp" => {
            r.be16()?;
            r.u8()?;
            if version == 5 {
                r.u8()?;
            }
            let len = r.be32()? as usize;
            if len > MAX_EXTRADATA {
                return Err(Error::Invalid("RealAudio codec data too large"));
            }
            if len >= 1 {
                r.u8()?;
                extradata = r.bytes(len - 1)?.to_vec();
            }
        }
        _ => {}
    }
    let interleaver = match &deint {
        b"Int0" => Interleaver::Int0,
        b"Int4" => Interleaver::Int4,
        b"genr" => Interleaver::Genr,
        b"sipr" => Interleaver::Sipr,
        b"vbrs" | b"vbrf" => Interleaver::Vbr,
        _ => return Err(Error::Invalid("unknown RealAudio interleaver")),
    };
    let h = u64::from(sub_packet_h);
    let (cfs, afs, sps) = (u64::from(coded_framesize), u64::from(audio_framesize), u64::from(sub_packet_size));
    match interleaver {
        Interleaver::Int4 => {
            if cfs > afs || h <= 1 || cfs * h > (2 + (h & 1)) * afs || cfs * h != 2 * afs {
                return Err(Error::Invalid("Int4 interleaver parameters"));
            }
        }
        Interleaver::Genr if sps == 0 || sps > afs || afs % sps != 0 => {
            return Err(Error::Invalid("genr interleaver parameters"));
        }
        _ => {}
    }
    if matches!(interleaver, Interleaver::Int4 | Interleaver::Genr | Interleaver::Sipr)
        && (block_align == 0 || afs * h > MAX_SUPERBLOCK as u64 || afs * h < u64::from(block_align))
    {
        return Err(Error::Invalid("RealAudio superblock size"));
    }
    Ok(AudioInfo {
        version,
        fourcc,
        flavor,
        sample_rate,
        channels,
        bytes_per_minute,
        coded_framesize,
        sub_packet_h,
        audio_framesize,
        sub_packet_size,
        block_align,
        interleaver,
        extradata,
    })
}

/// One codec frame out of the deinterleaver.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioFrame {
    /// The timestamp of the data packet that started the superblock, on its first frame only.
    pub timestamp_ms: Option<u32>,
    pub data: Vec<u8>,
}

/// `ff_rm_parse_packet` / `ff_rm_retrieve_cache` for audio: data packets in, codec frames out.
pub struct Deinterleaver {
    info: AudioInfo,
    buf: Vec<u8>,
    count: u16,
    timestamp: u32,
}

impl Deinterleaver {
    pub fn new(info: &AudioInfo) -> Self {
        let size = match info.interleaver {
            Interleaver::Int4 | Interleaver::Genr | Interleaver::Sipr => {
                (info.audio_framesize as usize).saturating_mul(usize::from(info.sub_packet_h)).min(MAX_SUPERBLOCK)
            }
            _ => 0,
        };
        Self { info: info.clone(), buf: vec![0; size], count: 0, timestamp: 0 }
    }

    /// Forgets a partly filled superblock (after a seek).
    pub fn reset(&mut self) {
        self.count = 0;
    }

    /// The frames a data packet completes (none while a superblock is being filled).
    pub fn push(&mut self, p: &Packet) -> Vec<AudioFrame> {
        let d = &p.data;
        let i = &self.info;
        match i.interleaver {
            Interleaver::Int4 | Interleaver::Genr | Interleaver::Sipr => {
                let (sps, cfs, h, w) = (usize::from(i.sub_packet_size), i.coded_framesize as usize, usize::from(i.sub_packet_h), i.audio_framesize as usize);
                if p.keyframe {
                    self.count = 0;
                }
                let y = usize::from(self.count);
                if y == 0 {
                    self.timestamp = p.timestamp_ms;
                }
                let mut at = 0usize;
                // readfull: a short packet leaves zeros
                let mut put = |buf: &mut Vec<u8>, dst: usize, n: usize| {
                    for k in 0..n {
                        if let Some(o) = buf.get_mut(dst + k) {
                            *o = d.get(at + k).copied().unwrap_or(0);
                        }
                    }
                    at += n;
                };
                match i.interleaver {
                    Interleaver::Int4 => {
                        for x in 0..h / 2 {
                            put(&mut self.buf, x * 2 * w + y * cfs, cfs);
                        }
                    }
                    Interleaver::Genr => {
                        for x in 0..w.checked_div(sps).unwrap_or(0) {
                            put(&mut self.buf, sps * (h * x + h.div_ceil(2) * (y & 1) + (y >> 1)), sps);
                        }
                    }
                    _ => put(&mut self.buf, y * w, w),
                }
                self.count += 1;
                if usize::from(self.count) < h {
                    return Vec::new();
                }
                self.count = 0;
                if i.interleaver == Interleaver::Sipr {
                    reorder_sipr(&mut self.buf, h, w);
                }
                let ba = i.block_align as usize;
                if ba == 0 {
                    return Vec::new();
                }
                let n = h * w / ba;
                (0..n)
                    .map(|k| AudioFrame { timestamp_ms: (k == 0).then_some(self.timestamp), data: self.buf.get(k * ba..(k + 1) * ba).unwrap_or(&[]).to_vec() })
                    .collect()
            }
            Interleaver::Vbr => {
                if d.len() < 2 {
                    return Vec::new();
                }
                let a = d.get(1).copied().unwrap_or(0);
                let n = usize::from((a & 0xF0) >> 4);
                let mut at = 2 + 2 * n;
                let mut out = Vec::new();
                for k in 0..n {
                    let len = d.get(2 + 2 * k..4 + 2 * k).map_or(0, |b| usize::from(u16::from_be_bytes([b[0], b[1]])));
                    let s = d.get(at..at.saturating_add(len).min(d.len())).unwrap_or(&[]);
                    out.push(AudioFrame { timestamp_ms: (k == 0).then_some(p.timestamp_ms), data: s.to_vec() });
                    at = at.saturating_add(len);
                }
                out
            }
            Interleaver::Int0 => vec![AudioFrame { timestamp_ms: Some(p.timestamp_ms), data: d.clone() }],
        }
    }
}

/// `ff_rm_reorder_sipr_data`.
fn reorder_sipr(buf: &mut [u8], sub_packet_h: usize, framesize: usize) {
    const SWAPS: [[u8; 2]; 38] = [
        [0, 63],
        [1, 22],
        [2, 44],
        [3, 90],
        [5, 81],
        [7, 31],
        [8, 86],
        [9, 58],
        [10, 36],
        [12, 68],
        [13, 39],
        [14, 73],
        [15, 53],
        [16, 69],
        [17, 57],
        [19, 88],
        [20, 34],
        [21, 71],
        [24, 46],
        [25, 94],
        [26, 54],
        [28, 75],
        [29, 50],
        [32, 70],
        [33, 92],
        [35, 74],
        [38, 85],
        [40, 56],
        [42, 87],
        [43, 65],
        [45, 59],
        [48, 79],
        [49, 93],
        [51, 89],
        [55, 95],
        [61, 76],
        [67, 83],
        [77, 80],
    ];
    let bs = sub_packet_h * framesize * 2 / 96; // nibbles per subpacket
    let nib = |b: &[u8], i: usize| b.get(i >> 1).map_or(0, |v| (v >> (4 * (i & 1))) & 0xF);
    for [a, b] in SWAPS {
        for j in 0..bs {
            let (i, o) = (bs * usize::from(a) + j, bs * usize::from(b) + j);
            let x = nib(buf, i);
            let y = nib(buf, o);
            if let Some(v) = buf.get_mut(o >> 1) {
                *v = (x << (4 * (o & 1))) | (*v & (0xF << (4 * (!o & 1))));
            }
            if let Some(v) = buf.get_mut(i >> 1) {
                *v = (y << (4 * (i & 1))) | (*v & (0xF << (4 * (!i & 1))));
            }
        }
    }
}
