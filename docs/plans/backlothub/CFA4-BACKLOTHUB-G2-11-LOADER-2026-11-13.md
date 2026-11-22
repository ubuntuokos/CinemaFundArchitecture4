# BacklotHub 11. lépcső – Loader: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-13 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.6 · **Előzmény:**
[G1 Loader](CFA4-BACKLOTHUB-G1-11-LOADER-2026-11-06.md) (funkcionális követelmények jóváhagyva 2026-11-10) ·
**Épül rá:** G2-07 (Search), G2-08 (Asset), G2-10 (Publisher, `version_input`) · **Állapot:** G2 **jóváhagyva** (István, 2026-11-14) · implementáció nincs

Új döntési kérdés nincs, új függőség nincs. A G2-08 itt kérte a Universal Search újravizsgálatát (7. pont).

## 1. Hatókör ebben a lépcsőben

- Alkalmazásadapter: **csak a CFA4 szerkesztő** (a futó szerkesztő vezérlőcsatornáján át). Más alkalmazás
  adaptere a 11a. lépcsőben (DCC Host), az Application Launcher után (#46).
- Minden alkalmazáshoz már most: „helyi út másolása” és „mappa megnyitása” (`files.resolve`, `xdg-open`).

## 2. Integrációs határok

```
 BacklotHub (Loader) ──► files.resolve ──► helyi út
        │
        └──► CFA4 szerkesztő vezérlőcsatornája (JSON-lines, docs/control-protocol.md)
               importálás, újrakapcsolás, médiaelem-azonosító vissza
```

| Határ | Irány | Szerződés |
|---|---|---|
| Search (7.) | ← | Jelöltek: `backlot.search.query` verzióra/assetre, a betöltési profil kompatibilitási szűrőjével |
| Asset (8.), Publisher (10.) | ← | Verzió, reprezentáció, `representation_file`, `version_input`, hero-mutató |
| File Services | → | `files.resolve` (gyökérfüggetlen → helyi út), `files.verify` (kérésre betöltés előtt) |
| CFA4 szerkesztő | → | A szerkesztő meglévő importáló és újrakapcsoló (Link Media) parancsai a vezérlőcsatornán; a szerkesztő visszaadja a létrejött médiaelem azonosítóját. A szerkesztő oldalán **új adatmodell nem kell**: a BacklotHub tárolja, mi honnan jött |
| Workfiles (9.) | ← | A célprojekt (`.fcproj`) munkafájl-kontextusa (`backlot.workfile.context`), így a betöltés a munkafájlhoz kötődik |
| DCC Host (11a.) | később | Ugyanez a szerződés más alkalmazásokra |

**Kapcsolat a szerkesztővel:** a BacklotHub külön alkalmazás (#54), ezért a szerkesztő vezérlőcsatornájához
kapcsolódik, csak a helyi gépen. A csatorna `CFA4_CONTROL_TOKEN`-t kér, amelyet csak örökölt környezetből
szabad átadni (fájlba, parancssorba nem; `docs/control-protocol.md`). Ezért:

- **Ha a szerkesztőt a BacklotHub indítja** (CFA4 Application Launcher, lásd a
  [tervvázlatot](CFA4-APP-LAUNCHER-001-TERVVAZLAT-2026-11-13.md)), a Launcher munkamenetenként új tokent
  generál, a szerkesztő környezetébe adja, a BacklotHub pedig a memóriájában tartja. Ez a szokásos út.
- **Ha a szerkesztőt a felhasználó külön indította**, a BacklotHub nem fér hozzá a tokenhez, és nem is kéri
  be: `LOAD_TARGET_UNAVAILABLE`, a felület a „helyi út másolása” lehetőséget ajánlja, és azt, hogy a
  szerkesztőt a BacklotHubból nyissa meg.

Ha a szerkesztő nem fut vagy nem érhető el: ugyanez a hibakód és ugyanez az ajánlás.

**Következmény a sorrendre:** a szerkesztőbe töltés az Application Launcher első fázisa (AL-1) után
működik; a 11. lépcső megvalósítása ezért az AL-1-re is vár. A „helyi út másolása” és a „mappa megnyitása”
nélküle is működik.

## 3. Adatmodell (DDL-vázlat, `backlot` séma)

```sql
CREATE TABLE backlot.load_profile (
    id uuid PRIMARY KEY, code text NOT NULL, version integer NOT NULL,
    app text NOT NULL,                              -- 'cfa4'; a 11a.-tól más is
    state text NOT NULL CHECK (state IN ('draft','active','retired')),
    definition jsonb NOT NULL,                      -- elfogadott reprezentációk és ellenőrzések (4. pont)
    production_id uuid REFERENCES backlot.production(id),
    ...közös oszlopok production_id nélkül,
    UNIQUE (code, version)
);

CREATE TABLE backlot.load_record (               -- „container” (AYON-minta)
    id                uuid PRIMARY KEY,
    production_id     uuid NOT NULL REFERENCES backlot.production(id),
    version_id        uuid NOT NULL REFERENCES backlot.version(id) ON DELETE RESTRICT,
    representation_id uuid NOT NULL REFERENCES backlot.representation(id) ON DELETE RESTRICT,
    app               text NOT NULL,
    workfile_id       uuid REFERENCES backlot.workfile(id),      -- ha munkafájlba töltődött
    target_ref        jsonb NOT NULL,              -- pl. {"project": "<fcproj uuid>", "media_item": "<id>"}
    mode              text NOT NULL CHECK (mode IN ('import','reference')),
    state             text NOT NULL CHECK (state IN ('active','replaced','removed')),
    replaced_by_id    uuid REFERENCES backlot.load_record(id),
    loaded_by_person_id uuid NOT NULL REFERENCES backlot.person(id),
    ...közös oszlopok
);
CREATE INDEX ON backlot.load_record (workfile_id) WHERE state = 'active';
CREATE INDEX ON backlot.load_record (version_id);
```

- A `load_record` egyben a **Publisher bemenet-forrása**: ha a munkafájlból publikálnak, az aktív betöltések
  verziói lesznek a `version_input` sorai (G2-10).
- **Betöltési előzmény** = a `load_record`-ok (`replaced`, `removed` is megmarad) + az auditnapló
  `load.*` eseményei.

## 4. Betöltési profil és kompatibilitás

```json
{"schema": "cfa4.backlot.load_profile/1", "app": "cfa4",
 "accepts": [
   {"role": "original", "kinds": ["single", "sequence"], "extensions": ["mov","mxf","mp4","exr","dpx","wav"]},
   {"role": "review_proxy"}],
 "checks": [{"id": "fps_matches_target"}, {"id": "resolution_max", "params": {"width": 8192}}],
 "mode": "import"}
```

- Ellenőrzők zárt regiszterből (mint a G2-10-ben), a reprezentáció `probe` adataira és a célprojekt
  tulajdonságaira (a szerkesztőtől lekérdezve). Nem megfelelőnél `LOAD_INCOMPATIBLE`, az ok megnevezésével;
  figyelmeztetésnél (pl. eltérő fps) a felhasználó dönt.
- Ha több reprezentáció is elfogadható, a profil sorrendje dönt (pl. `original`, ha nincs, `review_proxy`).

## 5. Függőségfeloldás

- `version_input` tranzitív lezárása rekurzív lekérdezéssel (kör nem lehet, G2-10).
- Alapértelmezés: **pontosan** a bemeneti verziók; választható: termékenként a hero (legutóbb jóváhagyott)
  verzió.
- A már aktívan betöltött függőség nem töltődik be újra.

## 6. Futás, frissítés, hibakezelés

- **Betöltés:** jelöltek → profil-ellenőrzés → függőségek → `files.resolve` → a szerkesztő importáló
  parancsai → `load_record`-ok és audit egy tranzakcióban.
- **Félig betöltött állapot nem marad:** ha N elemből egy elbukik, a már importáltakat a Loader eltávolítja a
  szerkesztőből (a szerkesztő parancsaival), és nem ír `load_record`-ot. Ha az eltávolítás is elbukik (pl. a
  szerkesztő közben bezárult), a hiba és az érintett elemek listája a felhasználónak jelenik meg; az
  adatbázisban akkor sincs félkész rekord.
- **„Van újabb verzió”:** a munkafájl aktív betöltései közül azok, amelyek termékének hero verziója (vagy
  legutóbbi publikált verziója) újabb. Lekérdezéskor számolva, tárolt jelző nincs.
- **Frissítés egy lépésben:** a szerkesztő újrakapcsoló (Link Media) parancsa az új verzió útjára → a régi
  rekord `replaced`, az új `active`, egy tranzakcióban. A szerkesztőbeli vágási adatok (be- és kipont)
  változatlanok; ha az új verzió rövidebb, `LOAD_UPDATE_RANGE_SHORTER` figyelmeztetés.

## 7. Universal Search (#44, a G2-08 kérésére)

**Javaslat: továbbra sem kell CFA4 Universal Search.** A Loader a `backlot.search.query`-t használja, a
szerkesztőnek nincs szüksége saját keresőre a BacklotHub verzióihoz. Közös keresés (szerkesztőprojektek,
médiatár és BacklotHub együtt) M2-igény; akkor külön terv. Újravizsgálat: a 26. lépcső (Archive) G2-jén, vagy
ha M2 elindul.

## 8. Parancsok

| Parancs | Mit csinál |
|---|---|
| `backlot.load.candidates` | Betölthető verziók a profil szerint (keresés + kompatibilitás) |
| `backlot.load.check` | Szárazfuttatás: kompatibilitás és függőségek, írás nélkül |
| `backlot.load.run` | Betöltés a célba (szerkesztő), függőségekkel |
| `backlot.load.inventory` | Egy munkafájl/projekt aktív betöltései, „van újabb” jelzéssel |
| `backlot.load.update` | Frissítés újabb verzióra |
| `backlot.load.remove` | Betöltés eltávolítása (a szerkesztőből is) |
| `backlot.load.copy_path` / `.open_folder` | Bármely alkalmazáshoz |
| `backlot.load.history` | Előzmény |
| `backlot.load_profile.*` | Profilok |

## 9. Hibakódok

`LOAD_INCOMPATIBLE`, `LOAD_TARGET_UNAVAILABLE`, `LOAD_PARTIAL_ROLLBACK_FAILED`, `LOAD_UPDATE_RANGE_SHORTER`,
`LOAD_PROFILE_INVALID`, `LOAD_DEPENDENCY_MISSING`, valamint a `FILES_*` kódok (`FILES_ROOT_UNAVAILABLE`,
`FILES_HASH_MISMATCH`).

## 10. Tesztek (G5-re előre)

- Szerződésteszt a szerkesztő vezérlőcsatornájával: importálás, újrakapcsolás, eltávolítás; a szerkesztő
  közben bezár → kezelt hiba, nincs félkész rekord.
- Függőségfeloldás: lánc és gyémánt alakú függőség, ismétlés nélkül.
- Frissítés: a vágási adatok megmaradnak; rövidebb új verziónál figyelmeztetés.
- Nem elérhető gyökér, sérült fájl: hibakód, összeomlás nincs (a szerkesztőben sem).
- Felületi ellenőrzés: `ui.screenshot` a szerkesztőben a betöltött elemről.

## 11. Licenc (G3-ra előre)

Új függőség nincs. AYON core (Apache-2.0): loader-bővítmény, container és scene inventory mint minta;
kódátvételnél attribúcióval. Kitsu (AGPL): referencia.

## 12. G2-eredmény

**Javaslat: PASS.** A más alkalmazásokhoz szóló adapterek a 11a. lépcső (DCC Host) és az Application Launcher
után jönnek.
