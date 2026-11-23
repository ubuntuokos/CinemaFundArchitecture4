CFA4-CONTROL-SURFACE-GESTURE-001

Unified Control Surface – Native Gesture Control Extension

Verzió: 1.1 FINAL
Státusz: FELHASZNÁLÓ ÁLTAL JÓVÁHAGYOTT FEJLESZTÉSI TERV (Horváth István, 2026-11-23). Jóváhagyás: „a terveket jóváhagyom véglegesítsd”. Ez a változat lép az 1.0 FINAL helyébe. Az implementáció engedélyezése külön döntés; a 21. fejezet belépési feltételei változatlanok.
Alapja: az 1.0 FINAL szövege (a repóba nem került; sha256 `c9a7f9d3ba3238e531c261fb602f59042530dd3068f0fa7deb5642f333c05a34`), összevetve a CFA4 `main` `3cdd81bc5ad3d981295fdb1b0c9d05f1652cbddd` állapotával (2026-11-23 08:01, Budapest). [ÚJ: V-01]
Kiegészítés (2026-11-23, István t47u kérésére): jobb- és balkezes használat (kezesség), V-30–V-38. [ÚJ: V-30]
Szülőprojekt: CinemaFundArchitecture4 (CFA4)
Szülőterv: CFA4-CONTROL-SURFACE-001 – Egységes vezérlőfelület-motor (Unified Control Surface Engine), ADR 0012 és az F0 műszaki specifikáció v1.0 [JAVÍTVA: V-02 – a „3.0” verziószám a repóban nem szerepel; a valós hivatkozás `docs/adr/ADR-0012-control-surface.md`, `docs/plans/CFA4-CONTROL-SURFACE-F0-TECHNICAL-SPEC.md`]
Testvérterv: CFA4-HW-CONTROL-SURFACES-001 FINAL 1.1 (közös alapra épül, `docs/plans/CFA4-HW-CONTROL-SURFACES-001-FINAL-1.1-CFA4-ILLESZTES.md`) [ÚJ: V-02]
Viselkedési referencia: FlowMouse – https://github.com/hmily-lcg/flowmouse (böngészőbővítmény, JavaScript, GPL-3.0; kódátvétel nem tervezett) [JAVÍTVA: V-15 – „technológiai donor” helyett]
Célplatform: Linux, Ubuntu 26.04 és kompatibilis változatai; referenciagép: Ubuntu Studio 26.04, KDE Plasma 6.6.7 (István közlése, 2026-11-23; a `docs/development/reference-machine.md` 15–17. sora még 6.6.6-ot ír), Wayland [JAVÍTVA: V-08]
Architektúra: amd64
GUI: Qt6 (PySide6, `python/cfa4_qt/`)
Nyelvek: Rust a motorban és az eszközrétegben (L1 `cfa4-surface-model`, L4 engine, L5 `cfa4-surface`); Python/PySide6 a Qt6 GUI-ban, amely csak bemenetet gyűjt, HUD-ot rajzol és a meglévő parancsúton végrehajt [JAVÍTVA: V-03]
Működés: Offline, lokális, CPU-alapú
Terv jóváhagyása (1.1): IGEN (Horváth István, 2026-11-23)
Implementáció engedélyezése: NEM. Sem az 1.0, sem az 1.1 jóváhagyásából nem következik.

0. JELÖLÉSEK ÉS HIVATKOZÁSOK [ÚJ]

- [JAVÍTVA: V-xx]: az 1.0 szövegének javítása; az indoklás és a bizonyíték a `docs/plans/CFA4-CONTROL-SURFACE-GESTURE-001-1.1-VALTOZASOK.md` V-xx tételében van.
- [ÚJ: V-xx]: új szöveg, ugyanígy indokolva.
- Jelölés nélküli szöveg: az 1.0-val azonos.
- `fájl:sor` hivatkozás a CFA4 `main` `3cdd81bc` állapotára vonatkozik, ha nincs más megadva.
- „[MG0-ban igazolandó]”: forrásból nem ellenőrzött állítás, az audit dönti el.

1. A FEJLESZTÉS CÉLJA

A CFA4 egységes vezérlőfelület-motorjának (CFA4-CONTROL-SURFACE-001) bővítése egy natív, konfigurálható egérgesztus- és többgombos inputrendszerrel.

A fejlesztés célja, hogy a CFA4 felhasználói egérmozdulatokkal, egérgombokkal, többgombos kombinációkkal, görgőkkel, rocker gesztusokkal és kontextusfüggő inputokkal irányítható legyen.

A Gesture Control nem külön vezérlőmotor.

A fejlesztés a jóváhagyott CFA4-CONTROL-SURFACE-001 rendszerének új inputforrása és képességbővítése.

[JAVÍTVA: V-08] A rendszer a lehető legszélesebb egér- és inputeszköz-kompatibilitást biztosítja úgy, hogy a CFA4 saját kódja nem vezet be gombszám-korlátot. A ténylegesen elérhető gombokat az adott bemeneti út határozza meg, és ezt a terv útvonalanként dokumentálja (5.1). „Korlátlan” gombszámot a terv nem ígér, mert ilyet egyik Linux-bemeneti út sem ad.

2. KÖTELEZŐ CFA4 ALAPELVEK

A fejlesztés során kötelező:

- A meglévő CFA4 architektúra elsődlegességének megőrzése.
- A CFA4-CONTROL-SURFACE-001 motorja maradjon az egyetlen központi vezérlőrendszer.
- A Gesture Control új inputforrás legyen.
- [JAVÍTVA: V-05] A meglévő rendszereket a valós nevükön kell használni: leképezés (`ControlSurfacePrefs`, `Binding`, `Triggers`: `crates/engine/src/control_surfaces.rs`), parancsút (`surfaceAction` esemény → Qt `run_surface_action` → a billentyűkkel közös parancsút; motorparancsok: `CommandSpec`, `Session::execute`), profilok (`crates/engine/src/surface_profile.rs`, `cfa4-surface-profile` JSON), Learn (`crates/engine/src/surface_runtime.rs`).
- [JAVÍTVA: V-07] Event Fabric: a gesztus- és eszközesemények folyamaton belül maradnak (ADR 0012, 11. döntés; Event Fabric illesztés E10). Az Event Fabricre legfeljebb audit- vagy eszközállapot-esemény kerülhet, és csak külön tulajdonosi döntéssel.
- [ÚJ: V-04] Egyetlen igazságforrás a motor (L4): a Qt-ban nincs leképezési és nincs gesztusfelismerési logika (ADR 0012, 1. döntés). A Qt rögzíti a mutató mozgását és a gombokat, rajzolja a HUD-ot, és végrehajtja a `surfaceAction`-t.
- [ÚJ: V-27] Rétegszabályok: eszköz-I/O (evdev) csak a `cfa4-surface` (L5) crate-ben lehet; `unsafe_code = "forbid"`; saját ioctl nem készül (ADR 0012, 26., 49. és 78–79. sor).
- [ÚJ: V-30] Kezesség: a jobb- és a balkezes használat egyenértékű. A bal/jobb gomb jelentése logikai (elsődleges/másodlagos), a két bemeneti úton azonos. A rendszer gombcseréjét a CFA4 se meg nem duplázza, se el nem hagyja (8.4).
- Ne készüljön második command engine.
- Ne készüljön második mapping engine.
- Ne készüljön párhuzamos automation vagy script engine.
- A MIDI és HID rendszerek működése ne sérüljön. [JAVÍTVA: V-05] Az OSC ma csak adatmodell-szinten létezik (`SurfaceProtocol::Osc`, `DeviceId::net`), működő OSC-bemenet nincs; amikor elkészül, rá is ugyanez vonatkozik.
- A meglévő ShuttlePRO támogatás ne regresszáljon.
- Linux-only működés.
- Ubuntu 26.04 kompatibilitás.
- amd64 célarchitektúra.
- Qt6 GUI-integráció.
- Offline működés.
- CPU-only gesture recognition.
- GPU használata nem szükséges.
- Felhőszolgáltatás nem szükséges.
- Telemetria nem készül.
- A felhasználó teljes konfigurációs szabadságát meg kell őrizni.
- A hagyományos egérműveletek nem sérülhetnek.
- A fizikai egér képességeit a rendszer nem színlelheti. [ÚJ: V-10] Ebből következik: `uinput`-tal a CFA4 nem állít elő egér- vagy billentyűeseményt, és nem foglalja le kizárólagosan az eszközt (EVIOCGRAB).

[JAVÍTVA: V-15, V-16] A FlowMouse viselkedési referencia. JavaScript nyelvű, Manifest V3 böngészőbővítmény (DOM-eseményekre épül), ezért kódja a CFA4-be technikailag nem vehető át. Az algoritmusa egyszerű: 4 irány (nincs átló), 5 indítógomb. Licence GPL-3.0, ami a CFA4 GPL-3.0-or-later licencével kompatibilis (`docs/licensing.md` 5., 9–11. sor). Ha mégis bármilyen rész portolásra kerülne, az csak a `licensing.md` nyilvántartásával, forrásmegjelöléssel és az eredeti fejlécekkel történhet, és az a rész GPL-3.0 alatt marad. Saját CFA4-implementáció készül.

3. INPUTRENDSZER ARCHITEKTÚRA

A Gesture Control alrendszer komponensei, a CFA4 rétegeihez rendelve [JAVÍTVA: V-04, V-05]:

| # | Komponens | Réteg és hely (javaslat) | Meglévő CFA4-elem, amelyre épül |
|---|---|---|---|
| 1 | Input Capture Layer | Qt: alkalmazásszintű eseményszűrő (`python/cfa4_qt`); opcionálisan L5 evdev-olvasó (`crates/surface`) | `hid_in.rs` evdev-dekóder mintája |
| 2 | Input Device Capability Detection | L5 (evdev út) / Qt (alkalmazásszintű út, korlátozott) | `hid.rs` sysfs-felderítés |
| 3 | Dynamic Button Registry | L4 konfiguráció + L5/Qt futásidejű állapot (külön, lásd 5.3) | `DeviceId`, `ControlId::Evdev` |
| 4 | Button Learn System | L4 | `surface_runtime.rs` Learn-állapotgép |
| 5 | Gesture Recorder | Qt (mintavétel) → L4 | – |
| 6 | Gesture Recognizer | L1 tiszta Rust (I/O nélkül, tesztelhető) vagy L4 [MG0 dönt] | – |
| 7 | Gesture Classifier | ugyanott | – |
| 8 | Gesture Profile Manager | L4 | `surface_profile.rs` |
| 9 | Context Resolver | L4 (a Qt adja az aktív panelt / widgetet) | billentyű-kontextusok (`shortcuts.rs`), a profilséma `context` mezője |
| 10 | Gesture Mapping Adapter | L4 | `ControlSurfacePrefs::action` |
| 11 | Combination Input Resolver | L4 | `Triggers` (ma egy vezérlős) |
| 12 | Wheel Gesture Handler | Qt → L4 | – |
| 13 | Rocker Gesture Handler | Qt → L4 | – |
| 14 | Context-Aware Drag Handler | Qt (a meglévő drag-logika) | panelek meglévő drag-kezelése |
| 15 | Area Selection Handler | Qt (a meglévő kijelölési logika) | panelek meglévő kijelölése |
| 16 | Gesture HUD / Feedback Layer | Qt, ablakon belüli overlay | – |
| 17 | Existing CFA4 Command Dispatch Integration | L4 `surfaceAction` → Qt `run_surface_action` | `surface_runtime.rs` 374. sor, `main_window.py` 2027–2028., 2343. sor |
| 18 | [ÚJ: V-30] Handedness Resolver (fizikai kód → logikai szerep, tükrözés) | L4 (a Qt-út már logikai; az evdev-út leképezése itt) | – (a repóban nincs kezesség-kezelés, V-38) |

[ÚJ: V-04] Adatút: Qt-eseményszűrő → nyers minták (pozíció, idő, gomb, módosító, aktív kontextus) → `serve` (`surface.*` parancs, a meglévő `PROTOCOL = 1` additív bővítése) → L4 felismerés és leképezés → `surfaceAction` → Qt parancsút. Az, hogy a minták folyamatosan vagy a gesztus végén egyben mennek-e át, és hogyan kap a HUD élő irány-előnézetet, MG0-döntés. Feltétel: a felismerő egy példányban, Rustban létezik; Python-másolat nem készül.

A Gesture Control kizárólag a meglévő CFA4 vezérlőrendszerbe továbbíthatja a felismert inputokat és a hozzájuk rendelt műveleteket.

4. NATÍV GESZTUSFELISMERÉS

Támogatandó mozgásirányok:

- Left
- Right
- Up
- Down
- Up-Left
- Up-Right
- Down-Left
- Down-Right

[ÚJ: V-15] Megjegyzés: a FlowMouse felismerője csak 4 irányt ismer (`js/gesture-recognizer.js` 220–229. sor), ezért a 8 irány és a szögtolerancia saját tervezést igényel.

Támogatandó további képességek:

- Többirányú gesztusok
- Egyedi felhasználói gesztusok
- Gesztusok rögzítése és tanítása
- Gesztusok szerkesztése
- Gesztusok érzékenységének beállítása
- Mozgási küszöbértékek (logikai pixelben, a DPI-skálázás figyelembevételével) [JAVÍTVA: V-22]
- Szögeltérés-tolerancia
- Minimális útvonalhossz
- Minimális és maximális idő
- Pontmintavételezés
- Zajszűrés
- Smoothing
- Confidence érték
- Minimum turn angle
- Téves felismerések visszaszorítása
- Cancel és timeout kezelés

A felismerő ne csak előre beégetett gesztusokat tudjon kezelni. A felhasználó saját gesztusokat rögzíthessen és profilhoz rendelhesse őket.

[ÚJ: V-22] Determinizmus: ugyanarra a mintasorozatra (időbélyegekkel) a felismerő mindig ugyanazt az eredményt adja; véletlen és falióra nem befolyásolja. Ez teszi lehetővé a CI-ben futó, hardver nélküli felismerési teszteket.

4.1 Irány-tükrözés kezesség szerint [ÚJ: V-32]

A gesztusok két csoportba tartoznak:

- **Térbeli gesztusok**: az irány a képernyőn látott irányhoz kötött. Ilyen az idővonal időiránya (előző/következő vágási pont, ugrás az elejére/végére, előző/következő sáv), a görgetés, a rácsban előző/következő elem és a zoom. Ezek soha nem tükrözhetők. Az idővonalon az idő balról jobbra halad, és a Qt GUI-ban nincs jobbról balra író elrendezés (`rg 'RightToLeft|setLayoutDirection' python/cfa4_qt` → 0 találat).
- **Ergonomikus (szimbolikus) gesztusok**: az irány csak a mozdulat formája, nem a képernyő iránya. Ilyen például a Vissza/Előre jellegű panel- vagy mappaváltás és a parancsindító alakzatok. Ezek a felhasználó választására vízszintesen tükrözhetők.

Beállítás profilonként: „Vízszintes tükrözés: ki (alapértelmezés) / csak az ergonomikus gesztusoknál”. A gesztusszerkesztőben minden gesztus jelölése „térbeli” vagy „ergonomikus”; a beépített gesztusoknál ezt a CFA4 adja.

Az alapérték (ki) indoklása:
1. A képernyő iránya kéztől független; a térbeli gesztusok tükrözése az idővonallal ellentétes mozdulatot adna.
2. Ugyanaz a parancs minden felhasználónál ugyanazzal a mozdulattal szerepel a súgóban és a HUD-on, így a dokumentáció egyértelmű.
3. A tükrözés így tudatos döntés, és nem írja felül váratlanul a konfigurációt (8.2).

Az alapértéket (nincs tükrözés) István a tervvel együtt jóváhagyta (2026-11-23). A HUD mindig a ténylegesen érvényes (esetleg tükrözött) irányt mutatja.

5. EGÉRGOMB-TÁMOGATÁS – DINAMIKUS FELISMERÉS, CFA4-OLDALI GOMBSZÁM-KORLÁT NÉLKÜL [JAVÍTVA: V-08]

5.1 Alapkövetelmény

A CFA4 Gesture Control nem korlátozhatja a támogatást fixen 3, 5, 6 vagy 8 egérgombra, és a CFA4 kódjában nem lehet statikus felső gombszám.

[JAVÍTVA: V-08, V-09] A ténylegesen elérhető gombkészletet a bemeneti út határozza meg. Útvonalanként (forrás: Qt 6.10 és Linux fejlécek, a változáslista V-08 tételében):

| Bemeneti út | Elérhető gombok | Eszköz megkülönböztethető? | Jogosultság |
|---|---|---|---|
| A) Qt alkalmazásszint, Wayland (referenciagép) | legfeljebb 16: a kernel `BTN_LEFT` (0x110) … 0x11F kódjai, Qt-ban Left/Right/Middle + ExtraButton1–13; minden más kódot a Qt Wayland-plugin eldob | nem: a Wayland `wl_seat` egy mutatóba vonja össze az egereket, a `wl_pointer.button` eseményben nincs eszközazonosító | nem kell |
| B) Qt alkalmazásszint, X11 | legfeljebb 27: Left/Right/Middle + ExtraButton1–24 (X-gomb 8–31; a 4–7 görgő) | nem (alap Qt-eseményben) | nem kell |
| C) evdev (L5, opcionális) | az eszköz által bejelentett összes `EV_KEY` kód (0 … `KEY_MAX` = 0x2ff), beleértve a billentyűként küldött gombokat | igen (eszközönként külön node) | külön udev-szabály vagy `input` csoport; lásd 24. és 26. fejezet |
| D) Gyártói protokoll (pl. Logitech HID++) | csak „elterelt” (diverted) gombok, gyártói eszközzel | igen | nem része az első kiadásnak |

Következmény: a „sokgombos gamer/MMO egér teljes gombkészlete” csak a C úton érhető el, és ott is csak azok a gombok, amelyeket az egér firmware-e egyáltalán eseményként küld. Ha a gyártói szoftver a gombot billentyűre vagy makróra állította, az billentyűeseményként érkezik (`KEY_*`), nem egérgombként.

[ÚJ: V-30] Kezesség a két úton: a rendszer balkezes beállítása (libinput) a bal és jobb gombot még a kompozitorban felcseréli, ezért az A/B úton a Qt már a cserélt, logikai gombot kapja. A C (evdev) út a fizikai kódot látja (`BTN_LEFT`/`BTN_RIGHT`), csere nélkül. A két út egységesítése: 8.4.

A gombszám felső korlátját nem a CFA4 GUI, nem egy statikus konfigurációs enum és nem egy fix méretű tömb határozza meg.

5.2 Támogatandó gombok

- Bal egérgomb
- Jobb egérgomb
- Középső egérgomb
- Oldalsó gombok
- Hüvelykujjgombok
- Extra gombok
- Hátsó gombok
- Programozható gombok
- Nagy gombszámú gamer egerek gombjai (a C úton) [JAVÍTVA: V-08]
- Linux input által felismert további gombok
- Nem szabványos vagy új gombazonosítók, amennyiben a választott bemeneti út átadja őket [JAVÍTVA: V-08]

A rendszer nem feltételezhet előre meghatározott maximális Button ID-t. [ÚJ: V-08] A gombazonosító útvonalfüggő: az A/B úton a Qt gombbitje, a C úton az evdev `(ev_type, code)` pár. Utóbbira a modell már ad típust: `ControlId::Evdev { ev_type, code }` (`crates/surface-model/src/lib.rs` 129–132. sor).

5.3 Dinamikus Button Registry

A gombokat dinamikus azonosítók alapján kell kezelni.

[JAVÍTVA: V-28] A regiszter két külön részből áll.

Tárolt konfiguráció (profilban, L4):

- Device ID (`DeviceId`; az A/B úton egyetlen „mutató” eszköz, lásd 5.1)
- Button ID (`ControlId`)
- [ÚJ: V-30] Fizikai kód (az evdev `(ev_type, code)`, ill. a Qt-gomb) és logikai szerep (elsődleges / másodlagos / középső / tanult szerep); a kötés a logikai szerepre hivatkozik
- [ÚJ: V-33] Fizikai hely (felhasználói megadás: pl. hüvelykujj-oldal elöl / hátul, kisujj-oldal)
- Physical/logical identifier, ha rendelkezésre áll
- User-defined button name
- Button category
- Combination membership
- Profile assignment
- Enabled/disabled status

Futásidejű állapot (nem kerül fájlba):

- Input capability (a felderítés eredménye)
- Current state
- Press event
- Release event
- Hold event (az L4 időzítéséből származik; a mai `SurfaceInput::Button` csak lenyomást és felengedést ismer)

A gombszám nem korlátozódhat statikus tömbre vagy fix enumra, amely a későbbi egereket kizárná. [ÚJ: V-27] Megjegyzés: a mai L5 evdev-dekóder csak a Contour-eszközökre és a `BTN_MISC + n` kódokra készült (`crates/surface/src/hid.rs` 15–20., `hid_in.rs` 89–129. sor); egerekhez új, általános dekóder kell.

5.4 Button Learn

Kötelező funkció.

A felhasználó:

1. Megnyitja a Mouse Gesture Control beállításait.
2. Elindítja a Button Learn funkciót.
3. Megnyomja a fizikai egérgombot.
4. A CFA4 rögzíti az inputréteg által szolgáltatott gombazonosítót.
5. A GUI megjeleníti a felismerést.
6. A felhasználó nevet adhat a gombnak.
7. A gomb kombinációba helyezhető.
8. A gomb gesztushoz vagy parancshoz rendelhető.
9. A profil menthető.

A Learn mód nem hajthat végre CFA4-parancsot. Csak inputrögzítés és konfiguráció történhet.

[ÚJ: V-05] A Button Learn a meglévő Learn-állapotgépre épül (F0 spec 7. fejezet; `surface_runtime.rs`): egyszerre egy Learn fut, időkorláttal, megerősítéssel és megszakítással. Második Learn-gép nem készül. [ÚJ: V-12] Learn alatt a megnyomott gomb a CFA4 felületén sem vált ki normál műveletet (pl. a jobb gomb nem nyit helyi menüt).

[ÚJ: V-33] Oldalsó (hüvelykujj-) gombok és kezesség:
- A kernel az oldalsó gombokat `BTN_SIDE` (0x113), `BTN_EXTRA` (0x114), `BTN_FORWARD` (0x115) és `BTN_BACK` (0x116) kóddal adja (`input-event-codes.h` 360–363. sor). A Qt Waylanden a 0x113-at `BackButton`-ként (ExtraButton1), a 0x114-et `ForwardButton`-ként (ExtraButton2) adja tovább (`qwaylandinputdevice.cpp` 809–810. sor). Hogy melyik fizikai gomb melyik kódot küldi, azt az egér firmware-e dönti el, nem a gomb helye.
- A libinput balkezes módja csak a bal és jobb gombot cseréli, az oldalsókat nem (`libinput/src/evdev.h` 661–670. sor).
- Ezért balkezes vagy szimmetrikus egérnél a „hátsó” és az „első” gomb helye eltérhet attól, amit a kód neve sugall. A Button Learn ezt így kezeli:
  1. A gombot a felhasználó a **fizikai helye** szerint nevezi el (a GUI egérvázlaton kínálja a helyeket, a kezesség szerint tükrözve).
  2. A „Vissza” vagy „Előre” szerepet a CFA4 nem a kódból következteti ki, hanem a felhasználó adja meg; a felismert kód csak javaslat.
  3. A beépített kiosztások a szerepre hivatkoznak (pl. „Vissza-gomb”), nem a kódra.

5.5 Button Test

A GUI biztosítson tesztfunkciót, amely:

- Megmutatja a megnyomott gombot.
- Megjeleníti annak azonosítóját és azt, hogy melyik bemeneti úton (A/B/C) érkezett. [JAVÍTVA: V-08]
- Megjeleníti a Press és Release állapotokat.
- Vizsgálhatóvá teszi a Hold eseményt.
- Nem hajt végre parancsot.
- Segít megállapítani, hogy az adott gomb hozzáférhető-e a CFA4 számára.
- [ÚJ: V-11] Jelzi, ha ugyanaz a fizikai gomb két úton (Qt és evdev) is megérkezik.
- [ÚJ: V-30] Megjeleníti a fizikai kódot és a logikai szerepet is (pl. „fizikai: BTN_LEFT → logikai: másodlagos”), és jelzi, ha a két út eltérő szerepet adna.

[ÚJ: V-05] A Button Test a meglévő MIDI Monitor mintáját követi (Window ▸ MIDI Monitor…, `window_menu.py` 366–369. sor; ≤ 20 Hz `surfaceMonitor` esemény).

6. KOMBINÁLT INPUTOK

Támogatandó:

- Egérgomb + mozgás
- Egérgomb + egérgomb
- Több egérgomb egyidejű lenyomása
- Egérgomb + görgő
- Egérgomb + irányított görgő
- Egérgomb + Ctrl
- Egérgomb + Shift
- Egérgomb + Alt
- Egérgomb + Meta
- Több egérgomb + billentyűmodifikátor
- Többgombos kombináció + gesture
- Többgombos kombináció + kontextus
- Hold, Press, Release trigger
- Konfigurálható aktiválási sorrend
- Konfigurálható kombinációs időablak

A rendszer ne korlátozza indokolatlanul a kombinálható gombok számát.

Az inputfeloldás legyen determinisztikus, konfigurálható és konfliktusvédett.

[ÚJ: V-20] Modellbővítés: a mai leképezés vezérlőnként egy parancsot ismer (`DeviceSetting.bindings`, „one per control”, `control_surfaces.rs` 35. sor; `Binding` 185–190. sor), és csak lenyomásra indít (`Triggers`, 226–249. sor). Kombináció, Hold, Release-trigger és módosító a közös modell additív bővítését kéri a `cfa4-surface-model`-ben és az L4-ben. Ez a MIDI- és HID-kötéseket is érinti, ezért a CFA4-CONTROL-SURFACE-001 jóváhagyásával készülhet (a HW-CONTROL-SURFACES Q3 mintájára). A régi CFA4 az ismeretlen kötést `unsupported`-ként megőrzi (`control_surfaces.rs` 20–26. sor).

[ÚJ: V-34] A kombinációk, a sorrend, a Hold és a kombinációs időablak a logikai gombokon (elsődleges, másodlagos, középső, tanult szerepek) működnek, nem a fizikai kódokon. Így ugyanaz a konfiguráció jobb- és balkezes beállítással azonosan viselkedik. Az időablakok kezességtől függetlenek.

[ÚJ: V-14] Bal + jobb gomb együttes lenyomását a libinput középsőgomb-emulációja középső kattintássá alakíthatja, és ilyenkor a bal és jobb eseményt eldobja. Ez az A/B úton a rocker- és kétgombos kombinációkat elnyelheti; MG0-ban a referenciagépen mérendő.

7. GESTURE ACTIVATION MÓDOK

Támogatandó aktiválási módok:

- Right-button hold + movement → logikailag: másodlagos gomb lenyomva tartása + mozgás (jobbkezes beállításnál a jobb, balkezesnél a bal fizikai gomb) [JAVÍTVA: V-34]
- Middle-button hold + movement (a középső gombot a libinput balkezes módja nem cseréli, `libinput.h` 6090–6091. sor) [JAVÍTVA: V-34]
- Custom mouse button + movement
- Multi-button hold + movement
- Modifier + mouse + movement
- Wheel activation
- Rocker activation
- Context-specific activation
- Custom activation profile

Az aktiválási módok felhasználó által konfigurálhatók legyenek.

A normál egérfunkciók megőrzése kötelező.

[ÚJ: V-12, V-13] Ismert ütközések a mai Qt GUI-ban, amelyeket az aktiválásnak kezelnie kell:
- Az idővonal a jobb gomb **lenyomásakor** azonnal helyi menüt nyit (`python/cfa4_qt/panels/timeline.py` 2373–2378. sor). Jobb gombos gesztushoz a menünek a felengedéskor, mozgási küszöb alatt kell megnyílnia, különben a gesztus nem indulhat. Ez a meglévő viselkedés változása, ezért külön jóváhagyást és regressziós tesztet kér. [ÚJ: V-35] A helyi menü a logikai másodlagos gombhoz kötődik. A Qt-úton a `Qt.MouseButton.RightButton` már a rendszer kezességét követi (Qt-dokumentáció, `qnamespace.qdoc` 299–301. sor: „The left button may be the right button on left-handed mice”), így a mai idővonal-kód balkezes rendszeren is a másodlagos gombra nyit menüt. A gesztus-aktiválás ugyanezt a logikai gombot használja; evdev-úton a 8.4 leképezése adja.
- A középső gomb ma pásztáz a monitorban (`panels/monitor.py` 439. sor), és bezárja a szekvencia fülét (`panels/timeline.py` 3386. sor). A Kdenlive-terv K53 pontja középső gombos görgetést tervez az idővonalra (`docs/plans/CFA4-KDENLIVE-PLAN-001-FINAL-1.0.md` 150., 595. sor).
- Alapértelmezésben ezért egyetlen aktiválási mód sincs bekapcsolva. A felhasználó kapcsolja be, és a GUI jelzi az ütközést.

8. GESTURE MAPPING ÉS PROFILE RENDSZER

A Gesture Mapping a meglévő CFA4 mapping infrastruktúrát használja (`ControlSurfacePrefs` / `Binding`, `crates/engine/src/control_surfaces.rs`). [JAVÍTVA: V-05]

A mapping adatai:

- Input device
- Device ID
- Button ID vagy Button Combination
- Kezesség-mód (profilból öröklődik, eszközönként felülírható, 8.4) [ÚJ: V-31]
- Gesture ID
- Activation Type
- Context
- Target CFA4 Command (parancs-id a regisztrált listából, `params`-szal, mint a mai `Binding`)
- Optional Command Chain [JAVÍTVA: V-06 – az első kiadásban nincs, lásd 9. fejezet]
- Profile
- Priority
- Enabled
- Feedback configuration

[JAVÍTVA: V-05] Az inputforrás valós neve a `SurfaceProtocol` enum (`crates/surface-model/src/lib.rs` 61–65. sor: `Hid`, `Midi`, `Osc`). A bővítés javasolt alakja (MG0 dönt):

- Hid
- Midi
- Osc
- Pointer (az A/B út: Qt-ból érkező egérbemenet) és/vagy az evdev-út a meglévő `Hid` + `ControlId::Evdev` alatt

A „GESTURE” nem protokoll, hanem bemenettípus: a felismert gesztus egy új `SurfaceInput`-változat (pl. `Gesture { id }`), a gombok pedig a meglévő `Button` változatot használják. Az elnevezés MG0-döntés.

A gesture mapping ne külön adatbázis- vagy parancsrendszerben működjön.

8.1 Profilok és kontextusok [JAVÍTVA: V-18]

A CFA4-ben a „profil” eszközönkénti kiosztás (`active-<device>.json`, F0 spec 8. fejezet). Az 1.0 példái (Timeline, Media Browser, Effects…) ezért kontextusok, nem profilok:

Példa-kontextusok:

- Global
- Timeline
- Media Browser
- Effects
- VFX
- Audio
- Project
- Custom User Context

A profilséma már tartalmaz `context` mezőt (alapértéke `"global"`, `docs/plans/CFA4-CONTROL-SURFACE-PROFILE-SCHEMA.json` 558–561. sor), de a motor `Binding` típusa ma nem kezeli. A kontextus bevezetése a közös leképezésben történik, így a MIDI és a ShuttlePRO is használhatja. A kontextusnevek a billentyűparancsok meglévő kontextusaihoz igazodnak (`crates/engine/src/shortcuts.rs`).

8.2 Mapping priority

A feloldási sorrend:

1. Widget-specific mapping
2. Module-specific mapping
3. Global mapping
4. No action

[ÚJ: V-18] Ez a sorrend a kontextusokra vonatkozik. Mellette érvényben marad a profilrétegek meglévő sorrendje: beépített → felhasználói → projektprofil, vezérlőnkénti felülírással (F0 spec 8. fejezet, N-06). A két tengely együttes szabályát MG0 rögzíti.

A felhasználó konfigurációja nem kerülhet váratlanul felülírásra.

[ÚJ: V-17] 8.3 Tárolás

- Hely: `$XDG_DATA_HOME/cfa4/Control Surfaces/` (`crates/engine/src/surface_profile.rs` 4. és 22. sor), írás `cfa4_format::atomic_write`-tal, a beállítások a `controlSurfaces.*` kulcsok alatt.
- Formátum: `cfa4-surface-profile`. A séma ma `additionalProperties: false` (séma 16. sor), ezért a gesztussablonok (pontsorozatok), a kombinációk és a kontextus vagy additív sémabővítést, vagy `version: 2`-t kérnek. Ez közös döntés a CFA4-CONTROL-SURFACE-001-gyel és a HW-CONTROL-SURFACES-001-gyel (illesztés E8).
- A gesztussablonok mérete legyen korlátos (pontszám-felső határ sablononként, MG0 rögzíti). Nyers mozgásnapló nem kerül fájlba (24. fejezet).
- Import/export ugyanebben a formátumban, sémaellenőrzéssel.
- [ÚJ: V-31] Az exportált profil rögzíti a kezet (jobb/bal) és a logikai szerepeket. Ha importáláskor a profil keze eltér a felhasználóétól, a GUI felajánlja a tükrözést (oldalsó gombok szerepe, ergonomikus gesztusok iránya), előnézettel. Csendes, automatikus tükrözés nincs, mert a 8.2 szerint a konfiguráció nem írható felül váratlanul.

8.4 Kezesség (jobb- és balkezes használat) [ÚJ: V-30, V-31, V-38]

Tények (forrással):
- A libinput balkezes módja egérnél a bal és jobb gombot cseréli, a középsőt nem; a változás csak minden gomb felengedése után lép életbe (`libinput/src/libinput.h` 6088–6098. sor; megvalósítás: `src/evdev.h` 661–670. sor, csak `BTN_LEFT` ↔ `BTN_RIGHT`).
- GNOME: `org.gnome.desktop.peripherals.mouse` `left-handed` (logikai, alapértéke `false`, globális; `gsettings-desktop-schemas` `org.gnome.desktop.peripherals.gschema.xml.in` 112–117. sor). Az érintőpad `left-handed` kulcsának `'mouse'` értéke az egér beállítását követi (53–57. sor).
- KDE Plasma (KWin): `LeftHanded` beállítás eszközönként, a `kcminputrc` fájl `[Libinput][<gyártó>][<termék>][<név>]` csoportjában (KWin `src/main.cpp` 112. sor, `src/backends/libinput/connection.cpp` 735. sor, `src/backends/libinput/device.cpp` 248. és 736. sor).
- A Qt a Wayland-kompozitortól már a csere utáni gombot kapja; a Qt-dokumentáció szerint a `LeftButton` balkezes egérnél a jobb gomb lehet (`qnamespace.qdoc` 299–301. sor).
- A C (evdev) út a kernel nyers kódját olvassa, a libinput cseréje nélkül.
- A CFA4-ben ma semmilyen kezesség-kezelés nincs (`rg -i 'left_?handed|handedness|balkez|jobbkez'` a `crates`, `apps`, `python/cfa4_qt` és `docs` alatt → 0 releváns találat).

Döntés (javaslat):
1. **Két külön fogalom.**
   - *Gombcsere*: mindig a rendszert követi, a CFA4 maga nem cserél.
   - *Kéz* (jobb/bal): a felhasználó keze. Csak a tükrözésre (4.1), az egérvázlatra és a feliratokra hat, a gombcserére nem. Így dupla csere nem jöhet létre.
2. **Logikai gombmodell.** Elsődleges, másodlagos és középső gomb, a többi gomb a tanult szerepével. Minden kötés logikai szerepre hivatkozik, a fizikai kód mellette tárolódik (5.3).
3. **A/B (Qt) út.** `LeftButton` = elsődleges, `RightButton` = másodlagos. További csere nincs.
4. **C (evdev) út.** A `BTN_LEFT`/`BTN_RIGHT` logikai szerepét eszközönként ebben a sorrendben kell megállapítani:
   a) a felhasználó kifejezett beállítása az eszközre („rendszer cseréli / nem cseréli”);
   b) kalibráció a Button Learnben: az elsődleges gomb lenyomásakor a CFA4 ugyanazt a nyomást a Qt-úton (logikai) és az evdev-úton (fizikai) is látja (grab nincs, V-11), az eltérésből adódik a csere. Ez asztali környezettől független;
   c) csak olvasás, ha (a) és (b) nincs: GNOME kulcs, ill. a KWin `kcminputrc` eszközcsoportja.
   Ha egyik sem ad eredményt, a GUI megkérdezi, és addig az evdev-úton a bal/jobb gomb nem köthető.
5. **Váltás futás közben.** A libinput mintájára a CFA4 is csak akkor alkalmazza a kezesség- vagy csereváltást, ha nincs lenyomott gomb. Addig a régi leképezés érvényes, így nincs beragadt állapot.
6. **Profilbeállítás.** `hand: "system" | "right" | "left"`, globálisan és eszközönként felülírhatóan. A `"system"` a rendszer gombcseréjéből következtet a kézre. A séma szerinti hely a 8.3 közös sémadöntésének része.
7. **Beépített gesztuskészlet.** Van jobbkezes alap és annak balkezes tükörképe. A tükörkép az ergonomikus gesztusokat és az oldalsó gombok szerepét tükrözi, a térbelieket nem (4.1). Mindkettő csak olvasható beépített profil (F0 spec 8. fejezet), szerkesztéskor másolat készül.
8. **Más vezérlőfelületek (V-38).** A ShuttlePRO és a MIDI-eszközök gombjait a CFA4 sorszám, illetve vezérlőazonosító szerint köti (`control_surfaces.rs` 185–190. sor; `crates/surface/src/hid_in.rs` 39–46. sor). Kezesség-kezelés nincs, balkezes használatnál ma a gombok átkiosztása segít. A gesztusterv ezen nem változtat. A „kéz” beállítás későbbi kiterjesztése a többi felületre a CFA4-CONTROL-SURFACE-001 döntése. A FlowMouse-ban sincs kezesség-beállítás (`rg -i 'left.?hand|mirror' js _locales/en` → nincs ilyen funkció).

9. COMMAND CHAIN INTEGRATION [JAVÍTVA: V-06]

A CFA4-ben ma nincs „command orchestration” vagy parancslánc-rendszer. A forráskódban nincs ilyen (a `command.?chain|orchestrat` keresés csak egy egui benchmarkot talál), és a leképezés egy vezérlőhöz egy parancsot rendel. A HW-CONTROL-SURFACES-001 makróigénye nyitott közös döntés (illesztés E8).

Ezért:

- Az első kiadásban a gesztus pontosan egy parancsot indít (Single command), a meglévő `surfaceAction` úton.
- A többparancsos kötés (Sequential command chain, Configurable sequence, Cancellation, Timeout, Error handling, Context revalidation, Execution feedback, Permission checking, Logging) csak azután jöhet, hogy a közös makró- vagy láncmodellről a tulajdonos a CFA4-CONTROL-SURFACE-001 és a HW-CONTROL-SURFACES-001 keretében döntött. A modell minden bemenetre (MIDI, HID, gesztus) közös lesz.
- Tilos külön script vagy automation engine létrehozása a Gesture Control számára.
- A Gesture Control nem hozhatja létre ezt a modellt.

10. WHEEL GESTURES

Támogatandó:

- Wheel Up
- Wheel Down
- Wheel Left
- Wheel Right
- Modifier + Wheel
- Button + Wheel
- Button combination + Wheel
- Context-dependent wheel
- Gesture sequence + Wheel

A normál scroll működése nem sérülhet.

A wheel gesture csak explicit konfigurációval aktiválódjon.

[ÚJ: V-12] A meglévő módosító + görgő kiosztások (pl. idővonal-zoom) ütközését a GUI jelezze, a billentyűparancs-ütközések meglévő kezelésének mintájára (`shortcuts.rs` `conflicts`).

11. ROCKER GESTURES

Támogatandó:

- Két egérgomb kombinációja
- Gombnyomási sorrend
- Konfigurálható timeout
- Konfigurálható aktiválási mód
- Kontextusfüggő rocker mapping
- Gesture + rocker kombináció

A rocker felismerés nem okozhat Qt6 inputkonfliktust vagy normál egérfunkció-vesztést.

[ÚJ: V-14] A bal + jobb rocker a libinput középsőgomb-emulációjával ütközhet (6. fejezet). A felhasználói felület ezt jelezze, MG0 mérje.

[ÚJ: V-34] A rocker a logikai gombokon értelmezett (pl. „másodlagos lenyomva + elsődleges kattintás”), így balkezes beállításnál ugyanazt a mozdulatot a tükörkéz végzi. A sorrend és a timeout kezességtől független.

12. CONTEXT-AWARE DRAG

Támogatandó CFA4 objektumok:

- Media Asset
- Timeline Clip
- Effect
- Transition
- Title
- Subtitle
- Audio Asset
- Project Object
- Keyframe – későbbi fázis
- VFX Object – későbbi fázis

Lehetséges műveletek:

- Preview
- Open
- Import
- Favorite
- Details
- Context-specific action

Destruktív műveletek nem rendelhetők véletlen gesztushoz megerősítés és a meglévő biztonsági mechanizmusok nélkül.

[ÚJ: V-19] A `CommandSpec`-ben ma nincs „destruktív” vagy jogosultsági jelölő (`crates/engine/src/commands.rs` 23–36. sor). MG0-ban dönteni kell arról, mi számít destruktívnak. Javaslat: gesztushoz alapból csak a navigációs, nézet- és lejátszásparancsok rendelhetők. Más parancs kötése a GUI-ban figyelmeztetést ad, és a végrehajtás visszavonható legyen (a meglévő undo-történeten át).

13. AREA SELECTION

A natív CFA4 objektumokra alkalmazott area selection.

Elsődleges célterületek:

- Media Browser grid/list
- VFX catalog
- Effect library
- Project object list

Timeline area selection csak külön konfliktusvizsgálat után engedélyezhető.

Támogatandó:

- Select
- Deselect
- Multi-select
- Preview
- Import
- Bulk action
- Context action

A meglévő modulok kijelölési logikáját kell használni.

14. GESTURE HUD

Qt6-alapú natív overlay.

Funkciók:

- Gesture trail
- Direction indicators
- Recognition preview
- Action hint
- Execution feedback
- Error feedback
- Cancel feedback
- Configurable visibility
- Reduced animation
- DPI awareness
- Multi-monitor support
- [ÚJ: V-35] Kezességhez igazodó gombfeliratok

A HUD:

- Nem blokkolhatja az egérmutatót.
- Nem okozhat érzékelhető UI-lag-ot (mérhető határ: 22. fejezet, AC-05). [JAVÍTVA: V-22]
- Kikapcsolható legyen.
- Minimális erőforrás-használatú legyen.

[ÚJ: V-25] A HUD a CFA4 ablakán belüli, eseményt át nem vevő réteg legyen, ne külön felső szintű ablak. Waylandon a kliens nem helyezheti el szabadon a saját ablakait [MG0-ban igazolandó]. A „Multi-monitor support” így a CFA4 ablakainak monitorjaira vonatkozik.

[ÚJ: V-35] A HUD és a súgó a gombokat logikai szerepük és a felhasználó keze szerint nevezi meg (pl. jobbkezesnél „jobb gomb”, balkezesnél „bal gomb” a másodlagosra; vagy semlegesen „másodlagos gomb”). A tényleges felirat a 8.4 szerint megállapított szerepből jön, nem a fizikai kódból. A feliratok az `i18n/` katalógusokba kerülnek.

15. QT6 GESTURE CONTROL UI

A CFA4-CONTROL-SURFACE-001 meglévő GUI-jának bővítése: Settings ▸ Control Surfaces (magyarul „Vezérlők”, HW-illesztés Q5), Window ▸ Control Surfaces… / MIDI Monitor… / Learn Assignments… (`python/cfa4_qt/window_menu.py` 366–369. sor; `panels/settings.py` 82. sor). [JAVÍTVA: V-05, V-26]

Új funkciócsoport:

Mouse Gesture Control (a meglévő lap új füle; külön beállításablak nem készül) [JAVÍTVA: V-26]

Alfunkciók:

- Gesture Profiles
- Gesture Editor
- Gesture Learn
- Button Learn
- Detected Buttons
- Button Test
- Button Combination Editor
- Activation Configuration
- Gesture Mapping
- Wheel Mapping
- Rocker Configuration
- Context Rules
- HUD Settings
- Recognition Thresholds
- Device Profiles (csak a C úton; az A/B úton egyetlen „mutató” eszköz van, lásd 5.1) [JAVÍTVA: V-09]
- [ÚJ: V-31] Kezesség (kéz: követi a rendszert / jobbkezes / balkezes; eszközönként; vízszintes tükrözés; evdev-csere kalibrálása)
- Import/Export
- [ÚJ: V-10] Input Access: a bemeneti út választása (alapértelmezés: A/B, alkalmazásszint), és az evdev-út kifejezett, eszközönkénti bekapcsolása figyelmeztetéssel

A GUI dinamikus gomblistát használjon, ne fix 3/5/8 gombos sablont. Minden új felirat az `i18n/` katalógusokba kerül (CFA4-QT6-FINISH-001, fordítási szabály). [JAVÍTVA: V-26]

16. INPUT KONFLIKTUS ÉS VÉDELEM

A rendszernek felismernie és védenie kell az alábbi helyzeteket:

- Timeline clip drag
- Clip trim
- Keyframe editing
- Text selection
- Text input
- Scrollbar
- Resize
- Normal click
- Normal double click
- Qt6 widget interaction
- File dialog
- Modal dialog
- Focus loss
- Input cancellation
- Escape
- Device disconnect
- Stuck button state
- [ÚJ: V-30] Dupla vagy elmaradt bal/jobb csere a Qt- és az evdev-út között
- [ÚJ: V-34] Kezesség- vagy rendszercsere-váltás lenyomott gomb mellett
- [ÚJ: V-12] Helyi menü (jobb gomb lenyomásra nyíló menük)
- [ÚJ: V-13] Középső gombos pásztázás és fülbezárás
- [ÚJ: V-11] Ugyanaz a gomb két úton (Qt és evdev)
- [ÚJ: V-10] Az evdev-úton érkező esemény, amikor a CFA4 ablaka nem aktív: ilyenkor nem indulhat parancs

Konfliktus esetén a gesture execution blokkolható.

A Gesture Control nem sértheti meg a meglévő modulok normál inputviselkedését.

A többgombos kombinációk csak explicit, érvényes konfiguráció alapján aktiválódjanak.

[ÚJ: V-05] Leválasztáskor a lenyomva maradt gombokhoz szintetikus felengedés megy az L4-be (F0 spec 9. fejezet 6. pont); ez a mutatóeszközökre is vonatkozik, és a Qt fókuszvesztésére is.

17. TIMELINE GESTURE PROFILE (kontextus, lásd 8.1) [JAVÍTVA: V-18]

Példaképességek:

- Previous edit point
- Next edit point
- Timeline zoom-in
- Timeline zoom-out
- Fit timeline
- Jump to timeline start/end
- Previous/next track
- Playback control
- Navigation

A gesztusok nem módosíthatnak automatikusan clipet, keyframe-et, tracket vagy projektadatot explicit mapping nélkül.

A felhasználó szabadon konfigurálhatja a gombokat, kombinációkat és mozdulatokat.

[ÚJ: V-05] Csak a regisztrált parancsok köthetők (a menük és billentyűk is a `command.list`-ből épülnek, ADR 0012, 22. sor); új parancs a Gesture Control miatt nem készül, kivéve, ha egy képességhez nincs parancs, és ezt MG0 kimutatja.

18. MEDIA BROWSER PROFILE (kontextus) [JAVÍTVA: V-18]

Példaképességek:

- Previous folder
- Next folder
- Previous/next asset
- Preview
- Import
- Favorite
- Details
- Zoom
- Context navigation

A gombok és gesztusok teljesen konfigurálhatók.

19. EFFECTS / VFX PROFILE (kontextus) [JAVÍTVA: V-18]

Példaképességek:

- Effect navigation
- VFX navigation
- Preview
- Favorite
- Open panel
- Context navigation
- Explicit mapped effect application

Effect alkalmazása csak explicit felhasználói mapping alapján történhet.

20. AUDIO PROFILE (kontextus) [JAVÍTVA: V-18]

Példaképességek:

- Audio track navigation
- Mute
- Solo
- Mixer panel
- Audio preview
- Context navigation
- Custom audio commands

A MIDI továbbra is a precíz fizikai audio-control egyik fő inputrendszere marad. A Gesture Control kiegészíti, nem helyettesíti.

21. FEJLESZTÉSI FÁZISOK [JAVÍTVA: V-29 – fázis-előtag MG, mert a G-azonosítók foglaltak]

[ÚJ: V-29] A fázisok előtagja MG (Mouse Gesture): MG0–MG6. A „G1…G5” azonosítók a `docs/gaps.md`-ben foglaltak (40–103. sor), és létezik `CFA4-PARITY-G7` terv is. A stáblista-tervnél ugyanígy jártak el (TC-F, `CFA4-TITLE-CREDITS-001-1.0-CFA4-ILLESZTES.md` 276. sor).

[ÚJ: V-21] Belépési feltételek (MG0 előtt mind teljesül, és mindegyikhez István külön engedélye kell):

1. A PostgreSQL-munka (CFA4-DATA-PG-001 jóváhagyott része) lezárult. Tulajdonosi döntés: a programfejlesztés csak a PostgreSQL után indul (a döntés a beszélgetésben hangzott el, a repóban nincs rögzítve).
2. A CFA4-QT6-FINISH-001 kapui: a `qt-parity.md` 1.9, 1.10 és 5.5 sora ✅ (ma 🟡). Ez az ADR 0012 12. döntésének surface-kapuja is.
3. A szülő CFA4-CONTROL-SURFACE-001 fizikai eszközös igazolása: az M1–M10 mérés kitöltve (ma üres, `docs/hardware-parity.md` 73. sor), az F1 elfogadva, az F2 (beépített MIDI-profilok) a szülőterv szerint lezárva.
4. A 28. fejezet elsődleges prioritásai stabilak. Ezek közül az OSC és a Mackie/HUI ma nincs kész; István dönti el, hogy a gesztus megvárja-e őket.
5. A CFA4 első kiadása: a gesztusvezérlés nem szerepel az első kiadás feltételei között, ezért javasolt az első kiadás utánra ütemezni (István dönt).

MG0 – Architecture & Reference Audit (csak olvasó, kódot nem változtat) [JAVÍTVA: V-15]

Feladat:

- A meglévő CFA4-CONTROL-SURFACE-001 motor auditálása (`crates/surface-model`, `crates/surface`, `crates/engine/src/control_surfaces.rs`, `surface_runtime.rs`, `surface_profile.rs`, `apps/cfa4-cli/src/serve_surface.rs`, `python/cfa4_qt/panels/settings.py`). [JAVÍTVA: V-05]
- [ÚJ: V-05] Az ADR 0012 eltérésének rögzítése: az ADR `crates/engine/src/surface/` modult ír, a kód lapos fájlokban van (`control_surfaces.rs`, `surface_*.rs`).
- Mapping rendszer vizsgálata.
- Parancsút vizsgálata (`surfaceAction` → `run_surface_action`). [JAVÍTVA: V-05]
- Event Fabric: annak rögzítése, hogy a gesztusesemény nem kerül rá (ADR 0012, 11. döntés). [JAVÍTVA: V-07]
- Qt6 inputintegráció vizsgálata (alkalmazásszintű eseményszűrő, a meglévő `mousePressEvent`-kezelők és helyi menük listája).
- Linux input képességeinek auditálása.
- Wayland korlátozások mérése a referenciagépen (KDE Plasma 6.6.7): mely gombok jutnak el a Qt-ig, elnyel-e a KWin gombot, libinput középsőgomb-emuláció. [JAVÍTVA: V-08, V-14]
- Extra egérgombok hozzáférhetőségének vizsgálata útvonalanként (A/B/C), legalább egy 8+ gombos és egy billentyű-interfészt is bejelentő egérrel.
- Dinamikus Button ID kezelés meghatározása.
- Fix gombszám-korlátozások felderítése (pl. `hid.rs` `KNOWN` tábla `u8` gombszáma, `control.py` 82. sor: a vezérlőcsatorna csak bal/jobb/középső gombot ismer).
- FlowMouse viselkedési elemzése (nem kódátvétel); licencjegyzet. [JAVÍTVA: V-15, V-16]
- Saját implementáció tervezése.
- Függőségek és kompatibilitás vizsgálata (új crate csak a `deny.toml` és a `licensing.md` szerint).
- [ÚJ: V-10] Biztonsági döntés az evdev-útról: jogosultságmodell (udev-szabály vagy semmi), a billentyű-interfészek kezelése, a Flatpak `--device=input` igénye.
- [ÚJ: V-17] Profilséma-döntés (additív vagy v2).
- [ÚJ: V-19] A gesztushoz köthető parancsok köre.
- [ÚJ: V-22] Az AC-k mérhető küszöbeinek véglegesítése.

Gate MG0: az architektúra, inputhozzáférés, dinamikus gombmodell és integrációs interfészek dokumentált és István által jóváhagyott állapota; ADR-módosítás (ADR 0012 kiegészítése vagy új ADR) jóváhagyva. [JAVÍTVA: V-26]

MG1 – Native Gesture Core

- Input capture
- Input normalization
- Dynamic button registry
- Button state
- Button Learn backend
- Gesture recording
- Gesture filtering
- Gesture recognition
- Direction detection
- Thresholds
- Timeout
- Cancel
- Confidence
- Multi-button input handling

Gate MG1: determinisztikus gesture recognition és dinamikus gombkezelés, rögzített mintakészleten CI-ben igazolva. [JAVÍTVA: V-23]

MG2 – Control Surface Integration

- Pointer/gesztus inputforrás a `SurfaceProtocol`/`SurfaceInput` modellben [JAVÍTVA: V-05]
- Input adapter
- Existing mapping integration
- Existing dispatch integration (`surfaceAction`)
- Profile integration
- [JAVÍTVA: V-07] Event Fabric: nincs bekötés (csak külön döntéssel audit)
- Command feedback

Gate MG2: minden gesture a közös CFA4 vezérlési útvonalon fut, és a meglévő MIDI/ShuttlePRO tesztek változatlanul zöldek.

MG3 – Qt6 Gesture UI

- Gesture Editor
- Gesture Learn
- Button Learn
- Button Test
- Detected Buttons
- Dynamic button list
- Combination Editor
- Profile management
- Mapping
- HUD
- Threshold configuration
- Import/export
- [ÚJ: V-10] Input Access beállítás

Gate MG3: teljes konfigurálhatóság a natív CFA4 UI-ból; a `qt-parity.md` új sorai.

MG4 – Timeline & Media Browser

- Timeline profile
- Media Browser profile
- Context resolver
- Input conflict protection
- Multi-button mapping
- Regression testing

Gate MG4: működő integráció, a hagyományos egérvezérlés regressziója nélkül.

MG5 – Advanced Gestures

- Wheel gestures
- Rocker gestures
- Multi-button gestures
- Context-aware drag
- Area selection
- Effects
- VFX
- Audio

Gate MG5: konfliktusmentes, konfigurálható advanced input.

MG6 – Validation & Stabilization

- Unit tests
- Recognition tests
- Button tests
- Combination tests
- Qt6 integration tests
- Wayland validation
- Input conflict tests
- Performance tests
- Accessibility
- Persistence
- Device disconnect tests
- Regression tests

Gate MG6: valamennyi elfogadási kritérium teljesül, bizonyítékkal alátámasztva.

[ÚJ: V-21] Minden fázis külön PR-sorozat, külön engedéllyel. „Kész” csak a `main`-en lévő, zöld CI-jú állapot. A szülőterv kódját érintő változás (modellbővítés) a CFA4-CONTROL-SURFACE-001 jóváhagyásával készül.

22. ACCEPTANCE CRITERIA [JAVÍTVA: V-22 – mérhető küszöbök]

A küszöbök javasolt kezdőértékek; MG0/MG1 mérése alapján István véglegesíti őket. A teljesítménycél a szülőterv mérési pontjához igazodik (motoroldali p95 ≤ 10 ms, F0 spec 214. sor).

| AC | Feltétel | Mérés / bizonyíték (javaslat) |
|---|---|---|
| AC-01 – Gesture Recognition | a 8 irány és a felhasználói gesztusok felismerése | rögzített szintetikus mintakészleten (MG1) ≥ 95% helyes; ≤ 1% téves egyezés a „nem gesztus” mintákon; determinisztikus |
| AC-02 – Unified Control Surface Integration | egy úton fut | minden gesztus `surfaceAction`-ként jelenik meg; nincs más végrehajtási út (kódellenőrzés + teszt) |
| AC-03 – Qt6 Configuration UI | minden beállítás a GUI-ból | a 15. fejezet minden alfunkciójához `qt-parity.md` sor és teszt |
| AC-04 – Input Conflict Protection | a 16. fejezet minden helyzete | helyzetenként automatizált teszt; a meglévő Qt GUI tesztek zöldek |
| AC-05 – Performance | nincs érzékelhető késés | felengedéstől `surfaceAction`-ig p95 ≤ 10 ms a motorban (a szülő mércéje); a HUD képkockaideje ≤ 16 ms; üresjáratban 0 többlet-CPU; a referenciagépen mérve |
| AC-06 – Ubuntu 26.04 / Wayland Validation | működik a referenciagépen | MG0 útvonaltáblája kitöltve; `test_qt_gui_on_wayland` mintájú CI-teszt (weston headless) + kézi jegyzőkönyv KDE Plasmán |
| AC-07 – Safety and Cancellation | Esc és timeout megszakít | teszt: megszakított gesztus nem indít parancsot |
| AC-08 – Extended Button Support | CFA4-oldali limit nincs | kódellenőrzés: nincs fix gombszám; útvonalanként dokumentált tényleges határ; a C úton tetszőleges `EV_KEY` kód tesztben |
| AC-09 – Dynamic Button Learn | Learn parancs nélkül | teszt: Learn alatt 0 `surfaceAction` |
| AC-10 – Multi-button Combination Support | kombinációk | 2 és 3+ gombos kombináció teszt; determinisztikus feloldás |
| AC-11 – Device-specific Profiles | eszközönkénti profil | csak a C úton; teszt két eszközzel (szintetikus) |
| AC-12 – Unknown Button Handling | ismeretlen gomb | nem okoz hibát, megjelenik a Button Testben, nem indít parancsot |
| AC-13 – Input Capability Detection | képesség-felderítés | a C úton a bejelentett kódok listája; az A/B úton a Qt által adott határ jelzése |
| AC-14 – Button State Safety | nincs beragadt gomb | leválasztás és fókuszvesztés után szintetikus felengedés (teszt) |
| AC-15 – No Regression | MIDI/HID/ShuttlePRO | a `crates/surface`, `crates/engine` surface-tesztjei és a `python/tests/test_qt_control_surfaces.py` változatlanul zöld; OSC-regresszió akkor, ha az OSC elkészült |
| [ÚJ: V-10] AC-16 – Input Privacy | nincs globális lehallgatás | evdev-út csak bekapcsolva és csak a kiválasztott eszközön; aktív CFA4-ablak nélkül nem indul parancs; nyers esemény nem kerül naplóba vagy fájlba |
| [ÚJ: V-24] AC-17 – Accessibility | gesztus nélkül is minden elérhető | minden gesztushoz köthető parancs menüből és billentyűvel is elérhető; állítható küszöbök és időkorlátok; a HUD csökkentett animációval és nagy kontraszttal |
| [ÚJ: V-36] AC-18 – Kezesség (bal- és jobbkezes egyenértékűség) | a két kéz ugyanazt kapja | (1) A rendszercsere (ki/be) × út (Qt/evdev) négy esetében az elsődleges és a másodlagos fizikai gomb 100%-ban ugyanazt a logikai szerepet kapja; dupla vagy elmaradt csere 0. (2) Tükrözött profillal a balkezes és az eredeti jobbkezes beállítás a szintetikus mintakészleten (AC-01, AC-04, AC-07, AC-10 tesztjei) 100%-ban azonos parancssorozatot ad. (3) A térbeli gesztusok tükrözéskor sem változnak. (4) A HUD és a súgó feliratai a szerep szerint helyesek (teszt). (5) Kézi jegyzőkönyv a referenciagépen (KDE, `LeftHanded` be/ki). |

Minden kritériumhoz teszt és dokumentált bizonyíték szükséges.

23. TESZTMÁTRIX

Kötelező tesztelési kategóriák:

- 3 gombos egér
- 5 gombos egér
- 8 gombos egér
- 12+ gombos egér
- Nagy gombszámú gamer egér
- Nem szabványos gombazonosítók
- [ÚJ: V-08] Egér, amely a plusz gombokat billentyűként küldi (külön billentyű-interfész)
- Button Learn
- Button Test
- Press
- Release
- Hold
- Kétgombos kombináció
- Három vagy több gomb kombinációja
- Button + gesture
- Button + wheel
- Button + modifier
- Rocker
- [ÚJ: V-14] Rocker bekapcsolt libinput középsőgomb-emulációval
- Device hotplug
- Device reconnect
- Profilváltás
- [ÚJ: V-36] Balkezes rendszerbeállítás, Qt-úton (a gombcsere nem duplázódik)
- [ÚJ: V-36] Balkezes rendszerbeállítás, evdev-úton (kalibráció, kézi beállítás, rendszerbeállítás olvasása)
- [ÚJ: V-36] Rendszercsere-váltás lenyomott gomb mellett
- [ÚJ: V-36] Tükrözött profil importja és exportja (felajánlott tükrözés, előnézet)
- [ÚJ: V-36] Beépített balkezes gesztuskészlet; térbeli gesztusok tükrözés nélkül
- [ÚJ: V-36] Szimmetrikus és balkezes egér oldalsó gombjai (hely szerinti elnevezés)
- [ÚJ: V-36] Rocker és kombináció logikai gombokon, mindkét kézzel
- Fel nem ismert gomb
- Konfliktusos input
- Learn mód parancsvégrehajtás nélkül
- Wayland input
- [ÚJ: V-08] X11 input (a `qt-parity.md` 1.9 szerint mindkettő cél)
- Hosszú használat
- CPU overhead
- RAM használat
- Recognition latency
- UI response
- HUD overhead

A 12+ gombos teszt nem a maximális támogatott gombszám meghatározására szolgál. A cél a fix korlát nélküli architektúra bizonyítása és a tesztelt eszközök kompatibilitásának dokumentálása.

A végleges teljesítményhatárokat MG0/MG1 során kell meghatározni és dokumentálni.

[ÚJ: V-23] Tesztelhetőség CI-ben, hardver nélkül:
- Felismerő: tiszta Rust egységtesztek rögzített mintasorozatokon (determinisztikus).
- Motor: a meglévő `surface.inject` ugyanazon az úton fut, mint az eszközesemény (`surface_runtime.rs` 3. sor); a gesztusbemenetre is ki kell terjeszteni.
- Qt: szintetikus egéresemények `QT_QPA_PLATFORM=offscreen` alatt és weston headless Waylanddel (`.github/workflows/ci-linux.yml` 126. sor; `qt-parity.md` 1.9). A vezérlőcsatorna `ui.click`/`ui.drag` ma csak bal, jobb és középső gombot ismer (`python/cfa4_qt/control.py` 82. sor); bővíteni kell az extra gombokkal.
- evdev-út: a dekóder bájtszintű tesztje (mint a mai `EvdevDecoder`-é); `uinput`-os teszt csak helyben, opcionálisan, nem kötelező CI-kapu (`/dev/uinput` írási jog kell, a GitHub-futókon nem igazolt).
- Fizikai eszközös mérés: jegyzőkönyv a referenciagépen, a szülő M1–M10 sablonjának mintájára.

24. PRIVACY ÉS LOCAL-FIRST

- Offline működés.
- Nincs telemetria.
- Nincs cloud.
- Nincs folyamatos felhasználói mozgásprofilozás.
- Nincs automatikus gesture adatküldés.
- Raw input minták csak átmenetileg kezelhetők.
- Mentés kizárólag felhasználói konfiguráció alapján.
- Profilok lokálisan tárolódnak.
- A felhasználó törölheti saját gesture profiljait.
- Inputadat nem kerül automatikusan felhőbe.
- [ÚJ: V-10] A Monitor/Button Test nyers eseményei csak a memóriában élnek, és nem kerülnek naplóba.
- [ÚJ: V-10] Az evdev-út soha nem olvas olyan node-ot, amelyet a felhasználó nem választott ki. Billentyűzetként bejelentett node-ot csak kifejezett, külön figyelmeztetett engedéllyel olvas, és onnan is csak a megtanult kódokat dolgozza fel.
- [ÚJ: V-10] Az evdev-útról érkező esemény csak akkor indíthat parancsot, ha a CFA4 ablaka aktív (a Qt jelzi a motornak).

25. TECHNOLÓGIAI KORLÁTOZÁSOK

Cél:

- Linux
- Ubuntu 26.04
- amd64
- Qt6 (PySide6) [JAVÍTVA: V-03]
- Rust
- CFA4 natív inputintegráció
- CPU-alapú felismerés
- Offline

Nem cél:

- Windows
- macOS
- Android
- Browser extension
- DOM input
- Global desktop gesture engine
- KDE/GNOME rendszerszintű gesztusvezérlés az első kiadásban
- GPU gesture recognition
- Cloud gesture recognition
- AI-alapú gesture felismerés
- Második command engine
- Második mapping engine
- Saját automation engine
- [ÚJ: V-04] A Qt beépített `QGestureRecognizer` keretrendszere a felismerésre: azzal a felismerés a Qt-ban (Pythonban) futna, ami ütközik az ADR 0012 1. döntésével
- [ÚJ: V-10] `uinput`-os eseménykeltés és EVIOCGRAB
- [ÚJ: V-08] Gyártói protokollok (HID++, libratbag) az első kiadásban

A konkrét crate-ek, API-k és input backendek kiválasztása az MG0 audit feladata. [ÚJ: V-27] Kötöttségek: evdev csak L5-ben, `unsafe` nélkül; az `evdev`/`udev` crate L5 alatt tiltott (ADR 0012, 78–79. sor); új függőség csak `deny.toml`-megfelelőséggel.

26. KOCKÁZATOK

- Véletlen gesture aktiválás
- Timeline input konfliktus
- Qt6 input események duplikációja [JAVÍTVA: V-11 – konkrétan: Qt és evdev ugyanarra a gombra, mert kizárólagos foglalás (grab) nem lesz]
- Wayland inputkorlátozások [JAVÍTVA: V-08, V-09 – a Qt Waylanden legfeljebb 16 gombot ad át, eszközazonosító nélkül]
- Egyes gyártók extra gombjai nem hozzáférhetők
- Dinamikus Button ID hibás kezelése
- Gombkombinációk ütközése
- Input state beragadás
- Hotplug utáni profilkeveredés
- HUD UI-zavar
- Gesture false positive
- CPU overhead
- [JAVÍTVA: V-16] Licenc: a FlowMouse GPL-3.0 licencű, a CFA4 GPL-3.0-or-later. Ez kompatibilis (`licensing.md` 9–11. sor; előzmény: a Kdenlive GPL-3.0-only kódja, ADR 0012 63–65. sor). Ha kódot vennénk át, az a rész GPL-3.0 alatt maradna, és nyilvántartást kérne. Mivel kódátvétel nem tervezett, a kockázat alacsony. Az AGPL-szabály (#1561) nem érinti.
- MIDI/HID/OSC regresszió
- Normál egérfunkciók sérülése
- [ÚJ: V-30] Dupla vagy elmaradt bal/jobb csere a két bemeneti út között (balkezes felhasználóknál téves parancs)
- [ÚJ: V-30] A rendszer kezesség-beállítása nem olvasható (ismeretlen asztali környezet, Flatpak-homokozó): ilyenkor a kalibráció vagy a kézi beállítás dönt
- [ÚJ: V-10] Biztonság: az egér evdev-node-jának olvasása az egész munkamenetre kiterjedő, fókusztól független bemenet. A systemd alap `uaccess`-szabálya egeret és billentyűzetet nem ad a felhasználónak, csak joystickot. Az `input` csoporttagság minden billentyűzetet is olvashatóvá tenne, ami billentyűnaplózásra alkalmas jogosultság. Sok egér a plusz gombjait billentyű-interfészen küldi.
- [ÚJ: V-14] libinput középsőgomb-emuláció (bal + jobb → középső)
- [ÚJ: V-12] A jobb gombos helyi menü lenyomásra nyílik
- [ÚJ: V-10] Flatpak-csomagban az evdev-úthoz `--device=input` jogosultság kellene
- [ÚJ: V-21] Ütemezési kockázat: a szülőrendszer fizikai igazolása még hiányzik

Minden kockázathoz MG0/MG6 alatt validált megoldás és teszt szükséges.

27. OUT OF SCOPE

Az első fejlesztési kiadásban nem tartozik a tervbe:

- Globális Linux desktop gesture kezelés.
- KDE/GNOME rendszerintegráció.
- Browser gesture.
- DOM manipuláció.
- Második control engine.
- Második mapping engine.
- Saját script engine.
- Saját automation engine.
- AI gesture recognition.
- GPU recognition.
- Cloud.
- Telemetria.
- Destruktív műveletek véletlen gesture alapján.
- [ÚJ: V-06] Többparancsos lánc (a közös makródöntésig).
- [ÚJ: V-10] Eseménykeltés `uinput`-tal; az eszköz kizárólagos lefoglalása.
- [ÚJ: V-08] Gyártói protokollok (HID++, libratbag).

28. CFA4 ROADMAP INTEGRATION

A Gesture Control fejlesztése nem előzheti meg a szülő CFA4-CONTROL-SURFACE-001 alapvető stabilitását. [ÚJ: V-21] A „stabilitás” mérhető jelentése a 21. fejezet belépési feltételei.

Elsődleges prioritás marad:

- MIDI Learn
- MIDI Profiles
- HID Mapping
- OSC (ma nincs kész) [JAVÍTVA: V-05]
- ShuttlePRO (fizikai mérés hiányzik) [JAVÍTVA: V-21]
- Control Surface Profiles
- [JAVÍTVA: V-07] Event Fabric: a surface-eseményekre nem vonatkozik (ADR 0012, 11. döntés); ez a sor törlendő vagy „csak audit, külön döntéssel” alakra hozandó

A Gesture Control későbbi, önálló fejlesztési workstreamként integrálható a CFA4 roadmapbe. [ÚJ: V-21] Javasolt sorrend: PostgreSQL → Qt6-befejezés → a szülő F1/F2 fizikai igazolása → első kiadás → MG0.

[ÚJ: V-26] Dokumentumhely: `docs/plans/CFA4-CONTROL-SURFACE-GESTURE-001-1.1.md`, a változáslista `docs/plans/CFA4-CONTROL-SURFACE-GESTURE-001-1.1-VALTOZASOK.md`. Később érintett dokumentumok: ADR 0012 (kiegészítés), `docs/hardware-parity.md` (egér/gesztus sor), `docs/qt-parity.md`, `docs/control-protocol.md`, `docs/licensing.md` (csak átvétel esetén).

29. ARCHITEKTURÁLIS DÖNTÉSEK

GDR-01 – Gesture Control új inputforrás a `SurfaceEvent` modellen. [JAVÍTVA: V-05]
GDR-02 – A meglévő leképezés használata (`ControlSurfacePrefs`/`Binding`). [JAVÍTVA: V-05]
GDR-03 – A meglévő parancsút használata (`surfaceAction` → Qt parancsút). [JAVÍTVA: V-05]
GDR-04 – Gesture Recognition elkülönített, tiszta Rust komponens (I/O nélkül). [JAVÍTVA: V-04]
GDR-05 – Elsődleges bemenet: Qt6 alkalmazásszint (A/B út); az evdev-út (C) opcionális, kifejezetten bekapcsolandó. [JAVÍTVA: V-09, V-10]
GDR-06 – Global desktop gesture nem része az első kiadásnak.
GDR-07 – Timeline és Media Browser elsődleges modulok.
GDR-08 – [JAVÍTVA: V-06] Command Chain: az első kiadásban nincs; később csak a CFA4-CONTROL-SURFACE-001 és a HW-CONTROL-SURFACES-001 közös makró-/láncmodelljén.
GDR-09 – Natív Qt6 HUD, az ablakon belül. [JAVÍTVA: V-25]
GDR-10 – FlowMouse csak viselkedési referencia; kódátvétel nem tervezett. [JAVÍTVA: V-15]
GDR-11 – MIDI/HID változatlan megőrzése; OSC, amikor elkészül. [JAVÍTVA: V-05]
GDR-12 – Offline, CPU-only működés.
GDR-13 – Destruktív művelet nem aktiválódhat véletlen gesture alapján; a köthető parancsok köre MG0-ban rögzítve. [JAVÍTVA: V-19]
GDR-14 – Regresszióteszt kötelező.
GDR-15 – Egérgombok dinamikus kezelése, CFA4-oldali hardcoded maximum nélkül. [JAVÍTVA: V-08]
GDR-16 – Button Learn és Button Test funkció, a meglévő Learn-gépre és Monitor-mintára építve. [JAVÍTVA: V-05]
GDR-17 – Többgombos kombinációk, a közös modell additív bővítésével. [JAVÍTVA: V-20]
GDR-18 – Eszközspecifikus egérprofilok csak az evdev-úton (Waylanden az alkalmazásszint nem különbözteti meg az egereket). [JAVÍTVA: V-09]
GDR-19 – A tényleges támogatást az input stack által elérhető képességek határozzák meg; ezt útvonalanként dokumentáljuk.
GDR-20 – [JAVÍTVA: V-08] A cél a választott bemeneti út által átadott összes gomb támogatása, CFA4-oldali mesterséges limit nélkül; a „felismerhető legmagasabb gombszám” útvonalfüggő.
[ÚJ: V-30] GDR-21 – Kezesség: a bal/jobb gomb logikai (elsődleges/másodlagos), a gombcsere mindig a rendszert követi, és a CFA4 nem cseréli meg még egyszer; a fizikai kód mellette tárolódik; a kéz (jobb/bal) csak a tükrözésre és a feliratokra hat; az idővonal időiránya nem tükrözhető; balkezes beépített tükörkészlet van.
[ÚJ: V-07; átszámozva: V-37] GDR-22 – A gesztusesemények nem kerülnek az Event Fabricre.
[ÚJ: V-10; átszámozva: V-37] GDR-23 – Nincs `uinput`-os eseménykeltés és nincs kizárólagos lefoglalás (grab).
[ÚJ: V-04; átszámozva: V-37] GDR-24 – A Qt-ban nincs felismerési vagy leképezési logika (ADR 0012, 1. döntés).
[ÚJ: V-24; átszámozva: V-37] GDR-25 – Gesztus soha nem az egyetlen út egy parancshoz.

30. STÁTUSZ [JAVÍTVA: V-01]

Terv: CFA4-CONTROL-SURFACE-GESTURE-001
Verzió: 1.1 FINAL
Jóváhagyás: Horváth István, 2026-11-23 („a terveket jóváhagyom véglegesítsd”)
Sokgombos egér támogatás: A terv része (útvonalfüggő határokkal)
Dinamikus Button Learn: A terv része
Multi-button kombináció: A terv része
Kezesség (jobb- és balkezes egyenértékűség): A terv része [ÚJ: V-30]
FlowMouse: viselkedési referencia
Architektúra: a CFA4-CONTROL-SURFACE-001 kiterjesztése
MG0: Nem indult
Implementáció: Nem indult, nem engedélyezett
Repository-módosítás: csak ez a tervdokumentum és a változáslista (`docs/plans/`); kód nem változott

A terv jóváhagyása nem jelent automatikus végrehajtási engedélyt. A további fejlesztési fázisok indítása külön engedélyezést és a CFA4 aktuális állapotának ellenőrzését igényli.

VÉGREHAJTÁSI UTASÍTÁS

A dokumentum egy jóváhagyott CFA4 fejlesztési terv. Felhasználható a CFA4 projekt tervezési dokumentációjához, későbbi fejlesztési feladat előkészítéséhez és implementációs agent számára történő átadásához.

A terv alapján semmilyen implementáció, kódmódosítás, repository-módosítás, telepítés vagy fejlesztési fázis nem indítható el automatikusan. A végrehajtás előtt ellenőrizni kell az aktuális CFA4 állapotot és az arra vonatkozó külön engedélyt.

VÁLTOZÁSTÖRTÉNET

| Verzió | Dátum | Állapot | sha256 | Megjegyzés |
|---|---|---|---|---|
| 1.0 FINAL | 2026-11-23 | jóváhagyva (Horváth István) | `c9a7f9d3ba3238e531c261fb602f59042530dd3068f0fa7deb5642f333c05a34` | eredeti terv; a repóba nem került |
| 1.1 JAVASLAT | 2026-11-23 | javaslat | `da1588eea091ecc3507f76a0e2b32096e910ae99547f4e526467960d51872e0e` | V-01–V-29 (elemzés a `main` `3cdd81bc` ellen), V-30–V-38 (kezesség, István kérésére); tételesen: `docs/plans/CFA4-CONTROL-SURFACE-GESTURE-001-1.1-VALTOZASOK.md` |
| 1.1 FINAL | 2026-11-23 | jóváhagyva (Horváth István: „a terveket jóváhagyom véglegesítsd”) | ez a fájl | a tartalom az 1.1 JAVASLAT-tal azonos; csak a verzió, a státusz, a jóváhagyás, a fájlhivatkozások és a 4.1 alapértékének jóváhagyási jelölése változott |

A [JAVÍTVA: V-xx] és [ÚJ: V-xx] jelölések az 1.0-hoz képesti változásokat mutatják. A szövegben maradt „javaslat” szavak a kidolgozás menetét őrzik; a jóváhagyás a teljes szövegre vonatkozik. Ahol a terv MG0-döntést vagy István külön döntését írja elő, az nyitott marad.
