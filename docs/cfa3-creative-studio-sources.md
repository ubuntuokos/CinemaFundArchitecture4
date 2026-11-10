# CFA3 Creative Studio: a CFA4-be átvehető források (szűrt lista)

> **Állapot:** referencialista, a tulajdonos kérésére (2026-11-10). **Nem jóváhagyott terv**, nem nyit
> feladatot, és nem engedélyez sem kódátvételt, sem új függőséget. Minden tételre a [`AGENTS.md`](../AGENTS.md)
> CFA4-szakasza, a [`licensing.md`](licensing.md) és a kodekekre vonatkozó clean-room szabály érvényes.
> **Készült:** 2026-11-10 · **Forrás:** `ubuntuokos/CFA3-CinenemFoundArchitecture3`, `main` @ `a021ef3`
> (2026-10-16), `canonical/registries/CFA3-DONOR-TRANSFER-SNAPSHOT-001.json` · **Kapcsolódik:**
> [`open-editors-parity.md`](open-editors-parity.md), [ADR-0003](adr/ADR-0003-aswf-and-conversion-sources.md),
> [`VISION.md`](../VISION.md), [`gaps.md`](gaps.md)

A CFA3 tároló hamarosan lekerül a GitHubról. Ez a dokumentum a benne lévő **Creative Studio**
forráslistájának azt a részét őrzi meg, amely a CFA4 céljához tartozik és licenc szerint
használható.

## 0. Röviden

- A CFA3 Creative Studio **nem tartalmaz kódot**. Egy felülettervből (README-kép) és egy
  forrásnyilvántartásból áll: ebben **221** tétel jelöli célként a „Creative Studio”-t.
- Ebből **70** tétel maradt meg. **59** GPL-3.0-kompatibilis licencű (a kód is átvehető),
  **2** AGPL (csak az ötlet vehető át), **9**-nél a licenc tisztázandó. **151** tétel kimaradt,
  csoportonként a 4. pontban.
- A licenceket nem a CFA3-nyilvántartásból vettem (ott 131 tétel licence „UNKNOWN”).
  2026-11-10-én a forrástárolók aktuális `HEAD`-jén olvastam ki őket, a commit a táblázatban áll.
- A **modellsúlyok licence külön kérdés.** A kódlicenc nem terjed ki rájuk. A YuE súlyai például
  CC-BY-NC, így nem szállíthatók. Minden MI-tételnél a súly licencét külön kell ellenőrizni.
- Az **MI-tételek** a már elfogadott alapszabály szerint csak az MI-réteg üzembe helyezése után
  jönnek ([`open-editors-parity.md`](open-editors-parity.md) 0. pont).

## 1. Szűrési szempontok

Egy tétel akkor maradt meg, ha mindhárom feltétel teljesül:

1. **A CFA4 céljához tartozik:** videószerkesztés, -konvertálás, -publikálás, feliratozás,
   szinkron, hangszerkesztés, 2D/3D animáció, helyi MI, vagy ezek csereformátumai
   ([`VISION.md`](../VISION.md)).
2. **Nyílt forrású, megtalálható forrással.** Zárt program, zárt bővítmény vagy SDK nem maradt
   meg, sem olyan, amelynek a CFA3-ban csak a neve szerepel.
3. **Nem ütközik a CFA4 szabályaival:** nincs Adobe-asset vagy -betűkészlet (AGENTS.md §1),
   nincs NVIDIA-kötés, nincs Windows-kompatibilitási réteg, nincs nem szabad (kutatási vagy nem
   kereskedelmi) licenc.

A GPL-3.0-kompatibilitás a [`licensing.md`](licensing.md) táblája szerint:

- **kód is:** MIT, BSD, ISC, Apache-2.0, LGPL, GPL-2.0-or-later, GPL-3.0.
- **csak ötlet:** AGPL. A kód nem vehető át, a képesség saját kóddal készül.
- **tisztázandó:** CeCILL-változatok, OpenUSD TOST-licence, licencfájl nélküli tároló, nem
  ellenőrzött vagy elavult tükör, nem szabad modellsúly.

## 2. Ütemjelölés

| Jel | Jelentés |
|---|---|
| **1** | Nem MI, főleg csereformátum vagy kis alapkönyvtár. Önálló, tulajdonos által jóváhagyott feladatként a Qt6 GUI mellett is mehet. |
| **2** | A Qt6 GUI elkészülte után (a Qt6-átállás elsőbbsége szerint). |
| **MI** | Csak az MI-réteg üzembe helyezése után. |

A „Már szerepel” oszlop azt mutatja, hol hivatkozik már rá a CFA4. Ott ez a lista nem ír felül
semmit.

## 3. Megtartott források (70)

### 1. Videó, idővonal, kompozitálás, felülvizsgálat

| Forrás | Licenc (HEAD) | Átvehető | Mit ad a CFA4-nek | Ütem | Már szerepel |
|---|---|---|---|---|---|
| [OpenCut](https://github.com/OpenCut-app/OpenCut) | MIT · `e668010` | kód is | szerkesztési előzmények, gyorsbillentyű-UX, idővonal-minták | 1 | open-editors-parity.md |
| [hyperframes](https://github.com/heygen-com/hyperframes) | Apache-2.0 · `124591a` | kód is | determinisztikus, programozott (szkriptelt) idővonal-renderelés | 2 | — |
| [xSTUDIO](https://github.com/AcademySoftwareFoundation/xstudio) | Apache-2.0 · `bb05faa` | kód is | felülvizsgálat (review), lejátszási listák, conform | 2 | ADR-0003 |
| [Natron](https://github.com/NatronGitHub/Natron) | GPL-2.0-or-later · `3763d80` | kód is | csomópontos (node) kompozitálás, OpenFX-gazda minták | 2 | ADR-0003, gaps.md |
| [Pixelle-Video](https://github.com/ATH-MaaS/Pixelle-Video) | Apache-2.0 · `848b054` | kód is | médiaképesség-router minta, jelenet-feladat folyamat (a cfa4-route-hoz összevetésnek) | 2 | — |
| [Donkey](https://github.com/DonkeyCut/Donkey) | Apache-2.0 · `1d31b5a` | kód is | vágóprogram (képességek felmérésre) | 2 | — |

### 2. Szín, kép, RAW, festés (Krita-képességek)

| Forrás | Licenc (HEAD) | Átvehető | Mit ad a CFA4-nek | Ütem | Már szerepel |
|---|---|---|---|---|---|
| [OpenColorIO](https://github.com/AcademySoftwareFoundation/OpenColorIO) | BSD-3-Clause · `22dd9c4` | kód is | ACES-megfelelés, színkezelési referencia, OCIO-transzformációk | 1 | ADR-0003, VISION.md |
| [OpenEXR](https://github.com/AcademySoftwareFoundation/openexr) | BSD-3-Clause · `7296358` | kód is | HDR-csere, többrétegű EXR, renderpass-megőrzés | 1 | — |
| [OpenImageIO](https://github.com/AcademySoftwareFoundation/OpenImageIO) | Apache-2.0 (régebbi részek BSD-3) · `cb9143b` | kód is | csempézett képfeldolgozás, mipmap, textúra-bemenetek | 2 | gaps.md (EXR-orákulum) |
| [LittleCMS](https://github.com/mm2/Little-CMS) | MIT · `780da28` | kód is | ICC-színkezelés | 1 | — |
| [babl](https://github.com/GNOME/babl) | LGPL-3.0 · `7419e33` | kód is | pixelformátum-konverzió (GIMP/GEGL alap) | 2 | — |
| [GEGL](https://github.com/GNOME/gegl) | LGPL-3.0 / GPL-3.0 · `b500a75` | kód is | nem destruktív képfeldolgozó gráf | 2 | — |
| [Lensfun](https://github.com/lensfun/lensfun) | LGPL-3.0 (könyvtár), GPL-3.0 · `52cf1a7` | kód is | objektívtorzítás- és vignettálás-korrekció adatbázissal | 2 | — |
| [LibRaw](https://github.com/LibRaw/LibRaw) | LGPL-2.1 vagy CDDL-1.0 · `a9a14b1` | kód is | RAW-dekódolás; a crates/dng külső tesztorákulumként futtatja | 2 | crates/dng (orákulum) |
| [RawSpeed](https://github.com/darktable-org/rawspeed) | LGPL-2.1 · `2baa3a2` | kód is | gyors RAW-dekódolás | 2 | crates/dng README |
| [Rawler (dnglab)](https://github.com/dnglab/dnglab) | LGPL-2.1 · `e98ce45` | kód is | Rust RAW-dekóder (dnglab része) | 2 | — |
| [darktable](https://github.com/darktable-org/darktable) | GPL-3.0 · `c96cf1d` | kód is | kamera-színfolyamat, nem destruktív RAW-munkafolyamat | 2 | crates/dng README |
| [RawTherapee](https://github.com/RawTherapee/RawTherapee) | GPL-3.0 · `64c2d90` | kód is | RAW-fejlesztési algoritmusok | 2 | crates/dng README |
| [vkdt](https://github.com/hanatos/vkdt) | BSD-2-Clause, egyes fájlok GPL-3.0 · `ea3deef` | kód is | Vulkan-alapú, GPU-gyártófüggetlen kép-/videófeldolgozó gráf | 2 | — |
| [RapidRAW](https://github.com/CyberTimon/RapidRAW) | AGPL-3.0 · `ff859fd` | csak ötlet | Rust + GPU RAW-szerkesztő | 2 | — |
| [digiKam / Showfoto](https://github.com/KDE/digikam) | GPL-2.0 / LGPL-2.1 (a GitHub-tükör 2019-es; a KDE-tároló az eredeti) · `33d0457` | tisztázandó | médiakatalógus, címkézés, arcfelismerés (KDE) | 2 | — |
| Hugin | GPL-2.0-or-later (projektállítás; itt nem ellenőrizve) | tisztázandó | panoráma-illesztés | 2 | — |
| Enblend | GPL-2.0-or-later (projektállítás; itt nem ellenőrizve) | tisztázandó | képösszemosás (panoráma) | 2 | — |
| Enfuse | GPL-2.0-or-later (projektállítás; itt nem ellenőrizve) | tisztázandó | expozíció-fúzió (HDR jellegű) | 2 | — |
| [libvips](https://github.com/libvips/libvips) | LGPL-2.1 · `93042c1` | kód is | gyors, kevés memóriát igénylő képfeldolgozás | 2 | — |
| [CImg](https://github.com/GreycLab/CImg) | CeCILL-C / CeCILL-2.1 · `f2a8d50` | tisztázandó | képfeldolgozó primitívek, képsorozatok | 2 | — |
| [G'MIC](https://github.com/GreycLab/gmic) | CeCILL-2.1 / CeCILL-C · `3694564` | tisztázandó | programozható képszűrők, képsorozat-feldolgozás, CLI | 2 | — |
| [G'MIC-Qt](https://github.com/c-koi/gmic-qt) | GPL-3.0 · `aa4a6f4` | kód is | Qt-s szűrő-UI külön folyamatban, gazda-adapter API | 2 | — |
| [G'MIC community filters](https://github.com/GreycLab/gmic-community) | nincs licencfájl · `550c50b` | tisztázandó | közösségi szűrők | 2 | — |
| [OpenCV](https://github.com/opencv/opencv) | Apache-2.0 · `370175b` | kód is | számítógépes látás: követés, stabilizálás, maszkolás | 2 | OBJECT-MASK-PLAN-001 |
| [Open Image Denoise](https://github.com/RenderKit/oidn) | Apache-2.0 · `6602ee2` | kód is | kép-/renderzajszűrés (CPU-n is fut) | 2 | — |
| [resvg](https://github.com/linebender/resvg) | MIT / Apache-2.0 · `500d860` | kód is | SVG-renderelés tiszta Rustban (címek, grafikák) | 1 | — |
| [tiny-skia](https://github.com/linebender/tiny-skia) | BSD-3-Clause · `5d47547` | kód is | 2D raszterizálás tiszta Rustban | 1 | — |
| [Vello](https://github.com/linebender/vello) | MIT / Apache-2.0 · `6293f3b` | kód is | GPU-s 2D vektorrenderelés (wgpu) | 2 | — |
| [PhotoshopAPI](https://github.com/EmilDohne/PhotoshopAPI) | BSD-3-Clause · `ffd83ca` | kód is | PSD/PSB fájlok olvasása-írása (nyílt implementáció) | 2 | — |
| OpenRaster | nyílt specifikáció | kód is | nyílt rétegzett képformátum (.ora) Krita/MyPaint felé | 2 | — |
| [libmypaint](https://github.com/mypaint/libmypaint) | ISC · `d5a88fb` | kód is | ecsetmotor | 2 | — |
| [Krita](https://github.com/KDE/krita) | GPL-3.0 (fájlonként vegyes, REUSE) · `0ad7895` | kód is | festés, rétegek, képkocka-animáció, natív .kra-megőrzés | 2 | VISION.md |

### 3. Hang, szinkron, felirat

| Forrás | Licenc (HEAD) | Átvehető | Mit ad a CFA4-nek | Ütem | Már szerepel |
|---|---|---|---|---|---|
| [Ardour](https://github.com/Ardour/ardour) | GPL-2.0-or-later · `a8af329` | kód is | többsávos szerkesztés, nem destruktív keverés, munkamenet-csere | 2 | — |
| [Qtractor](https://github.com/rncbc/qtractor) | GPL-2.0-or-later · `a8149a9` | kód is | Qt-alapú többsávos hang-szerkesztő (Qt6 UI-minták) | 2 | — |
| [Zrythm](https://github.com/zrythm/zrythm) | AGPL-3.0-or-later + védjegyfeltételek · `1d50dc9` | csak ötlet | modern hangszerkesztő-UI minták | 2 | — |
| [DAWproject](https://github.com/bitwig/dawproject) | MIT · `ee4dcdd` | kód is | nyílt DAW-projektcsere formátum (hangátadás) | 1 | — |
| [Matchering](https://github.com/sergree/matchering) | GPL-3.0 · `f3ff9ef` | kód is | referencia alapú automatikus mastering | 2 | — |
| [Audio Separator](https://github.com/nomadkaraoke/python-audio-separator) | MIT (modellsúlyok külön) · `4dd3477` | kód is | sávszétválasztás (beszéd/zene/effekt) – szinkronhoz | MI | — |
| [BS-MelBand RoFormer](https://github.com/ZFTurbo/Music-Source-Separation-Training) | MIT (modellsúlyok külön) · `1d4b3c4` | kód is | sávszétválasztó modellcsalád | MI | — |
| [ClearerVoice-Studio](https://github.com/modelscope/ClearerVoice-Studio) | Apache-2.0 (modellsúlyok külön) · `6b3774d` | kód is | beszédtisztítás, zajszűrés, beszélőszétválasztás | MI | — |
| [Seed-VC](https://github.com/Plachtaa/seed-vc) | GPL-3.0 (modellsúlyok külön) · `51383ef` | kód is | hangkonverzió (szinkronhang) | MI | — |
| [AudioSeal](https://github.com/facebookresearch/audioseal) | MIT · `1496f76` | kód is | hang-vízjelezés MI-vel generált hanghoz | MI | — |
| [ACE-Step](https://github.com/ace-step/ACE-Step) | Apache-2.0 (modellsúlyok külön) · `1bee4c9` | kód is | szerkeszthető zenegenerálás (aláfestő zene) | MI | — |
| [DiffRhythm](https://github.com/ASLP-lab/DiffRhythm) | Apache-2.0 (modellsúlyok külön) · `28ad63c` | kód is | dalgenerálás | MI | — |
| [YuE](https://github.com/multimodal-art-projection/YuE) | kód Apache-2.0; modellsúly CC-BY-NC-4.0 · `1116bea` | tisztázandó | dalgenerálás (YuE) | MI | — |
| [vibe](https://github.com/thewh1teagle/vibe) | MIT · `da8c2ca` | kód is | helyi Whisper-átirat → feliratozás (STT) | MI | KDENLIVE-PLAN-001 |

### 4. 2D/3D animáció és renderelés

| Forrás | Licenc (HEAD) | Átvehető | Mit ad a CFA4-nek | Ütem | Már szerepel |
|---|---|---|---|---|---|
| [OpenToonz](https://github.com/opentoonz/opentoonz) | BSD-3-Clause · `a250456` | kód is | hagyományos 2D animációs munkafolyamat | 2 | — |
| [Blender](https://github.com/blender/blender) | GPL-2.0-or-later (Blender: GPL) · `ca56249` | kód is | 3D jelenet, modellezés, rigging, animáció | 2 | — |
| [Cycles](https://github.com/blender/cycles) | Apache-2.0 · `e82386e` | kód is | útkövetéses renderelés, renderháttér-absztrakció | 2 | — |
| [OpenUSD](https://github.com/PixarAnimationStudios/OpenUSD) | Tomorrow Open Source Technology License 1.0 (Apache-2.0, módosított védjegypont) · `aaf0a11` | tisztázandó | USD jelenet-összeállítás, rétegek, időmintás animáció | 2 | — |
| [MaterialX](https://github.com/AcademySoftwareFoundation/MaterialX) | Apache-2.0 · `9176de8` | kód is | anyag-/shadergráf csere | 2 | ADR-0003 |
| [ACL](https://github.com/nfrechette/acl) | MIT · `5085112` | kód is | csontváz-animáció tömörítése | 2 | — |
| [ozz-animation](https://github.com/guillaumeblanc/ozz-animation) | MIT · `744eb9d` | kód is | futásidejű csontváz-animáció, blending | 2 | — |
| [OSPRay](https://github.com/RenderKit/ospray) | Apache-2.0 · `1fc3647` | kód is | CPU-s sugárkövetés, térfogat-renderelés | 2 | — |

### 5. Helyi MI – kép és videó

| Forrás | Licenc (HEAD) | Átvehető | Mit ad a CFA4-nek | Ütem | Már szerepel |
|---|---|---|---|---|---|
| [Monet](https://github.com/Monet-AI-Editor/Monet) | MIT · `74fcb16` | kód is | ügynök vezérelte videó/kép munkafolyamat, szemantikus médiakeresés | MI | — |
| [Krita AI Diffusion](https://github.com/Acly/krita-ai-diffusion) | GPL-3.0 · `828b70c` | kód is | rétegmegőrző MI-képszerkesztés (ComfyUI-híd) | MI | — |
| [ComfyUI](https://github.com/comfyanonymous/ComfyUI) | GPL-3.0 · `0228a9d` | kód is | csomópontos MI-munkafolyamat, eredetkövetés | MI | open-editors-parity.md |
| [Fooocus](https://github.com/lllyasviel/Fooocus) | GPL-3.0 · `ae05379` | kód is | egyszerűsített helyi képgenerálás UI-minta | MI | — |
| [Wan2.2](https://github.com/Wan-Video/Wan2.2) | Apache-2.0 (modellsúlyok külön) · `1ea34ff` | kód is | helyi szöveg/kép→videó modell | MI | — |
| [Helios](https://github.com/PKU-YuanGroup/Helios) | Apache-2.0 (modellsúlyok külön) · `f1a8aad` | kód is | hosszú videó generálása | MI | — |
| [ViMax](https://github.com/HKUDS/ViMax) | MIT · `fd4b72e` | kód is | ötlet/forgatókönyv→videó, felvétel-folytonosság | MI | — |

### 6. Metaadat és tartalom-eredet

| Forrás | Licenc (HEAD) | Átvehető | Mit ad a CFA4-nek | Ütem | Már szerepel |
|---|---|---|---|---|---|
| [xmp-toolkit-rs](https://github.com/adobe/xmp-toolkit-rs) | MIT / Apache-2.0 · `c93668b` | kód is | XMP-metaadat Rustból (nyílt kód; nem Adobe-asset) | 1 | — |
| [XMP-specifikáció](https://github.com/adobe/xmp-docs) | BSD-3-Clause · `e2573ad` | kód is | XMP nyílt szabvány dokumentációja | 1 | — |
| [C2PA (c2pa-rs)](https://github.com/contentauth/c2pa-rs) | MIT / Apache-2.0 · `4973da7` | kód is | C2PA tartalom-eredetjelölés (a Rust megfelelő: c2pa-rs) | MI | — |
## 4. Kimaradt tételek (151)

A teljes, eredeti nyilvántartás a tulajdonos CFA3-mentésében megmarad. A forgatókönyv- és
zenei tételek (F, G) nem rossz források, csak nem a CFA4 mostani hatókörébe tartoznak. A
hosszú távú, a forgatókönyvtől a terjesztésig tartó tervnél újra elővehetők.

| Csoport | Ok | Db | Tételek |
|---|---|---|---|
| A | Zárt, kereskedelmi program vagy ahhoz kötött bővítmény/SDK – a CFA4 nem tölt be zárt bővítményt, és Adobe-anyagot sem használ (AGENTS.md §1) | 25 | 3DCoat, Ableton Live, Adobe Lightroom, Bitwig Studio, Cubase, FL Studio, Fender Studio Pro / Studio One, REAPER, FireAlpaca, AdobeDocs/after-effects, AdobeDocs/illustrator, AdobeDocs/photoshop-cpp-sdk, AdobeDocs/premiere-pro, AdobeDocs/uxp-after-effects, AdobeDocs/uxp-photoshop-plugin-samples, AdobeDocs/uxp-premiere-pro-samples, adobe/substance-3d-connector, adobe/pdfservices-python-sdk-samples, adobe/asset-compute-xmp, AndrewShpagin/io-coat3d, Blender 3DCoat Applink, LiamSmyth/LKS_3DCTools, GitHub Houdini topic — C language filter, MonkeyRaveProduction/houdini_configs, melMass/cops-cl |
| B | Adobe-betűkészlet – AGENTS.md §1 tiltja | 3 | adobe-fonts/source-han-serif, adobe-fonts/source-sans, adobe-fonts/source-serif |
| C | Nem szabad licenc (csak kutatási / nem kereskedelmi, vagy Elastic-2.0) | 5 | DeepAFx, adobe-research/EditVerse, adobe-research/VideoDoodles, TangoFlux, waoowaoo |
| D | NVIDIA-hoz kötött – a CFA4 gyártófüggetlen | 3 | NVIDIA cosmos, NVIDIA DALI, NVIDIA GenerativeAIExamples |
| E | Windows-kompatibilitási réteg vagy grafikus API-alap – a CFA4 Linux + Qt6 | 8 | Wine (GitHub mirror), Valve Proton, DXVK, VKD3D-Proton, WineHQ VKD3D (official), DirectX Shader Compiler, Khronos OpenGL Registry, GLFW |
| F | Forgatókönyv, regény, előgyártás, gondolattérkép – a CFA3 Story/Screenplay Studio területe, nem a CFA4 mostani hatóköre | 36 | afterwriting-labs, ai-novel2script, AICON AI Movie Studio, Coco Preproduction, DramaClaw, EdwardAThomson/NovelWriter, EdwardAThomson/StoryDaemon, How to Make Script, Jellyfish (original upstream: Forget-C), Jellyfish short-drama studio, meander, modoojunko/awesome-novel-agent, Narrative Soundstage, Nigh/show-me-the-story, olivierkes/manuskript, Open Film Skills, OpenDraft, Recut (6174), rsdoiel/fdx, saga-soft/novelWriter, Scenarly, Screenplay Tools (wildwinter), screenplay-latex, Script Breakdown Workflow (feicaiclub), ScriptBreak (Wasserman Productions), ScriptFlow, steven-tey/novel (Notion-style WYSIWYG editor), Story Architect (STARc) screenplay and project migration reference, Tome Editor, Xiaoyangy/novel-studio, ZenStory Drama Skills, Devanik21/AI-mindmap-generator, La-a11y-adm/AI-Mind-Mapping-Tools, linus-sch/Mind-Map-Wizard, u14app/mindmap, presenton |
| G | Zeneszerző DAW, szintetizátor, zenei feladatkezelő – a CFA3 Music Studio területe | 13 | Bespoke Synth, LMMS, MusE, Radium, Rosegarden, Stargate, SuperCollider, FamiStudio, WebDAW, HeartMuLa Studio, MOSS-Music, yue2.cpp, Yadaw |
| H | Játék-/valós idejű 3D, archivált tükör vagy duplikátum | 12 | GPUInstance, IMP, Three.js Awesome Graphics Agent Skills, AlternativeTo: Octane Render alternatives, pages 1–4, Pixar OpenUSD proposals, Blender official legacy add-ons mirror (archived), Blender official community add-ons mirror (archived), Blender official GitHub discovery index, G'MIC Blender add-on (historical), G'MIC Python binding, GimpComfyUI, Stability Matrix |
| I | Fejlesztői eszköz, webes UI-könyvtár, általános szolgáltatás – nem termékképesség | 21 | A11yance accessibility organization index, A11yance aria-query, A11yance axobject-query, adobe/react-spectrum, adobe/spectrum-css, adobe/leonardo, directus/visual-editing, cal.diy, coolify, inngest, react-email, public-apis, opencode, oh-my-claudecode, skills, obsidian-skills, mustafakendiguzel/claude-code-ui-agents, kaankiziltug/logo-design-skill, agenticSeek, adobe-research/figure-editing, adobe-research/sam_inversion |
| J | GitHub-témaindex vagy szervezeti lista – nincs saját kódja | 12 | GitHub image-generation topic, GitHub light-novels topic (dynamic discovery index), GitHub novel-generation topic (dynamic discovery index), GitHub topic: creative-writing, GitHub topic: mind-map, GitHub topic: mind-mapping, GitHub topic: mindmap, GitHub video-generation topic discovery index, GitHub video-generator topic index, GitHub visual-editing topic, GitHub webnovel topic (dynamic discovery index), Comfy Org repositories (discovery index) |
| K | Azonosíthatatlan: a CFA3-nyilvántartásban csak projektnév van, forráshely nélkül | 13 | ART, Autom8AI fork collection, CinePaint/Film Gimp, odysseus, Open-Higgsfield-AI, OpenCreator, OpenDirector, OpenLive, paint.software, PhotoTux, Tsumugi, Vane, Stable Audio 3 |

## 5. A Creative Studio felületterve: átvehető minták

A CFA3 README Creative Studio-képe tervkép, nem működő program. A képet magát nem hozom át.
Ezek a felületi minták illeszkednek a CFA4 Qt6 GUI-jához és a menü-/beállításpanel-szabályhoz:

- **Munkaterület-sáv egy ablakban:** Projekt, Szerkesztés, Festés, Animáció, Felirat, Hang,
  Keverés, MI, Export. Ugyanez a minta, mint az „Élő” munkaterületnél (CFA4-LIVE-STUDIO-001).
- **Feliratszerkesztő panel az idővonal mellett,** lapokkal (idővonal, szerkesztés, stílus,
  fordítás, OCR/MI) és hullámforma-nézettel.
- **Tulajdonságok/effektek panel** Videó, Hang, Felirat és MI lappal.
- **MI-asszisztens panel** feladatgombokkal (automatikus felirat, vágási javaslat, zajszűrés,
  maszkolás). Ez az [`open-editors-parity.md`](open-editors-parity.md) E3 tételével fedi
  egymást, és az MI-réteg után jön.
- **Kép- és hangszerkesztő panel a vágóablakban** (festővászon rétegekkel, hullámforma-szerkesztő,
  keverőpult) a külön program helyett.

## 6. Mire nem jogosít ez a lista

- Nem nyit feladatot és nem sorol be a tervekbe. Ehhez tulajdonosi jóváhagyás kell, a
  [`coordination.md`](coordination.md) 2.8 pontja szerinti projekthatárral.
- Nem ad függőségi engedélyt. Az új Rust-függőségnek a `cargo deny check licenses`-en át kell
  mennie (`deny.toml`).
- Kodekek és formátumok (RAW, EXR, PSD, DAWproject stb.) alapból nyilvános specifikációból
  készülnek. Meglévő implementáció átültetése csak a [`licensing.md`](licensing.md) 3. pontja szerint
  engedélyezett licencnél megengedett, és be kell vezetni az átvett kód nyilvántartásába
  ([`licensing.md`](licensing.md) 5. pont, AGENTS.md §2). A `crates/dng` például egyetlen más
  RAW-dekóder forrását sem olvasta. A LibRaw ott csak külső tesztorákulum.
- A szabályokat és a licencfeltételeket az átvétel idején újra ellenőrizni kell: a táblázat a
  2026-11-10-i `HEAD`-et mutatja.
