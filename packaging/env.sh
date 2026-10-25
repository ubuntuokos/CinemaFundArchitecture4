# shellcheck shell=bash
# Shared setup for the packaging scripts. Source it: `. "$(dirname "$0")/../env.sh"`.
#
# Exports:
#   ROOT                    workspace root
#   VERSION                 [workspace.package] version from CFA4 Cargo.toml (no upstream override)
#   DIST                    output directory for release artifacts (default: $ROOT/dist/release)
#   CFA4_BUILD_SHA          CFA4 git commit embedded as build info
#   CFA4_BUILD_DATE         UTC release metadata date, YYYY-MM-DD (also embedded as build info)
#   CARGO_TARGET_DIR        cargo's target dir (default: $ROOT/target)

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export ROOT

# The version lives in exactly one place: `[workspace.package] version` in the root Cargo.toml.
# (`cargo xtask version` prints the same thing; awk avoids compiling xtask here.)
workspace_version() {
  awk '
    /^\[/ { in_pkg = ($0 == "[workspace.package]") ; next }
    in_pkg && $1 == "version" { gsub(/[" ]/, "", $3); print $3; exit }
  ' "$ROOT/Cargo.toml"
}

VERSION="$(workspace_version)"
if [[ -n "${CFA4_VERSION:-}" && "$CFA4_VERSION" != "$VERSION" ]]; then
  echo "error: CFA4_VERSION must match [workspace.package] version: $VERSION" >&2
  exit 2
fi
if [ -z "$VERSION" ]; then
  echo "error: could not read [workspace.package] version from $ROOT/Cargo.toml" >&2
  exit 1
fi
export VERSION

DIST="${DIST:-$ROOT/dist/release}"
mkdir -p "$DIST"
export DIST

# Historical binary internals retain CFA4_BUILD_* names; their values are
# always produced by this CFA4 repository (never read from parent release env).
CFA4_BUILD_SHA="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || true)"
export CFA4_BUILD_SHA
CFA4_BUILD_DATE="${CFA4_BUILD_DATE:-$(date -u +%Y-%m-%d)}"
if [[ ! "$CFA4_BUILD_DATE" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]]; then
  echo "error: CFA4_BUILD_DATE must be YYYY-MM-DD" >&2
  exit 2
fi
export CFA4_BUILD_DATE
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

# Emit a GitHub Actions warning (plain stderr outside Actions).
warn() {
  if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::warning::$*"; else echo "warning: $*" >&2; fi
}

# Copy licence and readme files that exist into a package directory, plus the licences of the
# extra-fonts fonts the build embedded (only when built with CFA4_FONTS_DIR).
copy_docs() {
  local dest="$1" f
  for f in README.md LICENSE LICENSE-MIT LICENSE-APACHE NOTICE COPYRIGHT; do
    if [ -f "$ROOT/$f" ]; then cp "$ROOT/$f" "$dest/"; fi
  done
  copy_font_licences "$dest"
}

# Copy each extra-fonts licence ($CFA4_FONTS_DIR/fonts/<family>/OFL.txt) to <dest>/OFL-<family>.txt.
# Does nothing without CFA4_FONTS_DIR.
copy_font_licences() {
  local dest="$1" lic family
  if [ -z "${CFA4_FONTS_DIR:-}" ]; then return 0; fi
  for lic in "$CFA4_FONTS_DIR"/fonts/*/OFL.txt; do
    [ -f "$lic" ] || continue
    family="$(basename "$(dirname "$lic")")"
    cp "$lic" "$dest/OFL-$family.txt"
  done
}

# Portable SHA-256 of a file (prints just the hash).
sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}
