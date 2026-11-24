# CFA4-FFMPEG-9.1-001 – Átállás a CFA4 saját FFmpeg-buildjében 9.0.2-ről 9.1-re

**Verzió:** 1.0 (tervezet, jóváhagyásra vár)
**Dátum:** 2026-11-24
**Projekt:** CinemaFundArchitecture4 (CFA4)
**Állapot:** a tulajdonos (`ubuntuokos`) 2026-11-24-én az irányt jóváhagyta („az irányt jóváhagyom”);
ez a terv a részleteket rögzíti. Fejlesztés csak a terv jóváhagyása után indul
([`coordination.md`](../coordination.md) §2.10).
**Forrás:** UbuntuHandbook, „FFmpeg 9.1 Released with Several New Audio/Video Codec Support”
(2026-11-21). A változáslistát az F1 elején a hivatalos `Changelog`-gal és a forrás-tarball
aláírásával kell megerősíteni (R3).
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

## III. Követelmények

| # | Követelmény | Átvételi teszt |
|---|-------------|----------------|
| R1 | Az FFmpeg-verzió **egyetlen helyen** van megadva. Szkriptek, workflowk, csomagolás, a `crates/ffmpeg` alapútvonala és a CI-cache kulcsa ebből olvas. A szkriptek nevében nincs verzió. | Új ellenőrzés (`cargo xtask` alparancs vagy CI-lépés): a verziószám a rögzített helyen és a kivételként felsorolt fájlokon (changelog, korábbi auditok, jóváhagyott tervek) kívül nem fordul elő; a régi szkriptnevek nem léteznek. |
| R2 | Az F0 viselkedést nem változtat: továbbra is 9.0.2 épül, ugyanazokra az útvonalakra. | Teljes validáció (`cfa4-full-validation`) zöld; a `package.sh` négy formátuma ugyanazt a BUILDINFO-t adja, mint előtte. |
| R3 | A 9.1-es forrás hitelesítve van (a hivatalos tarball aláírása és SHA-256-ja), és a változáslista megerősítve. | A fetch/build szkript ellenőrzi az aláírást és a checksumot; a BUILDINFO rögzíti őket; a III/IV. táblák eltérései a PR-ban jelezve. |
| R4 | A 9.1-es alapbuild és a `--restricted` változat GPL-3, nonfree nélkül; az előre fordított kiadás (`ffmpeg-9.1-cfa4-1`) elkészül. | A `build-ffmpeg` szkript licencellenőrzése; `cfa4-ffmpeg-prebuilt.yml` sikeres; a `fetch` szkript az új SHA-256-ot ellenőrzi. |
| R5 | A `crates/ffmpeg` a 9.1-gyel fordul és fut. A Rust-kötések újragenerálva (`scripts/dev/vendor_ffmpeg_bindings.py`), a minimumverzió a D5 szerint. | `cargo test -p cfa4-ffmpeg --features ffmpeg` zöld; a 9.0.2-nél régebbi, LGPL-es vagy nonfree build elutasítva (meglévő tesztek). |
| R6 | Minden FFmpeg-orákulum és fuzz-seed a 9.1-gyel fut; a bitpontos összevetések nem romlanak. | `ci-linux.yml`, `cfa4-fuzz.yml`, `cfa4-av2-oracle.yml` zöld; ha egy orákulum eredménye 9.0.2-höz képest változik, az okát a PR rögzíti. |
| R7 | A csomagok (.deb, tar.gz, AppImage, Flatpak, restricted extras) a 9.1-et szállítják, és a régi `/opt/cfa4/ffmpeg-9.0.2` telepítés frissítéskor nem marad árván. | `package.sh` négy formátuma; `cfa4-cli` a 9.1-et jelzi elérhetőnek; .deb frissítési próba 9.0.2-ről (a régi könyvtár eltávolítva vagy a csomagkezelő által kezelve). |
| R8 | ADR 0005 és a doksik a 9.1-et írják, az i18n-szöveg verziója a D5 szerint. | Doksi-átnézés; az `i18n` kulcs minden nyelven frissítve. |
| R9 | Hardveres gyorsítás felmérése a 9.1-gyel (`scripts/dev/audit-ffmpeg-hwaccel.sh`), az NVDEC CUDA Array hatása a referenciagépen (RTX A1000). | Mérési jelentés a `docs/audit/` alatt; ha a fizikai mérés kimarad, sor a [`deferred-physical-tests.md`](../development/deferred-physical-tests.md) listában (§2.14). |
| R10 | Az F2 minden új képessége a `cfa4-route` mögött, kötelező CPU-tartalékkal (a 2026-10-25-i router-döntés szerint), és csak ott, ahol a saját motor még nem tudja. | Router-tesztek: GPU nélkül a CPU-út fut; a képesség-tábla (`available(format)`) helyesen jelez. |
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
   cikkel (R3). A build-flagek változatlanok, kivéve a D2 szerinti AMF-et.
2. Az alap- és a restricted build helyi és CI-futtatása. Az előre fordított kiadás (`ffmpeg-9.1-cfa4-1`)
   a `cfa4-ffmpeg-prebuilt.yml`-lel készül.
3. A Rust-kötések újragenerálása, a `crates/ffmpeg` API-igazítása, a minimumverzió a D5 szerint.
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
| F2c | LCEVC payload-merge BSF, LCEVC az MPEG-TS muxerben | Tesztanyag-előállítás a `crates/lcevc` hátralékához (`lvc1` sáv és TS 0x36 hordozás, ma csak felismerve, 10–20 óra, [`codec-parity.md`](../codec-parity.md)). Csak orákulum; a dekódolás saját marad. | R6 |
| F2d | Panasonic RW2 dekódolás | Az első kamera-RAW, amit a CFA4 megnyit (#345, #383), az FFmpeg-háttéren át. A saját dekóder kérdése D3. | R10 |
| F2e | AMF Video Quality Enhancer | AMD-n futó minőségjavító szűrő a routerben. Csak D2 = igen esetén. | R10 |
| F2f | NVDEC CUDA Array | Dekódolási gyorsítás NVIDIA-n a meglévő NVDEC-útvonalon; a VA-API és a Vulkan útvonal egyenrangú marad. | R9, R10 |
| F2g | gainmap szűrő, JPEG MPF muxer/demuxer | HDR gain map képek (Ultra HDR / MPF) be- és kivitele. | R10 |
| F2h | DVD-Audio LPCM dekódolás, DSD kódolás | A hiányzó DVD- és hi-fi audioútvonal ([`target-app-parity.md`](../target-app-parity.md): MPEG-2 / DVD hiányzik). | R10 |
| F2i | Rhetorex ADPCM, ADPCM IMA Citrix, Infinity IMM6, MVR demuxer | Ritka formátumok megnyitása az FFmpeg-háttéren át; bekerülnek a kodek-hátralékba ([`codec-parity.md`](../codec-parity.md)) a szokásos döntéshez (saját kód vagy csak FFmpeg). | R10 |
| F2j | ASTC kódolás/dekódolás | Textúra-export a 3D/animációs munkákhoz; alacsony prioritás (D7). | R10 |
| F2k | NVIDIA optikai folyamos interpoláció | Képkocka-interpoláció NVIDIA-n, kötelező CPU-tartalékkal és egyenértékű nem-NVIDIA úttal (D6). | R10 |
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
| Windows system handle elérés | A CFA4 csak Linuxon fut. |
| iTerm2 inline image muxer | Nincs CFA4-haszna. |
| Az UbuntuHandbook PPA (`ppa:ubuntuhandbook1/ffmpeg9`) | Lecseréli az Ubuntu rendszer-FFmpegjét (ellentétes a 2026-10-24-i döntéssel: a rendszer-komponensek maradnak, a CFA4 izolált `/opt/cfa4` buildet használ). A készítője szerint „nincs jól tesztelve”. |

## VI. Nyitott tulajdonosi döntések

| # | Kérdés | Javaslat |
|---|--------|----------|
| D1 | Az F0 külön PR-ként, a 9.1 előtt kerüljön be? | **Igen.** Így a 9.1-es PR kicsi lesz, és minden későbbi FFmpeg-frissítés is. |
| D2 | Bekerüljön az AMF (`--enable-amf`) a buildbe? A fejlécek MIT-licencesek, de futáskor az AMD saját (zárt) AMF-runtime-ját tölti be, ahogy az NVENC is az NVIDIA-meghajtóét. | **Igen**, futásidőben betöltve, hiányában a VA-API/Vulkan/CPU út fut. Előbb ellenőrizni kell, hogy van-e Ubuntu 26.04-csomag a fejlécekhez; ha nincs, rögzített forrás checksummal. |
| D3 | RW2: csak FFmpeg-háttér, vagy saját dekóder is? | **Előbb FFmpeg-háttér (F2d)**; saját dekóder a kamera-RAW hátralékkal együtt, külön tervben. |
| D4 | Mikor induljon az F1? A CI-terhelés ütközhet a Qt6 teszteléssel. | Az F0 bármikor mehet (kicsi, viselkedést nem változtat); az F1 a tulajdonos által megadott időpontban, a gazda kötegén kívül, saját teljes validációval. |
| D5 | Emelkedjen-e a `crates/ffmpeg` minimumverziója 9.0.2-ről 9.1-re? | **Igen**, ha az F2 bármelyik része 9.1-es képességre épül. Addig maradhat 9.0.2, és a képesség futásidőben jelez. |
| D6 | NVIDIA optikai folyamos interpoláció (F2k): kell-e, ha nincs vele egyenértékű nem-NVIDIA út? | **Csak akkor**, ha mellé kerül egy gyártófüggetlen (Vulkan vagy CPU) interpoláció is; különben kimarad. |
| D7 | ASTC (F2j) bekerüljön-e most? | **Hátralékba** a 3D/animációs tervekhez. |

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
