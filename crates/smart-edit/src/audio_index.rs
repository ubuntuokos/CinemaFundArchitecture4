//! Audio access-point index for the bitstream audio a Smart Cut copies without decoding it
//! (`CFA4-SMART-CUT-RENDER-R2`, R2-01): AC-3, E-AC-3 (with Atmos JOC), DTS (core, DTS-HD HRA / MA,
//! DTS:X and other extension substreams) and Dolby TrueHD / MLP (with Atmos).
//!
//! A packet is one container sample (MP4 / Matroska): one or more frames of the codec. For every
//! packet the index says
//!
//! - whether decoding may **start** there ([`AudioUnit::entry`]): every AC-3 / E-AC-3 / DTS frame,
//!   and for TrueHD only an access unit with a major sync (restart point; Atmos also carries its
//!   object metadata there);
//! - how many packets before it a decoder needs for **exact** output from its first sample
//!   ([`AudioUnit::pre_roll`]): the overlapping transforms of AC-3, E-AC-3 and DTS need the
//!   previous frame, as AAC does; TrueHD is lossless from its restart point and needs none;
//! - its length in audio samples and its codec configuration ([`AudioUnit::config`]), so that a
//!   configuration change ends a copy as it does for video.
//!
//! Only the frame headers are read (sync words, sizes, rates, channel layouts); the payload, and
//! with it the Atmos, DTS:X or Auro-3D extensions, is never interpreted and is copied unchanged.
//! Header layouts: ATSC A/52:2018 §5.4 and Annex E (AC-3, E-AC-3), ETSI TS 102 114 §5.3 and §7.4
//! (DTS core and extension substream), and the publicly documented TrueHD access unit and major
//! sync layout (the format has no public specification; see `crates/truehd`).

use serde::{Deserialize, Serialize};

/// The bitstream audio codecs indexed here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AudioCodec {
    Ac3,
    Eac3,
    Dts,
    TrueHd,
}

impl AudioCodec {
    pub fn name(self) -> &'static str {
        match self {
            AudioCodec::Ac3 => "ac3",
            AudioCodec::Eac3 => "eac3",
            AudioCodec::Dts => "dts",
            AudioCodec::TrueHd => "truehd",
        }
    }

    /// Packets before an entry point a decoder needs for exact output from its first sample.
    pub fn pre_roll(self) -> u8 {
        match self {
            AudioCodec::Ac3 | AudioCodec::Eac3 | AudioCodec::Dts => 1,
            AudioCodec::TrueHd => 0,
        }
    }
}

/// The configuration of a packet, as far as the frame headers tell it. Two packets with different
/// configurations are not copied into one track (the sample entry / `CodecPrivate` would lie).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioConfig {
    pub sample_rate: u32,
    /// Codec-specific layout word: AC-3 / E-AC-3 `bsid`, `acmod`, `lfeon` and the substreams of
    /// the packet; DTS `AMODE`, `LFF` and the extension substream presence; TrueHD the format info
    /// and channel words of the major sync.
    pub layout: u64,
}

/// One packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioUnit {
    pub size: u32,
    /// Decoding may start at this packet.
    pub entry: bool,
    /// Audio samples (per channel) the packet decodes to.
    pub samples: u32,
    /// Index into [`AudioIndex::configs`]; for a TrueHD packet without a major sync, the
    /// configuration of the last major sync before it.
    pub config: u32,
}

/// The index of one audio stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioIndex {
    pub codec: AudioCodec,
    pub units: Vec<AudioUnit>,
    pub configs: Vec<AudioConfig>,
}

/// Why a stream cannot be indexed. The Smart Cut then does not copy the track and says why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioIndexError {
    /// The packet is not a whole number of frames of the codec.
    BadFrame {
        packet: u64,
        detail: &'static str,
    },
    /// A TrueHD stream whose first packet has no major sync: nothing before the first restart
    /// point can be decoded, and the configuration is unknown.
    NoMajorSync,
    TooManyPackets,
}

impl std::fmt::Display for AudioIndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioIndexError::BadFrame { packet, detail } => write!(f, "audio packet {packet}: {detail}"),
            AudioIndexError::NoMajorSync => write!(f, "the TrueHD stream does not start with a major sync"),
            AudioIndexError::TooManyPackets => write!(f, "more than {MAX_PACKETS} audio packets"),
        }
    }
}

impl std::error::Error for AudioIndexError {}

/// More packets than any real film has (one TrueHD access unit per packet is 1 200 a second:
/// this is about 230 hours), so that a hostile file cannot make the index grow without bound.
pub const MAX_PACKETS: usize = 1 << 30;

impl AudioIndex {
    /// The last entry point at or before packet `i`.
    pub fn entry_at_or_before(&self, i: usize) -> Option<usize> {
        let end = i.checked_add(1)?.min(self.units.len());
        self.units.get(..end)?.iter().rposition(|u| u.entry)
    }

    /// The first packet to copy so that packet `i` and everything after it decode exactly: the
    /// entry point at or before `i`, then [`AudioCodec::pre_roll`] packets earlier where the
    /// stream has them (fewer at the very start of the stream, where the source has none either).
    pub fn copy_start(&self, i: usize) -> Option<usize> {
        let entry = self.entry_at_or_before(i)?;
        Some(entry.saturating_sub(usize::from(self.codec.pre_roll())))
    }

    /// True when packets `first..=last` all have one configuration.
    pub fn one_config(&self, first: usize, last: usize) -> bool {
        match self.units.get(first..=last) {
            Some(run) => run.iter().all(|u| run.first().is_some_and(|f| f.config == u.config)),
            None => false,
        }
    }
}

/// Index the packets of one stream of `codec`.
pub fn index_audio<'a>(codec: AudioCodec, packets: impl IntoIterator<Item = &'a [u8]>) -> Result<AudioIndex, AudioIndexError> {
    let mut index = AudioIndex { codec, units: Vec::new(), configs: Vec::new() };
    let mut current: Option<u32> = None;
    for (n, data) in packets.into_iter().enumerate() {
        if n >= MAX_PACKETS {
            return Err(AudioIndexError::TooManyPackets);
        }
        let packet = n as u64;
        let bad = |detail| AudioIndexError::BadFrame { packet, detail };
        let size = u32::try_from(data.len()).map_err(|_| bad("larger than 4 GiB"))?;
        let parsed = match codec {
            AudioCodec::Ac3 | AudioCodec::Eac3 => dolby_packet(codec, data).map_err(bad)?,
            AudioCodec::Dts => dts_packet(data).map_err(bad)?,
            AudioCodec::TrueHd => truehd_packet(data).map_err(bad)?,
        };
        let config = match parsed.config {
            Some(c) => {
                let id = match index.configs.iter().position(|x| *x == c) {
                    Some(i) => i,
                    None => {
                        index.configs.push(c);
                        index.configs.len() - 1
                    }
                };
                let id = u32::try_from(id).map_err(|_| AudioIndexError::TooManyPackets)?;
                current = Some(id);
                id
            }
            None => current.ok_or(AudioIndexError::NoMajorSync)?,
        };
        let samples = if codec == AudioCodec::TrueHd {
            // 40 samples per access unit at 48 / 44.1 kHz, scaled with the rate
            let rate = index.configs.get(config as usize).map_or(0, |c| c.sample_rate);
            let scale = if rate.is_multiple_of(48_000) { rate / 48_000 } else { rate / 44_100 };
            parsed.samples.saturating_mul(40 * scale)
        } else {
            parsed.samples
        };
        index.units.push(AudioUnit { size, entry: parsed.entry, samples, config });
    }
    Ok(index)
}

/// Split a raw elementary stream of `codec` (`.ac3`, `.eac3`, `.dts`, `.thd` as FFmpeg writes them)
/// into packets the way MP4 and Matroska store them: one AC-3 frame, one E-AC-3 independent frame
/// with its dependent frames, one DTS core frame with its extension substream (or one extension
/// substream without a core), one TrueHD access unit. Stops at the first byte that is not a frame.
pub fn split_elementary(codec: AudioCodec, mut data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    while !data.is_empty() {
        let len = match codec {
            AudioCodec::Ac3 | AudioCodec::Eac3 => {
                // one frame, then the dependent E-AC-3 frames after it
                let Ok(first) = dolby_frame(data) else { break };
                let mut len = first.size;
                while let Some(rest) = data.get(len..) {
                    match dolby_frame(rest) {
                        Ok(f) if f.dependent => len += f.size,
                        _ => break,
                    }
                }
                len
            }
            AudioCodec::Dts => {
                let len = if be32(data, 0) == Some(DTS_CORE_SYNC) {
                    let Ok(core) = dts_core(data) else { break };
                    core.size
                } else {
                    0
                };
                match data.get(len..).and_then(|r| if be32(r, 0) == Some(DTS_EXSS_SYNC) { dts_exss_size(r).ok() } else { None }) {
                    Some(ext) => len + ext,
                    None if len > 0 => len,
                    None => break,
                }
            }
            AudioCodec::TrueHd => match truehd_unit_size(data) {
                Ok(n) => n,
                Err(_) => break,
            },
        };
        let Some((packet, rest)) = data.split_at_checked(len) else { break };
        if len == 0 {
            break;
        }
        out.push(packet);
        data = rest;
    }
    out
}

struct Parsed {
    entry: bool,
    /// Audio samples; for TrueHD the access units, scaled with the sample rate by [`index_audio`].
    samples: u32,
    config: Option<AudioConfig>,
}

fn be32(d: &[u8], at: usize) -> Option<u32> {
    let b = d.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

// ---------------------------------------------------------------------------------------------
// AC-3 / E-AC-3

/// AC-3 nominal bit rates in kbit/s, by `frmsizecod / 2` (A/52 Table 5.18).
const AC3_KBPS: [u32; 19] = [32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 448, 512, 576, 640];
const AC3_RATES: [u32; 3] = [48_000, 44_100, 32_000];

struct DolbyFrame {
    size: usize,
    rate: u32,
    /// Audio samples of the frame.
    samples: u32,
    bsid: u8,
    acmod: u8,
    lfe: bool,
    /// E-AC-3 dependent substream (`strmtyp` 1).
    dependent: bool,
    substream: u8,
}

fn dolby_frame(d: &[u8]) -> Result<DolbyFrame, &'static str> {
    let h = d.get(..8).ok_or("shorter than a sync frame header")?;
    if h[0] != 0x0B || h[1] != 0x77 {
        return Err("no AC-3 sync word");
    }
    let bsid = h[5] >> 3;
    if bsid <= 10 {
        // AC-3 (A/52 §5.4.1): crc1, fscod, frmsizecod, then bsid / bsmod / acmod
        let fscod = usize::from(h[4] >> 6);
        let frmsizecod = usize::from(h[4] & 0x3f);
        let rate = *AC3_RATES.get(fscod).ok_or("reserved AC-3 sample rate")?;
        let kbps = *AC3_KBPS.get(frmsizecod / 2).ok_or("reserved AC-3 frame size code")?;
        let words = match fscod {
            0 => kbps * 2,
            1 => kbps * 1000 * 1536 / (44_100 * 16) + (frmsizecod as u32 & 1),
            _ => kbps * 3,
        };
        let acmod = h[6] >> 5;
        // lfeon follows cmixlev / surmixlev / dsurmod, present by acmod (A/52 §5.4.2)
        let mut bit = 3u32;
        if acmod & 1 != 0 && acmod != 1 {
            bit += 2;
        }
        if acmod & 4 != 0 {
            bit += 2;
        }
        if acmod == 2 {
            bit += 2;
        }
        let lfe = (u16::from_be_bytes([h[6], h[7]]) >> (15 - bit)) & 1 == 1;
        return Ok(DolbyFrame { size: words as usize * 2, rate, samples: 1536, bsid, acmod, lfe, dependent: false, substream: 0 });
    }
    if !(11..=16).contains(&bsid) {
        return Err("unknown AC-3 bit stream id");
    }
    // E-AC-3 (A/52 Annex E §E.1.2.1): strmtyp, substreamid, frmsiz, fscod, numblkscod / fscod2,
    // acmod, lfeon, bsid
    let strmtyp = h[2] >> 6;
    if strmtyp == 3 {
        return Err("reserved E-AC-3 stream type");
    }
    let substream = (h[2] >> 3) & 7;
    let frmsiz = (u32::from(h[2] & 7) << 8) | u32::from(h[3]);
    let fscod = h[4] >> 6;
    let code2 = (h[4] >> 4) & 3;
    let (rate, blocks) = if fscod == 3 {
        let rate = match code2 {
            0 => 24_000,
            1 => 22_050,
            2 => 16_000,
            _ => return Err("reserved E-AC-3 reduced sample rate"),
        };
        (rate, 6)
    } else {
        (AC3_RATES[usize::from(fscod)], [1u32, 2, 3, 6][usize::from(code2)])
    };
    let acmod = (h[4] >> 1) & 7;
    let lfe = h[4] & 1 == 1;
    Ok(DolbyFrame { size: (frmsiz as usize + 1) * 2, rate, samples: blocks * 256, bsid, acmod, lfe, dependent: strmtyp == 1, substream })
}

fn dolby_packet(codec: AudioCodec, mut d: &[u8]) -> Result<Parsed, &'static str> {
    let mut samples = 0u32;
    let mut layout = 0u64;
    let mut rate = 0u32;
    let mut frames = 0;
    while !d.is_empty() {
        let f = dolby_frame(d)?;
        let is_eac3 = f.bsid > 10;
        if is_eac3 != (codec == AudioCodec::Eac3) {
            return Err(if is_eac3 { "an E-AC-3 frame in an AC-3 track" } else { "an AC-3 frame in an E-AC-3 track" });
        }
        if f.size < 8 || f.size > d.len() {
            return Err("the frame does not fit the packet");
        }
        if frames == 0 {
            if f.dependent {
                return Err("the packet starts with a dependent substream");
            }
            rate = f.rate;
        } else if f.rate != rate {
            return Err("the substreams of one packet have different sample rates");
        }
        // the samples are those of the independent substream 0; others run in parallel
        if !f.dependent && f.substream == 0 {
            samples = samples.saturating_add(f.samples);
        }
        let word = (u64::from(f.bsid) << 8) | (u64::from(f.acmod) << 4) | (u64::from(f.lfe) << 3) | (u64::from(f.dependent) << 2);
        layout = layout.rotate_left(13) ^ (word << 3 | u64::from(f.substream));
        frames += 1;
        if frames > 64 {
            return Err("more than 64 sync frames in one packet");
        }
        d = &d[f.size..];
    }
    if frames == 0 {
        return Err("empty packet");
    }
    Ok(Parsed { entry: true, samples, config: Some(AudioConfig { sample_rate: rate, layout }) })
}

// ---------------------------------------------------------------------------------------------
// DTS

const DTS_CORE_SYNC: u32 = 0x7FFE_8001;
const DTS_EXSS_SYNC: u32 = 0x6458_2025;
/// Core sample rates by `SFREQ` (TS 102 114 Table 5-5); 0 is invalid.
const DTS_RATES: [u32; 16] = [0, 8_000, 16_000, 32_000, 0, 0, 11_025, 22_050, 44_100, 0, 0, 12_000, 24_000, 48_000, 0, 0];

struct DtsCore {
    size: usize,
    samples: u32,
    rate: u32,
    amode: u8,
    lfe: bool,
}

fn dts_core(d: &[u8]) -> Result<DtsCore, &'static str> {
    // after SYNC: FTYPE 1, SHORT 5, CPF 1, NBLKS 7, FSIZE 14, AMODE 6, SFREQ 4, RATE 5, FixedBit 1,
    // DYNF 1, TIMEF 1, AUXF 1, HDCD 1, EXT_AUDIO_ID 3, EXT_AUDIO 1, ASPF 1, LFF 2
    let h = d.get(4..12).ok_or("shorter than a DTS core header")?;
    let bits = u64::from_be_bytes([h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]]);
    let field = |at: u32, len: u32| ((bits >> (64 - at - len)) & ((1 << len) - 1)) as u32;
    let nblks = field(7, 7);
    if nblks < 5 {
        return Err("invalid DTS block count");
    }
    let fsize = field(14, 14);
    if fsize < 95 {
        return Err("invalid DTS frame size");
    }
    let amode = field(28, 6) as u8;
    let rate = DTS_RATES[field(34, 4) as usize];
    if rate == 0 {
        return Err("invalid DTS sample rate");
    }
    let lff = field(53, 2);
    if lff == 3 {
        return Err("invalid DTS LFE flag");
    }
    Ok(DtsCore { size: fsize as usize + 1, samples: (nblks + 1) * 32, rate, amode, lfe: lff != 0 })
}

/// The size of an extension substream (TS 102 114 §7.4.1): UserDefinedBits 8, nExtSSIndex 2,
/// bHeaderSizeType 1, then the header size and the substream size minus one (8 + 16 or
/// 12 + 20 bits).
fn dts_exss_size(d: &[u8]) -> Result<usize, &'static str> {
    let h = be32(d, 4).ok_or("shorter than a DTS extension substream header")?;
    let next = |i: usize| d.get(i).copied().map(u32::from).ok_or("shorter than a DTS extension substream header");
    // bits 31..24 user defined, 23..22 index, 21 header size type, then the header size and the
    // frame size, which runs into the following bytes
    let size = if (h >> 21) & 1 == 1 { ((h & 0x1FF) << 11) | (next(8)? << 3) | (next(9)? >> 5) } else { ((h & 0x1FFF) << 3) | (next(8)? >> 5) };
    let size = size as usize + 1;
    if size < 16 {
        return Err("invalid DTS extension substream size");
    }
    Ok(size)
}

fn dts_packet(d: &[u8]) -> Result<Parsed, &'static str> {
    let (core, rest) = if be32(d, 0) == Some(DTS_CORE_SYNC) {
        let c = dts_core(d)?;
        let rest = d.get(c.size..).ok_or("the DTS core frame does not fit the packet")?;
        (Some(c), rest)
    } else {
        (None, d)
    };
    let mut ext = 0usize;
    let mut r = rest;
    while !r.is_empty() {
        if be32(r, 0) != Some(DTS_EXSS_SYNC) {
            return Err("unexpected bytes after the DTS frame");
        }
        let n = dts_exss_size(r)?;
        r = r.get(n..).ok_or("the DTS extension substream does not fit the packet")?;
        ext += 1;
        if ext > 8 {
            return Err("more than 8 DTS extension substreams in one packet");
        }
    }
    let Some(core) = core else {
        if ext == 0 {
            return Err("no DTS sync word");
        }
        // DTS-HD without a core: the extension substream header names neither the rate nor the
        // length in samples in its first bytes; the container timing is used for those.
        return Ok(Parsed { entry: true, samples: 0, config: Some(AudioConfig { sample_rate: 0, layout: 1 << 32 | ext as u64 }) });
    };
    let layout = (u64::from(core.amode) << 8) | (u64::from(core.lfe) << 4) | ext as u64;
    Ok(Parsed { entry: true, samples: core.samples, config: Some(AudioConfig { sample_rate: core.rate, layout }) })
}

// ---------------------------------------------------------------------------------------------
// TrueHD / MLP

const TRUEHD_SYNC: u32 = 0xF872_6FBA;
const MLP_SYNC: u32 = 0xF872_6FBB;

/// Access unit length in bytes: the low 12 bits of the first 16-bit word count 16-bit words.
fn truehd_unit_size(d: &[u8]) -> Result<usize, &'static str> {
    let h = d.get(..4).ok_or("shorter than an access unit header")?;
    let words = (usize::from(h[0] & 0x0f) << 8) | usize::from(h[1]);
    if words < 2 {
        return Err("invalid access unit length");
    }
    Ok(words * 2)
}

/// Sample rate of a major sync's 4-bit rate code: 48 kHz or 44.1 kHz times 1, 2 or 4.
fn truehd_rate(code: u8) -> Option<u32> {
    match code {
        0 => Some(48_000),
        1 => Some(96_000),
        2 => Some(192_000),
        8 => Some(44_100),
        9 => Some(88_200),
        10 => Some(176_400),
        _ => None,
    }
}

fn truehd_packet(mut d: &[u8]) -> Result<Parsed, &'static str> {
    let mut entry = false;
    let mut config = None;
    let mut units = 0u32;
    while !d.is_empty() {
        let n = truehd_unit_size(d)?;
        let unit = d.get(..n).ok_or("the access unit does not fit the packet")?;
        let sync = be32(unit, 4);
        if sync == Some(TRUEHD_SYNC) || sync == Some(MLP_SYNC) {
            let info = unit.get(8..16).ok_or("short major sync")?;
            // TrueHD: format info starts with the 4-bit rate; MLP: two 4-bit word sizes first
            let code = if sync == Some(TRUEHD_SYNC) { info[0] >> 4 } else { info[1] >> 4 };
            let rate = truehd_rate(code).ok_or("reserved TrueHD sample rate")?;
            let layout = u64::from_be_bytes([info[0], info[1], info[2], info[3], info[4], info[5], info[6], info[7]]);
            if units == 0 {
                entry = true;
                config = Some(AudioConfig { sample_rate: rate, layout });
            }
        }
        units += 1;
        if units > 4096 {
            return Err("more than 4096 access units in one packet");
        }
        d = &d[n..];
    }
    if units == 0 {
        return Err("empty packet");
    }
    Ok(Parsed { entry, samples: units, config })
}
