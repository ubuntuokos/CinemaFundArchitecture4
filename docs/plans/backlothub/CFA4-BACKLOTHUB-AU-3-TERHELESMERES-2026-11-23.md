# Audit Trail AU-3: terhelésmérés (2026-11-23)

Mérőszkript: `scripts/dev/audit-load-measure.py` (a valódi `cfa4_audit.record` úton ír, eldobható PostgreSQL-klaszteren).
Gép: felhős konténer, 4 vCPU (a CFA4-MIN **helyettesítője**, nem maga a CFA4-MIN), `fsync=off` a teszt-klaszteren.
Terhelés: 100 esemény tranzakciónként, 20 aktor, 2 000 tárgy, egyszerű `scope` és `change`.

| Mérés | 100 000 esemény | 1 000 000 esemény |
|---|---|---|
| Írás | 1 656 esemény/mp | 1 808 esemény/mp |
| Írás, egy esemény mediánja / p99 | 0,53 / 1,17 ms | 0,49 / 1,17 ms |
| `verify` (teljes lánc) | 13 800 esemény/mp (7,2 mp) | 14 300 esemény/mp (69,9 mp) |
| Lekérdezés tárgy szerint (50 sor), medián / p95 | 1,19 / 1,72 ms | 1,38 / 2,31 ms |
| Lekérdezés aktor szerint (50 sor), medián / p95 | 1,40 / 1,83 ms | 1,43 / 2,02 ms |
| Tárhely eseményenként, indexekkel | | 658 bájt |

**Értékelés.** Az írás, az ellenőrzés és a lekérdezések idői 10-szeres növekedésnél sem romlanak érdemben (lineáris
az ellenőrzés, a lekérdezések az indexeken maradnak). Az esemény egyenkénti ideje ~0,5 ms, ez a G2-01 §7 szerinti
sorosítás ára, és két nagyságrenddel kisebb, mint amit egy felhasználói művelet (több tíz ms) elbír.

**Extrapoláció 50 M eseményre (a G2-01 L-profilja), mérés nélkül:** ~33 GB tárhely, `verify` ~58 perc, írás ~7,7 óra.
A felhős konténer 25 GB szabad lemezzel ezt nem futtatja végig, ezért az 50 M-es mérés **nyitva marad** a CFA4-MIN
gépre (István egyeztetett idejében). Addig az extrapoláció becslés, nem eredmény.

**Nem mért:** több párhuzamos író (a korrektség 3 szálas teszttel igazolt, az áteresztés a lánc sorosítása miatt
egyetlen író sebessége: kb. 1 800 esemény/mp), `fsync=on` lemez, a CFA4-MIN korlátai.
