# Data model: Mért paritás bővítése (G7)

Minden fájl szöveges JSON, UTF-8. Minden bemenet ellenséges (AGENTS.md §0.4–0.5): a meglévő korlátok
(`MAX_DEPTH = 16`, `MAX_ITEMS = 20 000`, `MAX_PATH_BYTES = 4 KiB`, `xtask/src/parity.rs`) mindegyik új fájlra is
érvényesek.

## Area (terület)

| Mező | Érték |
|---|---|
| `name` | `menus` \| `effects` \| `transitions` \| `commands` \| `panels` |
| saját oldal | `menus`: `crates/ui-egui/examples/menu_tree.rs`; `effects` / `transitions`: `crates/project/examples/effect_catalog.rs`; `commands`: `crates/engine/examples/command_catalog.rs` (új); `panels`: `scripts/dev/qt_panel_catalog.py` (új, Qt6) |
| alapértelmezett referencia | `xtask/parity/reference/<name>.json`; a helyi `plan/premiere/<name>.json` csak `--reference`-szel (FR-006) |
| hatókörön kívül | `menus`, `effects`, `transitions`: változatlan (`out_of_scope`); `commands`: fiók-, felhő-, licenc- és fizetős tételek, más platform rendszerelemei, valamint a `Queries` csoport (FR-004); `panels`: fiók- és felhőpanelek |

## OurItem (saját tétel)

| Mező | Típus | Szabály |
|---|---|---|
| `id` | string | a területen egyedi (parancsazonosító, effekt-id vagy panelnév) |
| `path` | string[] | legalább 1 elem; parancsnál `menu + [label]`, menü nélkül `["Keyboard", label]` |
| `present` | bool | csak a `panels` területen; `false` = nincs Qt-dokk, ezért hiányzik |

## ReferenceItem (referenciatétel) – `xtask/parity/reference/<area>.json`

```json
{ "area": "commands",
  "items": [ { "path": ["Sequence", "Add Edit"], "source": "docs/keyboard.md" } ] }
```

| Mező | Szabály |
|---|---|
| `path` | 1–16 elem, mindegyik nem üres. A régi formák (`"A > B"` szöveg, fa) is elfogadottak, de a commitolt fájlban `{path, source}` objektumok állnak |
| `source` | **kötelező** a commitolt fájlban. Létező `docs/**/*.md` fájl, `plan/` alatti útvonal tilos (AGENTS.md §1.5, Q3) |
| ellenőrzés (FR-007a) | a levélcímke szó szerint, kis- és nagybetűre érzéketlenül szerepel a `source` szövegében, a `…` és a `...` egyenértékű. Ha nem, a betöltés hibát ad |

A `--reference`-szel megadott helyi fájlnál a `source` nem kötelező, és a FR-007a ellenőrzés nem fut.

## FidelityEntry (hűségi besorolás) – `xtask/parity/fidelity.json`

```json
{ "commands": { "sequence.addEdit": { "tier": "exact", "reason": "…", "evidence": ["crates/engine/src/…_tests.rs"] } } }
```

| Mező | Szabály |
|---|---|
| kulcs | terület → saját `id` |
| `tier` | `exact` \| `approximate` \| `stub`. Más érték: betöltési hiba |
| `reason` | nem üres, legfeljebb 200 karakter |
| `evidence` | `exact`-nál **legalább 1** elem (VISION.md 2.5), máshol nem kötelező. Egy elem lehet létező repóbeli fájl, vagy `#<szám>` alakú PR-hivatkozás. Hiányzó fájl: betöltési hiba |
| árva besorolás | olyan kulcs, amelyhez nincs saját tétel: a jelentés külön listázza, a CI figyelmeztet |

**Szintek a jelentésben:** egy megtalált tétel (*ugyanott* vagy *máshol*) szintje
`exact` / `approximate` / `stub` / `unrated` (nincs besorolása). Hiányzó tételnek nincs szintje.

## Threshold (küszöb) – `xtask/parity/thresholds.json`

```json
{ "commands": { "presence": 61.4, "exact": 12.0 } }
```

| Mező | Szabály |
|---|---|
| `presence`, `exact` | 0–100, egy tizedesjegy, lefelé kerekítve |
| racsni (FR-016a) | a `main`-en lévő fájlhoz képest egyetlen érték sem csökkenhet, és terület sem tűnhet el |

## Report (jelentés)

Területenként:

- referencia: az útvonal, a tartalom-hash, a tételszám, és forrásonként a darabszám;
- összesítő: `in scope`, `same`, `elsewhere`, `missing`, `presence %`;
- szintek: `exact`, `approximate`, `stub`, `unrated`, és a pontos jelenlét százaléka (`exact %`);
- listák: a területen kívüli tételek okonként, a csak nálunk meglévő tételek, az árva besorolások.

A `presence %` = (`same` + `elsewhere`) / `in scope`, az `exact %` = az `exact` szintű megtalált tételek /
`in scope`. Ha `in scope` = 0, mindkettő 0, osztás nélkül.

## Állapotátmenetek

A hűségi szint `unrated` → `approximate` | `stub` → `exact`, mindig commitolt változtatással. Az `exact`-ból
való visszalépés megengedett, mert a racsni a küszöbre vonatkozik, nem az egyes tételekre. A visszalépés
csökkentheti az `exact %`-ot, és a küszöb alá esve a CI elbukik. Ez a szándékolt viselkedés.
