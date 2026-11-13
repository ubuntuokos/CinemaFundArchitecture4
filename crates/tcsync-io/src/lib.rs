//! External timecode sync devices (plan TC-5).
//!
//! [`cfa4_tcsync`] holds the pure formats and the chase logic; this crate connects them to
//! hardware and to the player:
//!
//! - **LTC in** ([`ltc::LtcReader`], cpal input): decoded frames become chase observations.
//! - **LTC out** ([`ltc::LtcGenerator`], cpal output): frames that follow the player's transport.
//! - **MIDI Timecode** ([`mtc::MtcReader`], [`mtc::MtcScheduler`], ALSA sequencer): quarter
//!   frames and full-frame locates, in and out, on a sequencer client named `CFA4`.
//! - **[`SyncManager`]** opens what the settings ask for, feeds the chase and returns what the
//!   player should do.
//!
//! The pure parts take times and sample blocks as arguments, so they are tested without devices.
//! Device problems (missing device, no ALSA sequencer, a channel the device does not have) become
//! errors in [`SyncStatus`], never a crash. All positions are [`Tick`]s; the external clock is
//! kept as time of day (wrapping at 24 hours).

pub mod alsa_seq;
pub mod cpal_dev;
pub mod ltc;
pub mod manager;
pub mod mtc;

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use cfa4_tcsync::chase::Observation;
use cfa4_time::{FrameRate, Tick, fields_to_frames};

pub use alsa_seq::Hardware;
pub use manager::{Backend, Device, Source, SyncManager, SyncSettings, SyncStatus};

/// The local monotonic clock all devices and the chase share.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    epoch: Instant,
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock {
    pub fn new() -> Self {
        Clock { epoch: Instant::now() }
    }

    /// Time since the clock was made.
    pub fn now(&self) -> Tick {
        duration_ticks(self.epoch.elapsed())
    }
}

/// A duration in ticks (saturating).
pub fn duration_ticks(d: Duration) -> Tick {
    Tick::from_units(i64::try_from(d.as_nanos()).unwrap_or(i64::MAX / 2), 1_000_000_000)
}

/// `samples` at `sample_rate` in ticks.
pub fn samples_ticks(samples: u64, sample_rate: u32) -> Tick {
    Tick::from_units(i64::try_from(samples).unwrap_or(i64::MAX / 2), i64::from(sample_rate.max(1)))
}

/// What the player is doing, as last reported by it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transport {
    pub playing: bool,
    /// Sequence position at local time `at`.
    pub position: Tick,
    pub at: Tick,
    /// Play speed while playing (1.0 = normal).
    pub speed: f64,
}

impl Default for Transport {
    fn default() -> Self {
        Transport { playing: false, position: Tick(0), at: Tick(0), speed: 1.0 }
    }
}

impl Transport {
    /// The sequence position at local time `now`.
    pub fn position_at(&self, now: Tick) -> Tick {
        if !self.playing {
            return self.position;
        }
        let dt = now.0.saturating_sub(self.at.0);
        if self.speed == 1.0 {
            return Tick(self.position.0.saturating_add(dt));
        }
        let speed = if self.speed.is_finite() { self.speed } else { 0.0 };
        Tick(self.position.0.saturating_add((dt as f64 * speed) as i64))
    }

    /// Playing forward at normal speed (what LTC and MTC output follow; anything else is a
    /// shuttle, during which the outputs hold).
    pub fn rolling(&self) -> bool {
        self.playing && (0.98..=1.02).contains(&self.speed)
    }
}

/// The transport, shared with the output devices' threads.
pub type SharedTransport = Arc<Mutex<Transport>>;

/// Locks a mutex, recovering the data of a thread that panicked while holding it.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// What the input devices send to the manager.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// A received frame (external time of day).
    Frame(Observation),
    /// A locate without play (MIDI full frame).
    Locate(Observation),
    /// The device stopped with an error.
    Error(String),
}

/// The external clock's rate and timecode counting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExternalRate {
    pub rate: FrameRate,
    pub drop_frame: bool,
}

impl ExternalRate {
    pub fn new(rate: FrameRate, drop_frame: bool) -> Self {
        let rate = rate.sane();
        ExternalRate { rate, drop_frame: drop_frame && rate.supports_drop_frame() }
    }

    /// Frames in one day of timecode.
    pub fn frames_per_day(&self) -> i64 {
        fields_to_frames(24, 0, 0, 0, self.rate, self.drop_frame).max(1)
    }

    /// The frame (wrapped into one day) that time of day `t` falls in.
    pub fn frame_at(&self, t: Tick) -> i64 {
        self.rate.frame_at(wrap_day(t)).rem_euclid(self.frames_per_day())
    }
}

/// One day in ticks.
pub fn day() -> i64 {
    Tick::from_units(86_400, 1).0
}

/// `t` wrapped into `[0, 24 h)`.
pub fn wrap_day(t: Tick) -> Tick {
    Tick(t.0.rem_euclid(day()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_runs_at_its_speed() {
        let t = Transport { playing: true, position: Tick(1000), at: Tick(100), speed: 1.0 };
        assert_eq!(t.position_at(Tick(600)), Tick(1500));
        let half = Transport { speed: 0.5, ..t };
        assert_eq!(half.position_at(Tick(600)), Tick(1250));
        let stopped = Transport { playing: false, ..t };
        assert_eq!(stopped.position_at(Tick(600)), Tick(1000));
        assert!(t.rolling() && !half.rolling() && !stopped.rolling());
        let bad = Transport { speed: f64::NAN, ..t };
        assert_eq!(bad.position_at(Tick(600)), Tick(1000));
    }

    #[test]
    fn day_wrapping_frames() {
        let r = ExternalRate::new(FrameRate::FPS_25, true);
        assert!(!r.drop_frame, "25 fps has no drop-frame");
        assert_eq!(r.frames_per_day(), 25 * 86_400);
        assert_eq!(r.frame_at(Tick(-1)), 25 * 86_400 - 1);
        assert_eq!(r.frame_at(FrameRate::FPS_25.tick_of(25 * 86_400 + 3)), 3);
        assert_eq!(samples_ticks(48_000, 48_000), Tick::from_units(1, 1));
    }
}
