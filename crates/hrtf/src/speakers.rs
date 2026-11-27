// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the CFA4 contributors

//! Virtual loudspeakers: a channel layout heard through headphones. Every full-range channel is a
//! [`Binaural`] source at its loudspeaker's nominal direction (ITU-R BS.2051 / Dolby convention,
//! listed in [`Speaker::direction`]) and at the HRTF set's measurement distance, so the near field
//! is not used; the LFE is low-passed at 120 Hz (second-order Butterworth) and sent to both ears at
//! [`VirtualSpeakersConfig::lfe_db`]. Unity gain per channel, as a loudspeaker playback would be.

use std::sync::Arc;

use crate::{Binaural, BinauralConfig, Error, HrtfSet, Result};

/// A loudspeaker of a layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Speaker {
    L,
    R,
    C,
    Lfe,
    /// 5.1 surround left / right (±110°).
    Ls,
    Rs,
    /// 7.1 side surround (±90°).
    Lss,
    Rss,
    /// 7.1 rear surround (±135°).
    Lrs,
    Rrs,
    /// Top middle (7.1.2: ±90°, 60° up).
    Ltm,
    Rtm,
    /// Top front (±45°, 45° up).
    Ltf,
    Rtf,
    /// Top rear (±135°, 45° up).
    Ltr,
    Rtr,
    /// Any other direction: azimuth (counterclockwise, degrees) and elevation (degrees), in
    /// hundredths of a degree.
    At(i32, i32),
}

impl Speaker {
    /// The nominal direction: azimuth in degrees counterclockwise from straight ahead (left
    /// positive) and elevation in degrees up. `None` for the LFE.
    pub fn direction(self) -> Option<(f32, f32)> {
        use Speaker::*;
        Some(match self {
            L => (30.0, 0.0),
            R => (-30.0, 0.0),
            C => (0.0, 0.0),
            Lfe => return None,
            Ls => (110.0, 0.0),
            Rs => (-110.0, 0.0),
            Lss => (90.0, 0.0),
            Rss => (-90.0, 0.0),
            Lrs => (135.0, 0.0),
            Rrs => (-135.0, 0.0),
            Ltm => (90.0, 60.0),
            Rtm => (-90.0, 60.0),
            Ltf => (45.0, 45.0),
            Rtf => (-45.0, 45.0),
            Ltr => (135.0, 45.0),
            Rtr => (-135.0, 45.0),
            At(a, e) => (a as f32 / 100.0, e as f32 / 100.0),
        })
    }
}

/// The position (metres, listener's frame: x right, y up, -z ahead) of a direction at `distance`.
pub fn position_of(azimuth_degrees: f32, elevation_degrees: f32, distance: f32) -> [f32; 3] {
    let (a, e) = (azimuth_degrees.to_radians(), elevation_degrees.to_radians());
    [-a.sin() * e.cos() * distance, e.sin() * distance, -a.cos() * e.cos() * distance]
}

/// A channel layout in its channel order (the WAV / SMPTE order CFA4's decoders give).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeakerLayout(pub Vec<Speaker>);

impl SpeakerLayout {
    pub fn mono() -> Self {
        SpeakerLayout(vec![Speaker::C])
    }
    pub fn stereo() -> Self {
        SpeakerLayout(vec![Speaker::L, Speaker::R])
    }
    /// L R C LFE Ls Rs.
    pub fn surround_5_1() -> Self {
        use Speaker::*;
        SpeakerLayout(vec![L, R, C, Lfe, Ls, Rs])
    }
    /// L R C LFE Lss Rss Lrs Rrs.
    pub fn surround_7_1() -> Self {
        use Speaker::*;
        SpeakerLayout(vec![L, R, C, Lfe, Lss, Rss, Lrs, Rrs])
    }
    /// 7.1 and the top middle pair.
    pub fn surround_7_1_2() -> Self {
        use Speaker::*;
        SpeakerLayout(vec![L, R, C, Lfe, Lss, Rss, Lrs, Rrs, Ltm, Rtm])
    }
    /// 7.1 and the top front and top rear pairs.
    pub fn surround_7_1_4() -> Self {
        use Speaker::*;
        SpeakerLayout(vec![L, R, C, Lfe, Lss, Rss, Lrs, Rrs, Ltf, Rtf, Ltr, Rtr])
    }
    /// The layout CFA4's mixer uses for a channel count: 1, 2, 6, 8, 10 or 12.
    pub fn for_channels(channels: usize) -> Option<Self> {
        Some(match channels {
            1 => Self::mono(),
            2 => Self::stereo(),
            6 => Self::surround_5_1(),
            8 => Self::surround_7_1(),
            10 => Self::surround_7_1_2(),
            12 => Self::surround_7_1_4(),
            _ => return None,
        })
    }
    pub fn channels(&self) -> usize {
        self.0.len()
    }
}

/// How [`VirtualSpeakers`] are made.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VirtualSpeakersConfig {
    /// The LFE's level in both ears, in dB (fixed, e.g. -10 dB; headphones need less than a
    /// cinema's +10 dB LFE).
    pub lfe_db: f32,
    /// The longest block processed in one step.
    pub max_frames: usize,
}

impl Default for VirtualSpeakersConfig {
    fn default() -> Self {
        VirtualSpeakersConfig { lfe_db: -10.0, max_frames: 1024 }
    }
}

/// A second-order low-pass (RBJ cookbook, Q = 1/√2), transposed direct form II.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LowPass {
    b: [f32; 3],
    a: [f32; 2],
    z: [f32; 2],
}

impl LowPass {
    pub(crate) fn new(freq: f32, rate: f32) -> Self {
        let w = 2.0 * std::f32::consts::PI * freq / rate;
        let alpha = w.sin() / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let c = w.cos();
        let a0 = 1.0 + alpha;
        let b1 = (1.0 - c) / a0;
        LowPass { b: [b1 / 2.0, b1, b1 / 2.0], a: [-2.0 * c / a0, (1.0 - alpha) / a0], z: [0.0; 2] }
    }
    pub(crate) fn run(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = crate::dsp::flush(self.b[2] * x - self.a[1] * y);
        self.z[0] = crate::dsp::flush(self.z[0]);
        y
    }
    pub(crate) fn reset(&mut self) {
        self.z = [0.0; 2];
    }
}

enum Channel {
    Full(Box<Binaural>, [f32; 3]),
    Lfe(LowPass),
}

/// A channel layout rendered to two ears.
pub struct VirtualSpeakers {
    layout: SpeakerLayout,
    channels: Vec<Channel>,
    lfe_gain: f32,
    max_frames: usize,
    left: Vec<f32>,
    right: Vec<f32>,
    zeros: Vec<f32>,
}

impl VirtualSpeakers {
    /// Virtual loudspeakers for `layout` with `hrtf`.
    pub fn new(hrtf: Arc<HrtfSet>, layout: SpeakerLayout, config: VirtualSpeakersConfig) -> Result<Self> {
        if layout.0.is_empty() {
            return Err(Error::Invalid("a layout needs at least one channel"));
        }
        if !config.lfe_db.is_finite() {
            return Err(Error::Invalid("LFE level must be finite"));
        }
        let bc = BinauralConfig { max_frames: config.max_frames, near_field: false, ..BinauralConfig::default() };
        let rate = hrtf.sample_rate() as f32;
        let distance = hrtf.distance();
        let mut channels = Vec::with_capacity(layout.0.len());
        for s in &layout.0 {
            channels.push(match s.direction() {
                Some((a, e)) => Channel::Full(Box::new(Binaural::new(hrtf.clone(), bc)?), position_of(a, e, distance)),
                None => Channel::Lfe(LowPass::new(120.0, rate)),
            });
        }
        Ok(VirtualSpeakers {
            layout,
            channels,
            lfe_gain: 10f32.powf(config.lfe_db / 20.0),
            max_frames: config.max_frames,
            left: vec![0.0; config.max_frames],
            right: vec![0.0; config.max_frames],
            zeros: vec![0.0; config.max_frames],
        })
    }

    pub fn layout(&self) -> &SpeakerLayout {
        &self.layout
    }

    /// Forgets all history.
    pub fn reset(&mut self) {
        for c in &mut self.channels {
            match c {
                Channel::Full(b, _) => b.reset(),
                Channel::Lfe(f) => f.reset(),
            }
        }
    }

    /// Renders one block: `inputs[c]` is channel `c` of the layout (missing channels are silent),
    /// and the two ears are written over `left` and `right`. The frame count is the shortest of
    /// `left` and `right`; a shorter input counts as silence after its end.
    pub fn process(&mut self, inputs: &[&[f32]], left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len());
        left[..frames].fill(0.0);
        right[..frames].fill(0.0);
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(self.max_frames);
            for (c, ch) in self.channels.iter_mut().enumerate() {
                let input = inputs.get(c).and_then(|i| i.get(done..done + n));
                // past an input's end: silence, so the channel's history still moves
                let input = input.unwrap_or(&self.zeros[..n]);
                match ch {
                    Channel::Full(b, pos) => {
                        b.process(*pos, 1.0, input, &mut self.left[..n], &mut self.right[..n]);
                        for i in 0..n {
                            left[done + i] += self.left[i];
                            right[done + i] += self.right[i];
                        }
                    }
                    Channel::Lfe(f) => {
                        for i in 0..n {
                            let y = f.run(input[i]) * self.lfe_gain;
                            left[done + i] += y;
                            right[done + i] += y;
                        }
                    }
                }
            }
            done += n;
        }
    }
}
