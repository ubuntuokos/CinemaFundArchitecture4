#!/usr/bin/env bash
# Repackage a CFA4 Ubuntu 26.04 Linux tarball as a single-file Flatpak bundle:
#
#   $DIST/cfa4-<version>-linux-x86_64.flatpak
#
# Usage: packaging/linux/flatpak-bundle.sh [--no-test|--test] [TARBALL]
#
# TARBALL defaults to $DIST/cfa4-<version>-linux-x86_64.tar.gz from package.sh.
# Only Ubuntu 26.04 x86_64 (AMD64) build hosts are supported.
# Needs preinstalled flatpak, flatpak-builder, OSTree, Platform+SDK 25.08.
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
    -h | --help) sed -n '2,14p' "$0"; exit 0 ;;
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
cp "$HERE/flatpak/$APP_ID.bundle.yml" "$WORK/$APP_ID.yml"

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
  flatpak run --command=sh "$APP_ID" -c 'ls /app/share/applications /app/share/metainfo /app/share/mime/packages /app/share/icons/hicolor/scalable/apps'
fi
echo "==> done"
ls -lh "$OUT"
