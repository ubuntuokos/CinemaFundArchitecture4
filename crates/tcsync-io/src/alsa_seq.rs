//! MIDI Timecode through the ALSA sequencer.
//!
//! One sequencer client named `CFA4` with a `Timecode In` and/or `Timecode Out` port; other
//! programs and hardware connect to them (with `aconnect` or a patchbay), or the settings name a
//! port to connect to. One thread reads the input and sends the quarter frames on time.

use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use alsa::poll::Descriptors;
use alsa::seq::{Addr, ClientIter, MidiEvent, PortCap, PortIter, PortSubscribe, PortType, Seq};

use crate::manager::{Device, MtcConfig};
use crate::mtc::{MtcReader, MtcScheduler};
use crate::{Input, SharedTransport, lock};

/// How a port spec names a port: `client:port` numbers, `Client Name:Port Name`, a port name, or
/// a client name (its first port that fits).
pub fn port_matches(spec: &str, client: i32, port: i32, client_name: &str, port_name: &str) -> bool {
    let spec = spec.trim();
    if let Some((c, p)) = spec.split_once(':')
        && let (Ok(c), Ok(p)) = (c.trim().parse::<i32>(), p.trim().parse::<i32>())
    {
        return c == client && p == port;
    }
    spec == format!("{client_name}:{port_name}") || spec == port_name || spec == client_name
}

fn find_port(seq: &Seq, spec: &str, caps: PortCap, own: i32) -> Result<Addr, String> {
    for c in ClientIter::new(seq) {
        let client = c.get_client();
        if client == own {
            continue;
        }
        let cname = c.get_name().unwrap_or_default().to_string();
        for p in PortIter::new(seq, client) {
            if p.get_capability().contains(caps) && port_matches(spec, client, p.get_port(), &cname, p.get_name().unwrap_or_default()) {
                return Ok(p.addr());
            }
        }
    }
    Err(format!("MIDI port `{spec}` not found"))
}

fn seq_err(what: &str, e: alsa::Error) -> String {
    format!("MIDI Timecode: {what}: {e}")
}

struct Ports {
    seq: Seq,
    input: Option<i32>,
    output: Option<i32>,
}

fn open_ports(cfg: &MtcConfig) -> Result<Ports, String> {
    let seq = Seq::open(None, None, true).map_err(|e| seq_err("no ALSA sequencer", e))?;
    let name = CString::new("CFA4").unwrap_or_default();
    seq.set_client_name(&name).map_err(|e| seq_err("naming the client", e))?;
    let own = seq.client_id().map_err(|e| seq_err("client id", e))?;
    let kind = PortType::MIDI_GENERIC | PortType::APPLICATION;
    let mut ports = Ports { input: None, output: None, seq };
    if let Some(spec) = &cfg.input {
        let name = CString::new("Timecode In").unwrap_or_default();
        let port = ports.seq.create_simple_port(&name, PortCap::WRITE | PortCap::SUBS_WRITE, kind).map_err(|e| seq_err("creating the input port", e))?;
        if !spec.trim().is_empty() {
            let from = find_port(&ports.seq, spec, PortCap::READ | PortCap::SUBS_READ, own)?;
            let sub = PortSubscribe::empty().map_err(|e| seq_err("subscribing", e))?;
            sub.set_sender(from);
            sub.set_dest(Addr { client: own, port });
            ports.seq.subscribe_port(&sub).map_err(|e| seq_err(&format!("connecting from `{spec}`"), e))?;
        }
        ports.input = Some(port);
    }
    if let Some(spec) = &cfg.output {
        let name = CString::new("Timecode Out").unwrap_or_default();
        let port = ports.seq.create_simple_port(&name, PortCap::READ | PortCap::SUBS_READ, kind).map_err(|e| seq_err("creating the output port", e))?;
        if !spec.trim().is_empty() {
            let to = find_port(&ports.seq, spec, PortCap::WRITE | PortCap::SUBS_WRITE, own)?;
            let sub = PortSubscribe::empty().map_err(|e| seq_err("subscribing", e))?;
            sub.set_sender(Addr { client: own, port });
            sub.set_dest(to);
            ports.seq.subscribe_port(&sub).map_err(|e| seq_err(&format!("connecting to `{spec}`"), e))?;
        }
        ports.output = Some(port);
    }
    Ok(ports)
}

/// Reads every pending input event as MIDI bytes.
fn read_input(seq: &Seq, decoder: &MidiEvent, buf: &mut [u8], bytes: &mut Vec<u8>) {
    let mut input = seq.input();
    while let Ok(mut ev) = input.event_input() {
        if let Ok(n) = decoder.decode(buf, &mut ev) {
            bytes.extend_from_slice(&buf[..n.min(buf.len())]);
        }
    }
}

fn send(seq: &Seq, encoder: &mut MidiEvent, port: i32, msg: &[u8]) -> Result<(), String> {
    encoder.reset_encode();
    let (_, ev) = encoder.encode(msg).map_err(|e| seq_err("encoding", e))?;
    if let Some(mut ev) = ev {
        ev.set_source(port);
        ev.set_subs();
        ev.set_direct();
        seq.event_output_direct(&mut ev).map_err(|e| seq_err("sending", e))?;
    }
    Ok(())
}

fn run(ports: Ports, cfg: MtcConfig, tx: Sender<Input>, transport: SharedTransport, stop: Arc<AtomicBool>) -> Result<(), String> {
    let decoder = MidiEvent::new(64).map_err(|e| seq_err("MIDI decoder", e))?;
    decoder.enable_running_status(false);
    let mut encoder = MidiEvent::new(64).map_err(|e| seq_err("MIDI encoder", e))?;
    let mut reader = MtcReader::new(cfg.rate);
    let mut scheduler = MtcScheduler::new(cfg.rate, cfg.offset, cfg.latency);
    let mut fds = (&ports.seq, Some(alsa::Direction::Capture)).get().map_err(|e| seq_err("poll descriptors", e))?;
    let (mut buf, mut bytes, mut inputs, mut msgs) = ([0u8; 64], Vec::new(), Vec::new(), Vec::new());
    let max_wait = cfg.rate.rate.frame_duration();
    while !stop.load(Ordering::Relaxed) {
        let now = cfg.clock.now();
        let mut next = cfa4_time::Tick(now.0 + max_wait.0);
        if let Some(port) = ports.output {
            let tr = *lock(&transport);
            msgs.clear();
            next = next.min(scheduler.poll(now, &tr, &mut msgs));
            for m in &msgs {
                send(&ports.seq, &mut encoder, port, m)?;
            }
        }
        let wait_ms = next.0.saturating_sub(cfg.clock.now().0) / (cfa4_time::TICKS_PER_SECOND / 1000);
        let wait_ms = i32::try_from(wait_ms.clamp(0, 50)).unwrap_or(1);
        if ports.input.is_some() {
            if alsa::poll::poll(&mut fds, wait_ms).is_err() {
                std::thread::sleep(Duration::from_millis(1));
            }
            bytes.clear();
            read_input(&ports.seq, &decoder, &mut buf, &mut bytes);
            if !bytes.is_empty() {
                inputs.clear();
                reader.push(&bytes, cfg.clock.now(), &mut inputs);
                for i in inputs.drain(..) {
                    let _ = tx.send(i);
                }
            }
        } else {
            std::thread::sleep(Duration::from_millis(u64::try_from(wait_ms).unwrap_or(1)));
        }
    }
    Ok(())
}

struct MtcThread {
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    error: Arc<Mutex<Option<String>>>,
}

impl Device for MtcThread {
    fn error(&self) -> Option<String> {
        lock(&self.error).clone()
    }
}

impl Drop for MtcThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

/// Opens the sequencer ports and starts the MTC thread.
pub fn open_mtc(cfg: &MtcConfig, tx: Sender<Input>, transport: SharedTransport) -> Result<Box<dyn Device>, String> {
    let cfg = cfg.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let error: Arc<Mutex<Option<String>>> = Arc::default();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let (thread_stop, thread_error) = (stop.clone(), error.clone());
    let worker = std::thread::Builder::new()
        .name("timecode MTC".into())
        .spawn(move || {
            let body = move || -> Result<(), String> {
                let ports = open_ports(&cfg);
                let ok = ports.as_ref().map(|_| ()).map_err(Clone::clone);
                let _ = ready_tx.send(ok);
                run(ports?, cfg, tx.clone(), transport, thread_stop).inspect_err(|e| {
                    let _ = tx.send(Input::Error(e.clone()));
                })
            };
            if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)).unwrap_or_else(|_| Err("MIDI Timecode thread panicked".into())) {
                *lock(&thread_error) = Some(e);
            }
        })
        .map_err(|e| e.to_string())?;
    let dev = MtcThread { stop, worker: Some(worker), error };
    ready_rx.recv().map_err(|e| e.to_string()).and_then(|r| r)?;
    Ok(Box::new(dev))
}

/// The hardware backend: cpal for LTC, the ALSA sequencer for MTC.
#[derive(Clone, Copy, Debug, Default)]
pub struct Hardware;

impl crate::Backend for Hardware {
    fn ltc_input(&mut self, cfg: &crate::manager::LtcInputConfig, tx: Sender<Input>) -> Result<Box<dyn Device>, String> {
        crate::cpal_dev::open_ltc_input(cfg, tx)
    }
    fn ltc_output(&mut self, cfg: &crate::manager::LtcOutputConfig, transport: SharedTransport) -> Result<Box<dyn Device>, String> {
        crate::cpal_dev::open_ltc_output(cfg, transport)
    }
    fn mtc(&mut self, cfg: &MtcConfig, tx: Sender<Input>, transport: SharedTransport) -> Result<Box<dyn Device>, String> {
        open_mtc(cfg, tx, transport)
    }
}

#[cfg(test)]
mod tests {
    use cfa4_time::{FrameRate, Tick};

    use super::*;
    use crate::{Clock, ExternalRate};

    #[test]
    fn port_specs() {
        assert!(port_matches("20:0", 20, 0, "Midi Through", "Midi Through Port-0"));
        assert!(!port_matches("20:1", 20, 0, "Midi Through", "Midi Through Port-0"));
        assert!(port_matches("Midi Through:Midi Through Port-0", 20, 0, "Midi Through", "Midi Through Port-0"));
        assert!(port_matches(" Midi Through Port-0 ", 20, 0, "Midi Through", "Midi Through Port-0"));
        assert!(port_matches("Midi Through", 20, 0, "Midi Through", "Midi Through Port-0"));
        assert!(!port_matches("Other", 20, 0, "Midi Through", "Midi Through Port-0"));
    }

    /// Without a sequencer (containers, CI) opening is an error; with one, a port that does not
    /// exist is an error. Never a panic.
    #[test]
    fn opening_fails_cleanly() {
        let cfg = MtcConfig {
            input: Some("no such port ×".into()),
            output: None,
            rate: ExternalRate::new(FrameRate::FPS_25, false),
            offset: Tick(0),
            latency: Tick(0),
            clock: Clock::new(),
        };
        let (tx, _rx) = mpsc::channel();
        let r = open_mtc(&cfg, tx, SharedTransport::default());
        assert!(r.is_err(), "connected to a port that does not exist");
    }
}
