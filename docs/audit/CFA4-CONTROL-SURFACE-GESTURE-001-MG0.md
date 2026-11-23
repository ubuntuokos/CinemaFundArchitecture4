# CFA4-CONTROL-SURFACE-GESTURE-001 — MG0 audit

**Dátum:** 2026-11-23 · **Csak olvasó audit.** Termelőkód, függőség, címke, merge és kiadás nem változott.
**Kódalap:** `ubuntuokos/CinemaFundArchitecture4` `main` `3cdd81bc5ad3d981295fdb1b0c9d05f1652cbddd` (2026-11-23). A `fájl:sor` hivatkozások erre vonatkoznak, ha nincs más megadva.
**Terv:** [`CFA4-CONTROL-SURFACE-GESTURE-001-1.1.md`](../plans/CFA4-CONTROL-SURFACE-GESTURE-001-1.1.md) a `docs/cfa4-control-surface-gesture-001-plan` ágon, commit `329ff31e5ec0325274ed521db9b69f8b3d2b9fd3`. Ez a commit a `main` `3cdd81bc` leszármazottja, és **nincs a `main`-en** (`git merge-base --is-ancestor` nem teljesül). A tervsorok erre a commitra vonatkoznak. Változáslista ugyanott: `CFA4-CONTROL-SURFACE-GESTURE-001-1.1-VALTOZASOK.md`.
**Engedély:** Horváth István 2026-11-23-án az MG0 indítását a `main` felé engedélyezte, a PostgreSQL-sorrendtől eltérve (a feladat szövege). Ugyanezen a napon 10:16-kor (Europe/Budapest) a 12. fejezet javaslatait elfogadta („javaslataid elfogadva jóváhagyom”). A 10. kérdés érvényes döntése 10:54 (12.1): az MG1 nem várja meg a Qt6 nyitott sorait, a fizikai mérést, az OSC-t és a Mackie/HUI-t. A terv 21. fejezete szerint az implementáció ettől még nem engedélyezett (`…-1.1.md` 19. és 1042–1043. sor). Az MG1 a tulajdonos külön igenjével indul. Ez a dokumentum nem indítja.

A terv „[MG0-ban igazolandó]” jelölése (`…-1.1.md` 27. sor) azt kéri, hogy a forrásból nem ellenőrzött állítást ez az audit döntse el. Ahol élő KDE, fizikai egér vagy GitHub-futó kellett volna, a sor **nem mért**.

## 0. Belépési feltételek

A terv 21. fejezete öt feltételt ír az MG0 elé (`…-1.1.md` 679–685. sor). Állapotuk a `main` `3cdd81bc`-n:

| # | Feltétel a tervben | Ami a repóban van | Következmény |
|---|---|---|---|
| 1 | A PostgreSQL-munka lezárult; a fejlesztés csak utána indul | A döntés szövege a repóban nincs (a változáslista „Amit nem igazoltam” listája). Ezt az auditot a tulajdonos ettől eltérve indította | Az MG0 dokumentum indulhat. A PG-munka (#1629 és társai) ehhez a PR-hez nem tartozik, és nem módosult |
| 2 | `qt-parity.md` 1.9, 1.10 és 5.5 ✅ | Mindhárom **🟡**: `docs/qt-parity.md` 30., 31. és 111. sor. Az ADR 0012 12. döntése ugyanezt a kaput nevezi (`docs/adr/ADR-0012-control-surface.md` 70–71. sor), a 2026-11-13-i módosítással: a Qt-tól független Rust-rész előbb indulhat (72–74. sor) | A Qt-bekötés kapuja nyitott. Az MG0 auditra a tulajdonos külön engedélyt adott |
| 3 | A szülő fizikai igazolása (M1–M10) kitöltve | A jegyzőkönyv üres: `docs/audit/hw/CFA4-CONTROL-SURFACE-M1-M10.md` 3. sor („Nothing in this table was measured”). A `docs/hardware-parity.md` 73. sora ugyanezt írja | Nincs fizikai ShuttlePRO-mérés |
| 4 | OSC és Mackie/HUI stabil, vagy István felmenti a gesztust | OSC: a `SurfaceProtocol::Osc` és a `DeviceId::net` létezik (`crates/surface-model/src/lib.rs` 37–38., 64. és 133–135. sor). A `crates/surface` alatt `osc` találat nincs. A `serve` eszközjelentése csak `Hid` vagy `Midi` (`apps/cfa4-cli/src/serve_surface.rs` 513. sor). Mackie/HUI: `docs/hardware-parity.md` 72–73. sor, „not in the first control-surface release” | Nincs kész |
| 5 | A gesztus nem része az első kiadásnak | A terv 28. fejezete ugyanezt javasolja (`…-1.1.md` 999. sor). Az első kiadás feltétellistája ebben a repóban külön dokumentumként nem szerepel; a terv a tulajdonos mentett memóriájára hivatkozik (változáslista V-21) | Ütemezési kérdés, lásd a nyitott kérdéseket |

## 1. A meglévő vezérlőfelület

Az ADR 0012 egy `crates/engine/src/surface/` modult ír (`docs/adr/ADR-0012-control-surface.md` 6. és 40. sor). Ilyen könyvtár nincs. A motor lapos fájlokban van, `crates/engine/src/lib.rs` 23. és 85–87. sora:

| Szerep | Fájl | Ami ténylegesen ott van |
|---|---|---|
| Leképezés | `crates/engine/src/control_surfaces.rs` | `DeviceSetting`, `Binding`, `Triggers`, `ControlSurfacePrefs::action` / `motion` |
| Learn, inject, óra | `crates/engine/src/surface_runtime.rs` | egy Learn-gép; `surface.inject` ugyanazon az úton, mint az eszköz (`3–4`. sor) |
| Profilfájl | `crates/engine/src/surface_profile.rs` | `cfa4-surface-profile` v1, `$XDG_DATA_HOME/cfa4/Control Surfaces/` (`1–22`. sor) |
| Visszajelzés | `crates/engine/src/surface_feedback.rs` | külön modul, nem a `surface/` könyvtár |
| Eseménymodell (L1) | `crates/surface-model` | `("surface-model", 1)` a `xtask/src/main.rs` 125. sorában |
| Eszköz-I/O (L5) | `crates/surface` | `("surface", 5)` a `xtask/src/main.rs` 135. sorában |
| Katalógus (L1) | `crates/surface-catalog` | `("surface-catalog", 1)` a `xtask/src/main.rs` 133. sorában; a gesztus nem használja |
| Kiszolgáló | `apps/cfa4-cli/src/serve.rs` 28. sor | `PROTOCOL = 1`. Az ADR 7. döntése: a szám nem nő (`ADR-0012` 57–59. sor) |
| Parancsregiszter | `crates/engine/src/commands.rs` 23–36. sor | `CommandSpec`: `id`, `label`, `menu`, `shortcut`, `params`, `enabled`, `run`, `journal`. Destruktív jelölő nincs |
| Végrehajtás | `crates/engine/src/lib.rs` 834–836. sor | `Session::execute` azonosító szerint keres |

A Qt a `surfaceAction` eseményt a `MainWindow.run_surface_action` kezeli (`python/cfa4_qt/main_window.py` 2027–2028. és 2343–2346. sor): a parancs úgy fut, ahogy a billentyűje futna. A keverőcsík és a shuttle külön ág (2348–2364. sor).

### 1.1 Leképezés

`Binding` három mező: `control`, `command`, `params` (`control_surfaces.rs` 181–190. sor). Egy eszközön vezérlőnként egy kötés (`35`. sor: „one per control”). `action` a kötést adja vissza, ha a felületek és az eszköz engedélyezve van (`303–311`. sor). `clamp` a későbbi, azonos vezérlőre mutató kötést eldobja, és eszközönként `MAX_BINDINGS = 512` kötésnél vág (`178–179`., `416–424`. sor).

A séma `context` mezője a kötésobjektumban van, alapértéke `"global"` (`docs/plans/CFA4-CONTROL-SURFACE-PROFILE-SCHEMA.json` 558–561. sor). A Rust `Binding` ezt a mezőt nem olvassa. A profilrétegek sorrendje a F0 spec 8. fejezete: beépített, majd felhasználói, majd projektprofil, vezérlőnkénti felülírással (`docs/plans/CFA4-CONTROL-SURFACE-F0-TECHNICAL-SPEC.md` 181–188. sor). N-06 ezt 2026-11-16-án lezárta (a spec 243. sora).

A séma gyökerén `additionalProperties: false` áll (séma 16. sor), `version` minimuma 1 (21–24. sor). A `surface_profile.rs` 7. sora: magasabb `version` érintetlen marad.

**A két tengely együttes szabálya (javaslat, István hagyja jóvá):**

1. Előbb a meglévő profilréteg, vezérlőnként (N-06): a projekt kötése felülírja a felhasználóit, az a beépítettet, csak azon a vezérlőn, amelyet maga is leképez.
2. Az így kapott kötések közül a legszűkebb illeszkedő kontextus nyer: widget, különben modul, különben `global` (a terv 8.2 sorrendje, `…-1.1.md` 368–373. sor).
3. Ha a nyertes kontextusnak nincs kötése, a parancs nem indul. A tágabb kontextus nem lép a helyére, mert a 8.2 szerint a felhasználó konfigurációja nem írható felül váratlanul (`…-1.1.md` 377. sor). Ez a terv 4. lépését („No action”) úgy olvassa, hogy a szűkebb, üres kontextus szándékos csend, nem lyuk.

A `Binding`-ben ma nincs `context` mező, ezért ez a szabály csak a modellbővítés után érvényesíthető. Addig minden kötés globális.

### 1.2 Dispatch

`Triggers::press` (`control_surfaces.rs` 222–248. sor) egyetlen emelkedő élre ad vezérlőt:

- `SurfaceInput::Button`: csak `pressed == true`, és csak ha az a kulcs még nem volt a `down` halmazban.
- `SurfaceInput::Absolute`: a fél skála fölé lépés számít lenyomásnak.
- Minden más változat (`Relative`, `Jog`, `Shuttle`, `Touch`, `ProgramChange`, `PitchBend`, `SysEx`, `Raw`) `None`.

A felengedés kiveszi a kulcsot, és nem ad parancsot (244–246. sor). Hold-időzítő nincs. A `surface_runtime.rs` `emit_target` függvénye (317–319. sor) ugyanezt a `press`-t hívja, mielőtt `surfaceAction` eseményt írna (`372–374`. sor).

A jog és a shuttle nem a `Binding`-en megy: `motion` a profil táblájából `playback.shuttleSpeed` vagy képkockánkénti léptetés (`control_surfaces.rs` 364–393. sor). A sebességtáblák fix hetes tömbök: `SHUTTLE_ARDOUR` és `SHUTTLE_KDENLIVE` (195–201. sor).

`surface.inject` (`surface_runtime.rs` 556–566. sor) a JSON `input` mezőt `SurfaceInput`-tá parse-olja, és a `handle` útra teszi. A MIDI-időzítő bájtokat a `cmd_inject` előbb kiszűri (720–727. sor). Új `SurfaceInput`-változat, amelyet a serde `type` címkéje hordoz (191. sor), a régi `Button` JSON-t nem töri el. Ismeretlen kötés JSON-ja `unsupported`-ba kerül, letiltva, és visszaíródik (`control_surfaces.rs` 20–26. és 140–148. sor).

### 1.3 `SurfaceProtocol` és a GESTURE bővítési pont

`SurfaceProtocol` három változat: `Hid`, `Midi`, `Osc` (`crates/surface-model/src/lib.rs` 61–65. sor). A JSON-nevek: `hid`, `midi`, `osc` (70–74. sor).

`ControlId`: `Midi`, `Hid { name, usage_page, usage }`, `Evdev { ev_type, code }` (mindkettő `u16`), `Osc { path }` (115–136. sor). Az evdev-azonosító már nem fix gombszám.

`SurfaceInput` (`191–237`. sor): `Button { control, pressed }`, `Absolute`, `Relative`, `Jog`, `Shuttle`, `Touch`, `ProgramChange`, `PitchBend`, `SysEx`, `Raw`. Gesztusváltozat nincs. A `Button` csak lenyomást és felengedést ismer.

**MG0 döntés a névről (javaslat):** a `GESTURE` nem új protokoll. A felismert gesztus új `SurfaceInput`-változat. A Qt-ból érkező mutató, amely Waylanden nem eszköz (lásd 4. fejezet), új protokollváltozat: `Pointer`. Az evdev-út a meglévő `Hid` + `ControlId::Evdev` marad. A pontos típusok a 9. fejezetben vannak. István hagyja jóvá, mielőtt a modellcrate változik; az a CFA4-CONTROL-SURFACE-001 közös modellje (a HW-illesztés Q3 mintája: `docs/plans/CFA4-HW-CONTROL-SURFACES-001-FINAL-1.1-CFA4-ILLESZTES.md` 42. sor).

### 1.4 Event Fabric

Az ADR 11. döntése: a surface-események a folyamatban maradnak; a fabricre legfeljebb audit- vagy eszközállapot-esemény kerülhet, külön döntéssel (`ADR-0012` 68–69. sor). Az Event Fabric illesztés E10 pontja a motor belső eseményeit a folyamatban hagyja (`docs/plans/CFA4-EVENT-FABRIC-001-1.0-CFA4-ILLESZTES.md` 24. sor). A gesztus ugyanide tartozik (GDR-22). Ehhez a fabric kódjához ez az audit nem nyúl.

## 2. Hold és kombináció: hatás a MIDI- és HID-kötésekre

A terv V-20 pontja (`…-1.1.md` 289. sor) a közös modell bővítését kéri: kombináció, Hold, felengedés-trigger, módosító. A mai viselkedés, amelyet egy közös bővítés elronthat:

| Mai szabály | Hely | Ha a Hold vagy a kombinációs ablak alapértelmezés lenne |
|---|---|---|
| Egy vezérlő, egy parancs, a lenyomás élén | `Binding` 185–190; `Triggers::press` 232–247 | Minden MIDI-gomb és ShuttlePRO-gomb késne az ablak végéig, vagy a felengedésig |
| A fél skála fölé tolt fader is „lenyomás” | `press` 235–237 | Ugyanez a késleltetés a CC-gombokra |
| A felengedés nem indít parancsot | `press` 244–246 | Egy új, alapértelmezett release-trigger dupla parancsot adna |
| A jog és a shuttle a profiltáblán megy, nem a `Binding`-en | `motion` 372–393 | Ezeket a kombinációs feloldó nem nyelheti el |
| Az ismeretlen kötés megmarad, letiltva | `unsupported` 23–26., 145–148 | Új mező, amelyet a régi olvasó nem ért, ide kerül, és nem fut. Ez a kívánt irány |
| Egyszerre egy Learn | `surface_runtime.rs` 4. sor; F0 spec 177. sor | Második Learn-gép a terv 5.4 szerint tilos (`…-1.1.md` 240. sor) |

**Javasolt határ, hogy a MIDI és a HID ne változzon:**

- Az új mezők serde-alapértéke a mai viselkedés: trigger = lenyomás éle, kombináció nincs, Hold nincs, módosító nincs.
- A `Triggers::press` a régi hívóknak (MIDI Learn, HID-gomb, `emit_target`) továbbra is az emelkedő élen ad vezérlőt. A Hold és a kombináció külön függvény, amelyet csak a gesztusút hív.
- A jog, a shuttle és a `VALUE_COMMANDS` (`mixer.stripVolume`, `mixer.stripPan`, `control_surfaces.rs` 206. sor) kimarad a kombinációs feloldóból.
- A bővítés a `cfa4-surface-model` és az L4 közös típusait érinti, ezért a CFA4-CONTROL-SURFACE-001 jóváhagyása kell (V-20). Az MG1 felismerője e nélkül is megírható: a gesztus saját típusokon számol, és a `Binding`-et nem írja. A Learn-gépbe csak a jóváhagyás után kerül mód.

A kezesség a ShuttlePRO-ra és a MIDI-re nem terjed ki: a gombokat sorszám (`button1`…) és `ControlId` szerint köti a kód (`hid_in.rs` 61. és 145. sor; `Binding.control`). A terv 8.4 8. pontja ezt így hagyja (`…-1.1.md` 412. sor).

## 3. ShuttlePRO és a CS-8 evdev

A CS-8 a 2026-11-20-i megvalósítás: hidraw elsődleges, evdev tartalék (`docs/plans/CFA4-CONTROL-SURFACE-HID-EVALUATION.md` 20–26. sor; `docs/hardware-parity.md` 96. sor). Grab nincs (a HID-értékelés 26. sora). Az ADR a hidapi-t, a libusb-t és a saját ioctl-t elutasítja (`ADR-0012` 48–49. és 98. sor).

`crates/surface/Cargo.toml` 12–19. sor: függőség `cfa4-surface-model`, `serde`, `alsa` 0.9, `rustix` 1 (`event`, `fs`). Az `evdev` és az `udev` crate nincs behúzva.

`hid.rs` `KNOWN` tábla (15–20. sor), csak Contour `0x0b33`:

| PID | Név | Gombok (`u8`) |
|---|---|---|
| `0x0010` | Contour ShuttlePRO | 13 |
| `0x0020` | Contour ShuttleXpress | 5 |
| `0x0030` | Contour ShuttlePRO v2 | 15 |

`ShuttleDecoder::new` és `EvdevDecoder::new` a gombszámot `buttons.min(16)`-ra vágja (`hid_in.rs` 44–46. és 111–113. sor), mert a maszk `u16` (`39`., `54`., `104`. sor). Az evdev-dekóder csak a `BTN_MISC + n` tartományt fogadja (`0x100`, `hid_in.rs` 89–90. és 128–129. sor), és minden más `EV_KEY` kódot eldob (134. sor, `_ => {}`). A `BTN_LEFT` (`0x110`) ebbe a tartományba nem esik. Egérhez ez a dekóder nem használható; a terv 5.3 szerint új, általános dekóder kell (`…-1.1.md` 220. sor). Az új dekóder is L5, `unsafe` nélkül, a `rustix` polljával, új crate nélkül, amíg ioctl nem kell.

Beépített profilok: `cfa4.shuttlepro` (alap) és `cfa4.shuttlepro.kdenlive`, mindkettő `hid-shuttle` (`control_surfaces.rs` 457–459. sor). A Qt felirat: a jog és a shuttle a profilt követi (`python/cfa4_qt/panels/settings.py` 1201–1204. sor).

## 4. Útvonalankénti gombhatárok

A CFA4 saját forrása nem vezet be 3, 5 vagy 8 gombos menü-enumot az egérre. A határt az út adja. Ellenőrizve 2026-11-23-án, a letöltött fájlokon (nem a referenciagép futó folyamatán).

### A) Qt, Wayland

Qt 6.10.2 `src/plugins/platforms/wayland/qwaylandinputdevice.cpp` 803–822. sor (tag `v6.10.2`, <https://github.com/qt/qtbase/blob/v6.10.2/src/plugins/platforms/wayland/qwaylandinputdevice.cpp>):

- A komment: az egérkódok `0x110 <= gomb < 0x120`.
- A `switch` a `0x110`…`0x11f` kódokat képezi le: `Left`, `Right`, `Middle`, majd `ExtraButton1`…`ExtraButton13`. Ez **16** kód.
- `default: return;` — minden más kód eldobva, Qt-esemény nem készül.

A `0x113` a Qt-ban `ExtraButton1`, amelynek aliasa `BackButton`; a `0x114` az `ExtraButton2` / `ForwardButton` (ugyanaz a fájl, 809–810. sor). A kernel nevei mások: `BTN_SIDE` `0x113`, `BTN_EXTRA` `0x114`, `BTN_FORWARD` `0x115`, `BTN_BACK` `0x116` (Linux `input-event-codes.h` 355–362. sor, tag `v6.12`, <https://github.com/torvalds/linux/blob/v6.12/include/uapi/linux/input-event-codes.h>). A Qt „Back/Forward” felirata a Qt-alias, nem a kernel `BTN_BACK` / `BTN_FORWARD` kódja.

Eszközazonosító nincs a protokoll eseményben. A `wl_seat` leírása: „A seat is a group of keyboards, pointer and touch devices” (`protocol/wayland.xml` 2004–2006. sor, Wayland `main`, <https://gitlab.freedesktop.org/wayland/wayland/-/blob/main/protocol/wayland.xml>). A `wl_pointer.button` argumentumai: `serial`, `time`, `button`, `state` (2249–2269. sor). A `button` Linux-kód, eszközmező nélkül. A referenciagép grafikai platformja Wayland (`docs/development/reference-machine.md` 17. sor).

### B) Qt, X11

Qt 6.10.2 `qxcbconnection.cpp` `translateMouseButton` (482–514. sor, <https://github.com/qt/qtbase/blob/v6.10.2/src/plugins/platforms/xcb/qxcbconnection.cpp>):

- X-gomb 1, 2, 3 → bal, középső, jobb.
- A 4–7 „already handled as Wheel events, and won't occur here” (488. sor).
- A 8–31 → `ExtraButton1`…`ExtraButton24`.
- `default` → `NoButton`.

Ez 3 + 24 = **27** gomb. A `Qt::MouseButton` enum ugyanezen a tag-en `ExtraButton24 = 0x04000000` és `MaxMouseButton = ExtraButton24` (`qnamespace.h` 62–97. sor, <https://github.com/qt/qtbase/blob/v6.10.2/src/corelib/global/qnamespace.h>). `AllButtons = 0x07ffffff`. A komment (98. sor) négy magas bitet szabadon hagy (`0x08000000`…`0x40000000`); ezek nem nevesített gombok.

### C) evdev

`KEY_MAX` = `0x2ff`, `KEY_CNT` = `KEY_MAX + 1` (`input-event-codes.h` 811–812. sor, Linux `v6.12`). Az `EV_KEY` kódok `0`…`0x2ff`. A `BTN_MOUSE` `0x110`, a `BTN_JOYSTICK` `0x120` (355. és 365. sor). A `ControlId::Evdev` `u16` kódot bír (`surface-model` `lib.rs` 129–132. sor), tehát a `KEY_MAX` belefér. A CFA4 mai dekódere ezt a tartományt nem olvassa (3. fejezet).

A `BTN_TRIGGER_HAPPY40` `0x2e7` (ugyanabban a fejlécben, 807. sor) a `KEY_MAX` alatt van: egy joydev-jellegű kód egér-node-on is megjöhet, és a C út csak akkor látja, ha az általános dekóder minden `EV_KEY` kódot továbbad, nem csak a `0x110`…`0x11f` sávot.

### D) Gyártói protokoll

A terv az első kiadásból kiveszi (`…-1.1.md` 169. és 933. sor). Ezt a repó nem valósítja meg. Nem mérve.

### Fix gombszám a CFA4 kódjában

| Hely | Korlát | Egérre vonatkozik? |
|---|---|---|
| `hid.rs` 19–20 | Három Contour-PID, gomb `u8` (5, 13, 15) | Nem, csak ShuttlePRO / Xpress |
| `hid_in.rs` 46. és 113 | `buttons.min(16)` és `u16` maszk | Nem, ugyanaz a dekóder |
| `hid_in.rs` 128–129 | Csak `BTN_MISC`…`BTN_MISC + buttons` | Az egeret eldobja |
| `control_surfaces.rs` 195–201 | Shuttle-sebesség `[f64; 7]` | Nem, a gyűrű ±7 állása |
| `control_surfaces.rs` 179 | `MAX_BINDINGS = 512` | Kötésszám, nem gombszám |
| `python/cfa4_qt/control.py` 82. és 640–642 | `BUTTONS`: `left`, `right`, `middle`; más érték `ControlError` | Igen: a vezérlőcsatorna `ui.click` / `ui.drag` csak ezt a hármat küldi. Az extra gomb CI-ből így nem nyomható meg |

`QGesture` / `grabGesture` a `python/cfa4_qt` alatt nincs (keresés 2026-11-23, `main` `3cdd81bc`). A felismerés a Qt `QGestureRecognizer` osztályára nem épülhet: azzal a logika a Pythonban futna, az ADR 1. döntése ellen (`ADR-0012` 33–35. sor).

## 5. Qt6 / PySide6 bemenet

A szerkesztő a `python/cfa4_qt` csomag. Alkalmazásszintű egérszűrő, amely minden panel elé állna, nincs. A gombokat a panelek `mousePressEvent` és `eventFilter` kezelői kapják.

Jobb gomb, idővonal, **lenyomáskor** (`python/cfa4_qt/panels/timeline.py` 2368–2379. sor): ha van szekvencia és a gomb `RightButton`, a vágás vagy a `context_menu` azonnal megnyílik, és a függvény visszatér. Felengedés-küszöb nincs. A Qt dokumentációja szerint a `LeftButton` balkezes egéren a jobb fizikai gomb lehet (`qnamespace.qdoc` 299–301. sor, Qt 6.10.2, <https://github.com/qt/qtbase/blob/v6.10.2/src/corelib/global/qnamespace.qdoc>): a `RightButton` tehát a logikai másodlagos gomb, miután a kompozitor cserélt.

Középső gomb:

- Programmonitor: pásztáz, ha a gomb `MiddleButton`, vagy ha a kéz eszköz aktív és a gomb bal (`panels/monitor.py` 438–443. sor).
- Idővonal fülei: `MouseButtonRelease` + `MiddleButton` bezárja a szekvenciát (`panels/timeline.py` 3379–3390. sor).

Helyi menü a `python/cfa4_qt` alatt (2026-11-23, `contextMenuEvent` vagy `customContextMenuRequested`):

| Fájl | Sorok |
|---|---|
| `main_window.py` | 1215 |
| `panels/timeline.py` | 3210 (a vágófelület saját menüje a 2378. soron, `context_menu`, nem ez a minta) |
| `panels/tools.py` | 170 |
| `panels/project.py` | 973, 1155, 1163, 1189 |
| `panels/readouts.py` | 185 |
| `panels/scopes.py` | 240 |
| `panels/media_browser.py` | 390, 405, 407, 420 |
| `panels/effect_controls.py` | 294, 843 |
| `panels/effects.py` | 196, 206 |

A `python/cfa4_qt` alatt `RightToLeft`, `setLayoutDirection` és `isRightToLeft` találat nincs. A szövegtördelő külön, a `crates/text` `Direction::RightToLeft` kapcsolója (`crates/text/src/layout.rs` 338. sor); ez nem a Qt-ablak iránya.

A Kdenlive-terv K53 pontja középső gombos görgetést ír az idővonalra (`docs/plans/CFA4-KDENLIVE-PLAN-001-FINAL-1.0.md` 150. és 595. sor). Az a terv másik projekt. Ütközésként feljegyezve, kódja ehhez az audithoz nem készül.

## 6. Evdev jogosultság és biztonság

A csomagolt szabály `packaging/linux/70-cfa4-surfaces.rules` 1–5. sor:

```text
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="0b33", TAG+="uaccess"
SUBSYSTEM=="input", ATTRS{idVendor}=="0b33", TAG+="uaccess"
```

A komment: nem `MODE="0666"`. A F0 spec 6. fejezete ugyanezt írja (`CFA4-CONTROL-SURFACE-F0-TECHNICAL-SPEC.md` 157–159. sor). Telepítés: `packaging/linux/package.sh` 164. sor és `packaging/linux/nfpm.yaml` 109–110. sor, cél `/usr/lib/udev/rules.d/70-cfa4-surfaces.rules`. Más gyártót a szabály nem enged.

A systemd `rules.d/70-uaccess.rules.in` `main` ága, letöltve 2026-11-23-án (<https://github.com/systemd/systemd/blob/main/rules.d/70-uaccess.rules.in>): a 76. sor csak `SUBSYSTEM=="input", ENV{ID_INPUT_JOYSTICK}=="?*"` esetén ad `uaccess`-t. `ID_INPUT_MOUSE` és `ID_INPUT_KEYBOARD` ezen a listán nincs. A `hidraw` sorok (102., 110., 115., 119.) külön címkékhez kötöttek (AV-vezérlő, fény, hardvertárca, 3D-egér), általános egérhez nem.

Az `input` csoport minden `/dev/input/event*` csomópontot olvashatóvá tenne, amely `root:input` `0660`. Ez billentyűzetet is jelent. A CFA4 szabálya ezt nem kéri, és a terv GDR-23 szerint ne is kérje.

`EVIOCGRAB` a CFA4-ben nincs (HID-értékelés 26. sor). Grab nélkül ugyanaz a ShuttlePRO-esemény a desktophoz is eljut (HID-értékelés 52–53. sor, „mérendő”). Egérnél ugyanez a kettőzés: a Qt-út és az evdev-út egyszerre látja a gombot. A terv V-11 szerint eszközönként egy forrás lehet (`…-1.1.md` 11. tétel a változáslistában).

Flatpak: mindkét manifest `finish-args` listája `--share=ipc`, `--socket=wayland`, `--socket=fallback-x11`, `--socket=pulseaudio`, `--device=dri` és fájlrendszer-engedélyek (`packaging/linux/flatpak/io.github.ubuntuokos.cfa4.bundle.yml` 20–39. sor; a másik manifest 27. sorától ugyanígy). `--device=input` nincs. A Flatpak `doc/flatpak-run.xml` 400–405. sora (`main`, <https://github.com/flatpak/flatpak/blob/main/doc/flatpak-run.xml>): a `DEVICE` értéke `dri`, `input`, `usb`, `kvm`, `shm` vagy `all`. Az evdev-út a homokozóban ezt kérné. A Wayland-socket a kompozitor mutatóját adja, a `/dev/input` csomópontot nem.

**Javaslat (GDR-05, GDR-23, a terv 24. fejezete):** az elsődleges út a Qt alkalmazásszint (A, a referenciagépen Wayland). Az evdev (C) alapból ki van kapcsolva, eszközönként kapcsolható, csak a kiválasztott node-ra, és csak akkor indít parancsot, ha a Qt aktív ablakot jelez. Billentyű-node csak külön, figyelmeztetett engedéllyel. `uinput` nincs, grab nincs. A nyers esemény nem kerül fájlba vagy naplóba.

## 7. Kezesség és középsőgomb-emuláció

### 7.1 libinput balkezes mód

`libinput.h` a `main` ágon, letöltve 2026-11-23-án (<https://gitlab.freedesktop.org/libinput/libinput/-/blob/main/src/libinput.h>) 6088–6098. sor: egéren a bal és a jobb gomb cserélődik, a középső nem; a változás csak akkor lép életbe, ha minden gomb logikailag felengedett.

`src/evdev.h` 661–670. sor (<https://gitlab.freedesktop.org/libinput/libinput/-/blob/main/src/evdev.h>): `evdev_to_left_handed` csak `EVDEV_BTN_LEFT` és `EVDEV_BTN_RIGHT` között cserél. Oldalsó gombot nem.

GNOME-séma, `master` (<https://gitlab.gnome.org/GNOME/gsettings-desktop-schemas/-/blob/master/schemas/org.gnome.desktop.peripherals.gschema.xml.in>): `org.gnome.desktop.peripherals.mouse` `left-handed`, típus `b`, alapérték `false` (112–116. sor). Az érintőpad `left-handed` alapértéke `'mouse'` (53–56. sor). A Mutter forrását ez az audit nem olvasta; a séma szövege („Swap left and right mouse buttons”) igazolt, a megvalósítás nem.

### 7.2 KWin `LeftHanded`

A KWin `master` forrása, letöltve 2026-11-23-án. A `v6.6.7` tag nyers URL-je 404 volt, ezért **nem** a Plasma 6.6.7 fából származik.

- `src/main.cpp` 112. sor: a konfiguráció `kcminputrc` (<https://invent.kde.org/plasma/kwin/-/blob/master/src/main.cpp>).
- `src/backends/libinput/connection.cpp` 735. sor: eszközönkénti csoport `Libinput` / gyártó / termék / név (<https://invent.kde.org/plasma/kwin/-/blob/master/src/backends/libinput/connection.cpp>).
- `src/backends/libinput/device.cpp` 248. és 736. sor: `LeftHanded` kulcs, a libinput `left_handed_set` hívása (<https://invent.kde.org/plasma/kwin/-/blob/master/src/backends/libinput/device.cpp>).
- Ugyanaz a fájl, 421–426. sor: a támogatott mutató-gombok vizsgálata `BTN_LEFT` … `0x120` alatt, vagyis a 16-os kernel-sáv.
- `src/mousebuttons.cpp` 14–47. sor (<https://invent.kde.org/plasma/kwin/-/blob/master/src/mousebuttons.cpp>): a `0x110`…`0x11f` kódok `Left`/`Right`/`Middle`/`ExtraButton1`…`13` értékre kerülnek. Minden más kód `ExtraButton24`. A komment (41–46. sor): ez pontatlan, a KWin a magas extra gombokat belül nem használja, csak a lenyomottság bitjét nézi. Több ismeretlen kód tehát **ugyanarra** a Qt-gombra esik.

Ebből a forrásból nem következik, hogy a futó Plasma 6.6.7 egy 0x120 fölötti kódot eldob, mielőtt a Qt megkapná. Az igen: a KWin libinput-háttér gombkészlete a 16-os sáv, és a sávon kívüli kód nem kap saját Qt-gombot.

A referenciagép dokumentált asztala **KDE Plasma 6.6.6** (`docs/development/reference-machine.md` 15–17. sor, rögzítve 2026-11-02). A terv 13. sora István 2026-11-23-i közlése szerint 6.6.7. A futó gépet ez az audit nem érte el (14. fejezet). A `6.6.6` és a `6.6.7` eltérését a dokumentum nem simítja el.

A CFA4 `main` kódjában kezesség-kezelés nincs. A `left_handed`, `LeftHanded`, `handedness`, `balkez` és `jobbkez` szavak a `crates`, `apps`, `python` és `docs` alatt csak ebben az auditban szerepelnek; a gesztusterv ága nincs a `main`-en.

### 7.3 Középsőgomb-emuláció

libinput 1.32.0 dokumentáció, <https://wayland.freedesktop.org/libinput/doc/latest/middle-button-emulation.html>, a 2026-11-23-i letöltés szövege: ha az emuláció be van kapcsolva, a bal és a jobb együttes lenyomása középső gombot ad, és a bal meg a jobb eseményt eldobja („the left and right button events are discarded otherwise”). Ugyanott: sok eszközön az emuláció alapból ki van, és a kapcsoló a kompozitoré, nem az alkalmazásé.

A CFA4 ezt nem olvassa, és nem kapcsolja. Bekapcsolt emulációnál az A/B úton a bal+jobb rocker és a kétgombos kombináció nem látja a két fizikai gombot. Az evdev-út a kernel kódját látja, az emuláció nélkül, ezért a két út eltérhet. Hogy a referenciagép egerein az emuláció be van-e kapcsolva, nem mért.

### 7.4 A két út egységesítése (javaslat, GDR-21)

A terv 8.4 döntését a forrás alátámasztja, és ez az audit átveszi javaslatként:

1. Gombcsere: mindig a kompozitoré. A CFA4 másodszor nem cserél. Az A/B úton a `LeftButton` az elsődleges, a `RightButton` a másodlagos (`qnamespace.qdoc` 299–301. sor).
2. Kéz (jobb/bal): csak a tükrözésre és a feliratra hat. Az alapérték „nincs tükrözés”; István a tervvel együtt jóváhagyta (`…-1.1.md` 154. sor).
3. A C út a fizikai `BTN_LEFT` / `BTN_RIGHT` kódot látja. A szerepet eszközönként a felhasználó beállítása, majd a Learn-kalibráció (ugyanaz a nyomás mindkét úton, grab nélkül), majd ha egyik sincs, a `kcminputrc` olvasása adja. Ha egyik sem elérhető, a GUI kérdez, és addig a C úton a bal/jobb nem köthető.
4. Váltás csak felengedett gombok mellett (a libinput 6097–6098. sora).
5. A Flatpak-homokozó a `kcminputrc`-t és a GNOME-kulcsot nem feltétlenül látja (a manifest home-ot ad, a host `~/.config` egy része így elérhető lehet, de ezt nem mértem). Ezért a kalibráció az elsődleges, az olvasás a tartalék.

## 8. FlowMouse

Viselkedési referencia, kód nem kerül át.

- Tár: <https://github.com/hmily-lcg/flowmouse>, commit `08e1189fd0c4c145811ed26102cd92c4e235afaf` (a változáslista ezt a commitot nevezi meg).
- `manifest.json` ezen a commiton: `manifest_version` 3, `content_scripts` a `<all_urls>` mintára, `background.service_worker`. Böngészőbővítmény, JavaScript.
- A `js/gesture-recognizer.js` `#getDirection` függvénye csak négy irányt ad: a nagyobb tengely szerint `→` `←` `↓` `↑`. Átló nincs. A 8 irány és a szögtolerancia ezért saját CFA4-tervezés (a terv 4. fejezete, 115. sor).
- Licenc: a tár `LICENSE` fájlja a GNU GPL 3 szövege (ugyanezen a commiton). A README 10. sora a GitHub licenc-jelvényére mutat, amely erre a fájlra hivatkozik: <https://github.com/Hmily-LCG/FlowMouse/blob/main/LICENSE>. SPDX-sor a `package.json`-ban nincs (a fájl 404 ezen a commiton). A CFA4 `docs/licensing.md` 9–11. sora a GPL-3.0-kompatibilis kód átvételét a nyilvántartáshoz köti. Átvétel nincs tervezve (GDR-10), ezért nyilvántartási sor nem készül.
- Kezesség: a változáslista V-38 szerint a FlowMouse-ban nincs ilyen kapcsoló. Ezt az audit a `js/gesture-recognizer.js` irányfüggvényéig ellenőrizte; a teljes fát nem diffelte. Kódot nem másolt.

## 9. CI-tesztelhetőség

| Út | Ami ma fut | Hol | Kötelező PR-kapu? |
|---|---|---|---|
| Motor, hardver nélkül | `surface.inject` a `handle` útra teszi a `SurfaceInput`-ot | `surface_runtime.rs` 3–4. és 556–566. sor | A workspace-teszt a teljes validáció része, nem az automatikus `cargo-check` |
| Qt offscreen | `QT_QPA_PLATFORM: offscreen` | `.github/workflows/ci-linux.yml` 126., 288., 370. és 418. sor | A 370. sor a `qt6-gui` job, amely a 360–364. sor szerint **nem** blokkol, és csak `workflow_dispatch`, merge-queue vagy a `cfa4-full-validation` címke mellett fut |
| Weston headless és Xvfb | A `qt6-gui` job telepíti a `weston`-t és az `xvfb`-t, majd `python/tests/test_qt_display_servers.py` | `ci-linux.yml` 394–406. sor | Ugyanaz a nem blokkoló job. A `qt-parity.md` 30. sora: `test_qt_gui_on_x11` és `test_qt_gui_on_wayland` |
| Vezérlőcsatorna egér | Csak bal, jobb, középső | `control.py` 82. és 640–642. sor | Extra gomb ezzel a csatornával nem tesztelhető, amíg a szótár nem bővül |
| `uinput` | A workflow-ban a szó nem szerepel | `ci-linux.yml` (keresés: nincs találat) | Nem lehet kötelező kapu |

A GitHub-futók `/dev/uinput` jogát ez az audit nem mérte: futó `ubuntu-26.04` runnerre nem lépett be. A terv V-23 pontja ugyanezt nyitva hagyta.

Ebben a felhős munkamenetben (nem a referenciagép, nem a GitHub-futó): `/dev/uinput` és `/dev/input` nincs, `weston` és `plasmashell` nincs a `PATH`-on, a felhasználó csoportjai között az `input` nincs. Ebből a GitHub-futóra és a referenciagépre következtetni nem lehet.

A felismerő CI-je hardver nélkül is megvan, ha tiszta függvény: a minták időbélyeggel jönnek, `Instant::now` a felismerőben nincs (a terv 4. fejezet, 138. sor). A motoroldali cél a F0 spec 214. sora: p95 ≤ 10 ms az L5 időbélyegtől a `surfaceAction` kiírásáig, a referenciagépen. Ez még nincs megmérve (`CFA4-CONTROL-SURFACE-M1-M10.md` 35–36. sor).

## 10. Hova kerüljenek a gesztusbeállítások

A tulajdonos kérése: Qt6 Beállítások (Preferences) › Hardver. A „Hardver” szó a felületen ma nem lapcím.

### 10.1 A motor csoportja

`crates/engine/src/settings.rs` 1013–1032. sor: a Qt beállításablak csoportjai. A `hardware` csoport címe **„Hardware & Devices”**. A komment (1021–1022. sor): ide tartozik a csak Qt-os Accelerators lap is (`cfa4-cli hardware`), amelynek még nincs preference-kulcsa.

Ugyanennek a csoportnak a kategóriái:

| Kategória | Cím | Sor |
|---|---|---|
| `audioHardware` | Audio Hardware | 1157–1159 |
| `controlSurfaces` | Control Surfaces | 1221–1223 |
| `memory` | Memory | 1387–1389 |
| `timecodeSync` | Timecode Sync | 1424–1426 |

A `prefs.schema` visszaadja a `groups` tömböt és kategóriánként a `group` mezőt (`settings.rs` 2022–2040. sor). A Control Surfaces lap sorai (1225–1230): egy jelölőnégyzet `controlSurfaces.enabled`, majd `Row::Custom("controlSurfaces")`, majd egy jegyzet.

A magyar katalógusban a „Control Surfaces” fordítása „Vezérlőfelületek”, a menüponté „Vezérlőfelületek…” (`i18n/hu.tsv` 4691. és 4806. sor). „Hardware & Devices” sor a `hu.tsv`-ben nincs. A HW-illesztés Q5 pontja a „Vezérlők” nevet fogadta el (`…-ILLESZTES.md` 44. sor); a katalógus ezt a nevet még nem használja. Új felirat csak az `i18n/` alá kerülhet (`AGENTS.md`, fordítások).

### 10.2 Amit a szerkesztő tényleg rajzol

`python/cfa4_qt/panels/settings.py` a szerkesztő Preferences ablaka (1–6. sor: kategórialista balra, lap jobbra, Edit ▸ Preferences). A `set_data` (469–491. sor) a séma `categories` listáját járja, és **nem olvassa a `group` mezőt**. Csoportfejléc a listában nincs. A `MENU` tuple (65–83. sor) lapos; a 82. sor: `("controlSurfaces", N_("Control Surfaces…"))`. Az „Audio Hardware…” a 69. sor, testvér, nem szülő.

A lap törzsét a `_section` a `controlSurfaces` custom azonosítóra a `_control_surfaces` függvénybe adja (873–874. sor). Annak oszlopa (1014–1026. sor), felülről:

1. `_surface_devices` — objektumnév `settings.controlSurfaces.devices` (1110. sor)
2. `_assignments` — `settings.controlSurfaces.assign` (1206. sor), itt van a Learn
3. `_profile_editor` — `settings.controlSurfaces.profile` (1517. sor)
4. `_profile_warnings_box` — `settings.controlSurfaces.warnings` (1578. sor)
5. `_midi_monitor` — `settings.controlSurfaces.monitor` (1610. sor)

A fókuszált szakaszra a `_reveal_focus` görget (1029–1034. sor), `findChild` az objektumnévre.

Az Ablak menü ugyanezt a lapot nyitja, szakaszra görgetve (`python/cfa4_qt/window_menu.py` 361–375. és 397–400. sor):

| Parancs | Felirat | Szakasz |
|---|---|---|
| `window.controlSurfaces` | Control Surfaces… | `settings.controlSurfaces.devices` |
| `window.midiMonitor` | MIDI Monitor… | `settings.controlSurfaces.monitor` |
| `window.surfaceAssignments` | Learn Assignments… | `settings.controlSurfaces.assign` |

### 10.3 A fejlesztői beállításablak

`python/cfa4_devtools/qt_settings.py` 1–6. sor: Edit ▸ Settings…, csoportokkal, a Hardware & Devices az Accelerators lappal kezdődik. A `settings_backend.py` `Schema.parse` (211–214. sor) a `hardware` csoport elejére beszúrja az Accelerators lapot, és minden `group == "hardware"` kategóriát ide gyűjt. A Control Surfaces így ennél az ablaknál a Hardware & Devices alatt jelenne meg, de csak a séma `fields` mezőivel (204. sor). A `Row::Custom` eszközlistát ez az ablak nem rajzolja.

A `docs/settings-menu.md` 45. sora (frissítve 2026-11-06) a Hardware & Devices lapjai között az Accelerators, az Audio Hardware és a Memory szerepel. A Control Surfaces és a Timecode Sync azóta ugyanabba a csoportba került (`settings.rs`, fenti táblázat). A markdown tábla elmaradt a sémától.

### 10.4 Beillesztési pont

A gesztusbeállítás a **meglévő Control Surfaces lap új szakasza**, a `hardware` csoportban. Új kategória és új ablak nem kell: a lap már a Hardver csoport tagja, a szerkesztő pedig ezt a lapot nyitja a Preferences-ből és az Ablak menüből.

Konkrét hely, implementáció nélkül:

1. Motor: a `controlSurfaces` kategória `rows` tömbjében (`settings.rs` 1225–1230. sor) a `Row::Custom("controlSurfaces")` után egy `Row::Custom("controlSurfacesGesture")`. A `group` marad `"hardware"`. A `prefs.schema` így továbbra is ezt a csoportot adja.
2. Szerkesztő: a `_control_surfaces` oszlopában (`settings.py` 1020–1026. sor) az `_assignments` után egy új csoport, objektumnév `settings.controlSurfaces.gesture`. A Learn alatt, a profilszerkesztő előtt van: a gombtanítás és a gesztustanítás egymás mellett, a MIDI-monitor alattuk marad. A `_reveal_focus` a név alapján odagörget.
3. Ablak menü: negyedik sor a `window_menu.py` 366–369. és 397–400. táblájában, parancs `window.mouseGestures`, szakasz `settings.controlSurfaces.gesture`, ugyanaz az `open_surfaces`.
4. A `cfa4_devtools` ablakot nem kell gesztus-szerkesztővel bővíteni: a custom szakaszt nem rajzolja. A csoportcímet a szerkesztő Preferences listája ma nem mutatja (`set_data`, 478–491. sor).

István 2026-11-23 10:16-kor ezt a helyet fogadta el (12.1, 8. pont): a szakasz a Hardware & Devices › Control Surfaces lapon van, magyar címe **Egérgesztusok**, objektumneve `settings.controlSurfaces.gesture`. A „Hardver” fejléc kirajzolása a Preferences listában nincs ebben a döntésben. A lap magyar neve **Vezérlőfelületek** marad (12.1, 9. pont; `i18n/hu.tsv` 4691. és 4806. sor). Új felirat az `i18n/` katalógusokba kerül, amikor a szakasz elkészül; ez a commit feliratot nem ad hozzá.

## 11. MG1 interfész- és fájltervezet

Az MG1 a terv 21. fejezete szerint (`…-1.1.md` 712–729. sor): bemenet normalizálása, dinamikus gombregiszter, gombállapot, Learn-háttér, rögzítés, szűrés, felismerés, irány, küszöb, timeout, megszakítás, confidence, több gomb. Kapu: determinisztikus felismerés és dinamikus gombok, rögzített mintán, CI-ben. A dispatch (MG2) és a Qt-szerkesztő (MG3) nincs benne.

**Réteg:** a felismerő a `cfa4-surface-model` (L1) új modulja. Indok: az ADR 2. döntése az L1-et I/O nélküli, közös típuscrate-nek írja (`ADR-0012` 37–39. sor); a `lib.rs` 1–7. sora ugyanezt mondja (nincs óra, nincs szál). Új crate nincs, új függőség nincs. Az `evdev` és az `udev` crate továbbra sem kell: az ADR 78–79. sora L5 alatt tiltja őket, a `cargo xtask layers` viszont csak a `UI_ONLY` listát őrzi (`xtask/src/main.rs` 205–206. és 254–255. sor), az `evdev`/`udev` nevet nem. Az MG1 úgy kerüli el a lyukat, hogy ezeket a crate-eket be sem húzza.

Az L4 (`crates/engine`) csak hívja a felismerőt, küszöböt és időzítést tart, és a meglévő `surface.inject` útra teszi a kész `SurfaceInput`-ot. A Qt az MG1-ben nem változik.

### 11.1 Fájlok (még nem léteznek)

| Fájl | Réteg | Szerep |
|---|---|---|
| `crates/surface-model/src/gesture.rs` | L1 | mintatípus, küszöb, sablon, `recognize` |
| `crates/surface-model/src/lib.rs` | L1 | `pub mod gesture`; `SurfaceInput::Gesture`; `SurfaceProtocol::Pointer` |
| `crates/engine/src/gesture_runtime.rs` | L4 | gombállapot, rögzítés, timeout, megszakítás; a felismerőt hívja; `surfaceAction`-t nem küld, amíg a kötés nincs (az MG2) |
| `crates/engine/src/lib.rs` | L4 | `pub mod gesture_runtime` a `surface_runtime` mellé |
| `crates/surface-model` tesztek a `gesture.rs` `#[cfg(test)]` moduljában | L1 | rögzített mintasor, determinizmus |

A `crates/engine/src/control_surfaces.rs`, a `surface_runtime.rs` `Triggers` és a `crates/surface/src/hid_in.rs` az MG1-ben nem változik. A ShuttlePRO-tesztek így a gesztus nélkül is ugyanazt futják.

### 11.2 Típusok

A felismerő nem hív órát és nem olvas fájlt. Az időt a hívó adja, `MonoNs` (`surface-model` `lib.rs` 53–56. sor), nem `f64` másodperc. A koordináta logikai pixel, `i32`. A gombállapot bitkészlet, nem fix hosszú tömb: a bit a **logikai szerep** (elsődleges, másodlagos, középső, majd a tanult szerepek). A fizikai kód mellette, `ControlId::Evdev` vagy a Qt-gomb értéke `u32`-ben. A `KEY_MAX` (`0x2ff`) `u16`.

Javasolt alak, még nem kód:

```rust
pub struct PointerSample {
    pub t_ns: u64,
    pub x_px: i32,
    pub y_px: i32,
    pub buttons: u32,
    pub modifiers: u8,
}

pub struct Thresholds {
    pub min_length_px: u32,
    pub max_length_px: u32,
    pub slop_px: u32,
    pub min_ms: u64,
    pub max_ms: u64,
    pub angle_milli_deg: u32,
    pub hold_ms: u64,
}

pub struct Template {
    pub id: String,
    pub points: Vec<(i32, i32)>,
    pub spatial: bool,
}

pub enum Recognize {
    None,
    Direction(Direction),
    Match { id: String, confidence_milli: u16 },
    Cancelled,
}

pub fn recognize(samples: &[PointerSample], templates: &[Template], t: &Thresholds) -> Recognize;
```

`Direction`: nyolc érték (a négy égtáj és a négy átló). `confidence_milli` 0…1000, egész. `Template::points` hossza legfeljebb **64**. A 64 a minta felső korlátja (`AGENTS.md` §0.4); István 2026-11-23 10:16-kor elfogadta (12.1, 5. pont). A 64 fölötti minta hiba, nem csendes vágás.

`SurfaceInput` új változata, a meglévő `Button` mellett:

```rust
Gesture { id: String }
```

A `type` címke (`lib.rs` 191. sor) miatt a régi JSON érvényes marad. Ismeretlen címke a `Binding` úton `unsupported` (2. fejezet).

`SurfaceProtocol::Pointer`, JSON-név `pointer`. A Wayland-mutató eszközazonosítója a fix `pointer:seat` (`DeviceId`), mert a `wl_pointer.button` nem ad eszközt (4.A). Az evdev-eszköz marad `DeviceId::hid`.

### 11.3 Gombregiszter és Learn

A regiszter `HashMap`, kulcsa `(út, ControlId)`, nem `[T; N]`. Futásidejű állapot (lenyomva, a Hold határideje) nem kerül a profilfájlba (a terv 5.3, `…-1.1.md` 212–218. sor).

A Learn az MG1-ben a felismerő mellett, a motorban, **nem** a `surface_runtime` egyetlen Learn-gépébe kötve, amíg a 2. fejezet jóváhagyása nincs meg. A függvény nem hív `Session::execute`-ot. A meglévő `cmd_learn` (705–708. sor) változatlan marad, így a MIDI Learn tesztjei nem látnak második gépet. A közös gépre kötés az MG2, a szülőterv igenje után.

### 11.4 Amit az MG1 nem csinál

- Nem nyit `/dev/input` és `/dev/uinput` csomópontot.
- Nem módosítja a `70-cfa4-surfaces.rules` fájlt.
- Nem ad `QGestureRecognizer` osztályt.
- Nem ír `surfaceAction`-t gesztusra (az MG2, a `run_surface_action` úton).
- Nem bővíti a profilsémát. A sablonok a 2-es verzióba kerülnek (12.1, 4. pont): a v1 `additionalProperties: false`, a magasabb verziót a mai olvasó békén hagyja. A sémafájl írása a CONTROL-SURFACE-001 és a HW-illesztés E8 pontjával közös lépés (`…-ILLESZTES.md` 28. sor), és nincs ebben a commitban. Az MG1 a sablonokat csak a teszt argumentumában látja.
- Nem húz be crate-et. A `deny.toml` 39–41. sora a GPL-3.0-or-later függőséget engedi; az MG1-nek nincs rá szüksége.

### 11.5 ADR-kiegészítés szövege (még nem az ADR fájlban)

István 2026-11-23 10:16-kor úgy döntött, hogy a gesztusvezérlés az ADR 0012 kiegészítése lesz, nem új ADR (12.1, 2. pont). A bekezdés beírása az ADR fájlba külön lépés; ez a commit az ADR-t nem módosítja. A beírandó szöveg, változatlanul:

> 2026-11-23, MG0. A `crates/engine/src/surface/` könyvtár nem készült el; a leképezés, a Learn és a profil a `control_surfaces.rs`, a `surface_runtime.rs` és a `surface_profile.rs` fájl. A gesztus felismerője az L1 `cfa4-surface-model` I/O nélküli modulja. A `SurfaceProtocol` a `Pointer` változattal bővül (Qt-mutató, eszközazonosító nélkül). A felismert gesztus `SurfaceInput::Gesture`, nem új protokoll. Az evdev-egér csak az L5-ben, alapból kikapcsolva, `unsafe` és új crate nélkül. A Qt nem ismer fel és nem képez le. A `PROTOCOL` marad 1.

## 12. Nyitott kérdések Istvánnak

A kérdések 2026-11-23 10:16-ig (Europe/Budapest) nyitottak voltak. A választ a 12.1 rögzíti. A 10. kérdés érvényes döntése 10:54. Az MG1 ettől a mondattól még nem indul.

1. **Az MG0 kapu tartalma.** A dokumentált architektúra, a bemeneti utak, a dinamikus gombmodell és a 11. fejezet interfésze. Döntés: elfogadva (12.1, 1. pont). A kapu ettől még nem zárult (13. fejezet).
2. **ADR-kiegészítés.** A 11.5 bekezdés az ADR 0012-be kerül, vagy új ADR készül? Döntés: kiegészítés az ADR 0012-be, új ADR nincs. A fájl módosítása külön lépés, ez a commit nem írja meg (12.1, 2. pont).
3. **Közös modell.** A Hold, a kombináció és a `Binding` bővítése. Döntés: megvárja a CFA4-CONTROL-SURFACE-001 külön igenjét. Addig az MG1 a mostani gombindításhoz (`Triggers::press`, `control_surfaces.rs` 232–247. sor) nem nyúl (12.1, 3. pont).
4. **Profilséma.** Döntés: a gesztussablon a profilséma 2-es verziója, nem csendes v1-bővítés (12.1, 4. pont).
5. **Sablonpont-plafon.** Döntés: felismerőnként 64 pont (12.1, 5. pont).
6. **Köthető parancsok.** A `CommandSpec`-ben nincs destruktív jelölő (`commands.rs` 23–36. sor). Döntés: az első kiadásban gesztushoz csak `playback.*`, `playhead.*` és `window.*` köthető; minden más kötésnél a GUI figyelmeztet (12.1, 6. pont).
7. **Kontextus-üresjárat.** Döntés: ha a szűkebb kontextusban nincs kötés, a gesztus nem esik vissza a globálisra, és nem történik semmi (12.1, 7. pont).
8. **A beállítás helye.** Döntés: Hardware & Devices › Control Surfaces, új szakasz, magyar címe Egérgesztusok, objektumneve `settings.controlSurfaces.gesture`. A „Hardver” fejléc kirajzolása a Preferences listában nincs ebben a döntésben (12.1, 8. pont; 10.4).
9. **A magyar lapnév.** Döntés: Vezérlőfelületek, ahogy a katalógus ma írja (`i18n/hu.tsv` 4691. és 4806. sor). A Q5 „Vezérlők” neve nem lép a helyére (12.1, 9. pont).
10. **Belépési feltételek a további fázisokra.** Az MG1 megvárja-e a `qt-parity.md` 1.9 / 1.10 / 5.5 ✅ állapotát, az M1–M10 kitöltését, valamint az OSC és a Mackie/HUI elkészültét? Döntés, 2026-11-23 10:54 (Europe/Budapest): nem. Az MG1 ezeket nem várja meg; a tulajdonos külön igenjével indulhat (12.1, 10. pont). A 10:52-es, ellentétes bejegyzés hatályát veszti. A 0. fejezet 2–4. sora a `main`-en továbbra sem teljesül, és az MG1 indítását nem blokkolja.
11. **Plasma-verzió.** Döntés: a referenciaverzió 6.6.7. A `docs/development/reference-machine.md` 17. sora még 6.6.6. A fájl frissítése külön dokumentummódosítás, csak a tulajdonos külön igenjével; ez a commit nem nyúl hozzá (12.1, 11. pont).
12. **Evdev az MG1–MG3 alatt.** Döntés: csak a Qt-út épül. Az evdev-olvasás külön fázis, a jogosultsági döntés után. A ShuttlePRO útja nem változik (12.1, 12. pont).

### 12.1 Tulajdonosi döntések (2026-11-23)

Horváth István, 2026-11-23 10:16 (Europe/Budapest), a 12. fejezet javaslataira: „javaslataid elfogadva jóváhagyom”. A 10. pontot 2026-11-23 10:54-kor (Europe/Budapest) döntötte el; ez felülírja a 10:52-es, ellentétes rögzítést. Az MG1-et egyik mondat sem indítja. Az ADR 0012 szövege és a `reference-machine.md` nem változott.

| # | Döntés | Állapot |
|---|---|---|
| 1 | Az MG0 tartalma (architektúra, bemeneti utak, dinamikus gombmodell, a 11. fejezet interfész-tervezete) elfogadva | Elfogadva |
| 2 | A gesztusvezérlés az ADR 0012 kiegészítése, nem új ADR. A szöveg beírása külön lépés | Elfogadva; a fájl még a régi |
| 3 | A Hold, a kombináció és a kötésbővítés a szülőterv külön igenjére vár. Addig az MG1 a mostani gombindításhoz nem nyúl | Elfogadva |
| 4 | A gesztussablon a profilséma 2-es verziója, nem csendes v1-bővítés | Elfogadva |
| 5 | Sablonpont-plafon: felismerőnként 64 pont | Elfogadva |
| 6 | Az első kiadásban csak `playback.*`, `playhead.*` és `window.*` köthető gesztushoz. Minden más kötésnél a GUI figyelmeztet | Elfogadva |
| 7 | Szűkebb kontextusban kötés nélkül nincs visszaesés a globálisra, és nincs művelet | Elfogadva |
| 8 | Hely: Hardware & Devices › Control Surfaces, új „Egérgesztusok” szakasz (`settings.controlSurfaces.gesture`). A Preferences-lista „Hardver” fejléce nincs ebben a döntésben | Elfogadva |
| 9 | A magyar lapnév: Vezérlőfelületek | Elfogadva |
| 10 | 2026-11-23 10:54 (Europe/Budapest): nem. Az MG1 nem várja meg a Qt6 nyitott sorait (`qt-parity.md` 1.9, 1.10, 5.5), a fizikai mérést, az OSC-t és a Mackie/HUI-t. A tulajdonos külön igenjével indulhat. A 10:52-es igen felülírva | Elfogadva |
| 11 | A Plasma referenciaverzió 6.6.7. A `reference-machine.md` frissítése külön igenre vár | Elfogadva; a gép leírása még 6.6.6 |
| 12 | Az MG1–MG3 csak a Qt-utat építi. Az evdev-olvasás külön fázis a jogosultsági döntés után. A ShuttlePRO útja nem változik | Elfogadva |

## 13. Az MG0 kapu, pontonként

A kapu (`…-1.1.md` 710. sor): „az architektúra, inputhozzáférés, dinamikus gombmodell és integrációs interfészek dokumentált és István által jóváhagyott állapota; ADR-módosítás jóváhagyva.”

| Pont | Állapot | Hol |
|---|---|---|
| Meglévő motor auditja (`surface-model`, `surface`, `control_surfaces.rs`, `surface_runtime.rs`, `surface_profile.rs`, `serve_surface.rs`, `settings.py`) | Dokumentálva | 1. és 3. fejezet |
| ADR 0012 eltérés: nincs `surface/` könyvtár | Dokumentálva | 1. fejezet, `lib.rs` 23. és 85–87. sor |
| Leképezés és a két tengely szabálya | Elfogadva 10:16-kor: szűkebb üres kontextusnál nincs visszaesés és nincs művelet | 1.1, 12.1 / 7. pont |
| Parancsút `surfaceAction` → `run_surface_action` | Dokumentálva | 1. fejezet, `main_window.py` 2027–2028. és 2343–2346. sor |
| Event Fabric: a gesztus nem kerül rá | Dokumentálva, kód nem kell | 1.4 |
| Qt bemenet, helyi menük, középső gomb | Dokumentálva | 5. fejezet |
| Linux-út A/B/C határai (16 / 27 / `KEY_MAX`) | Forrásból igazolva | 4. fejezet, Qt 6.10.2 és Linux `v6.12` |
| Dinamikus gombazonosító | Elfogadva a 11. fejezet tervezetével: `ControlId` + bitkészlet, nincs fix tömb | 4., 11.2, 12.1 / 1. pont |
| Fix korlátok a kódban | Dokumentálva | 4. fejezet táblája |
| FlowMouse csak viselkedés, GPL-3.0, négy irány | Igazolva a megnevezett commiton | 8. fejezet |
| Saját felismerő fájljai és típusai | Elfogadva tervezetként, kód nincs | 11. fejezet, 12.1 / 1. pont |
| Új függőség | Nincs, és az MG1-hez nem kell | 11.4 |
| Evdev biztonság (udev, `uaccess`, `input` csoport, Flatpak) | Dokumentálva. Az MG1–MG3 csak a Qt-utat építi; az evdev külön fázis | 6. fejezet, 12.1 / 12. pont |
| Profilséma v1 vagy v2 | Elfogadva: 2-es verzió, nem csendes v1-bővítés | 11.4, 12.1 / 4. pont |
| Köthető parancsok köre | Elfogadva: első kiadásban `playback.*`, `playhead.*`, `window.*`; minden más kötésnél a GUI figyelmeztet | 12.1 / 6. pont |
| AC-küszöbök | A pontplafon 64, elfogadva (12.1 / 5. pont). A terv 22. fejezetének többi kezdőértéke (`…-1.1.md` 806–827. sor) nincs megmérve. A p95 ≤ 10 ms a F0 spec 214. sora, nem mért | M1–M10 üres |
| Beillesztés a Hardver helyére | Elfogadva: Hardware & Devices › Control Surfaces, „Egérgesztusok” (`settings.controlSurfaces.gesture`). A lista „Hardver” fejléce nincs a döntésben | 10.4, 12.1 / 8. pont |
| Wayland gombok a referenciagépen | **Nem mért.** A referenciaverzió 6.6.7 (12.1 / 11. pont); a `reference-machine.md` 17. sora még 6.6.6, és ez a commit nem írja át | 14. fejezet |
| Libinput középsőgomb-emuláció a referenciagép egerein | **Nem mért.** A dokumentáció: bekapcsolva eldobja a bal és a jobb eseményt | 7.3 |
| 8+ gombos és billentyű-interfészes egér | **Nem mért.** Eszköz nem volt | 14. fejezet |
| HUD ablakpozíció Waylanden | **Nem mért.** A terv V-25 ezt MG0-ra hagyta (`…-1.1.md` 540. sor). A KWin forrása a gombkészletet írja, az ablakhelyezést ez az audit nem nézte | 14. fejezet |
| `/dev/uinput` a GitHub-futón | **Nem mért.** A CI nem használja | 9. fejezet |
| Az MG0 tartalmának elfogadása | Elfogadva 2026-11-23 10:16-kor | 12.1 / 1. pont |
| ADR-módosítás | A forma elfogadva: kiegészítés az ADR 0012-be, nem új ADR. A szöveg beírása külön lépés, a fájl nem változott | 11.5, 12.1 / 2. pont |
| Az MG1 belépési feltételei (Qt6 nyitott sorok, eszközös mérés, OSC, Mackie/HUI) | Elfogadva 2026-11-23 10:54-kor: az MG1 ezeket nem várja meg. A sorok a `main`-en nem teljesülnek (0. fejezet, 2–4. sor), és az MG1 indítását nem blokkolják | 12.1 / 10. pont |

A döntések teljesek. A kapu **nem zárult le**, mert a fizikai mérés a tulajdonos gépén még hiányzik (az extra gombok eljutnak-e a Qt-ig, a középsőgomb-emuláció, a balkezes mód, a HUD helye; 14. fejezet). Ez az MG0 kapu feltétele, az MG1 indítását nem blokkolja. Az MG1 a tulajdonos külön igenjével indul. Ez a commit nem indítja.

## 14. Amit nem mértem

Ez a munkamenet felhős Linux, KDE nélkül. A `plasmashell` és a `weston` nincs a `PATH`-on. A `/dev/input` és a `/dev/uinput` nincs. A felhasználó nincs az `input` csoportban. A referenciagép (Dell Precision Tower 7910, Ubuntu Studio 26.04, Wayland; `docs/development/reference-machine.md` 9–19. sor) nem ez a gép.

- A KDE Plasma 6.6.7 futó példánya elnyeli-e az extra egérgombot, és mely kódok érnek el a Qt-ig. A KWin `master` forrása a 16-os sávot és az `ExtraButton24` összevonást mutatja (7.2); a 6.6.7 tagot a nyers URL nem adta, és élő munkamenet nem futott.
- A futó gép Plasma-verziója. A döntött referencia 6.6.7 (12.1, 11. pont). A `reference-machine.md` 17. sora 6.6.6; a fájl frissítése külön igenre vár, és ez a commit nem végzi el. A 6.6.7 munkamenet gombjait, balkezes módját és HUD-helyét nem mértem.
- A libinput középsőgomb-emuláció alapállapota a gép egerein.
- Konkrét egér: mely gomb `BTN_*`, melyik `KEY_*`.
- A GitHub `ubuntu-26.04` futó `/dev/uinput` joga. A `ci-linux.yml` nem említi.
- Waylandon egy kliens szabadon tehet-e HUD-ablakot a mutató mellé (V-25).
- A Mutter megvalósítja-e a GNOME `left-handed` kulcsot a libinput hívásával. A séma szövege igazolt (7.1), a Mutter-fa nem volt megnyitva.
- X11-en a csere útja (xf86-input-libinput vagy X-gombtérkép), és hogy a Qt xcb-pluginja a cserélt gombot kapja-e. Az xcb `translateMouseButton` a kapott X-gombot képezi (4.B); a cserét a szerver oldalán nem néztem.
- Flatpak-homokozóból olvasható-e a `kcminputrc` vagy a GNOME-kulcs. A manifest a home-ot adja (6. fejezet), a futó homokozót nem próbáltam.
- A FlowMouse fáját a licencfájlra, a manifestre és a `#getDirection` függvényre néztem, nem soronként az egész bővítményre.

## 15. Források

| Forrás | Azonosító |
|---|---|
| CFA4 `main` | `3cdd81bc5ad3d981295fdb1b0c9d05f1652cbddd` |
| Gesztusterv | `329ff31e5ec0325274ed521db9b69f8b3d2b9fd3` a `docs/cfa4-control-surface-gesture-001-plan` ágon, <https://github.com/ubuntuokos/CinemaFundArchitecture4/blob/329ff31e5ec0325274ed521db9b69f8b3d2b9fd3/docs/plans/CFA4-CONTROL-SURFACE-GESTURE-001-1.1.md> |
| Qt 6.10.2 | tag `v6.10.2`, `qnamespace.h`, `qnamespace.qdoc`, `qwaylandinputdevice.cpp`, `qxcbconnection.cpp` |
| Linux bemeneti kódok | `torvalds/linux` tag `v6.12`, `include/uapi/linux/input-event-codes.h` |
| Wayland | `wayland` `main`, `protocol/wayland.xml` 2004–2006. és 2249–2269. sor |
| libinput | `main`, `src/libinput.h` 6088–6098, `src/evdev.h` 661–670; dokumentáció 1.32.0, middle-button-emulation |
| systemd | `main`, `rules.d/70-uaccess.rules.in` 76. sor |
| KWin | `master` (nem a 6.6.7 tag), `src/main.cpp` 112, `connection.cpp` 735, `device.cpp` 248, 421–426, 736, `mousebuttons.cpp` 14–47 |
| GNOME séma | `gsettings-desktop-schemas` `master`, `org.gnome.desktop.peripherals.gschema.xml.in` 53–56. és 112–116. sor |
| Flatpak | `flatpak` `main`, `doc/flatpak-run.xml` 400–405. sor |
| FlowMouse | `hmily-lcg/flowmouse` `08e1189fd0c4c145811ed26102cd92c4e235afaf` |
