# Quickstart: a G7 validálása

Ez a lépéssor a megvalósítás után igazolja a sikerkritériumokat. Kapcsolók: [contracts/cli.md](contracts/cli.md);
fájlok: [contracts/data-files.md](contracts/data-files.md); szabályok: [data-model.md](data-model.md).

## Előfeltételek

- Linux x86_64. A `.venv` a [docs/development/linux.md](../../docs/development/linux.md) szerint, PySide6-tal
  (ez a `panels` területhez kell).
- Friss klón **`plan/` nélkül**: `test ! -e plan`.

## 1. Mérés `plan/` nélkül (US1, SC-001)

```sh
for a in menus effects transitions commands panels; do cargo xtask parity --area "$a" || exit 1; done
```

Elvárt eredmény:

- minden terület egy `parity <area>: presence …` sort ír;
- egyik sem írja ki, hogy „no reference list”;
- a jelentés fejléce az `xtask/parity/reference/<area>.json`-t nevezi meg.

## 2. Az új területek (US2)

```sh
cargo run -q -p cfa4-engine --example command_catalog | head
PYTHONPATH=python .venv/bin/python scripts/dev/qt_panel_catalog.py | head
cargo xtask parity --area commands --reference target/fixtures/parity-commands-mini.json
```

Ehhez egy kézzel írt, háromtételes referencia kell, amely nincs commitolva. Az elvárt besorolás tételenként:

| Tétel | Besorolás |
|---|---|
| létező parancs a saját menüútvonalán | *same place* |
| létező parancs más menüben | *elsewhere*, a mi útvonalunkkal |
| nem létező parancs | *missing* |

## 3. Hűségi szintek (US3, SC-003)

Ellenőrizd, hogy a `target/parity/parity-commands.md` táblázatában van `Exact | Approximate | Stub | Unrated`
oszlop és `exact %`.

Negatív próba, csak egy ideiglenes másolaton: adj egy `"tier": "exact"` bejegyzést `evidence` nélkül a
`fidelity.json`-hoz.

```sh
cargo xtask parity --area commands
```

Elvárt eredmény: 1-es kilépési kód, és a hibaüzenet megnevezi a bejegyzést.

## 4. Commitolt referencia ellenőrzése (US1, FR-007a, SC-006, SC-010)

Negatív próba: egy referenciatétel `source` mezőjét állítsd olyan dokumentumra, amelyben a név nem szerepel,
vagy egy `plan/…` útvonalra.

Elvárt eredmény: a mérés hibát ad.

```sh
cargo xtask assets && .venv/bin/python -m pytest -q python/tests/test_independence.py
```

Elvárt eredmény: mindkettő zöld.

## 5. Küszöb és racsni (US4, SC-004)

```sh
cargo xtask parity --all
git show origin/main:xtask/parity/thresholds.json > target/parity/base-thresholds.json
cargo xtask parity --all --ratchet target/parity/base-thresholds.json
```

Elvárt eredmény: mindkét futás 0-s kilépési kóddal ér véget.

Negatív próbák:

1. Csökkents egy küszöböt. Elvárt: a `--ratchet` 1-es kilépési kódot ad.
2. Emeld egy terület küszöbét a mért érték fölé. Elvárt: a `--all` 1-es kilépési kódot ad, és a napló
   megnevezi a területet, a mért értéket és a küszöböt.

A CI-ben a `ci-linux.yml` `parity` jobja `workflow_dispatch`-ra vagy a `cfa4-full-validation` címkére fut.
Ugyanezt kell látni benne, és a `full-validation-gate` csak akkor zöld, ha a `parity` job is zöld.

## 6. Determinisztikus checklist (US5, SC-005)

```sh
cargo xtask parity --all --checklist /tmp/a.md
cargo xtask parity --all --checklist /tmp/b.md
cmp /tmp/a.md /tmp/b.md
cmp /tmp/a.md docs/parity-checklist.md
```

Elvárt eredmény: mindkét `cmp` csendes, azaz a fájlok azonosak.

## 7. Visszafelé kompatibilitás és stabilitás (SC-007, SC-008)

```sh
cargo test -p xtask --release
```

Elvárt eredmény: a meglévő `parity` tesztek és az új tesztek mind zöldek. Az új tesztek:

- ellenséges bemenetek minden új fájlra;
- a régi területek eredménye változatlan;
- 0 tételes terület osztás nélkül.

A minőségi kapukat a `cfa4-ci-runner` futtatja: `cargo xtask ci`, a workspace-tesztek, és a Python-ellenőrzések
(`ruff`, `pytest`).
