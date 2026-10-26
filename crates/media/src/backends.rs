//! The supplementary media backends (FFmpeg, GStreamer; ADR 0005) as the rest of CFA4 sees them:
//! whether each is switched on, and a report for Settings ▸ Media Backends.
//!
//! The backend crates sit above the engine (layer L5), so the engine cannot call them. Instead
//! each backend registers a [`BackendEntry`] here at startup, and the engine reads the reports
//! (`media.backends`) and stores the user's on/off choice through [`set_backend_enabled`], which
//! the backends consult before opening a file. The switch exists before the backend registers:
//! preferences may be applied first.

use std::sync::RwLock;

use serde::Serialize;

/// One patent-restricted component of a backend (the Ubuntu model, `docs/licensing.md` §7).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestrictedItem {
    pub name: String,
    pub purpose: String,
    pub available: bool,
}

/// What a backend reports about itself, for the settings panel and diagnostics.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendReport {
    /// Short id, also the preference suffix and router id (`ffmpeg`, `gstreamer`).
    pub id: String,
    /// Display name (`FFmpeg`).
    pub name: String,
    /// What the backend is for (English; the GUI translates it).
    pub role: String,
    /// This build contains the backend (its Cargo feature was on).
    pub compiled_in: bool,
    /// The backend's libraries were found and accepted on this machine.
    pub available: bool,
    /// The user's switch ([`backend_enabled`]).
    pub enabled: bool,
    /// Library version, when available.
    pub version: Option<String>,
    /// Licence the libraries report, when available.
    pub license: Option<String>,
    /// Where the libraries were loaded from, when known.
    pub location: Option<String>,
    /// Why the backend is not available, in English.
    pub problem: Option<String>,
    /// What this installation of the backend can do (hardware acceleration types, elements…).
    pub capabilities: Vec<String>,
    /// Patent-restricted components and whether they are installed.
    pub restricted: Vec<RestrictedItem>,
    /// Package that installs the restricted components (`cfa4-restricted-extras`).
    pub restricted_package: Option<String>,
}

/// A registered backend.
#[derive(Clone, Copy, Debug)]
pub struct BackendEntry {
    pub id: &'static str,
    /// Builds the current report (cheap: no file is opened).
    pub report: fn() -> BackendReport,
}

/// Most backends registered (a guard against a registration loop).
const MAX_BACKENDS: usize = 16;

fn entries() -> &'static RwLock<Vec<BackendEntry>> {
    static R: RwLock<Vec<BackendEntry>> = RwLock::new(Vec::new());
    &R
}

fn disabled() -> &'static RwLock<Vec<String>> {
    static D: RwLock<Vec<String>> = RwLock::new(Vec::new());
    &D
}

/// Registers a backend's report after those registered before it. The same id twice keeps the
/// first registration.
pub fn register_backend(entry: BackendEntry) {
    let mut r = entries().write().unwrap_or_else(|e| e.into_inner());
    if r.len() < MAX_BACKENDS && !r.iter().any(|x| x.id == entry.id) {
        r.push(entry);
    }
}

/// The reports of the registered backends, in registration order (the order they are tried).
pub fn backend_reports() -> Vec<BackendReport> {
    let list = entries().read().unwrap_or_else(|e| e.into_inner()).clone();
    list.iter()
        .map(|e| {
            let mut r = (e.report)();
            r.id = e.id.to_string();
            r.enabled = backend_enabled(e.id);
            r
        })
        .collect()
}

/// Switches the backend `id` on or off (Settings ▸ Media Backends). Takes effect for the next
/// file opened; sources already open keep working.
pub fn set_backend_enabled(id: &str, enabled: bool) {
    if id.is_empty() || id.len() > 64 {
        return;
    }
    let mut d = disabled().write().unwrap_or_else(|e| e.into_inner());
    d.retain(|x| x != id);
    if !enabled {
        d.push(id.to_string());
    }
}

/// Whether the backend `id` is switched on (every backend is on until switched off).
pub fn backend_enabled(id: &str) -> bool {
    !disabled().read().unwrap_or_else(|e| e.into_inner()).iter().any(|x| x == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> BackendReport {
        BackendReport { name: "Test".into(), compiled_in: true, available: true, ..Default::default() }
    }

    #[test]
    fn registration_reports_and_switches() {
        register_backend(BackendEntry { id: "test-backend", report });
        register_backend(BackendEntry { id: "test-backend", report: BackendReport::default });
        let mine: Vec<_> = backend_reports().into_iter().filter(|r| r.id == "test-backend").collect();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].name, "Test", "the first registration is kept");
        assert!(mine[0].enabled);
        set_backend_enabled("test-backend", false);
        assert!(!backend_enabled("test-backend"));
        assert!(backend_reports().iter().any(|r| r.id == "test-backend" && !r.enabled));
        set_backend_enabled("test-backend", true);
        assert!(backend_enabled("test-backend"));
    }

    #[test]
    fn hostile_ids_are_ignored() {
        set_backend_enabled("", false);
        set_backend_enabled(&"x".repeat(1000), false);
        assert!(backend_enabled(""));
        assert!(backend_enabled(&"x".repeat(1000)));
    }
}
