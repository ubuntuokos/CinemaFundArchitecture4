#!/usr/bin/env bash
# Download the pinned sources of the CFA4 Flatpak bundle manifest (the krb5 tarball) into DEST and
# check each one's SHA-256 against the manifest:
#
#   scripts/release/fetch-flatpak-sources.sh DEST
#
# For runners that provision their own tools (cfa4-four-formats-hosted.yml). flatpak-bundle.sh
# itself never downloads: it builds with --disable-download and takes DEST as CFA4_FLATPAK_SOURCES.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MANIFEST="$ROOT/packaging/linux/flatpak/io.github.ubuntuokos.cfa4.bundle.yml"
HELPER="$ROOT/packaging/linux/flatpak-sources.py"
[[ $# == 1 ]] || { echo "usage: $0 DEST" >&2; exit 2; }
DEST="$1"
mkdir -p "$DEST"

python3 "$HELPER" list "$MANIFEST" | while read -r sha name url; do
  [[ "$url" == https://* ]] || { echo "error: $name: refusing a non-https source URL: $url" >&2; exit 1; }
  if [[ -f "$DEST/$name" ]] && printf '%s  %s\n' "$sha" "$DEST/$name" | sha256sum --check --status; then
    echo "present: $name"
    continue
  fi
  curl --fail --location --show-error --silent --retry 5 --proto '=https' --tlsv1.2 \
    "$url" --output "$DEST/$name.part"
  printf '%s  %s\n' "$sha" "$DEST/$name.part" | sha256sum --check
  mv "$DEST/$name.part" "$DEST/$name"
done
python3 "$HELPER" check "$MANIFEST" "$DEST"
