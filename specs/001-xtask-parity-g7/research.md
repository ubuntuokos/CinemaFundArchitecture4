# Research: Mért paritás bővítése (G7)

**Forrás:** `origin/main` @ `8b7c0ea99`. A specifikációban nincs nyitott kérdés: Q1–Q3-at a tulajdonos 2026-11-14-én
lezárta. Ez a dokumentum a tervezéshez szükséges technikai döntéseket rögzíti, a kódból ellenőrzött tényekkel.

## R1 – A parancsok területének saját oldala

- **Tény:** a regiszter a `crates/engine/src/commands.rs` `command_specs()` függvénye: 193 `cmd!`/`query!` hívás.
  `CommandSpec` mezői: `id`, `label`, `menu` (`&[&str]`, üres = nincs menüben), `shortcut`, `params`, `enabled`,
  `run`, `journal`. A `docs/keyboard.md` szerint a csak billentyűs parancsok (M3.12) a `cfa4_engine::keyboard`
  modulban vannak. Ezek is azonosítóval hívhatók (`cfa4-cli exec <id>`).
- **Döntés:** egy új példaprogram, a `crates/engine/examples/command_catalog.rs`, JSON-ként kiírja a
  `cfa4-cli exec` által hívható összes parancsot: `id`, `label`, `menu`, `shortcut`, `kind` (`command` vagy
  `query`). A minta a `crates/project/examples/effect_catalog.rs`. A menü nélküli parancsok útvonala
  `["Keyboard", <label>]` (FR-001, US2/3), a lekérdezéseké (`journal == false`) `["Queries", <label>]`. Az
  utóbbiak alapból kívül esnek a hatókörön, mert nincs felhasználói párjuk.
- **Megvalósítás előtt ellenőrizendő:** a `keyboard` modul parancsai benne vannak-e a `command_specs()`-ben,
  vagy külön listából kell őket hozzáfűzni. A feladatlista erre külön lépést tartalmaz.
- **Elvetett alternatívák:** a `cfa4-cli serve` lekérdezése (futó motor kell hozzá, lassabb, és a CI-ben
  törékeny); a `commands.rs` szöveges elemzése (törékeny, és megkerüli a valódi regisztert).

## R2 – A panelek területének saját oldala (Q1: Qt6 GUI)

- **Tény:** a Qt-dokkok egy statikus, modulszintű táblából épülnek fel: `python/cfa4_qt/main_window.py`,
  `DOCK_PLAN` (név, dokkterület). A `widgets` szótár 27 panelt rendel hozzájuk. A
  `python/cfa4_qt/window_menu.py` `PANELS` tuple-je az egui panellistája. Ha egy panelnek nincs Qt-dokkja, a
  menüben letiltva jelenik meg („not yet in the Qt GUI”).
- **Döntés:** egy új fejlesztői szkript, a `scripts/dev/qt_panel_catalog.py`, `QApplication` nélkül importálja
  a `DOCK_PLAN`-t és a `PANELS`-t, és JSON-t ír ki. Panelenként: `name`, `present` (van-e Qt-dokk), `path`
  (`["Window", <név>]`, mert a Qt Window menüjében így érhető el). Ha a `PANELS`-ben szerepel, de a Qt-ban
  nincs meg, `present: false`. A mérő ezt hiányzónak veszi, nem csonknak. Az xtask a `.venv/bin/python`-nal
  futtatja, `PYTHONPATH=python` mellett. A `--ours` kapcsolóval mentett kimenet is megadható.
- **Indok:** a Qt-ablak elindítása nélkül, a motor futtatása nélkül, determinisztikusan adja a Qt valódi
  paneltábláját.
- **Elvetett alternatívák:** a `--smoke` futás (motor és képernyő kell hozzá); a `main_window.py`
  regex-elemzése (törékeny); az egui `dock.rs` (Q1 kizárja).

## R3 – Hűségi szintek forrása (FR-009–FR-013)

- **Tény:** a kódban nincs hűségi adat. Az `EffectDef`-nek és a `CommandSpec`-nek nincs ilyen mezője.
  `EngineError`-nak sincs „unsupported” változata: a változatai `UnknownCommand`, `Disabled`, `BadParams`,
  `NoSequence`, `Edit`, `Media`, `Other` (`crates/engine/src/lib.rs:116`). A befejezetlen funkciók `Other`
  vagy `BadParams` szöveggel jeleznek, a 19 előfordulás nem egységes.
- **Döntés:** egy commitolt besorolófájl, az `xtask/parity/fidelity.json` (lásd data-model.md). Kulcsa a
  terület és a saját azonosító. Értéke: szint (`exact` / `approximate` / `stub`), `reason`, `evidence`. Az
  `exact` szinthez legalább egy `evidence` hivatkozás kell, amely egy létező repóbeli fájlra (tesztre) vagy
  egy `#PR` / CI-futás azonosítóra mutat. Ha ez hiányzik, a betöltés hibát ad. A besorolás nélküli tétel
  `unrated`.
- **FR-011, eltérés (D1):** a motor ma nem jelöl géppel olvashatóan „nem támogatott” parancsot, ezért az
  automatikus csonk-besorolásnak nincs mire támaszkodnia. **Javaslat:** a G7-ben a csonk szint csak a
  besorolófájlból jön. Az automatikus jelzés egy későbbi, külön motorváltozás (`EngineError::Unsupported`,
  AGENTS.md §0.2 szellemében) után lép életbe, amely nem része ennek a projektnek, mert a közös
  `commands.rs`-t érintené. Ehhez a specifikáció FR-011 pontját módosítani kell: **tulajdonosi döntés kell**
  (lásd plan.md, „Nyitott pont”).
- **Elvetett alternatívák:** a parancsok próbafuttatása demóprojekten (mellékhatásai vannak, lassú, és
  sérti a §0-t: ellenséges paraméterekkel futna); hibaszövegek keresése a forrásban (nem megbízható).

## R4 – Commitolt referencia (Q3) és az ellenőrzése

- **Döntés:** referenciafájlok területenként: `xtask/parity/reference/<area>.json`. Minden tétel:
  `path` (szöveges útvonal) és `source` (a commitolt CFA4-dokumentum repóbeli útvonala). A FR-007a ellenőrzés
  a betöltéskor fut: a tétel levélcímkéje szó szerint, kis- és nagybetűre érzéketlenül, a `…` / `...`
  egyenértékűségével szerepeljen a megjelölt `source` fájlban. A forrás csak `docs/` alatti, létező `.md` fájl
  lehet. `plan/` alatti útvonal tilos (AGENTS.md §1.5). Ha egy tétel elbukik, a mérés hibát ad, a tétel
  útvonalával és a forrással.
- **Lehetséges források** (a commitolt dokumentációban, a célalkalmazást leíró részek):
  - `docs/keyboard.md`, parancsok: az M3.12 táblázata és a „Skipped Premiere default shortcuts” szakasz,
    amely kifejezetten hiányzó tételeket ad;
  - `docs/ui-parity.md`, panelek és eszközök;
  - `docs/target-app-parity.md` és `docs/qt-parity.md`, menük és panelek;
  - a `docs/` effektlistái.
- **Kockázat (körkörösség):** ha a referencia olyan dokumentumból jön, amely a CFA4 saját katalógusát írja le,
  a jelenlét torzítottan 100% körül lesz. **Mérséklés:** a `README.md` rögzíti, hogy csak a célalkalmazást
  leíró szakaszokból vehető név. A jelentés területenként kiírja a referencia méretét és forrásait (spec,
  Assumptions). Az effektek területén ez a kockázat a legnagyobb, ezt a checklist külön megjegyzésben jelzi.
- **Asset-szabály:** a fájlok szöveges adatok (JSON), nem assetek (AGENTS.md §1: kép, ikon, preset, sablon…
  nem kerül beléjük). Megvalósításkor ellenőrizni kell, hogy a `cargo xtask assets` nem kér hozzájuk
  sidecart. Ha kér, a fájlokat az asset-gyökéren kívülre kell tenni; az `xtask/parity/` várhatóan kívül esik.

## R5 – Küszöbök és racsni (Q2)

- **Döntés:** az `xtask/parity/thresholds.json` területenként rögzíti a `presence` és az `exact` küszöböt,
  százalékban, egy tizedesjegyre lefelé kerekítve. Kezdőértéke: az első CI-futás mért értéke, amelyet a
  bevezető PR commitol.
- **Racsni-ellenőrzés (FR-016a):** `cargo xtask parity --ratchet <BASE_FILE>`. A CI a `main` ág
  `thresholds.json`-ját kapja (`git fetch --depth=1 origin main`, majd `git show FETCH_HEAD:…`), és hibát ad,
  ha bármely érték csökken, vagy ha egy terület eltűnik. Ha a fájl a `main`-en még nincs meg (a bevezető PR),
  az ellenőrzés figyelmeztetés, nem hiba.
- **FR-016b:** ha a mért érték legalább 1 százalékponttal a küszöb fölött van, a napló `::notice::` sorban
  jelzi, hogy a küszöb emelhető. Az emelés kézi commit.

## R6 – CI-job (FR-014, FR-015, FR-019)

- **Tény:** a teljes validáció a `.github/workflows/ci-linux.yml`-ben fut. Feltétele `workflow_dispatch`,
  `merge_group` vagy a `cfa4-full-validation` címke, és a `full-validation-gate` job foglalja össze. Az
  automatikus PR-ellenőrzés (`cfa4-gate.yml`) szándékosan nem fordít.
- **Döntés:** új `parity` job a `ci-linux.yml`-ben, ugyanazzal az `if:` feltétellel, mint a
  `rust-architecture-assets`, és bekerül a `full-validation-gate` `needs` listájába. Lépései:
  1. a `.venv` beállítása a `python-qt6-linux` job mintájára;
  2. `cargo xtask parity --all` a commitolt referenciákkal és küszöbökkel;
  3. `--ratchet` ellenőrzés;
  4. a checklist elavultságának ellenőrzése, csak figyelmeztetéssel (`::warning::`).
- **Koordináció:** a `.github/` módosítása kockázatos PR, ezért kell CodeRabbit-ellenőrzés
  (`docs/coordination.md` §2.9).

## R7 – Determinisztikus checklist (FR-017–FR-019)

- **Döntés:** `cargo xtask parity --all --checklist docs/parity-checklist.md` írja a fájlt. Sorrend: területek
  rögzített sorrendben, azon belül csoport, majd útvonal szerint rendezve. Nincs időbélyeg. A
  referenciarevízió a referenciafájlok tartalmának hash-e, nem git-SHA, hogy két gépen is azonos legyen. A
  `--check-checklist` a frissen generált szöveget összeveti a commitolt fájllal, és eltérésnél figyelmeztet.
- **Elvetett alternatíva:** a git-SHA a fejlécben, mert minden commit után elavulttá tenné a fájlt.

## R8 – Visszafelé kompatibilitás (FR-021)

- **Döntés:** a meglévő kapcsolók, alapértelmezések és kimeneti utak megmaradnak. Egy változás van: ha nincs
  `--reference`, a mérő először az `xtask/parity/reference/<area>.json`-t keresi, és csak `--reference`-szel
  használja a `plan/premiere/<area>.json`-t (FR-006). A meglévő tesztek változatlanul zöldek maradnak. Új
  regressziós teszt bizonyítja, hogy ugyanazzal a referenciával a jelenlét és az elhelyezés tételenként azonos.
