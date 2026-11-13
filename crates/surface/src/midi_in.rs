//! Reading MIDI from the controllers: the input side of the ALSA sequencer, for the MIDI Monitor
//! now and for mapping and MIDI Learn next.
//!
//! A [`MidiReader`] runs one thread with its own sequencer client (named `CFA4`, so CFA4's own
//! discovery skips it) and one input port. It subscribes to every control surface candidate
//! ([`midi::is_candidate`]) and to the System Announce port, so a controller plugged in while it
//! runs is read too. Each message is decoded into its bytes and handed over through a bounded
//! queue: when the queue is full the message is counted in [`MidiReader::dropped`], never blocking
//! the sequencer and never growing without limit. Dropping the reader stops the thread.

use crate::midi::{self, Port};
use alsa::Direction;
use alsa::seq::{Addr, EventType, MidiEvent, PortCap, PortSubscribe, PortType, Seq};
use cfa4_surface_model::{DeviceId, MAX_RAW, MonoNs};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::JoinHandle;
use std::time::Instant;

/// Messages waiting for the GUI thread at most; more are counted as dropped.
pub const QUEUE: usize = 4096;

/// How often the thread looks at its stop flag while no MIDI arrives (milliseconds).
const POLL_MS: i32 = 100;

/// One message from one controller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MidiMessage {
    pub device: DeviceId,
    /// Since the reader started.
    pub at: MonoNs,
    /// The complete message, at most [`MAX_RAW`] bytes (a longer SysEx is cut; its length is in
    /// [`MidiMessage::len`]).
    pub raw: Vec<u8>,
    pub len: usize,
}

/// The reading thread and its queue.
pub struct MidiReader {
    rx: Receiver<MidiMessage>,
    stop: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
}

impl MidiReader {
    /// Opens the sequencer and starts reading; an error when there is no ALSA sequencer.
    pub fn start() -> Result<Self, String> {
        let (tx, rx) = sync_channel(QUEUE);
        let (ready_tx, ready_rx) = sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicU64::new(0));
        let (s, d) = (stop.clone(), dropped.clone());
        let thread = std::thread::Builder::new()
            .name("cfa4-surface-midi".into())
            .spawn(move || run(&tx, &s, &d, &ready_tx))
            .map_err(|e| format!("MIDI: cannot start the reading thread ({e})"))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self { rx, stop, dropped, thread: Some(thread) }),
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

    /// Up to `max` waiting messages, oldest first.
    pub fn drain(&self, max: usize) -> Vec<MidiMessage> {
        self.rx.try_iter().take(max).collect()
    }

    /// Messages lost so far because the queue was full (or the sequencer overran).
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
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

fn run(tx: &SyncSender<MidiMessage>, stop: &AtomicBool, dropped: &AtomicU64, ready: &SyncSender<Result<(), String>>) {
    let opened = (|| {
        let seq = Seq::open(None, Some(Direction::Capture), true).map_err(|e| format!("MIDI: no ALSA sequencer ({e})"))?;
        let _ = seq.set_client_name(c"CFA4");
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
                    dropped.fetch_add(1, Ordering::Relaxed); // the sequencer's own queue overran
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
            let Some(device) = devices.get(&(source.client, source.port)) else { continue };
            let Ok(n) = coder.decode(&mut buf, &mut ev) else { continue }; // not a MIDI message
            let n = n.min(buf.len());
            if n == 0 {
                continue;
            }
            let at = MonoNs(u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX));
            let msg = MidiMessage { device: device.clone(), at, raw: buf[..n.min(MAX_RAW)].to_vec(), len: n };
            match tx.try_send(msg) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    dropped.fetch_add(1, Ordering::Relaxed);
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
                drop(r);
            }
            Err(e) => assert!(e.contains("ALSA sequencer"), "{e}"),
        }
    }
}
