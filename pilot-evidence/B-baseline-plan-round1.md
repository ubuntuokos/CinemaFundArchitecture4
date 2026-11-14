# CFA4-PARITY-G7: a `cargo xtask parity` bővítésének terve

Kódot nem írtam. A terv a `/root/.claude/plans/cfa4-parity-g7-a-cargo-replicated-beaver.md` fájlban van.

## Kontextus
- **Mai állapot:** a `xtask/src/parity.rs` (754 sor) három területet mér: menüket, effekteket és átmeneteket. A jelenlétet (same place / elsewhere / missing) és az elhelyezést külön jelenti, `--min` küszöbbel.
- **Miért nem fut CI-ban:** a referencia a helyi, gitignore-olt `plan/premiere/` alatt van. Ha hiányzik, az eszköz csak a saját listánkat írja ki, a `--min` pedig hibát ad.
- **Hűségi adat nincs:** a kódban sehol nincs strukturált exact / approximate / stub jelölés. A közelítések csak szövegként szerepelnek (`docs/gaps.md` G18, `docs/target-app-parity.md`).
- **A CI korlátja (AGENTS.md):** a fordító job csak `workflow_dispatch`, `merge_group` vagy `cfa4-full-validation` címke esetén futhat.

## Követelmények
| ID | Követelmény |
|---|---|
| R1 | `--area commands`: a `cfa4_engine::commands::command_specs()` listát méri. A saját útvonal a `menu` + `label`, menü nélkül a `(no menu)` csoportba kerül. A lekérdező parancsok (`journal == false`) nem számítanak. A referencia lehet útvonal vagy puszta név. |
| R2 | `--area panels`: a `PanelKind::ALL` címeit méri (a Qt `PANELS` ugyanezeket használja), puszta nevekkel. |
| R3 | `--area all`: mind az öt területet lefuttatja, és egy összesített jelentést ír, elöl összesítő táblázattal. |
| R4 | Hűségi szintek a commitolt `xtask/parity/fidelity.json` alapján, saját azonosítóval kulcsolva. Annotáció nélkül a tétel `exact`. A jelentés kimondja, hogy az `exact` csak annyit jelent: „nincs ismert eltérés”, a viselkedés nincs ellenőrizve (G1). |
| R5 | Az annotációk validálása: csak a három szint fogadható el; a nem `exact` szinthez kötelező a `note`; ismeretlen id esetén hiba. A menük a parancs-id-n keresztül öröklik a szintet. |
| R6 | A jelentés jelenlétet és szintenkénti darabszámot is mutat (összesen és csoportonként), és külön listázza az „Approximate” és a „Stub” tételeket a `note` szövegével. |
| R7 | Új küszöb: `--min-exact PERCENT`. A `--min` változatlan marad. |
| R8 | Commitolt referencia: `xtask/parity/reference/<area>.json`, `schema: 1`, `provenance`, `outOfScope`, `items`. Csak útvonal-stringek; nincs gyorsbillentyű, id, URL vagy base64. |
| R9 | A referencia feloldási sorrendje: `--reference`, utána a helyi `plan/premiere/`, végül a commitolt fájl. A jelentés megnevezi a forrást. A `--committed` kapcsoló kihagyja a helyi fájlt. |
| R10 | `--check-reference` szigorú validátor, amely bekerül a `cargo xtask ci`-be. |
| R11 | Küszöbök a commitolt `xtask/parity/thresholds.json`-ban. A `--ci` = `--area all --committed` plusz ezek a küszöbök. Minden területet lefuttat, és a végén együtt listázza a hibákat. |
| R12 | `parity` job a `ci-linux.yml`-ben, ugyanazzal a feltétellel, mint a többi drága job. A jelentést artifactként tölti fel, és felkerül a `full-validation-gate` `needs:` listájára. |
| R13 | Commitolt `docs/parity-checklist.md`, a `--ci --out` generálja, a fejlécben dátummal és commit-hash-sel. Nem drift-gate. |
| R14 | A dokumentáció frissítése: G7, a `target-app-parity.md` mérési szakasza, a `testing.md`, valamint az xtask usage-szövege és modul-dokumentációja. |
| R15 | Soha nem omlik össze: minden új bemenetre a meglévő korlátok érvényesek (`MAX_DEPTH`, `MAX_ITEMS`, `MAX_PATH_BYTES`), hibás bemenetre `Err` jön, pánik nem. |

## Elfogadási feltételek
- **A1 (commands):** a `--area commands --committed` lefut és jelentést ír. A `command.list` nem szerepel a „CFA4 only” listában.
- **A2 (panels):** a `--area panels` 28 saját panelt lát.
- **A3 (all):** a `--area all` egy fájlt ír öt szakasszal és összesítő táblázattal.
- **A4 (szintek):** ha egy fixture-annotáció egy effektet `approximate`-re állít, az a note-tal megjelenik a listában, és az összegzés `approximate 1`-et mutat. Ismeretlen id-re hiba jön.
- **A5 (`--min-exact`):** az exact-arány fölötti értékre nem nulla a kilépési kód, és az üzenet tartalmazza a százalékot.
- **A6 (`plan/` nélkül):** `plan/` nélküli klónban a commitolt referencia fut, a jelentés „committed reference”-t ír. A `--check-reference` a commitolt fájlokra zöld; shortcut-mezős vagy `data:` elemet tartalmazó fixture-re piros.
- **A7 (CI):** ha egy küszöböt 100-ra állítunk, a `parity` job piros. Az eredeti értékekkel zöld, és a jelentés letölthető artifact.
- **A8 (determinizmus):** újragenerálva a `docs/parity-checklist.md`-ben csak a fejléc tér el.
- **A9 (kapuk):** `cargo xtask ci`, a workspace-tesztek és a `cargo xtask assets` mind zöldek. A JSON nem asset, az `ASSET_EXT` nem változik.

## Technikai terv
1. **Refaktor a `parity.rs`-ben:** a `Vec<Vec<String>>` helyére `OurItem { path, id }` kerül, és a `compare` visszaadja a talált saját id-t, hogy a szint hozzárendelhető legyen. Az `Area` két új ága `Commands` és `Panels`, az összes `match` kiterjesztve. Ha a fájl 1000 sor fölé nő, `parity/` almodulokra bontjuk, külön commitban.
2. **Új dump-példák:** `crates/engine/examples/command_list.rs` (`[{id,label,menu,query}]`) és `crates/ui-egui/examples/panel_list.rs` (`[{id,title}]`). Mindkettő a meglévő crate-ben van, így a rétegszabály nem sérül. A `dump_ours` mindkettőt ismeri.
3. **Referencia v1 séma:** a `reference_paths` felismeri a `{"schema":1}` formátumot, az `items` a meglévő `walk` úton megy. Az `outOfScope` saját indoklást kap a beégetett szabályok mellett. A `check_reference` bekerül a `ci()`-be.
4. **Hűség:** új `Tier` típus és `fidelity.json` betöltő, korlátokkal. A szintek a riport sorainál, az összegzésben és a markdownban jelennek meg. A `--min-exact` a `presence` mintájára készül.
5. **Új kapcsolók:** az `Args` kiegészül a `committed`, `ci`, `min_exact` mezőkkel és az egy terület / mind választással. A `--ci` beolvassa a `thresholds.json`-t.
6. **Kezdő adatok:** öt referencia-fájl, csak útvonal-stringekkel. A `fidelity.json` a G18 közelítéseiből indul: Warp Stabilizer 2-D, Auto Reframe, Morph Cut, Immersive Video átmenetek, Optical Flow CPU-n; a pontos id-ket a dumpból keressük ki. A küszöb az első mérés mínusz 2 százalékpont, lefelé kerekítve.
7. **CI:** `parity` job a `rust-architecture-assets` mintájára, SHA-pinned `upload-artifact`-tal, a `full-validation-gate` `needs:` listáján.
8. **Dokumentáció:** a `docs/parity-checklist.md` generálása, valamint a G7, a `target-app-parity.md` és a `testing.md` frissítése.

## Feladatlista (egy commit = egy feladat, `G7.n: …`)
| # | Feladat | Tesztek |
|---|---|---|
| G7.1 | `OurItem` refaktor, id átvezetése (a viselkedés nem változik) | a meglévő 7 teszt zöld marad; új: `compare_returns_our_id_for_same_and_elsewhere` |
| G7.2 | `--area commands` + `command_list` példa | `command_dump_skips_queries`, `commands_reference_bare_and_menu_paths`, parse_args |
| G7.3 | `--area panels` + `panel_list` példa | `panel_dump_has_all_28`, `panels_reference_bare_names` |
| G7.4 | v1 séma, `outOfScope`, `--check-reference`, bekötés a `ci()`-be | `schema_v1_items_and_out_of_scope`, `check_reference_rejects_{shortcut_fields,data_uri_and_base64,non_string}`, a hostile-teszt kiterjesztése v1-re |
| G7.5 | Szintek, `fidelity.json`, riport, `--min-exact` | `tier_defaults_to_exact`, `approximate_and_stub_listed_with_note`, `unknown_fidelity_id_is_error`, `note_required_for_non_exact`, `menus_inherit_command_tiers`, `min_exact_fails_below`, `markdown_tier_columns` |
| G7.6 | `--area all`, `--committed`, feloldási sorrend, forrás a riportban | `resolution_order_reference_local_committed` (tempdir), `all_writes_five_sections`, `report_names_reference_source` |
| G7.7 | Commitolt referenciák, `fidelity.json`, `thresholds.json` | `committed_references_pass_check`, `committed_fidelity_ids_exist`, `thresholds_in_range` |
| G7.8 | `--ci` mód, összesített hibalista | `ci_mode_reports_all_failing_areas` (`--ours` fixture-ökkel, fordítás nélkül), `thresholds_missing_area_is_error` |
| G7.9 | CI-job, artifact, gate `needs:` | actionlint és zizmor (cfa4-gate); egy `cfa4-full-validation` címkés PR-futás |
| G7.10 | `docs/parity-checklist.md` és a dokumentáció | kétszeri generálás után csak a fejléc tér el; `cargo xtask assets` zöld |

**Sorrend:** G7.1, aztán G7.2–G7.4, majd G7.5 → G7.6 → G7.7 → G7.8 → G7.9 → G7.10. A kapukat minden commit előtt a `cfa4-ci-runner` subagent futtatja. Az xtask a `default-members` között van, így a `cargo test --workspace` lefuttatja a parity-teszteket is.

## Ellenőrzés a végén
- `plan/` nélküli worktreeben a `cargo xtask parity --ci` zöld, és öt szakaszos jelentést ír.
- Egy küszöböt 100-ra emelve piros, nem nulla kilépési kóddal.
- A `cfa4-full-validation` címkés PR-on a `parity` job artifactot ad.

## Három döntés, amit neked kell meghoznod
1. **Honnan jöjjön a commitolt referencia?** Ha nem szólsz, kézzel írt útvonal-stringekből indulok, nyilvános dokumentáció alapján, `provenance` mezővel. Gyorsabb lenne csak az útvonalakat kivonatolni a `plan/premiere/*.json`-ból, de az AGENTS.md §1.5 miatt ez a te döntésed.
2. **Mi a panelek forrása?** Ha nem szólsz, az egui `PanelKind::ALL`. Hogy a Qt-ben még hiányzó panelek `stub` szintet kapjanak, azt későbbre hagynám.
3. **Legyen-e drift-gate a `docs/parity-checklist.md`-n?** Ha nem szólsz, nem lesz, mert a párhuzamos PR-ok ütköznének rajta.

A tervezési munkafolyamat kérdező és plan-jóváhagyó eszköze ebben a munkamenetben nem érhető el. Ha ezekre válaszolsz és jóváhagyod a tervet, a G7.1-gyel kezdem.