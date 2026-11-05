#!/usr/bin/env bash
# CFA4 – a beolvasztási gazda állapotfelmérése egy paranccsal.
#
# Csak olvas: a nyitott PR-ok aktuális állapotát kéri le a GitHubtól (nem másolatból),
# és egy táblázatban kiírja a gazda §3.1 szerinti szempontjait:
# alapág, utolsó commit ideje, beolvaszthatóság a mostani main-re,
# a full-validation-gate állapota a mostani fejen, és egy javasolt kategória.
#
# Csak a REST API-t használja (a felhős Claude Code munkamenetekből a GraphQL tiltott),
# a PR-onkénti lekérdezések párhuzamosan futnak.
#
# Használat:  scripts/dev/gazda-pr-allapot.sh [owner/repo]
# Feltétel:   gh (bejelentkezve), jq, xargs
set -euo pipefail

REPO="${1:-ubuntuokos/CinemaFundArchitecture4}"
GATE="full-validation-gate"
ACTIVE_HOURS=3
JOBS="${GAZDA_JOBS:-8}"

command -v gh >/dev/null || { echo "Hiányzik: gh" >&2; exit 1; }
command -v jq >/dev/null || { echo "Hiányzik: jq" >&2; exit 1; }

# Egy PR adatai egy JSON-sorban: merge-állapot, utolsó commit ideje, kapu állapota.
pr_info() {
  local repo="$1" gate="$2" n="$3" pr sha date gate_state
  pr="$(gh api "repos/$repo/pulls/$n")"
  sha="$(jq -r '.head.sha' <<<"$pr")"
  date="$(gh api "repos/$repo/commits/$sha" --jq '.commit.committer.date')"
  gate_state="$(gh api "repos/$repo/commits/$sha/check-runs?check_name=$gate&per_page=100" \
    --jq '[.check_runs[] | (.conclusion // .status)] | first // "NINCS"')"
  jq -c --arg date "$date" --arg gate "$gate_state" '{
      number, title, draft, base: .base.ref,
      mergeable, mergeable_state, date: $date, gate: ($gate | ascii_upcase)
    }' <<<"$pr"
}
export -f pr_info

gh api --paginate "repos/$REPO/pulls?state=open&per_page=100" --jq '.[].number' \
| xargs -r -P "$JOBS" -I{} bash -c 'pr_info "$@"' _ "$REPO" "$GATE" {} \
| jq -rs --argjson hours "$ACTIVE_HOURS" '
    def age_h: ((now - (.date | fromdateiso8601)) / 3600);
    def category:
      if .draft then "a) piszkozat"
      elif age_h < $hours then "a) aktív (<\($hours) óra)"
      elif .base != "main" then "e) nem main-re épül"
      elif .mergeable_state == "dirty" then "c) ütközik"
      elif .mergeable == null or .mergeable_state == "unknown" then "c) GitHub még számol, futtasd újra"
      elif .mergeable_state == "behind" then "c) frissíteni kell"
      elif .gate == "SUCCESS" then "b) beolvasztható"
      elif .gate == "NINCS" or .gate == "QUEUED" or .gate == "IN_PROGRESS"
           or .gate == "PENDING" or .gate == "WAITING" then "c) kapu fut / hiányzik"
      else "c) kapu piros" end;
    (["PR", "kategória", "alapág", "utolsó commit (óra)", "merge-állapot", "kapu", "cím"] | @tsv),
    (sort_by(.number)[]
     | [ "#\(.number)", category, .base, (age_h | floor | tostring),
         .mergeable_state, .gate, .title ] | @tsv)
' | if command -v column >/dev/null; then column -t -s $'\t'; else cat; fi

echo
echo "A „d) kiváltotta” és az „e) tulajdonosi döntés kell” eseteket a PR-leírások alapján a gazda dönti el."
