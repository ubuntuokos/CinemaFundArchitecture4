# CFA4-CFA3-CREATIVE-001: a CFA3 Creative Studio forrásainak beillesztése a CFA4 tervébe

> **Állapot:** tervezet 1.0, **tulajdonosi jóváhagyásra vár**. Jóváhagyás előtt semmilyen
> fejlesztés nem indul belőle. · **Készült:** 2026-11-10 · **CFA4:** `main` @ `67b6666` ·
> **Bemenet:** [`docs/cfa3-creative-studio-sources.md`](../cfa3-creative-studio-sources.md)
> (70 megtartott forrás, PR #876) · **Összevetve:** [`VISION.md`](../../VISION.md),
> [`docs/roadmap.md`](../roadmap.md), [`docs/gaps.md`](../gaps.md),
> [ADR-0003](../adr/ADR-0003-aswf-and-conversion-sources.md),
> [Kdenlive-terv](CFA4-KDENLIVE-PLAN-001-FINAL-1.0.md),
> [Feliratozó terv 1.2](SUBTITLE-STUDIO-PLAN-001-FINAL-1.2.md),
> [OBJECT-MASK-PLAN-001](OBJECT-MASK-PLAN-001-2026-11-03.md),
> [`open-editors-parity.md`](../open-editors-parity.md),
> [`file-format-parity.md`](../file-format-parity.md)

## 0. Röviden

- A 70 megtartott forrás **nagy részének már van helye** a CFA4 tervében. Az ADR-0003 (OpenColorIO,
  OpenImageIO, OpenEXR, LibRaw, OpenFX/Natron, xSTUDIO, MaterialX), a Kdenlive-terv K26-os
  hangleválasztása, a Feliratozó terv, az OBJECT-MASK terv és a G12 MI-lista már lefedi őket. Ezekre
  **nem nyitok új feladatot**, csak megjelölöm, melyik meglévő terv veszi át a forrást (2. pont).
- **Valóban új munka hat munkacsomag** (CC0–CC5), összesen **~110–200 ügynökóra**:
  - CC0: licenctisztázás,
  - CC1: SVG-import,
  - CC2: OpenRaster/Krita-képek importja,
  - CC3: DAWproject hangátadás,
  - CC4: referencia alapú mastering,
  - CC5: C2PA tartalom-eredet.
- A nagy területekre (szinkron, 2D animáció, 3D, MI-generálás) ez a terv **nem indít munkát**.
  Csak rögzíti a jelölt forrásokat, hogy a későbbi saját tervük ne kezdje elölről (3. pont). Ez
  szándékos: a CFA3 gondja a túl sok, túl korai munka volt, és a párhuzamos tervek száma 5–10
  maradjon.
- **Ütemezés:**
  - a CC0 dokumentációs munka, jóváhagyás után mehet;
  - a CC1–CC4 csak a Qt6 GUI elkészülte után (`qt-parity.md` minden sora ✅);
  - a CC5 csak az MI-réteg után.

  Ma 90-nél több PR van nyitva, ezért korábbi indítást nem javaslok.
- **Hat döntés** vár a tulajdonosra (7. pont).

## 1. Kiindulás

A CFA3 Creative Studio nem kódot ad, hanem egy forrásnyilvántartást és egy felülettervet. A
nyilvántartás szűrt változata a [`cfa3-creative-studio-sources.md`](../cfa3-creative-studio-sources.md):
70 forrás, licenccel, ütemjelöléssel és azzal, hogy hol szerepel már a CFA4-ben.

A terv minden forrásról egyet dönt el:

1. **Meglévő terv viszi:** van már helye, nincs új teendő.
2. **Új munkacsomag:** a CFA4-ben nincs terve, kicsi, és illik a mostani célokhoz.
3. **Későbbi terület bemenete:** a terület (szinkron, 2D, 3D, MI-generálás) még terv nélküli.
   A forrás jelöltként vár rá.
4. **Csak ötlet:** AGPL vagy tisztázatlan licenc. Kód nem vehető át.

## 2. Összevetés a meglévő tervekkel

### 2.1 Meglévő terv viszi (nincs új teendő)

| Forrás(ok) | Hol van már | Megjegyzés |
|---|---|---|
| OpenColorIO | ADR-0003 A1 (orákulum), A2 (`crates/ocio`) | — |
| OpenImageIO, OpenEXR, LibRaw | ADR-0003 A3 | PSD, kamera-RAW, a hiányzó EXR-tömörítések. A **PhotoshopAPI** (BSD-3) az A3-ban rétegzett PSD-hez második jelölt lehet. A tervet nem bővítem vele, csak jelzem. |
| Natron (OpenFX-host, openfx-misc) | ADR-0003 A4, G11 | — |
| xSTUDIO, MaterialX | ADR-0003 A7 | — |
| OpenCut | `open-editors-parity.md` E13 | — |
| ComfyUI | `open-editors-parity.md` (külső alkalmazás oda-vissza, MI-réteg után) | — |
| Audio Separator, BS-RoFormer | Kdenlive-terv K26 (hangleválasztás, Demucs) | A K26 megvalósításakor a RoFormer-modellek (MIT kód) a Demucs mellett mérendő jelöltek. |
| vibe (helyi Whisper-átirat) | Feliratozó terv, G19 | — |
| OpenCV | OBJECT-MASK-PLAN-001 | — |
| darktable, RawTherapee, RawSpeed, Rawler | `crates/dng` (orákulum-kör), ADR-0003 A3 | A `crates/dng` clean-room módon készült. Forrásátvétel csak a licensing.md 3. és 5. pontja szerint, és csak ha a dekóder elakad. |
| hyperframes, Pixelle-Video, Monet, Donkey | `open-editors-parity.md` E1/E5 (szkriptelt, ügynökbiztos szerkesztés), ADR-0004 router | Mintaként az E1/E5 megvalósításakor, új teendő nélkül. |

### 2.2 Új munkacsomag (4. pont)

| Forrás(ok) | Munkacsomag | Miért új |
|---|---|---|
| a 9 „tisztázandó” licencű tétel, minden MI-modellsúly | **CC0** | A forráslistában nyitott licenckérdések vannak. |
| resvg, tiny-skia (Vello később) | **CC1** SVG-import | A `file-format-parity.md` szerint SVG-olvasás nincs. Tiszta Rust, MIT/Apache/BSD. |
| OpenRaster, Krita, libmypaint | **CC2** `.ora` / `.kra` import | A Krita a VISION.md 4. pontjában egyesítendő program, de rajzfájlja nem nyílik meg. |
| DAWproject, Ardour, Qtractor | **CC3** DAWproject hangátadás | Az AAF/OMF megvan, nyílt, modern DAW-csereformátum nincs. |
| Matchering | **CC4** referencia alapú mastering | Az Essential Sound-ban nincs ilyen. Az Audacity-képességekhez tartozik. |
| C2PA (c2pa-rs), AudioSeal | **CC5** tartalom-eredet (C2PA) | MI-vel generált tartalom jelölése. Nincs rá terv. |

### 2.3 Későbbi terület bemenete (3. pont)

| Terület | Jelölt források | Hol dől el |
|---|---|---|
| Szinkronizálás | Seed-VC (GPL-3), ClearerVoice-Studio (Apache-2.0), AudioSeal (MIT), a K26 leválasztása | VISION.md 4. pont „Szinkronizálás”. Saját terve még nincs. |
| 2D animáció | OpenToonz (BSD-3), Krita (GPL-3), libmypaint (ISC) | VISION.md 6.4 sorrend (2D animáció a publikálás után) |
| 3D / VFX | Blender (GPL), Cycles, OpenUSD, ACL, ozz-animation, OSPRay | VISION.md 8. pont: a 3D mélysége nyitott döntés. ADR-0003 A7. |
| Szín, kép, festés (szűrők) | G'MIC, G'MIC-Qt, CImg, Open Image Denoise, Lensfun, GEGL, babl, libvips, vkdt, digiKam, Hugin, Enblend/Enfuse | G11/G18 (bővítmények, effektmélység). A G'MIC és az OIDN felvétele a bővítményjelöltek közé tulajdonosi döntés (D4). |
| MI-generálás (kép, videó, zene) | Wan2.2, Helios, ViMax, Fooocus, Krita AI Diffusion, ACE-Step, DiffRhythm, YuE | G12 és az MI-réteg. A generáló modellekhez sok VRAM kell (a referenciagép számítási kártyája 16 GB-os RTX A4000), ezt modellenként mérni kell. |

### 2.4 Csak ötlet

| Forrás | Ok |
|---|---|
| RapidRAW | AGPL-3.0 (licensing.md: AGPL kód nem vehető át) |
| Zrythm | AGPL-3.0 + védjegyfeltételek |

## 3. Amit ez a terv nem csinál

- **Nem indít szinkron-, 2D-, 3D- vagy MI-generálási munkát.** Mindegyikhez saját terv kell,
  tulajdonosi jóváhagyással. A 2.3 pont csak a jelölteket adja nekik.
- **Nem nyúl a Qt6 GUI projekthez.** A Creative Studio felületterve (munkaterület-sáv,
  feliratpanel, MI-asszisztens panel) a forráslista 5. pontjában van. Ez **bemenet** a Qt6 GUI és
  a Feliratozó terv számára. Átvételükről a tulajdonos és a gazda dönt (`coordination.md` 2.8).
  Ez a terv nem vállal belőle feladatot.
- **Nem módosít más tervet vagy ADR-t.** Ahol a 2.1 pont egy meglévő tervre mutat, azt az a terv
  viszi tovább a saját ütemében. A D4 döntés (G'MIC, OIDN) is csak a tulajdonos jóváhagyása után
  kerülne be az ADR-0003-ba, külön PR-ban.
- **Nem hoz be felhős MI-szolgáltatást.** A CFA3 AI Video Tools tervének Runway- és Kling-adaptereit
  nem vesszük át (VISION.md 5. pont: alapvetően helyi).

## 4. Munkacsomagok

Minden munkacsomagra érvényes:

- egy munkacsomag egy PR, a gazda olvasztja be;
- teljesíti a kapukat (`cargo xtask ci`, `cargo test --workspace`, `cargo deny check licenses`);
- minden átvett kód bekerül a [`licensing.md`](../licensing.md) 5. pontjának nyilvántartásába;
- minden új funkció parancs (`crates/engine`), menüből elérhető, és ha kell, saját beállításpanelt
  kap;
- a felület csak Qt6-ban készül.

### CC0 · Licenc- és forrástisztázás · 3–6 óra · jóváhagyás után azonnal

**Tartalom:** a forráslista 9 „tisztázandó” tételének lezárása:

- **CImg, G'MIC:** melyik rész CeCILL-2.1, melyik CeCILL-C.
- **OpenUSD:** a TOST-licenc GPL-3-mal való összeférése.
- **G'MIC community filters:** licencfájl nincs, a szerzőktől kell tisztázni.
- **digiKam, Hugin, Enblend, Enfuse:** az eredeti KDE- és SourceForge-tárolón kell ellenőrizni.
- **YuE:** a modellsúly CC-BY-NC, ezt rögzíteni kell.

Ezen felül a modellsúly-licenc rögzítése minden MI-tételnél (a VISION.md 5. pontja előírja).

**Kész, ha:** a forráslista minden „tisztázandó” sora „kód is”, „csak ötlet” vagy „kizárva” lett,
forrásmegjelöléssel. Kódot nem érint.

### CC1 · SVG-állókép és -grafika import · 12–25 óra · Qt6 GUI után

**Tartalom:** SVG-fájl importja állóképként (`cfa4_media::STILL_EXTENSIONS`), a projekt felbontására
raszterizálva. Ha a klipet nagyítják, élesen újrarajzolódik. A megvalósítás a `resvg`, `usvg` és
`tiny-skia` crate-tel tiszta Rustban történik (MIT / Apache-2.0 / BSD-3), a rétegszabály szerinti
helyen (nem L0, mert az L0 csak a `cfa4-bitstream`-től függhet).

**Kész, ha:**

- a resvg saját tesztkészletének egy kiválasztott része a referencia-PNG-től legfeljebb
  1 LSB-vel tér el;
- sérült vagy rosszindulatú SVG-re hibaüzenet jön, nem összeomlás (fuzz-teszt);
- külső hivatkozást (http, fájl) az import nem tölt le.

### CC2 · OpenRaster (`.ora`) és Krita (`.kra`) import · 15–30 óra · Qt6 GUI után

**Tartalom:**

- **`.ora`:** a nyílt OpenRaster-specifikáció szerint ZIP + `stack.xml` + PNG-rétegek. A rétegek
  külön is importálhatók, a PSD-hez tervezett módon (rétegek sorozatként vagy beágyazott
  szekvenciaként).
- **`.kra`:** a Krita rétegformátuma nincs nyilvánosan specifikálva, ezért csak a fájlban lévő
  összesített kép (`mergedimage.png`) és az előnézet. Teljes réteg-import nincs.

A Krita (GPL-3) kódjából nem veszünk át, saját kód készül.

**Kész, ha:** a Krita és a MyPaint által mentett mintafájlok (a tesztben generálva, nem commitolva)
megnyílnak, és a kép a program saját exportjával egyezik. Hibás ZIP-re hibaüzenet jön.

### CC3 · DAWproject hangátadás · 25–45 óra · Qt6 GUI után

**Tartalom:** a sorozat hangsávjainak exportja DAWproject-formátumba (nyílt, MIT, XML + médiafájlok):

- sávok, klipek, fadek, hangerő- és panorámaautomáció,
- jelölők,
- a hivatkozott vagy kiírt hangfájlok.

Visszaolvasás csak a CFA4 saját exportjáé, kerekítési próbához. A megvalósítás a
`crates/interchange`-ben, a publikus XML-séma szerint, saját kóddal. Menü: Fájl ▸ Export ▸
DAWproject.

**Kész, ha:** legalább 5 saját tesztsorozat export–import oda-vissza útja után a sávok, a klipek
helye és hossza (`Tick` pontosan), a fadek és az automáció egyezik, és a fájl a hivatalos
XSD-sémán érvényes. Ha a tulajdonos egy DAW-ban (pl. Bitwig, Studio One) megnyitja, az kézi
próba, nem CI-kapu.

### CC4 · Referencia alapú mastering · 25–40 óra · Qt6 GUI után, a hangszerkesztési szakaszban

**Tartalom:** a felhasználó megad egy referenciadalt, a CFA4 a célhang spektrumát, hangosságát és
csúcsszintjét hozzá igazítja. Ez az Essential Sound új része, külön beállításpanellel. Alapja a
Matchering (GPL-3.0) algoritmusa, Rustra átírva a `crates/audio-dsp`-be, forrásmegjelöléssel és a
nyilvántartásba vezetve. Python-függőség nincs. Az ütemezés az ADR-0003 A6-tal (Rubber Band, LV2)
együtt, a VISION.md 6.4 hangszerkesztési szakaszában történik.

**Kész, ha:**

- a Matchering referencia-kimenetétől (tesztben, külső folyamatként futtatva) a hangosság legfeljebb
  0,5 LU-val, a sávos spektrum legfeljebb 1 dB-lel tér el;
- a true-peak nem lépi túl a beállított határt;
- 10 perc sztereó anyag feldolgozása gyorsabb a valós időnél a referenciagépen.

### CC5 · Tartalom-eredet (C2PA) · 30–50 óra · az MI-réteg után

**Tartalom:** exportkor opcionális C2PA-manifeszt, amely rögzíti:

- a CFA4 verzióját,
- a felhasznált MI-modelleket (az OBJECT-MASK terv `provenance` mezője alapján),
- a generált elemeket.

Importkor a meglévő C2PA-adat kiolvasása és megjelenítése. Alapja a `c2pa-rs` (MIT / Apache-2.0),
ha a függőségi fája átmegy a `cargo deny`-n. Ha nem megy át, előbb külön ADR kell. Az AudioSeal-féle
hang-vízjel nem része ennek, a szinkronterv dönt róla.

**Kész, ha:** a CFA4 exportja a C2PA hivatalos ellenőrző eszközén érvényes; aláírás nélkül
exportált fájlon nincs változás; hibás manifeszt importkor nem okoz összeomlást.

## 5. Ütemterv és függőségek

| Munkacsomag | Előfeltétel | Becslés (ügynökóra) |
|---|---|---|
| CC0 | jóváhagyás (D1) | 3–6 |
| CC1 | Qt6 GUI kész (`qt-parity.md` minden sora ✅) | 12–25 |
| CC2 | Qt6 GUI kész; a PSD-hez tervezett rétegkezelés (ADR-0003 A3) után vagy azzal együtt | 15–30 |
| CC3 | Qt6 GUI kész | 25–45 |
| CC4 | Qt6 GUI kész + a hangszerkesztési szakasz indul (VISION.md 6.4, ADR-0003 A6) | 25–40 |
| CC5 | az MI-réteg üzembe helyezése, OBJECT-MASK `provenance` | 30–50 |
| **Összesen** | | **~110–196** |

A CC1–CC3 egymástól független, sorrendjüket a gazda a nyitott PR-ok számához igazítja. Ha a
nyitott PR-ok száma a `coordination.md` 2.6 korlátja fölött van, új csomag csak a tulajdonos
kérésére indul.

## 6. Kockázatok

- **Függőségi fa (CC1, CC5).** A `resvg` és a `c2pa-rs` sok crate-et hoz. Ha bármelyik licence
  vagy RustSec-állapota nem megy át a `cargo deny`-n, a csomag megáll és jelez, nem kerüli meg a
  szabályt.
- **Kódátvétel GPL-3 forrásból (CC4).** Fájlonkénti licencellenőrzés és nyilvántartás kell. Ha a
  Matchering egy része más licencű, azt nem vesszük át.
- **Bővülő hatókör.** A 2.3 pont területeit vonzó lenne „menet közben” elkezdeni. Ez a terv
  kifejezetten nem engedi: mindegyikhez külön, jóváhagyott terv kell.
- **Hardver (2.3, MI-generálás).** A generáló modellek VRAM-igénye meghaladhatja a referenciagép
  16 GB-os számítási kártyáját. Ezt modellenként mérni kell, mielőtt bármi a tervbe kerül.

## 7. Tulajdonosi döntések

| # | Kérdés | Javaslat |
|---|---|---|
| **D1** | Elfogadod-e a terv hatókörét: új munka csak a CC0–CC5, minden más a 2.1 és 2.3 pont szerint? | Igen |
| **D2** | Ütemezés: CC0 most, CC1–CC4 a Qt6 GUI után, CC5 az MI-réteg után. Vagy induljon a CC1 (SVG) motoroldala most, felület nélkül? | Qt6 után (a nyitott PR-ok száma miatt) |
| **D3** | CC5 (C2PA) kell-e egyáltalán, vagy maradjon ötlet? | Kell, de az MI-réteg után |
| **D4** | Felkerüljön-e a G'MIC (külön folyamatban, CLI-n) és az Open Image Denoise az ADR-0003 6. pontjának bővítményjelöltjei közé? | Igen, jelöltként (külön PR az ADR-0003-hoz) |
| **D5** | A szinkron 2.3 pontbeli jelöltjei (Seed-VC, ClearerVoice, AudioSeal) várjanak a saját szinkrontervre? | Igen |
| **D6** | Az AGPL-es RapidRAW és Zrythm csak ötletként szerepeljen (kód nem)? | Igen (licensing.md szerint kötelező) |

## Változástörténet

| Dátum | Verzió | Változás |
|---|---|---|
| 2026-11-10 | 1.0 tervezet | Létrehozva a [`cfa3-creative-studio-sources.md`](../cfa3-creative-studio-sources.md) alapján, a `main` @ `67b6666` terveivel összevetve |
