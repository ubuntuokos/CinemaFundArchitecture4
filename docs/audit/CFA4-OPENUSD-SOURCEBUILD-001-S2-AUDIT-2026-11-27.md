<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# OpenUSD forrásbuild: S2 licenc- és eredetaudit

**Dátum:** 2026-11-27. **Státusz:** audit kész, **SB-G2 tulajdonosi jóváhagyásra vár** (SB-G7 külön nyitva).
**Terv:** `docs/plans/CFA4-PIXAR-DONOR-001-EGYESITETT-1.1.md` (G-USD-1 (b)), melléklet: `CFA4-OPENUSD-SOURCEBUILD-001-0.1.md` §5.
A vizsgálat olvasásos volt, a rögzített forrásokon (`packaging/usd-version.env`); a forrásból semmi nem került a CFA4 repóba.

## 1. Mit olvastunk és mit nem
- **Olvastuk:** a három rögzített fa (OpenUSD 26.11.1, oneTBB 2021.12.0, OpenSubdiv 3.6.1) licencfájljait, a `pxr/base`, `pxr/usd`, `pxr/usdValidation` ágak (2504 forrásfájl, teszt nélkül) fejléceit, a bundlolt külső könyvtárak fejléceit és README-it.
- **Nem olvastuk:** `pxr/imaging`, `pxr/usdImaging` (a minimális mag nem építi), `extras/`, `third_party/` (DCC-bővítmények), `docs/`. A teljes jogi szöveg és a TOST-1.0 §6 jogi értelmezése nem az én dolgom (SB-G7).
- **Nincs licencszkenner:** egyszerű fejléc-keresés (`grep`) készült, nem automatikus szkenner. Ez nem teljes jogi átvilágítás; a szkenner kiválasztása (pl. ScanCode) külön döntés.

## 2. Fő licencek
| Komponens | Licenc | Megjegyzés |
|---|---|---|
| OpenUSD | TOST-1.0 (Apache-2.0, kivéve a 6. szakasz) | `LICENSE.txt` SHA-256 a pinben; `NOTICE.txt` kötelezően csomagolandó |
| oneTBB | Apache-2.0 | `LICENSE.txt` pinelve; nincs `NOTICE`. A `third-party-programs.txt` szerint hwloc (BSD-3, nem épül a mi libtbb-nkbe), Google-eredetű BSD-3 rész, és egy libstdc++-megkerülő kód (`src/tbb/exception.cpp`, GPL-3.0 + GCC Runtime Library Exception 3.1) |
| OpenSubdiv | Modified Apache 2.0 | csak az F3 fázisban épül |

## 3. Az USD fában bundlolt külső kód (a megépülő magban)
| Hely | Eredet | Licenc |
|---|---|---|
| `pxr/base/js/rapidjson/` | RapidJSON (Tencent, Milo Yip; `uri.h`: IBM) | MIT |
| `pxr/base/tf/pxrLZ4/` | LZ4 (Yann Collet) | BSD-2-Clause |
| `pxr/base/tf/pxrCLI11/` | CLI11 2.3.1 (Univ. of Cincinnati, Henry Schreiner) | BSD-3-Clause |
| `pxr/base/tf/pxrTslRobinMap/` | tsl::robin-map (Thibaut Goetghebuer-Planchon) | MIT |
| `pxr/base/tf/pxrDoubleConversion/` | double-conversion (V8 projekt, Google) | BSD-3-Clause |
| `pxr/base/gf/ilmbase_half*` | IlmBase 2.5.3 (Industrial Light & Magic) | BSD-3-Clause |
| `pxr/base/pegtl/` | PEGTL 3.2.7 (Colin Hirsch, Daniel Frey) | MIT (a fejléc a projekt `LICENSE`-ére hivatkozik; a fában külön licencfájlt nem találtam) |
| `pxr/base/arch/darwin.h`, `pxr/usd/usdPmc/` | Apple | USD licenc (fejlécben „LICENSE.txt”); a PMC bővítmény alapból nem épül (`PXR_BUILD_PMC_PLUGIN`) |
| `pxr/usd/plugin/usdDraco/` | Google LLC | USD licenc; a Draco bővítmény ki van kapcsolva (`PXR_BUILD_DRACO_PLUGIN=OFF`) |

Eltérés a fejlécekben: a bundlolt külső könyvtárak fájljai nem Pixar-szerzői jogi sort viselnek; a fejléceket nem szabad módosítani (AGENTS.md).
Hiányosság: a bundlolt könyvtárak MIT/BSD szövegei a saját fejlécükben vannak; külön licencfájl nem mindegyikhez van (PEGTL, robin-map). A csomagba az USD `LICENSE.txt` + `NOTICE.txt` mellé külön `THIRD-PARTY-NOTICES` fájl kell (S4 feladat), amely ezeket a fejléc-szövegeket idézi.

## 4. Szállított bináris
Az S1 próbabuild (`docs/development/usd-s1-build-trial.md`): 40 megosztott könyvtár; az `ldd` szerint csak libtbb, libstdc++, libm, libgcc_s, libc. A bundlolt kód (3. szakasz) tehát be van fordítva a `libusd_*` könyvtárakba. A libtbb mellé kerülnek a TBB Apache-2.0 szövegei.

## 5. Reused code register
A CFA4 repó semmilyen USD/TBB/OSD fájlt nem tartalmaz (letöltés kézzel, `target/third-party`, nincs commit), ezért a `docs/licensing.md` §5 táblájába **most nem kerül sor**. A sorok az S4-ben kerülnek be, ha a bináris csomagba kerül, a csomag tartalmával (a három verzió, commit, licenc).

## 6. SBOM
`packaging/usd-sbom.cdx.json`: CycloneDX 1.5, kézzel írt, a pinekből és ebből az auditból. A generált, buildhez kötött SBOM (BUILDINFO-val) az S3 feladata.

## 7. Nyitott, a tulajdonosra vár
- **SB-G7:** a TOST-1.0 §6 és a GPL-3.0-or-later összeférhetőségének jóváhagyása (az USD külön folyamatban fut, nem linkelt kód; ez a terv feltevése, jogi vélemény nem).
- **D-SB-4:** a Pixar aláíró kulcs ujjlenyomata (az USD tag aláírása még nincs ellenőrizve).
- **SB-G2:** ezen audit jóváhagyása, beleértve a fenti „nem olvastuk” korlátot.
