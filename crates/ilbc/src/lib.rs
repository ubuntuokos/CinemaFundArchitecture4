// Ported from WebRTC's iLBC (libilbc v3.0.4), BSD-3-Clause,
// Copyright (c) 2011-2013 The WebRTC project authors (`LICENSE.webrtc`, `PATENTS.webrtc`).
// CFA4 changes GPL-3.0-or-later.

//! Pure-Rust iLBC (RFC 3951) speech decoder, ported from WebRTC's fixed-point decoder with the
//! enhancer off, so the output matches ffmpeg's `ilbc` decoder sample for sample.
//!
//! [`decode_file`] reads the RFC 3952 storage format (`#!iLBC20\n` or `#!iLBC30\n`, then frames
//! of 38 or 50 bytes) or, without a header, raw 30 ms frames.

mod decoder;
mod tables;

#[cfg(test)]
mod oracle_tests;

pub use decoder::{Decoder, Mode};

/// The sample rate of iLBC.
pub const SAMPLE_RATE: u32 = 8000;

/// The decoded output is capped at this many samples (about 37 hours at 8 kHz).
pub const MAX_SAMPLES: usize = 1 << 30;

/// Errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("no iLBC frames")]
    Empty,
}

/// The frame mode and the frame data of a file.
pub fn parse_header(data: &[u8]) -> (Mode, &[u8]) {
    if let Some(rest) = data.strip_prefix(b"#!iLBC20\n") {
        (Mode::Ms20, rest)
    } else if let Some(rest) = data.strip_prefix(b"#!iLBC30\n") {
        (Mode::Ms30, rest)
    } else {
        (Mode::Ms30, data)
    }
}

/// Decodes a whole iLBC file into 8 kHz 16-bit samples.
pub fn decode_file(data: &[u8]) -> Result<Vec<i16>, Error> {
    let (mode, frames) = parse_header(data);
    let mut d = Decoder::new(mode);
    let n = frames.len() / mode.frame_bytes();
    let mut out = Vec::with_capacity(n.saturating_mul(mode.frame_samples()).min(MAX_SAMPLES));
    for f in frames.chunks_exact(mode.frame_bytes()) {
        d.decode(Some(f), &mut out);
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

    #[test]
    fn headers_select_the_mode() {
        assert_eq!(parse_header(b"#!iLBC20\nab").0, Mode::Ms20);
        assert_eq!(parse_header(b"#!iLBC30\nab").0, Mode::Ms30);
        assert_eq!(parse_header(b"ab"), (Mode::Ms30, &b"ab"[..]));
    }

    #[test]
    fn damaged_input_never_panics() {
        assert_eq!(decode_file(&[]), Err(Error::Empty));
        assert_eq!(decode_file(b"#!iLBC20\n123"), Err(Error::Empty));
        let mut x = 0x9e37_79b9u32;
        for len in [37usize, 38, 50, 100, 1000, 4096] {
            let data: Vec<u8> = (0..len)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    x as u8
                })
                .collect();
            for head in [&b"#!iLBC20\n"[..], b"#!iLBC30\n", b""] {
                let mut f = head.to_vec();
                f.extend_from_slice(&data);
                let r = decode_file(&f);
                if let Ok(s) = r {
                    assert_eq!(s.len() % 80, 0);
                }
            }
        }
    }
}
