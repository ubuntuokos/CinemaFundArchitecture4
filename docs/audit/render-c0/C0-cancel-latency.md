# C0: AC-09 cancel-latencia és a végleges név védelme (mérés, 2026-11-23)

Mérő: `crates/export/src/cancel_latency_tests.rs` (`cargo test -p cfa4-export --release --lib cancel_latency -- --nocapture`).
20 MJPEG-export (a 48 képkockás tesztprojekt), mindegyik más képkocka-számnál megszakítva; a latencia a `Progress::cancel`
beállítása és az `export` visszatérése közti idő.

| Mérés (3 futás, egy gép, felhős konténer) | Érték |
|---|---|
| megszakított exportok | 20 / 20 |
| latencia, medián / p95 / max | ≈ 5–7 ms / ≈ 9–11 ms / ≈ 9–12 ms |
| Küszöb@F0 az AC-09-re | **PENDING** (referenciagépes mérés; ez a szám nem küszöb, csak mért érték) |

**Nyitott, nem elrejtve (C0 kilépési feltétel: „megszakítás után végleges néven 0 csonka fájl”):** a mérésben 20 megszakításból 4-nél
**maradt fájl a végleges néven**. Ok: `export` az `Out::create_path`-on át közvetlenül a végleges útvonalra ír; az `AtomicOutput`-ot
csak a `copy/` modul használja. A #1738 SIGKILL-mérő magát az `AtomicOutput`-ot igazolja (0 csonka végleges fájl), de a fő
exportút még nincs rajta. Az ezt kikényszerítő teszt (`a_cancelled_export_leaves_no_file_under_its_final_name`) `#[ignore]`-olt,
indoklással, amíg az exportút át nem kerül az `AtomicOutput`-ra (külön C0 lépés, a `lib.rs` megosztott fájl, ezért saját PR).
