// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the CFA4 contributors

//! A spatial scene heard through headphones: bed channels and moving objects, each a mono signal
//! with timed state updates (`docs/plans/CFA4-BINAURAL-MONITOR-001.md` §3.2, work package B-02).
//! The format adapters ([`crate::atmos`]) turn Dolby TrueHD Atmos and IAB metadata into
//! [`SceneState`]s; [`SceneRenderer`] renders them to two ears. Its choices, all CFA4's own:
//!
//! - **Room to listener.** An object's position is a point in the unit room cube (x left → right,
//!   y front → back, z ear height → ceiling), the convention of `cfa4_truehd::render` and
//!   `cfa4_iab::render`. The listener sits at the centre of the floor of the cube, at ear height;
//!   the walls are [`SceneConfig::room_half_width`] metres away. Azimuths are warped so that the
//!   cube's front corners lie at ±30° (where the room's L and R loudspeakers stand), its side
//!   walls' middles at ±90° and its rear corners at ±135°: from 0° to 45° the cube's azimuth is
//!   scaled by 2/3, from 45° to 90° stretched onto 30° to 90°, and past 90° kept. Elevation and
//!   distance come from the cube unwarped. An object near the head gets the near field.
//! - **Beds** play from their loudspeaker's nominal direction ([`crate::Speaker::direction`]) at
//!   the HRTF set's own distance; the LFE is low-passed at 120 Hz into both ears at
//!   [`SceneConfig::lfe_db`].
//! - **Updates** start at their sample and move position, size and gain linearly over their ramp;
//!   the renderer follows a ramp in steps of at most [`STEP`] samples (the HRTF crossfade smooths
//!   between steps). An object's first update applies at once, and it is silent before it.
//! - **Size** (0 a point, 1 the whole room) spreads an object over four more directions around it
//!   (up to ±90° in azimuth and ±45° in elevation), each fed through its own chain of allpass
//!   filters so that they are not correlated; the centre keeps 1 - 0.8 × size of the power.
//! - **Headphone mode** (Dolby's binaural render mode): [`HeadphoneMode::Off`] pans in plain
//!   stereo without HRTF by the object's azimuth, [`HeadphoneMode::Near`] is dry binaural, and
//!   [`HeadphoneMode::Mid`] and [`HeadphoneMode::Far`] add a small and a larger room: the
//!   existing Surround Reverb (`cfa4_audio_dsp`, early reflections and an FDN late field) fed with
//!   the objects' binaural signal. [`SceneConfig::room`] turns both rooms off.
//!
//! Scheduling may allocate (a new object, a first size); rendering does not.

use std::sync::Arc;

use cfa4_audio_dsp::AudioEffect;
use cfa4_audio_dsp::effects::reverbs::SurroundReverb;

use crate::speakers::LowPass;
use crate::{Binaural, BinauralConfig, Error, HrtfSet, Result, Speaker, position_of};

/// The longest step the renderer takes while an update ramps, in samples.
pub const STEP: usize = 64;
/// Most queued updates; more drop the oldest.
const MAX_PENDING: usize = 4096;
/// Most objects in a scene.
pub const MAX_OBJECTS: usize = 1024;
/// The share of power size 1 moves off the centre.
const SPREAD_POWER: f32 = 0.8;
/// Azimuth and elevation offsets of the spread directions at size 1, in degrees.
const SPREAD_DEGREES: [(f32, f32); 4] = [(90.0, 0.0), (-90.0, 0.0), (0.0, 45.0), (0.0, -45.0)];
/// Allpass delays (samples at 48 kHz) of each spread direction's decorrelator.
const ALLPASS_DELAYS: [[usize; 3]; 4] = [[113, 337, 509], [149, 293, 457], [167, 311, 541], [131, 269, 487]];
const ALLPASS_GAIN: f32 = 0.5;

/// Where an object plays from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Placement {
    /// A bed channel at its loudspeaker's direction ([`Speaker::Lfe`] for the LFE).
    Speaker(Speaker),
    /// A point in the unit room cube (see the module notes), and its size, 0 ..= 1.
    Room { position: [f32; 3], size: f32 },
}

/// How an object is to be rendered for headphones (Dolby's binaural render mode).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HeadphoneMode {
    /// Plain stereo panning, no HRTF.
    Off,
    /// Binaural, dry.
    Near,
    /// Binaural in a small room (Dolby's default).
    #[default]
    Mid,
    /// Binaural in a larger room.
    Far,
}

/// One object's state from one update.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneState {
    pub placement: Placement,
    /// Linear gain; 0 is silent.
    pub gain: f32,
    pub mode: HeadphoneMode,
}

/// How a [`SceneRenderer`] is made.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneConfig {
    /// Metres from the listener to the room's side walls (the cube's half width), 0.5 to 20.
    pub room_half_width: f32,
    /// The LFE's level in both ears, in dB.
    pub lfe_db: f32,
    /// Whether [`HeadphoneMode::Mid`] and [`HeadphoneMode::Far`] add their rooms.
    pub room: bool,
    /// The longest block processed in one step.
    pub max_frames: usize,
    /// Head radius in metres, 0.05 to 0.15.
    pub head_radius: f32,
}

impl Default for SceneConfig {
    fn default() -> Self {
        SceneConfig { room_half_width: 2.0, lfe_db: -10.0, room: true, max_frames: 1024, head_radius: 0.0875 }
    }
}

/// Azimuth (degrees, counterclockwise, left positive), elevation (degrees) and distance (metres)
/// of a room-cube point, warped as the module notes say.
pub fn room_direction(position: [f32; 3], half_width: f32) -> (f32, f32, f32) {
    let c = |v: f32| if v.is_finite() { v.clamp(-1.0, 2.0) } else { 0.5 };
    let x = (c(position[0]) - 0.5) * 2.0;
    let ahead = (0.5 - c(position[1])) * 2.0;
    let up = if position[2].is_finite() { position[2].clamp(0.0, 1.0) } else { 0.0 };
    let r = (x * x + ahead * ahead).sqrt();
    let azimuth = (-x).atan2(ahead).to_degrees();
    let a = azimuth.abs();
    let warped = if a <= 45.0 {
        a * 30.0 / 45.0
    } else if a <= 90.0 {
        30.0 + (a - 45.0) * 60.0 / 45.0
    } else {
        a
    };
    let azimuth = warped.copysign(azimuth);
    let elevation = up.atan2(r).to_degrees();
    (azimuth, elevation, (r * r + up * up).sqrt() * half_width)
}

/// The listener-frame position (metres) of a room-cube point.
pub fn room_position(position: [f32; 3], half_width: f32) -> [f32; 3] {
    let (a, e, d) = room_direction(position, half_width);
    position_of(a, e, d)
}

/// A Schroeder allpass.
#[derive(Clone, Debug)]
struct Allpass {
    line: Vec<f32>,
    at: usize,
}

impl Allpass {
    fn new(delay: usize) -> Self {
        Allpass { line: vec![0.0; delay.max(1)], at: 0 }
    }
    fn run(&mut self, x: f32) -> f32 {
        let delayed = self.line[self.at];
        let v = crate::dsp::flush(x + ALLPASS_GAIN * delayed);
        self.line[self.at] = v;
        self.at = if self.at + 1 == self.line.len() { 0 } else { self.at + 1 };
        delayed - ALLPASS_GAIN * v
    }
}

/// The four decorrelated directions of an object with a size.
struct Spread {
    branches: Vec<(Binaural, [Allpass; 3])>,
}

impl Spread {
    fn new(hrtf: &Arc<HrtfSet>, config: BinauralConfig) -> Result<Self> {
        let scale = hrtf.sample_rate() as f32 / 48_000.0;
        let mut branches = Vec::with_capacity(4);
        for d in ALLPASS_DELAYS {
            let ap = d.map(|n| Allpass::new((n as f32 * scale).round() as usize));
            branches.push((Binaural::new(hrtf.clone(), config)?, ap));
        }
        Ok(Spread { branches })
    }
}

/// What an object's state means for rendering, at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Target {
    /// Room-cube position, or the loudspeaker direction's position (metres) for a bed.
    position: [f32; 3],
    size: f32,
    gain: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Room,
    Bed,
    Lfe,
}

struct Object {
    binaural: Binaural,
    spread: Option<Spread>,
    lowpass: LowPass,
    kind: Kind,
    mode: HeadphoneMode,
    from: Target,
    to: Target,
    start: u64,
    len: u64,
    placed: bool,
    /// Placed since the last step: the pan and LFE gains start at their targets.
    fresh: bool,
    /// The stereo pan's gains at the end of the last step ([`HeadphoneMode::Off`]).
    pan: [f32; 2],
    /// The LFE gain at the end of the last step.
    lfe_gain: f32,
}

impl Object {
    fn at(&self, t: u64) -> Target {
        let done = t.saturating_sub(self.start);
        if self.len == 0 || done >= self.len {
            return self.to;
        }
        let k = done as f32 / self.len as f32;
        let mix = |a: f32, b: f32| a + (b - a) * k;
        Target {
            position: [
                mix(self.from.position[0], self.to.position[0]),
                mix(self.from.position[1], self.to.position[1]),
                mix(self.from.position[2], self.to.position[2]),
            ],
            size: mix(self.from.size, self.to.size),
            gain: mix(self.from.gain, self.to.gain),
        }
    }
}

struct Pending {
    at: u64,
    object: usize,
    state: SceneState,
    ramp: u64,
}

/// Renders a scene of objects to two ears, keeping each object's state and ramps across calls.
pub struct SceneRenderer {
    hrtf: Arc<HrtfSet>,
    config: SceneConfig,
    binaural: BinauralConfig,
    objects: Vec<Option<Object>>,
    pending: Vec<Pending>,
    lfe_gain: f32,
    /// One step's ears, the spread branches' input, silence.
    left: Vec<f32>,
    right: Vec<f32>,
    branch: Vec<f32>,
    zeros: Vec<f32>,
    /// The room sends (left, right) of [`HeadphoneMode::Mid`] and [`HeadphoneMode::Far`].
    sends: [[Vec<f32>; 2]; 2],
    rooms: [SurroundReverb; 2],
    room_levels: [f32; 2],
}

/// Surround Reverb settings of the mid and far rooms, and their levels.
const ROOMS: [(&[(&str, f32)], f32); 2] = [
    (&[("room_size", 25.0), ("decay", 0.35), ("predelay", 4.0), ("damping", 50.0), ("early", 70.0), ("high_cut", 9000.0)], 0.25),
    (&[("room_size", 60.0), ("decay", 0.8), ("predelay", 12.0), ("damping", 40.0), ("early", 50.0), ("high_cut", 8000.0)], 0.45),
];

impl SceneRenderer {
    /// A renderer of scenes with `hrtf`.
    pub fn new(hrtf: Arc<HrtfSet>, config: SceneConfig) -> Result<Self> {
        if !(0.5..=20.0).contains(&config.room_half_width) {
            return Err(Error::Invalid("room half width must be 0.5 to 20 m"));
        }
        if !config.lfe_db.is_finite() {
            return Err(Error::Invalid("LFE level must be finite"));
        }
        let binaural = BinauralConfig { max_frames: config.max_frames, near_field: true, head_radius: config.head_radius };
        // checks max_frames and the head radius
        Binaural::new(hrtf.clone(), binaural)?;
        let rate = hrtf.sample_rate() as f32;
        let room = |settings: &[(&str, f32)]| {
            let mut r = SurroundReverb::new(rate, 2);
            for &(k, v) in settings {
                r.set_param(k, v);
            }
            r.set_param("dry", 0.0);
            r.set_param("wet", 100.0);
            r.set_param("center_input", 100.0);
            r.reset();
            r
        };
        let n = config.max_frames;
        Ok(SceneRenderer {
            config,
            binaural,
            objects: Vec::new(),
            pending: Vec::new(),
            lfe_gain: 10f32.powf(config.lfe_db / 20.0),
            left: vec![0.0; n],
            right: vec![0.0; n],
            branch: vec![0.0; n],
            zeros: vec![0.0; n],
            sends: std::array::from_fn(|_| [vec![0.0; n], vec![0.0; n]]),
            rooms: [room(ROOMS[0].0), room(ROOMS[1].0)],
            room_levels: [ROOMS[0].1, ROOMS[1].1],
            hrtf,
        })
    }

    /// The HRTF set.
    pub fn hrtf(&self) -> &Arc<HrtfSet> {
        &self.hrtf
    }

    /// The latency the renderer adds, in samples.
    pub fn latency(&self) -> usize {
        1
    }

    /// Drops every object's state and history (after a seek).
    pub fn reset(&mut self) {
        self.objects.clear();
        self.pending.clear();
        for r in &mut self.rooms {
            r.reset();
        }
    }

    /// Sets `object` to `state` from sample `at` on, moving there over `ramp` samples.
    pub fn schedule(&mut self, at: u64, object: usize, state: SceneState, ramp: u64) -> Result<()> {
        if object >= MAX_OBJECTS {
            return Err(Error::Invalid("object index past the scene's limit"));
        }
        if self.objects.len() <= object {
            self.objects.resize_with(object + 1, || None);
        }
        let rate = self.hrtf.sample_rate() as f32;
        let slot = &mut self.objects[object];
        if slot.is_none() {
            let zero = Target { position: [0.5, 0.5, 0.0], size: 0.0, gain: 0.0 };
            *slot = Some(Object {
                binaural: Binaural::new(self.hrtf.clone(), self.binaural)?,
                spread: None,
                lowpass: LowPass::new(120.0, rate),
                kind: Kind::Room,
                mode: HeadphoneMode::Mid,
                from: zero,
                to: zero,
                start: 0,
                len: 0,
                placed: false,
                fresh: false,
                pan: [0.0; 2],
                lfe_gain: 0.0,
            });
        }
        if let (Some(o), Placement::Room { size, .. }) = (slot.as_mut(), state.placement)
            && size > 0.0
            && o.spread.is_none()
        {
            o.spread = Some(Spread::new(&self.hrtf, self.binaural)?);
        }
        if self.pending.len() >= MAX_PENDING {
            self.pending.remove(0);
        }
        // of two updates for the same sample the later one wins
        let i = self.pending.partition_point(|p| p.at <= at);
        self.pending.insert(i, Pending { at, object, state, ramp });
        Ok(())
    }

    fn apply(&mut self, p: Pending) {
        let distance = self.hrtf.distance();
        let Some(Some(o)) = self.objects.get_mut(p.object) else { return };
        let finite = |v: f32, or: f32| if v.is_finite() { v } else { or };
        let gain = finite(p.state.gain, 0.0).max(0.0);
        let (kind, target) = match p.state.placement {
            Placement::Speaker(s) => match s.direction() {
                Some((a, e)) => (Kind::Bed, Target { position: position_of(a, e, distance), size: 0.0, gain }),
                None => (Kind::Lfe, Target { position: [0.0; 3], size: 0.0, gain }),
            },
            Placement::Room { position, size } => {
                (Kind::Room, Target { position: position.map(|v| finite(v, 0.5)), size: finite(size, 0.0).clamp(0.0, 1.0), gain })
            }
        };
        // a change of kind or a first state applies at once: there is nothing to ramp from
        let now = if o.placed && o.kind == kind { o.at(p.at) } else { target };
        let len = if o.placed && o.kind == kind { p.ramp } else { 0 };
        o.fresh |= !(o.placed && o.kind == kind);
        o.from = now;
        o.to = target;
        o.start = p.at;
        o.len = len;
        o.kind = kind;
        o.mode = p.state.mode;
        o.placed = true;
    }

    /// Renders the objects (`objects[o][k]` is object o's sample at `start + k`; a missing or short
    /// signal is silence) into the two ears, overwriting `left` and `right`. The frame count is
    /// the shorter of the two.
    pub fn render(&mut self, start: u64, objects: &[&[f32]], left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len());
        left[..frames].fill(0.0);
        right[..frames].fill(0.0);
        let mut done = 0;
        while done < frames {
            let t = start.saturating_add(done as u64);
            while self.pending.first().is_some_and(|p| p.at <= t) {
                let p = self.pending.remove(0);
                self.apply(p);
            }
            let ramping = self.objects.iter().flatten().any(|o| o.placed && o.start.saturating_add(o.len) > t);
            let mut n = (frames - done).min(self.config.max_frames);
            if ramping {
                n = n.min(STEP);
            }
            if let Some(p) = self.pending.first() {
                n = n.min(usize::try_from(p.at - t).unwrap_or(usize::MAX).max(1));
            }
            self.step(t, done, n, objects, left, right);
            done += n;
        }
    }

    /// Renders `n` samples from absolute time `t` (`done` samples into the call).
    fn step(&mut self, t: u64, done: usize, n: usize, objects: &[&[f32]], left: &mut [f32], right: &mut [f32]) {
        let room_on = self.config.room;
        for s in self.sends.iter_mut().flatten() {
            s[..n].fill(0.0);
        }
        let half_width = self.config.room_half_width;
        for (index, slot) in self.objects.iter_mut().enumerate() {
            let Some(o) = slot else { continue };
            if !o.placed {
                continue;
            }
            let input = objects.get(index).and_then(|s| s.get(done..done + n));
            // past a signal's end: silence, so the object's history still moves
            let input = input.unwrap_or(&self.zeros[..n]);
            let target = o.at(t.saturating_add(n as u64));
            let fresh = std::mem::take(&mut o.fresh);
            let (out_l, out_r) = (&mut left[done..done + n], &mut right[done..done + n]);
            match o.kind {
                Kind::Lfe => {
                    let g1 = target.gain * self.lfe_gain;
                    let g0 = if fresh { g1 } else { o.lfe_gain };
                    for (i, &x) in input.iter().enumerate() {
                        let y = o.lowpass.run(x) * (g0 + (g1 - g0) * (i + 1) as f32 / n as f32);
                        out_l[i] += y;
                        out_r[i] += y;
                    }
                    o.lfe_gain = g1;
                }
                Kind::Bed | Kind::Room => {
                    let (position, azimuth, elevation, distance) = match o.kind {
                        Kind::Room => {
                            let (a, e, d) = room_direction(target.position, half_width);
                            (position_of(a, e, d), a, e, d)
                        }
                        _ => {
                            let p = target.position;
                            let d = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
                            (p, (-p[0]).atan2(-p[2]).to_degrees(), (p[1] / d.max(1e-9)).clamp(-1.0, 1.0).asin().to_degrees(), d)
                        }
                    };
                    if o.mode == HeadphoneMode::Off {
                        // constant-power stereo pan by azimuth: left positive
                        let s = azimuth.to_radians().sin().clamp(-1.0, 1.0);
                        let angle = std::f32::consts::FRAC_PI_4 * (1.0 - s);
                        let to = [angle.cos() * target.gain, angle.sin() * target.gain];
                        let from = if fresh { to } else { o.pan };
                        for (i, &x) in input.iter().enumerate() {
                            let k = (i + 1) as f32 / n as f32;
                            out_l[i] += x * (from[0] + (to[0] - from[0]) * k);
                            out_r[i] += x * (from[1] + (to[1] - from[1]) * k);
                        }
                        o.pan = to;
                        continue;
                    }
                    let centre = (1.0 - SPREAD_POWER * target.size).sqrt();
                    let send = match o.mode {
                        HeadphoneMode::Mid if room_on => Some(0),
                        HeadphoneMode::Far if room_on => Some(1),
                        _ => None,
                    };
                    let (bl, br) = (&mut self.left[..n], &mut self.right[..n]);
                    o.binaural.process(position, target.gain * centre, input, bl, br);
                    add(out_l, out_r, bl, br, send.map(|s| &mut self.sends[s]), n);
                    if let Some(spread) = &mut o.spread {
                        let share = (SPREAD_POWER * target.size / 4.0).sqrt() * target.gain;
                        for ((b, aps), &(da, de)) in spread.branches.iter_mut().zip(&SPREAD_DEGREES) {
                            let bin = &mut self.branch[..n];
                            for (y, &x) in bin.iter_mut().zip(input) {
                                *y = aps.iter_mut().fold(x, |v, ap| ap.run(v));
                            }
                            let p = position_of(azimuth + da * target.size, (elevation + de * target.size).clamp(-90.0, 90.0), distance);
                            b.process(p, share, bin, bl, br);
                            add(out_l, out_r, bl, br, send.map(|s| &mut self.sends[s]), n);
                        }
                    }
                }
            }
        }
        if room_on {
            for (r, (send, level)) in self.rooms.iter_mut().zip(self.sends.iter_mut().zip(self.room_levels)) {
                let [sl, sr] = send;
                r.process(&mut [&mut sl[..n], &mut sr[..n]]);
                for i in 0..n {
                    left[done + i] += level * sl[i];
                    right[done + i] += level * sr[i];
                }
            }
        }
    }

    #[cfg(test)]
    fn objects_placed(&self) -> usize {
        self.objects.iter().flatten().filter(|o| o.placed).count()
    }
}

fn add(out_l: &mut [f32], out_r: &mut [f32], bl: &[f32], br: &[f32], send: Option<&mut [Vec<f32>; 2]>, n: usize) {
    for i in 0..n {
        out_l[i] += bl[i];
        out_r[i] += br[i];
    }
    if let Some([sl, sr]) = send {
        for i in 0..n {
            sl[i] += bl[i];
            sr[i] += br[i];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_room_maps_onto_the_loudspeaker_directions() {
        let d = |p| {
            let (a, e, _) = room_direction(p, 2.0);
            (a.round(), e.round())
        };
        assert_eq!(d([0.5, 0.0, 0.0]), (0.0, 0.0));
        assert_eq!(d([0.0, 0.0, 0.0]), (30.0, 0.0));
        assert_eq!(d([1.0, 0.0, 0.0]), (-30.0, 0.0));
        assert_eq!(d([0.0, 0.5, 0.0]), (90.0, 0.0));
        assert_eq!(d([0.0, 1.0, 0.0]), (135.0, 0.0));
        assert_eq!(d([0.5, 1.0, 0.0]).0.abs(), 180.0);
        assert_eq!(d([0.5, 0.5, 1.0]).1, 90.0);
        let (_, _, dist) = room_direction([0.5, 0.0, 0.0], 2.0);
        assert!((dist - 2.0).abs() < 1e-5);
    }

    #[test]
    fn hostile_positions_stay_finite() {
        for p in [[f32::NAN, 0.0, 0.0], [f32::INFINITY, f32::NEG_INFINITY, f32::NAN], [1e30, -1e30, 1e30], [0.5, 0.5, 0.0]] {
            let q = room_position(p, 2.0);
            assert!(q.iter().all(|v| v.is_finite()), "{p:?} → {q:?}");
        }
    }

    #[test]
    fn a_reset_forgets_every_object() {
        let hrtf = Arc::new(HrtfSet::builtin(48_000).unwrap());
        let mut s = SceneRenderer::new(hrtf, SceneConfig::default()).unwrap();
        s.schedule(0, 2, SceneState { placement: Placement::Room { position: [0.2, 0.2, 0.0], size: 0.5 }, gain: 1.0, mode: HeadphoneMode::Near }, 0).unwrap();
        let x = vec![0.1f32; 256];
        let (mut l, mut r) = (vec![0.0; 256], vec![0.0; 256]);
        s.render(0, &[&[], &[], &x], &mut l, &mut r);
        assert_eq!(s.objects_placed(), 1);
        assert!(s.objects[2].as_ref().is_some_and(|o| o.spread.is_some()));
        s.reset();
        assert_eq!(s.objects_placed(), 0);
    }
}
