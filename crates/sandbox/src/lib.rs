//! CFA4's run-time security layer (CFA4-SECURITY-001, phase B1).
//!
//! When a hostile media file, project, web page, network stream, plug-in or AI model breaks one part
//! of CFA4, the damage must stay inside that one process: it must not read the user's other files
//! (`~/.ssh`, browser profiles, documents), start programs or open network connections. This crate
//! gives every CFA4 [`ProcessKind`] a profile it can lock itself down with at start-up (applied by
//! the engine from phase B3, by the media host from phase B4):
//!
//! - **Landlock** (file-system access, TCP ports on ABI ≥ 4, signal and abstract-socket scoping on
//!   ABI ≥ 6), through the safe `landlock` crate;
//! - **seccomp-bpf** (a deny list of system calls: program start, debugging other processes, kernel
//!   module and namespace calls, IPv4/IPv6 sockets where the process has no network, UDP and raw
//!   sockets where it has loopback TCP only), through the safe `seccompiler` crate.
//!
//! Both are unprivileged and irreversible: once applied, a process (and every child it starts)
//! keeps them. Apply a profile early, before the process opens untrusted input and before it starts
//! threads (Landlock and the seccomp filter bind the calling thread and its future children).
//!
//! **Strict by default (plan D4):** [`Mode::Strict`] refuses to run a process without the
//! protections its profile needs and returns [`SandboxError::Missing`], so the caller does not start
//! the risky part and tells the user what is missing; the rest of CFA4 keeps working (never crash).
//! [`Mode::Unprotected`] (the user's explicit opt-in after a warning) applies what the kernel offers
//! and marks the process unsupported in [`status`].
//!
//! The profiles are data ([`Profile::for_process`]) so `security.status` can show them and tests can
//! check them; [`apply`] turns one into kernel rules. No `unsafe` here (AGENTS.md §0.3).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

#[cfg(target_os = "linux")]
mod linux;
mod probe;
mod profile;

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use probe::{Support, probe};
pub use profile::{Context, Network, ProcessKind, Profile};

/// How strictly [`apply`] treats a missing protection (plan D4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    /// Refuse to run without every protection the profile needs (the default and the only
    /// supported mode).
    #[default]
    Strict,
    /// Apply what the kernel offers and carry on. The user enabled this in Settings after an explicit
    /// warning; CFA4 running like this is unsupported, and [`status`] says so.
    Unprotected,
}

/// What [`apply`] actually enforced.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Enforcement {
    /// The Landlock ABI the rules were built for (0: Landlock not used).
    pub landlock_abi: u8,
    /// Landlock enforced every rule of the profile (`false`: partially or not at all).
    pub landlock_full: bool,
    /// TCP ports restricted by Landlock (ABI ≥ 4).
    pub network_ports: bool,
    /// The seccomp filter is installed.
    pub seccomp: bool,
    /// `no_new_privs` is set (set-uid programs cannot gain rights).
    pub no_new_privs: bool,
}

/// Why a profile could not be applied.
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    /// Strict mode: the running system lacks protections the profile needs. The process must not run
    /// its risky part; the list names what is missing, for the user.
    #[error("missing protection: {}", .0.join(", "))]
    Missing(Vec<String>),
    /// This process kind is confined by another layer (the Chromium sandbox and its AppArmor
    /// profile for the browser source, plan B6), not by this crate.
    #[error("{0} is confined by its own sandbox, not by cfa4-sandbox")]
    External(&'static str),
    /// The platform has no Landlock or seccomp at all (not Linux).
    #[error("process confinement is only available on Linux")]
    Unsupported,
    /// A process applies one profile, once.
    #[error("this process is already confined ({0})")]
    AlreadyApplied(&'static str),
    /// The kernel refused the rules.
    #[error("Landlock: {0}")]
    Landlock(String),
    /// The seccomp filter could not be built or installed.
    #[error("seccomp: {0}")]
    Seccomp(String),
}

/// The confinement of this process, recorded by [`apply`].
#[derive(Clone, Debug)]
struct SelfState {
    kind: ProcessKind,
    mode: Mode,
    enforcement: Enforcement,
}

static SELF: Mutex<Option<SelfState>> = Mutex::new(None);

fn self_state() -> Option<SelfState> {
    SELF.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// What `profile` needs from the kernel that `support` lacks (empty: it can run strictly).
pub fn missing(profile: &Profile, support: &Support) -> Vec<String> {
    let mut out = Vec::new();
    if profile.kind == ProcessKind::BrowserSource {
        // Chromium's own sandbox needs unprivileged user namespaces, which Ubuntu only allows
        // through an AppArmor profile (plan III, D3).
        if !support.apparmor {
            out.push("AppArmor (Chromium sandbox profile)".to_string());
        }
        return out;
    }
    // ABI 3 also governs truncation: below it a process could truncate files it may only read
    if support.landlock_abi < 3 {
        out.push("Landlock ABI 3 (file truncation rules)".to_string());
    }
    if profile.network.port_limited() && support.landlock_abi < 4 {
        out.push("Landlock ABI 4 (network port rules)".to_string());
    }
    if !support.seccomp {
        out.push("seccomp filter".to_string());
    }
    out
}

/// Confine this process with `profile`. Call once, early, before threads are started and before any
/// untrusted input is opened. See the crate documentation for [`Mode`].
pub fn apply(profile: &Profile, mode: Mode) -> Result<Enforcement, SandboxError> {
    if let Some(s) = self_state() {
        return Err(SandboxError::AlreadyApplied(s.kind.id()));
    }
    if profile.kind == ProcessKind::BrowserSource {
        return Err(SandboxError::External(profile.kind.id()));
    }
    let support = probe();
    let gaps = missing(profile, &support);
    if mode == Mode::Strict && !gaps.is_empty() {
        return Err(SandboxError::Missing(gaps));
    }
    let enforcement = apply_impl(profile, &support, mode == Mode::Strict)?;
    let mut guard = SELF.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    *guard = Some(SelfState { kind: profile.kind, mode, enforcement: enforcement.clone() });
    Ok(enforcement)
}

#[cfg(target_os = "linux")]
fn apply_impl(profile: &Profile, support: &Support, strict: bool) -> Result<Enforcement, SandboxError> {
    linux::apply(profile, support, strict)
}

#[cfg(not(target_os = "linux"))]
fn apply_impl(_: &Profile, _: &Support, _: bool) -> Result<Enforcement, SandboxError> {
    Err(SandboxError::Unsupported)
}

/// `security.status`: what the running system supports, the profile of every CFA4 process kind
/// (built from `ctx`) and whether it can start under plan D4, and this process's own confinement.
pub fn status(ctx: &Context) -> Value {
    let support = probe();
    let processes: Vec<Value> = ProcessKind::ALL
        .iter()
        .map(|&kind| {
            let p = Profile::for_process(kind, ctx);
            let gaps = missing(&p, &support);
            json!({
                "kind": kind.id(),
                "label": kind.label(),
                "read": p.read,
                "write": p.write,
                "outputs": p.outputs,
                "exec": p.exec,
                "network": p.network,
                "gpu": p.gpu,
                "canStart": gaps.is_empty(),
                "missing": gaps,
            })
        })
        .collect();
    let me = self_state();
    json!({
        "support": support,
        "processes": processes,
        "self": {
            "kind": me.as_ref().map(|s| s.kind.id()),
            "mode": me.as_ref().map(|s| s.mode),
            "enforcement": me.as_ref().map(|s| &s.enforcement),
            // what the kernel reports for this process, independent of this crate's bookkeeping
            "seccompMode": support.self_seccomp_mode,
            "noNewPrivs": support.self_no_new_privs,
        },
        // D4: a process running without its protections is unsupported; bug reports record this.
        "unsupported": me.as_ref().is_some_and(|s| s.mode == Mode::Unprotected && !missing(&Profile::for_process(s.kind, ctx), &support).is_empty()),
    })
}

#[cfg(test)]
mod tests;
