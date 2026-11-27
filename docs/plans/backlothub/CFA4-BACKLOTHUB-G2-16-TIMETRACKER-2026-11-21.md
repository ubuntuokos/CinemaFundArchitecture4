# BacklotHub 16. lépcső – TimeTracker: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-21 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.10, F6 · **Előzmény:**
[G1 TimeTracker](CFA4-BACKLOTHUB-G1-16-TIMETRACKER-2026-11-07.md) (jóváhagyva 2026-11-07, funkcionális
követelmények 2026-11-10) · **Épül rá:** G2-04 (Workflow Engine), G2-05 (feladat, `person`), G2-07 (szűrőnyelv),
G2-12 (események), G2-15 (kapacitás, hátralévő munka) · **Állapot:** G2-javaslat, jóváhagyásra vár · implementáció nincs

Új döntési kérdés nincs, új függőség nincs. Egy kiegészítés a G2-15-ön (9. pont), amely a hátralévő munkát
pontosítja.

## 1. Hatókör ebben a lépcsőben

- **Időmérés stopperrel** és **kézi időrögzítés**, kategóriákkal.
- **Időlap** (hét) beadása és jóváhagyása a Workflow Engine-nel; jóváhagyás után **javítás ellenbejegyzéssel**.
- **Összesítések és riportok:** feladat, elem-részfa, személy, kategória, produkció, hét, hónap szerint; a
  becslés és a tény eltérése. Export CSV-be és PDF-be.
- **A hátralévő munka pontosítása** a Resource Planningben (9. pont).
- **Nem része:** díjak, munkaköltség és számlázás (24. lépcső, Budget Tracking; árak ott lesznek, itt nincsenek),
  időmérés a DCC-kből (a 11a. host parancsai ezt nem tartalmazzák), emlékeztető értesítés az időlap beadására
  (később, értesítési szabályként igény szerint), automatikus időrögzítés tevékenységfigyeléssel.

## 2. Integrációs határok

| Határ | Irány | Szerződés |
|---|---|---|
| Task (5.) | ← | `task`: azonosító, produkció, állapotkategória (lezárt feladatra is lehet rögzíteni, visszavontra nem) |
| Workflow (4.) | ↔ | `TimesheetAdapter`: az időlap `status_id`-je a hiteles állapot; alapfolyamat `timesheet_default`; két új őrfeltétel (4.3) |
| Resource Planning (15.) | → | A `backlot.task_remaining_minutes` nézet a rögzített időt is levonja (9. pont); az erőforrás és az időbejegyzés ugyanahhoz a `person`-höz kötődik |
| Search (7.) | → | A közös szűrőnyelv (G2-07) új típusa: `time_entry` (mezők: produkció, feladat, elem-részfa, személy, kategória, dátum) |
| Audit (1.) | → | A bejegyzések, az időlap-átmenetek és a javítások auditáltak; a stopper indítása nem (4.1) |
| Notification (12.) | → | `backlot.timesheet.submitted`, `.approved`, `.returned` |
| Dashboard (6.) | – | Nincs saját csempe ebben a lépcsőben; a riport lekérdezhető, csempe később igény szerint |
| Budget (24.) | később | A rögzített percek, a javításokkal együtt, a munkaköltség alapja |

**Az időbejegyzés a `person`-höz kötődik, nem az `resource`-hoz.** A G1 az erőforrást említette; a `resource` (G2-15) a
tervezési réteg, a `person` az identitás (G2-05, #53). Így az időt az is rögzítheti, akinek nincs erőforrás-sora,
és egy `resource` nélkül is teljes a riport. Az erőforrás és az időbejegyzés a `person_id`-n találkozik.

## 3. Alapelvek

1. **Az idő elveszhetetlen:** a futó stopper az adatbázisban van, nem a felület memóriájában. Összeomlás,
   újraindítás, a felület bezárása nem állítja meg és nem törli.
2. **Az idő nem találgatható:** ha a stopper gyanúsan régóta fut (4.1), a leállítás **kifejezett** befejezési
   időt kér, nem számol a leállítás pillanatáig.
3. **A jóváhagyott idő nem módosul:** jóváhagyás után csak **ellenbejegyzés** (javítás) adható, az eredeti sor
   marad. A riportok a nettó összeget mutatják, az előzmény az időlapon és az auditban látszik.
4. **Személyes adat:** a személyenkénti időadat személyes adat (SEC-06), a hozzáférést a 7. pont szabályozza.

## 4. Adatmodell (DDL-vázlat, `backlot` séma)

### 4.1 Kategória, stopper, bejegyzés

```sql
CREATE TABLE backlot.time_category (
    id uuid PRIMARY KEY, code text NOT NULL, label_key text NOT NULL,
    requires_task boolean NOT NULL DEFAULT false,           -- pl. alkotó munka: feladat kell; megbeszélés: nem
    ordinal smallint NOT NULL, retired_at timestamptz,
    production_id uuid REFERENCES backlot.production(id),    -- NULL: globális alapkategória
    ...közös oszlopok production_id nélkül,
    UNIQUE (code, production_id)
);
-- Alapkategóriák migrációban: creative (alkotó munka), retake (javítás), meeting, travel, admin, other.

CREATE TABLE backlot.time_timer (                            -- futó stopper; személyenként legfeljebb egy
    person_id uuid PRIMARY KEY REFERENCES backlot.person(id),
    production_id uuid NOT NULL REFERENCES backlot.production(id),
    task_id uuid REFERENCES backlot.task(id),
    category_id uuid NOT NULL REFERENCES backlot.time_category(id),
    note text,
    started_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL
);

CREATE TABLE backlot.timesheet (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    person_id uuid NOT NULL REFERENCES backlot.person(id),
    week_start date NOT NULL CHECK (extract(isodow FROM week_start) = 1),   -- ISO-hét, hétfő
    status_id uuid NOT NULL REFERENCES backlot.status(id),   -- timesheet_default folyamat (G2-04)
    submitted_at timestamptz, decided_at timestamptz,
    ...közös oszlopok,
    UNIQUE (production_id, person_id, week_start)
);

CREATE TABLE backlot.time_entry (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    timesheet_id uuid NOT NULL REFERENCES backlot.timesheet(id),
    person_id uuid NOT NULL REFERENCES backlot.person(id),
    task_id uuid REFERENCES backlot.task(id),
    category_id uuid NOT NULL REFERENCES backlot.time_category(id),
    kind text NOT NULL DEFAULT 'work' CHECK (kind IN ('work','correction')),
    source text NOT NULL CHECK (source IN ('timer','manual')),
    work_date date NOT NULL,                                 -- a helyi dátum a rögzítéskor; időzóna-változás nem mozgatja
    started_at timestamptz, ended_at timestamptz,            -- mindkettő vagy egyik sem
    minutes integer NOT NULL,                                -- ellenbejegyzésnél negatív is lehet
    note text,
    corrects_entry_id uuid REFERENCES backlot.time_entry(id),
    split_group_id uuid,                                     -- éjfélnél szétvágott stopper-bejegyzések
    retired_at timestamptz,                                  -- nyitott időlapon a „törlés”; az összegekből kimarad
    ...közös oszlopok,
    CHECK ((started_at IS NULL) = (ended_at IS NULL)),
    CHECK (ended_at IS NULL OR ended_at > started_at),
    CHECK (started_at IS NULL OR minutes = round(extract(epoch FROM (ended_at - started_at))::numeric / 60)),
    CHECK ((kind = 'work'       AND minutes BETWEEN 1 AND 1440 AND corrects_entry_id IS NULL)
        OR (kind = 'correction' AND minutes BETWEEN -1440 AND 1440 AND minutes <> 0 AND corrects_entry_id IS NOT NULL
                                AND started_at IS NULL))
);
CREATE INDEX ON backlot.time_entry (person_id, work_date) WHERE retired_at IS NULL;
CREATE INDEX ON backlot.time_entry (task_id) WHERE retired_at IS NULL;
CREATE INDEX ON backlot.time_entry (production_id, work_date) WHERE retired_at IS NULL;
```

**Szabályok** (adatbázisban kikényszerítve, ahol lehet; a többit a szolgáltatásréteg és a teszt őrzi):

- **A bejegyzés időtartama percben egész szám.** Stopperrel mért idő a leállításkor **a legközelebbi percre**
  kerekítve (félperc felfelé, `Decimal`/`numeric` kerekítéssel mindkét oldalon, így a Python és a CHECK
  megegyezik), legalább 1 perc; 30 másodpercnél rövidebb mérésnél a felület eldobást ajánl
  (`TIME_ENTRY_TOO_SHORT`, nem ír). Kézi bejegyzés vagy `started_at/ended_at`-tal (a perc ebből jön), vagy csak
  nappal és időtartammal.
- **A feladat és a hozzá tartozó időlap egy produkcióban van** (trigger). A kategória `requires_task` esetén
  feladat nélkül `TIME_ENTRY_INVALID`. **Lezárt állapotú feladatra rögzíthető** (utólagos munka), **visszavont
  feladatra nem** (`TIME_TASK_RETIRED`).
- **Napi felső korlát:** egy személy egy napra nettó összege (minden produkción át, `retired_at IS NULL`) legfeljebb
  1440 perc; túllépésnél `TIME_ENTRY_INVALID`. A korlátot **tranzakciós tanácsadó zár** védi
  (`pg_advisory_xact_lock` a (személy, nap) párra), így két egyidejű írás sem léphetné túl.
- **Átfedés:** két, `started_at/ended_at`-tal rendelkező bejegyzés ugyanannál a személynél átfed → **figyelmeztetés**
  (`TIME_ENTRY_OVERLAP`, a bejegyzés mégis menthető megerősítéssel); a G1 szerint nem tiltás. A csak időtartammal
  rögzített bejegyzésre átfedésvizsgálat nincs, csak a napi korlát.
- **A `work_date` a helyi dátum**, a stopper esetén a `started_at` helyi napja. Ha a stopper **éjfélen át**
  futott, a leállításkor a rendszer **a helyi éjféleknél szétvágja** bejegyzésekre (azonos `split_group_id`, mindegyik
  a saját napjához), hogy a napi összeg és az időlap hete helyes legyen. Az átállási napok (DST) a helyi idő
  szerint helyesen vágódnak (`zoneinfo`).
- **Idő visszaugrása:** ha a rendszeróra visszaállt, és a befejezés nem későbbi a kezdésnél, a leállítás
  `TIME_ENTRY_INVALID`-ot ad és felkínálja a kézi javítást; a stopper megmarad.
- **Törlés** nyitott (`open`/`returned`) időlapon a `retired_at` beállítása: az összegekből kimarad, a „törölt
  sorok” megjeleníthetők, az audit őrzi. Beadott vagy jóváhagyott időlap sorához nem nyúl (`TIME_TIMESHEET_LOCKED`).
- **Ellenbejegyzés** (`kind = 'correction'`): jóváhagyott időlap sorát javítja az eredetire hivatkozva
  (`corrects_entry_id`), előjeles perccel és **kötelező indokkal** (`note`); idő nélkül, nappal. Az eredeti és a javítás
  ugyanabban a produkcióban és ugyanannál a személynél van; a javítás ugyanabba az időlapba kerül, amelyiknek az
  eredeti sora van, és nem módosítja a jóváhagyott időlap állapotát. Javítás **csak jóváhagyott** időlapra adható
  (máskor az eredeti szerkeszthető). A nettó összeg a feladaton nem mehet 0 alá egy eredeti bejegyzés javításával
  sem (`TIME_CORRECTION_INVALID`).

### 4.2 A stopper

- **Egyszerre egy futó stopper személyenként** (a `time_timer` elsődleges kulcsa a `person_id`). `backlot.time.start`
  futó stopper mellett `TIME_TIMER_RUNNING`-ot ad; `switch: true` esetén a futót **egy tranzakcióban** leállítja és az
  újat indítja (feladatváltás).
- **Leállítás** (`backlot.time.stop`): a `time_timer` sorból bejegyzés lesz (szétvágással, kerekítéssel), a sor törlődik,
  egy tranzakcióban az audit és az outbox-sor. Hiba esetén a stopper **megmarad** (nincs idővesztés).
- **Gyanús stopper:** ha a futó stopper kora meghaladja a küszöböt (alapból 16 óra, beállítható), a felület
  figyelmeztet, és `backlot.time.stop` **kifejezett `ended_at`-ot** kér, különben `TIME_TIMER_STALE`. Az `ended_at` nem
  lehet korábbi a kezdésnél és nem későbbi a mostaninál.
- **Indulás összeomlás után:** a felület induláskor lekérdezi a stoppert (`backlot.time.timer.get`); ha fut, megmutatja a
  kezdést és a futásidőt, és felkínálja a folytatást, a leállítást megadott időponttal vagy az eldobást
  (`backlot.time.timer.discard`, megerősítéssel). A stopper indítása **nem auditált üzleti tény** (átmeneti), a
  bejegyzés leállításkor keletkezik és auditált.
- A stopper csak **az adott személyé** (`os_user_ref`, M1: a helyi felhasználó); másét nem lehet leállítani.

### 4.3 Időlap és jóváhagyás

- **Időlap = (produkció, személy, ISO-hét).** A G1 „erőforrás + hét” időlapot említett; a bejegyzések
  produkcióhoz kötöttek, a jóváhagyást a **produkció** felelős szerepe adja (#42), ezért produkciónként külön
  időlap van. Egy személy hetén átívelő több produkció így több időlapot jelent, mindegyiket a saját
  produkció `producer`-e/`owner`-e hagyja jóvá.
- Az első bejegyzés felveszi az időlapot (`get-or-create`, a `work_date` hete szerint), ha még nincs; az
  időlap állapota a folyamat kezdőállapota. Az időlap egy bejegyzés **áthelyezésekor** (`work_date` más hétre) is
  követi a bejegyzést.
- **Folyamat** (`timesheet_default`, a G2-04 formátumában, migrációban):

| Átmenet | Honnan → hová | Szerep | Őrfeltétel / megjegyzés |
|---|---|---|---|
| `submit` | `open` / `returned` → `submitted` | a személy maga (vagy `owner`/`producer` a nevében) | `no_running_timer`, `has_entries` |
| `return` | `submitted` → `returned` | `producer`, `owner` | megjegyzés kötelező (`WF_COMMENT_REQUIRED`) |
| `approve` | `submitted` → `approved` | `producer`, `owner` | – |

- **Őrfeltételek** (új, zárt predikátumok, a G2-04 regiszterébe): `no_running_timer` (a személynek nincs futó
  stopperje; a beadás előtt le kell állítani), `has_entries` (az időlapon van nem törölt bejegyzés).
- `approved` végállapot; további módosítás csak ellenbejegyzéssel (4.1). `returned` után a bejegyzések újra
  szerkeszthetők, és az időlap újra beadható.
- **Önjóváhagyás:** M1-ben a helyi felhasználó minden szerep birtokosa (#42), így maga hagyhatja jóvá az
  időlapját. Többfelhasználós használatra a `actor_not_subject` predikátum (a G2-13 `actor_not_author` mintájára)
  a produkció aktivált folyamatdefiníciójában bekapcsolható; alapból kikapcsolt.
- Beadott vagy jóváhagyott időlap bejegyzése nem szerkeszthető és nem törölhető; a `returned` állapot oldja fel.

## 5. Riportok és összesítések

- **`backlot.time.report`**: zárt szempontlistából csoportosít (`task`, `entity` (részfa-szint), `person`,
  `category`, `production`, `task_type`, `week`, `month`), a G2-07 közös szűrőnyelvével (`time_entry` típus: produkció,
  feladat, elem-részfa az `ltree`-vel, személy, kategória, időszak, időlapállapot). Mértékek: `logged_minutes`
  (nettó, javításokkal), `entries`, és ahol van becslés: `estimate_minutes`, `variance_minutes`
  (`logged − estimate`), `variance_percent`.
- **Becslés és tény:** feladatonként a rögzített összeg és a becslés összevetése; elem-részfánál a részfa
  feladatainak összege. **Becslés nélküli feladat** nem számít a becslés oldalán (a felület jelzi a számát), nem
  nulla.
- **A hátralévő idő** a riportban és a feladatnézetben: `max(becslés − rögzített, 0)`; ha a rögzített idő a becslést
  meghaladja és a feladat nyitott, `over_estimate` jelölés (a becslés frissítésére a felhasználó dönt).
- **Összesítés a jóváhagyás szerint:** a riport mutatja, mennyi a jóváhagyott, a beadott és a nyitott rész
  (időlapállapot szerint), mert a költségszámítás (24.) csak a jóváhagyottal számolhat.
- **Egy igazság:** a riport közvetlenül a `time_entry` sorokból számol (nincs tárolt összeg), a lefúrt lista
  elemszáma és összege egyezik a kimutatással (a G2-06 szabálya).
- **Export:** CSV (`production, task, person, category, date, minutes, kind, timesheet_status`; UTF-8, RFC 4180)
  és PDF (időlap vagy riport; `QPdfWriter`, mint a G2-13). Az export a hozzáférési szabály szerint szűrt
  (7. pont), a művelet auditált (`time.exported`, tartalom nélkül).

## 6. Parancsok, események, hibakódok

| Parancs | Mit csinál |
|---|---|
| `backlot.time.start` / `.stop` / `.timer.get` / `.timer.discard` | Stopper (4.2) |
| `backlot.time.entry.add` / `.update` / `.delete` / `.list` / `.get` | Kézi bejegyzés; szerkesztés és „törlés” nyitott időlapon |
| `backlot.time.entry.correct` | Ellenbejegyzés jóváhagyott időlapra |
| `backlot.time_category.*` | Kategóriák (létrehozás, átnevezés, visszavonás) |
| `backlot.timesheet.get` / `.list` | Időlap bejegyzésekkel, napi és heti összeggel |
| `backlot.timesheet.submit` / `.return` / `.approve` | A G2-04 szerint generált átmenetparancsok |
| `backlot.time.report` / `.export` | Riport és export (5. pont) |

**Események** (a G2-12 `backlot` domainjében, outbox, kritikus): `backlot.timesheet.submitted`,
`backlot.timesheet.approved`, `backlot.timesheet.returned`. A címzettek: beadáskor az időlap produkciójának
`producer`/`owner` tagjai (új szabályelem a G2-12 zárt listájában: `production_approver`), jóváhagyáskor és
visszaküldéskor az időlap személye. A payload azonosítókat és a hetet viszi, **perceket és megjegyzést nem**.

Hibakódok: `TIME_ENTRY_INVALID`, `TIME_ENTRY_TOO_SHORT`, `TIME_ENTRY_OVERLAP` (figyelmeztetés),
`TIME_TASK_RETIRED`, `TIME_TIMER_RUNNING`, `TIME_TIMER_STALE`, `TIME_TIMER_NOT_FOUND`, `TIME_TIMESHEET_LOCKED`,
`TIME_CORRECTION_INVALID`, `TIME_CATEGORY_NOT_FOUND`, `TIME_REPORT_RANGE_TOO_LARGE`, `TIME_EXPORT_FAILED`,
valamint a `WF_*` és `TASK_*` kódok változatlanul.

## 7. Hozzáférés és személyes adat (SEC-06)

- **Részletes bejegyzést** (ki, mikor, mennyit, megjegyzés) a **személy maga** és a bejegyzés **produkciójának
  `owner`/`producer` tagja** lát. A többi tag **csak összesítést** lát feladatonként és részfánként, **személy
  szerinti bontás nélkül** (`time.report` `person` csoportosítása nekik nem adható, a válasz a rájuk vonatkozó
  rész nélkül marad).
- M1-ben a helyi felhasználó tulajdonos, mindent lát; a szabály a lekérdezésben **már most él** (a G2-07 elve), hogy
  M2-ben ne kelljen átírni.
- A **megjegyzés** (`note`) szabad szöveg, személyes adatot tartalmazhat: az audit a bejegyzés megjegyzését
  **redaktálja** (G2-01), a riport csak a jogosultnak mutatja.
- **Törlési/anonimizálási kérés:** a bejegyzések a `person` sorhoz kötődnek. A `person` anonimizálása (G2-05)
  után az idők **megmaradnak az összesítésben** (költség- és elszámolási adat), de a személy már nem azonosítható; a
  `note` mezők törlődnek. Hogy egy konkrét kérésnél mit kell törölni vagy megőrizni, jogi kérdés, nem a
  rendszeré: a rendszer a **két lehetőséget** biztosítja (anonimizálás megőrzéssel, vagy a bejegyzések
  visszavonása nyitott időlapon). Az auditlánc a redaktálás miatt mindkét esetben ép marad.
- **Személyenkénti időadat nem hagyja el a gépet** (SEC-02); az export a felhasználó által megadott helyre ír.

## 8. Never crash

- A felület bezárása, az összeomlás vagy az áramszünet nem veszíti el a futó stoppert (adatbázisban van); a
  megszakadt írás nem hagy félbe maradt bejegyzést (egy tranzakció).
- Hibás vagy ellentmondó bemenet (negatív vagy nulla perc, jövőbeli befejezés, kezdés a befejezés után, ismeretlen
  kategória, más produkció feladata): angol hibakód, nincs írás, nincs összeomlás.
- A rendszeróra ugrása (előre vagy hátra) a stopper leállításánál kezelt hiba vagy `STALE` jelzés.
- Riport túl nagy időszakra: `TIME_REPORT_RANGE_TOO_LARGE` és lapozás, nem időtúllépés.
- Az export hibája (nincs hely, nincs jog): `TIME_EXPORT_FAILED`, a részben írt fájl nem marad meg.

## 9. Kiegészítés a G2-15-höz: hátralévő munka

A G2-15 4.1 pontja a hátralévő munkát a becslésből vette, a 16. lépcső pontosítását jelezve. **A számítás a
`backlot.task_remaining_minutes` SQL-nézetből jön** (a G2-15 ezt a nézetet olvassa); a 15. lépcsőben a nézet
a becslést adja a nyitott feladatokra, **ebben a lépcsőben a migráció újradefiniálja:**

```
task_remaining_minutes = CASE WHEN a feladat lezárt vagy visszavont THEN 0
                              ELSE greatest(estimate_minutes − net_logged_minutes, 0) END
net_logged_minutes     = SUM(minutes) a feladat nem törölt bejegyzésein, minden személytől és időlapállapotból
```

- A **minden személytől** és **minden időlapállapotból** számítás szándékos: a tervezés a ténylegesen ledolgozott
  időt vegye figyelembe, ne csak a jóváhagyottat.
- Becslés nélküli feladat továbbra sincs a hátralékban (`no_estimate`, G2-15).
- Ez a G2-15 képletének frissítése, új kampó vagy tábla nélkül; a G2-15 szövegét a megvalósításkor kell
  igazítani.

## 10. Sorrend és függés

| Rész | Kell hozzá |
|---|---|
| Stopper, bejegyzés, kategória, időlap, jóváhagyás, riport | 2., 4., 5. lépcső megvalósítása |
| Hátralévő munka pontosítása | 15. lépcső (a nézet) |
| Értesítések | 12. lépcső, EF-R1 |
| Munkaköltség | 24. lépcső |

## 11. Tesztek (G5-re előre)

- **Stopper:** indítás, leállítás, váltás (`switch`), egyidejű indítás két kérésből (egy nyer, egy
  `TIME_TIMER_RUNNING`); a szolgáltatás **leállítása futó stopper mellett**, újraindítás után a stopper és a kezdés
  megvan; gyanús stopper (`STALE`) kifejezett `ended_at` nélkül elutasítva; óraugrás visszafelé.
- **Éjfél és DST:** stopper éjfélen át (szétvágás helyes napokra és időlapokra), nyári/téli átállás napján, több éjfélen át.
- **Napi korlát és átfedés:** két egyidejű írás a 24 óra határán nem léphet át (tanácsadó zár); átfedés
  figyelmeztetés, csak időtartamos bejegyzés nem ütközik; a korlát a **több produkción át** összeadódik.
- **Kerekítés:** a Python és a CHECK egyezik a félperces határokon; 29 és 30 másodperc; legalább 1 perc.
- **Időlap:** beadás futó stopperrel elutasítva; üres időlap nem adható be; visszaküldés megjegyzés nélkül
  elutasítva; jóváhagyás után szerkesztés és törlés elutasítva; ellenbejegyzés (előjeles, indokkal, nem megy 0
  alá); az időlap követi az áthelyezett bejegyzést; produkciónként külön időlap.
- **Riport:** egyezik a nyers `time_entry` összegekkel minden csoportosításra (property-teszt, javításokkal együtt);
  a lefúrt lista egyezik a kimutatással; becslés nélküli feladat nem torzítja a variance-t; elem-részfa az `ltree`-vel.
- **Hátralévő munka:** a G2-15 terhelése a rögzített idő után csökken, a becslést meghaladó rögzítésnél a hátralék 0
  (`over_estimate`), lezárt feladatnál 0.
- **Hozzáférés:** más szerepű felhasználó nem lát részletes bejegyzést és személyes bontást; az audit a
  megjegyzést redaktálja; anonimizálás után az összesítés megmarad és az `audit.verify` sikeres.
- **Never crash:** hibás bemenetek fuzzolása (negatív perc, jövőbeli idő, ismeretlen azonosítók), riport
  nagy időszakra, export hibája.
- **Felületi ellenőrzés:** `ui.screenshot` a stopperről, az időlapról és a riportról.
- **Mérés** a CFA4-MIN gépen: 100 000 bejegyzés, riport csoportosításonként, időlap betöltése, a napi korlát
  ellenőrzésének ideje (a G2-07 S és L profiljának mintájára).

## 12. Licenc (G3-ra előre)

Új függőség nincs: a `zoneinfo` a Python standard könyvtára; a PDF `QPdfWriter` (PySide6, mint a G2-13). Stalker
(LGPL-3.0, `TimeLog`), Kitsu/Zou (AGPL-3.0, napi összesített ráfordítás) és Kimai (AGPL-3.0, stopper, kategóriák,
jóváhagyás): funkcionális és szerkezeti referencia, kód nem.

## 13. G2-eredmény

**Javaslat: PASS.** Új döntési kérdés nincs. A megvalósítás az 5. lépcső után indulhat. A G2-15 hátralévő-munka
képlete (9. pont) és a G2-12 zárt címzettlistája (`production_approver`) a fenti kiegészítést kapja
megvalósításkor, új tábla nélkül.
