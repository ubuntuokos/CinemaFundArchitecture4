# BacklotHub 23. lépcső – Shooting Schedule: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-21 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.14 · **Előzmény:**
[G1 Shooting Schedule](CFA4-BACKLOTHUB-G1-23-SHOOTING-SCHEDULE-2026-11-07.md) (funkcionális követelmények jóváhagyva
2026-11-10) · **Épül rá:** G2-04 (Workflow Engine), G2-15 (munkanaptár, szabadság), G2-17 (`shooting_day`, naptár),
G2-18 (stáb), G2-19 (szereplő, behívás), G2-20 (helyszín, foglalás, korlátozás), G2-21 (eszközfoglalás), G2-22 (jelenet,
elemek) · **Állapot:** G2-javaslat, jóváhagyásra vár · implementáció nincs

Új döntési kérdés nincs, **új függőség nincs** az M1-ben. Az **automatikus sorrendjavaslat** (OR-Tools) a G1-nek megfelelően
**külön al-lépcső (23-2)**, a G2-15 8. pontjával azonos függőségi döntéssel (3. pont). Négy pont tér el a G1-től, mind
indokolva (a 12. pontban összegezve): nincs külön „ütemterv” tároló, a **változat pillanatkép**, a **munkajogi
szabályokat a rendszer nem értékeli**, és a **hipotetikus változat (scenario) a 23-2-vel jön**.

## 1. Hatókör ebben a lépcsőben

- **Forgatási nap** (a G2-17 tábla bővítve): hívási idő, célzott időtartam, hívások.
- **Stripboard:** a jelenetek (és jelenetrészek) csíkjai forgatási napokra osztva, napon belüli sorrenddel, bannerekkel.
- **Ütközésvizsgálat** minden módosításra: szereplő, stáb, helyszín, eszköz, napszak, kiskorú, szerződött napok.
  Figyelmeztet, nem tilt; a **tudomásulvétel** (felülbírálás) auditált.
- **Behívások**: szereplői és stábhívások, hívási idők; a G2-19 `cast_call` táblájának bővítése.
- **Day out of days** szereplőnként és stábtagonként.
- **Napi diszpozíció (call sheet)** PDF-ben, a személyes elérhetőség kifejezett választással.
- **Változatok (revíziók)**: kiadott pillanatképek revíziószínnel, összevetés, érintettek.
- **Nem része:** automatikus sorrendjavaslat és hipotetikus változat (23-2); a **tényleges forgatási előrehaladás**
  követése (mi lett ténylegesen leforgatva: nincs a G1-ben); munkajogi szabályok (pihenőidő, túlóra, hétnapos
  munkahét, gyerekmunka-korlát) értékelése (7.5); jelmez-, smink- és kellék-ütemezés; költség (24.).

## 2. Integrációs határok

| Határ | Irány | Szerződés |
|---|---|---|
| Calendar (17.) | ↔ | A **`shooting_day` tábla bővül** (4.1), azonosítók változatlanok; a `shooting_days` réteg kiegészül a hívási idővel; új `subject_kind`: `shooting_day` |
| Breakdown (22.) | ← | A jelenet (`scene_meta`: INT/EXT, napszak, oldalnyolcad), az elemhasználat (diszpozíció „különleges igények”) |
| Cast (19.) | ↔ | `casting` kapcsolat → karakter → `cast_assignment` (`principal`) → személy; `cast_call` bővül (6. pont); `cast_engagement` a napszámhoz |
| Crew (18.) | ← | `crew_assignment` érvényessége, részleg; a stáb hívásai (4.2) |
| Location (20.) | ← | `setting` → `set` → `location_set_link` (`selected`) → `location`; `location_booking` (`shooting_day_id`), `location_restriction` (ugyanaz az értékelő, G2-20 4.2) |
| Equipment (21.) | ← | A napra/jelenetre kötött `equipment_booking` ütközései (`backlot.equipment.conflicts`) |
| Resource (15.) | ← | Munkanaptár és szabadság (`resource_day_capacity`, `calendar_exception`), ha az érintettnek van `resource`-sora |
| Workflow Engine (4.) | → | A revízió folyamata (`schedule_revision`, 4.3) |
| Document (14.) | → | A diszpozíció `call_sheet` típusú dokumentum, naponta egy, kiadásonként új verzió |
| File Services (ADR 0009) | → | A diszpozíció és az ütemterv-export PDF-je az `attachment` gyökérbe (`files.import`, `files.seal`) |
| Notification (12.) | → | Események (9. pont) |
| Search (7.) | → | `SearchProvider`: forgatási nap (dátum, egység, címke) |
| Audit (1.) | → | Minden írás, a tudomásulvétel, a revízió-kiadás auditált |
| Budget (24.) | később | A forgatási napok és a behívott napok száma bemenet; összeg itt nincs |

## 3. Ami nem készül el M1-ben: automatikus ütemezés (23-2)

A G1 az automatikus sorrendjavaslatot (OR-Tools CP-SAT) „később, opcionális” képességként jelölte. **Javaslat: a 23-2
al-lépcsőbe**, ugyanazon okokból, mint a G2-15 8. pontjában: a PyPI-kerék mérete és a Python-kiadás kompatibilitása külön
G3-at kér, a célfüggvény (helyszín- és szereplőnapok) hangolása felhasználói visszajelzést; az **ütközésvizsgálat
(7. pont) a kézi tervezéshez teljes értékű**, és a 23-2 ugyanazt a vizsgálót használja célfüggvény és korlát forrásaként.
A hipotetikus változat (a G2-15 `schedule_scenario` mintájára) **szintén a 23-2-vel jön**, mert a javaslat annak az
eredménye; **addig a kézi „mi lenne, ha”** a revízió-piszkozat (4.3) és a visszaállítás (`backlot.schedule.revision.restore`).
Nincs helyettesítő, nincs üres kapcsolódási pont; a 23-2 új migrációval és külön G2-bővítéssel indul.

## 4. Adatmodell (DDL-vázlat, `backlot` séma)

### 4.1 A forgatási nap bővítése (G2-17) és a hívások

```sql
ALTER TABLE backlot.shooting_day
    ADD COLUMN call_time time,                                    -- általános hívási idő, helyi falióra-idő
    ADD COLUMN target_duration_minutes integer CHECK (target_duration_minutes BETWEEN 1 AND 1440),
    ADD CONSTRAINT shooting_day_call_pair CHECK ((call_time IS NULL) = (target_duration_minutes IS NULL));

CREATE TABLE backlot.shooting_day_call (                          -- részleg- és személyi hívások (csak az eltérések)
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    shooting_day_id uuid NOT NULL REFERENCES backlot.shooting_day(id),
    department_id uuid REFERENCES backlot.team(id),               -- kind = 'department' (trigger)
    person_id uuid REFERENCES backlot.person(id),
    called boolean NOT NULL DEFAULT true,                          -- false: aznap nincs behívva
    call_time time,                                                -- NULL: a részleg vagy a nap hívási ideje
    note text CHECK (length(note) <= 500),
    ...közös oszlopok,
    CHECK ((department_id IS NULL) <> (person_id IS NULL)),
    CHECK (department_id IS NULL OR called),                       -- részlegre nincs „nem hívott”: azt személyenként
    UNIQUE NULLS NOT DISTINCT (shooting_day_id, department_id, person_id)
);

ALTER TABLE backlot.cast_call
    ADD COLUMN call_time time, ADD COLUMN makeup_start time, ADD COLUMN wardrobe_start time,
    DROP CONSTRAINT cast_call_source_check,
    ADD CONSTRAINT cast_call_source_check CHECK (source IN ('manual','breakdown','schedule'));
```

- **A nap időkezelése:** a `call_time` és a **célzott időtartam** (a tervezett munkaidő hossza percben) **helyi falióra-idő**
  a forgatás helyén (mint a G2-20-ban: nincs UTC-konverzió, mert a nap és a helyszínkorlátozás ugyanabban a helyi
  időben van); az időtartam megadása (a „lezárási idő” helyett) a **éjfélen átnyúló éjszakai forgatást** is kifejezi.
- **Stáb hívása:** alapból a **beosztás érvényessége** dönt: akinek a `crew_assignment`-je (G2-18) a napon érvényes, az a nap
  stábja, a részleg hívási ideje: `shooting_day_call` részleg-sora, különben a nap `call_time`-ja. A **személyi sor** a kivétel:
  `called = false` (aznap nem kell), vagy eltérő hívási idő. Így a stábhívás nem tömeges adat, csak az eltérések.
- **A szereplő hívása** a `cast_call` (G2-19): a `backlot.schedule.calls.sync` a csíkokból **állítja elő** (`source =
  'schedule'`); a kézi hívást (`manual`) és a breakdownból jöttet (`breakdown`) **nem módosítja** (4.2).
- **Visszavonás:** forgatási nap, amelyen csíkok vannak, **nem vonható vissza** (`SCHEDULE_DAY_HAS_STRIPS`), kivéve a
  `move_strips_to_unscheduled: true` kifejezett kapcsolóval, amely a csíkokat egy tranzakcióban az ütemezetlenek közé
  teszi. A G2-17 `backlot.calendar.shooting_day.retire` ezt a vizsgálatot hívja (G2-17 5.2 kiegészítése).

### 4.2 Csík (stripboard) és a munkaterv

```sql
CREATE TABLE backlot.schedule_strip (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    kind text NOT NULL CHECK (kind IN ('scene','banner')),
    scene_entity_id uuid REFERENCES backlot.production_entity(id),   -- trigger: scene típus, ugyanaz a produkció
    part_label text CHECK (length(part_label) <= 10),                 -- jelenetrész: „A”, „B”, …
    banner_text text CHECK (length(banner_text) <= 200),              -- ebéd, helyszínváltás, megjegyzés
    shooting_day_id uuid REFERENCES backlot.shooting_day(id),         -- NULL: még nincs napra téve
    position integer CHECK (position > 0),
    est_minutes integer NOT NULL CHECK (est_minutes BETWEEN 0 AND 1440),
    note text CHECK (length(note) <= 1000),
    retired_at timestamptz,
    ...közös oszlopok,
    CHECK ((kind = 'scene') = (scene_entity_id IS NOT NULL)),
    CHECK ((kind = 'banner') = (banner_text IS NOT NULL)),
    CHECK ((shooting_day_id IS NULL) = (position IS NULL)),
    UNIQUE (shooting_day_id, position) DEFERRABLE INITIALLY DEFERRED
);
CREATE UNIQUE INDEX ON backlot.schedule_strip (scene_entity_id, COALESCE(part_label, ''))
    WHERE kind = 'scene' AND retired_at IS NULL;
CREATE INDEX ON backlot.schedule_strip (production_id, shooting_day_id);
```

- **A munkaterv az élő állapot**: egy produkcióhoz **egy** munkaterv tartozik (a strip a produkcióhoz és a naphoz kötött).
  A **forgatási nap egysége** (`unit`, G2-17) különíti el a több egységet; a sorozat blokkforgatása is egy tervben fér el
  (a jelenetek epizódokon átívelve ugyanazon a napon). Külön „ütemterv” tároló **nincs** (a G1 `ShootingSchedule`
  fogalma a munkatervre és a revíziókra bomlik, 4.3).
- **A jelenet adatai nem másolódnak a csíkba**: INT/EXT, napszak, oldalhossz, szereplők, helyszín, elemek a **forrásból**
  (G2-22, G2-19, G2-20) jönnek lekérdezéskor (a G1 követelménye). A csík csak a **tervezési adatot** tárolja: melyik napon, hányadik,
  hány percre. A csíkszín a jelenet INT/EXT és napszak értékéből számolt (a megjelenítésben).
- **Jelenetrész:** egy jelenet több napra osztható (`part_label`): minden rész külön csík, külön becsléssel. A teljes jelenet
  hiányzó része nem jelzés (a részek összege nincs a jelenet oldalhosszához kötve, mert a bontás tervezői döntés).
- **Banner:** szöveges csík (ebédszünet, helyszínváltás, napi megjegyzés), perccel (a napi idő része).
- **Becslés:** `est_minutes` alapértéke `perc/oldal × nyolcad / 8`, a produkció beállításából (4.4); jelenetenként és
  részenként felülírható. A becslés **tervezési segéd**, nem mérés.
- **Sorrend:** `position` a napon belül, halasztott egyediségi korláttal: az áthelyezés és az átsorolás **egy tranzakcióban**
  (`backlot.schedule.strip.move`, `.reorder`). Az **ütemezetlen** csíkoknak nincs napjuk és pozíciójuk (a megjelenítés a jelenet
  sorrendjében listázza).
- **Napon belüli idő** számolt: egy csík várt kezdete = a nap `call_time` + az előtte lévő csíkok `est_minutes` összege.
  A nap várt vége = `call_time` + az összeg; ez a napszak- és a `time_window`-vizsgálat (7. pont) alapja.
- **A munkaterv teljessége:** minden **aktív** (nem kivezetett) jelenetnek legalább egy csíkja kell legyen; ellenkező esetben
  `scene_unscheduled` jelzés (7. pont); jelenet kivezetésekor (G2-03 `retire`) a csík jelzett (`scene_retired`), nem törlődik.

### 4.3 Revízió (kiadott változat)

```sql
CREATE TABLE backlot.schedule_revision (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    number integer NOT NULL CHECK (number > 0),
    color text NOT NULL CHECK (color IN ('white','blue','pink','yellow','green','goldenrod','buff','salmon','cherry')),
    color_cycle integer NOT NULL DEFAULT 1 CHECK (color_cycle > 0),
    status_id uuid NOT NULL REFERENCES backlot.status(id),            -- G2-04: draft | published | superseded | discarded
    base_revision_id uuid REFERENCES backlot.schedule_revision(id),
    note text CHECK (length(note) <= 2000),
    snapshot_hash text NOT NULL,                                       -- sha256 a kanonikus pillanatképre
    published_at timestamptz,
    ...közös oszlopok,
    UNIQUE (production_id, number)
);
CREATE TABLE backlot.schedule_revision_day (
    revision_id uuid NOT NULL REFERENCES backlot.schedule_revision(id),
    shooting_day_id uuid NOT NULL REFERENCES backlot.shooting_day(id),
    shoot_date date NOT NULL, unit text NOT NULL,
    call_time time, target_duration_minutes integer,
    PRIMARY KEY (revision_id, shooting_day_id)
);
CREATE TABLE backlot.schedule_revision_strip (
    revision_id uuid NOT NULL REFERENCES backlot.schedule_revision(id),
    strip_id uuid NOT NULL,                                            -- a munkaterv csíkjának azonosítója
    kind text NOT NULL, scene_entity_id uuid, part_label text, banner_text text,
    shooting_day_id uuid, position integer, est_minutes integer NOT NULL,
    PRIMARY KEY (revision_id, strip_id)
);
```

- **A revízió pillanatkép:** a munkaterv csíkjai és a napok adatai a kiadás pillanatában, **változtathatatlanul** (csak
  beszúrás; a sorok nem módosíthatók, mint a G2-14 verziói). A jelenet adatai **nem** kerülnek bele (azok élő adatok);
  a kiadott diszpozíció **PDF-je** őrzi, hogyan nézett ki a kiadáskor (10.1).
- **Folyamat** `schedule_revision` (G2-04, zárt őrfeltételekkel; **a jogosultság parancsszintű**):
  `draft → published → superseded`, `draft → discarded`. A `backlot.schedule.revision.create` **piszkozatot** hoz létre a
  jelenlegi munkatervből; a `publish` egy tranzakcióban kiadja, **az előző kiadottat `superseded`-re** teszi, a
  számot és a **színt** kiosztja, és eseményt ír (9. pont). Kiadáshoz legalább egy csík kell
  (`SCHEDULE_REVISION_EMPTY`), és az **ütközések** nem akadály, de a felület a kiadáskor kiírja őket.
- **Szín:** az **iparági revíziószín-sorrend** első kilenc eleme (fehér, kék, rózsaszín, sárga, zöld, aranysárga,
  drapp, lazac, meggy); a tizedik revíziótól a sorrend elölről indul, `color_cycle` növelésével. A színt a rendszer osztja ki
  (nem szabad választás), a megjelenítés a témához igazodik, a nyomtatás a papírszínt jelzi szöveggel is.
- **A „módosított” állapot számolt** (nem tárolt, nem folyamatállapot): a munkaterv `snapshot_hash`-e eltér az utolsó kiadott
  revízióétól. A G1 „tervezet → kiadott → módosított” sora így a **munkaterv oldalán** számolt jelző, a revízió pedig csak
  `draft`/`published`/`superseded`/`discarded`.
- **Visszaállítás** (`backlot.schedule.revision.restore`): egy korábbi revízió pillanatképét a munkatervre írja, **egy
  tranzakcióban, kifejezett paranccsal**; a jelenleg módosított (mentetlen) munkatervről előbb piszkozat-revízió készül
  (felajánlva), hogy ne vesszen el. A kivezetett jelenet vagy visszavont nap csíkja nem állítható vissza
  (`SCHEDULE_RESTORE_PARTIAL`, a felsorolt elemekkel; részleges alkalmazás nincs).
- **Összevetés** (`backlot.schedule.revision.compare {a, b}`): jelenetenként új, eltávolított, másik napra tett, napon belül
  átsorolt, becslésben változott; napok: új, megszűnt, hívási idő változott; és az **érintettek**: azok a szereplők és
  stábtagok, akiknek a behívott napjai eltérnek (a G1 „változások követése”). A munkatervet is lehet revízióval összevetni.

### 4.4 Tudomásulvétel, beállítások, diszpozíció

```sql
CREATE TABLE backlot.schedule_conflict_ack (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    kind text NOT NULL, fingerprint text NOT NULL,                     -- a jelzés ujjlenyomata (7.4)
    shooting_day_id uuid REFERENCES backlot.shooting_day(id),
    reason text NOT NULL CHECK (length(reason) BETWEEN 1 AND 1000),
    ...közös oszlopok,
    UNIQUE (production_id, fingerprint)
);
```

- **Beállítások** (a `production.settings['schedule']` zárt, sémával ellenőrzött kulcsai, G2-02): `minutes_per_page`
  (alapérték 60), `night_from` (18:00), `day_until` (18:00), `max_day_minutes` (720), `strip_colors` (a 4 INT/EXT×napszak
  kombináció színtokenje; alapértelmezés az elterjedt gyakorlat: nappal-belső semleges/szürke, nappal-külső sárga,
  éjszaka-belső kék, éjszaka-külső zöld). Ismeretlen kulcs `SCHEDULE_SETTINGS_INVALID`. A beállítás módosítása a meglévő csíkokat
  **nem írja át** (a becslés a csík adata).
- **Diszpozíció** (call sheet): 10.1.

### 4.5 A jelenet–szereplő feloldás (az ütközésvizsgálat alapja)

- A csík jelenetéből a **szereplők**: a jelenet `casting` kapcsolatai (G2-19) → karakter → a napon érvényes `principal`
  `cast_assignment` → személy. Ha a kapcsolat `metadata`-ja `{"variant": "…"}`-t hordoz (a G2-03 `entity_link.metadata` mezője;
  **G2-19 3.1 kiegészítése**: opcionális, a `variant` a szereposztás változatával egyezik), csak az egyező változat számít,
  különben a karakter minden érvényes `principal` szereplője (gyerek- és felnőttkori változat: a felület jelzi a kétértelműséget
  `ambiguous_variant`, információ).
- A **kaszkadőr, dublőr, statiszta** a breakdown elemeiből (G2-22 `stunts`, `extras`) **nem** oldódik személlyé (az elem
  szabad szöveg); ők a **kézi hívással** (`cast_call`, `manual`) kerülnek a tervbe. Ez korlát, nem hiba: kimondva a felületen.
- A **helyszín:** `setting` → `set` → `location_set_link` `selected` állapotú valós helyszínek; ha több van, a jelenet **bármelyik
  megfelelő foglalással fedett** (a felület mindet mutatja).

## 5. Parancsok

| Parancs | Mit csinál |
|---|---|
| `backlot.schedule.board` | A stripboard adata: napok, csíkok, napi összesítő (oldalnyolcad, becsült perc), szűrők (nap, egység, helyszín, szereplő, INT/EXT, napszak, ütemezetlen) |
| `backlot.schedule.strip.add` / `.update` / `.move` / `.reorder` / `.split` / `.merge` / `.remove` | Csík: felvétel (jelenet vagy banner), módosítás, áthelyezés napra (vagy ütemezetlenné), átsorolás, jelenetrészre bontás/összevonás, törlés (csak a csík) |
| `backlot.schedule.day.update` / `.call.set` / `.call.remove` / `.crew` | Nap hívási adata; részleg- és személyi hívás; a nap stábja (számolt) |
| `backlot.schedule.calls.sync` | A szereplői behívások előállítása a csíkokból (`source = 'schedule'`); előnézettel (`dry_run`) |
| `backlot.schedule.check` | Ütközésvizsgálat a munkatervre, egy napra vagy egy revízióra (7.) |
| `backlot.schedule.ack` / `.unack` | Tudomásulvétel és visszavonása (indoklással) |
| `backlot.schedule.revision.create` / `.publish` / `.discard` / `.restore` / `.compare` / `.get` / `.list` | Revíziók (4.3) |
| `backlot.schedule.dood` | Day out of days (8.) |
| `backlot.schedule.callsheet.generate` / `.get` | Diszpozíció (10.1) |
| `backlot.schedule.export` | Ütemterv: stripboard PDF, egysoros ütemterv CSV, napi lista |
| `backlot.schedule.settings.get` / `.set` | Beállítások (4.4) |

Hibakódok: `SCHEDULE_STRIP_INVALID`, `SCHEDULE_SCENE_INVALID` (nem jelenet, kivezetett, más produkció),
`SCHEDULE_PART_INVALID`, `SCHEDULE_DAY_NOT_FOUND`, `SCHEDULE_DAY_HAS_STRIPS`, `SCHEDULE_CALL_INVALID`,
`SCHEDULE_REVISION_NOT_FOUND`, `SCHEDULE_REVISION_STATE_INVALID`, `SCHEDULE_REVISION_EMPTY`, `SCHEDULE_RESTORE_PARTIAL`,
`SCHEDULE_ACK_INVALID`, `SCHEDULE_SETTINGS_INVALID`, `SCHEDULE_CALLSHEET_FAILED`, `SCHEDULE_EXPORT_FAILED`,
`CONFLICT_STALE_VERSION`; valamint a `STRUCTURE_*`, `CAST_*`, `CREW_*`, `LOCATION_*`, `EQUIPMENT_*`, `DOCUMENT_*`, `FILES_*`,
`WORKFLOW_*`, `AUDIT_*` kódok változatlanul.

## 6. Hozzáférés

- **Olvasás:** az ütemterv, a napok, a csíkok, a hívások és az ütközések a **produkció minden tagja** számára láthatók (a
  `cast_call` a G2-19 szerint is az), kivéve a **breakdown-elemek** (G2-22 9.1: a forgatókönyv-dokumentum láthatósága szerint).
- **Írás:** `owner`, `producer`, `coordinator`; **revízió kiadása és visszaállítása** `owner`, `producer`; a tudomásulvétel
  `owner`, `producer`, `coordinator`.
- **A diszpozíció személyes adata** (10.1) a G2-18 szintjei szerint: a `contact` szint adatai csak annak a felhasználónak kerülnek
  a PDF-be, aki azt látja (`owner`, `producer`, `coordinator`), és **kifejezett választásra** (alapból nincs elérhetőség).
- A szabályok a szolgáltatásrétegben és a lekérdezésben **már most** érvényesek (M2-re kész).

## 7. Ütközésvizsgálat

A vizsgáló egy **tiszta függvény** (`evaluate_plan`), amelynek bemenete egy **terv-nézet** (csíkok napokra, a napok adatai) és
a forrásadatok betöltői; a **munkatervre és a kiadott revíziókra ugyanaz fut** (csak a betöltő más). A 23-2 a hipotetikus
változatot ugyanígy a betöltővel adja. A számítás **lekérdezéskor**, nincs tárolt jelző (a naptárral, G2-15/17/19/20/21-gyel azonos
elv), halmaz-alapú SQL-ekkel, nem csíkonkénti lekérdezéssel; **nem tilt**, a mentés mindig lehetséges.

### 7.1 Jelzések

| Jelzés | Feltétel | Súly |
|---|---|---|
| `cast_on_leave` | A jelenet szereplője (4.5) aznap szabadságon van (G2-15 `calendar_exception`/erőforrás-naptár; csak ha van `resource`-sora) | figyelmeztetés |
| `cast_outside_assignment` | A szereplő `cast_assignment`-je a napon nem érvényes | figyelmeztetés |
| `cast_double_booked` | Ugyanaz a személy **két egységnél** is behívva ugyanarra a dátumra (két csík, különböző `unit`) | hiba |
| `cast_over_contracted_days` | A szereplő ütemezett napjainak száma (különböző dátum, a tervből és a kézi hívásokból) meghaladja a `contracted_days`-et | figyelmeztetés |
| `cast_over_option_days` | Meghaladja a `contracted_days + option_days`-et | hiba |
| `crew_on_leave` | A behívott stábtag aznap szabadságon van (G2-15) | figyelmeztetés |
| `crew_outside_assignment` | A személyi `called` sor olyan stábtagra szól, akinek a beosztása a napon nem érvényes | figyelmeztetés |
| `scene_no_location` | A jelenetnek nincs `setting` helyszíne, vagy a `set`-nek nincs `selected` valós helyszíne | információ |
| `location_not_booked` | A jelenet helyszínének egyik `selected` valós helyszínére sincs aznapra nem lemondott foglalás (G2-20) | figyelmeztetés |
| `location_booking_issue` | A napra szóló foglalásnak van a G2-20 4.3 szerinti jelzése (átfedés, tiltott ablak, hiányzó engedély) | a G2-20 súlya |
| `location_violates_time_window` | A nap `[call_time, call_time + összes csík]` kívül esik a helyszín `time_window` korlátozásán (G2-20 4.2); éjfélen átnyúló nap kívül esik, ha van `time_window` | figyelmeztetés |
| `location_violates_weekday` | A nap a helyszín `weekdays` korlátozásán kívül van | figyelmeztetés |
| `location_exceeds_max_crew` | A nap stábjának létszáma (számolt, 4.1) meghaladja a helyszín `max_crew` korlátját | figyelmeztetés |
| `equipment_booking_issue` | A napra vagy jelenetre kötött `equipment_booking`-nak van a G2-21 4.2 szerinti jelzése (átfedés, karbantartás, nincs ott) | a G2-21 súlya |
| `night_scene_in_daylight` | `night` jelenet várt kezdete a `night_from` előtt | figyelmeztetés |
| `day_scene_after_dark` | `day` jelenet várt vége a `day_until` után | figyelmeztetés |
| `day_over_target` | A csíkok és bannerek összes ideje meghaladja a `target_duration_minutes`-t | figyelmeztetés |
| `day_over_max` | Meghaladja a `max_day_minutes`-et | hiba |
| `day_without_call` | A napnak vannak csíkjai, de nincs `call_time` | információ |
| `scene_unscheduled` | Aktív jelenetnek nincs csíkja (a produkció szintjén) | információ |
| `scene_scheduled_twice` | Ugyanaz a jelenet két csíkon, rész nélkül, vagy ugyanaz a rész kétszer | hiba |
| `scene_retired` | A csík kivezetett jelenetre mutat | figyelmeztetés |
| `minor_called` | Kiskorú szereplő (G2-19 `is_minor`) szerepel a napon | információ |
| `ambiguous_variant` | A karakternek több érvényes `principal` változata van, és a kapcsolat nem szűkít (4.5) | információ |

### 7.2 Napszak-szabály

A napszak-vizsgálat **nem csillagászati számítás** (nincs helyzet- vagy napkelte-adat, nincs hálózat): a beállított `night_from`
és `day_until` időpontot használja. A `dawn` és `dusk` jelenetet **nem** értékeli (nincs egyértelmű szabály). A felület kiírja:
„A napszak-ellenőrzés a beállított idők alapján készül, nem a napkeltéből.”

### 7.3 Szerződött napok

A `cast_over_*` az **ütemezett napokat** számolja (a tervből és a kézi hívásokból, különböző dátumok), a G2-19 hasonló jelzése a
**`cast_call` táblából** számol. A két érték a `calls.sync` után egyezik; eltérés a felületen („a behívások nincsenek
szinkronban a tervvel”). Visszamenőleges (már leforgatott) napok nincsenek külön jelölve (nincs tényleges előrehaladás-követés).

### 7.4 Tudomásulvétel (felülbírálás)

- A felhasználó egy jelzést **tudomásul vehet** indoklással (`backlot.schedule.ack`): a jelzés a nézetben „tudomásul véve”
  jelölést kap, nem tűnik el. Az **ujjlenyomat** = `kind` + a jelzés tárgyainak azonosítói (nap, jelenet, személy, helyszín,
  eszköz) rendezve; ha a jelzés tárgya vagy tartalma megváltozik (másik napra kerül, más személy), az **ujjlenyomat is változik,
  a tudomásulvétel nem él tovább** (a régi sor megmarad az előzményben). Hiba súlyú jelzés is tudomásul vehető (a felhasználó dönt).
- Minden tudomásulvétel és visszavonás **auditált** (a G1 „a felülbírálás auditált” követelménye), az indoklás szövege az
  auditban szerepel (nem személyes jelölésű mező).

### 7.5 Amit a rendszer nem értékel

**Munkajogi szabályokat nem értékelünk:** pihenőidő a hívások között, túlóra, hétnapos munkahét, kiskorúak munkaidő-korlátja,
szakszervezeti szabályok. Ezek országonként, megállapodásonként és szerződésenként eltérnek; egy beépített, de téves szabály
hamis biztonságot adna (a G2-19 `minor_called` ugyanezen az elven csak jelez). A **`minor_called`** felhívja a figyelmet; a
konfigurálható szabálykészlet (felhasználó által megadott korlátok) külön döntéssel, későbbi bővítésként jöhet. Ez **eltérés a G1-től**,
amely „gyerekszereplő munkaidő-korlátját” említette: itt csak a jelzés van.

## 8. Day out of days

`backlot.schedule.dood {source: live | revision, unit?, from?, to?}`: oszlop = **forgatási nap** (dátum szerint, az egységek
összevonva), sor = **szereplő** és **stábtag**; a cella a státusz:

| Kód | Jelentés |
|---|---|
| `SW` | az első munkanap (Start Work) |
| `W` | munkanap (köztes) |
| `WF` | az utolsó munkanap (Work Finish) |
| `SWF` | az egyetlen munkanap (Start Work Finish) |
| `H` | tartás (Hold): az első és az utolsó munkanap között, nem munkanap (**csak szereplőnél**) |

- **Munkanap** a szereplőnél: a terv csíkjaiból (4.5) és a **kézi hívásokból** (`cast_call`, `manual`, `breakdown`) jövő dátum;
  a stábtagnál: a beosztása érvényes napjai, a személyi `called = false` kivételekkel; az egyszerű stáb (minden nap hívott) így
  `SW`…`WF` sor, a **részleges stáb** (pl. csak a kaszkadőrcsapat bizonyos napokon) a kivételekből adódik.
- **Összegzés:** munkanapok, tartásnapok, a `contracted_days`/`option_days` összevetése (`cast_engagement`), az első és az
  utolsó nap. Személyes adat nincs benne (megjelenített név, művésznév).
- **Egy hely az igazságnak:** a DOOD **számolt nézet**, nem tárolt mátrix; revízióra is kérhető (a kiadott verzió DOOD-ja).
- Export: CSV (UTF-8 BOM-mal, képlet-védelemmel, mint a G2-22) és PDF.

## 9. Események és naptár

- **Események** (outbox, G2-12): `backlot.schedule.revision_published`, `.revision_discarded`, `backlot.schedule.callsheet_issued`.
  A `revision_published` hasznos terhe (`payload`): a revízió száma és színe, az **összevetés összegzése** az előzőhöz
  (új/eltávolított/áthelyezett jelenet, érintett napok darabszáma); személyes adat nincs benne. Új címzett nincs
  (`production_owner`, `watcher`).
- **Naptár:** a `shooting_days` réteg (G2-17) mutatja a hívási időt, a csíkok számát és az oldalösszeget; a szerkesztés a
  `backlot.schedule.day.update` hívása. A `cast_calls` réteg (G2-19) a `schedule` forrású hívásokat is mutatja.

## 10. Diszpozíció (call sheet) és export

### 10.1 Napi diszpozíció

- **Tartalom (PDF, `QPdfWriter`):** fejléc (produkció, a nap sorszáma „N. nap / összes”, dátum, egység, hívási idő, célzott
  időtartam); **helyszínek** (név, cím; a G2-20 `tech.nearest_hospital`, parkolás, megközelítés); **jelenetek** a csíksorrendben
  (jelenetszám, INT/EXT, helyszín, napszak, nyolcad, szereplők cast ID-val, szinopszis, várt kezdet); **szereplők** (cast ID, karakter,
  művésznév, hívási, smink- és jelmezidő); **stáb részlegenként** hívási idővel; **banner**-megjegyzések; **különleges igények**
  (kaszkadőr, jármű, állat, effekt, biztonság, statiszták: a breakdown elemei kategóriánként, **csak a felhasználó által kijelölt
  kategóriákból**, és csak annak, aki a breakdownot láthatja, G2-22 9.1); a **következő nap** előzetese (opcionális).
- **Elérhetőség:** alapból **nincs telefonszám és e-mail** a PDF-ben. A felhasználó kifejezetten kiválaszthatja (szereplők /
  stáb / helyszín-kapcsolattartók), és csak a G2-18 szintjei szerint jogosult teheti. Ilyenkor a dokumentum
  `contains_personal_data` jelölésű (G2-14 8. pont: a fájlnév és a kinyert szöveg nem kerül az auditba).
- **Tárolás:** a diszpozíció **`call_sheet` típusú dokumentum** (G2-14) naponta (és egységenként) egy, **minden kiadás új verzió** (a
  `files.import`/`files.seal` az `attachment` gyökérbe, a G2-09 útsablonjával); a dokumentum a forgatási naphoz kötött
  (`document_link`, `subject_kind = 'shooting_day'`, `relation = 'call_sheet_for'`; **új `subject_kind`**, G2-05/G2-14
  regiszterbővítés). A kiadás a **munkaterv `snapshot_hash`-ét** is rögzíti (a verzió jegyzetében), így látszik, melyik
  tervállapotból készült.
- **A fájl a gépen marad (SEC-02):** a rendszer **nem küld e-mailt és nem oszt meg** semmit; a felhasználó a PDF-et saját
  csatornáján továbbítja. (Az értesítési központ helyi értesítése a revízióról szól, nem a személyes adatról.) A kiadott PDF
  **nem módosul**, az újabb kiadás új verzió; a régi megmarad.
- **Stripboard és egysoros ütemterv exportja:** PDF (nyomtatásra szánt csíkszínek, szöveges jelzéssel is, a színvak olvashatóságért)
  és CSV (UTF-8 BOM-mal, képlet-védelemmel). A PDF a **nem látható** breakdown-adatot nem tartalmazza.

## 11. Never crash

- Hibás csík (nem jelenet, kivezetett jelenet, ismétlődő rész, érvénytelen pozíció), hibás nap, hibás hívás, hibás beállítás:
  angol hibakód, nincs írás.
- **Átsorolás és áthelyezés egy tranzakcióban** (halasztott egyediség): hiba esetén semmi nem változik; egyidejű módosításnál
  `CONFLICT_STALE_VERSION` (a nap és a csík `row_version`-jével), félig átrendezett nap nincs.
- **A vizsgáló hibatűrő:** egy hiányzó vagy sérült forrásadat (pl. visszavont szereposztás, törölt helyszín-kapcsolat) a **konkrét
  jelzést** adja (`…_unresolved`, információ), nem dob kezeletlen kivételt, és a többi jelzés megjelenik.
- A `calls.sync` **egy tranzakció**; csak a `schedule` forrású sorokat írja; hiba esetén a hívások a régiek.
- A `revision.publish` és `.restore` egy tranzakció (pillanatkép + állapotlépés + esemény); félig kész revízió nincs;
  a `snapshot_hash` kanonikus (rendezett kulcsok, stabil szerializáció), ugyanabból az állapotból mindig ugyanaz.
- A PDF-generálás hibája: kezelt hiba (`SCHEDULE_CALLSHEET_FAILED`), nincs félig lezárt fájl (a lezárt fájl `deleted`, a G2-14 mintája).
- Éjfélen átnyúló nap, üres nap, csík nélküli nap, ezeregynapos terv: kezelt eset, nincs osztás nullával vagy üres lista-összeomlás.

## 12. Eltérések a G1-től (összegzés)

| # | G1 | G2-javaslat | Indok |
|---|---|---|---|
| 1 | `ShootingSchedule` mint objektum (változat, állapot) | Egy **munkaterv** a produkcióhoz (élő csíkok) + **revíziók** (pillanatképek) | A „tervezet → kiadott → módosított” a munkatervre számolt jelző, a kiadott változat a revízió; a külön tároló új tábla és kettős igazság lenne |
| 2 | A terv állapota a Workflow Engine-ben | A **revízió** folyamata van a Workflow Engine-ben; a „módosított” számolt | A munkaterv folytonos szerkesztésére nem illik állapotgép; a kiadás átmenet |
| 3 | „Gyerekszereplő munkaidő-korlátja” | Csak `minor_called` jelzés; **munkajogi szabályokat nem értékel** | Országonként eltérő; téves beépített szabály hamis biztonság |
| 4 | Az automatikus javaslat külön ütemterv-változatként | **23-2**, a hipotetikus változattal együtt | A G2-15 8. pontjának függőségi döntése; addig a revízió-piszkozat és a visszaállítás |

## 13. Sorrend és függés

| Rész | Kell hozzá |
|---|---|
| Nap, csík, board, becslés, beállítások | 17. lépcső (`shooting_day`), 3. lépcső (jelenet) |
| Jelenet-adatok, szereplők, helyszín a vizsgálathoz | 22., 19., 20. lépcső |
| Eszköz-ütközés | 21. lépcső; stáb: 18.; szabadság: 15. |
| Revíziók | G2-04 (Workflow Engine), 12. lépcső (esemény) |
| Diszpozíció | 14. lépcső (dokumentum), File Services FS-1, PySide6 `QPdfWriter` |
| Automatikus javaslat | 23-2: OR-Tools, külön G3 |

A G1 „Előfeltétel: 17–22. lépcső” **megmarad**; a 15. lépcső csak a szabadság-vizsgálathoz kell (nélküle az a jelzés hiányzik, a
többi működik).

## 14. Tesztek (G5-re előre)

- **Csík és board:** felvétel, áthelyezés napra és ütemezetlenné, átsorolás (halasztott egyediség, hibainjektálás),
  jelenetrész-bontás és összevonás, banner, a teljesség jelzése, kivezetett jelenet; az `est_minutes` alapérték (a beállítás
  szerint); a napon belüli idő (kezdet/vég) számítása éjfélen átnyúló napra is.
- **Ütközésvizsgáló (táblázatos teszt minden jelzésre):** szabadság, szerződés-érvényesség, kettős egység, szerződött és opciós
  napok (határesetek), helyszín (nincs foglalás, időablak, hétköznap, létszám), eszköz, napszak (határidők), napi hossz, kétszeres jelenet,
  változat-kétértelműség. **Tisztaság:** azonos bemenetre azonos kimenet; a munkatervre és a revízióra ugyanaz a függvény.
- **Tudomásulvétel:** ujjlenyomat stabil és változik, ha a tárgy változik (a régi ack megszűnik); audit.
- **Revízió:** létrehozás, kiadás (szín- és számkiosztás, `superseded`), 10. revízió színforduló (`color_cycle`), üres elutasítva,
  visszaállítás (egy tranzakció, piszkozat-felajánlás, részleges alkalmazás tiltva), pillanatkép változtathatatlansága, a
  „módosított” számolt jelző, `snapshot_hash` kanonikussága (véletlen sorrendű beszúrás).
- **Összevetés:** minden eltérésfajta, érintettek (szereplő és stáb), nagy terv (2000 csík) ideje.
- **Behívás:** `calls.sync` (idempotens; kézi hívás érintetlen), `dry_run`, a stáb számolt hívása és a személyi kivétel.
- **DOOD:** SW/W/WF/SWF/H táblázatos teszt (egy nap, egymást követő, szakadozott); kézi hívás beleszámít; stáb; revízióból; CSV BOM és
  képlet-védelem.
- **Diszpozíció:** tartalom, elérhetőség alapból nincs, kiválasztott elérhetőség csak jogosultnak, `contains_personal_data`, a nem látható
  breakdown nincs benne; új kiadás új verzió; a régi változatlan; hibainjektálás a PDF és a dokumentum között; **nincs hálózati kérés**
  (hamis hálózati réteg, nulla kérés).
- **Hozzáférés:** minden szerep minden adatra (olvasás, írás, kiadás), nem tag produkció adata nem látszik.
- **Naptár, események:** a `shooting_days` réteg, a `revision_published` hasznos terhe és annak személyesadat-mentessége.
- **Never crash:** fuzzolt csík-, nap- és hívás-bemenetek; sérült forrásadat a vizsgálóban.
- **Felületi ellenőrzés:** `ui.screenshot` a stripboardról ütközésekkel, a DOOD-ról és a diszpozíció előnézetéről; az áthelyezés
  billentyűzettel és a vezérlőcsatornán (`schedule.strip.move`) is elérhető (nem csak húzással).
- **Mérés** a CFA4-MIN gépen: 2000 csík, 150 forgatási nap, 400 szereplő- és 3000 stáb-hívás; a board betöltése, az ütközésvizsgálat, a
  revízió kiadása és összevetése, a DOOD ideje.

## 15. Licenc (G3-ra előre)

Új függőség nincs az M1-ben: a PDF `QPdfWriter` (PySide6), a pillanatkép-hash `hashlib`, a CSV standard modul. **OR-Tools (Apache-2.0)**
csak a 23-2-ben, külön G3-mal. A Movie Magic Scheduling, StudioBinder, Gorilla Scheduling **funkcionális referencia** (stripboard,
napzáró csík, day out of days, diszpozíció, revíziószínek); kódot, formátumot és arculatot nem veszünk át.

## 16. G2-eredmény

**Javaslat: PASS.** Új döntési kérdés nincs; négy G1-től eltérő pont indokolva (12. pont). A megvalósítás a 17., 22., 19., 20.
lépcső után indulhat; az ütközésvizsgálat a 15., 18., 21. lépcső adataival teljesedik ki, azok nélkül a megfelelő jelzés hiányzik, a többi
működik. A G2-17 (`shooting_day` bővítés, visszavonás-védelem), a G2-19 (`cast_call` oszlopai és `source = 'schedule'`, a `casting`
kapcsolat `variant` metaadata), a G2-05/G2-14 (`subject_kind = 'shooting_day'`) és a G2-12 13. pontja (események)
kiegészítése megvalósításkor kerül oda, új tábla nélkül.
