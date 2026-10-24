//! The device inventory: the GPUs and NPUs of this machine as plain data, so planning is a pure
//! function that tests run without hardware.

use serde::{Deserialize, Serialize};

use crate::vendor::Vendor;

/// What kind of accelerator a device is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    /// A GPU with a DRM render node (`/dev/dri/renderD*`).
    Gpu,
    /// A neural processing unit on the kernel's compute-accelerator subsystem (`/dev/accel/accel*`).
    Npu,
}

/// One GPU or NPU.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub kind: DeviceKind,
    pub vendor: Vendor,
    /// PCI vendor and device ids; zero when sysfs reports none.
    pub pci_vendor: u32,
    pub pci_device: u32,
    /// The kernel driver bound to the device (`amdgpu`, `i915`, `xe`, `nvidia`, `nouveau`,
    /// `intel_vpu`, `amdxdna`…); empty when unknown.
    pub driver: String,
    /// The device node's name (`renderD128`, `accel0`).
    pub node: String,
}

impl Device {
    /// NVIDIA's proprietary kernel driver, which NVDEC / NVENC need (nouveau has neither).
    pub fn is_nvidia_proprietary(&self) -> bool {
        self.vendor == Vendor::Nvidia && self.driver == "nvidia"
    }

    /// Kernel drivers with a Mesa or Intel VA-API user-space driver (radeonsi, iHD / i965,
    /// nouveau). NVIDIA's proprietary driver is left out: NVDEC serves it.
    pub fn has_vaapi_driver(&self) -> bool {
        self.kind == DeviceKind::Gpu && VAAPI_KERNEL_DRIVERS.contains(&self.driver.as_str())
    }
}

/// Kernel drivers whose GPUs have a VA-API user-space driver on Ubuntu.
pub const VAAPI_KERNEL_DRIVERS: &[&str] = &["amdgpu", "radeon", "i915", "xe", "nouveau"];

/// Every GPU and NPU found, plus notes about entries that could not be read.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    pub devices: Vec<Device>,
    /// Human-readable notes for the log and `perf.stats` (unreadable entries, truncation).
    pub notes: Vec<String>,
}

impl Inventory {
    pub fn gpus(&self) -> impl Iterator<Item = &Device> {
        self.devices.iter().filter(|d| d.kind == DeviceKind::Gpu)
    }

    pub fn npus(&self) -> impl Iterator<Item = &Device> {
        self.devices.iter().filter(|d| d.kind == DeviceKind::Npu)
    }

    pub fn has_gpu(&self) -> bool {
        self.gpus().next().is_some()
    }

    pub fn has_npu(&self) -> bool {
        self.npus().next().is_some()
    }

    /// The vendors present, GPUs and NPUs alike, without duplicates, in a stable order.
    pub fn vendors(&self) -> Vec<Vendor> {
        let mut vendors: Vec<Vendor> = self.devices.iter().map(|d| d.vendor).collect();
        vendors.sort();
        vendors.dedup();
        vendors
    }
}
