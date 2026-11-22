#!/usr/bin/env bash
# Builds the external test oracles of the MPEG-5 EVC decoder (crates/evc): Samsung's xevd v0.8.0
# (decoder) and xeve v0.7.1 (encoder), BSD-3-Clause, Baseline profile builds (`-DSET_PROF=BASE`)
# from the pinned upstream tags, installed as `xevdb_app` and `xeveb_app`. Nothing here is linked
# or shipped with CFA4. The EVC oracle tests skip when the tools are missing.
set -euo pipefail
xevd_tag=v0.8.0
xevd_commit=8fa744e865e9bdb94a5d5da6a4ab18268cdc4bb4
xeve_tag=v0.7.1
xeve_commit=fe229fec19e14907c60d65626ebd59f41517d9b3
prefix="${1:-/usr/local}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
sudo=""
if [[ ! -w "$prefix" ]]; then
  sudo="sudo"
fi

fetch() { # url tag commit dir
  git -c advice.detachedHead=false clone -q --depth 1 --branch "$2" "$1" "$4"
  local got
  got="$(git -C "$4" rev-parse HEAD)"
  [[ "$got" == "$3" ]] || { echo "$1 $2 is $got, expected $3" >&2; exit 1; }
}
build() { # dir app
  # the version comes from `git describe` or a version.txt; the shallow clone has the tag
  cmake -S "$1" -B "$1/build" -DSET_PROF=BASE -DCMAKE_BUILD_TYPE=Release >/dev/null
  cmake --build "$1/build" -j "$(nproc)" >/dev/null
  find "$1/build" -type f -name "$2" -perm -u+x | head -n 1
}
fetch https://github.com/mpeg5/xevd "$xevd_tag" "$xevd_commit" "$work/xevd"
fetch https://github.com/mpeg5/xeve "$xeve_tag" "$xeve_commit" "$work/xeve"
dec="$(build "$work/xevd" xevdb_app)"
enc="$(build "$work/xeve" xeveb_app)"
[[ -n "$dec" && -n "$enc" ]] || { echo "xevdb_app / xeveb_app not built" >&2; exit 1; }
$sudo mkdir -p "$prefix/bin"
$sudo install -m 0755 "$dec" "$prefix/bin/xevdb_app"
$sudo install -m 0755 "$enc" "$prefix/bin/xeveb_app"
echo "EVC oracle: xevd $xevd_tag, xeve $xeve_tag in $prefix/bin"
