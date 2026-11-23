# CFA4-VVC-02 – JVET konformanciajelentés (G2 belépő feltétel)

Dátum: 2026-11-23. Terv: [`CFA4-VVC-PLAN-001-2026-11-07.md`](CFA4-VVC-PLAN-001-2026-11-07.md) 6. fejezet.
Verzió: VVdeC v3.2.1 (`cb1a3efd`), a `cfa4-vvc-decode` segédfolyamaton és a `cfa4_codecs::vvc_external` adapteren át.

## Eredmény

JVET `FDIS_r1`, 283 bitfolyam (a zip SHA-256: `a09612a2…efa50`), a kimeneti YUV MD5-je a VTM `.yuv.md5`-jével összevetve:

| Kimenet | Darab |
|---|---|
| PASS, minden kép bitpontos | 245 |
| PASS, 4:0:0, a VTM csak a luma síkot írja (a luma bitpontos) | 5 |
| PASS, a referencia csak az első képre szól (`STILL_B_1`, `STILL444_B_1`) | 2 |
| Nem támogatott, dekódolási hibával elutasítva, kép nem jön ki | 31 |
| Összeomlás, időtúllépés, rossz kép | 0 |

A 31 elutasított folyam: 10 többrétegű vagy VPS/OLS (`ILRPL_A`, `OLS_A/B/C`, `OPI_B`, `SPATSCAL_A`, `SPATSCAL444_A`, `VPS_A/B/C`), és 21 4:2:2 / 4:4:4 képernyőtartalom-eszközös (`8b422_G–L`, `10b422_G–L`, `8b444_A/B`, `ACT_A/B`, `PALETTE_A–E`). Ezek nem Main 10 profilúak, a tervben vállalt profil (Main 10, Main 10 Still Picture) 100%-ban teljesül.

## Korábbi „hibák” magyarázata

* `FIELD_A_4`, `FIELD_B_2`: a mezős (`field_seq_flag`) folyamok mezőit a segéd külön képként adta; a #1631 összefésüli őket, azóta bitpontos.
* `STILL_B_1`, `STILL444_B_1`: a folyam leírása szerint a referencia-MD5 csak az első képre vonatkozik (Main 10 Still Picture dekóder). Az első kimeneti kép MD5-je egyezik a referenciával (ellenőrizve), a többi kép dekódolása hibátlan. Az első harness a teljes kimenetet hasonlította, ezért jelent hibát.

## Ismételhetőség

`scripts/dev/vvc-conformance.py fetch` letölti és kicsomagolja a korpuszt (`target/fixtures/vvc-conformance`, nem kerül a repóba), majd
`cargo test -p cfa4-codecs --release --features vvc-external --test vvc_conformance -- --nocapture`.
A teszt a fenti szabályokat kódolja, az elutasított folyamok listáját rögzíti (ha egy folyam dekódolni kezd, a teszt jelzi, hogy a listából ki kell venni), segéd vagy korpusz nélkül SKIPPED. Mérőgép: felhős konténer, nem a referenciagép. A folyamonkénti nyers eredmény: [`CFA4-VVC-02-CONFORMANCE-data/results.csv`](CFA4-VVC-02-CONFORMANCE-data/results.csv) (a `FAIL` sorok a fenti két szabály előtti, nyers összevetés).

## G2 állapota

* Konformancia (6. fejezet): teljesül a vállalt profilon.
* Hátra: hibatűrés és fuzz (VVC-TEST-017), teljesítményfeltétel és UI-szál (3.4, VVC-TEST-021), hiányzó extras csomag viselkedése (VVC-TEST-022), FFmpeg-tartalékút és router-sorok.
