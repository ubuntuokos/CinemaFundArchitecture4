# CFA4-DNX-SPEC-001 – A `cfa4-dnx` szabványhűsége (SMPTE ST 2019-1:2016 + Amd 1:2023)

**Verzió:** FINAL 1.0 (jóváhagyva 2026-11-24: „jóváhagyom véglegesítsd és készítsd el a fejlesztést is”)
**Projekt:** CinemaFundArchitecture4 (CFA4)
**Előzmény:** [CFA4-FFMPEG-9.1-001](CFA4-FFMPEG-9.1-001.md) F1: az FFmpeg 9.1 a CID 1271 súlytábláját a
szabványhoz igazította (a598cc99); a tulajdonos ekkor döntött (2026-11-24): **ahol van szabvány, azt
követjük; ha valamihez más kell, a fejlesztő megírja.**
**Forrás:** a tulajdonos által átadott SMPTE ST 2019-1 (2008, 2014, 2016, Amd 1:2023), ST 2019-3:2008
és ST 2019-4 (2009, 2014, 2016, Amd 1:2024) kiadások. A szabványszövegek szerzői jogvédettek: nem
kerülnek a tárolóba, a kód és a dokumentáció fejezetszámmal hivatkozik rájuk.

---

## I. Cél

A DNx-dekóder és -kódoló alapból a szabványt követi. Az FFmpeg-kompatibilis viselkedés (a régi
FFmpeg-gel írt fájlokhoz) kapcsoló mögé kerül.

## II. Eltérések a szabványtól (2026-11-24)

| # | Téma | Szabvány | Addig (FFmpeg szerint) |
|---|------|----------|-------------------------|
| E1 | Inverz kvantálás (§8.2.7, (8.1)) | `X = sgn(X̂)·⌊(|X̂|·W·qsf + ⌊W·qsf/2⌋ [+ p/2]) / p⌋`, ≈ `(A + ½)` | `(A + ¾)`; a szabványos út csak a `spec_dequant` kapcsolóval |
| E2 | 4:4:4 jelzés (§6.3, §7.2 CLF, §7.3.1.1 ACF, §8.1.1) | CLF = 0: Y′CbCr (Ch1 = Y); CLF = 1: RGB, Ch1 = R, Ch2 = G, Ch3 = B; ACF = 1 csak CLF = 1 és CLV ≠ 11b mellett, a makroblokk dekódoláskor BT.709 szerint RGB-re alakul | RGB: CLF = 0, G, B, R sorrend; Y′CbCr: CLF = 1 + ACF = 1 minden makroblokkon, Y′CbCr-ként visszaadva |
| E3 | CID 1271 súlytábla (C.2) | D.1 | javítva a CFA4-FFMPEG-9.1-001 F1-ben |
| – | RLE alfa (§7.3.1.2 és 50. ábra ellentmond) | a normatív szöveg | így van, marad |

Az Amd 1:2023 (alfa a HD CID-eknél, HVN 0x04, CID 1256 LLA, CID 1260 frame) már a szabvány szerint
megvan a crate-ben.

## III. Követelmények

| # | Követelmény | Átvételi teszt |
|---|-------------|----------------|
| R1 | A dekóder alapból a (8.1) egyenlet szerint kvantál vissza. | Egyetlen AC-együtthatós, kézzel összerakott adatfolyam minden CID-családra; a visszaállított érték a (8.1) képlettel számolt. |
| R2 | Alapból szabványos 4:4:4 jelzés a dekódolásban és a kódolásban (II. E2). A `Frame` RGB-síkjai továbbra is G, B, R sorrendben (`y`, `cb`, `cr`) vannak, a fogyasztók nem változnak. | Kézzel összerakott CLF = 1 (R, G, B), CLF = 1 + vegyes ACF, CLF = 0 (Y′CbCr 4:4:4) adatfolyamok; oda-vissza kódolás mindkét színtérben; a kódoló CLF/ACF bitjei. |
| R3 | `ffmpeg_compat` kapcsoló a `DecodeOptions`-ban és az `EncoderConfig`-ban: a régi (FFmpeg szerinti) viselkedés E1-re és E2-re. Az FFmpeg-orákulumtesztek ezzel futnak, és nem romlanak. | Az összes meglévő orákulumteszt `ffmpeg_compat`-tal zöld. |
| R4 | A szabvány szerint írt Y′CbCr 4:4:4 fájlt az FFmpeg 9.1 hiba nélkül dekódolja. A szabványos RGB 4:4:4-et (CLF = 1, ACF = 0) az FFmpeg 9.1 nem támogatja: „Unsupported: variable ACT flag” hibát ír, Y′CbCr-ként dekódol, rossz színekkel (2026-11-24-i megállapítás). Tulajdonosi döntés (2026-11-24): marad a szabvány, és az exportpanel RGB 4:4:4-nél figyelmeztet, az FFmpeg-kompatibilis beállításra mutatva. | Orákulumteszt: Y′CbCr `-xerror`-ral hibátlan; RGB: képkockaszám egyezik, és a teszt rögzíti az FFmpeg hibaüzenetét (ha az FFmpeg változik, a teszt jelez). |
| R5 | A motor (`crates/codecs`, `crates/export`) alapból szabványosan dekódol; FFmpeg által írt fájlra (MOV/MP4 `©too` = `Lavf…`, MXF Identification `ProductName`/`ToolkitVersion` FFmpeg/Lavf) automatikusan `ffmpeg_compat`. Kézi kapcsoló a médiatulajdonságoknál, menüből és beállítópanelről is (D2); FFmpeg-kompatibilis kódolás exportbeállításként (D3). | Egységtesztek a felismerésre; a beállítás parancsként (`Everything is a command`) és automation id-vel. |
| R6 | A crate README „Where we follow ffmpeg…” fejezete átírva: mi a szabvány, mit tud a kompatibilis mód. | Doksi-átnézés. |

## IV. Döntések (jóváhagyva 2026-11-24)

| # | Kérdés | Döntés |
|---|--------|--------|
| D1 | A motor alapértéke | szabvány szerint; FFmpeg-fájlra automatikus kompatibilis mód |
| D2 | Kézi kapcsoló, ha nem dönthető el | igen: médiatulajdonságoknál, menüből és beállítópanelről |
| D3 | Saját DNx-export | szabvány szerint; FFmpeg-kompatibilis kódolás exportbeállításként |
| D4 | RGB 4:4:4 export, amit az FFmpeg nem olvas helyesen | szabvány + figyelmeztetés az exportpanelen (2026-11-24). Megjegyzés: a CFA4 DNx-exportja ma csak 4:2:2 profilokat kínál; a figyelmeztetés a 4:4:4 export bekerülésekor kell. |

## V. PR-bontás

| PR | Tartalom | CodeRabbit |
|----|----------|------------|
| P1+P2+P4 | `cfa4-dnx`: E1, E2, `ffmpeg_compat` (R1–R4), README (R6), ez a terv; és R5-ből az FFmpeg által írt fájlok automatikus felismerése a `cfa4-codecs`-ben (enélkül az FFmpeg-gel készült tesztfájlok eltérnének) | igen (kodek) |
| P3 | motor: kézi kapcsoló (Qt6 GUI: menü + panel), exportbeállítás, RGB 4:4:4 figyelmeztetés (R5, D2–D4) | igen (kodek/konténer) |
