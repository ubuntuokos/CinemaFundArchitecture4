# BacklotHub 15. lépcső – Resource Planning: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-21 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.9, F6 · **Előzmény:**
[G1 Resource Planning](CFA4-BACKLOTHUB-G1-15-RESOURCE-PLANNING-2026-11-06.md) (funkcionális követelmények
jóváhagyva 2026-11-10) · **Épül rá:** G2-02 (produkció, ország), G2-05 (feladat, becslés, kiosztás, függőség),
G2-06 (csempe-regiszter), G2-12 (események) · **Állapot:** G2-javaslat, jóváhagyásra vár · implementáció nincs

Új döntési kérdés nincs. Új függőség egy van: `holidays` (MIT, Ubuntu: `python3-holidays`), a G3 kapuján
ellenőrzendő (11. pont). Az automatikus ütemező (OR-Tools) **nem része** ennek a lépcsőnek (8. pont).

## 1. Hatókör ebben a lépcsőben

- **Erőforrások** (M1-ben személyek), **csapatok**, **munkanaptár**, szabadság és ünnepnap.
- **Terhelés, ütközés, túlterhelés** számolt nézetként (nem tárolt adat).
- **Kézi ütemezés segítséggel:** Gantt-nézet, függőségek, kritikus út, függőségsértés-jelzés, eltolás
  következményekkel.
- **Ütemterv-változat** („mi lenne, ha”) elfogadásig az élő terv érintése nélkül.
- **Előrejelzés:** várható befejezés feladatonként, csapatonként és produkciónként.
- **Nem része:** tényleges ráfordítás és időnyilvántartás (16. lépcső), forgatási naptár és naptári események
  (17.), stábadatok (18.), eszközfoglalás (21.), automatikus kiegyenlítés.

## 2. Integrációs határok

| Határ | Irány | Szerződés |
|---|---|---|
| Task (5.) | ← | `task`: `estimate_minutes`, `start_date`, `due_date`, `status_id` (állapotkategória), `priority`; `task_assignment` (`assignee`); `task_dependency`. Írás csak a `backlot.task.*` parancsokon át (a Gantt-húzás és az elfogadott változat is ezeket hívja, egy tranzakcióban) |
| Production (2.) | ← | `production.country` (ünnepnap), új `production.work_calendar_id` (2.3) |
| Workflow (4.) | ← | A „lezárt” feladat az állapotkategóriából (`closed`), nem kódból |
| Dashboard (6.) | → | Két csempe a csempe-regiszterbe (7. pont); a kapacitáscsempe ennél a lépcsőnél kapcsol be (G1) |
| Notification (12.) | → | `backlot.capacity.overload_detected` esemény (7. pont); a címzett a `resource_person` |
| Search (7.) | → | `SearchProvider`: erőforrás, csapat (név, szerepkör) |
| Audit (1.) | → | Minden írás, a változat elfogadása külön eseményként |
| TimeTracker (16.), Production Calendar (17.), Crew (18.), Equipment (21.) | később | A 16. a hátralévő munkát pontosítja (4.2); a 17. ugyanezeket a naptártáblákat használja; a 18. és a 21. regisztrálja a saját erőforrástípusát (2.1) |

### 2.1 Az erőforrás fogalma

- Az **erőforrás** a végleges azonosító (G1): a 18. lépcső a stábtagot ehhez köti. Az eszköz (21.) **nem** erőforrás-sor
  (G2-21 2.1: nincs személye és kapacitása), a `kind` lista marad `person`. M1-ben csak `kind = 'person'`, a `backlot.person` (G2-05, #53) egy sorához kötve.
- Az erőforrás és a csapat **nem produkcióhoz kötött** (mint a `person`): ugyanaz az ember több produkción
  dolgozhat, és a túlterhelést **minden produkción át** össze kell adni. Egy erőforrás terhelésének
  **részletét** a felhasználó csak azokból a produkciókból látja, amelyeknek tagja (#42); a többi produkció
  hozzájárulása egy összegként („más produkciók: 12 ó”) jelenik meg, feladatok nélkül. M1-ben a helyi
  felhasználó mindenben tag, ezért mindent lát; a szűrés a lekérdezésben már most él (M2-re kész).
- Más erőforrástípus (eszköz, helyiség) **nem készül elő** ebben a lépcsőben, és nincs hozzá üres tábla vagy
  elágazás: az eszköz saját modellt kap (G2-21), a `kind` lista nem bővül.

### 2.2 „Ma” és a dátumok

Minden érték **dátum vagy perc**, óra nincs. A „ma” a szolgáltatás helyi dátuma (rendszer-időzóna); a számítás
paramétere is lehet (`as_of`), így a tesztek és az előrejelzés determinisztikusak.

### 2.3 Produkció és munkanaptár

A munkanaptár a produkció alapértelmezett naptára: a `production` tábla új `work_calendar_id` oszlopot kap
(`REFERENCES backlot.work_calendar`, a 15. lépcső migrációjában, a G2-02 kiegészítése). Ha nincs megadva, a
produkció országának alapnaptára (`<ország>-default`, ha van; különben a rendszer alapnaptára, hétfő–péntek,
8 óra, ünnepnap nélkül). A kapcsolat a függőségek késleltetésének és az eltolásnak a munkanapszámolásához kell
(3.3).

## 3. Adatmodell (DDL-vázlat, `backlot` séma)

### 3.1 Erőforrás, csapat, naptár

```sql
CREATE TABLE backlot.resource (
    id uuid PRIMARY KEY,
    kind text NOT NULL CHECK (kind IN ('person')),        -- a 18./21. lépcső bővíti
    person_id uuid NOT NULL REFERENCES backlot.person(id) ON DELETE RESTRICT,
    roles text[] NOT NULL DEFAULT '{}',                    -- szerepkörök: animátor, kompozitor, …
    note text,
    retired_at timestamptz,
    ...közös oszlopok production_id nélkül,
    UNIQUE (person_id)
);

CREATE TABLE backlot.team (
    id uuid PRIMARY KEY, code text NOT NULL, name text NOT NULL, label_key text,   -- label_key: a beépített részlegeknél (G2-18); különben a name jelenik meg
    kind text NOT NULL CHECK (kind IN ('department','team')),
    production_id uuid REFERENCES backlot.production(id),   -- NULL: produkciófüggetlen
    retired_at timestamptz,
    ...közös oszlopok production_id nélkül,
    UNIQUE (code, production_id)
);

CREATE TABLE backlot.team_member (
    team_id uuid REFERENCES backlot.team(id), resource_id uuid REFERENCES backlot.resource(id),
    valid_from date NOT NULL, valid_until date,
    PRIMARY KEY (team_id, resource_id, valid_from),
    CHECK (valid_until IS NULL OR valid_until >= valid_from)
);

CREATE TABLE backlot.work_calendar (
    id uuid PRIMARY KEY, code text NOT NULL UNIQUE, name text NOT NULL,
    country char(2),                                        -- ISO 3166-1; az ünnepnap-generáláshoz
    weekly_minutes smallint[] NOT NULL CHECK (array_length(weekly_minutes,1) = 7),   -- hétfő..vasárnap, percben
    ...közös oszlopok production_id nélkül
);

CREATE TABLE backlot.resource_calendar_period (            -- a munkaidő időszakonként (részmunkaidő, váltás)
    resource_id uuid REFERENCES backlot.resource(id),
    valid_from date NOT NULL, valid_until date,
    calendar_id uuid NOT NULL REFERENCES backlot.work_calendar(id),
    percent smallint NOT NULL DEFAULT 100 CHECK (percent BETWEEN 1 AND 100),
    PRIMARY KEY (resource_id, valid_from),
    CHECK (valid_until IS NULL OR valid_until >= valid_from)
);

CREATE TABLE backlot.calendar_exception (                  -- ünnepnap, szabadság, rendkívüli munkanap
    id uuid PRIMARY KEY,
    calendar_id uuid REFERENCES backlot.work_calendar(id), -- a naptár minden használójára …
    resource_id uuid REFERENCES backlot.resource(id),      -- … vagy egy erőforrásra (pontosan az egyik)
    date_from date NOT NULL, date_to date NOT NULL CHECK (date_to >= date_from),
    kind text NOT NULL CHECK (kind IN ('holiday','leave','other_off','extra_work')),
    minutes_override smallint CHECK (minutes_override BETWEEN 0 AND 1440),  -- NULL: egész nap szabad / a naptár szerint
    label text,                                            -- erőforrás-szintűnél személyes adat (audit: redaktált)
    source text NOT NULL CHECK (source IN ('manual','generated')),
    ...közös oszlopok production_id nélkül,
    CHECK ((calendar_id IS NULL) <> (resource_id IS NULL))
);
CREATE INDEX ON backlot.calendar_exception (calendar_id, date_from, date_to);
CREATE INDEX ON backlot.calendar_exception (resource_id, date_from, date_to);
```

- **Napi kapacitás** egy erőforrásra egy napon:
  1. a `resource_calendar_period` szerinti naptár hétköznapi perce × `percent` / 100;
  2. a naptár `holiday`, `other_off` kivétele (egész napra 0, vagy a `minutes_override`);
  3. az erőforrás saját `leave`, `other_off` kivétele (0 vagy `minutes_override`);
  4. `extra_work`: a `minutes_override` a nap kapacitása (hétvégi munka).
  Ha két kivétel esik ugyanarra a napra, a **kisebb kapacitás** nyer, kivéve az `extra_work`-öt, amely a
  naptári nap alapját írja felül, és az erőforrás saját szabadsága ezt is nullázza. A szabály egy helyen
  (SQL-függvény, `backlot.resource_day_capacity`) van, a nézetek és a Gantt ugyanazt használják.
- **Ünnepnapok generálása:** `backlot.calendar.holidays.generate {calendar, years}` a `holidays` csomagból
  `calendar_exception` sorokat ír (`kind = 'holiday'`, `source = 'generated'`), a naptár `country` mezője szerint.
  **A generált sorok tárolódnak**, nem futásidőben számolódnak: így egy későbbi csomagfrissítés (törvényváltozás)
  nem írja át csendben a múltbeli tervet. Újragenerálás kifejezett parancs, amely a `manual` sorokat nem érinti, és
  előbb megmutatja a különbséget. Ünnepnap bármikor kézzel is felvehető; a csomag nélkül is teljes a
  működés.
- **Szabadság és személyes adat (SEC-06):** a szabadság oka (betegség, családi ok) egészségügyi vagy más
  személyes adat lehet, ezért **nincs** `sick`-jellegű típus, a `kind` csak `leave` vagy `other_off`, és a
  `label` az erőforrás-szintű sorokon személyes mező (audit redaktálja, G2-01). Más szereplőnek a szabadság
  „nem elérhető” jelzésként látszik, felirat nélkül; a feliratot a tulajdonos és maga az érintett látja.
- **Jóváhagyás a szabadságra nincs** (nem része az M1-nek): a szabadságot a tulajdonos vagy a koordinátor
  rögzíti. Kérés–jóváhagyás folyamat a 18. lépcső (Crew) G2-jében dönthető el.

### 3.2 Ütemterv-változat

```sql
CREATE TABLE backlot.schedule_scenario (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    name text NOT NULL, note text,
    state text NOT NULL CHECK (state IN ('open','accepted','discarded')),
    accepted_at timestamptz,
    ...közös oszlopok,
    UNIQUE (production_id, name)
);

CREATE TABLE backlot.schedule_scenario_task (            -- csak az eltérések, nem teljes másolat
    scenario_id uuid NOT NULL REFERENCES backlot.schedule_scenario(id) ON DELETE CASCADE,
    task_id uuid NOT NULL REFERENCES backlot.task(id),
    base_row_version integer NOT NULL,                      -- a feladat állapota a változatba vételkor
    start_date date, due_date date,                          -- NULL = változatlan
    estimate_minutes integer,                                -- NULL = változatlan
    PRIMARY KEY (scenario_id, task_id)
);

CREATE TABLE backlot.schedule_scenario_assignment (
    scenario_id uuid NOT NULL REFERENCES backlot.schedule_scenario(id) ON DELETE CASCADE,
    task_id uuid NOT NULL REFERENCES backlot.task(id),
    person_id uuid NOT NULL REFERENCES backlot.person(id),
    op text NOT NULL CHECK (op IN ('add','remove')),
    PRIMARY KEY (scenario_id, task_id, person_id)
);
```

- **Eltérés-tárolás:** a változat csak azt őrzi, ami különbözik az élő tervtől. Így nincs másolat, amely
  elavulna, és a változat mérete a módosított feladatok számával arányos.
- A terhelés, az ütközés, a kritikus út és az előrejelzés a **változatra is** kiszámolható
  (`scenario` paraméter): ugyanaz a kód, az élő adatra rétegezett eltérésekkel. A felület az élő terv és a
  változat különbségét mutatja (ki lett túlterhelt, mi csúszott, mikor lesz kész).
- **Elfogadás** (`backlot.schedule.scenario.accept`): **csak kifejezett parancsra**, és csak akkor, ha az érintett
  feladatok `row_version`-je még egyezik a `base_row_version`-nel. Ha valamelyik időközben megváltozott:
  `SCHEDULE_SCENARIO_STALE`, a felsorolt feladatokkal, **részleges alkalmazás nincs**; a felhasználó dönt
  (frissíti a változatot vagy elveti). Sikeres elfogadás egy tranzakcióban a `backlot.task.*` parancsokkal
  írja az élő tervet (a G2-05 szabályaival és őrfeltételeivel, függőségkör-ellenőrzéssel), minden feladatról
  auditbejegyzéssel (`via = 'scenario'`, a változat azonosítójával), majd `accepted` állapotba lép. Ha egy
  parancs elbukik, semmi nem íródik.
- Egy nyitott változat a produkció tagjának látható (M1: a tulajdonos); törlés helyett `discarded`.

## 4. Terhelés, ütközés, előrejelzés (számolt nézetek)

Minden érték **lekérdezéskor számolódik**, tárolt terhelés vagy jelző nincs (mint a Dashboardnál, G2-06), így nem
térhet el a feladatoktól. A számítás SQL-ben (`generate_series` a napokra, a `resource_day_capacity`
függvény a kapacitásra) és kis, determinisztikus Python-kódban fut; véletlen vagy időfüggés nincs az `as_of`
dátumon kívül.

### 4.1 Mit számít terhelésnek

- Egy **feladat hátralévő munkája** a `backlot.task_remaining_minutes` SQL-nézetből jön. A 15. lépcsőben ez
  az `estimate_minutes`, ha a feladat nem `closed` állapotkategóriájú (és nem visszavont), különben 0. Becslés nélküli feladat (`estimate_minutes IS NULL`) **nem ad terhelést**, de
  `no_estimate` jelzést kap a listában (a hiányzó adat látható, nem csendben nulla).
- **Többször kiosztott feladat:** a hátralévő munkát a `assignee` szerepű személyek **egyenlően** osztják el;
  a `contributor` és a `reviewer` szerep **nem** terhel (az ő idejüket a G1 szerint nem tervezzük). Kiosztott
  személy nélküli feladat a **„kiosztatlan munka”** összegébe kerül, erőforráshoz nem.
- **Eloszlás a napokra:** a feladat ablaka `max(start_date, ma)` és `due_date` között van. A hátralévő munka az
  ablak **munkanapjaira** oszlik, **a nap kapacitásával arányosan** (egy félnapos munkanap fele terhelést kap, a
  szabadnap semmit). Az ablak tagjai az adott **erőforrás** napjai, a kapacitás is az övé.
- **Nincs `due_date`:** a feladat a „nem ütemezett munka” összegbe kerül, napi terhelést nem ad. **Lejárt
  határidő** (`due_date < ma`, nem lezárt): a hátralék a **mai napra** kerül („lejárt hátralék”, külön jelölve), mert
  azt most kellene elvégezni.
- Terhelés = a napi eloszlott percek összege az **összes produkción át** (2.1); a felület produkció szerint is
  bontja (a hozzáférési szabállyal).
- **16. lépcső (TimeTracker) után:** a nézetet a 16. lépcső migrációja újradefiniálja:
  `max(becslés − rögzített idő, 0)` ([G2-16](CFA4-BACKLOTHUB-G2-16-TIMETRACKER-2026-11-21.md) 9. pont). A terhelés
  számítása változatlan, mert csak a nézetet olvassa; a 15. lépcsőben tényleges idő még nincs, ezért a nézet a
  becslést adja.

### 4.2 Ütközések és túlterhelés

| Jelzés | Feltétel | Súly |
|---|---|---|
| `overload` | Egy erőforrás napi terhelése nagyobb a napi kapacitásánál (a küszöb beállítható, alapból 100%) | hiba (piros) |
| `on_leave` | Feladat `start_date`-je vagy `due_date`-je az erőforrás **szabadnapjára** esik, vagy az ablakában nincs egyetlen munkanapja sem | figyelmeztetés |
| `no_capacity` | A feladat ablakában az erőforrás összes kapacitása kisebb a hátralévő munkánál, és nincs `overload` (pl. rövid ablak, kevés nap) | figyelmeztetés |
| `no_estimate` | Kiosztott, nyitott feladat becslés nélkül | információ |
| `dependency_violated` | Az utód kezdete a megelőző végénél (+ késleltetés) korábbi (3.3) | hiba |

- Az `overload` **napi** alapú: az eredmény az erőforrás, a nap, a terhelt és a rendelkezésre álló perc, és
  a hozzájáruló feladatok listája (a hozzáférési szabállyal). Összefüggő napok egy tartományba vonva.
- A jelzések a felületen (Gantt, erőforrásnézet, csempe) és a `backlot.resource.conflicts` paranccsal
  érhetők el; **az ütközés nem tilt**: a felhasználó túlterhelt tervet is menthet, a rendszer jelez.

### 4.3 Előrejelzés

- **Elv:** egy erőforrás a hozzá kiosztott nyitott feladatait **egymás után** végzi (nincs párhuzamos munka),
  határidő szerint növekvő sorrendben, azonos határidőnél prioritás szerint (`urgent` előre), végül az azonosító
  szerint (stabil). Minden feladat a következő szabad kapacitású napoktól fogy `ma`-tól, a napi kapacitás
  szerint. Ebből adódik a feladat **várható befejezési napja** (`projected_finish`).
- Eredmény: **feladatonként** (késik-e a határidőhöz képest: `projected_late`), **erőforrásonként** (mikor
  szabadul fel), **csapatonként** (a tagjaihoz kiosztott feladatok legkésőbbi várható napja) és **produkciónként**
  (a produkció feladatainak legkésőbbi várható napja).
- A **kiosztatlan munka** és a **nem ütemezett munka** nem szerepel a várható napban, **külön összegként**
  jelenik meg (perc és munkanapnyi ekvivalens egy átlagos kapacitás mellett), hogy az előrejelzés ne tűnjön
  teljesebbnek, mint amennyi ismert.
- **Ez előrejelzés, nem terv:** a konzervatív „egy időben egy feladat” feltevést és a hiányzó becsléseket a
  felület kimondja a számok mellett. Az előrejelzés nem írja át a feladat dátumait.
- Teljesítmény: lásd a 10. pontban (mérés).

## 5. Kézi ütemezés

### 5.1 Gantt-nézet

- Qt `QPainter`-rajzolt egyedi vezérlő (mint a Dashboard csempéi, G2-06; nincs új grafikai függőség): sorok
  feladatonként (a szerkezeti elem szerint csoportosítva, az `ltree` útvonallal, G2-03), napi, heti és havi
  nagyítás, a függőségek nyilakkal, a `projected_finish` jelölés, a túlterhelés és a szabadnap sávszínezése, a
  mai nap jelölése.
- **Virtualizálás:** csak a látható sorok és napok rajzolódnak; néhány ezer feladat is folyamatosan görgethető
  (mérés a 10. pontban).
- **Szerkesztés:** a sáv húzása (eltolás, hosszváltoztatás) a `backlot.task.update` parancsot hívja a G2-05
  szabályaival; **előnézet a mentés előtt** (milyen függőség sérül, ki lesz túlterhelt), és csak megerősítésre ír.
- Az erőforrás-nézet erőforrásonként mutatja a napi terhelést és a kapacitást, a hozzájáruló feladatokkal.
- Minden ábra **adatparancson** át jön (`backlot.schedule.gantt`, `backlot.resource.load`), a CLI és az MCP is
  eléri (a G2-06 elvével: a felület nem tud olyat, amit a parancs nem).

### 5.2 Függőségek és kritikus út

- A `task_dependency` két fajtája: `finish_to_start`, `start_to_start` (G2-05). **A `lag_days` munkanap** a
  produkció munkanaptárában (2.3), nem naptári nap (a G2-05 oszlopának pontosítása; negatív érték nem megengedett,
  a CHECK a migrációban).
- **Függőségsértés:** `finish_to_start`: az utód `start_date`-je kisebb a megelőző `due_date`-jénél a késleltetéssel
  eltolt következő munkanapnál; `start_to_start`: az utód kezdete korábbi a megelőzőénél a késleltetéssel.
  Hiányzó dátumú feladat a számításból kimarad (jelezve), nem hiba.
- **Kritikus út:** a klasszikus előre-hátra számítás (CPM) a produkció dátumozott feladatain, a **munkanaptár
  napjaival**. A feladat időtartama a `start_date`–`due_date` közti munkanapok száma. A produkció végét a
  legkésőbbi `due_date` adja; a **kritikus** feladatok a nulla tartalékúak. A körmentességet a G2-05
  (`TASK_DEPENDENCY_CYCLE`) már biztosítja. Összetett függőséghálón is determinisztikus (azonos tartalék esetén
  azonosító szerinti sorrend).
- **Eltolás következményekkel** (`backlot.schedule.shift {task, working_days, cascade, dry_run}`): a feladatot
  és (ha `cascade`) az utódait a munkanaptár szerint tolja el, ahol a függőség ezt megköveteli; `dry_run` esetén
  csak az eredményt adja (mely feladat, mennyit, új ütközések). Ténylegesen a `backlot.task.update`-eken át ír,
  egy tranzakcióban, egy auditeseményként (`via = 'schedule.shift'`). Ha egy feladat lezárt állapotú, az
  eltolás **megáll** előtte és jelzi; lezárt feladat dátumát nem módosítja.

## 6. Parancsok és hibakódok

| Parancs | Mit csinál |
|---|---|
| `backlot.resource.create` / `.update` / `.retire` / `.list` / `.get` | Erőforrások (M1: személyből) |
| `backlot.team.*`, `backlot.team.member.add` / `.remove` | Csapatok, részlegek, tagság időszakkal |
| `backlot.calendar.*`, `backlot.calendar.exception.add` / `.update` / `.remove` / `.list` | Munkanaptárak, kivételek |
| `backlot.calendar.holidays.generate` | Ünnepnapok generálása a `holidays` csomagból (előnézettel) |
| `backlot.resource.calendar.set` | Munkaidő-időszak és részmunkaidő erőforrásra |
| `backlot.resource.load` | Napi terhelés és kapacitás (erőforrás, csapat, produkció, időszak, `scenario`) |
| `backlot.resource.conflicts` | Ütközések és túlterhelések (4.2) |
| `backlot.schedule.gantt` | A Gantt-nézet adata (feladatok, függőségek, jelzések) |
| `backlot.schedule.critical_path` | Kritikus út és tartalékok |
| `backlot.schedule.shift` | Eltolás következményekkel (`dry_run`-nal) |
| `backlot.schedule.forecast` | Előrejelzés (feladat, erőforrás, csapat, produkció) |
| `backlot.schedule.scenario.*` | Változatok: `create`, `set_task`, `set_assignment`, `compare`, `accept`, `discard`, `list` |

Hibakódok: `RESOURCE_NOT_FOUND`, `RESOURCE_PERSON_IN_USE`, `RESOURCE_CALENDAR_INVALID`,
`CALENDAR_EXCEPTION_INVALID`, `CALENDAR_HOLIDAYS_UNAVAILABLE` (a `holidays` csomag hiánya vagy ismeretlen ország),
`CALENDAR_COUNTRY_UNKNOWN`, `SCHEDULE_DEPENDENCY_VIOLATED`, `SCHEDULE_NO_DATES`, `SCHEDULE_SHIFT_BLOCKED`
(lezárt feladat az úton), `SCHEDULE_SCENARIO_STALE`, `SCHEDULE_SCENARIO_NOT_OPEN`, valamint a `TASK_*` és
`WF_*` kódok változatlanul.

## 7. Csempék, események, értesítés

- **Csempék** (a G2-06 csempe-regiszterébe, mert a kapacitáscsempe ennél a lépcsőnél kapcsol be):
  - `capacity.overloaded_resources`: a következő 14 napban túlterhelt erőforrások száma (a hozzáférési szabállyal),
    lefúrás az erőforrás-listára;
  - `capacity.utilization`: a következő 4 hét kihasználtsága százalékban (terhelés / kapacitás), produkcióra vagy
    csapatra szűrve; lefúrás a Gantt/erőforrásnézetre.
  A csempék számai a lefúrt lista elemszámával egyeznek (a G2-06 szabálya).
- **Esemény:** `backlot.capacity.overload_detected` (a G2-12 `backlot` domainjében, outbox, kritikus). Egy
  időzítő óránként és induláskor kiértékeli a következő 14 napot; **új** összefüggő túlterhelt tartományra egy
  eseményt ad. A kettős jelzés ellen:

```sql
CREATE TABLE backlot.capacity_signal (
    resource_id uuid NOT NULL REFERENCES backlot.resource(id),
    signal_key text NOT NULL,                     -- overload:<első nap>..<utolsó nap>
    event_id uuid NOT NULL,
    created_at timestamptz NOT NULL,
    PRIMARY KEY (resource_id, signal_key)
);
```

  A tartomány megváltozása új kulcsot, így új jelzést ad; a megszűnt túlterhelés nem küld külön eseményt
  (a lista és a csempe mutatja). A jelzés és az outbox-sor egy tranzakcióban íródik (G2-12 3.1).
- **G2-12 kiegészítés** (megvalósításkor a 12. lépcsőbe kerül, új tábla nélkül): a `notification_rule.recipients`
  zárt listája új elemmel bővül, `resource_person` (az érintett erőforrás személye); a `payload` csak
  azonosítókat és a tartományt viszi, **feladatcímet nem** (a hozzájáruló feladatokat a fogyasztó a
  hozzáférési szabállyal olvassa).
- **Nem történik semmi automatikusan:** a rendszer nem oszt át feladatot, nem tol el határidőt és nem
  zár le semmit a túlterhelés miatt.

## 8. Automatikus kiegyenlítés (nem része a lépcsőnek)

A G1 ezt „későbbi, opcionális” képességként jelölte (OR-Tools CP-SAT, Apache-2.0). **Javaslat: nem készül el ebben
a lépcsőben, és erre nem is készül elő hívó nélküli kód.** Akkor lesz külön allépcső (15-2, G3-mal), ha a kézi
ütemezés használata után igény mutatkozik. A helye már kész: az eredménye az ütemterv-változat lenne
(3.2), amelyet a felhasználó összehasonlít és kifejezetten elfogad; az élő tervet soha nem írná át közvetlenül
(G1, „csak javaslatként”). Az OR-Tools PyPI-kerékként jönne (Ubuntu-csomag nincs), mérete és a Python 3.14
támogatása a 15-2 G3-ján ellenőrzendő.

## 9. Never crash

- Hiányzó, hibás vagy ellentmondó adat (becslés nélküli feladat, dátum nélküli függőség, kivétel sor hibás
  időszakkal, ismeretlen ország): a nézet **kimarad vagy jelez**, nem omlik össze; a hibás elem azonosítóval
  listázódik.
- A `holidays` csomag hiánya: a generálás `CALENDAR_HOLIDAYS_UNAVAILABLE`-t ad, minden más működik.
- Túl nagy lekérdezés (sok erőforrás × hosszú időszak): lapozás és időszakkorlát (alapból legfeljebb 366 nap), a
  korlát túllépése `RESOURCE_RANGE_TOO_LARGE`, nem időtúllépés.
- A változat elfogadása közben bármilyen hiba: az élő terv változatlan (egy tranzakció).
- A Gantt-vezérlő rajzolási hibája a nézetet jelöli hibásnak, nem az alkalmazást.

## 10. Tesztek (G5-re előre)

- **Kapacitás:** a `resource_day_capacity` táblázatos tesztje (hétköznap, ünnepnap, szabadság, félnap,
  `extra_work`, részmunkaidő-időszak váltása, két kivétel ugyanarra a napra); a szabály egy helyen van, a Gantt és a
  terhelés ugyanazt adja.
- **Terhelés:** ismert feladathalmazra kézzel számolt napi értékek; több kiosztott személy egyenlő
  elosztása; contributor/reviewer nem terhel; lejárt hátralék a mai napon; dátum nélküli feladat a nem
  ütemezett összegben; **több produkción átívelő** terhelés összeadása, és a nem tag produkció csak összegként.
- **Property-teszt:** a napi terhelések összege egyenlő a feladat hátralévő munkájával (kerekítési maradék
  nélkül, a perc egész számú elosztásánál az utolsó nap viszi a maradékot); a terhelés nem negatív; ugyanaz az
  `as_of` ugyanazt adja.
- **Függőségek és kritikus út:** lánc, gyémánt, párhuzamos ágak, `start_to_start`, késleltetés munkanapban
  (hétvégén és ünnepnapon át); a CPM-tartalék egyezik egy kézzel levezetett példával; `dependency_violated` jelzés
  és az eltolás `dry_run`-ja egyezik az éles eltolással; lezárt feladat előtt az eltolás megáll.
- **Előrejelzés:** sorrend (határidő, prioritás, azonosító), szabadság által szétszakított kapacitás,
  kiosztatlan és nem ütemezett munka külön összegben, `projected_late`.
- **Változat:** eltérés-alapú kiszámítás egyezik azzal, mintha a változtatásokat valóban elvégeznénk; elfogadás
  sikere és `SCHEDULE_SCENARIO_STALE` (egy feladat közben módosult, semmi nem íródik); hibainjektálás az elfogadás
  közben (élő terv érintetlen); elvetés.
- **Jelzések és esemény:** a túlterhelés kétszeri kiértékelése egy eseményt ad; a tartomány változása újat;
  összeomlás az outbox-írás és az `capacity_signal` között (G2-12 összeomlási-ablak mintája).
- **Szerepek:** nem tag produkció feladatai nem látszanak a terhelés részletében; a szabadság felirata mások
  számára rejtett; az audit redaktálja a feliratot.
- **Never crash:** hibás és ellentmondó bemenetek fuzzolása (kivételsorok, függőségi adatok), `holidays`
  nélkül, túl nagy időszak.
- **Felületi ellenőrzés:** `ui.screenshot` a Gantt-nézetről és az erőforrásnézetről túlterheléssel,
  függőségekkel, kritikus úttal.
- **Mérés** a CFA4-MIN gépen: 10 000 feladat, 100 erőforrás, 90 napos terhelés, kritikus út és előrejelzés ideje;
  Gantt görgetése 5000 sorral (a G2-07 S és L profiljának mintájára); cél, hogy a lekérdezés interaktívnak
  számítson (a pontos küszöb a G5-ön rögzül a mérés után, mint a REV-célok).

## 11. Licenc (G3-ra előre)

- **Új függőség: `holidays`** (`vacanza/holidays`, MIT; Ubuntu: `python3-holidays`). A G3-on ellenőrizendő a
  rögzített verzió, a Python 3.14 támogatás, az Ubuntu-csomag verziója és a magyar naptár helyessége
  (a tesztben ismert évek). A működés nélküle is teljes (kézi ünnepnap), ezért a hiánya nem blokkol.
- **Meglévő:** python-dateutil (G2-05); PostgreSQL contrib semmi újat nem kér.
- **Referencia, kód nélkül:** Stalker (LGPL-3.0: munkaidő, szabadnap), Kitsu/Zou (AGPL-3.0: szabadnap, részleg,
  ütemterv-változatok), TaskJuggler (GPL-2.0, nem vesszük át, külső program sem fut). A CPM és az előrejelzés
  saját, klasszikus algoritmus.
- **Később:** OR-Tools (Apache-2.0) csak a 15-2 allépcsőben, külön G3-mal (8. pont).

## 12. G2-eredmény

**Javaslat: PASS.** Új döntési kérdés nincs. A megvalósítás az 5. lépcső után indulhat; az értesítés a 12.
lépcsőtől működik. A G2-02 (`production.work_calendar_id`), a G2-05 (`lag_days` munkanap), a G2-06 (két csempe)
és a G2-12 (`resource_person` címzett, `capacity_signal`) a fenti kiegészítést kapja megvalósításkor.
