# CFA4 ADR F10 – GPL-3.0-or-later licenc és a külső kód beépíthetősége

**Dátum:** 2026-10-24
**Státusz:** JAVASLAT (a tulajdonos jóváhagyására vár; a 7. pont nyitott döntései nélkül nem végleges)
**Tulajdonos:** ubuntuokos

## 1. Döntés

A CFA4 licence **GPL-3.0-or-later** lesz. Cél: a lehető legkevesebbet kelljen újraírni, vagyis a
GPL-3-mal kompatibilis meglévő nyílt kód beépíthető legyen. Amit így nem lehet átvenni, azt a CFA4
más módon valósítja meg.

- A FilmCraft 0.10.0-ból örökölt kód eredeti szerzői jogi fejlécei, valamint a `LICENSE-MIT`,
  `LICENSE-APACHE` és az eredeti `NOTICE`-tartalom **megmaradnak**. Az MIT és az Apache-2.0
  megengedi, hogy ez a kód GPL-3 alatt terjesztett egész része legyen.
- Az új CFA4-kód és a CFA4 egésze GPL-3.0-or-later alatt jelenik meg (`LICENSE`).
- Üzleti modell: a kész CFA4 ingyenes és szabadon terjeszthető; csak a többletfunkciók és a
  hosszabb támogatási idő fizetős (lásd 6. pont).

## 2. Mi változik a szabályokban

| Korábbi (örökölt FilmCraft) szabály | Új CFA4-szabály |
|---|---|
| Tilos GPL/LGPL/AGPL kódot másolni | GPL-3.0-or-later-rel **kompatibilis** kód beépíthető, forrásmegjelöléssel és a licencfeltételek betartásával |
| Csak megengedő licencű függőség | Bármely GPL-3.0-or-later-rel kompatibilis licenc |
| ffmpeg csak tesztorákulum, soha nem linkelve vagy szállítva | Az FFmpeg GPL-3-as buildje (`--enable-gpl --enable-version3`, `--enable-nonfree` **nélkül**) motorként is használható, saját ADR után |
| `unsafe` csak a `crates/platform`-ban | További FFI-crate-ek C/C++ könyvtárakhoz megengedettek, crate-enként saját ADR-rel és a `crates/platform` szabályaival (ADR 0001) |
| Saját kodek: csak specifikációból, más dekóder forrását olvasni is tilos | Saját, tiszta szobás implementációnál marad; egy könyvtár **beépítése** ettől külön, megengedett út |

Változatlan marad: a „Never crash” szabály, az Adobe-eszközök tilalma, az eszközök (assetek)
licencelése és forrásmegjelölése, a helyi MI elve.

## 3. Licenc-kompatibilitás – gyors referencia

| Licenc | Beépíthető GPL-3.0-or-later alá? | Megjegyzés |
|---|---|---|
| MIT, BSD-2/3, ISC, Zlib, Unicode, CC0, BSL-1.0 | igen | szerzői jogi megjegyzés megtartása |
| Apache-2.0 | igen | GPL-3 alá igen (GPL-2 alá nem); `NOTICE` megtartása, módosított fájlok jelölése |
| MPL-2.0 | igen | az MPL-fájlok MPL alatt maradnak |
| LGPL-2.1-or-later, LGPL-3 | igen | |
| GPL-2.0-or-later, GPL-3.0(-or-later) | igen | |
| **GPL-2.0-only** | **nem** | képesség csak saját kóddal valósítható meg |
| AGPL-3.0 | csak külön döntéssel | hálózati feltétel |
| NC/ND, zárt, „nonfree” | nem | |

Minden beépített projektnél: a licenc fájlonként ellenőrizendő (egy tárolóban lehetnek eltérő
licencű komponensek), és bekerül az `ATTRIBUTION.md`-be.

## 4. Academy Software Foundation (ASWF) projektek

Mind GPL-3.0-or-later-rel kompatibilis licencű (Apache-2.0 vagy BSD-3-Clause).

| Projekt | Licenc | Javasolt mód | Mikor |
|---|---|---|---|
| OpenTimelineIO | Apache-2.0 | tesztorákulum a meglévő `crates/interchange/src/otio.rs`-hez | most |
| OpenAPV | BSD-3 | tesztorákulum a meglévő `crates/apv`-hez | most |
| OpenColorIO + OpenColorIO-Config-ACES | BSD-3 | tesztorákulum a `crates/color`-hoz, majd **első FFI-beépítés** (`crates/ocio`) | orákulum most; beépítés az F8 5. lépésénél (előnézet, GPU) |
| OpenImageIO | Apache-2.0 | a G10 formátumhiányok (PSD, HEIF, JPEG 2000, kamera-RAW) újraírás nélkül; saját függőségei egyenként ellenőrizendők | Qt6-idővonal után |
| OpenEXR + Imath | BSD-3 | hiányzó PIZ/DWA/B44, többrészes és mély EXR | OIIO-val együtt |
| OpenFX | BSD-3 | plugin-host (G11) | Qt6-idővonal után; előtte döntés a 7. pontról |
| xSTUDIO | Apache-2.0 | lejátszó/átnéző részek modulonként | igény szerint |
| OpenRV | Apache-2.0, egyes komponensei eltérnek (`Notice.txt`) | csak komponensenkénti ellenőrzés után | igény szerint |
| MaterialX, OpenShadingLanguage, OpenVDB, openqmc | BSD-3 / Apache-2.0 | 3D/VFX szakasz | később |
| dna | Apache-2.0 | csak ha helyben fut | később |
| OpenCue, rez, aswf-docker | Apache-2.0 | nem szükséges (stúdió-infrastruktúra) | – |

## 5. Konvertálás: a HandBrake kikerül

A HandBrake licence **GPL-2.0-only** (a projekt saját licencfájljában ellenőrizendő), ezért a kódja
GPL-3 alá nem vihető be. A VISION.md táblázatában a helyét ez veszi át:

- **Minta:** Shutter Encoder (GPL-3.0, Java – képesség- és előbeállítás-minta) és Videomass
  (GPL-3.0, Python – az előbeállítás-/profilkezelő logika a Python-rétegbe átemelhető).
- **Motor:** FFmpeg GPL-3-as buildje, `--enable-nonfree` nélkül. A VA-API kódolói (AMD, Intel)
  pótolják a CFA4 hiányzó, nem NVIDIA-s hardveres kódolását.
- Avidemux csak a licencének tisztázása után (licencfájlja GPL v2-t ír, más források
  GPL-2.0-or-later-t).

## 6. Fizetős többletfunkciók GPL-3 alatt

- Ha egy fizetős többletfunkció a GPL-es maggal egy programot alkot (ugyanabba a folyamatba
  linkelve), az is GPL-3 alatt jelenik meg: eladható, de a vevő forrással továbbadhatja.
- A tulajdonos a **saját** kódját kettős licenccel is kiadhatja; más szerzők GPL-kódját nem.
  Külső GPL-kód beemelése után a mag nem licencelhető át zárt többletfunkciókhoz.
- Különálló program, amely a meglévő helyi JSON-hídhoz hasonlóan külön folyamatként, szabványos
  felületen kommunikál, általában nem alkot egy művet a maggal.
- A támogatás (hosszabb támogatási idő) díja a licenctől független.

Ez műszaki-licenc összefoglaló, nem jogi tanács. A végleges szöveget érdemes szakemberrel
átnézetni.

## 7. Nyitott döntések (a tulajdonosé, az első külső GPL-kód beemelése **előtt**)

1. **OpenFX-plugin-kivétel:** kapjon-e a CFA4 a GPL-3 7. pontja szerinti további engedélyt, hogy
   zárt, harmadik féltől származó OFX-effektek betölthetők legyenek. Most a legkönnyebb: az örökölt
   MIT/Apache kód nem akadályozza, a később beemelt külső GPL-kód igen.
2. **Hozzájárulási megállapodás (CLA vagy DCO):** ha a saját kód kettős licencelése cél, a külső
   közreműködőktől kell, mielőtt az első külső hozzájárulás bekerül.
3. **NVENC GPL-3 alatt:** az NVENC futáskor zárt NVIDIA-binárist igényel. Javaslat: opcionális,
   futáskor betöltött modul maradjon, ne kötelező rész; a jogi helyzetét át kell nézetni.

## 8. Ebben a PR-ban

- `LICENSE` (GPL-3.0 teljes szövege), `Cargo.toml` és `pyproject.toml` licencmezője.
- `NOTICE`, `README.md`, `CONTRIBUTING.md`, `AGENTS.md`, `CLAUDE.md`, `docs/contributing.md`:
  a fenti szabályok.
- `VISION.md`: licenc és gyártófüggetlenség alapelvként, a HandBrake-sor cseréje, a nyitott döntések.

Nincs kódváltozás, nincs új függőség. A márkanevek eltávolítása (ArtCraft, FilmCraft és a többi
„Craft”) külön PR.
