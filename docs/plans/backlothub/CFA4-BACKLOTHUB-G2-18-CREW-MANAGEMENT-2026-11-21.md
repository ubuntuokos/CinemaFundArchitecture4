# BacklotHub 18. lépcső – Crew Management: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-21 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.17 · **NFR:** SEC-06 (GDPR) · **Előzmény:**
[G1 Crew Management](CFA4-BACKLOTHUB-G1-18-CREW-MANAGEMENT-2026-11-07.md) (funkcionális követelmények
jóváhagyva 2026-11-10) · **Épül rá:** G2-01 (Audit, `personal_detail`), G2-02 (`production_member`, szervezet),
G2-05 (közös `person`), G2-15 (erőforrás, részleg/csapat), G2-17 (naptár) · **Állapot:** G2-javaslat,
jóváhagyásra vár · implementáció nincs

Új döntési kérdés nincs, új függőség nincs (a PostgreSQL contrib `btree_gist` kiterjesztés kell, mint a korábbi
lépcsők `ltree`-je és `pg_trgm`-je). **Három pontosítás kell korábbi, még meg nem valósított G2-ékben**
(2. pont): a G2-01 audit-kivonata sózott lesz, az `audit.personal.erase` parancs bővíti a G2-01-et, és a
`person.full_name` a G2-05-ből ide kerül. Ezek a törlési kérés (SEC-06) teljesíthetőségéhez kellenek.

## 1. Hatókör ebben a lépcsőben

- **Stábtagok** (a közös `person`-re épülve), **részlegek** és **munkakörök**, **beosztás** produkciónként
  (munkakör, időszak, felelősségi kör, szervezeti kapcsolat, hierarchia).
- **Személyes adatok elkülönítése** (SEC-06): külön tábla, szintenként szűkített elérés, törlés/anonimizálás
  az auditlánc épségének megőrzésével.
- **Stáblista export** (CSV, PDF).
- **Nem része:** bejelentkezés és jogosultság (a `production_member` marad, Identity M2-ben), szerződéskezelés
  és bérszámfejtés (a szerződés dokumentum, G2-14), szabadság-jóváhagyási folyamat (jelen lépcsőben nincs, lásd
  3.4), a szereplők (19.) és az ő személyes adataik (a 19. G2-je ugyanezt a táblát használja).

## 2. A korábbi G2-ék pontosításai

| G2 | Pontosítás | Miért |
|---|---|---|
| **G2-01 Audit** | A redaktált érték `digest`-je **sózott** (`SHA-256(salt ‖ érték)`, a só a `personal_detail` sorában). Új `audit.personal.erase` parancs és `cfa4_audit.erase_personal()` függvény | Sózatlan kivonat alacsony entrópiájú adatnál (telefonszám, születési dátum, IBAN) szótárral visszafejthető, és a láncban örökre ott marad; így a törlési kérés nem lenne teljesíthető. A módosítás **már a G2-01 szövegében szerepel** (2026-11-21), megvalósítás még nincs |
| **G2-05 Task (`person`)** | A `person.full_name` oszlop **nincs**; a teljes név a `person_personal_detail`-ben. A `person.anonymized_at` új. A `display_name` marad (megjelenített név vagy álnév) | Egy helyen legyen a személyes adat (#53 elve); a G2-05 szövege már **átvezetve** |
| **G2-15 Resource** | A `team` táblának `label_key` oszlopa (beépített részlegek fordítása) | A részlegek katalógusa a `team` (4.2), nem új tábla; **átvezetve** |

## 3. Integrációs határok

| Határ | Irány | Szerződés |
|---|---|---|
| Person (5., #53) | ← | A közös `person`: azonosító, `display_name`, `os_user_ref`, `retired_at`; a 18. lépcső ezt **bővíti** (4. pont), nem cseréli. A feladatkiosztás, az időbejegyzés és az esemény résztvevője már a `person_id`-re mutat, **nincs migráció** |
| Resource (15.) | ↔ | A részleg a `team` (`kind = 'department'`); az erőforrás a `person`-höz kötött sor. A stábtag tervezhetővé tétele a `backlot.crew.enable_planning` paranccsal (a `backlot.resource.create`-et hívja), automatikusan nem |
| Production (2.) | ← | `production_member` = **jogosultsági** tagság; a `crew_assignment` = **munkaügyi** beosztás. A kettő **külön marad** (G1): egy stábtagnak nem kell bejelentkeznie, és egy felhasználó nem feltétlenül stábtag |
| Organization (2.) | ← | A beosztás `organization_id`-ja (munkáltató, ügynökség, vállalkozó) a közös szervezetre mutat |
| Document (14.) | ← | A szerződés dokumentum; a dokumentum `person_ref` metaadata és kapcsolata a `person`-re mutat |
| Audit (1.) | → | Minden írás auditált, a személyes mezők redaktálva (sózott kivonat); a megtekintés auditált (6. pont) |
| Search (7.) | → | `SearchProvider`: stábtag (megjelenített név, munkakör, részleg, felelősségi kör). **A személyes részletek nincsenek indexelve** |
| Calendar (17.), Time (16.) | – | A `person` az összekötő; a naptár és az időlap a személyes részletet nem olvassa |

## 4. Adatmodell (DDL-vázlat, `backlot` séma)

### 4.1 Stábtag és személyes adat

```sql
-- A backlot.person a G2-05-ben él (megjelenített név, os_user_ref, retired_at, anonymized_at).

CREATE TABLE backlot.crew_profile (                     -- „ez a személy stábtag” + alapértelmezések
    person_id uuid PRIMARY KEY REFERENCES backlot.person(id) ON DELETE RESTRICT,
    default_department_id uuid REFERENCES backlot.team(id),      -- kind = 'department' (trigger)
    default_job_title_id  uuid REFERENCES backlot.job_title(id),
    note text CHECK (length(note) <= 2000),                       -- NEM személyes jegyzet; személyest a részletbe
    retired_at timestamptz,
    ...közös oszlopok production_id nélkül
);

CREATE TABLE backlot.person_personal_detail (            -- közös: a Cast (19.) is ezt használja
    person_id uuid PRIMARY KEY REFERENCES backlot.person(id) ON DELETE RESTRICT,
    legal_name text,
    email text, phone text,
    address jsonb,                                       -- {street, city, postal_code, country}
    birth_date date,
    tax_no text, bank_account text,
    emergency_contact jsonb,                             -- {name, phone, relation}
    updated_at timestamptz NOT NULL, updated_by uuid NOT NULL REFERENCES backlot.person(id),
    row_version integer NOT NULL
);
```

- **Kötelező mező csak a megjelenített név** (a `person.display_name`); minden más opcionális
  (adatminimalizálás, G1). A felület csak azt kérdezi, ami a feladathoz kell.
- **Szintek (mezőcsoportok)**, mert az elérés csoportonként más (5. pont): `contact` (`email`, `phone`),
  `identity` (`legal_name`, `birth_date`, `address`), `financial` (`tax_no`, `bank_account`), `emergency`
  (`emergency_contact`). Csoportonként külön lekérdezhető és külön szerkeszthető.
- **Ellenőrzés** saját kóddal, függőség nélkül: e-mail (egyszerű alak, hossz), telefon (számjegyek, `+`, szóköz,
  kötőjel; nemzetközi formátumra normalizálható, de szabad szöveg is elfogadott), `bank_account` (IBAN esetén
  mod 97 ellenőrzőösszeg, `PERSON_DETAIL_INVALID`; nem IBAN-alakú szám a felhasználó megerősítésével menthető),
  `birth_date` (múltbeli, ésszerű tartomány), `tax_no` (szabad szöveg, országonként eltér, nem validáljuk).
- **Titkosítás:** az adatbázisban a mezők **nyílt szövegek**; az M1 védelme a fájlrendszer-szintű titkosítás
  (kapcsolható, ADR 0006/NFR) és az elérésszabály. **Mezőszintű titkosítás nincs M1-ben** (kulcskezelés kellene,
  amely nincs); ezt a felület az adatbeállításoknál kimondja. A banki és adószám a felületen **maszkolt** (pl. az utolsó
  négy számjegy), a megjelenítés kérésre történik.
- **Azonosító:** a `person.id` az álneves hivatkozás minden láncolt és naplózott adatban (az auditba a név nem kerül,
  G2-05).

### 4.2 Részleg, munkakör, beosztás

```sql
-- Részleg = backlot.team, kind = 'department' (G2-15); a katalógus ide nem kap második táblát.
-- A 18. lépcső migrációja a szokásos filmes részlegeket beépített sorként (label_key-vel) szállítja:
-- direction, production, camera, lighting, grip, sound, art, costume, makeup, editing, vfx, animation.

CREATE TABLE backlot.job_title (
    id uuid PRIMARY KEY, code text NOT NULL, name text, label_key text,
    department_id uuid REFERENCES backlot.team(id),                -- alapértelmezett részleg (kind = 'department')
    ordinal smallint NOT NULL,
    production_id uuid REFERENCES backlot.production(id),         -- NULL: globális katalógus
    retired_at timestamptz,
    ...közös oszlopok production_id nélkül,
    UNIQUE (code, production_id),
    CHECK (name IS NOT NULL OR label_key IS NOT NULL)
);

CREATE TABLE backlot.crew_assignment (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    person_id uuid NOT NULL REFERENCES backlot.person(id),
    job_title_id uuid NOT NULL REFERENCES backlot.job_title(id),
    department_id uuid NOT NULL REFERENCES backlot.team(id),       -- kind = 'department' (trigger)
    responsibility text CHECK (length(responsibility) <= 200),     -- pl. „2. egység operatőre”
    organization_id uuid REFERENCES backlot.organization(id),      -- munkáltató, ügynökség, vállalkozó
    reports_to_id uuid REFERENCES backlot.crew_assignment(id),     -- hierarchia a produkción belül
    valid_from date NOT NULL, valid_until date,                    -- NULL = nyitott
    retired_at timestamptz,
    ...közös oszlopok,
    CHECK (valid_until IS NULL OR valid_until >= valid_from),
    EXCLUDE USING gist (production_id WITH =, person_id WITH =, job_title_id WITH =,
                        daterange(valid_from, valid_until, '[]') WITH &&) WHERE (retired_at IS NULL)
);
CREATE INDEX ON backlot.crew_assignment (production_id, department_id);
CREATE INDEX ON backlot.crew_assignment (person_id);
```

- **Egy stábtag több produkcióban és több munkakörben is lehet** (G1); ugyanabban a produkcióban egyszerre több
  munkakörben is, de **ugyanarra a (produkció, személy, munkakör) hármasra az időszakok nem fedhetik át egymást**
  (kizáró korlát, `CREW_ASSIGNMENT_OVERLAP`).
- **Hierarchia:** a `reports_to_id` ugyanabban a produkcióban lévő beosztásra mutat; **körmentességet** trigger
  őrzi (`CREW_REPORTS_TO_CYCLE`), legfeljebb 10 szint mélységig (`CREW_HIERARCHY_TOO_DEEP`). Az időszak
  nem kell egyezzen; a felület jelzi, ha a főnök beosztása már véget ért.
- **A részleg és a munkakör katalógusa konfigurálható**: a beépített elemek a migrációban, a sajátok a produkció
  vagy a globális szinten; a beépítetteket nem lehet törölni, csak visszavonni (`retired_at`). A `department_id` a
  beosztáson a munkakör alapértelmezett részlegét örökli, de felülírható (egy kamera-munkakör a világítás részlegen is
  előfordulhat).
- **A produkciós kapcsolatok** (G1) a `reports_to_id` (belső hierarchia) és az `organization_id` (külső kötődés,
  pl. vállalkozó cég) együttese; a `person` és a szervezet közötti tartós kapcsolat a G2-02 szervezetmodelljére
  épül, új tábla nem kell.

## 5. Hozzáférés és személyes adat (SEC-06)

### 5.1 Szintek

| Mezőcsoport | Ki látja és szerkeszti |
|---|---|
| Név, munkakör, részleg, felelősség, hierarchia (nem személyes részlet) | A produkció minden tagja (`production_member`, #42) |
| `contact` | `owner`, `producer`, `coordinator`, és a személy maga |
| `identity`, `financial`, `emergency` | `owner`, `producer` és a személy maga |

- A **személy maga** csak akkor, ha a `person.os_user_ref` az aktuális felhasználóra mutat (M1: a helyi
  felhasználó); a szolgáltatásréteg és a lekérdezések ezt **már most** alkalmazzák, hogy M2-ben ne kelljen átírni.
- **M1-ben a helyi felhasználó tulajdonos, mindent láthat**, de a felület alapból **rejti** a személyes részleteket és
  kattintásra mutatja (G1). **A megjelenítés (a `get` hívás) auditált** (`crew.personal.viewed`, az azonosítóval és a
  csoporttal, **érték nélkül**), mert ez az elszámoltathatóság része; az esemény a felhasználóhoz kötött, nem
  tömeges (a lista nézet a csoportokat nem tölti be).
- **Nem indexelt:** a személyes részletek nincsenek a keresésben, a `search_vector`-ban, az értesítések
  payloadjában és az események címében.

### 5.2 Naplózás

- A személyes mezők módosítása auditált, az érték helyén **redaktálva**: `{"redacted": true, "digest": …}`, a
  kivonat a **sózott** forma (2. pont), a tényleges (régi és új) érték és a só a `cfa4_audit.personal_detail`
  sorában, a törlésig.
- Az auditeseményben a tárgy `subject_type = 'person'`, `subject_id = person.id`, így az `audit.personal.erase`
  egy lépésben megtalálja az összes érintett sort.

### 5.3 Törlés és anonimizálás

**`backlot.person.anonymize`** (csak `owner`, kifejezett megerősítéssel, két lépésben: előbb a hivatkozásjelentés,
aztán a végrehajtás):

1. **Hivatkozásjelentés** (`backlot.person.references {person}`): minden modul regisztrál egy hivatkozás-szolgáltatót
   (zárt, kódban regisztrált lista; a megvalósított modulok jelennek meg, nincs helyettesítő). Eredmény: darabszám
   és lista modulonként (feladatkiosztás, időbejegyzés, dokumentum `person_ref` és aláíró, naptári résztvevő,
   beosztás, megjegyzések szerzőként, értesítések). Ebből látszik, hol maradhat még a személyre utaló adat; a szabad
   szöveget (megjegyzés törzse, feladatcím) a rendszer **nem írja át**, azt a felhasználó dönti el.
2. **Végrehajtás** egy tranzakcióban:
   - a `person_personal_detail` sor **törlődik**;
   - a `person.display_name` **semleges jelölővé** válik (a felület a kulcsot fordítja: „Anonimizált személy”, a
     rövid azonosítóval), `anonymized_at` beáll, az `os_user_ref` törlődik;
   - a `crew_profile.note` törlődik (szabad szöveg);
   - minden modul **törlő kezelőt** regisztrál (a hivatkozás-szolgáltatók mintájára, zárt, kódban regisztrált lista;
     a megvalósított modulok jelennek meg): a G2-14 dokumentumok személyes metaadat- és aláíróneve (a
     `document.anonymize` útján, ha a felhasználó kéri), a G2-16 időbejegyzés `note`-jai, a G2-19 `cast_personal_detail`
     és `cast_profile.note`;
   - az **`audit.personal.erase`** törli a személy `personal_detail` sorait és sóit az auditból (5.2).
3. **Ami megmarad, és miért:** a `person.id`, a beosztások (munkakör, időszak), a feladatkiosztás, az időbejegyzések
   összegei és a költségnézethez kellő adatok, mert ezek elszámolási és történeti adatok; a személy már **nem
   azonosítható** belőlük. Az auditlánc ép marad (`audit.verify` sikeres), a sózott kivonatok a só törlése után
   nem fejthetők vissza.
4. **Jogi kérdés:** hogy egy konkrét kérésnél mit kell törölni és mit szabad megőrizni (pl. bérszámfejtési, adóügyi
   megőrzési kötelezettség), jogi döntés, nem a rendszeré. A rendszer a **két lehetőséget** adja: teljes
   anonimizálás, vagy **mezőszintű törlés** (`backlot.person.personal.set` üres értékkel, csoportonként); a
   felület a jelentésben kiírja, mi marad.
- Az **anonimizálás nem visszavonható** (a törölt értékeknek nincs másolata a rendszerben); a mentésekben (`pg_dump`,
  archívum) lévő példányok a mentés életciklusa szerint járnak le, ezt a felület kimondja (az archívum megőrzési
  ideje a felhasználó döntése, #18).

## 6. Export

- **Stáblista** (`backlot.crew.export`): CSV (UTF-8, RFC 4180) és PDF (`QPdfWriter`, mint a G2-13), alapból név
  (megjelenített), részleg, munkakör, felelősség. A **`contact` és a többi csoport csak kifejezett kiválasztással**
  kerül bele (G1), és csak a jogosultnak (5.1).
- Az export **auditált** (`crew.exported`, a kiválasztott csoportokkal és darabszámmal, tartalom nélkül), a fájlt a felhasználó
  által megadott helyre írja. Személyes adat nem hagyja el a gépet (SEC-02).
- Az export a **hozzáférési szabállyal szűrt**: a nem jogosult nem kap olyan oszlopot, amelyhez nincs joga.

## 7. Parancsok és hibakódok

| Parancs | Mit csinál |
|---|---|
| `backlot.crew.member.create` / `.update` / `.retire` / `.restore` / `.list` / `.get` | Stábtag (új vagy meglévő `person` alapján, ez utóbbi nem hoz létre kettős sort) |
| `backlot.person.personal.get` / `.set` | Személyes részletek csoportonként, **közös** a stáb és a szereplők (19.) felületének; a `get` auditált |
| `backlot.crew.job_title.*` | Munkakörök katalógusa |
| `backlot.crew.assign` / `.update_assignment` / `.end_assignment` / `.retire_assignment` / `.list` | Beosztás |
| `backlot.crew.org_chart` | A produkció hierarchiája (a `reports_to_id`-ból), részleg szerint csoportosítva |
| `backlot.crew.enable_planning` | Erőforrás létrehozása a stábtaghoz (G2-15) |
| `backlot.crew.export` | Stáblista (6. pont) |
| `backlot.person.references` / `.anonymize` | Hivatkozásjelentés és anonimizálás (5.3) |
| `backlot.team.*` | A részlegek a G2-15 `team` parancsaival (`kind = 'department'`) |

Hibakódok: `CREW_MEMBER_NOT_FOUND`, `CREW_MEMBER_EXISTS`, `CREW_ASSIGNMENT_OVERLAP`, `CREW_REPORTS_TO_CYCLE`,
`CREW_HIERARCHY_TOO_DEEP`, `CREW_DEPARTMENT_INVALID` (nem `department` fajtájú `team`), `PERSON_DETAIL_INVALID`,
`PERSON_DETAIL_FORBIDDEN` (a felület ugyanúgy „nincs jogosultság” üzenetet ad, a mező létezését nem árulja el),
`PERSON_ANONYMIZE_NEEDS_CONFIRMATION`, `PERSON_ALREADY_ANONYMIZED`, `CREW_EXPORT_FAILED`, valamint az `AUDIT_*` és
`PERSON_IN_USE` (G2-05) kódok változatlanul.

## 8. Never crash

- Hibás vagy hiányos adat (hibás IBAN, ismeretlen részleg, kör a hierarchiában, átfedő beosztás): angol hibakód, nincs
  írás, nincs összeomlás.
- Az anonimizálás **egy tranzakció**: bármilyen hiba esetén a személyes részlet és az audit-személyesadat is
  változatlan; részleges törlés nincs (az `audit.personal.erase` a végrehajtó tranzakcióban fut, a lánctáblához nem
  nyúl).
- Az `audit.verify` anonimizálás után ép láncot ad; a törölt személyesadat-sorok ellenőrzését átugorja és jelzi,
  nem hibázik.
- Személyes adat sosem kerül hibaüzenetbe, naplóba vagy az `unhandled` jelentésbe (OPS-04: a diagnosztikai
  csomag személyes adat nélkül).

## 9. Sorrend és függés

| Rész | Kell hozzá |
|---|---|
| Stábtag, munkakör, beosztás, hierarchia, export | 2., 5. lépcső; a részleg a G2-15 `team`-je (15. lépcső); a G2-01 sózott kivonata és az `audit.personal.erase` az 1. lépcsőben |
| Tervezhetővé tétel | 15. lépcső |
| Hivatkozásjelentés teljessége | A modulok maguk regisztrálják a szolgáltatójukat, ahogy megvalósulnak (14., 16., 17., …) |
| Cast (19.) | A `person_personal_detail` és az anonimizálás közös; a 19. G2-je ezt bővíti |

## 10. Tesztek (G5-re előre)

- **Sózott kivonat:** ugyanaz az érték két sorban **különböző** kivonatot ad; szótárból a kivonat a só törlése után nem
  fejthető vissza (próba: ismert telefonszám-tartomány kivonatai az anonimizálás után nem egyeznek semmivel); a
  kivonat a törlés előtt ellenőrizhető.
- **Anonimizálás teljessége:** az anonimizálás után a személyes érték **sehol** nincs az adatbázisban és a
  keresési vektorban sem (a `person_personal_detail`, a `crew_profile.note`, az `audit.personal_detail`, a
  generált `tsvector`-ok szövegmintás átvizsgálása az ismert értékekre); `audit.verify` sikeres; a beosztások, a
  feladatkiosztás és az időösszegek megmaradnak.
- **Egy tranzakció:** hibainjektálás az anonimizálás minden lépése között: semmi nem törlődik félig.
- **Hozzáférés:** minden mezőcsoport minden szerepre (owner, producer, coordinator, artist, viewer, a személy maga,
  idegen); a nem jogosult sem a részletet, sem a keresést, sem az exportot nem látja; a `get` auditált,
  a lista nem tölti be a csoportokat.
- **Ellenőrzők:** IBAN ellenőrzőösszeg (érvényes, hibás, nem IBAN-alak megerősítéssel), e-mail, telefon, jövőbeli
  születési dátum.
- **Beosztás:** átfedés (kizáró korlát), azonos munkakör eltérő időszakban, több munkakör egyszerre; hierarchia-kör,
  túl mély hierarchia, másik produkció beosztására mutató `reports_to_id` elutasítva; részleg `kind` ellenőrzése.
- **Katalógus:** a beépített részlegek és munkakörök fordítása, a sajátok, a visszavonás utáni újrahasználat.
- **Hivatkozásjelentés:** minden regisztrált modul megjelenik, a nem megvalósított nem; a szabad szöveg nem íródik át.
- **Export:** csak a kért csoportok, a nem jogosult nem kap személyes oszlopot, a hiba nem hagy félig írt fájlt.
- **Never crash:** hibás bemenetek fuzzolása.
- **Felületi ellenőrzés:** `ui.screenshot` a stáblistáról, a szervezeti nézetről és a rejtett/megjelenített személyes
  részletekről.
- **Mérés** a CFA4-MIN gépen: 5000 stábtag, 20 000 beosztás, lista, keresés és szervezeti nézet felépítésének ideje.

## 11. Licenc (G3-ra előre)

- **Új függőség nincs.** A `btree_gist` a PostgreSQL contrib része (mint az `ltree`, `unaccent`, `pg_trgm`); a
  G3-on ellenőrizendő, hogy az Ubuntu `postgresql-contrib` csomagban a kiválasztott verzión elérhető.
- Az ellenőrzők (IBAN, e-mail, telefon) saját kód, a PDF `QPdfWriter` (PySide6, mint a G2-13).
- **Referencia, kód nélkül:** Kitsu/Zou (AGPL-3.0: személy, részleg; **ellenpélda** az összemosott munkaügyi és
  bejelentkezési adatra), Stalker (LGPL-3.0: részleg, felhasználó). A bejelentkezési adat itt szándékosan nincs.

## 12. G2-eredmény

**Javaslat: PASS.** Új döntési kérdés nincs. A megvalósítás a 2. és az 5. lépcső után indulhat, a G2-01 pontosított
(sózott kivonat, `audit.personal.erase`) megvalósításával együtt: ez az, ami nélkül a törlési kérés nem teljesíthető,
ezért **a G2-01 kivonat-pontosítása a G2-18 előfeltétele**. A G2-05 (`person`) és a G2-15 (`team.label_key`)
módosítása már átvezetve.
