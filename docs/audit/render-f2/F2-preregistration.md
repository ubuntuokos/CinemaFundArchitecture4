# F2: előregisztráció (JÓVÁHAGYVA, 2026-11-23 20:14 Europe/Budapest)

> **Állapot: jóváhagyva (István, 2026-11-23 20:14 Europe/Budapest; lásd a 7. szakaszt).** A jóváhagyás előtti vázlatszöveg: a terv szerint az előregisztráció az F2 indulása előtt készül, és **utólag nem módosítható**; ezért ez a fájl csak akkor válik
> érvényessé, ha a kapu-jóváhagyó (István) elfogadja, és a commit-ja a mérések kezdete előtt rögzül. A küszöbök két fajtája: **F0V-ből átvett** (mért zajból, `F0V-thresholds.json`)
> és **javasolt** (nincs rá mért zaj; az F2 első napjaiban a baseline futásaiból kellene származnia, ezért ezek "javasolt, mérés után rögzítendő" jelzésűek, és a
> rögzítés a mérés előtt, a baseline-futásokra történik, nem a jelölt eredményére). Előfeltétel: C0 lezárva (a spec vázlata: `docs/audit/render-c0/`), S1 aláírva.
> Futtatás nem történt; a hipotézisek nem eredmények.

## Közös szabályok

- Jelöltek (S1, István 2026-11-22): **OpenImageIO**, **VapourSynth** (csak külön workerként), **EC-jelölt (#11)** (feltételes), **GEGL + babl** (2026-11-23). A többi csak minta.
- Futásszám: döntéshez legalább **10 futás**, medián, p95 és bootstrap 95% konfidenciaintervallum (terv, mérési terv). Trendhez 3 meleg és 3 hideg.
- Baseline: a meglévő CFA4 út (CPU referencia, a meglévő GPU út új backend nélkül). Egy jelölt akkor "előnyös", ha az előregisztrált metrikán a CI alsó határa is meghaladja a küszöböt.
- MQ-kapu az első 3 munkanapon: FAIL → a jelölt kilép. Minden mérés a `cfa4_golden::validation` harnessen, mutációs önteszttel; mért hiányzó ellenőrzés sosem PASS.
- A POC-kód izolált ágon fut, nem kerül `main`-re. Mérés csak a referenciagépen érvényes a GPU- és gépközi zajra (PENDING addig).
- Fixture-ök: `docs/audit/render-f0v/F0V-fixtures-manifest.json` (`render_fixtures.py generate`).

## 1. OpenImageIO

| | |
|---|---|
| Hipotézis H-OIIO-1 | az OIIO EXR olvasása/írása a CFA4 saját EXR-útjánál (scan-line; RLE/ZIP/PXR24) szélesebb formátumot fed le (PIZ/DWA/B44, multi-part), azonos pixelhűséggel. |
| Fixture | `exr_sequence` (16 kocka, 32 bit float), plusz egy PIZ/DWA/B44 készlet (a generátor bővítendő; **új fixture**) |
| Metrika és küszöb | 32 bit float EXR round-trip pixelhiba: **bitpontos (0)** a veszteségmentes tömörítéseknél (F0V-ből: RGB eltérés 0); veszteséges (DWA, B44) a gyártói tolerancián belül (**javasolt: a tolerancia a baseline-ból, mérés előtt rögzítendő**) |
| Sebesség | olvasási kocka/s a meglévő úthoz képest; **javasolt küszöb: ≥ 1,2× medián, CI alsó határa ≥ 1,0×** (indoklás: kisebb nyereség nem fedezi a függőséget) |
| Metaadat | a csatornanevek, az adatablak és az OCIO-azonosító megőrzése: 100% (mérés: kulcsok egyezése) |
| Feltételek | az FFmpeg-plugin kikapcsolva (A7); beágyazás külön ADR (ADR 0003 A3) |
| Kilépés | MQ-1 (≥ 16 bit lebegő) PASS és egyetlen előregisztrált előny |

## 2. VapourSynth (külön worker)

| | |
|---|---|
| Hipotézisek | H-VS-1 deinterlace (`retouch_interlaced`), H-VS-2 IVTC (`retouch_telecined`), H-VS-3 zajcsökkentés (`retouch_noisy`), H-VS-4 debanding (`retouch_banded`): a szűrő minősége jobb az FFmpeg megfelelőjénél |
| Metrikák | deinterlace: PSNR/SSIM az ismert progresszív forráshoz képest (a fixture-ből generálható referencia); IVTC: helyreállított 24p kocka-sorrend hibaszáma (0 = tökéletes) és PSNR; zajcsökkentés: PSNR/SSIM-nyereség a zajmentes mintához; debanding: sávosodási metrika (**javasolt: a baseline-on rögzítendő definíció**) |
| Küszöb | **javasolt:** PSNR nyereség ≥ 1,0 dB (SSIM ≥ +0,005) az FFmpeg legjobb szűrőjéhez képest, CI alsó határán; IVTC: kocka-sorrend hibája 0 |
| Szűrő round-trip | a worker átmeneti formátuma (Frame ABI, C0) veszteségmentes: RGB eltérés 0 a küszöbön (F0V) |
| Kezdeti feltételek | Python ≥ 3.13 igény (vs-jetpack lánc): **elfogadva (István, 2026-11-23): ha a VapourSynth-nek kell, igen; A11 szerint a lehető legkevesebb Python-verzióval, kizárólag a külön worker folyamatban**; a QTGMC-motor licence PENDING; a neo_f3kdb GPL-3.0, tehát csak workerben; sandbox profil (`renderWorker`) még nincs alkalmazva |
| Kilépés | MQ PASS, helyesség nem rosszabb a baseline-nál, egy előregisztrált előny |

## 3. EC-jelölt (#11), feltételes

| | |
|---|---|
| Hipotézis H-EC-1 (MQ-kapu) | a jelölt réteges kompozíciója 32 bites belső ábrázolással is futtatható **rétegenkénti 8/16 bites kvantálás nélkül** (az F1 szerint most 8/16 bitre kvantál minden réteg után). |
| MQ-1/3/4/8 igazolás | MQ-1: a köztes pontosság ≥ 16 bit lebegő; MQ-3: DCI-P3 és X'Y'Z' konverzió hibája az F0V küszöbön (ΔE2000: 0 a CPU-úton, vagy a mért zaj); MQ-4: straight/premultiplied megkülönböztetve, peremhiba nincs (alfa eltérés 0); MQ-8: lineáris fényű blend választható |
| Közös próba | egy referenciaklip + 3 rétegű, időben változó effektgráf (terv F2/3), CPU-n és a meglévő GPU-úton |
| Döntési szabály | ha az MQ-1 az első 3 munkanapban nem igazolható a jelölt módosítása nélkül, a jelölt kilép, és csak minta marad |
| Átfedés | a donor-katalógus (VFX terv, G-33) külön szál; ez a mérés csak az MQ-kaput és a kompozíciós minőséget fedi, a katalógus-beemelést nem |

## 3b. GEGL + babl (negyedik jelölt, István 2026-11-23; terv rev 119)

| | |
|---|---|
| Hipotézis H-GEGL-1 (MQ-kapu) | a GEGL gráfja f32, lineáris fényű munkaformátumon (babl) fut, **köztes 8/16 bites kvantálás nélkül**, és a babl színtér-konverziója (sRGB, DCI-P3, X'Y'Z' felé) az F0V küszöbön belül marad. |
| MQ-1/3/4/8 igazolás | MQ-1: ≥ 16 bit belső pontosság (f32); MQ-3: DCI-P3 és X'Y'Z' konverzió hibája ΔE2000-ben az F0V küszöbön; MQ-4: alfa straight/premultiplied megkülönböztetve, peremhiba nincs; MQ-8: lineáris fényű blend a referenciához mérve. |
| Feladat | kompozíciós és lineáris f32 összehasonlítás az EC-jelölttel, ugyanazon a C0 szerződésen és a 4. pont közös próbáján, a Küszöb@F0 mérőkkel (PSNR, ΔE2000, alfa). |
| Küszöbök | **PENDING**: nincs kitalált szám; az F0V-mérésből (≥ 10 futás, referenciagép) és a jóváhagyó döntéséből jönnek, mint a többi jelöltnél. |
| Döntési szabály | MQ-1/3/4/8 FAIL az első 3 munkanapban: csak mintának marad. Az F1 kiegészítő audit feltételei ([`F1-gegl-babl-supplement.md`](../render-f1/F1-gegl-babl-supplement.md), #1688) az F2 kezdete előtt zárandók; ha a licenc-fájlaudit elbukik, a jelölt BLOCKED. |
| Futtatás | csak külön worker folyamatban (`renderWorker` profil, PENDING jóváhagyás); a GLib/GObject-lábnyom miatt a fő folyamatba nem linkelünk. |

## 4. Közös próba és költség (terv F2/3, F2/5)

- Referenciaklip: `sdr_1080p25` és `dci_4k24_12bit_p3`, 3 rétegű, időben változó effektgráf; mért: kép- és időhiba az F0V harnessen, valamint renderidő (AC-16, **küszöb PENDING a referenciagépig**).
- CPU↔GPU másolási és Frame ABI konverziós költség: byte/s és ms/kocka, a C0 konverziós mátrix soraira.

## 5. Amit ez a vázlat nem állít

Nincs mért eredmény. A "javasolt" küszöbök a jóváhagyóé; indoklásuk a táblázatban áll, de nem F0V-mérés. A referenciagépes futás (GPU és gépközi zaj, AC-07/09/16) PENDING.

## 6. Nyitott a jóváhagyóhoz

1. ~~A "javasolt" küszöbök (OIIO ≥ 1,2×; VS ≥ 1,0 dB / SSIM +0,005) elfogadása~~ **Elfogadva (István, 2026-11-23 20:14), lásd 7.**
2. ~~Python ≥ 3.13 a VapourSynth workerhez (A11)~~ **Eldöntve (István, 2026-11-23):** elfogadva, ha a VapourSynth-nek szüksége van rá; csak a külön workerben, a többi komponens nem függ tőle.
3. ~~EC-jelölt fenntartása az F2-ben~~ **Eldöntve (István, 2026-11-23):** marad az F2 harmadik jelöltje (az upstream családdal közös geometria-, szín- és időtípusok miatt); az MQ-kapu érvényes, bukásnál csak mintának marad (H-EC-1 az első 3 napban).
3b. ~~GEGL (+ babl) jelölt~~ **Felvéve (István, 2026-11-23, terv rev 119):** lásd 3b. szakasz; a küszöbei PENDING.
4. ~~Új fixture (PIZ/DWA/B44 EXR)~~ **Elkészült (előkészítés, nem mérés):** `python/cfa4_devtools/render_f2_exr_fixtures.py` (nem a `render_fixtures.py`, mert az `ffmpeg` EXR-kódolója csak none/RLE/ZIP-et tud). 10 tömörítés, egy 640×360 f32 kép, fix magból; kimenet a `target/fixtures/render-f2/`-be. Futtatás: OpenImageIO és numpy modul kell (F2 POC függőség, eldobható venvben); nélkülük SKIPPED. Önellenőrzés (nem F2-eredmény): a veszteségmentes tömörítések 0 eltérést adnak; a veszteséges tolerancia a baseline-ból rögzítendő, mérés előtt.

## 7. Jóváhagyás (István, 2026-11-23 20:14 Europe/Budapest)

István a koordinátor javaslatait együtt hagyta jóvá („jóváhagyva, 2026.11.23. 20.14-kor”). Ettől a committól az előregisztráció érvényes; a mérések kezdete előtt rögzült, és utólag nem módosítható (módosítás csak új, jelölt verziójú melléklettel, a mérés előtt).

1. **OpenImageIO és VapourSynth:** a fenti javasolt küszöbök érvényesek (OIIO olvasás ≥ 1,2×, a bootstrap CI alsó határa ≥ 1,0×; VapourSynth ≥ 1,0 dB PSNR és SSIM +0,005 az FFmpeg legjobb szűrőjéhez képest, a CI alsó határán; IVTC kocka-sorrend hibája 0).
2. **GEGL + babl:** a küszöbök PENDING maradnak, de ugyanabban a szerkezetben (sebesség, minőség) rögzítendők, és a rögzítés commitja a GEGL első mérése előtt készül el.
3. **Veszteséges EXR (DWA, B44, PXR24):** a tűrés a veszteségmentes baseline-ból jön, tömörítésenként külön határral, a mérés előtt rögzítve. A veszteségmentes tömörítéseknél az elvárás bitazonos (eltérés 0).
4. **Sorrend:** az F2 indulásához a C0 lezárása kell; előbb az exportút atomikus kimenete (a C0 rése, lásd `docs/audit/render-c0/C0-cancel-latency.md`).
