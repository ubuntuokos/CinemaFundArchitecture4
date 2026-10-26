//! `cfa4-cli settings …` and `cfa4-cli hardware`: the backend of the Qt Settings window
//! (`python/cfa4_devtools/qt_settings.py`). The Qt shell runs these as short local processes on a
//! worker thread (ADR F8): no running editor, no network.
//!
//! `settings` works on the preferences file of the data directory (`<data dir>/preferences.json`,
//! the file the desktop app reads), through the same `prefs.*` commands as the app, so values are
//! validated, clamped and written atomically exactly as there.

use std::path::{Path, PathBuf};

use cfa4_engine::Session;
use cfa4_engine::autosave::Preferences;
use serde_json::{Value, json};

/// A headless session whose preferences are the data directory's file: `prefs.set` and
/// `prefs.reset` save back to it.
fn session(data_dir: &Path) -> Session {
    let path = data_dir.join("preferences.json");
    let mut s = Session::default();
    s.prefs = Preferences::load(&path);
    s.prefs_path = Some(path);
    s
}

/// `settings schema [category]`, `settings get [key]`, `settings set '<{"key": value, …}>'`,
/// `settings reset [category]`.
pub fn settings(action: &str, arg: Option<&str>, data_dir: Option<PathBuf>) -> Result<Value, String> {
    let dir = data_dir.ok_or("no CFA4 data directory (set --data-dir, CFA4_DATA_DIR or HOME)")?;
    let mut s = session(&dir);
    let (id, params) = match action {
        "schema" => ("prefs.schema", arg.map_or(json!({}), |c| json!({ "category": c }))),
        "get" => ("prefs.get", arg.map_or(json!({}), |k| json!({ "key": k }))),
        "set" => {
            let raw = arg.ok_or("settings set needs a JSON object of key → value")?;
            let values: Value = serde_json::from_str(raw).map_err(|e| format!("settings set: invalid JSON: {e}"))?;
            if values.as_object().is_none_or(|m| m.is_empty()) {
                return Err("settings set needs a non-empty JSON object of key → value".into());
            }
            ("prefs.set", json!({ "values": values }))
        }
        "reset" => ("prefs.reset", arg.map_or(json!({}), |c| json!({ "category": c }))),
        other => return Err(format!("unknown settings action `{other}` (schema, get, set, reset)")),
    };
    s.execute(id, params).map_err(|e| e.to_string())
}

/// `cfa4-cli media-backends`: what Settings ▸ Media & Playback ▸ Media Backends shows under its
/// switches: the own engine and the supplementary backends (FFmpeg, GStreamer; ADR 0005) in the
/// order they are tried, each with whether it is compiled in, available and switched on, its
/// version, licence and problem, and the patent-restricted codecs (Ubuntu model). The switches
/// are read from the data directory's preferences, as the app does at startup.
pub fn media_backends(data_dir: Option<PathBuf>) -> Result<Value, String> {
    let mut s = match data_dir {
        Some(dir) => session(&dir),
        None => Session::default(),
    };
    s.prefs.apply_media_backends();
    s.execute("media.backends", json!({})).map_err(|e| e.to_string())
}

/// The accelerator inventory (GPUs and NPUs from sysfs), the backend chain per workload and the
/// vendor-neutrality coverage: what the Settings ▸ Hardware & Devices ▸ Accelerators page shows.
pub fn hardware() -> Value {
    hardware_from(cfa4_accel::probe())
}

fn hardware_from(inventory: cfa4_accel::Inventory) -> Value {
    let plans = cfa4_accel::plan_all(&inventory, &cfa4_accel::Settings::default());
    json!({ "inventory": inventory, "plans": plans, "coverage": cfa4_accel::coverage() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cfa4-cli-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn set_persists_and_get_reads_back() {
        let d = tmp("persist");
        let r = settings("set", Some(r#"{"timeline.autoScroll": "smoothScroll"}"#), Some(d.clone())).unwrap();
        assert_eq!(r["timeline"]["autoScroll"], "smoothScroll");
        assert!(d.join("preferences.json").is_file());
        assert_eq!(settings("get", Some("timeline.autoScroll"), Some(d.clone())).unwrap(), "smoothScroll");
        settings("reset", Some("timeline"), Some(d.clone())).unwrap();
        assert_eq!(settings("get", Some("timeline.autoScroll"), Some(d)).unwrap(), "pageScroll");
    }

    #[test]
    fn hostile_input_is_an_error_not_a_crash() {
        let d = tmp("hostile");
        for bad in ["", "[]", "{}", "{", r#"{"nope.key": 1}"#, r#"{"timeline.autoScroll": "sideways"}"#, "null"] {
            assert!(settings("set", Some(bad), Some(d.clone())).is_err(), "{bad}");
        }
        assert!(settings("set", None, Some(d.clone())).is_err());
        assert!(settings("frobnicate", None, Some(d.clone())).is_err());
        assert!(settings("schema", Some("nope"), Some(d.clone())).is_err());
        assert!(settings("get", None, None).is_err());
        // a corrupt preferences file loads as the defaults
        std::fs::write(d.join("preferences.json"), b"\xff not json").unwrap();
        assert_eq!(settings("get", Some("timeline.autoScroll"), Some(d)).unwrap(), "pageScroll");
    }

    #[test]
    fn schema_lists_groups() {
        let d = tmp("schema");
        let r = settings("schema", None, Some(d)).unwrap();
        let groups: Vec<&str> = r["groups"].as_array().unwrap().iter().filter_map(|g| g["id"].as_str()).collect();
        assert!(groups.contains(&"hardware"));
        let memory = r["categories"].as_array().unwrap().iter().find(|c| c["id"] == "memory").unwrap();
        assert_eq!(memory["group"], "hardware");
    }

    #[test]
    fn media_backends_lists_the_engine_first_and_reads_the_switches() {
        let d = tmp("backends");
        settings("set", Some(r#"{"media.ffmpegBackend": false}"#), Some(d.clone())).unwrap();
        let v = media_backends(Some(d)).unwrap();
        let list = v["backends"].as_array().unwrap();
        assert_eq!(list[0]["id"], "builtin");
        assert_eq!(list[0]["canDisable"], false);
        // the switch is read from the data directory, as the app does at startup
        assert!(!cfa4_media::backend_enabled("ffmpeg"));
        cfa4_media::set_backend_enabled("ffmpeg", true);
        assert!(media_backends(None).is_ok());
    }

    #[test]
    fn hardware_reports_the_cpu_fallback_without_devices() {
        let v = hardware_from(cfa4_accel::Inventory::default());
        let plans = v["plans"].as_array().unwrap();
        assert!(!plans.is_empty());
        for p in plans {
            assert_eq!(p["chain"].as_array().unwrap().last().unwrap(), "cpu", "{p}");
        }
        assert!(v["inventory"]["devices"].as_array().unwrap().is_empty());
    }
}
