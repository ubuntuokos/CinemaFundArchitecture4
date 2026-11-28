//! The closed list of components (ADR 0017 §3, CFA4-SETTINGS-HW-003 R1): what each one checks
//! without root and what its plan would change. Every component is vendor-neutral.

use std::collections::BTreeSet;
use std::fs;

use cfa4_route::DeviceKind;
use serde::Serialize;

use crate::plan::{Action, PLAN_FORMAT, Plan};
use crate::system::{System, User};

/// The components, in the order the settings page shows them.
pub const COMPONENTS: &[&str] = &["gpu-access", "vaapi-drivers", "npu-check"];

/// How a component stands on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Nothing to do.
    Ok,
    /// Something is missing; see the details (and the plan, when `can_apply`).
    NeedsAction,
    /// The machine has nothing this component is for.
    NotNeeded,
}

/// The result of `check`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    pub component: &'static str,
    pub status: Status,
    /// Plain-English facts for the page (the GUI translates the fixed ones it knows).
    pub details: Vec<String>,
    /// `apply` can fix what is missing; `false` for check-only components and for problems the
    /// helper does not fix (the user gets guidance instead).
    pub can_apply: bool,
    /// The actions `plan` would contain.
    #[serde(skip)]
    pub actions: Vec<Action>,
    #[serde(skip)]
    pub relogin_required: bool,
}

/// `true` when the user can read and write the node by its owner, group or other permission bits.
fn node_accessible(node: &crate::system::Node, user: &User, gids: &[u32]) -> bool {
    node.mode & 0o006 == 0o006 || (node.uid == user.uid && node.mode & 0o600 == 0o600) || (gids.contains(&node.gid) && node.mode & 0o060 == 0o060)
}

fn gpu_access(sys: &System, user: &User) -> Check {
    let nodes = sys.nodes();
    let mut check = Check { component: "gpu-access", status: Status::Ok, details: Vec::new(), can_apply: false, actions: Vec::new(), relogin_required: false };
    if nodes.is_empty() {
        check.status = Status::NotNeeded;
        check.details.push("No GPU render node or NPU accelerator node found.".into());
        return check;
    }
    let gids = sys.gids_of(user);
    let groups = sys.groups();
    let mut needed = BTreeSet::new();
    for node in &nodes {
        if node_accessible(node, user, &gids) {
            check.details.push(format!("{}: accessible", node.path));
            continue;
        }
        let group = groups.iter().find(|g| g.gid == node.gid).map(|g| g.name.as_str());
        match group {
            Some(name) if node.mode & 0o060 == 0o060 && crate::plan::KNOWN_GROUPS.contains(&name) => {
                check.details.push(format!("{}: needs group {name}", node.path));
                needed.insert(name.to_string());
            }
            _ => check.details.push(format!("{}: not accessible, and no group on the helper's list grants access", node.path)),
        }
    }
    if needed.is_empty() {
        if check.details.iter().any(|d| d.contains("not accessible")) {
            check.status = Status::NeedsAction;
        }
        return check;
    }
    check.status = Status::NeedsAction;
    check.can_apply = true;
    check.relogin_required = true;
    check.actions = needed.into_iter().map(|group| Action::AddToGroup { user: user.name.clone(), group }).collect();
    check.details.push("After the change, log out and back in.".into());
    check
}

/// The Ubuntu package and VA-API driver file for a kernel GPU driver.
fn vaapi_package(driver: &str) -> Option<(&'static str, &'static str)> {
    match driver {
        "i915" | "xe" => Some(("intel-media-va-driver", "iHD")),
        "amdgpu" => Some(("mesa-va-drivers", "radeonsi")),
        "radeon" => Some(("mesa-va-drivers", "r600")),
        "nouveau" => Some(("mesa-va-drivers", "nouveau")),
        _ => None,
    }
}

fn vaapi_drivers(sys: &System) -> Check {
    let inv = sys.inventory();
    let mut check =
        Check { component: "vaapi-drivers", status: Status::Ok, details: Vec::new(), can_apply: false, actions: Vec::new(), relogin_required: false };
    let mut missing = BTreeSet::new();
    let mut any = false;
    for dev in inv.devices().iter().filter(|d| d.has_vaapi_driver()) {
        let Some((package, driver_file)) = dev.driver.as_deref().and_then(vaapi_package) else { continue };
        any = true;
        if sys.package_installed(package) {
            let file = if sys.vaapi_driver_present(driver_file) { "present" } else { "missing" };
            check.details.push(format!("{}: {package} installed, {driver_file}_drv_video.so {file}", dev.name));
        } else {
            check.details.push(format!("{}: needs {package}", dev.name));
            missing.insert(package.to_string());
        }
    }
    if !any {
        check.status = Status::NotNeeded;
        let nvidia = inv.devices().iter().any(|d| d.is_nvidia_proprietary());
        check.details.push(if nvidia { "NVIDIA's driver decodes through NVDEC; VA-API is not needed." } else { "No GPU whose driver uses VA-API." }.into());
        return check;
    }
    if !missing.is_empty() {
        check.status = Status::NeedsAction;
        check.can_apply = true;
        check.actions = vec![Action::InstallPackages { packages: missing.into_iter().collect() }];
    }
    check
}

/// NPU firmware directory for a kernel driver.
fn npu_firmware(driver: &str) -> Option<&'static str> {
    match driver {
        "intel_vpu" => Some("lib/firmware/intel/vpu"),
        "amdxdna" => Some("lib/firmware/amdnpu"),
        _ => None,
    }
}

/// PCI processing accelerators (class 0x1200) from Intel or AMD that no driver has claimed.
fn unbound_npus(sys: &System) -> Vec<String> {
    let Ok(entries) = fs::read_dir(sys.path("sys/bus/pci/devices")) else { return Vec::new() };
    let mut found = Vec::new();
    for entry in entries.flatten().take(1024) {
        let dir = entry.path();
        let read = |name: &str| fs::read_to_string(dir.join(name)).map(|s| s.trim().to_ascii_lowercase()).unwrap_or_default();
        if !read("class").starts_with("0x1200") || dir.join("driver").exists() {
            continue;
        }
        let vendor = match read("vendor").as_str() {
            "0x8086" => "Intel",
            "0x1022" => "AMD",
            _ => continue,
        };
        found.push(format!("{vendor} NPU at {}", entry.file_name().to_string_lossy()));
    }
    found.sort();
    found
}

fn npu_check(sys: &System) -> Check {
    let inv = sys.inventory();
    let mut check = Check { component: "npu-check", status: Status::Ok, details: Vec::new(), can_apply: false, actions: Vec::new(), relogin_required: false };
    let npus: Vec<_> = inv.devices().iter().filter(|d| d.kind == DeviceKind::Npu).collect();
    let unbound = unbound_npus(sys);
    if npus.is_empty() && unbound.is_empty() {
        check.status = Status::NotNeeded;
        check.details.push("No NPU found.".into());
        return check;
    }
    for npu in npus {
        match npu.driver.as_deref().and_then(npu_firmware) {
            Some(dir) if sys.firmware_present(dir) => check.details.push(format!("{}: driver and firmware present", npu.name)),
            Some(dir) => {
                check.status = Status::NeedsAction;
                check.details.push(format!("{}: firmware missing under /{dir} (install the linux-firmware package)", npu.name));
            }
            None => check.details.push(format!("{}: driver {}", npu.name, npu.driver.as_deref().unwrap_or("unknown"))),
        }
    }
    for npu in unbound {
        check.status = Status::NeedsAction;
        let module = if npu.starts_with("Intel") { "intel_vpu" } else { "amdxdna" };
        check.details.push(format!("{npu}: no kernel driver loaded (needs the {module} module, in newer kernels)"));
    }
    check
}

/// Checks one component. `None` for a name not on the list.
pub fn check(component: &str, sys: &System, user: &User) -> Option<Check> {
    match component {
        "gpu-access" => Some(gpu_access(sys, user)),
        "vaapi-drivers" => Some(vaapi_drivers(sys)),
        "npu-check" => Some(npu_check(sys)),
        _ => None,
    }
}

/// The plan of one component. Errors for an unknown component, and for a component with nothing
/// to apply.
pub fn plan(component: &str, sys: &System, user: &User) -> Result<Plan, String> {
    let check = check(component, sys, user).ok_or_else(|| format!("unknown component: {component}"))?;
    if !check.can_apply {
        return Err(format!("{component}: nothing the helper can apply"));
    }
    Ok(Plan {
        format: PLAN_FORMAT,
        component: component.to_string(),
        user: user.name.clone(),
        actions: check.actions,
        relogin_required: check.relogin_required,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    struct Fake {
        dir: tempfile::TempDir,
    }

    impl Fake {
        fn new() -> Fake {
            Fake { dir: tempfile::tempdir().unwrap() }
        }
        fn sys(&self) -> System {
            System::new(self.dir.path())
        }
        fn write(&self, rel: &str, text: &str) {
            let p = self.dir.path().join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
        /// A DRM render node in /sys (for the cfa4-route probe) bound to `driver`.
        fn gpu(&self, node: &str, vendor: &str, driver: &str) {
            let dev = format!("sys/class/drm/{node}/device");
            self.write(&format!("{dev}/vendor"), vendor);
            self.write(&format!("{dev}/device"), "0x1234");
            let target = self.dir.path().join("sys/bus/pci/drivers").join(driver);
            fs::create_dir_all(&target).unwrap();
            symlink(&target, self.dir.path().join(&dev).join("driver")).unwrap();
        }
        /// A device node owned by this test's user with the given group id and mode.
        fn node(&self, rel: &str, mode: u32) -> u32 {
            self.write(rel, "");
            let p = self.dir.path().join(rel);
            fs::set_permissions(&p, fs::Permissions::from_mode(mode)).unwrap();
            use std::os::unix::fs::MetadataExt;
            fs::metadata(p).unwrap().gid()
        }
    }

    /// A user who is not the owner of the test files (uid 1) and whose groups come from /etc/group.
    fn stranger() -> User {
        User { name: "anna".into(), uid: u32::MAX - 1, gid: u32::MAX - 1 }
    }

    #[test]
    fn gpu_access_asks_for_the_render_group_when_only_the_group_can_open_the_node() {
        let f = Fake::new();
        let gid = f.node("dev/dri/renderD128", 0o660);
        f.write("etc/group", &format!("render:x:{gid}:\n"));
        let c = check("gpu-access", &f.sys(), &stranger()).unwrap();
        assert_eq!(c.status, Status::NeedsAction);
        assert!(c.can_apply && c.relogin_required);
        assert_eq!(c.actions, vec![Action::AddToGroup { user: "anna".into(), group: "render".into() }]);
        let p = plan("gpu-access", &f.sys(), &stranger()).unwrap();
        assert_eq!(p.user, "anna");
        assert!(p.relogin_required);
    }

    #[test]
    fn gpu_access_is_ok_for_a_member_and_for_world_readable_nodes() {
        let f = Fake::new();
        let gid = f.node("dev/dri/renderD128", 0o660);
        f.node("dev/accel/accel0", 0o666);
        f.write("etc/group", &format!("render:x:{gid}:anna\n"));
        let c = check("gpu-access", &f.sys(), &stranger()).unwrap();
        assert_eq!(c.status, Status::Ok, "{:?}", c.details);
        assert!(plan("gpu-access", &f.sys(), &stranger()).is_err());
    }

    #[test]
    fn gpu_access_never_asks_for_a_group_off_the_list() {
        let f = Fake::new();
        let gid = f.node("dev/dri/renderD128", 0o660);
        f.write("etc/group", &format!("sudo:x:{gid}:\n"));
        let c = check("gpu-access", &f.sys(), &stranger()).unwrap();
        assert_eq!(c.status, Status::NeedsAction);
        assert!(!c.can_apply);
        assert!(c.actions.is_empty());
    }

    #[test]
    fn gpu_access_without_nodes_is_not_needed() {
        let f = Fake::new();
        assert_eq!(check("gpu-access", &f.sys(), &stranger()).unwrap().status, Status::NotNeeded);
    }

    #[test]
    fn vaapi_installs_the_vendors_package_once() {
        let f = Fake::new();
        f.gpu("renderD128", "0x8086", "i915");
        f.gpu("renderD129", "0x1002", "amdgpu");
        f.gpu("renderD130", "0x1002", "amdgpu");
        f.write("var/lib/dpkg/status", "Package: intel-media-va-driver\nStatus: install ok installed\n");
        let c = check("vaapi-drivers", &f.sys(), &stranger()).unwrap();
        assert_eq!(c.status, Status::NeedsAction, "{:?}", c.details);
        assert_eq!(c.actions, vec![Action::InstallPackages { packages: vec!["mesa-va-drivers".into()] }]);
    }

    #[test]
    fn vaapi_is_not_needed_on_nvidias_driver() {
        let f = Fake::new();
        f.gpu("renderD128", "0x10de", "nvidia");
        let c = check("vaapi-drivers", &f.sys(), &stranger()).unwrap();
        assert_eq!(c.status, Status::NotNeeded);
        assert!(c.details[0].contains("NVDEC"), "{:?}", c.details);
    }

    #[test]
    fn vaapi_is_ok_when_installed() {
        let f = Fake::new();
        f.gpu("renderD128", "0x1002", "amdgpu");
        f.write("var/lib/dpkg/status", "Package: mesa-va-drivers\nStatus: install ok installed\n");
        f.write("usr/lib/x86_64-linux-gnu/dri/radeonsi_drv_video.so", "");
        let c = check("vaapi-drivers", &f.sys(), &stranger()).unwrap();
        assert_eq!(c.status, Status::Ok);
        assert!(c.details[0].contains("present"));
    }

    #[test]
    fn npu_check_reports_an_unbound_npu_and_never_applies() {
        let f = Fake::new();
        f.write("sys/bus/pci/devices/0000:00:0b.0/class", "0x120000\n");
        f.write("sys/bus/pci/devices/0000:00:0b.0/vendor", "0x8086\n");
        let c = check("npu-check", &f.sys(), &stranger()).unwrap();
        assert_eq!(c.status, Status::NeedsAction);
        assert!(!c.can_apply);
        assert!(c.details[0].contains("intel_vpu"), "{:?}", c.details);
        assert!(plan("npu-check", &f.sys(), &stranger()).is_err());
    }

    #[test]
    fn npu_check_without_npu_is_not_needed_and_unknown_components_are_refused() {
        let f = Fake::new();
        assert_eq!(check("npu-check", &f.sys(), &stranger()).unwrap().status, Status::NotNeeded);
        assert!(check("gpu-unlock", &f.sys(), &stranger()).is_none());
        assert!(plan("gpu-unlock", &f.sys(), &stranger()).is_err());
    }
}
