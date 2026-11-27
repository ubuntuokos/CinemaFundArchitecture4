# C0: meglévő munka leltára (A12), a szerződés-specifikáció előtt

> **Módszer:** statikus keresés a `main`-en (típusok, modulfejlécek, ADR-ek), futtatás nélkül; a sorok "létezik" állítása a megnevezett
> típus meglétét jelenti, nem azt, hogy megfelel a C0 követelményének. A C0 alapelve szerint (terv, C0 fejezet) párhuzamos típusrendszer
> nem keletkezhet; ez a leltár azt mutatja meg, mihez kell hozzányúlni, és mit kell valóban újat írni. Eredmény: a terv
> `cfa4-render-job-service` modulja nagyrészt **már létezik** a `cfa4-tasks` crate-ben.

## 1. cfa4-render-job-service tételei

| Terv-tétel | Meglévő | Státusz | Hiányzó rész |
|---|---|---|---|
| 1. Állapotgép (Queued, Running, Completed/Failed/Cancelled, Cancelling, tiltott átmenetek) | `cfa4_tasks::queue::State` (Scheduled, Queued, Waiting, Blocked, Running, Paused, Cancelling, Succeeded, Failed, Cancelled) és `transition(from, to)` | **LÉTEZIK** (bővebb a tervnél) | a terv négy állapota leképezendő; nincs új állapotgép |
| 2. Események (progress, log, figyelmeztetés, fallback-jelzés) | `QueueEvent`; az export oldalon `cfa4_export::Progress` (`done`/`total`, `status`, `error`, `eta()`) | RÉSZLEGES | nincs egységes, soros eseménytípus log, figyelmeztetés és fallback-jelzés számára |
| 3. Kooperatív cancel mért latenciával; atomikus kimenet | `Queue::cancel/cancelled`; `Progress.cancel` (AtomicBool); `crates/export/src/atomic.rs` (`AtomicOutput`: ideiglenes fájl, sync, rename; drop törli) | LÉTEZIK; a **mérés hiányzik** | cancel-latencia mérés (AC-09), SIGKILL-ciklus teszt (AC-08) |
| 4. Retry csak idempotens lépésre; prioritás | `Priority`, osztály/életkor szerinti sorrend, `checkpoint`, lease-ek (újrakezdés nélkül) | LÉTEZIK | "preview előzi az exportot" osztály-leképezése ellenőrizendő |
| 5. Szálmodell (hívó szálon semmi nem blokkol; Qt6 és CLI ugyanaz) | `Exporter::step()` lépésenként halad; a Qt6 a `cfa4-cli serve` vezérlőcsatornán át hajt | RÉSZLEGES | a közös job API-t a vezérlőcsatorna parancsaiként kell megfogalmazni |
| 6. Egyetlen feature flag / backendválasztó | `cfa4-route` (`Router`, `Backend`, `Table`, `Preferences`; mindig van CPU-tartalék) | LÉTEZIK | render-család bejegyzése és "alapértelmezés a meglévő renderer" szabály |
| 7. Vékony adapter a meglévő rendererhez + CLI kliens | `cfa4_export::Exporter` | RÉSZLEGES | az adapter és a CLI kliens új, de vékony |
| (kereszt) Erőforrás-kezelés (VRAM, RAM) | `cfa4_tasks::Manager` (`try_reserve`, `Denied::NeverFits/Wait`), `Budget`, `Monitor` | LÉTEZIK | F3 ütemező ezt használja, nem újat |

**Következtetés a job-service-re:** a `cfa4-render-job-service` ne legyen önálló sor és állapotgép. A `cfa4-tasks` ACS-4 (Queue, Manager) ADR-0008 szerint
a CFA4 minden erőforrásigényes moduljának közös infrastruktúrája, és a render nevesítve benne van. A C0 job-service v0 így egy **render-osztályú kliens**:
leképezés a `State`-re, eseménytípus, cancel-latencia mérés, adapter az `Exporter`-hez. Ez a gap-mátrix G-24 sorát (job queue, átfedés nyitott) lezárja
olyan irányban, amely nem hoz létre második sort; a PostgreSQL store (TASKSTORE-PG-001) ügyét nem érinti.

## 2. cfa4-render-contracts tételei

| Terv-tétel | Meglévő | Státusz | Hiányzó rész |
|---|---|---|---|
| 1. Frame ABI v0 (A2 teljes mezőkészlete, ownership, CPU/GPU hely, szinkron) | `cfa4_frame::VideoFrame` (méret, `PixelData`: Rgba8, RgbaF32, Yuv8, Yuv16, Rgb16 + alfa; `ColorInfo`; `par`; `pts`; `FrameLayout` mezősorrend és kroma-pozíció); `Arc<Vec<_>>` megosztott tulajdon; `pool.rs` | RÉSZLEGES | nincs: stride/plane-méret mező (a planeek sűrűn csomagoltak), forrás-azonosító, display matrix/forgatás, HDR side data (mastering display, MaxCLL/MaxFALL) a frame-en, GPU hely és fence |
| 2. Konverziós mátrix (pár, végrehajtó, költség; nem támogatott pár = hiba) | konverziók a frame és a color crate-ben szétszórva (`to_rgba8`, `convert_region`) | **GAP**: nincs explicit mátrix | táblázat (formátumpár, végrehajtó, költség) és explicit hibatípus |
| 3. Audio ABI v0 és időmodell | `cfa4_frame::AudioBuffer` (planáris `f32`, mintavételi ráta); `cfa4_time::Tick` (254 016 000 000/s) | RÉSZLEGES | nincs csatornaelrendezés-címke és bitmélység a pufferen; a Tick időmodell kész, a VFR-szabály (csendes CFR-kerekítés nélkül) külön ellenőrzendő |
| 4. Hiba- és capability modell, A4 szabálytábla | `cfa4_route::Error`, `Router` indoklásokkal (`Declined`, `Reason`); `catalog` | RÉSZLEGES | render-szintű capability-leírás nincs |
| 5. Worker protokoll v0 (külön folyamat, handshake, képcsere, timeout, limit, cancel, útvonal-korlát) | `crates/sandbox` (Landlock+seccomp, `apply`, `Profile`; még egy folyamatra sincs alkalmazva); `mediaHost`, `pluginHost` profilfajták; ADR 0007 (media services) | RÉSZLEGES | protokoll (handshake, keretcsere), profilfajták: `renderWorker`, `externalRender` (F1 javaslat) |
| 6. Capture I/O interfész implementáció nélkül | – | GAP (csak interfész) | új, kicsi |

## 3. Amit valóban újat kell írni (a C0 tényleges terjedelme)

1. Frame ABI kiegészítések a `cfa4-frame`-ben (mezők, nem új típusrendszer).
2. Explicit konverziós mátrix és "nem támogatott pár" hibatípus.
3. Render-eseménytípus (progress, log, figyelmeztetés, fallback) és a `State`-re való leképezés a `cfa4-tasks`-ban.
4. Cancel-latencia és SIGKILL/atomikus kimenet mérés (AC-08, AC-09); a mérőeszköz a `cfa4-metrics::resource` (PR #1540) része, a heartbeat-szünet már megvan.
5. Worker protokoll v0 és két új sandbox-profilfajta (SECURITY-001 tulajdonosával egyeztetve).
6. Vékony adapter és CLI kliens a vezérlőcsatornán.

## 4. Nyitott kérdések a döntéshez

1. **Új crate-ek vagy bővítés?** Az 1., 2. pont a `cfa4-frame`-et, a 3. a `cfa4-tasks`-ot érinti. A terv `cfa4-render-contracts` és `-job-service` neve megmaradhat vékony crate-ként, de a típusok forrása a meglévő crate-ekben marad. Ez a terv szövegét érinti (§2.10: tulajdonosi jóváhagyás kell, mielőtt a terv módosul).
2. **TASKSTORE-PG-001 / Event Fabric:** a job API v0 a `cfa4-tasks` `Queue`-jára épül, a PG store-tól független; a PG-store a `Queue`-szabályokat köteles teljesíteni, tehát a v0 nem zárja ki.
3. **Layering:** a `cfa4-tasks` L2, a `cfa4-frame` alacsonyabb; a render-kliens helye (L3/L4) a `cargo xtask layers` szabályai szerint ellenőrizendő a megvalósítás előtt.
