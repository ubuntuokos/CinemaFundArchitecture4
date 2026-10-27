# CFA4 Smart Render – megvalósítási terv

**Dátum:** 2026-10-27  
**Státusz:** megvalósítási terv; a natív CFA4-integráció irányát a korábbi tulajdonosi döntés rögzíti  
**Kapcsolódó terv:** [GOP-alapú Smart Cut](GOP-SMART-CUT-2026-10-27.md)

## Cél

A CFA4 timeline exportja a változatlan, közvetlenül másolható klipszakaszokat tömörített mintaként őrizze meg, és csak azokat a szakaszokat renderelje/kódolja újra, amelyeken vágási határ, átmenet, effekt, kompozitálás, sebességváltozás, szín-/méretkonverzió vagy exportbeállítás miatt képtranszformáció szükséges. A végeredmény ugyanazt a timeline-képet, időzítést, hangot és kimeneti specifikációt adja, mint a jelenlegi teljes render-export. A funkció a meglévő Rust exporter része; nincs külön alkalmazás vagy külső renderfolyamat.

## Architektúra

A jelenlegi `crates/export/src/pipeline.rs` képkockánként rendereli a szekvenciát; `crates/export/src/job.rs` az elkészült képkockákat kódolja és muxolja. A konténermodulok (MP4/MOV, Matroska, MXF) már rendelkeznek írási útvonallal. A Smart Render e pipeline előtt egy **render/copy szakasztervezőt** futtat, majd a kimenetet időrendben, egyetlen muxerbe írja: változatlan tömörített minták másolása és újrarenderelt/újrakódolt szakaszok keverednek.

Nem elég egy klipre ráírni, hogy „effekt nélküli”. A másolhatóságot a teljes timeline-rész, a dekóder/encoder képesség, a codec config, a frame rate/timebase, a color/alpha, a hang, a konténer és a szakaszhatáron érvényes GOP/dekódolási függőség együtt dönti el.

## Kiinduló állapot

- `crates/export`: jelenlegi frame-render → encode → mux pipeline; GPU és CPU renderer, cancel/progress.
- `crates/render`: a timeline összetett képének előállítása.
- `crates/project` és `crates/edit`: szekvencia, klip, sáv, időzítés és vágásmodell.
- `crates/codecs/src/gop.rs`: GOP cache a dekódoláshoz, nem muxolási/streamcopy planner.
- `crates/isobmff`, `crates/matroska`, `crates/mxf`: különböző demux/mux képességek; nem feltételezhető, hogy mindegyik kezeli a vegyes copy/render streamet.
- H.264 export és ProRes/DNx kódolók léteznek, de az export UI, profile/bitdepth, hardware és container kompatibilitás nem egyenlő a Smart Render-képességgel.

## Renderelési és másolási szabályok

A planner minden kimeneti képkocka-intervallumot az alábbi osztályok egyikébe sorol:

- **Copy-eligible:** egyetlen forrásszakasz; nincs effekt/átmenet/kompozitálás/transform; sebesség 1:1; teljes képkocka lefedettség; forrás- és kimeneti frame-rate/timebase kompatibilis; codec config, pixel format, szín/HDR, méret, aspect és kimeneti muxer támogatott; GOP-határ zárhatóan folytatható.
- **Render-required:** több réteg, alpha/blend, transition, effect, title/overlay, resize/scale, transform, speed/rate stretch, freeze, frame interpolation, stabilizálás, color conversion, project/export frame-rate eltérés vagy nem kompatibilis bitstream.
- **Boundary-required:** egyébként másolható szakasz, amelynek eleje/vége csak dekódolás és lokális újrakódolás után kapcsolható biztonságosan a szomszédos szakaszhoz.

A copy-eligible szakaszokat csak egész, biztonságosan dekódolható GOP-egységekben lehet másolni. Ha a közös kimeneti codec-config, keyframe/seam, időbélyeg vagy hang-időzítés nem garantálható, az érintett tartomány renderelendő; ha a kimenet ilyen vegyes formában sem állítható elő, az export előzetes validációban leáll.

## Copy-módok és garanciák

A CFA4 korábban rögzített Stream Copy döntése szerint két mód önállóan, külön választható:

- **Bitazonos:** az átmásolt tömörített minták payloadja bájtról bájtra változatlan. A teljes fájl bájtazonossága nem ígérhető, mert új timeline-t kell muxolni. A renderelt szakaszok természetüknél fogva új minták.
- **Kompatibilis:** az átmásolt szakasz dekódolt tartalma változatlan marad; az időbélyeg, konténer-index, codec metadata mező és muxolási elrendezés a támogatott konténerkövetelmény szerint újraszámítható.

A felhasználó a copy-policyt külön választja. Nincs silent fallback és nincs úgynevezett „quality-preserving” mód, amely elrejti az újrakódolást. A felületen meg kell jeleníteni a másolt/renderelt időtartamot, az érintett sávokat, az esetleges encoderhasználatot és a garanciát.

## Megközelítések

| Megközelítés | Előny | Korlát |
|---|---|---|
| Teljes timeline újrarenderelése (mai út) | Általános, stabil, egységes kimenet | Feleslegesen újrakódolja a változatlan részeket |
| Minden klip stream-copy, render nélkül | Gyors | Effektek, transitionök, illesztés és A/V sync hibás vagy elvesző |
| **Szegmensenkénti hibrid Smart Render (kiválasztott)** | **Megőrzi a másolható GOP-okat, a szükséges tartalmat rendereli** | **Előzetes képesség- és seam-elemzést, valamint konténerenkénti validációt igényel** |

## Terv

### 1. fázis – Szakaszbesorolás és előzetes becslés

**Fájl:** új `crates/export/src/smart_render.rs`; integráció a `settings.rs`/export-entry pontba.

- Definiáld a szerializálható `SmartRenderOptions`, `CopyPolicy`, `RenderSegment`, `SegmentReason`, `SmartRenderPlan`, `SmartRenderReport` adatmodelleket.
- Planner bemenete immutable `Project`/sequence snapshot és `ExportSettings`; kimenete a kimeneti frame/time intervallumok rendezett listája: Copy, Boundary, Render.
- `SegmentReason` géppel olvasható és UI-ban emberi szövegre képezhető; tartalmazza a sáv/klip azonosítót, forrástartományt, okot és releváns codec/container korlátot.
- A planner egyértelműen végigveszi: egyszeres klip, gaps/fekete, audio-only szakasz, linked audio, proxy/original, nested sequence, transition hatás, export overlay, bitdepth/color/HDR, scaling, framerate és speed.
- Minden numerikus idő `Tick`/racionális frame-time. A plan cache kulcsa a snapshot hash + exportbeállítás + codec capability verzió; elavult terv újraépül.
- UI-nak előre mutassa, mely részek másolhatók és melyek renderelendők, becsült render-időt (ha kalibrált), valamint a „miért renderel” indokot.

**Tesztek:** no-op klip → copy; effect/transition/overlay/resize/speed/framerate mismatch → render; kevert track → korrekt interval partition; null/üres sequence; nested cycle/deep nesting; snapshot változás cache invalidálást okoz.

### 2. fázis – Közös dekódolási és konténer seam-képesség

**Fájlok:** `crates/export/src/smart_render.rs`, a konténer/codec capability API-k.

- Adj vissza capability objektumot a codec config, sync sample, open/closed GOP, reference-frame dependencies, PTS/DTS reorder, pixel format, timebase, color/HDR, audio config és célmuxer alapján.
- Határozd meg a kimeneti GOP varratot: másolt GOP kezdetének random-access frame-nek kell lennie; az újrakódolt szakasz végének/kezdetének azonos codec configú, dekódolható határra kell esnie.
- Külön kezeld a kimeneti frame rate-et és forrás timebase-et. Fractional rate/drop-frame timecode nem kerekíthető f64 másodpercben.
- A hangmásolás csak audio sample-frame és codec delay/priming/padding kezelés mellett engedélyezhető. Ha videó Copy, de hangot mix/FX érint, a két stream külön segment-tervvel, de közös muxer timestamp tengellyel fut.
- A muxernek támogatnia kell az adott codec-paraméter-készletű minták folyamát; MP4/MOV, Matroska, MXF, MPEG-TS külön feature kapu.

**Kész, ha:** a planner nem jelöl copy-ra olyan szakaszt, amelynek decoder-seam, codec config vagy muxer korlátja nincs bizonyítva. MPEG-TS cél csak a dedikált MPEG-TS muxer terv/implementáció után.

### 3. fázis – Egy muxerbe író vegyes végrehajtó

**Fájl:** új `crates/export/src/smart_render_job.rs`; integráció a `Exporter`/job lifecycle-ba.

- A `SmartRenderPlan` szegmenseit időrendben fogyaszd; Copy szegmens packet/sample-okat ír, Boundary/Render szegmens a meglévő `Pipeline`-on, renderer poolon és encoder-regiszteren megy át.
- Az encoder és a muxer egységes kimeneti `TrackConfig`/codec configot használ. Nem szabad menet közben olyan paramétert váltani, amelyet a célkonténer egy tracken belül nem támogat.
- Minden szegmensnek explicit kimeneti PTS/DTS kezdete és vége van; monotonitást, durationt, frame-countot és hang/videó végpontot ellenőrizni kell.
- A másolt payloadot ne alakítsd képkockává, ha nem kell decode-olni seam ellenőrzéshez. Az összes másolás előtt a planner a safety-dependent prerollt meghatározza.
- Progress, cancel, renderer-hiba, encoder-hiba és file-finalization ugyanazt a biztonságos `Exporter` lifecycle-t követi; failed render részszegmens nem írhatja felül a végleges kimenetet.
- Determinisztikus export: ugyanazon snapshot, beállítás és policy azonos output frame/sample tartalmat és timestampsorozatot adjon; a meglévő teljes render út marad CPU reference fallback.

**Tesztek:** egyetlen copy szegmens; copy-render-copy; két codec config miatt tiltott terv; B-frame timestamp; GOP seam decode; audio FX + video copy; megszakítás minden szegmenstípus közben; determinisztikus export több batchmérettel.

### 4. fázis – Engine, Qt6, CLI/MCP és export-profilok

**Érintett területek:** `crates/engine` export parancsok; `python/cfa4_qt` exportpanel; CLI/control protocol.

- Tegyél elérhetővé `export.smartRender.plan`, `export.smartRender.start`, `export.smartRender.cancel` parancsot; a meglévő sima export változatlanul elérhető.
- Export UI-ban legyen explicit Smart Render be/ki kapcsoló, Bitazonos/Kompatibilis policy, preview table és okonkénti részletezés. Nincs „auto copy” választás, amely elfedné a tényleges működést.
- Export preset csak akkor ajánlhat Smart Rendert, ha a célcodec/bitrate/profile és konténer capability átmegy a preflighton.
- Az állapot és parancsok automatizálhatóak; UI, CLI és control/MCP azonos engine-kódot használ.
- Qt6 UI pontosan mutassa, ha minden szakasz renderelendő; a felhasználó döntheti el, hogy így is folytatja-e normál exporttal.

### 5. fázis – Bekapcsolás, kompatibilitási kapuk és mérések

- Először egy igazolt, szűk profil-készleten kapcsold be, pl. azonos kimeneti frame rate/méretű H.264, effekt/transition nélküli, zárt GOP-os single-layer klipszakasz és az adott konténer támogatott audioja.
- Ezután külön PR/fázisban add hozzá a ProRes/DNx intra, HEVC, MPEG-2, nested sequence és audio-only kombinációkat.
- Open GOP, B-frame reorder, variable frame rate, HDR/10-bit/4:2:2, speed effect, proxy, nested/transition és multitrack csak célzott conformance case után engedhető.
- Benchmarkon mérd: változatlan timeline teljes re-encode vs Smart Render; másolt minta arány; újrakódolt frame arány; időmegtakarítás; kimenet minőségi/bitstream különbség; A/V drift. Kis, közepes és hosszú timeline, cache hideg/meleg.
- Futtasd a releváns crate teszteket, workspace/layering/CI kapukat és Qt6 full export/reimport end-to-end-t.

## Elfogadási kritériumok

- A Smart Render kimenet látható képkockái, frame countja, frame time-ja, audio sample countja és A/V szinkronja egyezik a teljes render referenciaexporttal.
- Minden másolt payload a Bitazonos módban bájtazonos; Kompatibilis módban bitstream-dekódolás után a dokumentált tartalmi egyenértékűséget teljesíti.
- Effektes/összetett/nem kompatibilis szakasz renderelendőként szerepel, nem tűnik el, és nem másolódik változtatás nélkül.
- Minden váltási pont seekelhető, decode-olható és időbélyeg szerint folytonos; eltérés esetén a teljes export a preflightban elutasít vagy a kérdéses tartományt rendereli.
- Normál full-render export továbbra is ugyanúgy elérhető, és referenciaút marad.

## Felülvizsgálati fókusz

- Szegmenshatár pontatlan képkocka-rács vagy nem egész számú fps esetén: nincs frame duplikáció/vesztés.
- Nyitott GOP, B-frame, CRA/RASL, codec config váltás: nincs hibás dekódolás a seam-en.
- Transition, keyframe animáció, adjustment layer, export overlay és alpha: minden vizuálisan érintett frame renderelendő.
- Audio effekt, track mute/solo, sample rate conversion és AAC priming: nincs elnémulás vagy hangcsúszás.
- Cancel, lemezbetelés, encoder/renderer hiba: nincsenek maradandó részfájlok vagy sérült project állapot.

## Függőségek és nem célok

- A GOP Smart Cut terv capability és seam megállapításai közös alapként felhasználhatók; az export timeline-planner külön komponens.
- A korábban rögzített két Stream Copy mód: Bitazonos és Kompatibilis.
- MPEG-TS/PS muxer hiányában ilyen célkonténer Smart Render export nem kapcsolható be; ezt a külön MPEG-TS muxer munka előzi meg.
- Nem cél a minden timeline-ra automatikusan érvényes copy ígérete; a képességmátrix elutasíthat vagy renderre sorolhat.
- Nem cél a codec/decode/render logika Pythonba vagy külön folyamatba másolása.
