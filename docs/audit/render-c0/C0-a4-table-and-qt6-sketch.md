# C0 – A4 szabálytábla és Qt6 integrációs vázlat (VÉGLEGES)

**Dátum:** 2026-11-25 · **Állapot:** VÉGLEGES, JÓVÁHAGYVA (István, 2026-11-25: A1 a szabálytábla T1/T2 ajánlásokkal, A2 a Qt6 lépések stackelt implementációs PR-okban). Zárja a C0 két megmaradt átadandóját.
**Alap:** C0 spec v0.1 (VÉGLEGES), `C0-candidate-mapping.md` (a #1845-ben), a `main` (`cfa4-frame::convert`, `cfa4-render-contracts::capability`, `cfa4-engine::render_job`, `python/cfa4_qt`).
**Mit nem ellenőriztem:** nem futtattam mérést; a GPU-oszlopok és a referenciagépes értékek PENDING. A Qt6 elfogadás szövege külön nem kell: a spec 10. pontja szerint István mint a Qt6 felelőse a job API-t 2026-11-24-én elfogadta.

## 1. Megjegyzés a feladat előzményéhez

A jelölt-leképezés (S1) már kész és jóváhagyott (`C0-candidate-mapping.md`, István 2026-11-25), a hozzá tartozó szerződésmódosítás (worker kimeneti index, 0.2) a #1845-ben nyitott. Ezért új leképezést nem írtam. Ami a C0 átadandói közül még hiányzik: az **A4 szabálytábla** és a **Qt6 integrációs vázlat**; ez a dokumentum ezt adja.

## 2. A4 szabálytábla

**Elv (A4):** exportnál FAIL_CLOSED (nevesített hiba), előnézetnél JELZETT_FALLBACK (`RenderEvent` `fallback`), csendes minőségromlás soha. A döntést a meglévő `decide(&RenderCapability, Context)` adja; a tábla azt mondja meg, **mikor `supported: false`**.

### 2.1 Formátumpárok (a `cfa4_frame::convert::plan` szerint, mind az 5×5 pár tesztelt)

| Cél ↓ / Forrás → | azonos | bármi más |
|---|---|---|
| `RgbaF32` | Share | Convert, veszteségmentes |
| `Rgba8` | Share | Convert, **veszteséges** |
| `Yuv8`, `Yuv16`, `Rgb16` | Share | **Unsupported** |

| Helyzet | Export | Előnézet |
|---|---|---|
| Veszteségmentes konverzió | rendben | rendben |
| Veszteséges konverzió (→ `Rgba8`) | **FAIL_CLOSED**, ha a kimenet mozi/archív cél (MQ-1); egyébként megengedett és a naplóban jelölt | rendben, a kijelző 8 bit |
| `Unsupported` pár | **FAIL_CLOSED** `Unsupported{from,to}` | JELZETT_FALLBACK (a lépés kimarad vagy `RgbaF32`-n át megy, esemény) |

### 2.2 Képesség × környezet

| Képesség hiányzik | Export | Előnézet | Megjegyzés |
|---|---|---|---|
| Worker nem indul / kézfogás `Reject` / inkompatibilis verzió | FAIL_CLOSED | JELZETT_FALLBACK: a worker szűrője kimarad | a `negotiate` szabálya: azonos major, 1.0 előtt azonos minor |
| Worker timeout / erőforrás-limit túllépés | FAIL_CLOSED, részkimenet nem jelenik meg (atomikus kimenet) | JELZETT_FALLBACK | `ResourceLimits` nulla érték = érvénytelen |
| Útvonal a `PathPolicy`-n kívül | FAIL_CLOSED | FAIL_CLOSED (biztonsági, nincs fallback) | |
| EC-jelölt 32 bit/csatorna mód nélkül | FAIL_CLOSED (MQ-1) | JELZETT_FALLBACK | a belépési feltétel (`RgbaF32`, 32, Cpu) |
| Többcsatornás (AOV) kép | FAIL_CLOSED `Unsupported` | FAIL_CLOSED `Unsupported` | M3: v0-ban nem támogatott, ADR az F4 előtt |
| GPU hely (`Place::Gpu`) nem elérhető | a CPU referencia fut (A5), **és ez esemény**; a CPU-kimenet a bit-azonos | CPU, esemény | nem minőségromlás, mert a CPU a referencia (MQ-6); GPU küszöb PENDING |
| Matte-sík hiányzik / érvénytelen (`MatteError`) | FAIL_CLOSED | JELZETT_FALLBACK: matte nélkül, esemény | hostile-input teszt már van |
| VFR / változó képkockaszám (VapourSynth) | a worker `Describe`-ja az arányt közli; a progress összege változhat, nem hiba | ugyanaz | 0.2, #1845 |

### 2.3 Nyitott döntések a táblához
- **T1.** A veszteséges `Rgba8` konverzió exportnál mikor „mozi cél”: javaslat: ha a kimeneti profil DCP/IMF/ProRes 4444/EXR (az F9 profilkészlet szerint), akkor FAIL_CLOSED; a többinél jelölt engedély. *Ajánlás: igen.*
- **T2.** A GPU→CPU visszalépés exportnál csendes lehet-e? *Ajánlás: nem; esemény kell, de nem hiba.*

## 3. Qt6 integrációs vázlat

**Elv (A8):** a GUI főszálán nincs hosszú művelet; a Qt6 ugyanazt a job API-t olvassa, mint a CLI. A Qt6 a motort `cfa4-cli serve` JSON-csatornán éri el, ezen nem változtatunk.

### 3.1 Ma
- Beküldés: meglévő export-parancsok (`file.exportMedia`, `export.queue.add`), amelyek `job` azonosítót adnak.
- Követés: `jobs.list` lekérdezés időzítővel (`job_watch`, `main_window.py`), megszakítás `jobs.cancel`.

### 3.2 Javasolt (kis lépések, nincs törő változás)
1. **Olvasás:** a progress-panel `render.list`-et is kérheti ugyanarra a jobra: a `state` (`RenderState`) és az `event` (`RenderEvent`) típusos, a `fallback` és `warning` esemény így a Qt6-ban is megjelenik (toast), nem csak a naplóban. A `jobs.list` marad (kompatibilitás).
2. **Cancel:** a `jobs.cancel` és a `render.cancel` ugyanazt a jelzőt állítja; a Qt6 a Cancelling állapotot külön mutatja (a latencia ms-tartományú, a felület nem vár).
3. **Hibaüzenet:** `Unsupported{from,to}` és `FailClosed` hiba emberi szövegre fordítva (a név megmarad a naplóban).
4. **Verziókezelés:** a Qt6 a `ContractVersion`-t a `serve` kézfogásából olvassa; ismeretlen major esetén a render-panel „csak olvasható” módba áll, nem omlik össze.
5. **Tesztek:** a meglévő nem blokkoló `test_qt_gui.py` bővítése: fallback-esemény megjelenik, cancel után a panel Cancelled; blokkoló csak a Rust oldali szerződésteszt.

### 3.3 Amit ez nem tesz
Nem ad új képernyőt, nem írja át a Qt6 paritáslistát, nem érinti az egui frontendet. A Python/Qt oldali változás külön, implementációs PR-ban jönne, jóváhagyás után.

## 4. Döntések (István)

| # | Kérdés | Ajánlás |
|---|---|---|
| A1 | Elfogadod a 2. pont táblázatait a C0 A4 szabálytáblájaként (T1, T2 ajánlással)? | **Elfogadva** |
| A2 | A 3.2 pont a Qt6 integrációs vázlat, és az 1–5 lépés mehet implementációs PR-okban (stackelve)? | **Elfogadva** |

A Qt6 lépések (3.2) külön PR-okban, stackelve.
