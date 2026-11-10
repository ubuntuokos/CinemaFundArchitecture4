#!/usr/bin/env bash
# CI check (#220): build a real .deb with the pinned nfpm from the real packaging/linux/nfpm.yaml
# and the repository's real licence, attribution, desktop and Qt GUI files, then audit its
# contents with verify_cfa4_packages.verify_deb (dpkg-deb). Only the binaries, the FFmpeg
# libraries and the crate notices are stand-ins: building them needs the release toolchain and the
# FFmpeg 9.0.2 build, which the self-hosted four-formats job has (it audits the real ones too).
#   deb-contents-check.sh [--keep DIR]    keep the staged tree and the .deb in DIR
# Needs: the pinned nfpm (scripts/release/install-nfpm.sh), dpkg-deb, python3.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
keep=""
while [ $# -gt 0 ]; do
  case "$1" in
    --keep) keep="$2"; shift 2 ;;
    -h | --help) sed -n '2,8p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
work="${keep:-$(mktemp -d)}"
if [[ -z "$keep" ]]; then trap 'rm -rf "$work"' EXIT; fi
mkdir -p "$work"
work="$(cd "$work" && pwd)"
export DIST="$work/dist" CARGO_TARGET_DIR="$work/target"
# shellcheck source=../../packaging/env.sh
. "$ROOT/packaging/env.sh"
HERE="$ROOT/packaging/linux"
APP_ID=io.github.ubuntuokos.cfa4
STAGE="$work/root"
FFMPEG_DEB="$work/deb-ffmpeg"
rm -rf "$STAGE" "$FFMPEG_DEB"

# stand-ins for what needs the release build
for binary in cfa4 cfa4-cli; do
  install -d "$STAGE/usr/bin"
  printf '#!/bin/sh\necho "%s %s (deb-contents-check stand-in)"\n' "$binary" "$VERSION" >"$STAGE/usr/bin/$binary"
  chmod 755 "$STAGE/usr/bin/$binary"
done
ffmpeg="$FFMPEG_DEB/opt/cfa4/ffmpeg-9.0.2"
install -d "$ffmpeg/lib" "$ffmpeg/share/doc/ffmpeg"
for lib in libavcodec.so.62 libavformat.so.62 libavutil.so.60; do echo stand-in >"$ffmpeg/lib/$lib"; done
ln -s libavcodec.so.62 "$ffmpeg/lib/libavcodec.so"
for doc in BUILDINFO COPYING.GPLv3 LICENSE.md; do echo "FFmpeg 9.0.2 $doc (stand-in)" >"$ffmpeg/share/doc/ffmpeg/$doc"; done
printf 'CFA4 - third-party licence notices (deb-contents-check stand-in)\n' >"$work/THIRD-PARTY-LICENSES.txt"
printf '{"bomFormat": "CycloneDX", "specVersion": "1.6"}\n' >"$work/cfa4.cdx.json"
printf 'libzstd1\n' >"$work/ffmpeg-depends.txt"

# the real files, staged as package.sh stages them
install -Dm755 "$HERE/cfa4-qt" "$STAGE/usr/bin/cfa4-qt"
install -d "$STAGE/usr/lib/cfa4/python"
cp -R "$ROOT/python/cfa4_qt" "$STAGE/usr/lib/cfa4/python/"
find "$STAGE/usr/lib/cfa4/python" -name __pycache__ -type d -prune -exec rm -rf {} +
install -Dm644 "$HERE/$APP_ID.desktop" "$STAGE/usr/share/applications/$APP_ID.desktop"
install -Dm644 "$HERE/$APP_ID.mime.xml" "$STAGE/usr/share/mime/packages/$APP_ID.xml"
install -d "$STAGE/usr/share/metainfo" "$STAGE/usr/share/icons" "$STAGE/usr/share/doc/cfa4"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$CFA4_BUILD_DATE/g" \
  "$HERE/$APP_ID.metainfo.xml.in" >"$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
cp -R "$ROOT/assets/cfa4/hicolor" "$STAGE/usr/share/icons/"
CFA4_THIRD_PARTY_LICENSES_FILE="$work/THIRD-PARTY-LICENSES.txt" CFA4_SBOM_FILE="$work/cfa4.cdx.json" \
  copy_docs "$STAGE/usr/share/doc/cfa4"
install -Dm644 "$HERE/copyright" "$STAGE/usr/share/doc/cfa4/copyright"
bash "$ROOT/scripts/ci/check-package-docs.sh" "$STAGE/usr/share/doc/cfa4"

deb="$DIST/cfa4-$VERSION-linux-x86_64.deb"
rm -f "$deb"
export CFA4_MAINTAINER="${CFA4_MAINTAINER:-CFA4 contributors}"
bash "$HERE/nfpm-package.sh" "$HERE/nfpm.yaml" "$work/ffmpeg-depends.txt" "$deb" \
  STAGE="$STAGE" FFMPEG_DEB="$FFMPEG_DEB"
python3 -I -c '
import sys
sys.path.insert(0, sys.argv[3])
import verify_cfa4_packages as audit
from pathlib import Path
try:
    audit.verify_deb(Path(sys.argv[1]), sys.argv[2])
except audit.AuditError as exc:
    sys.exit(f"FAIL .deb contents: {exc}")
' "$deb" "$VERSION" "$ROOT/scripts/release"
depends="$(dpkg-deb --field "$deb" Depends)"
grep -q 'libzstd1' <<<"$depends" || { echo "error: the FFmpeg Depends line was not filled in: $depends" >&2; exit 1; }
if grep -q '@FFMPEG_DEPENDS@\|\${' <<<"$(dpkg-deb --info "$deb")"; then
  echo "error: an unsubstituted template placeholder reached the .deb control file" >&2; exit 1
fi
dpkg-deb --info "$deb" | sed -n '1,12p'
echo "PASS: real nfpm $(sed -n 's/^CFA4_NFPM_VERSION=//p' "$HERE/nfpm.lock") .deb with the licence/attribution files: $deb"
