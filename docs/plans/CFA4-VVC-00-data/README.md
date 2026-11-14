# CFA4-VVC-00 – nyers adatok (2026-11-14-i pótlás)

A [jelentés](../CFA4-VVC-00-REPORT-2026-11-14.md) 4.6–4.7 fejezetének nyers adatai és szkriptjei.
Médiafájl nincs a repóban: a JVET-folyamok az ITU `bitstream_exchange/VVC/FDIS_r1` mappájából,
a tesztvideók a `media.xiph.org/video/derf/` oldalról tölthetők le.

| Fájl | Tartalom |
|---|---|
| `conformance-results.csv` | JVET-folyamonként: referencia-MD5 (a `*.yuv.md5` első értéke; a nagybetűs értékeket a kiértékelés kisbetűsíti), VTM-24.0, VVdeC, FFmpeg kimenetének MD5-je (`FAIL` = nincs kimenet). Az FFmpeg itt még kapcsolók nélkül futott; a jelentésben szereplő 6 hamis eltérést `-fps_mode passthrough -noautoscale`-lal újrafuttatva javítottuk. |
| `conformance-vtm12.csv` | A VTM-12.0 kimenete azokon a folyamokon, ahol az első kiértékelés eltérést mutatott. |
| `encode-results.csv` | Kódolásonként: idő, fps, kbit/s, PSNR-Y, a VVdeC, VTM, FFmpeg kimenetének és az uvg266 rekonstrukciójának MD5-je. |
| `build.sh`, `conf.sh`, `enc.sh`, `run_enc.sh`, `bd.py` | Fordítás, konformanciafuttatás, kódolás és mérés, BD-rate (köbös illesztés). |

A szkriptek a felhős munkakönyvtár (`/home/claude/vvc`) útvonalait használják.
