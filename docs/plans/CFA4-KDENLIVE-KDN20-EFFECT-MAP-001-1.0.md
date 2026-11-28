# KDN-20 — Kdenlive azonosító → CFA4 effekt tábla (W1)

*Készült: 2026-11-25. Állapot: **jóváhagyva (István, 2026-11-25)**. Teljes tábla: `crates/interchange/data/kdenlive-effect-map.csv`.*

## 1. Mi ez, és mire jó

A `.kdenlive` projektfájl effektjei és átmenetei Kdenlive-azonosítóval (`id`, MLT-szolgáltatás `tag`) szerepelnek. A KDN-1 importernek tudnia kell, melyik CFA4-effektnek felel meg mindegyik. Ez a tábla ezt adja meg **minden** azonosítóra, amit a Kdenlive-fork `data/effects` és `data/transitions` könyvtára tartalmaz. A VFX-terv 1.1 szerint a KDN-20 kimenete a `G-xx` katalógus `aliases` mezőjébe illeszkedik, ezért a CSV `kdenlive_id` oszlopa közvetlenül alias-érték, a `cfa4_id` pedig a katalógus effekt-azonosítója.

**Forrás:** a `ubuntuokos/kdenlive` fork, 2026-11-22-i fagyasztott állapot (`abfef9bc035ad6cd643f491d3d755083d347d69f`), csak olvasott donor. **CFA4 oldal:** a `cfa4_project::effect::effect_defs()` tényleges listája (336 effekt, a repó `main`-jéről), nem emlékezetből: a `cfa4_id` minden értéke ellenőrzötten létezik.

## 2. Összkép

A fork 545 sort ad (488 effekt-sor + 57 átmenet-sor); az azonos fájlon belüli ismétlődő bejegyzések összevonva a CSV 531 sora, ezekből **513 egyedi** (típus, azonosító) pár.

| Csoport | Egyedi azonosító | SAME | CLOSE | BLEND | NA | NONE |
|---|--:|--:|--:|--:|--:|--:|
| root | 83 | 24 | 39 | 0 | 12 | 8 |
| movit | 14 | 7 | 7 | 0 | 0 | 0 |
| avfilter | 145 | 26 | 66 | 0 | 22 | 31 |
| frei0r | 125 | 22 | 65 | 0 | 12 | 26 |
| ladspa | 76 | 3 | 46 | 0 | 0 | 27 |
| sox | 10 | 5 | 3 | 0 | 1 | 1 |
| templates | 3 | 0 | 3 | 0 | 0 | 0 |
| transition | 12 | 6 | 3 | 0 | 2 | 1 |
| transition-frei0r | 45 | 0 | 16 | 27 | 0 | 2 |
| **Összesen** | **513** | 93 | 248 | 27 | 49 | 96 |

**Jelölések.** `SAME`: ugyanaz a funkció, a paraméterek leképezhetők. `CLOSE`: hasonló funkció, de a paraméterezés vagy az algoritmus eltér (az importnak figyelmeztetnie kell). `BLEND`: keverési mód, nem effekt (a frei0r-átmenetek nagy része ez; a CFA4 27 keverési módjára képződik). `NA`: nem effekt-import tárgya (analizátor/scope, maszk-lánc, deinterlace/klip-értelmezés, MI-szolgáltatás). `NONE`: nincs megfelelő CFA4-effekt.

## 3. Amit a tábla nem állít

- **Egyik sor sem „igazolt”.** Minden `SAME`/`CLOSE` sor `parity_level = NAME_MATCH_UNVERIFIED` (a VFX-terv szótára szerint): a megfeleltetés funkció és paraméternév alapján készült, renderrel nem mértem. A `PARAM_PARITY`/`VISUAL_PARITY_TESTED` szintet a KDN-25 `melt`-összevetése adhatja soronként.
- **Ismert eltérések:** a `brightness` MLT-szorzó, a CFA4 `brightness_contrast` eltolás (nincs pontos átváltás); a Kdenlive fade-ek (`fade_from_black`, `fadein`…) kulcskockás `brightness`/`volume` szűrők, a CFA4-ben átmenet vagy átlátszóság/hangerő-kulcs; a `speed` klip-tulajdonság, nem effekt; a `volume`/`gain` leképezése az importerben már kész (`volume.level`, dB, ±15 dB-re vágva).
- A **sablonok** (`Secondary color correction`, `Shut-off`) több effektből álló láncok; a tábla az egész lánc legjobb egy-effektes közelítését adja, a lánc elemei a saját soraikban szerepelnek.
- A **frei0r** effektek közül azok, amelyeknek van natív CFA4-párjuk, arra képződnek; a többi `NONE` „frei0r-host hiányzik (KDN-5)” megjegyzéssel, ahogy a terv megkövetelte. Az MI-alapú szűrők (`facedetect`, `facebl0r`, `sr`, `derain`) `NA`: csak az MI-réteg után.
- Új effekt nem a KDN-20 feladata: a `NONE` sorok a VFX-katalógusba jelöltként kerülnek (lent a lista).

## 4. `NONE` sorok — VFX-katalógus jelöltek

**avfilter:** `avfilter.allpass` (Allpass); `avfilter.apulsator` (Pulsator); `avfilter.backgroundkey` (Background Key); `avfilter.colormatrix` (Color Matrix); `avfilter.dcshift` (DC shift); `avfilter.deband` (Deband); `avfilter.deblock` (DeBlock); `avfilter.dedot` (DeDot); `avfilter.deflate` (Deflate); `avfilter.despill` (Despill); `avfilter.dilation` (Dilation); `avfilter.drawbox` (Draw Box); `avfilter.elbg` (Posterize (ELBG)); `avfilter.epx` (EPX Scaler); `avfilter.erosion` (Erosion); `avfilter.fillborders` (Fill Borders); `avfilter.fspp` (fspp); `avfilter.gradfun` (Gradfun); `avfilter.hqx` (Hq*x Interpolator); `avfilter.inflate` (Inflate); `avfilter.phase` (Phase); `avfilter.photosensitivity` (Photosensitivity Filter); `avfilter.pseudocolor` (Pseudocolor); `avfilter.random` (Random); `avfilter.shufflepixels` (Shuffle Pixels); `avfilter.shuffleplanes` (Shuffle planes); `avfilter.subtitles` (Subtitles); `avfilter.swapuv` (Swap UV); `avfilter.tmix` (Temporal frames Mixer); `avfilter.vibrato` (Vibrato); `avfilter.xbr` (xBR Interpolator); 
**frei0r:** `frei0r.autothresh0ld` (Auto Threshold); `frei0r.bgsubtract0r` (Background Subtractor); `frei0r.bigsh0t_eq_cap` (VR360 Cap Top and Bottom); `frei0r.bigsh0t_eq_mask` (VR360 Equirectangular Mask); `frei0r.cartoon` (Cartoon); `frei0r.cluster` (K-Means Clustering (ABANDONED)); `frei0r.colordistance` (Color Distance); `frei0r.colortap` (Color Effect); `frei0r.d90stairsteppingfix` (Nikon D90 Stairstepping fix); `frei0r.elastic_scale` (Elastic Scale); `frei0r.harris0r` (Harris Shutter Effect); `frei0r.heatmap0r` (Heatmap0r); `frei0r.keyspillm0pup` (Key Spill Mop Up); `frei0r.lightgraffiti` (Light Graffiti); `frei0r.motiondiff0r` (Motion Extraction); `frei0r.ndvi` (NDVI); `frei0r.nervous` (Nervous); `frei0r.nosync0r` (nosync0r); `frei0r.primaries` (Primaries); `frei0r.shadert0y` (Shadert0y); `frei0r.spillsupress` (Spill Suppress); `frei0r.tehRoxx0r` (TehRoxx0r); `frei0r.threshold0r` (Threshold); `frei0r.timeout` (Timeout Indicator); `frei0r.twolay0r` (Binarize dynamically); `frei0r.vertigo` (Vertigo); 
**ladspa:** `ladspa.1185` (Diode Processor); `ladspa.1190` (Comb Filter); `ladspa.1192` (Fractionally Addressed Delay Line); `ladspa.1218` (4x4 Pole Allpass); `ladspa.1411` (Comb Splitter); `ladspa.1415` (FM Oscillator); `ladspa.1431` (Bode Frequency Shifter); `ladspa.1432` (Bode Frequency Shifter (CV)); `ladspa.1886` (Exponential Signal Decay); `ladspa.1887` (Comb Delay Line (linear interpolation)); `ladspa.1888` (Comb Delay Line (cubic spline interpolation)); `ladspa.1889` (Comb Delay Line (non-interpolating)); `ladspa.1895` (Allpass Delay Line (Non-interpolating)); `ladspa.1896` (Allpass Delay Line (Linear Interpolation)); `ladspa.1897` (Allpass Delay Line (Cubic Spline Interpolation)); `ladspa.1902` (GLAME Butterworth X-Over); `ladspa.1905` (VyNil (Vinyl Effect)); `ladspa.2144` (TAP Tremolo); `ladspa.2148` (TAP Vibrato); `ladspa.2149` (TAP Rotary Speaker); `ladspa.2154` (TAP Reflector); `ladspa.2155` (TAP Pink/Fractal Noise); `ladspa.2156` (TAP Fractal Doubler); `ladspa.2157` (TAP Sigmoid Booster); `ladspa.2158` (TAP TubeWarmth); `rate_scale` (Rate Scaler); `vinyl` (Vinyl); 
**root:** `BurningTV` (Burning TV); `dance` (Dance); `gpsgraphic` (GPS Graphic); `gradientmap` (Gradient Map); `lightshow` (Light Show); `opencv.tracker` (Motion Tracker); `pillar_echo` (Pillar Echo); `threshold` (Binarize); 
**sox:** `sox_stretch` (Sox Stretch); 
**transition:** `autoduck` (Auto Duck); 
**transition-frei0r:** `frei0r.euclid_eraser` (Euclid Eraser); `frei0r.uvmap` (UV Map); 

## 5. Következő lépések (jóváhagyásra)

1. **Jóváhagyás után** a CSV bekerül a repóba az SC-5.3 szűrőleképezési tábla mellé (`crates/interchange` adatfájl + egy teszt, amely minden `cfa4_id`-t a futó effekt-regiszterhez ellenőriz, így a tábla nem avulhat el csendben).
2. A KDN-1 importer a `SAME`/`CLOSE` sorokat használja effektként (a `CLOSE` figyelmeztetéssel a kompatibilitási jelentésben), a `BLEND` sorokat keverési módként, a `NONE` sorokat „nem importálható” figyelmeztetésként. Paraméter-leképezés effektenként külön szelet (volume kész; következők: `brightness`, `lift_gamma_gain`, `qtblend`/`affine` kulcskockák).
3. A `melt`-összevetés (KDN-25) a `SAME` sorokra `PARAM_PARITY`-t adhat; ez a CI `melt` verziójának rögzítése után indulhat.
4. A VFX-katalógus gazdájának átadás: a `kdenlive_id` → `aliases`, a `NONE` lista → jelöltek.

## 6. Döntés nem kell

A tábla a jóváhagyott tervet (KDN-20, VFX 1.1 alias-kompatibilitás) hajtja végre; a repóba kerüléséhez a tulajdonos jóváhagyása kell (1. lépés), ezért nem nyitottam PR-t.
