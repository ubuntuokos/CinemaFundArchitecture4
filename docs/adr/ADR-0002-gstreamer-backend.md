# ADR 0002 – GStreamer mint kiegészítő háttérmotor

**Dátum:** 2026-10-24 · **Állapot:** elfogadva (a tulajdonos, `ubuntuokos` jóváhagyta) ·
**Érintett crate:** `crates/gstreamer` (`cfa4-gstreamer`, L5 réteg)

## Háttér

A CFA4 saját Rust médiamotorja (konténerek, kodekek, render, export) marad az alap
([VISION.md](../../VISION.md) 2.1). A motor azonban több területen még nem tud annyit, amennyit a
célok kívánnak:

- **Hardveres kódolás Linuxon csak NVIDIA-n van** (NVENC). AMD és Intel GPU-n az export
  szoftveres. Ez ellentmond a gyártófüggetlenségi célnak.
- **Hardveres dekódolás** csak H.264-re és HEVC-re van (natív VA-API és NVDEC, [ADR 0001](0001-platform-ffi.md)).
  VP9-re és AV1-re nincs.
- **Qt6-előnézet, felvétel (kamera, képernyő, PipeWire)** és a motor által még nem ismert
  formátumok megnyitása hiányzik.

A GStreamer (https://gstreamer.freedesktop.org, GitHub-tükör: https://github.com/gstreamer) ezeket
a rendszer csomagjaiból adja, gyártófüggetlenül (VA-API, Vulkan Video, V4L2, PipeWire), és van
Qt6/QML videókimenete is.

## Döntés

A GStreamer **kiegészítő háttérmotor** lesz. Nem váltja ki a saját motort.

1. **A saját Rust motor az elsődleges.** A GStreamer ott lép be, ahol a motor nem tud valamit,
   vagy ahol mérés igazolja, hogy jobb (például hardveres kódolás AMD/Intel GPU-n).
2. **A saját motor mindig tartalék.** Ha a GStreamer, egy bővítménye vagy a hardveres gyorsítás
   hiányzik vagy hibázik, a program a saját motorral fut tovább, ugyanúgy, ahogy az ADR 0001
   hibrid dekódere.
3. **Opcionális.** A `cfa4-gstreamer` crate `gstreamer` Cargo-funkciója alapból ki van kapcsolva.
   Nélküle a munkaterület GStreamer nélkül is fordul és tesztelhető, és minden belépési pont
   `Error::Unavailable`-t ad.
4. **Nincs `unsafe`.** A crate csak a gstreamer-rs kötések biztonságos API-ját használja
   (MIT OR Apache-2.0). Az `unsafe` kivétel továbbra is csak a `crates/platform`-ra vonatkozik.
5. **Nem crash-el** (AGENTS.md 0. pont): minden hiba `Result`, a kötésekben keletkező pánikot a
   crate elkapja és hibaként adja vissza, a bemeneti útvonalakat és időkorlátokat előre ellenőrzi.
6. **Licenc.** A GStreamer könyvtárai LGPL-2.1-or-later licencűek, és dinamikusan, a rendszerből
   töltődnek be. Ez összefér a CFA4 GPL-3.0-or-later licencével. GStreamer-forráskódot nem
   másolunk és nem tárolunk a repóban.
7. **Kerülendő bővítménycsomagok.** Az `ugly` (például x264, GPL, szabadalmi kérdések) és a
   `libav` (FFmpeg) csomag elemei nem részei a tervnek. A `CATALOGUE` csak LGPL vagy engedékenyebb
   elemeket sorol fel, ezt egységteszt őrzi, és a `probe()` futáskor jelenti minden elem
   bővítményének licencét.
8. **Bizonyíték, nem állítás** (VISION.md 2.5). Hogy egy formátumot vagy feladatot a saját motor
   vagy a GStreamer kezel, azt teszt és valódi gépes mérés dönti el, nem feltételezés.

## A tartalék szabálya (2. lépés)

A beépített megnyitók mindig elsők. A GStreamer csak akkor veszi át a fájlt, ha

- a beépített megnyitók nem támogatottnak vagy dekódolhatatlannak jelzik (`Unsupported`, `Decode`;
  I/O-hiba és offline média változatlanul továbbmegy), vagy
- egy videó-kiterjesztésű fájlt videó nélkül nyitnak meg (például az Ogg Theora + Vorbis fájlt
  csak hangként), és a GStreamer talál benne videót.

Ha a GStreamer is elutasítja, a beépített eredmény marad, így a regisztráció soha nem ront el
egy addig működő fájlt. Az alkalmazások csak a `gstreamer` Cargo-funkcióval fordítva
regisztrálják (`cfa4_gstreamer::register()`), és csak ha a rendszeren van GStreamer.

A GStreamer-forrás RGBA 8 bites képet és a kért mintavételezésű 32 bites lebegőpontos hangot ad.
Pontos (accurate) kereséssel ugrik, egymást követő kérésnél csak a következő puffert veszi,
így lejátszáskor nem keres minden képkockánál. A nem használt adattípusokhoz `fakesink` kerül,
mert a `playbin` automatikus kimenetei (hangkártya, kijelző) kijelző nélküli gépen összeomolhatnak.

## Viszony az ADR 0001-hez

A natív VA-API és NVDEC dekóder a `crates/platform`-ban marad. A GStreamer nem helyettesíti őket,
hanem azt adja hozzá, amit ők nem fednek le (VP9, AV1, Vulkan Video, hardveres kódolás nem
NVIDIA GPU-n). Ha ugyanarra a feladatra mindkettő képes, mérés dönti el az alapértelmezést.

## Lépések

| Lépés | Tartalom | Állapot |
|---|---|---|
| 1 | `cfa4-gstreamer` crate: `probe()` (verzió, elemek, licencek) és `discover()` (konténer, időtartam, sávok), tesztekkel | kész (#40) |
| 2 | Importálás: a motor által meg nem nyitható (vagy videó nélkül megnyitott) fájlok GStreamer-forrásként (`MediaSource`); `cfa4_media::fallback` regisztráció, új kiterjesztések (`.ogv`, `.flv`, `.wmv`, `.wma` …) | kész (#40) |
| 3 | Hardveres kódolás exporthoz AMD/Intel GPU-n (`vah264enc`, `vah265enc`, `vaav1enc`) a `VideoEncoder` mögött | tervezett |
| 4 | Qt6-előnézet `qml6glsink`-kel, az ADR F8 5. lépéséhez igazítva | tervezett |
| 5 | Felvétel: PipeWire (képernyő, hang) és V4L2 (kamera, rögzítőkártya) | tervezett |
| 6 | Csomagolás: `.deb` függőségek, AppImage-be csomagolás, Flatpak-futtatókörnyezet. **Döntés (2026-10-24): a kiadott csomag GStreamer nélkül fordul**; a `gstreamer` funkció fejlesztői és kísérleti buildekre marad | tervezett |

Minden lépés külön PR, tesztekkel, és csak akkor kerül be az alkalmazásba, ha a saját motor
tartalékként megmarad.

## Következmények

- Új rendszerfüggőség a fejlesztéshez, ha a funkció be van kapcsolva:
  `libgstreamer1.0-dev`, `libgstreamer-plugins-base1.0-dev`.
- Két médiaútvonalat kell karbantartani. Ezt a fenti szabály kezeli: a saját motor az alap, a
  GStreamer az erősítés és a tartalék.
- A futásidőben elérhető képességek gépenként eltérnek (telepített bővítmények, GPU-meghajtó).
  Ezért a `probe()` eredménye alapján kell dönteni, nem fordítási időben.
