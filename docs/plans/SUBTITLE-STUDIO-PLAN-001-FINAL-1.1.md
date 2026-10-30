# CFA4-SUBTITLE-STUDIO-PLAN-001

## Professzionális feliratozó munkakörnyezet — végleges, jóváhagyott terv

| Dokumentumadat | Érték |
|---|---|
| Verzió | FINAL 1.1 |
| Dátum | 2026. október 30. |
| Projekt | CFA4 — CinemaFundArchitecture4 |
| Jóváhagyó | Horváth H. István |
| Állapot | A beszélgetésben jóváhagyott, végleges tervezési alap |
| Előzmény | A jóváhagyott 1.1-es pontosított terv átadható kiadása |
| Végrehajtás | A fejlesztés nem indult el; a terv jóváhagyása és a fejlesztés indítása külön döntés |

## 1. Cél és bizonyítottság

A cél a CFA4 részeként olyan professzionális feliratozó munkakörnyezet létrehozása, amely kézi szerkesztésben, időzítésben, többnyelvű feldolgozásban és ellenőrizhető exportban meghaladja a Kdenlive feliratozóját.

A teljes beszélgetés alapján a fő irány saját CFA4-szerkesztőfelület és célzottan kiválasztott Rust-komponensek összeállítása. A kutatásban talált projektek jelöltek vagy funkcionális referenciák, nem automatikusan elfogadott függőségek.

A pontosítás kiemelt követelményei:

- a feliratfájl és a CFA4-projekt adatmodelljének megkülönböztetése;
- a forrásidő, projektidő és képkockaidő egységes kezelése;
- az NLE-vágások és sebességváltoztatások feliratokra gyakorolt hatásának szabályozása;
- a képkockapontosság és az adatmegőrzés mérhető elfogadási feltételei;
- az automatikus eredmények ellenőrizhető, visszavonható elfogadása;
- az OxiMedia megbízhatósági auditjának érvényesítése.

Ez a dokumentum a beszélgetés és a felhasználó által megadott OxiMedia-audit alapján készült. A CFA4 aktuális forráskódjának és a jelöltek teljes implementációjának ellenőrzése nem történt meg e terv véglegesítése során. A modulok és kapcsolódási pontok tervezett felelősségeket jelölnek, nem már létező CFA4-komponenseket.

Az OxiMedia-audit statikus forrásvizsgálatot ismertetett; annak build- és tesztállításai nem saját futtatási eredmények. A dokumentum nem állítja, hogy a jelöltek buildje, tesztje vagy integrációja igazolt lenne.

## 2. Rögzített keretek

| Terület | Követelmény |
|---|---|
| Projekt | Minden fejlesztés a CFA4-be kerül; FilmCraft upstream módosítása nélkül |
| Platform | Ubuntu 26.04 LTS és variánsai, amd64 |
| Felület | A CFA4 natív Qt6-felületébe illeszkedő feliratozó munkaterület |
| Feldolgozás | Rust központi motor; indokolt, ellenőrzött natív háttérkönyvtárak megengedettek |
| Média | A CFA4 meglévő média-, kodek- és exportútvonalainak használata |
| FFmpeg | A CFA4-ben rögzített 9.0.2-es minimumhoz és validációhoz igazodás |
| AI | Teljes kézi munkafolyamat és CPU-s feldolgozási út; opcionális GPU |
| Telepítés | A CFA4 csomagolásába illesztés, ütközésmentes működés a host alkalmazásaival |
| OxiMedia | Kísérleti jelölt, az alapműködés nem függhet tőle |
| Külső projektek | Forrás-, licenc-, függőség- és működésvizsgálat után választhatók ki |

A korábban közölt, körülbelül 53%-os Qt6-készültség 2026. október 28-i állapotközlés volt. Az ütemezést a tényleges integrációs kapukhoz kell kötni, nem ehhez a százalékhoz.

## 3. Megközelítés és a termék helye a CFA4-ben

Az elfogadott irány önálló CFA4 feliratozó munkakörnyezet, meglévő, ellenőrzött komponensekkel. A vizsgált alternatívák a Kdenlive feliratszerkesztőjének bővítése, illetve kész Rust-projekt átvétele voltak. A saját munkakörnyezet biztosítja a CFA4 adatmodelljéhez, idővonalához, Qt6-felületéhez és kodekfolyamataihoz illeszkedést; ennek ára a saját szerkesztői és kompatibilitási fejlesztés.

A **Feliratozás munkaterület** három belépési ponttal működjön:

1. **Projektfelirat:** a teljes CFA4-szekvencia feliratozása.
2. **Klipfelirat:** egy forrásklip átiratának és időzítésének elkészítése, majd felhasználása a projektben.
3. **Feliratfájl szerkesztése:** külső felirat megnyitása videóval vagy videó nélkül.

Mindhárom ugyanazt a motort használja. Önálló, külön telepíthető alkalmazás készítése további termékdöntés; nem szükséges a CFA4-be épülő változat elkészítéséhez.

## 4. Teljes képességcél

| Képességcsoport | Tervezett tartalom |
|---|---|
| Kézi szerkesztés | Táblázat, szövegmező, gyorsbillentyűk, keresés/csere, többes kijelölés, felosztás és összevonás |
| Időzítés | Hullámforma, spektrogram, elő-/utólejátszás, ismétlés, sebességállítás, többpontos szinkron |
| Több sáv | Nyelvi változatok, SDH, forced narrative, alternatív feliratok, rétegek |
| Stílus | ASS-stíluskezelés, helyzet, margó, körvonal, árnyék, forgatás, méretezés |
| Typesetting | Képernyőn végzett elhelyezés, mozgás, vágómaszk és vektoros alakzat; későbbi bővítésben követés |
| Karaoke | Szó- és szótagidőzítés, kézi javítás, kiemelési előnézet |
| Beszédfeldolgozás | Helyi ASR, beszélőszétválasztás, javított szöveg újraillesztése |
| Fordítás | Eredeti és célnyelv egymás mellett, terminológia, fordítási memória, lektorálás |
| OCR | PGS/VobSub kinyerése, felismerés, bizonytalan karakterek javítása |
| Minőségellenőrzés | Olvasási sebesség, időzítés, sortörés, átfedés, pozíció és exportálhatóság |
| Export | Külső fájl, kapcsolható feliratsáv, beégetés, kötegelt feldolgozás |
| Munkabiztonság | Undo/redo, automatikus mentés, helyreállítás, revíziók és változás-összehasonlítás |

A képernyőre égetett felirat videóképből történő felismerése külön OCR-feladat a PGS/VobSub feldolgozásához képest. Ezt későbbi bővítésként kell kezelni.

Fogalmak: az **ASR** automatikus beszédfelismerés; a **forced alignment** ismert, javított átirat hanghoz illesztése; az **SDH** siket és nagyothalló nézők számára készített, beszélőket és releváns hangokat is jelölő felirat; a **forced narrative** a történet megértéséhez szükséges, kiválasztott idegen nyelvű megszólalásokat vagy képi szövegeket fordító felirat.

## 5. Közös adatmodell

A CFA4-projekt saját feliratdokumentumot tároljon. Az SRT, ASS vagy TTML ennek import- és exportformátuma.

A dokumentum tartalmazza:

- a sávok nyelvét, szerepét és stabil azonosítóját;
- az egyes feliratok szövegét, időzítését, beszélőjét, stílusát és rétegét;
- az opcionális szó-/szótagidőzítést és felismerési bizonytalanságot;
- a forráskliphez vagy projekthez tartozó időkapcsolatot;
- a fordítási és lektorálási állapotot;
- az eredeti formátum megőrzendő mezőit;
- a változástörténetet és az alkalmazott automatikus műveleteket.

**Az időértékek tárolása egész alapú vagy racionális legyen, ne lebegőpontos másodperc.** A forrásidő, projektidő és képkockaszám közötti átváltás központi szolgáltatás feladata.

Az ASS képi rétege, a külön nyelvi sáv és a CEA-szolgáltatás külön fogalom maradjon. Ezek összevonása egy egyszerű „sáv” mezőbe később adatvesztést okozhat.

## 6. Import, export és adatmegőrzés

| Eset | Elvárt viselkedés |
|---|---|
| Megnyitás és változatlan visszamentés | A támogatott formátumnál az eredeti szerkezet megőrzése; ahol lehetséges, bájtszintű azonosság |
| Szerkesztés és mentés azonos formátumba | A módosított tartalom és a nem érintett stílus-/metaadatmezők megőrzése |
| Másik formátumba exportálás | Előzetes veszteségjelentés a célformátumban nem ábrázolható tulajdonságokról |

ASS → SRT esetén például a mozgás vagy vektoros rajz nem őrizhető meg. A felület ezt export előtt jelezze, és kínáljon megfelelő alternatívát.

Az ismeretlen mezők megőrzéséhez formátumspecifikus kiegészítő adattárolás szükséges. Egy közös, egyszerű szöveg/időzítés modell önmagában nem elegendő.

## 7. Együttműködés a CFA4-idővonallal

| Videós művelet | Feliratozási szabály |
|---|---|
| Klip mozgatása | A kliphez kötött felirat kövesse a klipet |
| Klip vágása | A felirat a látható klipszakaszhoz igazodjon; a szöveghatár javítható legyen |
| Klip felosztása | A feliratkapcsolat és a forrásidő maradjon követhető |
| Ripple szerkesztés | A kiválasztott feliratsávok kövessék a projekt eltolását |
| Sebességváltoztatás | A forrásidőből új projektidő számítódjon |
| Fordított lejátszás | A rendszer jelezze, hogy az automatikus beszédfelirat értelmezése külön döntést igényel |
| Klip törlése | Ne maradjon észrevétlenül hibásan kapcsolt felirat |
| Fordítási sáv módosítása | Az időkapcsolat megőrzése mellett a célnyelvi tördelés külön szerkeszthető legyen |

A felhasználó választhasson **kliphez kötött** és **projektidőhöz rögzített** felirat között. Az újraillesztés és automatikus újragenerálás előnézettel, visszavonható műveletként történjen.

## 8. Komponensek státusza és vizsgálati szerepe

| Projekt/család | Tervezett szerep |
|---|---|
| subtitler, subparse, subtp | Parser/író összehasonlítás; a legmegfelelőbb kiválasztása formátumonként |
| ass-editor / ass-rs | ASS-adatkezelés és szerkesztési API vizsgálata |
| OxideAV ASS/subtitle | Alternatív parser és író; adatmegőrzésének külön ellenőrzése |
| libass | Elsődleges ASS-megjelenítési referencia és lehetséges üzemi renderelő |
| rassa, Rust ASS-renderelők | Későbbi összehasonlítás; csak igazolt kompatibilitással |
| Samaku | Nem romboló typesetting és csomópontgráf funkcionális referencia |
| AutoSubs, Subtly, Whisper Desktop, SubPilot | Átirat-, modellkezelési és szerkesztői munkafolyamatok vizsgálata |
| OpenSubs | Sorbontás, olvashatósági ellenőrzés és közös motor mintája; licencvizsgálattal |
| SubForge | Automatizálás, fordítási memória és minőségértékelés mintája |
| sherpa-onnx, Whisper-hátterek, wavo | ASR- és beszédfeldolgozó adapterjelöltek |
| valle-asr, wav2vec2-rs | Javított átirat szóillesztése |
| subtile-ocr, Tesseract | Képalapú feliratok felismerésének jelölt útvonala |
| libbitsub | Bitmap-felirat megjelenítési referencia; natív integrálhatósága külön vizsgálandó |
| psyche-subtitle-toolkit | Fordítás, tagmegőrzés és konténerkezelés mintája |
| GStreamer Rust caption pluginek | Későbbi broadcast-útvonal |
| OxiMedia | Elkülönített kutatási jelölt a megadott audit korlátaival |

Az azonos projektcsaládhoz tartozó vagy egymást átfedő crate-eket az audit során össze kell vezetni. A cél kevés, világos felelősségű függőség.

A Kdenlive, Subtitle Composer, Subtitle Edit és Aegisub munkafolyamatai összehasonlítási referenciák. A referencia szerepe nem jelent automatikus kódátvételt.

### 8.1. OxiMedia-korlátozás

A felhasználó által rendelkezésre bocsátott statikus audit szerint az OxiMedia általános éles használatra vonatkozó állításai és részletes dokumentációja között lényeges eltérések vannak. Emiatt:

- nem lehet a CFA4 feliratozó rendszerének kötelező alapja;
- nem válthatja ki automatikusan a CFA4 meglévő média-/kodekútvonalát;
- részfunkció csak az adott implementáció, referenciaeredmények és függőségek külön ellenőrzése után választható;
- a README funkciólistája vagy a tesztek száma nem elegendő elfogadási bizonyíték.

## 9. AI, fordítás és OCR működési szabályai

Az ASR, a szóillesztés és a fordítás külön munkafolyamat legyen. A felhasználó javíthassa az átiratot, majd csak az időzítést számíthassa újra.

Minden automatikus eredményhez tartozzon:

- motor- és modellazonosító;
- feldolgozási beállítás;
- eredeti média- vagy feliratrevízió;
- ellenőrizendő szakaszok jelölése;
- elfogadható vagy elvethető módosításlista.

A kézzel javított vagy lezárt szakaszokat az újrafuttatás ne írja felül észrevétlenül. A fordítás terminológiája és memóriája legyen projekthez kötött, szerkeszthető és exportálható.

A modellek letöltése külön modulkezelőből történjen, azonosítható verzióval és licenccel. A külső fordítási szolgáltatás használatát a felhasználó választja ki; a helyi szerkesztéshez hálózat nem szükséges.

Az opcionális GPU-feldolgozás a CFA4 meglévő erőforrás-kezelési szabályaihoz és igazolt hardverképességeihez igazodjon. A teljes kézi és CPU-s működés önállóan használható maradjon.

## 10. Minőségellenőrzés

A felhasználó által beállítható QC-profil ellenőrizze az olvasási sebességet, sorhosszt, sorszámot, időtartamot, átfedést, szünetet, időzítési hibákat, feliratpozíciót és az exportálhatóságot.

A hibákhoz a felhasználó közvetlenül navigálhasson. Az automatikus javítások legyenek előnézhetők, visszavonhatók és követhetők. A profilhatárértékeket a célfelhasználáshoz kell kötni; egyetlen univerzális sorkorlát vagy CPS-érték nem tekinthető minden célra megfelelőnek.

A szabványprofil neve önmagában nem igazolja a szabványmegfelelést. A támogatási kör, az ellenőrizhető szabályok és az emberi ellenőrzést igénylő feltételek legyenek dokumentáltak.

## 11. Formátumok bevezetési sorrendje

| Szint | Formátumok és feladat |
|---|---|
| A — szerkesztői alap | SRT, WebVTT, ASS/SSA; Unicode és karakterkódolások |
| B — professzionális fájlcsere | TTML/IMSC, EBU STL, iTT; profilonként meghatározott támogatás |
| C — képalapú felirat | PGS, VobSub; kinyerés, előnézet, OCR és javítás |
| D — broadcast | SCC/MCC, CEA-608/708, szükség szerint Teletext/DVB |
| E — külön vizsgálandó bővítés | D-Cinema/IMF feliratozás, ARIB és további ritkább formátumok |

A parser megléte nem jelent automatikusan szabványos exportot. Minden irányhoz külön támogatási mátrix szükséges: **olvasás, szerkesztés, írás, megjelenítés, ismert veszteségek**.

## 12. Fejlesztési sorrend és kapuk

| Fázis | Eredmény | Belépési feltétel |
|---|---|---|
| F0 — CFA4- és komponensaudit | Aktuális kódállapot, meglévő feliratfunkciók, interfészek, licencek és jelöltek felmérése | Külön fejlesztési jóváhagyás után |
| F1 — Adatmodell és szerződések | Sávmodell, időkezelés, revíziók, import/export és Qt6-kapcsolat specifikációja | F0 |
| F2 — Rust szerkesztőmotor | Fájlműveletek, szerkesztési parancsok, undo/redo, mentés/helyreállítás | F1 |
| F3 — Qt6 alapfelület | Lista, videóelőnézet, hullámforma, gyorsbillentyűk | Stabil Qt6 shell és lejátszási interfész |
| F4 — NLE-integráció és QC | Vágáskövetés, több sáv, stílusok, minőségi profilok | F2–F3 |
| F5 — AI és lokalizáció | ASR, szóillesztés, fordítás, OCR, lektorálás | Stabil adatmodell és feladatkezelés |
| F6 — Typesetting és haladó funkciók | ASS-vizuális eszközök, karaoke, további formátumok | Igazolt ASS-megjelenítés |
| F7 — Export és konténerek | Feliratsáv, beégetés, exportprofilok | CFA4 média-/kodekútvonal megfelelő kapui |
| F8 — Kiadási validáció | Ubuntu-célkörnyezeten igazolt működés, csomagolás, dokumentáció | Az adott kiadásba vállalt képességek teljesülése |

Az adatmodell és a Rust-motor a Qt6-felület befejezése mellett fejleszthető. A végső GUI-integráció a szükséges Qt6-interfészek stabilitásához kötött. A feliratmotorhoz új videokodek implementálása nem szükséges; az AV2 és más új kodekek saját CFA4-kapuikon keresztül kapcsolódnak később.

Naptári határidő az F0 eredménye és a tényleges fejlesztési kapacitás ismerete után állítható fel.

## 13. Elfogadási feltételek

| Azonosító | Bizonyítandó eredmény |
|---|---|
| AC-01 | A vállalt formátumok támogatási mátrixa és ismert korlátai dokumentáltak |
| AC-02 | Azonos formátumú visszamentés megőrzi a támogatott adatokat; az átalakítási veszteségek előre láthatók |
| AC-03 | Forrásidő, projektidő és képkockaidő kezelése helyes, ismételt műveleteknél nincs felhalmozódó drift |
| AC-04 | A vágás, ripple és sebességmódosítás a kiválasztott feliratkapcsolati szabályt követi |
| AC-05 | Undo/redo és megszakítás utáni helyreállítás nem veszít elfogadott szerkesztést |
| AC-06 | Az ASS-előnézet és export a vállalt tagekre referenciával összevetett |
| AC-07 | A QC-profilt ellenőrző mintafájlok helyes hibákat és időpozíciókat adnak |
| AC-08 | A magyar ASR, időzítés és OCR minőségét külön mérés rögzíti; nincs bemondott minőségi ígéret |
| AC-09 | Az AI újrafuttatása megőrzi a zárolt és kézzel elfogadott tartalmat |
| AC-10 | Külső fájl, muxolt sáv és beégetett export a CFA4 célkörnyezetén visszaellenőrzött |
| AC-11 | CPU-s út működik; az opcionális GPU-képességek külön igazoltak |
| AC-12 | A csomag nem módosítja ütközően az önálló host-alkalmazások telepítését |

A teljesítménycélokat F0-ban kell rögzíteni rövid és többórás anyagra, sok felirateseményre, több sávra és nagy ASS-fájlra. Mérés előtt nem adható megalapozott sebesség- vagy memóriaígéret.

## 14. Kiadási fokozatok

1. **Szerkesztői alap:** kézi szerkesztés, SRT/VTT/ASS, helyes időkezelés, helyreállítás és CFA4-integráció.
2. **Professzionális feliratozó:** többnyelvű munka, részletes QC, fordítás/lektorálás, OCR, szóillesztés és fejlett ASS-kezelés.
3. **Broadcast és speciális bővítések:** igazolt szabványprofilok, kapcsolódó konténerek és ritkább formátumok.

Az első fokozat fejlesztési mérföldkő. A Kdenlive-nél professzionálisabb megoldás célját a második fokozat összehasonlító ellenőrzésével kell igazolni, nem pusztán a funkciólista hosszával.

## 15. Átadási állapot

**Elkészült:** a követelményeket, komponensjelölteket, korlátokat, fejlesztési sorrendet és elfogadási kapukat tartalmazó terv; a felhasználó jóváhagyása a beszélgetésben.

**Végrehajtáskor tisztázandó:** aktuális CFA4-kódállapot, tényleges integrációs pontok, kiválasztott függőségek és verzióik, licencmegfelelés, támogatási mátrix, teljesítménycélok és naptári ütemezés.

**Nem igazolt e dokumentummal:** működő implementáció, lefutott build vagy teszt, teljes szabványmegfelelés, a jelölt projektek éles használhatósága, illetve a dokumentum CFA4-repozitóriumba történő felvétele.

A dokumentum a jóváhagyott FINAL 1.1 tervezési alap átadható példánya. Implementáció nem indult el.
