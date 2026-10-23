# CFA4 – ellenőrzött állapot (2026-10-23, `cfa4-detach-branding` ág)

Ezt a fájlt add be minden új MI-beszélgetés elején, és kérd meg, hogy ehhez képest dolgozzon.
Ami itt áll, azt lefuttatva ellenőriztük; ami nincs ellenőrizve, azt külön jelöltük.

## Mi ez a projekt
- Rust nyelvű videóvágó, a FilmCraft 0.10.0 (`cdf43fd`, ArtCraft csapat) módosított változata.
  Az eredet és a változtatások leírása a `README.md` "Origin" fejezetében van. Licenc: MIT OR Apache-2.0.
- A `main` ága a 2026-10-23-i "forrás leválasztással" (source detachment) áll. Ez az ág erre épül.

## Mi változott ezen az ágon
1. **Munkaterület-javítás (`Cargo.toml`).** A `vendor/craft-libs/*` kizárása nem működött (a `exclude` nem
   kezeli a `*`-ot), ezért a vendorolt `craft-ui` és `craft-docpath` munkaterület-tagként viselkedett, és a
   `cargo test --workspace` / `cargo clippy --workspace --all-targets` elbukott a tesztjeiken. Most név szerint
   ki vannak zárva (tagok: 56 → 54). A `Cargo.lock` nem változott.
2. **ArtCraft-jelzések eltávolítva** (a márkalicenc ezt módosított változatnál megköveteli): `docs/brand/`,
   a logót kirajzoló `brand.rs` és hívásai (Kezdőlap, Névjegy), `ATTRIBUTION.md` és `AGENTS.md` vonatkozó részei.
3. **Közösségi linkek eltávolítva** (Discord, weboldal, getartcraft): Súgó menü, fejléc gomb, Névjegy,
   Kezdőlap, macOS natív menü. Az appon belüli linkek a `ubuntuokos/CinemaFundArchitecture4` repóra mutatnak.
4. **Metaadatok:** csomagolási URL-ek, `nfpm.yaml` (vendor: CFA4), Linux karbantartó (`CFA4 maintainers
   <ubuntuokos@users.noreply.github.com>`), AppImage frissítési forrás, `docs/releasing.md`.
5. **Fordítások:** 11 katalógusból az eltávolított szövegek törölve; a "CFA4 on GitHub" és két új About-szöveg
   (9 nyelven) hozzáadva.
6. **`README.md`** újraírva (a FilmCraft reklámszövege, letöltési és Discord részek nélkül). Egy mondat
   (a CFA4 célja) üresen maradt, azt neked kell megírnod. A régi README a git előzményekben megvan.

## Ellenőrzés eredménye (Linux, rustc 1.97.0)
| Parancs | Eredmény |
|---|---|
| `PYTHONPATH=python python3 -m cfa4_devtools.dependency_audit` + `unittest` (5 teszt) | OK |
| `cargo check --workspace --all-targets --locked` (az asztali appot is beleértve*) | OK, 0 figyelmeztetés |
| `cargo test -p filmcraft-ui-egui --lib` | 196 OK |
| `cargo test -p filmcraft-ui-egui --test links_ui --test chrome_ui --test menus_ui --test settings_ui --test shared_docking_ui --test project_panel_ui --test export_ui --test frame_export_ui` | mind OK |
| `cargo xtask assets` / `cargo xtask layers` | OK (61 eszköz) / OK |
| `cargo fmt --check` | OK |
| `cargo clippy -p filmcraft-ui-egui -p filmcraft --all-targets --locked -- -D warnings` | OK |

\* Az ellenőrző környezetben nincs ALSA fejlesztői csomag, ezért az app típusellenőrzéséhez egy csak
ellenőrzésre való üres `alsa.pc` leírót használtunk. Az app tényleges linkelése és futtatása nem történt meg.

## NEM ellenőrzött
- A teljes `cargo test --workspace` (csak a fent felsorolt tesztek futottak) és a `cargo xtask ci`, `cargo xtask wasm`.
- A GitHub Actions tényleges futása (`nix.yml`, `cfa4-source-detachment.yml`).
- Az app indítása és kézi kipróbálása; a macOS/Windows/Linux csomagoló scriptek futtatása.

## Szándékosan NEM változott (következő lépések)
- **A termék neve a felületen még "FilmCraft"** (About fejléc, menük). A "FilmCraft" szó 765 helyen szerepel,
  de nagyrészt belső azonosító (projektfájl-metaadatok, beállítások, AAF/OMF, billentyűparancs-készletek):
  ezeket nem szabad átnevezni. Csak a felhasználónak látható szövegeket szabad, a fordítási katalógusokkal együtt.
- **App-azonosító `ai.storyteller.filmcraft`** (csomagolás, ikon- és fájlnevek): a storyteller.ai domain nem a tiéd.
  Javaslat: `io.github.ubuntuokos.cfa4`. Sok fájlnév és az `ATTRIBUTION.md` sorai érintettek.
- `AGENTS.md`, `CLAUDE.md`, `docs/contributing.md` stb. még a `storytold/craftrules` szabványokra és a
  `storytold/craft-fonts` opcionális betűtípus-tárra hivatkoznak (a betűtípus nem build-függőség: csak helyi
  mappából olvas, hálózat nélkül). `contributors/contributors.json` repo-mezője is még `storytold/filmcraft`.
- Maga a vendorolt kód (`vendor/craft-libs/`) változatlan, bájtra az eredeti, ahogy a manifest rögzíti.

## Szabályok, hogy ne csússzon el a fejlesztés
- Egy beszélgetés = egy kis, konkrét feladat. Utána commit.
- Az MI csak azt módosíthatja, amit a feladat megnevez.
- Állítást csak akkor fogadj el az állapotról, ha ebből a fájlból vagy egy lefuttatott parancs kimenetéből következik.
- Működő állapotot címkézz: `git tag jo-YYYY-MM-DD`, hogy mindig legyen hova visszalépni.
