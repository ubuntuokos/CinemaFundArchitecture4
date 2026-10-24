# ADR 0003 – ASWF-projektek és a konvertálás forrásai GPL-3.0-or-later alatt

**Dátum:** 2026-10-24 · **Állapot:** JAVASLAT (a tulajdonos, `ubuntuokos` jóváhagyására vár)

**Előfeltételek:** a licencváltás (#37, `docs/licensing.md`) és a GStreamer-háttérmotor
(#40, [ADR 0002](ADR-0002-gstreamer-backend.md)). Ez az ADR a licencszabályokat nem ismétli meg,
csak alkalmazza őket két területre: az Academy Software Foundation (ASWF) projektjeire és a
HandBrake helyére a konvertálásban.

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
| OpenTimelineIO | Apache-2.0 | orákulum az `otio.rs`-hez (oda-vissza teszt) | most |
| OpenAPV | BSD-3 | orákulum a `crates/apv`-hez (bitpontos összevetés) | most |
| OpenColorIO + OpenColorIO-Config-ACES | BSD-3 | orákulum a `crates/color`-hoz (ΔE-mérés), majd az **első beépítés** (`crates/ocio`, külön ADR) | orákulum most; beépítés az ADR F8 5. lépésénél (előnézet, GPU) |
| OpenImageIO | Apache-2.0 | beépítés a hiányzó formátumokra (PSD, HEIF, JPEG 2000, kamera-RAW, `docs/gaps.md` G10); a saját függőségei (pl. LibRaw, libheif) egyenként ellenőrizendők | a Qt6-idővonal után |
| OpenEXR + Imath | BSD-3 | beépítés a hiányzó PIZ/DWA/B44, többrészes és mély EXR-re | az OpenImageIO-val együtt |
| OpenFX | BSD-3 | plugin-host (G11), csak GPL-3-kompatibilis effektekhez (4. pont); első betöltendő csomag a Natron openfx-misc (6. pont) | a Qt6-idővonal után |
| xSTUDIO | Apache-2.0 | lejátszó/átnéző részek modulonként (Qt/C++) | igény szerint |
| OpenRV | Apache-2.0, egyes komponensei eltérnek (`Notice.txt`) | csak komponensenkénti licencellenőrzés után | igény szerint |
| MaterialX, OpenShadingLanguage, OpenVDB, openqmc | BSD-3 / Apache-2.0 | 3D/VFX-szakasz | később |
| dna | Apache-2.0 | csak ha helyben fut (VISION.md 2.3) | később |
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

- Mind ugyanazon kódolófelület (`filmcraft_export::VideoEncoder`) mögött van, a választást a futásidejű felmérés
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

## 7. Következmények

- A tesztorákulumok a `python/cfa4_devtools` alá vagy a Rust-tesztekbe kerülnek, és ahogy az
  ffmpeg-orákulumok, hiányzó eszköz esetén kihagyódnak; a teljes CI-ben csak kézi indításra
  futnak.
- Minden beépítés külön PR és külön ADR, a saját motor tartalékként marad.
- Ez az ADR kódot nem változtat. A VISION.md konvertálási sorát és kapcsolódó-dokumentum
  listáját, valamint a `docs/gaps.md` G10, G11, G13 és G18 pontjait egészíti ki a 6. pont
  jelöltjeivel.

*Ez műszaki-licenc összefoglaló, nem jogi tanács.*
