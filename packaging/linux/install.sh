#!/usr/bin/env bash
# Install the extracted Linux tarball. No sudo is needed for the default prefix.
set -euo pipefail
if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
  echo "CFA4 installation supports Ubuntu 26.04 Linux x86_64 (AMD64) only." >&2
  exit 2
fi
# Included in the standalone tarball: must not depend on repository scripts.
if [[ ! -r /etc/os-release ]]; then
  echo "CFA4 cannot verify the Ubuntu 26.04 target platform." >&2
  exit 2
fi
. /etc/os-release
if [[ "${VERSION_ID:-}" != "26.04" ]] || {
  [[ "${ID:-}" != "ubuntu" ]] && [[ " ${ID_LIKE:-} " != *" ubuntu "* ]]
}; then
  echo "Unsupported CFA4 installation host: ${PRETTY_NAME:-unknown}. Requires Ubuntu 26.04 x86_64." >&2
  exit 2
fi

usage() {
  echo 'Usage: ./install.sh [--prefix /absolute/path]'
  echo 'Installs CFA4 and its desktop integration into ~/.local by default.'
  echo 'For all users: sudo ./install.sh --prefix /usr/local'
}
fail() { echo "error: $*" >&2; exit 1; }
prefix="${HOME:?HOME is not set}/.local"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --prefix)
      [ "$#" -ge 2 ] || fail '--prefix needs an absolute path'
      prefix="$2"; shift 2 ;;
    -h | --help) usage; exit 0 ;;
    *) usage >&2; fail "unknown argument: $1" ;;
  esac
done
case "$prefix" in /*) ;; *) fail '--prefix must be an absolute path' ;; esac
prefix="$(realpath -m -- "$prefix")"
# Desktop Exec paths cannot contain '='; launchers also misinterpret '%' in executable names.
[[ "$prefix" != *'='* && "$prefix" != *'%'* && ! "$prefix" =~ [[:cntrl:]] ]] || fail 'prefix cannot contain =, % or control characters'
[ "$prefix" != / ] || fail 'use /usr/local or a user directory, not /'
source_dir="$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)"
case "$prefix/" in "$source_dir/"*) fail 'choose an installation prefix outside the extracted tarball' ;; esac
app_id=io.github.ubuntuokos.cfa4
for binary in cfa4 cfa4-cli; do
  [ -f "$source_dir/bin/$binary" ] || fail "missing bin/$binary; extract the complete tarball first"
done
[ -f "$source_dir/share/applications/$app_id.desktop" ] || fail 'missing desktop entry; extract the complete tarball first'

mkdir -p -- "$prefix/bin" "$prefix/share"
install -m755 -- "$source_dir/bin/cfa4" "$prefix/bin/cfa4"
install -m755 -- "$source_dir/bin/cfa4-cli" "$prefix/bin/cfa4-cli"
cp -R -- "$source_dir/share/." "$prefix/share/"

# Use an absolute path so launching from the application menu does not depend on PATH.
# Escape the Exec argument, then the desktop entry's string value (two separate layers).
# https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html
exec_path="$(printf '%s' "$prefix/bin/cfa4" | sed 's/[\\"`$]/\\&/g; s/\\/\\\\/g')"
try_path="$(printf '%s' "$prefix/bin/cfa4" | sed 's/\\/\\\\/g')"
while IFS= read -r line || [ -n "$line" ]; do
  case "$line" in
    Exec=*) printf 'Exec="%s" %%F\n' "$exec_path" ;;
    TryExec=*) printf 'TryExec=%s\n' "$try_path" ;;
    *) printf '%s\n' "$line" ;;
  esac
done <"$source_dir/share/applications/$app_id.desktop" >"$prefix/share/applications/$app_id.desktop"

# Cache tools are optional; installing binaries must also work on minimal systems.
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database -q "$prefix/share/applications" || echo 'warning: desktop cache refresh failed' >&2
fi
if command -v update-mime-database >/dev/null 2>&1; then
  update-mime-database "$prefix/share/mime" || echo 'warning: MIME cache refresh failed' >&2
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q -t -f "$prefix/share/icons/hicolor" || echo 'warning: icon cache refresh failed' >&2
fi
printf 'Installed CFA4 in %s\nLaunch it from the application menu or run "%s/bin/cfa4".\n' "$prefix" "$prefix"
case ":${PATH:-}:" in
  *":$prefix/bin:"*) ;;
  *) printf 'For terminal commands, add %s/bin to your PATH.\n' "$prefix" ;;
esac
