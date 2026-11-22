# BacklotHub 20. lépcső – Location Management: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-21 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.18 · **Előzmény:**
[G1 Location Management](CFA4-BACKLOTHUB-G1-20-LOCATION-MANAGEMENT-2026-11-07.md) (funkcionális követelmények
jóváhagyva 2026-11-10) · **Épül rá:** G2-03 (forgatókönyvi helyszín mint elem), G2-05 (közös `subject_kind`
regiszter, `person`), G2-14 (engedély mint dokumentum), G2-17 (forgatási nap, rétegek), G2-18 (kapcsolattartó mint
`person`, személyes adat), File Services (ADR 0009), Media Services (ADR 0007) · **Állapot:** G2-javaslat,
jóváhagyásra vár · implementáció nincs

Új döntési kérdés nincs, új függőség nincs. **Két eltérés a G1-től**, mindkettő indokolva és a szövegben jelölve:
a valós helyszín **nem `resource`** (2.1), és a térkép/geokódolás **M1-ben nincs** (7. pont).

## 1. Hatókör ebben a lépcsőben

- **Forgatókönyvi helyszín** (a történet része) és **valós helyszín** (ahol forgatnak) két külön fogalomként, a
  köztük lévő kapcsolattal (jelölt, kiválasztott, elvetett).
- **Helyszínadatlap:** cím, koordináta, műszaki adatok, megközelítés, kapcsolattartók.
- **Fotók és dokumentumok** (engedély, szerződés, biztosítás), a File Services és a G2-14 szerint.
- **Forgatási korlátozások, elérhetőségi ablakok és foglalások**, ütközésjelzéssel.
- **Jelenetkapcsolat:** melyik jelenet melyik forgatókönyvi helyszínen játszódik, és melyik valós helyszín
  jeleníti meg.
- **Nem része:** térkép és címkeresés (7. pont), a forgatási terv és a jelenetek időbeosztása (23.), a helyszínbérleti
  díj és költség (24.), automatikus helyszínkinyerés a forgatókönyvből (Script Breakdown, 22.; kézi felvitel itt is
  lehetséges).

## 2. Integrációs határok

| Határ | Irány | Szerződés |
|---|---|---|
| Structure (3.) | ← | A **forgatókönyvi helyszín** `set` típusú `production_entity` (asset-fajta, nincs szülője); a jelenet–helyszín kapcsolat az `entity_link` `setting` típusa (3.2) |
| Person (5.), Crew (18.) | ← | A **kapcsolattartó** a közös `person` (tulajdonos, gondnok, helyszínügynök), a személyes adata a `person_personal_detail`-ben, a G2-18 szintjeivel és törlési szabályaival |
| Organization (2.) | ← | A kapcsolattartó lehet szervezet is (`organization_id`) |
| Document (14.) | ← | Az **engedély**, a szerződés és a biztosítás dokumentum, a `location` tárgyhoz kötve (`subject_kind = 'location'`), lejárattal |
| File Services (ADR 0009) | → | A fotók az `attachment` gyökérbe (`files.import`, `files.seal`, `files.resolve`, `files.verify`) |
| Media Services (ADR 0007, MS-1) | → | A bélyegkép (`thumbnail` job a `review` gyökérbe, a G2-08 mintájára); EXIF-adat **nem** olvasódik ki (5.3) |
| Calendar (17.) | ↔ | A foglalás új rétege (`location_bookings`); a `forgatási nap` a naptár táblája; az esemény kapcsolt objektuma lehet a helyszín |
| Search (7.) | → | `SearchProvider`: valós helyszín (név, kód, cím, típus, megjegyzés) |
| Audit (1.) | → | Minden írás auditált |
| Shooting Schedule (23.) | később | A forgatási korlátozásokat és foglalásokat a jelenet-időbeosztásnál értékeli; itt csak a foglalásra számolódnak |
| Script Breakdown (22.) | később | A forgatókönyvi helyszíneket és a `setting` kapcsolatokat ugyanezekkel a parancsokkal tölti |
| Budget (24.) | később | A helyszínbérlet költsége ott él, nem itt |

### 2.1 A valós helyszín nem erőforrás (eltérés a G1-től)

A G1 a valós helyszínt a Resource Planning erőforrásaként említette. A G2-15 **erőforrása személy-kapacitás**
(perc/nap, munkanaptár, terhelés), a helyszínnek nincs kapacitása, csak **elérhetősége** (ablakok) és **foglalásai**.
Az erőforrás-típus kényszerített bővítése üres kapacitásmezőket, értelmetlen terhelésszámítást és a `resource.kind`
listán hívó nélküli elágazást hozna. Ezért a helyszín saját, egyszerű elérhetőség- és foglalásmodellt kap (4.3), amely
**ugyanazt az ütközésjelzési elvet** használja (számolt, nem tilt). A G2-15 szövege nem változik: az `resource.kind`
lista továbbra is csak a személyt (és a 21. lépcsőtől az eszközt) kezeli.

### 2.2 Produkcióhoz kötött helyszín (nincs közös helyszínkönyvtár)

A valós helyszín **produkciónként** külön sor (`production_id`). A helyszínek produkciók közti megosztása közös,
módosítható állapotot és adatszivárgást jelentene (egy magánlakás címe, a kapcsolattartó, az engedélyek más
produkciókhoz tartoznak). Újrahasználatra a **`backlot.location.copy_to_production`** parancs szolgál (4.5): az adatlapot
másolja, a produkcióspecifikus részeket (foglalás, engedély, elérhetőség) soha. Helyszínkönyvtár M2-ben, külön
döntéssel jöhet.

## 3. Forgatókönyvi helyszín és jelenetkapcsolat

### 3.1 A forgatókönyvi helyszín

A G2-03 `set` típusú elemét használja (kód, név, leírás, állapot, metaadat). A **belső/külső** jelölés és a **napszak**
**nem a `set` tulajdonsága**, hanem a **jeleneté**: ugyanaz a konyha egyszer `INT. KONYHA - NAPPAL`, máskor `INT. KONYHA -
ÉJJEL` jelenetben szerepel. Ezért a két érték a **jelenet** (`scene`) elem `metadata` mezőjében él
(`cfa4.backlot.scene_meta/1`, [G2-22](CFA4-BACKLOTHUB-G2-22-SCRIPT-BREAKDOWN-2026-11-21.md) 5. pont); a `set` metaadata
itt üres marad, új tábla nincs. *(Pontosítás 2026-11-21: a korábbi szöveg a `set` metaadatába tette; a G2-22
írásakor derült ki, hogy ez hibás.)* A létrehozás `backlot.structure.entity.create`-tel vagy a kényelmi
`backlot.location.set.create`-tel (ez az entitás-létrehozót hívja).

### 3.2 A `setting` kapcsolat (jelenet ↔ forgatókönyvi helyszín)

A G2-03 `entity_link` táblájának `setting` típusa, a `casting` mintájára (G2-19 3.1): `from_id` = **jelenet, snitt
vagy szekvencia**, `to_id` = **`set` típusú elem** („ebben a helyszínen játszódik”). A szolgáltatás ellenőrzi a
típuspárt (`LOCATION_LINK_INVALID`), a G2-03 `UNIQUE (from_id, to_id, link_type)` szabálya a duplikációt nem engedi.
Egy jelenetnek több `set`-je is lehet (jelenetváltó mozgás), egy `set` több jelenetben szerepelhet. Saját parancsa
`backlot.location.scene_link.add` / `.remove` / `.list`, amely az entitás-kapcsolat parancsát hívja; a Script Breakdown
(22.) ugyanezt használja. A G2-03 `entity_link` szabályai közé ez a pont bekerül.

## 4. Adatmodell (DDL-vázlat, `backlot` séma)

### 4.1 Valós helyszín, kapcsolattartó, kapcsolat a forgatókönyvi helyszínnel

```sql
CREATE TABLE backlot.location (                           -- a valós helyszín
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    code text NOT NULL CHECK (length(code) BETWEEN 1 AND 40),
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 300),
    kind text NOT NULL CHECK (kind IN ('apartment','house','street','studio','nature','office','industrial','public','other')),
    address jsonb,                                         -- {street, city, postal_code, country}, sémával
    latitude numeric(9,6) CHECK (latitude BETWEEN -90 AND 90),
    longitude numeric(9,6) CHECK (longitude BETWEEN -180 AND 180),
    directions text CHECK (length(directions) <= 5000),    -- megközelítés
    tech jsonb NOT NULL DEFAULT '{}',                      -- cfa4.backlot.location_tech/1, zárt mezőkkel (lent)
    description text CHECK (length(description) <= 20000),
    search_vector tsvector GENERATED ALWAYS AS (…) STORED, -- G2-07
    retired_at timestamptz,
    ...közös oszlopok,
    UNIQUE (production_id, code),
    CHECK ((latitude IS NULL) = (longitude IS NULL))
);

CREATE TABLE backlot.location_contact (                    -- kapcsolattartó: person vagy szervezet
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    location_id uuid NOT NULL REFERENCES backlot.location(id),
    person_id uuid REFERENCES backlot.person(id),
    organization_id uuid REFERENCES backlot.organization(id),
    role text NOT NULL CHECK (role IN ('owner','manager','agent','neighbor','authority','other')),
    note text CHECK (length(note) <= 500),
    retired_at timestamptz,
    ...közös oszlopok,
    CHECK ((person_id IS NULL) <> (organization_id IS NULL)),
    UNIQUE NULLS NOT DISTINCT (location_id, person_id, organization_id, role)
);

CREATE TABLE backlot.location_set_link (                   -- valós helyszín ↔ forgatókönyvi helyszín
    location_id uuid NOT NULL REFERENCES backlot.location(id),
    set_entity_id uuid NOT NULL REFERENCES backlot.production_entity(id),   -- trigger: set típus, ugyanaz a produkció
    production_id uuid NOT NULL REFERENCES backlot.production(id),
    state text NOT NULL CHECK (state IN ('candidate','selected','rejected')),
    note text CHECK (length(note) <= 500),
    ...közös oszlopok,
    PRIMARY KEY (location_id, set_entity_id)
);
```

- **Több–több kapcsolat** (G1): egy forgatókönyvi helyszínt több valós helyszín is megjeleníthet (pl. belső a
  stúdióban, külső a házon), és egy valós helyszín több forgatókönyvi helyszínt is. Egy valós helyszín **több
  `selected`** kapcsolatot is hordozhat, és egy `set`-nek is lehet több `selected` valós helyszíne; a felület
  jelzi, ha egy `set`-nek **nincs** `selected` helyszíne, de jelenetben szerepel (`set_without_location`, információ).
- **A műszaki adatok** (`tech`, `cfa4.backlot.location_tech/1`, zárt mezőkkel, JSON Schema-val validálva):

```json
{"schema": "cfa4.backlot.location_tech/1",
 "power":         {"available": true, "note": "32 A, a pincében"},
 "parking":       {"spaces": 6, "note": "utcán, engedély kell"},
 "restroom":      true,
 "accessibility": {"step_free": false, "note": "2. emelet, lift nincs"},
 "noise_note":    "forgalmas út, a nap folyamán zajos",
 "nearest_hospital": {"name": "Városi Kórház", "address": "…", "distance_note": "8 perc autóval"}}
```

  Minden mező opcionális; ismeretlen kulcs `LOCATION_TECH_INVALID`.
- **Kapcsolattartó = `person`**: a név, telefon, e-mail a `person_personal_detail`-ben és a `person.display_name`-ben él,
  a G2-18 szintjei szerint (a `contact` szint: `owner`, `producer`, `coordinator`); ide **új személyes adat tábla nem
  készül**. A magánszemély kapcsolattartó (pl. lakástulajdonos) **harmadik fél**: a G2-18 törlési szabálya rá is
  vonatkozik (`person.anonymize`), a hivatkozásjelentés kiírja a helyszíneket, ahol szerepel. A `location_contact`
  `role` mezője nem személyes.
- **A cím és a koordináta** a helyszín üzleti adata, **nem** a kapcsolattartó személyes adata; az auditba nem
  redaktáltan kerül. Magánlakásnál a cím a tulajdonosra utalhat: ha a tulajdonos törlést kér, a kapcsolattartó
  anonimizálható, a **cím a produkció történeti rekordjában marad** (nem a személy adata), vagy a helyszín
  mezőszinten törölhető/visszavonható; hogy melyik szükséges, jogi döntés (G2-18 5.3/4). A felület a magánlakás
  jellegű (`apartment`, `house`) helyszínnél erre figyelmeztet.

### 4.2 Forgatási korlátozások

```sql
CREATE TABLE backlot.location_restriction (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    location_id uuid NOT NULL REFERENCES backlot.location(id),
    kind text NOT NULL CHECK (kind IN ('time_window','weekdays','noise','max_crew','other')),
    params jsonb NOT NULL DEFAULT '{}',                    -- fajtánként zárt séma
    note text CHECK (length(note) <= 1000),
    retired_at timestamptz,
    ...közös oszlopok
);
```

| Fajta | `params` | Kiértékelés |
|---|---|---|
| `time_window` | `{"from": "08:00", "to": "18:00"}` (helyi falióra-idő, a helyszínen) | foglalás napi idejére (4.3) |
| `weekdays` | `{"allowed": [1,2,3,4,5]}` (ISO 1 = hétfő … 7 = vasárnap) | foglalás napjaira |
| `max_crew` | `{"max": 20}` | a foglalás `expected_crew` értékére |
| `noise` | `{"max_db": 65}` (opcionális, a `note` szöveggel) | **tájékoztató**, nem értékeljük (nincs mérési adat) |
| `other` | – (a `note` a tartalom) | **tájékoztató** |

- A `time_window`, `weekdays`, `max_crew` **értékelt** korlátozás: a foglalás ellen ütközést jelez (4.3). A `noise` és az
  `other` **csak megjelenik** (a felület a helyszínadatlapon és a foglalásnál kiírja), de a rendszer nem tesz úgy,
  mintha ellenőrizné.
- A Shooting Schedule (23.) a jelenet-időbeosztás ellen ugyanezeket a korlátozásokat értékeli, ugyanazzal az
  értékelő függvénnyel (egy hely az ellenőrzésnek); a 20. lépcsőben a foglalásra fut.

### 4.3 Elérhetőség és foglalás

```sql
CREATE TABLE backlot.location_availability_window (       -- a helyszín tulajdonosa/gondnoka által közölt elérhetőség
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    location_id uuid NOT NULL REFERENCES backlot.location(id),
    kind text NOT NULL CHECK (kind IN ('available','unavailable')),
    date_from date NOT NULL, date_to date NOT NULL CHECK (date_to >= date_from),
    note text CHECK (length(note) <= 500),
    ...közös oszlopok
);

CREATE TABLE backlot.location_booking (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    location_id uuid NOT NULL REFERENCES backlot.location(id),
    shooting_day_id uuid REFERENCES backlot.shooting_day(id),    -- ha egy forgatási napra szól: a dátum a napé
    purpose text NOT NULL DEFAULT 'shoot' CHECK (purpose IN ('shoot','prep','strike','scout')),
    date_from date NOT NULL, date_to date NOT NULL CHECK (date_to >= date_from),
    start_time time, end_time time,                              -- napi időablak, helyi idő; vagy mindkettő, vagy egyik sem
    expected_crew integer CHECK (expected_crew >= 0),
    status text NOT NULL CHECK (status IN ('requested','confirmed','cancelled')),
    note text CHECK (length(note) <= 1000),
    ...közös oszlopok,
    CHECK ((start_time IS NULL) = (end_time IS NULL)),
    CHECK (start_time IS NULL OR end_time > start_time)
);
CREATE INDEX ON backlot.location_booking (location_id, date_from, date_to) WHERE status <> 'cancelled';
```

- **Elérhetőségi ablak:** a helyszín közölt elérhetősége. Ha egy helyszínhez **van `available` ablak**, a foglalásnak
  valamelyikbe bele kell esnie, különben jelzés; **`unavailable` ablak** bármely foglalással ütközik. Nincs ablak: nincs
  megkötés.
- **Foglalás:** a produkció helyszínhasználata (forgatás, előkészítés, bontás, szemle) dátumtartományra és
  opcionális napi időablakra. A `shooting_day_id` megadásakor `date_from = date_to` = a forgatási nap dátuma (trigger).
  A `requested`/`confirmed`/`cancelled` állapot a foglalás **külső visszaigazolását** követi (a tulajdonos
  engedélye); a rendszer ezt nem automatizálja, a felhasználó állítja. Folyamatmotor nem kell.
- **Az időkre:** a foglalás napi ideje **helyi falióra-idő** a helyszínen (időzóna nélkül); a G2-17 esemény
  időzónás modelljétől eltérően itt nincs UTC-konverzió, mert a foglalás és a korlátozás ugyanabban a helyi időben van.

**Ütközések (számolt, nem tilt, lekérdezéskor):**

| Jelzés | Feltétel | Súly |
|---|---|---|
| `booking_overlap` | Két nem lemondott foglalás ugyanazon a helyszínen átfed (a napok átfedése, és ha mindkettőnek van időablaka, az idő átfedése) | hiba (`confirmed`–`confirmed`), figyelmeztetés (ha az egyik `requested`) |
| `in_unavailable_window` | A foglalás metszi az `unavailable` ablakot | figyelmeztetés |
| `outside_available_windows` | A helyszínnek van `available` ablaka, de a foglalás nem esik bele egyikbe sem | figyelmeztetés |
| `violates_time_window` | A foglalás napi időablaka kilóg a `time_window` korlátozásból (vagy nincs időablak megadva, de van korlát: információ) | figyelmeztetés |
| `violates_weekday` | A foglalás egy napja nem esik az `allowed` napok közé | figyelmeztetés |
| `exceeds_max_crew` | `expected_crew` nagyobb a `max_crew` korlátnál | figyelmeztetés |
| `permit_missing_or_expired` | A helyszínhez nincs olyan `permit` típusú, nem visszavont dokumentum, amelynek érvényessége (G2-14 `valid_from`/`valid_until`) lefedi a foglalás napjait; `shoot` célnál | figyelmeztetés |
| `shooting_day_mismatch` | A `shooting_day_id` dátuma nem egyezik a foglalás dátumával (külső módosítás után) | figyelmeztetés |

- Az ütközés **nem tilt**: a foglalás menthető, a rendszer jelez, szerkesztéskor azonnal a válaszban és a nézetben.
  Aszinkron értesítés nincs M1-ben (mint a G2-17-ben).
- A **`permit` dokumentum** a G2-14 alaptípusa; a helyszínhez a `document_link` köti (`subject_kind = 'location'`,
  `relation = 'permit_for'`). A lejáratra a G2-14 szerinti jelzés (G2-12) is figyel.

### 4.4 Fotó

```sql
CREATE TABLE backlot.location_photo (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    location_id uuid NOT NULL REFERENCES backlot.location(id),
    file_object_id uuid NOT NULL REFERENCES cfa4_files.file_object(id) ON DELETE RESTRICT,
    thumbnail_file_object_id uuid REFERENCES cfa4_files.file_object(id) ON DELETE RESTRICT,
    thumbnail_state text NOT NULL DEFAULT 'pending' CHECK (thumbnail_state IN ('pending','ready','failed','none')),
    original_name text NOT NULL, mime text NOT NULL,
    caption text CHECK (length(caption) <= 500),
    taken_on date,                                        -- kézzel megadva
    ordinal integer NOT NULL,
    is_primary boolean NOT NULL DEFAULT false,
    retired_at timestamptz,
    ...közös oszlopok
);
CREATE UNIQUE INDEX ON backlot.location_photo (location_id) WHERE is_primary AND retired_at IS NULL;
```

- **Felvétel** (`backlot.location.photo.add`): `files.import` az `attachment` gyökérbe (útsablon:
  `{production.code}/locations/{location.code}/photos/{file.name}`, a G2-09 közös motorjával; a mezőregiszter
  `location.code` mezőt kap), `files.seal` (a fájl változtathatatlan; csere = új fotó, a régi visszavonható),
  majd egy tranzakcióban a sor, az audit; a bélyegkép háttér-jobbal készül (Media Services `thumbnail`, a `review`
  gyökérbe). Hibánál a lezárt fájl `deleted` állapotba kerül (G2-14 mintája).
- **Formátum:** amit a bélyegkép-készítés elvégez; más fájl (pl. a motor által nem támogatott kép) tárolódik,
  `thumbnail_state = 'none'`, általános ikonnal. Méretkorlát beállítható (alapból 64 MiB fényképenként).
- **Törlés** nincs, csak visszavonás (`retired_at`); a fizikai törlés a File Services szabálya szerint
  (két lépcső, kifejezett jóváhagyás, lezárt fájlra hivatkozó élő sor esetén tiltott).
- **Sorrend és kiemelt kép:** `ordinal`, legfeljebb egy `is_primary` helyszínenként (részleges egyedi index).

### 4.5 Másolás másik produkcióba

`backlot.location.copy_to_production {location, target_production, include: [restrictions, photos, contacts]}`:

- Alapból az **adatlap** (név, típus, cím, koordináta, megközelítés, műszaki adat, leírás) másolódik; opcionálisan a
  **korlátozások**, a **fotók** (új `file_object`-ek, másolat az új produkció gyökerébe, mert az archívum
  produkciónként önálló, ADR 0009/26.) és a **kapcsolattartók** (a `person` közös, csak a kapcsolat másolódik).
- **Soha nem másolódik:** foglalás, elérhetőségi ablak, engedély és más dokumentum, forgatókönyvi helyszín-kapcsolat.
- A célban új azonosítójú, új kódú helyszín (ütközésnél javasolt új kód); mindkét produkcióban auditált
  (`location.copied`); csak olyan felhasználó futtathatja, aki mindkét produkció tagja (#42); a forrás változatlan.

## 5. Hozzáférés, személyes adat és fotók

### 5.1 Hozzáférés

- A **helyszín, a foglalás, a korlátozás és az elérhetőség** a produkció minden tagja számára látható és
  az `owner`, `producer`, `coordinator` szerkeszti (#42; M1-ben a helyi felhasználó tulajdonos).
- A **kapcsolattartó személyes adata** a G2-18 szintjei szerint (`contact`: `owner`, `producer`, `coordinator`; többi
  szint: `owner`, `producer`); a `location_contact` sor és a szerep a produkció minden tagjának látható, a név és az
  elérhetőség nem.
- A szabály a szolgáltatásrétegben és a lekérdezésben **már most** érvényes.

### 5.2 Személyes adat és törlés

- A **magánszemély kapcsolattartó** adata a G2-18 tábláiban él, így az anonimizálás (G2-18 5.3) rá is kiterjed; a
  hivatkozásjelentésben a helyszínek is szerepelnek (a Location modul regisztrál hivatkozás-szolgáltatót; törlő
  kezelőre nincs szükség, mert a kapcsolat csak azonosítót tartalmaz).
- A `description`, `directions`, `note` mezők szabad szöveg: személyes adatot **tartalmazhatnak**, a rendszer nem írja
  át (G2-18 5.3/1); az audit ezeket a G2-01 szabálya szerint kezeli (személyes jelölésű mezők nincsenek, ezért az érték
  az auditban szerepel); a felület a szabad szöveges mezőknél a **személyes adat kerülését** kéri.

### 5.3 A fotók metaadata

- Az **EXIF-adat (GPS, készülék, idő) nem olvasódik ki és nem kerül az adatbázisba**: a G1 felvetette a GPS-koordináta
  kiolvasását, de egy magánlakásról készült fotó rejtett helyadata a felhasználó tudta nélkül kerülne a katalógusba
  és a keresésbe. A `taken_on` kézi adat. A **fotófájl maga megtartja az EXIF-et** (a lezárt eredeti nem módosul);
  ezt a felület a felvételnél kimondja, és az **export** (PDF adatlap) újrarenderelt bélyegképet használ, EXIF nélkül.
- A fotók nem indexeltek tartalom szerint (nincs képfelismerés).

## 6. Parancsok és hibakódok

| Parancs | Mit csinál |
|---|---|
| `backlot.location.create` / `.update` / `.retire` / `.restore` / `.list` / `.get` | Valós helyszín (adatlap, műszaki adat) |
| `backlot.location.set.create` | Forgatókönyvi helyszín (az entitás-létrehozót hívja, `set` típus, 3.1) |
| `backlot.location.set_link.set` / `.remove` / `.list` | Valós ↔ forgatókönyvi helyszín (jelölt, kiválasztott, elvetett) |
| `backlot.location.scene_link.add` / `.remove` / `.list` | Jelenet ↔ forgatókönyvi helyszín (`setting`, 3.2) |
| `backlot.location.contact.add` / `.update` / `.remove` / `.list` | Kapcsolattartók |
| `backlot.location.restriction.add` / `.update` / `.retire` / `.list` | Korlátozások |
| `backlot.location.availability.add` / `.update` / `.remove` / `.list` | Elérhetőségi ablakok |
| `backlot.location.booking.create` / `.update` / `.set_status` / `.cancel` / `.list` | Foglalások |
| `backlot.location.conflicts` | Ütközések egy tartományra vagy helyszínre (4.3) |
| `backlot.location.photo.add` / `.update` / `.retire` / `.list` / `.set_primary` | Fotók |
| `backlot.location.copy_to_production` | Másolás (4.5) |
| `backlot.location.distance` | Két helyszín **légvonalbeli** távolsága (haversine, km); nem útvonal |
| `backlot.location.export` | Helyszínadatlap (PDF) és helyszínlista (CSV) |

Hibakódok: `LOCATION_NOT_FOUND`, `LOCATION_CODE_TAKEN`, `LOCATION_TECH_INVALID`, `LOCATION_ADDRESS_INVALID`,
`LOCATION_COORDINATES_INVALID`, `LOCATION_LINK_INVALID`, `LOCATION_BOOKING_INVALID`, `LOCATION_RESTRICTION_INVALID`,
`LOCATION_PHOTO_INVALID` (típus, méret), `LOCATION_COPY_FORBIDDEN` (nem mindkét produkció tagja),
`LOCATION_EXPORT_FAILED`, valamint a `FILES_*`, `DOCUMENT_*`, `ENTITY_*`, `PERSON_*`, `AUDIT_*` kódok változatlanul.

## 7. Térkép és geokódolás: M1-ben nincs (eltérés a G1-től)

A G1 az OpenStreetMap-csempét és a Nominatim-geokódolást opcionálisan, kifejezett bekapcsolással javasolta.
**Javaslat: M1-ben nem készül, bekapcsolható változat sem.** Indok: a címkeresés a helyszín (gyakran magánlakás)
címét küldené egy külső szolgáltatásnak, ami a „felhasználói adat nem hagyja el a gépet” szabályba (SEC-02) ütközik
még opt-inként is, mert a harmadik fél (a tulajdonos) címéről van szó; a Nominatim használati feltételei sem
engedik a tömeges vagy automatikus lekérdezést; és a csempe-megjelenítés új hálózati és grafikai réteget kérne.

Ami van, hálózati kérés nélkül:
- **Koordináta kézzel** (tizedes fok, ellenőrzött tartomány); a koordinátát a felhasználó másolja a térképes
  alkalmazásból.
- **„Megnyitás térképen”:** `xdg-open geo:<szélesség>,<hosszúság>` (RFC 5870), vagyis a **felhasználó saját
  térképalkalmazása** nyílik meg a koordinátával. Ez felhasználói művelet; a BacklotHub maga nem küld hálózati
  kérést. Első használatkor a felület jelzi, hogy a koordinátát az operációs rendszer által választott alkalmazás kapja
  meg (amely maga használhat hálózatot).
- **Távolság** két helyszín között légvonalban (`backlot.location.distance`).

Újravizsgálat: ha M2/M3 elindul, vagy a felhasználó kifejezetten kér térképet; akkor külön terv (helyi
térképcsempe-csomag hálózati kérés nélkül, vagy kifejezett engedélyhez kötött szolgáltatás). PostGIS nem kell (a G1
szerint sem M1-ben).

## 8. Never crash

- Hibás cím, koordináta, korlátozás-paraméter vagy műszaki adat: angol hibakód, nincs írás, nincs összeomlás.
- Hiányzó vagy sérült fotófájl (`FILES_ROOT_UNAVAILABLE`, `FILES_HASH_MISMATCH`), a bélyegkép-job hibája: a
  helyszín használható marad, a fotó `thumbnail_state = 'failed'`, kezelt hiba.
- Foglalás törölt/visszavont forgatási napra: figyelmeztetés (`shooting_day_mismatch`), nincs árva sor; visszavont
  helyszínre meglévő foglalás a felületen jelölt.
- A másolás és a fotófelvétel hibája nem hagy félig kész állapotot (egy tranzakció, a lezárt fájl `deleted`).
- A `geo:` megnyitás sikertelensége (nincs kezelő alkalmazás): kezelt hibaüzenet, a koordináta szövegként másolható.

## 9. Sorrend és függés

| Rész | Kell hozzá |
|---|---|
| Helyszín, kapcsolattartó, korlátozás, elérhetőség, foglalás, ütközés, jelenetkapcsolat | 2., 3., 5. lépcső; a kapcsolattartó személyes adata a 18. lépcsőből |
| Fotók | **File Services FS-1**, Media Services **MS-1** (`thumbnail`) |
| Engedély-ellenőrzés, lejárat | 14. lépcső |
| Foglalás-naptárréteg és forgatási nap kapcsolat | 17. lépcső |
| Korlátozás értékelése jelenet-időbeosztásnál | 23. lépcső |

## 10. Tesztek (G5-re előre)

- **Kapcsolatok:** több–több (valós ↔ forgatókönyvi), több `selected`, `set_without_location`; a `setting` kapcsolat
  típuspárjai és a duplikáció elutasítása; a három nézet (jelenet→helyszín, helyszín→jelenetek, set→valós helyszínek)
  egyezik a nyers kapcsolatokkal (property-teszt).
- **Foglalás és ütközés:** minden jelzés (átfedés `confirmed`–`confirmed` hiba, `requested` figyelmeztetés; ablakok;
  időablak, hétköznap, létszám; engedély hiányzik/lejárt; forgatási nap eltérés); éjfélen átnyúló vagy hibás időablak
  elutasítva; az ütközés nem tiltja a mentést.
- **Korlátozás-értékelő:** táblázatos teszt a három értékelt fajtára (határesetekkel: pont a `to` időben,
  vasárnap-hétfő); a `noise` és az `other` nincs kiértékelve.
- **Fotó:** felvétel (import, lezárás, bélyegkép), hibainjektálás az import és a tranzakció között, nem támogatott
  formátum (`none`), sérült fájl, az `is_primary` egyedisége, EXIF-et tartalmazó kép: **az adatbázisban nincs
  EXIF-adat**, az export bélyegképe EXIF nélküli.
- **Hozzáférés:** minden adat minden szerepre; a kapcsolattartó neve/elérhetősége a G2-18 szintjei szerint; nem tag
  produkció helyszíne nem látszik; a másolás csak mindkét produkció tagjának működik.
- **Másolás:** alap (adatlap), a három opció, a „soha nem másolt” elemek hiánya, kódütközés, a forrás
  változatlansága.
- **Anonimizálás:** a kapcsolattartó anonimizálása után a helyszín, a kapcsolat és a foglalás megmarad, a személyes
  adat nincs sehol (a G2-18 teljességi vizsgálata); a hivatkozásjelentés kiírja a helyszíneket.
- **Térkép és hálózat:** a modul **semmilyen hálózati kérést nem indít** (teszt hamis hálózati réteggel, nulla kérés);
  `geo:` megnyitás kezelő nélkül kezelt hiba.
- **Távolság:** haversine ismert pontpárokra (határeset: sarkok, dátumvonal, azonos pont).
- **Naptárréteg:** a `location_bookings` réteg a forrásból számol, a szerkesztés a foglalás parancsát hívja.
- **Never crash:** hibás bemenetek fuzzolása.
- **Felületi ellenőrzés:** `ui.screenshot` az adatlapról, a fotógalériáról és a foglalási naptárról ütközéssel.
- **Mérés** a CFA4-MIN gépen: 2000 helyszín, 20 000 fotó (bélyegkép-lista), 10 000 foglalás, lista és
  ütközésszámítás ideje.

## 11. Licenc (G3-ra előre)

Új függőség nincs. A `geo:` URI RFC 5870 (nyílt szabvány), megnyitása `xdg-open`. A haversine és az ellenőrzők saját
kód; a PDF `QPdfWriter` (PySide6, mint a G2-13). OpenStreetMap/Nominatim: **nem használjuk** (7. pont). PostGIS
(GPL-2.0-or-later): nem kell. StudioBinder, Celtx, Reel Scout: funkcionális referencia.

## 12. G2-eredmény

**Javaslat: PASS.** Új döntési kérdés nincs; két G1-től eltérő pont indokolva (2.1: nem `resource`, 7.: nincs térkép és
geokódolás M1-ben). A megvalósítás a 3., 5., 18. lépcső és a File Services FS-1 után indulhat; a fotók a Media
Services MS-1-től, az engedély-ellenőrzés a 14., a naptárréteg a 17. lépcsőtől működik. A G2-03 (`setting` kapcsolat),
a G2-05 és G2-14 (`location` a `subject_kind` regiszterben) kiegészítése megvalósításkor kerül oda, új tábla nélkül.
