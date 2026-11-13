# CFA4-HW-CONTROL-SURFACES-001 – M0 audit

**Dátum:** 2026-11-13 · **Alap:** `main` `5baa2fe` · **Terv:** CFA4-HW-CONTROL-SURFACES-001 FINAL 1.1
(jóváhagyva 2026-10-31, kiegészítve és újra jóváhagyva 2026-11-13; <https://claude.ai/artifact/2YNfVud77rJovdg7C1RLvb>) ·
**Illesztés:** [`CFA4-HW-CONTROL-SURFACES-001-FINAL-1.1-CFA4-ILLESZTES.md`](../../plans/CFA4-HW-CONTROL-SURFACES-001-FINAL-1.1-CFA4-ILLESZTES.md)

Ez az M0 fázis jelentése. Az M0 a terv 9. fejezete szerint hat témát néz meg: a CFA4-parancsokat,
a MIDI-helyzetet, a crate-ek helyét, a licenceket, az eszközlistát és a rendszerszintű utakat.
A kapu: az illesztési terv jóváhagyása. Kódot ez a jelentés nem változtat.

Valódi eszköz ebben a körben nem volt csatlakoztatva. Ahol egy állítás csak forrásból vagy
dokumentációból származik, a jelentés ezt kiírja; ilyen állítás nem adhat VERIFIED állapotot (terv 4.1).

## 0. A legfontosabb megállapítás: átfedő projekt

A `main` mellett három nyitott PR egy másik, a tulajdonos által 2026-11-13-án jóváhagyott projekthez,
a **CFA4-CONTROL-SURFACE-001**-hez tartozik (ADR 0012):

| PR | Tartalom |
|---|---|
| #1184 | F0-dokumentumok: ADR 0012, műszaki specifikáció, HID-értékelés, protokollszerződés (`surface.*`), profilséma |
| #1187 | `cfa4-surface-model` (L1) és `cfa4-surface` (L5) crate; Beállítások ▸ Hardware & Devices ▸ Control Surfaces lap (MIDI, Contour ShuttlePRO / ShuttleXpress) |
| #1193 | MIDI Monitor ugyanazon a lapon (az #1187 ágára épül) |

Ugyanaz a terület, más architektúrával (leképezés a motorban, eszköz-I/O a `serve` folyamatban,
hidapi elutasítva). A `coordination.md` §2.8 szerint ezt jelezni kell. A tulajdonos 2026-11-13-án
döntött: **közös alapra építünk**; a FINAL 1.1 a CONTROL-SURFACE-001 crate-jeire, motorbeli
leképezésére és lapjára épül. A részleteket az illesztési melléklet tartalmazza.

## 1. CFA4-parancsok

| Kérdés | Eredmény | Forrás |
|---|---|---|
| Van-e parancsregiszter, amelyet egy vezérlő hívhat? | Igen: `CommandSpec` (`crates/engine/src/commands.rs`), `Session::execute(id, params)`; a Qt a `serve` `engine.execute` és `engine.commands` hívásával éri el | `docs/architecture.md` 1., `docs/qt-gui.md` „Protocol” |
| Kell-e párhuzamos műveletrendszer? | Nem. A CONTROL-SURFACE-001 is parancsazonosítóra köt (`surface.bind {command, params}`), a végrehajtás a `surfaceAction` eseményen át a Qt meglévő parancsútján történik | #1184 protokollszerződés 3–4. |
| A CFA4 saját ablakai és munkaterületei (terv 1.1) | Parancsként léteznek: `window.panel.*`, `window.nextPanel` / `prevPanel`, `window.closePanel`, `window.toggleFullScreen`, `window.workspace.*` (edit, color, audio, effects…) | `crates/engine/src`, `python/cfa4_qt/window_menu.py`, `dock_commands.py` |
| Billentyűparancs-profil minta (közvetett út, 8.3) | `crates/engine/src/shortcuts.rs`: beépített presetek, aktív fájl, ütközések | ADR 0012 „Háttér” |

Következmény: az M1 virtuális deckje és minden adapter csak parancsazonosítót köt; új parancs a
felülethez nem kell, kivéve a gombkép- és kijelzőkimenetet (illesztés E4).

## 2. MIDI-helyzet

| Kérdés | Eredmény |
|---|---|
| MIDI-kód a `main`-ben | Csak MIDI Timecode: `cfa4-tcsync-io` (L5), ALSA-szekvenszer, `alsa = "0.9"`, kliensnév `CFA4` |
| MIDI-vezérlők | A CONTROL-SURFACE-001 viszi (#1187 felderítés, #1193 monitor; Learn, leképezés később). Ez felel meg a terv 11.2 kockázatának („a MIDI-vezérlők külön ágon, saját tervben”) |
| Teendő a FINAL 1.1-ben | A `midi` adaptercsalád és a Mackie Control / HUI réteg a CONTROL-SURFACE-001-re kerül (illesztés E9) |
| Megkötés | Az `alsa-sys` `links = "alsa"` kulcsa miatt a munkaterületen egy ALSA-verzió lehet; minden új MIDI-kód az `alsa` 0.9-et használja |

A tulajdonostól várt MIDI-eszköztípus-lista (terv 12.1) a CONTROL-SURFACE-001 projekthez tartozik.

## 3. A crate-ek helye

| Terv (FINAL 1.1, 5.1) | Mai `main` / nyitott PR | Javaslat |
|---|---|---|
| `crates/surface-model` (L1) | A név foglalt: `cfa4-surface-model` (#1187), eseménymodell, `DeviceId`, `SurfaceInput`, `SurfaceFeedback` | Ezt használjuk; a katalógus, a státuszszámítás és a riportséma új L1 crate-be kerül: `cfa4-surface-catalog` (illesztés E1) |
| `crates/surfaces` (L5) | `cfa4-surface` (L5, #1187): hidraw- és ALSA-felderítés a `serve` folyamatban | Nem lesz második L5 eszközkezelő; a gyermekfolyamatos adaptereket a `cfa4-surface` felügyeli (E2) |
| `apps/cfa4-cli surfaces --adapter <család>` | Nincs | Marad, a hidapi- és tokio-alapú adapterek belépési pontja (E2, E5) |
| `python/cfa4_qt/surfaces/` (leképezés a GUI-ban) | A leképezés a motorban van (ADR 0012, 1. döntés) | A Qt csak felület és gombkép-renderelés (E3) |
| `cargo xtask layers` | `("surface-model", 1)`, `("surface", 5)` az #1187-ben | Új sor: `("surface-catalog", 1)` és a `SAME_LAYER` párja `("surface-catalog", "surface-model")` |

A Qt6 GUI és a `serve` már a `main`-ben van (`docs/qt-gui.md`), így a terv 9.1 „cfa4/qt6-gui ág
összefésülése” feltétele teljesült. A CONTROL-SURFACE-001 viszont a `serve`- és Qt-bekötést a
`qt-parity.md` 1.9, 1.10, 5.5 pontjához köti; az illesztés ugyanezt a kaput veszi át (E10).

## 4. Licencek és függőségek

A `cargo info` és a `cargo tree` 2026-11-13-i eredménye (crates.io):

| Crate | Verzió | Licenc | Megjegyzés |
|---|---|---|---|
| `elgato-streamdeck` | 0.14.0 | MPL-2.0 | Függőségei: `hidapi` (MIT), `image` 0.25 (MIT OR Apache-2.0; a `Cargo.lock`-ban már 0.25.10). Opcionális `async` feature: tokio |
| `hidapi` | 2.6.7 | MIT | Alapból `linux-static-hidraw`: a hidapi C-forrását fordítja be, `libudev` kell a fordításhoz. A `libudev-dev` minden CI-munkafolyamatban már telepítve van. Az `elgato-streamdeck` alapfeature-rel húzza be, így a C-rész nem kapcsolható ki |
| `mirajazz` | 0.16.3 | MPL-2.0 | Igazolva (a terv 12.1 nyitott tétele lezárva). Függőségei: `async-hid` 0.5.3 (MIT), `tokio` (MIT; a lockban már 1.54.1), `futures-lite`, `image` |
| `async-hid` | 0.5.3 | MIT | Linuxon saját hidraw-kezelés `nix`-szel (ioctl a függőségben) |
| `serialport` | 4.10.1 | MPL-2.0 | TourBoxhoz (CDC-ACM), M6 |
| `udev` | 0.9.3 | MIT | Hotplug-figyeléshez |

Minden licenc szerepel a `deny.toml` engedélyezett listáján (MPL-2.0, MIT, Apache-2.0, Zlib, BSD).
MPL-2.0 fájl módosítása esetén a módosított fájl MPL alatt marad és közzéteendő (terv 8.2).

**Never crash ellenőrzés a forrásban** (`unwrap`, `expect`, `panic!`, `unsafe`):

| Crate | Találat | Értékelés |
|---|---|---|
| `elgato-streamdeck` `src/lib.rs:386` | `lcd_strip_size().unwrap()` a `Kind::Plus` / `PlusXl` ágon | A típus garantálja (`Some` ezekre a fajtákra); kockázat alacsony |
| `mirajazz` `src/state.rs:90, 100` | `timeout.unwrap()`, `data.unwrap()` | Előtte `is_some` / `is_none` ellenőrzés; biztonságos |
| `mirajazz` `src/device.rs:139` | `query_devices(..).await.unwrap()` a hotplug-figyelőben | **Valódi pánikút** I/O-hibánál. A mirajazz `DeviceWatcher`-ét nem használjuk; a hotplugot a saját udev-figyelés adja |
| `hidapi` | `unsafe` FFI-blokkok (a C-könyvtár hívásai) | Függőségben van; a CFA4 `unsafe_code = "forbid"` szabálya a saját crate-ekre vonatkozik. Az izolációt a gyermekfolyamat adja (E2) |

A GPL-3.0 források (StreamController, OpenDeck, Aiacos/ajazz-control-center) csak referenciák
maradnak (terv 8.2, kiegészítés 2026-11-13).

## 5. Eszközlista és azonosítók

**Elgato** (`elgato-streamdeck` 0.14.0 `src/info.rs`, VID `0x0fd9`):

| Modell | PID | Állapot a katalógusban |
|---|---|---|
| Original | `0x0060` | adapterforrásból igazolt |
| Original V2 | `0x006d` | adapterforrásból igazolt |
| Mini | `0x0063` | adapterforrásból igazolt |
| Mini MK2 | `0x0090` | adapterforrásból igazolt |
| Mini Discord | `0x00b3` | adapterforrásból igazolt |
| XL | `0x006c` | adapterforrásból igazolt |
| XL V2 | `0x008f` | adapterforrásból igazolt |
| MK.2 | `0x0080` | adapterforrásból igazolt |
| MK.2 (scissor) | `0x00a5` | adapterforrásból igazolt |
| Neo | `0x009a` | adapterforrásból igazolt |
| Pedal | `0x0086` | adapterforrásból igazolt |
| + | `0x0084` | adapterforrásból igazolt |
| **+ XL** | `0x00c6` | **adapterforrásból igazolt** (a terv 12.1 nyitott tétele lezárva) |
| **Modules**: Mini MK2 Module, MK.2 Module, MK.2 Module (scissor), XL V2 Module | `0x00b8`, `0x00b9`, `0x00e4`, `0x00ba` | **adapterforrásból igazolt** (lezárva) |
| Studio | – | nincs a crate-ben: `identification = UNCONFIRMED`, PLANNED (terv 11.2 kockázat) |

**MiraBox-család:** a `mirajazz` nem tartalmaz VID/PID-táblát (a hívó adja meg), ezért a
modellek azonosítói `UNCONFIRMED`-ek maradnak, amíg valós eszközről vagy a hívó oldali
referenciából (`docs/protocols/streamdeck` mátrix a 2026-11-13-i kiegészítés szerint) be nem kerülnek.
SOOMFON-változatok és a Redragon SS-550: `UNCONFIRMED`.

**Kiegészítő eszközök** (Mackie Control, jog/shuttle, X-keys, SpaceMouse, lábkapcsoló): a jog/shuttle
(Contour ShuttlePRO, ShuttlePRO v2, ShuttleXpress) a CONTROL-SURFACE-001-ben van (#1187 felderítés).
A többi protokolladatát az adott fázis eszközkörében kell ellenőrizni; most `UNCONFIRMED`.

**Beszerzett vezérlő** (terv 12.1, a tulajdonos 2026-11-13-i válasza): **AJAZZ AKP153E**. A terv 1.3
pontja szerint mindig a beszerzett eszköz következik, ezért **az M2 első modellje az AKP153E** a
`mirabox-family` adapterrel (a 9. fejezet ábráján ez alapból M5 volt; az Elgato-modellek a beszerzésükig
EXPERIMENTAL / PLANNED maradnak). Az AKP153E VID/PID-jét, képméretét és forgatását az első csatlakoztatáskor
a varázsló azonosító lépése rögzíti (`lsusb`, `udevadm info`); addig `identification = UNCONFIRMED`. A
2026-11-13-i kiegészítés szabálya érvényes rá: eszközönként egy hosszan élő kapcsolat, az inicializálás
és a CRT DIS csak egyszer, a kimenet a „Kísérleti funkciók” mögött a riportig.

## 6. Rendszerszintű utak (terv 8.3)

Valós eszköz nélkül a `udevadm info`, a `/proc/bus/input/devices` és az `evtest` nem futtatható,
ezért az alábbi tábla **várt** értékeket ad forrásokból. A katalógus `system_path` mezője csak
eszközön végzett ellenőrzés után kap végleges értéket.

| Eszköz | Várt út | Forrás | `system_path` (javaslat, ellenőrizendő) |
|---|---|---|---|
| Elgato Stream Deck | csak hidraw, input-eseményt a kernel nem ad | `elgato-streamdeck` | `none` |
| MiraBox / AJAZZ | csak hidraw | `mirajazz`, Ajazz-értékelés | `none` |
| Contour ShuttlePRO / Xpress | kernel HID input (evdev) és hidraw | ADR 0012, HID-értékelés (#1184) | `kernel_hid_input` |
| TourBox | USB-soros (CDC-ACM); a tuxbox uinputon ad billentyűt | tuxbox | `userspace_uinput` |
| QMK / VIA billentyűzet, makrópad | kernel HID billentyűzet | QMK | `kernel_hid_input` |
| USB pedál, átirat-lábkapcsoló | modellfüggő (billentyűzet vagy vendor HID) | – | ellenőrizendő |
| 3Dconnexion SpaceMouse | spacenavd (felhasználói tér) | spacenavd | ellenőrizendő |

A mérés menete eszközönként, a referenciagépen
([`reference-machine.md`](../../development/reference-machine.md)):
`udevadm info -a /dev/hidrawN`, `cat /proc/bus/input/devices`, `sudo evtest` (csak olvasás), és a
tény, hogy a gyári vagy közösségi driver fut-e (a CFA4 ilyenkor nem nyitja meg ugyanazt az eszközt, 8.3).

## 7. Összefoglaló a kapuhoz

| Tétel | Állapot |
|---|---|
| CFA4-parancsok | kész: a parancsregiszter elég, új parancs csak a gombkép-kimenethez kell |
| MIDI-helyzet | kész: MTC a `main`-ben, MIDI-vezérlők a CONTROL-SURFACE-001-ben |
| Crate-ek helye | javaslat az illesztési mellékletben (E1–E3) |
| Licencek | kész: `mirajazz` MPL-2.0 igazolva; minden függőség megfelel a `deny.toml`-nak |
| Eszközlista | Elgato kész (+ XL, Modules igazolva; Studio nem); MiraBox, kiegészítők `UNCONFIRMED`; beszerzett eszköz: AJAZZ AKP153E → M2 első modellje |
| Rendszerszintű utak | várt értékek; eszközön ellenőrizendő |
| **Kapu** | **az illesztési melléklet tulajdonosi jóváhagyása** |
