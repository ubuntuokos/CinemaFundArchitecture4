# C0: AC-09 cancel-latencia és a végleges név védelme (mérés, 2026-11-23)

Mérő: `crates/export/src/cancel_latency_tests.rs` (`cargo test -p cfa4-export --release --lib cancel_latency -- --nocapture`).
20 MJPEG-export (a 48 képkockás tesztprojekt), mindegyik más képkocka-számnál megszakítva; a latencia a `Progress::cancel`
beállítása és az `export` visszatérése közti idő.

| Mérés a javítás előtt (3 futás, egy gép, felhős konténer) | Érték |
|---|---|
| megszakított exportok | 20 / 20 |
| latencia, medián / p95 / max | ≈ 5–7 ms / ≈ 9–11 ms / ≈ 9–12 ms |
| Küszöb@F0 az AC-09-re | **PENDING** (referenciagépes mérés; ez a szám nem küszöb, csak mért érték) |

**Rés megtalálva és bezárva (C0 kilépési feltétel: „megszakítás után végleges néven 0 csonka fájl”):** az első mérésben 20 megszakításból
4-nél maradt fájl a végleges néven, mert `export` az `Out::create_path`-on át közvetlenül a végleges útvonalra írt, és az `AtomicOutput`-ot csak
a `copy/` modul használta. Javítás: az `Out::create` (a fő, egyfájlos kimenet) az új `Out::Atomic` változatot adja, amely ideiglenes fájlba
ír, és csak az `Out::finish` publikálja (`AtomicOutput::commit`, a meglévő felülírási viselkedés megtartásával: `Overwrite::Replace`).
Megszakítás, hiba vagy kill után a kimenet nem jelenik meg. Mérés a javítás után: 20 / 20 megszakítva, **0 fájl a végleges néven, 0 maradt
`.cfa4-export-*` ideiglenes fájl** (a SIGKILL-lal árván maradó ideiglenes fájl külön tétel, lásd #1738).

**Még nem atomikus (külön lépés):** a csomagkimenetek (`create_path`: DCP, IMF, MXF, OP-Atom, többfájlos) és a kép-sorozat/GIF kimenet
(`write_output`) a helyükre írnak. Mellékhatás: az `AtomicOutput` az útvonalat cseréli, ezért egy szimbolikus linkre mutató kimeneti
útvonalnál a link helyére új fájl kerül (nem a link célja íródik felül).

Latencia a javítás után (3 futás, párhuzamos tesztekkel együtt): medián 0,2–9 ms, p95 12–32 ms, max 15–33 ms (a szórás a felhős konténer
zaja; Küszöb@F0 továbbra is PENDING a referenciagépig).
