# BacklotHub 26. lépcső – Archive: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-21 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.26 · **Előzmény:**
[G1 Archive](CFA4-BACKLOTHUB-G1-26-ARCHIVE-2026-11-07.md) (funkcionális követelmények jóváhagyva 2026-11-10) ·
**Döntések:** #18 (megőrzés), PORT-02 (az archívum megnyitható marad), SEC-05 (titkosítás), SEC-06 (személyes adat) · **Épül rá:**
G2-01 (Audit), G2-02 (produkció), G2-04 (Workflow Engine), G2-09 (útsablon), G2-12 (jelzések), minden korábbi modul adatmodellje,
File Services (ADR 0009, FS-1…FS-3) · **Állapot:** G2-javaslat, jóváhagyásra vár · implementáció nincs

Új döntési kérdés nincs. **Új függőség nincs** a termékben (a BagIt-írót és -ellenőrzőt saját kód adja; `bagit-python` csak tesztorákulum jelölt, G3).
A modul a **lezárt produkció hosszú távú megőrzése**; **nem azonos a mentéssel** (#12, #14), amely a folyamatban lévő munka helyreállítását szolgálja.
**Hat pont tér el a G1-től** vagy szűkíti azt, mind indokolva (13. pont); a legfontosabb: az **aktív fájlok felszabadítása** és az **üres rendszerbe való újraimport** nem M1.

## 1. Hatókör ebben a lépcsőben

- **Lezárhatósági ellenőrzés** (check), **archiválás** BagIt-csomagba (RFC 8493), **ellenőrzés** (gyors és teljes), **időszakos újraellenőrzés** jelzéssel.
- **Archív példányok** több archív gyökérben (lemezen), javítás másik példányból.
- **Produkció-metaadat** géptől és adatbázistól független formában (NDJSON + séma), **auditkivonat**, **fájlindex**.
- **Megőrzési szabály** (#18): a felhasználó dönt; lejáratkor csak jelzés, törlés kifejezett jóváhagyással.
- **Visszaállítás**: a produkció újranyitása és a fájlok **javítása** az archív példányból (részleges is).
- **Archív megnyitása adatbázis nélkül**: csak olvasható böngésző (inspector).
- **Nem része:** az aktív gyökerekből a fájlok **felszabadítása** (`files.relocate`, 26-2); a csomag **újraimportja üres rendszerbe** (26-2); felhő-feltöltés (SEC-02);
  alkalmazásszintű titkosítás (SEC-05: fájlrendszer-szint); termelési törlés (`production.purge` nincs, G2-02 5. pont: üzleti adat nem törlődik fizikailag).

## 2. Integrációs határok

| Határ | Irány | Szerződés |
|---|---|---|
| Production (2.) | ↔ | A produkció `archive_state` oszlopot kap (4.1); a `status` kategória `closed`/`cancelled` az archiválhatóság egyik feltétele; a produkció-szintű írásvédelem (4.2) |
| Workflow Engine (4.) | → | A csomag folyamata `archive_package_default` (a G2-04 `target_kind` listájában az `archive` már szerepel) |
| Minden modul (3–25.) | ← | **Archív-szolgáltató** (`ArchiveProvider`, 5. pont): a modul a saját, produkcióhoz kötött tábláit NDJSON-ként adja ki |
| Audit (1.) | ← | `audit.export {scope.production_id}` (JSON, hash-ekkel), `audit.verify` az archiválás előtt (G2-01 7. pont) |
| File Services (ADR 0009) | ↔ | `files.import` + `files.seal` (a csomag fájljai az `archive` gyökérbe), `files.verify`, `files.resolve`; **FS-3 `restore`** a javításhoz; **bővítések:** `extra_hashes` (SHA-256), `files.export` (G2-25), későbbi `files.relocate` (26-2) |
| Document (14.), Delivery (25.) | ← | Dokumentumok (az `attachment` gyökérből), a kiadási jegyzékek és QC-jelentések |
| Notification (12.) | → | Események és `deadline_signal` (11. pont) |
| Search (7.) | → | Az archivált produkció **marad kereshető**, `archive_state` jelöléssel és szűrővel |
| Person (5.), Crew (18.) | ↔ | A személyes adat kezelése (7. pont); a törlési kérés hivatkozás-szolgáltatója a csomagokat is kiírja |
| Dashboard (6.) | → | Az archivált produkció az aktív nézetekből kimarad, külön számláló |
| Audit (1.) | → | Minden lépés auditált |

## 3. Archiválási folyamat

### 3.1 Lépések

| Lépés | Mi történik | Hiba esetén |
|---|---|---|
| **Check** (`backlot.archive.check`) | A lezárhatósági lista (4. pont) kiértékelése; eredménye a csomag `check_report`-ja | Semmi nem változik |
| **Freeze** | Egy tranzakcióban: `archive_state = 'archiving'` (írásvédelem, 4.2), a csomag `building` állapotba, a **metaadat-pillanatkép** (egy `REPEATABLE READ` tranzakció, 5. pont) | Visszagörgetés, `archive_state` marad |
| **Stage** | A fájlok és a metaadat a célgyökér `<bag>.INCOMPLETE/` könyvtárába, fájlonként ellenőrzött másolással (6. pont); a haladás a `.cfa4-archive-progress` fájlban | `.INCOMPLETE` marad, folytatható; a produkció `archiving` állapota feloldható `backlot.archive.cancel`-lel |
| **Seal tags** | `bag-info.txt`, `manifest-sha256.txt`, `tagmanifest-sha256.txt`, `cfa4-archive.json` | mint fent |
| **Verify** | A **kész csomag teljes újraolvasása** a célról (SHA-256), a jegyzék és a `Payload-Oxum` ellen (6.3) | `failed`; a csomag nem lesz `archived` |
| **Commit** | Atomikus átnevezés `<bag>.INCOMPLETE` → `<bag>`; egy tranzakcióban: `archive_state = 'archived'`, a csomag `archived`, a másolat (`archive_copy`) és a megőrzés rögzítése, auditesemény, kimenő esemény | A csomag `archived` **csak** ellenőrzés után |

- **Egy időben egy építés** (IO-kímélés): `ARCHIVE_BUSY`; a sorban álló építés várakozik. A munka **háttérfeladat**, megszakítható; a megszakítás takarít és feloldja az írásvédelmet.
- **Folytatás:** indításkor a `building` állapotú csomag felismerhető; a felhasználó folytathatja (a már ellenőrzött fájlok megmaradnak, ha a másolatuk még egyezik) vagy elvetheti.
- **Becslés:** a felület a bájtszámból és a **CFA4-MIN gépen mért átviteli sebességből** (G5) ad időbecslést; ígéret nincs.
- **Helyigény:** a Freeze előtt ellenőrzés: a célgyökér szabad helye ≥ a becsült méret + 5% (`FILES_DISK_FULL` jellegű `ARCHIVE_TARGET_FULL`).
- A **csomag neve/útja** az `archive` célú útsablonból (G2-09 `path_template`, `purpose = 'archive'`): alapértelmezés `{production.code}/ARC-{package.number:03}`.

### 3.2 Folyamat

`archive_package_default` (G2-04, zárt őrfeltételekkel; a jogosultság parancsszintű): `building → verifying → archived`; `archived → superseded` (új csomag készült ugyanarra a produkcióra);
`archived | superseded → retired` (a felhasználó döntése, 8. pont) `→ destroyed`; `building | verifying → failed | cancelled`. Egy produkcióhoz **egyszerre egy `archived` csomag** tartozik (a legújabb); a korábbiak `superseded`
(megmaradnak a megőrzésük végéig).

## 4. Lezárhatósági ellenőrzés

| Szabály | Feltétel | Súly | Lemondható |
|---|---|---|---|
| `production_closed` | A produkció státuszkategóriája `closed` vagy `cancelled` (G2-02) | blokkoló | nem |
| `no_open_tasks` | Nincs nyitott feladat (`category` nem `closed`/`cancelled`), vagy a felhasználó kifejezetten megerősítette | blokkoló | **igen**, indoklással |
| `deliveries_settled` | Minden átadási csomag `accepted` vagy `cancelled` (G2-25) | blokkoló | igen |
| `versions_sealed` | Minden archiválandó verzió lezárt (`published_at` nem NULL, a fájlok `sealed`) | blokkoló | nem |
| `files_verified` | Az archiválandó fájlok `files.verify` szerint épek, az eredeti gyökér elérhető | blokkoló | **nem** |
| `audit_verified` | `audit.verify` hibátlan a produkció időszakára | blokkoló | **nem** |
| `no_running_jobs` | Nincs futó publikálás, QC, proxy, kiadás vagy importfolyamat a produkcióban | blokkoló | nem |
| `open_timesheets` | Van nem jóváhagyott időlap | figyelmeztetés | – |
| `single_copy` | Csak egy archív célgyökér van kiválasztva | figyelmeztetés | – |

- A **nem lemondható** szabályok az integritást védik: sérült fájlt vagy hibás láncot nem archiválunk; előbb a hiba kezelendő (mentésből visszaállítás, G2-01/FS-3).
- Az ellenőrzés **számolt**, bármikor futtatható; a `check_report` a csomagnál rögzül (a lemondások indoklásával).

### 4.1 A produkció archiválási állapota

```sql
ALTER TABLE backlot.production
    ADD COLUMN archive_state text NOT NULL DEFAULT 'active' CHECK (archive_state IN ('active','archiving','archived','restoring')),
    ADD COLUMN archived_at timestamptz;
```

- A **kereskedelmi státusz** (`status_id`, G2-02) és az **archiválási állapot** két külön tengely: egy lezárt produkció még nem archivált; egy archivált produkció újranyitható anélkül, hogy a kereskedelmi státusza megváltozna.
- A `restoring` a javítás közbeni állapot (9. pont); alatta az írás szintén védett.

### 4.2 Írásvédelem

- **Szolgáltatásréteg:** az archivált produkció minden írása `PRODUCTION_ARCHIVED` hibakóddal elutasított (kivéve az archív modul saját parancsai és a megjegyzés/kereső-karbantartás… **nincs kivétel az üzleti adatra**).
- **Adatbázis-védelem:** minden `production_id` oszlopot hordozó táblára **generált trigger** (`backlot.assert_production_writable()`): ha a produkció `archive_state` értéke nem `active`, a módosítás elutasított, **kivéve** ha a tranzakció
  az archív szolgáltatás `SET LOCAL backlot.archive_actor = 'on'` jelzését viseli (csak az `archive_*` parancsok állíthatják; a szerepkör-jog erre külön). A triggerek listája **nem kézi**: a migráció a katalógusból (`information_schema`) állítja elő, és
  **CI-teszt ellenőrzi**, hogy minden `production_id`-s táblának van triggere (egy új modul tábláját a teszt elbuktatja, ha kimarad).
- **Az olvasás, a keresés, az export** archivált produkción is működik. **A `cfa4_files` katalógus** írásai a BacklotHub parancsain át történnek (ugyanabban a tranzakcióban az üzleti sorral), ezért az archivált produkcióra nem íródnak.
- **A feloldás:** `backlot.archive.reopen` (9. pont), jogosultsággal és indoklással, auditálva.

## 5. Metaadat-export (archív-szolgáltatók)

- **`ArchiveProvider` regiszter** (kódban, mint a `SearchProvider`): minden modul regisztrál egyet, amely megadja (a) a **táblák listáját**, (b) a produkció soraira az **`export`** iterátort, (c) a **séma-azonosítót** (`cfa4.backlot.archive.<modul>/<n>`),
  (d) mely táblák **személyes adatot** tartalmaznak (`personal: true`), (e) mely **globális hivatkozott táblák** (pl. `person`, `organization`, `resource`, `team`, `currency`, `equipment`) sorai kellenek **csak a hivatkozott sorokra** korlátozva.
- **Kimenet** (a csomag `data/metadata/` alatt): táblánként egy **NDJSON** fájl (UTF-8, soronként egy JSON-objektum; UUID szöveg; időbélyeg ISO 8601 UTC; **`numeric` tizedes karakterláncként, nem lebegőpontosan**, a G2-14 `money` elve szerint), a
  szolgáltató `schema.json`-ja (az oszlopok neve, típusa, a `information_schema` szerint az építés pillanatában, és a szolgáltató séma-azonosítója), és a `data/metadata/index.json` (szolgáltató → táblák → sorszám → SHA-256).
  A pillanatkép **egy `REPEATABLE READ` tranzakcióban** készül (a freeze után), így konzisztens.
- **Teljességi garancia (a „minden adatmodell véglegesítve” feltétel ellenőrizhetősége):** a CI-teszt **kigyűjti az összes `production_id` oszlopú táblát**, és mindegyikről megköveteli, hogy valamelyik szolgáltató lefedje, vagy **indokolt kizárási listán** szerepeljen
  (operatív, újraszámolható vagy származtatott tábla: `search_vector`-ok, `event_outbox`, `notification`, `event_consumed`, `deadline_signal`, `capacity_signal`, `time_timer`, ideiglenes munkaállapotok). Egy új modul tábla nélkül **nem tud kihagyódni** észrevétlenül.
- **A fájlindex** (`data/metadata/files.ndjson`): minden payload-fájl katalógussora (azonosító, gyökér-független út, méret, BLAKE3 és SHA-256, lezárási idő, a hivatkozó verzió/dokumentum azonosítója); ez köti a metaadatot a fájlhoz.
- **Auditkivonat** (`data/audit/audit-extract.json`): az `audit.export` kimenete a `scope.production_id` szűrővel (esemény, `seq`, `prev_hash`, `hash`; a személyes értékek redaktáltak, G2-01). **Korlát (kimondva a csomag leírásában és a felületen):**
  a lánc **globális**, ezért a kivonat **eseményenként ellenőrizhető** (az esemény hash-e a tartalomból és a `prev_hash`-ből újraszámolható), de a **folytonosság** (hogy nincs kihagyott esemény) a teljes lánc hash-ellenőrzőpontjaival bizonyítható; a csomag rögzíti az
  érintett szakasz **határoló hash-eit** (a legkorábbi esemény `prev_hash`-e és a legutolsó `hash`) és az archiváláskori `audit.verify` eredményét (`seq`-tartomány, siker). Más produkció eseménye **nem** kerül bele.
- **Olvasható dokumentáció a csomagban:** `README.txt` (angol és magyar, a formátum leírása, az ellenőrzés `sha256sum -c`-vel), `cfa4-archive.json` (formátumverzió, szolgáltatók és séma-verziók, a CFA4 verziója, `audit.verify`-eredmény, beállítások).

## 6. A BagIt-csomag

### 6.1 Szerkezet (RFC 8493)

```
<bag>/
  bagit.txt                    BagIt-Version: 1.0 · Tag-File-Character-Encoding: UTF-8
  bag-info.txt                 Bagging-Date, Payload-Oxum, Bag-Size, External-Identifier, CFA4-Archive-Format, CFA4-Production-Code, …
  manifest-sha256.txt          a payload (data/) összes fájlja
  tagmanifest-sha256.txt       a címkefájlok (bagit.txt, bag-info.txt, manifest, cfa4-archive.json, README.txt)
  cfa4-archive.json · README.txt
  data/
    files/<gyökér-fajta>/<gyökér-független út>      a katalógus fájljai (publish, attachment, opcionálisan work)
    metadata/<szolgáltató>/<tábla>.ndjson + schema.json + index.json · files.ndjson
    audit/audit-extract.json
```

- **Kivonat:** SHA-256 (BagIt-regisztrált, minden eszközzel ellenőrizhető: `sha256sum -c manifest-sha256.txt`); a katalógus **BLAKE3** értéke külön, a `files.ndjson`-ban. **Mindkettő egy olvasásból:** a File Services `files.import`/`files.export`
  a `extra_hashes: ["sha256"]` paraméterrel a második kivonatot **ugyanabban az olvasási menetben** számolja (**FS-2 bővítés**), így a TB-os fájl nem olvasódik kétszer. A Xeon E5 v4-en nincs SHA-hardvergyorsítás, ezért az építés **fájlonként párhuzamos**
  (a munkaszálak száma beállítható, alapból a magok fele), a mérés a G5-ben (1 TB, CFA4-MIN).
- **Útvonalak:** a jegyzék a RFC 8493 szerinti **százalékos kódolást** alkalmazza (`%`, CR, LF a fájlnévben), a bejegyzés `<hash>␣␣<út>`; a fájlnevek **bájtra pontosan megmaradnak** (nincs Unicode-normalizálás), a katalógus UTF-8 szövege az út. **Nem képviselhető útvonal**
  (túl hosszú komponens: 255 bájtnál több, teljes út 4096-nál hosszabb, üres vagy `.`/`..` komponens, két fájl azonos úton) **az építés előtt** listázva (`ARCHIVE_PATH_UNSUPPORTED`), az építés nem indul.
- **Payload-Oxum** (`bájt.fájlszám`) a gyors ellenőrzéshez.
- **A csomagfájlok a File Services katalógusában**: a payload és a címkefájlok az `archive` gyökérben **egyenként katalogizált, lezárt (`seal`) fájlok** (ADR 0009: a BacklotHub nem másol és nem töröl a lemezen, a katalógus minden fájlt ismer), így a File Services `verify`
  és kétlépcsős törlési szabálya az archívumra is él. Ennek ára a katalógus-sorok megkettőződése (a mérésben látszik, G5); egy későbbi optimalizáció (könyvtárszintű katalógusobjektum) **File Services-döntés**, nem ez a lépcső.
- **Szerializálás:** M1-ben **könyvtár-bag**; tar/zip szerializálás (külső hordozóra) nincs.

### 6.2 Mi kerül a csomagba

| Tartalom | Alapból | Beállítás |
|---|---|---|
| Közzétett, lezárt verziók és reprezentációk fájljai (`publish` gyökér) | igen | – |
| Dokumentumok és csatolmányok (`attachment` gyökér), köztük a kiadási jegyzékek és QC-jelentések | igen | – |
| Munkafájlok (`work`) | **nem** | `include_workfiles` (méret!) |
| Proxyk, bélyegképek, review-médiák (`review`) | **nem** (a Media Services újra előállítja, ADR 0009) | `include_review` |
| Produkció-metaadat, fájlindex, auditkivonat | igen | – |
| **Személyes adat** (az adatbázis `personal` táblái) | **nem** | `include_personal_data` (7. pont) |

### 6.3 Ellenőrzés

- **Gyors** (`quick`): `bagit.txt`, címkefájlok (`tagmanifest`), a payload fájlok **megléte és mérete**, a `Payload-Oxum`; perc nagyságrend, olvasás nélkül (a tartalmat nem hash-eli).
- **Teljes** (`full`): minden payload- és címkefájl SHA-256-ja a jegyzék ellen, **a BacklotHubtól és a katalógustól független** ellenőrzővel (a standard `hashlib`), valamint a File Services `files.verify` a katalógus-kivonatra. Eredmény: ép / eltérések listája (fájl, várt, kapott).
- A teljes ellenőrzés **IO-igényes** (TB-os): háttérfeladat, megszakítható, folytatható (a már ellenőrzött fájlok listája megmarad), a felület becslést mutat.
- **Ellenőrzési napló:** minden futás rögzül (`archive_verification`, 10. pont), az eredmény a csomag állapotához hozzátartozik (számolt: az utolsó teljes ellenőrzés eredménye és ideje).

## 7. Személyes adat (SEC-06) és az archívum

- **Alapból nincs személyes adat az archívumban:** az adatbázis `personal` jelölésű táblái (`person_personal_detail`, `cast_personal_detail`, a személyhez kötött díjsorok, G2-18/19/24) **nem kerülnek** az NDJSON-ba. Ezek a **jelenlegi** elérhetőségek és pénzügyi azonosítók,
  amelyek az archivált produkció olvasásához nem szükségesek, és amelyeknek a törlése a G2-18 szerint kötelező, **az archívumban viszont nem lenne törölhető** (a lezárt csomag nem módosítható).
- **A megmaradó személyes vonatkozás:** a **dokumentumok** (szerződések, nyilatkozatok), amelyek a produkció jogi nyilvántartásai, és a `person.display_name` (művésznév, G2-18). Ezek bekerülnek; a csomag `contains_personal_data` jelölést kap, ha tartalmaz ilyen dokumentumot
  (`document_type.contains_personal_data`, G2-14), és az `archive_person_ref` táblában **rögzül, mely személyek szerepelnek** a személyes adatot hordozó dokumentumok metaadatában (`person_ref`) és a megnevezett `person`-sorokban.
- **`include_personal_data = true`**: a személyes táblák is bekerülnek (külön `data/metadata/_personal/` alatt), a csomag jelölt, **és a felület figyelmeztet**, hogy a törlési kérés ezután csak újraarchiválással teljesíthető. Csak `owner` választhatja, indoklással, auditálva.
- **Törlési kérés** (`person.anonymize`, G2-18 5.3): a hivatkozás-szolgáltatóhoz az Archive regisztrál egyet: a jelentés kiírja az `archive_person_ref` szerinti **csomagokat**. A rendszer **a lezárt csomagot nem módosítja**; a teljesítési mód: **újraarchiválás** a személy adata nélkül
  (az új csomag `superseded`-re teszi a régit, amelynek **megsemmisítése** a 8. pont szerint, `owner` jóváhagyásával történik). Hogy az archív másolat megtartható-e jogi alapon (számviteli, jogtulajdoni kötelezettség), **jogi döntés** (G2-18 5.3/4): a rendszer a lehetőséget adja, a döntést nem hozza meg.
- **A fájlok maguk** (szerződés-PDF) személyes adatot tartalmaznak; erről a csomag leírása kimondja (nincs tartalom-elemzés).
- **Titkosítás (SEC-05):** fájlrendszer-szinten (LUKS/fscrypt), nem alkalmazásszinten; a felület jelzi, ha az archív gyökér titkosított köteten van-e; külső hordozóra az archiválás előtt a felhasználó felel a titkosításért.

## 8. Megőrzés (#18) és megsemmisítés

```sql
CREATE TABLE backlot.archive_package (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    number integer NOT NULL CHECK (number > 0), code text NOT NULL,
    status_id uuid NOT NULL REFERENCES backlot.status(id),              -- archive_package_default (G2-04)
    options jsonb NOT NULL,                                               -- include_workfiles, include_review, include_personal_data, szálak
    check_report jsonb NOT NULL,
    format_version text NOT NULL,                                         -- cfa4.backlot.archive/1
    cfa4_version text NOT NULL,
    file_count bigint, total_bytes bigint, manifest_sha256 text, tagmanifest_sha256 text,   -- a kész csomagról
    contains_personal_data boolean NOT NULL DEFAULT false,
    audit_from_seq bigint, audit_to_seq bigint, audit_verify_ok boolean,
    retention_mode text NOT NULL CHECK (retention_mode IN ('keep','until')),
    keep_until date,
    archived_at timestamptz,
    ...közös oszlopok,
    UNIQUE (production_id, number),
    CHECK ((retention_mode = 'until') = (keep_until IS NOT NULL))
);

CREATE TABLE backlot.archive_copy (
    id uuid PRIMARY KEY, package_id uuid NOT NULL REFERENCES backlot.archive_package(id),
    root_id uuid NOT NULL,                                                -- cfa4_files archív gyökér
    relative_dir text NOT NULL,
    state text NOT NULL CHECK (state IN ('ok','unverified','damaged','offline','removed')),
    last_verified_at timestamptz,
    ...közös oszlopok, UNIQUE (package_id, root_id)
);

CREATE TABLE backlot.archive_verification (
    id uuid PRIMARY KEY, package_id uuid NOT NULL REFERENCES backlot.archive_package(id),
    copy_id uuid REFERENCES backlot.archive_copy(id),
    level text NOT NULL CHECK (level IN ('quick','full')),
    state text NOT NULL CHECK (state IN ('running','ok','failed','cancelled')),
    started_at timestamptz NOT NULL, ended_at timestamptz,
    files_checked bigint, bytes_checked bigint,
    errors jsonb NOT NULL DEFAULT '[]',                                   -- [{path, expected, actual, code}], legfeljebb 1000
    ...közös oszlopok
);

CREATE TABLE backlot.archive_person_ref (
    package_id uuid NOT NULL REFERENCES backlot.archive_package(id),
    person_id uuid NOT NULL REFERENCES backlot.person(id),
    via text NOT NULL CHECK (via IN ('document','personal_table','display_name')),
    PRIMARY KEY (package_id, person_id, via)
);
```

- **A megőrzés a felhasználó döntése** (#18): az archiváláskor **kötelező választás**: `keep` (amíg a felhasználó mást nem dönt) vagy `until` (dátum). **Nincs alapértelmezett időtartam** (a 10 év csak javaslat volt, a döntés a felhasználóé). A beállítás módosítható (auditált).
- **Lejárat:** `keep_until` után **csak figyelmeztetés** (`deadline_signal`, `archive_retention_due`), nincs automatikus törlés. A **megsemmisítés** kétlépcsős: `backlot.archive.retire` (a csomag `retired`, még visszaállítható állapotba tehető) → `backlot.archive.destroy` (**`owner` kifejezett jóváhagyása**,
  auditálva, **minden másolatra**, a File Services kétlépcsős törlésével, majd `purge` a jóváhagyás után). **Megsemmisítés után a csomag nem nyitható**, a produkció adatbázis-metaadata megmarad (nincs `production.purge`).
- **Amíg megvan, az archívum megnyitható és visszaállítható marad** (PORT-02): a formátumverzió és az olvasók (10. pont) ezt biztosítják.
- **A megsemmisítés nem érinti a más produkcióhoz tartozó adatot**; a csomag teljesen önálló (a más produkciók adata nem kerül be, 5. pont).

## 9. Visszaállítás (M1: újranyitás és fájljavítás)

A G1 „visszaállítás” fogalmát **kettéválasztjuk**, mert az adatbázis-sorok az archiválás után **az adatbázisban maradnak** (csak olvashatók, G1: „a keresés továbbra is megtalálja”), és a fájlok M1-ben **nem költöznek el** az aktív gyökerekből (13. pont/1):

| Művelet | Mit csinál | M1 |
|---|---|---|
| **Újranyitás** (`backlot.archive.reopen`) | A produkció `archive_state`-je `archived → active`; a csomag `archived` marad (a korábbi állapot pillanatképe); a következő archiválás új csomag, amely a régit `superseded`-re teszi | **igen** |
| **Fájljavítás** (`backlot.archive.repair`) | A produkció fájljai közül a `files.verify` szerint **hiányzó vagy sérült** fájlokat az archív példányból állítja vissza (FS-3 `restore`), **a katalógus-kivonat ellen ellenőrizve**; ép fájlt **soha nem ír felül**; a fájlazonosító (UUIDv7) változatlan | **igen** (FS-3 után) |
| **Részleges javítás** | Ugyanez egy kiválasztott körre (termék, verzió, dokumentum, szekvencia) | **igen** |
| **Másik példányból javítás** | Egy sérült archív példányt a másik ép példányból állít helyre (`backlot.archive.copy.repair`), a jegyzék ellen | **igen** |
| **Üres rendszerbe importálás** | A csomag metaadatának (NDJSON) betöltése új adatbázisba | **nem** (26-2, 13. pont/2) |
| **Aktív fájlok felszabadítása** | A fájlok eltávolítása az aktív gyökerekből, a katalógus áthelyezésével | **nem** (26-2, 13. pont/1) |

- A javítás `archive_state = 'restoring'` alatt történik (írásvédett), a végén **automatikus `files.verify`** a javított részre (FS-3); eredménye a jelentésben. A javítás **auditált**, a forrás a jegyzékkel ellenőrzött bag-fájl.
- **A visszaállítás a produkció üzleti adatát nem változtatja**: az adatbázis-sorok az archiválás utáni állapotban vannak; ha az adatbázis elveszett, az adatbázis-mentés (#12) a forrás, az NDJSON **olvasásra** szolgál.

### 9.1 Az archív megnyitása adatbázis nélkül (inspector)

- `backlot.archive.inspect {path}`: egy **tetszőleges** (akár külső lemezen lévő, akár más gépről érkező) BagIt-csomagot megnyit **csak olvasásra**: ellenőrzi a gyors vagy teljes jegyzéket, kiírja a leírást (`cfa4-archive.json`), a fájlfát (a `files.ndjson` alapján), a
  szolgáltatók és táblák listáját, és a **táblák tartalmát lapozva** (az NDJSON sorfolyam-olvasása, nem tölti be egészben). **Adatbázis nem kell.** A régebbi formátumverziót a kódbeli **verziólépcső** (`migrate_vN_to_vN+1` a szolgáltatónkénti séma-azonosítókra) hozza az
  aktuális olvasóalakra (PORT-02), **az eredeti fájl változatlan marad**.
- Az inspector **nem importál** és nem ír semmit a katalógusba.

## 10. Időszakos ellenőrzés és több példány

- **Ütemezés** (beállítások, felhasználói szint): `quick` alapból havonta, `full` alapból évente; a lejárat **jelzés** (`deadline_signal`, `archive_verify_due`), a futtatás **felhasználói indításra** (vagy a „automatikus indítás” bekapcsolásával, alapból ki: a TB-os olvasás
  nem indulhat a felhasználó tudta nélkül); az alkalmazás nem fut háttérdémonként, ezért a lejárat az indításkor és naponta egyszer (futás közben) értékelt.
- **Hiba:** `damaged` példány, esemény (`backlot.archive.verification_failed`), a felület **javítást ajánl a másik példányból**, ha van ép; ha egy példány sincs ép, kimondva és eszkalálva (a mentés, #14).
- **Több példány** (`archive_copy`): egy csomagot több **archív gyökérre** is másol (`backlot.archive.copy.add`: ellenőrzött másolás a File Services-szel, majd teljes ellenőrzés a jegyzékkel). Az egyetlen példány `single_copy` figyelmeztetés (nem hiba).
  Offline gyökér (kihúzott lemez): `FILES_ROOT_UNAVAILABLE`, a példány `offline`, nem `damaged`.

## 11. Parancsok, hibakódok, események

| Parancs | Mit csinál |
|---|---|
| `backlot.archive.check` | Lezárhatósági lista (4.), lemondási javaslattal |
| `backlot.archive.build` / `.cancel` / `.resume` / `.discard` | Építés (3.): `options`, célgyökerek, megőrzés (kötelező); háttérfeladat |
| `backlot.archive.package.list` / `.get` | Csomagok, állapot, példányok, utolsó ellenőrzés |
| `backlot.archive.verify` | `quick` vagy `full` ellenőrzés csomagra/példányra (6.3, 10.) |
| `backlot.archive.copy.add` / `.repair` / `.remove` | További példány; javítás másik példányból; eltávolítás (az utolsó példányé tiltott) |
| `backlot.archive.reopen` | Újranyitás (9.), indoklással |
| `backlot.archive.repair` | Fájljavítás (9.), körrel |
| `backlot.archive.retention.set` | Megőrzés módosítása (8.) |
| `backlot.archive.retire` / `.destroy` | Megsemmisítés két lépcsőben (8.), `owner` |
| `backlot.archive.inspect` | Csomag megnyitása adatbázis nélkül (9.1) |
| `backlot.archive.settings.get` / `.set` | Ellenőrzési ütemezés, szálak, alapbeállítások |

Hibakódok: `ARCHIVE_NOT_ARCHIVABLE` (blokkoló szabály), `ARCHIVE_BUSY`, `ARCHIVE_TARGET_FULL`, `ARCHIVE_PATH_UNSUPPORTED`, `ARCHIVE_PACKAGE_NOT_FOUND`, `ARCHIVE_PACKAGE_STATE_INVALID`, `ARCHIVE_VERIFY_FAILED`,
`ARCHIVE_COPY_INVALID`, `ARCHIVE_LAST_COPY`, `ARCHIVE_REPAIR_FAILED`, `ARCHIVE_FORMAT_UNSUPPORTED` (ismeretlen vagy túl új formátumverzió), `ARCHIVE_RETENTION_INVALID`, `ARCHIVE_DESTROY_NOT_APPROVED`,
`PRODUCTION_ARCHIVED` (írásvédelem, 4.2), `CONFLICT_STALE_VERSION`; valamint a `FILES_*`, `AUDIT_*`, `WORKFLOW_*` kódok változatlanul.

**Események** (outbox, G2-12): `backlot.archive.package_archived`, `.verification_failed`, `.repaired`, `.reopened`, `.destroyed`; **időzített jelzések** (`deadline_signal`): `archive_verify_due`, `archive_retention_due` (`subject_type = 'archive_package'`). Új címzett nincs (`production_owner`, `watcher`).

## 12. Never crash

- **Megtelt lemez, offline gyökér, jogosultsági hiba, forrásfájl-eltérés építés közben** (`FILES_*`): az építés `failed`, a `.INCOMPLETE` marad (folytatható), az **írásvédelem feloldható**, a produkció **nem ragad** `archiving` állapotban (a megszakítás és az indításkori helyreállítás feloldja); hibás állapotot nem hagy.
- **Összeomlás az építés közben:** indításkor a `building`/`verifying` csomag **felismert**, a felhasználó folytathatja vagy elvetheti; a katalógus **soha nem mutat félkész csomagra** (a `archived` állapot és a példány rögzítése a végső átnevezés után, egy tranzakcióban); a lezárt fájlok a megszakadt építésnél `deleted` állapotba kerülnek.
- **Ellenőrző hibái** (hiányzó/sérült fájl, hibás jegyzék, csonka csomag): az ellenőrzés **jelent**, nem dob kezeletlen kivételt, nem javít csendben; egy sérült fájl nem állítja meg a többi ellenőrzését (az összes eltérés listázva, 1000-ig).
- **Írásvédelem:** az archivált produkció írása hibakóddal elutasított, **nem** adatbázis-kivétel az alkalmazásban; a trigger-hiba (`PRODUCTION_ARCHIVED`) kezelt.
- **Ismeretlen vagy túl új formátumverzió** az inspectorban: `ARCHIVE_FORMAT_UNSUPPORTED`, a fájlok ellenőrzése attól még lehetséges (a BagIt-réteg verziófüggetlen).
- **Rendkívüli méret:** milliós fájlszám, TB-os csomag: sorfolyam-feldolgozás (nincs egészben betöltés), korlátos memória, folytatható munka.
- **Megsemmisítés:** csak kifejezett jóváhagyással; megszakadás után folytatható és auditált; az utolsó példány eltávolítása `ARCHIVE_LAST_COPY`, kivéve a `destroy` lépést.

## 13. Eltérések a G1-től (összegzés)

| # | G1 | G2-javaslat | Indok |
|---|---|---|---|
| 1 | Archiválás után a fájlok **eltávolíthatók az aktív gyökerekből** | **M1-ben nincs** felszabadítás; a csomag **másolat**, az eredeti marad; felszabadítás a 26-2-ben (`files.relocate`) | A hivatkozott lezárt fájl nem törölhető (ADR 0009, `RESTRICT`); az eltávolítás a katalógus-áthelyezést kéri a File Services-ben, amely nincs a tervben. Biztonságos alap: nincs adatvesztési út |
| 2 | „Visszaállítás”, részleges is | **Újranyitás + fájljavítás** (részleges is); **üres rendszerbe importálás 26-2** | Az adatbázis-sorok megmaradnak; a teljes újraimport szolgáltatónkénti importálót kér (séma-lépcsővel); a katalógus-hivatkozások és az audit-lánc újraépítése külön terv |
| 3 | A csomag „fájlai” | **Fájlonként katalogizált, lezárt** fájlok az `archive` gyökérben | ADR 0009: a BacklotHub nem másol és nem töröl; a File Services-szabályok (verify, törlés) élnek |
| 4 | „Kapcsolódó metaadatok” | **Archív-szolgáltató regiszter + teljességi CI-teszt**, **személyes adat alapból nincs** | Kimaradó tábla észrevétlenül nem lehet; a lezárt csomagból nem törölhető személyes adat |
| 5 | Jelölt függőség: `bagit-python` | **Saját BagIt-író és -ellenőrző** (standard könyvtár); `bagit-python` tesztorákulum jelölt (dev-függőség, G3) | A formátum kicsi; a fájlonkénti katalogizálás és az egy menetes kettős kivonat saját folyamatot kér |
| 6 | (nem szerepelt) | **Írásvédelem generált triggerrel**, CI-ellenőrzéssel | „A metaadat csak olvasható állapotban marad”: egyetlen szolgáltatásrétegbeli ellenőrzés kimaradhat |

## 14. Sorrend és függés

| Rész | Kell hozzá |
|---|---|
| Lezárhatósági ellenőrzés, írásvédelem, archív-szolgáltatók | **minden korábbi adatmodell véglegesítve** (G1 előfeltétel; a teljességi teszt ellenőrzi), G2-02 (`archive_state`) |
| BagIt-építés és -ellenőrzés | File Services FS-1 (`import`, `seal`, `resolve`), FS-2 (`verify`, **`extra_hashes` bővítés**) |
| Fájljavítás | File Services **FS-3** (`restore`) |
| Auditkivonat | G2-01 (`audit.export`, `audit.verify`) |
| Megőrzés és jelzések | 12. lépcső |
| Felszabadítás, üres rendszerbe importálás | **26-2**: File Services `files.relocate`; szolgáltatónkénti importálók |

A G1 „Előfeltétel: File Services (FS-3 is); minden adatmodell véglegesítve” **megmarad**: az FS-3 a fájljavításhoz kell (a csomagolás FS-3 nélkül is működik), a véglegesítés pedig a CI-teljességi teszttel **mérhető** (nem csupán állítás).

## 15. Tesztek (G5-re előre)

- **BagIt-megfelelőség:** a csomag megfelel az RFC 8493-nak (`bagit.txt`, `bag-info.txt`, jegyzékek, `Payload-Oxum`); **`sha256sum -c`** futtatva a jegyzéken; a `bagit-python` mint külső orákulum (dev-függőségként, G3), ha elfogadott; százalékos kódolás (`%`, CR, LF nevű fájl),
  Unicode-nevek, üres fájl, üres könyvtár (BagIt nem kezeli: nem kerül be), nem képviselhető útvonalak (`ARCHIVE_PATH_UNSUPPORTED`, az építés előtt).
- **Teljességi tesztek (CI):** minden `production_id`-s tábla lefedett szolgáltatóval vagy indokolt kizárással; minden ilyen táblán van írásvédelmi trigger.
- **Írásvédelem:** minden `production_id`-s táblára írási kísérlet archivált produkción elutasítva (a katalógusból generált teszt); az archív parancsok `archive_actor`-ral átmennek, más nem; olvasás, keresés, export működik; az újranyitás feloldja.
- **Építés:** sikeres kis és közepes produkció; **hibainjektálás** minden lépés közben (megtelt lemez, offline gyökér, forrás-kivonateltérés, összeomlás a Stage/Verify/Commit között): `.INCOMPLETE` folytatható, nincs félkész `archived`, az írásvédelem feloldható; a katalógus nem mutat félkész csomagra; megszakítás.
- **Metaadat:** NDJSON formátum (UTF-8, tizedes karakterláncként), a pillanatkép konzisztenciája (párhuzamos írás az építés alatt nem keveredik bele: a freeze után nincs írás), séma-fájlok, `index.json` hash-ek, fájlindex a katalógussal egyezik.
- **Személyes adat:** alapból a személyes táblák tartalma **nincs** a csomagban (kanári-értékekkel ellenőrizve minden fájlban); `include_personal_data` csak `owner`-nek; `archive_person_ref` kitöltése; a törlési hivatkozás-jelentés kiírja a csomagokat; az újraarchiválás után a régi megsemmisítése.
- **Auditkivonat:** csak az adott produkció eseményei, eseményenként újraszámolható hash, határoló hash-ek, az `audit.verify` eredménye rögzítve; más produkció adata nincs.
- **Ellenőrzés:** gyors (hiányzó fájl, csonkolt fájl, `Payload-Oxum` eltérés) és teljes (bitsérülés egy fájlban: megtalálja, a többit ellenőrzi); független a katalógustól (a katalógust szándékosan hibássá téve is jelent); folytatható; megszakítható.
- **Példányok:** másik példány hozzáadása, javítás a másikból, `offline` vs `damaged`, az utolsó példány eltávolításának tiltása.
- **Visszaállítás:** újranyitás; fájljavítás (hiányzó, sérült, ép marad: soha nem ír felül), részleges kör, a fájlazonosító változatlan, automatikus `files.verify`, forrás-kivonat egyezése a katalógussal.
- **Megőrzés:** kötelező választás archiváláskor, `keep_until` jelzés, **nincs automatikus törlés**, `retire` → `destroy` jóváhagyással minden példányra, megsemmisítés utáni nyithatatlanság.
- **Inspector:** csomag megnyitása DB nélkül (külső útról), régebbi formátumverzió a verziólépcsőn át, túl új verzió (`ARCHIVE_FORMAT_UNSUPPORTED`), nagy NDJSON lapozva, az eredeti fájl változatlan.
- **Never crash:** fuzzolt jegyzék, hibás `bag-info.txt`, csonka NDJSON-sor, hibás `cfa4-archive.json`.
- **Felületi ellenőrzés:** `ui.screenshot` a lezárhatósági listáról, a csomag részleteiről, az ellenőrzési előzményekről, a megőrzési párbeszédről, az inspectorról.
- **Mérés a CFA4-MIN gépen:** 1 TB-os produkció archiválása (másolás + SHA-256 + teljes ellenőrzés idő), a katalógus-megkettőződés (soronkénti méret, 200 000 fájl), a `quick` és `full` ellenőrzés ideje, a metaadat-export ideje 52 produkciós adatkészleten (SC-01).

## 16. Licenc (G3-ra előre)

Új termékfüggőség nincs: `hashlib`, `json`, `unicodedata`, `os`, a Qt `QPdf*` nem kell. **BagIt (RFC 8493)** nyílt szabvány; **`bagit-python`** (Library of Congress, **CC0**) csak tesztorákulum-jelölt dev-függőségként (G3 dönt); **OAIS (ISO 14721)** fogalmi referencia; **FA3 Unified Journal**
(`FA3-JOURNAL-ARCHIVE-001`) és a CFA4-AUDIT-001 8. pontja mint saját minta. Kitsu/Zou (AGPL-3.0): csomagmodellje nincs, nem releváns.

## 17. G2-eredmény

**Javaslat: PASS.** Új döntési kérdés nincs. Hat G1-től eltérő vagy azt szűkítő pont indokolva (13. pont). **A File Services-szel szemben három bővítés** (mind a saját ADR 0009 tervben, nem másik projektben): `extra_hashes` (SHA-256 egy olvasásból), `files.export` (G2-25), és a 26-2 `files.relocate`;
a fájljavítás az FS-3 `restore`-ra épül. A produkció `archive_state` oszlopa és az írásvédelmi trigger a G2-02 migrációjának része lesz; a G2-12 13. pontjának bővítése (események, `archive_*_due`) megvalósításkor kerül oda. Megvalósítani akkor lehet, ha **minden adatmodell véglegesített**
(a CI-teljességi teszt ezt ellenőrzi) és az FS-1/FS-2 kész; a fájljavításhoz az FS-3 is. Az **aktív fájlok felszabadítása** és az **üres rendszerbe importálás** 26-2: külön döntéssel és külön G2-bővítéssel.
