//! Dynamic MPEG-D DRC payload `uniDrcGain()` (ISO/IEC 23003-4 §7.4) and the time-domain gain
//! interpolation (§6.4).
//!
//! Ported from libxaac `impd_drc_dynamic_payload.c`, `impd_drc_extr_delta_coded_info.c` and
//! `impd_drc_gain_dec.c` (Apache-2.0, see `super` for the notice).

use cfa4_bitstream::BitReader;

use super::config::{
    Coefficients, GAIN_CODING_PROFILE_CLIPPING, GAIN_CODING_PROFILE_CONSTANT, GAIN_CODING_PROFILE_FADING, GAIN_CODING_PROFILE_REGULAR, GainModifiers, GainSet,
};
use crate::{Error, Result};

/// Largest supported DRC frame (`AUDIO_CODEC_FRAME_SIZE_MAX`).
pub const MAX_FRAME: usize = 4096;
const MAX_NODES: usize = MAX_FRAME / 8;
const SLOPE_FACTOR_DB_TO_LINEAR: f32 = 0.1151;

/// One spline node: time (samples, relative to the DRC frame), gain (dB), slope (dB per
/// `deltaTmin`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Node {
    pub time: i32,
    pub gain_db: f32,
    pub slope: f32,
}

/// (code length, code, delta gain in dB), profiles 0 and 1.
const DELTA_GAIN_PROF_0_1: [(u8, u16, f32); 25] = [
    (2, 0x003, -0.125),
    (2, 0x002, 0.125),
    (3, 0x001, -0.250),
    (3, 0x002, 0.000),
    (4, 0x000, -2.000),
    (5, 0x002, -0.500),
    (5, 0x00F, -0.375),
    (5, 0x00E, 1.000),
    (6, 0x019, -0.625),
    (6, 0x018, 0.250),
    (6, 0x006, 0.375),
    (7, 0x00F, -1.000),
    (7, 0x034, -0.875),
    (7, 0x036, -0.750),
    (7, 0x037, 0.500),
    (8, 0x01D, 0.625),
    (9, 0x039, -1.875),
    (9, 0x0D5, -1.125),
    (9, 0x0D7, 0.750),
    (9, 0x0D4, 0.875),
    (10, 0x070, -1.500),
    (10, 0x1AC, -1.375),
    (10, 0x1AD, -1.250),
    (11, 0x0E2, -1.750),
    (11, 0x0E3, -1.625),
];

/// Profile 2 (clipping).
const DELTA_GAIN_PROF_2: [(u8, u16, f32); 49] = [
    (3, 0x007, -0.125),
    (4, 0x00C, -0.625),
    (4, 0x009, -0.500),
    (4, 0x005, -0.375),
    (4, 0x003, -0.250),
    (4, 0x001, 0.000),
    (4, 0x00B, 0.125),
    (5, 0x011, -0.875),
    (5, 0x00E, -0.750),
    (5, 0x005, 0.250),
    (5, 0x004, 0.375),
    (5, 0x008, 0.500),
    (5, 0x000, 0.625),
    (5, 0x00D, 0.750),
    (5, 0x00F, 0.875),
    (5, 0x010, 1.000),
    (5, 0x01B, 1.125),
    (6, 0x02B, -1.250),
    (6, 0x028, -1.125),
    (6, 0x002, -1.000),
    (6, 0x012, 1.250),
    (6, 0x018, 1.375),
    (6, 0x029, 1.500),
    (7, 0x06A, -4.000),
    (7, 0x054, -1.750),
    (7, 0x068, -1.625),
    (7, 0x026, -1.500),
    (7, 0x006, -1.375),
    (7, 0x032, 1.625),
    (8, 0x0D2, -2.250),
    (8, 0x0AB, -2.125),
    (8, 0x0AA, -2.000),
    (8, 0x04F, -1.875),
    (8, 0x04E, 1.750),
    (8, 0x0D7, 1.875),
    (8, 0x00E, 2.000),
    (9, 0x1AD, -3.625),
    (9, 0x1AC, -3.375),
    (9, 0x1A6, -3.250),
    (9, 0x0CD, -3.125),
    (9, 0x0CE, -2.750),
    (9, 0x1A7, -2.625),
    (9, 0x01F, -2.500),
    (9, 0x0CC, -2.375),
    (10, 0x03C, -3.500),
    (10, 0x19E, -3.000),
    (10, 0x19F, -2.875),
    (11, 0x07A, -3.875),
    (11, 0x07B, -3.750),
];

/// (code length, code, slope).
const SLOPE_CODES: [(u8, u16, f32); 15] = [
    (1, 0x001, 0.0),
    (2, 0x000, -0.005),
    (4, 0x007, 0.005),
    (5, 0x00A, -0.1953),
    (5, 0x009, -0.0781),
    (5, 0x00D, -0.0312),
    (5, 0x00B, 0.0312),
    (6, 0x018, -3.0518),
    (6, 0x011, 0.0781),
    (7, 0x032, -0.4883),
    (7, 0x020, 1.2207),
    (7, 0x033, 3.0518),
    (8, 0x042, -1.2207),
    (9, 0x087, 0.1953),
    (9, 0x086, 0.4883),
];

/// Decode a prefix code from a table sorted by code length.
fn read_code(br: &mut BitReader, table: &[(u8, u16, f32)]) -> Result<f32> {
    let (mut code, mut len) = (0u32, 0u8);
    for &(size, c, v) in table {
        while len < size {
            code = (code << 1) | br.read_bits(1)?;
            len += 1;
        }
        if code == c as u32 {
            return Ok(v);
        }
    }
    Err(Error::Bitstream("uniDrcGain: invalid code"))
}

/// `deltaTmin` default for a sampling rate (`impd_get_delta_tmin`).
pub fn delta_tmin_default(sample_rate: u32) -> u32 {
    let lower = (0.5 + 0.0005 * sample_rate as f32) as u32;
    let mut r = 1u32;
    while r <= lower && r < 1 << 16 {
        r <<= 1;
    }
    r
}

/// Decoding parameters of one gain set.
#[derive(Debug, Clone, Copy)]
pub struct TimeParams {
    pub frame_size: usize,
    pub delta_tmin: u32,
}

fn read_time_delta(br: &mut BitReader, max_values: u32) -> Result<u32> {
    let mut z = 1u32;
    while (1u32 << z) < 2 * max_values && z < 16 {
        z += 1;
    }
    Ok(match br.read_bits(2)? {
        0 => 1,
        1 => 2 + br.read_bits(2)?,
        2 => 6 + br.read_bits(3)?,
        _ => {
            let v = 14 + br.read_bits(z)?;
            if v > 2 * max_values {
                return Err(Error::Bitstream("uniDrcGain: time delta out of range"));
            }
            v
        }
    })
}

fn initial_gain(br: &mut BitReader, profile: u8) -> Result<f32> {
    Ok(match profile {
        GAIN_CODING_PROFILE_REGULAR => {
            let sign = br.read_flag()?;
            let g = br.read_bits(8)? as f32 * 0.125;
            if sign { -g } else { g }
        }
        GAIN_CODING_PROFILE_FADING | GAIN_CODING_PROFILE_CLIPPING => {
            let nonzero = br.read_flag()?;
            let bits = if profile == GAIN_CODING_PROFILE_FADING { 10 } else { 8 };
            if nonzero { -((br.read_bits(bits)? + 1) as f32) * 0.125 } else { 0.0 }
        }
        _ => 0.0,
    })
}

/// `drcGainSequence()` → nodes, with the node-reservoir reordering of libxaac.
fn gain_sequence(br: &mut BitReader, gs: &GainSet, tp: TimeParams) -> Result<Vec<Node>> {
    let n = tp.frame_size as i32;
    let delta_tmin = gs.time_delta_min.unwrap_or(tp.delta_tmin).max(1);
    let time_offset = if !gs.time_alignment { -1 } else { -(delta_tmin as i32) + (delta_tmin as i32 - 1) / 2 };
    let mut nodes = Vec::new();
    if !br.read_flag()? {
        // drcGainCodingMode 0: one node at the end of the frame
        let g = initial_gain(br, gs.coding_profile)?;
        nodes.push(Node { time: n + time_offset, gain_db: g, slope: 0.0 });
        return Ok(nodes);
    }
    // slopes (end markers give the node count)
    let mut count = 0usize;
    loop {
        count += 1;
        if br.read_flag()? {
            break;
        }
        if count >= MAX_NODES {
            return Err(Error::Bitstream("uniDrcGain: too many nodes"));
        }
    }
    nodes.resize(count, Node::default());
    if gs.spline {
        for nd in &mut nodes {
            nd.slope = read_code(br, &SLOPE_CODES)?;
        }
    }
    // times
    let max_values = (tp.frame_size as u32 / delta_tmin).max(1);
    let limit = 2 * MAX_FRAME as i32 - n;
    let frame_end = gs.full_frame || br.read_flag()?; // frameEndFlag
    if frame_end {
        let mut t = time_offset;
        let mut reserved = false;
        let mut k = 0;
        while k + 1 < count {
            let d = read_time_delta(br, max_values)?;
            let tt = t.saturating_add((d * delta_tmin) as i32);
            if tt >= limit {
                return Err(Error::Bitstream("uniDrcGain: node time out of range"));
            }
            if tt > n + time_offset {
                if !reserved {
                    if let Some(nd) = nodes.get_mut(k) {
                        nd.time = n + time_offset;
                    }
                    reserved = true;
                }
                if let Some(nd) = nodes.get_mut(k + 1) {
                    nd.time = tt;
                }
            } else if let Some(nd) = nodes.get_mut(k) {
                nd.time = tt;
            }
            t = tt;
            k += 1;
        }
        if !reserved && let Some(nd) = nodes.get_mut(k) {
            nd.time = n + time_offset;
        }
    } else {
        let mut t = time_offset;
        for nd in &mut nodes {
            let d = read_time_delta(br, max_values)?;
            t = t.saturating_add((d * delta_tmin) as i32);
            if t >= limit {
                return Err(Error::Bitstream("uniDrcGain: node time out of range"));
            }
            nd.time = t;
        }
    }
    // gains
    let table: &[(u8, u16, f32)] = if gs.coding_profile == GAIN_CODING_PROFILE_CLIPPING { &DELTA_GAIN_PROF_2 } else { &DELTA_GAIN_PROF_0_1 };
    let mut g = initial_gain(br, gs.coding_profile)?;
    for (k, nd) in nodes.iter_mut().enumerate() {
        if k > 0 {
            g += read_code(br, table)?;
        }
        nd.gain_db = g;
    }
    // nodes beyond the frame belong to the node reservoir: move them first (libxaac)
    let (mut later, cur): (Vec<Node>, Vec<Node>) = nodes.into_iter().partition(|nd| nd.time >= n);
    for nd in &mut later {
        nd.time -= 2 * n;
    }
    later.extend(cur);
    Ok(later)
}

/// Parse `uniDrcGain()`: the nodes of every gain sequence of `coef`.
pub fn parse_gain(data: &[u8], coef: &Coefficients, tp: TimeParams) -> Result<Vec<Vec<Node>>> {
    let mut br = BitReader::new(data);
    let mut set_of_seq = vec![None; coef.gain_sequence_count];
    for (i, gs) in coef.gain_sets.iter().enumerate() {
        for &s in &gs.band_sequences {
            if let Some(slot) = set_of_seq.get_mut(s) {
                *slot = Some(i);
            }
        }
    }
    let default_set = GainSet::default();
    let mut out = Vec::with_capacity(set_of_seq.len());
    for idx in set_of_seq {
        let gs = idx.and_then(|i| coef.gain_sets.get(i)).unwrap_or(&default_set);
        if gs.coding_profile == GAIN_CODING_PROFILE_CONSTANT {
            out.push(vec![Node { time: tp.frame_size as i32 - 1, gain_db: 0.0, slope: 0.0 }]);
        } else {
            out.push(gain_sequence(&mut br, gs, tp)?);
        }
    }
    // uniDrcGainExtension() is skipped
    Ok(out)
}

/// How the dB gains of one channel group are mapped to linear gains.
#[derive(Debug, Clone, Copy)]
pub struct GainMapping {
    pub modifiers: GainModifiers,
    /// Apply the compress/boost scaling (not for ducking, fading and clipping sets).
    pub modification: bool,
    pub compress: f32,
    pub boost: f32,
    /// `limiterPeakTarget` of a clipping set, with the loudness normalization gain.
    pub clipping: Option<(f32, f32)>,
    pub spline: bool,
    pub delta_tmin: u32,
}

impl GainMapping {
    fn db_to_linear(&self, db: f32, slope_db: f32) -> (f32, f32) {
        let mut ratio = 1.0;
        if self.modification {
            ratio *= if db < 0.0 { self.compress } else { self.boost };
        }
        ratio *= if db < 0.0 { self.modifiers.attn_scaling } else { self.modifiers.ampl_scaling };
        let mut g = (ratio * db / 6.0).exp2();
        let mut s = SLOPE_FACTOR_DB_TO_LINEAR * ratio * g * slope_db;
        if self.modifiers.gain_offset_db != 0.0 {
            g *= (self.modifiers.gain_offset_db / 6.0).exp2();
        }
        if let Some((peak_target, ln_gain)) = self.clipping {
            g *= ((-peak_target - ln_gain).max(0.0) / 6.0).exp2();
            if g >= 1.0 {
                g = 1.0;
                s = 0.0;
            }
        }
        (g, s)
    }

    /// `impd_interpolate_drc_gain`: write `out[0..=step]`.
    fn interpolate(&self, step: usize, g0: f32, g1: f32, s0: f32, s1: f32, out: &mut [f32]) {
        let (gain1, slope1) = self.db_to_linear(g0, s0);
        let (gain2, slope2) = self.db_to_linear(g1, s1);
        if step == 0 || out.len() <= step {
            return;
        }
        out[0] = gain1;
        out[step] = gain2;
        let stepf = step as f32;
        if !self.spline {
            let a = (gain2 - gain1) / stepf;
            for (n, o) in out.iter_mut().enumerate().take(step).skip(1) {
                *o = a * n as f32 + gain1;
            }
            return;
        }
        let dt = self.delta_tmin.max(1) as f32;
        let (st1, st2) = (slope1 / dt, slope2 / dt);
        if st1.abs() > st2.abs() {
            let ins_f = 2.0 * (gain2 - gain1 - st2 * stepf) / (st1 - st2);
            let ins = (0.5 + ins_f) as i64;
            if ins >= 0 && (ins as usize) < step {
                let ins = ins as usize;
                let a = 0.5 * (st2 - st1) / ins_f;
                for (n, o) in out.iter_mut().enumerate().take(ins).skip(1) {
                    let t = n as f32;
                    *o = ((a * t + st1) * t + gain1).max(0.0);
                }
                for (n, o) in out.iter_mut().enumerate().take(step).skip(ins.max(1)) {
                    let t = n as f32 - stepf;
                    *o = st2 * t + gain2;
                }
                return;
            }
        } else if st1.abs() < st2.abs() {
            let ins_f = stepf - 2.0 * (gain1 - gain2 + st1 * stepf) / (st1 - st2);
            let ins = (0.5 + ins_f) as i64;
            if ins >= 0 && (ins as usize) < step {
                let ins = ins as usize;
                for (n, o) in out.iter_mut().enumerate().take(ins).skip(1) {
                    *o = st1 * n as f32 + gain1;
                }
                let a = (st2 - st1) / (2.0 * (stepf - ins_f));
                for (n, o) in out.iter_mut().enumerate().take(step).skip(ins.max(1)) {
                    let t = stepf - n as f32;
                    *o = ((a * t - st2) * t + gain2).max(0.0);
                }
                return;
            }
        }
        // cubic
        let inv = 1.0 / stepf;
        let k1 = (gain2 - gain1) * inv * inv;
        let k2 = st2 + st1;
        let a = inv * (inv * k2 - 2.0 * k1);
        let b = 3.0 * k1 - inv * (k2 + st1);
        for (n, o) in out.iter_mut().enumerate().take(step).skip(1) {
            let t = n as f32;
            *o = ((((a * t + b) * t + st1) * t) + gain1).max(0.0);
        }
    }
}

/// Room before the current frame in [`GainBuffer`] for segments that start in earlier frames.
const HISTORY: usize = 2 * MAX_FRAME;

/// Per gain element interpolation state (`ia_interp_buf_struct`): linear gains for the previous
/// and the current DRC frame.
#[derive(Debug, Clone)]
pub struct GainBuffer {
    frame: usize,
    prev: Node,
    /// `gains[HISTORY..HISTORY + frame]` apply to the frame being output, the following ones to
    /// the next frame.
    gains: Vec<f32>,
}

impl GainBuffer {
    pub fn new(frame: usize) -> GainBuffer {
        GainBuffer { frame, prev: Node { time: -1, gain_db: 0.0, slope: 0.0 }, gains: vec![1.0; HISTORY + 2 * frame + 2 * MAX_FRAME] }
    }

    /// Shift by one frame (`impd_advance_buf`), then add the segments of `nodes`
    /// (`impd_concatenate_segments`).
    pub fn push(&mut self, nodes: &[Node], map: &GainMapping) {
        let n = self.frame;
        self.gains.copy_within(n.., 0);
        self.prev.time -= n as i32;
        let (mut t_prev, mut g_prev, mut s_prev) = (self.prev.time, self.prev.gain_db, self.prev.slope);
        let mut last = self.prev;
        for nd in nodes {
            let start = (HISTORY + n) as i64 + t_prev as i64;
            let dur = nd.time as i64 - t_prev as i64;
            if start >= 0
                && dur > 0
                && let Some(out) = self.gains.get_mut(start as usize..)
            {
                map.interpolate(dur as usize, g_prev, nd.gain_db, s_prev, nd.slope, out);
            }
            t_prev = nd.time;
            g_prev = nd.gain_db;
            s_prev = nd.slope;
            last = *nd;
        }
        self.prev = last;
    }

    /// Gain of the last node received, in dB.
    pub fn last_gain_db(&self) -> f32 {
        self.prev.gain_db
    }

    /// Linear gains of the frame being output.
    pub fn current(&self) -> &[f32] {
        self.gains.get(HISTORY..HISTORY + self.frame).unwrap_or(&[])
    }
}
