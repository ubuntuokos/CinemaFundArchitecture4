//! Reading the Contour jog/shuttle controllers through `/dev/hidrawN`.
//!
//! The ShuttlePRO v1/v2 and the ShuttleXpress send a 5-byte input report whenever something
//! changes: `[shuttle i8 (−7..=7), jog u8 (a counter that wraps at 256), 0, buttons lo, buttons
//! hi]` (CFA4-CONTROL-SURFACE-HID-EVALUATION §3; the reference machine measurement M3–M6 confirms
//! the details). [`ShuttleDecoder`] turns consecutive reports into [`SurfaceInput`]s: a button
//! press or release, the shuttle ring's new position (0 = back in the middle, which hidraw does
//! report, unlike evdev) and the jog wheel's steps since the last report.
//!
//! When the hidraw node cannot be read, the device's `/dev/input/eventN` node is the fallback
//! (HID evaluation §2, decision N-02): [`EvdevDecoder`] reads its `input_event` records without
//! `unsafe`. evdev never reports the shuttle's return to the middle, so a `SYN_REPORT` that
//! carries the jog but no shuttle value means position 0; when the jog counter is 0 at the same
//! moment the kernel sends nothing and the return is missed (the known evdev defect).
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

/// `EV_SYN`, `EV_KEY` and `EV_REL` event types and the codes the Contour devices use.
const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const SYN_REPORT: u16 = 0;
const REL_DIAL: u16 = 7;
const REL_WHEEL: u16 = 8;
/// `BTN_MISC`: hid-generic gives button n the code `BTN_MISC + n − 1` (confirmed by M7).
const BTN_MISC: u16 = 0x100;

/// The size of one `struct input_event` on this target: a `timeval` (two `long`s), `type`,
/// `code` and `value`.
pub const INPUT_EVENT_SIZE: usize = 2 * std::mem::size_of::<usize>() + 8;

/// Turns the evdev records of a ShuttlePRO / ShuttleXpress into inputs, one `SYN_REPORT` frame
/// at a time.
#[derive(Clone, Debug, Default)]
pub struct EvdevDecoder {
    buttons: u8,
    pending: Vec<u8>,
    shuttle: i8,
    jog: Option<u8>,
    held: u16,
    frame_keys: Vec<(u8, bool)>,
    frame_wheel: Option<i32>,
    frame_dial: Option<i32>,
}

impl EvdevDecoder {
    /// A decoder for a device with `buttons` buttons (13, 15 or 5).
    pub fn new(buttons: u8) -> Self {
        Self { buttons: buttons.min(16), ..Self::default() }
    }

    /// The inputs the records in `bytes` complete. A record cut at the end waits for the next call.
    pub fn decode(&mut self, bytes: &[u8]) -> Vec<SurfaceInput> {
        self.pending.extend_from_slice(bytes);
        let whole = self.pending.len() / INPUT_EVENT_SIZE * INPUT_EVENT_SIZE;
        let records: Vec<u8> = self.pending.drain(..whole).collect();
        let mut out = Vec::new();
        for r in records.chunks_exact(INPUT_EVENT_SIZE) {
            let t = INPUT_EVENT_SIZE - 8;
            let kind = u16::from_ne_bytes([r[t], r[t + 1]]);
            let code = u16::from_ne_bytes([r[t + 2], r[t + 3]]);
            let value = i32::from_ne_bytes([r[t + 4], r[t + 5], r[t + 6], r[t + 7]]);
            match (kind, code) {
                (EV_KEY, c) if c >= BTN_MISC && c < BTN_MISC + u16::from(self.buttons) && value != 2 => {
                    self.frame_keys.push((u8::try_from(c - BTN_MISC).unwrap_or(0), value != 0));
                }
                (EV_REL, REL_WHEEL) => self.frame_wheel = Some(value),
                (EV_REL, REL_DIAL) => self.frame_dial = Some(value),
                (EV_SYN, SYN_REPORT) => self.frame(&mut out),
                _ => {}
            }
        }
        out
    }

    fn frame(&mut self, out: &mut Vec<SurfaceInput>) {
        for (b, pressed) in std::mem::take(&mut self.frame_keys) {
            let bit = 1u16 << b;
            if (self.held & bit != 0) != pressed {
                self.held ^= bit;
                out.push(SurfaceInput::Button { control: ControlId::hid_named(&format!("button{}", b + 1)), pressed });
            }
        }
        let wheel = self.frame_wheel.take();
        let dial = self.frame_dial.take();
        // no shuttle value next to a jog value: the ring is back in the middle
        let shuttle = match (wheel, dial) {
            (Some(w), _) => Some(i8::try_from(w.clamp(-i32::from(SHUTTLE_RANGE), i32::from(SHUTTLE_RANGE))).unwrap_or(0)),
            (None, Some(_)) => Some(0),
            (None, None) => None,
        };
        if let Some(position) = shuttle
            && position != self.shuttle
        {
            self.shuttle = position;
            out.push(SurfaceInput::Shuttle { control: ControlId::hid_named("shuttle"), position, range: SHUTTLE_RANGE });
        }
        if let Some(d) = dial {
            let count = u8::try_from(d.rem_euclid(256)).unwrap_or(0);
            if let Some(old) = self.jog {
                let delta = i32::from(i8::from_ne_bytes([count.wrapping_sub(old)]));
                if delta != 0 {
                    out.push(SurfaceInput::Jog { control: ControlId::hid_named("jog"), delta });
                }
            }
            self.jog = Some(count);
        }
    }

    /// Buttons that are down, so an unplug can release them (F0 spec §9.6).
    pub fn held(&self) -> Vec<ControlId> {
        (0..self.buttons).filter(|b| self.held & (1u16 << b) != 0).map(|b| ControlId::hid_named(&format!("button{}", b + 1))).collect()
    }
}

/// The report format of a node: hidraw reports or evdev records.
#[derive(Clone, Debug)]
enum Decoder {
    Hidraw(ShuttleDecoder),
    Evdev(EvdevDecoder),
}

impl Decoder {
    /// evdev for a `/dev/input/eventN` node, hidraw otherwise.
    fn for_node(node: &Path, buttons: u8) -> Self {
        if is_evdev(node) { Decoder::Evdev(EvdevDecoder::new(buttons)) } else { Decoder::Hidraw(ShuttleDecoder::new(buttons)) }
    }

    fn decode(&mut self, bytes: &[u8]) -> Vec<SurfaceInput> {
        match self {
            Decoder::Hidraw(d) => d.decode(bytes),
            Decoder::Evdev(d) => d.decode(bytes),
        }
    }

    fn held(&self) -> Vec<ControlId> {
        match self {
            Decoder::Hidraw(d) => d.held(),
            Decoder::Evdev(d) => d.held(),
        }
    }
}

/// Whether `node` is an evdev node (`eventN`).
pub fn is_evdev(node: &Path) -> bool {
    node.file_name().is_some_and(|n| n.to_string_lossy().starts_with("event"))
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
    /// Opens `node` (a `/dev/hidrawN`, or the `/dev/input/eventN` fallback) and starts reading it; an error that says why when the
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
            .spawn({
                let decoder = Decoder::for_node(node, buttons);
                move || run(file, device, decoder, tx, &s)
            })
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

fn release_held(decoder: &Decoder, device: &DeviceId, at: MonoNs, tx: &lane::LaneIn) {
    for control in decoder.held() {
        tx.push(device.clone(), at, SurfaceInput::Button { control, pressed: false }, Vec::new());
    }
}

fn run(mut file: File, device: DeviceId, mut decoder: Decoder, tx: lane::LaneIn, stop: &AtomicBool) {
    let started = Instant::now();
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

    /// One `input_event` record with a zero time stamp.
    fn ev(kind: u16, code: u16, value: i32) -> Vec<u8> {
        let mut r = vec![0u8; INPUT_EVENT_SIZE - 8];
        r.extend_from_slice(&kind.to_ne_bytes());
        r.extend_from_slice(&code.to_ne_bytes());
        r.extend_from_slice(&value.to_ne_bytes());
        r
    }

    fn syn() -> Vec<u8> {
        ev(EV_SYN, SYN_REPORT, 0)
    }

    #[test]
    fn evdev_records_become_inputs() {
        let mut d = EvdevDecoder::new(15);
        assert_eq!(d.decode(&[ev(EV_REL, REL_DIAL, 200), syn()].concat()), vec![], "the first dial value sets the jog's count");
        assert_eq!(d.decode(&[ev(EV_REL, REL_DIAL, 201), syn()].concat()), vec![jog(1)]);
        assert_eq!(d.decode(&[ev(EV_REL, REL_WHEEL, 3), ev(EV_REL, REL_DIAL, 201), syn()].concat()), vec![shuttle(3)]);
        assert_eq!(d.decode(&[ev(EV_REL, REL_WHEEL, -9), ev(EV_REL, REL_DIAL, 199), syn()].concat()), vec![shuttle(-7), jog(-2)], "clamped");
        assert_eq!(d.decode(&[ev(EV_REL, REL_DIAL, 199), syn()].concat()), vec![shuttle(0)], "a jog value without a shuttle value is the middle");
        assert_eq!(d.decode(&[ev(EV_KEY, BTN_MISC, 1), ev(EV_KEY, BTN_MISC + 14, 1), syn()].concat()), vec![button(1, true), button(15, true)]);
        assert_eq!(d.decode(&[ev(EV_KEY, BTN_MISC, 2), syn()].concat()), vec![], "autorepeat is ignored");
        assert_eq!(d.held(), vec![ControlId::hid_named("button1"), ControlId::hid_named("button15")]);
        assert_eq!(d.decode(&[ev(EV_KEY, BTN_MISC, 0), syn()].concat()), vec![button(1, false)]);
        assert_eq!(d.decode(&[ev(EV_KEY, BTN_MISC + 15, 1), ev(4, 4, 9), syn()].concat()), vec![], "beyond the device, or another type");
    }

    #[test]
    fn evdev_records_may_arrive_cut() {
        let mut d = EvdevDecoder::new(15);
        let bytes = [ev(EV_REL, REL_DIAL, 254), syn(), ev(EV_REL, REL_DIAL, 1), syn()].concat();
        let (a, b) = bytes.split_at(INPUT_EVENT_SIZE * 2 + 5);
        assert_eq!(d.decode(a), vec![]);
        assert_eq!(d.decode(b), vec![jog(3)], "254 → 1 is three steps forward, across the cut");
        assert_eq!(d.decode(&ev(EV_REL, REL_DIAL, 255)), vec![], "nothing before the SYN_REPORT");
        assert_eq!(d.decode(&syn()), vec![jog(-2)]);
    }

    #[test]
    fn the_node_name_picks_the_decoder() {
        assert!(is_evdev(Path::new("/dev/input/event12")));
        assert!(!is_evdev(Path::new("/dev/hidraw0")));
        assert!(matches!(Decoder::for_node(Path::new("/dev/input/event3"), 15), Decoder::Evdev(_)));
        assert!(matches!(Decoder::for_node(Path::new("/dev/hidraw3"), 15), Decoder::Hidraw(_)));
    }

    /// The evdev records of a file named like an event node are read the same way.
    #[test]
    fn reads_evdev_records_until_the_device_goes() {
        let dir = std::env::temp_dir().join(format!("cfa4-surface-evdev-{}", std::process::id()));
        assert!(std::fs::create_dir_all(&dir).is_ok());
        let path = dir.join("event5");
        let records = [ev(EV_REL, REL_DIAL, 10), syn(), ev(EV_REL, REL_DIAL, 12), syn(), ev(EV_KEY, BTN_MISC + 1, 1), ev(EV_REL, REL_DIAL, 12), syn()].concat();
        assert!(std::fs::write(&path, records).is_ok());
        let r = HidReader::start(DeviceId::hid(0x0b33, 0x0030, "usb-1"), &path, 15);
        let Ok(r) = r else { panic!("open") };
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while r.running() && Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!r.running(), "the end of the file ends the reader");
        let inputs: Vec<SurfaceInput> = r.drain(10).into_iter().map(|q| q.input).collect();
        assert_eq!(inputs.len(), 3, "{inputs:?}");
        for want in [jog(2), button(2, true), button(2, false)] {
            assert!(inputs.contains(&want), "{want:?} in {inputs:?}; the held button is released at the end");
        }
        let _ = std::fs::remove_dir_all(&dir);
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
