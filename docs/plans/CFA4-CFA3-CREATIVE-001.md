# CFA4-CFA3-CREATIVE-001: a CFA3 Creative Studio képességeinek átvétele a CFA4-be

> **Állapot:** tervezet 1.1, **tulajdonosi jóváhagyásra vár**. Jóváhagyás előtt semmilyen
> fejlesztés nem indul belőle. · **Készült:** 2026-11-10, átdolgozva 2026-11-11 · **CFA4:**
> `main` @ `67b6666` · **Bemenet:** [`docs/cfa3-creative-studio-sources.md`](../cfa3-creative-studio-sources.md)
> (70 megtartott forrás, PR #876) · **Összevetve:** [`VISION.md`](../../VISION.md),
> [`docs/roadmap.md`](../roadmap.md), [`docs/gaps.md`](../gaps.md),
> [ADR-0003](../adr/ADR-0003-aswf-and-conversion-sources.md),
> [ADR-0004](../adr/ADR-0004-compute-routing.md),
> [Kdenlive-terv](CFA4-KDENLIVE-PLAN-001-FINAL-1.0.md),
> [Feliratozó terv 1.2](SUBTITLE-STUDIO-PLAN-001-FINAL-1.2.md),
> [OBJECT-MASK-PLAN-001](OBJECT-MASK-PLAN-001-2026-11-03.md),
> [`open-editors-parity.md`](../open-editors-parity.md),
> [`file-format-parity.md`](../file-format-parity.md)

## 0. Röviden

- **Mind a 70 forrás benne van.** 11 területen **51 munkacsomag** (CS-…) fedi le őket, a későbbi,
  nagy területeket is: szinkron, 2D animáció, 3D, MI-generálás, zene.
- **A sorrendet a tulajdonos dönti el, igény szerint.** A terv nem ütemez. Csomagonként csak a
  **technikai előfeltételt** adja meg (mi kell hozzá, hogy egyáltalán elkészülhessen), és azt,
  hogy melyik **korábbi tulajdonosi döntés** érinti. Ilyen például: a felület csak Qt6-ban készül;
  MI-funkció csak az MI-réteg után jön. Ezeket a tulajdonos csomagonként felülbírálhatja.
- **Minden csomag külön jóváhagyással indul.** A terv elfogadása a tartalmat hagyja jóvá, nem
  indít el semmit.
- Ahol egy képességnek már van helye egy meglévő tervben, a csomag oda mutat. Ilyen az ADR-0003,
  a Kdenlive K26, a Feliratozó terv, az OBJECT-MASK terv és az open-editors E3. **Nem jön létre
  párhuzamos feladat**, de a képesség itt is szerepel, hogy a terv teljes legyen.
- Becslés: az új munka **~2 300–4 500 ügynökóra**. Ehhez jön a meglévő tervekben már becsült
  rész. A 3D, a festés/animáció és a szinkron becslése a legbizonytalanabb, ezekhez indítás előtt
  részletes csomagterv készül.

## 1. Elvek

1. **Képességet veszünk át, nem alkalmazást** (tulajdonosi döntés, 2026-10-25). Egy forrásból
   csak azt, ami a CFA4-ből hiányzik vagy jobbá teszi.
2. **Licenc** ([`licensing.md`](../licensing.md)):
   - GPL-3-kompatibilis kód átvehető, forrásmegjelöléssel és az 5. pont nyilvántartásában.
   - Az AGPL és a nem szabad licencű forrásból csak ötlet vehető át, saját kóddal.
   - Kodek és formátum alapból specifikációból készül (AGENTS.md §2).
3. **Gyártófüggetlenség.** Minden gyorsítás a `cfa4-route` routeren át megy (ADR-0004), NVIDIA-kötés
   nélkül.
4. **Helyi MI** (VISION.md 5. pont). Felhős szolgáltatás nincs. A CFA3 AI Video Tools tervének
   Runway- és Kling-adaptere ezért nem szerepel. Minden modellnél rögzíteni kell a súly licencét és
   forrását.
5. **Minden funkció** parancs (`crates/engine`), menüből elérhető, és ha kell, saját
   beállításpanelt kap. A felület csak Qt6-ban készül.
6. **Meglévő tervvel átfedés esetén** az a terv viszi a munkát. Ez a terv csak hivatkozik rá, és
   jelzi, ha a CFA3-forrás új jelöltet ad hozzá (`coordination.md` 2.8).

Az előfeltétel-oszlop rövidítései:

- **Qt6:** a felülethez a Qt6 GUI kell (`qt-parity.md`). Tulajdonosi döntés: a felület csak Qt6-ban
  készül.
- **MI-r.:** az MI-réteg üzembe helyezése (`open-editors-parity.md` 0. pont, 2026-10-27-i
  döntés).
- **CS-A0:** a licenctisztázás lezárása.
- **ADR:** külön ADR kell, FFI- vagy bővítményhost-kivétel miatt (AGENTS.md §0.3, ADR-0001).

A „(ott)” becslés azt jelenti, hogy a munkát és a becslést a megnevezett meglévő terv tartalmazza.

## 2. Munkacsomagok területenként

A „Forrás” oszlop a [`cfa3-creative-studio-sources.md`](../cfa3-creative-studio-sources.md)
neveit használja. A becslés ügynökóra.

### 2.1 Alap

| Csomag | Tartalom | Forrás | Előfeltétel | Meglévő terv | Becslés |
|---|---|---|---|---|---|
| **CS-A0** | Licenctisztázás: CImg és G'MIC (CeCILL-2.1 vagy CeCILL-C részenként), OpenUSD (TOST), G'MIC community (nincs licencfájl), digiKam, Hugin, Enblend, Enfuse (eredeti tároló), YuE-súly (CC-BY-NC). Minden MI-modell súlylicencének rögzítése. | a 9 „tisztázandó” tétel, minden MI-tétel | – | – | 3–6 |

### 2.2 Videó, idővonal, kompozitálás

| Csomag | Tartalom | Forrás | Előfeltétel | Meglévő terv | Becslés |
|---|---|---|---|---|---|
| **CS-V1** | Programozott, determinisztikus idővonal: sorozat leírása kódból/JSON-ból, ismételhető render | hyperframes | – (motor), Qt6 (felület) | open-editors E1/E5 mintája | 30–60 |
| **CS-V2** | Átnézés (review): lejátszási listák, jegyzetek képkockára, conform | xSTUDIO | Qt6 | **ADR-0003 A7** | (ott) |
| **CS-V3** | Csomópontos (node) kompozitálás, Fusion-szerű (VISION.md 4. pont, DaVinci-sor) | Natron | Qt6, ADR | ADR-0003 A4 (OpenFX-host); a csomópontos szerkesztő új | 200–400 |
| **CS-V4** | Médiaképesség-router minták összevetése a `cfa4-route`-tal (jelenet-feladat folyamat) | Pixelle-Video | – | ADR-0004 | 6–12 |
| **CS-V5** | Ügynökvezérelt szerkesztés és szemantikus médiakeresés (mit lát a klipen) | Monet | MI-r. | G12 (media-intelligence search) | 60–120 |
| **CS-V6** | Képességfelmérés: hiányzik-e valami a CFA4-ből, amit ezek tudnak | Donkey, OpenCut | – | open-editors E13 (OpenCut) | 6–12 |

### 2.3 Szín és kép

| Csomag | Tartalom | Forrás | Előfeltétel | Meglévő terv | Becslés |
|---|---|---|---|---|---|
| **CS-S1** | OpenColorIO/ACES színkezelés | OpenColorIO | ADR | **ADR-0003 A1–A2** | (ott) |
| **CS-S2** | Képformátumok: PSD (rétegzett), EXR PIZ/DWA/B44, többrészes és mély EXR, HEIF | OpenImageIO, OpenEXR, PhotoshopAPI | ADR | **ADR-0003 A3**. A PhotoshopAPI (BSD-3) új jelölt a rétegzett PSD-hez. | (ott) + 5–10 |
| **CS-S3** | ICC-profilok: beágyazott profil olvasása, CMYK-képek helyes megjelenítése (G10, #139) | LittleCMS | – | file-format-parity (JPEG XL ICC) | 20–35 |
| **CS-S4** | Kamera-RAW folyamat: dekódolás, fehéregyensúly, demozaikolás, nem destruktív RAW-beállítások klipen | LibRaw, RawSpeed, Rawler, darktable, RawTherapee, vkdt; RapidRAW (csak ötlet) | – | ADR-0003 A3 (LibRaw), `crates/dng` | 60–120 |
| **CS-S5** | Objektívkorrekció adatbázisból: torzítás, vignettálás, kromatikus aberráció objektívtípus szerint | Lensfun | CS-A0 (az adatbázis licence) | – | 20–40 |
| **CS-S6** | G'MIC-szűrők külön folyamatban (CLI), szűrőböngésző | G'MIC, G'MIC-Qt, CImg, G'MIC community | CS-A0, ADR | G11 (bővítmények) | 40–80 |
| **CS-S7** | Zajszűrés (kép és render), CPU-n és bármely GPU-n | Open Image Denoise | ADR | G18 (effektmélység) | 20–40 |
| **CS-S8** | Nagy képek és nem destruktív képgráf (32 bites lebegőpontos feldolgozás, nagyon nagy állóképek) | GEGL, babl, libvips | – | – | 40–80 |
| **CS-S9** | Panoráma-illesztés és expozíció-fúzió (HDR) állóképekből és képsorokból | Hugin, Enblend, Enfuse | CS-A0 | – | 40–70 |
| **CS-S10** | Médiakatalógus: címkék, értékelés, arc- és tárgyfelismerés a projektben | digiKam / Showfoto | CS-A0; az arcfelismeréshez MI-r. | Media Intelligence (TASKSTORE-PG-001) | 40–80 |
| **CS-S11** | SVG-import állóképként és grafikaként (tiszta Rust); később GPU-s vektorrajz | resvg, tiny-skia, Vello | – | – | 12–25 (+ Vello 20–40) |
| **CS-S12** | Számítógépes látás: objektumkövetés, stabilizálás, maszk | OpenCV | MI-r. a maszkhoz | **OBJECT-MASK-PLAN-001**, M5 (Warp Stabilizer) | (ott) |

### 2.4 Festés és 2D animáció (Krita-képességek)

| Csomag | Tartalom | Forrás | Előfeltétel | Meglévő terv | Becslés |
|---|---|---|---|---|---|
| **CS-F1** | `.ora` import rétegekkel, `.kra` import (összesített kép; a rétegformátum nem specifikált) | OpenRaster, Krita | – | – | 15–30 |
| **CS-F2** | Ecsetmotor: nyomásérzékeny ecsetek, MyPaint-ecsetkészletek | libmypaint | – | – | 30–60 |
| **CS-F3** | Festés munkaterület: festés klipre és képkockára, rétegek, maszkok, keverési módok, retusálás | Krita | Qt6, CS-F2 | – | 120–220 |
| **CS-F4** | Kockánkénti 2D animáció: hagymahéj, expozíciós lap, képkocka-idővonal | Krita, OpenToonz | Qt6, CS-F3 | VISION.md 4. pont (Krita-sor) | 100–200 |
| **CS-F5** | Hagyományos animációs munkafolyamat: beszkennelt rajzok, tisztázás, festés (ink & paint), kamera | OpenToonz | CS-F4 | – | 120–250 |

### 2.5 Hang

| Csomag | Tartalom | Forrás | Előfeltétel | Meglévő terv | Becslés |
|---|---|---|---|---|---|
| **CS-H1** | DAWproject export/import (hangátadás DAW-ba) | DAWproject | – (motor), Qt6 (menü) | – | 25–45 |
| **CS-H2** | Többsávos hangszerkesztő munkaterület (Audacity/Ardour-szerű): hullámforma-szerkesztés, spektrális nézet, sávok, címkék | Ardour, Qtractor; Zrythm (csak ötlet) | Qt6 | VISION.md 4. pont (Audacity-sor), ADR-0003 A6 | 150–280 |
| **CS-H3** | Referencia alapú mastering (az Essential Sound része) | Matchering | – (DSP), Qt6 (panel) | – | 25–40 |
| **CS-H4** | Hangleválasztás: beszéd, zene, effekt | Audio Separator, BS-RoFormer | MI-r. | **Kdenlive K26** (Demucs). A RoFormer új mérendő jelölt. | (ott) + 5–10 |
| **CS-H5** | Beszédtisztítás és beszélőszétválasztás | ClearerVoice-Studio | MI-r. | G12 (Enhance Speech), M7 (ML speech enhancement) | 30–60 |
| **CS-H6** | Helyi beszédfelismerés felirathoz (vibe-minták: modellkezelés, gyors átirat) | vibe | – | **Feliratozó terv**, G19 | (ott) |

### 2.6 Szinkronizálás

| Csomag | Tartalom | Forrás | Előfeltétel | Meglévő terv | Becslés |
|---|---|---|---|---|---|
| **CS-SZ1** | Szinkron munkafolyamat: átirat → fordítás → helyi gépi hang vagy saját felvétel → időzítés a vágáshoz és a szájmozgáshoz, sávkezelés (eredeti / M&E / szinkron) | (CS-H4, CS-H5, CS-H6 kimenete) | MI-r., CS-H4 | VISION.md 4. pont „Szinkronizálás”; Feliratozó terv (narráció) | 150–300 |
| **CS-SZ2** | Hangkonverzió: a szinkronhang az eredeti szereplő hangszínével (csak hozzájárulással, rögzített eredettel) | Seed-VC | MI-r., CS-SZ1 | – | 40–80 |
| **CS-SZ3** | Hang-vízjel MI-vel generált hangra | AudioSeal | MI-r. | – | 15–30 |

### 2.7 Zene- és hanggenerálás (MI)

| Csomag | Tartalom | Forrás | Előfeltétel | Meglévő terv | Becslés |
|---|---|---|---|---|---|
| **CS-Z1** | Aláfestő zene generálása helyben (hossz, hangulat, tempó a vágáshoz igazítva) | ACE-Step, DiffRhythm, YuE (súlya NC: CS-A0) | MI-r., CS-A0 | G12 („Generated … music”) | 60–120 |

### 2.8 3D és renderelés

| Csomag | Tartalom | Forrás | Előfeltétel | Meglévő terv | Becslés |
|---|---|---|---|---|---|
| **CS-D1** | Blender oda-vissza: Blender-jelenet renderelt eredménye hivatkozott klipként, változásjelzéssel | Blender | – | open-editors (külső alkalmazás oda-vissza) | 30–60 |
| **CS-D2** | 3D jelenet és 3D címek a CFA4-ben (OpenUSD-alapú jelenetleírás) | OpenUSD | CS-A0 (TOST), Qt6 | VISION.md 8. pont (a 3D mélysége nyitott döntés) | 200–400 |
| **CS-D3** | Anyagok (MaterialX) a 3D jelenethez | MaterialX | CS-D2 | ADR-0003 A7 | 40–80 |
| **CS-D4** | Csontváz-animáció: lejátszás, keverés, tömörítés | ACL, ozz-animation | CS-D2 | – | 60–120 |
| **CS-D5** | Fotórealisztikus render: útkövetés (Cycles), CPU-s sugárkövetés (OSPRay) a routeren át | Cycles, OSPRay | CS-D2, ADR | – | 120–250 |

### 2.9 Helyi MI: kép és videó

| Csomag | Tartalom | Forrás | Előfeltétel | Meglévő terv | Becslés |
|---|---|---|---|---|---|
| **CS-M1** | ComfyUI-híd: munkafolyamat futtatása, eredmény klipként, eredetadattal | ComfyUI | MI-r. | open-editors (külső alkalmazás oda-vissza) | 30–60 |
| **CS-M2** | Helyi videógenerálás: szövegből, képből és videóból videó; állókép animálása; videó meghosszabbítása | Wan2.2, Helios | MI-r.; VRAM-mérés a referenciagépen (RTX A4000, 16 GB) | G12 (Generative Extend, B-roll) | 80–160 |
| **CS-M3** | Ötletből vagy forgatókönyvből videó: felvétellista, folytonosság, durva vágás | ViMax | MI-r., CS-M2 | G12 („Promo first cut”) | 60–120 |
| **CS-M4** | Egyszerű helyi képgenerálás (kezdőbarát felület, stílusok) | Fooocus | MI-r. | – | 30–60 |
| **CS-M5** | Rétegmegőrző MI-képszerkesztés a festés munkaterületen (kitöltés, kibővítés, vázlatból kép) | Krita AI Diffusion | MI-r., CS-F3 | – | 40–80 |

### 2.10 Metaadat és tartalom-eredet

| Csomag | Tartalom | Forrás | Előfeltétel | Meglévő terv | Becslés |
|---|---|---|---|---|---|
| **CS-E1** | C2PA: eredetmanifeszt exportkor (CFA4-verzió, MI-modellek, generált elemek), meglévő C2PA kiolvasása importkor | C2PA (c2pa-rs) | MI-r. (a modelladatokhoz) | OBJECT-MASK `provenance` | 30–50 |
| **CS-E2** | XMP bővítése: oldalfájl (sidecar) írás-olvasás, XMP az exportban (az Update Metadata (XMP) megvan) | xmp-toolkit-rs, XMP-specifikáció | – | M3.11 (XMP) | 15–30 |

### 2.11 Felületi minták (a CFA3 Creative Studio tervképéből)

| Csomag | Tartalom | Forrás | Előfeltétel | Meglévő terv | Becslés |
|---|---|---|---|---|---|
| **CS-U1** | Munkaterület-sáv: Projekt, Szerkesztés, Festés, Animáció, Felirat, Hang, Keverés, MI, Export | CFA3 tervkép | Qt6 | Qt6 GUI projekt; „Élő” munkaterület (LIVE-STUDIO-001) | 10–20 |
| **CS-U2** | Feliratszerkesztő panel az idővonal mellett (idővonal, szerkesztés, stílus, fordítás, OCR/MI lap, hullámforma) | CFA3 tervkép | Qt6 | **Feliratozó terv** (§4 hullámforma) | (ott) |
| **CS-U3** | Tulajdonságok/effektek panel Videó, Hang, Felirat, MI lappal | CFA3 tervkép | Qt6 | Qt6 GUI projekt (Effect Controls) | 10–20 |
| **CS-U4** | MI-asszisztens panel feladatgombokkal | CFA3 tervkép | Qt6, MI-r. | **open-editors E3** | (ott) |
| **CS-U5** | Festés- és hangszerkesztő panel a vágóablakban | CFA3 tervkép | Qt6, CS-F3, CS-H2 | – | 15–30 |

A CS-U1, CS-U3 és CS-U5 a Qt6 GUI projekttel érintkezik. Ezekről a tulajdonos és a gazda közösen
dönt (`coordination.md` 2.8), mielőtt bármelyik indul.

## 3. Technikai függőségek (nem sorrend)

Az alábbi csomagok csak a megadott csomag után készülhetnek el. A többi egymástól független,
bármilyen sorrendben indítható.

- CS-A0 → CS-S5, CS-S6, CS-S9, CS-S10, CS-Z1, CS-D2
- CS-F2 → CS-F3 → CS-F4 → CS-F5; CS-F3 → CS-M5, CS-U5
- CS-H2 → CS-U5
- CS-H4 → CS-SZ1 → CS-SZ2
- CS-D2 → CS-D3, CS-D4, CS-D5
- CS-M2 → CS-M3

## 4. Elfogadási feltételek (minden csomagra)

Egy csomag akkor kész, ha:

1. teljesíti a kapukat (`cargo xtask ci`, `cargo test --workspace`, `cargo deny check licenses`);
2. minden átvett kód szerepel a `licensing.md` 5. pontjában;
3. a funkció parancsként, menüből és (ha kell) saját beállításpanelről elérhető;
4. sérült bemenetre hibaüzenet jön, nem összeomlás (AGENTS.md §0), fájlformátumnál fuzz-teszttel;
5. van mért bizonyíték a forrással vagy referenciával szemben, nem állítás. A mérés módját a
   csomag indításakor, a részletes csomagtervben kell rögzíteni (például: SVG legfeljebb 1 LSB a
   resvg referenciájától; DAWproject `Tick`-pontos oda-vissza; mastering legfeljebb 0,5 LU és
   1 dB a Matchering kimenetétől);
6. MI-csomagnál a modell licence, forrása és SHA-256-ja rögzítve van, és a modell nem töltődik le a
   felhasználó tudta nélkül.

A nagy csomagokhoz (több mint ~100 óra: CS-V3, CS-F3–F5, CS-H2, CS-SZ1, CS-D2, CS-D5) indítás előtt
részletes csomagterv készül, külön jóváhagyással.

## 5. Kockázatok

- **Hatókör.** 51 csomag, a CFA4 hátralévő, kb. 3 évének jelentős része. Egyszerre csak annyi
  fusson, amennyit a tulajdonos kijelöl (a cél 5–10 párhuzamos terv).
- **Licenc.** A 9 tisztázandó tétel lezárásáig (CS-A0) az érintett csomagok nem indulhatnak.
- **Hardver.** A videó- és zenegeneráló modellek VRAM-igénye meghaladhatja a 16 GB-ot. Ezt
  modellenként mérni kell a referenciagépen, mielőtt a csomag elindul.
- **Idegen kód a folyamatban.** A C/C++ bővítmények (G'MIC, OIDN, Cycles, OSPRay) külön
  folyamatban futnak vagy ADR-rel kapnak FFI-kivételt, hogy összeomlásuk ne vigye magával a
  programot (ADR-0003 9. pont).
- **Átfedés más tervekkel.** Ha egy csomag egy meglévő terv területére ér, a gazdának jelezni
  kell, nem párhuzamosan megírni.

## 6. Tulajdonosi döntések

| # | Kérdés |
|---|---|
| **D1** | Elfogadod-e a terv tartalmát (51 csomag, 2. pont)? Az elfogadás nem indít el egyetlen csomagot sem. |
| **D2** | Melyik csomaggal vagy csomagokkal kezdjünk, és milyen sorrendben? |
| **D3** | A G'MIC (CS-S6) és az Open Image Denoise (CS-S7) felkerüljön-e az ADR-0003 6. pontjának bővítményjelöltjei közé (külön PR az ADR-0003-hoz)? |
| **D4** | A 3D mélysége (VISION.md 8. pont): a CS-D2–D5 teljes 3D jelenetszerkesztés legyen, vagy csak 3D címek és a Blender oda-vissza (CS-D1)? |
| **D5** | A hangkonverzió (CS-SZ2) idegen hangszínnel: milyen hozzájárulási szabály kell (csak saját hang, vagy rögzített engedéllyel más hangja is)? |

## Változástörténet

| Dátum | Verzió | Változás |
|---|---|---|
| 2026-11-11 | 1.1 tervezet | Tulajdonosi kérésre átdolgozva: mind a 70 forrás bekerült, 51 munkacsomagban. Az ütemezés kikerült, a sorrendet a tulajdonos dönti el igény szerint. Csak a technikai előfeltételek maradtak. |
| 2026-11-10 | 1.0 tervezet | Létrehozva (6 munkacsomag, a többi forrás más tervekre vagy későbbre utalva) |
