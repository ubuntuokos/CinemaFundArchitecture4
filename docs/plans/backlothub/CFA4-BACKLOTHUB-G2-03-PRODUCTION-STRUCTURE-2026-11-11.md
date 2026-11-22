# BacklotHub 3. lépcső – Production Structure: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-11 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.2, 9. · **Előzmény:**
[G1 Production Structure](CFA4-BACKLOTHUB-G1-03-PRODUCTION-STRUCTURE-2026-11-06.md) (funkcionális
követelmények jóváhagyva 2026-11-10) · **Épül rá:** [G2-01 Audit](CFA4-BACKLOTHUB-G2-01-AUDIT-TRAIL-2026-11-10.md),
[G2-02 Production Management és szolgáltatásváz](CFA4-BACKLOTHUB-G2-02-PRODUCTION-MANAGEMENT-2026-11-11.md)
(mindkettő jóváhagyva 2026-11-11) · **Állapot:** G2 **jóváhagyva** (István, 2026-11-11) · implementáció nincs

Új döntési kérdés nincs. A G1 által G2-re hagyott technikai választást (`ltree` vagy lezárótábla) ez a
dokumentum javasolja (3. pont); a G2 jóváhagyása ezt is lefedi.

## 1. Függőségek

| Mi | Honnan | Állapot |
|---|---|---|
| Szolgáltatásváz, közös oszlopok, `backlot.*` parancsok, SQLAlchemy 2 + Alembic | G2-02, #51, #52 | Jóváhagyva |
| Produkció, produkciótípus, státuszlista | 2. lépcső | G2 jóváhagyva |
| Auditírás egy tranzakcióban | CFA4-AUDIT-001 AU-1…AU-3 | Terv a `main`-ben |
| `ltree` kiterjesztés | PostgreSQL contrib, az Ubuntu `postgresql-<verzió>` csomag része | Külön csomag nem kell |

Megvalósítás (G4) a 2. lépcső után.

## 2. Fogalmak

- **Szerkezeti elem:** évad, egység, epizód, spot, szekvencia, jelenet, snitt és saját típusok. Fába
  rendeződnek, a szabályokat a produkció szerkezeti sablonja adja.
- **Produkciós asset:** szereplő, kellék, helyszín-asset, jármű, effekt stb. **Nem a fa része**: a
  produkcióhoz tartozik, és linkkel kapcsolódik szerkezeti elemekhez (egy asset több snittben is szerepelhet).
  A fájlok és verziók a 8. lépcső (Asset Management) dolga; ott a verzió erre az elemre mutat.
- **Vágóprojekt-szekvencia** (`cfa4-project`) más fogalom: idővonal. Csak UUID-hivatkozás mutathat rá (#4).

## 3. Hierarchia tárolása: `ltree` (javaslat)

| Szempont | `ltree` | Lezárótábla |
|---|---|---|
| Tárolás | egy oszlop az elemen (`path`) | külön tábla, elemenként mélység × sor |
| Részfa-lekérdezés | `path <@ :p`, GiST-index | join a lezárótáblára |
| Áthelyezés | egy `UPDATE` a részfára (`subpath`) | a részfa összes sorának törlése és újraírása |
| Sorrend, mélység | `nlevel(path)` | külön mező |
| Függőség | contrib, a szerverrel együtt jön | nincs |

**Döntési javaslat: `ltree`.** A mélység kicsi (legfeljebb 5–6 szint), a részfa-lekérdezés a leggyakoribb
művelet (fanézet, feladatlista egy szekvencia alatt), és egy áthelyezés egyetlen utasítás. A címke az elem
UUID-je kötőjelek nélkül (32 hex karakter), így átnevezéskor a `path` nem változik.

A `parent_id` is megmarad (egyértelmű idegen kulcs). Az egyezést `path = parent.path || label(id)` trigger
tartja fenn, és a két mező nem térhet el.

## 4. Adatmodell (DDL-vázlat, `backlot` séma)

```sql
CREATE EXTENSION IF NOT EXISTS ltree;          -- migrációban, a sématulajdonos szerepével

CREATE TABLE backlot.entity_type (             -- közös, konfigurálható
    id          uuid PRIMARY KEY,
    code        text NOT NULL UNIQUE CHECK (code ~ '^[a-z][a-z0-9_]{1,31}$'),
    label_key   text NOT NULL,
    kind        text NOT NULL CHECK (kind IN ('structural','asset')),
    icon        text,
    has_timing  boolean NOT NULL DEFAULT false, -- snitt: igen
    status_list_id uuid REFERENCES backlot.status_list(id),
    retired_at  timestamptz,
    ...közös oszlopok production_id nélkül
);

CREATE TABLE backlot.structure_template (      -- produkciótípusonként alapsablon
    id uuid PRIMARY KEY, code text NOT NULL UNIQUE, label_key text NOT NULL,
    production_type_id uuid REFERENCES backlot.production_type(id)
);
CREATE TABLE backlot.structure_template_rule (
    template_id     uuid NOT NULL REFERENCES backlot.structure_template(id),
    parent_type_id  uuid REFERENCES backlot.entity_type(id),   -- NULL: a produkció gyökere alatt
    child_type_id   uuid NOT NULL REFERENCES backlot.entity_type(id),
    UNIQUE NULLS NOT DISTINCT (template_id, parent_type_id, child_type_id)
);

CREATE TABLE backlot.production_structure_rule ( -- létrehozáskor a sablonból másolva
    production_id   uuid NOT NULL REFERENCES backlot.production(id),
    parent_type_id  uuid REFERENCES backlot.entity_type(id),
    child_type_id   uuid NOT NULL REFERENCES backlot.entity_type(id),
    UNIQUE NULLS NOT DISTINCT (production_id, parent_type_id, child_type_id)
);

CREATE TABLE backlot.production_entity (
    id              uuid PRIMARY KEY,              -- UUIDv7
    production_id   uuid NOT NULL REFERENCES backlot.production(id),
    entity_type_id  uuid NOT NULL REFERENCES backlot.entity_type(id),
    parent_id       uuid REFERENCES backlot.production_entity(id),
    path            ltree NOT NULL,
    code            text NOT NULL,                 -- pl. SQ010, SH0040, CHR_ANNA
    name            text NOT NULL,
    ordinal         integer NOT NULL,              -- testvérek közti sorrend
    status_id       uuid REFERENCES backlot.status(id),
    description     text,
    metadata        jsonb NOT NULL DEFAULT '{}',   -- típusonkénti egyedi mezők
    retired_at      timestamptz,
    ...közös oszlopok,
    UNIQUE NULLS NOT DISTINCT (production_id, parent_id, entity_type_id, code)
);
CREATE INDEX ON backlot.production_entity USING gist (path);
CREATE INDEX ON backlot.production_entity (production_id, entity_type_id);

CREATE TABLE backlot.entity_timing (           -- csak has_timing típusnál (snitt)
    entity_id   uuid PRIMARY KEY REFERENCES backlot.production_entity(id),
    frame_in    integer NOT NULL, frame_out integer NOT NULL CHECK (frame_out >= frame_in),
    handle_in   integer NOT NULL DEFAULT 0, handle_out integer NOT NULL DEFAULT 0,
    otio_ref    text,                          -- későbbi conform (OTIO-terv)
    edit_project_ref uuid                      -- vágóprojekt UUID-je, ha van (#4)
);

CREATE TABLE backlot.entity_link (             -- nem hierarchikus kapcsolat
    id          uuid PRIMARY KEY,
    production_id uuid NOT NULL REFERENCES backlot.production(id),
    from_id     uuid NOT NULL REFERENCES backlot.production_entity(id),
    to_id       uuid NOT NULL REFERENCES backlot.production_entity(id),
    link_type   text NOT NULL CHECK (link_type ~ '^[a-z][a-z0-9_]*$'),  -- casting, depends_on, …
    metadata    jsonb NOT NULL DEFAULT '{}',
    ...közös oszlopok,
    UNIQUE (from_id, to_id, link_type),
    CHECK (from_id <> to_id)
);
```

**Az adatbázis által kikényszerített szabályok** (triggerrel, mert ezek sérülése adatvesztéshez vezetne):

- a szülő ugyanabban a produkcióban van;
- a (szülőtípus, gyerektípus) pár szerepel a `production_structure_rule`-ban;
- `asset` fajtájú elemnek nincs szülője, és nem lehet szülő;
- a `path` a szülő `path`-jából és a saját azonosítóból áll; áthelyezés saját részfája alá nem lehet;
- az `entity_link` két vége ugyanabban a produkcióban van.
- a `casting` típusú kapcsolat iránya: `from_id` = jelenet, snitt vagy szekvencia, `to_id` = `character` típusú elem (a [G2-19](CFA4-BACKLOTHUB-G2-19-CAST-MANAGEMENT-2026-11-21.md) 3.1 pontja szerint; az ellenőrzést a szolgáltatás végzi).

- a `setting` típusú kapcsolat iránya: `from_id` = jelenet, snitt vagy szekvencia, `to_id` = `set` típusú elem ([G2-20](CFA4-BACKLOTHUB-G2-20-LOCATION-MANAGEMENT-2026-11-21.md) 3.2); a `scene` elem `metadata` mezőjének zárt sémája `cfa4.backlot.scene_meta/1` ([G2-22](CFA4-BACKLOTHUB-G2-22-SCRIPT-BREAKDOWN-2026-11-21.md) 5.), ismeretlen kulcs `ENTITY_METADATA_INVALID`.

A szolgáltatás ugyanezeket előre ellenőrzi, hogy olvasható hibakódot adjon; a trigger a végső védvonal.

**Idő:** a snitt határa **képkockában** tárolódik; a `Tick` a produkció racionális fps-éből pontosan
számolható (`frame × 254 016 000 000 × fps_den / fps_num`), ezért nem tárolódik külön (nincs eltérő másolat).

**Kezdő adatok (migráció):**
- Típusok: `season`, `unit`, `episode`, `spot`, `sequence`, `scene`, `shot`; asset-típusok: `character`,
  `prop`, `set`, `vehicle`, `fx`, `environment`.
- Sablonok:

  | Sablon | Szerkezet |
  |---|---|
  | film | szekvencia → jelenet → snitt |
  | sorozat | évad → epizód → szekvencia → snitt |
  | reklám | spot → snitt |
  | animáció és VFX | a filmé, a produkcióban módosítható |

## 5. Átstrukturálás

| Művelet | Viselkedés | Audit |
|---|---|---|
| Létrehozás | Szabály szerint; az `ordinal` a testvérek végére kerül | `structure.entity.created` |
| Módosítás | Név, kód, leírás, metaadat, időzítés (`row_version`-nel) | `structure.entity.updated` (mezőnkénti régi/új) |
| Áthelyezés | Egy utasítás a részfára; a részfa elemei nem kapnak külön eseményt | `structure.entity.moved` (régi és új szülő, részfa mérete) |
| Sorrend | Testvérek átsorolása egy tranzakcióban | `structure.entity.reordered` |
| Összevonás | A gyerekek és a linkek átkerülnek a célelemre, a forrás `retired` lesz | `structure.entity.merged` |
| Kivezetés | `retired_at` beállítva; a részfa is; a hivatkozások megmaradnak | `structure.entity.retired` |
| Törlés | Csak ha nincs gyereke, linkje, és más modul sem hivatkozik rá | `structure.entity.deleted` |

**Más modulok hivatkozásai:** a feladat (5.), a verzió (8.) stb. `entity_id`-val hivatkozik, idegen kulccsal
(`ON DELETE RESTRICT`). Ezért a törlést az adatbázis is megakadályozza, ha hivatkozás van.

**Függőségtudatos változás (FA3-minta, CFA4-re szabva):** `backlot.structure.impact` megadja, mit érint egy
elem módosítása: a részfát és a `depends_on`/`casting` linkeken át elérhető elemeket. Az M1-ben ez
tájékoztató lekérdezés; automatikus továbbgyűrűzés (pl. feladat újranyitása) a 4. lépcső Workflow Engine-je
után, szabályként.

## 6. Parancsok

| Parancs | Mit csinál |
|---|---|
| `backlot.structure.tree` | Részfa egy elem vagy a produkció alatt, szintenként lapozva (lusta betöltés a felületen) |
| `backlot.structure.entity.create` / `.update` / `.move` / `.reorder` / `.merge` / `.retire` / `.delete` | Az 5. pont műveletei |
| `backlot.structure.link.add` / `.remove` / `.list` | Linkek |
| `backlot.structure.impact` | Érintett elemek |
| `backlot.structure.entity_type.*`, `backlot.structure.template.*`, `backlot.structure.rule.*` | Típusok, sablonok és a produkció szabályai |

## 7. Integrációs határok

| Határ | Irány | Szerződés |
|---|---|---|
| Auditnapló | → `cfa4_audit` | `scope = {"production_id", "entity_path"}`, így egy szekvencia teljes története szűrhető |
| Production Management (2.) | ← | Produkciótípus → sablon; létrehozáskor a szabályok átmásolódnak |
| Workflow Engine (4.) | → | Az elem állapota (`status_id`) később folyamatdefinícióra kerül, mint a produkcióé |
| Task Management (5.), Asset Management (8.), Review (13.) | → | `entity_id`-val hivatkoznak; törlés `RESTRICT` |
| Search (7.) | → | A `code` és a `name` a produkción belüli FTS-be kerül (#44) |
| Script Breakdown (22.), Shooting Schedule (23.), Location (20.) | → | A jelenet szerkezeti elem; a helyszín és a breakdown-elem linkkel kapcsolódik |
| OTIO / vágóprojekt | később | `entity_timing.otio_ref` és `edit_project_ref`; a conform külön lépés, nem a 3. lépcső része |
| Workfiles (9.) | később | A mappasablon az elem `path`-ja és kódjai alapján (Stalker `Structure`-minta) |

## 8. Hibakódok

`STRUCTURE_RULE_VIOLATION`, `STRUCTURE_CYCLE`, `STRUCTURE_CROSS_PRODUCTION`, `STRUCTURE_ASSET_HAS_PARENT`,
`ENTITY_CODE_TAKEN`, `ENTITY_IN_USE`, `ENTITY_NOT_FOUND`, `TIMING_INVALID`, `LINK_DUPLICATE`,
`CONFLICT_STALE_VERSION`.

## 9. Méret és tesztek (G5-re előre)

- **L-profil:** 52 produkció × 10 000 snitt + 2 000 asset ≈ 620 000 elem; a részfa-lekérdezés, egy 2 000 elemes
  részfa áthelyezése és a fanézet első szintje a CFA4-MIN gépen mérve (SC-02).
- Tulajdonságalapú tesztek: véletlen létrehozás, áthelyezés, összevonás sorozat után a `path` és a
  `parent_id` mindig egyezik, kör nincs, minden szabály teljesül.
- Minden módosító parancs pontosan egy auditeseményt ír; ha az audit elbukik, nincs változás.
- Hibás bemenet: angol hibakód, összeomlás nincs.

## 10. Licenc (G3-ra előre)

Új függőség nincs (`ltree`: PostgreSQL License). Kitsu (AGPL): csak a polimorf entitás + link minta, kód nem.
FA3 CreativeProjectGraph: a gráfszemlélet, CFA3-elemek nélkül. Stalker: csak a mappasablon ötlete a 9.
lépcsőhöz.

## 11. G2-eredmény

**Javaslat: PASS.** Új döntési kérdés nincs; a hierarchiatárolás: `ltree`. A 4. lépcső (Workflow Engine
mag) G2-je következik.
