#!/usr/bin/env bash
# Repackage a CFA4 Ubuntu 26.04 Linux tarball as a single-file Flatpak bundle:
#
#   $DIST/cfa4-<version>-linux-x86_64.flatpak
#
# Usage: packaging/linux/flatpak-bundle.sh [--no-test|--test] [TARBALL]
#
# TARBALL defaults to $DIST/cfa4-<version>-linux-x86_64.tar.gz from package.sh.
# Only Ubuntu 26.04 x86_64 (AMD64) build hosts are supported.
# Needs preinstalled flatpak, flatpak-builder, OSTree, Platform+SDK 25.08, and CFA4_PYSIDE6_WHEELS
# (the pinned PySide6 wheels the Qt GUI runs on inside the sandbox; stage-pyside6.py) and
# CFA4_FLATPAK_SOURCES (a local directory with the manifest's downloaded sources, the krb5 tarball,
# SHA-256-checked and seeded into the builder's cache by flatpak-sources.py).
# No remote-add, dependency download or automatic app installation.
# --test requires CFA4_ALLOW_FLATPAK_INSTALL_TEST=1, and uses --no-pull.
# Manifest: packaging/linux/flatpak/io.github.ubuntuokos.cfa4.bundle.yml.
set -euo pipefail
if [[ "${CFA4_ALLOW_LOCAL_PACKAGING:-0}" != "1" ]]; then
  echo "CFA4 Flatpak packaging is blocked pending trademark / application review." >&2
  exit 3
fi
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/linux"
APP_ID=io.github.ubuntuokos.cfa4
FLATHUB=https://dl.flathub.org/repo/flathub.flatpakrepo

TEST=0
TARBALL=""
while [ $# -gt 0 ]; do
  case "$1" in
    --no-test) TEST=0; shift ;;
    --test) TEST=1; shift ;;
    -h | --help) sed -n '2,16p' "$0"; exit 0 ;;
    -*) echo "unknown argument: $1" >&2; exit 2 ;;
    *) TARBALL="$1"; shift ;;
  esac
done

if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
  echo "CFA4 Flatpak bundles support Ubuntu 26.04 Linux x86_64 (AMD64) only." >&2
  exit 2
fi
. "$ROOT/scripts/ci/require-ubuntu-2604.sh"
ARCH=x86_64
BASENAME="cfa4-$VERSION-linux-$ARCH"
TARBALL="${TARBALL:-$DIST/$BASENAME.tar.gz}"
[ -f "$TARBALL" ] || { echo "error: $TARBALL not found (run packaging/linux/package.sh --formats tar first)" >&2; exit 1; }
bash "$ROOT/scripts/release/check-cfa4-package-prereqs.sh" --formats flatpak
if [[ "$TEST" == 1 && "${CFA4_ALLOW_FLATPAK_INSTALL_TEST:-0}" != "1" ]]; then
  echo "error: --test requires CFA4_ALLOW_FLATPAK_INSTALL_TEST=1 (modifies the user Flatpak installation)" >&2
  exit 3
fi

echo "==> CFA4 $VERSION Flatpak bundle for $ARCH from $(basename "$TARBALL")"

WORK="$CARGO_TARGET_DIR/flatpak-bundle"
rm -rf "$WORK"
mkdir -p "$WORK/stage"
tar -xzf "$TARBALL" -C "$WORK/stage" --strip-components=1
[ -x "$WORK/stage/bin/cfa4" ] || { echo "error: $TARBALL has no bin/cfa4" >&2; exit 1; }
# Q4.3: bin/cfa4 is the Qt GUI's launcher; a tarball with the native egui editor there is refused
if [ "$(head -c 4 -- "$WORK/stage/bin/cfa4" | od -An -c | tr -d ' ')" = '177ELF' ]; then
  echo "error: $TARBALL carries the egui editor as bin/cfa4 (build it with the current package.sh)" >&2; exit 1
fi
# ADR 0005 step 4: a release bundle carries CFA4's FFmpeg (package.sh without --without-ffmpeg), in
# lib/cfa4/ffmpeg-VERSION with the version pinned in packaging/ffmpeg-version.env
# shellcheck source=packaging/ffmpeg-version.env
. "$ROOT/packaging/ffmpeg-version.env"
[ -f "$WORK/stage/lib/cfa4/ffmpeg-$CFA4_FFMPEG_VERSION/share/doc/ffmpeg/BUILDINFO" ] \
  || { echo "error: $TARBALL has no lib/cfa4/ffmpeg-$CFA4_FFMPEG_VERSION (build it with package.sh, not --without-ffmpeg)" >&2; exit 1; }
# The manifest copies lib/cfa4/ffmpeg-*: exactly one FFmpeg build may be there.
ffmpeg_dirs=("$WORK"/stage/lib/cfa4/ffmpeg-*)
[ "${#ffmpeg_dirs[@]}" -eq 1 ] \
  || { echo "error: $TARBALL carries ${#ffmpeg_dirs[@]} FFmpeg builds in lib/cfa4, expected only ffmpeg-$CFA4_FFMPEG_VERSION" >&2; exit 1; }
[ -x "$WORK/stage/bin/cfa4-qt" ] && [ -d "$WORK/stage/lib/cfa4/python/cfa4_qt" ] \
  || { echo "error: $TARBALL has no Qt GUI (bin/cfa4-qt, lib/cfa4/python)" >&2; exit 1; }
# The Qt GUI's own PySide6 / Qt 6: the runtime has Python 3 but no PySide6 (docs/qt-parity.md 1.10).
python3 "$HERE/stage-pyside6.py" --wheels "${CFA4_PYSIDE6_WHEELS:?set CFA4_PYSIDE6_WHEELS to the local PySide6 wheelhouse}" \
  --dest "$WORK/stage/lib/cfa4/pyside6" --doc "$WORK/stage/share/doc/cfa4/pyside6"
cp "$HERE/flatpak/$APP_ID.bundle.yml" "$WORK/$APP_ID.yml"
# --disable-download only reads the builder's download cache: put the manifest's pinned sources
# (the krb5 tarball) there from the reviewed local copies, after checking their SHA-256.
python3 "$HERE/flatpak-sources.py" seed "$WORK/$APP_ID.yml" \
  "${CFA4_FLATPAK_SOURCES:?set CFA4_FLATPAK_SOURCES to the local Flatpak source directory}" "$WORK/state"

# Never fetch source or runtime dependencies. The SDK is already present.
# --disable-rofiles-fuse also supports runners without FUSE.
flatpak-builder --user --disable-download --disable-rofiles-fuse --force-clean \
  --default-branch=stable --repo="$WORK/repo" --state-dir="$WORK/state" "$WORK/build" "$WORK/$APP_ID.yml"

OUT="$DIST/$BASENAME.flatpak"
# --runtime-repo only embeds an external runtime repo hint; building is offline.
flatpak build-bundle --runtime-repo="$FLATHUB" "$WORK/repo" "$OUT" "$APP_ID" stable
echo "wrote $OUT"

if [ "$TEST" = 1 ]; then
  flatpak install --user -y --noninteractive --no-pull --no-related --reinstall "$OUT"
  flatpak info --user "$APP_ID"
  flatpak run --command=cfa4-cli "$APP_ID" --version
  # the Qt GUI's bundled PySide6 loads inside the sandbox: every module the GUI imports (as
  # stage-pyside6.py's MODULES), so a library the runtime lacks (libgssapi_krb5 for QtNetwork,
  # hence the krb5 module) fails here and not at the user's first start
  flatpak run --command=python3 "$APP_ID" -c 'import sys; sys.path[:0] = ["/app/lib/cfa4/python", "/app/lib/cfa4/pyside6"]; from PySide6 import QtCore, QtGui, QtWidgets, QtNetwork, QtTest, QtMultimedia, QtDBus; assert QtCore.__file__.startswith("/app/lib/cfa4/pyside6"), QtCore.__file__; print("PySide6", QtCore.qVersion())'
  # and the GUI itself starts with its engine (the bundle's command), without a display
  flatpak run --env=QT_QPA_PLATFORM=offscreen "$APP_ID" --demo --no-autosave --smoke

  flatpak run --command=sh "$APP_ID" -c 'ls /app/share/applications /app/share/metainfo /app/share/mime/packages /app/share/icons/hicolor/scalable/apps'
fi
echo "==> done"
ls -lh "$OUT"
