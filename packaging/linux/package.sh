#!/usr/bin/env bash
# CFA4 local Ubuntu 26.04 x86_64 (AMD64) packaging only.
# Outputs: AppImage (+ optional .zsync), .deb and .tar.gz; Flatpak uses flatpak-bundle.sh.
# Distribution remains blocked by the attribution/branding gate below.
# Other distributions and architectures are not CFA4 release targets.
# Usage: packaging/linux/package.sh [--skip-build] [--formats "appimage deb tar"].
# Prerequisites: cargo; nfpm for deb; for AppImage: reviewed local appimagetool
# plus CFA4_APPIMAGETOOL_SHA256; optional zsyncmake. Never auto-download tools.
set -euo pipefail
# CFA4 LICENSE/IDENTITY GATE: packaging is developer-only until the CFA4 icons,
# runtime identity and all third-party attribution have been audited.
if [[ "${CFA4_ALLOW_LOCAL_PACKAGING:-0}" != "1" ]]; then
  echo "CFA4 packaging blocked: set CFA4_ALLOW_LOCAL_PACKAGING=1 for a LOCAL TEST ONLY." >&2
  echo "No redistribution before brand/icon review and Qt6 migration." >&2
  exit 3
fi
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/linux"
APP_ID=io.github.ubuntuokos.cfa4

SKIP_BUILD=0
FORMATS="appimage deb tar"
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --formats) FORMATS="$2"; shift 2 ;;
    -h | --help) sed -n '2,21p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

read -r -a requested_formats <<< "$FORMATS"
if (( ${#requested_formats[@]} == 0 )); then
  echo "error: --formats requires one or more of: appimage deb tar" >&2
  exit 2
fi
for format in "${requested_formats[@]}"; do
  case "$format" in
    appimage|deb|tar) ;;
    *) echo "error: unsupported CFA4 package format: $format (supported: appimage deb tar; Flatpak uses flatpak-bundle.sh)" >&2; exit 2 ;;
  esac
done

if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
  echo "CFA4 packages only Ubuntu 26.04 Linux x86_64 (AMD64)." >&2
  exit 2
fi
. "$ROOT/scripts/ci/require-ubuntu-2604.sh"
if [[ -n "${CROSS_ARCH:-}" || -n "${CROSS_TARGET:-}" || -n "${CROSS_COMPILE:-}" || -n "${EMULATOR:-}" || -n "${CARGO_BUILD_TARGET:-}" ]]; then
  echo "Cross-compilation and CARGO_BUILD_TARGET overrides are not supported by CFA4 packaging." >&2
  exit 2
fi
ARCH=x86_64
export CFA4_MAINTAINER="${CFA4_MAINTAINER:-CFA4 contributors}"
BASENAME="cfa4-$VERSION-linux-$ARCH"

echo "==> CFA4 $VERSION for Linux $ARCH ($FORMATS)"

if [ "$SKIP_BUILD" = 0 ]; then
  (cd "$ROOT" && cargo build --release --locked -p cfa4 -p cfa4-cli)
fi
BIN="$CARGO_TARGET_DIR/release"
WORK="$CARGO_TARGET_DIR/linux-package"
STAGE="$WORK/root"
rm -rf "$WORK"

# ---- stage an FHS tree (shared by every format) -------------------------------------------------
install -Dm755 "$BIN/cfa4" "$STAGE/usr/bin/cfa4"
install -Dm755 "$BIN/cfa4-cli" "$STAGE/usr/bin/cfa4-cli"
strip "$STAGE/usr/bin/cfa4" "$STAGE/usr/bin/cfa4-cli" 2>/dev/null || true
install -Dm644 "$HERE/$APP_ID.desktop" "$STAGE/usr/share/applications/$APP_ID.desktop"
install -Dm644 "$HERE/$APP_ID.mime.xml" "$STAGE/usr/share/mime/packages/$APP_ID.xml"
mkdir -p "$STAGE/usr/share/metainfo"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$CFA4_BUILD_DATE/g" \
  "$HERE/$APP_ID.metainfo.xml.in" >"$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
mkdir -p "$STAGE/usr/share/icons"
cp -R "$ROOT/assets/cfa4/hicolor" "$STAGE/usr/share/icons/"
mkdir -p "$STAGE/usr/share/doc/cfa4"
copy_docs "$STAGE/usr/share/doc/cfa4"

if command -v desktop-file-validate >/dev/null; then
  desktop-file-validate "$STAGE/usr/share/applications/$APP_ID.desktop"
fi
if command -v appstreamcli >/dev/null; then
  appstreamcli validate --no-net --explain "$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
fi

has() { case " $FORMATS " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }

# ---- .tar.gz ------------------------------------------------------------------------------------
if has tar; then
  mkdir -p "$WORK/tar"
  cp -R "$STAGE/usr" "$WORK/tar/$BASENAME"
  install -m755 "$HERE/install.sh" "$WORK/tar/$BASENAME/install.sh"
  tar -C "$WORK/tar" -czf "$DIST/$BASENAME.tar.gz" "$BASENAME"
  echo "wrote $DIST/$BASENAME.tar.gz"
fi

# ---- .deb ----------------------------------------------------------------------------------------
if has deb; then
  command -v nfpm >/dev/null || { echo "error: nfpm not found (https://nfpm.goreleaser.com/install/)" >&2; exit 1; }
  export VERSION
  # nfpm.yaml pins arch=amd64; only VERSION is read from the environment.
  sed "s|\${STAGE}|$STAGE|g" "$HERE/nfpm.yaml" >"$WORK/nfpm.yaml"
  (cd "$ROOT" && nfpm package -f "$WORK/nfpm.yaml" -p deb -t "$DIST/$BASENAME.deb")
fi

# ---- AppImage -----------------------------------------------------------------------------------
if has appimage; then
  APPDIR="$WORK/CFA4.AppDir"
  cp -R "$STAGE" "$APPDIR"
  mv "$APPDIR/usr/share/doc" "$WORK/doc-unused"
  ln -s usr/bin/cfa4 "$APPDIR/AppRun"
  cp "$HERE/$APP_ID.desktop" "$APPDIR/$APP_ID.desktop"
  cp "$ROOT/assets/cfa4/hicolor/scalable/apps/$APP_ID.svg" "$APPDIR/$APP_ID.svg"
  ln -s "$APP_ID.svg" "$APPDIR/.DirIcon"

  # Never download or execute a moving upstream binary automatically.
  # Install a reviewed, version-pinned appimagetool beforehand and supply its
  # independently verified SHA-256 via CFA4_APPIMAGETOOL_SHA256.
  TOOL="${APPIMAGETOOL:-$(command -v appimagetool || true)}"
  if [[ -z "$TOOL" || ! -f "$TOOL" || ! -x "$TOOL" ]]; then
    echo "error: a trusted local appimagetool executable is required (set APPIMAGETOOL)." >&2
    exit 2
  fi
  EXPECTED_SHA="${CFA4_APPIMAGETOOL_SHA256:-}"
  if [[ ! "$EXPECTED_SHA" =~ ^[[:xdigit:]]{64}$ ]]; then
    echo "error: set CFA4_APPIMAGETOOL_SHA256 to the approved 64-hex SHA-256 of APPIMAGETOOL." >&2
    exit 2
  fi
  command -v sha256sum >/dev/null || { echo "error: sha256sum is required" >&2; exit 2; }
  ACTUAL_SHA="$(sha256sum -- "$TOOL" | awk '{print $1}')"
  if [[ "$ACTUAL_SHA" != "${EXPECTED_SHA,,}" ]]; then
    echo "error: appimagetool SHA-256 mismatch; refusing to execute unverified binary." >&2
    exit 2
  fi
  # Absolute, because appimagetool runs in $DIST below (APPIMAGETOOL may be relative).
  OUT="$(cd "$DIST" && pwd)/$BASENAME.AppImage"
  APPDIR="$(cd "$APPDIR" && pwd)"
  TOOL="$(cd "$(dirname "$TOOL")" && pwd)/$(basename "$TOOL")"
  # A .zsync left by an earlier run would hide a missing zsyncmake and describe another file.
  rm -f "$OUT.zsync"
  # Update information: AppImageUpdate, AppImageLauncher and the like read it from the file and
  # fetch only the blocks that changed in a newer release, through the .zsync published next to
  # each AppImage on CFA4 GitHub Releases. `latest` is the newest published release that is not a
  # pre-release. An inherited or fork update endpoint must never be silently substituted.
  REPO="ubuntuokos/CinemaFundArchitecture4"
  if [[ -n "${GITHUB_REPOSITORY:-}" && "$GITHUB_REPOSITORY" != "$REPO" ]]; then
    echo "error: refusing an AppImage update feed outside the CFA4 release repository" >&2
    exit 2
  fi
  UPDATE_INFO="gh-releases-zsync|${REPO%%/*}|${REPO#*/}|latest|cfa4-*-linux-$ARCH.AppImage.zsync"
  # Extract-and-run: works without FUSE (containers, CI). The output embeds the static runtime,
  # so users don't need libfuse2 either. With zsyncmake on the host (CI installs the zsync
  # package) appimagetool also writes the .zsync, into its working directory, hence the cd.
  (cd "$DIST" && ARCH="$ARCH" APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --no-appstream -u "$UPDATE_INFO" "$APPDIR" "$OUT")
  echo "wrote $OUT"
  if [ -s "$OUT.zsync" ]; then
    echo "wrote $OUT.zsync"
  else
    warn "zsyncmake not found, so $OUT.zsync was not written; AppImage delta updates need it"
  fi
fi

"$STAGE/usr/bin/cfa4-cli" --version
echo "==> done"
ls -lh "$DIST"
