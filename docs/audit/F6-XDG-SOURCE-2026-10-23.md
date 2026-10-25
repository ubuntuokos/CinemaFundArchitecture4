# CFA4 F6 — XDG metadata and desktop integration source gate

Target: Ubuntu 26.04 AMD64, KDE Plasma primary, GNOME/Cinnamon XDG
compatibility. This audit is source-only; **no graphical session is claimed**.

The first AppStream description paragraph is expanded to explain the
development editor and the upstream provenance. It is validated with
Python's XML parser and the desktop/MIME/Flatpak identity consistency test
\`scripts/ci/test_cfa4_xdg.py\`, now part of the fast Linux source gate
and the independent release-candidate policy gate.

The existing shared MIME project type \`application/x-cfa4\` and
\`*.fcproj\` extension are retained solely for compatibility with inherited
project files; the visible application, icon and package names remain CFA4.
This is **not** a claim that the complete codebase, CLI text, project file
schema or all inherited trademark assets have passed branding review.

### Still required before release

- KDE Plasma 6/Wayland menu entry, scalable icon, file open, file association
  and launcher behaviour from **installed** .deb/Flatpak/AppImage/tarball.
- Automated GNOME/Cinnamon XDG checks or desktop-level smoke on a target VM.
- AppStream validation against the actual generated metadata and package
  manager integration; no past warning is presumed fixed until retested.
- Approved icon visual/asset review, trademark and full attribution audit.

No previously accepted hand-run media playback, image/audio/scrub or
short export/reimport test is repeated.

**Release remains HOLD.**
