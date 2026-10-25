# ADR 0003 – ASWF-projektek és a konvertálás forrásai GPL-3.0-or-later alatt

**Dátum:** 2026-10-24 · **Állapot:** JÓVÁHAGYVA (a tulajdonos, `ubuntuokos`, 2026-10-24-én
jóváhagyta, #38). A jóváhagyás a 10. pont minden pontjára kiterjed.

**Előfeltételek:** a licencváltás (#37, `docs/licensing.md`, már a `main`-ben) és a
GStreamer-háttérmotor (#40, [ADR 0002](ADR-0002-gstreamer-backend.md), összevonásra vár).
Ez az ADR a licencszabályokat nem ismétli meg, csak alkalmazza őket: az Academy Software
Foundation (ASWF) projektjeire, a HandBrake helyére a konvertálásban, a zárt bővítményekre, a
hardveres kódolásra és a GPL-3 után beépíthető könyvtárakra. Az 1–6. pont a döntéseket és a
jelölteket rögzíti, a 7–10. pont a végrehajtási tervet és a jóváhagyandó pontokat.

**Már rögzített tulajdonosi döntések (2026-10):** GPL-3.0-or-later licenc; a GStreamer kiegészítő
háttérmotor, a saját Rust-motor az elsődleges és a tartalék; a HandBrake kikerül; zárt bővítmény
nem töltődik be, a nélkülözhetetlen zárt bővítmények képességét saját kód valósítja meg (4. pont);
hardveres kódolás gyártónként, NVIDIA-függőség nélkül (5. pont).

## 1. Háttér

- A CFA4 licence GPL-3.0-or-later lesz (#37). A cél, hogy a lehető legkevesebbet kelljen
  újraírni: GPL-3-mal kompatibilis kód átvehető, a GPL-2.0-only nem.
- Az ASWF (https://github.com/AcademySoftwareFoundation) a film- és VFX-ipar nyílt forráskódú
  közös alapja. Minden projektje Apache-2.0 vagy BSD-3-Clause licencű, így mind kompatibilis a
  GPL-3.0-or-later-rel.
- A CFA4 több ASWF-képességet már saját Rust-kóddal tud:
  - OpenTimelineIO: `crates/interchange/src/otio.rs`;
  - OpenEXR: saját olvasó és író (scan-line, RLE/ZIP/PXR24), az OpenImageIO-val bitre pontosan
    ellenőrizve;
  - APV: `crates/apv`;
  - színkezelés: `crates/color` (log, LUT, színterek, grade).
- A VISION.md a HandBrake képességeit sorolja a konvertáláshoz, de a HandBrake GPL-2.0-only, így
  a kódja GPL-3 alá nem vihető be (`docs/licensing.md` §3).

## 2. Döntés: ASWF-projektek

Három módon kerülhet be egy ASWF-projekt, ebben a sorrendben:

1. **Tesztorákulum.** A könyvtár csak a tesztekben, külső eszközként fut, és a saját Rust-kód
   eredményét méri hozzá, ahogy az OpenImageIO már ellenőrzi az EXR-t, és ahogy az ffmpeg is
   orákulum. Nem kerül a termékbe, ezért nincs `unsafe`, és nem kell hozzá külön döntés.
2. **Beépítés.** C/C++ könyvtárról van szó, ezért csak FFI-n keresztül, `unsafe` kóddal köthető
   be. Ez a jelenlegi szabály szerint (AGENTS.md §0.3, ADR 0002 4. pont) csak a `crates/platform`
   kivétel. Minden beépítéshez **külön ADR** kell, amely ugyanazokat a feltételeket teljesíti, mint
   az [ADR 0001](0001-platform-ffi.md): `// SAFETY:` megjegyzés minden `unsafe` blokknál,
   biztonságos `Result` API, opcionális Cargo-funkció, a saját Rust-kód tartalékként.
3. **Kódátvétel.** Fájlok vagy függvények átvétele forrásmegjelöléssel, a `docs/licensing.md` §5
   nyilvántartásába írva.

| Projekt | Licenc | Mód | Mikor |
|---|---|---|---|
| OpenTimelineIO | Apache-2.0 | orákulum az `otio.rs`-hez (oda-vissza teszt) | A1 |
| OpenAPV | BSD-3 | orákulum a `crates/apv`-hez (bitpontos összevetés) | A1 |
| OpenColorIO + OpenColorIO-Config-ACES | BSD-3 | orákulum a `crates/color`-hoz (ΔE-mérés), majd az **első beépítés** (`crates/ocio`, külön ADR) | A1 (orákulum), A2 (beépítés, az ADR F8 5. lépésénél) |
| OpenImageIO | Apache-2.0 | beépítés a hiányzó formátumokra (PSD, HEIF, JPEG 2000, kamera-RAW, `docs/gaps.md` G10); a saját függőségei (pl. LibRaw, libheif) egyenként ellenőrizendők | A3 (az ADR F8 3. lépése után) |
| OpenEXR + Imath | BSD-3 | beépítés a hiányzó PIZ/DWA/B44, többrészes és mély EXR-re | A3, az OpenImageIO-val együtt |
| OpenFX | BSD-3 | plugin-host (G11), csak GPL-3-kompatibilis effektekhez (4. pont); első betöltendő csomag a Natron openfx-misc (6. pont); külön folyamatban (9. pont) | A4 (az ADR F8 4. lépése után) |
| xSTUDIO | Apache-2.0 | lejátszó/átnéző részek modulonként (Qt/C++) | A7 |
| OpenRV | Apache-2.0, egyes komponensei eltérnek (`Notice.txt`) | csak komponensenkénti licencellenőrzés után | A7 |
| MaterialX, OpenShadingLanguage, OpenVDB, openqmc | BSD-3 / Apache-2.0 | 3D/VFX-szakasz | A7 |
| dna | Apache-2.0 | csak ha helyben fut (VISION.md 2.3) | A7 |
| OpenCue, rez, aswf-docker | Apache-2.0 | nem szükséges: stúdió-infrastruktúra, nem egygépes alkalmazás | – |

Minden beépítésnél az Apache-2.0 `NOTICE`-szövegét és a licencfájlokat a csomagban szállítani kell
(`docs/licensing.md` §6).

## 3. Döntés: a HandBrake helye a konvertálásban

A VISION.md táblázatában a HandBrake helyét két GPL-3.0 licencű program veszi át **mintaként**:

- **Shutter Encoder** (GPL-3.0, Java): kötegelt konvertálás, előbeállítások, loudness- és
  true-peak-mérés, hangnormalizálás, újrakódolás nélküli vágás, adási és vágókodekek. Java, ezért
  a kódja nem linkelhető a Rust-motorba; képesség- és előbeállítás-mintának használjuk.
- **Videomass** (GPL-3.0, Python): FFmpeg-felület testreszabható előbeállítás- és
  profilkezelővel. A logikája a Python/Qt6 rétegbe átemelhető, a wxPython-felületet Qt6-ban újra
  kell írni.

A konvertálás **motorja** a saját Rust-motor, kiegészítve a GStreamerrel (ADR 0002): ez adja a
gyártófüggetlen hardveres kódolást (VA-API AMD- és Intel-GPU-n). Az FFmpeg csak akkor kerül a
termékbe, ha a saját motor és a GStreamer együtt sem elég; akkor saját ADR kell hozzá, csak
GPL-3-as builddel (`--enable-gpl --enable-version3`, soha `--enable-nonfree`).

Az Avidemux csak a licencének tisztázása után jöhet szóba: a tárolójának licencfájlja GPL v2-t ír,
más források GPL-2.0-or-later-t.

Megjegyzés: a `docs/licensing.md` §3 szerint egy GPL-2.0-only program a kódja átvétele nélkül,
**külön folyamatként** indítva használható. Így a `HandBrakeCLI` külső programként hívható lenne,
ha a felhasználó telepítette; ha a CFA4-csomagba kerül, a licencét és a forrásajánlatát is
mellékelni kell. A tulajdonos döntése szerint a HandBrake kikerül, ez csak tartalék lehetőség.

## 4. Döntés: zárt bővítmények

**A tulajdonos döntése (2026-10-24):** a CFA4 nem tölt be zárt bővítményt, és nem kap GPL-3
plugin-kivételt. Ehelyett minden olyan zárt bővítmény képességét, amely hasznos és
nélkülözhetetlen, a CFA4 **saját kóddal valósítja meg**.

- Ez megerősíti a `docs/licensing.md` §4 (#37) szabályát: zárt bővítmény nem töltődhet be a CFA4
  folyamatába. Az OpenFX-host (G11) így csak GPL-3-mal kompatibilis effekteket tölt be.
- A megvalósítás a meglévő szabályok szerint történik: a zárt bővítmény kódját, ikonjait,
  előbeállításait, LUT-jait és egyéb eszközeit nem vesszük át; a viselkedést nyilvános
  dokumentációból és megfigyelésből építjük újra, ahol lehet, GPL-3-mal kompatibilis nyílt kódra
  támaszkodva.
- Melyik bővítmény „hasznos és nélkülözhetetlen”: erről a tulajdonos dönt. A jelöltek listája a
  `docs/gaps.md`-be kerül (G11 plugin-hosting, G12 MI, G18 effektmélység), bővítményenként a
  pótolandó képességgel, a nyílt alapként használható projektekkel és a mérési móddal
  (VISION.md 2. pont: bizonyíték, nem állítás).
- Külön folyamatként futó, dokumentált felületen kommunikáló fizetős CFA4-többletfunkciók
  (`docs/licensing.md` §4) ettől függetlenül lehetségesek.

## 5. Döntés: hardveres kódolás gyártónként

**A tulajdonos döntése (2026-10-24):** az NVENC az NVIDIA-út; AMD, Intel és más GPU-k mást
igényelnek. Ezért minden gyártónak saját útja van, közös felület mögött:

| GPU | Hardveres kódolás útja | Megjegyzés |
|---|---|---|
| NVIDIA | NVENC (`crates/platform`, ma is megvan) | a meghajtó zárt könyvtárát futáskor tölti be; opcionális |
| AMD | VA-API (Mesa `radeonsi`), GStreamer `va*enc` elemekkel (ADR 0002 3. lépés) | nyílt meghajtó; az AMD AMF nem kell, mert zárt futtatókörnyezetet igényel |
| Intel | VA-API (`iHD` meghajtó), ugyanígy | a oneVPL/Quick Sync csak akkor, ha a VA-API mérhetően kevés |
| bármely Vulkan-képes GPU | Vulkan Video (GStreamer `vulkan*enc`, ahol a meghajtó tudja) | gyártófüggetlen jövőbeli út |
| nincs GPU vagy hiba | a saját szoftveres kódolók (pl. `crates/h264enc`) | mindig elérhető tartalék |

- Mind ugyanazon kódolófelület (`cfa4_export::VideoEncoder`) mögött van, a választást a futásidejű felmérés
  (a GStreamer `probe()`-ja és a #39 gyorsítási terve) és mérés dönti el, nem fordítási idő.
- Egy funkció sem lehet csak NVIDIA-n elérhető (AGENTS.md, ADR 0002): ha egy képesség NVENC-cel
  megvan, a VA-API-s és a szoftveres útnak is meg kell lennie.
- Az NPU-k a helyi MI-hez valók (VISION.md 5. pont), a videókódoláshoz nem.
- Az NVENC GPL-3 alatti helyzetét (zárt meghajtó-könyvtár futáskori betöltése) a kiadás előtt
  jogilag át kell nézetni; ha gond van vele, az NVIDIA-gépek a szoftveres kódolót használják
  (a nyílt meghajtós VA-API NVIDIA-n csak dekódolni tud).

## 6. Új lehetőségek a GPL-3.0-or-later után

A licencváltással olyan GPL-es és LGPL-es könyvtárak is beépíthetők, amelyeket korábban saját
kóddal kellett volna megírni. Mind **jelölt**: mindegyikhez külön ADR és PR kell (FFI-szabály,
saját motor tartalékként, fájlonkénti licencellenőrzés, `docs/licensing.md` §5 nyilvántartás).

| Jelölt | Licenc | Mit ad | `docs/gaps.md` |
|---|---|---|---|
| Natron openfx-misc | GPL-2.0-or-later | kész OFX-effektek: keying (chroma key, despill), színkorrekció és görbék, transzformáció és pontkövetés, retime és deinterlace, zajszűrés és élesítés | G11, G18 |
| Natron további OFX-csomagjai (openfx-io, openfx-arena) | ellenőrizendő | további effektek és I/O | G11 |
| LibRaw (az OpenImageIO-n át vagy közvetlenül) | LGPL-2.1 vagy CDDL-1.0 | kamera-RAW olvasás (a saját fejlesztés becslése 60–100 óra) | G10 |
| libheif | LGPL-3.0 | HEIF/HEIC képek | G10 |
| x265 | GPL-2.0-or-later (a licencéről volt vita, ellenőrizendő) | szoftveres HEVC-kódolás (ma csak hardveres van) | G13 |
| x264 | GPL-2.0-or-later | jobb minőségű szoftveres H.264-kódolás | G13 |
| LAME | LGPL | MP3-export | G13 |
| Rubber Band | GPL-2.0-or-later | hang időnyújtása és hangmagasság-váltás | G18 |
| lilv (LV2-host) | ISC | LV2-hangbővítmények betöltése (sok GPL-es LV2-bővítmény) | G11 |
| frei0r | GPL-2.0-or-later | további videóeffektek | G11, G18 |
| Kdenlive, MLT, Audacity, Krita kódrészei | fájlonként ellenőrizendő (GPL-2.0-only fájl nem vehető át) | képességek átvétele újraírás helyett | VISION.md 4. pont |

Ami a licenctől függetlenül marad:

- **Szabadalmak.** A H.264, a HEVC és az AAC szabadalmi kérdése független a szerzői jogi
  licenctől; az x264 és az x265 terjesztése előtt külön meg kell vizsgálni.
- **Mérnöki szabályok.** Az AGENTS.md `unsafe`- és „pure Rust”-szabálya, valamint az, hogy az
  ffmpeg nem linkelhető, továbbra is érvényes; minden beépítéshez az ADR-nek kell kivételt
  kérnie, az ADR 0001 feltételeivel.

## 7. Végrehajtási terv

Minden szakasz külön PR (beépítésnél előtte külön ADR), a saját motor mindig tartalék, és a
VISION.md 6. pontjának sorrendjéhez igazodik: a Qt6-felület az első, ezért a beépítések az
ADR F8 megfelelő lépéséhez kötődnek. A becslések durva, ügynökórában megadott értékek a
`docs/gaps.md` kalibrációja szerint; a hardveres és jogi lépések emberi munkát is igényelnek.

| Szakasz | Tartalom | Feltétel / időzítés | Becslés |
|---|---|---|---|
| **A0** | A #40 (ADR 0002) és ez a PR összevonva; az ADR 0003 bekerül a `docs/audit/README.md` tartalomjegyzékébe | most | 1–2 óra |
| **A1** | Tesztorákulumok: OpenTimelineIO oda-vissza teszt az `otio.rs`-hez; OpenAPV bitpontos összevetés a `crates/apv`-vel; OpenColorIO + ACES-config ΔE-mérés a `crates/color`-hoz. A termékbe nem kerülnek, hiányzó eszköznél kihagyódnak | most indítható, a Qt6-munkát nem akasztja | 10–20 óra |
| **A2** | **ADR 0004: FFI-kivétel elve** (az AGENTS.md §0.3 bővítése: további FFI-crate-ek az ADR 0001 feltételeivel), majd `crates/ocio`: OpenColorIO-színkezelés, ACES-config olvasása | az ADR F8 5. lépésénél (előnézet, GPU) | 40–80 óra |
| **A3** | OpenImageIO + OpenEXR + LibRaw + libheif: PSD, HEIF/HEIC, JPEG 2000, kamera-RAW, a hiányzó EXR-tömörítések, többrészes és mély EXR (G10) | az ADR F8 3. lépése (médiaböngésző) után | 60–120 óra |
| **A4** | OpenFX-host **külön folyamatban** (lásd 9. pont), első csomag a Natron openfx-misc (G11, G18) | az ADR F8 4. lépése (idővonal) után | 120–250 óra |
| **A5** | Szoftveres kódolók: x265 (HEVC), x264 (H.264), LAME (MP3) (G13); előtte szabadalmi vizsgálat | az ADR F8 6. lépésénél (export) | 30–60 óra + jogi vizsgálat |
| **A6** | Hang: Rubber Band (időnyújtás, hangmagasság), LV2-host lilv-vel, szintén külön folyamatban | a hangszerkesztési szakaszban (VISION.md 6.4) | 60–120 óra |
| **A7** | Később: xSTUDIO- és OpenRV-részek, MaterialX, OpenShadingLanguage, OpenVDB, openqmc, dna, frei0r | a 3D/VFX- és az átnézési szakaszban | külön tervezendő |

A hardveres kódolás gyártónkénti útja (5. pont) nem itt, hanem az ADR 0002 3. lépésében és a
#39 gyorsítási tervében valósul meg; ez a terv csak a szoftveres kódolókat adja hozzá.

## 8. Elfogadási feltételek (bizonyíték, nem állítás)

| Szakasz | Akkor kész, ha |
|---|---|
| A1 | OTIO: legalább 10 saját tesztidővonal exportja és visszaolvasása után a sávok, klipek, be- és kipontok, sebességek és jelölők egyeznek. OpenAPV: a hivatalos tesztfolyamok minden kockája bitre egyezik. OCIO: az ACES-transzformok egy rácsra mért legnagyobb eltérése ΔE2000 ≤ 1,0, vagy az eltérés dokumentált hibaként a `docs/gaps.md`-be kerül |
| A2 | Az OCIO-s előnézet és export a CPU-s OCIO-referenciától ΔE2000 ≤ 1,0-ra tér el; OCIO nélkül a program a `crates/color`-ral fut tovább; az ADR 0001 minden feltétele teljesül (`// SAFETY:`, `Result` API, opcionális funkció) |
| A3 | Minden új formátum fájljai megnyílnak, és a kép egyezik az `oiiotool` referenciájával; sérült fájlra hibaüzenet jön, nem összeomlás (mutációs fuzz-teszttel) |
| A4 | Az openfx-misc effektjei betöltődnek és renderelnek; az eredmény a Natron referenciájától PSNR ≥ 50 dB; egy összeomló bővítmény nem viszi magával a programot |
| A5 | A kimenetet az ffprobe és a saját dekóder is helyesnek látja; bitráta–PSNR görbe dokumentálva; a szabadalmi vizsgálat eredménye rögzítve |
| A6 | Az időnyújtás és a hangmagasság-váltás hallható hibák nélkül, mérve; egy összeomló LV2-bővítmény nem viszi magával a programot |

Minden szakasz teljesíti a meglévő kapukat is (`cargo xtask ci`, `cargo deny check licenses`), és
minden átvett kód bekerül a `docs/licensing.md` §5 nyilvántartásába.

## 9. Kockázatok és kezelésük

- **Összeomlás idegen kódban.** Az OFX- és LV2-bővítmények C/C++ kódja a programon belül futva
  egy hibával az egész alkalmazást leállíthatja, amit Rust `catch_unwind` nem fog el. Ez ütközne
  az AGENTS.md 0. pontjával („Never crash”). Ezért a bővítmény-hostok **külön folyamatban**
  futnak, a meglévő helyi JSON-hídhoz hasonló felületen; a bővítmény összeomlása után a
  folyamat újraindul, a projekt nem sérül.
- **`unsafe` kód növekedése.** Minden FFI-crate szűk, ellenőrzött határfelület mögött van
  (ADR 0001 minta), mutációs fuzz-tesztekkel.
- **Rendszerfüggőségek.** Az új könyvtárak opcionális Cargo-funkció mögött vannak; nélkülük a
  munkaterület fordul és tesztelhető, a csomagolás (`.deb`, AppImage, Flatpak, `tar.gz`) külön
  ellenőrzést kap.
- **Licenc fájlonként.** Nagy projektekben vegyes licencek vannak; GPL-2.0-only fájl nem kerülhet
  be. A `cargo deny` és a §5 nyilvántartás ezt őrzi.
- **Szabadalmak.** H.264, HEVC, AAC: az A5 előtt jogi vizsgálat; addig az x264 és az x265 nem
  kerül kiadásba.
- **NVENC jogi helyzete** (5. pont): kiadás előtt vizsgálandó; ha gond van, NVIDIA-n a szoftveres
  kódoló fut.

## 10. Jóváhagyandó pontok

A PR összevonásával a tulajdonos ezeket hagyja jóvá:

1. Az 1–6. pont döntései és jelöltjei, a 7. pont szakaszai és sorrendje.
2. Az FFI-kivétel elve: további C/C++ könyvtár csak saját ADR-rel, az ADR 0001 feltételeivel
   köthető be (az első ilyen az ADR 0004, az A2-ben). Az AGENTS.md szövege az ADR 0004-gyel
   változik, nem ezzel a PR-ral.
3. A bővítmény-hostok (OpenFX, LV2) külön folyamatban futnak.
4. Az FFmpeg a termékbe csak saját ADR-rel kerülhet; addig külső tesztorákulum marad.
5. Az x264 és az x265 kiadása előtt szabadalmi, az NVENC kiadása előtt licencjogi vizsgálat kell.
6. Az A1 (tesztorákulumok) az összevonás után azonnal indulhat.

## 11. Következmények

- A tesztorákulumok a `python/cfa4_devtools` alá vagy a Rust-tesztekbe kerülnek, és ahogy az
  ffmpeg-orákulumok, hiányzó eszköz esetén kihagyódnak; a teljes CI-ben csak kézi indításra
  futnak.
- Minden beépítés külön PR és külön ADR, a saját motor tartalékként marad.
- Ez az ADR kódot nem változtat; a megvalósítás a 7. pont szakaszaiban, külön PR-okban történik.
  A VISION.md konvertálási sorát és kapcsolódó-dokumentum
  listáját, valamint a `docs/gaps.md` G10, G11, G13 és G18 pontjait egészíti ki a 6. pont
  jelöltjeivel.

*Ez műszaki-licenc összefoglaló, nem jogi tanács.*
