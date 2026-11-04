#!/usr/bin/env bash
# CFA4 – a beolvasztási gazda állapotfelmérése egyetlen lekérdezéssel.
#
# Csak olvas: a nyitott PR-ok aktuális állapotát kéri le a GitHubtól (nem másolatból),
# és egy táblázatban kiírja a gazda §3.1 szerinti szempontjait:
# alapág, utolsó commit ideje, beolvaszthatóság a mostani main-re,
# a full-validation-gate állapota a mostani fejen, és egy javasolt kategória.
#
# Használat:  scripts/dev/gazda-pr-allapot.sh [owner/repo]
# Feltétel:   gh (bejelentkezve), jq
set -euo pipefail

REPO="${1:-ubuntuokos/CinemaFundArchitecture4}"
GATE="full-validation-gate"
ACTIVE_HOURS=3

command -v gh >/dev/null || { echo "Hiányzik: gh" >&2; exit 1; }
command -v jq >/dev/null || { echo "Hiányzik: jq" >&2; exit 1; }

gh pr list --repo "$REPO" --state open --limit 200 \
  --json number,title,isDraft,baseRefName,headRefName,mergeable,mergeStateStatus,commits,statusCheckRollup \
| jq -r --arg gate "$GATE" --argjson hours "$ACTIVE_HOURS" '
  def gate_state:
    ([.statusCheckRollup[]?
      | select((.name // .context) == $gate)
      | (.conclusion // .state // .status // "")] | last) // "NINCS";
  def last_commit: (.commits | last | .committedDate);
  def age_h: ((now - (last_commit | fromdateiso8601)) / 3600);
  def category:
    if .isDraft then "a) piszkozat"
    elif age_h < $hours then "a) aktív (<\($hours) óra)"
    elif .baseRefName != "main" then "e) nem main-re épül"
    elif .mergeable == "CONFLICTING" then "c) ütközik"
    elif .mergeStateStatus == "BEHIND" then "c) frissíteni kell"
    elif gate_state == "SUCCESS" then "b) beolvasztható"
    elif gate_state == "NINCS" or gate_state == "PENDING"
         or gate_state == "IN_PROGRESS" or gate_state == "QUEUED" then "c) kapu fut / hiányzik"
    else "c) kapu piros" end;
  (["PR", "kategória", "alapág", "utolsó commit (óra)", "merge-állapot", "kapu", "cím"] | @tsv),
  (sort_by(.number)[]
   | [ "#\(.number)", category, .baseRefName, (age_h | floor | tostring),
       .mergeStateStatus, gate_state, .title ] | @tsv)
' | if command -v column >/dev/null; then column -t -s $'\t'; else cat; fi

echo
echo "A „d) kiváltotta” és az „e) tulajdonosi döntés kell” eseteket a PR-leírások alapján a gazda dönti el."
