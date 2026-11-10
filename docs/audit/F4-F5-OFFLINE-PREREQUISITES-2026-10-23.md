# CFA4 – F4/F5 offline csomagolási előfeltétel (2026-10-23)

**Kiadás: HOLD.** Az upstream szülőprojektet és a CFA4 `main` ágát nem módosítja.

A korábbi `flatpak-bundle.sh` automatikusan Flathub-tárolót vett fel, SDK/runtime-függőséget telepíthetett, és alapértelmezésben a felhasználó Flatpak-alkalmazásai közé telepítette a tesztcsomagot. Ez nem felelt meg az előre ellenőrzött, helyi eszközökkel történő csomagépítés követelményének.

Az új `scripts/release/check-cfa4-package-prereqs.sh` letöltés és telepítés nélkül ellenőrzi az Ubuntu 26.04 AMD64 platformot, a csomagolási eszközöket, a helyileg telepített Freedesktop Platform és SDK 25.08 verziót, valamint az ellenőrzőösszeggel jóváhagyott AppImage-készítő programot. Hiányzó előfeltételnél az építés előtt BLOCKED eredménnyel áll le.

A Flatpak-csomagolás `--disable-download` módban fut, nem vesz fel Flathub remote-ot, és nem telepít alapértelmezésben alkalmazást. Külön `--test` és `CFA4_ALLOW_FLATPAK_INSTALL_TEST=1` szükséges a helyi alkalmazástelepítéssel járó próbához; ez is `--no-pull --no-related` módban történik.

Az egyfájlos Flatpak-csomag **nem tartalmazza** a Freedesktop futtatókörnyezetet. Az offline végfelhasználói telepítéshez a célgépen a megfelelő runtime előzetes, külön ellenőrzött rendelkezésre állása kell. Az alkalmazás AI-jának nincs felhőfüggősége.

## Lokális ellenőrzés és építés

```bash
cd ~/GIT/CinemaFundArchitecture4
git fetch origin
git switch fix/f4-offline-four-package-preflight

APPIMAGETOOL=/abszolut/ut/ellenorzott/appimagetool \
CFA4_APPIMAGETOOL_SHA256=JOHAHAGYOTT_64_HEX_SHA256 \
CFA4_PYSIDE6_WHEELS=/abszolut/ut/pyside6-wheelhouse \
bash scripts/release/check-cfa4-package-prereqs.sh

CFA4_ALLOW_LOCAL_PACKAGING=1 \
APPIMAGETOOL=/abszolut/ut/ellenorzott/appimagetool \
CFA4_APPIMAGETOOL_SHA256=JOHAHAGYOTT_64_HEX_SHA256 \
CFA4_PYSIDE6_WHEELS=/abszolut/ut/pyside6-wheelhouse \
bash scripts/release/build-cfa4-four-formats.sh
```

A `CFA4_PYSIDE6_WHEELS` egy helyi könyvtár a `requirements/linux-amd64.txt`-ben rögzített PySide6 6.12 wheelekkel (`pyside6_essentials`, `pyside6_addons`, `shiboken6`): az AppImage és a Flatpak ebből viszi a Qt GUI PySide6 / Qt 6 futtatókörnyezetét (`packaging/linux/stage-pyside6.py`, qt-parity 1.10); a csomagolás nem tölt le semmit, és más SHA-256-ú wheelt elutasít. A kiadásjelölt workflow ugyanezt a `CFA4_PYSIDE6_WHEELS` repository-változóból veszi.

Az első parancs **csak** a helyi előfeltételeket ellenőrzi. A négy csomag előállítása, telepítése, a SHA-256 bizonyítékok és a már nyitott jogi/SBOM/XDG/runtime kapuk továbbra is csak tényleges eredményekkel zárhatók le. Nem történt csomagközzététel.
