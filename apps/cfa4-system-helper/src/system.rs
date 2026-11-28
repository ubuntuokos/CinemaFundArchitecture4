//! What the helper reads about the machine, always beneath a root directory (`/` on a real system,
//! a fake tree in tests): users and groups, the GPU and NPU device nodes, installed packages, VA-API
//! driver files and NPU firmware. Nothing here writes or runs a program.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Most entries read from one directory or file (a hostile tree cannot grow the output).
const MAX_ENTRIES: usize = 256;
/// Longest file the helper reads (`/etc/group`, the dpkg status file).
const MAX_FILE: u64 = 64 * 1024 * 1024;

/// A user account from `<root>/etc/passwd`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct User {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
}

/// A group from `<root>/etc/group`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Group {
    pub name: String,
    pub gid: u32,
    pub members: Vec<String>,
}

/// A GPU or NPU device node (`/dev/dri/renderD128`, `/dev/accel/accel0`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Node {
    /// The path on the real system (`/dev/dri/renderD128`), whatever the root.
    pub path: String,
    pub uid: u32,
    pub gid: u32,
    /// The permission bits (`0o660`).
    pub mode: u32,
}

/// The machine beneath `root`.
#[derive(Debug, Clone)]
pub struct System {
    root: PathBuf,
}

fn read_bounded(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE {
        return None;
    }
    fs::read_to_string(path).ok()
}

impl System {
    pub fn new(root: impl Into<PathBuf>) -> System {
        System { root: root.into() }
    }

    /// `true` when the root is the real `/`.
    pub fn is_real_root(&self) -> bool {
        self.root == Path::new("/")
    }

    /// `<root>/<rel>` (`rel` has no leading slash).
    pub fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel.trim_start_matches('/'))
    }

    pub fn users(&self) -> Vec<User> {
        let Some(text) = read_bounded(&self.path("etc/passwd")) else { return Vec::new() };
        text.lines()
            .filter_map(|line| {
                let f: Vec<&str> = line.split(':').collect();
                if f.len() < 4 || f[0].is_empty() {
                    return None;
                }
                Some(User { name: f[0].to_string(), uid: f[2].parse().ok()?, gid: f[3].parse().ok()? })
            })
            .take(MAX_ENTRIES * 64)
            .collect()
    }

    pub fn user_by_uid(&self, uid: u32) -> Option<User> {
        self.users().into_iter().find(|u| u.uid == uid)
    }

    pub fn groups(&self) -> Vec<Group> {
        let Some(text) = read_bounded(&self.path("etc/group")) else { return Vec::new() };
        text.lines()
            .filter_map(|line| {
                let f: Vec<&str> = line.split(':').collect();
                if f.len() < 4 || f[0].is_empty() {
                    return None;
                }
                let members = f[3].split(',').filter(|m| !m.is_empty()).map(str::to_string).collect();
                Some(Group { name: f[0].to_string(), gid: f[2].parse().ok()?, members })
            })
            .take(MAX_ENTRIES * 64)
            .collect()
    }

    /// The gids the user belongs to: the primary group and every group listing them as a member.
    pub fn gids_of(&self, user: &User) -> Vec<u32> {
        let mut gids = vec![user.gid];
        for g in self.groups() {
            if g.members.iter().any(|m| m == &user.name) && !gids.contains(&g.gid) {
                gids.push(g.gid);
            }
        }
        gids
    }

    /// The GPU render nodes and NPU accelerator nodes, sorted by path.
    pub fn nodes(&self) -> Vec<Node> {
        let mut nodes = Vec::new();
        for (dir, prefix) in [("dev/dri", "renderD"), ("dev/accel", "accel")] {
            let Ok(entries) = fs::read_dir(self.path(dir)) else { continue };
            for entry in entries.flatten().take(MAX_ENTRIES) {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.starts_with(prefix) || !name[prefix.len()..].chars().all(|c| c.is_ascii_digit()) {
                    continue;
                }
                let Ok(meta) = fs::metadata(entry.path()) else { continue };
                nodes.push(Node { path: format!("/{dir}/{name}"), uid: meta.uid(), gid: meta.gid(), mode: meta.mode() & 0o7777 });
            }
        }
        nodes.sort_by(|a, b| a.path.cmp(&b.path));
        nodes
    }

    /// `true` when dpkg lists the package as installed (`Status: install ok installed`).
    pub fn package_installed(&self, package: &str) -> bool {
        let Some(text) = read_bounded(&self.path("var/lib/dpkg/status")) else { return false };
        let mut name_matches = false;
        for line in text.lines() {
            if line.is_empty() {
                name_matches = false;
            } else if let Some(name) = line.strip_prefix("Package: ") {
                name_matches = name.trim() == package;
            } else if name_matches && line.starts_with("Status: ") {
                return line.trim_end().ends_with(" installed") && line.contains("install ok");
            }
        }
        false
    }

    /// `true` when the VA-API user-space driver file exists (`iHD`, `radeonsi`, `nouveau`, …).
    pub fn vaapi_driver_present(&self, driver: &str) -> bool {
        ["usr/lib/x86_64-linux-gnu/dri", "usr/lib/aarch64-linux-gnu/dri", "usr/lib/dri"]
            .iter()
            .any(|dir| self.path(dir).join(format!("{driver}_drv_video.so")).is_file())
    }

    /// `true` when the firmware directory exists and holds at least one entry.
    pub fn firmware_present(&self, dir: &str) -> bool {
        fs::read_dir(self.path(dir)).is_ok_and(|mut e| e.next().is_some())
    }

    /// The accelerator inventory of `cfa4-route` (GPUs, NPUs, vendors, drivers).
    pub fn inventory(&self) -> cfa4_route::Inventory {
        cfa4_route::probe::probe(&self.root)
    }
}

/// The real and effective user id of this process (from `/proc/self/status`, no `unsafe`).
pub fn process_uids() -> Option<(u32, u32)> {
    let text = fs::read_to_string("/proc/self/status").ok()?;
    let line = text.lines().find(|l| l.starts_with("Uid:"))?;
    let mut f = line.split_whitespace().skip(1);
    let real = f.next()?.parse().ok()?;
    let effective = f.next()?.parse().ok()?;
    Some((real, effective))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn users_groups_and_memberships_come_from_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let sys = System::new(dir.path());
        fs::create_dir_all(sys.path("etc")).unwrap();
        fs::write(sys.path("etc/passwd"), "root:x:0:0::/root:/bin/sh\nanna:x:1000:1000::/home/anna:/bin/bash\nbroken\n").unwrap();
        fs::write(sys.path("etc/group"), "root:x:0:\nanna:x:1000:\nvideo:x:44:anna,bob\nrender:x:110:\n").unwrap();
        let anna = sys.user_by_uid(1000).unwrap();
        assert_eq!(anna.name, "anna");
        assert_eq!(sys.gids_of(&anna), vec![1000, 44]);
        assert!(sys.user_by_uid(42).is_none());
    }

    #[test]
    fn device_nodes_are_listed_with_their_owner_and_mode() {
        let dir = tempfile::tempdir().unwrap();
        let sys = System::new(dir.path());
        fs::create_dir_all(sys.path("dev/dri")).unwrap();
        fs::create_dir_all(sys.path("dev/accel")).unwrap();
        for n in ["dev/dri/renderD129", "dev/dri/renderD128", "dev/dri/card0", "dev/dri/renderDx", "dev/accel/accel0"] {
            fs::write(sys.path(n), "").unwrap();
        }
        let paths: Vec<String> = sys.nodes().into_iter().map(|n| n.path).collect();
        assert_eq!(paths, ["/dev/accel/accel0", "/dev/dri/renderD128", "/dev/dri/renderD129"]);
    }

    #[test]
    fn dpkg_status_needs_install_ok_installed() {
        let dir = tempfile::tempdir().unwrap();
        let sys = System::new(dir.path());
        fs::create_dir_all(sys.path("var/lib/dpkg")).unwrap();
        fs::write(
            sys.path("var/lib/dpkg/status"),
            "Package: mesa-va-drivers\nStatus: install ok installed\n\nPackage: intel-media-va-driver\nStatus: deinstall ok config-files\n",
        )
        .unwrap();
        assert!(sys.package_installed("mesa-va-drivers"));
        assert!(!sys.package_installed("intel-media-va-driver"));
        assert!(!sys.package_installed("mesa"));
    }

    #[test]
    fn this_process_has_uids() {
        assert!(process_uids().is_some());
    }
}
