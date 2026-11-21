//! `security.status` (CFA4-SECURITY-001 B1): the run-time security layer's report, for Settings ▸
//! Security, agents and bug reports. Read-only; available without a project.

use std::path::PathBuf;

use serde_json::Value;

use crate::Session;

/// The paths of this session that the process profiles are built from. Phase B3 adds the media the
/// user approved (D6) and the export targets once the engine confines itself.
pub fn context(s: &Session) -> cfa4_sandbox::Context {
    let parent = |p: &std::path::Path| p.parent().filter(|d| !d.as_os_str().is_empty()).map(PathBuf::from);
    cfa4_sandbox::Context {
        project_dirs: s.path.as_deref().map(std::path::Path::new).and_then(parent).into_iter().collect(),
        config_dir: s.prefs_path.as_deref().and_then(parent),
        ..cfa4_sandbox::Context::default()
    }
}

/// The engine's `security.status`.
pub fn status(s: &Session) -> Value {
    cfa4_sandbox::status(&context(s))
}

/// Download `files` into `dir` through the confined `cfa4-fetch` process (plan D5): the engine
/// opens no connection to the internet itself. `progress` gets `(bytes done, bytes total, file)`
/// and returns false to cancel. Each file is `(name, https url, SHA-256, size)`.
pub fn download(
    dir: &std::path::Path,
    files: impl IntoIterator<Item = (String, String, String, u64)>,
    progress: &mut dyn FnMut(u64, u64, &str) -> bool,
) -> std::result::Result<(), String> {
    use cfa4_fetch_client::{FileSpec, Request};
    let files = files.into_iter().map(|(name, url, sha256, size)| FileSpec { name, url, sha256, size }).collect();
    // a relative data directory (CFA4_DATA_DIR=./data) is taken from the working directory; the
    // downloader wants an absolute target and still refuses one with `..`
    let dir = std::path::absolute(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let req = Request { dir, files };
    let program = cfa4_fetch_client::locate().ok_or_else(|| cfa4_fetch_client::FetchError::NotFound.to_string())?;
    cfa4_fetch_client::run(&program, &[], &req, progress).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    #[test]
    fn security_status_reports_support_and_profiles() {
        let mut s = Session::default();
        let v = s.execute("security.status", json!({})).unwrap();
        assert!(v["support"]["landlockAbi"].is_u64(), "{v}");
        assert!(v["support"]["seccomp"].is_boolean());
        let kinds: Vec<&str> = v["processes"].as_array().unwrap().iter().filter_map(|p| p["kind"].as_str()).collect();
        assert_eq!(kinds, ["qtGui", "engine", "mediaHost", "studioHost", "browserSource", "fetch", "pluginHost", "modelConverter"]);
        // the engine process itself is not confined yet (phase B3)
        assert!(v["self"]["kind"].is_null());
        // hostile parameters are ignored, never a crash
        assert!(s.execute("security.status", json!({"kind": [1, 2], "x": null})).is_ok());
        assert!(s.history.undo.is_empty(), "a query changes nothing");
    }

    #[test]
    fn security_status_names_the_project_directory() {
        let mut s = Session { path: Some("/work/film/film.fcproj".into()), ..Session::default() };
        let v = s.execute("security.status", json!({})).unwrap();
        let engine = v["processes"].as_array().unwrap().iter().find(|p| p["kind"] == "engine").unwrap();
        assert_eq!(engine["write"][0], "/work/film", "{engine}");
        assert_eq!(engine["network"]["kind"], "loopback");
    }
}
