# ADR 0006 – PostgreSQL a BacklotHub adattárolója, egygépes módban is

**Dátum:** 2026-11-02 · **Állapot:** elfogadva (a tulajdonos, `ubuntuokos`, 2026-11-02-án döntött) ·
**Érintett rész:** a BacklotHub produkciókezelő (CFA4-BACKLOTHUB-001) adatrétege; a CFA4-motor és a
`crates/*` nem változik

## Háttér

A BacklotHub (CFA4-BACKLOTHUB-001) 26 modulból álló produkciókezelő, amely lépcsőzetesen, modulonként
kerül a CFA4-be. Három működési módot ismer:

- **M1 – egygépes:** egy gép, egy felhasználó vagy egy kis csapat ugyanazon a gépen. Ezzel indulunk.
- **M2 – helyi hálózat:** több gép közös adattal.
- **M3 – nagy telepítés:** több helyszín, sok felhasználó.

Az M2 és az M3 igény szerint, később jön. A tervezett méret egygépes módban évi 52 produkció,
beépített méretkorlát nélkül.

A `VISION.md` ma nem nevez meg adatbázist. A kódban sincs adatbázis-crate (`rusqlite`, `sqlx`,
`postgres`). A tervezett K17 nyilvántartás (`docs/kdenlive-parity.md`) SQLite-sémamintát említ.

Az elemzés két irányt vetett össze:

- **A:** PostgreSQL minden módban.
- **B:** egygépes módban SQLite, hálózati módban PostgreSQL.

A B irány egyszerűbb indulást ad, de az M2-re váltáskor második adatréteget, adatmigrációt és kettős
tesztelést hoz.

## Döntés

1. **A BacklotHub adattárolója minden működési módban a PostgreSQL**, az egygépes M1-ben is. Az M1
   helyi PostgreSQL-példányt használ. Az M2/M3-ra váltás így ugyanaz a séma és ugyanaz a kód, csak más
   kiszolgálóval. A tulajdonos indoklása: „ha elindulunk többgépesre, ne kelljen újból feltalálni”.
2. **Az Ubuntu tárolójának `postgresql` alapcsomagja**, rendszercsomagként, nem beágyazva. Ez a
   GStreamer-szabály mintája (`ADR-0005`: alapból az Ubuntué).
   - A `.deb` függőségként hozza.
   - Az AppImage, a Flatpak és a `tar.gz` a rendszerre telepített PostgreSQL-t használja. Ha hiányzik,
     kezelhető hibaüzenetet ad a telepítés módjával, és nem omlik össze (`AGENTS.md` §0).
   - A BacklotHub saját adatbázis-klasztert használ, nem a rendszer alapértelmezett példányát. A
     telepítés és az indítás pontos módja (rendszer- vagy felhasználói szolgáltatás) a BacklotHub első
     lépcsőjének G2 kapuján dől el.
3. **Offline-first.** M1-ben semmilyen funkció nem igényel hálózatot. Stabil azonosítók: UUIDv7,
   kliensoldalon generálva.
4. **Adatbiztonság a PostgreSQL saját eszközeivel.** Összeomlás után a WAL és a BacklotHub
   műveletnaplója állítja vissza az állapotot. Cél: RPO ≤ 5 s, RTO ≤ 1 perc. Részleges írás és csendes
   felülírás nincs. A mentés módját (`pg_dump` vagy alapmentés + WAL-archívum) és a nyugalmi titkosítást
   a BacklotHub NFR-jei rögzítik. A titkosítás kapcsolható.
5. **A K17 nyilvántartást ez az ADR nem érinti.** Hogy a BacklotHub és a K17 közös tárolót használjon-e,
   külön, nyitott kérdés. Erről nem ez az ADR dönt.
6. **Kliensmeghajtó.** Komponensenként az a nyelv, amelyben a megbízható működés ésszerűbben
   garantálható (Rust vagy Python, tulajdonosi döntés 2026-11-02). A meghajtó (pl. `tokio-postgres` /
   `sqlx`, illetve `psycopg`) a lépcső G2 kapuján dől el. Feltétel: GPL-3.0-or-later-kompatibilis
   licenc.

## Kiegészítés, 2026-11-07: a klasztert más modulok is használják

A tulajdonos 2026-11-07-én jóváhagyta a [CFA4-TASKSTORE-PG-001](../plans/CFA4-TASKSTORE-PG-001.md)
tervet. Eszerint ugyanezt a CFA4 PostgreSQL-klasztert használja a közös feladatsor és ütemezés
(`tasks` séma), az AI-réteg (`ai` és `cache` séma) és a Media Intelligence (`mi` séma, pgvector
bővítménnyel), modulonként saját sémával és adatbázis-szereppel. Második adatbázismotor nincs. A
BacklotHub saját sémája (`backlot`) és a fenti döntések változatlanok; a kliensmeghajtó és a
telepítési mód a BacklotHub G2 kapuján közösen dől el, egy meghajtóval.

## Kiegészítés, 2026-11-11: telepítési mód és kliensmeghajtó (BacklotHub G2)

A tulajdonos 2026-11-11-én jóváhagyta a BacklotHub 1. és 2. lépcsőjének G2-jét (BacklotHub #48, #49, #52).
Ez lezárja a fenti 2. és 6. pontot:

1. **Telepítési mód (#48).** Az Ubuntu `postgresql` rendszercsomagja, **külön CFA4-klaszterrel**
   (`pg_createcluster <ubuntu-főverzió> cfa4`); a rendszer alapklaszterét a CFA4 nem használja. M1-ben
   csak Unix-socket, TCP-port nincs (`listen_addresses = ''`); hitelesítés `peer`, jelszó nincs tárolva.
   Egy `cfa4` adatbázis, benne modulonként saját séma és szerep (`backlot`, `cfa4_audit`, a TASKSTORE
   sémái). A közös auditnapló ([ADR 0008](ADR-0008-cfa4-audit.md)) a `backlot` sémával egy adatbázisban
   van, hogy az üzleti változás és az auditesemény egy tranzakcióban íródjon. Ha a `postgresql` hiányzik,
   angol hibakód (`DB_SERVER_MISSING`) és olvasható üzenet, összeomlás nincs.
2. **Kliensmeghajtó (#49).**
   - **Python:** psycopg 3 (`psycopg`, LGPL-3.0-or-later). A BacklotHub-szolgáltatás és a Python
     közös könyvtárak (`cfa4_audit`, `cfa4_files`) ezt használják.
   - **Rust:** egy meghajtó minden Rust-komponensnek, a `tokio-postgres` és az `sqlx` közül. A választás
     az első Rust-író megvalósítási kapuján dől el (TASKSTORE PG-fázis vagy Event Fabric EF-R1, amelyik
     előbb jön), és utána mindenkire érvényes. A BacklotHub Rust-oldala (`cfa4-audit` crate) addig
     adatbázis nélkül készül.
3. **Adatbázis-réteg és migráció a BacklotHubban (#52):** SQLAlchemy 2 (Core és ORM) és Alembic (mindkettő
   MIT), egy migrációs lánccal a `backlot` sémára. A többi modul sémáját a saját migrációja kezeli.

## Licenc

- A PostgreSQL licence (PostgreSQL License, BSD/MIT-típusú) GPL-3.0-kompatibilis.
- Rendszercsomagként fut, külön folyamatban. A CFA4 csak kliensként kapcsolódik hozzá.

## Következmények

- Az M1-ben is fut egy adatbázis-kiszolgáló a gépen. Az erőforrásigényét a minimális kliensgépen kell
  mérni (4 mag/8 szál vagy 8 mag, 16 GB RAM; a `CFA4-MIN` szimuláció).
- A sémamigráció egy úton megy minden módban. Visszafelé egy verziót kell tudni, és minden migrációt
  valós méretű adaton kell tesztelni.
- A `VISION.md` adatrétegre vonatkozó részét ez az ADR egészíti ki. Ha a kettő ellentmond, a tulajdonos
  dönt, és a `VISION.md` frissül.

## Kapcsolódó dokumentumok

- `VISION.md` – irányadó; `AGENTS.md` §0 – never crash
- `docs/adr/ADR-0005-media-backends.md` – a rendszercsomag-minta (GStreamer)
- `docs/kdenlive-parity.md` – K17 nyilvántartás (nem érintett)
- A BacklotHub-elemzések (SQL/offline elemzés, NFR-javaslat, döntési lista) a projekt munkaanyagai. A
  BacklotHub-terv `main`-re kerülésekor ide kerül a hivatkozásuk.
