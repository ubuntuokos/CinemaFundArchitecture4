# CFA4-SETTINGS-HW-002 – a Hardver és eszközök oldalak a valódi Qt6 szerkesztőben

**Állapot:** FINAL 1.0 – a tulajdonos 2026-11-27-én jóváhagyta és véglegesítette (a D1–D3 a javaslat
szerint: D1 igen, D2 serve-metódus, D3 a kísérleti héj marad). A terv minden további módosítása után a
munka a tulajdonos újabb jóváhagyásáig áll ([`docs/coordination.md`](../coordination.md) §2.10).
**Dátum:** 2026-11-27 · **Tulajdonos:** ubuntuokos
**Projekt:** „CFA4 hardver beállítása” (Beállítások ▸ Hardver és eszközök almenü).
**Előzmény:** a közös menüterv [`CFA4-SETTINGS-MENU-001.md`](CFA4-SETTINGS-MENU-001.md) (2026-10-26),
1. fázis: PR #122, 2. fázis: PR #169 (mindkettő a `main`-ben); leírás: [`docs/settings-menu.md`](../settings-menu.md).

## 1. Mit találtam (`main`, 2026-11-27)

1. **Az 1–2. fázis oldalai nem a szerkesztőben vannak.** A csoportos bal lista, a *Gyorsítók (GPU / NPU)*
   és a *Dekódolás és kódolás* oldal a kísérleti Qt-héjban él
   (`python/cfa4_devtools/qt_app.py` → `qt_settings.py`, `settings_backend.py`; a fájl maga írja:
   „Experimental Linux Qt 6 shell. NOT the complete video editor”). A valódi szerkesztő, a `cfa4-qt`
   (`python/cfa4_qt/`, [`docs/qt-gui.md`](../qt-gui.md)) Beállítások ablaka
   (`python/cfa4_qt/panels/settings.py`, `PreferencesDialog`, Edit ▸ Preferences, Ctrl+,) az egui-hoz
   igazodó lapos listát mutat: csoportfejléc, Gyorsítók és Dekódolás és kódolás oldal nincs benne.
   A felhasználó tehát a mai szerkesztőben az eddigi Hardver-munkából semmit nem lát.
2. **A motor sémája már kész hozzá.** A `prefs.schema` a `groups` listát is visszaadja
   (`crates/engine/src/settings.rs`, `groups()`); a `hardware` csoportban ma: Audio Hardware, Control
   Surfaces, Memory, Timecode Sync (és a Qt-only Gyorsítók). A `cfa4-qt` a `groups` mezőt nem olvassa.
3. **Az adat csak külön CLI-hívással érhető el.** A `cfa4-cli hardware` és `cfa4-cli codecs`
   (`apps/cfa4-cli/src/settings_cmd.rs`, `codecs_cmd.rs`) adja a gyorsító-leltárt és a kodektáblát; a
   `cfa4-qt` viszont csak a `cfa4-cli serve` csatornán beszél a motorral, ott ilyen metódus nincs.
   A *Media Backends* már a szerkesztőben van (`python/cfa4_qt/media_backends.py`, `media.backends`
   lekérdezés), ehhez ez a terv nem nyúl.
4. **A menüterv repón kívül volt** (Claude-dokumentum), ami a `docs/coordination.md` §2.11 szerint nem
   maradhat így. Ebben a PR-ban változatlan tartalommal átkerül:
   [`CFA4-SETTINGS-MENU-001.md`](CFA4-SETTINGS-MENU-001.md).

**Következtetés:** a logikus következő lépés nem új oldal, hanem az, hogy a már elkészült Hardver-oldalak
a valódi szerkesztőben is megjelenjenek. Az eredeti ütemezés 3. szakasza (root-keret) erre épül, ezért
utána jön, külön tervvel (6. pont).

## 2. Cél és hatókör

**Cél:** a `cfa4-qt` Beállítások ablakában a bal lista a séma csoportjai szerint rendeződik, és a
*Hardver és eszközök* csoportban megjelenik a *Gyorsítók (GPU / NPU)*, a *Média és lejátszás* csoportban
a *Dekódolás és kódolás* oldal, ugyanazzal a tartalommal és automatizálási azonosítókkal, mint a
kísérleti héjban.

**Nem része:** új beállítási kulcs; root-művelet; eszközoldalak (kijelzők, tárolók, kamerák, rajztáblák);
GPU Unlock; a kísérleti héj törlése; az egui (befagyasztva, nem bővítjük); a Media Backends ablak, a
Control Surfaces és a Timecode Sync oldal (más projektek).

## 3. Lépések (egy PR, külön commitokkal)

| # | Lépés | Fájlok | Átvétel |
|---|---|---|---|
| H1 | **Serve-metódusok:** `hardware.inventory` (a `settings_cmd::hardware()` kimenete) és `codecs.capabilities` (a `codecs_cmd::codecs()` kimenete). Az eredményt a serve-munkamenet megjegyzi; `{"refresh": true}` újra felmér. A JSON-alak azonos a CLI-ével, így a meglévő Python-adatmodell (`Hardware`, `Codecs`) változatlanul olvassa. | `apps/cfa4-cli/src/serve.rs` (+ a két meglévő modul újrahasznosítása) | Rust-teszt: mindkét metódus válaszol hamis sysfs-szel és GPU nélkül is (CPU-tartalék sor); a `cfa4-cli hardware/codecs` kimenete változatlan. |
| H2 | **Csoportos bal lista:** a `PreferencesDialog` a `prefs.schema` `groups` sorrendjében csoportfejléceket rajzol (nem kattintható címke), alattuk behúzva az oldalak. Ha egy régebbi motor nem ad `groups`-t, marad a mai lapos lista. Az oldal- és kulcsazonosítók, az Edit ▸ Preferences ▸ <oldal> menük nem változnak. | `python/cfa4_qt/panels/settings.py` | Új automatizálási azonosító: `settings.group.<id>`. Teszt: a sorrend a sémáé; régi séma esetén lapos lista. |
| H3 | **Gyorsítók oldal** a szerkesztőben: csak olvasható; eszköztábla (típus, eszköz, PCI-azonosító, gyártó, meghajtó, csomópont) és munkatípusonkénti háttérsor („Tried in this order”, „Not used” indokkal), „Detect Again” gomb. A hívás aszinkron (`engine.call`), a GUI-szál nem vár. A PCI-név a `pci.ids` fájlból jön, ahogy ma. | `python/cfa4_qt/panels/settings.py` (vagy új `python/cfa4_qt/panels/settings_hardware.py`, ha a fájl túl nagy lenne) | Azonosítók a kísérleti héjjal azonosak: `settings.accelerators.devices`, `.plans`, `.refresh`. Teszt: hamis válasszal GPU nélkül („No GPU or NPU found…”), VA-API GPU-val és NVIDIA-only háttér kihagyásával. |
| H4 | **Dekódolás és kódolás oldal** a szerkesztőben: a `playback.hardwareDecoding` és `playback.draftDecode` mező (ugyanazok a kulcsok, a Playback oldalon is megmaradnak, ahogy ma az egui-ban), és a csak olvasható „On this machine” tábla (hardveres dekóder, GStreamer állapota, kodekenkénti sor). Ha nincs hardveres dekóder, kiírja, melyik gyártói meghajtót kell telepíteni; a CFA4 semmit nem telepít. Reset Page itt nem aktív (a kulcsok a Playback kategóriáé). | ugyanott | Azonosítók: `settings.codecs.table`, `.hardwareDecoder`, `.gstreamer`, `.refresh`. Teszt: hamis válasszal, és a valódi motorral oda-vissza, ha a `cfa4-cli` elérhető. |
| H5 | **Dokumentáció és ellenőrzés:** `docs/settings-menu.md` (melyik ablak a mérvadó, új azonosítók), `docs/qt-parity.md` 4.5 sor kiegészítése (Qt-only többlet), `docs/qt-gui.md`. Képernyőkép: `cfa4-qt --smoke --screenshot` a két új oldalról, összevetve a projekt `settings-menu/` képterveivel. | docs | A kapuk zöldek: `cargo xtask ci`, `cargo test --workspace --exclude cfa4-ui --release`, `ruff`, `mypy` (az új fájl a kivétellista nélkül), `pytest`. |

A szövegek angol forrással kerülnek a felületre, a magyar fordítás az `i18n/hu.tsv`-be (csak saját sorok,
§2.5).

## 4. Bizonyíték és mérés

- A felhős CI-ben nincs GPU: ott a CPU-tartalék és a hamis-sysfs ágak futnak.
- Valódi gépen: a referenciagépen (RTX A1000 a kijelzőhöz, RTX A4000 számításra,
  [`docs/development/reference-machine.md`](../development/reference-machine.md)) a Gyorsítók oldalnak
  mindkét kártyát mutatnia kell, a Dekódolás és kódolás oldalnak az NVDEC-et. Ha ez a mérés a tulajdonos
  döntése szerint elmarad, sort kap a
  [`docs/development/deferred-physical-tests.md`](../development/deferred-physical-tests.md) listában
  (§2.14).

## 5. Kockázat

- A `codecs` felmérés hardveres dekódert regisztrál, ami pár száz ms-ig tarthat; ezért a serve az
  eredményt megjegyzi, és csak „Detect Again”-re mér újra. A szerkesztő közben nem akad meg (aszinkron
  válasz).
- A `python/cfa4_qt/panels/settings.py` közös fájl (Control Surfaces, Timecode Sync is benne van): csak
  új függvényeket és a bal lista építését módosítom, mások részeit nem rendezem át (§2.5).
- A PR nem kockázatos a §2.9 szerint (nincs `crates/platform`, kodek-crate, CI vagy szabályfájl): CodeRabbit nem kell.

## 6. Utána (külön terv, külön jóváhagyás)

A terv: [`CFA4-SETTINGS-HW-003.md`](CFA4-SETTINGS-HW-003.md) (jóváhagyva 2026-11-28).

1. **Root-keret** (a menüterv 3. szakasza): `cfa4-system-helper` `pkexec`-kel, tervezet → jóváhagyás →
   végrehajtás, első komponensek: `render`/`video` csoporttagság, VA-API csomagok, NPU-ellenőrzés; előtte
   ADR a rendszerszintű komponensekről.
2. **Áttekintés oldal** és a közös eszközoldal-minta (lista + fülek).
3. A kísérleti héj Beállítások ablakának sorsa (megtartás tesztorákulumnak vagy törlés): a tulajdonos dönt.

## 7. Döntések (eldöntve 2026-11-27, a javaslat szerint)

| # | Kérdés | Javaslat |
|---|---|---|
| D1 | Jóváhagyod-e, hogy a következő lépés a meglévő Hardver-oldalak átvitele a `cfa4-qt` szerkesztőbe (H1–H5), és a root-keret csak utána jön? | Igen: amíg a szerkesztőben nincs meg, a root-oldalnak sincs hova kerülnie. |
| D2 | Az adat a `cfa4-cli serve` saját metódusaként jöjjön (`hardware.inventory`, `codecs.capabilities`), ahogy a `surface.devices`, vagy motorparancsként (`crates/engine`)? | Serve-metódus: a kodektábla a `cfa4-platform` regisztrációjára és az export-formátumok próbájára épül, amely ma csak a CLI-ben van (a motor a `cfa4-route`-ra épül, a `cfa4-platform`-ra nem); így a két metódus egy helyen marad, és a CLI-vel azonos kódot futtat. Ha motorparancsot választasz, a `hardware.inventory` mehet a motorba, a kodektábla viszont csak a platformfüggőség felvételével. |
| D3 | A kísérleti héj (`cfa4_devtools/qt_settings.py`) maradjon-e változatlanul ebben a lépésben? | Igen, maradjon; a sorsáról külön döntés (6.3). |
