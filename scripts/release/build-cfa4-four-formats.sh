#!/usr/bin/env bash
# Build four CFA4 *local validation* artifacts; no release publishing or apt installs.
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/../../packaging/env.sh"
bash "$ROOT/scripts/ci/require-ubuntu-2604.sh"
if [[ "${CFA4_ALLOW_LOCAL_PACKAGING:-0}" != "1" ]]; then
  echo "CFA4 release HOLD: explicit CFA4_ALLOW_LOCAL_PACKAGING=1 is required for local tests." >&2
  exit 3
fi
for tool in cargo nfpm flatpak flatpak-builder sha256sum; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "error: reviewed tool '$tool' is not installed; no automatic installation." >&2
    exit 2
  }
done
# appimagetool must be supplied and pinned to an independently reviewed digest.
if [[ ! "${CFA4_APPIMAGETOOL_SHA256:-}" =~ ^[[:xdigit:]]{64}$ ]]; then
  echo "error: CFA4_APPIMAGETOOL_SHA256 is required for the reviewed appimagetool." >&2
  exit 2
fi
"$ROOT/packaging/linux/package.sh" --formats "deb appimage tar"
BASENAME="cfa4-$VERSION-linux-x86_64"
"$ROOT/packaging/linux/flatpak-bundle.sh" --no-test "$DIST/$BASENAME.tar.gz"
artifacts=(
  "$DIST/$BASENAME.deb"
  "$DIST/$BASENAME.AppImage"
  "$DIST/$BASENAME.flatpak"
  "$DIST/$BASENAME.tar.gz"
)
for artifact in "${artifacts[@]}"; do
  [[ -s "$artifact" ]] || { echo "error: missing or empty CFA4 package $artifact" >&2; exit 1; }
done
(cd "$DIST" && sha256sum "${artifacts[@]##*/}" > SHA256SUMS)
echo "Four CFA4 local validation packages and SHA256SUMS created in $DIST"
echo "RELEASE HOLD: no redistribution or public GitHub release is authorized."
