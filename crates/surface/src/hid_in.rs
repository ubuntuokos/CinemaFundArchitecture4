//! Reading the Contour jog/shuttle controllers through `/dev/hidrawN`.
//!
//! The ShuttlePRO v1/v2 and the ShuttleXpress send a 5-byte input report whenever something
//! changes: `[shuttle i8 (−7..=7), jog u8 (a counter that wraps at 256), 0, buttons lo, buttons
//! hi]` (CFA4-CONTROL-SURFACE-HID-EVALUATION §3; the reference machine measurement M3–M6 confirms
//! the details). [`ShuttleDecoder`] turns consecutive reports into [`SurfaceInput`]s: a button
//! press or release, the shuttle ring's new position (0 = back in the middle, which hidraw does
//! report, unlike evdev) and the jog wheel's steps since the last report.
//!
//! A [`HidReader`] reads one device on its own thread and hands the inputs over through a bounded
//! queue; it ends when the device is unplugged (the read fails) or the reader is dropped.

use crate::lane::{self, LaneOut, Queued};
use cfa4_surface_model::{ControlId, DeviceId, MAX_RAW, MonoNs, SurfaceInput};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Instant;

/// The shuttle ring's positions on each side.
pub const SHUTTLE_RANGE: i8 = 7;

/// Kept so callers can name the old bound. The F0 lane capacity is [`crate::lane::TRIGGER_CAPACITY`].
pub const QUEUE: usize = crate::lane::TRIGGER_CAPACITY;

/// Turns ShuttlePRO / ShuttleXpress reports into inputs.
#[derive(Clone, Debug)]
pub struct ShuttleDecoder {
    buttons: u8,
    last: Option<(i8, u8, u16)>,
}

impl ShuttleDecoder {
    /// A decoder for a device with `buttons` buttons (13, 15 or 5).
    pub fn new(buttons: u8) -> Self {
        Self { buttons: buttons.min(16), last: None }
    }

    /// The inputs one report means: buttons first (in button order), then the shuttle, then
    /// the jog. The first report only sets the jog's starting count.
    pub fn decode(&mut self, report: &[u8]) -> Vec<SurfaceInput> {
        let [shuttle, jog, _, lo, hi, ..] = *report else { return Vec::new() };
        let shuttle = i8::from_ne_bytes([shuttle]).clamp(-SHUTTLE_RANGE, SHUTTLE_RANGE);
        let buttons = u16::from_le_bytes([lo, hi]);
        let (old_shuttle, old_jog, old_buttons) = self.last.unwrap_or((0, jog, 0));
        self.last = Some((shuttle, jog, buttons));
        let mut out = Vec::new();
        for b in 0..self.buttons {
            let bit = 1u16 << b;
            if (buttons ^ old_buttons) & bit != 0 {
                out.push(SurfaceInput::Button { control: ControlId::hid_named(&format!("button{}", b + 1)), pressed: buttons & bit != 0 });
            }
        }
        if shuttle != old_shuttle {
            out.push(SurfaceInput::Shuttle { control: ControlId::hid_named("shuttle"), position: shuttle, range: SHUTTLE_RANGE });
        }
        // the counter wraps: the shorter way round is the movement
        let delta = i32::from(i8::from_ne_bytes([jog.wrapping_sub(old_jog)]));
        if delta != 0 {
            out.push(SurfaceInput::Jog { control: ControlId::hid_named("jog"), delta });
        }
        out
    }

    /// Buttons that are down, so an unplug can release them (F0 spec §9.6).
    pub fn held(&self) -> Vec<ControlId> {
        let Some((_, _, buttons)) = self.last else { return Vec::new() };
        (0..self.buttons).filter(|b| buttons & (1u16 << b) != 0).map(|b| ControlId::hid_named(&format!("button{}", b + 1))).collect()
    }
}

/// One input from one jog/shuttle device. [`Queued`] is the lane's form; this alias keeps the
/// call sites that name a HID input.
pub type HidInput = Queued;

/// The reading thread of one device and its lane.
pub struct HidReader {
    lane: LaneOut,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl HidReader {
    /// Opens `node` (a `/dev/hidrawN`) and starts reading it; an error that says why when the
    /// node cannot be opened (no permission without CFA4's udev rule).
    pub fn start(device: DeviceId, node: &Path, buttons: u8) -> Result<Self, String> {
        let file = File::open(node).map_err(|e| {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                format!("{device}: no permission to read {}: install CFA4's udev rule (uaccess) and plug it in again", node.display())
            } else {
                format!("{device}: cannot open {}: {e}", node.display())
            }
        })?;
        let (tx, lane) = lane::lane();
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let thread = std::thread::Builder::new()
            .name("cfa4-surface-hid".into())
            .spawn(move || run(file, device, buttons, tx, &s))
            .map_err(|e| format!("HID: cannot start the reading thread ({e})"))?;
        Ok(Self { lane, stop, thread: Some(thread) })
    }

    /// Waiting inputs, oldest first. `max` is accepted for the old call shape; triggers are never
    /// left in the lane, so the cap is not applied to them.
    pub fn drain(&self, _max: usize) -> Vec<HidInput> {
        self.lane.drain()
    }

    /// Trigger waits (`stalls`), not silent drops. Kept under the old name for callers.
    pub fn dropped(&self) -> u64 {
        self.lane.counters().values().map(|c| c.stalls).sum()
    }

    /// Whether the device is still read (the thread ends when it is unplugged).
    pub fn running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }
}

impl Drop for HidReader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn release_held(decoder: &ShuttleDecoder, device: &DeviceId, at: MonoNs, tx: &lane::LaneIn) {
    for control in decoder.held() {
        tx.push(device.clone(), at, SurfaceInput::Button { control, pressed: false }, Vec::new());
    }
}

fn run(mut file: File, device: DeviceId, buttons: u8, tx: lane::LaneIn, stop: &AtomicBool) {
    let started = Instant::now();
    let mut decoder = ShuttleDecoder::new(buttons);
    let mut buf = [0u8; MAX_RAW];
    // wake up ten times a second to see the stop flag
    let tick = Timespec { tv_sec: 0, tv_nsec: 100_000_000 };
    let at_now = || MonoNs(u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX));
    loop {
        if stop.load(Ordering::Relaxed) {
            release_held(&decoder, &device, at_now(), &tx);
            return;
        }
        let mut fds = [PollFd::new(&file, PollFlags::IN)];
        match poll(&mut fds, Some(&tick)) {
            Ok(0) => continue,
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => continue,
            Err(_) => {
                release_held(&decoder, &device, at_now(), &tx);
                return;
            }
        }
        if fds[0].revents().intersects(PollFlags::ERR | PollFlags::HUP | PollFlags::NVAL) {
            release_held(&decoder, &device, at_now(), &tx);
            return; // unplugged
        }
        let n = match file.read(&mut buf) {
            Ok(0) | Err(_) => {
                release_held(&decoder, &device, at_now(), &tx);
                return;
            }
            Ok(n) => n.min(buf.len()),
        };
        let at = at_now();
        let raw = buf[..n].to_vec();
        for input in decoder.decode(&raw) {
            tx.push(device.clone(), at, input, raw.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button(n: u8, pressed: bool) -> SurfaceInput {
        SurfaceInput::Button { control: ControlId::hid_named(&format!("button{n}")), pressed }
    }

    fn shuttle(position: i8) -> SurfaceInput {
        SurfaceInput::Shuttle { control: ControlId::hid_named("shuttle"), position, range: 7 }
    }

    fn jog(delta: i32) -> SurfaceInput {
        SurfaceInput::Jog { control: ControlId::hid_named("jog"), delta }
    }

    #[test]
    fn reports_become_inputs() {
        let mut d = ShuttleDecoder::new(15);
        assert_eq!(d.decode(&[0, 200, 0, 0, 0]), vec![], "the first report sets the jog's count");
        assert_eq!(d.decode(&[0, 201, 0, 0, 0]), vec![jog(1)]);
        assert_eq!(d.decode(&[0, 198, 0, 0, 0]), vec![jog(-3)]);
        assert_eq!(d.decode(&[3, 198, 0, 0, 0]), vec![shuttle(3)]);
        assert_eq!(d.decode(&[0xf9, 198, 0, 0, 0]), vec![shuttle(-7)]);
        assert_eq!(d.decode(&[0, 198, 0, 0, 0]), vec![shuttle(0)], "back in the middle is reported");
        assert_eq!(d.decode(&[0, 198, 0, 0b0000_0101, 0]), vec![button(1, true), button(3, true)]);
        assert_eq!(d.decode(&[0, 198, 0, 0b0000_0100, 0x40]), vec![button(1, false), button(15, true)]);
        assert_eq!(d.decode(&[2, 199, 0, 0b0000_0100, 0x40]), vec![shuttle(2), jog(1)], "several changes in one report");
        assert_eq!(d.decode(&[2, 199, 0, 0b0000_0100, 0x40]), vec![], "nothing changed");
        assert_eq!(d.decode(&[2, 199]), vec![], "a short report is ignored");
    }

    #[test]
    fn the_jog_counter_wraps() {
        let mut d = ShuttleDecoder::new(15);
        let _ = d.decode(&[0, 254, 0, 0, 0]);
        assert_eq!(d.decode(&[0, 1, 0, 0, 0]), vec![jog(3)], "254 → 1 is three steps forward");
        assert_eq!(d.decode(&[0, 253, 0, 0, 0]), vec![jog(-4)]);
    }

    #[test]
    fn buttons_beyond_the_device_are_ignored() {
        let mut d = ShuttleDecoder::new(5);
        let _ = d.decode(&[0, 0, 0, 0, 0]);
        assert_eq!(d.decode(&[0, 0, 0, 0b0011_0000, 0]), vec![button(5, true)], "ShuttleXpress has five");
        let mut first = ShuttleDecoder::new(15);
        assert_eq!(first.decode(&[4, 9, 0, 1, 0]), vec![button(1, true), shuttle(4)], "held from the start");
        assert_eq!(first.decode(&[0x80, 9, 0, 1, 0]), vec![shuttle(-7)], "out of range is clamped");
    }

    #[test]
    fn a_missing_node_says_why() {
        let e = HidReader::start(DeviceId::hid(0x0b33, 0x0030, "usb-1"), Path::new("/nonexistent/cfa4/hidraw0"), 15).err().unwrap_or_default();
        assert!(e.contains("cannot open"), "{e}");
    }

    /// A regular file stands in for the device node: its reports are read, then the end of the
    /// file ends the reader like an unplugged device.
    #[test]
    fn reads_reports_until_the_device_goes() {
        let path = std::env::temp_dir().join(format!("cfa4-surface-hidraw-{}", std::process::id()));
        assert!(std::fs::write(&path, [0u8, 10, 0, 0, 0]).is_ok());
        let r = HidReader::start(DeviceId::hid(0x0b33, 0x0030, "usb-1"), &path, 15);
        let Ok(r) = r else { panic!("open") };
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while r.running() && Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!r.running(), "the end of the file ends the reader");
        assert!(r.drain(10).is_empty(), "one report only sets the jog's count");
        let _ = std::fs::remove_file(&path);
    }
}
