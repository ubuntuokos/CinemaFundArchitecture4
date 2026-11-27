# DCP / IMF test oracles

> **Last updated:** 2026-11-15 · **Change:** new (CINECERT WP01-A)

Plan: [CFA4-PLAN-CINECERT-001](../plans/CFA4-CINECERT-001-v2.3.md), chapter 13 and WP01-A; status:
[CINECERT-WP00 gap matrix](../audit/CINECERT-WP00-GAP-MATRIX-2026-11-13.md). Decision D1 = A: these
programs are **test oracles only**. They run as separate processes in development and in tests, are
never linked into CFA4, never ship in a CFA4 package, and no code is taken from them.

| Oracle | Version | Licence | What it checks | Variable the tests read |
|---|---|---|---|---|
| asdcplib CLI (`asdcp-info`, `asdcp-unwrap`, `as-02-info`, …) | 2.13.2 (`rel_2_13_2`, commit `d53aac8`) | BSD-3-Clause (CineCert) | MXF track files of DCPs (AS-DCP) and IMF (AS-02) | `CFA4_ASDCPLIB_PREFIX` (also used by [`dcp_inspect`](dcp-validation.md)) |
| Netflix Photon | 5.0.1 (Maven Central `com.netflix.photon:Photon`) | Apache-2.0 | IMF packages (App 2 / 2E): CPL, PKL, asset map, essence descriptors | `CFA4_PHOTON_CP` (the Java class path; `crates/export/src/imf_tests.rs`) |
| Clairmeta | 1.6.3 (PyPI) | BSD-3-Clause (Ymagis) | DCPs (SMPTE and Interop): structure, hashes, XSD, signatures, reels, MXF metadata; DSM / DCDM sequences. Not IMF. | `CFA4_CLAIRMETA` (a wrapper: `cfa4-clairmeta check -type dcp DIR`) |
| test corpus | [`scripts/dev/cinema-oracles/corpus.txt`](../../scripts/dev/cinema-oracles/corpus.txt) | per entry | external DCP / IMF packages, never committed (`AGENTS.md`) | `CFA4_CINEMA_CORPUS` |

## Install

```sh
cargo xtask cinema-oracles                  # = --check: prerequisites and what is installed (read-only)
cargo xtask cinema-oracles --install        # all three; or --install asdcplib photon clairmeta
cargo xtask cinema-oracles --corpus         # fetch the corpus entries, each checked against its sha256
eval "$(cargo xtask cinema-oracles --print-env 2>/dev/null)"   # export the variables above
```

`cargo xtask cinema-oracles` runs [`scripts/dev/setup-cinema-oracles.sh`](../../scripts/dev/setup-cinema-oracles.sh),
which can also be called directly. Everything goes under `${XDG_CACHE_HOME:-~/.cache}/cfa4/cinema-oracles`
(`CFA4_CINEMA_ORACLES_DIR` moves it); remove that directory to uninstall. The script never runs `apt` or
`sudo` and never changes the system, the user's Python, Java or Maven setup:

- **asdcplib** is built with CMake from the pinned commit into its own prefix; the programs find their
  libraries through RPATH. Without the optional Xerces-C headers it builds without XML parse support,
  which the AS-DCP checks above do not need.
- **Photon** is not built from source (its Gradle build needs Netflix's release plugins): Maven copies
  the released jar and its runtime dependencies from Maven Central into the cache, and the script
  refuses them unless every jar matches [`photon-5.0.1.sha256`](../../scripts/dev/cinema-oracles/photon-5.0.1.sha256).
  `CFA4_MAVEN_CENTRAL` points at a mirror.
- **Clairmeta** goes into its own venv from the hash-pinned
  [`requirements/clairmeta-oracle.txt`](../../requirements/clairmeta-oracle.txt) (`--require-hashes`,
  wheels only). Its wrapper puts the asdcplib prefix first on `PATH`, which Clairmeta needs for its
  MXF checks. `mediainfo` and `sox` are optional for it.

### System prerequisites (installed by you, not by the script)

| Tool | Ubuntu 26.04 | For |
|---|---|---|
| git, CMake, g++, OpenSSL headers | `sudo apt install git cmake g++ libssl-dev` | asdcplib |
| Xerces-C headers (optional) | `sudo apt install libxerces-c-dev` | asdcplib XML support |
| Java ≥ 17, Maven | `sudo apt install openjdk-21-jre-headless maven` | Photon |
| Python venv | `sudo apt install python3-venv` | Clairmeta |
| mediainfo, sox (optional) | `sudo apt install mediainfo sox` | Clairmeta |
| curl | `sudo apt install curl` | corpus |

## In tests

A missing oracle skips its check and says so; it never turns into a pass (plan WP01-A). The
existing hook is the IMF export test: with `CFA4_PHOTON_CP` set, `cargo test -p cfa4-export --lib imf`
runs Photon's `IMPAnalyzer` on the packages it writes. The conformance tests of plan chapter 8 add
the asdcplib and Clairmeta checks and the corpus entries.

## Changing a version

Change the pin in the script and its lock file together:

- asdcplib: `ASDCPLIB_VERSION` and `ASDCPLIB_COMMIT` (the commit the tag points to,
  `git ls-remote https://github.com/cinecert/asdcplib 'refs/tags/rel_X_Y_Z^{}'`);
- Photon: `PHOTON_VERSION` and a new `photon-X.Y.Z.sha256` (`sha256sum *.jar` in the `lib/` the
  install produced, after checking the new dependency set);
- Clairmeta: `CLAIRMETA_VERSION`, `requirements/clairmeta-oracle.in` and the regenerated lock
  (command in its header).
