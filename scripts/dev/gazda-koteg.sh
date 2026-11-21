#!/usr/bin/env bash
# CFA4 – a beolvasztási gazda köteg-PR-ja (docs/coordination.md §3a, tulajdonosi döntés 2026-11-08).
#
# A tulajdonos által jóváhagyott PR-okat a mostani main-ből nyitott `koteg/ÉÉÉÉHHNN-N` ágra
# egyenként, merge committal rárakja, a köteget PR-ként megnyitja a main-re, és felteszi rá a
# `cfa4-full-validation` címkét: az egész kötegre EGY teljes validáció fut.
# Ütköző PR-t kihagy (git merge --abort) és kiírja. Beolvasztani NEM olvaszt be: a köteg-PR-t a
# gazda zöld full-validation-gate után merge committal (soha nem squash-sal) olvasztja be; így az
# eredeti PR-ok fejei a main-be kerülnek, és a GitHub azokat magától „Merged”-nek jelöli.
#
# Csak a REST API-t használja (a felhős Claude Code munkamenetekből a GraphQL tiltott).
#
# Használat:  scripts/dev/gazda-koteg.sh [--dry-run] <PR-szám>...
#             (a sorrend a megadott sorrend; --dry-run: helyben összerakja, nem pushol, nem nyit PR-t)
# Feltétel:   gh (bejelentkezve), jq, git; tiszta munkakönyvtár
set -euo pipefail

REPO="${GAZDA_REPO:-ubuntuokos/CinemaFundArchitecture4}"
REMOTE="${GAZDA_REMOTE:-origin}"
LABEL="cfa4-full-validation"
DRY_RUN=0

if [[ "${1:-}" == "--dry-run" ]]; then DRY_RUN=1; shift; fi
if (( $# == 0 )); then
  echo "Használat: $0 [--dry-run] <PR-szám>..." >&2
  exit 2
fi
for tool in gh jq git; do
  command -v "$tool" >/dev/null || { echo "Hiányzik: $tool" >&2; exit 1; }
done
if [[ -n "$(git status --porcelain)" ]]; then
  echo "A munkakönyvtár nem tiszta; előbb rendezd (commit vagy stash)." >&2
  exit 1
fi

git fetch -q "$REMOTE" main
base_sha="$(git rev-parse "$REMOTE/main")"

# Következő szabad köteg-ágnév a mai napra.
day="$(date -u +%Y%m%d)"
n=1
while git ls-remote --exit-code --heads "$REMOTE" "koteg/$day-$n" >/dev/null 2>&1; do
  n=$((n + 1))
done
branch="koteg/$day-$n"
git switch -q -c "$branch" "$base_sha"

included=()
skipped=()
for raw in "$@"; do
  pr="${raw#\#}"
  [[ "$pr" =~ ^[0-9]+$ ]] || { skipped+=("$raw: nem PR-szám"); continue; }
  info="$(gh api "repos/$REPO/pulls/$pr" 2>/dev/null)" || { skipped+=("#$pr: nincs ilyen PR"); continue; }
  state="$(jq -r '.state' <<<"$info")"
  base="$(jq -r '.base.ref' <<<"$info")"
  title="$(jq -r '.title' <<<"$info")"
  head_sha="$(jq -r '.head.sha' <<<"$info")"
  if [[ "$state" != "open" ]]; then skipped+=("#$pr: nem nyitott ($state)"); continue; fi
  if [[ "$base" != "main" ]]; then skipped+=("#$pr: nem a main-re épül ($base)"); continue; fi
  git fetch -q "$REMOTE" "pull/$pr/head"
  if [[ "$(git rev-parse FETCH_HEAD)" != "$head_sha" ]]; then
    skipped+=("#$pr: a feje lekérdezés közben változott, aktív")
    continue
  fi
  if git merge -q --no-ff --no-edit -m "Köteg $branch: #$pr $title" "$head_sha" >/dev/null 2>&1; then
    included+=("#$pr ($head_sha) $title")
  else
    git merge --abort
    skipped+=("#$pr: ütközik a köteggel vagy a main-nel")
  fi
done

echo "Köteg-ág: $branch (alap: main @ ${base_sha:0:7})"
echo "Bekerült (${#included[@]}):"
printf '  %s\n' "${included[@]:-–}"
echo "Kihagyva (${#skipped[@]}):"
printf '  %s\n' "${skipped[@]:-–}"

if (( ${#included[@]} == 0 )); then
  echo "Egy PR sem került a kötegbe; nincs köteg-PR." >&2
  git switch -q --detach "$base_sha"
  git branch -q -D "$branch"
  exit 1
fi
if (( DRY_RUN )); then
  echo "--dry-run: nem pusholok, nem nyitok PR-t (a helyi ág: $branch)."
  exit 0
fi

git push -q -u "$REMOTE" "$branch"
body="$(
  echo "Köteg-PR a beolvasztási gazdától ([docs/coordination.md](https://github.com/$REPO/blob/main/docs/coordination.md) §3a)."
  echo
  echo "Alap: \`main\` @ \`${base_sha:0:7}\`. Egy teljes validáció az egész kötegre."
  echo "**Csak merge committal olvasztható be (squash/rebase tilos)**, hogy az alábbi PR-ok magától beolvasztottnak látsszanak."
  echo
  echo "### Bekerült"
  printf -- '- %s\n' "${included[@]}"
  if (( ${#skipped[@]} > 0 )); then
    echo
    echo "### Kihagyva"
    printf -- '- %s\n' "${skipped[@]}"
  fi
)"
url="$(gh api "repos/$REPO/pulls" -f base=main -f head="$branch" \
  -f title="Köteg $branch (${#included[@]} PR)" -f body="$body" --jq '.html_url')"
number="${url##*/}"
gh api "repos/$REPO/issues/$number/labels" -f "labels[]=$LABEL" >/dev/null
echo "Köteg-PR: $url (címke: $LABEL, a teljes validáció elindult)"
