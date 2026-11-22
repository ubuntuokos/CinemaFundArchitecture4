# BacklotHub 21. lépcső – Equipment Management: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-21 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.19 · **Előzmény:**
[G1 Equipment Management](CFA4-BACKLOTHUB-G1-21-EQUIPMENT-MANAGEMENT-2026-11-07.md) (funkcionális követelmények
jóváhagyva 2026-11-10) · **Épül rá:** G2-04 (Workflow Engine), G2-05 (`person`, közös `subject_kind` regiszter),
G2-12 (jelzések), G2-14 (dokumentum), G2-17 (naptárrétegek, forgatási nap), G2-18 (stábtag), G2-20 (helyszín) ·
**Állapot:** G2-javaslat, jóváhagyásra vár · implementáció nincs

Új döntési kérdés nincs, új függőség nincs. **Két eltérés a G1-től**, mindkettő indokolva: az eszköz **nem
`resource`-sor** (2.1), és a **tárolási hely nem `location`** (2.2). Ugyanez az elv, mint a G2-20-ban.

## 1. Hatókör ebben a lépcsőben

- **Eszköz** (fizikai felszerelés: kamera, objektív, világítás, hang, grip, adathordozó), **típuskatalógus**,
  **eszközcsoport** (kit), **állapot**, **tulajdon** (saját, bérelt, kölcsönkapott).
- **Foglalás** produkcióra, opcionálisan forgatási napra és jelenetre, **ütközésjelzéssel** produkciók között is.
- **Kiadás és visszavétel** stábtagnak vagy helyszínre, állapotfelméréssel és hiánylistával.
- **Karbantartás** (javítás, kalibrálás, tisztítás, ellenőrzés), esedékességgel.
- **Dokumentumok** (kézikönyv, bérleti szerződés, biztosítás) a G2-14 szerint.
- **Nem része:** a bérleti díj és a karbantartási költség **összegzése és elszámolása** (Budget, 24.; itt csak
  tényadat); vonalkód/QR-olvasás és címkenyomtatás (21-2 al-lépcső, külön döntés nélkül, új függőség nélkül nem
  vállalható); amortizáció és leltári érték (nem produkciós adat); a **digitális asset** (8. lépcső): az `equipment`
  és az `asset` neve a kódban és a felületen sem keveredik.

## 2. Integrációs határok

| Határ | Irány | Szerződés |
|---|---|---|
| Workflow Engine (G2-04) | ← | Az eszköz állapota a `status_id` (a tárgyon hiteles); folyamat: `equipment_default` (4.1). A kiadás és a visszavétel tranzakcióban lépteti |
| Person (5.), Crew (18.) | ← | Az átvevő a közös `person`; új személyes adat tábla nincs; a törlési szabály a G2-18 szerint (a kiadási sorban csak azonosító) |
| Organization (2.) | ← | A bérbeadó `organization_id` |
| Production (2.), Structure (3.) | ← | A foglalás `production_id`-t, opcionálisan forgatási napot (G2-17) és jelenet-elemet (`production_entity`) hordoz |
| Location (G2-20) | ← | A kiadás **célja** lehet valós helyszín (azonos produkció); a tárolási hely nem (2.2) |
| Document (14.) | ← | Kézikönyv, szerződés, biztosítás: `subject_kind = 'equipment'`; lejárat a G2-14 szerint |
| Calendar (17.) | ↔ | Új rétegek: `equipment_bookings`, `equipment_returns` (várt visszavétel) |
| Event Fabric / Notification (12.) | → | Események (7. pont); `deadline_signal`: esedékes karbantartás és késő visszavétel |
| Search (7.) | → | `SearchProvider`: eszköz (név, leltári szám, sorozatszám, típus, gyártó), típus, csoport |
| Audit (1.) | → | Minden írás, minden állapotváltás auditált |
| Budget (24.) | később | A `rental_fee_minor`, `cost_minor` tényadatot olvassa; itt nincs összeg-számítás |
| Shooting Schedule (23.) | később | A forgatási napra foglalt eszközöket a naptárréteg és a `backlot.equipment.conflicts` adja |

### 2.1 Az eszköz nem erőforrás (eltérés a G1-től és a G2-15 előrejelzésétől)

A G1 és a G2-15 2.1. pontja az eszközt a `resource` újabb típusaként tervezte. A G2-15 `resource` sora **személyhez
kötött** (`person_id NOT NULL`, munkanaptár, perc/nap kapacitás, terhelésszámítás). Az eszköznek nincs `person_id`-je és
nincs napi percben mért kapacitása, csak **foglalhatósága** (egy darab, vagy ott van, vagy nincs). A `kind` bővítése
nullázható oszlopokat, hamis terhelésszámítást és hívó nélküli elágazást hozna. A G2-20-szal azonos elv: az eszköz saját,
egyszerű foglalási modellt kap, **ugyanazzal az ütközésjelzési elvvel** (számolt, nem tilt). A G2-15 szövege
megvalósításkor pontosodik (a `kind` lista marad `person`); a 21. lépcső ehhez **nem nyúl**.

### 2.2 Tárolási hely: saját, kis tábla (eltérés a G1-től)

A G1 az eszköz tárolási helyét a Location modulból vette. A G2-20 szerint a helyszín **produkciónként külön sor**, az
eszköz viszont **produkciófüggetlen** (2.3), így egy produkcióhoz kötött sor nem lehet a tárolási helye. Ezért
**`equipment_storage`**: kis, produkciófüggetlen tábla (raktár, stúdió, szekrény, jármű), amely a helyszínkönyvtárat
sem előlegezi meg. A **kiadás célja** viszont lehet `location` (azonos produkcióban), mert az a produkció
helyszínhasználata.

### 2.3 Produkciófüggetlen eszköz, produkcióhoz kötött foglalás

A saját eszköz **több produkcióban** is használatos, ezért a `equipment`, `equipment_type`, `equipment_kit`,
`equipment_storage`, `maintenance_record` **nem produkcióhoz kötött** (a G2-15 `resource` mintája: közös oszlopok
`production_id` nélkül). A **foglalás és a kiadás** produkcióhoz kötött. A produkciók közötti ütközést ez teszi
láthatóvá, ami a G1 egyik fő elvárása. A láthatóság a G2-15 2.1 szabálya szerint: más produkció foglalásának
**részletét** a felhasználó csak ott látja, ahol tag (#42); egyébként „foglalt (másik produkció)” és időszak. M1-ben a
helyi felhasználó mindenben tag; a szűrés a lekérdezésben már él.

## 3. Állapot és folyamat

A G1 hat állapotot sorolt fel (`available`, `reserved`, `checked_out`, `maintenance`, `damaged`, `retired`). A
**`reserved` nem tárolt állapot**: a foglalás időfüggő és több is lehet egyidejűleg, ezért számolt (lekérdezéskor a
foglalásból). Tárolt állapot ötféle:

| `status` | Jelentés | Belépés |
|---|---|---|
| `in_stock` | a tárolási helyen, használható | alapállapot; visszavétel jó állapotban; javítás vége |
| `checked_out` | kiadva | `equipment.checkout` |
| `maintenance` | karbantartásban | karbantartás megkezdése |
| `damaged` | sérült, még nincs javítás | visszavétel sérüléssel; kézzel |
| `retired` | kivonva a használatból | kézzel; kivonás után nem foglalható |

Folyamat `equipment_default` (G2-04 zárt őrfeltételekkel): `in_stock → checked_out` (őr: nincs nyitott kiadás),
`checked_out → in_stock | damaged`, `in_stock | damaged → maintenance`, `maintenance → in_stock | damaged`,
`* → retired` (nem `checked_out`-ból: előbb visszavétel), `retired → in_stock` (visszaállítás, indoklással). Az
állapotváltás a tárgy parancsának **ugyanabban a tranzakciójában** történik és auditált, a kiadás-visszavétel nem
megkerülhető kézi átállítással (a kézi átállítás `checked_out`-ba és -ból tiltott, `EQUIPMENT_STATUS_INVALID`).

## 4. Adatmodell (DDL-vázlat, `backlot` séma)

### 4.1 Katalógus, eszköz, csoport, tárolási hely

```sql
CREATE TABLE backlot.equipment_storage (                  -- tárolási hely, produkciófüggetlen
    id uuid PRIMARY KEY,
    code text NOT NULL UNIQUE CHECK (length(code) BETWEEN 1 AND 40),
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    note text CHECK (length(note) <= 1000),
    retired_at timestamptz,
    ...közös oszlopok production_id nélkül
);

CREATE TABLE backlot.equipment_type (                     -- katalógus
    id uuid PRIMARY KEY,
    category text NOT NULL CHECK (category IN ('camera','lens','lighting','audio','grip','media','power','other')),
    manufacturer text CHECK (length(manufacturer) <= 200),
    model text NOT NULL CHECK (length(model) BETWEEN 1 AND 200),
    specs jsonb NOT NULL DEFAULT '{}',                    -- cfa4.backlot.equipment_specs/1, kategóriánként zárt mezők
    note text CHECK (length(note) <= 2000),
    search_vector tsvector GENERATED ALWAYS AS (…) STORED,
    retired_at timestamptz,
    ...közös oszlopok production_id nélkül,
    UNIQUE (category, manufacturer, model)
);

CREATE TABLE backlot.equipment (
    id uuid PRIMARY KEY,
    type_id uuid NOT NULL REFERENCES backlot.equipment_type(id),
    inventory_no text NOT NULL UNIQUE CHECK (length(inventory_no) BETWEEN 1 AND 60),
    serial_no text CHECK (length(serial_no) <= 100),
    ownership text NOT NULL CHECK (ownership IN ('own','rented','borrowed')),
    lessor_organization_id uuid REFERENCES backlot.organization(id),
    rental_from date, rental_until date,                  -- bérelt/kölcsönkapott: az érvényesség
    rental_fee_minor bigint CHECK (rental_fee_minor >= 0),-- tényadat a Budgetnek (24.); a pénznem külön
    currency_code char(3),
    status_id uuid NOT NULL REFERENCES backlot.workflow_status(id),    -- G2-04, a hiteles állapot
    storage_id uuid REFERENCES backlot.equipment_storage(id),
    note text CHECK (length(note) <= 2000),
    search_vector tsvector GENERATED ALWAYS AS (…) STORED,
    retired_at timestamptz,
    ...közös oszlopok production_id nélkül,
    CHECK (ownership <> 'rented' OR lessor_organization_id IS NOT NULL),
    CHECK (rental_until IS NULL OR rental_from IS NULL OR rental_until >= rental_from),
    CHECK ((rental_fee_minor IS NULL) = (currency_code IS NULL))
);

CREATE TABLE backlot.equipment_kit (                      -- csoport, pl. „A-kamera csomag”
    id uuid PRIMARY KEY,
    code text NOT NULL UNIQUE, name text NOT NULL, note text,
    retired_at timestamptz,
    ...közös oszlopok production_id nélkül
);

CREATE TABLE backlot.equipment_kit_member (
    kit_id uuid NOT NULL REFERENCES backlot.equipment_kit(id),
    equipment_id uuid NOT NULL REFERENCES backlot.equipment(id),
    removed_at timestamptz,
    PRIMARY KEY (kit_id, equipment_id)
);
CREATE UNIQUE INDEX ON backlot.equipment_kit_member (equipment_id) WHERE removed_at IS NULL;   -- legfeljebb egy aktív csoport
```

- **A pénznem** a G1/#15 szerint ISO 4217 kód; az összeg **tényadat**, a rendszer nem szorozza, nem adja össze, nem
  vált pénznemet (az a Budget feladata). `ownership = 'own'` eszköznél a bérleti mezők üresek (trigger).
- **A `specs`** zárt mezőkkel, kategóriánként (pl. kamera: `sensor`, `mount`, `max_fps`; objektív: `mount`,
  `focal_length_mm`, `aperture`), JSON Schema-val, ismeretlen kulcs `EQUIPMENT_SPECS_INVALID`. A séma a
  típuskatalógus része, nem a felhasználó szabadon bővíthető.
- **A csoport** foglalható és kiadható egységként, de a foglalás és a kiadás **tagonként** tárolódik (4.2, 4.3), mert
  egy tag külön is meghibásodhat vagy hiányozhat. A csoport tagja **legfeljebb egy aktív csoportban** van (részleges
  egyedi index): egy objektívet két csomag egyszerre nem hordoz.
- **A kódban és a felületen `equipment`**, soha nem `asset` (a 8. lépcső digitális eleme).

### 4.2 Foglalás

```sql
CREATE TABLE backlot.equipment_booking (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    equipment_id uuid NOT NULL REFERENCES backlot.equipment(id),
    kit_booking_id uuid,                                  -- ha csoportfoglalásból jött (azonos érték a tagoknál)
    kit_id uuid REFERENCES backlot.equipment_kit(id),
    shooting_day_id uuid REFERENCES backlot.shooting_day(id),
    entity_id uuid REFERENCES backlot.production_entity(id),     -- jelenet/snitt, opcionális
    starts_at timestamptz NOT NULL, ends_at timestamptz NOT NULL CHECK (ends_at > starts_at),
    status text NOT NULL CHECK (status IN ('requested','confirmed','cancelled')),
    note text CHECK (length(note) <= 1000),
    ...közös oszlopok
);
CREATE INDEX ON backlot.equipment_booking (equipment_id, starts_at, ends_at) WHERE status <> 'cancelled';
```

- **Egyedi darab, nincs mennyiség.** Egy sor egy eszköz egy időszakra. Fogyóeszköz és kötegelt készlet (kábelek, elemek
  darabszámmal) **nem része** a lépcsőnek: azt egy darabonként nyilvántartott eszközként lehet felvenni, ha kell.
- **Az időszak** `timestamptz` (felvétel kezdete és vége, helyi időzónában megadva, UTC-ben tárolva), hogy a
  bérbeadói átvételi idő is kifejezhető legyen. `shooting_day_id` esetén a nap dátuma szerinti helyi napot vet alapul
  (G2-17), az `entity_id` csak jelzés (a Shooting Schedule, 23., értékeli).
- **Csoportfoglalás** (`backlot.equipment.booking.create_for_kit`): egy `kit_booking_id` alatt tagonként egy sor, a
  **foglalás pillanatában aktív tagokkal**; a csoport későbbi változása a meglévő foglalásokat **nem** módosítja
  (a felület jelzi az eltérést: `kit_membership_changed`, információ). Lemondás: egyben vagy tagonként.
- **Állapot** (`requested`/`confirmed`/`cancelled`): mint a G2-20-ban, a külső visszaigazolást (bérbeadó, raktár) követi,
  a felhasználó állítja; folyamatmotor nem kell.

**Ütközések (számolt, nem tilt, lekérdezéskor):**

| Jelzés | Feltétel | Súly |
|---|---|---|
| `booking_overlap` | Ugyanazon eszközre két nem lemondott foglalás időben átfed (**bármely produkcióban**) | hiba (`confirmed`–`confirmed`), figyelmeztetés (ha az egyik `requested`) |
| `unavailable_status` | Az eszköz állapota `damaged` vagy `retired` a foglalás kezdetén | hiba (`retired`), figyelmeztetés (`damaged`) |
| `in_maintenance` | A foglalás metszi a tervezett vagy folyamatban lévő karbantartás időszakát | figyelmeztetés |
| `maintenance_overdue` | Az eszköz esedékes karbantartása (`next_due_on`) a foglalás kezdetéig lejár, és nincs tervezett karbantartás | figyelmeztetés |
| `outside_rental_period` | `rented`/`borrowed` eszköznél a foglalás kívül esik a `rental_from`–`rental_until` időszakon | figyelmeztetés |
| `still_checked_out` | A foglalás kezdetekor az eszköz egy másik foglaláshoz kiadva van, és a várt visszavétel későbbi | figyelmeztetés |
| `shooting_day_mismatch` | A `shooting_day_id` dátuma és a foglalás időszaka nem egyezik (külső módosítás után) | figyelmeztetés |
| `kit_membership_changed` | A csoport tagjai eltérnek a foglaláskori állapottól | információ |

- A **karbantartás miatti foglalási tilalom** a G1-ben (esedékes karbantartás alatt nem foglalható) **figyelmeztetésként**
  szerepel itt: a foglalás menthető, a rendszer jelez. A tiltás ott lenne helyes, ahol a rendszer biztosan tudja az
  igazságot; a karbantartás tényét a felhasználó viszi fel, ezért a hamis tiltás (nem rögzített elvégzett javítás miatt)
  a forgatást akadályozná. Ez a G2-15/G2-17/G2-20 elvével azonos.
- A foglalás menthető ütközéssel is; a jelzés a szerkesztéskor a válaszban és a nézetben azonnal megjelenik.
  Aszinkron értesítés M1-ben nincs az ütközésre (az események a 7. pontban).

### 4.3 Kiadás és visszavétel

```sql
CREATE TABLE backlot.equipment_checkout (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    equipment_id uuid NOT NULL REFERENCES backlot.equipment(id),
    booking_id uuid REFERENCES backlot.equipment_booking(id),
    to_person_id uuid REFERENCES backlot.person(id),
    to_location_id uuid REFERENCES backlot.location(id),        -- trigger: ugyanaz a produkció
    out_at timestamptz NOT NULL,
    expected_return_at timestamptz NOT NULL,
    condition_out text NOT NULL CHECK (condition_out IN ('good','worn','damaged')),
    note_out text CHECK (length(note_out) <= 1000),
    acknowledged_at timestamptz,                                -- az átvevő visszaigazolta (kézzel jelölve)
    in_at timestamptz,
    condition_in text CHECK (condition_in IN ('good','worn','damaged')),
    missing_items text CHECK (length(missing_items) <= 2000),
    note_in text CHECK (length(note_in) <= 1000),
    ...közös oszlopok,
    CHECK ((to_person_id IS NULL) <> (to_location_id IS NULL)),
    CHECK (expected_return_at > out_at),
    CHECK ((in_at IS NULL) = (condition_in IS NULL)),
    CHECK (in_at IS NULL OR in_at >= out_at)
);
CREATE UNIQUE INDEX ON backlot.equipment_checkout (equipment_id) WHERE in_at IS NULL;   -- legfeljebb egy nyitott kiadás
```

- **Kiadás** (`backlot.equipment.checkout`): ellenőrzi az állapotot (`in_stock`), a nyitott kiadás hiányát, az átvevő
  érvényességét; egy tranzakcióban beírja a sort, **lépteti az állapotot** `checked_out`-ra, auditál, eseményt
  (outbox) és — ha a várt visszavétel megadott — a késő-visszavétel jelzés alapját rögzíti. Csoportnál tagonként egy
  sor, egy közös `checkout_group_id`-vel a parancs eredményében (nem külön oszlop: a `out_at` és az átvevő azonos,
  a felület csoportosít).
- **Visszavétel** (`backlot.equipment.checkin`): egy tranzakcióban lezárja a sort (`in_at`, `condition_in`, hiánylista,
  megjegyzés), lépteti az állapotot: `good`/`worn` → `in_stock`, `damaged` → `damaged`; opcionális `plan_repair: true`
  ugyanebben a tranzakcióban `planned` javítási rekordot hoz létre (4.4). A **hiánylista** szabad szöveg (nincs külön
  tétel-modell M1-ben); ha a hiány nem üres, a felület figyelmeztet, de az eszköz visszavehető.
- **A „visszaigazolás”** (`acknowledged_at`): a Snipe-IT átvételi elfogadás **egyszerűsített** megfelelője: a felhasználó
  jelöli, hogy az átvevő tudomásul vette az állapotot (M1-ben egy felhasználó, nincs külön bejelentkezés és aláírás).
  Kriptográfiai aláírás vagy e-mailes elfogadás **nincs** (a G2-14 `document_signer` szabálya szerinti aláírás a
  szerződésre marad).
- **Késő visszavétel:** a `expected_return_at` túllépése számolt (`overdue`), és a G2-12 szerinti `deadline_signal`
  időzítő jelzi (`subject_type = 'equipment_checkout'`), nem aszinkron újraszámolás.
- **Anonimizálás:** a kiadási sor csak `person_id`-t hordoz, a G2-18 törlése után az azonosító megmarad, a név nem
  elérhető (a megjelenítés „törölt személy”); a hivatkozásjelentés kiírja a kiadásokat.

### 4.4 Karbantartás

```sql
CREATE TABLE backlot.maintenance_record (
    id uuid PRIMARY KEY,
    equipment_id uuid NOT NULL REFERENCES backlot.equipment(id),
    kind text NOT NULL CHECK (kind IN ('repair','calibration','cleaning','sensor_cleaning','inspection','other')),
    status text NOT NULL CHECK (status IN ('planned','in_progress','done','cancelled')),
    planned_from date, planned_until date,
    performed_on date,
    performed_by text CHECK (length(performed_by) <= 200),       -- szervizcég vagy személy neve, szabad szöveg
    cost_minor bigint CHECK (cost_minor >= 0), currency_code char(3),
    next_due_on date,                                            -- a következő esedékesség
    note text CHECK (length(note) <= 2000),
    ...közös oszlopok production_id nélkül,
    CHECK (planned_until IS NULL OR planned_from IS NULL OR planned_until >= planned_from),
    CHECK ((cost_minor IS NULL) = (currency_code IS NULL)),
    CHECK (status <> 'done' OR performed_on IS NOT NULL)
);
CREATE INDEX ON backlot.maintenance_record (equipment_id, next_due_on) WHERE status = 'done';
```

- **A karbantartás megkezdése** (`maintenance.start`) lépteti az eszközt `maintenance` állapotba (ha `in_stock` vagy
  `damaged`; kiadott eszközre `EQUIPMENT_STATUS_INVALID`); **lezárása** (`maintenance.complete`) `done` státuszt,
  `performed_on`-t kér, és az eszközt `in_stock` (vagy az `outcome = 'still_damaged'` esetén `damaged`) állapotba teszi.
- **Esedékesség:** egy eszköz esedékessége a **legutolsó `done` rekord `next_due_on` értéke** fajtánként; a számolt nézet
  `equipment_maintenance_due`. A G2-12 időzítő ezt a `deadline_signal`-ba írja (`subject_type = 'equipment'`, `kind =
  'maintenance_due'`), a szabály szerint, ugyanúgy mint a G2-14 lejárata. Nincs új időzítő mechanizmus.
- **Költség:** tényadat a Budgetnek (24.); a rendszer nem összesít.

## 5. Hozzáférés

- Az **eszközkatalógus, a csoport, a tárolási hely, a karbantartás** minden produkció minden tagja számára látható; szerkeszti
  az `owner`, `producer`, `coordinator` (#42). A **foglalás és a kiadás** részlete az adott produkció tagjainak látható; a
  más produkcióból érkező ütközést csak „foglalt” és időszak mutatja (2.3).
- A **bérleti díj és a karbantartás költsége** a `producer`/`owner` számára látható (pénzügyi jellegű adat, mint a G2-18
  `financial` szintje); a többi szerepnek az összeg mezője üres.
- A szabály a szolgáltatásrétegben és a lekérdezésben **már most** él.

## 6. Parancsok és hibakódok

| Parancs | Mit csinál |
|---|---|
| `backlot.equipment.type.create` / `.update` / `.retire` / `.list` / `.get` | Típuskatalógus |
| `backlot.equipment.create` / `.update` / `.set_status` / `.retire` / `.restore` / `.list` / `.get` | Eszköz (a `set_status` a 3. pont szabályai szerint) |
| `backlot.equipment.storage.create` / `.update` / `.retire` / `.list` | Tárolási helyek |
| `backlot.equipment.kit.create` / `.update` / `.member.add` / `.member.remove` / `.retire` / `.list` / `.get` | Csoportok |
| `backlot.equipment.booking.create` / `.create_for_kit` / `.update` / `.set_status` / `.cancel` / `.list` | Foglalások |
| `backlot.equipment.conflicts` | Ütközések egy tartományra, eszközre vagy produkcióra (4.2) |
| `backlot.equipment.availability` | Mely eszközök szabadok egy időszakban (kategória, típus szerint szűrve) |
| `backlot.equipment.checkout` / `.checkin` / `.acknowledge` / `.checkouts` | Kiadás, visszavétel, visszaigazolás, lista |
| `backlot.equipment.maintenance.create` / `.update` / `.start` / `.complete` / `.cancel` / `.list` / `.due` | Karbantartás |
| `backlot.equipment.export` | Csomaglista (PDF), eszközlista és foglalás (CSV) |

Hibakódok: `EQUIPMENT_NOT_FOUND`, `EQUIPMENT_INVENTORY_NO_TAKEN`, `EQUIPMENT_SPECS_INVALID`,
`EQUIPMENT_STATUS_INVALID` (tiltott állapotváltás), `EQUIPMENT_BOOKING_INVALID`, `EQUIPMENT_KIT_INVALID`
(már aktív csoport tagja), `EQUIPMENT_CHECKOUT_INVALID` (nem `in_stock`, nyitott kiadás van, hibás cél),
`EQUIPMENT_CHECKIN_INVALID`, `EQUIPMENT_MAINTENANCE_INVALID`, `EQUIPMENT_RENTAL_INVALID`, `EQUIPMENT_EXPORT_FAILED`,
valamint a `DOCUMENT_*`, `PERSON_*`, `WORKFLOW_*`, `AUDIT_*` kódok változatlanul.

## 7. Események és naptár

- **Események** (outbox, G2-12): `backlot.equipment.checked_out`, `.checked_in`, `.damaged`, `.maintenance_started`,
  `.maintenance_completed`, `.booking_confirmed`, `.booking_cancelled`; **időzített jelzések** (`deadline_signal`):
  `maintenance_due`, `checkout_overdue`. Új címzett nincs: a G2-12 zárt listájának `production_owner` és a
  `watcher` elég; az átvevő (`to_person_id`) értesítése M1-ben nem kell (egy felhasználó).
- **Naptárrétegek** (G2-17 szolgáltató-regiszter): `equipment_bookings` (foglalás időszaka az eszköz és a produkció
  nevével) és `equipment_returns` (várt visszavétel és karbantartás-esedékesség). A rétegek a forrásból számolnak; a
  szerkesztés a foglalás parancsát hívja.

## 8. Never crash

- Hibás típusséma, hibás időszak, tiltott állapotváltás, nyitott kiadás melletti új kiadás: angol hibakód, nincs írás.
- **Félig kész állapot nincs:** a kiadás, a visszavétel, a karbantartás megkezdése és lezárása egy tranzakcióban lépteti
  az állapotot és írja a sort; hiba esetén visszagörgetés, az eszköz állapota nem tér el a nyitott kiadástól
  (invariáns: `status = checked_out` ⇔ van nyitott kiadás; a **konzisztencia-vizsgálat** `backlot.equipment.verify`
  ezt ellenőrzi és jelenti, nem javít csendben).
- A törölt/visszavont személyre vagy helyszínre mutató kiadás megjeleníthető (nincs árva sor, jelölt azonosító).
- Csoportfoglalásnál egy tag hibája az **egész** műveletet visszagörgeti (nincs félig lefoglalt csomag), és megnevezi a
  tagot.
- A késő visszavétel és az esedékesség időzítő hibája (G2-12) nem érinti az adatot; a jelzés pótlódik a következő
  futáskor.

## 9. Sorrend és függés

| Rész | Kell hozzá |
|---|---|
| Katalógus, eszköz, csoport, tárolási hely, foglalás, ütközés | 2., 3., 5. lépcső; Workflow Engine (G2-04) az állapothoz |
| Kiadás, visszavétel, karbantartás | G2-04; a kiadási cél a 18. (személy) és a 20. lépcső (helyszín) |
| Dokumentumok | 14. lépcső |
| Naptárrétegek | 17. lépcső |
| Esedékesség- és késés-jelzés | 12. lépcső |

Ezek a részek a 15. lépcső terhelésszámításától **függetlenek** (2.1), így a 21. lépcső külön, a 15. nélkül is
elindítható; a G1 „Előfeltétel: 15. lépcső” sora ezzel megszűnik (a lépcsősor sorrendje marad).

## 10. Tesztek (G5-re előre)

- **Állapotgép:** minden megengedett és minden tiltott átmenet; a kézi `checked_out`-ba lépés elutasítva; az invariáns
  (`checked_out` ⇔ nyitott kiadás) property-teszt véletlen parancssorozatra; a `verify` hibát jelent kézzel rontott
  adatnál.
- **Foglalás és ütközés:** minden jelzés (`confirmed`–`confirmed` hiba, `requested` figyelmeztetés; állapot; karbantartás;
  bérleti időszak; még kiadva; forgatási nap eltérés); **két produkció** ütközése, a másik produkció részletének
  elrejtése nem tagnak; az ütközés nem tiltja a mentést.
- **Csoport:** csoportfoglalás tagonként, egy tag hibája mindent visszagörget, a tagság változása a meglévő foglalást
  nem módosítja (`kit_membership_changed`), egy eszköz csak egy aktív csoportban.
- **Kiadás és visszavétel:** sikeres kör, csoportos kiadás, kettős kiadás elutasítása, sérült visszavétel +
  `plan_repair`, hiánylista, nem azonos produkciójú helyszín elutasítva, hibainjektálás a tranzakció közepén.
- **Karbantartás:** indítás/lezárás állapotváltással, esedékesség fajtánként a legutóbbi `done` rekordból,
  `maintenance_due` jelzés, `done` rekord `performed_on` nélkül elutasítva.
- **Pénznem és díj:** a tényadat nem számolódik; a `cost_minor` és a `currency_code` együtt; a díj mező rejtve nem
  `producer`/`owner` szerepnek.
- **Hozzáférés:** minden szerep minden adatra; a nem tag produkció foglalása csak „foglalt”.
- **Anonimizálás:** az átvevő törlése után a kiadás megmarad, név nincs, a hivatkozásjelentés kiírja.
- **Események, naptár:** minden esemény és jelzés; a rétegek a forrásból számolnak.
- **Never crash:** hibás bemenetek fuzzolása (időszak, specs, állapot, csoport).
- **Felületi ellenőrzés:** `ui.screenshot` az eszközlistáról, a kiadási képernyőről és a foglalási naptárról ütközéssel.
- **Mérés** a CFA4-MIN gépen: 20 000 eszköz, 200 000 foglalás, 100 000 kiadás; lista, elérhetőség- és ütközésszámítás
  ideje.

## 11. Licenc (G3-ra előre)

Új függőség nincs. A Snipe-IT (AGPL-3.0) és a bérlőcégek rendszerei (Rentman, Flex, Current RMS) **csak
funkcionális referencia**, kódot nem veszünk át. A PDF `QPdfWriter` (PySide6), a CSV a standard `csv` modul.

## 12. G2-eredmény

**Javaslat: PASS.** Új döntési kérdés nincs; két G1-től eltérő pont indokolva (2.1: nem `resource`, 2.2: saját
tárolási hely). A megvalósítás a 3., 5. lépcső és a Workflow Engine után indulhat, a 15. lépcső nélkül; a kiadási cél
a 18. és a 20., a dokumentumok a 14., a naptárrétegek a 17., az időzített jelzések a 12. lépcsőtől működnek. A G2-12
13. pontjának bővítése (események, időzített jelzések) és a G2-15 2.1 pontosítása megvalósításkor kerül oda, új tábla
nélkül.
