//! CFA4: the decoder interface the rest of CFA4 uses (not part of the upstream crate).
//!
//! [`Decoder::decode`] takes container packets (one or more whole access units, or a raw `.thd`
//! / `.mlp` stream in pieces) and gives planar `f32` channels in the WAV / SMPTE order used across
//! CFA4 (L R C LFE back-L back-R side-L side-R, then any further channels). The largest
//! channel-based presentation of at most eight channels is decoded: 7.1 (or 5.1.2 …) when the
//! stream has one, else 5.1, else stereo. Atmos streams carry such a bed presentation (7.1 or 5.1)
//! beside their 16-channel object presentation.
//!
//! [`Decoder::with_objects`] decodes the object presentation instead and renders its objects to a
//! loudspeaker [`Layout`] (up to 7.1.4) with [`crate::render`]. A stream without an object
//! presentation, or with objects the renderer cannot place, gives its channel presentation then.
//!
//! [`ObjectDecoder`] gives the object presentation itself, unrendered: one mono signal per object
//! and the objects' metadata updates ([`crate::render::Event`]) on the same sample clock, for a
//! caller that keeps the objects as objects (an IAB track of a DCP).

use crate::process::decode::Decoder as AuDecoder;
use crate::process::extract::Extractor;
use crate::process::parse::Parser;
use crate::process::{PresentationMap, PresentationType};
use crate::render::{Layout, Renderer};
use crate::structs::channel::ChannelLabel;

/// Most channels of a decoded channel presentation.
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
    /// The object renderer, when objects are wanted.
    renderer: Option<Renderer>,
    /// Whether the object presentation is decoded (chosen at each major sync).
    objects: bool,
    /// The stream's objects could not be placed: its channel presentation is decoded instead.
    refused: bool,
    /// Samples rendered from the object presentation: the clock object metadata is timed by.
    position: u64,
}

/// The object presentation of an Atmos stream.
const OBJECT_PRESENTATION: usize = 3;

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

    /// A decoder that renders the object presentation of Dolby Atmos streams to `layout`; other
    /// streams decode as with [`Decoder::new`].
    pub fn with_objects(layout: Layout) -> Self {
        Self { renderer: Some(Renderer::new(layout)), ..Self::default() }
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
                self.objects =
                    self.renderer.is_some() && !self.refused && matches!(map.presentation_type_by_index(OBJECT_PRESENTATION), PresentationType::Independent);
            }
            let Some(presentation) = self.presentation else { continue };
            let presentation = if self.objects { OBJECT_PRESENTATION } else { presentation };
            let d = match self.decoder.decode_presentation(&au, presentation) {
                Ok(d) => d,
                Err(e) => {
                    self.resync();
                    return Err(Error::Decode(e.to_string()));
                }
            };
            let samples = d.pcm_data.get(..d.sample_length).unwrap_or_default();
            if self.objects {
                let Some(renderer) = self.renderer.as_mut() else { continue };
                let mut events = Vec::new();
                for p in &d.oamd {
                    match crate::render::events(p, d.channel_count) {
                        Some(e) => events.extend(e),
                        None => self.refused = true,
                    }
                }
                if self.refused {
                    // the channel presentation takes over at the next major sync
                    renderer.reset();
                    self.objects = false;
                    self.presentation = None;
                    self.parser.reset_for_next_major_sync();
                    self.decoder.reset_for_next_major_sync();
                    continue;
                }
                for e in events {
                    renderer.schedule(self.position.saturating_add(e.offset), e.object, &e.state, e.ramp);
                }
                let labels: Vec<Option<ChannelLabel>> = renderer.layout().labels().into_iter().map(Some).collect();
                if out.channels.is_empty() || out.labels != labels || out.sample_rate != d.sampling_frequency {
                    out = Decoded { sample_rate: d.sampling_frequency, channels: vec![Vec::new(); labels.len()], labels };
                }
                let objects: Vec<Vec<f32>> = (0..d.channel_count.min(crate::render::MAX_OBJECTS))
                    .map(|c| samples.iter().map(|s| s.get(c).copied().unwrap_or(0) as f32 * SCALE).collect())
                    .collect();
                let objects: Vec<&[f32]> = objects.iter().map(Vec::as_slice).collect();
                renderer.render(self.position, &objects, &mut out.channels);
                self.position = self.position.saturating_add(samples.len() as u64);
                continue;
            }
            let channels = d.channel_count.min(MAX_CHANNELS);
            let mut sorted: Vec<usize> = (0..channels).collect();
            sorted.sort_by_key(|&c| d.channel_labels.get(c).map_or(u8::MAX, |&l| rank(l)));
            let labels: Vec<Option<ChannelLabel>> = sorted.iter().map(|&c| d.channel_labels.get(c).copied()).collect();
            if out.channels.is_empty() || out.labels != labels || out.sample_rate != d.sampling_frequency {
                // a new layout ends what this call returns; keep the latest
                order = sorted;
                out = Decoded { sample_rate: d.sampling_frequency, channels: vec![Vec::new(); channels], labels };
            }
            for (o, &c) in out.channels.iter_mut().zip(&order) {
                o.extend(samples.iter().map(|s| s.get(c).copied().unwrap_or(0) as f32 * SCALE));
            }
        }
        Ok(out)
    }

    /// Drops all stream state (after a seek); decoding resumes at the next major sync.
    pub fn reset(&mut self) {
        let renderer = self.renderer.take().map(|r| Renderer::new(r.layout().clone()));
        *self = Self { renderer, ..Self::default() };
    }

    /// After a bad access unit: the rest of the failed call's bytes are dropped too, so they
    /// cannot come out with the next call's packet.
    fn resync(&mut self) {
        self.extractor = Default::default();
        self.parser.reset_for_next_major_sync();
        self.decoder.reset_for_next_major_sync();
        self.presentation = None;
        if let Some(r) = self.renderer.as_mut() {
            r.reset();
        }
    }
}

/// Object audio of one or more access units, unrendered.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ObjectChunk {
    pub sample_rate: u32,
    /// One mono signal per object (bed objects first), in -1.0 ..= 1.0, all of one length.
    pub objects: Vec<Vec<f32>>,
    /// Metadata updates; `offset` counts samples from the start of the stream.
    pub events: Vec<crate::render::Event>,
}

/// Decodes the object presentation of a Dolby Atmos TrueHD stream without rendering it.
#[derive(Default)]
pub struct ObjectDecoder {
    extractor: Extractor,
    parser: Parser,
    decoder: AuDecoder,
    /// Whether the last major sync declared an object presentation.
    objects: Option<bool>,
    /// Samples decoded so far: the clock of the events.
    position: u64,
}

impl ObjectDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Samples decoded so far.
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Decodes the access units in `data` (a raw `.thd` / `.mlp` stream in pieces, or container
    /// packets). A stream without an object presentation, or with intermediate spatial format
    /// objects, is an error; access units before the first major sync give nothing.
    pub fn decode(&mut self, data: &[u8]) -> Result<ObjectChunk, Error> {
        self.extractor.push_bytes(data);
        let mut out = ObjectChunk::default();
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
                let has = matches!(map.presentation_type_by_index(OBJECT_PRESENTATION), PresentationType::Independent);
                if !has {
                    return Err(Error::Decode("not a Dolby Atmos stream (no object presentation)".into()));
                }
                self.objects = Some(true);
            }
            if self.objects != Some(true) {
                continue;
            }
            let d = match self.decoder.decode_presentation(&au, OBJECT_PRESENTATION) {
                Ok(d) => d,
                Err(e) => {
                    self.resync();
                    return Err(Error::Decode(e.to_string()));
                }
            };
            let samples = d.pcm_data.get(..d.sample_length).unwrap_or_default();
            for p in &d.oamd {
                let Some(events) = crate::render::events(p, d.channel_count) else {
                    return Err(Error::Decode("Atmos intermediate spatial format objects are not supported".into()));
                };
                out.events.extend(events.into_iter().map(|mut e| {
                    e.offset = self.position.saturating_add(e.offset);
                    e
                }));
            }
            if out.sample_rate != d.sampling_frequency && !out.objects.is_empty() {
                return Err(Error::Decode("the sample rate changes within the stream".into()));
            }
            out.sample_rate = d.sampling_frequency;
            let have = out.objects.first().map_or(0, Vec::len);
            let count = d.channel_count.min(crate::render::MAX_OBJECTS);
            if out.objects.len() < count {
                out.objects.resize(count, vec![0.0; have]);
            }
            for (c, o) in out.objects.iter_mut().enumerate() {
                if c < count {
                    o.extend(samples.iter().map(|s| s.get(c).copied().unwrap_or(0) as f32 * SCALE));
                } else {
                    o.resize(o.len() + samples.len(), 0.0);
                }
            }
            self.position = self.position.saturating_add(samples.len() as u64);
        }
        Ok(out)
    }

    /// After a bad access unit, decoding resumes at the next major sync.
    fn resync(&mut self) {
        self.extractor = Default::default();
        self.parser.reset_for_next_major_sync();
        self.decoder.reset_for_next_major_sync();
        self.objects = None;
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

    /// The object decoder refuses a stream without an object presentation instead of giving its
    /// channels as objects.
    #[test]
    fn the_object_decoder_refuses_a_stream_without_objects() {
        let mut dec = ObjectDecoder::new();
        let e = dec.decode(crate::process::EXAMPLE_DATA).unwrap_err();
        assert!(e.to_string().contains("not a Dolby Atmos stream"), "{e}");
        assert_eq!(dec.position(), 0);
        // nothing at all before the first major sync
        assert_eq!(ObjectDecoder::new().decode(&[0u8; 64]).unwrap(), ObjectChunk::default());
    }

    /// A stream without an object presentation decodes the same with an object renderer, and a
    /// reset keeps the renderer's layout.
    #[test]
    fn a_stream_without_objects_decodes_its_channels_with_a_renderer() {
        let plain = Decoder::new().decode(crate::process::EXAMPLE_DATA).unwrap();
        let mut dec = Decoder::with_objects(Layout::surround_7_1_4());
        assert_eq!(dec.decode(crate::process::EXAMPLE_DATA).unwrap(), plain);
        dec.reset();
        assert_eq!(dec.renderer.as_ref().map(|r| r.layout().name), Some("7.1.4"));
        assert_eq!(dec.decode(crate::process::EXAMPLE_DATA).unwrap(), plain);
    }
}
