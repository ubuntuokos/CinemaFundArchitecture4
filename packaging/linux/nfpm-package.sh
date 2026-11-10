#!/usr/bin/env bash
# Build one .deb from an nfpm template with the pinned nfpm (check-nfpm.sh, #220).
#   nfpm-package.sh <template.yaml> <depends-file> <output.deb> [NAME=VALUE ...]
# Each NAME=VALUE replaces ${NAME} and @NAME@ in the template (nfpm doesn't expand env vars in
# `src`); the lines of <depends-file> (Ubuntu package names, may be empty) replace the
# `# @FFMPEG_DEPENDS@` line. VERSION and CFA4_MAINTAINER come from the environment (nfpm expands
# them). Runs from the workspace root, where the templates' script paths are relative to.
# Used by package.sh and by the CI package-contents check (scripts/release/deb-contents-check.sh).
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
if [ $# -lt 3 ]; then
  sed -n '2,8p' "$0" >&2
  exit 2
fi
template="$1" depends_file="$2" output="$3"
shift 3
: "${VERSION:?VERSION must be exported (packaging/env.sh)}"
NFPM="$(bash "$HERE/check-nfpm.sh")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cp "$template" "$work/nfpm.yaml"
for pair in "$@"; do
  name="${pair%%=*}" value="${pair#*=}"
  if [[ ! "$name" =~ ^[A-Z][A-Z0-9_]*$ || "$pair" != *=* ]]; then
    echo "error: expected NAME=VALUE, got $pair" >&2
    exit 2
  fi
  NAME="$name" VALUE="$value" python3 -c '
import os, sys
path, name, value = sys.argv[1], os.environ["NAME"], os.environ["VALUE"]
text = open(path, encoding="utf-8").read()
text = text.replace("${" + name + "}", value).replace("@" + name + "@", value)
open(path, "w", encoding="utf-8").write(text)' "$work/nfpm.yaml"
done
depends="$(sed '/^[[:space:]]*$/d; s/^/      - /' "$depends_file")"
deps="$depends" awk '/# @FFMPEG_DEPENDS@/ { if (ENVIRON["deps"] != "") print ENVIRON["deps"]; next } { print }' \
  "$work/nfpm.yaml" >"$work/nfpm.final.yaml"
mkdir -p "$(dirname "$output")"
output="$(cd "$(dirname "$output")" && pwd)/$(basename "$output")"
(cd "$ROOT" && "$NFPM" package -f "$work/nfpm.final.yaml" -p deb -t "$output")
