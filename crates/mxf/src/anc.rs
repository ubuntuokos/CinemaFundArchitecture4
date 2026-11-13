//! Ancillary data packets in MXF: SMPTE ST 436-1 (MXF Mappings for VI Lines and Ancillary Data
//! Packets, revision of ST 436M-2006).
//!
//! Each content package may hold one frame-wrapped ANC element (item type 17h, element type 02h,
//! Table 6). Its value is the number of ANC structures (UInt16) followed by that many structures
//! (Table 7): line number (UInt16), wrapping type (UInt8), payload sample coding (UInt8), payload
//! sample count (UInt16) and the payload array (an MXF array: element count and element length,
//! UInt32 each, then the bytes, padding included; Annex B). The 8-bit codings store the low 8
//! bits of each ANC word from the DID through the last user data word (7.2); the 10-bit codings
//! pack three words into the high 30 bits of each big-endian 32-bit word (5.4.4) and also carry
//! the checksum. ANC elements are stored in display order (5.2), so element `n` belongs to frame
//! `n` of the track.
//!
//! A VI element (element type 01h, Tables 3, 4) carries digitised vertical interval lines the same
//! way: the number of lines, then per line its number, wrapping type, sample coding (1-, 8- or
//! 10-bit luma, colour difference or both; 5.4.1, 5.4.2, 5.4.4) and the samples. [`ViLine`] keeps
//! them as 10-bit values. ST 334-1 Annex B CEA-608 packets (DID 61h, SDID 02h) give a line byte
//! (field in bit 7) and one byte pair; [`AncData::cea608`] lists them.

use crate::essence::Codec;
use crate::file::{WalkResult, Wrapping, best_metadata, walk};
use crate::klv::find_header_partition;
use crate::meta::{set_type as st, tag};
use crate::source::{ByteSource, read_upto};
use crate::{Error, Rational, Result};

/// Set kind of the ANC Data Descriptor (Table 11).
pub(crate) const ANC_DATA_DESCRIPTOR: u8 = 0x5C;

/// Set kind of the VBI Data Descriptor (Table 10).
pub(crate) const VBI_DATA_DESCRIPTOR: u8 = 0x5B;

/// ANC elements larger than this are skipped (a frame of ANC packets is a few kilobytes).
pub const MAX_ANC_ELEMENT_BYTES: u64 = 1 << 20;
/// [`read_anc`] reads at most this many ANC elements (about 19 hours at 59.94 fps).
pub const MAX_ANC_FRAMES: usize = 1 << 22;

/// One ANC packet of an ANC element (an ANC structure).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AncPacket {
    /// Line number (ST 377-1 Annex G.1.4).
    pub line: u16,
    /// Wrapping type: 01h–04h VANC (frame, field 1, field 2, progressive frame), 11h–14h HANC.
    pub wrapping: u8,
    /// Payload sample coding (Table 7): 4–6 8-bit, 7–9 10-bit, 10–12 8-bit with parity errors.
    pub coding: u8,
    /// The low 8 bits of the packet's words: DID, SDID (or DBN), data count, then the user data
    /// words. A trailing checksum (10-bit codings) is dropped when the data count is consistent.
    pub words: Vec<u8>,
}

impl AncPacket {
    pub fn did(&self) -> Option<u8> {
        self.words.first().copied()
    }
    pub fn sdid(&self) -> Option<u8> {
        self.words.get(1).copied()
    }
    /// The user data words (at most the data count of them).
    pub fn user_data(&self) -> &[u8] {
        let dc = self.words.get(2).map_or(0, |&d| d as usize);
        let end = 3usize.saturating_add(dc).min(self.words.len());
        self.words.get(3..end).unwrap_or(&[])
    }
    /// A caption distribution packet (CEA-708 / CEA-608 over ST 334-2): DID 61h, SDID 01h.
    pub fn is_cdp(&self) -> bool {
        self.did() == Some(0x61) && self.sdid() == Some(0x01)
    }
    /// A CEA-608 packet of ST 334-1 (DID 61h, SDID 02h).
    pub fn is_cea608(&self) -> bool {
        self.did() == Some(0x61) && self.sdid() == Some(0x02)
    }
    /// VANC (not HANC).
    pub fn is_vanc(&self) -> bool {
        self.wrapping & 0x10 == 0
    }
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

/// One stored vertical interval line of a VI element (Table 4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViLine {
    /// Line number (ST 377-1 Annex G.1.4).
    pub line: u16,
    /// Wrapping type: 1 frame, 2 field 1, 3 field 2, 4 progressive frame.
    pub wrapping: u8,
    /// Payload sample coding: 1–3 1-bit, 4–6 8-bit, 7–9 10-bit (luma, colour difference, both).
    pub coding: u8,
    /// The samples as 10-bit values (8-bit samples are the high bits, 5.4.2; a 1-bit sample is
    /// black 64 or white 940, 5.4.1), in order from the sample after SAV.
    pub samples: Vec<u16>,
}

impl ViLine {
    /// Field 2 of an interlaced picture: the wrapping type, else a 525-line field 2 line number
    /// (264–525) or a 625-line one (314–625; both start past 263).
    pub fn field2(&self) -> bool {
        self.wrapping == 3 || (self.wrapping != 2 && self.line >= 264)
    }
}

/// VI lines longer than this many samples are refused (an 8K line has 7680 luma samples).
const MAX_VI_SAMPLES: usize = 16_384;

/// Parse one VI element value (Table 4).
pub fn parse_vi_element(v: &[u8]) -> Result<Vec<ViLine>> {
    let short = || Error::Invalid("VI element cut short".into());
    let n = be16(v, 0).ok_or_else(short)?;
    let mut at = 2usize;
    let mut out = Vec::with_capacity((n as usize).min(v.len() / 14 + 1));
    for _ in 0..n {
        let line = be16(v, at).ok_or_else(short)?;
        let wrapping = *v.get(at + 2).ok_or_else(short)?;
        let coding = *v.get(at + 3).ok_or_else(short)?;
        let count = (be16(v, at + 4).ok_or_else(short)? as usize).min(MAX_VI_SAMPLES);
        let elems = be32(v, at + 6).ok_or_else(short)? as usize;
        let elem_len = be32(v, at + 10).ok_or_else(short)? as usize;
        let bytes = elems.checked_mul(elem_len).ok_or_else(short)?;
        let start = at + 14;
        let end = start.checked_add(bytes).ok_or_else(short)?;
        let payload = v.get(start..end).ok_or_else(short)?;
        at = end;
        let samples: Vec<u16> = match coding {
            1..=3 => payload.iter().flat_map(|b| (0..8).rev().map(move |k| if b >> k & 1 == 1 { 940 } else { 64 })).take(count).collect(),
            7..=9 => payload
                .chunks_exact(4)
                .flat_map(|c| {
                    let w = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
                    [(w >> 22) as u16 & 0x3FF, (w >> 12) as u16 & 0x3FF, (w >> 2) as u16 & 0x3FF]
                })
                .take(count)
                .collect(),
            4..=6 => payload.iter().map(|&b| u16::from(b) << 2).take(count).collect(),
            other => return Err(Error::Unsupported(format!("VI sample coding {other}"))),
        };
        out.push(ViLine { line, wrapping, coding, samples });
    }
    Ok(out)
}

/// Parse one ANC element value (Table 7).
pub fn parse_anc_element(v: &[u8]) -> Result<Vec<AncPacket>> {
    let short = || Error::Invalid("ANC element cut short".into());
    let n = be16(v, 0).ok_or_else(short)?;
    let mut at = 2usize;
    let mut out = Vec::with_capacity((n as usize).min(v.len() / 14 + 1));
    for _ in 0..n {
        let line = be16(v, at).ok_or_else(short)?;
        let wrapping = *v.get(at + 2).ok_or_else(short)?;
        let coding = *v.get(at + 3).ok_or_else(short)?;
        let count = be16(v, at + 4).ok_or_else(short)? as usize;
        let elems = be32(v, at + 6).ok_or_else(short)? as usize;
        let elem_len = be32(v, at + 10).ok_or_else(short)? as usize;
        let bytes = elems.checked_mul(elem_len).ok_or_else(short)?;
        let start = at + 14;
        let end = start.checked_add(bytes).ok_or_else(short)?;
        let payload = v.get(start..end).ok_or_else(short)?;
        at = end;
        let mut words: Vec<u8> = match coding {
            7..=9 => payload
                .chunks_exact(4)
                .flat_map(|c| {
                    let w = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
                    [(w >> 22) as u8, (w >> 12) as u8, (w >> 2) as u8]
                })
                .take(count)
                .collect(),
            _ => payload.iter().copied().take(count).collect(),
        };
        // DID, SDID, DC and DC user data words; anything after them is the checksum or padding
        if let Some(&dc) = words.get(2) {
            let full = 3 + dc as usize;
            if words.len() > full {
                words.truncate(full);
            }
        }
        out.push(AncPacket { line, wrapping, coding, words });
    }
    Ok(out)
}

/// One ANC element value (Table 7) holding `packets`, each written with the 8-bit luma payload
/// coding (04h): the low 8 bits of its words from the DID on (7.2), padded to whole 32-bit
/// words. A packet's own `coding` is not used. At most 65535 packets of 65535 words.
pub fn anc_element(packets: &[AncPacket]) -> Vec<u8> {
    let packets = &packets[..packets.len().min(u16::MAX as usize)];
    let mut v = (packets.len() as u16).to_be_bytes().to_vec();
    for p in packets {
        let words = &p.words[..p.words.len().min(u16::MAX as usize)];
        let padded = words.len().next_multiple_of(4);
        v.extend_from_slice(&p.line.to_be_bytes());
        v.push(p.wrapping);
        v.push(0x04);
        v.extend_from_slice(&(words.len() as u16).to_be_bytes());
        // the payload array: element count and element size (Annex B)
        v.extend_from_slice(&(padded as u32).to_be_bytes());
        v.extend_from_slice(&1u32.to_be_bytes());
        v.extend_from_slice(words);
        v.resize(v.len() + padded - words.len(), 0);
    }
    v
}

/// The ANC and VI elements of a file, one per frame.
#[derive(Clone, Debug)]
pub struct AncData {
    /// Edit rate of the ANC data track (the frames of `frames`).
    pub edit_rate: Rational,
    /// The packets of each frame, in frame order (empty for a frame without packets).
    pub frames: Vec<Vec<AncPacket>>,
    /// The VI lines of each frame, in frame order (empty when the file has no VI elements).
    pub vi: Vec<Vec<ViLine>>,
    /// Problems recovered from.
    pub warnings: Vec<String>,
}

/// A CEA-608 byte pair of an ST 334-1 Annex B packet (DID 61h, SDID 02h).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cea608Pair {
    pub frame: usize,
    /// Field 2 (bit 7 of the line byte clear).
    pub field2: bool,
    /// Line offset from the format's base line (bits 4–0 of the line byte).
    pub line_offset: u8,
    /// The two bytes as stored (with their parity bits).
    pub bytes: [u8; 2],
}

impl AncData {
    /// The CEA-608 byte pairs of the ST 334-1 CEA-608 packets, in frame order.
    pub fn cea608(&self) -> Vec<Cea608Pair> {
        let mut out = Vec::new();
        for (frame, f) in self.frames.iter().enumerate() {
            for p in f.iter().filter(|p| p.is_cea608()) {
                if let [line, a, b] = p.user_data() {
                    out.push(Cea608Pair { frame, field2: line & 0x80 == 0, line_offset: line & 0x1F, bytes: [*a, *b] });
                }
            }
        }
        out
    }

    /// The caption distribution packets as ancillary bytes (DID, SDID, DC, CDP), with their frames.
    pub fn cdps(&self) -> Vec<(usize, Vec<u8>)> {
        let mut out = Vec::new();
        for (i, f) in self.frames.iter().enumerate() {
            for p in f.iter().filter(|p| p.is_cdp()) {
                out.push((i, p.words.clone()));
            }
        }
        out
    }
}

/// Whether `head` (the first bytes of a file) is an MXF file whose header partition lists the
/// ANC packet or the VI line essence container (Table 1: 06 0E 2B 34 04 01 01 xx 0D 01 03 01 02
/// 0E 00 00, or mapping 0Dh), or line-wrapped uncompressed pictures (ST 384: 0D 01 03 01 02 05
/// xx 03), whose VANC / HANC data elements [`read_anc`] also reads.
pub fn has_anc(head: &[u8]) -> bool {
    let Some(at) = find_header_partition(head) else { return false };
    let Some(rest) = head.get(at + 16..) else { return false };
    let Some((len, n)) = crate::klv::ber_length(rest) else { return false };
    let Some(value) = rest.get(n..).map(|v| &v[..v.len().min(len as usize)]) else { return false };
    // 80 bytes of fixed fields, then the essence container batch
    value.get(80..).is_some_and(|b| {
        crate::klv::batch_of(b).into_iter().filter_map(crate::klv::ul_of).any(|ul| {
            // the ANC packet or VI line container, or line-wrapped uncompressed pictures (ST 384
            // Table 5), whose VANC / HANC data elements may carry ANC packets
            ul.is_smpte()
                && (ul.item_starts_with(&[0x0D, 0x01, 0x03, 0x01, 0x02, 0x0E])
                    || ul.item_starts_with(&[0x0D, 0x01, 0x03, 0x01, 0x02, 0x0D])
                    || (ul.item_starts_with(&[0x0D, 0x01, 0x03, 0x01, 0x02, 0x05]) && ul.0[15] == 0x03))
        })
    })
}

/// A frame-wrapped ANC element key: 06 0E 2B 34 01 02 01 01 0D 01 03 01 17 cc 02 nn (Table 6).
fn is_anc_element(k: &crate::Ul) -> bool {
    k.0[8..12] == [0x0D, 0x01, 0x03, 0x01] && k.0[12] == 0x17 && k.0[14] == 0x02
}

/// The ANC packets in the line-wrapped VANC / HANC data elements of an uncompressed picture
/// track (ST 384 7.2), or None when the file has no line-wrapped uncompressed pictures. Each
/// element holds stored samples in the picture's layout; the ST 291 packets (ancillary data flag
/// 000h 3FFh 3FFh, or 00h FFh FFh in 8-bit layouts) are looked for in the luma samples, the colour
/// difference samples and the multiplexed sequence, and packets of 10-bit samples need a correct
/// checksum. The line is the stored line of the system element before the data element (ST 384
/// Table 4), moved to the video line when the descriptor has a Video Line Map; the frame is the
/// picture the element follows a "start of stored data" of (or, without system elements, the
/// picture whose first line comes after it).
fn read_line_wrapped_anc(src: &(impl ByteSource + ?Sized), w: &WalkResult) -> Option<Result<AncData>> {
    let file = match crate::file::open(src) {
        Ok(f) => f,
        Err(e) => return Some(Err(e)),
    };
    let t = file.tracks.iter().find(|t| t.wrapping == Wrapping::Line && t.codec == Codec::Uncompressed)?;
    let mut warnings = Vec::new();
    let Some(words) = t.picture.as_ref().and_then(|p| crate::uncompressed::Layout::of(p).ok()).and_then(|l| SampleWords::of(&l)) else {
        warnings.push("line-wrapped VANC / HANC data in this picture layout are not read".into());
        return Some(Ok(AncData { edit_rate: t.edit_rate, frames: vec![Vec::new(); t.samples.len()], vi: Vec::new(), warnings }));
    };
    let first_line = best_metadata(w)
        .ok()
        .and_then(|(_, md)| {
            md.sets.iter().filter(|s| matches!(s.kind(), Some(st::CDCI_DESCRIPTOR | st::RGBA_DESCRIPTOR))).find_map(|d| {
                let v = d.get(tag::VIDEO_LINE_MAP)?;
                let n = be32(v, 0)?;
                (n > 0).then(|| be32(v, 8)).flatten().filter(|&l| (1..=u32::from(u16::MAX)).contains(&l))
            })
        })
        .unwrap_or(1);
    // value offset -> (size, HANC)
    let elements: std::collections::HashMap<u64, (u64, bool)> = w
        .elements
        .iter()
        .filter(|e| e.key.0[12] == 0x17 && matches!(e.key.0[14], 0x09 | 0x0A) && !e.truncated)
        .map(|e| (e.value_offset, (e.size, e.key.0[14] == 0x0A)))
        .collect();
    let picture_starts: Vec<u64> = t.samples.iter().map(|s| s.offset).collect();
    let mut frames: Vec<Vec<AncPacket>> = vec![Vec::new(); t.samples.len()];
    let mut skipped = 0usize;
    for &(at, stored_line) in &w.anc_lines {
        let Some(&(size, hanc)) = elements.get(&at) else { continue };
        // which picture: the last start of stored data before the element, else the next picture
        let frame =
            if w.line_starts.is_empty() { picture_starts.partition_point(|&s| s < at) } else { w.line_starts.partition_point(|&s| s < at).saturating_sub(1) };
        if frame >= MAX_ANC_FRAMES || size > MAX_ANC_ELEMENT_BYTES {
            skipped += 1;
            continue;
        }
        let v = match read_upto(src, at, size as usize) {
            Ok(v) => v,
            Err(e) => return Some(Err(e.into())),
        };
        let line = u16::try_from(stored_line.max(1).saturating_add(first_line - 1)).unwrap_or(u16::MAX);
        if frames.len() <= frame {
            frames.resize(frame + 1, Vec::new());
        }
        let Some(slot) = frames.get_mut(frame) else { continue };
        for (coding, packet) in words.packets(&v) {
            if !slot.iter().any(|p| p.line == line && p.words == packet) {
                slot.push(AncPacket { line, wrapping: if hanc { 0x11 } else { 0x01 }, coding, words: packet });
            }
        }
    }
    if skipped > 0 {
        warnings.push(format!("{skipped} line-wrapped VANC / HANC data elements skipped"));
    }
    Some(Ok(AncData { edit_rate: t.edit_rate, frames, vi: Vec::new(), warnings }))
}

/// How the stored samples of a line-wrapped data element become ancillary words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SampleWords {
    /// One byte per sample; `luma_odd`: the luma samples are the odd ones (Cb Y Cr Y), `subsampled`:
    /// every other sample is luma (4:2:2).
    Bytes { luma_odd: bool, subsampled: bool },
    /// v210: three 10-bit samples in the low 30 bits of each little-endian 32-bit word, Cb Y Cr Y.
    V210,
    /// 16-bit big-endian samples (the top 10 bits are the word), Cb Y Cr Y.
    Words16,
}

impl SampleWords {
    fn of(l: &crate::uncompressed::Layout) -> Option<SampleWords> {
        use crate::UncompressedLayout as U;
        use crate::uncompressed::{Comp, Layout};
        Some(match l {
            Layout::YCbCr(U::Uyvy8) => SampleWords::Bytes { luma_odd: true, subsampled: true },
            Layout::YCbCr(U::Yuy2x8) => SampleWords::Bytes { luma_odd: false, subsampled: true },
            Layout::YCbCr(U::CbYCr444x8 | U::Planar422x8 | U::Planar420x8) => SampleWords::Bytes { luma_odd: false, subsampled: false },
            Layout::YCbCr(U::V210 | U::Planar422x10) => SampleWords::V210,
            Layout::YCbCr(U::CbYCrY16) => SampleWords::Words16,
            Layout::Interleaved { pattern, pixels: 2, depth: 8, padding: 0 } => {
                SampleWords::Bytes { luma_odd: pattern.first() != Some(&Comp::Y), subsampled: true }
            }
            Layout::Interleaved { depth: 8, padding: 0, .. } => SampleWords::Bytes { luma_odd: false, subsampled: false },
            Layout::Interleaved { pattern, pixels: 2, depth: 16, padding: 0 } if pattern.first() != Some(&Comp::Y) => SampleWords::Words16,
            _ => return None,
        })
    }

    /// The ST 291 packets in `v` as (ST 436 payload coding, DID SDID DC user data words).
    fn packets(self, v: &[u8]) -> Vec<(u8, Vec<u8>)> {
        let (samples, ten, luma_odd, subsampled): (Vec<u16>, bool, bool, bool) = match self {
            SampleWords::Bytes { luma_odd, subsampled } => (v.iter().map(|&b| u16::from(b)).collect(), false, luma_odd, subsampled),
            SampleWords::V210 => (
                v.chunks_exact(4)
                    .flat_map(|c| {
                        let w = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                        [(w & 0x3FF) as u16, (w >> 10 & 0x3FF) as u16, (w >> 20 & 0x3FF) as u16]
                    })
                    .collect(),
                true,
                true,
                true,
            ),
            SampleWords::Words16 => (v.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]]) >> 6).collect(), true, true, true),
        };
        // Table 7 codings: luma, colour difference, both; 8-bit 4–6, 10-bit 7–9
        let base = if ten { 7 } else { 4 };
        let mut streams: Vec<(u8, Vec<u16>)> = vec![(base + 2, samples.clone())];
        if subsampled {
            let pick = |odd: bool| samples.iter().skip(usize::from(odd)).step_by(2).copied().collect::<Vec<u16>>();
            streams.push((base, pick(luma_odd)));
            streams.push((base + 1, pick(!luma_odd)));
        }
        let mut out: Vec<(u8, Vec<u8>)> = Vec::new();
        for (coding, s) in streams {
            for p in st291_packets(&s, ten) {
                if !out.iter().any(|o| o.1 == p) {
                    out.push((coding, p));
                }
            }
        }
        out
    }
}

/// The ST 291 packets of a sequence of ancillary words: after each ancillary data flag
/// (000h 3FFh 3FFh, 8-bit 00h FFh FFh) the DID, SDID / DBN, data count and that many user data
/// words, as their low 8 bits; with 10-bit words the checksum (9-bit sum of DID through the
/// last user data word) must match.
fn st291_packets(s: &[u16], ten: bool) -> Vec<Vec<u8>> {
    let max = if ten { 0x3FF } else { 0xFF };
    let mut out = Vec::new();
    let mut i = 0usize;
    while let Some(w) = s.get(i..i + 6) {
        if w[0] != 0 || w[1] != max || w[2] != max {
            i += 1;
            continue;
        }
        let dc = usize::from(w[5] & 0xFF);
        let Some(body) = s.get(i + 3..i + 6 + dc) else { break };
        if ten {
            let sum = body.iter().fold(0u16, |a, &x| a.wrapping_add(x & 0x1FF)) & 0x1FF;
            match s.get(i + 6 + dc) {
                Some(&cs) if cs & 0x1FF == sum => {}
                _ => {
                    i += 1;
                    continue;
                }
            }
        }
        out.push(body.iter().map(|&x| (x & 0xFF) as u8).collect());
        i += 6 + dc;
    }
    out
}

/// A frame-wrapped VI element key: 06 0E 2B 34 01 02 01 01 0D 01 03 01 17 cc 01 nn (Table 3).
fn is_vi_element(k: &crate::Ul) -> bool {
    k.0[8..12] == [0x0D, 0x01, 0x03, 0x01] && k.0[12] == 0x17 && k.0[14] == 0x01
}

/// Read every ANC element and every VI element of the file (the first stream of each when there
/// are several), or else the VANC / HANC packets of line-wrapped uncompressed pictures.
pub fn read_anc(src: &(impl ByteSource + ?Sized)) -> Result<AncData> {
    let w = walk(src)?;
    let mut warnings = w.warnings.clone();
    let Some(first) = w.elements.iter().find(|e| is_anc_element(&e.key)).or_else(|| w.elements.iter().find(|e| is_vi_element(&e.key))) else {
        return read_line_wrapped_anc(src, &w).unwrap_or_else(|| Err(Error::Invalid("no ANC or VI data elements (ST 436)".into())));
    };
    let number: [u8; 4] = first.key.0[12..16].try_into().unwrap_or([0; 4]);
    let vi_number: Option<[u8; 4]> = w.elements.iter().find(|e| is_vi_element(&e.key)).and_then(|e| e.key.0[12..16].try_into().ok());
    let edit_rate = best_metadata(&w)
        .ok()
        .and_then(|(_, md)| {
            let desc = md.of_kind(ANC_DATA_DESCRIPTOR).chain(md.of_kind(VBI_DATA_DESCRIPTOR)).find_map(|d| d.rational(tag::SAMPLE_RATE));
            let track = || {
                md.of_kind(st::TIMELINE_TRACK)
                    .filter(|t| t.u(tag::TRACK_NUMBER) == Some(u32::from_be_bytes(number) as i64))
                    .find_map(|t| t.rational(tag::EDIT_RATE))
            };
            desc.or_else(track).filter(Rational::is_valid)
        })
        .ok_or_else(|| Error::Invalid("the ANC data track has no edit rate".into()))?;
    let mut frames = Vec::new();
    let mut vi = Vec::new();
    if let Some(vn) = vi_number {
        for e in w.elements.iter().filter(|e| e.key.0[12..16] == vn) {
            if vi.len() >= MAX_ANC_FRAMES {
                warnings.push(format!("more than {MAX_ANC_FRAMES} VI elements; the rest are ignored"));
                break;
            }
            if e.truncated || e.size > MAX_ANC_ELEMENT_BYTES {
                warnings.push(format!("VI element of frame {} skipped ({} bytes{})", vi.len(), e.size, if e.truncated { ", truncated" } else { "" }));
                vi.push(Vec::new());
                continue;
            }
            let v = read_upto(src, e.value_offset, e.size as usize)?;
            match parse_vi_element(&v) {
                Ok(l) => vi.push(l),
                Err(err) => {
                    warnings.push(format!("VI element of frame {}: {err}", vi.len()));
                    vi.push(Vec::new());
                }
            }
        }
    }
    for e in w.elements.iter().filter(|e| e.key.0[12..16] == number && is_anc_element(&e.key)) {
        if frames.len() >= MAX_ANC_FRAMES {
            warnings.push(format!("more than {MAX_ANC_FRAMES} ANC elements; the rest are ignored"));
            break;
        }
        if e.truncated || e.size > MAX_ANC_ELEMENT_BYTES {
            warnings.push(format!("ANC element of frame {} skipped ({} bytes{})", frames.len(), e.size, if e.truncated { ", truncated" } else { "" }));
            frames.push(Vec::new());
            continue;
        }
        let v = read_upto(src, e.value_offset, e.size as usize)?;
        match parse_anc_element(&v) {
            Ok(p) => frames.push(p),
            Err(err) => {
                warnings.push(format!("ANC element of frame {}: {err}", frames.len()));
                frames.push(Vec::new());
            }
        }
    }
    Ok(AncData { edit_rate, frames, vi, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{anc_element, build_anc, build_timed_text, build_vi, vi_element};

    fn cdp_packet(seq: u8) -> Vec<u8> {
        // DID 61h, SDID 01h, DC, then a CDP header fragment (enough for the ANC layer)
        let udw = [0x96, 0x69, 0x08, 0x4F, 0x43, 0x00, seq, 0x74];
        let mut w = vec![0x61, 0x01, udw.len() as u8];
        w.extend_from_slice(&udw);
        w
    }

    /// 8-bit and 10-bit payload codings give the same words; the 10-bit checksum is dropped.
    #[test]
    fn eight_and_ten_bit_codings() {
        let packets = vec![cdp_packet(1), vec![0x41, 0x07, 0x02, 0xAA, 0x55]];
        for ten in [false, true] {
            let got = parse_anc_element(&anc_element(&packets, ten)).unwrap();
            assert_eq!(got.len(), 2);
            assert_eq!(got[0].words, packets[0], "ten_bit {ten}");
            assert_eq!(got[1].words, packets[1], "ten_bit {ten}");
            assert_eq!((got[0].line, got[0].wrapping, got[0].coding), (9, 1, if ten { 7 } else { 4 }));
            assert!(got[0].is_cdp() && got[0].is_vanc() && !got[1].is_cdp());
            assert_eq!(got[1].user_data(), &[0xAA, 0x55]);
        }
        assert_eq!(parse_anc_element(&[0, 0]).unwrap(), vec![]);
    }

    /// ST 291 packets in sample words: cut packets, a data count past the end and a bad 10-bit
    /// checksum give nothing, never a panic; 8-bit packets need no checksum.
    #[test]
    fn st291_packets_in_samples() {
        let ten = [0u16, 0x3FF, 0x3FF, 0x161, 0x101, 0x102, 0x1AA, 0x155, 0];
        let sum = ten[3..8].iter().fold(0u16, |a, &x| a + (x & 0x1FF)) & 0x1FF;
        let mut good = ten.to_vec();
        good[8] = sum;
        assert_eq!(st291_packets(&good, true), vec![vec![0x61, 0x01, 0x02, 0xAA, 0x55]]);
        for cut in 0..good.len() {
            assert!(st291_packets(&good[..cut], true).is_empty(), "cut {cut}");
        }
        let mut bad = good.clone();
        bad[8] ^= 1;
        assert!(st291_packets(&bad, true).is_empty());
        assert_eq!(st291_packets(&[7, 0, 0xFF, 0xFF, 0x41, 0x05, 1, 9], false), vec![vec![0x41, 0x05, 1, 9]]);
        assert!(st291_packets(&[0, 0xFF, 0xFF, 0x41, 0x05, 200, 9], false).is_empty());
        assert!(SampleWords::V210.packets(&[0xFF; 7]).is_empty());
    }

    /// Short, oversized or inconsistent elements are errors, never panics.
    #[test]
    fn hostile_elements() {
        let good = anc_element(&[cdp_packet(1)], false);
        for cut in 0..good.len() {
            assert!(parse_anc_element(&good[..cut]).is_err(), "cut {cut}");
        }
        let mut big = good.clone();
        big[0..2].copy_from_slice(&0xFFFFu16.to_be_bytes());
        assert!(parse_anc_element(&big).is_err());
        // element count x length overflowing, or a count past the payload
        let mut huge = good.clone();
        huge[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        huge[12..16].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(parse_anc_element(&huge).is_err());
        let mut count = good;
        count[6..8].copy_from_slice(&0xFFFFu16.to_be_bytes());
        assert_eq!(parse_anc_element(&count).unwrap()[0].words, cdp_packet(1));
        let p = AncPacket { line: 0, wrapping: 0x11, coding: 4, words: vec![0x61, 0x01, 200, 1] };
        assert_eq!(p.user_data(), &[1]);
        assert!(!p.is_vanc());
    }

    /// The ANC track of an OP1a file: one element per frame, the descriptor's rate, CDPs by frame.
    #[test]
    fn reads_the_anc_track() {
        let frames = vec![anc_element(&[cdp_packet(0)], false), anc_element(&[], false), anc_element(&[cdp_packet(2), vec![0x41, 0x05, 0]], true), vec![0, 9]];
        let f = build_anc((30000, 1001), &frames);
        assert!(has_anc(&f));
        let a = read_anc(&f).unwrap();
        assert_eq!(a.edit_rate, Rational::new(30000, 1001));
        assert_eq!(a.frames.len(), 4);
        assert!(a.frames[1].is_empty() && a.frames[3].is_empty());
        assert_eq!(a.warnings.len(), 1, "{:?}", a.warnings);
        assert_eq!(a.cdps(), vec![(0, cdp_packet(0)), (2, cdp_packet(2))]);
        let tt = build_timed_text(b"<SubtitleReel/>", &[]);
        assert!(!has_anc(&tt));
        assert!(read_anc(&tt).is_err());
        for cut in [0, 20, f.len() / 2, f.len() - 30] {
            let _ = read_anc(&f[..cut]);
            let _ = has_anc(&f[..cut]);
        }
    }

    /// The 1-, 8- and 10-bit VI codings give the line's samples as 10-bit values; the field comes
    /// from the wrapping type or the line number.
    #[test]
    fn vi_line_codings() {
        let ramp: Vec<u16> = (0..21).map(|i| (i * 48) as u16 & !3).collect();
        let bits: Vec<u16> = (0..13).map(|i| if i % 3 == 0 { 940 } else { 64 }).collect();
        let lines = vec![(21u16, 2u8, 7u8, ramp.clone()), (284, 1, 4, ramp.clone()), (21, 3, 1, bits.clone())];
        let got = parse_vi_element(&vi_element(&lines)).unwrap();
        assert_eq!(got.len(), 3);
        for (g, (line, wrapping, coding, samples)) in got.iter().zip(&lines) {
            assert_eq!((g.line, g.wrapping, g.coding), (*line, *wrapping, *coding));
            assert_eq!(&g.samples, samples, "coding {coding}");
        }
        assert_eq!(got.iter().map(ViLine::field2).collect::<Vec<_>>(), vec![false, true, true]);
        let good = vi_element(&lines);
        for cut in 0..good.len() {
            let _ = parse_vi_element(&good[..cut]);
        }
        let mut bad = vi_element(&lines[..1]);
        bad[5] = 12;
        assert!(parse_vi_element(&bad).is_err());
        let mut huge = vi_element(&lines[..1]);
        huge[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        huge[12..16].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(parse_vi_element(&huge).is_err());
    }

    /// ST 334-1 Annex B packets give their byte pairs and fields; other packets are skipped.
    #[test]
    fn cea608_packets() {
        let frames = vec![
            anc_element(&[vec![0x61, 0x02, 3, 0x89, 0x94, 0x2C], cdp_packet(1)], false),
            anc_element(&[vec![0x61, 0x02, 3, 0x09, 0x80, 0x80], vec![0x61, 0x02, 2, 0x89, 0x94]], true),
        ];
        let a = read_anc(&build_anc((30000, 1001), &frames)).unwrap();
        assert_eq!(
            a.cea608(),
            vec![
                Cea608Pair { frame: 0, field2: false, line_offset: 9, bytes: [0x94, 0x2C] },
                Cea608Pair { frame: 1, field2: true, line_offset: 9, bytes: [0x80, 0x80] }
            ]
        );
        assert!(a.vi.is_empty());
    }

    /// A file with only a VBI data track reads its VI lines frame by frame.
    #[test]
    fn reads_a_vbi_track() {
        let line = |v: u16| vec![(21u16, 2u8, 4u8, vec![v; 8])];
        let frames = vec![vi_element(&line(64)), vi_element(&[]), vi_element(&line(500)), vec![0, 1]];
        let f = build_vi((25, 1), &frames);
        assert!(has_anc(&f));
        let a = read_anc(&f).unwrap();
        assert_eq!(a.edit_rate, Rational::new(25, 1));
        assert!(a.frames.is_empty());
        assert_eq!(a.vi.len(), 4);
        assert_eq!(a.vi[0][0].samples, vec![64; 8]);
        assert!(a.vi[1].is_empty() && a.vi[3].is_empty());
        assert_eq!(a.vi[2][0].samples, vec![500; 8]);
        assert_eq!(a.warnings.len(), 1, "{:?}", a.warnings);
        for cut in [0, 20, f.len() / 2, f.len() - 30] {
            let _ = read_anc(&f[..cut]);
        }
    }
}
