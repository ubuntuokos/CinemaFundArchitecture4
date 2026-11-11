// SPDX-License-Identifier: GPL-3.0-or-later
//! CFA4: the decoder interface the rest of CFA4 uses (not part of the ported crate).
//!
//! [`Decoder::decode_packet`] takes whole frames (a Matroska / MP4 / MPEG-TS packet: a DTS core
//! frame, optionally followed by its extension substream, or the extension substream alone of a
//! DTS-HD Master Audio stream without a core) and [`Decoder::push`] a raw `.dts`
//! stream in pieces. Both give planar `f32` channels in the WAV / SMPTE order used across CFA4
//! (L R C LFE back-L back-R … side-L side-R …).
//!
//! What is decoded: the DTS core (Coherent Acoustics, up to 5.1); DTS-HD Master Audio losslessly
//! (up to 7.1, 96 / 192 kHz); DTS-HD High Resolution as its core plus the XXCH channels. A DTS:X
//! stream gives its backward-compatible bed, in which the encoder has folded the heights and
//! objects; the extension waveforms are not placed on their own loudspeakers. An extension
//! substream nothing here reads (XBR-only High Resolution) plays its core.

use crate::dcadec::core::DCA_SPEAKER_COUNT;
use crate::hd::{ExssKind, HdDecoder, HdError, HdFrame, exss_declared_size, exss_kind, exss_substream_size, exss_xll_geometry};
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
    /// Rate and loudspeakers of the last frame [`Decoder::push_frames`] gave: a frame that fails
    /// after it becomes silence of its length in this layout.
    layout: Option<(u32, Vec<Speaker>)>,
}

/// One frame of a raw stream from [`Decoder::push_frames`].
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// The frame's audio: silence of its length when it failed to decode after a layout was
    /// known, empty when it failed before.
    pub audio: Decoded,
    /// Why the frame did not decode.
    pub error: Option<Error>,
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

    /// Like [`Decoder::push`], one [`Frame`] per frame, so that a caller sees each frame's
    /// layout and each error: [`Decoder::push`] gives the frames of a piece in the layout of
    /// its last frame and only the error of a piece in which nothing decodes.
    pub fn push_frames(&mut self, data: &[u8]) -> Vec<Frame> {
        self.buf.extend_from_slice(data);
        self.drain_frames(false)
    }

    /// Like [`Decoder::flush`], one [`Frame`] per frame.
    pub fn flush_frames(&mut self) -> Vec<Frame> {
        self.drain_frames(true)
    }

    fn drain_frames(&mut self, at_end: bool) -> Vec<Frame> {
        let mut frames = Vec::new();
        let mut p = 0;
        let buf = std::mem::take(&mut self.buf);
        while let Some(rest) = buf.get(p..).filter(|r| !r.is_empty()) {
            let mut out = match &self.layout {
                Some((rate, speakers)) => {
                    Decoded { sample_rate: *rate, channels: vec![Vec::new(); speakers.len()], speakers: speakers.clone(), lossless: true }
                }
                None => Decoded::default(),
            };
            let mut err = None;
            let step = self.frame(rest, at_end, &mut out, &mut err);
            let has_audio = out.channels.first().is_some_and(|c| !c.is_empty());
            if has_audio && err.is_none() {
                self.layout = Some((out.sample_rate, out.speakers.clone()));
            }
            if has_audio || err.is_some() {
                if !has_audio {
                    out = Decoded::default();
                }
                frames.push(Frame { audio: out, error: err });
            }
            match step {
                Step::Frame(n) | Step::Skip(n) => p += n.max(1),
                Step::Short => break,
            }
        }
        self.buf = buf;
        self.buf.drain(..p.min(self.buf.len()));
        if at_end || self.buf.len() > MAX_BUFFER {
            self.buf.clear();
        }
        frames
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
        let Some(sync) = data.windows(4).position(|w| w == CORE_SYNC || w == EXSS_SYNC) else {
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
        if data.get(..4) == Some(&EXSS_SYNC[..]) {
            return self.exss_frame(data, complete, out, err);
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

    /// Decodes the extension substream at the start of `data` that no core frame precedes: a
    /// frame of DTS-HD Master Audio without a core. One that needs a core (a stream entered
    /// between a core frame and its substream) is skipped.
    fn exss_frame(&mut self, data: &[u8], complete: bool, out: &mut Decoded, err: &mut Option<Error>) -> Step {
        let short = if complete { Step::Skip(data.len()) } else { Step::Short };
        let Some(size) = exss_declared_size(data) else { return short };
        let Some(exss) = data.get(..size) else { return if complete { Step::Skip(4) } else { Step::Short } };
        if exss_substream_size(exss).is_none() {
            return Step::Skip(4);
        }
        match self.hd.decode_exss_into(exss, &mut self.hd_frame) {
            Ok(()) => {
                let frame = &self.hd_frame;
                let chans: Vec<(Speaker, Vec<f32>)> =
                    frame.samples.iter().take(DCA_SPEAKER_COUNT).enumerate().filter_map(|(i, c)| Some((dca_speaker(i)?, c.clone()?))).collect();
                if chans.is_empty() {
                    *err = Some(Error::Decode("no channels".into()));
                } else {
                    out.append(frame.sample_rate, frame.lossless, chans);
                }
            }
            Err(HdError::Pending) => {}
            Err(HdError::NeedsCore) => return Step::Skip(size),
            Err(e) => {
                if let Some((n, rate)) = exss_xll_geometry(exss) {
                    out.silence(n, rate);
                }
                *err = Some(Error::Decode(format!("{e:?}")));
            }
        }
        Step::Frame(size)
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

/// The audio stream of a raw DTS file: a DTS-HD file (`.dtshd`, chunks of an 8-byte id and a
/// 64-bit big-endian size starting with `DTSHDHDR`) gives its `STRMDATA` chunk only, since other
/// chunks (a `BLACKOUT` chunk holds a silent frame) are not the stream; an elementary stream
/// (`.dts`) comes back whole. A chunk whose size runs past the end gives what is there.
pub fn stream_data(data: &[u8]) -> &[u8] {
    if !data.starts_with(b"DTSHDHDR") {
        return data;
    }
    let mut p = 0usize;
    while let Some(head) = data.get(p..p.saturating_add(16)) {
        let (Some(id), Some(size)) = (head.get(..8), head.get(8..16).and_then(|b| <[u8; 8]>::try_from(b).ok())) else { break };
        let size = usize::try_from(u64::from_be_bytes(size)).unwrap_or(usize::MAX);
        let body_start = p.saturating_add(16);
        let body = data.get(body_start..).map_or(&[][..], |b| b.get(..size).unwrap_or(b));
        if id == b"STRMDATA" {
            return body;
        }
        p = body_start.saturating_add(size);
    }
    &[]
}

/// Samples a packet of whole DTS frames decodes to: the sum of its core frames' lengths, or for a
/// stream without a core its lossless frames' lengths. `None` when the packet starts with
/// neither.
pub fn packet_samples(data: &[u8]) -> Option<i64> {
    if data.get(..4) == Some(&EXSS_SYNC[..]) {
        let mut n = 0i64;
        let mut p = 0usize;
        while let Some(rest) = data.get(p..).filter(|r| r.len() >= 4) {
            let size = exss_substream_size(rest)?;
            n = n.saturating_add(exss_xll_geometry(rest.get(..size)?)?.0 as i64);
            p = p.saturating_add(size.max(4));
        }
        return Some(n);
    }
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
    fn stream_data_takes_strmdata_and_survives_damage() {
        let chunk = |id: &[u8; 8], size: u64, body: &[u8]| [&id[..], &size.to_be_bytes(), body].concat();
        let f = [chunk(b"DTSHDHDR", 2, &[1, 2]), chunk(b"BLACKOUT", 1, &[9]), chunk(b"STRMDATA", 3, &[7, 7, 7])].concat();
        assert_eq!(stream_data(&f), &[7, 7, 7]);
        assert_eq!(stream_data(b"\x7F\xFE\x80\x01"), b"\x7F\xFE\x80\x01", "an elementary stream comes back whole");
        // a size past the end, a huge size and a cut-off chunk header: no panic
        let f = [chunk(b"DTSHDHDR", 0, &[]), chunk(b"STRMDATA", 99, &[5, 6])].concat();
        assert_eq!(stream_data(&f), &[5, 6]);
        let f = [chunk(b"DTSHDHDR", u64::MAX, &[]), chunk(b"STRMDATA", 1, &[5])].concat();
        assert!(stream_data(&f).is_empty());
        assert!(stream_data(b"DTSHDHDR\0\0").is_empty());
    }

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
        // an extension substream syncword without a readable substream
        let mut x = vec![0u8; 64];
        x[..4].copy_from_slice(&EXSS_SYNC);
        assert!(d.decode_packet(&x).map_or(true, |o| o.channels.is_empty()));
        assert!(d.push(&x).unwrap().channels.is_empty());
        assert_eq!(packet_samples(&x), None);
    }
}
