# CFA4 R0 #16 – natív szerkesztő / FFmpeg alkalmazásszintű E2E audit

**Dátum:** 2026-10-22  
**Rögzített cél:** Ubuntu 26.04.1 LTS, Linux AMD64, Wayland, Rust 1.95, Python 3.14, Qt6/PySide6 6.10, FFmpeg 9.0.2, NVIDIA RTX A1000 / 615.71.09.  
**Alap:** a [#15](https://github.com/ubuntuokos/CinemaFundArchitecture4/pull/15) draft ágra épülő `audit/r0-native-media-e2e`; nincs merge, nincs kiadási engedély.

## 1. Forrás-alapú architektúra audit: fontos korrekció

| Képesség | CFA4 forrásban bizonyított tény | R0 státusz |
|---|---|---|
| Teljes videószerkesztő | `apps/filmcraft` az `eframe`/WGPU/`filmcraft-ui-egui` szerkesztőt indítja; `filmcraft-cli --bridge` localhost JSON RPC-vel vezérelheti az élő appot | Valós Wayland E2E szükséges |
| Python/Qt6 | `python/cfa4_devtools/qt_app.py` saját fejlécében **„Experimental Linux Qt 6 shell. NOT the complete video editor.”**; csak a CLI-verzió kijelzését tudja | **QT6 NLE NOT_IMPLEMENTED**; a Qt6-smoke nem teljes GUI-elfogadás |
| H.264 CFA4 NVENC | `crates/platform` regisztrálja a Linux NVENC H.264 export-factoryt, `hardwareEncoding=auto` választással | Hardveres **alkalmazásszintű** exportot igazolni kell |
| HEVC CFA4 NVENC | A korábbi `crates/platform/src/nvenc/export.rs` kifejezetten **Windows-only** gyári kaput tartalmazott, Linuxon nem működött a CFA4 HEVC export, **akkor sem**, ha az FFmpeg CLI `hevc_nvenc` használható | A PR #17 Linuxra is engedélyezi a meglévő NVENC HEVC-kódolót és regisztrálja a runtime feature-probe-okat; Rust/hosted CI és fizikai teszt szükséges |
| Linux CFA4 NVDEC | `crates/platform/src/nvdec/mod.rs` H.264 8-bit és HEVC 8/10-bit 4:2:0 natív CUDA dekódolót valósít meg. Az AV1 NVDEC **nincs implementálva ebben a Rust backendben** | H.264/HEVC CFA4 hardware-vs-software paritás mérendő; AV1 hw = **NOT_IMPLEMENTED**, FFmpeg CLI AV1 NVDEC PASS ettől független |
| Vulkan/OpenCL/libplacebo | A [#14](https://github.com/ubuntuokos/CinemaFundArchitecture4/issues/14) CLI hardveres tesztek igazoltak, de nem a Rust szerkesztő saját GPU compute/pipeline API-útvonalai | CFA4 appszintű integráció csak tényleges hívási helyek alapján vizsgálható |
| HEIF/HEIC | Első-kép import és egyképes export #10 natív libheif roundtrip tesztekkel; GUI kezelés és HDR/színprofil/multi-image **nem** igazolt | Külön nyitott GUI/colour subgate |
| Jog, SBOM | FFmpeg/HEVC, Qt6, libheif, NVENC, assetek | Külön [F1-01 #6](https://github.com/ubuntuokos/CinemaFundArchitecture4/issues/6), RELEASE HOLD |

**Nem szabad** a Rust/egui NLE Window proofját a kísérleti Qt6 shell bizonyítékaként feltüntetni, vagy az FFmpeg CLI NVDEC eredményét CFA4 natív NVDEC-ként elfogadni.

## 2. Megvalósított célzott integrációs orákulumok

A `crates/platform/tests/r0_native_media_e2e.rs` két **szigorú és alapból ignored** hardveres tesztet ad a Linux-only workspace-hez:

1. `physical_nvdec_h264_hevc_reimport_matches_cfa4_software`: pontosan a korábbi #14 hardveres `h264.mp4` és `hevc.mp4` bizonyítékminták dekódolása a **CFA4 natív NVDEC-factory** és a **CFA4 szoftveres dekóder** segítségével. Hardware session+képkocka számlálók **nőnek**, a képkockák/PTS/pixeladat paritás az `assert_same` képpontossági függvénnyel ellenőrizve, mind a nyolc frame kötelező.
2. `physical_nvenc_h264_and_hevc_export_roundtrip`: **CFA4 export pipeline**, generátorklipből 8×640×360 frame H.264 és HEVC NVENC-kódolása, natív hardware session/frame counter kötelező növekedése, saját CFA4 dekóderrel visszaolvasás, külső FFprobe kodek/méret/8 frame integritás.

A tesztek **nem** fogadják el a hardver hiányát sikeres kihagyásként. A normál GitHub CI csak lefordítja őket (`--no-run`), sosem minősíti a hostolt runner software-GPU-ját valós NVIDIA PASS-nak.

## 3. Valódi, élő natív CFA4 alkalmazás próba – a saját Wayland gépen

A `python/cfa4_devtools/native_media_e2e.py` ellenőrzi a pontos Ubuntu/Python/Wayland/NVIDIA/FFmpeg környezetet, **csak a meglévő futtatható binárisokat** használja; először külön teszteli a kísérleti PySide6 Qt shell létrehozását `--smoke` módban (ennek eredménye nem teljes szerkesztő).

Ezután az **igazi `target/release/filmcraft` egui szerkesztőt** indítja `--empty --control <loopback-port> --data-dir <izolált auditmappa>` kapcsolókkal. A `target/release/filmcraft-cli --bridge` a teljes CFA4 Session UI engine-hez csatlakozik, és az eredeti #14 hardveres H.264/HEVC/AV1 bemeneti mintákat importálja.

A H.264 és HEVC bemenetekből új CFA4 sequence-et készít, exportál H.264/HEVC NVENC outputot, FFprobe-pal mind a 8 képkockát és 640×360 méretet igazolja, **CFA4 saját médiatárolójába visszaimportál**, majd egy külön, izolált `.fcproj` projektet elment és újranyit. A `perf.stats` `decode.hardware.frames`, `export.hardware.frames` és `export.hardware.sessions` számlálóknak mérhetően nőniük kell; így a szoftveres fallback **nem** minősül automatikusan hardveres PASS-nak.

**Indítás (először egyszeri célzott binárisépítés, csak ha hiányzik):**

```bash
cd ~/GIT/CinemaFundArchitecture4
git fetch origin audit/r0-native-media-e2e
git switch --track origin/audit/r0-native-media-e2e

# Ha target/release/filmcraft* nincs kész:
# cargo build --release --locked -p filmcraft -p filmcraft-cli

# Csak a natív app és PySide6 Qt shell; meglévő FFmpeg fixture:
bash scripts/dev/audit-native-media-e2e.sh

# Opcionálisan a két CARGO hardver-integrációs oracle IS:
# bash scripts/dev/audit-native-media-e2e.sh --rust-gates
```

A `target/audit/native-media/<UTC-stamp>/results.json` és `native-editor.log` megtartja az eredményt, GUI-felhasználói profilt nem módosítunk. Az opcionális Rust hardware fixture és output napló külön `target/audit/native-media/rust-oracles/` alatt keletkezik. A teljes workspace-release orákulumot **nem** ismételjük.

## 4. Elfogadási döntési mátrix

| Tétel | R0 döntés 2026-10-22 |
|---|---|
| Korábbi GPU/WGPU shader parity #12 | PASS / CLOSED |
| Korábbi FFmpeg CLI HW gyorsítás #14 | PASS / CLOSED (24 pass + külön libplacebo 8-frame PASS) |
| CFA4 Linux HEVC encoder gate kivezetése | SOURCE IMPLEMENTED; fordítás és fizikai CFA4 export teszt még függőben |
| Két CFA4 natív Rust hardver oracle | TEST CODE ADDED / NATIVE NOT_RUN |
| Valós Rust/egui GUI, Wayland, media import/GUI backend/export | AUTOMATION ADDED / PHYSICAL NOT_RUN |
| Kísérleti Qt6 shell létrehozása | NATIVE NOT_RUN; **nem a kész editor** |
| Valódi Qt6 NLE import/render/export | **NOT_IMPLEMENTED** (portolás külön fejlesztési scope) |
| CFA4 AV1 *natív* CUDA NVDEC | **NOT_IMPLEMENTED**; FFmpeg CLI AV1 NVDEC megléte nem elegendő |
| ICC/HDR/HEIF multi-image, médiahang-szinkron, valódi UI-scrub | OPEN; a szintetikus 8-frame próba ezeket nem igazolja |
| HEVC + FFmpeg + Qt6 + libheif kiadási licenc/SBOM | OPEN / RELEASE HOLD |

A natív script kimenete akkor sem zárja le önmagában a teljes #16-ot, ha a kontrollált frontendben minden scripted step sikeres: a **kézi GUI-interakció, valós médiahang, AV1 native CUDA, Qt6 NLE hiány és teljes SBOM/licencek** önálló kapuk.

## 5. Következő tényleges függőség

A távoli GitHub Actions **nem fér hozzá** a felhasználó Wayland grafikus munkamenetéhez és NVIDIA GPU-eszközéhez. A pull request CI legfeljebb az új források Rust-fordíthatóságát, Python stdlib regressziós tesztjeit és policy-kompatibilitását igazolja. A #16 hardveres és grafikus runtime PASS csak a fenti **helyi eredmények** alapján jelenthető ki, nem statikus CI alapján.
