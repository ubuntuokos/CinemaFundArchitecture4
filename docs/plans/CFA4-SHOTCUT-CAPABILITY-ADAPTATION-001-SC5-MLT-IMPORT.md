# CFA4-SHOTCUT-CAPABILITY-ADAPTATION-001 / SC-5: Shotcut-projekt (`.mlt`) import, tervpótlás

**Állapot:** **tulajdonosi jóváhagyásra vár.** A dokumentum **nem indít el semmit**: a megvalósítás (SC-5.0…SC-5.6) csak a tulajdonos jóváhagyása után kezdődhet (`docs/coordination.md` §2.10). A D1–D6 döntések (9. pont) a tulajdonosé.
**Dátum:** 2026-11-22 · **Alap:** `main` @ `bd84c4d` · **Módszer:** csak olvasás: `VISION.md`, `AGENTS.md`, `docs/coordination.md`, a [`CFA4-SHOTCUT-CAPABILITY-ADAPTATION-001`](CFA4-SHOTCUT-CAPABILITY-ADAPTATION-001.md) F0-terv, a [`CFA4-V1-SILENCE-REMOVAL-001`](CFA4-V1-SILENCE-REMOVAL-001.md) terv (formai minta), a Kdenlive-terv K1/K20/K25/K42 pontjai, a `crates/interchange` fejléce és `Report` típusa. A kódot nem futtattam, a Shotcut forrását és fájlformátum-leírását nem vizsgáltam.
**Nyelv:** magyar.
**Forrás és előzmények:** a tulajdonos 2026-11-22-i észrevétele az F0-tervről: kimaradt, hogy a Shotcut projektfájl importálható legyen a CFA4-be; az **exportálás szándékosan kimarad**, mert a CFA4 funkciói sokkal összetettebbek a Shotcutnál, ezért az export értelmetlen lenne. Az F0-terv F4 sora csak az MLT/Kdenlive-importot és a „nincs MLT-export” döntést említi (2026-11-03, E10/E11); a **Shotcut-projekt importja** nem szerepelt benne, és más terv sem fedi.

---

## 0. Röviden

**Cél:** egy Shotcut-projektfájl (`.mlt`, MLT XML) megnyitása a CFA4-ben úgy, hogy a **sávok, a klipek, a vágópontok (trimmelés), az átmenetek és a gyakori szűrők** képkocka-pontosan és a CFA4 saját megfelelőire leképezve átjöjjenek, ami pedig nem képezhető le, azt az import **névvel és helyével jelentse**, nem csendben dobja el, és soha ne omoljon össze.

**Új tétel: SC-5** (a négy meglévő SC-tétel mellé). Nincs új motor és nincs párhuzamos formátumréteg: az olvasó a `crates/interchange` új moduljaként készül, a meglévő `Imported` + `Report` + `merge_into` úton, egy `file.import` felismeréssel.

**Az export nem cél** (5. pont): a CFA4-projekt nem írható vissza Shotcut-formátumba.

**Fontos átfedés (9. pont, D1):** a Kdenlive-terv **K1/K42/K20/K25** ugyanazt a fájlcsaládot (MLT XML) olvassa. A Shotcut és a Kdenlive ugyanarra az MLT-szerkezetre épül, csak a saját tulajdonságaik és szűrőazonosítóik különböznek. A `main`-en jelenleg **nincs MLT-olvasó** (ellenőrizve: a `crates/interchange` modullistájában nincs, a repóban `.mlt` kezelés nem található). A javaslat: **egy közös MLT-mag, két dialektus**, és az SC-5 a K1-gyel egyeztetve épüljön (`docs/coordination.md` §2.8: a döntés a tulajdonosé és a gazdáé).

| Tétel | Tartalom | Prioritás | Becslés* |
|---|---|---|---|
| **SC-5** | Shotcut `.mlt` import: timeline, klipek, trimmelés, átmenetek, szűrők, jelentés | B | 66–116 ó önállóan; a K1 közös magjával kb. 40–75 ó (lásd 7. pont) |

\* A becslés az enyém, nem mért (a CFA4 „Opus 5.5 ügynökórában” adja, `docs/roadmap.md`).

---

## 1. A CFA4 ma (a kód és a tervek alapján)

| Mi | Hol | Mit tud | Mit nem |
|---|---|---|---|
| Importerek | `crates/interchange/src/{edl,fcp7,fcpxml,otio,aaf,omf,ale}.rs` | Minden importer önálló `Project`-töredéket (`Imported`) ad, amit `merge_into` fűz a nyitott projektbe; `Format` enum, `file.import` felismeri | **MLT XML-t nem olvas** |
| Importjelentés | `Report`, `ReportEntry { level, message, count }` | Azonos üzenetek összevonva számlálóval; `warn`/`info`, `has_warnings` | Nincs elem-szintű hely (sáv, klip) a bejegyzésben; ezt az SC-5 bővíti (4. pont) |
| Idő | `cfa4_time::{Tick, FrameRate}` | Racionális képkockaszám és másodperc kerekítés nélkül minden műsorszóró sebességre | – |
| XML | `crates/interchange/src/xml.rs` | A meglévő XML-olvasó (FCP7, FCPXML) | Hogy az MLT-hez újrahasznosítható-e, az SC-5.0 első lépése |
| Átmenetek, szűrők | `crates/render/src/transitions/`, `crates/render/src/effects.rs`, `sequence.applyVideoTransition`, `effects.apply` | ~30 saját átmenet; Gaussian Blur, Lumetri Color, Fade In/Out, Volume, Panner, Invert stb. | A Shotcut-szűrő → CFA4-effekt tábla nincs |
| Sebesség | `clip.speedDuration` | Sebességváltás, visszafelé játszás | – |
| Kdenlive-import | K1, K20, K25, K42 (Kdenlive-terv, jóváhagyva 2026-11-03; K1 „kötelező”, 2026-10-25) | Terv szerint: MLT XML olvasó, szolgáltatás → parancs tábla, kompatibilitási jelentés | **Még nem készült el**, a `main`-en nincs kód |
| MLT mint motor | Kdenlive-terv E11 (2026-11-03) | A saját Rust-motor az alap; az MLT (`melt`) csak **tesztorákulum** | – |

**Következmény:** az SC-5 nagy része az MLT XML szerkezetének olvasása (profil, producer, playlist, tractor, entry, blank, transition, filter), ami **a K1-gyel azonos**. A Shotcut-specifikus rész a `shotcut:` tulajdonságok, a Shotcut által használt szolgáltatásnevek és szűrőazonosítók táblája, valamint a Shotcut időzítési és sávszerkezeti szokásai.

---

## 2. A követelmények (R1–R14)

| # | Követelmény |
|---|---|
| R1 | A `.mlt` fájl **felismerése** (kiterjesztés és `<mlt>` gyökérelem; Shotcut-eredet a `shotcut:` tulajdonságokból), a `file.import` és a Qt6 Open/Import útvonalon |
| R2 | **Profil:** felbontás, képkockasebesség (racionális), képarány, pixelarány, szkennelés → CFA4-szekvenciabeállítás; ismeretlen vagy nulla érték hibajelentés, nem osztás nullával |
| R3 | **Idővonal:** a videó- és hangsávok sorrendje, nevük, némítás/rejtés/zárolás állapotuk; a sávok közötti pozíciók |
| R4 | **Klipek:** forrásfájl (abszolút és a projekthez viszonyított út, `file://`), pozíció a sávon, hossz; a hiányzó média **offline klipként** jön át (a meglévő relink útvonallal), nem hiba |
| R5 | **Trimmelés:** a be- és kivágási pont (In/Out) **képkocka-pontosan**, a Tick-modellben; az MLT `out` jelentésének (a kivágás utolsó, benne lévő képkockája) helyes kezelése, a `blank` (üres rész) és a sávon belüli sorrend |
| R6 | **Átmenetek:** a Shotcut áttűnései (keverés/`mix`, `luma`, `qtblend`/`affine`/`frei0r` alapú) a CFA4 megfelelő átmenetére vagy átlátszóság/hang-keresztáttűnésre; hossz és igazítás megmarad |
| R7 | **Szűrők/effektek:** a gyakori Shotcut-szűrők (fényerő/kontraszt, színkorrekció, vágás, méret/pozíció/forgatás, elmosás, hangerő, panoráma, fade be/ki, sebesség) leképezése a CFA4 effektjeire, **paraméterekkel és kulcskockákkal** (3. pont) |
| R8 | **Sebesség:** a klip sebességváltása és a visszafelé játszás (`timewarp` és a Shotcut saját jelölése) |
| R9 | **Markerek és szövegek:** ahol a Shotcut ezeket a fájlban tárolja, markerként, illetve CFA4-címként; a nem leképezhető kijelölés/jelölés jelentve |
| R10 | **Nem támogatott elem:** minden elem **besorolva** (támogatott / átalakított / nem támogatott / kihagyott), a jelentés **elem-szintű** (sáv, klip, szűrő neve, pozíció), és az import **nem áll meg** egy ismeretlen elemen |
| R11 | **Egy lépés:** az import egy undo-lépés, az eredmény új szekvencia (nem írja felül a meglévőt); mentés/újratöltés megőrzi |
| R12 | **Nem crash (AGENTS.md §0):** hibás, csonka, mutált vagy ellenséges XML hibát ad, nem pánikot (4. pont) |
| R13 | **Nincs külső folyamat és nincs MLT-függőség** a termékben: az olvasó tiszta Rust; a `melt` csak tesztorákulum (E11) |
| R14 | **Az eredeti fájl érintetlen**; a `.mlt` csak olvasva nyílik, nem íródik vissza (5. pont) |

---

## 3. Az import hatóköre és a leképezés

> **Fontos korlát:** a Shotcut fájlformátumát és szűrőazonosítóit ebben a tervben **nem forrásból, hanem a nyilvános MLT-ismeretből és a Shotcut általános működéséből írom le**. Minden alábbi azonosító és szokás az **SC-5.0 ellenőrzési feladata** valódi, saját magunk által készített Shotcut-projekteken és az MLT XML-dokumentáción (10. pont). Amit az SC-5.0 nem igazol, az a táblában „nem támogatott” marad.

### 3.1 Szerkezet

| Shotcut/MLT elem | CFA4 megfelelő | Besorolás (cél) |
|---|---|---|
| `<profile>` (szélesség, magasság, `frame_rate_num/den`, képarány, `progressive`) | szekvenciabeállítás (`FrameRate`, méret, pixelarány) | támogatott |
| `<producer>` / `<chain>` fájl-erőforrás (`resource`) | médiahivatkozás a Binben; a `shotcut:` metaadat (felirat, ujjlenyomat) a klip megjegyzésébe/metaadatába | támogatott |
| `main_bin` playlist (a Shotcut projekt-lomtára) | Bin-bejegyzések (nem idővonal) | átalakított |
| `<tractor>` a sávokkal (`<track producer=…>`, `hide`) | szekvencia videó- és hangsávokkal | támogatott |
| sáv-playlist `<entry producer in out>` és `<blank length>` | klip a sávon; a `blank` üres rész | támogatott |
| sávnév, `shotcut:video` / `shotcut:audio` jelölés | sávtípus és név | támogatott |
| beágyazott `<tractor>` (a Shotcut „nested” vagy több szintű szerkezete) | beágyazott szekvencia (`clip.nest` modell) mélységkorláttal | átalakított (SC-5.4) |
| generátor producer (`color`, `noise`, `count`, szín/szöveg/hangsáv) | színklip / generátor; a nem leképezhető jelentve | átalakított / nem támogatott |
| képsorozat producer | `file.importImageSequence` modell | átalakított |

### 3.2 Trimmelés és idő (R5)

- Az MLT idő lehet képkockaszám vagy óra-formátum (órák:percek:másodpercek.ezredmásodperc). Az ezredmásodperces forma **kerekítési kockázat**: a CFA4 `Tick`-pontos, ezért az olvasó a profil racionális képkockasebességéből számol, és a kerekítést a jelentésben jelzi, ha az érték nem esik képkockahatárra.
- Az MLT `out` **a kivágás utolsó, benne lévő** képkockája (hossz = `out − in + 1`); az SC-5.0 ezt tényként ellenőrzi a `melt` orákulummal, mert egy kockányi eltérés minden vágáson látszana.
- Tilos az `f64` másodperc (`CLAUDE.md`: pontos idő = `Tick`); minden in/out/hossz `checked_*`/`saturating_*` művelettel számolódik (AGENTS.md §0.4).

### 3.3 Átmenetek (R6)

| Shotcut-átmenet (MLT-szolgáltatás) | CFA4 megfelelő | Besorolás (cél) |
|---|---|---|
| keresztáttűnés videón (`luma`/`mix` alapú) | Cross Dissolve (a `crates/render/src/transitions/dissolve.rs` családja) | támogatott |
| hang-keresztáttűnés (`mix`) | hang-keresztáttűnés | támogatott |
| törlés/„wipe” (`luma` maszkkal) | a legközelebbi CFA4-törlés (`wipe.rs`); a maszkkép nem vihető át | átalakított |
| átlátszóság-keverés (`qtblend`, `frei0r.cairoblend`) a sávok között | sávátlátszóság és keverési mód | átalakított |
| ismeretlen vagy külső (`frei0r.*`, `webvfx`) átmenet | – | nem támogatott: **kemény vágás** a közös ponton, jelentés |

A Shotcut-áttűnés a **sávon belüli átfedésből** adódik (a két klip átfedő szakasza); a CFA4-ben ez átmeneti objektum, tehát az átfedő szakaszt az olvasónak **fel kell ismernie és át kell alakítania**, hogy a klipek hossza és a sorozat hossza ne csússzon el. Ez a leképezés legkockázatosabb pontja (10. pont, 4.).

### 3.4 Szűrők és effektek (R7, R8)

A tábla a **célt** írja le; az azonosítók az SC-5.0-ban igazolandók (a Kdenlive-terv K20-ához hasonló tábla: azonosító → CFA4-effekt, paraméternevek, tartományok, alapértékek).

| Shotcut-oldali csoport | CFA4 megfelelő | Besorolás (cél) |
|---|---|---|
| fényerő, kontraszt, telítettség, színhőmérséklet-jellegű alapszűrők | Lumetri Color alapparaméterei | átalakított (a skálák eltérnek, a leképezés dokumentált) |
| színkorrekció (lift/gamma/gain, színkerék, színezés) | Lumetri színkerekek | átalakított |
| vágás (Crop), méretezés, pozíció, forgatás (affin/„size, position & rotate”) | Crop és Transform (`motion.*`) | támogatott / átalakított |
| elmosás (Gaussian/box) | Gaussian Blur | átalakított |
| invertálás, szürkeárnyalat | Invert, színszűrők | támogatott |
| fade be/ki (videó: fényerő/átlátszóság; hang: hangerő) | Fade In / Fade Out, vagy kulcskockás Opacity/Volume (D5) | átalakított |
| hangerő, normalizálás, panoráma/balansz, hangszínszabályzó | Volume, Panner, EQ | támogatott / átalakított |
| sebességváltás, visszafelé (`timewarp`) | `clip.speedDuration` | támogatott |
| kulcskockák (animált paraméter) | a CFA4 kulcskocka-modellje; az interpoláció típusa (lineáris/sima/ugrás) megőrzendő, ahol van megfelelője | átalakított |
| LUT, maszk, kulcsolás (chroma/luma key) | a CFA4 megfelelő effektje, ahol van | átalakított / nem támogatott |
| `frei0r.*`, `avfilter.*`, `movit.*`, `webvfx`/QML, `ladspa`/`lv2` | – (külső bővítmény, nincs saját megfelelő; a K5/K44 terv szerint később) | nem támogatott: kihagyva, jelentve névvel |
| Shotcut-specifikus „Text”, „Subtitles”, „Overlay HTML” | CFA4-cím (`graphics.*`), ahol tartalom és stílus kinyerhető | átalakított / nem támogatott (SC-5.4) |

**Alapelv:** a szűrő **kikapcsolt állapota** (a Shotcut szűrőlistán kikapcsolt szűrő) átjön kikapcsolva, hogy ne változtassa meg a képet; egy ismeretlen szűrő **nem helyettesíthető „valami hasonlóval”**, mert az csendes tartalmi hibát adna.

### 3.5 Amit a hatókör szándékosan nem vállal

- A Shotcut **felhasználói felületi állapota** (panelelrendezés, nagyítás, billentyűk): nem projektadat.
- A Shotcut **render-/exportbeállításai** (a fájlba nem, vagy nem megbízhatóan kerülnek): nem importálódnak; a CFA4 saját export-előbeállítását használja a felhasználó.
- Külső, élő hivatkozás más `.mlt` projektre (a Shotcutban ilyen beágyazott projektforma lehet): nem követi az olvasó, jelentés (az F0-terv F12 döntésével egyező: a CFA4 nem vezet be második, élő hivatkozási rendszert).
- **Proxy-fájlok** (Shotcut proxy-szerkesztés): az eredeti forrásra mutat vissza az olvasó, ha az megtalálható; ha nem, offline klip és jelentés.

---

## 4. A nem támogatott elemek kezelése és a jelentés (R10, R12)

### 4.1 Besorolás és jelentés

Minden beolvasott elem négy osztály egyikébe kerül: **támogatott** (hűen átjött), **átalakított** (átjött, de a leképezés közelítő, a jelentés megmondja, mi változott), **nem támogatott** (kihagyva, a jelentés megnevezi), **kihagyott szándékosan** (3.5).

- A `Report` bejegyzése bővül **helyadattal** (sáv, klip, szűrő neve, idővonali pozíció) és osztállyal; az azonos üzenetek összevonása (számláló) megmarad a nagy projektekhez. A kiegészítés a meglévő `Report` típus visszafelé kompatibilis bővítése, a többi importer változatlan.
- A jelentés az import után **megjelenik** a Qt6 felületen (Import Report ablak: összesítő számok, szűrhető lista, „Ugrás a klipre”), és a parancs válaszában gépileg olvasható (MCP/CLI).
- A nem támogatott szűrőt viselő klipen az import **klipmarkert** hagy (`Marker`, megjegyzés-előtaggal, a `markers.addBeats` mintájára), hogy a felhasználó az idővonalon is lássa, hol hiányzik valami; az újrafuttatás a saját markereket cseréli, mást nem bánt. (D3: ez a javaslat.)
- A jelentés **összegző sora** kimondja, ha az eredmény **nem azonos** a Shotcut-kimenettel (például nem támogatott szűrő miatt a kép eltérhet).

### 4.2 Ellenséges bemenet (AGENTS.md §0.4–0.8)

A `.mlt` fájl nem megbízható bemenet. Kötelező:

- **XML-biztonság:** külső entitás (XXE) és DTD-feloldás **tiltva**; entitás-bővítési korlát; a fájlméret, az elemmélység, az elemszám és az attribútumhossz korlátos.
- **Hivatkozási ciklus:** a producer/tractor/playlist hivatkozások egymásra mutathatnak; a bejárás látott-halmazzal és mélységkorláttal (a beágyazás mélysége korlátos, `nest_cycle`-védelem a projektmodellben).
- **Számok:** képkockasebesség 0 vagy negatív, `out < in`, óriási hossz/blank, túlcsordulás, NaN a kulcskockákban → hibajelentés, nem pánik, nem osztás nullával (AGENTS.md §0.4).
- **Utak:** a médiahivatkozás nem léphet ki a megengedett gyökérből csendben (a meglévő `resolve_path`/relink szabályai szerint), és az olvasó **nem nyit meg** hálózati erőforrást (a távoli hivatkozás figyelmeztetés, K46 mintájára).
- **Mutációs fuzz-teszt** (csonkolás, bitfordítás, hibás méretek, `catch_unwind` alatt), a talált hibák regressziós tesztté válnak (AGENTS.md §0.8). Az importer a `fuzz/` mappa meglévő mintájához csatlakozik.

---

## 5. Nem-cél: a Shotcut-export (rögzített döntés)

**Az exportálás (CFA4 → Shotcut `.mlt`) explicit nem cél, és ebben a tervben sem szerepel semmilyen fázisban.**

**Indoklás (a tulajdonos döntése, 2026-11-22):** a CFA4 funkciói sokkal összetettebbek a Shotcutnál (többsávos Premiere-modell, kulcskockás Lumetri-színkezelés, csomópontos színkorrekció és VFX, beágyazott szekvenciák, multicam, MI-funkciók, saját átmenet- és effektkatalógus, Tick-pontos idő). Egy CFA4-projekt tartalma ezért **jelentős részben nem fejezhető ki** Shotcut-projektben; az export csak veszteséggel menne, és a veszteség a felhasználó számára nehezen látható. Az import fordított irányban értelmes: a Shotcut a szűkebb forrásmodell, ennek tartalma leképezhető a CFA4-be.

Ez összhangban van a korábbi döntésekkel: **MLT-be/Kdenlive-be exportálás nem lesz** (tulajdonosi döntés 2026-11-03, E10/E11; Kdenlive-terv K1: „Kdenlive-be exportálás nem lesz, mert csak veszteséggel menne”).

Következmények, amelyek a tervben rögzítettek:

1. Nincs `file.exportMlt`, nincs MLT-író modul, nincs „Export to Shotcut” menüpont, nincs kétirányú (round-trip) elfogadási feltétel.
2. A kapcsolódó csere-formátumok (EDL, OTIO, FCP7 XML, FCPXML, AAF) exportja változatlanul megmarad: aki Shotcutba szeretne vinni vágást, használja az ott is olvasható csereformátumokat; a veszteséget azok jelentése mutatja.
3. A `melt` (MLT) továbbra is **csak tesztorákulum** (E11), a termékbe nem kerül.
4. Az eredeti `.mlt` nem íródik vissza (R14).

---

## 6. Tesztek és elfogadás

### 6.1 Az elfogadási teszt: `sc5_shotcut_import_acceptance`

(`crates/interchange` + `crates/engine`, a fixture projektek saját magunk által készítettek és CC0-ként kiadottak; a kis `.mlt` XML-ek a `crates/interchange/tests/data/shotcut/` alatt lehetnek, a **média generált** `ffmpeg`-gel a `target/fixtures/`-be, nem commitolt.) Öt ellenőrzés:

- **(a) Vágópontok képkocka-pontosan:** a fixture projektek (több sáv, trimmelt klipek, `blank` részek, sebességváltás) importja után minden klip pozíciója, In/Out pontja és hossza megegyezik az **elvárt listával** (a `melt` orákulum kimenetéből és a fixture ismert felépítéséből); a sorozat teljes hossza `Tick`-pontosan egyezik.
- **(b) Teljes besorolás:** minden fixture-elem szerepel a jelentésben vagy a leképezésben; **nincs csendben eldobott elem** (a fixture ismert elemszámához mérve); a nem támogatott szűrő neve, a klip és a pozíció a jelentésben áll.
- **(c) Undo/redo és perzisztencia (R11):** az import egy undo-lépés (undo után a projekt **bájtra azonos** az import előttivel); mentés és újratöltés után a szekvencia azonos.
- **(d) Render:** a támogatott részhalmazra (vágások, átmenetek, alapszűrők, hangerő, sebesség) az import utáni CFA4-render és a `melt` render **képkockánként összevetve** a megengedett tűréssel (a tűrést az SC-5.6 méri és rögzíti); a tűrésen kívüli eltérés a jelentésben kimondott közelítésre vezethető vissza.
- **(e) Hibatűrés (R12):** a mutált/csonkolt/ellenséges fixture-ek (4.2) nem okoznak pánikot; a hibaüzenet használható.

### 6.2 További kategóriák

| Kategória | Teszt |
|---|---|
| Funkcionális | profil (több képkockasebesség: 24, 25, 30, 29.97, 23.976, 50, 59.94), sávsorrend és -állapot, klipek, `blank`, trimmelés, sebesség |
| Idő | ezredmásodperces és képkockás időformátum ugyanarra a vágásra azonos eredményt ad; nem képkockahatáron lévő érték jelentve |
| Átmenetek | átfedésből átmenet, a hossz és a sorozathossz nem csúszik; ismeretlen átmenet → kemény vágás + jelentés |
| Szűrők | a 3.4 tábla minden sorára legalább egy fixture; kikapcsolt szűrő kikapcsolva marad; kulcskockák és interpoláció |
| Beágyazás | beágyazott tractor, ciklus (önmagát tartalmazó) és mélységkorlát |
| Hiányzó média | offline klip, relink a meglévő útvonalon (`media.relink`) |
| Jelentés | elem-szintű helyadat, összevonás, gépi olvashatóság, a Qt6 ablak automatizálási azonosítói |
| Projektperzisztencia | a `.fcproj` séma nem változik; az importált szekvencia mentés/újratöltés után azonos |
| Regresszió | a többi importer (EDL, FCP7, FCPXML, OTIO, AAF, OMF) tesztjei változatlanok; a `Report` bővítése nem töri a meglévő hívókat |
| GUI | import párbeszéd és jelentésablak automatizálási azonosítókkal; valódi motoros teszt: megnyitás → jelentés → idővonal → undo |
| Hardver | CPU-only (az import nem igényel GPU-t) |
| Teljesítmény | nagy projekt (ezernyi klip) korlátos memóriával és belátható idő alatt; a feldolgozás megszakítható |
| Licenc | a fixture-ek saját készítésűek, `.attribution` ahol kell; `cargo deny` zöld |

**Nincs kész státusz** puszta fordulás vagy a GUI megjelenése alapján: kell a tesztkimenet (CI-napló) és a kimeneti viselkedés ellenőrzése (VISION.md 2.5: bizonyíték, nem állítás).

---

## 7. Szállítási bontás és becslés

Minden fázis **külön tulajdonosi engedélyt** igényel, és csak a terv jóváhagyása után indulhat. A becslés tájékoztató, nem mért. A fázisok sorrendje a K1-gyel való egyeztetéstől függ (D1).

| Fázis | Tartalom | Becslés |
|---|---|---|
| **SC-5.0** | Tényellenőrzés: 3–4 saját Shotcut-projekt (a Shotcut legfrissebb stabil kiadásával, D4) + az MLT XML-dokumentáció; a fájlszerkezet, a `shotcut:` tulajdonságok, az `out` jelentése, az időformátumok, a szűrő- és átmenetazonosítók valódi listája; fixture-generátor; a 3. pont táblájainak igazolása vagy javítása; egyeztetés a K1/K42-vel; a meglévő `xml.rs` alkalmasságának eldöntése | 4–8 ó |
| **SC-5.1** | Olvasó mag (vagy a K1 közös magja): profil, producerek, playlistek, tractor, sávok, klipek, `blank`, trimmelés; `file.import` felismerés; `Report` elem-szintű bővítése; offline média | 14–24 ó |
| **SC-5.2** | Sebesség, átmenetek (átfedés → átmenet), hang-keresztáttűnés, fade | 10–16 ó |
| **SC-5.3** | Szűrőleképezési tábla (K20-mintájú) és a gyakori szűrők, kulcskockák, interpoláció | 14–26 ó |
| **SC-5.4** | Markerek, szöveg/generátor, képsorozat, beágyazott tractor, proxy | 8–14 ó |
| **SC-5.5** | Qt6 import párbeszéd és jelentésablak, i18n (`i18n/<code>.tsv`), klipmarker a hiányzó szűrőhöz | 8–14 ó |
| **SC-5.6** | `sc5_shotcut_import_acceptance`, a 6.2 tesztjei, `melt`-összevetés, fuzz, dokumentáció (`qt-parity.md`, `docs/interop`-jellegű leírás, F0-terv állapota) | 8–14 ó |

Összesen kb. **66–116 óra** önállóan. Ha a K1 közös MLT-magja elkészül vagy együtt épül, az SC-5.1 és a szűrőtábla egy része közös (a K20/K42 költségét nem kell kétszer fizetni), és az SC-5 saját terhe kb. **40–75 óra**; ennek elosztása a D1 döntéstől függ.

---

## 8. Licenc, függőség, kockázat

- **Függőség:** új függőség nem kell; az olvasó tiszta Rust, a meglévő XML-eszközökre vagy a `crates/interchange`-ben már használt megoldásra épül (SC-5.0 dönti el). MLT-könyvtár nem kerül a termékbe (R13, E11).
- **Licenc:** a CFA4 GPL-3.0-or-later. A Shotcut (GPL-3.0) és az MLT (LGPL-2.1-or-later) kódja az `AGENTS.md` szerint átvehető volna fájlonkénti fejlécellenőrzéssel és a `docs/licensing.md` §5 nyilvántartás sorával, **de a javaslat: CFA4-natív megvalósítás**, a Shotcut/MLT csak viselkedési és formátum-referencia (az F0-terv 8. pontjával egyezően). Ha mégis átvételre kerülne sor, az külön döntés: fájl, fejléc, SPDX, nyilvántartási sor ugyanabban a commitban.
- **Tesztminták:** csak saját készítésű, általunk kiadott (CC0) projektfájlok és generált média; idegen vagy jogilag nem tisztázott Shotcut-projektet nem teszünk a repóba (AGENTS.md §1: ha a licenc nem tiszta, nem kerül be).
- **Kockázatok:** (1) a formátumot nem forrásból ismerem, ezért az SC-5.0 előtt a 3. pont táblái feltételesek; (2) az átfedésből képzett átmenet és az `out` jelentése képkocka-eltolódást okozhat, ha rosszul kezeljük, ezért az elfogadás (a) pontja `melt`-orákulummal ellenőriz; (3) a Shotcut új verziói új szűrőket és tulajdonságokat hozhatnak, ezért a tábla bővíthető és az ismeretlen elem mindig jelentés, soha nem hiba; (4) a szűrőskálák (fényerő, kontraszt, telítettség) nem egyeznek a Lumetri skáláival, a leképezés közelítő, ezért a jelentés „átalakított” minősítése kötelező; (5) a K1-gyel való átfedés kettős munkát okozna, ha nem egyeztetjük (D1).

---

## 9. A tulajdonosnak eldöntendő kérdések

| # | Kérdés | Javaslatom |
|---|---|---|
| **D1** | Az MLT XML olvasó legyen-e **közös** a Kdenlive-terv K1/K42/K20/K25 tételével (egy mag, két dialektus: Kdenlive és Shotcut)? Ez átfedés két projekt között, ezért a tulajdonos és a gazda dönt (`docs/coordination.md` §2.8). | Igen: közös MLT-mag a `crates/interchange`-ben, a dialektusonkénti tábla külön; az SC-5 a K1 magjára épül, vagy a mag közös PR-sorozatban készül |
| **D2** | A modul helye és neve: új `Format` változat (`Mlt`) a `crates/interchange`-ben, a `file.import` automatikus felismerésével? | Igen, egyetlen `Mlt` formátum, a dialektust a fájl tartalma dönti el |
| **D3** | A nem támogatott szűrő/átmenet kezelése: **jelentés + klipmarker** a klipen, vagy csak jelentés? (Helyettesítő „hasonló” effekt nincs.) | Jelentés és klipmarker |
| **D4** | Melyik Shotcut-verzió(k)kel készüljenek a fixture-ek és melyik legkorábbi verzió projektjét kell olvasnia az importernek? | A legfrissebb stabil kiadás és egy régebbi, a tulajdonos által megnevezett verzió; amit a régebbi nem teljesít, az az SC-5.0 jelentésében áll |
| **D5** | A fade be/ki szűrők: a CFA4 **Fade In/Out effektjére** képezzük, ahol a minta felismerhető, egyébként kulcskockás Opacity/Volume-ra? | Igen |
| **D6** | Az eredeti `.mlt` fájl **megőrzése** a projekt mappájában import után (a K22 „az eredeti megőrzése” mintájára), hogy később újraimportálható és összevethető legyen? | Igen, csak olvasott másolat, a CFA4-projekt nem hivatkozik rá élőben |

---

## 10. Mit nem ellenőriztem (kötelező nyilvánosság)

1. **A Shotcut forrását és fájlformátumának leírását nem vizsgáltam**; az MLT XML-szerkezetről, a `shotcut:` tulajdonságokról, az `out` jelentéséről, az időformátumokról és a szűrő-/átmenetazonosítókról írtak **ismeretből valók**, nem ellenőrzöttek. Az SC-5.0 dolga igazolni vagy javítani őket.
2. **Valódi Shotcut-projektet nem láttam**; a repóban nincs Shotcut-fixture.
3. **Kódot nem futtattam**; a `crates/interchange` felépítése és a `Report` típus a kód olvasásából következik.
4. Hogy a meglévő `crates/interchange/src/xml.rs` biztonságosan (XXE-tiltás, korlátok) használható-e az MLT-hez, nem néztem meg; az SC-5.0 első lépése.
5. Hogy a **K1/K42/K20/K25 elkezdődött-e** valamelyik nyitott PR-ban vagy ágon, nem ellenőriztem teljesen: a `main`-en kódja nincs, a nyitott PR-ok címei között `mlt`/`kdenlive`/`shotcut` olvasóra utalót nem találtam, de a tartalmukat nem néztem át. Az SC-5.0 előtt a Kdenlive-terv IV. pontja szerinti újraelemzés kötelező.
6. A **`plan/` mappát** (STATUS.md, execution-plan.md) nem láttam (gép-lokális, gitignore-olt).
7. A becslések sajátok, nem mértek.
8. A Qt6 Open/Import útvonal pontos helyét és a menüszerkezetet a megvalósításkor kell a meglévőhöz igazítani.

---

## 11. Fázis-lezárási bizonyíték (a megvalósításra)

Minden fázis lezárásához kötelező: CI-napló (`cargo xtask ci` + tesztek), a `melt`-összevetés kimenete (6.1 (a), (d)), az undo/redo és mentés-újratöltés teszt kimenete, a mutációs fuzz-teszt kimenete, a GUI-automatizálási napló, regressziós összevetés (a többi importer tesztjei), a `docs/qt-parity.md` és az F0-terv állapotsorának frissítése. Az SC-5 akkor **kész**, ha az `sc5_shotcut_import_acceptance` mind az öt ellenőrzése zöld, a 6.2 táblázat minden sorára van teszt, és az SC-5.0 a 3. pont táblázatait valódi Shotcut-projektekkel igazolta. A terv menet közbeni módosítása leállítja a fejlesztést a tulajdonosi jóváhagyásig (`docs/coordination.md` §2.10).

---

## 12. Kapcsolat az F0-tervvel

- Az F0-terv 1. pont táblájának **F4** sora és a 2. pont F4 szakasza az MLT-**importot** a K1/K42-re bízza és az exportot elveti. Ez a pótlás az **F4 Shotcut-specifikus részét** nyitja meg önálló tételként (SC-5), az export elvetése változatlan (5. pont).
- Az SC-1…SC-4 tételeket, sorrendjüket és jóváhagyásukat ez a pótlás nem érinti.
- A meglévő CFA4-ütemezés (`docs/roadmap.md` „Current focus”) nem változik; az SC-5 nem kerül be az ütemezésbe, csak jóváhagyás és külön indítási engedély után indulhat.
