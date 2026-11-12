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

use crate::file::{best_metadata, walk};
use crate::klv::find_header_partition;
use crate::meta::{set_type as st, tag};
use crate::source::{ByteSource, read_upto};
use crate::{Error, Rational, Result};

/// Set kind of the ANC Data Descriptor (Table 11).
pub(crate) const ANC_DATA_DESCRIPTOR: u8 = 0x5C;

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

/// The ANC elements of a file, one per frame.
#[derive(Clone, Debug)]
pub struct AncData {
    /// Edit rate of the ANC data track (the frames of `frames`).
    pub edit_rate: Rational,
    /// The packets of each frame, in frame order (empty for a frame without packets).
    pub frames: Vec<Vec<AncPacket>>,
    /// Problems recovered from.
    pub warnings: Vec<String>,
}

impl AncData {
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
/// ANC packet essence container (Table 1: 06 0E 2B 34 04 01 01 xx 0D 01 03 01 02 0E 00 00).
pub fn has_anc(head: &[u8]) -> bool {
    let Some(at) = find_header_partition(head) else { return false };
    let Some(rest) = head.get(at + 16..) else { return false };
    let Some((len, n)) = crate::klv::ber_length(rest) else { return false };
    let Some(value) = rest.get(n..).map(|v| &v[..v.len().min(len as usize)]) else { return false };
    // 80 bytes of fixed fields, then the essence container batch
    value.get(80..).is_some_and(|b| {
        crate::klv::batch_of(b).into_iter().filter_map(crate::klv::ul_of).any(|ul| ul.is_smpte() && ul.item_starts_with(&[0x0D, 0x01, 0x03, 0x01, 0x02, 0x0E]))
    })
}

/// A frame-wrapped ANC element key: 06 0E 2B 34 01 02 01 01 0D 01 03 01 17 cc 02 nn (Table 6).
fn is_anc_element(k: &crate::Ul) -> bool {
    k.0[8..12] == [0x0D, 0x01, 0x03, 0x01] && k.0[12] == 0x17 && k.0[14] == 0x02
}

/// Read every ANC element of the file (the first ANC element stream when there are several).
pub fn read_anc(src: &(impl ByteSource + ?Sized)) -> Result<AncData> {
    let w = walk(src)?;
    let mut warnings = w.warnings.clone();
    let first = w.elements.iter().find(|e| is_anc_element(&e.key)).ok_or_else(|| Error::Invalid("no ANC data elements (ST 436)".into()))?;
    let number: [u8; 4] = first.key.0[12..16].try_into().unwrap_or([0; 4]);
    let edit_rate = best_metadata(&w)
        .ok()
        .and_then(|(_, md)| {
            let desc = md.of_kind(ANC_DATA_DESCRIPTOR).find_map(|d| d.rational(tag::SAMPLE_RATE));
            let track = || {
                md.of_kind(st::TIMELINE_TRACK)
                    .filter(|t| t.u(tag::TRACK_NUMBER) == Some(u32::from_be_bytes(number) as i64))
                    .find_map(|t| t.rational(tag::EDIT_RATE))
            };
            desc.or_else(track).filter(Rational::is_valid)
        })
        .ok_or_else(|| Error::Invalid("the ANC data track has no edit rate".into()))?;
    let mut frames = Vec::new();
    for e in w.elements.iter().filter(|e| e.key.0[12..16] == number) {
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
    Ok(AncData { edit_rate, frames, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{anc_element, build_anc, build_timed_text};

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
}
