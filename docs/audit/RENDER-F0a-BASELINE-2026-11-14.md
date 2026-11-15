# CFA4-RENDER-INTEGRATION-PLAN-001 – F0a: gépi tényfeltárás (render baseline)

**Dátum:** 2026-11-14 · **Terv:** [`CFA4-RENDER-INTEGRATION-PLAN-001`](../plans/CFA4-RENDER-INTEGRATION-PLAN-001.md) v1.1, 6. fejezet, F0a ·
**Forrás:** `main` @ `b76870906438ffc3503c36f2210528dd9ab7fab0` (2026-11-14 17:55 UTC, a #1258 köteg-beolvasztás) ·
**Jelleg:** csak olvasó művelet; nem épít, nem telepít, nem módosít terméket.

Átadandók (a terv F0a pontja szerint):

| Terv szerinti név | Itt |
|---|---|
| `F0-render-baseline.md` | ez a dokumentum |
| `F0-call-graph` (géppel generált) | [`render-f0a/call-graph.md`](render-f0a/call-graph.md) + [`call-graph.json`](render-f0a/call-graph.json), generátor: `python3 python/cfa4_devtools/render_inventory.py` (`--check`: frissesség) |
| `F0-environment.md` | [`RENDER-F0a-ENVIRONMENT-2026-11-14.md`](RENDER-F0a-ENVIRONMENT-2026-11-14.md), gyűjtő: `python3 python/cfa4_devtools/render_environment.py` |

## 1. Összefoglaló

1. **A terv több kiinduló feltevése elavult**: a CFA4-ben már megvan a mozi-kimenet nagy része.
   Van saját DCP-export (SMPTE ST 429-2, JPEG 2000 12 bites X'Y'Z', DCI 2K/4K, 5.1 24 bites PCM),
   IMF-export (ST 2067-21, J2K 10 bites R'G'B' 4:4:4), OpenEXR-szekvencia (half-float, alfa), DPX,
   ProRes / DNxHR / CineForm / APV köztes kodekek, determinizmus-teszt az exportra, golden-image
   tesztkészlet CPU–GPU paritással, valamint ΔE₀₀ és SSIM metrika. Részletek: 3–5. pont.
2. **A Rust toolchain 1.97.1, az MSRV 1.97** (nem 1.95): a ReelForge 1.97+ MSRV-akadálya a terv F1
   pontjában megszűnik.
3. **A terv modulnevei helyett meglévő crate-eket kell bővíteni** (a terv 5. fejezetének szabálya):
   `cfa4-render-validation` → `cfa4-golden` + `cfa4-metrics` + `cfa4-qc` + `cfa4-testkit`;
   `cfa4-render-backend-cpu` → `cfa4-render`; `cfa4-render-backend-gpu` → `cfa4-gpu`. Javasolt
   leképezés: 6. pont. Ez az F0V és a C0 munkáját érdemben csökkenti.
4. **Átfedés más, jóváhagyott vagy folyamatban lévő projektekkel** (DCP-mesterelés, job-sor,
   Smart Render, Media QC, OxiMedia-eredetű metrikák): 7. pont. A
   [`coordination.md`](../coordination.md) §2.8 szerint ezeken a részeken a render-projekt nem dolgozik
   tovább, amíg a tulajdonos és a gazda nem dönt.
5. **Kilépési feltételek** (8. pont): a forrásból olvasható adatok igazolva; a referenciagép
   gépspecifikus adatai (NVIDIA-driver, Vulkan, CUDA, rendszer-Python, a CFA4 FFmpeg-buildjének
   BUILDINFO-ja) a gyűjtő futtatásáig indokolt PENDING.

## 2. A terv 2. fejezetének adatai a HEAD-en

| Adat | Terv v1.1 | F0a-ban igazolt érték | Forrás | Állapot |
|---|---|---|---|---|
| HEAD, ág | – | `b768709…`, `main` | `git rev-parse` | PASS |
| Lockfile-ok | – | `Cargo.lock` sha256 `e12b8e69b96936230bd18613381ee37d20563beec99c2a591038f8a25a650334`; `requirements/linux-amd64.txt` `2aa0254d…8c90`; `requirements/otio-oracle.txt` `6608e9d5…bb94` | `render_environment.py` | PASS (git-követett fájlok: ugyanaz a SHA mindenhol ugyanazt a hasht adja) |
| Alap | Rust-központú, upstream-derivált | Rust Cargo workspace, 0.10.0-s upstream-ősből (`NOTICE`), GPL-3.0-or-later; `unsafe_code = "forbid"` a workspace-ben | `Cargo.toml`, `VISION.md` 1. | PASS |
| Rust toolchain | 1.95 (közlés) | **1.97.1** (`rust-toolchain.toml`), MSRV **1.97**, edition 2024 | `rust-toolchain.toml`, `Cargo.toml` | PASS, **a terv helyesbítendő** |
| FFmpeg | 9.0.2 | Termék: saját 9.0.2 build (`scripts/dev/build-ffmpeg-9.0.2.sh`, forrás sha256 `8c385028…e002e`, `--enable-gpl --enable-version3 --enable-shared --disable-autodetect`, opcionálisan VA-API, NVENC/NVDEC, oneVPL, dav1d, libvpx, Opus, WebP, LAME, Vulkan; x264/x265 csak a `cfa4-restricted-extras`-ben), `/opt/cfa4/ffmpeg-9.0.2`. CI-orákulum: külön statikus 9.0.2 (`scripts/ci/install-ffmpeg-9.0.2.sh`, `--enable-libopenjpeg --enable-libx264 --enable-libx265 …`) | a két szkript | PASS a forrásban; a referenciagépen lévő build BUILDINFO-ja PENDING (8. pont) |
| Qt6 | – | Referenciagép: Qt 6.10.2 (KDE Plasma 6.6.6, Wayland). Fejlesztői venv és AppImage/Flatpak: PySide6 6.12.0 (pinelve); .deb/tar.gz: Ubuntu `python3-pyside6.*` (6.10) | `reference-machine.md`, `requirements/linux-amd64.txt`, `qt-parity.md` 1.10 | PASS |
| Python | „rendszer-Python PENDING” | `requires-python >= 3.12`, mypy cél 3.12, CI 3.13 (blokkoló) és 3.14 (nem blokkoló, 2026-11-14) | `pyproject.toml`, `docs/development/linux.md` | részben; az Ubuntu 26.04 rendszer-Python pontos verziója PENDING (8. pont) |
| R0 GPU eredmények | NVENC/NVDEC PASS | Változatlan: a 2026-10-22-i R0 (RTX A1000) PASS-ai érvényesek (`reference-machine.md`); a javított libplacebo-próba fizikai eredménye még nincs | `docs/audit/R0-*.md` | PENDING (az újrafuttatás a terv szerint F0b) |
| Exportteszt-rések | J2K orákulum, HEIC, phone_vfr | Nem hiányzó tesztek, hanem **feltételesen kihagyott** tesztek: a DCP-teszt J2K/X'Y'Z' ellenőrzése ffprobe/ffmpeg nélkül csendben kimarad (`crates/export/src/dcp_tests.rs`); a HEIC round-trip `heif-enc`/`heif-dec` nélkül kihagy (`crates/export/src/still.rs`); a `phone_vfr` munkafolyamat libx264-es ffmpeg nélkül kihagy (`apps/cfa4-cli/tests/workflows.rs`) | forráskód | PENDING: hogy a CI-ben lefutnak-e, az F0b feladata (a kihagyások listája a release-riportba, kockázatnapló „zöld részteszt hamis biztonsága”) |
| Qt6 GUI készültség | kb. 53% (közlés) | Egyetlen gépi szám nem olvasható ki: a forrás a [`qt-parity.md`](../qt-parity.md) pontonkénti táblája (✅ / 🟡 / ❌, bizonyítékkal) | `qt-parity.md` | indokolt PENDING: a számszerűsítés az F0b „Qt6 funkcionális akadályok” feladatával együtt |
| Kiadási cél | – | Ubuntu 26.04 AMD64, .deb / AppImage / Flatpak / tar.gz; F9 ítélet: **HOLD** | `docs/audit/F9-RELEASE-STATUS-2026-11-08.md` | PASS |

## 3. A render-terület felépítése (a generált call graph összefoglalója)

A teljes, újragenerálható gráf: [`render-f0a/call-graph.md`](render-f0a/call-graph.md) (19 crate +
`cfa4-cli`, 72 crate-él, ~1200 modulszintű hivatkozás, 727 nyilvános típus, 90 belépési pont).

| Út | Hol van | Lényeg |
|---|---|---|
| Idővonal, projekt | `cfa4-project` (L1), `cfa4-edit` (L2), `cfa4-format` | `Arc<Project>` pillanatképek; idő mindig `cfa4_time::Tick` (254 016 000 000/s), `FrameRate` racionális |
| CPU referencia-renderer | `cfa4-render` (L3): `render_sequence`, `render_clip`, `render_item`, `plan`, `preview`, `mixer`, `audio` | Munkaformátum: `Image` = premultiplikált, lineáris fényű RGBA **f32** (`crates/render/src/image.rs`); a CPU-kompozitor az orákulum (`docs/architecture.md` 5. elv) |
| GPU-út | `cfa4-gpu` (L3): wgpu 30.0.1, WGSL, `export_renderer.rs` | Lineáris fényű premultiplikált kompozitálás; a CPU-hoz mérve (`crates/golden`) |
| Export (render → kódolás → mux) | `cfa4-export` (L3): `pipeline.rs` (képkocka-csővezeték), `job.rs` (`Exporter::step`, lépésenkénti export, megszakítás, haladás), formátumonkénti `*_out.rs` | Párhuzamos render (rayon) + soros kódolás/mux; a fájl tartalma nem függ a magszámtól (`determinism_tests.rs`) |
| Smart Cut / Smart Render | `cfa4-smart-edit` (L3), `cfa4-export` `smart_cut*.rs`, `boundary_reencode*.rs` | Csomagmásolás és határ-újrakódolás |
| Előnézet, proxy, render bar | `cfa4-render::preview`, `cfa4-engine` `previews.rs`, `proxies.rs`, `smart_render.rs` | A Qt6-felület ezeket a `cfa4-cli serve`-en éri el |
| Hang | `cfa4-render::{audio, audio_fx, mixer}`, `cfa4-audio-dsp` (BS.1770 / R128), `cfa4-export` `audio_out.rs`, `pcm.rs` | – |
| Útválasztás, háttérmotorok | `cfa4-route` (L1), `cfa4-ffmpeg`, `cfa4-gstreamer` (L5, opcionális) | ADR 0004/0005: saját út → saját szoftveres kodek → FFmpeg → GStreamer |
| Motor-homlokzat, job-hívók | `cfa4-engine` (L4), `apps/cfa4-cli` (`serve`, MCP) | Minden parancs `Session::execute(id, params)`; a Qt6 csak a `cfa4-cli serve`-en át |

### Meglévő képkocka-, idő- és job-típusok (a C0 kiindulópontja)

- **Képkocka:** `cfa4_frame::VideoFrame` (méret, `PixelData`, `ColorInfo`, PAR, PTS `Tick`,
  `FrameLayout` = mezősorrend + kroma-pozíció); `PixelData`: `Rgba8`, `RgbaF32` (premult. lineáris),
  `Yuv8`, `Yuv16` (9–16 bit, 4:2:0/4:2:2/4:4:4, opcionális alfa), `Rgb16` (R'G'B' vagy X'Y'Z',
  9–16 bit, egyenes alfa); `AudioBuffer`. A terv A2 „egységes Frame ABI” mezőinek többsége
  megvan; hiányzik a CPU/GPU-hely, a fence/szinkronizáció, a HDR side data (mastering display,
  MaxCLL/MaxFALL) frame-szintű hordozása és a forrásazonosító – ezt a C0 ellenőrzi.
- **Idő:** `Tick`, `TimeRange`, `FrameRate`, időkód-formázás (`cfa4-time`).
- **Job:** `cfa4_export::job::{Exporter, Step}`, `Stage`, haladás és megszakítás az exportban;
  a motor render-sora és a Qt6 export-sora a `cfa4-engine`/`cfa4-cli serve` parancsain át. Ez a
  terv C0 „job API v0” kiindulópontja, nem új rendszer (a terv C0 alapelve: párhuzamos
  típusrendszer nem keletkezhet).

## 4. Exportutak kodekenként (`cfa4_export::Format`)

| Csoport | Formátumok (a `Format` enum változói) | Mozi/mastering szempontból |
|---|---|---|
| Szállítás | `H264` (MP4/MOV, AAC), `Hevc` (csak regisztrált hardveres kódolóval), `Av1` (saját szoftveres, 8 bit / 10 bit PQ-HLG), `WebM` (VP9 + Opus), `Hls`, `Mpeg2` (PS/TS/ES) | – |
| Köztes kodekek | `ProRes` (422 HQ…), `DnxHr`, `Apv` (422-10…), `Cineform` (10 bit 4:2:2, 12 bit RGB/RGBA), `Mjpeg`, `MxfOp1a`, `MxfOpAtom` | MQ-1/MQ-2 jelöltek F0V-mérésre |
| Képszekvencia | `PngSequence`, `TiffSequence`, `BmpSequence`, `JpegSequence`, `TgaSequence`, `DpxSequence` (10 bit RGB), `ExrSequence` (half-float, alfa) | EXR: a terv OpenImageIO-jelöltjének (#07) összevetési alapja |
| Mozi | `Dcp` (ST 429-2, J2K 12 bit X'Y'Z', DCI 2K/4K, 5.1 24 bit; `dcp_iab.rs`/`dcp_atmos.rs`: immerzív hang), `Imf` (ST 2067-21 App #2E) | A terv „J2K orákulum-rés” és „DCP-csomagolás nyitott döntés” pontja helyesbítendő: saját implementáció **van**; ami nyitott, az a külső validálás és a DCP-o-matic-kapcsolat (7. pont) |
| Hang | `Wav`, `Aiff`, `Aac`, `Flac`, `Opus` | MQ-7 |
| Egyéb | `Gif`; állókép: HEIC/HEIF (`still.rs`) | – |

## 5. CI-jobok és utolsó eredményeik (2026-11-14 18:05 UTC)

A `main`-re pusholt HEAD (`b768709`) gyors kapui: `ci-linux` **success**, `cfa4-security`
**success**, `cfa4-source-detachment` **success**; `cfa4-gate`, `codeql`, `ci-cache-warm` a lekérdezéskor
még futott. A HEAD-et adó köteg-ág (`koteg/20261114-gazda`) teljes validációja
(„CFA4 Linux AMD64 (manual full validation)”) **success** 2026-11-14 17:17 UTC-kor; mellette a GPU-forrás
audit, a live studio, a release candidate és az F9 hold-konzisztencia is success.

| Workflow | Szerep a render-terv szempontjából | Utolsó ismert eredmény |
|---|---|---|
| `pr-check.yml` | kötelező `cargo-check` | success (köteg) |
| `ci-linux.yml` | teljes Rust/Python/Qt6 validáció (`cfa4-full-validation` címke vagy kézi indítás) | success (köteg és `main` push) |
| `ci-gpu-source.yml` | GPU-forrás audit | success (köteg) |
| `ci-native-media-source.yml`, `cfa4-f7-media-timing.yml` | natív média, A/V időzítés | success (2026-11-13 / 11-14) |
| `cfa4-ffmpeg-prebuilt.yml` | FFmpeg 9.0.2 előre fordított build | success (2026-11-11) |
| `cfa4-four-formats-hosted.yml` | négy csomagformátum | **failure** (2026-11-13, `koteg/20261113-1`) – nem render-ügy, de a terv F9 kapuját érinti |
| `cfa4-release-candidate.yml` | kiadásjelölt-ellenőrzés | success (köteg); `main` kézi futás 2026-11-08: failure |
| `dcpomatic.yml` | DCP-o-matic build (másik projekt) | success (2026-11-14) |

A terv F0V-je által előírt CI-szintek (smoke / nightly / hardware) közül a smoke-nak a
`pr-check` + a gyors kapuk, a nightly-nak a `cfa4-full-validation` címkés teljes futás, a
hardware-nek a `ci-gpu-source` felel meg legközelebb; új workflow helyett ezek bővítése javasolt
(a `.github/` kockázatos terület, `coordination.md` §2.9).

## 6. A terv moduljai és a meglévő crate-ek (javasolt leképezés)

A terv 5. fejezete szerint, ha a CFA4-ben már létezik megfelelő modul, azt kell bővíteni.

| Terv szerinti modul | Meglévő megfelelő | Javaslat |
|---|---|---|
| `cfa4-render-validation` (F0V) | `cfa4-golden` (golden PNG-k, CPU–GPU paritás), `cfa4-metrics` (ΔE₀₀, SSIM), `cfa4-qc` (riportséma, küszöbök, verdikt), `cfa4-testkit` (orákulumok, fixture-könyvtár) | Nincs új crate: a hiányzó részek (PSNR síkonként, hang null-teszt, PTS/DTS diff, mutációs önteszt, 16 bites/EXR golden, küszöbfájl) ezekbe kerülnek |
| `cfa4-render-contracts` (C0) | `cfa4-frame`, `cfa4-time`, `cfa4-color`, `cfa4-render::RenderOptions` | A2/A3 hiányzó mezőinek felvétele ezekbe, nem párhuzamos típusok |
| `cfa4-render-job-service` (C0) | `cfa4-export::job`, `cfa4-engine` render-/export-parancsai, `cfa4-cli serve` | Lásd 7. pont: átfedés a `CFA4-TASKSTORE-PG-001`-gyel |
| `cfa4-render-graph`, `-scheduler` (F3) | `cfa4-render::plan`, `cfa4-smart-edit` render plan | F3 előtt eldöntendő, bővítés vagy új crate |
| `cfa4-render-backend-cpu` / `-gpu` (F3/F4) | `cfa4-render` / `cfa4-gpu` | Ezek maradnak |
| `cfa4-imageio` (F5) | `cfa4-export` képszekvencia-írók, `cfa4-media` állókép-olvasók | D1 után |
| `cfa4-render-adapter-*` (F6) | – | Új, izolált worker (AGENTS.md: nem-Rust komponens csak felügyelt workerként) |

## 7. Átfedések más projektekkel (`coordination.md` §2.8)

Ezeken a pontokon a render-projekt **nem lép tovább**, amíg a tulajdonos és a gazda nem dönt:

| Terület | Másik projekt / dokumentum | A render-terv érintett pontja |
|---|---|---|
| DCP-mesterelés, DCP-validálás | [`CFA4-DCPOMATIC-001`](../plans/CFA4-DCPOMATIC-001-FINAL-1.0.md), [ADR 0011](../adr/ADR-0011-dcpomatic-external-mastering.md) (javasolt), [`CFA4-DCPVAL-LIBDCP-001`](../plans/CFA4-DCPVAL-LIBDCP-001-FINAL-1.0.md), [`DCP-VALIDATION-2026-10-27`](../plans/DCP-VALIDATION-2026-10-27.md) | MQ „mozi kimenet”, 14. fejezet „DCP-csomagolás nyitott döntés”, J2K-orákulum |
| Job-sor, ütemezés, események | [`CFA4-TASKSTORE-PG-001`](../plans/CFA4-TASKSTORE-PG-001.md), [`CFA4-EVENT-FABRIC-001`](../plans/CFA4-EVENT-FABRIC-001-1.0.md), [ADR 0007](../adr/ADR-0007-media-services.md) | C0 `cfa4-render-job-service`, F8 Qt6 job API |
| Smart Render, render-terv | [`CFA4-SMART-CUT-RENDER-R1`](../plans/CFA4-SMART-CUT-RENDER-R1.md), [`SMART-RENDER-2026-10-27`](../plans/SMART-RENDER-2026-10-27.md) | F3 render graph, dirty-region/cache (ReelForge-jelölt) |
| Médiaminőség-ellenőrzés | [`MEDIA-QC-2026-10-27`](../plans/MEDIA-QC-2026-10-27.md), `cfa4-qc` | F0V riportséma, küszöbök |
| OxiMedia | `cfa4-metrics` CIEDE2000/SSIM az OxiMedia 0.2.1 kódjából átírva (Apache-2.0, [`licensing.md`](../licensing.md) §5) | A terv „Kizárva: OxiMedia” sora jelöltként zárja ki; a már beépített, nyilvántartott metrika-átírás ezzel **nem** ütközik, de a tulajdonosnak érdemes megerősítenie |

## 8. Kilépési feltételek

| Feltétel (terv F0a) | Állapot |
|---|---|
| A 2. fejezet minden PENDING sorához igazolt érték vagy indokolt PENDING | **PASS**: 2. pont; a maradék PENDING-ek indokolva (F0b-feladat vagy gépspecifikus adat) |
| A call graph újragenerálható egy paranccsal | **PASS**: `python3 python/cfa4_devtools/render_inventory.py`; `--check` és a `python/tests/test_render_inventory.py` teszt jelzi, ha elavult |
| Másik gépen ugyanaz a SHA ugyanazt a lockfile-hash-t adja | **PASS**: a lockfile-ok git-követettek, a hash a `render_environment.py` kimenetében (2. pont) |
| A hivatkozott rögzítési terv és kodeksorrend dokumentum-ID-ja | **Feltételezett azonosítás, tulajdonosi megerősítésre:** kodeksorrend = [`CODEC-BACKLOG-PLAN-2026-10-27`](../plans/CODEC-BACKLOG-PLAN-2026-10-27.md) (+ [`CODEC-BACKLOG-2026-10-27`](../plans/CODEC-BACKLOG-2026-10-27.md)); rögzítés (capture/stream) = [`CFA4-LIVE-STUDIO-001`](../plans/CFA4-LIVE-STUDIO-001.md) (GStreamer/libobs, felvétel) |
| Környezet (kernel, driver, Vulkan/CUDA, display-szerver, rendszer-Python) | **Részben:** a referenciagép ismert adatai a [`reference-machine.md`](../development/reference-machine.md)-ből; a többi a gyűjtő referenciagépes futtatásáig indokolt PENDING ([`RENDER-F0a-ENVIRONMENT-2026-11-14.md`](RENDER-F0a-ENVIRONMENT-2026-11-14.md)) |

## 9. A terv helyesbítései (F0a alapján, a tulajdonos jóváhagyására)

1. Rust toolchain 1.95 → 1.97.1, MSRV 1.97; a ReelForge MSRV-feltétel (F1) teljesül.
2. A „J2K orákulum-rés” és a „DCP-csomagolás nyitott döntés” átfogalmazandó: saját DCP/IMF-export
   van; a nyitott kérdés a külső validálás (libdcp / DCP-o-matic projekt) és a kihagyott tesztek CI-lefedettsége.
3. Az 5. fejezet modulnevei a 6. pont szerinti meglévő crate-ekre képezendők.
4. Az F0V CI-szintjei a meglévő workflow-k bővítéseként (5. pont), új workflow nélkül.
5. A 7. pont átfedéseinek eldöntése az S1 / C0 előtt szükséges.

## 10. Következő lépés a terv szerint

Az F0a után párhuzamosan indulhat az **F0b/F0V** (tesztbizonyíték és validációs alap, 5–8 nap) és
az **F1** (jogi, biztonsági, komponens-audit, 5 nap). Az F0V első, átfedésmentes része a
validációs harness a meglévő `cfa4-golden` / `cfa4-metrics` / `cfa4-testkit` bővítésével
(referenciacsomag-manifest, PSNR/ΔE/SSIM síkonként, mutációs önteszt), CI-módosítás nélkül.
