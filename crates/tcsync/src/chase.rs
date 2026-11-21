//! Chase: follow an external timecode clock (LTC or MTC) with the local player.
//!
//! The device layer turns each received frame into an [`Observation`]: the external position (as
//! time of day) and the local monotonic time at which the clock was at that position. [`Chase`]
//! turns observations and the player's state into [`ChaseAction`]s:
//!
//! `NoSignal → Locating → Locked ↔ Freewheel → NoSignal`, plus `Scrubbing` for reverse or
//! off-speed play.
//!
//! - **Locate:** after `lock_frames` consecutive frames that agree with each other, play from the
//!   external position plus the offset.
//! - **Locked:** while playing, the player's position is compared with the expected one; a drift
//!   larger than `resync_threshold` that lasts `resync_after` gives a new locate (version 1 of the
//!   plan: re-seek, no varispeed).
//! - **Freewheel:** no new frame for two frames (a repeated frame counts as none): keep playing on
//!   the estimated clock; after `freewheel` stop.
//! - **Jumps:** a frame outside `accept_window` of the expected position starts a new lock.
//! - **Scrubbing:** reverse play or a speed outside 0.9×–1.1× gives locates without playback.
//!
//! The rules for dropouts, repeated frames and the acceptance window follow how lighting consoles
//! read LTC. Positions wrap at 24 hours.

use cfa4_time::{FrameRate, Tick};

/// One day of timecode in ticks.
fn day() -> i64 {
    Tick::from_units(86_400, 1).0
}

/// `a - b` on the 24-hour circle, in `[-12 h, 12 h)`.
fn wrap_diff(a: Tick, b: Tick) -> i64 {
    let d = day();
    let half = d / 2;
    (a.0.wrapping_sub(b.0)).wrapping_add(half).rem_euclid(d) - half
}

fn wrap_pos(t: i64) -> Tick {
    Tick(t.rem_euclid(day()))
}

/// Chase settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChaseConfig {
    /// Frame rate of the external timecode (for frame-based thresholds).
    pub rate: FrameRate,
    /// Added to the external position to get the sequence position.
    pub offset: Tick,
    /// Consecutive agreeing frames before locking.
    pub lock_frames: u32,
    /// Drift (in frames) the player may have before a re-seek.
    pub resync_threshold_frames: i64,
    /// How long the drift must last before a re-seek.
    pub resync_after: Tick,
    /// How long to keep playing without signal before stopping.
    pub freewheel: Tick,
    /// Frames further than this from the expected position start a new lock.
    pub accept_window: Tick,
}

impl ChaseConfig {
    pub fn new(rate: FrameRate) -> Self {
        let rate = rate.sane();
        let frame = rate.frame_duration();
        ChaseConfig {
            rate,
            offset: Tick(0),
            lock_frames: 3,
            resync_threshold_frames: 1,
            resync_after: Tick(frame.0.saturating_mul(5)),
            freewheel: Tick(frame.0.saturating_mul(10)),
            accept_window: Tick::from_units(2, 1),
        }
    }
}

/// A received frame: the external clock was at `position` (time of day) at local time `at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observation {
    pub at: Tick,
    pub position: Tick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChaseState {
    NoSignal,
    Locating,
    Locked,
    Freewheel,
    Scrubbing,
}

/// What the player should do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChaseAction {
    /// Go to sequence position `to` (as of the current local time) and play or hold.
    Locate { to: Tick, play: bool },
    /// Stop playback where it is.
    Stop,
}

/// Status for the UI and the control channel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChaseStatus {
    pub state: ChaseState,
    /// Last external position received.
    pub position: Option<Tick>,
    /// Estimated external speed (1.0 = real time, negative = reverse).
    pub speed: f64,
    /// Player position minus expected position at the last check.
    pub drift: Option<Tick>,
}

#[derive(Clone, Debug)]
pub struct Chase {
    cfg: ChaseConfig,
    state: ChaseState,
    last: Option<Observation>,
    agree: u32,
    speed: f64,
    drift: Option<Tick>,
    drift_since: Option<Tick>,
}

impl Chase {
    pub fn new(cfg: ChaseConfig) -> Self {
        Chase { cfg, state: ChaseState::NoSignal, last: None, agree: 0, speed: 1.0, drift: None, drift_since: None }
    }

    pub fn config(&self) -> &ChaseConfig {
        &self.cfg
    }

    pub fn set_config(&mut self, cfg: ChaseConfig) {
        self.cfg = cfg;
    }

    pub fn status(&self) -> ChaseStatus {
        ChaseStatus { state: self.state, position: self.last.map(|o| o.position), speed: self.speed, drift: self.drift }
    }

    fn frame(&self) -> i64 {
        self.cfg.rate.frame_duration().0.max(1)
    }

    /// The external position expected at local time `now`.
    fn expected(&self, now: Tick) -> Option<Tick> {
        let o = self.last?;
        let dt = now.0.saturating_sub(o.at.0) as f64;
        Some(wrap_pos(o.position.0.saturating_add((dt * self.speed) as i64)))
    }

    fn sequence_position(&self, external: Tick) -> Tick {
        Tick(external.0.saturating_add(self.cfg.offset.0))
    }

    /// Feeds a received frame.
    pub fn observe(&mut self, obs: Observation, out: &mut Vec<ChaseAction>) {
        let Some(prev) = self.last else {
            self.last = Some(obs);
            self.agree = 1;
            self.state = ChaseState::Locating;
            return;
        };
        if obs.position == prev.position {
            // A repeated frame is no new information: the dropout timer keeps running.
            return;
        }
        let dt = obs.at.0.saturating_sub(prev.at.0);
        let moved = wrap_diff(obs.position, prev.position);
        let expected = self.expected(obs.at).unwrap_or(obs.position);
        let jump = wrap_diff(obs.position, expected).abs() > self.cfg.accept_window.0;
        let speed = if dt > 0 { moved as f64 / dt as f64 } else { self.speed };
        self.last = Some(obs);
        if jump || dt <= 0 {
            self.agree = 1;
            self.speed = 1.0;
            if self.state != ChaseState::NoSignal {
                self.state = ChaseState::Locating;
            }
            return;
        }
        // Smooth the speed estimate; a frame jitter of the device must not look like varispeed.
        self.speed = if self.agree <= 1 { speed } else { 0.7 * self.speed + 0.3 * speed };
        self.agree = self.agree.saturating_add(1);
        let normal = (0.9..=1.1).contains(&self.speed);
        if !normal {
            self.state = ChaseState::Scrubbing;
            self.drift_since = None;
            out.push(ChaseAction::Locate { to: self.sequence_position(obs.position), play: false });
            return;
        }
        match self.state {
            ChaseState::NoSignal | ChaseState::Locating | ChaseState::Scrubbing => {
                if self.agree >= self.cfg.lock_frames {
                    self.state = ChaseState::Locked;
                    self.drift_since = None;
                    out.push(ChaseAction::Locate { to: self.sequence_position(obs.position), play: true });
                } else {
                    self.state = ChaseState::Locating;
                }
            }
            ChaseState::Freewheel => self.state = ChaseState::Locked,
            ChaseState::Locked => {}
        }
    }

    /// Periodic check at local time `now` with the player's position (`None` while stopped).
    pub fn tick(&mut self, now: Tick, playhead: Option<Tick>, out: &mut Vec<ChaseAction>) {
        let Some(last) = self.last else {
            return;
        };
        let silent = now.0.saturating_sub(last.at.0);
        let frame = self.frame();
        if silent > self.cfg.freewheel.0.max(2 * frame) {
            if matches!(self.state, ChaseState::Locked | ChaseState::Freewheel) {
                out.push(ChaseAction::Stop);
            }
            self.state = ChaseState::NoSignal;
            self.last = None;
            self.agree = 0;
            self.drift = None;
            self.drift_since = None;
            return;
        }
        if silent > 2 * frame {
            if self.state == ChaseState::Locked {
                self.state = ChaseState::Freewheel;
            }
            return;
        }
        if self.state != ChaseState::Locked {
            return;
        }
        let Some(expected) = self.expected(now) else {
            return;
        };
        let target = self.sequence_position(expected);
        let Some(ph) = playhead else {
            // Locked but the player stopped (the user pressed stop, or a seek is pending): lock again.
            self.drift = None;
            self.drift_since = None;
            out.push(ChaseAction::Locate { to: target, play: true });
            return;
        };
        let drift = wrap_diff(ph, target);
        self.drift = Some(Tick(drift));
        if drift.abs() > self.cfg.resync_threshold_frames.saturating_mul(frame) {
            let since = *self.drift_since.get_or_insert(now);
            if now.0.saturating_sub(since.0) >= self.cfg.resync_after.0 {
                self.drift_since = None;
                out.push(ChaseAction::Locate { to: target, play: true });
            }
        } else {
            self.drift_since = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: FrameRate = FrameRate::FPS_25;

    fn f(n: i64) -> Tick {
        RATE.tick_of(n)
    }

    /// Feeds frames `from..to` of a real-time external clock that started at position `start`.
    fn feed(c: &mut Chase, start: i64, from: i64, to: i64, out: &mut Vec<ChaseAction>) {
        for n in from..to {
            c.observe(Observation { at: f(n), position: f(start + n) }, out);
        }
    }

    #[test]
    fn locks_after_three_frames_and_applies_offset() {
        let mut cfg = ChaseConfig::new(RATE);
        cfg.offset = Tick(-f(3600 * 25).0); // external 01:00:00:00 = sequence 0
        let mut c = Chase::new(cfg);
        let mut out = Vec::new();
        feed(&mut c, 3600 * 25, 0, 2, &mut out);
        assert!(out.is_empty());
        assert_eq!(c.status().state, ChaseState::Locating);
        feed(&mut c, 3600 * 25, 2, 3, &mut out);
        assert_eq!(out, vec![ChaseAction::Locate { to: f(2), play: true }]);
        assert_eq!(c.status().state, ChaseState::Locked);
    }

    #[test]
    fn small_drift_is_tolerated_large_drift_reseeks() {
        let mut c = Chase::new(ChaseConfig::new(RATE));
        let mut out = Vec::new();
        feed(&mut c, 0, 0, 3, &mut out);
        out.clear();
        // Player half a frame late: no action.
        for n in 3..40 {
            c.observe(Observation { at: f(n), position: f(n) }, &mut out);
            c.tick(f(n), Some(Tick(f(n).0 - f(1).0 / 2)), &mut out);
        }
        assert!(out.is_empty());
        // Player 3 frames late: re-seek after 5 frames of drift; the player then follows.
        let mut late = 3;
        for n in 40..60 {
            c.observe(Observation { at: f(n), position: f(n) }, &mut out);
            c.tick(f(n), Some(f(n - late)), &mut out);
            if !out.is_empty() {
                late = 0;
            }
        }
        assert_eq!(out, vec![ChaseAction::Locate { to: f(45), play: true }]);
    }

    #[test]
    fn dropout_freewheels_then_stops_and_relocks() {
        let mut c = Chase::new(ChaseConfig::new(RATE));
        let mut out = Vec::new();
        feed(&mut c, 0, 0, 10, &mut out);
        out.clear();
        c.tick(f(12), Some(f(12)), &mut out);
        assert_eq!(c.status().state, ChaseState::Freewheel);
        assert!(out.is_empty());
        // Signal returns within the freewheel time: locked again, no locate.
        c.observe(Observation { at: f(13), position: f(13) }, &mut out);
        assert_eq!(c.status().state, ChaseState::Locked);
        assert!(out.is_empty());
        // Signal gone for longer than the freewheel time: stop.
        c.tick(f(30), Some(f(30)), &mut out);
        assert_eq!(out, vec![ChaseAction::Stop]);
        assert_eq!(c.status().state, ChaseState::NoSignal);
        out.clear();
        feed(&mut c, 0, 100, 103, &mut out);
        assert_eq!(out, vec![ChaseAction::Locate { to: f(102), play: true }]);
    }

    #[test]
    fn repeated_frame_counts_as_dropout() {
        let mut c = Chase::new(ChaseConfig::new(RATE));
        let mut out = Vec::new();
        feed(&mut c, 0, 0, 5, &mut out);
        for n in 5..9 {
            c.observe(Observation { at: f(n), position: f(4) }, &mut out);
        }
        c.tick(f(8), Some(f(8)), &mut out);
        assert_eq!(c.status().state, ChaseState::Freewheel);
    }

    #[test]
    fn jump_relocks_at_the_new_position() {
        let mut c = Chase::new(ChaseConfig::new(RATE));
        let mut out = Vec::new();
        feed(&mut c, 0, 0, 5, &mut out);
        out.clear();
        // The external clock jumps 10 minutes ahead.
        let jump = 10 * 60 * 25;
        feed(&mut c, jump, 5, 7, &mut out);
        assert_eq!(c.status().state, ChaseState::Locating);
        assert!(out.is_empty());
        feed(&mut c, jump, 7, 8, &mut out);
        assert_eq!(out, vec![ChaseAction::Locate { to: f(jump + 7), play: true }]);
    }

    #[test]
    fn reverse_and_fast_play_scrub() {
        let mut c = Chase::new(ChaseConfig::new(RATE));
        let mut out = Vec::new();
        for n in 0..4 {
            c.observe(Observation { at: f(n), position: f(1000 - n) }, &mut out);
        }
        assert_eq!(c.status().state, ChaseState::Scrubbing);
        assert!(c.status().speed < 0.0);
        assert!(out.iter().all(|a| matches!(a, ChaseAction::Locate { play: false, .. })));
        assert_eq!(out.last(), Some(&ChaseAction::Locate { to: f(997), play: false }));
    }

    #[test]
    fn midnight_wrap() {
        let mut c = Chase::new(ChaseConfig::new(RATE));
        let mut out = Vec::new();
        let last = 24 * 3600 * 25 - 2;
        for (k, n) in [last, last + 1, 0, 1].into_iter().enumerate() {
            c.observe(Observation { at: f(k as i64), position: f(n) }, &mut out);
        }
        assert_eq!(c.status().state, ChaseState::Locked);
        assert!((c.status().speed - 1.0).abs() < 1e-9);
        assert_eq!(out, vec![ChaseAction::Locate { to: f(0), play: true }]);
    }

    #[test]
    fn hostile_times_never_panic() {
        let mut c = Chase::new(ChaseConfig::new(FrameRate { num: 0, den: 0 }));
        let mut out = Vec::new();
        for (at, pos) in [(i64::MAX, i64::MIN), (i64::MIN, i64::MAX), (0, 0), (-5, 7), (i64::MAX, 3)] {
            c.observe(Observation { at: Tick(at), position: Tick(pos) }, &mut out);
            c.tick(Tick(at), Some(Tick(pos)), &mut out);
            c.tick(Tick(i64::MAX), None, &mut out);
        }
    }
}
