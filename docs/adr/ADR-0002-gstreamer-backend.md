# ADR 0002 – GStreamer mint kiegészítő háttérmotor

**Dátum:** 2026-10-24 · **Állapot:** elfogadva (a tulajdonos, `ubuntuokos` jóváhagyta) ·
**Érintett crate:** `crates/gstreamer` (`cfa4-gstreamer`, L5 réteg) ·
**Kiegészítette:** [ADR 0005](ADR-0005-media-backends.md) (2026-10-25): a GStreamer az Ubuntu tárolóiból
telepített rendszercsomag (alapértelmezés; elkülönített futtatókörnyezet csak újabb verziót igénylő funkcióhoz, lásd „GStreamer-verzió és frissítés”); az FFmpeg (≥ 9.0.2) is kiegészítő háttér; a hátterek között az
automatikus útválasztó (`cfa4-route`, ADR 0004) választ, alapsorrend: saját → FFmpeg → GStreamer

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

## GStreamer-verzió és frissítés (döntés, 2026-10-24; egységesítve 2026-10-27)

A GStreamer fejlesztői Linuxra nem adnak saját csomagtárolót, az Ubuntu LTS pedig a kiadás után
jellemzően csak hibajavítást hoz, új főverziót nem. A 26.04 GStreamere (1.28.2) a referenciagépen
mindent tud, ami a 2–5. lépéshez kell (Vulkan Video dekódolás, `qml6glsink`, PipeWire, V4L2).

- **Alapértelmezés: a rendszer (Ubuntu) GStreamere.** Ez az alap: a kiadott csomag az Ubuntu
  archívumából függőségként húzza be ([ADR 0005](ADR-0005-media-backends.md)). A kód csak azt
  használja, ami az aktuális LTS GStreamerében megvan; ami újabb, azt futáskori felmérés (`probe()`)
  kapcsolja be.
- **Ha egy funkcióhoz újabb GStreamer kell** (legvalószínűbb a Vulkan Video kódolás): saját,
  **elkülönített** futtatókörnyezet-csomag készül (`cfa4-gstreamer-runtime`, `/opt/cfa4/gstreamer`
  alá), amelyet csak a CFA4 használ, és a CFA4 saját csomagtárolójából (F3 kiadási lánc)
  terjesztünk. **Csak az adott funkcióhoz** veszi át a munkát; a sorrend: elkülönített
  futtatókörnyezet (ha telepítve van és a funkció igényli) → rendszer GStreamer → saját Rust-motor.
  Ugyanezt a fordítást az AppImage is beépítheti; a Flatpak a futtatókörnyezete saját, évente
  frissülő GStreamerét használja.
  - Csak a szükséges bővítmények kerülnek bele; **szabadalmilag korlátozott elemek és kodekek
    nem** (az `ugly` és a `libav` kimarad, fent, 7. pont). Ezeket továbbra is az Ubuntu adja, az
    Ubuntu-modell szerint (`cfa4-restricted-extras`, #57).
  - LGPL: a bináris mellé a forrás is elérhető.
  - **A biztonsági frissítések a CFA4 felelőssége:** az elkülönített futtatókörnyezetet a CFA4
    minden kiadásával frissítjük (és biztonsági hiba esetén soron kívül).
- **A rendszer GStreamerét soha nem cseréljük le:** nem készül ilyen tároló (az a GNOME-ot, a
  lejátszókat, a böngészőket és a PipeWire-t is érintené), és harmadik féltől származó ilyen PPA-t
  sem ajánlunk.

Ez feloldja a korábbi ellentmondást: a #62 (ADR 0005) szerint „csak az Ubuntu GStreamere”, a #51
szerint elkülönített futtatókörnyezet is lehet. Az egységes szabály: az Ubuntu GStreamere az alap és
az alapértelmezés, az elkülönített futtatókörnyezet csak kiegészítés, ha egy funkció újabbat igényel.

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
| 6 | Csomagolás: `.deb` függőségek, AppImage-be csomagolás, Flatpak-futtatókörnyezet. **Döntés (2026-10-25, a 2026-10-24-i „GStreamer nélkül” helyett): a kiadott csomag GStreamerrel fordul**, a GStreamer az Ubuntu archívumából függőségként települ (`Depends` az alapkönyvtárakra, `Recommends` a bővítményekre), nem ágyazzuk be ([ADR 0005](ADR-0005-media-backends.md)). Újabb GStreamer csak elkülönített futtatókörnyezetként: lásd fent, „GStreamer-verzió és frissítés” | tervezett |

Minden lépés külön PR, tesztekkel, és csak akkor kerül be az alkalmazásba, ha a saját motor
tartalékként megmarad.

## Következmények

- Új rendszerfüggőség a fejlesztéshez, ha a funkció be van kapcsolva:
  `libgstreamer1.0-dev`, `libgstreamer-plugins-base1.0-dev`.
- Két médiaútvonalat kell karbantartani. Ezt a fenti szabály kezeli: a saját motor az alap, a
  GStreamer az erősítés és a tartalék.
- A futásidőben elérhető képességek gépenként eltérnek (telepített bővítmények, GPU-meghajtó).
  Ezért a `probe()` eredménye alapján kell dönteni, nem fordítási időben.
