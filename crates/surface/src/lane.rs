//! Per-device input lanes (F0 spec §3, decision D8).
//!
//! Triggers (`Button`, `ProgramChange`, `SysEx`, `Jog`, `Shuttle`, `Touch`, and any other
//! non-continuous input that is not monitor-only raw) are never dropped: a full lane blocks the
//! reader and [`DeviceCounters::stalls`] grows. Continuous values (`Absolute`, `Relative`,
//! `PitchBend`) keep the last value per control; a relative control sums its deltas, and each
//! overwrite increments [`DeviceCounters::coalesced`]. `seq` grows by one for every queued input
//! of a device. The starting capacity is 256; the final size waits on a measurement (N-08).

use cfa4_surface_model::{ControlId, DeviceCounters, DeviceId, MonoNs, SurfaceInput};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};

/// Trigger events waiting at once. A starting value; measured before it is final (N-08, D8).
pub const TRIGGER_CAPACITY: usize = 256;

/// One input waiting for the session, in device order (`seq`).
#[derive(Clone, Debug, PartialEq)]
pub struct Queued {
    pub device: DeviceId,
    pub seq: u64,
    pub at: MonoNs,
    pub input: SurfaceInput,
    pub raw: Vec<u8>,
}

#[derive(Debug, Default)]
struct Atomics {
    received: AtomicU64,
    dispatched: AtomicU64,
    coalesced: AtomicU64,
    stalls: AtomicU64,
    gaps: AtomicU64,
    errors: AtomicU64,
}

impl Atomics {
    fn snapshot(&self) -> DeviceCounters {
        DeviceCounters {
            received: self.received.load(Ordering::Relaxed),
            dispatched: self.dispatched.load(Ordering::Relaxed),
            coalesced: self.coalesced.load(Ordering::Relaxed),
            stalls: self.stalls.load(Ordering::Relaxed),
            gaps: self.gaps.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
        }
    }
}

/// What a continuous value is keyed by. Relative deltas of the same control add up.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ContKey {
    Control(ControlId),
    Bend(u8),
}

fn cont_key(input: &SurfaceInput) -> Option<ContKey> {
    match input {
        SurfaceInput::Absolute { control, .. } | SurfaceInput::Relative { control, .. } => Some(ContKey::Control(control.clone())),
        SurfaceInput::PitchBend { channel, .. } => Some(ContKey::Bend(*channel)),
        _ => None,
    }
}

struct DeviceLane {
    seq: AtomicU64,
    counters: Atomics,
}

/// The send half, held by a reader thread.
#[derive(Clone)]
pub struct LaneIn {
    triggers: SyncSender<Queued>,
    continuous: Arc<Mutex<HashMap<(String, ContKey), Queued>>>,
    devices: Arc<Mutex<HashMap<String, DeviceLane>>>,
    kernel_dropped: Arc<AtomicU64>,
}

/// The receive half, held by the session thread.
pub struct LaneOut {
    triggers: Receiver<Queued>,
    continuous: Arc<Mutex<HashMap<(String, ContKey), Queued>>>,
    devices: Arc<Mutex<HashMap<String, DeviceLane>>>,
    /// Kernel overruns of the ALSA queue (not a process-side drop). HID leaves this at 0.
    pub kernel_dropped: Arc<AtomicU64>,
}

/// Both halves of one lane.
pub fn lane() -> (LaneIn, LaneOut) {
    let (tx, rx) = sync_channel(TRIGGER_CAPACITY);
    let continuous = Arc::new(Mutex::new(HashMap::new()));
    let devices = Arc::new(Mutex::new(HashMap::new()));
    let kernel_dropped = Arc::new(AtomicU64::new(0));
    (
        LaneIn { triggers: tx, continuous: continuous.clone(), devices: devices.clone(), kernel_dropped: kernel_dropped.clone() },
        LaneOut { triggers: rx, continuous, devices, kernel_dropped },
    )
}

fn devices(map: &Mutex<HashMap<String, DeviceLane>>) -> std::sync::MutexGuard<'_, HashMap<String, DeviceLane>> {
    map.lock().unwrap_or_else(|e| e.into_inner())
}

impl LaneIn {
    fn device<'a>(&self, id: &str, guard: &'a mut HashMap<String, DeviceLane>) -> &'a DeviceLane {
        guard.entry(id.to_string()).or_insert_with(|| DeviceLane { seq: AtomicU64::new(0), counters: Atomics::default() })
    }

    /// Queue one input. Triggers block when the lane is full (`stalls` grows). Continuous values
    /// replace the previous one for that control (`coalesced` grows). Returns the `seq`.
    pub fn push(&self, device: DeviceId, at: MonoNs, input: SurfaceInput, raw: Vec<u8>) -> u64 {
        let mut guard = devices(&self.devices);
        let lane = self.device(&device.0, &mut guard);
        lane.counters.received.fetch_add(1, Ordering::Relaxed);
        let seq = lane.seq.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        let queued = Queued { device: device.clone(), seq, at, input: input.clone(), raw };
        if let Some(key) = cont_key(&input) {
            let mut table = self.continuous.lock().unwrap_or_else(|e| e.into_inner());
            let map_key = (device.0.clone(), key);
            if let Some(prev) = table.get_mut(&map_key) {
                lane.counters.coalesced.fetch_add(1, Ordering::Relaxed);
                let summed = match (&prev.input, &input) {
                    (SurfaceInput::Relative { delta: was, .. }, SurfaceInput::Relative { delta: now, .. }) => Some(was.saturating_add(*now)),
                    _ => None,
                };
                *prev = queued;
                if let Some(delta) = summed
                    && let SurfaceInput::Relative { delta: slot, .. } = &mut prev.input
                {
                    *slot = delta;
                }
            } else {
                table.insert(map_key, queued);
            }
            return seq;
        }
        // release the device map before a blocking send so a consumer can still read counters
        drop(guard);
        match self.triggers.try_send(queued) {
            Ok(()) => {}
            Err(TrySendError::Full(queued)) => {
                if let Some(lane) = devices(&self.devices).get(&device.0) {
                    lane.counters.stalls.fetch_add(1, Ordering::Relaxed);
                }
                if self.triggers.send(queued).is_err()
                    && let Some(lane) = devices(&self.devices).get(&device.0)
                {
                    lane.counters.errors.fetch_add(1, Ordering::Relaxed);
                }
            }
            Err(TrySendError::Disconnected(_)) => {
                if let Some(lane) = devices(&self.devices).get(&device.0) {
                    lane.counters.errors.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        seq
    }

    /// The ALSA sequencer dropped a message before this process saw it.
    pub fn kernel_lost(&self) {
        self.kernel_dropped.fetch_add(1, Ordering::Relaxed);
    }
}

impl LaneOut {
    /// Every trigger waiting, then the latest continuous value of each control, ordered by
    /// device and `seq`. Triggers are never left behind.
    pub fn drain(&self) -> Vec<Queued> {
        let mut out = Vec::new();
        while let Ok(q) = self.triggers.try_recv() {
            out.push(q);
        }
        {
            let mut table = self.continuous.lock().unwrap_or_else(|e| e.into_inner());
            out.extend(table.drain().map(|(_, q)| q));
        }
        out.sort_by(|a, b| a.device.cmp(&b.device).then(a.seq.cmp(&b.seq)));
        let guard = devices(&self.devices);
        for q in &out {
            if let Some(lane) = guard.get(&q.device.0) {
                lane.counters.dispatched.fetch_add(1, Ordering::Relaxed);
            }
        }
        out
    }

    /// Counters for every device that has sent something.
    pub fn counters(&self) -> HashMap<String, DeviceCounters> {
        devices(&self.devices).iter().map(|(id, lane)| (id.clone(), lane.counters.snapshot())).collect()
    }

    /// Add `n` to a device's `gaps` (a loss the kernel reported, or a hole the caller counted).
    pub fn add_gaps(&self, device: &str, n: u64) {
        if n == 0 {
            return;
        }
        let mut guard = devices(&self.devices);
        let lane = guard.entry(device.to_string()).or_insert_with(|| DeviceLane { seq: AtomicU64::new(0), counters: Atomics::default() });
        lane.counters.gaps.fetch_add(n, Ordering::Relaxed);
    }

    pub fn kernel_dropped(&self) -> u64 {
        self.kernel_dropped.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfa4_surface_model::MidiKind;
    use std::thread;
    use std::time::Duration;

    fn cc(n: u16) -> ControlId {
        ControlId::Midi { channel: 0, kind: MidiKind::Cc, number: n }
    }

    fn abs(n: u16, v: u16) -> SurfaceInput {
        SurfaceInput::Absolute { control: cc(n), value: v, bits: 7 }
    }

    #[test]
    fn triggers_are_kept_when_the_lane_is_full_and_continuous_values_collapse() {
        let (tx, rx) = lane();
        let device = DeviceId("midi:k:p".into());
        let button = SurfaceInput::Button { control: cc(1), pressed: true };
        let send = tx.clone();
        let dev = device.clone();
        let worker = thread::spawn(move || {
            for i in 0..TRIGGER_CAPACITY + 3 {
                send.push(dev.clone(), MonoNs(i as u64), button.clone(), vec![]);
            }
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while rx.counters().get("midi:k:p").map(|c| c.stalls).unwrap_or(0) == 0 && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        let stalls = rx.counters().get("midi:k:p").map(|c| c.stalls).unwrap_or(0);
        assert!(stalls >= 1, "a full trigger lane blocks and counts a stall, got {stalls}");
        let mut got = Vec::new();
        while got.len() < TRIGGER_CAPACITY + 3 && std::time::Instant::now() < deadline {
            got.extend(rx.drain());
            thread::sleep(Duration::from_millis(1));
        }
        drop(rx);
        let _ = worker.join();
        assert_eq!(got.len(), TRIGGER_CAPACITY + 3, "a full lane does not drop a trigger");
        assert!(got.windows(2).all(|w| w[0].seq < w[1].seq));

        let (tx, rx) = lane();
        tx.push(device.clone(), MonoNs(1), abs(7, 1), vec![]);
        tx.push(device.clone(), MonoNs(2), abs(7, 2), vec![]);
        tx.push(device.clone(), MonoNs(3), abs(7, 9), vec![]);
        let got = rx.drain();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].input, abs(7, 9));
        assert_eq!(rx.counters().get("midi:k:p").map(|c| c.coalesced), Some(2));
        let stalls = rx.counters().get("midi:k:p").map(|c| c.stalls).unwrap_or(0);
        assert_eq!(stalls, 0, "continuous values do not stall");

        let rel = |d| SurfaceInput::Relative { control: cc(8), delta: d, encoding: cfa4_surface_model::RelEncoding::TwosComplement };
        tx.push(device.clone(), MonoNs(4), rel(1), vec![]);
        tx.push(device, MonoNs(5), rel(-3), vec![]);
        let got = rx.drain();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].input, rel(-2), "relative deltas add");
    }
}
