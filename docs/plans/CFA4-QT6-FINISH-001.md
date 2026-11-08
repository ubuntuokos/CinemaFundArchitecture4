# CFA4-QT6-FINISH-001 – a Qt6 GUI befejezése és az egui eltávolítása

**Állapot:** TERVEZET 1.0 (2026-11-08), tulajdonosi jóváhagyásra vár. Fejlesztés csak a jóváhagyott
fázisokban indul. A 7. fejezet döntési pontjait a tulajdonos hozza meg.

**Kapcsolódó dokumentumok:** döntés és fázisok: [`docs/adr/ADR-F8-QT6-RUST-ENGINE-2026-10-23.md`](../adr/ADR-F8-QT6-RUST-ENGINE-2026-10-23.md);
pontonkénti lista: [`docs/qt-parity.md`](../qt-parity.md); technikai leírás: [`docs/qt-gui.md`](../qt-gui.md);
PR-szabályok: [`docs/coordination.md`](../coordination.md).

**A szabály, amelyhez ez a terv igazodik (tulajdonosi döntés, 2026-10-24):** az egui akkor törölhető,
ha a Qt6 GUI pontról pontra mindent tud, amit az egui (a `qt-parity.md` minden sora ✅), **és** a
tulajdonos ezt kifejezetten jóváhagyja, a jóváhagyás az ADR F8-ba kerül. Ez a terv ezen nem változtat.

## 1. Kiinduló állapot (`main`, 2026-11-08)

A `qt-parity.md` 63 sorából 59 ✅, 4 🟡, ❌ nincs.

| Sor | Mi hiányzik a sor szerint | Jellege |
|---|---|---|
| 1.9 Wayland/X11 minden Ubuntu 26.04 változaton | kézi ellenőrzés a valódi asztalokon (tört skálázás, beviteli módok); CI-ben X11 (Xvfb) és Wayland (weston) már fut | kézi teszt + javítások |
| 1.10 Csomagolás | Qt az AppImage-ben és a Flatpakban; az asztali bejegyzés még az egui-t indítja | fejlesztés + döntés (D1) |
| 4.2 Export mód | a sor nem nevezi meg (a táblázat szabálya szerint kellene) | tisztázás; nyitott: #764 |
| 5.5 GPU képút | GPU-feltöltés RGBA8-másolat nélkül (dmabuf / GL textúra) | fejlesztés + mérés |

Nyitott Qt-PR-ek, amelyek ✅-os sorokat finomítanak: #733 (#712, #716, #723, #727, #729, #731), #751, #764.
A beolvasztásuk a gazda dolga, ez a terv nem nyúl hozzájuk.

Dokumentációs eltérés a `qt-parity.md`-ben: az 1.11 sor a végén azt írja, hogy a sávszínt, a renderelési
sávot és a zoomsávot a Qt idővonal nem rajzolja; a 3.13 sor szerint mindhárom kész. Ezt a Qt-paritás
munkamenet javítja a saját sorában (közös fájl, `coordination.md` §2.5).

## 2. Fázisok

Minden fázis külön PR (vagy témánként több), friss `main`-ből, a gazdának átadva. Egy sor csak
bizonyítékkal lesz ✅ (teszt a `python/tests/`-ben vagy rögzített kézi ellenőrzés a referenciagépen).

### Q1 – Export mód lezárása (4.2)

1. A #764 (exportsor oszlopa az egui szerint) beolvasztása után a 4.2 sort a Qt-paritás munkamenet
   összeveti az egui `export_mode.rs`-ével, és vagy megnevezi a még hiányzó elemet, vagy ✅-ra állítja.
2. Ha marad hiány, az egy újabb, kis PR.

**Átvétel:** a 4.2 sor ✅, bizonyítékkal; vagy 🟡 a hiány megnevezésével.

### Q2 – Kézi asztali ellenőrzés (1.9)

A tulajdonos gépén (gépeken) fut, mert valódi asztal kell hozzá. Egy fejlesztő munkamenet előkészíti:

1. **Ellenőrzőlista** `docs/development/qt-desktop-checks.md` néven: asztalonként egy táblázat.
   Asztalok: a 26.04 hivatalos változatainak asztalai (a lista a kiadási jegyzék szerint ellenőrizendő),
   legalább GNOME (Ubuntu), KDE Plasma (Kubuntu), Xfce (Xubuntu), LXQt (Lubuntu), MATE, Budgie, Cinnamon;
   Wayland- és X11-munkamenetben is, ahol az asztal mindkettőt kínálja.
2. **Pontok asztalonként:** indulás; a tálca neve és ikonja (asztali bejegyzés); 100 / 125 / 150 / 200 %
   skálázás; több monitor eltérő skálázással; panelek dokkolása, lebegtetése, fülek; natív fájlpárbeszédek
   (XDG portál); húzás a fájlkezelőből; vágólap; beviteli mód (ibus és fcitx5: magyar ékezetek, egy CJK
   bevitel a Type eszközzel és a feliratoknál); hang lejátszás (PipeWire); billentyűparancsok fókusz nélkül.
3. **Segédszkript:** `cfa4-qt --smoke --screenshot` futtatása és a képernyőképek gyűjtése asztalonként,
   hogy a kézi munka rövidebb legyen.
4. A talált hibák javítása témánként külön PR-ban.

**Átvétel:** minden felsorolt asztal kitöltött sora a listában, a hibák javítva vagy a tulajdonos
döntésével elfogadva; ezután az 1.9 Qt-oszlopa ✅.

### Q3 – GPU képút (5.5)

A mai út: a motor RGBA8 képkockát ír egy osztott memóriás gyűrűbe, a Qt helyben megjeleníti
(1080p-n kb. 25 ms-mal gyorsabb, mint a csővezeték). A hiány a GPU-feltöltés.

1. **Mérés előbb** a referenciagépen (`docs/development/reference-machine.md`) és GPU nélkül: a monitor
   mostani rajzolási ideje 1080p-n és 4K-n, lejátszás közbeni kiesett képkockák (`perf.stats`).
2. **1. lépés:** textúrafeltöltés az osztott memóriából (QOpenGLWidget vagy QRhi), a QPainter-es
   rajzolás helyett. Nem kell hozzá motorváltozás.
3. **2. lépés (csak ha az 1. után is indokolt):** dmabuf-import (EGL) a motor hardveres dekódolásából,
   gyártófüggetlenül (VA-API / Vulkan; NVIDIA-út csak mellettük, `AGENTS.md`). Ez motorváltozással jár,
   és a `cfa4-route` routeren keresztül választ.
4. **Kötelező CPU-tartalék:** ha a GPU-út nem érhető el (nincs GL, `offscreen`, hiba), a mai osztott
   memóriás út marad, hibaüzenettel az Events panelen, összeomlás nélkül.

**Átvétel:** teszt mindkét útra (GPU és tartalék), mérés a referenciagépen a ✅ előtt.

### Q4 – Csomagolás (1.10)

1. A D1 döntés szerint (7. fejezet): az AppImage és a Flatpak Qt-tal, vagy ezek nélkül.
2. Ha Qt-tal: a PySide6/Qt beágyazásának licencvizsgálata (`AGENTS.md`: „bundling Qt/PySide binaries
   still needs a packaging review”). Figyelem: a nyitott #757 a Qt be nem ágyazását őrző ellenőrzést
   vezetne be; a két irány ütközik, ezt a gazda és a tulajdonos dönti el (`coordination.md` §2.8).
3. Az asztali bejegyzés (`packaging/linux/io.github.ubuntuokos.cfa4.desktop`, most `Exec=cfa4`) átállítása
   a Qt GUI-ra az egui eltávolításával együtt történik (Q5), nem előtte.

**Átvétel:** a négy formátum a D1 szerint, telepítési teszttel; az 1.10 ✅.

### Q5 – Az egui eltávolítása

Csak akkor indul, ha minden sor ✅ **és** a tulajdonos jóváhagyása az ADR F8-ban rögzítve van.
Előtte fel kell számolni azt, ami ma az egui forrására támaszkodik:

1. **Közös fordítási katalógusok áthelyezése.** A Qt GUI a `crates/ui-egui/src/i18n/*.tsv` fájlokat
   olvassa (`python/cfa4_qt/i18n.py`, `SHARED_CATALOGS`), a csomagolás is onnan másolja
   (`packaging/linux/package.sh`). Ezeket (a `.attribution` fájlokkal együtt) a Qt csomagba vagy egy
   közös helyre kell átköltöztetni, a hivatkozásokkal együtt.
2. **Tesztek, amelyek az egui forrását olvassák.** `test_qt_gui.py` (például
   `crates/ui-egui/src/panels/export_mode.rs`, `crates/ui-egui/src/credits.rs`),
   `test_qt_translations.py`, `test_qt_installed_catalogs.py`. Az összevetést rögzített elvárt listára
   kell cserélni, mielőtt a forrás eltűnik.
3. **Asztali bejegyzés és csomagok:** `Exec`/`TryExec` a `cfa4-qt`-ra; az `nfpm.yaml` és a csomagszkriptek
   az egui binárisa nélkül.
4. **A munkaterület:** az `apps/cfa4` és a `crates/ui-egui` törlése (a `members` glob miatt a
   `Cargo.toml`-ban nincs külön sor); a `Cargo.lock` frissítése `cargo`-val, nem kézzel.
5. **Szabályfájlok és nyilvántartások:** `AGENTS.md` §0.7 (a pánikhorog most `crates/ui-egui/src/crash.rs`;
   helyette a `cfa4-cli serve` `catch_unwind`-ja és a `python/cfa4_qt/crash.py`), §1.6 és az
   `ATTRIBUTION.md` (az `icons.rs` ikonjai), az `AGENTS.md` „The Qt shell is a development prototype”
   mondata, a `CLAUDE.md` futtatási része (`cargo run -p cfa4`), a `docs/control-protocol.md` és a
   `docs/agents.md` egui-hivatkozásai.
6. **Dependabot:** az egui befagyasztása miatti kihagyások (`.github/dependabot.yml`, valamint a nyitott
   #690) megszüntetése.
7. **Ami nem változik:** a motor, a régi upstream projektfájlok megnyithatósága (tulajdonosi döntés,
   2026-10-24), a vezérlőcsatorna és az automatizálási azonosítók (a Qt GUI már az egui azonosítóit
   használja), az upstream-szinkron terve (az upstream egui-változásai továbbra is a Qt6-ba kerülnek;
   a forrás ezután az upstream tárolója, nem a CFA4 saját egui-ja).

**Átvétel:** a teljes CI zöld egui nélkül; a négy csomag a Qt GUI-t indítja; az ADR F8 állapota
lezárul (ACCEPTED, QT6_GUI_DONE), a `qt-parity.md` archív dokumentum lesz.

## 3. Sorrend

Q1 és Q2 előkészítése párhuzamosan indulhat (más fájlokat érintenek). Q3 független. Q4 a D1 után.
Q5 utolsó, minden ✅ és a tulajdonos jóváhagyása után.

## 4. Becslés

Becslés, nem mérés: Q1 1–2 nap, Q2 előkészítése 1 nap + a kézi tesztek a tulajdonos idejéből,
Q3 1. lépése 2–4 nap (a 2. lépés, ha kell, további 1–2 hét), Q4 a D1-től függően 0–5 nap,
Q5 2–4 nap. A szűk keresztmetszet a beolvasztás (a gazda, kötegelve, `coordination.md` §3a).

## 5. Kockázatok

- **5.5:** a GL/RHI-út PySide6 alatt Waylanden és X11-en eltérően viselkedhet; a tartalék út ezért kötelező.
- **1.10:** a beágyazott PySide6 licenc- és méretkérdés; ütközik a #757 irányával.
- **Q5:** a fordítási katalógusok és az egui-forrást olvasó tesztek kihagyása eltörné a Qt GUI-t
  vagy a CI-t; ezért ezek a Q5 első lépései.

## 6. Ami nincs ebben a tervben

Új funkciók, a Smart Cut / Smart Edit és a stáblista folytatása (ezek a tulajdonos döntése szerint
a Qt6 GUI elkészültét várják), valamint a csomagkiadás (F9) egésze.

## 7. Döntési pontok a tulajdonosnak

- **D1 – AppImage és Flatpak:** (a) PySide6/Qt beágyazva, licencvizsgálattal; (b) ez a két formátum
  Qt nélkül marad, amíg nincs megoldás, és az 1.10 a .deb + tar.gz alapján lesz ✅; (c) a két formátum
  kimarad a kiadásból. Javaslat: (b), mert a .deb és a tar.gz már az Ubuntu PySide6-ját használja,
  és nem ütközik a #757-tel.
- **D2 – 5.5 terjedelme:** az 1. lépés (textúrafeltöltés) elég-e a ✅-hoz, vagy a dmabuf is kell.
  Javaslat: az 1. lépés és a mérés után döntsön a tulajdonos.
- **D3 – 1.9 asztallistája:** melyik asztalok kötelezők a ✅-hoz. Javaslat: GNOME és KDE Plasma
  Waylanden és X11-en kötelező, a többi ajánlott.
