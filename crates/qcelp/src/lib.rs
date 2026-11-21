// The QCELP decoder and the QCP reader are ported from FFmpeg n7.1.1 (libavcodec/qcelpdec.c,
// qcelpdata.h, celp_filters.c, acelp_filters.c, acelp_vectors.c, lsp.c, libavutil/float_dsp.c and
// libavformat/qcp.c), LGPL-2.1-or-later (`COPYING.LGPL-2.1`), an owner-approved exception (owner
// decision 2026-11-14, CB-38.2); see docs/licensing.md §5. CFA4 changes GPL-3.0-or-later.

//! Pure-Rust QCELP (QCELP-13K, TIA/EIA/IS-733, "PureVoice") speech decoder and QCP (RFC 3625)
//! file reader. The decoder is a port of FFmpeg's, in FFmpeg's float operation order, so its
//! output matches FFmpeg bit for bit. A damaged packet is concealed as an erasure, never a panic.

mod celp;
mod decoder;
mod qcp;
#[allow(clippy::all)]
mod tables;

#[cfg(test)]
mod oracle_tests;

pub use decoder::{Decoder, FRAME_SAMPLES};
pub use qcp::{Qcp, QcpCodec, parse, probe};

/// The sample rate of QCELP.
pub const SAMPLE_RATE: u32 = 8000;

/// The decoded output is capped at this many samples (about 37 hours at 8 kHz).
pub const MAX_SAMPLES: usize = 1 << 30;

/// Errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("not a QCP file")]
    NotQcp,
    #[error("truncated QCP header")]
    Truncated,
    #[error("unknown codec GUID")]
    UnknownCodec,
    #[error("the QCP file holds {0:?}, which is not QCELP-13K")]
    NotQcelp(QcpCodec),
    #[error("no QCELP packets")]
    Empty,
}

/// Decodes the packets of a QCP file into 8 kHz mono samples (full scale about ±1).
pub fn decode_file(data: &[u8]) -> Result<Vec<f32>, Error> {
    let qcp = parse(data)?;
    if qcp.codec != QcpCodec::Qcelp13k {
        return Err(Error::NotQcelp(qcp.codec));
    }
    let mut d = Decoder::new();
    let mut out = Vec::with_capacity(qcp.packets.len().saturating_mul(FRAME_SAMPLES).min(MAX_SAMPLES));
    for p in &qcp.packets {
        d.decode(p, &mut out);
        if out.len() >= MAX_SAMPLES {
            break;
        }
    }
    if out.is_empty() {
        return Err(Error::Empty);
    }
    out.truncate(MAX_SAMPLES);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(rate_map: &[(u8, u8)], packet_size: u16) -> Vec<u8> {
        let mut f = b"RIFF\0\0\0\0QLCMfmt ".to_vec();
        f.extend_from_slice(&150u32.to_le_bytes());
        f.extend_from_slice(&[1, 0, 0x41, 0x6d, 0x7f, 0x5e, 0x15, 0xb1, 0xd0, 0x11, 0xba, 0x91, 0x00, 0x80, 0x5f, 0xb4, 0xb9, 0x7e, 0, 0]);
        f.extend_from_slice(&[b'Q'; 80]);
        for v in [13000u16, packet_size, 160, 8000, 16] {
            f.extend_from_slice(&v.to_le_bytes());
        }
        f.extend_from_slice(&(rate_map.len() as u32).to_le_bytes());
        for &(size, mode) in rate_map {
            f.extend_from_slice(&[size, mode]);
        }
        f.resize(170, 0);
        f
    }

    #[test]
    fn probe_and_header_checks() {
        assert!(!probe(b"RIFF"));
        assert_eq!(parse(b"RIFF\0\0\0\0WAVEfmt xxxx").unwrap_err(), Error::NotQcp);
        let mut h = header(&[(35, 4)], 0);
        h.truncate(30);
        assert_eq!(parse(&h).unwrap_err(), Error::Truncated);
        let mut h = header(&[(35, 4)], 0);
        h[22] = 0x43; // not a QCELP GUID
        assert_eq!(parse(&h).unwrap_err(), Error::UnknownCodec);
    }

    #[test]
    fn damaged_input_never_panics() {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for round in 0..300 {
            let map: Vec<(u8, u8)> = (0..(next() % 12) as u8).map(|m| ((next() % 40) as u8, m % 7)).collect();
            let mut f = header(&map, if round % 3 == 0 { (next() % 40) as u16 } else { 0 });
            f.resize(f.len().max(170), 0);
            f.extend_from_slice(b"vrat");
            f.extend_from_slice(&8u32.to_le_bytes());
            f.extend_from_slice(&((next() % 2) as u32).to_le_bytes());
            f.extend_from_slice(&[0; 4]);
            f.extend_from_slice(b"data");
            let n = (next() % 3000) as usize;
            f.extend_from_slice(&(if round % 5 == 0 { next() as u32 } else { n as u32 }).to_le_bytes());
            f.extend((0..n).map(|_| next() as u8));
            // a cut at any length must not panic either
            let cut = (next() as usize) % (f.len() + 1);
            for data in [&f[..], &f[..cut]] {
                let _ = decode_file(data);
                if let Ok(q) = parse(data) {
                    let mut d = Decoder::new();
                    let mut out = Vec::new();
                    for p in q.packets.iter().take(50) {
                        d.decode(p, &mut out);
                    }
                    assert_eq!(out.len() % FRAME_SAMPLES, 0);
                }
            }
        }
    }

    #[test]
    fn every_packet_size_decodes() {
        let mut d = Decoder::new();
        let mut out = Vec::new();
        for len in 0..80usize {
            for fill in [0u8, 0xFF, 0x5A, 1, 2, 3, 4] {
                let mut p = vec![fill; len];
                if let Some(f) = p.first_mut() {
                    *f = fill % 5;
                }
                d.decode(&p, &mut out);
            }
        }
        assert!(out.iter().all(|v| v.is_finite()));
    }
}
