# CFA4-SHOTCUT-CAPABILITY-ADAPTATION-001 / SC-5.0: a Shotcut `.mlt` formátum ellenőrzése (jelentés)

**Állapot:** SC-5.0 **részben kész**: a formátum a Shotcut és az MLT forrásából, `melt`-kísérletekkel és **egy valódi Shotcut 26.2.16 (MLT 7.36.1) mintafájllal** (a tulajdonostól, 2026-11-22) ellenőrizve; **több sávos, szűrős, átmenetes és vágott valódi minta még hiányzik** (7. pont). A megvalósítás (SC-5.1…) külön indítási engedélyt kér.
**Dátum:** 2026-11-22 · **Terv:** [`CFA4-SHOTCUT-CAPABILITY-ADAPTATION-001-SC5-MLT-IMPORT.md`](CFA4-SHOTCUT-CAPABILITY-ADAPTATION-001-SC5-MLT-IMPORT.md) (jóváhagyva 2026-11-22; D1 közös MLT-mag, D2–D6 alapértelmezés) · **Alap:** `main` @ `bd84c4d` (a terv-PR #1559 nincs beolvasztva, erre a jelentés nem épít).
**Nyelv:** magyar. Kód nincs a jelentésben; a tesztadat és a generátorszkript a PR-ban van (6. pont).

**Módszer és eredet (nyilvánosság):**
- Shotcut forrása: `mltframework/shotcut` `master` @ `2eba56a72af3b7806876f2a2956cfff789db1468` (2026-11-21), **GPL-3.0**; csak **olvastam**, semmit nem másoltam át.
- MLT forrása: `mltframework/mlt` `master` @ `34088d6f47ae8896ab71101ddd72f600b3a18f8d` (2026-11-20), LGPL-2.1-or-later; csak olvasás.
- Valódi minta: `shotcut-26.2.16-single-clip.mlt` (Shotcut 26.2.16, MLT 7.36.1; egy videóklip a V1 sávon, `mix` és `qtblend` átmenet-transitionök, 1280x720 @ 30 fps), a tulajdonos töltötte fel; a repóba az útvonal (személyes könyvtárnév) cseréjével kerül, mást nem szerkesztettem rajta.
- Kísérletek: `melt` 7.22.0 (Ubuntu csomag), `ffmpeg`/`ffprobe` a generált médiához. A `melt` csak **tesztorákulum** (E11), nem kerül a termékbe.
- A Shotcut-kiadás pontos száma nem ismert (a sekély klón nem hordoz címkét): a „legfrissebb stabil” (D4) az SC-5.1 előtt még megnevezendő.

---

## 1. Megerősített tények

| # | Tény | Forrás / bizonyíték |
|---|---|---|
| T1 | A Shotcut projekt MLT XML: `<mlt LC_NUMERIC version root title producer>`, `<profile>` kötelező attribútumai: `width, height, frame_rate_num/den, progressive, sample_aspect_num/den` (a megjelenítési képarány és a színtér opcionális) | `mlt/src/modules/xml/mlt-xml.dtd` |
| T2 | A Shotcut **óra-formátumban** ment (`HH:MM:SS.mmm`) | `shotcut/src/mltcontroller.cpp:514, 577` (`time_format=clock`) |
| T3 | Az óra-forma visszaolvasása: `floor(fps·óra·3600) + floor(fps·perc·60) + lrint(fps·másodperc)`; a CFA4-olvasónak **ugyanezt a képkockaszámot kell előállítania**, és csak azután váltson `Tick`-re (képkocka → Tick pontos) | `mlt/src/framework/mlt_property.c` `time_clock_to_frames` (426–451. sor) |
| T4 | A `<entry>` `in`/`out` **képkocka-index**, és az `out` **az utolsó, benne lévő** képkocka (hossz = `out − in + 1`); az `<blank length>` viszont **képkockaszám** (hossz = `length`). Bizonyíték: egy 0…49 képkockás szakasz `out="00:00:01.960"` (=49/25), a 10 képkockás üres rész `length` = 0,4 s | `consumer_xml.c` 481–510. sor; `melt` kísérlet (`fixtures/mix-luma-melt.mlt`) |
| T5 | Szerkezet: `background` playlist (fekete `color` producer) + `main_bin` playlist (a Shotcut lomtára: `xml_retain`, `shotcut:projectAudioChannels`) + egy playlist **sávonként** + egy fő `<tractor>` (`shotcut`=1 tulajdonság), benne `<track producer=…>` | `shotcut_mlt_properties.h`, `multitrackmodel.cpp` (3898, 3945, 3521), `mltxmlchecker.cpp` |
| T6 | Sávtípus és sávadatok: `shotcut:video` / `shotcut:audio` (a sáv-playlisten), `shotcut:name`, `shotcut:lock`, `shotcut:trackVolume…`; némítás/rejtés a `hide` attribútummal (MLT: 1 = a kép rejtve, 2 = a hang rejtve, 3 = mindkettő; az XML-ben szó alakban is: `video`, `audio`, `both`): az olvasónak **mindkét alakot** értenie kell | `multitrackmodel.cpp` 488–490, 705–737, 4735–4774; `consumer_xml.c` |
| T7 | Sávonkénti keverés a tractorban: hang `mix` (`a_track`=0, `b_track`=i, `always_active`=1, `sum`=1); kép `qtblend` (CPU) / `frei0r.cairoblend` / `movit.overlay` (GPU) / régi `composite` (a Shotcut `qtblend`-re alakítja) | `multitrackmodel.cpp` 3639–3643, 3742–3751, 3905–3910, 4847–4852 |
| T8 | **Klipek közti átmenet = „mix” bejegyzés a playlistben** (nem átfedés): egy kis `<tractor>` két sávval (A vége, B eleje) + `luma` (vagy GPU-n `movit.luma_mix`) és `mix:-2` hang-keresztáttűnés; a tractoron `shotcut:transition` (alapérték `lumaMix`). A bal klip vége és a jobb klip eleje **rövidül** a mix hosszával | `multitrackmodel.cpp` 45, 2540–2570; `mlt_playlist.c` `mlt_playlist_mix` (1195. sor); `melt` kísérlet |
| T9 | Klipmarkerek: a producer `shotcut:markers` tulajdonságcsoportja, elemenként `text`, `start`, `end` (óra-forma; `start == end` pontmarker), `color` (`#rrggbb`) | `shotcut/src/models/markersmodel.cpp` 39–58; `shotcut:markers` |
| T10 | Szűrő-azonosság: a Shotcut a szűrő **saját nevét** a `shotcut:filter` tulajdonságban őrzi (a `meta.qml` `objectName`-je, pl. `audioGain`, `fadeInBrightness`, `brightnessOpacity`); **több Shotcut-szűrő ugyanazt az MLT-szolgáltatást használja** (`volume`: erősítés, fade be/ki, némítás; `brightness`: fényerő, átlátszóság, fade be/ki; `lift_gamma_gain`: szín, kontraszt; `affine`: forgatás) | `shotcut/src/qml/filters/*/meta.qml`; 8. pont melléklete |
| T11 | A GPU-mód (Movit) saját szolgáltatásokat használ (`movit.opacity`, `movit.luma_mix`, `movit.overlay`, …); az importernek a CPU- és a GPU-változatot **egyformán** kell kezelnie | `meta.qml` fájlok; `multitrackmodel.cpp` |
| T12 | A kulcskockás paraméter MLT-animációs szöveg (`0=0;00:00:00.480=1`); az interpoláció a kulcs előtti jel: üres = lineáris, `|`/`!` = ugrás, `~` = sima, `$` = természetes, `-` = szoros, `a`…`l` = szinuszos, négyzetes, köbös, negyedfokú változatok | `mlt/src/framework/mlt_animation.c` 67–89. sor |
| T13 | Sebesség: `timewarp` producer, `resource` = `<sebesség>:<fájl>`, `warp_speed`, `warp_resource` | `mltxmlchecker.cpp` 969–984; `proxymanager.cpp` 83–84 |
| T14 | A Shotcut elemzője az XML-ben `producer`, `chain`, `link`, `filter`, `playlist`, `tractor`, `track`, `transition`, `profile`, `consumer` elemeket ismeri; az olvasónak a **`<chain>` és a `<producer>` alakot is** értenie kell; a Shotcut a fájlprodukciót `avformat-novalidate` szolgáltatásra cseréli | `mltxmlchecker.cpp` 41–43; `mltcontroller.cpp` 1111–1112 |
| T16 | **Valódi mintával igazolt** (Shotcut 26.2.16): `title="Shotcut version 26.2.16"`, `producer="main_bin"` a gyökéren; az üres `main_bin` csak `xml_retain`=1-et hordoz (a lomtárba nem került klip); a videóklip `<chain … out="00:00:03.967">` (**`in` hiányzik, ha 0**, az olvasó 0-nak veszi), `mlt_service` **tulajdonságként** (nem attribútumként) `avformat-novalidate`, `length` óra-formában (`00:00:04.000` = 120 képkocka); az `out="00:00:03.967"` 30 fps-en a 119. képkocka, tehát a klip 120 képkockás (T4 megerősítve); a fő tractor `shotcut`=1; `mix` (`always_active`=1, `sum`=1) és `qtblend` (`compositing`=0, `distort`=0, `rotate_center`=0, `threads`=0, **`disable`=1**: letiltva, amíg nincs szükség rá) a sáv transition-jei (T7 megerősítve); a `transition`/`filter` szolgáltatásnév is **tulajdonság** (`<property name="mlt_service">`); `shotcut:projectFolder`, `shotcut:processingMode` (`Native8Cpu`), `shotcut:hash`, `shotcut:caption` a chainen; `xml`=`was here` jelölő; az `audio_index`=-1 hangtalan klipet jelent | `fixtures/shotcut-26.2.16-single-clip.mlt`; `melt` 7.22.0: a fájl betölt és **120 képkockát** renderel (csak a `qtblend` modul hiányzik ezen a gépen) |
| T15 | Proxy: `shotcut:proxy`, `shotcut:originalIn/Out`, `shotcut:resource` őrzi az eredetit; további `shotcut:` kulcsok: `hash`, `caption`, `comment`, `group`, `hidden`, `animIn/animOut`, `defaultAudioIndex`, `projectFolder`, `projectNote`, `colorTransfer` | `shotcut_mlt_properties.h` |

---

## 2. A terv javítandó pontjai (az SC-5.0 eredménye)

| # | A tervben | Javítás |
|---|---|---|
| **C1** | 3.3: „a Shotcut-áttűnés a sávon belüli átfedésből adódik”; SC-5.2 „átfedés → átmenet” | **Téves.** A klipek közti átmenet külön **mix-bejegyzés** (T8): az olvasónak a playlist `<entry producer="tractorN">` elemét kell felismernie (a tractoron `shotcut:transition`, a transition `luma`/`movit.luma_mix`), a mix hosszát a tractor `in/out` adja, a szomszédos klipek már rövidítettek. A sorozat hossza így nem csúszik el, ha a mixet **egy** átmenetként olvassuk. Az SC-5.2 leírását ennek megfelelően kell érteni |
| **C2** | 3.2: az `out` jelentése „ellenőrzendő” | **Ellenőrizve (T4):** az `out` inclusive, a `blank length` darabszám. A két mező más szemantikájú, az olvasó külön kezelje |
| **C3** | 3.1: „sáv-playlist `shotcut:video/audio` jelölés” | Kiegészítés: `hide` szám- és szóalak (T6), a fő tractor `shotcut`=1 jelölése és a `background` sáv (T5) kihagyandó a CFA4-sávok közül |
| **C4** | 3.4: szűrőtábla szolgáltatásnév szerint | Kiegészítés: a leképezés kulcsa **(`shotcut:filter` név, MLT-szolgáltatás)** pár, mert egy szolgáltatásnév több Shotcut-szűrőt takar (T10); a hiányzó `shotcut:filter` esetén a szolgáltatásnév és a paraméterek döntenek; a GPU-párok (T11) azonos sorba tartoznak |
| **C5** | 3.4: „vágás (Crop)” egyetlen szűrő | Két szolgáltatás: `crop` (szélek számmal) és `qtcrop` (`cropRectangle`, `rect`/`radius`), a `crop_circle` is `qtcrop`; mindkettőt le kell képezni a CFA4 Crop effektre, a lekerekítés közelítés |
| **C6** | 3.4: „fade be/ki” | A Shotcutban háromféle: kép `brightness` kulcskockával (CPU) vagy `movit.opacity` (GPU), hang `volume` kulcskockával (`level`); az `autofade` szolgáltatás is létezik. D5 (CFA4 Fade In/Out) ezekre érvényes |
| **C7** | 1. pont: „`.mlt` kiterjesztés és `<mlt>` gyökér” felismerés | A Shotcut-eredet a `shotcut`=1 tractor-tulajdonságból és a `shotcut:*` annotációkból ismerhető fel; a közös MLT-mag (D1) ezt dialektus-jelzőnek használja, nem kizárási feltételnek |

**A C1–C7 pontosításokat István elfogadta (2026-11-22)**, és átvezetésre kerültek a tervbe (SC-5 terv, 13. pont; PR #1559).

---

## 3. Mit mértem és mit nem

- **Mértem (`melt` 7.22.0):** `mix-luma-melt.mlt` (két klip, 10 képkockás luma-mix) renderje **90 képkocka** (40 + 10 + 40, a T4/T8 szerint); `timeline-shotcut-style.mlt` renderje **100 képkocka** (a tractor hossza). A `melt` csak a kimeneti képkockaszámot adta; a képkockánkénti összevetés az SC-5.6 dolga.
- **A fixture-ben szándékosan hibás elemek** (a `melt` jelzi, a CFA4 olvasónak jelentenie kell majd): `frei0r.nonexistent_filter_for_unsupported_test` (ismeretlen szűrő) és `frei0r.cairoblend` (ezen a gépen nincs frei0r; a Shotcut sávátlátszósági transition-je, letiltva).
- **Nem mértem:** a mix B-oldalának pontos forrástartományát: a `melt`-kimenet a B klip belépési pontját a mix előttivel azonosnak mutatja, a kódolvasás (`mlt_playlist_mix`) viszont a B bejegyzést a mix hosszával előre lépteti; ezt az SC-5.2 a `melt`-képkockaösszevetéssel dönti el, addig a B-oldal forrástartománya **nem igazolt**.

---

## 4. Az SC-5.1…5.3 számára kötelező olvasási szabályok (a tényekből)

1. Idő: az MLT-képlettel képkockára (T3), utána `Tick`; `in`/`out` index, `out` inclusive; `blank length` darabszám (T4).
2. Sáv: `shotcut:video`/`shotcut:audio` + `hide` (szám és szó) (T6); a `background` és a `main_bin` nem idővonali sáv (T5).
3. Átmenet: playlist-mix tractor (T8); sávblend-transitionök (T7) a sáv átlátszóságaként/keverési módjaként, nem klipátmenetként.
4. Szűrő: (`shotcut:filter`, szolgáltatás) pár; animációs szöveg a T12 szerint; kikapcsolt szűrő (`disable`) kikapcsolva marad.
5. Elemek: `producer` és `chain` egyaránt; `avformat`, `avformat-novalidate`, `timewarp`, `color`, képsorozat; ismeretlen szolgáltatás → jelentés.
6. Marker: `shotcut:markers` (T9).

---

## 5. Hogyan használható a közös MLT-mag (D1)

A Kdenlive-terv K1/K42 ugyanezt a szerkezetet olvassa (`kdenlive:` tulajdonságokkal). A közös mag felelőssége a T1–T5, T8, T12–T14 szerinti, dialektusfüggetlen rész (profil, idő, producer/chain, playlist, tractor, transition, szűrő és animáció); a dialektusonkénti rész a `shotcut:*`/`kdenlive:*` tulajdonságok és a szűrőtáblák. Az SC-5.1 és a K1 első szállítása ezért **egy közös PR-sorozat** legyen; ennek ütemezése a tulajdonos és a gazda döntése (§2.8).

---

## 6. A PR tartalma (tesztadat és eszköz)

- `crates/interchange/tests/data/shotcut/mix-luma-melt.mlt`: az MLT saját XML-kimenete egy luma-mixes két klipről (`meta.*` tulajdonságok nélkül).
- `crates/interchange/tests/data/shotcut/timeline-shotcut-style.mlt`: kézzel írt, a valódi minta alakját követő (chain, tulajdonság-alakú `mlt_service`) két sávos idővonal (üres részek, trimmelés, kulcskockás fade, kikapcsolt szűrő, `crop`, ismeretlen szűrő, `timewarp`, marker, mix- és blend-transition).
- `crates/interchange/tests/data/shotcut/shotcut-26.2.16-single-clip.mlt`: a tulajdonos valódi Shotcut 26.2.16 mintája (útvonal és felirat cserélve).
- `crates/interchange/tests/data/shotcut/README.md`: eredet és licenc (saját készítés, CC0).
- `scripts/dev/sc5-make-fixtures.sh`: a médiát (`a.mp4`, `b.mp4`) generálja a `target/fixtures/shotcut/` alá (nem commitolt), és `melt`-tel kiszámolja a képkockaszámot.

---

## 7. Mit nem ellenőriztem, és mi kell az SC-5.0 lezárásához

1. **Egyetlen valódi Shotcut-projektet láttam** (T16: egy klip, egy sáv, nincs vágás, szűrő vagy klipek közti mix, nincs hang). A cloud-környezetben nincs Shotcut (grafikus alkalmazás). A lezáráshoz még kellene **2–3 valódi `.mlt`** a tulajdonos Shotcutjából: (a) két videósáv + egy hangsáv, vágott és trimmelt klipekkel, üres résszel; (b) két szomszédos klip köztes átmenettel (a T8 mix-bejegyzés valódi alakja) és hang-keresztáttűnéssel; (c) néhány szűrő (fade be/ki, fényerő, vágás, hangerő), egy sebességváltás és egy klipmarker. Idegen vagy jogilag nem tisztázott projektfájl nem kerül a repóba.
2. A D4 szerinti „legfrissebb stabil” a mintából **26.2.16**; a **régebbi verzió** neve még nincs megadva.
3. A **teljes szűrő- és paraméter-táblát** (K20-minta) nem készítettem el; a 8. pont melléklete csak a szolgáltatás-leképezés kiindulópontja. A paraméternevek és skálák (fényerő, kontraszt, telítettség) és a Lumetri-megfeleltetés az SC-5.3 dolga.
4. A `shotcut:transition` további értékeit (az `lumaMix`-en kívül) nem soroltam fel.
5. A mix B-oldalának forrástartománya (3. pont).
6. A Shotcut `Subtitles`, `Overlay HTML` és `Text` szűrők tartalom- és stílusmezőit nem vizsgáltam részletesen.
7. A `plan/` mappát nem láttam (gép-lokális).

---

## 8. Melléklet: Shotcut-szűrő → MLT-szolgáltatás (a `meta.qml` fájlokból, 143 szűrő)

Formátum: `shotcut:filter név → MLT-szolgáltatás`. Hang: (h). A név nélküli sorok (a `meta.qml` más alakban adja a nevet) a végén, csak a szolgáltatással.

**Szín, kép:** `brightnessOpacity`→brightness · `fadeInBrightness`→brightness · `fadeOutBrightness`→brightness · `fadeInMovit`→movit.opacity · `fadeOutMovit`→movit.opacity · `contrast`→lift_gamma_gain · (színkerék)→lift_gamma_gain · `affineRotate`→affine · `cropRectangle`→qtcrop · `cropCircle`→qtcrop · (crop)→crop · `autoFadeVideo`→autofade · `gradientMap`→gradientmap · `gradient`→frei0r.cairogradient · `posterize`→frei0r.posterize · `halftone`→frei0r.colorhalftone · `dither`→frei0r.dither · `vertigo`→frei0r.vertigo · `noise_fast`→avfilter.noise · `noise_keyframes`→frei0r.rgbnoise · `blur_gaussian`/`blur_exponential`/`blur_lowpass`→frei0r.IIRblur · `blur_pad`→pillar_echo · `deband`→avfilter.deband · `vaguedenoiser`→avfilter.vaguedenoiser · `reflect`→mirror · `dynamicText`→dynamictext · `richText`/`typewriter`→qtext · `subtitles`→subtitle · `timer`→timer · `shake`→frei0r.camerashake · `scanlines`→frei0r.scanline0r · `spotRemover`→spot_remover · `elastic_scale`→frei0r.elastic_scale · `maskChromaKey`/`maskFromFile`/`maskGlaxnimate`/`maskSimpleShape`/`reframe`→mask_start · `alphaChannelAdjust`/`alphaChannelView`→frei0r.alpha0ps · `nosync`→frei0r.nosync0r · 360°-szűrők (`bigsh0t_*`)→frei0r.bigsh0t_*

**Hang (h):** `audioGain`→volume · `fadeInVolume`→volume · `fadeOutVolume`→volume · `muteVolume`→volume · `audioPan`→panner · `audioBalance`→panner · `audioChannelCopy`/`audioSwapChannels`→channelcopy · `autoFade`→autofade · `audioDeclick`→avfilter.adeclick · `audioSeam`→audioseam · `audioRnnoiseLink`→rnnoise · `stereoEnhance`→avfilter.haas · `15BandEq`→ladspa.1197 · `3BandEq`/`parametricEq`→ladspa.1204 · `ambisonic-decoder`/`-encoder` · (normalizálás)→dynamic_loudness / loudness · (mono)→mono · (hangmagasság)→rbpitch · további `ladspa.*` (kompresszor, zajkapu, reverb stb.) · `vst2.*`

**Név nélküli sorok (csak szolgáltatás):** avfilter.chromahold, fspp, hflip, hue, lut3d, random, smartblur, tmix, vflip, vibrance · charcoal · choppy · dropshadow · dust · frei0r.alphaspot, bluescreen0r, c0rners, cairoimagegrid, defish0r, distort0r, glitch0r, hqdn3d, keyspillm0pup, lenscorrection, levels, pixeliz0r, premultiply, rgbsplit0r, select0r, spillsupress · gpsgraphic · gpstext · grain · hslprimaries · hslrange · invert · lines · mask_apply · movit.diffusion · oldfilm · opencv.tracker · openfx.wtf.vala^NtscRs · outline · sepia · strobe · tcolor · threshold · timeremap · vidstab · wave · brightness · lift_gamma_gain

**Következtetés a leképezéshez:** a CFA4 megfelelője most is megvan a fényerőre, színkorrekcióra (Lumetri), vágásra és méretezésre (Crop, Transform), elmosásra (Gaussian Blur), invertálásra, hangerőre/fade-re/panorámára és a sebességre; a **frei0r/avfilter/ladspa/vst/opencv/openfx** csoport külső bővítmény, ezek a tervben „nem támogatott, jelentve” osztályba esnek (K5/K44 később nyithatja meg).
