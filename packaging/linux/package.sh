#!/usr/bin/env bash
# CFA4 local Ubuntu 26.04 x86_64 (AMD64) packaging only.
# Outputs: AppImage (+ optional .zsync), .deb and .tar.gz; Flatpak uses flatpak-bundle.sh.
# Distribution remains blocked by the attribution/branding gate below.
# Other distributions and architectures are not CFA4 release targets.
# Usage: packaging/linux/package.sh [--skip-build] [--formats "appimage deb tar restricted"]
#        [--without-ffmpeg].
# Prerequisites: cargo; nfpm for deb; for AppImage: reviewed local appimagetool
# plus CFA4_APPIMAGETOOL_SHA256; optional zsyncmake. Never auto-download tools.
# FFmpeg (ADR 0005 step 4): CFA4's own FFmpeg 9.0.2 build from scripts/dev/build-ffmpeg-9.0.2.sh
# (CFA4_FFMPEG_BUILD, default /opt/cfa4/ffmpeg-9.0.2) ships in every format: the .deb installs it
# into /opt/cfa4/ffmpeg-9.0.2, the tarball/AppImage/Flatpak carry it in lib/cfa4/ffmpeg-9.0.2.
# `restricted` (not a default format) builds cfa4-restricted-extras, the separate .deb with the same
# FFmpeg plus x264/x265 (`--restricted` build) in /opt/cfa4/ffmpeg-9.0.2-restricted (Ubuntu
# model, docs/licensing.md §7). --without-ffmpeg is for local tests of the tarball only.
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
WITH_FFMPEG=1
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --formats) FORMATS="$2"; shift 2 ;;
    --without-ffmpeg) WITH_FFMPEG=0; shift ;;
    -h | --help) sed -n '2,28p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

read -r -a requested_formats <<< "$FORMATS"
if (( ${#requested_formats[@]} == 0 )); then
  echo "error: --formats requires one or more of: appimage deb tar restricted" >&2
  exit 2
fi
for format in "${requested_formats[@]}"; do
  case "$format" in
    appimage|deb|tar|restricted) ;;
    *) echo "error: unsupported CFA4 package format: $format (supported: appimage deb tar restricted; Flatpak uses flatpak-bundle.sh)" >&2; exit 2 ;;
  esac
  if [[ "$WITH_FFMPEG" == 0 && "$format" != tar ]]; then
    echo "error: --without-ffmpeg is for local tarball tests only; release formats ship FFmpeg 9.0.2 (ADR 0005)" >&2
    exit 2
  fi
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

FFMPEG_VERSION=9.0.2
FFMPEG_BUILD="${CFA4_FFMPEG_BUILD:-/opt/cfa4/ffmpeg-$FFMPEG_VERSION}"
# the run-time search path the binaries must carry (apps/*/build.rs with the default prefix)
FFMPEG_RUNPATH="/opt/cfa4/ffmpeg-$FFMPEG_VERSION-restricted/lib:/opt/cfa4/ffmpeg-$FFMPEG_VERSION/lib:\$ORIGIN/../lib/cfa4/ffmpeg-$FFMPEG_VERSION/lib"
has() { case " $FORMATS " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }
if [[ "$WITH_FFMPEG" == 1 ]]; then
  builds=("$FFMPEG_BUILD")
  if has restricted; then builds+=("$FFMPEG_BUILD-restricted"); fi
  for build in "${builds[@]}"; do
    [ -x "$build/bin/ffmpeg" ] || { echo "error: no FFmpeg $FFMPEG_VERSION build in $build (scripts/dev/build-ffmpeg-9.0.2.sh$([[ "$build" == *-restricted ]] && echo ' --restricted'); CFA4_FFMPEG_BUILD moves it)" >&2; exit 1; }
  done
else
  warn "--without-ffmpeg: a local test tarball without the FFmpeg backend, not a release package"
fi

echo "==> CFA4 $VERSION for Linux $ARCH ($FORMATS)"

if [ "$SKIP_BUILD" = 0 ]; then
  features=cfa4/gstreamer,cfa4-cli/gstreamer
  if [[ "$WITH_FFMPEG" == 1 ]]; then
    features+=,cfa4/ffmpeg,cfa4-cli/ffmpeg
    # Link against the build, but keep the canonical /opt/cfa4 run-time path (no CFA4_FFMPEG_PREFIX).
    (cd "$ROOT" && env -u CFA4_FFMPEG_PREFIX PKG_CONFIG_PATH="$FFMPEG_BUILD/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}" \
      cargo build --release --locked -p cfa4 -p cfa4-cli --features "$features")
  else
    (cd "$ROOT" && cargo build --release --locked -p cfa4 -p cfa4-cli --features "$features")
  fi
fi
BIN="$CARGO_TARGET_DIR/release"
WORK="$CARGO_TARGET_DIR/linux-package"
STAGE="$WORK/root"
rm -rf "$WORK"
if [[ "$WITH_FFMPEG" == 1 ]]; then
  for binary in cfa4 cfa4-cli; do
    runpath="$(readelf -d "$BIN/$binary" | sed -n 's/.*(RUNPATH).*\[\(.*\)\]/\1/p')"
    if [[ "$runpath" != *"$FFMPEG_RUNPATH"* ]]; then
      echo "error: $BIN/$binary has RUNPATH '$runpath', expected '$FFMPEG_RUNPATH' (built without the ffmpeg feature, or with CFA4_FFMPEG_PREFIX?)" >&2
      exit 1
    fi
  done
fi

# ---- stage an FHS tree (shared by every format) -------------------------------------------------
install -Dm755 "$BIN/cfa4" "$STAGE/usr/bin/cfa4"
install -Dm755 "$BIN/cfa4-cli" "$STAGE/usr/bin/cfa4-cli"
strip "$STAGE/usr/bin/cfa4" "$STAGE/usr/bin/cfa4-cli" 2>/dev/null || true
# The Qt 6 GUI (ADR F8 phase 7): its Python package and the cfa4-qt launcher. PySide6 is a package
# dependency from Ubuntu's archive (python3-pyside6.*, LGPL-3.0), never bundled. The shared egui
# catalogs (with their attribution sidecars) go with it, so an installed GUI translates as a
# checkout does.
install -Dm755 "$HERE/cfa4-qt" "$STAGE/usr/bin/cfa4-qt"
QT_PY="$STAGE/usr/lib/cfa4/python"
mkdir -p "$QT_PY"
cp -R "$ROOT/python/cfa4_qt" "$QT_PY/"
find "$QT_PY" -name __pycache__ -type d -prune -exec rm -rf {} +
mkdir -p "$QT_PY/cfa4_qt/i18n/shared"
cp "$ROOT"/crates/ui-egui/src/i18n/*.tsv "$ROOT"/crates/ui-egui/src/i18n/*.tsv.attribution "$QT_PY/cfa4_qt/i18n/shared/"
# the About window's credits, as egui bakes them in (crates/ui-egui/build.rs)
mkdir -p "$QT_PY/cfa4_qt/credits"
cp "$ROOT/contributors/contributors.json" "$ROOT/contributors/people.toml" "$QT_PY/cfa4_qt/credits/"
# FFmpeg 9.0.2 (ADR 0005 step 4). Relocatable formats: the libraries with the third-party ones they
# load, beside the binaries ($ORIGIN RUNPATH). The .deb: /opt/cfa4/ffmpeg-9.0.2, its dependencies
# from Ubuntu's archive as Depends.
FFMPEG_DEB="$WORK/deb-ffmpeg"
if [[ "$WITH_FFMPEG" == 1 ]]; then
  bash "$HERE/stage-ffmpeg.sh" --build "$FFMPEG_BUILD" --bundle --dest "$STAGE/usr/lib/cfa4/ffmpeg-$FFMPEG_VERSION"
  if has deb; then
    bash "$HERE/stage-ffmpeg.sh" --build "$FFMPEG_BUILD" --dest "$FFMPEG_DEB/opt/cfa4/ffmpeg-$FFMPEG_VERSION" \
      --depends-out "$WORK/ffmpeg-depends.txt"
  fi
fi
install -Dm644 "$HERE/$APP_ID.desktop" "$STAGE/usr/share/applications/$APP_ID.desktop"
install -Dm644 "$HERE/$APP_ID.mime.xml" "$STAGE/usr/share/mime/packages/$APP_ID.xml"
mkdir -p "$STAGE/usr/share/metainfo"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$CFA4_BUILD_DATE/g" \
  "$HERE/$APP_ID.metainfo.xml.in" >"$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
mkdir -p "$STAGE/usr/share/icons"
cp -R "$ROOT/assets/cfa4/hicolor" "$STAGE/usr/share/icons/"
mkdir -p "$STAGE/usr/share/doc/cfa4"
copy_docs "$STAGE/usr/share/doc/cfa4"
# Debian-style copyright summary (the full texts sit next to it).
install -Dm644 "$HERE/copyright" "$STAGE/usr/share/doc/cfa4/copyright"
bash "$ROOT/scripts/ci/check-package-docs.sh" "$STAGE/usr/share/doc/cfa4"

if command -v desktop-file-validate >/dev/null; then
  desktop-file-validate "$STAGE/usr/share/applications/$APP_ID.desktop"
fi
if command -v appstreamcli >/dev/null; then
  appstreamcli validate --no-net --explain "$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
fi

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
  # nfpm.yaml pins arch=amd64; only VERSION is read from the environment. The FFmpeg libraries'
  # Ubuntu packages replace the @FFMPEG_DEPENDS@ line.
  ffmpeg_depends="$(sed 's/^/      - /' "$WORK/ffmpeg-depends.txt")"
  sed -e "s|\${STAGE}|$STAGE|g" -e "s|\${FFMPEG_DEB}|$FFMPEG_DEB|g" "$HERE/nfpm.yaml" \
    | deps="$ffmpeg_depends" awk '/# @FFMPEG_DEPENDS@/ { if (ENVIRON["deps"] != "") print ENVIRON["deps"]; next } { print }' >"$WORK/nfpm.yaml"
  (cd "$ROOT" && nfpm package -f "$WORK/nfpm.yaml" -p deb -t "$DIST/$BASENAME.deb")
fi

# ---- cfa4-restricted-extras .deb (only on request: --formats "... restricted") ---------------------
if has restricted; then
  command -v nfpm >/dev/null || { echo "error: nfpm not found (https://nfpm.goreleaser.com/install/)" >&2; exit 1; }
  RESTRICTED="$WORK/restricted"
  bash "$HERE/stage-ffmpeg.sh" --restricted --build "$FFMPEG_BUILD-restricted" \
    --dest "$RESTRICTED/opt/cfa4/ffmpeg-$FFMPEG_VERSION-restricted" --depends-out "$WORK/restricted-depends.txt"
  # Exactly this cfa4: the extras replace its FFmpeg libraries through the binaries' RUNPATH.
  # nfpm's semver schema writes 0.2.0-rc.1 as 0.2.0~rc.1, release 1.
  deb_version="${VERSION/-/\~}-1"
  restricted_depends="$(sed 's/^/      - /' "$WORK/restricted-depends.txt")"
  sed -e "s|\${RESTRICTED}|$RESTRICTED|g" -e "s|@CFA4_DEB_VERSION@|$deb_version|g" "$HERE/nfpm-restricted.yaml" \
    | deps="$restricted_depends" awk '/# @FFMPEG_DEPENDS@/ { if (ENVIRON["deps"] != "") print ENVIRON["deps"]; next } { print }' >"$WORK/nfpm-restricted.yaml"
  export VERSION
  (cd "$ROOT" && nfpm package -f "$WORK/nfpm-restricted.yaml" -p deb \
    -t "$DIST/cfa4-restricted-extras-$VERSION-linux-$ARCH.deb")
fi

# ---- AppImage -----------------------------------------------------------------------------------
if has appimage; then
  APPDIR="$WORK/CFA4.AppDir"
  cp -R "$STAGE" "$APPDIR"
  # The AppImage stays the egui app: it cannot depend on the host's python3-pyside6 packages and
  # does not bundle PySide6, so the Qt GUI (cfa4-qt and its Python package) ships only in .deb/tar.
  rm -rf "$APPDIR/usr/bin/cfa4-qt" "$APPDIR/usr/lib/cfa4/python"
  rmdir "$APPDIR/usr/lib/cfa4" "$APPDIR/usr/lib" 2>/dev/null || true
  # Keep usr/share/doc: licences, NOTICE and attributions must ship inside the AppImage too.
  ln -s usr/bin/cfa4 "$APPDIR/AppRun"
  cp "$HERE/$APP_ID.desktop" "$APPDIR/$APP_ID.desktop"
  # the CFA4 logo is raster artwork: the 256 px PNG is the AppImage icon
  cp "$ROOT/assets/cfa4/hicolor/256x256/apps/$APP_ID.png" "$APPDIR/$APP_ID.png"
  ln -s "$APP_ID.png" "$APPDIR/.DirIcon"

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
if [[ "$WITH_FFMPEG" == 1 ]]; then
  # The staged binaries must start on the bundled FFmpeg alone (no /opt/cfa4 needed) and report it.
  for binary in cfa4 cfa4-cli; do
    if env -u LD_LIBRARY_PATH ldd "$STAGE/usr/bin/$binary" | grep -q 'not found'; then
      env -u LD_LIBRARY_PATH ldd "$STAGE/usr/bin/$binary" | grep 'not found' >&2
      echo "error: the staged $binary misses libraries" >&2
      exit 1
    fi
  done
  backends="$(env -u LD_LIBRARY_PATH "$STAGE/usr/bin/cfa4-cli" media-backends --data-dir "$WORK/data")"
  python3 -c '
import json, sys
def walk(v):
    if isinstance(v, dict):
        yield v
        for x in v.values(): yield from walk(x)
    elif isinstance(v, list):
        for x in v: yield from walk(x)
ff = [d for d in walk(json.loads(sys.argv[1])) if d.get("id") == "ffmpeg" and "version" in d]
assert ff and ff[0].get("available") and "9.0.2" in str(ff[0]["version"]), ff
' "$backends" || { echo "error: the staged cfa4-cli does not report FFmpeg 9.0.2 as available" >&2; exit 1; }
fi
# -P: the staged package, never a checkout's python/ that the caller's directory would put first
PYTHONPATH="$QT_PY" python3 -P -c 'import cfa4_qt.i18n as i; assert i.CATALOG_DIRS[1].name == "shared"' \
  || { echo "error: the staged Qt GUI does not find its shared catalogs" >&2; exit 1; }
PYTHONPATH="$QT_PY" python3 -P -c 'import cfa4_qt.credits as c; assert c.CREDITS_DIR.name == "credits" and c.load().contributors' \
  || { echo "error: the staged Qt GUI does not find its credits" >&2; exit 1; }
echo "==> done"
ls -lh "$DIST"
