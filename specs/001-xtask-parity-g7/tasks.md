---

description: "Feladatlista: mért paritás bővítése (G7)"
---

# Tasks: Mért paritás bővítése – parancsok, panelek, hűségi szintek, commitolt referencia, CI (G7)

**Input**: tervezési dokumentumok a `specs/001-xtask-parity-g7/` alatt

**Prerequisites**: [plan.md](plan.md), [spec.md](spec.md), [research.md](research.md), [data-model.md](data-model.md),
[contracts/](contracts/), [quickstart.md](quickstart.md)

**Tests**: kötelezők. Az FR-020 és az AGENTS.md §0.8 ellenséges-bemeneti teszteket ír elő minden új bemenetre.
Minden user storyban a tesztfeladat áll elöl, és a tesztnek a megvalósítás előtt pirosnak kell lennie.

**Organization**: user storyk szerint. A PR-bontás a [plan.md](plan.md) szerint (PR-1 … PR-6), egy PR = egy téma,
mindegyik friss `main`-ből (`docs/coordination.md` §2.1–2.2).

**CFA4-korlátok**:

- **Jóváhagyás:** a megvalósítás jóváhagyása PENDING, a `/speckit-implement` tiltott. Ez a lista egy
  munkamenet kézi végrehajtására szól, a megvalósítás tulajdonosi jóváhagyása után.
- **Commitok:** CFA4-feladatazonosító kell hozzájuk (CLAUDE.md, „one task id per commit”). Ez az azonosító ma
  PENDING (spec, `task_id`).
- **Minőségi kapuk:** a `cfa4-ci-runner` futtatja őket minden commit előtt.
- **Nyitott döntés (D1, FR-011):** amíg nincs döntés, a T028 a plan.md javaslata szerint halad.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: párhuzamosan végezhető (más fájl, és nem függ befejezetlen feladattól)
- **[Story]**: US1–US5 (spec.md)

## Path Conventions

| Útvonal | Tartalom |
|---|---|
| `xtask/src/parity.rs` | a mérő fő modulja (ma 754 sor) |
| `xtask/src/parity/{reference,fidelity,thresholds,checklist}.rs` | új almodulok, `mod` deklarációval a `parity.rs`-ben |
| `xtask/parity/` | commitolt adatfájlok ([contracts/data-files.md](contracts/data-files.md)) |
| `crates/engine/examples/`, `scripts/dev/`, `python/tests/` | a saját oldali katalógusok és tesztjeik |

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: a research-ben nyitva hagyott két tény ellenőrzése és a modulváz létrehozása.

- [ ] T001 Ellenőrizd a `crates/engine/src/keyboard.rs` és a `crates/engine/src/commands.rs` alapján, hogy a csak
  billentyűs (M3.12) parancsok szerepelnek-e a `command_specs()` kimenetében. Ha nem, nevezd meg azt a
  publikus függvényt vagy táblát, amelyből elérhetők. Az eredményt írd be a `specs/001-xtask-parity-g7/research.md`
  R1 pontjába („Megvalósítás előtt ellenőrizendő” → tény).
- [ ] T002 [P] Ellenőrizd az `xtask/src/main.rs` `fn assets` függvényében, mely könyvtárakat és kiterjesztéseket
  vizsgál a `cargo xtask assets`, és kérne-e `.attribution` sidecart az `xtask/parity/**/*.json` fájlokhoz. Az
  eredményt írd be a `specs/001-xtask-parity-g7/research.md` R4 pontjába. Ha kérne, a T004 előtt jelezd a
  tulajdonosnak; a mappát ne helyezd át magadtól.
- [ ] T003 [P] Hozd létre az `xtask/parity/README.md`-t. Tartalma: a referencia eredetszabálya a Q3 szerint.
  - Csak olyan név, amely már a commitolt `docs/**/*.md`-ben szerepel, és ott is csak a célalkalmazást leíró
    szakaszokban. Példák: a `docs/keyboard.md` M3.12 táblázata és „Skipped Premiere default shortcuts”
    szakasza, a `docs/ui-parity.md`, a `docs/target-app-parity.md`, a `docs/qt-parity.md`.
  - A `plan/` alól semmi (AGENTS.md §1.5), és nincs asset (§1).
  - Minden tételnek kötelező `source` mezője van.
  - Írd le a körkörösség kockázatát (research R4) és a küszöbök racsniszabályát (Q2).
- [ ] T004 Hozd létre az üres almodulokat: `xtask/src/parity/reference.rs`, `xtask/src/parity/fidelity.rs`,
  `xtask/src/parity/thresholds.rs`, `xtask/src/parity/checklist.rs`. Tegyél `mod` deklarációt rájuk az
  `xtask/src/parity.rs` elejére. A fordításnak zöldnek kell maradnia (`cargo clippy -p xtask --all-targets --release -- -D warnings`).

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: közös betöltő és visszafelé-kompatibilitási védőháló. Minden user story erre épül.

**⚠️ CRITICAL**: egyetlen user story sem kezdhető el, amíg ez a fázis nincs kész.

- [ ] T005 Írj regressziós tesztet az `xtask/src/parity.rs` `mod tests` blokkjába (FR-021, SC-007). Egy rögzített
  menü-, effekt- és átmenet-referencia és saját oldali dump `compare()`-ből kapott eredményét (tételenként
  `Same` / `Elsewhere(path)` / `Missing`, a `presence()` és az `out_of_scope` darabszámai) vesse össze egy
  beégetett elvárt értékkel. A tesztnek **már most zöldnek** kell lennie, és a G7 egyik változtatása sem
  törheti el.
- [ ] T006 Írj egy közös, korlátozott JSON-fájlbetöltőt az `xtask/src/parity/reference.rs`-be:
  `load_bounded_json(path, what) -> Result<Value, String>`.
  - Legfeljebb 8 MiB méretű fájlt fogadjon el; nagyobbnál hibát ad.
  - Ne használjon `unwrap`/`expect`-et.
  - A hibaüzenet tartalmazza a `what`-ot és az útvonalat.
  - A meglévő `MAX_DEPTH = 16`, `MAX_ITEMS = 20_000` és `MAX_PATH_BYTES = 4 * 1024` konstansokat tedd
    `pub(crate)`-vé, és ezeket használja.
  - Írj hozzá egységteszteket: hiányzó fájl, túl nagy fájl, hibás JSON.
- [ ] T007 Írj stabil tartalom-hash függvényt az `xtask/src/parity/reference.rs`-be: FNV-1a 64 bit, 16 jegyű
  hexa. A `std::hash::DefaultHasher` nem stabil a Rust-verziók között, ezért nem jó. Egy teszt egy rögzített
  bemenetre rögzített kimenetet vár el (research R7: a checklist fejléce ezt használja).

**Checkpoint**: az alap kész, a user storyk indíthatók.

---

## Phase 3: User Story 1 – Mérés `plan/` nélkül, commitolt referenciával (Priority: P1) 🎯 MVP · PR-1

**Goal**: a menük, effektek és átmenetek mérése friss klónban, `plan/` nélkül, a commitolt, forrásmegjelölt
referenciával.

**Independent Test**: [quickstart.md](quickstart.md) 1. és 4. lépés a három meglévő területre. Mindhárom jelenlétszámot
ad, egyik sem írja ki, hogy „no reference list”, és a hibás `source` próbája elbukik.

### Tests for User Story 1

- [ ] T008 [P] [US1] Írj teszteket az `xtask/src/parity/reference.rs` `mod tests` blokkjába a commitolt
  formátumra: `{area, items: [{path, source}]}`.
  - **Hibát kell adjon:**
    - hiányzó `source`;
    - `plan/` alatti `source`;
    - nem `docs/` alatti vagy nem `.md` `source`;
    - nem létező `source` fájl;
    - olyan levélcímke, amely nem szerepel a `source` szövegében;
    - az `area` mező nem egyezik a fájl területével;
    - túl mély (> 16), túl sok tételes (> 20 000) vagy túl hosszú (> 4 KiB) útvonal.
  - **El kell fogadjon:** kis- és nagybetűtől független egyezést; a `…` és a `...` egyenértékűségét.
  - A tesztek ideiglenes könyvtárban saját `docs/x.md` fájlt használjanak.

### Implementation for User Story 1

- [ ] T009 [US1] Valósítsd meg a commitolt referencia betöltését és a FR-007a ellenőrzést az
  `xtask/src/parity/reference.rs`-ben, a [data-model.md](data-model.md) „ReferenceItem” szabályai szerint, szó
  szerint:
  - `path`: „1–16 elem, mindegyik nem üres”;
  - `source`: „létező `docs/**/*.md` fájl, `plan/` alatti útvonal tilos”;
  - ellenőrzés: „a levélcímke szó szerint, kis- és nagybetűre érzéketlenül szerepel a `source` szövegében, a
    `…` és a `...` egyenértékű”.

  A `--reference`-szel megadott helyi fájlra a régi formátumok (útvonaltömb, `"A > B"`, fa) továbbra is
  érvényesek, `source`-ellenőrzés nélkül.
- [ ] T010 [US1] Módosítsd az `xtask/src/parity.rs` `run()` függvényét (FR-006, research R8):
  - `--reference` nélkül az `xtask/parity/reference/<area>.json` a referencia;
  - a `plan/premiere/<area>.json` csak `--reference`-szel használható;
  - a jelentés fejléce és a konzol első sora megnevezi a referencia útvonalát, a tartalom-hash-t (T007), a
    tételszámot és forrásonként a darabszámot.

  Frissítsd a modul doc-kommentjét (1–29. sor) és a `parse_args` usage-szövegét.
- [ ] T011 [P] [US1] Állítsd össze az `xtask/parity/reference/menus.json`-t. Csak olyan menüútvonal kerülhet
  bele, amelynek levélcímkéje egy commitolt, a célalkalmazást leíró `docs/` szakaszban szerepel, és minden
  tételnek legyen `source` mezője. Semmit ne vegyél a `plan/` alól.
- [ ] T012 [P] [US1] Állítsd össze az `xtask/parity/reference/effects.json`-t ugyanezzel a szabállyal. Ne vegyél
  nevet olyan dokumentumból, amely a CFA4 saját effektkatalógusát írja le (research R4, körkörösség). Ha így
  kevés tétel marad, az elfogadható: a jelentés kiírja a referencia méretét.
- [ ] T013 [P] [US1] Állítsd össze az `xtask/parity/reference/transitions.json`-t ugyanezzel a szabállyal.
- [ ] T014 [US1] Futtasd a [quickstart.md](quickstart.md) 1. lépését (`menus`, `effects`, `transitions`) és a 4.
  lépését `plan/` nélküli friss klónban. Mentsd a kimenetet a PR leírásába bizonyítékként.

**Checkpoint**: az US1 önmagában működik. A PR-1 beadható („CodeRabbit: nem kell”).

---

## Phase 4: User Story 2 – Parancsok és panelek mérése (Priority: P2) · PR-2 (parancsok), PR-3 (panelek)

**Goal**: két új terület: `commands` (motorregiszter) és `panels` (Qt6 GUI, Q1).

**Independent Test**: [quickstart.md](quickstart.md) 2. lépés. Egy háromtételes kézi referencia egy *same place*, egy
*elsewhere* és egy *missing* eredményt ad.

### Tests for User Story 2

- [ ] T015 [P] [US2] Írj teszteket az `xtask/src/parity.rs` `mod tests` blokkjába a parancsok területére:
  - **Útvonalképzés** ([contracts/catalog-dumps.md](contracts/catalog-dumps.md)):
    - ha a `menu` nem üres: `menu + [label]`;
    - különben `query`-nél `["Queries", label]`, és ez hatókörön kívül esik;
    - minden más esetben `["Keyboard", label]`.
  - **FR-004 hatókör:** a fiók-, felhő-, licenc- és fizetős tételek, valamint más platformok rendszerelemei
    kívül esnek.
  - **Ugyanaz a címke két menüben:** egyszer számít; „ugyanott”, ha bármelyik útvonal egyezik.
  - **Ellenséges dump:** nem tömb, hiányzó `id`/`label`, túl sok tétel.
- [ ] T016 [P] [US2] Írj teszteket az `xtask/src/parity.rs` `mod tests` blokkjába a panelek területére:
  - a `present: false` panel nem saját tétel, ezért egy rá vonatkozó referencia *missing*;
  - egy másik panel lapjaként meglévő panel *elsewhere*, ha a dump így jelzi;
  - ellenséges dump.
- [ ] T017 [P] [US2] Írd meg a `python/tests/test_qt_panel_catalog.py`-t.
  - Futtassa a `scripts/dev/qt_panel_catalog.py`-t (`PYTHONPATH=python`, `QT_QPA_PLATFORM=offscreen`).
  - Ellenőrizze:
    - a kimenet érvényes JSON;
    - a `main_window.DOCK_PLAN` minden neve `present: true`;
    - egy `window_menu.PANELS`-beli, a `DOCK_PLAN`-ből hiányzó név `present: false`;
    - a szkript nem hoz létre `QApplication`-t;
    - a rendezés stabil.
  - Ha nincs PySide6, a teszt legyen skip.

### Implementation for User Story 2 – parancsok (PR-2)

- [ ] T018 [P] [US2] Írd meg a `crates/engine/examples/command_catalog.rs`-t az `effect_catalog.rs` mintájára.
  - A kimenet JSON-tömb `id`, `label`, `menu`, `shortcut` és `kind` mezőkkel. A `kind` értéke `command`, vagy
    `query`, ha `journal == false`.
  - A tömb `id` szerint rendezett.
  - Legyenek benne a T001 szerint fellelt csak billentyűs parancsok is.
  - Szerializációs hiba esetén `ExitCode::FAILURE`, az ok a stderr-en. Nincs `unwrap`.
  - A modul doc-kommentje nevezze meg a `cargo xtask parity --area commands`-t.
- [ ] T019 [US2] Add hozzá az `Area::Commands`-t az `xtask/src/parity.rs`-hez.
  - Bővítsd hozzá: `parse`, `name` = `commands`, `ours`, `place`, `out_file`, `min_len`, `wrapper_keys`, `group`.
  - A `dump_ours` a `cfa4-engine` csomag `command_catalog` példáját futtassa.
  - Az új `our_commands()` a T015 szerinti útvonalakat képzi.
  - Az `out_of_scope` kapjon parancs-ágat (FR-004).
- [ ] T020 [P] [US2] Állítsd össze az `xtask/parity/reference/commands.json`-t a T011 szabályával.
  - Elsődleges forrás a `docs/keyboard.md`: az M3.12 táblázata és a „Skipped Premiere default shortcuts”
    szakasz. Az utóbbi tételei várhatóan *missing* besorolást kapnak, és ez a cél.
  - Minden tételnek legyen `source` mezője.

### Implementation for User Story 2 – panelek (PR-3)

- [ ] T021 [P] [US2] Írd meg a `scripts/dev/qt_panel_catalog.py`-t.
  - `QApplication` nélkül importálja a `cfa4_qt.main_window.DOCK_PLAN`-t és a `cfa4_qt.window_menu.PANELS`-t.
  - A kimenet JSON-tömb `name`, `present` és `dock_area` mezőkkel.
  - Rendezés: a `PANELS` sorrendje, majd a csak `DOCK_PLAN`-ben szereplők név szerint.
  - Ha az import nem sikerül, 1-es kilépési kód, az ok a stderr-en.
  - Feleljen meg a `ruff check python scripts`-nek.
- [ ] T022 [US2] Add hozzá az `Area::Panels`-t az `xtask/src/parity.rs`-hez.
  - A `dump_ours` a `.venv/bin/python scripts/dev/qt_panel_catalog.py`-t futtassa `PYTHONPATH=python`
    környezettel.
  - Ha nincs `.venv/bin/python`, a hiba nevezze meg a területet, és utaljon a
    `docs/development/linux.md`-re vagy a `--ours` kapcsolóra.
  - Az új `our_panels()` útvonala `["Window", name]`, és csak a `present: true` tételeket veszi.
- [ ] T023 [P] [US2] Állítsd össze az `xtask/parity/reference/panels.json`-t a T011 szabályával. Lehetséges
  források: `docs/ui-parity.md`, `docs/qt-parity.md`, `docs/target-app-parity.md`.
- [ ] T024 [US2] Futtasd a [quickstart.md](quickstart.md) 2. lépését mindkét területre, és frissítsd az
  `xtask/src/parity.rs` modul doc-kommentjét a két új területtel.

**Checkpoint**: az US1 és az US2 önállóan működik. A PR-2 és a PR-3 beadható („CodeRabbit: nem kell”).

---

## Phase 5: User Story 3 – Hűségi szintek a jelentésben (Priority: P2) · PR-4

**Goal**: minden megtalált tétel szintet kap (`exact` / `approximate` / `stub` / `unrated`), és a jelentésben
megjelenik a pontos jelenlét százaléka.

**Independent Test**: [quickstart.md](quickstart.md) 3. lépés. Négy tétel négy különböző szinten, és egy bizonyíték
nélküli `exact` bejegyzés betöltési hibát ad.

### Tests for User Story 3

- [ ] T025 [P] [US3] Írj teszteket az `xtask/src/parity/fidelity.rs` `mod tests` blokkjába a [data-model.md](data-model.md)
  „FidelityEntry” szabályaira, szó szerint:
  - `tier`: „`exact` | `approximate` | `stub`. Más érték: betöltési hiba”;
  - `reason`: „nem üres, legfeljebb 200 karakter”;
  - `evidence`: „`exact`-nál **legalább 1** elem”, és egy elem „létező repóbeli fájl, vagy `#<szám>` alakú
    PR-hivatkozás. Hiányzó fájl: betöltési hiba”;
  - az árva besorolás külön listába kerül;
  - besorolás nélkül a szint `unrated`;
  - ha `in scope` = 0, az `exact %` is 0, osztás nélkül;
  - ellenséges fájl: mélység, méret, hibás JSON.

### Implementation for User Story 3

- [ ] T026 [US3] Valósítsd meg az `xtask/parity/fidelity.json` betöltését az `xtask/src/parity/fidelity.rs`-ben
  (`Tier` enum, `load_fidelity()`, `tier_of(area, id)`). A betöltő a T006 közös betöltője legyen.
- [ ] T027 [US3] Bővítsd az `xtask/src/parity.rs` `Report` szerkezetét.
  - Minden *Same* vagy *Elsewhere* tétel kapjon szintet.
  - A `summary()` és a `markdown()` kapjon `Exact | Approximate | Stub | Unrated` oszlopot és `exact %`-ot.
  - Kerüljön a jelentésbe egy „Orphan fidelity entries” szakasz.
  - A meglévő oszlopok ne változzanak (FR-021). A T005-nek zöldnek kell maradnia.
- [ ] T028 [US3] FR-011 a D1 javaslat szerint: a `stub` szint csak a `fidelity.json`-ból jön. Írd le a
  korlátot az `xtask/parity/README.md`-ben: a motor ma nem jelöl „unsupported” parancsot, az automatikus jelzés
  egy későbbi, külön projekt. Ha a tulajdonos a D1-ben mást dönt, ezt a feladatot a döntés szerint kell
  átírni, mielőtt elkezded.
- [ ] T029 [US3] Add hozzá a `--min-exact PERCENT` kapcsolót a `parse_args`-hoz (0–100, a `--min`-nel azonos
  ellenőrzéssel), és a `run()` ellenőrizze. Bővítsd a `our_dump_and_arguments_are_checked` tesztet.
- [ ] T030 [P] [US3] Hozd létre az `xtask/parity/fidelity.json`-t kezdő bejegyzésekkel. Csak olyan tétel kapjon
  `exact` szintet, amelyhez létező teszt tartozik. Példa: a CPU/WGSL paritástesztekkel lefedett effektek
  (docs/roadmap.md M5). Az ilyen tételek `evidence` mezőjében legyen a tesztfájl útvonala. A többi tétel
  maradjon `unrated`; ne találj ki szintet.

**Checkpoint**: a hűségi szintek minden területen megjelennek. A PR-4 beadható („CodeRabbit: nem kell”).

---

## Phase 6: User Story 4 – CI-job a küszöbbel (Priority: P3) · PR-5

**Goal**: a küszöbök racsnival, és CI-job a teljes validációban.

**Independent Test**: [quickstart.md](quickstart.md) 5. lépés. A küszöb csökkentése hibát ad a `--ratchet`-tel, a
küszöb a mért érték fölé emelése hibát ad a `--all`-lal, és a napló megnevezi a területet, a mért értéket és a
küszöböt.

### Tests for User Story 4

- [ ] T031 [P] [US4] Írj teszteket az `xtask/src/parity/thresholds.rs` `mod tests` blokkjába.
  - **Értékek:** „0–100, egy tizedesjegy, lefelé kerekítve”.
  - **Racsni:** „a `main`-en lévő fájlhoz képest egyetlen érték sem csökkenhet, és terület sem tűnhet el”.
  - **Hiányzó alapfájl:** figyelmeztetés, nem hiba.
  - **`::notice::`:** ha a mért érték legalább 1 százalékponttal a küszöb fölött van.
  - **Ellenséges fájl.**
- [ ] T032 [P] [US4] Írj teszteket a `--all` kapcsolóra az `xtask/src/parity.rs`-ben:
  - nem adható meg az `--area`, a `--reference` és az `--ours` mellett;
  - a területek sorrendje rögzített: `menus`, `effects`, `transitions`, `commands`, `panels`;
  - a naplósor formátuma megfelel a [contracts/cli.md](contracts/cli.md)-nek;
  - GitHub-annotáció csak beállított `GITHUB_ACTIONS` mellett jelenik meg.

### Implementation for User Story 4

- [ ] T033 [US4] Valósítsd meg az `xtask/src/parity/thresholds.rs`-t: `load_thresholds()` és
  `check_ratchet(base, current)`.
- [ ] T034 [US4] Valósítsd meg a `--all`, a `--thresholds` és a `--ratchet` kapcsolót az `xtask/src/parity.rs`-ben
  a [contracts/cli.md](contracts/cli.md) kilépési kódjai szerint. Hiba esetén a kód 1, és nincs részeredmény.
- [ ] T035 [US4] Futtasd a `cargo xtask parity --all` parancsot a commitolt referenciákkal. Írd az
  `xtask/parity/thresholds.json`-ba a mért `presence` és `exact` értékeket, egy tizedesjegyre lefelé kerekítve.
  Ez a racsni kezdőértéke (Q2).
- [ ] T036 [US4] Adj új `parity` jobot a `.github/workflows/ci-linux.yml`-hez.
  - **`if:`:** azonos a `rust-architecture-assets` feltételével.
  - **Beállítások:** `needs: platform-scope`, `runs-on: ubuntu-26.04`, `timeout-minutes: 20`, és a checkout
    `persist-credentials: false`-szal, a meglévő pinelt SHA-val.
  - **Lépések:**
    1. `.venv` a `python-qt6-linux` job mintájára;
    2. `cargo xtask parity --all`;
    3. `git fetch --depth=1 origin main`, majd `git show FETCH_HEAD:xtask/parity/thresholds.json`, és ha a
       fájl létezik, `cargo xtask parity --all --ratchet <fájl>`;
    4. `if [ -f docs/parity-checklist.md ]` mellett `cargo xtask parity --all --check-checklist docs/parity-checklist.md`.
       Ez csak figyelmeztet; így a PR-6-nak nem kell a `.github/`-hoz nyúlnia.
  - Add hozzá a `parity`-t a `full-validation-gate` `needs` listájához.
  - A `cfa4-gate.yml`-hez ne nyúlj: az automatikus PR-ellenőrzés nem bővül.
- [ ] T037 [US4] A PR-5 ágán futtasd a jobot `workflow_dispatch`-csal: egyszer zöldre, egyszer egy szándékosan
  megemelt küszöbbel pirosra. A két futás linkjét írd a PR leírásába. A PR-on kérj CodeRabbit-ellenőrzést
  (`@coderabbitai review`, „CodeRabbit: kell”), mert a PR a `.github/`-ot érinti (`docs/coordination.md` §2.9).

**Checkpoint**: a CI őrzi a küszöböket. A PR-5 csak zöld CodeRabbit check-run után adható át.

---

## Phase 7: User Story 5 – Commitolt `docs/parity-checklist.md` (Priority: P3) · PR-6

**Goal**: determinisztikus, commitolt checklist mind az öt területről.

**Independent Test**: [quickstart.md](quickstart.md) 6. lépés. Két generálás és a commitolt fájl bájtra azonos.

### Tests for User Story 5

- [ ] T038 [P] [US5] Írj teszteket az `xtask/src/parity/checklist.rs` `mod tests` blokkjába:
  - két egymás utáni renderelés bájtra azonos;
  - nincs benne időbélyeg és abszolút útvonal;
  - a sorrend a rögzített területsorrend, azon belül csoport, majd útvonal;
  - a `--check-checklist` eltérésnél 0-s kilépési kóddal figyelmeztet, egyezésnél csendes.

### Implementation for User Story 5

- [ ] T039 [US5] Valósítsd meg a renderelést az `xtask/src/parity/checklist.rs`-ben.
  - **Fejléc:**
    - a generáló parancs (`cargo xtask parity --all --checklist docs/parity-checklist.md`);
    - a referenciafájlok tartalom-hash-e (T007);
    - területenként a referencia mérete és forrásai;
    - a saját oldal forrása: `menus` = egui menüfa, `panels` = Qt6 GUI.
  - **Megjegyzések:**
    - a menüknél: a szám nem vethető össze a gaps.md ad hoc 92%-ával (SC-009);
    - az effekteknél: a körkörösség kockázata (research R4).
  - **Táblázatok területenként:** jelenlét, elhelyezés, szintek.
- [ ] T040 [US5] Valósítsd meg a `--checklist FILE` és a `--check-checklist FILE` kapcsolót az `xtask/src/parity.rs`-ben
  (csak `--all` mellett).
- [ ] T041 [US5] Generáld le a `docs/parity-checklist.md`-t
  (`cargo xtask parity --all --checklist docs/parity-checklist.md`). Kézzel ne szerkeszd.

**Checkpoint**: minden user story kész.

---

## Phase 8: Polish & Cross-Cutting Concerns

- [ ] T042 [P] Frissítsd a `docs/gaps.md` G7 szakaszát.
  - **Done:** a commands, a panels, a szintek, a commitolt referencia, a CI és a checklist.
  - **Missing:** preferences, formats, valamint az automatikus stub-jelzés, ha a D1 szerint az kimaradt.
  - **Estimate:** frissítve.

  Csak a G7 sorait módosítsd (`docs/coordination.md` §2.5).
- [ ] T043 [P] Frissítsd a `docs/roadmap.md` 8. sorát: a parity már CI-ban fut, és mind az öt területet méri.
  Csak ezt a sort módosítsd.
- [ ] T044 Futtasd a minőségi kapukat a `cfa4-ci-runner` subagenttel:
  - `cargo xtask ci`;
  - `cargo test --workspace --exclude cfa4-ui --release`;
  - `.venv/bin/python -m ruff check python`;
  - `.venv/bin/python -m pytest -q python/tests --ignore=python/tests/test_qt_gui.py`.

  A PR-bizonyíték a szűretlen kapunapló.
- [ ] T045 Futtasd végig a teljes [quickstart.md](quickstart.md)-t (1–7. lépés). Az eredményeket (PR-ok és CI-futások)
  írd a `specs/001-xtask-parity-g7/spec.md` CFA4-követelmény blokkjának `evidence_refs` mezőjébe. A
  `validation_state` csak lefutott CFA4-ellenőrzés után lehet VERIFIED, és azt a tulajdonos vagy a gazda
  rögzíti.

---

## Dependencies & Execution Order

### Phase Dependencies

| Fázis | Függés |
|---|---|
| Setup (T001–T004) | nincs; a T001 eredménye a T018-hoz, a T002 eredménye a T011–T013-hoz kell |
| Foundational (T005–T007) | a Setup után; minden user storyt blokkol |
| US1 (T008–T014) | a Foundational után |
| US2 (T015–T024) | az US1 után, mert az alapértelmezett referencia feloldása (T010) kell hozzá; a PR-2 és a PR-3 egymástól független |
| US3 (T025–T030) | az US1 után; az US2 nélkül is működik, de akkor a `commands` és a `panels` nem kap szintet |
| US4 (T031–T037) | az US1–US3 után, mert a `--all` mind az öt területet és a pontos jelenlétet méri |
| US5 (T038–T041) | az US4 után, mert a `--all`-ra épül; a CI-lépése már a T036-ban benne van |
| Polish (T042–T045) | minden user story után |

### User Story Dependencies

```text
Setup → Foundational → US1 (PR-1) ─┬→ US2-commands (PR-2) ─┐
                                   ├→ US2-panels  (PR-3) ──┼→ US4 (PR-5) → US5 (PR-6) → Polish
                                   └→ US3 (PR-4) ──────────┘
```

A coordination §2.1 szerint egy PR nem épülhet egy másik, még nyitott PR-ra. Ezért minden PR csak az előfeltétele
beolvasztása után indul, friss `main`-ből.

### Within Each User Story

A teszt kerül előre, és a megvalósítás előtt pirosnak kell lennie. Utána jön az adatbetöltő, a mérőlogika,
végül az adatfájlok és a bizonyíték.

### Parallel Opportunities

- **Setup:** a T002 és a T003 párhuzamosan mehet, a T001 mellett.
- **US1:** a T011, a T012 és a T013 (három külön referenciafájl) párhuzamosan mehet, a T009–T010 után.
- **US2:** a T015, a T016 és a T017 (tesztek) párhuzamosan; a T018, a T020, a T021 és a T023 (külön fájlok)
  párhuzamosan.
- **US3:** a T025 és a T030 párhuzamosan.
- **US4:** a T031 és a T032 párhuzamosan.
- **Polish:** a T042 és a T043 párhuzamosan.

---

## Parallel Example: User Story 2

```text
# Tesztek egyszerre:
T015 parancs-útvonal és hatókör tesztjei        (xtask/src/parity.rs)
T017 Qt-panelkatalógus tesztje                   (python/tests/test_qt_panel_catalog.py)
# T016 a T015 után, mert ugyanabba a fájlba ír.

# Külön fájlok egyszerre:
T018 crates/engine/examples/command_catalog.rs
T021 scripts/dev/qt_panel_catalog.py
T020 xtask/parity/reference/commands.json
T023 xtask/parity/reference/panels.json
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Setup (T001–T004), majd Foundational (T005–T007).
2. US1 (T008–T014): ettől a mérés `plan/` nélkül is fut, és a paritásszámot bárki előállíthatja.
3. **Állj meg és validálj:** quickstart 1. és 4. lépés. Utána a PR-1 beadható.

### Incremental Delivery

| PR | Feladatok | Mit ad hozzá |
|---|---|---|
| PR-1 | US1 | MVP |
| PR-2 | US2, parancsok | a 193 parancs mérése |
| PR-3 | US2, panelek | a Qt6 panelek mérése |
| PR-4 | US3 | hűségi szintek |
| PR-5 | US4 | CI és racsni; CodeRabbit kell |
| PR-6 | US5 | commitolt checklist |
| — | Polish | a T042–T043 a PR-6 része lehet, mert dokumentáció és egy téma |

### Notes

- **[P]:** más fájl, nincs függés befejezetlen feladattól.
- **Közös fájlok** (`docs/gaps.md`, `docs/roadmap.md`, referenciafájlok): csak a saját soraidat add hozzá vagy
  módosítsd, ne rendezd át őket.
- **Commit:** csak zöld állapot kerülhet commitba, egy commit egy CFA4-feladatazonosítóval. Az azonosító ma
  PENDING.
- **Tiltott:** a `/speckit-implement`, a `/speckit-taskstoissues` és issue-k nyitása a Spec Kitből (constitution).
