# ADR 0004 – Feladatonkénti GPU/NPU/CPU útvonalválasztó

**Dátum:** 2026-10-24 · **Állapot:** a döntések elfogadva (a tulajdonos, `ubuntuokos`, 2026-10-25-én
elfogadta a javasolt alapértelmezéseket); a 0. fázis ezzel a PR-ral · **Érintett crate:** `crates/route` (`cfa4-route`, L1 réteg)

## Háttér

A CFA4-ben ma minden gyorsítás külön úton dől el:

- a hardveres dekódolás a `crates/platform` beégetett gyársorrendjével (NVDEC → VA-API →
  szoftveres dekóder, `HybridDecoder`);
- az export kódolása a saját kapcsolójával (Export ▸ Hardveres kódolás: NVENC vagy szoftver);
- a kompozitor a wgpu adapterválasztásával (a szoftveres adapter ma csendben fut, #632);
- az MI (Whisper, Kokoro) a candle-lel, csak CPU-n.

Linuxon a hardveres kódolás csak NVIDIA-n van. Ez ellentmond a gyártófüggetlenségi szabálynak
(AGENTS.md, CFA4 rész: NVIDIA-út csak AMD/Intel-út vagy CPU-tartalék mellett). NPU-t a CFA4
egyáltalán nem használ.

## Döntés

Minden gyorsítható feladat egyetlen útvonalválasztón keresztül indul: az választja ki, melyik
háttér melyik eszközön (GPU, NPU, CPU) végzi a munkát. Ugyanaz a minta, mint a médiaháttérnél
(feladatonkénti tábla az alapsorrend fölött).

1. **A saját Rust motor marad az elsődleges** (ADR 0002). A kodek-családokban a saját utak
   (NVDEC, VA-API, NVENC, a szoftveres kodekek) állnak elöl és a végén. Az FFmpeg 9 és a
   GStreamer kiegészítő háttér, ebben a sorrendben: ott kerülnek sorra, ahol a saját motor nem tud
   valamit (Vulkan Video, VP9 és AV1 hardveresen), vagy ahol mérés igazolja, hogy jobb.
2. **Gyártófüggetlen alap.** Minden családnak van útja, amely bármelyik gyártó GPU-ján fut
   (Vulkan, wgpu, VA-API). A gyártói gyorsító (CUDA, NVENC, OpenVINO) csak gyorsabb kiegészítő.
3. **Mindig van CPU-tartalék.** Minden család sorrendje CPU-úttal zárul. A
   `Router::families_without_cpu_route` ezt ellenőrzi, a beépített tábla tesztje őrzi. Egyetlen
   kivétel a valós idejű MI-előnézet: GPU nélkül kikapcsol, de ugyanaz a funkció exportkor CPU-n
   fut.
4. **Futásidejű felmérés.** A kiadott csomag minden gépre ugyanaz; a választás az induláskori
   eszközfelmérésből (`cfa4_route::probe`) és a hátterek saját képességvizsgálatából jön.
5. **Bizonyíték, nem feltételezés** (VISION.md 2.5). Két képes út közül mérés dönti el az
   alapértelmezést.
6. **Átlátható választás.** Minden kihagyott útvonal okkal együtt rögzül (`Declined`), és a
   `perf.stats` `route.<feladat>` kulcson jelenik meg.
7. **A hardveres kódolás továbbra is bekapcsolásra vár.** A beépített enkóderek kimenete minden
   gépen bájtra azonos, ezért a `Preferences` alapértéke a kódolási családban „Csak CPU”.
8. **Nincs `unsafe`.** A `cfa4-route` csak dönt: meghajtót nem tölt be, eszközt nem nyit meg. A
   felmérés `/sys` szöveges attribútumait és szimbolikus linkjeit olvassa. Az `unsafe` kivétel
   továbbra is csak a `crates/platform`-ra vonatkozik (ADR 0001).

## Fogalmak

| Fogalom | Típus | Példa |
|---|---|---|
| Feladat | `Task` (`TaskId` + `Family`) | `video.decode.hevc`, `ai.speech.whisper` |
| Család | `Family` | `display`, `codec_decode`, `codec_encode`, `ai`, `compute` |
| Háttér | `Backend` trait, `BackendId` | `nvdec`, `vaapi`, `burn-vulkan`, `candle-cpu` |
| Eszköz | `Device`, `DeviceId` | `cpu`, `gpu:renderd128`, `npu:accel0` |
| Döntés | `Decision` (háttér + eszköz + kihagyott útvonalak) | `nvdec` @ `gpu:renderd128` |

NPU csak az `ai` családban jön szóba.

## A tábla

A kiadott alapértelmezés `crates/route/routing.toml` (telepítve: `/usr/share/cfa4/routing.toml`),
a felhasználói felülírás `~/.config/cfa4/routing.toml`. Két szintje van: családonkénti
alapsorrend, és fölötte feladatonkénti sorok.

```toml
[compute.default]
codec_decode = ["nvdec", "vaapi", "ffmpeg", "gstreamer-vulkan-video", "software"]
ai = ["openvino-npu", "burn-cuda", "burn-vulkan", "candle-cpu"]

[compute.task."ai.speech.whisper"]
order = ["burn-cuda", "burn-vulkan", "candle-cpu"]
min_free_vram_mb = 2048
```

- A felhasználói fájl családonként és feladatsor-mezőnként írja felül a kiadottat.
- Hibás fájl egészében elutasítva: ismeretlen család vagy kulcs, érvénytelen azonosító, kétszer
  szereplő háttér, túl hosszú lista (32), túl sok feladatsor (1024), 256 KiB-nál nagyobb fájl.
  Egy félig beolvasott tábla olyan helyre küldené a feladatot, amit senki nem kért.
- Más felső szintű szakaszt (például egy későbbi `[media]`) ez a crate figyelmen kívül hagy.

## A választás

A `Router::choose` végigmegy a feladat sorrendjén, és az első olyan útvonalat választja, amelyik

1. regisztrált és engedélyezett háttér (például Beállítások ▸ Lejátszás ▸ Hardveres dekódolás Ki
   letiltja az `nvdec` és `vaapi` hátteret),
2. olyan eszközön fut, amely a gépen van, és amelyet a család beállítása megenged
   (`Automatikus`, egy eszköz a CPU-val mint végső tartalékkal, vagy `Csak CPU`),
3. a háttér szerint elvállalja ezt a feladatot ezen az eszközön (`Backend::accepts`),
4. belefér a memóriakeretbe: ha a feladatsor `min_free_vram_mb`-t kér, és az eszköz szabad
   memóriája ismert, legalább annyi kell. Ismeretlen szabad memória esetén a keret nem érvényes.

Futás közbeni hiba esetén a `Router::next_after` a következő útvonalat adja. Ez csak újrakezdhető
munkára való (dekódolás, MI-darab, effekt); egy félig megírt kódolt folyam hibája a hívó hibája
marad, ahogy ma az NVENC-exportnál.

## Katalógus és gyártófüggetlenségi jelentés

A `cfa4-accel` crate (#39) beolvadt ide (a tulajdonos döntése, 2026-10-25): egyetlen router van.
Átkerült belőle:

- a háttérkatalógus (`catalog::CATALOG`): a tábla minden háttere családonként, megvalósult vagy
  tervezett állapottal, az eszközfajtával és a gyártói kötéssel (NVDEC, NVENC, Burn CUDA, OpenVINO
  NPU);
- a `Preferences::allow_vendor_locked` kapcsoló: kikapcsolva a gyártóhoz kötött hátterek
  `vendor_locked` okkal kimaradnak, így tesztelhető, hogy minden család fut nélkülük;
- a `coverage()` jelentés: családonként a megvalósult gyártófüggetlen és gyártóhoz kötött
  gyorsítók, és a tervezettek, amelyek a hiányt bezárják (`coverage_matches_the_workspace`
  teszt);
- a `/sys`-felmérés szigorúbb részei: csak számjegyre végződő csomópontnevek, numerikus sorrend,
  64 bájtos olvasás, kernelmeghajtó és PCI-eszközazonosító, megjegyzések az olvashatatlan
  bejegyzésekről, és az ellenséges bemenetek tesztjei.

A tervezett, még meg nem írt háttér a döntésben `not_implemented` okkal szerepel, a megvalósult,
de ebben a buildben nem regisztrált `not_registered` okkal.

## Rétegek

A `cfa4-route` az L1 rétegben van, munkaterületi függősége nincs (`serde`, `thiserror`, `toml`).
Így a hátteret adó crate-ek bármelyik rétegből használhatják: `speech`, `tts` (L2), `gpu`
(L3), `engine` (L4), `platform`, `gstreamer` (L5).

## Lépések

| Fázis | Tartalom | Kapu | Állapot |
|---|---|---|---|
| 0 | ADR 0004, `cfa4-route`: típusok, tábla, választás, tartalék, `/sys`-felmérés, álháttér-tesztek | CI zöld GPU nélkül | ez a PR |
| 1 | A mai utak átköltöztetése: platform-gyársorrend, NVENC-kapcsoló, wgpu-adapterválasztás, #632 | a mai tesztek változatlanul zöldek | részben kész: a Linux hardveres dekóder és az NVENC-export a routeren át, `perf.stats` → `route`; a wgpu-adapterválasztás és a #632 hátravan |
| 2 | MI GPU-n: Burn Vulkan- és CUDA-háttér a Whisperhez és a Kokoróhoz | az MI-küszöbök teljesülnek az RTX A1000-en | tervezett |
| 3 | Saját VA-API kódolás AMD-n és Intelen; kiegészítőként az FFmpeg 9, majd a GStreamer (ADR 0002, 3. lépés) | mérés Intel vagy AMD GPU-n | tervezett |
| 4 | Intel NPU OpenVINO-val; AMD NPU egyelőre csak felismerés | teszt Intel NPU-s gépen | tervezett |
| 5 | Felület: Beállítások ▸ Teljesítmény, haladó útvonal-nézet (Qt6) | kiadható, kísérleti jelölésekkel | tervezett |

A 2–4. fázis egymástól független. Minden fázis külön PR, tesztekkel.

## Döntések (elfogadva 2026-10-25)

1. Kötelező CPU-tartalék minden funkcióhoz: **igen**; kivétel csak a valós idejű MI-előnézet,
   amely GPU nélkül kikapcsol, de exportkor ugyanaz a funkció CPU-n fut.
2. „Ugyanúgy használható” jelentése: ugyanazok a funkciók minden gyártónál, családonkénti
   mércével (dekódolás bitre azonos, kódolás PSNR-küszöb, MI a 2. fázisban rögzített küszöb).
3. NPU: első körben Intel (az Intel NPU Driver 1.38 hivatalosan támogatja az Ubuntu 26.04-et).
   AMD: a kernelmeghajtó benne van az Ubuntu 26.04-ben, de az AMD futtatórétege még nem működik
   a rendszer XRT-jével, ezért egyelőre csak felismerés.
4. MI-futtatókörnyezet a candle mellé: **Burn** (Rust; Vulkan, CUDA, ROCm és CPU háttér egy
   kódból). Alternatíva: ONNX Runtime.
5. Tesztgép AMD-re és Intelre: amíg nincs, ezek az útvonalak „kísérleti” jelölést kapnak, és a
   CI-ben szoftveres Vulkan (lavapipe) ellenőrzi a gyártófüggetlen utat.
6. Médiamotor: a saját Rust motor marad az elsődleges (ADR 0002). Az FFmpeg 9 és a GStreamer
   kiegészítő háttér az útválasztó táblájában.

## Elvetett lehetőség

**NVIDIA cuda-rust** (https://github.com/NVIDIA/cuda-rust, Apache-2.0): csak NVIDIA-n fut, korai
kutatási fázisban van (API-törések várhatók), és a SIMT-modellje rögzített nightly toolchaint kér.
Az NVIDIA-utat a Burn CUDA-háttere és a meglévő NVDEC/NVENC lefedi.

## Következmények

- Új függőség: `toml` 1.x (MIT OR Apache-2.0, a `ui-egui` már használja).
- A hátterek a saját crate-jükben implementálják a `Backend` traitet; a `cfa4-route` nem függ
  tőlük.
- Az 1. fázisig a mai kód nem használja a routert, a viselkedés nem változik.
- **Szabályütközés az FFmpeg-háttérrel.** A `CLAUDE.md` ma azt írja, hogy az ffmpeg/ffprobe csak
  külső tesztorákulum és fixture-generátor, soha nem linkelt vagy szállított. Az `ffmpeg` háttér
  ezért egyelőre csak a táblában szerepel (regisztrálatlanul `NotRegistered`). A megvalósítás
  előtt a szabályt módosítani kell, és rögzíteni kell, hogyan kapcsolódik a CFA4 az FFmpeg 9-hez
  (dinamikus linkelés a rendszerből vagy külső folyamat), milyen licenccel épített FFmpeggel.
