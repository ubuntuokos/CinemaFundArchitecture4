#!/usr/bin/env bash
# Assert the licence/attribution files every CFA4 Linux package must ship (issue #6, F1-01).
#   check-package-docs.sh <doc-dir>   check an already staged usr/share/doc/cfa4
#   check-package-docs.sh             stage copy_docs + copyright into a temp dir and check it,
#                                     and check that the AppImage path keeps usr/share/doc
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
required=(LICENSE NOTICE ATTRIBUTION.md LICENSE-MIT LICENSE-APACHE LICENSE-logo.txt copyright)
for f in "$ROOT"/assets/fonts/OFL-*.txt; do required+=("$(basename "$f")"); done

check_dir() {
  local dir="$1" f missing=0
  for f in "${required[@]}"; do
    if [ ! -s "$dir/$f" ]; then echo "error: $f is missing from $dir" >&2; missing=1; fi
  done
  return "$missing"
}

if [ $# -ge 1 ]; then check_dir "$1"; exit; fi

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
DIST="$tmp/dist" CARGO_TARGET_DIR="$tmp/target" bash -c '
  set -euo pipefail
  . "$1/packaging/env.sh"
  mkdir -p "$2"; copy_docs "$2"
  install -m644 "$1/packaging/linux/copyright" "$2/copyright"' _ "$ROOT" "$tmp/doc"
check_dir "$tmp/doc"
if grep -nE 'usr/share/doc"? +"?\$WORK|rm -rf? .*usr/share/doc' "$ROOT/packaging/linux/package.sh"; then
  echo "error: packaging/linux/package.sh removes usr/share/doc from a package" >&2; exit 1
fi
echo "package docs OK: ${required[*]}"
