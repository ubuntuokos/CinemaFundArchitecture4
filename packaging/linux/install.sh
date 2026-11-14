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
# the Qt GUI's launcher comes with its Python package: checked before anything is written
if [ -f "$source_dir/bin/cfa4-qt" ] && [ ! -f "$source_dir/lib/cfa4/python/cfa4_qt/__main__.py" ]; then
  fail 'missing lib/cfa4/python/cfa4_qt; extract the complete tarball first'
fi

mkdir -p -- "$prefix/bin" "$prefix/share"
install -m755 -- "$source_dir/bin/cfa4" "$prefix/bin/cfa4"
install -m755 -- "$source_dir/bin/cfa4-cli" "$prefix/bin/cfa4-cli"
cp -R -- "$source_dir/share/." "$prefix/share/"
# The Qt 6 GUI: its launcher finds the Python package in ../lib/cfa4/python.
if [ -f "$source_dir/bin/cfa4-qt" ]; then
  install -m755 -- "$source_dir/bin/cfa4-qt" "$prefix/bin/cfa4-qt"
  rm -rf -- "$prefix/lib/cfa4/python/cfa4_qt"
  mkdir -p -- "$prefix/lib/cfa4/python"
  cp -R -- "$source_dir/lib/cfa4/python/cfa4_qt" "$prefix/lib/cfa4/python/"
fi
# CFA4's FFmpeg 9.0.2 (ADR 0005): the binaries find it in ../lib/cfa4/ffmpeg-9.0.2 ($ORIGIN RUNPATH)
# when /opt/cfa4 has none. Replaced as a whole, so no library of an older release stays behind.
ffmpeg_dir=lib/cfa4/ffmpeg-9.0.2
if [ -d "$source_dir/$ffmpeg_dir/lib" ]; then
  rm -rf -- "${prefix:?}/$ffmpeg_dir"
  mkdir -p -- "$prefix/lib/cfa4"
  cp -R -- "$source_dir/$ffmpeg_dir" "$prefix/$ffmpeg_dir"
fi

# Use an absolute path so launching from the application menu does not depend on PATH.
# Escape the Exec argument, then the desktop entry's string value (two separate layers).
# https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html
# The menu entry starts the Qt 6 GUI (owner decision 2026-11-12); a tarball without it keeps egui.
gui=cfa4
if [ -f "$source_dir/bin/cfa4-qt" ]; then
  gui=cfa4-qt
fi
exec_path="$(printf '%s' "$prefix/bin/$gui" | sed 's/[\\"`$]/\\&/g; s/\\/\\\\/g')"
try_path="$(printf '%s' "$prefix/bin/$gui" | sed 's/\\/\\\\/g')"
while IFS= read -r line || [ -n "$line" ]; do
  case "$line" in
    Exec=*) printf 'Exec="%s" %%F\n' "$exec_path" ;;
    TryExec=*) printf 'TryExec=%s\n' "$try_path" ;;
    *) printf '%s\n' "$line" ;;
  esac
done <"$source_dir/share/applications/$app_id.desktop" >"$prefix/share/applications/$app_id.desktop"
# Credits files open in the Qt GUI (cfa4-qt): its entry with the absolute launcher, or none
# when this tarball has no Qt GUI.
credits_entry="$prefix/share/applications/$app_id.credits.desktop"
if [ -f "$source_dir/bin/cfa4-qt" ] && [ -f "$source_dir/share/applications/$app_id.credits.desktop" ]; then
  qt_exec="$(printf '%s' "$prefix/bin/cfa4-qt" | sed 's/[\\"`$]/\\&/g; s/\\/\\\\/g')"
  qt_try="$(printf '%s' "$prefix/bin/cfa4-qt" | sed 's/\\/\\\\/g')"
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      Exec=*) printf 'Exec="%s" %%f\n' "$qt_exec" ;;
      TryExec=*) printf 'TryExec=%s\n' "$qt_try" ;;
      *) printf '%s\n' "$line" ;;
    esac
  done <"$source_dir/share/applications/$app_id.credits.desktop" >"$credits_entry"
else
  rm -f -- "$credits_entry"
fi

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
# The few libraries the tarball leaves to the system (VA-API, DRM, zlib, libstdc++, GStreamer):
# name what a minimal system lacks instead of failing at the first start.
if command -v ldd >/dev/null 2>&1; then
  missing="$(ldd "$prefix/bin/cfa4" 2>/dev/null | awk '$2 == "=>" && $3 == "not" {print $1}' | sort -u | tr '\n' ' ' || true)"
  if [ -n "$missing" ]; then
    echo "warning: CFA4 needs these libraries from Ubuntu's archive: $missing" >&2
    echo "  e.g. sudo apt install libva2 libva-drm2 libdrm2 libgstreamer1.0-0 libgstreamer-plugins-base1.0-0" >&2
  fi
fi
printf 'Installed CFA4 in %s\nLaunch it from the application menu or run "%s/bin/cfa4".\n' "$prefix" "$prefix"
case ":${PATH:-}:" in
  *":$prefix/bin:"*) ;;
  *) printf 'For terminal commands, add %s/bin to your PATH.\n' "$prefix" ;;
esac
