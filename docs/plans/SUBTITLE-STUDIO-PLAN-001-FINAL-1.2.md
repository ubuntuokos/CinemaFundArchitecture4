# CFA4-SUBTITLE-STUDIO-PLAN-001 — FINAL 1.2

## Professzionális feliratozó munkakörnyezet — egységes, végleges, jóváhagyott terv

| Dokumentumadat | Érték |
|---|---|
| Verzió | FINAL 1.2 |
| Állapot | **Jóváhagyva** a tulajdonos által, 2026-10-30; a D1–D4 döntések beépítve. A FINAL 1.1-et e változat váltja fel |
| Dátum | 2026. október 30. |
| Projekt | CFA4 — CinemaFundArchitecture4 |
| Tulajdonos, jóváhagyó | ubuntuokos (Horváth H. István) |
| Érvényes alap | [FINAL 1.1](SUBTITLE-STUDIO-PLAN-001-FINAL-1.1.md) (jóváhagyott; a tárolóba vételkor csak az upstream márkanév lett semleges „upstream” szóra cserélve, `AGENTS.md`) |
| Beolvasztott javaslat | „1.2 JAVASLAT — CFA3–CFA4 összevetés és kiegészítési javaslat” (2026-10-30, nem jóváhagyott) |
| Új ebben a változatban | Összevetés a CFA4 `main` tényleges kódjával (`a3886a4`, 2026-10-30) és a `VISION.md`-vel |
| Végrehajtás | A fejlesztés nem indul el ezzel a dokumentummal; az F0 indítása külön tulajdonosi döntés |

## 0. Összefoglaló

Az 1.1 funkcionális célja (a Kdenlive feliratozójánál professzionálisabb, Subtitle Edit-szintű
munkakörnyezet) és technikai alapja (Rust-motor, Qt6-felület, egész alapú idő, adatmegőrzés,
NLE-szabályok, AC-01–12, F0–F8) **változatlanul érvényes**. Az 1.2 javaslatból ez a változat
átveszi: az önállóan indítható feliratozó belépést, a külön narrációs felelősségi kört, a
verziózott átadást, a revízióütközés kezelését és a fordítás/narráció elavulási szabályait.

A két korábbi dokumentum egyike sem vizsgálta a CFA4 kódját (1.1 §1, 1.2 §8). Ez a változat
megtette, és a legfontosabb eredménye: **a CFA4-ben már működő feliratozó alap van**. A terv ezért
nem új motor és új adatmodell építését írja elő, hanem a meglévő `cfa4-project` feliratmodell és
`cfa4-captions` crate **bővítését**, a meglévő `captions.*`, `transcript.*` és `tts.*` parancsokra
építve.

Négy pont tulajdonosi döntést igényelt; a tulajdonos 2026-10-30-án döntött róluk (9. fejezet):
a feliratozó **külön alkalmazásként is indítható**, és a CFA4-en belül is a helyén van (D1); a
fejlesztés **a Qt6-felület elkészülte után azonnal** indul (D2); külső fordítás **csak
kapcsolható, külön folyamatként** (D3); egy nyelv (például a magyar) hangjának megléte **nem
kiadási feltétel** (D4).

## 1. Források és bizonyítottság

| Jel | Forrás | Mit bizonyít |
|---|---|---|
| S1 | [FINAL 1.1](SUBTITLE-STUDIO-PLAN-001-FINAL-1.1.md) | A jóváhagyott követelmények, kapuk és elfogadási feltételek |
| S2 | 1.2 JAVASLAT (CFA3-összevetés) | A FA3/CFA3 előzményekből levezetett kiegészítések; maga is javaslat |
| S3 | CFA4 `main` @ `a3886a4`: `docs/captions.md`, `crates/captions/README.md`, `crates/project/src/caption.rs`, `docs/transcripts.md`, `crates/speech`, `crates/tts`, `crates/engine/src/narration.rs`, `docs/qt-parity.md` 3.23 | A ma meglévő feliratozó, átirat- és narrációs képességek (dokumentáció és forrás olvasásával; build és teszt ebben a munkában **nem futott**) |
| S4 | `VISION.md`, `AGENTS.md`, `docs/coordination.md`, ADR F8, ADR 0004, ADR 0005, `docs/licensing.md` | A kötelező keretek |
| S5 | `ubuntuokos/CFA4-AI_modules`: ADR 0006 (AI-feldolgozó réteg, elfogadva 2026-10-30) | Az ASR, fordítás és modellkezelés közös rétege |
| S6 | `ubuntuokos/CFA3-CinenemFoundArchitecture3` (`a021ef3`, 2026-10-16) README | A CFA3-ban a Subtitle Studio és a Narration Studio **külön felület**; részletes feliratozó terv a nyilvános tárolóban nincs |

Az S6 megerősíti az 1.2 javaslat megállapítását: az eredeti FA3/CFA3 feliratozó terv teljes
szövege nem került elő (MISSING_SOURCE). A CFA3-ból átvett követelmények ezért az S2-ben
rögzített, visszakeresett kivonatokra támaszkodnak, és nem pótolhatók kitalált követelménnyel
(AC-22).

## 2. Rögzített keretek (az 1.1 2. fejezete, a `VISION.md`-hez igazítva)

| Terület | Követelmény | Változás az 1.1-hez képest |
|---|---|---|
| Termék | A CFA4 része; **külön alkalmazásként is indítható**, és a CFA4-en belül is elérhető (D1, tulajdonosi döntés 2026-10-30) | Az 1.2 javaslata szerint, a 4.1 megvalósításával |
| Projekt | Minden fejlesztés a CFA4-be kerül; upstream módosítása nélkül | — |
| Platform | Ubuntu 26.04 LTS minden grafikus változata, AMD64; Wayland és X11 (`VISION.md` 3.) | Bővítés: nem csak „Ubuntu és variánsai”, hanem a 3. pont tesztmátrixa |
| Felület | **Csak Qt6** (Qt6 Widgets + PySide6). Az egui befagyasztva: feliratozó újdonság nem kerül bele (tulajdonosi döntés, 2026-10-24) | Új kifejezett szabály |
| Motor | A meglévő Rust-motor (`crates/engine`) parancsai; minden művelet parancs, MCP-ről és CLI-ről is vezérelhető (`CLAUDE.md`) | Pontosítás: nem külön „szerkesztőmotor”, hanem a meglévő parancsrendszer bővítése |
| Idő | `cfa4_time::Tick` (254 016 000 000/s, egész). Lebegőpontos másodperc szerkesztési számításban tilos | Az 1.1 §5 követelménye már teljesül (3.3) |
| Média | Saját Rust-motor az alap, FFmpeg (≥ 9.0.2) és GStreamer kiegészítő háttér az útválasztón keresztül (ADR 0005, ADR 0004) | Hivatkozás pontosítva |
| AI | Helyi, CPU-s tartalékkal; gyorsítás **gyártófüggetlenül** (VA-API/Vulkan/NPU mellett NVIDIA), ADR 0004 útválasztón és ADR 0006 rétegen keresztül | Bővítés: gyártófüggetlenség és ADR 0006 |
| Licenc | GPL-3.0-or-later; GPL-3-kompatibilis kód átvehető fejléccel és nyilvántartási sorral (`docs/licensing.md` §3, §5); GPL-2.0-only, AGPL, licenc nélküli és zárt kód tilos | Új kifejezett szabály |
| Hibatűrés | `AGENTS.md` §0: minden fájl, paraméter és állapot ellenséges bemenet; nincs `unwrap`/`panic`; minden parser mutációs fuzz-tesztet kap | Új kifejezett szabály |
| OxiMedia | Kísérleti jelölt; az alapműködés nem függhet tőle | — |

## 3. A CFA4 mai állapota (S3) és az 1.1 céljainak összevetése

### 3.1. Ami már megvan

| Terület | Meglévő CFA4-képesség | Hol |
|---|---|---|
| Adatmodell | `CaptionTrack` (név, formátum, nyelv, stílus, kimenet, zár, sync lock) és `Caption` (stabil `id`, `start`/`duration` `Tick`-ben, szöveg, beszélő, WebVTT cue id és beállítások, `vtt_blocks`); a `.fcproj` része | `crates/project/src/caption.rs` |
| Fájlformátumok | SRT, WebVTT, SCC (CEA-608), MCC (CEA-608 + 708), EBU STL, TTML/IMSC1, DFXP — **olvasás és írás**, képkockapontosan, property-tesztekkel | `crates/captions` |
| Szerkesztés | `captions.add/split/merge/setText/setTimes/trim/move/delete/select/goTo/setTrack/setStyle/import/export/list`, undo | `docs/captions.md` |
| NLE-kapcsolat | Sync-lockolt feliratsáv követi a beszúrást és kivágást; „Add Edit to All Tracks” a feliratot is vágja | `docs/captions.md` |
| Qt6-felület | Text panel ▸ Captions fül (lista, keresés, időkód-szerkesztés, stílussáv), Captions munkaterület | `docs/qt-parity.md` 2.2, 3.23 (✅) |
| Megjelenítés | Program monitor, beégetés exportkor (`burnCaptions`), oldalfájl exportkor | `crates/captions/src/burn.rs` |
| Beszédfelismerés | Whisper candle-lel, CPU-n, szóidők, nyelvfelismerés, beszélő-szétválasztás; az átirat **médiaidőben** tárolva; `transcript.createCaptions` | `crates/speech`, `docs/transcripts.md` |
| Narráció | `tts.*` parancsok, Formant és Kokoro-82M hangok, narrációs klip az idővonalon | `crates/tts`, `crates/engine/src/narration.rs` |

### 3.2. Az 1.1 céljaihoz képest hiányzik

| Hiány | Az 1.1 helye | Megjegyzés |
|---|---|---|
| **ASS/SSA** olvasás, írás, stílus, réteg, megjelenítés | §4, §11 A-szint | Az A-szint legnagyobb hiánya |
| Kliphez kötött felirat (forrásidő-kapcsolat) | §7 | Ma a felirat sorozatidőben él, csak sync lock van; az átirat már médiaidős |
| Szó-/szótagidő a feliratban, bizonytalanság | §5 | Az átiratban megvan, a feliratba nem kerül át |
| Revíziók, változás-összehasonlítás | §4, §5 | Ma undo/redo és mentés/helyreállítás van |
| Fordítási és lektorálási állapot, terminológia, memória | §4, §9 | — |
| Formátumspecifikus kiegészítő adattárolás | §6 | Ma csak WebVTT-re (`settings`, `vtt_blocks`) |
| Veszteségjelentés más formátumba exportálás előtt | §6 | — |
| Konfigurálható QC-profil | §10 | A Media QC (#302, #305, #317 nyitott PR-ok) más célú; illeszthető |
| Hullámforma/spektrogram a feliratidőzítéshez | §4 | A Qt idővonal hullámformája megvan, felirat-specifikus nézet nincs |
| iTT, PGS, VobSub, OCR, Teletext, 608/708 videóba ágyazás | §11 B–D | A 608/708 beágyazás a roadmap M10-ben is nyitott |
| Önálló belépés NLE-projekt nélkül | 1.2 §3.1 | — |
| Fordítás, magyar TTS | §9, 1.2 §3.2 | A Kokoro-hangok ma csak amerikai angolok |

### 3.3. Következmény a tervre

1. Az 1.1 §5 „saját feliratdokumentumát” a **meglévő `cfa4-project` modell bővítése** valósítja
   meg, projektséma-verzió emeléssel és a régi projektek változatlan megnyitásával. Párhuzamos,
   második modell nem készül (ez az 1.2 „egyetlen CFA4-adatmodell” elvét is teljesíti).
2. Az 1.1 F2 „Rust szerkesztőmotorja” a meglévő `crates/engine` parancsrendszer és a
   `cfa4-captions` bővítése. Új háttérszerver nem kell.
3. Az 1.1 §11 B- és D-szintjének egy része (TTML/IMSC, EBU STL, SCC/MCC) már olvasható és
   írható; ezekre az F0 a támogatási mátrixot (AC-01) és a hiányzó profilellenőrzést rögzíti,
   nem újraírást.
4. A `CaptionFormat` enum ma egy mezőben keveri a fájl- és szolgáltatásfogalmat (`Subtitle`,
   `CEA-608`, `CEA-708`, `Teletext`). Az 1.1 §5 utolsó bekezdése szerint az ASS képi réteg, a
   nyelvi sáv és a CEA-szolgáltatás külön fogalom: az F1 ezt **visszafelé kompatibilisen**
   szétválasztja.

## 4. Termékszerkezet

### 4.1. Külön indítható alkalmazás és CFA4-munkaterület, közös motor (D1)

| Belépés | Mit nyit meg | Megvalósítás |
|---|---|---|
| **Feliratozás munkaterület** a videószerkesztőben | Az 1.1 három belépési pontja: projektfelirat, klipfelirat, külső feliratfájl | A meglévő Captions munkaterület és Text panel bővítése |
| **CFA4 Feliratozó** külön alkalmazásként | Helyi videó és/vagy feliratfájl, a teljes NLE-projekt betöltése nélkül | Saját menübejegyzés, ikon, ablakcím és fájltársítás (`.srt`, `.vtt`, `.ass`, `.ttml` …); a CFA4 csomagjában települ, és ugyanazt a Rust-motort, Qt6-komponenseket és beállításokat használja. A Feliratozóból a dokumentum egy lépésben átadható a CFA4 videószerkesztőnek, és vissza |

Mindkét belépés ugyanazokat a parancsokat, import/exportot, QC-t és fájlformátumot használja
(AC-14). Külön kódbázis vagy külön motor nem készül. Hogy a külön alkalmazás saját programfájl
vagy a `cfa4` program saját indítási módja legyen, az F0 felmérése és az F3 eldönti (csomagolás,
indítási idő, párhuzamos futás). Két menübejegyzés önmagában nem teljesíti a követelményt: a két
belépés közötti oda-vissza együttműködés a közös kiadási kapu része (AC-15).

A `VISION.md` 1. pontja ma „egyetlen, egységes” alkalmazást ír. A D1 döntés ezt a feliratozóra
pontosítja; a `VISION.md` szövegének frissítése külön PR (a tulajdonos döntése szerint).

### 4.2. Narráció

A feliratból készülő narráció a `VISION.md` **Szinkronizálás** területéhez tartozik, saját
felülettel és felelősséggel. A feliratozó nem épít saját TTS-motort: a meglévő `tts.*`
parancsokhoz és később az ADR 0006 M13 (beszédszintézis) motorcsaládjához csatlakozik a
6. fejezet szerződésével.

| Feliratozó felelőssége | Narrációs oldal felelőssége |
|---|---|
| Jóváhagyott szöveg, nyelv, cue-azonosító, időablak és időalap, caption-revízió átadása | Hangválasztás, jogosultság, szintézis, hangminőség, audioasset |

A narrációs klip ma „megtartja a hosszát” (tulajdonosi döntés, 2026-10-07). Ez összhangban van az
1.2 §4 szabályával: túl hosszú beszédnél a szöveg és az idővonal nem változik észrevétlenül, a
felhasználó választ (újragenerálás, engedélyezett nyújtás, szövegmódosítás, idővonal-módosítás).

### 4.3. Stáblista és címek

Külön tartomány marad (ADR F10, `crates/credits`). A beszédfelirat cue-modellje nem váltja ki a
stáblista adatmodelljét. A közösen használt rész a `crates/text` szövegmotor. Az élő adás (OBS)
tervezet a `VISION.md` 4. pontja szerint külön döntés; a feliratozóba nem öröklődik automatikusan.

## 5. Adatmodell-bővítés (F1)

Az 1.1 §5 felsorolása változatlanul a cél. A meglévő modellhez képest hozzáadandó:

| Elem | Tartalom |
|---|---|
| Sávszerep | Nyelv (BCP 47), szerep (normál, SDH, forced narrative, alternatív, kommentár), stabil azonosító, a forrássáv hivatkozása fordításnál |
| Fogalmi szétválasztás | Fájlformátum ≠ szolgáltatás (CEA-608/708 csatorna, Teletext-oldal) ≠ ASS-réteg |
| Időkapcsolat | `projektidőhöz rögzített` vagy `kliphez kötött` (klip azonosító + forrásidő), az 1.1 §7 választása szerint |
| Szóidő | Opcionális szó/szótag `Tick`-tartományok és bizonytalanság, az átiratból átvihetően |
| Stílus | Sávstílus mellett nevesített stílusok és cue-szintű felülírás (ASS-kompatibilis) |
| Eredeti mezők | Formátumspecifikus kiegészítő tár (ASS `[Script Info]`, ismeretlen sorok, TTML-attribútumok), az AC-02 adatmegőrzéshez |
| Szerkesztői állapot | Cue-nként: piszkozat, ellenőrzésre vár, javítandó, jóváhagyott; **külön** a zárolás (szerkesztésvédelem) és az elavultság jelzése |
| Revízió | Dokumentumrevízió-szám, cue-szintű változásnapló, az automatikus műveletek eredete (motor, modell, beállítás, forrásrevízió; 1.1 §9) |

A fordítás és az időzítés állapota külön mező, hogy egy egyszerű időeltolás ne tegye
fordítandóvá a változatlan szöveget (1.2 §4).

### 5.1. Egyidejű szerkesztés

Az önálló belépés és a videószerkesztő ugyanazt a dokumentumot szerkesztheti. Minden módosító
parancs tartalmazza a várt revíziót. Eltéréskor nincs csendes „utolsó író nyer”: különbséglista,
ellenőrzött újraalkalmazás vagy külön munkaváltozat következik; a történeti revíziók megmaradnak
(AC-16). Ez a minta megegyezik az ADR 0006 projektverzió-ellenőrzésével, ezért közös
megvalósítás javasolt.

## 6. Caption → Narration átadási szerződés (F1)

Az 1.2 §4 mezőcsoportjai a cél; a mezőnevek az F1-ben a meglévő `cfa4-project` és ADR 0006
jobsémához illesztendők.

| Mezőcsoport | Tartalom |
|---|---|
| Azonosítás | projekt, feliratdokumentum, caption-revízió, cue-azonosító, kérésazonosító |
| Bemenet | jóváhagyott szöveg, nyelv, beszélő, célidőablak és időalap |
| Hang | hangreferencia és jogosultsági hivatkozás, ha releváns |
| Végrehajtás | motor/modell (ADR 0006 modellkezelő), beállítások, megszakítás és ismétlés azonosítója |
| Kimenet | audioasset és revízió, tényleges hossz, cue-hoz rendelés, minőségi jelentés |
| Átvétel | elfogadva/elutasítva, hibakód, időzítési eltérés, veszteségjelentés |

Szövegváltozás után a korábbi narráció korábbi revízióként megmarad, de elavultként jelölt
(AC-19). Hiányzó motor vagy kiesés esetén nincs észrevétlen szolgáltatóváltás (AC-20).

A CFA3 átadási elve (forrásrevízió, nyugta, idempotens ismétlés) mérnöki referencia. A CFA4 nem
függhet a CFA3 SDK-tól, adatbázisától, Temporal-, HRB- vagy más platformelemétől.

## 7. AI, fordítás, OCR

Az 1.1 §9 szabályai változatlanok, a következő illesztéssel:

- **ASR:** a meglévő `cfa4-speech` (Whisper, candle, CPU) az alap; új motorok az ADR 0006 rétegen
  és az ADR 0004 útválasztón keresztül, gyártófüggetlen gyorsítással és CPU-s tartalékkal.
- **Modellkezelés:** az 1.1 „külön modulkezelője” az ADR 0006 modellkezelője; modell nem töltődik
  le a felhasználó tudta nélkül, licenc és forrás rögzítve (`VISION.md` 5.).
- **Szóillesztés:** javított átirat újraillesztése az átirat médiaidős szóidőire építve.
- **Fordítás:** alapból helyi modell. Külső szolgáltatás a D3 tulajdonosi döntés szerint: **csak kapcsolható, külön folyamatként**, alapból
  kikapcsolva, a felhasználó kifejezett bekapcsolásával; a helyi szerkesztés és fordítás hálózat
  nélkül is működik (9. fejezet, D3). Az ADR 0006 nyitott kérdése (a SeamlessM4T v2 nem kereskedelmi
  licence) ide is vonatkozik.
- **OCR:** PGS/VobSub a C-szinten; képre égetett felirat felismerése későbbi bővítés (1.1 §4).
- **Nyelvek:** a CFA4 többnyelvű alkalmazás. A magyar ASR, időzítés és OCR minőségét az AC-08
  szerint külön kell mérni, de egy nyelv (köztük a magyar) hangjának vagy modelljének megléte
  **nem kiadási feltétele** a feliratozónak vagy a narrációs körnek (D4). A narráció azokon a
  nyelveken érhető el, amelyekhez van telepített hang; a többinél a felület ezt jelzi.

## 8. Komponensek és licencszűrés

Az 1.1 §8 táblázata érvényes, a következő szűréssel (`docs/licensing.md`, `deny.toml`):

| Jelölt | Licencállapot | Szerep |
|---|---|---|
| Subtitle Edit | GPL-3.0 — kód átvehető fejléccel és nyilvántartási sorral | Funkcionális és kódreferencia (a `VISION.md` 4. pontjában a feliratozás mintaprogramja) |
| Aegisub | BSD (a forrásfájlok egyenként ellenőrizendők) | ASS-szerkesztési munkafolyamat referenciája |
| Subtitle Composer | GPL-2.0-or-later (egyenként ellenőrizendő) | A FA3-ban fő jelölt volt; CFA4-ben referencia és kompatibilitási cél, nem alap |
| libass | ISC | Elsődleges ASS-megjelenítési referencia (AC-06 orákulum); üzemi beépítése ADR-t igényel a „pure Rust” szabály miatt |
| Rust-crate-ek (subparse, ass-rs stb.) | Egyenként, `cargo deny check licenses` | Csak ha jobbak a meglévő `cfa4-captions` olvasóknál; a meglévők nem cserélendők ok nélkül |
| OxiMedia | Az 1.1 §8.1 szerint | Kísérleti; a #345 (metrikák) mintája szerint csak ellenőrzött részfunkció |
| Kdenlive | GPL | Munkafolyamat-referencia (`docs/kdenlive-parity.md`) |

Ismeretlen licencű projekt (például a Samaku, ha az ellenőrzés nem talál licencet) csak
funkcionális referencia lehet, kódja nem.

## 9. Tulajdonosi döntések (2026-10-30)

| # | Kérdés | Döntés | Előzmény |
|---|---|---|---|
| D1 | Önálló feliratozó? | **Külön alkalmazásként is indítható, és a CFA4-en belül is a megfelelő helyén van** (4.1) | Az 1.2 önálló Qt6-feliratozót javasolt; a `VISION.md` 1. pontja egyetlen alkalmazást ír, ezért a szöveg frissítése külön PR |
| D2 | Mikor induljon a fejlesztés? | **A Qt6-felület elkészülte után azonnal** (10. fejezet) | A `VISION.md` 6. pontja: a Qt6-felület az első prioritás |
| D3 | Külső fordítószolgáltatás? | **Csak kapcsolható, külön folyamatként** (7. fejezet) | A `VISION.md` 8. pontjában nyitott kérdés, a feliratozóra eldöntve |
| D4 | Függjön-e a kiadás egy nyelv (a magyar) hangjától? | **Nem:** a CFA4 többnyelvű, a magyar hang megléte nem mérvadó (7. fejezet, 12. fejezet) | Az 1.2 a narrációt a közös csomag feltételének tekintette |

## 10. Fejlesztési sorrend és kapuk

**Indítás (D2, tulajdonosi döntés 2026-10-30):** a feliratozó fejlesztése a Qt6-felület
elkészülte után **azonnal** indul, az F0-val. A Qt6-felület akkor számít elkészültnek, amikor az
ADR F8 2–7. lépése lezárult, és a `docs/qt-parity.md` szerint a Qt6-változat pontról pontra
tudja, amit az egui (`VISION.md` 2.1). Ez felülírja az 1.1 §12 azon mondatát, hogy az adatmodell
és a Rust-motor a Qt6-felület befejezése mellett fejleszthető. Ha a tulajdonos ennél korábbi
mérföldkövet (például csak az idővonal és az előnézet lépését) akar indítási pontnak, azt a
`VISION.md` 6. pontjában kell rögzíteni.

Az 1.1 F0–F8 fázisai és belépési feltételei ezen felül érvényesek, ezekkel a kiegészítésekkel:

| Fázis | Eredmény (1.1 + 1.2 + e változat) | Belépési feltétel |
|---|---|---|
| F0 — Audit | A 3. fejezet táblázatainak igazolása futtatással (`cargo test -p cfa4-captions`, Qt `test_text_panel`); támogatási mátrix a meglévő 7 formátumra; jelöltek licencauditja; CFA3-követelmények eredetmátrixa a MISSING_SOURCE jelöléssel; teljesítménycélok rögzítése | A Qt6-felület elkészülte (D2), utána azonnal |
| F1 — Modell és szerződések | Az 5. és 6. fejezet specifikációja, projektséma-emelés terve, revízió- és review-állapotok, egyidejű szerkesztés | F0 |
| F2 — Motor | Modellbővítés, ASS/SSA olvasó-író mutációs fuzz-tesztekkel, kiegészítő tár, veszteségjelentés, revízióellenőrzött parancsok | F1 |
| F3 — Qt6 felület | A Text panel és Captions munkaterület bővítése; a külön indítható Feliratozó (D1) | Stabil Qt6 shell és lejátszás (ADR F8 2–4. lépés) |
| F4 — NLE és QC | Kliphez kötött felirat, vágáskövetés, sebesség, oda-vissza út a két belépés között, QC-profil (a Media QC-vel közös riportséma, ha a #302 bekerül) | F2–F3 |
| F5 — AI és lokalizáció | ASR/szóillesztés/fordítás/OCR az ADR 0006 rétegen; narrációs átadás a meglévő hangokkal (D4) | ADR 0006 F-kapui, stabil modell |
| F6 — Typesetting | ASS-vizuális eszközök, karaoke, libass-orákulum (AC-06) | Igazolt ASS-megjelenítés |
| F7 — Export | 608/708 beágyazás, muxolt sáv, beégetés, exportprofilok | ADR 0005 médiahátterek kapui |
| F8 — Kiadás | A `VISION.md` 3. pontjának Ubuntu-változatain, Waylanden és X11-en igazolt működés | A kiadásba vállalt képességek |

Minden fázis külön PR-sorozat, egy PR egy téma (`docs/coordination.md`). Naptári határidő az F0
eredménye után adható.

## 11. Elfogadási feltételek

Az AC-01–AC-12 (1.1 §13) és az 1.2 AC-13–AC-22 feltételei érvényesek, a következő pontosítással:

| ID | Követelmény |
|---|---|
| AC-01 … AC-12 | Változatlanul az 1.1 szerint |
| AC-13 | A feliratozó **NLE-projekt nélkül** közvetlenül indítható; helyi média és felirat megnyitható |
| AC-14 | A két belépés azonos dokumentumon azonos szerkesztési szemantikát és exportot ad |
| AC-15 | Feliratozó → videószerkesztő → feliratozó út megőrzi a cue-azonosságot, stílust, időkapcsolatot és revíziót |
| AC-16 | Egyidejű szerkesztés nem okoz csendes felülírást vagy elveszett módosítást |
| AC-17 | A narrációs kérés rögzített caption-revízióra hivatkozik; a visszaadott audio ahhoz visszaköthető |
| AC-18 | A narráció időtúllépése látható; szöveg és idővonal nem módosul észrevétlenül |
| AC-19 | Szövegváltozás után az érintett fordítás/narráció elavultsága helyesen jelölt; a régi revízió megmarad |
| AC-20 | Hiányzó motor, kiesés és megszakítás esetén nincs észrevétlen szolgáltatóváltás vagy hibás részleges elfogadás |
| AC-21 | Az átadási csomag visszanyitható, a hivatkozott assetek és revíziók feloldhatók |
| AC-22 | A CFA3 → CFA4 követelményeredet tételesen nyomon követhető; hiányzó forrást nem pótol kitalált követelmény |
| **AC-23** (új) | A régi (mai sémájú) `.fcproj` feliratsávjai a bővített modellben változatlan tartalommal nyílnak meg és menthetők vissza |
| **AC-24** (új) | Minden új olvasó (ASS, iTT, PGS, VobSub) mutációs fuzz-tesztje `catch_unwind` alatt összeomlás nélkül fut (`AGENTS.md` §0.8) |
| **AC-25** (új) | Minden feliratozó művelet parancs, és a CLI-ről, a vezérlőcsatornáról és MCP-ről is elérhető, automatizálási azonosítókkal a Qt-felületen |

## 12. Kiadási fokozatok

1. **Szerkesztői alap:** kézi szerkesztés, SRT/VTT/**ASS**, helyes időkezelés, helyreállítás,
   a külön indítható Feliratozó és a videószerkesztő-integráció (AC-01–07, 13–16, 23–25).
2. **Professzionális feliratozó:** többnyelvű munka, QC-profilok, fordítás/lektorálás, OCR,
   szóillesztés, fejlett ASS, és a narrációs kör azokon a nyelveken, amelyekhez van hang (D4:
   egyetlen nyelv hangja sem kiadási feltétel). A Kdenlive-nél jobb megoldást itt összehasonlító méréssel kell
   igazolni (1.1 §14).
3. **Broadcast és speciális bővítések:** 608/708 beágyazás, szabványprofilok és a ritkább
   formátumok.

## 13. Az 1.1-hez és az 1.2 javaslathoz képest

| Téma | Döntés ebben a változatban |
|---|---|
| Funkcionális cél, F0–F8, AC-01–12 | Az 1.1 szerint marad |
| Önálló feliratozó | Az 1.2-ből átvéve: külön indítható alkalmazás és CFA4-munkaterület közös motorral (D1) |
| Narráció | Az 1.2-ből átvéve; a meglévő `tts.*`-re és az ADR 0006-ra illesztve; a 2. kiadási fokozatban, nyelvi feltétel nélkül (D4) |
| Adatmodell | Új modell helyett a meglévő `cfa4-project` bővítése (3.3) |
| Szerkesztőmotor | A meglévő `crates/engine` parancsrendszer és `cfa4-captions` |
| Formátumsorrend | Az 1.1 szerint, de a már meglévő B/D-szintű formátumok auditálva, nem újraírva; az ASS az első új formátum |
| Modulkezelő, GPU | Az ADR 0006 modellkezelője és az ADR 0004 gyártófüggetlen útválasztója |
| Review-állapotok, revízióütközés, elavulás | Az 1.2-ből átvéve |
| CFA3 platform | Referencia, nem futásidejű függőség (1.2 szerint) |
| Indítás | A Qt6-felület elkészülte után azonnal (D2); az 1.1 párhuzamos fejlesztési mondata helyett |
| Külső fordítás | Csak kapcsolható, külön folyamat (D3) |
| Új elfogadási feltételek | AC-23–25 |

## 14. Átadási állapot

**Elkészült:** az 1.1 és az 1.2 javaslat egyesítése, összevetve a CFA4 `main` dokumentációjával
és forrásával, valamint a `VISION.md`-vel; a D1–D4 tulajdonosi döntések (2026-10-30) beépítése.

**Nem igazolt e dokumentummal:** a 3. fejezet állításai dokumentáció- és forrásolvasáson
alapulnak; build vagy teszt nem futott. Implementáció nem indult.

**Jóváhagyás:** a tulajdonos 2026-10-30-án jóváhagyta a tervet.

**Következő lépés:** a terv beolvasztása a `main`-be (a gazda feladata); a `VISION.md` pontosítása
a D1 szerint külön PR-ban (#362); az F0 a Qt6-felület elkészülte után azonnal indul (D2).
