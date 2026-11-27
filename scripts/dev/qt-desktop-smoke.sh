#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Q2 (CFA4-QT6-FINISH-001): asztali ellenőrzés segédszkript. Lefuttatja a `cfa4-qt --smoke
# --screenshot` parancsot a jelenlegi asztali munkamenetben, és a képernyőképet, valamint a
# munkamenet adatait egy mappába gyűjti (docs/development/qt-desktop-checks.md).
#
#   scripts/dev/qt-desktop-smoke.sh [KIMENETI_MAPPA]    (alapértelmezés: target/qt-desktop-checks)
#
# Környezeti változók: CFA4_QT (a `cfa4-qt` indító; alapból a checkout .venv/bin/cfa4-qt, majd a PATH),
# CFA4_ENGINE (motor, lásd docs/qt-gui.md). A skála a munkamenetből jön (KDE: Rendszerbeállítások >
# Kijelző); a szkript nem állít semmit. Kilépési kód: a `cfa4-qt --smoke` kódja (0: elindult és kilépett).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="${1:-$root/target/qt-desktop-checks}"

session="${XDG_SESSION_TYPE:-ismeretlen}"
desktop="${XDG_CURRENT_DESKTOP:-ismeretlen}"
case "$session" in
  wayland | x11) ;;
  *)
    echo "Figyelem: az XDG_SESSION_TYPE '$session'; valódi asztali munkamenetből futtasd (Wayland vagy X11)." >&2
    ;;
esac

qt="${CFA4_QT:-}"
if [[ -z "$qt" ]]; then
  if [[ -x "$root/.venv/bin/cfa4-qt" ]]; then qt="$root/.venv/bin/cfa4-qt"; else qt="$(command -v cfa4-qt || true)"; fi
fi
if [[ -z "$qt" ]]; then
  echo "Nincs cfa4-qt: telepítsd (docs/qt-gui.md), vagy add meg a CFA4_QT változóban." >&2
  exit 2
fi

slug="$(printf '%s-%s' "$desktop" "$session" | tr 'A-Z:/ ' 'a-z--')"
mkdir -p "$out/$slug"
shot="$out/$slug/smoke-$(date +%Y%m%dT%H%M%S).png"
info="$out/$slug/session.txt"
{
  echo "dátum: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "asztal: $desktop"
  echo "munkamenet: $session"
  echo "WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-} DISPLAY=${DISPLAY:-}"
  echo "QT_QPA_PLATFORM=${QT_QPA_PLATFORM:-} QT_SCALE_FACTOR=${QT_SCALE_FACTOR:-} QT_AUTO_SCREEN_SCALE_FACTOR=${QT_AUTO_SCREEN_SCALE_FACTOR:-}"
  echo "XMODIFIERS=${XMODIFIERS:-} GTK_IM_MODULE=${GTK_IM_MODULE:-} QT_IM_MODULE=${QT_IM_MODULE:-}"
  echo "rendszermag: $(uname -r)"
  echo "cfa4-qt: $qt"
} >"$info"

# Az indulás kimenete (a Qt platform- és skálaüzenetei) is kerüljön a mappába.
set +e
"$qt" --smoke --screenshot "$shot" 2>&1 | tee "$out/$slug/smoke.log"
code="${PIPESTATUS[0]}"
set -e
echo "kód: $code" >>"$info"
echo "Mentve: $out/$slug (képernyőkép: $shot, kilépési kód: $code)"
exit "$code"
