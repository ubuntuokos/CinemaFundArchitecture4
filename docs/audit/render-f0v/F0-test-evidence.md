# F0: tesztállapot és akadályok (F0b/F0V 1. feladat)

**Szabály:** amit teszt vagy mérés nem támaszt alá, az PENDING. A PR-adatok a GitHubról, a futtatási adatok az ellenőrző konténerről jönnek
(Ubuntu 24.04.5, kernel 6.18.44, Rust 1.97.1); a referenciagép (Ubuntu 26.04) adatai PENDING.

## PR #271 és #331

| PR | Állapot | Címke | Tesztelhetetlen vagy kihagyott rész (a PR szövege szerint) |
|---|---|---|---|
| #271 (kodek-összevonás, 2026-10-27) | beolvasztva | `cfa4-full-validation` | `heif-enc`/libheif hiányában a HEIF-tesztek; f16 pixelformátum hiányában az EXR és YCbCr TIFF orákulumok |
| #331 (G9: csak Ubuntu 26.04, 2026-11-09) | beolvasztva | `cfa4-full-validation` | az FFmpeg ≥ 9.0.2 és a `heif-enc` hiánya miatt: EXR/HEIF/TIFF-YCbCr orákulumok, HEIC export round-trip, `cli phone_vfr` workflow; a `main` teljes futása a J2K orákulumon piros volt |

Ezek az ismert eltérések (KD-01…KD-04, lásd [`F0V-known-deviations.md`](F0V-known-deviations.md)). A jelenlegi `main` teljes CI-állapotát ez a PR nem
vizsgálta; a `full-validation-gate` a címke nélkül tervezetten piros.

## Qt6 funkcionális akadályok

A terv szerint a Qt6 GUI készültsége kb. 53% (közlés, 2026-10-28), a paritáshibák listája nincs csatolva. A repó saját listája: [`docs/qt-parity.md`](../../qt-parity.md);
a Qt GUI tesztjei (`test_qt_gui.py`) futnak, de nem kapuznak, amíg a GUI nincs kész (`docs/coordination.md` §3.4). A főszál-watchdog (AC-09) nem létezik
(G-27). Az akadálylista pontosítása a `qt-parity.md` egyenkénti ellenőrzését igényli: **PENDING** (nem végeztem el ebben a körben).

## R0 NVENC/NVDEC újrafuttatás

Az ellenőrző konténerben nincs GPU (a harness `GPU: none (CPU path)`-ot jelent). Az R0 újrafuttatása HEAD-en a referenciagépen szükséges:
**PENDING**. Az utolsó igazolt eredmény: [`R0-FFMPEG-HWACCEL-2026-10-22.md`](../R0-FFMPEG-HWACCEL-2026-10-22.md) (FFmpeg 9.0.2, NVENC/NVDEC: PASS).

## Ebben a körben futtatott ellenőrzések

| Mit | Eredmény |
|---|---|
| `cargo fmt --check`, `cargo clippy -D warnings`, `xtask layers`, `xtask assets`, tesztek a `cfa4-metrics` és `cfa4-golden` crate-en (PR #1427) | zöld |
| `render_fixtures.py generate` és `verify` az FFmpeg 6.1.1 (Ubuntu) konténer-buildjével | 13/13 fixture OK (a referenciagépen az FFmpeg 9.0.2-vel ismétlendő) |
| teljes `cargo xtask ci` és a teljes workspace-teszt | **nem futott** (a teljes validáció a `cfa4-full-validation` címkével jön) |
