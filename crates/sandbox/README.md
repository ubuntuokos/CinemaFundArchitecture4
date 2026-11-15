# cfa4-sandbox

CFA4's run-time security layer ([CFA4-SECURITY-001](../../docs/plans/CFA4-SECURITY-001.md), phase B1).
This phase adds the sandbox API and the profile of every CFA4 process kind, so that a process can
confine itself at start-up and a bug in a parser, decoder, plug-in or browser stays inside that one
process. No process applies it yet: the engine confines itself in phase B3, and the separate media
host that takes FFmpeg and GStreamer out of the engine comes in phase B4; until then they run
in-process, unconfined.

- **Landlock** (unprivileged, in the kernel): which files and directories the process may read or
  write, which programs it may start, which TCP ports it may bind or connect (ABI ≥ 4), and that it
  cannot signal processes outside its domain (ABI ≥ 6).
- **seccomp-bpf**: a deny list (`EPERM`) of system calls no CFA4 process needs (`ptrace`,
  `process_vm_*`, kernel modules, mounts, namespaces, keyrings, clocks, `io_uring`, `bpf`), plus
  `execve` where the profile starts no program and IPv4/IPv6/packet sockets where it has no network.

Both go through their safe Rust crates (`landlock` 0.4.7, MIT OR Apache-2.0; `seccompiler` 0.5.0,
Apache-2.0 OR BSD-3-Clause); this crate has no `unsafe`. Layer L1: it depends on no CFA4 crate.

## Process kinds (plan IV.2)

| Kind | Reads (besides the system) | Writes | Network | Starts programs |
|---|---|---|---|---|
| `qtGui` | app dirs, project, fonts | settings, runtime dir | loopback TCP ports | the engine |
| `engine` | added media, fonts | project, cache, settings, the export files (only the files) | loopback TCP ports | its helpers |
| `mediaHost` | the one opened file | nothing | none | none |
| `studioHost` | scene sources, fonts | the recording file | any | its Chromium helper |
| `browserSource` | – | – | any | confined by Chromium's sandbox + AppArmor (B6), not here |
| `fetch` | – | download target | connect to 443 (and UDP for DNS) | none |
| `pluginHost` | plug-in files | nothing | none | none |
| `modelConverter` | source model | output file | none | none |

"The system" is `SYSTEM_READ` (`/usr`, `/lib*`, `/etc`, `/opt/cfa4`, `/sys`, this process's own
`/proc` entries, a few `/proc` summaries, the fontconfig cache), never `$HOME` or other processes'
`/proc`. GPU processes also get `/dev/dri` and the NVIDIA nodes with device ioctls.

## Use

```rust
use cfa4_sandbox::{Context, Mode, ProcessKind, Profile, apply};

let ctx = Context { media_file: Some(path.into()), ..Context::default() };
match apply(&Profile::for_process(ProcessKind::MediaHost, &ctx), Mode::Strict) {
    Ok(enforced) => log::info!("confined: {enforced:?}"),
    // D4: do not open the file; tell the user what is missing (Settings ▸ Security)
    Err(e) => return Err(e.into()),
}
```

Call `apply` once, before starting threads and before reading untrusted input. `Mode::Strict` is the
default (plan D4): a missing protection is an error and the risky part does not start.
`Mode::Unprotected` is the user's explicit opt-in, and `status()` then marks the process unsupported.

`status(&Context)` is the JSON behind the engine's `security.status` command: kernel support
(`landlockAbi`, `seccomp`, `apparmor`, `usernsRestricted`), each kind's profile with `canStart` and
`missing`, and this process's own confinement.

## Tests

`tests/confinement.rs` is the B1 acceptance test: each case re-runs the test binary as a child,
confines it, and checks that `~/.ssh` and the rest of a fake home are unreadable, no program starts,
no IPv4/IPv6 socket opens and only the granted ports bind, while the granted files stay usable. On a
kernel without Landlock or seccomp the child prints `SKIP` with the reason.

## Next (plan V.)

B3 confines the engine itself (`cfa4-cli serve`) with the file chooser granting access through the
portal; B4 starts `cfa4-media-host`; B6 adds the studio host and its AppArmor profile; B8 checks the
profiles against the packaged application (fonts, GPU drivers, PipeWire).
