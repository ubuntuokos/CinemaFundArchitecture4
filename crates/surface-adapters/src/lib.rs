//! Hardware control surface adapter processes (layer L5; plan CFA4-HW-CONTROL-SURFACES-001,
//! integration annex E2, E5).
//!
//! An adapter family that brings C code or its own runtime (here: `mirabox-family` on mirajazz,
//! tokio and async-hid) runs in its own process, `cfa4-cli surfaces --adapter <family>`, so a crash
//! or a hang in a device library is an error for that family and never a CFA4 crash (plan §5.4).
//! The process speaks the [`wire`] protocol on stdin / stdout; its supervisor (`cfa4-surface` in
//! `cfa4-cli serve`, integration annex Q1) turns the events into `SurfaceEvent`s.
//!
//! [`run`] is the adapter loop, generic over a [`Backend`], so it is tested without hardware
//! (`testing::FakeBackend`, feature `testing`). Each device runs in its own task: a panic inside a device library
//! ends that device's task with an `internal` error and leaves the others running. One connection
//! per device is kept open for as long as the device is present (the 2026-11-13 MiraBox rule:
//! initialisation and CRT DIS only once).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub mod profile;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
pub mod wire;

#[cfg(feature = "mirabox")]
pub mod mirabox;

#[cfg(test)]
mod tests;

use profile::{Control, Profile};
use wire::{Command, ErrorCode, Event, InputAction};

/// The adapter family id of this crate's MiraBox adapter.
pub const MIRABOX_FAMILY: &str = "mirabox-family";

/// An adapter error with its machine code.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct AdapterError {
    pub code: ErrorCode,
    pub message: String,
}

impl AdapterError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        AdapterError { code, message: message.into() }
    }
}

/// A device the backend found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// The device node (`/dev/hidraw3`); stable while the device stays plugged in.
    pub path: String,
    /// Index into the profile list.
    pub profile: usize,
}

/// What opening a device told about it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Opened {
    pub firmware: Option<String>,
    pub serial: Option<String>,
}

/// One input change: a 0-based device index and whether it went down.
pub type RawInput = (u8, bool);

/// The device library side of an adapter.
pub trait Backend: Send + Sync + 'static {
    type Device: DeviceIo;
    /// The connected devices that match a profile.
    fn scan(&self, profiles: &[Profile]) -> impl Future<Output = Result<Vec<Found>, AdapterError>> + Send;
    /// Opens one device. Called once per connection.
    fn open(&self, found: &Found, profile: &Profile) -> impl Future<Output = Result<(Self::Device, Opened), AdapterError>> + Send;
}

/// One open device.
pub trait DeviceIo: Send + 'static {
    /// Waits at most `timeout` for input.
    fn read(&mut self, timeout: Duration) -> impl Future<Output = Result<Vec<RawInput>, AdapterError>> + Send;
    fn set_image(&mut self, device_index: u8, image: image::RgbaImage) -> impl Future<Output = Result<(), AdapterError>> + Send;
    /// `None` clears every key.
    fn clear(&mut self, device_index: Option<u8>) -> impl Future<Output = Result<(), AdapterError>> + Send;
    fn set_brightness(&mut self, percent: u8) -> impl Future<Output = Result<(), AdapterError>> + Send;
    fn sleep(&mut self) -> impl Future<Output = Result<(), AdapterError>> + Send;
    fn close(&mut self) -> impl Future<Output = ()> + Send;
}

/// Loop timing.
#[derive(Clone, Copy, Debug)]
pub struct RunOptions {
    /// How often the backend is scanned for plugged and unplugged devices.
    pub scan_every: Duration,
    /// How long a device read waits before pending commands are applied.
    pub read_slice: Duration,
    /// How long a device that failed to open is left alone.
    pub retry_after: Duration,
}

impl Default for RunOptions {
    fn default() -> Self {
        RunOptions { scan_every: Duration::from_millis(1000), read_slice: Duration::from_millis(15), retry_after: Duration::from_secs(5) }
    }
}

/// The id a device has on the wire.
pub fn device_id(p: &Profile, path: &str) -> String {
    format!("hid:{:04x}:{:04x}:{path}", p.vid, p.pid)
}

struct Slot {
    path: String,
    commands: mpsc::UnboundedSender<Command>,
    task: JoinHandle<()>,
}

type Out = mpsc::UnboundedSender<Event>;

fn send(out: &Out, e: Event) {
    // A closed channel means the writer has ended (stdout gone); nothing is left to tell.
    let _ = out.send(e);
}

fn error(out: &Out, device: Option<&str>, seq: Option<u64>, e: &AdapterError) {
    send(out, Event::Error { device: device.map(str::to_owned), seq, code: e.code, message: e.message.clone() });
}

/// Runs the adapter until `quit`, end of input, or an output error.
pub async fn run<B, R, W>(backend: Arc<B>, adapter_version: &str, profiles: Vec<Profile>, input: R, mut output: W, opts: RunOptions) -> std::io::Result<()>
where
    B: Backend,
    R: AsyncBufRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (out, mut events) = mpsc::unbounded_channel::<Event>();
    let writer = tokio::spawn(async move {
        while let Some(e) = events.recv().await {
            let mut line = match serde_json::to_vec(&e) {
                Ok(l) => l,
                Err(_) => continue,
            };
            line.push(b'\n');
            output.write_all(&line).await?;
            output.flush().await?;
        }
        Ok::<(), std::io::Error>(())
    });

    let mut models: Vec<String> = profiles.iter().map(|p| p.model.clone()).collect();
    models.dedup();
    send(
        &out,
        Event::Hello {
            protocol: wire::PROTOCOL.into(),
            version: wire::PROTOCOL_VERSION,
            adapter: MIRABOX_FAMILY.into(),
            adapter_version: adapter_version.into(),
            models,
        },
    );

    let profiles = Arc::new(profiles);
    let mut slots: HashMap<String, Slot> = HashMap::new();
    let mut held_off: HashMap<String, Instant> = HashMap::new();
    let mut last_scan_error: Option<String> = None;
    let mut lines = input.lines();
    let mut tick = tokio::time::interval(opts.scan_every);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            line = lines.next_line() => {
                let line = match line {
                    Ok(Some(l)) => l,
                    Ok(None) | Err(_) => break,
                };
                if line.trim().is_empty() {
                    continue;
                }
                let cmd: Command = match serde_json::from_str(&line) {
                    Ok(c) => c,
                    Err(e) => {
                        error(&out, None, None, &AdapterError::new(ErrorCode::BadRequest, format!("not a command: {e}")));
                        continue;
                    }
                };
                if cmd == Command::Quit {
                    break;
                }
                let Some(dev) = cmd.device().map(str::to_owned) else { continue };
                match slots.get(&dev) {
                    Some(slot) if slot.commands.send(cmd.clone()).is_ok() => {}
                    _ => error(&out, Some(&dev), cmd.seq(), &AdapterError::new(ErrorCode::DeviceNotFound, "no such device is open")),
                }
            }
            _ = tick.tick() => {
                // finished device tasks: their device closed, failed or panicked
                let done: Vec<String> = slots.iter().filter(|(_, s)| s.task.is_finished()).map(|(k, _)| k.clone()).collect();
                for id in done {
                    if let Some(slot) = slots.remove(&id) {
                        if let Err(e) = slot.task.await {
                            let msg = if e.is_panic() { "the device library panicked" } else { "the device task stopped" };
                            error(&out, Some(&id), None, &AdapterError::new(ErrorCode::Internal, msg));
                            send(&out, Event::Disconnected { device: id.clone() });
                        }
                        held_off.insert(slot.path, Instant::now() + opts.retry_after);
                    }
                }
                let found = match backend.scan(&profiles).await {
                    Ok(f) => {
                        last_scan_error = None;
                        f
                    }
                    Err(e) => {
                        if last_scan_error.as_deref() != Some(e.message.as_str()) {
                            error(&out, None, None, &e);
                            last_scan_error = Some(e.message.clone());
                        }
                        continue;
                    }
                };
                // unplugged
                let present: Vec<String> = found.iter().filter_map(|f| profiles.get(f.profile).map(|p| device_id(p, &f.path))).collect();
                let gone: Vec<String> = slots.keys().filter(|k| !present.contains(k)).cloned().collect();
                for id in gone {
                    if let Some(slot) = slots.remove(&id) {
                        slot.task.abort();
                        send(&out, Event::Disconnected { device: id });
                    }
                }
                held_off.retain(|path, until| *until > Instant::now() && found.iter().any(|f| &f.path == path));
                // plugged in
                for f in found {
                    let Some(p) = profiles.get(f.profile) else { continue };
                    let id = device_id(p, &f.path);
                    if slots.contains_key(&id) || held_off.contains_key(&f.path) {
                        continue;
                    }
                    match backend.open(&f, p).await {
                        Ok((dev, opened)) => {
                            send(&out, Event::Connected {
                                device: id.clone(),
                                model: p.model.clone(),
                                vid: p.vid,
                                pid: p.pid,
                                firmware: opened.firmware,
                                serial_hash: opened.serial.as_deref().map(wire::serial_hash),
                                serial_unique: p.unique_serial,
                                controls: p.controls(),
                            });
                            let (tx, rx) = mpsc::unbounded_channel();
                            let task = tokio::spawn(device_task(dev, p.clone(), id.clone(), rx, out.clone(), opts.read_slice));
                            slots.insert(id, Slot { path: f.path.clone(), commands: tx, task });
                        }
                        Err(e) => {
                            error(&out, Some(&id), None, &e);
                            held_off.insert(f.path.clone(), Instant::now() + opts.retry_after);
                        }
                    }
                }
            }
        }
    }

    // close every device: dropping the command channel ends its task after `close`
    let tasks: Vec<JoinHandle<()>> = slots.into_values().map(|s| s.task).collect();
    for t in tasks {
        let _ = tokio::time::timeout(Duration::from_secs(2), t).await;
    }
    drop(out);
    match writer.await {
        Ok(r) => r,
        Err(e) => Err(std::io::Error::other(e.to_string())),
    }
}

async fn apply<D: DeviceIo>(dev: &mut D, p: &Profile, cmd: Command) -> Result<(), AdapterError> {
    match cmd {
        Command::SetImage { control, index, width, height, rgba, .. } => {
            if control != "keys" {
                return Err(AdapterError::new(ErrorCode::Unsupported, format!("`{control}` has no display")));
            }
            let Some(target) = p.device_index(Control::Key(index)) else {
                return Err(AdapterError::new(ErrorCode::Unsupported, format!("no key {index}")));
            };
            let img = wire::decode_rgba(width, height, &rgba).map_err(|m| AdapterError::new(ErrorCode::BadRequest, m))?;
            dev.set_image(target, img).await
        }
        Command::ClearImage { index: Some(i), .. } => {
            let Some(target) = p.device_index(Control::Key(i)) else {
                return Err(AdapterError::new(ErrorCode::Unsupported, format!("no key {i}")));
            };
            dev.clear(Some(target)).await
        }
        Command::ClearImage { index: None, .. } => dev.clear(None).await,
        Command::SetBrightness { percent, .. } => dev.set_brightness(percent.min(100)).await,
        Command::Sleep { .. } => dev.sleep().await,
        Command::Quit => Ok(()),
    }
}

async fn device_task<D: DeviceIo>(mut dev: D, p: Profile, id: String, mut commands: mpsc::UnboundedReceiver<Command>, out: Out, slice: Duration) {
    loop {
        loop {
            match commands.try_recv() {
                Ok(cmd) => {
                    let seq = cmd.seq();
                    match apply(&mut dev, &p, cmd).await {
                        Ok(()) => {
                            if let Some(seq) = seq {
                                send(&out, Event::Ack { seq });
                            }
                        }
                        Err(e) => error(&out, Some(&id), seq, &e),
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    dev.close().await;
                    return;
                }
            }
        }
        match dev.read(slice).await {
            Ok(inputs) => {
                for (index, down) in inputs {
                    if let Some(c) = p.control_of_device(index) {
                        let action = if down { InputAction::Press } else { InputAction::Release };
                        send(&out, Event::Input { device: id.clone(), control: c.group().into(), index: c.index(), action });
                    }
                }
            }
            Err(e) => {
                error(&out, Some(&id), None, &e);
                send(&out, Event::Disconnected { device: id.clone() });
                dev.close().await;
                return;
            }
        }
    }
}
