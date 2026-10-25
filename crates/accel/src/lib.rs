//! Vendor-neutral acceleration planning (layer L1, no UI, no FFI, no `unsafe`).
//!
//! CFA4 must run accelerated on AMD, Intel and NVIDIA GPUs (and NPUs where they help) without
//! depending on any one vendor, and must always work on the CPU. This crate is the single place
//! that decides *which backend is tried for which job, in which order*:
//!
//! 1. [`probe`] reads the device inventory from Linux sysfs (`/sys/class/drm/renderD*`,
//!    `/sys/class/accel/accel*`): PCI vendor and device ids and the kernel driver of every GPU
//!    render node and NPU. Plain file reads, bounded, never a panic.
//! 2. [`catalog::CATALOG`] lists every backend the workspace has (or plans) per [`Workload`],
//!    with its [`Maturity`], the crate that implements it and whether it is locked to a vendor.
//! 3. [`plan`] turns inventory + catalogue + the user's [`Settings`] into an ordered fallback
//!    chain for one workload, always ending with [`Backend::Cpu`], and says why every other
//!    backend was skipped.
//! 4. [`coverage`] reports per workload whether an implemented accelerator exists that is *not*
//!    locked to a vendor: the vendor-neutrality gaps still to close.
//!
//! The chain is a list of candidates, not a promise: the backend's own factory (for example the
//! VA-API or NVDEC decoder in `cfa4-platform`) still verifies the hardware at run time and
//! declines what it cannot do, and the next entry of the chain runs instead.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod catalog;
pub mod inventory;
pub mod plan;
pub mod probe;
pub mod vendor;

pub use catalog::{Backend, Maturity, Unit, Workload};
pub use inventory::{Device, DeviceKind, Inventory};
pub use plan::{Coverage, Plan, Preference, Settings, Skip, coverage, plan, plan_all};
pub use probe::probe;
pub use vendor::Vendor;
