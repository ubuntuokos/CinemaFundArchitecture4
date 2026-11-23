# Qt6 GUI – kézi asztali ellenőrzőlista (KDE Plasma)

**Terv:** [CFA4-QT6-FINISH-001](../plans/CFA4-QT6-FINISH-001.md) Q2 · **qt-parity sor:** 1.9 ·
**Első kör (D3, tulajdonosi döntés 2026-11-14): csak KDE Plasma (Kubuntu 26.04)**, Wayland- és
X11-munkamenetben is, mert az X11-kompatibilitás egyelőre marad. A többi asztal (GNOME, Xfce, LXQt,
MATE, Budgie, Cinnamon; a lista a 26.04 kiadási jegyzéke szerint ellenőrizendő) csak a KDE sikeres
próbaüzeme után, külön lépésben indul, a tulajdonos döntése szerint.

Valódi asztal kell hozzá, ezért a tulajdonos gépén (gépein) fut
([referenciagép](reference-machine.md): KDE Plasma 6.6, Wayland). Az X11-munkamenethez a bejelentkezéskor
a „Plasma (X11)” munkamenetet kell választani, ha a gép kínálja.

## Előkészítés

1. A GUI indítása a checkoutból: [`docs/qt-gui.md`](../qt-gui.md) (`.venv/bin/cfa4-qt`), vagy telepített csomagból.
2. Az asztali bejegyzés egyszeri telepítése, hogy a tálca a „CFA4” nevet és ikont mutassa:
   `.venv/bin/cfa4-qt --install-desktop-entry`.
3. **Segédszkript:** `scripts/dev/qt-desktop-smoke.sh [mappa]` lefuttatja a `cfa4-qt --smoke --screenshot`
   parancsot a jelenlegi munkamenetben, és az asztal–munkamenet nevű almappába menti a képernyőképet, a
   kimenetet és a munkamenet adatait (`session.txt`: Wayland/X11, Qt-környezeti változók, bemeneti módok).
   Minden skálázási fokozat és munkamenet után futtasd újra, és a képeket csatold az eredményhez.
4. Az eredményt az alábbi táblázat sorai mellé kell írni: ✅ (rendben), ❌ (hiba, issue vagy PR száma),
   n/a (az asztal nem kínálja). A talált hibákat témánként külön PR javítja (Q2.4).

## Ellenőrzőlista

Egy táblázat munkamenetenként. Másold le a táblázatot a rögzítéskor; az eredmény egy PR-ban megy a
`docs/development/qt-desktop-results/` alá (dátummal), nem ebbe a fájlba.

| # | Pont | Mit nézz | Wayland | X11 |
|---|---|---|---|---|
| 1 | Indulás | a GUI elindul, a motor csatlakozik, a `--smoke` kilépési kódja 0, a képernyőkép nem üres | | |
| 2 | Tálca neve és ikonja | a tálcán és az alt+tab listában „CFA4” és a logó látszik, nem „python3” | | |
| 3 | Skálázás 100 % | a szöveg és az idővonal éles, nincs levágás | | |
| 4 | Skálázás 125 % | tört skálán nincs elmosódás, a panelek és a monitorkép illeszkedik | | |
| 5 | Skálázás 150 % | mint fent | | |
| 6 | Skálázás 200 % | mint fent | | |
| 7 | Több monitor eltérő skálával | az ablak áthúzása a két monitor között: nincs villogás, a méret és a rajz helyes | | |
| 8 | Panelek | dokkolás, lebegtetés, fülek, az elrendezés mentése és visszaállítása | | |
| 9 | Natív fájlpárbeszédek | megnyitás és mentés az XDG-portálon át (a KDE saját párbeszédje jön) | | |
| 10 | Húzás a fájlkezelőből | a Dolphinból a médiaböngészőre és az idővonalra húzott fájl behívódik | | |
| 11 | Vágólap | másolás és beillesztés az idővonalon belül és más programmal (szöveg) | | |
| 12 | Beviteli mód – ibus | magyar ékezetek (á é í ó ö ő ú ü ű) a Type eszközben és a feliratoknál; egy CJK bevitel | | |
| 13 | Beviteli mód – fcitx5 | mint fent | | |
| 14 | Hanglejátszás | PipeWire-on át szól, a hangerő és a némítás működik, a szerkesztés közbeni szünet nem akaszt | | |
| 15 | Billentyűparancsok | az eszközparancsok működnek; Waylanden a fókuszt nem kapó ablakra is érvényes viselkedés (qt-parity 1.9) | | |

## Átvétel (terv Q2)

A KDE Plasma kitöltött sorai (Wayland és X11) a listában, a hibák javítva vagy a tulajdonos döntésével
elfogadva, és a KDE-próbaüzem sikeresen lezárva. Ezután az 1.9 sor Qt-oszlopa ✅; a többi asztal köréről a
tulajdonos dönt.
