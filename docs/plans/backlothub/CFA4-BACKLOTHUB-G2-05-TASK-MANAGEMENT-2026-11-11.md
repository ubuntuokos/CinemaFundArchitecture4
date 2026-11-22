# BacklotHub 5. lépcső – Task Management: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-11 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.3 · **Előzmény:**
[G1 Task Management](CFA4-BACKLOTHUB-G1-05-TASK-MANAGEMENT-2026-11-06.md) (funkcionális követelmények
jóváhagyva 2026-11-10; #43 eldöntve 2026-11-08) · **Épül rá:** G2-01…G2-04 (mind jóváhagyva 2026-11-11) ·
**Állapot:** G2 **jóváhagyva** (István, 2026-11-11; #53 eldöntve) · implementáció nincs

## 1. Függőségek

| Mi | Honnan | Állapot |
|---|---|---|
| Produkció, szerkezeti elem (`ltree`), státuszlista | 2., 3. lépcső | G2 jóváhagyva |
| Feladat-folyamat (állapotok, átmenetek, őrfeltételek) | 4. lépcső | G2 jóváhagyva; a feladat adaptere és alapfolyamata itt készül |
| Audit (előzmények forrása) | CFA4-AUDIT-001 | Terv a `main`-ben |
| Csatolmány | #43: az 5. lépcső csatolmány nélkül indul; bekapcsol a File Services FS-1 után (ADR 0009 a `main`-ben) | Eldöntve |
| Ismétlődés | python-dateutil (`rrule`), Ubuntu: `python3-dateutil` | Új függőség, G3-on ellenőrizve |

## 2. Új döntési kérdés: kihez lehet feladatot rendelni M1-ben? (#53)

A G1 szerint „az OS-felhasználó vagy egy név szerint felvett stábtag”. A stáb a 18. lépcső (Crew), a nevek
személyes adatok. Ha most egy ideiglenes „név” mezőt vennénk fel, a 18. lépcső után kettős nyilvántartás
lenne (ez a „nincs stub” szabályba ütközik).

**Javaslat (#53):** az 5. lépcső bevezeti a **közös `person` táblát** a személy minimális adataival (név,
megjelenített név, opcionális OS-felhasználó-kötés). A 18. lépcső ezt **bővíti** (elérhetőség, szerződés,
szakterület, díj), nem cseréli le. A Cast (19.) ugyanerre a táblára épül. Így a feladatkiosztás a végleges
modellre mutat, és a személyes adat egy helyen van (SEC-06: az auditba csak az azonosító kerül, a név nem).

## 3. Integrációs határok

| Határ | Irány | Szerződés |
|---|---|---|
| Szerkezet (3.) | ← | A feladat egy elemhez tartozik (`entity_id`), vagy közvetlenül a produkcióhoz (pl. „zenei jogok tisztázása”) |
| Workflow Engine (4.) | ↔ | `TaskAdapter`: a feladat `status_id`-je a hiteles állapot; alapfolyamat `task_default`; őrfeltétel-predikátumok: `all_subtasks_in_state`, `predecessors_done` |
| Audit | → | Minden módosítás egy esemény, `scope = {production_id, entity_path, task_id}`; az előzmény nézet az auditból |
| Megjegyzés | közös | A `comment` tábla bármely objektumhoz köthető; a 13. (Review) és a 14. (Document) is ezt használja |
| Resource Planning (15.), TimeTracker (16.) | → | Becslés, kiosztás, függőség; ütemezés és tényleges idő ott |
| Dashboard (6.), Search (7.) | → | Listák, összesítések; FTS a címre és a leírásra |
| Event Fabric | később | Kiosztási és határidő-értesítés a 12. lépcsőben; most nincs kapcsolat |
| Motor `jobs.*` | nincs | A gépi háttérfeladat („job”) más fogalom; a névtér `backlot.task.*`, ütközés nincs |

## 4. Adatmodell (DDL-vázlat, `backlot` séma)

```sql
CREATE TABLE backlot.person (                  -- #53; a 18. lépcső bővíti
    id            uuid PRIMARY KEY,
    display_name  text NOT NULL,                  -- megjelenített név vagy álnév; a teljes név és az elérhetőség a person_personal_detail-ben (G2-18)
    anonymized_at timestamptz,                    -- SEC-06 (G2-18): a display_name ekkor már semleges jelölő
    os_user_ref   text UNIQUE,                   -- pl. uid:1000/istvan; M1-ben a helyi felhasználó
    retired_at    timestamptz,
    ...közös oszlopok production_id nélkül
);

CREATE TABLE backlot.task_type (                -- konfigurálható: animáció, kompozitálás, hang …
    id uuid PRIMARY KEY, code text NOT NULL UNIQUE, label_key text NOT NULL,
    applies_to_entity_type_id uuid REFERENCES backlot.entity_type(id),  -- NULL: bármely / produkciószintű
    default_workflow_code text NOT NULL DEFAULT 'task_default',
    color text, ordinal smallint NOT NULL, retired_at timestamptz
);

CREATE TABLE backlot.task (
    id              uuid PRIMARY KEY,
    production_id   uuid NOT NULL REFERENCES backlot.production(id),
    entity_id       uuid REFERENCES backlot.production_entity(id) ON DELETE RESTRICT,
    parent_task_id  uuid REFERENCES backlot.task(id) ON DELETE RESTRICT,  -- alfeladat
    task_type_id    uuid NOT NULL REFERENCES backlot.task_type(id),
    title           text NOT NULL,
    description     text,
    status_id       uuid NOT NULL REFERENCES backlot.status(id),
    priority        text NOT NULL DEFAULT 'normal' CHECK (priority IN ('low','normal','high','urgent')),
    estimate_minutes integer CHECK (estimate_minutes >= 0),
    start_date      date,
    due_date        date CHECK (due_date IS NULL OR start_date IS NULL OR due_date >= start_date),
    actual_start_at timestamptz,               -- első „wip”-be lépés, a futtató tölti
    actual_end_at   timestamptz,               -- végállapotba lépés
    recurrence_id   uuid REFERENCES backlot.task_recurrence(id),
    occurrence_date date,
    retired_at      timestamptz,
    ...közös oszlopok,
    UNIQUE (recurrence_id, occurrence_date)
);
CREATE INDEX ON backlot.task (production_id, status_id);
CREATE INDEX ON backlot.task (entity_id);
CREATE INDEX ON backlot.task (due_date) WHERE retired_at IS NULL;

CREATE TABLE backlot.task_assignment (
    task_id   uuid REFERENCES backlot.task(id),
    person_id uuid REFERENCES backlot.person(id),
    role      text NOT NULL CHECK (role IN ('assignee','contributor','reviewer')),
    PRIMARY KEY (task_id, person_id, role)
);

CREATE TABLE backlot.task_dependency (
    predecessor_id uuid REFERENCES backlot.task(id),
    successor_id   uuid REFERENCES backlot.task(id),
    kind           text NOT NULL CHECK (kind IN ('finish_to_start','start_to_start')),
    lag_days       integer NOT NULL DEFAULT 0,
    PRIMARY KEY (predecessor_id, successor_id),
    CHECK (predecessor_id <> successor_id)
);

CREATE TABLE backlot.task_recurrence (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    template jsonb NOT NULL,                   -- a létrehozandó feladat mezői (típus, cím, elem, kiosztás)
    rrule text NOT NULL,                       -- RFC 5545, dátumpontosságú (DTSTART dátum)
    dtstart date NOT NULL, until date,
    materialized_until date,                   -- eddig jöttek létre a példányok
    active boolean NOT NULL DEFAULT true,
    ...közös oszlopok
);

CREATE TABLE backlot.comment (                  -- közös: feladat, elem, verzió, dokumentum …
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    subject_kind text NOT NULL, subject_id uuid NOT NULL,
    author_person_id uuid NOT NULL REFERENCES backlot.person(id),
    body text NOT NULL CHECK (length(body) <= 20000),   -- egyszerű Markdown, HTML nélkül
    reply_to_id uuid REFERENCES backlot.comment(id),
    transition_code text,                      -- ha a megjegyzés átmenettel együtt jött
    edited_at timestamptz, retired_at timestamptz,
    ...közös oszlopok
);
CREATE INDEX ON backlot.comment (subject_kind, subject_id, created_at);
```

**Szabályok (adatbázisban kikényszerítve):**
- a feladat eleme, szülője, függőségei ugyanabban a produkcióban vannak;
- az alfeladat szülője nem lehet saját leszármazottja; mélység legfeljebb 3;
- **függőségi kör tilos:** beszúráskor rekurzív lekérdezés ellenőrzi a tranzakción belül, a produkcióra vett
  tanácsadó zárral (`pg_advisory_xact_lock`), hogy két párhuzamos beszúrás se zárhasson kört;
- a feladattípus elemtípusa (ha meg van adva) egyezik az elem típusával.

**Megjegyzések:**
- A megjegyzés szerkesztése és törlése nem írja felül a történetet: a régi szöveg az auditban marad
  (`comment.edited`), törléskor a felület „törölt megjegyzés” jelzést mutat.
- Az **előzmény** (`backlot.task.history`) az auditnapló objektumra szűrt nézete, összefésülve a
  megjegyzésekkel; külön előzménytábla nincs.
- A becslés percben van; a tényleges idő a 16. lépcső (TimeTracker) dolga.

## 5. Feladat-folyamat (alap, a 4. lépcső formátumában)

`waiting` → `ready` → `wip` → `pending_review` → `done`, valamint `retake` (vissza `wip`-be) és `omitted`.
Őrfeltételek: `start` csak ha `predecessors_done` (befejezés → kezdés függőségeknél, a csúszással);
`done` csak ha `all_subtasks_in_state {category: closed}`. A „megjegyzés + státuszváltás egy lépésben”
(Kitsu-minta): a megjegyzés parancsa opcionálisan átmenetkódot kap, és a kettő egy tranzakció.

## 6. Ismétlődő feladatok

- `RRULE` dátumpontossággal (pl. heti produkciós megbeszélés előkészítése); időzóna nem kell.
- **Példányosítás:** indításkor és a feladatlista lekérdezésekor a `materialized_until`-tól a következő 30
  napig; idempotens a `(recurrence_id, occurrence_date)` egyediség miatt. M1-ben nincs háttérszolgáltatás;
  az eseményre induló példányosítás a 12. lépcsőtől.
- A szabály módosítása csak a jövőbeli, még nem létrehozott példányokra hat.

## 7. Parancsok

| Parancs | Mit csinál |
|---|---|
| `backlot.task.create` / `.update` / `.retire` / `.delete` (csak üres, friss) | Feladat |
| `backlot.task.bulk_create` | Egy feladattípus sok elemre (pl. kompozitálás az SQ010 összes snittjére); feladatonként egy auditesemény, egy tranzakció |
| `backlot.task.list` | Szűrés: produkció, elem-részfa (`ltree`), státuszkategória, kiosztott, határidő, lejárt |
| `backlot.task.assign` / `.unassign` | Kiosztás szereppel |
| `backlot.task.dependency.add` / `.remove` | Függőség, körellenőrzéssel |
| `backlot.task.recurrence.create` / `.update` / `.stop` / `.materialize` | Ismétlődés |
| `backlot.task.history` | Előzmény (audit + megjegyzések) |
| `backlot.comment.add` / `.edit` / `.retire` / `.list` | Megjegyzés, opcionális átmenettel |
| `backlot.person.create` / `.update` / `.list` / `.retire` | Személyek (#53) |
| a `task_default` folyamat átmenetei (`backlot.task.start`, `.submit`, …) | A 4. lépcső generált parancsai |

## 8. Hibakódok

`TASK_NOT_FOUND`, `TASK_CROSS_PRODUCTION`, `TASK_TYPE_MISMATCH`, `TASK_SUBTASK_DEPTH`, `TASK_DEPENDENCY_CYCLE`,
`TASK_DATES_INVALID`, `RECURRENCE_RULE_INVALID`, `COMMENT_TOO_LONG`, `PERSON_IN_USE`, valamint a 4. lépcső
`WF_*` kódjai és a `CONFLICT_STALE_VERSION`.

## 9. Méret és tesztek (G5-re előre)

- **L-profil:** ≈ 620 000 elem × 3 feladat ≈ 1,9 millió feladat, 52 produkcióban; „saját feladataim”, egy
  szekvencia feladatai és a lejárt feladatok listája a CFA4-MIN gépen (SC-02).
- Tulajdonságalapú teszt: véletlen függőségsorok után nincs kör, párhuzamos beszúrásnál sem.
- Ismétlődés: RFC 5545 példák (hónap utolsó munkanapja, kéthetente) és idempotens példányosítás.
- Megjegyzés + átmenet: ha az átmenet elutasított, a megjegyzés sem jön létre (egy tranzakció).
- Hibás bemenet: angol hibakód, összeomlás nincs.

## 10. Licenc (G3-ra előre)

- python-dateutil: Apache-2.0 és BSD-3-Clause, GPL-3.0-or-later-rel összeegyeztethető; Ubuntu `python3-dateutil`.
- Stalker (LGPL): függőségtípusok és alfeladat-DAG mint minta, kód nem. Kitsu (AGPL): mezőkészlet és a
  „megjegyzés + státusz” minta, kód nem.

## 11. Döntési kérdés

| # | Kérdés | Javaslat |
|---|---|---|
| 53 | Kihez lehessen feladatot rendelni, amíg nincs Crew (18.)? | Közös `person` tábla már az 5. lépcsőben (név, OS-felhasználó-kötés); a 18. és a 19. lépcső ezt bővíti, nem cseréli |

## 12. G2-eredmény

**Javaslat: PASS a #53 jóváhagyásával.** Csatolmány a #43 szerint később, a File Services FS-1 után.
