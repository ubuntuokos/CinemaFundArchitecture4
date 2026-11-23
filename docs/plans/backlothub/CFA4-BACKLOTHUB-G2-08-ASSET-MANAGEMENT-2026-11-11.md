# BacklotHub 8. lépcső – Asset Management: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-11 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.4, 11. · **Előzmény:**
[G1 Asset Management](CFA4-BACKLOTHUB-G1-08-ASSET-MANAGEMENT-2026-11-06.md) (funkcionális követelmények
jóváhagyva 2026-11-10) · **Közös szolgáltatások:** File Services (ADR 0009, `main`), Media Services (ADR 0007,
`main`) · **Épül rá:** G2-01…G2-07 · **Állapot:** G2 **jóváhagyva** (István, 2026-11-11): a
proxyformátum rögzítve a V1 alapján (István, 2026-11-21): lásd a 15. sort · implementáció nincs

## 1. Kapu-feltételek

| Feltétel | Állapot |
|---|---|
| V0: Media Services meghatározása | Kész (ADR 0007) |
| File Services terve jóváhagyva | Kész (ADR 0009, #43, #45) |
| **V1: CPU-only mérés, proxyformátum** | **Előzetesen lezárva (István elfogadta, 2026-11-21).** Proxyszabvány (`proxy_profile` alapértéke): H.264 High, 1080p, yuv420p, zárt GOP ≈ 1 s, ~8 Mbps, AAC 128 kbps, MP4; CPU-only gépen a két verzió összehasonlításához 720p összehasonlító profil (REV-07). Hátravan: REV-05 A/V-szinkron és CFA4-MIN mérés a G4 előtt. Jegyzőkönyv: CFA4-BACKLOTHUB-V1-PROXY-MERES-2026-11-21.md |
| Megvalósításhoz | File Services FS-1, Media Services MS-1 (probe, bélyegkép, proxy-job), 3–5. lépcső |

## 2. Modell: asset → termék → verzió → reprezentáció

```
 production_entity (asset vagy snitt, G2-03)
   └─ product (pl. model/main, rig/main, comp/main)
        └─ version (v001, v002 …; publikálás után változtathatatlan)
             └─ representation (original, review_proxy, thumbnail …) ──► cfa4_files.file_object
```

- **Asset** = `production_entity` `asset` fajtájú típussal (karakter, kellék, díszlet, jármű, FX, környezet).
  Új tábla nem kell. Az assetkapcsolatok `entity_link`-ek: `part_of`, `variant_of`, `depends_on`; az asset ↔
  snitt/jelenet kapcsolat `casting`. Így az „assethierarchia” gráf, nem fa (G1).
- **Termék** bármely elemhez tartozhat (asset, snitt, jelenet), mert snittnek is van kimenete (plate, comp).
- **Verzió:** termékenként folytonos sorszám. Publikálás után a tartalma és az eredeti reprezentációi nem
  változhatnak; javítás csak új verzió.
- **Reprezentáció:** a verzió egy fájlformája. Az `original` a szerző anyaga (lezárt fájl). A származtatott
  (`review_proxy`, `thumbnail`, `waveform`) a Media Services terméke, újragenerálható.
- A File Servicesben **nincs verziófogalom** (ADR 0009): a verziót a BacklotHub tárolja, a fájlt a katalógus.

## 3. Adatmodell (DDL-vázlat, `backlot` séma)

```sql
CREATE TABLE backlot.product_type (
    id uuid PRIMARY KEY, code text NOT NULL UNIQUE,           -- model, rig, texture, plate, comp, edit, audio …
    label_key text NOT NULL, applies_to_kind text CHECK (applies_to_kind IN ('asset','structural')),
    retired_at timestamptz
);

CREATE TABLE backlot.product (
    id              uuid PRIMARY KEY,
    production_id   uuid NOT NULL REFERENCES backlot.production(id),
    entity_id       uuid NOT NULL REFERENCES backlot.production_entity(id) ON DELETE RESTRICT,
    product_type_id uuid NOT NULL REFERENCES backlot.product_type(id),
    name            text NOT NULL DEFAULT 'main' CHECK (name ~ '^[a-z0-9][a-z0-9_-]{0,31}$'),
    last_version    integer NOT NULL DEFAULT 0,      -- sorszámosztáshoz, sorzárral
    retired_at      timestamptz,
    ...közös oszlopok,
    UNIQUE (entity_id, product_type_id, name)
);

CREATE TABLE backlot.version (
    id              uuid PRIMARY KEY,
    production_id   uuid NOT NULL REFERENCES backlot.production(id),
    product_id      uuid NOT NULL REFERENCES backlot.product(id) ON DELETE RESTRICT,
    number          integer NOT NULL CHECK (number > 0),
    source_task_id  uuid REFERENCES backlot.task(id),
    author_person_id uuid NOT NULL REFERENCES backlot.person(id),
    note            text,
    status_id       uuid NOT NULL REFERENCES backlot.status(id),   -- version_default folyamat
    published_at    timestamptz,                   -- nem NULL = változtathatatlan
    frame_in integer, frame_out integer,           -- ha idő alapú (plate, comp), képkockában
    metadata        jsonb NOT NULL DEFAULT '{}',
    search_vector   tsvector GENERATED ALWAYS AS (…) STORED,   -- G2-07
    retired_at      timestamptz,
    ...közös oszlopok,
    UNIQUE (product_id, number)
);

CREATE TABLE backlot.representation (
    id              uuid PRIMARY KEY,
    version_id      uuid NOT NULL REFERENCES backlot.version(id) ON DELETE RESTRICT,
    role            text NOT NULL CHECK (role IN ('original','review_proxy','thumbnail','waveform','other')),
    name            text NOT NULL DEFAULT 'main',  -- több eredeti fájl (pl. textúrakészlet) megkülönböztetése
    file_object_id  uuid REFERENCES cfa4_files.file_object(id) ON DELETE RESTRICT,
    state           text NOT NULL CHECK (state IN ('pending','ready','failed')),
    probe           jsonb,                         -- Media Services probe: kodek, felbontás, fps, időtartam
    profile         text,                          -- származtatottnál: a proxy_profile kódja
    job_ref         text,                          -- Media Services job-azonosító
    error_code      text,
    ...közös oszlopok,
    UNIQUE (version_id, role, name)
);
```

**Kikényszerítve (trigger):**
- a sorszám a `product.last_version` sorzárával osztódik ki, így párhuzamos létrehozásnál sincs ütközés vagy
  lyuk;
- publikált verzión a tartalmi mezők és az `original` reprezentációk nem módosíthatók, nem törölhetők
  (`VERSION_IMMUTABLE`);
- publikáláskor minden `original` reprezentáció `ready`, és a fájlja a File Servicesben **lezárt** (`sealed`);
- a verzió, a termék és az elem ugyanabban a produkcióban van.

## 4. Verzió-folyamat (`version_default`, a 4. lépcső formátumában)

`draft` → `published` → `approved`, `changes_requested` vagy `rejected`, valamint `retired`. (A `changes_requested`
állapotot és átmenetet a [G2-13](CFA4-BACKLOTHUB-G2-13-REVIEW-APPROVAL-2026-11-21.md) 2. pontja pontosítja, 2026-11-21.)

- `publish`: őrfeltétel `all_originals_sealed`; a File Services `seal` és az állapotváltás egy
  műveletsorban (a Rust-adatút lezár, majd a katalógus, a verzió és az audit egy tranzakcióban).
- `approve` / `reject`: szerep (`producer`, `owner`), az elutasításhoz megjegyzés kell (G2-04). A Review (13.)
  ugyanezt a folyamatot használja a saját felületével (review-munkamenet, rajzolt megjegyzés).
- A „publikált” és a „jóváhagyott” állapot így egy helyen van (a verzió `status_id`-je), ahogy a G1 kérte.
- A forrásfeladat (`source_task_id`) átmenetét a verzió nem váltja automatikusan; az eseményvezérelt
  összekötés (pl. „publikáláskor a feladat `pending_review`”) a 12. lépcsőtől, szabályként.

## 5. Integrációs határok

| Határ | Irány | Szerződés |
|---|---|---|
| File Services (ADR 0009) | → | `files.import` (ellenőrzött másolás a `publish` gyökérbe), `files.seal`, `files.resolve`, `files.verify`, `files.delete`. Az út a BacklotHub sablonjából jön, a File Services csak a gyökéren belüliséget ellenőrzi |
| Media Services (ADR 0007, MS-1) | → | `probe` az eredetire; `thumbnail` és proxy-job a `review` gyökérbe; a job végén a reprezentáció `ready` vagy `failed` (hibakóddal). Proxy-job sosem blokkolja a publikálást |
| Structure (3.) | ← | Az asset és a snitt `production_entity`; `casting` és egyéb linkek |
| Task (5.) | ← | `source_task_id`; a feladat nézetében a verziói |
| Workflow (4.) | ↔ | `VersionAdapter`, `version_default` folyamat, predikátum: `all_originals_sealed`, `has_approved_version` (ez most kerül regisztrálásra, G2-04) |
| Search (7.) | → | `SearchProvider`: verzió (termék, szám, megjegyzés, szerző), asset |
| Publisher (10.), Loader (11.), Review (13.), Delivery (25.), Archive (26.) | → | A verzió és a reprezentáció az ő bemenetük |

**Útsablon (publikálás):** `{production.code}/{entity.path_codes}/{product_type}/{product.name}/v{number:03}/{file}`,
a `publish` gyökérben. Ütközés nem lehet, mert a verziószám egyedi; felülírás a File Servicesben amúgy is tilos.

## 6. Proxy és bélyegkép

- **`proxy_profile`** (produkciós beállítás, alapérték globálisan): kodek, felbontás, GOP, hangformátum.
  **Az alapértéket a V1 mérés döntötte el (István, 2026-11-21/22):** H.264 High, 1080p, yuv420p, zárt GOP ≈ 1 s,
  ~8 Mbps, AAC 128 kbps, MP4; CPU-only gépen a két verzió összehasonlításához 720p összehasonlító profil (REV-07).
  A REV-05 A/V-szinkron valódi hangeszközön a V2-ben (Review G5) dől el. A G4 előfeltétele a proxyszabvány
  szempontjából teljesült; a File Services FS-1, a Media Services MS-1 és a 3–5. lépcső megléte továbbra is kell.
- A proxy újragenerálható (pl. profilváltáskor): új `file_object`, a régi `deleted` állapotba kerül (a File
  Services kétlépcsős törlése), auditálva. Az `original` soha nem cserélhető.
- Bélyegkép: képkocka a verzió közepéről (vagy a megadott képkockáról), PNG, a `review` gyökérben.

## 7. Parancsok

| Parancs | Mit csinál |
|---|---|
| `backlot.asset.create` / `.update` / `.list` | Asset-típusú elem (a 3. lépcső parancsaira épül, asset-mezőkkel) |
| `backlot.product.create` / `.list` / `.retire` | Termék |
| `backlot.version.create` | Új `draft` verzió fájlokkal: `files.import` → reprezentációk → probe- és bélyegkép-job |
| `backlot.version.publish` / `.approve` / `.reject` / `.retire` | Folyamat-átmenetek (G2-04 generált parancsai) |
| `backlot.version.list` / `.get` / `.latest` | Lekérdezés: termékenként, elemenként, feladatonként; „utolsó jóváhagyott” |
| `backlot.representation.regenerate` | Származtatott reprezentáció újragenerálása |
| `backlot.version.verify` | A verzió fájljainak `files.verify`-a |

## 8. Universal Search (#44 újravizsgálata)

A G1 szerint itt kell újra megnézni, kell-e a CFA4 Universal Search terve. **Javaslat: továbbra sem.** A
BacklotHub az assetet és a verziót a saját `SearchProvider`-én adja (G2-07). Második fogyasztó (a szerkesztő
médiatára) a 11. lépcsőnél (Loader) jelenik meg, amikor a szerkesztő BacklotHub-verziót tölt be; ott a
`backlot.search.query` parancs elég. Közös keresőszolgáltatás akkor kell, ha a szerkesztőprojektek és a
médiatár is kereshetővé válnak a BacklotHubbal együtt; ez M2-kérdés. Újravizsgálat: a 11. lépcső G2-jén.

## 9. Hibakódok

`VERSION_IMMUTABLE`, `VERSION_NOT_SEALED`, `VERSION_NUMBER_CONFLICT`, `PRODUCT_EXISTS`,
`REPRESENTATION_FAILED`, `PROXY_PROFILE_UNSET`, valamint a File Services (`FILES_*`) és a Media Services
hibakódjai változatlanul továbbadva.

## 10. Tesztek (G5-re előre)

- Párhuzamos verziólétrehozás ugyanarra a termékre: folytonos, ütközésmentes sorszámok.
- Publikált verzió módosítási kísérlete minden úton (parancs, közvetlen SQL a `backlot_app` szereppel):
  elutasítva.
- Összeomlás publikálás közben (a lezárás után, a katalógus előtt): újraindításkor a verzió `draft`, a fájl
  lezárt és hivatkozás nélküli → a File Services jelzi, a publikálás megismételhető.
- Proxy-job hibája (sérült forrás, teli lemez): `failed` reprezentáció hibakóddal, a verzió használható marad.
- V1-hez kötött mérések a Media Services MS-1-ben (REV-01, -04, -05) a CFA4-MIN gépen.

## 11. Licenc (G3-ra előre)

Új Python-függőség nincs. AYON (FSL-1.1): csak a háromszintű minta ötlete, kód és séma nem. Stalker (LGPL):
verzió–feladat kapcsolat mint minta. Kitsu (AGPL): kimeneti és előnézeti fájl mint minta, kód nem.

## 12. G2-eredmény

**Javaslat: PASS a proxyformátum kivételével.** A `proxy_profile` alapértéke a V1 mérés után kerül be (külön
kiegészítésként ebbe a dokumentumba), és addig a megvalósítás nem indul. A Universal Search kérdése a 11.
lépcső G2-jére tolódik.
