# Kodek-backlog végrehajtási terv

> **Utolsó frissítés:** 2026-11-13 · **Változás:** minor (a „kész” feltétel és a 📄 kapu szövege az AGENTS.md §2-höz igazítva) · **Forrás:** [CODEC-BACKLOG-2026-10-27.md](CODEC-BACKLOG-2026-10-27.md)

A [CODEC-BACKLOG-2026-10-27.md](CODEC-BACKLOG-2026-10-27.md) 82 tétele heti ütemezésben. **Követelmény: minden héten
legalább 2 tétel elkészül** (a tulajdonos döntése, 2026-10-27). Így a backlog 41 hét alatt fogy el:
**2026-11-02-tól 2027-08-15-ig**. Ha egy héten több is elkészül, a következő tételek előrébb jönnek.

Az ütemterv a meglévő kodekmunkával (M9, B3) **párhuzamosan, alacsony prioritással** fut. Ha a
backlognak meg kell várnia az M9 és a B3 lezárását, minden dátum ugyanannyi héttel tolódik.

## Mikor „kész” egy tétel

1. Saját pure-Rust dekóder `unsafe` nélkül, nyilvános leírásból, vagy engedélyezett licencű megvalósítás nyilvántartott portjaként ([licensing.md](../licensing.md) §3, §5) ([AGENTS.md](../../AGENTS.md) §0, §2), új L0 crate-ben vagy a meglévőben. Az FFmpeg-tartalékon át működés nem elég (tulajdonosi döntés, 2026-11-08).
2. Be van kötve az importba (a megfelelő konténerben: AVI, MOV, MKV, MP4, Ogg, nyers fájl).
3. ffmpeg-orákulumteszt (bitpontos, vagy veszteséges hangnál SNR-határ), generált fixture-ökkel a `target/fixtures/` alatt.
4. Crate-README, valamint egy sor a [codec-parity.md](../codec-parity.md)-ben; a tétel kikerül a [CODEC-BACKLOG-2026-10-27.md](CODEC-BACKLOG-2026-10-27.md)-ből.
5. A minőségi kapuk zöldek ([CLAUDE.md](../../CLAUDE.md)); egy tétel = egy commit (`CB-<hét>.<1|2>: <kodek>`).

Az enkódolás nem része a tételnek, csak ha a sor külön jelzi.

## Döntési kapuk

| Jel | Jelentés | Határidő |
|---|---|---|
| ⚖️ | Licenc / szabadalom (MPEG-H, AC-4): tulajdonosi döntés kell | A 21. hétig (2027-03-22) |
| 📄 | Nincs teljes nyilvános specifikáció: tulajdonosi döntés kell (port engedélyezett licencű megvalósításból, vagy kimarad) | A K2-esekre a 9. hétig (2026-12-28), a K3-asokra a 22. hétig (2027-03-29) |

Ha egy tételre a döntés nemleges vagy késik, az a hét a következő szabad tétellel telik meg, így a
heti 2 tétel nem csúszik. A nemleges döntésű tételek kikerülnek a backlogból.

## Ütemterv

Sávok: K1 az 1–6. hét, K2 a 7–22. hét, K3 a 23–40. hét, K4 a 40–41. hét.

| Hét | Kezdete | Sáv | 1. tétel | 2. tétel |
|---|---|---|---|---|
| 1 | 2026-11-02 | K1 | FFV1 | Huffyuv |
| 2 | 2026-11-09 | K1 | Ut Video | MagicYUV |
| 3 | 2026-11-16 | K1 | Lagarith 📄 | HAP |
| 4 | 2026-11-23 | K1 | G.711 / G.722 | ADPCM (IMA, Microsoft) |
| 5 | 2026-11-30 | K1 | WavPack | Monkey's Audio (APE) |
| 6 | 2026-12-07 | K1 | Speex | AMR-NB / AMR-WB |
| 7 | 2026-12-14 | K2 | Matrox MPEG-2 I-frame (ellenőrzés + teszt) | VC-2 |
| 8 | 2026-12-21 | K2 | Dirac | Canopus HQ / HQX |
| 9 | 2026-12-28 | K2 | TTA | Shorten |
| 10 | 2027-01-04 | K2 | TAK 📄 | Musepack SV7 / SV8 |
| 11 | 2027-01-11 | K2 | OptimFROG 📄 | G.729 |
| 12 | 2027-01-18 | K2 | iLBC | EVS |
| 13 | 2027-01-25 | K2 | RealVideo RV10 / RV20 📄 | RealVideo RV30 / RV40 📄 |
| 14 | 2027-02-01 | K2 | RealAudio Cook 📄 | RealAudio SIPR, 14.4 / 28.8 📄 |
| 15 | 2027-02-08 | K2 | ATRAC3 📄 | ATRAC3plus 📄 |
| 16 | 2027-02-15 | K2 | ATRAC9 📄 | Sony SR Codec 📄 |
| 17 | 2027-02-22 | K2 | MPEG-5 EVC | LCEVC |
| 18 | 2027-03-01 | K2 | AVS1 / AVS+ | AVS2 |
| 19 | 2027-03-08 | K2 | AVS3 | AVS Audio |
| 20 | 2027-03-15 | K2 | AVS2 Audio | AVS3 Audio |
| 21 | 2027-03-22 | K2 | Audio Vivid | DRA |
| 22 | 2027-03-29 | K2 | MPEG-H 3D Audio ⚖️ | AC-4 ⚖️ |
| 23 | 2027-04-05 | K3 | Bink Video 📄 | Bink 2 📄 |
| 24 | 2027-04-12 | K3 | Bink Audio 📄 | Smacker + Smacker Audio 📄 |
| 25 | 2027-04-19 | K3 | id RoQ 📄 | Interplay MVE + ACM 📄 |
| 26 | 2027-04-26 | K3 | CRI ADX 📄 | CRI HCA 📄 |
| 27 | 2027-05-03 | K3 | Sofdec.Prime 📄 | Sofdec2 (konténer) 📄 |
| 28 | 2027-05-10 | K3 | Nintendo THP 📄 | Nintendo Mobiclip 📄 |
| 29 | 2027-05-17 | K3 | Nintendo HVQM / HVQM4 📄 | Nintendo DSP ADPCM + BRSTM 📄 |
| 30 | 2027-05-24 | K3 | BCSTM / BFSTM 📄 | PSP MPEG-4 AVC (PMF) 📄 |
| 31 | 2027-05-31 | K3 | Yamaha + OKI ADPCM 📄 | NTT TwinVQ 📄 |
| 32 | 2027-06-07 | K3 | AMV Video 📄 | Cinepak 📄 |
| 33 | 2027-06-14 | K3 | Intel Indeo 2 / 3 📄 | Intel Indeo 4 / 5 📄 |
| 34 | 2027-06-21 | K3 | Sorenson Video 1 📄 | Sorenson Video 3 📄 |
| 35 | 2027-06-28 | K3 | Microsoft RLE + Video 1 📄 | Autodesk FLIC / FLC |
| 36 | 2027-07-05 | K3 | HuffYUV MT | QDesign Music 2 📄 |
| 37 | 2027-07-12 | K3 | Voxware MetaSound 📄 | TrueSpeech 📄 |
| 38 | 2027-07-19 | K3 | Apple MACE 3 / 6 📄 | QCELP |
| 39 | 2027-07-26 | K3 | AVS2 3D / Multiview | AVS3 panoráma / immerszív profilok |
| 40 | 2027-08-02 | K3 / K4 | AVS3 RTC Speech | TPG |
| 41 | 2027-08-09 | K4 | SMT (Smart Media Transport) | AVS3 Immersive Media |

## Követés

Minden hét végén a hét két tételét pipálni kell ebben a fájlban (✅ a tétel neve elé), és ha
valamelyik nem készült el, a következő hétre kell vinni a pótlással együtt.

## Revíziótörténet

| Dátum | Változás | Összefoglaló |
|---|---|---|
| 2026-11-13 | minor | A „kész” 1. feltétele és a 📄 kapu indoklása az AGENTS.md §2-höz igazítva (port megengedett, ha a licenc engedélyezett és nyilvántartott). Az ütemterv és a határidők nem változtak |
| 2026-11-09 | minor | A Lagarith 📄 lett (nincs teljes nyilvános leírás, ffmpeg-kódoló sincs): a tulajdonos 2026-12-28-ig dönt róla, addig a 4. hét tételei (G.711 / G.722, ADPCM; #818) jöttek előre |
| 2026-11-09 | minor | A H.266 / VVC kivéve: saját, jóváhagyott terve van (CFA4-VVC-PLAN-001, #721). 82 tétel, 41 hét (2027-08-15-ig); a 22. héttől minden tétel egy hellyel előrébb jött. A 2. hét MagicYUV-ja tulajdonosi döntésre vár, ezért a HAP (CB-3.2) előrejött |
| 2026-11-08 | major | Áthelyezve a CFA4-AI_modules tárolóból a CinemaFundArchitecture4-be. Kivéve a CineForm, a JPEG XL, az AVIF, a DTS, a DTS-HD MA és a TrueHD (itt már megvan vagy tervben van); 83 tétel, 42 hét, a döntési határidők újraszámolva |
| 2026-10-27 | major | Létrehozva: 89 tétel, heti 2, 45 hét (2026-11-02 – 2027-09-12), döntési kapukkal |
