# cfa4-system-helper

The only program CFA4 runs as root, and only through `pkexec` after the user approved the plan in
CFA4's window ([ADR 0017](../../docs/adr/ADR-0017-rendszerszintu-komponensek.md),
[CFA4-SETTINGS-HW-003](../../docs/plans/CFA4-SETTINGS-HW-003.md) R1). The `.deb` installs it as
`/usr/libexec/cfa4/cfa4-system-helper` with the polkit rule
`packaging/linux/io.github.ubuntuokos.cfa4.system-helper.policy` (`auth_admin`: the password is asked
for every time). The tarball, the AppImage and the Flatpak do not carry it.

## Verbs

| Verb | Root | What it does |
|---|---|---|
| `status` | no | every component's check and the state record |
| `check [--component ID]` | no | what is missing, in plain facts |
| `plan --component ID` | no | the plan (`actions`, `user`, `relogin_required`) and its SHA-256 `digest` |
| `apply --component ID --digest HEX [--plan FILE\|-]` | yes | rebuilds the plan, runs it only if the digest matches, records it; a failed step rolls back the earlier ones |
| `disable --component ID` | yes | undoes, newest first, what the helper did for the component (and nothing else) |
| `uninstall --component ID` | yes | `disable`, then forgets the component |

Common options: `--root DIR` (a fake tree: every read and write goes beneath it, and only `--dry-run`
may change anything), `--uid N` (with `--root` only), `--dry-run` (prints the commands instead of
running them). Output is one JSON object (`{"ok": true, …}` or `{"ok": false, "error": {"code",
"message"}}`); exit status 0, 1 (refused or failed) or 2 (usage).

## Components

| Component | Check | Apply | Undo |
|---|---|---|---|
| `gpu-access` | can the user open every `/dev/dri/renderD*` and `/dev/accel/accel*` by its permission bits; which group (`render`, `video`) grants it | `gpasswd -a USER GROUP`; log out and back in afterwards | `gpasswd -d` |
| `vaapi-drivers` | the VA-API package of each GPU driver (Intel `i915`/`xe`: `intel-media-va-driver`; `amdgpu`, `radeon`, `nouveau`: `mesa-va-drivers`); NVIDIA's own driver: not needed (NVDEC) | `apt-get install -y --no-install-recommends` | `apt-get remove -y` |
| `npu-check` | NPU driver (`intel_vpu`, `amdxdna`) and firmware; unclaimed Intel/AMD PCI accelerators | none: check and guidance only | – |

The user comes from `PKEXEC_UID` when run as root, never from the caller; programs run by absolute
path with a cleared environment. Groups and packages outside the lists above are refused, also in a
forged plan. The access check reads permission bits only (not ACLs), so the plan is the same when the
user and root compute it.

## Tests

`cargo test -p cfa4-system-helper` runs everything on fake roots with a recording runner; no real
`gpasswd` or `apt-get` runs. A real `apply` and undo on the reference machine needs the owner's
explicit, on-the-spot confirmation (ADR 0017, Következmények).
