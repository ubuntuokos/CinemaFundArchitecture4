# C0: szerződés-specifikáció v0 (VÁZLAT)

> **Állapot:** vázlat a `C0-existing-work-inventory.md` alapján. A job API alapja István 2026-11-23-i döntése: **a job API a meglévő
> `cfa4-tasks` sorra épül, második sor és állapotgép nem készül.** A terv szövege (a `cfa4-render-job-service` önálló sorként való leírása) ennek
> megfelelően módosítandó; a terv élő példánya a tulajdonos Claude Docs dokumentuma, a repó-példány annak másolata, ezért a szövegmódosítást ott kell
> átvezetni (a koordinátor/tulajdonos dolga), itt csak a döntés van rögzítve. A mezőlisták a meglévő típusokból indulnak; ami nincs ellenőrizve futtatással,
> az "ellenőrizendő".

## 1. Job API v0 (a `cfa4-tasks` Queue-ján)

**Elv:** a render egy `cfa4_tasks::Class` (osztály) a meglévő `Queue`-ban. A render-specifikus rész csak a tartalom, nem a sor: eseménytípus,
cancel-mérés, adapter az `Exporter`-hez.

**Állapotleképezés** (terv négy állapota → `cfa4_tasks::State`):

| Terv | `State` | Megjegyzés |
|---|---|---|
| Queued | Queued (Scheduled, Waiting, Blocked a sor saját állapotai) | a terv nem ismeri a Waiting/Blocked állapotot; a kliens megjeleníti |
| Running | Running (Paused külön) | |
| Completed | Succeeded | |
| Failed | Failed | |
| Cancelled | Cancelled | |
| Cancelling | Cancelling | már létezik |

A tiltott átmeneteket `cfa4_tasks::queue::transition` adja; a C0 nem ad hozzá átmenetet. Ha a render igényel (pl. Paused → Running), azt a `cfa4-tasks`
szabályai közt kell kérni, külön PR-ban.

**Render-esemény (új, vékony típus a kliensben):**

| Mező | Tartalom | Forrás |
|---|---|---|
| `job` | task azonosító | `Task.id` |
| `kind` | `progress` / `log` / `warning` / `fallback` | új |
| `progress` | képkockaindex, `done`/`total`, ETA | `cfa4_export::Progress` (`done`, `total`, `eta()`) |
| `message` | szöveg | `Progress.status` |
| `fallback` | melyik backend esett vissza, miért | `cfa4_route::Declined`/`Reason` |

**Cancel:** `Queue::cancel` kéri; a worker `Progress.cancel` flaget figyeli (kooperatív). **Mért érték:** a cancel kérés és a leállás közti idő
(AC-09), a mérőeszköz `cfa4_metrics::resource::max_pause_ms` a heartbeat-szünetre.

**Atomikus kimenet:** `cfa4_export::atomic::AtomicOutput` (ideiglenes fájl, rename). **Hiányzó teszt:** SIGKILL-ciklus (AC-08), N véletlen idejű megszakítás
után végleges néven 0 csonka fájl.

**Backendválasztás:** `cfa4_route::Router`; render-család és "alapértelmezés a meglévő renderer" szabály. **Ellenőrizendő:** van-e render-család a routing
táblában (`Family`).

**Hívó szál:** a Qt6 és a CLI a `cfa4-cli serve` vezérlőcsatornán át hajt; a job API ennek a parancskészletnek a bővítése ("minden parancs": id, címke, paraméterek).
A parancsnevek kijelölése a megvalósítás első lépése.

## 2. Frame ABI v0 (a `cfa4-frame` bővítése)

Alap: `cfa4_frame::VideoFrame` (`width`, `height`, `data: PixelData`, `color: ColorInfo`, `par`, `pts: Tick`, `layout: FrameLayout`).

| A2 mező | Státusz | Teendő |
|---|---|---|
| pixelformátum, bitmélység, alfa | megvan (`PixelData`: Rgba8, RgbaF32, Yuv8, Yuv16, Rgb16 + alfa) | – |
| primerek, transfer, mátrix, tartomány | megvan (`ColorInfo`) | – |
| méret, SAR | megvan | – |
| field order, chroma siting | megvan (`FrameLayout`) | – |
| PTS | megvan (`Tick`) | időalap-szabály rögzítendő |
| stride | **nincs** (sűrű planeek `Arc<Vec<_>>`) | döntés: marad a sűrű csomagolás, ezt a szerződés kimondja; külső (worker) határon külön leírás |
| forrás-azonosító | **megvalósítva** (`FrameSideData.source`, `cfa4_frame::side`) | a frame mellett utazik, nem benne |
| forgatás / display matrix | **megvalósítva** (`Rotation`: 0/90/180/270; más mátrix nem ábrázolható, `None`) | |
| HDR side data (mastering display, MaxCLL/MaxFALL) | a típus megvan: `cfa4_color::HdrMetadata` (nem a `ColorInfo`-ban); **megvalósítva** `FrameSideData.hdr` | |
| ownership (saját, kölcsönzött, megosztott) | `Arc` = megosztott; kölcsönzés nincs | a szerződés kimondja: az ABI értéktípus, megosztás `Arc`-kal |
| CPU/GPU hely, fence | **megvalósítva** (`Residency::{Cpu, Gpu{device, fence}}`, leíró; a `crates/gpu` típusaihoz kötés később) | |

**Elv:** csak új, opcionális mezők, hogy a meglévő hívók ne törjenek. A `VideoFrame` struct-literállal épül 54 fájlban (95 helyen), ezért az új mezők **nem a `VideoFrame`-ben**, hanem mellette, egy `FrameSideData` értékben utaznak (a 9/4. nyitott pont eldőlt). A `VideoFrame` mezőkészletét szerződésteszt rögzíti (kimerítő destrukturálás: mező hozzáadása/átnevezése nem fordul, amíg a lista és a spec nem frissül).

## 3. Audio ABI és időmodell

`cfa4_frame::AudioBuffer`: planáris `f32`, `sample_rate`, csatornák. **Hiányzik:** csatornaelrendezés-címke (5.1, 7.1; MQ-7), forrás bitmélység. **Időmodell:** `cfa4_time::Tick` (254 016 000 000/s) megvan;
a timeline → média PTS leképezés és a VFR-szabály (csendes CFR-kerekítés nélkül) **külön ellenőrizendő** egy valódi VFR klippel (F0V nyitott tétele).

## 4. Konverziós mátrix

Nincs explicit mátrix (GAP). Vázlat: sorok a (forrásformátum, célformátum, hely) hármasokból; oszlopok a végrehajtó (CPU/GPU), költség (másolás, átszámítás) és a hiba, ha nem támogatott.
Nem támogatott pár **explicit hiba**, nem csendes átalakítás. Első lépés: a meglévő konverziók felsorolása (`to_rgba8`, `convert_region`, `crates/color`) és a mátrix kitöltése belőlük.

## 5. Hiba- és capability modell, A4 szabálytábla

Alap: `cfa4_route::Error`, `Declined`, `Reason`. A render-szintű capability-leírás (melyik formátum/bitmélység/helyzet támogatott) új, vékony típus a mátrix (4.) mellett. Az A4 szabálytábla
(OpenFX-pont) a G-18 sorhoz kötődik, az ADR 0003 A4 szerint.

## 6. Worker protokoll v0

Alap: `crates/sandbox` (`Profile`, `apply`, `Mode`; még egy folyamatra sincs alkalmazva), profilfajták `mediaHost`, `pluginHost`; az F1 két újat javasol: `renderWorker`, `externalRender`.
A protokoll tartalma (terv szerint): verzió-handshake, képkocka-csere (megosztott memória vagy fájl), timeout, erőforrás-limit, cancel, útvonal-korlát. **Döntés kell:** a képkocka-csere módja (memfd/shm vagy fájl); ezt az
F2 POC mérése dönti el, a C0 csak az interfészt rögzíti. A két új profilfajtához a SECURITY-001 tulajdonosa jóváhagyása kell.

## 7. Capture I/O interfész

Csak interfész, implementáció nélkül (a terv szerint). Nincs meglévő megfelelője; új, kicsi trait a contracts crate-ben.

## 8. Változáskezelés és szerződésteszt

Szemantikus verziózás; a mezőkészletet és az állapotleképezést szerződésteszt rögzíti (pl. a `State` ↔ terv-állapot táblázat és a `VideoFrame` mezőlista). Törő változás befagyasztás után csak ADR-rel.

## 9. Nyitott

1. A `cfa4-tasks` osztálya (`Class`) a renderhez: van-e megfelelő, vagy kell új (ellenőrizendő a `manager.rs`-ben).
2. Layering: a render-kliens helye a `cargo xtask layers` szabályai szerint.
3. A parancsnevek a vezérlőcsatornán.
4. ~~A mezőkészlet-bővítés hatása a meglévő hívókra~~ eldőlt: a bővítés a `VideoFrame` mellett (`FrameSideData`), a hívók nem változnak.
