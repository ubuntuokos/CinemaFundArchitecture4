# DCP validation: release gate (DCP-19)

The plan's §14 acceptance criteria (`docs/plans/DCP-VALIDATION-2026-10-27.md`), each with the evidence in the
repository and an honest status. **Met** means a test or document in the repository shows it; **Partly** says
what is missing; nothing here is claimed without a check you can run.

Run the DCP checks of this gate (offline, CPU only, no GPU, no network, no Ruby):

~~~bash
cargo test --release -p cfa4-dcp-validate -p cfa4-dcp -p cfa4-dcp-inspect
cargo test --release -p cfa4-engine dcp
python3 packaging/linux/test_dcp_validation.py
~~~

| # | Criterion | Evidence | Status |
|---|---|---|---|
| 1 | Own DCP Validation menu item | File ▸ Validate DCP… (`python/cfa4_qt/main_window.py`, DCP-12) | Met |
| 2 | A DCP can be loaded from the Qt6 window | `python/cfa4_qt/panels/dcp_validation.py` (DCP-12) | Met (Qt GUI tests do not block CI yet, `docs/coordination.md` §3.4) |
| 3 | SMPTE and Interop packages | `tests/fixture_matrix.rs` (SMPTE), `src/tests.rs` (Interop) | Met |
| 4 | Structured results | report schema `cfa4-dcp-report/1`, `ValidationReport` | Met |
| 5 | ASSETMAP / PKL / CPL / MXF links | `tests/fixture_matrix.rs` cases (DCP-16) | Met |
| 6 | Audio, picture, subtitle and security checks with documented coverage | `docs/development/dcp-validation.md`, `tests/subtitles.rs`, `tests/signatures.rs` | Partly: IAB is structural only; J2K coverage is reported separately |
| 7 | Unchecked properties never show as a pass | `fixture_matrix.rs::unchecked_properties_never_count_as_a_pass` (all profiles) | Met |
| 8 | Results can be exported | JSON, Markdown, HTML (`dcp.validate.save`, DCP-14) | Met |
| 9 | Checks can be cancelled | `src/tests.rs` (`cancellation_interrupts`, `cancel_while_*`) | Met |
| 10 | Original files stay unchanged | `fixture_matrix.rs::validation_never_changes_the_package_under_any_profile` (every built-in profile; bytes, sizes, mtimes, no new files) | Met |
| 11 | Linux packages handle dependencies | `packaging/linux/test_dcp_validation.py` (DCP-18): engine in .deb / Flatpak / AppImage, no Ruby bundled | Partly: static check; no real package build was run for this step |
| 12 | No global host dependency changes | same test (no `gem install`, `GEM_HOME`); reference engine installs into `~/.cache/cfa4/dcp-inspect` only | Met |
| 13 | CI and regression tests pass | `cargo xtask ci`, the tests above; no dedicated DCP workflow yet | Partly: no separate DCP-validation workflow (plan §13) |
| 14 | Licence compliance documented | `docs/licensing.md`, `docs/development/dcp-validation.md` (dcp_inspect is an external, optional tool) | Partly: no source-level audit of dcp_inspect beyond that |
| 15 | Native vs reference engine differences documented | `crates/dcp-inspect/tests/parity.rs`, parity section of `dcp-validation.md` (DCP-17) | Partly: the real-engine run needs Ubuntu 26.04 and was not run |
| 16 | Profile and standard version traceable | `fixture_matrix.rs::the_report_names_the_profile_version_and_standard_and_survives_json` | Met |

## Not covered yet (plan §12)

An encrypted DCP, a package with several composition or packing lists, a very large DCP, file permission
errors, resource-leak checks and coexistence with host applications. They are listed here so they are not
mistaken for passed.
