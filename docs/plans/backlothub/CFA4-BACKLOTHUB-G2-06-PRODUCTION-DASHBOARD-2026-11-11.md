# BacklotHub 6. lépcső – Production Dashboard (alap): G2 architektúra

**Dátum:** 2026-11-11 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.11 · **NFR:** PERF-05 (frissítés ≤ 2 s) ·
**Előzmény:** [G1 Production Dashboard](CFA4-BACKLOTHUB-G1-06-PRODUCTION-DASHBOARD-2026-11-06.md) (funkcionális
követelmények jóváhagyva 2026-11-10) · **Épül rá:** G2-01…G2-05 (jóváhagyva 2026-11-11) · **Állapot:** G2 **jóváhagyva** (István, 2026-11-11; #54 eldöntve) · implementáció nincs

## 1. Új döntési kérdés: hol fut a BacklotHub felülete? (#54)

Ez az első lépcső, amelynek a lényege a felület. A kérdés azonban az 1–5. lépcsőt is érinti (auditlista,
produkció- és feladatűrlapok), és a G2-02-ben nem rögzítettük.

| Lehetőség | Előny | Hátrány |
|---|---|---|
| **A. Külön PySide6-alkalmazás** (`cfa4-backlothub`, saját menübejegyzés), a `cfa4_qt` témáját, i18n-jét és közös vezérlőit használja | A szerkesztőtől független: a BacklotHub hibája nem érinti a vágást, és fordítva (AGENTS.md §0). Produkciós munkatárs vágóprogram nélkül is használhatja. Saját kiadási ütem | Két ablak a vágónál; a „feladat megnyitása a szerkesztőben” parancson át megy |
| B. Panelként a Qt6-héjban (`cfa4_qt`) | Egy ablak | A héj még prototípus (AGENTS.md); egy BacklotHub-hiba a szerkesztő felületét is érintheti; a produkciós munka a szerkesztőhöz kötődik |

**Javaslat: A.** A szerkesztő egy `backlot.open` paranccsal nyitja a BacklotHubot (és fordítva, a 9. és a 11a.
lépcsőben a munkafájl a szerkesztőben nyílik). A közös vezérlők (téma, i18n, ikonok) a `cfa4_qt`-ből jönnek,
nem másolatként.

## 2. Felület és diagramok

- **Csempék:** szám, állapotsáv (halmozott vízszintes sáv), lista, mérföldkő-idővonal. Ezek egyszerű
  QWidget/QPainter-elemek a CFA4 Blue témával. **Az alap lépcsőhöz diagrammodul nem kell.**
- **QtGraphs / QtCharts:** a Qt a QtGraphs-t ajánlja a QtCharts utódjaként, a QtGraphs 2D-s nézete viszont QML-alapú (a
  `cfa4_qt` csak QtWidgets-et használ, QQuickWidget kellene hozzá). Ezért ezek csak akkor kerülnek be, amikor
  valódi diagram kell (pl. kapacitásgörbe a 15. lépcsőben); akkor a 15. lépcső G2-je dönt, és a G3-on
  ellenőrizzük a licencet (a Qt e modulja nyílt forrásban GPL-3.0, ami a CFA4 GPL-3.0-or-later licencével
  összeegyeztethető).

## 3. Integrációs határok

| Határ | Irány | Szerződés |
|---|---|---|
| Adatforrások | ← | Csak élő lekérdezés: produkció, mérföldkő (2.), elem (3.), folyamatpéldány és határidő-nézet (4.), feladat (5.), audit (1.). Saját dashboard-adat nincs |
| Csempe-regiszter | modulok → | Minden modul kódban regisztrálja a csempéit (`tile_id`, paraméterséma, adatparancs). A 10., 13., 15. lépcső a saját csempéjét hozza; addig az nem létezik (nem üres helyőrző) |
| Lefúrás | → | Minden szám egy szűrőobjektumot ad (pl. a `backlot.task.list` paraméterei); a felület azzal nyitja a listát |
| Felhasználó | ↔ | `dashboard_layout` személyenként; a megjelenítést tárolja, adatot nem |
| Event Fabric | később | Eseményvezérelt frissítés a 12. lépcsőtől |

## 4. Csempék az alap lépcsőben

| Csempe | Tartalom | Forrás |
|---|---|---|
| `production.overview` | Az összes aktív produkció (#9: ≤ 52) státusszal, következő mérföldkővel, késésszámmal | 2., 4., 5. |
| `production.status` | Egy produkció státusza, kezdete, vége, mérföldkövei | 2. |
| `milestones.upcoming` | Következő és lejárt mérföldkövek | 2. |
| `tasks.matrix` | Státuszkategória × feladattípus, egy választott szinten (pl. szekvenciánként) | 3., 5. (Kitsu-minta) |
| `tasks.overdue` / `tasks.due_soon` | Késedelmes és 7 napon belül esedékes feladatok | 4. határidő-nézet, 5. |
| `tasks.mine` | Az aktuális személy feladatai | 5. (#53 `person`) |
| `entities.progress` | Részfa készültsége (lezárt / összes feladat), szülőre összesítve | 3., 5. (Stalker-minta) |
| `changes.recent` | Legutóbbi változások a produkcióban | 1. (`audit.list` a `scope` szerint) |

**„Késedelmes”** ugyanaz a szabály, mint a 4. lépcső határidő-nézetében és a `backlot.task.list` „lejárt”
szűrőjében: határidő elmúlt, és a feladat nincs végállapotban. Egy helyen van definiálva (SQL-nézet).

## 5. Adatmodell

```sql
CREATE TABLE backlot.dashboard_layout (
    id            uuid PRIMARY KEY,
    person_id     uuid NOT NULL REFERENCES backlot.person(id),
    production_id uuid REFERENCES backlot.production(id),   -- NULL: áttekintő
    name          text NOT NULL,
    layout        jsonb NOT NULL,     -- [{tile_id, x, y, w, h, params}], sémával validálva
    is_default    boolean NOT NULL DEFAULT false,
    ...közös oszlopok production_id nélkül
);
CREATE UNIQUE INDEX ON backlot.dashboard_layout (person_id, production_id) NULLS NOT DISTINCT WHERE is_default;

CREATE VIEW backlot.task_overdue AS …;     -- a 4. lépcső határidő-szabálya + due_date, egy helyen
CREATE INDEX ON backlot.task (production_id, task_type_id, status_id) WHERE retired_at IS NULL;
```

- A nézet elrendezése **megjelenítési beállítás**, ezért nem auditált (nem üzleti adat). Ismeretlen vagy
  időközben megszűnt csempe a betöltéskor kimarad, figyelmeztetéssel; a nézet nem sérül meg.
- Alapnézetek (migrációban): „Áttekintés” (összes produkció) és „Produkció” (egy produkció).

## 6. Frissítés és teljesítmény (PERF-05)

- Frissítés megnyitáskor, kézzel és időzítve (alapérték 60 s).
- **Olcsó változásfigyelés:** a frissítés előtt a szolgáltatás az auditlánc végének `seq`-jét kérdezi le
  (`cfa4_audit.chain_head`, egy sor). Ha nem változott, nincs újraszámolás. Minden üzleti változás
  auditeseményt ír, ezért ez megbízható jelzés.
- Az összesítő lekérdezések a fenti indexekre épülnek. **Mérés az L-profilon** (≈ 1,9 millió feladat, 52
  produkció) a CFA4-MIN gépen. Ha a PERF-05 nem teljesül, akkor és csak akkor jön tranzakcióban frissített
  összesítő tábla vagy materializált nézet, a mért csempére.
- A csempék párhuzamosan töltődnek; egy csempe hibája csak azt a csempét jelöli hibásnak (hibakóddal), a
  többi megjelenik.

## 7. Parancsok

| Parancs | Mit csinál |
|---|---|
| `backlot.dashboard.tiles` | Az elérhető csempék leírói |
| `backlot.dashboard.tile` | Egy csempe adata (`tile_id`, paraméterek, szűrők), JSON; a CLI, az MCP és az ügynökök is ezt használják |
| `backlot.dashboard.summary` | Egy produkció összes alapcsempéje egyben |
| `backlot.dashboard.layout.list` / `.save` / `.delete` / `.set_default` | Nézetek |
| `backlot.open` | A BacklotHub megnyitása (a szerkesztőből is), opcionálisan egy produkcióra vagy objektumra (#54) |

## 8. Hibakódok

`DASHBOARD_TILE_UNKNOWN`, `DASHBOARD_TILE_FAILED`, `DASHBOARD_LAYOUT_INVALID`, `PRODUCTION_NOT_FOUND`.

## 9. Tesztek (G5-re előre)

- Minden csempe számai egyeznek a lefúrással kapott lista elemszámával (ugyanaz a szűrő).
- PERF-05: S- és L-profil, hideg és meleg gyorsítótárral, a CFA4-MIN gépen.
- Egy csempe adatparancsa hibát dob: a többi csempe megjelenik, a felület nem omlik össze.
- Felületi teszt a vezérlőcsatornán: nézet mentése, betöltése, ismeretlen csempe kezelése (`ui.screenshot`).

## 10. Licenc (G3-ra előre)

Új függőség nincs (QtWidgets, a PySide6 már a függőségek között). Kitsu (AGPL): statisztika-mátrix mint minta;
Stalker (LGPL): készültség-összesítés mint minta; kód egyikből sem.

## 11. Döntési kérdés

| # | Kérdés | Javaslat |
|---|---|---|
| 54 | Hol fusson a BacklotHub felülete? | Külön PySide6-alkalmazás (`cfa4-backlothub`), a `cfa4_qt` közös témájával és vezérlőivel; a szerkesztőből `backlot.open` paranccsal nyílik |

## 12. G2-eredmény

**Javaslat: PASS a #54 jóváhagyásával.** A diagrammodul választása a 15. lépcsőre marad.
