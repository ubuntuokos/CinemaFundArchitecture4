# ADR 0012 – CFA4-CONTROL-SURFACE-001: Egységes vezérlőfelület-motor (Unified Control Surface Engine)

**Dátum:** 2026-11-13 · **Állapot:** elfogadva (a tulajdonos, `ubuntuokos`, 2026-11-13) ·
**Specifikáció:** CFA4-CONTROL-SURFACE-001 F0 v1.0; F0-dokumentumok: [műszaki specifikáció](../plans/CFA4-CONTROL-SURFACE-F0-TECHNICAL-SPEC.md), [átvételi jegyzőkönyv](../plans/CFA4-CONTROL-SURFACE-F0-ACCEPTANCE-REPORT.md) · **Alap:** `ubuntuokos/CinemaFundArchitecture4` main `92e29e4`, újraellenőrizve `5baa2fe`-n ·
**Repóbeli név:** `docs/adr/ADR-0012-control-surface.md` (a 0011-et a nyitott #1173 foglalja) ·
**Érintett rész:** új crate-ek (`cfa4-surface-model` L1, `cfa4-surface` L5), új modul (`crates/engine/src/surface/`, L4),
`apps/cfa4-cli` (`serve`), `python/cfa4_qt`. Ez az ADR kódot nem változtat.

## Háttér

A CFA4-nek nincs külső vezérlőeszköz-támogatása. A felhasználó a Contour ShuttlePRO/ShuttleXpress
jog-shuttle eszközt, később általános MIDI-vezérlőket (CC, Note, 14 bit, enkóderek) és motoros
felületeket akar használni. A Kdenlive (`src/jogshuttle/`), az Ardour és a Mixxx működése a minta;
a Kdenlive képességeit a tulajdonos döntése szerint (2026-11-02) a CFA4-be be kell építeni.

A meglévő keretek:

- A Qt6 GUI (PySide6, `python/cfa4_qt/`) a `cfa4-cli serve` gyermekfolyamattal beszél JSON-sorokon
  (`PROTOCOL = 1`, `{"id","method","params"}` → `{"id","ok","result"|"error"}`, események `{"event",…}`).
  A lejátszási órát és a hangot a Qt birtokolja (`panels/monitor.py`: `play(speed)`, `MAX_SPEED = 8.0`,
  `SHUTTLE_SOUND = (0.5, 4.0)`).
- Minden művelet parancs (`CommandSpec`, `Session::execute`); a menük és billentyűk a `command.list`-ből épülnek.
- A billentyűparancs-profilok kezelése kész mintát ad: `crates/engine/src/shortcuts.rs`
  (beépített presetek, `active.json`, undo/redo, szerkesztési munkamenet, audit, ütközések, `atomic_write`).
- Rétegszabályok (`cargo xtask layers`): csak lefelé mutató élek; UI/OS crate L5 alatt tilos;
  `unsafe_code = "forbid"` a `platform` (és az `obs` `libobs::sys`) kivételével.
- Az Event Fabric (CFA4-EVENT-FABRIC-001, E10) szerint a folyamaton belüli engine-események helyben maradnak.
- A külső időkód-szinkron (`docs/plans/TIMECODE-SYNC-LTC-MTC-2026-11-11.md`) már bevezette az első ALSA-szekvenszer
  klienst: `cfa4-tcsync-io` (L5, `alsa = "0.9"`, MTC be/ki saját porttal). Ez minta és egyben szomszédos projekt.

## Döntés

1. **Egyetlen igazságforrás a motor (L4).** Eszköz, MIDI, HID és később OSC nem hajt végre parancsot
   közvetlenül: minden bemenet `SurfaceEvent` → L4 leképezés → feloldott akció. A Qt-ban nincs
   MIDI- vagy HID-leképezési logika.
2. **Rétegek.**
   - **L1 `cfa4-surface-model`** (új crate, csak `serde`, wasm-tiszta): `SurfaceEvent`, `SurfaceInput`,
     `SurfaceFeedback`, `DeviceId`, MIDI-üzenet-parser, profilséma-típusok. Közös L4 és L5 számára,
     így az L5 nem függ az engine-től, és az L4 nem függ OS-kódtól.
   - **L4 `crates/engine/src/surface/`** (modul, nem új crate): leképezés, bank, skálázás, pickup,
     MIDI Learn állapotgép, profilkezelés a `shortcuts.rs` mintájára, `surface.*` parancsok, audit, feedback-előállítás.
   - **L5 `cfa4-surface`** (új crate): eszközfelderítés, hot-plug, nyers olvasás, időbélyeg, normalizálás,
     feedback-kódolás és -küldés, eszközönkénti szál és hibaizoláció. Linuxon valódi, máshol üres megvalósítás.
   - **L6 `cfa4-cli serve`**: az L5 sorainak átvétele, továbbítás az L4-nek, `surfaceAction` és a többi
     esemény kiírása, `surface.*` kiszolgálása. Eszköz-I/O nem kerül a `serve` főciklusába.
   - **Qt6**: beállítások, profilszerkesztő, Learn-megjelenítés, eszközlista, MIDI Monitor,
     `surfaceAction` végrehajtása a meglévő parancsúton (ugyanaz, mint a billentyűké), `surface.clock` küldése.
3. **HID:** elsődlegesen `hidraw` + sysfs (VID/PID, példányazonosító), tartalék a Linux `evdev`
   (a Kdenlive `media_ctrl` útja); libusb, hidapi és saját ioctl elutasítva. Indoklás és jegyzőkönyv:
   [`CFA4-CONTROL-SURFACE-HID-EVALUATION.md`](../plans/CFA4-CONTROL-SURFACE-HID-EVALUATION.md). A döntés a WP3 mérése után véglegesedik.
4. **MIDI:** ALSA Sequencer közvetlenül az `alsa` crate `seq` moduljával (a lockfile-ban már lévő
   `alsa 0.9.1`), hot-plug a System Announce (0:1) porton. `midir` nem kell.
5. **Idő:** minden belső idő `cfa4_time::Tick` (254 016 000 000/s), a monoton időbélyeg `u64` ns.
   A `surface.clock` tickben adja a lejátszófej helyét; f64 másodperc csak a kijelzésben.
6. **Sorok:** eszközönként két sáv: triggerek korlátos, eldobás nélküli sorban (torlódáskor az L5
   olvasószála vár és számlál), folyamatos értékek „utolsó érték” táblában (összevonás megengedett).
7. **Protokoll:** a `PROTOCOL = 1` additívan bővül: `hello.features: ["surface"]`, hibaválaszban opcionális
   `code`; új parancsok `surface.*`, új események `surfaceAction`, `surfaceLearn`, `surfaceDevices`,
   `surfaceMonitor` (≤ 20 Hz), `surfaceError`. A `PROTOCOL` szám nem nő.
8. **Profil:** saját JSON (`"format": "cfa4-surface-profile"`, `"version": 1`), tárolás
   `$XDG_DATA_HOME/cfa4/Control Surfaces/` (a `Keyboard Shortcuts` testvére), aktív fájl `active-<device>.json`,
   írás `cfa4_format::atomic_write`-tal. A billentyű- és a surface-profilok külön névtérben vannak.
9. **Kódátvétel:** Ardour és Mixxx csak viselkedési referencia. A Kdenlive `src/jogshuttle/`
   (GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL) a `licensing.md` szerint portolható (fájlfejléc-ellenőrzés,
   `ATTRIBUTION.md`, §5 nyilvántartás).
10. **OSC:** később, külön telepíthető modul ugyanazon a `SurfaceEvent`/`SurfaceFeedback` modellen; az
    alapcsomag nélküle teljes értékű.
11. **Event Fabric:** a surface-eszközesemények folyamaton belül maradnak (E10). A fabricre legfeljebb
    audit- és eszközállapot-esemény kerülhet, külön döntéssel.
12. **Ütemezés:** a fejlesztés (F1a-tól) a Qt6 GUI befejezése után indul (tulajdonosi döntés 2026-11-02);
    a kapu a `qt-parity.md` 1.9, 1.10, 4.2 és 5.2–5.5 pontja ✅ (2026-11-13-án nyitott: 1.9, 1.10, 5.5).
    Az egui-ba nem kerül átmeneti megoldás. **Módosítás (tulajdonos, 2026-11-13):** az F1a Qt-tól független Rust-része
    (`cfa4-surface-model` L1, `cfa4-surface` L5 hidraw-olvasó, tesztekkel) a kapu előtt elindulhat; a `serve`- és Qt-bekötés
    továbbra is a kapura vár.

## Következmények

- Két új crate kerül a `LAYERS` táblába: `("surface-model", 1)`, `("surface", 5)`; az `alsa`, `alsa-sys`,
  `evdev`, `udev`, `inotify` crate L5 alatt tiltott (a `UI_ONLY` mintájára).
- A `serve` egy belső `mpsc` csatornát kap az L5-től; a főciklus a kéréskezelés mellett ezt is üríti.
- A Qt-oldalon egy új `surfaceAction`-kezelő a meglévő `commands.py` útvonalat hívja; új Preferences-lap,
  profilszerkesztő és MIDI Monitor panel készül (F1/F2).
- Új dokumentumsorok: `architecture.md` (L5 `surface`), `control-protocol.md`/`qt-gui.md` (`surface.*`),
  `hardware-parity.md` (ShuttlePRO, MIDI), jog-shuttle sor a `CFA4-KDENLIVE-PLAN-001-FINAL-1.0.md`-ben (a Kdenlive-terv
  projektjének döntése, `coordination.md` §2.8), `qt-parity.md`
  (Control Surface GUI), `licensing.md`, `NOTICE`, szükség esetén `deny.toml`.
- A `cfa4-surface` és a `cfa4-tcsync-io` két külön szekvenszer-kliens ugyanabban a folyamatban; ugyanarra a hardverportra
  mindkettő feliratkozhat. A surface a Learnből és a leképezésből kiszűri az MTC-üzeneteket (0xF1 és a full-frame SysEx).
- A ShuttlePRO a Qt meglévő hangszabályát követi (`SHUTTLE_SOUND`, 0,5–4× előre hallható), külön
  hangmotor nem készül. Ez eltér a specifikáció „shuttle közben nincs hang” mondatától; a tulajdonos 2026-11-13-án így döntött (N-01), a profilban némítható.

## Elvetett lehetőségek

| Lehetőség | Miért nem |
|---|---|
| Leképezés a Qt-ban (Python) | Két igazságforrás lenne; az MCP és a CLI nem érné el. |
| `midir` | Nem ad hot-plug eseményt; a szükséges ALSA-réteg már a lockfile-ban van. |
| hidapi / libusb | C-függőség, a kernel HID-meghajtójának leválasztása kellene; a hidraw elég. |
| Átmeneti egui-host | Tulajdonosi döntés (2026-11-02): nincs. |
| Új `PROTOCOL = 2` | A bővítés additív, a régi kliens nem törik. |

## Revíziók

| Dátum | Változás |
|---|---|
| 2026-11-13 | Első változat (F0, WP1). |
| 2026-11-13 | Elfogadva; ADR-szám 0012; a `cfa4-tcsync-io` ALSA-kliens figyelembevétele; N-01 és N-12 eldöntve. |
