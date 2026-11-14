//! A backend without hardware, for the adapter loop's tests and for supervisor tests in other
//! crates (feature `testing`).
//!
//! The test drives it through a [`FakeHandle`]: plug and unplug devices, queue key input, make
//! opening or reading fail, or make the next image write panic (to check that a panic in a device
//! library stays inside that device's task). Every call the loop makes is logged.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::profile::Profile;
use crate::wire::ErrorCode;
use crate::{AdapterError, Backend, DeviceIo, Found, Opened, RawInput};

/// A call the loop made on a fake device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FakeOp {
    Open,
    /// Device index and image size.
    SetImage(u8, u32, u32),
    Clear(Option<u8>),
    Brightness(u8),
    Sleep,
    Close,
}

#[derive(Default)]
struct State {
    present: Vec<Found>,
    inputs: HashMap<String, VecDeque<RawInput>>,
    fail_open: HashMap<String, ErrorCode>,
    fail_read: HashSet<String>,
    panic_on_image: HashSet<String>,
    log: Vec<(String, FakeOp)>,
    serial: Option<String>,
}

/// The test's side of a [`FakeBackend`].
#[derive(Clone, Default)]
pub struct FakeHandle(Arc<Mutex<State>>);

impl FakeHandle {
    fn state(&self) -> MutexGuard<'_, State> {
        // A panicking fake device must not poison the test's view of the log.
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Plugs in a device at `path` that matches profile `profile`.
    pub fn plug(&self, path: &str, profile: usize) {
        let mut s = self.state();
        s.present.retain(|f| f.path != path);
        s.present.push(Found { path: path.into(), profile });
    }

    pub fn unplug(&self, path: &str) {
        self.state().present.retain(|f| f.path != path);
    }

    /// Queues a key change; `device_index` is 0-based.
    pub fn press(&self, path: &str, device_index: u8, down: bool) {
        self.state().inputs.entry(path.into()).or_default().push_back((device_index, down));
    }

    pub fn fail_open(&self, path: &str, code: ErrorCode) {
        self.state().fail_open.insert(path.into(), code);
    }

    pub fn allow_open(&self, path: &str) {
        self.state().fail_open.remove(path);
    }

    pub fn fail_read(&self, path: &str) {
        self.state().fail_read.insert(path.into());
    }

    pub fn panic_on_image(&self, path: &str) {
        self.state().panic_on_image.insert(path.into());
    }

    /// The serial number devices report when opened.
    pub fn set_serial(&self, serial: &str) {
        self.state().serial = Some(serial.into());
    }

    /// Every call made so far on the device at `path`.
    pub fn ops(&self, path: &str) -> Vec<FakeOp> {
        self.state().log.iter().filter(|(p, _)| p == path).map(|(_, o)| o.clone()).collect()
    }
}

/// A [`Backend`] whose devices exist only in a [`FakeHandle`].
#[derive(Clone, Default)]
pub struct FakeBackend {
    pub handle: FakeHandle,
}

impl FakeBackend {
    pub fn new() -> (Arc<FakeBackend>, FakeHandle) {
        let b = FakeBackend::default();
        let h = b.handle.clone();
        (Arc::new(b), h)
    }
}

impl Backend for FakeBackend {
    type Device = FakeDevice;

    async fn scan(&self, profiles: &[Profile]) -> Result<Vec<Found>, AdapterError> {
        Ok(self.handle.state().present.iter().filter(|f| f.profile < profiles.len()).cloned().collect())
    }

    async fn open(&self, found: &Found, _profile: &Profile) -> Result<(FakeDevice, Opened), AdapterError> {
        let mut s = self.handle.state();
        if let Some(code) = s.fail_open.get(&found.path) {
            return Err(AdapterError::new(*code, format!("cannot open {}", found.path)));
        }
        s.log.push((found.path.clone(), FakeOp::Open));
        let opened = Opened { firmware: Some("fake-1.0".into()), serial: s.serial.clone() };
        Ok((FakeDevice { handle: self.handle.clone(), path: found.path.clone() }, opened))
    }
}

/// One open fake device.
pub struct FakeDevice {
    handle: FakeHandle,
    path: String,
}

impl FakeDevice {
    fn log(&self, op: FakeOp) {
        self.handle.state().log.push((self.path.clone(), op));
    }
}

impl DeviceIo for FakeDevice {
    async fn read(&mut self, timeout: Duration) -> Result<Vec<RawInput>, AdapterError> {
        let got: Vec<RawInput> = {
            let mut s = self.handle.state();
            if s.fail_read.contains(&self.path) {
                return Err(AdapterError::new(ErrorCode::IoError, "the device stopped answering"));
            }
            s.inputs.get_mut(&self.path).map(|q| q.drain(..).collect()).unwrap_or_default()
        };
        if got.is_empty() {
            tokio::time::sleep(timeout).await;
        }
        Ok(got)
    }

    #[allow(clippy::panic)] // the point of `panic_on_image`: a device library that panics
    async fn set_image(&mut self, device_index: u8, image: image::RgbaImage) -> Result<(), AdapterError> {
        if self.handle.state().panic_on_image.contains(&self.path) {
            panic!("fake device library panic");
        }
        self.log(FakeOp::SetImage(device_index, image.width(), image.height()));
        Ok(())
    }

    async fn clear(&mut self, device_index: Option<u8>) -> Result<(), AdapterError> {
        self.log(FakeOp::Clear(device_index));
        Ok(())
    }

    async fn set_brightness(&mut self, percent: u8) -> Result<(), AdapterError> {
        self.log(FakeOp::Brightness(percent));
        Ok(())
    }

    async fn sleep(&mut self) -> Result<(), AdapterError> {
        self.log(FakeOp::Sleep);
        Ok(())
    }

    async fn close(&mut self) {
        self.log(FakeOp::Close);
    }
}
