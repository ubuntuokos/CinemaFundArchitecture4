//! Clean-room decoders for telephony and early PC audio codecs:
//!
//! - ITU-T G.711 μ-law and A-law ([`g711`]),
//! - ITU-T G.722 sub-band ADPCM at 64 kbit/s ([`g722`]),
//! - IMA ADPCM in its WAV block and QuickTime `ima4` packings ([`ima`]),
//! - Microsoft ADPCM ([`ms`]).
//!
//! [`Codec`] names a stream's format (from a WAV `fmt ` / AVI `strf` / Matroska `A_MS/ACM`
//! `WAVEFORMATEX`, or a QuickTime sample description); [`Decoder`] turns its packets into planar
//! 16-bit samples. The block formats decode each packet on their own; G.722 carries its
//! adaptation state from one packet to the next (see [`Codec::is_stateful`]).

pub mod g711;
pub mod g722;
pub mod ima;
pub mod ms;

/// Decoding errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("truncated {0}")]
    Truncated(&'static str),
    #[error("invalid {0}")]
    Invalid(&'static str),
    #[error("unsupported {0}")]
    Unsupported(String),
}

/// `WAVE_FORMAT_*` tags handled here.
pub const WAVE_FORMAT_ADPCM: u16 = 0x0002;
pub const WAVE_FORMAT_ALAW: u16 = 0x0006;
pub const WAVE_FORMAT_MULAW: u16 = 0x0007;
pub const WAVE_FORMAT_IMA_ADPCM: u16 = 0x0011;
pub const WAVE_FORMAT_G722: u16 = 0x028F;

/// Largest block accepted from a header (bytes); real files use a few kilobytes.
const MAX_BLOCK: usize = 1 << 20;

/// A stream format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Codec {
    MuLaw,
    ALaw,
    /// G.722 at 64 kbit/s (one octet per two 16 kHz samples).
    G722,
    /// IMA ADPCM in `block_align`-byte WAV blocks.
    ImaWav {
        block_align: usize,
    },
    /// QuickTime `ima4`.
    ImaQt,
    /// Microsoft ADPCM in `block_align`-byte blocks with this coefficient set.
    Ms {
        block_align: usize,
        coeffs: Vec<(i32, i32)>,
    },
}

impl Codec {
    /// The codec of a `WAVEFORMATEX` (`tag` already resolved from a `WAVEFORMATEXTENSIBLE`
    /// sub-format), `extra` its `cbSize` bytes. `None` when the tag is not one of ours.
    pub fn from_wave_format(tag: u16, block_align: u16, bits: u16, extra: &[u8]) -> Option<Result<Self, Error>> {
        let block = usize::from(block_align);
        let blocked = |c: Codec| if block == 0 || block > MAX_BLOCK { Err(Error::Invalid("block alignment")) } else { Ok(c) };
        Some(match tag {
            WAVE_FORMAT_MULAW => Ok(Self::MuLaw),
            WAVE_FORMAT_ALAW => Ok(Self::ALaw),
            WAVE_FORMAT_G722 => Ok(Self::G722),
            WAVE_FORMAT_IMA_ADPCM if bits == 4 || bits == 0 => blocked(Self::ImaWav { block_align: block }),
            WAVE_FORMAT_IMA_ADPCM => Err(Error::Unsupported(format!("{bits}-bit IMA ADPCM"))),
            WAVE_FORMAT_ADPCM => blocked(Self::Ms { block_align: block, coeffs: ms::coefficients(extra) }),
            _ => return None,
        })
    }

    /// The codec of a QuickTime / MP4 sound sample description.
    pub fn from_fourcc(fourcc: &[u8; 4]) -> Option<Self> {
        match fourcc {
            b"ulaw" => Some(Self::MuLaw),
            b"alaw" => Some(Self::ALaw),
            b"ima4" => Some(Self::ImaQt),
            _ => None,
        }
    }

    /// Display name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::MuLaw => "G.711 µ-law",
            Self::ALaw => "G.711 A-law",
            Self::G722 => "G.722",
            Self::ImaWav { .. } => "IMA ADPCM",
            Self::ImaQt => "IMA ADPCM (QuickTime)",
            Self::Ms { .. } => "Microsoft ADPCM",
        }
    }

    /// Whether decoding a packet depends on the packets before it: G.722 throughout, QuickTime IMA
    /// on the last sample of the packet before (one packet of pre-roll restores it exactly).
    pub fn is_stateful(&self) -> bool {
        matches!(self, Self::G722 | Self::ImaQt)
    }

    /// Sample frames that `bytes` of stream data decode to.
    pub fn frames_in(&self, bytes: usize, channels: usize) -> usize {
        let ch = channels.max(1);
        match self {
            Self::MuLaw | Self::ALaw => bytes / ch,
            Self::G722 => bytes.saturating_mul(2) / ch,
            Self::ImaQt => bytes / (ima::QT_PACKET * ch) * ima::QT_FRAMES,
            Self::ImaWav { block_align } => blocks(bytes, *block_align, |b| ima::wav_block_frames(b, ch)),
            Self::Ms { block_align, .. } => blocks(bytes, *block_align, |b| ms::block_frames(b, ch)),
        }
    }

    /// The smallest unit packets are cut on: a block, a QuickTime packet group, or a frame.
    pub fn unit_bytes(&self, channels: usize) -> usize {
        let ch = channels.max(1);
        match self {
            Self::MuLaw | Self::ALaw => ch,
            Self::G722 => 1,
            Self::ImaQt => ima::QT_PACKET * ch,
            Self::ImaWav { block_align } | Self::Ms { block_align, .. } => (*block_align).max(1),
        }
    }
}

/// Frames in `bytes` of `block`-byte blocks, the last one possibly short.
fn blocks(bytes: usize, block: usize, frames: impl Fn(usize) -> usize) -> usize {
    let block = block.max(1);
    let full = bytes / block;
    full.saturating_mul(frames(block)).saturating_add(frames(bytes % block))
}

/// A packet decoder producing planar 16-bit samples.
#[derive(Debug, Clone)]
pub struct Decoder {
    codec: Codec,
    channels: usize,
    g711: Option<Box<[i16; 256]>>,
    g722: Option<Box<g722::Decoder>>,
    /// QuickTime `ima4`: each channel's last sample (see [`ima::decode_qt`]).
    qt_last: Vec<Option<i16>>,
}

/// Most channels accepted (the WAV / AVI / Matroska layers bound it further).
const MAX_CHANNELS: usize = 64;

impl Decoder {
    pub fn new(codec: Codec, channels: usize) -> Result<Self, Error> {
        if channels == 0 || channels > MAX_CHANNELS {
            return Err(Error::Invalid("channel count"));
        }
        if codec == Codec::G722 && channels != 1 {
            return Err(Error::Unsupported(format!("{channels}-channel G.722")));
        }
        let g711 = match codec {
            Codec::MuLaw => Some(Box::new(g711::table(false))),
            Codec::ALaw => Some(Box::new(g711::table(true))),
            _ => None,
        };
        let g722 = (codec == Codec::G722).then(Box::default);
        Ok(Self { codec, channels, g711, g722, qt_last: vec![None; channels] })
    }

    pub fn codec(&self) -> &Codec {
        &self.codec
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Back to the start-of-stream state (G.722 and QuickTime IMA carry state between packets).
    pub fn reset(&mut self) {
        if let Some(d) = &mut self.g722 {
            **d = g722::Decoder::new();
        }
        self.qt_last.fill(None);
    }

    /// The G.722 state, to checkpoint and restore around a seek.
    pub fn g722_state(&self) -> Option<&g722::Decoder> {
        self.g722.as_deref()
    }

    /// Restore a G.722 state taken with [`Self::g722_state`].
    pub fn set_g722_state(&mut self, state: &g722::Decoder) {
        if let Some(d) = &mut self.g722 {
            **d = state.clone();
        }
    }

    /// Decode one packet (whole blocks for the block formats; a short last block is decoded as far
    /// as it goes) into planar channels. A damaged block decodes to silence of its length.
    pub fn decode(&mut self, data: &[u8]) -> Result<Vec<Vec<i16>>, Error> {
        let ch = self.channels;
        let frames = self.codec.frames_in(data.len(), ch);
        let mut out: Vec<Vec<i16>> = (0..ch).map(|_| Vec::with_capacity(frames)).collect();
        match &self.codec {
            Codec::MuLaw | Codec::ALaw => {
                let t = self.g711.as_deref().ok_or(Error::Invalid("decoder state"))?;
                for f in data.chunks_exact(ch) {
                    for (o, &b) in out.iter_mut().zip(f) {
                        o.push(t[usize::from(b)]);
                    }
                }
            }
            Codec::G722 => {
                let d = self.g722.as_deref_mut().ok_or(Error::Invalid("decoder state"))?;
                if let Some(o) = out.first_mut() {
                    d.decode_into(data, o);
                }
            }
            Codec::ImaQt => ima::decode_qt(data, ch, &mut self.qt_last, &mut out)?,
            Codec::ImaWav { block_align } => {
                for b in data.chunks((*block_align).max(1)) {
                    if b.len() >= 4 * ch {
                        ima::decode_wav_block(b, ch, &mut out)?;
                    }
                }
            }
            Codec::Ms { block_align, coeffs } => {
                for b in data.chunks((*block_align).max(1)) {
                    // a block naming a predictor the stream doesn't have plays as silence
                    if b.len() >= 7 * ch && ms::decode_block(b, ch, coeffs, &mut out).is_err() {
                        let n = ms::block_frames(b.len(), ch);
                        for o in &mut out {
                            o.resize(o.len() + n, 0);
                        }
                    }
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wave_format_mapping() {
        assert_eq!(Codec::from_wave_format(7, 1, 8, &[]), Some(Ok(Codec::MuLaw)));
        assert_eq!(Codec::from_wave_format(6, 2, 8, &[]), Some(Ok(Codec::ALaw)));
        assert_eq!(Codec::from_wave_format(0x28F, 1, 4, &[]), Some(Ok(Codec::G722)));
        assert_eq!(Codec::from_wave_format(0x11, 1024, 4, &[]), Some(Ok(Codec::ImaWav { block_align: 1024 })));
        assert!(matches!(Codec::from_wave_format(0x11, 1024, 3, &[]), Some(Err(Error::Unsupported(_)))));
        assert!(matches!(Codec::from_wave_format(0x11, 0, 4, &[]), Some(Err(Error::Invalid(_)))));
        let Some(Ok(Codec::Ms { coeffs, .. })) = Codec::from_wave_format(2, 512, 4, &[]) else { panic!("ms") };
        assert_eq!(coeffs, ms::STANDARD_COEFFS.to_vec());
        assert_eq!(Codec::from_wave_format(1, 4, 16, &[]), None);
    }

    #[test]
    fn frame_counts() {
        assert_eq!(Codec::ImaWav { block_align: 1024 }.frames_in(2048, 2), 2 * 1017);
        assert_eq!(Codec::Ms { block_align: 1024, coeffs: vec![] }.frames_in(1024, 1), 2036);
        assert_eq!(Codec::ImaQt.frames_in(68, 2), 64);
        assert_eq!(Codec::G722.frames_in(10, 1), 20);
    }

    #[test]
    fn hostile_packets_never_panic() {
        let mut x = 7u32;
        let mut noise = |n: usize| -> Vec<u8> {
            (0..n)
                .map(|_| {
                    x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    (x >> 16) as u8
                })
                .collect()
        };
        let codecs = [
            Codec::MuLaw,
            Codec::ALaw,
            Codec::ImaQt,
            Codec::ImaWav { block_align: 36 },
            Codec::Ms { block_align: 70, coeffs: ms::STANDARD_COEFFS.to_vec() },
            Codec::Ms { block_align: 70, coeffs: vec![(32767, 32767)] },
        ];
        for c in codecs {
            for ch in [1, 2, 3, 6] {
                let mut d = Decoder::new(c.clone(), ch).expect("decoder");
                for len in [0, 1, 5, 7, 35, 36, 70, 71, 1000] {
                    let _ = d.decode(&noise(len));
                }
            }
        }
        let mut g = Decoder::new(Codec::G722, 1).expect("g722");
        assert_eq!(g.decode(&noise(9)).expect("g722 decode")[0].len(), 18);
        assert!(Decoder::new(Codec::G722, 2).is_err());
        assert!(Decoder::new(Codec::MuLaw, 0).is_err());
    }
}
