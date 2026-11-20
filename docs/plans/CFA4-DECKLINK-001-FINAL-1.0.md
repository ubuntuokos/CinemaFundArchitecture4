# CFA4-DECKLINK-001 – Végrehajtási terv (FINAL 1.0, jóváhagyva)

> Forrás: ez a terv eredetileg online Claude-dokumentumként készült (https://claude.ai/artifact/DubGtWa6mpHkyx5GHT1zFU); a CFA4 szabálya szerint („minden tervnek a repóban a helye”) 2026-11-15-én átkerült ide, tartalma változatlan. Ettől kezdve ez a fájl az irányadó.

2026-11-14 · Horváth H. István

**Állapot: FINAL 1.0. István a tervet 2026-11-14-én jóváhagyta, a D1–D13 döntéseket a javaslatok szerint.** Fejlesztés nem indult, és ez a jóváhagyás sem indítja el: a DeckLink-fejlesztés (F0-tól) indításáról István külön dönt ([`docs/coordination.md`](https://github.com/ubuntuokos/CinemaFundArchitecture4/blob/main/docs/coordination.md) §2.10). Addig a futó CFA4-fejlesztések a 9. fejezet feltételei szerint készítik elő a DeckLink beilleszthetőségét.

**Véglegesítéskor javítva (2026-11-14):** a jóváhagyott D4 szabály („ami más projektben elkészül, az közös réteg”) szerint a D6 módosult. A vezérlőfelület-motor (CFA4-CONTROL-SURFACE-001, ADR 0012, elfogadva 2026-11-13) és a hardveres vezérlőfelület-projekt (CFA4-HW-CONTROL-SURFACES-001) már fut, ezért a terv nem épít saját `cfa4-control-io` crate-et: a TSL UMD, a tally, az OSC és a MIDI a vezérlőfelület-motor forrásaként kapcsolódik. Az F7 becslése emiatt 16–24 órával csökkent.

## 1. Összefoglaló és döntések

A CFA4-DECKLINK-001 megvalósítható, de csak egy új licencdöntés után. A DeckLink futtatókönyvtára (`libDeckLinkAPI.so`, a Desktop Video része) zárt kód, a `docs/licensing.md` 4. pontja pedig ma csak a BRAW-adapternek ad kivételt. A javaslat a BRAW-minta: GPL-3 7. pont szerinti kiegészítő engedély, külön folyamat, a Desktop Videót a felhasználó telepíti.

A terv a spec F0–F8 fázisait követi. Az F1-et három részre bontja, hogy hardver és licencdöntés nélkül is legyen beolvasztható eredmény (F1a). A DeckLink és az ST 2110 két önálló fejlesztési és integrációs terület (István döntése, 2026-11-14), közös médiapipeline-nal és közös vezérlési infrastruktúrával. Ez a terv csak a DeckLink-területet és a közös rétegeket tartalmazza; az F4 PTP-része, az F5 és az F6 az ST 2110-területé (LIVE-STUDIO-001 ST 2110 kiegészítés, T0–T7).

A teljes munka nagyságrendi becslése 674–1166 óra (az ST 2110-terület nélkül), ebből hardver nélkül végezhető kb. 250–350 óra (F0, F1a, a rétegek és a tesztkörnyezet).

**Döntések (jóváhagyva 2026-11-14, a javaslatok szerint; a D6 a véglegesítéskor pontosítva):**

| # | Kérdés | Döntés |
| --- | --- | --- |
| D1 | Kaphat-e a DeckLink futtatókönyvtára GPL-3 7. pont szerinti kivételt, mint a BRAW? | Igen, BRAW-mintával; nélküle a DeckLink kimarad, a terv IP-úton (ST 2110) folytatódik |
| D2 | Vékony C++-réteg vagy tisztán Rust vtable-kötés? | C++-réteg (`extern "C"`), csak a DeckLink-folyamatba fordítva; Rust-port később |
| D3 | Külön `cfa4-io-host` folyamat vagy a `cfa4-studio-host` része? | Külön folyamat; a stúdió megosztott memórián kapja a képet |
| D4 | Hogyan viszonyul a DeckLink-terv az ST 2110-höz és a más projektekben kész részekhez? | Eldöntve (István, 2026-11-14): a DeckLink és az ST 2110 két külön terület, közös médiapipeline-nal és vezérlési infrastruktúrával; az ST 2110-tervet nem kettőzzük meg; ami más projektben elkészült, az közös réteg (4. fejezet) |
| D5 | Egy közös rögzítő (`cfa4-ingest`) legyen-e a DeckLink- és az élő stúdiós (S5) felvételhez? | Igen, a spec „nincs párhuzamos rögzítőmotor” szabálya szerint |
| D6 | A „közös vezérlőmotor” a parancsregiszter és a vezérlőfelület-motor (ADR 0012); az F7 erre épül, saját vezérlő-crate nélkül? | Igen: a TSL UMD és a tally a vezérlőfelület-motor új forrása és visszajelzése, az OSC annak F-OSC modulja, a MIDI a cfa4-surface meglévő útja; új cfa4-control-io nem készül |
| D7 | Mikor jönnek a Qt6-panelek? | A Rust-oldal most; a Qt-panelek a Qt6 befejezése után (LIVE D7 mintája); addig vezérlőcsatorna és MCP |
| D8 | Melyik DeckLink-kártya kerüljön a referenciagépbe? | Egy kétirányú 12G-SDI-s, referencia-bemenetes, 12 bites RGB-t és HDR-metaadatot tudó kártya (8K Pro osztály), mellé egy olcsó egybemenetes a többeszközös teszthez |
| D9 | PostgreSQL a DeckLink-részben? | Nem: preferencia és projekt; a BacklotHub csak „új felvétel” eseményt kap |
| D10 | Kikerül-e a DNG az F2-ből? | Igen, a PROCODEC-002 11. munkacsomagjába tartozik |
| D11 | Szigorú fázislánc vagy függőségi gráf? | Gráf (6. fejezet); az elfogadási feltételek változatlanok |
| D12 | Pótolod-e a spec csonka F8-részét („Telepítési együtt…”)? | A terv addig a 2.3 pont együttélési listájával egészíti ki |
| D13 | A Blackmagic IP-kártyák (DeckLink SDK-n át ST 2110-et adó hardver) melyik területhez tartoznak? | A DeckLink-területhez mint eszköz (SDK-kötés), az ST 2110-megfelelőség mérése az ST 2110-területé; addig nem készül rá feladat |

## 2. Kiinduló helyzet a CFA4-ben

A terv alapjainak egy része megvan, egy része csak a specben létezik. Az alábbi állapot a `main` 4be66a6e commitján mérve, a 2026-11-14-i állapot.

| Terület | Amit a spec feltételez | Ami a `main`-en van | Következmény |
| --- | --- | --- | --- |
| Képkocka-modell | közös pipeline videóval, hanggal, színnel, HDR-rel | `cfa4-frame`: `VideoFrame` (`Yuv8`, `Yuv16` 9–16 bit, `Rgb16`, `Rgba8`, `RgbaF32`; `ColorInfo`; mezőrend; `pts: Tick`), `AudioBuffer` síkonkénti `f32` | a 10/12 bites kép veszteségmentesen átfér; **hiányzik** az ANC, az időkód, a képkockánkénti HDR-metaadat, a hardveres időbélyeg és az elejtésjelző |
| Médiaforrás | a backend átadja a pipeline-nak | `cfa4-media::MediaSource` húzó modell: `video_frame(time)`, `audio(start, frames)` | élő, toló forrásra nincs trait: **új élőforrás-réteg kell** |
| Kódolás | „FFmpeg-alapú kódolási útvonal” | `cfa4-export`: saját kódolók (ProRes, DNxHR, H.264, HEVC, AV1, MPEG-2, APV) és muxerek (MOV/MP4, MXF OP1a/OP-Atom, WebM, MPEG-TS, DCP, IMF); a `VideoEncoder` `EncoderFrame { rgba, hdr: Option<&[f32]> }`-t kap; az FFmpeg a `cfa4-route` mögötti tartalék | a rögzítés a saját kódolókra épül; **a YUV-kép ma RGBA-n át kódolódik**, élő rögzítéshez közvetlen YUV-bemenet kell |
| „CFA4 Recording” | létező rögzítőrendszer és terv | nincs ilyen terv; van: hangalámondás WAV-ba (`engine::voiceover`), növekvő fájlok frissítése (`media_watch`), az élő stúdió tervezett S5 felvétele (libobs) | a rögzítő ebben a tervben készül (F1a) |
| FFmpeg DeckLink-eszköze | — | a `decklink` az FFmpeg `configure` nonfree listáján van, a CFA4 szabálya: soha `--enable-nonfree` | az FFmpeg DeckLink-útja kizárt |
| GStreamer | kiegészítő interoperabilitás | `cfa4-gstreamer` (L5, opcionális); az Ubuntu 26.04 `gstreamer1.0-plugins-bad` csomagja tartalmazza a `libgstdecklink.so`-t | tesztorákulumnak kiváló, termékútként nem kell |
| Élő stúdió | — | `cfa4-obs` S0 kész: `StudioBackend` trait, `cfa4-studio-host` külön folyamat felügyelettel, képkocka-gyűrű `unsafe` nélkül | ez a minta a DeckLink-folyamathoz |
| ST 2110, PTP, NMOS | F4–F6 új munka | `cfa4-st2110` (L0) T0 kész; T1–T7 jóváhagyva; PTP: a rendszer `linuxptp`-je, a CFA4 csak olvas (D9); NMOS IS-04/05 a T7-ben (D11); -22 csak VC-2 (D10) | az F4 PTP-része = T1, F5 = T2–T6, F6 = T7 |
| Időkód | timecode | `cfa4-time` (`Tick`, NDF/DF), `cfa4-tcsync` (LTC, MTC, chase), `cfa4-tcsync-io` (cpal, ALSA szekvenszer) | a DeckLink RP188/VITC és a genlock ugyanabba a szinkronállapotba kerül |
| Feliratok | caption adatok | `cfa4-captions`: CEA-608, MCC, SCC, TTML, STL | az ST 334 bontás után ide kerül, az MCC az oldalfájl |
| Vezérlés | „meglévő Control Surface architektúra” | nincs (`docs/hardware-parity.md`: 0%); van: parancsregiszter, vezérlőcsatorna, MCP, MIDI csak az MTC-hez | a közös vezérlőmotor a parancsregiszter (D6) |
| Adatkezelés | PostgreSQL: profilok, konfiguráció, metaadat | a szerkesztőben nincs adatbázis; a PostgreSQL a BacklotHub és a `cfa4_files` adattára (ADR 0006, 0009) | profil a preferenciákba, előbeállítás a projektbe (D9) |
| Qt6 | eszköz-, jel-, profilfelületek | `python/cfa4_qt/`, a motor `cfa4-cli serve`; a migráció folyamatban | Qt-panelek a Qt6 befejezése után (D7) |
| Hardver | valós DeckLink | a referenciagépen nincs DeckLink; 2,5 GbE; két NUMA-csomópont; kernel 7.0 | hardveres elfogadás csak a kártya beszerzése után (D8) |
| Gyártói SDK | aktuális SDK | a közösségi kötések már a Desktop Video 16.0 SDK-ra épülnek (pydecklink) | a célverzió: 16.x, a pontos verziót az F0 rögzíti |

## 3. Ellentmondások a spec és a CFA4 között

A spec és a CFA4 három ponton akadályozóan ütközik: a licencben (E1), az ST 2110-terv megkettőzésében (E2) és a rögzítőmotor kérdésében (E3). A többi pont csak pontosítás. A `VISION.md` szerint az ellentmondást jelezni kell, ezért egyiket sem oldja fel a terv csöndben; mindegyikhez döntés tartozik.

| # | Téma | A spec | A CFA4 szabálya vagy állapota | Javaslat | Döntés |
| --- | --- | --- | --- | --- | --- |
| E1 | **Licenc (blokkoló)** | a Blackmagic SDK a kötelező alap | `licensing.md` 4.: zárt kód sem a folyamatban, sem külön; egyetlen kivétel a BRAW. LIVE D4: zárt SDK-t igénylő protokoll (NDI) nincs. Az SDK fejlécei megengedőek, de a Blackmagictől letöltött példányra a DeckLink SDK EULA vonatkozik; a futtatókönyvtár zárt | BRAW-minta: GPL-3 7. pont szerinti kivétel, külön folyamat, a Desktop Videót a felhasználó telepíti, a CFA4 nem szállítja; kiadás előtt jogi átnézés | D1 |
| E2 | **Átfedés az ST 2110 kiegészítéssel** | F4 PTP, F5 ST 2110, F6 NMOS saját fázisként | jóváhagyva: T0–T7, D8–D12; a `cfa4-st2110` T0 kész | két külön terület: az ST 2110 a T-fázisoké, a DeckLink-terv nem tartalmaz ST 2110-feladatot; a találkozási pont a közös pipeline és vezérlés; átfedési bejegyzés a `coordination-log.md`-be | D4 |
| E3 | **Párhuzamos rögzítőmotor** | „nem hozhat létre párhuzamos rögzítőmotort” | ma nincs általános rögzítő; az élő stúdió S5-je a libobs-szal rögzítene; az FFmpeg és a GStreamer nem lehet fő rögzítő (ADR 0005) | egy rögzítő: `cfa4-ingest`, saját kódolókkal és muxerekkel, a `cfa4-route` választásával; az S5 is ezt használja | D5 |
| E4 | A spec „meglévő Control Surface architektúrát” feltételez | illeszkedjen a meglévőhöz | a main-en még nincs, de készül: CFA4-CONTROL-SURFACE-001 (ADR 0012, #1184, CS-1…CS-5) és CFA4-HW-CONTROL-SURFACES-001 (#1194, #1195, #1287) | nincs saját crate: a TSL, a tally, az OSC és a MIDI a vezérlőfelület-motor `SurfaceEvent`/`SurfaceFeedback` modelljén kapcsolódik (9. fejezet, V4) | D6 |
| E5 | PostgreSQL mint konfigurációs tár | profilok, konfiguráció PostgreSQL-ben | a szerkesztő offline, adatbázis nélkül indul | profil: preferencia; előbeállítás: projekt; metaadat: fájl és oldalfájl; BacklotHub: esemény | D9 |
| E6 | „C++ FFI” | Rust–C++ FFI | Rust first; `unsafe` csak a kötés moduljában (ADR 0001, 2026-11-07) | vékony C++-réteg csak a DeckLink-folyamatban | D2 |
| E7 | A DeckLink kódjának helye | a Rust backend része | never-crash; libobs-precedens: külön folyamat (LIVE D2) | `cfa4-io-host` külön folyamat | D3 |
| E8 | GStreamer szerepe | interoperabilitás | a GStreamer DeckLink-pluginja ugyanúgy zárt könyvtárat tölt be | termékútként nem; tesztorákulum a hardveres gépen | — |
| E9 | Szigorú fáziskapuk | F(n+1) csak F(n) után | az F2 és F3 független, a T-fázisok hardver nélkül haladnak | függőségi gráf, változatlan elfogadási feltételekkel | D11 |
| E10 | DNG az F2-ben | DNG-alapú RAW-munkafolyamatok | a DeckLink nem ad szenzoradatot; a DNG a PROCODEC-002 része | kikerül; az EXR marad | D10 |
| E11 | Key/Fill és többkimenet | F7 | a kompozitálás az élő stúdióban (S4) van | Key/Fill az S4 után; a kártya belső keyere önállóan is mehet | — |
| E12 | Hardveres teszt | valós hardver, kétórás teszt | nincs kártya, a CI-ben nincs hardver | mock és visszajátszó eszköz a CI-ben; hurokteszt a referenciagépen | D8 |
| E13 | A spec csonka | F8 „Telepítési együtt…” | — | pótlásig a 2.3 pont listája | D12 |

## 4. Célarchitektúra: két terület, közös rétegek

A DeckLink és az ST 2110 két önálló terület, amely két közös rétegen találkozik: a médiapipeline-on (eszközfüggetlen HAL, rögzítő, lejátszó) és a vezérlési infrastruktúrán (parancsregiszter, események, felvételfelügyelet). Ami más CFA4-projektben már elkészült, az közös réteg, és egyik terület sem építi újra.

### 4.1 Területek és felelősségek

| Terület | Felelősség | Nem felelőssége |
| --- | --- | --- |
| **DeckLink** (ez a terv) | SDI/HDMI hardver: SDK-kötés, `cfa4-io-host`, felderítés, be- és kimenet, kártyaidőkód, genlock, kártyás ANC és HDR-metaadat, keyer | ST 2110, PTP, NMOS; fájlírás; kódolás; projekt |
| **ST 2110** (LIVE-STUDIO-001 ST 2110 kiegészítés) | RTP/SDP, -20/-30/-40 stb., PTP (linuxptp olvasása), forgalomformálás, NMOS IS-04/05 | SDI-hardver; fájlírás; kódolás; projekt |
| **Közös médiapipeline** | HAL (`cfa4-capture`), ANC-kód (`cfa4-anc`), rögzítő (`cfa4-ingest`), lejátszó/kimeneti ütemező, képkocka-gyűrű, metaadat-megőrzés | gyártó- vagy protokollspecifikus kód |
| **Közös vezérlési infrastruktúra** | `io.*` parancsok, eszköz- és jelállapot, események, folyamatfelügyelet (`cfa4-io`), szinkronállapot-nézet; a TSL, a tally, az OSC és a MIDI a vezérlőfelület-motoron át (ADR 0012) | eszközspecifikus logika |

### 4.2 Közös rétegként átvett, már létező elemek

| Elem | Honnan | Mire használja a DeckLink-terület |
| --- | --- | --- |
| `cfa4-time` (`Tick`, időkód) | alap | minden időbélyeg, RP188 |
| `cfa4-frame`, `cfa4-color` | alap | képadat, színtér, HDR-görbék |
| `cfa4-export` kódolók és muxerek, `cfa4-mxf`, `cfa4-isobmff` | PROCODEC, export | rögzítés (YUV-bemenettel kiegészítve, F1a.6) |
| `cfa4-route` | ADR 0004 | kódolóválasztás, FFmpeg csak tartalék |
| `cfa4-captions` | feliratok | ST 334 → feliratsáv, MCC |
| `cfa4-tcsync`, `cfa4-tcsync-io` | TIMECODE-SYNC | LTC/MTC a szinkronállapotban |
| `cfa4-obs` gyűrű és host-felügyelet mintája | LIVE-STUDIO S0 | `cfa4-io-host` és gyűrű v2 |
| `cfa4-st2110`, linuxptp, NMOS (T0–T7) | ST 2110-terület | csak a közös HAL-on keresztül, közvetlenül nem |
| `media_watch`, projektimport, parancsregiszter, vezérlőcsatorna, MCP | motor | felvétel a projektbe, ügynöki vezérlés |

### 4.3 Új crate-ek és rétegek

| Crate / alkalmazás | Réteg | Terület | Feladat |
| --- | --- | --- | --- |
| `cfa4-anc` | L0 alap vagy L1 (F0.5 dönti) | közös | ST 291 csomag, ST 334, ST 2016-3, ST 12-2, ST 2038; az ST 2110-40 is ezt használja |
| `cfa4-capture` | L1 | közös | HAL típusok és traitek, pixelcsomagolók, gyűrű v2, mock és visszajátszó szolgáltató |
| `cfa4-ingest` | L3 | közös | rögzítő, veszteségnapló, metaadat-jelentés |
| `cfa4-io` | L5 | közös vezérlés | szolgáltató-folyamatok felügyelete, gyűrűolvasó |
| `cfa4-control-io` (elhagyva) | L5 | közös vezérlés | helyette a vezérlőfelület-motor (`cfa4-surface-model` L1, `crates/engine/src/surface/` L4, `cfa4-surface` L5) új forrásai |
| `cfa4-decklink` | L5 | DeckLink | SDK-kötés (C++-réteg, `unsafe` csak az FFI-modulban) |
| `cfa4-io-host` | L6 | DeckLink | a kártyát kezelő külön folyamat |
| `apps/cfa4-cli/src/serve_io.rs` | L6 | közös vezérlés | `io.*` parancsok |

### 4.4 Ami az ST 2110-ből ma megvan (ellenőrizve 2026-11-14)

Az ST 2110-ből a T0 van a `main`-en, a T1 egy nyitott PR-ban; a T2–T7 még nem kezdődött el. A DeckLink-terv ezek közül egyiket sem építi újra; ahol közös réteg kell, ott az egyszer készül el, és mindkét terület azt használja.

| ST 2110 elem | Állapot | Hol | Teendő a duplázás elkerülésére |
| --- | --- | --- | --- |
| T0: RTP-fejléc, SDP (`ts-refclk`, `mediaclk`, `fmtp`), -20 GPM/BPM csomagolás és bontás, fuzz, GStreamer-orákulum | kész a `main`-en (#1002) | `crates/st2110` (L0, kb. 1900 sor) | nincs; a DeckLink-terület nem nyúl hozzá |
| T1: SMPTE Epoch médiaóra, képkocka-időbélyeg, PTP-állapot (`pmc`-olvasás), SDP-órák | nyitott PR (#1128, `cfa4/live-studio-st2110-t1`) | `crates/st2110/src/clock.rs`, `crates/obs/src/ptp.rs` | **a PTP-állapot olvasása ma a stúdióhostban (`cfa4-obs`) van**; a közös szinkronállapot-nézet (F4.2) ezt használja, nem ír sajátot. Javaslat: a #1128 beolvasztása után a `ptp.rs` a közös vezérlési rétegbe kerül (`cfa4-io` vagy külön crate), a stúdióhost onnan hívja; ez a T1 gazdájával egyeztetendő (koordinátor) |
| T2: AES67 hang (-30) | nem kezdődött el | — | a hang a közös HAL `PcmBlock`-ját használja |
| T3: képbemenet (A típusú vevő) | nem kezdődött el | — | a vevő a közös HAL `DeviceProvider`-ként épül, így a rögzítő és az `io.*` parancsok kód nélkül kezelik |
| T4: programkimenet (W típusú küldő) | nem kezdődött el | — | a közös `PlayoutStream` mintájára |
| T5: ANC (-40), TTML (-43), -41, AM824 (-31) | nem kezdődött el | — | **az ANC-kód egyszer készül**: `cfa4-anc` (F3.1), a T5 csak az RFC 8331 csomagolást teszi hozzá |
| T6: -22 VC-2 | nem kezdődött el | — | nem érinti a DeckLinket |
| T7: NMOS IS-04/05, felület | nem kezdődött el | — | a DeckLink-portok később a T7 NMOS-ében jelenhetnek meg; saját NMOS-kód a DeckLink-területen nem készül |
| Nyitott kérdés: ST 2110 és JPEG XS (RFC 9134) | nyitott issue (#989) | — | nem érinti a DeckLink-tervet |

Az átfedés-ellenőrzés minden DeckLink-fázis indulásakor újra lefut (F0.6 állandó lépésként): ha közben egy közös elem más projektben elkészült, a fázis azt veszi át.

Az alábbi ábra az adatutat mutatja: a DeckLink- és az ST 2110-szolgáltató ugyanazon a HAL-on és rögzítőn keresztül ér a projekthez.

*Ábra: architektúra · két terület, két közös réteg*

```text
Két terület, egy közös pipeline és egy közös vezérlés

+-----------------------------------------+   +-----------------------------------------+
| DeckLink-terület (ez a terv)            |   | ST 2110-terület (LIVE kiegészítés)      |
| cfa4-io-host + cfa4-decklink (C++-réteg)|   | cfa4-st2110: T0 kész, T1 nyitott PR     |
| libDeckLinkAPI.so: a felhasználó        |   |   (#1128)                               |
|   telepíti                              |   | T2–T7: hang, kép, ANC, VC-2, NMOS       |
| SDI/HDMI, genlock, RP188, ANC, HDR,     |   | PTP: linuxptp, a CFA4 csak olvassa      |
|   keyer                                 |   |                                         |
+-----------------------------------------+   +-----------------------------------------+
                    | DeviceProvider                              | DeviceProvider
                    v                                             v
+-----------------------------------------------------------------------------------------+
| Közös médiapipeline                                                                     |
|  [cfa4-capture: HAL, gyűrű, mock eszköz] -> [cfa4-ingest: rögzítő, veszteségnapló]      |
|  cfa4-anc (ST 291/334/2016-3/12-2/2038) · natív kódolók és muxerek                      |
|  File Services (files.stage.*) → projekt / idővonal                                     |
+-----------------------------------------------------------------------------------------+
                    ^  vezérel és állapotot kap
                    v
+-----------------------------------------------------------------------------------------+
| Közös vezérlési infrastruktúra                                                          |
| io.* parancsok, cfa4-io folyamatfelügyelet, szinkronállapot (genlock, PTP, LTC/MTC,     |
|   RP188)                                                                                |
| vezérlőfelület-motor (ADR 0012: TSL, tally, OSC, MIDI) · cfa4-tcsync (időkód)           |
| · PTP-állapot (cfa4-io-core)                                                            |
+-----------------------------------------------------------------------------------------+
                    ^
                    v
            [Qt6 felület, MCP, ügynökök]
```

A két terület csak `DeviceProvider`-ként csatlakozik: fájlt, kódolást és projektet egyik sem kezel, ez a közös pipelineé.

## 5. API-szerződések és FFI-határ

A szerződés lényege: a hardveres szolgáltató csak ad és vesz (kép, hang, időkód, ANC, HDR, órák, események), minden puffer, kódolás, fájl és projekt a `cfa4-ingest`-é és a motoré (a spec 3.3 pontja). Az alábbi vázlatot az F0.5 véglegesíti; minden publikus függvény `Result`-ot ad, pánik nincs (`AGENTS.md` §0).

### 5.1 Eszközfüggetlen HAL (`cfa4-capture`, L1)

```rust
pub struct DeviceId { pub provider: ProviderKind, pub persistent: String }
pub enum ProviderKind { DeckLink, St2110, Mock, Replay }

/// Csak az szerepel, amit a hardver, a gyártói API ÉS a CFA4-út is tud (spec 5.4).
pub struct Support { pub hardware: bool, pub api: bool, pub pipeline: bool }

pub struct Capabilities {
    pub inputs: Vec<Connector>, pub outputs: Vec<Connector>,
    pub modes: Vec<ModeSupport>,      // VideoMode × irány × PixelFormat × Support
    pub audio: AudioCaps,             // csatornaszám, 16/32 bites egész, 48 kHz
    pub anc: AncCaps, pub hdr: HdrCaps, pub sync: SyncCaps,
    pub keyer: Option<KeyerCaps>, pub duplex: DuplexCaps,
}

pub enum PixelFormat { Yuv8_2vuy, Yuv10_v210, Rgb10_r210, Rgb10_r10l, Rgb12_r12l, Bgra8, Argb8 }

pub struct CapturedFrame {
    pub seq: u64,                              // rés = elejtés
    pub mode: VideoMode,
    pub video: Option<cfa4_frame::VideoFrame>, // None: jel nélküli képkocka
    pub audio: Option<PcmBlock>,               // egész PCM, bitátlátszó (ST 337 is)
    pub timecode: Option<FrameTimecode>,       // RP188 LTC/VITC1/VITC2, VITC; forrás jelölve
    pub anc: Vec<cfa4_anc::AncPacket>,
    pub hdr: Option<HdrMetadata>,              // EOTF, ST 2086, MaxCLL, MaxFALL
    pub clocks: FrameClocks,
    pub flags: FrameFlags,                     // NoSignal, FormatChanged, Dropped(n), Late
}

pub struct PcmBlock { pub sample_rate: u32, pub channels: u16, pub bits: u8, pub interleaved: Vec<i32> }

pub struct FrameClocks {
    pub stream: cfa4_time::Tick,       // a DeckLink stream time-ja közvetlenül Tick-időskálán kérve
    pub hw_reference_ns: Option<u64>,  // hardveres érkezési idő
    pub host_monotonic_ns: u64,
    pub tai_ns: Option<u64>,           // ha van PTP (ST 2110 T1)
}

pub trait DeviceProvider: Send {
    fn kind(&self) -> ProviderKind;
    fn available(&self) -> Availability; // NotInstalled { hint } | Ready { api_version } | Failed { reason }
    fn devices(&mut self) -> Result<Vec<DeviceInfo>>;
    fn open_input(&mut self, id: &DeviceId, cfg: &InputConfig) -> Result<Box<dyn CaptureStream>>;
    fn open_output(&mut self, id: &DeviceId, cfg: &OutputConfig) -> Result<Box<dyn PlayoutStream>>;
    fn poll_events(&mut self) -> Vec<DeviceEvent>;
}

pub trait CaptureStream: Send {
    fn start(&mut self) -> Result<()>;
    fn next(&mut self, timeout: std::time::Duration) -> Result<Option<CapturedFrame>>;
    fn stats(&self) -> CaptureStats;   // érkezett, elejtett, késett, puffermélység
    fn stop(&mut self) -> Result<()>;
}

pub trait PlayoutStream: Send {
    fn preroll(&mut self, frames: &[OutFrame]) -> Result<()>;
    fn schedule(&mut self, frame: OutFrame) -> Result<()>; // kép + PCM + TC + ANC + HDR, Tick-időponttal
    fn start_at(&mut self, at: cfa4_time::Tick) -> Result<()>;
    fn stats(&self) -> PlayoutStats;   // ütemezett, késve kiadott, elejtett, alulcsordulás
    fn stop(&mut self) -> Result<()>;
}
```

A 32 bites egész PCM szándékos: az `f32` a 24 bites hangot pontosan viszi, de az AES3-ban átvitt nem-PCM tartalmat (ST 337, pl. Dolby E) csak az egész út hagyja bitazonosan. A stream time-ot a DeckLink tetszőleges időskálán adja, ezért közvetlenül `Tick`-ben (254 016 000 000/s) kérjük, átváltási hiba nélkül.

### 5.2 Rögzítő (`cfa4-ingest`, L3)

```rust
pub struct RecordJob { pub target: RecordTarget, pub video: VideoCodecChoice, pub audio: AudioChoice,
                       pub segment: Option<Segmenting>, pub metadata: MetadataPolicy }
pub enum VideoCodecChoice { UncompressedV210, UncompressedRgb12, ProRes(ProResProfile),
                            DnxHr(DnxProfile), Ffv1, Routed(cfa4_route::Task) }
pub enum OnUnsupported { Warn, Sidecar, Refuse }
pub trait Recorder {
    fn push(&mut self, f: &CapturedFrame) -> Result<RecordStatus>;
    fn finish(self) -> Result<RecordReport>;
}
pub struct RecordReport { pub frames: u64, pub audio_samples: u64, pub drops: Vec<DropRecord>,
                          pub preserved: Vec<MetaKind>, pub lost: Vec<(MetaKind, String)> }
```

A `RecordReport.lost` teljesíti a spec 3.2 pontját: ha a célformátum nem visz egy metaadatot, a jelentés és a felület megnevezi, oldalfájlba menti (`Sidecar`), vagy a rögzítés el sem indul (`Refuse`).

### 5.3 Parancsok (minden parancs, ügynökkel vezérelhető)

| Parancs | Feladat | Fázis |
| --- | --- | --- |
| `io.providers` | telepítve van-e a Desktop Video, API-verzió, mock/ST 2110 szolgáltatók | F1a |
| `io.devices.list`, `io.device.status` | eszközök, képességek, jelállapot | F1a (mock), F1b |
| `io.input.open` / `close` | bemenet megnyitása móddal és pixelformátummal | F1a, F1b |
| `io.capture.start` / `stop` | rögzítési feladat haladással, megszakítással, jelentéssel | F1a |
| `io.output.open` / `close` | kimenet megnyitása | F1c |
| `io.monitor.set` | a Program (Source) monitor képe a külső kimenetre | F1c |
| `io.sync.status` | genlock, PTP, LTC/MTC, RP188 külön-külön | F4 |

### 5.4 FFI-határ és hibahatár

1. **Betöltés:** a `libDeckLinkAPI.so` futásidőben töltődik be (`dlopen`), a fordítás és az indulás nem függ tőle. Hiányában `Availability::NotInstalled` telepítési útmutatóval, nem hiba.
2. **C++-réteg:** `crates/decklink/shim/`, csak `extern "C"` függvények és POD-struktúrák. A COM-szerű referenciaszámlálás és a visszahívások (`IDeckLinkInputCallback`, `IDeckLinkVideoOutputCallback`, `IDeckLinkAudioOutputCallback`, `IDeckLinkDeviceNotificationCallback`) itt vannak.
3. **Nincs szinkron visszahívás Rustba:** a C++ oldal egy korlátos sorba tesz, a Rust-szál olvas. Így a C++-kivétel és a Rust-pánik nem keresztezi a határt.
4. **`unsafe`:** csak a `cfa4-decklink::ffi` modulban, `// SAFETY:` megjegyzéssel, ABI-tesztekkel (méretek, eltolások), mint a `vaapi` és a `libobs` kötésnél (ADR 0001).
5. **Fejlécek:** a vendorálás csak a D1 és az EULA-átnézés után; addig a `CFA4_DECKLINK_SDK_DIR` mutat a helyi SDK-ra, és a `decklink` feature nélkül a workspace fejlécek nélkül is fordul.
6. **Folyamathatár:** a `cfa4-io-host` összeomlása a motorban esemény (`SignalLost`/`Error`), a felügyelő újraindítja. A felvétel a motor folyamatában fut, így a fájl az utolsó teljes képkockáig szabályosan lezárul, a folytatás új szegmensbe megy.
7. **Valós idő:** a gyűrű rekeszei előre lefoglaltak, a host szálai a kártya NUMA-csomópontjához rögzíthetők. A túlcsordulás számlált elejtés; a DeckLink-visszahívás soha nem blokkol.

## 6. Fázisok feladatszinten

Minden fázis végén beölvasztható, zöld állapot áll: `cargo xtask ci`, a workspace-tesztek, Python-ellenőrzések, never-crash tesztek (hibás bemenet `catch_unwind` alatt). Minden feladat külön commit a saját azonosítójával (`DL-F1a.4: …`). Az óraszámok nagyságrendi becslések, a hardveres méréshez szükséges emberi jelenlétet (kábelezés, jelgenerátor) nem tartalmazzák.

### F0 – Előkészítés és döntések (hardver nélkül, 30–50 óra)

| ID | Feladat | Eredmény | Óra |
| --- | --- | --- | --- |
| F0.1 | A D1–D12 döntések rögzítése; a D1-től függően a `licensing.md` 4. pontjának és a regiszternek a módosítása | jóváhagyott terv, licencmódosítás | 4–6 |
| F0.2 | Desktop Video és SDK: célverzió (16.x), EULA (fejlécek terjesztése, futtatókönyvtár), Ubuntu 26.04 / kernel 7.0 DKMS-kompatibilitás, Flatpak-korlátok | `docs/decklink.md` „Licenc és telepítés” | 6–8 |
| F0.3 | Donortriázs véglegesítése (7. fejezet): rögzített commit, fájlonkénti licencfejlécek, a felhasználható részek | `docs/decklink-donors.md`, 26 sor | 8–12 |
| F0.4 | ADR 0012: DeckLink I/O (folyamatmodell, FFI, licenckivétel, betöltés, gyűrű) | `docs/adr/ADR-0012-decklink-io.md` | 4–6 |
| F0.5 | API-szerződés véglegesítése (5. fejezet), a `cfa4-capture` és `cfa4-anc` üres váza, felvétel a `cargo xtask layers` táblájába; a `cfa4-anc` réteghelye (alapcrate vagy L1) | fordítható váz, zöld rétegellenőrzés | 4–8 |
| F0.6 | Átfedés-ellenőrzés a többi projekttel (ST 2110, LIVE-STUDIO, PROCODEC, TIMECODE-SYNC): mi kész, mi van nyitott PR-ban; minden fázis indulásakor megismételve (4.4) | sor a `docs/coordination-log.md`-ben | 1–2 |
| F0.7 | Hardveres tesztkörnyezet terve: hurokteszt, jelgenerátor, referencia-szinkronjel, NUMA-elhelyezés | `docs/decklink.md` „Tesztkörnyezet” | 3–8 |

**Elfogadás:** a spec F0-feltételei; ezen felül az ADR elfogadva és a D1 eldöntve. Ha a D1 „nem”, a terv az F1a-val és az IP-úton folytatódik, DeckLink nélkül.

### F1a – HAL, mock eszköz, natív rögzítő (hardver nélkül, CI-ben; 100–160 óra)

| ID | Feladat | Eredmény | Óra |
| --- | --- | --- | --- |
| F1a.1 | `cfa4-capture` típusai és traitjei | crate, egységtesztek | 10–16 |
| F1a.2 | Pixelcsomagolók: v210, r210, R10L, R12L, 2vuy, BGRA/ARGB ⇄ `Yuv16`/`Rgb16`/`Yuv8` | bitazonos oda-vissza tesztek, fuzz | 12–18 |
| F1a.3 | I/O-gyűrű v2 (kép + hang + oldaladat rekeszenként, a `cfa4-obs::ring` mintájára) és vezérlőprotokoll (JSON-sorok Unix-socketen) | gyűrű, protokoll, szakadtképkocka-teszt | 12–20 |
| F1a.4 | Mock szolgáltató: tesztábra, futó RP188, 1–16 csatorna 24 bites PCM (29,97 Hz-en 1602/1601-es ütem), ANC, beállítható elejtés és formátumváltás | determinisztikus jelforrás a CI-hez | 8–12 |
| F1a.5 | Fájlból visszajátszó szolgáltató (rögzített hardveres jel visszajátszása) | regressziós tesztforrás | 6–10 |
| F1a.6 | Közvetlen YUV-bemenet a ProRes- és DNxHR-kódolóba (RGBA-kerülő nélkül) | bitazonos a mai út 8 bites eredményével, 10 bitnél nincs kerekítés | 14–20 |
| F1a.7 | `cfa4-ingest`: puffer, tömörítetlen v210 MOV-ba és MXF-be, többcsatornás egész PCM, időkódsáv, növekvő fájl, szegmentálás | rögzítő crate | 20–30 |
| F1a.8 | Veszteségnapló és metaadat-megőrzési jelentés (`RecordReport`) | jelentés a fájl mellett és a feladatban | 6–10 |
| F1a.9 | `io.*` parancsok a mock eszközzel (`apps/cfa4-cli/src/serve_io.rs`), hostile-paraméter tesztek | parancsok a vezérlőcsatornán és MCP-n | 8–12 |
| F1a.10 | Felvétel a projektbe: import, klipmetaadat (időkód, forráseszköz, jelentés) | egy mozdulattal az idővonalra | 4–8 |

**Elfogadás:**

1. Minden csomagoló bitazonos oda-vissza minden szélességre (a v210 6 pixeles csoportja, páratlan szélesség, sorvégi kitöltés); mutációs fuzz a gyűrűn és a csomagolókon.
2. 10 perc mock 1080p50 v210 + 16 csatorna + RP188 → MOV és MXF. Visszaolvasva a CFA4 `MediaSource`-szal és az ffprobe-orákulummal: képkockaszám, kezdő időkód, hangminták száma (= képkockák × 960), képkockánkénti hash bitazonos.
3. Elejtés-injektálás (1, 10, 100 képkocka): a jelentés pontosan ennyit és ott jelez; a fájl lejátszható, az idővonal-pozíció nem csúszik.
4. Formátumváltás felvétel közben: új szegmens, a régi fájl lezárva.
5. A motor megölése felvétel közben: a fájl újranyitható, legfeljebb az utolsó szegmens hiányzik (dokumentált határ).

### F1b – DeckLink-bemenet (a D1 után; 120–200 óra)

| ID | Feladat | Eredmény | Óra |
| --- | --- | --- | --- |
| F1b.1 | C++-réteg: betöltés (`dlopen`), iterátor, attribútumok, profilok, `extern "C"` felület, ABI-tesztek | `crates/decklink/shim/` | 24–36 |
| F1b.2 | `cfa4-decklink` Rust-kötés, `DeviceProvider` megvalósítása, `Availability` | crate (`decklink` feature) | 16–24 |
| F1b.3 | `cfa4-io-host` alkalmazás és `cfa4-io` felügyelő (indítás, újraindítás, állapot) | külön folyamat | 16–24 |
| F1b.4 | Felderítés és hot-plug (`IDeckLinkDiscovery`), képességek (`DoesSupportVideoMode`), több kártya | `io.devices.list` valós eszközzel | 12–18 |
| F1b.5 | Bemenet 8/10 bites YUV-ben, automatikus formátumfelismeréssel | képkockák a gyűrűben | 16–24 |
| F1b.6 | Hang 2–16 (a kártya szerint 32/64) csatorna 32 bites egészként | `PcmBlock` | 8–12 |
| F1b.7 | Időkód (RP188 VITC1/VITC2/LTC, VITC), stream time `Tick`-időskálán, hardveres érkezési idő | `FrameClocks`, `FrameTimecode` | 8–12 |
| F1b.8 | Események: jelvesztés, formátumváltás, elejtés, foglalt eszköz | `DeviceEvent` | 6–10 |
| F1b.9 | Hardveres teszthám: hurokteszt-szkript, mérési jegyzőkönyv-sablon, GStreamer-orákulum (`decklinkvideosrc`) | `scripts/dev/decklink-loop-test.sh` | 14–30 |

**Elfogadás (referenciagép, kártyával):**

1. **Hurokteszt:** a kártya saját kimenete a bemenetére kötve, ismert tesztábra + sorszám a képben és ANC-ban, ismert hang (csatornánként más frekvencia és impulzus). A rögzített adat bitazonos, a hang–kép eltolás mérve és állandó, az időkód folytonos.
2. Kábelkihúzás, rossz formátum, a host megölése: esemény, nincs összeomlás, a felvétel lezárul.
3. 2 órás rögzítés 1080p50 10 bit + 16 csatorna, majd 2160p50 10 bit (ha a kártya és a lemez bírja): 0 megmagyarázatlan elejtés; CPU-terhelés és a szerkesztő lejátszása mérve („nem lassít” szabály).
4. Összevetés a GStreamer `decklinkvideosrc`-vel ugyanazon a jelen.

### F1c – DeckLink-kimenet és külső monitor (80–140 óra)

| ID | Feladat | Eredmény | Óra |
| --- | --- | --- | --- |
| F1c.1 | Ütemezett kimenet (`ScheduleVideoFrame`, előtöltés), hangütemezés | `PlayoutStream` | 24–40 |
| F1c.2 | Kimeneti időkód és ANC | RP188 és ANC a kimeneten | 8–14 |
| F1c.3 | Alulcsordulás és késés jelzése, számlálók | `PlayoutStats`, események | 6–10 |
| F1c.4 | `io.monitor.set`: a Program/Source monitor képe a lejátszóból a kártyára, A/V-késleltetés-kiegyenlítéssel | külső monitor (Premiere „Mercury Transmit” megfelelője) | 24–40 |
| F1c.5 | A kimeneti ütem zárása a kártya órájához | nincs csendés csúszás | 10–20 |
| F1c.6 | `docs/hardware-parity.md` „External video monitoring” sorának frissítése | dokumentáció | 1–2 |

**Elfogadás:** hurokteszt kimenet → bemenet bitazonos; 2 órás lejátszás alulcsordulás nélkül; a szerkesztő lejátszása közben a külső monitor képe és hangja szinkronban (villás + sípszó méréssel).

### F2 – HDR és nagy bitmélység (80–140 óra)

| ID | Feladat | Eredmény | Óra |
| --- | --- | --- | --- |
| F2.1 | 10/12 bites RGB (r210, R12L) és YUV be/ki a kártyán | módok a képességlistában | 12–20 |
| F2.2 | HDR-metaadat be/ki: EOTF (SDR, PQ, HLG), ST 2086, MaxCLL/MaxFALL a kártya metaadat-interfészén és ST 2108 ANC-n | `HdrMetadata` mindkét irányban | 16–24 |
| F2.3 | Leképezés a `cfa4-color` színtereire és a `ColorInfo`-ra | helyes színkezelés a szerkesztőben | 8–14 |
| F2.4 | Rögzítés ProRes 4444 XQ 12 bit, DNxHR 444, tömörítetlen 12 bites RGB; FFV1, ha a kódoló elkészült (PROCODEC-002 2.) | kódolóválasztás a `RecordJob`-ban | 16–30 |
| F2.5 | EXR-sorozat mint köztes kimenet | EXR-út | 8–14 |
| F2.6 | Metaadat-megőrzési mátrix konténerenként (MOV `mdcv`/`clli`, MXF, EXR-fejléc), figyelmeztetés rögzítés előtt | táblázat a `docs/decklink.md`-ben + ellenőrzés | 10–18 |
| F2.7 | Numerikus összevetés: PSNR = ∞ a veszteségmentes utakon, küszöb a ProRes-en | tesztek | 10–20 |

**Elfogadás:** a spec F2-feltételei; hurokteszt 12 bites RGB-vel bitazonos; ha a célformátum kevesebbet tud, a rögzítés előtt figyelmeztet; a HDR-metaadat oda-vissza azonos, vagy a jelentés megnevezi.

### F3 – ANC/VANC (80–140 óra)

| ID | Feladat | Eredmény | Óra |
| --- | --- | --- | --- |
| F3.1 | `cfa4-anc`: ST 291-1 csomag (DID/SDID, DBN, DC, UDW, ellenőrzőösszeg, paritás), v210 VANC-sor bontása és építése | crate, fuzz | 16–24 |
| F3.2 | DeckLink ANC-csomag API be és ki | ANC a `CapturedFrame`-ben és az `OutFrame`-ben | 10–16 |
| F3.3 | ST 334 (CEA-608/708 CDP) → `cfa4-captions` → feliratsáv a projektben + MCC-oldalfájl | caption-út | 14–24 |
| F3.4 | AFD és bar data (ST 2016-3) a klipmetaadatba; ATC időkód (ST 12-2) | AFD-út | 8–14 |
| F3.5 | ST 2038 MPEG-TS-be (`cfa4-mpegts`); ismeretlen csomagok átlátszóan oldalfájlba | megőrzés | 10–18 |
| F3.6 | Kimeneten visszaírás; a `cfa4-st2110` -40 (T5) ugyanerre a kódra | közös ANC-kód | 8–14 |
| F3.7 | Tesztvektorok (saját, a szabvány szerint építve; a klvanc-tools összevetésre) | tesztkészlet | 14–30 |

**Elfogadás:** a spec F3-feltételei; sérült csomag (rossz összeg, paritás, hossz) jelölve, nincs összeomlás; rögzítés → export → visszaolvasás után a feliratok és az AFD megmaradnak.

### F4 – Genlock, időkód, többeszközös szinkron (40–80 óra; a PTP az ST 2110-területé)

| ID | Feladat | Eredmény | Óra |
| --- | --- | --- | --- |
| F4.1 | Referenciajel állapota (`bmdDeckLinkStatusReferenceSignalLocked`), kimeneti időzítési eltolás | genlock-állapot és -beállítás | 8–14 |
| F4.2 | `io.sync.status`: genlock, PTP (T1), LTC/MTC (`cfa4-tcsync-io`), RP188 egy nézetben, külön-külön | szinkronállapot | 8–14 |
| F4.3 | Szinkronvesztés-esemény és napló | esemény, napló | 6–10 |
| F4.4 | Több kártya bemenetének egymáshoz mért eltérése hardveres időbélyegből | mérés és napló | 10–20 |
| F4.5 | Többkamerás rögzítés közös időkóddal, a meglévő `multicam` szinkronjához | többkamerás felvétel | 8–22 |

**Elfogadás:** a spec F4-feltételei; a többeszközös teszt két kártyával és közös referenciával dokumentálva.

### F5 és F6 – kikerülnek a DeckLink-területből

Az ST 2110 (F5) és az NMOS (F6) az önálló ST 2110-terület része (LIVE-STUDIO-001 ST 2110 kiegészítés, T0–T7), István 2026-11-14-i döntése szerint. A DeckLink-terv nem tartalmaz ST 2110-, PTP- vagy NMOS-feladatot. A két terület a közös médiapipeline-on és a közös vezérlési infrastruktúrán találkozik (4. fejezet): az ST 2110-vevő és -küldő ugyanazt a HAL-t, rögzítőt és parancsrendszert használja, ezért az SDI ↔ ST 2110 átjárás külön kód nélkül, a közös rétegekből adódik. A Blackmagic IP-kártyák hardveres ST 2110-útja (a DeckLink SDK-n át) nyitott határkérdés (D13).

### F7 – Vezérlés, TSL UMD, tally, Key/Fill, többkimenet (84–156 óra)

| ID | Feladat | Eredmény | Óra |
| --- | --- | --- | --- |
| F7.1 | A DeckLink-parancsok és a jelállapot bekötése a vezérlőfelület-motorba (ADR 0012): `io.*` parancsok leképezhetők, a jelállapot `SurfaceFeedback`-ként kimegy | leképezési profil, teszt | 4–8 |
| F7.2 | TSL UMD v3.1 (UDP), v4, v5 (UDP/TCP) be és ki | protokoll + fuzz | 16–28 |
| F7.3 | Tally → forrás/kimenet állapota a felületen és a monitorokon | tally-megjelenítés | 8–14 |
| F7.4 | OSC és MIDI: nem a DeckLink-területen készül (OSC = a vezérlőfelület-terv F-OSC modulja, MIDI = `cfa4-surface`); itt csak a DeckLink-műveletek alapprofilja | alapprofil | 4–8 |
| F7.5 | DeckLink belső/külső keyer | keyer-vezérlés | 12–20 |
| F7.6 | Key/Fill két szinkron kimeneten, több kimenet (program, előnézet) a stúdió módhoz (LIVE S4 után) | többkimenet | 24–48 |
| F7.7 | Cue-vezérlés a műsortervvel (LIVE S4a) | cue-playout | 16–30 |

**Elfogadás:** a spec F7-feltételei; teszt arra, hogy az adapter csak parancsokat hív; Key/Fill fázishelyesség mérve a huroktesztnél; két kimenet között 0 képkocka eltérés.

### F8 – Rendszerszintű validáció és csomagolás (60–100 óra)

| ID | Feladat | Eredmény | Óra |
| --- | --- | --- | --- |
| F8.1 | Csomagolás: `cfa4-io-host` a `.deb`-ben (a Desktop Video nem függőség), AppImage/Flatpak a rendszer `libDeckLinkAPI.so`-jával | csomagok | 12–20 |
| F8.2 | Diagnosztika: `help.systemReport` DeckLink-szakasz, a host naplója a `Logs` könyvtárban | diagnosztika | 8–12 |
| F8.3 | Együttélési tesztek (lent) | jegyzőkönyv | 10–20 |
| F8.4 | Hosszú futási tesztek: 2 óra minden támogatott útra, 8 óra a fő útra | jegyzőkönyv | 16–28 |
| F8.5 | Telepítés KDE-n (referenciagép) és egy GNOME-os gépen | jegyzőkönyv | 6–10 |
| F8.6 | Felhasználói dokumentáció | `docs/decklink.md` | 8–10 |

**Együttélés (a spec 2.3 pontja, a csonka F8 kiegészítéseként):**

- A CFA4 nem ír a Desktop Video fájljaiba (`/usr/lib/libDeckLinkAPI.so`, `/usr/lib/blackmagic/`, `/etc/blackmagic/`, a kernelmodul), és kérés nélkül nem módosítja a kártya globális beállításait.
- Binárisnév `cfa4-io-host`, beállítás a `~/.config/cfa4/` alatt, nincs D-Bus-név, nincs saját MIME.
- A hálózati portok (TSL, OSC, NMOS) beállíthatók; foglaltság esetén kezelhető hiba.
- Egyidejű futás az OBS-szel, a Blackmagic Media Expresszel és a GStreamerrel: a foglalt kártya érthető üzenet, nem összeomlás.
- A CFA4 libobs-fordítása a DeckLink-plugin nélkül készül; a kártyához csak a `cfa4-io-host` nyúl.

**Elfogadás:** a spec F8-feltételei a fentiekkel, Ubuntu 26.04-en.

## 7. Donortriázs

A 26 donorból a CFA4-be kódként legfeljebb négy kerülhet: a többi referencia vagy teszteszköz. A kódot is a saját rétegeinkhez igazítjuk, nem átvesszük az architektúrájukat. A licencadatok a tárolók gyökerében lévő licencfájlokból és leírásokból származnak (lekérdezve 2026-11-14). A fájlonkénti fejlécellenőrzés és a rögzített commit az F0.3 feladata.

### 7.1 Módszer

Minden donorhoz egy sor kerül a `docs/decklink-donors.md`-be ezekkel a mezőkkel: pontos URL és vizsgált commit; a tároló licence és fájlonként a fejlécek (`licensing.md` 3.); almodulok és beágyazott SDK-másolat; kategória; mit veszünk át konkrétan; biztonság és karbantartottság; együttélési hatás; döntés dátummal.

Kategóriák (a spec 4.5 pontja, egy kiegészítéssel): **1** alap, **2** kód- vagy komponensdonor, **3** algoritmus- és protokollreferencia, **4** tesztdonor, **5** kizárva.

Szabályok:

- SDK-másolatot tartalmazó tárolóból SDK-fájlt nem veszünk át, csak a Blackmagictől letöltött, rögzített verzióból (D1).
- GPL-2.0-only, AGPL vagy licenc nélküli kód: csak külső programként vagy egyáltalán nem; a forrását kódátvétel céljából nem olvassuk (`AGENTS.md` §2).
- Windows-központú részek (DirectShow, Media Foundation) kizárva (spec 2.1).
- A tesztdonor fejlesztői gépen, külső folyamatként fut, nem kerül a csomagba.
- Ami más CFA4-projektben már elkészült, az közös réteg, nem donorfeladat (D4).

### 7.2 A 26 donor besorolása

| # | Donor | Licenc (tárolószint) | Mi van benne | Kategória | Felhasználás |
| --- | --- | --- | --- | --- | --- |
| 1 | [Blackmagic DeckLink SDK](https://www.blackmagicdesign.com/developer) | fejléc: megengedő, EULA-hoz kötve; futtatókönyvtár: zárt | hivatalos API (Desktop Video 16.x) | 1 (D1-től függően) | a kötés alapja |
| 2 | [DeckLinkCapture](https://github.com/soramimi/DeckLinkCapture) | licencfájl a gyökérben nem található | C++11, Qt5, opcionális OpenCV/FFmpeg; Windows, macOS, Linux | 3 (licencig) | callback- és felderítési minta; licenc nélkül kód nem vehető át |
| 3 | [decklink-rs](https://github.com/Julusian/decklink-rs) | MIT/Apache-2.0 | Rust-kötés a decklink-c C-burkolón át; saját bevallása szerint „very incomplete” | 2 vagy 3 | a C-burkoló szerkezete mintának a D2-höz; a decklink-c licence az F0-ban |
| 4 | [hhool SDK-másolat](https://github.com/hhool/Blackmagic_DeckLink_SDK) | nem ellenőrizhető (a várt ágon nincs gyökérfájl) | régi SDK-másolat | 3 (csak dokumentáció) | fájlt nem veszünk át |
| 5 | [manoreken2/projects](https://github.com/manoreken2/projects) | MIT | HDR10Capture: r210/R12B/v210 rögzítés, EXR-export; Windows-alkalmazás | 3 | a pixelformátumok és a PQ-kezelés mintája, saját bitazonos tesztekkel |
| 6 | [bmdtools](https://github.com/lu-zero/bmdtools) | licencfájl nem található | Libav-alapú rögzítő/lejátszó eszközök, az SDK minta könyvtárába fordítva | 3 | csak leírás; a Libav elavult |
| 7 | [GStreamer DeckLink-plugin](https://gitlab.freedesktop.org/gstreamer/gstreamer) | LGPL-2.1-or-later | `decklinkvideosrc`/`sink`, ANC, formátumfelismerés; Ubuntu 26.04-ben csomagolva | 3 és 4 | referencia és tesztorákulum |
| 8 | [blackmagic-io](https://github.com/nick-shaw/blackmagic-io) | MIT | Python I/O könyvtár a hivatalos SDK-ra; capture, HDR statikus metaadat, mátrix; „minimal testing” | 3 | HDR-metaadat- és tartománykezelési minta (F2) |
| 9 | [OBS DeckLink-plugin](https://github.com/obsproject/obs-studio/tree/master/plugins/decklink) | GPL-2.0-or-later | eszközkezelés, be- és kimenet, hibakezelés | 3 | hibakezelési és formátumfelismerési minta; a CFA4 libobs-fordításában kikapcsolva |
| 10 | [pydecklink](https://github.com/Fuse-Technical-Group/pydecklink) | BSD-3-Clause | Python-kötés, capture + ütemezett lejátszás, DeckLink IP-kártyák ST 2110 hálózati felülete; SDK 16.0 | 3 | az IP-kártyák API-ja az F5.3-hoz; a 16.0-s SDK-cél megerősítése |
| 11 | [pi-nmos-st2110 (Atoll)](https://github.com/dgp1000/pi-nmos-st2110) | Apache-2.0 | ST 2110/NMOS multiviewer és bemutató rig, IS-04/05/07, NVIDIA GPU-ra | 3 és 4 | közös réteg: a LIVE T-fázisaihoz referencia és tesztpartner |
| 12 | [Deckboy](https://github.com/Utopian-Academy/Deckboy) | GPL-3.0 | műsorvezérlő és lejátszó: cue, TAKE, broadcast-kimenet, rögzítés | 3 (esetleg 2) | a cue-vezérlés (F7.7) és a LIVE S4a mintája; a GPL-3 átvételt is enged |
| 13 | [Frame Ferret](https://github.com/stoatworks-labs/frame-ferret) | MIT | szoftveres „virtuális capture kártya”: NDI, SRT, ST 2110 végpont, DeckLink-kimenet; a DeckLink capture még nincs kész | 3 | a DeckLink-kimenet mintája; az NDI-rész a D4 szerint kizárva |
| 14 | [libklvanc](https://github.com/stoth68000/libklvanc) | LGPL-2.1 | KLV és ANC/VANC feldolgozás | 3 (esetleg 2) | a saját `cfa4-anc`-hoz referencia; port engedélyezett, ha szükséges (regiszterrel) |
| 15 | [rust-broadcast](https://github.com/fishloa/rust-broadcast) | MIT OR Apache-2.0 | Rust-workspace: MPEG-2 TS, SCTE, feliratok, RTP/SRT/RTMP, SMPTE formátumok; sans-IO | 2 vagy 3 | crate-szintű átvétel lehetséges (ST 291, ST 2038, SCTE); átfedés a CFA4 saját crate-jeivel F0-ban mérendő |
| 16 | [EBU LIST](https://github.com/ebu/pi-list) | GPL-3.0 | ST 2110 mérő- és elemzőeszköz | 4 | teszteszköz a T3/T4 és F5 méréseihez |
| 17 | [statime](https://github.com/pendulum-project/statime) | Apache-2.0 OR MIT | Rust PTP-megvalósítás | 3 | közös réteg: a D9 szerint a rendszer linuxptp-je fut, a CFA4 csak olvas |
| 18 | [linuxptp](https://github.com/richardcochran/linuxptp) | GPL-2 (COPYING; az „or later” a fejlécekből igazolandó) | ptp4l, phc2sys | rendszerkomponens | közös réteg (LIVE D9), külső folyamatként |
| 19 | [nmos-cpp](https://github.com/sony/nmos-cpp) | Apache-2.0 | NMOS IS-04/05/07/08 C++-ban | 2 vagy 3 | a LIVE T7 dönti el (külön folyamat vagy saját Rust) |
| 20 | [AMWA nmos-testing](https://github.com/AMWA-TV/nmos-testing) | Apache-2.0 | NMOS megfelelőségi tesztek | 4 | F6.3 |
| 21 | [tslumd](https://github.com/nocarryr/tslumd) | MIT | Python TSL UMD v5 | 3 | protokollreferencia az F7.2-höz |
| 22 | [tsl-umd-protocol](https://github.com/bitaffair/tsl-umd-protocol) | MIT | JavaScript TSL UMD | 3 | protokollreferencia |
| 23 | [TSL-UMD-Tester](https://github.com/videojedi/TSL-UMD-Tester) | MIT | TSL UMD teszteszköz | 4 | F7.2 elfogadás |
| 24 | [SDPoker](https://github.com/Streampunk/sdpoker) | Apache-2.0 | SDP-ellenőrző ST 2110-hez | 4 | közös réteg: a `cfa4-st2110` SDP-tesztjeihez |
| 25 | [smpte2110-tester](https://github.com/necat101/smpte2110-tester) | MIT (a leírás szerint) | Python tesztkészlet: -10, -20, -21, -30 | 4 | kiegészítő teszt a T-fázisokhoz |
| 26 | [klvanc-tools](https://github.com/stoth68000/klvanc-tools) | LGPL-2.1 | KLV/ANC diagnosztika | 4 | ANC-tesztvektorok összevetésre (F3.7) |

**Összesítve:** kódként átvehető jelölt négy van (decklink-rs, rust-broadcast, libklvanc, Deckboy), mindegyik csak az F0.3 fájlonkénti ellenőrzése után. Két donornál nem található licenc (DeckLinkCapture, bmdtools): ezek csak leírásként használhatók. A hhool-másolat állapota innen nem volt ellenőrizhető.

## 8. Kockázatok, ütemezés és becslés

A legnagyobb kockázat a licencdöntés (D1), utána a hardver hiánya. Az F0 és az F1a mindkettőtől független, ezért azonnal indulhat; az eredménye (élő forrás, natív rögzítő) az ST 2110-vevőnek és az élő stúdiónak is kell.

### 8.1 Sorrend

1. **F0** (döntések, ADR, donorok) → **F1a** (hardver és D1 nélkül).
2. D1 = igen és van kártya → **F1b** → **F1c**.
3. Az F1b után párhuzamosan: **F2**, **F3**, **F4** (a genlock-rész; a PTP a LIVE T1-ből jön).
4. **Az ST 2110-terület (T1–T7) a saját ütemezése szerint halad; a közös rétegeket (F0.5, F1a) mindkét terület használja.**
5. **F7** az F1c után; a Key/Fill és a többkimenet a LIVE S4 után.
6. **F8** a végén; a Qt6-panelek minden fázisban a Qt6 befejezése után (D7).

### 8.2 Becslés

| Fázis | Óra (nagyságrend) | Hardver kell? |
| --- | --- | --- |
| F0 | 30–50 | nem |
| F1a | 100–160 | nem |
| F1b | 120–200 | igen |
| F1c | 80–140 | igen |
| F2 | 80–140 | igen |
| F3 | 80–140 | részben (a `cfa4-anc` nem) |
| F4 (DeckLink-rész) | 40–80 | igen, két kártya |
| F5, F6 (az ST 2110-területé, itt nem számolva) | — | igen, + hálózat |
| F7 | 84–156 | részben |
| F8 | 60–100 | igen |
| **Összesen** | 674–1166 |  |

### 8.3 Kockázatok

| Kockázat | Hatás | Kezelés |
| --- | --- | --- |
| A D1 „nem” | a DeckLink kimarad | az F1a, az ST 2110 és a vezérlés önállóan is értékes |
| Desktop Video DKMS-modul és a 7.0-s kernel | a kártya egy kernelfrissítés után eltűnhet | felismerés és érthető üzenet; a CFA4 nem nyúl a driverhez; F0.2-ben ellenőrizve |
| Valós idő a Broadwell-magokon | 2160p50 12 bites RGB tömörítetlenül kb. 10 Gbit/s lemezre | lemez- és NUMA-mérés az F1b-ben; ProRes-kódoló sebessége mérve |
| Globális kártyabeállítások | más program felvétel közben átállíthatja | esemény, nem csendes hiba |
| Két rögzítő a gyakorlatban | a „nincs párhuzamos motor” szabály sérül, ha a LIVE S5 a libobs felvételével indul | D5 a LIVE-tervvel együtt döntendő |
| Nincs hardver a CI-ben | hardveres hibák későn derülnek ki | mock és visszajátszó szolgáltató a CI-ben; hurokteszt-jegyzőkönyv a referenciagépen |
| Csonka spec | az F8 elfogadási feltételei változhatnak | D12 |

## 9. Beilleszthetőség: feltételek a futó CFA4-fejlesztésekben

A DeckLink-fejlesztés nem indul el. Ez a fejezet azt rögzíti, mit kell a **most futó** projekteknek megőrizniük vagy előkészíteniük, hogy a DeckLink-terület később átírás nélkül beilleszthető legyen, és semmi ne készüljön kétszer. A felmérés a `main` (0d598d11) és a 2026-11-14-én nyitott PR-ok alapján készült.

### 9.1 A feltételek típusai

- **A – már illeszkedik:** nincs teendő, csak a DeckLink-terület alkalmazkodik.
- **B – megőrzendő tulajdonság:** a projekt saját tervén belül marad, nem tervmódosítás. A koordinátor a PR-ellenőrzéskor nézi, hogy a tulajdonság nem sérül.
- **C – tervkiegészítés:** az érintett projekt tervébe új lépés vagy átvételi feltétel kerül. A `coordination.md` §2.10 szerint ehhez István jóváhagyása kell; addig az érintett rész ott sem változik.

### 9.2 Feltételek projektenként

| # | Futó projekt (PR-ok) | Amit a DeckLink-terület használni fog | Feltétel | Típus |
| --- | --- | --- | --- | --- |
| V1 | LIVE-STUDIO-001 ST 2110 T1 (#1128) | PTP-állapot, SMPTE-epoch médiaóra | A PTP-állapot olvasója (`crates/obs/src/ptp.rs`) ne a stúdióhosthoz kötve maradjon: stúdiófüggetlen típussal egy közös, L5 alatti helyre kerüljön (javaslat: `cfa4-st2110::clock` mellé), hogy a `cfa4-io-host` ugyanazt olvassa. A médiaóra (`clock.rs`) maradjon stúdiófüggetlen. | C |
| V2 | LIVE-STUDIO-001 S0b, S1a (#1129, #1189) | jelenetforrás, host-felügyelet, `cfa4-obs` gyűrű | A `SourceKind` (`crates/project/src/studio.rs`, `tag = "type"`) új „capture” forrástípussal additívan bővíthető maradjon, projektformátum-törés nélkül; ismeretlen típus betöltése kezelhető hiba, nem összeomlás. A gyűrű képformátuma ne legyen végleg RGBA-ra rögzítve: a fejléc hordozzon formátumot és verziót (gyűrű v2). | B |
| V3 | LIVE-STUDIO-001 S5 (felvétel, még nem indult) | egyetlen rögzítő (`cfa4-ingest`) | A D5 szerint az S5 ne induljon a libobs saját felvételével; a felvétel a közös rögzítőn menjen. | C |
| V4 | CONTROL-SURFACE-001 (ADR 0012; #1184, #1187, #1193, #1231, #1281, #1286) és HW-CONTROL-SURFACES-001 (#1194, #1195, #1287) | TSL UMD, tally, OSC, MIDI, adapterfolyamat-minta | A `SurfaceProtocol` és a profilséma additívan bővíthető maradjon (új forrás: TSL UMD). A `SurfaceFeedback` tudjon tally-állapotot hordozni (piros/zöld/sárga, forrás- vagy kimenetazonosító). Az F-OSC modul a vezérlőfelület-terv része marad. Az adapterfolyamat-protokoll (`cfa4-surface-adapter` v1) és felügyelője verziózott, általános minta maradjon, mert a `cfa4-io-host` is ezt követi. | B (bővíthetőség), C (tally-visszajelzés, TSL-forrás) |
| V5 | PROCODEC-002 (#1204, `VideoFrame::layout`) | 10 és 12 bites YUV és RGB képkockák | A `FrameLayout`/`Chroma` lefedje a 4:2:2 10 bites és a 4:4:4 12 bites RGB képkockát. A kódolók (ProRes, DNx) natív YUV-bemenete (ma RGBA) a PROCODEC-ben készüljön, ne a DeckLink-területen. | A (a kódolók már síkokra bontott YUV-t kapnak), B: ez maradjon így |
| V6 | TIMECODE-SYNC LTC/MTC (#1285; `cfa4-tcsync`, `cfa4-tcsync-io`) | RP188/VITC, referencia-bemenet állapota | A szinkronállapot forrásai additívan bővíthetők maradjanak (új forrás: SDI-időkód, referenciajel). A chase-motor ne feltételezze, hogy a forrás hang vagy MIDI. | B |
| V7 | SECURITY-001 (#1020, `cfa4-sandbox`) | a `cfa4-io-host` korlátozása | A `ProcessKind` additívan bővíthető maradjon egy `ioHost` folyamattípussal: a `/dev/blackmagic*` eszközcsomópontok és a Desktop Video könyvtárai olvashatók, a felvételi fájl írható, hálózat és programindítás nincs. | C (új folyamattípus a SECURITY-tervben) |
| V8 | EVENT-FABRIC-001 | folyamatok közti események (pl. „új felvétel” a BacklotHubnak) | A DeckLink-terület nem hoz létre saját eseménybuszt. Az io-host és a szerkesztő kapcsolata pont-pont adapterprotokoll (az E10 értelmezése szerint nem busz); a folyamatok közti események az Event Fabricen mennek. | A |
| V9 | FILE-SERVICES-001 | a kész felvételi fájlok | A kész felvétel a fájlszolgáltatás katalógusába kerüljön (atomikus véglegesítés, BLAKE3). Ellenőrizni kell, hogy az API-ban van-e „kész, kívül írt fájl regisztrálása” művelet; ha nincs, az kiegészítés. | B, ha van ilyen művelet; különben C |
| V10 | QT6-FINISH-001 (#1291) | io-panelek | A D7 szerint a Qt-panelek a Qt6 befejezése után jönnek; a panelkeretnek nem kell előre semmit tudnia. | A |
| V11 | APP-LAUNCHER-001 (ADR 0014, #1275) | nem használja | Az io-hostot a `cfa4-io` felügyelő indítja, nem az alkalmazásindító; nincs átfedés. | A |

A V1–V11 feladatszintű terve a fejlesztőknek: [CFA4-DECKLINK-READY-001](CFA4-DECKLINK-READY-001-FINAL-1.0.md) (FINAL 1.0, jóváhagyva 2026-11-15).

### 9.3 Hogyan érvényesül

1. A koordinátor a fenti listát átadja az érintett projektek munkameneteinek. A B típusú tételeket a PR-ellenőrzéskor nézi; ha egy PR sértené valamelyiket, az a PR leírásában jelzett átfedés (`coordination.md` §2.8).
2. A C típusú tételeket (V1, V3, V4 tally/TSL, V7, és szükség esetén V9) az érintett projekt munkamenete tervmódosításként javasolja, István jóváhagyja. Jóváhagyásig az érintett lépés ott sem változik (§2.10).
3. A DeckLink-terület semmilyen kódot nem ír a futó projektekbe, és nem vesz át onnan feladatot.
4. **Indítási kapu a DeckLink F0-hoz:** (a) minden C tételről döntés született; (b) a F0.6 átfedés-ellenőrzés újra lefutott az akkori `main`-en; (c) István külön elindítja a fejlesztést.

### 9.4 Változásnapló

| Verzió | Dátum | Változás |
| --- | --- | --- |
| FINAL 1.0 | 2026-11-14 | István jóváhagyta (D1–D13 a javaslatok szerint). Véglegesítéskor: D6 a vezérlőfelület-motorra épül, a `cfa4-control-io` elhagyva; F7 84–156 óra, összesen 674–1166 óra; új 9. fejezet a beilleszthetőségi feltételekkel. |

