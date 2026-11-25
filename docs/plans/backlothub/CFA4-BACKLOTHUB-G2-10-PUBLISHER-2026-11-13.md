# BacklotHub 10. lépcső – Publisher: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-13 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.5 · **Előzmény:**
[G1 Publisher](CFA4-BACKLOTHUB-G1-10-PUBLISHER-2026-11-06.md) (funkcionális követelmények jóváhagyva
2026-11-10) · **Épül rá:** G2-01…G2-09 (jóváhagyva; főleg G2-08 Asset és G2-09 útsablon) · **Állapot:** G2 **jóváhagyva** (István, 2026-11-14) · implementáció nincs

Új döntési kérdés nincs, új függőség nincs. **Egy pontosítás a jóváhagyott G2-08-hoz** (3. pont): a
képsorok miatt a reprezentáció és a fájl között egy-a-többhöz kapcsolat kell.

## 1. Szerep és határ

A Publisher a **folyamat**, amely egy munkafájlból vagy egy export kimenetéből publikált verziót készít. A
verzió, a termék és a reprezentáció modellje a G2-08-é; a Publisher ezeket hozza létre, a G2-08
`version_default` folyamatának `publish` átmenetével. Két út marad:

- **Publisher** (a szokásos út): profil szerint gyűjt, ellenőriz, kiír, integrál;
- **kézi verzió** (`backlot.version.create`, G2-08): fájlok kézi feltöltése vázlatként, majd `publish`.
  Ugyanazokat az őrfeltételeket kapja, de profil szerinti ellenőrzés nélkül.

A Publisher és a Loader (11.) külön modul (terv 4.6).

## 2. Négy fázis (Pyblish-minta, saját megvalósítás)

| Fázis | Mit csinál | Hibánál |
|---|---|---|
| **Collect** | A forrás (munkafájl, export kimenete, megadott fájlok vagy képsor) összegyűjtése; a kontextus (produkció, elem, feladat, termék) a forrásból vagy a hívótól | `PUBLISH_SOURCE_MISSING`; semmi nem változik |
| **Validate** | A profil ellenőrzései (4. pont); jelentés tételenként: megfelel / figyelmeztetés / hiba, okkal | Hiba: `PUBLISH_VALIDATION_FAILED`, semmi nem változik. Figyelmeztetés: csak `accept_warnings` jelzővel megy tovább |
| **Extract** | Fájlok a File Services `publish` gyökerének `staging` részébe (ellenőrzött másolás, kivonat), az útsablon szerint (G2-09) | A `staging` takarítódik; semmi nem változik |
| **Integrate** | (a) a Rust-adatút véglegesíti és lezárja (`seal`) a fájlokat; (b) **egy tranzakcióban**: katalógusrekordok, verzió (`draft` → `publish` átmenet), reprezentációk, a futás jelentése, auditesemény `publish.committed` | Ha (b) elbukik: a már lezárt fájlok azonnal `deleted` állapotba kerülnek (a File Services kétlépcsős törlése), hivatkozás nélkül; a futás `failed`. Félig publikált verzió nem lehet |

Az (a)–(b) sorrend a File Services szabálya: a katalógus soha nem mutat félkész fájlra (ADR 0009). Az
összeomlás (a) és (b) között: indításkor a futás `integrating` állapotban marad, a helyreállítás a fájlokat
`deleted`-re teszi, a futást `failed`-re, és a publikálás megismételhető (G2-08 tesztje).

**Utómunka a véglegesítés után** (a publikálást nem vonja vissza): `probe` az eredetire, bélyegkép és
review-proxy a Media Services jobjaként (MS-1), a `proxy_profile` szerint (a V1 után, G2-08 6. pont). A
reprezentáció állapota `pending` → `ready` / `failed`.

## 3. Pontosítás a G2-08-hoz: képsor = egy reprezentáció, sok fájl

Egy EXR-képsor (pl. 2000 kocka) egy `original` reprezentáció, de 2000 `file_object`. Ezért a
`representation.file_object_id` helyett:

```sql
CREATE TABLE backlot.representation_file (
    representation_id uuid NOT NULL REFERENCES backlot.representation(id) ON DELETE RESTRICT,
    file_object_id    uuid NOT NULL REFERENCES cfa4_files.file_object(id) ON DELETE RESTRICT,
    frame             integer,                   -- képsornál a kockaszám, egyedi fájlnál NULL
    PRIMARY KEY (representation_id, file_object_id),
    UNIQUE (representation_id, frame)
);
-- representation: + kind ('single','sequence'), frame_in, frame_out, file_count
```

Egyedi fájl = egy sor `frame` nélkül. A G2-08 többi része (szabályok, változtathatatlanság, folyamat)
változatlan; a „minden `original` lezárt” őrfeltétel a reprezentáció összes fájljára vonatkozik.

## 4. Publikálási profil és ellenőrzések

```json
{
  "schema": "cfa4.backlot.publish_profile/1",
  "product_type": "comp",
  "sources": ["workfile_output", "files"],
  "representations": [
    {"role": "original", "kind": "sequence", "extensions": ["exr"]},
    {"role": "review_proxy", "derived": true},
    {"role": "thumbnail", "derived": true}
  ],
  "validators": [
    {"id": "files_readable"},
    {"id": "extension_in", "params": {"values": ["exr"]}},
    {"id": "sequence_complete"},
    {"id": "frame_range_matches_entity", "params": {"include_handles": true}},
    {"id": "probe_matches", "params": {"resolution": "production", "fps": "production"}},
    {"id": "metadata_required", "params": {"fields": ["note"]}, "severity": "warning"}
  ],
  "path_template": "publish_default",
  "hero": true
}
```

- **Ellenőrzők zárt regiszterből**, paramétersémával (mint a G2-04 predikátumai): `files_readable`,
  `extension_in`, `size_max`, `sequence_complete` (nincs hiányzó vagy dupla kocka), `frame_range_matches_entity`
  (a snitt `entity_timing`-je, G2-03), `probe_matches` (felbontás, fps, hangcsatornák; Media Services `probe`),
  `metadata_required`, `name_matches_template`. Tetszőleges kód nem fut; bővítmény a DCC Hostban (11a.), az ott
  elfogadott szabályok szerint.
- Profil termék-típusonként, produkciós felülírással; aktiváláskor validálva, aktiválás után
  változtathatatlan (új verzió), mint a folyamatdefiníció (G2-04).
- **Hero verzió:** a termék `hero_version_id` mutatója a legutóbb jóváhagyott verzióra (a `approve` átmenet
  állítja). **M1-ben csak adatbázis-mutató**, fizikai „stabil út” (másolat vagy kemény link) nincs; a Loader
  (11.) a mutatót oldja fel. Fizikai hero-út később, igény szerint.

## 5. Adatmodell (a G2-08-on felül)

```sql
CREATE TABLE backlot.publish_profile (
    id uuid PRIMARY KEY, code text NOT NULL, version integer NOT NULL,
    product_type_id uuid NOT NULL REFERENCES backlot.product_type(id),
    production_id uuid REFERENCES backlot.production(id),
    state text NOT NULL CHECK (state IN ('draft','active','retired')),
    definition jsonb NOT NULL, digest bytea,
    ...közös oszlopok production_id nélkül,
    UNIQUE (code, version)
);

CREATE TABLE backlot.publish_run (             -- futásállapot és ellenőrzési jelentés; a történet az auditban
    id              uuid PRIMARY KEY,
    production_id   uuid NOT NULL REFERENCES backlot.production(id),
    profile_id      uuid NOT NULL REFERENCES backlot.publish_profile(id),
    product_id      uuid REFERENCES backlot.product(id),
    source          jsonb NOT NULL,             -- munkafájl-id, export-azonosító vagy fájllista
    state           text NOT NULL CHECK (state IN
                    ('collecting','validating','extracting','integrating','done','rejected','failed','cancelled')),
    report          jsonb,                      -- ellenőrzőnként: eredmény, ok, részletek
    version_id      uuid REFERENCES backlot.version(id),
    error_code      text,
    started_at      timestamptz NOT NULL, ended_at timestamptz,
    ...közös oszlopok
);

ALTER TABLE backlot.product ADD COLUMN hero_version_id uuid REFERENCES backlot.version(id);

CREATE TABLE backlot.version_input (           -- a G1-11 kérése: miből készült a verzió (AYON input links)
    version_id       uuid NOT NULL REFERENCES backlot.version(id) ON DELETE RESTRICT,
    input_version_id uuid NOT NULL REFERENCES backlot.version(id) ON DELETE RESTRICT,
    source           text NOT NULL CHECK (source IN ('load_record','declared')),
    PRIMARY KEY (version_id, input_version_id),
    CHECK (version_id <> input_version_id)
);
```

**Bemenetek (`version_input`)** a Collect fázisban gyűlnek, és az Integrate tranzakciójában íródnak:
- a forrás munkafájlba betöltött verziókból (a Loader `load_record`-jai, 11. lépcső);
- a hívó által megadott listából (`declared`), amíg az adott alkalmazáshoz nincs Loader-adapter.

Kör nem lehet: a bemenet mindig korábban publikált verzió, a változtathatatlanság miatt utólag nem módosul.

A „publikálási napló” az auditnapló szűrt nézete (`publish.*` műveletek) és a `publish_run` jelentése; külön
naplótábla nincs. Az elutasított futás jelentése megmarad (miért nem ment át), és auditesemény is lesz
(`outcome = rejected`).

## 6. Integrációs határok

| Határ | Irány | Szerződés |
|---|---|---|
| File Services | → | `files.import` a `staging`-be, véglegesítés + `seal`, hibánál `files.delete` |
| Media Services (MS-1) | → | `probe` (ellenőrzéshez és utána), `thumbnail`, `proxy.create` |
| Asset (8.) | → | Verzió, reprezentáció, `representation_file`, `version_default` folyamat |
| Workfiles (9.) | ← | Forrás: munkafájl; útsablon-motor |
| CFA4 szerkesztő | ← | Export után `backlot.publish.run` a kimenettel és a `.fcproj` kontextusával (G2-09 `context`) |
| DCC Host (11a.) | később | DCC-oldali collect/validate, ugyanerre a futtatóra |
| Event Fabric (12.) | később | `publish.committed` esemény outboxból |

## 7. Parancsok

| Parancs | Mit csinál |
|---|---|
| `backlot.publish.validate` | Szárazfuttatás: Collect + Validate, jelentés; semmit nem ír |
| `backlot.publish.run` | Teljes futás háttérfeladatként; azonnal visszaadja a futás azonosítóját |
| `backlot.publish.status` / `.report` | Állapot, haladás, jelentés |
| `backlot.publish.cancel` | Megszakítás az Integrate előtt (`staging` takarítás) |
| `backlot.publish.profile.*` | Profilok kezelése (`validate`, `activate`, …) |
| `backlot.product.hero` | A hero verzió lekérdezése |

## 8. Hibakódok

`PUBLISH_SOURCE_MISSING`, `PUBLISH_VALIDATION_FAILED`, `PUBLISH_WARNINGS_NOT_ACCEPTED`,
`PUBLISH_PROFILE_INVALID`, `PUBLISH_PROFILE_INACTIVE`, `PUBLISH_CANCELLED`, `PUBLISH_INTEGRATE_FAILED`, a
G2-08 `VERSION_*` kódjai, valamint a `FILES_*` és a Media Services kódjai.

## 9. Tesztek (G5-re előre)

- Hibainjektálás minden fázishatáron (Extract közben, a `seal` után, a tranzakció előtt és közben, összeomlás):
  soha nincs félig publikált verzió; a lezárt, de hivatkozás nélküli fájl `deleted`; a futás megismételhető.
- Képsor: hiányzó, dupla vagy rossz tartományú kocka → az ellenőrző elutasítja, okkal.
- Párhuzamos publikálás ugyanarra a termékre: folytonos verziószámok.
- Proxy-job hibája: a verzió publikált marad, a reprezentáció `failed`, újragenerálható.
- Mérés: 2000 kockás 4K EXR-sor publikálása a CFA4-MIN gépen (az Extract a File Services FS-1 mérésére épül).

## 10. Licenc (G3-ra előre)

Új függőség nincs. Pyblish (LGPL-3.0): a négy fázis mint minta, kód nem. AYON core (Apache-2.0): integrálás
és hero-verzió mint minta; kódátvételnél attribúcióval. Kitsu (AGPL): referencia.

## 11. G2-eredmény

**Javaslat: PASS**, a G2-08 3. pont szerinti pontosításával (`representation_file`). Megvalósítás: File
Services FS-1, Media Services MS-1 és a V1 után.
