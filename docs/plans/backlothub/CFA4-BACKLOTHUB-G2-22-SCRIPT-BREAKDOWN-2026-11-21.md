# BacklotHub 22. lépcső – Script Breakdown: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-21 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.15 · **Előzmény:**
[G1 Script Breakdown](CFA4-BACKLOTHUB-G1-22-SCRIPT-BREAKDOWN-2026-11-07.md) (funkcionális követelmények jóváhagyva
2026-11-10) · **Épül rá:** G2-03 (jelenet, karakter, `set` elem, `entity_link`), G2-14 (a forgatókönyv-változat mint
dokumentum), G2-19 (karakter, `casting`), G2-20 (`set`, `setting`), File Services (ADR 0009) · **Állapot:** G2-javaslat,
jóváhagyásra vár · implementáció nincs

Új döntési kérdés nincs, **új függőség nincs**. Három pont tér el a G1-től vagy az előző G2-któl, mindegyik indokolva:
a Fountain-olvasó **saját kód**, Screenplain referencia és tesztforrás (3.1); az **INT/EXT és a napszak a jelenet
tulajdonsága**, nem a `set`-é (5. pont; a **G2-20 3.1 ennek megfelelően javítva**); a **gépi elemjavaslat** nincs, és nem
is készül hozzá helyettesítő (12. pont, mint a G1-ben).

## 1. Hatókör ebben a lépcsőben

- **Import** egy konkrét forgatókönyv-változatból (Fountain, Final Draft FDX): jelenetek, helyszínnevek, szereplő-névjegyek,
  oldalhossz nyolcadokban, **javaslatként**.
- **Kézi ellenőrzés:** minden importált elem javaslat; a felhasználó elfogadja, meglévőhöz köti vagy elveti.
- **Elemek jelenetenként** (kellék, jelmez, smink, jármű, állat, effekt, …), **jelölés a forgatókönyv szövegében**.
- **Új változat összevetése**, az eltérések egyenkénti elfogadása; a meglévő breakdown nem vész el.
- **Jelenetlap** (PDF) és CSV-jelentések.
- **Nem része:** forgatókönyv-**szerkesztés** (a szöveg változatlan marad; a szerkesztés külső programban történik, az
  eredmény új változatként jön be); gépi elemjavaslat (12.); az ütemezés (23.) és a költség (24.); PDF-ből import (nincs
  megbízható szerkezeti információ benne: a PDF a dokumentumkezelésben marad, G2-14); karakter nélküli „automatikus
  szereplőfelismerés” az akciósorokból (4.3).

## 2. Integrációs határok

| Határ | Irány | Szerződés |
|---|---|---|
| Document (14.) | ← | A forrás a `script_revision` típusú dokumentum **egy `content` verziója**; az importálás ezt a verziót és annak SHA-256-ját rögzíti |
| File Services (ADR 0009) | ← | `files.resolve`, `files.verify` a forrásfájlra; a forrás **nem módosul** |
| Structure (3.) | → | A jelenet `scene`, a karakter `character`, a helyszín `set` elem; létrehozás **kizárólag** a tulajdonos modul parancsán át (alább), az `entity_link` szabályaival |
| Cast (19.) | → | `backlot.cast.character.create` (karakter + cast ID), `backlot.cast.link.add` (`casting`) |
| Location (20.) | → | `backlot.location.set.create`, `backlot.location.scene_link.add` (`setting`) |
| Search (7.) | → | `SearchProvider`: breakdown-elem (név, kategória, jelenetszámok); a jelenet a G2-03 szerint már keresett |
| Audit (1.) | → | Minden írás auditált; a szöveges tartalom **nem** kerül az auditba (9.3) |
| Workflow Engine (4.) | – | Nem használ: az import és az elem állapota egyszerű (nincs jóváhagyási folyamat) |
| Shooting Schedule (23.) | később | Bemenet: jelenet (`scene_meta`), `setting`, `casting`, elemhasználat; a 23. **nem** ír a breakdown táblákba |
| Budget (24.) | később | Bemenet: kategóriánként elemhasználat; összeg itt nincs |
| AI-szolgáltatás (`cfa4-ai-registry`) | – | **Nem épít rá** (12. pont) |

## 3. Forrásformátumok és az elemző

### 3.1 Elemző: saját kód, Python, függőség nélkül

- **Fountain** (nyílt szabvány, fountain.io): jelenetfejléc (`INT.`/`EXT.`/`EST.`/`INT./EXT.`/`I/E`, valamint a kényszerített
  `.fejléc`), jelenetszám (`#12A#`), szereplő-névjegy (nagybetűs sor, kényszerített `@`), párbeszéd, zárójeles
  utasítás, akció, váltás, kettős párbeszéd (`^`), jegyzet (`[[…]]`), **boneyard** (`/* … */`, kihagyott), szekció
  (`#`) és szinopszis (`=`), oldaltörés (`===`), címoldal-blokk.
- **Final Draft FDX** (XML): `Paragraph` elemek `Type` szerint (`Scene Heading`, `Action`, `Character`, `Dialogue`,
  `Parenthetical`, `Transition`, `General`, `Shot`); a `Paragraph` jelenetszám-attribútuma és a `SceneProperties`
  `Length`/`Page` adata, **ha jelen van** (a pontos attribútumneveket a G3 a nyilvános FDX-mintafájlokon ellenőrzi, a
  G1 forrásai szerint).
- **Miért saját kód, nem a Screenplain átvétele:** a Fountain szerkezeti szabályai kicsik és meghatározottak; az elemzőnek
  jelenet-szinten **pontos sor- és karakter-eltolást** kell adnia (a szövegbe jelöléshez, 7.3), a Screenplain viszont
  HTML/FDX-kimenetre, nem eltolásra készült. A saját, **determinisztikus, lineáris idejű** elemző (nincs visszalépő
  reguláris kifejezés) a hibatűrést (13.) és az eltolásokat is megadja, és nincs új függőség. A **Screenplain (MIT)** és
  a Better Fountain **referencia és tesztforrás**: a Screenplain tesztfájljait és az FDX-mintákat a mi elemzőnk
  eredményével vetjük össze (G5). Ha a G3 mégis átvételt javasolna, az attribúcióval, külön döntéssel történik.
- **FDX-olvasó:** a Python standard könyvtár `xml.etree.ElementTree`; a forrás **megbízhatatlan**, ezért az elemzés előtt
  elutasítjuk a `<!DOCTYPE` és `<!ENTITY` deklarációt tartalmazó fájlt (`SCRIPT_XML_INVALID`; az FDX nem használ
  DTD-t), és a fájlméret korlátozott. A `defusedxml` **nem** kell.
- **Kódolás:** UTF-8 (BOM-mal vagy anélkül). Érvénytelen UTF-8 → `SCRIPT_ENCODING_INVALID`, a felület kéri az UTF-8-ban
  mentést; **találgatás (pl. cp1250) nincs**, mert csendben elrontaná a hosszú ékezetes neveket.
- **Korlát:** legfeljebb 20 MiB forrásfájl, 2000 jelenet és 500 000 karakter egy jelenetben (túllépés: figyelmeztetés a
  csonkolásról, `SCRIPT_TOO_LARGE` a fájlméretre). A korlátok beállításban emelhetők.

### 3.2 Jelenetfejléc értelmezése

A fejlécből **négy adat** készül: a belső/külső jelölés, a **helynév**, a napszak, és a **nyers fejléc** (mindig
megmarad). A felismerés **zárt, kódban regisztrált nyelvi táblákkal** történik (#16: magyar és angol; további nyelv =
új tábla, nincs találgatás):

| Adat | Angol | Magyar |
|---|---|---|
| Belső | `INT`, `INT.` | `BELSŐ`, `BELS.`, `BELSŐ.` |
| Külső | `EXT`, `EXT.`, `EST.` | `KÜLSŐ`, `KÜLS.`, `KÜLSŐ.` |
| Belső–külső | `INT./EXT.`, `INT/EXT`, `I/E` | `BELSŐ-KÜLSŐ`, `BELSŐ/KÜLSŐ` |
| Nappal | `DAY` | `NAPPAL`, `REGGEL`, `DÉLUTÁN` |
| Éjjel | `NIGHT` | `ÉJJEL`, `ESTE` |
| Hajnal / alkony | `DAWN`, `DUSK`, `SUNRISE`, `SUNSET` | `HAJNAL`, `ALKONY`, `NAPKELTE`, `NAPNYUGTA` |
| Egyéb (`other`) | `CONTINUOUS`, `LATER`, `SAME`, … | `FOLYTATÁS`, `KÉSŐBB`, `UGYANAZ`, … |

- A fejlécet az **utolsó ` - `** választja ketté: bal oldal = jelzés + helynév, jobb oldal = napszak. A helynév a jelzés után
  és a választó előtti szöveg (`INT. HÁZ - KONYHA - ÉJJEL` → helynév `HÁZ - KONYHA`, napszak `night`). Ha a jobb oldal
  **nem** ismert napszó, az egész a helynév, a napszak `other`, a nyers szöveg megmarad, és figyelmeztetés készül
  (`time_of_day_unrecognized`).
- A magyar nyelvű fejléc Fountainben **kényszerített** (`.BELSŐ. KONYHA - NAPPAL`), mert a szabvány csak az angol
  jelzéseket ismeri fel; az elemző a kényszerített fejlécet a magyar táblával is értelmezi, és az **FDX** `Scene Heading`
  mindkét táblát használja.
- Ismeretlen jelzés: `interior_exterior = unspecified`, figyelmeztetés `heading_unrecognized`; a jelenet attól még létrejön.
- A helynevek **összevetése** (azonosság): kisbetűsítés, ékezetek elhagyása (Unicode NFKD), a szóközök egységesítése;
  **hierarchia-felismerés** (`HÁZ - KONYHA` mint a `HÁZ` része) M1-ben nincs.

### 3.3 Oldalhossz nyolcadokban

- Az iparági mérték: egy oldal 8 nyolcad. A forrásformátumok **nem tárolnak oldalhosszt** (a Fountain egyáltalán nem; az FDX csak
  nyomtatási kiosztás után tud adatot adni). Ha az FDX **megadja** a jelenet hosszát, azt használjuk (`page_eighths_source =
  'script'`); különben **becsüljük** (`'estimated'`).
- **Becslés (determinisztikus, a Fountain nyomtatási konvencióját követve):** a jelenet elemeit szokásos szélességre törjük
  (akció 60 karakter, párbeszéd 35, szereplőnév és zárójeles utasítás a szokásos behúzással), a sorok számát összegezzük
  üres sorokkal együtt, egy oldal 55 sor; `nyolcad = max(1, ceil(sorok × 8 / 55))`. A fejléc sorai a jelenethez
  számítanak. Mono-szélességű számítás; ékezetes és egybetűs karakter 1 egység, a **kelet-ázsiai széles karakter 2**.
- A becslés **jelölt becslés** (a felület „~” jellel és tooltippel mutatja), a **tényleges nyomtatott oldalszám eltérhet**; a
  felhasználó **felülírhatja** jelenetenként (`backlot.breakdown.scene.update_meta`), a felülírás a következő importnál
  megmarad, amíg a jelenet szövege változatlan.
- Összegzés: a forgatókönyv oldalszáma = `Σ nyolcad / 8`, tört alakban és tizedesben is kiírva.

## 4. Import-folyamat (javaslat → elfogadás)

### 4.1 Lépések

1. **Indítás** (`backlot.breakdown.import.start`): a `document_version`, a cél **szülő elem** (szekvencia, epizód vagy más
   olyan elem, amely alá a produkció szerkezeti szabálya jelenetet enged), opcionálisan alapimport az összevetéshez (8.). Ellenőrzi:
   a felhasználó látja a dokumentumot (9.1), a verzió `content` fajtájú, a formátum támogatott, a forrás `files.verify`
   szerint ép. **Azonos `(document_version, parser_version)` párra** a meglévő importot adja vissza (idempotens);
   újraelemzés kifejezett parancs (`.reparse`), amely új futást hoz létre.
2. **Elemzés:** háttérfolyamatként, **külön folyamatban**, időkorláttal (13.), folyamatjelzéssel. Az eredmény a 6. pont
   tábláiban **javaslatként** áll: jelenetek, helyszínek, szereplő-névjegyek, figyelmeztetések. **A szerkezet még nem
   változik.**
3. **Felülvizsgálat:** a felhasználó jelenetenként és elemenként dönt (`create` / `link` / `skip`); a szolgáltatás
   **javaslatot ad** a meglévő elemhez (név-egyezés ékezet- és kisbetű-függetlenül); egyértelmű egyezés **előre kitöltött,
   de nem alkalmazott**. A jelenet kódját (`proposed_code`) és a számát szerkesztheti.
4. **Alkalmazás** (`backlot.breakdown.import.apply`): **egy tranzakcióban**, függőségi sorrendben: helyszínek → karakterek →
   jelenetek (szülő alatt, sorrenddel) → `setting` és `casting` kapcsolatok → a jelenet metaadata (5.). Bármely hiba az
   egészet visszagörgeti (`BREAKDOWN_APPLY_CONFLICT`, ha közben megváltozott a cél: `row_version`). **Félig alkalmazott
   import nincs.**
5. **Lezárás:** `status = 'applied'`; az import marad (alap a következő összevetéshez), a javaslati sorok változatlanok,
   az `entity_id` kitöltve.

### 4.2 Létrehozás és kódok

| Elem | Parancs (tulajdonos modul) | Kód |
|---|---|---|
| Jelenet | `backlot.structure.entity.create` | `SC` + a szám (számrész három jegyre kitöltve, a betűs utótag nagybetűvel): `SC012`, `SC012A`; ütközésnél `_2`, `_3` (a javaslatban látható és szerkeszthető) |
| Karakter | `backlot.cast.character.create` (cast ID-val) | `CHR_` + a név ASCII-ra egyszerűsítve (`ÁGNES` → `CHR_AGNES`); ha nem képezhető (nem latin írás): `CHR_<sorszám>` |
| Helyszín (`set`) | `backlot.location.set.create` | `SET_` + a név ugyanígy |
| `setting` kapcsolat | `backlot.location.scene_link.add` | – |
| `casting` kapcsolat | `backlot.cast.link.add` | – |

- A szülőszabály szerint a **jelenet csak megengedett szülő alatt** jöhet létre. A sorozat és a reklám alapsablonjában nincs
  jelenet szint: ilyenkor `BREAKDOWN_PARENT_INVALID`, a hibaüzenet megnevezi a hiányzó szabályt; **a rendszer a produkció
  szerkezeti szabályait nem módosítja csendben**, a felhasználó előbb felveszi a szabályt (G2-03, `structure.rule.*`).
- **Szereplő-névjegy → karakter:** a névjegyből a **kiegészítők** (`(V.O.)`, `(O.S.)`, `(CONT'D)`, `(FOLYT.)`) lekerülnek;
  `BOB` és `BOB (V.O.)` ugyanaz a javaslat. Kisbetűs/ékezet-eltérés összevonva; **hasonló, de eltérő név** (`BOB`,
  `YOUNG BOB`) külön javaslat, a felhasználó köti össze. Több javaslat is mutathat ugyanarra a meglévő karakterre
  (álnév/becenév).
- A **cast ID** kiosztása a G2-19 szerint (legnagyobb + 1), a **fontosság** (`importance`) alapértéke `supporting`; a felhasználó
  az elfogadáskor állíthatja.
- **A `casting` kapcsolat** a jelenetben **megszólaló** karakterekből készül (legalább egy párbeszéd-blokk). **Néma
  szereplés** (akciósorban szereplő, de nem beszélő karakter) **nem** automatikus: a felhasználó a jelenetlapon vagy
  a jelölésnél adja hozzá (`backlot.cast.link.add`). Az akciósorok nagybetűs neveit **nem** találgatjuk (zajos, hamis
  pozitívot adna); a statisztikák ezt kimondják (`character_without_dialogue` csak ha a név máshol szerepel).

### 4.3 Újrafuttatás, elvetés

- Az alkalmazott import nem vonható vissza egyetlen paranccsal (a létrehozott elemek a szerkezet részei; a visszavonás a
  G2-03 kivezetése). A **nem alkalmazott** import `discarded` állapotba tehető, a javaslatai törlődnek (a sorok
  `retired`; `script_import` marad az auditálhatóságért).
- Ugyanabból a forrásból **ismételt import** nem hoz létre duplikátumot: az elfogadás meglévőhöz köti az elemeket (4.1
  3. lépés).

## 5. Jelenet-adatok (`scene_meta`)

A jelenet `production_entity.metadata` mezőjének zárt sémája (`cfa4.backlot.scene_meta/1`, a G2-03 típusonkénti
metaadat-sémájaként regisztrálva, ismeretlen kulcs `ENTITY_METADATA_INVALID`):

```json
{"schema": "cfa4.backlot.scene_meta/1",
 "scene_number": "12A",
 "interior_exterior": "INT",          // INT | EXT | INT_EXT | unspecified
 "time_of_day": "night",              // day | night | dawn | dusk | other
 "time_of_day_note": "FOLYTATÁS",     // a nyers szöveg, ha 'other'
 "page_eighths": 11,
 "page_eighths_source": "estimated"}  // estimated | script | manual
```

- A **szinopszis** a `production_entity.description` (a Fountain `=` sora, vagy kézi).
- **A helyszín** (`set`) a `setting` kapcsolat (G2-20), **a szereplők** a `casting` kapcsolatok (G2-19), **az elemek** a
  `breakdown_usage` (7.): ezeket **nem** tároljuk a metaadatban másodszor (egy hely az igazságnak).
- A `scene_number` **megjelenítési** szám (lehet `12A`, `12B`: beszúrt jelenet); a kód a 4.2 szerint képzett, a sorrend az
  `ordinal`. A szám nem egyedi kulcs (a forgatókönyv hibás is lehet): ismétlődő szám **figyelmeztetés**
  (`duplicate_scene_number`), nem hiba.
- Az **INT/EXT** és a **napszak** a jelenet saját adata; a G2-20 korábbi szövege (a `set` metaadata) ezért javítva.

## 6. Adatmodell (DDL-vázlat, `backlot` séma)

### 6.1 Import és javaslatok

```sql
CREATE TABLE backlot.script_import (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    document_version_id uuid NOT NULL REFERENCES backlot.document_version(id) ON DELETE RESTRICT,
    parent_entity_id uuid NOT NULL REFERENCES backlot.production_entity(id),
    baseline_import_id uuid REFERENCES backlot.script_import(id),
    format text NOT NULL CHECK (format IN ('fountain','fdx')),
    parser_version text NOT NULL,
    source_sha256 text NOT NULL, source_bytes integer NOT NULL,
    status text NOT NULL CHECK (status IN ('parsing','parsed','applied','discarded','failed')),
    scene_count integer, character_count integer, page_eighths_total integer,
    warnings jsonb NOT NULL DEFAULT '[]',                -- [{code, line, detail?}], legfeljebb 1000
    error_code text,
    applied_at timestamptz,
    ...közös oszlopok
);
CREATE UNIQUE INDEX ON backlot.script_import (document_version_id, parser_version) WHERE status <> 'discarded';

CREATE TABLE backlot.script_set_proposal (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    import_id uuid NOT NULL REFERENCES backlot.script_import(id),
    name text NOT NULL, key_norm text NOT NULL, scene_count integer NOT NULL,
    match_entity_id uuid REFERENCES backlot.production_entity(id),     -- javasolt meglévő (set)
    decision text NOT NULL DEFAULT 'pending' CHECK (decision IN ('pending','create','link','skip')),
    entity_id uuid REFERENCES backlot.production_entity(id),            -- link: a választott; create után: a létrehozott
    ...közös oszlopok, UNIQUE (import_id, key_norm)
);

CREATE TABLE backlot.script_character_proposal (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    import_id uuid NOT NULL REFERENCES backlot.script_import(id),
    name text NOT NULL, key_norm text NOT NULL,
    dialogue_blocks integer NOT NULL, scene_count integer NOT NULL,
    match_entity_id uuid REFERENCES backlot.production_entity(id),     -- javasolt meglévő (character)
    decision text NOT NULL DEFAULT 'pending' CHECK (decision IN ('pending','create','link','skip')),
    importance text CHECK (importance IN ('lead','supporting','day_player','background')),
    entity_id uuid REFERENCES backlot.production_entity(id),
    ...közös oszlopok, UNIQUE (import_id, key_norm)
);

CREATE TABLE backlot.script_scene (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    import_id uuid NOT NULL REFERENCES backlot.script_import(id),
    ordinal integer NOT NULL CHECK (ordinal > 0),
    scene_number text, number_source text NOT NULL CHECK (number_source IN ('script','generated')),
    proposed_code text NOT NULL,
    heading_raw text NOT NULL,
    interior_exterior text NOT NULL CHECK (interior_exterior IN ('INT','EXT','INT_EXT','unspecified')),
    time_of_day text NOT NULL CHECK (time_of_day IN ('day','night','dawn','dusk','other')),
    time_of_day_note text,
    set_proposal_id uuid REFERENCES backlot.script_set_proposal(id),
    synopsis text CHECK (length(synopsis) <= 2000),
    body text NOT NULL CHECK (length(body) <= 500000),   -- a jelenet szövege (a jelöléshez); bizalmas
    body_hash text NOT NULL,                              -- sha256, normalizált szövegre
    line_from integer NOT NULL, line_to integer NOT NULL,
    page_eighths integer NOT NULL CHECK (page_eighths >= 1),
    page_eighths_source text NOT NULL CHECK (page_eighths_source IN ('script','estimated')),
    decision text NOT NULL DEFAULT 'pending' CHECK (decision IN ('pending','create','link','skip')),
    entity_id uuid REFERENCES backlot.production_entity(id),     -- a jelenet elem (create után, vagy link)
    ...közös oszlopok,
    UNIQUE (import_id, ordinal)
);
CREATE INDEX ON backlot.script_scene (entity_id) WHERE entity_id IS NOT NULL;
CREATE INDEX ON backlot.script_scene (import_id, body_hash);

CREATE TABLE backlot.script_scene_character (
    scene_id uuid NOT NULL REFERENCES backlot.script_scene(id),
    character_proposal_id uuid NOT NULL REFERENCES backlot.script_character_proposal(id),
    dialogue_blocks integer NOT NULL CHECK (dialogue_blocks > 0),
    PRIMARY KEY (scene_id, character_proposal_id)
);
```

- **A jelenet szövegét az adatbázis tárolja** (a jelöléshez gyors, eltolás-pontos hozzáférés kell; a fájl újraelemzése minden
  megnyitáskor lassú és hibaforrás). Ez a szöveg **a forgatókönyv másolata**, ezért **ugyanazt a hozzáférési szabályt
  követi, mint a forrásdokumentum** (9.1), és az **auditba nem kerül** (9.3).
- A javaslati sorok az import **története**; az alkalmazás után sem törlődnek (az összevetés alapja, 8.).
- A tárolt `body` mérete 120 oldalas forgatókönyvnél kb. 200 KB, 2000 jelenetnél legfeljebb néhány MiB: a mérés a CFA4-MIN
  gépen (15.).

### 6.2 Elemek és jelölés

```sql
CREATE TABLE backlot.breakdown_category (
    id uuid PRIMARY KEY, production_id uuid REFERENCES backlot.production(id),   -- NULL: beépített
    code text NOT NULL CHECK (code ~ '^[a-z][a-z0-9_]{0,38}$'),
    label_key text, name text CHECK (length(name) <= 100),                        -- beépítetten label_key, sajátnál name
    color_token text NOT NULL CHECK (color_token IN ('red','orange','yellow','green','teal','blue','purple','pink','brown','grey','olive','navy')),
    entity_type_code text CHECK (entity_type_code IN ('prop','vehicle','fx','environment')),   -- előléptethető-e elemmé (7.2)
    sort integer NOT NULL DEFAULT 0, retired_at timestamptz,
    ...közös oszlopok production_id nélkül,
    UNIQUE NULLS NOT DISTINCT (production_id, code),
    CHECK ((label_key IS NULL) <> (name IS NULL))
);

CREATE TABLE backlot.breakdown_item (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    category_id uuid NOT NULL REFERENCES backlot.breakdown_category(id),
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    name_key text NOT NULL,                                -- kisbetű, ékezet nélkül, egységes szóköz
    note text CHECK (length(note) <= 2000),
    entity_id uuid REFERENCES backlot.production_entity(id),   -- ha előléptetve (7.2)
    search_vector tsvector GENERATED ALWAYS AS (…) STORED,
    retired_at timestamptz,
    ...közös oszlopok
);
CREATE UNIQUE INDEX ON backlot.breakdown_item (production_id, category_id, name_key) WHERE retired_at IS NULL;

CREATE TABLE backlot.breakdown_usage (                     -- az elem egy jelenetben: ez az igazság
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    scene_entity_id uuid NOT NULL REFERENCES backlot.production_entity(id),    -- trigger: scene típus
    item_id uuid NOT NULL REFERENCES backlot.breakdown_item(id),
    quantity integer NOT NULL DEFAULT 1 CHECK (quantity BETWEEN 1 AND 10000),
    note text CHECK (length(note) <= 1000),
    ...közös oszlopok,
    UNIQUE (scene_entity_id, item_id)
);

CREATE TABLE backlot.breakdown_tag (                        -- a forgatókönyv szövegében kijelölt hely (jelölés)
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    usage_id uuid NOT NULL REFERENCES backlot.breakdown_usage(id) ON DELETE CASCADE,
    script_scene_id uuid NOT NULL REFERENCES backlot.script_scene(id),
    start_offset integer NOT NULL CHECK (start_offset >= 0),    -- karakter-eltolás a jelenet body-jában
    end_offset integer NOT NULL CHECK (end_offset > start_offset),
    text text NOT NULL,                                          -- a jelölt szöveg, az újrahorgonyzáshoz
    state text NOT NULL DEFAULT 'anchored' CHECK (state IN ('anchored','orphaned')),
    ...közös oszlopok
);
```

- **Két szint:** az **elem** (`breakdown_item`) produkció-szintű azonosság („a revolver”), a **használat**
  (`breakdown_usage`) az elem egy jelenetben, mennyiséggel. Az elemjelentés („hol szerepel a revolver”) és a költség- és
  ütemezési bemenet így azonos, nem szövegegyezés.
- **A jelölés (tag) csak jegyzet a szövegen**: a használat az igazság; a jelölés törlése a használatot **nem** törli
  (a felület felajánlja). Ezért a szöveg megváltozása sem veszíti el a breakdownt.
- A beépített kategóriák és színeik (migráció): `extras`, `stunts`, `props`, `costumes`, `makeup`, `set_dressing`, `vehicles`,
  `animals`, `special_effects`, `vfx`, `music`, `sound`, `special_equipment`, `safety`, `notes`. Szereplő és helyszín
  **nem kategória**: azok a `casting` és `setting` kapcsolatokból jönnek (G2-19, G2-20), így nincs kettős tárolás.
- A **szín** zárt **szín-token**, nem hexadecimális kód: a téma (világos/sötét) adja a tényleges színt, így a
  jelenetlap és a felület olvasható marad mindkettőben, és a PDF-be a nyomtatható változat kerül.
- A kategória **konfigurálható** (G1): a produkció saját kategóriát vehet fel, a beépítettet elrejtheti, nem törölheti;
  kategória `retired`, ha van használata, és az elemek megmaradnak.

## 7. Elemek, kategóriák, jelölés

### 7.1 Elem és használat

- **Felvétel:** `backlot.breakdown.usage.set {scene, category, name, quantity, note}` — az elem **név szerint** (az
  azonosságot a `name_key` adja) létrejön vagy a meglévőt használja, majd a használat beíródik egy tranzakcióban. Az
  azonos nevű elem kategóriánként egy; különböző kategóriában lehet azonos név („kés” kellék és „kés” kaszkadőr-eszköz).
- **Összevonás:** `backlot.breakdown.item.merge {from, into}`: a használatok átkerülnek; ha egy jelenetben mindkettő szerepel,
  a mennyiség a **nagyobb**, a megjegyzések összefűzve; az érintett jelenetek **a válaszban**, hogy a felhasználó ellenőrizze.
  A forrás `retired`; megjegyzés: az összevonás nem visszavonható egy paranccsal, de auditált.
- **Nincs törlés, csak visszavonás** (az elem), a használat törölhető.

### 7.2 Előléptetés elemmé

`backlot.breakdown.item.promote`: ha a kategória `entity_type_code`-ot hordoz (kellék, jármű, effekt, környezet), az elem
`production_entity`-ként (G2-03 `prop`/`vehicle`/`fx`/`environment`) is létrejön, és a `breakdown_item.entity_id`
hivatkozik rá. **Nem automatikus**: sok breakdown-tétel („egy pohár”) sosem lesz külön követett elem. Az előléptetés után a
8. lépcső assetként kezelheti. Nem előléptethető elem (jelmez, smink, zene, hang, biztonság, jegyzet) `BREAKDOWN_CATEGORY_INVALID`.

### 7.3 Jelölés a szövegben

- A felület a **jelenet szövegét** mutatja, a felhasználó kijelöl egy szakaszt, kategóriát és (opcionálisan) mennyiséget ad. A
  kijelölés `(start_offset, end_offset)` az elemző által adott **karakter-eltolás** (Unicode kódpont, nem bájt); a rögzített
  `text` a kijelölt szöveg.
- A színes kiemelés a kategória tokenjét használja; egy szakasz **több** jelölést is kaphat (átfedés megengedett).
- A kijelölés **eltolása a megjelenítőben a szöveg ugyanazon verziójához** tartozik (`script_scene_id`); új változatnál az
  újrahorgonyzás a 8. pont szerint.

## 8. Új forgatókönyv-változat és összevetés

Új változatból (új `document_version`) az import az **alapimporthoz** (alapértelmezés: ugyanennek a dokumentumnak a legutóbbi
alkalmazott importja; más dokumentum esetén a felhasználó választ) **hasonlít össze**, mielőtt bármi változna.

**Párosítás (determinisztikus, standard könyvtár, `difflib`):**
1. **Jelenetszám egyezik**, mindkét oldalon a forgatókönyvből jövő szám (`number_source = 'script'`) → pár.
2. A maradékon **azonos `body_hash`** → pár (áthelyezett vagy átszámozott jelenet).
3. A maradékon azonos **normalizált fejléc** és `SequenceMatcher` arány ≥ 0,6 a szövegre (`quick_ratio` előszűrővel; a
   számítás jelenetenként korlátozott méretig) → pár, módosítva.
4. A többi: **új** (nincs pár az újban) vagy **eltávolított** (nincs pár a régiben).

**Az eltérés fajtái** és a javasolt kezelés:

| Fajta | Alkalmazás elfogadáskor |
|---|---|
| `unchanged` | a jelenet új `script_scene` sora ugyanahhoz az entitáshoz kötődik; a jelölések **átmennek** |
| `text_changed` | ugyanez; a jelölések újrahorgonyzása (lent); oldalnyolcad-változás kijelezve |
| `heading_changed` | a `scene_meta` frissül; ha a helyszín nem egyezik a `setting`-gel, **új javaslat a kapcsolatra** (a régit a felhasználó távolítja el) |
| `renumbered` | a `scene_number` frissül, a kód nem (a kód stabil azonosító) |
| `moved` | az `ordinal` a G2-03 `reorder`-jével |
| `added` | új javaslat, mint az első importnál |
| `removed` | javaslat a jelenet **kivezetésére** (G2-03 `retire`); **automatikus törlés nincs**, a kapcsolódó feladat, snitt, használat figyelmeztetésként |
| `cast_changed` | a megszólaló karakterek halmaza eltér: a `casting` kapcsolatokra javaslat (hozzáadás/eltávolítás); a **kézzel felvett néma szereplés** nem érintett |

- **Jelölések újrahorgonyzása:** a rögzített `text` pontos keresése az új jelenetszövegben; több találatnál a **régi
  eltoláshoz legközelebbi**; nincs találat → `orphaned` (a használat megmarad, a felület „nem található a szövegben”
  jelzést ad). Az újrahorgonyzás **determinisztikus** és az eredményét a diff mutatja (hány jelölés megy át, hány árva).
- A felhasználó az eltéréseket **egyenként** vagy csoportosan (fajta szerint) fogadja el (`backlot.breakdown.diff.resolve`), az
  alkalmazás ugyanúgy **egy tranzakció** (4.1/4), és a meglévő breakdown (használatok, elemek) **nem vész el**: az `removed`
  jelenet használatai a kivezetett entitáson maradnak.
- Az összevetés **nem módosít**: előnézete (`backlot.breakdown.diff.get`) bármikor kérhető az alkalmazás előtt.

## 9. Hozzáférés

### 9.1 A forgatókönyv bizalmas

- A forgatókönyv gyakran **a legérzékenyebb anyag** (kiszivárgás, spoiler). Ezért a jelenet szövege (`script_scene.body`), az
  import javaslatai és a **breakdown-adat** (elemek, használatok, jelölések) azokhoz a szerepekhez látható, amelyek a
  **forrásdokumentumot** látják (G2-14 5. pont: `visibility` a típus alapértéke vagy a dokumentum felülírása). Ha egy
  felhasználó a dokumentumot nem látja, a breakdown-adatot és a jelenetszöveget **sem** (és a találat sem jelenik
  meg a keresésben: nincs „létezik, de nem láthatod”).
- A **jelenet-elem** (szerkezeti elem: kód, név, `scene_meta`, a `setting` és `casting` kapcsolat) a G2-03 szerint a produkció
  minden tagjának látható: a szerkezet ismerete a munkához kell, a **szöveg** és az **elemlista** nem része ennek.
- A szerepek: **írás** `owner`, `producer`, `coordinator`; **olvasás** a dokumentum szerepei; az import **indítása** és
  **alkalmazása** `owner`, `producer`, `coordinator`.

### 9.2 Személyes adat

A breakdown **nem tárol személyes adatot** a G2-18 értelmében: a szereplő-névjegy a **karakter** neve (kitalált), nem a
szereplő (`person`). Ha a forgatókönyv valós személyt említ, az a szabad szövegben van, és a G2-18 5.3/1 szabálya szerint
a rendszer nem írja át. A felület a szabad szöveges mezőknél a személyes adat kerülését kéri.

### 9.3 Audit

- A jelenet **szövege, fejléce és szinopszisa** a „bizalmas tartalom” mezők: az auditba **hossz és `body_hash`** kerül, a szöveg
  nem (mint a G2-14 kinyert szövege). Az import, az elfogadás, az elemek és használatok **azonosítókkal és mezőnként
  régi/új értékkel** auditáltak (az elem neve nem bizalmas).
- A jelenetszöveg **megtekintése** auditált (felhasználó, jelenet, időpont; tartalom nélkül), mert a kiszivárgás utólagos
  vizsgálatához ez kell; a lista- és keresési megjelenítés nem.

## 10. Parancsok és hibakódok

| Parancs | Mit csinál |
|---|---|
| `backlot.breakdown.import.start` / `.reparse` / `.get` / `.list` / `.discard` / `.apply` | Import (4.1) |
| `backlot.breakdown.proposal.list` / `.set` / `.accept_unambiguous` | Javaslatok (jelenet, helyszín, karakter): döntés, kód, szám, fontosság; `accept_unambiguous` csak az **egyértelmű** egyezéseket tölti |
| `backlot.breakdown.diff.get` / `.resolve` | Összevetés új változattal (8.) |
| `backlot.breakdown.scene.get` | Jelenet: fejléc, szöveg, szereplők, helyszín, elemek, jelölések |
| `backlot.breakdown.scene.update_meta` | `scene_meta` kézi módosítása (INT/EXT, napszak, nyolcad-felülírás, szám) |
| `backlot.breakdown.category.create` / `.update` / `.retire` / `.list` | Kategóriák |
| `backlot.breakdown.item.list` / `.get` / `.update` / `.merge` / `.retire` / `.promote` | Elemek |
| `backlot.breakdown.usage.set` / `.remove` / `.list` | Használat jelenetenként |
| `backlot.breakdown.tag.add` / `.remove` / `.list` | Jelölés a szövegben |
| `backlot.breakdown.report.items` / `.scenes` / `.sets` / `.pages` | Jelentések (11.) |
| `backlot.breakdown.export` | Jelenetlap (PDF), CSV (11.) |

Hibakódok: `BREAKDOWN_IMPORT_UNSUPPORTED_FORMAT`, `BREAKDOWN_PARENT_INVALID`, `BREAKDOWN_PROPOSAL_INVALID`,
`BREAKDOWN_APPLY_CONFLICT`, `BREAKDOWN_IMPORT_STATE_INVALID`, `BREAKDOWN_DIFF_INVALID`, `BREAKDOWN_CATEGORY_INVALID`,
`BREAKDOWN_ITEM_NOT_FOUND`, `BREAKDOWN_ITEM_DUPLICATE`, `BREAKDOWN_USAGE_INVALID`, `BREAKDOWN_TAG_INVALID`,
`BREAKDOWN_FORBIDDEN` (a dokumentum nem látható), `BREAKDOWN_EXPORT_FAILED`; az elemző hibái: `SCRIPT_SOURCE_MISSING`,
`SCRIPT_TOO_LARGE`, `SCRIPT_ENCODING_INVALID`, `SCRIPT_XML_INVALID`, `SCRIPT_NO_SCENES`, `SCRIPT_PARSE_TIMEOUT`,
`SCRIPT_PARSE_FAILED`; valamint a `FILES_*`, `DOCUMENT_*`, `ENTITY_*`, `STRUCTURE_*`, `CAST_*`, `LOCATION_*`,
`AUDIT_*` kódok változatlanul.

**Figyelmeztetések** (nem hibák; az import `warnings` mezőjében, sorszámmal): `heading_unrecognized`,
`time_of_day_unrecognized`, `duplicate_scene_number`, `empty_scene`, `very_long_scene`, `unclosed_boneyard`,
`character_without_dialogue`, `text_truncated`, `no_scene_heading_before_text` (a szöveg az első fejléc előtt: kihagyva,
számolva), `no_title_page`.

## 11. Jelenetlap és jelentések

- **Jelenetlap (PDF, `QPdfWriter`, a G2-13 mintájára):** jelenetenként egy lap: jelenetszám, INT/EXT, helyszín, napszak,
  oldalhossz (nyolcad, „~” ha becsült), szinopszis, szereplők **cast ID-val** (G2-19), elemek **kategóriánként
  csoportosítva, színnel és mennyiséggel**, megjegyzések. Egy menet: kijelölt jelenetek vagy az egész import; a PDF a
  **nyomtatásra** szánt színeket használja, nem a téma színét.
- **CSV:** `scenes.csv` (jelenet, szám, INT/EXT, napszak, helyszín, nyolcad, szereplők), `items.csv` (kategória, elem, jelenet,
  mennyiség), `characters.csv` (cast ID, karakter, jelenetek száma, párbeszéd-blokkok). **A CSV UTF-8 BOM-mal**, a
  táblázatkezelők ékezetkezeléséhez; a **képlet-injektálás** elleni védelem: a `=`, `+`, `-`, `@` kezdetű cella elé aposztróf.
- **Jelentések** (számolt nézetek, nincs új tábla): elem → jelenetek (`report.items`), jelenet → elemek, helyszín → jelenetek és
  oldalösszeg (`report.sets`), oldalösszeg INT/EXT és napszak szerint (`report.pages`), karakter → jelenetek.
- Az exportok **a hozzáférési szabályt** követik (9.1): nem látható dokumentum breakdownjából nincs export.

## 12. Gépi elemjavaslat: M1-ben nincs

A G1 szerint a gépi elemjavaslat csak a CFA4 AI-szolgáltatás (`cfa4-ai-registry`) elkészülte után jöhet, csak helyi modellel
(SEC-02) és mindig javaslatként. Az AI-szolgáltatás **nem ennek a projektnek a része**, és **nem létezik**; erre a
szabályra (nincs stub, nincs nem létezőre épülő modul) tekintettel **a 22. lépcső nem tartalmaz sem helyettesítőt, sem
üres kapcsolódási pontot**: nincs „javaslat-szolgáltató” interfész, nincs `source = 'ai'` érték (a `breakdown_usage`-nek
ma nincs `source` oszlopa). Az elkészülte után külön G2-bővítés vezeti be (új migráció). A kategória- és elemjavaslat
M1-ben **szabályalapú sem** (a szavak szerepének találgatása zajos); a jelölés kézi.

## 13. Never crash

- **Az elemző külön folyamatban fut** (alfolyamat), **időkorláttal** (alapból 60 s) és memóriakorláttal; hiba vagy túllépés =
  `SCRIPT_PARSE_FAILED` / `SCRIPT_PARSE_TIMEOUT`, az import `failed`, az alkalmazás **változatlan**, a felület él.
- Hiányzó vagy sérült forrás (`FILES_ROOT_UNAVAILABLE`, `FILES_HASH_MISMATCH`), érvénytelen kódolás, rosszul formált XML, DTD:
  angol hibakód, nincs írás.
- **Torz bemenet:** lezáratlan boneyard (`/*`), hiányzó fejléc, üres jelenet, rendkívül hosszú sor, ismeretlen elem, szokatlan
  Unicode (nulla szélességű, kétirányú vezérlő): az elemző **lineáris idejű**, nem akad el és nem dob kezeletlen kivételt;
  eredménye figyelmeztetés; a **kétirányú vezérlő karakterek** a megjelenítésben semlegesítve (nincs megtévesztő szöveg).
- Az `apply` egy tranzakció; közbenső hiba nem hagy félig kész szerkezetet; a tulajdonos modulok parancsai ugyanazokat a
  szabályokat kényszerítik (trigger a végső védvonal, G2-03).
- A jelölés eltolása érvénytelen (kívül esik, felezett szurrogátpár): `BREAKDOWN_TAG_INVALID`.
- A **jelenetszám-ütközés** és a kódütközés nem összeomlás, hanem figyelmeztetés/javaslat (4.2).

## 14. Sorrend és függés

| Rész | Kell hozzá |
|---|---|
| Elemző (Fountain, FDX), import-javaslat | 14. lépcső (dokumentum), File Services FS-1 |
| Alkalmazás: jelenet, helyszín, karakter | 3., 19., 20. lépcső (a tulajdonos modulok parancsai) |
| Elemek, kategóriák, használatok, jelölés | 3. lépcső; az alkalmazás nélkül is használható (kézi jelenetekkel) |
| Összevetés új változattal | az alkalmazott első import |
| Jelenetlap (PDF) | PySide6 `QPdfWriter` (a felületi csomagban) |

A G1 „Előfeltétel: 3., 19., 20. lépcső” **megmarad**, kiegészítve a 14. lépcsővel (a forrás dokumentum) és a File Services
FS-1-gyel.

## 15. Tesztek (G5-re előre)

- **Fountain-elemző:** a szabvány példafájljai és a Screenplain tesztfájljai (referencia: ugyanazokat az elemeket találjuk-e); fejlécfajták
  (`INT.`, `EXT.`, `I/E`, kényszerített `.`, jelenetszám `#12A#`), többtagú helynév (`HÁZ - KONYHA - ÉJJEL`), ismeretlen
  napszak, magyar fejléc, kettős párbeszéd, boneyard (lezárt, lezáratlan), jegyzet, szekció, szinopszis, címoldal.
- **Eltolás:** minden jelenet `body[start:end]` visszaadja a jelölt szöveget (property-teszt véletlen Unicode szövegre, kombináló jelek,
  emoji, szélesszóközök); az eltolás kódpont-alapú.
- **FDX-elemző:** nyilvános mintafájlok; jelenetszám és hossz, ha van; DTD/ENTITY elutasítva; üres és hibás XML; nagy fájl.
- **Oldalnyolcad:** táblázatos teszt (rövid, egy oldal, hosszú jelenet, csak párbeszéd, csak akció, széles karakterek); a becslés
  determinisztikus; a felülírás megmarad változatlan szövegnél, és eldobódik módosult szövegnél.
- **Szereplő-névjegyek:** `BOB`/`BOB (V.O.)`/`BOB (CONT'D)` egy javaslat; `YOUNG BOB` külön; ékezetes nevek; nem latin írás kódja.
- **Párosítás és összevetés:** minden eltérésfajta (új, törölt, átszámozott, áthelyezett, módosított, fejléc-változás, szereplőkör-változás);
  hamis pár nincs azonos szövegű, de különböző jelenetek között; nagy (300 jelenetes) összevetés ideje; a jelölések
  újrahorgonyzása (egy és több találat, árva).
- **Alkalmazás:** egy tranzakció (hibainjektálás a közepén → nincs félig kész szerkezet), jelenet nem engedett szülő alatt
  (`BREAKDOWN_PARENT_INVALID`, nincs csendes szabálymódosítás), kódütközés, meglévő karakterhez/helyszínhez kötés, `casting` csak
  megszólalókra, idempotens újraindítás.
- **Hozzáférés:** nem látható dokumentum breakdownja, szövege, keresési találata és exportja **nem** látszik; a jelenet-elem
  (szerkezet) látszik; a szövegmegtekintés auditált, az audit **nem tartalmaz szöveget**.
- **Elemek:** elem név szerint egyedi kategóriánként, összevonás (mennyiség: nagyobb), előléptetés (csak engedett kategória),
  a jelölés törlése nem törli a használatot.
- **Exportok:** a PDF minden kategóriára (nyomtatási szín), a CSV UTF-8 BOM-mal és képlet-védelemmel (`=SUM(...)` kezdetű elemnév).
- **Never crash:** fuzzolt Fountain és FDX bemenet (véletlen bájtok, hosszú sorok, mélyen egymásba ágyazott XML), időkorlát-teszt
  kényszerített lassú bemenettel.
- **Felületi ellenőrzés:** `ui.screenshot` az import-felülvizsgálatról, a jelölt jelenetszövegről és a jelenetlapról.
- **Mérés** a CFA4-MIN gépen: 150 oldalas és 2000 jelenetes forrás elemzési ideje; 5000 elem és 50 000 használat listázása; az
  összevetés ideje.

## 16. Licenc (G3-ra előre)

Új függőség nincs: az elemző saját kód (visszalépésmentes lineáris szkenner, nincs `re` a szerkezeti elemzésben, `xml.etree`, `difflib`, `hashlib`, `unicodedata`);
a PDF `QPdfWriter` (PySide6). **Screenplain (MIT)** és **Better Fountain (MIT)**: referencia és tesztforrás; ha a G3 kódátvételt
javasol, attribúcióval és külön döntéssel. **Story Architect/Starc (GPL-3.0)**: funkcionális és UX-referencia, kódot nem
veszünk át. StudioBinder, Celtx, Movie Magic Scheduling: funkcionális referencia. A Fountain szabvány (fountain.io) és az FDX
formátum nyilvános dokumentációja a forrás; a Final Draft nevét és formátumát **csak a fájlformátum megnevezésére** használjuk.

## 17. G2-eredmény

**Javaslat: PASS.** Új döntési kérdés nincs. Három pont indokolva: saját Fountain/FDX-elemző (3.1), az INT/EXT és a napszak a
jelenet tulajdonsága (5.; a G2-20 3.1 **javítva**), és a gépi elemjavaslat hiánya (12.). A megvalósítás a 14. lépcső és a
File Services FS-1 után indulhat; az alkalmazás a 3., 19., 20. lépcső parancsaira épül, **nem készít helyettesítő
tárolást** ezekhez. A G2-03 kiegészítése (a `scene_meta` séma regisztrálása és az `ENTITY_METADATA_INVALID` hibakód) megvalósításkor
kerül oda, új tábla nélkül; a G2-14 `subject_kind` listája **nem** bővül (a breakdown nem dokumentum-tárgy).
