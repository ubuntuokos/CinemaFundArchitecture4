#!/usr/bin/env bash
# Isolated dcp_inspect reference engine for CFA4 DCP validation development (DCP-01, DCP-04).
#
# The reference engine (wolfgangw/backports, GPL-3.0-or-later; audit:
# docs/audit/DCP-01-SOURCE-LICENSE-AUDIT-2026-10-27.md) is never vendored into this repository.
# It is fetched at one pinned commit into a CFA4-owned cache directory, its Ruby gems go to a
# private GEM_HOME there, and a wrapper runs it with that environment only. Nothing is written to
# the system, the global gem environment or the user's Ruby setup, and no package is installed
# through apt: missing system tools are reported with the command to install them.
#
#   --check        (default) read-only: report prerequisites and the installed engine
#   --install      fetch the pinned commit, verify it and its XSD manifest, install pinned gems
#   --verify       run the engine's own test suite and XSD check in the isolated environment
#   --print-env    print the paths the CFA4 adapter uses (shell-sourceable)
#
# Environment:
#   CFA4_DCP_INSPECT_DIR   cache root (default: ${XDG_CACHE_HOME:-~/.cache}/cfa4/dcp-inspect)
#   CFA4_FFMPEG_CACHE_DIR  pinned FFmpeg 9.0.2 oracle (scripts/ci/install-ffmpeg-9.0.2.sh)
#   CFA4_ASDCPLIB_PREFIX   optional asdcplib install prefix (bin/asdcp-info, bin/asdcp-unwrap)
set -euo pipefail

# Reject non-Ubuntu and non-26.04 hosts, as the other developer scripts do.
. "$(dirname "${BASH_SOURCE[0]}")/../ci/require-ubuntu-2604.sh"

REPO_URL="https://github.com/wolfgangw/backports"
COMMIT="99d0fd863fe05eb806f1625233f2e93676ead268"
ENGINE_VERSION="1.2026.10.03"
# Pinned gems (name version), dependencies first; installed with --ignore-dependencies so the
# isolated GEM_HOME holds exactly this set. racc and bigdecimal build native extensions (ruby-dev,
# build-essential). base64 is a separate gem from Ruby 3.4 on.
GEMS=("racc 1.8.1" "nokogiri 1.19.4" "bigdecimal 3.3.1" "ttfunk 1.8.0")
GEM_BASE64="base64 0.2.0"
FFMPEG_VERSION="9.0.2"

root="${CFA4_DCP_INSPECT_DIR:-${XDG_CACHE_HOME:-${HOME}/.cache}/cfa4/dcp-inspect}"
prefix="${root}/${COMMIT}"
src="${prefix}/src"
gems="${prefix}/gems"
wrapper="${prefix}/bin/cfa4-dcp-inspect"
ffmpeg_bin="${CFA4_FFMPEG_CACHE_DIR:-${HOME}/.cache/cfa4/ffmpeg-9.0.2-linux-x86_64-v6}/bin"
asdcp_bin="${CFA4_ASDCPLIB_PREFIX:+${CFA4_ASDCPLIB_PREFIX}/bin}"

# PATH the engine runs with: the pinned FFmpeg oracle and an optional asdcplib prefix go first,
# so the system FFmpeg is neither used nor changed.
engine_path() {
  local p="${PATH}"
  [[ -d "$ffmpeg_bin" ]] && p="${ffmpeg_bin}:${p}"
  [[ -n "$asdcp_bin" && -d "$asdcp_bin" ]] && p="${asdcp_bin}:${p}"
  printf '%s' "$p"
}

ruby_at_least() { # major minor
  ruby -e "exit(Gem::Version.new(RUBY_VERSION) >= Gem::Version.new('$1.$2') ? 0 : 1)"
}

gem_list() {
  local g
  for g in "${GEMS[@]}"; do printf '%s\n' "$g"; done
  if ruby_at_least 3 4; then printf '%s\n' "$GEM_BASE64"; fi
}

# Run a command with the isolated gem environment and nothing inherited from the user's Ruby setup.
isolated() {
  env -u RUBYOPT -u RUBYLIB -u BUNDLE_GEMFILE -u GEM_HOME -u GEM_PATH \
    GEM_HOME="$gems" GEM_PATH="$gems" GEM_SPEC_CACHE="${prefix}/gem-spec-cache" \
    PATH="$(engine_path)" "$@"
}

status=0
ok() { printf 'OK      %s\n' "$*"; }
miss() { printf 'MISSING %s\n' "$*"; status=1; }
note() { printf 'NOTE    %s\n' "$*"; }

check() {
  if command -v ruby >/dev/null 2>&1; then
    ok "ruby: $(ruby -e 'print RUBY_VERSION') ($(command -v ruby))"
    if ! ruby_at_least 3 0; then miss "ruby >= 3.0 (reviewed with 3.3)"; fi
    if ruby -rrbconfig -e 'exit(File.exist?(File.join(RbConfig::CONFIG["rubyhdrdir"], "ruby.h")) ? 0 : 1)'; then
      ok "ruby headers (native gem extensions)"
    else
      miss "ruby headers for racc/bigdecimal: sudo apt install ruby-dev build-essential"
    fi
  else
    miss "ruby: sudo apt install ruby"
  fi
  for t in git sha256sum mkfifo; do
    if command -v "$t" >/dev/null 2>&1; then ok "$t"; else miss "$t"; fi
  done
  local p
  p="$(engine_path)"
  local ff
  ff="$(PATH="$p" command -v ffmpeg || true)"
  if [[ -n "$ff" ]]; then
    local v
    v="$("$ff" -version 2>/dev/null | awk 'NR==1 {print $3}')"
    if [[ "$v" == "$FFMPEG_VERSION" ]]; then ok "ffmpeg ${v} (${ff})"; else
      miss "ffmpeg ${FFMPEG_VERSION} oracle (found ${v:-unknown} at ${ff}): bash scripts/ci/install-ffmpeg-9.0.2.sh"
    fi
  else
    miss "ffmpeg ${FFMPEG_VERSION} oracle: bash scripts/ci/install-ffmpeg-9.0.2.sh"
  fi
  for t in asdcp-info asdcp-unwrap; do
    if PATH="$p" command -v "$t" >/dev/null 2>&1; then ok "$t ($(PATH="$p" command -v "$t"))"; else
      miss "$t (asdcplib, BSD-3-Clause; set CFA4_ASDCPLIB_PREFIX, see docs/development/dcp-validation.md)"
    fi
  done
  if command -v fd >/dev/null 2>&1 || command -v fdfind >/dev/null 2>&1; then ok "fd/fdfind (optional)"; else
    note "fd/fdfind not found (optional; the engine falls back to its Ruby walker): sudo apt install fd-find"
  fi
  if [[ -d "${src}/.git" ]]; then
    local head
    head="$(git -C "$src" rev-parse HEAD 2>/dev/null || true)"
    if [[ "$head" == "$COMMIT" ]]; then ok "dcp_inspect ${ENGINE_VERSION} at ${src}"; else
      miss "dcp_inspect checkout at ${src} is ${head:-broken}, expected ${COMMIT}: run --install"
    fi
  else
    miss "dcp_inspect ${ENGINE_VERSION}: bash scripts/dev/setup-dcp-inspect.sh --install"
  fi
  if command -v ruby >/dev/null 2>&1; then
    local name version
    while read -r name version; do
      if isolated gem list --installed --exact "$name" --version "$version" >/dev/null 2>&1; then
        ok "gem ${name} ${version} (${gems})"
      else
        miss "gem ${name} ${version} in ${gems}: run --install"
      fi
    done < <(gem_list)
  fi
  if [[ -x "$wrapper" ]]; then ok "wrapper ${wrapper}"; else miss "wrapper ${wrapper}: run --install"; fi
  return "$status"
}

install() {
  command -v ruby >/dev/null 2>&1 || { echo "ruby is required: sudo apt install ruby" >&2; exit 2; }
  command -v git >/dev/null 2>&1 || { echo "git is required" >&2; exit 2; }
  mkdir -p "$prefix" "$gems" "${prefix}/bin"
  if [[ ! -d "${src}/.git" ]]; then
    git init --quiet "$src"
    git -C "$src" remote add origin "$REPO_URL"
  fi
  if [[ "$(git -C "$src" rev-parse HEAD 2>/dev/null || true)" != "$COMMIT" ]]; then
    GIT_LFS_SKIP_SMUDGE=1 git -C "$src" fetch --quiet --depth 1 origin "$COMMIT"
    git -C "$src" checkout --quiet --detach FETCH_HEAD
  fi
  local head
  head="$(git -C "$src" rev-parse HEAD)"
  if [[ "$head" != "$COMMIT" ]]; then
    echo "dcp_inspect checkout is ${head}, expected ${COMMIT}" >&2
    exit 1
  fi
  if [[ "$(tr -d '[:space:]' < "${src}/VERSION")" != "$ENGINE_VERSION" ]]; then
    echo "dcp_inspect VERSION is not ${ENGINE_VERSION}" >&2
    exit 1
  fi
  if [[ -n "$(git -C "$src" status --porcelain)" ]]; then
    echo "dcp_inspect checkout at ${src} has local changes; remove it and run --install again" >&2
    exit 1
  fi
  # The engine's own XSD manifest (21 schemas, audited 2026-10-27).
  (cd "${src}/xsd" && sha256sum --check --quiet MANIFEST.sha256)
  local name version
  while read -r name version; do
    if ! isolated gem list --installed --exact "$name" --version "$version" >/dev/null 2>&1; then
      isolated gem install --install-dir "$gems" --no-document --no-user-install \
        --ignore-dependencies --version "$version" "$name"
    fi
  done < <(gem_list)
  cat > "$wrapper" <<EOF
#!/usr/bin/env bash
# Generated by scripts/dev/setup-dcp-inspect.sh: dcp_inspect ${ENGINE_VERSION} (${COMMIT}),
# isolated gems, pinned FFmpeg ${FFMPEG_VERSION} oracle first on PATH. Read-only on the DCP.
set -euo pipefail
p="\${PATH}"
[[ -d "${ffmpeg_bin}" ]] && p="${ffmpeg_bin}:\${p}"
[[ -n "${asdcp_bin}" && -d "${asdcp_bin}" ]] && p="${asdcp_bin}:\${p}"
exec env -u RUBYOPT -u RUBYLIB -u BUNDLE_GEMFILE \\
  GEM_HOME="${gems}" GEM_PATH="${gems}" PATH="\${p}" \\
  ruby "${src}/dcp_inspect" "\$@"
EOF
  chmod 0755 "$wrapper"
  echo "Installed dcp_inspect ${ENGINE_VERSION} (${COMMIT}) under ${prefix}"
  echo "Run: ${wrapper} --dump-result /tmp/result.json /path/to/DCP"
}

verify() {
  [[ -x "$wrapper" ]] || { echo "Not installed: run --install first" >&2; exit 2; }
  verify_work="$(mktemp -d)"
  trap 'rm -rf "$verify_work"' EXIT
  local work="$verify_work"
  # The test suite writes only temporary files; HOME and TMPDIR point into a scratch directory.
  (cd "$src" && HOME="$work" TMPDIR="$work" isolated ruby -Ilib -S rake xsd:check)
  (cd "$src" && HOME="$work" TMPDIR="$work" isolated ruby -Ilib -S rake test)
  "$wrapper" --version
}

print_env() {
  printf 'CFA4_DCP_INSPECT=%q\n' "$wrapper"
  printf 'CFA4_DCP_INSPECT_COMMIT=%q\n' "$COMMIT"
  printf 'CFA4_DCP_INSPECT_VERSION=%q\n' "$ENGINE_VERSION"
  printf 'CFA4_DCP_INSPECT_XSD=%q\n' "${src}/xsd"
}

case "${1:---check}" in
  --check) check ;;
  --install) install ;;
  --verify) verify ;;
  --print-env) print_env ;;
  *)
    echo "Usage: bash scripts/dev/setup-dcp-inspect.sh [--check|--install|--verify|--print-env]" >&2
    exit 2
    ;;
esac
