# CFA4 – GitHub Spec Kit integrációs és fejlesztési terv

**Azonosító:** CFA4-DEV-SPEC-KIT-001 · **Verzió:** TERV 1.2 · **Dátum:** 2026-11-01
**Állapot:** a tulajdonos (ubuntuokos) 2026-11-01-én jóváhagyta, a D1′ és a D5–D10 döntéssel együtt (D9, D10: az SK-F1 tanulságai, 12:56); az 1.1-et ez váltja fel. Az SK-F1 indítását a tulajdonos 2026-11-01-én engedélyezte; a #450 beolvasztása után indul. A további fázisok (SK-F2-től) külön engedélyhez kötöttek.
**Kiinduló állapot:** `main` @ `64d8f86` (2026-11-01 04:19 UTC, #438 beolvasztása).

## Változások 1.1 → 1.2

Az 1.1 óta a terv (#366) és az SK-F0 jelentés (#370) a `main`-re került; a K1–K3 ütközések tulajdonosi döntéssel rendeződtek; a gazda csak `/gazda`-hívásra dolgozik (#438); döntés született a szabályok egyetlen helyen tartásáról (#449) és a tokentakarékosságról.

| # | Változás | Oka | Szakasz |
| --- | --- | --- | --- |
| W1 | A constitution **hivatkozó fájl**, nem szabálykivonat | Tulajdonosi döntés 2026-11-01: minden szabály csak egy helyen | 4. (SK-01) |
| W2 | A D1 a 2026-10-31-i projektszabály szerint értelmezve: jóváhagyás fázisonként, a fázison belül önálló munka | #363, `docs/agents.md` §5–6, `CLAUDE.md` 3. lépés | 2., 10. (D1′) |
| W3 | Spec Kit-fájl nem kerül a `main`-re az SK-F7 előtt | Minden párhuzamos munkamenet a `main` `.claude/skills/` könyvtárát tölti be | 5., 10. (D7) |
| W4 | Új mérőszám: munkamenetenkénti kontextus-többlet | Tokentakarékosság; a `disable-model-invocation` hatása nem garantált | 7., 8. (AC-SK-15) |
| W5 | Rögzített forrásellenőrzés: v1.2.5, `git` extension nélkül nincs ágnyitás | SK-F0 | 4., 6. |
| W6 | Az SK-F1 leválasztása az orkesztrációs F0-audit (#336) elfogadásáról | Az SK-F1 eldobható, a `main`-t nem érinti; #336 nyitott | 6., 10. (D6) |
| W7 | Gazdakapcsolat: jelzés a PR-leírásban, `/gazda` | #438 | 4. (SK-07), 9. |
| W8 | Előfeltétel: a szabályfájl-tisztítás (#449) a `main`-en | Az SK-01 hivatkozásai erre épülnek | 6. |
| W9 | Elavult pontok törölve (K1–K3 nyitott; „issue = gazdacsatorna”; `cargo xtask wasm` kapu) | 2026-10-31 és 11-01 döntései | több helyen |

## 1. Vezetői javaslat

A Spec Kit maradjon alacsony prioritású, alárendelt és opcionális specifikációs réteg, amely a pilot lezártáig egyetlen fájllal sem kerül a `main`-re. A CFA4 most a Qt6-felületre, az upstream-felzárkózásra (CFA4-UPSTREAM-SYNC-001, U0) és a tokentakarékosságra figyel; a Spec Kit egyiket sem lassíthatja.

- A szabályok egyetlen forrása az `AGENTS.md`, a `VISION.md` és a `docs/coordination.md`; a Spec Kit csak hivatkozik rájuk.
- Beolvasztás kizárólag a gazdán keresztül, a tulajdonos `/gazda`-hívására.
- Minden SK-fázis külön tulajdonosi engedéllyel indul; azon belül a munkamenet önállóan halad (2026-10-31-i projektszabály).
- A Spec Kit nem nyit ágat, PR-t vagy issue-t, nem indít ügynököt, és nem növelheti a többi munkamenet kontextusát.

Nem jön létre új ügynök, fizetős szolgáltatás, második szabályforrás, hibanyilvántartás vagy ágkezelési rend.

## 2. A CFA4 állapota 2026-11-01-én

Az SK-F0 óta a terv függőségeinek többsége rendeződött; nyitott maradt az F0-audit elfogadása, a költségbaseline és a szabályfájl-tisztítás. Forrás: `main` @ `64d8f86`, GitHub API.

| Tétel | Állapot | Bizonyíték |
| --- | --- | --- |
| Terv 1.1, SK-F0 jelentés | a `main`-en | #366, #370 (2026-10-31) |
| SK-F0 kilépési kapu („felülvizsgált baseline”) | **elfogadva 2026-11-01** | D5 |
| K1: autonóm munkahurok ↔ D1 | **rendezve**: önálló munka csak a saját, jóváhagyott projekten belül | #363; `docs/agents.md` §5–6; `CLAUDE.md` 3. lépés |
| K2: CI-szöveg ↔ `cargo check` | **rendezve** | #364; `AGENTS.md` 17. sor |
| K3: teljes CI kötelező | **rendezve**: `cargo-check` + `full-validation-gate`, szigorú; kötelező jóváhagyás 0 | #365, ruleset |
| Gazda | csak `/gazda`-hívásra; `.claude/skills/gazda`, `disable-model-invocation: true`; a munkamenetek a PR-leírásban jeleznek | #438 |
| Szabályfájlok | döntés 2026-11-01: egy szabály egy helyen, a `wasm` nem kapu | #449 nyitott |
| Orkesztrációs F0-audit | **nem elfogadott** | #336 nyitott |
| Költségbaseline (BASELINE-COST-001, TC-10/AC-07) | **BLOCKED** | SK-F0 8. pont |
| Spec Kit | v1.2.5 a legfrissebb stabil tag; Python ≥ 3.11 | tag-lista, `pyproject.toml` |
| `.specify/`, `specs/`, `.claude/settings.json` | nincs | fájllista |
| ThinkRail, Pi | nincs a környezetben | F0-audit v2 |
| Nyitott PR-ok | 71 | GitHub API |

**Új tényezők, amelyekhez a terv igazodik:**

- **Tokentakarékosság.** A tulajdonos visszavonta a TokenEconomizer-tervet; a megtakarítást a szabályfájlok tisztításától várja. A Spec Kit Claude-integrációja 10 core skillt (v1.2.5) ír a `.claude/skills/` alá; ha ezek a `main`-re kerülnek, minden munkamenet kontextusát terhelhetik.
- **Upstream-szinkron.** A CFA4-UPSTREAM-SYNC-001 U0-felzárkózása fut (0.10 → 0.15); a gazdakapacitást ez és a Qt6-munka viszi.
- **71 nyitott PR.** A Spec Kit a pilot előtt csak dokumentációs PR-t nyithat, legfeljebb fázisonként egyet.

## 3. Célarchitektúra

Tervezett logikai integráció, nem működő rendszer. A Spec Kit nem kap saját szabálykészletet, csak hivatkozik a meglévő forrásokra, és a pilot végéig nem kerül a `main`-re.

```text
┌──────────────────────────────────────────────────────────────────────┐
│ CFA4-szabályok – egyetlen forrás                                     │
│ AGENTS.md · VISION.md · docs/coordination.md · docs/licensing.md     │
│ (a main aktuális állapota)                                           │
└──────────────────────────────────┬───────────────────────────────────┘
                                   ▼
┌──────────────────────────────────────────────────────────────────────┐
│ Jóváhagyott projekt és fázisengedély                                 │
│ Tulajdonosi döntés fázisonként · követelményazonosítók ·             │
│ ThinkRail/Pi még nincs a környezetben                                │
└──────────────────────────────────┬───────────────────────────────────┘
                                   ▼
┌──────────────────────────────────────────────────────────────────────┐
│ GitHub Spec Kit v1.2.5 – pilotágon, eldobható worktree-ben           │
│ [Hivatkozó constitution] [Specify/Clarify] [Plan/Tasks]              │
│ [Analyze/Converge]                                                   │
│ Tiltva: implement · taskstoissues · constitution-újraírás ·          │
│         beépített workflow (specify workflow run) ·                  │
│         git- és issue-extension · Claude-események                   │
│ --integration claude · --script sh · --no-model-invocation ·         │
│ --events false                                                       │
└──────────────────┬───────────────────────────────┬───────────────────┘
                   ▼                               ▼
┌──────────────────────────────────┐ ┌─────────────────────────────────┐
│ Fejlesztő munkamenet             │ │ Claude Code CLI                 │
│ Saját projekt, saját ág,         │ │ Implementáció és tesztek        │
│ egy PR = egy téma                │ │ a CFA4-úton                     │
└──────────────────┬───────────────┘ └───────────────┬─────────────────┘
                   ▼                                 ▼
┌──────────────────────────────────────────────────────────────────────┐
│ CI · EGM · bizonyítékok · gazda (/gazda)                             │
│ cargo-check + full-validation-gate · jelzés a PR-leírásban ·         │
│ csak a gazda olvaszt be, tulajdonosi jóváhagyással                   │
└──────────────────────────────────────────────────────────────────────┘
```

## 4. Integrációs komponensek

A nyolc komponens neve és célja az 1.1-ből változatlan; az alábbi pontok a 2026-11-01-i állapot miatt módosultak. A v1.2.5-re vonatkozó állításokat a Spec Kit forrásában ellenőriztem; CFA4-ben még nem futottak.

| Azonosító | Komponens | Változás az 1.1-hez képest |
| --- | --- | --- |
| SK-01 | Policy Adapter | Hivatkozó constitution, nem kivonat (W1) |
| SK-02 | Specification Bridge | változatlan (lásd 1.1: `requirement_id`, `authority_ref`, `source_revision`, `task_id`, `approval_state`, `validation_state`, `evidence_refs`) |
| SK-03 | Workflow Adapter | Ágnyitás: `git` extension nem telepíthető (W5) |
| SK-04 | Task/Evidence Linker | Bizonyíték a PR-leírásban és a `docs/audit/spec-kit/` alatt (W7) |
| SK-05 | Convergence Validator | változatlan: a Converge eredménye PENDING, VERIFIED csak lefutott CFA4-ellenőrzés után |
| SK-06 | Cost Control Adapter | Új mérőszám: kontextus-többlet (W4) |
| SK-07 | Security/Approval Guard | Tiltott parancsok mindkét írásmódban, tiltott extensionok |
| SK-08 | Compatibility Adapter | Együttélés a `gazda` és a `cfa4-skill-review` skillel |

### SK-01 – hivatkozó constitution (W1)

A tulajdonos 2026-11-01-i döntése szerint minden szabály csak egy helyen áll. Ezért a `.specify/memory/constitution.md` nem másolja a CFA4-szabályokat, hanem:

1. felsorolja a kötelező forrásokat (`AGENTS.md`, `VISION.md`, `docs/coordination.md`, `docs/licensing.md`) és a `main` SHA-t, amelyre épül;
2. előírja, hogy a Spec Kit-lépés előbb ezeket olvassa be a `main` aktuális állapotából;
3. csak a Spec Kit-specifikus szabályokat tartalmazza (tiltott parancsok, ágnyitás tilalma, PENDING-kezelés), mert ezek máshol nincsenek.

Ha a források ellentmondanak egymásnak, a munka leáll, és a tulajdonos dönt.

### SK-03 – ágkezelés (W5)

A v1.2.5-ben a git-init, a feature-ág (`before_specify` hook) és az auto-commit a `git` extensionben van, amely csak `--extension git`-tel települ. A CFA4-ben ez az extension, valamint a GitHub-issue-kat kezelő extension nem telepíthető. Ágat a munkamenet nyit friss `main`-ből, a CFA4-konvenció szerint; egy feature = egy téma = egy PR.

A pilotban csak az új funkció tervezése fut (Specify → Clarify → Plan → Tasks → Analyze). A bugfix- és assess-folyamat csak az SK-F1 forrásellenőrzése után vehető fel (D4).

### SK-06 – kontextus-többlet (W4)

A v1.2.5 Claude-integrációja a 10 core parancsot külön skillként írja a `.claude/skills/speckit-*` alá. A `--no-model-invocation` ezekre `disable-model-invocation: true`-t ír, de az extension-skillekre nem. A Claude Code-ban nyitott hibajegyek szerint a jelző hatása a skill-leírás kontextusba kerülésére nem egyértelmű. Ezért a hatását az SK-F1-ben mérni kell (AC-SK-15).

Forrásbeli korlát: a `preset add` és az `integration use` a skilleket az alapértelmezéssel írja újra, vagyis a `--no-model-invocation` ilyenkor elvész. A CFA4-preset telepítése után ezért minden skill fejlécét ellenőrizni kell (AC-SK-16).

### SK-07 – tiltott parancsok és extensionok

| Elem | Mindkét írásmódban | Miért tiltott |
| --- | --- | --- |
| implement | `/speckit.implement`, `/speckit-implement` | Végrehajtás a CFA4 ügynökútján kívül; SK-F3-ig tiltott |
| taskstoissues | `/speckit.taskstoissues`, `/speckit-taskstoissues` | Második feladatnyilvántartás; a gazdakapcsolat a PR-leírás |
| constitution | `/speckit.constitution`, `/speckit-constitution` | A constitution hivatkozó fájl, kézzel karbantartva (SK-01) |
| `git` extension | – | Ágnyitás, auto-commit |
| GitHub-issue extension | – | Issue-generálás |
| Claude-események | `--events false` | Nincs automatikus indítás (`SessionStart` stb. a `.claude/settings.json`-ban) |
| Beépített „Full SDD Cycle” workflow | `specify workflow run speckit` | A `tasks` után kapu nélkül futtatja az `implement`-et (SK-F1, 9.1). Az SK-F3-ban CFA4-overlay távolítja el az `implement` lépést, vagy teszi kapu mögé (D9) |

Feloldás parancsonként, tulajdonosi döntéssel és negatív teszttel.

### SK-08 – együttélés

A `.claude/skills/` alatt már ott van a `gazda` és a `cfa4-skill-review`. A `speckit-` előtag miatt névütközés nem várható, de a telepítés utáni diffnek ezt igazolnia kell. A Spec Kit-skilleket telepítés előtt a `cfa4-skill-review` (SkillSpector) nézi át.

## 5. Fájlok és izoláció (W3)

A `main`-re az SK-F7 előtt csak dokumentáció kerül; a `.specify/`, a `specs/` és a `.claude/skills/speckit-*` addig csak eldobható worktree-ben, illetve egy be nem olvasztandó pilotágon él. Ok: minden párhuzamos munkamenet a `main` `.claude/skills/` könyvtárából dolgozik, így egy ott lévő Spec Kit-skill mindegyikre hatna.

| Hely | Mi kerül ide | Mikortól a `main`-en |
| --- | --- | --- |
| `docs/development/spec-kit/` | terv, `authority-mapping.md`, `compatibility-matrix.md`, `rollback.md` | már most (doksi) |
| `docs/audit/spec-kit/` | fázisjelentések (SK-F0, SK-F1 …) | már most (doksi) |
| `.specify/` (hivatkozó constitution, `cfa4-policy` preset, sablonok) | Spec Kit-konfiguráció | SK-F7, külön jóváhagyással |
| `.claude/skills/speckit-*` | Spec Kit-parancsok | SK-F7, csak ha az AC-SK-15 teljesül |
| `specs/<CFA4-azonosító>/` | pilot `spec.md`, `plan.md`, `tasks.md` | a pilotban csak a pilotágon; SK-F7 után feature-enként, a saját PR-jában |
| `.claude/settings.json` | – | soha (események kikapcsolva) |
| `extensions/` | – | üres; külső extension tiltott |

A `specs/` könyvtárak neve a CFA4-feladatazonosító, nem a Spec Kit sorszáma; a pontos sémát az SK-F1 rögzíti.

## 6. Fázisok

Az SK-F0 elkészült és elfogadott (D5); az SK-F1 indítása engedélyezve (2026-11-01), a későbbi fázisok három külső feltételre várnak. Minden fázis külön engedéllyel indul (D1′).

| Fázis | Tartalom | Elfogadási tesztek | Külső előfeltétel |
| --- | --- | --- | --- |
| SK-F0 · Állapotfelmérés | **kész, elfogadva** (#370, D5) | – | – |
| SK-F1 · Izolált próba, v1.2.5 | **kész** (#451): AC-SK-04, -12, -13, -14, -15 PASS; kontextus-többlet 0 token | AC-SK-04, -12, -13, -14, -15 | – |
| SK-F2 · Hivatkozó constitution, preset | SK-01, SK-02 | AC-SK-02, -03, -16 | **#449 a `main`-en** |
| SK-F3 · Munkafolyamat-integráció | SK-03 | AC-SK-05, -06 | **#336 elfogadva** |
| SK-F4 · Biztonsági kapuk | SK-04–SK-08 | AC-SK-07 | – |
| SK-F5 · Pilot a pilotágon | egy új funkció tervezése (D8) | AC-SK-08, -09 | **költségbaseline** (BASELINE-COST-001, TC-10/AC-07) |
| SK-F6 · CI- és kiadási validáció | azonos PR-fejen | AC-SK-01, -10, -11, -12 | – |
| SK-F7 · Spec Kit a `main`-re (D7) | kiválasztott munkafolyamatok aktiválása | – | külön tulajdonosi élesítési jóváhagyás |

### SK-F1 – pontos menet

1. Eldobható worktree a friss `main`-ből, saját `UV_TOOL_DIR`-rel, a felhős munkamenetben.
2. `uv tool install specify-cli --from git+https://github.com/github/spec-kit.git@v1.2.5` (vagy az akkor legfrissebb stabil tag, D3).
3. A Spec Kit-skilleket telepítés előtt a `cfa4-skill-review` nézi át.
4. `specify init --here --force --non-interactive --integration claude --script sh --integration-options="--no-model-invocation --events false"`, `--extension` nélkül. A `--force` csak eldobható worktree-ben használható, utána kötelező a teljes diff; meglévő fájl módosulása = leállás (D10; a v1.2.5 nem üres könyvtárban `--force` nélkül nem fut).
5. Teljes diff a `main`-hez: meglévő fájl nem változhat; `.claude/settings.json` nem jöhet létre; ág és commit nem keletkezhet.
6. AC-SK-15: a kontextus mérése két friss munkamenetben ugyanazon a fán, `.claude/skills/speckit-*`-szal és nélküle.
7. `test_independence.py` a generált fájlokkal.
8. A worktree törlése; a `main`-re csak a `docs/audit/spec-kit/SK-F1-….md` jelentés kerül, egy doksi-PR-ban.

### Sorrend és párhuzamosság

- Az SK-F1 nem vár a #449-re és a #336-ra, mert a `main`-t nem érinti (D6).
- Az SK-F2 a #449 után indul, mert a hivatkozó constitution a tisztított szabályfájlokra mutat.
- Az SK-F3-tól a #336 elfogadása feltétel; az SK-F5 a költségbaseline-ra vár.
- A Qt6-munka és az upstream-szinkron (U0) előbbre való; a Spec Kit-munka egyiket sem tarthatja fel.

## 7. Költség és tokenkontextus

A Spec Kit nem növelheti sem a saját feladatainak, sem a többi munkamenetnek a költségét. Az alábbi célértékek a `CFA4-SUPERPOWERS-COST-001` tervhez igazodnak; nem mért eredmények.

| Mutató | Követelmény |
| --- | --- |
| Kontextus-többlet Spec Kitet nem használó munkamenetben (új, W4) | 0 token; ha nem érhető el, a skillek nem kerülnek a `main`-re |
| Kontextus-többlet Spec Kit-munkamenetben, hívás előtt | mérve, az SK-F1 jelentésében rögzítve |
| Összes LLM-token azonos feladatra | ≤ a hitelesített baseline 100%-a; cél 70–85% |
| Teljes végrehajtási költség | ≤ a baseline 100%-a; cél 70–90% |
| Új fizetős szolgáltatás, jogosulatlan modellhívás | 0 |
| Kötelező tesztek csökkentése | nem engedélyezett |

**Baseline-függőség.** Hitelesített baseline még nincs (BASELINE-COST-001, TC-10, AC-07: BLOCKED). A kontextus-többlet viszont baseline nélkül is mérhető: ugyanazon a `main`-en egy munkamenet Spec Kit-skillekkel és nélkülük. Ezért az AC-SK-15 már az SK-F1-ben lefuthat, az SK-F5 pilot viszont továbbra is a baseline-ra vár.

Takarékossági szabályok: a Spec Kit nem indít ügynököt, ismételt ellenőrzést vagy EGM-review-t; a constitution rövid hivatkozó fájl, így nem duplázza az `AGENTS.md` tartalmát; a változatlan specifikációrészek nem generálódnak újra.

## 8. Elfogadási tesztek

A Spec Kit Converge eredménye nem jelent kiadási készséget; az a CFA4 validációs és jóváhagyási rendszerének döntése. A kötelező CI-kapu: `cargo-check` + `full-validation-gate` (a Qt GUI-tesztek a Qt6 GUI elkészültéig nem blokkolnak).

| Azonosító | Teszt | Elvárt eredmény | Fázis | Hol fut |
| --- | --- | --- | --- | --- |
| AC-SK-01 | Ubuntu 26.04/amd64 | PASS | SK-F6 | self-hosted runner, CI-job |
| AC-SK-02 | Hivatkozó constitution | Nincs másolt CFA4-szabály; minden forrás elérhető a `main`-en | SK-F2 | felhős munkamenet |
| AC-SK-03 | Követelményazonosítók | Nincs adatvesztés | SK-F2 | felhős munkamenet |
| AC-SK-04 | Claude-integráció | Izolált, reprodukálható; meglévő `.claude/` fájl nem változik | SK-F1 | felhős munkamenet |
| AC-SK-05 | ThinkRail/Pi | Igazolt, vagy a funkció kikapcsolva | SK-F3 | felhős munkamenet |
| AC-SK-06 | Superpowers, EGM, gazda együttélése | Nincs duplikált végrehajtás | SK-F3 | felhős munkamenet |
| AC-SK-07 | Negatív jogosultsági tesztek | Minden tiltott parancs, extension és esemény blokkolva | SK-F4 | felhős munkamenet |
| AC-SK-08 | Költségösszehasonlítás | Nem romlik | SK-F5 | baseline után |
| AC-SK-09 | Követelmény–teszt megfelelés | Visszakövethető | SK-F5 | felhős munkamenet |
| AC-SK-10 | CFA4 kötelező CI | `cargo-check` + `full-validation-gate` PASS | SK-F6 | GitHub CI |
| AC-SK-11 | Visszaállíthatóság | PASS | SK-F6 | felhős munkamenet |
| AC-SK-12 | Host és upstream érintetlenség | PASS | SK-F1, SK-F6 | munkamenet + runner |
| AC-SK-13 | Ágkezelés | A Spec Kit nem nyit ágat, nem commitol | SK-F1 | felhős munkamenet |
| AC-SK-14 | Függetlenségi teszt | `test_independence.py` PASS a Spec Kit fájljaival | SK-F1 | helyben + CI |
| AC-SK-15 | Kontextus-többlet (új) | 0 token Spec Kitet nem használó munkamenetben | SK-F1 | felhős munkamenet |
| AC-SK-16 | Skillfejlécek preset után (új) | Minden core skill `disable-model-invocation: true` | SK-F2 | felhős munkamenet |

## 9. Kockázatok

| Kockázat | Kezelés |
| --- | --- |
| A Spec Kit-skillek minden munkamenet kontextusát terhelik | `main`-re csak SK-F7-ben, az AC-SK-15 teljesülése után (W3, W4) |
| `preset add` / `integration use` visszaállítja a modellhívhatóságot | AC-SK-16 minden preset- vagy integrációművelet után |
| Az extension-skillekre a `--no-model-invocation` nem hat | Extension nem telepíthető (SK-07) |
| Második szabályforrás keletkezik | Hivatkozó constitution, AC-SK-02 (W1) |
| A Spec Kit ágat, commitot, issue-t vagy PR-t hoz létre | `git` és issue-extension tiltva, `/speckit-taskstoissues` tiltva, AC-SK-13 |
| A beépített workflow kapu nélkül implementál | `specify workflow run speckit` tiltva; SK-F3-ban overlay (D9) |
| A `--force` meglévő fájlt ír felül | Csak eldobható worktree-ben; kötelező diff, módosulás = leállás (D10) |
| Autonóm végrehajtás egy jóváhagyott fázison kívül | `/speckit-implement` tiltva; minden fázis külön engedéllyel (D1′) |
| Eseménykezelők a `.claude/settings.json`-ban | `--events false`; a fájl létrejötte = leállás |
| A gazdakapacitást elviszi | Előtte csak doksi-PR, fázisonként legfeljebb egy; a Qt6 és az upstream-szinkron előbbre való |
| A szabályfájlok közben változnak | Az SK-F2 a #449 beolvasztása után indul; a constitution a `main` SHA-jára hivatkozik |
| Költség nem mérhető baseline híján | SK-F5 csak a baseline után; a kontextus-többlet addig is mérhető |
| Upstream márkanév kerül be | AC-SK-14 |
| Spec Kit-frissítés kompatibilitást tör | Rögzített tag, frissítés csak döntéssel (D3) |

Az integráció csak a fejlesztői környezetet érinti; a Spec Kit nem része az alkalmazásnak, a Qt6 GUI-nak vagy a telepítőcsomagoknak.

## 10. Tulajdonosi döntések

| # | Kérdés | Döntés | Állapot |
| --- | --- | --- | --- |
| D1′ | A D1 (előzetes jóváhagyás) és a 2026-10-31-i projektszabály viszonya | Minden SK-fázis külön tulajdonosi engedéllyel indul; a fázison belül a munkamenet önállóan halad, más projekt feladatát nem veszi át; `/speckit-implement` tiltott marad | elfogadva 2026-11-01 |
| D2 | AC-SK-01 helye | Self-hosted runner, CI-jobként | elfogadva 2026-10-31 |
| D3 | Rögzített verzió | Az SK-F1 indulásakor a legfrissebb stabil tag (2026-11-01-én v1.2.5) | elfogadva 2026-10-31 |
| D4 | Pilotmunkafolyamat | Csak új funkció tervezése | elfogadva 2026-10-31 |
| D5 | Az SK-F0 jelentés (#370) elfogadása | Elfogadás; a K1–K3 azóta rendeződött, a K4 javítva | elfogadva 2026-11-01 |
| D6 | Az SK-F1 függ az orkesztrációs F0-audit (#336) elfogadásától? | Nem: az SK-F1 eldobható worktree-ben fut, a `main`-re csak jelentés kerül. Az SK-F3-tól a #336 elfogadása feltétel marad | elfogadva 2026-11-01 |
| D7 | Spec Kit-fájlok a `main`-en | Csak SK-F7-ben, külön jóváhagyással; a pilotág nem olvad be | elfogadva 2026-11-01 |
| D8 | Pilotfeladat | Egy már jóváhagyott projekt (pl. Qt6-paritás) egy új funkciója, a projekt gazdájának tudtával; a választás az SK-F5 indításakor | elfogadva 2026-11-01 |
| D9 | Beépített „Full SDD Cycle” workflow (SK-F1 9.1) | `specify workflow run speckit` tiltva; az SK-F3-ban CFA4-overlay veszi ki az `implement` lépést, vagy teszi kapu mögé | elfogadva 2026-11-01 |
| D10 | `--force` a `specify init`-nél (SK-F1 E1) | A tiltás helyett: csak eldobható worktree-ben, utána kötelező teljes diff; meglévő fájl módosulása = leállás | elfogadva 2026-11-01 |

Jóváhagyási csomag:

- [x] A CFA4-DEV-SPEC-KIT-001 1.2 tervet jóváhagyom; az 1.1-et ez váltja fel
- [x] D1′, D5, D6, D7, D8 a javaslat szerint
- [x] Az SK-F1 indításához engedélyt adok (2026-11-01)

**Jóváhagyva:** 2026-11-01, tulajdonos (ubuntuokos): a terv, a D1′ és D5–D8, valamint az SK-F1 indítása. Az SK-F1-et a Spec Kit-projekt fejlesztő munkamenete végzi, nem a gazda; a `main`-re csak a jelentése kerül.

## 11. Végső javaslat

A Spec Kit akkor éri meg a CFA4-nek, ha egyetlen szabályt sem dupláz, egyetlen más munkamenetet sem terhel, és mérhetően nem drágítja a munkát. A sorrend:

SK-F1 izolált próba – kész (#451) → #449 a `main`-en → SK-F2 hivatkozó constitution és preset → #336 elfogadása → SK-F3 munkafolyamat → SK-F4 biztonsági kapuk → költségbaseline → SK-F5 pilot → SK-F6 teljes validáció → SK-F7 külön élesítési döntés.

Ha az AC-SK-15 nem teljesül, a Spec Kit csak a pilotban marad használható, a `main`-re nem kerül.

### Források

- [Spec Kit v1.2.5 kiadás](https://github.com/github/spec-kit/releases/tag/v1.2.5)
- [Spec Kit – Core Commands](https://github.github.io/spec-kit/reference/core.html)
- [Spec Kit – Claude Code integráció](https://github.github.io/spec-kit/integrations/claude.html)
- [Claude Code hibajegy #31935: disable-model-invocation és a skill-leírások](https://github.com/anthropics/claude-code/issues/31935)
- [CFA4 SK-F0 jelentés](../../audit/spec-kit/SK-F0-2026-10-31.md)
- [CFA4 coordination.md](../../coordination.md)
- Az 1.1-es változat: a fájl Git-előzményeiben (#366)
