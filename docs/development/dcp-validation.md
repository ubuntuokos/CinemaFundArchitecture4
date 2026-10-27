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
| asdcplib CLI (`asdcp-info`, `asdcp-unwrap`) | package availability on 26.04 not yet verified | BSD-3-Clause (CineCert). If built from source, install it into its own prefix and set `CFA4_ASDCPLIB_PREFIX`; without it the engine's deep MXF and audio checks report missing tools |
| fd (optional) | `sudo apt install fd-find` | faster discovery; the engine has a Ruby fallback |

### Isolation rules for the adapter (DCP-04)

- Start the engine with an argument array (no shell), its own process group, a time limit, and a
  scratch working directory, `HOME` and `TMPDIR`; read the result from `--dump-result`.
- The DCP is only read. Reports, logs and temporary files go outside the DCP directory.
- Never run it with elevated privileges. No network access is needed: schemas resolve through the
  engine's local XML catalogue.
- The Qt GUI never talks to Ruby: it asks the Rust engine, which runs the backend.

## Test data

No DCP media is committed (`AGENTS.md`). Fixtures are generated into `target/fixtures/` from
synthetic XML and the existing J2K / PCM MXF writers, or fetched pinned by commit and sha256.
