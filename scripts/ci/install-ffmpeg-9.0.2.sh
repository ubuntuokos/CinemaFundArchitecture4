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
prefix="${CFA4_FFMPEG_CACHE_DIR:-${HOME}/.cache/cfa4/ffmpeg-9.0.2-linux-x86_64-v6}"
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
      --disable-autodetect --disable-shared --enable-static --enable-gpl --enable-zlib \
      --enable-libdav1d --enable-libsvtav1 --enable-libaom \
      --enable-libx264 --enable-libx265 --enable-libvpx --enable-libmp3lame --enable-libopus --enable-libwebp
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
# Keep this comprehensive matrix in sync with the external FFmpeg calls in
# apps/cfa4-cli/tests and crates/*/tests (including their common fixture helpers).
# Check every dependency in one pass, rather than discovering missing codecs
# only after a long Rust release-test run.
video_encoders=(libx264 libx265 libvpx libvpx-vp9 libsvtav1 libaom-av1 libwebp mpeg2video mjpeg prores_ks dnxhd rawvideo png)
audio_encoders=(libmp3lame libopus aac ac3 eac3 mp2 flac vorbis pcm_s16le pcm_s24le pcm_dvd)
video_decoders=(apv libdav1d h264 hevc vp8 vp9 mpeg2video prores dnxhd png webp)
audio_decoders=(aac ac3 eac3 mp3 mp2 opus vorbis flac)
encoders="$("${prefix}/bin/ffmpeg" -hide_banner -encoders)"
decoders="$("${prefix}/bin/ffmpeg" -hide_banner -decoders)"
missing=()
check_codecs() {
  local listing="$1" kind="$2" label="$3"
  shift 3
  local codec
  for codec in "$@"; do
    if ! awk -v kind="$kind" -v codec="$codec" '$1 ~ ("^" kind) && $2 == codec { found=1 } END { exit !found }' <<< "$listing"; then
      missing+=("$label: $codec")
    fi
  done
}
check_codecs "$encoders" V "video encoder" "${video_encoders[@]}"
check_codecs "$encoders" A "audio encoder" "${audio_encoders[@]}"
check_codecs "$decoders" V "video decoder" "${video_decoders[@]}"
check_codecs "$decoders" A "audio decoder" "${audio_decoders[@]}"
if (( ${#missing[@]} > 0 )); then
  echo "::error::Pinned FFmpeg ${version} is missing ${#missing[@]} required oracle codecs:" >&2
  printf '::error::  %s\n' "${missing[@]}" >&2
  exit 1
fi
echo "Verified pinned FFmpeg ${version}: ${#video_encoders[@]} video encoders, ${#audio_encoders[@]} audio encoders, ${#video_decoders[@]} video decoders and ${#audio_decoders[@]} audio decoders."
"${prefix}/bin/ffmpeg" -version | awk 'NR==1 {print}'
"${prefix}/bin/ffprobe" -version | awk 'NR==1 {print}'
"${prefix}/bin/ffmpeg" -buildconf
if [[ -n "${GITHUB_PATH:-}" ]]; then
  printf '%s\n' "${prefix}/bin" >> "$GITHUB_PATH"
fi
export PATH="${prefix}/bin:$PATH"
