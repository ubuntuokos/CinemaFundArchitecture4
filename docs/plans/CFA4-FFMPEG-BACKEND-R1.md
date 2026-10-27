# CFA4-FFMPEG-BACKEND-R1 — Natív FFmpeg 9 médiabackend

> **Állapot:** tulajdonos által jóváhagyott, végleges megvalósítási terv (2026-10-27). **Nem** jelenti a funkció implementálását vagy a CI-kapuk teljesülését.
>
> **Célrepo:** [ubuntuokos/CinemaFundArchitecture4](https://github.com/ubuntuokos/CinemaFundArchitecture4)
> **Kiinduló `main`:** `85b4fc1d6d40d37df65282b177946aff64ec8b9d` (2026-10-27, ADR 0005 / #224 egyesítve).
> **Platform:** Ubuntu 26.04 LTS és grafikus változatai, kizárólag Linux AMD64 / x86_64.
> **Hatókör:** architektúra, függőségek, integrációs sorrend, tesztek, csomagolás és elfogadás.
> **Tulajdonosi döntés:** az eredeti integrációs javaslat és annak FF-00–FF-09 munkacsomagjai jóváhagyva.

## 0. Véglegesített döntések és határok

1. **A saját CFA4 Rust médiamotor az elsődleges.** Megmaradnak a saját demuxerek, kodekek, kompozitor, export, időzítés és projektmodell. Az FFmpeg nem második szerkesztőmotor.
2. **Egy FFmpeg-adapter:** `crates/ffmpeg`, Cargo-név `cfa4-ffmpeg`, az L5 rétegben. A médiafeladatok a meglévő `cfa4-route`, `cfa4-media`, `cfa4-codecs`, `cfa4-export` interfészeken keresztül jutnak hozzá; közvetlen FFmpeg-hívás a GUI-ból vagy a projektmodellből tilos.
3. **Kötelező cél ABI/runtime:** az első ellenőrzött cél az FFmpeg **9.0.2**; régebbi libav-könyvtárak nem fogadhatók el. Újabb 9.x kizárólag külön verzió-/ABI-/funkcionális validáció után.
4. **Kötés kiválasztása méréssel:** elsődleges, de **feltételes** jelölt `zmwangx/rust-ffmpeg` (`ffmpeg-next`); alternatíva `dromara/rsmedia` / `rsmpeg`. A WTFPL és a tranzitív függőségek licencellenőrzése kötelező. Egyetlen kötés kerülhet a végleges adapterbe.
5. **Hibahatár:** az FFmpeg C-könyvtárakat használó tényleges médiafeldolgozás külön, belső `cfa4-ffmpeg-worker` folyamatban történik; az adapter biztonságos, verziózott IPC-n át vezérli. A worker **nem** külön felhasználói alkalmazás, és **nem** FFmpeg CLI-wrapper.
6. **Nincs új, jogosulatlan `unsafe` kivétel:** a CFA4 adapterkódja biztonságos Rust; szükséges saját FFI esetén külön ADR, biztonsági és szabályzati döntés előzi meg az implementációt. A kiválasztott harmadik fél crate-jének saját FFI-jét külön auditálni kell.
7. **Natív Stream Copy, GOP Smart Cut, Smart Render:** végrehajtásuk a CFA4 saját Rust rendszerében marad; az FFmpeg nem válhat a két stream-copy garancia rejtett végrehajtójává, és nem hajthat végre észrevétlen újrakódolást.
8. **GPU és fallback:** feladatonként a már meglévő `cfa4-route` választ; saját utak → FFmpeg → GStreamer → saját CPU/szoftveres végső út, a képesség- és profilmátrix szerint. Méréssel indokolt, feladatspecifikus felülírás megengedett; hardveres kódolás továbbra is kifejezett felhasználói választás.
9. **Csomagolás:** a CFA4 FFmpeg 9.0.2 megosztott könyvtárai elkülönítve települnek; a rendszer FFmpeg-jét nem cseréljük le. GPLv3-kompatibilis build, `--enable-nonfree` nélkül. Az opcionális, szabadalmilag korlátozott komponensek a jóváhagyott `cfa4-restricted-extras` modellbe tartoznak.
10. **Nincs upstream módosítás:** minden CFA4-bővítés downstream történik; a FilmCraft upstream változatlan.

## 1. Kapcsolódó, már elfogadott architektúra

- [ADR 0005 — FFmpeg/GStreamer médiahátterek](../adr/ADR-0005-media-backends.md): az elfogadott keret, kötelező sorrend és FFmpeg 9.0.2 minimum.
- [ADR 0004 — compute routing](../adr/ADR-0004-compute-routing.md): feladatalapú döntés és naplózható fallback.
- [ADR 0002 — GStreamer](../adr/ADR-0002-gstreamer-backend.md): GStreamer kiegészítő backend, nem FFmpeg-helyettesítő.
- [VISION.md](../../VISION.md): Qt6 az első felületi prioritás, saját Rust-motor, Linux AMD64, gyártófüggetlen gyorsítás.
- [AGENTS.md](../../AGENTS.md) és [CLAUDE.md](../../CLAUDE.md): biztonsági, minőségi és tiszta rétegezési szabályok.
- [Licencszabályzat](../licensing.md): GPL-3.0-or-later alap, elkülönített korlátozott extrák.
- [GOP Smart Cut terv](GOP-SMART-CUT-2026-10-27.md) és [Smart Render terv](SMART-RENDER-2026-10-27.md): natív exportfolyamat.
- A folyamatban lévő CFA4-SCE-001 v1.5 és WP-00/WP-01/WP-02 dokumentációja/PR-jei a saját Smart Editre vonatkoznak: **nem** képezik az FFmpeg adapter kódfüggőségét.

**Állapotkód:** `IMPLEMENTED` = összevont és igazolt forráskód, `PR_OPEN` = még nem a `main`, `PLANNED` = e tervben vállalt munka, `BLOCKED` = feltétel teljesítéséig tiltott. A terv státusza nem azonos a funkció állapotával.

A vizsgált `main`:
- `cfa4-route`: **IMPLEMENTED**, a `routing.toml` már tartalmazza a tervezett `ffmpeg` backendet.
- `cfa4-gstreamer`: **IMPLEMENTED**, opcionális médiafelfedezés és import-fallback; encode/capture további fázis.
- `cfa4-media/src/fallback.rs`: **IMPLEMENTED**, regisztrációs sorrendű import fallback.
- `cfa4-platform`: **IMPLEMENTED**, Linux hardveres dekódoló és NVENC-kódoló útvonalak.
- `cfa4-ffmpeg` és `ffmpeg-next` a munkaterületben: **PLANNED**, jelenleg nincs.
- Natív Smart Edit-végrehajtás: terve egyesítve (#222), a további fejlesztési PR-ek (#215, #216, #219) önmagukban nem tekinthetők egyesített megvalósításnak.
- A Qt6 már átvette az idővonalat, a forrásmonitort, az exportpanelt és az effektvezérlés több elemét; az egui befagyasztva. Új funkció UI-ja kizárólag Qt6.

## 2. Kötésértékelés és kötelező döntési kapu (FF-01)

| Kritérium | `ffmpeg-next` | `rsmedia/rsmpeg` |
|---|---|---|
| Modell | FFmpeg-hez közeli Rust wrapper | Magasabb szintű médiaszolgáltatások |
| CFA4 saját absztrakcióival átfedés | Kisebb | Nagyobb |
| FFmpeg 9 API | Upstream FFmpeg 9 CI és 9.0.1 forrás; helyi 9.0.2 teszt kell | FFmpeg 9 feature; pontos fork/revízió ellenőrizendő |
| Stream/packet hozzáférés | Közvetlenebb | Magasabb szintű és alacsonyabb szintű elérési út |
| Függőségi jogállás | WTFPL (`ffmpeg-next`, `ffmpeg-sys-next`): jelenleg a CFA4 deny listán nincs | MIT/Apache wrapper; pontos tranzitív ellenőrzés |
| Kockázat | FFI és helyenként low-level műveletek | Git-függőség, eltérő saját pipeline-fogalmak |

A `ffmpeg-next` első tesztjelölt, **nem előre jóváhagyott Cargo-függőség**. A GitHub-forrásban ellenőrzött `ffmpeg-next` verzió 9.0.1; a publikált és a forráskód-verzió külön rögzítendő. A `ffmpeg-sys-next` külön frissülhet: a lockfile és `cargo tree` dönti el, hogy valójában mit építünk.

**FF-01 PASS feltétele:** a kiválasztott egyetlen kötés FFmpeg 9.0.2 libav-könyvtárakkal `cargo build/test/clippy` kapukat teljesít, a szükséges decode/encode/demux/remux/seek API-kat teszt bizonyítja, a biztonsági és licenc/SBOM ellenőrzés dokumentált, és a fork/release pontos SHA/verziója rögzített. Ha bármelyik kapu bukik: másik kötés vizsgálata vagy `BLOCKED`; nincs csendes függőségi kivétel.

## 3. Rétegezés és komponensek

```text
Qt6/PySide6 GUI | cfa4-cli | Control/MCP
                |
           cfa4-engine
                |
    cfa4-route + capability/probe
                |
+---------------+--------------+---------------+
| CFA4 natív    | cfa4-ffmpeg  | cfa4-gstreamer|
| media/codecs  | (L5 adapter) | (L5)          |
| platform GPU  |              |               |
+---------------+------+-------+---------------+
                       |
                 verziózott IPC
                       |
              cfa4-ffmpeg-worker
                       |
      saját, elkülönített FFmpeg 9.0.2 .so
                |
       cfa4-export / render (közös workflow)
```

Az ábra **logikai integráció**, nem újraépített függőségi gráf: az L1–L4 crate-ek nem függhetnek az L5 FFmpeg crate-től. A felső L5/L6 réteg regisztrálja az adaptert a meglévő trait-ekhez, és onnan érhető el a `cfa4-media` és `cfa4-export` funkciója. A `cfa4-route` csak döntést hoz, FFmpeg-könyvtárat nem tölt be.

### Új tervezett útvonalak

- `crates/ffmpeg/Cargo.toml` — `cfa4-ffmpeg`, L5, kezdetben default feature nélkül.
- `crates/ffmpeg/src/probe.rs` — FFmpeg-verzió, konfiguráció, codec, hwaccel, licencek, ABI.
- `crates/ffmpeg/src/discovery.rs` — média-/stream-adatok, profilok, metadata.
- `crates/ffmpeg/src/source.rs` és `decode.rs` — `MediaSource`, `VideoDecoder` adapter.
- `crates/ffmpeg/src/encode.rs` és `mux.rs` — `VideoEncoder`, esetleges mux bridge.
- `crates/ffmpeg/src/frame.rs`, `timestamp.rs`, `metadata.rs` — CFA4 típuskonverzió.
- `crates/ffmpeg/src/route.rs` — `Backend` regisztráció és döntési indok.
- `crates/ffmpeg/src/error.rs`, `ipc.rs` — biztonságos hibák, vezérlőprotokoll.
- `apps/cfa4-ffmpeg-worker/` — belső folyamat a tényleges libav-kötéssel.

A fájlnevek tervezettek; megvalósításkor a meglévő interfészekhez igazítandók. Ugyanazt a funkciót két új crate nem implementálhatja.

### IPC-protokoll minimális követelménye

- Protokollverzió, verzió-/capability-handshake és explicit worker-build azonosító.
- Bounded, típusos request/response; validált elérési út, stream index, méret, timestamp és codec option.
- Decode, encode, probe, cancel, progress, error és shutdown műveletek.
- Műveletenkénti timeout, teljes worker-leállítás és újraindítás.
- Kezdetben korlátozott méretű chunk/frame átvitel; nagy felbontású megosztott memória csak külön mérés és jogosultság-/élettartam-audit után.
- Üzenetben nincs nyers FFmpeg pointer, memória- vagy fájlleíró korlátlan átadás.
- Nem megengedett nem megbízható bemenetből tetszőleges szűrőparancs, protokoll vagy hálózati URL automatikus futtatása.

## 4. A jelenlegi import-fallback és route összehangolása

A `cfa4-media/src/fallback.rs` jelenleg regisztrációs sorrendben próbálkozik; a `cfa4-route` feladatalapú sorrendje ettől eltérhet. **Ezt egyetlen döntéssé kell összevezetni**:

1. Beépített CFA4 megnyitó kísérlete.
2. `Unsupported`/`Decode` vagy hiányos videó esetén az L5/L6 regisztrációs réteg kikéri a `cfa4-route` döntését.
3. A router által jóváhagyott, ténylegesen rendelkezésre álló FFmpeg/GStreamer háttér sorrendben következik.
4. I/O, jogosultsági vagy offline médiahiba nem válthat át automatikusan másik decoderre.
5. Minden elutasítás `Declined`/log/`perf.stats` rekordban megjelenik.
6. Ha a natív megnyitás helyes, az alternatív háttér nem ronthatja el.

A `routing.toml` meglévő feladatsorait kell bővíteni; új, párhuzamos routing motor nem létesül. Profil- és színmélység-próba hiányában nem szabad `Supported` eredményt visszaadni.

## 5. Időzítés, kép/hang és formátumintegritás

**Időalap:** `cfa4_time::Tick` = 254 016 000 000 tick/s. Az FFmpeg racionális timebase és a CFA4 `Tick` között **ellenőrzött `i128` köztes aritmetika** történjen; nincs f64-alapú szerkesztési matematika.

Megőrzendő:
- PTS és DTS, eredeti timebase, start offset, negatív PTS/DTS.
- B-frame decode/display reorder; open/closed GOP és a seek safe point.
- VFR és 33 bites MPEG-TS timestamp-wrap.
- AAC priming/padding; hangmintaszám, delay, channel layout és A/V szinkron.
- Pixel formátum, 8/10/12-bit, 4:2:0/4:2:2/4:4:4, alpha, SAR/DAR.
- Color primaries, matrix, range, transfer PQ/HLG, mastering/CLL, ICC ahol releváns.
- Codec extradata, side data, stream disposition és nyelvi/metaadat-jelzések.

**Tilos a rejtett minőségromlás.** Ha egy CFA4 Frame/Media típus a forrásformátumot nem tudja reprezentálni, az adott út `Unsupported` vagy explicit, felhasználó által kért konverzió; nem csendes 8-bit/4:2:0 normalizálás.

## 6. Funkcionális bevezetési prioritás

### Import/dekódolás (FF-04)

Első körben csak célzottan hiányzó professzionális formátumok:
- H.264 High 10, High 4:2:2, High 4:4:4 és interlaced/MBAFF/PAFF.
- HEVC RExt 4:2:2, 4:4:4, 12-bit.
- DV/DVCPRO, MPEG-4 Part 2 és WMV/VC-1, bizonyítottan szükséges profilokra.
- Más, csak FFmpeg által támogatott formátum esetén előbb codec/container/profile teszt.

A CFA4 meglévő H.264 8-bit, HEVC Main/Main 10, ProRes, DNx, AV1, JPEG 2000 és más natív dekódolói **nem kerülnek lecserélésre**.

### Kódolás/GPU (FF-06)

- VA-API alapú H.264/HEVC/AV1 encode AMD/Intel eszközön, csak valós init és encode+decode teszt után.
- Intel QSV / Vulkan Video / AV1 hardveres dekódolás csak az adott eszközön elérhető, igazolt profilokkal.
- Natív NVDEC/NVENC és a saját szoftveres encoder útvonalak regressziómentesek.
- A hardveres encode gyárilag opt-in; CPU-only működés teljes.
- A GPU-választás támogatási kijelentése nem alapulhat kizárólag encoderlista vagy PCI-vendor felismerésén.

### Konverzió/export (FF-07)

A meglévő `cfa4-export` exportjob, renderer, progress/cancel és atomic writer marad. Az FFmpeg a hiányzó encoder-, filter-, resampler- vagy konténerképességet adhatja; új export-munkasor vagy párhuzamos Qt6 exportfelület nem készül.

## 7. Stream Copy / Smart Cut / Smart Render: kötelező határ

A jóváhagyott `CFA4-SCE-001 v1.5` és a két külön `Bitazonos` / `Kompatibilis` stream-copy garancia **saját CFA4 végrehajtást** követel.

- **Bitazonos Stream Copy:** változatlan tömörített payload és sorrend; ha nem teljesíthető, elutasítás. FFmpeg bitstream-filter/újrakódolás nem használható a garancia megtartására.
- **Kompatibilis Stream Copy:** csak az előre megengedett container/timestamp/metadata transzformációk; nincs néma újrakódolás.
- **GOP Smart Cut:** saját GOP/függőség-index, safe point, határ-újrakódolás; saját mux/packet copy.
- **Smart Render:** saját timeline planner és végrehajtó; FFmpeg codec-művelet csak explicit képességként, nem a teljes szakasztervezés helyett.
- **MPEG-TS/M2TS mux:** külön jóváhagyott natív CFA4 fejlesztés, nem FFmpeg-muxerre áttolt kötelezettség.
- **Auto mód:** csak javaslatot tehet; re-encode előtt kifejezett megerősítés.

Kapcsolódó fejlesztési munkákat/PR-eket nem kell az FFmpeg miatt újranyitni vagy megszakítani. FFmpeg import/decode profilok hozzáadhatók később, de ettől a Smart Copy képességek nem válhatnak automatikusan `Supported`-dé.

## 8. Biztonság, fallback és hibaállapotok

- A fő alkalmazás **FFmpeg nélkül is indul**; a worker-hiány `Unavailable`, nem linker/crash.
- A workerben történő natív hibát a főfolyamat izoláltan érzékeli, naplózza és kezelhető hibaként jelzi.
- Max. streamszám, képméret, felbontás, demux byte-size, időkód, csomaghossz, allokáció és időkorlát konfigurált; bemenet ellenséges.
- Nem fogadunk tetszőleges network protocol/probing URL-t automatikusan; az első kör helyi fájlokra korlátozott.
- Az eredeti média érintetlen marad, a végleges export kizárólag ellenőrzés és atomikus commit után kerül a helyére.
- Dekódolás újraindítható a következő backenddel, a codec állapotának újraépítésével.
- Félkész export nem folytatható észrevétlenül másik encoderrel; a temp kimenetet vissza kell vonni és a műveletet újra kell kezdeni.
- Crash, cancel, timeout, lemezmegtelés, könyvtárhiány, sérült packet, nem dekódolható stream és process-kill mind kap szintetikus regressziótesztet.

## 9. Runtime, licenc, disztribúció

### FFmpeg 9.0.2 build

- Reprodukálható, verziózott forrás és build recept; FFmpeg-forráskód nem kerül a CFA4 Git repóba.
- `--enable-gpl --enable-version3 --disable-nonfree`, FFmpeg-kötések és külső codec-könyvtárak pontos licence auditált.
- Az alapcsomag x264/x265 nélkül is funkcionálisan működik.
- `cfa4-restricted-extras` csak explicit felhasználói megerősítéssel, külön kompatibilis FFmpeg libav builddel vagy elkülönített workerrel; a dinamikus `libx264` telepítése önmagában nem aktiválja egy azt nélkülöző build encoderét.
- Az FFmpeg runtime és a CI külső orákulum **külön forrás/build/teszt-artefaktum**; nem szabad összekeverni.

### DEB

FFmpeg megosztott könyvtárak például `/opt/cfa4/ffmpeg-9.0.2/lib` alatt, ellenőrzött `RUNPATH`-szal; nem írhatják felül a rendszer `/usr/lib` FFmpeg-jét. A worker hiánya nem akadályozhatja a fő alkalmazás indulását.

### AppImage

AppDir-be kerülő elkülönített libav; relatív betöltési útvonal, `/opt` függőség nélkül, a rendszer FFmpeg-jének módosítása nélkül.

### Flatpak

Az alkalmazási sandbox saját FFmpeg runtime-ja, a Flatpak-jogosultságokkal és terjesztési/licencfeltételekkel összhangban.

### Ellenőrzések

`readelf -d`, `ldd`, `pkg-config`, ténylegesen betöltött libav verzió/fájlút, build flags, codec/hwaccel `probe()`, `NOTICE`/attribution/SBOM, nulla nem jóváhagyott külső Git-függőség.

**A `ffmpeg -version` CLI nem bizonyíték a worker által betöltött libav verziójára.**

## 10. Végrehajtási munkacsomagok

| ID | Feladat | Kimenet | Kötelező kapu |
|---|---|---|---|
| **FF-00** | Szabályok és ADR összhangja | AGENTS/CLAUDE/VISION/licenc politika feloldása | Nincs kötelező szabályütközés |
| **FF-01** | Wrapper és FFmpeg 9.0.2 ABI kiválasztása | Reprodukálható `ffmpeg-next` vs. `rsmedia/rsmpeg` audit | Egy kötés, lock, cargo-deny PASS |
| **FF-02** | `cfa4-ffmpeg` adapter alap | L5 crate, feature, probe, errors | FFmpeg nélkül teljes build és teszt |
| **FF-03** | Izolált `cfa4-ffmpeg-worker` | Verziózott IPC + process supervision | Worker-kill nem omlasztja össze GUI-t |
| **FF-04** | Import és decode | `MediaSource`, timestamp, frame/meta | Profilonként E2E + natív regresszió |
| **FF-05** | Egységes routing/fallback | Router-regisztráció, legacy fallback illesztés | Megismételhető döntés és napló |
| **FF-06** | Kódolás/GPU | `VideoEncoder`, VA-API/QSV/NVENC próba | HW+CPU oracle, nincs gyártói kizárás |
| **FF-07** | Export és konverzió | Export adapter, cancel, atomic writer | Import→export→reimport PASS |
| **FF-08** | DEB/Flatpak/AppImage | Izolált FFmpeg 9.0.2 runtime | Loader/licenc/csomag E2E |
| **FF-09** | Teljes regresszió | Qt6, CLI, media E2E, hibaforgatókönyvek | Kötelező CI + célgépes bizonyíték |

**Sorrend:** FF-00 → FF-01 → FF-02 → FF-03 → FF-04 → FF-05 → FF-06 → FF-07 → FF-08 → FF-09. Az FF-06 utáni codec-specifikus munkák a közös API stabilizálása után párhuzamosíthatók.

Minden munkacsomag önálló, kis PR, saját teszttel és visszavonási lehetőséggel. A dokumentum jóváhagyása **nem** jelent felhatalmazást a hiányzó minőségi/licenckapuk megkerülésére.

## 11. Tesztmátrix és igazolási bizonyíték

| Tesztosztály | Minta / környezet | PASS feltétel |
|---|---|---|
| Natív regresszió | H.264 8-bit, HEVC Main/Main10, AV1 Main, ProRes, DNx, MXF | Korábban igazolt kimenet nem romlik |
| Hiányzó professzionális import | H.264 High10/422/444/interlaced; HEVC RExt 422/444/12bit | Pontos profil- és mintamélység-egyezés |
| További formátumok | DV/DVCPRO, MPEG-4 Part 2, WMV/VC-1 | Hibamentes import és helyes seek |
| Időzítés | VFR, B-frame, negatív PTS, TS wrap, audio priming | PTS/DTS és A/V mintapontosság igazolt |
| Kép- és HDR-metadata | PQ/HLG, full/limited range, alpha, SAR | Nincs néma konverzió/metaadatvesztés |
| GPU | Nvidia, AMD, Intel / szintetikus kihagyás | Csak valós képesség; mindig CPU-fallback |
| Crash/fault | sérült bitstream, worker-kill, timeout, disk full | GUI/projekt túlél, temp cleanup |
| Stream Copy & Smart Edit | saját fejlesztések referenciatesztjei | FFmpeg nélkül is átmennek |
| Csomag | DEB/Flatpak/AppImage | Saját libav, rendszer könyvtár érintetlen |
| Futtatás FFmpeg nélkül | worker nincs vagy disabled | Natív és GStreamer utak működnek |

Kötelező ellenőrzések az aktuális CI szabályoknak megfelelően:
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`;
- `cargo xtask layers`, `cargo xtask assets`, `cargo deny check licenses sources advisories`;
- az új crate/worker célzott buildje és a FFmpeg 9.0.2 célzott oracle/E2E tesztek;
- Qt6/PySide6 regresszió, CLI/control tesztek és export-reimport;
- kiadott csomagok loader- és licenctesztjei.

A CFA4 automatikus PR/push kapuja a jelenlegi szabály szerint gyors, nem fordít; teljes validáció a dokumentált manuális workflow/PR `cfa4-full-validation` címkével indul. A lokálisan hiányzó FFmpeg- orákulumtesztet nem szabad sikeresnek feltüntetni.

## 12. Részletes elfogadási kritériumok

- [ ] Az ADR 0005 és AGENTS/CLAUDE/licenc szabályok a termékbeli FFmpeg-háttérre egységesek.
- [ ] A wrapper licenc- és verzióauditja, commit/release rögzítése kész.
- [ ] A kiválasztott kötés FFmpeg 9.0.2 mellett tesztelt; nincs nem ellenőrzött ABI.
- [ ] A CFA4 **FFmpeg nélkül is fordul és elindul**.
- [ ] A worker összeomlása nem veszít projektet és nem omlasztja a főfolyamatot.
- [ ] Egyetlen központi route/fallback döntés van, naplózott elutasításokkal.
- [ ] A professzionális profilok, időbélyegek, HDR és hangmetadata megfelelősége igazolt.
- [ ] AMD/Intel/NVIDIA út, ahol támogatott, illetve CPU fallback igazolt.
- [ ] A natív Stream Copy/Smart Cut/Smart Render teljesen független marad.
- [ ] A Qt6 import-export-reimport végponttól végpontig validált.
- [ ] A DEB, AppImage és Flatpak nem cseréli le az Ubuntu rendszer-FFmpeg-jét.
- [ ] A licencek, SBOM, notice, Cargo deny és teljes kiadási tesztkapuk PASS állapotúak.

## 13. Kockázatok, leállási és visszavonási szabályok

| Kockázat | Kezelés |
|---|---|
| WTFPL nem felel meg a `deny.toml`-nak | Licencdöntés; másik wrapper, ha nem engedélyezhető |
| Dinamikus FFmpeg ABI eltérés | Worker runtime verzió-/szimbólum- és linkellenőrzés, művelet letiltása |
| Natív C crash | Process-határ és hibatűrő IPC |
| Szín/HDR adatvesztés | Profilmátrix és explicit `Unsupported` |
| GPU-gyártóhoz kötött hiány | Route/képesség probe + CPU-fallback |
| Rejtett, második exportlánc | A `cfa4-export` egyetlen job/execution path marad |
| Smart Edit garanciák sérülése | Smart Edit natív, izolált céltesztjei kötelezők |
| GStreamer fallback ütközés | Egyetlen router sorrend + előre rögzített hibapolitika |
| Rendszer FFmpeg felülírása | Saját `RUNPATH`, sandbox/bundle izoláció és kiadási teszt |
| Szabadalmilag korlátozott kodek | Explicit telepítés külön csomagból, licenc- és működési próba |

Hibás PR nem olvasztható egybe; rollback a korábbi natív backend útvonalra. Már használt, stabil saját médiakomponenst nem szabad kizárólag helyettesítő FFmpeg út miatt eltávolítani.

## 14. Definition of Done és indulási pont

**Az R1 terv végleges**, a megvalósítás státusza **NOT_STARTED** mindaddig, amíg önálló bizonyítékkal rendelkező kód-PR-ek nem jelennek meg. A munkacsomagok `PASS` státusza kizárólag konkrét teszteredmény, CI-run és célgépes futtatási bizonyíték alapján rögzíthető.

**Első végrehajtási csomag: FF-00 + FF-01.** Az ADR- és szabálykonzisztencia, WTFPL/licenc-kapu, kötés és FFmpeg 9.0.2 ABI-prototípus nélkül semmilyen termékbeli `ffmpeg-next`/rsmpeg függőséget nem szabad véglegesíteni.

**A jóváhagyott terv célja:** a CFA4 saját Rust médiamotorja mellett célzott, profi formátumokat lefedő FFmpeg 9 háttér működjön, biztonságosan, visszavonhatóan és az új natív Smart Edit funkcióktól függetlenül.

---

**Dokumentum verziója:** R1 FINAL · **Jóváhagyva:** 2026-10-27 · **Kódváltoztatás:** a terv közzététele nem módosít médiamotort vagy buildfüggőséget.
