# Spec Kit – elfogadási tesztek és bizonyítékok (SK-04)

CFA4-DEV-SPEC-KIT-001 1.2. Egy helyen mutatja, melyik elfogadási teszt melyik fázisban, melyik jelentésben és melyik PR-ban igazolt. **Állapot: 2026-11-14.** Bizonyítékot csak lefutott ellenőrzés adhat. Az integráció 2026-11-14-én lezárult (D13); az SK-F6-hoz tartozó tesztek elmaradnak.

| Teszt | Mit igazol | Állapot | Fázis | Bizonyíték |
| --- | --- | --- | --- | --- |
| AC-SK-01 | Ubuntu 26.04/amd64 (self-hosted runner, CI-job) | elmarad (D13) | SK-F6 | – |
| AC-SK-02 | Hivatkozó constitution, nincs másolt szabály | PASS | SK-F2 | [SK-F2](../../audit/spec-kit/SK-F2-2026-11-01.md) 3., 9. · #457 |
| AC-SK-03 | Követelményazonosítók megőrzése | PASS (próbaspecifikáción) | SK-F2, SK-F3 | [SK-F2](../../audit/spec-kit/SK-F2-2026-11-01.md) 5. · [SK-F3](../../audit/spec-kit/SK-F3-2026-11-06.md) 5. |
| AC-SK-04 | Meglévő `.claude/` fájl nem változik | PASS | SK-F1–F5 | [SK-F1](../../audit/spec-kit/SK-F1-2026-11-01.md) 4. · [SK-F3](../../audit/spec-kit/SK-F3-2026-11-06.md) 4. · [SK-F4](../../audit/spec-kit/SK-F4-2026-11-08.md) · [SK-F5](../../audit/spec-kit/SK-F5-2026-11-14.md) |
| AC-SK-05 | ThinkRail/Pi | PASS (funkció kikapcsolva) | SK-F3 | [SK-F3](../../audit/spec-kit/SK-F3-2026-11-06.md) 4. |
| AC-SK-06 | Superpowers, EGM, gazda, subagentek együttélése | PASS | SK-F3 | [SK-F3](../../audit/spec-kit/SK-F3-2026-11-06.md) 4. |
| AC-SK-07 | Tiltott parancsok, extensionok, események blokkolva | PASS | SK-F4 | [SK-F4](../../audit/spec-kit/SK-F4-2026-11-08.md) |
| AC-SK-08 | Költség nem romlik | **FAIL** – páros mérés (D12): Spec Kit 5,06 USD, mostani módszer 3,01 USD (168%); n = 1 | SK-F5 | [SK-F5](../../audit/spec-kit/SK-F5-2026-11-14.md) 3. |
| AC-SK-09 | Követelmény–teszt megfelelés | Részben – 25-ből kb. 21 követelmény követhető tesztig, explicit mátrix nélkül | SK-F5 | [SK-F5](../../audit/spec-kit/SK-F5-2026-11-14.md) 4. |
| AC-SK-10 | `cargo-check` + `full-validation-gate` | elmarad (D13) | SK-F6 | – |
| AC-SK-11 | Visszaállíthatóság | elmarad (D13) | SK-F6 | – |
| AC-SK-12 | Host és upstream érintetlen | PASS (felhős rész); runner: elmarad (D13) | SK-F1, SK-F6 | [SK-F1](../../audit/spec-kit/SK-F1-2026-11-01.md) 5. |
| AC-SK-13 | Nincs ág, nincs commit a Spec Kittől | PASS | SK-F1–F4 | [SK-F1](../../audit/spec-kit/SK-F1-2026-11-01.md) 4. · [SK-F4](../../audit/spec-kit/SK-F4-2026-11-08.md) |
| AC-SK-14 | `test_independence.py` a Spec Kit-fájlokkal | PASS | SK-F1–F4 | minden fázisjelentés |
| AC-SK-15 | Kontextus-többlet 0 token | PASS (Claude Code 2.1.316 és 2.1.325) – verziófüggő, frissítés után újramérendő | SK-F1–F5 | [SK-F1](../../audit/spec-kit/SK-F1-2026-11-01.md) 7. · [SK-F4](../../audit/spec-kit/SK-F4-2026-11-08.md) · [SK-F5](../../audit/spec-kit/SK-F5-2026-11-14.md) |
| AC-SK-16 | Skillfejlécek a preset után | PASS (`init --preset`-tel); a `preset add` megsérti, az ellenőrző kimutatja | SK-F2, SK-F4 | [SK-F2](../../audit/spec-kit/SK-F2-2026-11-01.md) 4. · [SK-F4](../../audit/spec-kit/SK-F4-2026-11-08.md) |

Fázisjelentések: [SK-F0](../../audit/spec-kit/SK-F0-2026-10-31.md) · [SK-F1](../../audit/spec-kit/SK-F1-2026-11-01.md) · [SK-F2](../../audit/spec-kit/SK-F2-2026-11-01.md) · [SK-F3](../../audit/spec-kit/SK-F3-2026-11-06.md) · [SK-F4](../../audit/spec-kit/SK-F4-2026-11-08.md) · [SK-F5](../../audit/spec-kit/SK-F5-2026-11-14.md).
