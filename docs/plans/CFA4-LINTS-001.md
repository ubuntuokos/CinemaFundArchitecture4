# CFA4-LINTS-001 – Szigorúbb Rust-lintek (elavult API, jövőben inkompatibilis kód, idiomatikus Rust)

**Verzió:** 1.0 (jóváhagyva)
**Dátum:** 2026-11-12
**Projekt:** CinemaFundArchitecture4 (CFA4)
**Állapot:** a tulajdonos (`ubuntuokos`) 2026-11-12-én egy beküldött `clippy.toml` / `Cargo.toml`
javaslatot kért átnézni; az átnézés után javított változatot jóváhagyta.
**Kapcsolódik:** [`Cargo.toml`](../../Cargo.toml) `[workspace.lints]`, [`clippy.toml`](../../clippy.toml),
[`rust-toolchain.toml`](../../rust-toolchain.toml), [`CLAUDE.md`](../../CLAUDE.md) (quality gates),
[ADR 0001](../adr/0001-platform-ffi.md) (a `cfa4-platform` saját lintjei)

---

## I. Cél

Elavult (`#[deprecated]`) Rust API, jövőbeli fordítóval inkompatibilis kód és nem idiomatikus
(2018+) Rust ne kerülhessen a kódba, még figyelmeztetésként sem: fordítási hiba legyen.

## II. Döntések

| # | Döntés |
|---|--------|
| L1 | `[workspace.lints.rust]`: `deprecated = "deny"`, `future_incompatible` és `rust_2018_idioms` csoport `deny`, `priority = -1`-gyel (különben a clippy `lint_groups_priority` hibát ad). A meglévő `unsafe_code = "forbid"` marad. |
| L2 | A `cfa4-platform` (az egyetlen `unsafe`-et tartalmazó crate) saját `[lints]` táblájába ugyanez kerül, mert nem örökli a workspace lintjeit. |
| L3 | Az `xtask` eddig nem örökölte a workspace lintjeit; `[lints] workspace = true`-t kap. |
| L4 | A `rust_2018_idioms` miatti hibák javítása: az `elided_lifetimes_in_paths` lint ~900 helyén a rejtett élettartam kiírása (`Foo` → `Foo<'_>`). Viselkedést nem változtat, csak típusannotáció. |

## III. Amit a beküldött javaslatból szándékosan **nem** veszünk át

| Javaslat | Ok |
|----------|-----|
| `clippy.toml` `msrv = "1.97.1"` | Fölösleges: a clippy a `Cargo.toml` `rust-version = "1.97"` értékét olvassa; két eltérő érték figyelmeztetést ad. |
| `std::env::temp_dir` tiltása | 106 fájl használja; a hivatkozott „izolált munkakönyvtár-kezelő” nem létezik. Ha egyszer lesz ilyen (pl. a CFA4-SECURITY-001 keretében), saját tervvel jön a tiltás is. |
| `std::sync::mpsc::Sender` tiltása | 104 helyen használjuk, crossbeam/flume nincs a projektben; a javaslat saját indoklása szerint is az `mpsc`-nél kell maradni. |
| `std::mem::uninitialized` / `zeroed` tiltása | Az `unsafe_code = "forbid"` már kizárja (mindkettő `unsafe`); az `uninitialized` ráadásul elavult, így a `deprecated = "deny"` is elkapja. A `zeroed` nem elavult; a `cfa4-platform` NVDEC/NVENC FFI-je jogosan használja. |
| `clippy::all` / `clippy::pedantic` workspace-szinten | A CI és a quality gate `-D warnings`-szel futtatja a clippyt, így a pedantic ezernyi figyelmeztetése mind hibává válna. Később crate-enként, kitakarítás után vezethető be (külön terv). |

## IV. Ellenőrzés

- `cargo check --workspace --all-targets` alapfeature-ökkel és minden feature-rel
  (`ffmpeg` az FFmpeg 9.0.2-vel, `gstreamer`, `whisper`, `kokoro`, `threads`, `writer`, `test-util` …): hibamentes.
- `cargo xtask ci` (fmt, clippy `-D warnings`, layers, assets): zöld.
- `cargo clippy --workspace --all-targets --features <minden, a whisper kivételével> -- -D warnings`: zöld.
  A `cfa4-speech` `whisper` feature-ében két clippy-hiba a `main`-en is fennáll (nem ennek a tervnek
  a tárgya; a PR-ban jelezve).
- Teljes tesztfuttatás: `cfa4-full-validation` címkével, a gazda kötegében.
