# BacklotHub 1. lépcső – Audit Trail: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-10 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.1 · **Közös szolgáltatás:** CFA4-AUDIT-001 1.0
(ADR 0008, PR #767, a gazda olvasztja be) · **Előzmény:** [G1 Audit Trail](CFA4-BACKLOTHUB-G1-01-AUDIT-TRAIL-2026-11-06.md),
a funkcionális követelmények jóváhagyva (István, 2026-11-10) · **Állapot:** G2 **jóváhagyva** (István, 2026-11-11)
(#48–#51 eldöntve 2026-11-11) · implementáció nincs

A G2 kapu feltétele: „Az integrációs határok és az adatmodell jóváhagyottak.” Ez a dokumentum a
CFA4-AUDIT-001 AU-1 és AU-2 fázisához rögzíti mindkettőt, és lezárja a terv 13. pontjának nyitott kérdéseit,
valamint az ADR 0006 két, „a BacklotHub G2-jén” eldöntendő pontját (telepítési mód, kliensmeghajtó).

## 1. Előfeltételek és sorrend

| # | Előfeltétel | Állapot | Teendő |
|---|---|---|---|
| E1 | CFA4-AUDIT-001 terv és ADR 0008 a `main`-ben | PR #767, kész, a gazdánál (#866) | A gazda olvasztja be |
| E2 | `AGENTS.md`: a #21 döntés (üzleti logika Pythonban is lehet) | **Hiányzik.** Az `AGENTS.md` szerint a Python csak fejlesztői eszköz és Qt6-felület lehet | Külön kis PR (a #21 ezt előírja), saját sorral |
| E3 | A G2 jóváhagyása (#48–#51) | Ez a dokumentum | István |
| E4 | ADR 0006 kiegészítés: a meghajtó és a telepítési mód rögzítése | A #48/#49 döntése után | ADR 0006-kiegészítés a G2 jóváhagyása után, külön PR |

Az AU-1 megvalósítása (G4) csak E1–E4 után indulhat.

## 2. Integrációs határok

```
 BacklotHub üzleti modul (Python)          Event Fabric Rust Core (később, EF-R1)
        │ ugyanabban a tranzakcióban               │ EF Audit Adapter (EF-CORE-013)
        ▼                                          ▼
   cfa4_audit (Python)  ◄── közös tesztvektorok ──►  cfa4-audit (Rust crate)
        │ psycopg 3                                │ AU-1: csak kanonikus forma + hash
        ▼                                          │ (adatbázis-kapcsolat az EF-R1-gyel)
   PostgreSQL „cfa4” klaszter, közös adatbázis ◄───┘
     séma: backlot (üzleti)  ·  séma: cfa4_audit (napló)
```

| Határ | Ki hív kit | Szerződés | Ami **nem** megy át a határon |
|---|---|---|---|
| H1 Üzleti modul → `cfa4_audit` | Python, folyamaton belül | `record(conn, AuditEvent)` a hívó **nyitott tranzakciójában**; visszatér a kiosztott `seq`-kel és `hash`-sel | Saját tranzakció, saját kapcsolat (kivétel: `record_system`) |
| H2 `cfa4_audit` → PostgreSQL | psycopg 3, Unix-socket | Csak a `cfa4_audit.append()` SQL-függvényen és `SELECT`-en át | Közvetlen `INSERT`/`UPDATE`/`DELETE` a naplótáblára |
| H3 Felület / CLI / MCP → napló | parancsok | `audit.list`, `audit.verify`, `audit.export`, `audit.checkpoint` (lásd 7.) | Írás: a felületről nem lehet auditeseményt írni |
| H4 Event Fabric → napló | Rust, később | `cfa4-audit` crate, ugyanaz a séma és függvény | A Fabric nem olvas a naplóból eseményt kézbesítésre |
| H5 Mentés, archívum | File Services FS-3, Archive (26.) | `pg_dump` + ellenőrzőpont-fájl; partícióexport (AU-4) | Visszaállítás `audit.verify` nélkül |

**Hatáskör (megerősítve a tervből):** a napló nem jogosultsági hatóság, nem munkafolyamat-motor, nem
eseménybusz, nem diagnosztikai napló.

## 3. Nyelv és meghajtó (#21, ADR 0006 6. pont)

- **Python `cfa4_audit`** (`python/cfa4_audit/`): a BacklotHub üzleti logikája Python, és az auditírásnak az
  üzleti változással egy tranzakcióban kell történnie. Ezért a hívó nyelvén kell lennie. Pythonban a hibakezelés
  és a tranzakciókezelés ugyanabban a kódban marad, ezért a megbízható működés így garantálható ésszerűbben (#21).
- **Meghajtó: psycopg 3** (`psycopg`, LGPL-3.0-or-later, GPL-3.0-or-later-rel összeegyeztethető; Ubuntu-csomag:
  `python3-psycopg`). Indok:
  - a hivatalos PostgreSQL-protokollt a `libpq`-n át használja, amely a `postgresql` rendszercsomaggal együtt
    jön;
  - a szerveroldali paraméterkötés és a `COPY` a 26. lépcső exportjához is kell;
  - a TASKSTORE (ADR 0006 kiegészítés) Python-oldala ugyanezt használhatja, így egy meghajtó marad.
- **Rust:** az AU-1-ben a `cfa4-audit` crate **adatbázis nélkül** készül, csak kanonikus szerializálással,
  hash-sel és tesztvektorokkal. A Rust-meghajtó (`tokio-postgres` vagy `sqlx`) kiválasztása az EF-R1 G2-jére
  marad, amikor az első Rust-író megjelenik. Így most nem épül olyan rész, amelynek nincs hívója.

## 4. Telepítési mód és kapcsolat (ADR 0006 2. pont, #30)

- Az Ubuntu `postgresql` rendszercsomagja, **külön CFA4-klaszter** (`pg_createcluster <ubuntu-főverzió> cfa4`).
  A rendszer alapklaszterét a CFA4 nem használja, így nem ütközik a felhasználó egyéb adatbázisaival.
- **M1-ben csak Unix-socket**, TCP-port nincs (`listen_addresses = ''`, SEC-02). Hitelesítés `peer`: az
  adatbázis-szerep neve az OS-felhasználó, jelszó nem kell és nincs tárolva.
- Egy adatbázis (`cfa4`), benne modulonként saját séma és szerep (ADR 0006 kiegészítés): `backlot`,
  `cfa4_audit`, a TASKSTORE sémái (`tasks`, `ai`, `cache`, `mi`).
- **Az audit ugyanabban az adatbázisban van, mint a `backlot` séma** (a terv 13. pontjának nyitott kérdése).
  Indok: egy tranzakció, nincs kétfázisú véglegesítés és nincs outbox. Külön adatbázisban a fail-closed
  (terv 10.) nem teljesülne.
- A klaszter létrehozását a CFA4 telepítő/első indítás végzi, `pkexec`-kel, egyszer. Ha a `postgresql` hiányzik
  (nem `.deb` csomagformátum), angol hibakód (`DB_SERVER_MISSING`) és magyar/angol üzenet, összeomlás nélkül.

## 5. Szerepek és jogok

| Szerep | Bejelentkezhet | Jogok |
|---|---|---|
| `cfa4_audit_owner` | nem | A `cfa4_audit` séma, táblák, függvények tulajdonosa. Csak a migráció és a partíciókezelés használja (`SET ROLE`) |
| `cfa4_audit_writer` | nem | `EXECUTE` a `cfa4_audit.append()`-en. A tábla közvetlen írására **nincs** joga |
| `cfa4_audit_reader` | nem | `SELECT` a naplón és a nézeteken |
| `<os-felhasználó>` (M1) | igen, `peer` | tagja: `backlot_app`, `cfa4_audit_writer`, `cfa4_audit_reader` |

- Az `append()` `SECURITY DEFINER` függvény (tulajdonos: `cfa4_audit_owner`, rögzített `search_path`). Csak ezen
  át kerülhet sor a táblába.
- Trigger minden `UPDATE`, `DELETE` és `TRUNCATE` műveletet elutasít (`AUDIT_APPEND_ONLY`), a tulajdonosét is.

**M1-korlát (kimondva):** egygépes módban a felhasználónak `sudo`-val elérhető a `postgres` szuperfelhasználó,
aki a triggert is kikapcsolhatja. A jogosultság és a trigger tehát a **programhibák és a véletlen módosítás**
ellen véd, a gép tulajdonosa ellen nem. A szándékos módosítást a hash-lánc és a **külső ellenőrzőpont** mutatja
ki (6. és 8. pont). Ezt a G1 is így kezelte; M2-ben a szerver külön gépen van, ott a védelem valódi. (#50)

## 6. Adatmodell (DDL-vázlat)

```sql
CREATE SCHEMA cfa4_audit AUTHORIZATION cfa4_audit_owner;

CREATE TYPE cfa4_audit.actor_kind AS ENUM ('os_user', 'identity', 'system', 'agent');
CREATE TYPE cfa4_audit.outcome    AS ENUM ('success', 'rejected', 'failed');

CREATE TABLE cfa4_audit.chain_head (            -- egyetlen sor: a lánc vége
    id          smallint PRIMARY KEY CHECK (id = 1),
    seq         bigint   NOT NULL,
    hash        bytea    NOT NULL CHECK (octet_length(hash) = 32)
);

CREATE TABLE cfa4_audit.event (
    seq            bigint      NOT NULL,
    event_id       uuid        NOT NULL,          -- UUIDv7
    schema_ver     smallint    NOT NULL DEFAULT 1,
    occurred_at    timestamptz NOT NULL,
    recorded_at    timestamptz NOT NULL,
    app            text        NOT NULL,
    action         text        NOT NULL CHECK (action ~ '^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$'),
    command_id     text,
    actor_kind     cfa4_audit.actor_kind NOT NULL,
    actor_ref      text        NOT NULL,
    on_behalf_of   text,
    subject_type   text        NOT NULL,
    subject_id     uuid        NOT NULL,
    scope          jsonb       NOT NULL DEFAULT '{}',
    change         jsonb,
    outcome        cfa4_audit.outcome NOT NULL,
    reason_code    text        CHECK (reason_code ~ '^[A-Z][A-Z0-9_]*$'),
    correlation_id uuid,
    causation_id   uuid,
    trace_id       text,
    prev_hash      bytea       NOT NULL CHECK (octet_length(prev_hash) = 32),
    hash           bytea       NOT NULL CHECK (octet_length(hash) = 32),
    PRIMARY KEY (seq, recorded_at),
    UNIQUE (event_id, recorded_at)
) PARTITION BY RANGE (recorded_at);

CREATE TABLE cfa4_audit.personal_detail (       -- nem láncolt (terv 7.)
    event_id    uuid PRIMARY KEY,
    detail      jsonb NOT NULL,
    erased_at   timestamptz
);

CREATE TABLE cfa4_audit.checkpoint (
    seq         bigint PRIMARY KEY,
    hash        bytea  NOT NULL,
    taken_at    timestamptz NOT NULL,
    file_path   text   NOT NULL
);
```

**Indexek:** `(subject_type, subject_id, seq)`, `(actor_kind, actor_ref, seq)`, `(action, seq)`,
`(occurred_at)`, `scope` GIN (`jsonb_path_ops`). Partíciónként jönnek létre.

**Partícionálás:** havi, a **`recorded_at`** szerint, nem az `occurred_at` szerint. A `recorded_at` a `seq`-kel
együtt nő, ezért egy havi partíció a lánc egy összefüggő szakasza; ez az archiváláshoz (AU-4, határoló
hash-ek) kell. Az `occurred_at` lehet korábbi (pl. utólag rögzített tény), ezért arra csak index van.
A partíciókat az `audit.partition.ensure` két hónappal előre létrehozza (indításkor és naponta). Egy
`DEFAULT` partíció biztonsági háló: ha mégis hiányzik a havi partíció, az írás nem bukik el, de figyelmeztetés
megy (`AUDIT_PARTITION_DEFAULT_USED`), és a következő `ensure` áthelyezi a sorokat.

**A `change` mező:** `{"<mező>": {"old": …, "new": …}}`. Személyes mezőnél az érték helyén
`{"redacted": true, "digest": "<sha256-hex>"}`, a tényleges érték a `personal_detail`-ben.

**Pontosítás (2026-11-21, a G2-18 kérésére; megvalósítás még nincs):** a redaktált érték `digest`-je **sózott**:
`SHA-256(salt ‖ kanonikus érték)`, ahol a `salt` 16 véletlen bájt, **a `personal_detail` sorában tárolódik, nem a
láncban**. Sózatlan kivonat alacsony entrópiájú értéknél (telefonszám, születési dátum, IBAN, név) szótártámadással
visszafejthető, és a láncban örökre ott maradna, vagyis a törlési kérés (SEC-06) után is. A sózott kivonat a törlésig
ellenőrizhető (a `personal_detail` megvan), a `salt` törlése után **nem fejthető vissza**, a lánc `prev_hash`-ei
pedig nem változnak (a lánc a kivonat szövegét tartalmazza, nem a sót). Az érték `erased_at` jelöléssel törlődik
a `personal_detail`-ből; az `audit.verify` a láncot továbbra is ellenőrzi, a személyes értékek ellenőrzését a
törölt soroknál átugorja és jelzi.

## 7. Írás és a lánc sorosítása

`cfa4_audit.append(event jsonb, hash bytea, prev_hash bytea, seq bigint) RETURNS void`, és a könyvtár így hívja:

1. A hívó tranzakciójában: `SELECT seq, hash FROM cfa4_audit.chain_head WHERE id = 1 FOR UPDATE`.
2. A könyvtár kiszámolja: `seq = head.seq + 1`, `recorded_at = now()` (a tranzakció ideje), a kanonikus formát
   és a hash-t (8. pont).
3. `append(...)`, amely ellenőrzi, hogy `prev_hash = head.hash` és `seq = head.seq + 1` (különben
   `AUDIT_CHAIN_CONFLICT`), beszúr, és frissíti a `chain_head`-et.

A `FOR UPDATE` sorzár a tranzakció végéig tart, ezért a szabály: **a `record()` a tranzakció utolsó lépése a
`COMMIT` előtt.** A könyvtár ezt egy kontextuskezelővel segíti (`with audit.unit(conn) as u: … u.record(...)`
a végén). A `pg_advisory_xact_lock`-hoz képest a sorzár előnye, hogy a lánc vége ugyanabban a sorban van, nem
kell külön `ORDER BY seq DESC LIMIT 1` keresés a partícionált táblán.

A szerver a hash-t **nem** számolja újra, mert a kanonikus formát PL/pgSQL-ben nem lehet megbízhatóan
reprodukálni. A rossz hash-t az `audit.verify` találja meg (fail-detect, nem fail-closed). A `seq` és a
`prev_hash` folytonosságát viszont a szerver kikényszeríti.

**Terhelés:** M1-ben néhány esemény másodpercenként. A sorosítás csak az auditköteles tranzakciók utolsó
milliszekundumait érinti. Mérés: AU-3, L-profil, CFA4-MIN.

## 8. Kanonikus forma és hash

- **Kanonikus forma: RFC 8785 (JSON Canonicalization Scheme, JCS)**, saját szűkítéssel:
  - csak egész számok (`int64` tartomány); **lebegőpontos szám tilos** (`AUDIT_FLOAT_FORBIDDEN`), pénzösszeg
    és arány decimális **szövegként** (#15). Így a JCS szám-szerializálásának Python/Rust eltérései nem
    jelentkezhetnek;
  - időpont RFC 3339, UTC, `Z`, mikroszekundum pontosság (a PostgreSQL `timestamptz` felbontása);
  - UUID kisbetűs, kötőjeles; `bytea` mező a hash-inputban kisbetűs hex.
- **A hash-elt tartalom:** az esemény minden mezője a `hash` kivételével, a `seq`-kel és a `prev_hash`-sel
  együtt (így az átsorolás is kimutatható).
- `hash = SHA-256( "cfa4-audit/v1" || 0x00 || JCS(event) )`. A tartományjelölő és a `schema_ver` miatt a
  formátum később változtatható úgy, hogy a régi szakasz ellenőrizhető marad.
- Kezdőérték: `prev_hash` az első eseménynél 32 nullbájt; az első esemény `action = audit.chain.genesis`,
  `app = cfa4-audit`, `scope`-ban a klaszter- és adatbázis-azonosítóval.
- **Közös tesztvektorok:** `tests/vectors/audit/v1/*.json` (bemenet, várt kanonikus bájtsor, várt hash),
  köztük az RFC 8785 saját példái a szűkítésen belül. A Python- és a Rust-teszt is ezt olvassa.

## 9. Ellenőrzőpont (terv 5.)

- Fájl: `$XDG_STATE_HOME/cfa4/audit/checkpoints/<cluster-id>/<seq>.json` (`seq`, `hash`, idő, klaszter-
  azonosító), **a PostgreSQL adatkönyvtárán kívül**, a felhasználó tulajdonában.
- Mikor: naponta, kilépéskor és minden mentés előtt; a mentés mellé is másolat kerül (FS-3).
- `audit.verify --from-checkpoint` a fájlból indul, és jelzi, ha a lánc vége a fájlnál korábbi vagy eltér
  (`AUDIT_CHECKPOINT_MISMATCH`). Ez mutatja ki a lánc végének levágását.
- Aláírás M1-ben nincs; M2-ben a szerver kulcsával aláírt ellenőrzőpont külön döntés.

## 10. Parancsok és hibakódok

| Parancs | Paraméterek | Eredmény |
|---|---|---|
| `audit.list` | szűrők (szereplő, objektum, művelet, idő, `scope`), `after_seq`, `limit` | eseménylista, lapozás `seq` szerint |
| `audit.verify` | `from_seq`, `to_seq`, `from_checkpoint` | `ok` vagy az első eltérés helye és kódja |
| `audit.export` | szűrők, `format` (`json`, `csv`) | fájl; a JSON tartalmazza a hash-eket, így kívül is ellenőrizhető |
| `audit.checkpoint` | – | új ellenőrzőpont-fájl |
| `audit.partition.ensure` | – | karbantartás, `cfa4_audit_owner` szereppel |
| `audit.personal.erase` | `subject_type`, `subject_id` | A tárgyhoz tartozó `personal_detail` sorok tartalmának és sójának törlése (`erased_at`); tartalom nélküli `audit.personal.erased` esemény a láncban. Csak a `cfa4_audit.erase_personal()` `SECURITY DEFINER` függvényen át (a lánctáblát nem érinti), a hívó a SEC-06 szerinti törlést végző üzleti parancs |

Hibakódok (angol, a felület fordít, #16): `AUDIT_WRITE_DENIED`, `AUDIT_CHAIN_CONFLICT`, `AUDIT_CHAIN_BROKEN`,
`AUDIT_CHECKPOINT_MISMATCH`, `AUDIT_APPEND_ONLY`, `AUDIT_FLOAT_FORBIDDEN`, `AUDIT_SCHEMA_VERSION_UNKNOWN`,
`AUDIT_PARTITION_DEFAULT_USED`, `DB_SERVER_MISSING`, `DB_UNAVAILABLE`. Sérült lánc, hiányzó partíció, teli lemez
vagy elérhetetlen adatbázis: kezelt hiba, összeomlás nincs (AGENTS.md §0).

**Nyitott (#51):** a „minden egy parancs” elv a Rust `crates/engine` parancsregiszterére épül. A BacklotHub
üzleti parancsai Python-folyamatban futnak. Javaslat: a BacklotHub-szolgáltatás **saját parancsregisztert** kap
**ugyanazzal a leíróval** (id, címke, menüút, gyorsbillentyű, paraméterek, `enabled()`, `run()`), saját
JSON-lines vezérlőcsatornával és MCP-felülettel. A Qt6-héj mindkét regisztert ugyanúgy kezeli. Ez nem csak az
audit kérdése, hanem az egész BacklotHubé, ezért a 2. lépcső G2-jén véglegesítendő. Az auditparancsok ehhez
igazodnak.

## 11. Az AU-1 hatóköre (G4-re)

- PostgreSQL-migráció (séma, típusok, táblák, szerepek, `append()`, trigger, kezdő partíciók, genesis).
- Python `cfa4_audit`: `AuditEvent` (pydantic nélkül, `dataclass` + kézi validálás; kevesebb függőség),
  kanonikus forma, hash, `unit()` / `record()`.
- Rust `cfa4-audit` crate (L1, csak `serde`, `serde_json`, `sha2`): kanonikus forma és hash, **adatbázis
  nélkül**. JCS-crate helyett saját, szűkített megvalósítás (≈150 sor), mert a szűkítés (nincs float) a
  külső crate-ből nem kényszeríthető ki.
- Közös tesztvektorok, mindkét oldalon futtatva.
- Licencellenőrzés (G3): psycopg (LGPL-3.0-or-later), `sha2`/`serde_json` (MIT/Apache-2.0); kód nem kerül át
  donorból (FA3-minta csak mint terv).

## 12. Döntési kérdések (a közös listában: #48–#51)

| # | Kérdés | Javaslat |
|---|---|---|
| 48 | Hol fusson az adatbázis, és egy adatbázisban legyen-e az audit a BacklotHubbal? | Ubuntu `postgresql`, külön `cfa4` klaszter, csak Unix-socket, `peer`; egy `cfa4` adatbázis, az audit a `backlot` sémával egy adatbázisban |
| 49 | Melyik PostgreSQL-meghajtó legyen? | Python: psycopg 3 (a TASKSTORE is ezt használja). Rust: az EF-R1 G2-jén, addig a Rust-crate adatbázis nélkül |
| 50 | Elfogadható-e, hogy M1-ben a szándékos módosítást csak kimutatjuk, nem akadályozzuk meg? | Igen: jogok + trigger a programhibák ellen, hash-lánc + külső ellenőrzőpont a kimutatáshoz; valódi védelem M2-ben |
| 51 | Hogyan legyenek a Python BacklotHub üzleti műveletei CFA4-parancsok? | Saját regiszter ugyanazzal a parancsleíróval, saját vezérlőcsatorna és MCP; véglegesítés a 2. lépcső G2-jén |

## 13. G2-eredmény

**Javaslat: PASS a #48–#51 jóváhagyásával.** Utána: ADR 0006-kiegészítés (#48, #49) és az `AGENTS.md`
#21-sora külön PR-okban, majd az AU-1 megvalósítása (G4), ha a #767 a `main`-ben van.

## 14. Megvalósítási pontosítások (AU-1, 2026-11-22)

Az AU-1 megvalósítása során a G2-től ezek a pontosítások kerültek be (a kód és a teszt ezt követi):

- **`cfa4_audit.lock_head()`**: a 7. pont 1. lépése (`SELECT … FOR UPDATE` a `chain_head`-en) a `writer` szerepnek
  nem lehet, mert a `FOR UPDATE` `UPDATE` jogot kér, a `writer`-nek pedig a táblához nincs joga (5. pont). Ezért egy
  `SECURITY DEFINER` függvény (`lock_head()`) veszi el a zárat, és adja vissza a lánc végét.
- **`recorded_at` = a zár megszerzése utáni `clock_timestamp()`**, nem a `now()`: a `now()` a tranzakció kezdete, ami
  párhuzamos íróknál korábbi lehet, mint a már beírt utolsó esemény ideje, és a havi partíció nem lenne a lánc
  összefüggő szakasza. A `lock_head()` ezt az időt is visszaadja (`now_utc`).
- **`chain_head.recorded_at`** új oszlop: az `append()` ezzel utasítja el a visszalépő időt (`AUDIT_CHAIN_CONFLICT`).
- **`personal_detail`**: új oszlopok `subject_type`, `subject_id` (a törlés tárgy szerint megy), `salt` (16 bájt);
  a `detail` és a `salt` törlés után `NULL` (`erased_at` jelzi).
- **Függvények**: `write_personal()`, `erase_personal()` (`SECURITY DEFINER`), `ensure_partitions(n)` (csak
  `cfa4_audit_owner`); a `reader` a `personal_detail`-t **nem** olvashatja.
- **Hibakód és SQLSTATE**: `AUDIT_APPEND_ONLY` = `AU001`, `AUDIT_CHAIN_BROKEN` = `AU002`, `AUDIT_CHAIN_CONFLICT` = `AU003`.
  Új Python-oldali kódok: `AUDIT_EVENT_NOT_OBJECT`, `AUDIT_EVENT_INVALID`, `AUDIT_INVALID_VALUE`,
  `AUDIT_SCHEMA_INSTALL_FAILED`.
- **Mozgatás az alapértelmezett partícióból**: az `ensure_partitions` a `DEFAULT` partíciót lecsatolja, a sorokat az új
  hónapba másolja, majd visszacsatolja; a lánc tartalma nem változik. A telepítő (`cfa4_audit.migrate.install`) a
  migrációt és a genesis eseményt **egy tranzakcióban** futtatja.
- **Függőség**: `psycopg` (`backlot` extra, `requirements/linux-amd64.txt`).
