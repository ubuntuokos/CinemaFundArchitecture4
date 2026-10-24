# CFA4 F1-02 – Az örökölt Rust Git-függőségek saját forrásba migrálása

> **Aktuális forrás- és konfigurációs audit (2026-10-21):**
> [F1-02-SOURCE-CONFIG-GATE-2026-10-21.md](F1-02-SOURCE-CONFIG-GATE-2026-10-21.md).
> A lentebbi időrendi CI-beszámolók történeti állapotokat írnak le; az aktuális
> Linux AMD64 CI csak explicit teljes indításra fordít, és FFmpeg 9.0.2-t
> csak a release-oracle job telepít. A statikus audit PASS; a teljes CI HOLD.

Kapcsolódó ügy: https://github.com/ubuntuokos/CinemaFundArchitecture4/issues/9

Az eredeti CFA4 forrásában két `upstream library Git`
függőség volt. Az F1-02 szándékosan **két elkülönített PR-re és CI-validációra**
osztja a migrációt.

| Lépés | Crate | Korábbi revízió | Saját CFA4-forrás | Git-fetch |
| --- | --- | --- | --- | --- |
| F1-02/1 | `cfa4-docpath` | `82a24b006eefa036b43c09a67fb9a3fbd17ffe63` | `vendor/cfa4-libs/docpath` | megszüntetve |
| F1-02/2 | `cfa4-ui` | `5c874d51eeb3de70e735c8ce87a59a7dedd47675` | `vendor/cfa4-libs/ui` | megszüntetve |

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
- upstream library `crates/docpath` @ `82a24b006eef`
- upstream library `NOTICE` @ `82a24b006eef`
- upstream library `crates/ui` @ `5c874d51eeb3`
- upstream library `NOTICE` @ `5c874d51eeb3`
- upstream library `LICENSE-MIT` @ `5c874d51eeb3`
- upstream library `LICENSE-APACHE` @ `5c874d51eeb3`

A régi `cfa4-ui` NOTICE több más upstream projektből adaptált UI/ikon
elemeket is dokumentál. Ezeknek az eredet- és jogfenntartó szövegeit az
új helyi forrás is megőrzi; a két licencfájl és a NOTICE **nem**
bizonyítja, hogy az eredeti upstream védjegyek felhasználhatók.

## Külön technikai validáció

**F1-02/1 / docpath**:
- `cargo test --offline --manifest-path <ideiglenes-külön-másolat>/Cargo.toml`
- `cargo clippy --offline --manifest-path <ideiglenes-külön-másolat>/Cargo.toml --all-targets -- -D warnings`
- a munkaterület Cargo-függőséggrafjának és lockfile-jának ellenőrzése.

**F1-02/2 / UI**:
- `bash scripts/dev/test-vendored-ui.sh` (a detached scratch Cargo workspace with original upstream test-only dependencies)
- a teljes CFA4 Linux CLI-, workspace- és FFmpeg-oracle ellenőrzés.

A `cfa4-ui` saját tesztjei továbbra is használhatnak crates.io registry
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
3. Az upstream márkaelemeket 2026-10-24-én eltávolítottuk; a kiadási csomagolási
   tiltás továbbra is felülvizsgálatra szorul.
4. A két PR **piszkozat**, nem egyesíthető sikeres CI-tesztek és ellenőrzés
   nélkül.

Ez a dokumentum a forrásmigrációt rögzíti, nem tekintendő kiadási engedélynek.

### Követett Clippy kivétel (csak a cfa4-ui harmadik féltől származó crate-re)

A CFA4 Rust 1.95 toolchain `cargo clippy --workspace --all-targets --release -- -D warnings`
futtatása az upstream `cfa4-ui/src/docking/surface.rs` eredeti forrásában
`clippy::collapsible_match` figyelmeztetést talált (CI: 39489385744).
A forrás immutabilitásának megőrzésére **csak a vendorizált crate helyi
Cargo.toml fájljába** került `collapsible_match = "allow"` lint-engedmény.
Ez nem lazítja a CFA4 saját crate-jeinek szigorú Clippy kapuját, és a
SHA-jegyzék eredeti upstream forrást továbbra is változatlanul igazol.

## CI utóellenőrzés és javítás – 2026-10-20

### Ellenőrzött CI-állapot

- **PR #8**: https://github.com/ubuntuokos/CinemaFundArchitecture4/pull/8 — commit `1e7e02063be23afca0791077cf3123a8ffdaa258`; Actions: https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/39488156883. A külön `cfa4-docpath` offline ellenőrzés, Python/Qt6, architektúra/asset/format, valamint CLI smoke **sikeres**. A teljes Rust workspace és FFmpeg-oracle release teszt lekérdezéskor **folyamatban**; eredményük nem minősíthető sikeresnek.
- **PR #10, eredeti UI-revízió**: `96c113f4c2bf0feaf2e93989badde3cfc258355e`; Actions: https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/39490004898. A `cfa4-ui` elkülönített teszt, `cfa4-docpath`, Python/Qt6, formázás, architektúra és asset ellenőrzések **sikeresek**; a CLI smoke szintén **sikeres**. A Rust workspace/Clippy job **megbukott**; a release/FFmpeg job lekérdezéskor még futott.

### Clippy-fordítási hiba és célzott javítás

A `cargo xtask ci` által indított `cargo clippy --workspace --all-targets --release -- -D warnings` a `vendor/cfa4-libs/ui/tests/visual_regression.rs` tesztmodulban négy `E0432`/`E0433` hibát jelzett: a `egui_kittest`, `image` és `egui_extras` tesztfüggőségek nem részei a helyi vendor-manifestnek. Ezeket a dedikált UI-tesztfuttató szándékosan csak egy átmeneti, önálló Cargo-munkaterülethez adja hozzá, hogy a fő `Cargo.lock` ne bővüljön csak tesztcélú UI-csomagokkal.

A javítás **nem** módosítja az upstream Rust- és tesztforrásokat, és nem lazítja a CFA4 saját crate-jeinek `-D warnings` ellenőrzését:

1. `9fc2ac635675fafbf4ae28b1da5a56b92a3e2867`: a `vendor/cfa4-libs/ui/Cargo.toml` explicit `[workspace]` / `resolver = "3"` deklarációt kapott, hogy a vendor teszt-targetjei ne legyenek a CFA4 root `--workspace` vizsgálat részei. A termék `cfa4-ui` path-függősége megmaradt.
2. `ad8ea64d6758dadd14ae991eadd30918dbadf8c4`: a `scripts/dev/test-vendored-ui.sh` többé nem duplikálja a `[workspace]` táblát; az eredeti tesztfüggőségeket az izolált scratch-másolatban továbbra is betölti.

**Friss validáció:** https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/39491167243 — az új fejlesztési ágbeli commit után elindult. Amíg a Rust Clippy, release workspace és FFmpeg-oracle, valamint az új UI-teszt véglegesen nem sikeres, az F1-02 CI gate **nyitott**, a PR-ek **draft** állapotban maradnak és nem egyesíthetők.

### Második CI-felfedezés és korrigált javítás (ugyanezen nap)

A `[workspace]` deklarációval végzett **első kísérlet hibásnak bizonyult**: a
`39491364136` Actions-futtatás Rustfmt előtti Cargo-metaadat feldolgozása a
`multiple workspace roots found in the same workspace` hibával megállt.
A `9fc2ac6` és `ad8ea64` commitok ilyen konfigurációja ezért nem érvényes
megoldás; mindkét fájl tartalmát az eredeti, működő állapotra állítottuk vissza.

**Javított megoldás:** a `xtask/src/main.rs` saját `ci` parancsa és a GitHub
release-workspace workflow azonos, explicit `--workspace --exclude cfa4-ui`
Cargo-kiválasztást alkalmaz. Ez **kizárólag a külső cfa4-ui crate saját
teszt-targetjeit** veszi ki a *fő* munkaterület Clippy / release teszt futásából;
a CFA4 saját csomagjainak Clippy `-D warnings` és teszt ellenőrzése megmarad,
és az alkalmazások továbbra is függnek a helyi `cfa4-ui` könyvtártól.
Az upstream UI unit/integrációs/vizuális teszteket a
`scripts/dev/test-vendored-ui.sh` változatlanul a saját `vendored-ui`
CI-jobban futtatja. A források SHA-azonossága és a root `Cargo.lock`
megőrződik. **A teljes CI csak sikeres újrafuttatással zárható le.**

## CI-FINAL – a 120 perces release fordítás miatti leállások (2026-10-21)

A [PR #8 / run 39488156883](https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/39488156883) és a [PR #10 / run 39491958991](https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/39491958991) négy Rust workspace/release jobja végül **cancelled** lett a ~120 perces limitnél. A végleges naplók mind a négy esetben `The operation was canceled.` üzenettel végződnek. A PR #10 Clippy `--release` ellenőrzése már kb. 2 perc alatt befejeződött, majd a job a **második, teljes release-teszt fordítását** indította el; a külön FFmpeg-oracle job ezzel párhuzamosan ugyanezt fordította. A tesztekhez nem jutottak el; bizonyított új Rust/FFmpeg funkcionális teszthiba nem szerepel a naplókban.

**Kijavított CI-szerkezet:**

- A `cargo xtask ci` megtartja a format/Clippy/layers/assets kapukat, de elhagyja a másodszor, párhuzamosan indított teljes release-tesztet.
- A külön `rust-release-tests` job továbbra is lefuttatja a **teljes workspace release tesztkészletét, FFmpeg és ffprobe oracle mellett**; `cfa4-ui` upstream-only integrációs/visual tesztjeit változatlanul az elkülönített `vendored-ui` job futtatja.
- A teszt-harnesszek **CI-only** Cargo release környezeti felülírásai: `CARGO_PROFILE_RELEASE_LTO=false` és `CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16`, a memóriaterhelés korlátozására `CARGO_BUILD_JOBS=2`. Ez megőrzi a release-optimalizálási szintet és a tesztkészletet, de a teszt-harness linkelés **nem byte-azonos** a tényleges disztribúciós release profiléval. A normál `cargo build --release -p cfa4-cli` CLI-smoke megtartja a projekt eredeti teljes release-beállításait (`lto=thin`, `codegen-units=1`).
- `--timings`: a Cargo összeállításról készült mérés segíti a következő esetleges regresszió azonosítását.

**Audit gate:** a fentiek teljes, új, sikeres GitHub Actions futtatásig **nincsenek igazolva**, nincs merge vagy F1-02 lezárás. A két PR draft marad; az upstream források, licencek/NOTICE, `Cargo.lock` és a `main` változatlanok.


## Ubuntu 26.04 LTS támogatási korrekció – 2026-10-21

**Kötelező célplatform:** Ubuntu **26.04 LTS (Resolute)**. A korábbi
`.github/workflows/ci-linux.yml` fájlban szereplő `ubuntu-24.04` futtató
örökölt CI-konfiguráció volt, nem a CFA4 támogatott platformja.

Az F1-02 újrafuttatásainak **valós, végleges hibája**:

- PR #8: https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/39542032618
- PR #10: https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/39542084318
- Mindkettőben a Rust workspace/Clippy, a CLI, a Python/Qt6 és a vendor-kapuk
  sikeresek; a release workspace `cfa4-apv --test oracle` három
  összehasonlító tesztje sikertelen.
- A naplóban az ok egyértelmű: `ffmpeg version 6.1.1-3ubuntu5 cannot read apv
  (needs 8.0+), and CFA4_REQUIRE_ORACLES=1 forbids skipping oracle tests`.
  Ezt nem a kodek hibája és nem bizonyított regresszió okozza, hanem az Ubuntu
  24.04 FFmpeg 6.1.1 csomagja.

**Javítás:** az összes Linux CI-job `runs-on: ubuntu-26.04` értéket kapott,
beleértve a Python/Qt6, UI-vendor, CLI és provenance jobokat. A 26.04-es Ubuntu
hivatalos `ffmpeg` csomagja 8.0.1-es verziójú. A release oracle kapu külön
ellenőrzi a *valódi* Ubuntu 26.04 futtatókörnyezetet, az FFmpeg 8+ főverziót,
a `ffprobe` jelenlétét és az `apv` dekódert. A
`CFA4_REQUIRE_ORACLES=1` beállítás változatlan; a három tesztet nem
hagyjuk ki. A külön, eredeti release-profillal épülő CLI és a gyorsított
release-teszt-harness stratégia is változatlan.

A Python/Qt6 job explicit Python `3.13` verzióját ez a célzott OS-korrekció
nem módosítja; ez külön futtatókörnyezet-kompatibilitási döntés, nem az
Ubuntu 26.04 rendszerverziójának visszaállítása.

**Bizonyítási feltétel:** a két PR módosított commitjára indult új CI-futtatás
mindegyik kötelező jobjának sikere. Az F1-02, a PR-ek és a release gate addig
**nyitott / draft**, és a `main` ág változatlan.


## R0 FFmpeg 9.0.2 – matching the Kubuntu reference host

The supported reference machine is Kubuntu 26.04.1 with **FFmpeg 9.0.2**.
The Ubuntu 26.04 apt package supplied FFmpeg 8.0.1, so the Linux Rust CI
now builds **official FFmpeg 9.0.2** from the source archive
https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz with an independently checked
SHA256 of `8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e`.
The custom executables are isolated from system FFmpeg libraries, cached using
a commit-pinned GitHub action, and supplied to three jobs (Rust CI, CLI smoke
and strict release tests). Both `ffmpeg` and `ffprobe` must report precisely
9.0.2 and the APV decoder must be present; failures are fail-fast.

**Evidence boundary:** the FFmpeg upstream *version* is aligned, but
different compile-time codec flags and binary hashes may remain until the
local `ffmpeg -buildconf` is compared with CI. FFmpeg 9 compatibility must
also be validated against the full CLI/codec/export tests. The separate
9 GPU-effect failures on GitHub-hosted runners are not presumed fixed by
this change. Do not merge or mark these checks PASS before an actual new CI run.
