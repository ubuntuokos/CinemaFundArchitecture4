# CFA4-QT6-FINISH-001 – a Qt6 GUI befejezése és az egui eltávolítása

**Állapot:** TERVEZET 1.2 (2026-11-14; 1.1: 2026-11-10; 1.0: 2026-11-08), tulajdonosi jóváhagyásra vár.
Az 1.2 változásai: D2 és D3 eldőlt, a Q4.3 a tulajdonos 2026-11-12-i és 2026-11-14-i döntése szerint
(a csomagok semmilyen úton nem indítanak egui-t), a Q2 és a Q5.3 ehhez igazítva. Fejlesztés csak a jóváhagyott
fázisokban indul. A 7. fejezet döntési pontjait a tulajdonos hozza meg.

**Kapcsolódó dokumentumok:** döntés és fázisok: [`docs/adr/ADR-F8-QT6-RUST-ENGINE-2026-10-23.md`](../adr/ADR-F8-QT6-RUST-ENGINE-2026-10-23.md);
pontonkénti lista: [`docs/qt-parity.md`](../qt-parity.md); technikai leírás: [`docs/qt-gui.md`](../qt-gui.md);
PR-szabályok: [`docs/coordination.md`](../coordination.md).

**A szabály, amelyhez ez a terv igazodik (tulajdonosi döntés, 2026-10-24):** az egui akkor törölhető,
ha a Qt6 GUI pontról pontra mindent tud, amit az egui (a `qt-parity.md` minden sora ✅), **és** a
tulajdonos ezt kifejezetten jóváhagyja, a jóváhagyás az ADR F8-ba kerül. Ez a terv ezen nem változtat.

**Fordítások (tulajdonosi döntés, 2026-11-09, #831):** a CFA4 minden fordítása a felső szintű `i18n/`
könyvtárban van, nyelvenként egyetlen katalógusban (`i18n/<kód>.tsv`, a `.attribution` kísérőfájlokkal),
amelyet az egui és a Qt GUI is onnan olvas. Az áthelyezést és a Qt-katalógusok összevonását a #865
(I18N-HOME) végzi. E terv minden fázisában új vagy módosított fordítás csak az `i18n/` alá kerülhet,
sem a `crates/ui-egui/src/i18n/`, sem a `python/cfa4_qt/i18n/` alá.

## 1. Kiinduló állapot (`main`, 2026-11-08)

A `qt-parity.md` 63 sorából 59 ✅, 4 🟡, ❌ nincs.

| Sor | Mi hiányzik a sor szerint | Jellege |
|---|---|---|
| 1.9 Wayland/X11 minden Ubuntu 26.04 változaton | kézi ellenőrzés a valódi asztalokon (tört skálázás, beviteli módok); CI-ben X11 (Xvfb) és Wayland (weston) már fut | kézi teszt + javítások |
| 1.10 Csomagolás | a Qt-t hordozó AppImage és Flatpak ellenőrzése valódi Ubuntu 26.04 asztalon (a felülvizsgált appimagetool és a Flatpak SDK kell hozzá); az asztali bejegyzés még az egui-t indítja | kézi ellenőrzés + Q5 |
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
   **Első kör (D3): csak KDE Plasma (Kubuntu 26.04)**, Wayland-munkamenetben, és X11-ben is, ha a
   rendszer felkínálja. A többi asztal (GNOME, Xfce, LXQt, MATE, Budgie, Cinnamon; a lista a 26.04
   kiadási jegyzéke szerint ellenőrizendő) csak a KDE sikeres próbaüzeme után, külön lépésben.
2. **Pontok asztalonként:** indulás; a tálca neve és ikonja (asztali bejegyzés); 100 / 125 / 150 / 200 %
   skálázás; több monitor eltérő skálázással; panelek dokkolása, lebegtetése, fülek; natív fájlpárbeszédek
   (XDG portál); húzás a fájlkezelőből; vágólap; beviteli mód (ibus és fcitx5: magyar ékezetek, egy CJK
   bevitel a Type eszközzel és a feliratoknál); hang lejátszás (PipeWire); billentyűparancsok fókusz nélkül.
3. **Segédszkript:** `cfa4-qt --smoke --screenshot` futtatása és a képernyőképek gyűjtése asztalonként,
   hogy a kézi munka rövidebb legyen.
4. A talált hibák javítása témánként külön PR-ban.

**Átvétel:** a KDE Plasma kitöltött sora a listában, a hibák javítva vagy a tulajdonos döntésével
elfogadva, és a KDE-próbaüzem sikeresen lezárva; ezután az 1.9 Qt-oszlopa ✅. A többi asztal köre ezután
külön lépés, amelynek indításáról a tulajdonos dönt.

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

Az irány eldőlt (tulajdonosi döntés, 2026-11-09; a #757 beolvadt): a .deb és a tar.gz az Ubuntu
`python3-pyside6.*` csomagjait használja, semmit nem ágyaz be; az AppImage és a Flatpak a saját
PySide6 / Qt 6.12 példányát hordozza, kizárólag a `packaging/linux/stage-pyside6.py`-n keresztül
(a `requirements/linux-amd64.txt`-ben rögzített hash-ű kerekekből, letöltés nélkül, WebEngine nélkül,
az LGPL-szövegekkel és a forrásajánlattal). Ezt a `python/tests/test_qt_not_bundled.py` őrzi, a
licencleírás a `docs/audit/F1-01-SBOM-2026-10-25.md` 4. fejezete. A korábbi ütközés a #757-tel tehát
megszűnt, a Q4 ehhez igazodik:

1. Az AppImage és a Flatpak megépítése a felülvizsgált appimagetoollal és a Flatpak SDK-val, majd
   ellenőrzése valódi Ubuntu 26.04 asztalon (a Q2 listájával együtt is futhat).
2. A beágyazás minden változása a `stage-pyside6.py`-n, a `test_qt_not_bundled.py`-n és az SBOM 4.
   fejezetén egyszerre megy át; más út a Qt bejuttatására nincs.
3. **A csomagok semmilyen úton nem indítanak egui-t** (tulajdonosi döntés, 2026-11-12: minden csomag
   menübejegyzése a Qt6 GUI-t indítja; 2026-11-14: „már ne indítson semmi egui-t”). Az asztali
   bejegyzések a `main`-en már a `cfa4-qt`-t indítják. Ami még egui-t indít, és ami ezért változik:
   - **AppImage** (`AppRun`): megszűnik a `--egui` kapcsoló, a `CFA4_GUI=egui` és az egui-tartalék
     („python3 hiányzik, az egui indul”); python3 hiányában hibaüzenet, egui nélkül.
   - **tar.gz** (`install.sh`): megszűnik a „Qt nélküli tarball az egui-t tartja” ág; a záró üzenet a
     Qt GUI-t nevezi meg.
   - **Flatpak**: a `--command=cfa4` (egui) út megszűnik.
   - **.deb, tar.gz, AppImage, Flatpak**: a csomagok nem tartalmazzák az egui binárisát (`cfa4`);
     a `cfa4` parancs megmarad, de a Qt6 GUI-t indítja (a `cfa4-qt` indítója), hogy a terminálból
     megszokott parancs is működjön. A csomagépítés (`package.sh`) az egui-t nem fordítja.
   - Az egui forrása (`apps/cfa4`, `crates/ui-egui`) ettől még a tárolóban marad a Q5-ig; fejlesztői
     gépen `cargo run -p cfa4` továbbra is elérhető.

**Átvétel:** a négy formátum telepítési teszttel, az AppImage és a Flatpak valódi asztalon ellenőrizve;
teszt, amely csomagonként ellenőrzi, hogy nincs benne egui-bináris, és hogy a menübejegyzés, a `cfa4`
parancs és az AppImage minden indítási módja a Qt6 GUI-t indítja; az 1.10 ✅.

### Q5 – Az egui eltávolítása

Csak akkor indul, ha minden sor ✅ **és** a tulajdonos jóváhagyása az ADR F8-ban rögzítve van.
Előtte fel kell számolni azt, ami ma az egui forrására támaszkodik:

1. **Fordítási katalógusok: a felső szintű `i18n/`.** A tulajdonos döntése (2026-11-09) szerint minden
   fordítás az `i18n/<kód>.tsv` alatt van; ezt a #865 (I18N-HOME) valósítja meg: `git mv` a
   `crates/ui-egui/src/i18n/`-ből (a `.attribution` fájlokkal), a Qt-katalógusok „Qt GUI” szakaszként
   ugyanabba a fájlba, az egui a `catalog_file!` makróval, a `cfa4_qt.i18n` egyetlen `CATALOG_DIR`-ből
   olvas, a `packaging/linux/package.sh` onnan másol. A #865 a #827 (ru/uk) után olvad be, és az orosz
   és ukrán katalógust is áthozza. Ha a #865 bent van, a Q5-ben itt már nincs teendő: az egui törlése
   a katalógusokat nem érinti. Ha a Q5 indulásakor még nincs bent, előbb a #865 kerül sorra.
2. **Tesztek, amelyek az egui forrását olvassák.** `test_qt_gui.py` (például
   `crates/ui-egui/src/panels/export_mode.rs`, `crates/ui-egui/src/credits.rs`), valamint minden teszt,
   amely a #865 után is a `crates/ui-egui/` alatti fájlt nyit meg (a `test_qt_translations.py` és a
   `test_qt_installed_catalogs.py` a #865-tel már az `i18n/`-t olvassa). Az összevetést rögzített
   elvárt listára kell cserélni, mielőtt a forrás eltűnik.
3. **Csomagok:** a Q4.3 már elvégzi (asztali bejegyzés, indítási utak, egui-bináris a csomagokból);
   itt csak az ellenőrzés, hogy egyik csomagszkript sem hivatkozik a törölt forrásra.
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

Q1 és Q2 előkészítése párhuzamosan indulhat (más fájlokat érintenek). Q3 független. Q4 bármikor indulhat (a D1 eldőlt), a valódi asztalon a Q2-vel együtt is futhat.
Q5 utolsó, minden ✅ és a tulajdonos jóváhagyása után.

## 4. Becslés

Becslés, nem mérés: Q1 1–2 nap, Q2 előkészítése 1 nap + a kézi tesztek a tulajdonos idejéből,
Q3 1. lépése 2–4 nap (a 2. lépés, ha kell, további 1–2 hét), Q4 1–2 nap + a kézi ellenőrzés a tulajdonos idejéből,
Q5 2–4 nap. A szűk keresztmetszet a beolvasztás (a gazda, kötegelve, `coordination.md` §3a).

## 5. Kockázatok

- **5.5:** a GL/RHI-út PySide6 alatt Waylanden és X11-en eltérően viselkedhet; a tartalék út ezért kötelező.
- **1.10:** a beágyazott PySide6 (143 MB) mérete és frissítése: minden PySide6-emelés új kerékhash-t,
  új átmeneti állományt és SBOM-frissítést jelent; a licencet a `stage-pyside6.py` és a
  `test_qt_not_bundled.py` kezeli.
- **Q5:** az egui-forrást olvasó tesztek kihagyása eltörné a CI-t; ezért ezek a Q5 első lépései.
  A fordítási katalógusok a #865 után már nem az egui alatt vannak; ha a #865 nincs bent, az egui
  törlése a Qt GUI fordításait is elvinné.
- **Fordítások:** a #865 beolvadásáig nyitott ágak (például a stáblista-lánc, #803) még a régi
  útvonalakra írhatnak; ezeket a `main` beolvasztásakor az `i18n/` alá kell áthozni
  („mindig az újabb van érvényben”).

## 6. Ami nincs ebben a tervben

Új funkciók, a Smart Cut / Smart Edit és a stáblista folytatása (ezek a tulajdonos döntése szerint
a Qt6 GUI elkészültét várják), valamint a csomagkiadás (F9) egésze.

## 7. Döntési pontok a tulajdonosnak

- **D1 – AppImage és Flatpak: eldőlt** (tulajdonosi döntés, 2026-11-09): a két formátum a saját
  PySide6 / Qt 6.12 példányát hordozza a `stage-pyside6.py`-n keresztül, a .deb és a tar.gz az
  Ubuntu PySide6-ját használja (lásd Q4; a #757 őre ezt rögzíti).
- **D2 – 5.5 terjedelme: eldőlt** (tulajdonosi döntés, 2026-11-14): a GPU-mérés után dönt a
  tulajdonos. Előbb a mérés és az 1. lépés (textúrafeltöltés), majd az eredmény alapján dől el,
  kell-e a dmabuf is.
- **D3 – 1.9 asztallistája: eldőlt** (tulajdonosi döntés, 2026-11-14): egyelőre csak KDE Plasma;
  a többi asztal csak akkor, ha a KDE már túl van a sikeres próbaüzemen (lásd Q2).
- **Q4.3: eldőlt** (tulajdonosi döntés, 2026-11-12 és 2026-11-14): a csomagok semmilyen úton nem
  indítanak egui-t (lásd Q4.3).
