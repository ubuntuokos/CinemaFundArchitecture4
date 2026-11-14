# ADR 0014 – CFA4 Application Launcher: közös alkalmazásindító (CFA4-APP-LAUNCHER-001)

**Dátum:** 2026-11-14 · **Állapot:** elfogadva (a tulajdonos, `ubuntuokos`, 2026-11-14; BacklotHub #46, #55, #56) ·
**Terv:** [`docs/plans/CFA4-APP-LAUNCHER-001-1.0.md`](../plans/CFA4-APP-LAUNCHER-001-1.0.md) · **Érintett rész:**
`apps/cfa4-cli` (`serve`), új crate (`cfa4-launcher`) az AL-1 fázisban; ez az ADR kódot nem változtat

## Háttér

A BacklotHub (CFA4-BACKLOTHUB-001) terve szerint az alkalmazások indítása a CFA4 Application Launcher
feladata (11. pont). Ilyen szolgáltatásnak nem volt terve. A szabály szerint nem létező közös szolgáltatásra
modul nem épülhet, ezért a BacklotHub munkafájl-kezelése (9. lépcső) indítás nélkül készül, és a
DCC-integráció (11a. lépcső) a Launcher után jöhet (#46).

A BacklotHub Loader (11. lépcső) G2-je egy további igényt mutatott: a szerkesztő vezérlőcsatornája
`CFA4_CONTROL_TOKEN`-t kér, amelyet csak örökölt környezetből szabad átadni
([`docs/control-protocol.md`](../control-protocol.md)). Egy külön indított szerkesztőhöz a BacklotHub nem fér
hozzá; az indítónak kell a tokent generálnia és átadnia.

## Döntés

1. **Egy közös CFA4 alkalmazásindító készül:** alkalmazásdefiníciók és változatok, a telepített alkalmazások
   felismerése (XDG `.desktop`, Flatpak), indítás munkakörnyezettel (produkció, feladat, munkafájl, gyökerek),
   a folyamat figyelése, indítási napló. A szerkesztő BacklotHub nélkül is használja.
2. **Nem része:** a munkafájl létrehozása és verziózása (BacklotHub), a DCC-n belüli integráció (BacklotHub
   11a.), telepítés és licenckezelés.
3. **Rustban, a `cfa4-cli serve` `launcher.*` parancsaiként** (#55), új `cfa4-launcher` crate-ben, `unsafe`
   nélkül, mint a Media Services (ADR 0007) és a File Services adatútja (ADR 0009). Így a CLI, az MCP, a
   szerkesztő és a BacklotHub ugyanazt hívja.
4. **Az alkalmazásdefiníciók gépenkénti TOML-fájlban** vannak (`$XDG_CONFIG_HOME/cfa4/apps.toml`), a csomag
   alapdefinícióival (#56), nem a BacklotHub adatbázisában, mert gépfüggők.
5. **Deklaratív indítási környezet:** zárt helyőrzőlista, kód nem fut (az AYON applications Apache-2.0
   mintáját követjük, a Python-hookjait nem). Indítás argumentumlistával, shell nélkül.
6. **Szerkesztő indítása tokennel:** a Launcher munkamenetenként új `CFA4_CONTROL_TOKEN`-t generál, a
   szerkesztő környezetébe adja, és a hívónak csak a válaszban adja vissza; fájlba nem írja.
7. **Never crash** (`AGENTS.md` §0): hiányzó vagy nem induló alkalmazás, korai kilépés, Flatpak-sandbox
   engedélyhiánya kezelt hibát ad angol hibakóddal (`LAUNCH_APP_NOT_FOUND`, `LAUNCH_EXEC_FAILED`,
   `LAUNCH_EXITED_EARLY`, `LAUNCH_SANDBOX_NO_ACCESS`); engedélyt a Launcher magától nem módosít.

## Fázisok

| Fázis | Tartalom | Kell, mielőtt … |
|---|---|---|
| AL-0 | Ez az ADR és a terv a `main`-ben | – |
| AL-1 | Definíciók, felismerés, indítás kontextussal, szerkesztő tokennel; tesztek: hiányzó alkalmazás, korai kilépés, Flatpak-engedély, helyőrző-fuzz | BacklotHub 9. „megnyitás alkalmazásban”, 11. betöltés a szerkesztőbe |
| AL-2 | Az adapterek helye (`{launcher.addons}`) és telepítésük ellenőrzése | BacklotHub 11a. |
| AL-3 | M2: gépenkénti definíció-szinkron, távoli gyökerek | M2 |

## Következmények

- A `cfa4-cli serve` szerződése a `launcher.*` parancsokkal bővül; a változásra az ADR 0007 10. pontjának
  szerződéstesztjei vonatkoznak.
- Új Rust-függőség nem szükséges (a TOML-olvasáshoz a munkaterület meglévő függőségét használjuk; a G3 kapun
  ellenőrizve).
- A BacklotHub „megnyitás alkalmazásban” parancsa és a szerkesztőbe töltés az AL-1 után kapcsol be.

## Kapcsolódó dokumentumok

- [`docs/plans/CFA4-APP-LAUNCHER-001-1.0.md`](../plans/CFA4-APP-LAUNCHER-001-1.0.md)
- [ADR 0007](ADR-0007-media-services.md), [ADR 0009](ADR-0009-cfa4-file-services.md)
- [`docs/control-protocol.md`](../control-protocol.md)
