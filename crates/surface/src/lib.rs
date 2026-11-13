//! Control surface device I/O (L5; CFA4-CONTROL-SURFACE-001, ADR 0012).
//!
//! This first part finds the devices a user can set up in Preferences ▸ Control Surfaces:
//!
//! - **MIDI** ([`midi`]): the ports of the ALSA sequencer that send MIDI (hardware controllers and
//!   PipeWire's MIDI bridge alike), without the system ports, `Midi Through` and CFA4's own client;
//! - **HID** ([`hid`]): Contour ShuttlePRO v1 / v2 and ShuttleXpress through `/sys/class/hidraw`.
//!
//! Discovery never fails as a whole: a machine without an ALSA sequencer (containers, CI) or
//! without `/sys/class/hidraw` gives an empty list for that part and a note in
//! [`Discovery::problems`]. Reading the devices, hot-plug and feedback come in the next steps.

pub mod hid;
pub mod midi;

use cfa4_surface_model::{DeviceId, DeviceState, SurfaceProtocol};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One device as Preferences lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub device: DeviceId,
    pub name: String,
    pub protocol: SurfaceProtocol,
    /// `hid-shuttle` or `generic-midi` (the profile driver).
    pub driver: String,
    pub state: DeviceState,
    /// Human-readable location: `20:0` for a MIDI port, `/dev/hidraw3` for a HID device.
    pub port: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vid: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u16>,
    /// Why the device cannot be used now (for example no read permission).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What [`discover`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovery {
    pub devices: Vec<DeviceInfo>,
    /// Parts that could not be searched (no ALSA sequencer, no hidraw), as text for the user.
    pub problems: Vec<String>,
}

/// Lists the MIDI ports and the supported HID devices of this machine.
pub fn discover() -> Discovery {
    discover_in(Path::new("/sys/class/hidraw"), Path::new("/dev"))
}

/// [`discover`] with the sysfs and device directories given (tests use a fake tree).
pub fn discover_in(sys_hidraw: &Path, dev: &Path) -> Discovery {
    let mut out = Discovery::default();
    match midi::ports() {
        Ok(mut ports) => out.devices.append(&mut ports),
        Err(e) => out.problems.push(e),
    }
    match hid::scan(sys_hidraw, dev) {
        Ok(mut found) => out.devices.append(&mut found),
        Err(e) => out.problems.push(e),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without a sequencer and without hidraw (CI containers) discovery is empty with notes,
    /// never a panic.
    #[test]
    fn discovery_never_fails() {
        let d = discover_in(Path::new("/nonexistent/cfa4/hidraw"), Path::new("/nonexistent/cfa4/dev"));
        assert!(d.problems.iter().any(|p| p.contains("hidraw")), "{:?}", d.problems);
        let j = serde_json::to_value(&d).unwrap_or_default();
        assert!(j["devices"].is_array());
    }
}
