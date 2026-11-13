//! AV2 identification (AV2 plan X/4): recognise AV2 video and answer with a clear `unsupported`.
//!
//! CFA4 does not decode AV2 by default yet (the early decoder of ADR 0010 is behind the
//! `av2-external` feature and `CFA4_AV2_DECODER`, see [`crate::av2_external`] when enabled; the
//! native `cfa4-av2` comes later), but an AV2 file must not be mistaken for something else, fed
//! to another decoder or reported as an unknown format. This module recognises
//!
//! - **Annex B** length-delimited bitstreams (AV2 v1.0.0 Annex B, the raw format `avmenc --obu`
//!   writes): a `leb128` OBU size before every OBU, the first OBU a temporal delimiter
//!   (§ 5.2.2 OBU header: `obu_header_extension_flag` f(1), `obu_type` f(5), `obu_tlayer_id`
//!   f(2); temporal delimiter = type 2), followed by a non-picture OBU (sequence header, layer
//!   configuration record, operating point set, MSDO, metadata, …);
//! - **IVF** with the `AV02` fourcc (what AVM writes for `.ivf`);
//! - **MP4** `av02` sample entries (identified in `cfa4-isobmff`; [`crate::make_video_decoder`]
//!   answers them with [`unsupported_message`]).
//!
//! The result is `MediaError::Unsupported`, so the supplementary backends (FFmpeg, GStreamer) may
//! still try the file (ADR 0005); none of them decodes AV2 today. A registered AV2 decoder factory
//! takes precedence over the MP4 answer, so the later adapter needs no change here.

use cfa4_media::{MediaError, SharedSource};

/// OBU type of a temporal delimiter (AV2 v1.0.0 Table 6.1).
const OBU_TEMPORAL_DELIMITER: u8 = 2;
/// Highest assigned OBU type (`OBU_PADDING`); 0 and 26–31 are reserved.
const OBU_TYPE_MAX: u8 = 25;
/// OBU types that may follow the first temporal delimiter before any picture data: sequence
/// header, metadata (short, group), layer configuration record, atlas segment, operating point
/// set, MSDO, quantization matrix, film grain, content interpretation, padding.
const LEADING_OBU_TYPES: &[u8] = &[1, 8, 9, 16, 17, 18, 20, 22, 23, 24, 25];
/// Longest `leb128` AV2 allows (§ 4.10.5: at most 8 bytes).
const LEB128_MAX_BYTES: usize = 8;

/// How an AV2 bitstream is framed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    /// AV2 Annex B length-delimited OBUs (`.obu`).
    AnnexB,
    /// IVF container with the `AV02` fourcc (`.ivf`).
    Ivf,
    /// ISO-BMFF `av02` sample entry (`.mp4`).
    Mp4,
}

impl Framing {
    fn label(self) -> &'static str {
        match self {
            Framing::AnnexB => "Annex B bitstream",
            Framing::Ivf => "IVF",
            Framing::Mp4 => "MP4",
        }
    }
}

/// The user-facing answer for an AV2 file: recognised, not decodable yet.
pub fn unsupported_message(framing: Framing) -> String {
    format!("AV2 video ({}) is recognised, but CFA4 cannot decode AV2 yet", framing.label())
}

/// `leb128()` at the start of `b`: (value, bytes read). `None` when it is cut off, longer than
/// 8 bytes or does not fit in 32 bits (an OBU size never does).
fn leb128(b: &[u8]) -> Option<(u64, usize)> {
    let mut value = 0u64;
    for (i, &byte) in b.iter().take(LEB128_MAX_BYTES).enumerate() {
        value |= u64::from(byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            return (value <= u64::from(u32::MAX)).then_some((value, i + 1));
        }
    }
    None
}

/// (`obu_type`, header length) of an OBU header byte, with the size it was given.
/// `None` for a reserved type or a size too small for its header.
fn obu_header(byte: u8, size: u64) -> Option<(u8, u64)> {
    let ext = u64::from(byte >> 7);
    let obu_type = (byte >> 2) & 0x1f;
    let header_len = 1 + ext;
    (obu_type != 0 && obu_type <= OBU_TYPE_MAX && size >= header_len).then_some((obu_type, header_len))
}

/// Whether `head` starts like an AV2 Annex B bitstream: a temporal delimiter OBU (empty payload)
/// and then an OBU that may lead a temporal unit, with every further OBU header inside `head`
/// well formed. AV1 streams fail: their first OBU header byte (`0x12`/`0x10`) is not a valid
/// AV2 size/header pair.
pub fn sniff_annexb(head: &[u8]) -> bool {
    let mut pos = 0usize;
    let mut index = 0usize;
    while pos < head.len() {
        let Some((size, n)) = head.get(pos..).and_then(leb128) else { return index >= 2 };
        let Some(&byte) = head.get(pos + n) else { return index >= 2 };
        let Some((obu_type, header_len)) = obu_header(byte, size) else { return false };
        match index {
            // a temporal delimiter has no payload: its size is exactly its header
            0 if obu_type != OBU_TEMPORAL_DELIMITER || size != header_len => return false,
            1 if !LEADING_OBU_TYPES.contains(&obu_type) => return false,
            _ => {}
        }
        index += 1;
        let Some(next) = usize::try_from(size).ok().and_then(|s| pos.checked_add(n)?.checked_add(s)) else { return false };
        pos = next;
    }
    index >= 2
}

/// Whether `head` is an IVF file header with the `AV02` fourcc (version 0, 32-byte header).
pub fn sniff_ivf(head: &[u8]) -> bool {
    head.len() >= 32
        && head.starts_with(b"DKIF")
        && head.get(4..6) == Some(&[0, 0][..])
        && head.get(6..8) == Some(&[32, 0][..])
        && head.get(8..12) == Some(&b"AV02"[..])
}

/// How `head` frames AV2 video, if it does.
pub fn sniff(head: &[u8]) -> Option<Framing> {
    if sniff_ivf(head) {
        Some(Framing::Ivf)
    } else if sniff_annexb(head) {
        Some(Framing::AnnexB)
    } else {
        None
    }
}

fn answer(name: &str, head: &[u8]) -> Option<Result<SharedSource, MediaError>> {
    let framing = sniff(head)?;
    Some(Err(MediaError::Unsupported(format!("{name}: {}", unsupported_message(framing)))))
}

/// With the `av2-external` feature and `CFA4_AV2_DECODER` set, an Annex B file opens through the
/// external decoder helper (ADR 0010, AV2-LAB); otherwise `None`.
#[cfg(feature = "av2-external")]
fn external(name: &str, head: &[u8], reader: &cfa4_media::SharedReader) -> Option<Result<SharedSource, MediaError>> {
    if sniff(head)? != Framing::AnnexB {
        return None;
    }
    let helper = crate::av2_external::configured_helper()?;
    Some(crate::av2_external::Av2AnnexBSource::open(name, reader.clone(), helper).map(|s| std::sync::Arc::new(s) as SharedSource).map_err(Into::into))
}

#[cfg(not(feature = "av2-external"))]
fn external(_name: &str, _head: &[u8], _reader: &cfa4_media::SharedReader) -> Option<Result<SharedSource, MediaError>> {
    None
}

/// Byte opener: AV2 Annex B / IVF files are answered with `Unsupported` (Annex B opens through
/// the external decoder when it is configured).
pub fn opener(name: &str, bytes: std::sync::Arc<[u8]>) -> Option<Result<SharedSource, MediaError>> {
    let reader: cfa4_media::SharedReader = std::sync::Arc::new(cfa4_media::reader::MemReader(bytes.clone()));
    external(name, &bytes, &reader).or_else(|| answer(name, &bytes))
}

/// Reader opener: like [`opener`], from the sniffed head only (the file is never read whole).
pub fn reader_opener(name: &str, head: &[u8], reader: &cfa4_media::SharedReader) -> Option<Result<SharedSource, MediaError>> {
    external(name, head, reader).or_else(|| answer(name, head))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first 16 bytes of the AVM v1.0.0 corpus vectors (AV2-00, `avmenc --obu`): a temporal
    /// delimiter (`01 08`) and the sequence header (size, `04`).
    const AVM_HEADS: &[&[u8]] = &[
        &[0x01, 0x08, 0x18, 0x04, 0x80, 0x0a, 0x00, 0x22, 0x2b, 0xf1, 0xf0, 0x79, 0x80, 0x00, 0x00, 0x39],
        &[0x01, 0x08, 0x18, 0x04, 0x80, 0x0c, 0x00, 0x88, 0xaf, 0xc7, 0xc1, 0xe6, 0x00, 0x00, 0x00, 0xe6],
        &[0x01, 0x08, 0x17, 0x04, 0x80, 0x04, 0x80, 0x08, 0x8a, 0xfc, 0x7c, 0x19, 0x80, 0x00, 0x00, 0x3e],
        &[0x01, 0x08, 0x19, 0x04, 0x90, 0x06, 0x80, 0x08, 0x8a, 0xfc, 0x7c, 0x1e, 0x60, 0x00, 0x00, 0x0e],
    ];

    /// A temporal delimiter, a 4-byte sequence header OBU, then a 3-byte closed-loop key OBU.
    fn small_stream() -> Vec<u8> {
        vec![0x01, 0x08, 0x04, 0x04, 0xaa, 0xbb, 0xcc, 0x03, 0x10, 0x01, 0x02]
    }

    #[test]
    fn avm_corpus_heads_are_annex_b() {
        for head in AVM_HEADS {
            assert_eq!(sniff(head), Some(Framing::AnnexB), "{head:02x?}");
        }
        assert_eq!(sniff(&small_stream()), Some(Framing::AnnexB));
    }

    #[test]
    fn temporal_delimiter_with_extension_header() {
        // ext flag set: the delimiter is two bytes (header + extension)
        let s = [0x02, 0x88, 0x00, 0x02, 0x84, 0x00, 0x05];
        assert!(sniff_annexb(&s));
        // ext flag set but only one byte: malformed
        assert!(!sniff_annexb(&[0x01, 0x88, 0x02, 0x04, 0x00]));
    }

    #[test]
    fn other_formats_are_not_av2() {
        // AV1 low-overhead OBU stream (temporal delimiter 0x12 0x00, then a sequence header)
        assert!(!sniff_annexb(&[0x12, 0x00, 0x0a, 0x0b, 0x00, 0x00, 0x00, 0x24, 0xc6, 0xab, 0xdf, 0x3e]));
        // AV1 Annex B (temporal_unit_size, frame_unit_size, obu_length, obu_header 0x10)
        assert!(!sniff_annexb(&[0x20, 0x1e, 0x01, 0x10, 0x0c, 0x08, 0x00, 0x00]));
        // MPEG-2 / H.264 start codes, IVF AV1, MP4, empty
        assert!(sniff(&[0x00, 0x00, 0x01, 0xb3, 0x16, 0x00]).is_none());
        let mut ivf_av1 = b"DKIF\0\0\x20\0AV01".to_vec();
        ivf_av1.resize(32, 0);
        assert!(sniff(&ivf_av1).is_none());
        assert!(sniff(b"\0\0\0\x18ftypisom\0\0\0\0isomiso2").is_none());
        assert!(sniff(&[]).is_none());
        // a lone temporal delimiter is not enough evidence
        assert!(!sniff_annexb(&[0x01, 0x08]));
        // a picture right after the delimiter (no sequence header) is not a stream start
        assert!(!sniff_annexb(&[0x01, 0x08, 0x03, 0x10, 0x01, 0x02]));
        // reserved OBU types (0, 26-31)
        assert!(!sniff_annexb(&[0x01, 0x08, 0x02, 0x00, 0x00]));
        assert!(!sniff_annexb(&[0x01, 0x08, 0x04, 0x04, 0x00, 0x00, 0x00, 0x02, 0x68, 0x00]));
    }

    #[test]
    fn ivf_av02_is_recognised() {
        let mut h = b"DKIF\0\0\x20\0AV02".to_vec();
        h.resize(32, 0);
        assert_eq!(sniff(&h), Some(Framing::Ivf));
        assert!(!sniff_ivf(&h[..31]), "short header");
        h[4] = 1;
        assert!(!sniff_ivf(&h), "unknown IVF version");
    }

    #[test]
    fn hostile_heads_do_not_panic() {
        // unterminated / overlong leb128, size past u32, size overflow, every 1- and 2-byte head
        assert!(!sniff_annexb(&[0x01, 0x08, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]));
        assert!(!sniff_annexb(&[0x01, 0x08, 0xff, 0xff, 0xff, 0xff, 0x7f, 0x04]));
        for a in 0..=255u8 {
            let _ = sniff(&[a]);
            for b in 0..=255u8 {
                let _ = sniff(&[a, b, 0x04, 0x04]);
            }
        }
        // a large OBU cut off by the head length still counts once its header is in the head
        let mut s = vec![0x01, 0x08, 0xff, 0x7f, 0x04];
        s.extend_from_slice(&[0; 32]);
        assert!(sniff_annexb(&s));
    }

    #[test]
    fn openers_answer_unsupported_with_the_framing() {
        let bytes: std::sync::Arc<[u8]> = small_stream().into();
        match opener("clip.obu", bytes.clone()) {
            Some(Err(MediaError::Unsupported(m))) => assert!(m.contains("clip.obu") && m.contains("AV2") && m.contains("Annex B"), "{m}"),
            other => panic!("expected Unsupported, got {:?}", other.map(|r| r.map(|_| ()))),
        }
        let reader: cfa4_media::SharedReader = std::sync::Arc::new(cfa4_media::reader::MemReader(bytes));
        assert!(matches!(reader_opener("clip.obu", &small_stream(), &reader), Some(Err(MediaError::Unsupported(_)))));
        assert!(opener("x.bin", std::sync::Arc::from(&b"hello"[..])).is_none());
    }
}
