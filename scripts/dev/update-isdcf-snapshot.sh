#!/usr/bin/env bash
# Refresh or check the ISDCF registry snapshot of cfa4-cinema-pkg (CINECERT WP09).
# CFA4 never fetches the registries at run time; this script is the only way the snapshot
# (crates/cinema-pkg/data/isdcf-registry.json) changes, and only when someone runs it.
#
#   bash scripts/dev/update-isdcf-snapshot.sh --check [COMMIT]   reduce upstream COMMIT (default:
#                                                                the snapshot's own) and compare
#   bash scripts/dev/update-isdcf-snapshot.sh --update COMMIT    rewrite the snapshot from COMMIT
#
# After --update: review the diff, update the commit in the .attribution sidecar and in
# ATTRIBUTION.md, and name the commit in the PR.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
snapshot="${repo}/crates/cinema-pkg/data/isdcf-registry.json"
upstream="https://github.com/ISDCF/registries"
cache="${CFA4_ISDCF_CHECKOUT:-${XDG_CACHE_HOME:-${HOME}/.cache}/cfa4/isdcf-registries}"

die() { echo "update-isdcf-snapshot: $*" >&2; exit 1; }

mode="${1:-}"
commit="${2:-}"
case "$mode" in
  --check) [[ -n "$commit" ]] || commit="$(grep -m1 '"commit"' "$snapshot" | sed -E 's/.*"commit": "([0-9a-f]+)".*/\1/')" ;;
  --update) [[ -n "$commit" ]] || die "--update needs an upstream commit (40 hex digits)" ;;
  *) echo "Usage: bash scripts/dev/update-isdcf-snapshot.sh --check [COMMIT] | --update COMMIT" >&2; exit 2 ;;
esac
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || die "not a full commit id: ${commit}"

if [[ ! -d "${cache}/.git" ]]; then
  mkdir -p "$(dirname "$cache")"
  git clone --quiet "$upstream" "$cache"
fi
git -C "$cache" fetch --quiet origin
git -C "$cache" -c advice.detachedHead=false checkout --quiet "$commit"

cd "$repo"
if [[ "$mode" == --update ]]; then
  CFA4_ISDCF_REGISTRIES="$cache" CFA4_ISDCF_WRITE=1 cargo test --quiet -p cfa4-cinema-pkg --test isdcf_snapshot
  echo "Snapshot rewritten from ${commit}; now update the commit in ${snapshot#"${repo}"/}.attribution and ATTRIBUTION.md."
fi
CFA4_ISDCF_REGISTRIES="$cache" cargo test --quiet -p cfa4-cinema-pkg --test isdcf_snapshot
echo "Snapshot matches ${upstream} at ${commit}."
