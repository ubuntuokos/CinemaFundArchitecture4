//! CFA4 compute router (layer L1).
//!
//! Every accelerated task in CFA4 (showing and compositing video, hardware decoding and encoding,
//! local AI inference, general compute) asks this crate which **route** to take: which backend on
//! which device. The answer comes from three inputs:
//!
//! - the **routing table** ([`Table`]): an ordered list of backends per task family, with
//!   per-task rows on top (the shipped `routing.toml`, overridden by the user's file);
//! - the **inventory** ([`Inventory`]): the devices found on this machine (GPUs, NPUs, the CPU);
//! - the **preferences** ([`Preferences`]): the user's choice per family (Automatic, one device,
//!   or CPU only).
//!
//! The [`Router`] walks the order and takes the first route whose backend is registered and
//! enabled, whose device is present and allowed, whose backend accepts the task on that device,
//! and which fits the memory budget. Every route it passes over is kept with its reason
//! ([`Declined`]), so the choice can be shown in the UI and in `perf.stats` (`route.<task>`).
//! When a running route fails, [`Router::next_after`] continues from the next one.
//!
//! The [`catalog`] says, for every backend the table names, whether it works today or is planned
//! and whether it is locked to one vendor's hardware (NVDEC, NVENC). [`coverage`] turns that into
//! the vendor-neutrality report: per family, whether an implemented accelerator exists that runs
//! on any vendor's GPU. [`Preferences::allow_vendor_locked`] switches the vendor-locked backends
//! off, to prove that every family still runs without them.
//!
//! The rules (vendor-neutral path and CPU fallback for every family, CFA4's own engine first in
//! the codec families) are in `docs/adr/ADR-0004-compute-routing.md`.
//!
//! This crate only decides. It loads no driver and contains no `unsafe` code: backends are
//! registered by the crates that own them (`crates/platform`, `crates/gpu`, `crates/speech`,
//! `crates/tts`, `crates/gstreamer`), and [`probe`] reads only what Linux shows in `/sys`.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

use std::fmt;

use serde::{Deserialize, Serialize};

pub mod catalog;
pub mod probe;
pub mod report;
mod router;
mod table;

pub use catalog::{Coverage, Maturity, coverage};
pub use router::{Backend, Chain, Decision, Declined, Reason, Route, Router};
pub use table::{Table, TaskRule};

/// Longest identifier (task, backend, device) accepted from any input.
pub const MAX_ID_LEN: usize = 64;

/// Errors of the router. Every failure is reported, never a panic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// An identifier is empty, too long or has characters outside `a-z 0-9 . _ - :`.
    #[error("invalid identifier {0:?}")]
    InvalidId(String),
    /// The routing table could not be read.
    #[error("routing table: {0}")]
    Table(String),
    /// No route in the order could take the task. `declined` says why each one was passed over.
    #[error("no route for task {task}")]
    NoRoute { task: TaskId, declined: Vec<Declined> },
}

fn check_id(s: &str) -> Result<(), Error> {
    let ok =
        !s.is_empty() && s.len() <= MAX_ID_LEN && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-' | b':'));
    if ok { Ok(()) } else { Err(Error::InvalidId(s.chars().take(MAX_ID_LEN).collect())) }
}

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Checks the identifier (`a-z 0-9 . _ - :`, 1 to [`MAX_ID_LEN`] bytes).
            pub fn new(s: impl Into<String>) -> Result<Self, Error> {
                let s = s.into();
                check_id(&s)?;
                Ok(Self(s))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = Error;
            fn try_from(s: String) -> Result<Self, Error> {
                Self::new(s)
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> String {
                id.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(
    /// A task, named by family and job: `video.decode.hevc`, `ai.speech.whisper`.
    TaskId
);
id_type!(
    /// A backend: `nvdec`, `vaapi`, `burn-vulkan`, `candle-cpu`.
    BackendId
);
id_type!(
    /// A device on this machine: `cpu`, `gpu:renderD128`, `npu:accel0`.
    DeviceId
);

/// The four families of accelerated work, plus general compute. Each has its own default order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    /// Preview, compositing, effects, scopes, GPU export rendering.
    Display,
    /// Hardware video decoding.
    CodecDecode,
    /// Hardware video encoding.
    CodecEncode,
    /// Local AI inference (speech, voices, dubbing, upscaling, masks). The only family an NPU serves.
    Ai,
    /// General compute (denoise, stabilisation, optical flow, audio processing).
    Compute,
}

impl Family {
    pub const ALL: [Family; 5] = [Family::Display, Family::CodecDecode, Family::CodecEncode, Family::Ai, Family::Compute];

    /// The key used in the routing table.
    pub fn key(self) -> &'static str {
        match self {
            Family::Display => "display",
            Family::CodecDecode => "codec_decode",
            Family::CodecEncode => "codec_encode",
            Family::Ai => "ai",
            Family::Compute => "compute",
        }
    }

    pub fn from_key(key: &str) -> Option<Family> {
        Family::ALL.into_iter().find(|f| f.key() == key)
    }
}

/// One concrete job to route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub family: Family,
}

impl Task {
    pub fn new(id: &str, family: Family) -> Result<Task, Error> {
        Ok(Task { id: TaskId::new(id)?, family })
    }
}

/// GPU and NPU makers CFA4 tells apart. Routing never depends on one of them alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
    Other,
}

impl Vendor {
    /// From a PCI vendor id (`/sys/.../device/vendor`). AMD has two: ATI's (graphics) and its own
    /// (some APU functions and the NPUs).
    pub fn from_pci(id: u16) -> Vendor {
        match id {
            0x10de => Vendor::Nvidia,
            0x1002 | 0x1022 => Vendor::Amd,
            0x8086 => Vendor::Intel,
            _ => Vendor::Other,
        }
    }

    /// From the kernel driver, for devices without a PCI id (platform devices).
    pub fn from_driver(driver: &str) -> Vendor {
        match driver {
            "nvidia" | "nouveau" | "nvidia-drm" => Vendor::Nvidia,
            "amdgpu" | "radeon" | "amdxdna" => Vendor::Amd,
            "i915" | "xe" | "intel_vpu" => Vendor::Intel,
            _ => Vendor::Other,
        }
    }

    /// A short display name.
    pub fn name(self) -> &'static str {
        match self {
            Vendor::Nvidia => "NVIDIA",
            Vendor::Amd => "AMD",
            Vendor::Intel => "Intel",
            Vendor::Other => "Other",
        }
    }
}

/// Parses a PCI id as sysfs writes it (`0x10de\n`). `None` for anything that is not a
/// hexadecimal 16-bit number (no sign, no spaces inside, at most 4 digits after the prefix).
pub fn parse_pci_id(text: &str) -> Option<u16> {
    let t = text.trim();
    let digits = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    if digits.is_empty() || digits.len() > 4 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u16::from_str_radix(digits, 16).ok()
}

/// What kind of device it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Cpu,
    Gpu,
    Npu,
}

/// One device on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub id: DeviceId,
    pub kind: DeviceKind,
    pub vendor: Vendor,
    /// Human-readable name, for the UI (`NVIDIA RTX A1000`).
    pub name: String,
    /// The kernel driver bound to the device (`amdgpu`, `i915`, `xe`, `nvidia`, `nouveau`,
    /// `intel_vpu`, `amdxdna`), when known.
    #[serde(default)]
    pub driver: Option<String>,
    /// The PCI device id, when known.
    #[serde(default)]
    pub pci_device: Option<u16>,
    /// Free device memory in MiB when a backend reported it. `None`: unknown, and the memory
    /// budget of a task rule is not applied.
    pub free_memory_mb: Option<u64>,
}

impl Device {
    /// The CPU, which every machine has.
    pub fn cpu() -> Device {
        Device {
            id: DeviceId("cpu".into()),
            kind: DeviceKind::Cpu,
            vendor: Vendor::Other,
            name: "CPU".into(),
            driver: None,
            pci_device: None,
            free_memory_mb: None,
        }
    }

    fn driver_is(&self, names: &[&str]) -> bool {
        self.driver.as_deref().is_some_and(|d| names.contains(&d))
    }

    /// An NVIDIA GPU on NVIDIA's proprietary driver, which NVDEC, NVENC and CUDA need (nouveau
    /// has none of them).
    pub fn is_nvidia_proprietary(&self) -> bool {
        self.kind == DeviceKind::Gpu && self.vendor == Vendor::Nvidia && self.driver_is(&["nvidia"])
    }

    /// A GPU whose kernel driver has a VA-API user-space driver (Mesa radeonsi / nouveau, Intel
    /// iHD / i965). NVIDIA's proprietary driver is left out: NVDEC serves it.
    pub fn has_vaapi_driver(&self) -> bool {
        self.kind == DeviceKind::Gpu && self.driver_is(VAAPI_KERNEL_DRIVERS)
    }
}

/// Kernel drivers whose GPUs have a VA-API user-space driver on Ubuntu.
pub const VAAPI_KERNEL_DRIVERS: &[&str] = &["amdgpu", "radeon", "i915", "xe", "nouveau"];

/// The devices found on this machine. Always holds the CPU.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    devices: Vec<Device>,
    /// What the probe could not read or left out (for the log and `perf.stats`).
    #[serde(default)]
    notes: Vec<String>,
}

/// Most devices an inventory keeps (hostile `/sys` contents cannot grow it without bound).
pub const MAX_DEVICES: usize = 64;
/// Most notes an inventory keeps, and their longest length in bytes.
pub const MAX_NOTES: usize = 32;
const MAX_NOTE_LEN: usize = 256;

impl Default for Inventory {
    fn default() -> Self {
        Inventory { devices: vec![Device::cpu()], notes: Vec::new() }
    }
}

impl Inventory {
    /// An inventory with the CPU and the given devices. A second device with an id already present
    /// is dropped, and so is anything past [`MAX_DEVICES`].
    pub fn new(devices: impl IntoIterator<Item = Device>) -> Inventory {
        let mut inv = Inventory::default();
        for d in devices {
            inv.add(d);
        }
        inv
    }

    /// Adds a device; returns false when it was dropped (duplicate id or inventory full).
    pub fn add(&mut self, device: Device) -> bool {
        if self.devices.len() >= MAX_DEVICES || self.get(&device.id).is_some() {
            return false;
        }
        self.devices.push(device);
        true
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    /// The GPUs.
    pub fn gpus(&self) -> impl Iterator<Item = &Device> {
        self.devices.iter().filter(|d| d.kind == DeviceKind::Gpu)
    }

    /// The NPUs.
    pub fn npus(&self) -> impl Iterator<Item = &Device> {
        self.devices.iter().filter(|d| d.kind == DeviceKind::Npu)
    }

    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// Records a note (bounded in number and length).
    pub fn note(&mut self, text: impl Into<String>) {
        if self.notes.len() < MAX_NOTES {
            let mut text: String = text.into();
            if text.len() > MAX_NOTE_LEN {
                let mut end = MAX_NOTE_LEN;
                while !text.is_char_boundary(end) {
                    end = end.saturating_sub(1);
                }
                text.truncate(end);
            }
            self.notes.push(text);
        }
    }

    pub fn get(&self, id: &DeviceId) -> Option<&Device> {
        self.devices.iter().find(|d| &d.id == id)
    }

    /// Updates the free memory a backend measured on a device. Unknown device: false.
    pub fn set_free_memory(&mut self, id: &DeviceId, mb: Option<u64>) -> bool {
        match self.devices.iter_mut().find(|d| &d.id == id) {
            Some(d) => {
                d.free_memory_mb = mb;
                true
            }
            None => false,
        }
    }
}

/// The user's choice for one family (Settings ▸ Performance).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preference {
    /// The table decides.
    #[default]
    Automatic,
    /// Only this device (and the CPU as the last resort, so the task still runs).
    Device(DeviceId),
    /// Only the CPU.
    CpuOnly,
}

/// Preferences for every family. The default leaves everything to the table, except hardware
/// encoding, which stays opt-in: the built-in encoders give byte-identical files on every machine
/// (Export ▸ Hardware encoding, off by default).
///
/// Serde, so the UI, the control channel and MCP read and set them; fields left out keep their
/// default, unknown fields are an error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub display: Preference,
    pub codec_decode: Preference,
    pub codec_encode: Preference,
    pub ai: Preference,
    pub compute: Preference,
    /// Whether backends that run on one vendor's hardware only (NVDEC, NVENC, CUDA, OpenVINO NPU)
    /// may be chosen. Off gives a strictly vendor-neutral routing, to test that every family still
    /// runs without them. On by default.
    pub allow_vendor_locked: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Preferences {
            display: Preference::Automatic,
            codec_decode: Preference::Automatic,
            codec_encode: Preference::CpuOnly,
            ai: Preference::Automatic,
            compute: Preference::Automatic,
            allow_vendor_locked: true,
        }
    }
}

impl Preferences {
    pub fn get(&self, family: Family) -> &Preference {
        match family {
            Family::Display => &self.display,
            Family::CodecDecode => &self.codec_decode,
            Family::CodecEncode => &self.codec_encode,
            Family::Ai => &self.ai,
            Family::Compute => &self.compute,
        }
    }

    pub fn set(&mut self, family: Family, pref: Preference) {
        let slot = match family {
            Family::Display => &mut self.display,
            Family::CodecDecode => &mut self.codec_decode,
            Family::CodecEncode => &mut self.codec_encode,
            Family::Ai => &mut self.ai,
            Family::Compute => &mut self.compute,
        };
        *slot = pref;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_checked() {
        assert!(TaskId::new("ai.speech.whisper").is_ok());
        assert!(DeviceId::new("gpu:renderD128").is_err(), "upper case is refused");
        assert!(DeviceId::new("gpu:renderd128").is_ok());
        assert!(BackendId::new("").is_err());
        assert!(BackendId::new("a".repeat(MAX_ID_LEN + 1)).is_err());
        assert!(BackendId::new("nv dec").is_err());
        assert!(BackendId::new("ü").is_err());
        let long = "x".repeat(10_000);
        match BackendId::new(long) {
            Err(Error::InvalidId(s)) => assert_eq!(s.len(), MAX_ID_LEN, "the error does not echo huge input"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn families_round_trip_their_keys() {
        for f in Family::ALL {
            assert_eq!(Family::from_key(f.key()), Some(f));
        }
        assert_eq!(Family::from_key("video"), None);
    }

    #[test]
    fn inventory_always_has_the_cpu_and_stays_bounded() {
        let inv = Inventory::default();
        assert_eq!(inv.devices().len(), 1);
        assert_eq!(inv.devices()[0].kind, DeviceKind::Cpu);

        let gpu = |i: usize| Device {
            id: DeviceId::new(format!("gpu:{i}")).unwrap(),
            kind: DeviceKind::Gpu,
            vendor: Vendor::Amd,
            name: format!("GPU {i}"),
            driver: Some("amdgpu".into()),
            pci_device: None,
            free_memory_mb: None,
        };
        let mut inv = Inventory::new((0..200).map(gpu));
        assert_eq!(inv.devices().len(), MAX_DEVICES);
        assert!(!inv.add(gpu(0)), "duplicate ids are dropped");
        assert!(!inv.add(Device::cpu()), "a second CPU is a duplicate");
        assert!(inv.set_free_memory(&DeviceId::new("gpu:3").unwrap(), Some(512)));
        assert!(!inv.set_free_memory(&DeviceId::new("gpu:999").unwrap(), Some(512)));
    }

    #[test]
    fn hardware_encoding_stays_opt_in_by_default() {
        let p = Preferences::default();
        assert_eq!(p.get(Family::CodecEncode), &Preference::CpuOnly);
        assert_eq!(p.get(Family::CodecDecode), &Preference::Automatic);
        let mut p = p;
        p.set(Family::CodecEncode, Preference::Automatic);
        assert_eq!(p.get(Family::CodecEncode), &Preference::Automatic);
    }

    #[test]
    fn notes_are_bounded() {
        let mut inv = Inventory::default();
        for _ in 0..1000 {
            inv.note("é".repeat(1000));
        }
        assert_eq!(inv.notes().len(), MAX_NOTES);
        assert!(inv.notes().iter().all(|n| n.len() <= MAX_NOTE_LEN && !n.is_empty()));
    }

    #[test]
    fn devices_know_their_driver_family() {
        let gpu = |vendor, driver: &str| Device {
            id: DeviceId::new("gpu:x").unwrap(),
            kind: DeviceKind::Gpu,
            vendor,
            name: String::new(),
            driver: Some(driver.into()),
            pci_device: None,
            free_memory_mb: None,
        };
        assert!(gpu(Vendor::Nvidia, "nvidia").is_nvidia_proprietary());
        assert!(!gpu(Vendor::Nvidia, "nouveau").is_nvidia_proprietary());
        assert!(gpu(Vendor::Nvidia, "nouveau").has_vaapi_driver());
        assert!(!gpu(Vendor::Nvidia, "nvidia").has_vaapi_driver());
        for d in ["amdgpu", "radeon", "i915", "xe"] {
            assert!(gpu(Vendor::Other, d).has_vaapi_driver(), "{d}");
        }
        assert!(!Device::cpu().has_vaapi_driver() && !Device::cpu().is_nvidia_proprietary());
    }

    #[test]
    fn preferences_round_trip_and_hostile_json_is_an_error() {
        let p: Preferences = serde_json::from_str("{}").unwrap();
        assert_eq!(p, Preferences::default());
        let mut p = Preferences { allow_vendor_locked: false, ..Preferences::default() };
        p.set(Family::Ai, Preference::Device(DeviceId::new("npu:accel0").unwrap()));
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Preferences>(&json).unwrap(), p);
        let partial: Preferences = serde_json::from_str(r#"{"allow_vendor_locked":false}"#).unwrap();
        assert_eq!(partial.codec_encode, Preference::CpuOnly, "fields left out keep their default");
        assert!(!partial.allow_vendor_locked);
        for bad in
            [r#"{"mining":"automatic"}"#, r#"{"ai":"tpu"}"#, r#"{"ai":{"device":"GPU 0"}}"#, r#"{"ai":[]}"#, r#"{"allow_vendor_locked":"yes"}"#, "", "[1]"]
        {
            assert!(serde_json::from_str::<Preferences>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn pci_ids_parse_and_hostile_text_does_not() {
        assert_eq!(parse_pci_id("0x10de\n"), Some(0x10de));
        assert_eq!(parse_pci_id("  0X8086 "), Some(0x8086));
        assert_eq!(parse_pci_id("1002"), Some(0x1002));
        for bad in ["", "0x", "\n", "zz", "0x-1", "0x12345", "0x1 2", "0x+1", "+10de", "１２", "0x10de0x10de", "not hex"] {
            assert_eq!(parse_pci_id(bad), None, "{bad:?}");
        }
        assert_eq!(Vendor::from_driver("amdxdna"), Vendor::Amd);
        assert_eq!(Vendor::from_driver("intel_vpu"), Vendor::Intel);
        assert_eq!(Vendor::from_driver("virtio_gpu"), Vendor::Other);
    }

    #[test]
    fn pci_vendors() {
        assert_eq!(Vendor::from_pci(0x10de), Vendor::Nvidia);
        assert_eq!(Vendor::from_pci(0x1002), Vendor::Amd);
        assert_eq!(Vendor::from_pci(0x8086), Vendor::Intel);
        assert_eq!(Vendor::from_pci(0x1af4), Vendor::Other);
    }
}
