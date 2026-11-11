#!/usr/bin/env bash
# Build and install the pinned nfpm (packaging/linux/nfpm.lock) for CFA4 .deb packaging (#220).
#   install-nfpm.sh [--dest DIR]    default: ${CFA4_TOOLS_DIR:-~/.cache/cfa4/tools}/nfpm-<version>
# Explicit provisioning, never called by the packaging scripts. Needs `go` (1.21 or newer, e.g.
# Ubuntu's golang-go) and network access to proxy.golang.org / sum.golang.org: Go fetches the
# pinned toolchain and the module, both checked against the checksum database, and this script
# also compares the module hash and the built binary's SHA-256 with nfpm.lock before installing.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LOCK="$ROOT/packaging/linux/nfpm.lock"
lock() { sed -n "s/^$1=\\([^[:space:]]*\\)\$/\\1/p" "$LOCK"; }
version="$(lock CFA4_NFPM_VERSION)"
module_sum="$(lock CFA4_NFPM_MODULE_SUM)"
toolchain="$(lock CFA4_NFPM_GO_TOOLCHAIN)"
expected="$(lock CFA4_NFPM_SHA256)"
module=github.com/goreleaser/nfpm/v2

dest="${CFA4_TOOLS_DIR:-$HOME/.cache/cfa4/tools}/nfpm-$version"
while [ $# -gt 0 ]; do
  case "$1" in
    --dest) dest="$2"; shift 2 ;;
    -h | --help) sed -n '2,7p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
if [[ "$(uname -s)" != Linux || "$(uname -m)" != x86_64 ]]; then
  echo "CFA4 packages only on Linux x86_64 (AMD64)." >&2
  exit 2
fi
command -v go >/dev/null || { echo "error: go is required (apt: golang-go)" >&2; exit 2; }

tmp="$(mktemp -d)"
# Go writes its module cache read-only (the toolchain ignores -modcacherw)
trap 'chmod -R u+w "$tmp" 2>/dev/null; rm -rf "$tmp"' EXIT
# A private, empty module cache and the public proxy and checksum database, whatever the
# caller's Go environment says; the pinned toolchain and a reproducible, static build.
export GOPATH="$tmp/gopath" GOMODCACHE="$tmp/gopath/mod" GOCACHE="$tmp/gocache" GOBIN="$tmp/bin"
export GOPROXY=https://proxy.golang.org GOSUMDB=sum.golang.org GONOSUMDB= GONOSUMCHECK= GOPRIVATE= \
  GONOPROXY= GOINSECURE= GOFLAGS=-modcacherw GOENV=off GOEXPERIMENT= GOTOOLCHAIN="$toolchain" \
  CGO_ENABLED=0 GOOS=linux GOARCH=amd64 GOAMD64=v1
cd "$tmp"
sum="$(go mod download -json "$module@$version" | python3 -c 'import json, sys; print(json.load(sys.stdin)["Sum"])')"
if [[ "$sum" != "$module_sum" ]]; then
  echo "error: $module@$version has module hash $sum, nfpm.lock pins $module_sum" >&2
  exit 1
fi
go install -trimpath -ldflags "-s -w -buildid= -X main.version=$version -X main.builtBy=cfa4" \
  "$module/cmd/nfpm@$version"
actual="$(sha256sum -- "$GOBIN/nfpm" | awk '{print $1}')"
if [[ "$actual" != "$expected" ]]; then
  echo "error: the built nfpm has SHA-256 $actual, nfpm.lock pins $expected (not reproducible here?)" >&2
  exit 1
fi
install -Dm755 "$GOBIN/nfpm" "$dest/nfpm"
echo "installed nfpm $version (sha256 $actual) to $dest/nfpm"
echo "packaging/linux/check-nfpm.sh finds it there by default; NFPM=$dest/nfpm names it explicitly."
