# CFA4-DECKLINK-001
# Egységes DeckLink és professzionális Broadcast I/O rendszer

> **Eredet és állapot.** A tervjavaslatot a tulajdonos (ubuntuokos) 2026-11-14-én adta át elemzésre
> és megvalósítási tervezésre. A szöveg itt változatlan; csak a formázás igazodik a Markdownhoz. **A
> beillesztett szöveg az F8 elfogadási feltételeinél („Telepítési együtt…”) megszakad**; a hiányzó
> rész a tulajdonostól pótolandó. A CFA4 `main`-ben rögzített szabályaihoz és meglévő terveihez való
> illesztés, az eltérések és a végrehajtási terv a
> [CFA4-illesztési mellékletben és végrehajtási tervben](CFA4-DECKLINK-001-1.0-CFA4-ILLESZTES.md)
> vannak; azok tulajdonosi döntésig nyitottak, és nem módosítják ezt a szöveget.

**Dokumentumtípus:** Részletes fejlesztési és integrációs terv
**Verzió:** 1.0
**Állapot:** Átadható tervjavaslat
**Célplatform:** Ubuntu 26.04 / Kubuntu 26.04 / Ubuntu Studio 26.04
**Architektúra:** Linux-only, amd64-only
**Projekt:** CinemaFundArchitecture4 (CFA4)

---

## 1. A fejlesztés célja

A CFA4 számára olyan professzionális videó- és hangbeviteli, -kimeneti, valamint broadcast-integrációs rendszer kialakítása, amely támogatja a Blackmagic DeckLink hardvereket, a professzionális SDI-jelkezelést, a nagy bitmélységű és HDR-videót, valamint a későbbi IP-alapú broadcast munkafolyamatokat.

A fejlesztés célja nem egy különálló DeckLink-rögzítőprogram beépítése, hanem egy egységes, a CFA4 meglévő médiakezelési és rögzítési rendszeréhez illeszkedő hardveres I/O infrastruktúra.

A rendszer hosszabb távon az alábbi képességeket biztosíthatja:

- Blackmagic DeckLink eszközök felismerése és kezelése.
- SDI- és HDMI-alapú videobemenet és -kimenet, az adott eszköz képességeinek megfelelően.
- Professzionális hangbemenet és -kimenet.
- Timecode és időbélyeg-kezelés.
- 8, 10 és 12 bites videóformátumok kezelése, ahol a hardver és az SDK támogatja.
- HDR10, PQ és releváns HDR-metaadatok kezelése.
- Tömörítetlen és tömörített rögzítési munkafolyamatok.
- ANC/VANC és broadcast-metaadatok feldolgozása.
- PTP és Genlock alapú szinkronizáció.
- SMPTE ST 2110 IP-video integráció.
- NMOS-alapú eszköz- és kapcsolatkezelés.
- TSL UMD, Tally és broadcast-vezérlés.
- Későbbi Key/Fill és többkimenetes playout.

**Alapelv:** a CFA4 saját médiapipeline-ja marad a rendszer központi eleme. A DeckLink és a broadcast komponensek bemeneti, kimeneti, szinkronizációs és vezérlési szolgáltatásokat adnak ehhez.

## 2. Projektkorlátok és kötelező alapelvek

### 2.1. Platformkorlátozás

A CFA4 támogatott célplatformjai:

- Ubuntu 26.04.
- Kubuntu 26.04.
- Ubuntu Studio 26.04.

Kötelező korlátozások:

- Kizárólag Linux.
- Kizárólag amd64.
- Windows-, macOS-, Android- és más CPU-architektúrákra történő fejlesztés nem része a tervnek.
- A Qt6 a grafikus felület technológiai alapja.
- A KDE az elsődleges asztali környezet, GNOME és Cinnamon kompatibilitással.

A Windows-alapú donorprojektekből kizárólag olyan algoritmus, adatstruktúra vagy protokollmegoldás vizsgálható át, amely Linuxon önállóan megvalósítható.

### 2.2. Technológiai alapok

| Réteg | Technológia | Feladat |
|---|---|---|
| Felhasználói felület | Qt6 | Eszközök, bemenetek, kimenetek és munkafolyamatok kezelése |
| Fő vezérlési réteg | Rust | Eszközkezelés, állapotok, események és koordináció |
| Gyártói SDK-kapcsolat | C++ FFI | Blackmagic SDK és szükséges natív API-k |
| Médiapipeline | CFA4 meglévő közös pipeline | Videó, hang, időbélyeg, metaadat |
| Videófeldolgozás | CFA4 codec- és FFmpeg-integráció | Kódolás, dekódolás, konverzió |
| Kiegészítő média- és protokollintegráció | GStreamer | Indokolt interoperabilitási és speciális feladatok |
| Adatkezelés | PostgreSQL | Eszközprofilok, konfigurációk és metaadatok |

A GStreamer nem hozhat létre a CFA4 FFmpeg-alapú rögzítési rendszerével párhuzamos, önálló és versengő fő rögzítési motort.

### 2.3. CFA4- és donor-együttélés

Minden donorintegrációnál kötelező az eredeti alkalmazásokkal való ütközésmentes együttélés.

Ellenőrizendő területek:

- Alkalmazásnevek és binárisnevek.
- Csomagnevek.
- Telepítési útvonalak.
- Konfigurációs könyvtárak.
- MIME-típusok és fájltársítások.
- D-Bus szolgáltatásnevek.
- Hálózati portok.
- Modell- és médiacache-ek.
- Megosztott könyvtárak.
- Felhasználói és rendszerjogosultságok.

A CFA4 nem írhatja felül az eredeti donoralkalmazás beállításait vagy telepített fájljait.

### 2.4. Licencelési alapelv

Egy repository MIT vagy Apache-2.0 licencelése nem jelenti automatikusan a teljes függőségi lánc azonos licencét.

Külön ellenőrizendő:

- A donor saját forráskódjának licence.
- Almodulok és vendored könyvtárak licence.
- Blackmagic SDK licencfeltételei.
- SDK redisztribúciós korlátozásai.
- Külső binárisok és könyvtárak licence.
- Módosított vagy átvett fájlok licenckötelezettségei.
- Európai terjesztésre gyakorolt korlátozások.

A CFA4-be csak dokumentáltan felhasználható komponens kerülhet. Bizonytalan licenc esetén a donor kizárólag technikai referenciaként használható.

## 3. Célarchitektúra

### 3.1. Rétegmodell

A rendszer nyolc logikai rétegből áll.

**A. Hardver- és SDK-réteg**

- Hivatalos Blackmagic DeckLink SDK.
- DeckLink eszközfelderítés.
- Eszközállapotok.
- Bemeneti és kimeneti képességek.
- Driver- és SDK-kompatibilitás.

**B. Hardware Abstraction Layer**

- Egységes eszközazonosítás.
- Bemeneti és kimeneti képességek leírása.
- Pixelformátumok és módok.
- Audioformátumok.
- Eszközállapotok.
- Hiba- és megszakításkezelés.

**C. DeckLink Backend**

- SDI/HDMI videobemenet.
- Hangbemenet.
- Videokimenet.
- Hangkimenet.
- Timecode.
- Frame- és bufferkezelés.

**D. Közös CFA4 Media Pipeline**

- Videoframe-átadás.
- Hangbuffer-átadás.
- PTS/DTS és időbélyegek.
- A/V szinkronizáció.
- Színinformáció.
- HDR-metaadatok.
- Kódolási és dekódolási útvonal.
- Rögzítés és lejátszás.

**E. Broadcast Metadata Engine**

- ANC/VANC.
- SMPTE ST 291.
- Feliratok és caption adatok.
- AFD.
- ST 2038.
- Egyéb támogatott broadcast-metaadatok.

**F. Synchronization Engine**

- PTP.
- Genlock.
- SMPTE ST 2059.
- Timecode.
- Többeszközös szinkronizáció.

**G. IP Broadcast és vezérlési réteg**

- SMPTE ST 2110.
- NMOS.
- TSL UMD.
- Tally.
- OSC és MIDI adapterek.
- Többkimenetes vezérlés.

**H. Qt6 felhasználói felület**

- Eszközök listája.
- Bemeneti és kimeneti konfiguráció.
- Jelállapot és szinkronizáció.
- Rögzítési profilok.
- HDR és színbeállítások.
- Broadcast-vezérlés.
- Diagnosztika és naplózás.

### 3.2. A fő adatút

A tervezett rögzítési útvonal:

DeckLink hardver
→ Blackmagic SDK
→ C++ FFI
→ Rust DeckLink Backend
→ CFA4 közös Media Pipeline
→ FFmpeg-alapú kódolási útvonal
→ CFA4 rögzítési rendszer
→ PostgreSQL-metaadatok és projektkapcsolatok.

A kimeneti útvonal ennek fordított irányában működik.

Az ANC/VANC, HDR, timecode és egyéb metaadatok nem veszhetnek el észrevétlenül a pipeline egyetlen szakaszában sem. Ha egy célformátum nem képes valamely metaadat megőrzésére, ezt a rendszernek jeleznie kell.

### 3.3. Modulhatárok

A DeckLink Backend nem kezelheti önállóan a CFA4 teljes rögzítési folyamatát.

A hardveres backend felelőssége:

- hardverkapcsolat;
- eszközállapot;
- frame- és audiobemenet;
- kimeneti adatátadás;
- időzítés és eszközspecifikus metaadatok.

A közös pipeline felelőssége:

- pufferelés;
- kódolás;
- fájlkezelés;
- projektintegráció;
- médiametaadatok;
- rögzítési munkafolyamat.

Ez a határ biztosítja, hogy később más gyártók hardverei is ugyanahhoz a CFA4-rendszerhez csatlakozhassanak.

## 4. A 26 donor és referenciaprojekt felhasználási terve

A donorokat nem egységesen kell átvenni. Minden forrásnál külön kell eldönteni, hogy teljes komponens, részleges kód, algoritmus, protokollreferencia vagy kizárólag dokumentáció használható-e.

### 4.1. DeckLink és hardveres I/O donorok

| # | Donor | Tervezett szerep | Állapot és korlátozás |
|---|---|---|---|
| 1 | Hivatalos Blackmagic SDK – https://www.blackmagicdesign.com/developer | Elsődleges hardveres API | Kötelező alap. Aktuális SDK-verzió és licenc ellenőrzendő |
| 2 | soramimi/DeckLinkCapture – https://github.com/soramimi/DeckLinkCapture | DeckLink videó- és hangbemeneti folyamat, callbackek, eszközfelderítés | Részletesen vizsgált donor. Qt5 GUI és saját encoder nem vehető át automatikusan |
| 3 | Julusian/decklink-rs – https://github.com/Julusian/decklink-rs | Rust SDK-wrapper, eszközkezelés, frame- és memóriakezelési minták | Részletesen vizsgált, de hiányos. Input capture nincs igazolva |
| 4 | hhool/Blackmagic_DeckLink_SDK – https://github.com/hhool/Blackmagic_DeckLink_SDK | Régi SDK- és API-referencia | Részletesen vizsgált. Régi SDK-verzió, nem használható aktuális alapként |
| 5 | manoreken2/projects – https://github.com/manoreken2/projects | HDR10, PQ, r210/v210, DNG és EXR minták | Részletesen vizsgált. Windows-központú részek kizárandók |
| 6 | bmdtools – https://github.com/lu-zero/bmdtools | DeckLink és GStreamer jellegű integrációs minták | Újraellenőrzendő repo, branch, licenc és működési állapot |
| 7 | GStreamer DeckLink plugin – https://gitlab.freedesktop.org/gstreamer/gstreamer | DeckLink–GStreamer interoperabilitás, formátum- és eszközkezelési referenciák | Célzott vizsgálat szükséges; nem helyettesíti a CFA4 fő pipeline-ját |
| 8 | blackmagic-io – https://github.com/nick-shaw/blackmagic-io | DeckLink I/O integrációs minták | Pontos repoazonosság, tartalom és licenc újraellenőrzendő |
| 9 | OBS DeckLink plugin – https://github.com/obsproject/obs-studio/tree/master/plugins/decklink | Eszközkezelés, bemeneti és kimeneti folyamat, hibakezelési minták | Referenciaként vizsgálandó; OBS-architektúra nem emelendő át |
| 10 | pydecklink – https://github.com/Fuse-Technical-Group/pydecklink | Python-alapú SDK-hozzáférés és prototípusminták | Repoazonosság, licenc, SDK-kompatibilitás ellenőrzendő |

### 4.2. ST 2110 és IP-broadcast donorok

| # | Donor | Tervezett szerep | Állapot és korlátozás |
|---|---|---|---|
| 11 | pi-nmos-st2110 – https://github.com/dgp1000/pi-nmos-st2110 | ST 2110 és NMOS jellegű IP-broadcast referencia | Repoállapot, protokolllefedettség és licenc ellenőrzendő |
| 12 | Deckboy – https://github.com/Utopian-Academy/Deckboy | DeckLink/broadcast munkafolyamatok vizsgálata | Pontos funkciók és licenc nem tekinthető igazoltnak |
| 13 | Frame Ferret – https://github.com/stoatworks-labs/frame-ferret | Frame- és médiavizsgálati minták | Repoazonosság, aktuális működés és licenc ellenőrzendő |
| 14 | libklvanc – https://github.com/stoth68000/libklvanc | KLV és ANC/VANC feldolgozás | Licenc- és linkelési feltételek ellenőrzendők |
| 15 | rust-broadcast – https://github.com/fishloa/rust-broadcast | Rust-alapú broadcast komponensek és protokollminták | Pontos modulok, érettség és licenc ellenőrzendő |
| 16 | EBU LIST – https://github.com/ebu/pi-list | EBU broadcast- és protokollreferencia | A ténylegesen hasznosítható modulokat és licencet külön kell vizsgálni |

### 4.3. Szinkronizációs és hálózati donorok

| # | Donor | Tervezett szerep | Állapot és korlátozás |
|---|---|---|---|
| 17 | statime – https://github.com/pendulum-project/statime | Rust-alapú PTP megoldás és időszinkronizáció | Aktuális protokolltámogatás és licenc ellenőrzendő |
| 18 | linuxptp – https://github.com/richardcochran/linuxptp | Linux PTP infrastruktúra, diagnosztika, referencia | Rendszerkomponensként vagy külső szolgáltatásként vizsgálandó |
| 19 | sony/nmos-cpp – https://github.com/sony/nmos-cpp | NMOS protokollok és eszközkapcsolatok | A pontos IS-04/05/07/08 támogatás és licenc ellenőrzendő |
| 20 | AMWA nmos-testing – https://github.com/AMWA-TV/nmos-testing | NMOS megfelelőségi tesztek | Tesztelési donor, nem futásidejű CFA4-függőség |

### 4.4. Broadcast vezérlési és tesztelési donorok

| # | Donor | Tervezett szerep | Állapot és korlátozás |
|---|---|---|---|
| 21 | tslumd – https://github.com/nocarryr/tslumd | TSL UMD protokoll | Protokollverziók és licenc ellenőrzendő |
| 22 | tsl-umd-protocol – https://github.com/bitaffair/tsl-umd-protocol | TSL UMD protokollreferencia | Repoazonosság és licenc ellenőrzendő |
| 23 | TSL-UMD-Tester – https://github.com/videojedi/TSL-UMD-Tester | TSL UMD interoperabilitási tesztelés | Tesztelőeszközként vizsgálandó |
| 24 | SDPoker – https://github.com/Streampunk/sdpoker | SDP- és ST 2110 jellegű hálózati tesztelés | Pontos képesség és protokolllefedettség ellenőrzendő |
| 25 | smpte2110-tester – https://github.com/necat101/smpte2110-tester | ST 2110 tesztelés | Tesztelési érték és licenc ellenőrzendő |
| 26 | klvanc-tools – https://github.com/stoth68000/klvanc-tools | KLV/ANC/VANC tesztelési és diagnosztikai eszközök | A libklvanc kapcsolataival együtt vizsgálandó |

### 4.5. Donorfelhasználási szabály

A donorok négy kategóriába kerülnek:

1. **Elsődleges technológiai alap:** hivatalos Blackmagic SDK.
2. **Kód- vagy komponensdonor:** csak részletes licenc-, biztonsági és kompatibilitási vizsgálat után.
3. **Algoritmus- és protokollreferencia:** a CFA4 saját architektúrájához igazítva.
4. **Tesztelési donor:** tesztvektorok, teszteszközök, protokollmegfelelőség.

A korábban részletesen megvizsgált DeckLinkCapture, decklink-rs, hhool SDK-másolat és manoreken2/projects sem tekinthető automatikusan kész CFA4-komponensnek.

A többi donor esetében a repository pontos állapotának ellenőrzése a F0 kötelező feladata.

## 5. Funkcionális modulok

### 5.1. DeckLink Device Manager

Feladatok:

- Eszközfelderítés.
- Modell- és eszközazonosítás.
- Bemeneti és kimeneti képességek lekérdezése.
- Csatlakozók és jelutak megjelenítése.
- Eszközállapotok figyelése.
- Eszközvesztés és újracsatlakozás kezelése.
- Több DeckLink eszköz kezelése.

A hardver által nem támogatott funkciókat a felület nem jelenítheti meg elérhetőként.

### 5.2. DeckLink Capture Backend

Feladatok:

- SDI/HDMI videobemenet.
- Hangbemenet.
- Videomódok kiválasztása.
- Pixelformátumok kiválasztása.
- Frame callbackek.
- Időbélyeg-kezelés.
- Timecode beolvasása.
- Audio/video szinkronizáció.
- Frame drop és túlterhelés felismerése.

A capture backend közvetlenül a CFA4 közös Media Pipeline-nak adja át az adatokat.

### 5.3. DeckLink Playback Backend

Feladatok:

- Videó- és hangkimenet.
- Scheduled playback.
- Frame-időzítés.
- Timecode-kimenet.
- Kimeneti módok kezelése.
- Többkimenetes működés előkészítése.

### 5.4. HDR és High Bit Depth Engine

Támogatandó területek:

- 8-bit, 10-bit és 12-bit formátumok.
- YUV422 és RGB formátumok.
- r210 és v210 jellegű formátumok.
- PQ/HDR10.
- HDR-metaadatok.
- Színprimerek, transfer function és matrix információk.
- OpenEXR-alapú köztes feldolgozás.
- DNG-alapú RAW munkafolyamatok vizsgálata.

A formátumtámogatás mindig három külön feltételtől függ:

1. A DeckLink hardver támogatja-e?
2. A Blackmagic SDK támogatja-e?
3. A CFA4 pipeline képes-e az adatot helyesen továbbítani és feldolgozni?

E három feltétel közül bármelyik hiánya esetén a funkció nem jelölhető teljes körűen támogatottnak.

### 5.5. ANC/VANC Engine

Tervezett képességek:

- SMPTE ST 291.
- ANC adatcsomagok kezelése.
- VANC adatok feldolgozása.
- Caption/feliratadatok.
- AFD.
- ST 2038.
- Egyéb, később validált broadcast-metaadatok.

A metaadatok tárolását, továbbítását és exportját külön ellenőrizni kell. A videó rögzítése önmagában nem bizonyítja az ANC/VANC adatok megőrzését.

### 5.6. Time Synchronization Engine

Feladatok:

- PTP.
- SMPTE ST 2059.
- Genlock állapotok.
- Timecode.
- Eszközök közötti szinkronizáció.
- Szinkronvesztés felismerése.
- Időeltérések naplózása.

A PTP és Genlock nem tekinthető egymás helyettesítőjének. A rendszernek külön kell kezelnie a hálózati időszinkronizációt és a hardveres videojel-szinkronizációt.

### 5.7. ST 2110 IP-video Backend

Tervezett képességek:

- ST 2110 video- és audiofolyamok.
- RTP-alapú adatátvitel.
- SDP leírások.
- Multicast.
- Hálózati interfészek kiválasztása.
- Hálózati késleltetés és csomagvesztés monitorozása.
- Redundáns hálózati útvonalak vizsgálata.

Az ST 2110 bevezetése külön fejlesztési szakasz. Nem feltételezhető, hogy a DeckLink capture megléte automatikusan IP-broadcast képességet is biztosít.

### 5.8. NMOS Integration

Tervezett képességek:

- IS-04 eszközfelderítés és regisztráció.
- IS-05 kapcsolatkezelés.
- IS-07 események.
- IS-08 audio-csatorna mapping, ha a célprofil és az alkalmazási igény ezt indokolja.
- Kapcsolati állapotok.
- Eszköz- és node-metaadatok.
- Külső NMOS rendszerekkel való együttműködés.

Minden NMOS-profil támogatását külön megfelelőségi teszttel kell igazolni.

### 5.9. Broadcast Control Engine

Tervezett képességek:

- TSL UMD.
- Tally állapotok.
- Külső vezérlőrendszerek.
- OSC adapter.
- MIDI adapter.
- Eszköz- és csatornaállapotok.
- Qt6 felületi visszajelzés.

A CFA4 meglévő Control Surface architektúrájához kell illeszkednie. Nem hozhat létre attól független, párhuzamos MIDI/HID/OSC vezérlőmotort.

### 5.10. Key/Fill és többkimenetes rendszer

Későbbi funkciók:

- Key/Fill kimenet.
- Két vagy több szinkronizált kimenet.
- Különálló program- és preview-kimenet.
- Cue-alapú playout.
- Külső broadcast vezérlés.

Ezek csak az alapvető DeckLink playback, időzítés és szinkronizáció validálása után kerülhetnek fejlesztésre.

## 6. Fejlesztési fázisok

A fejlesztés F0–F8 szakaszokra tagolódik. A fázisok egymásra épülnek; a következő szakasz csak az előző elfogadási feltételeinek teljesülése után indítható.

### F0 – Előkészítés és donorvalidáció

**Cél:** a CFA4 jelenlegi állapotának, a donoroknak és a technikai függőségeknek az ellenőrzése.

Feladatok:

1. A CFA4 Recording terveinek és aktuális állapotának áttekintése.
2. A meglévő codec- és Media Pipeline áttekintése.
3. A Qt6 GUI készültségének ellenőrzése.
4. Mind a 26 donor pontos repository-azonosságának ellenőrzése.
5. Branch, commit, release és karbantartási állapot felmérése.
6. Licenc- és függőségi audit.
7. Blackmagic SDK aktuális verziójának és licencének ellenőrzése.
8. Linux/amd64 kompatibilitás ellenőrzése.
9. A hardveres tesztkörnyezet meghatározása.
10. A CFA4 Media Pipeline és a DeckLink Backend API-határainak meghatározása.

**Elfogadási feltételek:**

- Mind a 26 donor státusza dokumentált.
- Minden kódátvételre jelölt komponens licencállapota tisztázott.
- A Blackmagic SDK használati feltételei dokumentáltak.
- A CFA4 Recording és codec előfeltételek állapota igazolt.
- Az architektúra nem hoz létre párhuzamos rögzítési motort.

### F1 – DeckLink Core

**Cél:** stabil hardverfelismerés, videó- és hangbemenet, valamint alapszintű kimenet.

Feladatok:

- SDK-integráció.
- Rust–C++ FFI.
- Eszközfelderítés.
- Bemeneti és kimeneti módok.
- SDI/HDMI capture.
- Audio capture.
- Frame callbackek.
- Bufferkezelés.
- Timecode.
- Alapszintű playback.

**Elfogadási feltételek:**

- Valós DeckLink hardver felismerése.
- Stabil videó- és hangbemenet.
- Helyes frame-mód és pixelformátum.
- Helyes időbélyegek.
- Ellenőrzött A/V szinkronizáció.
- Frame drop és megszakadás felismerése.
- Stabil kimeneti működés.
- Hosszabb rögzítés alatt nem jelentkezhet megmagyarázatlan adatvesztés.

### F2 – HDR és nagy bitmélység

**Cél:** professzionális képformátumok és HDR-adatok helyes kezelése.

Feladatok:

- 10/12 bites YUV és RGB.
- r210/v210.
- PQ/HDR10.
- HDR-metaadatok.
- OpenEXR.
- DNG-feldolgozási minták vizsgálata.
- Színinformációk megőrzése.
- Képminőség-ellenőrzés.

**Elfogadási feltételek:**

- A bemeneti és kimeneti bitmélység igazolt.
- Nincs észrevétlen bitmélység-csökkenés.
- A színinformációk helyesen kerülnek átadásra.
- A HDR-metaadatok megőrzése vagy elvesztése dokumentált.
- A képkockák vizuális és numerikus összehasonlítása sikeres.
- A DNG/EXR útvonalak külön validáltak.

### F3 – ANC/VANC és broadcast-metaadatok

**Cél:** a professzionális broadcast-metaadatok feldolgozása.

Feladatok:

- ST 291.
- ANC/VANC.
- Caption adatok.
- AFD.
- ST 2038.
- KLV/ANC segédkomponensek.
- Metaadatok továbbítása a CFA4 pipeline-ban.

**Elfogadási feltételek:**

- Ismert tesztadatok helyes beolvasása.
- Metaadatok megőrzése.
- Sérült vagy hibás csomagok felismerése.
- Export és visszaolvasás ellenőrzése.
- Külön tesztelt caption és AFD útvonal.

### F4 – PTP és Genlock

**Cél:** professzionális időszinkronizáció és többeszközös működés.

Feladatok:

- linuxptp/statime vizsgálat.
- PTP állapotkezelés.
- Genlock.
- SMPTE ST 2059.
- Timecode.
- Szinkronizációs hibák felismerése.
- Több eszköz közötti időeltérés mérése.

**Elfogadási feltételek:**

- PTP állapot és időforrás látható.
- Genlock állapot ellenőrizhető.
- Szinkronvesztés felismerhető.
- Többeszközös teszt dokumentált.
- Időeltérések mérhetőek és naplózottak.

### F5 – SMPTE ST 2110 IP-video

**Cél:** hálózati broadcast I/O bevezetése.

Feladatok:

- ST 2110 video- és audiofolyamok.
- RTP.
- SDP.
- Multicast.
- Hálózati interfészek kezelése.
- Csomagvesztés és késleltetés monitorozása.
- Redundáns útvonalak vizsgálata.

**Elfogadási feltételek:**

- Külső ST 2110 eszközzel igazolt kapcsolat.
- Video- és audiofolyamok helyes feldolgozása.
- Hálózati hiba felismerése.
- Csomagvesztés és késleltetés mérése.
- Hosszabb futási teszt.

### F6 – NMOS Integration

**Cél:** professzionális IP-broadcast eszközfelderítés és kapcsolatkezelés.

Feladatok:

- IS-04.
- IS-05.
- IS-07.
- Indokolt esetben IS-08.
- NMOS node és device modellek.
- Registry kapcsolat.
- Kapcsolati állapotok.
- Megfelelőségi tesztek.

**Elfogadási feltételek:**

- Külső NMOS rendszerrel sikeres együttműködés.
- A támogatott profilok külön teszteltek.
- Kapcsolatfelépítés és bontás helyes.
- Hibaállapotok kezelése.
- AMWA tesztelési eredmények dokumentálása.

### F7 – Broadcast Control, TSL UMD és Key/Fill

**Cél:** külső broadcast vezérlés és többkimenetes munkafolyamatok.

Feladatok:

- TSL UMD.
- Tally.
- OSC/MIDI adapterek.
- Külső vezérlőkapcsolatok.
- Key/Fill.
- Többkimenetes playout.
- Cue-alapú vezérlés.

**Elfogadási feltételek:**

- TSL UMD külső eszközzel tesztelt.
- Tally állapotok helyesen jelennek meg.
- OSC/MIDI adapterek a CFA4 közös vezérlőmotorját használják.
- Key/Fill szinkronizáció igazolt.
- Többkimenetes működés tesztelt.

### F8 – Rendszerszintű validáció és csomagolás

**Cél:** a teljes rendszer integrációs és telepítési ellenőrzése.

Feladatok:

- Teljes rendszerintegráció.
- Hosszú futási tesztek.
- Több DeckLink eszköz tesztelése.
- Hálózati broadcast tesztek.
- Metaadat- és HDR-megőrzés ellenőrzése.
- Hibakezelés.
- Naplózás és diagnosztika.
- Csomagolás.
- Együttélési tesztek.

**Elfogadási feltételek:**

- Ubuntu 26.04 amd64 rendszeren igazolt működés.
- Qt6 felület és Rust backend integrációja.
- Helyes A/V szinkronizáció.
- Nem maradhat megmagyarázatlan frame- vagy audiovesztés.
- A fájlok szabályosan lezárhatók és újranyithatók.
- Legalább kétórás folyamatos hardveres teszt.
- A támogatott HDR- és metaadat-útvonalak külön ellenőrzöttek.
- Telepítési együtt… *(a beillesztett szöveg itt megszakad)*
