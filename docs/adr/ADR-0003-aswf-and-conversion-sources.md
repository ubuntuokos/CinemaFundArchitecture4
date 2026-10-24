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
| OpenFX | BSD-3 | plugin-host (G11), lásd a 4. pontot | a Qt6-idővonal után |
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

## 4. Nyitott kérdések a tulajdonosnak

1. **Zárt OFX-effektek.** A `docs/licensing.md` §4 (#37) szerint zárt bővítmény nem töltődhet be
   a CFA4 folyamatába. Ennek a következménye: az OpenFX-host csak GPL-3-mal kompatibilis
   effekteket tölthet be, a kereskedelmi OFX-csomagokat (pl. a Resolve-hoz árultakat) nem.
   Ha ezeket is engedni kell, a GPL-3 7. pontja szerinti plugin-kivételt az első külső GPL-kód
   beemelése **előtt** kell felvenni, mert utána már nem lehet. Kérdés: maradjon a #37 szabálya?
2. **NVENC.** Futáskor zárt NVIDIA-binárist igényel. Javaslat: opcionális, futáskor betöltött modul
   maradjon, mindig gyártófüggetlen párral (ADR 0002, AGENTS.md); a GPL-3 alatti helyzetét jogilag
   át kell nézetni.

## 5. Következmények

- A tesztorákulumok a `python/cfa4_devtools` alá vagy a Rust-tesztekbe kerülnek, és ahogy az
  ffmpeg-orákulumok, hiányzó eszköz esetén kihagyódnak; a teljes CI-ben csak kézi indításra
  futnak.
- Minden beépítés külön PR és külön ADR, a saját motor tartalékként marad.
- Ez az ADR kódot nem változtat. Csak a VISION.md konvertálási sorát és kapcsolódó-dokumentum
  listáját módosítja.

*Ez műszaki-licenc összefoglaló, nem jogi tanács.*
