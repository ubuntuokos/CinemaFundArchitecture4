# Contract: `cargo xtask parity` parancssor (G7 után)

A meglévő kapcsolók jelentése nem változik (FR-021). Új elemek: **új**.

```text
cargo xtask parity [--area menus|effects|transitions|commands|panels] [--all]
                   [--reference FILE] [--ours FILE] [--out FILE]
                   [--min PERCENT] [--min-exact PERCENT]
                   [--thresholds FILE] [--ratchet BASE_FILE]
                   [--checklist FILE] [--check-checklist FILE]
```

| Kapcsoló | Jelentés |
|---|---|
| `--area` | egy terület mérése; **új** értékek: `commands`, `panels` |
| `--all` **új** | mind az öt terület, rögzített sorrendben; nem adható meg az `--area`, a `--reference` és az `--ours` mellett |
| `--reference FILE` | helyi referencia, például a `plan/premiere/<area>.json`. Megadása nélkül a commitolt `xtask/parity/reference/<area>.json` a referencia (**változás**, FR-006) |
| `--ours FILE` | a saját oldal mentett kimenete (változatlan; a `panels` területnél a Qt-szkript JSON-kimenete) |
| `--min PERCENT` | a jelenlét küszöbe, 0–100 (változatlan) |
| `--min-exact PERCENT` **új** | a pontos jelenlét küszöbe, 0–100 |
| `--thresholds FILE` **új** | területenkénti küszöbök; `--all` mellett ez az alapértelmezés: `xtask/parity/thresholds.json` |
| `--ratchet BASE_FILE` **új** | a küszöbfájl összevetése az alap (a `main`) változatával; csökkenésnél hibát ad |
| `--checklist FILE` **új** | a determinisztikus markdown checklist megírása (`--all` mellett) |
| `--check-checklist FILE` **új** | összevetés a commitolt checklisttel; eltérésnél figyelmeztet, a kilépési kód 0 marad |

## Kilépési kódok

| Kód | Mikor |
|---|---|
| 0 | siker; figyelmeztetés lehet: elavult checklist, árva besorolás, emelhető küszöb |
| 1 | egy terület a küszöbe alatt van; racsni-sértés; érvénytelen referencia, besorolás vagy küszöbfájl; FR-007a forrásellenőrzési hiba; a saját oldal lekérése sikertelen; hibás kapcsoló |

A hibaüzenet mindig megnevezi a területet és az okot. Részeredmény hibánál nem keletkezik (spec, Edge Cases).

## Napló (CI-hez)

Minden terület egy sort ír:

```text
parity <area>: presence 61.4% (min 61.4%), exact 12.0% (min 12.0%), reference 312 items (3 sources)
```

GitHub-annotációk: a küszöb alatti terület `::error::`, az emelhető küszöb `::notice::`, az elavult checklist
és az árva besorolás `::warning::`. Ezeket csak akkor írja ki, ha a `GITHUB_ACTIONS` környezeti változó be
van állítva.
