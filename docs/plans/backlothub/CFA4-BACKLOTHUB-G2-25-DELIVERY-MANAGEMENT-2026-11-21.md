# BacklotHub 25. lépcső – Delivery Management: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-21 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.22 · **Előzmény:**
[G1 Delivery Management](CFA4-BACKLOTHUB-G1-25-DELIVERY-MANAGEMENT-2026-11-07.md) (funkcionális követelmények jóváhagyva
2026-11-10; **feltételes PASS**: előfeltétel a CFA4 Media QC) · **Épül rá:** G2-08 (verzió, reprezentáció), G2-10
(Publisher, ellenőrzők), G2-12 (jelzések), G2-13 (jóváhagyás), G2-14 (dokumentum), File Services (ADR 0009), Media Services
(ADR 0007), CFA4 Media QC (`cfa4-qc`) · **Állapot:** G2-javaslat, jóváhagyásra vár · implementáció nincs

Új döntési kérdés nincs. **Új függőség nincs.** A modul az átadás **követése, ellenőrzése és naplózása**; **külső
feltöltést nem végez** (SEC-02), és **nem csomagol DCP-t vagy IMF-et** (4. pont).

## 0. A G1 feltétele: a Media QC állapota (ellenőrizve, `main` @ `01ff4e41`)

A G1 szerint a 25. lépcső előfeltétele a `cfa4-qc` megvalósítása. **Ez mostanra teljesült a fájlalapú médiaellenőrzésre**:
a `crates/qc` a `main`-ben van (QC-B…QC-F), a `cfa4-cli qc` és az engine `qc.run`/`qc.report.get`/`qc.review.save`/`qc.profiles.list`
parancsai működnek, a jelentés `cfa4-qc-report/1`, a profil `cfa4-qc-profile/1` (`docs/media-qc.md`). A „nem létezőre nem épülhet
modul” szabály szerint azonban **csak arra építünk, ami van**, és az alábbi korlátok **a G2 szövegében is szerepelnek**:

| QC-képesség | Állapot a `main`-ben | Következmény a 25. lépcsőre |
|---|---|---|
| Mintaszintű elemzés (dekódolás, hangosság, true peak, fekete/vágott kép, A/V-eltolás) | **Csak MP4/MOV** | Más konténernél (MXF, MKV, …) a mintaszintű szabály `not_run` (a QC saját szövegével), sosem `pass` |
| Önálló hangfájl (WAV/BWF) hangosságmérése | **Nincs parancs** (a `cfa4-audio-dsp` könyvtár van, de a QC csak MP4/MOV-ot ad) | WAV-sávok: csak `probe` és fájl-szabályok; a hangosság `not_run`; **kérés a QC-projektnek** (25-2) |
| Beépített profil | Csak `delivery-web` v1 (a terv `broadcast-ebu-r128`, `master-archive` profilt is említ, de **nincsenek**) | A profilt **a leírás (spec) generálja** (3.3), nem beépítettre épül |
| `qc.run` kimeneti hely | A jelentés és a kontaktlap a **fájl mellé** íródik (`<fájl>.cfa4qc.json`), a parancsnak nincs kimeneti-út paramétere | **Lezárt fájl mellé nem írunk** (3.4): a `cfa4-cli qc --out … --contact-sheet …` hívást használjuk |
| Emberi ellenőrzés | `qc.report.get` kérdéssort ad, `qc.review.save` ment | A BacklotHub ezt használja (3.5) |
| DCP-ellenőrzés | `cfa4-dcp-validate` **könyvtár** (profilok: `quick-scan`, `full-integrity`), **nincs engine-parancs** | DCP-formátumellenőrzés **nem M1** (4. pont) |
| Felirat-formátum ellenőrzés | Nem ellenőrzött, hogy van-e validáló engine-parancs (a `captions.*` parancsokat a G3 vizsgálja) | M1: csak fájl-szabályok; formátumellenőrzés a G3 eredménye szerint vagy 25-2 |

## 1. Hatókör ebben a lépcsőben

- **Átadási leírás (delivery spec)**: tételek és **zárt, deklaratív ellenőrzési szabályok**; verziózott, aktiválás után változtathatatlan.
- **Átadási csomag**: leírás + határidő + címzett; tételek **lezárt, jóváhagyott** forrásokhoz kötve; folyamat és állapotok.
- **Ellenőrzés**: fájl- és metaadat-szabályok, `probe` (Media Services), **QC** (`cfa4-qc`), dokumentum- és metaadat-szabályok; eredmény tételenként és szabályonként.
- **Hiányosságok**: a hibákból és a címzett visszaküldött listájából; lemondás (waiver) indoklással.
- **Határidők** és jelzések; **teljesítési napló** (audit-nézet és az átadott fájlok kivonatlistája).
- **Csomag kiadása** a felhasználó által megadott helyre ellenőrzött másolással, kivonatlistával (File Services bővítés, 7. pont).
- **Nem része:** külső feltöltés; DCP/IMF **csomagolás és formátumellenőrzés**; KDM-kezelés; szerződéses átadási ütemterv és számlázás; a **gépi minőségellenőrzés** új mérése (a QC dolga).

## 2. Integrációs határok

| Határ | Irány | Szerződés |
|---|---|---|
| Asset (8.), Publisher (10.) | ← | A tétel forrása **reprezentáció** (`backlot.representation`, `role = 'original'`) egy **közzétett (`published_at` nem NULL), jóváhagyott `version`-ben**; az ellenőrző-regisztert a G2-10 adja, itt bővül (3.2) |
| Review (13.) | ← | A jóváhagyás a verzió `approved` állapota (G2-13) |
| Document (14.) | ← | A **nem média** tétel forrása **dokumentum-verzió** (`approved` vagy `filed`); a címzett eredeti előírása `delivery_spec` típusú dokumentum (G2-14 alaptípus) |
| File Services (ADR 0009) | ↔ | `files.resolve`, `files.verify` az ellenőrzéshez; a kiadás **új** `files.export` parancs (7. pont, **FS-2 bővítés**) |
| Media Services (ADR 0007) | ← | `probe` (a G2-08 reprezentáción már tárolt `probe` újrahasznosítható; frissítés `media.service.probe`) |
| Media QC (`cfa4-qc`) | ← | `cfa4-cli qc` (3.4), `qc.report.get`, `qc.review.save` (3.5); a 0. pont korlátaival |
| Organization (2.) | ← | A címzett (`organization_id`) |
| Task (5.) | → | Hiányosságból feladat nyitható (kifejezett parancs; nincs automatikus) |
| Notification (12.) | → | Események és `deadline_signal` (9. pont) |
| Search (7.) | → | `SearchProvider`: csomag (név, címzett), leírás (kód, név) |
| Audit (1.) | → | Minden írás, minden ellenőrzés, kiadás és lemondás auditált |
| Archive (26.) | később | A kiadott csomag jegyzéke és a jelentések az archívum részei |

## 3. Átadási leírás és ellenőrzés

### 3.1 Leírás (spec)

```sql
CREATE TABLE backlot.delivery_spec (
    id uuid PRIMARY KEY, code text NOT NULL, version integer NOT NULL,
    state text NOT NULL CHECK (state IN ('draft','active','retired')),
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    recipient_organization_id uuid REFERENCES backlot.organization(id),
    source_document_id uuid REFERENCES backlot.document(id),            -- a címzett előírása (delivery_spec típusú dokumentum)
    production_id uuid REFERENCES backlot.production(id),               -- NULL: globális alaptípus
    definition jsonb NOT NULL,                                           -- cfa4.backlot.delivery_spec/1
    digest bytea,                                                         -- az aktiváláskor
    ...közös oszlopok production_id nélkül,
    UNIQUE (code, version)
);
```

```json
{"schema": "cfa4.backlot.delivery_spec/1",
 "items": [
   {"code": "master", "title": "Master (H.264/AAC)", "required": true, "source": "representation",
    "rules": [
      {"id": "files_readable"},
      {"id": "extension_in", "params": {"values": ["mp4", "mov"]}},
      {"id": "probe_matches", "params": {"width": 1920, "height": 1080, "fps": "24000/1001", "video_codec": ["H.264"], "audio_tracks": 1, "audio_channels": 2}},
      {"id": "qc_decode"},
      {"id": "qc_loudness", "params": {"target_lufs": -14.0, "tolerance_lu": 1.0, "warn_lu": 2.0}},
      {"id": "qc_true_peak", "params": {"max_dbtp": -1.0, "warn_dbtp": 0.0}},
      {"id": "qc_human_review", "severity": "error"}]},
   {"code": "subtitles_en", "title": "English subtitles", "required": true, "source": "representation",
    "rules": [{"id": "files_readable"}, {"id": "extension_in", "params": {"values": ["srt", "vtt"]}}]},
   {"code": "e_and_o", "title": "E&O insurance", "required": true, "source": "document",
    "rules": [{"id": "document_type", "params": {"type": "insurance"}}, {"id": "document_valid_on", "params": {"date": "due"}}]}],
 "metadata_fields": [{"key": "title_original", "type": "text", "required": true},
                     {"key": "runtime_minutes", "type": "integer"}, {"key": "language", "type": "enum", "values": ["hu","en","de"]}],
 "qc_depth": "full"}
```

- A leírás **zárt szabályregiszterből** áll, paramétersémával (mint a G2-04 predikátumai és a G2-10 ellenőrzői); **tetszőleges kód nem fut**. Aktiváláskor validált
  (kódok egyediek, ismert szabály, a paraméterek érvényesek, `qc_*` szabály csak médiatételnél), aktiválás után **változtathatatlan**, módosítás új verzió. A csomag a **leírás verzióját** rögzíti.
- **Szabályonként `severity`** (`error` alapból, vagy `warning`) és **`requirement`** (kötelező, vagy `optional`): a QC saját profil-nyelvét követi (kötelező szabály `not_run`-ja nem lehet `pass`).
- **Tétel forrásfajtája:** `representation` (média/fájl) vagy `document`. **Több fájlos tétel** (EXR-képsor, hangsávok): a reprezentáció az egyetlen tétel, a szabály az összes fájlra vonatkozik.
- **Beépített alapleírások** (migráció): a G3-on a nyilvános előírások (Netflix Partner Help Center, DPP AS-11, EBU R 128) **funkcionális referenciája** alapján **saját, minimális** sablonok (pl. „web-átadás”, „sugárzási átadás”); a felhasználó másol és módosít. **A címzett hivatalos előírását a rendszer nem tölti be és nem értelmezi** (nincs gépi olvasás a PDF-ből): a felhasználó a `source_document_id`-vel köti az eredetit, a szabályokat kézzel veszi fel.
- **Metaadat-mezők** (`metadata_fields`): zárt típusok (`text`, `integer`, `date`, `boolean`, `enum`, `organization_ref`); a csomag `metadata` mezője ez ellen validált.

### 3.2 Szabályregiszter

| Szabály | Mit ellenőriz | Honnan |
|---|---|---|
| `files_readable` | Minden fájl feloldható és olvasható; a katalógus-kivonat egyezik (`files.verify`) | File Services |
| `extension_in`, `size_max`, `name_matches_template` | A G2-10 szabályai szerint | G2-10 |
| `sequence_complete`, `frame_range_matches_entity` | Képsor teljessége; a snitt időzítéséhez illesztve | G2-10 |
| `probe_matches` | Konténer, kodek, felbontás, fps (racionális), hangsáv-szám és csatorna, mintavétel, időtartam-tartomány, színinformáció (primaries, transfer, matrix, range) a tárolt/friss `probe` szerint; az **időt `Tick`/racionális számmal veti össze, nem lebegőpontosan** | Media Services |
| `qc_decode` | A QC `integrity.decode`, `integrity.frames`, `integrity.streams` | QC (MP4/MOV) |
| `qc_loudness` | `audio.loudness` cél-érték és tűrés (`pass` = ± tolerance, `accept` = ± warn) és `audio.loudnessRange` (opcionális maximum) | QC (MP4/MOV) |
| `qc_true_peak` | `audio.truePeak` | QC (MP4/MOV) |
| `qc_picture` | `video.black`, `video.clipBlack`, `video.clipWhite` küszöbökkel | QC (MP4/MOV) |
| `qc_av_sync` | `integrity.avOffset` (ms) | QC (MP4/MOV) |
| `qc_human_review` | A QC emberi ellenőrzése kötelező, és el van végezve (3.5) | QC |
| `metadata_required` | A csomag metaadat-mezői (G2-10 elve, itt a csomag szintjén) | csomag |
| `document_type`, `document_valid_on` | A dokumentum típusa; az érvényessége (`valid_until`, G2-14) a megadott napon (`due` = a csomag határideje) | G2-14 |

- **Mi a mérés és mi a szabály:** a QC **mér és küszöb szerint ítél**; a BacklotHub a leírás paramétereiből **QC-profilt generál** (3.3), nem ítél újra.
- **Bővítés:** új szabály kódmódosítás és a regiszter verziója (nincs felhasználói kód); a `dcp_valid`, a hangfájl-hangosság és a felirat-formátum szabálya **akkor kerül be, ha a hozzá tartozó motorparancs
  létezik** (25-2), addig a regiszterben nincs, nincs helyettesítő.

### 3.3 QC-profil előállítása

- A tételhez tartozó `qc_*` szabályokból **`cfa4-qc-profile/1` TOML** készül (a QC saját szerkezetében: `checks."audio.loudness"` `pass`/`accept` határokkal, `requirement`, `human_review`); a fájl a **QC-futás
  munkakönyvtárában** keletkezik, a **QC rögzíti a profil azonosítóját, verzióját és SHA-256-ját** a jelentésben, így a leírás-verzió és a mérés összefügg.
- A profil `id`-je: `bl-<spec-code>-v<version>-<item-code>`; a `version` a leírás verziójából jön. **Ismeretlen kulcs/ellenőrzés azonosító hiba** a QC-ban (`unknown keys are errors`), ezért a generátor csak a QC
  `catalog.rs` ismert azonosítóit használja; a lista a QC-vel **szerződésteszten** (G5) szinkronban marad (a QC azonosítók „soha nem nevezhetők át”).
- A `--expect` terv (JSON) a `probe_matches` paramétereiből képzett (képkockaszám, ráta, méret, kodek, hangsávok, csatornák, mintavétel); a hiányzó paraméter → a megfelelő QC-ellenőrzés `NOT_RUN` („no render plan given”),
  ami **kötelező szabálynál nem lehet `pass`**.

### 3.4 A QC futtatása és tárolása

- **Hívás:** a szolgáltatás **külön folyamatban** indítja a `cfa4-cli qc <feloldott út> --profile <profil.toml> --expect <terv.json> --out <jelentés> --contact-sheet <kép>` parancsot (argumentumlista, **shell nélkül**), a kimenetek
  **a BacklotHub munkakönyvtárába** (`$XDG_CACHE_HOME/cfa4/backlothub/qc/<futás-azonosító>/`) mennek, **nem a lezárt fájl mellé**: a File Services lezárt fájljai mellé a QC nem írhat, és a katalógus nem látna
  kéretlen oldalfájlokat. A kilépési kód: 0 `PASS`, 3 `WARN`, 4 `FAIL`, 5 `INCOMPLETE`, 1 hiba, 2 használati hiba.
- **Miért a CLI és nem a `qc.run`:** a `qc.run` a jelentést a fájl mellé írja, és nincs kimeneti-út paramétere. A `cfa4-cli qc` ma létezik és támogatja a kimeneti utat; **kérés a QC-projekt felé**
  (a merge-koordinátoron át, nem ebből a szálból): `qc.run` kapjon `out`/`contactSheet` paramétert, hogy a BacklotHub a `serve` protokollon át, mint minden más parancsot hívhassa. Amíg ez nincs, a CLI az út, **nem
  helyettesítő**, hanem a QC kiadott felülete; a verzió-kompatibilitást a `cfa4-cli --version` és a jelentés `cfa4-qc-report/1` sémaellenőrzése adja (`DELIVERY_QC_UNAVAILABLE`, ha nem egyezik vagy nincs a bináris).
- **Jelentés ellenőrzése:** a kapott jelentést a QC saját olvasója ellenőrzi (`qc.report.get {path: <jelentés>}`: a státuszok az értékekkel és határokkal egyezzenek, az összesítés a találatokkal): a kézzel módosított vagy csonka jelentést **elutasítja**
  (`DELIVERY_QC_REPORT_INVALID`).
- **Tárolás:** a jelentés a `delivery_qc_report` táblában (`jsonb`, legfeljebb 8 MiB, felette a File Services `attachment` gyökerébe lezárt fájlként, hivatkozással); a kontaktlap (PNG) `files.import` + `files.seal` az `attachment` gyökérbe (útsablon:
  `{production.code}/delivery/{package.code}/qc/{item.code}/…`, G2-09 motorja). Az emberi ellenőrzés után a jelentés **új revízióként** tárolódik (a régi megmarad).
- **Mélység:** `qc_depth` a leírásban: `full` (teljes dekódolás) vagy `container` (`--no-decode`, csak amit a fájl magáról mond; a mintaszintű szabályok ekkor `not_run`). A **futási idő nagy mesterfájlnál jelentős** (a QC szoftveres
  dekóderrel mintánként olvas): a felület **háttérfeladatként, megszakíthatóan** futtatja, nem ígér időt; a CFA4-MIN gépen mért értékek a G5 mérésben (14. pont).

### 3.5 Emberi ellenőrzés

- Ha a leírás `qc_human_review`-t kér, a QC profilja `human_review = "mandatory"`: a jelentés **`INCOMPLETE`** marad, amíg az emberi ellenőrzés nincs kész. A BacklotHub a **kérdéssort a `qc.report.get`-ből** veszi, a válaszokat és észrevételeket (súlyosság,
  időkód, megjegyzés) a `qc.review.save` menti a munkakönyvtárbeli jelentésbe, a QC **újraszámolja az ítéletet**; az új jelentést a 3.4 szerint tároljuk. Az ellenőrző `person` a felhasználó (M1: egy).
- A felület az **űrlapot a QC Qt6-ablakának kódjából használja újra**, ha az megosztható modulként elérhető (`python/cfa4_qt/panels/qc.py`, a QC-projekt tulajdona); ha nem, a BacklotHub a kérdéseket a `qc.report.get` mezőiből **maga rajzolja** — a G3 dönti el, a viselkedés azonos.
- A lejátszás a jelzett időkódra a **Review (G2-13) lejátszójának** képkocka-szolgáltatásával történhet; ez **nem előfeltétel**.

## 4. Mi nem készül el M1-ben: cinema-csomag és társai

- **DCP-ellenőrzés:** a `cfa4-dcp-validate` **könyvtár**, `serve`-en át elérhető parancs **nincs**; a `cfa4-dcp-inspect` külső motort igényel. **DCP/IMF-csomag formátumellenőrzése és kiadása a 25-2 al-lépcső**, amely **a DCP-projekt engine-parancsait** (`dcp.validate` jelleg) és a QC
  hangfájl-bővítést várja. **Kérés a koordinátornak**, nem ebből a szálból elvégezhető.
- **Addig** a DCP/IMF mappa a rendszerben **átlátszatlan, de követett tétel** lehet: reprezentációként (több fájl, a G2-10 `sequence`-szerű kezelése), `files_readable`-lel (fájlok épsége, kivonat), `name_matches_template`-tel; a felület **kimondja, hogy a formátum
  érvényességét nem ellenőrzi** („fájlépség ellenőrizve, DCP-formátum nem”). A leírásban ilyenkor nincs `dcp_valid` szabály (nincs a regiszterben), tehát a „kész” állapotot nem blokkolja és nem is igazolja a formátumot; a leírás szerzője felel azért, hogy mit ír elő.
- **IMF:** nincs kész csomagoló (G1); nem része.
- **Hangsáv (WAV/BWF) hangossága és felirat-formátum:** 0. pont táblája szerint.

## 5. Csomag, tételek, állapotok

```sql
CREATE TABLE backlot.delivery_package (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    code text NOT NULL, name text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    spec_id uuid NOT NULL REFERENCES backlot.delivery_spec(id),
    recipient_organization_id uuid REFERENCES backlot.organization(id),
    due_date date,
    status_id uuid NOT NULL REFERENCES backlot.status(id),               -- delivery_package_default (G2-04)
    round integer NOT NULL DEFAULT 1 CHECK (round > 0),                  -- visszaküldés után új kör
    metadata jsonb NOT NULL DEFAULT '{}',                                 -- a leírás metadata_fields szerint validálva
    note text CHECK (length(note) <= 2000),
    search_vector tsvector GENERATED ALWAYS AS (…) STORED,
    retired_at timestamptz,
    ...közös oszlopok,
    UNIQUE (production_id, code)
);

CREATE TABLE backlot.delivery_item (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    package_id uuid NOT NULL REFERENCES backlot.delivery_package(id),
    item_code text NOT NULL,                                              -- a leírás tételkódja
    source_kind text NOT NULL CHECK (source_kind IN ('representation','document')),
    representation_id uuid REFERENCES backlot.representation(id) ON DELETE RESTRICT,
    document_version_id uuid REFERENCES backlot.document_version(id) ON DELETE RESTRICT,
    due_date date,                                                        -- opcionális tételhatáridő
    ...közös oszlopok,
    UNIQUE (package_id, item_code),
    CHECK ((source_kind = 'representation' AND representation_id IS NOT NULL AND document_version_id IS NULL)
        OR (source_kind = 'document' AND document_version_id IS NOT NULL AND representation_id IS NULL)
        OR (representation_id IS NULL AND document_version_id IS NULL))   -- üres tétel: még nincs forrás
);
```

- **A tétel üres, amíg nincs forrása**; a `backlot.delivery.item.assign` ellenőrzi: a forrás ugyanabban a produkcióban van; **a reprezentáció verziója közzétett (`published_at` nem NULL, változtathatatlan) és `approved`**; a dokumentum-verzió `approved` vagy `filed`; a
  forrásfajta egyezik a leírás tételével (`DELIVERY_ITEM_SOURCE_INVALID`). **Nem jóváhagyott vagy nem lezárt anyag nem adható át** (G1).
- **Újabb verzió:** ha a terméknek a tételben lévőnél **újabb jóváhagyott** verziója van, a csomag jelzi (`newer_version_available`, számolt); a csere **kifejezett parancs** (`backlot.delivery.item.replace`, indoklással), amely az
  előző ellenőrzéseket **érvényteleníti** (a sorok megmaradnak, az új forrásra újra kell futtatni).
- **Folyamat** `delivery_package_default` (G2-04, zárt őrfeltételekkel): `assembling → checking → ready → delivered → accepted`, `delivered → returned`, `returned → assembling` (új `round`), `* → cancelled` (kivéve
  `accepted`). **A `checking` és az ellenőrzés** nem kötelező külön lépés: a felhasználó bármikor futtathat ellenőrzést; az őrfeltétel a **`ready`**-hez: a leírás minden `required` tételéhez van forrás, és minden **kötelező szabály** `pass`
  (vagy `warn`, ha a szabály `severity = warning`) **vagy lemondva** (6. pont). `delivered`: csak `ready`-ből, és csak a kiadás (7. pont) után. A **jogosultság parancsszintű**.
- **A tétel állapota számolt** (nem tárolt): `empty` (nincs forrás), `unchecked`, `running`, `passed`, `warned`, `failed`, `incomplete` (kötelező szabály `not_run`), `waived` (minden hiba lemondva). A csomag állapota ennek és a folyamatnak az együttese.

### 5.1 Ellenőrzés eredménye

```sql
CREATE TABLE backlot.delivery_check_run (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    package_id uuid NOT NULL REFERENCES backlot.delivery_package(id),
    item_id uuid NOT NULL REFERENCES backlot.delivery_item(id),
    round integer NOT NULL,
    source_fingerprint text NOT NULL,                                     -- forrás-azonosító + katalógus-kivonatok (BLAKE3) + leírás-verzió
    state text NOT NULL CHECK (state IN ('running','done','failed','cancelled')),
    started_at timestamptz NOT NULL, ended_at timestamptz,
    error_code text,
    ...közös oszlopok
);

CREATE TABLE backlot.delivery_check_result (
    run_id uuid NOT NULL REFERENCES backlot.delivery_check_run(id) ON DELETE CASCADE,
    rule_id text NOT NULL, rule_index integer NOT NULL,
    status text NOT NULL CHECK (status IN ('pass','warn','fail','not_run')),
    severity text NOT NULL CHECK (severity IN ('error','warning')),
    requirement text NOT NULL CHECK (requirement IN ('required','optional')),
    measured jsonb, expected jsonb, unit text,
    reason text,                                                           -- not_run/fail oka (angol kód + paraméterek)
    events jsonb NOT NULL DEFAULT '[]',                                    -- QC-események: képkocka, Tick, időkód
    PRIMARY KEY (run_id, rule_index)
);

CREATE TABLE backlot.delivery_qc_report (
    id uuid PRIMARY KEY, run_id uuid NOT NULL REFERENCES backlot.delivery_check_run(id),
    revision integer NOT NULL CHECK (revision > 0),                        -- az emberi ellenőrzés után új revízió
    verdict text NOT NULL CHECK (verdict IN ('PASS','WARN','FAIL','INCOMPLETE')),
    profile_id text NOT NULL, profile_version integer NOT NULL, profile_sha256 text NOT NULL,
    report jsonb, report_file_object_id uuid REFERENCES cfa4_files.file_object(id) ON DELETE RESTRICT,
    report_sha256 text NOT NULL,
    contact_sheet_file_object_id uuid REFERENCES cfa4_files.file_object(id) ON DELETE RESTRICT,
    ...közös oszlopok,
    UNIQUE (run_id, revision),
    CHECK ((report IS NULL) <> (report_file_object_id IS NULL))
);
```

- **A futás a forrás ujjlenyomatához kötött**: ha a forrás vagy a leírás-verzió változik, a korábbi eredmény **nem érvényes** az új állapotra (a sorok megmaradnak, a tétel `unchecked`). A `source_fingerprint` a fájlok **katalógus-kivonatát** hordozza, így egy ellenőrzött fájl utólagos
  módosítása (File Services `FILES_HASH_MISMATCH`) is kiderül.
- **`not_run` sosem `pass`**: a tétel `incomplete`, a kötelező szabály `not_run` a `ready`-t akadályozza (lemondható, 6. pont). Az ok szöveges kódja (`not_run_reason` a QC-ból, vagy `FILES_ROOT_UNAVAILABLE`, `QC_UNSUPPORTED_CONTAINER` jelleg) látszik.
- **A mért érték és az elvárt** együtt tárolt (nem csak a `pass/fail`), hogy a hiba oka a hiánylistán érthető legyen („mért −9,8 LUFS, elvárt −14 ± 1”); az idő **`Tick`/racionális** (QC-jelentésből), a BacklotHub nem számol idővel.

## 6. Hiányosságok és lemondás

```sql
CREATE TABLE backlot.delivery_deficiency (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    package_id uuid NOT NULL REFERENCES backlot.delivery_package(id),
    item_id uuid REFERENCES backlot.delivery_item(id),
    round integer NOT NULL,
    origin text NOT NULL CHECK (origin IN ('check','recipient')),
    check_run_id uuid REFERENCES backlot.delivery_check_run(id), rule_index integer,
    title text NOT NULL CHECK (length(title) BETWEEN 1 AND 300),
    detail text CHECK (length(detail) <= 5000),
    severity text NOT NULL CHECK (severity IN ('critical','major','minor')),
    status text NOT NULL DEFAULT 'open' CHECK (status IN ('open','resolved','waived')),
    task_id uuid REFERENCES backlot.task(id),
    resolved_note text CHECK (length(resolved_note) <= 1000),
    ...közös oszlopok
);

CREATE TABLE backlot.delivery_waiver (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    package_id uuid NOT NULL REFERENCES backlot.delivery_package(id),
    item_id uuid NOT NULL REFERENCES backlot.delivery_item(id),
    rule_id text NOT NULL,
    source_fingerprint text NOT NULL,                                      -- az a forrás-állapot, amelyre a lemondás szól
    reason text NOT NULL CHECK (length(reason) BETWEEN 1 AND 1000),
    ...közös oszlopok,
    UNIQUE (item_id, rule_id, source_fingerprint)
);
```

- **Hiány a hibából:** a `backlot.delivery.deficiency.from_check` egy hibás/`not_run` szabályból hiányosságot hoz létre (cím: a szabály, részlet: mért/elvárt, súlyosság az eredmény `severity`-jéből); **kifejezett parancs**, nem automatikus, hogy ne tömje el a listát. A
  `.open_task` feladatot nyit (G2-05 `task.create`-et hívja, a hiányosságra hivatkozással). A hiányosság **lezárása** (`resolved`) a felhasználó döntése, indoklással; újrafutó ellenőrzés a hibát nem zárja le automatikusan.
- **A címzett visszaküldött listája** (`returned`): a felhasználó **kézzel** vagy (a leírás-dokumentum mintájára) a visszaküldött dokumentumra hivatkozva felveszi a hiányosságokat (`origin = 'recipient'`); a csomag `returned → assembling` új
  `round`-dal. A régi kör hiányosságai, ellenőrzései és eredményei **megmaradnak** (a kör száma jelöli).
- **Lemondás (waiver):** a kötelező szabály hibáját vagy `not_run`-ját a **`owner`/`producer`** indoklással lemondhatja; a lemondás **egy tételre, egy szabályra és egy forrás-állapotra** szól (ha a forrás vagy a leírás változik,
  a lemondás nem él tovább, mint a G2-23 tudomásulvétele). A lemondott szabály a nézetben „lemondva”, **nem tűnik el**; minden lemondás **auditált**, az indoklás szövege az auditban szerepel. A lemondás **a mérést nem írja át** (a `fail` marad `fail` a jelentésben).

## 7. Kiadás: ellenőrzött másolás a felhasználó helyére

- **Mi a kiadás:** a csomag fájljainak **ellenőrzött másolata egy, a felhasználó által megadott könyvtárba** (külső meghajtó, hálózati megosztás, `NAME` mappa), **kivonatlistával** (`sha256sum` formátum, rendezett, ugyanabból a
  csomagból mindig azonos) és a csomag **`manifest.json`**-jával (tételek, forrás-azonosítók, fájlnevek, méretek, SHA-256, a leírás kódja és verziója, a kör). **Külső feltöltés nincs** (SEC-02); a felhasználó a kiadott mappát maga továbbítja.
- **Új File Services parancs: `files.export`** (**FS-2 bővítés**; a [CFA4-FILE-SERVICES-001](CFA4-FILE-SERVICES-001-TERVVAZLAT-2026-11-06.md) tervben ma `import`, `seal`, `verify`, `resolve`, `delete`, `purge`, `roots.*` szerepel, **másolás kifelé nincs**): lezárt katalógusfájlokat másol egy célkönyvtárba a G2 szerinti
  mintával — `NAME.INCOMPLETE/`-be, fájlonként ideiglenes névre írva, `fsync`, átnevezés, **visszaolvasás és kivonat-összevetés** (a forrás BLAKE3-kivonata ellen a katalógusban, és SHA-256 a címzetti jegyzékhez), majd a jegyzék, végül az
  atomikus átnevezés `NAME`-re; **meglévő `NAME` soha nem íródik felül**; megszakadás után folytatható. A minta a `cfa4-cinema-pkg` már működő `deliver` függvénye (DCP-specifikus, de ugyanez a szabály), amelyet a File Services **általánosít** katalógusfájlokra. **Amíg a `files.export` nincs, a
  kiadás nem készül el** (nincs helyettesítő kézi másolás); az ellenőrzés, a hiányosságok és a követés ettől függetlenül működik.
- **A kiadás naplója:** `delivery_export` tábla (csomag, kör, felhasználó, a cél címkéje és útja, fájlok száma, bájtok, a jegyzék SHA-256-ja, időpont) és a jegyzék **lezárt fájlként** az `attachment` gyökérben; a `delivered` átmenet ugyanabban a tranzakcióban történik.
  Hibás kiadás (lemez megtelt, a cél nem írható, kivonat-eltérés): `DELIVERY_EXPORT_FAILED`, a `NAME.INCOMPLETE` marad (folytatható), **a csomag állapota nem változik**.
- **Elfogadás/visszaküldés** (`accepted`/`returned`): a felhasználó rögzíti a címzett válaszát (dátum, jegyzet, opcionálisan a levél dokumentum-hivatkozása); **a rendszer nem kommunikál a címzettel**.

```sql
CREATE TABLE backlot.delivery_export (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    package_id uuid NOT NULL REFERENCES backlot.delivery_package(id),
    round integer NOT NULL,
    target_label text NOT NULL CHECK (length(target_label) BETWEEN 1 AND 200),
    target_path text NOT NULL,
    file_count integer NOT NULL, total_bytes bigint NOT NULL,
    manifest_sha256 text NOT NULL,
    manifest_file_object_id uuid NOT NULL REFERENCES cfa4_files.file_object(id) ON DELETE RESTRICT,
    exported_at timestamptz NOT NULL,
    ...közös oszlopok
);
```

## 8. Hozzáférés

- **Olvasás:** a csomag, a tételek, az ellenőrzési eredmények, a hiányosságok és a kiadási napló a **produkció minden tagja** számára látható; **szerkesztés** `owner`, `producer`, `coordinator`; **lemondás, `ready`/`delivered`/`accepted`/`returned` átmenet, kiadás**
  `owner`, `producer`; a leírás **aktiválása** `owner`, `producer`. A QC-jelentés és a kontaktlap **képkockákat tartalmaz**: a megtekintés a verzió/dokumentum **láthatóságát** követi (G2-08, G2-14 5. pont): akinek a forrás nem látható, annak a jelentés sem.
- **Személyes adat:** nincs új személyes adat; a címzett szervezet. A QC-jelentés a **fájl útját** tartalmazza: a tárolt jelentésben a feloldott út **a katalógus relatív útjára cserélve** kerül az adatbázisba (a gépfüggő abszolút út nem).
- A szabályok a szolgáltatásrétegben és a lekérdezésben **már most** érvényesek.

## 9. Parancsok, hibakódok, események

| Parancs | Mit csinál |
|---|---|
| `backlot.delivery.spec.create` / `.update` / `.activate` / `.new_version` / `.retire` / `.list` / `.get` / `.validate` | Leírások (3.1); `owner` aktivál |
| `backlot.delivery.package.create` / `.update` / `.cancel` / `.list` / `.get` | Csomag (5.) |
| `backlot.delivery.package.ready` / `.deliver` / `.accept` / `.return` / `.reopen` | Átmenetek a G2-04 szerint generálva; a `deliver` a kiadást hívja (7.) |
| `backlot.delivery.item.assign` / `.replace` / `.clear` / `.list` | Tétel forrása |
| `backlot.delivery.check` | Ellenőrzés tételre vagy az egész csomagra (háttérfeladat, megszakítható); `backlot.delivery.check.get` / `.cancel` |
| `backlot.delivery.qc.review.get` / `.save` | Emberi ellenőrzés (3.5) |
| `backlot.delivery.deficiency.add` / `.from_check` / `.update` / `.resolve` / `.reopen` / `.open_task` / `.list` | Hiányosságok (6.) |
| `backlot.delivery.waive` / `.unwaive` | Lemondás (6.) |
| `backlot.delivery.export` / `.verify` / `.list` | Kiadás és a kiadott mappa újraellenőrzése a jegyzék szerint (7.) |
| `backlot.delivery.log` | A csomag naplója: az audit szűrt nézete (csomag-hatókör), az ellenőrzések, a kiadások |

Hibakódok: `DELIVERY_SPEC_NOT_FOUND`, `DELIVERY_SPEC_INVALID` (ismeretlen szabály, hibás paraméter), `DELIVERY_SPEC_STATE_INVALID`, `DELIVERY_PACKAGE_NOT_FOUND`, `DELIVERY_PACKAGE_STATE_INVALID`,
`DELIVERY_NOT_READY` (kötelező szabály nem `pass` és nincs lemondva), `DELIVERY_ITEM_SOURCE_INVALID`, `DELIVERY_ITEM_REPLACE_INVALID`, `DELIVERY_QC_UNAVAILABLE`, `DELIVERY_QC_REPORT_INVALID`,
`DELIVERY_DEFICIENCY_INVALID`, `DELIVERY_WAIVER_INVALID`, `DELIVERY_EXPORT_FAILED`, `DELIVERY_METADATA_INVALID`, `CONFLICT_STALE_VERSION`; valamint a `FILES_*`, `DOCUMENT_*`, `WORKFLOW_*`, `AUDIT_*` kódok változatlanul.

**Események** (outbox, G2-12): `backlot.delivery.package_ready`, `.package_delivered`, `.package_accepted`, `.package_returned`, `.check_failed` (csomag és tétel azonosítóval, a mért értékek nélkül), `.deficiency_opened`;
**időzített jelzés** (`deadline_signal`): `delivery_due` (`subject_type = 'delivery_package'` és `'delivery_item'`, a határidő előtti napok és az átlépés), ugyanazzal az időzítővel, mint a G2-14 lejárata. Új címzett nincs (`production_owner`, `watcher`).

## 10. Never crash

- **A QC külön folyamat**, időkorláttal és megszakítással (`SIGTERM`, majd a félkész kimenet takarítása); nem létező bináris, nem egyező verzió, hibás vagy csonka jelentés, a kilépési kód és a jelentés ellentmondása: `DELIVERY_QC_UNAVAILABLE` / `DELIVERY_QC_REPORT_INVALID`,
  a futás `failed`, **a tétel `incomplete`, a csomag nem omlik össze, és nincs `pass`**.
- **Hiányzó gyökér, sérült vagy módosult fájl** (`FILES_ROOT_UNAVAILABLE`, `FILES_HASH_MISMATCH`): az érintett szabályok `not_run`/`fail` okkal, a többi tétel ellenőrizhető marad.
- **A leírás aktiválás előtt validált**; hibás paraméter nem fut el menet közben; a szabályregiszter ismeretlen azonosítóját a betöltő hibaként kezeli (nem hagyja ki csendben).
- **A kiadás megszakadása** (lemez megtelt, kihúzott meghajtó): `NAME.INCOMPLETE` marad, folytatható; a csomag állapota és az eredmények változatlanok; **meglévő célt soha nem ír felül**.
- **A `ready` őrfeltétel egy tranzakcióban** értékelt (állapot, ujjlenyomat-egyezés, lemondások); versenyhelyzetben (közben új forrás) `CONFLICT_STALE_VERSION`.
- **Időszámítás:** a QC-ből és a Media Services-ből jövő `Tick`/racionális értékek; a BacklotHub nincs lebegőpontos idővel (`24000/1001` nem `23,976`).

## 11. Sorrend és függés

| Rész | Kell hozzá |
|---|---|
| Leírás, csomag, tétel, állapotok, hiányosság, lemondás | 2., 8., 10., 13., 14. lépcső (közzétett, jóváhagyott forrás), Workflow Engine (G2-04) |
| Fájl-, `probe`-, metaadat-, dokumentum-szabályok | File Services FS-1/FS-2 (`verify`), Media Services MS-1 (`probe`) |
| QC-szabályok (MP4/MOV) | **`cfa4-qc` a `main`-ben (teljesült)**; `cfa4-cli` bináris a gépen |
| Kiadás | **`files.export` (FS-2 bővítés)** — a kiadás nélküle nem indul |
| Határidő-jelzés | 12. lépcső |
| Hangfájl-hangosság, DCP/IMF, felirat-formátum | 25-2, a motor- és QC-projekt parancsai után |

A G1 „Feltételes PASS” feltétele **teljesült a fájlalapú ellenőrzésre**; a maradó korlátok a 0. pontban és a 4. pontban nevesítettek, nincs helyettesítő.

## 12. Eltérések a G1-től

| # | G1 | G2-javaslat | Indok |
|---|---|---|---|
| 1 | „A fájlok a `archive` vagy külön `delivery` gyökérbe” | **Nincs `delivery` gyökér**: az átadott mappa a katalóguson **kívül**, a felhasználó által megadott helyen; a jegyzék lezárt fájl az `attachment` gyökérben | A katalógusban nem maradna értelmes „átadott másolat”; a címzetti mappa külső hordozóra megy |
| 2 | Mozis csomag a CFA4 DCP-írásával | **Nem M1**: csak követés és fájlépség; formátumellenőrzés/kiadás 25-2 | `cfa4-dcp-validate` csak könyvtár, nincs engine-parancs; IMF-csomagoló nincs |
| 3 | A QC `serve`-parancson át | **`cfa4-cli qc` kimeneti úttal** M1-ben; a `qc.run` kimeneti-út kérése a QC-projekt felé | A `qc.run` a lezárt fájl mellé írna; a CLI ma létezik |
| 4 | Egyetlen „megfelel/nem felel meg” | + **lemondás** (waiver), `not_run` sosem `pass` | A QC saját szemantikája; a hiányosság-kezelés kéri |
| 5 | (nem szerepelt) | **`files.export` új File Services parancs** | Az ellenőrzött kifelé másolás nincs a tervben; nélküle a kiadás nem készül, helyettesítő nincs |

## 13. Tesztek (G5-re előre)

- **Leírás:** minden szabály paraméter-érvényessége, ismeretlen szabály/kulcs elutasítása aktiváláskor, változtathatatlanság, új verzió, a `qc_*` szabály csak médiatételnél, a metaadat-séma.
- **QC-profil generálás:** minden `qc_*` szabály → QC-profil (a QC saját betöltője elfogadja), ismeretlen azonosító szerződésteszt a QC `catalog.rs`-ével, a `pass`/`accept` határok, `requirement`, `human_review`.
- **QC futtatás:** hamis `cfa4-cli` (kilépési kódok 0/3/4/5/1/2, hibás/csonka/kézzel módosított jelentés, időtúllépés, megszakítás, hiányzó bináris, verzióeltérés); **valódi** `cfa4-cli qc` szintetikus mintán (a QC saját mintageneráló útján, `target/fixtures/`): sikeres, hibás hangosság, csonka fájl; a kimenet **nem** a lezárt fájl mellé kerül.
- **Szabályok:** `probe_matches` (racionális fps, `24000/1001`), színinformáció, `files_readable` (módosult fájl: `FILES_HASH_MISMATCH`), `document_valid_on`, `metadata_required`; **`not_run` sosem `pass`** minden szabályra; nem MP4/MOV konténer.
- **Tétel és csomag:** forrásellenőrzés (közzétett, jóváhagyott, más produkció, nem lezárt), `replace` érvénytelenít, `newer_version_available`, a számolt tételállapot, az őrfeltétel a `ready`-hez (lemondással is), a körök (`returned → assembling`).
- **Hiányosság és lemondás:** `from_check`, `open_task`, a címzetti hiány, a lemondás ujjlenyomat-kötése (forrásváltásnál megszűnik), audit.
- **Emberi ellenőrzés:** kérdéssor a `qc.report.get`-ből, `qc.review.save`, az ítélet újraszámolása, az új jelentés-revízió, `INCOMPLETE` amíg nincs kész.
- **Kiadás** (a `files.export` után): sikeres másolat, kivonatlista (`sha256sum`-mal ellenőrizve), `NAME.INCOMPLETE` folytatása, felülírás tiltása, lemez megtelt, kihúzott meghajtó, kivonat-eltérés; `delivery.verify`.
- **Hozzáférés:** minden szerep minden adatra; a jelentés/kontaktlap láthatósága a forráshoz kötött; az abszolút út nem kerül az adatbázisba.
- **Határidő és események:** `delivery_due`, az események tartalma (mért érték nélkül).
- **Never crash:** fuzzolt leírás-definíció, hibás jelentés-JSON, extrém tételszám.
- **Felületi ellenőrzés:** `ui.screenshot` a csomag-táblázatról (tételek × állapot), a szabály-részletekről mért/elvárt értékkel, a hiánylistáról és a kiadás-párbeszédről.
- **Mérés** a CFA4-MIN gépen: a QC futásideje reprezentatív mesterfájlokon (ProRes/H.264, 10 és 90 perc; `full` és `container` mélység), 500 tételes csomag ellenőrzése, a `files.export` átvitele nagy csomagra.

## 14. Licenc (G3-ra előre)

Új függőség nincs a BacklotHub oldalán (a `subprocess`, `json`, `hashlib`, `csv`; a `cfa4-cli`/`cfa4-qc` a CFA4 saját kódja, GPL-3.0-or-later). A `files.export` a File Services BLAKE3-függőségét (ADR 0009) használja, a SHA-256 jegyzék a
`sha2` Rust-crate-tel (már a QC és a `cinema-pkg` függősége). Netflix, DPP, EBU, SMPTE: **funkcionális referencia**, előírásszöveget nem veszünk át. Kitsu/Zou: nincs átadásmodulja, nem releváns.

## 15. G2-eredmény

**Javaslat: PASS** (a G1 feltételes PASS-ának feltétele a fájlalapú ellenőrzésre teljesült). Új döntési kérdés nincs; öt G1-től eltérő pont indokolva (12. pont). **A koordinátornak szóló kérések** (más projektek munkája, ebből a szálból nem végezhetők): a QC `qc.run`
kimeneti-út paramétere, hangfájl-hangosság parancs, DCP-validálás engine-parancsként. **A BacklotHub saját** függősége a File Services `files.export` bővítése (FS-2). A modul a 8., 10., 13., 14. lépcső után indulhat; a kiadás a `files.export`, a cinema-csomag a 25-2 előtt nem.
A G2-12 13. pontjának bővítése (események, `delivery_due`) és a G2-14 `subject_kind` (`delivery_package`) megvalósításkor kerül oda, új tábla nélkül.
