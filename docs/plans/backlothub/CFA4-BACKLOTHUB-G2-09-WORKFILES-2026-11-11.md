# BacklotHub 9. lépcső – Workfiles: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-11 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.7 · **Döntés:** #46 (indítás nélkül; „megnyitás
alkalmazásban” az Application Launcher után) · **Előzmény:** [G1 Workfiles](CFA4-BACKLOTHUB-G1-09-WORKFILES-2026-11-06.md)
(funkcionális követelmények jóváhagyva 2026-11-10) · **Épül rá:** G2-01…G2-08 · **Állapot:** G2 **jóváhagyva** (István, 2026-11-11) · implementáció nincs

Új döntési kérdés nincs, új függőség nincs.

## 1. Közös úttemplate (a 8. és a 10. lépcsővel együtt)

A munkafájl útja (`work` gyökér) és a publikált verzió útja (`publish` gyökér, G2-08) **ugyanazzal a
sablonmotorral** készül. Egy motor, egy mezőregiszter, egy ellenőrzés.

- **Szintaxis:** `{mező}` vagy `{mező:formázó}`. Formázók: `03` (nullával kitöltött szám), `lower`, `upper`.
  Más nincs: nincs kifejezés, feltétel, ciklus (a Stalker Jinja2-jét a G1 elvetette).
- **Mezők** (a kontextusból, regiszterben): `production.code`, `entity.code`, `entity.type`, `entity.path_codes`
  (az ősök kódjai `/`-rel), `<típus>.code` (pl. `sequence.code`, `shot.code`, az ősök közül), `task.type`,
  `task.code`, `product.type`, `product.name`, `version`, `profile.code`, `ext`, `date` (ÉÉÉÉHHNN).
- **Ellenőrzés mentéskor:** minden mező ismert; a sablon kiterjesztéssel végződik.
- **Ellenőrzés feloldáskor:** minden mező feloldható (`TEMPLATE_FIELD_MISSING`, pl. nincs `sequence` ős); a
  kész út csak `[A-Za-z0-9._-]` karaktereket és `/`-t tartalmaz (ékezet és szóköz `unaccent`-tel és `_`-re
  cserélve, előre látható módon); szakaszonként ≤ 255 bájt; nincs `..`; a gyökéren belül marad (ezt a File
  Services is ellenőrzi, ADR 0009).
- Alapsablonok (migrációban):
  - munkafájl: `{production.code}/{entity.path_codes}/{task.type}/work/{profile.code}/{entity.code}_{task.type}_v{version:03}.{ext}`
  - publikálás: G2-08, 5. pont.

```sql
CREATE TABLE backlot.path_template (
    id uuid PRIMARY KEY, code text NOT NULL UNIQUE,
    purpose text NOT NULL CHECK (purpose IN ('workfile','publish','delivery','archive')),
    pattern text NOT NULL,
    production_id uuid REFERENCES backlot.production(id),   -- NULL: közös; produkciós felülírás lehetséges
    ...közös oszlopok production_id nélkül
);
```

## 2. Profilok és munkafájlok

```sql
CREATE TABLE backlot.workfile_profile (
    id uuid PRIMARY KEY,
    code text NOT NULL UNIQUE CHECK (code ~ '^[a-z][a-z0-9_]{1,31}$'),   -- cfa4, blender, krita, natron
    label text NOT NULL,
    extensions text[] NOT NULL,                     -- {fcproj}, {blend}, {kra}, {ntp}
    template_file_object_id uuid REFERENCES cfa4_files.file_object(id),   -- kezdő fájl, opcionális
    path_template_id uuid NOT NULL REFERENCES backlot.path_template(id),
    subfolders text[] NOT NULL DEFAULT '{}',        -- pl. {renders, cache}, a munkafájl mellett
    retired_at timestamptz
);

CREATE TABLE backlot.workfile (
    id              uuid PRIMARY KEY,
    production_id   uuid NOT NULL REFERENCES backlot.production(id),
    task_id         uuid NOT NULL REFERENCES backlot.task(id) ON DELETE RESTRICT,
    profile_id      uuid NOT NULL REFERENCES backlot.workfile_profile(id),
    number          integer NOT NULL CHECK (number > 0),
    file_object_id  uuid NOT NULL REFERENCES cfa4_files.file_object(id) ON DELETE RESTRICT,  -- mutable
    author_person_id uuid NOT NULL REFERENCES backlot.person(id),
    note            text,
    registered_via  text NOT NULL CHECK (registered_via IN ('create','save_version','scan','editor','dcc')),
    retired_at      timestamptz,
    ...közös oszlopok,
    UNIQUE (task_id, profile_id, number)
);
```

- A sorszám **feladatonként és profilonként** folytonos (egy feladaton dolgozhatnak Blenderben és Kritában is),
  a feladat sorának zárolásával kiosztva.
- A munkafájl `mutable` (ADR 0009): az alkalmazás menti, a tartalma változhat; a katalógus kivonata a
  `files.verify`-jal vagy a következő regisztrációkor frissül. Lezárni a Publisher (10.) zárja le a publikált
  másolatot, nem a munkafájlt.
- Profilok (kezdő adatok): CFA4 szerkesztő (`.fcproj`), Blender, Krita, Natron. Csak adat; alkalmazást a
  BacklotHub nem indít (#46).

## 3. Regisztráció

| Út | Mit csinál |
|---|---|
| `create` | Út a sablonból → a profil kezdő fájlja (ha van) `files.import`-tal a `work` gyökérbe → `workfile` sor. Ha nincs kezdő fájl, üres hely, a felhasználó ide ment |
| `save_version` | A legutóbbi munkafájl másolata a következő sorszámú útra (`files.import`, ellenőrzött másolás) → új sor |
| `scan` | A feladat munkamappájában a sablonnak megfelelő, még nem regisztrált fájlok listája; a felhasználó választja ki, mit regisztrál (automatikusan nem) |
| `editor` | A CFA4 szerkesztő `backlot.workfile.register` parancsa a `.fcproj` útjával; a BacklotHub-szolgáltatás a vezérlőcsatornán kapja (#54) |
| `dcc` | Más alkalmazásból a 11a. lépcsőtől (DCC Integration Host) |

Minden út egy tranzakció: katalógus + munkafájl + audit. Ütközésnél (a fájl már létezik a sablonúton, de nincs
regisztrálva) nincs felülírás: `WORKFILE_PATH_OCCUPIED`, a felület a `scan`-t ajánlja.

**Megnyitás:** a munkafájl mappája `xdg-open`-nel (a rendszer fájlkezelője, nem alkalmazásindítás, #46). A
„megnyitás az alkalmazásban” parancs a CFA4 Application Launcher után kapcsol be (a 11a. lépcső előfeltétele);
addig a parancs **nem létezik** (nem letiltott gomb).

## 4. Integrációs határok

| Határ | Irány | Szerződés |
|---|---|---|
| File Services | → | `files.import` (ellenőrzött másolás, felülírás nélkül), `files.resolve`, `files.verify`; `work` gyökér |
| Task (5.) | ← | A munkafájl feladathoz tartozik; a feladat nézetében listázva |
| Structure (3.) | ← | A sablon mezői az elem őseiből |
| Publisher (10.) | → | A publikálás forrása egy munkafájl (vagy annak kimenete) |
| CFA4 szerkesztő | ← | `backlot.workfile.register`, `backlot.workfile.context` (melyik feladathoz tartozik a megnyitott `.fcproj`) |
| Application Launcher, DCC Host (11a.) | később | Megnyitás alkalmazásban, automatikus regisztráció |
| Workspace / zárolás | M2 | Szereplőnkénti alkönyvtár és zár az AYON-minta szerint; M1-ben a `work` gyökér a felhasználóé |

## 5. Parancsok

`backlot.workfile.create`, `.save_version`, `.scan`, `.register`, `.list`, `.latest`, `.open_folder`,
`.context`, `.retire`; `backlot.path_template.*` (lista, mentés, `preview`: a sablon feloldása egy adott
feladatra, írás nélkül); `backlot.workfile_profile.*`.

## 6. Hibakódok

`TEMPLATE_FIELD_UNKNOWN`, `TEMPLATE_FIELD_MISSING`, `TEMPLATE_PATH_INVALID`, `WORKFILE_PATH_OCCUPIED`,
`WORKFILE_PROFILE_UNKNOWN`, `WORKFILE_NOT_FOUND`, valamint a `FILES_*` kódok.

## 7. Tesztek (G5-re előre)

- Sablonmotor: tulajdonságalapú teszt véletlen kódokkal (ékezet, szóköz, `..`, `/`, nagyon hosszú név):
  az eredmény mindig érvényes és a gyökéren belüli, vagy hibakód; kód nem fut.
- Párhuzamos `save_version`: folytonos sorszám, nincs felülírás.
- Összeomlás másolás közben: a katalógus nem mutat félkész fájlra (File Services FS-1 garancia), a
  munkafájl-sor sincs meg.
- `scan`: csak a sablonnak megfelelő, nem regisztrált fájlok; a regisztrált fájl nem jelenik meg újra.

## 8. Licenc (G3-ra előre)

Új függőség nincs. AYON core (Apache-2.0): útfeloldás és következő verziószám mintája, adaptálható; kód
átvétele esetén attribúcióval (`ATTRIBUTION.md`). Stalker (LGPL): sablon-szerkezet mint minta. Kitsu (AGPL):
munkafájl-mezők mint referencia.

## 9. G2-eredmény

**Javaslat: PASS.** A „megnyitás alkalmazásban” a CFA4 Application Launcher tervéhez és megvalósításához kötött
(#46), ami a 11a. lépcső előfeltétele.
