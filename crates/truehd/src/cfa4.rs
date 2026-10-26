//! CFA4: the decoder interface the rest of CFA4 uses (not part of the upstream crate).
//!
//! [`Decoder::decode`] takes container packets (one or more whole access units, or a raw `.thd`
//! / `.mlp` stream in pieces) and gives planar `f32` channels in the WAV / SMPTE order used across
//! CFA4 (L R C LFE back-L back-R side-L side-R, then any further channels). The largest
//! channel-based presentation of at most eight channels is decoded: 7.1 (or 5.1.2 …) when the
//! stream has one, else 5.1, else stereo. The 16-channel object presentation of Dolby Atmos needs
//! an object renderer and is not decoded; Atmos streams carry a 7.1 (or 5.1) bed presentation
//! that is.

use crate::process::decode::Decoder as AuDecoder;
use crate::process::extract::Extractor;
use crate::process::parse::Parser;
use crate::process::{PresentationMap, PresentationType};
use crate::structs::channel::ChannelLabel;

/// Most channels of a decoded presentation.
pub const MAX_CHANNELS: usize = 8;

/// Decoder errors. Malformed input gives an error, never a panic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("TrueHD: {0}")]
    Decode(String),
}

/// Decoded audio of one or more access units.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Decoded {
    pub sample_rate: u32,
    /// Planar samples in -1.0 ..= 1.0, in the order of [`Decoded::labels`].
    pub channels: Vec<Vec<f32>>,
    /// The channels' speaker positions; `None` for a channel the stream's channel assignment
    /// does not name (such as the back centre of 4.0, which the six-channel assignment cannot
    /// express). Unnamed channels follow the named ones in stream order.
    pub labels: Vec<Option<ChannelLabel>>,
}

/// The rank of a channel in CFA4's output order.
fn rank(label: ChannelLabel) -> u8 {
    use ChannelLabel::*;
    match label {
        L => 0,
        R => 1,
        C => 2,
        LFE => 3,
        Lb => 4,
        Rb => 5,
        Ls => 6,
        Rs => 7,
        Cb => 8,
        Lsc => 9,
        Rsc => 10,
        Lw => 11,
        Rw => 12,
        Lsd => 13,
        Rsd => 14,
        Tfl => 15,
        Tfr => 16,
        Tfc => 17,
        Tc => 18,
        Tsl => 19,
        Tsr => 20,
        Tbl => 21,
        Tbr => 22,
        LFE2 => 23,
    }
}

/// Full scale of the decoder's 24-bit output.
const SCALE: f32 = 1.0 / 8_388_608.0;

/// A TrueHD / MLP stream decoder.
#[derive(Default)]
pub struct Decoder {
    extractor: Extractor,
    parser: Parser,
    decoder: AuDecoder,
    /// Presentation decoded, chosen at each major sync.
    presentation: Option<usize>,
}

/// The presentation to decode: the largest channel-based one (2, 6 or 8 channels) the stream
/// declares.
fn pick(map: &PresentationMap) -> Option<usize> {
    (0..3).rev().find_map(|i| match map.presentation_type_by_index(i) {
        PresentationType::Invalid => None,
        PresentationType::CopyOf(j) => Some(j),
        _ => Some(i),
    })
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decodes the access units in `data`. Bytes of an access unit split across calls are kept
    /// for the next call; access units before the first major sync give no audio.
    pub fn decode(&mut self, data: &[u8]) -> Result<Decoded, Error> {
        self.extractor.push_bytes(data);
        let mut out = Decoded::default();
        let mut order: Vec<usize> = Vec::new();
        while let Some(frame) = self.extractor.next() {
            let Ok(frame) = frame else { continue };
            let au = match self.parser.parse(&frame) {
                Ok(au) => au,
                Err(e) => {
                    self.resync();
                    return Err(Error::Decode(e.to_string()));
                }
            };
            if let Some(ms) = &au.major_sync_info {
                let map = PresentationMap::for_format_sync(ms.format_sync, ms.substream_info, ms.extended_substream_info);
                self.presentation = pick(&map);
            }
            let Some(presentation) = self.presentation else { continue };
            let d = match self.decoder.decode_presentation(&au, presentation) {
                Ok(d) => d,
                Err(e) => {
                    self.resync();
                    return Err(Error::Decode(e.to_string()));
                }
            };
            let channels = d.channel_count.min(MAX_CHANNELS);
            let mut sorted: Vec<usize> = (0..channels).collect();
            sorted.sort_by_key(|&c| d.channel_labels.get(c).map_or(u8::MAX, |&l| rank(l)));
            let labels: Vec<Option<ChannelLabel>> = sorted.iter().map(|&c| d.channel_labels.get(c).copied()).collect();
            if out.channels.is_empty() || out.labels != labels || out.sample_rate != d.sampling_frequency {
                // a new layout ends what this call returns; keep the latest
                order = sorted;
                out = Decoded { sample_rate: d.sampling_frequency, channels: vec![Vec::new(); channels], labels };
            }
            let samples = d.pcm_data.get(..d.sample_length).unwrap_or_default();
            for (o, &c) in out.channels.iter_mut().zip(&order) {
                o.extend(samples.iter().map(|s| s.get(c).copied().unwrap_or(0) as f32 * SCALE));
            }
        }
        Ok(out)
    }

    /// Drops all stream state (after a seek); decoding resumes at the next major sync.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    fn resync(&mut self) {
        self.parser.reset_for_next_major_sync();
        self.decoder.reset_for_next_major_sync();
        self.presentation = None;
    }
}

/// The channel layout of the presentation [`Decoder`] decodes, from the first major sync in
/// `data`, in output order. For probing a stream's channel count without decoding it.
pub fn stream_labels(data: &[u8]) -> Result<(u32, Vec<Option<ChannelLabel>>), Error> {
    let mut dec = Decoder::new();
    let d = dec.decode(data)?;
    if d.channels.is_empty() {
        return Err(Error::Decode("no decodable access unit".into()));
    }
    Ok((d.sample_rate, d.labels))
}

/// Samples per access unit at `rate`: 40 at 44.1 / 48 kHz, 80 at 88.2 / 96 kHz, 160 at 176.4 /
/// 192 kHz. `None` for a rate TrueHD does not have.
pub fn samples_per_au(rate: u32) -> Option<i64> {
    match rate {
        44_100 | 48_000 => Some(40),
        88_200 | 96_000 => Some(80),
        176_400 | 192_000 => Some(160),
        _ => None,
    }
}

/// Access units a decoder may have to read before it reaches a major sync: a stream repeats the
/// major sync at least every 128 access units, so starting this many access units before a
/// position gives the decoder its state there.
pub const MAJOR_SYNC_INTERVAL: i64 = 129;

/// Samples a packet of whole access units (as Matroska and MP4 store them) decodes to at `rate`:
/// the access units are chained by the length in each one's first two bytes. `None` when the
/// chain does not end at the end of the packet.
pub fn packet_samples(data: &[u8], rate: u32) -> Option<i64> {
    let per_au = samples_per_au(rate)?;
    let (mut p, mut n) = (0usize, 0i64);
    while p < data.len() {
        let [a, b] = *data.get(p..p + 2)?.first_chunk::<2>()?;
        let len = ((usize::from(a & 0x0F) << 8) | usize::from(b)) * 2;
        if len < 4 {
            return None;
        }
        p += len;
        n += 1;
    }
    (p == data.len() && n > 0).then_some(n * per_au)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_samples_follow_the_access_unit_chain() {
        // two access units of 8 and 6 bytes
        let au = [0x10, 0x04, 0, 0, 0, 0, 0, 0, 0x20, 0x03, 0, 0, 0, 0];
        assert_eq!(packet_samples(&au, 48_000), Some(80));
        assert_eq!(packet_samples(&au, 96_000), Some(160));
        assert_eq!(packet_samples(&au[..10], 48_000), None);
        assert_eq!(packet_samples(&au, 32_000), None);
        assert_eq!(packet_samples(&[], 48_000), None);
        assert_eq!(packet_samples(&[0x10, 0x01], 48_000), None);
    }

    #[test]
    fn the_example_stream_decodes_through_the_cfa4_interface() {
        let mut dec = Decoder::new();
        let d = dec.decode(crate::process::EXAMPLE_DATA).unwrap_or_default();
        assert!(!d.channels.is_empty());
        assert_eq!(d.labels.len(), d.channels.len());
        assert!(d.channels.iter().all(|c| c.len() == d.channels[0].len()));
    }
}
