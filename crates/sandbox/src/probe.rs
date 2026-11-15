//! What the running kernel offers, read without restricting anything.

use serde::{Deserialize, Serialize};

/// The protections the running system supports, and this process's current seccomp state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Support {
    /// `/proc/sys/kernel/osrelease` (empty when unreadable).
    pub kernel: String,
    /// Highest Landlock ABI both the kernel and the `landlock` crate understand (0: none).
    pub landlock_abi: u8,
    /// The kernel can install seccomp-bpf filters (`Seccomp_filters` in `/proc/self/status`).
    pub seccomp: bool,
    /// AppArmor is enabled (`/sys/module/apparmor/parameters/enabled`).
    pub apparmor: bool,
    /// Unprivileged user namespaces need an AppArmor profile (Ubuntu ≥ 23.10;
    /// `kernel.apparmor_restrict_unprivileged_userns`).
    pub userns_restricted: bool,
    /// The calling thread's seccomp mode (0 none, 1 strict, 2 filter).
    pub self_seccomp_mode: u8,
    /// `no_new_privs` is set on the calling thread.
    pub self_no_new_privs: bool,
}

fn read_trim(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// The value of `key:` in `/proc/self/status`.
fn proc_status_field(status: &str, key: &str) -> Option<String> {
    status.lines().find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix(':')).map(|v| v.trim().to_string()))
}

/// Read the kernel's support for every protection. Cheap (a few small `/proc` and `/sys` reads and,
/// on Linux, creating and closing empty Landlock rulesets); restricts nothing.
pub fn probe() -> Support {
    // per thread: Landlock and seccomp bind the thread that applied them (and its children)
    let status = std::fs::read_to_string("/proc/thread-self/status").or_else(|_| std::fs::read_to_string("/proc/self/status")).unwrap_or_default();
    Support {
        kernel: read_trim("/proc/sys/kernel/osrelease").unwrap_or_default(),
        landlock_abi: landlock_abi(),
        seccomp: proc_status_field(&status, "Seccomp_filters").is_some(),
        apparmor: read_trim("/sys/module/apparmor/parameters/enabled").is_some_and(|v| v.starts_with('Y')),
        userns_restricted: read_trim("/proc/sys/kernel/apparmor_restrict_unprivileged_userns").is_some_and(|v| v == "1"),
        self_seccomp_mode: proc_status_field(&status, "Seccomp").and_then(|v| v.parse().ok()).unwrap_or(0),
        self_no_new_privs: proc_status_field(&status, "NoNewPrivs").is_some_and(|v| v == "1"),
    }
}

#[cfg(target_os = "linux")]
fn landlock_abi() -> u8 {
    crate::linux::probe_abi()
}

#[cfg(not(target_os = "linux"))]
fn landlock_abi() -> u8 {
    0
}

#[cfg(test)]
mod tests {
    use super::proc_status_field;

    #[test]
    fn reads_proc_status_fields() {
        let s = "Name:\tcfa4\nNoNewPrivs:\t1\nSeccomp:\t2\nSeccomp_filters:\t1\n";
        assert_eq!(proc_status_field(s, "Seccomp").as_deref(), Some("2"));
        assert_eq!(proc_status_field(s, "Seccomp_filters").as_deref(), Some("1"));
        assert_eq!(proc_status_field(s, "NoNewPrivs").as_deref(), Some("1"));
        assert_eq!(proc_status_field(s, "Missing"), None);
        assert_eq!(proc_status_field("", "Seccomp"), None);
    }
}
