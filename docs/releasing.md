# Releasing CFA4

Every push to the `release` branch runs `.github/workflows/release.yml`. The workflow builds
installers for macOS, Windows, Linux (AppImage, deb, rpm, tarball, Flatpak) and FreeBSD, plus
the web build, and creates or updates a **draft**
GitHub Release named `CFA4 v<version>`. Nobody sees a draft until a maintainer publishes it.

User-facing names say **CFA4**. Files, binaries and ids stay lowercase
(`cfa4-<version>-<platform>-<arch>.<ext>`, `io.github.ubuntuokos.cfa4`).

## Cutting a release

1. **Bump the version** on `main`. It lives in exactly one place, `[workspace.package] version`
   in the root `Cargo.toml`; every crate inherits it and the packaging scripts read it from there:

   ```sh
   cargo xtask version                 # prints the current version, e.g. 0.2.1
   cargo xtask version set 0.3.0       # or 0.3.0-rc.1; updates Cargo.toml and Cargo.lock
   ```

   Commit the change (`Cargo.toml` + `Cargo.lock`) through the normal review flow, as a
   `Release: CFA4 v0.3.0` commit whose message says what changed for users.
2. **Merge `main` into `release`** (or fast-forward it) and push. The workflow starts by itself.
3. **Wait for the draft.** When every job is done (macOS notarization is the slow part), the
   Releases page has a draft `CFA4 v0.3.0`, targeting the pushed commit, with every artifact
   and `SHA256SUMS.txt`. The notes are generated from the merged pull requests.
4. **Check it.** Download an installer or two and read the job summaries. A `::warning::` there
   means a signing secret was missing and that artifact is unsigned.
5. **Publish** the draft in the GitHub UI. Publishing creates the `v0.3.0` tag. A version with a
   pre-release suffix (`-rc.1`) is marked as a pre-release.

Pushing to `release` again before you publish rebuilds the same draft and replaces its assets.
Once the draft is published, the workflow refuses to touch that version again: bump it first.

**Test runs:** *Actions › Release › Run workflow* runs the whole pipeline by hand. The optional
`version` input (such as `0.3.0-rc.1`) overrides `Cargo.toml` for that run only; each build job
applies it with `cargo xtask version set` before building. The signing jobs (macOS, Windows)
and the draft-release job run in the `release` environment, which only the `release` branch can
use, so pick that branch for a full run. Run from any other branch, it is a dry run: the jobs that
sign nothing (Linux, Flatpak, FreeBSD, web) build and upload their artifacts to the run, the
signing jobs are refused, and the draft-release job, which needs them, is skipped.

## What gets built

| Platform | Artifacts | Built on |
|---|---|---|
| Linux x86_64 | `cfa4-<v>-linux-x86_64.{AppImage,deb,rpm,tar.gz}` | `ubuntu-22.04` |
| Linux aarch64 | `cfa4-<v>-linux-aarch64.{AppImage,deb,rpm,tar.gz}` | `ubuntu-22.04-arm` |
| Linux riscv64 | `cfa4-<v>-linux-riscv64.tar.gz` | `ubuntu-24.04`, cross-compiled (glibc 2.39+; CLI smoke-tested under QEMU) |
| AppImage updates | `cfa4-<v>-linux-{x86_64,aarch64}.AppImage.zsync` | with the AppImage |
| Flatpak | `cfa4-<v>-linux-x86_64.flatpak`, `cfa4-<v>-linux-aarch64.flatpak` | `ubuntu-24.04`, `ubuntu-24.04-arm` (repackaged Linux tarball) |

`cfa4 --version` and `cfa4-cli --version` print the version from `Cargo.toml`.

**Fonts.** Every build job checks out the external font directory at a
pinned commit (the `ref:` of its extra-fonts checkout step, marked `# bump deliberately`) and builds
with `CFA4_FONTS_DIR` and `CFA4_FONTS_REQUIRED=1`, so releases embed its Japanese fonts and fail
rather than ship without them (the web build embeds only the UI font, to stay small). The packages
carry each embedded font's licence (`copy_font_licences` in `packaging/env.sh`). Bump the pins
deliberately, in every job at once.

### Linux

`packaging/linux/package.sh` builds the release binaries and packages them as an AppImage, a
`.deb` and an `.rpm` (with [nfpm](https://nfpm.goreleaser.com), from `nfpm.yaml`) and a plain
`.tar.gz` tree. The packages install both programs, the desktop entry, the AppStream metainfo, the
icons and the licence files.

The Linux tarball includes `install.sh`. Extract it, open a terminal in the extracted
directory and run `./install.sh` to install both programs and their desktop integration
under `~/.local` (no administrator permissions needed). The application-menu entry uses
the installed binary's absolute path; add `~/.local/bin` to `PATH` for terminal commands.
Run the installer again to update. For a system-wide installation, use
`sudo ./install.sh --prefix /usr/local`. A custom absolute `--prefix` is also supported;
its `share` directory must be in the desktop's data search path to appear in the menu.
Prefixes containing `=`, `%` or control characters are rejected for desktop-launcher compatibility.

The jobs run on `ubuntu-26.04`, so the binaries need glibc 2.43 or newer (measured with
`scripts/dev/audit-runtime-deps.sh`, see `docs/audit/F2-01-RUNTIME-DEPS-2026-10-25.md`). The deb
declares `libc6 (>= 2.43)`, so older systems refuse the install. Moving the job to a newer image
raises the floor, so re-run the measurement and update `nfpm.yaml` with it.
After packaging, the job prints the `.deb`'s metadata and contents, runs `ldd` on the binary and
runs each AppImage with `--version`.

**AppImage updates.** Each AppImage embeds update information,
`gh-releases-zsync|ubuntuokos|CinemaFundArchitecture4|latest|cfa4-*-linux-<arch>.AppImage.zsync`, and
`appimagetool` writes the matching `.zsync` beside it (the job installs `zsync` for
`zsyncmake`). [AppImageUpdate](https://github.com/AppImageCommunity/AppImageUpdate) and
AppImageLauncher then fetch only the changed blocks from the newest published (non-pre-release)
release. The job checks both with `--appimage-updateinformation` and `test -s`.

**Flatpak.** The `flatpak` jobs (x86_64 on `ubuntu-24.04`, aarch64 on `ubuntu-24.04-arm`) run
`packaging/linux/flatpak-bundle.sh` on the Linux job's tarball: it installs the binaries with
`packaging/linux/flatpak/io.github.ubuntuokos.cfa4.bundle.yml` into a single-file bundle,
`cfa4-<v>-linux-<arch>.flatpak` (no Rust build, so it takes a minute), then installs it and
runs `cfa4-cli --version` in the sandbox. Users install it with
`flatpak install --user <file>`; the freedesktop runtime comes from Flathub.
`packaging/linux/flatpak/io.github.ubuntuokos.cfa4.yml` is the from-source manifest for a Flathub
submission (its header says how to build it). Both manifests must keep the same runtime and
`finish-args`, and install the same programs: the app and `cfa4-cli`, which runs the
headless commands and the MCP server in the sandbox
(`flatpak run --command=cfa4-cli io.github.ubuntuokos.cfa4 mcp`). packaging-lint checks this.

## The draft release

The last job waits for every build, downloads their artifacts, writes `SHA256SUMS.txt` and creates
the draft `CFA4 v<version>` with notes generated from the merged pull requests. If the draft
already exists, it replaces its assets and keeps it a draft. If that version is already published,
the job fails and asks for a version bump (`cargo xtask version set`).

## Secrets

The macOS, Windows and draft-release jobs run in the `release` environment, which only the
`release` branch can use and which holds the signing secrets. The other jobs sign nothing and get
no secrets. Every secret is optional: a missing one produces unsigned artifacts and a
warning, never a failed build.

| Secret | Used for |
|---|---|
| `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD` | the Developer ID certificate (base64 `.p12`), imported into a temporary keychain |
| `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | notarization (`APPLE_PASSWORD` is an app-specific password) |
| `WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD` | Windows signing with a `.pfx` |
| `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_SIGNING_ENDPOINT`, `AZURE_SIGNING_ACCOUNT`, `AZURE_CERT_PROFILE` | Windows signing with Azure Trusted Signing |
