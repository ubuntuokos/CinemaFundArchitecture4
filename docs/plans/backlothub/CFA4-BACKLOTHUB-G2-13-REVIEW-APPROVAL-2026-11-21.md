# BacklotHub 13. lépcső – Review és Approval: G2 architektúra (integrációs határok és adatmodell)

**Dátum:** 2026-11-21 · **Terv:** CFA4-BACKLOTHUB-001 1.1, 4.8 · **NFR:** REV-01…08 · **Előzmény:**
[G1 Review és Approval](CFA4-BACKLOTHUB-G1-13-REVIEW-APPROVAL-2026-11-06.md) (funkcionális követelmények
jóváhagyva 2026-11-10) · **Épül rá:** G2-04 (Workflow Engine), G2-05 (közös `comment`), G2-08 (verzió,
reprezentáció, `proxy_profile`), G2-10 (hero-mutató), G2-12 (események), ADR 0007 (Media Services MS-2, MS-3) ·
**Állapot:** G2-javaslat, jóváhagyásra vár · implementáció nincs

Új döntési kérdés nincs, új függőség nincs. Egy pontosítás kell a G2-08 verzió-folyamatán (2. pont), és a
lépcső két külső feltételhez kötött (10. pont): a V1 proxymérésre és a Media Services MS-2/MS-3 fázisára.

## 1. Hatókör ebben a lépcsőben

- **Review-munkamenet** (playlist) verziókból, körökkel.
- **Képkocka- és időalapú megjegyzés**, vektoros **annotáció** (rajz, jelölés).
- **Döntés:** jóváhagyás, módosítás kérése, elutasítás, mindig konkrét verzióhoz kötve.
- **Összehasonlítás:** két verzió egymás mellett, váltva, áttűnéssel (wipe), szinkron léptetéssel (REV-07).
- **Döntési előzmények**, jegyzet-export (PDF, CSV, OTIO-marker).
- **Nem része:** külső (ügyfél-) review hálózaton át (M2/M3; a hozzáférés ott az Identity kérdése), hang- és
  színellenőrző műszerek a review-ban (`crates/scopes`, később), élő közös review több gépen.

## 2. A verzió-folyamat pontosítása (G2-08 4. pont)

A G1 háromféle döntést kért: jóváhagyás, módosítás kérése, elutasítás. A G2-08 `version_default` folyamata
csak `approved` és `rejected` állapotot ad. **Pontosítás:** új állapot és átmenet.

| Átmenet | Honnan → hová | Szerep | Megjegyzés |
|---|---|---|---|
| `approve` | `published` → `approved` | `producer`, `owner` | Hero-mutató frissül (G2-10) |
| `request_changes` | `published` → `changes_requested` | `producer`, `owner` | Megjegyzés kötelező (`WF_COMMENT_REQUIRED`) |
| `reject` | `published` → `rejected` | `producer`, `owner` | Megjegyzés kötelező; a verzió nem folytatódik |

- A `changes_requested` és a `rejected` **végállapot a verzióra nézve**: a folytatás egy új verzió (a verzió
  publikálás után változtathatatlan, G2-08). A különbség jelentés és szűrés: a módosítás kérése az új kör
  alapja (4. pont), az elutasítás nem.
- Az átmeneteket a Workflow Engine futtatja, a Review nem ír `status_id`-t közvetlenül (G2-04). A folyamat
  új változata a megvalósításkor készül (aktiválás után változtathatatlan), mert megvalósítás még nincs, ez
  csak a G2-08 szövegének pontosítása.
- **Őrfeltétel:** a G2-04-ben jelzett `has_approved_version` predikátumot ez a lépcső regisztrálja
  (`has_approved_version {product}`: van-e a terméknek `approved` verziója); a feladat- és elemfolyamatok így
  hivatkozhatnak rá (pl. „a snitt akkor `approved`, ha minden termékének van jóváhagyott verziója”).
- **Önjóváhagyás:** M1-ben a helyi felhasználó minden szereppel bír (#42), így a szerző jóváhagyhatja a saját
  verzióját. Többfelhasználós használatra a G2-04 őrfeltétel-nyelvébe új predikátum kerül (`actor_not_author`),
  amelyet a produkció az aktivált folyamatdefiníciójában kapcsolhat be. Alapból kikapcsolt, mert egygépes
  használatban értelmetlen.

## 3. Integrációs határok

```
 cfa4-backlothub (Qt, Review-felület)
      │  backlot.review.*  ─────────────►  BacklotHub-szolgáltatás ──► PostgreSQL (backlot.*)
      │                                              │  backlot.version.<átmenet> (Workflow Engine)
      │                                              └► outbox → események (12.)
      │  media.service.*  (frame, audio, proxy)
      ▼
 cfa4-cli serve (Rust-motor, ADR 0007) ◄── a lejátszás és a képkocka-kiszolgálás csak itt
```

| Határ | Irány | Szerződés |
|---|---|---|
| Media Services (ADR 0007) | → | `frame`, `audio.render`, `probe`, proxy-job; **időszámítás és konverzió csak itt** (`Tick` ↔ képkocka ↔ időkód, DF is). A BacklotHub Python-kódja nem számol időt |
| Asset (8.) | ← | Verzió, `review_proxy` reprezentáció (`ready`), `frame_in/out`, hero-mutató |
| Workflow (4.) | → | A döntés a verzió átmenete (2. pont); a megjegyzéssel egy tranzakcióban |
| Közös `comment` (5.) | ← | A review-megjegyzés szövege a közös megjegyzéstáblában van (`subject_kind = 'version'`), a képkocka-horgonyt a Review-tábla adja (4.1) |
| Publisher (10.) | ← | Hero-mutató; a `publish.committed` esemény indítja a „review-ra vár” értesítést (12.) |
| Notification (12.) | → | `backlot.version.approved`, `.rejected`, `.changes_requested`, `backlot.comment.created`; új kör létrehozása |
| Interchange (`crates/interchange`) | → | Jegyzet-export OTIO-markerként (8. pont) |
| File Services | → | A lejátszott fájlok helyi útja `files.resolve`-ból; a Review nem ír fájlt, kivéve az exportot (a felhasználó által kért helyre) |

## 4. Adatmodell (DDL-vázlat, `backlot` séma)

### 4.1 Munkamenet, megjegyzés, annotáció

```sql
CREATE TABLE backlot.review_session (
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    name text NOT NULL,
    round integer NOT NULL CHECK (round > 0),
    previous_session_id uuid REFERENCES backlot.review_session(id),   -- az előző kör
    state text NOT NULL CHECK (state IN ('open','closed')),
    closed_at timestamptz,
    ...közös oszlopok
);

CREATE TABLE backlot.review_session_item (
    session_id uuid NOT NULL REFERENCES backlot.review_session(id),
    version_id uuid NOT NULL REFERENCES backlot.version(id) ON DELETE RESTRICT,
    position integer NOT NULL,
    PRIMARY KEY (session_id, version_id),
    UNIQUE (session_id, position)
);

CREATE TABLE backlot.review_session_participant (   -- M1-ben a helyi felhasználó; M2-ben az Identity
    session_id uuid NOT NULL REFERENCES backlot.review_session(id),
    person_id uuid NOT NULL REFERENCES backlot.person(id),
    PRIMARY KEY (session_id, person_id)
);

CREATE TABLE backlot.review_note (                  -- a comment sorához 1:1 horgony
    comment_id    uuid PRIMARY KEY REFERENCES backlot.comment(id),
    production_id uuid NOT NULL REFERENCES backlot.production(id),
    version_id    uuid NOT NULL REFERENCES backlot.version(id) ON DELETE RESTRICT,
    session_id    uuid REFERENCES backlot.review_session(id),
    -- horgony: a verzió médiaidejében, nem a proxy fájlban
    frame_in      integer,                           -- a verzió első kockájától számolva, 0-tól
    frame_out     integer,                           -- NULL = egy kocka
    tick_in       bigint, tick_out bigint,           -- Tick (254 016 000 000/s), ugyanonnan számolva
    fps_num integer, fps_den integer,                -- a horgonyzáskor érvényes képkockasebesség
    timecode_in   text,                              -- megjelenítéshez, a motor számolta
    state         text NOT NULL DEFAULT 'open' CHECK (state IN ('open','resolved')),
    resolved_by_person_id uuid REFERENCES backlot.person(id), resolved_at timestamptz,
    carried_from_comment_id uuid REFERENCES backlot.comment(id),  -- előző körből áthozott jegyzet
    CHECK ((frame_in IS NULL) = (tick_in IS NULL)),
    CHECK (frame_out IS NULL OR frame_out >= frame_in)
);
CREATE INDEX ON backlot.review_note (version_id, frame_in);
CREATE INDEX ON backlot.review_note (session_id) WHERE state = 'open';

CREATE TABLE backlot.review_annotation (
    id uuid PRIMARY KEY, note_id uuid NOT NULL REFERENCES backlot.review_note(comment_id),
    frame integer NOT NULL,                          -- melyik kockán látszik (a jegyzet tartományán belül)
    shapes jsonb NOT NULL,                           -- cfa4.backlot.annotation/1 (5. pont)
    ...közös oszlopok
);

CREATE TABLE backlot.review_decision (               -- a döntés munkamenet-kontextusa; a hiteles állapot a version.status_id
    id uuid PRIMARY KEY, production_id uuid NOT NULL REFERENCES backlot.production(id),
    version_id uuid NOT NULL REFERENCES backlot.version(id) ON DELETE RESTRICT,
    session_id uuid REFERENCES backlot.review_session(id),
    transition_code text NOT NULL CHECK (transition_code IN ('approve','request_changes','reject')),
    comment_id uuid REFERENCES backlot.comment(id),
    decided_by_person_id uuid NOT NULL REFERENCES backlot.person(id),
    decided_at timestamptz NOT NULL,
    audit_seq bigint NOT NULL
);
CREATE INDEX ON backlot.review_decision (version_id, decided_at);
```

**Szabályok (adatbázisban és a parancsrétegben):**

- **A horgony a verzió médiaideje**, nem a proxy-fájl kockasorszáma. Ha a proxyt újragenerálják (G2-08),
  a jegyzet ugyanott marad. Ha az új proxynak más a kockaszáma vagy képkockasebessége, a megjelenítés a
  `tick_in` szerint megy, nem a `frame_in` szerint; a `frame_in` a horgonyzáskor érvényes számozást őrzi.
- **A számítást a motor végzi** (ADR 0007, 4. pont). A `backlot.review.note.add` a felülettől kapja a
  `Tick` értéket és a képkockát (a felület a motor lejátszási állapotából veszi), a szolgáltatás a motor
  `media.service.time` konverziójával ellenőrzi az egyezést. Eltérés: `REVIEW_ANCHOR_MISMATCH`, nincs írás.
  Tartományon kívüli kocka (a verzió `frame_in..frame_out` határain túl): `REVIEW_FRAME_OUT_OF_RANGE`.
- A `bigint` elég: a legnagyobb ábrázolható idő (2⁶³ Tick) ≈ 420 nap.
- A megjegyzés **publikált verzióra is írható** (a verzió változtathatatlan, a jegyzet külön sor).
  `retired` verzióra nem.
- A döntés sorai nem módosíthatók és nem törölhetők (a `review_decision` csak beszúrható; az indok az
  auditlánc és a verzió státuszelőzménye). Megjegyzés **szerkesztése** a közös szabály szerint (G2-05),
  az előzmény az auditban.
- Egy kör lezárása (`closed`) után az új megjegyzés ebbe a munkamenetbe nem kerül; a verzió új körben vagy
  munkamenet nélkül is megjegyezhető.

### 4.2 Döntés egy tranzakcióban

`backlot.review.decide {version, transition, comment?, session?}`:

1. Jogosultság és őrfeltétel a G2-04 szerint (`roles`, `comment_required`, `WF_GUARD_FAILED`).
2. Átmenet a Workflow Engine-nel (`version.status_id`), megjegyzés a közös `comment` táblába
   (`transition_code`-dal, G2-05).
3. `review_decision` sor.
4. `approve` esetén a termék `hero_version_id` mutatója (G2-10); a megelőző hero verzió `approved` marad.
5. Audit és kimenő esemény (12.): `version.approved` / `version.changes_requested` / `version.rejected`.

Egy tranzakció, az audit `record()` az utolsó lépés a COMMIT előtt (G2-01). Ha bármi elbukik, a verzió
állapota változatlan, nincs árva megjegyzés vagy döntéssor.

### 4.3 Több kör

- **Új kör** (`backlot.review.session.next_round`): új `review_session`, `round = előző + 1`,
  `previous_session_id` beállítva. Elemei az előző kör `changes_requested` verzióinak **utódverziói**: ugyanannak
  a terméknek a következő, publikált verziója. Ha még nincs utód, a verzió „vár az új verzióra” jelzéssel
  listázódik, és nem kerül a körbe, amíg nincs publikált utódja (lekérdezéskor számolva).
- **Nyitott jegyzetek átvétele:** az előző kör nyitott (`open`) jegyzetei az új körben megjelennek az utódverzió
  mellett, hivatkozásként (`carried_from_comment_id`), kopizált szöveg nélkül. A felület két nézetet ad: „az előző
  kör jegyzetei” és „az új körben tett jegyzetek”; a felhasználó egy jegyzetet `resolved`-ra állíthat, amikor
  az új verzióban javítottnak látja. A jegyzetet nem a rendszer zárja le.
- A korábbi kör döntései a régi verzión maradnak (G1).

## 5. Annotáció

- **Formátum:** `cfa4.backlot.annotation/1`, zárt alakzatlista JSON-ban, JSON Schema-val validálva:

```json
{"schema": "cfa4.backlot.annotation/1",
 "shapes": [
  {"type": "arrow",  "from": [0.12, 0.40], "to": [0.30, 0.28], "color": "#E5484D", "width": 0.004},
  {"type": "rect",   "min": [0.50, 0.20], "max": [0.70, 0.45], "color": "#E5484D", "width": 0.004},
  {"type": "ellipse","center": [0.40, 0.60], "radius": [0.08, 0.05], "color": "#FFB224", "width": 0.004},
  {"type": "stroke", "points": [[0.1,0.1],[0.12,0.13]], "color": "#46A758", "width": 0.006},
  {"type": "text",   "at": [0.55, 0.80], "text": "ragyog", "size": 0.03, "color": "#FFFFFF"}]}
```

- **Koordináták** normalizáltak (0–1), a **megjelenítési kép** (pixelarány-korrekció után) bal felső sarkától.
  Így ugyanott jelenik meg a proxyn és az eredetin, bármely felbontásban. A pixelarányt a motor `probe` adatából
  veszi a felület.
- Korlátok: legfeljebb 500 alakzat és 256 KiB kockánként; a szöveg legfeljebb 500 karakter, HTML nélkül.
  Túllépés: `REVIEW_ANNOTATION_INVALID`.
- **Megjelenítés:** Qt `QPainterPath` a lejátszó fölötti rétegen. Az annotáció **nem kerül a képbe** (nem
  beégetett), kivéve az exportban (8. pont).
- **Mentés:** egy kocka egy sor; a rajzolás közben a felület memóriában tart, mentés a kocka elhagyásakor vagy
  kérésre (REV-06: ≤ 100 ms helyben).

## 6. Lejátszás és összehasonlítás

- **Lejátszás:** a Media Services `frame.render` és `audio.render` útján, a Qt-szerkesztő lejátszójának mintájára:
  legfeljebb egy kérés repülésben, az elavult kockák eldobva (ADR 0007, 5. pont). A Review nem tartalmaz
  saját dekódert és időalapot.
- **Forrás:** a verzió `review_proxy` reprezentációja (`ready`). Az **eredeti** csak léptetésre és állóképes
  annotációra, nem valós idejű lejátszásra (REV-02, 4K CPU-n).
- **Hiányzó vagy hibás proxy:** `REVIEW_PROXY_MISSING` (nincs vagy `failed`), a felület felajánlja
  a proxy-job indítását (`backlot.representation.create_proxy`, G2-08); sérült fájlnál `FILES_HASH_MISMATCH` a
  `files.verify` után. Összeomlás nincs (REV-08).
- **Proxyformátum:** amíg a `proxy_profile` alapértéke nincs a V1 mérés után kitöltve, `PROXY_PROFILE_UNSET`
  (G2-08); a Review a meglévő proxyt tudja lejátszani, újat nem készít.
- **Összehasonlítás (REV-07):** két verzió, módok: egymás mellett, váltva (A/B), áttűnés (wipe), különbségkép
  (később, ha kell). Szinkron léptetés a **közös Tick-tengelyen**; a két verzió igazítása alapból a
  **kockaindex** (az első kocka a 0.), választható **időkód** szerint, ha mindkettőnek van kezdő időkódja.
  Különböző képkockasebességnél a motor leképezi a Tick-et; a felület jelzi az eltérést.
- A munkamenet nézeti állapota (melyik kocka, melyik mód) **nem tárolódik**; a felület helyi állapota.
- Minden jegyzet a verzióhoz kötődik, amelyiken készült, összehasonlítás közben is (a felület a kiválasztott
  oldalt kérdezi meg).

## 7. Parancsok és hibakódok

| Parancs | Mit csinál |
|---|---|
| `backlot.review.session.create` / `.update` / `.list` / `.close` / `.next_round` | Munkamenetek, körök |
| `backlot.review.session.add_version` / `.remove_version` / `.reorder` | Elemek (verziók) |
| `backlot.review.note.add` / `.update` / `.resolve` / `.reopen` / `.list` | Jegyzetek (tartomány, szöveg, állapot) |
| `backlot.review.annotation.set` / `.get` / `.clear` | Annotáció kockánként |
| `backlot.review.decide` | Döntés (4.2); a felület a verzió átmeneteit mutatja, nem a nyers parancsokat |
| `backlot.review.history` | Döntési előzmény: `review_decision` + audit, verzióra vagy termékre szűrve |
| `backlot.review.export` | PDF, CSV, OTIO-marker (8. pont) |

Az átmenetek `backlot.version.approve` / `.request_changes` / `.reject` néven a G2-04 szerint generált
parancsok; a `review.decide` ezekre épül, nem kerüli meg őket.

Hibakódok: `REVIEW_ANCHOR_MISMATCH`, `REVIEW_FRAME_OUT_OF_RANGE`, `REVIEW_PROXY_MISSING`,
`REVIEW_ANNOTATION_INVALID`, `REVIEW_SESSION_CLOSED`, `REVIEW_VERSION_NOT_IN_SESSION`,
`REVIEW_NO_SUCCESSOR_VERSION`, `REVIEW_EXPORT_FAILED`, valamint a `WF_*`, `FILES_*`, `PROXY_*` és a Media
Services kódjai változatlanul.

## 8. Export

- **PDF:** verziónként a jegyzetek időkóddal és képkockával, a szerzővel, az állapottal és (kérésre)
  az annotált állóképpel. Az állókép a motor `frame` kockájára rajzolt annotáció (Qt `QPainter`, PNG), a
  PDF Qt `QPdfWriter`-rel készül (PySide6-ba tartozik, új függőség nincs; a G3-on ellenőrizendő).
- **CSV:** `version, frame, timecode, tick, author, state, text`; UTF-8, RFC 4180.
- **OTIO-marker:** a `crates/interchange` OTIO-írója ismeri a markereket (clip-marker médiaidőben), de a
  `file.exportInterchange` ma projekt-szekvenciából dolgozik. A BacklotHub-jegyzetekből a marker-export
  ezért **egy projekttől független Rust-parancsot** kíván a `crates/interchange` fölött
  (`interchange.markers_export {media, markers[], path}`, MS-1 mintájára). Ez a Rust-csomagok bővítése,
  nem BacklotHub-kód; **saját allépcső (13-2)**, amely akkor indul, ha a parancs elkészül. Addig a PDF és
  a CSV működik, az OTIO-gomb **nincs a felületen** (nem stub).

## 9. Never crash

- Hiányzó, sérült vagy nem támogatott média, a motor leállása lejátszás közben: kezelt hiba, a felület
  megmarad, a már mentett jegyzetek és annotációk épek (mentés a kocka elhagyásakor, nem csak kilépéskor).
- A motor `serve` folyamata újraindul (a felület `EngineClient`-je); a lejátszó a legutóbbi Tick-től folytatható.
- A döntés tranzakciója félig nem marad meg (4.2).

## 10. Sorrend és függés

| Rész | Kell hozzá |
|---|---|
| Munkamenet, jegyzet, annotáció, döntés (adat- és szolgáltatásréteg) | 2., 4., 5., 8. lépcső; G2-12 az eseményekhez |
| Lejátszás és összehasonlítás | **MS-2** (képkocka-kiszolgálás, két verzió szinkron léptetése) |
| A/V-szinkron és valós idejű lejátszás (REV-01, REV-05) | **MS-3** és az ADR F8 5. lépés |
| Proxy-lejátszás | **V1 mérés** (a `proxy_profile` alapértéke; István: „később, egyeztetett időben”) |
| OTIO-marker (13-2) | `interchange.markers_export` Rust-parancs |
| **V2** (a teljes REV-01…08 a Review felületén, CPU-only és GPU-s gépen) | a Review **G5** kapuja |

## 11. Tesztek (G5-re előre)

- **Képkockapontosság (REV-03):** property-teszt a motor konverziójára minden támogatott fps-en (23,976, 24,
  25, 29,97 DF és NDF, 30), oda-vissza eltérés nélkül; a jegyzet horgonya proxycsere után ugyanarra a Tick-re
  esik.
- **Döntés:** tranzakció hibainjektálással (átmenet után, hero előtt, audit előtt): nincs félkész állapot;
  elavult (más által megváltoztatott) verzión `WF_TRANSITION_NOT_ALLOWED`.
- **Jogosultság:** szerep nélküli felhasználó nem dönthet; `comment_required` az elutasításnál és a módosítás
  kérésénél.
- **Körök:** utódverzió-feloldás (van, nincs, több utód); nyitott jegyzet átvétele; a régi döntés a régi
  verzión marad.
- **Annotáció:** séma-fuzz (érvénytelen alakzat, tartományon kívüli koordináta, túl nagy tartalom); pixelarányos
  és nem négyzetes pixeles forrás ugyanoda esik.
- **Hiányzó proxy, sérült fájl, leálló motor:** kezelt hiba, a jegyzetek épek.
- **Felületi ellenőrzés:** `ui.screenshot` a lejátszóról annotációval, összehasonlító nézetben.
- **V2** a G5-ön: REV-01…08 mérése a CFA4-MIN gépen és GPU-s gépen.

## 12. Licenc (G3-ra előre)

Új függőség nincs (PDF: `QPdfWriter` a PySide6-ból, ellenőrzendő). OpenRV és xSTUDIO (Apache-2.0): összehasonlító
módok, playlist és jegyzet-export mint minta; kódátvételnél attribúcióval. Kitsu/Zou (AGPL-3.0): a „megjegyzés
és státuszváltás egy lépésben”, a playlist és a kockánkénti annotáció mint referencia, kód nem. OpenRV beágyazása
elvetve (saját dekódert hozna a Rust-motor mellé).

## 13. G2-eredmény

**Javaslat: PASS.** Új döntési kérdés nincs; a G2-08 verzió-folyamata a `changes_requested` állapottal pontosodik
(2. pont). A megvalósítás az MS-2 után indulhat, az elfogadás (V2) az MS-3 és a V1 mérés után.
