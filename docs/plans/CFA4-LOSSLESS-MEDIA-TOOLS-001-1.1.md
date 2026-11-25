# CFA4-LOSSLESS-MEDIA-TOOLS-001
## Veszteségmentes vágási és média-előkészítési képességek a CFA4-ben

**Dokumentumverzió:** 1.1
**Állapot:** **végleges, a tulajdonos által jóváhagyva** (István, 2026-11-13); a megvalósítás fázisonként külön jóváhagyással indul (14. pont)
**Előző változat:** 1.0 (tervezet, 2026-11-12)
**Alapul szolgáló funkcionális referencia:** LosslessCut – https://github.com/mifi/lossless-cut (csak funkcionális referencia, kódot nem veszünk át)
**Célplatform:** Ubuntu 26.04 LTS és kompatibilis változatai, amd64
**Felület:** a CFA4 Qt6-felülete (Qt6 Widgets + PySide6, ADR F8)
**Médiamotor:** a CFA4 Rust-motorja, a `cfa4-route` útválasztóval és az ADR 0005 szerinti kiegészítő FFmpeg- és GStreamer-háttérrel
**Adatkezelés:** CFA4-projektfájl (`.fcproj`); a PostgreSQL legfeljebb opcionális, későbbi index
**F0-jelentés:** [`docs/audit/LOSSLESS-MEDIA-TOOLS-001-F0-2026-11-13.md`](../audit/LOSSLESS-MEDIA-TOOLS-001-F0-2026-11-13.md)

### Változások az 1.0-hoz képest (tulajdonosi döntések, 2026-11-13)

1. **Nem készül külön alkalmazás.** A LosslessCut funkciói közül csak a CFA4-ből **hiányzó
   képességek** kerülnek át, a CFA4 meglévő helyeire. Nincs külön indító, `.desktop` bejegyzés,
   ablak vagy terméknév. A „Media Cutter” csak a terv munkaneve.
2. **A médiafeldolgozás alapja a CFA4 Rust-motorja** (ADR 0005), nem az FFmpeg. A képességek
   saját FFmpeg-hívást vagy -parancsot nem tartalmaznak.
3. **A felület PySide6**, és a `cfa4-cli serve` folyamaton át éri el a motort. Minden művelet
   engine-parancs.
4. **A szegmensek projektadatok:** tartományjelölők és subclipek a `.fcproj`-ban. Külön
   munkamenetfájl nem készül.
5. **A remux és a streamkezelés** a CFA4-SCE-001 P2/P2a/P3 fázisával egyesül.
6. A F0-jelentés 2–9. fejezetének javaslatai beépültek; a részletek a jelentésben vannak.
7. **Minden részleges és hiányzó követelmény megoldást kap** (3.4 pont, K12–K16).

---

## 1. A fejlesztés célja

A CFA4 kapja meg azokat a veszteségmentes vágási és média-előkészítési képességeket, amelyek ma
hiányoznak belőle. Így egy forrás gyors szegmensekre bontása, újrakódolás nélküli vágása,
elemzése és az idővonalra adása a CFA4 meglévő felületén belül, kevés lépésben elvégezhető.

A képességek a meglévő helyükre kerülnek: forrásmonitor, Jelölők panel, Export ablak, Eszközök
menü, illetve a motor meglévő parancscsaládjai. Külön kódbázis, motor vagy alkalmazás nem
készül (VISION 1. és 2.7).

## 2. Alapvető tervezési elvek

- A CFA4 Qt6-felülete; az egui be van fagyasztva, ezért új felület csak a Qt6-ba kerül.
- Linux-only, Ubuntu 26.04 LTS, amd64.
- A CFA4 Rust-motorja az alap. Az FFmpeg-háttér csak a `cfa4-route` útválasztón át dolgozhat, ott,
  ahol a saját út nem képes a feladatra. A stream copy, a Smart Cut és a muxolás saját Rust-kódon
  fut (tulajdonosi döntés, 2026-10-27), az FFmpeg ott csak tesztorákulum.
- Ahol technikailag lehetséges, újrakódolás nélküli műveletek.
- A forrásfájl soha nem módosul; minden kimenet új fájl, atomikus írással.
- Újrakódolás csak kifejezett megerősítéssel; a módszer mindig előre látható.
- Minden művelet engine-parancs (CLI, MCP, vezérlőcsatorna), és minden vezérlőnek automatizálási
  azonosítója van.
- Minden idő `cfa4_time::Tick`; f64 másodperc nem használható.
- Never crash (AGENTS.md §0): hibás bemenetre kezelhető hiba jön, összeomlás nincs.
- Az eredeti LosslessCut a CFA4 mellett telepíthető és használható marad.

## 3. Funkcionális követelmények: hiányzó képességek

A már meglévő képességek (nem kell átvenni):

- GOP-határ és képkockapontos forrásvágás (`export.analyze/plan/start`, `scope: source`);
- a kért és a megvalósítható vágáspont kijelzése, csendes igazítás nélkül;
- bitazonos és kompatibilis garancia;
- határ-újrakódolás csak megerősítéssel;
- AAC/PCM hangmásolás;
- kimenet-ellenőrzés és atomikus írás;
- streamadatok megjelenítése (`mediaBrowser.probe`, `cfa4-cli probe`);
- jelölők, fejezetjelölők, EDL-export;
- képkocka-export PNG-be.

### 3.1. Ebben a tervben készülő képességek

| # | Képesség | Hely a CFA4-ben | Követelmények |
|---|---|---|---|
| K1 | Több vágási tartomány egy forráson, egyenkénti vagy együttes export újrakódolás nélkül | Forrásmonitor ▸ Smart Cut Source… ablak; motor: `export.plan/start` több tartományra, a `jobs.*` / `export.queue.*` sorban | Minden tartománynál látszik a módszer (másolás, határ-újrakódolás) és a tényleges határ; az újrakódolás egyszer, a teljes kötegre kér megerősítést |
| K2 | Szegmens mint projektadat | Tartományjelölő a mesterklipen, a `.fcproj`-ban; szegmensnézet a Jelölők panelen | Létrehozás, törlés, átnevezés, címke, megjegyzés, exportsorrend; mentés és visszatöltés a projekttel; a határok `Tick`-ben, a forrás PTS-idővonalán |
| K3 | Szegmensek átadása az idővonalra | Új parancs: kijelölt tartományok → subclipek vagy idővonal-klipek | A forráshoz és az időbélyegekhez fűződő kapcsolat megmarad |
| K4 | Jelenetfelismerés forrásklipre | A `clip.sceneEditDetection` kiterjesztése projektelemre és forrásmonitorra | Állítható érzékenység; az eredmény tartományjelölő-javaslat, kézzel javítható; a forrás nem változik |
| K5 | Fekete- és csendszakaszok keresése | A `cfa4-qc` elemzői (fekete kép, csend) új, jelölőt adó parancsként | Állítható küszöb és minimális hossz; az eredmény jelölő vagy tartomány; automatikus törlés nincs |
| K6 | Jelölők CSV-importja | `markers.importCsv`, a `markers.exportCsv` párja | Az időalap ellenőrzése; eltérés esetén csendes konverzió helyett hiba vagy kérdés |
| K7 | Képkocka-export bővítése | `file.exportFrame` | Formátum, minőség és méret választása; teljes újrakódolás nélkül |

### 3.2. A CFA4-SCE-001 / SMART-CUT-RENDER-R1 területén készülő képességek

Ezek a SCE-001 ütemezésében és annak projektjében készülnek. Ez a terv csak igényként rögzíti
őket (D7, a gazdával egyeztetve).

| # | Képesség | Megjegyzés |
|---|---|---|
| K8 | Smart Cut megszakítása, műveleti napló | WP-09 18. eset |
| K9 | Matroska (MKV/WebM) forrás Smart Cutja | `cfa4-smart-edit` + `cfa4-export` |
| K10 | Streamválasztás, -kinyerés, hang- és feliratsáv hozzáadása vagy cseréje, remux (MKV↔MP4), kompatibilitás-ellenőrzés | SCE-001 P2/P2a/P3; az FFmpeg-háttér itt csak külön tulajdonosi döntéssel (D3) |
| K11 | Forgatás és metaadatok szerkesztése, besorolással (konténer / stream / képtartalom) | SCE-001 P3 |

### 3.3. Közös követelmények

- **Időalap:** az importált időpontokat a forrás időalapjához kell mérni; csendes konverzió
  nincs.
- **Kompatibilitás:** a támogatott kodek- és konténerkombinációkat a `cfa4-smart-edit` registry
  adja (`Registry::report()`), nem a fájlkiterjesztés.
- **Visszajelzés:** minden műveletnél látszik a művelet típusa, a forrás, a cél, a módszer, a
  várható és a tényleges állapot, a hibaüzenet és az esetleges újrakódolás.
- **Szövegek:** a felület minden szövege a közös fordítási katalógusba kerül (`i18n/`; az
  áthelyezésig a meglévő katalógusokba).

### 3.4. Teljessé tétel: minden részleges és hiányzó követelmény megoldása

Tulajdonosi döntés, 2026-11-13: „A hiányzó követelményekre megoldás kell, a részben meglévőket
meg teljessé kell tenni.” Ezért az F0-jelentés 2. fejezetének minden 🟡 és ❌ sora megoldást
kap. Az alábbi táblázat a K1–K11 kiegészítése (K12–K16), és azt is megadja, mi számít
„teljesnek”.

| 1.0 pont | Hiány (F0-jelentés 2. fejezet) | Megoldás | „Teljes”, ha | K | Terület |
|---|---|---|---|---|---|
| 3.1 | Több szegmens egymás utáni feldolgozása | Többtartományos `export.plan/start`, kötegként a jobsorban | N tartomány egy paranccsal, egyenkénti és összefűzött kimenettel | K1 | ez a terv |
| 3.1 | Határ-újrakódolás csak H.264/HEVC, a HEVC csak NVENC-cel | MPEG-2: **már kész** (`boundary_reencode_mpeg.rs`, WP-09e; frissítendő a WP-09 jelentés). HEVC és AV1 gyártófüggetlenül: a CFA4 FFmpeg-buildjének VA-API kódolói (`hevc_vaapi`, `av1_vaapi`) a `cfa4-route`-on át, CPU-tartalékként x265 (restricted extras), illetve SVT-AV1 | H.264, HEVC, AV1, MPEG-2 képkockapontos vágása AMD/Intel GPU-n és CPU-n is | K12 | SCE-001 / R1; **tulajdonosi döntés, 2026-11-13: engedélyezve** (VA-API-kódolók a `cfa4-route`-on át, CPU-tartalék x265 / SVT-AV1); ez feloldja a SCE-001 7. döntésének gyártófüggetlen-kódoló feltételét HEVC-re és AV1-re; az SVT-AV1 bekerülése előtt licencellenőrzés (`docs/licensing.md` §6) |
| 3.1 | Matroska forrás vágása nincs | MKV/WebM demux → GOP-index → csomagmásolás → `MkvWriter`, a meglévő MP4-út mintájára | H.264/HEVC/AV1/VP9 MKV-forrás GOP-határ és képkockapontos vágása, MKV- és MP4-kimenet | K9 | SCE-001 / R1 |
| 3.1 | Hangmásolás csak AAC és PCM; MP3, Opus, AC-3, E-AC-3, ALAC, FLAC eldobva | `audio_copy` bővítése kodekenkénti keretezéssel és pre-roll-szabállyal (Opus: pre-skip; AC-3/E-AC-3: szinkronkeret; MP3: bit-reservoir miatt +1 keret) | Minden felsorolt kodek másolva, A/V-szinkron mérve (`verify_audio_copy`) | K15 | SCE-001 / R1 |
| 3.2 | Szegmenslista nincs, mentés/átadás nincs | Tartományjelölő + szegmensnézet; átadás subclipként vagy idővonalra | Lásd K2, K3 | K2, K3 | ez a terv |
| 3.3 | Az adatfolyamok listázásának teljessége nem igazolt | `probe` kiegészítése: adat-, időkód-, melléklet- (MKV attachment), fejezet- és feliratfolyamok minden támogatott konténerben; ffprobe-orákulumos teszt | Minden folyam megjelenik nyelvvel, kodekkel, csatornával, felbontással, azonosítóval; a CFA4 és az ffprobe listája egyezik | K13 | ez a terv |
| 3.3 | Streamválasztás és -kinyerés, hang/felirat hozzáadása vagy cseréje | Strict/Compatible Copy végrehajtó, folyamonkénti kiválasztással és külső sáv hozzáadásával | Minden folyam kiválasztható, külön fájlba kinyerhető, cserélhető; ismeretlen jelentőségű folyamot automatikusan nem dob el | K10 | SCE-001 P2/P3 |
| 3.4 | Remux nincs teljes fájlra | Ugyanaz a végrehajtó teljes tartománnyal; a natív TS-muxer bővítése (P2a) | MKV↔MP4, MP4/MKV→TS/M2TS, előzetes kompatibilitási jelentés | K10 | SCE-001 P2/P2a/P3 |
| 3.5 | Jelenetfelismerés csak idővonal-klipre | A `clip.sceneEditDetection` forrásfájlra és projektelemre, előnézettel | Lásd K4 | K4 | ez a terv |
| 3.6 | Fekete-felismerés csak QC-jelentés | Állítható küszöbű parancs, eredmény jelölő | Lásd K5 | K5 | ez a terv |
| 3.7 | Csendfelismerés csak QC-jelentés, fix küszöb | Állítható dBFS-küszöb és minimális hossz, eredmény jelölő | Lásd K5 | K5 | ez a terv |
| 3.8 | Fejezetek a vágott kimenetben | Az MP4 (`chpl`) és MKV fejezetírás már kész; a Smart Cut kimenete vigye át a tartományba eső fejezeteket, eltolva | A vágott fájl fejezetei ffprobe-bal ellenőrizve | K14 | ez a terv |
| 3.8 | CSV-import, időalap-ellenőrzés | `markers.importCsv`; az EDL-import már van (`file.import`, `crates/engine/src/interchange.rs`), kiegészítve tartományjelölővé alakítással | CSV és EDL oda-vissza adatvesztés nélkül, eltérő időalapnál hiba vagy kérdés | K6 | ez a terv |
| 3.9 | Metaadat- és forgatásszerkesztés besorolással | Konténer-, stream- és tartalomszintű metaadat-szerkesztés a Compatible Copy szabályaival | Minden módosítás előtt látszik a szintje; a forgatás konténerszinten, újrakódolás nélkül | K11 | SCE-001 P3 |
| 3.10 | Képkocka-export csak PNG | `file.exportFrame` bővítése (PNG, JPEG, TIFF, WebP; minőség, méret) | Formátum, minőség és méret választható | K7 | ez a terv |
| 4.3 / 7 | Műveleti napló, megszakítás | `jobs.cancel` a Smart Cutra; műveleti napló az export-összegzésben | Megszakítás után nincs részleges fájl; a napló minden műveletet rögzít | K8 | SCE-001 / R1 |
| 10 | Nyitott tesztesetek (WP-09: eltérő timescale, VFR, HDR, megszakítás) | Hiányzó tesztek pótlása, generált fixture-ökkel | A WP-09 mátrix minden sora ✅ vagy indokolt ⚪ | K16 | SCE-001 / R1 + ez a terv |

**A „Terület” oszlop:** a SCE-001 / R1 területén lévő megoldásokat ez a terv **követelményként**
rögzíti. Azokat a SCE-001 projekt valósítja meg, a gazdával egyeztetett sorrendben (D7); a
CLAUDE.md szerint egy projekt nem veheti át egy másik projekt feladatát. Ha a tulajdonos és a
gazda úgy dönt, egy sor átkerülhet ebbe a tervbe.

## 4. Felhasználói felület

- **Forrásmonitor:** tartománylista és többtartományos Smart Cut (K1), szegmensek átadása az
  idővonalra (K3), elemzések indítása (K4, K5).
- **Jelölők panel:** szegmensnézet, címkék, megjegyzések, sorrend, CSV-import (K2, K6).
- **Export ablak:** a Smart Export meglévő módjai; a streamkezelés a SCE-001 szerint (K10).
- **Eszközök menü:** a meglévő Videóeszközök ▸ Smart Cut Source… belépés.
- A Chromium-alapú LosslessCut felülete és lejátszása nem minta.

## 5. Technikai architektúra

```text
CFA4 Qt6-felület (python/cfa4_qt, PySide6) – nem számol, nem épít parancsot
  -> EngineClient (cső, olvasószál, Qt-jelek)
     -> cfa4-cli serve (fejetlen Rust Session)
        -> engine-parancsok: export.*, markers.*, clip.sceneEditDetection, qc.*, file.exportFrame, jobs.*
           -> cfa4-smart-edit, cfa4-export, cfa4-qc, cfa4-render, cfa4-media
              -> cfa4-route -> (cfa4-ffmpeg / GStreamer, csak tartalékként)
```

- **Motor:** a terv 1.0 „FFmpeg-vezérlőréteg” feladatait (bemenet ellenőrzése, streamadatok,
  feladatindítás és -felügyelet, kimenet-ellenőrzés, naplózás) a motor meglévő parancsai és a
  `jobs.*` sor látják el. Új parancs a K1–K7-hez kell.
- **Rétegzés:** a `cargo xtask layers` szerint az L5 alatti réteg nem függhet UI-tól.
- **Adatok:** a szegmensek a `.fcproj`-ban vannak. A média útvonallal hivatkozott; az
  elérhetetlen forrást a meglévő Link Media / `media.relink` / `media.findMissing` kezeli. A
  PostgreSQL opcionális, későbbi index (File Services `files.*`, Media Intelligence `mi` séma),
  saját séma nélkül. Minden képesség PostgreSQL nélkül is teljes értékűen működik.

## 6. CFA4-integráció

- Videoszerkesztő: a szegmensek eleve projektadatok; a K3 adja át őket az idővonalra.
- Media Intelligence: később a `mi` séma találataiból tartományjelölő-javaslatok jöhetnek létre.
  Ez nem része a kötelező első kiadásnak.

## 7. Feldolgozási biztonság

- Az eredeti fájl soha nem íródik felül; a célfájl létezése esetén megerősítés kell.
- Atomikus írás (ideiglenes fájl, fsync, átnevezés); részleges kimenet nem marad.
- A kimenet ellenőrzése (`verify_*`, `qc.run`).
- Az útvonalak és fájlnevek ellenőrzése (ellenséges bemenet).
- Minden újrakódolás előre jelzett, és megerősítéshez kötött.

## 8. Licenc és függőségek

- A CFA4 GPL-3.0-or-later. A LosslessCut GPL-2.0-only, ezért **kódja, felületi szövegei, ikonjai
  és gyorsbillentyű-táblái nem vehetők át**. Csak a nyilvános dokumentáció és a program
  viselkedése szolgálhat funkcionális referenciaként (AGENTS.md §2, clean-room). A LosslessCut
  neve, ikonja és kifejezései a termékben nem jelenhetnek meg.
- FFmpeg: csak a CFA4 saját 9.0.2-buildje (`--enable-gpl --enable-version3`, soha
  `--enable-nonfree`), a `cfa4-ffmpeg` crate-en át; új FFmpeg-opció nem kell.
- Új külső függőség nem tervezett. Ha mégis kell: `cargo deny check licenses` és a
  `docs/licensing.md` §6 ellenőrzése.
- A H.264/HEVC határ-újrakódoláshoz kódoló kell; az x264/x265 a `cfa4-restricted-extras`
  csomagban van. A felület jelzi, ha nincs telepítve.
- Fizetős rész nincs.
- A végleges licencmegfelelőséget jogi ellenőrzés támasztja alá.

## 9. Fejlesztési fázisok

| Fázis | Tartalom | Kilépési feltétel |
|---|---|---|
| F0 | Architektúra- és követelményellenőrzés | **Kész** (F0-jelentés, 2026-11-13) |
| F1 | K1, K2, K3: többtartományos vágás, szegmensek mint projektadat, átadás az idővonalra | A támogatott tesztfájlokon (registry-kombinációk) megbízható vágás, mentés és visszatöltés |
| F2 | K4, K5: jelenet-, fekete- és csendfelismerés jelölőkként | Reprodukálható találatok, kézi javítás |
| F3 | K6, K7, K13, K14: CSV/EDL-import, képkocka-export, teljes streamlista, fejezetek a vágott kimenetben | Adatvesztés nélküli import és export, időalap-ellenőrzés, ffprobe-egyezés |
| – | K8–K12, K15, K16 | A SCE-001 / R1 ütemezésében, a gazdával egyeztetve; a K12 HEVC/AV1-része engedélyezve (2026-11-13) |
| F4 | Stabilizálás | A 10. pont minden kötelező tesztjének dokumentált eredménye |

Az F1 a Qt6-paritás lezárása előtt is indulhat (D1), mert csak a meglévő motorparancsokra épül.
Az F1 előtt az átfedést jelezni kell a gazdának (D7; [F0-jelentés](../audit/LOSSLESS-MEDIA-TOOLS-001-F0-2026-11-13.md) A. melléklet).

## 10. Kötelező tesztelési területek

| Terület | Ellenőrzés |
|---|---|
| Veszteségmentes vágás | Payload-egyezés, időbélyegek, vágási határok |
| Kulcsképkockák | A kért és a tényleges határ, csendes igazítás nélkül |
| Újrakódolás | Csak megerősítéssel, csak a határokon |
| Hang | Szinkron, csatornák, eldobott sávok jelzése |
| Szegmensek | Mentés, visszatöltés, átadás az idővonalra |
| Elemzések | Jelenet-, fekete- és csendtalálatok pontossága, küszöbök |
| CSV/EDL | Import-export és időalap |
| Hibatűrés | Sérült fájlok, megszakítás, teli lemez |
| Adatvédelem | Az eredeti média érintetlen |
| Teljesítmény | CPU, RAM, GPU és I/O a referenciagépen |

Kiegészítő esetek a WP-09 jelentésből: eltérő timescale, VFR, HDR, nem AAC/PCM hangsávok,
megszakított export. Az FFmpeg csak orákulum (`cfa4_testkit::ffmpeg_or_skip`). A fixture-ök a
`target/fixtures/` alá generálódnak, médiát nem commitolunk.

## 11. Elfogadási kritériumok

1. A 3.4 pont minden sora a „Teljes, ha” oszlop szerint teljesül; a K1–K7, K13 és K14 ebben a tervben, a többi a SCE-001-ben.
2. Az újrakódolást igénylő eseteket a rendszer előre jelzi, és megerősítést kér.
3. Az eredeti médiafájl nem módosul.
4. A szegmensek a projekttel menthetők és visszatölthetők.
5. Az automatikus elemzések eredménye kézzel javítható.
6. A szegmensek átadása az idővonalra működik.
7. A hibakezelés és a műveleti visszajelzés megfelelő.
8. A minőségkapuk zöldek: `cargo fmt --check`, clippy, `cargo test --workspace`,
   `cargo xtask layers`, `cargo xtask assets`, valamint a Python-ellenőrzések.
9. A licencellenőrzés lezárult.

## 12. Telepítés és együttélés

A képességek a CFA4 meglévő csomagjaival települnek (`.deb`, tar.gz, AppImage, Flatpak), új
csomag, programfájl, indító vagy MIME-típus nélkül. A LosslessCut-tal nincs ütközés: eltérő a
program- és csomagnév, a konfigurációs és gyorsítótár-útvonal, a CFA4 nem nyit portot, és a
CFA4 nem teszi magát csendben alapértelmezett videóalkalmazássá. Az AppImage és a Flatpak
Qt-támogatása a CFA4-QT6-FINISH-001 terv feladata.

## 13. Későbbi bővítési lehetőségek

Nem részei a kötelező első kiadásnak: kötegelt feldolgozás több forrásfájlon, automatikus
szegmenselnevezés, MI-alapú jelenet- és tartalomfelismerés, Media Intelligence-alapú
szegmenskeresés, előkészítési profilok, többkamerás előkészítés.

## 14. A terv végrehajtásának határai

A terv jóváhagyása önmagában nem jelent engedélyt kód írására, a repó módosítására, függőségek
telepítésére, csomagok készítésére vagy külső forráskód átvételére. Minden fázis indításához
külön jóváhagyás kell.

---

**Végső tervezési döntés:** a LosslessCut funkciói közül a CFA4-ből hiányzó képességek kerülnek
át, a CFA4 meglévő Qt6-felületébe és Rust-motorjába, önálló implementációként. Külön alkalmazás
nem készül, és a LosslessCut forráskódja nem kerül át a CFA4-be. Az első fejlesztési cél a
többtartományos veszteségmentes vágás és a szegmensek projektadatként kezelése (F1).
