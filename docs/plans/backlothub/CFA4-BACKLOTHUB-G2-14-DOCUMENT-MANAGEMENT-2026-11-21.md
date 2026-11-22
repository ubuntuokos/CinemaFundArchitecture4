# BacklotHub 14. lépcső – Document Management: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-21 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.20 · **Döntés:** #47 (M1-ben aláírás-követés, nem
aláírás a BacklotHubban; eldöntve 2026-11-08) · **Előzmény:**
[G1 Document Management](CFA4-BACKLOTHUB-G1-14-DOCUMENT-MANAGEMENT-2026-11-06.md) (funkcionális követelmények
jóváhagyva 2026-11-10) · **Épül rá:** G2-01 (Audit, személyes adat), G2-03 (kapcsolatok), G2-04 (Workflow Engine),
G2-07 (Search), G2-09 (útsablon), G2-12 (események), File Services (ADR 0009) · **Állapot:** G2-javaslat,
jóváhagyásra vár · implementáció nincs

Új döntési kérdés nincs. Új függőség az M1 alapban nincs; az aláírás-ellenőrzés (pyHanko, MIT) külön
allépcső (14-2), amely a G3 licenc- és függőségi kapuja után indul (9. pont).

## 1. Hatókör ebben a lépcsőben

- **Dokumentumtípusok** konfigurálható metaadatmezőkkel és jóváhagyási folyamattal.
- **Dokumentumok és verziók** (megváltoztathatatlan; új tartalom = új verzió), fájltárolás a File Servicesen át.
- **Kapcsolat produkciós objektumokkal** (jelenet, helyszín, szereplő, stábtag, asset, feladat, …).
- **Jóváhagyás** (verzióhoz kötve), **aláírás-követés** (#47 1. szint), **lejárat**.
- **Hozzáférés** szerep szerint (M1-ben a helyi felhasználó mindent lát, az adatmodell M2-re kész), **keresés**
  a címben, a metaadatban és a PDF szövegében, **PDF-előnézet**.
- **Nem része:** aláírás a BacklotHubban (eIDAS, kulcskezelés; külön döntéssel később), OCR (M1-ben nincs),
  dokumentumsablonból generálás és levelezés (call sheet, szerződéskészítés: a 23. lépcsőtől, ha igény lesz),
  e-mail csatolmány bevétele.

## 2. Integrációs határok

```
 cfa4-backlothub (Qt)  ──►  backlot.document.*  ──►  BacklotHub-szolgáltatás ──► PostgreSQL (backlot.*)
                                  │                          │ files.import / files.seal / files.verify
                                  │                          ▼
                                  │                 cfa4-cli serve (File Services, ADR 0009) → `attachment` gyökér
                                  └── PDF-előnézet és -szöveg: QtPdf (a felületen / a szolgáltatásban)
```

| Határ | Irány | Szerződés |
|---|---|---|
| File Services (ADR 0009) | → | `files.import` (ellenőrzött másolás az **`attachment`** gyökérbe), `files.seal`, `files.resolve`, `files.verify`, `files.delete`. A katalógus a tartalomkivonatot és a méretet őrzi; a dokumentum-verzió csak hivatkozik a `file_object`-re |
| Workflow (4.) | ↔ | `DocumentVersionAdapter`; két alapfolyamat (3. pont); az átmenetek generált parancsok |
| Structure (3.), Task (5.), később 18–20. | ← | Kapcsolt objektumok (4.2) |
| Útsablon (G2-09) | ← | A közös sablonmotor és mezőregiszter kap `document.*` mezőket (4.3) |
| Search (7.) | → | `SearchProvider` a dokumentumokra: cím, kód, metaadat, kinyert szöveg; láthatóság a lekérdezésben |
| Audit (1.) | → | Minden írás és letöltés/megnyitás-kérés; személyes mezők redaktálva (`personal_detail`) |
| Notification (12.) | → | `backlot.document.*` események; lejárat a meglévő határidő-jelzéssel (6. pont) |
| Publisher, Loader, Review | nincs | A dokumentum nem termék: nem publikálható és nem tölthető be a szerkesztőbe |

**Gyökér:** a dokumentumok az **`attachment`** szerepű gyökérbe kerülnek (ADR 0009, 5. pont: `work`, `publish`,
`review`, `attachment`, `archive`). Külön `document` gyökér nem kell, így az ADR nem változik; a feladat- és
elemcsatolmányok (#43) ugyanezt a gyökeret használják, különböző útsablonnal. A G1 mindkét lehetőséget említette;
ez az egyszerűbb.

## 3. Verzió-folyamat

A dokumentum **verziójához** kötődik az állapot (G1: a jóváhagyás konkrét verzióhoz kötődik). A típus
folyamatot választ a két alapfolyamatból (a G2-04 formátumában, migrációban szállítva):

| Folyamat | Állapotok | Mire |
|---|---|---|
| `document_filed` | `filed` (kezdő és végállapot) | Nem jóváhagyandó irat (call sheet, szkennelt számla, engedélymásolat) |
| `document_approval` | `submitted` → `approved` / `changes_requested` / `rejected` | Jóváhagyandó irat (szerződéstervezet, átadási specifikáció) |

- Átmenetek: `approve` (szerep: `producer`, `owner`), `request_changes` és `reject` (ugyanezek, megjegyzés
  kötelező, `WF_COMMENT_REQUIRED`). A szerepek a G2-04 szerint a `production_member`-ből jönnek (#42).
- A `changes_requested` és a `rejected` a verzióra nézve végállapot; a folytatás új verzió (ahogy a G2-13
  verzióinál). Aktiválás után a folyamatdefiníció változtathatatlan (új verzió), mint mindenhol.
- A dokumentum „aktuális” verziója a legmagasabb sorszámú, nem visszavont verzió. A „jóváhagyott aktuális
  verzió” a legmagasabb sorszámú `approved` verzió; ezt lekérdezéskor számoljuk, tárolt mutató nincs (a
  dokumentumnál nincs hero-fogalom, és a számolt érték nem tud eltérni).

## 4. Adatmodell (DDL-vázlat, `backlot` séma)

### 4.1 Típus, dokumentum, verzió

```sql
CREATE TABLE backlot.document_type (
    id uuid PRIMARY KEY, code text NOT NULL, version integer NOT NULL,
    label_key text NOT NULL,                            -- a felület fordítja (#16)
    state text NOT NULL CHECK (state IN ('draft','active','retired')),
    workflow_code text NOT NULL,                        -- 'document_filed' | 'document_approval'
    metadata_schema jsonb NOT NULL,                     -- zárt mezőlista (lent)
    expiry text NOT NULL CHECK (expiry IN ('none','optional','required')),
    expiry_warning_days integer NOT NULL DEFAULT 30,
    default_visibility text[] NOT NULL,                 -- szerepek: owner, producer, coordinator, artist, viewer
    contains_personal_data boolean NOT NULL DEFAULT false,   -- SEC-06
    production_id uuid REFERENCES backlot.production(id),    -- NULL: globális alaptípus
    ...közös oszlopok production_id nélkül,
    UNIQUE (code, version)
);

CREATE TABLE backlot.document (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    type_id uuid NOT NULL REFERENCES backlot.document_type(id) ON DELETE RESTRICT,
    code text NOT NULL,                                  -- produkción belül egyedi, a felhasználó adja vagy a felület javasolja
    title text NOT NULL,
    metadata jsonb NOT NULL DEFAULT '{}',                -- a típus sémája szerint validálva
    valid_from date, valid_until date,
    visibility text[],                                   -- NULL = a típus alapértéke; felülírását az owner és a producer végezheti
    last_version integer NOT NULL DEFAULT 0,             -- sorszámosztás sorzárral, mint a termékeknél (G2-08)
    search_vector tsvector GENERATED ALWAYS AS (…) STORED,
    retired_at timestamptz,
    anonymized_at timestamptz,                           -- SEC-06, 8. pont
    ...közös oszlopok,
    UNIQUE (production_id, code),
    CHECK (valid_until IS NULL OR valid_from IS NULL OR valid_until >= valid_from)
);

CREATE TABLE backlot.document_version (                  -- csak beszúrható; a tartalom lezárt
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    document_id uuid NOT NULL REFERENCES backlot.document(id) ON DELETE RESTRICT,
    number integer NOT NULL CHECK (number > 0),
    kind text NOT NULL CHECK (kind IN ('content','signed_copy')),
    file_object_id uuid NOT NULL REFERENCES cfa4_files.file_object(id) ON DELETE RESTRICT,
    original_name text NOT NULL,                         -- a feltöltött fájl neve (az út sablonból készül)
    mime text NOT NULL,
    uploaded_by_person_id uuid NOT NULL REFERENCES backlot.person(id),
    note text,
    status_id uuid NOT NULL REFERENCES backlot.status(id),   -- a típus folyamata szerint (G2-04: ez a hiteles állapot)
    text_state text NOT NULL DEFAULT 'pending' CHECK (text_state IN ('pending','extracted','none','failed')),
    extracted_text text,                                 -- legfeljebb 2 MiB, 4.4
    search_vector tsvector GENERATED ALWAYS AS (…) STORED,
    retired_at timestamptz,
    ...közös oszlopok,
    UNIQUE (document_id, number)
);
```

**A típus metaadatsémája** (`cfa4.backlot.document_metadata/1`, zárt mezőtípusokkal):

```json
{"schema": "cfa4.backlot.document_metadata/1",
 "fields": [
   {"key": "parties",      "type": "organization_ref", "multiple": true, "required": true},
   {"key": "signing_date", "type": "date"},
   {"key": "fee",          "type": "money"},
   {"key": "talent",       "type": "person_ref", "personal": true},
   {"key": "territory",    "type": "text", "max_length": 200},
   {"key": "rights_kind",  "type": "enum", "values": ["music","archive","location","likeness","other"]}]}
```

- Mezőtípusok: `text`, `date`, `boolean`, `enum`, `person_ref`, `organization_ref`, `money`. Más nincs.
- **`money`:** `{"amount": "12500.00", "currency": "EUR"}`, az összeg tizedes karakterlánc (lebegőpontos szám
  nincs), a pénznem ISO 4217 (#15, `CURRENCY_UNKNOWN`). Átváltást a dokumentum nem tárol; megjelenítéskor a
  napi árfolyam (#29) használható, dátummal jelölve.
- **`personal: true`** mezőnél az audit a változás értékét redaktálja (G2-01: `{"redacted": true, "digest": …}`,
  a tényleges érték a `personal_detail`-ben). A `contains_personal_data` típusnál a **fájlnév és a kinyert szöveg
  sem kerül az auditba**, csak az azonosítók.
- Aktiváláskor a séma validált (kulcsok egyediek, a `enum` nem üres, ismert típusok); aktiválás után
  változtathatatlan, módosítás új verzió (mint a `load_profile`, G2-11). Meglévő dokumentum a régi típusverzión
  marad; áttérés külön parancs, séma szerint újravalidálva.
- Alaptípusok (migrációban, a produkció felülírhatja): `contract`, `release_form`, `permit`, `insurance`,
  `script_revision`, `call_sheet`, `delivery_spec`, `other`.

### 4.2 Kapcsolatok

```sql
CREATE TABLE backlot.document_link (
    document_id uuid NOT NULL REFERENCES backlot.document(id),
    production_id uuid NOT NULL REFERENCES backlot.production(id),
    subject_kind text NOT NULL,                          -- regisztrált, zárt lista
    subject_id uuid NOT NULL,
    relation text NOT NULL DEFAULT 'about' CHECK (relation ~ '^[a-z][a-z0-9_]*$'),  -- about, permit_for, release_of …
    ...közös oszlopok,
    PRIMARY KEY (document_id, subject_kind, subject_id, relation)
);
CREATE INDEX ON backlot.document_link (subject_kind, subject_id);
```

- **A `subject_kind` zárt, kódban regisztrált lista**, ugyanúgy, mint a közös `comment` táblánál (G2-05). Az
  adatbázis nem tud idegen kulcsot ellenőrizni polimorf hivatkozásra, ezért **a regisztrált feloldó**
  ellenőrzi íráskor (létezik-e, ugyanabban a produkcióban van-e); a produkció törlésekor/visszavonásakor a
  kapcsolat a dokumentumon megmarad, a felület „nem elérhető objektum”-ként jelzi.
- Az alap lépcsőben regisztrálva: `production_entity` (jelenet, snitt, asset, …, G2-03), `task` (5.), `product`
  és `version` (8.), `organization`, `person` (2., 5.).
- **A helyszín (20.), a szereplő (19.) és a stábtag (18.) kapcsolata** akkor regisztrálódik, amikor az adott
  lépcső elkészül; addig ismeretlen típusra `DOCUMENT_LINK_KIND_UNKNOWN`, és nem készül helyettesítő tábla. A
  szerződés és az engedély addig a metaadat `person_ref`/`organization_ref` mezőjén át köthető.
- **Visszafelé:** minden modul a saját nézetében listázhatja a hozzá kapcsolt dokumentumokat
  (`backlot.document.list {linked_to: {kind, id}}`), a láthatósági szabály szerint szűrve.

### 4.3 Út és fájl

- **Útsablon** a közös motorral (G2-09, `path_template`, alapsablon migrációban):
  `{production.code}/documents/{document.type}/{document.code}/v{document.version:03}/{document.file_name}`.
  A mezőregiszter ezzel `document.type`, `document.code`, `document.version`, `document.file_name` mezőt kap
  (egy motor, egy mezőregiszter; a bővítés a G2-09 szerinti, kódban regisztrált).
- **Fájlnév:** a feltöltött név a File Services ellenőrzésén át kerül az útba (`..`, vezérlőkarakter,
  túl hosszú név elutasítva: `DOCUMENT_FILE_NAME_INVALID`); az eredeti név az `original_name`-ben marad.
- **Méretkorlát:** beállítható (alapból 512 MiB) a File Services lemezellenőrzésén felül; túllépés:
  `DOCUMENT_TOO_LARGE`.
- **Verzió felvétele** (`backlot.document.add_version`): `files.import` az `attachment` gyökérbe → `files.seal`
  (a tartalom változtathatatlan) → egy tranzakcióban a `document_version`, a `last_version` (sorzárral), a
  `status_id` kezdőállapota, az audit és az outbox-sor (G2-12). Ha a tranzakció elbukik, a lezárt fájl `deleted`
  állapotba kerül (a Publisher mintája, G2-10), felhasználói tisztítás nélkül nem purge-ölődik.
- **Törlés** nincs: a dokumentum és a verzió **visszavonható** (`retired_at`, visszaállítható); a fizikai
  törlés (`purged`) a File Services szabálya szerint csak a felhasználó kifejezett jóváhagyásával, és lezárt
  fájlra hivatkozó élő verzió esetén tiltott (ADR 0009). SEC-06 szerinti törlés/anonimizálás: 8. pont.
- **Letöltés és megnyitás:** `files.resolve` a helyi útra; `xdg-open` vagy PDF-előnézet. A megnyitás-kérés
  auditált (`document.opened`), mert szerződéseknél ez számít.

### 4.4 Szöveg, keresés, előnézet

- **PDF-szövegkinyerés: Qt PDF (`QtPdf`, `QPdfDocument`)**, nincs új függőség. A G1 a G2-re hagyta az eszköz
  kiválasztását; a Qt PDF a felületi csomaggal (PySide6) eleve jön, és a kinyerésen kívül az előnézetet is adja.
  Ellenőrzendő a G3-on, hogy a rögzített PySide6-kiadásban a `QtPdf` és az oldalankénti szövegkinyerés elérhető
  (Qt 6.6-tól `getAllText`; a referenciagépen Qt 6.10).
- A kinyerés **a verzió felvétele után, háttérben** fut (`text_state = 'pending'` → `extracted` / `none` /
  `failed`), nem akadályozza a feltöltést. Szöveg nélküli PDF (szkennelt kép) → `none` (OCR M1-ben nincs).
  Hibás vagy jelszavas PDF → `failed`, kezelt hiba, a dokumentum használható marad.
- **Korlát:** legfeljebb 2 MiB kinyert szöveg verziónként (a többi csonkolva, jelezve); szabad szöveg csak PDF-ből
  és sima szövegfájlból (`text/plain`, `text/markdown`). Más formátum (docx, xlsx, …) címmel és metaadattal
  kereshető, szövege nem; a felvételkor a felület ezt jelzi.
- **Keresés:** a G2-07 szerint generált `tsvector` (`backlot_hu`, `backlot_en`, `backlot_code`), súlyok: kód
  `A`, cím `B`, metaadat-szöveg `C`, kinyert szöveg `D`. A `SearchProvider` típusa `document`, a találat a
  verzióra nem, a dokumentumra mutat; a kiemelt részlet (`ts_headline`) a kinyert szövegből is jöhet.
- **Láthatóság a lekérdezésben:** a keresés és a lista csak azt adja vissza, amihez a felhasználónak szerepe
  van (5. pont). A rejtett dokumentum címe, kódja és kiemelt részlete **sem** jelenik meg (nincs „létezik, de
  nem láthatod” szivárgás).
- **Előnézet:** `QPdfView` a felületen, helyi úton, csak olvasásra; nem PDF fájlnál „megnyitás alkalmazásban”.

## 5. Hozzáférés

- **Szabály:** egy dokumentumot az lát, akinek a produkcióban olyan szerepe van, amely szerepel a dokumentum
  `visibility` listájában (ha NULL, a típus `default_visibility` listájában). A `contains_personal_data` típusok
  alapértéke szűkített (`owner`, `producer`); a szerződéseké is.
- **Írás és verziófelvétel:** `owner`, `producer`, `coordinator`; jóváhagyás: a folyamat szerepei (3. pont).
  A `visibility` listát csak az `owner` és a `producer` módosíthatja; a változás auditált.
- **M1-ben** a helyi felhasználó `owner` (#42), mindent lát; a szabályokat a szolgáltatásréteg és a lekérdezések
  már most alkalmazzák, hogy M2-ben ne kelljen átírni. Tesztek a más szerepű felhasználóra (11. pont).
- **Fájlszinten** a File Services nem végez szerepellenőrzést (egy felhasználós gyökér, OS-jogok); a
  szerep szerinti elrejtés a BacklotHub rétegében van, és **M1-ben nem védi a fájlt** attól, aki a lemezhez
  hozzáfér. Ezt a felület a típusbeállításnál kimondja; a védelem a fájlrendszer jogaira és a titkosított
  kötetre épül (ADR 0009, titkosítás).

## 6. Aláírás-követés és lejárat

### 6.1 Aláírás-követés (#47 1. szint)

```sql
CREATE TABLE backlot.document_signer (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    document_id uuid NOT NULL REFERENCES backlot.document(id),
    party_kind text NOT NULL CHECK (party_kind IN ('person','organization','external')),
    person_id uuid REFERENCES backlot.person(id),
    organization_id uuid REFERENCES backlot.organization(id),
    display_name text NOT NULL,                          -- külső félnél a név; személyes adat, ha person/external
    role_label text,                                     -- pl. „producer”, „előadó”, „tanú”
    required boolean NOT NULL DEFAULT true,
    signed_at timestamptz,
    signed_in_version_id uuid REFERENCES backlot.document_version(id),   -- az aláírt példányt hordozó verzió
    method text CHECK (method IN ('paper_scan','digital_pdf','external_service','other')),
    verification jsonb,                                  -- 9. pont (14-2); NULL, amíg nincs ellenőrzés
    ...közös oszlopok,
    CHECK ((signed_at IS NULL) = (signed_in_version_id IS NULL))
);
```

- **Állapot lekérdezéskor számolva** (tárolt jelző nincs, így nem térhet el):
  - `not_required`: nincs `required` aláíró;
  - `awaiting`: van kötelező aláíró, és egy sem írt alá;
  - `partial`: néhány kötelező aláíró aláírt;
  - `signed`: minden kötelező aláíró aláírt;
  - `expired`: lejárt (`valid_until` elmúlt) és a dokumentum nincs visszavonva; a felület a `signed` mellett
    mutatja, külön szűrhető.
- **Az aláírt példány új verzióként kerül fel** (`kind = 'signed_copy'`): szkennelt vagy digitálisan aláírt PDF.
  A rögzítéskor a felhasználó megadja, ki(k) írt(ak) alá, mikor, és melyik módon (`backlot.document.sign.record`).
  **Egy aláírás csak azokra a tartalmi verziókra számít, amelyek után nem érkezett új `content` verzió:**
  ha a tartalom megváltozik (új `content` verzió), a korábbi aláírások nem érvényesek az újra, az állapot
  visszaesik (`awaiting`/`partial`), és a felület kiírja, miért. Az aláírt példány (`signed_copy`) nem változtat
  a tartalmon.
- Az aláírás-rögzítés **tény rögzítése**, nem aláírás: a BacklotHub nem tanúsít jogi érvényességet. Ezt a
  felület és a jelentések is kimondják.
- A kötelező aláírók listája a dokumentum felvételekor az alaptípus sablonjából jön (típusonként
  megadható), és módosítható; módosítása auditált.

### 6.2 Lejárat

- A `valid_until` (és az opcionális `valid_from`) dokumentumszintű. A típus `expiry` beállítása szerint
  kötelező, megadható vagy nincs.
- **Figyelmeztetés:** a G2-12 időzítője (percenként) a `valid_until` mezőt is kiértékeli, a típus
  `expiry_warning_days` előidejével: `backlot.deadline.approaching` és `backlot.deadline.missed` esemény egyszer
  dokumentumonként és `valid_until`-értékenként (`deadline_key = valid_until:<dátum>`, ugyanaz a
  `deadline_signal` tábla, `subject_type = 'document'`). Új tábla és új eseménytípus nincs: ez a G2-12 6.1 pontjának
  bővítése (időzítő forrása), amely a megvalósításkor kerül oda. A dátum módosítása új jelzést ad.
- **Lekérdezés EF nélkül:** a `backlot.document.expiring {within_days}` lista az Event Fabric nélkül is működik
  (az események csak az értesítést adják). A Dashboard lekérdezhet rá (`documents.expiring`, G2-06 mintája, külön
  csempe a 6. lépcső megvalósításakor, ha igény lesz; most nincs).
- A lejárat **nem törli** és nem rejti el a dokumentumot; csak jelzi.

## 7. Parancsok és események

| Parancs | Mit csinál |
|---|---|
| `backlot.document_type.*` | Típusok (létrehozás, aktiválás, új verzió, visszavonás); csak `owner` |
| `backlot.document.create` / `.update` / `.retire` / `.restore` / `.list` / `.get` | Dokumentumok (metaadat, érvényesség, láthatóság) |
| `backlot.document.add_version` | Fájl feltöltése új verzióként (`kind = content`) |
| `backlot.document.version.list` / `.open` / `.verify` | Verziók; megnyitás (auditált); épség (`files.verify`) |
| `backlot.document.version.approve` / `.request_changes` / `.reject` | Generált átmenetparancsok (G2-04) |
| `backlot.document.link.add` / `.remove` / `.list` | Kapcsolatok |
| `backlot.document.signer.set` / `.list` | Kötelező aláírók |
| `backlot.document.sign.record` / `.status` | Aláírás rögzítése (aláírt példány verzióként); állapot |
| `backlot.document.expiring` | Lejáró dokumentumok |
| `backlot.document.extract_text` | Szövegkinyerés újrafuttatása (`failed` után) |
| `backlot.document.anonymize` | SEC-06 (8. pont) |

**Események** (a G2-12 `backlot` domainjében, `classification: internal`, outbox, mind kritikus):
`backlot.document.version_added`, `backlot.document.version_approved`, `backlot.document.version_changes_requested`,
`backlot.document.version_rejected`, `backlot.document.signature_recorded`. A lejárat a `backlot.deadline.*`
eseményeken megy (6.2). A payload csak azonosítókat, a típuskódot és a verziószámot viszi, **címet és személyes
adatot nem**; a fogyasztó az adatbázisból olvassa a megjelenítendőt a láthatósági szabály szerint.

Hibakódok: `DOCUMENT_TYPE_INVALID`, `DOCUMENT_METADATA_INVALID`, `DOCUMENT_CODE_TAKEN`,
`DOCUMENT_FILE_NAME_INVALID`, `DOCUMENT_TOO_LARGE`, `DOCUMENT_LINK_KIND_UNKNOWN`, `DOCUMENT_NOT_VISIBLE`
(a felület ugyanúgy „nem található”-ként mutatja), `DOCUMENT_SIGNER_INVALID`, `DOCUMENT_TEXT_EXTRACTION_FAILED`,
`DOCUMENT_EXPIRY_REQUIRED`, valamint a `WF_*`, `FILES_*` és `CURRENCY_UNKNOWN` kódok változatlanul.

## 8. Személyes adat és törlés (SEC-06)

- A `personal: true` mezők, az aláíróneveket és a `contains_personal_data` típusok fájljait az audit nem
  tartalmazza értékként (G2-01 redaktálás), így a törlési/anonimizálási kérés az **auditlánc épségének
  megőrzésével** teljesíthető.
- **`backlot.document.anonymize`:** a dokumentum személyes metaadatmezőit, az aláíró `display_name` mezőit és a
  kinyert szöveget törli/átírja (`personal_detail` megfelelő sorai törlődnek), a `document_version` fájlt a
  File Services két lépcsős törlésével (`deleted` → `purged`) a felhasználó kifejezett jóváhagyásával
  távolítja el (ADR 0009, #18 elve), és az `anonymized_at` mezőt beállítja. Az audit egy `document.anonymized` eseményt kap, tartalom nélkül.
- Lezárt fájlra hivatkozó élő verziónál a purge tiltott (ADR 0009): a parancs előbb visszavonja a verziót,
  kiírja az érintett hivatkozásokat, és csak megerősítés után folytatja.
- A felület a `contains_personal_data` dokumentumoknál nem tesz ki címet vagy előnézetet az értesítésekbe és a
  keresési javaslatokba, csak a megnyitott nézetben.

## 9. Aláírás-ellenőrzés (14-2, pyHanko)

- A #47 szerint PDF-aláírás **épsége** ellenőrizhető **pyHankóval (MIT)**. Ez **új Python-függőség**
  (a tranzitív függőségeivel, `--require-hashes` szerint rögzítve), ezért **külön allépcső (14-2)**, amely a G3
  licenc- és függőségi kapuja után indul. Az M1 alap (14-1: típusok, verziók, kapcsolatok, jóváhagyás,
  aláírás-követés, lejárat, keresés, előnézet) pyHanko nélkül teljes; az ellenőrzés gombja addig **nincs a
  felületen** (nem stub).
- **Eredmény** (`verification`, `cfa4.backlot.signature_check/1`): aláírásmezőnként a megjelölt aláíró neve, a
  **bejelentett** aláírási idő, `intact` (az aláírt bájtok nem módosultak), `covers_whole_document` (van-e az
  aláírás után hozzáfűzött tartalom), és `trust` (`unknown`, `trusted`, `untrusted`). Alapból **nincs
  megbízható gyökértanúsítvány**, ezért a `trust` `unknown`; a felhasználó adhat hozzá saját gyökereket, az
  operációs rendszer tanúsítványtárát automatikusan nem használjuk.
- Az ellenőrzés **tájékoztató** (nem eIDAS-szerinti minősítés, nincs időbélyeg-ellenőrzés vagy
  visszavonás-ellenőrzés hálózaton át, SEC-02). Nem aláírt vagy nem PDF fájlon: `not_applicable`, kezelt
  hiba nélkül. Hibás vagy rosszindulatú PDF: kezelt hiba (`DOCUMENT_SIGNATURE_CHECK_FAILED`), a folyamat
  nem omlik össze (a pyHanko külön folyamatban, időkorláttal fut).

## 10. Sorrend és függés

| Rész | Kell hozzá |
|---|---|
| 14-1: típus, dokumentum, verzió, kapcsolat, jóváhagyás, aláírás-követés, lejárat-lekérdezés, keresés, előnézet | **File Services FS-1** (`attachment` gyökér, `import`, `seal`), 2., 3., 4., 5., 7. lépcső |
| Lejárati és más értesítések | 12. lépcső, EF-R1 |
| Kapcsolat szereplőhöz, stábtaghoz, helyszínhez | 18., 19., 20. lépcső (feloldó regisztrálása, 4.2) |
| 14-2: aláírás-ellenőrzés | G3 (pyHanko, hash-rögzített függőségek) |

## 11. Tesztek (G5-re előre)

- **Verziózás:** párhuzamos `add_version` ugyanarra a dokumentumra: folytonos, ütközésmentes sorszám; a
  lezárt fájl módosítása sikertelen (`FILES_SEALED_MODIFIED`); a verzió tartalma `files.verify`-val ép.
- **Hibainjektálás** a `files.import` és a tranzakció között: nincs félkész verzió, a lezárt fájl `deleted`,
  nincs árva `document_version`.
- **Metaadat:** séma-fuzz (ismeretlen típus, hiányzó kötelező mező, rossz pénznem, túl hosszú szöveg, tizedes
  összeg lebegőpontos kerekítés nélkül); a `personal` mező a naplóban redaktált.
- **Aláírás-állapot:** minden átmenet (`not_required` → `awaiting` → `partial` → `signed`), új tartalmi verzió
  után visszaesés, aláírt példány nem ront; `expired` jelzés.
- **Láthatóság:** más szerepű (nem M1) felhasználó nem látja a dokumentumot a listában, a keresésben, a
  kiemelt részletben, az értesítésben; az `anonymize` után nincs személyes érték sem az adatbázisban, sem a
  keresési vektorban; az auditlánc ellenőrzése (`audit.verify`) sikeres.
- **Szövegkinyerés:** szöveges PDF, szkennelt (kép) PDF, jelszavas és sérült PDF, 2 MiB feletti szöveg
  (csonkolás jelezve); a feltöltés mindegyiknél sikeres. Keresés magyar ragozott és ékezet nélküli szóra.
- **Kapcsolatok:** ismeretlen `subject_kind` → `DOCUMENT_LINK_KIND_UNKNOWN`; más produkció objektuma
  elutasítva; a kapcsolt objektum visszavonása után a dokumentum megmarad.
- **Lejárat:** óraugrás, a `valid_until` módosítása, leállás az előidő átlépésekor; a jelzés egyszer megy
  (G2-12 6.1 tesztjei).
- **Never crash:** nem elérhető `attachment` gyökér (`FILES_ROOT_UNAVAILABLE`), teli lemez, sérült fájl a
  megnyitáskor, a File Services leállása feltöltés közben: kezelt hiba, a katalógus nem mutat félkész fájlra.
- **Felületi ellenőrzés:** `ui.screenshot` a dokumentumlistáról és a PDF-előnézetről.
- **Mérés** a CFA4-MIN gépen: 10 000 dokumentum, 50 000 verzió, keresés a kinyert szövegben (a G2-07 S és L
  profiljával).

## 12. Licenc (G3-ra előre)

- **Új függőség 14-1-ben nincs.** Qt PDF (`QtPdf`) a PySide6 része (LGPL-3.0; a G3-on ellenőrizendő, hogy a
  rögzített csomagkészletben benne van és az oldalankénti szövegkinyerés elérhető). A PostgreSQL-oldali
  eszközök (`unaccent`, `pg_trgm`) contrib.
- **14-2:** pyHanko (MIT) és tranzitív függőségei (a G3-on feltérképezendő: licenc, `--require-hashes`,
  méret, a Python 3.14 támogatás).
- **Referencia, kód nélkül:** Mayan EDMS (**GPL-2.0-only**, nem kompatibilis a CFA4 GPL-3.0-or-later
  licencével, kód nem vehető át), paperless-ngx (GPL-3.0; dokumentumtípus, címkék, szövegkinyerés és
  -keresés mintája, kód nem). Új, külső szolgáltatás nincs (SEC-02).

## 13. G2-eredmény

**Javaslat: PASS.** Új döntési kérdés nincs. A megvalósítás a File Services FS-1 fázisa után indulhat; a pyHanko
alapú ellenőrzés külön allépcső a G3 után. A G2-12 időzítője a dokumentum lejáratával bővül (6.2), a G2-09
mezőregisztere a `document.*` mezőkkel (4.3), a G2-12 eseménykatalógusa a 7. pont eseményeivel; mindhárom
kiegészítés megvalósításkor kerül a megfelelő lépcsőbe, új tábla nélkül.
