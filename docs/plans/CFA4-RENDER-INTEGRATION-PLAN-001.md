# CFA4-RENDER-INTEGRATION-PLAN-001 v1.1

2026-10-31 · ubuntuokos

> **Repó-példány.** A terv élő példánya a tulajdonos Claude Docs dokumentuma
> (<https://claude.ai/artifact/LU4M9conuPpCpHp4JLWbW1>, 33. revízió, 2026-11-14); ez a fájl annak a
> `main`-re hozott másolata (`VISION.md` 2.6). Eltérés a forrástól: az upstream- és a #11-es jelölt
> márkanevét a `python/tests/test_independence.py` szűrője miatt semleges megnevezés váltja.
> **Végrehajtási állapot:** az F0a eredménye és a tervet érintő helyesbítések:
> [`docs/audit/RENDER-F0a-BASELINE-2026-11-14.md`](../audit/RENDER-F0a-BASELINE-2026-11-14.md).

Videórenderelési alrendszer célzott fejlesztési és integrációs terve. Célplatform: Ubuntu 26.04 LTS, amd64. Állapot: TERV, jóváhagyásra vár; végrehajtás nem indult. Kizárva: OxiMedia, Vercel, HandBrake.

## 1. Vezetői összefoglaló

A CFA4 natív, upstream-eredetű (`NOTICE`) renderelési motorja, projekt- és idővonalmodellje marad az elsődleges; teljes motorkiváltás nincs. A fejlesztés célja, hogy a CFA4 **mozi minőségű** kimenetet adjon CPU-only módban is, minimális külső függőséggel és a lehető legkevesebb Python verzióval.

A v1.0-hoz képest a v1.1 fő változásai:

- **Mozi-minőségi kapu (MQ-1–MQ-8)** minden jelöltre és kiadásra; a 20 jelöltből 6 jut a POC-ba.
- **Két új alapozó fázis:** F0V (validációs alap) és C0 (szerződések és job API v0). Ezek szállítják a v1.0-ban fázishoz nem rendelt cfa4-render-validation, -contracts és -job-service modulokat.
- **S1 szűkítési döntési pont** F1 és F2 között.
- **A11 szabály:** a lehető legkevesebb Python verzió, minden további verzió indoklással és nyilvántartva.
- **Mérhető AC-k:** minden AC-hez metrika, módszer és F0-ban rögzített küszöb; új AC-13–AC-18.
- **Minden fázis egységes sablonnal:** cél, előfeltétel, feladatok, átadandók, kilépési feltételek, kockázat, visszavonás, időkeret.
- A régi F7 (capture) interfészrésze C0-ba kerül; önálló rögzítőfejlesztés ebben a tervben nincs.

**Jóváhagyásra javasolt döntések**

1. A célarchitektúra, az A1–A11 szabályok és az MQ kapu elfogadása.
2. Az F0a, F0b/F0V és F1 végrehajtásának engedélyezése (időkeret összesen kb. 13–16 munkanap).
3. Minden további fázis (S1-től) kizárólag a megelőző kapu eredménye és külön jóváhagyás alapján indul.

## 2. Kiindulási adatok és bizonyítottság

Az alábbi adatok korábbi állapotközlések; a jelenlegi HEAD-en egyik sincs újraellenőrizve, ezért mind PENDING az F0a/F0b lezárásáig.

| Adat | Érték | Forrás | Állapot |
|---|---|---|---|
| Qt6 GUI készültség | kb. 53% | közlés, 2026-10-28 | PENDING |
| Alap | Rust-központú upstream-derivált (`NOTICE`) | közlés | PENDING |
| FFmpeg | 9.0.2 | közlés | PENDING (verzió és `configure` flagek) |
| Rust toolchain | 1.95 | közlés | PENDING |
| R0 GPU eredmények | H.264/HEVC/AV1 NVENC PASS; H.264/HEVC NVDEC PASS | korábbi R0 | PENDING (újrafuttatás HEAD-en) |
| Exportteszt-rések | J2K orákulum, heif-enc/HEIC round-trip, phone_vfr | PR #271, #331 | PENDING (mozi célra a J2K blokkoló) |
| Qt6 paritáshibák | ismertek, listájuk nincs csatolva | közlés | PENDING |

**Hivatkozott, de azonosító nélküli dokumentumok:** a korábban jóváhagyott hang/videó-rögzítési terv és a kodekfejlesztési sorrend. Ezek ID-jét és verzióját F0a-ban rögzíteni kell, különben a rájuk épülő kapuk nem ellenőrizhetők.

**Bizonyítási szabály:** amit forráskód, teszt vagy reprodukálható mérés nem támaszt alá, az PENDING. Upstream marketingállítás nem PASS.

## 3. Jelöltjegyzék a mozi-minőségi szűkítés után

A 20 jelöltből 1 alap, 6 POC-jelölt, 2 külső munkafolyamat, 5 referencia, 6 kizárt. A besorolás előzetes; véglegesítése az S1 döntési pont (F1 eredménye alapján). A táblázat nem bizonyítja, hogy egy képesség hiányzik a CFA4-ből; azt F0 mutatja ki.

| # | Jelölt | Szerep a CFA4-ben | Státusz | Python | Megjegyzés |
|---|---|---|---|---|---|
| 01 | FFmpeg 9.0.2 | decode/filter/encode/mux, export-orákulum | ALAP, MEGTARTANDÓ | nem | A7 szerint egyetlen libav ABI. |
| 07 | OpenImageIO | EXR és képszekvencia I/O, OCIO-kapcsolat | POC-JELÖLT | nem | Erős MQ-jelölt. |
| 04 | VapourSynth | resztauráció, deinterlace, szűrő worker | POC-JELÖLT | igen, 1 nyilvántartott verzió | Retusáláshoz szükséges; a szkriptes eszközök (pl. QTGMC) Pythont igényelnek, ez indokolt A11-igény. |
| 09 | ReelForge | DAG, dirty-region, cache, resume algoritmus | POC-JELÖLT | nem | Képet nem érint; MSRV 1.97+ PENDING. |
| 08 | avio / ff-* | Rust médiaréteg | POC-JELÖLT, feltételes | nem | Csak ha 10/12/16 bit és 4:4:4 torzítás nélkül átmegy. |
| 11 | EC-jelölt (#11) | réteges kompozíció, kulcskockák | POC-JELÖLT, feltételes | PENDING | MQ-1/3/4/8 igazolása F2 elején. |
| 10 | rust-motion | címek, SVG, procedurális grafika | POC-JELÖLT, feltételes | nem | Alfa- és élsimítási pontosság. |
| 05 | Blender | 3D/VFX képszekvencia-renderjob | KÜLSŐ MUNKAFOLYAMAT | saját, beépített | EXR csere, külön folyamat, verzió pinelve. |
| 06 | Natron | OFX node-modell minta | KÜLSŐ / REFERENCIA | saját | GPL: kódátvétel tilos. |
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

## 4. Architekturális szabályok és a mozi-minőségi kapu

Az A1–A11 szabályok és az MQ kapu minden fázisra kötelezők; eltérés csak írásos architektúra-döntéssel (ADR) és jóváhagyással.

**A1 – Egyetlen projektigazság.** A CFA4 (upstream-eredetű) projektmodell és idővonal a source of truth. Külső projektséma csak import/export peremen.

**A2 – Egységes Frame ABI.** Pixel- és plane-formátum, bitmélység, színprimerek, transfer, mátrix, tartomány, alfa mód (straight/premultiplied), méretek és stride, SAR, field order, forgatás/display matrix, chroma siting, HDR side data (mastering display, MaxCLL/MaxFALL), PTS és timebase, forrás-azonosító, ownership/lifetime, CPU/GPU hely, fence/szinkronizáció. Részletes definíció: C0.

**A3 – Egységes Audio ABI.** Mintavételi frekvencia, mintaformátum, csatornaelrendezés, időalap, mintapontos pozíció, buffer-élettartam, késleltetés, drift-kezelés.

**A4 – Capability discovery és szabálytábla.** A CPU, Vulkan/wgpu, CUDA és adapter-képességek egységes lekérdezése. Capability × környezet szabálytábla: exportnál FAIL_CLOSED, preview-nál JELZETT_FALLBACK; csöndes minőségromlás soha.

**A5 – CPU-only kötelező.** Minden kiadáshoz szükséges workflow és az MQ kapu GPU nélkül is teljesül, legfeljebb lassabban.

**A6 – Host coexistence.** Külső binárisok, plugin cache, FFmpeg és Rust könyvtárak kizárólag izolált, névterezett módon. Nincs globális csomagfrissítés, PATH- vagy plugin-módosítás, root művelet, AppArmor- vagy sysctl-változtatás jóváhagyás nélkül. Mérésre és sandboxra is vonatkozik.

**A7 – FFmpeg elsőbbség.** Az FFmpeg 9.0.2 az export-orákulum és az egyetlen betöltött libav ABI. Másik libav (pl. wheel-be csomagolt) a fő folyamatba nem tölthető be.

**A8 – Qt6 izoláció.** A GUI főszálán nincs hosszú művelet; progress/cancel/log a közös job API-n (C0). A meglévő Qt6 paritásjavítások nem vethetők vissza.

**A9 – Licenc és provenance.** Átvett kódhoz eredeti commit, licenc, tranzitív függőségek, attribúció, SBOM, licenckompatibilitási mátrix a CFA4 licencével, FFmpeg `configure` flagek rögzítése. Licenc-review nélkül nincs merge.

**A10 – Reprodukálhatóság.** Bemeneti hash, beállítások, verziók, GPU driver, kernel, display-szerver, kodek, seed és render graph hash minden benchmark és release teszt mellett.

**A11 – Minimális Python-verziószám.** A CFA4 a lehető legkevesebb Python verzióval dolgozik. Alapértelmezés az Ubuntu 26.04 rendszer-Pythonja (verzió PENDING). További verzió akkor vehető fel, ha egy jóváhagyott komponens igazoltan nem fut az alapértelmezetten; ezt az F1-ben induló Python-nyilvántartás rögzíti indoklással. Workerenként külön venv megengedett, de csak nyilvántartott verzión; conda/mamba nem használható. A fő alkalmazás Python nélkül is teljes. Ahol a Python funkcióvesztés nélkül kiváltható, ott a kiváltás az első választás; a VapourSynth retusáló szkriptjei indokolt Python-igénynek számítanak.

### MQ – Mozi-minőségi kapu

Az MQ kapu: bármely FAIL kizárja a jelöltet a képláncból, és blokkolja a mozi célú kiadást. A képet nem érintő eszközöknél (pl. gráfalgoritmus) a feltétel a bit-azonos továbbítás.

| ID | Kritérium | PASS definíció |
|---|---|---|
| MQ-1 | Belső pontosság | Legalább 16 bites lebegőpontos vagy 16 bites egész feldolgozás; nincs rejtett 8 bites köztes lépés. |
| MQ-2 | Kroma | 4:4:4 és RGB alulmintavételezés nélkül átmegy. |
| MQ-3 | Színkezelés | Explicit színtér-kezelés (OpenColorIO vagy azzal egyező CFA4 színszerződés); DCI-P3, Rec.2020, PQ/HLG, X'Y'Z' konverzió hibája Küszöb@F0 alatt. |
| MQ-4 | Alfa | Straight/premultiplied megkülönböztetve; nincs peremhiba. |
| MQ-5 | Felbontás, képkockasebesség | 2K DCI (2048×1080), 4K DCI (4096×2160); 23,976, 24, 25, 48 fps képkockapontosan. |
| MQ-6 | Determinizmus | CPU-n bit-azonos kimenet; GPU-n dokumentált, küszöb alatti eltérés. |
| MQ-7 | Hang | 24 bit, 48 és 96 kHz, 5.1 és 7.1, mintapontos szinkron. |
| MQ-8 | Skálázás, blend | Lineáris fényben végzett blend és skálázás választható; megnevezett, rögzített szűrő. |

**Mozi kimenet (DCP):** a DCP képe JPEG 2000, X'Y'Z', 12 bit. Ezért a J2K orákulum-rés mozi célú kiadásnál blokkoló. A DCP csomagolás (CPL/PKL, titkosítás) külső eszközzel vagy saját implementációval valósítandó meg; ez nyitott döntés (14. fejezet).

## 5. Modulhatárok és gazda fázisok

Minden modulnak pontosan egy fázis a gazdája, amely az első használható változatát szállítja. A nevek javaslatok; F0a után a tényleges repo-struktúrához igazodnak, és ha a CFA4-ben már létezik megfelelő modul, azt kell bővíteni, nem újat létrehozni.

| Modul | Felelősség | Gazda fázis | Továbbfejlesztő fázisok |
|---|---|---|---|
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

**Kilépési feltételek:** a 2. fejezet minden PENDING sorához igazolt érték vagy indokolt PENDING; a call graph újragenerálható egy paranccsal; másik gépen ugyanaz a SHA ugyanazt a lockfile-hash-t adja.

**Kockázat és visszavonás:** csak olvasó művelet, nincs visszavonandó változás.

### F0b és F0V – Tesztbizonyíték és validációs alap (blokkoló F2-re)

**Cél:** a jelenlegi renderer viselkedésének gépi rögzítése és a cfa4-render-validation v0 szállítása, amelyen minden későbbi kapu mér. **Időkeret:** 5–8 munkanap, F1-gyel párhuzamosan. **Előfeltétel:** F0a kész; licenctiszta tesztanyag.

**Feladatok**

1. PR #271/#331 tesztjeinek állapota; Qt6 funkcionális akadályok listája; R0 NVENC/NVDEC újrafuttatása HEAD-en.
2. Referenciacsomag: 1080p25 8-bit SDR; 4K DCI 24 fps 12-bit DCI-P3; 4K30 10-bit PQ; VFR forgatott mobilfelvétel; alfás klip és EXR szekvencia; 5.1 és 7.1 hang 24 bit/48 kHz; szinkronfixture (villanás + sípszó); retusálási fixtures: interlaced, telecine-elt, zajos és sávosodó anyag. Manifest: hash, forrás, licenc.
3. Golden frame-ek a jelenlegi rendererből: SDR 16 bites PNG, HDR/alfa/DCI EXR, képkockánkénti hash; frissítés csak review-zott változtatással.
4. Ismert eltérések listája (J2K, HEIC, phone_vfr és az F0 által talált hibák), hogy a golden ne rögzítse a hibát elvárt eredményként.
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
- Minden AC-hez (AC-01–AC-18) van mérő vagy indokolt PENDING.
- A gap-mátrix minden tervezett képességhez bizonyított GAP-et, meglévő funkciót vagy mérhető optimalizálási célt rendel.
- Ha egy területen nincs GAP és nincs mérhető cél, az ottani jelöltek automatikusan kiesnek.

**Kockázat és visszavonás:** a golden beégetheti a hibákat (védelem: ismert eltérések listája); a fixtures mérete (külön tárolás, módja PENDING). A harness terméktől független, eltávolítása nem okoz regressziót.

### F1 – Jogi, biztonsági és komponens-audit

**Cél:** a POC-jelöltek és a külső munkafolyamatok jogi, biztonsági, érettségi és host-lábnyom szerinti előszűrése. **Időkeret:** 5 munkanap, F0a után. **Előfeltétel:** F0a (toolchain, FFmpeg flagek, Python verzió).

**Feladatok jelöltenként** (OpenImageIO, VapourSynth, ReelForge, avio ff-*, EC-jelölt (#11), rust-motion, Blender, valamint a referenciák licence)

1. Pinelt commit vagy kiadás, licenc, tranzitív függőségek, SBOM-tervezet, licenckompatibilitás a CFA4 licencével.
2. Upstream egészség: utolsó 12 hónap kiadásai, aktív karbantartók száma, CI, issue-reakció. Küszöb: legalább egy kiadás és 2 aktív karbantartó, különben legfeljebb ADAPT_PATTERN_ONLY.
3. MSRV vs. CFA4 toolchain (ReelForge 1.97+ vs. 1.95 PENDING); toolchain-emelés csak projektlokálisan és külön jóváhagyással.
4. FFmpeg API/ABI kompatibilitás 9.0.2-vel; második libav tilalma (A7).
5. Claim-teszt: minden átvenni kívánt README-állításhoz upstream teszt vagy saját reprodukálás.
6. MQ előszűrés dokumentáció és kód alapján (belső pixelformátumok, színkezelés, alfa).
7. Host-lábnyom: szükséges rendszercsomagok, Python-igény; a VapourSynth retusáláshoz szükséges plugin- és szkriptkészlete (pl. QTGMC, BM3D, deband, IVTC) és ezek licencei; a Python-nyilvántartás indítása (verzió, igénylő komponens, indoklás) az A11 szerint.
8. Sandbox-technológia root nélkül (`systemd-run --user` + cgroup limitek, seccomp, Landlock); az Ubuntu AppArmor user-namespace korlátozás hatása 26.04-en.
9. rust-motion és Dioxuscut kódátfedése; EC-jelölt (#11) érettségi állításai.

**Átadandók:** `F1-source-license-dependency-audit.md`, `F1-risk-register.csv`, `F1-host-footprint.md`, `F1-sandbox-decision.md`.

**Kilépési feltételek:** minden jelölt PASS / REFERENCE_ONLY / BLOCKED besorolást kap indoklással; minden BLOCKED-hoz megnevezett blokkoló; a sandbox-döntés nem igényel host-módosítást, vagy a szükséges módosítás jóváhagyásra van készítve; a VapourSynth Python-kérdés eldöntve.

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

**Feladatok – cfa4-render-job-service**

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
2. MQ igazolás az első 3 munkanapon (feltételes jelöltek: avio, EC-jelölt (#11), rust-motion); FAIL esetén a jelölt kilép a POC-ból.
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

**Cél:** jelöltenként egyetlen döntés: INTEGRATE_COMPONENT, ADAPT_PATTERN_ONLY, EXTERNAL_ADAPTER, RESEARCH_ONLY vagy REJECT. **Időkeret:** 1–2 munkanap.

| Szempont | Súly | Mérce |
|---|---|---|
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

**Cél:** CFA4 timeline → validált DAG, erőforrás-kezelt ütemezés, megszakítható és folytatható render, bit-azonos CPU referencia, audio renderút. **Plafon:** 30 munkanap. **Előfeltétel:** D1 (ReelForge döntés), C0 v0.

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
5. Natron csak minta; kódátvétel nincs.

**Átadandók:** `F5-vfx-adapter.md`, `F5-imageio-gap-tests.md`, `F5-color-pipeline.md`.

**Kilépési feltételek:** AC-04 (EXR/alfa), AC-14, AC-18 PASS; headless export PASS; a kompozíció nem hoz létre második projektmodellt (A1).

**Kockázat és visszavonás:** alfa- és HDR-regresszió (védelem: golden EXR, MQ-4); az adapter flaggel kikapcsolható, a meglévő kép-I/O megmarad.

## 9. Fázisok IV. – Speciális adapterek, GUI és kiadás

### F6 – Speciális renderadapterek

**Cél:** procedurális grafika és resztaurációs szűrés izolált workerként, úgy, hogy nélkülük is teljes legyen a CFA4. **Plafon:** 20 munkanap. **Előfeltétel:** F3 kilépés (stabil render API), C0 worker protokoll, D1.

**Feladatok**

1. rust-motion adapter (a D1 szerint): címek, SVG, procedurális mozgógrafika, átlátszó képkockák, MQ-4 alfa.
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
2. Ismert J2K/HEIC/phone_vfr/FFmpeg-orákulum hiányok önálló nyilvántartással; a J2K mozi célú kiadásnál blokkoló.
3. Feature flag állások, rollback-próba, SBOM, függőségi audit, benchmark, release note.
4. Fokozatos bekapcsolás: új út opt-in → alapértelmezett, a régi út egy kiadásig megmarad.

**Átadandók:** `F9-release-gate.md`, `F9-verified-vs-pending.md`, géppel generált kihagyottteszt-lista.

**Kilépési feltételek:** minden AC (AC-01–AC-18) PASS vagy explicit, jóváhagyott PENDING/BLOCKED; mozi minőségű kiadási állítás csak AC-18 és a J2K orákulum PASS esetén; nincs release-kész állítás piros kötelező kapu mellett.

**Kockázat és visszavonás:** zöld részteszt hamis biztonséga (védelem: kihagyottteszt-lista a riportban); rollback a feature flagekkel.

## 10. Kötelező elfogadási tesztek (AC)

Minden AC-hez metrika, módszer és gazda fázis tartozik. A számszerű küszöbök (*Küszöb@F0*) az F0V-ben, a mért baseline-zajból kerülnek rögzítésre, és utólag csak ADR-rel módosíthatók; mérés nélkül választott univerzális szám nem lehet release-kritérium.

| AC | Teszt | PASS definíció | Gazda fázis |
|---|---|---|---|
| AC-01 | Build | Tiszta Ubuntu 26.04/amd64, `--locked` build; két független build azonos lockfile-hash-sel és azonos teszteredménnyel. | F0b, F9 |
| AC-02 | Alap export | H.264/HEVC/AV1 decode → render → encode → reimport; képkockaszám, PTS-lista, színmetaadat egyezik; PSNR/SSIM ≥ Küszöb@F0. | F0b, F9 |
| AC-03 | CPU-only | GPU elrejtve: az F0-ban listázott workflow-k mind PASS, N egymás utáni futásban 0 crash. | F3, F6 |
| AC-04 | Renderhelyesség | CPU: bit-azonos a golden halmazzal; GPU: PSNR/SSIM és ΔE 2000 Küszöb@F0-n belül; alfa, HDR, EXR külön. | F2, F4, F5 |
| AC-05 | Időzítés | PTS-diff 0 (vagy dokumentált); A/V drift ≤ Küszöb@F0 ms a teljes hosszon; seek képkockapontos; VFR, loop, trim regressziómentes. | F2, F3 |
| AC-06 | GPU | Hibainjektálás (driver hiány, VRAM-kimerülés, NVENC session-limit) után A4 szerinti FAIL_CLOSED vagy jelzett fallback, logolva. | F4 |
| AC-07 | Erőforrások | Hosszú renderen RSS/VRAM növekedés az első 10 perc után ≤ Küszöb@F0; heaptrack/ASan: 0 új leak; csúcs ≤ konfigurált budget. | F3, F4 |
| AC-08 | Resume/cancel | Atomikus kimenet; N véletlen idejű SIGKILL után végleges néven 0 csonka fájl; resume kimenete = megszakítás nélküli kimenet. | C0, F3 |
| AC-09 | Qt6 | Főszál-watchdog: render, preview és cancel alatt nincs Küszöb@F0 ms-nál hosszabb event loop szünet; cancel latencia mérve. | F8 |
| AC-10 | Coexistence | Futás előtti/utáni pillanatkép (`$PATH`, `/usr`, `/etc`, `~/.local`, `~/.config`, felhasználói szolgáltatások, csomaglista): eltérés csak a CFA4 névtérben. | F6, F9 |
| AC-11a | Licenc | SBOM és licenckompatibilitási mátrix hiánytalan, FFmpeg flagek rögzítve. | F1, F9 |
| AC-11b | Bemenet-biztonság | Path traversal és symlink tesztek PASS. | F6 |
| AC-11c | Sandbox | Worker a projektmappán kívül nem ír, hálózatot nem nyit, erőforrás-limitet nem lép túl (teszttel). | F6 |
| AC-12 | Ismert rések | J2K, HEIC, phone_vfr explicit BLOCKED/PENDING; a kihagyott tesztek géppel generált listája a release-riportban. | F0b, F9 |
| AC-13 | Audio | Keverés/resampling null-teszt ≤ Küszöb@F0 dBFS; csatornaelrendezés helyes; több hangsáv megmarad. | F3 |
| AC-14 | Preview–export paritás | Ugyanarra a képkockára a preview és az export színe és tartalma Küszöb@F0-n belül egyezik. | F5, F8 |
| AC-15 | Projektmigráció | Régi sémájú projekt megnyitása, mentése, újranyitása után 0 elveszett mező és azonos render. | F3, F8 |
| AC-16 | Teljesítmény | End-to-end renderidő és csúcsmemória nem rosszabb a baseline-nál (CI alsó határ alapján). | F2, F9 |
| AC-17 | Feliratok | Feliratsáv megmarad, begetés pixelpontos a golden-hez (ha a scope-ban marad). | F5 vagy PENDING |
| AC-18 | Mozi minőség | MQ-1–MQ-8 PASS a referenciacsomagon, CPU-n és GPU-n; DCI 2K/4K X'Y'Z' konverziós hiba Küszöb@F0 alatt; mozi célú kiadáshoz J2K orákulum PASS. | F3, F4, F5, F9 |

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
|---|---|---|
| Második idővonal vagy projektmodell | Egyetlen CFA4 adatmodell; adapter-séma csak peremen (A1) | F3, F5 |
| Rejtett minőségromlás (8 bit, kroma, szín) | MQ kapu, mutációs önteszt, A4 FAIL_CLOSED exportnál | minden |
| J2K/DCP rés mozi kiadásnál | AC-18 és J2K orákulum mint blokkoló | F0b, F9 |
| Rust MSRV/ABI eltérés | F1 pin, projektlokális toolchain, nincs automatikus host-frissítés | F1 |
| FFmpeg libav ütközés | Egyetlen libav ABI (A7); folyamatizoláció, ha szükséges | F1, F6 |
| CPU↔GPU másolási költség | End-to-end mérés, ownership szerződés (C0) | F2, F4 |
| Alfa/HDR regresszió | Golden EXR, color-managed összevetés | F4, F5 |
| Python-verziók elszaporodása | A11 nyilvántartás, indoklás, kiváltás ahol lehet | F1, F6 |
| Plugin- és worker-biztonság | Sandbox root nélkül, erőforrás-limit, útvonal-korlátozás | F1, F6 |
| Ubuntu AppArmor user-namespace korlátozás | Sandbox-választás, amely nem igényel host-módosítást | F1 |
| A6 és mérés ütközése | Root nélküli hideg futás, energia csak jóváhagyott mérőgépen | F0V |
| Kis upstream (bus factor) | Egészségküszöb; alatta legfeljebb ADAPT_PATTERN_ONLY | F1, D1 |
| Provenance / ellenőrizetlen állítások | Claim-teszt, pinelt commit, SBOM | F1 |
| GPL/LGPL és kodekszabadalmak | Licencmátrix, FFmpeg flagek, Natron-kód tilalma | F1, F9 |
| Vendorizálás karbantartási terhe | D1 rubrika karbantartási súlya | D1 |
| NVIDIA driver / Wayland eltérés | A10 rögzítés, hardware CI-szint, R0 újrafuttatás | F0b, F4 |
| Qt6 újraírás a késői API miatt | Job API v0 már C0-ban, Qt6 felelős elfogadása | C0, F8 |
| Golden halmaz beégeti a hibákat | Ismert eltérések listája | F0V |
| CI költségrobbanás | Szintezett smoke/nightly/hardware CI | F0V, F9 |
| Idő- és költségtúllépés | Fázisonkénti plafon, 50%-os túllépésnél döntési pont | minden |
| Zöld részteszt hamis biztonséga | Kihagyottteszt-lista a release-riportban | F9 |

## 13. Sorrend, időkeret, szerepkörök és tiltott műveletek

A kutatási szakasz (F0a–D1) kritikus útja 25–32 munkanap; az implementációs szakasz plafonja a kritikus úton 60 munkanap, összes ráfordításban 120 munkanap. Ezek kezdeti plafonok, a D1 után pontosítandók; a v1.0 F7 fázisa megszűnt (interfészrésze C0-ban van), ezért a számozásban nincs F7.

### Végrehajtási sorrend

1. **F0a** (3 nap) → utána párhuzamosan **F0b/F0V** (5–8 nap) és **F1** (5 nap).
2. **S1** szűkítés (1 nap), mindkettő lezárása után.
3. **C0** szerződések és job API v0 (5–8 nap).
4. **F2** POC (10 nap) → **D1** döntés (1–2 nap).
5. **F3** (plafon 30 nap) és **F4** (plafon 25 nap) párhuzamosan, ha a C0 fence/VRAM mezői rögzítettek.
6. F3 után párhuzamosan: **F5** (20 nap, F4 lezárását is igényli), **F6** (20 nap) és **F8** (15 nap, a Qt6 stabil API-jától függ).
7. **F9** kiadási kapu (10 nap), minden engedélyezett fázis után.

Minden nyíl egyben jóváhagyási pont: a következő fázis csak a megelőző kilépési feltételeinek PASS-a és külön jóváhagyás után indul. Az 50%-os időtúllépés automatikusan döntési pontot nyit.

### Szerepkörök

| Szerep | Felelősség | Személy |
|---|---|---|
| Kapu-jóváhagyó | fázisindítás, S1, D1, F9, küszöb-ADR-ek | István (projektgazda) |
| Licenc-review felelős | A9, AC-11a, licencmátrix | PENDING |
| Qt6 felelős | C0 job API elfogadása, F8, A8 | PENDING |
| Mérési felelős | F0V, küszöbök, benchmark-riportok | PENDING |
| Architektúra-felelős | C0 szerződések, ADR-ek, A1–A11 | PENDING |

### Tiltott automatikus műveletek

A terv jóváhagyása önmagában nem jogosít fel repo-módosításra, PR-nyitásra, merge-re, telepítésre, fő toolchain frissítésére, rendszerbeállításra (beleértve AppArmor-t, sysctl-t, `drop_caches`-t), új Python verzió felvételére vagy új backend alapértelmezett bekapcsolására. Minden fázist külön kell engedélyezni. Az upstream módosítása tilos; minden fejlesztés kizárólag a CFA4-ben történik. POC-kód nem kerül automatikusan a fő ágba vagy az alkalmazáscsomagba.

## 14. Végső döntési javaslat

Javaslat: a célarchitektúra, az A1–A11 szabályok és az MQ kapu elfogadása, valamint az F0a, F0b/F0V és F1 engedélyezése. A többi fázis csak kapunkénti eredmény és külön jóváhagyás alapján indul.

- **Alap:** meglévő CFA4 renderer + FFmpeg 9.0.2.
- **POC-jelöltek (D1 előtt nem integrációs döntés):** OpenImageIO, VapourSynth, ReelForge; feltételesen avio ff-*, EC-jelölt (#11), rust-motion.
- **Külső munkafolyamat:** Blender; Natron csak referencia.
- **Referencia:** MLT, MovieLite (módszer), Gausian, Dioxuscut, GStreamer (rögzítési terv).
- **Kizárva ebből a ciklusból:** Lumen, MoviePy, Movis, Manim, PyAV, VidGear.

**Engedélyezési állapot:** jóváhagyásra váró terv; megvalósítás NEM INDULT.

### Nyitott döntések a projektgazdánál

- **DCP:** a CFA4 maga készítsen-e teljes DCP csomagot (CPL/PKL, opcionálisan titkosítás), vagy csak DCP-kész J2K/X'Y'Z' mastert adjon egy külső csomagolónak. Ez határozza meg az F5/F9 mozi kimenetének scope-ját.
- **Feliratok:** az AC-17 a scope-ban marad-e ebben a ciklusban.
- **OFX plugin-hosting:** cél-e, vagy a Natron csak node-modell minta.

### PENDING tételek

- A CFA4 HEAD szerkezete, a meglévő renderer, a Qt6 állapot és a PR #271/#331 tartalma (F0a/F0b).
- FFmpeg 9.0.2 verzió és build-konfiguráció; Rust 1.95 vs. ReelForge 1.97+ MSRV (F0a/F1).
- A kis upstreamek licence, aktivitása és MQ-képessége (F1, F2).
- A VapourSynth retusáló plugin- és szkriptkészlete és licencei; az Ubuntu 26.04 rendszer-Python verziója (F0a/F1).
- Ubuntu 26.04 AppArmor user-namespace alapértelmezése (F1).
- A hivatkozott rögzítési terv és kodeksorrend dokumentum-ID-ja (F0a).

### Források

- CFA4: [github.com/ubuntuokos/CinemaFundArchitecture4](https://github.com/ubuntuokos/CinemaFundArchitecture4)
- [avio](https://github.com/itsakeyfut/avio) · [ReelForge](https://github.com/sergii-ziborov/ReelForge) · EC-jelölt (#11; a neve és a linkje a terv Claude Docs példányában áll, ide az upstream-márkaszűrő, `python/tests/test_independence.py` miatt nem kerül) · [rust-motion](https://github.com/sjkim1127/rust-motion)
- [OpenImageIO](https://github.com/AcademySoftwareFoundation/OpenImageIO) · [VapourSynth](https://github.com/vapoursynth/vapoursynth) · [Blender](https://www.blender.org/) · [Natron](https://natrongithub.github.io/)
- [GStreamer](https://gstreamer.freedesktop.org/) · [MLT](https://www.mltframework.org/) · [Lumen](https://docs.lumiscia.com/open-source) · [Gausian](https://github.com/gausian-AI/Gausian_native_editor) · [Dioxuscut renderer](https://docs.rs/crate/dioxuscut-renderer/latest)
- [MoviePy](https://github.com/Zulko/moviepy) · [Movis](https://github.com/rezoo/movis) · [Manim](https://github.com/ManimCommunity/manim) · [MovieLite](https://github.com/francozanardi/movielite) · [PyAV](https://github.com/PyAV-Org/PyAV) · [VidGear](https://github.com/abhiTronix/vidgear)

A forrásokat ebben a körben nem nyitottam meg; a jelöltekre vonatkozó állítások általános ismeretre épülnek, és F1-ig PENDING státuszúak. Kritikai előzmény: [a v1.0 elemzése](https://claude.ai/artifact/CqWMfhEN18rakUkSigB4zp).
