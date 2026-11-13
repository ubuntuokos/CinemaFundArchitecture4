# CFA4-TITLE-CREDITS-001 1.0 – CFA4-illesztési melléklet

**Dátum:** 2026-11-14 · **Állapot:** a terv javasolt, jóváhagyásra vár; a lenti kérdések a
tulajdonos döntéséig nyitottak · **Terv:** [`CFA4-TITLE-CREDITS-001-1.0.md`](CFA4-TITLE-CREDITS-001-1.0.md) ·
**Meglévő terv:** [`docs/credits-plan.md`](../credits-plan.md) (F10, C1–C14), [ADR F10](../adr/ADR-F10-CREDITS-SPREADSHEET-2026-10-23.md),
[`docs/credits.md`](../credits.md)

Ez a melléklet **nem módosítja** a tervet, és nem dönt helyette. Azt rögzíti, hogy a terv egyes
pontjai mennyire vannak meg ma a CFA4 `main` ágán, hol tér el a terv a `main`-ben rögzített
szabályoktól és tervektől (`VISION.md` fejléce szerint az ellentmondást jelezni kell, nem csendben
feloldani), és milyen döntések kellenek. Ha a tulajdonos egy pontban dönt, a döntés a 3. pontba
kerül, és szükség esetén a terv új verziót kap (1.1).

Állapotjelek: **kész** = a `main`-en, tesztekkel; **részben** = a képesség egy része megvan;
**nincs** = nincs a CFA4-ben; **PR** = nyitott PR-ban, a tulajdonos összevonására vár.

## 1. Megfeleltetés: a terv pontjai és a CFA4 mai állapota

### 1.1. A stáblista-terv fázisai (C1–C14) a terv pontjaiban

| Fázis | Állapot | A terv pontja |
|---|---|---|
| C1 kapcsolt stáblista, `.cfa4credits`, soronkénti eltérés és frissítés | kész | 3.1 (szűkebb, lásd E4), 4.1 (táblázatra), 7.1 (részben) |
| C2 ellenőrzések (betűtípus, hiányzó karakter, címbiztos terület, kontraszt, gördülés) | kész | 6 (projektvalidáció), 13.6 |
| C3 igazítás a zenéhez (ütemhatár, hang vége, kártya ütemre) | kész | 2.1 E (zenei anyaghoz igazított hossz) |
| C4 háttér, átmenet, futószöveg, segédvonalak | kész | 2.1 F (részben), 2.1 D (háttér) |
| C5 gazdagabb elrendezés, portré, videó a stáblistában, mozgás-előbeállítások | kész | 2.1 D (kép, videó), 2.1 C (részben) |
| C6 stáblista-sablonok (`.cfa4credtpl`) | kész | 3.2 (részben, lásd E9) |
| C7 Cinecred-import | kész | 4.2 (Cinecred-kompatibilis adatforrás) |
| C8 Qt6 stáblista-munkaterület | kész | 6 (részben), 7 |
| C9 csomagolás és együttélés (saját MIME, Cinecreddel együtt próbálva) | kész | 14, 12 F9 (részben) |
| C10 élő grafika (cue-k, PREVIEW/TAKE/NEXT/HOLD/UPDATE/OUT, Browser Source, obs-websocket) | kész | 8, 8.1, 8.2 |
| C11 ismétlődő videóháttér (vonalrajz, 1. lépcső) | kész | 2.1 D (háttér, animált objektum) |
| C12 vonalrajz-réteg Rustban | tervezet (jövő évi kiadás) | 2.1 C (2D grafikai animáció) |
| C13 ismétlődő videó cellában (`{{Video walk.mp4 loop}}`) | PR #1188 | 2.1 D |
| C14 horgas gerincek (`Hook N From-To dx dy`) | PR #1196 | 2.1 D (elrendezés) |

### 1.2. A terv komponensei (2.1)

| Pont | Állapot | Mi van ma | Mi hiányzik |
|---|---|---|---|
| A. Credits Core | részben | `crates/credits` (adat, elrendezés, ellenőrzés) és az `engine` `graphics.credits.*` parancsai; minden felület ugyanazokat hívja (credits-plan 3.1) | jelenet- és rétegmodell a stáblistán túl (címek, alsó sávok közös projektje), lásd E4–E5 |
| B. Typography Engine | részben | `crates/text`: betűtípus, méret, vastagság, betűköz, sortáv, tördelés (pont- és bekezdésszöveg), karakterenkénti stílus, árnyék (elmosással), két kontúr, háttérdoboz, színátmenet, hiányzó karakter jelzése, olvashatósági ellenőrzés; Text Animator: *Typewriter*, *Word by Word* ([`graphics.md`](../graphics.md)) | körív menti szöveg, perspektivikus szöveg, ragyogás, maszkolás szöveggel (*Not yet* a `graphics.md`-ben), kulcsképkockázható betűnkénti animáció |
| C. Motion Graphics Engine | részben | kulcsképkocka minden animálható tulajdonságon, easing-előbeállítások, Motion (pozíció, méret, forgatás) és Opacity; 2D „kamera” a Motion nagyításával és mozgatásával | valódi kamera (3D), rétegenkénti keverési módok, objektumcsoport közös animációja, maszkos átmenet |
| D. Credits Composer | részben | a CFA4 idővonala: grafikus klipek, kép, videó, hang, effekt, átmenet külön sávon, saját időzítéssel; stáblistában kép és videó (C5) | médiaréteg a grafikán belül (*Not yet* a `graphics.md`-ben); külön jelenetszerkesztő nincs (E5) |
| E. Audio Synchronization | részben | zenéhez igazítás ütemfelismeréssel (C3), hullámforma és több hangsáv az idővonalon, helyi beszédfelismerés szóidőkkel (`crates/speech`), narráció (`tts.*`) | narrációvezérelt animáció (5.1), szavankénti kiemelés (E12) |
| F. Transition Engine | részben | a meglévő átmenet-rendszer (áttűnés, eltolás, csúsztatás, törlés, áttűnés feketén át és bármely átmenet azonosítója), az idővonalon külön szerkeszthető objektumként; oldalak közt (C4) | fény-, maszkos, kameramozgásos és tipográfiai átmenet mint külön típus |

### 1.3. A terv többi pontja

| Pont | Állapot | Mi van ma | Megjegyzés |
|---|---|---|---|
| 3.1 projektformátum | részben | `.cfa4credits`: verziózott JSON, forrás, stílus, beágyazott sorok, illesztés (C1, C3) | a terv teljes jelenetprojektet kér: E4 |
| 3.2 Template Manager | részben | `.cfa4credtpl` (C6) és a grafikák `.fcgt` sablonjai (alsó sávok, címek, záró stáblista) kategóriával | a terv 10 tartalmi kategóriát és előnézeti képet kér: E9 |
| 4 Story-híd | nincs | – | halasztva (credits-plan 7.): E6 |
| 4.1 adatszinkron | kész (táblázatra) | C1: eltérés soronként, előnézet, elfogadás, elutasítás, kézi módosítás megtartása, stabil rétegkulcsok, a forrás megőrzése | ugyanez a minta használható a Story-hídhoz |
| 4.2 adatforrások | részben | CSV, TSV, XLSX, ODS, `.cfa4credits`, Cinecred-projekt | külső produkciós adatbázis: E7; online táblázat: a `cfa4-fetch`-re vár |
| 5.2 feliratok | részben | SRT és VTT be/ki, a felirat külön, szerkeszthető objektum, beszélő (`<v Speaker>`), pozíció, beégetés külön lépésben ([`captions.md`](../captions.md)) | szavankénti kiemelés, karaoke: E12 |
| 6 önálló alkalmazás | részben | `cfa4-qt --credits FÁJL`: a teljes stáblista-munkaterület, rejtett asztali bejegyzéssel, fájltársítással (C9) | E3 |
| 7 integráció a szerkesztővel | kész | a munkaterület a szerkesztőben él (C8), elhelyezés, frissítés, sablon, illesztés a kijelölt stáblistán | – |
| 7.1 natív idővonal-objektum | részben | az elhelyezett oldalak a dokumentumhoz kötött grafikus klipek, a kézi módosítást a frissítés megtartja (C1); hossz a zenéhez igazítással (C3) | a szerkesztőben végzett módosítás nem íródik vissza a táblázatba / `.cfa4credits`-be; nem egy objektum, hanem oldalankénti klipcsoport: E10 |
| 8 OBS, élő grafika | kész (C10) | futtató, élő oldal 127.0.0.1-en, obs-websocket 5, Qt6 ablak; nincs teljes újrarenderelés | a terv „későbbi szakaszra” teszi (15.), a CFA4-ben már kész; a folytatás (műsorterv, OBS-képességek) a [CFA4-LIVE-STUDIO-001](CFA4-LIVE-STUDIO-001.md) |
| 9–10 donorok | nincs | a repóban egyik donorról sincs vizsgálat | E11 |
| 11 rendszerkövetelmények | részben | Ubuntu 26.04 / amd64 (referenciagép), Qt6, Rust-mag, FFmpeg, gyártófüggetlen GPU, CPU-n is | PostgreSQL: E7; Python: E8; AI-modellek letöltése: E13 |
| 12 fázisok | – | – | elnevezés-ütközés a CFA4 F-azonosítóival: E14 |
| 13 minőségbiztosítás | nagyrészt kész | 2: minden oldal külön grafikus klip; 4: egész `Tick`-alapú, képkockára pontos idő; 5–6: C1 és C2; 7: automatikus mentés projektverziókkal (`autoSave.maxVersions`); 8: az élő munkamenet nem kerül a projektbe (C10); 9: CPU-renderelés; 10: modell csak a felhasználó jóváhagyásával töltődik le | 1 (export) és 3 (narráció–felirat időzítés ellenőrzése) külön teszt nélkül |
| 14 együttélés | kész | C9 (saját MIME, saját beállításmappa, Cinecreddel együtt próbálva) | a donorokra (E11) ugyanígy kell majd |
| 15 első kiadás | nagyrészt kész | cím (grafikák és sablonok), nyitó/záró képsor, hagyományos és görgetett stáblista, alsó sávok, egyszerű átmenetek, kép és videó, külön hangsáv, mentés és újraszerkesztés, szerkesztő-integráció | hiányzik: narrációhoz igazított szöveg; részben: animált tipográfia, sablonkezelő |

## 2. Eltérések

| # | Téma | A terv | A CFA4 mai állapota | Javaslat |
|---|---|---|---|---|
| E1 | A Cinecred szerepe (4.2) | „kizárólag adatforrási kompatibilitási referencia, nem beépítendő alkalmazás” | Az [ADR F10](../adr/ADR-F10-CREDITS-SPREADSHEET-2026-10-23.md) „Cinecred-like capability”; a C13–C15 és az egységes törzsszélesség a Cinecred nyilvános útmutatójában leírt elrendezési képességek saját megvalósítása | A mondat a Cinecred *alkalmazás* beépítését zárja ki; a saját, útmutatóból leírt elrendezési képességek (C13–C15) maradnak. Döntés: Q1 |
| E2 | Tisztaszoba és a stáblista-terv licencpontja | (a terv csak licencellenőrzést ír, F0) | A `credits-plan.md` 3.5 szerint a Cinecred kódja „felhasználható”; a [`licensing.md`](../licensing.md) §5 szerint a C7 a formátumot „a dokumentációjából és publikált forrásából” írta le (kód nem került át). A stáblista-munka mai szabálya: csak a nyilvános útmutató (cinecred.com/guide), a forráskód soha | A `credits-plan.md` 3.5 kapjon megjegyzést a mai szabályról; a `licensing.md` sora történeti nyilvántartás, marad. Döntés: Q2 |
| E3 | Önálló alkalmazás (1., 6., F5) | két önálló megjelenési forma, teljes funkcionalitással | `credits-plan.md` 8.1 (2026-10-25): „csak Qt6, a CFA4-en belül”; utána a `VISION.md` 2.7 (2026-10-30): külön is indítható részek, egy motor, egy csomag; a C9 óta `cfa4-qt --credits FÁJL` a teljes munkaterületet nyitja | A `VISION.md` 2.7 mintája: külön indítási mód és menübejegyzés, ugyanaz a motor és parancsok; külön kódbázis nem. Döntés: Q3 |
| E4 | Projektformátum (3.1, F1) | `.cfa4credits` teljes jelenetprojekt (jelenetek, rétegek, animációk, hang, effektek) | A `.cfa4credits` szándékosan szűk: forrás, stílus, sorok, illesztés (credits-plan 3.4, 7. utolsó sor); a jelenetek, rétegek és animációk a `.fcproj` grafikus klipjeiben élnek | A `.cfa4credits` maradjon a stáblista adatforrása; jelenet-szintű tartalom csak akkor kerüljön bele, ha az önálló indítás (E3) projekt nélkül is igényli. Döntés: Q4 |
| E5 | Külön motorok (2.1 B–F) | Typography, Motion Graphics, Composer, Audio Sync és Transition Engine mint komponensek | Egy közös mag (credits-plan 3.1): a szöveg a `crates/text`, az animáció a grafikus klipek kulcsképkockái, a kompozíció az idővonal, az átmenet a meglévő átmenet-rendszer; „nincs külön stáblista-motor” | A terv komponensei a meglévő CFA4-elemekre képeződjenek le (1.2. táblázat), párhuzamos motor nélkül. Döntés: Q4 |
| E6 | Story-alkalmazás (4., F7) | a filmadatok elsődleges forrása | A CFA4-ben nincs Story-alkalmazás; a híd halasztva (credits-plan 7.); a 4.1 szinkronszabályai a táblázatra a C1-ben már teljesülnek | A híd maradjon halasztva, amíg nincs Story-adatforrás; akkor a C1 eltérés/frissítés mintáját használja. Döntés: Q5 |
| E7 | PostgreSQL (11., 4.2 külső adatbázis) | a CFA4 központi adatkezeléséhez igazítva | Az [ADR 0006](../adr/ADR-0006-backlothub-postgresql.md) a BacklotHub produkciókezelő adattára; a stáblista fájlokkal dolgozik, adatbázis nélkül | A stáblista ne igényeljen adatbázist; a „külső produkciós adatbázis” forrása a BacklotHub legyen, amikor a stáb- és szereplőadat ott megjelenik. Döntés: Q5 |
| E8 | Python helye (11., 10.) | „Python csak indokolt, izolált segédfolyamatokban”; Blender és Python nem kerülhet a Qt6 főfolyamatba | A Qt6-felület maga PySide6 (Python) a főfolyamatban ([`qt-gui.md`](../qt-gui.md)); az üzleti logika a Rust-motorban fut (`cfa4-cli serve`); `requires-python >= 3.12`, a CI 3.13-at futtat | A szöveg pontosítása: „üzleti logika Rustban; Python a Qt6-felületben (PySide6) és izolált segédfolyamatokban; Blender soha nem a felület folyamatában”. A Python 3.14-et a CI-mátrixba érdemes felvenni. Döntés: Q6 |
| E9 | Sablonkategóriák (3.2) | 10 tartalmi kategória, előnézeti kép | `.cfa4credtpl`: beépített / felhasználói / projekt; a `.fcgt` saját `category` mezővel | A forrás szerinti hármas maradjon, mellé a terv 10 tartalmi kategóriája és előnézeti kép a sablonválasztóban. Döntés: Q7 |
| E10 | Visszaírás a kreditprojektbe (7.1) | a szerkesztőben végzett módosítás visszakerül az eredeti kreditprojektbe | C1: a kézi módosítást a frissítés megtartja, de a táblázatba vagy a `.cfa4credits`-be nem ír vissza; a CFA4-ben a táblázat a forrás | A táblázat maradjon az igazság forrása; a kézi módosítás a stáblista-dokumentumban tárolódjon (mentéskor a `.cfa4credits`-be), a táblázatba visszaírás csak kérésre. Döntés: Q8 |
| E11 | Donorok (9., 10.) | Cinematic Skill, sand3d, Papercut Explainer vizsgálata; Skia-renderelés vizsgálata | A repóban nincs róluk vizsgálat; a licencük nincs nyilvántartva; a CFA4 saját renderelőt használ, Skia nem függőség; a 3D mélysége a `VISION.md` 8. pontjában nyitott | Az F0 donorvizsgálata külön dokumentum: első a licenc (GPL-3.0-kompatibilitás, [`licensing.md`](../licensing.md)), kódátvétel csak tulajdonosi döntés után, az [`AGENTS.md`](../../AGENTS.md) §2 szerint. Döntés: Q9 |
| E12 | Narrációvezérelt animáció és szavankénti kiemelés (5.1–5.2) | a generátor része | A feliratozó tervének része ([SUBTITLE-STUDIO 1.2](SUBTITLE-STUDIO-PLAN-001-FINAL-1.2.md) F6 karaoke, 6. caption → narráció szerződés); a [`gaps.md`](../gaps.md) „Word Highlight captions” jelöltje; a szóidőket a `crates/speech` adja | Egy helyen készüljön (feliratozó + szóidők), a cím- és kreditgenerátor azt használja. Döntés: Q10 |
| E13 | AI-modellek telepítése (11., 13.10) | a CFA4 csak letöltési hivatkozást, ajánlott helyet és tárhelyigényt adhat | A beszédfelismerés modelljeit és a Kokoro-hangcsomagot (336 MB) a CFA4 tölti le, de csak a felhasználó jóváhagyása után (`transcript.downloadModel`, `tts.downloadVoices`) | Ha a terv a jóváhagyott, alkalmazáson belüli letöltést is tiltja, az a meglévő funkciókat érinti; javaslat: „automatikusan nem, a felhasználó jóváhagyásával igen”. Döntés: Q11 |
| E14 | Fázisnevek (12.) | F0–F9 | A CFA4-ben az F-azonosítók foglaltak (ADR F8 Qt6, ADR F10 stáblista; a feliratozó F0–F6 fázisai) | A terv fázisai kapjanak saját előtagot (pl. TC-F0…TC-F9), a megvalósítás pedig folytassa a C-számozást (C15-től). Döntés: Q12 |

## 3. Kérdések és döntések

| # | Kérdés | Javaslat | Döntés |
|---|---|---|---|
| Q1 | A Cinecred „kizárólag adatforrás-referencia” mondata megállítja-e a Cinecred-szerű elrendezési munkát (C13–C15, egységes törzsszélesség) (E1)? | Nem: az import marad kompatibilitási referencia, az útmutatóból leírt elrendezési képességek a CFA4 saját funkciói | nyitott |
| Q2 | A `credits-plan.md` 3.5 kapjon-e megjegyzést a tisztaszobás szabályról (E2)? | Igen; a `licensing.md` sora marad | nyitott |
| Q3 | Az önálló alkalmazás a `VISION.md` 2.7 szerinti külön indítási mód legyen-e (E3)? | Igen: `cfa4-qt --credits` bővítése saját menübejegyzéssel és ikonnal, egy motorral | nyitott |
| Q4 | A terv komponensei és projektformátuma a meglévő CFA4-elemekre képeződjenek-e le (E4–E5)? | Igen: nincs párhuzamos motor; a `.cfa4credits` adatforrás marad, jelenet-tartalom csak az önálló indításhoz, ha kell | nyitott |
| Q5 | Story-híd és külső adatbázis: halasztás, majd BacklotHub (E6–E7)? | Igen: halasztva; adatbázis a stáblistához nem kell | nyitott |
| Q6 | Pontosítsuk-e a Python helyét a tervben, és kerüljön-e a Python 3.14 a CI-be (E8)? | Igen, mindkettő | nyitott |
| Q7 | Bővüljenek-e a sablonok a 10 tartalmi kategóriával és előnézeti képpel (E9)? | Igen, a forrás szerinti hármas mellett | nyitott |
| Q8 | Hová kerüljenek a szerkesztőben végzett kézi módosítások (E10)? | A stáblista-dokumentumba / `.cfa4credits`-be; a táblázatba csak kérésre | nyitott |
| Q9 | Induljon-e az F0 donorvizsgálat, és milyen sorrendben (E11)? | Igen, külön dokumentumban: licenc első, kódátvétel csak döntés után | nyitott |
| Q10 | A narrációvezérelt animáció és a szavankénti kiemelés a feliratozó tervében készüljön-e (E12)? | Igen, a cím- és kreditgenerátor használja | nyitott |
| Q11 | Megengedett-e a jóváhagyással indított, alkalmazáson belüli modell-letöltés (E13)? | Igen (a mai működés); automatikus telepítés nem | nyitott |
| Q12 | Milyen azonosítót kapjanak a terv fázisai (E14)? | TC-F0…TC-F9; a megvalósítás C15-től | nyitott |
