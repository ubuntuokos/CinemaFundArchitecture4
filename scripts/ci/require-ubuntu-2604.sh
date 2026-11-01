#!/usr/bin/env bash
# CFA4 Ubuntu 26.04 / x86_64 host gate; no package installation.
if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
  echo "CFA4 requires Ubuntu 26.04 x86_64 (AMD64)." >&2
  exit 2
fi
if [[ ! -r /etc/os-release ]]; then
  echo "CFA4 cannot verify Ubuntu 26.04 (/etc/os-release unavailable)." >&2
  exit 2
fi
# Read /etc/os-release in subshells: packaging scripts source this file, and os-release's own
# VERSION (e.g. "26.04.1 LTS (Resolute Raccoon)") must not replace the caller's CFA4 VERSION.
_cfa4_os_field() { (. /etc/os-release && eval "printf '%s' \"\${$1:-}\""); }
_cfa4_os_id="$(_cfa4_os_field ID)"
_cfa4_os_id_like="$(_cfa4_os_field ID_LIKE)"
_cfa4_os_version_id="$(_cfa4_os_field VERSION_ID)"
_cfa4_os_pretty="$(_cfa4_os_field PRETTY_NAME)"
# Ubuntu flavours typically report ID=ubuntu. Ubuntu derivatives may declare
# ubuntu as an ID_LIKE token while retaining VERSION_ID=26.04.
if [[ "$_cfa4_os_version_id" != "26.04" ]] || {
  [[ "$_cfa4_os_id" != "ubuntu" ]] && [[ " $_cfa4_os_id_like " != *" ubuntu "* ]]
}; then
  echo "Unsupported CFA4 host: ${_cfa4_os_pretty:-unknown}. Requires Ubuntu 26.04 or a matching flavour, x86_64." >&2
  exit 2
fi
unset -f _cfa4_os_field
unset _cfa4_os_id _cfa4_os_id_like _cfa4_os_version_id _cfa4_os_pretty
