# CFA4-FFMPEG-BACKEND-R1
## FFmpeg 9.0.2 natív integráció – végrehajtási terv

**Verzió:** R1  
**Dátum:** 2026-10-27  
**Státusz:** Megvalósításra javasolt terv  
**CFA4 repository:** https://github.com/ubuntuokos/CinemaFundArchitecture4  
**Vizsgált main:** `2f0371fc2e45`  
**Cél:** a CFA4 natív Rust médiamotorjának kiegészítése FFmpeg 9.0.2-alapú funkciókkal, a meglévő architektúra és fejlesztések megőrzésével.  
**Kapcsolódó tervek:** [ADR 0005](../adr/ADR-0005-media-backends.md), [CFA4-SCE-001 v1.5](CFA4-SCE-001-STREAM-COPY-EXPORT-2026-10-27.md), [GOP-alapú Smart Cut](GOP-SMART-CUT-2026-10-27.md), [Smart Render](SMART-RENDER-2026-10-27.md)

> **Állapot-megjegyzés (2026-10-27, a repóba emeléskor):** a tulajdonos által megadott terv szövege
> változatlan; e megjegyzésen kívül két módosítás történt: az upstream ős nevét a repository függetlenségi szabálya miatt
> (`python/tests/test_independence.py`) általános megnevezés helyettesíti, és a 2. fejezet PR-táblázata
> a #224 állapotát frissíti. A 3. fejezet wrapper-jelöltjét a tulajdonos 2026-10-27-i döntése semlegesítette. A #224 2026-10-27-én egyesült (`85b4fc1`): az ADR 0005 a `main`-ben van,
> az `AGENTS.md` és a `CLAUDE.md` a tervezett `cfa4-ffmpeg` crate-en keresztül, a `VISION.md` kiegészítő
> háttérmotorként engedi az FFmpeg ≥ 9.0.2-t. Az FF-00-ból így a licencvizsgálat (WTFPL nincs a `deny.toml`
> listáján), a worker/FFI biztonsági határ dokumentálása és a Smart Cut különválasztásának rögzítése
> marad. A CFA4-SCE-001 v1.5 „FFmpeg csak CI-orákulum” szabálya a Stream Copy, GOP Smart Cut, Smart
> Render és a natív MPEG-TS muxer végrehajtására vonatkozik, összhangban e terv 9. fejezetével.

---

## 1. Kötelező technológiai keretek

| Terület | Követelmény |
|---|---|
| Operációs rendszer | Ubuntu 26.04 LTS és változatai |
| Architektúra | Linux AMD64 / x86_64 |
| Rust | CFA4 toolchain: 1.97.1 |
| Fő médiamotor | CFA4 saját Rust médiamotor |
| FFmpeg | Minimum 9.0.2 |
| Integráció | Rust API és natív FFmpeg-könyvtárak |
| Alternatív médiamotor | Meglévő GStreamer |
| Útválasztás | `cfa4-route` |
| Felület | Qt6 / PySide6 |
| Licenc | CFA4: GPL-3.0-or-later |
| Csomagolás | DEB, Flatpak, AppImage |
| GPU | NVIDIA, AMD, Intel; CPU fallback |
| Conda | Nem alkalmazható |
| Windows/macOS/Android | Nem célplatform |

Az upstream (örökölt ős) projekt változatlan marad. Minden módosítás kizárólag a CFA4-ben történik.

### Alapelvek

1. A meglévő saját Rust kodekek és médiakomponensek megmaradnak.
2. Nem épül második szerkesztőmotor vagy párhuzamos exportvezérlő.
3. A médiaháttereket a meglévő CFA4 infrastruktúrán keresztül kell elérni.
4. A hiányzó képességeket FFmpeg-ből egészítjük ki, nem írjuk újra indokolatlanul.
5. A már igazolt saját dekódolókat és enkódereket csak tesztelt előny esetén válthatja fel egy alternatív útvonal.
6. Az FFmpeg nem módosíthatja a CFA4 projekt- és időzítésmodelljét.
7. Hibás vagy hiányzó FFmpeg esetén a CFA4 főalkalmazásnak működőképesnek kell maradnia.
8. Smart Cut/Smart Render alatt nincs automatikus, felhasználó elől rejtett újrakódolás.

---

## 2. Ellenőrzött CFA4 kiindulási állapot

### Meglévő rendszerek

- `crates/route`: központi GPU/NPU/CPU feladatútválasztó.
- `crates/gstreamer`: opcionális GStreamer backend, médiafelismerés és import.
- `crates/platform`: natív Linux hardveres dekódolás és NVENC integráció.
- `crates/media`: médiaforrások és fallback-regisztráció.
- `crates/codecs`: saját kodekek és GOP-dekódolási infrastruktúra.
- `crates/export`: saját kódolási, renderelési és muxolási rendszer.
- `crates/render`: natív CPU/GPU kompozitor.
- `crates/isobmff`, `matroska`, `mxf`, `mpegts`: saját konténerkezelés.
- `python/cfa4_qt`: Qt6 szerkesztőfelület.

A `cfa4-route` táblájában már szerepel az `ffmpeg` háttér, de végrehajtó implementáció még nincs hozzá.

A `cfa4-media` fallback-regisztere több háttér kezelésére alkalmas, de a sorrend jelenleg regisztrációs sorrendhez kötött. Ezt össze kell hangolni a központi útválasztóval.

### Kapcsolódó PR-ek

| PR | Tárgy | Ellenőrzött állapot |
|---|---|---|
| #224 | ADR 0005 – médiahátterek | Egyesítve (2026-10-27, `85b4fc1`) |
| #222 | GOP Smart Cut / Smart Render tervek | Egyesítve |
| #228 | CFA4-SCE-001 v1.5 végleges terv | Nyitott |
| #229 | Smart Cut R1 terv összehangolása | Nyitott |
| #215 | Smart Edit kiindulási audit | Nyitott |
| #216 | Smart Edit Capability Registry | Nyitott |
| #219 | MPEG-2 és H.264 GOP-indexelő | Nyitott |

Az új FFmpeg-integráció nem készíthet második Capability Registryt, GOP-indexelőt vagy export-plannert.

### Dokumentációs szabályütközések

A jelenlegi `AGENTS.md` és `CLAUDE.md` az FFmpeg-et még csak külső tesztorákulumként engedi használni.

Az ADR 0005 ezzel szemben már megengedi a natív FFmpeg-háttér tervezett termékbeli integrációját.

Az új szabály csak a #224 megfelelő egyesítése, valamint a kötelező fejlesztői szabályok összehangolása után alkalmazható.

---

## 3. A Rust–FFmpeg kötés kiválasztása

### Jelöltek

**A – rust-ffmpeg / ffmpeg-next**

Repository: https://github.com/zmwangx/rust-ffmpeg

- Közvetlen Rust wrapper a natív FFmpeg-könyvtárakhoz.
- FFmpeg 9 támogatás.
- Saját demux, decode, encode és filtervezérlés.
- Alacsony szintű hozzáférés a médiacsomagokhoz.
- Viszonylag könnyen illeszthető a CFA4 egyedi absztrakcióihoz.
- A crate elsősorban karbantartási üzemmódban működik.

Ellenőrzött verzióállapot:
- crates.io `ffmpeg-next`: 9.0.0
- GitHub-forrás: 9.0.1
- crates.io `ffmpeg-sys-next`: 9.0.1
- FFmpeg célverzió: 9.0.2

**B – rsmedia / rsmpeg**

Repository: https://github.com/dromara/rsmedia

- Magasabb szintű Rust médiakezelés.
- FFmpeg 9 támogatási útvonal.
- Integrált dekódolási, kódolási és remuxolási API-k.
- Készebb media-pipeline építőelemek.
- Hátránya, hogy az absztrakciói részben átfednek a CFA4 meglévő rendszereivel.
- A vizsgált `rsmedia` verzió Git-függőségen keresztül hivatkozik az `rsmpeg` egy forkjára.

### Javasolt kiválasztási eljárás

Mindkét jelöltet ugyanazzal az FFmpeg 9.0.2 builddel kell kipróbálni, egymástól izolált teszt-crate-ekben.

A vizsgálat összehasonlítja:

1. Fordíthatóság Rust 1.97.1 alatt.
2. H.264/HEVC/AV1 dekódolási API-t.
3. 10/12 bites és 4:2:2/4:4:4 formátumok megőrzését.
4. DTS/PTS és codec extradata kezelését.
5. Remux és csomagszintű hozzáférést.
6. GPU-API és hardveres enkóder kezelését.
7. Memóriaélettartam- és szálbiztonságot.
8. Licenc- és függőségi megfelelőséget.
9. Futásidejű hibák kezelését.
10. A szükséges CFA4-specifikus adapterkód mennyiségét.

**Tulajdonosi döntés (2026-10-27): nincs előre kijelölt elsődleges jelölt; az FF-01 mérése dönt.**

Az eredeti javaslat az `ffmpeg-next`-et nevezte meg elsődlegesnek a közvetlenebb API miatt, az elfogadott ADR 0005 viszont az rsmediát rögzíti („az FFmpeg-háttér (rsmedia) a kiadott csomagokba is bekerül”). A két jelöltet az FF-01 azonos feltételekkel hasonlítja össze; az ADR 0005-öt csak az FF-01 eredménye után kell igazítani, ha a mérés az `ffmpeg-next`-et választja. A kiválasztás csak a licenc- és fordítási kapu teljesülése után véglegesíthető.

A `ffmpeg-next` és `ffmpeg-sys-next` WTFPL licencű. A WTFPL nincs a jelenlegi `deny.toml` engedélyezési listájában. Az átvétel előtt külön licencellenőrzés és dokumentált policy-módosítás szükséges. Ezt nem szabad automatikusan átírni.

Ha az `ffmpeg-next` licence nem fogadható el, az `rsmedia/rsmpeg` útvonalat kell választani, rögzített és auditált függőségekkel.

**Egyetlen FFmpeg-kötés kerülhet a végleges CFA4-adapterbe.** Párhuzamos wrapper-rendszer nem épülhet.

---

## 4. Végleges architekturális cél

A CFA4 teljes médiafolyamának felépítése:

**CFA4 Qt6 GUI / CLI / Control / MCP**

↓

**CFA4 Engine**

↓

**cfa4-route + egységes médiaképesség-nyilvántartás**

↓

- CFA4 natív Rust média- és kodekútvonal
- CFA4 platform GPU backend
- CFA4 FFmpeg 9.0.2 backend
- CFA4 GStreamer backend
- CFA4 szoftveres tartalék

↓

**Meglévő render/export pipeline**

A tényleges sorrend feladatonként és igazolt képesség alapján érvényesül, az ADR 0004 és ADR 0005 szabályai szerint.

### Új komponensek

**`crates/ffmpeg/`**

Cargo-név: `cfa4-ffmpeg`

Réteg: L5.

Feladata:
- FFmpeg backend állapotának vizsgálata.
- Biztonságos médiainterfészek.
- FFmpeg-műveletek indítása.
- CFA4 adatmodell-konverzió.
- Hibák és képességek publikálása.
- Backend-regisztráció.

Javasolt modulok:

- `probe.rs`
- `backend.rs`
- `discovery.rs`
- `decode.rs`
- `encode.rs`
- `mux.rs`
- `filter.rs`
- `frames.rs`
- `timestamps.rs`
- `metadata.rs`
- `errors.rs`
- `ipc.rs`

**`apps/cfa4-ffmpeg-worker/`**

Opcionális, belső natív Rust segédfolyamat.

Feladata az FFmpeg C-könyvtárakat használó műveletek izolált végrehajtása.

Ez nem önálló felhasználói alkalmazás, nem külön GUI és nem `ffmpeg` CLI-meghívás.

### Miért javasolt az elkülönített worker?

A Rust wrapper biztonságos API-ja sem akadályozza meg minden esetben, hogy egy natív C-könyvtár hibája megszakítsa a teljes folyamatot.

Az FFmpeg-feldolgozás külön folyamatban történő futtatása:

- csökkenti a CFA4 szerkesztő összeomlásának kockázatát;
- megengedi a worker újraindítását;
- elválasztja a natív könyvtárak betöltését a főalkalmazástól;
- lehetővé teszi, hogy a főalkalmazás hiányzó FFmpeg mellett is elinduljon.

A két komponens között típusos, verziózott és korlátozott méretű IPC-protokoll legyen. Az első implementáció vezérlőüzeneteket és korlátozott képkocka-/csomagátvitelt használjon; a nagy teljesítményű memóriamegosztást csak mérések után kell bevezetni.

A Smart Cut és Smart Render saját Rust-végrehajtása **nem függhet a workertől**.

---

## 5. FFmpeg verzió- és futtatókörnyezet-kezelés

### Kötelező változat

FFmpeg 9.0.2 vagy megfelelően auditált, kompatibilis 9.x patch-verzió.

A kezdeti reprodukálható referencia pontosan FFmpeg 9.0.2.

### Build

- Csak Linux AMD64.
- Megosztott FFmpeg-könyvtárak.
- GPLv3-kompatibilis konfiguráció.
- `--enable-gpl`
- `--enable-version3`
- `--disable-nonfree`
- Kezdetben x264/x265 nélküli alapbuild.
- A szükséges VA-API/Vulkan képességek külön ellenőrzéssel.
- Nincs rendszer FFmpeg-csere.

A build összes külső könyvtárának és licencének ellenőrzése kötelező.

### Verzióvizsgálat

A `probe()` ellenőrizze:

- a ténylegesen betöltött libav könyvtárak verzióját;
- az egyes libav komponensek ABI-kompatibilitását;
- az FFmpeg build konfigurációját;
- a szükséges dekódereket és enkódereket;
- a hardveres gyorsítási képességeket;
- a tényleges könyvtárbetöltési útvonalat.

Nem elegendő az `ffmpeg -version` parancs kimenete: az nem bizonyítja, hogy a CFA4 ugyanazokat a megosztott könyvtárakat használja.

### Verziórögzítés

A kiválasztott Rust wrapper:
- pontos release-verzióval vagy commit SHA-val rögzítendő;
- átmeneti Git-tesztfüggőségből csak auditált, reprodukálható kiadási függőség lehet;
- külön SBOM- és licencnyilvántartási bejegyzést kap.

---

## 6. MediaBackend adapter

### Kötelező interfészek

Az FFmpeg-adapter a meglévő CFA4 interfészeket valósítsa meg, vagy azokhoz biztosítson adaptert:

- `MediaSource`
- `VideoDecoder`
- `VideoEncoder`
- Audio-dekódolási és kódolási interfészek
- Konténer-demux/mux szolgáltatások
- Filter/feldolgozó interfészek
- Backend capability/probe

A CFA4 projektmodellje nem függhet FFmpeg-specifikus típusoktól.

### Időzítés

Minden FFmpeg időbélyeg a CFA4 pontos időmodelljére legyen leképezve.

Alap:
`cfa4_time::Tick`

Követelmények:
- racionális timebase megőrzése;
- ellenőrzött `i128` köztes műveletek;
- DTS és PTS külön kezelése;
- B-frame reorder;
- negatív kezdő PTS/DTS;
- hiányzó időbélyeg;
- változó képkockasebesség;
- MPEG-TS timestamp wrap;
- audio encoder delay, priming és padding.

Szerkesztési időzítésben lebegőpontos másodperc nem használható elsődleges reprezentációként.

### Képkocka- és metaadatkezelés

A dekódolt média CFA4-formátumba történő átalakításakor meg kell őrizni:

- felbontás és sample aspect ratio;
- 8/10/12 bites mintamélység;
- 4:2:0, 4:2:2 és 4:4:4 chroma;
- alpha, ha támogatott;
- color primaries;
- transfer characteristics;
- matrix coefficients;
- teljes/korlátozott színtartomány;
- PQ/HLG és HDR mastering metadata;
- pixel- és hangminták időzítése;
- audio channel layout;
- codec extradata és szükséges side data.

Ha a CFA4 aktuális `Frame` típusa nem tud veszteségmentesen átvenni egy formátumot, az adapter nem végezhet észrevétlen 8 bites konverziót. Előbb megfelelő típusbővítés vagy explicit, felhasználó által engedélyezett konverzió szükséges.

---

## 7. Kapcsolat a cfa4-route rendszerrel

A meglévő `cfa4-route` marad az egyetlen központi útválasztó.

Az FFmpeg backend `ffmpeg` azonosítóval regisztrálódik.

### Példafeladatok

- `video.decode.h264`
- `video.decode.hevc`
- `video.decode.av1`
- `video.encode.h264`
- `video.encode.hevc`
- `video.encode.av1`

A demux, médiafelismerés és filterfeldolgozás további képességeit a meglévő média- és backend-interfészeken keresztül kell kezelni. Új útválasztó-család csak akkor létesülhet, ha a jelenlegi modell bizonyítottan elégtelen.

### Kötelező döntési szabály

A backend kiválasztása az alábbiaktól függjön:

1. Támogatja-e a műveletet?
2. Támogatja-e a pontos profilt?
3. Megőrzi-e a szükséges formátumot és metaadatokat?
4. Van-e megfelelő GPU vagy CPU végrehajtási út?
5. Teljesülnek-e a licenc- és runtime-követelmények?
6. Átment-e az adott képesség a tesztkapun?
7. A választás megfelel-e a felhasználó beállításának?

A `routing.toml` már tartalmazza az FFmpeg helyét. A háttér csak sikeres próba után kaphat működőképes státuszt.

### MediaSource fallback

A meglévő `crates/media/src/fallback.rs` regisztrációs mechanizmusát össze kell hangolni a routerrel:

- Natív megnyitás elsőként.
- FFmpeg és GStreamer sorrendje a router döntése szerint.
- I/O-hibára és offline médiára nem indul automatikus másik dekóder.
- Ha a natív megnyitás már helyes médiaeredményt ad, a fallback nem ronthatja el.
- Minden választás és elutasítás oka naplózandó.

A két útválasztási mechanizmus nem adhat egymásnak ellentmondó eredményt.

---

## 8. A FFmpeg elsőként megvalósítandó képességei

### R1 – Import és dekódolási kiegészítések

Elsődlegesen azokat a hiányokat kell kezelni, amelyek ma akadályozzák a professzionális média importálását:

| Terület | Javasolt FFmpeg-feladat |
|---|---|
| H.264 High 10 | Kiegészítő dekódolás |
| H.264 High 4:2:2 / 4:4:4 | Kiegészítő dekódolás |
| H.264 interlaced/MBAFF/PAFF | Kiegészítő dekódolás |
| HEVC RExt | Hiányzó 4:2:2/4:4:4/12-bit profilok |
| DV/DVCPRO | Import |
| MPEG-4 Part 2 | Import |
| WMV/VC-1 | Import |
| Egyéb FFmpeg által kezelt konténerek | Csak igazolt fallback |

Minden profil külön képességként szerepeljen, ne általános „H.264 támogatott” állapotként.

A meglévő ProRes, DNx, AV1, JPEG 2000 és más saját dekódolók megmaradnak. Ezek helyettesítésére nincs automatikus felhatalmazás.

### R2 – Hardveres kódolás és dekódolás

- FFmpeg VA-API kódolási lehetőségek AMD/Intel gépeken.
- Intel QSV támogatás, ha az adott FFmpeg-build és rendszer ténylegesen biztosítja.
- FFmpeg Vulkan Video képességek vizsgálata.
- AV1 hardveres dekódolás, ahol a GPU támogatja.
- Meglévő NVDEC/NVENC CFA4 útvonal regressziómentes megtartása.
- CPU fallback.

Az adott GPU képessége nem következtethető kizárólag az FFmpeg encoder/decoder listájából. Valós inicializálási és mintafeldolgozási próba szükséges.

### R3 – Formátumkonverzió

- Natív konténerkonverzió kiegészítése.
- FFmpeg filtergráfok használata, ahol a saját motor még nem rendelkezik megfelelő feldolgozóval.
- Hangformátumok konverziója.
- Átméretezés és pixel-formátum átalakítás.
- Professzionális bitmélység- és HDR-megőrzés.

Nem készül új párhuzamos képfeldolgozó vagy effektvezérlő.

---

## 9. Smart Cut, Stream Copy és Smart Render elhatárolása

A következő projektek különállóan megmaradnak:

- `CFA4-SCE-001 v1.5`
- GOP Smart Cut
- Smart Render
- Strict / Compatible Stream Copy
- Natív MPEG-TS/M2TS muxer

**Ezek végrehajtó motorja a CFA4 saját Rust-motorja.**

Az FFmpeg-adapter nem helyettesíti:
- a saját GOP-indexelőt;
- a referenciafüggőség-vizsgálatot;
- a Stream Copy garanciáinak ellenőrzését;
- a Smart Render szakasztervezőt;
- a natív muxereket;
- az atomi exportírót.

A `cfa4-smart-edit` képességmodellje és a `cfa4-route` backend-döntése egységesen legyen felhasználva, de a kettő eltérő felelőssége maradjon világos.

### Bitazonos Stream Copy

A szigorú módot kizárólag az igazolt natív CFA4 csomagmásolás teljesítheti.

Ha FFmpeg bitstream-filter, transzkódoló vagy egyéb payload-módosító művelet kellene, a Strict mód nem engedélyezhető.

### Kompatibilis Stream Copy

A szükséges konténer- és időbélyeg-módosításokat a jóváhagyott szabályok szerint lehet elvégezni. A másolt tömörített payload integritását külön kell igazolni.

A mód nem változhat automatikusan teljes transzkódolássá.

### Smart Render

A Smart Render planner továbbra is a `cfa4-smart-edit` modulban, a végrehajtó pedig a `cfa4-export` rendszerben marad.

FFmpeg használata legfeljebb külön igazolt codec- vagy feldolgozási háttérként jöhet szóba. Ettől egy szakasz nem válik automatikusan biztonságosan másolhatóvá.

---

## 10. Biztonság és hibakezelés

### Kötelező elvárások

- Sérült média nem okozhat CFA4 összeomlást.
- Hibás FFmpeg runtime nem akadályozhatja a főalkalmazás indulását.
- Minden adapterhiba tipizált hibakóddal és magyarázattal térjen vissza.
- Védett legyen a túlméretes felbontás, packet, időbélyeg és stream-szám.
- Legyen műveleti timeout, memóriahatár és megszakítás.
- A forrásfájl ne módosuljon.
- Export során csak validált ideiglenes kimenetből lehessen végleges fájl.
- Worker-crash esetén a szerkesztő állapota maradjon meg.

### Unsafe kezelés

A CFA4 által írt adapterkód maradjon `unsafe`-mentes.

Az FFmpeg natív FFI-hívásai a kiválasztott wrapperben és a külön workerben legyenek elszigetelve. Ha saját `unsafe` FFI-kód válik szükségessé, a meglévő `crates/platform`-ra korlátozott szabály alól nem lehet csendben kivételt tenni.

Előbb új, kifejezett architekturális szabály és biztonsági audit szükséges.

### Fallback

Dekódolásnál a feladat biztonságosan újraindítható a következő backenddel.

Exportnál részlegesen kiírt kimeneten nem szabad észrevétlen backendváltást végezni. Csak törlés/rollback és teljes újraindítás engedhető meg a kiválasztott vagy a felhasználó által jóváhagyott tartalék útvonalon.

---

## 11. FFmpeg 9.0.2 csomagolása

### DEB

A CFA4 saját FFmpeg megosztott könyvtárai elkülönített, verziózott telepítési helyre kerüljenek, például:

`/opt/cfa4/ffmpeg-9.0.2/lib/`

A saját FFmpeg build nem írhatja felül az Ubuntu rendszerkönyvtárait.

Az adapter/worker betöltési útvonala kizárólag az erre kijelölt könyvtárra mutasson, ellenőrzött `RUNPATH`/loader-konfigurációval.

### AppImage

Az FFmpeg-könyvtárak az AppDir külön belső könyvtárába kerüljenek.

A futtatás relatív könyvtárbetöltési útvonalat használjon. Ne igényeljen `/opt/cfa4` telepítést.

### Flatpak

Az FFmpeg-könyvtárak a Flatpak csomag saját alkalmazási területére kerüljenek, az alkalmazás futtatókörnyezetének szabályai szerint.

### Korlátozott kodekkiegészítők

Az x264/x265 és minden más, külön kezelt komponens a `cfa4-restricted-extras` telepítési modelljét kövesse.

Fontos: egy x264 nélkül fordított FFmpeg nem feltétlenül válik x264-képessé pusztán egy könyvtár utólagos telepítésével. A kiegészítő csomaghoz kompatibilis FFmpeg-build vagy külön végrehajtó szükséges, amelyet a CFA4 futáskor felismer.

A licenc- és szabadalmi tájékoztatás, valamint a felhasználói jóváhagyás kötelező.

### Kiadási ellenőrzések

- `readelf -d`
- `ldd`
- `pkg-config`
- FFmpeg runtime probe
- licencfájlok/NOTICE
- SBOM
- csomagból telepített indítás
- eltávolított vagy hibás backend melletti indítás
- rendszer FFmpeg sértetlensége

---

## 12. Végrehajtási munkacsomagok

### FF-00 – Architektúra és szabályok összehangolása

**Függőség:** #224 ADR 0005.

Feladatok:
1. `VISION.md`, `AGENTS.md`, `CLAUDE.md` összehangolása.
2. ADR 0005 véglegesítés.
3. A saját Rust Smart Cut és az opcionális FFmpeg-backend különválasztásának rögzítése.
4. `docs/licensing.md` és `deny.toml` licencvizsgálata.
5. A worker-folyamat és az FFI biztonsági határának dokumentálása.

**Elfogadás:** nincs ellentmondás a kötelező fejlesztői szabályok és az FFmpeg integrációja között.

### FF-01 – Wrapper kiválasztása

Feladatok:
1. `ffmpeg-next` és `rsmedia/rsmpeg` izolált prototípus.
2. FFmpeg 9.0.2 build teszt.
3. Dinamikus linkelés ellenőrzése.
4. Demux, dekódolás, seek, remux API-összevetés.
5. Függőségi és licencellenőrzés.

**Elfogadás:** egyetlen választott Rust-kötés, rögzített forrásrevízióval és dokumentált mérési eredményekkel.

### FF-02 – cfa4-ffmpeg crate

Feladatok:
1. Új L5 crate.
2. Feature-gate és `Unavailable` állapot.
3. `probe()` és képességriport.
4. Biztonságos adapterinterfészek.
5. Backend-hibaosztályok.

**Elfogadás:** a teljes Rust workspace FFmpeg nélkül is fordul, és az adapter tesztjei zöldek.

### FF-03 – Natív FFmpeg worker

Feladatok:
1. Belső worker futtatható állomány.
2. Verziózott IPC.
3. FFmpeg 9.0.2 könyvtárbetöltés.
4. Timeout, cancel és crash kezelés.
5. Biztonságos worker-életciklus.

**Elfogadás:** worker-megszakításkor a CFA4 nem omlik össze, nem veszít projektadatot, és a hibát jelenteni tudja.

### FF-04 – MediaSource és dekódolás

Feladatok:
1. FFmpeg `MediaSource` adapter.
2. Médiafelismerés és metaadatok.
3. Pontos PTS/DTS kezelés.
4. H.264/HEVC professzionális profilok.
5. Seek, VFR és interlace.
6. Formátum- és HDR-konverzió ellenőrzése.

**Elfogadás:** az első kiválasztott profilok importja, seekje és dekódolása regresszió nélkül működik.

### FF-05 – Route és GStreamer integráció

Feladatok:
1. FFmpeg regisztráció a `cfa4-route` rendszerben.
2. `cfa4-media` fallback sorrendjének összehangolása.
3. Capability Registry integráció.
4. `perf.stats` route-állapot.
5. Backend kiesése és fallback-tesztek.

**Elfogadás:** a kiválasztott backend, az elutasított lehetőségek és a fallback sorrendje reprodukálható.

### FF-06 – Kódolás és hardveres gyorsítás

Feladatok:
1. `VideoEncoder` adapter.
2. H.264/HEVC/AV1 képességvizsgálat.
3. VA-API kódolás.
4. NVIDIA/AMD/Intel kompatibilitás.
5. CPU fallback.
6. HDR és professzionális profilok.

**Elfogadás:** csak ténylegesen működő profilok kapcsolhatók be, a kimenet visszaolvasható és validált.

### FF-07 – Export és konverzió

Feladatok:
1. FFmpeg encoder és mux adapter.
2. Egységes CFA4 exportjob-kezelés.
3. A közös `cfa4-export` atomikus író használata (CFA4-SCE-001 v1.5, 6. döntés; nem készül második író).
4. Filter- és resample-illesztés.
5. Cancel/progress.
6. Kimeneti visszaellenőrzés.

**Elfogadás:** import → feldolgozás → export → reimport tesztek sikeresek. A natív Smart Cut/Smart Render változatlanul működik.

### FF-08 – Csomagolás

Feladatok:
1. FFmpeg 9.0.2 reprodukálható build.
2. DEB integráció.
3. AppImage integráció.
4. Flatpak integráció.
5. `cfa4-restricted-extras` kompatibilitás.
6. Runtime és licencellenőrzések.

**Elfogadás:** a három támogatott csomagformátum elindul a célplatformon, a rendszer FFmpeg-jét nem módosítja, és hiányzó backend mellett megfelelően viselkedik.

### FF-09 – E2E validáció

Feladatok:
1. Qt6 médiaimport.
2. Lejátszás és pontos seek.
3. Kódolás és export.
4. Reimport.
5. CPU/GPU váltás.
6. Sérült médiák.
7. Hiányzó runtime.
8. Teljes regressziós teszt.

**Elfogadás:** reprodukálható zöld CI és célgépes E2E bizonyíték.

---

## 13. Tesztelési követelmények

### Kötelező tesztmátrix

| Terület | Követelmény |
|---|---|
| H.264 8-bit 4:2:0 | Natív regresszió |
| H.264 High 10 | FFmpeg fallback |
| H.264 4:2:2 | FFmpeg fallback |
| HEVC Main/Main 10 | Natív regresszió |
| HEVC RExt | Profilfüggő fallback |
| AV1 Main | Natív regresszió + GPU probe |
| ProRes/DNx | Natív export és import megtartása |
| MPEG-2 | Natív dekóder regresszió |
| DV/VC-1/MPEG-4 Part 2 | FFmpeg importpróbák |
| VFR | Pontos PTS/DTS |
| B-frame | Decode/display sorrend |
| HDR PQ/HLG | Jelzés és mintahelyesség |
| Audio | Mintaszám, channel layout, szinkron |
| Sérült bitstream | Hiba, nincs főalkalmazás-crash |
| Backend hiánya | Működő fallback |
| Full disk / cancel | Atomic output és cleanup |

### Minőség

A saját és FFmpeg dekóder eredményét ugyanazon referenciaanyagokon kell összehasonlítani.

Elvárás:
- bitazonosság, ahol az algoritmus és a formátum ezt garantálja;
- megfelelő toleranciával végzett pixel-/hangminta-összehasonlítás más esetekben;
- felbontás, krominancia és bitmélység megőrzése;
- időbélyeg- és mintaszámhelyesség;
- dekódolási és export-regresszió kizárása.

### CI

Kötelező ellenőrzési csoportok:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo xtask layers`
- `cargo xtask assets`
- `cargo deny check licenses sources advisories`
- FFmpeg 9.0.2 adaptertesztek
- Qt6 integrációs tesztek
- natív export/reimport
- csomagolási tesztek

A teljes vizsgálat az aktuális CFA4 CI-szabályok szerint indítandó. Az FFmpeg-et igénylő tesztek és a FFmpeg nélküli build külön ellenőrzési konfigurációban fussanak.

A Smart Cut/Smart Render tesztjeinek FFmpeg nélkül is sikeresen le kell futniuk, a külön CI-orákulumokat kivéve.

---

## 14. Végrehajtási sorrend és függőségek

| Sorrend | Munkacsomag | Előfeltétel |
|---|---|---|
| 1 | FF-00 szabályok | ADR 0005 rendezése |
| 2 | FF-01 wrapper-audit | FF-00 |
| 3 | FF-02 adapter alap | FF-01 |
| 4 | FF-03 worker | FF-02 |
| 5 | FF-04 import/decode | FF-03 |
| 6 | FF-05 route | FF-04 |
| 7 | FF-06 encode/GPU | FF-05 |
| 8 | FF-07 export | FF-06 |
| 9 | FF-08 packaging | Stabil backend |
| 10 | FF-09 E2E | FF-04–FF-08 |

Az FF-06 egyes kodekfeladatai önálló PR-ekben párhuzamosíthatók, miután az egységes adapter stabil.

### Kapcsolat a folyamatban lévő fejlesztésekkel

- #224: médiaháttér szabályrendszer.
- #228/#229: Smart Cut/Stream Copy terveinek összehangolása.
- #215/#216/#219: natív Smart Edit munkacsomagok.
- Qt6-paritás: a backend nem késleltetheti a már folyó UI-migrációt.

Az FFmpeg-integráció nem blokkolhatja a natív Smart Edit munkát.

---

## 15. Visszaállítás és kompatibilitás

A régi rendszer minden szakaszban működőképes marad.

Kötelező kiadási állapotok:

1. FFmpeg nélküli fejlesztői build.
2. FFmpeg-backenddel rendelkező build.
3. FFmpeg-backendet letiltó konfiguráció.
4. GStreamerrel rendelkező és GStreamer nélküli futtatás.
5. CPU-only üzemmód.
6. GPU-gyorsított üzemmód.
7. Hibás/hiányzó FFmpeg worker kezelése.

Ha egy FFmpeg-funkció regressziót okoz, backend-szinten letiltható legyen a teljes CFA4 médiamotor visszaállítása vagy újraírása nélkül.

---

## 16. Végső elfogadási kapuk

Az integráció csak akkor jelölhető befejezettnek, ha:

- [ ] Az ADR- és fejlesztői szabályok ütközésmentesek.
- [ ] Egyetlen, jóváhagyott Rust–FFmpeg kötés van kiválasztva.
- [ ] Az FFmpeg 9.0.2 verzió- és ABI-ellenőrzés igazolt.
- [ ] A licencek és a Cargo-függőségek megfelelnek.
- [ ] FFmpeg nélkül is működik a CFA4 alapbuild.
- [ ] A worker hibája nem dönti meg a főalkalmazást.
- [ ] Az FFmpeg a meglévő `cfa4-route` rendszerhez kapcsolódik.
- [ ] A GStreamer és az FFmpeg fallbackje nem ütközik.
- [ ] A professzionális profilok bitmélysége és metaadatai megmaradnak.
- [ ] A meglévő Rust dekódolók és enkóderek regressziómentesek.
- [ ] A natív Smart Cut, Stream Copy és Smart Render független marad.
- [ ] A Qt6 import/export munkafolyamat igazolt.
- [ ] A DEB, Flatpak és AppImage ellenőrzött.
- [ ] A teljes CI és célgépes E2E validáció sikeres.

---

## 17. Záró műszaki döntési javaslat

A CFA4-be **egyetlen FFmpeg 9.0.2 backend** kerüljön, `cfa4-ffmpeg` néven.

Ennek részei:

1. Az elsődleges saját CFA4 médiamotor megtartása.
2. FFmpeg-kötés kiválasztása `ffmpeg-next` és `rsmedia/rsmpeg` összehasonlításával; előre kijelölt elsődleges jelölt nélkül (tulajdonosi döntés, 2026-10-27).
3. FFmpeg C-könyvtárak elkülönített, biztonságos végrehajtási környezete.
4. Beillesztés a meglévő `cfa4-route` rendszerbe.
5. A natív médiaképességek hiányainak célzott kiegészítése.
6. Professzionális kodekprofilok és gyártófüggetlen hardveres gyorsítás bővítése.
7. A teljes Smart Cut/Smart Render saját Rust-végrehajtásának megőrzése.
8. Reprodukálható, rendszerfüggetlen FFmpeg-csomagolás.
9. Bizonyítékalapú integráció külön PR-ekkel és elfogadási kapukkal.

**A bevezetés első kötelező lépése az FF-00 és FF-01.** Az API-választás, a licenc és a natív FFmpeg-könyvtárak kezelése addig nem tekinthető lezártnak.

**A végső cél nem az FFmpeg beépítése önmagáért, hanem az, hogy a CFA4 a saját médiamotorjának megtartásával hozzáférjen azokhoz a professzionális FFmpeg-képességekhez, amelyekből ténylegesen hiánya van.**
