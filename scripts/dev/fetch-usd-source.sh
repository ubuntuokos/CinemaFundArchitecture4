#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Fetches (by hand) or checks the pinned source of CFA4's 3D donors from packaging/usd-version.env
# (docs/plans/CFA4-PIXAR-DONOR-001-EGYESITETT-1.1.md, G-USD-1 path (b)). Nothing in CFA4's cargo, test, CI or
# install steps calls it: a person (or an agent working for one) runs it.
#
#   scripts/dev/fetch-usd-source.sh NAME [--yes] [--dest DIR]
#   scripts/dev/fetch-usd-source.sh NAME --from-tarball FILE [--dest DIR]
#
#   NAME             usd | opensubdiv | onetbb
#   --yes            really download; without it the script only says what it would do (exit 2)
#   --from-tarball   no network: check FILE (the uncompressed `git archive --format=tar` of the pinned commit,
#                    or that tar compressed as .gz/.xz/.zst) against the pinned SHA-256 and unpack it
#   --dest DIR       where the source goes (default: target/third-party); it lands in DIR/NAME-VERSION
#   --env FILE       read the pins from FILE instead of packaging/usd-version.env (tests)
#   --skip-tag-signature  do not check the signed tag (only if ssh-keygen is missing; says so loudly)
#
# The source is fetched with `git fetch --depth 1 <url> <commit>` (https only, no prompts), the fetched commit
# must be the pinned one, and the `git archive` of it must have the pinned SHA-256 and licence hashes. The
# tree is unpacked from that archive (no .git directory). Where the pins name a tag object and a signers file
# (OpenUSD), the tag is fetched too: its object id must be the pinned one and `git tag -v` must accept it
# against packaging/usd-allowed-signers (needs ssh-keygen; trust level "tofu", see usd-version.env).
# A mismatch stops the script and leaves nothing behind.
# The unpacked source is untrusted data: do not run scripts or interpreters from inside it.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
envfile="$root/packaging/usd-version.env"
name="" assume_yes=0 dest="$root/target/third-party" tarball="" skip_sig=0
fail() { echo "error: $*" >&2; exit 1; }
usage() { sed -n '3,24p' "$0"; }

while [ $# -gt 0 ]; do
  case "$1" in
    usd | opensubdiv | onetbb) [[ -z "$name" ]] || fail "only one NAME"; name="$1"; shift ;;
    --yes) assume_yes=1; shift ;;
    --dest) dest="${2:?--dest needs a directory}"; shift 2 ;;
    --from-tarball) tarball="${2:?--from-tarball needs a file}"; shift 2 ;;
    --skip-tag-signature) skip_sig=1; shift ;;
    --env) envfile="${2:?--env needs a file}"; shift 2 ;;
    -h | --help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done
[[ -n "$name" ]] || { usage >&2; exit 2; }
[[ -r "$envfile" ]] || fail "cannot read $envfile"

# Read KEY=VALUE lines without sourcing the file (no code runs from it).
pin() {
  local key="$1" line
  line="$(grep -E "^${key}=" "$envfile" | tail -n 1 || true)"
  [[ -n "$line" ]] || fail "$envfile has no $key"
  printf '%s' "${line#*=}"
}
# Optional pin: empty when the line is absent.
opt_pin() { local line; line="$(grep -E "^$1=" "$envfile" | tail -n 1 || true)"; printf '%s' "${line#*=}"; }
prefix="CFA4_$(printf '%s' "$name" | tr '[:lower:]' '[:upper:]')"
version="$(pin "${prefix}_VERSION")"
url="$(pin "${prefix}_GIT_URL")"
commit="$(pin "${prefix}_GIT_COMMIT")"
want_sha="$(pin "${prefix}_SOURCE_SHA256")"
want_license="$(pin "${prefix}_LICENSE_SHA256")"
want_notice="$(pin "${prefix}_NOTICE_SHA256")"
verified="$(pin "${prefix}_SIGNATURE_VERIFIED")"
tag_name="$(opt_pin "${prefix}_GIT_TAG")" tag_object="$(opt_pin "${prefix}_TAG_OBJECT")"
signers="$(opt_pin "${prefix}_ALLOWED_SIGNERS")"
[[ -z "$tag_object" || "$tag_object" =~ ^[0-9a-f]{40}$ ]] || fail "${prefix}_TAG_OBJECT is not a 40-digit object id"
[[ -z "$signers" || ( "$signers" != /* && "$signers" != *..* ) ]] || fail "${prefix}_ALLOWED_SIGNERS must be a relative path"
sig_note="tag signature not checked"
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || fail "${prefix}_GIT_COMMIT is not a 40-digit commit"
[[ "$want_sha" =~ ^[0-9a-f]{64}$ ]] || fail "${prefix}_SOURCE_SHA256 is not a SHA-256"
[[ "$url" =~ ^https://[A-Za-z0-9._/-]+$ ]] || fail "${prefix}_GIT_URL must be a plain https URL"
target="$dest/$name-$version"

if [[ -z "$tarball" && $assume_yes -ne 1 ]]; then
  cat >&2 <<EOF
Would download $name $version: git fetch --depth 1 $url $commit
  check:  git archive SHA-256 = $want_sha
  into:   $target
  tag signature trust level: $verified (see packaging/usd-version.env)
Nothing was downloaded. Run again with --yes to do it, or use --from-tarball FILE (no network).
EOF
  exit 2
fi
[[ ! -e "$target" ]] || fail "$target exists; remove it first"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
archive="$tmp/source.tar"

if [[ -n "$tarball" ]]; then
  [[ -f "$tarball" ]] || fail "$tarball is not a file"
  case "$tarball" in
    *.tar) cp -- "$tarball" "$archive" ;;
    *.tar.gz | *.tgz) gzip -dc -- "$tarball" > "$archive" ;;
    *.tar.xz) xz -dc -- "$tarball" > "$archive" ;;
    *.tar.zst) zstd -dc -- "$tarball" > "$archive" ;;
    *) fail "unknown tarball type: $tarball (.tar, .tar.gz, .tar.xz or .tar.zst)" ;;
  esac
else
  command -v git >/dev/null || fail "git is required"
  mkdir "$tmp/repo"
  (
    cd "$tmp/repo"
    export GIT_TERMINAL_PROMPT=0 GIT_ALLOW_PROTOCOL=https
    git init -q .
    git remote add origin "$url"
    git fetch -q --depth 1 origin "$commit"
    git -c advice.detachedHead=false checkout -q FETCH_HEAD
    [[ "$(git rev-parse HEAD)" == "$commit" ]] || { echo "error: fetched commit is not $commit" >&2; exit 1; }
    git archive --format=tar HEAD > "$archive"
    if [[ -n "$tag_object" && -n "$signers" && $skip_sig -eq 0 ]]; then
      command -v ssh-keygen >/dev/null || { echo "error: ssh-keygen is needed to check the tag signature (or pass --skip-tag-signature)" >&2; exit 1; }
      [[ -r "$root/$signers" ]] || { echo "error: cannot read $root/$signers" >&2; exit 1; }
      git fetch -q --depth 1 origin "refs/tags/$tag_name:refs/tags/$tag_name"
      [[ "$(git rev-parse "refs/tags/$tag_name")" == "$tag_object" ]] || { echo "error: tag $tag_name is not the pinned object $tag_object" >&2; exit 1; }
      [[ "$(git rev-parse "refs/tags/$tag_name^{commit}")" == "$commit" ]] || { echo "error: tag $tag_name does not point at $commit" >&2; exit 1; }
      git -c gpg.format=ssh -c gpg.ssh.program=ssh-keygen -c "gpg.ssh.allowedSignersFile=$root/$signers" tag -v "$tag_name" >/dev/null 2>"$tmp/tagverify.txt" \
        || { cat "$tmp/tagverify.txt" >&2; echo "error: tag signature check failed" >&2; exit 1; }
      echo "ok" > "$tmp/tag-ok"
    fi
  ) || fail "could not fetch $url at $commit"
fi

[[ -f "$tmp/tag-ok" ]] && sig_note="tag $tag_name signature checked against $signers (trust: $verified)"
got_sha="$(sha256sum "$archive" | cut -d' ' -f1)"
[[ "$got_sha" == "$want_sha" ]] || fail "source SHA-256 mismatch: got $got_sha, pinned $want_sha"

# Unpack only plain relative paths (no absolute names, no `..`, no links that leave the tree).
if tar -tf "$archive" | grep -Eq '(^/|(^|/)\.\.(/|$))'; then
  fail "the archive holds absolute or parent paths; refusing to unpack"
fi
mkdir -p "$tmp/out"
tar -xf "$archive" -C "$tmp/out" --no-same-owner --no-same-permissions
if find "$tmp/out" -type l | while read -r l; do
  t="$(readlink -m -- "$l")"; case "$t" in "$tmp/out"/*) ;; *) echo bad ;; esac
done | grep -q bad; then
  fail "the archive holds a link that leaves the tree; refusing to unpack"
fi

check_file() { # file expected-hash
  local f="$tmp/out/$1" want="$2"
  [[ -z "$want" ]] && return 0
  [[ -f "$f" ]] || fail "$1 is missing from the source"
  [[ "$(sha256sum "$f" | cut -d' ' -f1)" == "$want" ]] || fail "$1 has not the pinned SHA-256"
}
check_file LICENSE.txt "$want_license"
check_file NOTICE.txt "$want_notice"

mkdir -p "$dest"
mv "$tmp/out" "$target"
echo "$name $version (commit $commit) unpacked in $target; source SHA-256 verified; $sig_note."
