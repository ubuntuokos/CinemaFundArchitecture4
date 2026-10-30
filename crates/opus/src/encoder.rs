//! Opus encoder (RFC 6716): CELT-only fullband packets of 20 ms at a constant bit rate.

use crate::celt::encoder::{CeltEncoder, FRAME_SIZE};
use crate::header::OpusHead;
use crate::{Error, Result};

/// TOC config 31: CELT-only, fullband, 20 ms.
const TOC_CELT_FB_20MS: u8 = 31 << 3;

/// Opus encoder for one mono or stereo stream at 48 kHz.
pub struct Encoder {
    celt: CeltEncoder,
    channels: usize,
    bitrate: u32,
}

impl Encoder {
    /// Samples per channel consumed by each [`Encoder::encode`] call.
    pub const FRAME_SIZE: usize = FRAME_SIZE;
    /// Delay of the decoded signal against the input, in 48 kHz samples (the `OpusHead`
    /// pre-skip).
    pub const LOOKAHEAD: usize = crate::celt::tables::OVERLAP;

    /// `channels` is 1 or 2; `bitrate` in bit/s (6 000 ..= 510 000).
    pub fn new(channels: usize, bitrate: u32) -> Result<Encoder> {
        if !(1..=2).contains(&channels) {
            return Err(Error::InvalidArgument("Opus encoder supports 1 or 2 channels"));
        }
        if !(6_000..=510_000).contains(&bitrate) {
            return Err(Error::InvalidArgument("Opus bit rate must be 6..510 kbit/s"));
        }
        Ok(Encoder { celt: CeltEncoder::new(channels), channels, bitrate })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn bitrate(&self) -> u32 {
        self.bitrate
    }

    /// Bytes in every packet (TOC included).
    pub fn packet_bytes(&self) -> usize {
        ((self.bitrate as usize * FRAME_SIZE / 48_000 / 8).clamp(3, 1276)).max(3)
    }

    /// The `OpusHead` describing this stream.
    pub fn head(&self, input_sample_rate: u32) -> OpusHead {
        OpusHead {
            channels: self.channels,
            pre_skip: Self::LOOKAHEAD as u16,
            input_sample_rate,
            output_gain: 0,
            mapping_family: 0,
            streams: 1,
            coupled_streams: self.channels - 1,
            mapping: (0..self.channels as u8).collect(),
        }
    }

    /// Final range coder state of the last packet (matches the decoder's after decoding it).
    pub fn final_range(&self) -> u32 {
        self.celt.final_range()
    }

    /// Encodes [`Self::FRAME_SIZE`] interleaved samples per channel (nominal range ±1.0; a short
    /// slice is padded with silence) into one Opus packet.
    pub fn encode(&mut self, pcm: &[f32]) -> Vec<u8> {
        let bytes = self.packet_bytes();
        let mut out = Vec::with_capacity(bytes);
        out.push(TOC_CELT_FB_20MS | (((self.channels == 2) as u8) << 2));
        out.extend(self.celt.encode(pcm, bytes - 1, self.bitrate));
        out
    }
}
