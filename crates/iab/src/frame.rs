//! IABitstream frames and the IAFrame element tree (clauses 5, 7-10).

use cfa4_bitstream::BitReader;

use crate::{Error, Result, dlc, element};

/// Deepest nesting of conditional Beds / Objects accepted (Annex C allows two levels).
const MAX_DEPTH: usize = 8;
/// Longest AudioDescriptionText / AuthoringToolURI read (Annex C caps descriptions at 64 bytes).
const MAX_TEXT: usize = 1024;

/// IAFrame Rate (Table 18).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FrameRate {
    R24,
    R25,
    R30,
    R48,
    R50,
    R60,
    R96,
    R100,
    R120,
    R23_976,
    R29_97,
    R59_94,
}

impl FrameRate {
    pub const ALL: [FrameRate; 12] = [
        FrameRate::R24,
        FrameRate::R25,
        FrameRate::R30,
        FrameRate::R48,
        FrameRate::R50,
        FrameRate::R60,
        FrameRate::R96,
        FrameRate::R100,
        FrameRate::R120,
        FrameRate::R23_976,
        FrameRate::R29_97,
        FrameRate::R59_94,
    ];

    /// The 4-bit FrameRate code.
    pub fn code(self) -> u8 {
        match self {
            FrameRate::R24 => 0,
            FrameRate::R25 => 1,
            FrameRate::R30 => 2,
            FrameRate::R48 => 3,
            FrameRate::R50 => 4,
            FrameRate::R60 => 5,
            FrameRate::R96 => 6,
            FrameRate::R100 => 7,
            FrameRate::R120 => 8,
            FrameRate::R23_976 => 9,
            FrameRate::R29_97 => 10,
            FrameRate::R59_94 => 11,
        }
    }

    pub fn from_code(code: u32) -> Option<FrameRate> {
        FrameRate::ALL.into_iter().find(|r| u32::from(r.code()) == code)
    }

    /// Frames per second as (numerator, denominator).
    pub fn rational(self) -> (u32, u32) {
        match self {
            FrameRate::R24 => (24, 1),
            FrameRate::R25 => (25, 1),
            FrameRate::R30 => (30, 1),
            FrameRate::R48 => (48, 1),
            FrameRate::R50 => (50, 1),
            FrameRate::R60 => (60, 1),
            FrameRate::R96 => (96, 1),
            FrameRate::R100 => (100, 1),
            FrameRate::R120 => (120, 1),
            FrameRate::R23_976 => (24000, 1001),
            FrameRate::R29_97 => (30000, 1001),
            FrameRate::R59_94 => (60000, 1001),
        }
    }

    /// The rate from a frames-per-second fraction (e.g. an MXF edit rate).
    pub fn from_rational(num: u32, den: u32) -> Option<FrameRate> {
        FrameRate::ALL.into_iter().find(|r| {
            let (n, d) = r.rational();
            u64::from(n) * u64::from(den) == u64::from(num) * u64::from(d)
        })
    }

    /// The 1001 rates whose sample counts follow a five-frame cadence (FrameSequence).
    pub fn has_sequence(self) -> bool {
        matches!(self, FrameRate::R29_97 | FrameRate::R59_94)
    }

    /// Samples per frame (Table 19) at 48 or 96 kHz, for FrameIndex `index` (taken modulo 5).
    pub fn samples(self, rate96: bool, index: u8) -> usize {
        let i = usize::from(index % 5);
        let n48 = match self {
            FrameRate::R24 => 2000,
            FrameRate::R25 => 1920,
            FrameRate::R30 => 1600,
            FrameRate::R48 => 1000,
            FrameRate::R50 => 960,
            FrameRate::R60 => 800,
            FrameRate::R96 => 500,
            FrameRate::R100 => 480,
            FrameRate::R120 => 400,
            FrameRate::R23_976 => 2002,
            FrameRate::R29_97 => {
                return if rate96 { [3204, 3203, 3203, 3203, 3203][i] } else { [1602, 1601, 1602, 1601, 1602][i] };
            }
            FrameRate::R59_94 => {
                return if rate96 { [1602, 1601, 1602, 1601, 1602][i] } else { [801, 801, 800, 801, 801][i] };
            }
        };
        if rate96 { n48 * 2 } else { n48 }
    }

    /// Number of panning sub-blocks per frame (Table 24).
    pub fn sub_blocks(self) -> usize {
        match self {
            FrameRate::R24 | FrameRate::R25 | FrameRate::R30 | FrameRate::R23_976 | FrameRate::R29_97 => 8,
            FrameRate::R48 | FrameRate::R50 | FrameRate::R60 | FrameRate::R59_94 => 4,
            FrameRate::R96 | FrameRate::R100 | FrameRate::R120 => 2,
        }
    }

    /// Samples of each panning sub-block (Table 24), summing to [`FrameRate::samples`].
    pub fn sub_block_sizes(self, rate96: bool, index: u8) -> Vec<usize> {
        let i = usize::from(index % 5);
        let even = |n: usize| vec![n / self.sub_blocks(); self.sub_blocks()];
        match (self, rate96) {
            (FrameRate::R23_976, false) => vec![251, 250, 250, 250, 251, 250, 250, 250],
            (FrameRate::R23_976, true) => vec![501, 500, 501, 500, 501, 500, 501, 500],
            (FrameRate::R29_97, false) if i % 2 == 0 => vec![201, 200, 200, 200, 201, 200, 200, 200],
            (FrameRate::R29_97, false) => vec![201, 200, 200, 200, 200, 200, 200, 200],
            (FrameRate::R29_97, true) if i == 0 => vec![401, 400, 401, 400, 401, 400, 401, 400],
            (FrameRate::R29_97, true) => vec![401, 400, 400, 401, 400, 400, 401, 400],
            (FrameRate::R59_94, false) if i == 2 => vec![200; 4],
            (FrameRate::R59_94, false) => vec![201, 200, 200, 200],
            (FrameRate::R59_94, true) if i % 2 == 0 => vec![401, 400, 401, 400],
            (FrameRate::R59_94, true) => vec![401, 400, 400, 400],
            _ => even(self.samples(rate96, 0)),
        }
    }

    /// NumDLCSubBlocks and DLCSubBlockSize48 (Table 31); `None` for the 1001 rates, which cannot
    /// carry DLC.
    pub fn dlc_blocks(self) -> Option<(usize, usize)> {
        Some(match self {
            FrameRate::R24 => (10, 200),
            FrameRate::R25 => (10, 192),
            FrameRate::R30 => (8, 200),
            FrameRate::R48 => (5, 200),
            FrameRate::R50 => (5, 192),
            FrameRate::R60 => (4, 200),
            FrameRate::R96 => (5, 100),
            FrameRate::R100 => (4, 120),
            FrameRate::R120 => (4, 100),
            _ => return None,
        })
    }
}

/// A 10-bit gain with its prefix (5.5, Table 21).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gain {
    Unity,
    Silent,
    /// 2^(-code / 64); 0x3FF is silence.
    Code(u16),
}

impl Gain {
    pub fn linear(self) -> f32 {
        match self {
            Gain::Unity => 1.0,
            Gain::Silent => 0.0,
            Gain::Code(0x3FF) => 0.0,
            Gain::Code(c) => (-f32::from(c) / 64.0).exp2(),
        }
    }
}

/// A zone gain with its prefix (Table 26; note 0 is silence and 1 unity here).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoneGain {
    Zero,
    Unity,
    /// code / 1023.
    Code(u16),
}

impl ZoneGain {
    pub fn linear(self) -> f32 {
        match self {
            ZoneGain::Zero => 0.0,
            ZoneGain::Unity => 1.0,
            ZoneGain::Code(c) => f32::from(c.min(1023)) / 1023.0,
        }
    }
}

/// Decorrelation (Tables 22 and 28): 0 none … 255 maximum. The algorithm is the renderer's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decorrelation {
    None,
    Max,
    Coef(u8),
}

impl Decorrelation {
    pub fn amount(self) -> u8 {
        match self {
            Decorrelation::None => 0,
            Decorrelation::Max => 255,
            Decorrelation::Coef(c) => c,
        }
    }
}

/// AudioDescription bits (Table 23) and the optional text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Description {
    pub bits: u8,
    pub text: Option<String>,
}

/// One channel of a Bed.
#[derive(Clone, Debug, PartialEq)]
pub struct BedChannel {
    /// ChannelID (Table 20): the routing destination.
    pub id: u32,
    /// The audio asset; 0 is silence.
    pub audio_id: u32,
    pub gain: Gain,
    pub decorrelation: Option<Decorrelation>,
}

/// One sub-block of a BedRemap matrix.
#[derive(Clone, Debug, PartialEq)]
pub struct RemapBlock {
    /// DestinationChannelID of each output.
    pub destinations: Vec<u32>,
    /// gains[output][source channel].
    pub gains: Vec<Vec<Gain>>,
}

/// A BedRemap: how to play the parent Bed on another loudspeaker configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct Remap {
    pub meta_id: u32,
    pub use_case: u8,
    pub sources: u32,
    pub destinations: u32,
    /// One entry per sub-block; `None` keeps the previous sub-block's matrix.
    pub blocks: Vec<Option<RemapBlock>>,
}

/// A BedDefinition with its conditional child Beds and remaps.
#[derive(Clone, Debug, PartialEq)]
pub struct Bed {
    pub meta_id: u32,
    /// `Some` for a conditional Bed: activated only in that Target Environment (or 0xFF).
    pub use_case: Option<u8>,
    pub channels: Vec<BedChannel>,
    pub description: Description,
    pub beds: Vec<Bed>,
    pub remaps: Vec<Remap>,
}

/// ObjectSnap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Snap {
    Off,
    /// Snap to the nearest loudspeaker within the tolerance (12-bit DistanceZ code; `None` is
    /// DEFAULT_OBJ_SNAP_TOL, 1.0: always snap).
    On(Option<u16>),
}

/// ObjectSpreadMode (Table 27) and its codes (DistanceZ coding).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spread {
    /// Equal spread, 8-bit code.
    LowRes(u8),
    /// Point source.
    None,
    /// Equal spread, 12-bit code.
    OneD(u16),
    /// Spread per axis, 12-bit codes.
    ThreeD([u16; 3]),
}

impl Spread {
    /// Extent in each dimension, 0.0 to 1.0.
    pub fn extent(self) -> [f32; 3] {
        let z = |c: u32, n: u32| c as f32 / ((1u32 << n) - 1) as f32;
        match self {
            Spread::LowRes(c) => [z(u32::from(c), 8); 3],
            Spread::None => [0.0; 3],
            Spread::OneD(c) => [z(u32::from(c.min(4095)), 12); 3],
            Spread::ThreeD(c) => c.map(|v| z(u32::from(v.min(4095)), 12)),
        }
    }
}

/// Panning information of one Object sub-block.
#[derive(Clone, Debug, PartialEq)]
pub struct Pan {
    pub gain: Gain,
    /// ObjectPosX / Y / Z codes (5.4, 16 bits).
    pub position: [u16; 3],
    pub snap: Snap,
    /// Gains of the nine zones of Table 25, when zone control is on.
    pub zones: Option<[ZoneGain; 9]>,
    pub spread: Spread,
    pub decorrelation: Decorrelation,
}

impl Pan {
    /// Position in the unit room cube: x left → right, y front → back, z floor → ceiling.
    pub fn position(&self) -> [f32; 3] {
        let [x, y, z] = self.position;
        [distance_xy(x), distance_xy(y), f32::from(z) / 65535.0]
    }
}

/// DistanceXY of a 16-bit code (Formula 1): codes below 2^15 - 1 lie outside the room and clamp
/// to 0.
fn distance_xy(code: u16) -> f32 {
    ((f32::from(code) - 32767.0) / 32768.0).clamp(0.0, 1.0)
}

/// ObjectZoneDefinition19: the 19 zones of Table 29, per sub-block (`None` keeps the previous).
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectZones19 {
    pub blocks: Vec<Option<[ZoneGain; 19]>>,
}

/// An ObjectDefinition with its conditional replacements.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub meta_id: u32,
    pub audio_id: u32,
    pub use_case: Option<u8>,
    /// One entry per sub-block; `None` keeps the previous sub-block's panning (the first is
    /// always present).
    pub pan: Vec<Option<Pan>>,
    pub description: Description,
    pub objects: Vec<Object>,
    pub zones19: Option<ObjectZones19>,
}

/// How an audio asset was (or is to be) coded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coding {
    Pcm,
    Dlc,
}

/// One mono audio asset of the frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioData {
    pub id: u32,
    pub coding: Coding,
    /// Samples left-justified in 32 bits (full scale ±2^31), [`FrameRate::samples`] of them.
    pub samples: Vec<i32>,
}

/// A UserData element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserData {
    pub user_id: [u8; 16],
    pub bytes: Vec<u8>,
}

/// One decoded IAFrame.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub version: u8,
    /// 48000 or 96000.
    pub sample_rate: u32,
    /// 16 or 24.
    pub bit_depth: u8,
    pub frame_rate: FrameRate,
    /// FrameSequence index (0-4) for the 30000/1001 and 60000/1001 rates, else 0.
    pub frame_index: u8,
    pub max_rendered: u32,
    pub beds: Vec<Bed>,
    pub objects: Vec<Object>,
    pub audio: Vec<AudioData>,
    pub authoring_tool: Option<String>,
    pub user_data: Vec<UserData>,
}

impl Frame {
    pub fn rate96(&self) -> bool {
        self.sample_rate == 96_000
    }
    /// Samples per channel in this frame.
    pub fn samples(&self) -> usize {
        self.frame_rate.samples(self.rate96(), self.frame_index)
    }
    /// The asset with this AudioDataID.
    pub fn audio_data(&self, id: u32) -> Option<&AudioData> {
        (id != 0).then(|| self.audio.iter().find(|a| a.id == id)).flatten()
    }
}

/// Plex(n) (5.2): an n-bit field whose all-ones value escapes to a field twice as wide.
pub(crate) fn plex(r: &mut BitReader, n: u32) -> Result<u32> {
    let mut width = n;
    loop {
        let v = r.read_bits(width)?;
        let ones = if width >= 32 { u32::MAX } else { (1u32 << width) - 1 };
        if v != ones {
            return Ok(v);
        }
        if width >= 32 {
            return Err(Error::Invalid("Plex value 0xFFFFFFFF"));
        }
        width *= 2;
    }
}

/// Splits an element at the start of `data`: (ElementID, payload, bytes taken).
fn element(data: &[u8]) -> Result<(u32, &[u8], usize)> {
    let mut r = BitReader::new(data);
    let id = plex(&mut r, 8)?;
    let size = plex(&mut r, 8)? as usize;
    let start = r.byte_pos();
    let end = start.checked_add(size).ok_or(Error::Truncated)?;
    let payload = data.get(start..end).ok_or(Error::Truncated)?;
    Ok((id, payload, end))
}

/// The children of an element: `count` elements in `data`.
fn children(data: &[u8], count: u32) -> Result<Vec<(u32, &[u8])>> {
    let mut out = Vec::new();
    let mut at = 0;
    for _ in 0..count {
        let rest = data.get(at..).ok_or(Error::Truncated)?;
        if rest.is_empty() {
            return Err(Error::Truncated);
        }
        let (id, payload, used) = element(rest)?;
        out.push((id, payload));
        at += used;
    }
    Ok(out)
}

/// Total length of the IABitstream frame at the start of `data` (preamble and IAFrame
/// segments), from its first bytes; `None` if they are not an IABitstream frame header.
pub fn frame_size(data: &[u8]) -> Option<usize> {
    if data.first() != Some(&0x01) {
        return None;
    }
    let pre = be32(data, 1)?;
    let tag = 5usize.checked_add(pre)?;
    if data.get(tag) != Some(&0x02) {
        return None;
    }
    tag.checked_add(5)?.checked_add(be32(data, tag.checked_add(1)?)?)
}

fn be32(data: &[u8], at: usize) -> Option<usize> {
    let b: [u8; 4] = data.get(at..at.checked_add(4)?)?.try_into().ok()?;
    usize::try_from(u32::from_be_bytes(b)).ok()
}

/// Splits a concatenation of IABitstream frames; stops at the first malformed one.
pub fn split(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(rest) = data.get(at..).filter(|r| !r.is_empty()) {
        let Some(n) = frame_size(rest).filter(|&n| n <= rest.len()) else { break };
        out.push(&rest[..n]);
        at += n;
    }
    out
}

/// Frame-wide values the children need.
#[derive(Clone, Copy)]
pub(crate) struct Ctx {
    pub rate: FrameRate,
    pub rate96: bool,
    pub bit_depth: u8,
    pub index: u8,
}

impl Ctx {
    fn sub_blocks(&self) -> usize {
        self.rate.sub_blocks()
    }
}

/// Parses one IABitstream frame (preamble and IAFrame segments) or a bare IAFrame element, and
/// decodes its audio.
pub fn parse(data: &[u8]) -> Result<Frame> {
    let body = if data.first() == Some(&0x01) {
        let n = frame_size(data).ok_or(Error::Invalid("IABitstream frame header"))?;
        let all = data.get(..n).ok_or(Error::Truncated)?;
        let pre = be32(all, 1).ok_or(Error::Truncated)?;
        all.get(pre.saturating_add(10)..).ok_or(Error::Truncated)?
    } else {
        data
    };
    let (id, payload, _) = element(body)?;
    if id != element::IA_FRAME {
        return Err(Error::Invalid("not an IAFrame element"));
    }
    iaframe(payload)
}

fn iaframe(p: &[u8]) -> Result<Frame> {
    let mut r = BitReader::new(p);
    let version = r.read_bits(8)? as u8;
    if version == 0 || version == 2 {
        return Err(Error::Invalid("forbidden IAFrame version"));
    }
    let sample_rate = match r.read_bits(2)? {
        0 => 48_000,
        1 => 96_000,
        _ => return Err(Error::Unsupported("reserved SampleRate")),
    };
    let bit_depth = match r.read_bits(2)? {
        0 => 16,
        1 => 24,
        _ => return Err(Error::Unsupported("reserved BitDepth")),
    };
    let frame_rate = FrameRate::from_code(r.read_bits(4)?).ok_or(Error::Unsupported("reserved FrameRate"))?;
    let max_rendered = plex(&mut r, 8)?;
    r.byte_align();
    let count = plex(&mut r, 8)?;
    let rest = p.get(r.byte_pos()..).ok_or(Error::Truncated)?;
    let mut f = Frame {
        version,
        sample_rate,
        bit_depth,
        frame_rate,
        frame_index: 0,
        max_rendered,
        beds: Vec::new(),
        objects: Vec::new(),
        audio: Vec::new(),
        authoring_tool: None,
        user_data: Vec::new(),
    };
    let mut ctx = Ctx { rate: frame_rate, rate96: sample_rate == 96_000, bit_depth, index: 0 };
    for (id, c) in children(rest, count)? {
        match id {
            element::FRAME_SEQUENCE => {
                let i = *c.first().ok_or(Error::Truncated)?;
                if i > 4 {
                    return Err(Error::Invalid("FrameIndex above 4"));
                }
                ctx.index = i;
                f.frame_index = i;
            }
            element::BED_DEFINITION => f.beds.push(bed(c, &ctx, 0)?),
            element::OBJECT_DEFINITION => f.objects.push(object(c, &ctx, 0)?),
            element::AUDIO_DATA_DLC => {
                let mut r = BitReader::new(c);
                let id = plex(&mut r, 8)?;
                let _dlc_size = r.read_bits(16)?;
                let samples = dlc::decode(&mut r, &ctx)?;
                push_audio(&mut f.audio, AudioData { id, coding: Coding::Dlc, samples })?;
            }
            element::AUDIO_DATA_PCM => {
                let mut r = BitReader::new(c);
                let id = plex(&mut r, 8)?;
                let samples = pcm(c.get(r.byte_pos()..).unwrap_or_default(), &ctx)?;
                push_audio(&mut f.audio, AudioData { id, coding: Coding::Pcm, samples })?;
            }
            element::AUTHORING_TOOL_INFO => f.authoring_tool = Some(text(c)),
            element::USER_DATA => {
                let user_id: [u8; 16] = c.get(..16).and_then(|b| b.try_into().ok()).ok_or(Error::Truncated)?;
                f.user_data.push(UserData { user_id, bytes: c.get(16..).unwrap_or_default().to_vec() });
            }
            _ => {}
        }
    }
    Ok(f)
}

fn push_audio(list: &mut Vec<AudioData>, a: AudioData) -> Result<()> {
    if a.id == 0 {
        return Err(Error::Invalid("AudioDataID 0"));
    }
    if list.iter().any(|b| b.id == a.id) {
        return Err(Error::Invalid("duplicate AudioDataID"));
    }
    list.push(a);
    Ok(())
}

/// AudioDataPCM samples: little-endian 16 or 24 bits, left-justified to 32.
fn pcm(b: &[u8], ctx: &Ctx) -> Result<Vec<i32>> {
    let n = ctx.rate.samples(ctx.rate96, ctx.index);
    let w = usize::from(ctx.bit_depth / 8);
    let bytes = b.get(..n * w).ok_or(Error::Truncated)?;
    Ok(bytes
        .chunks_exact(w)
        .map(|s| match s {
            [a, b] => i32::from(i16::from_le_bytes([*a, *b])) << 16,
            [a, b, c] => i32::from_le_bytes([0, *a, *b, *c]),
            _ => 0,
        })
        .collect())
}

/// A NUL-terminated ASCII string (the whole slice when there is no NUL).
fn text(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len()).min(MAX_TEXT);
    String::from_utf8_lossy(&b[..end]).into_owned()
}

fn gain(r: &mut BitReader) -> Result<Gain> {
    Ok(match r.read_bits(2)? {
        0 => Gain::Unity,
        1 => Gain::Silent,
        2 => Gain::Code(r.read_bits(10)? as u16),
        _ => Gain::Unity, // reserved
    })
}

fn zone_gain(r: &mut BitReader) -> Result<ZoneGain> {
    Ok(match r.read_bits(2)? {
        0 => ZoneGain::Zero,
        1 => ZoneGain::Unity,
        2 => ZoneGain::Code(r.read_bits(10)? as u16),
        _ => ZoneGain::Unity, // reserved
    })
}

fn decorrelation(r: &mut BitReader) -> Result<Decorrelation> {
    Ok(match r.read_bits(2)? {
        0 => Decorrelation::None,
        1 => Decorrelation::Max,
        2 => Decorrelation::Coef(r.read_bits(8)? as u8),
        _ => Decorrelation::None, // reserved
    })
}

/// AudioDescription and its text, at a byte boundary.
fn description(r: &mut BitReader) -> Result<Description> {
    let bits = r.read_bits(8)? as u8;
    let mut text = None;
    if bits & 0x80 != 0 {
        let mut s = Vec::new();
        loop {
            let c = r.read_bits(8)? as u8;
            if c == 0 {
                break;
            }
            if s.len() < MAX_TEXT {
                s.push(c);
            }
        }
        text = Some(String::from_utf8_lossy(&s).into_owned());
    }
    Ok(Description { bits, text })
}

fn bed(p: &[u8], ctx: &Ctx, depth: usize) -> Result<Bed> {
    if depth > MAX_DEPTH {
        return Err(Error::Invalid("BedDefinition nesting too deep"));
    }
    let mut r = BitReader::new(p);
    let meta_id = plex(&mut r, 8)?;
    let use_case = if r.read_bit()? { Some(r.read_bits(8)? as u8) } else { None };
    let count = plex(&mut r, 4)?;
    let mut channels = Vec::new();
    for _ in 0..count {
        let id = plex(&mut r, 4)?;
        let audio_id = plex(&mut r, 8)?;
        let gain = gain(&mut r)?;
        let decorrelation = if r.read_bit()? {
            r.skip(4)?;
            Some(decorrelation(&mut r)?)
        } else {
            None
        };
        channels.push(BedChannel { id, audio_id, gain, decorrelation });
    }
    r.skip(10)?;
    r.byte_align();
    let description = description(&mut r)?;
    let n = plex(&mut r, 8)?;
    let mut b = Bed { meta_id, use_case, channels, description, beds: Vec::new(), remaps: Vec::new() };
    for (id, c) in children(p.get(r.byte_pos()..).unwrap_or_default(), n)? {
        match id {
            element::BED_DEFINITION => b.beds.push(bed(c, ctx, depth + 1)?),
            element::BED_REMAP => b.remaps.push(remap(c, ctx)?),
            _ => {}
        }
    }
    Ok(b)
}

fn remap(p: &[u8], ctx: &Ctx) -> Result<Remap> {
    let mut r = BitReader::new(p);
    let meta_id = plex(&mut r, 8)?;
    let use_case = r.read_bits(8)? as u8;
    let sources = plex(&mut r, 4)?;
    let destinations = plex(&mut r, 4)?;
    let mut blocks = Vec::new();
    for sb in 0..ctx.sub_blocks() {
        if sb > 0 && !r.read_bit()? {
            blocks.push(None);
            continue;
        }
        let mut block = RemapBlock { destinations: Vec::new(), gains: Vec::new() };
        for _ in 0..destinations {
            block.destinations.push(plex(&mut r, 4)?);
            let mut row = Vec::new();
            for _ in 0..sources {
                row.push(gain(&mut r)?);
            }
            block.gains.push(row);
        }
        blocks.push(Some(block));
    }
    Ok(Remap { meta_id, use_case, sources, destinations, blocks })
}

fn object(p: &[u8], ctx: &Ctx, depth: usize) -> Result<Object> {
    if depth > MAX_DEPTH {
        return Err(Error::Invalid("ObjectDefinition nesting too deep"));
    }
    let mut r = BitReader::new(p);
    let meta_id = plex(&mut r, 8)?;
    let audio_id = plex(&mut r, 8)?;
    let use_case = if r.read_bit()? {
        r.skip(1)?;
        Some(r.read_bits(8)? as u8)
    } else {
        None
    };
    r.skip(1)?;
    let mut pan = Vec::new();
    for sb in 0..ctx.sub_blocks() {
        if sb > 0 && !r.read_bit()? {
            pan.push(None);
            continue;
        }
        let gain = gain(&mut r)?;
        r.skip(3)?;
        let position = [r.read_bits(16)? as u16, r.read_bits(16)? as u16, r.read_bits(16)? as u16];
        let snap = if r.read_bit()? {
            let tol = if r.read_bit()? { Some(r.read_bits(12)? as u16) } else { None };
            r.skip(1)?;
            Snap::On(tol)
        } else {
            Snap::Off
        };
        let zones = if r.read_bit()? {
            let mut z = [ZoneGain::Unity; 9];
            for g in &mut z {
                *g = zone_gain(&mut r)?;
            }
            Some(z)
        } else {
            None
        };
        let spread = match r.read_bits(2)? {
            0 => Spread::LowRes(r.read_bits(8)? as u8),
            1 => Spread::None,
            2 => Spread::OneD(r.read_bits(12)? as u16),
            _ => Spread::ThreeD([r.read_bits(12)? as u16, r.read_bits(12)? as u16, r.read_bits(12)? as u16]),
        };
        r.skip(4)?;
        let decorrelation = decorrelation(&mut r)?;
        pan.push(Some(Pan { gain, position, snap, zones, spread, decorrelation }));
    }
    r.byte_align();
    let description = description(&mut r)?;
    let n = plex(&mut r, 8)?;
    let mut o = Object { meta_id, audio_id, use_case, pan, description, objects: Vec::new(), zones19: None };
    for (id, c) in children(p.get(r.byte_pos()..).unwrap_or_default(), n)? {
        match id {
            element::OBJECT_DEFINITION => o.objects.push(object(c, ctx, depth + 1)?),
            element::OBJECT_ZONE_DEFINITION19 => o.zones19 = Some(zones19(c, ctx)?),
            _ => {}
        }
    }
    Ok(o)
}

fn zones19(p: &[u8], ctx: &Ctx) -> Result<ObjectZones19> {
    let mut r = BitReader::new(p);
    let mut blocks = Vec::new();
    for sb in 0..ctx.sub_blocks() {
        if sb > 0 && !r.read_bit()? {
            blocks.push(None);
            continue;
        }
        let mut z = [ZoneGain::Unity; 19];
        for g in &mut z {
            *g = zone_gain(&mut r)?;
        }
        blocks.push(Some(z));
    }
    Ok(ObjectZones19 { blocks })
}
