# CFA4-CFA3-CREATIVE-001: a CFA3 Creative Studio képességeinek átvétele

> **Állapot:** tervezet 1.2, tulajdonosi jóváhagyásra vár. Jóváhagyás előtt semmi nem indul. ·
> **Dátum:** 2026-11-11 · **Alapja:** [`cfa3-creative-studio-sources.md`](../cfa3-creative-studio-sources.md)
> (a CFA3-ból megtartott 70 forrás)

## 1. Mi ez a terv?

A CFA3 Creative Studio 70 nyílt forrású programjából és könyvtárából összegyűjti, milyen
**képességeket** kaphat a CFA4. Minden képesség egy **munkacsomag**: összesen 51, 11 területen.

- **Minden benne van**, a későbbi dolgok is.
- **Sorrend nincs benne.** Azt a tulajdonos dönti el, igény szerint.
- Egy csomag csak külön engedéllyel indul. A terv elfogadása nem indít el semmit.

## 2. Áttekintés

| # | Terület | Mit kap a CFA4 | Csomag | Becslés (óra) |
|---|---|---|---|---|
| 1 | Alap | lezárt licenckérdések | 1 | 3–6 |
| 2 | Videó | programozott idővonal, csomópontos kompozitálás, átnézés, MI-s médiakeresés | 6 | 300–600 |
| 3 | Szín és kép | színkezelés, kamera-RAW, objektívkorrekció, szűrők, zajszűrés, panoráma, SVG | 12 | 300–580 |
| 4 | Festés és 2D animáció | Krita-szerű festés, ecsetek, kockánkénti animáció | 5 | 390–760 |
| 5 | Hang | Audacity-szerű hangszerkesztő, mastering, DAW-átadás, beszédtisztítás | 6 | 240–440 |
| 6 | Szinkron | átirat → fordítás → gépi vagy saját hang → időzítés | 3 | 205–410 |
| 7 | Zene | aláfestő zene generálása helyben | 1 | 60–120 |
| 8 | 3D | 3D jelenet és címek, anyagok, csontváz-animáció, render | 5 | 450–910 |
| 9 | MI kép és videó | helyi kép- és videógenerálás, ComfyUI-kapcsolat | 5 | 240–480 |
| 10 | Metaadat | tartalom-eredet (C2PA), XMP-bővítés | 2 | 45–80 |
| 11 | Felület | munkaterület-sáv, panelek a CFA3 tervképe alapján | 5 | 35–70 |
| | **Összesen** | | **51** | **~2 300–4 500** |

Az órák ügynökórák. Ahol egy csomag már egy meglévő CFA4-tervben szerepel, ott a munka abban a
tervben készül. Ezt a csomag leírása jelzi („→ már tervezve”), és az órája nincs benne az
összegben.

## 3. Döntések, amelyek a tulajdonostól kellenek

1. **Elfogadod a tartalmat?** Ez semmit nem indít el.
2. **Melyik csomaggal kezdjünk?**
3. **3D:** teljes 3D jelenetszerkesztés (8. terület egésze), vagy csak 3D címek és a
   Blender-kapcsolat (3D-1)?
4. **Hangkonverzió** (Szinkron-2): csak a saját hang, vagy írásos engedéllyel más hangja is?
5. **G'MIC-szűrők és zajszűrő (Szín-6, Szín-7):** felkerüljenek a bővítményjelöltek közé?

## 4. A csomagok

A „Mi kell hozzá” oszlop szavai:

- **Qt6-felület:** a csomag felülete a kész Qt6-felületre épül.
- **MI-réteg:** a helyi MI-réteg üzembe helyezése után készülhet.
- **licenc tisztázása:** az Alap-1 csomag után.
- Ha üres („–”), bármikor indítható.

### 4.1 Alap

| Csomag | Mit ad a CFA4-nek | Mi kell hozzá | Óra |
|---|---|---|---|
| **Alap-1** Licenctisztázás | Kilenc forrás licence és az MI-modellek licence lezárva, hogy tudjuk, mi vehető át | – | 3–6 |

### 4.2 Videó

| Csomag | Mit ad a CFA4-nek | Mi kell hozzá | Óra |
|---|---|---|---|
| **Videó-1** Programozott idővonal | Sorozat összeállítása leírásból (szkript, JSON), mindig ugyanazzal az eredménnyel | – | 30–60 |
| **Videó-2** Átnézés | Lejátszási listák, megjegyzés képkockára, conform → már tervezve (ADR-0003) | Qt6-felület | – |
| **Videó-3** Csomópontos kompozitálás | DaVinci Fusion-szerű csomópontos szerkesztő effektekhez és VFX-hez | Qt6-felület | 200–400 |
| **Videó-4** Router-összevetés | Más programok feladatelosztásának összevetése a CFA4 routerével | – | 6–12 |
| **Videó-5** MI-s médiakeresés | Keresés abban, ami a klipeken látszik; ügynökvezérelt szerkesztés | MI-réteg | 60–120 |
| **Videó-6** Képességfelmérés | Mit tud még két nyílt vágóprogram, ami a CFA4-ből hiányzik | – | 6–12 |

### 4.3 Szín és kép

| Csomag | Mit ad a CFA4-nek | Mi kell hozzá | Óra |
|---|---|---|---|
| **Szín-1** OpenColorIO | ACES-színkezelés → már tervezve (ADR-0003) | – | – |
| **Szín-2** Képformátumok | Rétegzett PSD, a hiányzó EXR-változatok, HEIF → már tervezve (ADR-0003); új jelölt a PSD-hez | – | 5–10 |
| **Szín-3** ICC-profilok | Beágyazott színprofilok és CMYK-képek helyes megjelenítése | – | 20–35 |
| **Szín-4** Kamera-RAW | RAW-fájlok fejlesztése a klipen: fehéregyensúly, expozíció, nem destruktívan | – | 60–120 |
| **Szín-5** Objektívkorrekció | Torzítás és vignettálás javítása objektívtípus szerint, adatbázisból | licenc tisztázása | 20–40 |
| **Szín-6** G'MIC-szűrők | Több száz képszűrő külön folyamatban, szűrőböngészővel | licenc tisztázása | 40–80 |
| **Szín-7** Zajszűrés | Kép- és renderzaj csökkentése, bármely GPU-n vagy CPU-n | – | 20–40 |
| **Szín-8** Nagy képek | Nagyon nagy állóképek és 32 bites, nem destruktív képfeldolgozás | – | 40–80 |
| **Szín-9** Panoráma és HDR | Képek összeillesztése panorámává, expozíciók egyesítése | licenc tisztázása | 40–70 |
| **Szín-10** Médiakatalógus | Címkék, értékelés, arc- és tárgyfelismerés a projektben | licenc tisztázása; arcfelismeréshez MI-réteg | 40–80 |
| **Szín-11** SVG | SVG-fájlok importja élesen méretezhető grafikaként; később GPU-s vektorrajz | – | 12–25 |
| **Szín-12** Követés, stabilizálás | Tárgykövetés, stabilizálás, maszk → már tervezve (OBJECT-MASK terv) | MI-réteg | – |

### 4.4 Festés és 2D animáció

| Csomag | Mit ad a CFA4-nek | Mi kell hozzá | Óra |
|---|---|---|---|
| **Festés-1** Rajzfájlok importja | Krita (`.kra`) és OpenRaster (`.ora`) fájlok megnyitása | – | 15–30 |
| **Festés-2** Ecsetmotor | Nyomásérzékeny ecsetek, MyPaint-ecsetkészletek | – | 30–60 |
| **Festés-3** Festés munkaterület | Festés klipre és képkockára, rétegek, maszkok, retusálás | Qt6-felület, Festés-2 | 120–220 |
| **Festés-4** 2D animáció | Kockánkénti animáció hagymahéjjal és képkocka-idővonallal | Festés-3 | 100–200 |
| **Festés-5** Hagyományos animáció | Beszkennelt rajzok tisztázása, kifestése, kameramozgás | Festés-4 | 120–250 |

### 4.5 Hang

| Csomag | Mit ad a CFA4-nek | Mi kell hozzá | Óra |
|---|---|---|---|
| **Hang-1** DAW-átadás | A hangsávok átadása zenei programba (DAWproject-formátum) | – | 25–45 |
| **Hang-2** Hangszerkesztő | Audacity-szerű munkaterület: hullámforma, spektrum, sávok, címkék | Qt6-felület | 150–280 |
| **Hang-3** Mastering | A hang automatikus igazítása egy mintadalhoz | – | 25–40 |
| **Hang-4** Hangleválasztás | Beszéd, zene és effekt szétválasztása → már tervezve (Kdenlive-terv); új modelljelölt | MI-réteg | 5–10 |
| **Hang-5** Beszédtisztítás | Zaj eltávolítása a beszédből, beszélők szétválasztása | MI-réteg | 30–60 |
| **Hang-6** Beszédfelismerés | Helyi átirat a feliratozáshoz → már tervezve (Feliratozó terv) | – | – |

### 4.6 Szinkron

| Csomag | Mit ad a CFA4-nek | Mi kell hozzá | Óra |
|---|---|---|---|
| **Szinkron-1** Szinkron munkafolyamat | Átirat → fordítás → gépi vagy saját hang → időzítés a vágáshoz és a szájmozgáshoz | MI-réteg, Hang-4 | 150–300 |
| **Szinkron-2** Hangkonverzió | A szinkronhang az eredeti szereplő hangszínével (csak engedéllyel) | Szinkron-1 | 40–80 |
| **Szinkron-3** Hang-vízjel | Az MI-vel készült hang megjelölése | MI-réteg | 15–30 |

### 4.7 Zene

| Csomag | Mit ad a CFA4-nek | Mi kell hozzá | Óra |
|---|---|---|---|
| **Zene-1** Aláfestő zene | Zene generálása helyben, a vágás hosszához és tempójához igazítva | MI-réteg, licenc tisztázása | 60–120 |

### 4.8 3D

| Csomag | Mit ad a CFA4-nek | Mi kell hozzá | Óra |
|---|---|---|---|
| **3D-1** Blender-kapcsolat | Blenderben készült jelenet klipként; jelzés, ha a jelenet megváltozott | – | 30–60 |
| **3D-2** 3D jelenet és címek | 3D-s jelenet és 3D címek a CFA4-ben | Qt6-felület, licenc tisztázása | 200–400 |
| **3D-3** Anyagok | Felületek, anyagok a 3D jelenethez | 3D-2 | 40–80 |
| **3D-4** Csontváz-animáció | Szereplők mozgatása csontvázzal | 3D-2 | 60–120 |
| **3D-5** Fotórealisztikus render | Valósághű fény és árnyék renderelése | 3D-2 | 120–250 |

### 4.9 MI kép és videó

| Csomag | Mit ad a CFA4-nek | Mi kell hozzá | Óra |
|---|---|---|---|
| **MI-1** ComfyUI-kapcsolat | ComfyUI-munkafolyamat futtatása, az eredmény klipként | MI-réteg | 30–60 |
| **MI-2** Videógenerálás | Videó szövegből vagy képből, állókép animálása, videó meghosszabbítása | MI-réteg; a gépigényt előbb mérni kell | 80–160 |
| **MI-3** Ötletből videó | Ötletből vagy forgatókönyvből felvétellista és durva vágás | MI-2 | 60–120 |
| **MI-4** Képgenerálás | Egyszerű helyi képgenerálás, kezdőknek is | MI-réteg | 30–60 |
| **MI-5** MI a festésben | Kitöltés, kibővítés, vázlatból kép a festés munkaterületen | MI-réteg, Festés-3 | 40–80 |

### 4.10 Metaadat

| Csomag | Mit ad a CFA4-nek | Mi kell hozzá | Óra |
|---|---|---|---|
| **Meta-1** Tartalom-eredet | Az exportált fájl jelzi, mi készült MI-vel és milyen modellel (C2PA) | MI-réteg | 30–50 |
| **Meta-2** XMP-bővítés | Metaadat külön fájlban és az exportban | – | 15–30 |

### 4.11 Felület

| Csomag | Mit ad a CFA4-nek | Mi kell hozzá | Óra |
|---|---|---|---|
| **Felület-1** Munkaterület-sáv | Projekt, Szerkesztés, Festés, Animáció, Felirat, Hang, Keverés, MI, Export | Qt6-felület | 10–20 |
| **Felület-2** Feliratpanel | Feliratszerkesztő az idővonal mellett → már tervezve (Feliratozó terv) | Qt6-felület | – |
| **Felület-3** Tulajdonságok panel | Videó, Hang, Felirat, MI lapok egy panelen | Qt6-felület | 10–20 |
| **Felület-4** MI-asszisztens panel | MI-feladatok gombokkal → már tervezve (open-editors E3) | MI-réteg | – |
| **Felület-5** Festés és hang a vágóablakban | Festő- és hangszerkesztő panel a külön program helyett | Festés-3, Hang-2 | 15–30 |

A Felület-1, -3 és -5 a Qt6-felület munkájához kapcsolódik. Ezekről indítás előtt a tulajdonos
és a gazda közösen dönt.

## 5. Ami minden csomagra érvényes

- **Licenc:** GPL-3-kompatibilis kód átvehető, forrásmegjelöléssel. AGPL-ből csak ötlet.
- Minden funkció **menüből elérhető**, és ha kell, saját beállításpanelt kap.
- **Nincs NVIDIA-kötés, nincs felhős MI.** A modellek helyben futnak, és a felhasználó tudta nélkül
  nem töltődnek le.
- **Kész csak mért bizonyítékkal:** a csomag az összevetéssel együtt kész, nem állítással.
- **Hibás fájl** nem okozhat összeomlást.
- **A 100 óránál nagyobb csomagokhoz** indítás előtt részletes terv készül (Videó-3, Festés-3–5,
  Hang-2, Szinkron-1, 3D-2, 3D-5).

## A. függelék: melyik forrás melyik csomaghoz tartozik

| Csomag | Forrás(ok) |
|---|---|
| Alap-1 | CImg, G'MIC, G'MIC community, OpenUSD, digiKam, Hugin, Enblend, Enfuse, YuE |
| Videó-1 | hyperframes |
| Videó-2 | xSTUDIO |
| Videó-3 | Natron |
| Videó-4 | Pixelle-Video |
| Videó-5 | Monet |
| Videó-6 | Donkey, OpenCut |
| Szín-1 | OpenColorIO |
| Szín-2 | OpenImageIO, OpenEXR, PhotoshopAPI |
| Szín-3 | LittleCMS |
| Szín-4 | LibRaw, RawSpeed, Rawler, darktable, RawTherapee, vkdt, RapidRAW (csak ötlet, AGPL) |
| Szín-5 | Lensfun |
| Szín-6 | G'MIC, G'MIC-Qt, CImg, G'MIC community |
| Szín-7 | Open Image Denoise |
| Szín-8 | GEGL, babl, libvips |
| Szín-9 | Hugin, Enblend, Enfuse |
| Szín-10 | digiKam / Showfoto |
| Szín-11 | resvg, tiny-skia, Vello |
| Szín-12 | OpenCV |
| Festés-1 | OpenRaster, Krita |
| Festés-2 | libmypaint |
| Festés-3 | Krita |
| Festés-4 | Krita, OpenToonz |
| Festés-5 | OpenToonz |
| Hang-1 | DAWproject |
| Hang-2 | Ardour, Qtractor, Zrythm (csak ötlet, AGPL) |
| Hang-3 | Matchering |
| Hang-4 | Audio Separator, BS-RoFormer |
| Hang-5 | ClearerVoice-Studio |
| Hang-6 | vibe |
| Szinkron-2 | Seed-VC |
| Szinkron-3 | AudioSeal |
| Zene-1 | ACE-Step, DiffRhythm, YuE |
| 3D-1 | Blender |
| 3D-2 | OpenUSD |
| 3D-3 | MaterialX |
| 3D-4 | ACL, ozz-animation |
| 3D-5 | Cycles, OSPRay |
| MI-1 | ComfyUI |
| MI-2 | Wan2.2, Helios |
| MI-3 | ViMax |
| MI-4 | Fooocus |
| MI-5 | Krita AI Diffusion |
| Meta-1 | C2PA (c2pa-rs) |
| Meta-2 | xmp-toolkit-rs, XMP-specifikáció |
| Felület-1–5 | a CFA3 Creative Studio tervképe |

A licenceket és a forrástárolókat a [`cfa3-creative-studio-sources.md`](../cfa3-creative-studio-sources.md)
tartalmazza.

## B. függelék: kapcsolódó CFA4-dokumentumok

[`VISION.md`](../../VISION.md) ·
[ADR-0003](../adr/ADR-0003-aswf-and-conversion-sources.md) (ASWF, bővítmények) ·
[ADR-0004](../adr/ADR-0004-compute-routing.md) (router) ·
[Kdenlive-terv](CFA4-KDENLIVE-PLAN-001-FINAL-1.0.md) ·
[Feliratozó terv](SUBTITLE-STUDIO-PLAN-001-FINAL-1.2.md) ·
[OBJECT-MASK terv](OBJECT-MASK-PLAN-001-2026-11-03.md) ·
[`open-editors-parity.md`](../open-editors-parity.md) ·
[`licensing.md`](../licensing.md) ·
[`coordination.md`](../coordination.md)

## Változástörténet

| Dátum | Verzió | Változás |
|---|---|---|
| 2026-11-11 | 1.2 tervezet | A tulajdonos szerint átláthatatlan volt: összesítő és döntések előre, egyszerű táblák rövidítések nélkül, a források függelékben |
| 2026-11-11 | 1.1 tervezet | Mind a 70 forrás bekerült (51 csomag), ütemezés nélkül |
| 2026-11-10 | 1.0 tervezet | Első változat (6 csomag) |
