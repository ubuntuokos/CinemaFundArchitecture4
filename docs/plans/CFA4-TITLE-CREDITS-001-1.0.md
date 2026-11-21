# CFA4-TITLE-CREDITS-001 – Cím-, felirat- és kreditgenerátor – Fejlesztési terv – 1.0

> **Eredet és állapot.** A tervet a tulajdonos (ubuntuokos) 2026-11-14-én adta át összeegyeztetésre.
> **Állapota: JÓVÁHAGYVA (a tulajdonos, 2026-11-14)**, a CFA4-illesztési melléklet döntéseivel (Q1–Q18) együtt; ahol a melléklet döntése eltér a szövegtől, a döntés érvényes. A szöveg itt változatlan; csak a formázás (címsorok,
> felsorolások) igazodik a Markdownhoz. A CFA4 `main`-ben rögzített szabályaihoz és a meglévő
> stáblista-tervhez ([`docs/credits-plan.md`](../credits-plan.md), C1–C14) való illesztés a
> [CFA4-illesztési mellékletben](CFA4-TITLE-CREDITS-001-1.0-CFA4-ILLESZTES.md) van: mi kész, mi
> hiányzik, hol tér el, és milyen döntések kellenek. A melléklet kérdései a tulajdonos döntéséig
> nyitottak, és nem módosítják ezt a szöveget.
>
> **Folytatás:** a döntésekkel (Q1–Q18) beépített szöveg: [1.1](CFA4-TITLE-CREDITS-001-1.1.md). Ez az 1.0 változatlan, történeti szöveg.

| Adat | Érték |
|---|---|
| Azonosító | CFA4-TITLE-CREDITS-001 |
| Cím | Cím-, felirat- és kreditgenerátor |
| Változat | Fejlesztési terv – 1.0 |
| Állapot | Jóváhagyva (2026-11-14), a melléklet döntéseivel |
| Projekt | CinemaFundArchitecture4 (CFA4) |
| Célplatform | Ubuntu 26.04 / amd64 |
| Grafikus környezet | Qt6 |
| Elsődleges technológiák | Rust, Qt6, Python (izolált segédfolyamatokban), FFmpeg |
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

## 4. Adatkapcsolat a Story alkalmazással

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

## 6. Önálló alkalmazás

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
- PostgreSQL a CFA4 központi adatkezelési architektúrájához igazítva.
- FFmpeg a CFA4 renderelési infrastruktúrájához kapcsolódva.
- Python csak indokolt, izolált segédfolyamatokban.
- GPU-gyorsítás opcionálisan.
- CPU-alapú működés kötelezően biztosítandó.

A rendszer nem követelhet meg AI-modellt.

A TTS és egyéb AI-funkciók opcionálisak. A modellek nem kerülhetnek automatikusan telepítésre. A
CFA4 csak letöltési hivatkozást, ajánlott telepítési helyet és tárhelyigényt biztosíthat.

## 12. Fejlesztési fázisok

### F0 – Követelmény- és donorvizsgálat

- Korábbi CFA3 és CFA4 tervek ellenőrzése.
- A jelenlegi CFA4 architektúra ellenőrzése.
- Cinematic Skill részletes auditja.
- Papercut és sand3d korábbi vizsgálatainak felhasználása.
- Licencellenőrzés.
- Függőségvizsgálat.
- Python 3.14 kompatibilitás.
- Renderelési technológia kiválasztása.

Elfogadási feltétel: jóváhagyható műszaki specifikáció.

### F1 – Credits Core

- Projektformátum.
- Jelenet- és rétegmodell.
- Objektumkezelés.
- Időzítés.
- Mentés és visszatöltés.
- Projektvalidáció.

Elfogadási feltétel: stabil, tesztelhető projektmotor.

### F2 – Typography és Motion Graphics

- Szövegmotor.
- Animációs rendszer.
- Kulcsképkockák.
- Átmenetek.
- Kameramozgás.
- Grafikai kompozíció.

Elfogadási feltétel: összetett animált címsorok készítése.

### F3 – Credits Composer és Template Manager

- Réteges kompozíció.
- Kép- és videóobjektumok.
- Sablonok.
- Előnézet.
- Effektkezelés.

Elfogadási feltétel: teljes filmcím- és stáblistajelenet összeállítása.

### F4 – Audio és Narration Synchronization

- Hangimport.
- Narráció.
- Feliratszinkron.
- Idővonal.
- Zenei ütemhez igazítás.

Elfogadási feltétel: narrációvezérelt animált jelenet.

### F5 – Önálló Qt6 alkalmazás

- Teljes grafikus felület.
- Projektkezelés.
- Sablonkezelő.
- Előnézet.
- Exportálás.

Elfogadási feltétel: önálló alkalmazásként használható rendszer.

### F6 – CFA4 videószerkesztő-integráció

- Natív idővonal-objektum.
- Kétirányú projektkapcsolat.
- Gyorsítótárazott előnézet.
- Újrarenderelés.
- Módosítások visszaírása.

Elfogadási feltétel: a szerkesztőből megnyitható és visszaszerkeszthető kreditprojekt.

### F7 – Story Data Bridge

- Metaadatimport.
- Változáskövetés.
- Adatforrás-kezelés.
- Elfogadás és elutasítás.
- Helyi felülírás.

Elfogadási feltétel: adatvesztés nélküli szinkronizálás.

### F8 – OBS és Live Graphics

- Browser Source.
- obs-websocket.
- Élő grafikai kiszolgálás.
- Vezérlési parancsok.
- Show/Rundown kapcsolat.

Elfogadási feltétel: stabil élő grafikai működés.

### F9 – Integrációs és minőségbiztosítási kapu

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

**Jóváhagyási megjegyzés:** Ez a dokumentum fejlesztési tervjavaslat. A terv elfogadása, a donorok
kiválasztása, a kódátvétel és az implementáció külön döntést igényel.
