# CFA4 F1-02 – Az örökölt Rust Git-függőségek saját forrásba migrálása

Kapcsolódó ügy: https://github.com/ubuntuokos/CinemaFundArchitecture4/issues/9

Az eredeti CFA4 forrásában két `git+https://github.com/storytold/craft-libs`
függőség volt. Az F1-02 szándékosan **két elkülönített PR-re és CI-validációra**
osztja a migrációt.

| Lépés | Crate | Korábbi revízió | Saját CFA4-forrás | Git-fetch |
| --- | --- | --- | --- | --- |
| F1-02/1 | `craft-docpath` | `82a24b006eefa036b43c09a67fb9a3fbd17ffe63` | `vendor/craft-libs/docpath` | megszüntetve |
| F1-02/2 | `craft-ui` | `5c874d51eeb3de70e735c8ce87a59a7dedd47675` | `vendor/craft-libs/ui` | megszüntetve |

## Források és származási bizonyíték

Mindkét vendorizált crate saját `SOURCE_MANIFEST.json` jegyzéke tartalmazza:
- a teljes upstream Git URL-t és a pontos commitot;
- az eredeti upstream crate `Cargo.toml` Git-blob SHA-1 azonosítóját;
- a változatlanul átvett Rust-, README-, teszt-, licenc- és NOTICE-fájlok
  **Git-blob SHA-1 azonosítóját**.

Az eredeti `Cargo.toml` fájlok workspace-függő deklarációit szándékosan
helyi, explicit verziójú manifestekre alakítottuk. A `src` és `tests`
mappák forrásai, a README, valamint a licencek/NOTICE **byte-azonosak**
a rögzített upstream revíziók tartalmával. A `python -m
cfa4_devtools.dependency_audit` offline ellenőrzőprogram újraszámolja a
Git-blob SHA-1 azonosítókat, és a root `Cargo.toml`/`Cargo.lock`
helyi path-függőségeinek egyezését is vizsgálja.

Pontos upstream hivatkozások:
- https://github.com/storytold/craft-libs/tree/82a24b006eefa036b43c09a67fb9a3fbd17ffe63/crates/docpath
- https://github.com/storytold/craft-libs/blob/82a24b006eefa036b43c09a67fb9a3fbd17ffe63/NOTICE
- https://github.com/storytold/craft-libs/tree/5c874d51eeb3de70e735c8ce87a59a7dedd47675/crates/ui
- https://github.com/storytold/craft-libs/blob/5c874d51eeb3de70e735c8ce87a59a7dedd47675/NOTICE
- https://github.com/storytold/craft-libs/blob/5c874d51eeb3de70e735c8ce87a59a7dedd47675/LICENSE-MIT
- https://github.com/storytold/craft-libs/blob/5c874d51eeb3de70e735c8ce87a59a7dedd47675/LICENSE-APACHE

A régi `craft-ui` NOTICE több más ArtCraft projektből adaptált UI/ikon
elemeket is dokumentál. Ezeknek az eredet- és jogfenntartó szövegeit az
új helyi forrás is megőrzi; a két licencfájl és a NOTICE **nem**
bizonyítja, hogy az eredeti ArtCraft védjegyek felhasználhatók.

## Külön technikai validáció

**F1-02/1 / docpath**:
- `cargo test --offline --manifest-path <ideiglenes-külön-másolat>/Cargo.toml`
- `cargo clippy --offline --manifest-path <ideiglenes-külön-másolat>/Cargo.toml --all-targets -- -D warnings`
- a munkaterület Cargo-függőséggrafjának és lockfile-jának ellenőrzése.

**F1-02/2 / UI**:
- `bash scripts/dev/test-vendored-ui.sh` (a detached scratch Cargo workspace with original upstream test-only dependencies)
- a teljes CFA4 Linux CLI-, workspace- és FFmpeg-oracle ellenőrzés.

A `craft-ui` saját tesztjei továbbra is használhatnak crates.io registry
függőségeket. Ez **nem** tekinthető a teljes build offline bizonyításának.

A harmadik féltől változatlanul átvett Rust forrásokat szándékosan nem
írja át a projekt Rustfmt-ja. A `cargo xtask fmt-check` a CFA4 saját
Rust-csomagjait ellenőrzi, a vendor-kódot pedig a Git-blob eredetellenőrzés,
az önálló Cargo build/teszt és a Clippy vizsgálja.

## Ellenőrzendő kockázatok – nem lezárt F1

1. A crates.io registry (a kezdeti auditban 652 csomag) továbbra is hálózatot
   igényelhet; a két Git-függőség kiváltása **nem** egyenlő teljes offline builddel.
2. A licencek változatlanul érvényesek, és az összes transitív crate,
   PySide6/Qt, FFmpeg, AppImage eszköz külön SBOM- és terjesztési auditot igényel.
3. A runtime FilmCraft/ArtCraft márkaelemek és a kiadási csomagolási tiltás
   továbbra is felülvizsgálatra szorulnak.
4. A két PR **piszkozat**, nem egyesíthető sikeres CI-tesztek és ellenőrzés
   nélkül.

Ez a dokumentum a forrásmigrációt rögzíti, nem tekintendő kiadási engedélynek.

### Követett Clippy kivétel (csak a craft-ui harmadik féltől származó crate-re)

A CFA4 Rust 1.95 toolchain `cargo clippy --workspace --all-targets --release -- -D warnings`
futtatása az upstream `craft-ui/src/docking/surface.rs` eredeti forrásában
`clippy::collapsible_match` figyelmeztetést talált (CI: 39489385744).
A forrás immutabilitásának megőrzésére **csak a vendorizált crate helyi
Cargo.toml fájljába** került `collapsible_match = "allow"` lint-engedmény.
Ez nem lazítja a CFA4 saját crate-jeinek szigorú Clippy kapuját, és a
SHA-jegyzék eredeti upstream forrást továbbra is változatlanul igazol.
