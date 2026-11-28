//! Carrying out a plan and undoing it: the command runner (replaceable in tests), the state file of
//! what the helper did, and the roll-back of a half-finished apply (ADR 0017 §5).

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::plan::{Action, Cmd, Plan};
use crate::system::System;

/// Where the state file lives beneath the root.
pub const STATE_FILE: &str = "var/lib/cfa4/system-helper/state.json";
/// Largest state file the helper reads back.
const MAX_STATE: u64 = 4 * 1024 * 1024;
/// Most records kept per component.
const MAX_RECORDS: usize = 64;

/// Runs the commands of a plan.
pub trait Runner {
    /// Runs one command; `Err` carries a short reason.
    fn run(&mut self, cmd: &Cmd) -> Result<(), String>;
    /// Writes one line to the system log.
    fn log(&mut self, message: &str);
}

/// Runs real programs, with a fixed `PATH`, `LC_ALL=C` and no inherited standard input.
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&mut self, cmd: &Cmd) -> Result<(), String> {
        let out = Command::new(cmd.program)
            .args(&cmd.args)
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .env("LC_ALL", "C")
            .envs(cmd.env.iter().copied())
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("{} could not start: {e}", cmd.program))?;
        if out.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        let last = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").chars().take(300).collect::<String>();
        Err(format!("{} failed ({}): {last}", cmd.program, out.status))
    }

    fn log(&mut self, message: &str) {
        let _ = Command::new("/usr/bin/logger")
            .args(["--tag", "cfa4-system-helper", "--", message])
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Records the commands instead of running them (`--dry-run`, and tests).
#[derive(Default)]
pub struct DryRunner {
    pub commands: Vec<Cmd>,
    pub log: Vec<String>,
    /// Fail the command with this index (tests of the roll-back).
    pub fail_at: Option<usize>,
}

impl Runner for DryRunner {
    fn run(&mut self, cmd: &Cmd) -> Result<(), String> {
        let index = self.commands.len();
        self.commands.push(cmd.clone());
        if self.fail_at == Some(index) {
            return Err(format!("{} failed (test)", cmd.program));
        }
        Ok(())
    }

    fn log(&mut self, message: &str) {
        self.log.push(message.to_string());
    }
}

/// One change the helper made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub action: Action,
    /// RFC 3339 time in UTC.
    pub at: String,
}

/// What the helper did for one component.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentState {
    /// `applied` or `disabled`.
    pub status: String,
    /// The changes still in effect, oldest first: exactly what `disable` undoes.
    pub applied: Vec<Record>,
}

/// The state file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub format: u32,
    pub components: BTreeMap<String, ComponentState>,
}

impl Default for State {
    fn default() -> Self {
        State { format: 1, components: BTreeMap::new() }
    }
}

impl State {
    /// Reads the state file. A missing file is an empty state; a file that cannot be read back is
    /// an error, so a change never overwrites the record of what to undo.
    pub fn load(sys: &System) -> Result<State, String> {
        let path = sys.path(STATE_FILE);
        let meta = match fs::metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(State::default()),
            Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
        };
        if !meta.is_file() || meta.len() > MAX_STATE {
            return Err(format!("{} is not a state file", path.display()));
        }
        let bytes = fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        serde_json::from_slice(&bytes).map_err(|_| format!("{} is damaged; nothing was changed", path.display()))
    }

    /// Writes the state file atomically (a temporary file, then a rename).
    pub fn save(&self, sys: &System) -> Result<(), String> {
        let path = sys.path(STATE_FILE);
        let dir = path.parent().ok_or("state path has no directory")?;
        fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let tmp = dir.join("state.json.tmp");
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        let write = |p: &Path| -> std::io::Result<()> {
            let mut f = fs::File::create(p)?;
            f.write_all(&bytes)?;
            f.sync_all()
        };
        write(&tmp).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        fs::rename(&tmp, &path).map_err(|e| format!("cannot replace {}: {e}", path.display()))
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Carries out the plan. When a step fails, the steps already done are undone in reverse order,
/// so the system is left as it was; the error names the failed step.
pub fn apply(plan: &Plan, state: &mut State, runner: &mut dyn Runner) -> Result<Vec<Record>, String> {
    for action in &plan.actions {
        action.validate()?;
    }
    let mut done: Vec<Record> = Vec::new();
    for action in &plan.actions {
        runner.log(&format!("apply {}: {:?}", plan.component, action));
        if let Err(e) = runner.run(&action.command()) {
            runner.log(&format!("apply {} failed: {e}; rolling back", plan.component));
            for record in done.iter().rev() {
                if let Err(undo) = runner.run(&record.action.undo().command()) {
                    runner.log(&format!("roll-back of {:?} failed: {undo}", record.action));
                }
            }
            return Err(e);
        }
        done.push(Record { action: action.clone(), at: now() });
    }
    let entry = state.components.entry(plan.component.clone()).or_default();
    entry.status = "applied".into();
    entry.applied.extend(done.iter().cloned());
    let excess = entry.applied.len().saturating_sub(MAX_RECORDS);
    entry.applied.drain(..excess);
    Ok(done)
}

/// Undoes what the helper did for the component (and only that), newest first. A step that fails
/// stays in the state, so a later `disable` can retry it. `forget` (uninstall) also drops the
/// component's entry once nothing is left to undo.
pub fn undo(component: &str, state: &mut State, runner: &mut dyn Runner, forget: bool) -> Result<Vec<Action>, String> {
    let Some(entry) = state.components.get_mut(component) else { return Ok(Vec::new()) };
    let mut undone = Vec::new();
    let mut failed = None;
    while let Some(record) = entry.applied.last() {
        let reverse = record.action.undo();
        reverse.validate()?;
        runner.log(&format!("undo {component}: {reverse:?}"));
        match runner.run(&reverse.command()) {
            Ok(()) => {
                undone.push(reverse);
                entry.applied.pop();
            }
            Err(e) => {
                failed = Some(e);
                break;
            }
        }
    }
    if let Some(e) = failed {
        return Err(e);
    }
    entry.status = "disabled".into();
    if forget {
        state.components.remove(component);
    }
    Ok(undone)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::PLAN_FORMAT;

    fn plan() -> Plan {
        Plan {
            format: PLAN_FORMAT,
            component: "gpu-access".into(),
            user: "anna".into(),
            actions: vec![
                Action::AddToGroup { user: "anna".into(), group: "render".into() },
                Action::AddToGroup { user: "anna".into(), group: "video".into() },
            ],
            relogin_required: true,
        }
    }

    #[test]
    fn apply_records_and_disable_undoes_newest_first() {
        let mut state = State::default();
        let mut r = DryRunner::default();
        apply(&plan(), &mut state, &mut r).unwrap();
        assert_eq!(r.commands.len(), 2);
        assert_eq!(state.components["gpu-access"].applied.len(), 2);
        let mut r = DryRunner::default();
        let undone = undo("gpu-access", &mut state, &mut r, false).unwrap();
        assert_eq!(r.commands[0].args, ["-d", "anna", "video"]);
        assert_eq!(r.commands[1].args, ["-d", "anna", "render"]);
        assert_eq!(undone.len(), 2);
        assert_eq!(state.components["gpu-access"].status, "disabled");
        assert!(state.components["gpu-access"].applied.is_empty());
        // Nothing left: a second disable runs nothing.
        let mut r = DryRunner::default();
        undo("gpu-access", &mut state, &mut r, true).unwrap();
        assert!(r.commands.is_empty());
        assert!(!state.components.contains_key("gpu-access"));
    }

    #[test]
    fn a_failed_step_rolls_back_the_earlier_ones_and_records_nothing() {
        let mut state = State::default();
        let mut r = DryRunner { fail_at: Some(1), ..DryRunner::default() };
        assert!(apply(&plan(), &mut state, &mut r).is_err());
        let args: Vec<_> = r.commands.iter().map(|c| c.args.join(" ")).collect();
        assert_eq!(args, ["-a anna render", "-a anna video", "-d anna render"]);
        assert!(state.components.is_empty());
    }

    #[test]
    fn a_failed_undo_stays_in_the_state() {
        let mut state = State::default();
        apply(&plan(), &mut state, &mut DryRunner::default()).unwrap();
        let mut r = DryRunner { fail_at: Some(1), ..DryRunner::default() };
        assert!(undo("gpu-access", &mut state, &mut r, true).is_err());
        let left = &state.components["gpu-access"].applied;
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].action, Action::AddToGroup { user: "anna".into(), group: "render".into() });
    }

    #[test]
    fn a_forged_plan_is_refused_before_anything_runs() {
        let mut p = plan();
        p.actions.push(Action::InstallPackages { packages: vec!["openssh-server".into()] });
        let mut r = DryRunner::default();
        assert!(apply(&p, &mut State::default(), &mut r).is_err());
        assert!(r.commands.is_empty());
    }

    #[test]
    fn the_state_file_round_trips_beneath_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let sys = System::new(dir.path());
        assert_eq!(State::load(&sys).unwrap(), State::default());
        let mut state = State::default();
        apply(&plan(), &mut state, &mut DryRunner::default()).unwrap();
        state.save(&sys).unwrap();
        assert_eq!(State::load(&sys).unwrap(), state);
        fs::write(sys.path(STATE_FILE), "not json").unwrap();
        assert!(State::load(&sys).is_err());
    }
}
