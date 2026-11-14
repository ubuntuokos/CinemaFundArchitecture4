//! Minimal WAV (RIFF/RF64 PCM 8/16/24/32-bit int, 32/64-bit float) source, with the Broadcast Wave
//! start time: `bext` TimeReference (EBU Tech 3285), else iXML `BWF_TIME_REFERENCE_LOW/HIGH`. The
//! full `riff` crate (all BWF metadata, AIFF) will replace this.
//!
//! Compressed WAV: G.711 μ-law / A-law, G.722, IMA ADPCM and Microsoft ADPCM decode through
//! `cfa4-adpcm`, ATRAC3 (format 0x0270), ATRAC3plus and ATRAC9 (`WAVE_FORMAT_EXTENSIBLE` with
//! Sony's GUIDs; Sony's `.at3` and `.at9`) through `cfa4-realaudio`, still read in place (block by block; G.722
//! and ATRAC from periodic decoder checkpoints). The ATRAC streams of OpenMG files
//! ([`crate::oma`]) are read the same way.

use std::sync::{Arc, Mutex, PoisonError};

use cfa4_adpcm::{Codec, Decoder};

use cfa4_frame::{AudioBuffer, VideoFrame};
use cfa4_time::Tick;

use crate::reader::{MemReader, SharedReader, read_range};
use crate::{AudioStreamInfo, FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, Result};

pub fn sniff(b: &[u8]) -> bool {
    b.len() >= 12 && (&b[0..4] == b"RIFF" || &b[0..4] == b"RF64") && &b[8..12] == b"WAVE"
}

/// Chunks walked looking for `fmt `/`data`/`bext`/`iXML` (real files have a handful).
const MAX_CHUNKS: usize = 1 << 16;
/// Bytes of an iXML chunk searched for the time reference.
const IXML_MAX: u64 = 1 << 20;
/// Output frames converted per read of the data chunk.
const BLOCK_FRAMES: usize = 4096;
/// Largest single read of the data chunk; a wider span (an extreme resampling ratio) reads the
/// two frames each output frame needs instead.
const MAX_SPAN_BYTES: u64 = 4 << 20;
/// Bytes of the `fmt ` chunk read (a `WAVEFORMATEXTENSIBLE` is 40, Microsoft ADPCM's coefficient
/// table makes it 50).
const FMT_MAX: u64 = 1024;
/// G.722 data bytes between two decoder checkpoints (2 s at 64 kbit/s).
const G722_CHECKPOINT: u64 = 16_384;
/// ATRAC packets between two decoder checkpoints (about 1.5 s of ATRAC3, 3 s of ATRAC3plus,
/// 0.3 to 1.4 s of ATRAC9).
const ATRAC_CHECKPOINT: u64 = 64;
/// WAVE_FORMAT_SONY_SCX (ATRAC3).
const TAG_ATRAC3: u16 = 0x0270;
/// The `WAVE_FORMAT_EXTENSIBLE` sub-format of ATRAC3plus (E923AABF-CB58-4471-A119-FFFA01E4CE62).
const ATRAC3P_GUID: [u8; 16] = [0xBF, 0xAA, 0x23, 0xE9, 0x58, 0xCB, 0x71, 0x44, 0xA1, 0x19, 0xFF, 0xFA, 0x01, 0xE4, 0xCE, 0x62];
/// The `WAVE_FORMAT_EXTENSIBLE` sub-format of ATRAC9 (47E142D2-36BA-4D8D-88FC-61654F8C836C).
const ATRAC9_GUID: [u8; 16] = [0xD2, 0x42, 0xE1, 0x47, 0xBA, 0x36, 0x8D, 0x4D, 0x88, 0xFC, 0x61, 0x65, 0x4F, 0x8C, 0x83, 0x6C];

/// An ATRAC decoder state (cloned at checkpoints).
#[derive(Clone)]
pub(crate) enum Atrac {
    Three(Box<cfa4_realaudio::atrac3::Decoder>),
    Plus(Box<cfa4_realaudio::atrac3p::Decoder>),
    Nine(Box<cfa4_realaudio::atrac9::Decoder>),
}

impl Atrac {
    /// One frame as planar samples, `None` for a damaged one.
    fn decode(&mut self, f: &[u8]) -> Option<Vec<Vec<f32>>> {
        match self {
            Atrac::Three(d) => d.decode(f).ok(),
            Atrac::Plus(d) => d.decode(f).ok(),
            Atrac::Nine(d) => d.decode(f).ok(),
        }
    }

    fn samples_per_channel(&self) -> usize {
        match self {
            Atrac::Three(d) => d.samples_per_channel(),
            Atrac::Plus(d) => d.samples_per_channel(),
            Atrac::Nine(d) => d.samples_per_channel(),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Atrac::Three(_) => "ATRAC3",
            Atrac::Plus(_) => "ATRAC3plus",
            Atrac::Nine(_) => "ATRAC9",
        }
    }
}

/// How the data chunk's samples are stored.
enum Encoding {
    /// Integer (`float` false) or IEEE float PCM of `bits` per sample.
    Pcm { bits: u16, float: bool },
    /// A codec decoded in `unit`-byte units of `unit_frames` frames each (G.711 frames, ADPCM
    /// blocks), every unit on its own.
    Units { codec: Codec, unit: u64, unit_frames: u64 },
    /// G.722 (mono, two frames per byte): decoder states at every [`G722_CHECKPOINT`] bytes,
    /// found as reads reach them.
    G722 { checkpoints: Mutex<Vec<cfa4_adpcm::g722::Decoder>> },
    /// ATRAC3, ATRAC3plus or ATRAC9 packets of `block` bytes (`spf` frames each): decoder states at every
    /// [`ATRAC_CHECKPOINT`] frames, found as reads reach them (the overlap, gain control and
    /// filter bank memories carry from frame to frame).
    Atrac { block: u64, spf: u64, checkpoints: Mutex<Vec<Atrac>> },
}

/// A WAV file read in place: opening it reads its chunk headers (not its samples), and
/// [`MediaSource::audio`] reads only the sample range it is asked for, so a 90-minute recording
/// costs its header rather than its size (#279).
pub struct WavSource {
    info: MediaInfo,
    reader: SharedReader,
    data_off: u64,
    /// Bytes of the data chunk.
    data_len: u64,
    /// Whole frames only.
    frames: u64,
    channels: usize,
    encoding: Encoding,
    rate: u32,
    time_reference: Option<u64>,
}

fn le16(b: &[u8], o: usize) -> u16 {
    b.get(o..o.saturating_add(2)).and_then(|s| s.try_into().ok()).map_or(0, u16::from_le_bytes)
}
fn le32(b: &[u8], o: usize) -> u32 {
    b.get(o..o.saturating_add(4)).and_then(|s| s.try_into().ok()).map_or(0, u32::from_le_bytes)
}

impl WavSource {
    /// Parse a WAV file held in memory.
    pub fn parse(name: &str, bytes: Arc<[u8]>) -> Result<Self> {
        Self::open(name, Arc::new(MemReader(bytes)))
    }

    /// Open a WAV file through a random-access reader.
    pub fn open(name: &str, reader: SharedReader) -> Result<Self> {
        let io = |e: std::io::Error| MediaError::Io(format!("{name}: {e}"));
        let len = reader.len();
        let mut pos: u64 = 12;
        let mut fmt = None;
        let mut data = None;
        let mut bext_tr = None;
        let mut ixml_tr = None;
        let mut ds64_data = None;
        let mut atrac3p = false;
        let mut atrac9 = false;
        for _ in 0..MAX_CHUNKS {
            let Some(body) = pos.checked_add(8).filter(|&b| b <= len) else { break };
            let mut hdr = [0u8; 8];
            reader.read_at(pos, &mut hdr).map_err(io)?;
            let id = &hdr[0..4];
            let mut clen = u64::from(le32(&hdr, 4));
            let rest = len - body;
            if id == b"data" {
                // RF64 sentinel: the real size is ds64.dataSize (EBU 3306 §3.4), else the rest of the file
                if clen == 0xFFFF_FFFF {
                    clen = ds64_data.unwrap_or(rest);
                }
                clen = clen.min(rest);
            }
            if clen > rest {
                break;
            }
            match id {
                b"fmt " if clen >= 16 => {
                    let b = read_range(&*reader, body, clen.min(FMT_MAX) as usize).map_err(io)?;
                    let mut tag = le16(&b, 0);
                    let ch = le16(&b, 2);
                    let rate = le32(&b, 4);
                    let align = le16(&b, 12);
                    let bits = le16(&b, 14);
                    if tag == 0xFFFE && clen >= 40 {
                        atrac3p = b.get(24..40) == Some(&ATRAC3P_GUID[..]);
                        atrac9 = b.get(24..40) == Some(&ATRAC9_GUID[..]);
                        tag = le16(&b, 24);
                    }
                    // cbSize bytes after the 18-byte WAVEFORMATEX
                    let extra = b.get(18..).map(|x| x.get(..usize::from(le16(&b, 16))).unwrap_or(x).to_vec()).unwrap_or_default();
                    fmt = Some((tag, ch, rate, bits, align, extra));
                }
                // ds64: riffSize u64, dataSize u64, sampleCount u64, ...
                b"ds64" if clen >= 16 => {
                    let b = read_range(&*reader, body + 8, 8).map_err(io)?;
                    ds64_data = Some(u64::from(le32(&b, 0)) | u64::from(le32(&b, 4)) << 32);
                }
                b"data" => data = Some((body, clen)),
                // bext: Description 256, Originator 32, OriginatorReference 32, OriginationDate 10,
                // OriginationTime 8, then TimeReference (low u32, high u32)
                b"bext" if clen >= 346 => {
                    let b = read_range(&*reader, body + 338, 8).map_err(io)?;
                    bext_tr = Some(u64::from(le32(&b, 0)) | u64::from(le32(&b, 4)) << 32);
                }
                b"iXML" => {
                    let b = read_range(&*reader, body, clen.min(IXML_MAX) as usize).map_err(io)?;
                    ixml_tr = ixml_time_reference(&b);
                }
                _ => {}
            }
            pos = body + clen + (clen & 1);
        }
        let (tag, mut ch, mut rate, bits, align, extra) = fmt.ok_or_else(|| MediaError::Decode(format!("{name}: missing fmt chunk")))?;
        let (off, dlen) = data.ok_or_else(|| MediaError::Decode(format!("{name}: missing data chunk")))?;
        if ch == 0 || rate == 0 || u32::from(ch) > crate::MAX_AUDIO_CHANNELS || rate > crate::MAX_AUDIO_SAMPLE_RATE {
            return Err(MediaError::Unsupported(format!("{name}: WAV format tag {tag}, {ch} channels at {rate} Hz")));
        }
        let float = tag == 3;
        let (encoding, frames, codec_label, bits_per_sample) = if (tag == 1 || float) && matches!(bits, 8 | 16 | 24 | 32 | 64) {
            let frame_bytes = u64::from(ch) * u64::from(bits / 8);
            let label = if float { "PCM float" } else { "PCM" };
            (Encoding::Pcm { bits, float }, dlen / frame_bytes, label.to_string(), Some(u32::from(bits)))
        } else if tag == TAG_ATRAC3 || atrac3p || atrac9 {
            let unsupported = |e: cfa4_realaudio::Error| MediaError::Unsupported(format!("{name}: {e}"));
            let d = if atrac9 {
                // FFmpeg's extradata: what follows the 22 extensible bytes; the configuration
                // names the layout and rate, which win over the header's
                let e = extra.get(22..).unwrap_or(&[]);
                let d = cfa4_realaudio::atrac9::Decoder::new(e, usize::from(align)).map_err(unsupported)?;
                ch = u16::try_from(d.channels()).unwrap_or(ch);
                rate = d.sample_rate();
                Atrac::Nine(Box::new(d))
            } else if atrac3p {
                Atrac::Plus(Box::new(cfa4_realaudio::atrac3p::Decoder::new(ch, usize::from(align)).map_err(unsupported)?))
            } else {
                let p = cfa4_realaudio::atrac3::Params { channels: ch, block_align: usize::from(align), extradata: extra.clone() };
                Atrac::Three(Box::new(cfa4_realaudio::atrac3::Decoder::new(&p).map_err(unsupported)?))
            };
            let (encoding, frames) = atrac_encoding(d, u64::from(align), dlen, |at, len| read_range(&*reader, off.saturating_add(at), len).unwrap_or_default());
            let label = match &encoding {
                Encoding::Atrac { checkpoints, .. } => checkpoints.lock().unwrap_or_else(PoisonError::into_inner).first().map_or("ATRAC", Atrac::label),
                _ => "ATRAC",
            };
            (encoding, frames, label.to_string(), None)
        } else if let Some(codec) = Codec::from_wave_format(tag, align, bits, &extra) {
            let codec = codec.map_err(|e| MediaError::Unsupported(format!("{name}: {e}")))?;
            // a decoder for this layout must exist (G.722 is mono only)
            Decoder::new(codec.clone(), usize::from(ch)).map_err(|e| MediaError::Unsupported(format!("{name}: {e}")))?;
            let c = usize::from(ch);
            let frames = u64::try_from(codec.frames_in(usize::try_from(dlen).unwrap_or(usize::MAX), c)).unwrap_or(u64::MAX);
            let label = codec.name().to_string();
            let encoding = if codec.is_stateful() {
                Encoding::G722 { checkpoints: Mutex::new(vec![cfa4_adpcm::g722::Decoder::new()]) }
            } else {
                let unit = codec.unit_bytes(c);
                let unit_frames = codec.frames_in(unit, c);
                Encoding::Units { codec, unit: unit as u64, unit_frames: unit_frames.max(1) as u64 }
            };
            (encoding, frames, label, None)
        } else {
            return Err(MediaError::Unsupported(format!("{name}: WAV format tag {tag}, {bits} bits")));
        };
        let time_reference = bext_tr.or(ixml_tr);
        let mut info = MediaInfo {
            name: name.into(),
            kind: MediaKind::AudioOnly,
            duration: Tick::from_units(i64::try_from(frames).unwrap_or(i64::MAX), rate as i64),
            video: None,
            audio_streams: vec![AudioStreamInfo { sample_rate: rate, channels: ch as u32, codec: codec_label, bits_per_sample, speakers: Vec::new() }],
            container: if time_reference.is_some() { "Broadcast WAV".into() } else { "WAV".into() },
            start_timecode: None,
            file_size: Some(len),
        };
        // start timecode: frames at the media's frame rate (the default rate for audio-only media)
        if let Some(tr) = time_reference.filter(|_| rate > 0) {
            let r = info.frame_rate();
            let n = tr as i128 * r.num as i128;
            let d = rate as i128 * r.den as i128;
            info.start_timecode = (n + d / 2).checked_div(d).and_then(|t| i64::try_from(t).ok());
        }
        Ok(Self { info, reader, data_off: off, data_len: dlen, frames, channels: ch as usize, encoding, rate, time_reference })
    }

    /// Broadcast Wave start time: samples since midnight.
    pub fn time_reference(&self) -> Option<u64> {
        self.time_reference
    }

    fn io(&self, e: std::io::Error) -> MediaError {
        MediaError::Io(format!("{}: {e}", self.info.name))
    }

    /// Bytes `from..to` of the data chunk (clipped to it).
    fn read_data(&self, from: u64, to: u64) -> Result<Vec<u8>> {
        let to = to.min(self.data_len);
        let n = usize::try_from(to.saturating_sub(from)).map_err(|_| MediaError::Decode(format!("{}: audio read too large", self.info.name)))?;
        let mut buf = vec![0u8; n];
        self.reader.read_at(self.data_off.saturating_add(from), &mut buf).map_err(|e| self.io(e))?;
        Ok(buf)
    }

    /// Decoded bytes per frame of a read, for the span limit.
    fn span_frame_bytes(&self) -> u64 {
        let bytes = match self.encoding {
            Encoding::Pcm { bits, .. } => u64::from(bits / 8),
            _ => 4,
        };
        bytes.saturating_mul(self.channels as u64)
    }

    /// Whole frames `first..=last`, interleaved, as f32.
    fn read_frames(&self, first: u64, last: u64) -> Result<Vec<f32>> {
        let ch = self.channels;
        let count = last.saturating_sub(first).saturating_add(1);
        match &self.encoding {
            Encoding::Pcm { bits, float } => {
                let fb = u64::from(bits / 8).saturating_mul(ch as u64);
                let raw = self.read_data(first.saturating_mul(fb), first.saturating_add(count).saturating_mul(fb))?;
                let bps = usize::from(bits / 8).max(1);
                Ok(raw.chunks_exact(bps).map(|b| pcm_sample(*bits, *float, b)).collect())
            }
            Encoding::Units { codec, unit, unit_frames } => {
                let (u0, u1) = (first / unit_frames, last / unit_frames);
                let raw = self.read_data(u0.saturating_mul(*unit), u1.saturating_add(1).saturating_mul(*unit))?;
                let mut dec = Decoder::new(codec.clone(), ch).map_err(|e| MediaError::Decode(format!("{}: {e}", self.info.name)))?;
                let planes = dec.decode(&raw).map_err(|e| MediaError::Decode(format!("{}: {e}", self.info.name)))?;
                Ok(interleave_range(&planes, first - u0 * unit_frames, count))
            }
            Encoding::G722 { checkpoints } => {
                // two frames per byte: start at the checkpoint before the first frame's byte
                let byte = first / 2;
                let k = byte / G722_CHECKPOINT;
                let mut state = self.g722_checkpoint(checkpoints, k)?;
                let from = k * G722_CHECKPOINT;
                let raw = self.read_data(from, last / 2 + 1)?;
                let mut out = Vec::with_capacity(raw.len() * 2);
                state.decode_into(&raw, &mut out);
                let skip = usize::try_from(first - from * 2).unwrap_or(usize::MAX);
                let n = usize::try_from(count).unwrap_or(usize::MAX);
                Ok(out.iter().skip(skip).take(n).map(|&s| f32::from(s) / 32768.0).collect())
            }
            Encoding::Atrac { block, spf, checkpoints } => {
                let spf = (*spf).max(1);
                let (f0, f1) = (first / spf, last / spf);
                let k = f0 / ATRAC_CHECKPOINT;
                let mut d = self.atrac_checkpoint(checkpoints, *block, k)?;
                let from = k * ATRAC_CHECKPOINT;
                let raw = self.read_data(from.saturating_mul(*block), f1.saturating_add(1).saturating_mul(*block))?;
                let ch = self.channels;
                let sp = usize::try_from(spf).unwrap_or(usize::MAX);
                let skip = usize::try_from(first - from * spf).unwrap_or(usize::MAX).saturating_mul(ch);
                let n = usize::try_from(count).unwrap_or(usize::MAX).saturating_mul(ch);
                let mut out = Vec::with_capacity(n.saturating_add(skip).min(1 << 26));
                for f in raw.chunks(usize::try_from(*block).unwrap_or(usize::MAX).max(1)) {
                    // a damaged frame is silence; the decoder goes on as ffmpeg's does
                    match d.decode(f) {
                        Some(p) => {
                            for i in 0..sp {
                                out.extend(p.iter().map(|c| c.get(i).copied().unwrap_or(0.0)));
                            }
                        }
                        None => out.resize(out.len().saturating_add(sp.saturating_mul(ch)), 0.0),
                    }
                }
                Ok(out.into_iter().skip(skip).take(n).collect())
            }
        }
    }

    /// The ATRAC decoder state at checkpoint `k` (frame `k · ATRAC_CHECKPOINT`), decoding
    /// forward from the last known one as far as needed.
    fn atrac_checkpoint(&self, checkpoints: &Mutex<Vec<Atrac>>, block: u64, k: u64) -> Result<Atrac> {
        let k = usize::try_from(k).map_err(|_| MediaError::Decode(format!("{}: ATRAC position", self.info.name)))?;
        let mut cps = checkpoints.lock().unwrap_or_else(PoisonError::into_inner);
        while cps.len() <= k {
            let i = cps.len() - 1;
            let Some(mut d) = cps.last().cloned() else { break };
            let from = i as u64 * ATRAC_CHECKPOINT * block;
            let raw = self.read_data(from, from + ATRAC_CHECKPOINT * block)?;
            for f in raw.chunks(usize::try_from(block).unwrap_or(usize::MAX).max(1)) {
                let _ = d.decode(f);
            }
            cps.push(d);
        }
        cps.get(k).or(cps.last()).cloned().ok_or_else(|| MediaError::Decode(format!("{}: ATRAC state", self.info.name)))
    }

    /// The G.722 decoder state at checkpoint `k` (byte `k · G722_CHECKPOINT`), decoding forward
    /// from the last known one as far as needed.
    fn g722_checkpoint(&self, checkpoints: &Mutex<Vec<cfa4_adpcm::g722::Decoder>>, k: u64) -> Result<cfa4_adpcm::g722::Decoder> {
        let k = usize::try_from(k).map_err(|_| MediaError::Decode(format!("{}: G.722 position", self.info.name)))?;
        let mut cps = checkpoints.lock().unwrap_or_else(PoisonError::into_inner);
        while cps.len() <= k {
            let i = cps.len() - 1;
            let Some(mut d) = cps.last().cloned() else { break };
            let from = i as u64 * G722_CHECKPOINT;
            let raw = self.read_data(from, from + G722_CHECKPOINT)?;
            let mut sink = Vec::with_capacity(raw.len() * 2);
            d.decode_into(&raw, &mut sink);
            cps.push(d);
        }
        cps.get(k).or(cps.last()).cloned().ok_or_else(|| MediaError::Decode(format!("{}: G.722 state", self.info.name)))
    }
}

/// The encoding and frame count of an ATRAC stream of `dlen` bytes in `block`-byte frames
/// (ATRAC3plus decodes a short last frame and ATRAC9 the whole frames of a short last packet, as
/// FFmpeg's do); `tail` reads the short last packet.
fn atrac_encoding(d: Atrac, block: u64, dlen: u64, tail: impl FnOnce(u64, usize) -> Vec<u8>) -> (Encoding, u64) {
    let spf = d.samples_per_channel() as u64;
    let frames = match &d {
        Atrac::Plus(_) => dlen.div_ceil(block.max(1)).saturating_mul(spf),
        Atrac::Three(_) => (dlen / block.max(1)).saturating_mul(spf),
        // a short last packet: what the decoder makes of it (from the stream's start state)
        Atrac::Nine(n) => {
            let short = dlen % block.max(1);
            let last = if short > 0 { n.samples_in_packet(&tail(dlen - short, usize::try_from(short).unwrap_or(0))) as u64 } else { 0 };
            (dlen / block.max(1)).saturating_mul(spf).saturating_add(last)
        }
    };
    (Encoding::Atrac { block, spf, checkpoints: Mutex::new(vec![d]) }, frames)
}

impl WavSource {
    /// An ATRAC stream of `data_len` bytes at `data_off` in another container (OpenMG).
    pub(crate) fn atrac_stream(name: &str, reader: SharedReader, data: (u64, u64), rate: u32, d: Atrac, container: &str) -> Self {
        let (data_off, data_len) = data;
        let channels = match &d {
            Atrac::Three(d) => d.channels(),
            Atrac::Plus(d) => d.channels(),
            Atrac::Nine(d) => d.channels(),
        };
        let block = match &d {
            Atrac::Three(d) => d.block_align(),
            Atrac::Plus(d) => d.block_align(),
            Atrac::Nine(d) => d.block_align(),
        } as u64;
        let codec = d.label().to_string();
        let (encoding, frames) = atrac_encoding(d, block, data_len, |at, len| read_range(&*reader, data_off.saturating_add(at), len).unwrap_or_default());
        let info = MediaInfo {
            name: name.into(),
            kind: MediaKind::AudioOnly,
            duration: Tick::from_units(i64::try_from(frames).unwrap_or(i64::MAX), i64::from(rate.max(1))),
            video: None,
            audio_streams: vec![AudioStreamInfo { sample_rate: rate, channels: channels as u32, codec, bits_per_sample: None, speakers: Vec::new() }],
            container: container.into(),
            start_timecode: None,
            file_size: Some(reader.len()),
        };
        Self { info, reader, data_off, data_len, frames, channels, encoding, rate, time_reference: None }
    }
}

/// One PCM sample of `bits` (little-endian, 8-bit unsigned) as f32.
fn pcm_sample(bits: u16, float: bool, b: &[u8]) -> f32 {
    match (bits, float, b) {
        (8, _, &[a]) => (a as f32 - 128.0) / 128.0,
        (16, _, &[a, b]) => i16::from_le_bytes([a, b]) as f32 / 32768.0,
        (24, _, &[a, b, c]) => ((i32::from_le_bytes([0, a, b, c]) >> 8) as f32) / 8_388_608.0,
        (32, false, &[a, b, c, d]) => i32::from_le_bytes([a, b, c, d]) as f32 / 2_147_483_648.0,
        (32, true, &[a, b, c, d]) => f32::from_le_bytes([a, b, c, d]),
        (64, true, b) => f64::from_le_bytes(b.try_into().unwrap_or([0; 8])) as f32,
        _ => 0.0,
    }
}

/// Frames `skip..skip + count` of planar 16-bit `planes`, interleaved as f32 (fewer at the end).
fn interleave_range(planes: &[Vec<i16>], skip: u64, count: u64) -> Vec<f32> {
    let len = planes.iter().map(Vec::len).min().unwrap_or(0);
    let from = usize::try_from(skip).unwrap_or(usize::MAX).min(len);
    let to = usize::try_from(skip.saturating_add(count)).unwrap_or(usize::MAX).min(len);
    let mut out = Vec::with_capacity((to - from) * planes.len());
    for i in from..to {
        out.extend(planes.iter().map(|p| p.get(i).map_or(0.0, |&s| f32::from(s) / 32768.0)));
    }
    out
}

impl MediaSource for WavSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }
    fn video_frame(&self, _req: FrameRequest) -> Result<Arc<VideoFrame>> {
        Err(MediaError::NoStream("video"))
    }
    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> Result<AudioBuffer> {
        let total = self.frames;
        crate::validate_audio_buffer_shape(self.channels, frames)?;
        let mut out = AudioBuffer::silence(sample_rate, self.channels, frames);
        if total == 0 || sample_rate == 0 {
            return Ok(out);
        }
        // Linear-interpolating resample when rates differ (the audio crate provides sinc resampling).
        let ratio = self.rate as f64 / sample_rate as f64;
        let src_of = |i: usize| start.saturating_add(i as i64) as f64 * ratio;
        let mut i = 0;
        while i < frames {
            let end = frames.min(i + BLOCK_FRAMES);
            let (lo, hi) = (src_of(i), src_of(end - 1));
            if hi < 0.0 {
                i = end;
                continue;
            }
            let first = lo.max(0.0).floor() as u64;
            if first >= total {
                break;
            }
            let last = (hi.floor() as u64).saturating_add(1).min(total - 1);
            let span = last.saturating_sub(first).saturating_add(1).saturating_mul(self.span_frame_bytes());
            let block = if span <= MAX_SPAN_BYTES { Some(self.read_frames(first, last)?) } else { None };
            for slot in i..end {
                let src = src_of(slot);
                if src < 0.0 {
                    continue;
                }
                let i0 = src.floor() as u64;
                if i0 >= total {
                    break;
                }
                let i1 = (i0 + 1).min(total - 1);
                let fr = (src - i0 as f64) as f32;
                let (buf, base) = match &block {
                    Some(b) => (std::borrow::Cow::Borrowed(b.as_slice()), first),
                    None => (std::borrow::Cow::Owned(self.read_frames(i0, i1)?), i0),
                };
                let (k0, k1) = ((i0 - base) as usize, (i1 - base) as usize);
                let at = |k: usize, c: usize| buf.get(k * self.channels + c).copied().unwrap_or(0.0);
                for c in 0..self.channels {
                    let a = at(k0, c);
                    let b = at(k1, c);
                    if let Some(s) = out.channels.get_mut(c).and_then(|ch| ch.get_mut(slot)) {
                        *s = a + (b - a) * fr;
                    }
                }
            }
            i = end;
        }
        Ok(out)
    }
}

/// TimeReference from an iXML chunk (`<BWF_TIME_REFERENCE_LOW>` / `<BWF_TIME_REFERENCE_HIGH>`).
fn ixml_time_reference(x: &[u8]) -> Option<u64> {
    let text = String::from_utf8_lossy(x);
    let field = |tag: &str| -> Option<u64> {
        let open = format!("<{tag}>");
        let a = text.find(&open)? + open.len();
        let b = a + text[a..].find('<')?;
        text[a..b].trim().parse().ok()
    };
    let low = field("BWF_TIME_REFERENCE_LOW")?;
    Some(low | field("BWF_TIME_REFERENCE_HIGH").unwrap_or(0) << 32)
}

/// Encode interleaved f32 samples as a 16-bit PCM WAV file.
pub fn write_wav16(samples: &[f32], channels: u16, rate: u32) -> Vec<u8> {
    let data_len = samples.len() * 2;
    let mut v = Vec::with_capacity(44 + data_len);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&channels.to_le_bytes());
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
    v.extend_from_slice(&(channels * 2).to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&(data_len as u32).to_le_bytes());
    for s in samples {
        v.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let s: Vec<f32> = (0..200).map(|i| ((i as f32) * 0.1).sin() * 0.5).collect();
        let bytes = write_wav16(&s, 2, 48000);
        let src = WavSource::parse("t.wav", bytes.into()).unwrap();
        assert_eq!(src.info().audio().unwrap().channels, 2);
        let a = src.audio(0, 100, 48000).unwrap();
        assert!((a.channels[0][3] - s[6]).abs() < 1e-4);
        assert!((a.channels[1][3] - s[7]).abs() < 1e-4);
        assert_eq!(src.info().duration, Tick::from_units(100, 48000));
        assert_eq!(src.time_reference(), None);
        assert_eq!(src.info().start_timecode, None);
    }

    /// Insert a chunk before `data`.
    fn with_chunk(wav: &[u8], id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = wav[..36].to_vec();
        v.extend_from_slice(id);
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(body);
        if body.len() % 2 == 1 {
            v.push(0);
        }
        v.extend_from_slice(&wav[36..]);
        let riff = (v.len() - 8) as u32;
        v[4..8].copy_from_slice(&riff.to_le_bytes());
        v
    }

    #[test]
    fn bwf_time_reference_from_bext() {
        let wav = write_wav16(&[0.0; 96], 2, 48_000);
        // 01:00:00:00 at 23.976 fps: 86 400 frames of 2002 samples
        let tr: u64 = 86_400 * 2002;
        let mut bext = vec![0u8; 602];
        bext[338..342].copy_from_slice(&(tr as u32).to_le_bytes());
        bext[342..346].copy_from_slice(&((tr >> 32) as u32).to_le_bytes());
        let src = WavSource::parse("bwf.wav", with_chunk(&wav, b"bext", &bext).into()).unwrap();
        assert_eq!(src.time_reference(), Some(tr));
        assert_eq!(src.info().frame_rate(), cfa4_time::FrameRate::FPS_23_976);
        assert_eq!(src.info().start_timecode, Some(86_400));
        assert_eq!(src.info().container, "Broadcast WAV");
        // the audio is unaffected
        assert_eq!(src.info().duration, Tick::from_units(48, 48_000));
    }

    #[test]
    fn bwf_time_reference_from_ixml_and_large_values() {
        let wav = write_wav16(&[0.0; 4], 1, 48_000);
        let tr: u64 = (1 << 32) + 5;
        let xml = format!(
            "<?xml version=\"1.0\"?><BWFXML><BEXT><BWF_TIME_REFERENCE_LOW>{}</BWF_TIME_REFERENCE_LOW><BWF_TIME_REFERENCE_HIGH>{}</BWF_TIME_REFERENCE_HIGH></BEXT></BWFXML>",
            tr & 0xFFFF_FFFF,
            tr >> 32
        );
        let src = WavSource::parse("ixml.wav", with_chunk(&wav, b"iXML", xml.as_bytes()).into()).unwrap();
        assert_eq!(src.time_reference(), Some(tr));
        // a short bext (no TimeReference) is ignored
        let src = WavSource::parse("short.wav", with_chunk(&wav, b"bext", &[0u8; 100]).into()).unwrap();
        assert_eq!(src.time_reference(), None);
    }

    #[test]
    fn hostile_header_dimensions_are_rejected_before_publishing_or_reading() {
        let mut wav = write_wav16(&[0.0; 2], 1, 48_000);
        // `fmt ` starts at byte 12: channels at +22, rate at +24.
        wav[22..24].copy_from_slice(&u16::MAX.to_le_bytes());
        assert!(WavSource::parse("wide.wav", wav.into()).is_err());
        let mut wav = write_wav16(&[0.0; 2], 1, 48_000);
        wav[24..28].copy_from_slice(&(crate::MAX_AUDIO_SAMPLE_RATE + 1).to_le_bytes());
        assert!(WavSource::parse("fast.wav", wav.into()).is_err());
    }

    /// A WAV of `data` with format `tag` (mono, `align`-byte blocks, 4 bits).
    fn compressed_wav(tag: u16, rate: u32, align: u16, data: &[u8]) -> Vec<u8> {
        let mut w = write_wav16(&[], 1, rate);
        w[20..22].copy_from_slice(&tag.to_le_bytes());
        w[32..34].copy_from_slice(&align.to_le_bytes());
        w[34..36].copy_from_slice(&4u16.to_le_bytes());
        w.truncate(40);
        w.extend_from_slice(&(data.len() as u32).to_le_bytes());
        w.extend_from_slice(data);
        let riff = (w.len() - 8) as u32;
        w[4..8].copy_from_slice(&riff.to_le_bytes());
        w
    }

    fn noise(n: usize) -> Vec<u8> {
        let mut x = 0x2545_F491u32;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x >> 8) as u8
            })
            .collect()
    }

    /// G.722 read from the middle (from a decoder checkpoint) gives the samples a read from the
    /// start does, across checkpoint boundaries and at the end of the data.
    #[test]
    fn g722_random_access_matches_sequential() {
        let data = noise(3 * G722_CHECKPOINT as usize + 1000);
        let src = WavSource::parse("g.wav", compressed_wav(0x028F, 16_000, 1, &data).into()).unwrap();
        let total = data.len() * 2;
        assert_eq!(src.info().audio().unwrap().codec, "G.722");
        let whole = src.audio(0, total, 16_000).unwrap().channels.remove(0);
        let fresh = || WavSource::parse("g.wav", compressed_wav(0x028F, 16_000, 1, &data).into()).unwrap();
        for start in [total - 10, 2 * G722_CHECKPOINT as usize * 2 - 3, 12_345, 1] {
            let part = fresh().audio(start as i64, 600, 16_000).unwrap().channels.remove(0);
            let n = 600.min(total - start);
            assert_eq!(&part[..n], &whole[start..start + n], "from {start}");
            assert!(part[n..].iter().all(|&s| s == 0.0), "silence past the end");
        }
    }

    /// Damaged ADPCM headers are refused at open; random blocks decode without failing.
    #[test]
    fn hostile_adpcm_headers_and_blocks() {
        for (tag, align) in [(0x11u16, 0u16), (0x02, 0)] {
            assert!(WavSource::parse("bad.wav", compressed_wav(tag, 8000, align, &[0; 64]).into()).is_err(), "tag {tag:#x} align {align}");
        }
        for (tag, align) in [(0x11u16, 36u16), (0x02, 70), (0x07, 1), (0x06, 1), (0x11, 3), (0x02, 6)] {
            let src = WavSource::parse("n.wav", compressed_wav(tag, 8000, align, &noise(1001)).into()).unwrap();
            let a = src.audio(-5, 3000, 11_025).unwrap();
            assert!(a.channels[0].iter().all(|s| s.is_finite() && s.abs() <= 1.0), "tag {tag:#x}");
        }
    }
}
