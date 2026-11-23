# F2: előregisztráció (VÁZLAT, jóváhagyásra)

> **Állapot: vázlat.** A terv szerint az előregisztráció az F2 indulása előtt készül, és **utólag nem módosítható**; ezért ez a fájl csak akkor válik
> érvényessé, ha a kapu-jóváhagyó (István) elfogadja, és a commit-ja a mérések kezdete előtt rögzül. A küszöbök két fajtája: **F0V-ből átvett** (mért zajból, `F0V-thresholds.json`)
> és **javasolt** (nincs rá mért zaj; az F2 első napjaiban a baseline futásaiból kellene származnia, ezért ezek "javasolt, mérés után rögzítendő" jelzésűek, és a
> rögzítés a mérés előtt, a baseline-futásokra történik, nem a jelölt eredményére). Előfeltétel: C0 lezárva (a spec vázlata: `docs/audit/render-c0/`), S1 aláírva.
> Futtatás nem történt; a hipotézisek nem eredmények.

## Közös szabályok

- Jelöltek (S1, István 2026-11-22): **OpenImageIO**, **VapourSynth** (csak külön workerként), **EC-jelölt (#11)** (feltételes). A többi csak minta.
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
| Kezdeti feltételek | Python ≥ 3.13 igény (vs-jetpack lánc) **jóváhagyandó (A11)**; a QTGMC-motor licence PENDING; a neo_f3kdb GPL-3.0, tehát csak workerben; sandbox profil (`renderWorker`) még nincs alkalmazva |
| Kilépés | MQ PASS, helyesség nem rosszabb a baseline-nál, egy előregisztrált előny |

## 3. EC-jelölt (#11), feltételes

| | |
|---|---|
| Hipotézis H-EC-1 (MQ-kapu) | a jelölt réteges kompozíciója 32 bites belső ábrázolással is futtatható **rétegenkénti 8/16 bites kvantálás nélkül** (az F1 szerint most 8/16 bitre kvantál minden réteg után). |
| MQ-1/3/4/8 igazolás | MQ-1: a köztes pontosság ≥ 16 bit lebegő; MQ-3: DCI-P3 és X'Y'Z' konverzió hibája az F0V küszöbön (ΔE2000: 0 a CPU-úton, vagy a mért zaj); MQ-4: straight/premultiplied megkülönböztetve, peremhiba nincs (alfa eltérés 0); MQ-8: lineáris fényű blend választható |
| Közös próba | egy referenciaklip + 3 rétegű, időben változó effektgráf (terv F2/3), CPU-n és a meglévő GPU-úton |
| Döntési szabály | ha az MQ-1 az első 3 munkanapban nem igazolható a jelölt módosítása nélkül, a jelölt kilép, és csak minta marad |
| Átfedés | a donor-katalógus (VFX terv, G-33) külön szál; ez a mérés csak az MQ-kaput és a kompozíciós minőséget fedi, a katalógus-beemelést nem |

## 4. Közös próba és költség (terv F2/3, F2/5)

- Referenciaklip: `sdr_1080p25` és `dci_4k24_12bit_p3`, 3 rétegű, időben változó effektgráf; mért: kép- és időhiba az F0V harnessen, valamint renderidő (AC-16, **küszöb PENDING a referenciagépig**).
- CPU↔GPU másolási és Frame ABI konverziós költség: byte/s és ms/kocka, a C0 konverziós mátrix soraira.

## 5. Amit ez a vázlat nem állít

Nincs mért eredmény. A "javasolt" küszöbök a jóváhagyóé; indoklásuk a táblázatban áll, de nem F0V-mérés. A referenciagépes futás (GPU és gépközi zaj, AC-07/09/16) PENDING.

## 6. Nyitott a jóváhagyóhoz

1. A "javasolt" küszöbök (OIIO ≥ 1,2×; VS ≥ 1,0 dB / SSIM +0,005) elfogadása vagy módosítása az F2 indulása előtt.
2. Python ≥ 3.13 a VapourSynth workerhez (A11): jóváhagyás, indoklás.
3. EC-jelölt fenntartása az F2-ben (H-EC-1 az első 3 napban).
4. Új fixture (PIZ/DWA/B44 EXR) létrehozása a `render_fixtures.py`-ban.
