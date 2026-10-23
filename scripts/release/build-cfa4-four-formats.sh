#!/usr/bin/env bash
# Build four CFA4 *local validation* artifacts; no release publishing or apt installs.
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/../../packaging/env.sh"
bash "$ROOT/scripts/ci/require-ubuntu-2604.sh"
if [[ "${CFA4_ALLOW_LOCAL_PACKAGING:-0}" != "1" ]]; then
  echo "CFA4 release HOLD: explicit CFA4_ALLOW_LOCAL_PACKAGING=1 is required for local tests." >&2
  exit 3
fi
# Verify every reviewed local prerequisite BEFORE creating any package.
bash "$ROOT/scripts/release/check-cfa4-package-prereqs.sh" --formats "deb appimage flatpak tar"
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
python3 "$ROOT/scripts/release/verify_cfa4_packages.py" --dist "$DIST" --version "$VERSION"
echo "Four CFA4 locally verified packages and SHA256SUMS created in $DIST"
echo "RELEASE HOLD: no redistribution or public GitHub release is authorized."
