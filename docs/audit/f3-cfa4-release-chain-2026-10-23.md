# CFA4 F3 — independent release-candidate chain (2026-10-23)

## Boundary and current state

CFA4 repository only: `ubuntuokos/CinemaFundArchitecture4`. The upstream
project is the parent and is neither modified nor used as an artifact
publisher, version override or AppImage update endpoint.

**RELEASE: HOLD**. The new workflow never publishes a GitHub Release. The
`four-formats` workflow job is opt-in and requires a separately provisioned
trusted Ubuntu 26.04 AMD64 self-hosted runner with reviewed packaging tools.
The runner and end-to-end package installation have **not been validated
by this source change**. Packages must remain on the user's controlled host;
the workflow does not upload unapproved binaries or create public releases.

- Version is read from CFA4 `Cargo.toml` exactly once and may not be
  overridden by `CFA4_VERSION`.
- The package maintainer metadata uses `CFA4_MAINTAINER`; binary-internal
  `CFA4_BUILD_*` variables remain solely for inherited Rust engine
  compatibility and are assigned CFA4-origin values.
- AppImage update information is restricted to CFA4 GitHub Releases and
  rejects a conflicting `GITHUB_REPOSITORY`.
- Packaging output IDs/names are `cfa4` and
  `io.github.ubuntuokos.cfa4`; `.deb`, AppImage, Flatpak bundle and
  `tar.gz` share the `Cargo.toml` version.
- `scripts/release/build-cfa4-four-formats.sh` runs local-only builders,
  rejects missing prereqs, verifies four nonempty outputs and writes a
  `SHA256SUMS` inventory, without apt/system installs.

## Evidence gates still open

1. Full #23 native media + Cargo integration CI at the combined merge commit.
2. `four-formats` manual runner setup and real four-package build.
3. Clean installation/uninstallation on Ubuntu 26.04, KDE primary and
   GNOME/Cinnamon XDG smoke.
4. Runtime dependency inventory, attribution/SBOM, brand and license approval.
5. Standalone AV1 NVDEC/Qt6 editor feature claims remain disabled unless tested.

No user hardware media/GUI evidence is repeated in this change.
