# BacklotHub 4. lépcső – Workflow Engine (mag): G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-11 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.12 · **Döntés:** #19 (mag most, eseményvezérelt
rész a 12. lépcsőben) · **Előzmény:** [G1 Workflow Engine](CFA4-BACKLOTHUB-G1-04-WORKFLOW-ENGINE-MAG-2026-11-06.md)
(funkcionális követelmények jóváhagyva 2026-11-10) · **Épül rá:** G2-01, G2-02 (jóváhagyva), G2-03 (jóváhagyva) · **Állapot:** G2 **jóváhagyva** (István, 2026-11-11) · implementáció nincs

## 1. A G1-hez képest egy változás: nincs állapotgép-könyvtár

A G1 jelöltje a python-statemachine (MIT) volt, a választást a G2-re hagyta. **Javaslat: egyik könyvtár sem
kell.** Indok:

- A folyamatdefiníció adat (PostgreSQL, JSON), nem Python-osztály. A python-statemachine és a transitions
  osztályban leírt gépre készült; adatból építve a könyvtár csak egy szótárkeresést végezne el helyettünk.
- A futtatás lényege kevés kód: az aktuális állapotból a kért átmenet megkeresése, az őrfeltételek kiértékelése,
  az állapot írása és az audit, egy tranzakcióban. Saját kódként ez teljesen tesztelhető, és nincs új függőség
  (`--require-hashes`, Ubuntu-csomag kérdése nem merül fel).
- A diagram (a könyvtár egyik előnye) a definícióból Graphviz DOT-ként előállítható, a felület ezt mutatja.

A python-statemachine referencia marad (validálási szabályok, diagram). Ha a jóváhagyás ezt a pontot
elutasítja, a futtató helyére a python-statemachine kerül, a többi rész változatlan.

## 2. Hatókör (#19)

| 4. lépcső (mag) | 12. lépcső (Event Fabric EF-R1 után) |
|---|---|
| Definíció, verzió, példány; átmenet parancsként | Eseményre induló átmenet, automatikus kiosztás |
| Őrfeltételek zárt nyelven | Értesítés átmenetre |
| Jóváhagyási ágak (átmenet szereppel, kötelező megjegyzéssel) | Határidő-figyelmeztetés eseményként |
| Határidő kiértékelése lekérdezéskor | – |
| Elutasítás hibakóddal, auditálva | – |
| **Alapfolyamatok: produkció és szerkezeti elem** | Feladat (5.) és verzió-jóváhagyás (13.) folyamata a saját lépcsőjével jön |

## 3. Integrációs határok

```
 Qt6 / CLI / MCP ──► backlot.<…>.<átmenet> parancs ──► WorkflowRunner ──► SubjectAdapter (produkció, elem, …)
                                                           │   őrfeltétel-kiértékelő (zárt predikátumkészlet)
                                                           └──► cfa4_audit (ugyanabban a tranzakcióban)
```

- **SubjectAdapter:** minden folyamatba vonható objektumtípus egy adaptert ad: státusz olvasása és írása, az
  őrfeltételek kontextusa (pl. gyerekek, linkek, verziók), és a státuszlista. A 2. és a 3. lépcső adaptere a 4.
  lépcsőben készül; az 5., 8., 13. lépcső a sajátját hozza.
- **A státusz egy helyen van:** az objektum saját `status_id` mezője (2. és 3. lépcső) a hiteles állapot. A
  folyamatpéldány csak azt köti, melyik definícióverzió érvényes rá. Így nincs két, eltérhető állapotmező.
- **Event Fabric:** a 4. lépcsőben nincs kapcsolat. A 12. lépcső az átmenet után egy outbox-sort ír ugyanabban
  a tranzakcióban; ehhez a futtatónak egy bővítési pontja van (`after_transition` kampó), most hívó nélkül
  nem készül el.

## 4. Folyamatdefiníció (JSON, sémával validálva)

```json
{
  "schema": "cfa4.backlot.workflow/1",
  "target": {"kind": "entity", "entity_type": "shot"},
  "states": ["waiting", "wip", "pending_review", "approved", "retake", "omitted"],
  "initial": "waiting",
  "terminal": ["approved", "omitted"],
  "transitions": [
    {"code": "start",   "command": "backlot.shot.start",   "from": ["waiting", "retake"], "to": "wip"},
    {"code": "submit",  "command": "backlot.shot.submit",  "from": ["wip"], "to": "pending_review"},
    {"code": "approve", "command": "backlot.shot.approve", "from": ["pending_review"], "to": "approved",
     "roles": ["producer", "owner"]},
    {"code": "request_changes", "command": "backlot.shot.request_changes", "from": ["pending_review"],
     "to": "retake", "roles": ["producer", "owner"], "comment_required": true},
    {"code": "omit",    "command": "backlot.shot.omit", "from": "*", "to": "omitted",
     "guard": {"not": {"has_children_in_state": {"category": "active"}}}}
  ],
  "deadlines": [{"state": "pending_review", "max_days": 3}]
}
```

- Az állapotkódok a cél státuszlistájának kódjai (2. lépcső `status`), így a felület ugyanazt a címkét mutatja.
- **Aktiváláskor ellenőrzött:** minden állapot létezik a státuszlistában; a kezdő állapot benne van; minden
  állapot elérhető a kezdőből; végállapotból nincs kimenő átmenet (a `*` forrás nem érvényes rájuk); a
  parancsazonosítók egyediek a `backlot.` névtérben; az őrfeltételek a nyelvtan szerint helyesek.
- Aktiválás után a definíció **változtathatatlan** (SHA-256 kivonat rögzítve); módosítás új verzió.

## 5. Őrfeltétel-nyelv (zárt, deklaratív)

- Kombinátorok: `all`, `any`, `not`.
- Predikátumok (bővíthető, de csak kódban regisztrálva, paramétersémával):

  | Predikátum | Jelentés |
  |---|---|
  | `has_children_in_state {category}` | Van-e gyerek adott státuszkategóriában (`planned`, `active`, `closed`, …) |
  | `all_children_in_state {category}` | Minden gyerek adott kategóriában |
  | `linked_count {link_type, op, value}` | Linkek száma (pl. legalább egy `casting`) |
  | `field_set {field}` / `field_equals {field, value}` | Az objektum mezője (a sémában engedélyezett mezőkre) |
  | `before_date {field}` | A dátummező még nem múlt el |
  | `has_approved_version` | A 8./13. lépcső hozza, addig nem regisztrált (aktiváláskor ismeretlen predikátum → hiba) |

- Nincs ciklus, nincs tetszőleges kód, nincs külső hívás. A kiértékelés a tranzakción belül, olvasással.
  Predikátum belső hibája: `WF_GUARD_ERROR`, az átmenet elutasítva, összeomlás nincs.

## 6. Adatmodell (DDL-vázlat, `backlot` séma)

```sql
CREATE TABLE backlot.workflow_definition (
    id            uuid PRIMARY KEY,
    code          text NOT NULL,                  -- pl. shot_default
    version       integer NOT NULL,
    production_id uuid REFERENCES backlot.production(id),  -- NULL: közös
    target_kind   text NOT NULL CHECK (target_kind IN ('production','entity','task','version','document','delivery','archive')),
    target_type_id uuid REFERENCES backlot.entity_type(id), -- entity esetén a típus
    state         text NOT NULL CHECK (state IN ('draft','active','retired')),
    definition    jsonb NOT NULL,
    digest        bytea,                          -- aktiváláskor
    activated_at  timestamptz,
    ...közös oszlopok production_id nélkül,
    UNIQUE (code, version)
);
-- Célonként egy aktív: produkciós felülírás > közös
CREATE UNIQUE INDEX ON backlot.workflow_definition
    (target_kind, target_type_id, production_id) NULLS NOT DISTINCT WHERE state = 'active';

CREATE TABLE backlot.workflow_instance (
    subject_kind  text NOT NULL,
    subject_id    uuid NOT NULL,
    production_id uuid NOT NULL REFERENCES backlot.production(id),
    definition_id uuid NOT NULL REFERENCES backlot.workflow_definition(id),
    state_entered_at timestamptz NOT NULL,        -- határidőhöz
    row_version   integer NOT NULL,
    PRIMARY KEY (subject_kind, subject_id)
);

CREATE TABLE backlot.workflow_state_period (   -- riporthoz (6. Dashboard): mennyi ideig volt egy állapotban
    subject_kind text NOT NULL, subject_id uuid NOT NULL,
    status_code text NOT NULL, entered_at timestamptz NOT NULL, left_at timestamptz,
    PRIMARY KEY (subject_kind, subject_id, entered_at)
);
```

A hiteles történet az auditnapló; a `workflow_state_period` csak gyors riportra van, és ugyanabban a
tranzakcióban íródik. **Határidő:** a `deadlines` szabály és a `state_entered_at` alapján lekérdezéskor
számolt `overdue` jelzés (nézet), tárolt mező nincs.

## 7. Futás egy átmenetnél (egy tranzakció)

1. Példány és objektum zárolása (`FOR UPDATE`), `row_version` ellenőrzés.
2. Az átmenet megkeresése a kötött definícióverzióban: van-e ilyen kód, és az aktuális állapot a forrásai közt
   van-e (`WF_TRANSITION_NOT_ALLOWED`).
3. Szerep (`roles`) a `production_member` alapján (#42: M1-ben a tulajdonos mindenre jogosult)
   (`PERMISSION_DENIED`); kötelező megjegyzés (`WF_COMMENT_REQUIRED`).
4. Őrfeltételek (`WF_GUARD_FAILED` a nem teljesült predikátum nevével).
5. Írás: objektum `status_id`, példány `state_entered_at`, állapotszakasz lezárása és nyitása.
6. Audit: `workflow.transition` (`change` = régi/új állapot, átmenetkód, megjegyzés, definíció kódja és
   verziója). **Elutasításkor** is auditesemény, `outcome = rejected`, `reason_code`-dal, de üzleti
   változás nélkül; ehhez az elutasító ág visszagörgeti az üzleti részt, és csak az auditot véglegesíti.

## 8. Parancsok

| Parancs | Mit csinál |
|---|---|
| A definíciók átmenetei (pl. `backlot.shot.submit`) | Az aktív definíciókból generált parancsleírók; `enabled()` = az átmenet most lehetséges-e; mind a futtatóra mutat |
| `backlot.wf.available` | Egy objektumra a lehetséges átmenetek, és ami nem lehetséges, annak oka (a felület gombjaihoz) |
| `backlot.wf.definition.create` / `.update` / `.validate` / `.activate` / `.retire` / `.diagram` | Definíciókezelés; `diagram` DOT-ot ad |
| `backlot.wf.instance.migrate` | Példányok átkötése új verzióra, kifejezett állapotmegfeleltetéssel; auditált |
| `backlot.wf.overdue` | Határidőn túli objektumok listája |

A generált parancsok az aktív definíció változásakor frissülnek a regiszterben (`commands.changed`
értesítés a vezérlőcsatornán).

## 9. Átállás a 2. és 3. lépcső egyszerű státuszváltásáról

A 4. lépcső migrációja:
- létrehozza a `production_default` és az alap szerkezeti elem definíciókat (sorozat, szekvencia, jelenet,
  snitt), aktívként;
- minden meglévő produkcióhoz és elemhez példányt köt az aktuális státusszal (`state_entered_at` = utolsó
  státuszváltás ideje az auditból, ennek hiányában most);
- a `backlot.production.status.set` és az elem-státusz közvetlen beállítása megszűnik; helyette a definíció
  átmenetei. Ha egy meglévő státusz nem érhető el a definícióban, a migráció megáll és listázza (nem
  módosít csendben).

## 10. Hibakódok

`WF_TRANSITION_NOT_ALLOWED`, `WF_GUARD_FAILED`, `WF_GUARD_ERROR`, `WF_COMMENT_REQUIRED`,
`WF_DEFINITION_INVALID`, `WF_DEFINITION_IMMUTABLE`, `WF_NO_ACTIVE_DEFINITION`, `WF_MIGRATION_UNMAPPED_STATE`,
`PERMISSION_DENIED`, `CONFLICT_STALE_VERSION`.

## 11. Tesztek (G5-re előre)

- Definíció-validálás: minden szabály megsértésére külön teszt.
- Tulajdonságalapú teszt: véletlen definíciók és átmenetsorok mellett a státusz mindig a definíció állapota,
  végállapotból nincs kilépés, elutasítás után az üzleti adat változatlan, de van auditesemény.
- Párhuzamos átmenet ugyanarra az objektumra: pontosan egy sikerül.
- Verzióváltás: régi példány a régi definíció szerint fut tovább; migráció után az új szerint.
- Őrfeltétel-fuzz: hibás vagy mély kifejezés elutasítva, összeomlás nincs.

## 12. Licenc (G3-ra előre)

Új függőség nincs. Referencia: python-statemachine (MIT), transitions (MIT), Stalker (LGPL, státuszlista),
Kitsu (AGPL, státuszattribútumok, kód nem). SpiffWorkflow (BPMN) elvetve, ahogy a G1-ben.

## 13. G2-eredmény

**Javaslat: PASS.** Új döntési kérdés nincs; a G1 nyitott pontját (futtatókönyvtár) a G2 saját futtatóval
zárja (1. pont). Előfeltétel a megvalósításhoz: a 2. és a 3. lépcső.
