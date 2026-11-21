//! Reading MIDI from the controllers: the input side of the ALSA sequencer.
//!
//! A [`MidiReader`] runs one thread with its own sequencer client (named `CFA4 Surfaces`, so
//! discovery skips it and it is not confused with the timecode client `CFA4`; D7) and one input
//! port. It subscribes to every control surface candidate ([`midi::is_candidate`]) and to the
//! System Announce port, so a controller plugged in while it runs is read too.
//!
//! Mapping inputs go through an F0 lane ([`lane`]): triggers block when the lane is full
//! (`stalls`), continuous values keep the last one (`coalesced`). The monitor has its own
//! bounded queue and may drop a line; that drop is not a lost trigger. Clock, Active Sensing
//! and MTC are monitor-only ([`midi::is_timing`]). Dropping the reader stops the thread.

use crate::lane::{self, LaneIn, LaneOut, Queued};
use crate::midi::{self, Port};
use alsa::Direction;
use alsa::seq::{Addr, EventType, MidiEvent, PortCap, PortSubscribe, PortType, Seq};
use cfa4_surface_model::midi::is_timing;
use cfa4_surface_model::{DeviceCounters, DeviceId, MAX_RAW, MonoNs};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::JoinHandle;
use std::time::Instant;

/// Monitor lines waiting at once. A full monitor queue drops a line; it does not drop a trigger.
const MONITOR_QUEUE: usize = 256;

/// How often the thread looks at its stop flag while no MIDI arrives (milliseconds).
const POLL_MS: i32 = 100;

/// The ALSA client name of the surface reader (D7). The timecode client stays `CFA4`.
pub const CLIENT_NAME: &str = "CFA4 Surfaces";

/// One message from one controller, for the MIDI Monitor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MidiMessage {
    pub device: DeviceId,
    /// Since the reader started.
    pub at: MonoNs,
    /// The complete message, at most [`MAX_RAW`] bytes (a longer SysEx is cut; its length is in
    /// [`MidiMessage::len`]).
    pub raw: Vec<u8>,
    pub len: usize,
    pub seq: u64,
}

/// The reading thread and its queues.
pub struct MidiReader {
    lane: LaneOut,
    monitor_rx: Receiver<MidiMessage>,
    monitor_dropped: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MidiReader {
    /// Opens the sequencer and starts reading; an error when there is no ALSA sequencer.
    pub fn start() -> Result<Self, String> {
        let (lane_in, lane) = lane::lane();
        let (monitor_tx, monitor_rx) = sync_channel(MONITOR_QUEUE);
        let (ready_tx, ready_rx) = sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let monitor_dropped = Arc::new(AtomicU64::new(0));
        let (s, d) = (stop.clone(), monitor_dropped.clone());
        let thread = std::thread::Builder::new()
            .name("cfa4-surface-midi".into())
            .spawn(move || run(lane_in, &monitor_tx, &s, &d, &ready_tx))
            .map_err(|e| format!("MIDI: cannot start the reading thread ({e})"))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self { lane, monitor_rx, monitor_dropped, stop, thread: Some(thread) }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err("MIDI: the reading thread ended before it started".into())
            }
        }
    }

    /// Up to `max` monitor lines, oldest first.
    pub fn drain(&self, max: usize) -> Vec<MidiMessage> {
        self.monitor_rx.try_iter().take(max).collect()
    }

    /// Mapping inputs since the last call (triggers and the latest continuous values).
    pub fn inputs(&self) -> Vec<Queued> {
        self.lane.drain()
    }

    /// Which controls are endless encoders (see [`LaneOut::set_relative`]).
    pub fn set_relative(&self, table: HashMap<(String, cfa4_surface_model::ControlId), cfa4_surface_model::RelEncoding>) {
        self.lane.set_relative(table);
    }

    /// Monitor lines lost because the monitor queue was full, plus sequencer overruns.
    /// A full trigger lane is [`MidiReader::counters`] `stalls`, not a drop.
    pub fn dropped(&self) -> u64 {
        self.monitor_dropped.load(Ordering::Relaxed).saturating_add(self.lane.kernel_dropped())
    }

    /// Per-device counters (F0 spec §3).
    pub fn counters(&self) -> HashMap<String, DeviceCounters> {
        self.lane.counters()
    }

    /// Whether the thread still runs (it ends when the sequencer goes away).
    pub fn running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }
}

impl Drop for MidiReader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The device id and whether to read a port, from the sequencer's own description of it.
fn candidate(seq: &Seq, addr: Addr, own: i32) -> Option<DeviceId> {
    let info = seq.get_any_port_info(addr).ok()?;
    let client_name = seq.get_any_client_info(addr.client).ok()?.get_name().unwrap_or_default().to_string();
    let caps = info.get_capability();
    let port = Port {
        client: addr.client,
        port: addr.port,
        client_name,
        port_name: info.get_name().unwrap_or_default().to_string(),
        readable: caps.contains(PortCap::READ | PortCap::SUBS_READ),
        writable: caps.contains(PortCap::WRITE | PortCap::SUBS_WRITE),
    };
    midi::is_candidate(&port, own).then(|| midi::info(&port).device)
}

fn subscribe(seq: &Seq, sender: Addr, dest: Addr) -> bool {
    let Ok(sub) = PortSubscribe::empty() else { return false };
    sub.set_sender(sender);
    sub.set_dest(dest);
    seq.subscribe_port(&sub).is_ok()
}

fn run(lane: LaneIn, monitor: &SyncSender<MidiMessage>, stop: &AtomicBool, monitor_dropped: &AtomicU64, ready: &SyncSender<Result<(), String>>) {
    let opened = (|| {
        let seq = Seq::open(None, Some(Direction::Capture), true).map_err(|e| format!("MIDI: no ALSA sequencer ({e})"))?;
        let _ = seq.set_client_name(c"CFA4 Surfaces");
        let port = seq
            .create_simple_port(c"Control Surfaces", PortCap::WRITE | PortCap::SUBS_WRITE, PortType::MIDI_GENERIC | PortType::APPLICATION)
            .map_err(|e| format!("MIDI: cannot create the input port ({e})"))?;
        let own = seq.client_id().map_err(|e| format!("MIDI: no client id ({e})"))?;
        let coder = MidiEvent::new(MAX_RAW as u32 * 16).map_err(|e| format!("MIDI: no event decoder ({e})"))?;
        coder.enable_running_status(false);
        Ok::<_, String>((seq, Addr { client: own, port }, coder))
    })();
    let (seq, me, coder) = match opened {
        Ok(v) => v,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    // hot-plug: the System Announce port tells about ports that come and go
    subscribe(&seq, Addr::system_announce(), me);
    let mut devices: HashMap<(i32, i32), DeviceId> = HashMap::new();
    for p in midi::all_ports(&seq) {
        let addr = Addr { client: p.client, port: p.port };
        if midi::is_candidate(&p, me.client) && subscribe(&seq, addr, me) {
            devices.insert((p.client, p.port), midi::info(&p).device);
        }
    }
    let _ = ready.send(Ok(()));
    let started = Instant::now();
    let mut input = seq.input();
    let mut buf = vec![0u8; MAX_RAW * 16];
    while !stop.load(Ordering::Relaxed) {
        let mut fds = match alsa::poll::Descriptors::get(&(&seq, Some(Direction::Capture))) {
            Ok(f) => f,
            Err(_) => return,
        };
        if alsa::poll::poll(&mut fds, POLL_MS).is_err() {
            continue; // interrupted: look at the stop flag and wait again
        }
        while input.event_input_pending(true).unwrap_or(0) > 0 {
            let mut ev = match input.event_input() {
                Ok(ev) => ev,
                Err(_) => {
                    lane.kernel_lost(); // the sequencer's own queue overran
                    break;
                }
            };
            let source = ev.get_source();
            match ev.get_type() {
                EventType::PortStart => {
                    if let Some(a) = ev.get_data::<Addr>()
                        && let Some(id) = candidate(&seq, a, me.client)
                        && subscribe(&seq, a, me)
                    {
                        devices.insert((a.client, a.port), id);
                    }
                    continue;
                }
                EventType::PortExit => {
                    if let Some(a) = ev.get_data::<Addr>() {
                        devices.remove(&(a.client, a.port));
                    }
                    continue;
                }
                _ => {}
            }
            let Some(device) = devices.get(&(source.client, source.port)).cloned() else { continue };
            let Ok(n) = coder.decode(&mut buf, &mut ev) else { continue }; // not a MIDI message
            let n = n.min(buf.len());
            if n == 0 {
                continue;
            }
            let at = MonoNs(u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX));
            let bytes = &buf[..n];
            // timing is not a control and must not fill the trigger lane
            let seq_n =
                if is_timing(bytes) { 0 } else { lane.push(device.clone(), at, cfa4_surface_model::midi::input(bytes), bytes[..n.min(MAX_RAW)].to_vec()) };
            let msg = MidiMessage { device, at, raw: bytes[..n.min(MAX_RAW)].to_vec(), len: n, seq: seq_n };
            match monitor.try_send(msg) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    monitor_dropped.fetch_add(1, Ordering::Relaxed);
                }
                Err(TrySendError::Disconnected(_)) => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In a container without `/dev/snd/seq` starting is an error; on a desktop the reader runs
    /// and stops when dropped. Never a panic, never a hang.
    #[test]
    fn start_and_stop() {
        match MidiReader::start() {
            Ok(r) => {
                assert!(r.running());
                assert!(r.drain(10).iter().all(|m| m.device.0.starts_with("midi:")));
                let _ = r.inputs();
                drop(r);
            }
            Err(e) => assert!(e.contains("ALSA sequencer"), "{e}"),
        }
    }

    #[test]
    fn the_client_name_is_not_the_timecode_client() {
        assert_eq!(CLIENT_NAME, "CFA4 Surfaces");
        assert!(midi::is_own_client(CLIENT_NAME));
        assert!(midi::is_own_client("CFA4"));
    }
}
