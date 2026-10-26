//! `cfa4-cli settings …` and `cfa4-cli hardware`: the backend of the Qt Settings window
//! (`python/cfa4_devtools/qt_settings.py`). The Qt shell runs these as short local processes on a
//! worker thread (ADR F8): no running editor, no network.
//!
//! `settings` works on the preferences file of the data directory (`<data dir>/preferences.json`,
//! the file the desktop app reads), through the same `prefs.*` commands as the app, so values are
//! validated, clamped and written atomically exactly as there.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cfa4_engine::Session;
use cfa4_engine::autosave::Preferences;
use cfa4_route::catalog::{self, Entry, Maturity};
use cfa4_route::{Backend, BackendId, Device, DeviceKind, Family, Inventory, Reason, Router, Table, Task, Vendor};
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

/// The accelerator inventory (GPUs and NPUs from sysfs), the backend chain per family and the
/// vendor-neutrality coverage: what the Settings ▸ Hardware & Devices ▸ Accelerators page shows.
pub fn hardware() -> Value {
    hardware_from(cfa4_route::probe::probe(Path::new("/")))
}

/// A backend as the catalogue describes it: it runs on every device of the kinds it lists (on the
/// proprietary driver for NVDEC / NVENC / CUDA, on a VA-API driver for VA-API) and takes every
/// task. No driver is opened: the Settings page shows what the table and the devices allow, the
/// running app's own driver probes decide per stream (`perf.stats` `route.*`).
/// One id can name a backend in several families (`software` decodes and encodes): one `Listed`
/// holds its implemented entries in all of them.
struct Listed {
    id: BackendId,
    entries: Vec<&'static Entry>,
}

impl Listed {
    fn runs_on(e: &Entry, d: &Device) -> bool {
        e.runs_on.contains(&d.kind)
            && match e.vendor_lock {
                Some(Vendor::Nvidia) => d.is_nvidia_proprietary(),
                Some(v) => d.vendor == v,
                None if e.backend == "vaapi" => d.has_vaapi_driver(),
                None => true,
            }
    }
}

impl Backend for Listed {
    fn id(&self) -> &BackendId {
        &self.id
    }

    fn devices<'a>(&self, inventory: &'a Inventory) -> Vec<&'a Device> {
        inventory.devices().iter().filter(|d| self.entries.iter().any(|e| Self::runs_on(e, d))).collect()
    }

    fn accepts(&self, _task: &Task, _device: &Device) -> Result<(), String> {
        Ok(())
    }

    fn serves(&self, family: Family) -> bool {
        self.entries.iter().any(|e| e.family == family)
    }
}

fn hardware_from(inventory: Inventory) -> Value {
    let table = match Table::user_file_path() {
        Some(path) => Table::builtin_with_user_file(&path).map(|(t, _)| t),
        None => Table::builtin(),
    };
    let devices: Vec<Value> = inventory
        .devices()
        .iter()
        .filter(|d| d.kind != DeviceKind::Cpu)
        .map(|d| json!({ "kind": d.kind, "vendor": d.vendor, "pci_vendor": pci_vendor(d), "name": d.name, "driver": d.driver, "pci_device": d.pci_device, "node": d.id }))
        .collect();
    let inventory_json = json!({ "devices": devices, "notes": inventory.notes() });
    let plans = match table {
        Ok(table) => plans(table, inventory),
        Err(e) => {
            log::warn!("routing table: {e}");
            Vec::new()
        }
    };
    json!({ "inventory": inventory_json, "plans": plans, "coverage": cfa4_route::coverage() })
}

/// The PCI vendor id of a device (for the PCI ID column and the pci.ids model lookup), from the
/// vendor the probe read from it: AMD graphics carry ATI's id, AMD NPUs AMD's own.
fn pci_vendor(d: &Device) -> Option<u16> {
    d.pci_device?;
    match (d.vendor, d.kind) {
        (Vendor::Nvidia, _) => Some(0x10de),
        (Vendor::Amd, DeviceKind::Npu) => Some(0x1022),
        (Vendor::Amd, _) => Some(0x1002),
        (Vendor::Intel, _) => Some(0x8086),
        (Vendor::Other, _) => None,
    }
}

/// One plan per family: the backends tried in order (each once, best device first) and the ones
/// passed over, with why.
fn plans(table: Table, inventory: Inventory) -> Vec<Value> {
    let mut router = Router::new(table, inventory, cfa4_route::Preferences::default());
    let implemented: Vec<&'static Entry> = catalog::CATALOG.iter().filter(|e| e.maturity == Maturity::Implemented).collect();
    let mut ids: Vec<&str> = implemented.iter().map(|e| e.backend).collect();
    ids.sort_unstable();
    ids.dedup();
    for name in ids {
        if let Ok(id) = BackendId::new(name) {
            let entries = implemented.iter().copied().filter(|e| e.backend == name).collect();
            router.register(Arc::new(Listed { id, entries }));
        }
    }
    Family::ALL
        .into_iter()
        .filter_map(|family| {
            let task = Task::new(family.key(), family).ok()?;
            let chain = router.chain(&task);
            let mut backends: Vec<&str> = Vec::new();
            for r in &chain.routes {
                if !backends.contains(&r.backend.as_str()) {
                    backends.push(r.backend.as_str());
                }
            }
            let skipped: Vec<Value> = chain
                .declined
                .iter()
                .filter(|d| !backends.contains(&d.backend.as_str()))
                .map(|d| {
                    let reason = serde_json::to_value(&d.reason).unwrap_or(Value::Null);
                    let needs = match &d.reason {
                        Reason::Declined { why } => why.clone(),
                        Reason::NotEnoughMemory { need_mb, free_mb } => format!("needs {need_mb} MiB free video memory, {free_mb} MiB free"),
                        _ => String::new(),
                    };
                    json!({ "backend": d.backend, "device": d.device, "reason": reason["kind"], "needs": needs })
                })
                .collect();
            Some(json!({ "workload": family.key(), "chain": backends, "routes": chain.routes, "skipped": skipped }))
        })
        .collect()
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
    fn hardware_reports_the_cpu_fallback_without_devices() {
        let v = hardware_from(Inventory::default());
        let plans = v["plans"].as_array().unwrap();
        assert_eq!(plans.len(), Family::ALL.len());
        for p in plans {
            assert_eq!(p["routes"].as_array().unwrap().last().unwrap()["device"], "cpu", "{p}");
            assert!(p["chain"].as_array().unwrap().iter().all(|b| b.is_string()), "{p}");
        }
        assert!(v["inventory"]["devices"].as_array().unwrap().is_empty());
        assert_eq!(v["coverage"].as_array().unwrap().len(), Family::ALL.len());
    }

    #[test]
    fn hardware_routes_a_vaapi_gpu_and_skips_nvidia_only_backends() {
        let gpu = Device {
            id: cfa4_route::DeviceId::new("gpu:renderd128").unwrap(),
            kind: DeviceKind::Gpu,
            vendor: Vendor::Amd,
            name: "AMD GPU".into(),
            driver: Some("amdgpu".into()),
            pci_device: Some(0x73bf),
            free_memory_mb: None,
        };
        let v = hardware_from(Inventory::new([gpu]));
        let decode = v["plans"].as_array().unwrap().iter().find(|p| p["workload"] == "codec_decode").unwrap();
        let chain: Vec<&str> = decode["chain"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert_eq!(chain, ["vaapi", "software"]);
        assert!(decode["skipped"].as_array().unwrap().iter().any(|s| s["backend"] == "nvdec"), "{decode}");
        assert_eq!(v["inventory"]["devices"][0]["driver"], "amdgpu");
        assert_eq!(v["inventory"]["devices"][0]["pci_vendor"], 0x1002);
        assert_eq!(v["inventory"]["devices"][0]["pci_device"], 0x73bf);
    }
}
