# CFA4-TITLE-CREDITS-001 – Cím-, felirat- és kreditgenerátor – Fejlesztési terv – 1.1

> **Eredet és állapot.** Az 1.0 szöveget a tulajdonos (ubuntuokos) 2026-11-14-én hagyta jóvá, a
> [CFA4-illesztési melléklet](CFA4-TITLE-CREDITS-001-1.0-CFA4-ILLESZTES.md) döntéseivel (Q1–Q18) együtt. Az 1.1 ugyanaz a szöveg, a
> döntésekkel beépítve: ahol egy döntés a szöveget módosítja, ott „1.1 – döntés” jelölésű
> megjegyzés áll, és az érintett mondat a döntés szerint pontosított. Új döntést az 1.1 nem tartalmaz;
> a változások listája a 17. fejezetben van. Az 1.0 szöveg változatlanul megmarad:
> [CFA4-TITLE-CREDITS-001-1.0.md](CFA4-TITLE-CREDITS-001-1.0.md). A megvalósítás a meglévő
> stáblista-generátorra épül ([`docs/credits-plan.md`](../credits-plan.md), C1–C15 = Credits Core),
> fázisai C16–C24 (melléklet 0.3).

| Adat | Érték |
|---|---|
| Azonosító | CFA4-TITLE-CREDITS-001 |
| Cím | Cím-, felirat- és kreditgenerátor |
| Változat | Fejlesztési terv – 1.1 (az 1.0 a döntésekkel) |
| Állapot | Jóváhagyva (2026-11-14), a melléklet döntéseivel |
| Projekt | CinemaFundArchitecture4 (CFA4) |
| Célplatform | Ubuntu 26.04 / amd64 |
| Grafikus környezet | Qt6 (csak Qt6 / PySide6, egui nem; Q14) |
| Elsődleges technológiák | Rust (üzleti logika), Qt6 / PySide6 (felület), Python izolált segédfolyamatokban is, FFmpeg (Q6) |
| Nyelvek | A CFA4 összes támogatott nyelve: angol forrás + minden `i18n/*.tsv` katalógus (Q15) |
| Üzemmód | Offline-first, helyi feldolgozás |

## 1. A rendszer célja

A CFA4 Cím- és kreditgenerátor egy teljes értékű filmes grafikai és tipográfiai munkakörnyezet.

Nem egyszerű feliratkészítő. Olyan eszköz, amellyel professzionális filmcímek, nyitó- és
záróképsorok, stáblisták, animált feliratok, narrációhoz igazított szövegek, vizuális átvezetések
és összetett grafikai jelenetek készíthetők.

A rendszernek két önállóan használható megjelenési formája lesz:

1. Önálló CFA4 alkalmazás.
2. Integrált modul a CFA4 videószerkesztőben.

A két megjelenési forma ugyanazt a Credits Core rendszert használja.

### 1.1. Fő célterületek

- Filmek és sorozatok főcímei.
- Záró stáblisták.
- Szereplők és stáb bemutatása.
- Animált filmfeliratok.
- Dokumentumfilmek grafikai elemei.
- Alsó információs sávok (Lower Thirds).
- Fejezetcímek és köztes címkártyák.
- Narrációhoz igazított szövegek.
- Mozgó tipográfia.
- Vizuális átvezetések.
- Élő műsorok és közvetítések grafikai elemei.

## 2. Alapvető rendszerarchitektúra

> **1.1 – döntés:** a komponensek nem külön motorok, hanem a meglévő CFA4-elemekre képeződnek le (Q4; melléklet [1.2](CFA4-TITLE-CREDITS-001-1.0-CFA4-ILLESZTES.md) és [0.4](CFA4-TITLE-CREDITS-001-1.0-CFA4-ILLESZTES.md)): Credits Core = `crates/credits` + a stáblista-dokumentum; szöveg = `crates/text`; mozgás = a grafikus klipek kulcsképkockái; kompozíció = az idővonal; átmenet = a meglévő átmenet-rendszer. Párhuzamos motor nem készül.

A rendszer központi eleme a Credits Core.

Ez biztosítja, hogy az önálló alkalmazás, a videószerkesztő és az OBS-integráció ugyanazokat a
projekteket, sablonokat és grafikai objektumokat kezelje.

### 2.1. Fő komponensek

#### A. Credits Core

Központi projekt- és adatkezelő.

Feladatai:

- Projektstruktúra kezelése.
- Szöveges és grafikai objektumok kezelése.
- Időzítés és animációs kulcsképkockák.
- Rétegek és szegmensek.
- Projektverziózás.
- Objektumok és hivatkozások kezelése.
- Importálás és exportálás.
- Renderelési feladatok előkészítése.

#### B. Typography Engine

Filmes tipográfiai motor.

Funkciók:

- Betűtípus- és betűcsalád-kezelés.
- Betűméret, vastagság, térköz és sortávolság.
- Szövegtördelés.
- Betűnkénti és szavankénti animáció.
- Karakterenkénti megjelenítés.
- Maszkolás és szövegfelfedés.
- Körív mentén elhelyezett szöveg.
- Perspektivikus szöveghatás.
- Árnyék, kontúr, ragyogás és fényhatások.
- Többnyelvű és Unicode-szövegkezelés.

#### C. Motion Graphics Engine

Mozgó grafikai motor.

Funkciók:

- Kulcsképkockás animáció.
- Pozíció, méret, forgatás és áttetszőség.
- Kameramozgás.
- Zoom és panoráma.
- 2D grafikai animáció.
- Rétegek közötti mozgás.
- Maszkok és átmenetek.
- Animációs görbék és easing.
- Objektumok csoportos animációja.
- Több objektum szinkronizálása.

#### D. Credits Composer

Komplex jelenetszerkesztő.

Egy jelenet több különböző elemből épülhet fel:

- Szöveg.
- Fénykép.
- Videó.
- Grafika.
- Háttér.
- Animált objektum.
- Hang.
- Effekt.
- Átmenet.

Minden elem külön rétegen és időzítéssel kezelhető.

#### E. Audio Synchronization Engine

Hang- és narrációszinkronizáló.

Funkciók:

- Narráció importálása.
- Hanghullámforma megjelenítése.
- Szövegrészek időzítése.
- Mondat- és szószintű szinkron.
- Feliratok időzítése.
- Zenei ütemhez igazított animáció.
- Hang alapján vezérelt időtartam.
- Hangbe- és kifutások.
- Több hangsáv kezelése.

A generátor képes legyen a stáblista hosszát a kiválasztott zenei vagy narrációs anyaghoz igazítani.

#### F. Transition Engine

Vizuális átmenetek.

Támogatott átmenetek:

- Fade in / Fade out.
- Dissolve.
- Wipe.
- Blur.
- Fényátmenetek.
- Maszkos átmenetek.
- Kameramozgásos átvezetés.
- Mozgó tipográfiai átmenetek.
- Rétegek közötti animált átvezetés.
- Egyedi, kulcsképkockás átmenetek.

Az átmenetek külön szerkeszthető objektumok legyenek.

## 3. Projekt- és sablonrendszer

### 3.1. Projektformátum

> **1.1 – döntés:** a `.cfa4credits` a stáblista adatforrása marad (forrás, stílus, sorok, illesztés, kézi módosítások); a jelenetek, rétegek és animációk a `.fcproj` grafikus klipjeiben élnek. Jelenet-szintű tartalom csak akkor kerül a `.cfa4credits`-be, ha az önálló indítás (6.) projekt nélkül igényli (Q4).

A CFA4 saját, verziózott projektformátumot használjon.

Javasolt kiterjesztés: `.cfa4credits`.

A projekt tartalmazza:

- Metaadatok.
- Jelenetek.
- Rétegek.
- Szöveges objektumok.
- Médiahivatkozások.
- Animációk.
- Hanganyagok.
- Időzítések.
- Effektbeállítások.
- Sablonhivatkozások.
- Forrásadatok és eredetinformációk.

A külső médiafájlok alapértelmezésben hivatkozással szerepeljenek. A projektcsomagoláskor a
szükséges állományok opcionálisan összegyűjthetők.

### 3.2. Template Manager

Külön sablonkezelő modul szükséges.

Sablonkategóriák:

1. Filmnyitók.
2. Filmzárók.
3. Hagyományos stáblisták.
4. Görgetett stáblisták.
5. Szereplőbemutatók.
6. Dokumentumfilmes címek.
7. Alsó információs sávok.
8. Fejezetcímek.
9. Animált feliratok.
10. Műsor- és közvetítési grafikák.

A sablonok:

- Szerkeszthetők legyenek.
- Saját névvel menthetők legyenek.
- Kategorizálhatók legyenek.
- Előnézeti képpel rendelkezzenek.
- Importálhatók és exportálhatók legyenek.
- Más projektekben újra felhasználhatók legyenek.

A sablon ne legyen összekeverve a projekt konkrét tartalmával.

> **1.1 – döntés:** a 10 tartalmi kategória és az előnézeti kép a meglévő forrás szerinti hármas (beépített / felhasználói / projekt) mellé kerül (Q7). Az elkészült stáblista minden elemével sablonként menthető; a sablon a tartalmat nem viszi, csak mintatáblázatot (Q16). Filmműfajonként legalább egy beépített, szerkeszthető és új néven menthető sablon készül, bővíthető listával (Q17; a műfajok listája: melléklet [0.6](CFA4-TITLE-CREDITS-001-1.0-CFA4-ILLESZTES.md)). Megvalósítás: C17.

## 4. Adatkapcsolat a Story alkalmazással

> **1.1 – döntés:** a Story-híd halasztva, amíg nincs Story-adatforrás; akkor a táblázatfrissítés meglévő (C1) eltérés-előnézet / elfogadás / elutasítás mintáját használja (Q5). A szereplőknél a szinkronhang-színész is átvehető adat (Q18, C16).

A Story alkalmazás a filmadatok egyik elsődleges forrása.

A Credits Data Bridge feladata az adatok átvétele.

Átvehető adatok:

- Film címe.
- Alternatív cím.
- Rendező.
- Producer.
- Forgatókönyvíró.
- Szereplők.
- Stábtagok.
- Produkciós cégek.
- Gyártási adatok.
- Forgatási helyszínek.
- Fejezetek.
- Egyéb kreditadatok.

### 4.1. Adatszinkronizálás

A Story alkalmazás módosításai ne írják felül automatikusan a már szerkesztett kreditprojektet.

Kötelező funkciók:

- Változás-összehasonlítás.
- Módosítások előnézete.
- Elfogadás.
- Elutasítás.
- Helyi felülírás.
- Forrásadatok megőrzése.
- Stabil objektumazonosítók.

A rendszernek minden importált adatnál meg kell őriznie annak eredetét.

### 4.2. Egyéb adatforrások

Támogatandó:

- CSV.
- XLSX.
- ODS.
- Kézi adatbevitel.
- Külső produkciós adatbázis.
- Strukturált fájlimport.
- Cinecred-kompatibilis adatforrások.

A Cinecred itt kizárólag adatforrási kompatibilitási referencia, nem beépítendő alkalmazás.

> **1.1 – döntés:** a mondat a Cinecred *alkalmazás* beépítését zárja ki; a Cinecred nyilvános útmutatójából (cinecred.com/guide) leírt elrendezési képességek (C13–C15, egységes törzsszélesség) a CFA4 saját funkciói. Tisztaszoba: csak a nyilvános útmutató, a forráskód soha (Q1, Q2). A „külső produkciós adatbázis” forrása a BacklotHub lesz, amikor a stáb- és szereplőadat ott megjelenik; a stáblista adatbázist nem igényel (Q5).

## 5. Narráció és animált feliratok

A rendszer különösen fontos része a narráció és a tipográfia összehangolása.

### 5.1. Narrációvezérelt animáció

Például egy dokumentumfilmben:

- A narrátor kimond egy nevet.
- A név megjelenik a képernyőn.
- A kapcsolódó fénykép vagy dokumentum előtűnik.
- A szöveg kiemelése követi a narrációt.
- A következő információ animált átmenettel érkezik.

### 5.2. Feliratozási lehetőségek

- Hagyományos alsó felirat.
- Szavanként kiemelt felirat.
- Karaoke-stílusú megjelenítés.
- Animált felirat.
- Többsoros felirat.
- Külön szereplőkhöz rendelt színek.
- Többnyelvű felirat.
- Egyedi pozicionálás.
- Háttérrel és kerettel ellátott felirat.

Import és export:

- SRT.
- VTT.
- A CFA4 belső projektformátuma.

A feliratok külön objektumként legyenek szerkeszthetők, ne egyszerűen a videóra égetett szövegként.

> **1.1 – döntés:** a narrációvezérelt animáció és a szavankénti / karaoke-kiemelés egy helyen, a feliratozóval közösen készül (SUBTITLE-STUDIO, szóidők a `crates/speech`-ből); a cím- és kreditgenerátor azt használja (Q10, C21).

## 6. Önálló alkalmazás

> **1.1 – döntés:** az önálló alkalmazás a `VISION.md` 2.7 szerinti külön indítási mód: `cfa4-qt --credits` saját menübejegyzéssel és ikonnal, ugyanazzal a motorral és parancsokkal; külön kódbázis nem készül (Q3, C23). Csak Qt6 felület (Q14), a CFA4 összes nyelvén (Q15).

A Qt6-alapú önálló alkalmazás teljes funkcionalitást biztosítson.

Fő munkaterületek:

1. Projektkezelés.
2. Adatimport.
3. Szöveg- és kreditadat-szerkesztés.
4. Grafikai kompozíció.
5. Animációs szerkesztő.
6. Hang- és narrációs idővonal.
7. Sablonkezelő.
8. Előnézet.
9. Renderelés.
10. Exportálás.

Az alkalmazásban legyen:

- Többpaneles munkafelület.
- Réteglista.
- Idővonal.
- Kulcsképkocka-szerkesztő.
- Szövegformázó panel.
- Médiaeszközök.
- Előnézeti ablak.
- Projektvalidáció.

## 7. Integráció a CFA4 videószerkesztővel

A generátor önálló modul legyen a videószerkesztőben.

A szerkesztőben:

- Új címobjektum létrehozása.
- Kreditprojekt megnyitása.
- Sablon kiválasztása.
- Kreditadatok szerkesztése.
- Animáció módosítása.
- Narrációhoz igazítás.
- Előnézet.
- Idővonalra helyezés.

### 7.1. Natív idővonal-objektum

A teljes kreditprojekt egy szerkeszthető idővonal-objektumként is elhelyezhető legyen.

Az objektum:

- Tartsa meg az eredeti projektkapcsolatot.
- Legyen megnyitható és újraszerkeszthető.
- Támogassa az időtartam módosítását.
- Támogassa a gyorsítótárazott előnézetet.
- Támogassa a teljes minőségű újrarenderelést.

A szerkesztőben végzett módosítások visszakerüljenek az eredeti kreditprojektbe.

> **1.1 – döntés:** a táblázat marad az igazság forrása. A szerkesztőben végzett kézi módosítás a stáblista-dokumentumba kerül (mentéskor a `.cfa4credits`-be); a táblázatba visszaírás csak kérésre (Q8, C22).

A módosítások ne eredményezzenek adatvesztést.

## 8. OBS és élő műsorintegráció

A generátor rendelkezzen külön Live Graphics móddal.

Célterületek:

- Élő közvetítések.
- Stúdióműsorok.
- Interjúk.
- Rendezvények.
- Konferenciák.
- Produkciós műsorrendek.

### 8.1. OBS-kapcsolat

Támogatandó:

- OBS Browser Source.
- obs-websocket.
- Helyi grafikai kiszolgálás.
- Élő előnézet.
- Grafikai elemek betöltése.
- Feliratfrissítés.
- Animált be- és kifutás.

### 8.2. Vezérlési parancsok

- LOAD
- PREVIEW
- TAKE
- NEXT
- UPDATE
- HOLD
- OUT

A vezérlés kapcsolódhasson a CFA4 Show/Rundown rendszeréhez.

A Live Graphics mód ne igényeljen minden alkalommal teljes videórenderelést.

## 9. Cinematic Skill technológiai adaptáció

A `Moh4696/cinematic-skill` projektet technológiai donorként kell kezelni.

Vizsgálandó képességek:

- Folyamatos kameramozgás.
- Narrációhoz igazított jelenetidőzítés.
- Mozgó tipográfia.
- Dokumentum- és fényképanimáció.
- Térképes grafikai megjelenítés.
- Kulcsképkockás átmenetek.
- Hanghoz kapcsolódó animáció.
- Feliratgenerálás.
- Filmes kompozíció.

A teljes projekt változtatás nélküli átvétele nem cél.

A donor képességeit a CFA4 saját Credits Core, Motion Graphics Engine és renderelési
infrastruktúrájához kell igazítani.

A Skia-alapú renderelés külön teljesítmény- és kompatibilitási vizsgálatot igényel.

A donor nem válhat a CFA4 elsődleges renderelőmotorjává.

> **1.1 – döntés:** a donorvizsgálat (Cinematic Skill, sand3d, Papercut Explainer, Skia) külön dokumentum a CFA4-LINEART-001 mintájára: első a licenc (GPL-3.0-kompatibilitás), kódátvétel csak tulajdonosi döntés után (Q9).

## 10. További technológiai donorok

A korábban azonosított projekteket külön kell megvizsgálni:

- sand3d: 3D grafikai képességek és objektumgenerálás.
- Papercut Explainer: magyarázóvideós és animációs munkafolyamatok.
- Cinematic Skill: filmes kompozíció és narrációszinkronizálás.

Ezek vizsgálata nem jelent automatikus kódátvételi engedélyt.

A sand3d esetében az izolált Blender Worker koncepciót kell megtartani. Blender és Python
segédfolyamat nem kerülhet a Qt6 főfolyamatba.

## 11. Technológiai és rendszerkövetelmények

- Ubuntu 26.04 és kompatibilis változatai.
- amd64 architektúra.
- Qt6 grafikus felület.
- Rust-alapú központi üzleti logika.
- PostgreSQL a CFA4 központi adatkezelési architektúrájához igazítva (1.1: a stáblista nem igényel adatbázist; adatbázis-forrás a BacklotHub, Q5).
- FFmpeg a CFA4 renderelési infrastruktúrájához kapcsolódva.
- Üzleti logika Rustban; Python a Qt6-felületben (PySide6) és indokolt, izolált segédfolyamatokban; Blender soha nem a felület folyamatában (1.1, Q6). A CI a Python 3.13 mellett 3.14-en is ellenőriz (nem blokkoló job, #1206).
- GPU-gyorsítás opcionálisan.
- CPU-alapú működés kötelezően biztosítandó.

A rendszer nem követelhet meg AI-modellt.

A TTS és egyéb AI-funkciók opcionálisak. A modellek nem kerülhetnek automatikusan telepítésre. A
CFA4 csak letöltési hivatkozást, ajánlott telepítési helyet és tárhelyigényt biztosíthat.

> **1.1 – döntés:** automatikus telepítés nem; a felhasználó kifejezett jóváhagyásával indított, alkalmazáson belüli letöltés megengedett (a mai `transcript.downloadModel`, `tts.downloadVoices` működés; Q11).

## 12. Fejlesztési fázisok

> **1.1 – döntés:** a fázisok azonosítója TC-F0…TC-F9 (a CFA4-ben az F-azonosítók foglaltak); a megvalósítás a stáblista C-számozását folytatja (Q12, Q13). Megfeleltetés (melléklet [0.3](CFA4-TITLE-CREDITS-001-1.0-CFA4-ILLESZTES.md)):
>
> | Terv-fázis | Megvalósítás |
> |---|---|
> | TC-F0 | Donorvizsgálat külön dokumentumban (Q9); Python 3.14 a CI-ben (Q6, kész) |
> | TC-F1 | C1–C15 + egységes törzsszélesség (kész); C16 címjelenetek (Title / Opening / Chapter / LowerThird oldaltípus `.fcgt`-ből) és szinkronhang-oszlop |
> | TC-F2 | C18 tipográfia; C19 mozgás és kompozíció (2.5D kamera, keverési módok, médiaréteg); C20 átmenetek |
> | TC-F3 | C17 sablonkezelő (10 kategória, előnézet, elkészült stáblista mentése sablonként, műfaji sablonok) |
> | TC-F4 | C21 narráció és animált feliratok (a feliratozóval közösen) |
> | TC-F5 | C23 önálló indítás (`cfa4-qt --credits`) |
> | TC-F6 | C22 kétirányú kapcsolat (kézi módosítások a stáblista-dokumentumban) |
> | TC-F7 | C24 Story / BacklotHub (halasztva) |
> | TC-F8 | élő grafika: a meglévő élő stáblista-cue-k (C10) bővítése, később |
> | TC-F9 | minden C-fázis a CFA4 kapuival (fmt, clippy, layers, assets, tesztek) zárul |
>
> A vonalrajz (C11, C12 = CFA4-LINEART-001) grafikus rétegként kapcsolódik (melléklet 0.5).

### TC-F0 – Követelmény- és donorvizsgálat

- Korábbi CFA3 és CFA4 tervek ellenőrzése.
- A jelenlegi CFA4 architektúra ellenőrzése.
- Cinematic Skill részletes auditja.
- Papercut és sand3d korábbi vizsgálatainak felhasználása.
- Licencellenőrzés.
- Függőségvizsgálat.
- Python 3.14 kompatibilitás.
- Renderelési technológia kiválasztása.

Elfogadási feltétel: jóváhagyható műszaki specifikáció.

### TC-F1 – Credits Core

- Projektformátum.
- Jelenet- és rétegmodell.
- Objektumkezelés.
- Időzítés.
- Mentés és visszatöltés.
- Projektvalidáció.

Elfogadási feltétel: stabil, tesztelhető projektmotor.

### TC-F2 – Typography és Motion Graphics

- Szövegmotor.
- Animációs rendszer.
- Kulcsképkockák.
- Átmenetek.
- Kameramozgás.
- Grafikai kompozíció.

Elfogadási feltétel: összetett animált címsorok készítése.

### TC-F3 – Credits Composer és Template Manager

- Réteges kompozíció.
- Kép- és videóobjektumok.
- Sablonok.
- Előnézet.
- Effektkezelés.

Elfogadási feltétel: teljes filmcím- és stáblistajelenet összeállítása.

### TC-F4 – Audio és Narration Synchronization

- Hangimport.
- Narráció.
- Feliratszinkron.
- Idővonal.
- Zenei ütemhez igazítás.

Elfogadási feltétel: narrációvezérelt animált jelenet.

### TC-F5 – Önálló Qt6 alkalmazás

- Teljes grafikus felület.
- Projektkezelés.
- Sablonkezelő.
- Előnézet.
- Exportálás.

Elfogadási feltétel: önálló alkalmazásként használható rendszer.

### TC-F6 – CFA4 videószerkesztő-integráció

- Natív idővonal-objektum.
- Kétirányú projektkapcsolat.
- Gyorsítótárazott előnézet.
- Újrarenderelés.
- Módosítások visszaírása.

Elfogadási feltétel: a szerkesztőből megnyitható és visszaszerkeszthető kreditprojekt.

### TC-F7 – Story Data Bridge

- Metaadatimport.
- Változáskövetés.
- Adatforrás-kezelés.
- Elfogadás és elutasítás.
- Helyi felülírás.

Elfogadási feltétel: adatvesztés nélküli szinkronizálás.

### TC-F8 – OBS és Live Graphics

- Browser Source.
- obs-websocket.
- Élő grafikai kiszolgálás.
- Vezérlési parancsok.
- Show/Rundown kapcsolat.

Elfogadási feltétel: stabil élő grafikai működés.

### TC-F9 – Integrációs és minőségbiztosítási kapu

- Projektformátum-tesztek.
- Renderelési tesztek.
- Hang- és képszinkron.
- Unicode és többnyelvűség.
- Memóriakezelés.
- CPU/GPU kompatibilitás.
- OBS-kapcsolat.
- Mentés-visszaállítás.
- Külső alkalmazásokkal való együttélés.
- Csomagolási validáció.

Elfogadási feltétel: minden kötelező teszt dokumentált PASS eredménye.

## 13. Kötelező minőségbiztosítási követelmények

1. Nem veszhet el szerkesztési információ exportáláskor.
2. A projekt minden eleme külön szerkeszthető maradjon.
3. A narráció és a felirat időzítése ellenőrizhető legyen.
4. A renderelt eredmény és az előnézet időzítése egyezzen.
5. A hiányzó médiafájlokat a rendszer jelezze.
6. A hibás betűtípusokat és karaktereket jelezze.
7. A projektverziók legyenek visszaállíthatók.
8. Az OBS-kapcsolat megszakadása ne okozzon projektvesztést.
9. CPU-alapú renderelés GPU nélkül is működjön.
10. A rendszer ne telepítsen automatikusan külső AI-modelleket.

## 14. Külső alkalmazásokkal való együttélés

A CFA4 telepítése nem akadályozhatja a donor vagy hasonló alkalmazások külön telepítését és használatát.

Különösen:

- Saját alkalmazásnevek.
- Saját konfigurációs könyvtárak.
- Saját gyorsítótár.
- Saját MIME-típusok.
- Saját IPC- és hálózati portok.
- Elkülönített csomagok.
- Elkülönített projektformátumok.

## 15. Első kiadás javasolt funkcionalitása

Az első használható kiadás tartalmazza:

- Filmcím-generátor.
- Nyitó és záró képsorok.
- Hagyományos és görgetett stáblista.
- Animált tipográfia.
- Alsó információs sávok.
- Egyszerű vizuális átmenetek.
- Képek és videók beillesztése.
- Narrációhoz igazított szöveg.
- Külön hangidővonal.
- Sablonkezelő.
- Projektmentés és újraszerkesztés.
- CFA4 videószerkesztő-integráció.

Az OBS-integráció és az összetettebb élő grafikai funkciók a későbbi fejlesztési szakaszban készüljenek el.

## 16. Végső fejlesztési cél

A CFA4 Cím- és kreditgenerátor legyen egy önállóan is használható, professzionális filmes grafikai rendszer.

Ne korlátozódjon egyszerű feliratok és stáblisták készítésére.

A végső rendszer biztosítsa a teljes folyamatot:

Adatbevitel → Forgatókönyvi és kreditadatok → Grafikai kompozíció → Tipográfia → Animáció →
Narráció → Hangszinkron → Előnézet → Renderelés → Videószerkesztő / OBS.

A Credits Core legyen közös alap, így az önálló alkalmazás, a videószerkesztő és az élő grafikai
rendszer ugyanazt a projektet és ugyanazokat az objektumokat használhatja.

**Jóváhagyási megjegyzés:** A tervet a tulajdonos 2026-11-14-én jóváhagyta, a melléklet döntéseivel.
A donorok kiválasztása és a kódátvétel továbbra is külön döntést igényel (Q9).

## 17. Változások az 1.0-hoz képest

Az 1.1 csak a 2026-11-14-i tulajdonosi döntéseket építi be (melléklet 3. fejezet), új döntést nem hoz.

| Hely | Változás | Döntés |
|---|---|---|
| Fejléc | Csak Qt6 felület; minden CFA4-nyelv; a Python helye | Q6, Q14, Q15 |
| 2. | A komponensek a meglévő CFA4-elemekre képeződnek le, párhuzamos motor nélkül | Q4 |
| 3.1 | A `.cfa4credits` adatforrás marad | Q4 |
| 3.2 | Kategóriák + előnézet; az elkészült stáblista sablonként; műfaji sablonok | Q7, Q16, Q17 |
| 4., 4.2 | Story-híd halasztva; Cinecred-mondat pontosítva, tisztaszoba; BacklotHub; szinkronhang | Q1, Q2, Q5, Q18 |
| 5. | Narráció és kiemelés a feliratozóval közösen | Q10 |
| 6. | Önálló alkalmazás = külön indítási mód | Q3 |
| 7.1 | Kézi módosítások a stáblista-dokumentumba, a táblázatba csak kérésre | Q8 |
| 9., 10. | Donorvizsgálat külön dokumentumban, licenc első | Q9 |
| 11. | Adatbázis, Python, modell-letöltés pontosítva | Q5, Q6, Q11 |
| 12. | TC-F0…TC-F9 és a C16–C24 megfeleltetés | Q12, Q13 |
