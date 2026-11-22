# BacklotHub 24. lépcső – Budget Tracking: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-21 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.21 · **Előzmény:**
[G1 Budget Tracking](CFA4-BACKLOTHUB-G1-24-BUDGET-TRACKING-2026-11-07.md) (funkcionális követelmények jóváhagyva
2026-11-10) · **Döntések:** #15 (pénznem), #29 (árfolyam) · **Épül rá:** G2-02 (produkció, szervezet), G2-04 (Workflow
Engine), G2-14 (bizonylat), G2-16 (időlap), G2-18 (stábtag, részleg), G2-21 (eszköz), G2-23 (forgatási napok) · **Állapot:**
G2-javaslat, jóváhagyásra vár · implementáció nincs

Új döntési kérdés nincs. **Új függőség nincs.** A modul **követési és tervezési** képesség, **nem könyvelés**: nincs főkönyv,
áfabevallás, számlázás, bankegyeztetés (G1, terv 4.21).

**Fontos megállapítás a korábbi G2-kről:** a #15 (pénznem) és #29 (árfolyam) döntéseire a G2-02, G2-14, G2-19 és G2-21 már hivatkozik
(`CURRENCY_UNKNOWN`, `currency char(3)`, „napi árfolyam”), de **sehol nincs megtervezve** sem a pénznemlista, sem az
árfolyam-szolgáltatás. A „nem létező szolgáltatásra nem építhetünk” szabály miatt ez a G2 **definiálja** őket (2.1, 3. pont); a
**pénznemlista** (statikus adat) a 2., 14., 19., 21. lépcső **előfeltétele**, ezért a **G2-02 migrációjába kerül** (13. pont),
az **árfolyamletöltés** és a költségvetés a 24. lépcsővel jön.

## 1. Hatókör ebben a lépcsőben

- **Pénznem és árfolyam** közös szolgáltatása (3. pont): pénznemlista, napi árfolyam (EKB), kézi felülírás, átváltás.
- **Költségvetés** változatokkal: kategóriafa (sablonból), költséghelyek, sorok (mennyiség × egység × díjtétel × szorzó), zárolt jóváhagyott változat.
- **Tényleges költség**: kézi rögzítés, CSV-import, **származtatott javaslat** (időlap, bérelt eszköz), amelyet a felhasználó jóváhagy.
- **Díjtábla** (rate card) a származtatáshoz és a sorok kitöltéséhez.
- **Eltérésvizsgálat és előrejelzés**, küszöbjelzés; **riportok** (összesítő lap, részletezés, eltérés), CSV és PDF.
- **Nem része:** könyvelés (főkönyv, áfa, számla- és bankkezelés); bérszámfejtés és járulékok számítása; élő képletek és
  százalékos tételek (a szorzó és a kézi összeg elég M1-ben); `.xlsx` export (12. pont); szerződéses fizetési ütemterv és cash flow.

## 2. Integrációs határok

| Határ | Irány | Szerződés |
|---|---|---|
| Production (2.) | ← | `production.currency` (ISO 4217), `reference_currency` (EUR/USD), `country` (a sablonhoz); a modul beállításai a `production.settings['budget']`-ben |
| Organization (2.) | ← | A szállító/partner: `organization_id` (opcionális) |
| Workflow Engine (4.) | → | A költségvetés-változat folyamata `budget_default` (4.3) |
| Document (14.) | ← | A bizonylat dokumentum (`invoice` alaptípus, 13. pont), a költségtételhez `document_link`: `subject_kind = 'cost_entry'` |
| TimeTracker (16.) | ← | Jóváhagyott időlap percei → származtatott költségjavaslat (6.2); csak `approved` időlap |
| Crew (18.) | ← | A személy részlege a hétre (a költséghely javaslatához), a munkakör (díjtábla) |
| Equipment (21.) | ← | Bérelt eszköz `rental_fee_minor`/`currency_code` → származtatott javaslat (6.3); a karbantartás költsége **nem** kerül produkcióra (az eszköz produkciófüggetlen) |
| Shooting Schedule (23.) | ← | A forgatási napok száma a sor mennyiségéhez (`quantity_source = 'shooting_days'`) |
| Cast (19.) | ← | A szerződött napok **információként** (nincs automatikus költség: a `cast_engagement`-ben nincs összeg) |
| Notification (12.) | → | Események (9. pont) |
| Search (7.) | → | `SearchProvider`: költségtétel (leírás, szállító, bizonylatszám), költségvetési sor (leírás) |
| Audit (1.) | → | Minden írás auditált; személyhez kötött díj redaktált (7. pont) |
| Archive (26.) | később | A költségvetés és a tételek a produkciós archívum része |

### 2.1 Mi lett volna hiányzó szolgáltatás, és hol van most

| Szolgáltatás | Korábbi hivatkozás | Hol készül |
|---|---|---|
| Pénznemlista és `currency.validate` (`CURRENCY_UNKNOWN`) | G2-02 (`currency`, `reference_currency`), G2-14 (`money`), G2-19, G2-21 | **3.1**, a G2-02 migrációjában (statikus adat) |
| Árfolyam-szolgáltatás (#29) és `fx.convert` | G2-14 (megjelenítéskor napi árfolyam) | **3.2–3.4**, a 24. lépcsővel; a G2-14 csak megjelenítésre hívja (addig a `money` mező átváltás nélkül jelenik meg) |

## 3. Pénznem és árfolyam (közös szolgáltatás)

### 3.1 Pénznemlista

```sql
CREATE TABLE backlot.currency (
    code char(3) PRIMARY KEY CHECK (code ~ '^[A-Z]{3}$'),
    exponent smallint NOT NULL CHECK (exponent BETWEEN 0 AND 4),       -- tizedesjegyek száma (ISO 4217 „minor unit”)
    name_key text NOT NULL,                                             -- a felület fordítja (#16)
    active boolean NOT NULL DEFAULT true
);
```

- A lista a **projektben tárolt adatfájl** (`iso4217.json`, az ISO 4217 nyilvános listájából generálva, a forrás és a dátum a fájlban); a
  migráció tölti. **Csak forgalomban lévő, tizedes-egységgel rendelkező kód** kerül bele (a nemesfém-, teszt- és számlaegységkódok
  nem). Új kód felvétele adatfájl-módosítás és migráció, nincs felhasználói szerkesztés. Az adatfájl **licencét a G3 ellenőrzi**
  (a lista tényadat, de a közzétevő feltételei megerősítendők).
- **`currency.validate(code)`** → `CURRENCY_UNKNOWN`; a **`production.currency`, `reference_currency`, a `money` mező, a `cast_engagement.currency`, az
  `equipment.currency_code`** ellen ez fut (a hivatkozó táblák idegen kulccsal is a `backlot.currency`-re mutatnak).
- **Kerekítés:** az összeg a pénznem `exponent`-jére kerekítve (**félre felfelé**, `decimal.ROUND_HALF_UP`), a `decimal` standard modullal; a
  rendszer **sehol nem használ lebegőpontos számot** pénzre.

### 3.2 Árfolyam-tábla

```sql
CREATE TABLE backlot.fx_rate (                                          -- EUR-alapú, a forrás szerint
    quote_currency char(3) NOT NULL REFERENCES backlot.currency(code),  -- 1 EUR = rate × quote
    rate_date date NOT NULL,
    rate numeric(19,8) NOT NULL CHECK (rate > 0),
    source text NOT NULL CHECK (source IN ('ecb','manual','file')),
    fetched_at timestamptz NOT NULL,
    note text CHECK (length(note) <= 500),
    ...közös oszlopok production_id nélkül,
    PRIMARY KEY (quote_currency, rate_date, source)
);
```

- **Alap: EUR** (az EKB-referenciaárfolyamok EUR-alapúak); minden más pár **kereszt-árfolyam az EUR-on át**: 1 A = `rate(B) / rate(A)` B-ben
  (az EUR maga 1). Az **USD-keresztárfolyam** ugyanebből adódik (#29).
- **Egy napra egy pénznemre több forrás is lehet** (`ecb`, `manual`, `file`); az érvényes a **kézi > fájl > EKB** sorrendben (a
  kézi felülírás nyer, #29), a felülírt EKB-sor megmarad (előzmény).
- **Dátumkeresés:** egy dátumra az **utolsó, a dátumnál nem későbbi** árfolyam érvényes (hétvége, ünnepnap); az átváltás eredménye
  **mindig kiírja a használt árfolyam dátumát és forrását** (#15). Ha a dátumnál korábbi árfolyam sincs, `FX_RATE_MISSING`.
- **Lefedettség:** az EKB csak mintegy harminc pénznemre ad referencia-árfolyamot; a #15 szerint **minden ISO-pénznem támogatott**, ezért
  az EKB-lista **hiányzó pénznemeire kézi vagy fájlból importált** árfolyam kell (`FX_RATE_MISSING` jelzi, melyik). Az EKB által
  megszüntetett vagy felfüggesztett pénznem (régi sorai megmaradnak) új árfolyamot nem kap.
- **Fájlimport** (`backlot.fx.rate.import`): CSV (`dátum, pénznem, árfolyam`; EUR-alapú vagy megadott alappal), próbafuttatással, sorhibákkal.

### 3.3 Automatikus letöltés (#29)

- **Mit tölt le:** az EKB nyilvános **referencia-árfolyam** XML-jét (napi fájl; első indításkor vagy kimaradás után a 90 napos fájl; a **teljes
  előzmény** csak a felhasználó kifejezett kérésére). A cél **rögzített gazdagép és útvonal a kódban** (nem beállítható, nem
  tetszőleges URL): `www.ecb.europa.eu`, **HTTPS, tanúsítvány-ellenőrzéssel**, időkorláttal, **legfeljebb 16 MiB**-os válasszal.
- **Mi megy ki:** egy **sima GET kérés, adat nélkül**, **általános kliensnévvel** (nincs felhasználói név, e-mail vagy produkciós adat
  a kérésben). A felhasználói adat **nem** hagyja el a gépet (SEC-02, #29); a kérés a gép IP-címét természetesen mutatja a szolgáltatónak.
- **Mikor:** az alkalmazás indításakor és utána naponta egyszer, **amíg fut** (nincs háttérdémon), és csak ha az utolsó sikeres
  letöltés nem mai. Az EKB közzétételi rendje miatt a hétvégi/ünnepi nap újra a legutóbbi árfolyamot adja.
- **Kikapcsolható:** az **első indításkor a beállítás látható** (#29), felhasználói szinten ki-be kapcsolható, alapértelmezés: be (István döntése).
  Kikapcsolva a rendszer **egyetlen hálózati kérést sem indít**.
- **Hiba:** nincs hálózat, a gazdagép nem válaszol, a tartalom érvénytelen: **nem hiba**, a legutóbbi árfolyam érvényes, a felület
  **dátummal jelzi** („árfolyam: 2026-11-19”), és ha 7 napnál régebbi, külön **elavultsági jelzést** ad (számolt, nincs időzítő). A hiba
  `FX_FETCH_FAILED` kóddal az eseménynaplóba kerül, felhasználónak szánt üzenet csak a beállításoknál.
- **A letöltött XML megbízhatatlan:** a G2-22 3.1 szabálya szerint a `<!DOCTYPE`/`<!ENTITY` tartalmú válasz elutasított, az elemzés a
  standard `xml.etree`-vel; a pénznemkód a `backlot.currency` listában kell legyen (ismeretlen kód kihagyva, számolva), az árfolyam pozitív és a
  tartományban (`FX_RATE_INVALID`); **részleges válasz nem íródik be** (egy tranzakció). A HTTP-kérés a standard `urllib`-lel megy, nincs új függőség.

### 3.4 Átváltás

`backlot.fx.convert {amount, from, to, date}` → `{amount, rate, rate_date, source}`; a kerekítés a cél pénznem `exponent`-jére (3.1). **Tiszta
függvény** a betöltött árfolyamokon (azonos bemenetre azonos kimenet), egy hely az átváltás számolásának (a G2-14 megjelenítése is ezt hívja).

## 4. Adatmodell (DDL-vázlat, `backlot` séma)

### 4.1 Sablon, kategória, költséghely

```sql
CREATE TABLE backlot.budget_template (                                   -- kategóriafa-sablon, produkciófüggetlen
    id uuid PRIMARY KEY, code text NOT NULL UNIQUE,
    label_key text, name text CHECK (length(name) <= 200),
    country char(2),                                                      -- NULL: bárhol használható
    production_type_id uuid REFERENCES backlot.production_type(id),
    retired_at timestamptz,
    ...közös oszlopok production_id nélkül,
    CHECK ((label_key IS NULL) <> (name IS NULL))
);
CREATE TABLE backlot.budget_template_category (
    template_id uuid NOT NULL REFERENCES backlot.budget_template(id),
    code text NOT NULL CHECK (code ~ '^[A-Z0-9][A-Z0-9._-]{0,30}$'),
    parent_code text,
    section text NOT NULL CHECK (section IN ('above_the_line','below_the_line','post','other')),
    label_key text, name text, ordinal integer NOT NULL,
    PRIMARY KEY (template_id, code),
    CHECK ((label_key IS NULL) <> (name IS NULL))
);

CREATE TABLE backlot.cost_category (                                      -- a produkció fája, a sablonból másolva
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    code text NOT NULL, parent_id uuid REFERENCES backlot.cost_category(id),
    section text NOT NULL CHECK (section IN ('above_the_line','below_the_line','post','other')),
    label_key text, name text CHECK (length(name) <= 200), ordinal integer NOT NULL,
    retired_at timestamptz,
    ...közös oszlopok,
    UNIQUE (production_id, code),
    CHECK ((label_key IS NULL) <> (name IS NULL))
);

CREATE TABLE backlot.cost_center (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    code text NOT NULL, name text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    department_id uuid REFERENCES backlot.team(id),                      -- kind = 'department' (trigger)
    entity_id uuid REFERENCES backlot.production_entity(id),             -- epizód vagy szekvencia
    retired_at timestamptz,
    ...közös oszlopok,
    UNIQUE (production_id, code)
);
```

- **A beépített sablonok** (migráció, nyelvi kulcsokkal #16): **film**, **sorozat**, **reklám**: az iparági felosztás (above the line: jogok és
  történet, producer, rendező, szereplők; below the line: gyártási stáb, díszlet, kamera, fény/grip, hang, jelmez, smink, helyszínek,
  szállítás, eszközbérlet; post: utómunka, VFX, zene, hang-utómunka; other: biztosítás, jogi, általános költség, tartalék). A **pontos
  felosztás** a G3-on a Movie Magic és az AICP nyilvános szerkezete mint **funkcionális referencia** alapján, **saját számozással** készül (a
  számlaszámokat nem vesszük át). **Országonként eltérő sablon** a `country` oszlopon át vehető fel.
- **Saját sablon** vehető fel (`backlot.budget.template.*`, `owner`); a beépített nem törölhető, csak visszavonható.
- **A produkció kategóriafája** létrehozáskor (vagy az első költségvetéskor) **másolódik** a sablonból (mint a G2-03 `production_structure_rule`): a
  produkció szabadon módosíthatja (új ág, átnevezés, visszavonás), a sablon változása a meglévő fát **nem** érinti. Körmentességet és a mélységet
  (legfeljebb 6 szint) trigger őrzi (`BUDGET_CATEGORY_INVALID`); törlés nincs, csak visszavonás, ha van rá hivatkozó sor vagy tétel.
- **Költséghely:** részleg (a `team`, G2-15/18), epizód vagy egység, vagy szabad; egy tétel és egy sor **egy** költséghelyre tartozhat, **a
  kategória nem költséghely** (két független tengely: *mi* és *kié*).

### 4.2 Költségvetés-változat és sorok

```sql
CREATE TABLE backlot.budget (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    number integer NOT NULL CHECK (number > 0),
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),            -- pl. „előzetes”, „jóváhagyott”, „módosított”
    status_id uuid NOT NULL REFERENCES backlot.status(id),                 -- G2-04: draft | approved | superseded | discarded
    base_budget_id uuid REFERENCES backlot.budget(id),
    note text CHECK (length(note) <= 2000),
    approved_at timestamptz,
    ...közös oszlopok,
    UNIQUE (production_id, number)
);

CREATE TABLE backlot.budget_fx (                                          -- a változat árfolyamai a produkció pénznemére
    budget_id uuid NOT NULL REFERENCES backlot.budget(id),
    currency char(3) NOT NULL REFERENCES backlot.currency(code),
    rate_to_production numeric(19,8) NOT NULL CHECK (rate_to_production > 0),   -- 1 egység = rate egység a produkció pénznemében
    rate_date date NOT NULL, source text NOT NULL,
    PRIMARY KEY (budget_id, currency)
);

CREATE TABLE backlot.budget_line (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    budget_id uuid NOT NULL REFERENCES backlot.budget(id),
    category_id uuid NOT NULL REFERENCES backlot.cost_category(id),
    cost_center_id uuid REFERENCES backlot.cost_center(id),
    description text NOT NULL CHECK (length(description) BETWEEN 1 AND 500),
    quantity numeric(14,4) NOT NULL CHECK (quantity >= 0),
    quantity_source text NOT NULL DEFAULT 'manual' CHECK (quantity_source IN ('manual','shooting_days')),
    unit text NOT NULL CHECK (unit IN ('flat','day','hour','week','month','piece','km','allowance')),
    rate numeric(19,4) NOT NULL CHECK (rate >= 0),
    currency char(3) NOT NULL REFERENCES backlot.currency(code),
    multiplier numeric(10,4) NOT NULL DEFAULT 1 CHECK (multiplier >= 0),   -- pl. hétvégi pótlék, többszörös
    amount numeric(19,4) NOT NULL,                                          -- az eredeti pénznemben, kerekítve (trigger számolja)
    amount_production numeric(19,4) NOT NULL,                               -- a produkció pénznemében, a budget_fx szerint
    note text CHECK (length(note) <= 1000),
    ordinal integer NOT NULL,
    retired_at timestamptz,
    ...közös oszlopok
);
CREATE INDEX ON backlot.budget_line (budget_id, category_id);
```

- **A sor összege:** `amount = round(quantity × rate × multiplier, exponent(currency))`, `amount_production = round(amount × rate_to_production,
  exponent(production.currency))`; **a triggerek számolják** (nincs kézzel írható származtatott érték), így a kettő nem térhet el az
  összetevőktől. A kategória- és a főösszeg a **már kerekített sorok összege** (a nézetben látott számok összeadódnak).
- **Pénznem-pillanatkép:** a `budget_fx` a **változat** árfolyamait rögzíti a produkció pénznemére; **piszkozatnál** a felhasználó frissítheti
  (`backlot.budget.fx.refresh`, a legutóbbi árfolyamokra), **jóváhagyáskor lezárul**, így a jóváhagyott összeg **nem mozog** az árfolyammal.
  A produkció pénznemében lévő sor árfolyama 1 (nincs sor a `budget_fx`-ben).
- **Mennyiség a tervből:** `quantity_source = 'shooting_days'`: a mennyiség a **legutóbbi kiadott revízió** (G2-23) forgatási napjainak száma
  (különböző dátum, opcionálisan egységre szűrve); **kifejezett paranccsal frissül** (`backlot.budget.line.refresh_quantity`), **csak
  piszkozatban**. Más forrás (szereplő napjai, stábnapok) **nincs** M1-ben (a kézi mennyiség és a 24-2).
- **Százalékos és képletes tételek** (tartalék, járulék) M1-ben **nincsenek**: a felhasználó a szorzóval vagy kézi összeggel viszi fel
  (a G1 sem kért élő képletet). Segéd: `backlot.budget.line.add_percent` **egyszeri kiszámítással** felvesz egy lapos sort (nem élő hivatkozás; a leírása
  kiírja az alapot és a dátumot).
- **Zárolás:** jóváhagyott (vagy lecserélt) változat sorai **nem módosíthatók** (trigger: `BUDGET_LOCKED`); módosítás **új változat** a jóváhagyottból
  (`backlot.budget.new_version`, másolat `draft`-ként, `base_budget_id`-vel).

### 4.3 Folyamat

`budget_default` (G2-04, zárt őrfeltételekkel; **a jogosultság parancsszintű**): `draft → approved`, `draft → discarded`, `approved → superseded` (**automatikusan**, amikor
egy újabb változat jóváhagyásra kerül). Egy produkcióhoz **egyszerre egy `approved`** változat tartozik (az **aktív**: ehhez mérünk). Jóváhagyás: sorok legalább
egy; a `budget_fx` minden használt pénznemre teljes (`FX_RATE_MISSING`, ha nem). A név **címke** („előzetes”, „jóváhagyott”, „módosított”), nem állapot.

### 4.4 Díjtábla

```sql
CREATE TABLE backlot.rate_card (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    person_id uuid REFERENCES backlot.person(id),
    job_title_id uuid REFERENCES backlot.job_title(id),
    category_id uuid NOT NULL REFERENCES backlot.cost_category(id),         -- ide kerül a származtatott költség
    cost_center_id uuid REFERENCES backlot.cost_center(id),
    unit text NOT NULL CHECK (unit IN ('hour','day','week','flat')),
    rate numeric(19,4) NOT NULL CHECK (rate >= 0),
    currency char(3) NOT NULL REFERENCES backlot.currency(code),
    valid_from date NOT NULL, valid_until date,
    ...közös oszlopok,
    CHECK ((person_id IS NULL) <> (job_title_id IS NULL)),
    CHECK (valid_until IS NULL OR valid_until >= valid_from),
    EXCLUDE USING gist (production_id WITH =, COALESCE(person_id, job_title_id) WITH =, unit WITH =,
                        daterange(valid_from, valid_until, '[]') WITH &&) WHERE (person_id IS NOT NULL OR job_title_id IS NOT NULL)
);
```

- **Keresés:** a személyhez kötött sor **elsőbbséget** élvez a munkakörhöz kötöttel szemben; az átfedés kizárt (`RATE_CARD_OVERLAP`), így a keresés egyértelmű.
- **Használat:** az **időlap-költség származtatása** (6.2; csak `hour` egységű sor) és a sor **kitöltése** (`backlot.budget.line.apply_rate_card`: a díjat előtölti, a felhasználó
  jóváhagyja). A `day`/`week`/`flat` egység a **kitöltést** szolgálja, a származtatást nem (nincs ismert napi óraszám).
- **A személyhez kötött díj személyes (pénzügyi) adat** (G2-18 `financial` szint): `owner`, `producer` látja; az audit a személyhez kötött `rate`/`currency` értékét
  **redaktálja** (G2-01 sózott kivonat); az anonimizálás törlő kezelője **törli a személyhez kötött díjsorokat** (7. pont).

### 4.5 Tényleges költség, import, javaslat

```sql
CREATE TABLE backlot.cost_import (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    original_name text NOT NULL, source_sha256 text NOT NULL, row_count integer NOT NULL, applied_count integer NOT NULL,
    status text NOT NULL CHECK (status IN ('dry_run','applied','failed')),
    options jsonb NOT NULL,                                                -- elválasztó, tizedesjel, dátumformátum, oszlop-megfeleltetés
    errors jsonb NOT NULL DEFAULT '[]',                                    -- [{row, code, detail}], legfeljebb 1000
    ...közös oszlopok
);

CREATE TABLE backlot.cost_entry (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    entry_date date NOT NULL,
    category_id uuid NOT NULL REFERENCES backlot.cost_category(id),
    cost_center_id uuid REFERENCES backlot.cost_center(id),
    organization_id uuid REFERENCES backlot.organization(id),             -- szállító/partner
    vendor_name text CHECK (length(vendor_name) <= 200),
    description text NOT NULL CHECK (length(description) BETWEEN 1 AND 500),
    reference_no text CHECK (length(reference_no) <= 100),                -- számlaszám, bizonylatszám
    amount numeric(19,4) NOT NULL CHECK (amount <> 0),                    -- az eredeti pénznemben; negatív = jóváírás
    currency char(3) NOT NULL REFERENCES backlot.currency(code),
    fx_rate numeric(19,8) NOT NULL CHECK (fx_rate > 0), fx_date date NOT NULL, fx_source text NOT NULL,   -- a rögzítéskor befagyasztva
    amount_production numeric(19,4) NOT NULL,                             -- a produkció pénznemében (trigger)
    origin text NOT NULL CHECK (origin IN ('manual','import','derived')),
    import_id uuid REFERENCES backlot.cost_import(id),
    source_kind text CHECK (source_kind IN ('timesheet','equipment_rental')),
    source_id uuid, dedupe_key text,
    note text CHECK (length(note) <= 1000),
    retired_at timestamptz,
    search_vector tsvector GENERATED ALWAYS AS (…) STORED,
    ...közös oszlopok,
    CHECK ((source_kind IS NULL) = (source_id IS NULL))
);
CREATE INDEX ON backlot.cost_entry (production_id, category_id, entry_date);
CREATE UNIQUE INDEX ON backlot.cost_entry (production_id, dedupe_key) WHERE dedupe_key IS NOT NULL AND retired_at IS NULL;

CREATE TABLE backlot.cost_proposal (                                      -- származtatott költség: javaslat, amíg a felhasználó el nem fogadja
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    source_kind text NOT NULL CHECK (source_kind IN ('timesheet','equipment_rental')),
    source_id uuid NOT NULL, source_state text NOT NULL,                  -- a forrás ujjlenyomata (6.2, 6.3)
    entry_date date NOT NULL, description text NOT NULL,
    category_id uuid REFERENCES backlot.cost_category(id), cost_center_id uuid REFERENCES backlot.cost_center(id),
    amount numeric(19,4) NOT NULL, currency char(3) NOT NULL REFERENCES backlot.currency(code),
    status text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','accepted','rejected','superseded')),
    entry_id uuid REFERENCES backlot.cost_entry(id),
    ...közös oszlopok,
    UNIQUE (production_id, source_kind, source_id, source_state)
);
```

- **Befagyasztott átváltás:** a tétel a rögzítésekor **eltárolja** az árfolyamot, annak dátumát és forrását (a **tétel dátumára** érvényes árfolyam, 3.2), és a **produkció
  pénznemére átszámolt összeget**. A későbbi árfolyam-változás vagy -frissítés a **rögzített tételeket nem mozgatja** (a #15 „minden összeg pénznemmel és árfolyam-dátummal”).
  Kézi felülírás (a felhasználó megadja az árfolyamot) `fx_source = 'manual'`; az átváltási adat később is látszik a tételen. A módosított összeg/pénznem/dátum új átváltást számol.
- **Negatív összeg = jóváírás** (visszatérítés, helyesbítés); a nulla tilos. Törlés helyett **visszavonás** (`retired_at`), kivéve a tévesen felvett tétel azonnali
  visszavonását (ez is `retired_at`, auditált).
- **Duplikáció-védelem:** a `dedupe_key` a (dátum, szállító vagy `organization_id`, összeg, pénznem, `reference_no`) normalizált kivonata; **importnál** az azonos kulcsú sor **kihagyva és
  jelentve** (nem hiba); kézi felvételnél figyelmeztetés (a felhasználó felülbírálhatja: a `dedupe_key` ilyenkor `NULL`).
- **Bizonylat:** a `document_link` (`subject_kind = 'cost_entry'`, `relation = 'receipt'`) köti; a fájl a G2-14 `attachment` gyökerében. A tétel a bizonylat nélkül is felvehető.

## 5. Eltérésvizsgálat, előrejelzés, riportok (számolt nézetek)

Minden érték **lekérdezéskor** számolódik (nincs tárolt összesítő, a Dashboard G2-06 elvével): SQL-összesítés a tételekből és a jóváhagyott változat soraiból.

- **Eltérés** kategóriánként és költséghelyenként: `budget` (az **aktív jóváhagyott** változat sorainak `amount_production`-ja) − `actual` (a nem visszavont tételek
  `amount_production`-ja, a kért időszakra: `entry_date`) = `variance`; a százalék az alapértékhez. A kategóriafa **szintjei szerint gördül fel** (levél → szülő).
- **Előrejelzés** (`estimate to complete`): `forecast` = `actual + etc`; az `etc` a `budget_forecast` kézi értéke, ennek hiányában **a hátralévő költségvetés**
  (`max(budget − actual, 0)`); a felület megkülönbözteti a kézi és a számolt értéket. **A számolt `etc` nem „előrejelzés”**, csak a tervezett hátralék; ezt a
  riport szövege kimondja.
- **Összesítő lap (topsheet):** szekciónként (above the line, below the line, post, other) és főösszegként: költségvetés, tényleges, előrejelzés, eltérés.
- **Referenciapénznem:** kérésre a riport a `reference_currency`-ben (EUR/USD) is megjeleníthető, **a kért dátum árfolyamán, dátummal jelölve**, mindig **számolt megjelenítés**
  (a tárolt érték a produkció pénznemében marad).
- **Küszöbjelzés:** a `production.settings['budget']`: `warn_percent` (alap 90), `over_percent` (alap 100). Egy tétel/sor írása után a szolgáltatás **ugyanabban a tranzakcióban**
  összeveti az érintett kategória **előtte/utána** állapotát, és ha **küszöböt lépett át**, eseményt ír (9. pont); **nincs időzítő és nincs tárolt jelző**.
- **Riportok:** `backlot.budget.report.topsheet`, `.category` (részletezés), `.variance`, `.cost_center`, `.cost_log` (tételek szűrve), `.derived_pending` (feldolgozatlan javaslatok);
  mind csak a jogosultnak (6. pont), CSV (UTF-8 BOM-mal, `;` vagy `,` választható, képlet-védelem) és PDF (`QPdfWriter`).

## 6. Tényleges költség forrásai

### 6.1 Kézi rögzítés és CSV-import

- **Kézi:** `backlot.cost.add`: dátum, kategória, költséghely, szállító, leírás, összeg + pénznem; az átváltás és a `amount_production` a mentéskor (3.4, 4.5).
- **CSV-import** (`backlot.cost.import`, **próbafuttatással**): standard `csv` modul, **UTF-8** (BOM-mal vagy anélkül), a felhasználó **kifejezetten választja** az elválasztót (`,` vagy `;`), a tizedesjelet (`.` vagy
  `,`) és a dátumformátumot (ISO, `ÉÉÉÉ.HH.NN`, `NN/HH/ÉÉÉÉ`), **oszlop-megfeleltetéssel** (kötelező: dátum, kategória-kód, leírás, összeg, pénznem); találgatás (pl. a tizedesjel
  kitalálása) **nincs**, mert csendben hibás összeget adna; a felület a fejlécből **javaslatot** tesz. A próbafuttatás sorhibákat listáz (`COST_IMPORT_INVALID` részletekkel); az **alkalmazás
  mindent vagy semmit** (egy tranzakció, legfeljebb 100 000 sor, a duplikátumok kihagyva és jelentve). A `cost_import` rögzíti a forrásfájl hash-ét és a beállításokat (az importált sorok `import_id`-t
  hordoznak, így a **teljes import egy paranccsal visszavonható** `backlot.cost.import.revert`, ha a tételek még nem módosultak).
- Importált sor ismeretlen kategóriakódja vagy pénzneme **sorhiba** (nem hozunk létre automatikusan kategóriát).

### 6.2 Időlapból származtatott költség (#16)

- **Forrás:** `approved` időlap (G2-16: produkció, személy, ISO-hét); **a perc** a `backlot.timesheet.get` összege, a javító ellenbejegyzésekkel együtt. **Piszkozat vagy beadott időlap soha nem számít.**
- **Javaslat** (`backlot.cost.derive.timesheets`, kifejezett parancs, nem időzített): minden jóváhagyott időlapra, amelyhez van **érvényes `hour` díjsor** (személy > munkakör, a hét hétfőjén), egy `cost_proposal`:
  `amount = round(minutes / 60 × rate, exponent)`, kategória és költséghely a díjsorból (a költséghely hiányában a személy hétre érvényes részlege, ha van `cost_center` hozzá). **Díjsor nélküli időlap** a
  `derived_pending` riportban „nincs díj” jelzéssel szerepel, javaslat nem készül.
- **Ujjlenyomat:** `source_state` = a percek összege és a díjsor azonosítója+verziója. Ha egy **elfogadott** javaslat forrása később megváltozik (javító ellenbejegyzés), új javaslat készül **a különbözettel**
  (lehet negatív: jóváírás), a régi `accepted` marad; a még függő régit a rendszer `superseded`-re teszi. Az elfogadás **a felhasználó döntése** (`backlot.cost.proposal.accept` / `.reject`), pénzmozgás itt nincs.
- A **tétel leírása nem tartalmaz nevet** („Munkaidő, 2026-W46”), a személy a forrás-hivatkozáson (`source_id`) át azonosítható; az anonimizálás (G2-18) a hivatkozott időlapot kezeli, a tétel (számviteli adat)
  marad, a név nem jelenik meg (jogi szempontok: G2-18 5.3/4).

### 6.3 Bérelt eszközből származtatott költség (#21)

- **Forrás:** `equipment` sor `ownership ∈ {rented, borrowed}` és `rental_fee_minor` megadva, a **produkcióra szóló** `equipment_booking`-gal. Az `rental_fee_minor` a **bérleti időszak teljes díja** (G2-21 tényadat, a pénznem külön).
- **Javaslat** (`backlot.cost.derive.equipment`): akkor készül, ha a **bérleti időszak minden foglalása ugyanazé a produkcióé**; az összeg a teljes díj, a dátum a `rental_from`, a kategória a felhasználó választása
  (**elfogadáskor kötelező**, az előző választást előtölti). Ha a foglalások **több produkción** osztoznak, a javaslat **nem készül** (`ambiguous_allocation` jelzés a riportban), a költséget a felhasználó kézzel osztja meg
  (a megosztási arány tervezői döntés, nem származtatott tény). A karbantartás költsége (G2-21) **nem** kerül produkcióra (az eszköz produkciófüggetlen).
- Az **ujjlenyomat** a díj, a pénznem és az időszak; változásnál új javaslat a 6.2 szerint.
- **Más forrásokból nincs származtatás** M1-ben: a szereplői szerződéses díj (a `cast_engagement`-ben nincs összeg, G2-19), a szerződés `money` metaadata, a helyszíndíj (G2-20: nincs költségmező) → ezek a
  **kézi** vagy **importált** tételben jönnek; a származtatott források bővítése (24-2) külön döntés.

## 7. Hozzáférés, személyes adat

| Adat | Ki látja és szerkeszti |
|---|---|
| Költségvetés, sorok, díjtábla, költségtételek, javaslatok, előrejelzés | `owner`, `producer` (olvasás és írás) |
| **Összesített** eltérés kategóriánként és költséghelyenként (összegek, tételek és díjak nélkül) | `coordinator` (csak olvasás) |
| Pénznem, árfolyam | minden felhasználó (olvasás); kézi árfolyam és import: `owner`, `producer` |
| Sablon és kategóriafa | olvasás a produkció tagjainak, szerkesztés `owner`, `producer` |

- A szabályok a szolgáltatásrétegben és a lekérdezésben **már most** érvényesek (M2-re kész; G1: szűkített elérés). A keresés a költségtételekre és sorokra **csak a jogosultnak** ad találatot.
- **Személyes adat (SEC-06):** a **személyhez kötött díj** (4.4) pénzügyi adat: az audit a `rate`/`currency` értéket redaktálja (G2-01), a megtekintés auditált, a **törlő kezelő** (G2-18 5.3
  regiszter) törli a személyhez kötött díjsorokat az `person.anonymize`-nál; a **költségtétel** számviteli adat marad. A hivatkozásjelentés kiírja a díjsorokat és a származtatott tételeket. A **szállító**
  `organization` (nem személy), a `vendor_name` szabad szöveg (egyéni vállalkozó neve személyes adat lehet: a G2-18 5.3/1 szabálya szerint nem írjuk át, a felület a kerülését kéri).
- **Az összeg és a leírás nem személyes jelölésű mező**: az audit értéke szerepel (üzleti adat).

## 8. Parancsok és hibakódok

| Parancs | Mit csinál |
|---|---|
| `backlot.fx.rate.get` / `.list` / `.set` / `.import` / `.refresh` | Árfolyam: lekérdezés, kézi megadás, fájlimport, letöltés kérése; `backlot.fx.convert`; `backlot.fx.settings.get/set` (automatikus letöltés ki/be) |
| `backlot.budget.template.*` | Sablonok és kategóriáik (`owner`) |
| `backlot.budget.category.create` / `.update` / `.move` / `.retire` / `.list` | A produkció kategóriafája |
| `backlot.budget.cost_center.create` / `.update` / `.retire` / `.list` | Költséghelyek |
| `backlot.budget.create` / `.new_version` / `.approve` / `.discard` / `.list` / `.get` / `.fx.refresh` | Változatok (4.2, 4.3) |
| `backlot.budget.line.add` / `.update` / `.retire` / `.reorder` / `.refresh_quantity` / `.apply_rate_card` / `.add_percent` / `.list` | Sorok |
| `backlot.budget.rate_card.set` / `.retire` / `.list` | Díjtábla |
| `backlot.cost.add` / `.update` / `.retire` / `.list` / `.get` | Tényleges tétel |
| `backlot.cost.import` / `.import.revert` | CSV-import (6.1) |
| `backlot.cost.derive.timesheets` / `.equipment` / `.proposal.accept` / `.proposal.reject` / `.proposal.list` | Származtatott költség (6.2, 6.3) |
| `backlot.budget.forecast.set` / `.clear` | Kézi előrejelzés (`etc`) |
| `backlot.budget.report.*` / `backlot.budget.export` | Riportok és export (5.) |
| `backlot.budget.settings.get` / `.set` | Küszöbök (`warn_percent`, `over_percent`) |

Hibakódok: `BUDGET_NOT_FOUND`, `BUDGET_STATE_INVALID`, `BUDGET_LOCKED`, `BUDGET_LINE_INVALID`, `BUDGET_CATEGORY_INVALID`, `BUDGET_COST_CENTER_INVALID`,
`BUDGET_FORBIDDEN`, `BUDGET_EXPORT_FAILED`, `BUDGET_SETTINGS_INVALID`, `COST_ENTRY_INVALID`, `COST_IMPORT_INVALID`, `COST_PROPOSAL_INVALID`, `RATE_CARD_INVALID`,
`RATE_CARD_OVERLAP`, `FX_RATE_MISSING`, `FX_RATE_INVALID`, `FX_FETCH_FAILED`, `CURRENCY_UNKNOWN`, `CONFLICT_STALE_VERSION`; valamint a `PERSON_*`, `ORG_*`, `DOCUMENT_*`, `WORKFLOW_*`,
`AUDIT_*` kódok változatlanul.

## 9. Események

`backlot.budget.approved`, `backlot.budget.threshold_crossed` (hasznos teher: kategória-azonosító, szint `warn`/`over`, százalék; **összeg nélkül**, hogy az értesítés ne szivárogtassa a
pénzügyi adatot), `backlot.cost.imported`; az értesítés **címzettje** a `production_owner` és a `production_approver` (G2-16/G2-12 13. pont); az értesítés szövege nem tartalmaz összeget. A küszöbjelzés
**csak olyan címzettnek** mehet, aki a költségadatot látja (7. pont).

## 10. Never crash

- **Pénz = `decimal`**: nincs lebegőpontos szám, az adatbázisban `numeric` korlátozott tartománnyal (a túlcsordulás `CHECK`-sértés, `BUDGET_LINE_INVALID`/`COST_ENTRY_INVALID`, nem összeomlás).
- **Hiányzó árfolyam** egy tételnél vagy sornál: a művelet `FX_RATE_MISSING` hibával áll meg, **nincs félig írt tétel**; a riportok a hiányzó árfolyamú sort **jelzéssel kihagyva
  mutatják**, nem dőlnek össze.
- **Hálózat és letöltés:** minden hiba kezelt (3.3); a tartalom korlátos (16 MiB), időkorlátos; az XML-elemzés külön ellenőrzéssel; a részleges válasz nem íródik be.
- **Import:** hibás sor sorhiba, nem leállás; a fájl mérete és a sorok száma korlátos; a tranzakció mindent vagy semmit; a lemezes/kódolási hiba kezelt (`COST_IMPORT_INVALID`).
- **Zárolt változat módosítása** triggerrel megakadályozva (a szolgáltatás előre hibakódot ad); egyidejű módosítás `CONFLICT_STALE_VERSION`; a jóváhagyás **egy tranzakció** (állapot,
  `budget_fx` lezárás, korábbi `superseded`, esemény).
- **Osztás:** a százalék és az arány számításánál nulla alap kezelt (nincs osztás nullával; a százalék ilyenkor „nem értelmezett”).
- **Származtatott költség:** hibás forrás (törölt időlap, kivezetett eszköz) a javaslatot `superseded`/kihagyottra teszi, nem akasztja meg a többit.

## 11. Sorrend és függés

| Rész | Kell hozzá |
|---|---|
| Pénznemlista, `currency.validate` | **G2-02 migrációja** (2. lépcső), a 14., 19., 21. lépcső előfeltétele |
| Árfolyam, letöltés, átváltás | a pénznemlista; a hálózati rész külön kikapcsolható |
| Sablon, kategória, költséghely, költségvetés, sorok, díjtábla, kézi/import tétel, eltérés, riport | 2. lépcső, Workflow Engine (G2-04) |
| Bizonylat a tételhez | 14. lépcső |
| Időlap-származtatás | 16. és 18. lépcső |
| Eszköz-származtatás | 21. lépcső |
| Mennyiség a forgatási napokból | 23. lépcső |
| Esemény, küszöbértesítés | 12. lépcső |

A G1 „Előfeltétel: 15. és 23. lépcső” **szűkül**: a modul **önmagában** (kézi tételekkel és importtal) a 15. és 23. nélkül is használható; a 23. csak a `shooting_days` mennyiségforráshoz,
a 15. **semmihez** nem kell (a költségvetés nem erőforrás-terv).

## 12. Eltérések a G1-től (összegzés)

| # | G1 | G2-javaslat | Indok |
|---|---|---|---|
| 1 | `.xlsx` export, „a G2-n eldöntendő eszközzel” | **M1-ben nincs**: CSV (UTF-8 BOM, választható elválasztó, képlet-védelem) és PDF; a táblázatkezelők a CSV-t megnyitják | Új függőség nélkül; az `.xlsx` a 24-2 (XlsxWriter vagy hasonló jelölt, külön G3), ha a felhasználó kéri |
| 2 | Összegek „pénznemmel és árfolyam-dátummal” | Eredeti pénznemben tárolt összeg + **befagyasztott átváltás** a tételen (és a változat `budget_fx`-én) | A jóváhagyott költségvetés és a rögzített tétel nem mozog a napi árfolyammal; így az eltérés reprodukálható |
| 3 | Szorzó, díjtétel (képlet nincs megadva) | Nincs élő képlet és százalékos sor; szorzó és egyszeri számítású segéd | Az élő képlet függőségi gráfot és ciklusvédelmet kérne; M1-ben nem szükséges |
| 4 | Származtatott tétel: „munkaórák, eszközbérlés” | **Időlap** és **bérelt eszköz** (egyértelmű elosztással); más forrás nincs | A többi forrásban nincs összeg (G2-19, G2-20), vagy az elosztás tervezői döntés (G2-21) |
| 5 | Hozzáférés „M1-ben a helyi felhasználó látja” | A szerepszabály **már most** él; a `coordinator` csak összesítést lát | A G2-18/19 mintája (M2-re kész) |
| 6 | (nem szerepelt) | **Pénznemlista és árfolyam-szolgáltatás definiálva** (2.1, 3. pont), a pénznemlista a G2-02-be | A #15/#29 hivatkozott, de nem tervezett szolgáltatás |

## 13. Kiegészítések a korábbi G2-khez (megvalósításkor)

| G2 | Kiegészítés |
|---|---|
| [G2-02](CFA4-BACKLOTHUB-G2-02-PRODUCTION-MANAGEMENT-2026-11-11.md) | `backlot.currency` tábla és adatfájl (3.1); a `production.currency` és `reference_currency` idegen kulcs rá; `currency.validate` |
| [G2-14](CFA4-BACKLOTHUB-G2-14-DOCUMENT-MANAGEMENT-2026-11-21.md) | Új alaptípus: `invoice` (számla/nyugta); új `subject_kind`: `cost_entry`; a `money` mező átváltása a `backlot.fx.convert`-et hívja (megjelenítés) |
| [G2-12](CFA4-BACKLOTHUB-G2-12-NOTIFICATION-CENTER-2026-11-14.md) 13. pont | Események: `backlot.budget.approved`, `.threshold_crossed`, `backlot.cost.imported` |
| [G2-18](CFA4-BACKLOTHUB-G2-18-CREW-MANAGEMENT-2026-11-21.md) 5.3 | Törlő kezelő a személyhez kötött `rate_card` sorokra |
| [G2-21](CFA4-BACKLOTHUB-G2-21-EQUIPMENT-MANAGEMENT-2026-11-21.md) | `equipment.currency_code` idegen kulcs a `backlot.currency`-re |

## 14. Tesztek (G5-re előre)

- **Pénz és kerekítés:** táblázatos teszt a félre felfelé kerekítésre minden `exponent`-re (0, 2, 3); a sor- és kategóriaösszeg egyezése (összeg = kerekített sorok összege); nincs lebegőpontos érték
  (statikus ellenőrzés a modulban); túlcsordulás.
- **Árfolyam:** kereszt-árfolyam az EUR-on át (ellenőrzött párok), dátumkeresés hétvégén, forrás-sorrend (kézi > fájl > EKB), hiányzó pénznem (`FX_RATE_MISSING`), fájlimport hibás sorokkal,
  az átváltás tisztasága.
- **Letöltés (kiszolgáló-hamisítóval, nincs valós hálózat a tesztben):** sikeres XML, hiányzó mezők, ismeretlen pénznemkód, `<!DOCTYPE`/`<!ENTITY`, túl nagy válasz, időtúllépés, tanúsítványhiba,
  részleges válasz (nem íródik be), **a kérés nem tartalmaz felhasználói adatot** (fejlécek és útvonal ellenőrizve), kikapcsolva **nulla kérés**, a gazdagép nem módosítható, elavultsági jelzés 7 nap után.
- **Költségvetés:** sablonból másolt fa, kategória-ciklus és mélység, sor számítása (trigger), zárolás (`BUDGET_LOCKED`), új változat, jóváhagyás (egy aktív, régi `superseded`, `budget_fx` lezárás, hiányzó árfolyam),
  `refresh_quantity` a G2-23 revíziójából, `add_percent` egyszeri számítás.
- **Tétel:** befagyasztott átváltás (a későbbi árfolyam nem mozgatja), kézi árfolyam, jóváírás, duplikáció-védelem (`dedupe_key`), visszavonás.
- **Import:** elválasztó és tizedesjel kifejezett választása (a `;`/`,` kombinációk), dátumformátumok, UTF-8 BOM, hibás sorok, mindent vagy semmit, 100 000 sor, visszavonás (`import.revert`), képlet-védelem a CSV-exportban.
- **Származtatás:** csak `approved` időlap, díjsor-keresés (személy > munkakör), díjsor nélküli időlap jelzése, javító ellenbejegyzés → különbözet-javaslat (negatív is), bérelt eszköz egyértelmű és megosztott foglalással,
  idempotens újrafuttatás (egyedi `source_state`).
- **Eltérés és előrejelzés:** kategória-fa gördülés, időszak-szűrés, kézi és számolt `etc`, küszöb-átlépés **tranzakción belül** (előtte/utána), a hasznos teher **összeg nélkül**, referenciapénznem-megjelenítés dátummal.
- **Hozzáférés:** minden szerep minden adatra (a `coordinator` csak összesítés), a keresés nem szivárogtat; az audit **redaktálja** a személyhez kötött díjat; az anonimizálás törli a díjsorokat, a tétel marad.
- **Riport és export:** topsheet, részletezés, eltérés, cost log; CSV és PDF; a hiányzó árfolyamú sor kihagyva, jelezve.
- **Never crash:** fuzzolt import, hibás XML, extrém számok, nulla alap a százalékban, egyidejű módosítás.
- **Felületi ellenőrzés:** `ui.screenshot` a költségvetés-szerkesztőről, az összesítő lapról, az eltérésnézetről és az árfolyam-beállításról (első indítási beállítás).
- **Mérés** a CFA4-MIN gépen: 20 000 költségvetési sor, 200 000 tétel; az eltérés- és topsheet-számítás, az import, a származtatás ideje.

## 15. Licenc (G3-ra előre)

Új függőség nincs: `decimal`, `csv`, `urllib`, `xml.etree` (standard könyvtár); PDF `QPdfWriter` (PySide6). Az **ISO 4217 adatfájl** és az **EKB referencia-árfolyam használati feltételei** a G3-on
ellenőrzendők (az EKB a referencia-árfolyamokat nyilvánosan, a forrás megjelölésével teszi közzé; a megjelenítés a forrást és a dátumot mutatja). **Kitsu/Zou (AGPL-3.0)**, **GnuCash (GPL-2.0-or-later)**:
csak funkcionális referencia (kód nincs átvéve). **Movie Magic Budgeting, Showbiz Budgeting, AICP**: funkcionális referencia a szerkezethez (összesítő lap, above/below the line), számlaszámokat, formátumot
és arculatot nem veszünk át.

## 16. G2-eredmény

**Javaslat: PASS.** Új döntési kérdés nincs. Hat G1-től eltérő pont indokolva (12. pont); a **legfontosabb megállapítás**, hogy a pénznemlista és az árfolyam-szolgáltatás (#15, #29) korábbi G2-kben hivatkozott, de nem tervezett
szolgáltatás volt: a **pénznemlista a G2-02 migrációjába kerül** (előfeltétel a 14., 19., 21. lépcsőhöz), az árfolyamletöltés és a költségvetés a 24. lépcsővel jön. A modul a 15. és 23. lépcső nélkül is használható; az
időlap-, eszköz- és forgatásnap-kapcsolat az érintett lépcsők után működik. A 13. pont kiegészítései megvalósításkor kerülnek a hivatkozott G2-kbe, új tábla nélkül (a pénznemlista kivételével, amely a G2-02 része lesz).
