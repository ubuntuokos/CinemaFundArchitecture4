//! Immersive audio in DCP exports (#343): an IAB track file (SMPTE ST 429-18) next to the
//! picture and the main sound, as ST 429-19 packages it.
//!
//! - `bed`: the export's 5.1 mix becomes a 7.1 DS bed (L R C LFE, Ls and Rs each at -3 dB on the
//!   side and the rear surrounds), the one bed layout besides 9.1OH that ISDCF Doc 15 (IAB
//!   Application Profile 1) allows. No objects.
//! - `file`: the frames of an existing IAB (an IMF or DCP IAB track file, or a raw
//!   IABitstream) are re-wrapped, made fit for a DCP where needed (ST 429-18 7.2: DLC, 24 bit,
//!   no UserData; Profile 1: no AuthoringToolInfo) and otherwise copied byte for byte. A shorter
//!   source is padded with silent frames, a longer one ends with the picture.
//!
//! Every frame is DLC coded (lossless), 48 kHz, 24 bit, at the composition's edit rate.
//!
//! The main sound track becomes 16 channels, Configuration 4 (ST 429-19 7.1.3), with the FSK
//! synchronisation signal of ST 430-12 in channel 14 (7.1.1) and, unless switched off, a 5.1
//! fallback in channels 1-6: the 5.1 mix (`bed`) or the IAB rendered to 5.1 (`file`).

use std::io::{Read, Seek, SeekFrom};
use std::sync::Mutex;

use cfa4_iab::{AudioData, Bed, BedChannel, Coding, Description, Frame, FrameRate, Gain, Layout};
use cfa4_mxf::{ByteSource, Codec, IabDesc, MxfFile, MxfWriter};

use crate::{ExportError, ExportSettings, Out, Result};

/// Immersive audio of a DCP export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DcpImmersive {
    Off,
    /// The 5.1 mix as a 7.1 DS IAB bed.
    Bed,
    /// An existing IAB, re-wrapped.
    File,
}

impl DcpImmersive {
    pub const ALL: [DcpImmersive; 3] = [DcpImmersive::Off, DcpImmersive::Bed, DcpImmersive::File];

    /// `off` (also empty, `none`), `bed` or `file` (`source`).
    pub fn from_name(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "off" | "none" => Some(DcpImmersive::Off),
            "bed" | "iab" => Some(DcpImmersive::Bed),
            "file" | "source" => Some(DcpImmersive::File),
            _ => None,
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            DcpImmersive::Off => "off",
            DcpImmersive::Bed => "bed",
            DcpImmersive::File => "file",
        }
    }
    /// What an export asks for (off unless set).
    pub fn of(s: &ExportSettings) -> Self {
        DcpImmersive::from_name(&s.dcp_immersive).unwrap_or(DcpImmersive::Off)
    }
}

/// Whether the main sound carries the 5.1 fallback (`5.1`, also empty) or silence (`none`).
fn fallback_name(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "" | "5.1" | "51" | "on" => Some(true),
        "none" | "off" => Some(false),
        _ => None,
    }
}

/// Errors in the immersive audio settings.
pub(crate) fn validate(s: &ExportSettings) -> Result<()> {
    let Some(mode) = DcpImmersive::from_name(&s.dcp_immersive) else {
        return Err(ExportError::Unsupported(format!("unknown DCP immersive audio {:?}: choose off, bed or file", s.dcp_immersive)));
    };
    if fallback_name(&s.dcp_iab_fallback).is_none() {
        return Err(ExportError::Unsupported(format!("unknown IAB fallback {:?}: choose 5.1 or none", s.dcp_iab_fallback)));
    }
    if mode == DcpImmersive::File && s.dcp_iab_source.trim().is_empty() {
        return Err(ExportError::Unsupported("immersive audio from a file needs an IAB source (an IAB MXF or .iab file)".into()));
    }
    Ok(())
}

/// ST 429-2 Configuration 4 (Table A.6, no configuration specified: CH01-CH16), the channel
/// assignment ST 429-19 7.1.3 asks of the main sound of an immersive audio composition.
pub(crate) const CHANNEL_CONFIG_4: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x0B, 0x04, 0x02, 0x02, 0x10, 0x03, 0x01, 0x04, 0x00];
/// Main sound channels of an immersive audio DCP.
pub(crate) const MAIN_SOUND_CHANNELS: u32 = 16;
/// Container channel 14 (ST 429-19 7.1.1), zero-based.
const FSK_CHANNEL: usize = 13;

// ---------------------------------------------------------------------------------------------
// ST 430-12 FSK synchronisation signal

/// 0x4D56 (6.2.1).
const SYNC_WORD: u16 = 0x4D56;

/// The FSK synchronisation signal of a main sound track file at 48 kHz: four samples per bit, a
/// zero a half cycle of 6 kHz, a one a full cycle of 12 kHz, each symbol starting where the last
/// one's slope left off (a zero flips the polarity, a one keeps it).
pub(crate) struct FskSync {
    uuid: [u8; 16],
    /// EditRate code (Table 2).
    code: u8,
    /// Sync packets per edit unit (Table 3).
    packets: u32,
    /// RemBits (Table 5).
    rem: u32,
    /// Packets sent: the UUIDSubIndex is this modulo 4.
    sent: u64,
    positive: bool,
}

impl FskSync {
    /// The signal of the track file `uuid` at `fps` (24, 25, 30, 48, 50, 60, 96, 100 or 120).
    pub fn new(uuid: [u8; 16], fps: u32) -> Option<Self> {
        let (code, packets, rem) = match fps {
            24 => (0, 4, 25),
            25 => (1, 4, 20),
            30 => (2, 4, 0),
            48 => (3, 2, 25),
            50 => (4, 2, 20),
            60 => (5, 2, 0),
            96 => (6, 1, 25),
            100 => (7, 1, 20),
            120 => (8, 1, 0),
            _ => return None,
        };
        Some(FskSync { uuid, code, packets, rem, sent: 0, positive: true })
    }

    /// The bits of one SyncPacket (Table 1), MSB first.
    fn packet_bits(&self, sub: u8, edit_unit: u32) -> Vec<bool> {
        let mut bits = Vec::with_capacity(125);
        let put = |v: u64, n: u32, bits: &mut Vec<bool>| (0..n).rev().for_each(|i| bits.push(v >> i & 1 == 1));
        put(u64::from(SYNC_WORD), 16, &mut bits);
        put(u64::from(self.code), 4, &mut bits);
        put(0, 2, &mut bits);
        put(u64::from(sub), 2, &mut bits);
        let k = usize::from(sub) * 4;
        let part = self.uuid.get(k..k + 4).map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
        put(u64::from(part), 32, &mut bits);
        put(u64::from(edit_unit & 0x00FF_FFFF), 24, &mut bits);
        let crc = crc16(&bits);
        put(u64::from(crc), 16, &mut bits);
        put(0, 4 + self.rem, &mut bits);
        bits
    }

    /// The samples of edit unit `index` (full scale ±1.0).
    pub fn edit_unit(&mut self, index: u32) -> Vec<f32> {
        const ZERO: [f32; 4] = [0.038_268_343, 0.092_387_95, 0.092_387_95, 0.038_268_343];
        const ONE: [f32; 4] = [0.070_710_68, 0.070_710_68, -0.070_710_68, -0.070_710_68];
        let mut out = Vec::new();
        for _ in 0..self.packets {
            let sub = (self.sent % 4) as u8;
            self.sent += 1;
            for bit in self.packet_bits(sub, index) {
                let sign = if self.positive { 1.0 } else { -1.0 };
                out.extend((if bit { ONE } else { ZERO }).iter().map(|v| v * sign));
                if !bit {
                    self.positive = !self.positive;
                }
            }
        }
        out
    }
}

/// CRC-16 with x^16 + x^12 + x^5 + 1 over `bits`, registers reset to zero (6.2.6).
fn crc16(bits: &[bool]) -> u16 {
    let mut r = 0u16;
    for &b in bits {
        let fb = (r >> 15 == 1) ^ b;
        r <<= 1;
        if fb {
            r ^= 0x1021;
        }
    }
    r
}

/// The FSK signal of `units` edit units (tests and tools).
pub fn fsk_sync(uuid: [u8; 16], fps: u32, units: u32) -> Option<Vec<f32>> {
    let mut f = FskSync::new(uuid, fps)?;
    Some((0..units).flat_map(|i| f.edit_unit(i)).collect())
}

// ---------------------------------------------------------------------------------------------
// IAB frames

/// ChannelIDs (ST 2098-2 Table 20) of the 7.1 DS bed: L R C LFE Lss Rss Lrs Rrs.
const BED_7_1_DS: [u32; 8] = [0x00, 0x04, 0x02, 0x0D, 0x05, 0x09, 0x07, 0x08];

/// A left-justified 24-bit sample (ST 429-18 7.2.3).
fn sample24(x: f32) -> i32 {
    let v = (x.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
    v.saturating_mul(256)
}

/// One IAFrame with a single 7.1 DS bed carrying the 5.1 `planar` (L R C LFE Ls Rs; the
/// surrounds split -3 dB to the side and the rear surrounds), DLC coded.
pub(crate) fn bed_frame(rate: FrameRate, planar: &[Vec<f32>]) -> Frame {
    let n = rate.samples(false, 0);
    let k = std::f32::consts::FRAC_1_SQRT_2;
    let ch = |i: usize, g: f32| -> Vec<i32> {
        let src = planar.get(i).map(Vec::as_slice).unwrap_or_default();
        (0..n).map(|j| sample24(src.get(j).copied().unwrap_or(0.0) * g)).collect()
    };
    let sources = [(0, 1.0), (1, 1.0), (2, 1.0), (3, 1.0), (4, k), (5, k), (4, k), (5, k)];
    let audio: Vec<AudioData> = sources.iter().enumerate().map(|(a, &(i, g))| AudioData { id: a as u32 + 1, coding: Coding::Dlc, samples: ch(i, g) }).collect();
    let channels = BED_7_1_DS.iter().enumerate().map(|(a, &id)| BedChannel { id, audio_id: a as u32 + 1, gain: Gain::Unity, decorrelation: None }).collect();
    Frame {
        version: 1,
        sample_rate: 48_000,
        bit_depth: 24,
        frame_rate: rate,
        frame_index: 0,
        max_rendered: BED_7_1_DS.len() as u32,
        beds: vec![Bed { meta_id: 1, use_case: None, channels, description: Description::default(), beds: Vec::new(), remaps: Vec::new() }],
        objects: Vec::new(),
        audio,
        authoring_tool: None,
        user_data: Vec::new(),
    }
}

/// A silent copy of `f` (padding after a short source): every asset's samples zeroed.
fn silenced(f: &Frame) -> Frame {
    let mut s = f.clone();
    for a in &mut s.audio {
        a.samples.iter_mut().for_each(|v| *v = 0);
        a.coding = Coding::Dlc;
    }
    s.user_data.clear();
    s.authoring_tool = None;
    s
}

/// Bed channels and objects of a frame (top level: the ones a renderer plays at once).
fn counts(f: &Frame) -> (u32, u32) {
    (f.beds.iter().map(|b| b.channels.len() as u32).sum(), f.objects.len() as u32)
}

/// Make a parsed source frame fit a DCP: DLC (ST 429-18 7.2.2), 24 bit (7.2.3), no UserData
/// (7.2.5), no AuthoringToolInfo (ISDCF Doc 15 IM2). Returns whether anything changed.
fn conform(f: &mut Frame) -> bool {
    let mut changed = false;
    for a in &mut f.audio {
        if a.coding != Coding::Dlc {
            a.coding = Coding::Dlc;
            changed = true;
        }
    }
    if f.bit_depth != 24 {
        f.bit_depth = 24;
        changed = true;
    }
    if !f.user_data.is_empty() {
        f.user_data.clear();
        changed = true;
    }
    if f.authoring_tool.take().is_some() {
        changed = true;
    }
    changed
}

fn iab_err(e: impl std::fmt::Display) -> ExportError {
    ExportError::Encode(format!("IAB: {e}"))
}

// ---------------------------------------------------------------------------------------------
// IAB sources

/// A file read through the MXF reader without loading it.
struct FileSrc {
    f: Mutex<std::fs::File>,
    len: u64,
}

impl ByteSource for FileSrc {
    fn len(&self) -> u64 {
        self.len
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
        let mut f = self.f.lock().unwrap_or_else(|e| e.into_inner());
        f.seek(SeekFrom::Start(offset))?;
        f.read_exact(buf)
    }
}

/// The frames of an IAB source.
enum Source {
    /// An IAB track file (IMF ST 2067-201 or DCP ST 429-18) and its IAB track.
    Mxf { src: FileSrc, file: Box<MxfFile>, track: usize },
    /// A raw IABitstream: the frames' (start, end).
    Raw { data: Vec<u8>, frames: Vec<(usize, usize)> },
}

impl Source {
    fn open(path: &str) -> Result<Source> {
        let f = std::fs::File::open(path).map_err(|e| ExportError::Io(format!("{path}: {e}")))?;
        let len = f.metadata().map_err(|e| ExportError::Io(format!("{path}: {e}")))?.len();
        let src = FileSrc { f: Mutex::new(f), len };
        let mut head = [0u8; 4];
        let is_mxf = src.read_at(0, &mut head).is_ok() && head == [0x06, 0x0E, 0x2B, 0x34];
        if is_mxf {
            let file = cfa4_mxf::open(&src).map_err(|e| ExportError::Io(format!("{path}: {e}")))?;
            let track =
                file.tracks.iter().position(|t| t.codec == Codec::Iab).ok_or_else(|| ExportError::Io(format!("{path}: no IAB track in this MXF file")))?;
            return Ok(Source::Mxf { src, file: Box::new(file), track });
        }
        let data = std::fs::read(path).map_err(|e| ExportError::Io(format!("{path}: {e}")))?;
        let base = data.as_ptr() as usize;
        let frames: Vec<(usize, usize)> =
            cfa4_iab::split(&data).iter().map(|f| (f.as_ptr() as usize - base, f.as_ptr() as usize - base + <[u8]>::len(f))).collect();
        if frames.is_empty() {
            return Err(ExportError::Io(format!("{path}: neither an MXF file nor an IABitstream")));
        }
        Ok(Source::Raw { data, frames })
    }

    fn len(&self) -> usize {
        match self {
            Source::Mxf { file, track, .. } => file.tracks.get(*track).map_or(0, |t| t.samples.len()),
            Source::Raw { frames, .. } => frames.len(),
        }
    }

    fn frame(&self, i: usize) -> Result<Vec<u8>> {
        match self {
            Source::Mxf { src, file, track } => file.read_sample(src, *track, i).map_err(|e| ExportError::Io(format!("IAB frame {i}: {e}"))),
            Source::Raw { data, frames } => {
                frames.get(i).and_then(|&(a, b)| data.get(a..b)).map(<[u8]>::to_vec).ok_or_else(|| ExportError::Io(format!("no IAB frame {i}")))
            }
        }
    }
}

/// What an IAB source holds (for the export dialog).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IabSourceInfo {
    /// `mxf` or `iab` (a raw IABitstream).
    pub container: String,
    pub frames: u64,
    /// Frames per second as (numerator, denominator).
    pub frame_rate: (u32, u32),
    pub sample_rate: u32,
    pub bit_depth: u8,
    /// Most bed channels / objects / MaxRendered in the frames looked at.
    pub max_bed_channels: u32,
    pub max_objects: u32,
    pub max_rendered: u32,
    /// Frames looked at (up to 240, spread over the source).
    pub scanned: u64,
    /// Some frames need re-coding (PCM assets, 16 bit, UserData or AuthoringToolInfo).
    pub needs_conform: bool,
    /// Where the frames exceed ISDCF Doc 15 (IAB Application Profile 1).
    pub profile_notes: Vec<String>,
}

/// Look at an IAB source: its frame count and rate, and the bed channels and objects of up to
/// 240 frames spread over it.
pub fn probe_iab_source(path: &str) -> Result<IabSourceInfo> {
    let src = Source::open(path)?;
    let n = src.len();
    let mut info = IabSourceInfo { container: if matches!(src, Source::Mxf { .. }) { "mxf" } else { "iab" }.into(), frames: n as u64, ..Default::default() };
    let step = n.div_ceil(240).max(1);
    let mut beds_max = 0;
    for i in (0..n).step_by(step) {
        let mut f = cfa4_iab::parse(&src.frame(i)?).map_err(|e| iab_err(format!("frame {i}: {e}")))?;
        if info.scanned == 0 {
            info.frame_rate = f.frame_rate.rational();
            info.sample_rate = f.sample_rate;
            info.bit_depth = f.bit_depth;
        }
        info.scanned += 1;
        let (c, o) = counts(&f);
        info.max_bed_channels = info.max_bed_channels.max(c);
        info.max_objects = info.max_objects.max(o);
        info.max_rendered = info.max_rendered.max(f.max_rendered);
        beds_max = beds_max.max(f.beds.len());
        info.needs_conform |= conform(&mut f);
    }
    let notes = &mut info.profile_notes;
    if info.sample_rate != 48_000 {
        notes.push(format!("sample rate {} Hz (Profile 1: 48 kHz)", info.sample_rate));
    }
    if info.max_bed_channels > 10 {
        notes.push(format!("{} bed channels (Profile 1: at most 10)", info.max_bed_channels));
    }
    if info.max_objects > 118 {
        notes.push(format!("{} objects (Profile 1: at most 118)", info.max_objects));
    }
    if info.max_rendered > 128 {
        notes.push(format!("MaxRendered {} (Profile 1: at most 128)", info.max_rendered));
    }
    if beds_max > 1 {
        notes.push(format!("{beds_max} beds in a frame (Profile 1: one)"));
    }
    Ok(info)
}

// ---------------------------------------------------------------------------------------------
// The tracks

/// The IAB track file's descriptor values before the frames are known ([`ImmersiveMux::finish`]
/// sets the counts).
pub(crate) fn initial_desc() -> IabDesc {
    IabDesc { version: 1, max_channels: 0, max_objects: 0, first_frame: 0, sample_rate: 48_000 }
}

/// The immersive audio of a DCP export in progress: the IAB track file and the 16-channel main
/// sound (fallback + FSK), written one edit unit at a time.
pub(crate) struct ImmersiveMux {
    mode: DcpImmersive,
    fallback: bool,
    rate: FrameRate,
    /// Samples per edit unit at 48 kHz.
    spu: usize,
    pub id: [u8; 16],
    pub name: String,
    iab: MxfWriter<Out>,
    fsk: FskSync,
    /// 5.1 mix waiting for a whole edit unit (`bed`).
    pending: Vec<Vec<f32>>,
    units: u64,
    desc: IabDesc,
    source: String,
}

impl ImmersiveMux {
    /// `iab` is the IAB track file writer, `sound_id` the main sound's asset UUID (in the FSK).
    pub fn new(s: &ExportSettings, fps: u32, id: [u8; 16], name: String, iab: MxfWriter<Out>, sound_id: [u8; 16]) -> Result<Self> {
        let rate = FrameRate::from_rational(fps, 1).ok_or_else(|| ExportError::Unsupported(format!("IAB has no {fps} fps frames")))?;
        let fsk = FskSync::new(sound_id, fps).ok_or_else(|| ExportError::Unsupported(format!("no FSK sync signal at {fps} fps")))?;
        Ok(ImmersiveMux {
            mode: DcpImmersive::of(s),
            fallback: fallback_name(&s.dcp_iab_fallback).unwrap_or(true),
            rate,
            spu: rate.samples(false, 0),
            id,
            name,
            iab,
            fsk,
            pending: vec![Vec::new(); 6],
            units: 0,
            desc: initial_desc(),
            source: s.dcp_iab_source.clone(),
        })
    }

    /// The export's 5.1 mix (`bed`; ignored for `file`, whose sound comes from the source).
    pub fn write_audio(&mut self, planar: &[Vec<f32>], sound: &mut MxfWriter<Out>) -> Result<()> {
        if self.mode != DcpImmersive::Bed {
            return Ok(());
        }
        for (p, c) in self.pending.iter_mut().zip(planar) {
            p.extend_from_slice(c);
        }
        while self.pending.iter().all(|p| p.len() >= self.spu) {
            let unit: Vec<Vec<f32>> = self.pending.iter_mut().map(|p| p.drain(..self.spu).collect()).collect();
            self.bed_unit(&unit, sound)?;
        }
        Ok(())
    }

    fn bed_unit(&mut self, unit: &[Vec<f32>], sound: &mut MxfWriter<Out>) -> Result<()> {
        let f = bed_frame(self.rate, unit);
        let bytes = cfa4_iab::write(&f).map_err(iab_err)?;
        self.push(&bytes, &f, Some(unit), sound)
    }

    /// One edit unit: the IAB frame and the main sound (fallback in 1-6, FSK in 14).
    fn push(&mut self, bytes: &[u8], f: &Frame, fallback: Option<&[Vec<f32>]>, sound: &mut MxfWriter<Out>) -> Result<()> {
        let (c, o) = counts(f);
        self.desc.version = f.version;
        self.desc.max_channels = self.desc.max_channels.max(c.min(u32::from(u16::MAX)) as u16);
        self.desc.max_objects = self.desc.max_objects.max(o.min(u32::from(u16::MAX)) as u16);
        self.iab.push_data(bytes).map_err(|e| ExportError::Io(e.to_string()))?;
        let mut planar = vec![vec![0.0f32; self.spu]; MAIN_SOUND_CHANNELS as usize];
        if self.fallback
            && let Some(fb) = fallback
        {
            for (dst, src) in planar.iter_mut().zip(fb.iter().take(6)) {
                for (d, s) in dst.iter_mut().zip(src) {
                    *d = *s;
                }
            }
        }
        let index = u32::try_from(self.units).unwrap_or(u32::MAX);
        if let Some(ch) = planar.get_mut(FSK_CHANNEL) {
            *ch = self.fsk.edit_unit(index);
            ch.resize(self.spu, 0.0);
        }
        sound.push_sound(0, &cfa4_mxf::encode_pcm(&planar, 24)).map_err(|e| ExportError::Io(e.to_string()))?;
        self.units += 1;
        Ok(())
    }

    /// Complete both tracks to `frames` edit units (the picture's); returns the IAB writer and
    /// its edit units.
    pub fn finish(mut self, frames: u64, sound: &mut MxfWriter<Out>) -> Result<(MxfWriter<Out>, u64)> {
        match self.mode {
            DcpImmersive::Bed | DcpImmersive::Off => {
                if self.pending.iter().any(|p| !p.is_empty()) {
                    let unit: Vec<Vec<f32>> = std::mem::take(&mut self.pending)
                        .into_iter()
                        .map(|mut p| {
                            p.resize(self.spu, 0.0);
                            p
                        })
                        .collect();
                    self.bed_unit(&unit, sound)?;
                }
                let silence = vec![vec![0.0f32; self.spu]; 6];
                while self.units < frames {
                    self.bed_unit(&silence, sound)?;
                }
            }
            DcpImmersive::File => self.copy_source(frames, sound)?,
        }
        let desc = self.desc;
        self.iab.set_iab(desc).map_err(|e| ExportError::Io(e.to_string()))?;
        let units = self.iab.edit_units();
        Ok((self.iab, units))
    }

    /// `file`: the source's frames up to `frames`, then silent ones.
    fn copy_source(&mut self, frames: u64, sound: &mut MxfWriter<Out>) -> Result<()> {
        let path = self.source.clone();
        let src = Source::open(&path)?;
        let layout = Layout::surround_5_1();
        let mut last: Option<Frame> = None;
        let want = self.rate.rational();
        for i in 0..src.len().min(usize::try_from(frames).unwrap_or(usize::MAX)) {
            let bytes = src.frame(i)?;
            let mut f = cfa4_iab::parse(&bytes).map_err(|e| iab_err(format!("{path}: frame {i}: {e}")))?;
            if f.frame_rate.rational() != want {
                let (n, d) = f.frame_rate.rational();
                return Err(ExportError::Unsupported(format!(
                    "the IAB source runs at {} fps, the DCP at {} fps",
                    if d == 1 { n.to_string() } else { format!("{n}/{d}") },
                    want.0
                )));
            }
            if f.sample_rate != 48_000 {
                return Err(ExportError::Unsupported(format!("the IAB source is {} Hz; DCP immersive audio is 48 kHz", f.sample_rate)));
            }
            let fb = if self.fallback { Some(cfa4_iab::render(&f, &layout).map_err(iab_err)?) } else { None };
            let out = if conform(&mut f) { cfa4_iab::write(&f).map_err(iab_err)? } else { bytes };
            self.push(&out, &f, fb.as_deref(), sound)?;
            last = Some(f);
        }
        if self.units < frames {
            // a short source: silent frames with the last frame's metadata (or an empty bed)
            let pad = match &last {
                Some(f) => silenced(f),
                None => bed_frame(self.rate, &[]),
            };
            let bytes = cfa4_iab::write(&pad).map_err(iab_err)?;
            while self.units < frames {
                self.push(&bytes, &pad, None, sound)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_of_the_standard_check_string() {
        // CRC-16/XMODEM (poly 0x1021, init 0, no reflection) of "123456789" is 0x31C3
        let bits: Vec<bool> = b"123456789".iter().flat_map(|b| (0..8).rev().map(move |i| b >> i & 1 == 1)).collect();
        assert_eq!(crc16(&bits), 0x31C3);
    }

    /// Demodulate 48 kHz FSK: four samples per bit, a one changes sign inside the symbol.
    fn demodulate(s: &[f32]) -> Vec<bool> {
        s.chunks_exact(4).map(|c| (c[0] > 0.0) != (c[3] > 0.0)).collect()
    }

    #[test]
    fn fsk_packets_follow_st_430_12() {
        let uuid = [0x10, 0x11, 0x12, 0x13, 0x20, 0x21, 0x22, 0x23, 0x30, 0x31, 0x32, 0x33, 0x40, 0x41, 0x42, 0x43];
        for (fps, packets, spu) in [(24u32, 4usize, 2000usize), (25, 4, 1920), (30, 4, 1600), (48, 2, 1000), (50, 2, 960), (60, 2, 800)] {
            let s = fsk_sync(uuid, fps, 3).unwrap();
            assert_eq!(s.len(), 3 * spu, "{fps} fps");
            let bits = demodulate(&s);
            let per_packet = bits.len() / (3 * packets);
            for (p, pk) in bits.chunks_exact(per_packet).enumerate() {
                let field = |a: usize, n: usize| pk[a..a + n].iter().fold(0u64, |v, b| v << 1 | u64::from(*b));
                assert_eq!(field(0, 16), 0x4D56);
                assert_eq!(field(20, 2), 0);
                let sub = field(22, 2) as usize;
                assert_eq!(sub, p % 4);
                let k = sub * 4;
                assert_eq!(field(24, 32), u64::from(u32::from_be_bytes(uuid[k..k + 4].try_into().unwrap())));
                assert_eq!(field(56, 24), (p / packets) as u64, "EditUnitIndex");
                assert_eq!(field(80, 16), u64::from(crc16(&pk[..80])));
                assert!(pk[96..].iter().all(|b| !b), "reserved and RemBits are zero");
            }
            // the first bit (a zero) is a positive half cycle; peaks stay near 0.1
            assert!(s[0] > 0.0 && s[1] > s[0]);
            assert!(s.iter().all(|v| v.abs() <= 0.0925));
        }
        assert!(FskSync::new(uuid, 23).is_none());
    }

    #[test]
    fn fsk_symbols_join_without_a_slope_break() {
        // Figure 1: the 11th bit of the SyncWord, a zero, is a negative half cycle
        let s = fsk_sync([0; 16], 24, 1).unwrap();
        let sym = &s[40..44];
        assert!(sym.iter().all(|v| *v < 0.0), "{sym:?}");
    }

    #[test]
    fn bed_frames_are_profile_1_7_1_ds_and_lossless() {
        let n = 2000;
        let planar: Vec<Vec<f32>> = (0..6).map(|c| (0..n).map(|i| ((i * (c + 1)) as f32 * 0.01).sin() * 0.5).collect()).collect();
        let f = bed_frame(FrameRate::R24, &planar);
        let bytes = cfa4_iab::write(&f).unwrap();
        let back = cfa4_iab::parse(&bytes).unwrap();
        assert_eq!(back, f);
        assert_eq!(back.beds.len(), 1);
        let ids: Vec<u32> = back.beds[0].channels.iter().map(|c| c.id).collect();
        assert_eq!(ids, BED_7_1_DS);
        assert!(back.audio.iter().all(|a| a.coding == Coding::Dlc));
        assert!(back.authoring_tool.is_none() && back.user_data.is_empty());
        // L passes through; Ls feeds the side and the rear surround at -3 dB
        assert_eq!(back.audio[0].samples[100], sample24(planar[0][100]));
        let ls = planar[4][100] * std::f32::consts::FRAC_1_SQRT_2;
        assert_eq!(back.audio[4].samples[100], sample24(ls));
        assert_eq!(back.audio[6].samples[100], sample24(ls));
        // rendered to 5.1, centre and LFE come back as mixed
        let out = cfa4_iab::render(&back, &Layout::surround_5_1()).unwrap();
        for c in 2..4 {
            assert!((out[c][100] - planar[c][100]).abs() < 1e-5, "channel {c}");
        }
    }

    #[test]
    fn conform_recodes_pcm_and_strips_user_data() {
        let mut f = bed_frame(FrameRate::R24, &[]);
        assert!(!conform(&mut f));
        f.audio[0].coding = Coding::Pcm;
        f.bit_depth = 16;
        f.authoring_tool = Some("x".into());
        assert!(conform(&mut f));
        assert!(f.audio.iter().all(|a| a.coding == Coding::Dlc) && f.bit_depth == 24 && f.authoring_tool.is_none());
    }

    #[test]
    fn immersive_names() {
        for m in DcpImmersive::ALL {
            assert_eq!(DcpImmersive::from_name(m.id()), Some(m));
        }
        assert_eq!(DcpImmersive::from_name(""), Some(DcpImmersive::Off));
        assert_eq!(DcpImmersive::from_name("atmos"), None);
        let mut s = ExportSettings { dcp_immersive: "file".into(), ..Default::default() };
        assert!(validate(&s).is_err(), "a file without a source");
        s.dcp_iab_source = "/x.mxf".into();
        assert!(validate(&s).is_ok());
        s.dcp_iab_fallback = "stereo".into();
        assert!(validate(&s).is_err());
    }
}
