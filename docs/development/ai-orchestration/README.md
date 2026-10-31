# CFA4-DEV-AI-ORCHESTRATION-001 – AI-fejlesztési infrastruktúra

**Állapot:** FINAL 1.1 – APPROVED (2026-10-29). **Végrehajtás:** F0 folyamatban
([#336](https://github.com/ubuntuokos/CinemaFundArchitecture4/issues/336)); F1–F9 és E0–E6
**nincs engedélyezve**. A terv jóváhagyása nem jelent telepítési, kódmódosítási, merge- vagy
release-engedélyt.

A terv a CFA4 **fejlesztésének** eszközeiről szól (ThinkRail, Pi + Claude, Claude Code CLI,
Google Elephant Goldfish Model, AIQT 1.0.6), nem a CFA4 termékről. A termék helyi-MI és
adatvédelmi céljait (`VISION.md`) nem módosítja. A hagyományos CFA4-fejlesztésnek ezek
nélkül is működnie kell (FINAL 1.0, 8.16).

## Dokumentumok

| Fájl | Tartalom | SHA-256 |
|---|---|---|
| [`CFA4-DEV-AI-ORCHESTRATION-001-FINAL-1.0.md`](CFA4-DEV-AI-ORCHESTRATION-001-FINAL-1.0.md) | FINAL 1.0 részletes terv: TC-01–TC-10, F0–F9, E0–E6, AC-01–AC-07, szabályok, állapotgép | `4db2e755…66a3b3f7` |
| [`CFA4-DEV-AI-ORCHESTRATION-001-FINAL-1.1-record.txt`](CFA4-DEV-AI-ORCHESTRATION-001-FINAL-1.1-record.txt) | A jóváhagyási beszélgetés szövege: FINAL 1.0 jóváhagyási feljegyzés (1–90. sor), FINAL 1.1 AIQT-kiegészítés: TC-11–TC-15, AC-08–AC-14, negatív tesztek (96–839. sor), FINAL 1.1 végleges jóváhagyás (841–982. sor) | `e34aecb6…f9978308` |
| [`../../reference/aiqt/1.0.6/`](../../reference/aiqt/1.0.6/README.md) | AIQT 1.0.6 teljes szabvány, bájtra változatlanul | `fa4076e7…a3131516` |
| [`../../audit/AI-ORCHESTRATION-F0-2026-10-30.md`](../../audit/AI-ORCHESTRATION-F0-2026-10-30.md) | F0-audit: baseline, VERIFIED/PENDING/BLOCKED mátrix, gap-elemzés | – |

A végrehajtási megbízás szövege a #336 issue törzse.

## Forrás és módosítások (provenance)

Mindhárom forrást a tulajdonos csatolta 2026-10-30-án; a dokumentumimportot ugyanekkor hagyta jóvá.

| Forrás (csatolmány) | Bájt | Eredeti SHA-256 | Változtatás a tárolóbeli példányon |
|---|---|---|---|
| AIQT 1.0.6 | 16 448 | `fa4076e74c15a99000414f18fe88e3b9d3d2a0167b78b6e5240aefe0a3131516` | nincs |
| FINAL 1.0 részletes terv | 22 837 | `7a6a639612664e271ad428eaad90da2c7638fb9140a6decaeb7f304692e67b41` | 2 helyen az upstream projekt neve → `[upstream]` (118., 696. sor); záró sortörés |
| FINAL 1.0/1.1 jóváhagyási beszélgetés | 30 387 | `5e5cfb21e446bf02556dcbd275065bd33c141ed0a8c34068cc8da40cfcc92ed3` | csak az 1–982. sor (a 984. sortól a ChatGPT későbbi végrehajtási válasza és GitHub-útmutatója következik, ami **nem része** a jóváhagyott tervnek); 1 helyen az upstream név → `[upstream]` (937. sor); záró sortörés |

Az upstream márkanév cseréjét az `AGENTS.md` és a `python/tests/test_independence.py`
követeli meg (a márkanév nem kerülhet vissza a tárolóba). Más szöveg nem változott.

**A FINAL 1.0 eredetéről:** a dokumentum 779. sora szerint ez a jóváhagyás előtti
beszélgetési tartalomból **utólag összeállított** változat, nem a szó szerinti eredeti üzenet.
A tulajdonos 2026-10-30-án ezt a változatot fogadta el a tárolóba kerülő FINAL 1.0-ként.

## Elsőbbségi sorrend

1. `VISION.md`, `AGENTS.md`, `docs/coordination.md` és a CFA4 többi jóváhagyott terve.
2. CFA4-DEV-AI-ORCHESTRATION-001 (FINAL 1.0 + FINAL 1.1).
3. AIQT 1.0.6 – csak az, ami nem mond ellent a fentieknek (tulajdonosi döntés, 2026-10-30).

## CFA4-AIQT-PROFILE-001

A FINAL 1.1 9. pontja szerinti alkalmazási profil (AIQT-pont ↔ TC-komponens ↔ AC-kapu,
átvett és kihagyott pontok indoklással). **Tervezett helye:**
`docs/development/ai-orchestration/CFA4-AIQT-PROFILE-001.md`. Még nem készült el; az F2 fázis
része, külön jóváhagyás után.
