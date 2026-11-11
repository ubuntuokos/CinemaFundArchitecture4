#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Installs CFA4's prebuilt FFmpeg 9.0.2 base build into /opt/cfa4/ffmpeg-9.0.2 instead of compiling
# it: downloads the archive for this Ubuntu release from the GitHub prerelease made by
# .github/workflows/cfa4-ffmpeg-prebuilt.yml, checks it against the SHA-256 pinned below and
# extracts it. It is the same build scripts/dev/build-ffmpeg-9.0.2.sh makes (GPL-3 or later,
# shared libraries, no nonfree, no x264/x265); its BUILDINFO and licences are in
# /opt/cfa4/ffmpeg-9.0.2/share/doc/ffmpeg/.
#
#   scripts/dev/fetch-ffmpeg-9.0.2.sh            exit 0: installed (or already there)
#                                                exit 3: no prebuilt archive for this host
#
# Hosts: Ubuntu 26.04 (CFA4 target, CI, packaging) and Ubuntu 24.04 (cloud development
# containers), Linux x86_64. Other hosts build from source with scripts/dev/build-ffmpeg-9.0.2.sh.
# The prefix is fixed: the libraries carry an /opt/cfa4/ffmpeg-9.0.2/lib RUNPATH.
set -euo pipefail

version=9.0.2
release_tag=ffmpeg-9.0.2-cfa4-1
base_url="https://github.com/ubuntuokos/CinemaFundArchitecture4/releases/download/${release_tag}"
prefix=/opt/cfa4/ffmpeg-${version}

if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
  echo "No prebuilt FFmpeg ${version} for $(uname -s) $(uname -m)." >&2
  exit 3
fi
host_id="$(. /etc/os-release 2>/dev/null && echo "${ID:-}:${VERSION_ID:-}")"
case "$host_id" in
  ubuntu:26.04)
    asset=cfa4-ffmpeg-9.0.2-ubuntu26.04-x86_64.tar.xz
    sha256=b43d88a3e07edaf60675bf1051ca3599be88ea7d6ec5845a142ad72c7c7501b3
    ;;
  ubuntu:24.04)
    asset=cfa4-ffmpeg-9.0.2-ubuntu24.04-x86_64.tar.xz
    sha256=4d50fe1d66204ad42cd5aa15d25882fdaae3146710fb75e23935200a89e29843
    ;;
  *)
    echo "No prebuilt FFmpeg ${version} for ${host_id:-this host}; build it with scripts/dev/build-ffmpeg-9.0.2.sh." >&2
    exit 3
    ;;
esac

installed() {
  [[ -x "$prefix/bin/ffmpeg" && -s "$prefix/lib/pkgconfig/libavcodec.pc" && -s "$prefix/share/doc/ffmpeg/BUILDINFO" ]] \
    && [[ "$("$prefix/bin/ffmpeg" -version 2>/dev/null | awk 'NR==1 {print $3}')" == "$version" ]]
}
if installed; then
  echo "FFmpeg ${version} is already installed in ${prefix}."
  exit 0
fi
if [[ -e "$prefix" ]]; then
  echo "$prefix exists but is not a complete FFmpeg ${version} build; remove it first." >&2
  exit 1
fi
command -v curl >/dev/null || { echo "missing tool: curl" >&2; exit 1; }
command -v xz >/dev/null || { echo "missing tool: xz (apt: xz-utils)" >&2; exit 1; }

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl --fail --location --show-error --silent --retry 5 --retry-all-errors --retry-delay 5 \
  --proto '=https' --tlsv1.2 "${base_url}/${asset}" --output "${tmp}/${asset}"
printf '%s  %s\n' "$sha256" "${tmp}/${asset}" | sha256sum --check --quiet
# The archive holds exactly one top-level directory, ffmpeg-9.0.2/; refuse anything else.
if tar -tJf "${tmp}/${asset}" | grep -Ev "^ffmpeg-${version}(/|$)" | grep -q .; then
  echo "unexpected paths in ${asset}; refusing to extract" >&2
  exit 1
fi

parent="$(dirname "$prefix")"
run=()
if [[ ! -d "$parent" ]]; then
  mkdir -p "$parent" 2>/dev/null || { sudo mkdir -p "$parent"; }
fi
if [[ ! -w "$parent" && "$(id -u)" != 0 ]]; then
  run=(sudo)
fi
"${run[@]}" tar -xJf "${tmp}/${asset}" -C "$parent" --no-same-owner

if ! installed; then
  echo "the extracted FFmpeg in ${prefix} does not run; build it with scripts/dev/build-ffmpeg-9.0.2.sh" >&2
  exit 1
fi
missing="$(ldd "$prefix/bin/ffmpeg" | awk '$2 == "=>" && $3 == "not" {print $1}' | tr '\n' ' ')"
if [[ -n "$missing" ]]; then
  echo "warning: libraries missing on this host: ${missing}(see ${prefix}/share/doc/ffmpeg/HOST)" >&2
fi
echo "FFmpeg ${version} installed in ${prefix} from ${release_tag} (${asset}, SHA-256 verified)."
