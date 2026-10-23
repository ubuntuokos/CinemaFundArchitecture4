# CFA4 – ellenőrzött állapot (2026-10-23, `cfa4-rename` ág)

Ezt a fájlt add be minden új MI-beszélgetés elején, és kérd meg, hogy ehhez képest dolgozzon.
Ami itt áll, azt lefuttatva ellenőriztük; ami nincs ellenőrizve, azt külön jelöltük.

## Mi ez a projekt
- Rust nyelvű videóvágó, a FilmCraft 0.10.0 (`cdf43fd`, ArtCraft csapat) módosított változata.
  Az eredet és a változtatások leírása a `README.md` "Origin" fejezetében van. Licenc: MIT OR Apache-2.0.
- A `main` ága a 2026-10-23-i "forrás leválasztással" (source detachment) áll. Ez az ág öt commitot tesz rá.

## Mi változott ezen az ágon (a main-hez képest, 5 commit)
1. **Munkaterület-javítás (`Cargo.toml`).** A `vendor/craft-libs/*` kizárása nem működött (a `exclude` nem
   kezeli a `*`-ot), ezért a vendorolt `craft-ui` és `craft-docpath` munkaterület-tagként viselkedett, és a
   `cargo test --workspace` / `cargo clippy --workspace --all-targets` elbukott a tesztjeiken. Most név szerint
   ki vannak zárva (tagok: 56 → 54). A `Cargo.lock` nem változott.
2. **ArtCraft-jelzések eltávolítva** (a márkalicenc ezt módosított változatnál megköveteli): `docs/brand/`,
   a logót kirajzoló `brand.rs` és hívásai, `ATTRIBUTION.md` és `AGENTS.md` vonatkozó részei.
3. **Közösségi linkek eltávolítva** (Discord, weboldal, getartcraft): Súgó menü, fejléc, Névjegy, Kezdőlap,
   macOS natív menü. Az appon belüli linkek, a csomagolási URL-ek és az AppImage frissítési forrása a
   `ubuntuokos/CinemaFundArchitecture4` repóra mutatnak.
4. **A termék neve a felületen CFA4** (ablak- és párbeszédcímek, menük, Névjegy, üzenetek, beállítási
   megjegyzések, fájltípus-címkék, Windows fájltulajdonságok, weboldal). Minden átírt angol szöveg a
   forrásban és mind a 11 fordítási katalógusban együtt változott.
5. **App-azonosító: `io.github.ubuntuokos.cfa4`** (régi: `ai.storyteller.filmcraft`): Linux-csomagolás,
   ikonfájlok és sidecar-ok, `ATTRIBUTION.md` sorai, `omastore.toml`, macOS `Info.plist.in`, FreeBSD és
   Flatpak fájlok. A Linux metaadat fejlesztője: `CFA4 maintainers`.
6. **README** újraírva. A "Goal of CFA4" mondat a te megfogalmazásod alapján készült: hibrid videó-előállító, szerkesztő és
   publikáló alkalmazás, amely a klasszikus digitális eszközöket és a mai MI-alkalmazásokat ötvözi. Ez cél, nem
   meglévő funkciólista: az MI-funkciók még nincsenek megvalósítva.

## Ellenőrzés eredménye (Linux, rustc 1.97.0)
| Parancs | Eredmény |
|---|---|
| `PYTHONPATH=python python3 -m cfa4_devtools.dependency_audit` + `unittest` (5 teszt) | OK |
| `cargo check --workspace --all-targets --locked` (az asztali appot is beleértve*) | OK, 0 figyelmeztetés |
| `cargo test -p filmcraft-ui-egui --lib` + 8 érintett integrációs teszt (links, chrome, menus, settings, sequence, graphics_templates, clipboard_paste, shared_docking) | mind OK (196 egységteszt) |
| `cargo test -p filmcraft-engine --lib` | 651 OK, **2 bukik** (lásd lent) |
| `cargo test -p filmcraft --bin filmcraft`** | 74 OK, **1 bukik** (lásd lent) |
| `python3 packaging/linux/test_install.py` | 6 OK |
| `cargo xtask assets` / `layers`, `cargo fmt --check`, `cargo clippy` (ui-egui, engine, filmcraft; `-D warnings`) | OK |

\* Az ellenőrző környezetben nincs ALSA fejlesztői csomag, ezért az app típusellenőrzéséhez egy csak
ellenőrzésre való üres `alsa.pc` leírót használtunk.
\*\* Az app tesztjeinek futtatásához üres ALSA-szimbólumokat adtunk át (`LD_PRELOAD`), csak a tesztekhez.

## Már a változtatások előtt is bukó tesztek (a mi változtatásainktól függetlenek)
- `export_tests::every_builtin_preset_exports_and_ffprobe_confirms`: az itteni ffprobe (6.1.1) nem ismeri az APV kodeket.
- `nest_editing_tests::with_the_toggle_off_a_sequence_edits_in_as_its_clips_with_their_transitions`: az átnevezés
  előtti commiton is bukik; okát nem vizsgáltuk.
- `dev_icon::tests::desktop_file_matches_the_packaged_fields`: a csomagolt `.desktop` fájlban van `MimeType=` sor
  (upstream commit 69a6c4b óta), a fejlesztői generátor viszont nem írja ki.

## NEM ellenőrzött
- A teljes `cargo test --workspace`, `cargo xtask ci`, `cargo xtask wasm`, a GitHub Actions futása.
- Az app indítása és kézi kipróbálása; a csomagolók tényleges futtatása (Flatpak, AppImage, deb/rpm, macOS, Windows).

## Szándékosan NEM változott (következő lépések)
- **macOS, Windows, FreeBSD és web csomagolás megjelenített nevei** még "FilmCraft" (`FilmCraft.app`, a `Contents/MacOS`
  bináris, DMG-elrendezés, MSI termék- és könyvtárnév, `packaging/web/README.md`). Ezek egymásra épülnek, és itt nem
  futtathatók; macOS-en/Windowson kell kipróbálni. (A macOS `Info.plist` azonosítói már az új app-azonosítót használják.)
- **Belső azonosítók és kimeneti fájlokba írt nevek**: crate- és binárisnevek, parancsazonosítók, projektfájl-metaadat,
  OMF/XMP/MXF-címkék, kodeknevek, beállítási mappák (`FilmCraft`), a projektben tárolt renderernevek, naplósorok.
- **Az alkalmazás ikonja még a FilmCraft bagolya** (MIT OR Apache-2.0, `ATTRIBUTION.md`).
- A Linux karbantartó-cím: `CFA4 maintainers <ubuntuokos@users.noreply.github.com>`. Írd át, ha mást szeretnél.
- `AGENTS.md`, `CLAUDE.md`, `docs/contributing.md` stb. még a `storytold/craftrules` szabványokra és a
  `storytold/craft-fonts` opcionális betűtípus-tárra hivatkoznak (a betűtípus nem build-függőség). A
  `contributors/contributors.json` repo-mezője is még `storytold/filmcraft`.
- A vendorolt kód (`vendor/craft-libs/`) változatlan, bájtra az eredeti, ahogy a manifest rögzíti.

## Szabályok, hogy ne csússzon el a fejlesztés
- Egy beszélgetés = egy kis, konkrét feladat. Utána commit.
- Az MI csak azt módosíthatja, amit a feladat megnevez.
- Állítást csak akkor fogadj el az állapotról, ha ebből a fájlból vagy egy lefuttatott parancs kimenetéből következik.
- Működő állapotot címkézz: `git tag jo-YYYY-MM-DD`, hogy mindig legyen hova visszalépni.
