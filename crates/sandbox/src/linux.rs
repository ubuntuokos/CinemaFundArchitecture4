//! Landlock and seccomp through their safe Rust crates.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use landlock::{
    ABI, Access, AccessFs, AccessNet, BitFlags, CompatLevel, Compatible, LandlockStatus, NetPort, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreated,
    RulesetCreatedAttr, RulesetStatus, Scope,
};
use seccompiler::{BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter, SeccompRule, TargetArch};

use crate::profile::{GPU_DEVICES, SYSTEM_DEVICES, SYSTEM_READ};
use crate::{Enforcement, Network, ProcessKind, Profile, SandboxError, Support};

/// The Landlock ABIs this crate builds rules for, newest first. Capped at ABI 8: ABI 9 adds
/// `ResolveUnix`, which would cut the Wayland, PipeWire and D-Bus sockets until the profiles list
/// them (phase B3).
const ABIS: [(ABI, u8); 8] = [(ABI::V8, 8), (ABI::V7, 7), (ABI::V6, 6), (ABI::V5, 5), (ABI::V4, 4), (ABI::V3, 3), (ABI::V2, 2), (ABI::V1, 1)];

/// The dynamic loader: starting any program needs execute access to it.
const LOADERS: &[&str] = &["/lib64/ld-linux-x86-64.so.2", "/usr/lib64/ld-linux-x86-64.so.2"];

/// The highest ABI [`probe_abi`] can tell apart: every right CFA4's rules use exists by ABI 6
/// (ABIs 7 and 8 add audit-logging and thread-sync flags, which only show when a ruleset is
/// enforced). A kernel with ABI 6 or newer probes as 6.
const PROBE_MAX: u8 = 6;

fn abi_of(n: u8) -> ABI {
    ABIS.iter().find(|(_, v)| *v == n).map(|(a, _)| *a).unwrap_or(ABI::Unsupported)
}

fn abi_number(abi: ABI) -> u8 {
    ABIS.iter().find(|(a, _)| *a == abi).map(|(_, n)| *n).unwrap_or(0)
}

/// Whether the kernel accepts a ruleset that handles every right `abi` introduced: the file-system
/// rights (ABI 1–3, 5), the TCP port rights (ABI 4) and the scopes (ABI 6). Creating and dropping a
/// ruleset restricts nothing.
fn kernel_accepts(abi: ABI, n: u8) -> bool {
    let hard = || Ruleset::default().set_compatibility(CompatLevel::HardRequirement);
    let fs = hard().handle_access(AccessFs::from_all(abi)).and_then(|r| r.create()).is_ok();
    let net = n < 4 || hard().handle_access(AccessNet::from_all(abi)).and_then(|r| r.create()).is_ok();
    let scope = n < 6 || hard().scope(Scope::from_all(abi)).and_then(|r| r.create()).is_ok();
    fs && net && scope
}

/// The kernel's Landlock ABI, up to [`PROBE_MAX`], from the rights it accepts: each ABI up to 6
/// adds rights the previous one lacks (ABI 4 only network rights, so the file-system rights alone
/// would read ABI 3 as 4).
pub(crate) fn probe_abi() -> u8 {
    ABIS.iter().filter(|(_, n)| *n <= PROBE_MAX).find(|(abi, n)| kernel_accepts(*abi, *n)).map(|(_, n)| *n).unwrap_or(0)
}

fn ll(e: impl std::fmt::Display) -> SandboxError {
    SandboxError::Landlock(e.to_string())
}

/// Add a rule for `path` (skipped when it does not exist: a missing directory grants nothing).
fn allow(c: RulesetCreated, path: &Path, access: BitFlags<AccessFs>, abi: ABI) -> Result<RulesetCreated, SandboxError> {
    let Ok(fd) = PathFd::new(path) else { return Ok(c) };
    let access = if path.is_dir() { access } else { access & AccessFs::from_file(abi) };
    if access.is_empty() {
        return Ok(c);
    }
    c.add_rule(PathBeneath::new(fd, access)).map_err(ll)
}

/// An output file ([`Profile::outputs`]) that does not exist yet is created empty now, before
/// confinement, so the rule can name the file itself: a rule on its directory would cover every
/// sibling (`~/model.safetensors` would open `~/.ssh`). `create_new` never follows or replaces an
/// existing path, so a planted link is not written through.
fn create_output(path: &Path) -> Result<(), SandboxError> {
    if path.exists() {
        return Ok(());
    }
    match std::fs::OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(ll(format!("cannot create the output {}: {e}", path.display()))),
    }
}

/// The NVIDIA device nodes (`/dev/nvidia0`, `/dev/nvidia1`…) present now.
fn nvidia_nodes() -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir("/dev") else { return Vec::new() };
    rd.filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_prefix("nvidia"))
                .is_some_and(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()))
        })
        .collect()
}

fn landlock(profile: &Profile, abi: ABI, strict: bool, en: &mut Enforcement) -> Result<(), SandboxError> {
    let all = AccessFs::from_all(abi);
    let read: BitFlags<AccessFs> = AccessFs::ReadFile | AccessFs::ReadDir;
    let device_ops: BitFlags<AccessFs> = AccessFs::MakeChar | AccessFs::MakeBlock;
    let write = AccessFs::from_write(abi) & !device_ops & all;
    let ioctl = BitFlags::from(AccessFs::IoctlDev) & all;

    // strict: a right the kernel lacks is an error, not silently dropped (BestEffort)
    let level = if strict { CompatLevel::HardRequirement } else { CompatLevel::BestEffort };
    let mut rs = Ruleset::default().set_compatibility(level).handle_access(all).map_err(ll)?;
    let net_ports = profile.network.port_limited() && abi >= ABI::V4;
    if net_ports {
        rs = rs.handle_access(AccessNet::from_all(abi)).map_err(ll)?;
    }
    if abi >= ABI::V6 {
        let mut scope: BitFlags<Scope> = Scope::Signal.into();
        // X11 (XWayland) and some sound servers use abstract sockets: keep them for display processes
        if !matches!(profile.kind, ProcessKind::QtGui | ProcessKind::StudioHost) {
            scope |= Scope::AbstractUnixSocket;
        }
        rs = rs.scope(scope).map_err(ll)?;
    }
    let mut c = rs.create().map_err(ll)?;

    for p in SYSTEM_READ {
        c = allow(c, Path::new(p), read, abi)?;
    }
    for p in SYSTEM_DEVICES {
        c = allow(c, Path::new(p), AccessFs::ReadFile | AccessFs::WriteFile, abi)?;
    }
    if profile.gpu {
        let gpu = read | AccessFs::WriteFile | ioctl;
        for p in GPU_DEVICES.iter().map(PathBuf::from).chain(nvidia_nodes()) {
            c = allow(c, &p, gpu, abi)?;
        }
    }
    for p in &profile.read {
        c = allow(c, p, read, abi)?;
    }
    for p in &profile.write {
        c = allow(c, p, read | write, abi)?;
    }
    for p in &profile.outputs {
        create_output(p)?;
        c = allow(c, p, read | write, abi)?;
    }
    if !profile.exec.is_empty() {
        let run = AccessFs::ReadFile | AccessFs::Execute;
        for p in LOADERS.iter().map(PathBuf::from).chain(profile.exec.iter().cloned()) {
            c = allow(c, &p, run, abi)?;
        }
    }
    if net_ports {
        match &profile.network {
            Network::Loopback { ports } => {
                for &port in ports {
                    c = c.add_rule(NetPort::new(port, AccessNet::BindTcp | AccessNet::ConnectTcp)).map_err(ll)?;
                }
            }
            Network::Outbound { ports } => {
                for &port in ports {
                    c = c.add_rule(NetPort::new(port, AccessNet::ConnectTcp)).map_err(ll)?;
                }
            }
            Network::None | Network::Any => {}
        }
    }
    let st = c.restrict_self().map_err(ll)?;
    en.landlock_full = st.ruleset == RulesetStatus::FullyEnforced;
    // the ABI the kernel reports now that a ruleset is enforced (the probe stops at 6)
    if let LandlockStatus::Available { effective_abi, .. } = st.landlock {
        en.landlock_abi = abi_number(effective_abi).max(en.landlock_abi);
    }
    if strict && !en.landlock_full {
        return Err(ll(format!("the rules were not fully enforced ({:?})", st.ruleset)));
    }
    en.network_ports = net_ports && st.ruleset != RulesetStatus::NotEnforced;
    en.no_new_privs = st.no_new_privs;
    Ok(())
}

/// System calls no CFA4 process needs: debugging or reading other processes, kernel modules, mounts
/// and namespaces, keyrings, clocks, io_uring (a frequent source of kernel bugs), eBPF.
const DENY_ALWAYS: &[libc::c_long] = &[
    libc::SYS_ptrace,
    libc::SYS_process_vm_readv,
    libc::SYS_process_vm_writev,
    libc::SYS_kexec_load,
    libc::SYS_kexec_file_load,
    libc::SYS_init_module,
    libc::SYS_finit_module,
    libc::SYS_delete_module,
    libc::SYS_bpf,
    libc::SYS_perf_event_open,
    libc::SYS_mount,
    libc::SYS_umount2,
    libc::SYS_pivot_root,
    libc::SYS_chroot,
    libc::SYS_fsopen,
    libc::SYS_fsmount,
    libc::SYS_fspick,
    libc::SYS_move_mount,
    libc::SYS_open_tree,
    libc::SYS_swapon,
    libc::SYS_swapoff,
    libc::SYS_reboot,
    libc::SYS_settimeofday,
    libc::SYS_clock_settime,
    libc::SYS_clock_adjtime,
    libc::SYS_adjtimex,
    libc::SYS_add_key,
    libc::SYS_request_key,
    libc::SYS_keyctl,
    libc::SYS_userfaultfd,
    libc::SYS_open_by_handle_at,
    libc::SYS_name_to_handle_at,
    libc::SYS_io_uring_setup,
    libc::SYS_io_uring_enter,
    libc::SYS_io_uring_register,
    libc::SYS_acct,
    libc::SYS_quotactl,
    libc::SYS_syslog,
    libc::SYS_iopl,
    libc::SYS_ioperm,
    libc::SYS_vhangup,
];

/// Namespaces: Chromium's sandbox (the studio host's browser source) needs them; nothing else does.
const DENY_NAMESPACES: &[libc::c_long] = &[libc::SYS_unshare, libc::SYS_setns];

const DENY_EXEC: &[libc::c_long] = &[libc::SYS_execve, libc::SYS_execveat];

/// Socket families refused when the process has no network (Unix sockets stay for the parent channel).
const NET_FAMILIES: &[libc::c_int] = &[libc::AF_INET, libc::AF_INET6, libc::AF_PACKET];

/// IP families whose socket types a profile with some network may still lose.
const IP_FAMILIES: &[libc::c_int] = &[libc::AF_INET, libc::AF_INET6];

/// The `type` argument of `socket` without `SOCK_NONBLOCK` / `SOCK_CLOEXEC`.
const SOCK_TYPE_MASK: u64 = 0xf;

fn sc(e: impl std::fmt::Display) -> SandboxError {
    SandboxError::Seccomp(e.to_string())
}

/// The seccomp filter of `profile`: allow by default, `EPERM` for the listed calls.
pub(crate) fn seccomp_program(profile: &Profile) -> Result<BpfProgram, SandboxError> {
    let mut rules: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();
    let mut deny: Vec<libc::c_long> = DENY_ALWAYS.to_vec();
    if profile.kind != ProcessKind::StudioHost {
        deny.extend_from_slice(DENY_NAMESPACES);
    }
    if profile.exec.is_empty() {
        deny.extend_from_slice(DENY_EXEC);
    }
    for nr in deny {
        rules.insert(nr, Vec::new());
    }
    let socket_rules = socket_rules(&profile.network)?;
    if !socket_rules.is_empty() {
        rules.insert(libc::SYS_socket, socket_rules);
    }
    let arch = TargetArch::try_from(std::env::consts::ARCH).map_err(sc)?;
    let eperm = u32::try_from(libc::EPERM).map_err(sc)?;
    let filter = SeccompFilter::new(rules, SeccompAction::Allow, SeccompAction::Errno(eperm), arch).map_err(sc)?;
    BpfProgram::try_from(filter).map_err(sc)
}

/// The `socket` calls `network` refuses:
///
/// - `None`: IPv4, IPv6 and packet sockets;
/// - `Loopback`: packet sockets and IPv4/IPv6 datagram and raw sockets. Landlock (up to ABI 8)
///   restricts TCP ports only, so UDP would reach any address; a loopback process needs neither
///   UDP nor DNS;
/// - `Outbound`: packet and raw sockets. UDP stays for DNS (the system resolver, often
///   `127.0.0.53`), so a compromised downloader could still send UDP datagrams: an accepted risk
///   until Landlock handles UDP (docs/security/threat-model.md). It reads none of the user's files;
/// - `Any`: nothing.
fn socket_rules(network: &Network) -> Result<Vec<SeccompRule>, SandboxError> {
    let family = |f: libc::c_int| -> Result<SeccompCondition, SandboxError> {
        SeccompCondition::new(0, SeccompCmpArgLen::Dword, SeccompCmpOp::Eq, u64::try_from(f).map_err(sc)?).map_err(sc)
    };
    let sock_type = |t: libc::c_int| -> Result<SeccompCondition, SandboxError> {
        SeccompCondition::new(1, SeccompCmpArgLen::Dword, SeccompCmpOp::MaskedEq(SOCK_TYPE_MASK), u64::try_from(t).map_err(sc)?).map_err(sc)
    };
    let mut out = Vec::new();
    let refused_types: &[libc::c_int] = match network {
        Network::None => {
            for &f in NET_FAMILIES {
                out.push(SeccompRule::new(vec![family(f)?]).map_err(sc)?);
            }
            return Ok(out);
        }
        Network::Loopback { .. } => &[libc::SOCK_DGRAM, libc::SOCK_RAW],
        Network::Outbound { .. } => &[libc::SOCK_RAW],
        Network::Any => return Ok(out),
    };
    out.push(SeccompRule::new(vec![family(libc::AF_PACKET)?]).map_err(sc)?);
    for &f in IP_FAMILIES {
        for &t in refused_types {
            out.push(SeccompRule::new(vec![family(f)?, sock_type(t)?]).map_err(sc)?);
        }
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) fn socket_rules_len(network: &Network) -> usize {
    socket_rules(network).map(|r| r.len()).unwrap_or(0)
}

pub(crate) fn apply(profile: &Profile, support: &Support, strict: bool) -> Result<Enforcement, SandboxError> {
    let mut en = Enforcement::default();
    let abi = abi_of(support.landlock_abi);
    if abi != ABI::Unsupported {
        en.landlock_abi = support.landlock_abi;
        landlock(profile, abi, strict, &mut en)?;
    }
    if support.seccomp {
        let prog = seccomp_program(profile)?;
        // also sets no_new_privs
        seccompiler::apply_filter(&prog).map_err(sc)?;
        en.seccomp = true;
        en.no_new_privs = true;
    }
    Ok(en)
}
