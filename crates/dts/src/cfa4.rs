// SPDX-License-Identifier: GPL-3.0-or-later
//! CFA4: the decoder interface the rest of CFA4 uses (not part of the ported crate).
//!
//! [`Decoder::decode_packet`] takes whole frames (a Matroska / MP4 / MPEG-TS packet: a DTS core
//! frame, optionally followed by its extension substream) and [`Decoder::push`] a raw `.dts`
//! stream in pieces. Both give planar `f32` channels in the WAV / SMPTE order used across CFA4
//! (L R C LFE back-L back-R … side-L side-R …).
//!
//! What is decoded: the DTS core (Coherent Acoustics, up to 5.1); DTS-HD Master Audio losslessly
//! (up to 7.1, 96 / 192 kHz); DTS-HD High Resolution as its core plus the XXCH channels. A DTS:X
//! stream gives its backward-compatible bed, in which the encoder has folded the heights and
//! objects; the extension waveforms are not placed on their own loudspeakers. An extension
//! substream nothing here reads (XBR-only High Resolution) plays its core.

use crate::dcadec::core::DCA_SPEAKER_COUNT;
use crate::hd::{ExssKind, HdDecoder, HdError, HdFrame, exss_declared_size, exss_kind, exss_substream_size};
use crate::parser::{ParseError, SYNCWORD_CORE_BE, SYNCWORD_SUBSTREAM, parse_header};
use crate::pcm::{CorePcmFrame, PcmDecoder};
use crate::types::BedChannel;

const CORE_SYNC: [u8; 4] = SYNCWORD_CORE_BE.to_be_bytes();
const EXSS_SYNC: [u8; 4] = SYNCWORD_SUBSTREAM.to_be_bytes();

/// Most bytes a raw stream may buffer without a frame: a core frame is at most 16 KiB and its
/// extension substream at most 64 KiB.
const MAX_BUFFER: usize = 1 << 20;

/// Decoder errors. Malformed input gives an error, never a panic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("DTS: {0}")]
    Decode(String),
}

/// A loudspeaker of a decoded layout.
#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Speaker {
    L,
    R,
    C,
    LFE,
    /// Back (rear) surround left / right.
    Lb,
    Rb,
    /// Front left / right of centre.
    Lc,
    Rc,
    /// Back centre.
    Cs,
    /// Surround left / right (side surrounds of 7.1).
    Ls,
    Rs,
    /// Top centre (overhead).
    Tc,
    Tfl,
    Tfc,
    Tfr,
    Tbl,
    Tbc,
    Tbr,
    Lw,
    Rw,
    /// Side surrounds coded beside a surround pair (the pair then plays as [`Speaker::Ls`] /
    /// [`Speaker::Rs`]).
    Lss,
    Rss,
    Tsl,
    Tsr,
    LFE2,
    /// Bottom (below ear height) front centre / left / right.
    Bfc,
    Bfl,
    Bfr,
}

/// The speaker of a DCA speaker index (ETSI TS 102 114 Table 7-9; FFmpeg's `DCA_SPEAKER_*`).
fn dca_speaker(i: usize) -> Option<Speaker> {
    use Speaker::*;
    Some(match i {
        0 => C,
        1 => L,
        2 => R,
        3 => Ls,
        4 => Rs,
        5 => LFE,
        6 => Cs,
        7 => Lb,
        8 => Rb,
        9 => Lss,
        10 => Rss,
        11 => Lc,
        12 => Rc,
        13 => Tfl,
        14 => Tfc,
        15 => Tfr,
        16 => LFE2,
        17 => Lw,
        18 => Rw,
        19 => Tc,
        20 => Tsl,
        21 => Tsr,
        22 => Tbc,
        23 => Tbl,
        24 => Tbr,
        25 => Bfc,
        26 => Bfl,
        27 => Bfr,
        _ => return None,
    })
}

fn bed_speaker(b: BedChannel) -> Speaker {
    match b {
        BedChannel::FrontLeft => Speaker::L,
        BedChannel::FrontRight => Speaker::R,
        BedChannel::Center => Speaker::C,
        BedChannel::LowFrequencyEffects => Speaker::LFE,
        BedChannel::SurroundLeft => Speaker::Ls,
        BedChannel::SurroundRight => Speaker::Rs,
        BedChannel::RearCenter => Speaker::Cs,
        BedChannel::RearLeft => Speaker::Lb,
        BedChannel::RearRight => Speaker::Rb,
        BedChannel::WideLeft => Speaker::Lw,
        BedChannel::WideRight => Speaker::Rw,
    }
}

/// The rank of a speaker in CFA4's output order (the WAV channel-mask order, then the speakers
/// WAV has no bit for).
fn rank(s: Speaker) -> u8 {
    s as u8
}

/// Decoded audio of one or more frames.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Decoded {
    pub sample_rate: u32,
    /// Planar samples in -1.0 ..= 1.0, in the order of [`Decoded::speakers`].
    pub channels: Vec<Vec<f32>>,
    pub speakers: Vec<Speaker>,
    /// Whether the samples are a DTS-HD Master Audio lossless reconstruction.
    pub lossless: bool,
}

impl Decoded {
    /// Appends one frame; a frame of another layout or rate replaces what came before (the
    /// latest layout is what a caller plays on).
    fn append(&mut self, rate: u32, lossless: bool, mut chans: Vec<(Speaker, Vec<f32>)>) {
        chans.sort_by_key(|(s, _)| rank(*s));
        let speakers: Vec<Speaker> = chans.iter().map(|(s, _)| *s).collect();
        if self.channels.is_empty() || self.speakers != speakers || self.sample_rate != rate {
            *self = Decoded { sample_rate: rate, channels: vec![Vec::new(); speakers.len()], speakers, lossless };
        }
        self.lossless &= lossless;
        for (o, (_, c)) in self.channels.iter_mut().zip(chans) {
            o.extend(c);
        }
    }

    /// Appends the silence of `n` samples at `rate` (a frame that failed to decode, so that timing
    /// holds) in the current layout, at its rate: a 48 kHz core under a 96 kHz lossless stream
    /// pads twice its own length.
    fn silence(&mut self, n: usize, rate: u32) {
        let n = if rate == 0 || self.sample_rate == 0 || rate == self.sample_rate {
            n
        } else {
            usize::try_from((n as u64).saturating_mul(u64::from(self.sample_rate)) / u64::from(rate)).unwrap_or(usize::MAX)
        };
        for c in &mut self.channels {
            c.resize(c.len().saturating_add(n), 0.0);
        }
    }
}

/// A DTS decoder: the core, DTS-HD Master Audio and High Resolution.
#[derive(Default)]
pub struct Decoder {
    core: PcmDecoder,
    hd: HdDecoder,
    core_frame: CorePcmFrame,
    hd_frame: HdFrame,
    /// Raw-stream bytes not yet decoded.
    buf: Vec<u8>,
}

/// What [`Decoder::frame`] found at the start of its input.
enum Step {
    /// A frame of `len` bytes was decoded into the output (or replaced by silence).
    Frame(usize),
    /// More bytes are needed.
    Short,
    /// No frame starts here: skip this many bytes.
    Skip(usize),
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drops all stream state (after a seek).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Decodes the whole frames of a container packet. Bytes before the first core syncword and
    /// a truncated last frame are ignored. A frame that does not decode gives silence of its
    /// length once a layout is known; an error only when nothing in the packet decodes.
    pub fn decode_packet(&mut self, data: &[u8]) -> Result<Decoded, Error> {
        let mut out = Decoded::default();
        let mut p = 0;
        let mut err = None;
        while let Some(rest) = data.get(p..).filter(|r| !r.is_empty()) {
            match self.frame(rest, true, &mut out, &mut err) {
                Step::Frame(n) | Step::Skip(n) => p += n.max(1),
                Step::Short => break,
            }
        }
        match err {
            Some(e) if out.channels.is_empty() => Err(e),
            _ => Ok(out),
        }
    }

    /// Decodes a raw stream given in pieces: complete frames come out, the rest waits for the
    /// next call. [`Decoder::flush`] decodes what is left at the end of the stream.
    pub fn push(&mut self, data: &[u8]) -> Result<Decoded, Error> {
        self.buf.extend_from_slice(data);
        self.drain(false)
    }

    /// Decodes the frame left at the end of a raw stream.
    pub fn flush(&mut self) -> Result<Decoded, Error> {
        self.drain(true)
    }

    fn drain(&mut self, at_end: bool) -> Result<Decoded, Error> {
        let mut out = Decoded::default();
        let mut err = None;
        let mut p = 0;
        let buf = std::mem::take(&mut self.buf);
        while let Some(rest) = buf.get(p..).filter(|r| !r.is_empty()) {
            match self.frame(rest, at_end, &mut out, &mut err) {
                Step::Frame(n) | Step::Skip(n) => p += n.max(1),
                Step::Short => break,
            }
        }
        self.buf = buf;
        self.buf.drain(..p.min(self.buf.len()));
        if at_end || self.buf.len() > MAX_BUFFER {
            self.buf.clear();
        }
        match err {
            Some(e) if out.channels.is_empty() => Err(e),
            _ => Ok(out),
        }
    }

    /// Decodes the frame at the start of `data` into `out`. `complete`: `data` holds everything
    /// there is (a packet, or the end of a stream).
    fn frame(&mut self, data: &[u8], complete: bool, out: &mut Decoded, err: &mut Option<Error>) -> Step {
        let Some(sync) = data.windows(4).position(|w| w == CORE_SYNC) else {
            // keep a possible partial syncword: skipping it too would lose the frame it starts
            return match data.len().saturating_sub(3) {
                _ if complete => Step::Skip(data.len()),
                0 => Step::Short,
                n => Step::Skip(n),
            };
        };
        if sync > 0 {
            return Step::Skip(sync);
        }
        let info = match parse_header(data) {
            Ok(i) => i,
            Err(ParseError::InsufficientData) => return if complete { Step::Skip(data.len()) } else { Step::Short },
            Err(e) => {
                *err = Some(Error::Decode(format!("{e:?}")));
                return Step::Skip(4);
            }
        };
        let fs = info.frame_size;
        let Some(core) = data.get(..fs) else {
            return if complete { Step::Skip(data.len()) } else { Step::Short };
        };
        let after = data.get(fs..).unwrap_or_default();
        if after.len() < 4 && !complete {
            return Step::Short;
        }
        let exss = if after.get(..4) == Some(&EXSS_SYNC[..]) {
            // the declared size comes from the fixed header, so a substream still arriving is
            // waited for rather than mistaken for an unreadable one
            match exss_declared_size(after) {
                Some(es) if es <= after.len() => after.get(..es).filter(|_| exss_substream_size(after).is_some()),
                Some(_) if !complete => return Step::Short,
                None if !complete => return Step::Short,
                // a substream that is cut off or unreadable: the core plays alone
                _ => None,
            }
        } else {
            None
        };
        let len = fs.saturating_add(exss.map_or(0, <[u8]>::len));
        let samples = info.samples_per_channel();
        let ok = match exss.filter(|e| exss_kind(e) != ExssKind::Core) {
            Some(exss) => self.hd_frame(core, exss, out),
            None => self.core_frame(core, out),
        };
        match ok {
            Ok(()) => {}
            Err(None) => {}
            Err(Some(e)) => {
                out.silence(samples, info.sample_rate);
                *err = Some(e);
            }
        }
        Step::Frame(len)
    }

    /// Decodes a core-only frame. `Err(None)` is not used here.
    fn core_frame(&mut self, core: &[u8], out: &mut Decoded) -> Result<(), Option<Error>> {
        let pcm = &mut self.core_frame;
        self.core.decode_into(core, pcm).map_err(|e| Some(Error::Decode(e.to_string())))?;
        let mut chans: Vec<(Speaker, Vec<f32>)> =
            pcm.fullband_channel_order.iter().zip(&pcm.fullband_channels).map(|(&b, c)| (bed_speaker(b), c.clone())).collect();
        if let Some(lfe) = &pcm.lfe_channel {
            chans.push((Speaker::LFE, lfe.clone()));
        }
        out.append(pcm.sample_rate, false, chans);
        Ok(())
    }

    /// Decodes a core frame with its extension substream. `Err(None)`: the decoder is buffering
    /// (peak bit rate smoothing) and gives no audio for this frame.
    fn hd_frame(&mut self, core: &[u8], exss: &[u8], out: &mut Decoded) -> Result<(), Option<Error>> {
        let frame = &mut self.hd_frame;
        match self.hd.decode_into(core, exss, frame) {
            Ok(()) => {}
            Err(HdError::Pending) => return Err(None),
            Err(e) => return Err(Some(Error::Decode(format!("{e:?}")))),
        }
        let chans: Vec<(Speaker, Vec<f32>)> =
            frame.samples.iter().take(DCA_SPEAKER_COUNT).enumerate().filter_map(|(i, c)| Some((dca_speaker(i)?, c.clone()?))).collect();
        if chans.is_empty() {
            return Err(Some(Error::Decode("no channels".into())));
        }
        out.append(frame.sample_rate, frame.lossless, chans);
        Ok(())
    }
}

/// Samples a packet of whole DTS frames decodes to: the sum of its core frames' lengths. `None`
/// when the packet does not start with a core frame.
pub fn packet_samples(data: &[u8]) -> Option<i64> {
    parse_header(data).ok()?;
    let mut n = 0i64;
    let mut p = 0usize;
    while let Some(rest) = data.get(p..).filter(|r| r.len() >= 4) {
        if rest.get(..4) == Some(&EXSS_SYNC[..]) {
            p = p.saturating_add(exss_substream_size(rest)?.max(4));
            continue;
        }
        let i = parse_header(rest).ok()?;
        n = n.saturating_add(i.samples_per_channel() as i64);
        p = p.saturating_add(i.frame_size.max(4));
    }
    Some(n)
}

/// The sample rate and speakers of the first decodable frame of a raw stream, for probing.
pub fn stream_layout(data: &[u8]) -> Result<(u32, Vec<Speaker>), Error> {
    let mut dec = Decoder::new();
    let mut d = dec.push(data)?;
    if d.channels.is_empty() {
        d = dec.flush()?;
    }
    if d.channels.is_empty() {
        return Err(Error::Decode("no decodable frame".into()));
    }
    Ok((d.sample_rate, d.speakers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speakers_rank_in_wav_order() {
        use Speaker::*;
        let mut v = vec![Rs, Ls, Rb, Lb, LFE, C, R, L];
        v.sort_by_key(|s| rank(*s));
        assert_eq!(v, [L, R, C, LFE, Lb, Rb, Ls, Rs]);
        assert_eq!(dca_speaker(5), Some(LFE));
        assert_eq!(dca_speaker(28), None);
    }

    /// A damaged 48 kHz core frame under 96 kHz lossless output padded only half its length.
    #[test]
    fn padding_is_at_the_output_rate() {
        let mut d = Decoded { sample_rate: 96_000, channels: vec![Vec::new(); 2], ..Decoded::default() };
        d.silence(512, 48_000);
        assert!(d.channels.iter().all(|c| c.len() == 1024));
        d.silence(512, 96_000);
        d.silence(512, 0);
        assert!(d.channels.iter().all(|c| c.len() == 2048));
    }

    #[test]
    fn garbage_gives_nothing_and_no_panic() {
        let mut d = Decoder::new();
        assert_eq!(d.decode_packet(&[]).unwrap(), Decoded::default());
        assert_eq!(d.decode_packet(&[0x7F, 0xFE, 0x80]).unwrap(), Decoded::default());
        let mut z = vec![0u8; 64];
        z[..4].copy_from_slice(&CORE_SYNC);
        assert!(d.decode_packet(&z).map_or(true, |o| o.channels.is_empty()));
        assert!(d.push(&[0x7F, 0xFE, 0x80, 0x01, 0xFF, 0xFF]).unwrap().channels.is_empty());
        assert!(d.flush().is_ok());
        assert_eq!(packet_samples(&[1, 2, 3]), None);
    }
}
