# CFA4-HW-CONTROL-SURFACES-001 FINAL 1.1 – CFA4-illesztési melléklet

**Dátum:** 2026-11-13 · **Állapot:** jóváhagyásra vár (M0 kapu) · **Terv:** CFA4-HW-CONTROL-SURFACES-001
FINAL 1.1 (<https://claude.ai/artifact/2YNfVud77rJovdg7C1RLvb>) · **Audit:**
[`CFA4-HW-CONTROL-SURFACES-001-M0-AUDIT-2026-11-13.md`](../audit/hw/CFA4-HW-CONTROL-SURFACES-001-M0-AUDIT-2026-11-13.md) ·
**Szomszédos projekt:** CFA4-CONTROL-SURFACE-001 ([ADR 0012](../adr/ADR-0012-control-surface.md), #1184, #1187, #1193)

Ez a melléklet **nem módosítja** a jóváhagyott tervet. Azt rögzíti, hol tér el a terv a `main`-től és
a CONTROL-SURFACE-001 projekttől, és hogyan illeszkedik hozzájuk. Ha a tulajdonos egy pontban dönt, a
döntés ide kerül, és szükség esetén a terv új verziót kap.

**Alapdöntés (tulajdonos, 2026-11-13): közös alapra építünk.** A hardvervezérlő-terv a CONTROL-SURFACE-001
eseménymodelljét, motorbeli leképezését, profilkezelését és Beállítások-lapját használja. Saját része a
katalógus és a támogatottsági modell, a kijelzős eszközök (Stream Deck, MiraBox-család, Loupedeck stb.)
adapterei és a hozzájuk tartozó lapok. Nem lesz két eszközkezelő a CFA4-ben.

## 1. Eltérések és illesztés

| # | Téma | FINAL 1.1 | CONTROL-SURFACE-001 / `main` | Illesztés (javaslat) |
|---|---|---|---|---|
| E1 | Modell-crate (5.1) | `crates/surface-model` (L1): katalógus, képességleírás, státuszszámítás, riportséma | `cfa4-surface-model` (L1, #1187): `SurfaceEvent`, `SurfaceInput`, `SurfaceFeedback`, `DeviceId` | Az eseménymodell az övék. A katalógus (TOML), a képességleírás, a 3.2 státuszszámítás és a 4.2 riportséma új L1 crate-be kerül: **`cfa4-surface-catalog`**, függ a `cfa4-surface-model`-től (`SAME_LAYER` sor). Így egyik projekt sem szerkeszti a másik fájljait |
| E2 | Folyamatmodell (5., 5.4) | adaptercsaládonként gyermekfolyamat + registry-folyamat, a Qt GUI felügyeli | eszköz-I/O a `serve` folyamatban, a `cfa4-surface` (L5) szálain | A tiszta Rust, kis adapterek (hidraw, ALSA, evdev) a `cfa4-surface`-ben maradnak. **Gyermekfolyamat** azoknak a családoknak, amelyek C-kódot (hidapi) vagy saját futtatókörnyezetet (tokio) hoznak: `elgato`, `mirabox-family`, később `loupedeck` és a többi. Belépési pont: `cfa4-cli surfaces --adapter <család>`, JSON-sorok stdin/stdout-on. **Felügyelő:** a `cfa4-surface` (a `serve`-ben), a `cfa4-studio-host` mintájára, nem a Qt GUI; a gyermek `SurfaceEvent`-et küld, és `SurfaceFeedback`-et kap. Külön registry-folyamat nem lesz: a felderítést a `cfa4-surface` udev/sysfs-figyelése adja, és ez indítja a családfolyamatot (5.4 utolsó pontja változatlan) |
| E3 | Leképezés (5.1 action router) | `python/cfa4_qt/surfaces/`: esemény → parancsazonosító a GUI-ban | a motorban (L4, `crates/engine/src/surface/`), `surfaceAction` a Qt parancsútján | A motoré. A Qt-oldal csak felület, gombkép-renderelés és a `surfaceAction` végrehajtása. Így az MCP és a CLI is eléri (ADR 0012, 1. döntés) |
| E4 | Kimenet: gombkép, LCD, fényerő (5.3) | `SetKeyImage`, `SetLcdRegion`, `SetBrightness`, `SetLed`, `Clear`, `Sleep`… | `FeedbackTarget`: `Led`, `Fader`, `Ring`, `Display`, `Timecode`, `Meter`, `DeviceState`; kép nincs | Additív bővítés a `cfa4-surface-model`-ben, a CONTROL-SURFACE-001 jóváhagyásával: `KeyImage {control}`, `LcdRegion {…}`, `Brightness`, és egy képhivatkozás az értékben. A képbájtok nem mennek JSON-ban: a `frame.render` megosztott memóriás gyűrűjének mintáját követik (`docs/qt-gui.md`), így az AC-05 ≤ 100 ms tartható. A képet a Qt rendereli (QPainter: ikon, szöveg, állapot) |
| E5 | HID-könyvtár | `rust-elgato-streamdeck` közvetlen függőség (hidapi) | ADR 0012: hidraw, a hidapi elutasítva | A hidapi **csak** az `elgato` gyermekfolyamatban van; a `serve` és az engine nem linkeli. A tiszta hidraw nem elég a Stream Deckhez (feature reportok ioctl-t kérnének, amit az ADR 0012 szintén elvet). A `mirabox-family` az `async-hid`-et használja (a `mirajazz` függősége), szintén gyermekfolyamatban. Ez megfelel az `AGENTS.md` „nem Rust komponens a folyamaton kívül” szabályának |
| E6 | Menüpont és lap (6.) | Beállítások → Hardver → Vezérlők, 10 lap | Settings ▸ Hardware & Devices ▸ Control Surfaces (#1187), magyarul „Vezérlőfelületek” | **Egy lap.** A tulajdonos 2026-10-31-i döntése szerint a magyar neve „Vezérlők” legyen (a `hu.tsv` egy sorának cseréje a CONTROL-SURFACE-001 PR-jában vagy utána). A FINAL 1.1 lapjai (Összes eszköz, Kompatibilitás, Diagnosztika, gombkiosztás-szerkesztő) ennek a lapnak az új fülei; az ő eszközlistájuk a „Csatlakoztatott eszközök” fül alapja |
| E7 | Stabil példányazonosító (7.3) | elsősorban sorozatszám; ha nincs vagy ismétlődik, USB-port útvonala „helyhez kötött” jelzéssel | `DeviceId::hid(vid, pid, phys)`: a fizikai útvonal | Additív bővítés: `hid:<vid>:<pid>:sn:<sha256-előtag>` alak, ha a sorozatszám egyedi; különben a mai `phys` alak. A profil a sorozatszámos azonosítóhoz kötődik, így áthelyezett eszköznél is megmarad. A CONTROL-SURFACE-001-gyel egyeztetendő (Q3) |
| E8 | Profilformátum (7.1) | profilséma a modell-crate-ben; munkaterület-kötés, makrók, tíz alapprofil (M4) | `cfa4-surface-profile` v1 JSON, `$XDG_DATA_HOME/cfa4/Control Surfaces/`, `context` mező a kötésben | Az ő formátumuk. A FINAL 1.1 igényei (munkaterület, makró) additív mezők a v1-ben vagy v2 migrációval, közös döntéssel. A 10 alapprofil beépített profil lesz az ő mechanizmusukkal |
| E9 | MIDI, Mackie Control, jog/shuttle (2., M3) | `midi` adapter + MCU-réteg, `generic-hid` jog/shuttle a FINAL 1.1 M3-ában | MIDI és Contour ShuttlePRO / Xpress a CONTROL-SURFACE-001-ben | Ezek a CONTROL-SURFACE-001-hez tartoznak. A FINAL 1.1 katalógusa felsorolja őket (AC-01), adapterként a CONTROL-SURFACE-001 azonosítóját jelöli, státuszukat az ő riportjaikból számolja. A Mackie Control / HUI réteg gazdáját a tulajdonos dönti el (Q4) |
| E10 | Indulási kapu (9.1) | M1 Qt-lapjai a `cfa4/qt6-gui` összefésülése után | a Qt GUI a `main`-ben van; a CONTROL-SURFACE-001 `serve`- és Qt-bekötése a `qt-parity.md` 1.9, 1.10, 5.5 pontjára vár | Ugyanez a kapu érvényes ránk is; a Qt-tól független Rust-rész (katalógus crate, adapterfolyamatok tesztekkel) előtte indulhat |
| E11 | Sorrend a PR-ok között | – | #1187 és #1193 nyitott | A `coordination.md` §2.1 szerint nyitott PR-ra nem építünk: a kódot érintő M1-PR-ok a #1187 beolvasztása után indulnak friss `main`-ről. Ez a dokumentum-PR független tőlük |
| E12 | M2 első modellje (9., 1.3) | Elgato-modellek az M2-ben, MiraBox-család az M5-ben | – | A beszerzett eszköz az **AJAZZ AKP153E** (tulajdonos, 2026-11-13), ezért az M2 első modellje a `mirabox-family` adapterrel készül; az 1.3 pont („mindig a beszerezhető eszköz következik”) ezt már megengedi. Az Elgato-adapter hardver nélkül EXPERIMENTAL marad (9.1) |
| E13 | Protokoll | gyermekfolyamat JSON-sorai | `PROTOCOL = 1` additív bővítése, `hello.features` | A Qt felé új képességjelző: `hello.features: ["surfaceAdapters"]`, új események és parancsok a `surface.*` névtérben (pl. `surface.catalog`, `surface.adapters`), az ő hibakódjaikkal (`permission_denied`, `device_offline`…) és a FINAL 1.1 hibakódjaival (`DEVICE_BUSY`, `FIRMWARE_UNKNOWN`, `PROTOCOL_MISMATCH`, `IO_TIMEOUT`) kisbetűs alakban |

## 2. Kérdések és döntések

| # | Kérdés | Javaslat | Döntés |
|---|---|---|---|
| Q0 | Két külön rendszer vagy közös alap? | Közös alap | **Közös alap** (tulajdonos, 2026-11-13) |
| Q1 | Ki felügyeli az adapter-gyermekfolyamatokat (E2)? | A `cfa4-surface` a `serve`-ben, a `cfa4-studio-host` mintájára; nincs külön registry-folyamat | |
| Q2 | Hova kerül a katalógus és a státuszszámítás (E1)? | Új L1 crate: `cfa4-surface-catalog` | |
| Q3 | Bővíthető-e a `DeviceId` sorozatszámos alakkal és a `SurfaceFeedback` képcélokkal (E4, E7)? | Igen, additívan, a CONTROL-SURFACE-001 jóváhagyásával | |
| Q4 | Ki viszi a Mackie Control / HUI réteget (E9)? | A CONTROL-SURFACE-001 (MIDI-alapú) | |
| Q5 | A lap magyar neve „Vezérlők” legyen „Vezérlőfelületek” helyett (E6)? | Igen, a 2026-10-31-i döntés szerint | |
| Q6 | Az M2 első modellje az AKP153E (E12)? | Igen | |
