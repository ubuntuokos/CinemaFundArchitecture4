# CFA4 F5 — Isolated install staging and runtime audit

**Scope:** Ubuntu 26.04 and official flavours, x86_64/amd64; CFA4 only.
The upstream parent and the published release infrastructure are unchanged.

## Prerequisites and invocation

This test requires **all four real** CFA4 candidate artifacts from the same
version in \`dist/release/\`, plus a matching \`SHA256SUMS\`.
It does **not** download packages, install apt dependencies, modify a system
Flatpak installation, or access a user's existing media projects.

\`\`\`sh
python3 scripts/release/f5_staged_runtime.py \
  --dist dist/release \
  --report target/audit/f5-package-runtime.json
\`\`\`

This performs the existing strict four-artifact F4 integrity audit **first**,
then uses a disposable directory and a private HOME/XDG profile.

| Format | Actual F5 staged check | Not proven |
| --- | --- | --- |
| tar.gz | Extract; run packaged \`install.sh --prefix\` into disposable path; test \`cfa4-cli --version\`; record \`ldd\` for editor and CLI | Installed KDE/GNOME/Cinnamon desktop GUI and A/V path |
| .deb | \`dpkg-deb --extract\` to disposable FHS root; test CLI and \`ldd\` | \`dpkg -i\`, maintainer hooks, resolver, system removal |
| AppImage | \`--appimage-extract\`; verify AppRun and run extracted CLI; record \`ldd\` | Real AppRun/GUI launch and FUSE variants |
| Flatpak | Import bundle into private temporary OSTree repo; inspect exact \`app/io.github.ubuntuokos.cfa4/x86_64/*\` ref | Install, Freedesktop runtime compatibility, sandbox CLI/GUI and removal |

The JSON result intentionally reports \`PASS_STAGING_ONLY\`, **never**
\`PASS_INSTALL\`. It only appears after successful checks of actual binaries.
Missing artifacts, libraries, incorrect IDs and command failures are fatal.

## Clean VM acceptance gate (still OPEN)

On a disposable, clean Ubuntu 26.04 amd64 installation with KDE Plasma
and separately on GNOME/Cinnamon compatibility targets:

1. Confirm the four SHA-256 digests and F4/F5 test results.
2. Install the \`.deb\` and confirm its dependencies with the Ubuntu package
   database; launch \`cfa4-cli --version\`, verify desktop integration and
   remove the package. Record actual exit statuses and dependency list.
3. Install Flatpak with its runtime available; check the app ref, sandbox
   command \`flatpak run --command=cfa4-cli io.github.ubuntuokos.cfa4 --version\`,
   XDG integration, and remove the package. Do not silently fetch a runtime
   or override the sandbox's security restrictions.
4. Check the AppImage actual AppRun in a target desktop, CLI payload, icon,
   MIME entry and clean removal.
5. For tarball, use a **dedicated versioned prefix** (not shared \`~/.local\`),
   check CLI/XDG and remove only the owned prefix. Do not use \`rm -rf ~/.local\`.
6. Capture \`ldd\`/loaded graphics/audio/HEIF plugin dependencies. Direct
   ELF \`NEEDED\` alone does not prove runtime \`dlopen\` dependencies.
7. Require PASS evidence for all four formats and target desktops before
   advancing the release gate.

The earlier verified user GUI/audio/manual-edit test is accepted and is not
repeated by this staging script. AV1 NVDEC, complete Qt6 editor and license
review remain independent unresolved gates.

**Release status: HOLD. No public package or binary artifact publication.**
