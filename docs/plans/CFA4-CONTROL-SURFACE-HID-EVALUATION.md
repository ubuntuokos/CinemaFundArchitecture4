# CFA4-CONTROL-SURFACE-001 – HID-hozzáférés értékelése (ShuttlePRO)

**Dátum:** 2026-11-13 · **Állapot:** asztali (forrás- és dokumentációalapú) értékelés kész; a fizikai mérés
(4. fejezet) a referenciagépen az F1a első lépése · **Lefedi:** F0-AC-03, T0.2 · **Célgép:** a repó
`docs/development/reference-machine.md` fájlja (Ubuntu Studio 26.04, kernel 7.0.0-43-generic, KDE Plasma 6.6.6, Wayland)

## 1. Eredmény

| | Megoldás | Új crate |
|---|---|---|
| **Elsődleges** | `hidraw`: `/dev/hidrawN` olvasása `std::fs::File`-lal, azonosítás és riportleíró a sysfs-ből (`/sys/class/hidraw/hidrawN/device/uevent`, `…/report_descriptor`), hot-plug `inotify`-jal a `/dev`-en | nincs (`rustix` 1.1.6 már a lockfile-ban, `fs` funkcióval adja az inotify-t) |
| **Tartalék** | `evdev`: `/dev/input/eventN` olvasása `std::fs::File`-lal, a 24 bájtos `input_event` rekordok saját, `unsafe` nélküli feldolgozásával; azonosítás a sysfs-ből | nincs (az `evdev` crate 0.13.2 csak akkor, ha ioctl kell, lásd 3. fejezet) |
| Elutasítva | hidapi, libusb/rusb, közvetlen ioctl saját `unsafe` kóddal, `udev`/`libudev-sys` | – |

Indoklás röviden: a hidraw a ShuttlePRO teljes riportját adja, benne a shuttle-gyűrű 0 állását is,
így az evdev középállás-hibája nem jelentkezik; sem C-könyvtár, sem `unsafe` nem kell, és a kernel
saját HID-meghajtója nem válik le az eszközről. Az evdev a Kdenlive (`media_ctrl`) útja, ezért tartaléknak jó,
de a középállást csak kikövetkeztetni tudja.

## 2. Jelöltek összevetése (5.4. pont 1–12.)

| # | Szempont | hidraw (std + sysfs) | evdev (std + saját parser) | `evdev` crate 0.13.2 | hidapi (`hidapi` crate) | libusb (`rusb`) |
|---|---|---|---|---|---|---|
| 1 | Projekt, verzió | Linux kernel ABI | Linux kernel ABI | `evdev` 0.13.2 (2025-09-15) | hidapi + C `libhidapi` | `rusb` + C `libusb-1.0` |
| 2 | Licenc | – (kernel-ABI) | – | Apache-2.0 OR MIT | BSD/GPL-3/egyedi (C), MIT (crate) | LGPL-2.1+ (C), MIT (crate) |
| 3 | Linux-támogatás | igen, minden kernel | igen | igen | igen | igen |
| 4 | Rust API / FFI | `std::fs`, nincs FFI | `std::fs`, nincs FFI | biztonságos API, belül `nix` 0.29 ioctl | FFI C-könyvtárhoz | FFI C-könyvtárhoz |
| 5 | Eszközazonosítás | `HID_ID=0003:00000B33:00000030` a uevent-ben, példány a `HID_PHYS`-ből | sysfs `id/vendor`, `id/product`, `phys` | ioctl `EVIOCGID` | enumerálás | USB-leíró |
| 6 | Gombok | riport-bitmaszk **[mérendő]** | `EV_KEY` | `EV_KEY` | riport | riport, de a kernel meghajtóját le kell választani |
| 7 | Shuttle | riport 0. bájt, 0 is jön **[mérendő]** | `REL_WHEEL`, 0 nem jön | ugyanaz | riport | riport |
| 8 | Hot-plug | inotify `/dev` (`IN_CREATE`/`IN_DELETE`) + sysfs | ugyanaz | ugyanaz | nincs beépített | `libusb` hotplug |
| 9 | Jogosultság | udev `TAG+="uaccess"` a `hidraw` alrendszerre | `uaccess` az `input` alrendszerre | ugyanaz | mint hidraw | `uaccess` az `usb` eszközre, meghajtóleválasztás |
| 10 | CPU, memória | eseményvezérelt blokkoló olvasás, ~0 **[mérendő]** | ugyanaz | ugyanaz | hasonló | hasonló |
| 11 | Korlát | a riportformátum eszközönként eltérhet (v1/v2/Xpress) | középállás-hiba; a desktop is megkapja az eseményeket | +`nix` 0.29 második verzió, +`bitvec` lánc | C-függőség | a többi alkalmazástól elveszi az eszközt |
| 12 | Döntés | **elsődleges** | **tartalék** | csak ha az `EVIOCGRAB` kell | elutasítva | elutasítva |

## 3. Részletek

**Az evdev középállás-hibája.** A kernel a 0 értékű relatív eseményt nem küldi el, ezért a shuttle
visszatérése középre csak abból látszik, hogy egy `SYN_REPORT`-csomagban nincs `REL_WHEEL`.
Ha közben a jog-számláló épp 0, ez a kikövetkeztetés is hibázik. Három független forrás írja le:
a Kdenlive `jogshuttle`, a shuttle-go `mapper.go` és a Shuttle Lander `ShuttleWorker` megjegyzése.

**Ha az evdev tartalék kell, de grab nélkül:** a CFA4 olvasása mellett a KDE is megkapja az eseményeket.
A ShuttlePRO `BTN_*` kódokat ad, nem billentyűket, így ez Wayland alatt várhatóan nem okoz mellékhatást
**[mérendő]**. Ha mégis, az `EVIOCGRAB` ioctl kell: ez vagy az `evdev` crate (új `nix` 0.29 és `bitvec`
függőség), vagy a `rustix` ioctl-je. A döntés a mérés után születik.

**Riportformátum (várt, mérendő).** ShuttlePRO v2: 5 bájtos bemeneti riport: `[shuttle i8, jog u8, 0, gombok lo, gombok hi]`.
A riportleíróból (sysfs `report_descriptor`) a hosszt és a mezőket ellenőrizni kell; v1 és Xpress külön mérendő.

## 4. Mérési protokoll a referenciagépen (F1a első lépése)

Minden jegyzőkönyv fejlécébe: `uname -r`, `dpkg -l linux-image-$(uname -r)`, `lsusb -v -d 0b33:` kivonat,
`lscpu` (a repó mérési szabálya szerint), a KDE Plasma és a PipeWire verziója.

| Lépés | Parancs / teendő | Elvárt eredmény |
|---|---|---|
| M1 Kötés | `ls -l /dev/hidraw* /dev/input/by-id/*Contour*`; `cat /sys/class/hidraw/hidraw*/device/uevent` | Van `hidraw` és `event` csomópont, `HID_ID` 0B33:0030 (0020, 0010) |
| M2 Jogosultság | `getfacl /dev/hidrawN` udev-szabály nélkül és a `uaccess` szabállyal | Szabály nélkül nem olvasható; vele a bejelentkezett felhasználó olvashatja, más nem |
| M3 Riportleíró | `xxd /sys/class/hidraw/hidrawN/device/report_descriptor` | Mezőhosszak megfeleltetése a 3. fejezettel |
| M4 Shuttle | `xxd -c5 /dev/hidrawN` tekerés közben, ±7 és vissza 0-ra | Minden pozíció és a 0 is megjelenik |
| M5 Jog | ugyanott, lassan és gyorsan, 255→0 átfordulással | A számláló körbefordul, nincs kihagyás |
| M6 Gombok | minden gomb külön és együtt | Lenyomás és felengedés egyaránt látszik |
| M7 evdev | `evtest /dev/input/eventN` | A középállás-hiba megjelenik vagy nem (a 7.0-s kernelen) |
| M8 Hot-plug | kihúzás és visszadugás olvasás közben; `inotifywait -m /dev` | `IN_DELETE`/`IN_CREATE`; az olvasás `ENODEV`-vel ér véget, újranyitás működik |
| M9 Desktop-mellékhatás | gombok KDE alatt CFA4 nélkül | Nincs nem kívánt művelet |
| M10 Terhelés | `pidstat` 60 s tekerés alatt (prototípus-olvasóval) | Elhanyagolható CPU |

A jegyzőkönyv sablonja a táblázat „Mért eredmény” oszloppal bővítve; a döntés (5.4. pont) a 12. sorral zárul.
Ha az M4 nem adja a 0-t, a tartalék és az elsődleges felcserélődik, és erről ADR-kiegészítés készül.

## 5. Források

- Kdenlive `src/jogshuttle/` (jogaction.cpp, jogshuttle.cpp, jogshuttle.h), GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL
- shuttle-go (MIT), <https://github.com/abourget/shuttle-go>, `mapper.go`
- Shuttle Lander (GPL-3.0), <https://gitlab.com/Robotista/shuttle-lander>
- Linux `Documentation/hid/hidraw.rst`, `Documentation/input/event-codes.rst`
- crates.io (2026-11-13): `evdev` 0.13.2 (Apache-2.0 OR MIT; `nix` ^0.29, `bitvec` ^1), `udev` 0.9.3 (MIT, `libudev-sys`), `inotify` 0.11.5 (ISC)
