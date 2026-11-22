# BacklotHub 12. lépcső – Notification Center: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-14 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.23 · **Előzmény:**
[G1 Notification Center](CFA4-BACKLOTHUB-G1-12-NOTIFICATION-CENTER-2026-11-06.md) (funkcionális követelmények
jóváhagyva 2026-11-10) · **Épül rá:** CFA4-EVENT-FABRIC-001 1.0 és CFA4-illesztése (Q1–Q12 eldöntve), G2-01
(Audit), G2-04 (Workflow Engine, `after_transition` kampó), G2-05 (`person`, feladatszerepek), G2-10
(`publish.committed`) · **Állapot:** G2 **jóváhagyva** (István, 2026-11-21) · implementáció nincs, és az EF-R1 előtt nem
is indulhat

Egy új döntési kérdés van (#58: a `backlot` domain az Event Fabric subject-katalógusában). Ez az Event Fabric
projektjét is érinti, ezért a koordinátornak is jelzem. Új függőség nincs: az Event Fabric Python SDK-ja hozza a
NATS-klienst, és az ott dől el.

## 1. Hatókör ebben a lépcsőben

- **Eseménykibocsátás a BacklotHubból** tranzakciós kimenőtárral (outbox), mert az EF-illesztés Q9-e kötelezővé
  teszi a kritikus eseményeknél.
- **Notification Center:** szabályok, feliratkozás, beérkező lista, asztali értesítés, beállítások,
  rendszerüzenetek.
- **A Workflow Engine eseményvezérelt része (#19):** határidő-figyelmeztetés eseményként és eseményre induló
  szabályok (automatikus kiosztás, átmenet).
- **Nem része:** e-mail és más külső csatorna (SEC-02; M2-ben, kifejezett beállítással), értesítés a DCC-be
  (a 11a. adapter lekérdezéssel jelez, a push később), mobil.

## 2. Integrációs határok

```
 BacklotHub-parancs ──► egy tranzakció: üzleti sor + audit + outbox-sor
                                                   │
                                     outbox-továbbító (BacklotHub-szolgáltatás)
                                                   │  Event Fabric Python SDK
                                                   ▼
                                       JetStream (cfa4.backlot.>)
                                     ┌─────────────┴──────────────┐
                         Notification-fogyasztó          Workflow-trigger-fogyasztó
                         (backlot-notify)                (backlot-wf-trigger)
                                     │                            │
                    notification sorok, asztali értesítés   átmenet / kiosztás a
                    (D-Bus), beérkező lista                 parancsregiszteren át
```

| Határ | Irány | Szerződés |
|---|---|---|
| Event Fabric (EF-R1) | ↔ | Kanonikus boríték (EF 5.1), JetStream tartós stream, ACK a feldolgozás véglegesítése után (8.2), idempotencia `consumer_id + event_id` (9.1), replay külön fogyasztó-identitással (Q6) |
| Audit (1.) | → | Ugyanaz a változás egy auditbejegyzés és egy esemény; az esemény `event_type`-ja `backlot.` + az auditművelet neve (3.2) |
| Workflow Engine (4.) | ← → | `after_transition` kampó outbox-sort ír; a triggerfogyasztó a meglévő átmenetparancsokat hívja (6. pont) |
| Task (5.), Review (13.), Publisher (10.) | → | Eseményforrások: kiosztás, státuszváltás, megjegyzés, határidő, publikálás, jóváhagyás |
| Felület (`cfa4-backlothub`, #54) | ← | Beérkező lista, olvasatlan-számláló, beállítások |
| Asztal | → | `org.freedesktop.Notifications` a munkamenet D-Buszán |

## 3. Eseménykibocsátás

### 3.1 Outbox

```sql
CREATE TABLE backlot.event_outbox (
    seq           bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    event_id      uuid NOT NULL UNIQUE,             -- UUIDv7, az outbox-íráskor; újrapróbáláskor nem változik (EF 5.3/1)
    event_type    text NOT NULL,                    -- pl. backlot.task.status_changed
    event_version integer NOT NULL,
    occurred_at   timestamptz NOT NULL,             -- a tranzakció ideje; újrakézbesítéskor nem íródik felül (5.3/2)
    correlation_id uuid NOT NULL,                   -- a kiváltó parancs futásáé
    causation_id  uuid,                             -- ha eseményre induló szabály okozta (6. pont)
    production_id uuid,
    subject_type  text, subject_id uuid,            -- az entitás, a sorrendhez (Q10)
    entity_seq    bigint,                           -- entitásonkénti sorszám (Q10)
    classification text NOT NULL,                   -- EF 5.1, nem hagyható el
    payload       jsonb NOT NULL,                   -- titok nélkül (5.3/6)
    audit_seq     bigint NOT NULL,                  -- a kapcsolódó auditbejegyzés
    published_at  timestamptz,                      -- NULL = még nincs kint
    attempts      integer NOT NULL DEFAULT 0,
    last_error    text
);
CREATE INDEX ON backlot.event_outbox (seq) WHERE published_at IS NULL;
```

- Az outbox-sort ugyanaz a tranzakció írja, amelyik az üzleti sort és az auditot (G2-01: az audit `record()`
  az utolsó lépés a COMMIT előtt; az outbox-sor előtte íródik, hogy az audit láncában benne legyen a hivatkozott
  `event_id`).
- **Továbbító:** a BacklotHub-szolgáltatás egy szála `seq` sorrendben olvassa a ki nem küldött sorokat
  (`FOR UPDATE SKIP LOCKED`), és az SDK-val JetStreamre küldi; a JetStream deduplikációs azonosítója az
  `event_id`. Sikeres közzététel után `published_at`. Hibánál visszalépéses újrapróbálás; a sor megmarad.
- **Összeomlási ablak** (Q9): ha a közzététel és a `published_at` írása között áll le, újraindításkor
  újraküldi; a JetStream az `event_id` alapján kiszűri, a fogyasztók idempotensek. Esemény nem vész el, és nem
  keletkezik két különböző azonosítójú példány.
- **Ha az Event Fabric nem fut:** a BacklotHub minden parancsa működik, az outbox gyűlik. A szolgáltatás
  helyi rendszerüzenetet ad („az értesítések késnek”, 5.5), a sorok a helyreállás után sorrendben mennek ki
  (EF 14.4).
- **Takarítás:** a kiküldött sorok 30 nap után törölhetők (a JetStream és az audit őrzi őket); beállítható.

### 3.2 Eseménytípusok és subjectek (#58)

- **`event_type` = `backlot.<komponens>.<esemény>`**, verzió nélkül; a verzió az `event_version`, a `.vN` csak a
  `schema_id`-ban (Q8). **Subject:** `cfa4.` + `event_type` (EF 7.1: `cfa4.<domain>.<component>.<event>`).
- Az eseményt kiváltó auditművelet neve ugyanez `backlot.` nélkül (`task.status_changed`). Ezért az eseményt
  kibocsátó auditműveletek pontosan két tagúak; ez a G2-01 mintájának (`^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$`)
  szűkítése, nem módosítása.
- **Javaslat (#58): új `backlot` domain** az EF 7.2 domainlistájában. Indok: a BacklotHub eseményei
  (produkció, elem, feladat, verzió, publikálás) nem férnek a `workflow` domainbe, és a jogosultság (EF 7.3,
  12.2) így egy prefixszel adható: a BacklotHub csak a `cfa4.backlot.>` alá ír, a fogyasztói csak onnan
  olvasnak. A domain felvétele az Event Fabric eseménykatalógusának változása, ezért a koordinátornak jelzem.
- **Első katalógus** (mind JSON Schema, `classification: internal`):

| `event_type` | Forrás | Kritikus (JetStream) |
|---|---|---|
| `backlot.task.assigned`, `backlot.task.unassigned` | 5. | igen |
| `backlot.task.status_changed` | 4./5. (`after_transition`) | igen |
| `backlot.comment.created` | 5. | igen |
| `backlot.deadline.approaching`, `backlot.deadline.missed` | 6.1 | igen |
| `backlot.version.created`, `backlot.publish.committed` | 9./10. | igen |
| `backlot.version.approved`, `backlot.version.rejected` | 13. (a G2-13-ban) | igen |
| `backlot.load.updated_available` | – | **nem**: lekérdezéskor számolt (G2-11), nem esemény |

- A payload csak azonosítókat és rövid, megjelenítéshez kellő adatot visz (címke, régi és új státusz,
  határidő), nem teljes rekordot; a fogyasztó a részleteket az adatbázisból olvassa.

## 4. Notification Center

### 4.1 Adatmodell

```sql
CREATE TABLE backlot.notification_rule (            -- csomagban szállított alapszabályok + produkciós felülírás
    id uuid PRIMARY KEY, code text NOT NULL, version integer NOT NULL,
    event_type text NOT NULL,
    recipients text[] NOT NULL,                       -- zárt lista: assignee, contributor, reviewer, watcher, production_owner
    exclude_actor boolean NOT NULL DEFAULT true,      -- aki a változást csinálta, ne kapja
    state text NOT NULL CHECK (state IN ('draft','active','retired')),
    production_id uuid REFERENCES backlot.production(id),
    ...közös oszlopok production_id nélkül,
    UNIQUE (code, version)
);

CREATE TABLE backlot.notification_subscription (    -- „figyelem” egy objektumot
    person_id uuid NOT NULL REFERENCES backlot.person(id),
    subject_type text NOT NULL, subject_id uuid NOT NULL,
    production_id uuid NOT NULL REFERENCES backlot.production(id),
    created_at timestamptz NOT NULL,
    PRIMARY KEY (person_id, subject_type, subject_id)
);

CREATE TABLE backlot.notification (
    id               uuid PRIMARY KEY,                -- UUIDv7
    person_id        uuid NOT NULL REFERENCES backlot.person(id),
    event_id         uuid,                            -- NULL: helyi rendszerüzenet (4.4)
    kind             text NOT NULL,                   -- event_type vagy system.<kód>
    production_id    uuid REFERENCES backlot.production(id),
    subject_type     text, subject_id uuid,
    summary          jsonb NOT NULL,                  -- üzenetkulcs + paraméterek; a szöveget a felület fordítja
    severity         text NOT NULL CHECK (severity IN ('info','warning','error')),
    occurred_at      timestamptz NOT NULL,            -- az eseményé
    created_at       timestamptz NOT NULL,
    read_at          timestamptz,
    desktop_state    text NOT NULL CHECK (desktop_state IN ('none','pending','sent','suppressed')),
    UNIQUE (person_id, event_id)                      -- replay és ismételt kézbesítés nem hoz második értesítést
);
CREATE INDEX ON backlot.notification (person_id, created_at DESC) WHERE read_at IS NULL;

CREATE TABLE backlot.notification_setting (
    person_id uuid NOT NULL REFERENCES backlot.person(id),
    kind_pattern text NOT NULL,                       -- pl. backlot.task.* vagy system.*
    inbox boolean NOT NULL, desktop boolean NOT NULL,
    PRIMARY KEY (person_id, kind_pattern)
);
CREATE TABLE backlot.notification_preference (       -- személyenként egy sor
    person_id uuid PRIMARY KEY REFERENCES backlot.person(id),
    quiet_hours jsonb,                                -- pl. {"from": "22:00", "to": "07:00"}, sémával
    digest_time time,                                 -- NULL = nincs napi összesítő
    row_version integer NOT NULL
);

CREATE TABLE backlot.event_consumed (                 -- fogyasztónkénti idempotencia (EF 9.1)
    consumer_id text NOT NULL, event_id uuid NOT NULL, processed_at timestamptz NOT NULL,
    PRIMARY KEY (consumer_id, event_id)
);
```

- A szöveg nem tárolódik kész mondatként: `summary` = üzenetkulcs és paraméterek, a felület fordítja (magyar
  vagy angol, #16), ugyanúgy, mint a hibakódokat.
- Sorrend (Q10): az értesítés történeti tény, nem állapot, ezért elavult esemény nem írja felül; a lista az
  `occurred_at` szerint rendez. A trigger-fogyasztó (6.) az `entity_seq`-et figyeli.

### 4.2 Feldolgozás

1. A `backlot-notify` tartós fogyasztó megkapja az eseményt.
2. Egy tranzakció: ha a `(backlot-notify, event_id)` már az `event_consumed`-ben van, ACK és kész. Különben a
   szabályokból címzettek → `notification` sorok (`ON CONFLICT DO NOTHING` a `(person_id, event_id)`-re) →
   `event_consumed` sor → COMMIT.
3. ACK csak a COMMIT után (EF 8.2). Ha a COMMIT előtt áll le, a JetStream újrakézbesíti, és a 2. lépés
   ugyanazt adja.
4. Az **asztali értesítés a COMMIT után**, csak az ebben a tranzakcióban **újonnan létrejött** sorokra
   (`desktop_state = 'pending'` → `sent`). Ha a küldés előtt áll le, induláskor a 24 óránál nem régebbi
   `pending` sorokat elküldi, a régebbieket `suppressed`-re állítja.
5. **Replay** (EF 10.3, Q6) külön fogyasztó-identitással (`backlot-notify-replay`) fut: létrehozhat hiányzó
   beérkező sort, de `desktop_state = 'suppressed'`-del, asztali értesítést soha nem küld.

Így ugyanarra az eseményre egy címzett legfeljebb egy beérkező sort és legfeljebb egy asztali értesítést kap.

### 4.3 Asztali értesítés

- `org.freedesktop.Notifications.Notify` a munkamenet D-Buszán, a PySide6 `QtDBus` moduljával (meglévő
  függőség). A szolgáltatás felhasználói folyamat, ezért eléri a munkamenet buszát; ha nincs busz (pl. SSH),
  az asztali csatorna csendben kimarad, a beérkező lista működik.
- Művelet a buborékon: „Megnyitás” → a `cfa4-backlothub` az objektumra ugrik (`ActionInvoked` jel).
- **Csendes órák:** ilyenkor `suppressed`, a beérkező lista megkapja. **Napi összesítő:** bekapcsolva az
  eseményenkénti asztali értesítés helyett naponta egy, a megadott időben, az olvasatlanok számával.
- Hibás D-Bus-válasz vagy időtúllépés (2 s) nem akasztja meg a fogyasztót: napló, `suppressed`.

### 4.4 Rendszerüzenetek

- Amit **a BacklotHub maga észlel**: sérült auditlánc (`audit.verify`), nem elérhető tárolási gyökér
  (`FILES_ROOT_UNAVAILABLE`), sikertelen mentés, outbox-torlódás (3.1), Event Fabric nem érhető el. Ezek
  **nem** az Event Fabricon jönnek, hanem közvetlenül íródnak (`event_id` NULL, `kind = system.<kód>`), mert
  épp az EF kiesését is jelezniük kell. Ismétlés ellen: ugyanaz a kód ugyanarra az objektumra csak akkor új
  sor, ha az előző már olvasott vagy megszűnt.
- Más CFA4-komponensek rendszereseménye (pl. „proxy kész” a Media Servicestől) akkor jelenik meg, amikor az a
  komponens az Event Fabricra publikál; addig nincs, és nem is készül rá helyettesítő.
- A rendszerüzenetek a beérkező listából nem kapcsolhatók ki, csak az asztali csatornájuk.

## 5. Felület és parancsok

| Parancs | Mit csinál |
|---|---|
| `backlot.notify.list` | Beérkező lista: olvasatlan/összes, produkció, típus, idő szerint; lapozva |
| `backlot.notify.mark_read` / `.mark_unread` / `.mark_all_read` | Olvasottság |
| `backlot.notify.unread_count` | A felület számlálójához |
| `backlot.notify.subscribe` / `.unsubscribe` / `.subscriptions` | Objektum figyelése |
| `backlot.notify.settings.get` / `.set` | Csatornák típusonként, csendes órák, napi összesítő |
| `backlot.notification_rule.*` | Szabályok (aktiválás után változtathatatlan, új verzió; mint a G2-04 definíciói) |
| `backlot.events.outbox_status` | Kiküldetlen sorok száma, legrégebbi kora, utolsó hiba (diagnosztika) |

A felület (`cfa4-backlothub`) a számlálót az audit `chain_head` sorszáma szerint frissíti, ugyanúgy, mint a
dashboard (G2-06); élő push a felületre nem kell M1-ben.

## 6. A Workflow Engine eseményvezérelt része (#19)

### 6.1 Határidők

- A G2-04 a határidőt lekérdezéskor értékeli. Itt egy **időzítő** jön a BacklotHub-szolgáltatásban: percenként
  a `backlot.task_overdue` nézet (G2-06) és a folyamatdefiníciók `deadlines` szabályai alapján.
- `backlot.deadline.approaching` (beállítható előidő, alapból 24 óra) és `backlot.deadline.missed` egyszer
  objektumonként és határidőnként:

```sql
CREATE TABLE backlot.deadline_signal (
    subject_type text NOT NULL, subject_id uuid NOT NULL,
    deadline_key text NOT NULL,                 -- pl. due_date:2026-11-20 vagy state:pending_review:<state_entered_at>
    kind text NOT NULL CHECK (kind IN ('approaching','missed')),
    event_id uuid NOT NULL,
    PRIMARY KEY (subject_type, subject_id, deadline_key, kind)
);
```

- A jelzés és az outbox-sor egy tranzakcióban; ha a határidő módosul, új `deadline_key`, új jelzés. Leállás
  után az időzítő pótolja a kimaradt jelzéseket, egyszer.

### 6.2 Eseményre induló szabályok

- A folyamatdefiníció (G2-04, JSON-séma) új, opcionális része:

```json
"triggers": [
  {"on": "backlot.task.status_changed", "when": {"to": "approved", "task_type": "compositing"},
   "do": {"action": "transition", "target": "next_task", "transition": "start"}},
  {"on": "backlot.publish.committed", "when": {"product_type": "plate"},
   "do": {"action": "assign", "role": "assignee", "from_role_of": "parent_task"}}
]
```

- `when`: a G2-04 zárt őrfeltétel-nyelve; `do`: zárt műveletlista (`transition`, `assign`, `notify`). Kód nem
  fut.
- A `backlot-wf-trigger` tartós fogyasztó a műveletet **a parancsregiszteren át** futtatja, a rendszer
  szereplőjeként (`actor_kind = 'system'`, `via = 'trigger'`, a szabály azonosítójával), ugyanazzal az
  őrfeltétel- és auditútvonallal, mint a felhasználó.
- **Idempotencia:** `event_consumed` a `(backlot-wf-trigger, event_id)` párra, a művelettel egy tranzakcióban.
- **Hurokvédelem:** a kibocsátott esemény `causation_id`-ja a kiváltó esemény; legfeljebb 8 mélységű lánc,
  utána `WF_TRIGGER_DEPTH_EXCEEDED`, a művelet nem fut, rendszerüzenet keletkezik. Egy szabály a saját maga
  okozta eseményre nem indul újra.
- **Sorrend:** ha az esemény `entity_seq`-je kisebb, mint amit a fogyasztó az entitásra már feldolgozott,
  a szabály nem fut (elavult, Q10).
- Ha az őrfeltétel a futás idején már nem teljesül (közben más módosított), a művelet nem fut, a napló és a
  trigger-előzmény ezt mutatja; hiba nincs.
- **Replay** a trigger-fogyasztót nem indítja (Q6, EF 10.3): az átmenet nem ismétlődhet.

## 7. Never crash és hibakódok

- Event Fabric kiesése: a BacklotHub működik, az outbox gyűlik, rendszerüzenet; a fogyasztók újracsatlakoznak.
- D-Bus hiánya vagy hibája: csak az asztali csatorna marad ki.
- Hibás vagy ismeretlen sémájú esemény: a fogyasztó nem omlik össze; NACK a megadott próbálkozásszámig, utána
  az EF DLQ-ja (8.4), rendszerüzenettel.

Hibakódok: `NOTIFY_RULE_INVALID`, `NOTIFY_SETTING_INVALID`, `NOTIFY_SUBJECT_NOT_FOUND`, `EVENT_PUBLISH_DEFERRED`
(az outbox kiküldése késik; tájékoztató), `EVENT_SCHEMA_UNKNOWN`, `WF_TRIGGER_INVALID`,
`WF_TRIGGER_DEPTH_EXCEEDED`.

## 8. Sorrend és függés

| Rész | Kell hozzá |
|---|---|
| Az egész lépcső | **EF-R1** (F1–F5 + Rust és Python SDK, Q2); a #58 a katalógusban |
| Outbox és továbbító | 1., 4., 5. lépcső megvalósítása |
| `version.*`, `publish.*` események | 9., 10. lépcső |
| `version.approved/rejected` | 13. lépcső |
| Értesítés a DCC-be | 11a. adapter és egy később eldöntendő push-csatorna |

## 9. Tesztek (G5-re előre)

- **Összeomlási ablak** (Q9): leállás az outbox-írás után, a közzététel után a `published_at` előtt, a fogyasztó
  COMMIT-ja előtt és után; eredmény: minden esemény pontosan egyszer van kint, minden címzett pontosan egy
  beérkező sort és legfeljebb egy asztali értesítést kap.
- **Replay:** a teljes stream visszajátszása után nincs új asztali értesítés, nincs új átmenet.
- **Átrendezett kézbesítés** (Q10): a trigger nem fut elavult eseményre.
- **Hurok:** egymást kiváltó szabályok → megáll a 8. mélységnél, rendszerüzenettel.
- **Határidő:** óraugrás (rendszeridő előre és vissza), leállás a határidő átlépésekor; jelzés egyszer.
- **Event Fabric nélkül:** BacklotHub-parancsok futnak, outbox nő, helyreálláskor sorrendben kiürül.
- **Mutációs fuzz** a fogyasztóra (Q7): csonkolt, hibás sémájú, ismeretlen típusú boríték.
- **D-Bus:** hamis értesítési szolgáltatás a munkamenet buszán (`dbus-run-session`); nincs busz → kimarad.
- **Mérés** a CFA4-MIN gépen: 10 000 esemény feldolgozási ideje és a beérkező lista lekérdezése 100 000 sornál.

## 10. Licenc (G3-ra előre)

Új függőség nincs: a NATS-kliens az Event Fabric Python SDK-jával jön (annak G3-ja), a D-Bus a PySide6
`QtDBus`-a. Kitsu/Zou (AGPL-3.0): értesítés- és feliratkozás-modell mint referencia, kód nem. Apprise: M1-ben
nem.

## 11. Nyitott kérdés

| # | Kérdés | Javaslat |
|---|---|---|
| 58 | Kapjon-e a BacklotHub saját `backlot` domaint az Event Fabric subject-katalógusában? | Igen: `cfa4.backlot.<komponens>.<esemény>`, a jogosultság egy prefixszel adható; a felvétel az Event Fabric projektjének katalógusváltozása |

## 12. G2-eredmény

**Javaslat: PASS**, a #58 eldöntésével. A megvalósítás az EF-R1 után indulhat.

## 13. A későbbi lépcsők G2-jeinek kiegészítései (2026-11-21)

Ezeket a 12. lépcső megvalósításakor kell átvezetni; mind új tábla nélkül (kivéve a jelzett `capacity_signal`), és
egyik sem változtat a fenti szerkezeten. A jóváhagyásuk a hivatkozott G2-ékkel együtt történik.

| Forrás | Kiegészítés |
|---|---|
| [G2-13 Review](CFA4-BACKLOTHUB-G2-13-REVIEW-APPROVAL-2026-11-21.md) | A `backlot.version.approved/rejected` mellé `backlot.version.changes_requested` esemény |
| [G2-14 Document](CFA4-BACKLOTHUB-G2-14-DOCUMENT-MANAGEMENT-2026-11-21.md) | Események: `backlot.document.version_added`, `.version_approved`, `.version_changes_requested`, `.version_rejected`, `.signature_recorded`; a 6.1 időzítő a `document.valid_until` mezőt is kiértékeli (`deadline_signal`, `subject_type = 'document'`) |
| [G2-15 Resource Planning](CFA4-BACKLOTHUB-G2-15-RESOURCE-PLANNING-2026-11-21.md) | `backlot.capacity.overload_detected` esemény és a `capacity_signal` tábla (a `deadline_signal` mintájára); új címzett: `resource_person` |
| [G2-16 TimeTracker](CFA4-BACKLOTHUB-G2-16-TIMETRACKER-2026-11-21.md) | Események: `backlot.timesheet.submitted`, `.approved`, `.returned`; új címzett: `production_approver` (a produkció `producer`/`owner` tagjai) |
| [G2-21 Equipment](CFA4-BACKLOTHUB-G2-21-EQUIPMENT-MANAGEMENT-2026-11-21.md) | Események: `backlot.equipment.checked_out`, `.checked_in`, `.damaged`, `.maintenance_started`, `.maintenance_completed`, `.booking_confirmed`, `.booking_cancelled`; a 6.1 időzítő `deadline_signal` fajtái: `maintenance_due` (`subject_type = 'equipment'`), `checkout_overdue` (`'equipment_checkout'`); új címzett nincs |
| [G2-23 Shooting Schedule](CFA4-BACKLOTHUB-G2-23-SHOOTING-SCHEDULE-2026-11-21.md) | Események: `backlot.schedule.revision_published` (hasznos teher: szám, szín, összevetés-összegzés, személyes adat nélkül), `.revision_discarded`, `backlot.schedule.callsheet_issued`; új címzett nincs |
| [G2-24 Budget Tracking](CFA4-BACKLOTHUB-G2-24-BUDGET-TRACKING-2026-11-21.md) | Események: `backlot.budget.approved`, `backlot.budget.threshold_crossed` (összeg nélkül), `backlot.cost.imported`; új címzett nincs (`production_owner`, `production_approver`) |
| [G2-25 Delivery](CFA4-BACKLOTHUB-G2-25-DELIVERY-MANAGEMENT-2026-11-21.md) | Események: `backlot.delivery.package_ready`, `.package_delivered`, `.package_accepted`, `.package_returned`, `.check_failed`, `.deficiency_opened`; a 6.1 időzítő `deadline_signal` fajtája: `delivery_due` (`subject_type = 'delivery_package'`, `'delivery_item'`); új címzett nincs |
| [G2-26 Archive](CFA4-BACKLOTHUB-G2-26-ARCHIVE-2026-11-21.md) | Események: `backlot.archive.package_archived`, `.verification_failed`, `.repaired`, `.reopened`, `.destroyed`; időzítők (`deadline_signal`): `archive_verify_due`, `archive_retention_due`; új címzett nincs |
| [G2-17 Production Calendar](CFA4-BACKLOTHUB-G2-17-PRODUCTION-CALENDAR-2026-11-21.md) | Események: `backlot.calendar.event_invited`, `.event_changed`, `.event_cancelled`; új címzett: `event_participant` |

A `notification_rule.recipients` zárt listája így: `assignee`, `contributor`, `reviewer`, `watcher`, `production_owner`,
`resource_person`, `production_approver`, `event_participant`.
