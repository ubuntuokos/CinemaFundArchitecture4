# CFA4-FFMPEG-9.1-001 – Átállás a CFA4 saját FFmpeg-buildjében 9.0.2-ről 9.1-re

**Verzió:** FINAL 1.4 (jóváhagyva 2026-11-24); kiegészítés 2026-11-24: D4 időpont, F1-es megállapítások, D10 (nyitott)
**Dátum:** 2026-11-24
**Projekt:** CinemaFundArchitecture4 (CFA4)
**Állapot:** a tulajdonos (`ubuntuokos`) 2026-11-24-én az irányt jóváhagyta („az irányt jóváhagyom”);
ez a terv a részleteket rögzíti. A D1–D9 döntéseket a tulajdonos 2026-11-24-én meghozta (VI. fejezet), és a tervet 2026-11-24-én véglegesen
jóváhagyta („A tervet jóváhagyom véglegesítsd”). A fejlesztés ennek alapján indulhat
([`coordination.md`](../coordination.md) §2.10); az F2 részei a IV. fejezet szerint külön jóváhagyáshoz kötöttek.
**Forrás:** a hivatalos FFmpeg-tároló (<https://github.com/FFmpeg/FFmpeg>): az `n9.1` tag
(`74e4393595cf2b5857f0da925c4e0dd74ea306e5`, 2026-11-19), a `release/9.1` ág, a `Changelog` „version 9.1”
szakasza és a `configure` függőségei, 2026-11-24-én átnézve. Az első forrás az UbuntuHandbook cikke volt
(„FFmpeg 9.1 Released with Several New Audio/Video Codec Support”, 2026-11-21); ahol a kettő eltér,
a hivatalos forrás az irányadó (II/a). A forrás-tarball aláírását az F1 ellenőrzi (R3).
**Kapcsolódik:** [ADR 0005](../adr/ADR-0005-media-backends.md) (médiaháttér, 4. lépés: saját
FFmpeg-build), [`CFA4-FFMPEG-BACKEND-R1.md`](CFA4-FFMPEG-BACKEND-R1.md),
[`CFA4-SECURITY-001.md`](CFA4-SECURITY-001.md) (az FFmpeg külön, sandboxolt folyamatban fut),
[`CFA4-SMART-CUT-RENDER-R1.md`](CFA4-SMART-CUT-RENDER-R1.md), [`crates/ffmpeg`](../../crates/ffmpeg/README.md),
[`codec-parity.md`](../codec-parity.md), [`patent-review.md`](../patent-review.md),
[`licensing.md`](../licensing.md)

---

## I. Cél

A CFA4 által szállított és a CI-ben orákulumként használt FFmpeg 9.0.2 helyére az FFmpeg 9.1 kerül.
Az új képességek közül csak az kerül be, ami (a) egy meglévő CFA4-hiányt pótol, (b) tesztorákulumként
használható, vagy (c) a GPU-router gyártófüggetlenségét erősíti. A saját Rust-motor marad az elsődleges.
Az FFmpeg továbbra is kiegészítő háttér a `cfa4-route` mögött.

## II. Kiinduló állapot (`main`, `19ad4e24`, 2026-11-24)

- A `9.0.2` **182 fájlban** szerepel beégetve. Ebből 3 szkript nevében is
  (`scripts/dev/fetch-ffmpeg-9.0.2.sh`, `scripts/dev/build-ffmpeg-9.0.2.sh`,
  `scripts/ci/install-ffmpeg-9.0.2.sh`), 7 workflowban (köztük a `contents: write` jogú
  `cfa4-ffmpeg-prebuilt.yml`), a `packaging/linux` 11 fájljában, a `crates/ffmpeg` kódjában
  (`build.rs` alapútvonal, minimumverzió-ellenőrzés, tesztek), egy UI-szövegben (az összes
  `i18n/*.tsv`-ben) és sok doksiban/auditban.
- Build: GPL-3 (`--enable-gpl --enable-version3`), `--disable-autodetect`, soha nem `--enable-nonfree`.
  Bekapcsolva: VA-API, libdrm, NVENC/NVDEC/CUVID, oneVPL, Vulkan Video, dav1d, libvpx, Opus,
  WebP, LAME. A `--restricted` változat ezen felül x264/x265-öt is tartalmaz. **AMF nincs.**
- Előre fordított build GitHub-kiadásként: `ffmpeg-9.0.2-cfa4-1`, SHA-256-tal rögzítve.
- A CI-cache kulcsa `cfa4-ffmpeg-9.0.2-ubuntu-26.04-linux-x86_64-v7`.

## II/a. A hivatalos forrás és a cikk eltérései (2026-11-24)

| Téma | A hivatalos forrás szerint | Hatás a tervre |
|------|----------------------------|----------------|
| AMF Video Quality Enhancer (`vf_vqe_amf`) | `vqe_amf_filter_deps="amf windows_h"`: **csak Windowson** épül; a `frc_amf` is. Linuxon az AMF-ből a H.264 / HEVC / AV1 kódoló és dekódoló (`*_amf`), az `sr_amf` (felskálázás) és a `vpp_amf` (skálázás, színtér) érhető el. | Az F2e eredeti tartalma Linuxon nem valósítható meg; helyette a D8 szerinti AMF-képességek. |
| ASTC | külső könyvtárral: `--enable-libastcenc`, **libastcenc 5.4 vagy újabb** (ARM astc-encoder, Apache-2.0, GPL-3-kompatibilis); `.astc` és KTX 1.0 muxer/demuxer is jön. | F2j hátralékban (D7); bekerülésekor új build-függőség. |
| NVIDIA optikai folyamos interpoláció | `vf_fruc_vulkan`: Vulkan-szűrő, de a `VK_NV_optical_flow` NVIDIA-bővítményre épül; Vulkan-fejléc ≥ 1.4.317 és SPIR-V fordító kell. | A D6 változatlanul érvényes (csak gyártófüggetlen úttal együtt). |
| APV Vulkan-kódoló | `vulkan` + `spirv_compiler`: a buildhez `glslc` vagy `glslangValidator` program kell. A mai CFA4-build ezt nem biztosítja. | F1: a `glslc` kötelező build-eszköz lesz (D9). |
| NVDEC CUARRAY | `ffnvcodec_cuarray`: az nv-codec-headers-nek ismernie kell a `CUDA_ARRAY3D_VIDEO_ENCODE_DECODE`-ot. **F1-es megállapítás (2026-11-24):** ezt csak az nv-codec-headers n13.1.15.0 (Video Codec SDK 13.1, 2026-06) tudja; az Ubuntu 26.04-ben 12.1.14 van. A 13.1-es fejlécek **legalább 610-es NVIDIA-meghajtót** írnak elő, ezért velük az NVENC/NVDEC a régebbi meghajtókon leállna. | Az F1 az Ubuntu 12.1.14-es fejléceivel épül, a CUARRAY nélkül; az F2f a D10-ig áll. |
| `libcurl` | `--enable-libcurl` autodetect; a CFA4 `--disable-autodetect`-tel épít, így magától kimarad. | R11 változatlan, az ellenőrzés marad. |
| xHE-AAC (`libfdk-xhe-aac`) | az `EXTERNAL_LIBRARY_NONFREE_LIST` tagja. | Megerősíti, hogy kimarad (V.). |
| A cikkből kimaradt tételek | erősen javított natív AAC-kódoló; H.264 data partitioning; az `ffmpeg` CLI kétsávos LCEVC-kezelése; gráf alapú bitstream-szűrő API; az `asf_o` demuxer eltávolítva; D3D12 DirectML (Windows); AVFoundation (macOS). | F2m, F2n és F2c bővül; az új BSF API a kötések újragenerálását érinti (R5); az `asf_o`-t a CFA4 nem használja; a Windows/macOS tételek kimaradnak (V.). |
| ABI | `libavcodec` 63, `libavformat` 63, `libavutil` 61 (főverziók). | A kötések újragenerálása (R5) kötelező. |

## III. Követelmények

| # | Követelmény | Átvételi teszt |
|---|-------------|----------------|
| R1 | Az FFmpeg-verzió **egyetlen helyen** van megadva. Szkriptek, workflowk, csomagolás, a `crates/ffmpeg` alapútvonala és a CI-cache kulcsa ebből olvas. A szkriptek nevében nincs verzió. | Új ellenőrzés (`cargo xtask` alparancs vagy CI-lépés): a verziószám a rögzített helyen és a kivételként felsorolt fájlokon (changelog, korábbi auditok, jóváhagyott tervek) kívül nem fordul elő; a régi szkriptnevek nem léteznek. |
| R2 | Az F0 viselkedést nem változtat: továbbra is 9.0.2 épül, ugyanazokra az útvonalakra. | Teljes validáció (`cfa4-full-validation`) zöld; a `package.sh` négy formátuma ugyanazt a BUILDINFO-t adja, mint előtte. |
| R3 | A 9.1-es forrás hitelesítve van (a hivatalos tarball aláírása és SHA-256-ja), és a változáslista megerősítve. | A fetch/build szkript ellenőrzi az aláírást és a checksumot; a BUILDINFO rögzíti őket; a III/IV. táblák eltérései a PR-ban jelezve. |
| R4 | A 9.1-es alapbuild és a `--restricted` változat GPL-3, nonfree nélkül; az előre fordított kiadás (`ffmpeg-9.1-cfa4-1`) elkészül. | A `build-ffmpeg` szkript licencellenőrzése; `cfa4-ffmpeg-prebuilt.yml` sikeres; a `fetch` szkript az új SHA-256-ot ellenőrzi. |
| R5 | A `crates/ffmpeg` a 9.1-gyel fordul és fut. A Rust-kötések újragenerálva (`scripts/dev/vendor_ffmpeg_bindings.py`), a minimumverzió 9.1 (D5). | `cargo test -p cfa4-ffmpeg --features ffmpeg` zöld; a 9.1-nél régebbi, LGPL-es vagy nonfree build elutasítva (meglévő tesztek). |
| R6 | Minden FFmpeg-orákulum és fuzz-seed a 9.1-gyel fut; a bitpontos összevetések nem romlanak. | `ci-linux.yml`, `cfa4-fuzz.yml`, `cfa4-av2-oracle.yml` zöld; ha egy orákulum eredménye 9.0.2-höz képest változik, az okát a PR rögzíti. |
| R7 | A csomagok (.deb, tar.gz, AppImage, Flatpak, restricted extras) a 9.1-et szállítják, és a régi `/opt/cfa4/ffmpeg-9.0.2` telepítés frissítéskor nem marad árván. | `package.sh` négy formátuma; `cfa4-cli` a 9.1-et jelzi elérhetőnek; .deb frissítési próba 9.0.2-ről (a régi könyvtár eltávolítva vagy a csomagkezelő által kezelve). |
| R8 | ADR 0005 és a doksik a 9.1-et írják, az i18n-szöveg „FFmpeg 9.1 vagy újabb” (D5). | Doksi-átnézés; az `i18n` kulcs minden nyelven frissítve. |
| R9 | Hardveres gyorsítás felmérése a 9.1-gyel (`scripts/dev/audit-ffmpeg-hwaccel.sh`), az NVDEC CUDA Array hatása a referenciagépen (RTX A1000). | Mérési jelentés a `docs/audit/` alatt; ha a fizikai mérés kimarad, sor a [`deferred-physical-tests.md`](../development/deferred-physical-tests.md) listában (§2.14). |
| R10 | Az F2 minden új képessége a `cfa4-route` mögött, kötelező CPU-tartalékkal (a 2026-10-25-i router-döntés szerint), és csak ott, ahol a saját motor még nem tudja. | Router-tesztek: GPU nélkül a CPU-út fut; a képesség-tábla (`available(format)`) helyesen jelez. |
| R12 | A buildben a `glslc` a SPIR-V fordító (D9); hiányában a `build-ffmpeg` szkript hibával leáll. | A szkript függőség-ellenőrzése; a BUILDINFO rögzíti a `glslc` verzióját; `ffmpeg -encoders` listázza az `apv_vulkan` kódolót. |
| R11 | A biztonsági terv érvényes marad: az FFmpeg hálózat nélkül, külön sandboxolt folyamatban fut; az új `libcurl` protokoll nincs bekapcsolva. | `ffmpeg -protocols` nem listáz `libcurl`-t; a `-buildconf` nem tartalmaz `--enable-libcurl`-t (build-szkript ellenőrzés). |

## IV. Fázisok

A fázisok csak a függőségeket rögzítik. A sorrendről és az időzítésről a tulajdonos dönt (D4).

### F0 – Egyetlen verzióforrás (verzióváltás nélkül) – R1, R2

1. Új fájl a verzióhoz (javaslat: `packaging/ffmpeg-version.env`: `CFA4_FFMPEG_VERSION=9.0.2`,
   `CFA4_FFMPEG_RELEASE_TAG=ffmpeg-9.0.2-cfa4-1` és a prebuilt SHA-256-ok).
2. A szkriptek átnevezése verzió nélkülire (`fetch-ffmpeg.sh`, `build-ffmpeg.sh`,
   `install-ffmpeg.sh`). Ezek a fájlból olvasnak.
3. A workflowk a fájlból olvasnak (egy lépés `$GITHUB_ENV`-be tölti); a cache kulcsa is ebből épül.
4. `crates/ffmpeg/build.rs`: az alapútvonal a fájlból vagy `CFA4_FFMPEG_PREFIX`-ből jön.
5. Az élő doksik (README-k, `docs/development/`, ADR 0005) verziót csak egy helyen írnak, máshol a
   fájlra hivatkoznak. **A régi auditok és jóváhagyott tervek változatlanok maradnak** (történeti szöveg).
6. Az R1 ellenőrzés bekerül a CI-be.

### F1 – Átállás 9.1-re – R3–R9, R11

1. Forrás: a hivatalos tarball letöltése, aláírás és SHA-256 ellenőrzése, a `Changelog` összevetése a
   cikkel (R3). A build-flagek változatlanok, egy kivétellel: bekerül az `--enable-amf` (D2) a Vulkan-fejléc minimuma 1.4.317-re nő (II/a), és a **`glslc` kötelező build-eszköz** lesz (D9): az Ubuntu `glslc` csomagja (forrás: shaderc; Ubuntu 24.04-en 2023.8-1build1, a 26.04-es meglétét a self-hosted runneren kell igazolni). Bekerül a `build-ffmpeg` szkript függőség-ellenőrzésébe (hiányában a build hibával leáll, nem épül csendben Vulkan-shader nélkül), a `cfa4-ffmpeg-prebuilt.yml`, a `ci-cache-warm.yml` és a többi FFmpeg-építő workflow csomaglistájába, valamint a [`docs/development/linux.md`](../development/linux.md) függőséglistájába. Az AMF-fejlécek forrása: Ubuntu 26.04-csomag, ha van; ha nincs, rögzített forrás SHA-256-tal. Az AMD AMF-runtime-ot futáskor tölti be; hiányában a VA-API, a Vulkan vagy a CPU út fut.
2. Az alap- és a restricted build helyi és CI-futtatása. Az előre fordított kiadás (`ffmpeg-9.1-cfa4-1`)
   a `cfa4-ffmpeg-prebuilt.yml`-lel készül.
3. A Rust-kötések újragenerálása, a `crates/ffmpeg` API-igazítása, a minimumverzió 9.1 (D5).
4. A cache-kulcs cseréje (`…-9.1-…-v8`); az orákulumok és fuzz-seedek újragenerálása; az eltérések
   kivizsgálása (R6). Számolni kell egy kb. 2 órás egyszeri FFmpeg-újraépítéssel.
5. Csomagolás: új útvonal (`/opt/cfa4/ffmpeg-9.1`, `lib/cfa4/ffmpeg-9.1`), RUNPATH, a régi könyvtár
   kezelése frissítéskor (R7).
6. Hardveres felmérés és NVDEC CUDA Array mérés (R9).
7. ADR 0005, `crates/ffmpeg/README.md`, i18n (R8).

### F2 – Új képességek a 9.1-ből (F1 után; mindegyik külön PR, külön tulajdonosi jóváhagyással)

| Rész | 9.1 újdonság | CFA4-haszon | Követelmény |
|------|--------------|-------------|-------------|
| F2a | APV kódolás Vulkanon | Gyártófüggetlen GPU-s APV-export útvonal a `cfa4-route`-ban; a saját `crates/apv` kódoló marad az elsődleges és a CPU-tartalék. Előbb mérés: mely GPU-kon fut, és bitrátában/minőségben hol áll a saját kódolóhoz képest. | R10 |
| F2b | APV kódolás (orákulum) | Második független APV-forrás a `crates/apv` oda-vissza tesztjéhez (az OpenAPV mellett, [ADR 0003](../adr/ADR-0003-aswf-and-conversion-sources.md) A1). | R6 |
| F2c | LCEVC payload-merge BSF, LCEVC az MPEG-TS muxerben | Tesztanyag-előállítás a `crates/lcevc` hátralékához (`lvc1` sáv és TS 0x36 hordozás, ma csak felismerve, 10–20 óra, [`codec-parity.md`](../codec-parity.md)); az `ffmpeg` CLI kétsávos LCEVC-kezelése a kétsávos minták előállításához. Csak orákulum; a dekódolás saját marad. | R6 |
| F2d | Panasonic RW2 dekódolás | Az első kamera-RAW, amit a CFA4 megnyit ([`camera-raw.md`](../camera-raw.md)), az FFmpeg-háttéren át. A saját dekóder kérdése D3. **Kész:** az FFmpeg a Bayer-mozaikot adja, a `crates/dng` hívja elő, a felhasználó telepített DNG-kameraprofiljával (`crates/ffmpeg/src/rw2.rs`). A régebbi gépek RawFormat 4-es RW2-jét az FFmpeg nem dekódolja: érthető hibát ad. | R10 |
| F2e | AMF-képességek AMD-n (az AMF VQE helyett, amely Linuxon nem épül, II/a) | H.264 / HEVC / AV1 hardveres kódolás és dekódolás (`*_amf`), `sr_amf` (felskálázás) és `vpp_amf` (skálázás, színtér) a routerben, CPU-tartalékkal; a VA-API és a Vulkan út egyenrangú marad (D2, D8). | R10 |
| F2f | NVDEC CUDA Array | Dekódolási gyorsítás NVIDIA-n a meglévő NVDEC-útvonalon; a VA-API és a Vulkan útvonal egyenrangú marad. | R9, R10 |
| F2g | gainmap szűrő, JPEG MPF muxer/demuxer | HDR gain map képek (Ultra HDR / MPF) be- és kivitele. | R10 |
| F2h | DVD-Audio LPCM dekódolás, DSD kódolás | A hiányzó DVD- és hi-fi audioútvonal ([`target-app-parity.md`](../target-app-parity.md): MPEG-2 / DVD hiányzik). | R10 |
| F2i | Rhetorex ADPCM, ADPCM IMA Citrix, Infinity IMM6, MVR demuxer | Ritka formátumok megnyitása az FFmpeg-háttéren át; bekerülnek a kodek-hátralékba ([`codec-parity.md`](../codec-parity.md)) a szokásos döntéshez (saját kód vagy csak FFmpeg). | R10 |
| F2j | ASTC kódolás/dekódolás | Textúra-kódolás és -dekódolás (libastcenc), `.astc` és KTX 1.0 be- és kivitel a 3D/animációs munkákhoz. **Hátralékba** a 3D/animációs tervekhez (D7); amikor sorra kerül, új build-függőség: libastcenc ≥ 5.4. | R10 |
| F2k | NVIDIA optikai folyamos interpoláció | Képkocka-interpoláció NVIDIA-n (`vf_fruc_vulkan`, `VK_NV_optical_flow`), de csak akkor, ha mellé egy gyártófüggetlen (Vulkan vagy CPU) interpoláció is kerül; különben kimarad (D6). | R10 |
| F2m | H.264 data partitioning | Orákulum a saját H.264-dekóderhez (data partitioning, Extended profil), ha a dekóder támogatása a kodek-hátralékba kerül. | R6 |
| F2n | Javított natív AAC-kódoló | Orákulum és összevetési alap a saját AAC-kódolóhoz; FFmpeg-tartalék a routerben az „AAC-only audio” exporthoz ([`target-app-parity.md`](../target-app-parity.md)). | R6, R10 |
| F2l | `trim` és `side_data` BSF | Smart Cut segédeszköz. **A Smart Cut munka a Qt6 befejezéséig vár** (tulajdonosi döntés, 2026-10-27), ezért ez a rész is. | – |

### F3 – Saját kód (külön tervekben, a kodek-szabályok szerint)

A 9.1 csak forrás vagy orákulum lehet. Saját dekóder/kódoló a clean-room szabály szerint
([`AGENTS.md`](../../AGENTS.md) §2): nyilvános specifikációból, vagy engedélyezett licencű kód
portjaként, a reuse-regiszterbe bejegyezve. Ide tartozik a D3 szerinti saját RW2-dekóder és az F2i
formátumok közül az, amelyről a tulajdonos saját kódot kér.

## V. Amit szándékosan **nem** veszünk át

| 9.1 újdonság | Ok |
|--------------|-----|
| xHE-AAC kódolás (`libfdk-xhe-aac`) | A Fraunhofer FDK licence nem GPL-kompatibilis; az FFmpeg az fdk-t nonfree-ként kezeli. A CFA4 soha nem épít `--enable-nonfree`-vel. |
| `libcurl` protokoll | A CFA4-SECURITY-001 szerint az FFmpeg hálózat nélkül fut; a letöltés külön folyamat dolga (R11). |
| Windows system handle elérés, D3D12 DirectML, AVFoundation-eszközválasztás, `frc_amf` / `vqe_amf` | A CFA4 csak Linuxon fut (ezek Windows- vagy macOS-only tételek). |
| iTerm2 inline image muxer | Nincs CFA4-haszna. |
| Az UbuntuHandbook PPA (`ppa:ubuntuhandbook1/ffmpeg9`) | Lecseréli az Ubuntu rendszer-FFmpegjét (ellentétes a 2026-10-24-i döntéssel: a rendszer-komponensek maradnak, a CFA4 izolált `/opt/cfa4` buildet használ). A készítője szerint „nincs jól tesztelve”. |

## VI. Tulajdonosi döntések (mind meghozva, 2026-11-24)

| # | Kérdés | Javaslat | Döntés (2026-11-24) |
|---|--------|----------|---------------------|
| D1 | Az F0 külön PR-ként, a 9.1 előtt kerüljön be? | **Igen.** Így a 9.1-es PR kicsi lesz, és minden későbbi FFmpeg-frissítés is. | igen (külön PR, a 9.1 előtt) |
| D2 | Bekerüljön az AMF (`--enable-amf`) a buildbe? A fejlécek MIT-licencesek, de futáskor az AMD saját (zárt) AMF-runtime-ját tölti be, ahogy az NVENC is az NVIDIA-meghajtóét. | **Igen**, futásidőben betöltve, hiányában a VA-API/Vulkan/CPU út fut. Előbb ellenőrizni kell, hogy van-e Ubuntu 26.04-csomag a fejlécekhez; ha nincs, rögzített forrás checksummal. | bekerül |
| D3 | RW2: csak FFmpeg-háttér, vagy saját dekóder is? | **Előbb FFmpeg-háttér (F2d)**; saját dekóder a kamera-RAW hátralékkal együtt, külön tervben. | a javaslat szerint |
| D4 | Mikor induljon az F1? A CI-terhelés ütközhet a Qt6 teszteléssel. | Az F0 bármikor mehet (kicsi, viselkedést nem változtat); az F1 a tulajdonos által megadott időpontban, a gazda kötegén kívül, saját teljes validációval. | a javaslat szerint; az F1 időpontja (2026-11-24 18:09): „amilyen hamar engedik a jelenleg futó fordítások” |
| D5 | Emelkedjen-e a `crates/ffmpeg` minimumverziója 9.0.2-ről 9.1-re? | **Igen.** | igen (a minimum 9.1) |
| D6 | NVIDIA optikai folyamos interpoláció (F2k): kell-e, ha nincs vele egyenértékű nem-NVIDIA út? | **Csak akkor**, ha mellé kerül egy gyártófüggetlen (Vulkan vagy CPU) interpoláció is; különben kimarad. | a javaslat szerint |
| D7 | ASTC (F2j) bekerüljön-e most? | **Hátralékba** a 3D/animációs tervekhez. | a javaslat szerint: hátralékba (a tulajdonos pontosítása, 2026-11-24) |
| D8 | Az F2e (AMF VQE) Linuxon nem épül. Mi kerüljön a helyére? | **AMF H.264 / HEVC / AV1 hardveres kódolás és dekódolás, plusz `sr_amf` és `vpp_amf`** a routerben, CPU-tartalékkal; így a D2 (AMF bekerül) Linuxon is hasznot hoz. | a javaslat szerint, jóváhagyva |
| D9 | Melyik SPIR-V fordító kerüljön a CFA4-buildbe? | – | a tulajdonos döntése (2026-11-24): **a `glslc`**, kötelező build-eszközként |
| D10 | Az F2f (NVDEC CUARRAY) a 610-es vagy újabb NVIDIA-meghajtót igénylő nv-codec-headers 13.1-et kívánja. Mikor kerüljön be? | **Később**: amikor a CFA4 támogatott NVIDIA-meghajtóinak minimuma eléri a 610-et (vagy két külön NVIDIA-build); addig az F1 az Ubuntu 12.1.14-es fejléceit használja. | nyitott |

## VII. PR-bontás

| PR | Tartalom | Érintett fájlok (fő) | CodeRabbit (§2.9) |
|----|----------|----------------------|-------------------|
| P0 | Ez a terv | `docs/plans/CFA4-FFMPEG-9.1-001.md` | nem |
| P1 | F0: egyetlen verzióforrás | `packaging/`, `scripts/dev/`, `scripts/ci/`, `scripts/release/`, `.github/workflows/` (köztük a `contents: write` jogú `cfa4-ffmpeg-prebuilt.yml`), `crates/ffmpeg/build.rs`, élő doksik | **igen** (kapu- és kiadás-workflow) |
| P2 | F1: 9.1 forrás, build, prebuilt kiadás, kötések, cache, orákulumok, csomagolás, ADR 0005, i18n | a fenti fájlok verzióértéke, `crates/ffmpeg`, `docs/adr/ADR-0005-media-backends.md`, `i18n/*.tsv` (csak a saját sor) | **igen** (kodek/konténer, kiadás-workflow) |
| P3 | F1 hardveres mérési jelentés (R9) | `docs/audit/` | nem |
| P4… | F2a–F2k, egy rész egy PR | a router és az érintett crate-ek | kodek/konténer részeknél igen |

## VIII. Követelmény → teszt → feladat

| Követelmény | Teszt | Fázis / PR |
|-------------|-------|------------|
| R1 | verzió-ellenőrző CI-lépés | F0 / P1 |
| R2 | teljes validáció, BUILDINFO-összevetés | F0 / P1 |
| R3 | aláírás + SHA-256 a fetch/build szkriptben | F1 / P2 |
| R4 | licencellenőrzés, prebuilt workflow | F1 / P2 |
| R5 | `cargo test -p cfa4-ffmpeg --features ffmpeg` | F1 / P2 |
| R6 | orákulum- és fuzz-workflowk | F1 / P2, F2b–c |
| R7 | `package.sh` négy formátum, .deb frissítési próba | F1 / P2 |
| R8 | doksi- és i18n-átnézés | F1 / P2 |
| R9 | hwaccel audit, referenciagépes mérés | F1 / P3 |
| R10 | router-tesztek CPU-tartalékkal | F2 / P4… |
| R11 | `-protocols` / `-buildconf` ellenőrzés | F1 / P2 |
| R12 | függőség-ellenőrzés, `-encoders` | F1 / P2 |
