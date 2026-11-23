# CFA4-RENDER-INTEGRATION-PLAN-001 v1.1

2026-10-31 · ubuntuokos

> **Repó-példány.** A terv élő példánya a tulajdonos Claude Docs dokumentuma
> (<https://claude.ai/artifact/LU4M9conuPpCpHp4JLWbW1>, 119. revízió, 2026-11-23, a tulajdonos által jóváhagyott végleges változat); ez a fájl annak a
> `main`-re hozott másolata (`VISION.md` 2.6). Eltérés a forrástól: az upstream- és a #11-es jelölt
> márkanevét a `python/tests/test_independence.py` szűrője miatt semleges megnevezés váltja. A szöveg egyébként szó szerint a forrás.
> **Végrehajtási állapot:** az F0a eredménye és a tervet érintő helyesbítések:
> [`docs/audit/RENDER-F0a-BASELINE-2026-11-14.md`](../audit/RENDER-F0a-BASELINE-2026-11-14.md).

Videórenderelési alrendszer célzott fejlesztési és integrációs terve. Célplatform: Ubuntu 26.04 LTS, amd64. Állapot: FINAL 1.1, JÓVÁHAGYVA (Horváth H. István, 2026-11-21; a kritika javaslataival, az ADR 0003 §14-gyel és az ST 428 csomaggal kiegészített végleges változat jóváhagyva 2026-11-22). A 2b szakasz (2026-11-23: S1, F0V, F1 és C0 leltár hatása) is jóváhagyva (István, 2026-11-23); a végleges, egységesített változat (rev 102: az S1 szerinti 3 F2-jelölt mindenhol, AC-14 PASS-definíció, kritikai hiányzárás) jóváhagyva 2026-11-23; ugyanezen a napon jóváhagyva: az EC-jelölt marad MQ-kapuval, a VapourSynth Python ≥ 3.13 igenénye a külön workerben, a GEGL csak feltételes összehasonlítási pont (rev 110); a GEGL és a babl 2026-11-23-án továbbépült tényleges F2 POC-jelöltté (rev 117); a GEGL+babl-lel bővített végleges terv, 4 F2-jelölttel (OpenImageIO, VapourSynth, EC-jelölt, GEGL+babl), jóváhagyva 2026-11-23 (végső rev 119). A repo-ban lévő másolat szinkronizálása a végrehajtó szál feladata (PR, "Gazda: …" issue). A végrehajtás az F0b/F0V és F1 fázissal indul, külön végrehajtó szálakban, a repo szabályai szerint (PR, "Gazda: …" issue, nincs merge). Kizárva: OxiMedia, Vercel, HandBrake.

## 1. Vezetői összefoglaló

A CFA4 natív, upstream-eredetű (`NOTICE`) renderelési motorja, projekt- és idővonalmodellje marad az elsődleges; teljes motorkiváltás nincs. A fejlesztés célja, hogy a CFA4 **mozi minőségű** kimenetet adjon CPU-only módban is, minimális külső függőséggel és a lehető legkevesebb Python verzióval.

A v1.0-hoz képest a v1.1 fő változásai:

- **Mozi-minőségi kapu (MQ-1–MQ-8)** minden jelöltre és kiadásra; a 20 jelöltből 6 jut a POC-ba.
- **Két új alapozó fázis:** F0V (validációs alap) és C0 (szerződések és job API v0). Ezek szállítják a v1.0-ban fázishoz nem rendelt cfa4-render-validation, -contracts és -job-service modulokat.
- **S1 szűkítési döntési pont** F1 és F2 között.
- **A11 szabály:** a lehető legkevesebb Python verzió, minden további verzió indoklással és nyilvántartva.
- **Mérhető AC-k:** minden AC-hez metrika, módszer és F0-ban rögzített küszöb; új AC-13–AC-20.
- **Minden fázis egységes sablonnal:** cél, előfeltétel, feladatok, átadandók, kilépési feltételek, kockázat, visszavonás, időkeret.
- A régi F7 (capture) interfészrésze C0-ba kerül; önálló rögzítőfejlesztés ebben a tervben nincs.

**2026-11-21 módosítás:** a projektgazda döntött a három nyitott kérdésről (teljes DCP a CFA4-ből, feliratok a ciklusban, OFX-hosting cél). A 2026-11-21-i repóellenőrzés szerint mindhárom területre már létezik jóváhagyott terv, ezért az F5S, F5D és F5O nem új fejlesztés, hanem hivatkozás a meglevő tervekre (2a. szakasz); a render-terv csak az AC-17, AC-19 és AC-20 mérését adja.

**Jóváhagyásra javasolt döntések**

1. A célarchitektúra, az A1–A12 szabályok és az MQ kapu elfogadása.
2. Az F0a, F0b/F0V és F1 végrehajtásának engedélyezése (időkeret összesen kb. 13–16 munkanap).
3. Minden további fázis (S1-től) kizárólag a megelőző kapu eredménye és külön jóváhagyás alapján indul.

## 2. Kiindulási adatok és bizonyítottság

Az alábbi adatok korábbi állapotközlések; a jelenlegi HEAD-en egyik sincs újraellenőrizve, ezért mind PENDING az F0a/F0b lezárásáig.

| Adat | Érték | Forrás | Állapot |
| --- | --- | --- | --- |
| Qt6 GUI készültség | kb. 53% | közlés, 2026-10-28 | PENDING |
| Alap | Rust-központú upstream-derivált | közlés | PENDING |
| FFmpeg | 9.0.2 | közlés | PENDING (verzió és `configure` flagek) |
| Rust toolchain | 1.95 | közlés | PENDING |
| R0 GPU eredmények | H.264/HEVC/AV1 NVENC PASS; H.264/HEVC NVDEC PASS | korábbi R0 | PENDING (újrafuttatás HEAD-en) |
| Exportteszt-rések | J2K orákulum, heif-enc/HEIC round-trip, phone\_vfr | PR #271, #331 | PENDING (mozi célra a J2K blokkoló) |
| Qt6 paritáshibák | ismertek, listájuk nincs csatolva | közlés | PENDING |

**Hivatkozott, de azonosító nélküli dokumentumok:** a korábban jóváhagyott hang/videó-rögzítési terv és a kodekfejlesztési sorrend. Ezek ID-jét és verzióját F0a-ban rögzíteni kell, különben a rájuk épülő kapuk nem ellenőrizhetők.

**Bizonyítási szabály:** amit forráskód, teszt vagy reprodukálható mérés nem támaszt alá, az PENDING. Upstream marketingállítás nem PASS.

### 2a. Meglévő munkák leltára (A12), ellenőrizve 2026-11-21

Forrás: a `ubuntuokos/CinemaFundArchitecture4` `main` ága, `5d5cd4c` (#1422 beolvasztás, 2026-11-21 05:31), csak olvasással, shallow klónból. A repó F0a-jelentése (2026-11-14) több kiinduló feltevést helyesbít: a Rust toolchain **1.97.1**, MSRV **1.97** (nem 1.95, ezért a ReelForge MSRV-akadálya megszűnik); a Python `requires-python >= 3.12`, CI 3.13 (blokkoló) és 3.14 (nem blokkoló); az FFmpeg saját 9.0.2 build.

| Terv-terület | Meglévő terv és kód | Minősítés | Mi marad e tervben |
| --- | --- | --- | --- |
| F0V validáció | `cfa4-golden` (CPU–GPU golden), `cfa4-metrics` (ΔE₀₀, SSIM), `cfa4-qc`, `cfa4-testkit`; `docs/testing.md` | RÉSZBEN | PSNR síkonként, hang null-teszt, PTS/DTS diff, mutációs önteszt, 16 bites/EXR golden, küszöbfájl: a meglévő crate-ekbe |
| C0 szerződések | `cfa4-frame` (VideoFrame, PixelData, ColorInfo, PAR, FrameLayout), `cfa4-time`, `cfa4-color` | RÉSZBEN | CPU/GPU-hely, fence, frame-szintű HDR side data, forrásazonosító |
| C0 job API | `cfa4-export::job`, engine-parancsok; `CFA4-TASKSTORE-PG-001` (elfogadva 2026-11-07, kód nincs), `CFA4-EVENT-FABRIC-001` | RÉSZBEN, ÁTFEDÉS | tulajdonosi döntés kell (`coordination.md` §2.8) |
| F3 render graph, cache | `cfa4-render::plan`, `preview`; `cfa4-smart-edit`; `CFA4-SMART-CUT-RENDER-R1`, `SMART-RENDER-2026-10-27` | RÉSZBEN, ÁTFEDÉS | F3 előtt eldöntendő: bővítés vagy új crate |
| F4 GPU | `cfa4-gpu` (wgpu, WGSL), ADR 0004; golden CPU–GPU paritás | RÉSZBEN | színhelyesség és VRAM-budget mérése |
| F5 EXR, képsorozat | `cfa4-export` (EXR half-float alfa, DPX, TIFF, PNG), `crates/media` EXR-olvasó OpenImageIO-orákulummal; ADR 0003 A2 (OCIO) és A3 (OpenImageIO) | RÉSZBEN, TERV VAN | OpenImageIO csak ott, ahol az A3 nem fedi |
| Feliratok | `cfa4-captions`, `cfa4-text`; `SUBTITLE-STUDIO-PLAN-001` FINAL 1.2 (jóváhagyva 2026-10-30); PROCODEC-002 10. munkacsomag | LEFEDETT | ST 428-7 írás és ST 429-5 timed text MXF írás hiányzik (a fenti tervek viszik) |
| DCP | `dcp_out.rs` (SMPTE OV), `cfa4-dcp`, `cfa4-dcp-validate`, `cfa4-dcp-inspect`, `cfa4-cinema-pkg`, `cfa4-dcpomatic`; `CFA4-DCPOMATIC-001` v1.0, `CFA4-DCPVAL-LIBDCP-001`, `CFA4-CINECERT-001` v2.3, PROCODEC-002 5a | LEFEDETT/RÉSZBEN | csak az AC-19 mérése; lásd F5D |
| IMF | `imf_out.rs` (App 2E), `cfa4-j2k::ImfProfile` | KÉSZ | – |
| OFX | ADR 0003 A4 (jóváhagyva 2026-10-24): OpenFX-host külön folyamatban, első csomag clean-room (§14: az openfx-misc csak referencia); `docs/gaps.md` G11; `crates/sandbox` `pluginHost` profil | TERV VAN, KÓD NINCS | lásd F5O és a döntési kérdés |
| Media QC | `cfa4-qc`, `MEDIA-QC-2026-10-27` | RÉSZBEN | – |
| VapourSynth, ReelForge, rust-motion | semmilyen említés a repóban (a render-terven kívül) | NINCS | valóban új jelöltek |

A leltár `F0-existing-plans-inventory.md` formában az F0a feladata marad; ez a táblázat az első, kézi ellenőrzésen alapuló változata. A capture, a DeckLink és az élő stúdió terveit (`CFA4-LIVE-STUDIO-001`, `CFA4-DECKLINK-001`) nem néztem át részletesen: PENDING.

### 2b. Végrehajtási állapot és tervmódosítások (2026-11-23)

Forrás: a `main` ág `e493e95` (#1640 beolvasztása) és a #1650 PR (C0 leltár, még nincs beolvasztva), csak olvasással. A FINAL 1.1 szerkezete és az A12 megmarad; az alábbi pontok a végrehajtás bizonyítékából adódó módosítások.

- **F0V kész (mérő és küszöb):** a Küszöb@F0 a mért zajból származik (CPU-út, egy gép, Ubuntu 24.04 konténer): RGB és alfa eltérés 0, PSNR ≥ 96 dB, SSIM ≥ 0,9999, ΔE2000 0, PTS-hiba 0 tick, hang null-teszt ≤ −200 dBFS, A/V drift ≤ 0,1 ms. Gép- és GPU-közötti zaj, AC-07, AC-09 és AC-16 küszöbe PENDING a referenciagépig. Az ismert eltérések (KD-01–KD-08) és a CI-szintek (smoke, nightly, hardware) definiálva vannak, bekapcsolásuk nincs. A J2K orákulum (KD-01) továbbra is blokkoló mozi kiadáshoz.
- **F1 kész, S1 javaslat jóváhagyva (István, 2026-11-22):** F2-be 4 jelölt megy (a negyedik a GEGL és babl, 2026-11-23): OpenImageIO, VapourSynth (csak külön workerként), EC-jelölt (feltételesen, MQ-1: 32 bites réteg-kvantálás nélkül). A ReelForge, az avio/ff-\* és a rust-motion F1 REFERENCE\_ONLY (egyetlen karbantartó), ezért csak minták. Következmény: a D1-ben nincs ReelForge-döntés, az F3 render graph a CFA4 sajátja. Az S1 teljes aláírása még nincs meg.
- **F1 sandbox-döntés:** a meglévő `cfa4-sandbox` (Landlock és seccomp), a CFA4-SECURITY-001 D3 szerint, nem döntjük újra. Erőforrás-korlát: `systemd-run --user` cgroup v2. A render-workerekhez két új profilfajta kell (`renderWorker`, `externalRender`), ezt a SECURITY-001 felelőse veszi fel. A sandbox még egyik folyamatban sincs alkalmazva, ezért a worker-POC csak kísérleti.
- **C0 leltár (#1650):** a `cfa4-render-job-service` nagyrésze már létezik a `cfa4-tasks` crate-ben (ADR-0008: Queue állapotgép, sorrend, lease, Manager, Budget, Monitor). Ezért a job-service v0 egy render-osztályú kliens, nem újabb sor és állapotgép. Valóban új csak: Frame ABI kiegészítések a `cfa4-frame`-ben, explicit konverziós mátrix, render-eseménytípus, cancel-latencia és SIGKILL-mérés (AC-08, AC-09), worker protokoll v0, vékony adapter és CLI kliens. A `cfa4-render-contracts` és a `-job-service` név vékony crate-ként maradhat, a típusok forrása a meglévő crate-ekben marad. A rétegezést (`cargo xtask layers`) a megvalósítás előtt ellenőrizni kell. A TASKSTORE-PG-001 PG-store most külön szálon kerül a főágba, ez a job API v0-t nem érinti.
- **Nyitott döntések:** (1) a C0 crate-név és a terv szövegének tulajdonosi jóváhagyása (coordination §2.10); (2) a QTGMC-motor és a Blender licence és verzió-pin: PENDING.
- **Hatás a sorrendre:** a F2 határa 4 jelölt, ez a 10 munkanapos F2-ben csökkenti a terhet; az F3 előfeltétele (ReelForge) megszűnt, az F3 a C0 v0-n és a meglévő `cfa4-tasks`/`cfa4-render::plan` bővítésén indulhat.

**A kritikai elemzés javasolt javításai elfogadva (István, 2026-11-23).** Ellenőriztem, hogy a FINAL 1.1 minden javaslatot tartalmaz (F0V és C0 fázis, hang- és színkezelés, AC-ek mérhető definíciója, Frame ABI mezők, kockázatnapló, szerepkörök, üzemrend). Két szűk kiegészítés tartozik még ide: (1) a Frame ABI mezői közé a timecode is bekerül (a proxy–eredeti megfeleltetéshez, a forrásazonosító mellett); (2) leállási szabály: ha az F0V/F1 után egy területhez nincs bizonyított GAP vagy mérhető cél, az a terület lezárul, és ez a terv nem nyújt új fejlesztést (az S1 3. szabálya ezt már alkalmazza).

**A kritika hiányainak zárása (2026-11-23, A12 szerint: ahol van döntés, arra hivatkozunk).** A átvizsgálás szerint a hiánylistából két pont maradt nyitva a tervben, mindkettőre van már meglévő forrás:

- **Színkezelési rendszer:** a döntés megvan, ADR 0003 A2 (OpenColorIO és ACES-config, `crates/ocio`, opcionális, OCIO nélkül a program a `crates/color`-ral fut tovább). Az EXR/OpenImageIO bevezetése (F5) erre épül, nem hoz új rendszert. A render-terv teendője: a mozi útvonalra (MQ-3) külön igazolni a DCI-P3 és X'Y'Z' átalakítást; a munkaszíntér döntése (ACES vagy nem) az ADR 0003 A2 szerinti, nem ennek a tervnek a döntése, ez PENDING az ADR gazdájánál.
- **Előnézet–export színparitás (AC-14):** mérő még nincs (F0V mérőtérkép: PENDING, a harness nem hívja az előnézet-rendert). Javasolt PASS-definíció: az előnézet és az export ΔE2000 eltérése a referenciacsomagon ≤ a Küszöb@F0 és legfeljebb 1,0 (az ADR 0003 A2 elfogadási kritériuma szerint), CPU-n és GPU-n; alfa- és HDR-eset külön. Kilépés: az F3 előtt a harness az előnézet-renderre hívódik.

A többi hiány (hang-renderút, hang null-teszt, projektfájl-migráció, determinizmus, szerepkörök, időkeretek, leállási szabály) a tervben már szerepel; az AC-15 (projektmigráció) fixture-je (régi sémájú projekt) még hiányzik, ezt az F0V-nek kell pótolnia.

**GEGL és babl F2 POC-jelölt (István, 2026-11-23).** Indok: sok projektben használják, ezért nem vetjük el. Az S1 5. szabálya (legfeljebb 6 jelölt) teljesül: 4 jelölt van az F2-ben (OpenImageIO, VapourSynth, EC-jelölt, GEGL+babl).

- **Feladat az F2-ben:** kompozíciós és lineáris f32 összehasonlítás az EC-jelölttel, ugyanazon a C0 szerződésen, a Küszöb@F0 mérőkkel (PSNR, ΔE2000, alfa).
- **MQ-kapu jelöltenként:** MQ-1 (≥ 16 bit belső pontosság; f32), MQ-3 (színkezelés DCI-P3 és X'Y'Z' felé; a babl színtér-képessége), MQ-4 (alfa), MQ-8 (lineáris fényű blend). FAIL esetén csak mintának marad. Az első 3 munkanapon dől el.
- **Licenc:** a GEGL és a babl általános ismeret szerint LGPL-3.0-or-later, ami összefér a CFA4 GPL-3.0-or-later licencével; ez PENDING, az F1 kiegészítő audit igazolja (A9: forrásfák, tranzitív függőségek, SBOM). Kezelés: dinamikus kapcsolás vagy külön worker folyamat, kódátvétel és vendorizálás nélkül.
- **F1 kiegészítés (még nincs meg):** licenc és tranzitív függőségek, karbantartók száma és aktivitása (életképességi küszöb: ≥ 2 karbantartó, 12 hónapon belüli kiadás), host-lábnyom (GLib/GObject-függőség az A6 szerint), sandbox-profil (`renderWorker`), Python-igény (nem várt). Ha az F1 BLOCKED-et ad, a jelölt kiesik.
- **Az EC-jelölt és a Python-döntés változatlan.**

## 3. Jelöltjegyzék a mozi-minőségi szűkítés után

A 20 jelöltből 1 alap, 4 F2-jelölt (S1, 2026-11-22: OpenImageIO, VapourSynth, EC-jelölt; 2026-11-23: GEGL és babl hozzáadva), 2 külső munkafolyamat, 8 referencia (ReelForge, avio és rust-motion csak mintának), 6 kizárt. A besorolás előzetes; véglegesítése az S1 döntési pont (F1 eredménye alapján). A táblázat nem bizonyítja, hogy egy képesség hiányzik a CFA4-ből; azt F0 mutatja ki.

| # | Jelölt | Szerep a CFA4-ben | Státusz | Python | Megjegyzés |
| --- | --- | --- | --- | --- | --- |
| 01 | FFmpeg 9.0.2 | decode/filter/encode/mux, export-orákulum | ALAP, MEGTARTANDÓ | nem | A7 szerint egyetlen libav ABI. |
| 07 | OpenImageIO | EXR és képszekvencia I/O, OCIO-kapcsolat | POC-JELÖLT | nem | Erős MQ-jelölt. |
| 04 | VapourSynth | resztauráció, deinterlace, szűrő worker | POC-JELÖLT | igen, 1 nyilvántartott verzió | Retusáláshoz szükséges; a szkriptes eszközök (pl. QTGMC) Pythont igényelnek, ez indokolt A11-igény. |
| 09 | ReelForge | DAG, dirty-region, cache, resume algoritmus | REFERENCIA (S1, 2026-11-22) | nem | F1 REFERENCE\_ONLY (egyetlen karbantartó): csak a DAG, dirty-region és resume mintája; az F3 render graph a CFA4 sajátja. |
| 08 | avio / ff-\* | Rust médiaréteg | REFERENCIA (S1, 2026-11-22) | nem | F1 REFERENCE\_ONLY (egyetlen karbantartó, saját libav-kötés sértené az A7-et); semmi nem marad belőle, a `crates/ffmpeg` megvan. |
| 11 | EC-jelölt (#11) | réteges kompozíció, kulcskockák | POC-JELÖLT, feltételes | PENDING | MQ-1/3/4/8 igazolása F2 elején. |
| 10 | rust-motion | címek, SVG, procedurális grafika | REFERENCIA (S1, 2026-11-22) | nem | F1 REFERENCE\_ONLY (egyetlen karbantartó): minta a G-19 alfa- és élsimítási célhoz, a megvalósítás a CFA4-ben történik. |
| 05 | Blender | 3D/VFX képszekvencia-renderjob | KÜLSŐ MUNKAFOLYAMAT | saját, beépített | EXR csere, külön folyamat, verzió pinelve. |
| 06 | Natron | OFX node-modell minta | KÜLSŐ / REFERENCIA | saját | A projektgazda szerint kódja korábbi Qt-ra épül, nem kompatibilis a CFA4-gyel; az ADR 0003 §14 (2026-11-21) szerint az openfx-misc csak képesség-referencia, az első OFX-csomag clean-room; a Natron orákulumnak sem használható (Qt4 összeomlik Kubuntu 26.04-en). |
| 18 | MovieLite | golden-frame módszer | REFERENCIA | – | Csak módszer, F0V-ben. |
| 02 | MLT | átmenet- és playlist-modell | REFERENCIA | – | Képláncba nem. |
| 13 | Gausian Native Editor | proxy/preview minták | REFERENCIA | – | Proxy nem mastering út. |
| 14 | Dioxuscut Renderer | rust-motion átfedés | REFERENCIA | – | Csak átfedés-döntéshez. |
| 03 | GStreamer | capture/stream | REFERENCIA (rögzítési terv) | – | Mastering láncon kívül. |
| 12 | Lumen / Lumiscia | Vulkan GPU benchmark | KIZÁRVA ebből a ciklusból | – | Kísérleti; később külön benchmark. |
| 15 | MoviePy | – | KIZÁRVA | igen | MQ-1 FAIL (8 bites RGB). |
| 16 | Movis | – | KIZÁRVA | igen | Mozi láncba nem; A11. |
| 17 | Manim | – | KIZÁRVA | igen | 8 bites kimenet; A11. |
| 19 | PyAV | – | KIZÁRVA | igen | Második libav ABI (A7); A11. |
| 20 | VidGear | – | KIZÁRVA | igen | OpenCV, 8 bites BGR; A11. |
| 21 | OpenFX (API és host support library) | OFX-gazdagép alapja | POC-JELÖLT (F5O) | nem | Nem új jelölt: ADR 0003 A4 (jóváhagyva 2026-10-24) már tartalmazza; BSD-3. Hivatkozás. |
| 22 | GEGL + babl | gráf-alapú képfeldolgozás, lebegőpontos (f32) lineáris fényű kompozíció, pixelformátum-konverzió | POC-JELÖLT (2026-11-23, István) | nem | Sok projektben használt; LGPL-3.0-or-later (PENDING, F1 igazolja); MQ-1/3/4/8 jelöltenként; összehasonlítás az EC-jelölttel. |

## 4. Architekturális szabályok és a mozi-minőségi kapu

Az A1–A12 szabályok és az MQ kapu minden fázisra kötelezők; eltérés csak írásos architektúra-döntéssel (ADR) és jóváhagyással.

**A1 – Egyetlen projektigazság.** A CFA4 upstream-eredetű projektmodell és idővonal a source of truth. Külső projektséma csak import/export peremen.

**A2 – Egységes Frame ABI.** Pixel- és plane-formátum, bitmélység, színprimerek, transfer, mátrix, tartomány, alfa mód (straight/premultiplied), méretek és stride, SAR, field order, forgatás/display matrix, chroma siting, HDR side data (mastering display, MaxCLL/MaxFALL), PTS és timebase, forrás-azonosító, ownership/lifetime, CPU/GPU hely, fence/szinkronizáció. Részletes definíció: C0.

**A3 – Egységes Audio ABI.** Mintavételi frekvencia, mintaformátum, csatornaelrendezés, időalap, mintapontos pozíció, buffer-élettartam, késleltetés, drift-kezelés.

**A4 – Capability discovery és szabálytábla.** A CPU, Vulkan/wgpu, CUDA és adapter-képességek egységes lekérdezése. Capability × környezet szabálytábla: exportnál FAIL\_CLOSED, preview-nál JELZETT\_FALLBACK; csöndes minőségromlás soha.

**A5 – CPU-only kötelező.** Minden kiadáshoz szükséges workflow és az MQ kapu GPU nélkül is teljesül, legfeljebb lassabban.

**A6 – Host coexistence.** Külső binárisok, plugin cache, FFmpeg és Rust könyvtárak kizárólag izolált, névterezett módon. Nincs globális csomagfrissítés, PATH- vagy plugin-módosítás, root művelet, AppArmor- vagy sysctl-változtatás jóváhagyás nélkül. Mérésre és sandboxra is vonatkozik.

**A7 – FFmpeg elsőbbség.** Az FFmpeg 9.0.2 az export-orákulum és az egyetlen betöltött libav ABI. Másik libav (pl. wheel-be csomagolt) a fő folyamatba nem tölthető be.

**A8 – Qt6 izoláció.** A GUI főszálán nincs hosszú művelet; progress/cancel/log a közös job API-n (C0). A meglévő Qt6 paritásjavítások nem vethetők vissza.

**A9 – Licenc és provenance.** Átvett kódhoz eredeti commit, licenc, tranzitív függőségek, attribúció, SBOM, licenckompatibilitási mátrix a CFA4 licencével, FFmpeg `configure` flagek rögzítése. Licenc-review nélkül nincs merge.

**A10 – Reprodukálhatóság.** Bemeneti hash, beállítások, verziók, GPU driver, kernel, display-szerver, kodek, seed és render graph hash minden benchmark és release teszt mellett.

**A11 – Minimális Python-verziószám.** A CFA4 a lehető legkevesebb Python verzióval dolgozik. Alapértelmezés az Ubuntu 26.04 rendszer-Pythonja (verzió PENDING). További verzió akkor vehető fel, ha egy jóváhagyott komponens igazoltan nem fut az alapértelmezetten; ezt az F1-ben induló Python-nyilvántartás rögzíti indoklással. Workerenként külön venv megengedett, de csak nyilvántartott verzión; conda/mamba nem használható. A fő alkalmazás Python nélkül is teljes. Ahol a Python funkcióvesztés nélkül kiváltható, ott a kiváltás az első választás; a VapourSynth retusáló szkriptjei indokolt Python-igénynek számítanak.

**A12 – Meglévő terv vagy munka nem készül el újra (2026-11-21 döntés).** Ha egy funkcióra máshol már létezik terv, ADR, issue, PR vagy kód a CFA4-ben, ez a terv arra hivatkozik, és csak azt vizsgálja meg, hogy a hiányzó részt lefedi-e. Új terv vagy implementáció csak a bizonyított hiányra készül, a meglévő tulajdonos tudtaval. Egy funkcióhoz egyszerre csak egy főterv és egy főimplementáció lehet. Az átfedés feloldhatóságát az F0a útján rögzítjük (`F0-existing-plans-inventory.md`); amit nem sikerül ellenőrizni, PENDING, és nem indul rajta újírás.

### MQ – Mozi-minőségi kapu

Az MQ kapu: bármely FAIL kizárja a jelöltet a képláncból, és blokkolja a mozi célú kiadást. A képet nem érintő eszközöknél (pl. gráfalgoritmus) a feltétel a bit-azonos továbbítás.

| ID | Kritérium | PASS definíció |
| --- | --- | --- |
| MQ-1 | Belső pontosság | Legalább 16 bites lebegőpontos vagy 16 bites egész feldolgozás; nincs rejtett 8 bites köztes lépés. |
| MQ-2 | Kroma | 4:4:4 és RGB alulmintavételezés nélkül átmegy. |
| MQ-3 | Színkezelés | Explicit színtér-kezelés (OpenColorIO vagy azzal egyező CFA4 színszerződés); DCI-P3, Rec.2020, PQ/HLG, X'Y'Z' konverzió hibája Küszöb@F0 alatt. |
| MQ-4 | Alfa | Straight/premultiplied megkülönböztetve; nincs peremhiba. |
| MQ-5 | Felbontás, képkockasebesség | 2K DCI (2048×1080), 4K DCI (4096×2160); 23,976, 24, 25, 48 fps képkockapontosan. |
| MQ-6 | Determinizmus | CPU-n bit-azonos kimenet; GPU-n dokumentált, küszöb alatti eltérés. |
| MQ-7 | Hang | 24 bit, 48 és 96 kHz, 5.1 és 7.1, mintapontos szinkron. |
| MQ-8 | Skálázás, blend | Lineáris fényben végzett blend és skálázás választható; megnevezett, rögzített szűrő. |

**Mozi kimenet (DCP):** a DCP képe JPEG 2000, X'Y'Z', 12 bit. Ezért a J2K orákulum-rés mozi célú kiadásnál blokkoló. A DCP-csomagot a CFA4 maga állítja elő (döntés: 2026-11-21, F5D), független validálással.

## 5. Modulhatárok és gazda fázisok

Minden modulnak pontosan egy fázis a gazdája, amely az első használható változatát szállítja. A nevek javaslatok; F0a után a tényleges repo-struktúrához igazodnak, és ha a CFA4-ben már létezik megfelelő modul, azt kell bővíteni, nem újat létrehozni.

| Modul | Felelősség | Gazda fázis | Továbbfejlesztő fázisok |
| --- | --- | --- | --- |
| cfa4-render-validation | golden frame-ek, kép-/hang-/időzítési metrikák, MQ mérés, riport | F0V | minden fázis; F9 teljes |
| cfa4-render-contracts | Frame/Audio ABI, időmodell, hiba- és capability modell, worker protokoll | C0 | F3, F4 (csak ADR-rel) |
| cfa4-render-job-service | Qt6/CLI közös job, queue, progress, cancel, diagnosztika | C0 | F3, F8 |
| cfa4-render-graph | timeline/effekt szándék → validált DAG, invalidáció | F3 | F5 |
| cfa4-render-scheduler | RAM/VRAM budget, prefetch, cache, resume | F3 | F4 |
| cfa4-render-backend-cpu | kötelező referencia backend, MQ-6 bit-azonos | F3 | F5 |
| cfa4-render-backend-gpu | meglévő Vulkan/CUDA út, opcionális wgpu | F4 | – |
| cfa4-imageio | EXR, képszekvencia, metaadat (OpenImageIO vagy meglévő) | F5 | F6 |
| cfa4-render-adapter-vfx | kompozíciós végrehajtás | F5 | – |
| cfa4-render-adapter-procedural | címek, SVG, mozgógrafika | F6 | – |
| cfa4-render-adapter-filter | VapourSynth worker | F6 | – |

**Kötelező elválasztás:** scheduler ≠ képkockakompozitáló ≠ kódoló ≠ Qt6 GUI. Egy funkcióhoz egyszerre csak egy aktív főimplementáció lehet; az új út feature flag mögött él, amíg az F9 kapu nem engedélyezi az alapértelmezett bekapcsolást.

## 6. Fázisok I. – Tényfeltárás és audit

Minden fázis egységes sablont követ. A kilépési feltételek mind kötelezők; egy FAIL esetén a fázis nem zárul le, és a következő nem indulhat. Ha egy fázis időkerete 50%-kal túllépésre kerül, döntési pont következik: folytatás, szűkítés vagy leállítás.

### F0a – Gépi tényfeltárás (blokkoló)

**Cél:** a CFA4 jelenlegi állapotának reprodukálható rögzítése, hogy minden későbbi döntés tényre épüljön. **Időkeret:** 3 munkanap. **Előfeltétel:** a terv 1. és 2. döntésének jóváhagyása; olvasási hozzáférés a repo-hoz és a CI-hoz.

**Feladatok**

1. HEAD SHA, aktív ág, release cél, lockfile-ok (Cargo.lock és egyéb) rögzítése.
2. Toolchain: Rust verzió (`rust-toolchain.toml`), MSRV, FFmpeg verzió és `configure` flagek, Qt6 verzió.
3. Környezet: kernel, NVIDIA driver, Vulkan/CUDA verzió, display-szerver (Wayland/X11), rendszer-Python verzió.
4. Render call graph: renderer, timeline, render queue, proxy, preview, audio, GPU útvonalak; meglévő frame-, idő- és job-típusok térképe.
5. Exportutak listája kodekenként; CI-jobok és utolsó eredményeik.
6. A hivatkozott rögzítési terv és kodeksorrend dokumentum-ID-jének rögzítése.

**Átadandók:** `F0-render-baseline.md`, `F0-call-graph` (géppel generált), `F0-environment.md`.

**További F0a átadandó (A12):** `F0-existing-plans-inventory.md` – a CFA4 minden meglevő terve, ADR-je, nyitott issue-ja, PR-je és modulja, amely e terv témáit érinti (render, DCP, felirat, OFX, validáció, job/queue, capture, Qt6 ütemezés), tulajdonossal és státusszal; minden e terv szerinti fázishoz jelölve: LEFEDETT (hivatkozás, nincs új munka), RÉSZBEN (csak a hiány) vagy NINCS (új munka). Kilépési feltétel: az S1 előtt minden fázis kap egy ilyen minősítést.

**Kilépési feltételek:** a 2. fejezet minden PENDING sorához igazolt érték vagy indokolt PENDING; a call graph újragenerálható egy paranccsal; másik gépen ugyanaz a SHA ugyanazt a lockfile-hash-t adja.

**Kockázat és visszavonás:** csak olvasó művelet, nincs visszavonandó változás.

### F0b és F0V – Tesztbizonyíték és validációs alap (blokkoló F2-re)

**Cél:** a jelenlegi renderer viselkedésének gépi rögzítése és a cfa4-render-validation v0 szállítása, amelyen minden későbbi kapu mér. **Időkeret:** 5–8 munkanap, F1-gyel párhuzamosan. **Előfeltétel:** F0a kész; licenctiszta tesztanyag.

**Feladatok**

1. PR #271/#331 tesztjeinek állapota; Qt6 funkcionális akadályok listája; R0 NVENC/NVDEC újrafuttatása HEAD-en.
2. Referenciacsomag: 1080p25 8-bit SDR; 4K DCI 24 fps 12-bit DCI-P3; 4K30 10-bit PQ; VFR forgatott mobilfelvétel; alfás klip és EXR szekvencia; 5.1 és 7.1 hang 24 bit/48 kHz; szinkronfixture (villanás + sípszó); retusálási fixtures: interlaced, telecine-elt, zajos és sávosodó anyag. Manifest: hash, forrás, licenc.
3. Golden frame-ek a jelenlegi rendererből: SDR 16 bites PNG, HDR/alfa/DCI EXR, képkockánkénti hash; frissítés csak review-zott változtatással.
4. Ismert eltérések listája (J2K, HEIC, phone\_vfr és az F0 által talált hibák), hogy a golden ne rögzítse a hibát elvárt eredményként.
5. Metrikák: hash, PSNR, SSIM síkonként (FFmpeg saját szűrőivel); színeltolás (ΔE 2000) színpróba-képkockákon; hang null-teszt dBFS-ben; PTS/DTS diff; A/V drift; seek-pontosság.
6. Erőforrás-mérés root nélkül (`/proc`, NVML); Qt6 főszál-watchdog interfész; coexistence pillanatkép.
7. Riport JSON + Markdown az A10 metaadatokkal.
8. Küszöb@F0: változatlan HEAD-en ≥ 10 futás, a mérési zajból származtatott, dokumentált küszöbök minden számszerű AC-hez.
9. CI szintek definíciója: smoke (PR, CPU-only), nightly (teljes CPU + MQ), hardware (GPU). Bekapcsolás a fázis jóváhagyásával.
10. Mutációs önteszt: kieső képkocka, BT.601/709 mátrixcsere, 1 képkockás PTS-eltolás, alfa elvesztése, 8 bitre kerekítés, egy némított hangcsatorna.

**Átadandók:** cfa4-render-validation v0, `F0-capability-gap-matrix.csv`, `F0-test-evidence.md`, `F0V-fixtures-manifest`, golden halmaz, `F0V-thresholds.md` + gépi küszöbfájl, `F0V-validation-report.md`, CI job-definíciók.

**Kilépési feltételek**

- A harness egy paranccsal fut tiszta Ubuntu 26.04/amd64 gépen, GPU és root jog nélkül.
- A 10. pont mutációinak 100%-át FAIL-ként jelzi; változatlan HEAD-en 10 futásból 0 hamis FAIL.
- Minden AC-hez (AC-01–AC-20) van mérő vagy indokolt PENDING.
- A gap-mátrix minden tervezett képességhez bizonyított GAP-et, meglévő funkciót vagy mérhető optimalizálási célt rendel.
- Ha egy területen nincs GAP és nincs mérhető cél, az ottani jelöltek automatikusan kiesnek.

**Kockázat és visszavonás:** a golden beégetheti a hibákat (védelem: ismert eltérések listája); a fixtures mérete (külön tárolás, módja PENDING). A harness terméktől független, eltávolítása nem okoz regressziót.

### F1 – Jogi, biztonsági és komponens-audit

**Cél:** a POC-jelöltek és a külső munkafolyamatok jogi, biztonsági, érettségi és host-lábnyom szerinti előszűrése. **Időkeret:** 5 munkanap, F0a után. **Előfeltétel:** F0a (toolchain, FFmpeg flagek, Python verzió).

**Feladatok jelöltenként** (OpenImageIO, VapourSynth, ReelForge, avio ff-\*, EC-jelölt (#11), rust-motion, Blender, valamint a referenciák licence)

1. Pinelt commit vagy kiadás, licenc, tranzitív függőségek, SBOM-tervezet, licenckompatibilitás a CFA4 licencével.
2. Upstream egészség: utolsó 12 hónap kiadásai, aktív karbantartók száma, CI, issue-reakció. Küszöb: legalább egy kiadás és 2 aktív karbantartó, különben legfeljebb ADAPT\_PATTERN\_ONLY.
3. MSRV vs. CFA4 toolchain (ReelForge 1.97+ vs. 1.95 PENDING); toolchain-emelés csak projektlokálisan és külön jóváhagyással.
4. FFmpeg API/ABI kompatibilitás 9.0.2-vel; második libav tilalma (A7).
5. Claim-teszt: minden átvenni kívánt README-állításhoz upstream teszt vagy saját reprodukálás.
6. MQ előszűrés dokumentáció és kód alapján (belső pixelformátumok, színkezelés, alfa).
7. Host-lábnyom: szükséges rendszercsomagok, Python-igény; a VapourSynth retusáláshoz szükséges plugin- és szkriptkészlete (pl. QTGMC, BM3D, deband, IVTC) és ezek licencei; a Python-nyilvántartás indítása (verzió, igénylő komponens, indoklás) az A11 szerint.
8. Sandbox-technológia root nélkül (`systemd-run --user` + cgroup limitek, seccomp, Landlock); az Ubuntu AppArmor user-namespace korlátozás hatása 26.04-en.
9. rust-motion és Dioxuscut kódátfedése; EC-jelölt (#11) érettségi állításai.

**Átadandók:** `F1-source-license-dependency-audit.md`, `F1-risk-register.csv`, `F1-host-footprint.md`, `F1-sandbox-decision.md`.

**Kilépési feltételek:** minden jelölt PASS / REFERENCE\_ONLY / BLOCKED besorolást kap indoklással; minden BLOCKED-hoz megnevezett blokkoló; a sandbox-döntés nem igényel host-módosítást, vagy a szükséges módosítás jóváhagyásra van készítve; a VapourSynth Python-kérdés eldöntve.

**Kockázat és visszavonás:** csak olvasó elemzés; jelölt nem kerül a CFA4-be.

## 7. Fázisok II. – Szűkítés, szerződések, POC, döntés

### S1 – Szűkítés (döntési pont)

**Cél:** a 3. fejezet előzetes besorolásának véglegesítése F0 és F1 bizonyítékai alapján. **Időkeret:** 1 munkanap. **Előfeltétel:** F0b/F0V és F1 lezárva.

**Szabályok:** (1) MQ előszűrés FAIL → kiesik a képláncból; (2) F1 BLOCKED → kiesik; (3) nincs bizonyított GAP vagy mérhető cél → kiesik; (4) ha új, még nem nyilvántartott Python verziót igényel → csak indoklással és jóváhagyással maradhat; (5) F2-be legfeljebb 6 jelölt.

**Átadandó:** `S1-shortlist.md` jelöltenkénti indoklással. **Kilépés:** a kapu-jóváhagyó aláírt döntése.

### C0 – Szerződések és job API v0 (blokkoló F2-re)

**Cél:** a renderer, az adapterek, a Qt6 és a CLI közös szerződésének rögzítése, és annak bizonyítása, hogy a meglévő renderer regresszió nélkül fut rajta. **Időkeret:** 5–8 munkanap. **Előfeltétel:** S1, F0V harness, F0a típustérkép. **Alapelv:** a meglévő CFA4 típusok formalizálása és kiegészítése; párhuzamos típusrendszer nem keletkezhet.

**Feladatok – cfa4-render-contracts**

1. Frame ABI v0 az A2 teljes mezőkészletével; ownership modell (saját, kölcsönzött, megosztott), CPU/GPU hely, szinkronizációs absztrakció.
2. Konverziós mátrix: formátum- és helypárok, végrehajtó, másolási költség; nem támogatott pár = explicit hiba.
3. Audio ABI v0 (A3); időmodell: racionális időalap, timeline → média PTS, VFR-szabály csöndes CFR-kerekítés nélkül.
4. Hiba- és capability modell; az A4 szabálytábla.
5. Worker protokoll v0: külön folyamat, verzió-handshake, képkocka-csere (megosztott memória vagy fájl), timeout, erőforrás-limit, cancel, útvonal-korlátozás.
6. Capture I/O interfész (a v1.0 F7 interfészrésze), implementáció nélkül.

**Feladatok – cfa4-render-job-service (a C0 leltár szerint a cfa4-tasks render-osztályú kliense, nincs saját sor és állapotgép)**

1. Állapotgép: Queued → Running → Completed / Failed / Cancelled, Cancelling köztes állapottal; tiltott átmenetek.
2. Események: progress (képkockaindex, %, ETA), log, figyelmeztetés, fallback-jelzés.
3. Kooperatív cancel mért latenciával; atomikus kimenet (ideiglenes név + rename).
4. Retry csak idempotens lépésre; prioritás: preview előzi az exportot.
5. Szálmodell: a hívó szálon semmi nem blokkol; Qt6 és CLI ugyanazt az API-t használja.
6. Egyetlen feature flag / backendválasztó mechanizmus, alapértelmezés a meglévő renderer.
7. Vékony adapter a meglévő rendererhez és CLI kliens a job API-n.

**Változáskezelés:** szemantikus verziózás; befagyasztás után törő változás csak ADR-rel és verzióemeléssel; szerződésteszt minden változásra.

**Átadandók:** `C0-contracts-spec.md` ADR-ekkel; cfa4-render-contracts v0; cfa4-render-job-service v0 + CLI; A4 szabálytábla; konverziós mátrix; `C0-candidate-mapping.md`; Qt6 integrációs vázlat.

**Kilépési feltételek**

- A meglévő renderer a szerződésen keresztül bit-azonos a golden halmazzal; PTS-diff 0.
- Cancel latencia ≤ Küszöb@F0; 20 véletlen idejű cancel/SIGKILL után végleges néven 0 csonka fájl.
- Minden S1 jelölt leképezhető, vagy BLOCKED, vagy a szerződés ADR-rel módosul még F2 előtt.
- A Qt6 fejlesztés felelőse írásban elfogadja a job API-t.
- Nincs új külső függőség, vagy F1-ben PASS.

**Kockázat és visszavonás:** túltervezés (csak meglévő és S1 igények); az adapter feature flaggel kikapcsolható, a régi út változatlan.

### F2 – Összehasonlító POC (izolált tesztág)

**Cél:** a POC-jelöltek mért előnyének vagy hátrányának bizonyítása a CFA4 baseline-hoz képest, a C0 szerződésen. **Időkeret:** 10 munkanap. **Előfeltétel:** C0 lezárva.

**Feladatok**

1. Előregisztráció F2 indulása előtt, jelöltenként: hipotézis, metrika, küszöb, futásszám (`F2-preregistration.md`, utólag nem módosítható).
2. MQ igazolás az első 3 munkanapon (az S1 után csak az EC-jelölt feltételes); FAIL esetén a jelölt kilép a POC-ból.
3. Közös próba: egy referenciaklip + 3 rétegű, időben változó effektgráf, CPU-n és a meglévő GPU úton (új GPU backend nélkül).
4. Jelöltenkénti mérés: OpenImageIO (EXR I/O pontosság, metaadat, sebesség); VapourSynth (retusáló szűrők: deinterlace, zajcsökkentés, debanding, IVTC minősége az FFmpeg szűrőihez képest; szűrő round-trip); ReelForge (DAG, dirty-region, resume helyessége és gyorsulása); avio (ff-render/ff-preview/ff-pipeline vs. baseline); EC-jelölt (#11) (kompozíció MQ-1/3/4/8); rust-motion (címek, SVG, alfa).
5. Erőforrás- és másolási költség (CPU↔GPU, Frame ABI konverzió).

**Átadandók:** `F2-preregistration.md`, `F2-poc-comparison.md`, géppel futtatható benchmarkjegyzőkönyv, reprodukálható fixtures.

**Kilépési feltételek (jelöltenként)**

- MQ PASS és helyesség nem rosszabb a baseline-nál (AC-04, AC-05).
- Legalább egy előregisztrált előny, ahol a bootstrap 95% konfidenciaintervallum alsó határa is meghaladja a küszöböt (≥ 10 futás).
- A POC kód nem kerül a fő ágba.

**Kockázat és visszavonás:** a POC izolált ágon fut, törlése nem érinti a CFA4-et.

### D1 – Integrációs döntés

**Cél:** jelöltenként egyetlen döntés: INTEGRATE\_COMPONENT, ADAPT\_PATTERN\_ONLY, EXTERNAL\_ADAPTER, RESEARCH\_ONLY vagy REJECT. **Időkeret:** 1–2 munkanap.

| Szempont | Súly | Mérce |
| --- | --- | --- |
| MQ PASS | kapu | bármely FAIL → REJECT a képláncra |
| Helyesség a baseline-hoz | kapu | AC-04/05 rosszabb → REJECT |
| Licenc és provenance | kapu | A9 PASS nélkül BLOCKED |
| A11 Python | kapu | új, nem nyilvántartott Python verzió → csak indoklással és jóváhagyással |
| Mért előny | 30% | előregisztrált metrika, CI alsó határ |
| Karbantartási költség | 20% | éves becsült munkanap |
| Integrációs felület | 20% | érintett C0 szerződések, konverziós költség |
| Upstream egészség | 15% | kiadások, karbantartók |
| Host-lábnyom (A6) | 15% | rendszercsomagok, izolálhatóság |

**Átadandó:** `D1-decision-record.md` jelöltenként bizonyítékkal és aláírással. **Kilépés:** minden jelöltnek pontosan egy döntése van, és a F3–F6 tartalma ehhez igazodik.

## 8. Fázisok III. – Render graph, GPU, VFX és kép-I/O

Az implementációs fázisok időkerete kezdeti plafon; pontos becslés a D1 után készül, és nem lépheti túl a plafont jóváhagyás nélkül. Minden új út feature flag mögött él, alapértelmezés a régi renderer.

### F3 – Render graph, scheduler, CPU backend és audio

**Cél:** CFA4 timeline → validált DAG, erőforrás-kezelt ütemezés, megszakítható és folytatható render, bit-azonos CPU referencia, audio renderút. **Plafon:** 30 munkanap. **Előfeltétel:** D1 (a ReelForge-döntés az S1-ben megszűnt: az F3 render graph a CFA4 saját megvalósítása, a ReelForge csak mintának), C0 v0.

**Feladatok**

1. Timeline → DAG fordítás, időbeli függőségek, hash-alapú invalidáció (render graph hash az A10-hez).
2. Scheduler: bounded queue, RAM-budget és VRAM-budget interfész (a GPU-oldal F4), prefetch, cache eviction, preview-prioritás, cancellation a job API-n.
3. Resume checkpoint sérülésvizsgálattal; részleges újrarenderelés csak valid hash-egyezés mellett.
4. CPU backend: MQ-1 (≥ 16 bites belső ábrázolás), MQ-6 (bit-azonos), MQ-8 (lineáris fényű blend).
5. Audio render: keverés, resampling, csatornaelrendezés-konverzió, több hangsáv, mintapontos szinkron (MQ-7).
6. Projektfájl-sémaverzió és migráció az új mezőkhöz.
7. Hibainjektálás: kimerülő memória, sérült checkpoint, SIGKILL, lassú I/O.

**Átadandók:** kód + tesztek + profil; `F3-fault-injection.md`; backout terv.

**Kilépési feltételek:** AC-03, AC-05, AC-07 (RAM), AC-08, AC-13, AC-15 PASS; a teljes golden halmaz bit-azonos a CPU backenden; nincs képkockahiány, sorrend- vagy A/V hiba; a régi renderer egy kapcsolóval visszaállítható.

**Kockázat és visszavonás:** a cache hibás érvényessége hamis kimenetet adhat (védelem: hash-ellenőrzés, cache kikapcsolásával is PASS kell); feature flag ki = régi út.

### F4 – GPU kompozitálás és színhelyesség

**Cél:** GPU-gyorsítás a CPU referenciával egyező eredménnyel. **Plafon:** 25 munkanap. **Előfeltétel:** D1; C0 fence/VRAM mezők; F3-mal párhuzamos, ha a scheduler VRAM-interfésze rögzített.

**Feladatok**

1. Meglévő Vulkan/CUDA/libplacebo út előnyben; wgpu csak F2-ben mért előnnyel.
2. RGBA/YUV, 10/12/16 bit, HDR, alfa út; transfer- és mátrixegyezés; GPU fence, DMA és másolás-követés.
3. VRAM-budget a scheduleren keresztül; VRAM-kimerüléskor A4 szerinti viselkedés.
4. Kijelző- és compute-GPU választása több-GPU-s NVIDIA gépen.
5. CPU/GPU paritás a golden halmazon és a DCI fixture-ön.

**Átadandók:** GPU capability matrix, `F4-cpu-gpu-parity.md`.

**Kilépési feltételek:** AC-04 (GPU-tolerancia), AC-06, AC-07 (VRAM), AC-18 GPU-n PASS; GPU letiltásakor minden export CPU-n PASS.

**Kockázat és visszavonás:** driver-függő eltérések (védelem: A10 rögzítés, hardware CI-szint); GPU út flaggel kikapcsolható.

### F5 – VFX kompozíció és professzionális kép-I/O

**Cél:** réteges kompozíció és mastering-szintű képszekvencia I/O a közös CFA4 timeline- és színszerződésen. **Plafon:** 20 munkanap. **Előfeltétel:** F3 és F4 kilépés; D1 (EC-jelölt (#11), OpenImageIO).

**Feladatok**

1. Kompozíciós modell: layer, keyframe, blend, mask, a D1 szerinti formában (komponens vagy csak minta); külön kodek stack átvétele nélkül.
2. cfa4-imageio: EXR (half/float), alfa, bitmélység, metaadat, sequence range; OpenImageIO vagy meglévő natív I/O a D1 szerint.
3. Színkezelés: OpenColorIO konfiguráció vagy azzal egyező CFA4 színszerződés; DCI-P3, Rec.2020, X'Y'Z' konverziók.
4. Blender külső munkafolyamat: EXR szekvencia export/import, verzió pinelve, külön folyamatban.
5. Natron/OFX: az ADR 0003 A4 viszi (F5O hivatkozás), az ADR 0003 §14 szerint a Natron csak képesség-referencia.

**Átadandók:** `F5-vfx-adapter.md`, `F5-imageio-gap-tests.md`, `F5-color-pipeline.md`.

**Kilépési feltételek:** AC-04 (EXR/alfa), AC-14, AC-18 PASS; headless export PASS; a kompozíció nem hoz létre második projektmodellt (A1).

**Kockázat és visszavonás:** alfa- és HDR-regresszió (védelem: golden EXR, MQ-4); az adapter flaggel kikapcsolható, a meglévő kép-I/O megmarad.

### F5S – Feliratok: hivatkozás a meglevő tervekre (A12)

**Állapot (ellenőrizve 2026-11-21, `main` @ 5d5cd4c): LEFEDETT.** Jóváhagyott terv: SUBTITLE-STUDIO-PLAN-001 FINAL 1.2 (2026-10-30), CFA4-PROCODEC-002 10. munkacsomag (DCP-felirat ST 428-7, IMF IMSC1), CFA4-DCPOMATIC-001 D5 (feliratok a 3. fázisban). Kód: `crates/captions` (SRT, WebVTT, SCC, MCC, EBU STL, TTML/IMSC1/DFXP import és export, D-Cinema ST 428-7 import, kiosztás, begetés) és `crates/text` (betűkészlet, szövegformázás, bidi). Bizonyított hiány (CINECERT-WP00): az ST 428-7 írás és az ST 429-5 timed text MXF írás.

**A render-terv teendője:** nincs új fejlesztés. Az AC-17 mérése a meglévő golden-keretben (`cfa4-golden`); a megmaradó hiányt a fenti tervek gazdája zárja. **Kilépés:** AC-17 PASS (CPU-n bit-azonos begetés a golden halmazzal; a sáv megmarad exportnál; DCP-ben érvényes timed text, ha a hiány addigra zárult).

**F5S bővítés (István, 2026-11-21): minden nyelv, minden elérhető feliratformátum.** A CFA4 nemzetközi, importálás és export bárhonnan történhet, ezért a cél a teljes kör. A részletes terv a SUBTITLE-STUDIO-PLAN-001 FINAL 1.2 (A12), ezt a négy hiányt javasoljuk a gazdának:

- **Formátum-nyilvántartás:** minden formátumra olvasás, írás és beágyazás státusz, valamint kifejezett lista arról, mi nem támogatott és miért. A ma meglevő hét formátum (SRT, WebVTT, SCC, MCC, EBU STL, TTML/IMSC1, DFXP) mellett az ASS/SSA az első új (F2), utána iTT, PGS, VobSub, OCR, Teletext, 608/708 beágyazás.
- **Nyelvek:** a szöveg Unicode-teljes (jobbról balra írás, arab és héber ligatúrkák, CJK, indiai írások, függőleges írás), minden írásrendszerhez betűkészlet-tartalék, és írásrendszerenként egy tesztfelirat a golden halmazban; a sáv nyelve BCP 47, az írásrendszer ISO 15924.
- **Mozi, ST 428-7:** a DCDM Subtitle 2007, 2010 és 2014-es változata külön névtér (smpte-ra.org/schemas/428-7/2007/DCST, 2010/DCST). Olvasás mindhárom változatból, írás a 2014-es változatból (új: Zposition és LoadVariableZ a sztereo feliratokhoz, Direction "hor", EffectSize, Feather, tisztázott Ruby szöveg), és külön az Interop felirat. A PNG-s (képi) és a szöveges felirat külön eset. A ST 428-10 a zárt felirat (closed caption) és zárt subtitle korátait adja.
- **Képi formátumok (PGS, VobSub):** olvasás OCR-ral; írásuk csak ha a licenc és a minőség enged (PENDING).

Megállapítás: ezek a Feliratozó projekt tulajdonosa felé érkező kérések, a render-terv nem írja felül az 1.2 tervet. A kilépés: AC-17 és a formátum-nyilvántartás minden sora státuszban.

### F5D – DCP: hivatkozás a meglevő tervekre (A12)

**Állapot (ellenőrizve 2026-11-21): LEFEDETT/RÉSZBEN.**

- Saját SMPTE OV export: `crates/export/src/dcp_out.rs` (JPEG 2000 12 bit X'Y'Z', DCI 2K/4K, 5.1 és 16 csatornás PCM, IAB, CPL/PKL/ASSETMAP/VOLINDEX, SHA-1, aláírás és titkosítás nélkül).
- Olvasás és validáció: `cfa4-dcp` (SMPTE és Interop), `cfa4-dcp-validate`, `cfa4-dcp-inspect`, `cfa4-cinema-pkg`, `cfa4-cinema-acceptance`.
- DCP-o-matic: `cfa4-dcpomatic` és CFA4-DCPOMATIC-001 v1.0 (jóváhagyva 2026-11-13): a CFA4-gyel szállított példány külön folyamatban készít Interop, 3D, feliratos és titkosított DCP-t (D4: titkosítás igen), kizárólag a CFA4 részeként.
- CFA4-DCPVAL-LIBDCP-001 (jóváhagyva 2026-11-14): két ismert eltérés (Interop ASSETMAP elemsorrend, J2K MCT).
- CFA4-CINECERT-001 v2.3 (jóváhagyva 2026-11-13): az orákulum-környezet (asdcplib, Photon, Clairmeta) telepítője kész; a CineCert saját része: MXF-digest, ISDCF-név, átadás és archíválás, külső validátoros elfogadási tesztek.

A "CFA4 maga készítsen teljes DCP-csomagot" döntést ez a felosztás teljesíti: saját SMPTE-export alapértelmezésként, mellette a CFA4 részeként szállított DCP-o-matic. **A render-terv teendője:** nincs új DCP-fejlesztés; az AC-19 a meglévő validátorokra (`cfa4-dcp-validate`, `dcp_inspect`, libdcp `dcpverify`, a CINECERT orákulumai) épül. **Eldöntött (DCPOMATIC D4: István 2026-11-13; CINECERT D3: M4-ben; az ADR 0011 jóváhagyásra vár) (`coordination.md` §2.8):** a PROCODEC-002 "titkosítás/KDM nem" állítása elavult, a 2026-11-13-i "igen" döntés felülírja. Sorrend: előbb a DCP-o-matic külön folyamatban, később a saját clean-room Rust megvalósítás (CINECERT WP10).

### F5O – OFX: hivatkozás az ADR 0003 A4-re (A12)

Állapot (ellenőrizve 2026-11-21, ADR 0003 §14 szerint): TERV VAN, KÓD NINCS. ADR 0003 (jóváhagyva 2026-10-24) A4 szakasz: OpenFX-host külön folyamatban (keying, színkorrekció, transzformáció, pontkövetés, retime, deinterlace, zajszűrés képességek). A §14 (István, 2026-11-21) felülírja a csomag forrását: az openfx-misc (GPL-2.0-or-later) csak képesség-referencia, az első OFX-csomag clean-room, a Natron orákulumnak sem használható (Qt4 összeomlik Kubuntu 26.04-en); az orákulum a golden fájlok, az OCIO és a meglévő orákulumok. Az A4 elfogadása ennek megfelelően módosul: a csomag betöltődik és renderel, plugin-összeomlás nem viszi el a programot; a PSNR-hivatkozás Natron-referenciára elavult. A CFA4-SECURITY-001 D7 és a threat-model.md B7 külön pluginHost folyamatot ír elő, a crates/sandbox profilja megvan. A docs/gaps.md G11 szerint plugin-hosting még nincs.

**Qt- és licenc-döntés (eldöntve, István 2026-11-21, ADR 0003 §14):** a Natron kódja korábbi Qt-ra épül, ezért nem kompatibilis, és orákulumnak sem használható. Az openfx-misc csak képesség-referencia. Az előzetes upstream-nézet (README és Makefile.master nem említ Qt-t, GPL v2+, függőségek CImg, pocketfft/FFTW, OSMesa/LLVM) csak tájékoztató, a döntést nem változtatja; a teljes forrásfa-ellenőrzés nem szükséges.

**A render-terv teendője:** nincs saját OFX-fázis; az ADR 0003 A4 és a G11 viszi. Az AC-20 az ADR §8 A4 kritériumaira (betöltés, PSNR, plugin-összeomlás elszigetelése) épül, kiegészítve az MQ-1 jelöléssel (8 bites OFX út mozi láncban nem használható).

**F5O bővítés (István, 2026-11-21): minden OFX plugin megvalósítása lehetőség szerint.** Az ADR 0003 A4 keretei között két útra bontjuk:

- **Betöltés:** a CFA4 OFX-hostja minden szabványos OFX-pluginnal működjön. Az első OFX-csomag clean-room, az openfx-misc csak képesség-referencia (ADR 0003 §14).
- **Újraimplementálás:** ahol a plugin a host egy hiányzó képességére ép (maszkszerkesztés, OpenGL, idővonal-lekérdezés), a képességet a hostban valósítjuk meg, a plugin vagy a Natron kódját nem másoljuk.
- **Plugin-kompatibilitási mátrix:** minden plugin státusza: fut, részben fut (mit hiányol), nem fut (miért). Az F0a leltár rögzíti az első változatot, az F2 POC méri.
- **MQ-kapu:** a plugin csak akkor mozi-minőségű, ha 16 bites lebegőpontos pontosságon és a színkezeléssel is megfelel (MQ-1, MQ-3); a többit jelöljük.
- **Kilépés (AC-20 kiegészítés):** a mátrix minden sorának van státusza, az állása Küszöb@F0 szerinti arányban fut, a nem futó pluginok oka dokumentált.

Feltétel: az ADR 0003 A4 elfogadási kritériumai változatlanok (betöltés, PSNR a referenciához, plugin-összeomlás elszigetelése).

### SMPTE ST 428 szabványcsomag (István feltöltötte, 2026-11-21)

A feltöltött dokumentumokat átnéztem (a szöveget nem másolom a repo-ba, csak hivatkozom). A legfrissebb kiadás irányadó, a régebbi csak örökölt fájlok olvasásához kell.

- **ST 428-1:2019 (DCDM képjellemzők, felülírja a 2006-ot):** működési szintek a legnagyobb képpontszám és képkockasebesség szerint (ellenőrizve: 1. szint 4096×2160 24 fps, 2. szint 2048×1080 48 fps, 3. szint 2048×1080 24 fps, 4. szint 4096×2160 25 fps, 5. szint 4096×2160 30 fps, 6. szint 2048×1080 25 fps; a többi szint PENDING). Ez az MQ-5 (2K/4K DCI) mérőszáma.
- **ST 428-2:2006 (hang):** bitmélység, mintavételi frekvencia, minimum csatornaszám és referenciaszint; a pontos értékek ellenőrzése az F0a-ban PENDING, nem olvastam ki.
- **ST 428-3:2006 VISSZAVONVA (2017-09-28):** nem implementáljuk. A csatornakiosztást az ST 377-4 (MXF többcsatornás hangcímkézés) és az ST 428-12 (közös hangcsatornák és soundfield csoportok, címkék) írja elő. Az MQ-7 csatornakiosztása csak ezekre épülhet.
- **ST 428-7 (felirat):** 2007, 2010, 2014 kiadás; lásd F5S bővítés. **ST 428-10:** zárt felirat és zárt subtitle.
- **ST 428-11:2013 és ST 428-21:2011 (további és archív képkockasebességek):** a 24 és 48 fps mellé 25, 30, 50, 60, 96, 100 és 120 fps, plusz archív sebességek. A CFA4 időmodelljének (cfa4-time) tesztelnie kell ezeket, külön a képkockasebesség-átalakítást a DCP-ben.
- **ST 428-9 és ST 428-19 (SDI-jelformázás):** hardverfelület, SDI-hardver nélkül nem tesztelhető; a render-terven kívül.
- **ST 428-24:2024 (csomagolt kép, packed image):** veszteségmentes JPEG 2000 (a ITU-T T.814 gyors blokk-kódolóval is), a 12 bites kódértékek 16 bites tároló nélkül, kb. 50% méretcsökkentéssel, fájlsorozatként az RP 428-5 szerint, digitális leaderrel. Ez nem a DCP-be kerülő vesztes J2K. Jelölt: köztes és archív DCDM-formátum (MQ-6 determinizmus, MQ-2). A csomag mintafájljait (TIFF, Kakadu- és OpenJPH-kódolt .j2c) tesztvektornak javaslom, a licenc ellenőrzése PENDING.

**Mozis szerver hiánya (István, 2026-11-21):** nincs mozis szerver és nincs hozzáférés. Ezért az AC-18 és az AC-19 csak szoftveres referenciákon dönt (libdcp/DCP-o-matic validáció, asdcp-test, dcp.loadKdm tesztanúsítvánnyal, J2K orákulum). A CFA4 nem állít mozi-kompatibilitást: szabványnak megfelelő és szoftveresen ellenőrzött, valós mozis szerveren nem tesztelt. Kockázat: a gyártó-specifikus szerverhibák csak a felhasználói pilóton derülnek ki; ezt egy külső mozis partner egyszeri ellenőrzése csökkentené.

## 9. Fázisok IV. – Speciális adapterek, GUI és kiadás

### F6 – Speciális renderadapterek

**Cél:** procedurális grafika és resztaurációs szűrés izolált workerként, úgy, hogy nélkülük is teljes legyen a CFA4. **Plafon:** 20 munkanap. **Előfeltétel:** F3 kilépés (stabil render API), C0 worker protokoll, D1.

**Feladatok**

1. rust-motion adapter: az S1 (2026-11-22) szerint nincs adapter, csak minta; a cél (a G-19 alfa/élsimítás) a CFA4-ben valósul meg: címek, SVG, procedurális mozgógrafika, átlátszó képkockák, MQ-4 alfa.
2. VapourSynth worker: a retusáló szkriptek (pl. QTGMC) miatt nyilvántartott Python verzión, izolált venv-ben. Szigorú CPU-, RAM- és időkorlát, útvonal-korlátozás, szűrő round-trip.
3. Worker-izolációs tesztek: írás a projektmappán kívül, hálózat, túlfutás, összeomlás.
4. Fallback: hiányzó vagy hibás adapternél az A4 szabálytábla szerinti viselkedés.

**Átadandók:** capability/worker protokoll implementáció, `F6-isolation-tests.md`, fallback-leírás.

**Kilépési feltételek:** AC-03, AC-10, AC-11 PASS; adapterek nélkül a CFA4 alap- és exportfunkciói teljesek; az A11 teljesül (csak nyilvántartott, indokolt Python verziók).

**Kockázat és visszavonás:** natív VapourSynth pluginok biztonsága (védelem: pinelt pluginlista, sandbox); adapterek egyenként kikapcsolhatók.

### F8 – Qt6 megjelenítés, CLI és renderqueue

**Cél:** a job API GUI-integrációja és felhasználói elfogadása, a Qt6 fejlesztési sorrend megtartásával. **Plafon:** 15 munkanap. **Előfeltétel:** C0 job API; F3 kilépés; a Qt6 érintett részének stabil API-ja.

**Feladatok**

1. Progress, cancel, retry, queue-prioritás, log és backendválasztás a Qt6-ban és a CLI-ben, ugyanazon a job API-n.
2. Preview és export ugyanazt a frame-, szín- és időszerződést használja.
3. Főszál-watchdog bekötése (F0V interfész).
4. Szerkesztett projekt megőrzése régi és új sémaverzió között.

**Átadandók:** GUI-paritás teszt, `F8-uat-record.md` (felhasználói elfogadási jegyzőkönyv).

**Kilépési feltételek:** AC-09, AC-14, AC-15 PASS; a meglévő Qt6 paritásjavítások tesztjei zöldek; a fennmaradó GUI-hiányok külön PENDING listán.

**Kockázat és visszavonás:** ütközés a Qt6 ütemezéssel (védelem: integráció csak stabil API-ra); az új GUI-elemek flaggel elrejthetők.

### F9 – Kiadási kapu, dokumentáció, fokozatos bekapcsolás

**Cél:** igazolni, hogy a release minden kötelező kapun átment, és semmi nincs elhallgatva. **Plafon:** 10 munkanap. **Előfeltétel:** minden engedélyezett fázis lezárva.

**Feladatok**

1. CI Ubuntu 26.04/amd64: CPU-only, GPU smoke, codec-regresszió, MQ nightly.
2. Ismert J2K/HEIC/phone\_vfr/FFmpeg-orákulum hiányok önálló nyilvántartással; a J2K mozi célú kiadásnál blokkoló.
3. Feature flag állások, rollback-próba, SBOM, függőségi audit, benchmark, release note.
4. Fokozatos bekapcsolás: új út opt-in → alapértelmezett, a régi út egy kiadásig megmarad.

**Átadandók:** `F9-release-gate.md`, `F9-verified-vs-pending.md`, géppel generált kihagyottteszt-lista.

**Kilépési feltételek:** minden AC (AC-01–AC-20) PASS vagy explicit, jóváhagyott PENDING/BLOCKED; mozi minőségű kiadási állítás csak AC-18, AC-19 és a J2K orákulum PASS esetén; nincs release-kész állítás piros kötelező kapu mellett.

**Kockázat és visszavonás:** zöld részteszt hamis biztonséga (védelem: kihagyottteszt-lista a riportban); rollback a feature flagekkel.

## 10. Kötelező elfogadási tesztek (AC)

Minden AC-hez metrika, módszer és gazda fázis tartozik. A számszerű küszöbök (*Küszöb@F0*) az F0V-ben, a mért baseline-zajból kerülnek rögzítésre, és utólag csak ADR-rel módosíthatók; mérés nélkül választott univerzális szám nem lehet release-kritérium.

| AC | Teszt | PASS definíció | Gazda fázis |
| --- | --- | --- | --- |
| AC-01 | Build | Tiszta Ubuntu 26.04/amd64, `--locked` build; két független build azonos lockfile-hash-sel és azonos teszteredménnyel. | F0b, F9 |
| AC-02 | Alap export | H.264/HEVC/AV1 decode → render → encode → reimport; képkockaszám, PTS-lista, színmetaadat egyezik; PSNR/SSIM ≥ Küszöb@F0. | F0b, F9 |
| AC-03 | CPU-only | GPU elrejtve: az F0-ban listázott workflow-k mind PASS, N egymás utáni futásban 0 crash. | F3, F6 |
| AC-04 | Renderhelyesség | CPU: bit-azonos a golden halmazzal; GPU: PSNR/SSIM és ΔE 2000 Küszöb@F0-n belül; alfa, HDR, EXR külön. | F2, F4, F5 |
| AC-05 | Időzítés | PTS-diff 0 (vagy dokumentált); A/V drift ≤ Küszöb@F0 ms a teljes hosszon; seek képkockapontos; VFR, loop, trim regressziómentes. | F2, F3 |
| AC-06 | GPU | Hibainjektálás (driver hiány, VRAM-kimerülés, NVENC session-limit) után A4 szerinti FAIL\_CLOSED vagy jelzett fallback, logolva. | F4 |
| AC-07 | Erőforrások | Hosszú renderen RSS/VRAM növekedés az első 10 perc után ≤ Küszöb@F0; heaptrack/ASan: 0 új leak; csúcs ≤ konfigurált budget. | F3, F4 |
| AC-08 | Resume/cancel | Atomikus kimenet; N véletlen idejű SIGKILL után végleges néven 0 csonka fájl; resume kimenete = megszakítás nélküli kimenet. | C0, F3 |
| AC-09 | Qt6 | Főszál-watchdog: render, preview és cancel alatt nincs Küszöb@F0 ms-nál hosszabb event loop szünet; cancel latencia mérve. | F8 |
| AC-10 | Coexistence | Futás előtti/utáni pillanatkép (`$PATH`, `/usr`, `/etc`, `~/.local`, `~/.config`, felhasználói szolgáltatások, csomaglista): eltérés csak a CFA4 névtérben. | F6, F9 |
| AC-11a | Licenc | SBOM és licenckompatibilitási mátrix hiánytalan, FFmpeg flagek rögzítve. | F1, F9 |
| AC-11b | Bemenet-biztonság | Path traversal és symlink tesztek PASS. | F6 |
| AC-11c | Sandbox | Worker a projektmappán kívül nem ír, hálózatot nem nyit, erőforrás-limitet nem lép túl (teszttel). | F6 |
| AC-12 | Ismert rések | J2K, HEIC, phone\_vfr explicit BLOCKED/PENDING; a kihagyott tesztek géppel generált listája a release-riportban. | F0b, F9 |
| AC-13 | Audio | Keverés/resampling null-teszt ≤ Küszöb@F0 dBFS; csatornaelrendezés helyes; több hangsáv megmarad. | F3 |
| AC-14 | Preview–export paritás | Ugyanarra a képkockára a preview és az export színe és tartalma Küszöb@F0-n belül egyezik: ΔE2000 ≤ Küszöb@F0 és legfeljebb 1,0 (ADR 0003 A2), CPU-n és GPU-n, alfa- és HDR-eset külön. | F5, F8 |
| AC-15 | Projektmigráció | Régi sémájú projekt megnyitása, mentése, újranyitása után 0 elveszett mező és azonos render. | F3, F8 |
| AC-16 | Teljesítmény | End-to-end renderidő és csúcsmemória nem rosszabb a baseline-nál (CI alsó határ alapján). | F2, F9 |
| AC-17 | Feliratok | Feliratsáv megmarad, begetés pixelpontos a golden-hez (döntés 2026-11-21: a ciklus része; DCP-ben SMPTE ST 428-7 timed text; a nyelvek és formátumok körét az F5S bővítés rögzíti: minden írásrendszer, minden elérhető formátum státusszal). | F5S |
| AC-18 | Mozi minőség | MQ-1–MQ-8 PASS a referenciacsomagon, CPU-n és GPU-n; DCI 2K/4K X'Y'Z' konverziós hiba Küszöb@F0 alatt; mozi célú kiadáshoz J2K orákulum PASS; valós mozis szerveres vagy vetítős mérés nincs (nincs hozzáférés), ezért szoftveres referenciák döntenek. | F3, F4, F5, F9 |
| AC-19 | DCP | Független szabványellenőrző 0 hibát jelez (SMPTE alap, Interop is tesztelve); ASSETMAP/PKL/CPL hash-ek és azonosítók egyeznek; a kép DCI-profil szerinti J2K 12 bit X'Y'Z'; hang 24 bites PCM; felirat érvényes; round-trip PSNR ≥ Küszöb@F0, hang bit-azonos; szándékosan hibás csomagokat az ellenőrző elutasítja; mozis szerveren vagy vetítőn való lejátszás nem vizsgálható (István, 2026-11-21: nincs ilyen szerver és nincs hozzáférés); az állítás: szabványnak megfelelő és szoftveresen ellenőrzött, valós mozis szerveren nem tesztelt; KDM csak generált tesztanúsítványra. | meglevő tervek, F9 |
| AC-20 | OFX-gazdagép | Az OpenFX referencia tesztpluginek PASS; összeomló vagy lefagyó plugin nem viszi el a CFA4 fő folyamatát (N injektált hiba, 0 összeomlás a fő folyamatban); 8 bites OFX út mozi láncban jelezve (MQ-1); a plugin keresési útja névterezett, AC-10 pillanatkép tiszta; a CFA4 plugin nélkül is teljes. | ADR 0003 A4, F9 |

## 11. Mérési terv

Minden összevetés ugyanazon a forráson, pixel pipeline-on, encoder preseten és hardver/driver kombináción fut; a mikrobenchmark nem helyettesíti az end-to-end mérést.

- **Referenciacsomag:** 1080p25/30, 4K25/30/60, 2K és 4K DCI 24/48 fps DCI-P3, VFR forgatott mobilfelvétel, 8/10/12/16 bit, SDR/HDR (PQ, HLG), alfa, EXR szekvencia, zajos anyag, felirat, 5.1/7.1 hang, több hangsáv, seek-próba, legalább 60 perces hosszú render.
- **Futásszám:** trendhez 3 meleg + 3 hideg; döntéshez (F2, D1, AC-16) legalább 10 futás. Medián, p95 és bootstrap 95% konfidenciaintervallum.
- **Hideg futás root nélkül:** `posix_fadvise(POSIX_FADV_DONTNEED)` a saját bemeneti fájlokra vagy friss fájlmásolat; `drop_caches` csak külön jóváhagyott mérőgépen (A6).
- **Naplózott értékek:** CPU%, GPU%, RSS, VRAM, I/O, képkockánkénti idő, cancel-latencia, event loop szünetek.
- **Energia:** csak ha root nélkül megbízhatóan mérhető (a RAPL számláló általában root-jogú); különben PENDING.
- **Metaadat:** az A10 teljes készlete minden riportban.
- **Küszöbképzés:** Küszöb@F0 = a baseline mérési zajából származtatott, dokumentált érték; F0V rögzíti, ADR nélkül nem változhat.
- **CI szintek:** smoke (minden PR, CPU-only, rövid), nightly (teljes CPU + MQ), hardware (GPU-mátrix, ütemezett vagy kézi); teljes GPU-mátrix nem fut minden PR-ben.

## 12. Kockázatnapló

A napló F1-ben géppel olvasható formába kerül (`F1-risk-register.csv`), és minden kapunál frissül.

| Kockázat | Védőintézkedés | Figyelt fázis |
| --- | --- | --- |
| Második idővonal vagy projektmodell | Egyetlen CFA4 adatmodell; adapter-séma csak peremen (A1) | F3, F5 |
| Rejtett minőségromlás (8 bit, kroma, szín) | MQ kapu, mutációs önteszt, A4 FAIL\_CLOSED exportnál | minden |
| J2K/DCP rés mozi kiadásnál | AC-18 és J2K orákulum mint blokkoló | F0b, F9 |
| Rust MSRV/ABI eltérés | F1 pin, projektlokális toolchain, nincs automatikus host-frissítés | F1 |
| FFmpeg libav ütközés | Egyetlen libav ABI (A7); folyamatizoláció, ha szükséges | F1, F6 |
| CPU↔GPU másolási költség | End-to-end mérés, ownership szerződés (C0) | F2, F4 |
| Alfa/HDR regresszió | Golden EXR, color-managed összevetés | F4, F5 |
| Python-verziók elszaporodása | A11 nyilvántartás, indoklás, kiváltás ahol lehet | F1, F6 |
| Plugin- és worker-biztonság | Sandbox root nélkül, erőforrás-limit, útvonal-korlátozás | F1, F6 |
| Ubuntu AppArmor user-namespace korlátozás | Sandbox-választás, amely nem igényel host-módosítást | F1 |
| A6 és mérés ütközése | Root nélküli hideg futás, energia csak jóváhagyott mérőgépen | F0V |
| Kis upstream (bus factor) | Egészségküszöb; alatta legfeljebb ADAPT\_PATTERN\_ONLY | F1, D1 |
| Provenance / ellenőrizetlen állítások | Claim-teszt, pinelt commit, SBOM | F1 |
| GPL/LGPL és kodekszabadalmak | Licencmátrix, FFmpeg flagek, Natron-kód tilalma | F1, F9 |
| Vendorizálás karbantartási terhe | D1 rubrika karbantartási súlya | D1 |
| NVIDIA driver / Wayland eltérés | A10 rögzítés, hardware CI-szint, R0 újrafuttatás | F0b, F4 |
| Qt6 újraírás a késői API miatt | Job API v0 már C0-ban, Qt6 felelős elfogadása | C0, F8 |
| Golden halmaz beégeti a hibákat | Ismert eltérések listája | F0V |
| CI költségrobbanás | Szintezett smoke/nightly/hardware CI | F0V, F9 |
| Idő- és költségtúllépés | Fázisonkénti plafon, 50%-os túllépésnél döntési pont | minden |
| Zöld részteszt hamis biztonséga | Kihagyottteszt-lista a release-riportban | F9 |
| DCP-csomag szabványeltérése vagy csak saját olvasóval igazolt érvényessége; átfedés a meglévő DCP-munkákkal | Független validátor kötelező, negatív tesztek, egy főimplementáció, tulajdonosi egyeztetés (coordination §2.8) | F5D |
| OFX plugin = tetszőleges natív kód (összeomlás, biztonság, licenc) | Külön folyamat, sandbox, alapértelmezetten tiltva, hash-lista, pluginok egyenkénti licencvizsgálata, Natron-kód tilalma | F1, F5O |
| Feliratok: betűtípus- és szövegformázó-licenc, komplex írásrándszerek | F1 licenc-review, minden írásrendszer (nincs korlátozott célnyelv-lista, István 2026-11-21), a támogatatlan nyelvek explicit PENDING | F1, F5S |
| Nincs mozis szerver vagy vetítő a teszteléshez (István, 2026-11-21): gyártó-specifikus szerverhibák csak a felhasználói pilóton derülnek ki | AC-18/AC-19 szoftveres referenciákon; kimondott állítás: nem tesztelt valós mozis szerveren; KDM csak generált tesztanúsítványra; külső mozis partner egyszeri ellenőrzése ajánlott | F9 |

## 13. Sorrend, időkeret, szerepkörök és tiltott műveletek

A kutatási szakasz (F0a–D1) kritikus útja 25–32 munkanap; az implementációs szakasz plafonja a kritikus úton 60 munkanap (F3 → F5 → F9), összes ráfordításban 120 munkanap; az F5S, F5D és F5O nem új végrehajtási fázis, hanem hivatkozás a meglevő tervekre (2a. szakasz). Ezek kezdeti plafonok, a D1 után pontosítandók; a v1.0 F7 fázisa megszűnt (interfészrésze C0-ban van), ezért a számozásban nincs F7.

### Végrehajtási sorrend

1. **F0a** (3 nap) → utána párhuzamosan **F0b/F0V** (5–8 nap) és **F1** (5 nap).
2. **S1** szűkítés (1 nap), mindkettő lezárása után.
3. **C0** szerződések és job API v0 (5–8 nap).
4. **F2** POC (10 nap) → **D1** döntés (1–2 nap).
5. **F3** (plafon 30 nap) és **F4** (plafon 25 nap) párhuzamosan, ha a C0 fence/VRAM mezői rögzítettek.
6. F3 után párhuzamosan: **F5** (20 nap, F4 lezárását is igényli), **F6** (20 nap) és **F8** (15 nap, a Qt6 stabil API-jától függ).
7. **F9** kiadási kapu (10 nap), minden engedélyezett fázis után.

Minden nyíl egyben jóváhagyási pont: a következő fázis csak a megelőző kilépési feltételeinek PASS-a és külön jóváhagyás után indul. Az 50%-os időtúllépés automatikusan döntési pontot nyit.

**2026-11-21 kiegészítés a sorrendhez:** az F5S, F5D és F5O nem külön végrehajtási lépés: a meglevő terveket (2a. szakasz) azok gazdái viszik. Az F9 a megmaradó AC-17, AC-19 és AC-20 mérésétől függ.

### Szerepkörök

| Szerep | Felelősség | Személy |
| --- | --- | --- |
| Kapu-jóváhagyó | fázisindítás, S1, D1, F9, küszöb-ADR-ek | István (projektgazda) |
| Licenc-review felelős | A9, AC-11a, licencmátrix | PENDING |
| Qt6 felelős | C0 job API elfogadása, F8, A8 | PENDING |
| Mérési felelős | F0V, küszöbök, benchmark-riportok | PENDING |
| Architektúra-felelős | C0 szerződések, ADR-ek, A1–A11 | PENDING |

### Tiltott automatikus műveletek

A terv jóváhagyása önmagában nem jogosít fel repo-módosításra, PR-nyitásra, merge-re, telepítésre, fő toolchain frissítésére, rendszerbeállításra (beleértve AppArmor-t, sysctl-t, `drop_caches`-t), új Python verzió felvételére vagy új backend alapértelmezett bekapcsolására. Minden fázist külön kell engedélyezni. Az upstream módosítása tilos; minden fejlesztés kizárólag a CFA4-ben történik. POC-kód nem kerül automatikusan a fő ágba vagy az alkalmazáscsomagba.

## 14. Végső döntési javaslat

Javaslat: a célarchitektúra, az A1–A12 szabályok és az MQ kapu elfogadása, valamint az F0a, F0b/F0V és F1 engedélyezése. A többi fázis csak kapunkénti eredmény és külön jóváhagyás alapján indul.

- **Alap:** meglévő CFA4 renderer + FFmpeg 9.0.2.
- F2-jelöltek (S1, 2026-11-22; D1 előtt nem integrációs döntés): OpenImageIO, VapourSynth (csak külön workerként), EC-jelölt (feltételesen, MQ-kapuval), GEGL és babl (2026-11-23). ReelForge, avio ff-\* és rust-motion csak minták.
- **Külső munkafolyamat:** Blender; Natron csak referencia.
- **Referencia:** MLT, MovieLite (módszer), Gausian, Dioxuscut, GStreamer (rögzítési terv).
- **Kizárva ebből a ciklusból:** Lumen, MoviePy, Movis, Manim, PyAV, VidGear.

**Engedélyezési állapot:** FINAL 1.1 jóváhagyva (2026-11-21, 2b szakasz 2026-11-23); az F0a, F0V és F1 kész, a végrehajtás folyik. A további fázisok kapunkénti jóváhagyással indulnak.

### Meghozott döntések és új nyitott kérdések

- **DCP (eldöntve):** a CFA4 maga készít teljes DCP-csomagot; ezt a jóváhagyott CFA4-DCPOMATIC-001 (a CFA4-gyel szállított DCP-o-matic külön folyamatban végzi az Interop, 3D, felirat és titkosított DCP/KDM részt) és a saját SMPTE-export együtt teljesíti. A render-terv nem épít újat; elfogadás: AC-19 mérése.
- **Feliratok (eldöntve):** az AC-17 a ciklusban marad; a munkát a jóváhagyott SUBTITLE-STUDIO-PLAN-001 FINAL 1.2 és a PROCODEC-002 viszi, a render-terv csak az AC-17 mérését adja.
- **OFX (eldöntve):** az OFX-hosting cél; a terv: ADR 0003 A4 (jóváhagyva 2026-10-24); ADR 0003 §14: az első csomag clean-room (F5O hivatkozás).
- Nyitott kérdések (2026-11-23): (1) a C0 crate-név jóváhagyása (§2.10); (2) a QTGMC-motor és a Blender licence; (3) referenciagépes mérések. Eldöntve: az EC-jelölt marad az F2 harmadik jelöltje (István, 2026-11-23: a CFA4 az upstream családra épül, közös geometria-, szín- és időtípusok), az MQ-kapu érvényes, bukásnál csak mintának marad; a GEGL csak opcionális összehasonlítási pont, ha István kéri vagy az MQ-1/MQ-3 elbukik; a VapourSynth Python ≥ 3.13 igénye elfogadva (István, 2026-11-23; A11: egy nyilvántartott verzió, csak a külön worker folyamatban, a pontos verziót a rendszer-Python és az F0a környezetjelentés dönti el, PENDING); DCP-titkosítás és KDM (DCPOMATIC D4, CINECERT D3), nincs mozis szerver, minden OFX plugin lehetőség szerint, minden nyelv és feliratformátum.

### PENDING tételek

- A CFA4 HEAD szerkezete, a meglévő renderer, a Qt6 állapot és a PR #271/#331 tartalma (F0a/F0b).
- FFmpeg 9.0.2 verzió és build-konfiguráció (F0a/F1); a Rust toolchain 1.97.1, a ReelForge MSRV-akadálya megszűnt.
- A kis upstreamek licence, aktivitása és MQ-képessége (F1, F2).
- A VapourSynth retusáló plugin- és szkriptkészlete és licencei; az Ubuntu 26.04 rendszer-Python verziója (F0a/F1).
- Ubuntu 26.04 AppArmor user-namespace alapértelmezése (F1).
- A hivatkozott rögzítési terv és kodeksorrend dokumentum-ID-ja (F0a).

### Források

- CFA4: [github.com/ubuntuokos/CinemaFundArchitecture4](https://github.com/ubuntuokos/CinemaFundArchitecture4)
- [avio](https://github.com/itsakeyfut/avio) · [ReelForge](https://github.com/sergii-ziborov/ReelForge) · EC-jelölt (#11)((tároló: lásd az élő terv)) · [rust-motion](https://github.com/sjkim1127/rust-motion)
- [OpenImageIO](https://github.com/AcademySoftwareFoundation/OpenImageIO) · [VapourSynth](https://github.com/vapoursynth/vapoursynth) · [Blender](https://www.blender.org/) · [Natron](https://natrongithub.github.io/)
- [GStreamer](https://gstreamer.freedesktop.org/) · [MLT](https://www.mltframework.org/) · [Lumen](https://docs.lumiscia.com/open-source) · [Gausian](https://github.com/gausian-AI/Gausian_native_editor) · [Dioxuscut renderer](https://docs.rs/crate/dioxuscut-renderer/latest)
- [MoviePy](https://github.com/Zulko/moviepy) · [Movis](https://github.com/rezoo/movis) · [Manim](https://github.com/ManimCommunity/manim) · [MovieLite](https://github.com/francozanardi/movielite) · [PyAV](https://github.com/PyAV-Org/PyAV) · [VidGear](https://github.com/abhiTronix/vidgear)

A forrásokat ebben a körben nem nyitottam meg; a jelöltekre vonatkozó állítások általános ismeretre épülnek, és F1-ig PENDING státuszúak. Kritikai előzmény: [a v1.0 elemzése](https://claude.ai/artifact/CqWMfhEN18rakUkSigB4zp).
