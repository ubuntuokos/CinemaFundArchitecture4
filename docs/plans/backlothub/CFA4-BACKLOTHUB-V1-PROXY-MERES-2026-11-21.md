# V1 CPU-only proxy-mérés (előzetes, 2026-11-21)

**Állapot:** előzetes, ffmpeg-szintű mérés. A CFA4-motoros (A/V-szinkron, eldobott kocka) rész még hátravan.

## Környezet (a mérés a felhős gépen futott, István döntése szerint)
- 4 vCPU (Intel Xeon @ 2,1 GHz, szálanként 1 szál, HT nélkül), 15 GiB RAM, GPU nincs; `taskset -c 0-3`.
- Közelíti a CFA4-MIN-t (4 mag/16 GB, GPU nélkül), de **nem azonos**: HT nélküli, más CPU-generáció, virtualizált. A számok irányadók, a hivatalos CFA4-MIN (8 szál, István gépe) mérés ezt felülírhatja.
- ffmpeg 6.1.1-3ubuntu5 (rendszer), libx264/libvpx-vp9/libsvtav1 kódolók; dekóderek: h264, vp9, libdav1d.

## Fixture
20 s, 1920×1080, 24 fps, Mandelbrot + időbeli zaj (a tömörítésnek **nehéz**, vagyis pesszimista), 48 kHz hang. Nem kerül commitba (`/tmp`, nem a repóban).

## Eredmények (480 kocka, 3 futás legjobbja)
| Jelölt | Fájlméret | Dekódolás 1 szál | Dekódolás 4 szál | Ugrás (átlag/max, 4 szál) |
|---|---|---|---|---|
| H.264 High, 8 Mbps, GOP 24 | 21,3 MB | 135 fps | 374 fps | 179 / 327 ms |
| H.264 High, 8 Mbps, GOP 250 | 21,3 MB | 134 fps | 391 fps | 464 / 734 ms |
| H.264 all-intra, ~40 Mbps | 102 MB | 43 fps | 149 fps | 225 / 231 ms |
| VP9, 6 Mbps, GOP 24 | 15,5 MB | 152 fps | 346 fps | 150 / 206 ms |
| AV1 (SVT), 5 Mbps, GOP 24 | 10,2 MB | 167 fps | 258 fps | 123 / 157 ms |

Az ugrás értéke a teljes ffmpeg-folyamat ideje (≈41 ms indulási költséggel), pontos (input-oldali) keresés, 1 kocka dekódolva.

## Értékelés a követelményekhez
- **REV-01** (valós idejű 1080p, 0 eldobott kocka): a dekódolási tartalék 30 fps-hez képest egy szálon 4,5×, négy szálon 8–13×. Két adatfolyam egyidejű dekódolása (REV-07) is tartalékkal fér. Az „eldobott kocka 10 percen át" a motor/Qt lejátszóút mérése, **itt nem mértem**.
- **REV-04** (ugrás ≤ 250 ms): a rövid GOP-ú (≈1 s) H.264, VP9 és AV1 teljesíti (max 327 ms egy kiugró H.264-esetnél, a folyamat-indítással együtt); GOP 250 **nem** teljesíti (max 734 ms). All-intra teljesít, de 5× méretű és egy szálon nem éri el az 50 fps-t.
- **REV-03** (képkockapontosság): property-teszt, nem teljesítménymérés; a G5/V2-ben, nem itt.
- **REV-05** (A/V-szinkron ±1 kocka): csak a CFA4 lejátszóútján mérhető; **hátravan**.

## Javaslat a proxyszabványra (a verziómodellhez, 8. lépcső)
**H.264 High profil, 1080p, yuv420p, GOP ≈ 1 s (24–30 kocka, zárt GOP), ~8 Mbps, AAC 128 kbps, MP4.** Indok: legszélesebb támogatás, legjobb dekódolási tartalék és kiszámítható ugrás, a `cfa4-qc` is MP4/MOV-ot ismer. VP9/AV1 később kisebb tárhelyért opció (AV1 dekódolása 4 szálon a leglassabb, de így is ~8× valós idő).

## Hátravan a V1 lezárásához
1. Ugyanez a CFA4-motor lejátszóútján (`cargo xtask bench --hw off`): REV-01 10 perc eldobott kocka, REV-05 A/V-szinkron.
2. A hivatalos CFA4-MIN gép (István gépén, 4C/8T pinelés) mérése, ha kell (a felhős érték csak előzetes).
3. Valós (kevésbé zajos) tartalom ellenőrzése.


---

# 2. rész: motoros mérés (CFA4 `bench_playback`, CPU-only, 4 mag, 2026-11-21)

`cargo build --release -p cfa4-ui-egui --example bench_playback` a `main`-en (01ff4e41e), futtatás `taskset -c 0-3 … --cpu --res full --seconds 10 --refresh 30`, 4 képkocka-munkás, ugyanaz a felhős gép (4 vCPU, 15 GiB, GPU nélkül). A fixture a bench saját H.264 (libx264) tartalma: `testsrc2` 1080p + zaj, 23,976 fps (a repó `fixtures.rs` szerint). Ez **nem pontosan** a javasolt proxyprofil (a bench-fixture GOP/bitráta-beállítása a repó alapértelmezése).

| Forgatókönyv | Mért | Eredmény |
|---|---|---|
| `h264-1080` lejátszás (1 adatfolyam, 10 s) | 239 megjelenített kocka, **0 eldobott**, 100% időben | **REV-01 egy adatfolyamra teljesül** |
| – CPU-igény | 92 ms CPU/kocka ≈ **2,2 mag** a 4-ből | tartalék kb. 45% |
| `stack3` (3 réteg kompozit, 10 s) | 10 megjelenített, 229 eldobott, 4% időben | **nem tartja**; ez 3 adatfolyam + kompozitálás (3,9 mag kellene), a REV-07 (2 verzió) ennél könnyebb, de **külön nem mértem** |
| `seek-storm` (gyors egymás utáni ugrások) | munkás-idő 209 ms (p50), 237 ms (p95) ugrásonként; 29 dekóder-újraindítás | **REV-04 (≤250 ms) a határon teljesül**, de terhelés alatt (sorban álló ugrások) a késleltetés nő |

## Fő tanulság
A **CFA4 saját H.264-dekódere kb. 10× lassabb**, mint az ffmpeg (≈11 kocka/s/mag a motorban, 135 kocka/s/szál ffmpeg-gel). A 1. rész ffmpeg-számai ezért **felső korlátként** olvasandók: egy 1080p adatfolyam CPU-n belefér, de a tartalék csak ~45%, két verzió egymás melletti lejátszására (REV-07) és összetett idővonalra **nincs tartalék**.

## Nem mértem
- **REV-05 (A/V-szinkron ±1 kocka):** a `bench_playback` képet mér, hangot nem; a `bench_audio` külön példa, de a szinkron (hang+kép együtt) mérése a Qt-előnézet útján kell (ADR F8 5. lépés, ma nincs mérve).
- 10 perces eldobott-kocka-mentesség: csak 10 s futás.
- REV-07 két adatfolyammal külön forgatókönyv nélkül.

## Következtetés a V1-re
1. **A CPU-only cél (REV-01) egy 1080p proxyra tartható**, csekély tartalékkal; ez a javasolt proxyszabványt nem változtatja (H.264 High 1080p, ~1 s GOP).
2. **REV-07 (két verzió egymás mellett) CPU-only 4 magon jelenleg kockázatos**; a megoldás István döntése (NFR 3.5/D-pont): vagy a motor H.264-dekódere gyorsul (CFA4 motorprojekt, nem BacklotHub), vagy az összehasonlítás kisebb proxyval (pl. 720p) fut CPU-n, vagy a REV-07 CPU-n enyhül. Ezt a 8. lépcső G4 kapuja előtt el kell dönteni.
3. A hivatalos CFA4-MIN mérés (István gépén) és a REV-05 mérés még nyitott.

## Döntés (István, 2026-11-21)
REV-07 CPU-only gépen: **720p összehasonlító proxy**; REV-01 marad 1080p. A 8. lépcső G4 kapuja ezzel nem blokkolt. Hátravan: REV-05 A/V-szinkron mérés és CFA4-MIN mérés.
