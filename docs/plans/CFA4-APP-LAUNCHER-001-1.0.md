# CFA4-APP-LAUNCHER-001 – Közös CFA4 alkalmazásindító (1.0)

**Dátum:** 2026-11-14 · **Állapot:** a tulajdonos (`ubuntuokos`) jóváhagyta 2026-11-14-én (BacklotHub #46, #55, #56). Döntés: [ADR 0014](../adr/ADR-0014-cfa4-app-launcher.md). A terv és az első fázis (AL-1) a BacklotHub 11a. lépcsőjének (DCC Integration Host) és a 11. lépcső szerkesztőbe töltésének előfeltétele, és a 9. lépcső „megnyitás alkalmazásban” funkciója is erre vár.
**Előzmény:** G1 Workfiles (BacklotHub-munkaanyag) 6. pont,
G1 DCC Integration Host (BacklotHub-munkaanyag),
G2-09 (BacklotHub-munkaanyag), G2-11 (BacklotHub-munkaanyag) ·
CFA4-BACKLOTHUB-001 11. pont

## 1. Cél és hatókör

Egy alkalmazás (a CFA4 szerkesztő, Blender, Krita, Natron, később más) elindítása **munkakörnyezettel**:
melyik produkció, melyik feladat, melyik munkafájl, hol vannak a tárolási gyökerek, és hogyan éri el az
alkalmazás a BacklotHubot. A BacklotHub terve szerint ez a CFA4 közös szolgáltatása, nem a BacklotHubé: a
szerkesztő is használhatja (pl. „megnyitás Blenderben”), BacklotHub nélkül is.

**Része:**
- alkalmazásdefiníciók és változatok (pl. Blender 4.x és 5.x egymás mellett);
- telepített alkalmazások felismerése;
- indítási környezet összeállítása (környezeti változók, argumentumok) deklaratívan;
- indítás, a folyamat figyelése (fut-e még), indítási napló;
- munkamenet-token a CFA4 szerkesztő vezérlőcsatornájához (G2-11).

**Nem része:**
- a munkafájl létrehozása és verziózása (BacklotHub 9.);
- a DCC-n belüli integráció (menü, publikálás, betöltés): BacklotHub 11a.;
- telepítés, frissítés, licenckezelés (a rendszer csomagkezelője dolga).

## 2. Mi van már meg a CFA4-ben

| Forrás | Eredmény |
|---|---|
| Alkalmazásindító terv vagy kód | **Nincs** |
| `cfa4-cli serve` (ADR 0007, 0009) | A Rust-motor közös szolgáltatásainak kiajánlása parancsként; ide illeszthető a `launcher.*` |
| Vezérlőcsatorna ([`docs/control-protocol.md`](../control-protocol.md)) | `CFA4_CONTROL_TOKEN` csak örökölt környezetből; az indító tudja biztonságosan átadni |
| XDG-integráció (F6) | A CFA4 saját `.desktop` bejegyzése; ugyanez a szabvány a többi alkalmazás felismeréséhez |

## 3. CFA3/FA3

Alkalmazásindító terv nincs (CFA3 `a021ef36`, FA3 `a9c724da`).

## 4. Külső donorok és szabványok

| Forrás | Pontos forrás | Licenc | Mit ad | Értékelés |
|---|---|---|---|---|
| **AYON applications** | `ynput/ayon-applications` @ `b9c88832` (`manager.py`, `hooks.py`, `process.py`, `defs.py`) | Apache-2.0 | Alkalmazás → változatok, végrehajtható út platformonként, környezet-összeállítás, indítás előtti/utáni hookok | **Elsődleges minta.** A hookok tetszőleges Python-kódját **nem** vesszük át: nálunk deklaratív szabályok (6. pont) |
| XDG Desktop Entry Specification | freedesktop.org | nyílt szabvány | Telepített alkalmazások, `Exec`, `TryExec`, MIME-típusok | Felismerés |
| Flatpak (`flatpak run`, `--env=`, `--filesystem`) | flatpak.org dokumentáció | LGPL (eszköz) | Sandboxolt alkalmazás indítása környezettel | Figyelni kell: a sandbox nem feltétlenül látja a gyökereket (8. pont) |
| Rez | `AcademySoftwareFoundation/rez` | Apache-2.0 | Csomagalapú környezetfeloldás | **Nem javaslom M1-re**: stúdió-szintű csomagrendszer, egygépes módban aránytalan |

## 5. Modell

- **`AppDefinition`** (gépenkénti konfiguráció, nem produkciós adat): `code` (`cfa4`, `blender`, `krita`,
  `natron`), név, változatok.
- **`AppVariant`**: `code` (pl. `blender/4.5`), végrehajtható (abszolút út, vagy `.desktop`-azonosító, vagy
  `flatpak:org.blender.Blender`), alapargumentumok, környezeti szabályok, támogatott kiterjesztések (a G2-09
  profiljaihoz).
- **`LaunchContext`** (a hívó adja, a Launcher nem értelmezi): produkció, feladat, munkafájl azonosítója és
  feloldott helyi útja, gyökerek. JSON-ként kerül a `CFA4_CONTEXT` környezeti változóba.
- **`LaunchRecord`**: indítás azonosítója, változat, kontextus, PID, indítás ideje, kilépési kód, naplófájl.
  Diagnosztikai adat (XDG state könyvtárban), nem üzleti adat; az üzleti eseményt (pl. „munkafájl
  megnyitva”) a hívó auditálja, ha kell.

## 6. Indítási környezet (deklaratív, kód nem fut)

```toml
[app.blender]
label = "Blender"

[app.blender.variant."4.5"]
exec = "/usr/bin/blender"            # vagy "desktop:blender.desktop" / "flatpak:org.blender.Blender"
args = ["{workfile.path}"]
extensions = ["blend"]

[app.blender.variant."4.5".env]
set = { CFA4_HOST_SOCKET = "{host.socket}" }
prepend = { BLENDER_USER_SCRIPTS = "{launcher.addons}/blender" }   # a 11a. adapterének helye
```

- Helyőrzők zárt listából (`{workfile.path}`, `{context.json}`, `{host.socket}`, `{launcher.addons}`,
  `{root.work}` …), a G2-09 sablonmotorjának szabályaival; nincs kifejezés, nincs shell.
- **Indítás `execve`-vel, argumentumlistával**, shell nélkül; a környezet a Launcher saját környezetéből
  indul, a megadott szabályokkal módosítva; titok (pl. a vezérlőtoken) csak annak az alkalmazásnak jut, akinek
  kell (a CFA4 szerkesztőnek).
- Külön folyamatcsoport (`CommandExt::process_group`, `unsafe` nélkül), a kimenet naplófájlba; a Launcher nem várja meg a kilépést, de figyeli.

## 7. A CFA4 szerkesztő indítása (a G2-11 igénye)

- A Launcher munkamenetenként új `CFA4_CONTROL_TOKEN`-t generál (32 bájt véletlen), a szerkesztőt
  `--control <port>` kapcsolóval és ezzel a tokennel indítja (szabad portot választ a 127.0.0.1-en), és a
  hívónak (a BacklotHubnak) **a válaszban, memóriában** adja vissza a portot és a tokent. Fájlba nem írja.
- Ha a hívó maga a szerkesztő (pl. „megnyitás Blenderben”), token nem kell.

## 8. Never crash és hibák

- Nem található végrehajtható, nem indítható, azonnal kilép, Flatpak-sandbox nem látja a gyökeret: angol
  hibakód (`LAUNCH_APP_NOT_FOUND`, `LAUNCH_EXEC_FAILED`, `LAUNCH_EXITED_EARLY`, `LAUNCH_SANDBOX_NO_ACCESS`),
  olvasható üzenet, a hívó nem omlik össze.
- Flatpaknál indítás előtt a `flatpak info --show-permissions` alapján ellenőrzés: ha a munkafájl gyökere
  nincs engedélyezve, a felület megmutatja a szükséges `flatpak override` parancsot (a Launcher nem módosít
  engedélyt magától).

## 9. Architektúra és interfész (#55, #56)

- **Döntés (#55): Rust, a `cfa4-cli serve` `launcher.*` parancsaiként**, mint a Media Services és a File
  Services adatútja. Indok: a szerkesztő (Rust) és a BacklotHub (Python) is hívja; a folyamatindítás és
  -figyelés a motor meglévő job-modelljébe illik; a BacklotHub hibája így nem érinti az indítást, és fordítva.
  Új crate: `cfa4-launcher` (L-szint a G2-n, `unsafe` nélkül; `std::process::Command`).
- **Döntés (#56): az alkalmazásdefiníciók gépenkénti TOML-fájlban** (`$XDG_CONFIG_HOME/cfa4/apps.toml`, plusz
  a CFA4 alapdefiníciói a csomagban), nem a BacklotHub adatbázisában. Indok: a végrehajtható út gépfüggő (M2-ben
  gépenként más), és a szerkesztő BacklotHub nélkül is használja. A felhasználó a felületen szerkeszti; a
  felismerés javaslatot tesz.
- **Parancsok:** `launcher.apps` (definíciók és felismert változatok), `launcher.detect`, `launcher.launch`
  (változat + kontextus + fájl), `launcher.status`, `launcher.log`.

## 10. Fázisok

| Fázis | Tartalom | Kell ehhez |
|---|---|---|
| AL-0 | Ez a terv jóváhagyva (#55, #56); ADR 0014 a `main`-be | – |
| AL-1 | Definíciók, felismerés (XDG, Flatpak), indítás kontextussal, CFA4 szerkesztő tokennel; tesztek: hiányzó alkalmazás, azonnali kilépés, Flatpak-engedély hiánya, kontextus-helyőrzők fuzz | BacklotHub 9. „megnyitás alkalmazásban”, 11. betöltés a szerkesztőbe |
| AL-2 | Adapterek helye (`{launcher.addons}`) és telepítésük ellenőrzése | BacklotHub 11a. |
| AL-3 | M2: gépenkénti definíció-szinkron, távoli gyökerek | M2 |

## 11. Döntések

| # | Kérdés | Döntés (tulajdonos, 2026-11-14) |
|---|---|---|
| 55 | Hol és milyen nyelven készüljön az alkalmazásindító? | Rust, `cfa4-cli serve` `launcher.*` parancsai, új `cfa4-launcher` crate |
| 56 | Hol legyenek az alkalmazásdefiníciók? | Gépenkénti TOML (`$XDG_CONFIG_HOME/cfa4/apps.toml`) + a csomag alapdefiníciói; nem a BacklotHub adatbázisában |
