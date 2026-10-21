#!/usr/bin/env bash
# Pinned FFmpeg 9.0.2 CLI test oracles, isolated from Ubuntu FFmpeg shared libraries.
set -euo pipefail
if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
  echo "CFA4 FFmpeg CI oracle is supported on Linux x86_64 (AMD64) only." >&2
  exit 2
fi
version=9.0.2
archive="ffmpeg-${version}.tar.xz"
source_url="https://ffmpeg.org/releases/${archive}"
sha256=8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e
prefix="${CFA4_FFMPEG_CACHE_DIR:-${HOME}/.cache/cfa4/ffmpeg-9.0.2-linux-x86_64-v1}"
if [[ ! -x "${prefix}/bin/ffmpeg" || ! -x "${prefix}/bin/ffprobe" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  curl --fail --location --show-error --silent --retry 3 \
    --proto '=https' --tlsv1.2 "$source_url" --output "${tmp}/${archive}"
  printf '%s  %s\n' "$sha256" "${tmp}/${archive}" | sha256sum --check
  mkdir -p "${tmp}/src" "$prefix"
  tar -xJf "${tmp}/${archive}" -C "${tmp}/src" --strip-components=1
  (
    cd "${tmp}/src"
    ./configure --prefix="$prefix" --disable-doc --disable-debug --disable-ffplay \
      --disable-autodetect --disable-shared --enable-static
    make -j2
    make install
  )
fi
for exe in ffmpeg ffprobe; do
  if [[ ! -x "${prefix}/bin/${exe}" ]]; then
    echo "::error::Missing pinned ${exe} executable" >&2
    exit 1
  fi
  actual="$("${prefix}/bin/${exe}" -version | awk 'NR==1 {print $3}')"
  if [[ "$actual" != "$version" ]]; then
    echo "::error::Expected ${exe} ${version}, got ${actual}" >&2
    exit 1
  fi
done
if ! "${prefix}/bin/ffmpeg" -hide_banner -decoders |
  grep -Eq '^[[:space:]]*V[^[:space:]]*[[:space:]]+apv([[:space:]]|$)'; then
  echo "::error::Pinned FFmpeg 9.0.2 has no APV decoder" >&2
  exit 1
fi
echo "Verified pinned FFmpeg 9.0.2 with APV:"
"${prefix}/bin/ffmpeg" -version | awk 'NR==1 {print}'
"${prefix}/bin/ffprobe" -version | awk 'NR==1 {print}'
"${prefix}/bin/ffmpeg" -buildconf
if [[ -n "${GITHUB_PATH:-}" ]]; then
  printf '%s\n' "${prefix}/bin" >> "$GITHUB_PATH"
fi
export PATH="${prefix}/bin:$PATH"
