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
. /etc/os-release
# Ubuntu flavours typically report ID=ubuntu. Ubuntu derivatives may declare
# ubuntu as an ID_LIKE token while retaining VERSION_ID=26.04.
if [[ "${VERSION_ID:-}" != "26.04" ]] || {
  [[ "${ID:-}" != "ubuntu" ]] && [[ " ${ID_LIKE:-} " != *" ubuntu "* ]]
}; then
  echo "Unsupported CFA4 host: ${PRETTY_NAME:-unknown}. Requires Ubuntu 26.04 or a matching flavour, x86_64." >&2
  exit 2
fi
