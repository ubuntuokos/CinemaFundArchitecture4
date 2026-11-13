# cfa4-dcp-inspect

DCP validation backend (DCP-04) that runs the dcp_inspect reference engine
(wolfgangw/backports, GPL-3.0-or-later) as a separate process and returns the same
`cfa4-dcp-report/1` report as the native backend of [`cfa4-dcp-validate`](../dcp-validate/README.md).

The engine is not part of CFA4. It is installed outside the repository, at one pinned commit, by
[`scripts/dev/setup-dcp-inspect.sh`](../../scripts/dev/setup-dcp-inspect.sh), with its own gem
directory. There is no Ruby in Rust or in the GUI. Audits:
[DCP-00](../../docs/audit/DCP-00-CAPABILITY-AUDIT-2026-10-27.md) §6.1 and
[DCP-01](../../docs/audit/DCP-01-SOURCE-LICENSE-AUDIT-2026-10-27.md) §4 and §6.

## Use

```rust
use cfa4_dcp_inspect::DcpInspect;
use cfa4_dcp_validate::{DcpValidator, Profile, ValidationControl, ValidationRequest};

if let Some(engine) = DcpInspect::locate() {
    let req = ValidationRequest::new("/path/to/DCP", Profile::builtin("full-integrity").unwrap());
    let report = engine.validate(&req, &ValidationControl::new())?;
}
```

`DcpInspect::locate` finds the wrapper that the install script writes. It looks in this order:

1. `CFA4_DCP_INSPECT`;
2. `CFA4_DCP_INSPECT_DIR/<commit>/bin/cfa4-dcp-inspect`;
3. `${XDG_CACHE_HOME:-~/.cache}/cfa4/dcp-inspect/<commit>/bin/cfa4-dcp-inspect`.

If no wrapper is found, it returns `None`. If the wrapper does not run, `validate` returns
`ValidationError::Unavailable`. The rest of CFA4 works without the engine.

The engine needs `asdcp-info` from asdcplib (BSD-3-Clause, CineCert) and refuses to start without
it (exit code 12, reported as unavailable). Set `CFA4_ASDCPLIB_PREFIX` before `--install` so the
wrapper puts it on the engine's `PATH` (DCP-01, L4).

## Pinned engine

| | |
|---|---|
| Commit | `99d0fd863fe05eb806f1625233f2e93676ead268` (`ENGINE_COMMIT`) |
| Version | 1.2026.10.03 (`ENGINE_VERSION`) |

The `--dump-result` JSON has no schema version. The adapter therefore reads only the form of this
commit and refuses any other form (`ValidationError::Backend`). Moving to a new commit means
updating the install script, these constants and the mapping together.

## How a run works

- The wrapper is started with an argument array, never through a shell. It runs in its own process
  group, with stdin closed.
- The environment is cleared. `PATH` and the locale are passed on. `HOME`, `TMPDIR` and the working
  directory point into a private directory (mode 0700), which is removed afterwards. `TERM=dumb` is
  set because the engine's progress bar needs a terminal size.
- The engine only reads the package. The result, stdout and stderr go to files in the private
  directory, so no pipe can fill up and nothing is written into the package.
- On cancel, `SIGINT` goes to the whole process group (the engine stops with 18). After 5 s,
  `SIGKILL` follows. The report is `INTERRUPTED`.
- On timeout (24 h by default, `with_timeout`), `SIGKILL` goes to the group and `validate` returns
  `ValidationError::Backend`.
- The profile selects engine options. Without `DCP.PKL.004` the run uses `--no-hash`. Without
  `DCP.AUD.002` it uses `--no-audio-analysis`.

## Mapping

The engine attaches checks (kind, status, message) to the asset maps, packing lists, compositions
and assets it read.

| Engine check | Rule |
|---|---|
| asset map `schema` | `DCP.AM.002` |
| packing list `schema`, asset `type` | `DCP.PKL.001` |
| composition `schema` | `DCP.CPL.001` |
| asset `presence` | `DCP.AM.005` |
| asset `hash` | `DCP.PKL.004` |
| `signature` | `DCP.SIG.001` |
| `dci_content_authenticator` | `DCP.SIG.002` |
| `jpeg2000` | `DCP.J2K.001` |
| `iab` | `DCP.IAB.001` |
| `audio_channels` | `DCP.AUD.001` |
| `composition_audio` | `DCP.AUD.002` |
| `subtitles` | `DCP.SUB.001` |
| `naming` | `DCP.NAME.001` |
| `markers` | `DCP.MARK.001` |
| any other check, every engine error and hint | `DCP.REF.001` |

Statuses map as follows:

- `ok` → `PASS`;
- `error` → `FAIL`;
- `hint` or `warning` → `WARNING`;
- `skipped` → `SKIPPED`;
- `info` → `PASS`, except on an unsigned document's signature check, where it is `NOT_APPLICABLE`.

`DCP.AM.001` passes when the engine read an asset map.

A profile rule the engine reported nothing on is `UNCHECKED`, so it never passes by omission.
`DCP.PIC.001` (picture format, DCP-06) has no engine check of its own, so it is always `UNCHECKED`
here; the engine's frame-rate and edit-rate messages land in `DCP.REF.001`.

`DCP.REF.001` keeps whatever the engine said that no rule covers, for example "Reel duration less
than 1 second". A `FAIL` there fails the package even if every profile rule passes. Absolute paths
under the package root are made relative in every message.

## Tests

- 8 unit tests drive a fake engine (a shell script):
  - mapping onto the rules and the verdict;
  - package-relative paths;
  - the engine options each profile selects;
  - the private, cleared environment and the removal of the private directory;
  - every exit code;
  - malformed result JSON;
  - cancel, which stops a background child in the process group too;
  - timeout.
- `tests/real_engine.rs` writes a DCP with `cfa4-export` and inspects it with the real engine under
  Full Integrity. The asset map, PKL, CPL and hash rules must pass, no absolute path may appear in
  the report, and the package must be unchanged afterwards. Where the engine is not installed, the
  test says so and passes.

## Engine self-test (DCP-01 §8)

`setup-dcp-inspect.sh --verify` runs the engine's own `rake xsd:check` and `rake test`. Those need
rake and minitest, which the isolated gem directory did not have. The script now installs them,
pinned (rake 13.1.0, minitest 5.20.0), into a separate `test-gems` directory, so the engine's
runtime gem set does not change.

Result on 2026-11-11 (Ruby 3.3.6, asdcplib 2.13.3 built from source without Xerces):

- `xsd:check` passed: the XSD store matches `MANIFEST.sha256`.
- `rake test` ran 185 tests: 1 failure and 4 skips. The failure
  (`test_native_smpte_extraction_resources_identity_and_schema`) needs asdcplib built with its XML
  parser (Xerces-C) to unwrap SMPTE timed text. It is a build option of asdcplib, not an engine
  fault.
