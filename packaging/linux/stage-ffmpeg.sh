#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Stage CFA4's own FFmpeg 9.0.2 build (scripts/dev/build-ffmpeg-9.0.2.sh, ADR 0005 step 4) for a
# package. Called by package.sh; it never builds, downloads or installs anything.
#
#   stage-ffmpeg.sh --build DIR --dest DIR [--restricted] [--bundle] [--depends-out FILE]
#
#   --build DIR        the installed FFmpeg prefix (default build: /opt/cfa4/ffmpeg-9.0.2)
#   --dest DIR         where DIR/lib and DIR/share/doc are written
#   --restricted       the cfa4-restricted-extras build (x264 and x265 required); without it the
#                      build must not contain any patent-restricted encoder (docs/licensing.md §7)
#   --bundle           also copy the third-party libraries the FFmpeg libraries load (the
#                      relocatable formats: tarball, AppImage, Flatpak), with their copyright
#                      files; libraries every Ubuntu 26.04 desktop and the Flatpak runtime provide
#                      (glibc, libstdc++, zlib, VA-API, DRM, X11/Wayland/GL) stay on the host
#   --depends-out FILE the Ubuntu packages that provide those libraries, one per line (.deb
#                      Depends; glibc and libgcc are already listed in nfpm.yaml)
#
# Only the run-time libraries are staged, each under its SONAME (no development symlinks, headers,
# pkg-config files or programs), with RUNPATH `$ORIGIN`, so the libraries find each other in
# /opt/cfa4/ffmpeg-9.0.2/lib as well as in a relocated bundle.
set -euo pipefail

version=9.0.2
build="" dest="" restricted=0 bundle=0 depends_out=""
while [ $# -gt 0 ]; do
  case "$1" in
    --build) build="${2:?--build needs a directory}"; shift 2 ;;
    --dest) dest="${2:?--dest needs a directory}"; shift 2 ;;
    --restricted) restricted=1; shift ;;
    --bundle) bundle=1; shift ;;
    --depends-out) depends_out="${2:?--depends-out needs a file}"; shift 2 ;;
    -h | --help) sed -n '3,22p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[[ -n "$build" && -n "$dest" ]] || { echo "usage: $0 --build DIR --dest DIR [--restricted] [--bundle] [--depends-out FILE]" >&2; exit 2; }
fail() { echo "error: $*" >&2; exit 1; }
for tool in readelf patchelf ldd dpkg-query; do
  command -v "$tool" >/dev/null || fail "$tool is required to stage FFmpeg (apt: binutils patchelf libc-bin dpkg)"
done

kind=base rebuild="build it with scripts/dev/build-ffmpeg-9.0.2.sh"
if (( restricted )); then kind=restricted rebuild+=" --restricted"; fi
[ -d "$build/lib" ] || fail "no FFmpeg build in $build; $rebuild"
ffmpeg="$build/bin/ffmpeg"
[ -x "$ffmpeg" ] || fail "$ffmpeg is missing; $rebuild"
actual="$("$ffmpeg" -version | awk 'NR==1 {print $3}')"
[[ "$actual" == "$version" || "$actual" == "n$version" ]] || fail "$build is FFmpeg $actual, CFA4 ships $version"
buildconf="$("$ffmpeg" -hide_banner -buildconf)"
for flag in --enable-gpl --enable-version3 --enable-shared; do
  grep -q -- "$flag" <<<"$buildconf" || fail "$build was not configured with $flag (ADR 0005: GPL-3 shared build); $rebuild"
done
if grep -q -- '--enable-nonfree' <<<"$buildconf"; then
  fail "$build is a non-free FFmpeg build, which cannot be distributed"
fi
doc="$build/share/doc/ffmpeg"
for file in LICENSE.md COPYING.GPLv3 BUILDINFO; do
  [ -s "$doc/$file" ] || fail "$doc/$file is missing (licence and source information); $rebuild"
done

needed() { readelf -d "$1" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p'; }
soname() { readelf -d "$1" | sed -n 's/.*(SONAME).*\[\(.*\)\]/\1/p'; }

mapfile -t libs < <(find "$build/lib" -maxdepth 1 -type f -name 'lib*.so*' | sort)
(( ${#libs[@]} )) || fail "$build/lib has no shared libraries; $rebuild"
own=" "
for lib in "${libs[@]}"; do own+="$(soname "$lib") "; done
[[ "$own" == *" libavcodec.so."* && "$own" == *" libavformat.so."* && "$own" == *" libavutil.so."* ]] \
  || fail "$build/lib lacks libavcodec, libavformat or libavutil"

restricted_libs=""
for lib in "${libs[@]}"; do
  restricted_libs+="$(needed "$lib" | grep -E '^libx26[45]\.so' | tr '\n' ' ' || true)"
done
if (( restricted )); then
  for codec in libx264 libx265; do
    [[ "$restricted_libs" == *"$codec.so"* ]] || fail "the restricted build in $build is not linked with $codec"
  done
elif [[ -n "$restricted_libs" ]]; then
  fail "the base build in $build links patent-restricted libraries ($restricted_libs); those belong in cfa4-restricted-extras"
fi

# Libraries that stay on the host. glibc_re: covered by nfpm.yaml's libc6 / libgcc-s1 Depends.
glibc_re='^(linux-vdso|ld-linux|libc\.so|libm\.so|libdl\.so|libpthread\.so|librt\.so|libresolv\.so|libmvec\.so|libgcc_s\.so)'
host_re='^(linux-vdso|ld-linux|libc\.so|libm\.so|libdl\.so|libpthread\.so|librt\.so|libresolv\.so|libmvec\.so|libgcc_s\.so|libstdc\+\+\.so|libz\.so|libva(-[a-z0-9]+)?\.so|libdrm\.so|libX[A-Za-z0-9]*\.so|libxcb[a-z0-9-]*\.so|libwayland-[a-z]+\.so|libGL[A-Za-z]*\.so|libEGL\.so|libvulkan\.so|libOpenCL\.so)'

owner() { # the Ubuntu package that installed a library (usrmerge: try /lib and /usr/lib spellings)
  local path="$1" candidate pkg
  for candidate in "$path" "$(readlink -f -- "$path")" "${path#/usr}" "/usr${path#/usr}"; do
    pkg="$(dpkg-query -S -- "$candidate" 2>/dev/null | grep -v '^diversion' | head -n1 | cut -d: -f1 || true)"
    [ -n "$pkg" ] && { printf '%s\n' "$pkg"; return 0; }
  done
  return 1
}

# name => path for every library the FFmpeg libraries load, directly or not
declare -A resolved=()
for lib in "${libs[@]}"; do
  while read -r name arrow path _; do
    [[ "$arrow" == "=>" ]] || continue
    [[ "$path" == "not" ]] && fail "$(basename "$lib") needs $name, which is not installed"
    resolved["$name"]="$path"
  done < <(ldd "$lib")
done

mkdir -p "$dest/lib" "$dest/share/doc/ffmpeg"
for lib in "${libs[@]}"; do
  name="$(soname "$lib")"
  [ -n "$name" ] || fail "$lib has no SONAME"
  install -m644 -- "$lib" "$dest/lib/$name"
done
cp -- "$doc"/* "$dest/share/doc/ffmpeg/"

declare -A depends=()
for lib in "${libs[@]}"; do
  while read -r name; do
    [[ "$own" == *" $name "* || "$name" =~ $glibc_re ]] && continue
    path="${resolved[$name]:-}"
    [ -n "$path" ] || fail "cannot resolve $name for $(basename "$lib")"
    pkg="$(owner "$path")" || fail "$path ($name) does not come from an Ubuntu package; install its -dev package from the archive and rebuild FFmpeg"
    depends["$pkg"]=1
  done < <(needed "$lib")
done

if (( bundle )); then
  mkdir -p "$dest/share/doc/third-party"
  for name in "${!resolved[@]}"; do
    [[ "$own" == *" $name "* || "$name" =~ $host_re ]] && continue
    path="${resolved[$name]}"
    [[ "$name" =~ ^libx26[45]\.so ]] && fail "refusing to bundle patent-restricted $name"
    install -m644 -- "$(readlink -f -- "$path")" "$dest/lib/$name"
    pkg="$(owner "$path")" || fail "$path ($name) does not come from an Ubuntu package"
    copyright="/usr/share/doc/$pkg/copyright"
    [ -s "$copyright" ] || fail "$copyright is missing; the licence of the bundled $name must ship"
    cp -L -- "$copyright" "$dest/share/doc/third-party/$pkg.copyright"
  done
fi

for lib in "$dest"/lib/*; do
  patchelf --set-rpath '$ORIGIN' "$lib"
done

if [ -n "$depends_out" ]; then
  if (( ${#depends[@]} )); then printf '%s\n' "${!depends[@]}" | sort -u >"$depends_out"; else : >"$depends_out"; fi
fi
echo "staged FFmpeg $version ($kind build, $(find "$dest/lib" -type f | wc -l) libraries) in $dest"
