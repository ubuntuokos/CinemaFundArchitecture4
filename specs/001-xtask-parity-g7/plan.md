# Implementation Plan: Mért paritás bővítése – parancsok, panelek, hűségi szintek, commitolt referencia, CI (G7)

**Branch**: nincs (a Spec Kit nem nyit ágat; a setup-szkript `001-xtask-parity-g7` néven csak címkét adott) | **Date**: 2026-11-14 | **Spec**: [spec.md](spec.md)

**Input**: a `specs/001-xtask-parity-g7/spec.md` specifikáció, a 2026-11-14-i tulajdonosi döntésekkel (Q1–Q3).

**Jóváhagyás:** csak tervezésre (CFA4-DEV-SPEC-KIT-001 SK-F5 pilot). A megvalósítás jóváhagyása PENDING; a
`/speckit-implement` tiltott.

## Summary

A `cargo xtask parity` két új területet kap.

- **`commands`:** saját oldala a motor parancsregisztere, amelyet egy új példaprogram, a
  `crates/engine/examples/command_catalog.rs` ír ki.
- **`panels`:** saját oldala a Qt6 GUI dokktáblája, amelyet egy új fejlesztői szkript, a
  `scripts/dev/qt_panel_catalog.py` ír ki.

Minden megtalált tétel hűségi szintet kap a commitolt `xtask/parity/fidelity.json`-ból: `exact` (csak
bizonyítékkal), `approximate`, `stub` vagy `unrated`. A mérés alapértelmezett referenciája a commitolt
`xtask/parity/reference/<area>.json` lesz. Ennek minden tétele egy commitolt `docs/` dokumentumra hivatkozik,
és a mérő ellenőrzi, hogy a név ott tényleg szerepel (Q3).

A `ci-linux.yml` egy új `parity` jobja mind az öt területet méri. A küszöbök az `xtask/parity/thresholds.json`
fájlban vannak, és csak felfelé mozdulhatnak (racsni, Q2). A job a `docs/parity-checklist.md`-t
determinisztikusan generálja, és figyelmeztet, ha a commitolt példány elavult.

A részletes döntések a [research.md](research.md)-ben vannak.

## Technical Context

- **Nyelv:** Rust (workspace-toolchain, `rust-toolchain.toml`) az `xtask`-hoz és a motorpéldához; Python 3
  (`.venv`) a Qt-panelszkripthez.
- **Függőségek:** `serde_json` (már megvan az `xtask`-ban), `cfa4-engine` (csak a példaprogramnak), PySide6
  (már megvan a `.venv`-ben). **Új függőség nincs**, a `Cargo.toml` és a `Cargo.lock` nem változik.
- **Tárolás:** commitolt JSON-fájlok az `xtask/parity/` alatt; a generált jelentés a `target/parity/` alá
  kerül, a checklist a `docs/parity-checklist.md`-be.
- **Tesztelés:** `cargo test -p xtask` (egységtesztek a `parity.rs`-ben, a meglévő mintára), Python `pytest` a
  panelszkripthez, és a `ci-linux.yml` `parity` jobja.
- **Célplatform:** Linux x86_64, Ubuntu 26.04 (a CI-futó `ubuntu-26.04`).
- **Projekttípus:** fejlesztői CLI-eszköz (`xtask`) és CI.
- **Teljesítménycél:** az ötterületes mérés előfordított állapotból legfeljebb 2 perc a referenciagépen
  (SC-002).
- **Korlátok:**
  - a meglévő területek viselkedése nem változik (FR-021);
  - az automatikus PR-ellenőrzés nem bővül (AGENTS.md CFA4-szakasz);
  - nincs `plan/` a repóban (§1.5);
  - nincs `unwrap`/`panic`, és ellenséges bemenetekre korlátok vannak (§0).
- **Méret:** 5 terület, 193 regisztrált parancs (a csak billentyűs parancsokat lásd: research R1), 27 Qt-panel,
  a referencia várhatóan néhány száz tétel.

## Constitution Check

*GATE: a 0. fázis előtt teljesül; az 1. fázis után újraellenőrizve.*

A constitution csak hivatkozás. A forrásai: AGENTS.md, VISION.md, docs/coordination.md, docs/licensing.md,
CLAUDE.md.

| Kapu | Forrás | Eredmény |
|---|---|---|
| Soha ne omoljon össze: `Result`, nincs `unwrap`, ellenséges bemeneti korlátok, ellenséges-bemeneti tesztek | AGENTS.md §0.1–0.5, §0.8 | ✅ FR-020; a data-model.md a meglévő korlátokat az új fájlokra is kiterjeszti; a quickstart 7. lépése teszteli |
| Nincs Adobe-asset; a helyi referencia (`plan/premiere/`) nem kerül a repóba | AGENTS.md §1, §1.5 | ✅ csak szöveges nevek commitolt `docs/` forrásból (Q3); a FR-007a ellenőrzés tiltja a `plan/` forrást |
| Tisztaszobás munka | AGENTS.md §2 | ✅ nincs átvett kód; a nevek a saját dokumentációból jönnek |
| Licenc, új függőség | AGENTS.md CFA4, docs/licensing.md | ✅ nincs új függőség (a docs/licensing.md-t ezért nem kellett beolvasni) |
| CI: automatikusan csak a `cargo-check` fut | AGENTS.md CFA4 (CI) | ✅ a `parity` job csak a teljes validációban fut (R6) |
| Qt6 az első; az egui befagyasztva | AGENTS.md CFA4, VISION 2.1 | ✅ a `panels` terület a Qt6-ot méri (Q1); az egui forrása nem változik |
| „Bizonyíték, nem állítás” | VISION 2.5 | ✅ `exact` szint csak `evidence`-szel (R3) |
| Rétegzés (`cargo xtask layers`) | CLAUDE.md, architecture §1 | ✅ a motorpélda csak a `cfa4-engine`-t használja; az `xtask` függőségei nem változnak |
| Egy PR = egy téma; közös fájlokban csak a saját sor | coordination §2.2, §2.5 | ✅ hat PR (lásd a PR-bontást lent); a referenciafájlokban soronkénti hozzáadás |
| CodeRabbit kockázatos PR-on (`.github/`) | coordination §2.9 | ✅ a PR-5 „CodeRabbit: kell” |
| Projekthatár | coordination §2.8 | ✅ csak a G7; a beállítások és formátumok kimaradnak (FR-022); a motor `EngineError` bővítése nem része (D1) |
| Spec Kit: nincs implement, nincs ág, commit vagy PR; a validation PENDING | constitution | ✅ ez a terv nem ír kódot, és nem nyit ágat |
| Fordítások csak az `i18n/`-ben | AGENTS.md CFA4 | ✅ nem érinti: a mérés forrásnyelvi címkéket használ |

**Újraellenőrzés az 1. fázis után:** minden kapu teljesül, megsértett kapu nincs, ezért a Complexity Tracking
üres. Egy pont nyitott, de az nem kapu-sértés, hanem a specifikáció pontosítása: **D1**, lásd lent.

## Nyitott pont – tulajdonosi döntés kell (D1)

**FR-011** („a motor által nem támogatottnak jelzett parancs automatikusan csonk”) ma nem valósítható meg, mert
a motornak nincs géppel olvasható „unsupported” jelzése (research R3).

**Javaslat:** a G7-ben a csonk szint csak a `fidelity.json`-ból jön. Az automatikus jelzés egy későbbi, külön
projektben kerül be (`EngineError::Unsupported`), ezért az FR-011 erre módosulna. Amíg nincs döntés, a
feladatlista az FR-011-et ezzel a feltétellel kezeli.

## Project Structure

### Documentation (this feature)

```text
specs/001-xtask-parity-g7/
├── spec.md
├── plan.md              # ez a fájl
├── research.md          # 0. fázis
├── data-model.md        # 1. fázis
├── quickstart.md        # 1. fázis
├── contracts/
│   ├── cli.md           # a cargo xtask parity kapcsolói, kilépési kódjai, naplója
│   ├── catalog-dumps.md # a parancs- és panelkatalógus JSON-kimenete
│   └── data-files.md    # a commitolt adatfájlok és a gazdájuk
├── checklists/requirements.md
└── tasks.md             # a /speckit-tasks kimenete (még nincs)
```

### Source Code (repository root)

```text
xtask/src/parity.rs                       # módosul: új területek, szintek, --all, küszöbök, racsni, checklist
xtask/src/parity/                         # új almodulok, ha a parity.rs túl nagy lenne (fidelity.rs, thresholds.rs, reference.rs)
xtask/parity/README.md                    # új: a referencia eredete és szabályai (Q3)
xtask/parity/reference/{menus,effects,transitions,commands,panels}.json   # új
xtask/parity/fidelity.json                # új
xtask/parity/thresholds.json              # új (a PR-5-ben)
crates/engine/examples/command_catalog.rs # új: a parancsregiszter JSON-ként
scripts/dev/qt_panel_catalog.py           # új: a Qt-dokktábla JSON-ként (Q1)
python/tests/test_qt_panel_catalog.py     # új
.github/workflows/ci-linux.yml            # módosul: parity job, full-validation-gate needs
docs/parity-checklist.md                  # új, generált
docs/gaps.md                              # módosul: a G7 Done/Missing sorai
```

**Structure Decision:** minden a meglévő `xtask` parity-modulra épül. A saját oldali katalógusok a forrásuk
mellé kerülnek, a meglévő `effect_catalog.rs` mintájára. A commitolt adatok az `xtask/parity/` alatt vannak,
nem a `docs/` alatt, így az adatfájlokra nem vonatkozik a `docs/*-parity.md` közösfájl-szabály. Az olvasható
eredmény a `docs/parity-checklist.md`.

## PR-bontás (egy PR = egy téma, mindegyik friss `main`-ből)

| PR | Tartalom | User story | CodeRabbit |
|---|---|---|---|
| PR-1 | commitolt referencia a meglévő 3 területre, `source`-ellenőrzés (FR-007a), alapértelmezett referencia (FR-006), README | US1 | nem kell |
| PR-2 | `commands` terület és a `command_catalog.rs` | US2 | nem kell |
| PR-3 | `panels` terület és a `qt_panel_catalog.py` | US2 | nem kell |
| PR-4 | hűségi szintek, `fidelity.json`, `--min-exact` | US3 | nem kell |
| PR-5 | `thresholds.json`, `--all`, `--ratchet`, a `ci-linux.yml` `parity` jobja | US4 | **kell** (`.github/`) |
| PR-6 | `--checklist` / `--check-checklist`, `docs/parity-checklist.md`, a gaps.md G7 frissítése | US5 | nem kell |

A PR-2 és a PR-3 a PR-1 után párhuzamosan haladhat. A PR-5 és a PR-6 az összes előzőre épül. A coordination
§2.1 szerint mindegyik csak az előző beolvasztása után indul.

## Complexity Tracking

Nincs megsértett kapu.
