# DCP validation – development dependencies

Plan: [CFA4-DCP-VALIDATION-001](../plans/DCP-VALIDATION-2026-10-27.md). Audits:
[DCP-00 capability audit](../audit/DCP-00-CAPABILITY-AUDIT-2026-10-27.md),
[DCP-01 source and licence audit](../audit/DCP-01-SOURCE-LICENSE-AUDIT-2026-10-27.md).

DCP validation has two engines that must give the same answers on the same packages:

| Engine | What it is | Needed for |
|---|---|---|
| **Native** (planned crates `cfa4-dcp`, `cfa4-dcp-validate`) | CFA4's own Rust code, written from the SMPTE / Interop specifications | the product; works offline and needs nothing outside the Rust workspace |
| **Reference** (dcp_inspect, `wolfgangw/backports`) | an external Ruby program run as a separate process (planned adapter `cfa4-dcp-inspect`) | development, parity tests (DCP-17) and an optional backend |

The reference engine is **never vendored** into this repository and never touches the system:
it is fetched at one audited commit into a CFA4-owned cache, with its own gem directory.

## Native engine

Nothing beyond the normal Rust setup ([linux.md](linux.md)). The Rust dependencies proposed in
DCP-00 §6.5 (`sha1`, later `rsa` and `x509-cert`) are added to `Cargo.toml` by the PR that adds the
crate using them, and must pass `cargo deny check licenses advisories`.

## Reference engine (dcp_inspect)

~~~bash
bash scripts/dev/setup-dcp-inspect.sh            # read-only check (default)
bash scripts/dev/setup-dcp-inspect.sh --install  # pinned checkout + pinned gems, no sudo
bash scripts/dev/setup-dcp-inspect.sh --verify   # the engine's own tests and XSD check
eval "$(bash scripts/dev/setup-dcp-inspect.sh --print-env)"
"$CFA4_DCP_INSPECT" --dump-result /tmp/dcp-result.json /path/to/DCP
~~~

What `--install` does, and nothing more:

- fetches `wolfgangw/backports` at `99d0fd863fe05eb806f1625233f2e93676ead268` (dcp_inspect
  1.2026.10.03) into `${CFA4_DCP_INSPECT_DIR:-~/.cache/cfa4/dcp-inspect}/<commit>/src`, refuses any
  other commit or a modified checkout, and checks `xsd/MANIFEST.sha256`;
- installs exactly `racc 1.8.1`, `nokogiri 1.19.4`, `bigdecimal 3.3.1`, `ttfunk 1.8.0` (and
  `base64 0.2.0` on Ruby ≥ 3.4) into `<commit>/gems` with `--ignore-dependencies`;
- writes the wrapper `<commit>/bin/cfa4-dcp-inspect`, which clears `RUBYOPT`, `RUBYLIB` and
  `BUNDLE_GEMFILE`, uses only that gem directory, and puts the pinned FFmpeg 9.0.2 oracle and an
  optional asdcplib prefix first on its own `PATH`.

It never runs `apt`, `sudo` or a global `gem install`, and never changes the user's Ruby, gem or
FFmpeg setup. Remove the cache directory to uninstall.

### System prerequisites (installed by you, not by the script)

| Tool | Ubuntu 26.04 | Notes |
|---|---|---|
| Ruby ≥ 3.0 and headers | `sudo apt install ruby ruby-dev build-essential` | `racc` and `bigdecimal` build native extensions |
| git, sha256sum, mkfifo | base system | |
| FFmpeg 9.0.2 | `bash scripts/ci/install-ffmpeg-9.0.2.sh` | the existing pinned oracle in `~/.cache/cfa4/`; the system FFmpeg stays untouched |
| asdcplib CLI (`asdcp-info`, `asdcp-unwrap`) | `cargo xtask cinema-oracles --install asdcplib` builds the pinned 2.13.2 into its own prefix ([cinema-oracles.md](cinema-oracles.md)); `--print-env` gives `CFA4_ASDCPLIB_PREFIX` | BSD-3-Clause (CineCert). Without it the engine's deep MXF and audio checks report missing tools |
| fd (optional) | `sudo apt install fd-find` | faster discovery; the engine has a Ruby fallback |

### Isolation rules for the adapter (DCP-04)

- Start the engine with an argument array (no shell), its own process group, a time limit, and a
  scratch working directory, `HOME` and `TMPDIR`; read the result from `--dump-result`.
- The DCP is only read. Reports, logs and temporary files go outside the DCP directory.
- Never run it with elevated privileges. No network access is needed: schemas resolve through the
  engine's local XML catalogue.
- The Qt GUI never talks to Ruby: it asks the Rust engine, which runs the backend.

### Parity of the two engines (DCP-17)

`cargo test -p cfa4-dcp-inspect --test parity -- --nocapture` runs structural cases (valid package, asset map missing or not XML, packing list missing or not XML, composition not XML, track
file missing, changed byte, truncated file) through the native backend and through dcp_inspect with
`full-integrity`, and writes `target/dcp-parity/report.md`: per case the two verdicts and the rules only one
engine failed. It is a measurement: only the valid package is asserted (neither engine may fail it); a case
where one engine fails and the other does not is a divergence to read, not a test failure. Without the engine
(`DcpInspect::locate()` finds nothing) it says so and passes. The media cases (picture, sound) are not in it
yet: they need the engine's asdcplib ([cinema-oracles.md](cinema-oracles.md)).

## Commands and the Qt window (DCP-12)

The engine runs the native backend as a background job; the Qt6 GUI and the control channel drive it
through these commands (none writes into the package):

| Command | What it does |
|---|---|
| `dcp.validate.profiles` | the built-in profiles with their rules (required / optional) |
| `dcp.validate` `{root, profile?, wait?}` | validates the DCP folder `root` (default profile `quick-scan`, or a `.json` profile file) as a job followed in `jobs.list`; `wait` returns the report |
| `dcp.validate.report` `{root}` | the latest report of that package (`cfa4-dcp-report/1`) and its Markdown |
| `dcp.validate.save` `{root, path, format?}` | writes the report as `json`, `md` or `html` (self-contained page, no scripts, every value escaped; the format follows the file extension when not given); refuses a path inside the package |

The window is **File ▸ Validate DCP…** (`python/cfa4_qt/panels/dcp_validation.py`): folder field, profile
selector, Validate / Cancel, the verdict with its counts, the findings (worst first, with rule id,
location, evidence and how to fix it), the profile rules with their result so Unchecked and Skipped rules
stay visible, the Markdown report, and Save JSON / Markdown. The package browser, tree and the
audio / video / subtitle / security detail pages of the plan's DCP-F19 follow in later steps.

## Export integration (DCP-13)

Plan DCP-F20. A DCP export goes through the export queue (`export.queue.add`, the Export window's Queue):

- **Preflight**, before anything is queued: `dcp.preflight` (the parameters of `export.resolve`, the format
  defaults to `dcp`) and every `export.queue.add` of a DCP return findings `DCP.PRE.nnn` with a severity
  (`error`, `warning`, `info`) and how to fix it. An `error` refuses the whole add: invalid DCP settings, an
  empty range, and a target folder that is not empty (the exporter would mix the new track files into the
  package that is there, so an earlier valid export is never touched). Warnings: the frame rate is converted
  to a DCI rate, no language, no sound. Info: picture scaled with black bars, shortened ISDCF title, and
  that the package is unencrypted and unsigned. A format other than DCP gets one `DCP.PRE.000` info and no
  validation.
- **Postflight**, when the package is written: `dcpValidate` on `export.queue.add` names the profile (a
  built-in id or a `.json` profile; default for a DCP `delivery-preflight`; `false` switches it off). The
  result is the queue item's `dcpValidation` (`status` `notChecked`, `running`, `passed`, `failed`,
  `incomplete`; `profile`, `report`, `error`), kept apart from the render `status` as `qc` is. The report
  (`cfa4-dcp-report/1`) is written next to the package as `<package>.cfa4dcp.json`, never inside it, and
  is also the latest report of `dcp.validate.report`. A failed package keeps its files; fix the cause,
  then `export.queue.retry` validates the new render. Qt6: the queue's Quality column shows the DCP result.

Not covered: a direct `file.exportMedia` of a DCP is not validated (queue only); validation does not say
that CFA4 writes complete DCPs (encryption, signing and KDMs are other projects).

## Repair proposals (DCP-14)

`dcp.repair.plan {root}` turns the latest report of a package into `cfa4-dcp-repair/1`: every failure or
warning gets one of five classes (plan DCP-F21) and an action, the finding's own remedy when it has one.
`projectMetadata` (title, language, container, markers: fix in the project, export again), `reexport`
(the content is wrong: export again, or ask the supplier of an imported package), `userDecision`
(loudness, subtitle timing), `externalTool` (certificates, the reference engine) and `unsupported` (the rule
did not run, so there is nothing to repair and the result is unknown). Failures come first. The plan only
proposes: `modifiesPackage` is always `false`, and the original package is never changed. The classes are
in `crates/dcp-validate/src/repair.rs`; applying a repair to a project or an approved working copy is not
part of this step.

## Test data

No DCP media is committed (`AGENTS.md`). Fixtures are generated into `target/fixtures/` from
synthetic XML and the existing J2K / PCM MXF writers, or fetched pinned by commit and sha256.
