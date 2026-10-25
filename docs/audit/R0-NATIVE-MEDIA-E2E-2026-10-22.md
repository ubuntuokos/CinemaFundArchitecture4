# CFA4 R0 #16 – natív szerkesztő / FFmpeg alkalmazásszintű E2E audit

**Dátum:** 2026-10-22  
**Rögzített cél:** Ubuntu 26.04.1 LTS, Linux AMD64, Wayland, Rust 1.95, Python 3.14, Qt6/PySide6 6.10, FFmpeg 9.0.2, NVIDIA RTX A1000 / 615.71.09.  
**Alap:** a [#15](https://github.com/ubuntuokos/CinemaFundArchitecture4/pull/15) draft ágra épülő `audit/r0-native-media-e2e`; nincs merge, nincs kiadási engedély.

## 1. Forrás-alapú architektúra audit: fontos korrekció

| Képesség | CFA4 forrásban bizonyított tény | R0 státusz |
|---|---|---|
| Teljes videószerkesztő | `apps/cfa4` az `eframe`/WGPU/`cfa4-ui-egui` szerkesztőt indítja; `cfa4-cli --bridge` localhost JSON RPC-vel vezérelheti az élő appot | Valós Wayland E2E szükséges |
| Python/Qt6 | `python/cfa4_devtools/qt_app.py` saját fejlécében **„Experimental Linux Qt 6 shell. NOT the complete video editor.”**; csak a CLI-verzió kijelzését tudja | **QT6 NLE NOT_IMPLEMENTED**; a Qt6-smoke nem teljes GUI-elfogadás |
| H.264 CFA4 NVENC | `crates/platform` regisztrálja a Linux NVENC H.264 export-factoryt, `hardwareEncoding=auto` választással | Hardveres **alkalmazásszintű** exportot igazolni kell |
| HEVC CFA4 NVENC | A korábbi `crates/platform/src/nvenc/export.rs` kifejezetten **Windows-only** gyári kaput tartalmazott; a PR #17 ezt Linuxra is engedélyezi és runtime feature-probe-ot regisztrál | RTX A1000 fizikai app E2E és külön Rust NVENC-oracle: **PASS**; targeted Linux source CI: **PASS** |
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

Ezután az **igazi `target/release/cfa4` egui szerkesztőt** indítja `--empty --control <loopback-port> --data-dir <izolált auditmappa>` kapcsolókkal. A `target/release/cfa4-cli --bridge` a teljes CFA4 Session UI engine-hez csatlakozik, és az eredeti #14 hardveres H.264/HEVC/AV1 bemeneti mintákat importálja.

A H.264 és HEVC bemenetekből új CFA4 sequence-et készít, exportál H.264/HEVC NVENC outputot, FFprobe-pal mind a 8 képkockát és 640×360 méretet igazolja, **CFA4 saját médiatárolójába visszaimportál**, majd egy külön, izolált `.fcproj` projektet elment és újranyit. A `perf.stats` `decode.hardware.frames`, `export.hardware.frames` és `export.hardware.sessions` számlálóknak mérhetően nőniük kell; így a szoftveres fallback **nem** minősül automatikusan hardveres PASS-nak.

**Indítás (először egyszeri célzott binárisépítés, csak ha hiányzik):**

```bash
cd ~/GIT/CinemaFundArchitecture4
git fetch origin audit/r0-native-media-e2e
git switch --track origin/audit/r0-native-media-e2e

# Ha target/release/cfa4* nincs kész:
# cargo build --release --locked -p cfa4 -p cfa4-cli

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
| CFA4 Linux HEVC encoder gate kivezetése | SOURCE IMPLEMENTED; fizikai Rust/egui app HEVC NVENC export + FFprobe + CFA4 visszaimport PASS a 2026-10-22-i RTX A1000 futásban |
| Két külön CFA4 natív Rust hardver oracle (`r0_native_media_e2e`) | PHYSICAL PASS – RTX A1000: 2 passed, 0 failed, 0 ignored; 9.45 s |
| Valós Rust/egui GUI, Wayland, médiaimport, H.264/HEVC NVENC export és projekt roundtrip | PASS_NATIVE_ENGINE_PARTIAL_R0 – fizikai teszt 2026-10-22, lásd a 4.1. szakaszt |
| Kísérleti Qt6 shell létrehozása | PySide6 6.10.3 / Qt 6.10.2 smoke valódi Wayland sessionben: **PASS**; ez továbbra sem a kész editor |
| Valódi Qt6 NLE import/render/export | **NOT_IMPLEMENTED** (portolás külön fejlesztési scope) |
| CFA4 AV1 *natív* CUDA NVDEC | **NOT_IMPLEMENTED**; FFmpeg CLI AV1 NVDEC megléte nem elegendő |
| ICC/HDR/HEIF multi-image és színkezelés; A/V szinkron pontossága | OPEN; a 2026-10-23-i csomagpróba egy valódi klipen igazolta a látható képet, hallható hangot, alapvető lejátszást/szünetet/súrolást, exportot és visszaimportot, de nem mért szinkronpontosságot és nem fedte le a HEIF/HDR ágakat |
| HEVC + FFmpeg + Qt6 + libheif kiadási licenc/SBOM | OPEN / RELEASE HOLD |

A natív script kimenete és a kézi csomagpróba sem zárja le önmagában a teljes #16-ot: az AV1 natív CUDA, a teljes Qt6 NLE, a HEIF/HDR/ICC és pontos A/V-szinkron, valamint az SBOM/licenc és teljes terjesztési függőségleltár továbbra is önálló kapu. A kézi GUI-próba eredménye felhasználói visszajelzés; a folyamatnapló csak az indítást rögzítette, a részletes kodek- és export-metaadatokat nem.

## 4.1. Fizikai RTX A1000 futás – 2026-10-22 19:48 UTC

A célgépen rögzített `results.json` eredménye **`PASS_NATIVE_ENGINE_PARTIAL_R0`**, commit `cb70744c7983e400fe70e870ca1e0c7404e56e4c`. Környezet: Ubuntu 26.04.1 LTS, Linux x86_64, Wayland, Rust 1.95.0, Python 3.14.4, PySide6 6.10.3 / Qt 6.10.2, FFmpeg/FFprobe 9.0.2, NVIDIA RTX A1000 / 615.71.09.

Igazolt lépések: a valódi Rust/egui Wayland ablak válaszolt a loopback vezérlőn; a natív CFA4 H.264, HEVC és AV1 mintát hibamentesen importálta; H.264→H.264 és HEVC→HEVC export egyaránt 8 darab 640×360 frame-et adott, FFprobe szerint helyes kodekkel, majd a CFA4 saját importere mindkét exportot újra beolvasta. A projekt mentése és újranyitása sikerült, 0 hiányzó médiával. A `perf.stats` hardveres számlálói (NVDEC frame, NVENC frame, NVENC session) értéke (0, 0, 0)-ról (53, 16, 2)-re nőtt.

Bizonyítékfájlok a célgépen: `target/audit/native-media/20261022T194835Z/results.json`, `native-editor.log`, `runtime-linkage.json`, `ffmpeg-buildconf.txt`, valamint a két MP4 export és `native-roundtrip.fcproj`. A `target/` futási bizonyítékok nincsenek ebben a commitban feltöltve; a futásazonosító és a JSON eredmény itt kerül rögzítésre.

Ez **részleges alkalmazásszintű PASS**, nem a teljes #16 lezárása: a három bemeneti kodek importja igazolt, de AV1 sequence/dekód/render/export nem; natív CFA4 AV1 NVDEC nincs; a Qt6 shell nem teljes szerkesztő; a kézi GUI/valós hang-szinkron, ICC/HDR/HEIF multi-image nyitott.

**Runtime-linkage egyeztetés:** a célgépes `ldd` felvételekben a `cfa4` és `cfa4-cli` bináris nem mutat közvetlen `libav*` függőséget. A PR-ág `Cargo.lock` fájljában nincs FFmpeg/libav binding crate; `cfa4-codecs` Rust/Symphonia codec stackre épül, míg `cfa4-platform` a GPU-driver API-kat futásidőben tölti be. Ez alapján a CFA4-bináris és a rendszer-FFmpeg libav ABI-paritása **nem alkalmazható**. A kiadási csomag teljes runtime-dependency leltára és a futás közbeni CPU/GPU/kodek útvonalak naplózása továbbra is nyitott.

A két külön fizikai Rust-oracle ugyanazon célgépen sikeresen lefutott: `physical_nvdec_h264_hevc_reimport_matches_cfa4_software` és `physical_nvenc_h264_and_hevc_export_roundtrip`. Eredmény: **2 passed; 0 failed; 0 ignored; 9.45 s**. A célgépen rögzített napló útvonala a dátumgenerált mappában van: `target/audit/native-media/rust-oracles/<UTC-stamp>/r0-native-rust.log`. A tesztek H.264/HEVC NVDEC frame-paritást, valamint H.264/HEVC NVENC exportot, CFA4 visszaimportot és FFprobe ellenőrzést végeztek.

## 5. Következő tényleges függőség

A távoli GitHub Actions **nem fér hozzá** a felhasználó Wayland grafikus munkamenetéhez és NVIDIA GPU-eszközéhez. A pull request CI legfeljebb az új források Rust-fordíthatóságát, Python stdlib regressziós tesztjeit és policy-kompatibilitását igazolja. A #16 hardveres és grafikus runtime PASS csak a fenti **helyi eredmények** alapján jelenthető ki, nem statikus CI alapján.


## 4.2. Telepített tar-csomag kézi GUI-médiapróba – 2026-10-23

A felhasználó a frissen épített `cfa4-0.10.0-linux-x86_64.tar.gz` csomagot egy ideiglenes, majd egy tartós, felhasználói prefixbe telepítette (`~/.local/opt/cfa4-0.10.0`). A telepítő lefutott; a `cfa4-cli --version` eredménye `CFA4 CLI 0.10.0` volt. A csomagolt `cfa4` és `cfa4-cli` binárisokra futtatott `ldd` nem mutatott feloldatlan közvetlen függőséget. Ez a megfigyelés nem fedi le a dinamikusan betöltött komponenseket és az összes terjesztési formátumot.

A telepített GUI naplója szerint az alkalmazás elérte az egui eseményciklust, az NVDEC eszközpróba `Available("NVDEC")` eredménnyel zárult, a GPU-renderelő regisztrálódott, és létrejött a hangbemeneti/kimeneti backend. A folyamat 0 kilépési kóddal tért vissza. A felhasználó a tesztinteraktív kérdéseire igennel válaszolt: a valódi klip képe megjelent, hangja hallható volt, a lejátszás/szüneteltetés/súrolás működött, és a rövid export visszaimportja sikerült.

**Bizonyíték jellege és korlátai:** ez kézi, felhasználó által visszajelzett csomag-GUI smoke PASS, nem automatikus alkalmazás-orákulum. A bemásolt folyamatnapló indítási adatokat tartalmaz, de nem rögzítette a médiaútvonal részletes kodek-/hardverhasználatát, az export FFprobe-metaadatait vagy A/V-szinkron mérését. A napló szűrése azért maradt ki, mert a gépen a `rg` parancs nincs telepítve. Csak a tar formátumot vizsgáltuk; .deb/.rpm/AppImage, teljes runtime-függőségleltár és futásidejű codec-path naplózás továbbra is OPEN.
