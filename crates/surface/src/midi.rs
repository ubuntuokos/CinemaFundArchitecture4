//! MIDI ports of the ALSA sequencer.
//!
//! A port is a control surface candidate when it sends MIDI that others may subscribe to
//! (`READ | SUBS_READ`). Left out: the System client (0), `Midi Through` (it only echoes) and
//! CFA4's own clients (named `CFA4`, such as the MIDI Timecode ports of `cfa4-tcsync-io`).

use crate::DeviceInfo;
use alsa::seq::{ClientIter, PortCap, PortIter, Seq};
use cfa4_surface_model::{DeviceId, DeviceState, SurfaceProtocol};

/// One sequencer port as the sequencer reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Port {
    pub client: i32,
    pub port: i32,
    pub client_name: String,
    pub port_name: String,
    /// Sends MIDI that can be subscribed to.
    pub readable: bool,
    /// Accepts MIDI (feedback can be sent to it).
    pub writable: bool,
}

/// Whether a port is offered as a control surface.
pub fn is_candidate(p: &Port, own_client: i32) -> bool {
    p.readable && p.client != 0 && p.client != own_client && p.client_name != "Midi Through" && p.client_name != "CFA4"
}

/// The [`DeviceInfo`] of a candidate port.
pub fn info(p: &Port) -> DeviceInfo {
    let name = if p.port_name.is_empty() || p.port_name == p.client_name { p.client_name.clone() } else { format!("{} – {}", p.client_name, p.port_name) };
    DeviceInfo {
        device: DeviceId(format!("midi:{}:{}", p.client_name, p.port_name)),
        name,
        protocol: SurfaceProtocol::Midi,
        driver: "generic-midi".into(),
        state: DeviceState::Online,
        port: format!("{}:{}", p.client, p.port),
        vid: None,
        pid: None,
        error: None,
    }
}

/// The candidate ports, in sequencer order. An error when there is no ALSA sequencer.
pub fn ports() -> Result<Vec<DeviceInfo>, String> {
    let seq = Seq::open(None, None, true).map_err(|e| format!("MIDI: no ALSA sequencer ({e}); MIDI controllers are not listed"))?;
    let own = seq.client_id().unwrap_or(-1);
    let mut out = Vec::new();
    for c in ClientIter::new(&seq) {
        let client = c.get_client();
        let client_name = c.get_name().unwrap_or_default().to_string();
        for p in PortIter::new(&seq, client) {
            let caps = p.get_capability();
            let port = Port {
                client,
                port: p.get_port(),
                client_name: client_name.clone(),
                port_name: p.get_name().unwrap_or_default().to_string(),
                readable: caps.contains(PortCap::READ | PortCap::SUBS_READ),
                writable: caps.contains(PortCap::WRITE | PortCap::SUBS_WRITE),
            };
            if is_candidate(&port, own) {
                out.push(info(&port));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(client: i32, client_name: &str, port_name: &str, readable: bool) -> Port {
        Port { client, port: 0, client_name: client_name.into(), port_name: port_name.into(), readable, writable: true }
    }

    #[test]
    fn candidates() {
        assert!(is_candidate(&port(24, "nanoKONTROL2", "nanoKONTROL2 _ CTRL", true), 128));
        assert!(!is_candidate(&port(0, "System", "Announce", true), 128));
        assert!(!is_candidate(&port(14, "Midi Through", "Midi Through Port-0", true), 128));
        assert!(!is_candidate(&port(128, "x", "y", true), 128), "our own client");
        assert!(!is_candidate(&port(129, "CFA4", "Timecode Out", true), 128), "CFA4's timecode client");
        assert!(!is_candidate(&port(24, "Synth", "In", false), 128), "input-only ports send nothing");
    }

    #[test]
    fn device_info() {
        let i = info(&port(24, "nanoKONTROL2", "nanoKONTROL2 _ CTRL", true));
        assert_eq!(i.device.0, "midi:nanoKONTROL2:nanoKONTROL2 _ CTRL");
        assert_eq!(i.name, "nanoKONTROL2 – nanoKONTROL2 _ CTRL");
        assert_eq!(i.port, "24:0");
        assert_eq!(i.driver, "generic-midi");
        assert_eq!(info(&port(20, "X-Touch MINI", "X-Touch MINI", true)).name, "X-Touch MINI");
    }

    /// In a container without `/dev/snd/seq` this is an error; on a desktop a list. Never a panic.
    #[test]
    fn listing_does_not_panic() {
        match ports() {
            Ok(list) => assert!(list.iter().all(|d| d.device.0.starts_with("midi:"))),
            Err(e) => assert!(e.contains("ALSA sequencer")),
        }
    }
}
