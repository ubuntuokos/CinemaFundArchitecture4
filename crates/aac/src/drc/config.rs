//! Static MPEG-D DRC payloads: `uniDrcConfig()` and `loudnessInfoSet()` (ISO/IEC 23003-4 §7.3),
//! versions 0 and 1.
//!
//! Ported from libxaac `impd_drc_static_payload.c` / `impd_drc_dynamic_payload.c` (Apache-2.0, see
//! `super` for the notice).

use cfa4_bitstream::BitReader;

use crate::{Error, Result};

/// `drcSetEffect` bits.
pub const EFFECT_NIGHT: u16 = 1 << 0;
pub const EFFECT_NOISY: u16 = 1 << 1;
pub const EFFECT_LIMITED: u16 = 1 << 2;
pub const EFFECT_LOWLEVEL: u16 = 1 << 3;
pub const EFFECT_DIALOG: u16 = 1 << 4;
pub const EFFECT_GENERAL: u16 = 1 << 5;
pub const EFFECT_EXPAND: u16 = 1 << 6;
pub const EFFECT_ARTISTIC: u16 = 1 << 7;
pub const EFFECT_CLIPPING: u16 = 1 << 8;
pub const EFFECT_FADE: u16 = 1 << 9;
pub const EFFECT_DUCK_OTHER: u16 = 1 << 10;
pub const EFFECT_DUCK_SELF: u16 = 1 << 11;

/// `downmixId` meaning "any downmix".
pub const ID_FOR_ANY_DOWNMIX: u8 = 0x7F;

const MAX_CHANNELS: usize = 128;
const MAX_GAIN_SETS: usize = 64;
const MAX_BANDS: usize = 16;
const MAX_SEQUENCES: usize = 64;

pub const GAIN_CODING_PROFILE_REGULAR: u8 = 0;
pub const GAIN_CODING_PROFILE_FADING: u8 = 1;
pub const GAIN_CODING_PROFILE_CLIPPING: u8 = 2;
pub const GAIN_CODING_PROFILE_CONSTANT: u8 = 3;

/// `gainSetParams()`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GainSet {
    pub coding_profile: u8,
    /// `gainInterpolationType`: true = spline, false = linear.
    pub spline: bool,
    pub full_frame: bool,
    pub time_alignment: bool,
    /// `timeDeltaMin` in samples, when signalled.
    pub time_delta_min: Option<u32>,
    /// Gain sequence index of each DRC band.
    pub band_sequences: Vec<usize>,
}

impl GainSet {
    pub fn band_count(&self) -> usize {
        self.band_sequences.len().max(1)
    }
}

/// `drcCoefficientsUniDrc()`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Coefficients {
    pub version: u8,
    pub location: u8,
    pub frame_size: Option<u32>,
    pub gain_sequence_count: usize,
    pub gain_sets: Vec<GainSet>,
}

/// Per-channel-group gain modifiers (`gainModifiers()`, one DRC band only).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GainModifiers {
    pub attn_scaling: f32,
    pub ampl_scaling: f32,
    pub gain_offset_db: f32,
}

impl Default for GainModifiers {
    fn default() -> Self {
        GainModifiers { attn_scaling: 1.0, ampl_scaling: 1.0, gain_offset_db: 0.0 }
    }
}

/// `drcInstructionsUniDrc()`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Instructions {
    pub drc_set_id: i32,
    pub location: u8,
    pub downmix_ids: Vec<u8>,
    pub effect: u16,
    pub limiter_peak_target: Option<f32>,
    /// (`drcSetTargetLoudnessValueUpper`, `…Lower`), when present.
    pub target_loudness: Option<(i32, i32)>,
    pub depends_on: Option<i32>,
    pub no_independent_use: bool,
    pub requires_eq: bool,
    /// Gain set index (0-based, -1 = no DRC) of every channel.
    pub channel_gain_set: Vec<i32>,
    /// Channel group of every channel (-1 = not processed).
    pub channel_group: Vec<i32>,
    /// Gain set and modifiers of every channel group.
    pub groups: Vec<(usize, GainModifiers)>,
}

/// `channelLayout()` + `uniDrcConfig()`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DrcConfig {
    pub sample_rate: Option<u32>,
    pub base_channel_count: usize,
    /// `downmixId` and target channel count of every `downmixInstructions()`.
    pub downmixes: Vec<(u8, usize)>,
    pub coefficients: Vec<Coefficients>,
    pub instructions: Vec<Instructions>,
}

impl DrcConfig {
    /// Parse `uniDrcConfig()`.
    pub fn parse(data: &[u8]) -> Result<DrcConfig> {
        let mut br = BitReader::new(data);
        let mut c = DrcConfig::default();
        if br.read_flag()? {
            c.sample_rate = Some(br.read_bits(18)? + 1000);
        }
        let dmx_count = br.read_bits(7)? as usize;
        let basic_present = br.read_flag()?;
        let (basic_coef, basic_instr) = if basic_present { (br.read_bits(3)? as usize, br.read_bits(4)? as usize) } else { (0, 0) };
        let coef_count = br.read_bits(3)? as usize;
        let instr_count = br.read_bits(6)? as usize;
        // channelLayout()
        c.base_channel_count = br.read_bits(7)? as usize;
        if c.base_channel_count == 0 || c.base_channel_count > MAX_CHANNELS {
            return Err(Error::Bitstream("uniDrcConfig: bad baseChannelCount"));
        }
        if br.read_flag()? && br.read_bits(8)? == 0 {
            br.skip(7 * c.base_channel_count)?; // speakerPosition
        }
        for _ in 0..dmx_count {
            c.downmixes.push(downmix_instructions(&mut br, 0, c.base_channel_count)?);
        }
        br.skip(11 * basic_coef)?;
        for _ in 0..basic_instr {
            instructions_basic(&mut br)?;
        }
        for _ in 0..coef_count {
            c.coefficients.push(coefficients(&mut br, 0)?);
        }
        for _ in 0..instr_count {
            let i = instructions(&mut br, 0, &c)?;
            c.instructions.push(i);
        }
        if br.read_flag()? {
            config_extension(&mut br, &mut c)?;
        }
        Ok(c)
    }

    /// The coefficients for `location`, preferring version 1 (`impd_sel_drc_coeff`).
    pub fn coefficients_for(&self, location: u8) -> Option<&Coefficients> {
        let mut best: Option<&Coefficients> = None;
        for co in self.coefficients.iter().filter(|co| co.location == location) {
            if co.version == 1 || best.is_none_or(|b| b.version == 0) {
                best = Some(co);
            }
        }
        best
    }
}

fn downmix_instructions(br: &mut BitReader, version: u8, base_channels: usize) -> Result<(u8, usize)> {
    let id = br.read_bits(7)? as u8;
    let target_channels = br.read_bits(7)? as usize;
    br.skip(8)?; // targetLayout
    if br.read_flag()? {
        if version == 0 {
            br.skip(4 * target_channels * base_channels)?;
        } else {
            br.skip(4 + 5 * target_channels * base_channels)?;
        }
    }
    Ok((id, target_channels))
}

fn instructions_basic(br: &mut BitReader) -> Result<()> {
    br.skip(6 + 4)?; // drcSetId, drcLocation
    br.skip(7)?; // downmixId
    if br.read_flag()? {
        let n = br.read_bits(3)? as usize;
        br.skip(7 * n)?;
    }
    let effect = br.read_bits(16)? as u16;
    if effect & (EFFECT_DUCK_OTHER | EFFECT_DUCK_SELF) == 0 {
        if br.read_flag()? {
            br.skip(8)?;
        }
    } else {
        // dimensionless duckingScaling (bsDuckingScaling)
        if br.read_flag()? {
            br.skip(4)?;
        }
    }
    if br.read_flag()? {
        br.skip(6)?;
        if br.read_flag()? {
            br.skip(6)?;
        }
    }
    if br.read_flag()? {
        br.skip(6)?;
    } else {
        br.skip(1)?;
    }
    Ok(())
}

fn gain_set_params(br: &mut BitReader, version: u8, seq: &mut i32) -> Result<GainSet> {
    let coding_profile = br.read_bits(2)? as u8;
    let spline = !br.read_flag()?; // gainInterpolationType 0 = spline
    let full_frame = br.read_flag()?;
    let time_alignment = br.read_flag()?;
    let time_delta_min = if br.read_flag()? { Some(br.read_bits(11)? + 1) } else { None };
    let mut g = GainSet { coding_profile, spline, full_frame, time_alignment, time_delta_min, band_sequences: Vec::new() };
    if coding_profile == GAIN_CODING_PROFILE_CONSTANT {
        *seq += 1;
        g.band_sequences.push(seq_index(*seq)?);
        return Ok(g);
    }
    let bands = br.read_bits(4)? as usize;
    if bands > MAX_BANDS {
        return Err(Error::Bitstream("uniDrcConfig: too many DRC bands"));
    }
    let band_type = if bands > 1 { br.read_flag()? } else { false };
    for _ in 0..bands {
        if version == 0 {
            *seq += 1;
        } else if br.read_flag()? {
            *seq = br.read_bits(6)? as i32;
        } else {
            *seq += 1;
        }
        g.band_sequences.push(seq_index(*seq)?);
        // gain set characteristics
        if version == 0 {
            br.skip(7)?;
        } else if br.read_flag()? {
            if br.read_flag()? {
                br.skip(7)?;
            } else {
                br.skip(8)?;
            }
        }
    }
    let crossover_bits = if band_type { 4 } else { 10 };
    br.skip(crossover_bits * bands.saturating_sub(1))?;
    Ok(g)
}

fn seq_index(seq: i32) -> Result<usize> {
    usize::try_from(seq).ok().filter(|&s| s < MAX_SEQUENCES).ok_or(Error::Bitstream("uniDrcConfig: bad gain sequence index"))
}

fn split_characteristic(br: &mut BitReader) -> Result<()> {
    if !br.read_flag()? {
        br.skip(6 + 8 + 1)?; // sigmoid: gain, ioRatio + exp, flipSign
    } else {
        let nodes = br.read_bits(2)? as usize + 1;
        br.skip(nodes * (5 + 8))?;
    }
    Ok(())
}

fn coefficients(br: &mut BitReader, version: u8) -> Result<Coefficients> {
    let mut c = Coefficients { version, location: br.read_bits(4)? as u8, ..Default::default() };
    if br.read_flag()? {
        c.frame_size = Some(br.read_bits(15)? + 1);
    }
    let mut seq = -1i32;
    if version == 0 {
        let n = br.read_bits(6)? as usize;
        if n > MAX_GAIN_SETS {
            return Err(Error::Bitstream("uniDrcConfig: too many gain sets"));
        }
        for _ in 0..n {
            c.gain_sets.push(gain_set_params(br, 0, &mut seq)?);
        }
        c.gain_sequence_count = c.gain_sets.iter().map(GainSet::band_count).sum();
    } else {
        for _ in 0..2 {
            // left / right split characteristics
            if br.read_flag()? {
                let n = br.read_bits(4)?;
                for _ in 0..n {
                    split_characteristic(br)?;
                }
            }
        }
        if br.read_flag()? {
            let n = br.read_bits(4)?;
            for _ in 0..n {
                for _ in 0..4 {
                    // LF cut, LF boost, HF cut, HF boost
                    if br.read_flag()? {
                        br.skip(5)?;
                    }
                }
            }
        }
        c.gain_sequence_count = br.read_bits(6)? as usize;
        let n = br.read_bits(6)? as usize;
        for _ in 0..n {
            c.gain_sets.push(gain_set_params(br, 1, &mut seq)?);
        }
    }
    Ok(c)
}

fn gain_modifiers(br: &mut BitReader, version: u8, bands: usize) -> Result<GainModifiers> {
    let mut m = GainModifiers::default();
    let read_scaling_offset = |br: &mut BitReader, m: &mut GainModifiers| -> Result<()> {
        if br.read_flag()? {
            m.attn_scaling = br.read_bits(4)? as f32 * 0.125;
            m.ampl_scaling = br.read_bits(4)? as f32 * 0.125;
        }
        if br.read_flag()? {
            let sign = br.read_flag()?;
            let v = (1 + br.read_bits(5)?) as f32 * 0.25;
            m.gain_offset_db = if sign { -v } else { v };
        }
        Ok(())
    };
    if version == 0 {
        read_scaling_offset(br, &mut m)?;
        return Ok(m);
    }
    for b in 0..bands {
        let mut mb = GainModifiers::default();
        for _ in 0..2 {
            // targetCharacteristicLeft/RightPresent
            if br.read_flag()? {
                br.skip(4)?;
            }
        }
        read_scaling_offset(br, &mut mb)?;
        if b == 0 {
            m = mb;
        }
    }
    if bands == 1 && br.read_flag()? {
        br.skip(4)?; // shapeFilterIndex
    }
    Ok(m)
}

fn instructions(br: &mut BitReader, version: u8, cfg: &DrcConfig) -> Result<Instructions> {
    let mut ins = Instructions { drc_set_id: br.read_bits(6)? as i32, ..Default::default() };
    if version == 1 {
        br.skip(4)?; // drcSetComplexityLevel
    }
    ins.location = br.read_bits(4)? as u8;
    let dmx_present = if version == 1 { br.read_flag()? } else { true };
    let mut apply_to_downmix = false;
    if dmx_present {
        ins.downmix_ids.push(br.read_bits(7)? as u8);
        apply_to_downmix = if version == 1 { br.read_flag()? } else { ins.downmix_ids.first() != Some(&0) };
        if br.read_flag()? {
            let n = br.read_bits(3)?;
            for _ in 0..n {
                ins.downmix_ids.push(br.read_bits(7)? as u8);
            }
        }
    } else {
        ins.downmix_ids.push(0);
    }
    ins.effect = br.read_bits(16)? as u16;
    let ducking = ins.effect & (EFFECT_DUCK_OTHER | EFFECT_DUCK_SELF) != 0;
    if !ducking && br.read_flag()? {
        ins.limiter_peak_target = Some(-(br.read_bits(8)? as f32) * 0.125);
    }
    if br.read_flag()? {
        let upper = br.read_bits(6)? as i32 - 63;
        let lower = if br.read_flag()? { br.read_bits(6)? as i32 - 63 } else { -63 };
        ins.target_loudness = Some((upper, lower));
    }
    if br.read_flag()? {
        ins.depends_on = Some(br.read_bits(6)? as i32);
    } else {
        ins.no_independent_use = br.read_flag()?;
    }
    if version == 1 {
        ins.requires_eq = br.read_flag()?;
    }

    let coef = cfg.coefficients_for(ins.location);
    let mut channels = cfg.base_channel_count;
    if ducking {
        let mut c = 0;
        while c < channels {
            let idx = br.read_bits(6)? as i32 - 1;
            if br.read_flag()? {
                br.skip(4)?; // duckingScaling
            }
            ins.channel_gain_set.push(idx);
            c += 1;
            if br.read_flag()? {
                let n = br.read_bits(5)? + 1;
                for _ in 0..n {
                    ins.channel_gain_set.push(idx);
                    c += 1;
                }
            }
        }
        // Ducking sets are never selected by this decoder (no ducking requests): keep the
        // parse, leave the groups empty.
        ins.channel_group = vec![-1; ins.channel_gain_set.len()];
        return Ok(ins);
    }
    let any = ins.downmix_ids.len() > 1 || ins.downmix_ids.first() == Some(&ID_FOR_ANY_DOWNMIX);
    if (version == 0 || apply_to_downmix) && ins.downmix_ids.first().is_some_and(|&d| d != 0) {
        if any {
            channels = 1;
        } else {
            let id = ins.downmix_ids.first().copied().unwrap_or(0);
            channels = cfg.downmixes.iter().find(|d| d.0 == id).map(|d| d.1).ok_or(Error::Bitstream("uniDrcConfig: unknown downmixId"))?;
        }
    }
    let mut c = 0;
    while c < channels {
        let idx = br.read_bits(6)? as i32 - 1;
        let repeat = br.read_flag()?;
        ins.channel_gain_set.push(idx);
        c += 1;
        if repeat {
            let n = br.read_bits(5)? + 1;
            for _ in 0..n {
                ins.channel_gain_set.push(idx);
                c += 1;
            }
        }
        if c > MAX_CHANNELS {
            return Err(Error::Bitstream("uniDrcConfig: too many channels"));
        }
    }
    let mut unique: Vec<usize> = Vec::new();
    for &idx in &ins.channel_gain_set {
        let g = match usize::try_from(idx) {
            Ok(i) => match unique.iter().position(|&u| u == i) {
                Some(p) => p as i32,
                None => {
                    unique.push(i);
                    unique.len() as i32 - 1
                }
            },
            Err(_) => -1,
        };
        ins.channel_group.push(g);
    }
    for &set in &unique {
        let bands = coef.and_then(|co| co.gain_sets.get(set)).map_or(1, GainSet::band_count);
        let m = gain_modifiers(br, version, bands)?;
        ins.groups.push((set, m));
    }
    Ok(ins)
}

fn config_extension(br: &mut BitReader, c: &mut DrcConfig) -> Result<()> {
    loop {
        let t = br.read_bits(4)?;
        if t == 0 {
            return Ok(());
        }
        let size_bits = br.read_bits(4)? + 4;
        let size = br.read_bits(size_bits)? as usize + 1;
        let start = br.position();
        if size > br.bits_left() {
            return Err(Error::Bitstream("uniDrcConfigExtension exceeds the payload"));
        }
        if t == 2 {
            // UNIDRCCONFEXT_V1; a part we cannot parse only drops what follows it in this extension
            let _ = config_v1(br, c);
        }
        br.seek_bits(start + size);
    }
}

fn config_v1(br: &mut BitReader, c: &mut DrcConfig) -> Result<()> {
    if br.read_flag()? {
        let n = br.read_bits(7)?;
        for _ in 0..n {
            let d = downmix_instructions(br, 1, c.base_channel_count)?;
            c.downmixes.push(d);
        }
    }
    if br.read_flag()? {
        let n = br.read_bits(3)?;
        for _ in 0..n {
            let co = coefficients(br, 1)?;
            c.coefficients.push(co);
        }
        let n = br.read_bits(6)?;
        for _ in 0..n {
            let i = instructions(br, 1, c)?;
            c.instructions.push(i);
        }
    }
    // loudEqInstructions / eqCoefficients / eqInstructions are not used
    Ok(())
}

/// One `loudnessInfo()` entry.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoudnessInfo {
    pub drc_set_id: i32,
    pub eq_set_id: i32,
    pub downmix_id: u8,
    pub sample_peak: Option<f32>,
    pub true_peak: Option<f32>,
    /// (`methodDefinition`, value, `measurementSystem`, reliability).
    pub measurements: Vec<(u8, f32, u8, u8)>,
}

/// `loudnessInfoSet()`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoudnessInfoSet {
    pub album: Vec<LoudnessInfo>,
    pub track: Vec<LoudnessInfo>,
}

pub const METHOD_PROGRAM_LOUDNESS: u8 = 1;
pub const METHOD_ANCHOR_LOUDNESS: u8 = 2;

impl LoudnessInfoSet {
    pub fn parse(data: &[u8]) -> Result<LoudnessInfoSet> {
        let mut br = BitReader::new(data);
        let mut s = LoudnessInfoSet::default();
        s.read_counts(&mut br, 0)?;
        if br.read_flag()? {
            loop {
                let t = br.read_bits(4)?;
                if t == 0 {
                    break;
                }
                let size_bits = br.read_bits(4)? + 4;
                let size = br.read_bits(size_bits)? as usize + 1;
                let start = br.position();
                if size > br.bits_left() {
                    return Err(Error::Bitstream("loudnessInfoSetExtension exceeds the payload"));
                }
                if t == 1 {
                    s.read_counts(&mut br, 1)?;
                }
                br.seek_bits(start + size);
            }
        }
        Ok(s)
    }

    fn read_counts(&mut self, br: &mut BitReader, version: u8) -> Result<()> {
        let album = br.read_bits(6)?;
        let track = br.read_bits(6)?;
        for _ in 0..album {
            self.album.push(loudness_info(br, version)?);
        }
        for _ in 0..track {
            self.track.push(loudness_info(br, version)?);
        }
        Ok(())
    }
}

fn loudness_info(br: &mut BitReader, version: u8) -> Result<LoudnessInfo> {
    let mut l = LoudnessInfo { drc_set_id: br.read_bits(6)? as i32, ..Default::default() };
    if version >= 1 {
        l.eq_set_id = br.read_bits(6)? as i32;
    }
    l.downmix_id = br.read_bits(7)? as u8;
    if br.read_flag()? {
        let v = br.read_bits(12)?;
        if v != 0 {
            l.sample_peak = Some(20.0 - v as f32 * 0.03125);
        }
    }
    if br.read_flag()? {
        let v = br.read_bits(12)?;
        br.skip(6)?; // measurement system, reliability
        if v != 0 {
            l.true_peak = Some(20.0 - v as f32 * 0.03125);
        }
    }
    let n = br.read_bits(4)?;
    for _ in 0..n {
        let method = br.read_bits(4)? as u8;
        let value = match method {
            0..=2 | 4..=6 => -57.75 + br.read_bits(8)? as f32 * 0.25,
            3 => {
                let t = br.read_bits(8)?;
                match t {
                    0 => 0.0,
                    1..=128 => t as f32 * 0.25,
                    129..=204 => 0.5 * t as f32 - 32.0,
                    _ => t as f32 - 134.0,
                }
            }
            7 => br.read_bits(5)? as f32 + 80.0,
            8 => br.read_bits(2)? as f32,
            9 => -116.0 + br.read_bits(8)? as f32 * 0.5,
            _ => return Err(Error::Bitstream("loudnessInfo: reserved methodDefinition")),
        };
        let system = br.read_bits(4)? as u8;
        let reliability = br.read_bits(2)? as u8;
        l.measurements.push((method, value, system, reliability));
    }
    Ok(l)
}
