# CFA4 – GitHub Spec Kit integrációs és fejlesztési terv

**Azonosító:** CFA4-DEV-SPEC-KIT-001 · **Verzió:** TERV 1.1 · **Dátum:** 2026-10-31
**Állapot:** a tulajdonos (ubuntuokos) 2026-10-31-én jóváhagyta. **Kivitelezési engedélyt nem tartalmaz**; telepítést, CI-futást vagy GitHub-módosítást nem indít.
**Kiinduló állapot:** `main` @ `c294cca` (2026-10-30 22:55 UTC, #188 beolvasztása).

## Változások az 1.0-hoz képest

| # | Változás | Érintett szakasz |
| --- | --- | --- |
| V1 | A Spec Kit saját ág- és `specs/`-kezelése kiegészítve; CFA4-ágnév és PR-szabály elsőbbsége rögzítve | 4. (SK-03), 5. |
| V2 | Alapértelmezetten tiltott parancsok név szerint: `/speckit.implement`, `/speckit.taskstoissues`, `/speckit.constitution` | 4. (SK-07) |
| V3 | Fázisjelölés SK-F0 … SK-F7, hogy ne keveredjen az orkesztrációs terv F0–F9 fázisaival | 6. |
| V4 | Futtatási hely tisztázva: felhős Claude Code munkamenet + self-hosted runner; AC-SK-01 csak a runneren igazolható | 6., 8., 10. |
| V5 | Költségkapu függősége: orkesztrációs bázismérés (TC-10 / AC-07) nélkül nincs SK-F5 pilot | 6., 7. |
| V6 | Spec Kit munkafolyamat-lépésnevek „ellenőrizendő” jelölést kaptak; az SK-F1 a rögzített verzióból olvassa ki | 4. (SK-03) |
| V7 | Verziórögzítés: konkrét tag + `uv tool` telepítés, izolált `UV_TOOL_DIR` | 4., 6. (SK-F1) |
| V8 | A függőségi szakasz kiegészítve | 6. |
| V9 | Új sorrendi feltétel: SK-F3 csak az F0-audit 5.3/1. pontjának (autonóm munkahurok) eldöntése után | 6., 10. |

## 1. Vezetői döntési javaslat

A GitHub Spec Kit bevezetését javaslom a CFA4 fejlesztési infrastruktúrájába, kizárólag a meglévő AI-orkesztráció alárendelt, opcionális specifikációkezelő rétegeként.

- A CFA4 Development Authority marad a követelmények, jogosultságok és jóváhagyások elsődleges forrása.
- A ThinkRail marad a fejlesztési munkaterület és a SPEC-gráf tervezett kezelője.
- A Pi + Claude és a Claude Code CLI maradnak a kijelölt végrehajtó ügynökök.
- Az EGM megőrzi független tervellenőrzési szerepét; a Superpowers szelektív kiegészítő marad.
- A beolvasztás továbbra is kizárólag a beolvasztási gazda feladata ([`docs/coordination.md`](../../coordination.md)).
- A Spec Kit csak a követelmények strukturálását, a feladatok előkészítését és az eredmény–specifikáció megfelelés vizsgálatát támogatja.

Nem jön létre új AI-ügynök, új kötelező fizetős szolgáltatás, második fejlesztésirányítási rendszer, második hibanyilvántartás vagy második ágkezelési rend.

## 2. Ellenőrzött kiinduló állapot

A felsorolt elemek mind megvannak a `main` `c294cca` állapotán; `.specify/` és `specs/` könyvtár még nincs, tehát Spec Kit nem települt.

| Meglévő elem | Szerepe az integrációban |
| --- | --- |
| `VISION.md`, `AGENTS.md`, `CLAUDE.md` | Projektirányelvek és ügynökszabályok |
| `docs/coordination.md` | Beolvasztási gazda, egy PR = egy téma, 20-as PR-korlát, teljes CI a beolvasztott fejen |
| `docs/development/ai-orchestration/` | FINAL 1.0 terv, FINAL 1.1 kiegészítés |
| `docs/reference/aiqt/1.0.6/` | AIQT követelmények (csak a CFA4-szabályokkal nem ütköző részek érvényesek) |
| `docs/plans/CFA4-SUPERPOWERS-COST-001.md` | AI-használati költségkorlátok |
| `docs/audit/AI-ORCHESTRATION-F0-2026-10-30.md` | F0-audit v2 – **nem elfogadott F0-kapu** |
| `.claude/`, `.agents/` | Ügynökképességek (skillek) |
| `.github/workflows/` | `cargo-check` PR-kapu, teljes validáció |

Az F0-audit 5.3 pontja három nyitott ütközést rögzít:

1. Autonóm munkahurok (`docs/agents.md` §5–6, `CLAUDE.md` 3. lépés) ↔ előzetes jóváhagyás (FINAL 1.0 8.7., AIQT).
2. „Fordításmentes CI” szöveg az `AGENTS.md`-ben ↔ `pr-check.yml` `cargo check`-je.
3. Ruleset: 0 kötelező jóváhagyás, a teljes CI nem kötelező check.

A 2. és 3. pontban a tulajdonos 2026-10-31-én döntött (a `cargo-check` kapu marad, a teljes CI kötelező gate check lesz a `main`-en); az 1. pont nyitott. A Spec Kit ezért nem előzheti meg az orkesztrációs F0–F9 kapukat.

A Spec Kit hivatalos útmutatója szerint meglévő projektben nem kell a rendszert előbb újraspecifikálni: helyben inicializálható, és a következő, jól körülhatárolt változtatásnál használható ([Existing projects](https://github.github.com/spec-kit/guides/existing-projects.html)).

## 3. Célarchitektúra

Tervezett logikai integráció, nem működő rendszer. A rétegek felülről lefelé:

```text
┌──────────────────────────────────────────────────────────────────────┐
│ CFA4 Development Authority                                           │
│ Jóváhagyott követelmények · AIQT · jogosultságok · költségkeretek ·   │
│ beolvasztási gazda                                                   │
└──────────────────────────────────┬───────────────────────────────────┘
                                   ▼
┌──────────────────────────────────────────────────────────────────────┐
│ ThinkRail + Context/Spec Bridge                                      │
│ Követelményazonosítók · SPEC-gráf · feladatkoordináció               │
└──────────────────────────────────┬───────────────────────────────────┘
                                   ▼
┌──────────────────────────────────────────────────────────────────────┐
│ GitHub Spec Kit – CFA4 adapter (alárendelt, opcionális)              │
│  [Specify]   [Plan / Tasks]   [Bugfix / Assess]   [Converge]         │
│ Alapból tiltva: /speckit.implement · /speckit.taskstoissues ·        │
│                 /speckit.constitution                                │
└──────────────────┬───────────────────────────────┬───────────────────┘
                   ▼                               ▼
┌──────────────────────────────────┐ ┌─────────────────────────────────┐
│ Pi + Claude                      │ │ Claude Code CLI                 │
│ Tervezés és koordináció          │ │ Implementáció és tesztek,       │
│                                  │ │ jóváhagyott feladatra           │
└──────────────────┬───────────────┘ └───────────────┬─────────────────┘
                   ▼                                 ▼
┌──────────────────────────────────────────────────────────────────────┐
│ CFA4 CI · EGM · Evidence Collector · beolvasztási gazda              │
│ Független ellenőrzés · regressziók · bizonyítékok ·                   │
│ emberi jóváhagyás · csak a gazda olvaszt be                          │
└──────────────────────────────────────────────────────────────────────┘
```

A Spec Kit csak a jóváhagyott feladatkört, az ellenőrzött sablonokat és az engedélyezett munkafolyamatokat kapja meg; végrehajtás csak a meglévő ügynökökön át, jóváhagyás és beadás csak a CFA4 ellenőrző rétegén keresztül történik.

## 4. Integrációs komponensek

A megvalósítás nyolc elkülöníthető komponens; mindegyik a meglévő CFA4-szabályokat kényszeríti ki, újat nem vezet be.

| Azonosító | Komponens | Feladat |
| --- | --- | --- |
| SK-01 | Policy Adapter | A meglévő CFA4-szabályokból származtatott, rövidített szabálykészlet a Spec Kit számára |
| SK-02 | Specification Bridge | Követelményazonosítók, források és állapotok megőrzése |
| SK-03 | Workflow Adapter | Spec Kit munkafolyamatok és ágkezelés illesztése a CFA4 rendjéhez |
| SK-04 | Task/Evidence Linker | Issue, PR, commit, teszt és specifikáció összekötése |
| SK-05 | Convergence Validator | Követelmény–implementáció eltérések jelzése |
| SK-06 | Cost Control Adapter | Token-, futás- és modellköltség korlátozása |
| SK-07 | Security/Approval Guard | Hatókör, jogosultság, jóváhagyás, tiltott parancsok |
| SK-08 | Compatibility Adapter | Claude, Pi, Superpowers, EGM és CI együttműködés |

### SK-01 – Policy Adapter

A `.specify/memory/constitution.md` nem második szabályforrás: a meglévő, érvényes CFA4-szabályokból generált, forrásverzióval ellátott kivonat. Kézi szerkesztése és a `/speckit.constitution` általi újraírása tilos (SK-07).

Kötelezően átkerül: Ubuntu 26.04 LTS/amd64 célplatform, saját Rust médiamotor elsődlegessége, Qt6 GUI prioritás, FFmpeg ≥ 9.0.2, licencellenőrzés (GPL-3.0-or-later), hostrendszer érintetlensége, upstreambe írás tilalma, beolvasztás csak a gazdán keresztül. Ha a forrásszabályok ellentmondanak egymásnak, a folyamat leáll és tulajdonosi döntést kér.

### SK-02 – Specification Bridge

A Spec Kit által készített `spec.md`, `plan.md`, `tasks.md` származtatott munkadokumentum; az eredeti CFA4-követelményt nem írhatja felül.

| Mező | Rendeltetés |
| --- | --- |
| `requirement_id` | Változatlan CFA4-azonosító |
| `authority_ref` | Jóváhagyott forrás |
| `source_revision` | Dokumentumverzió vagy Git SHA |
| `task_id` | Kapcsolódó fejlesztési feladat |
| `approval_state` | Jóváhagyási státusz |
| `validation_state` | VERIFIED / PENDING / BLOCKED / NOT RUN |
| `evidence_refs` | Teszt-, PR- és auditbizonyítékok |

### SK-03 – Workflow Adapter

A Spec Kit alapfolyamata Specify → Plan → Tasks → Implement → Converge ([Spec Kit](https://github.github.io/spec-kit/)). A CFA4-ben ebből csak a tervező lépések futnak automatikusan; az Implement lépés a meglévő ügynökutakon, jóváhagyott feladatként fut.

**Ágkezelés (V1).** A `specify` lépés alapból saját, sorszámos feature-ágat és `specs/NNN-…/` könyvtárat hoz létre. A CFA4-ben:

- ágat csak a CFA4-konvenció szerint, friss `main`-ből, a feladatot végző munkamenet hoz létre; a Spec Kit automatikus ágnyitása ki van kapcsolva vagy overlay-jel felülírva;
- egy Spec Kit feature = egy CFA4-téma = egy PR (`coordination.md` §2);
- a `specs/` könyvtár helyét és számozását az SK-F1 rögzíti; ha a kikapcsolás a rögzített verzióban nem lehetséges, a Spec Kit csak ág nélküli módban használható, vagy a bevezetés leáll.

**Átvehető munkafolyamatok** – csak szükség esetén:

| Munkafolyamat | Lépések | Korlát |
| --- | --- | --- |
| Új funkció | Specify → Plan → Tasks → (Implement) → Converge | A tervezés nem ad implementációs engedélyt |
| Hibajavítás | bugfix bundle – lépésnevek **ellenőrizendők** (V6) | Nem hoz létre második hibanyilvántartást |
| Új technológia értékelése | assess bundle – lépésnevek **ellenőrizendők** (V6) | Az eredmény javaslat, nem fejlesztési utasítás |

A bugfix és assess bundle az 1.0.9-es kiadásban jelent meg beépített workflow-kkal ([v1.0.9 release](https://github.com/github/spec-kit/releases/tag/v1.0.9)); a konkrét lépéseket az SK-F1 a rögzített verzióból olvassa ki. Testreszabás presettel és workflow-overlayjel, nem forkkal.

### SK-07 – Security/Approval Guard

A specifikációgenerálás nem jogosít fel kódmódosításra, pushra, merge-re vagy release-re. Alapértelmezetten tiltott parancsok (V2):

| Parancs | Mit csinálna | Miért tiltott |
| --- | --- | --- |
| `/speckit.implement` | A terv összes feladatát végrehajtja | Autonóm munkahurok; F0-audit 5.3/1. nyitott |
| `/speckit.taskstoissues` | GitHub-issue-kat generál a feladatlistából | Az issue-k a gazdával való kommunikáció csatornái |
| `/speckit.constitution` | Újraírja a constitutiont | A constitution csak származtatott (SK-01) |

Feloldás csak parancsonként, tulajdonosi döntéssel, negatív teszttel igazolva. Külső extension, preset vagy bundle alapból tiltott; csak rögzített verzióval, forrásellenőrzés után engedélyezhető.

### SK-04, SK-05, SK-06, SK-08

- **SK-04** a meglévő bizonyítékgyűjtéshez (`docs/audit/`, TC-04) kapcsol, saját nyilvántartást nem nyit.
- **SK-05** a Converge eredményét PENDING-ként kezeli; VERIFIED csak lefutott CFA4-ellenőrzés után lehet.
- **SK-06** a `CFA4-SUPERPOWERS-COST-001` korlátaihoz igazodik.
- **SK-08** megakadályozza az ismételt EGM-review-t, a párhuzamos ügynökindítást és a duplikált munkafolyamatot; a meglévő `.claude/skills/` fájlokat nem írhatja felül.

## 5. Tervezett fájlstruktúra

A struktúra még nem létezik; a `.specify/` tényleges, telepítéskor létrejövő szerkezetét az SK-F1 a rögzített verzióhoz igazítja. A Spec Kit sablonfeloldási sorrendje: projektszintű override → preset → extension → core ([specify-cli](https://pypi.org/project/specify-cli/0.16.0/)).

```text
.specify/
├── memory/
│   └── constitution.md        # generált, forrásverzióval; kézzel nem szerkeszthető
├── templates/
│   └── overrides/             # csak ha preset nem elég
├── presets/
│   └── cfa4-policy/
├── workflows/
│   └── overlays/cfa4/         # ágkezelés, jóváhagyási megállók
└── extensions/                # alapból üres; külső extension tiltott

specs/                         # (V1): a Spec Kit munkadokumentumai
└── <SK-F1-ben rögzített séma>/
    ├── spec.md
    ├── plan.md
    └── tasks.md

docs/development/spec-kit/
├── README.md
├── integration-plan.md        # ez a dokumentum
├── authority-mapping.md
├── compatibility-matrix.md
├── cost-baseline.md
├── validation-matrix.md
├── pilot-results.md
└── rollback.md

docs/audit/spec-kit/
```

A `specs/` a közös fájlok szabálya alá esik: egy PR csak a saját feature-könyvtárát módosítja. Az ügynökparancs-fájlok (Claude-integráció) helyét és a `.claude/`-dal való ütközést az SK-F1 diffje dönti el; meglévő fájl felülírása megszakítja a próbát.

## 6. Kivitelezési fázisok

A nyolc fázis SK-F0 … SK-F7 jelölést kapott (V3), hogy ne keveredjen az orkesztrációs terv F0–F9 fázisaival; mindegyik csak a saját kapuja teljesülése és külön engedély után zárul.

| Fázis | Tartalom | Kilépési kapu | Külső előfeltétel |
| --- | --- | --- | --- |
| SK-F0 · Állapotfelmérés | Verzió, érvényes szabályok, ütközések | Felülvizsgált baseline | Orkesztrációs F0-audit elfogadva |
| SK-F1 · Izolált telepítési próba | Rögzített tag, uv, ágkezelés, diff | Host és repo érintetlen | – |
| SK-F2 · Policy és Specification Bridge | SK-01, SK-02 | Szabályparitás, azonosító-megőrzés PASS | – |
| SK-F3 · Munkafolyamat-integráció | SK-03 | Nincs autonóm vagy duplikált végrehajtás | **D1 tulajdonosi döntés** (F0 5.3/1.) |
| SK-F4 · Biztonsági és költségkapuk | SK-04–SK-08 | Negatív biztonsági tesztek PASS | – |
| SK-F5 · Korlátozott pilot | 3 feladattípus kontrollal | Minőség és költség elfogadva | **Orkesztrációs baseline** (TC-10 / AC-07) |
| SK-F6 · CI- és kiadási validáció | Azonos PR-fejen | Minden kötelező ellenőrzés igazolt | – |
| SK-F7 · Kontrollált bevezetés | Kiválasztott munkafolyamatok aktiválása | Külön tulajdonosi élesítési jóváhagyás | – |

### Függőségek (V8, V9)

- Az SK-F0 csak az orkesztrációs F0-audit elfogadása után indul.
- Az SK-F2, SK-F3 és SK-F4 részben párhuzamosítható, de az SK-F3 csak a D1 döntés után kezdődhet.
- Az SK-F5 pilot csak az SK-F4 összes kapuja és az orkesztrációs bázismérés után indulhat.
- Az integráció nem állíthatja meg a Qt6 GUI és a kodekfejlesztés ütemezését.

### Futtatási hely (V4, V7)

- Minden Spec Kit telepítés és próba felhős Claude Code munkamenetben fut, nem a tulajdonos gépén.
- Telepítés: `uv tool install specify-cli --from git+https://github.com/github/spec-kit.git@<rögzített tag>`, külön `UV_TOOL_DIR`-rel, saját worktree-ben.
- Az Ubuntu 26.04-es ellenőrzés (AC-SK-01) csak CI-jobként fut a `cfa4-ubuntu-26-04` self-hosted runneren.

### SK-F1 részletei

1. Rögzített tag kiválasztása és forrásellenőrzése.
2. `specify init --here` egy eldobható worktree-ben; teljes diff a `main`-hez képest.
3. A létrejövő `.specify/`, `specs/` és ügynökparancs-fájlok listája; meglevő fájl felülírása = leállás.
4. Az automatikus ágnyitás kikapcsolhatóságának igazolása.
5. A bugfix és assess bundle tényleges lépéseinek kiolvasása a rögzített verzióból.
6. `test_independence.py` lefuttatása a generált fájlokkal.
7. A worktree törlése; eredmény a `docs/audit/spec-kit/` alá, külön PR-ban.

## 7. Token- és végrehajtási költségek

A Spec Kit bevezetése nem növelheti az azonos minőségben elvégzett feladatok mért teljes költségét. Ezek a célértékek a `CFA4-SUPERPOWERS-COST-001` tervhez igazodnak; nem a Spec Kit mért eredményei.

| Mutató | Elfogadási követelmény |
| --- | --- |
| Összes LLM-token | ≤ a hitelesített baseline 100%-a |
| Teljes végrehajtási költség | ≤ a baseline 100%-a |
| Célzott tokenfelhasználás | a baseline 70–85%-a |
| Célzott teljes költség | a baseline 70–90%-a |
| Új kötelező fizetős szolgáltatás | 0 |
| Jogosulatlan modellhívás | 0 |
| Kötelező tesztek csökkentése | nem engedélyezett |

**Bázismérés-függőség (V5).** Hitelesített baseline jelenleg nincs: az F0-audit szerint a TC-10 és az AC-07 BLOCKED. Ezért a Spec Kit költségmérése nem kezdődhet el saját baseline-nal; az orkesztrációs terv bázismérésére épül. Amíg az nincs meg, az SK-F5 pilot nem indulhat.

Költségcsökkentési szabályok: a Spec Kit nem indít önállóan ügynököt vagy ismételt ellenőrzést; először helyi sablonfeldolgozás és determinisztikus ellenőrzés fut; a már elkészült specifikációk változatlan részei nem generálódnak újra.

A pilot három összevethető feladattípust mér azonos minőségi követelménnyel: teljes token, tényleges szolgáltatói díj, CI-ráfordítás, helyi végrehajtás, javítási ciklusok. Ha a költség nő, az érintett munkafolyamat nem aktiválható.

## 8. Kötelező elfogadási tesztek

A Spec Kit Converge eredménye önmagában nem jelenti a CFA4 release-kész állapotát; a kiadási döntést a CFA4 validációs és jóváhagyási rendszere hozza meg.

| Azonosító | Teszt | Elvárt eredmény | Hol fut |
| --- | --- | --- | --- |
| AC-SK-01 | Ubuntu 26.04/amd64 kompatibilitás | PASS | Self-hosted runner (`cfa4-ubuntu-26-04`) – a felhős sandbox 24.04, ott nem igazolható (V4) |
| AC-SK-02 | Policy Adapter | Szabályparitás PASS | Felhős munkamenet |
| AC-SK-03 | Követelményazonosítók | Nincs adatvesztés | Felhős munkamenet |
| AC-SK-04 | Claude-integráció | Izolált, reprodukálható; nem ír felül `.claude/` fájlt | Felhős munkamenet |
| AC-SK-05 | ThinkRail/Pi-kompatibilitás | Igazolt, vagy funkció kikapcsolva | Felhős munkamenet |
| AC-SK-06 | Superpowers és EGM együttélés | Nincs duplikált végrehajtás | Felhős munkamenet |
| AC-SK-07 | Jogosultsági negatív tesztek | Minden tiltott művelet és a 3 tiltott parancs blokkolva | Felhős munkamenet |
| AC-SK-08 | Költségösszehasonlítás | Nem romlik a teljes költség | Az orkesztrációs baseline után (V5) |
| AC-SK-09 | Követelmény–teszt megfelelés | Visszakövethető bizonyítékok | Felhős munkamenet |
| AC-SK-10 | CFA4 teljes releváns CI | Kötelező kapuk PASS (Qt GUI-tesztek kivéve, amíg a Qt6 GUI nincs kész) | GitHub CI |
| AC-SK-11 | Visszaállíthatóság | PASS | Felhős munkamenet |
| AC-SK-12 | Host és upstream érintetlenség | PASS | Runner + felhős munkamenet |
| AC-SK-13 | Ágkezelés (V1) | Spec Kit nem nyit ágat; CFA4-ágnév és egy PR = egy téma | Felhős munkamenet |
| AC-SK-14 | Függetlenségi egységteszt | `test_independence.py` PASS a Spec Kit fájljaival együtt | GitHub CI |

## 9. Kockázatok és kezelésük

| Kockázat | Kezelés |
| --- | --- |
| Párhuzamos szabályrendszer keletkezik | Constitution csak generált, `/speckit.constitution` tiltva |
| Spec Kit saját ágat, PR-t vagy issue-t nyit | Ágnyitás kikapcsolva, `/speckit.taskstoissues` tiltva, AC-SK-13 |
| Autonóm végrehajtás átlép egy kaput | `/speckit.implement` tiltva, végrehajtási szintű tiltás, negatív tesztek |
| AI-tokenfogyasztás emelkedik | Mért költségkapu, opcionális fázisok |
| Költség nem mérhető baseline híján | SK-F5 csak az orkesztrációs bázismérés után |
| Több ügynök ugyanazt módosítja | Worktree-koordináció, kizárólagos írás, `specs/` közös-fájl szabály |
| Specifikáció eltér a jóváhagyott tervtől | Forrásverzió-ellenőrzés, drift-jelentés |
| Külső bővítmény nem biztonságos | Alapértelmezett tiltás, verziórögzítés, forrásellenőrzés |
| Spec Kit fájl upstream márkanevet hoz be | AC-SK-14 (`test_independence.py`) a PR-on |
| Telepítés felülír meglévő `.claude/` fájlt | SK-F1 diff, ütközésnél leállás |
| Spec Kit frissítés kompatibilitást tör | Rögzített tag, staging, visszaállítás |
| Az autonóm munkahurok kérdése nyitva marad | SK-F3 nem indul (V9) |

Az integráció csak a CFA4 fejlesztői környezetére vonatkozik; a Spec Kit nem része a felhasználói alkalmazásnak, a Qt6 GUI-nak vagy a telepítőcsomagoknak.

## 10. Tulajdonosi döntések és jóváhagyás

A négy tulajdonosi döntést 2026-10-31-én elfogadták; a kivitelezés megkezdése ezektől külön, későbbi döntés.

| # | Döntendő kérdés | Döntés | Állapot |
| --- | --- | --- | --- |
| D1 | Az F0-audit 5.3/1.: autonóm munkahurok vagy előzetes jóváhagyás | Előzetes jóváhagyás | elfogadva (2026-10-31) |
| D2 | AC-SK-01 futtatási helye | Self-hosted runner, csak CI-jobként, a gép kézi módosítása nélkül | elfogadva (2026-10-31) |
| D3 | Rögzített Spec Kit verzió | Az SK-F1 indulásakor legfrissebb stabil tag (2026-10-31-én v1.0.9), utána csak külön döntéssel frissül | elfogadva (2026-10-31) |
| D4 | Mely munkafolyamatok engedélyezhetők a pilotban | Új funkció tervezése; a bugfix és assess bundle csak az SK-F1 ellenőrzése után | elfogadva (2026-10-31) |

Jóváhagyási csomag:

- [x] Spec Kit alárendelt, opcionális fejlesztési réteg
- [x] Meglévő CFA4-szabályok, a gazda és a jóváhagyási kapuk elsőbbsége
- [x] Három parancs alapértelmezett tiltása és a Spec Kit ágnyitásának kikapcsolása
- [x] Költségnövekedést kizáró mérési kapu, az orkesztrációs baseline-ra építve
- [x] Először izolált pilot, majd külön élesítési döntés
- [x] A CFA4-DEV-SPEC-KIT-001 1.1 tervet jóváhagyom; a kivitelezés megkezdésére még nem adok engedélyt

**Jóváhagyva:** 2026-10-31, tulajdonos (ubuntuokos). A D1–D4 javaslatokat a tulajdonos 2026-10-31-én elfogadta; a kivitelezés (SK-F0-tól) továbbra is külön engedélyt igényel. A jóváhagyás nem indít telepítést, CI-t vagy GitHub-módosítást.

## 11. Végső javaslat

A Spec Kit akkor ad értéket a CFA4-nek, ha a meglévő szabályokat és AI-eszközöket egészíti ki, nem pedig újra megvalósítja őket. A javasolt sorrend:

F0-audit elfogadása és D1 → SK-F0 állapotfelmérés → SK-F1 izolált telepítési próba → SK-F2 szabály- és specifikációillesztés → SK-F3 munkafolyamat-integráció → SK-F4 biztonsági és költségkapuk → orkesztrációs baseline → SK-F5 pilot → SK-F6 teljes validáció → SK-F7 külön élesítési jóváhagyás.

A Spec Kit felhasználását támogatom; éles integrációt csak bizonyított kompatibilitás, változatlan vagy alacsonyabb költség és sikeres CFA4-tesztek mellett.

### Források

- [Adopting Spec Kit in an Existing Project](https://github.github.com/spec-kit/guides/existing-projects.html)
- [GitHub Spec Kit](https://github.github.io/spec-kit/)
- [Spec Kit v1.0.9 release](https://github.com/github/spec-kit/releases/tag/v1.0.9)
- [specify-cli (PyPI)](https://pypi.org/project/specify-cli/0.16.0/)
- [CFA4 F0-audit v2](../../audit/AI-ORCHESTRATION-F0-2026-10-30.md)
- [CFA4 coordination.md](../../coordination.md)
