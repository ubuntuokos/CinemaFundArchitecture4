# ADR 0009 – CFA4 File Services: közös fájlszolgáltatás (CFA4-FILE-SERVICES-001)

**Dátum:** 2026-11-08 · **Állapot:** elfogadva (a tulajdonos, `ubuntuokos`, 2026-11-08; BacklotHub #43, #45, #14) ·
**Terv:** [`docs/plans/CFA4-FILE-SERVICES-001-1.0.md`](../plans/CFA4-FILE-SERVICES-001-1.0.md) · **Érintett rész:**
`apps/cfa4-cli` (`serve`), az ingest-kód (`crates/engine`), új Python-csomag (`cfa4_files`) az FS-1 fázisban; ez az
ADR kódot nem változtat

## Háttér

A BacklotHub (CFA4-BACKLOTHUB-001) terve szerint „a fájlok fizikai tárolása a CFA4 File Services feladata”
(11. pont); az Asset (4.4), a Document (4.20) és az Archive (4.26) modul erre épül, és a feladatok
csatolmányai is. Ilyen szolgáltatásnak a CFA4-ben nem volt terve. A szabály szerint nem létező közös
szolgáltatásra modul nem épülhet, stubbal sem, és ideiglenes adatbázisbeli fájltárolás sem lesz (kettős
tárolás és migrációs teher miatt).

A szükséges képességek egy része már megvan: az ingest ellenőrzött másolása (ujjlenyomat-ellenőrzés,
felülírás soha), a médiaazonosság (`MediaClip::identity`), a Link Media újrakapcsolás és a `cfa4-docpath`
útfeloldása.

## Döntés

1. **Egy közös CFA4 File Services készül**: tárolási gyökerek, fájlkatalógus stabil UUIDv7-azonosítóval és
   **gyökérfüggetlen úttal** (az AYON core, Apache-2.0 mintájára), ellenőrzött írás, lezárás, épségellenőrzés,
   kétlépcsős törlés és a médiatár mentése.
2. **Nem része:** médiafeldolgozás (ADR 0007, Media Services), a verziók jelentése (a BacklotHub adata),
   felhős szinkron (SEC-02), elosztott fájlrendszer.
3. **Adatút Rustban:** másolás, kivonatolás, `fsync`, atomikus átnevezés, lezárás és `verify` a meglévő
   ingest-kódra építve, `files.*` parancsként a `cfa4-cli serve`-en (ADR 0007 mintájára), így a CLI, az MCP
   és a felület is eléri.
4. **Katalógus Pythonban** (`cfa4_files`), saját `cfa4_files` sémában a BacklotHub PostgreSQL-adatbázisában
   (ADR 0006). Így a katalógusírás, a BacklotHub hivatkozása és az auditesemény (ADR 0008) **egy
   tranzakcióban** történik. A Rust-motornak M1-ben nem kell PostgreSQL-kliens.
5. **Kivonat: BLAKE3** (CC0-1.0 / Apache-2.0, új Rust-függőség, a G3 kapun rögzített verzióval). Ok: a
   referenciagép Xeon E5 v4 processzorán nincs SHA-hardvergyorsítás. A katalógus az algoritmust is tárolja.
6. **Írás:** `staging` → kivonat → `fsync` → atomikus átnevezés → katalógus `committed`. A katalógus soha nem
   mutat félkész fájlra; felülírás soha.
7. **Lezárás és törlés:** a publikált fájl lezárva csak olvasható; törlés először rejtett (`deleted`), a
   fizikai törlés (`purged`) csak jóváhagyással és auditálva; hivatkozott lezárt fájl nem törölhető.
8. **Mentés (#14):** a publikált, lezárt fájlok növekményesen, kivonat alapján; a munkafájlok és
   csatolmányok naponta; a proxyk és bélyegképek nem (a Media Services újra előállítja). A katalógus a
   PostgreSQL-mentés része. Felhő csak kifejezett beállítással.
9. **Titkosítás (SEC-05, kapcsolható):** fájlrendszer-szinten (LUKS vagy fscrypt), nem alkalmazásszinten.
10. **Never crash** (`AGENTS.md` §0): nem elérhető gyökér, teli lemez, jogosultsági hiba vagy sérült fájl
    kezelhető hibát ad angol hibakóddal (`FILES_ROOT_UNAVAILABLE`, `FILES_DISK_FULL`,
    `FILES_PERMISSION_DENIED`, `FILES_HASH_MISMATCH`, `FILES_SEALED_MODIFIED`); a felület fordít.

## Fázisok

| Fázis | Tartalom | Kell, mielőtt … |
|---|---|---|
| FS-0 | Ez az ADR és a terv a `main`-ben | – |
| FS-1 | Gyökerek, katalógus, ellenőrzött írás, lezárás, `resolve`; összeomlás-, teli lemez- és útkiszökés-tesztek | BacklotHub 5. lépcső csatolmánya, 8. lépcső |
| FS-2 | `verify`, újrakapcsolás, kétlépcsős törlés, hivatkozás nélküli fájlok | 8. lépcső G5 |
| FS-3 | Mentés és visszaállítás, 1 TB-os mérés a `CFA4-MIN` profilon | 14. és 26. lépcső |
| FS-4 | M2: gépenkénti gyökérfeloldás, hálózati megosztás | M2 |

## Következmények

- Egy fájlkatalógus és egy írási út minden CFA4-fogyasztónak; a BacklotHub nem másol és nem töröl a lemezen.
- A `cfa4-cli serve` szerződése a `files.*` parancsokkal bővül; a változásra az ADR 0007 10. pontjának
  szabálya (szerződéstesztek, protokollverzió) érvényes.
- A BLAKE3 új függőség, a G3 licencellenőrzésen megy át.

## Kapcsolódó dokumentumok

- [`docs/plans/CFA4-FILE-SERVICES-001-1.0.md`](../plans/CFA4-FILE-SERVICES-001-1.0.md) – a részletes terv
- ADR 0006 (BacklotHub: PostgreSQL), ADR 0007 (Media Services), ADR 0008 (közös auditnapló, PR #767)
- `docs/project-files.md` (ingest, médiaazonosság, Link Media)
