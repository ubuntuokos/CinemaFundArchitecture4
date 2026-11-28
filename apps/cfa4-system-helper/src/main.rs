//! `cfa4-system-helper`: the only program CFA4 runs as root, and only through `pkexec` after the user
//! approved the plan in CFA4's window ([ADR 0017](../../../docs/adr/ADR-0017-rendszerszintu-komponensek.md),
//! CFA4-SETTINGS-HW-003 R1).
//!
//! ```text
//! cfa4-system-helper status                                   all components and the state record
//! cfa4-system-helper check   [--component ID]                 root not needed
//! cfa4-system-helper plan    --component ID                   root not needed; prints the plan and its digest
//! cfa4-system-helper apply   --component ID --digest HEX [--plan FILE|-]
//! cfa4-system-helper disable --component ID                   undoes what the helper did
//! cfa4-system-helper uninstall --component ID                 disable, then forget the component
//! common: [--root DIR] [--uid N] [--dry-run]
//! ```
//!
//! The verbs and components are closed lists. The user a plan is for comes from `PKEXEC_UID` when run
//! as root, never from the caller. `apply` rebuilds the plan as root and runs it only when its SHA-256
//! digest equals the approved one, so a plan that changed between approval and execution does not
//! run. `--root` points every read and write at a fake tree (tests); with it only `--dry-run` may
//! change anything, and `--uid` picks the user. Output is one JSON object on standard output; the exit
//! status is 0 on success, 1 on a refused or failed step, 2 on a usage error. It never panics on bad
//! input ([`AGENTS.md`] §0).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

mod components;
mod exec;
mod plan;
mod system;

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Value, json};

use components::COMPONENTS;
use exec::{DryRunner, Runner, State, SystemRunner};
use plan::Plan;
use system::{System, User};

/// Largest plan accepted on `apply`.
const MAX_PLAN: u64 = 1024 * 1024;

#[derive(Debug, Default)]
struct Args {
    verb: String,
    component: Option<String>,
    digest: Option<String>,
    plan: Option<String>,
    root: Option<PathBuf>,
    uid: Option<u32>,
    dry_run: bool,
}

struct Failure {
    code: &'static str,
    message: String,
    exit: u8,
}

fn fail(code: &'static str, message: impl Into<String>) -> Failure {
    Failure { code, message: message.into(), exit: 1 }
}

fn usage(message: impl Into<String>) -> Failure {
    Failure { code: "usage", message: message.into(), exit: 2 }
}

const VERBS: &[&str] = &["status", "check", "plan", "apply", "disable", "uninstall"];

fn parse(argv: &[String]) -> Result<Args, Failure> {
    let mut args = Args::default();
    let mut it = argv.iter();
    args.verb = it.next().cloned().ok_or_else(|| usage("missing verb"))?;
    if !VERBS.contains(&args.verb.as_str()) {
        return Err(usage(format!("unknown verb: {}", args.verb)));
    }
    while let Some(flag) = it.next() {
        let mut value = || it.next().cloned().ok_or_else(|| usage(format!("{flag} needs a value")));
        match flag.as_str() {
            "--component" => args.component = Some(value()?),
            "--digest" => args.digest = Some(value()?),
            "--plan" => args.plan = Some(value()?),
            "--root" => args.root = Some(PathBuf::from(value()?)),
            "--uid" => args.uid = Some(value()?.parse().map_err(|_| usage("--uid needs a number"))?),
            "--dry-run" => args.dry_run = true,
            other => return Err(usage(format!("unknown option: {other}"))),
        }
    }
    if let Some(c) = &args.component
        && !COMPONENTS.contains(&c.as_str())
    {
        return Err(fail("unknown_component", format!("unknown component: {c}")));
    }
    Ok(args)
}

/// The user the helper acts for: `--uid` on a fake root, `PKEXEC_UID` as root, else the caller.
fn resolve_user(args: &Args, sys: &System, euid: u32, real_uid: u32) -> Result<User, Failure> {
    let uid = match args.uid {
        Some(_) if sys.is_real_root() => return Err(usage("--uid is only for a fake --root")),
        Some(uid) => uid,
        None if euid == 0 => std::env::var("PKEXEC_UID")
            .ok()
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| fail("not_through_pkexec", "run as root only through pkexec (PKEXEC_UID is missing)"))?,
        None => real_uid,
    };
    if uid == 0 {
        return Err(fail("root_user", "the helper acts for a regular user, not root"));
    }
    sys.user_by_uid(uid).ok_or_else(|| fail("unknown_user", format!("no user with uid {uid}")))
}

fn component_of(args: &Args) -> Result<&str, Failure> {
    args.component.as_deref().ok_or_else(|| usage(format!("{} needs --component", args.verb)))
}

fn read_plan(source: &str) -> Result<Plan, Failure> {
    let mut bytes = Vec::new();
    let bad = || fail("bad_plan", "the plan is not a valid plan");
    if source == "-" {
        std::io::stdin().take(MAX_PLAN + 1).read_to_end(&mut bytes).map_err(|_| bad())?;
    } else {
        let meta = std::fs::metadata(source).map_err(|_| bad())?;
        if !meta.is_file() || meta.len() > MAX_PLAN {
            return Err(bad());
        }
        bytes = std::fs::read(source).map_err(|_| bad())?;
    }
    if bytes.len() as u64 > MAX_PLAN {
        return Err(bad());
    }
    // The error never echoes the content: as root, the file may be anything.
    serde_json::from_slice(&bytes).map_err(|_| bad())
}

fn check_json(c: &components::Check) -> Value {
    json!({ "component": c.component, "status": c.status, "details": c.details, "can_apply": c.can_apply, "relogin_required": c.relogin_required })
}

fn run(args: &Args) -> Result<Value, Failure> {
    let sys = System::new(args.root.clone().unwrap_or_else(|| PathBuf::from("/")));
    let (real_uid, euid) = system::process_uids().ok_or_else(|| fail("no_proc", "cannot read /proc/self/status"))?;
    let mutating = matches!(args.verb.as_str(), "apply" | "disable" | "uninstall");
    if mutating && !args.dry_run {
        if !sys.is_real_root() {
            return Err(fail("fake_root", "a fake --root only allows --dry-run"));
        }
        if euid != 0 {
            return Err(fail("needs_root", format!("{} needs root: run it through pkexec", args.verb)));
        }
    }
    let user = resolve_user(args, &sys, euid, real_uid)?;
    let mut dry = DryRunner::default();
    let mut real = SystemRunner;
    let runner: &mut dyn Runner = if args.dry_run || !sys.is_real_root() { &mut dry } else { &mut real };

    let result = match args.verb.as_str() {
        "status" => {
            let state = State::load(&sys);
            let checks: Vec<Value> = COMPONENTS.iter().filter_map(|c| components::check(c, &sys, &user)).map(|c| check_json(&c)).collect();
            let (state, note) = match state {
                Ok(s) => (serde_json::to_value(s).unwrap_or(Value::Null), Value::Null),
                Err(e) => (Value::Null, Value::String(e)),
            };
            json!({ "user": user.name, "components": checks, "state": state, "state_error": note })
        }
        "check" => {
            let list: Vec<&str> = match &args.component {
                Some(c) => vec![c.as_str()],
                None => COMPONENTS.to_vec(),
            };
            let checks: Vec<Value> = list.iter().filter_map(|c| components::check(c, &sys, &user)).map(|c| check_json(&c)).collect();
            json!({ "user": user.name, "components": checks })
        }
        "plan" => {
            let plan = components::plan(component_of(args)?, &sys, &user).map_err(|e| fail("nothing_to_apply", e))?;
            json!({ "digest": plan.digest(), "plan": plan })
        }
        "apply" => {
            let component = component_of(args)?;
            let digest = args.digest.as_deref().ok_or_else(|| usage("apply needs --digest"))?;
            if let Some(source) = &args.plan {
                let shown = read_plan(source)?;
                if shown.component != component || shown.digest() != digest {
                    return Err(fail("digest_mismatch", "the plan file does not match the approved digest"));
                }
            }
            // Rebuilt here, as the user who runs it: what runs is what the helper itself plans now.
            let plan = components::plan(component, &sys, &user).map_err(|e| fail("nothing_to_apply", e))?;
            if plan.digest() != digest {
                return Err(fail("digest_mismatch", "the system changed since the plan was approved; check and approve again"));
            }
            let mut state = State::load(&sys).map_err(|e| fail("state", e))?;
            let done = exec::apply(&plan, &mut state, runner).map_err(|e| fail("apply_failed", e))?;
            if !args.dry_run {
                state.save(&sys).map_err(|e| fail("state", e))?;
            }
            json!({ "component": component, "applied": done, "relogin_required": plan.relogin_required })
        }
        _ => {
            let component = component_of(args)?;
            let mut state = State::load(&sys).map_err(|e| fail("state", e))?;
            let forget = args.verb == "uninstall";
            let outcome = exec::undo(component, &mut state, runner, forget);
            if !args.dry_run {
                // Saved even after a failed step, so what was undone is not undone twice.
                state.save(&sys).map_err(|e| fail("state", e))?;
            }
            let undone = outcome.map_err(|e| fail("undo_failed", e))?;
            json!({ "component": component, "undone": undone })
        }
    };
    let mut result = result;
    if args.dry_run || !sys.is_real_root() {
        let commands: Vec<Value> = dry.commands.iter().map(|c| json!({ "program": c.program, "args": c.args })).collect();
        if let Value::Object(map) = &mut result {
            map.insert("dry_run".into(), Value::Bool(true));
            map.insert("commands".into(), Value::Array(commands));
        }
    }
    Ok(result)
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let (body, exit) = match parse(&argv).and_then(|a| run(&a)) {
        Ok(Value::Object(mut map)) => {
            map.insert("ok".into(), Value::Bool(true));
            (Value::Object(map), 0)
        }
        Ok(other) => (json!({ "ok": true, "result": other }), 0),
        Err(f) => (json!({ "ok": false, "error": { "code": f.code, "message": f.message } }), f.exit),
    };
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{body}");
    let _ = out.flush();
    ExitCode::from(exit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn verbs_and_components_are_closed_lists() {
        assert_eq!(parse(&argv("rm -rf")).err().map(|f| f.exit), Some(2));
        assert_eq!(parse(&argv("apply --component gpu-unlock")).err().map(|f| f.code), Some("unknown_component"));
        assert_eq!(parse(&argv("check --bogus")).err().map(|f| f.exit), Some(2));
        assert_eq!(parse(&[]).err().map(|f| f.exit), Some(2));
        let a = parse(&argv("plan --component gpu-access --root /tmp/x --uid 1000 --dry-run")).ok().unwrap();
        assert_eq!(a.uid, Some(1000));
        assert!(a.dry_run);
    }

    #[test]
    fn uid_override_is_refused_on_the_real_root_and_root_needs_pkexec() {
        let real = System::new("/");
        let a = Args { uid: Some(1000), ..Args::default() };
        assert_eq!(resolve_user(&a, &real, 1000, 1000).err().map(|f| f.code), Some("usage"));
        // As root without PKEXEC_UID (tests do not run under pkexec).
        if std::env::var_os("PKEXEC_UID").is_none() {
            assert_eq!(resolve_user(&Args::default(), &real, 0, 0).err().map(|f| f.code), Some("not_through_pkexec"));
        }
        let dir = tempfile::tempdir().unwrap();
        let fake = System::new(dir.path());
        assert_eq!(resolve_user(&Args { uid: Some(0), ..Args::default() }, &fake, 1, 1).err().map(|f| f.code), Some("root_user"));
    }
}
