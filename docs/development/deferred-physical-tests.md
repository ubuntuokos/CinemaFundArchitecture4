# Elhalasztott fizikai tesztek

**Szabály** (tulajdonosi döntés, 2026-11-22; [`docs/coordination.md`](../coordination.md) §2.14): minden
fizikai teszt, amelyet a tulajdonos döntése alapján kihagyunk (referenciagépes mérés, valódi
hardveren, kamerával, kijelzőn vagy hálózaton végzett próba), ide kerül egy sorral. Így később
bármikor pótolható, és senki nem hiszi, hogy lefutott.

A gép adatai és a mérési szabályok: [`reference-machine.md`](reference-machine.md).

| Felvéve | Tétel | Mit kell mérni vagy kipróbálni | Parancs / módszer | Helyettesítő eredmény | Miért maradt el | Állapot |
|---|---|---|---|---|---|---|
| 2026-11-22 | DM4 – datamosh (#1516) | 1080p, 960×540 és 480×270 képidők: a Datamosh átmenet és a Datamosh Bloom egy-egy képkockája, a referenciagépen (2 × Xeon E5-2696 v4, processzoronként 44 szál, összesen 88 szál; CPU-n fut: saját H.264 kódoló és dekóder; `lscpu`, rögzített magok, terhelés feljegyezve) | `cargo test -p cfa4-render --release --lib mosh::tests::frame_times_1080p -- --ignored --nocapture` | Felhős gép (4 vCPU, Xeon 2,1 GHz): 1080p átmenet ~196 ms/képkocka, Bloom ~342 ms/ismétlés; 960×540: ~51 / ~96 ms (#1516 leírása) | A tulajdonos döntése (2026-11-22): a felhős mérés elég | elhalasztva |

Ha egy tétel lefutott, az állapot legyen `elvégezve, ÉÉÉÉ-HH-NN` és a mérés helye (PR vagy jegyzőkönyv); a sort ne töröld.
