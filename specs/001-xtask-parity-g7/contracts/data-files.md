# Contract: commitolt adatfájlok

A mezők és az ellenőrzési szabályok a [data-model.md](../data-model.md)-ben vannak; itt csak a fájlok helye és
a gazdájuk.

| Fájl | Tartalom | Ki módosítja |
|---|---|---|
| `xtask/parity/README.md` | a referencia eredete és szabályai: csak commitolt `docs/` nevek, a célalkalmazást leíró szakaszokból, semmi a `plan/` alól (Q3, AGENTS.md §1.5, §2); hogyan kell új tételt felvenni | a G7 bevezető PR-ja |
| `xtask/parity/reference/{menus,effects,transitions,commands,panels}.json` | `{area, items: [{path, source}]}` | bárki, csak a saját soraival (`docs/coordination.md` §2.5) |
| `xtask/parity/fidelity.json` | hűségi besorolás területenként | a funkció PR-ja, amely a szintet bizonyítja |
| `xtask/parity/thresholds.json` | `presence` és `exact` küszöb területenként | csak emelés; csökkentéshez tulajdonosi döntés kell (Q2, FR-016a) |
| `docs/parity-checklist.md` | a `--all --checklist` generált kimenete | a mérő; kézzel nem szerkesztendő |

Az adatfájlok szöveges JSON-ok. Nem tartalmaznak assetet (képet, ikont, presetet, sablont: AGENTS.md §1), és
nincs bennük upstream márkanév.
