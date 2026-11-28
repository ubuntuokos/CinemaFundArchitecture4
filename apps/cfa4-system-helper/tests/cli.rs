//! The helper as a program, on a fake root: JSON out, the plan and digest round trip, a changed
//! system refuses the old digest, and nothing runs for real.

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::process::Command;

use serde_json::Value;

fn helper(root: &Path, args: &[&str]) -> (Value, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_cfa4-system-helper"))
        .args(args)
        .args(["--root", root.to_str().unwrap(), "--uid", "4242"])
        .env_remove("PKEXEC_UID")
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    (serde_json::from_str(text.trim()).unwrap_or_else(|_| panic!("not JSON: {text}")), out.status.code().unwrap())
}

/// A fake root with one render node only its group can open, and user `anna` (uid 4242).
fn machine() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("dev/dri")).unwrap();
    fs::create_dir_all(root.join("etc")).unwrap();
    let node = root.join("dev/dri/renderD128");
    fs::write(&node, "").unwrap();
    fs::set_permissions(&node, fs::Permissions::from_mode(0o660)).unwrap();
    let gid = fs::metadata(&node).unwrap().gid();
    fs::write(root.join("etc/passwd"), "root:x:0:0::/root:/bin/sh\nanna:x:4242:4242::/home/anna:/bin/bash\n").unwrap();
    fs::write(root.join("etc/group"), format!("anna:x:4242:\nrender:x:{gid}:\n")).unwrap();
    dir
}

#[test]
fn check_plan_and_dry_run_apply_round_trip() {
    let m = machine();
    let (status, code) = helper(m.path(), &["status"]);
    assert_eq!(code, 0, "{status}");
    assert_eq!(status["user"], "anna");
    assert_eq!(status["components"].as_array().unwrap().len(), 3);
    assert_eq!(status["components"][0]["status"], "needs_action");

    let (plan, code) = helper(m.path(), &["plan", "--component", "gpu-access"]);
    assert_eq!(code, 0, "{plan}");
    let digest = plan["digest"].as_str().unwrap().to_string();
    assert_eq!(plan["plan"]["actions"][0]["group"], "render");
    let plan_file = m.path().join("plan.json");
    fs::write(&plan_file, plan["plan"].to_string()).unwrap();

    let (applied, code) = helper(m.path(), &["apply", "--component", "gpu-access", "--digest", &digest, "--plan", plan_file.to_str().unwrap(), "--dry-run"]);
    assert_eq!(code, 0, "{applied}");
    assert_eq!(applied["dry_run"], true);
    assert_eq!(applied["commands"][0]["program"], "/usr/bin/gpasswd");
    assert_eq!(applied["commands"][0]["args"], serde_json::json!(["-a", "anna", "render"]));
    assert_eq!(applied["relogin_required"], true);
    // A dry run writes no state.
    assert!(!m.path().join("var/lib/cfa4/system-helper/state.json").exists());
}

#[test]
fn a_changed_system_or_a_wrong_digest_is_refused() {
    let m = machine();
    let (plan, _) = helper(m.path(), &["plan", "--component", "gpu-access"]);
    let digest = plan["digest"].as_str().unwrap().to_string();
    let (bad, code) = helper(m.path(), &["apply", "--component", "gpu-access", "--digest", &"0".repeat(64), "--dry-run"]);
    assert_eq!((code, bad["error"]["code"].as_str()), (1, Some("digest_mismatch")));
    // anna joins the group by other means: the approved plan no longer matches.
    let group = fs::read_to_string(m.path().join("etc/group")).unwrap().replace("render:x:", "video:x:");
    fs::write(m.path().join("etc/group"), group).unwrap();
    let (changed, code) = helper(m.path(), &["apply", "--component", "gpu-access", "--digest", &digest, "--dry-run"]);
    assert_eq!(code, 1, "{changed}");
}

#[test]
fn a_fake_root_never_changes_anything_for_real() {
    let m = machine();
    let (out, code) = helper(m.path(), &["disable", "--component", "gpu-access"]);
    assert_eq!((code, out["error"]["code"].as_str()), (1, Some("fake_root")));
    let (out, code) = helper(m.path(), &["apply", "--component", "npu-check", "--digest", "x", "--dry-run"]);
    assert_eq!((code, out["error"]["code"].as_str()), (1, Some("nothing_to_apply")));
    let (out, code) = helper(m.path(), &["frobnicate"]);
    assert_eq!((code, out["ok"].as_bool()), (2, Some(false)));
}

#[test]
fn a_bad_plan_file_is_refused_without_echoing_it() {
    let m = machine();
    let (plan, _) = helper(m.path(), &["plan", "--component", "gpu-access"]);
    let digest = plan["digest"].as_str().unwrap().to_string();
    let secret = m.path().join("secret");
    fs::write(&secret, "TOP-SECRET-LINE").unwrap();
    let (out, code) = helper(m.path(), &["apply", "--component", "gpu-access", "--digest", &digest, "--plan", secret.to_str().unwrap(), "--dry-run"]);
    assert_eq!((code, out["error"]["code"].as_str()), (1, Some("bad_plan")));
    assert!(!out.to_string().contains("TOP-SECRET"));
}
