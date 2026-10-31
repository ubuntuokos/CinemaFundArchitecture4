# CFA4-DCP-VALIDATION-001
## Cinema DCP Validation & Delivery – megvalósítási terv

**Célprojekt:** ubuntuokos/CinemaFundArchitecture4  
**Referencia:** wolfgangw/backports – dcp_inspect  
**Platform:** Ubuntu 26.04 LTS és variánsai, Linux amd64  
**Architektúra:** Rust/Cargo + Qt6 + FFmpeg 9.0.2  
**Állapot:** Megvalósítási terv; nem végrehajtott fejlesztés  
**Verzió:** 1.0 – 2026-10-27

> **Repó-megjegyzés (2026-10-27):** a tulajdonos által beküldött terv változatlan szövege, hogy a
> `VISION.md` 2.6. pontja szerint a `main` ágon éljen. Az első szakasz auditjai:
> [DCP-00 képességaudit](../audit/DCP-00-CAPABILITY-AUDIT-2026-10-27.md),
> [DCP-01 forrás- és licencaudit](../audit/DCP-01-SOURCE-LICENSE-AUDIT-2026-10-27.md),
> fejlesztői környezet: [`docs/development/dcp-validation.md`](../development/dcp-validation.md).
> A DCP-02-től kezdődő fejlesztés az auditok tulajdonosi jóváhagyása után indul. Az upstream
> projekt nevét az `AGENTS.md` márkaszabálya miatt (`python/tests/test_independence.py`) az „upstream” szó helyettesíti.

---

# 1. Projektcél és hatókör

A CFA4 rendelkezzen saját, integrált DCP-ellenőrző és moziforgalmazási előkészítő alrendszerrel.

A rendszernek alkalmasnak kell lennie:

1. Meglévő DCP-csomagok betöltésére és elemzésére.
2. SMPTE és Interop DCP-formátumok ellenőrzésére.
3. A CFA4-ből exportált DCP-k automatikus ellenőrzésére, amennyiben a szükséges DCP-exportképesség rendelkezésre áll.
4. A DCP technikai, szerkezeti, tartalmi és biztonsági hibáinak kimutatására.
5. A hibák CFA4 Qt6 grafikus felületén történő megjelenítésére.
6. Javítható problémákhoz kontextusfüggő javítási műveletek biztosítására.
7. Strukturált, archiválható validációs jelentések előállítására.
8. A jövőbeni natív Rust-validációs motor fokozatos bevezetésére.

A teljes DCP-generálás, a KDM-kibocsátás és a moziszerverre történő kiszállítás külön képesség. Ezekkel integrációs pontokat kell kialakítani, de önmagukban nem részei a dcp_inspect átvételének.

# 2. Kötelező architekturális szabályok

- Az upstream (a CFA4 kiindulópontjául szolgáló, `NOTICE`-ban megnevezett projekt) eredeti forrását nem módosítjuk visszamenőleg.
- Minden fejlesztés kizárólag a CFA4-be kerül.
- A már meglévő CFA4-modulokat a fejlesztés előtt auditálni kell.
- A meglévő implementációt csak akkor szabad kiváltani, ha az új megoldás ellenőrzötten jobb vagy hiányzó képességet biztosít.
- A CFA4 elsődleges fejlesztési nyelve Rust.
- A natív asztali felület Qt6.
- A Ruby kizárólag izolált ellenőrzőmotorként, átmeneti vagy opcionális komponensként használható.
- Nem használható Conda vagy Mamba.
- Nem vezethető be Vercel-függőség.
- A meglévő FFmpeg 9.0.2 környezetet meg kell őrizni.
- CPU-only működés kötelező.
- GPU-gyorsítás csak opcionális lehet.
- Az alkalmazás nem változtathatja meg más multimédiás programok telepítését, függőségeit vagy működését.
- A rendszernek az internet elérése nélkül is képesnek kell lennie a helyi DCP-ellenőrzésre.
- A validációs motor alapértelmezésben csak olvasási jogosultságot kap a vizsgált médiára.
- A DCP eredeti állományait ellenőrzés közben megváltoztatni tilos.

# 3. Tervezett modularchitektúra

## 3.1. Logikai felépítés

```text
CFA4
│
├── Qt6 Desktop GUI
│   └── Cinema Delivery / DCP Validation
│       ├── DCP Browser
│       ├── Preflight Dashboard
│       ├── Composition Inspector
│       ├── Media Inspector
│       ├── Subtitle Inspector
│       ├── Audio Inspector
│       ├── Security Inspector
│       └── Validation Reports
│
├── CFA4 Rust Engine
│   └── Cinema Validation Service
│       ├── Job Manager
│       ├── Validation Profiles
│       ├── Result Normalizer
│       ├── Findings Engine
│       ├── Report Generator
│       └── Repair Action Controller
│
├── Validation Backends
│   ├── Native Rust Backend
│   └── Isolated dcp_inspect Backend
│
└── Existing CFA4 Services
    ├── MXF
    ├── Export
    ├── Codecs
    ├── Audio DSP
    ├── Captions
    ├── Timeline
    └── Media Browser
```

## 3.2. Tervezett Rust-crate-ek

Új modulok:

- `crates/dcp-validation/` – a fő validációs API és a vizsgálati szabályok.
- `crates/dcp-model/` – a DCP-adatmodell.
- `crates/dcp-inspect-adapter/` – a Ruby-motor illesztése.
- `crates/dcp-report/` – a jelentések előállítása.

A végleges könyvtárelhelyezést a meglévő workspace-konvenciók ellenőrzése után kell rögzíteni. A felsorolt könyvtárak tervezett, nem már létező állományok.

## 3.3. Egységes backend-interfész

A CFA4 különítse el az ellenőrzési szabályokat a konkrét ellenőrzőmotoroktól.

Javasolt logikai interfész:

```rust
pub trait DcpValidator {
    fn capabilities(&self) -> ValidatorCapabilities;

    fn validate(
        &self,
        request: ValidationRequest,
        control: ValidationControl,
    ) -> Result<ValidationReport, ValidationError>;
}
```

A `ValidationControl` biztosítja a megszakítási és előrehaladási mechanizmusok átadását. A hosszú műveleteket aszinkron munkakezelő hajtja végre, nem a Qt fő szálán.

A backendeknek azonos jelentésformátumot kell előállítaniuk.

# 4. Funkcionális megvalósítási katalógus

## DCP-F01 – DCP-felfedezés

- DCP-k keresése könyvtárstruktúrában.
- Több DCP egyidejű azonosítása.
- ASSETMAP és ASSETMAP.xml felismerése.
- DCP-gyökérkönyvtár ellenőrzése.
- Hiányzó állományok azonosítása.
- Szimbolikus linkek biztonságos kezelése.
- Önálló DCP és többcsomagos asset-store vizsgálat.

## DCP-F02 – ASSETMAP-validáció

- XML-sémavalidáció.
- Asset-azonosítók ellenőrzése.
- Duplikált asset-azonosítók felismerése.
- Állományhivatkozások ellenőrzése.
- Fájlelérhetőség ellenőrzése.
- Az ismert, de még nem implementált AssetMap-funkciók külön jelzése.

## DCP-F03 – PKL-validáció

- Packing List XML-ellenőrzés.
- Asset ID és típus egyeztetése.
- Fájlméret-ellenőrzés.
- Hash-ellenőrzés.
- PKL és ASSETMAP konzisztenciája.
- Többszörösen deklarált állományok kezelése.
- Hiányzó vagy hibás PKL-bejegyzések felismerése.

## DCP-F04 – CPL-validáció

- Composition Playlist XML-ellenőrzése.
- CPL-azonosítók vizsgálata.
- Reel-azonosítók vizsgálata.
- Reel-sorrend ellenőrzése.
- EntryPoint, Duration és IntrinsicDuration vizsgálata.
- Kép-, hang- és feliratsávok időbeli összhangjának ellenőrzése.
- Rational frame-rate értékek pontos összehasonlítása.
- Hiányzó vagy hibás erőforrás-hivatkozások felismerése.

## DCP-F05 – SMPTE és Interop profilok

- SMPTE/Interop csomagok felismerése.
- Profilfüggő validáció.
- Külön szabályazonosítók.
- Profilfüggő kompatibilitási jelentések.
- Nem támogatott kiterjesztések explicit jelzése.
- Az alkalmazott szabványverziók dokumentálása.

## DCP-F06 – MXF-validáció

- MXF-fejlécek elemzése.
- Track-azonosítók vizsgálata.
- Essence típusának ellenőrzése.
- Frame-rate és időtartam ellenőrzése.
- Kép- és hangsávok műszaki összevetése.
- MXF-metaadatok és CPL-deklarációk egyeztetése.
- A meglévő CFA4 MXF-képességek újrahasznosítása.

## DCP-F07 – JPEG2000-validáció

- JPEG2000 codestream-fejlécek vizsgálata.
- Cinema profile követelmények ellenőrzése.
- SIZ-paraméterek vizsgálata.
- Képméretek összevetése az MXF-adatokkal.
- Tile-struktúra ellenőrzése.
- Komponensprecizitás vizsgálata.
- Progression és coding paraméterek ellenőrzése.
- Tile-part határok és hosszadatok ellenőrzése.
- Nem támogatott bitstream-részletek külön kimutatása.

A teljes JPEG2000 dekódolás és pixelpontos képellenőrzés nem keverhető össze a codestream szerkezeti validációjával.

## DCP-F08 – Képformátumok és képkockasebesség

- Natív felbontás ellenőrzése.
- Képarány és ScreenAspectRatio egyeztetése.
- Stereoscopic frame-rate szabályok vizsgálata.
- Az alkalmazott SMPTE/Interop profilok frame-rate követelményeinek ellenőrzése.
- A nem szabványos sebességek megkülönböztetése az opcionális profiloktól.
- Figyelmeztetés a célmozival kapcsolatos kompatibilitási kockázatokra.

## DCP-F09 – Audio-validáció

- PCM-formátum ellenőrzése.
- Mintavételezési frekvencia és bitmélység.
- Csatornaszám.
- MCA metaadatok.
- Hangsávok időtartama.
- Hiányzó hangsávok.
- Reel-határok hangfolytonossága.
- Csatorna-hozzárendelések.
- Nem értelmezhető csatornakiosztások jelzése.

## DCP-F10 – Loudness-ellenőrzés

- EBU R128 mérés.
- Integrált LUFS.
- Loudness range.
- True peak.
- Teljes kompozícióra vonatkozó mérés.
- Reel-sorrend és vágási tartomány figyelembevétele.
- A nem programszintű csatornák megfelelő elkülönítése.
- Mérési eredmények tárolása.

Az elfogadási küszöböket külön exportprofilhoz kell kötni. Nem szabad egyetlen általános LUFS-értéket minden mozis tartalomra kötelezőnek tekinteni.

## DCP-F11 – IAB/Atmos-validáció

- IAB-essence azonosítása.
- IAFrame struktúra vizsgálata.
- Object és bed definíciók.
- AudioData referenciák.
- Objektumazonosítók.
- Strukturális konzisztencia.
- Az ellenőrzött és nem ellenőrzött részek külön kimutatása.

A teljes objektumalapú hangrenderelés későbbi, külön implementációs feladat.

## DCP-F12 – Feliratellenőrzés

- Interop DCSubtitle.
- SMPTE timed text.
- XML-sémavalidáció.
- Felirat-időzítés.
- Fade-in és fade-out időtartamok.
- Reel-határok.
- Szövegblokkok sorrendje.
- Betűkészlet-hivatkozások.
- Szükséges glyph-ek megléte.
- PNG-feliraterőforrások.
- Hiányzó feliraterőforrások.
- Felirat és CPL közötti összhang.
- A tényleges feliratmegjelenítés validációjának elkülönítése a szerkezeti ellenőrzéstől.

## DCP-F13 – ISDCF elnevezések

- ContentTitleText feldolgozása.
- ISDCF-regiszteren alapuló mezőfelismerés.
- Formátum és felbontás.
- Frame-rate állítások.
- OV/VF megjelölések.
- Verzió- és nyelvjelölések.
- Az elnevezésből következtethető és a bizonyított műszaki tulajdonságok összevetése.
- Elnevezési javaslat előállítása a CFA4 exportbeállításai alapján.

A név önmagában nem igazolja a tényleges fájltartalmat.

## DCP-F14 – Digitális aláírások

- CPL-aláírás ellenőrzése.
- PKL-aláírás ellenőrzése.
- Kriptográfiai érvényesség.
- Aláírói identitás.
- Tanúsítványlánc szerkezete.
- Certificate role és CommonName ellenőrzés.
- Content Signer szerepkörök.
- Hiányzó aláírások profilfüggő minősítése.
- Külső bizalmi státusz elkülönítése a kriptográfiai ellenőrzéstől.

## DCP-F15 – KDM-kapcsolódás

Első fázis:

- A titkosított tartalom felismerése.
- KeyId-hivatkozások megjelenítése.
- Nem dekódolható essence megjelölése.
- A KDM-ellenőrzés hiányának egyértelmű jelzése.

Későbbi, külön engedélyezendő fázis:

- KDM/XML ellenőrzés.
- Érvényességi intervallum.
- Célkészülék és tanúsítvány összerendelése.
- KeyId és CPL kapcsolatának ellenőrzése.
- Biztonságos kulcskezelési interfész.

A KDM-integrációhoz nem szabad a titkosítási kulcsokat általános logfájlba vagy hibajelentésbe írni.

## DCP-F16 – Composition Markers

- Standard markerek felismerése.
- Reel-pozíciók.
- Összesített kompozíciós időpontok.
- Duplikált markerek.
- Érvénytelen sorrendek.
- Credits és kompozícióhatár-jelölések.
- A CFA4 idővonaljelölőivel történő megfeleltetés.

## DCP-F17 – Integritásvizsgálat

- Fájlméretek.
- Hash-értékek.
- Hiányzó assetek.
- Hibás assetek.
- PKL és CPL hivatkozások.
- Több PKL-ben szereplő azonos assetek.
- Részleges ellenőrzések jelentése.
- Megszakított vizsgálatok állapotának megőrzése.

## DCP-F18 – Jelentésgenerálás

Kötelező formátumok:

- JSON – géppel feldolgozható.
- Markdown – fejlesztői és auditdokumentáció.
- HTML – emberi olvasásra alkalmas.

A PDF-export külön jelentésmegjelenítő útvonalon is megvalósítható.

A jelentéseknek tartalmazniuk kell:

- Ellenőrzési profil.
- Ellenőrzőmotor és verzió.
- Ellenőrzött DCP azonosítója.
- CPL/PKL/ASSETMAP azonosítók.
- Ellenőrzött assetek.
- Hibák.
- Figyelmeztetések.
- Nem végrehajtott vizsgálatok.
- Az egyes eredmények bizonyítékai.
- Összesített eredmény.
- Az ellenőrzés időpontja.

## DCP-F19 – CFA4 GUI

Önálló Qt6-felület:

**Cinema Delivery → DCP Validation**

Fő képernyőrészek:

1. DCP Browser.
2. Package Tree.
3. Composition Overview.
4. Findings.
5. Audio / Video / Subtitle részletes adatok.
6. Security / Certificates.
7. Validation Profile Selector.
8. Report Export.

Minden hiba mellett megjelenjen a hiba pontos helye, az ellenőrzési szabály azonosítója és a javításhoz szükséges információ.

## DCP-F20 – CFA4 exportintegráció

- DCP-előkészítési ellenőrzés export előtt.
- Végleges csomag validációja export után.
- Állapot visszajelzése az Export Manager számára.
- Sikertelen validáció esetén hibajelentés.
- A korábbi érvényes exportok megőrzése.
- Javítás után új validáció.
- Nem támogatott exportfunkciók explicit kezelése.

A validáció megléte nem igazolja automatikusan, hogy a CFA4 már teljes DCP-generáló rendszerrel rendelkezik.

## DCP-F21 – Javítási műveletek

A CFA4 különböztesse meg:

- Automatikusan javítható projekt-metaadat.
- Újraexportálással javítható tartalom.
- Felhasználói döntést igénylő eltérés.
- Külső eszközt vagy tanúsítványt igénylő probléma.
- Nem támogatott ellenőrzés.

Automatikus módosítás kizárólag a CFA4 saját projektadatain vagy felhasználó által jóváhagyott munkapéldányon történhet.

Az eredeti DCP közvetlen módosítása nem lehet alapértelmezett.

# 5. Validációs profilok

A következő profilok szükségesek:

| Profil | Cél |
|---|---|
| Quick Scan | Gyors szerkezeti ellenőrzés |
| Full Integrity | Teljes fájlhash és kapcsolati ellenőrzés |
| SMPTE Cinema | SMPTE-profilfüggő szabályok |
| Interop Cinema | Interop-profilfüggő szabályok |
| Subtitle & Accessibility | Felirat és kapcsolódó erőforrások |
| Audio Quality | Hangadatok és loudness |
| Security | Aláírások és tanúsítványok |
| Delivery Preflight | Export utáni összetett ellenőrzés |

A profilok szabálykészlete verziózott és bővíthető legyen.

A `Quick Scan` nem kaphat teljes integritást igazoló eredményt, ha például a fájlok tartalmi hash-ellenőrzése kimaradt.

# 6. Ellenőrzési eredmények modellje

Kötelező eredményállapotok:

- PASS – ellenőrzés sikeres.
- FAIL – igazolt hiba.
- WARNING – figyelmeztetés.
- SKIPPED – szándékosan kihagyott vizsgálat.
- UNCHECKED – nem volt megvizsgálható.
- NOT_APPLICABLE – nem vonatkozik a csomagra.
- INTERRUPTED – megszakított ellenőrzés.

Az egész csomagra vonatkozó összesítés külön mező legyen.

A `PASS` csak meghatározott profilra és annak teljes, kötelező ellenőrzési halmazára érvényes.

A hiányos vizsgálat nem minősíthető automatikusan hibátlannak.

# 7. dcp_inspect illesztése

## 7.1. Kezdeti backend

A Ruby-alapú ellenőrzőmotor külön folyamatként működjön.

Alapfunkciók:

- Elérhetőségvizsgálat.
- Verzióazonosítás.
- Folyamatindítás.
- Kilépési kód feldolgozása.
- Strukturált JSON fogadása.
- Időkorlát.
- Felhasználói megszakítás.
- Ideiglenes állományok biztonságos kezelése.
- Szabályozott erőforrás-felhasználás.

A CFA4 Qt6-felülete semmilyen Ruby-függőséget ne tartalmazzon közvetlenül.

## 7.2. Függőségek

Auditálandó:

- Ruby-runtime.
- Nokogiri/libxml2.
- TTFunk.
- OpenSSL.
- asdcplib.
- FFmpeg 9.0.2.
- Az EBU R128 és astats szűrők.
- fd/fdfind opcionális fájlfelfedezéshez.
- XSD-katalógusok.

A szükséges programok verzióit és tényleges futásidejű kapcsolatait külön ellenőrizni kell.

## 7.3. Kompatibilitás

- A CFA4 saját telepítési prefixet vagy izolált erőforrás-elhelyezést használjon.
- Más rendszerszintű Ruby-alkalmazások csomagjait ne módosítsa.
- Ne írjon globális GEM-környezetbe a felhasználó külön engedélye nélkül.
- A feldolgozás működjön Linux amd64 alatt.
- A szükséges erőforrásokat a .deb, Flatpak és AppImage csomagolási módszerhez külön kell összeállítani.
- A Flatpak-sandbox jogosultságait dokumentálni kell.

# 8. Natív Rust-migráció

A natív Rust-megvalósítás modulonként történjen.

Javasolt sorrend:

1. DCP-felfedezés és fájlkezelés.
2. ASSETMAP/PKL/CPL adatmodell.
3. XML- és XSD-validáció.
4. Assetkapcsolatok és integritásvizsgálat.
5. MXF-metaadatok ellenőrzése.
6. Reel- és idővonal-validáció.
7. JPEG2000 szerkezeti ellenőrzés.
8. Hang- és feliratintegráció.
9. ISDCF és ContentTitleText.
10. Digitális aláírások és tanúsítványok.
11. IAB-ellenőrzés.
12. Teljes profilparitási tesztelés.

Minden átállított funkció esetén azonos tesztcsomagon kell összevetni:

- A referencia dcp_inspect eredményét.
- A CFA4 natív Rust eredményét.
- Az alkalmazott szabványt.
- A felismert hibákat.
- A kihagyott vizsgálatokat.

A régi backend csak akkor kapcsolható ki egy funkciónál, ha a natív helyettesítés elfogadási tesztjei sikeresek.

# 9. Biztonság

A DCP-k külső eredetű, potenciálisan hibás vagy rosszindulatú állományokat tartalmazhatnak.

Kötelező védelmek:

- Olvasásra korlátozott bemeneti hozzáférés.
- Kimenet elkülönítése a DCP könyvtárától.
- Fájlelőfordulási és méretkorlátok.
- XML-entitáskezelési védelem.
- XML bomb és túlzott erőforrás-igény elleni védelem.
- Elérésiút-normalizálás.
- Symlink- és útvonalkitörés elleni védelem.
- Külső folyamatok argumentumainak biztonságos kezelése.
- Erőforráslimitek.
- Ideiglenes állományok törlése.
- Titkos anyagok és tanúsítványadatok védelme.
- A hibásan formázott médiától függetlenül stabil Qt6-felület.

A Ruby-folyamatot nem szabad emelt rendszerjogosultsággal futtatni.

# 10. Licenc- és eredetellenőrzés

A fejlesztés előtt:

1. Rögzíteni kell a referencia Git-commitot.
2. Ellenőrizni kell minden átvenni kívánt fájl licencét.
3. Ellenőrizni kell az XSD-k eredetét és terjesztési feltételeit.
4. Azonosítani kell a külön licencű komponenseket.
5. Dokumentálni kell a harmadik féltől származó függőségeket.
6. El kell készíteni a csomagonkénti licencegyeztetést.

A `backports` fő forrásában GPL-licencfájl található, továbbá elkülönített, más feltételekkel rendelkező komponens is szerepel.

A CFA4-ben történő közvetlen forráskód-átvétel csak a licencellenőrzés után engedélyezhető.

# 11. Fejlesztési munkacsomagok

| ID | Munkacsomag | Kötelező eredmény |
|---|---|---|
| DCP-00 | CFA4 képességaudit | Meglévő és hiányzó funkciók leltára |
| DCP-01 | Forrás- és licencaudit | Jóváhagyható függőségi jegyzék |
| DCP-02 | DCP-adatmodell | Rust modellek és szerializáció |
| DCP-03 | Backend-interfész | Egységes Rust validation API |
| DCP-04 | Ruby-adapter | Izolált dcp_inspect futtatás |
| DCP-05 | Alapvető DCP-validáció | ASSETMAP, PKL, CPL, assetek |
| DCP-06 | MXF/JPEG2000-integráció | Kép- és médiavizsgálat |
| DCP-07 | Hang/IAB-integráció | PCM, loudness, IAB |
| DCP-08 | Felirat-integráció | Interop/SMPTE subtitle validation |
| DCP-09 | ISDCF-integráció | Elnevezési állítások ellenőrzése |
| DCP-10 | Biztonsági validáció | Aláírások és tanúsítványok |
| DCP-11 | Profilkezelés | Verziózott szabálykészletek |
| DCP-12 | Qt6-felület | Teljes DCP Validation panel |
| DCP-13 | Exportintegráció | Preflight és postflight |
| DCP-14 | Javítási vezérlő | Kontrollált javítási javaslatok |
| DCP-15 | Jelentésgenerálás | JSON/Markdown/HTML |
| DCP-16 | Teszt- és referenciaadatok | Pozitív és negatív DCP-fixture-ök |
| DCP-17 | Rust-paritás | Natív funkciók összehasonlítása |
| DCP-18 | Linux-csomagolás | .deb/Flatpak/AppImage ellenőrzése |
| DCP-19 | Release Gate | Integrált átvételi tesztcsomag |

## Végrehajtási sorrend

**Első szakasz – Alapozás**

DCP-00 → DCP-01 → DCP-02 → DCP-03 → DCP-04

**Második szakasz – Ellenőrzési képességek**

DCP-05 → DCP-06 → DCP-07 → DCP-08 → DCP-09 → DCP-10

Az egyes, egymástól független ellenőrző modulok párhuzamosan is fejleszthetők, miután a közös adatmodell és eredményinterfész stabil.

**Harmadik szakasz – CFA4-integráció**

DCP-11 → DCP-12 → DCP-13 → DCP-14 → DCP-15

**Negyedik szakasz – Validáció és kiadás**

DCP-16 → DCP-17 → DCP-18 → DCP-19

A tesztadatok előállítását már az első szakaszban el kell kezdeni. A táblázat a fő fejlesztési sorrendet, nem a tesztelés legkorábbi kezdőpontját jelöli.

# 12. Tesztelési terv

Kötelező tesztesetek:

- Érvényes SMPTE DCP.
- Érvényes Interop DCP.
- Hibás ASSETMAP.
- Hiányzó PKL.
- Hibás CPL-referencia.
- Sérült XML.
- Hiányzó MXF.
- Hibás hash.
- Hibás reel-időtartam.
- Eltérő képkockasebesség.
- Érvénytelen JPEG2000-fejléc.
- Hiányzó hangtartalom.
- Hibás csatornák.
- Hibás felirat-időzítés.
- Hiányzó font.
- Hibás PNG-feliraterőforrás.
- Érvénytelen digitális aláírás.
- Hibás tanúsítványidentitás.
- Titkosított DCP.
- Több CPL-t tartalmazó csomag.
- Több PKL-t tartalmazó csomag.
- Duplikált assetek.
- Megszakított ellenőrzés.
- Nagyméretű DCP.
- Hibás fájlelérési utak.
- Jogosultsági hibák.
- Sérült vagy szándékosan manipulált bemenet.

A működésellenőrzésnek a következőket is tartalmaznia kell:

- Qt6 GUI-paritás.
- CLI-paritás.
- CPU-only futtatás.
- Offline működés.
- Csomagolási tesztek.
- Erőforrásszivárgás-vizsgálat.
- Host-alkalmazásokkal való együttélés.
- Ismert eredményű DCP-fixture-ökön végrehajtott regressziós teszt.

# 13. CI/CD és kiadási feltételek

A CFA4 GitHub Actions rendszeréhez új, elkülönített DCP-validation workflow szükséges.

A CI ellenőrizze:

- Rust build és clippy.
- Unit tesztek.
- Integrációs tesztek.
- JSON-séma megfelelősége.
- XSD-katalógus ellenőrzése.
- Referenciamotor paritása.
- Hibás médiára vonatkozó negatív tesztek.
- Linux-csomagolási ellenőrzés.
- Licenc- és függőségi megfelelőség.
- Nem várt külső állományírás hiánya.

A fejlesztési PR csak akkor egyesíthető, ha a kapcsolódó kötelező tesztek sikeresek.

A DCP-funkciók hibája nem eredményezheti a CFA4 meglévő videószerkesztő funkcióinak regresszióját.

# 14. Átvételi kritériumok

A teljes DCP Validation integráció akkor tekinthető megvalósítottnak, ha:

1. A CFA4 önálló DCP Validation menüponttal rendelkezik.
2. A Qt6-felületről DCP-csomag betölthető.
3. SMPTE és Interop csomagok vizsgálhatók.
4. Az ellenőrzések eredménye strukturáltan megjelenik.
5. Az ASSETMAP, PKL, CPL és MXF kapcsolatellenőrzések működnek.
6. A hang-, kép-, felirat- és biztonsági vizsgálatok dokumentált lefedettséggel rendelkeznek.
7. A nem ellenőrzött tulajdonságok nem jelennek meg sikeres validációként.
8. Az eredmények exportálhatók.
9. Az ellenőrzések megszakíthatók.
10. Az eredeti DCP-állományok változatlanok maradnak.
11. A Linux telepítőcsomagok kezelik a szükséges függőségeket.
12. Nem történik globális host-függőségmódosítás.
13. A CI és regressziós tesztek sikeresek.
14. A licencmegfelelőség dokumentált.
15. A natív Rust és a referenciamotor közötti eltérések dokumentáltak.
16. Az alkalmazott validációs profil és szabványverzió visszakereshető.

# 15. Kockázatok és blokkolók

| Kockázat | Kötelező kezelés |
|---|---|
| GPL és egyéb licencek | Forrásszintű megfelelőségi audit |
| Ruby runtime | Izolált futtatás és csomagolás |
| XSD-kompatibilitás | Sématesztek és verziókövetés |
| Eltérő DCP-profilok | Profilfüggő szabályok |
| Nagy fájlméretek | Streamelt ellenőrzés |
| Hiányos JPEG2000-validáció | Lefedettség külön jelentése |
| Titkosított tartalom | UNCHECKED és KDM-korlátok |
| IAB-részleges lefedettség | Strukturális vizsgálat elkülönítése |
| Qt6-felület blokkolása | Aszinkron munkakezelés |
| MXF-funkciók duplikációja | Meglévő CFA4-modulok auditja |
| Harmadik fél függőségei | Rögzített verziók, izoláció |
| Hibás automatikus javítás | Csak jóváhagyott munkapéldány módosítható |

# 16. Végleges architekturális döntés

A CFA4 a `wolfgangw/backports` projektet referencia-ellenőrzőmotorként és lehetséges ideiglenes backendként használja.

A végleges cél egy natív, CFA4-be integrált DCP Validation Service, amely:

- A CFA4 saját Rust-motorjához kapcsolódik.
- A meglévő MXF-, audio-, felirat- és exportképességeket használja.
- Qt6 grafikus kezelőfelületet biztosít.
- Offline, Linux amd64 rendszeren működik.
- Nem avatkozik be a host alkalmazásaiba.
- Elkülönített validációs profilokkal működik.
- Reprodukálható és auditálható eredményeket szolgáltat.
- Megőrzi a kompatibilitást a CFA4 további fejlesztéseivel.

**A terv megvalósítása kizárólag a CinemaFundArchitecture4 projektet érinti. Az upstream változatlan marad.**

**Végállapot:** egyetlen CFA4-be integrált Cinema DCP Validation & Delivery alrendszer, egységes adatmodellel, GUI-val, ellenőrzőmotorral, riportálással és automatizált validációs tesztekkel.
