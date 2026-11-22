# F0V: ismert eltérések (hogy a golden ne rögzítse a hibát elvárt eredményként)

Forrás: PR #271 és #331 teszt-megjegyzései, a terv 2. pontja és az F0V-1 mérés. Minden sor vagy mérhető, vagy PENDING.

| Azonosító | Terület | Eltérés | Eredet | Hatás a validációra | Állapot |
|---|---|---|---|---|---|
| KD-01 | J2K / DCP | A J2K orákulum a `main` futásán piros (#331 szöveg: „main's full run is currently red on the unrelated j2k oracle”) | #331 | Mozi célú kiadás blokkolva (AC-12, AC-18) | BLOCKED; CINECERT területe, nem érintem |
| KD-02 | HEIC | `heif-enc` / libheif hiányában a HEIC round-trip és a HEIF orákulum kimarad | #271, #331 | Géppel generált kihagyási lista kell a riportba (AC-12) | PENDING |
| KD-03 | `phone_vfr` | A `cli phone_vfr` workflow-teszt orákulumot igényel (FFmpeg ≥ 9.0.2) | #331 | A valódi mobilfelvétel nincs; a `vfr_synthetic` csak pótlék | PENDING |
| KD-04 | EXR / TIFF YCbCr | Az EXR és YCbCr TIFF orákulumok f16 pixelformátumot kérnek az FFmpeg-től | #271, #331 | Ahol az FFmpeg-build nem tudja, kimarad | PENDING (build-függő) |
| KD-05 | A/V drift | **Lezárva ebben a körben.** A mérő (`timing::rising_edges`, `timing::beep_onsets`, `validation::compare_with` + `SyncSpec`) a motoron átvitt `sync_flash_beep` renderen fut (`crates/golden/tests/sync_render.rs`, a saját dekóderekkel): eltolás −0,0208 ms (egy minta 48 kHz-en), drift 0, 10 futásban azonos; a 80 ms-os hangkésést elkapja. Határok: eltolás ≤ 0,1 ms, drift ≤ 0,1 ms (négy minta). Az ítélet a teljes harnessben akkor lesz `Pass`, ha ezt a mérést a riport is tartalmazza (riportba kötés: következő lépés) | #1465, F0V-3 | A `compare` önmagában továbbra is `Incomplete` | KÉSZ (mérve, CPU-út, egy gép; referenciagépen ismétlendő) |
| KD-06 | 16 bites / EXR golden | Még nincs 16 bites PNG és EXR golden fájl; a meglévő golden 8 bites RGBA (`testkit`, 50 KiB korlát) | testkit | A bit-azonos CPU-kapu csak a procedurális jelenetre áll | PENDING |
| KD-07 | Gép-/GPU-zaj | A küszöbök egy gép CPU-útját fedik | F0V-1 | AC-04 GPU-ága nem mérhető | PENDING (referenciagép) |
| KD-08 | Futtató gép | A mérések Ubuntu 24.04 konténeren futottak (nem 26.04) | környezet | A referenciagépes ismétlés kötelező a hivatalos küszöbhöz | PENDING |

A lista a golden frissítési szabálya szerint bővül: egy új eltérés felvétele nem a golden módosítása, hanem a sor hozzáadása és a vizsgálat.
