# CFA4-TITLE-CREDITS-001 1.0 – CFA4-illesztési melléklet

**Dátum:** 2026-11-14 · **Állapot:** a terv javasolt; a tulajdonos 2026-11-14-én elfogadta az E1–E14 javaslatait (Q1–Q12)
és döntött az 1.2 hiányainak pótlásáról; a 3. pont javaslatai (Q1–Q13) mind elfogadva · **Terv:** [`CFA4-TITLE-CREDITS-001-1.0.md`](CFA4-TITLE-CREDITS-001-1.0.md) ·
**Meglévő terv:** [`docs/credits-plan.md`](../credits-plan.md) (F10, C1–C14), [ADR F10](../adr/ADR-F10-CREDITS-SPREADSHEET-2026-10-23.md),
[`docs/credits.md`](../credits.md)

Ez a melléklet **nem módosítja** a tervet, és nem dönt helyette. A döntések a 3. pontban vannak; a
szövegbe a terv következő verziója (1.1) vezeti át őket. Azt rögzíti, hogy a terv egyes
pontjai mennyire vannak meg ma a CFA4 `main` ágán, hol tér el a terv a `main`-ben rögzített
szabályoktól és tervektől (`VISION.md` fejléce szerint az ellentmondást jelezni kell, nem csendben
feloldani), és milyen döntések kellenek. Ha a tulajdonos egy pontban dönt, a döntés a 3. pontba
kerül, és szükség esetén a terv új verziót kap (1.1).

Állapotjelek: **kész** = a `main`-en, tesztekkel; **részben** = a képesség egy része megvan;
**nincs** = nincs a CFA4-ben; **PR** = nyitott PR-ban, a tulajdonos összevonására vár.

## 0. Illeszkedés a meglévő stáblista-generátorhoz (összefoglaló)

**Röviden:** a CFA4-TITLE-CREDITS-001 nem új rendszer a stáblista-generátor mellett, hanem annak
**folytatása és kiszélesítése**. A már majdnem kész stáblista-generátor (F10, C1–C15) a terv
„Credits Core”-ja: ugyanaz a mag, ugyanaz a fájl, ugyanazok a parancsok, ugyanaz a Qt6
munkaterület. A terv új részei erre épülnek rá, új C-fázisokként (javaslat, 0.3). A C1–C15
semmilyen része nem íródik újra, és a futó munka (C13, C14, C15, egységes törzsszélesség) változás
nélkül folytatódik.

### 0.1. Ami ma a stáblista-generátor, az a terv melyik része

```text
 Adatbevitel                 Credits Core (a mag)                    Kimenet
 ───────────                 ─────────────────────                   ───────
 táblázat (CSV/XLSX/ODS) ─┐  crates/credits: blokkok, stílusok,      grafikus klipek az idővonalon
 Cinecred-projekt (C7) ───┼─> oldalak, elrendezés, ellenőrzés (C2) ─> (Qt6 munkaterület, C8)
 .cfa4credits (C1) ───────┤  engine: graphics.credits.* parancsok     ├─> export (normál render)
 [később: Story,          │  .cfa4credits: forrás + stílus + sorok    └─> élő grafika (C10, crates/live)
  BacklotHub] ────────────┘  + illesztés (C3) + sablon (C6)
                                     │
                 a közös CFA4-elemekre épül:
                 crates/text (tipográfia) · grafikus rétegek és kulcsképkockák (mozgás)
                 · idővonal (kompozíció, hang) · átmenet-rendszer (C4) · .fcgt sablonok
```

| A terv része | A stáblista-generátorban ma | Ami a tervből hozzájön |
|---|---|---|
| Credits Core (2.1 A, 3.1) | `crates/credits` + `graphics.credits.*`, `.cfa4credits` (C1) | a dokumentum ne csak stáblista-oldalakat, hanem címet, nyitó képsort, fejezetcímet, alsó sávot is tartalmazhasson (C16) |
| Typography Engine (2.1 B) | a stílusok a `crates/text`-et használják (C5 betűstílus soron belül, C2 betű- és karakterellenőrzés) | körív, perspektíva, ragyogás, betűnkénti animáció a grafikákban, így a stílusokban is (C18) |
| Motion Graphics (2.1 C) | mozgás-előbeállítások kártyára (C5), gördülés, futószöveg (C4), minden oldal kulcsképkockázható klip | kamera- és csoportanimáció mint stílus-beállítás (C19) |
| Credits Composer (2.1 D) | háttér (C4), portré, kép, videó (C5), ismétlődő videó (C11, C13), gerincek (C14–C15) | médiaréteg a grafikán belül; ez a Composer hiányzó része (C19) |
| Audio Sync (2.1 E) | igazítás a zenéhez, ütemre (C3) | igazítás narrációhoz, név megjelenése a kimondásakor (C21) |
| Transition Engine (2.1 F) | átmenet az oldalak közt (C4) | új átmenettípusok a közös átmenet-rendszerben (C20) |
| Template Manager (3.2) | `.cfa4credtpl`, négy beépített sablon (C6) | 10 tartalmi kategória, előnézeti kép, a `.fcgt` grafikasablonokkal egy böngészőben (C17) |
| Story-híd (4) | táblázat-kapcsolat, eltérés, részleges elfogadás (C1) | ugyanez a mechanizmus más forrásra (C24, halasztva) |
| Önálló alkalmazás (6) | `cfa4-qt --credits FÁJL` (C9) | saját menübejegyzés és ikon, projekt nélküli mentés (C23) |
| Szerkesztő-integráció (7) | a munkaterület a szerkesztőben (C8), frissítés a kézi módosítások megtartásával (C1) | a kézi módosítás visszaírása a dokumentumba (C22) |
| OBS, élő (8) | C10 kész | a folytatás a [CFA4-LIVE-STUDIO-001](CFA4-LIVE-STUDIO-001.md) |

### 0.2. Ami nem változik

- **Egy mag, egy fájl:** a `crates/credits` és a `.cfa4credits` marad; a terv `.cfa4credits`
  kiterjesztése ugyanaz, mint a C1 fájljáé, ezért új formátum nem kell, csak a meglévő bővül
  (új verzió, a régi fájlok megnyílnak).
- **A táblázat marad az elsődleges adatbevitel**, a Cinecred-import (C7) a kompatibilitási
  referencia (E1).
- **Minden eredmény szerkeszthető grafikus klip** az idővonalon, egy visszavonási lépéssel; a
  „Composer”, a „Motion” és a „Transition Engine” a CFA4 meglévő idővonala, grafikái és átmenetei,
  nem külön motorok (E5).
- **Csak Qt6 felület** (a tulajdonos döntése, 2026-11-14, Q14; összhangban a `credits-plan.md` 3.2-vel):
  a C16–C24 minden felülete a Qt6 GUI-ban (`python/cfa4_qt/`) készül; egui-felület nem készül.
- **Minden támogatott nyelven** (a tulajdonos döntése, 2026-11-14, Q15): az alkalmazás és a C16–C24
  minden új felületi szövege a CFA4 összes nyelvén elérhető: az angol forrás mellett az `i18n/` minden
  katalógusában (ma 34 `.tsv`); az engine új figyelmeztetései a `credits_messages.py` NOTES-ába is
  bekerülnek. Fordítás nélküli szöveg nem kerül a `main`-be.
- **A Qt6 Stáblista-munkaterület (C8)** lesz a terv „munkafelülete”; a tervben felsorolt panelek
  (réteglista, idővonal, kulcsképkocka-szerkesztő, előnézet, validáció) a CFA4 Qt6 felületének
  meglévő paneljei.

### 0.3. Folytatás C-fázisokként (elfogadva: Q13, tulajdonos, 2026-11-14)

| Fázis | Tartalom | A terv pontja | Függ |
|---|---|---|---|
| C13–C15, törzsszélesség | folyamatban (#1188, #1196, C15) | 2.1 D | – |
| C16 Címjelenetek a stáblista-dokumentumban | `Title`, `Opening`, `Chapter`, `LowerThird` oldaltípus: a táblázat egy sora egy `.fcgt` grafikát tölt ki (cím, alcím, név, szerep); főcím, nyitó képsor, fejezetcím, szereplőbemutató egy dokumentumban a stáblistával | 1.1, 15. | – |
| C17 Sablonkezelő | a terv 10 kategóriája, előnézeti kép, `.cfa4credtpl` és `.fcgt` egy böngészőben, import/export; **az elkészült stáblista sablonként menthető** (Q16): a C6 *Mentés sablonként…* (`graphics.credits.template.save`, `clip`) kiterjesztése a C16–C22 minden elemére (oldaltípusok, hátterek és vonalrajz, átmenetek, mozgás, narrációs igazítás, a kézi módosításokból a stílus), a konkrét nevek helyett mintatáblázattal; **filmműfajonként legalább egy beépített sablon** (Q17), mindegyik szerkeszthető és új néven menthető (felhasználói vagy projekt-sablonként; a beépített változatlan marad) | 3.2 | C16 |
| C18 Tipográfia | körív menti szöveg, perspektivikus szöveg, ragyogás és fényhatás, maszkolás szöveggel és szövegfelfedés, kulcsképkockázható betűnkénti és szavankénti animáció (a mai Typewriter / Word by Word előbeállítások mellett); a stílusok (`ContentStyle`, `LetterStyle`) is elérik | 2.1 B | – (a grafikai rendszerben) |
| C19 Mozgás és kompozíció | kamera: 2.5D réteg-tér (mélység, perspektivikus kamera, kamerapálya kulcsképkockákkal; a teljes 3D jelenet a `VISION.md` 8. pontjának nyitott döntése); rétegenkénti keverési módok; objektumcsoport közös animációja és több objektum szinkronja; médiaréteg (kép, videó) a grafikán belül | 2.1 C, D | – |
| C20 Átmenetek | fény-, elmosásos, maszkos, kameramozgásos és tipográfiai átmenet a közös átmenet-rendszerben, külön szerkeszthető objektumként; oldalak közt a stílusból is | 2.1 F | C18, C19 |
| C21 Narráció és animált feliratok | a C3 kiterjesztése: stáblista hossza narrációhoz; név / kártya / kép megjelenése a szóidőkhöz kötve (5.1); szavanként kiemelt és karaoke-felirat, szereplőnkénti szín (5.2) | 2.1 E, 5.1, 5.2 | szóidők (`crates/speech`); a feliratozóval közös megvalósítás (E12, Q10) |
| C22 Kétirányú kapcsolat | a szerkesztőben végzett kézi módosítás a stáblista-dokumentumba íródik; az elhelyezett stáblista egy objektumként kezelhető (csoport) | 7.1 | – |
| C23 Önálló indítás | menübejegyzés, ikon, projekt nélküli munka, átadás a szerkesztőnek és vissza | 1., 6. | C16, C22 |
| C24 Story / BacklotHub adatforrás | a C1 eltérés/frissítés más forrásból | 4. | Story- vagy BacklotHub-adat (halasztva) |
| C12 Vonalrajz-réteg | [CFA4-LINEART-001](CFA4-LINEART-001.md), a jövő évi kiadás, változatlanul | 2.1 C, D | – (a C16 oldaltípusai is használják) |

A sorrend azt követi, hogy a terv első kiadási köréből (15.) mi hiányzik a legjobban: a címek és
alsó sávok a stáblistával egy helyen (C16), a sablonkezelő (C17), az animált tipográfia (C18) és a
narrációhoz igazított szöveg (C21). Az 1.2 hiányainak pótlása a tulajdonos döntése (2026-11-14):
C18 (tipográfia), C19 (mozgás és kompozíció), C20 (átmenetek), C21 (narráció és animált feliratok).

### 0.4. Hogyan épülnek össze: egy folyamat, a terv elemei a lépéseibe kötve

A terv elemei nem a stáblista-generátor mellé kerülnek, hanem **a meglévő feldolgozási lánc
lépéseibe**. Így minden új képesség automatikusan megkapja, amit a lánc már tud: kapcsolat a
forráshoz és frissítés (C1), ellenőrzés (C2), igazítás (C3), sablon (C6), Qt6 munkaterület (C8),
élő adás (C10).

```text
 1. Forrás          2. Beolvasás         3. Stílus            4. Elrendezés        5. Elhelyezés            6. Kimenet
 táblázat,          sheet.rs             style.rs             layout.rs            engine/credits.rs        idővonal → export
 Cinecred,     ──>  sorok → oldalak, ──> oldal-, tartalom-, ──> oldalak képkockái ──> grafikus klipek,     ──> engine/live.rs →
 .cfa4credits       blokkok              betűstílus                                háttér, kép, átmenet,     élő grafika, OBS
 (doc.rs)                                                                         kulcsképkockák
      ▲                                       mellettük: diff.rs (C1) · validate.rs (C2) · fit.rs (C3) · template.rs (C6)
      │
 [C24 Story / BacklotHub → ugyanolyan sorok]
```

| A terv eleme | Melyik lépésbe épül | Hogyan (javaslat) | Mit kap ingyen a lánctól |
|---|---|---|---|
| Címek, nyitó képsor, fejezetcím, alsó sáv (1.1; C16) | 3–5 | új oldaltípus a `Card` / `Scroll` / `Crawl` mellé (`style.rs` `Behaviour`): az oldal egy `.fcgt` grafikát tölt ki a blokk fej / törzs / láb mezőiből; a 4. lépés ilyenkor nem blokkokat rendez, hanem a sablon vezérlőit tölti; az 5. lépés ugyanúgy grafikus klipet tesz le | frissítés a táblázatból, ellenőrzés, igazítás a zenéhez, sablon, élő cue (egy alsó sáv élőben is adható) |
| Typography Engine (2.1 B; C18) | 3 → a közös `crates/text` | az új szövegképességek (körív, perspektíva, ragyogás, betűnkénti animáció) a grafikus rétegbe kerülnek; a `ContentStyle` / `LetterStyle` új mezői ezekre képeződnek le | minden oldaltípus és az élő grafika is használja |
| Motion Graphics, Transition (2.1 C, F; C19, C20) | 5 | a kártyák mozgása ma is kulcsképkocka (`card_keys`); új mozgás- és átmenet-előbeállítás a stílusban, a közös átmenet-rendszerből | visszavonás, szerkesztés az Effect Controlsban |
| Credits Composer (2.1 D) | 5 | a háttér, kép, videó, ismétlődő videó már itt kerül a klipek alá (C4, C5, C11, C13); a hiányzó médiaréteg a grafikán belül a grafikai rendszer bővítése | – |
| Audio Sync, narráció (2.1 E, 5.1; C21) | `fit.rs` | a zenéhez igazítás (C3) általánosítása: a hossz forrása lehet narráció is, és egy blokk megjelenése egy kimondott szóhoz köthető (a szóidők a feliratozóból) | a meglévő „csak javasol, jóváhagyásra igazít” működés, egy visszavonás |
| Projektformátum (3.1) | 1 (`doc.rs`) | a `.cfa4credits` új verziója: oldaltípusok, kézi módosítások (C22), narráció-hivatkozás; a régi fájlok megnyílnak | más gépen is megnyílik táblázat nélkül (C1) |
| Template Manager (3.2; C17) | `template.rs` | kategória és előnézeti kép a `.cfa4credtpl`-ben, a `.fcgt` grafikasablonok egy listában | – |
| Visszaírás a szerkesztőből (7.1; C22) | `diff.rs` → `doc.rs` | a C1 ma felismeri és megtartja a kézi módosítást; ezt a dokumentumba is el kell menteni | frissítéskor nem vész el |
| Story-híd (4; C24) | 1 | egy adapter a Story / BacklotHub adataiból táblázatsorokat ad; onnan a lánc változatlan | eltérés soronként, részleges elfogadás, eredet megőrzése (C1) |
| Önálló alkalmazás (6; C23) | Qt6 (`credits_workspace.py`) | ugyanaz a munkaterület, a `.cfa4credits` mint projekt | minden fenti |
| Vonalrajz-animáció (2.1 C, D; C11–C12, [CFA4-LINEART-001](CFA4-LINEART-001.md)) | 2, 3, 5 | ma (C11): ismétlődő videóháttér a külső eszközzel renderelt animációból; a jövő évi kiadásban saját grafikus réteg (`LayerContent::LineArt`): a 3. lépésben háttérfajta (`"kind": "lineart"`), a 2. lépésben cellacímke (`{{LineArt walker …}}`), az 5. lépésben ugyanaz a grafikus klip | címekben, alsó sávokban, élő grafikában változtatás nélkül; frissítés, ellenőrzés (a kontraszt a napszak-palettához mérhető) |
| Live Graphics (8) | 6 (`engine/live.rs`) | a C10 futtató a dokumentumból készít cue-kat; az új oldaltípusok (cím, alsó sáv) cue-ként is megjelennek | PREVIEW / TAKE / NEXT / UPDATE / OUT |

Példa a kész állapotra: **egy** `.cfa4credits` dokumentum, **egy** táblázattal: az első oldala a
főcím (`Title`), utána három alsó sáv a szereplőknek (`LowerThird`), egy fejezetcím, végül a
gördülő stáblista. Egy paranccsal kerül az idővonalra, a zenéhez vagy a narrációhoz igazítva; ha a
táblázat változik, egy frissítéssel mind cserélődik, a kézi módosítások megmaradnak; az alsó sávok
ugyanebből élőben is adhatók.

### 0.5. A vonalrajzos stáblista helye (C11, C12 = CFA4-LINEART-001)

A vonalrajzos stáblista ugyanebbe a láncba épül, és a terv több pontját is lefedi:

- **Most (C11, kész):** a vonalrajz-animáció külső eszközzel MP4-be renderelve, a stáblista
  oldalstílusában ismétlődő videóháttérként (`loop`); a C13 óta cellában is (`{{Video … loop}}`).
  Ez a terv 2.1 D „háttér” és „animált objektum” elemének első formája.
- **A jövő évi kiadásban (C12, [CFA4-LINEART-001](CFA4-LINEART-001.md), jóváhagyva):** a CFA4
  maga rajzolja, **grafikus rétegként** (`LayerContent::LineArt`). Mivel réteg, nem
  stáblista-specifikus elem, a terv minden megjelenési formájában ugyanaz: stáblista-háttér és
  -cella (a nevekkel haladó figura), cím és alsó sáv (C16 oldaltípusai is használhatják), élő
  grafika (C10). Ezzel a terv 2.1 C „Motion Graphics” elemének „2D grafikai animáció” pontja is
  teljesül.
- **Kapcsolódás az új fázisokhoz:** a LINEART D6 kulcskockázható mozdulatai (megáll, köszön,
  továbbmegy) a C21 narrációhoz igazítással köthetők egy kimondott névhez; a napszak-paletta a C2
  kontrasztellenőrzését mérhetővé teszi mozgó háttér előtt.
- **Minta a donorokhoz (E11):** a LINEART a terv 9–10. pontjában kért donorkezelés kész példája:
  MIT licenc, csak a matematika kerül át Rustra, a figurák saját tervezésűek, a rajzolás a CFA4
  saját renderelőjében, külső böngésző vagy renderelőmotor nélkül. A Cinematic Skill, a sand3d és
  a Papercut vizsgálata ugyanezt a mintát követheti.
- **Ütemezés:** a LINEART saját terv és saját munkamenet marad (D7); az új terv nem változtat
  rajta, csak hivatkozik rá.

### 0.6. Műfaji sablonok (Q17, a C17 része)

A tulajdonos döntése (2026-11-14): **filmműfajonként legalább egy-egy** beépített sablon készül,
amely szerkeszthető és új néven elmenthető. Mindegyik a teljes láncot használja: főcím és nyitó
képsor (C16), alsó sávok, fejezetcím, záró stáblista, háttér, átmenet, mozgás, és ahol illik,
vonalrajz (C12). A beépített sablonok saját tervezésűek, MIT/Apache licencűek (C6), saját vagy
szabad licencű betűtípussal (`AGENTS.md` §1.9), minden nyelven elnevezve (Q15).

Javasolt műfajlista (bővíthető): dráma, vígjáték, akció, thriller és krimi, horror, sci-fi,
fantasy, romantikus, animációs és családi, dokumentumfilm, történelmi és kosztümös, háborús,
western, zenés és musical, táncfilm, paródia, rövidfilm és fesztiválfilm, élő műsor és közvetítés (ez az Élő grafikában, C10, cue-ként is betölthető: főcím, alsó sávok, stáblista).

Szerkesztés és mentés: a beépített sablon megnyitva szerkeszthető, mentéskor **új néven**
felhasználói vagy projekt-sablonként (vagy `.cfa4credtpl` fájlként) kerül el; a beépített
változat nem íródik felül, így mindig visszaállítható. A sablonválasztóban a műfaj a terv 3.2
tartalmi kategóriái mellett külön szűrő.

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
| C12 vonalrajz-réteg Rustban | jóváhagyott terv: [CFA4-LINEART-001](CFA4-LINEART-001.md) (jövő évi kiadás) | 2.1 C (2D grafikai animáció), 2.1 D (animált objektum) |
| C13 ismétlődő videó cellában (`{{Video walk.mp4 loop}}`) | PR #1188 | 2.1 D |
| C14 horgas gerincek (`Hook N From-To dx dy`) | PR #1196 | 2.1 D (elrendezés) |

### 1.2. A terv komponensei (2.1)

**Döntés (tulajdonos, 2026-11-14): az itt hiányzóként felsorolt képességeket pótolni kell.** Mindegyik
hiány egy fázishoz van kötve (→ C16–C21, 0.3).

| Pont | Állapot | Mi van ma | Mi hiányzik |
|---|---|---|---|
| A. Credits Core | részben | `crates/credits` (adat, elrendezés, ellenőrzés) és az `engine` `graphics.credits.*` parancsai; minden felület ugyanazokat hívja (credits-plan 3.1) | jelenet- és rétegmodell a stáblistán túl (címek, alsó sávok közös projektje), lásd E4–E5 → **C16** |
| B. Typography Engine | részben | `crates/text`: betűtípus, méret, vastagság, betűköz, sortáv, tördelés (pont- és bekezdésszöveg), karakterenkénti stílus, árnyék (elmosással), két kontúr, háttérdoboz, színátmenet, hiányzó karakter jelzése, olvashatósági ellenőrzés; Text Animator: *Typewriter*, *Word by Word* ([`graphics.md`](../graphics.md)) | körív menti szöveg, perspektivikus szöveg, ragyogás, maszkolás szöveggel (*Not yet* a `graphics.md`-ben), kulcsképkockázható betűnkénti animáció → **C18** |
| C. Motion Graphics Engine | részben | kulcsképkocka minden animálható tulajdonságon, easing-előbeállítások, Motion (pozíció, méret, forgatás) és Opacity; 2D „kamera” a Motion nagyításával és mozgatásával | valódi kamera (3D), rétegenkénti keverési módok, objektumcsoport közös animációja → **C19**; maszkos átmenet → **C20** |
| D. Credits Composer | részben | a CFA4 idővonala: grafikus klipek, kép, videó, hang, effekt, átmenet külön sávon, saját időzítéssel; stáblistában kép és videó (C5) | médiaréteg a grafikán belül (*Not yet* a `graphics.md`-ben) → **C19**; jelenetek a stáblistán túl → **C16**; külön jelenetszerkesztő nincs (E5) |
| E. Audio Synchronization | részben | zenéhez igazítás ütemfelismeréssel (C3), hullámforma és több hangsáv az idővonalon, helyi beszédfelismerés szóidőkkel (`crates/speech`), narráció (`tts.*`) | narrációvezérelt animáció (5.1), szavankénti kiemelés (E12) → **C21** |
| F. Transition Engine | részben | a meglévő átmenet-rendszer (áttűnés, eltolás, csúsztatás, törlés, áttűnés feketén át és bármely átmenet azonosítója), az idővonalon külön szerkeszthető objektumként; oldalak közt (C4) | fény-, maszkos, kameramozgásos és tipográfiai átmenet mint külön típus → **C20** |

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
| E2 | Tisztaszoba és a stáblista-terv licencpontja | (a terv csak licencellenőrzést ír, F0) | A `credits-plan.md` 3.5 szerint a Cinecred kódja „felhasználható”; a [`licensing.md`](../licensing.md) C5 szerint a C7 a formátumot „a dokumentációjából és publikált forrásából” írta le (kód nem került át). A stáblista-munka mai szabálya: csak a nyilvános útmutató (cinecred.com/guide), a forráskód soha | A `credits-plan.md` 3.5 kapjon megjegyzést a mai szabályról; a `licensing.md` sora történeti nyilvántartás, marad. Döntés: Q2 |
| E3 | Önálló alkalmazás (1., 6., F5) | két önálló megjelenési forma, teljes funkcionalitással | `credits-plan.md` 8.1 (2026-10-25): „csak Qt6, a CFA4-en belül”; utána a `VISION.md` 2.7 (2026-10-30): külön is indítható részek, egy motor, egy csomag; a C9 óta `cfa4-qt --credits FÁJL` a teljes munkaterületet nyitja | A `VISION.md` 2.7 mintája: külön indítási mód és menübejegyzés, ugyanaz a motor és parancsok; külön kódbázis nem. Döntés: Q3 |
| E4 | Projektformátum (3.1, F1) | `.cfa4credits` teljes jelenetprojekt (jelenetek, rétegek, animációk, hang, effektek) | A `.cfa4credits` szándékosan szűk: forrás, stílus, sorok, illesztés (credits-plan 3.4, 7. utolsó sor); a jelenetek, rétegek és animációk a `.fcproj` grafikus klipjeiben élnek | A `.cfa4credits` maradjon a stáblista adatforrása; jelenet-szintű tartalom csak akkor kerüljön bele, ha az önálló indítás (E3) projekt nélkül is igényli. Döntés: Q4 |
| E5 | Külön motorok (2.1 B–F) | Typography, Motion Graphics, Composer, Audio Sync és Transition Engine mint komponensek | Egy közös mag (credits-plan 3.1): a szöveg a `crates/text`, az animáció a grafikus klipek kulcsképkockái, a kompozíció az idővonal, az átmenet a meglévő átmenet-rendszer; „nincs külön stáblista-motor” | A terv komponensei a meglévő CFA4-elemekre képeződjenek le (1.2. táblázat), párhuzamos motor nélkül. Döntés: Q4 |
| E6 | Story-alkalmazás (4., F7) | a filmadatok elsődleges forrása | A CFA4-ben nincs Story-alkalmazás; a híd halasztva (credits-plan 7.); a 4.1 szinkronszabályai a táblázatra a C1-ben már teljesülnek | A híd maradjon halasztva, amíg nincs Story-adatforrás; akkor a C1 eltérés/frissítés mintáját használja. Döntés: Q5 |
| E7 | PostgreSQL (11., 4.2 külső adatbázis) | a CFA4 központi adatkezeléséhez igazítva | Az [ADR 0006](../adr/ADR-0006-backlothub-postgresql.md) a BacklotHub produkciókezelő adattára; a stáblista fájlokkal dolgozik, adatbázis nélkül | A stáblista ne igényeljen adatbázist; a „külső produkciós adatbázis” forrása a BacklotHub legyen, amikor a stáb- és szereplőadat ott megjelenik. Döntés: Q5 |
| E8 | Python helye (11., 10.) | „Python csak indokolt, izolált segédfolyamatokban”; Blender és Python nem kerülhet a Qt6 főfolyamatba | A Qt6-felület maga PySide6 (Python) a főfolyamatban ([`qt-gui.md`](../qt-gui.md)); az üzleti logika a Rust-motorban fut (`cfa4-cli serve`); `requires-python >= 3.12`, a CI 3.13-at futtat | A szöveg pontosítása: „üzleti logika Rustban; Python a Qt6-felületben (PySide6) és izolált segédfolyamatokban; Blender soha nem a felület folyamatában”. A Python 3.14-et a CI-mátrixba érdemes felvenni. Döntés: Q6 |
| E9 | Sablonkategóriák (3.2) | 10 tartalmi kategória, előnézeti kép | `.cfa4credtpl`: beépített / felhasználói / projekt; a `.fcgt` saját `category` mezővel | A forrás szerinti hármas maradjon, mellé a terv 10 tartalmi kategóriája és előnézeti kép a sablonválasztóban. Döntés: Q7 |
| E10 | Visszaírás a kreditprojektbe (7.1) | a szerkesztőben végzett módosítás visszakerül az eredeti kreditprojektbe | C1: a kézi módosítást a frissítés megtartja, de a táblázatba vagy a `.cfa4credits`-be nem ír vissza; a CFA4-ben a táblázat a forrás | A táblázat maradjon az igazság forrása; a kézi módosítás a stáblista-dokumentumban tárolódjon (mentéskor a `.cfa4credits`-be), a táblázatba visszaírás csak kérésre. Döntés: Q8 |
| E11 | Donorok (9., 10.) | Cinematic Skill, sand3d, Papercut Explainer vizsgálata; Skia-renderelés vizsgálata | A repóban nincs róluk vizsgálat; a licencük nincs nyilvántartva; a CFA4 saját renderelőt használ, Skia nem függőség; a 3D mélysége a `VISION.md` 8. pontjában nyitott | Az F0 donorvizsgálata külön dokumentum, a [CFA4-LINEART-001](CFA4-LINEART-001.md) mintájára (0.5): első a licenc (GPL-3.0-kompatibilitás, [`licensing.md`](../licensing.md)), kódátvétel csak tulajdonosi döntés után, az [`AGENTS.md`](../../AGENTS.md) C2 szerint. Döntés: Q9 |
| E12 | Narrációvezérelt animáció és szavankénti kiemelés (5.1–5.2) | a generátor része | A feliratozó tervének része ([SUBTITLE-STUDIO 1.2](SUBTITLE-STUDIO-PLAN-001-FINAL-1.2.md) F6 karaoke, 6. caption → narráció szerződés); a [`gaps.md`](../gaps.md) „Word Highlight captions” jelöltje; a szóidőket a `crates/speech` adja | Egy helyen készüljön (feliratozó + szóidők), a cím- és kreditgenerátor azt használja. Döntés: Q10 |
| E13 | AI-modellek telepítése (11., 13.10) | a CFA4 csak letöltési hivatkozást, ajánlott helyet és tárhelyigényt adhat | A beszédfelismerés modelljeit és a Kokoro-hangcsomagot (336 MB) a CFA4 tölti le, de csak a felhasználó jóváhagyása után (`transcript.downloadModel`, `tts.downloadVoices`) | Ha a terv a jóváhagyott, alkalmazáson belüli letöltést is tiltja, az a meglévő funkciókat érinti; javaslat: „automatikusan nem, a felhasználó jóváhagyásával igen”. Döntés: Q11 |
| E14 | Fázisnevek (12.) | F0–F9 | A CFA4-ben az F-azonosítók foglaltak (ADR F8 Qt6, ADR F10 stáblista; a feliratozó F0–F6 fázisai) | A terv fázisai kapjanak saját előtagot (pl. TC-F0…TC-F9), a megvalósítás pedig folytassa a C-számozást (C15-től). Döntés: Q12 |

## 3. Kérdések és döntések

| # | Kérdés | Javaslat | Döntés |
|---|---|---|---|
| Q1 | A Cinecred „kizárólag adatforrás-referencia” mondata megállítja-e a Cinecred-szerű elrendezési munkát (C13–C15, egységes törzsszélesség) (E1)? | Nem: az import marad kompatibilitási referencia, az útmutatóból leírt elrendezési képességek a CFA4 saját funkciói | Elfogadva a javaslat szerint (tulajdonos, 2026-11-14) |
| Q2 | A `credits-plan.md` 3.5 kapjon-e megjegyzést a tisztaszobás szabályról (E2)? | Igen; a `licensing.md` sora marad | Elfogadva a javaslat szerint (tulajdonos, 2026-11-14) |
| Q3 | Az önálló alkalmazás a `VISION.md` 2.7 szerinti külön indítási mód legyen-e (E3)? | Igen: `cfa4-qt --credits` bővítése saját menübejegyzéssel és ikonnal, egy motorral | Elfogadva a javaslat szerint (tulajdonos, 2026-11-14) |
| Q4 | A terv komponensei és projektformátuma a meglévő CFA4-elemekre képeződjenek-e le (E4–E5)? | Igen: nincs párhuzamos motor; a `.cfa4credits` adatforrás marad, jelenet-tartalom csak az önálló indításhoz, ha kell | Elfogadva a javaslat szerint (tulajdonos, 2026-11-14) |
| Q5 | Story-híd és külső adatbázis: halasztás, majd BacklotHub (E6–E7)? | Igen: halasztva; adatbázis a stáblistához nem kell | Elfogadva a javaslat szerint (tulajdonos, 2026-11-14) |
| Q6 | Pontosítsuk-e a Python helyét a tervben, és kerüljön-e a Python 3.14 a CI-be (E8)? | Igen, mindkettő | Elfogadva a javaslat szerint (tulajdonos, 2026-11-14) |
| Q7 | Bővüljenek-e a sablonok a 10 tartalmi kategóriával és előnézeti képpel (E9)? | Igen, a forrás szerinti hármas mellett | Elfogadva a javaslat szerint (tulajdonos, 2026-11-14) |
| Q8 | Hová kerüljenek a szerkesztőben végzett kézi módosítások (E10)? | A stáblista-dokumentumba / `.cfa4credits`-be; a táblázatba csak kérésre | Elfogadva a javaslat szerint (tulajdonos, 2026-11-14) |
| Q9 | Induljon-e az F0 donorvizsgálat, és milyen sorrendben (E11)? | Igen, külön dokumentumban: licenc első, kódátvétel csak döntés után | Elfogadva a javaslat szerint (tulajdonos, 2026-11-14) |
| Q10 | A narrációvezérelt animáció és a szavankénti kiemelés a feliratozó tervében készüljön-e (E12)? | Igen, a cím- és kreditgenerátor használja | Elfogadva a javaslat szerint (tulajdonos, 2026-11-14) |
| Q11 | Megengedett-e a jóváhagyással indított, alkalmazáson belüli modell-letöltés (E13)? | Igen (a mai működés); automatikus telepítés nem | Elfogadva a javaslat szerint (tulajdonos, 2026-11-14) |
| Q12 | Milyen azonosítót kapjanak a terv fázisai (E14)? | TC-F0…TC-F9; a megvalósítás C15-től | Elfogadva a javaslat szerint (tulajdonos, 2026-11-14) |
| Q13 | Elfogadod-e a 0.3 szerinti folytatást (C16–C24) a meglévő stáblista-generátorra építve? | Igen, a C15 és a törzsszélesség után C16-tal | Elfogadva a javaslat szerint (tulajdonos, 2026-11-14), az 1.2 hiányainak pótlásával (C18–C21) |
| Q14 | Milyen felület készül a C16–C24-hez? | Csak Qt6 | Eldöntve (tulajdonos, 2026-11-14): csak Qt6 GUI, egui nem készül |
| Q15 | Milyen nyelveken érhető el az alkalmazás? | A CFA4 összes támogatott nyelvén | Eldöntve (tulajdonos, 2026-11-14): a CFA4 összes támogatott nyelvén (angol forrás + minden `i18n/*.tsv` katalógus) |
| Q16 | Menthető-e az elkészült stáblista sablonként? | Igen | Eldöntve (tulajdonos, 2026-11-14): igen, minden elemével (C17); a sablon a tartalmat nem viszi, csak mintatáblázatot (terv 3.2) |
| Q17 | Legyenek-e műfaji sablonok? | Igen | Eldöntve (tulajdonos, 2026-11-14): filmműfajonként legalább egy, szerkeszthető és új néven menthető (0.6, C17) |
