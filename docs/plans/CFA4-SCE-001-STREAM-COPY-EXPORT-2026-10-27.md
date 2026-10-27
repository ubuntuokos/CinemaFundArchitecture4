# CFA4-SCE-001 v1.5: Stream Copy export, MPEG-TS kimenet, GOP Smart Cut és Smart Render

**Dátum:** 2026-10-27  
**Státusz:** végleges, a tulajdonos által jóváhagyva (2026-10-27); implementáció a Qt port elkészülte után  
**Kapcsolódó tervek:** [GOP-alapú Smart Cut](GOP-SMART-CUT-2026-10-27.md), [Smart Render](SMART-RENDER-2026-10-27.md), `CFA4-SMART-CUT-RENDER-R1.md` (ág: `feature/native-smart-cut-render`)

> Ez a fájl a jóváhagyott terv repóba emelt példánya. A *[vX.Y]* jelölések a terv változásait követik.

Ez a v1.1 Stream Copy terv és az MPEG-TS muxer terv összevont változata. A TS terv új, 12. fejezetként került be a bitstream filter szabályok után, mert a TS kimenet a Compatible Stream Copy elsődleges célkonténere, és a render export is használja. A későbbi fejezetek számozása eggyel nőtt. A v1.2-ben módosított vagy új sorokat *\[v1.2\]* jelöli. A tartalmi ütközéseket a dokumentum végén soroltam fel. A v1.3 a natív Rust TS muxerről szóló döntést (2026-10-27) vezeti át; ezeket a részeket \[v1.3\] jelöli. A v1.4 a repo GOP Smart Cut és Smart Render tervét építi be (13., 15. és 22. fejezet), jelölése \[v1.4\].

A v1.5 a tulajdonos 2026-10-27-i döntéseit vezeti át (lásd a Döntési naplót); az érintett részeket \[v1.5\] jelöli.

**Dokumentumazonosító:** CFA4-SCE-001 · **Verzió:** 1.5 (végleges) · **Repository:** https://github.com/ubuntuokos/CinemaFundArchitecture4 · **Célplatform:** Ubuntu 26.04 LTS, amd64 · **Technológiai alapok:** Rust/Cargo, Qt6 (FFmpeg 9.0.2 csak CI-teszt-orákulum) · **Implementáció:** Stream Copy és TS még nem; Smart Cut/Render WP-00–WP-02 folyamatban, nem a main-ben.

## Döntési napló (v1.5)

A tulajdonos 2026-10-27-én az egyesített terv ellenőrzésének mind a hét javaslatát, majd a 8. pontot is elfogadta. Ezek a döntések felülírják a dokumentum korábbi, velük ellentétes mondatait.

1. **GOP Smart Cut helye.** A GOP Smart Cut a Smart Render egyklipes esete: ugyanaz a tervtípus és ugyanaz a végrehajtó. A Qt6 felületen a forrásmonitor „Forrás vágása” műveleteként jelenik meg, nem hatodik exportüzemmódként. (4., 13., 16. fejezet)
2. **Egyetlen Auto szabály.** Az Auto csak javasol, Strict Copy → Compatible Copy → Smart Render → Full Render sorrendben. Minden újrakódolást tartalmazó javaslat végrehajtásához kifejezett megerősítés kell, a jelentés szegmensenként mutatja a másolt és az újrakódolt részeket. Ez váltja ki a 4. fejezet, az R1 §14 és a Smart Render terv „nincs auto copy” szabályát. (4., 15., 16. fejezet)
3. **Terminológia.** „Smart Render” = idővonal-hibrid (szakaszonként másolás vagy újrakódolás). Az R1 `BoundaryReencode` módjának neve „határ-újrakódolás”; az R1 dokumentumot ennek megfelelően át kell nevezni. (8., 15. fejezet)
4. **Bitstream-adapterek.** Natív Rustban készülnek (Annex B ↔ AVCC/HVCC, ADTS ↔ nyers AAC, AUD). Az FFmpeg filternevei csak referencia-azonosítók, az FFmpeg csak CI-orákulum. (2.2, 11. fejezet)
5. **Rétegzett validáció.** A szerkezeti egyezés (`BITSTREAM_EQUIVALENT`) elég a PASS-hoz. Dekódolásos ellenőrzés csak ott fut, ahol a saját dekóder támogatja a profilt; máshol a jelentés „nem dekódolva ellenőrizve” jelzést kap. (10., 18. fejezet)
6. **Atomikus írás.** Közös segéd a `cfa4-export`-ban (ideiglenes fájl a célmappában, fsync, átnevezés). Minden muxer ezt használja, a közvetlen `File::create` megszűnik. (12.6/6, 20. fejezet)
7. **Határ-újrakódolás kodekjei.** R1-ben csak H.264. MPEG-2, HEVC (csak NVENC, gyártófüggő) és AV1 forrásnál a Smart Render kulcskockára kerekít vagy teljes újrakódolást javasol, amíg nincs gyártófüggetlen kódoló. (13., 15., 22. fejezet)
8. **Egységes parancsnévtér.** Minden exportmód az export.analyze, export.plan, export.start, export.cancel és export.validate parancsokon keresztül érhető el; a mód és a hatókör a terv paramétere. A meglévő export.formats, export.resolve és export.queue.\* megmarad. (17. fejezet)

**Véglegesítve 2026-10-27:** a tulajdonos a 8. döntést is elfogadta, és a tervet véglegesnek nyilvánította. A megvalósítás a Qt port elkészülte után indul, a 22. fejezet fázissorrendje szerint. Nyitva csak a repó oldali teendők maradnak: a két párhuzamos Smart Cut/Render terv összefésülése, az R1 dokumentum átnevezése („határ-újrakódolás”) és FFmpeg-utalásainak javítása.

## 1. Célkitűzés

A CFA4 natív exportmotorja két külön választható Stream Copy üzemmóddal bővül:

1. Strict Bit-Exact Stream Copy.
2. Compatible Stream Copy.

Mindkét üzemmód közös követelménye: **videó- vagy hangadatfolyam újrakódolása semmilyen körülmények között nem történhet.**

A két üzemmód közötti különbség nem a minőség, hanem a tömörített médiaadatok megengedett átalakításának mértéke.

A fejlesztés a CFA4 meglévő exportarchitektúrájába épül be, külön alkalmazás vagy párhuzamos multimédia-motor létrehozása nélkül. A két Stream Copy mód együttműködik a GOP Smart Cut, Smart Render és Full Render alrendszerekkel.

*\[v1.2\]* Az exportmotor szabványos MPEG-2 Transport Stream (`.ts`) és külön választható M2TS (`.m2ts`) kimenettel is bővül (12. fejezet). Ezt a Stream Copy és a render export egyaránt használja.

## 2. Végleges Stream Copy üzemmódok

### 2.1 Strict Bit-Exact Stream Copy

Belső azonosító: `stream_copy_strict`. Feladata a kiválasztott tömörített médiaadatok bitazonos megőrzése.

**Engedélyezett műveletek**

- Tömörített médiacsomagok másolása.
- Kimeneti konténer létrehozása.
- Konténerszintű metadata módosítása.
- Adatfolyamok kiválasztása.
- Adatfolyamok konténerszintű átrendezése.
- Időbélyegek átalakítása, amennyiben a csomagok tömörített tartalma változatlan marad.
- Biztonságos GOP-határokon történő vágás.
- Kompatibilis, változatlan tartalmú szakaszok összefűzése.
- A konténerspecifikus szerkezeti adatok létrehozása.

**Tiltott műveletek**

- Videó újrakódolása.
- Hang újrakódolása.
- Tömörített médiaadatok módosítása.
- NAL-unit tartalmának módosítása.
- Kodekparaméterek átírása a tömörített adatfolyamon belül.
- Bitstream filter használata, ha az megváltoztatja a másolandó tömörített tartalmat.
- Referenciaképek újragenerálása.
- Képkockák rekonstrukciója és újrakódolása.
- Automatikus Smart Renderre váltás.
- Automatikus Full Renderre váltás.

**Bitazonossági követelmény.** A bitazonosságot a demuxer által meghatározott, másolásra kijelölt tömörített csomagpayloadok szintjén kell értelmezni. A forráskonténer és a célkonténer teljes fájlszintű bitazonossága nem követelmény. A konténer fejlécének, indexeinek, időbélyegeinek, streamazonosítóinak és metadata-struktúrájának változása megengedett. A forrásból kiválasztott csomagok payload-bájtsorozatának azonban változatlannak kell maradnia.

A szigorú módhoz külön meg kell határozni a forrásadatfolyam reprezentációját. Nem tekinthető bitazonosnak egy olyan átalakítás, amely a csomagpayloadban például start code-okat vagy NAL-hosszmezőket módosít.

*\[v1.2\]* Ebből következik, hogy H.264/HEVC hosszmezős forrásból (MP4, MKV) MPEG-TS célkonténerbe Strict módban nem lehet exportálni, mert a TS Annex B reprezentációt és AUD-ot követel. Ez a kombináció csak Compatible módban érhető el.

**Sikertelen megfelelés.** Ha a célkonténer megköveteli a tömörített csomagpayload megváltoztatását, a Strict Copy exportot el kell utasítani. A program nem kapcsolhat át automatikusan kompatibilis Stream Copy módba. A felhasználó külön döntéssel engedélyezheti a másik üzemmódot.

### 2.2 Compatible Stream Copy

Belső azonosító: `stream_copy_compatible`. Feladata a tömörített médiaadatok újrakódolás nélküli másolása, szükség esetén kompatibilitási átalakításokkal.

**Engedélyezett műveletek**

- A Strict Copy valamennyi megengedett művelete.
- Kodekek közötti újrakódolást nem igénylő bitstream-reprezentációváltás.
- Konténerfüggő bitstream filterek alkalmazása.
- Kodekparaméterek konténerszintű reprezentációjának átalakítása.
- Támogatott, szemantikailag veszteségmentes fejléc- és paraméterátrendezés.
- Extradata kinyerése vagy elhelyezése.
- A célmuxer követelményeinek megfelelő adatfolyam-előkészítés.

Példák (referencianevek; a megvalósítás natív Rust, 11. fejezet) *\[v1.5\]*: H.264: `h264_mp4toannexb`; HEVC: `hevc_mp4toannexb`; AAC: `aac_adtstoasc`. Ezek megfelelő alkalmazása nem igényel dekódolást és újrakódolást.

**Kötelező korlátozások.** A Compatible Stream Copy nem módosíthatja a dekódolt média tényleges tartalmát. Nem engedélyezhető:

- Képkockák újrakódolása.
- Képkockák interpolálása.
- Képméret módosítása.
- Színkorrekció.
- Hangkeverés.
- Hangnormalizálás.
- Codec transcoding.
- Olyan metadata- vagy paramétermódosítás, amely megváltoztatja a dekódolt média értelmezését.
- Tartalomvesztéssel járó bitstream filter.

A Compatible Stream Copy kizárólag olyan bitstream-átalakítást alkalmazhat, amelynek veszteségmentessége és dekódolási megfelelősége az adott kodekre igazolt. A bitstream filter létezése önmagában nem bizonyítja annak veszteségmentességét.

**Kompatibilitási szabály.** A megengedett átalakításokat külön, verziózott allowlistben kell nyilvántartani. Minden engedélyezett átalakításhoz tartozzon: forráskodek, forrásreprezentáció, célreprezentáció, engedélyezett konténerkombinációk, alkalmazott bitstream filter, elvárt átalakítási tulajdonságok, kötelező validációs módszer és regressziós teszt. Ismeretlen vagy nem igazolt bitstream-konverzió automatikusan nem engedélyezhető.

*\[v1.2\]* Az allowlist a muxer által implicit végzett átalakításokat is tartalmazza (MPEG-TS: Annex B, AUD-beszúrás, ADTS-keretezés; ezeket a Bitstream Adapter végzi, a natív TS muxer nem módosít payloadot \[v1.3\]; lásd 12.4).

## 3. A két üzemmód összehasonlítása

| Tulajdonság | Strict Copy | Compatible Copy |
| --- | --- | --- |
| Videó újrakódolása | Tiltott | Tiltott |
| Hang újrakódolása | Tiltott | Tiltott |
| Tömörített payload megváltoztatása | Tiltott | Igazolt reprezentációváltás esetén megengedett |
| Konténerváltás | Feltételesen | Feltételesen |
| MP4/MKV H.264/HEVC → MPEG-TS *\[v1.2\]* | Tiltott | Megengedett (12.4) |
| Időbélyegek módosítása | Megengedett | Megengedett |
| Metadata módosítása | Konténerszinten | Konténerszinten |
| Veszteségmentes bitstream filter | Payloadváltozás esetén tiltott | Engedélyezett |
| GOP-biztos vágás | Támogatott | Támogatott |
| Pontos GOP-közepi vágás | Nem garantált | Nem garantált |
| Smart Renderre váltás | Tiltott | Tiltott |
| Full Renderre váltás | Tiltott | Tiltott |
| Csomagpayloadok bitazonossága | Kötelező | Nem feltétlenül |
| Dekódolt tartalom megőrzése | Kötelező | Kötelező |

A kompatibilis mód nem magasabb és nem alacsonyabb minőségű, mint a szigorú mód. A különbség kizárólag a megengedett, újrakódolás nélküli bitstream-átalakítás.

## 4. Exportüzemmódok végleges modellje

A CFA4 öt exportüzemmódot kínáljon:

1. Auto.
2. Stream Copy – Strict.
3. Stream Copy – Compatible.
4. Smart Render.
5. Full Render.

**Auto.** *\[v1.5\]* Az Auto csak javasol. Kompatibilitási vizsgálat alapján a Strict Copy, Compatible Copy, Smart Render, Full Render sorrendben az első teljesíthető módot ajánlja, előnézettel és indoklással. Ha a javaslat bármilyen újrakódolást tartalmaz, a végrehajtáshoz a felhasználó kifejezett megerősítése kell; a jelentés szegmensenként mutatja, mi lett másolva és mi újrakódolva. Az Auto soha nem sértheti a felhasználó által megadott újrakódolási tilalmat.

**Strict Copy.** Kizárólag bitazonos csomagpayload-másolás.

**Compatible Copy.** Újrakódolás nélküli, kompatibilis bitstream-konverzióval kiegészített másolás.

**Smart Render.** *\[v1.5\]* Idővonal-hibrid: a másolható szakaszokat Strict vagy Compatible Copyval őrzi meg, a többit újrakódolja, a vágáspontoknál határ-újrakódolást alkalmaz. A GOP Smart Cut ennek egyklipes esete (13. fejezet).

**Full Render.** Teljes renderelés és szükséges újrakódolás.

*\[v1.2\]* A kimeneti formátum (pl. MPEG-TS vagy M2TS) az exportüzemmódtól független választás: mind az öt üzemmód célozhat TS kimenetet, a 12. fejezet szabályai szerint.

*\[v1.5\]* A GOP Smart Cut nem hatodik exportüzemmód. A Qt6 forrásmonitorban „Forrás vágása” műveletként indítható, és egy forrástartományra szűkített Smart Render tervet hoz létre.

## 5. Végleges architektúra

A CFA4 natív exportarchitektúrája:

```text
   CFA4 Qt6 GUI / CLI            [v1.2: egui kivezetve]
               |
               v
      Unified Export Planner
               |
        Compatibility Analyzer
               |
     +---------+---------+
     |                   |
     v                   v
   Strict             Compatible
   Stream Copy        Stream Copy
     |                   |
     |                   v
     |             Bitstream Adapter
     |                   |
     +---------+---------+
               |
               v
        Packet Scheduler
               |
               v
        Timestamp Mapper
               |
               v
          Output Muxer   (MP4/MOV, MKV, [v1.2] MPEG-TS / M2TS)
               |
               v
        Export Validator
               |
               v
         Atomic Commit
```

A Smart Render és Full Render a közös Export Planner más végrehajtási útvonalai. A Stream Copy motorban egyik üzemmód sem használhat encodert.

*\[v1.2\]* Az MPEG-TS muxer az Output Muxer egyik megvalósítása; a render és a Stream Copy út ugyanazt használja (12.1).

## 6. Repository integráció

Elsődleges meglévő komponens: `crates/export`.

Kapcsolódó meglévő komponensek: `crates/engine`, `crates/render`, `crates/bitstream`, `crates/isobmff`, `crates/matroska`, `crates/mpegts`, `crates/h264`, `crates/hevc`, `crates/av1`, `crates/aac`, `crates/opus`, `crates/flac`, `crates/platform`, `apps/cfa4-cli`, `python/cfa4_qt` *\[v1.2: a `crates/ui-egui` kikerült, az egui kivezetésre kerül\]*.

A jelenlegi render → encode → mux exportútvonalat meg kell őrizni. A Stream Copy új, csomagszintű végrehajtási útvonalként csatlakozik hozzá. Nem készül párhuzamos export-feladatsor vagy második exportbeállítás-kezelő.

## 7. Új modulok

Tervezett állományok:

```text
# [v1.5] Tervezés: cfa4-smart-edit (L3), a P1 szerint
crates/smart-edit/src/capabilities.rs
crates/smart-edit/src/media_index.rs
crates/smart-edit/src/h264_index.rs
crates/smart-edit/src/mpeg2_index.rs
crates/smart-edit/src/packet_index.rs
crates/smart-edit/src/eligibility.rs
crates/smart-edit/src/plan.rs            # közös ExportExecutionPlan
crates/smart-edit/src/copy_planner.rs
crates/smart-edit/src/smart_render_planner.rs

# Végrehajtás: cfa4-export
crates/export/src/atomic.rs              # közös atomikus író segéd
crates/export/src/copy/mod.rs
crates/export/src/copy/executor.rs
crates/export/src/copy/timestamps.rs
crates/export/src/copy/stream_map.rs
crates/export/src/copy/bitstream.rs      # natív Rust adapterek
crates/export/src/copy/strict.rs
crates/export/src/copy/compatible.rs
crates/export/src/copy/validate.rs
crates/export/src/copy/tests.rs
crates/export/src/smart_render/mod.rs
crates/export/src/smart_render/executor.rs
crates/export/src/smart_render/boundary.rs   # határ-újrakódolás
crates/export/src/smart_render/tests.rs
```

Új tesztállományok:

```text
crates/export/tests/stream_copy_strict.rs
crates/export/tests/stream_copy_compatible.rs
crates/export/tests/stream_copy_e2e.rs
```

Meglévő állományok bővítése: `crates/export/src/lib.rs`, `crates/export/src/settings.rs`, `crates/export/src/job.rs`, `crates/engine/src/export_tools.rs`. Szükség esetén módosítandók a meglévő konténer- és bitstream-kezelők.

*\[v1.2\]* Az MPEG-TS muxeradapter, a `MpegTsProfile` modell és a hozzájuk tartozó tesztek pontos útvonalát a 12.6 fejezet 1. feladata rögzíti.

## 8. Közös adatmodell

**MediaStreamInfo:** kodekazonosító, profil, level, codec extradata, streamazonosító, időalap, kép- és hangparaméterek, szín- és HDR-paraméterek.

**PacketInfo:** csomagazonosító, forrásfájl és streamazonosító, payload, PTS, DTS, időtartam, flags, referenciafüggőségekhez szükséges információk.

**CopyMode:** Strict, Compatible.

**BitstreamTransformation:** átalakítás azonosítója, forrásreprezentáció, célreprezentáció, filter neve, validációs követelmények, engedélyezett mód.

**ExportExecutionPlan:** exportüzemmód, forrásszegmensek, céladatfolyamok, másolási műveletek, engedélyezett átalakítások, időbélyeg-leképezés, muxolási konfiguráció *\[v1.2: TS kimenetnél `MpegTsProfile`, 12.6/2\]*, validációs terv.

A meglévő `cfa4-time` időtípusok használata kötelező.

*\[v1.5\]* Egyetlen tervtípus: az `ExportExecutionPlan` szegmenslistát tartalmaz, és minden szegmens módja `StrictCopy`, `CompatibleCopy`, `BoundaryReencode` (határ-újrakódolás) vagy `Render`. Ez váltja ki az R1 `RenderMode` típusát, valamint a Smart Cut és a Smart Render terv külön plan-típusait. A `PacketInfo` a payloadot nem tartja memóriában, csak a forrásbeli helyét (fájl, eltolás, hossz) és a hash-ét; a bájtokat a végrehajtó streamelve olvassa.

## 9. Strict Copy végrehajtási szerződés

A Strict Copy végrehajtásakor:

1. A forrásadatfolyamot azonosítani kell.
2. A tömörített csomagpayloadokat ki kell olvasni.
3. A másolandó szakasz csomaghatárait ellenőrizni kell.
4. A célkonténer kompatibilitását meg kell vizsgálni.
5. Minden payloadmódosítást igénylő konverziót el kell utasítani.
6. A csomagokat változatlan payloadtartalommal kell továbbítani.
7. A konténerhez szükséges időbélyeg- és streamazonosító-módosítások végrehajthatók.
8. A kimeneti adatfolyamot validálni kell.

A strict validátor a forrásból kiválasztott payloadokat hasonlítsa össze a kimenetből visszaolvasott payloadokkal. A validáció ne támaszkodjon kizárólag teljes fájlhashre. A konténer saját indexei, fejlécadatai és metaadatai változhatnak. Ha a kimeneti demuxer olyan reprezentációváltást végez, amely miatt a payloadok nem hasonlíthatók össze közvetlenül, a Strict Copy adott kombinációja csak külön igazolt, bitazonos ellenőrzési módszer megléte esetén fogadható el.

## 10. Compatible Copy végrehajtási szerződés

A Compatible Copy végrehajtásakor:

1. A forrásadatfolyamot azonosítani kell.
2. Ellenőrizni kell a célkonténert.
3. Meg kell állapítani a szükséges bitstream-reprezentációt.
4. Ki kell választani az engedélyezett átalakítást.
5. Ellenőrizni kell az átalakítás allowlist-jogosultságát.
6. Végre kell hajtani a szükséges bitstream filtert.
7. A csomagokat a célkonténerbe kell muxolni.
8. Ellenőrizni kell a dekódolt tartalom egyezőségét.

A kompatibilis módhoz alkalmazott átalakításnak garantálnia kell a dekódolt média tartalmi megőrzését. A konténerrel kapcsolatos technikai jelölések változhatnak, de a dekódolt képek és hangminták tartalma nem változhat a kompatibilitási művelet következtében.

*\[v1.5\]* A validáció rétegzett. A szerkezeti réteg minden kodeknél kötelező: a kimenetből visszaolvasott hozzáférési egységek az átalakítás visszafordítása után megegyeznek a forráséval, és a paraméterkészletek, időbélyegek és flagek is egyeznek. Ez a `BITSTREAM_EQUIVALENT` eredmény, és önmagában elég a PASS-hoz. A dekódolási réteg (rekonstruált képkockák, illetve hangminták összevetése illesztett időtengelyen) csak ott fut, ahol a saját dekóder támogatja a profilt (jelenleg H.264 8 bit 4:2:0 progresszív, HEVC Main/Main 10); máshol a jelentés „nem dekódolva ellenőrizve” jelzést kap, ami nem hiba. Az összehasonlítás feltételeit előre rögzíteni kell.

*\[v1.2\]* MPEG-TS célkonténernél a 7. lépés a 12. fejezet szerinti muxeren keresztül történik, és a 12.6/7 utóellenőrzés is kötelező.

## 11. Bitstream filter szabályok

*\[v1.5\]* A CFA4 kizárólag ellenőrzött, natív Rust bitstream-adaptereket használhat automatikusan. Elsőként támogatandó: Annex B ↔ AVCC (H.264), Annex B ↔ HVCC (HEVC), ADTS ↔ nyers AAC, AUD beszúrása és eltávolítása. Az FFmpeg filternevei (`h264_mp4toannexb`, `hevc_mp4toannexb`, `aac_adtstoasc`) csak referencia-azonosítók a CI-orákulumhoz. Minden további adaptert külön kell értékelni.

Különösen nem szabad automatikusan veszteségmentesnek minősíteni azokat a filtereket, amelyek:

- NAL-unitokat távolítanak el.
- Képtartalmat módosítanak.
- Hangmintákat elhagynak.
- Kodekparamétereket szemantikailag módosítanak.
- A dekódolási eredményt megváltoztatják.

A filterek támogatottsága verziózott kompatibilitási mátrix alapján történjen.

*\[v1.2\]* A muxer által automatikusan beszúrásra kerülő filterek és keretezések (a natív muxerekben ilyen nem lehet, a Matroska író ProRes icpf-levágását is az adapterbe kell átvinni \[v1.3\]) ugyanazon allowlist alá esnek, és a végrehajtási naplóban meg kell jelenniük. Rejtett, naplózatlan muxeroldali átalakítás nem megengedett.

## 12. MPEG-TS kimenet *\[v1.2 új fejezet; v1.3: natív Rust muxer\]*

*\[v1.3\]* Döntés (2026-10-27): a TS muxer natív Rustban készül a `cfa4-mpegts` crate-ben. Az FFmpeg 9.0.2 csak a CI-tesztek orákuluma marad. Az eredeti terv FFmpeg/`libavformat` hivatkozásait ennek megfelelően átírtam; a tartalmi követelmények (módok, profilok, validáció, tesztek) változatlanok.

### 12.1 Cél

A CFA4 exportútja tudjon szabványos MPEG-2 Transport Stream fájlokat készíteni a saját, natív Rust MPEG-TS muxerével. A megoldás támogassa a fájlba exportálást, a több programot és hangsávot tartalmazó kimenetet, a beállítható szolgáltatás- és muxparamétereket, valamint a muxolt fájl gépi ellenőrzését. A muxert a render export és a Stream Copy út egyaránt használja.

### 12.2 Projektkeretek

- Célplatform: amd64, Ubuntu 26.04 és variánsai.
- Működési környezet: CFA4, Rust/Cargo. Az FFmpeg 9.0.2 és az ffprobe csak a CI-ben fut, külső orákulumként.
- A muxolásért a natív `cfa4-mpegts` muxer felel: L0 réteg, csak `cfa4-bitstream` függőség, `unsafe` nélkül, „never crash” szabályok szerint. Kiindulópontja a `crates/mpegts/src/tests.rs` tesztmuxere (PAT, PMT CRC-vel, PES, 188/192 bájtos csomagok).
- A felület a Qt6-os CFA4 exportfolyamába illeszkedik.
- A kimenet önálló `.ts` fájl legyen. Az M2TS kimenet külön választható mód; nem változtathatja meg észrevétlenül a normál TS kimenetet.
- A terv nem tartalmaz HLS/DASH szegmentálást, élő közvetítést, DVB/ATSC adáslánc teljes megfelelőségi tanúsítását vagy saját kodekimplementációt.
- A projekt már meglévő videó- és hangkodek-támogatása határozza meg a ténylegesen kiválasztható stream-ek körét.

### 12.3 Architektúra

1. A CFA4 exporttervező (az 5. fejezet Unified Export Plannere) összeállítja a kimeneti profilból és a timeline-ból a muxolási specifikációt.
2. Az export-validátor ellenőrzi, hogy a kiválasztott kodekek és streamkombinációk MPEG-TS-be muxolhatók-e.
3. A `cfa4-export` a stream-paramétereket, a pakettsorozatot, az időbélyegeket és a típusos muxeropciókat (`MpegTsProfile`) átadja a natív TS muxernek. A muxer payload-átlátszatlan: a kapott bájtokat nem módosítja, minden átalakítás (Annex B, AUD, ADTS) előtte, a Bitstream Adapterben történik.
4. Az exportfeladat az ideiglenes kimenetet lezárja, majd sikeres ellenőrzés után véglegesíti (Atomic Commit).
5. A Qt6 felület a CFA4 profilokat és a validáció eredményeit jeleníti meg; a muxerbeállításokat az engine-től kapja, nem maga számolja.

### 12.4 Kapcsolat a Stream Copy módokkal

- H.264/HEVC hosszmezős forrásból (MP4, MOV, MKV) TS kimenet csak Compatible módban készíthető, mert Annex B átalakítás és AUD-beszúrás kell. Strict módban az exportot `COPY_PAYLOAD_MODIFICATION_REQUIRED` kóddal el kell utasítani.
- Nyers AAC (MP4/MKV) TS-be csak ADTS-keretezéssel kerülhet, ez is Compatible mód.
- TS → TS másolás (Annex B forrás AUD-dal, ADTS hang) Strict módban is lehetséges, mert a natív muxer nem módosítja a payloadot.
- Minden átalakítás (Annex B, AUD, ADTS) a 2.2 szerinti allowlist része, a Bitstream Adapter végzi, és a végrehajtási naplóban megjelenik. A muxerben nincs automatikus, rejtett bitstream filter.

### 12.5 Felhasználói kimeneti módok és profilok

**MPEG-TS:** kiterjesztés `.ts`, TS-csomagméret 188 bájt. Általános fájlexport és stream-copy esetén ez az alapértelmezett MPEG-TS mód.

**M2TS:** kiterjesztés `.m2ts` vagy `.mts`. 192 bájtos csomagok (4 bájtos TP\_extra\_header időbélyeggel + 188); a felület külön módválasztót ad. Az M2TS kimenet nem szerepelhet normál `.ts` profilként, és a csomagméretet a validátor külön ellenőrzi. Blu-ray-kompatibilis teljes authoringot nem ígér: a mód csak az M2TS csomagolást jelenti.

A CFA4 profilbeállításai nevezzék meg a felhasználási célt, de minden tényleges muxerérték legyen explicit és menthető.

1. **Általános fájl:** VBR muxolás; PAT/PMT és SDT dokumentált CFA4-alapértékekkel (az FFmpeg alapértékeihez igazítva, hogy az orákulumteszt összevethető legyen); PCR automatikus ütemezése; program/service név és szolgáltató megadható; a CFA4 az időbélyegeket a projekt kimeneti időalapjához igazítja.
2. **CBR / rögzített muxráta:** kötelező muxráta-beállítás; a validátor ellenőrzi, hogy a megadott muxráta nem kisebb a kiválasztott elemi stream-ek összbitsebességénél a szükséges TS-overheaddel együtt; a muxráta fölötti kitöltést (null-csomagok) a muxer végzi; alacsony muxráta vagy nem teljesíthető célbitráta esetén az export nem indul el.
3. **Műsorszórási paraméterezés:** állítható rendszer (System A / System B); Service ID, Transport Stream ID, Original Network ID és service type; külön PCR PID csak akkor engedélyezhető, ha a profil vagy célformátum megköveteli; a felület jelzi, hogy a paraméterezés önmagában nem igazolja a teljes DVB/ATSC megfelelőséget.
4. **M2TS:** külön kimeneti mód, saját kiterjesztéssel és ellenőrzési szabályokkal; a `.ts` és `.m2ts` profilopciók nem keveredhetnek.

### 12.6 Feladatokra bontott megvalósítás

#### 1. A meglévő MPEG-TS kód felmérése

- A `crates/mpegts` demuxerének, PES- és PSI-kódjának, valamint a `src/tests.rs` tesztmuxerének felmérése: mi emelhető át termelő kódba, mi hiányzik (PCR-ütemezés, CBR kitöltés, SDT, több program, M2TS időbélyeg).
- Az új muxer a meglévő `cfa4-export` útba illeszkedik a `cfa4-isobmff` és `cfa4-matroska` író mintájára; párhuzamos exportút nem készül.
- Rögzíteni kell a CI FFmpeg 9.0.2 orákulum muxer- és ffprobe-képességeit, amelyekkel a tesztek összevetik a kimenetet.
- Kimenet: rövid integrációs jegyzet és a módosítandó modulok pontos listája.

#### 2. Muxolási specifikáció és profilmodell

- Bevezetendő egy szerializálható `MpegTsProfile` modell, benne legalább:
  - kimeneti mód (`Ts188` vagy `M2ts192`);
  - muxolási mód (`Vbr` vagy `Cbr { mux_rate }`);
  - rendszer (`SystemA` vagy `SystemB`);
  - `service_name`, `service_provider`;
  - `service_id`, `transport_stream_id`, `original_network_id`;
  - `pcr_period_ms`, opcionális `pcr_pid`;
  - PAT/PMT és SDT periódus;
  - timestamp-kezelési mód;
  - opcionális `tables_version`.
- A hiányzó mezők determinisztikus CFA4-alapértéket kapjanak; a ténylegesen felhasznált értékek kerüljenek bele az exportnaplóba.
- Profilverziózás legyen része a mentésnek, hogy később új opciók hozzáadhatók legyenek régi profilok eltörése nélkül.

#### 3. Kodek- és stream-validáció

- A validátor ellenőrizze minden streamnél a kodekazonosítót, extradata/header meglétét, időalapot, a PMT stream\_type hozzárendelést és a muxer kompatibilitási mátrixát.
- A stream-copy út csak kompatibilis, már kódolt csomagokat fogadjon; a szükséges átalakítást a Bitstream Adapter végzi, csak Compatible módban (12.4).
- Ha egy stream nem muxolható közvetlenül, a CFA4 adjon egyértelmű okot és javaslatot: másik profil, engedélyezett újrakódolás vagy stream eltávolítása.
- Ne legyen csendes kodekcsere, hangtörlés vagy kényszerített újrakódolás.
- Támogatandó és tesztelendő első körös stream-típusok: H.264, HEVC, MPEG-2 Video, AAC és MPEG audio. MPEG-2 Video és MPEG audio csak Stream Copyval, mert a CFA4-nek nincs ilyen kódolója; HEVC rendereléssel csak hardveres kódoló esetén.
- További kodekek csak külön kompatibilitási mátrix és mintafájlteszt után kerüljenek a felületre.

#### 4. Natív TS muxer

- A `cfa4-mpegts` crate-ben készüljön termelő `TsWriter`, amely:
  - PAT-ot, PMT-t (programonként) és SDT-t ír a profil periódusai szerint, CRC-32-vel;
  - hozzárendeli a validált audio-, video- és adatstreameket PID-ekhez és stream\_type-okhoz;
  - PES-csomagokat képez PTS/DTS-sel, és 188 bájtos TS-csomagokra bontja adaptation-field kitöltéssel;
  - PCR-t ütemez a `pcr_period_ms` szerint, CBR módban null-csomagokkal tartja a muxrátát;
  - M2TS módban 4 bájtos időbélyeg-fejlécet tesz minden csomag elé;
  - `Write` (és nem `Seek`) kimenetre ír, így streamelhető ideiglenes fájlba.
- Az opciók explicit, típusellenőrzött mezők legyenek; ne felhasználói bemenetből összefűzött szövegként kerüljenek továbbításra.
- Kezelni kell az írási, lezárási és fájlzárási hibákat; minden hiba `Result`, pánik nélkül.
- A muxer nem módosítja a kapott payloadot (12.3/3); ezt teszt rögzíti.

#### 5. Időbélyegek és streamszinkron

- Az export előkészítése ellenőrizze, hogy minden stream időbélyege monoton és a stream időalapjában értelmezhető.
- A CFA4 projektidőből (`Tick`) a 90 kHz-es PTS/DTS-re és 27 MHz-es PCR-re történő átalakítás legyen egyetlen közös, tesztelt útvonalon (a 14. fejezet `TimestampNormalizer` modulja).
- B-frame-es videónál a DTS/PTS viszonyt a bemeneti codec packetekből kell megőrizni; a muxernek nem szabad mesterségesen egyenlővé tennie őket.
- A stream-ek kezdési eltérését a kiválasztott timestamp-profil szerint kezelje; az eredeti timestamp megőrzése és a nulláról induló időbélyeg külön mód legyen.
- A hibás, hiányzó vagy visszafelé ugró timestamp az export előtt vagy közben diagnosztizálható hibát okozzon; ne készüljön megtévesztően sikeres fájl.

#### 6. Exportbiztonság és részleges fájlok

- Az export ideiglenes fájlba történjen a célmappa mellett.
- A CFA4 csak a sikeres lezárás, fájlzárás (fsync) és utóellenőrzés után nevezze át az ideiglenes fájlt a végleges célra.
- Hiba vagy megszakítás esetén a feladat törölje az ideiglenes fájlt, és őrizze meg a hibakódot, a muxer hibaüzenetét és az exportprofil-azonosítót.
- A célfájl felülírási szabálya egyezzen a CFA4 többi exportformátumának viselkedésével (20. fejezet).

#### 7. Utóellenőrzés és diagnosztika

Sikeres muxolás után a CFA4 a saját `cfa4-mpegts` demuxerével ellenőrizze az elkészült fájlt (az ffprobe csak a CI-ben fut):

- a fájl TS-ként felismerhető-e;
- az elvárt program és stream-ek megjelennek-e;
- a codec, felbontás, frame rate, audio sample rate és csatornaszám megfelel-e a tervnek;
- a TS módnál a 188 bájtos csomagszinkron felismerhető-e; M2TS módban a 192 bájtos csomagolás ellenőrizhető-e;
- a PAT/PMT alapján a programok és PID-ek olvashatók-e, a continuity counterek hibátlanok-e;
- mintavételes dekódolási próba során nincs-e sérült vagy hiányzó csomagokra utaló hiba. Ha a saját dekóder a profilt nem támogatja, az állapot „nem futott”, nem hiba.

Az utóellenőrzés különböztesse meg a „muxolás sikerült” és a „dekódolási próba sikerült” állapotot. A felület mindkettőt mutassa. Stream Copy exportnál ez a 18. fejezet szerinti módspecifikus validáció mellett fut.

#### 8. Qt6 exportfelület

- Formátumválasztó: MPEG-TS (`.ts`) és M2TS (`.m2ts`).
- Profilválasztó: Általános fájl, CBR, Műsorszórási paraméterezés, egyéni profil.
- A haladó mezők csak a részletes beállításokban jelenjenek meg.
- Muxráta, service metaadatok, PID-azonosítók és periódusok mellett legyen rövid magyarázat és érvényességi tartomány.
- A felület az export előtti validáció hibáit streamenként mutassa meg.
- A kész export összefoglalója tartalmazza a kiválasztott módot, muxrátát, programot, streameket, tényleges bitrátát és utóellenőrzési eredményt.

#### 9. Tesztelési és elfogadási kapuk

**Egységtesztek:** profil szerializálása és visszaolvasása; alapértékek, opciótartományok és hibás PID/muxráta elutasítása; kodek- és streamkombinációk validációja; timestamp átalakítás és monotonitásellenőrzés; muxer → saját demuxer körteszt bájtazonos payloadokkal; csonkolt és véletlenszerűen rontott bemenetre sincs pánik.

**Orákulum-integrációs tesztek (CI, FFmpeg 9.0.2 / ffprobe):**

- H.264 + AAC TS export és ffprobe-ellenőrzés.
- HEVC + AAC TS export.
- MPEG-2 Video + MPEG audio mintakimenet (Stream Copy).
- CBR muxolás tényleges kimeneti muxrátájának ellenőrzésével.
- M2TS kimenet csomagméret- és felismerési ellenőrzéssel.
- Több hangsáv és több program tesztje.
- B-frame-es videó, eltérő streamkezdések és nem nulláról induló timestamp tesztje.
- Az FFmpeg által dekódolt képek és hangok egyeznek a forráséval.
- Megszakítás, megtelt lemez vagy lezárási hiba esetén ne maradjon véglegesnek látszó részleges fájl.

**Teljes exportfolyam tesztjei:** Qt6 exportbeállítás → CFA4 exportterv → natív TS muxer → ideiglenes fájl → utóellenőrzés → végleges fájl; export újranyitása és CFA4-be történő visszaimportálása; a visszaimportált streamlista és időtartam összevetése az exporttervvel.

**Elfogadási feltételek:**

- A CFA4 képes `.ts` fájlt készíteni az engedélyezett kodek- és streamkombinációkból.
- A TS és M2TS mód külön választható, és a kiterjesztés, csomagolás, metaadatok összhangban vannak.
- A CBR profil a kért muxrátával működik, vagy előzetesen érthető hibát ad.
- A stream-copy út nem kódolja újra és nem módosítja csendben a streameket.
- Hibás vagy részleges export nem kerül végleges célfájlként publikálásra.
- A kész fájl saját demuxeres ellenőrzése, visszaimportja és mintán végzett dekódolási próbája sikeres, és a CI-ben az ffprobe/FFmpeg orákulum is elfogadja.

### 12.7 Megvalósítási sorrend

1. A meglévő `cfa4-mpegts` kód és a CI orákulum felmérése.
2. Profilmodell és validáció.
3. Natív `TsWriter` és `.ts` fájlexport.
4. Timestamp-kezelés és több stream/program támogatása.
5. CBR és műsorszórási paraméterek.
6. M2TS külön kimeneti mód.
7. Ideiglenes fájl, hibakezelés és utóellenőrzés.
8. Qt6 felület és exportösszefoglaló.
9. Orákulum-integrációs, visszaimport- és dekódolási elfogadási tesztek.

A 22. fejezetben ez a P2a fázis.

### 12.8 Projektadat (ellenőrizve)

A `main` 24f061a commitján: nincs FFmpeg-híd és `libavformat` kötés; az FFmpeg 9.0.2 csak teszt-orákulum (`AGENTS.md`, `CLAUDE.md`). A `cfa4-mpegts` demuxer kész, a muxer csak tesztsegédként létezik. Az export ma közvetlenül a végleges útvonalra ír (`crates/export/src/lib.rs`, `File::create`), ezért az ideiglenes fájl + átnevezés új, közös segédet igényel.

## 13. GOP Smart Cut *\[v1.4: a repo terve beépítve\]*

*\[v1.4\]* Forrás: [`docs/plans/GOP-SMART-CUT-2026-10-27.md`](GOP-SMART-CUT-2026-10-27.md) (main, #222), valamint a `docs/plans/CFA4-SMART-CUT-RENDER-R1.md` munkacsomagjai (ág: `feature/native-smart-cut-render`). A fázisok a 22. fejezet P1, P4, P6 és P7 lépéseibe kerültek.

### 13.1 Közös alapelvek (v1.1)

Mindkét Stream Copy mód ugyanazt a GOP-elemző infrastruktúrát használja. A rendszer vizsgálja: I-, P- és B-képkockák; IDR és CRA; nyitott GOP; zárt GOP; dekódolási függőségek; referenciaképek; megjelenítési sorrend; dekódolási sorrend.

A Strict Copy és Compatible Copy egyike sem garantál tetszőleges képkockahatáron pontos vágást. Biztonságos GOP-határ esetén mindkét mód alkalmazható. Nem biztonságos határ esetén a műveletet vissza kell utasítani, vagy külön felhasználói döntés alapján Smart Renderre kell váltani. A módváltás nem történhet észrevétlenül.

### 13.2 Cél

Egyetlen videóforrás képkocka-pontos In/Out tartományának kivágása úgy, hogy a teljes, biztonságosan másolható belső GOP-ok változatlan tömörített mintákként kerüljenek át, és csak a vágási határokhoz szükséges képkockák/GOP-ok legyenek újrakódolva. A GOPchop működési elve a CFA4 saját Rust-médiamotorjába épül; nincs külső folyamat és nincs külön alkalmazás. A vágási számítás `Tick`-ben, a demux-index minta-, decode- és display-sorrendet megőrizve működik; a biztonságos copy-tartományt a konténerképesség, a codec config és a GOP-szerkezet dönti el, nem a fájlkiterjesztés vagy egy általános keyframe flag.

### 13.3 Garanciák és felhasználói módok

1. **Képkocka-pontos mód:** az eredmény a kiválasztott forrásképkocka-határokat tartja; a szükséges határ-GOP-ok dekódolódnak és újrakódolódnak.
2. **GOP-határ mód:** stream-copy vágás a biztonságos random-access határokon (ez a 2. fejezet tiszta Stream Copy esete). A CFA4 a kiválasztott és tényleges határt, valamint az eltérést előre mutatja; a kért időt nem igazítja el csendben.
3. **Két, külön választható copy-garancia** (a 2. fejezet Strict és Compatible módja): Bitazonos – a másolt tömörített minták bájtjai és sorrendje változatlan, különben elutasítás; Kompatibilis – a másolt minták dekódolt tartalma változatlan. A garanciák között nincs automatikus átváltás; a konténer bájtjai egyik módban sem azonosak.
4. A belső GOP-minták payloadja nem változhat. A széleken használt encodernek kompatibilis codec configot és zárt, dekódolható csatlakozási GOP-ot kell előállítania.
5. Nem támogatott profile/bitmélység/kroma, nyitott GOP, B-frame reorder, interlace, HDR, audio delay/priming vagy célmuxer esetén a terv pontos okkal leáll. Teljes render-export csak külön felhasználói választással indulhat.
6. Az eredeti forrás nem módosul; ideiglenes kimenet csak ellenőrzött finalizálás után nevezhető át (20. fejezet).

Választott megközelítés: **GOP-tudatos hibrid** (képkocka-pontos határok, belső GOP-ok változatlan másolása), szemben a csak GOP-határra illesztett copyval (nem képkocka-pontos) és a teljes újrakódolással (lassú, minőségromlás).

*\[v1.5\]* A GOP Smart Cut a Smart Render egyklipes esete: ugyanazt az `ExportExecutionPlan` típust és ugyanazt a végrehajtót használja, egy forrástartományra szűkítve. A Qt6-ban a forrásmonitor „Forrás vágása” művelete indítja. Határ-újrakódolás R1-ben csak H.264 forrásnál van; MPEG-2, HEVC és AV1 forrásnál a vágás kulcskockára kerekít (a felület mutatja az eltérést), vagy teljes újrakódolást javasol.

### 13.4 Fázisok és helyük a 22. fejezetben

| GOP Smart Cut fázis | Tartalom | R1 munkacsomag | Hely (22.) |
| --- | --- | --- | --- |
| 1. Képesség- és formátumleltár | Demux/mux API-k feltérképezése (payload, DTS/PTS, sync/random-access típus, codec config, időalap, szín, hang); `SmartCutCapabilities` supported/unsupported indokkal; első cél H.264 8-bit 4:2:0 zárt GOP | WP-00, WP-01, WP-02 | P0, P1 |
| 2. Tiszta vágási tervelőállítás | `SmartCutRequest`, `SmartCutOptions`, `SmartCutPlan`, `CutMode`, `CopyGuarantee`, `SmartCutReport`; fájlírás nélküli planner, UI-előnézet; DTS/PTS külön, CRA/RASL leading frame-ek helyesen | WP-03 | P4 |
| 3. Hibrid mintamásolás és határ-GOP render | Belső GOP-ok payload-módosítás nélkül; határrégió dekódolása a legkorábbi random-access mintától, kódolás; illeszkedő codec config; AAC priming/padding, A/V eltolás explicit; muxolás utáni ellenőrzés | WP-04 | P4 |
| 4. Engine parancs és Qt6 felület | `smartCut.plan`, `smartCut.run`, `smartCut.cancel` `enabled()` indoklással; Qt6-ban külön vágási mód és garancia; projektadat nem változik | WP-08 | P7 |
| 5. Codec-bővítés és teljes ellenőrzés | HEVC, ProRes/DNx, MPEG-2 egyesével; nyitott GOP, B-frame, interlace, HDR külön kapuval; CPU referenciaút, gyártófüggetlen hardveres fallback | WP-07, WP-09 | P6, P7 |

**A 2. fázis tesztjei:** frame-rácsra eső/nem eső Tick; első/utolsó képkocka; üres/túl hosszú/negatív tartomány; nincs random-access minta; nyitott GOP; PTS≠DTS; 33 bites időbélyeg-wrap; hiányzó hang; nem támogatott profile.

**A 3. fázis tesztjei:** másolt minták payload hash-e; határképkockák dekódolt összehasonlítása; A/V szinkron; codec config és seek; megszakítás; írási hiba és temp-fájl takarítás. Az ffmpeg/ffprobe csak külső teszt-orákulum.

### 13.5 Elfogadási kritériumok

- Támogatott bemeneten a képkocka-pontos kimenet pontosan a kiválasztott frame-tartományt adja.
- Belső, változatlan GOP-ok tömörített payloadja hash szerint egyezik; határképkockák a CFA4 dekódolt forrásframe-ekkel egyeznek a kódolás dokumentált tűrésén belül.
- Bitazonos és Kompatibilis mód külön választható, egyik sem vált át csendben a másikra.
- Kimenet helyesen seekelhető, lejátszható, megfelelő időtartamú és A/V szinkronú; megszakítás nem hagy sérült végleges fájlt.
- Csak igazolt codec/container/profile kombináció jelölhető támogatottnak.

**Felülvizsgálati fókusz:** nyitott GOP és CRA/RASL; B-frame reorder DTS/PTS-leképezése; AAC priming/padding és timestamp-wrap, A/V drift nélkül; sérült minta, megtelt lemez, cancel pánik és végleges részfájl nélkül; ismeretlen codec metadata esetén konkrét okkal tiltás, csendes transzkód nélkül.

**Függőségek:** a 2. fejezet két Stream Copy módja; a célkonténer muxer-képessége (MPEG-TS csak a P2a után); kompatibilis határ-GOP-ot előállító encoder.

## 14. Timestamp kezelés

Mindkét üzemmód közös `TimestampNormalizer` modult használ. Feladata: PTS/DTS leképezés; racionális időalapváltás; VFR megőrzése; dekódolási sorrend ellenőrzése; negatív időbélyegek kezelése; több adatfolyam szinkronizációja; összefűzési időeltolások kiszámítása.

Az időbélyegek módosítása nem számít a tömörített média újrakódolásának. A kimeneti időrendnek azonban teljesítenie kell a konténer és az adott kodek szabályait.

*\[v1.2\]* Az MPEG-TS muxer is ezt a modult használja (12.6/5): 90 kHz-es PTS/DTS, 33 bites átfordulás, valamint az "eredeti időbélyeg" és a "nulláról induló" mód itt van definiálva.

## 15. Smart Render *\[v1.4: a repo terve beépítve\]*

*\[v1.4\]* Forrás: [`docs/plans/SMART-RENDER-2026-10-27.md`](SMART-RENDER-2026-10-27.md) (main, #222), valamint az R1 terv WP-05/WP-06 munkacsomagjai. A fázisok a 22. fejezet P1, P5, P6 és P7 lépéseibe kerültek.

### 15.1 Közös alapelvek (v1.1)

A Smart Render külön exportüzemmód marad. A Smart Render jogosult bizonyos szakaszokat újrakódolni; a két Stream Copy mód nem. A Smart Render saját végrehajtási tervében hivatkozhat Strict Copy, Compatible Copy és Reencode szakaszokra. Minden vegyes végrehajtás esetén ellenőrizni kell az összefűzhetőséget. A Strict és Compatible Copy közötti váltás nem engedélyezi automatikusan a Smart Render alkalmazását.

*\[v1.5\]* Terminológia: a „Smart Render” ebben a tervben mindig az idővonal-hibridet jelenti. Az R1 terv `BoundaryReencode` módja ennek egyik szegmenstípusa, neve „határ-újrakódolás”. Ahol az R1 dokumentum „Smart Render” néven a határ-újrakódolást érti, ott át kell nevezni.

### 15.2 Cél

A CFA4 timeline-exportja a változatlan, közvetlenül másolható klipszakaszokat tömörített mintaként őrizze meg, és csak azokat renderelje/kódolja újra, amelyeken vágási határ, átmenet, effekt, kompozitálás, sebességváltozás, szín-/méretkonverzió vagy exportbeállítás miatt képtranszformáció szükséges. A végeredmény ugyanazt a timeline-képet, időzítést, hangot és kimeneti specifikációt adja, mint a teljes render-export. A Smart Render a meglévő `pipeline.rs` előtt egy render/copy szakasztervezőt futtat, majd a kimenetet időrendben, egyetlen muxerbe írja.

A másolhatóságot a teljes timeline-rész, a dekóder/encoder képesség, a codec config, a frame rate/timebase, a szín/alpha, a hang, a konténer és a szakaszhatáron érvényes GOP/dekódolási függőség együtt dönti el. *\[v1.4\]* Ez egyben a Stream Copy „idővonal-jogosultság” szabályrendszere is: egy szekvencia akkor Stream Copy-zható, ha minden szakasza Copy-eligible.

### 15.3 Szakaszosztályok

- **Copy-eligible:** egyetlen forrásszakasz; nincs effekt/átmenet/kompozitálás/transform; sebesség 1:1; teljes képkocka-lefedettség; forrás- és kimeneti frame-rate/timebase kompatibilis; codec config, pixel format, szín/HDR, méret, aspect és kimeneti muxer támogatott; GOP-határ zárhatóan folytatható.
- **Render-required:** több réteg, alpha/blend, transition, effect, title/overlay, resize/scale, transform, speed/rate stretch, freeze, frame interpolation, stabilizálás, color conversion, projekt/export frame-rate eltérés vagy nem kompatibilis bitstream.
- **Boundary-required:** egyébként másolható szakasz, amelynek eleje/vége csak dekódolás és lokális újrakódolás után kapcsolható biztonságosan a szomszédhoz.

Copy-eligible szakasz csak egész, biztonságosan dekódolható GOP-egységekben másolható. Ha a közös codec config, keyframe/seam, időbélyeg vagy hangidőzítés nem garantálható, az érintett tartomány renderelendő; ha a kimenet vegyes formában sem állítható elő, az export az előzetes validációban leáll.

**Copy-garancia:** a 2. fejezet Bitazonos és Kompatibilis módja, külön választva. Nincs silent fallback és nincs újrakódolást elrejtő „quality-preserving” mód. A felületen látszik a másolt/renderelt időtartam, az érintett sávok, az encoderhasználat és a garancia. Választott megközelítés: **szegmensenkénti hibrid Smart Render**.

*\[v1.5\]* Viszony az Auto móddal: az Auto javasolhat Smart Rendert, de a végrehajtáshoz kifejezett megerősítés kell (4. fejezet), így a „nincs silent fallback” elv megmarad.

### 15.4 Fázisok és helyük a 22. fejezetben

| Smart Render fázis | Tartalom | R1 munkacsomag | Hely (22.) |
| --- | --- | --- | --- |
| 1. Szakaszbesorolás és előzetes becslés | `SmartRenderOptions`, `CopyPolicy`, `RenderSegment`, `SegmentReason`, `SmartRenderPlan`, `SmartRenderReport`; immutable snapshot + `ExportSettings` → Copy/Boundary/Render intervallumok; nested, gap, linked audio, proxy, overlay, HDR, framerate, speed; cache kulcs: snapshot hash + beállítás + capability verzió | WP-05 | P5 (az osztályozás már a P1-ben, a Stream Copy jogosultsághoz) |
| 2. Dekódolási és konténer seam-képesség | Capability objektum (codec config, sync sample, open/closed GOP, referenciák, PTS/DTS, pixel format, timebase, szín, hang, célmuxer); GOP-varrat szabályai; frakcionális rate f64 nélkül; hangmásolás csak priming/padding kezeléssel | WP-01, WP-02 | P1 |
| 3. Egy muxerbe író vegyes végrehajtó | Copy szegmens csomagot ír, Boundary/Render a meglévő `Pipeline`-on és encoder-regiszteren megy; egységes `TrackConfig`; explicit PTS/DTS szegmenshatárok; közös `Exporter` életciklus; determinisztikus export | WP-06 | P5 |
| 4. Engine, Qt6, CLI/MCP, export-profilok | `export.smartRender.plan`, `.start`, `.cancel`; explicit Smart Render kapcsoló, Bitazonos/Kompatibilis policy, előnézeti tábla okonként; preset csak preflight után ajánlhatja | WP-08 | P7 |
| 5. Bekapcsolás, kompatibilitási kapuk, mérések | Első szűk profil: H.264, azonos méret/frame rate, zárt GOP, single-layer; utána ProRes/DNx, HEVC, MPEG-2, nested, audio-only; open GOP, VFR, HDR/10-bit/4:2:2 csak conformance után; benchmark: másolt/újrakódolt arány, időmegtakarítás, A/V drift | WP-07, WP-09 | P6, P7 |

**Az 1. fázis tesztjei:** no-op klip → copy; effect/transition/overlay/resize/speed/framerate eltérés → render; kevert track → helyes intervallumfelbontás; null/üres szekvencia; nested ciklus/mély egymásba ágyazás; snapshot-változás cache-érvénytelenítést okoz.

**A 3. fázis tesztjei:** egyetlen copy szegmens; copy-render-copy; két codec config miatt tiltott terv; B-frame timestamp; GOP-varrat dekódolása; audio FX + video copy; megszakítás minden szegmenstípus közben; determinisztikus export több batchmérettel.

### 15.5 Elfogadási kritériumok

- A Smart Render kimenet látható képkockái, frame countja, frame time-ja, audio sample countja és A/V szinkronja egyezik a teljes render referenciaexporttal.
- Minden másolt payload Bitazonos módban bájtazonos; Kompatibilis módban a dokumentált tartalmi egyenértékűséget teljesíti.
- Effektes/összetett/nem kompatibilis szakasz renderelendőként szerepel, nem tűnik el, és nem másolódik változtatás nélkül.
- Minden váltási pont seekelhető, dekódolható és időbélyeg szerint folytonos; eltérés esetén a preflight elutasít vagy a kérdéses tartományt rendereli.
- A normál full-render export továbbra is elérhető, és referenciaút marad.

**Felülvizsgálati fókusz:** nem egész fps képkocka-rácsa (nincs duplikáció/vesztés); nyitott GOP, B-frame, CRA/RASL, codec config váltás a varraton; transition, keyframe-animáció, adjustment layer, export overlay és alpha mind renderelendő; audio effekt, mute/solo, mintavételezés-váltás, AAC priming; cancel, lemezbetelés, encoder-/rendererhiba után nincs részfájl vagy sérült projekt.

**Függőségek és nem célok:** a 13. fejezet capability- és seam-megállapításai közös alapként; MPEG-TS cél csak a P2a után; nem cél a minden timeline-ra érvényes copy-ígéret, sem a codec/render logika Pythonba vagy külön folyamatba másolása.

## 16. GUI-megvalósítás

A CFA4 Qt6 exportpaneljén az alábbi exportválasztók jelenjenek meg: Automatikus; Stream Copy – Bitazonos; Stream Copy – Kompatibilis; Smart Render; Teljes renderelés. A két Stream Copy módhoz külön ismertető tartozzon.

**Bitazonos Stream Copy** – felhasználói leírás: „A kiválasztott tömörített médiaadatok bitazonos másolása. Nem történik újrakódolás vagy payloadmódosítás. A kompatibilitást igénylő műveletek elutasításra kerülhetnek.”

**Kompatibilis Stream Copy** – felhasználói leírás: „Újrakódolás nélküli adatfolyam-másolás, szükség esetén igazoltan veszteségmentes bitstream-átalakításokkal.”

A felület mutassa: másolási mód; célkonténer; megengedett átalakítások; tervezett bitstream filterek; kodekkompatibilitás; GOP-kompatibilitás; validációs stratégia; az export visszautasításának konkrét oka.

*\[v1.2\]* Az új exportmódok kizárólag a Qt6 felületen jelennek meg; a megvalósítás a Qt port elkészülte után indul, az egui kivezetésre kerül. A célkonténer-választó tartalmazza az MPEG-TS (`.ts`) és M2TS (`.m2ts`) formátumot, a TS profilválasztót és exportösszefoglalót a 12.6/8 szerint.

*\[v1.5\]* Az Automatikus mód javaslatot mutat (választott mód, indoklás, szegmensenként a másolt és az újrakódolt idő), és újrakódolás esetén megerősítő párbeszédablakot kér. A GOP Smart Cut nem exportválasztó, hanem a forrásmonitor „Forrás vágása” művelete, amely jelzi, hogy a vágás képkocka-pontos vagy kulcskockára kerekített, és kell-e határ-újrakódolás.

## 17. API-megvalósítás

*\[v1.5, 8. döntés\]* A meglévő export API-t egységes `export.*` névtérrel kell bővíteni: `export.analyze` (forrás és képességek; az R1 `media.analyze_cut_points` helyett), `export.plan` (egyetlen `ExportExecutionPlan`, módtól függetlenül; az R1 `edit.plan_smart_cut` helyett), `export.start`, `export.cancel`, `export.validate`. A mód és a hatókör (idővonal vagy forrástartomány) a terv paramétere, nem külön parancscsalád, így nincs külön `smartCut.*`, `export.smartRender.*` vagy `export.copy.*` névtér. A meglévő `export.formats`, `export.resolve` és `export.queue.*` parancsok megmaradnak. A beállításokban külön mező jelölje a Stream Copy változatát.

Javasolt konfiguráció (Strict):

```json
{
  "exportMode": "streamCopy",
  "streamCopy": {
    "mode": "strict",
    "allowBitstreamFilters": false,
    "allowReencode": false,
    "validateOutput": true
  }
}
```

Kompatibilis változat:

```json
{
  "exportMode": "streamCopy",
  "streamCopy": {
    "mode": "compatible",
    "allowBitstreamFilters": true,
    "allowReencode": false,
    "validateOutput": true
  }
}
```

A szerializált mezőket a CFA4 meglévő `ExportSettings` rendszeréhez kell illeszteni. A korábbi presetek és projektek kompatibilitása kötelező.

*\[v1.2\]* TS kimenetnél a beállítások egy verziózott `mpegTs` objektumot is tartalmaznak (a `MpegTsProfile` szerializált alakja, 12.6/2). A pontos mezőneveket a 12.6/2 feladat rögzíti.

## 18. Validációs rendszer

Két külön validációs profilt kell kialakítani.

**Strict Validator** – kötelező ellenőrzések: kiválasztott tömörített csomagok azonosítása; csomagszám és sorrend ellenőrzése; payload-bájtsorozatok összehasonlítása; csomaghatárok ellenőrzése; PTS/DTS ellenőrzése; dekódolhatóság ellenőrzése; kimeneti konténer ellenőrzése. Eredményjelzés: `PAYLOAD_BIT_EXACT: PASS`.

**Compatible Validator** – kötelező ellenőrzések: alkalmazott bitstream filterek listája; filterek allowlist-megfelelősége; kimeneti adatfolyam ellenőrzése; dekódolhatóság ellenőrzése; dekódolt tartalom egyezősége; időbélyeg- és szinkronvizsgálat; kodekparaméterek ellenőrzése. Eredményjelzés: `DECODED_CONTENT_PRESERVED: PASS`.

Az export csak az adott mód kötelező validációinak sikeres teljesítése esetén tekinthető sikeresnek. Ha a teljes dekódolt tartalom egyezőségének bizonyítása nem végezhető el, a rendszer nem állíthatja, hogy a Compatible Copy ellenőrzése sikeresen teljesült.

*\[v1.2\]* TS/M2TS kimenetnél a módspecifikus validáció mellett a 12.6/7 utóellenőrzés is kötelező, külön „muxolás sikerült” és „dekódolási próba sikerült” állapottal.

*\[v1.5\]* A Compatible Validator a 10. fejezet rétegzett szabálya szerint fut: a szerkezeti réteg (`BITSTREAM_EQUIVALENT`) kötelező és elég a PASS-hoz, a dekódolási réteg csak támogatott profilnál fut. A Strict Validator változatlanul payload-egyezést kér. Smart Render és Smart Cut esetén a másolt szegmenseket a saját garanciájuk validátora ellenőrzi; az újrakódolt szegmenseknél a frame count, a frame time és a sample count pontos egyezése kötelező, a képtartalmat a forrás dekódolt képéhez kell mérni, dokumentált tűréssel.

## 19. Hibakezelés

Kötelező hibakódok: `COPY_CODEC_UNSUPPORTED`, `COPY_CONTAINER_UNSUPPORTED`, `COPY_BOUNDARY_UNSAFE`, `COPY_PARAMS_MISMATCH`, `COPY_TIMESTAMP_INVALID`, `COPY_BITSTREAM_UNSUPPORTED`, `COPY_PAYLOAD_MODIFICATION_REQUIRED`, `COPY_TRANSFORMATION_NOT_ALLOWED`, `COPY_STRICT_VALIDATION_FAILED`, `COPY_COMPATIBLE_VALIDATION_FAILED`, `COPY_SOURCE_CHANGED`, `COPY_IO_FAILED`, `COPY_CANCELLED`.

*\[v1.2\]* Javasolt TS-specifikus kódok (a 12.6/4–7 hibaeseteihez): `TS_STREAM_NOT_MUXABLE`, `TS_MUXRATE_TOO_LOW`, `TS_PROFILE_INVALID` (hibás PID, periódus, azonosító), `TS_MUX_OPEN_FAILED`, `TS_PACKET_WRITE_FAILED`, `TS_TRAILER_FAILED`, `TS_POSTCHECK_FAILED`.

*\[v1.5\]* Smart Render, Smart Cut és Auto kódok: `SMART_BOUNDARY_ENCODER_UNAVAILABLE` (a forráskodekhez nincs gyártófüggetlen határ-kódoló; javaslat: kulcskocka-kerekítés vagy teljes újrakódolás), `SMART_CODEC_CONFIG_MISMATCH`, `SMART_SEGMENT_NOT_CONCATENABLE`, `AUTO_REENCODE_NOT_CONFIRMED`, `EXPORT_ATOMIC_COMMIT_FAILED`. Figyelmeztetés, nem hiba: `VALIDATION_DECODE_NOT_AVAILABLE`.

Minden hibakódhoz részletes technikai és felhasználói magyarázat tartozzon.

## 20. Adatbiztonság

- A forrásfájlok kizárólag olvasásra nyithatók meg.
- Az export ideiglenes fájlba történjen.
- A véglegesítés csak sikeres validáció után történhet.
- Meglévő fájl felülírása kizárólag jóváhagyással történhet.
- A megszakítás nem módosíthatja a forrásállományt.
- Az I/O-hibákat kontrolláltan kell kezelni.
- A pufferhasználat korlátozott legyen.
- A hibás export nem minősíthető sikeresnek.

*\[v1.2\]* Ugyanez a szabályrendszer vonatkozik a TS/M2TS exportra (12.6/6).

*\[v1.5\]* Mindez egyetlen közös segéden keresztül történik a `cfa4-export`-ban: ideiglenes fájl a célmappában, streamelt írás, fsync, majd átnevezés. Hiba vagy megszakítás esetén az ideiglenes fájl törlődik, a célfájl érintetlen marad. Minden muxer (MP4/MOV, Matroska, MPEG-TS/M2TS) ezt használja, a közvetlen `File::create` hívás megszűnik.

## 21. Tesztstratégia

**Strict Copy tesztek:** H.264 MP4 → MP4; HEVC MP4 → MP4; AAC változatlan másolása; MKV → MKV kompatibilis adatfolyamokkal; payload-hash összehasonlítás; payload-bájtösszehasonlítás; időbélyeg-módosítás; konténer-metadata módosítás; tiltott bitstream-konverzió visszautasítása; nem biztonságos GOP-határ visszautasítása. *\[v1.2\]* Új: H.264 MP4 → TS Strict módban `COPY_PAYLOAD_MODIFICATION_REQUIRED` hibával elutasítva; TS → TS Strict másolás.

**Compatible Copy tesztek:** H.264 MP4 → MPEG-TS Annex B konverzióval; HEVC MP4 → MPEG-TS Annex B konverzióval; AAC ADTS → MP4; kodekparaméterek megőrzése; dekódolt képkockák egyezősége; dekódolt hangminták egyezősége; hibás bitstream filter visszautasítása; nem engedélyezett metadata-transzformáció visszautasítása. *\[v1.2\]* A TS célú tesztek a P2a fázis (MPEG-TS muxer) elkészültét feltételezik.

**Közös E2E tesztek:** import → idővonal → export → visszaimport; több klip összefűzése; VFR; nyitott GOP; több hangsáv; A/V-szinkron; megszakított export; sérült forrás; lezárási hiba; konténerkompatibilitási hiba.

**MPEG-TS muxer tesztek *\[v1.2\]*:** a 12.6/9 szerinti egység-, integrációs és teljes exportfolyam-tesztek.

*\[v1.5\]* Smart Cut és Smart Render tesztek: a 13.4 és a 15.4 fázistesztjei; az Auto nem indít újrakódolást megerősítés nélkül; H.264 határ-újrakódolás; MPEG-2, HEVC és AV1 forrásnál kulcskocka-kerekítés vagy teljes újrakódolási javaslat; rétegzett validáció (`BITSTREAM_EQUIVALENT` és „nem dekódolva ellenőrizve” állapot); atomikus írás megszakításkor és betelt lemeznél.

## 22. Fejlesztési sorrend *\[v1.4: összevonva a GOP Smart Cut, Smart Render és R1 tervvel\]*

A fázisok sorrendje változatlan; a GOP Smart Cut (13.4), a Smart Render (15.4) és az R1 terv munkacsomagjai (WP-00–WP-10) a megfelelő fázisba kerültek. A WP-00–WP-02 már folyamatban van a `feature/native-smart-cut-render*` ágakon (`crates/smart-edit`: capability registry, H.264/MPEG-2 médiaindex), még nincs a `main`-ben.

| Fázis | Tartalom | Stream Copy / TS | GOP Smart Cut | Smart Render | R1 WP | Függ |
| --- | --- | --- | --- | --- | --- | --- |
| P0 | Meglévő exportarchitektúra és képességek auditja | P0 audit; TS 12.6/1 | 1. fázis (leltár) | – | WP-00 | – |
| P1 | Közös capability registry, média/GOP/csomagindex, jogosultság | Packet Index, Eligibility Analyzer, idővonal-jogosultság | 1. fázis (`SmartCutCapabilities`) | 2. fázis (seam-képesség) + 1. fázis szakaszosztályai | WP-01, WP-02 | P0 |
| P2 | Strict Copy alapmotor | 9. fejezet, Strict Validator | GOP-határ mód, Bitazonos garancia | – | – | P1 |
| P2a | Natív MPEG-TS muxer | 12. fejezet | – | – | – | P1 |
| P3 | Compatible Copy alapmotor | 10–11. fejezet, Compatible Validator | Kompatibilis garancia | – | – | P2, P2a |
| P4 | GOP Smart Cut | – | 2. fázis (planner), 3. fázis (hibrid végrehajtó) | – | WP-03, WP-04 | P2, P3 |
| P5 | Smart Render | – | – | 1. fázis (planner), 3. fázis (vegyes végrehajtó) | WP-05, WP-06 | P4 |
| P6 | Kodek- és konténerbővítés | AV1, VP9, Opus, FLAC, WebM | 5. fázis | 5. fázis | WP-07 | P4, P5 |
| P7 | Qt6 GUI, CLI, teljes regresszió, csomagolás | 16–17. fejezet | 4. fázis | 4. fázis | WP-08, WP-09, WP-10 | P6, Qt port |

**P0 – Audit.** `cfa4-export`, a konténerkezelők és az FFmpeg-orákulum felmérése; API- és függőségi határok. A WP-00 eredménye (`docs/audit/smart-edit-baseline.md`) és a TS 12.6/1 integrációs jegyzete együtt adja a kimenetet.

**P1 – Közös index és jogosultság.** Streaminformációk; GOP-index; csomagindex; kompatibilitási döntések; vágási határok. *\[v1.4\]* A Stream Copy `copy/packet_index.rs` és `copy/eligibility.rs` modulja nem készül külön: a `cfa4-smart-edit` capability registryje (WP-01) és médiaindexe (WP-02) a közös alap, a Smart Render szakaszosztályai (15.3) adják az idővonal-jogosultságot. Kilépési feltétel: a támogatott és nem támogatott kombinációk tesztben lefedettek, elutasítás előtt nem nyílik kimeneti fájl.

**P2 – Strict Copy alapmotor.** Bitazonos csomagmásolás; konténermuxolás; timestamp-kezelés; Strict Validator; H.264/HEVC/AAC alaptesztek. Ez egyben a GOP Smart Cut „GOP-határ módja”. Kilépési feltétel: `STRICT_STREAM_COPY_E2E: PASS`.

**P2a – Natív MPEG-TS muxer.** A 12.7 kilenc lépése. Kilépési feltétel: `MPEGTS_MUX_E2E: PASS`.

**P3 – Compatible Copy alapmotor.** Bitstream-adapter; átalakítási allowlist; kompatibilitási elemzés; Compatible Validator. Kilépési feltétel: `COMPATIBLE_STREAM_COPY_E2E: PASS`.

**P4 – GOP Smart Cut.** Tiszta, fájlírás nélküli vágási planner (WP-03), majd hibrid mintamásolás és határ-GOP render (WP-04); több klip összefűzése, GOP-függőségek, A/V-szinkron. Első bekapcsolt cél: H.264 8-bit 4:2:0 zárt GOP, MP4/MOV. Kilépési feltétel: a 13.5 elfogadási kritériumai (`SMART_CUT_E2E: PASS`).

**P5 – Smart Render.** Szakaszbesoroló planner (WP-05), majd egy muxerbe író vegyes végrehajtó (WP-06); kodekparaméter-egyeztetés, vegyes exporttervezés. Első bekapcsolt profil: H.264, azonos méret/frame rate, zárt GOP, single-layer. Kilépési feltétel: a 15.5 elfogadási kritériumai (`SMART_RENDER_E2E: PASS`).

**P6 – Kodek- és konténerbővítés.** AV1, VP9, Opus, FLAC, WebM a Stream Copyhoz; HEVC, ProRes/DNx, MPEG-2, nested sequence, audio-only a Smart Cut és Smart Render számára, egyesével, conformance-kapuval (WP-07). Open GOP, B-frame reorder, interlace, VFR, HDR/10-bit/4:2:2 csak célzott conformance után. Az MPEG-TS a P2a-ban van.

**P7 – Qt6 GUI, CLI, teljes regresszió.** Öt exportüzemmód; közös Export Planner; Smart Cut és Smart Render parancsok és előnézet (WP-08); kompatibilitási jelentés; validációs visszajelzés; teljes regresszió és benchmark (WP-09); DEB, Flatpak, AppImage, Wayland tesztek (WP-10). Előfeltétel a Qt port elkészülte; egui nincs.

*\[v1.5\]* A döntések hatása a fázisokra: a közös atomikus író segéd a P2 része, és minden későbbi muxer erre épül. A natív bitstream-adapterek a P3-ban készülnek. A közös `ExportExecutionPlan` típust a P4 vezeti be egyklipes hatókörrel (GOP Smart Cut), a P5 bővíti idővonalra (Smart Render), a végrehajtó közös. A határ-újrakódolás a P4–P5-ben csak H.264; MPEG-2, HEVC és AV1 a P6-ban is csak gyártófüggetlen kódoló megléte után kapcsolható be. A P7-ben öt exportüzemmód van, a GOP Smart Cut a forrásmonitor művelete.

## 23. Kötelező kiadási feltételek

A kiadás csak akkor fogadható el, ha:

1. Mindkét Stream Copy mód külön választható.
2. Mindkét mód garantáltan újrakódolásmentes.
3. A Strict Copy megőrzi a kijelölt csomagpayloadokat.
4. A Compatible Copy csak igazoltan veszteségmentes átalakításokat használ.
5. Egyik mód sem kapcsol át rejtetten Smart Renderre.
6. Egyik mód sem kapcsol át rejtetten Full Renderre.
7. A GOP-határok kezelése helyes.
8. Az időbélyegek helyesek.
9. Az A/V-szinkron megfelel az exporttervnek.
10. A kimenet dekódolható.
11. A forrásfájlok sértetlenek.
12. A validáció eredménye részletesen naplózott.
13. A meglévő exporttesztek sikeresek.
14. A Qt6 GUI és a CLI ugyanazt a natív exportmotort használja.
15. *\[v1.2\]* Az MPEG-TS és M2TS kimenet teljesíti a 12.6/9 elfogadási feltételeit.

*\[v1.5\]* További kiadási feltételek: az Auto újrakódolást csak kifejezett megerősítés után indít; a GOP Smart Cut és a Smart Render ugyanazt a tervtípust és végrehajtót használja; minden export a közös atomikus író segéden megy át; a bitstream-adapterek natív Rust megvalósításúak; a validációs jelentés megkülönbözteti a szerkezeti PASS-t és a dekódolásos ellenőrzést.

## 24. Dokumentáció

Létrehozandó:

```text
docs/architecture/stream-copy-export.md
docs/specifications/stream-copy-api.md
docs/specifications/stream-copy-strict.md
docs/specifications/stream-copy-compatible.md
docs/specifications/stream-copy-bitstream-policy.md
docs/specifications/stream-copy-compatibility.md
docs/specifications/stream-copy-timestamp-rules.md
docs/specifications/stream-copy-validation.md
docs/specifications/mpegts-muxer.md          [v1.2]
docs/testing/stream-copy-test-matrix.md
docs/user/stream-copy-export-guide.md
docs/user/mpegts-export-guide.md             [v1.2]
```

*\[v1.2\]* A 12.6/1 feladat integrációs jegyzete a `docs/specifications/mpegts-muxer.md` első fejezete. Minden dokumentum legyen verziózott.

## 25. Végleges döntés

A CFA4 Stream-Copy Export két külön, egymástól műszakilag elhatárolt üzemmóddal valósul meg.

**STRICT BIT-EXACT STREAM COPY** – a kiválasztott tömörített médiaadatok bitazonos megőrzését biztosítja.

**COMPATIBLE STREAM COPY** – újrakódolás nélkül engedélyezi az igazoltan veszteségmentes bitstream-reprezentációváltásokat.

Mindkét üzemmód közös alapelve: **NO DECODE–REENCODE. NO HIDDEN TRANSCODING.**

A két mód ugyanazt a natív médiaindexet, GOP-elemzőt, időbélyegkezelőt, muxolási infrastruktúrát és exportfeladat-kezelőt használja, különböző jogosultsági és validációs szabályokkal. A két Stream Copy mód mellett a Smart Render és a Full Render külön, explicit exportstratégiaként marad elérhető.

*\[v1.2\]* A muxolási infrastruktúra része az MPEG-TS/M2TS kimenet, amelyet minden exportüzemmód használhat.

A fejlesztés a CFA4 meglévő `cfa4-export` alrendszerébe épül be, annak visszafelé kompatibilis bővítéseként. Az upstream (örökölt ős) projekt nem módosítható.

*\[v1.5\]* A két Stream Copy mód mellett a Smart Render az idővonal-hibrid, a GOP Smart Cut ennek egyklipes esete a forrásmonitorban. Az Auto csak javasol, és újrakódolást csak megerősítés után indít. Minden bitstream-átalakítás és muxolás natív Rust, az FFmpeg kizárólag CI-orákulum.

## Integrációs megjegyzések (v1.3)

A TS terv fő ütközése (FFmpeg/`libavformat` vs. a projekt "csak teszt-orákulum" szabálya) a natív Rust muxerről szóló döntéssel megszűnt. Az ellenőrzés alapja a `main` 24f061a commitja.

| # | Téma | Állapot | Hol van rögzítve |
| --- | --- | --- | --- |
| 1 | FFmpeg/`libavformat` muxer vs. projekt szabályok | Eldöntve 2026-10-27: natív Rust TS muxer a `cfa4-mpegts`-ben, FFmpeg csak CI-orákulum | 12. fejezet |
| 2 | "Nem készül saját csomagolómotor" | Törölve; a muxer a `crates/mpegts/src/tests.rs` tesztmuxeréből indul | 12.2, 12.6/4 |
| 3 | Automatikus muxeroldali bitstream filter | Megszűnt: minden átalakítás a Bitstream Adapterben, a muxer payload-átlátszatlan; TS cél hosszmezős H.264/HEVC-ből csak Compatible | 2.2, 11, 12.3, 12.4 |
| 4 | Utóellenőrzés ffprobe-bal | Termékben a saját demuxer ellenőriz, az ffprobe csak CI-ben | 12.6/7, 12.6/9 |
| 5 | MPEG-2 Video és MPEG audio első körben | Csak Stream Copyval (nincs ilyen kódoló); HEVC renderelés csak hardveren | 12.6/3 |
| 6 | Teljes fájl dekódolási próba | Mintavételes; nem támogatott profilnál "nem futott" állapot | 12.6/7 |
| 7 | Ideiglenes fájl és átnevezés | Nyitott: közös streamelő atomic commit segéd kell minden exportformátumhoz | 12.6/6, 20. fejezet |
| 8 | Stream Copy bitstream filterei (`h264_mp4toannexb` stb.) FFmpegből vagy natívan | Nyitott, de a TS döntéssel összhangban a natív Rust adapter a kézenfekvő; a filternevek referencia-azonosítók | 2.2, 11 |

### Ütközések a GOP Smart Cut és Smart Render tervvel (v1.4)

A két terv tartalma nagyrészt összhangban van a Stream Copy tervvel (két garancia, nincs csendes átváltás, MPEG-TS csak a muxer után, FFmpeg csak orákulum). Az alábbiakat kell eldönteni vagy egységesíteni:

| # | Téma | Ütközés | Javaslat |
| --- | --- | --- | --- |
| 9 | Auto mód | A 4. fejezet Auto módja Smart Renderre is léphet; a Smart Render terv szerint „nincs auto copy választás”, és a Smart Render explicit kapcsoló | Az Auto csak javaslatot adjon (előnézet + indok), a végrehajtáshoz explicit megerősítés kelljen, ha a javaslat újrakódolást tartalmaz |
| 10 | Két párhuzamos terv | `docs/plans/GOP-SMART-CUT` + `SMART-RENDER` (ág: `docs/gop-smart-cut-smart-render-plans`) és `CFA4-SMART-CUT-RENDER-R1` (ág: `feature/native-smart-cut-render`) ugyanazt fedik le, egyik sincs a `main`-ben | Az R1 maradjon a munkacsomag-terv, a két új terv a fázisok részletezése; merge előtt kereszthivatkozás |
| 11 | Modulhely | A tervek `crates/export/src/smart_cut.rs` és `smart_render.rs` fájlt írnak; az R1 a plannert az új `cfa4-smart-edit` (L3) crate-be teszi; a Stream Copy terv `crates/export/src/copy/*` modulokat | Tervezés (capability, index, planner) a `cfa4-smart-edit`-ben, végrehajtás (copy, smart cut, smart render executor) a `cfa4-export`-ban |
| 12 | Parancsnevek | `smartCut.plan/run/cancel`, `export.smartRender.plan/start/cancel`, `export.copy.*` | Egységes `export.*` névtér: `export.copy.*`, `export.smartCut.*`, `export.smartRender.*`, azonos igékkel (`plan`, `start`, `cancel`) |
| 13 | Bitazonos garancia újrakódolt határral | A saját H.264 kódoló SPS/PPS-e eltér a forrásétól; az MP4 író track-enként egy sample entryt, a Matroska egy `CodecPrivate`-et kezel | Bitazonos Smart Cut/Render csak több sample entry vagy `avc3`/in-band paraméterkészlet támogatásával; addig a határ-újrakódolás csak Kompatibilis garanciával, vagy a kombináció elutasítva |
| 14 | Smart Render referencia-összevetés | „Látható képkockái egyeznek a teljes render referenciaexporttal”: a másolt szakasz az eredeti, a referencia újrakódolt, bájtra nem egyezhetnek | Rögzíteni kell: frame count, frame time, sample count pontos egyezés; képtartalom a forrás dekódolt képéhez mérve, dokumentált tűréssel |

*\[v1.5\]* Állapot: a 9. (Auto mód), 11. (modulhely, 7. fejezet) és 12. (parancsnevek, 17. fejezet) tétel a Döntési napló 1–3. és 8. pontja szerint rendezve. A 13. tétel változatlanul érvényes: a határ-újrakódolás addig csak Kompatibilis garanciával fut. A 14. tétel a 18. fejezet v1.5 bekezdésében rögzítve. A 10. tétel (két párhuzamos terv a repóban) a repó oldalán nyitott. Az integrációs táblázat 7. és 8. sora a Döntési napló 6. és 4. pontjával eldöntve.
