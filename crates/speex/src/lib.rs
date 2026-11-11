//! Pure-Rust Speex decoder, ported from the float build of the Speex 1.2.1 reference library
//! (`libspeex`, BSD-3-Clause, Xiph.Org Foundation and Jean-Marc Valin; `LICENSE.Speex`).
//!
//! - [`Header`]: the 80-byte Speex header packet (`speex_header.c`).
//! - [`Decoder`]: narrowband (8 kHz), wideband (16 kHz) and ultra-wideband (32 kHz) packets,
//!   every submode, the perceptual enhancer (on, as in `speexdec`) and in-band intensity
//!   stereo, to 16-bit samples exactly as `speex_decode_int` + `speex_decode_stereo_int` give
//!   them.
//!
//! The reference's arithmetic is reproduced to the bit: `f32` where C computes in `float`, `f64`
//! where a `double` literal or a libm call promotes the expression, and the four-lane SSE inner
//! product that Linux builds of libspeex use.
//!
//! Layer L0: depends only on `thiserror`; no `unsafe`, no panics.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

mod bits;
mod dsp;
mod nb;
mod sb;
mod tables;

use bits::Bits;
use nb::{Nb, StereoParams};
use sb::{Low, Sb};

/// Errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("not a Speex header")]
    NotSpeex,
    #[error("unsupported Speex stream: {0}")]
    Unsupported(String),
}

/// The Speex header (`SpeexHeader`, all fields little-endian).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub rate: u32,
    /// 0 narrowband, 1 wideband, 2 ultra-wideband.
    pub mode: u32,
    pub channels: u32,
    pub bitrate: i32,
    pub vbr: bool,
    /// Frames in each packet (at least 1).
    pub frames_per_packet: u32,
    /// Header packets after the comment packet.
    pub extra_headers: u32,
}

/// The largest frame count per packet accepted (the reference allows any; real files use 1–10).
const MAX_FRAMES_PER_PACKET: u32 = 1000;

impl Header {
    pub fn parse(p: &[u8]) -> Result<Header, Error> {
        if p.len() < 80 || !p.starts_with(b"Speex   ") {
            return Err(Error::NotSpeex);
        }
        let i32_at = |o: usize| p.get(o..o + 4).and_then(|b| b.try_into().ok()).map_or(0, i32::from_le_bytes);
        let version_id = i32_at(28);
        let rate = i32_at(36);
        let mode = i32_at(40);
        let bitstream = i32_at(44);
        let channels = i32_at(48);
        let fpp = i32_at(64);
        if !(0..3).contains(&mode) {
            return Err(Error::Unsupported(format!("mode {mode}")));
        }
        if version_id > 1 {
            return Err(Error::Unsupported(format!("version {version_id}")));
        }
        if bitstream != 4 {
            return Err(Error::Unsupported(format!("bitstream version {bitstream}")));
        }
        if rate <= 0 || rate > 192_000 {
            return Err(Error::Unsupported(format!("rate {rate}")));
        }
        Ok(Header {
            rate: rate as u32,
            mode: mode as u32,
            channels: if channels >= 2 { 2 } else { 1 },
            bitrate: i32_at(52),
            vbr: i32_at(60) != 0,
            frames_per_packet: (fpp.max(1) as u32).min(MAX_FRAMES_PER_PACKET),
            extra_headers: i32_at(68).clamp(0, 16) as u32,
        })
    }
}

/// Smoothed stereo gains (`RealSpeexStereoState`).
struct StereoSmooth {
    left: f32,
    right: f32,
}

/// A Speex decoder for one stream.
pub struct Decoder {
    low: Low,
    frame_size: usize,
    channels: u32,
    frames_per_packet: u32,
    smooth: StereoSmooth,
    /// The float output buffer, kept between frames like the reference's.
    buf: Vec<f32>,
}

impl Decoder {
    /// A decoder for the stream `header` describes, with the perceptual enhancer on.
    pub fn new(header: &Header) -> Decoder {
        let (mut low, frame_size) = match header.mode {
            0 => (Low::Nb(Box::new(Nb::new())), nb::FRAME),
            1 => (Low::Sb(Box::new(Sb::new(false))), 2 * nb::FRAME),
            _ => (Low::Sb(Box::new(Sb::new(true))), 4 * nb::FRAME),
        };
        if header.channels == 2 {
            low.nb_mut().stereo = Some(StereoParams { balance: 1.0, e_ratio: 0.5 });
        }
        Decoder {
            low,
            frame_size,
            channels: header.channels,
            frames_per_packet: header.frames_per_packet,
            smooth: StereoSmooth { left: 1.0, right: 1.0 },
            buf: vec![0.0; 2 * frame_size],
        }
    }

    /// Samples per channel in one frame (160, 320 or 640).
    pub fn frame_size(&self) -> usize {
        self.frame_size
    }

    /// The decoder's own delay (`SPEEX_GET_LOOKAHEAD`): 40, 80 or 160 samples.
    pub fn lookahead(&self) -> usize {
        self.frame_size / 4
    }

    pub fn channels(&self) -> u32 {
        self.channels
    }

    /// Turn the perceptual enhancer on or off (`SPEEX_SET_ENH`; on by default).
    pub fn set_enhancer(&mut self, on: bool) {
        self.low.nb_mut().enhance = on;
    }

    /// Decode one packet as `speexdec` does: up to `frames_per_packet` frames, stopping at the end
    /// of the packet or a corrupt frame. Returns interleaved 16-bit samples.
    pub fn decode_packet(&mut self, packet: &[u8]) -> Vec<i16> {
        let mut bits = Bits::new(packet);
        let n = self.frame_size;
        let mut out = Vec::with_capacity(n * self.channels as usize);
        for _ in 0..self.frames_per_packet {
            let ret = match &mut self.low {
                Low::Nb(d) => d.decode(&mut bits, &mut self.buf, None),
                Low::Sb(d) => d.decode(&mut bits, &mut self.buf, None),
            };
            if ret != 0 || bits.remaining() < 0 {
                break;
            }
            let mut frame: Vec<i16> = self.buf[..n].iter().map(|&x| to_i16(x)).collect();
            if self.channels == 2 {
                let st = self.low.nb_mut().stereo.unwrap_or(StereoParams { balance: 1.0, e_ratio: 0.5 });
                frame = self.stereo(&frame, st);
            }
            out.extend_from_slice(&frame);
        }
        out
    }

    /// `speex_decode_stereo_int`.
    fn stereo(&mut self, mono: &[i16], st: StereoParams) -> Vec<i16> {
        let e_right = 1.0f32 / (f64::from(st.e_ratio) * (1.0 + f64::from(st.balance))).sqrt() as f32;
        let e_left = f64::from(st.balance).sqrt() as f32 * e_right;
        let mut out = vec![0i16; 2 * mono.len()];
        for i in (0..mono.len()).rev() {
            let tmp = f32::from(mono[i]);
            self.smooth.left = self.smooth.left * 0.98 + e_left * 0.02;
            self.smooth.right = self.smooth.right * 0.98 + e_right * 0.02;
            out[2 * i] = (self.smooth.left * tmp) as i32 as i16;
            out[2 * i + 1] = (self.smooth.right * tmp) as i32 as i16;
        }
        out
    }
}

/// `speex_decode_int`'s conversion: clamp, then round half up.
fn to_i16(x: f32) -> i16 {
    if x > 32767.0 {
        32767
    } else if x < -32768.0 {
        -32768
    } else {
        (0.5 + f64::from(x)).floor() as i16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(mode: u32, channels: u32) -> Vec<u8> {
        let mut h = b"Speex   speex-1.2.1\0\0\0\0\0\0\0\0\0".to_vec();
        for v in [1i32, 80, [8000, 16000, 32000][mode as usize], mode as i32, 4, channels as i32, -1, 160, 0, 1, 0, 0, 0] {
            h.extend_from_slice(&v.to_le_bytes());
        }
        h
    }

    #[test]
    fn parses_the_header() {
        let h = Header::parse(&header(1, 2)).unwrap();
        assert_eq!((h.rate, h.mode, h.channels, h.frames_per_packet), (16000, 1, 2, 1));
        assert_eq!(Header::parse(b"Speex"), Err(Error::NotSpeex));
        let mut bad = header(0, 1);
        bad[40] = 7;
        assert!(Header::parse(&bad).is_err());
    }

    #[test]
    fn garbage_packets_do_not_panic() {
        for mode in 0..3 {
            let mut d = Decoder::new(&Header::parse(&header(mode, 2)).unwrap());
            let mut seed = 1u32;
            for len in 0..300 {
                let p: Vec<u8> = (0..len)
                    .map(|_| {
                        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                        (seed >> 16) as u8
                    })
                    .collect();
                let out = d.decode_packet(&p);
                assert!(out.len() <= d.frame_size() * 2);
            }
        }
    }

    #[test]
    fn rounding_matches_speex_decode_int() {
        assert_eq!(to_i16(1.5), 2);
        assert_eq!(to_i16(-1.5), -1);
        assert_eq!(to_i16(40000.0), 32767);
        assert_eq!(to_i16(f32::NAN), 0);
    }
}
