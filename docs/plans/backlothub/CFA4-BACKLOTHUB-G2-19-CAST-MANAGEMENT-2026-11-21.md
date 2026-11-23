# BacklotHub 19. lépcső – Cast Management: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-21 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.16 · **NFR:** SEC-06 (GDPR) · **Előzmény:**
[G1 Cast Management](CFA4-BACKLOTHUB-G1-19-CAST-MANAGEMENT-2026-11-07.md) (funkcionális követelmények
jóváhagyva 2026-11-10) · **Épül rá:** G2-03 (karakter mint elem, `casting` kapcsolat), G2-05 (`person`), G2-14
(szerződés mint dokumentum), G2-15 (erőforrás, szabadság), G2-17 (forgatási nap, rétegek), G2-18 (közös személyes
adat, anonimizálás) · **Állapot:** G2-javaslat, jóváhagyásra vár · implementáció nincs

Új döntési kérdés nincs, új függőség nincs (a `btree_gist` contrib a G2-18 óta kell). A G2-18 két pontján
kiegészítés kell (2. pont), új tábla nélkül.

## 1. Hatókör ebben a lépcsőben

- **Karakter** (a történet része) és **szereplő** (valós személy) két külön fogalomként, a **cast ID**-val.
- **Szereposztás:** szereplő ↔ karakter, szerepkörrel és időszakkal; egy karakternek több szereplője is lehet.
- **Jelenetkapcsolat:** a karakter melyik jelenetben, snittben szerepel (G2-03 `casting` kapcsolat).
- **Behívás a forgatási napokra** és a hozzá tartozó ütközésjelzés (szabadság, szerződött napok).
- **Szerződéses metaadat:** szerződés-hivatkozás, szerződött és opciós napok, pénznem.
- **Szereposztási lista export** (CSV, PDF).
- **Nem része:** díjak és összegek (24. lépcső, Budget Tracking, szűkített eléréssel), a szerződés szövege és
  aláírása (G2-14 dokumentum), a forgatási terv (23.), a jelenetekből automatikus karakter- és behívás-kinyerés
  (Script Breakdown, 22.; kézi felvitel itt is lehetséges), munkaidő-korlátok országonkénti szabályai
  (kiskorúaknál csak a jelzés, 4.2).

## 2. Kiegészítések a G2-18-hoz

| Pont | Kiegészítés |
|---|---|
| Törlő kezelők (G2-18 5.3) | A szereplő-modul regisztrál egyet: törli a `cast_personal_detail` sort és a `cast_profile.note` szöveget (4.1) |
| Parancsok (G2-18 7.) | A személyes részletek közös parancsa `backlot.person.personal.get/set`, amelyet a szereplők felülete is használ; a G2-18 szövege **átvezetve** |

## 3. Integrációs határok

| Határ | Irány | Szerződés |
|---|---|---|
| Structure (3.) | ← | A **karakter** `character` típusú `production_entity` (asset-fajta, nincs szülője, G2-03); a jelenet–karakter kapcsolat az `entity_link` `casting` típusa (3.1) |
| Person (5.), Crew (18.) | ← | A szereplő a közös `person`; a személyes adat a `person_personal_detail`, szintekkel (G2-18 5.1); a `person.anonymize` a szereplőre is érvényes |
| Document (14.) | ← | A szerződés `contract` típusú dokumentum; a `cast_engagement` hivatkozik rá |
| Resource (15.) | ← | Elérhetőség = az erőforrás naptára és szabadsága; a szereplő tervezhetővé tétele `backlot.cast.enable_planning` (a `resource.create`-et hívja) |
| Calendar (17.) | ↔ | A `shooting_day` a naptár tábla; új réteg a naptárba: `cast_calls` (megjelenítés, szerkesztés a `backlot.cast.call.*`-on át, 3.2) |
| Search (7.) | → | `SearchProvider`: szereplő (megjelenített név, típus); a karakter a G2-03 szerint már keresett, a `cast_no` a találat kiegészítő mezője |
| Budget (24.) | később | A szerződött napok és a pénznem az alap; az összegek a Budgetben vannak |
| Script Breakdown (22.), Shooting Schedule (23.) | később | A 22. a karaktereket és a `casting` kapcsolatokat ugyanezekkel a parancsokkal tölti; a 23. a `cast_call` táblát bővíti (hívási idők), azonosító változatlan |
| Audit (1.) | → | Minden írás auditált, a személyes mezők redaktálva, a megtekintés auditált (G2-18) |

### 3.1 A `casting` kapcsolat iránya

A G2-03 az `entity_link` táblát általánosan, `casting` típusnévvel említi, irány nélkül. **Javaslat:**
`from_id` = a **jelenet vagy a snitt** (vagy a szekvencia), `to_id` = a **karakter**, `link_type = 'casting'` („ebben
a jelenetben szerepel”). A szolgáltatás ellenőrzi: a `from` típusa `scene`, `shot` vagy `sequence`, a `to` típusa
`character` (`CAST_LINK_INVALID`); a G2-03 `UNIQUE (from_id, to_id, link_type)` szabálya ugyanazt a párt nem engedi
kétszer. A kapcsolat saját parancsa `backlot.cast.link.add` / `.remove`, amely az entitás-kapcsolat parancsát
hívja a fenti ellenőrzéssel; a Script Breakdown (22.) ugyanezt használja. **A 23. lépcső kiegészítése** ([G2-23](CFA4-BACKLOTHUB-G2-23-SHOOTING-SCHEDULE-2026-11-21.md) 4.1, 4.5): a `cast_call` új oszlopai (`call_time`, `makeup_start`, `wardrobe_start`) és `source = 'schedule'`; a kapcsolat `entity_link.metadata`-ja opcionálisan `{"variant": "…"}`-t hordozhat (a szereposztás változatával egyezően), amelyet az ütemezés a szereplő kiválasztásához használ.

## 4. Adatmodell (DDL-vázlat, `backlot` séma)

### 4.1 Szereplő és karakter

```sql
CREATE TABLE backlot.character_profile (                -- a `character` elem produkciós kiegészítése
    entity_id uuid PRIMARY KEY REFERENCES backlot.production_entity(id) ON DELETE RESTRICT,   -- trigger: típus = character
    production_id uuid NOT NULL REFERENCES backlot.production(id),
    cast_no integer CHECK (cast_no > 0),                  -- sorszám a diszpozícióban és a forgatási tervben
    importance text NOT NULL DEFAULT 'supporting' CHECK (importance IN ('lead','supporting','day_player','background')),
    ...közös oszlopok,
    UNIQUE (production_id, cast_no) DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE backlot.cast_profile (                      -- „ez a személy szereplő” + típus
    person_id uuid PRIMARY KEY REFERENCES backlot.person(id) ON DELETE RESTRICT,
    kind text NOT NULL CHECK (kind IN ('actor','extra','stunt','double','voice')),
    is_minor boolean NOT NULL DEFAULT false,              -- kiskorú: a felhasználó állítja; az ütemezéshez (23.)
    agency_organization_id uuid REFERENCES backlot.organization(id),
    note text CHECK (length(note) <= 2000),               -- NEM személyes jegyzet
    retired_at timestamptz,
    ...közös oszlopok production_id nélkül
);

CREATE TABLE backlot.cast_personal_detail (               -- a Cast-specifikus személyes adat; a közös a person_personal_detail
    person_id uuid PRIMARY KEY REFERENCES backlot.person(id) ON DELETE RESTRICT,
    agent_contact jsonb,                                  -- {name, phone, email}: az ügynök adata (harmadik fél személyes adata)
    updated_at timestamptz NOT NULL, updated_by uuid NOT NULL REFERENCES backlot.person(id),
    row_version integer NOT NULL
);
```

- **A karakter** a G2-03 `production_entity`-je (`character` típus: kód, név, leírás, állapot, metaadat); a
  `character_profile` a két karakter-specifikus tulajdonságot adja: a **cast ID**-t és a **fő- vagy mellékszerep** fokot.
  A karakter létrehozása (`backlot.cast.character.create`) egy tranzakcióban az entitás-létrehozó parancsot és a
  profilt hívja; a karakter törlése/visszavonása az entitásé (G2-03 szabályaival).
- **Cast ID:** produkción belül egyedi (halasztott egyediségi korlát, így átszámozás egy tranzakcióban lehetséges:
  `backlot.cast.character.renumber`); az új karakter a legnagyobb szám + 1-et kapja, ha nem adnak meg mást (a sorszám
  kiosztása a produkció soránál lévő zárral, mint a `last_version` a G2-08-ban). A cast ID **a karakter**
  száma, nem a személyé: ha a szereplőt lecserélik, a szám marad.
- **A szereplő típusa** (`kind`): `actor`, `extra` (statiszta), `stunt` (kaszkadőr), `double` (dublőr), `voice`
  (szinkronhang). A gyerekszereplő **nem külön típus**, hanem az `is_minor` jelző (egy gyerek is lehet színész vagy
  statiszta). Az `is_minor` **a felhasználó által állított üzleti jelzés**, nem a születési dátumból számolt érték,
  mert az idő múlásával elavulna; a születési dátum megadásakor a felület **javasolja** a jelzést. A kiskorúakra
  vonatkozó munkaidő-szabályokat (országonként eltérnek) a rendszer **nem tartalmazza**; a jelzés a Shooting Schedule
  (23.) figyelmeztetéséhez kell.
- **Személyes adat:** a legal név, elérhetőség, születési dátum, cím stb. a közös `person_personal_detail`-ben vannak
  (G2-18), ugyanazokkal a szintekkel; az **ügynök** elérhetősége a harmadik fél adata, ezért a `cast_personal_detail`-ben,
  a `contact` szinten. A megtekintés és a módosítás auditált és redaktált, mint a stábnál.
- A **megjelenített név** a szereposztásban gyakran **művésznév** (a `person.display_name` álnév is lehet): a lista és
  az export ezt használja, a jogi nevet csak a jogosult látja.

### 4.2 Szereposztás, behívás, szerződés

```sql
CREATE TABLE backlot.cast_assignment (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    person_id uuid NOT NULL REFERENCES backlot.person(id),
    character_id uuid NOT NULL REFERENCES backlot.production_entity(id),    -- trigger: character, ugyanaz a produkció
    capacity text NOT NULL CHECK (capacity IN ('principal','understudy','double','stunt_double','voice')),
    variant text CHECK (length(variant) <= 80),            -- pl. „gyerekkori”, „felnőtt”
    valid_from date, valid_until date,                     -- NULL: nincs megkötés
    note text CHECK (length(note) <= 500),
    retired_at timestamptz,
    ...közös oszlopok,
    CHECK (valid_until IS NULL OR valid_from IS NULL OR valid_until >= valid_from),
    EXCLUDE USING gist (production_id WITH =, person_id WITH =, character_id WITH =, capacity WITH =,
                        COALESCE(variant, '') WITH =,
                        daterange(valid_from, valid_until, '[]') WITH &&) WHERE (retired_at IS NULL)
);
CREATE INDEX ON backlot.cast_assignment (production_id, character_id);
CREATE INDEX ON backlot.cast_assignment (person_id);

CREATE TABLE backlot.cast_engagement (                    -- szerződéses metaadat: személyenként produkciónként egy
    production_id uuid NOT NULL REFERENCES backlot.production(id),
    person_id uuid NOT NULL REFERENCES backlot.person(id),
    contract_document_id uuid REFERENCES backlot.document(id),    -- trigger: contract típus, ugyanaz a produkció
    contracted_days integer CHECK (contracted_days >= 0),
    option_days integer CHECK (option_days >= 0),
    currency char(3),                                      -- ISO 4217 (#15); az összeg a 24. lépcsőben
    ...közös oszlopok,
    PRIMARY KEY (production_id, person_id)
);

CREATE TABLE backlot.cast_call (                          -- a szereplő behívása egy forgatási napra
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    shooting_day_id uuid NOT NULL REFERENCES backlot.shooting_day(id),
    person_id uuid NOT NULL REFERENCES backlot.person(id),
    character_id uuid REFERENCES backlot.production_entity(id),   -- melyik karakterként (opcionális)
    source text NOT NULL DEFAULT 'manual' CHECK (source IN ('manual','breakdown')),   -- 'breakdown': a 22. lépcső tölti
    note text CHECK (length(note) <= 500),
    retired_at timestamptz,
    ...közös oszlopok,
    UNIQUE (shooting_day_id, person_id, character_id)
);
```

- **Szereposztás:** egy karakternek **több szereplője** lehet (gyermek- és felnőttkori változat, dublőr, kaszkadőr-dublőr);
  egy szereplő **több karaktert** is játszhat. Ugyanarra a (személy, karakter, szerepkör, változat) négyesre az
  érvényességi időszakok nem fedhetik át egymást (kizáró korlát, `CAST_ASSIGNMENT_OVERLAP`). A szerepkör nem
  helyettesíti a szereplő típusát (`kind`): egy `actor` lehet egy karakter `principal` szereplője, egy `stunt` ugyanannak
  a `stunt_double`-je.
- **Szerződéses metaadat:** `cast_engagement` személyenként és produkciónként egy, mert a szerződés a személyre szól,
  nem a karakterre. **Összeg nincs benne** (G1: a Budgethez kapcsolódik, szűkített eléréssel); a `currency` csak a későbbi
  összegek pénznemét rögzíti (`CURRENCY_UNKNOWN`). A szerződés maga a G2-14 dokumentum (jóváhagyási és aláírási
  állapottal, lejárattal); a hivatkozás ellenőrzi a típust és a produkciót.
- **Behívás** (`cast_call`): a forgatási nap (G2-17) és a szereplő kapcsolata; a 22. lépcső (Script Breakdown) a
  jelenet–karakter kapcsolatokból javasolja és `source = 'breakdown'`-nal tölti, kézzel is felvehető. A 23. lépcső
  ezt bővíti (hívási idők, jelmez/smink idő), az azonosítók változatlanok (mint a `shooting_day`). **A jelenet és a
  forgatási nap kapcsolata a 23. lépcsőben jön létre**, ezért itt a behívás **nem** jelenetből számolódik: a G1
  „ebből számolható” megfogalmazása a 22.–23. lépcsőre vonatkozik, a 19. lépcsőben a kézi behívás a teljes funkció, a
  jelenet–karakter kapcsolat pedig a „melyik jelenetben szerepel” nézetet adja.

### 4.3 Ütközések és számolt mutatók

Számolt jelzések **lekérdezéskor** (nincs tárolt jelző), a naptárral és a G2-15-tel azonos elven; **nem tiltanak**:

| Jelzés | Feltétel | Súly |
|---|---|---|
| `call_on_leave` | A behívott szereplő aznap szabadságon van (G2-15 erőforrás-naptár; csak ha van `resource`-sora) | figyelmeztetés |
| `call_outside_engagement` | A behíváshoz nincs a napon érvényes szereposztás (`cast_assignment`), vagy a karakter nem a szereplőé | figyelmeztetés |
| `over_contracted_days` | A szereplő behívott napjainak száma (különböző forgatási nap, nem visszavont) meghaladja a `contracted_days`-et | figyelmeztetés |
| `over_option_days` | Meghaladja a `contracted_days + option_days`-et | hiba |
| `no_contract` | Behívott szereplőhöz nincs `cast_engagement` vagy szerződés-hivatkozás | információ |
| `minor_called` | Kiskorú (`is_minor`) behívása: a felhasználó figyelmét hívja fel a helyi munkaidő-szabályra (szabályt nem értékel) | információ |
| `character_unassigned` | Egy karakternek, amely szerepel egy jelenetben (`casting`), nincs `principal` szereplője | információ |

- **A behívott napok száma** (`called_days`) számolt: a különböző `shooting_day`-ek száma a személyre, produkciónként; a
  `contracted_days` és `option_days` mellett a felület „felhasznált/szerződött/opciós” mutatót ad. **Nem számolunk**
  a forgatási napon belüli órákat (az a 23. lépcső).
- Az **ütközés nem tilt**: a felhasználó menthet túllépett behívást is (a szerződés bővíthető), a rendszer jelez.

### 4.4 A jelenetkapcsolat nézete

- **Karakter → jelenetek:** a `casting` kapcsolatok a karakterre; **jelenet → karakterek és szereplők:** a kapcsolatok
  és a szereposztás összekapcsolva; **szereplő → jelenetek:** a szereplő karaktereiből. Mind lekérdezés, nincs tárolt
  származtatott adat (a G2-03 elve).
- Az `entity_link` `casting` kapcsolat a G2-03 szerinti **részfa-hatásvizsgálatban** is szerepel (a karakter módosítása
  a jeleneteit érinti); ezt a G2-03 már kezeli.

## 5. Hozzáférés és személyes adat (SEC-06)

| Adat | Ki látja |
|---|---|
| Karakter, cast ID, szerepkör, szereposztás (ki játssza), szereplő megjelenített neve, típus, kiskorú-jelzés | A produkció minden tagja |
| `cast_call` (ki mikor van behíva) | A produkció minden tagja |
| `contact` szint: elérhetőség (G2-18) és az ügynök adata (`cast_personal_detail`) | `owner`, `producer`, `coordinator`, és a személy maga |
| `identity`, `financial`, `emergency` szint (jogi név, születési dátum, cím, …) | `owner`, `producer` és a személy maga |
| `cast_engagement` (szerződött és opciós napok, szerződés-hivatkozás, pénznem) | `owner`, `producer` (szűkített, G1) |

- A **kiskorú-jelzés** a produkció minden tagjának látható (az ütemezéshez kell), de a **születési dátum** nem (az
  `identity` szinten). Gyerekszereplőnél a szülő/gondviselő adata a `emergency_contact`-ba kerül, nem külön táblába.
- A szabályok a lekérdezésben **már most** érvényesek (M2-re kész, mint a stábnál); a megtekintés auditált; a személyes
  részletek **nincsenek indexelve**.
- **Törlés, anonimizálás:** a `backlot.person.anonymize` (G2-18 5.3) a szereplőre is érvényes: a közös részlet, a
  `cast_personal_detail` és a `cast_profile.note` törlődik, a művésznév semleges jelölővé válik, az `audit.personal.erase`
  fut. **Megmarad** a szereposztás szerkezete (melyik karaktert ki játszotta, nem azonosítható módon), a behívott
  napok száma és a szerződött napok (elszámolási adat). A szerződés-dokumentum anonimizálását a G2-14 parancsa
  végzi, ha a felhasználó kéri; a hivatkozásjelentés (G2-18) kiírja.
- **Gyermekek adatai** fokozott védelmet érdemelnek; a rendszer ezt a szint-szabályokon (születési dátum és
  gondviselői kapcsolat csak `owner`/`producer`) és a megtekintés auditján túl **nem külön technikai
  eszközzel** kezeli (jogi kérdés, G2-18 5.3/4).

## 6. Naptárréteg

A G2-17 szerint a réteg regisztrálása a modulhoz kötött: a Cast megvalósításakor a naptár egy új réteget kap.

| Réteg | Forrás | Elem | Szerkesztés |
|---|---|---|---|
| `cast_calls` | `cast_call` | egész napos jelölés a forgatási napon és a szereplő idővonalán | `backlot.cast.call.add` / `.remove` |

A réteg csak azt adja, amit a felhasználó látna (az 5. pont táblája szerint, a szolgáltatóban szűrve). Az
`call_on_leave` és az `over_*` jelzés a nézetben és a `backlot.cast.conflicts` listában látszik; aszinkron
értesítés nincs M1-ben (mint a G2-17 4.4-ben).

## 7. Export

- **Szereposztási lista** (`backlot.cast.export`): CSV (UTF-8, RFC 4180) és PDF (`QPdfWriter`, mint a G2-13). Alapból:
  cast ID, karakter, szereplő (megjelenített név), típus, szerepkör, változat. **Opcionális, kifejezett választásra és csak
  a jogosultnak:** behívott/szerződött napok (`cast_engagement` szint), `contact` szint.
- Az export **auditált** (`cast.exported`, a kiválasztott csoportokkal és darabszámmal, tartalom nélkül), a
  hozzáférési szabállyal szűrt, a fájlt a felhasználó által megadott helyre írja; személyes adat nem hagyja el a gépet
  (SEC-02).

## 8. Parancsok és hibakódok

| Parancs | Mit csinál |
|---|---|
| `backlot.cast.character.create` / `.update` / `.retire` / `.renumber` / `.list` / `.get` | Karakter (entitás + profil), cast ID |
| `backlot.cast.member.create` / `.update` / `.retire` / `.restore` / `.list` / `.get` | Szereplő (új vagy meglévő `person` alapján) |
| `backlot.cast.personal.get` / `.set` | Az ügynök adata (`contact` szint); a többi `backlot.person.personal.*` (G2-18) |
| `backlot.cast.assign` / `.update_assignment` / `.end_assignment` / `.retire_assignment` / `.list` | Szereposztás |
| `backlot.cast.link.add` / `.remove` / `.list` | Jelenet–karakter (`casting`) kapcsolat (3.1) |
| `backlot.cast.engagement.set` / `.get` | Szerződéses metaadat (`owner`, `producer`) |
| `backlot.cast.call.add` / `.remove` / `.list` | Behívás a forgatási napra |
| `backlot.cast.conflicts` | Ütközések és mutatók (4.3) |
| `backlot.cast.enable_planning` | Erőforrás létrehozása a szereplőhöz (G2-15) |
| `backlot.cast.export` | Szereposztási lista |

Hibakódok: `CAST_MEMBER_NOT_FOUND`, `CAST_MEMBER_EXISTS`, `CAST_NO_TAKEN`, `CAST_ASSIGNMENT_OVERLAP`,
`CAST_LINK_INVALID`, `CAST_CHARACTER_INVALID` (az elem nem `character`), `CAST_ENGAGEMENT_INVALID` (a dokumentum
nem szerződés vagy más produkció), `CAST_CALL_EXISTS`, `CAST_EXPORT_FAILED`, valamint a `PERSON_*`, `ENTITY_*`,
`DOCUMENT_*`, `CURRENCY_UNKNOWN`, `AUDIT_*` kódok változatlanul.

## 9. Never crash

- Hibás vagy hiányos adat (nem karakter elemre szereposztás, más produkció dokumentuma, átfedő időszak, ismeretlen
  pénznem, ismétlődő cast ID): angol hibakód, nincs írás, nincs összeomlás.
- A karakter visszavonása szereposztással vagy behívással: a szereposztás megmarad, a felület jelzi a visszavont
  karaktert; nincs árva sor.
- Az anonimizálás és az export hibája nem hagy félig kész állapotot (G2-18 8. pont).

## 10. Sorrend és függés

| Rész | Kell hozzá |
|---|---|
| Karakter, szereplő, szereposztás, jelenetkapcsolat, export | 3., 5. lépcső; a közös személyes adat és az anonimizálás a 18. lépcsőből |
| Szerződés-hivatkozás | 14. lépcső |
| Behívás és forgatási napok, naptárréteg | 17. lépcső (`shooting_day`, rétegek) |
| Elérhetőség, szabadság-ütközés | 15. lépcső |
| Automatikus behívás-javaslat | 22., 23. lépcső |

## 11. Tesztek (G5-re előre)

- **Cast ID:** automatikus kiosztás párhuzamos karakterlétrehozásnál (egyedi, folytonos), kézi megadás, átszámozás egy
  tranzakcióban (a halasztott korlát), ütközés `CAST_NO_TAKEN`; a szám a karakteré marad szereplőcserénél.
- **Szereposztás:** több szereplő egy karakterre, több karakter egy szereplőre, átfedő időszak elutasítása,
  eltérő szerepkör/változat engedett; nem karakter és más produkció elutasítva.
- **Jelenetkapcsolat:** `casting` kapcsolat irányai és típusai (`scene`/`shot`/`sequence` → `character`), kettős
  kapcsolat elutasítva; a három nézet (karakter→jelenetek, jelenet→karakterek és szereplők, szereplő→jelenetek)
  egyezik a nyers kapcsolatokkal (property-teszt).
- **Ütközések:** minden jelzés (szabadság, szerződésen kívüli behívás, túllépett napok, opció, szerződés hiánya,
  kiskorú, hozzá nem rendelt karakter); az ütközés nem tiltja a mentést; a `called_days` különböző napokat számol.
- **Hozzáférés:** minden adatcsoport minden szerepre; a nem jogosult nem látja az engagement-et, az ügynök adatát,
  a születési dátumot; a kiskorú-jelzés mindenkinek látszik, a születési dátum nem; a személyes részletek nincsenek a
  keresésben.
- **Anonimizálás:** a szereplő személyes adata sehol sincs az anonimizálás után (a G2-18 teljességi vizsgálata a
  `cast_personal_detail`-re is), a szereposztás szerkezete és a napszámok megmaradnak, `audit.verify` sikeres.
- **Export:** csak a kért csoportok, a nem jogosult nem kap szűkített oszlopot, hiba nem hagy félig írt fájlt.
- **Naptárréteg:** a `cast_calls` réteg a forrásból számol, a szerkesztés a `backlot.cast.call.*`-ot hívja.
- **Never crash:** hibás bemenetek fuzzolása, visszavont karakter szereposztással.
- **Felületi ellenőrzés:** `ui.screenshot` a szereposztási listáról, a karakternézetről és az ütközésjelzésről.
- **Mérés** a CFA4-MIN gépen: 500 karakter, 2000 szereplő, 10 000 behívás, lista és ütközésszámítás ideje.

## 12. Licenc (G3-ra előre)

Új függőség nincs (`btree_gist`: contrib, G2-18; PDF: `QPdfWriter`). Kitsu/Zou (AGPL-3.0: „casting”, karakter–jelenet
kötés) referencia, kód nélkül; a StudioBinder, Celtx és Movie Magic nyilvános dokumentációja funkcionális
referencia (cast ID, szerződéses napok).

## 13. G2-eredmény

**Javaslat: PASS.** Új döntési kérdés nincs. A megvalósítás a 3., 5. és 18. lépcső után indulhat; a behívás és a
naptárréteg a 17., a szerződés-hivatkozás a 14. lépcsőtől működik. A `casting` kapcsolat iránya (3.1) a G2-03
kiegészítése, új tábla nélkül.
