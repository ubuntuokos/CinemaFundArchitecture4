//! `surface.devices`: the control surfaces of this machine for Preferences ▸ Control Surfaces
//! (CFA4-CONTROL-SURFACE-001, `docs/adr/ADR-0012-control-surface.md`).
//!
//! The device list comes from `cfa4-surface` (L5: ALSA sequencer MIDI ports, hidraw jog/shuttle
//! devices); each device carries the user's choices from `controlSurfaces.devices` (enabled,
//! profile, falling back to the driver's default profile). A machine without an ALSA sequencer or
//! hidraw gives a shorter list and a note in `problems`, never an error.

use cfa4_engine::Session;
use cfa4_engine::control_surfaces::PROFILES;
use serde_json::{Value, json};

/// The reply of `surface.devices`.
pub fn devices(s: &Session) -> Value {
    with_settings(s, cfa4_surface::discover())
}

fn with_settings(s: &Session, found: cfa4_surface::Discovery) -> Value {
    let prefs = &s.prefs.control_surfaces;
    let devices: Vec<Value> = found
        .devices
        .iter()
        .map(|d| {
            let set = prefs.device(&d.device.0);
            let profile = if set.profile.is_empty() {
                PROFILES.iter().find(|p| p.driver == d.driver && p.default).map(|p| p.id).unwrap_or_default().to_string()
            } else {
                set.profile.clone()
            };
            let mut v = serde_json::to_value(d).unwrap_or_else(|_| json!({}));
            if let Some(o) = v.as_object_mut() {
                o.insert("enabled".into(), json!(set.enabled));
                o.insert("profile".into(), json!(profile));
            }
            v
        })
        .collect();
    json!({"enabled": prefs.enabled, "devices": devices, "problems": found.problems})
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfa4_engine::control_surfaces::DeviceSetting;
    use cfa4_surface::{DeviceInfo, Discovery};
    use cfa4_surface_model::{DeviceId, DeviceState, SurfaceProtocol};

    fn dev(id: &str, driver: &str) -> DeviceInfo {
        DeviceInfo {
            device: DeviceId(id.into()),
            name: id.into(),
            protocol: if driver == "hid-shuttle" { SurfaceProtocol::Hid } else { SurfaceProtocol::Midi },
            driver: driver.into(),
            state: DeviceState::Online,
            port: "24:0".into(),
            vid: None,
            pid: None,
            error: None,
        }
    }

    #[test]
    fn devices_carry_the_users_choices() {
        let mut s = Session::default();
        s.prefs.control_surfaces.devices.push(DeviceSetting { device: "midi:a:b".into(), enabled: false, profile: String::new() });
        let found = Discovery { devices: vec![dev("midi:a:b", "generic-midi"), dev("hid:0b33:0030:usb-1", "hid-shuttle")], problems: vec!["MIDI: x".into()] };
        let v = with_settings(&s, found);
        assert_eq!(v["enabled"], true);
        assert_eq!(v["devices"][0]["enabled"], false);
        assert_eq!(v["devices"][0]["profile"], "cfa4.generic-midi");
        assert_eq!(v["devices"][0]["protocol"], "midi");
        assert_eq!(v["devices"][1]["enabled"], true);
        assert_eq!(v["devices"][1]["profile"], "cfa4.shuttlepro");
        assert_eq!(v["devices"][1]["state"], "online");
        assert_eq!(v["problems"][0], "MIDI: x");
    }

    /// In CI there is neither a sequencer nor hidraw: the reply is still a list.
    #[test]
    fn real_discovery_is_a_list() {
        let s = Session::default();
        let v = devices(&s);
        assert!(v["devices"].is_array());
        assert!(v["problems"].is_array());
    }
}
