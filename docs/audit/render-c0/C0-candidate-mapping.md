# C0 jelölt-leképezés (`C0-candidate-mapping.md`) – VÉGLEGES

**Dátum:** 2026-11-25 · **Állapot:** VÉGLEGES, JÓVÁHAGYVA (István, 2026-11-25: a leképezést és a VapourSynth-ADR irányát elfogadta; a többcsatornás AOV a v0-ban `Unsupported`, ADR az F4 előtt). A szerződésmódosítás: `docs/adr/ADR-RENDER-WORKER-OUTPUT-INDEX-2026-11-25.md` (elfogadva, megvalósítva a 0.2-ben) ·
**Cél:** a C0 utolsó nyitott kilépési feltétele: „minden S1 jelölt leképezhető, vagy BLOCKED, vagy a szerződés ADR-rel módosul még F2 előtt”.
**Alap:** a beolvadt C0 szerződés (`cfa4-render-contracts`, `cfa4-frame`, `cfa4-tasks`), az S1-shortlist, az F1 audit (licenc, függőség, sandbox, GEGL/babl kiegészítés).
**Módszer:** csak olvasás. Amit az F1 audit nem igazol, az PENDING; mérés nincs, F2 méri. Licencszabály: kód csak saját, a leképezés külső kódot nem vesz át.

## 1. Ami a szerződésből minden jelöltre közös

| Elem | Mit jelent a jelöltnek |
|---|---|
| Kép | `VideoFrame` (`Rgba8`, `RgbaF32`, `Yuv8`, `Yuv16`, `Rgb16`); a konverziót a `cfa4_frame::convert` mátrix adja, ismeretlen pár = `Unsupported` |
| Matte | `MatteFrame` (u8/u16/f32, straight, `Tick`, eredet) – külön sík, nem színcsatorna |
| Idő | `Tick` képkockánként; csendes CFR-kerekítés tilos |
| Job | a jelölt a `cfa4-tasks` jobján át fut (`render.status/list/cancel`); a beküldés a meglévő export-parancsok |
| Worker | külön folyamat: `Hello{version, kind}` → `HelloAck` / `Reject`; `Submit{job, frame_index}` → `FrameReady` / `Failed`; `Cancel{job}`; `ResourceLimits`, `PathPolicy`; `kind` = `RenderWorker` vagy `ExternalRender` |
| Capability | `RenderCapability{format, bit_depth, place, supported, reason}`; exportnál `FailClosed`, előnézetnél `FallbackWarned` |
| Router | a meglévő `Display`/`Compute` család (D4); új család nincs |

## 2. Jelöltenként

### 2.1 OpenImageIO (kép I/O, EXR) – LEKÉPEZHETŐ, két kérdéssel
- **Határ:** olvasó/író a CFA4 és a képfájl között, nem renderer. Olvasás → `RgbaF32` (veszteségmentes), írás `RgbaF32` / `Rgb16` / `Rgba8` felől; a mátrix ezeket már tudja.
- **Folyamat:** a C ABI miatt (`unsafe`) külön worker (`RenderWorker`, `mediaHost`-hoz hasonló profil: csak a megnyitott fájl olvasása); beágyazás külön ADR (ADR 0003 A3), az FFmpeg-bővítmény kikapcsolva (A7).
- **Matte:** egycsatornás EXR csatorna ↔ `MatteFrame` f32, veszteségmentes.
- **Rés 1 – többcsatornás (AOV, multi-part) képek:** a v0 `VideoFrame` RGBA-t hordoz, több csatornát nem. Javaslat: a v0-ban az OIIO csak RGBA(+matte) csatornát ad át, a többi csatorna `Unsupported` hibával kerül ki (nem csendben); a többcsatornás szerződés az F4 előtti ADR tárgya (G-10).
- **Rés 2 – színinformáció:** a frame szándékosan nem hordoz színtér-címkét. Az OCIO/kromaticitás-metaadat a projekt színszerződéséből jön (`cfa4-color`); az F2 méri (MQ-3), hogy ez elég-e, ha nem, ADR.
- **Állapot:** leképezhető; nincs szerződésmódosítás a v0-ban.

### 2.2 VapourSynth (retusáló szűrők) – LEKÉPEZHETŐ FELTÉTELLEL, a szerződést egy ponton bővíteni kell
- **Határ:** csak külön workerben (`ExternalRender`, a `pluginHost`-hoz hasonló profil + interpreter-olvasás); Python ≥ 3.13 csak itt.
- **Kép:** `Yuv8`/`Yuv16` közvetlenül megfelel; a lebegőpontos és sík-RGB formátumra nincs `PixelFormat` – ezek csak `RgbaF32`-n át, a worker saját oldalán konvertálva; a mátrix többi párja `Unsupported`. A csere módja (D5) F2-mérésre vár.
- **Idő:** a szűrő képkockaszáma és sebessége változhat (IVTC 5→4, 2× sebességű deinterlace). A `Tick` ezt hordozni tudja, de a worker üzenet `frame_index`-e ma 1:1-et sugall, és a `RenderProgress` összege fix.
- **Rés – kimeneti indexelés:** javaslat: a `Submit`/`FrameReady` `frame_index`-e a **kimeneti** index; a worker a `HelloAck` után közli a bemeneti→kimeneti arányt és az időalapot; az összes képkockaszám a `progress` eseményben csökkenhet/nőhet (a kliens nem tekinti hibának). Ez kisebb verzióemelést (`ContractVersion` minor) és szerződésteszt-bővítést kér, ADR-rel.
- **Export:** hiányzó worker = `FailClosed` (A4); előnézetben jelzett fallback (a szűrő kihagyása, `RenderFallback` esemény).
- **Állapot:** leképezhető a fenti ADR-rel; a QTGMC-motor licence és a neo_f3kdb (GPL-3.0) miatt csak workerben maradhat (F1).

### 2.3 EC-jelölt (#11) (réteges kompozíció) – FELTÉTELESEN LEKÉPEZHETŐ; az F2 elején dől el (MQ-1)
- **Határ:** kompozíciós motor; saját projektmodellje (`.ecproj`) van, ami második projektmodell volna (A1) → csak import/export peremen, a CFA4 idővonalból épített, eldobható fordítással, nem tárolt modellként.
- **Kép:** `RgbaF32` kimenet; a szerződés `RenderCapability{RgbaF32, 32, Cpu}` a belépési feltétel. 8/16 bites projekt rétegenkénti kvantálása miatt a 32 bit/csatorna üzemmód nélkül `supported: false`, exportnál `FailClosed`.
- **Alfa:** premultiplikált lineáris `RgbaF32` a CFA4 render-szerződése; az EC-jelölt alfa-konvenciója PENDING (MQ-4), eltérés esetén a worker adja a konverziót, nem a mátrix.
- **Folyamat:** tiszta Rust, de a hibaizoláció és a második projektmodell elkülönítése miatt külön worker javasolt (`RenderWorker`); a külső effekt-helper minta (ADR 0001) csak minta, kód nem kerül át.
- **Rés:** nincs szerződési rés; a kérdés mért (MQ-1/3/4/8). Ha az MQ-1 32 bites út nem érhető el: **BLOCKED** képláncnak, csak minta.

### 2.4 GEGL + babl (gráf és pixelformátum) – LEKÉPEZHETŐ, workerként, szűkített operációkészlettel
- **Határ:** a CFA4 render-gráfja (F3) a CFA4 sajátja; a GEGL csak operációkönyvtár/összehasonlítási pont, képkockánként hívva. A babl a konverzió összehasonlítási pontja a `cfa4_frame::convert` mátrixhoz (ΔE2000, alfa), nem helyettese.
- **Kép:** a GEGL belső lineáris, lebegőpontos, premultiplikált munkaformátuma elvileg egyezik a CFA4 render-szerződésével (`RgbaF32`); ez **általános ismeret, F2-ben ellenőrizendő**, nem igazolt (F1-szabály).
- **Folyamat:** kötelező GLib/GObject-lábnyom miatt kizárólag külön worker (`RenderWorker`); moduláris operációbetöltés Landlock alatt PENDING (F1 6. pont). A `common-gpl3+` operációk miatt a teljes build GPL-3+ hatású: összefér a CFA4 licencével, de LGPL-csomaghoz szűkíteni kell.
- **Rés:** nincs szerződési rés; a képkocka-csere módja (D5) F2-mérésre vár.
- **Állapot:** leképezhető; PENDING: lábnyom és sandbox-mérés a referenciagépen.

## 3. Összegzés

| Jelölt | Leképezés | Szerződésmódosítás F2 előtt | Mi dönt |
|---|---|---|---|
| OpenImageIO | LEKÉPEZHETŐ | nincs (többcsatornás kép: ADR F4 előtt) | MQ-3 (színinfó) F2-ben |
| VapourSynth | LEKÉPEZHETŐ feltétellel | **igen: kimeneti `frame_index` és változó képkockaszám (ADR, minor verzió)** | az ADR jóváhagyása |
| EC-jelölt (#11) | FELTÉTELES | nincs | MQ-1 (32 bit/csatorna); ha FAIL: BLOCKED |
| GEGL + babl | LEKÉPEZHETŐ | nincs | F2-mérés (D5, lábnyom, sandbox) |

Az egyetlen szerződésmódosítás tehát a VapourSynth miatti, kicsi, visszafelé kompatibilis bővítés. Javaslat: ha jóváhagyod, ADR-tervezetként (md) hozom, a szerződésteszt bővítésével együtt, csak jóváhagyás után kódolom.

## 4. Döntések (István, 2026-11-25: mind a három javaslat elfogadva)

| # | Kérdés | Döntés |
|---|---|---|
| M1 | A leképezés a négy jelöltre | **Elfogadva** |
| M2 | VapourSynth: a `frame_index` a kimeneti index, a worker közli az arányt és az időalapot, a progress összege változhat | **Elfogadva**; ADR-tervezet jóváhagyásra |
| M3 | Többcsatornás (AOV) kép | **Elfogadva:** a v0-ban `Unsupported`, ADR az F4 előtt |

A C0 harmadik kilépési feltétele („minden S1 jelölt leképezhető, BLOCKED, vagy ADR-rel módosul”) ezzel teljesül, az ADR elfogadásával és megvalósításával lezárt.

## 5. Mit nem ellenőriztem
Nem futtattam egyik jelöltet sem, nem építettem és nem mértem; a GEGL munkaformátuma és az EC-jelölt alfa-konvenciója nincs igazolva. A VapourSynth képkocka-tulajdonságait (időalap) nem néztem forrásból. A SECURITY-001 felelősének a `renderWorker`/`externalRender` profilokra adott egyetértése nincs meg.
