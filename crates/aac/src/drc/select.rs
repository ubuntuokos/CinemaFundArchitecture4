//! DRC set selection and loudness normalization (ISO/IEC 23003-4 §6.3, Annex E), for the base
//! channel layout without EQ or downmix requests.
//!
//! Ported from libxaac `impd_drc_selection_process_drcset_selection.c` and
//! `impd_drc_loudness_control.c` (Apache-2.0, see `super` for the notice).

use super::config::{
    DrcConfig, EFFECT_ARTISTIC, EFFECT_CLIPPING, EFFECT_DIALOG, EFFECT_DUCK_OTHER, EFFECT_DUCK_SELF, EFFECT_EXPAND, EFFECT_FADE, EFFECT_GENERAL,
    EFFECT_LIMITED, EFFECT_LOWLEVEL, EFFECT_NIGHT, EFFECT_NOISY, ID_FOR_ANY_DOWNMIX, Instructions, LoudnessInfo, LoudnessInfoSet, METHOD_ANCHOR_LOUDNESS,
    METHOD_PROGRAM_LOUDNESS,
};

const ID_FOR_ANY_DRC: i32 = 0x3F;
const UNDEFINED_LOUDNESS: f32 = 1000.0;
/// Effect types the selection compares (`effect_types_request_table`).
const EFFECT_TYPES: [u16; 8] = [EFFECT_NIGHT, EFFECT_NOISY, EFFECT_LIMITED, EFFECT_LOWLEVEL, EFFECT_DIALOG, EFFECT_GENERAL, EFFECT_EXPAND, EFFECT_ARTISTIC];
/// Measurement system preference for BS.1770 (`measurement_system_bs1770_3_tbl`).
const SYSTEM_BONUS: [i32; 12] = [0, 0, 8, 0, 1, 3, 0, 5, 6, 7, 4, 2];

/// Decoder-side DRC and loudness settings (the "decoder interface" of ISO/IEC 23003-4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrcSettings {
    /// Target output loudness in LKFS; `None` turns loudness normalization off.
    pub target_loudness: Option<f32>,
    /// Apply a DRC set from the stream.
    pub drc: bool,
    /// Requested DRC effect (one of the `EFFECT_*` bits); 0 = the default selection.
    pub effect: u16,
    /// Scaling of attenuating / amplifying DRC gains (1.0 = as transmitted).
    pub compress: f32,
    pub boost: f32,
    /// Largest output peak (dBFS) the selection aims for.
    pub output_peak_level_max: f32,
    /// How far (dB) the normalization gain may be lowered to avoid clipping.
    pub loudness_deviation_max: f32,
    /// With loudness normalization, apply a 5 ms look-ahead peak limiter at -1 dBFS when the
    /// selected output can clip.
    pub peak_limiter: bool,
}

impl Default for DrcSettings {
    /// Normalization to -16 LKFS with the stream's DRC, as the Fraunhofer reference decoder
    /// outputs it.
    fn default() -> Self {
        DrcSettings {
            target_loudness: Some(-16.0),
            drc: true,
            effect: 0,
            compress: 1.0,
            boost: 1.0,
            // 6 dB of headroom are left to the peak limiter (ISO/IEC 23003-4 default)
            output_peak_level_max: 6.0,
            loudness_deviation_max: 63.0,
            peak_limiter: true,
        }
    }
}

/// Result of the selection process.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Selection {
    /// Index into [`DrcConfig::instructions`], `None` for no DRC.
    pub instructions: Option<usize>,
    pub loudness_normalization_db: f32,
    pub output_peak_level_db: f32,
    /// Output loudness, when known.
    pub output_loudness: Option<f32>,
}

#[derive(Debug, Clone, Copy)]
struct Candidate {
    ins: Option<usize>,
    set_id: i32,
    ln_gain: f32,
    peak: f32,
    loudness: f32,
    explicit_peak: bool,
    target_match: bool,
}

fn has_overall_loudness(l: &LoudnessInfo) -> bool {
    l.measurements.iter().any(|m| m.0 == METHOD_PROGRAM_LOUDNESS || m.0 == METHOD_ANCHOR_LOUDNESS)
}

/// `impd_check_loud_payload`: the loudness infos for a DRC set, with the standard fallbacks.
fn matching_loudness(infos: &[LoudnessInfo], set_id: i32) -> Vec<&LoudnessInfo> {
    let any = ID_FOR_ANY_DOWNMIX;
    let order: [(u8, i32); 9] =
        [(0, set_id), (any, set_id), (0, ID_FOR_ANY_DRC), (0, 0), (any, ID_FOR_ANY_DRC), (any, 0), (0, set_id), (0, ID_FOR_ANY_DRC), (0, 0)];
    for (dmx, id) in order {
        let v: Vec<&LoudnessInfo> = infos.iter().filter(|l| l.downmix_id == dmx && l.drc_set_id == id && has_overall_loudness(l)).collect();
        if !v.is_empty() {
            return v;
        }
    }
    Vec::new()
}

/// Program loudness of one loudness info (`impd_init_loudness_control`).
fn method_value(l: &LoudnessInfo) -> Option<f32> {
    for wanted in [METHOD_PROGRAM_LOUDNESS, METHOD_ANCHOR_LOUDNESS] {
        let mut best: Option<(i32, f32)> = None;
        for &(method, value, system, _) in &l.measurements {
            let bonus = SYSTEM_BONUS.get(system as usize).copied().unwrap_or(0);
            if method == wanted && best.is_none_or(|b| b.0 < bonus) {
                best = Some((bonus, value));
            }
        }
        if let Some(b) = best {
            return Some(b.1);
        }
    }
    None
}

/// `impd_signal_peak_level_info` for the base layout: (peak dBFS, explicit).
fn signal_peak(loud: &LoudnessInfoSet, ins: Option<&Instructions>, set_id: i32) -> (f32, bool) {
    let id = set_id.max(0);
    for want in [id, ID_FOR_ANY_DRC] {
        for l in loud.track.iter().filter(|l| l.drc_set_id == want && l.downmix_id == 0) {
            if let Some(p) = l.true_peak.or(l.sample_peak) {
                return (p, true);
            }
        }
    }
    if let Some(p) = ins
        .and_then(|i| i.limiter_peak_target)
        .filter(|_| ins.is_some_and(|i| !i.requires_eq && matches!(i.downmix_ids.first(), Some(&0) | Some(&ID_FOR_ANY_DOWNMIX))))
    {
        return (p, true);
    }
    (0.0, false)
}

fn effect_count(ins: &Instructions, dep: Option<&Instructions>) -> usize {
    EFFECT_TYPES.iter().filter(|&&e| e != EFFECT_GENERAL && (ins.effect & e != 0 || dep.is_some_and(|d| d.effect & e != 0))).count()
}

fn dependent<'a>(cfg: &'a DrcConfig, ins: &Instructions) -> Option<&'a Instructions> {
    ins.depends_on.and_then(|id| cfg.instructions.iter().find(|i| i.drc_set_id == id))
}

/// Run the DRC set selection.
pub fn select(cfg: Option<&DrcConfig>, loud: &LoudnessInfoSet, s: &DrcSettings) -> Selection {
    let empty = DrcConfig::default();
    let cfg = cfg.unwrap_or(&empty);
    let norm = s.target_loudness.is_some();
    let target = s.target_loudness.unwrap_or(0.0);
    // Candidate DRC sets: the transmitted ones for the base layout, plus "no DRC".
    let mut sets: Vec<(Option<usize>, i32)> = Vec::new();
    if s.drc {
        for (k, ins) in cfg.instructions.iter().enumerate() {
            let dmx_ok = ins.downmix_ids.iter().any(|&d| d == ID_FOR_ANY_DOWNMIX || (d == 0 && ins.drc_set_id > 0));
            let effect_ok = ins.effect != EFFECT_FADE && ins.effect != EFFECT_DUCK_OTHER && ins.effect != EFFECT_DUCK_SELF && ins.effect != 0;
            let use_ok = ins.depends_on.is_some() || !ins.no_independent_use;
            let groups_ok = !ins.groups.is_empty() && !ins.requires_eq;
            if dmx_ok && effect_ok && use_ok && groups_ok && ins.effect & (EFFECT_DUCK_OTHER | EFFECT_DUCK_SELF) == 0 {
                sets.push((Some(k), ins.drc_set_id));
            }
        }
    }
    sets.push((None, -1));

    let mut cands: Vec<Candidate> = Vec::new();
    for &(k, set_id) in &sets {
        let ins = k.and_then(|k| cfg.instructions.get(k));
        // loudness normalization gain(s)
        let mut lns: Vec<(f32, f32)> = Vec::new();
        if norm {
            let lookup = if set_id < 0 { 0 } else { set_id };
            for l in matching_loudness(&loud.track, lookup) {
                if let Some(v) = method_value(l) {
                    lns.push((target - v, v));
                }
            }
        }
        if lns.is_empty() {
            lns.push((0.0, UNDEFINED_LOUDNESS));
        }
        let (peak, explicit) = signal_peak(loud, ins, set_id);
        for (ln_gain, loudness) in lns {
            let mut c = Candidate { ins: k, set_id, ln_gain, peak: peak + ln_gain, loudness, explicit_peak: explicit, target_match: false };
            if set_id < 0 {
                // with DRC off the normalization gain is lowered right away to avoid clipping
                if !s.drc && explicit {
                    let adj = (peak + ln_gain - s.output_peak_level_max).max(0.0).min(s.loudness_deviation_max.max(0.0));
                    c.ln_gain -= adj;
                    c.peak = peak + c.ln_gain;
                }
            } else if let Some(i) = ins
                && let Some((upper, lower)) = i.target_loudness
                && (!norm || (upper as f32 >= target && (lower as f32) < target))
            {
                c.target_match = true;
                if !explicit {
                    c.peak = if norm { target - upper as f32 } else { 0.0 };
                }
            }
            let keep = set_id < 0 || c.target_match || c.explicit_peak || ins.is_some_and(|i| i.target_loudness.is_none());
            if keep {
                cands.push(c);
            }
        }
    }

    // Peak-level preselection
    let mut min_peak = 1000.0f32;
    let mut step2: Vec<Candidate> = Vec::new();
    for c in &cands {
        if c.target_match && !c.explicit_peak {
            step2.push(*c);
        } else {
            if c.peak <= s.output_peak_level_max {
                step2.push(*c);
            }
            min_peak = min_peak.min(c.peak);
        }
    }
    if step2.is_empty() {
        step2 = cands.iter().filter(|c| c.target_match && c.explicit_peak).copied().collect();
    }
    if step2.is_empty() {
        for c in &cands {
            if c.peak < min_peak + 1.0 {
                let mut c = *c;
                let adj = (c.peak - s.output_peak_level_max).max(0.0).min(s.loudness_deviation_max.max(0.0));
                c.ln_gain -= adj;
                c.peak -= adj;
                if c.loudness != UNDEFINED_LOUDNESS {
                    c.loudness -= adj;
                }
                step2.push(c);
            }
        }
    }
    let mut cands = step2;

    if s.drc && cands.len() > 1 {
        // effect type matching
        let has = |c: &Candidate, bit: u16| -> bool {
            c.ins.and_then(|k| cfg.instructions.get(k)).is_some_and(|i| i.effect & bit != 0 || dependent(cfg, i).is_some_and(|d| d.effect & bit != 0))
        };
        if s.effect != 0 {
            let m: Vec<Candidate> = cands.iter().filter(|c| has(c, s.effect)).copied().collect();
            if !m.is_empty() {
                cands = m;
            }
        } else {
            let without: Vec<Candidate> = cands
                .iter()
                .filter(|c| c.ins.and_then(|k| cfg.instructions.get(k)).is_none_or(|i| EFFECT_TYPES.iter().all(|&e| i.effect & e == 0)))
                .copied()
                .collect();
            if !without.is_empty() {
                cands = without;
            } else {
                for e in [EFFECT_GENERAL, EFFECT_NIGHT, EFFECT_NOISY, EFFECT_LIMITED, EFFECT_LOWLEVEL] {
                    let m: Vec<Candidate> = cands.iter().filter(|c| has(c, e)).copied().collect();
                    if !m.is_empty() {
                        cands = m;
                        break;
                    }
                }
            }
        }
        cands = final_selection(cfg, cands, norm, target);
    }
    let best = cands.iter().max_by_key(|c| c.set_id).copied();
    match best {
        Some(c) => Selection {
            instructions: c.ins,
            loudness_normalization_db: c.ln_gain,
            output_peak_level_db: c.peak,
            output_loudness: (c.loudness != UNDEFINED_LOUDNESS).then_some(c.loudness + c.ln_gain),
        },
        None => Selection { instructions: None, loudness_normalization_db: 0.0, output_peak_level_db: 0.0, output_loudness: None },
    }
}

/// `impd_drc_set_final_selection` (without EQ).
fn final_selection(cfg: &DrcConfig, cands: Vec<Candidate>, norm: bool, target: f32) -> Vec<Candidate> {
    let ins_of = |c: &Candidate| c.ins.and_then(|k| cfg.instructions.get(k));
    let min_peak = cands.iter().map(|c| c.peak).fold(10000.0f32, f32::min);
    if min_peak > 0.0 {
        return cands.into_iter().filter(|c| c.peak == min_peak).collect();
    }
    let mut v: Vec<Candidate> = cands.into_iter().filter(|c| c.peak <= 0.0).collect();
    // fewest effects
    let counts: Vec<usize> = v.iter().map(|c| ins_of(c).map_or(0, |i| effect_count(i, dependent(cfg, i)))).collect();
    if let Some(&m) = counts.iter().min() {
        v = v.into_iter().zip(counts).filter(|(_, n)| *n == m).map(|(c, _)| c).collect();
    }
    let upper = |c: &Candidate| ins_of(c).and_then(|i| i.target_loudness).map_or(100, |t| t.0);
    let lowest_upper = |v: Vec<Candidate>| -> Vec<Candidate> {
        let m = v.iter().map(upper).min().unwrap_or(100);
        v.into_iter().filter(|c| upper(c) == m).collect()
    };
    let matched = v.iter().filter(|c| c.target_match).count();
    if matched != 0 && matched != v.len() {
        v.retain(|c| !c.target_match);
    } else if matched == v.len() && matched > 0 {
        v = lowest_upper(v);
    }
    let in_range = |c: &Candidate| norm && ins_of(c).and_then(|i| i.target_loudness).is_some_and(|(u, l)| u as f32 >= target && (l as f32) < target);
    let n_in = v.iter().filter(|c| in_range(c)).count();
    if n_in != 0 && n_in != v.len() {
        v.retain(in_range);
        v = lowest_upper(v);
    } else if n_in == v.len() && n_in > 0 {
        v = lowest_upper(v);
    }
    let max_peak = v.iter().map(|c| c.peak).filter(|&p| p <= 0.0).fold(-1000.0f32, f32::max);
    v.into_iter().filter(|c| c.peak <= 0.0 && c.peak == max_peak).collect()
}

/// Whether a DRC set is a clipping-prevention set whose gains depend on the normalization gain.
pub fn is_clipping(ins: &Instructions) -> bool {
    ins.effect == EFFECT_CLIPPING
}
