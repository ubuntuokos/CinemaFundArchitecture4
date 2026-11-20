//! MPEG-5 Part 2 LCEVC (ISO/IEC 23094-2): detection of the enhancement layer and an own
//! decoder for it ([`Decoder`], a port of V-Nova's LCEVCdec 4.3.1, see `README.md`). LCEVC adds
//! an enhancement layer on top of a base video (H.264, HEVC, …); [`extract_lcevc`] takes it out
//! of a base sample's T.35 SEI and [`Decoder::decode`] applies it to the decoded base picture.
//!
//! Carriage, as V-Nova's public carriage documentation describes it
//! (<https://docs.v-nova.com/technologies/mpeg-5.lcevc/carriage/overview-of-lcevc-carriages>):
//!
//! - **In the base stream**: one LCEVC access unit per picture as the payload of a
//!   `user_data_registered_itu_t_t35` SEI message (payload type 4) in an H.264 SEI NAL unit
//!   (type 6) or an HEVC prefix SEI NAL unit (type 39), with the UK country code `0xB4` and the
//!   UK register bytes `0x00 0x50 0x00` (V-Nova's manufacturer code).
//! - **As its own stream**: an ISOBMFF track with the `lvc1` sample entry (ISO/IEC 14496-15,
//!   linked to its base track by a `sbas` track reference), or an MPEG-2 TS elementary stream
//!   of `stream_type` 0x36 (ISO/IEC 13818-1).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

mod bits;
mod config;
mod decoder;
mod entropy;
mod residual;

pub use config::Chroma;
pub use decoder::{Decoded, Decoder, Picture};

/// Why an LCEVC access unit could not be decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Damaged or non-conforming data.
    Invalid(&'static str),
    /// A valid feature the decoder does not implement (tiles, field pictures, dithering).
    Unsupported(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Invalid(m) => write!(f, "invalid LCEVC data: {m}"),
            Error::Unsupported(m) => write!(f, "unsupported LCEVC feature: {m}"),
        }
    }
}

impl std::error::Error for Error {}

/// ISOBMFF sample entry of an LCEVC enhancement track.
pub const SAMPLE_ENTRY: [u8; 4] = *b"lvc1";
/// MPEG-2 TS `stream_type` of an LCEVC elementary stream.
pub const TS_STREAM_TYPE: u8 = 0x36;

/// SEI payload type `user_data_registered_itu_t_t35`.
const SEI_T35: u32 = 4;
/// `itu_t_t35_country_code` (United Kingdom) and the UK register bytes V-Nova's LCEVC uses.
const T35_PREFIX: [u8; 4] = [0xB4, 0x00, 0x50, 0x00];

/// The base codec whose NAL units are scanned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseCodec {
    H264,
    Hevc,
}

/// How a sample's NAL units are delimited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    /// Start codes (`00 00 01`), as in MPEG-TS, raw `.h264` / `.hevc` and AVI.
    AnnexB,
    /// A big-endian length of 1-4 bytes before each NAL unit, as in MP4 and Matroska.
    LengthPrefixed(u8),
}

/// True when `sample` (one access unit) carries LCEVC enhancement data in a T.35 SEI message.
pub fn sample_has_lcevc(sample: &[u8], codec: BaseCodec, framing: Framing) -> bool {
    let mut found = false;
    for_each_nal(sample, framing, |nal| {
        if !found && is_sei(nal, codec) {
            found = sei_lcevc_payload(nal, codec).is_some();
        }
    });
    found
}

/// The LCEVC access unit carried in `sample`'s first LCEVC T.35 SEI message, as an Annex B NAL
/// unit for [`Decoder::decode`]: the SEI emulation prevention removed, and a start code put in
/// front when the payload has none (LCEVCdec `LCEVC_extractEnhancementFromNAL`).
pub fn extract_lcevc(sample: &[u8], codec: BaseCodec, framing: Framing) -> Option<Vec<u8>> {
    let mut out = None;
    for_each_nal(sample, framing, |nal| {
        if out.is_none() && is_sei(nal, codec) {
            out = sei_lcevc_payload(nal, codec);
        }
    });
    let payload = out?;
    if payload.starts_with(&[0, 0, 1]) || payload.starts_with(&[0, 0, 0, 1]) {
        Some(payload)
    } else {
        let mut v = Vec::with_capacity(payload.len() + 3);
        v.extend_from_slice(&[0, 0, 1]);
        v.extend_from_slice(&payload);
        Some(v)
    }
}

/// True when any of `samples` carries LCEVC (see [`sample_has_lcevc`]).
pub fn samples_have_lcevc<'a>(samples: impl IntoIterator<Item = &'a [u8]>, codec: BaseCodec, framing: Framing) -> bool {
    samples.into_iter().any(|s| sample_has_lcevc(s, codec, framing))
}

fn is_sei(nal: &[u8], codec: BaseCodec) -> bool {
    match (codec, nal.first()) {
        (BaseCodec::H264, Some(&b)) => b & 0x1f == 6,
        (BaseCodec::Hevc, Some(&b)) => (b >> 1) & 0x3f == 39,
        _ => false,
    }
}

fn for_each_nal(data: &[u8], framing: Framing, mut f: impl FnMut(&[u8])) {
    match framing {
        Framing::LengthPrefixed(n) => {
            let n = usize::from(n.clamp(1, 4));
            let mut rest = data;
            while rest.len() > n {
                let len = rest.iter().take(n).fold(0usize, |acc, &b| (acc << 8) | usize::from(b));
                let Some(nal) = rest.get(n..n + len) else {
                    f(rest.get(n..).unwrap_or(&[]));
                    return;
                };
                f(nal);
                rest = rest.get(n + len..).unwrap_or(&[]);
            }
        }
        Framing::AnnexB => {
            // positions just after each 00 00 01
            let mut starts = Vec::new();
            let mut i = 0;
            while i + 2 < data.len() {
                if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
                    starts.push(i + 3);
                    i += 3;
                } else {
                    i += 1;
                }
            }
            for (k, &s) in starts.iter().enumerate() {
                let mut end = starts.get(k + 1).map_or(data.len(), |&n| n - 3);
                // a four-byte start code leaves a zero byte behind the previous unit
                while end > s && data.get(end - 1) == Some(&0) {
                    end -= 1;
                }
                if let Some(nal) = data.get(s..end) {
                    f(nal);
                }
            }
        }
    }
}

/// The RBSP of a NAL unit payload: emulation prevention bytes (`00 00 03`) removed.
fn rbsp(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len());
    let mut zeros = 0;
    for &b in payload {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}

/// Walk the SEI messages of an SEI NAL unit and return the first LCEVC T.35 payload (the bytes
/// after the T.35 prefix).
fn sei_lcevc_payload(nal: &[u8], codec: BaseCodec) -> Option<Vec<u8>> {
    let header = match codec {
        BaseCodec::H264 => 1,
        BaseCodec::Hevc => 2,
    };
    let data = rbsp(nal.get(header..).unwrap_or(&[]));
    let mut p = 0usize;
    // each message: payload type and size as runs of 0xFF plus a last byte
    let read_value = |p: &mut usize| -> Option<u32> {
        let mut v = 0u32;
        loop {
            let b = *data.get(*p)?;
            *p += 1;
            v = v.saturating_add(u32::from(b));
            if b != 0xFF {
                return Some(v);
            }
        }
    };
    // stop at the RBSP trailing bits (0x80 alone)
    while data.get(p).is_some_and(|&b| b != 0x80) || data.len() > p + 1 {
        let (Some(ty), Some(size)) = (read_value(&mut p), read_value(&mut p)) else { return None };
        let size = size as usize;
        let payload = data.get(p..p.saturating_add(size))?;
        if ty == SEI_T35 && payload.starts_with(&T35_PREFIX) {
            return payload.get(T35_PREFIX.len()..).map(<[u8]>::to_vec);
        }
        p += size;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An SEI NAL unit (header `hdr`) with one T.35 message.
    fn t35_sei(hdr: &[u8], body: &[u8]) -> Vec<u8> {
        let mut v = hdr.to_vec();
        v.push(4);
        v.push(body.len() as u8);
        v.extend_from_slice(body);
        v.push(0x80);
        v
    }

    fn annexb(nals: &[&[u8]]) -> Vec<u8> {
        nals.iter().flat_map(|n| [&[0u8, 0, 0, 1][..], n].concat()).collect()
    }

    fn prefixed(nals: &[&[u8]]) -> Vec<u8> {
        nals.iter().flat_map(|n| [&(n.len() as u32).to_be_bytes()[..], n].concat()).collect()
    }

    const LCEVC_T35: &[u8] = &[0xB4, 0x00, 0x50, 0x00, 0x79, 0xFF, 0x12, 0x34];
    const HDR10P_T35: &[u8] = &[0xB5, 0x00, 0x3C, 0x00, 0x01, 0x04, 0x01];

    #[test]
    fn finds_lcevc_sei_in_h264_and_hevc() {
        let idr = [0x65u8, 0x88, 0x80, 0x10];
        let h264 = t35_sei(&[0x06], LCEVC_T35);
        assert!(sample_has_lcevc(&annexb(&[&[0x09, 0xF0], &h264, &idr]), BaseCodec::H264, Framing::AnnexB));
        assert!(sample_has_lcevc(&prefixed(&[&h264, &idr]), BaseCodec::H264, Framing::LengthPrefixed(4)));
        let hevc = t35_sei(&[39 << 1, 0x01], LCEVC_T35);
        assert!(sample_has_lcevc(&prefixed(&[&hevc]), BaseCodec::Hevc, Framing::LengthPrefixed(4)));
        // the same bytes under the other codec's NAL header are not an SEI there
        assert!(!sample_has_lcevc(&prefixed(&[&hevc]), BaseCodec::H264, Framing::LengthPrefixed(4)));
    }

    #[test]
    fn other_t35_payloads_and_second_messages() {
        let hdr10p = t35_sei(&[0x06], HDR10P_T35);
        assert!(!sample_has_lcevc(&annexb(&[&hdr10p]), BaseCodec::H264, Framing::AnnexB));
        // a recovery point message (type 6) first, then the LCEVC one
        let mut two = vec![0x06, 6, 1, 0x84, 4, LCEVC_T35.len() as u8];
        two.extend_from_slice(LCEVC_T35);
        two.push(0x80);
        assert!(sample_has_lcevc(&annexb(&[&two]), BaseCodec::H264, Framing::AnnexB));
    }

    #[test]
    fn emulation_prevention_is_removed() {
        // country code then 00 00 03 01: the payload is B4 00 50 00 00 01 after unescaping
        let mut nal = vec![0x06, 4, 6, 0xB4, 0x00, 0x50, 0x00, 0x00, 0x03, 0x01, 0x80];
        assert!(sample_has_lcevc(&annexb(&[&nal]), BaseCodec::H264, Framing::AnnexB));
        nal[5] = 0x51;
        assert!(!sample_has_lcevc(&annexb(&[&nal]), BaseCodec::H264, Framing::AnnexB));
    }

    #[test]
    fn extracts_the_access_unit_with_a_start_code() {
        let h264 = t35_sei(&[0x06], LCEVC_T35);
        let au = extract_lcevc(&annexb(&[&h264]), BaseCodec::H264, Framing::AnnexB);
        assert_eq!(au.as_deref(), Some(&[0, 0, 1, 0x79, 0xFF, 0x12, 0x34][..]));
        // a payload that already starts with a start code keeps it
        // (the size counts the payload without its emulation prevention byte)
        let with_sc = [39 << 1, 0x01, 4, 9, 0xB4, 0x00, 0x50, 0x00, 0x00, 0x00, 0x03, 0x01, 0x79, 0x80, 0x80];
        let au = extract_lcevc(&prefixed(&[&with_sc]), BaseCodec::Hevc, Framing::LengthPrefixed(4));
        assert_eq!(au.as_deref(), Some(&[0, 0, 1, 0x79, 0x80][..]));
        assert_eq!(extract_lcevc(&annexb(&[&t35_sei(&[0x06], HDR10P_T35)]), BaseCodec::H264, Framing::AnnexB), None);
    }

    /// Random and mutated LCEVC access units over a matching base picture: errors, never a panic.
    #[test]
    fn damaged_access_units_never_panic() {
        // global (IDR) configuration for a 32×16 4:2:0 8-bit picture over a 16×8 base, then a
        // picture configuration and an encoded-data block with all chunks off
        let au: Vec<u8> = vec![0, 0, 1, 0x7B, 0xFF, 0xE1, 0x08, 0x7E, 0x40, 0x08, 0x80, 0x00, 0x20, 0x00, 0x10, 0x62, 0x10, 0x07, 0xD0, 0x43, 0x00, 0x00, 0x80];
        let base = Picture { width: 16, height: 8, bit_depth: 8, chroma: Chroma::C420, planes: vec![vec![100; 128], vec![128; 32], vec![128; 32]] };
        let mut d = Decoder::new();
        let out = d.decode(&au, &base).expect("clean access unit decodes");
        assert_eq!((out.picture.width, out.picture.height), (32, 16));
        assert!(out.picture.planes[0].iter().all(|&v| v == 100));
        assert_eq!(Decoder::probe(&au), Some((32, 16)));
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for round in 0..20_000 {
            let mut m = au.clone();
            let edits = 1 + next() % 4;
            for _ in 0..edits {
                let i = 5 + (next() as usize) % (m.len() - 6);
                match next() % 3 {
                    0 => m[i] = next() as u8,
                    1 => m.insert(i, next() as u8),
                    _ => {
                        m.remove(i);
                    }
                }
            }
            if round % 7 == 0 {
                let n = (next() % 64) as usize;
                m = [&[0u8, 0, 1, 0x7B, 0xFF][..], &(0..n).map(|_| next() as u8).collect::<Vec<_>>(), &[0x80]].concat();
            }
            let _ = d.decode(&m, &base);
            let _ = d.upscale_only(&base);
            let _ = Decoder::probe(&m);
        }
    }

    #[test]
    fn damaged_input_never_panics() {
        let good = annexb(&[&t35_sei(&[0x06], LCEVC_T35)]);
        let mut x = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..2000 {
            let mut d = good.clone();
            for _ in 0..4 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                let i = (x as usize) % d.len();
                d[i] = (x >> 32) as u8;
            }
            d.truncate((x as usize >> 8) % (d.len() + 1));
            for f in [Framing::AnnexB, Framing::LengthPrefixed(1), Framing::LengthPrefixed(4)] {
                let _ = sample_has_lcevc(&d, BaseCodec::H264, f);
                let _ = sample_has_lcevc(&d, BaseCodec::Hevc, f);
            }
        }
        assert!(!sample_has_lcevc(&[], BaseCodec::H264, Framing::AnnexB));
    }
}
