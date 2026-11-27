#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Packaging trial for CFA4's minimal OpenUSD core (docs/plans/CFA4-OPENUSD-SOURCEBUILD-001-0.1.md §8, S4, SB-G5).
# Stages the runtime part of an install made by scripts/dev/build-usd.sh (shared libraries and plugin resources, no
# headers, no CMake files) with the licence files, a generated THIRD-PARTY-NOTICES.txt, BUILDINFO.json and SHA256SUMS,
# then writes a deterministic tar.gz and a .deb ("cfa4-usd-worker": USD runtime only, the worker itself comes in U2).
# Nothing in CFA4's cargo, test or release steps calls it. AppImage and Flatpak are NOT done here (see the plan).
#
#   packaging/linux/stage-usd.sh [--prefix DIR] [--src DIR] [--out DIR] [--env FILE]
#
#   --prefix DIR  the build-usd.sh install (default: target/usd-install)
#   --src DIR     the fetched sources (default: target/third-party); their licence files are copied from there
#   --out DIR     where stage/, the tar.gz and the .deb go (default: target/usd-package); must not exist
#   --env FILE    read the pins from FILE instead of packaging/usd-version.env (tests)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
envfile="$root/packaging/usd-version.env"
prefix="$root/target/usd-install" src="$root/target/third-party" out="$root/target/usd-package"
fail() { echo "error: $*" >&2; exit 1; }
usage() { sed -n '3,13p' "$0"; }
while [ $# -gt 0 ]; do
  case "$1" in
    --prefix) prefix="${2:?--prefix needs a directory}"; shift 2 ;;
    --src) src="${2:?--src needs a directory}"; shift 2 ;;
    --out) out="${2:?--out needs a directory}"; shift 2 ;;
    --env) envfile="${2:?--env needs a file}"; shift 2 ;;
    -h | --help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done
prefix="$(realpath -m -- "$prefix")" src="$(realpath -m -- "$src")" out="$(realpath -m -- "$out")"
for tool in tar gzip sha256sum python3 find; do command -v "$tool" >/dev/null || fail "$tool is required"; done
[[ -r "$envfile" ]] || fail "cannot read $envfile"
pin() { local line; line="$(grep -E "^$1=" "$envfile" | tail -n 1 || true)"; [[ -n "$line" ]] || fail "$envfile has no $1"; printf '%s' "${line#*=}"; }
usd_ver="$(pin CFA4_USD_VERSION)" tbb_ver="$(pin CFA4_ONETBB_VERSION)" epoch="$(pin CFA4_USD_SOURCE_DATE_EPOCH)"
[[ -d "$prefix/lib" && -f "$prefix/BUILDINFO.json" && -f "$prefix/SHA256SUMS" ]] || fail "$prefix is not a build-usd.sh install"
usd_src="$src/usd-$usd_ver" tbb_src="$src/onetbb-$tbb_ver"
for f in "$usd_src/LICENSE.txt" "$usd_src/NOTICE.txt" "$tbb_src/LICENSE.txt" "$tbb_src/third-party-programs.txt"; do
  [[ -f "$f" ]] || fail "missing $f (run scripts/dev/fetch-usd-source.sh usd --yes and onetbb --yes)"
done
[[ ! -e "$out" ]] || fail "$out exists; remove it first"

name="cfa4-usd-worker"
inst="opt/cfa4/usd-$usd_ver"
stage="$out/stage"
mkdir -p "$stage/$inst/lib" "$stage/$inst/LICENSES" "$stage/usr/share/doc/$name"
export LC_ALL=C TZ=UTC SOURCE_DATE_EPOCH="$epoch"

# Runtime files only: the shared libraries (with their symlinks), the plugin resources, no headers or CMake files.
(cd "$prefix/lib" && find . \( -name '*.so*' -o -path './usd/*' \) ! -path './cmake/*' ! -path './pkgconfig/*' -print0 | LC_ALL=C sort -z \
  | tar --null --no-recursion -T - -cf - | tar -C "$stage/$inst/lib" -xf -) || fail "copying the libraries failed"
[[ -d "$prefix/plugin" ]] && cp -a "$prefix/plugin" "$stage/$inst/plugin"

# Licences and provenance.
cp "$usd_src/LICENSE.txt" "$stage/$inst/LICENSES/OpenUSD-LICENSE.txt"
cp "$usd_src/NOTICE.txt" "$stage/$inst/LICENSES/OpenUSD-NOTICE.txt"
cp "$tbb_src/LICENSE.txt" "$stage/$inst/LICENSES/oneTBB-LICENSE.txt"
cp "$tbb_src/third-party-programs.txt" "$stage/$inst/LICENSES/oneTBB-third-party-programs.txt"
cp "$prefix/BUILDINFO.json" "$stage/$inst/BUILDINFO.json"
python3 -I - "$usd_src" "$stage/$inst/LICENSES/THIRD-PARTY-NOTICES.txt" <<'PY'
import sys
from pathlib import Path

usd, dest = Path(sys.argv[1]), Path(sys.argv[2])
# Code bundled in the OpenUSD tree and compiled into the libusd_* libraries: the leading licence comment of one file each.
ITEMS = [
    ("RapidJSON", "pxr/base/js/rapidjson/rapidjson.h"),
    ("LZ4", "pxr/base/tf/pxrLZ4/lz4.cpp"),
    ("CLI11", "pxr/base/tf/pxrCLI11/CLI11.h"),
    ("tsl::robin-map", "pxr/base/tf/pxrTslRobinMap/robin_hash.h"),
    ("double-conversion", "pxr/base/tf/pxrDoubleConversion/ieee.h"),
    ("IlmBase half", "pxr/base/gf/ilmbase_half.h"),
    ("PEGTL", "pxr/base/pegtl/pegtl.hpp"),
]


def leading_comment(text: str) -> str:
    if text.startswith("/*"):
        return text[: text.index("*/") + 2]
    lines = []
    for line in text.splitlines():
        if line.startswith("//") or not line.strip():
            lines.append(line)
        else:
            break
    return "\n".join(lines).strip("\n")


parts = ["Third-party code bundled in OpenUSD and compiled into its libraries.", "The text below is copied from each file's own header.", ""]
for label, rel in ITEMS:
    block = leading_comment((usd / rel).read_text(encoding="utf-8", errors="replace"))
    parts += ["=" * 72, f"{label} ({rel})", "=" * 72, block, ""]
dest.write_text("\n".join(parts), encoding="utf-8")
PY

cat > "$stage/usr/share/doc/$name/copyright" <<COPY
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: OpenUSD, oneTBB (CFA4 build)
Source: https://github.com/PixarAnimationStudios/OpenUSD (v$usd_ver), https://github.com/oneapi-src/oneTBB (v$tbb_ver)

Files: /$inst/*
Copyright: Pixar; Intel Corporation; the authors of the bundled code named in THIRD-PARTY-NOTICES.txt
License: TOST-1.0 and Apache-2.0
 The full texts are in /$inst/LICENSES/ (OpenUSD-LICENSE.txt, OpenUSD-NOTICE.txt, oneTBB-LICENSE.txt,
 oneTBB-third-party-programs.txt, THIRD-PARTY-NOTICES.txt). OpenUSD is under the Tomorrow Open Source Technology
 License 1.0 (Apache-2.0 except section 6, trademarks). The provenance of every file is in /$inst/BUILDINFO.json.
COPY

# A hash list of the staged files, then everything gets the pinned commit time.
(cd "$stage" && find . -type f ! -name SHA256SUMS -print0 | LC_ALL=C sort -z | xargs -0 sha256sum > "$inst/SHA256SUMS")
find "$stage" -print0 | xargs -0 touch -h -d "@$epoch"

# The tar.gz (deterministic: sorted, fixed owner and time, no gzip name).
tgz="$out/$name-$usd_ver-x86_64.tar.gz"
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$epoch" -C "$stage" -cf - "$inst" "usr" | gzip -n -9 > "$tgz"

# The .deb, only when dpkg-deb is there.
deb=""
if command -v dpkg-deb >/dev/null; then
  mkdir -p "$stage/DEBIAN"
  size_kb="$(du -sk "$stage/$inst" | cut -f1)"
  cat > "$stage/DEBIAN/control" <<CTRL
Package: $name
Version: $usd_ver-1
Architecture: amd64
Section: video
Priority: optional
Maintainer: ${CFA4_MAINTAINER:-CFA4 contributors}
Installed-Size: $size_kb
Depends: libc6, libstdc++6
Description: CFA4 OpenUSD runtime (packaging trial)
 OpenUSD $usd_ver and oneTBB $tbb_ver built from pinned sources, minimal core (no Python, imaging, MaterialX,
 OpenVDB or OpenGL). The USD worker itself is not in this package yet.
CTRL
  find "$stage/DEBIAN" -print0 | xargs -0 touch -h -d "@$epoch"
  deb="$out/${name}_${usd_ver}-1_amd64.deb"
  dpkg-deb --root-owner-group -Zxz --build "$stage" "$deb" > /dev/null
fi
echo "staged in $stage"
echo "tar.gz: $tgz ($(stat -c %s "$tgz") bytes)"
[[ -n "$deb" ]] && echo "deb:    $deb ($(stat -c %s "$deb") bytes)"
echo "installed size: $(du -sk "$stage/$inst" | cut -f1) KiB"
