// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Sirac Ozmen
//
// Ported to Rust for CFA4 from Maul Audio (https://github.com/Rawframe-Project/maul-audio, commit
// aeb383458250759f92a3e724a5bd006505e45b4d, src/binaural.c). CFA4 changes: owned buffers; a call
// longer than the block size is split instead of refused; non-finite input parameters are
// ignored (the last good ones are kept) instead of failing the call.

//! The binaural effect: each ear blends the four measured responses and delays around the
//! source's direction (as that ear sees it, near the head), reads its delayed signal from the
//! input's history by cubic interpolation, filters it for the head's near field and convolves it
//! with a direct-form FIR. A change of direction runs the old and new responses side by side for
//! a fixed fade (128 frames at 48 kHz), crossfading their outputs while the delays and near-field
//! filters move continuously. Blending FIR coefficients directly would comb-filter between
//! measured directions; crossfading outputs does not.

use std::sync::Arc;

use crate::dsp::{fir, read_delayed, shelve};
use crate::near_field::{NearFieldFilter, ear_direction, near_field};
use crate::{Error, HrtfSet, Result};

const FADE_SECONDS: f64 = 128.0 / 48_000.0;
/// Every delay is lengthened by one sample, so the cubic read never needs a later sample.
const BASE_DELAY: f32 = 1.0;
const MIN_RADIUS: f32 = 0.05;
const MAX_RADIUS: f32 = 0.15;
/// The near field's nearest distance: closer sources count as this close.
const MIN_DISTANCE: f32 = 0.1;
/// And its farthest, where its table is a source at infinity.
const MAX_DISTANCE: f32 = 1.0e9;

/// How a [`Binaural`] is made.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BinauralConfig {
    /// The longest block processed in one step; longer calls are split.
    pub max_frames: usize,
    /// Model the head's near field (sources closer than the set's distance).
    pub near_field: bool,
    /// Head radius in metres, 0.05 to 0.15.
    pub head_radius: f32,
}

impl Default for BinauralConfig {
    fn default() -> Self {
        BinauralConfig { max_frames: 1024, near_field: true, head_radius: 0.0875 }
    }
}

/// An ear pair's responses, delays and near-field filters.
#[derive(Clone, Debug)]
struct Filter {
    response: [Vec<f32>; 2],
    delay: [f32; 2],
    shelf: [[f32; 3]; 2],
}

impl Filter {
    fn new(taps: usize) -> Self {
        Filter { response: [vec![0.0; taps], vec![0.0; taps]], delay: [0.0; 2], shelf: [[1.0, 0.0, 0.0]; 2] }
    }
}

/// One mono source rendered binaurally.
#[derive(Clone, Debug)]
pub struct Binaural {
    hrtf: Arc<HrtfSet>,
    max_frames: usize,
    taps: usize,
    fade_frames: u32,
    /// The input history kept before each block: the largest delay and the cubic read's reach.
    reach: usize,
    history: Vec<f32>,
    /// Each ear's delayed signal, `taps - 1` samples of history first.
    ears: [Vec<f32>; 2],
    /// The new filter's output during a fade.
    scratch: Vec<f32>,
    near_field: bool,
    head_radius: f32,
    shelf_state: [[f32; 2]; 2],
    current: Filter,
    next: Filter,
    current_key: [f32; 3],
    next_key: [f32; 3],
    wanted_key: [f32; 3],
    fade_left: u32,
    gain: f32,
    primed: bool,
    last_position: [f32; 3],
    last_gain: f32,
}

fn largest(v: [f32; 3]) -> f32 {
    v[0].abs().max(v[1].abs()).max(v[2].abs())
}

/// The direction as a unit vector; straight ahead for a zero vector.
fn unit(v: [f32; 3]) -> [f32; 3] {
    let m = largest(v);
    let s = if m < 2f32.powi(-32) {
        2f32.powi(88)
    } else if m >= 2f32.powi(32) {
        2f32.powi(-88)
    } else {
        1.0
    };
    let v = [v[0] * s, v[1] * s, v[2] * s];
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length == 0.0 {
        return [0.0, 0.0, -1.0];
    }
    [v[0] / length, v[1] / length, v[2] / length]
}

impl Binaural {
    /// A renderer of one source with `hrtf`.
    pub fn new(hrtf: Arc<HrtfSet>, config: BinauralConfig) -> Result<Self> {
        if config.max_frames == 0 || config.max_frames > 1 << 16 {
            return Err(Error::Invalid("max_frames must be 1 to 65536"));
        }
        if !(MIN_RADIUS..=MAX_RADIUS).contains(&config.head_radius) {
            return Err(Error::Invalid("head radius must be 0.05 to 0.15 m"));
        }
        let taps = hrtf.taps();
        let reach = (hrtf.largest_delay() + BASE_DELAY).ceil() as usize + 3;
        let fade = (FADE_SECONDS * f64::from(hrtf.sample_rate())).round();
        let max_frames = config.max_frames;
        Ok(Binaural {
            max_frames,
            taps,
            fade_frames: if fade < 1.0 { 1 } else { fade as u32 },
            reach,
            history: vec![0.0; reach + max_frames],
            ears: [vec![0.0; taps - 1 + max_frames], vec![0.0; taps - 1 + max_frames]],
            scratch: vec![0.0; max_frames],
            near_field: config.near_field,
            head_radius: config.head_radius,
            shelf_state: [[0.0; 2]; 2],
            current: Filter::new(taps),
            next: Filter::new(taps),
            current_key: [0.0; 3],
            next_key: [0.0; 3],
            wanted_key: [0.0; 3],
            fade_left: 0,
            gain: 1.0,
            primed: false,
            last_position: [0.0, 0.0, -1.0],
            last_gain: 1.0,
            hrtf,
        })
    }

    /// The HRTF set this renderer uses.
    pub fn hrtf(&self) -> &Arc<HrtfSet> {
        &self.hrtf
    }

    /// Forgets the input history and the fade: the next block starts from silence.
    pub fn reset(&mut self) {
        self.history.fill(0.0);
        for e in &mut self.ears {
            e.fill(0.0);
        }
        self.shelf_state = [[0.0; 2]; 2];
        self.fade_left = 0;
        self.primed = false;
    }

    /// The latency the renderer adds, in samples: the base delay. (The responses' own onset
    /// delays are part of the sound, not latency.)
    pub fn latency(&self) -> usize {
        BASE_DELAY as usize
    }

    /// What the filters depend on: the position from 0.1 m to 1e9 m away with the near field, the
    /// direction alone without it.
    fn key(&self, position: [f32; 3]) -> [f32; 3] {
        let u = unit(position);
        if !self.near_field {
            return u;
        }
        let length = if largest(position) >= MAX_DISTANCE {
            MAX_DISTANCE
        } else {
            (position[0] * position[0] + position[1] * position[1] + position[2] * position[2]).sqrt()
        };
        let d = length.clamp(MIN_DISTANCE, MAX_DISTANCE);
        [u[0] * d, u[1] * d, u[2] * d]
    }

    /// Fills one ear of a filter for a key: the direction the ear looks its response up at (with
    /// parallax under the near field), converted from the listener's frame to the set's (azimuth
    /// counterclockwise from ahead, -z; left is -x; up is +y), and its near-field filter.
    fn design_ear(hrtf: &HrtfSet, near: bool, head_radius: f32, filter: &mut Filter, ear: usize, key: [f32; 3]) {
        let ear_x = if ear == 0 { -head_radius } else { head_radius };
        let look = if near { ear_direction(key, ear_x, hrtf.distance()) } else { key };
        let azimuth = (-look[0]).atan2(-look[2]).to_degrees();
        let elevation = look[1].clamp(-1.0, 1.0).asin().to_degrees();
        let neighbours = hrtf.neighbours(azimuth, elevation);
        filter.delay[ear] = hrtf.blend(&neighbours, ear, &mut filter.response[ear]) + BASE_DELAY;
        let mut shelf = NearFieldFilter::IDENTITY;
        if near {
            let u = unit(key);
            let facing = (if ear == 0 { -u[0] } else { u[0] }).clamp(-1.0, 1.0);
            let distance = (key[0] * key[0] + key[1] * key[1] + key[2] * key[2]).sqrt();
            shelf = near_field(facing.acos().to_degrees(), head_radius / distance, head_radius / hrtf.distance(), head_radius, hrtf.sample_rate() as f32);
        }
        filter.shelf[ear] = [shelf.b0, shelf.b1, shelf.a1];
    }

    fn design(&self, filter: &mut Filter, key: [f32; 3]) {
        for ear in 0..2 {
            Self::design_ear(&self.hrtf, self.near_field, self.head_radius, filter, ear, key);
        }
    }

    fn render_steady(&mut self, out: [&mut [f32]; 2], done: usize, span: usize) {
        let x_at = self.reach + done;
        let at = self.taps - 1 + done;
        let [out_l, out_r] = out;
        for (ear, o) in [out_l, out_r].into_iter().enumerate() {
            let signal = &mut self.ears[ear][at..at + span];
            read_delayed(&self.history, x_at, self.current.delay[ear], 0.0, signal);
            if self.near_field {
                shelve(signal, self.current.shelf[ear], [0.0; 3], &mut self.shelf_state[ear]);
            }
            fir(&self.ears[ear], at, &self.current.response[ear], &mut o[done..done + span]);
        }
    }

    fn render_fade(&mut self, out: [&mut [f32]; 2], done: usize, span: usize) {
        let fade = self.fade_frames as f32;
        let faded = self.fade_frames - self.fade_left;
        let x_at = self.reach + done;
        let at = self.taps - 1 + done;
        let [out_l, out_r] = out;
        for (ear, o) in [out_l, out_r].into_iter().enumerate() {
            let from = self.current.delay[ear];
            let step = (self.next.delay[ear] - from) / fade;
            let signal = &mut self.ears[ear][at..at + span];
            read_delayed(&self.history, x_at, from + step * (faded + 1) as f32, step, signal);
            if self.near_field {
                let mut start = [0.0; 3];
                let mut steps = [0.0; 3];
                for k in 0..3 {
                    steps[k] = (self.next.shelf[ear][k] - self.current.shelf[ear][k]) / fade;
                    start[k] = self.current.shelf[ear][k] + steps[k] * (faded + 1) as f32;
                }
                shelve(signal, start, steps, &mut self.shelf_state[ear]);
            }
            let old = &mut o[done..done + span];
            fir(&self.ears[ear], at, &self.current.response[ear], old);
            let scratch = &mut self.scratch[..span];
            fir(&self.ears[ear], at, &self.next.response[ear], scratch);
            for (n, (o, s)) in old.iter_mut().zip(scratch.iter()).enumerate() {
                let share = (faded as usize + n + 1) as f32 / fade;
                *o += share * (s - *o);
            }
        }
    }

    fn render(&mut self, out: [&mut [f32]; 2], frames: usize) {
        let [out_l, out_r] = out;
        let mut done = 0;
        while done < frames {
            if self.fade_left == 0 && self.wanted_key != self.current_key {
                let mut next = std::mem::replace(&mut self.next, Filter::new(0));
                self.design(&mut next, self.wanted_key);
                self.next = next;
                self.next_key = self.wanted_key;
                self.fade_left = self.fade_frames;
            }
            if self.fade_left == 0 {
                self.render_steady([&mut *out_l, &mut *out_r], done, frames - done);
                return;
            }
            let span = (self.fade_left as usize).min(frames - done);
            self.render_fade([&mut *out_l, &mut *out_r], done, span);
            self.fade_left -= span as u32;
            done += span;
            if self.fade_left == 0 {
                std::mem::swap(&mut self.current, &mut self.next);
                self.current_key = self.next_key;
            }
        }
    }

    /// Renders `input` from `position` (metres, listener's frame) at linear `gain` into the two
    /// ears, overwriting `left` and `right` (each at least `input.len()` long). Position and gain
    /// move from the previous call's across the call. A non-finite position or gain keeps the
    /// previous one.
    pub fn process(&mut self, position: [f32; 3], gain: f32, input: &[f32], left: &mut [f32], right: &mut [f32]) {
        let position = if position.iter().all(|v| v.is_finite()) { position } else { self.last_position };
        let gain = if gain.is_finite() { gain } else { self.last_gain };
        if !self.primed {
            self.gain = gain;
        }
        self.last_position = position;
        self.last_gain = gain;
        let frames = input.len().min(left.len()).min(right.len());
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(self.max_frames);
            // the gain ramps across the whole call
            let g0 = self.gain + (gain - self.gain) * done as f32 / frames as f32;
            let g1 = self.gain + (gain - self.gain) * (done + n) as f32 / frames as f32;
            self.block(position, g0, g1, &input[done..done + n], &mut left[done..done + n], &mut right[done..done + n]);
            done += n;
        }
        if frames > 0 {
            self.gain = gain;
        }
    }

    fn block(&mut self, position: [f32; 3], g0: f32, g1: f32, input: &[f32], left: &mut [f32], right: &mut [f32]) {
        let frames = input.len();
        let key = self.key(position);
        if !self.primed {
            let mut current = std::mem::replace(&mut self.current, Filter::new(0));
            self.design(&mut current, key);
            self.current = current;
            self.current_key = key;
            self.primed = true;
        }
        self.wanted_key = key;
        self.history[self.reach..self.reach + frames].copy_from_slice(input);
        self.render([&mut *left, &mut *right], frames);
        let step = (g1 - g0) / frames as f32;
        for o in [left, right] {
            for (n, v) in o.iter_mut().enumerate() {
                *v *= g0 + step * (n + 1) as f32;
            }
        }
        self.history.copy_within(frames..frames + self.reach, 0);
        for e in &mut self.ears {
            e.copy_within(frames..frames + self.taps - 1, 0);
        }
    }
}
