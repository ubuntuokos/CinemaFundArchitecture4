# CFA4-AUDIT-001 – Közös CFA4-auditnapló (1.0)

**Dátum:** 2026-11-08 · **Állapot:** a tulajdonos (`ubuntuokos`) jóváhagyta 2026-11-08-án (BacklotHub #40: a
napló közös CFA4-szolgáltatás; #41: M1-ben a szereplő a helyi OS-felhasználó). Döntés: [ADR 0008](../adr/ADR-0008-cfa4-audit.md).
**Előzmény:** a BacklotHub 1. lépcső (Audit Trail) G1-forrásvizsgálata (projekt-munkaanyag)
**Kiinduló terv:** FA3 Unified Journal (`FA3-JOURNAL-ARCHIVE-001`, FA3 `a9c724da`), CFA4-hez igazítva (lásd 11. pont)

## 1. Cél és hatókör

A CFA4 egyetlen, közös, csak hozzáfűzhető auditnaplója. Minden CFA4-alkalmazás és -szolgáltatás ide írja a
visszakövetendő műveleteit. Az első fogyasztók:

- a BacklotHub (1. lépcső: Audit Trail, a felülettel együtt);
- az Event Fabric (terv 12.4: közzététel, elutasítás, jogosultságmódosítás, DLQ, replay).

A naplóban nem cserélhető le, nem írható át és normál művelettel nem törölhető egyetlen esemény sem. A
manipuláció kimutatható.

**Nem része:**
- a diagnosztikai napló (a motor `EventLog`-ja, a strukturált alkalmazásnaplók);
- a recovery journal;
- a metrikák és a tracing.

Az auditnapló azt rögzíti, **ki mit változtatott**. Azt, hogy mi történt a programban, a diagnosztikai napló
rögzíti.

## 2. Hatásköri határok

Az auditnapló nem:
- jogosultsági hatóság (nem dönt, csak rögzíti a döntést);
- munkafolyamat-motor;
- eseménybusz (nem kézbesít, nem értesít);
- biztonsági mentés (a mentés része, de nem helyettesíti).

## 3. Eseménymodell

| Mező | Típus | Kötelező | Leírás |
|---|---|---|---|
| `seq` | bigint | igen | Naplón belüli folytonos sorszám (a lánc sorrendje) |
| `event_id` | UUIDv7 | igen | Globálisan egyedi azonosító |
| `occurred_at` | timestamptz | igen | A művelet ideje (UTC) |
| `recorded_at` | timestamptz | igen | A rögzítés ideje |
| `app` | text | igen | Kibocsátó alkalmazás (`backlothub`, `event-fabric`, …) |
| `action` | text | igen | Kanonikus művelet, pl. `task.status.changed`, `version.approved`, `permission.granted`, `publish.committed` |
| `command_id` | text | nem | A kiváltó CFA4-parancs azonosítója (a „minden egy parancs” elv szerint) |
| `actor_kind` | enum | igen | `os_user` (M1), `identity` (M2-től), `system`, `agent` |
| `actor_ref` | text | igen | Az `actor_kind` szerinti azonosító (M1: `uid:1000/istvan`) |
| `on_behalf_of` | text | nem | Ha ügynök vagy automatizmus egy felhasználó nevében jár el |
| `subject_type` / `subject_id` | text / UUID | igen | Az érintett objektum (pl. `task`, `version`) |
| `scope` | jsonb | igen | Kontextus: produkció, szekvencia, snitt stb. azonosítói (szűréshez) |
| `change` | jsonb | nem | Régi és új érték mezőnként; személyes adat nem kerülhet bele (lásd 7.) |
| `outcome` | enum | igen | `success`, `rejected`, `failed` |
| `reason_code` | text | nem | Angol hibakód elutasításkor (#16) |
| `correlation_id` / `causation_id` | UUID | nem | Kapcsolat más eseményekkel és az Event Fabric eseményeivel |
| `trace_id` | text | nem | W3C trace-id (#35) |
| `prev_hash` | bytea | igen | Az előző esemény `hash`-e (az első eseménynél rögzített kezdőérték) |
| `hash` | bytea | igen | SHA-256 a kanonikus szerializált eseményre + `prev_hash`-re |

A kanonikus szerializálás rögzített: kulcsok rendezve, UTF-8, időpontok RFC 3339 UTC-ben, számok szöveg
nélkül. A minta az FA3 `canonical_json` + `stable_digest` páros. Közös tesztvektorok biztosítják, hogy a
Python- és a Rust-oldal ugyanazt a kivonatot adja.

## 4. Tárolás

- **PostgreSQL** (ADR 0006), saját sémában (`cfa4_audit`), az alkalmazás adatbázisán belül vagy külön
  adatbázisban. **Javaslat: ugyanabban az adatbázisban**, mert így az auditesemény ugyanabban a tranzakcióban
  íródik, mint az üzleti változás. Vagy mindkettő megtörténik, vagy egyik sem. Ezzel kettős írási probléma
  nem keletkezik (vö. Event Fabric E16, outbox).
- **Csak hozzáfűzés, kikényszerítve:**
  - az alkalmazás adatbázis-szerepe az auditsémán csak `INSERT` és `SELECT` jogot kap;
  - egy trigger minden `UPDATE`, `DELETE` és `TRUNCATE` műveletet elutasít;
  - a sémát egy külön tulajdonosi szerep birtokolja, amelyet csak a karbantartó eszköz használ.
- **Lánc:** egy naplón belül egyetlen lineáris hash-lánc. Az írás egy rövid tranzakciós zárral sorosítja a
  `seq` és a `prev_hash` kiosztását. A várható terhelés (egygépes mód, néhány esemény másodpercenként) ezt
  bőven elbírja.
- **Méret (SC-03, ≥ 50 millió esemény):**
  - havi partíciók;
  - index a `subject`, `actor`, `action`, `occurred_at` és `scope` (GIN) mezőkre.

  A lekérdezési célértékek mérése az L mérési profilon történik (SC-02).

## 5. Ellenőrzés

- **`audit verify`**: végigszámolja a láncot (teljesen, vagy egy `seq`-tartományon), és az első eltérésnél
  megáll. Az eredmény a megszakadás helye és oka. A program nem omlik össze, a felületen olvasható jelentés
  jelenik meg.
- Időszakos **ellenőrzőpont**: a lánc aktuális végének kivonata egy külön fájlba is kikerül, a mentés mellé.
  Ezzel a lánc vége is ellenőrizhető, nem csak a belseje.
- A mentésből visszaállítás után a `verify` automatikusan lefut (BCK-07).

## 6. Szereplő (actor)

- **M1 (#41 javaslata):** `actor_kind = os_user`. Az `actor_ref` a helyi Linux-felhasználó uid-ja és login
  neve, a folyamat saját hitelesítő adataiból (nem a felhasználó által beírt név).
- **M2-től:** `actor_kind = identity`, a CFA4 Identity szolgáltatás hitelesített azonosítójával. Az Identity
  az M2 előfeltétele, és külön terv kell hozzá.
- **Ügynök vagy automatizmus:** `actor_kind = agent` vagy `system`. Ha valakinek a nevében jár el, akkor
  kitöltendő az `on_behalf_of` mező is.

## 7. Személyes adatok (SEC-06, GDPR)

- A láncolt eseménybe nem kerül személyes adat. Csak az objektum azonosítója és álneves hivatkozás kerül
  bele. A régi és az új érték személyes mezőnél egy kivonat vagy egy `redacted` jelzés.
- A személyes részletek külön, nem láncolt táblában vannak (`cfa4_audit.personal_detail`), az esemény
  azonosítójához kötve.
- Törlési kérésnél csak ez a tábla törlődik vagy anonimizálódik. A lánc ép marad, és egy `TOMBSTONE`
  esemény rögzíti, hogy a törlés megtörtént (FA3-minta).

## 8. Megőrzés és archívum

- A megőrzés idejét és módját a felhasználó dönti el (#18). Automatikus törlés csak jóváhagyással történik.
- **Archiválás:** a lezárt havi partíció egy önleíró archívumcsomagba kerül. A csomagban van a partíció
  adata, az első és az utolsó `seq`, a határoló hash-ek és a csomag SHA-256 kivonata (FA3-minta).
- **Visszaállítás** csak érvényes kivonattal lehetséges. Az eredeti `event_id` és `seq` megmarad. Maga a
  visszaállítás is auditesemény.
- A fizikai törlés nem érhető el normál felhasználói műveletként.

## 9. Interfész

- **Közös könyvtár** két nyelven, ugyanarra a sémára és tesztvektorokra:
  - Python (`cfa4_audit`): a BacklotHub üzleti logikájához (#21);
  - Rust (`cfa4-audit` crate): az Event Fabric Rust Core-hoz és a motorhoz.
- **Írás:** `record(event, tx)`, mindig a hívó tranzakciójában. Önálló írás csak rendszereseményhez van.
- **Olvasás:** szűrés szereplőre, objektumra, műveletre, időre, produkcióra; lapozás `seq` szerint; export
  JSON-ba és CSV-be.
- **CFA4-parancsok:** `audit.list`, `audit.verify`, `audit.export`. Így a CLI, az MCP és a felület is eléri
  („minden egy parancs”).
- **Event Fabric:** az EF Audit Adapter (EF-CORE-013) ezt a könyvtárat hívja. A Fabric nem auditnapló, és
  az auditnapló sem eseménybusz.

## 10. Hibakezelés

- Auditköteles művelet nem hajtható végre, ha az auditesemény nem írható. Mivel a kettő egy tranzakcióban
  van, ez automatikusan teljesül (fail-closed).
- Minden hiba angol hibakóddal tér vissza (`AUDIT_CHAIN_BROKEN`, `AUDIT_WRITE_DENIED`, …). A felület
  fordít.
- Never crash: sérült lánc, hiányzó partíció vagy teli lemez esetén kezelhető hibát ad, összeomlás nincs.

## 11. Mit vettünk át, mit módosítottunk (terv 3.3 szerint)

| FA3 Unified Journal | CFA4-AUDIT-001 |
|---|---|
| JSONL-fájl, csak hozzáfűzés | **Módosítva:** PostgreSQL, jogosultság + trigger kényszeríti |
| `TOMBSTONE` törlés helyett | **Megtartva** (GDPR-esetre is) |
| Archívum SHA-256 kivonattal, ellenőrzött visszaállítás, eredetjelölés | **Megtartva** |
| Rotáció 5000 eseménynél / 30 napnál | **Módosítva:** havi partíció, felhasználói megőrzés |
| Kivonat csak az archívumra | **Bővítve:** eseményenkénti hash-lánc + ellenőrzőpont |
| Napló = rendszer, beszélgetés, projekt és audit együtt | **Szűkítve:** csak audit; a diagnosztika külön marad |
| FA3 current-host evidence, Canonical Registry, HRB, MCP-hivatkozások | **Elhagyva** (CFA3-elemek) |

## 12. Fázisok és kapuk

| Fázis | Tartalom | Kapu |
|---|---|---|
| AU-0 | Ez a terv, jóváhagyva; ADR a `main`-be | G2 |
| AU-1 | Séma, migráció, csak-hozzáfűzés kényszerítése, kanonikus szerializálás, közös tesztvektorok (Python + Rust) | G4 |
| AU-2 | `record` / `list` / `verify` / `export`, CFA4-parancsok | G4 |
| AU-3 | Tesztek (G5): manipuláció kimutatása (sor átírása, törlése, beszúrása), összeomlás írás közben, mutációs fuzz a bemenetekre, visszaállítás mentésből + `verify`, 50 M eseményes L-profil mérés a CFA4-MIN gépen | G5 |
| AU-4 | Archívum és GDPR-tombstone | G5 |
| AU-5 | BacklotHub Audit Trail felület (Qt6/PySide6, CFA4 Blue): lista, szűrés, objektumtörténet, ellenőrzés | BacklotHub 1. lépcső G4–G6 |

## 13. Nyitott kérdések

- **Ugyanaz vagy külön adatbázis (4. pont):** a javaslat szerint ugyanaz. Az AU-1 G2 kapuján rögzül.
- **pgaudit:** M1-ben nem; M2-szerveren opcionális kiegészítő adatbázisszintű napló. Akkor külön döntés.
