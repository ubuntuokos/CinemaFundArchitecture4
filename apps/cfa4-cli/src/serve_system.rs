//! Settings ▸ Hardware & Devices ▸ System Settings (root) (CFA4-SETTINGS-HW-003 R2): the serve
//! methods that run `cfa4-system-helper` ([ADR 0017](../../../docs/adr/ADR-0017-rendszerszintu-komponensek.md)).
//!
//! `system.components` and `system.plan` run the helper as the user. `system.apply`,
//! `system.disable` and `system.uninstall` run it through `pkexec` (the password dialog), so they
//! are answered later, from a worker thread, and the editor keeps working meanwhile; only one of
//! them runs at a time. With `{"dryRun": true}` the helper runs as the user and only reports the
//! commands. Nothing here runs a program other than the helper and `pkexec`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

use crate::serve::Reply;

/// Where the `.deb` installs the helper; the polkit rule allows exactly this path.
pub const INSTALLED_HELPER: &str = "/usr/libexec/cfa4/cfa4-system-helper";
const PKEXEC: &str = "/usr/bin/pkexec";
const POLICY: &str = "/usr/share/polkit-1/actions/io.github.ubuntuokos.cfa4.system-helper.policy";
/// Largest helper reply read back.
const MAX_REPLY: u64 = 4 * 1024 * 1024;

/// Replies of finished background calls, written by the serve loop.
static DONE: Mutex<Vec<Reply>> = Mutex::new(Vec::new());
/// A change (apply, disable, uninstall) is running.
static BUSY: AtomicBool = AtomicBool::new(false);

/// The helper this session runs as the user: the installed one, else the one built next to
/// `cfa4-cli` (a development build).
fn user_helper() -> Option<PathBuf> {
    let installed = PathBuf::from(INSTALLED_HELPER);
    if installed.is_file() {
        return Some(installed);
    }
    let sibling = std::env::current_exe().ok()?.parent()?.join("cfa4-system-helper");
    sibling.is_file().then_some(sibling)
}

/// What this installation offers, for the page.
fn availability() -> Value {
    json!({
        "helper": user_helper().map(|p| p.display().to_string()),
        "installed": Path::new(INSTALLED_HELPER).is_file() && Path::new(POLICY).is_file(),
        "pkexec": Path::new(PKEXEC).is_file(),
    })
}

/// Runs `program args…`, feeds `stdin`, and reads the helper's one JSON object.
pub(crate) fn run_json(program: &Path, args: &[String], stdin: Option<&[u8]>) -> Result<Value, (String, String)> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| ("not_started".to_string(), format!("{} could not start: {e}", program.display())))?;
    if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let _ = pipe.write_all(bytes);
    }
    let mut out = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let _ = stdout.take(MAX_REPLY).read_to_end(&mut out);
    }
    let status = child.wait().map_err(|e| ("not_started".to_string(), e.to_string()))?;
    let text = String::from_utf8_lossy(&out);
    let Some(value) = text.lines().rev().find_map(|l| serde_json::from_str::<Value>(l.trim()).ok()) else {
        // pkexec: 126 = the user dismissed the dialog or authentication failed, 127 = not authorised
        let (code, message) = match status.code() {
            Some(126) => ("cancelled", "the password dialog was cancelled or the password was wrong"),
            Some(127) => ("not_authorised", "the system did not allow the change (polkit)"),
            _ => ("no_reply", "the system helper gave no answer"),
        };
        return Err((code.to_string(), message.to_string()));
    };
    if value.get("ok").and_then(Value::as_bool) == Some(true) {
        let mut v = value;
        if let Value::Object(map) = &mut v {
            map.remove("ok");
        }
        Ok(v)
    } else {
        let code = value.pointer("/error/code").and_then(Value::as_str).unwrap_or("failed").to_string();
        let message = value.pointer("/error/message").and_then(Value::as_str).unwrap_or("the system helper refused").to_string();
        Err((code, message))
    }
}

fn component(params: &Value) -> Result<String, String> {
    params.get("component").and_then(Value::as_str).map(str::to_string).ok_or_else(|| "missing `component`".to_string())
}

fn reply(id: Value, result: Result<Value, (String, String)>) -> Reply {
    match result {
        Ok(v) => Reply::ok(id, v),
        Err((code, message)) => Reply::err_code(id, message, &code),
    }
}

/// The change's command line: `pkexec <installed helper> …`, or the helper itself for a dry run.
fn change_command(verb: &str, component: &str, extra: &[String], dry_run: bool) -> Result<(PathBuf, Vec<String>), (String, String)> {
    let mut args: Vec<String> = vec![verb.into(), "--component".into(), component.into()];
    args.extend(extra.iter().cloned());
    if dry_run {
        let helper = user_helper().ok_or(("not_installed".to_string(), "the system helper is not installed".to_string()))?;
        args.push("--dry-run".into());
        return Ok((helper, args));
    }
    let a = availability();
    if a["installed"] != json!(true) {
        return Err(("not_installed".into(), "this installation has no system helper (it comes with the .deb package)".into()));
    }
    if a["pkexec"] != json!(true) {
        return Err(("no_pkexec".into(), "pkexec is not installed (Ubuntu package `pkexec`)".into()));
    }
    let mut full = vec![INSTALLED_HELPER.to_string()];
    full.extend(args);
    Ok((PathBuf::from(PKEXEC), full))
}

/// Starts a change on a worker thread; its reply arrives through [`upkeep`].
fn start_change(id: Value, verb: &'static str, params: &Value) -> Reply {
    let component = match component(params) {
        Ok(c) => c,
        Err(e) => return Reply::err(id, e),
    };
    let dry_run = params.get("dryRun").and_then(Value::as_bool).unwrap_or(false);
    let mut extra = Vec::new();
    let mut stdin = None;
    if verb == "apply" {
        let Some(digest) = params.get("digest").and_then(Value::as_str) else { return Reply::err(id, "missing `digest`") };
        extra.extend(["--digest".to_string(), digest.to_string()]);
        if let Some(plan) = params.get("plan").filter(|p| p.is_object()) {
            extra.extend(["--plan".to_string(), "-".to_string()]);
            stdin = Some(plan.to_string().into_bytes());
        }
    }
    let (program, args) = match change_command(verb, &component, &extra, dry_run) {
        Ok(c) => c,
        Err((code, message)) => return Reply::err_code(id, message, &code),
    };
    if BUSY.swap(true, Ordering::SeqCst) {
        return Reply::err_code(id, "another system change is still running", "busy");
    }
    let spawned = std::thread::Builder::new().name("cfa4-system-helper".into()).spawn(move || {
        let result = std::panic::catch_unwind(|| run_json(&program, &args, stdin.as_deref()))
            .unwrap_or_else(|_| Err(("internal".into(), "the system helper call failed".into())));
        DONE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(reply(id, result));
        BUSY.store(false, Ordering::SeqCst);
    });
    match spawned {
        Ok(_) => Reply::deferred(),
        Err(e) => {
            BUSY.store(false, Ordering::SeqCst);
            Reply::err(Value::Null, format!("cannot start the helper thread: {e}"))
        }
    }
}

/// Handles a `system.*` method; `None` for any other.
pub fn handle(id: Value, method: &str, params: &Value) -> Option<Reply> {
    let reply = match method {
        "system.components" => match user_helper() {
            None => Reply::ok(id, json!({ "available": availability(), "components": [] })),
            Some(helper) => match run_json(&helper, &["status".to_string()], None) {
                Ok(mut v) => {
                    v["available"] = availability();
                    Reply::ok(id, v)
                }
                Err((code, message)) => Reply::err_code(id, message, &code),
            },
        },
        "system.plan" => match (component(params), user_helper()) {
            (Err(e), _) => Reply::err(id, e),
            (_, None) => Reply::err_code(id, "the system helper is not installed", "not_installed"),
            (Ok(c), Some(helper)) => reply(id, run_json(&helper, &["plan".to_string(), "--component".to_string(), c], None)),
        },
        "system.apply" => start_change(id, "apply", params),
        "system.disable" => start_change(id, "disable", params),
        "system.uninstall" => start_change(id, "uninstall", params),
        _ => return None,
    };
    Some(reply)
}

/// The replies of changes that finished since the last call.
pub fn upkeep() -> Vec<Reply> {
    std::mem::take(&mut *DONE.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn script(dir: &Path, body: &str) -> PathBuf {
        let p = dir.join("helper.sh");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn the_helpers_json_becomes_a_result_or_a_coded_error() {
        let dir = tempfile::tempdir().unwrap();
        let ok = script(dir.path(), r#"echo '{"ok": true, "user": "anna", "components": []}'"#);
        let v = run_json(&ok, &[], None).unwrap();
        assert_eq!(v["user"], "anna");
        assert!(v.get("ok").is_none());
        let refused = script(dir.path(), r#"echo '{"ok": false, "error": {"code": "digest_mismatch", "message": "changed"}}'; exit 1"#);
        assert_eq!(run_json(&refused, &[], None).unwrap_err().0, "digest_mismatch");
        let cancelled = script(dir.path(), "exit 126");
        assert_eq!(run_json(&cancelled, &[], None).unwrap_err().0, "cancelled");
        assert_eq!(run_json(Path::new("/nonexistent/helper"), &[], None).unwrap_err().0, "not_started");
    }

    #[test]
    fn the_plan_reaches_the_helper_on_stdin() {
        let dir = tempfile::tempdir().unwrap();
        let echo = script(dir.path(), r#"read line; printf '{"ok": true, "got": %s}\n' "$line""#);
        let v = run_json(&echo, &[], Some(b"{\"component\":\"gpu-access\"}\n")).unwrap();
        assert_eq!(v["got"]["component"], "gpu-access");
    }

    #[test]
    fn changes_need_a_component_and_a_digest() {
        let r = handle(json!(1), "system.apply", &json!({})).unwrap();
        assert_eq!(r.json["ok"], false);
        let r = handle(json!(2), "system.apply", &json!({"component": "gpu-access"})).unwrap();
        assert_eq!(r.json["error"], "missing `digest`");
        assert!(handle(json!(3), "system.frobnicate", &json!({})).is_none());
    }
}
