# CFA4-PARITY-G7: a `cargo xtask parity` bővítésének végleges terve (2026-11-14)

Beépítettem a három döntést: Q1 (a panelek a Qt6 GUI-ból jönnek), Q2 (racsni) és Q3 (csak a commitolt doksikban szereplő nevek). Két dolgot érdemes tudnod, mielőtt elolvasod:
- **A racsni megnehezíti a referencia bővítését.** Ha egy PR új, nálunk hiányzó nevet visz a referenciába, a jelenlét csökken, és a küszöb alá eshet. Az ilyen PR-nak a funkciót is hoznia kell.
- **A mért jelenlét a valósnál magasabb lesz.** A doksik főleg a már meglévő funkciókat írják le, ezért a jelentés kiírja a referencia tételszámát és forrásait.

Kódot nem írtam. A terv a `/root/.claude/plans/cfa4-parity-g7-a-cargo-replicated-beaver.md` fájlban is megvan. Minden elfogadási feltétel állapota **PENDING**: VERIFIED csak lefutott CFA4-teszt vagy CI után lehet.

## Kontextus
- **Mai állapot:** a `cargo xtask parity` (`xtask/src/parity.rs`) a menüsort (#781, egui `menu_tree`), valamint az effekteket és átmeneteket (#831, `effect_catalog`) méri. A jelenlétet és az elhelyezést külön jelenti, `--min` küszöbbel.
- **Miért nem fut CI-ban:** a referencia a gitignore-olt `plan/premiere/` alatt van.
- **Hűségi adat nincs:** strukturált exact / approximate / stub jelölés sehol nincs a kódban.
- **Korlátok:**
  - AGENTS.md §1–2: nincs Adobe-asset, és a `plan/` tartalma nem kerül a repóba.
  - CI: fordító job csak `workflow_dispatch`, `merge_group` vagy `cfa4-full-validation` címke esetén futhat.
  - A rétegszabályok érvényesek, és a never crash szabály miatt minden bemenet ellenségesnek számít.
- **SK-F5 pilot:** a terv kiszolgálja az AC-SK-09-et (követelmény–teszt mátrix). Az AC-SK-08 (költség) a költségbaseline-ra vár.

## Követelmények
| ID | Forrás | Követelmény |
|---|---|---|
| G7-R1 | G7/1 | `--area commands`: a `command_specs()` mérése. A saját útvonal a `menu` + `label`, menü nélkül a `(no menu)` csoportba kerül. A lekérdező parancsok (`journal == false`) nem számítanak. A referencia lehet útvonal vagy puszta név. |
| G7-R2 | G7/1, Q1 | `--area panels` a **Qt6 GUI** paneljeit méri. A forrás egy új, Qt-mentes `python/cfa4_qt/panel_registry.py`, egyetlen igazságforrásként: a Window menü `PANELS` listája (átkerül a `window_menu.py`-ból), és a ténylegesen dokkolt panelek (`DOCK_PLAN` + a központi Program). A `main_window.py` és a `window_menu.py` innen importál. A dump parancsa `python3 -m cfa4_qt.panel_registry`, JSON kimenettel. Az egui `PanelKind` nem forrás. |
| G7-R3 | G7/1 | `--area all`: mind az öt területet lefuttatja, és egy összesített jelentést ír, elöl összesítő táblázattal. |
| G7-R4 | G7/2 | Hűségi szintek: exact, approximate, stub. A forrás a commitolt `xtask/parity/fidelity.json`, saját azonosítóval kulcsolva. Annotáció nélkül a tétel `exact`. A jelentés kimondja, hogy az `exact` csak „nincs ismert eltérés”, a viselkedés nincs ellenőrizve (G1). |
| G7-R5 | G7/2, Q1 | Automatikus `stub` a paneleknél: ha egy panel a menüben szerepel, de nincs dokkja („Not yet in the Qt GUI”), vagy csak helyőrző dokkja van (Libraries), akkor `stub`. Ezt a dump jelzi, nem kézi annotáció. |
| G7-R6 | G7/2 | Az annotációk validálása: csak a három szint fogadható el; a nem `exact` szinthez kötelező a `note`; ismeretlen id esetén hiba. A menük a parancs-id-n keresztül öröklik a szintet. |
| G7-R7 | G7/2 | A jelentés szintenkénti darabszámot mutat (összesen és csoportonként), és külön listázza az Approximate és a Stub tételeket a `note` szövegével. Új opcionális küszöb: `--min-exact`. |
| G7-R8 | G7/3, Q3 | Commitolt referencia: `xtask/parity/reference/<area>.json`, `{"schema":1,"area","outOfScope","items":[{"path":"File ▸ Export ▸ Media","source":"docs/ui-parity.md"}]}`. Csak szöveges útvonalak, és minden tételnek van `source` mezője. |
| G7-R9 | Q3 | `--check-reference` (a `cargo xtask ci` része): a `source` egy git-követett `docs/**/*.md` fájl, nem a `plan/` alatt, és a tétel minden útvonalszegmense (`norm()` után) szerepel abban a fájlban. Ezen felül: rögzített séma, csak stringek, méretkorlátok, nincs `data:` / URL / base64, nincs ismétlődés. |
| G7-R10 | G7/3 | A referencia feloldási sorrendje: `--reference`, utána a helyi `plan/premiere/`, végül a commitolt fájl. A `--committed` kapcsoló csak a commitolt fájlt használja. A jelentés megnevezi a forrást. Nem commitolt referenciával a jelentés csak a `target/` alá írható. |
| G7-R11 | G7/4, Q2 | Küszöbök a `xtask/parity/thresholds.json`-ban. A kezdőérték a bevezetéskor mért érték, 0,1-re lefelé kerekítve. A `--ci` = `--area all --committed` plusz ezek a küszöbök; minden területet lefuttat, a végén együtt listázza a hibákat, és jelzi, ha egy küszöb emelhető (automatikusan nem szerkeszt). |
| G7-R12 | Q2 | Racsni: `--check-ratchet --base REF` összeveti a küszöböket a base verzióval. Hiba, ha bármely érték csökken, vagy egy terület vagy `minExact` kulcs eltűnik. Ha a base-ben még nincs ilyen fájl, az nem hiba. |
| G7-R13 | G7/4 | `parity` job a `ci-linux.yml`-ben, a drága jobokéval azonos `if:` feltétellel. Futtatja a `--check-ratchet` (a PR vagy merge group base SHA ellen, `workflow_dispatch` esetén `origin/main` ellen) és a `--ci` lépést, a jelentést artifactként feltölti, és felkerül a `full-validation-gate` `needs:` listájára. |
| G7-R14 | G7/5 | Commitolt `docs/parity-checklist.md`, csak a commitolt referenciából, a fejlécben dátummal és commit-hash-sel, determinisztikus sorrendben. Nem drift-gate. |
| G7-R15 | G7 | Dokumentáció: a G7 szakasz, a `target-app-parity.md`, a `testing.md`, a `qt-gui.md`, valamint az xtask usage-szövege és modul-dokumentációja. |
| G7-R16 | AGENTS §0 | Never crash: korlátos olvasás (a meglévő korlátok, plusz annotáció ≤ 20 000 és note ≤ 1 KiB). Hibás bemenetre `Err` jön, pánik nem. Ha a Python-dump nem fut, a hibaüzenet megnevezi az `--ours FILE` kerülőutat. |

## Elfogadási feltételek (PENDING)
- **AC-1 (R1):** a `--area commands --committed` jelentést ír; a `command.list` nincs a „CFA4 only” listában.
- **AC-2 (R2):** a panels terület a registry-dumpot használja, cargo-fordítás nélkül. A `main_window` dokknevei azonosak a registry `docked` halmazával (pytest).
- **AC-3 (R5):** a dokk nélküli, de listázott panel `stub`, a „Not yet in the Qt GUI” megjegyzéssel. A Libraries `stub`.
- **AC-4 (R3):** a `--area all` egy fájlt ír öt szakasszal és összesítő táblázattal.
- **AC-5 (R4, R6, R7):**
  - Egy approximate fixture-annotáció a note-tal megjelenik a listában, és az összegzés `approximate 1`-et mutat.
  - Ismeretlen id-re és note nélküli nem `exact` szintre hiba jön.
  - A `--min-exact` az exact-arány fölött nem nulla kilépési kódot ad.
- **AC-6 (R8, R9):** a commitolt referenciákra a check zöld. Piros, ha:
  - egy név nem szerepel a megadott `source` doksiban;
  - a `source` a `plan/` alatt van, vagy nincs git-követve;
  - a fixture-ben shortcut-mező vagy `data:` elem van.
- **AC-7 (R10):** `plan/` nélküli klónban a commitolt referencia fut. Helyi referenciával a `--out docs/x.md` hibát ad.
- **AC-8 (R11, R12):** a racsni-ellenőrzés így viselkedik:
  - egy küszöböt 0,1-gyel csökkentve piros, emelve zöld;
  - egy terület törlése piros;
  - ha a base-ben nincs `thresholds.json`, zöld.
- **AC-9 (R13):** egy 100-ra állított küszöbbel a CI piros, az eredetivel zöld, és a jelentés letölthető artifact.
- **AC-10 (R14):** kétszeri generálás után a `docs/parity-checklist.md`-ben csak a fejléc tér el.
- **AC-11:** a kapuk zöldek: `cargo xtask ci`, workspace-tesztek, `ruff`, `pytest` (a `test_qt_gui.py` nélkül).
- **AC-SK-09:** minden G7-R-azonosítóhoz van teszt vagy CI-lépés (lásd a mátrixot).

## Technikai terv
1. **Refaktor a `parity.rs`-ben:** a saját oldal `OurItem { path, id, tier }` lesz, és a `compare` a talált saját tételt adja vissza. Az `Area` két új ága `Commands` és `Panels`. Ha a fájl 1000 sor fölé nő, `xtask/src/parity/` almodulokra bontjuk (`args`, `reference`, `fidelity`, `ratchet`, `report`), külön, viselkedést nem változtató commitban.
2. **Parancsdump:** `crates/engine/examples/command_list.rs` → `[{id,label,menu,query}]`, a `menu_tree` mintájára.
3. **Panel registry (Q1):**
   - A `panel_registry.py` csak a standard könyvtárat használja. Benne van a `PanelEntry`, a `PANELS`, a `DOCKED` és a `PLACEHOLDERS`.
   - A `window_menu.py` újraexportálja a `PANELS`-t, a `main_window.py` `DOCK_PLAN`-ja ehhez ellenőrzött.
   - A dump kimenete `[{title,shortcut,status:"docked"|"placeholder"|"listed"}]`.
   - Az xtask a `.venv/bin/python`-t használja, ha van, különben a `python3`-at, `-P` kapcsolóval és `PYTHONPATH=python` beállítással.
4. **Referencia v1 (Q3):** `{"path","source"}` tételek, `▸` vagy `>` elválasztóval. A `check_reference` a `git ls-files docs` alapján dolgozik, és bekerül a `ci()`-be.
5. **Hűség:** `Tier` enum és `fidelity.json` betöltő. A szintek a riport sorainál, az összegzésben és a markdownban jelennek meg. A `--min-exact` a `presence` mintájára készül.
6. **Új kapcsolók:** az `Args` kiegészül a `committed`, `ci`, `min_exact`, `check_reference`, `check_ratchet` mezőkkel és az egy terület / mind választással. Az `--out` védelme R10 szerint.
7. **Kezdő adatok:**
   - Referencia: neveket csak a commitolt doksikból veszünk (`target-app-parity.md`, `keyboard.md`, `ui-parity.md`, `qt-parity.md`, `hardware-parity.md`, `gaps.md` stb.).
   - `fidelity.json`: a G18 közelítéseiből (Warp Stabilizer 2-D, Auto Reframe, Morph Cut, Immersive Video átmenetek, Optical Flow CPU-n).
   - `thresholds.json`: a bevezetéskor mért értékek.
8. **CI:** `parity` job `fetch-depth: 0`-val, SHA-pinned `upload-artifact`-tal, és felkerül a `full-validation-gate` `needs:` listájára.
9. **Dokumentáció:** a `docs/parity-checklist.md` generálása és a doksik frissítése (R15).

Kritikus fájlok: `xtask/src/parity.rs`, `xtask/src/main.rs`, `crates/engine/examples/command_list.rs`, `python/cfa4_qt/{panel_registry,window_menu,main_window}.py`, `xtask/parity/{reference/*.json,fidelity.json,thresholds.json}`, `.github/workflows/ci-linux.yml`, `docs/parity-checklist.md`.

## Feladatlista (egy commit = egy feladat, `G7.n: …`)
| # | Feladat | Tesztek |
|---|---|---|
| G7.1 | `OurItem` refaktor (a viselkedés nem változik) | a meglévő 7 teszt zöld; `compare_returns_our_item_for_same_and_elsewhere` |
| G7.2 | `--area commands` + `command_list` | `command_dump_skips_queries`, `commands_reference_bare_and_menu_paths`, `parse_args_accepts_commands` |
| G7.3 | `panel_registry.py` kiemelése, a `window_menu` és a `main_window` átállítása | `test_panel_registry_is_qt_free` (alfolyamat, nincs PySide6), `test_registry_dump_json`, `test_docked_matches_dock_plan`; a meglévő Window-menü teszt zöld marad |
| G7.4 | `--area panels`, automatikus stub | `panels_dump_status_maps_to_tier`, `panels_reference_bare_names`, `python_missing_is_error_not_panic` |
| G7.5 | v1 séma, `outOfScope`, `--check-reference` eredet-ellenőrzéssel, bekötés a `ci()`-be | `schema_v1_path_source_items`, `check_reference_rejects_{name_not_in_source,source_under_plan,untracked_source,shortcut_fields,data_uri_and_base64,non_string}`, hostile-teszt v1-re |
| G7.6 | Hűség és `--min-exact` | `tier_defaults_to_exact`, `approximate_and_stub_listed_with_note`, `unknown_fidelity_id_is_error`, `note_required_for_non_exact`, `menus_inherit_command_tiers`, `min_exact_fails_below`, `markdown_tier_columns` |
| G7.7 | `--area all`, `--committed`, feloldási sorrend, `--out` védelem | `resolution_order_reference_local_committed`, `all_writes_five_sections`, `report_names_reference_source`, `local_reference_cannot_write_into_repo` |
| G7.8 | Commitolt referenciák (csak doksinevek), `fidelity.json` | `committed_references_pass_check`, `committed_fidelity_ids_exist` |
| G7.9 | `thresholds.json`, `--ci`, `--check-ratchet` | `ci_mode_reports_all_failing_areas`, `ratchet_{decrease_fails,increase_ok,removed_area_fails,removed_min_exact_fails,missing_base_ok}`, `raise_hint_when_above_min` |
| G7.10 | `parity` CI-job | actionlint és zizmor; címkés PR-futás egyszer zöldre, egyszer csökkentett küszöbbel pirosra |
| G7.11 | `docs/parity-checklist.md` és dokumentáció | determinisztikus újragenerálás; `assets`, ruff és pytest zöld |

**Sorrend:** G7.1, majd párhuzamosan G7.2 és G7.3, utána G7.4 → G7.5 → G7.6 → G7.7 → G7.8 → G7.9 → G7.10 → G7.11.

## Követelmény–teszt mátrix (AC-SK-09)
| R | Feladat | Teszt / CI |
|---|---|---|
| R1 | G7.2 | `command_dump_skips_queries`, `commands_reference_bare_and_menu_paths` |
| R2 | G7.3–4 | `test_panel_registry_is_qt_free`, `test_docked_matches_dock_plan`, `panels_reference_bare_names` |
| R3 | G7.7 | `all_writes_five_sections` |
| R4 | G7.6 | `tier_defaults_to_exact`, `markdown_tier_columns` |
| R5 | G7.4 | `panels_dump_status_maps_to_tier` |
| R6 | G7.6 | `unknown_fidelity_id_is_error`, `note_required_for_non_exact`, `menus_inherit_command_tiers` |
| R7 | G7.6 | `approximate_and_stub_listed_with_note`, `min_exact_fails_below` |
| R8 | G7.5, G7.8 | `schema_v1_path_source_items`, `committed_references_pass_check` |
| R9 | G7.5 | `check_reference_rejects_*` (6 eset), `cargo xtask ci` |
| R10 | G7.7 | `resolution_order_…`, `local_reference_cannot_write_into_repo` |
| R11 | G7.9 | `ci_mode_reports_all_failing_areas`, `raise_hint_when_above_min` |
| R12 | G7.9–10 | `ratchet_*` (5 eset), a CI `--check-ratchet` lépése |
| R13 | G7.10 | címkés PR-futás (zöld / piros) |
| R14 | G7.11 | determinisztikus újragenerálás |
| R15 | G7.11 | review |
| R16 | G7.4–6 | hostile-teszt (v1), `python_missing_is_error_not_panic` |

## Ellenőrzés
Commitonként a `cfa4-ci-runner` subagent futtatja a kapukat: `cargo xtask ci`, workspace-tesztek, ruff, pytest. A végén `plan/` nélküli worktreeben ennek kell zöldnek lennie:

```
cargo xtask parity --check-ratchet --base origin/main && cargo xtask parity --ci
```

Ezután jön a címkés PR-futás.

## Nyitott pontok
1. **Racsni és a referencia bővítése:** a fent leírt ütközés miatt, ha ez túl merevnek bizonyul, dönthetsz a hiányzó tételek darabszámára épülő racsniról. Amíg nem döntesz, a Q2 marad.
2. **Q3 torzítása:** a doksiból vett referencia felfelé torzítja a jelenlétet, mert a doksik főleg meglévő funkciókat írnak le. A `plan/premiere/` helyi méréshez továbbra is használható (`--reference`), de az eredmény csak a `target/` alá kerülhet.
3. **Menük:** a `--area menus` továbbra is az egui menüsort méri. A Q1 csak a panelekre szól, a Qt-menüsor mérése külön feladat lehet.
4. **SK-F5:** a megvalósításhoz külön engedély kell, és az AC-SK-08 a költségbaseline-ra vár.