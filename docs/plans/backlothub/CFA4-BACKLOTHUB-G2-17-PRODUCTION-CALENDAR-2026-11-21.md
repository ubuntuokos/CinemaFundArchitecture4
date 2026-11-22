# BacklotHub 17. lépcső – Production Calendar: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-21 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.13 · **Előzmény:**
[G1 Production Calendar](CFA4-BACKLOTHUB-G1-17-PRODUCTION-CALENDAR-2026-11-07.md) (funkcionális követelmények
jóváhagyva 2026-11-10) · **Épül rá:** G2-02 (mérföldkő), G2-03 (elem), G2-05 (feladat, RRULE-szabály, közös
`subject_kind` regiszter), G2-14 (`subject_kind` regiszter), G2-15 (munkanaptár, szabadság, ünnepnap),
G2-12 (események) · **Állapot:** G2-javaslat, jóváhagyásra vár · implementáció nincs

Új döntési kérdés nincs. Új függőség az alap lépcsőben (17-1) nincs; az `.ics` export és import
(`icalendar`, BSD-2-Clause) külön allépcső (17-2), amely a G3 kapuja után indul (9. pont).

## 1. Hatókör ebben a lépcsőben

- **A naptár nézet, nem új igazságforrás** (G1): a határidő a feladatban, a mérföldkő a produkcióban, a szabadság
  és az ünnepnap a munkanaptárban él; a naptár ezeket egy időtengelyen mutatja, **rétegekként**, és szerkesztéskor
  a forrás objektumot módosítja.
- **Saját adat csak kettő:** **naptári esemény** (megbeszélés, csapatesemény; ismétlődéssel, résztvevőkkel) és
  **forgatási nap**, amíg a Shooting Schedule (23.) el nem készül, és át nem veszi.
- **Nézetek:** nap, hét, hónap, idővonal erőforrásonként. **Ütközésjelzés.**
- **Nem része:** élő szinkron külső naptárral (CalDAV, Google; kifelé küldene adatot, SEC-02), esemény-
  emlékeztető (a G1 nem kérte; értesítési szabályként később, igény szerint), meghívás-visszaigazolás
  (RSVP; M1-ben egy felhasználó van), erőforrás-foglalás eszközre és helyiségre (21. lépcső), forgatási terv
  részletei (23.).

## 2. Integrációs határok

| Határ | Irány | Szerződés |
|---|---|---|
| Production (2.) | ← | `milestone` (név, `due_date`, `done_at`); a mérföldkő marad a forrás |
| Task (5.) | ← | `task`: `start_date`, `due_date`, állapotkategória, kiosztás; szerkesztés a `backlot.task.update`-en át |
| Document (14.) | ← | A dokumentum `valid_until` lejárata a „lejáratok” rétegben (csak megjelenítés) |
| Resource Planning (15.) | ← | Munkanaptár és kivételek (ünnepnap, szabadság) a produkció alapnaptárából és az erőforrásokból; a terhelés-túllépés jelzése |
| Structure (3.), közös `subject_kind` regiszter (G2-05, G2-14) | ← | Az esemény kapcsolt objektuma (jelenet, feladat, később helyszín) |
| Search (7.) | → | `SearchProvider`: esemény (cím, leírás, hely), forgatási nap (címke) |
| Notification (12.) | → | `backlot.calendar.event_invited`, `.event_changed`, `.event_cancelled` (6. pont) |
| Audit (1.) | → | Minden írás auditált |
| Shooting Schedule (23.) | később | Átveszi és kibővíti a `shooting_day` táblát, ugyanazzal az azonosítóval (5.2) |
| Külső naptár | később | Csak `.ics` fájlcsere (17-2), élő szinkron kifejezett beállítással, külön döntéssel |

**Falióra-idő, nem médiaidő:** a naptár UTC + IANA-időzóna alapú, a `cfa4_time::Tick` nem része.

## 3. Rétegek (a nézet adatforrásai)

A **réteg** egy kódban regisztrált szolgáltató (zárt lista, mint a Dashboard csempéi, G2-06): azonosító, címke,
`range(production, from, to, szűrők)` → elemek. Az elem **másolat nélkül** a forrás sorára mutat, és megmondja,
hogyan szerkeszthető (melyik parancsot hívja a húzás).

| Réteg | Forrás | Elem | Szerkesztés (a forrás parancsa) |
|---|---|---|---|
| `deadlines` | `task.due_date` (és a kezdőnap, ha van) | egész napos jelölés vagy sáv | `backlot.task.update` |
| `milestones` | `milestone.due_date` | egész napos jelölés | `backlot.production.milestone.update` |
| `expiries` | `document.valid_until` (G2-14) | egész napos jelölés | – (csak megjelenítés; a lejárat a dokumentumon szerkeszthető) |
| `assignments` | a kiosztott, dátumozott nyitott feladatok ablaka erőforrásonként | sáv az idővonalon | `backlot.task.update` |
| `unavailability` | `calendar_exception` (ünnepnap a produkció naptárából, szabadság az erőforrásokból) | egész napos sáv | `backlot.calendar.exception.*` (G2-15) |
| `events` | `calendar_event` (saját) | idős vagy egész napos elem | `backlot.calendar.event.*` |
| `shooting_days` | `shooting_day` (saját) | egész napos elem | `backlot.calendar.shooting_day.*` |

- **„Erőforrás-foglalás” a G1-ben:** a G2-15-ben **nincs külön foglalás-objektum**, a tervezés a feladatkiosztásból és
  a szabadságból áll. A G1 „erőforrás-foglalások” rétege ezért az **`assignments`** (egy kiosztott feladat időablaka
  az erőforrásnál), ami a Gantt erőforrásnézetének naptári megfelelője. Az eszköz- és helyiségfoglalás a 21. lépcső saját
  rétege lesz, akkor regisztrálódik; addig nincs, és nem készül helyettesítő.
- Minden réteg **ki- és bekapcsolható** a felületen (a beállítás felhasználói, `backlot.calendar.layers` adja a
  listát). Egy réteg hibája csak azt a réteget jelöli hibásnak, a többi megjelenik (G2-06 mintája).
- **Hozzáférés a szolgáltatóban:** a réteg csak azt adja, amit a felhasználó látna a forrás nézetében (produkciós
  tagság, #42; a szabadság felirata mások számára rejtett, G2-15), nem utólag szűrve.
- **Egy igazság:** a naptár nem tárol rétegadatot és nem cache-el másolatot, a számok és elemek a forrásból jönnek.
  A húzás a forrás parancsát hívja a saját szabályaival és őrfeltételeivel (pl. a G2-05 dátumellenőrzése, a
  függőségsértés jelzése, G2-15); a parancs elutasítása a húzás visszavonása hibajelzéssel.
- **Tartományhatár:** legfeljebb 366 nap és 20 000 elem lekérdezésenként (`CALENDAR_RANGE_TOO_LARGE`), a felület
  lapozva vagy nagyítással tölt.

## 4. Naptári esemény

### 4.1 Idő és időzóna

- Az esemény **saját időzónával** rendelkezik (IANA-név, alapból a rendszer időzónája a létrehozáskor, kifejezetten
  tárolva, hogy a gép időzónájának változása ne mozgassa az eseményt). Egy külföldi forgatás eseménye a helyszín
  zónájában adható meg; a felület a nézet zónájától eltérő eseménynél kiírja az esemény zónáját.
- **Tárolás helyi időben + zóna:** az esemény `start_local`/`end_local` (időzóna nélküli időbélyeg) és a `tz`; az
  UTC-érték számolt (`zoneinfo`). Így az ismétlődő esemény **helyi időben marad** nyári/téli átállás után is
  (a heti 9:00-s megbeszélés mindig 9:00). **Egész napos** esemény: `all_day`, a helyi idő éjfél, a vége a következő
  nap éjfélig (kizáró); a nézet a dátumot mutatja, UTC-átváltás nélkül.
- **Nem létező helyi idő** (az átállás kihagyott órája) a következő létező időre kerül, **kétértelmű idő** (az
  ismétlődő óra) az első előfordulásra; a szabály a `zoneinfo` `fold` kezelésén áll, tesztelve (11. pont).

### 4.2 Adatmodell (DDL-vázlat, `backlot` séma)

```sql
CREATE TABLE backlot.calendar_event (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    title text NOT NULL CHECK (length(title) BETWEEN 1 AND 300),
    description text CHECK (length(description) <= 20000),     -- egyszerű Markdown, HTML nélkül
    location_text text CHECK (length(location_text) <= 500),   -- szabad szöveg; a helyszín-objektumot a link adja (20. lépcsőtől)
    tz text NOT NULL,                                          -- IANA; ismeretlen zónára CALENDAR_TIMEZONE_UNKNOWN
    all_day boolean NOT NULL DEFAULT false,
    start_local timestamp NOT NULL, end_local timestamp NOT NULL,
    busy boolean NOT NULL DEFAULT true,                        -- ütközés számít-e a résztvevőknek
    rrule text,                                                -- a G2-05 zárt RRULE-részhalmaza; NULL = egyszeri
    series_split_from_id uuid REFERENCES backlot.calendar_event(id),   -- „ettől kezdve” módosításnál az előző sorozat
    external_uid text,                                         -- 17-2 importhoz
    first_start_utc timestamptz NOT NULL,                      -- előszűréshez, a szolgáltatás tölti
    last_end_utc timestamptz,                                  -- NULL = nem korlátos sorozat
    retired_at timestamptz,
    ...közös oszlopok,
    CHECK (end_local > start_local),
    CHECK (NOT all_day OR (start_local::time = '00:00' AND end_local::time = '00:00')),
    UNIQUE (production_id, external_uid)
);
CREATE INDEX ON backlot.calendar_event (production_id, first_start_utc);

CREATE TABLE backlot.calendar_event_override (                  -- az ismétlődő sorozat egy előfordulásának eltérése
    event_id uuid NOT NULL REFERENCES backlot.calendar_event(id) ON DELETE CASCADE,
    occurrence_local timestamp NOT NULL,                         -- az eredeti előfordulás helyi kezdete
    kind text NOT NULL CHECK (kind IN ('cancelled','modified')),
    title text, location_text text,
    start_local timestamp, end_local timestamp,                  -- modified: az új idő; NULL = változatlan
    PRIMARY KEY (event_id, occurrence_local),
    CHECK (kind = 'modified' OR (title IS NULL AND location_text IS NULL AND start_local IS NULL AND end_local IS NULL))
);

CREATE TABLE backlot.calendar_event_participant (
    event_id uuid NOT NULL REFERENCES backlot.calendar_event(id) ON DELETE CASCADE,
    person_id uuid NOT NULL REFERENCES backlot.person(id),
    PRIMARY KEY (event_id, person_id)
);

CREATE TABLE backlot.calendar_event_link (                       -- kapcsolt objektum, a közös subject_kind regiszterrel
    event_id uuid NOT NULL REFERENCES backlot.calendar_event(id) ON DELETE CASCADE,
    production_id uuid NOT NULL REFERENCES backlot.production(id),
    subject_kind text NOT NULL, subject_id uuid NOT NULL,
    PRIMARY KEY (event_id, subject_kind, subject_id)
);
```

- **Résztvevő a `person`, nem a `resource`:** egy résztvevő nem feltétlenül erőforrás (pl. külső partner szerepe
  később a Crew/Cast után). Az ütközés a `resource.person_id`-n át, azoknál a résztvevőknél értékelhető, akiknek van
  erőforrás-sora (G2-15); a többinél csak az esemény-átfedés (4.4).
- **Kapcsolt objektum:** ugyanaz a zárt, kódban regisztrált `subject_kind` lista és feloldó, mint a G2-05
  megjegyzéseinél és a G2-14 dokumentumkapcsolatainál (egy regiszter, három használó); ismeretlen típus
  `CALENDAR_LINK_KIND_UNKNOWN` (a helyszín, szereplő, stábtag a 18–20. lépcsőtől, nincs helyettesítő tábla).
- **Egyetlen hely az idő számolására:** az ismétlődés kibontása és az UTC-konverzió egy Python-modul (`dateutil.rrule` a
  helyi, időzóna nélküli időn; utána `zoneinfo` az UTC-hez); a nézetek, az ütközésjelzés és az export (17-2) ugyanazt
  hívja.

### 4.3 Ismétlődés

- **Szabály:** a G2-05 zárt RRULE-részhalmaza (`FREQ` `DAILY`, `WEEKLY`, `MONTHLY`, `YEARLY`; `INTERVAL`, `BYDAY`,
  `BYMONTHDAY`, `COUNT`, `UNTIL`), a task-ismétlődés validátorával (`RECURRENCE_RULE_INVALID`). A szabályt itt **helyi idejű
  kezdéshez** (`start_local`) kötjük, a task-ismétlődés dátumpontosságával szemben.
- **Kibontás lekérdezéskor, nem példányosítva:** az esemény könnyű adat (nem objektum, amelyre más hivatkozik), ezért
  az előfordulások a kért tartományra számolódnak (`first_start_utc`/`last_end_utc` előszűréssel); nincs háttérszolgáltatás
  és nincs materializált sor. Korlát: legfeljebb 2000 előfordulás eseményenként és lekérdezésenként.
- **Egy előfordulás módosítása vagy törlése:** `calendar_event_override` sor (az eredeti előfordulás helyi
  kezdetével kulcsolva).
- **„Ettől kezdve” módosítás** (`update_scope: 'following'`): a régi sorozat `UNTIL`-lal véget ér az előfordulás előtt,
  és új esemény indul a módosított adatokkal (`series_split_from_id` az előzőre), egy tranzakcióban; az új
  esemény résztvevői és kapcsolatai másolódnak. **`update_scope: 'all'`** a teljes sorozatot, **`'occurrence'`** egy
  előfordulást érint.
- **Törlés:** az esemény `retired_at`-tal visszavonható (kimarad a nézetből, az audit őrzi); fizikai törlés nincs.

### 4.4 Ütközések

Számolt jelzés, **lekérdezéskor** (nincs tárolt jelző); az ütközés **nem tilt**, a felhasználó mentheti, a rendszer jelez.

| Jelzés | Feltétel | Súly |
|---|---|---|
| `event_overlap` | Egy résztvevő két `busy` eseményén átfedés (idős–idős átfedése; egész napos `busy` esemény ugyanazon a napon bármely másik `busy` eseménnyel) | figyelmeztetés |
| `participant_on_leave` | Résztvevő az esemény (vagy előfordulás) napján szabadságon van (G2-15 `leave`/`other_off`) | figyelmeztetés |
| `shooting_on_holiday` | Forgatási nap a produkció alapnaptárának ünnepnapján (hétvége nem jelez: a forgatás gyakran hétvégére esik) | figyelmeztetés |
| `participant_overload_day` | Résztvevő aznap túlterhelt (G2-15 `overload`) — **tájékoztató**, az esemény idejét nem számítja a kapacitásba | információ |

- A jelzést **szerkesztéskor azonnal** visszaadja az esemény írása (a válasz tartalmazza), és a nézet is mutatja.
  **Aszinkron ütközés-értesítés nincs M1-ben** (pl. ha utólag felvett szabadság okoz ütközést, az a nézetben és a
  `backlot.calendar.conflicts` listában látszik, értesítést nem küld): a G1 az értesítést „a 12. lépcsőtől”
  kérte, de az esemény-meghívás értesítése (6. pont) a lényegi eset; a további jelzések szabályként később adhatók.
- Az esemény **kapacitásba nem számít**: a G2-15 terhelése a feladatokból jön; a megbeszélés ideje a
  `time_entry`-ben (16.) jelenik meg, ha rögzítik. Ez szándékos, hogy a két nézet ne számolja kétszer ugyanazt.

## 5. Forgatási nap

### 5.1 Adatmodell

```sql
CREATE TABLE backlot.shooting_day (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    shoot_date date NOT NULL,
    unit text NOT NULL DEFAULT '1' CHECK (length(unit) BETWEEN 1 AND 40),   -- egység: „1”, „2. egység”, …
    label text CHECK (length(label) <= 200),
    note text CHECK (length(note) <= 5000),
    retired_at timestamptz,
    ...közös oszlopok,
    UNIQUE (production_id, shoot_date, unit)
);
```

- A forgatási nap a G1 szerint **átmeneti**: dátum, egység, megjegyzés. Semmi más (híváslista, jelenetlista, idők) nincs,
  mert az a Shooting Schedule (23.) feladata.
- **Átadás a 23. lépcsőnek:** a 23. G2-je a **`shooting_day` táblát bővíti** új oszlopokkal és kapcsolatokkal, az
  azonosítók változatlanok (G1 követelmény), a 17-ben felvett napok megmaradnak. A 17. lépcsőben a tábla már
  **teljes értékű**: a naptár rétege, a keresés és az ütközésjelzés használja; nem helyettesítő.
- **A 23. lépcső kiegészítése** ([G2-23](CFA4-BACKLOTHUB-G2-23-SHOOTING-SCHEDULE-2026-11-21.md) 4.1): `call_time`, `target_duration_minutes` oszlop; a visszavonás nem engedett, ha a napnak csíkjai vannak (`SCHEDULE_DAY_HAS_STRIPS`), kivéve a csíkok ütemezetlenné tételével; új `subject_kind`: `shooting_day`.
- Visszavonás `retired_at`-tal; ugyanarra a (dátum, egység) párra újrafelvétel a visszavont sort élesíti, nem duplikál.

## 6. Parancsok, események, hibakódok

| Parancs | Mit csinál |
|---|---|
| `backlot.calendar.layers` | A rétegek listája (és a felhasználó beállítása) |
| `backlot.calendar.range` | Elemek egy tartományra rétegenként (nap/hét/hónap és idővonal adata) |
| `backlot.calendar.conflicts` | Ütközések egy tartományra (4.4) |
| `backlot.calendar.event.create` / `.update` (`update_scope`) / `.cancel_occurrence` / `.retire` / `.get` / `.list` | Események és ismétlődés |
| `backlot.calendar.event.participant.add` / `.remove` | Résztvevők |
| `backlot.calendar.event.link.add` / `.remove` | Kapcsolt objektumok |
| `backlot.calendar.shooting_day.create` / `.update` / `.retire` / `.list` | Forgatási napok |
| `backlot.calendar.export` / `.import` | `.ics` (17-2, 9. pont) |

**Események** (a G2-12 `backlot` domainjében, outbox, kritikus): `backlot.calendar.event_invited` (résztvevő
felvétele vagy az esemény létrehozása résztvevőkkel), `backlot.calendar.event_changed` (az idő, az időzóna vagy
a hely módosult; egy előfordulás módosítása is), `backlot.calendar.event_cancelled` (visszavonás vagy egy
előfordulás lemondása). **Címzett:** az esemény résztvevői, az eseményt módosító személy kivételével (új szabályelem a
G2-12 zárt címzettlistájában: `event_participant`). A payload azonosítókat, az esemény kezdetét és az előfordulás
helyi kezdetét viszi, **címet, leírást és helyszín-szöveget nem** (a fogyasztó a hozzáférési szabállyal olvassa).

Hibakódok: `CALENDAR_EVENT_NOT_FOUND`, `CALENDAR_EVENT_INVALID`, `CALENDAR_TIMEZONE_UNKNOWN`,
`CALENDAR_LINK_KIND_UNKNOWN`, `CALENDAR_OCCURRENCE_NOT_FOUND`, `CALENDAR_RANGE_TOO_LARGE`,
`CALENDAR_LAYER_FAILED`, `SHOOTING_DAY_EXISTS`, `SHOOTING_DAY_INVALID`, valamint a `RECURRENCE_RULE_INVALID`,
`TASK_*`, `WF_*` kódok változatlanul.

## 7. Nézetek és felület

- **Qt `QPainter`-rajzolt egyedi vezérlők** (mint a Dashboard csempéi és a Gantt, G2-06, G2-15; nincs új
  grafikai függőség): nap/hét (időtengellyel), hónap (rács), idővonal erőforrásonként (sorok az erőforrások,
  sávok az `assignments`, `unavailability` és az események). A mai nap jelölése, a réteg-szűrő, az ütközések jelölése.
- **Szerkesztés:** húzás (áthelyezés), élhúzás (hosszváltoztatás), duplakattintás a forrás szerkesztőjére; a húzás a
  forrás parancsát hívja és megerősítés előtt **előnézetet** ad az ütközésekről (4.4), a forrás szabályai szerint.
- **Billentyűzet és automatizálás:** minden vezérlő automatizálási azonosítót kap, a nézet állapota (tartomány, rétegek)
  serde-állapot, így a vezérlőcsatorna olvassa és állítja (CFA4 „agent-drivable” szabály); a `calendar.range` az
  adatparancs, a felület nem tud olyat, amit a parancs nem.
- A nézet beállításai (melyik réteg, melyik tartomány) **felhasználói konvenció**, nem üzleti adat; nem auditált.

## 8. Never crash

- Hibás szabály (`RRULE`), ismeretlen időzóna, nem létező helyi idő (átállás), nagy tartomány, hibás kapcsolt objektum: kezelt
  hiba vagy jelzett kihagyás, az érintett elem azonosítóval listázódik; a naptár nem omlik össze.
- Egy réteg szolgáltatójának hibája csak azt a réteget jelöli hibásnak (`CALENDAR_LAYER_FAILED`).
- Határtalan ismétlődés (`COUNT` és `UNTIL` nélkül) lusta kibontással, a tartomány és a 2000-es előfordulás-korlát
  védi; a korlát elérése jelzett csonkolás, nem időtúllépés.
- A húzás elbukó szerkesztése: a vizuális változás visszaáll, hibajelzéssel; az adatbázis változatlan.

## 9. `.ics` export és import (17-2, icalendar)

- Az **`.ics`** (RFC 5545) a G1 szerint a fájlcsere formátuma; **új Python-függőség** (`icalendar`, BSD-2-Clause, Ubuntu:
  `python3-icalendar`, a 26.04-es elérhetőség a G3-on ellenőrizendő, tranzitív függőségekkel és `--require-hashes`
  rögzítéssel), ezért **külön allépcső**, amely a G3 után indul. A 17-1 (rétegek, események, forgatási napok,
  ütközések) függőség nélkül teljes; az export- és importgomb addig **nincs a felületen** (nem stub).
- **Export** (`backlot.calendar.export`): a tartományra és a kiválasztott rétegekre (események, forgatási napok,
  mérföldkövek, határidők) `VEVENT`-ek, időzónával (`TZID` IANA-névvel, `VTIMEZONE` az `icalendar` képességei
  szerint); az ismétlődés `RRULE`-lal, a kivételek `EXDATE`/`RECURRENCE-ID`-vel; az `UID` stabil
  (`<azonosító>@backlot.cfa4.invalid`). Az export a hozzáférési szabállyal szűrt, a művelet auditált
  (`calendar.exported`, tartalom nélkül); a fájlt a felhasználó által megadott helyre írja.
- **Import** (`backlot.calendar.import`): **csak naptári eseményt** hoz létre (nem feladatot, mérföldkövet, forgatási
  napot), a felhasználó által megadott produkcióba; **előnézet** a mentés előtt (hány esemény, mi marad ki).
  - **Védelem a megbízhatatlan fájl ellen:** legfeljebb 5 MiB és 5000 `VEVENT`; csak a `SUMMARY`, `DESCRIPTION`,
    `LOCATION`, `DTSTART`, `DTEND`/`DURATION`, `RRULE` (a G2-05 részhalmazára validálva), `EXDATE`, `UID` mező kerül át; a
    `ATTACH`, `URL`, `ORGANIZER`, `ATTENDEE` (e-mail-cím = személyes adat), `CONFERENCE` és minden egyéb **elmarad**, és az
    importjelentés megszámolja. **Hálózati letöltés nincs** (az `ATTACH`/`URL` nem követhető, SEC-02).
  - **Időzóna:** csak IANA-nevű `TZID` fogadható el; egyedi `VTIMEZONE`-ra épülő vagy ismeretlen zóna esetén az
    esemény kimarad `CALENDAR_ICS_TIMEZONE_UNKNOWN`-nal a jelentésben; „lebegő” idő a felhasználó választotta zónába kerül.
  - **Ismétlődés, amelyet nem ismerünk:** az esemény egyszeri előfordulásként kerül be, figyelmeztetéssel (nem tévesen
    bontjuk ki).
  - **Azonosság:** az `UID` az `external_uid`-ba kerül; ismételt importnál a meglévő `UID` alapértelmezetten
    **kimarad** (jelezve), `overwrite` kapcsolóval frissül (auditált).
  - Hibás fájl: `CALENDAR_ICS_INVALID`, kezelt hiba, részleges írás nincs (egy tranzakció).

## 10. Sorrend és függés

| Rész | Kell hozzá |
|---|---|
| 17-1: rétegek, események, forgatási napok, ütközések, nézetek | 2., 3., 5. lépcső; a szabadság, ünnepnap és munkanaptár rétegek a 15. lépcsőtől; a `expiries` réteg a 14. lépcsőtől (addig nincs regisztrálva) |
| Esemény-értesítések | 12. lépcső, EF-R1 |
| Helyszín, szereplő, stábtag kapcsolat | 18., 19., 20. lépcső (feloldó regisztrálása) |
| 17-2: `.ics` | G3 (`icalendar`) |
| Forgatási nap bővítése | 23. lépcső |

A rétegek **regisztrációja a megvalósított modulokhoz kötött**: egy modul rétege akkor kerül a regiszterbe, amikor a
modul megvan, így nincs üres vagy helyettesítő réteg (a G2-06 csempe-regiszter elve).

## 11. Tesztek (G5-re előre)

- **Idő és DST:** heti 9:00 helyi idejű esemény mindkét átállás után 9:00; havi „utolsó péntek”; nem létező és
  kétértelmű helyi idő; egész napos esemény átállási napon; külföldi zónájú esemény más nézetzónában; az
  UTC-előszűrés (`first_start_utc`/`last_end_utc`) **egyezik a nyers, teljes kibontással** (property-teszt
  véletlen szabályokra és tartományokra).
- **Ismétlődés:** `occurrence`, `following`, `all` módosítás és lemondás; a megosztás utáni két sorozat lefedi az eredetit
  átfedés és hézag nélkül; `UNTIL`, `COUNT`, határtalan szabály; 2000-es korlát; hibás szabály.
- **Rétegek:** minden réteg a forrásból számol (nincs eltérő másolat), a hozzáférési szűrő a szolgáltatóban él (nem
  tag produkció és rejtett szabadság-felirat nem szivárog); egy réteg hibája nem viszi el a többit.
- **Húzás:** a határidő és a mérföldkő húzása a forrás parancsát hívja, az elutasítás (dátumsértés, `WF_*`)
  visszaállítja; a függőségsértés a G2-15 jelzésével egyezik.
- **Ütközések:** átfedés, egész napos `busy`, szabadság alatt résztvevő, forgatási nap ünnepnapon, hétvégén nem; az
  ütközés nem tiltja a mentést.
- **Események:** meghívás, változás, lemondás egy-egy eseményt ad, a módosító nem kap értesítést; replay nem küld újat
  (G2-12 szabálya).
- **Forgatási nap:** egyediség (dátum, egység), visszavonás utáni újrafelvétel, átadás a 23. lépcsőnek (azonosító
  változatlan, séma-kompatibilitási teszt a 23. G2-jén).
- **Never crash:** hibás RRULE, ismeretlen zóna, óriás tartomány, hibás réteg, határtalan szabály.
- **17-2:** `.ics` import fuzz (csonkolt, óriás, rosszindulatú `ATTACH`/`URL`, ismeretlen zóna, ismeretlen `RRULE`,
  duplikált `UID`), export-import körút (az esemény újra azonos időpontokat ad), a fájl nem tölt le semmit.
- **Felületi ellenőrzés:** `ui.screenshot` a hónap-, hét- és idővonal-nézetről rétegekkel és ütközésjelzéssel.
- **Mérés** a CFA4-MIN gépen: 366 napos tartomány 20 000 elemmel és 200 ismétlődő eseménnyel, a nézet váltása és a
  húzás utáni újraszámolás ideje (a G2-07 S és L profiljának mintájára).

## 12. Licenc (G3-ra előre)

- **17-1: új függőség nincs.** `python-dateutil` (G2-05, RRULE) és `zoneinfo` (Python standard könyvtár; az
  IANA-adatbázis a rendszer `tzdata`-ja vagy a `tzdata` csomag, a G3-on ellenőrizendő, hogy a Python 3.14-es
  környezet melyiket használja).
- **17-2: `icalendar`** (`collective/icalendar`, BSD-2-Clause; Ubuntu `python3-icalendar`) és tranzitív függőségei,
  a G3-on: verzió, Python 3.14, `--require-hashes`.
- **Referencia, kód nélkül:** Kitsu/Zou (AGPL-3.0: mérföldkő, szabadnap, ütemtervelem). A `holidays` csomag a
  15. lépcsőben érkezik (G2-15), itt nem kell újra.

## 13. G2-eredmény

**Javaslat: PASS.** Új döntési kérdés nincs. A megvalósítás az 5. lépcső után indulhat; a rétegek a hozzájuk tartozó
modulokkal együtt kapcsolnak be, az `.ics` a G3 után. A G2-12 zárt címzettlistája az `event_participant`
elemmel (és a korábbi `resource_person`, `production_approver` elemekkel) bővül megvalósításkor, új tábla nélkül.
