# CFA4-MEDIA-INTELLIGENCE-001 – Médiaintelligencia: a projekt médiájának helyi elemzése, indexe és keresése

**Verzió:** 1.0 (tervezet; jóváhagyáskor **FINAL 2.0** jelölést kap)
**Dátum:** 2026-11-22
**Projekt:** CinemaFundArchitecture4 (CFA4)
**Állapot:** tervezet, a tulajdonos (`ubuntuokos`) jóváhagyására vár. Fejlesztés nem indulhat, amíg
a tulajdonos jóvá nem hagyja (`docs/coordination.md` §2.10).
**Előzmény:** az eredeti „CFA4-MEDIA-INTELLIGENCE-001 FINAL 2.0” szövege egyik tárolóban sincs meg
(`CFA4-AI-MODEL-TRAINING-001.md` D5; `CFA4-DATA-PG-001-1.0.md` 232. sor). A tulajdonos 2026-11-22-én
úgy döntött, hogy a tervet újra kell írni. Négy alapdöntését ugyanaznap meghozta, ezek a 0. pontban vannak.
Ez a szöveg nem az eredeti visszaállítása. Arra épít, ami a jóváhagyott tervekben a Media
Intelligence-ről megmaradt.
**Kapcsolódik:** [`VISION.md`](../../VISION.md) (2.3: helyi adat; 8: felhős MI nyitott kérdés),
[`CFA4-TASKSTORE-PG-001`](CFA4-TASKSTORE-PG-001.md) 4. pont (`mi` séma, pgvector),
[`CFA4-DATA-PG-001-1.0`](CFA4-DATA-PG-001-1.0.md) 4.5 (a `mi` séma részletei, D-6),
[`CFA4-AI-MODEL-TRAINING-001`](CFA4-AI-MODEL-TRAINING-001.md) 10. fejezet (LLM-es lekérdezés),
[`OBJECT-MASK-PLAN-001`](OBJECT-MASK-PLAN-001-2026-11-03.md) (objektumcímkék),
[`CFA4-SECURITY-001`](CFA4-SECURITY-001.md) (modellworkerek külön folyamatban),
[`CFA4-VIDEO-EXTERNAL-SOURCES-001-1.0`](CFA4-VIDEO-EXTERNAL-SOURCES-001-1.0.md) (adatküldési tilalom),
[ADR 0004](../adr/ADR-0004-compute-routing.md) (`cfa4-route`), [ADR 0006](../adr/ADR-0006-backlothub-postgresql.md),
CFA4-AI_modules ADR 0008 (AI-réteg), [`docs/gaps.md`](../gaps.md) (AI-hiányok),
[`docs/transcripts.md`](../transcripts.md), [`docs/qt-parity.md`](../qt-parity.md) 3.1 (Project panel)

---

## 0. Tulajdonosi alapdöntések (2026-11-22)

| # | Kérdés | Döntés |
|---|---|---|
| A1 | Terjedelem | **Teljes kör.** Az index és a keresés mellett az automatikus címkézés, a jelenet- és hangtípus-felismerés, a hasonló felvételek és duplikátumok, az okos gyűjtemények, az LLM-es magyar lekérdezés és az opcionális arcok/személyek is ebben a tervben vannak. A sorrendről a tulajdonos dönt (2026-11-11-i szabály: a terv nem ír elő ütemezést). |
| A2 | Név és verzió | 1.0 tervezetként kerül a repóba; jóváhagyáskor **FINAL 2.0** lesz. Ugyanabban a PR-ban javítani kell a három terv [nyitott] hivatkozását (7. pont). |
| A3 | Felület | **Mindkettő:** gyorskeresés a médiaböngészőben (Project panel), haladó keresés és indexkezelés saját „Média” munkaterületen. |
| A4 | PostgreSQL nélkül | **Korlátozott mód:** az átirat-, név-, metaadat- és jelölőkeresés a `.fcproj`-ból működik; a vektoros, képes és hasonlósági keresés nincs. A felület megmondja, mi hiányzik, és hogyan kapcsolható be. |

## I. Cél

A felhasználó **gyorsan megtalálja, amit a felvételei között keres**, akkor is, ha több száz órányi
anyaga van: szöveggel („kutya a parton”, „ahol Anna azt mondja, hogy holnap”), képpel (egy
képkockához hasonló felvételek), szűrőkkel (felbontás, kamera, dátum, hangtípus, címke), vagy
magyarul megfogalmazott kérdéssel. A találat mindig **egy klip adott szakaszára** mutat, amely
egy lépésben a forrásnézőbe vagy az idővonalra kerül.

Ehhez a CFA4 a háttérben, helyben elemzi a médiát (átirat, vágás- és jelenethatárok,
hangtípusok, képi címkék, beágyazások, technikai minőség), és az eredményből kereshető indexet épít.

**Nem cél:** felhős elemzés vagy keresés; a felhasználó adatainak bármilyen továbbítása; modell
terjesztése a CFA4 csomagjaiban; saját modell tanítása ennek a tervnek a keretében (az a
CFA4-AI-MODEL-TRAINING-001 dolga).

## II. Kiinduló helyzet (2026-11-22, `main` `bd84c4d`)

**Ami megvan:**

- **Átirat:** helyi Whisper-átírás (`crates/speech`, kiadásokban kikapcsolva, G19); a
  `Project::transcripts` szavanként Tick-határral, beszélővel, nyelvvel; a `transcript.search`
  kifejezés-egyezéssel keres a `.fcproj`-ban (`docs/transcripts.md`).
- **Jelenet- és vágásfelismerés:** `crates/engine` (`scene_detect`), színillesztés.
- **Objektumkövetés:** `crates/objtrack` (Object Mask OM-A4), az objektumcímkék később a `mi`
  sémába indexelhetők (OBJECT-MASK-PLAN-001, 2026-11-20).
- **Adatréteg:** `crates/store` (PostgreSQL 18, `cfa4-db-setup`, migrációk, `tasks` séma),
  `crates/tasks` (közös feladatsor P0/R/P1/P2 osztályokkal), `crates/route` (GPU/NPU/CPU-router).
- **Terv:** a `mi` séma (TASKSTORE-PG-001 4. pont; DATA-PG-001 4.5): `asset`, `project_link`,
  `segment`, `transcript`, `tag`, `embedding_model`, `embedding`, `face_consent`; modellenkénti
  részleges HNSW-index, `tsvector` + GIN `simple` konfiggal; a PG-4 fázis építi meg.
- **Felület:** a Qt6 Project panel kereséssel, List/Icon/Freeform nézettel, metaadat-oszlopokkal
  (qt-parity 3.1), Metadata és Info panel (3.4, 3.11), Text panel átiratkereséssel (3.23).

**Ami hiányzik** (`docs/gaps.md` AI-szakasz, `docs/target-app-parity.md`): médiaintelligencia-keresés,
Auto-Tag Audio Types, automatikus címkézés, hasonlósági keresés, okos gyűjtemények, természetes
nyelvű lekérdezés; a `mi` séma még nincs megépítve (PG-4).

## III. Alapelvek

- **P1 Helyi adat.** Az index, az elemzés eredménye, a lekérdezések és a médiából készült
  bármilyen származék **soha nem hagyja el a gépet** (VISION 2.3). A Media Intelligence nem nyit
  hálózati kapcsolatot; modellt csak a meglévő, jóváhagyásos letöltőút tölthet le (VISION 154,
  K18). *Ez a pont adja azt az „adatküldési tilalmat”, amelyre a VIDEO-EXTERNAL-SOURCES-001 a
  FINAL 2.0-ként hivatkozott.*
- **P2 A projektfájl az igazság.** A `mi` index tisztán származtatott adat (DATA-PG P8, P9): ha
  eltér a `.fcproj`-tól, a `.fcproj` nyer, és az index újraépíthető. A felhasználó kézi döntései
  (kézzel adott címke, okos gyűjtemény, személynév) a projektfájlba kerülnek, nem csak az indexbe.
- **P3 Modellfüggetlenség.** Minden elemző egy implementáció + modell páros, amelyet a súly
  SHA-256-ja azonosít (`ai.model`). Modellcsere nem rontja el a régi találatokat; különböző modellek
  vektorai sosem keverednek egy keresésben (TASKSTORE 4. pont).
- **P4 Feladatként fut.** Minden elemzés és indexelés a közös `tasks` sorban fut: üresjáratban P2,
  a felhasználó kérésére P1; szakaszonként ellenőrzőponttal, megszakítható és folytatható, kétszer
  nem dolgoz fel ugyanazt. Nem lassíthatja a lejátszást és a renderelést (R osztály elsőbbsége).
- **P5 Gyártófüggetlen gyorsítás.** Az elemzők a `cfa4-route` útválasztón át futnak; minden
  elemzőnek kötelező CPU-tartaléka van (2026-10-25-i döntés). NVIDIA nem feltétel.
- **P6 Never crash.** Elemzőhiba, hiányzó modell, hiányzó pgvector vagy PostgreSQL kezelhető
  hiba, nem összeomlás (AGENTS.md §0). Modellworker külön folyamatban, homokozóban fut
  (CFA4-SECURITY-001).
- **P7 Pontos idő.** Minden szakaszhatár `cfa4_time::Tick`, egész szám.
- **P8 Biometrikus adat külön kezelés.** Arcbeágyazás csak kifejezett, projektenkénti
  bekapcsolással, alapból ki, egy lépésben törölhető (TASKSTORE 4. pont; AI ADR 0008).
- **P9 Licenc.** Kód GPL-3.0-or-later. Modell a felhasználó letöltésével, egyszeri
  licencelfogadással; a modell, a súly és a tanítóadat licence rögzítve és megjelenítve
  (AI-MODEL-TRAINING D3). AGPL-es modell (pl. YOLO) kizárt.
- **P10 Minden elérhető a menüből**, és minden beállításnak saját panelje van (2026-10-26).
  Minden interaktív elem automatizálási azonosítót kap; minden művelet parancs (`crates/engine`).
- **P11 Minden felületi nyelv**, az orosz és az ukrán is (2026-11-09).

## IV. Képességek (modulok)

Az MI-A–MI-L modulok a teljes kört írják le (A1). A betűjel nem sorrend.

### MI-A Index és tárolás

A `mi` séma a TASKSTORE-PG-001 4. pontja és a DATA-PG-001 4.5 szerint. **Ezt a PG-4 fázis építi meg
a TASKSTORE/DATA-PG projektben; ez a terv nem veszi át, csak használja** (CLAUDE.md 3. pont: más
projekt feladata nem vehető át). Ez a terv a következőket teszi hozzá, a DATA-PG projekttel
egyeztetve (D12):

- `mi.tag` mellé a címke forrása: `auto` (elemző) vagy `user` (kézi; a projektfájlban is).
- `mi.quality`: technikai mérőszámok szakaszonként (élesség, zaj, expozíció, remegés,
  hangszint, torzítás), a mérő implementációjával.
- `mi.ocr`: képen megjelenő szöveg szakaszonként (MI-H).
- `mi.fingerprint`: perceptuális kép- és hangujjlenyomat a duplikátumkereséshez (MI-F).
- `mi.person` és `mi.face`: csak bekapcsolt `face_consent` mellett (MI-J).

### MI-B Elemzők (az index forrásai)

Mindegyik `tasks` feladat, `kind = analyze.<név>`, saját beállítással (be/ki, modell, osztály).

| Elemző | Mit ad | Alap | Modell |
|---|---|---|---|
| B1 Technikai metaadat | kodek, felbontás, képfrekvencia, HDR, színtér, kamera- és objektívadat, időkód, dátum, GPS ha van | meglévő `metadata.get` | nincs |
| B2 Vágás- és jelenethatár | `segment` (shot, scene) | meglévő `scene_detect` | nincs (első kör) |
| B3 Átirat | `segment` (speech) + `transcript` | `crates/speech` | Whisper-család (G19) |
| B4 Hangtípus (Auto-Tag Audio Types) | `segment` (speech, music, silence, effekt, zaj) + címke | csend meglévő; beszéd/zene/effekt új | kis hangosztályozó, nyílt licenc (D3) |
| B5 Képi címkézés | címkék szakaszonként (tárgy, helyszín, napszak, beállítás: közeli/totál) | új | nyílt licencű képosztályozó (D2) |
| B6 Kép- és szövegbeágyazás | `embedding` (kép, szöveg) | új | nyílt licencű kép–szöveg modell (D1) |
| B7 Objektumok | objektumcímkék és -sávok | `crates/objtrack` (Object Mask) | az Object Mask modellje |
| B8 Képi szöveg (OCR) | `mi.ocr` | új | D4 |
| B9 Technikai minőség | `mi.quality` | új, DSP/heurisztika | nincs (első kör) |
| B10 Ujjlenyomat | `mi.fingerprint` | új, DSP | nincs |
| B11 Arcok | `mi.face` | új, csak hozzájárulással | D5 |
| B12 Jelenetleírás | rövid szöveges leírás szakaszonként | új | helyi kép–szöveg LLM (D6) |

### MI-C Keresés

- **C1 Szöveges keresés:** átirat, címke, név, metaadat, jelölő, OCR, jelenetleírás együtt;
  `tsvector`, `simple` konfiggal, ékezet- és kisbetű-érzéketlen illesztéssel.
- **C2 Szemantikus szöveg → kép keresés:** a lekérdezés szövegbeágyazása a kiválasztott
  modellel, HNSW-keresés.
- **C3 Kép → kép keresés:** egy képkocka vagy kijelölt terület alapján („ehhez hasonló”).
- **C4 Szűrők:** metaadat (B1), hangtípus, címke, minőség, időtartam, projekt, bin, dátum,
  személy (csak MI-J mellett).
- **C5 Rangsorolás:** a C1–C3 találatainak összefésülése (D7: módszer, pl. reciprok rangösszeg),
  a találat indoklásával („átiratban: …”, „címke: kutya 0,91”).
- **C6 Eredmény:** klip + szakasz (Tick), bélyegkép, hover scrub; egy lépésben forrásnézőbe,
  idővonalra (beszúrás/felülírás), binbe, vagy In/Out-ként a klipre.
- **C7 Parancsok:** `mi.search`, `mi.similar`, `mi.index.status`, `mi.index.run`,
  `mi.index.delete`, `mi.collection.*`, `mi.person.*`; CLI-ből, vezérlőcsatornán és MCP-n is.

### MI-D Felület (A3)

- **D-1 Gyorskeresés a Project panelen:** a meglévő keresőmező kap egy „tartalomban is” kapcsolót;
  bekapcsolva a C1 (és ha van, C2) találatai szakaszokként jelennek meg a bin tartalma alatt.
- **D-2 „Média” munkaterület** (Window ▸ Workspaces, Alt+Shift+billentyű a D8 szerint): haladó
  keresés (C1–C5 minden szűrővel), találati rács, előnézet, „ehhez hasonló”, okos gyűjtemények,
  indexállapot.
- **D-3 Indexkezelő panel:** mely médiák indexeltek, mely elemző fut vagy vár, becsült idő,
  szünet/folytatás, újraindexelés, törlés; a modellek listája licenccel.
- **D-4 Beállítások panel** (Preferences ▸ Media Intelligence): elemzők be/ki, osztály
  (üresjárat/azonnal), modellválasztás, erőforráskeret, arcok hozzájárulása projektenként,
  korlátozott mód magyarázata.
- **D-5 Menü:** File ▸ Media Intelligence (indexelés indítása, kijelölt média elemzése),
  Edit ▸ Search in Media (Ctrl+Shift+F, D8), Window ▸ Media Intelligence panelek; jobb klikk a
  Project panelen: „Elemzés”, „Hasonló felvételek”, „Index törlése”.
- **D-6 Metadata panel:** a kiválasztott klip címkéi (automatikus és kézi), kézi címke hozzáadása,
  automatikus címke elvetése (a döntés a projektfájlba kerül, P2).

### MI-E Okos gyűjtemények

Mentett keresés binként: a tartalma a lekérdezés szerint frissül (új anyag indexelésekor is).
A lekérdezés a projektfájlban tárolódik (P2), az eredmény az indexből jön. Korlátozott módban a
`.fcproj`-ból kiszolgálható feltételek működnek, a többiről a bin jelzést ad.

### MI-F Hasonlóság és duplikátumok

- **F1 Hasonló felvételek:** C3 egy teljes klipre vagy szakaszra.
- **F2 Duplikátumok:** azonos tartalomhash (már az asset-azonosító), átkódolt vagy vágott másolat
  ujjlenyomattal (B10); jelentés a felhasználónak, törlés nélkül.
- **F3 Felvételváltozatok (take-ek) csoportosítása:** azonos beállítás egymás utáni felvételei
  (kép- és szöveghasonlóság + időkód/dátum); javaslat, a csoportosítást a felhasználó fogadja el.

### MI-G LLM-es lekérdezés és leírás

- **G1** Magyar (és minden felületi nyelvű) kérdésből strukturált keresési paraméterek
  („a tavalyi esküvői anyagból azok a közeli képek, ahol nevetnek”) → C1–C4. A felhasználó látja
  és szerkesztheti a kapott szűrőket, mielőtt futnak.
- **G2** Jelenetleírás (B12) a kereséshez és az akadálymentes leíráshoz.
- Helyi modell, opcionális, alapból ki, amíg a modell nincs letöltve; LLM nélkül C1–C5 változatlanul
  működik. A modellválasztás és a finomhangolás a CFA4-AI-MODEL-TRAINING-001 10. fejezete szerint.

### MI-H Képi szöveg (OCR)

Feliratok, táblák, inzertek, stáblisták szövege kereshetően, szakaszhoz kötve; a meglévő feliratsávok
és grafikai rétegek szövegét nem OCR-rel, hanem a projektből olvassa.

### MI-I Korlátozott mód (A4)

PostgreSQL nélkül: C1 a `.fcproj` adataiból (átirat, klipnév, metaadat, jelölők, kézi címkék,
grafikai szövegek), C4 a B1 metaadataiból; okos gyűjtemények a kiszolgálható feltételekkel. A
C2/C3, MI-F, MI-G, B5–B12 nem érhető el; a felület jelzi, és a `cfa4-db-setup`-ra mutat. pgvector
nélkül (de PostgreSQL-lel) csak a vektoros rész esik ki (TASKSTORE 4. pont).

### MI-J Arcok és személyek (opcionális)

Csak `mi.face_consent.enabled = true` mellett, projektenként. Arcfelismerés → csoportosítás →
a felhasználó nevet adhat (a név a projektfájlba kerül). Egy lépésben törölhető minden arcadat
projektenként. A csoportosítás javaslat, soha nem automatikus azonosítás külső adatbázis alapján.
Az arccserére (38. típus) vonatkozó szabályok (AI ADR 0008) nem változnak.

### MI-K Erőforrás és ütemezés

Az ACS-keret szabja meg a memóriát, a GPU/NPU-t és a kötegméretet a 16 GB-os minimumgépen is; a
HNSW-építés a többi feladattal együtt ütemeződik. Akkumulátoron vagy lejátszás közben a P2
elemzés szünetel. Mérés a referenciagépen (RTX A1000) és CPU-tartalékkal.

### MI-L Későbbi bővítések (a teljes kör része, sorrend a tulajdonosé)

- L1 Index megosztása több gép között / a BacklotHub-bal (ADR 0006 klasztere).
- L2 Index exportja és importja (projekt átvitele másik gépre újraelemzés nélkül).
- L3 Szkript- és forgatókönyv-összevetés: a forgatókönyv jeleneteihez illeszkedő felvételek (a
  CFA3-ban előkészített forgatókönyv-modulokkal összhangban, külön döntés után).
- L4 Beszélőazonosítás név szerint (a B3 beszélőcímkéiből, MI-J-hez hasonló hozzájárulással).
- L5 Automatikus válogatás (selects) javaslata: legjobb minőségű take-ek, MI-F3 + B9 alapján.
- L6 Keresés az élő stúdió (CFA4-LIVE-STUDIO-001) felvett anyagában rögzítés közben.

## V. Követelmények

| # | Követelmény | Elfogadási teszt |
|---|---|---|
| R1 | Az index nem hagyja el a gépet; a Media Intelligence nem nyit hálózatot | homokozóprofil-teszt: a workerek hálózati hívása elutasítva; Semgrep-szabály |
| R2 | Minden elemző feladatként fut, megszakítás után folytatódik, kétszer nem dolgoz fel | `cfa4-tasks` + `cfa4-store` integrációs teszt: megszakított indexelés folytatása, azonos eredmény |
| R3 | A találat klip + Tick-szakasz, egy lépésben forrásnézőbe / idővonalra kerül | engine-teszt `mi.search` → `source.open` / `timeline.insert`; Qt-teszt a vezérlőcsatornán |
| R4 | Szöveges keresés átiratban, címkében, névben, metaadatban, OCR-ben | ismert mintakészlet: az elvárt szakasz a top 5-ben |
| R5 | Szemantikus és képes keresés modellenként elkülönítve | mintakészlet; modellcsere után a régi találatok változatlanok, a két modell vektorai nem keverednek |
| R6 | Korlátozott mód PostgreSQL nélkül, kezelhető hibákkal | teszt szerver nélkül: C1 működik, C2 kezelhető hibát ad, nincs összeomlás |
| R7 | pgvector nélkül csak a vektoros rész esik ki | teszt pgvector nélküli klaszteren |
| R8 | Az index a `.fcproj`-ból újraépíthető; eltérésnél a projekt nyer | újraépítési teszt `source_rev`-vel, idempotencia |
| R9 | Kézi címke, elvetett címke, okos gyűjtemény, személynév a projektfájlban | mentés–betöltés teszt DB nélkül és DB-vel |
| R10 | Arcadat csak hozzájárulással, egy lépésben törölhető | teszt: hozzájárulás nélkül nincs `mi.face` sor; törlés után nulla sor |
| R11 | Elemzés nem lassítja a lejátszást és a renderelést | referenciagép: képkockaidő mérése indexelés közben és nélküle, eltérés a mérési zajon belül |
| R12 | Minden elemzőnek CPU-tartaléka van, GPU/NPU a routeren át | router-teszt GPU nélkül |
| R13 | Minden funkció menüből elérhető, saját beállítási panellel, automatizálási azonosítókkal | Qt-teszt a menüre és a panelekre; automatizálási id-lista |
| R14 | Minden felületi nyelv (ru, uk is) | fordítási katalógus-ellenőrzés |
| R15 | Modell csak jóváhagyott letöltéssel, licenc rögzítve és megjelenítve; AGPL kizárva | modelljegyzék-teszt a négy licencmezőre |
| R16 | Duplikátumjelentés törlés nélkül | teszt átkódolt másolattal |
| R17 | LLM-es lekérdezés: a szűrők láthatók és szerkeszthetők futás előtt; LLM nélkül minden más működik | Qt-teszt; teszt modell nélkül |
| R18 | Never crash: hibás médiára, hibás modellre, megszakadt workerre kezelhető hiba | fuzz-célpont a szegmens- és átiratbetöltőre; worker-leállítási teszt |

## VI. Fázisok és PR-bontás

A fázisok a függőségeket mutatják, **nem ütemezést** (A1). Mit és mikor építünk be, a tulajdonos
dönti el (D10). Minden PR egy témát fed (coordination §2.2), a `Munkamenet:` sorral (§2.10a).

| Fázis | Tartalom | Előfeltétel | PR-ok (fájlok) | CodeRabbit (§2.9) |
|---|---|---|---|---|
| MI-0 | Csak olvasó állapotfelmérés: meglévő elemzők, adatok, a PG-4 állása, mintakészlet összeállítása (`target/fixtures`, nem kerül a repóba) | jóváhagyás | 1 PR: `docs/audit/MI-0-*.md` | nem |
| MI-1 | Elemzőkeret: `analyze.*` feladatfajták, eredményformátum, `crates/mi` (L4 körül, D9) | MI-0 | `crates/mi`, `crates/tasks` regisztráció | nem |
| MI-2 | B1, B2, B3, B4, B9 az indexbe; C1 + C4 + korlátozott mód (MI-I); `mi.search` parancs | MI-1; a PG-4 (más projekt) a DB-s részhez | 2–3 PR: elemzők; keresés; korlátozott mód | nem |
| MI-3 | Gyorskeresés a Project panelen (D-1), Metadata panel címkék (D-6), menü (D-5) | MI-2; Qt6 GUI kész (D11) | `python/cfa4_qt/…` | nem |
| MI-4 | B6 beágyazás, C2, C3, C5; modellválasztás (D1) | MI-2, PG-4, AI-F4 kapu | 2 PR: beágyazó worker; keresés | igen (modellworker, homokozó) |
| MI-5 | „Média” munkaterület (D-2), indexkezelő (D-3), beállítások (D-4) | MI-3, MI-4 | 2 PR | nem |
| MI-6 | Okos gyűjtemények (MI-E) | MI-2, MI-5 | 1 PR (engine + Qt) | nem |
| MI-7 | B10 ujjlenyomat, MI-F hasonlóság, duplikátum, take-csoportosítás | MI-4 | 2 PR | nem |
| MI-8 | B5 képi címkék, B7 objektumcímkék, B8 OCR | MI-4; Object Mask OM-A | 3 PR | igen (új modellworkerek) |
| MI-9 | MI-J arcok és személyek | MI-4; D5 | 1–2 PR | igen (biometrikus adat, adatvédelem) |
| MI-10 | MI-G LLM-es lekérdezés és B12 jelenetleírás | MI-4; AI-MODEL-TRAINING TR-F10 | 2 PR | igen (modellworker) |
| MI-11 | MI-L későbbi bővítések, egyenként külön döntéssel | tulajdonosi döntés | külön tervrészek | eseti |

Minden fázis a végén frissíti a `docs/gaps.md`, a `docs/target-app-parity.md` és az érintett
crate README-jét.

## VII. Követelmény → teszt → fázis

| Követelmény | Teszt helye | Fázis |
|---|---|---|
| R1 | `crates/sandbox` profilteszt, Semgrep | MI-1, MI-4 |
| R2, R8 | `crates/mi` + `crates/store` integrációs teszt | MI-1, MI-2 |
| R3, R4 | `crates/engine` parancsteszt, mintakészlet | MI-2 |
| R5 | mintakészlet, modellcsere-teszt | MI-4 |
| R6, R7 | szerver nélküli és pgvector nélküli teszt | MI-2, MI-4 |
| R9 | projekt mentés–betöltés teszt | MI-3, MI-6, MI-9 |
| R10 | `mi.face` teszt | MI-9 |
| R11, R12 | referenciagép-mérés (`docs/development/reference-machine.md`), router-teszt | MI-2, MI-4, MI-8 |
| R13, R14 | Qt-tesztek, fordításellenőrzés | MI-3, MI-5 |
| R15 | modelljegyzék-teszt | MI-4, MI-8, MI-9, MI-10 |
| R16 | ujjlenyomat-teszt | MI-7 |
| R17 | Qt-teszt, modell nélküli teszt | MI-10 |
| R18 | fuzz-célpont, worker-leállítás | MI-2, MI-4 |

## VIII. Nyitott tulajdonosi döntések

Ajánlással; amíg nincs döntés, az érintett rész nem épül.

| # | Kérdés | Ajánlás |
|---|---|---|
| D1 | Az első kép–szöveg beágyazó modell | Az AI-F4 kapunál mérés dönt nyílt licencű jelöltek közül (súly- és adatlicenc rögzítve); `halfvec`, ha a dimenzió 2000 fölött van |
| D2 | Képi címkéző modell és szókészlet | Nyílt licencű osztályozó; a szókészlet fordítható katalógusban, nem a modellben |
| D3 | Hangtípus-osztályozó | Kis, nyílt licencű modell CPU-n; addig a csend DSP-vel |
| D4 | OCR-motor | Külön folyamatban futó, nyílt licencű motor (a „nem Rust maradhat, ha nem lassít” szabály szerint) |
| D5 | Arcmodell, és belekerül-e egyáltalán | Igen, MI-J szerint, alapból ki; nyílt licencű modell |
| D6 | LLM a lekérdezéshez és a jelenetleíráshoz | A CFA4-AI-MODEL-TRAINING-001 döntései szerint; alapból ki |
| D7 | Rangsorolási módszer | Reciprok rangösszeg, mérés a mintakészleten |
| D8 | Billentyűparancsok és a munkaterület neve | „Média”, Alt+Shift+7; keresés Ctrl+Shift+F, ha nem ütközik |
| D9 | Új `crates/mi` vagy a meglévő crate-ek bővítése | Új `cfa4-mi` crate a rétegszabályok szerint; az adatbázis-hozzáférés a `cfa4-store`-ban marad |
| D10 | Mely fázisok és milyen sorrendben | A tulajdonos dönti el; javaslat: MI-0 → MI-1 → MI-2, a többi a Qt6 GUI után |
| D11 | Megvárja-e a felületi rész a Qt6 GUI befejezését | Igen (a Qt6 elsőbbsége, 2026-11-09); a motoroldali MI-0–MI-2 indulhat előbb |
| D12 | A `mi` séma bővítése (MI-A: `quality`, `ocr`, `fingerprint`, `person`, `face`, címkeforrás) | A DATA-PG projekt felelőse veszi fel a saját PG-4 PR-jaiba, vagy egy később külön migráció ebben a projektben; a gazdával egyeztetve (§2.8) |
| D13 | Külön munkamenet | Igen, saját munkamenetben, mint a SECURITY és a LIVE-STUDIO |

## IX. Kockázatok

| Kockázat | Hatás | Kezelés |
|---|---|---|
| A PG-4 később készül el | a DB-s rész vár | korlátozott mód (MI-I) addig is ad keresést |
| Modelllicenc nem tisztázható | elemző kimarad | P9; modellenként külön döntés |
| Indexelés lassítja a szerkesztést | felhasználói élmény | P4, R11, ACS-keret, P2 osztály |
| Nagy archívum (100+ óra) indexelési ideje | hosszú első indexelés | szakaszos, folytatható feladat; előbb B1–B3, a drágább elemzők később |
| Arcadat visszaélés | jogi/etikai | P8, R10, alapból ki, egy lépéses törlés |

## X. Jóváhagyáskor elvégzendő

1. A verzió **FINAL 2.0**, az állapot „jóváhagyva” a döntések dátumával.
2. A három [nyitott] hivatkozás javítása erre a fájlra mutató hivatkozással:
   `CFA4-AI-MODEL-TRAINING-001.md` D5; `CFA4-DATA-PG-001-1.0.md` 232., 554., 598. sor;
   `CFA4-VIDEO-EXTERNAL-SOURCES-001-1.0.md` 56. és 545. sor (az adatküldési tilalom: III. P1).
3. Sor a `docs/audit/README.md` tartalomjegyzékében.

## Változásnapló

| Verzió | Dátum | Változás |
|---|---|---|
| 1.0 | 2026-11-22 | Első, újraírt tervezet a tulajdonos A1–A4 döntéseivel; az eredeti FINAL 2.0 szövege elveszett |
