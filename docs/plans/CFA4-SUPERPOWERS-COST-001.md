# CFA4-SUPERPOWERS-COST-001 — FINAL 1.0

**Jóváhagyás:** 2026-10-29, projektgazda.  
**Állapot:** Végleges kivitelezési terv; végrehajtási validáció folyamatban.  
**Cél:** CinemaFundArchitecture4, Ubuntu 26.04 LTS / amd64.  
**Hivatkozás:** obra/superpowers v7.0.1, commit 2ce8aa15d05bd57213e6f746f953aafb7ded7a8b.

## Kötelező döntés

A Superpowers **nem** új AI-orkesztrátor, hanem a már jóváhagyott
ThinkRail Pi + Claude Code CLI + EGM rendszer szelektív, alárendelt
módszertani kiegészítője. `VISION.md`, `AGENTS.md`,
`docs/coordination.md` és AIQT 1.0.6 elsőbbséget élveznek.
Fejlesztés, PR, merge és release külön jogosultsági kapuhoz kötött.

A Superpowers teljes automatikus telepítése, automatikus
SessionStart/using-superpowers aktiválása, korlátlan újrapróbálkozás,
duplikált review, új daemon és új kötelező fizetős szolgáltatás tiltott.
A meglévő Rust médiamotor, Qt6-prioritás, FFmpeg 9.0.2 követelmény,
upstream-szeparáció (a szülőprojekt nem módosul) és szoftver-együttélési szabály megmarad.

## Költség- és minőségi követelmények

Azonos jellegű, azonos minőségben végrehajtott CFA4 feladatoknál:

- **Új teljes LLM-tokenfogyasztás ≤ korábbi hitelesített fogyasztás.**
- **Új teljes végrehajtási költség ≤ korábbi hitelesített költség.**
- Fizetős modellköltség cél: a baseline 60–80%-a.
- Teljes LLM-token cél: a baseline 70–85%-a.
- Teljes végrehajtási költség cél: a baseline 70–90%-a.
- Kötelező regressziós / release tesztkövetelmények nem csökkenhetnek.
- Jogosulatlan művelet és új kötelező fizetős szolgáltatás: 0.

E célok teljesülése méréstől függ, nem garantálható és nem állítható
előre. A költségbe beletartozik a provider, cache, alügynök,
újrapróbálkozás, CI és a helyi compute energia- és időráfordítása.

## Engedélyezett munkamód

Az alapértelmezett végrehajtás egymenetes (`executing-plans`).
Szelektíven használható: `writing-plans`,
`systematic-debugging`, `verification-before-completion`,
`test-driven-development`, `requesting-code-review`,
`diagnosing-superpowers`.
Párhuzamos ügynök és `subagent-driven-development` külön engedély
nélkül nem indulhat. A meglévő worktree- és EGM-review folyamat nem
duplikálható. Helyi modell csak mért minőségi és költségelőnynél.

## Végrehajtási csomagok és kapuk

| Kapu | Kimenet | Kötelező átvételi feltétel |
|---|---|---|
| S0 | BASELINE-COST-001 | Valós token, USD, CI és helyi compute adatok |
| S1 | Szelektív adapter | Nincs globális plugin és autonóm modellhívás |
| S2 | COST-GATE-001 | Atomikus foglalás, pontos provider-enforcement, fail closed |
| S3 | Orkesztráció összhang | Nincs duplikált ThinkRail/Claude/EGM munkamenet |
| S4 | Tesztintegráció | CFA4 kötelező Rust/Qt6/Python/FFmpeg/CI kapuk megtartva |
| S5 | COST-VALIDATION-001 | Összehasonlítható pilot, költségnövekedés nélkül |
| S6 | Visszaállítható élesítés | Teljes SC-validáció és gazdai jóváhagyás |

**A már meglévő képességek újraírása tilos.** A költségkapu
éles engedélyezése addig tiltott, amíg a szolgáltatói modellhívásokon
nem garantálható a token- és költségkorlát.

## Jelenlegi végrehajtási ellenőrzőpont

Létrejött: inaktív modul-allowlist és önálló, offline, fail-closed
Python költség-előellenőrzés negatív tesztekkel. Ezek **nem**
szolgáltatói hívás-kikényszerítők; S0 baseline és S2–S6 elfogadási
bizonyíték még hiányzik. Az SC-01–SC-16 részletes státusz:
[Superpowers policy](../development/superpowers-policy.md).

## Biztonsági és kiadási határok

Tilos automatikus push más munkamenet ágára, upstream szülőprojekt
módosítás, merge, release, root hostváltoztatás, titkok naplózása
vagy prompt injection által kiváltott jogosultságnövelés.
Minden upstream képesség végrehajtása előtt a CFA4 engedélye és
a tényleges költségkeret szükséges; hiányos bizonyíték = tiltás.
A teljes CI a megfelelő PR-fejre vonatkozó merge-kapuban kötelező.

**FINAL 1.0 terv; az integráció csak bizonyíték alapján nyilvánítható késznek.**
