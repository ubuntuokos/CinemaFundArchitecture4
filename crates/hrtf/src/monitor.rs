// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the CFA4 contributors

//! A channel mix heard through headphones block after block, as a monitor plays it
//! (`docs/plans/CFA4-BINAURAL-MONITOR-001.md` §3.3, work package B-03).
//!
//! [`ChannelMonitor`] takes the blocks a monitor plays, each a mix in one of the mixer's layouts
//! (1, 2, 6, 8, 10 or 12 channels, [`SpeakerLayout::for_channels`]), and renders every channel as
//! a bed of a [`SceneRenderer`] at its loudspeaker's direction. The scene keeps its history while
//! the blocks follow each other; a block that does not start where the last one ended (a seek, a
//! new play) or that changes the layout starts it over. [`MonitorConfig::room`] chooses the
//! headphone mode: 0 is dry ([`HeadphoneMode::Near`]), up to 0.5 the small room
//! ([`HeadphoneMode::Mid`]), above that the larger one ([`HeadphoneMode::Far`]).

use std::sync::Arc;

use crate::scene::{HeadphoneMode, Placement, SceneConfig, SceneRenderer, SceneState};
use crate::{Error, HrtfSet, Result, SpeakerLayout};

/// How a [`ChannelMonitor`] renders.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonitorConfig {
    /// How much room the listener hears, 0 (none) ..= 1.
    pub room: f32,
    /// The LFE's level in both ears, in dB (-30 ..= 0).
    pub lfe_db: f32,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        // the plan's decision 6: the room is on unless turned off
        MonitorConfig { room: 0.35, lfe_db: -10.0 }
    }
}

impl MonitorConfig {
    /// The headphone mode of [`MonitorConfig::room`].
    pub fn mode(&self) -> HeadphoneMode {
        if self.room <= 0.0 {
            HeadphoneMode::Near
        } else if self.room <= 0.5 {
            HeadphoneMode::Mid
        } else {
            HeadphoneMode::Far
        }
    }

    fn check(&self) -> Result<()> {
        if !(self.room.is_finite() && (0.0..=1.0).contains(&self.room)) {
            return Err(Error::Invalid("room must be 0 to 1"));
        }
        if !(self.lfe_db.is_finite() && (-30.0..=0.0).contains(&self.lfe_db)) {
            return Err(Error::Invalid("LFE level must be -30 to 0 dB"));
        }
        Ok(())
    }
}

/// A monitor's channel mix through headphones (see the module notes).
pub struct ChannelMonitor {
    scene: SceneRenderer,
    config: MonitorConfig,
    /// The channel count whose beds are placed.
    channels: Option<usize>,
    /// The sample the next block must start at to continue, in the caller's clock.
    next: Option<i64>,
    /// The scene's own clock (it starts at 0 with each start over).
    clock: u64,
}

impl ChannelMonitor {
    /// A monitor with `hrtf` (at the rate of the blocks to come).
    pub fn new(hrtf: Arc<HrtfSet>, config: MonitorConfig) -> Result<Self> {
        config.check()?;
        let scene = SceneRenderer::new(hrtf, SceneConfig { lfe_db: config.lfe_db, room: config.room > 0.0, ..SceneConfig::default() })?;
        Ok(ChannelMonitor { scene, config, channels: None, next: None, clock: 0 })
    }

    pub fn config(&self) -> MonitorConfig {
        self.config
    }

    /// The rate the monitor renders at (its HRTF set's).
    pub fn sample_rate(&self) -> u32 {
        self.scene.hrtf().sample_rate()
    }

    /// The HRTF set.
    pub fn hrtf(&self) -> &Arc<HrtfSet> {
        self.scene.hrtf()
    }

    /// Whether the monitor plays `channels` channels (a layout of the mixer).
    pub fn supports(channels: usize) -> bool {
        SpeakerLayout::for_channels(channels).is_some()
    }

    /// Forgets the history: the next block starts over.
    pub fn reset(&mut self) {
        self.next = None;
    }

    /// Renders the block starting at sample `start` of the caller's clock (`inputs[c]` is channel
    /// `c`; a short channel is silence after its end) into `left` and `right` (overwritten; the
    /// frame count is the shorter of the two). A channel count the mixer has no layout for is an
    /// error, and the ears are then silent.
    pub fn process(&mut self, start: i64, inputs: &[&[f32]], left: &mut [f32], right: &mut [f32]) -> Result<()> {
        let frames = left.len().min(right.len());
        let Some(layout) = SpeakerLayout::for_channels(inputs.len()) else {
            left[..frames].fill(0.0);
            right[..frames].fill(0.0);
            self.next = None;
            return Err(Error::Invalid("the monitor plays 1, 2, 6, 8, 10 or 12 channels"));
        };
        if self.next != Some(start) || self.channels != Some(inputs.len()) {
            self.scene.reset();
            self.clock = 0;
            let mode = self.config.mode();
            for (i, &speaker) in layout.0.iter().enumerate() {
                self.scene.schedule(0, i, SceneState { placement: Placement::Speaker(speaker), gain: 1.0, mode }, 0)?;
            }
            self.channels = Some(inputs.len());
        }
        self.scene.render(self.clock, inputs, &mut left[..frames], &mut right[..frames]);
        self.clock = self.clock.saturating_add(frames as u64);
        self.next = Some(start.saturating_add(frames as i64));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(room: f32) -> ChannelMonitor {
        ChannelMonitor::new(Arc::new(HrtfSet::builtin(48_000).expect("set")), MonitorConfig { room, lfe_db: -10.0 }).expect("monitor")
    }

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut x = seed.max(1);
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as f32 / u32::MAX as f32 - 0.5
            })
            .collect()
    }

    #[test]
    fn consecutive_blocks_equal_one_block() {
        let input = [noise(4_800, 1), noise(4_800, 2)];
        let whole = {
            let mut m = monitor(0.35);
            let (mut l, mut r) = (vec![0.0; 4_800], vec![0.0; 4_800]);
            m.process(100, &[&input[0], &input[1]], &mut l, &mut r).expect("block");
            (l, r)
        };
        let mut m = monitor(0.35);
        let (mut l, mut r) = (vec![0.0; 4_800], vec![0.0; 4_800]);
        for (a, b) in [(0, 1_000), (1_000, 2_500), (2_500, 4_800)] {
            let ins = [&input[0][a..b], &input[1][a..b]];
            m.process(100 + a as i64, &ins, &mut l[a..b], &mut r[a..b]).expect("block");
        }
        let worst = whole.0.iter().zip(&l).chain(whole.1.iter().zip(&r)).map(|(x, y)| (x - y).abs()).fold(0.0_f32, f32::max);
        assert!(worst < 1e-6, "blocks differ from one block by {worst}");
    }

    #[test]
    fn a_jump_starts_over() {
        let input = [noise(2_000, 3), noise(2_000, 4)];
        let ins = [&input[0][..], &input[1][..]];
        let fresh = {
            let mut m = monitor(0.0);
            let (mut l, mut r) = (vec![0.0; 2_000], vec![0.0; 2_000]);
            m.process(9_000, &ins, &mut l, &mut r).expect("block");
            l
        };
        let mut m = monitor(0.0);
        let (mut l, mut r) = (vec![0.0; 2_000], vec![0.0; 2_000]);
        m.process(0, &ins, &mut l, &mut r).expect("block");
        m.process(9_000, &ins, &mut l, &mut r).expect("block after a jump");
        assert_eq!(fresh, l, "a jump keeps no history");
    }

    #[test]
    fn a_7_1_4_mix_renders_well_ahead_of_real_time() {
        let mut m = monitor(0.35);
        let input: Vec<Vec<f32>> = (0..12).map(|c| noise(48_000, 10 + c)).collect();
        let (mut l, mut r) = (vec![0.0; 1_024], vec![0.0; 1_024]);
        let started = std::time::Instant::now();
        let mut at = 0;
        while at < 48_000 {
            let n = 1_024.min(48_000 - at);
            let ins: Vec<&[f32]> = input.iter().map(|c| &c[at..at + n]).collect();
            m.process(at as i64, &ins, &mut l[..n], &mut r[..n]).expect("block");
            at += n;
        }
        let took = started.elapsed().as_secs_f64();
        println!("1 s of 7.1.4 through headphones: {:.1} ms", took * 1e3);
        assert!(took < 1.0, "slower than real time: {took} s");
    }

    #[test]
    fn a_bad_layout_or_setting_is_an_error_not_a_crash() {
        let mut m = monitor(1.0);
        let one = vec![0.5; 64];
        let (mut l, mut r) = (vec![1.0; 64], vec![1.0; 64]);
        assert!(m.process(0, &[&one, &one, &one], &mut l, &mut r).is_err());
        assert!(l.iter().chain(&r).all(|&v| v == 0.0));
        let set = Arc::new(HrtfSet::builtin(48_000).expect("set"));
        for config in [MonitorConfig { room: f32::NAN, lfe_db: -10.0 }, MonitorConfig { room: 0.5, lfe_db: 6.0 }] {
            assert!(ChannelMonitor::new(set.clone(), config).is_err());
        }
        assert_eq!(MonitorConfig { room: 0.0, lfe_db: 0.0 }.mode(), HeadphoneMode::Near);
        assert_eq!(MonitorConfig::default().mode(), HeadphoneMode::Mid);
        assert_eq!(MonitorConfig { room: 0.8, lfe_db: 0.0 }.mode(), HeadphoneMode::Far);
    }
}
