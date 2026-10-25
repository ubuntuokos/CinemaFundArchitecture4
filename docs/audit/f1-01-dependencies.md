# CFA4 – F1-01 külső függőségek és eredet auditja

> **Frissítés 2026-10-25:** a mai állapotot és a teljes licencleltárt a
> [F1-01-SBOM-2026-10-25.md](F1-01-SBOM-2026-10-25.md) írja le. Az alábbi szöveg a
> `273b164` állapotot rögzíti; a Git-függőségek és az AppImage-letöltés kockázata azóta megszűnt.

Vizsgált alap: `ubuntuokos/CinemaFundArchitecture4`, `273b164df4ec0a835450c7a830e4bb6c6118584c`.
Kiadási minősítés: **nem kiadásra kész**; a vizsgálat nem teljes OSS-licenctanúsítás.
Nyomon követés: [CFA4-FIX-003 / #6](https://github.com/ubuntuokos/CinemaFundArchitecture4/issues/6),
F1 szülőfeladat [#4](https://github.com/ubuntuokos/CinemaFundArchitecture4/issues/4).
A változások külön ágon készülnek, a PR #5 futó Rust CI-jának megszakítása nélkül.

## 1. Azonosított Cargo-források

A `Cargo.lock` alapján **708 feloldott csomag**: **54 helyi workspace-csomag**,
**652 crates.io registry-forrású csomag**, **2 rögzített Git-forrású csomag**.
Ezek csomagszámok, nem a megvizsgált licencek száma. A registry-csomagok
licencei, build-szkriptjei és tranzitív felhasználási feltételei **nincsenek
egyenként ellenőrizve**.

| Név | Forrás | SHA-1 Git-commit | Közvetlen rendeltetés |
|---|---|---|---|
| `cfa4-ui` | upstream library repository | `5c874d51eeb3de70e735c8ce87a59a7dedd47675` | egui UI-komponensek és ikoninterakciók |
| `cfa4-docpath` | upstream library repository | `82a24b006eefa036b43c09a67fb9a3fbd17ffe63` | dokumentumokban tárolt helyi útvonalak felismerése |

Mindkét commit és az adott revízióban szereplő crate-manifest elérhető.
A két upstream commit `Cargo.toml` állományának workspace-licence
`MIT OR Apache-2.0`; a két revízióban ellenőrzött
`LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE` megvan.
A licencek és a NOTICE szerzői/eredetmegjelölési kötelezettségei
**nem** szűnnek meg azzal, hogy a forrást saját GitHub-tárolóba másoljuk.
A `cfa4-ui` régebbi `NOTICE` több upstream testvérprojektből adaptált megoldásokat
is megnevez; ezeket a későbbi vendor-másolat származási jegyzékében
meg kell őrizni.

Források (változatlan, pontos URL-ek):
- upstream library `crates/ui` @ `5c874d51eeb3`
- upstream library `crates/docpath` @ `82a24b006eef`
- upstream library `NOTICE` @ `5c874d51eeb3`
- upstream library `NOTICE` @ `82a24b006eef`

**Kockázat:** a `rev` egy változtathatatlan Git-commitra mutat, ezért nem
mozgó ág vagy címke; azonban a CI és az új munkaállomások továbbra is
az upstream Git-szerver elérhetőségétől és a commit hosszú távú
megtartásától függnek. A `Cargo.lock` nem helyettesíti a saját
forrásarchívumot és a belső, ellenőrzött checksummal tárolt snapshotot.

## 2. Python/Qt és külső csomagolóeszközök

- `pyproject.toml`: `PySide6>=6.10,<7` (Qt6), `pytest>=8,<10`,
  `ruff>=0.9,<1`, `setuptools>=75` és `wheel`: tartományok,
  **nem** hash-pinnelt Python lockfile. A PySide6 alá transzitív
  Qt-/Shiboken-kerekek kerülhetnek. Dinamikus linkelési, LGPLv3/GPL
  modulonkénti terjesztési vizsgálat és reprodukálható lock/constraints
  szükséges; jelen audit nem ad kereskedelmi vagy terjesztési jóváhagyást.
- `packaging/linux/package.sh`: hiányzó `appimagetool` esetén
  `https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$ARCH.AppImage`
  URL-ről tölt le futtatható fájlt **SHA-256 ellenőrzés nélkül**.
  Ez **nyitott ellátásilánc-kockázat**. Az automatikus letöltést
  ellenőrzött, verzióhoz és hashhez kötött eszközre vagy előtelepített,
  explicit megadott helyi binárisra kell cserélni az F1-02 során.
- A fejlesztői környezet apt-/pip-/Rust-registry elérései nem teszik
  a teljes buildet hálózatfüggetlenné. A `cargo build --locked`
  a verziófeloldást rögzíti, **nem** garantálja az offline működést.
- Az upstream márkalicenc szerint az upstream védjegyei nem kerülhetnek
  módosított CFA4-terjesztésbe (2026-10-24 óta eltávolítva); az attribúciók megtartandók, az eredeti
  márkajelzések nem használhatók a független csomagban.

## 3. Auditkapu – átmeneti biztonsági ellenőrzés

A `python -m cfa4_devtools.dependency_audit` parancs hálózati kérés
nélkül, Python 3.12+ környezetben ellenőrzi:
1. A két megengedett Git-forrás **pontos repository URL- és 40 karakteres SHA-egyezését**
   a `Cargo.toml` `workspace.dependencies` mezőiben.
2. A `Cargo.lock` két egyező, teljes `git+...` forrásrekordját.
3. A további Rust workspace-Cargo-manifestekben megjelenő
   közvetlen `git` forrásokra vonatkozó tiltást.
4. Új vagy eltérő Git-forrás esetén hibaüzenettel nem nulla kilépést.

Ez **átmeneti drift-ellenőrzés**, nem a Git-függőségek önállósításának
helyettesítője. Amikor F1-02-ben a függőségeket saját forrásra vagy
path-függőségre migráljuk, **egyazon PR-ben** kell frissíteni a kontrollt,
a `Cargo.lock` fájlt és a NOTICE dokumentációt.

## 4. Javasolt F1-02 migrációs sorrend

1. **`cfa4-docpath` önállósítása elsőként:** a rögzített revízió
   megfelelő crate-forrása, upstream szerzői jog és licencek pontos
   átvétele; fájlonként ellenőrzött és SHA-256 listával dokumentált
   `third_party` snapshot. A workspace `path` függőségének
   rögzítése és `cargo test --locked` után upstream elérés
   nélküli, tiszta rebuild bizonyítása.
2. **`cfa4-ui` önállósítása külön commitban:** ugyanígy, de az eredeti
   `NOTICE` forráseredet-, ikon-, UI-minta- és védjegyhatár
   felülvizsgálatával; Qt6 migráció idején a ki nem használt elemek
   későbbi eltávolítása.
3. **Reprodukálhatóság:** Python constraints/hash-szintű feloldás,
   `cargo vendor` registry-forrásokra és offline CI-próba
   csak akkor, ha a licenc- és disztribúciós követelmények teljesülnek.
4. **Kiadási kontroll:** az AppImage-eszköz automatikus, folyamatos
   letöltésének kiváltása verzióhoz kötött SHA-256-tal; Python/Qt,
   Rust és médiaeszközök SBOM-ja, attribúciós és F0 regressziós tesztek.

## 5. Ellenőrzési hatókör és hiányzó bizonyíték

| Vizsgálat | Állapot |
|---|---|
| Közvetlen Cargo Git-source revíziók és `Cargo.lock` | Ellenőrizve |
| A két `cfa4-libs` revízió gyökér licence, NOTICE és crate manifest | Ellenőrizve |
| Python/Rust Git drift-kapu | Bevezetve az auditágon; CI-eredmény külön rögzítendő |
| 652 registry-függőség minden licence | Nincs teljesen ellenőrizve |
| PySide6 és összes tranzitív Qt licenc | Nincs teljesen ellenőrizve |
| Offline, eredeti upstream nélküli build | Nem történt meg |
| Linux kiadás, appcsomagok és márkajogok | **Nem engedélyezett, F1 nyitott** |

F1-01 kimenete ezért *audit és következetesség-ellenőrzés*, nem
a teljes projektfüggetlenítés lezárása.
