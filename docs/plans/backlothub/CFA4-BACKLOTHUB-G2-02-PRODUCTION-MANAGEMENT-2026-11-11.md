# BacklotHub 2. lépcső – Production Management: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-11 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.1, 9., 10. · **Előzmény:**
[G1 Production Management](CFA4-BACKLOTHUB-G1-02-PRODUCTION-MANAGEMENT-2026-11-06.md) (funkcionális
követelmények jóváhagyva 2026-11-10; #42 eldöntve 2026-11-08) · **Épül rá:**
[G2 Audit Trail](CFA4-BACKLOTHUB-G2-01-AUDIT-TRAIL-2026-11-10.md) · **Állapot:** G2 **jóváhagyva** (István, 2026-11-11); a #48–#52 eldöntve · implementáció nincs

Ez az első üzleti modul, ezért a G2 a **BacklotHub-szolgáltatás vázát** is rögzíti (folyamat, csomag,
adatbázis-réteg, migráció, közös oszlopok). A későbbi lépcsők erre épülnek, és csak a saját modelljüket adják hozzá.

## 1. Függőségek

| Mi | Honnan | Állapot |
|---|---|---|
| Adatbázis-klaszter és séma-elhelyezés | #48 (G2-01, 4.) | Jóváhagyásra vár |
| Python-meghajtó (psycopg 3) | #49 (G2-01, 3.) | Jóváhagyásra vár |
| Parancsregiszter Pythonban | #51 (G2-01, 10.) | Jóváhagyásra vár; **itt véglegesítendő** (3. pont) |
| Auditírás egy tranzakcióban | CFA4-AUDIT-001 AU-1…AU-3 | Terv a `main`-ben; megvalósítás az 1. lépcsőben |
| Python az üzleti logikára | #21; `AGENTS.md`-pont: PR #915 (a gazdánál) | Döntés megvan |
| Jogosultság M1-ben | #42 | Eldöntve: tagság és szerep tárolva, az OS-felhasználó a tulajdonos |

Megvalósítani (G4) csak az 1. lépcső AU-3 után lehet: minden produkciós változás auditeseményt ír.

## 2. A BacklotHub-szolgáltatás váza (minden későbbi lépcsőre érvényes)

- **Folyamat:** `cfa4-backlothub` Python-szolgáltatás (`python/cfa4_backlothub/`), a szerkesztőtől külön
  folyamat. A Qt6-felület (PySide6) és a CLI/MCP a vezérlőcsatornán át éri el. Hiba vagy kivétel a
  szolgáltatásban angol hibakóddal tér vissza, a felületet és a szerkesztőt nem dönti le (AGENTS.md §0).
- **Csomagszerkezet:**
  ```
  python/cfa4_backlothub/
    service.py        # indítás, kapcsolatkészlet, vezérlőcsatorna
    commands.py       # parancsregiszter (3. pont)
    db/               # kapcsolat, tranzakció-egység, közös oszlopok
    migrations/       # Alembic, egy lánc a teljes `backlot` sémára
    production/       # 2. lépcső: model.py, service.py, commands.py
  ```
- **Adatbázis-réteg:** SQLAlchemy 2 (MIT), Core és ORM, a psycopg 3 meghajtón; migráció: **Alembic** (MIT).
  Indok: a Stalker (a G1 elsődleges szerkezeti referenciája) is ezt használja, így a minták adaptálhatók;
  a 26 modul sémaváltozásait egy verziózott, tesztelhető lánc kezeli; mindkettő Ubuntu-csomagként is elérhető
  (`python3-sqlalchemy`, `python3-alembic`). **Új döntési kérdés: #52.**
- **Tranzakció-egység:** minden parancs egy tranzakció: üzleti változás + auditesemény (G2-01, 7. pont: az
  `audit.record` a `COMMIT` előtti utolsó lépés). Egységes segéd: `with uow() as u: … u.audit(...)`.
- **Közös oszlopok** minden üzleti táblán:

  | Oszlop | Típus | Leírás |
  |---|---|---|
  | `id` | `uuid` | UUIDv7, a kliens generálja (DATA-02) |
  | `production_id` | `uuid` | a produkció (a `production` táblán maga az `id`); minden lekérdezés szűr rá |
  | `created_at`, `updated_at` | `timestamptz` | UTC |
  | `created_by`, `updated_by` | `text` | `actor_ref` (M1: `uid:1000/istvan`), ugyanaz, mint az auditban |
  | `row_version` | `integer` | optimista zárolás; eltérésnél `CONFLICT_STALE_VERSION` |

  A „ki és mikor” oszlop csak gyors megjelenítésre van; a hiteles történet az auditnapló.
- **Törlés:** üzleti adat nem törlődik fizikailag normál művelettel. Helyette állapot (`cancelled`,
  `closed`) vagy `retired_at`. Kivétel: üres, frissen létrehozott tétel törölhető, auditálva.

## 3. Parancsok és a parancsregiszter (#51 véglegesítése)

**Javaslat:** a BacklotHub saját regisztert kap, a `crates/engine` parancsleírójával azonos mezőkkel:
`id`, `label` (fordítási kulcs), `menu_path`, `shortcut`, `params` (JSON Schema), `enabled(ctx)`, `run(ctx,
params)`. A regiszter a vezérlőcsatornán `commands.list` válaszban adja ki a leírókat, így a Qt6-héj, a CLI és
az MCP ugyanúgy kezeli, mint a motor parancsait. Az azonosítók névtere `backlot.` (pl.
`backlot.production.create`), hogy a motorparancsokkal ne ütközzön.

A 2. lépcső parancsai:

| Parancs | Mit csinál | Auditművelet |
|---|---|---|
| `backlot.production.create` | Új produkció alapadatokkal, típussal; a létrehozó a tulajdonos tag | `production.created` |
| `backlot.production.update` | Alapadatok módosítása (`row_version`-nel) | `production.updated` |
| `backlot.production.list` / `.get` | Lista szűréssel (típus, státusz, aktív) / egy produkció | – |
| `backlot.production.status.set` | Státuszváltás a típus státuszlistáján belül | `production.status.changed` |
| `backlot.production.milestone.add` / `.update` / `.remove` | Mérföldkövek | `production.milestone.*` |
| `backlot.organization.create` / `.update` / `.list` | Szervezetek (közösek a produkciók között) | `organization.*` |
| `backlot.production.organization.link` / `.unlink` | Szervezet szereppel a produkcióban | `production.organization.*` |
| `backlot.production.member.list` | Tagok és szerepek | – |
| `backlot.production.type.*`, `backlot.status_list.*` | Típusok és státuszlisták beállítása | `config.*` |

Az M1-ben a `member.add`/`.remove` nincs, mert egy felhasználó van (#42); a tábla és a modell kész.

## 4. Adatmodell (DDL-vázlat, `backlot` séma)

```sql
CREATE TABLE backlot.status_list (              -- Stalker StatusList-minta
    id uuid PRIMARY KEY, code text UNIQUE NOT NULL, target text NOT NULL  -- 'production', később 'task' …
);
CREATE TABLE backlot.status (
    id uuid PRIMARY KEY, status_list_id uuid NOT NULL REFERENCES backlot.status_list(id),
    code text NOT NULL, label_key text NOT NULL, ordinal smallint NOT NULL,
    category text NOT NULL CHECK (category IN ('planned','active','on_hold','closed','cancelled')),
    UNIQUE (status_list_id, code)
);

CREATE TABLE backlot.production_type (          -- film, sorozat, reklám, animáció, VFX + saját
    id uuid PRIMARY KEY, code text UNIQUE NOT NULL, label_key text NOT NULL,
    status_list_id uuid NOT NULL REFERENCES backlot.status_list(id),
    retired_at timestamptz, ...közös oszlopok production_id nélkül
);

CREATE TABLE backlot.production (
    id              uuid PRIMARY KEY,
    code            text NOT NULL UNIQUE CHECK (code ~ '^[A-Z0-9][A-Z0-9_-]{1,31}$'),
    title           text NOT NULL,
    type_id         uuid NOT NULL REFERENCES backlot.production_type(id),
    status_id       uuid NOT NULL REFERENCES backlot.status(id),
    country         char(2) NOT NULL,            -- ISO 3166-1 alpha-2
    currency        char(3) NOT NULL,            -- ISO 4217 (#15); a G2-24 3.1 szerint idegen kulcs a backlot.currency-re (pénznemlista, a 2. lépcső migrációja)
    reference_currency char(3) NOT NULL CHECK (reference_currency IN ('EUR','USD')),
    fps_num         integer NOT NULL CHECK (fps_num > 0),   -- racionális: 24000/1001
    fps_den         integer NOT NULL CHECK (fps_den > 0),
    width           integer, height integer,
    pixel_aspect_num integer NOT NULL DEFAULT 1, pixel_aspect_den integer NOT NULL DEFAULT 1,
    start_timecode  text,                        -- pl. 01:00:00:00
    episode_count   integer CHECK (episode_count >= 0),
    planned_start   date, planned_end date,
    description     text,
    settings        jsonb NOT NULL DEFAULT '{}', -- modulok saját beállításai, kulcs = modul
    ...közös oszlopok (production_id = id)
);

CREATE TABLE backlot.milestone (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    name text NOT NULL, due_date date NOT NULL, done_at timestamptz, ordinal smallint NOT NULL,
    ...közös oszlopok
);

CREATE TABLE backlot.organization (             -- közös: több produkcióban is szerepelhet
    id uuid PRIMARY KEY, name text NOT NULL, legal_name text, country char(2),
    registration_no text, tax_no text, notes text, retired_at timestamptz,
    ...közös oszlopok production_id nélkül
);
CREATE TABLE backlot.production_organization (
    production_id uuid REFERENCES backlot.production(id),
    organization_id uuid REFERENCES backlot.organization(id),
    role text NOT NULL CHECK (role IN ('client','producer','coproducer','subcontractor','distributor','other')),
    PRIMARY KEY (production_id, organization_id, role)
);

CREATE TABLE backlot.production_member (        -- #42: M1-ben egy sor, a tulajdonos
    production_id uuid REFERENCES backlot.production(id),
    actor_kind text NOT NULL CHECK (actor_kind IN ('os_user','identity')),
    actor_ref text NOT NULL,
    role text NOT NULL CHECK (role IN ('owner','producer','coordinator','artist','viewer')),
    PRIMARY KEY (production_id, actor_kind, actor_ref)
);
```

**Megjegyzések:**
- Az **fps racionális szám**, nem lebegőpontos; a CFA4 `Tick`-hez (254 016 000 000/s) pontosan átváltható.
- A szervezet adószáma, cégjegyzékszáma nem személyes adat; a kapcsolattartó személyek a 18. lépcsőben
  (Crew) jönnek, személyes adatként kezelve.
- A **`settings` jsonb** csak modulbeállításra való (pl. kódolási sablon); üzleti adat saját táblába kerül.
- Kezdő adatok (migrációban): az öt alaptípus és egy alap státuszlista (`development` → `pre_production` →
  `production` → `post_production` → `delivered` → `closed`, valamint `on_hold`, `cancelled`).
- **Státuszváltás M1-ben:** bármely státuszra a listán belül, auditálva. A 4. lépcső (Workflow Engine)
  után a produkció állapota folyamatdefinícióra kerül; a státuszlista és az adatok megmaradnak, csak az
  átmeneteket ellenőrzi a motor. Ez nem stub: a G1 szerint ez a valós M1-működés.

## 5. Integrációs határok

| Határ | Irány | Szerződés |
|---|---|---|
| Auditnapló | BacklotHub → `cfa4_audit` | Minden módosító parancs egy auditeseményt ír, `scope = {"production_id": …}`, `subject_type = production / milestone / organization …` |
| Qt6-felület, CLI, MCP | → parancsok | Csak a `backlot.*` parancsokon át; közvetlen adatbázis-hozzáférés nincs |
| Vágóprojekt (`cfa4-project`) | nincs a 2. lépcsőben | Később a produkció UUID-vel hivatkozhat vágóprojektre; a két modell külön marad (#4) |
| Event Fabric | nincs a 2. lépcsőben | Eseményközzététel (outbox) az EF-R1 és a 12. lépcső után; a séma most nem kap outbox-táblát, hogy ne legyen hívó nélküli rész |
| Naptár (17.) | később | A mérföldkövek a naptárban megjelennek; a mérföldkő marad a forrás |
| Költségvetés (24.) | később | A `currency` és a `reference_currency` innen jön |
| Workflow Engine (4.) | később | Átveszi az átmenetek ellenőrzését (4. pont) |

## 6. Hibakódok

`PRODUCTION_CODE_TAKEN`, `PRODUCTION_NOT_FOUND`, `STATUS_NOT_IN_LIST`, `CURRENCY_UNKNOWN`, `COUNTRY_UNKNOWN`,
`FPS_INVALID`, `ORGANIZATION_IN_USE`, `CONFLICT_STALE_VERSION`, `PERMISSION_DENIED` (M2-től ténylegesen;
M1-ben a nem tag OS-felhasználóra), valamint az audit hibakódjai (G2-01, 10.).

## 7. Tesztek (G5-re előre)

- Migráció fel és vissza üres adatbázison; a kezdő adatok egyszer kerülnek be.
- Minden módosító parancs pontosan egy auditeseményt ír; ha az audit elbukik, az üzleti változás sincs meg.
- Optimista zárolás: két párhuzamos módosításból az egyik `CONFLICT_STALE_VERSION`.
- 52 aktív produkció a CFA4-MIN gépen az S-profillal: lista és produkcióváltás válaszideje (SC-02).
- Hibás bemenet (fps 0, ismeretlen pénznem, túl hosszú kód): angol hibakód, összeomlás nincs.

## 8. Licenc (G3-ra előre)

- SQLAlchemy 2, Alembic: MIT; psycopg 3: LGPL-3.0-or-later; mind összeegyeztethető a GPL-3.0-or-later-rel.
- Stalker (LGPL-3.0): szerkezeti minta (típus, státuszlista, ügyfél-szerep); ha kód kerül át, az
  attribúcióval jár (`ATTRIBUTION.md`). Javaslat: **kódot nem veszünk át**, a minta elég.
- Kitsu (AGPL-3.0): csak a technikai alapmezők listája mint referencia, kód nem.

## 9. Döntési kérdés

| # | Kérdés | Javaslat |
|---|---|---|
| 52 | Milyen adatbázis-réteggel és migrációs eszközzel készüljön a BacklotHub? | SQLAlchemy 2 (Core + ORM) és Alembic, mindkettő MIT, a psycopg 3-on; egy migrációs lánc a `backlot` sémára |

## 10. G2-eredmény

**Javaslat: PASS a #48, #49, #51 és #52 jóváhagyásával.** A 3. lépcső (Production Structure) G2-je erre a
vázra épül.

## 11. Megvalósítási pontosítások (PM-1, 2026-11-23)

A megvalósítás során a G2-től ezek a pontosítások kerültek be (a kód és a teszt ezt követi):

- **Lépcsőzés:** PM-1 = váz (csomag, Alembic-lánc, tranzakció-egység auditeseménnyel, parancsregiszter), a
  `production.create/update/get/list/status.set` és a `member.list`; PM-2 = mérföldkő, szervezet, szervezet-kapcsolat,
  típus- és státuszlista-beállítás (külön migrációk ugyanabban a láncban).
- **Országlista és pénznemlista forrása** (a G3 ISO 4217 feltétele): `scripts/dev/gen-backlot-data.py` generálja a
  rendszerből: ország = IANA `tzdata` `iso3166.tab` (közkincs), pénznem = Debian `iso-codes` (LGPL-2.1-or-later), a kitevőt
  (0/2/3 tizedes) a CFA4 adja hozzá. A migráció ezeket tölti a `backlot.country` és `backlot.currency` táblába; a
  `production.country` is idegen kulcs. Az ISO közzétevőjének saját használati feltétele ebből a környezetből nem
  ellenőrizhető; a két forrás a saját licencét adja.
- **Az `delivered` státusz kategóriája `closed`** (a `closed` is `closed`), így az „aktív" szűrő (`planned`/`active`/`on_hold`)
  a leszállított produkciót már nem tartalmazza.
- **`production.production_id`** generált oszlop (`= id`), hogy minden üzleti tábla egységesen szűrhető legyen; az alkalmazás nem írja.
- **A státusz a típus listájából való**: a szolgáltatás `STATUS_NOT_IN_LIST`-tel utasítja el, és egy adatbázis-trigger is megköveteli.
- **Jogosultság M1-ben:** a tulajdonos/producer/koordinátor módosíthat, csak a tulajdonos/producer válthat státuszt,
  a nem tag `PERMISSION_DENIED`-et kap (a lista csak a saját produkciókat adja).
- **Mérés (SC-02, 4 vCPU stand-in, 52 produkció):** lista 3,4 ms medián (p95 4,3 ms), egy produkció megnyitása 3,1 ms, létrehozás 7,6 ms (audittal).

## 12. Megvalósítási pontosítások (PM-2, 2026-11-23)

- **Mérföldkő:** a `milestone.list` olvasó parancs is bekerült. A kész (`done`) mérföldkő tény, **nem törölhető**
  (`MILESTONE_DONE`); a nem kész törölhető, auditálva az utolsó értékeivel. A sorrend: esedékesség, majd pozíció.
- **Szervezet:** közös a produkciók között, ezért M1-ben bármely OS-felhasználó létrehozhatja és szerkesztheti; a
  produkcióhoz kapcsolás írási szerepet kíván. Nincs törlés: a `retired` jelzés visszavonja a használatot, és
  `ORGANIZATION_IN_USE` jelzi, ha nyitott (tervezett, aktív, szüneteltetett) produkcióhoz még kapcsolódik; visszahozható.
  A kapcsolat szerepenként egyedi (`ORGANIZATION_ALREADY_LINKED`, `ORGANIZATION_NOT_LINKED`), a visszavont szervezet nem
  kapcsolható (`ORGANIZATION_RETIRED`).
- **Típus és státuszlista:** `backlot.production.type.create/update/list`, `backlot.status_list.create/list`,
  `backlot.status.add/update`. A saját típus és státusz nevét `text:<név>` címke tárolja (a mintaadatok fordítási
  kulcsok). A típus státuszlistája csak addig cserélhető, amíg nincs a típusnak produkciója (`PRODUCTION_TYPE_IN_USE`);
  a visszavont típusra nem jön létre új produkció. Státusz nem törölhető. M1-ben a beállítás bármely OS-felhasználóé (egy
  felhasználó van, #42); M2-től szerepellenőrzés lép a helyére. Auditesemények: `config.*`, produkcióhatókör nélkül.
