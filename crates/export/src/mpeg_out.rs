//! MPEG-2 output of the stepped exporter: the MPEG-2 video pictures in a program stream with
//! 16- or 24-bit DVD LPCM, a transport stream (188-byte, or 192-byte BDAV `.m2ts`) with AAC in ADTS,
//! either of them with MPEG-1 Layer II (MP2) audio instead, or a bare elementary stream (`.m2v`,
//! no audio), written with [`cfa4_mpegts::Muxer`].

use std::io::Write;

use cfa4_mpegts::{MuxCodec, MuxFormat, Muxer};
use cfa4_time::FrameRate;

use crate::aac_out::AdtsHeader;
use crate::{EncodedPacket, ExportError, ExportSettings, Out, Result};

/// The container of an MPEG-2 export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mpeg2Container {
    /// MPEG-2 program stream (`.mpg`) with DVD LPCM audio.
    Ps,
    /// MPEG-2 transport stream (`.ts`) with AAC audio.
    Ts,
    /// BDAV transport stream (`.m2ts`): 192-byte packets with arrival timestamps, AAC audio.
    M2ts,
    /// The video elementary stream alone (`.m2v`).
    Es,
}

impl Mpeg2Container {
    pub const ALL: [Mpeg2Container; 4] = [Mpeg2Container::Ps, Mpeg2Container::Ts, Mpeg2Container::M2ts, Mpeg2Container::Es];

    /// From an id (`ps`, `ts`, `m2ts`, `es`) or a file extension.
    pub fn from_name(s: &str) -> Option<Mpeg2Container> {
        Some(match s.trim().trim_start_matches('.').to_ascii_lowercase().as_str() {
            "ps" | "mpg" | "mpeg" | "m2p" | "vob" | "program" => Mpeg2Container::Ps,
            "ts" | "transport" => Mpeg2Container::Ts,
            "m2ts" | "mts" | "bdav" => Mpeg2Container::M2ts,
            "es" | "m2v" | "mpv" | "elementary" => Mpeg2Container::Es,
            _ => return None,
        })
    }

    pub fn id(self) -> &'static str {
        match self {
            Mpeg2Container::Ps => "ps",
            Mpeg2Container::Ts => "ts",
            Mpeg2Container::M2ts => "m2ts",
            Mpeg2Container::Es => "es",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mpeg2Container::Ps => "MPEG-2 Program Stream (.mpg)",
            Mpeg2Container::Ts => "MPEG-2 Transport Stream (.ts)",
            Mpeg2Container::M2ts => "BDAV Transport Stream (.m2ts)",
            Mpeg2Container::Es => "Elementary Stream (.m2v, no audio)",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Mpeg2Container::Ps => "mpg",
            Mpeg2Container::Ts => "ts",
            Mpeg2Container::M2ts => "m2ts",
            Mpeg2Container::Es => "m2v",
        }
    }

    /// Whether the container carries audio (as AAC: the transport streams).
    pub fn has_audio(self) -> bool {
        self != Mpeg2Container::Es
    }

    pub fn is_transport(self) -> bool {
        matches!(self, Mpeg2Container::Ts | Mpeg2Container::M2ts)
    }

    /// The container of `settings`: [`ExportSettings::mpeg2_container`], or else the path's
    /// extension, or else a program stream.
    pub fn of(settings: &ExportSettings) -> Mpeg2Container {
        Mpeg2Container::from_name(&settings.mpeg2_container)
            .or_else(|| std::path::Path::new(&settings.path).extension().and_then(|e| e.to_str()).and_then(Mpeg2Container::from_name))
            .unwrap_or(Mpeg2Container::Ps)
    }
}

/// The MPEG-2 frame rates (frame_rate_code 1-8).
pub(crate) const MPEG2_RATES: [(i64, i64); 8] = [(24000, 1001), (24, 1), (25, 1), (30000, 1001), (30, 1), (50, 1), (60000, 1001), (60, 1)];

/// Whether MPEG-2 video can signal `rate`.
pub(crate) fn mpeg2_rate(rate: FrameRate) -> bool {
    MPEG2_RATES.iter().any(|&(n, d)| i128::from(rate.num) * i128::from(d) == i128::from(n) * i128::from(rate.den))
}

/// Peak bit rate of the multiplex for `video_kbps` (peak) and `audio_bps`: twice the video peak
/// leaves room for I pictures, plus 10 % for the packet headers.
pub(crate) fn mux_rate(video_kbps: u32, audio_bps: u64) -> u64 {
    (u64::from(video_kbps) * 2000 + audio_bps) * 11 / 10
}

enum Sink {
    Es(Out),
    Mux(Box<Muxer<Out>>),
}

/// The audio of an MPEG-2 export.
pub(crate) enum MpegAudio {
    /// DVD LPCM (program streams) of 16 or 24 bits.
    Lpcm { sample_rate: u32, channels: u32, bits: u32 },
    /// AAC access units with their ADTS header (transport streams); `priming` samples of encoder
    /// delay come before the first output sample.
    Aac { header: AdtsHeader, sample_rate: u32, priming: u32 },
    /// AC-3 syncframes of our encoder (program and transport streams).
    Ac3(Box<cfa4_ac3::Encoder>),
    /// MPEG-1 Layer II frames from our encoder (program and transport streams).
    Mp2 { enc: Box<cfa4_mpa::encode::Encoder>, sample_rate: u32 },
}

impl MpegAudio {
    /// An MP2 encoder for `channels` (1 or 2) at `sample_rate` and the nearest allowed bit rate.
    pub fn mp2(sample_rate: u32, channels: usize, kbps: u32) -> Result<MpegAudio> {
        let rate = crate::settings::mp2_bitrate_kbps(sample_rate, channels, kbps);
        let cfg = cfa4_mpa::encode::Config::new(sample_rate, channels, rate);
        let enc = cfa4_mpa::encode::Encoder::new(cfg).map_err(|e| ExportError::Unsupported(format!("MP2 audio: {e}")))?;
        Ok(MpegAudio::Mp2 { enc: Box::new(enc), sample_rate })
    }
}

/// The AC-3 bit rates (ATSC A/52 Table 5.18), kb/s.
pub const AC3_BITRATES: [u32; 19] = [32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 448, 512, 576, 640];

/// AC-3 output lags the input by this many samples (the transform overlap).
const AC3_DELAY: i64 = 256;

/// An MPEG-2 export in progress.
pub(crate) struct MpegMux {
    sink: Sink,
    rate: FrameRate,
    audio: Option<MpegAudio>,
    /// Pictures and audio samples written so far.
    pictures: i64,
    samples: i64,
    /// Decode time of the first picture in frames: before 0 when B pictures delay the display.
    dts_base: i64,
    /// 20- and 24-bit LPCM goes in groups of two sample frames: an odd frame waits here
    /// (interleaved samples).
    pending: Vec<i32>,
}

fn mux_err(e: cfa4_mpegts::MuxError) -> ExportError {
    match e {
        cfa4_mpegts::MuxError::Io(e) => ExportError::Io(e.to_string()),
        cfa4_mpegts::MuxError::Unsupported(m) => ExportError::Unsupported(m),
    }
}

/// `n` ticks of a `num / den` clock in 90 kHz units, rounded down (also before 0, so that units
/// keep their spacing across it).
fn to_90k(n: i64, num: i64, den: i64) -> i64 {
    let t = (i128::from(n) * 90_000 * i128::from(den)).div_euclid(i128::from(num.max(1)));
    i64::try_from(t).unwrap_or(i64::MAX)
}

impl MpegMux {
    pub fn new(settings: &ExportSettings, rate: FrameRate, audio: Option<MpegAudio>, mux_rate: u64) -> Result<Self> {
        let out = Out::create(settings)?;
        let container = Mpeg2Container::of(settings);
        let sink = match container {
            Mpeg2Container::Es => Sink::Es(out),
            c => {
                let mut streams = vec![MuxCodec::Mpeg2Video];
                match &audio {
                    Some(MpegAudio::Lpcm { sample_rate, channels, bits }) => {
                        streams.push(MuxCodec::LpcmDvd { sample_rate: *sample_rate, channels: *channels, bits: *bits })
                    }
                    Some(MpegAudio::Aac { .. }) => streams.push(MuxCodec::AacAdts),
                    Some(MpegAudio::Ac3(_)) => streams.push(MuxCodec::Ac3),
                    Some(MpegAudio::Mp2 { .. }) => streams.push(MuxCodec::MpegAudio),
                    None => {}
                }
                let format = match c {
                    Mpeg2Container::Ts => MuxFormat::Ts,
                    Mpeg2Container::M2ts => MuxFormat::M2ts,
                    _ => MuxFormat::Ps,
                };
                Sink::Mux(Box::new(Muxer::new(out, format, &streams, mux_rate).map_err(mux_err)?))
            }
        };
        let audio = if container.has_audio() { audio } else { None };
        Ok(MpegMux { sink, rate, audio, pictures: 0, samples: 0, dts_base: 0, pending: Vec::new() })
    }

    pub fn write_video(&mut self, packets: Vec<EncodedPacket>) -> Result<()> {
        for p in packets {
            match &mut self.sink {
                Sink::Es(out) => out.write_all(&p.data).map_err(|e| ExportError::Io(e.to_string()))?,
                Sink::Mux(m) => {
                    // decode order; the composition offset (in 1/num s) is how much later the
                    // picture is shown, and the first one is shown at 0
                    let offset = i64::from(p.composition_offset) / self.rate.den.max(1);
                    if self.pictures == 0 {
                        self.dts_base = -offset;
                    }
                    let dts = self.pictures + self.dts_base;
                    let (num, den) = (self.rate.num, self.rate.den);
                    m.write(0, &p.data, to_90k(dts + offset, num, den), Some(to_90k(dts, num, den)), p.key).map_err(mux_err)?;
                }
            }
            self.pictures += 1;
        }
        Ok(())
    }

    /// Planar samples of an LPCM, AC-3 or MP2 export.
    pub fn write_pcm(&mut self, planar: &[Vec<f32>]) -> Result<()> {
        if let (Some(MpegAudio::Ac3(enc)), Sink::Mux(_)) = (&mut self.audio, &self.sink) {
            let refs: Vec<&[f32]> = planar.iter().map(Vec::as_slice).collect();
            let frames = enc.encode(&refs).map_err(|e| ExportError::Encode(format!("AC-3: {e}")))?;
            return self.write_ac3(frames);
        }
        if let (Some(MpegAudio::Mp2 { enc, .. }), Sink::Mux(_)) = (&mut self.audio, &self.sink) {
            let ch = enc.config().channels;
            let n = planar.first().map_or(0, Vec::len);
            let silent = vec![0f32; n];
            let slices: Vec<&[f32]> = (0..ch).map(|c| planar.get(c).filter(|p| p.len() == n).map_or(&silent[..], |p| &p[..])).collect();
            let frames = enc.encode(&slices).map_err(|e| ExportError::Encode(format!("MP2: {e}")))?;
            return self.write_mp2(frames);
        }
        let (Some(MpegAudio::Lpcm { sample_rate, channels, bits }), Sink::Mux(m)) = (&self.audio, &mut self.sink) else { return Ok(()) };
        let n = planar.first().map_or(0, Vec::len);
        if n == 0 {
            return Ok(());
        }
        let (ch, bits) = (*channels as usize, *bits);
        // the samples quantised like a WAV export's, interleaved, after a frame held back last time
        let format = if bits == 24 { crate::SampleFormat::Int24 } else { crate::SampleFormat::Int16 };
        let held = self.pending.len() / ch.max(1);
        let mut frames = std::mem::take(&mut self.pending);
        frames.reserve(n * ch);
        for i in 0..n {
            for c in 0..ch {
                let v = planar.get(c).and_then(|p| p.get(i)).copied().unwrap_or(0.0);
                frames.push(format.quantise(v));
            }
        }
        if bits != 16 && (frames.len() / ch.max(1)) % 2 == 1 {
            self.pending = frames.split_off(frames.len() - ch);
        }
        let data = pack_dvd_lpcm(&frames, ch, bits);
        // the packet starts with the held frame, which is one sample period earlier
        let pts = to_90k(self.samples - held as i64, i64::from(*sample_rate), 1);
        if !data.is_empty() {
            m.write(1, &data, pts, None, true).map_err(mux_err)?;
        }
        self.samples += n as i64;
        Ok(())
    }

    /// AC-3 syncframes of 1536 samples, each shown [`AC3_DELAY`] samples before its input.
    fn write_ac3(&mut self, frames: Vec<Vec<u8>>) -> Result<()> {
        let (Some(MpegAudio::Ac3(enc)), Sink::Mux(m)) = (&self.audio, &mut self.sink) else { return Ok(()) };
        let rate = i64::from(enc.sample_rate());
        for f in frames {
            m.write(1, &f, to_90k(self.samples - AC3_DELAY, rate, 1), None, true).map_err(mux_err)?;
            self.samples += 1536;
        }
        Ok(())
    }

    /// The AC-3 encoder's last syncframes (the input it still holds, padded with silence).
    pub fn flush_ac3(&mut self) -> Result<()> {
        let Some(MpegAudio::Ac3(enc)) = &mut self.audio else { return Ok(()) };
        let frames = enc.flush().map_err(|e| ExportError::Encode(format!("AC-3: {e}")))?;
        self.write_ac3(frames)
    }

    /// AAC access units of `frame` samples each.
    pub fn write_aac(&mut self, units: Vec<Vec<u8>>, frame: u32) -> Result<()> {
        let (Some(MpegAudio::Aac { header, sample_rate, priming }), Sink::Mux(m)) = (&self.audio, &mut self.sink) else { return Ok(()) };
        for au in units {
            let mut data = header.bytes(au.len())?.to_vec();
            data.extend_from_slice(&au);
            let pts = to_90k(self.samples - i64::from(*priming), i64::from(*sample_rate), 1);
            m.write(1, &data, pts, None, true).map_err(mux_err)?;
            self.samples += i64::from(frame);
        }
        Ok(())
    }

    /// MP2 frames of 1152 samples; the first sample of the stream is the encoder's delay before
    /// the first input sample.
    fn write_mp2(&mut self, frames: Vec<Vec<u8>>) -> Result<()> {
        let (Some(MpegAudio::Mp2 { sample_rate, .. }), Sink::Mux(m)) = (&self.audio, &mut self.sink) else { return Ok(()) };
        for f in frames {
            let pts = to_90k(self.samples - cfa4_mpa::encode::DELAY as i64, i64::from(*sample_rate), 1);
            m.write(1, &f, pts, None, true).map_err(mux_err)?;
            self.samples += cfa4_mpa::encode::FRAME_SAMPLES as i64;
        }
        Ok(())
    }

    /// End the video with a sequence end code and finish the file; returns its size.
    pub fn finish(mut self, settings: &ExportSettings) -> Result<u64> {
        if let Some(MpegAudio::Mp2 { enc, .. }) = &mut self.audio {
            let rest = enc.flush();
            self.write_mp2(rest)?;
        }
        const SEQUENCE_END: [u8; 4] = [0, 0, 1, 0xB7];
        let io = |e: std::io::Error| ExportError::Io(e.to_string());
        let out = match self.sink {
            Sink::Es(mut out) => {
                out.write_all(&SEQUENCE_END).map_err(io)?;
                out
            }
            Sink::Mux(mut m) => {
                // a 20- / 24-bit group still missing its second frame: completed with silence
                if let Some(MpegAudio::Lpcm { sample_rate, channels, bits }) = &self.audio
                    && !self.pending.is_empty()
                {
                    let ch = *channels as usize;
                    let mut frames = self.pending.clone();
                    frames.resize(2 * ch, 0);
                    let pts = to_90k(self.samples - 1, i64::from(*sample_rate), 1);
                    m.write(1, &pack_dvd_lpcm(&frames, ch, *bits), pts, None, true).map_err(mux_err)?;
                }
                // the last picture is still queued; were it not, the stream merely lacks its end code
                if self.pictures > 0 {
                    m.append_to_last(0, &SEQUENCE_END).ok();
                }
                m.finish().map_err(mux_err)?
            }
        };
        out.finish(settings)
    }
}

/// Interleaved samples (`ch` channels, whole frames; 20 and 24 bits: an even number of them) as
/// big-endian DVD LPCM: 16-bit samples one after the other; 20- and 24-bit ones in groups of two
/// frames, the upper 16 bits of the group's samples first, then their low 4 bits (two to a byte)
/// or low 8 bits.
pub(crate) fn pack_dvd_lpcm(frames: &[i32], ch: usize, bits: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(frames.len() * 3);
    if bits == 16 || ch == 0 {
        for &v in frames {
            out.extend_from_slice(&(v as i16).to_be_bytes());
        }
        return out;
    }
    let low = bits - 16;
    for group in frames.chunks_exact(2 * ch) {
        for &v in group {
            out.extend_from_slice(&((v >> low) as i16).to_be_bytes());
        }
        if low == 8 {
            out.extend(group.iter().map(|&v| v as u8));
        } else {
            for pair in group.chunks(2) {
                let nib = |k: usize| pair.get(k).map_or(0, |&v| (v & 0xF) as u8);
                out.push(nib(0) << 4 | nib(1));
            }
        }
    }
    out
}
