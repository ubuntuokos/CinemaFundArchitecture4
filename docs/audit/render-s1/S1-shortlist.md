# S1: szűkítés, javaslat a kapu-jóváhagyónak (CFA4-RENDER-INTEGRATION-PLAN-001)

> **Állapot: JAVASLAT, nem döntés.** A terv szerint (S1 kilépés) a döntés a kapu-jóváhagyó aláírása. Ez a dokumentum az F0a, F0V és F1
> bizonyítékából állítja össze a besorolást; csak olvasott, nem mért. Referenciagépes mérés nem szerepel benne (PENDING).

## 1. Szabályok és alkalmazásuk

A terv S1 szabályai: (1) az MQ előszűrés FAIL kizár; (2) az F1 BLOCKED kizár; (3) bizonyított GAP vagy mérhető cél nélkül kiesik;
(4) új, nem nyilvántartott Python verzióhoz indoklás és jóváhagyás kell; (5) F2-be legfeljebb 6 jelölt.

Az MQ előszűrés csak olvasás alapján történt, tehát egyetlen jelöltnél sem FAIL-ból, hanem nem mért állapotból indul: az F2 méri.

## 2. Javasolt F2-jelöltek (3 a megengedett 6-ból)

| # | Jelölt | F1 | Bizonyított GAP vagy mérhető cél (`F0-capability-gap-matrix.csv`) | Feltétel az F2-ben |
|---|---|---|---|---|
| 07 | OpenImageIO | PASS (Apache-2.0, 3.3.0.0) | G-10 (EXR tömörítések, multi-part), G-09 az OCIO-n át | az FFmpeg-plugin kikapcsolva fordul (A7); beágyazás külön ADR-t kér (ADR 0003 A3) |
| 04 | VapourSynth | PASS külön workerként (LGPL-2.1) | G-14 (deinterlace, cél), G-15 (IVTC, GAP), G-16 (zajcsökkentés, cél), G-17 (debanding, GAP) | csak külön folyamatban; a QTGMC-motor licence PENDING; a neo_f3kdb GPL-3.0, tehát csak workerben |
| 11 | EC-jelölt (#11) | PASS, feltételes POC (MIT OR Apache-2.0, v0.24.1) | G-33 (kompozíció-donor), MQ-1/3/4/8 igazolása | 8/16 bites kvantálás rétegenként: MQ-1-hez 32 bit kell; DCI-P3, X'Y'Z' és OCIO nincs leírva |

## 3. Javasolt kiesők az F2-ből

| # | Jelölt | Ok | Mi marad belőle |
|---|---|---|---|
| 09 | ReelForge | F1 REFERENCE_ONLY: egyetlen karbantartó (életképességi szabály ≥ 2) | a DAG/dirty-region/resume ötlet mintaként (G-01, G-02, G-04); kód és függőség nincs |
| 08 | avio / `ff-*` | F1 REFERENCE_ONLY: egyetlen karbantartó, FFmpeg 7.x/8.x-re épül (a CFA4 9.0.2), saját libav-kötés sértené A7-et | semmi; a `crates/ffmpeg` már megvan |
| 10 | rust-motion | F1 REFERENCE_ONLY: egyetlen karbantartó, upstream-eredetű közös kód | minta a G-19 alfa/élsimítási céljához |
| 05 | Blender | külső munkafolyamat (terv szerint), nem képlánc | G-26: külső folyamat, EXR-csere; licenc és verzió-pin PENDING |
| 06 | Natron | tulajdonosi döntés (2026-11-21): csak képességi minta, nem kimeneti orákulum sem | ADR 0003 §14 |

A többi jelölt (FFmpeg alap, MLT, MovieLite, Gausian, Dioxuscut, GStreamer, a kizártak) a terv szerinti besorolásában marad.

## 4. Amit a jóváhagyónak el kell döntenie

1. **A három F1 REFERENCE_ONLY jelölt (ReelForge, avio, rust-motion) kiesik az F2-ből.** A terv v1.1 ezeket feltételes POC-jelöltnek jelölte, és a D1 egy "ReelForge döntés" (F3 előfeltétele). Ha kiesnek, az F3 render graph saját (CFA4) megvalósítás, ReelForge mintával; a D1 ezt a döntést nem tartalmazza. Ez a terv szövegét érinti, ezért a koordinációs szabályzat §2.10 szerint tulajdonosi jóváhagyás kell, mielőtt a terv módosul.
2. **Python verzió (4. szabály).** A VapourSynth szkriptlánca (havsfunc → vs-jetpack, MIT) Python ≥ 3.13-at kér. Ha ez nem a nyilvántartott verzió, indoklás és jóváhagyás kell; az A11 szerint nem kötelezően egy verzió, de a számuk minimális.
3. **EC-jelölt fenntartása.** Az F2 elején el kell dönteni (MQ-1), hogy a 32 bites réteg-kvantálás nélküli út elérhető-e; ha nem, az EC-jelölt kiesik, és csak minta marad.

## 5. Nem része ennek a javaslatnak

Referenciagépes mérés (Python, driver, GPU, keresztgépes zaj), a QTGMC-motor licence, a Blender licence és verzió-pin, a mozi-kompatibilitási állítás (nincs moziszerver, csak szoftveres referencia).
