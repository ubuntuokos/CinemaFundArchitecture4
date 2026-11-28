//! Plans: what the helper would change, shown to the user before anything runs, and the SHA-256
//! digest that ties the approved plan to what runs as root (ADR 0017 §2).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Version of the plan format.
pub const PLAN_FORMAT: u32 = 1;

/// Packages the helper may install or remove (the closed list of ADR 0017 §3).
pub const KNOWN_PACKAGES: &[&str] = &["intel-media-va-driver", "mesa-va-drivers"];

/// Groups the helper may add the user to or remove them from.
pub const KNOWN_GROUPS: &[&str] = &["render", "video"];

/// One change to the system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// Add the user to a group (`gpasswd -a`).
    AddToGroup { user: String, group: String },
    /// Remove the user from a group (`gpasswd -d`): the undo of [`Action::AddToGroup`].
    RemoveFromGroup { user: String, group: String },
    /// Install packages from the distribution's archive (`apt-get install`).
    InstallPackages { packages: Vec<String> },
    /// Remove packages (`apt-get remove`): the undo of [`Action::InstallPackages`].
    RemovePackages { packages: Vec<String> },
}

/// A program and its arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Cmd {
    pub program: &'static str,
    pub args: Vec<String>,
    /// Extra environment variables.
    pub env: Vec<(&'static str, &'static str)>,
}

/// A user or group name as `useradd` accepts it: `[a-z_][a-z0-9_-]*` and at most 32 bytes.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 32
        && chars.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

impl Action {
    /// The action that undoes this one.
    pub fn undo(&self) -> Action {
        match self {
            Action::AddToGroup { user, group } => Action::RemoveFromGroup { user: user.clone(), group: group.clone() },
            Action::RemoveFromGroup { user, group } => Action::AddToGroup { user: user.clone(), group: group.clone() },
            Action::InstallPackages { packages } => Action::RemovePackages { packages: packages.clone() },
            Action::RemovePackages { packages } => Action::InstallPackages { packages: packages.clone() },
        }
    }

    /// Checks the action against the closed lists, so a forged plan cannot name anything else.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Action::AddToGroup { user, group } | Action::RemoveFromGroup { user, group } => {
                if !valid_name(user) || user == "root" {
                    return Err("invalid user name in plan".into());
                }
                if !KNOWN_GROUPS.contains(&group.as_str()) {
                    return Err("group not on the helper's list".into());
                }
            }
            Action::InstallPackages { packages } | Action::RemovePackages { packages } => {
                if packages.is_empty() || packages.iter().any(|p| !KNOWN_PACKAGES.contains(&p.as_str())) {
                    return Err("package not on the helper's list".into());
                }
            }
        }
        Ok(())
    }

    /// The command that carries the action out. Programs are absolute paths (no `PATH` lookup).
    pub fn command(&self) -> Cmd {
        const APT_ENV: &[(&str, &str)] = &[("DEBIAN_FRONTEND", "noninteractive")];
        match self {
            Action::AddToGroup { user, group } => Cmd { program: "/usr/bin/gpasswd", args: vec!["-a".into(), user.clone(), group.clone()], env: Vec::new() },
            Action::RemoveFromGroup { user, group } => {
                Cmd { program: "/usr/bin/gpasswd", args: vec!["-d".into(), user.clone(), group.clone()], env: Vec::new() }
            }
            Action::InstallPackages { packages } => {
                let mut args: Vec<String> = ["install", "-y", "--no-install-recommends", "--"].map(String::from).to_vec();
                args.extend(packages.iter().cloned());
                Cmd { program: "/usr/bin/apt-get", args, env: APT_ENV.to_vec() }
            }
            Action::RemovePackages { packages } => {
                let mut args: Vec<String> = ["remove", "-y", "--"].map(String::from).to_vec();
                args.extend(packages.iter().cloned());
                Cmd { program: "/usr/bin/apt-get", args, env: APT_ENV.to_vec() }
            }
        }
    }
}

/// What applying a component would change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub format: u32,
    pub component: String,
    /// The user the plan is for (from `PKEXEC_UID` when run as root, never from the caller).
    pub user: String,
    pub actions: Vec<Action>,
    /// The user has to log out and back in for the change to take effect.
    #[serde(default)]
    pub relogin_required: bool,
}

impl Plan {
    /// SHA-256 of the plan's canonical JSON, as lowercase hex.
    pub fn digest(&self) -> String {
        let bytes = serde_json::to_vec(self).unwrap_or_default();
        Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> Plan {
        Plan {
            format: PLAN_FORMAT,
            component: "gpu-access".into(),
            user: "anna".into(),
            actions: vec![Action::AddToGroup { user: "anna".into(), group: "render".into() }],
            relogin_required: true,
        }
    }

    #[test]
    fn the_digest_changes_with_any_field() {
        let a = plan();
        assert_eq!(a.digest().len(), 64);
        assert_eq!(a.digest(), plan().digest());
        let mut b = plan();
        b.actions = vec![Action::AddToGroup { user: "anna".into(), group: "video".into() }];
        assert_ne!(a.digest(), b.digest());
        let mut c = plan();
        c.user = "bob".into();
        assert_ne!(a.digest(), c.digest());
    }

    #[test]
    fn only_listed_groups_packages_and_valid_users_pass() {
        assert!(Action::AddToGroup { user: "anna".into(), group: "render".into() }.validate().is_ok());
        assert!(Action::AddToGroup { user: "anna".into(), group: "sudo".into() }.validate().is_err());
        assert!(Action::AddToGroup { user: "root".into(), group: "video".into() }.validate().is_err());
        assert!(Action::AddToGroup { user: "-o x".into(), group: "video".into() }.validate().is_err());
        assert!(Action::InstallPackages { packages: vec!["mesa-va-drivers".into()] }.validate().is_ok());
        assert!(Action::InstallPackages { packages: vec!["openssh-server".into()] }.validate().is_err());
        assert!(Action::InstallPackages { packages: vec![] }.validate().is_err());
    }

    #[test]
    fn commands_use_absolute_programs_and_undo_reverses() {
        let add = Action::AddToGroup { user: "anna".into(), group: "render".into() };
        assert_eq!(add.command().program, "/usr/bin/gpasswd");
        assert_eq!(add.command().args, ["-a", "anna", "render"]);
        assert_eq!(add.undo().command().args, ["-d", "anna", "render"]);
        let install = Action::InstallPackages { packages: vec!["mesa-va-drivers".into()] };
        let cmd = install.command();
        assert_eq!(cmd.program, "/usr/bin/apt-get");
        assert_eq!(cmd.args.last().map(String::as_str), Some("mesa-va-drivers"));
        assert!(cmd.args.contains(&"--".to_string()));
        assert_eq!(install.undo(), Action::RemovePackages { packages: vec!["mesa-va-drivers".into()] });
    }
}
