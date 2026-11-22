# VFX-0a – donor-pillanatkép (CFA4-VFX-001)

**Dátum:** 2026-11-22 · **Állapot:** rögzítve, olvasással; kód nem került át, a regiszter (`docs/licensing.md` §5) még nem kap sort.

A tulajdonos döntése (2026-11-22): a donorok a mai állapoton, befagyasztva maradnak a VFX-terv megvalósításának végéig, nem frissülnek, és a fork leválasztása nem előfeltétel. Ha frissítés kell, a tulajdonos jelzi a fejlesztő szálnak. A donorokba nem írunk.

**Hatókör (2026-11-22):** csak Linux; a macOS-, Windows-, FreeBSD- és web-specifikus részek nem képezik a pillanatkép és az átvétel tárgyát (`CFA4-VFX-001` 2. fejezet).

A márkaszűrő (`python/tests/test_independence.py`) miatt a donor-repók neve és URL-je itt nem szerepel; a tulajdonosi nyilvántartásban a megfelelő `ubuntuokos/CFA4_*` repók azok.

| Donor | Rögzített `HEAD` (SHA) | A SHA kelte | Licenc (a repó fájljai szerint) | Megjegyzés |
|---|---|---|---|---|
| EC-donor (#11), Rust | `171e92b0772019357fa3bea724067a32c5875ad7` | 2026-11-22 | `MIT OR Apache-2.0` (`Cargo.toml`, `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`) | Az upstream márkajelei és logói **nem** nyílt licencűek (`NOTICE`); ezek nem vehetők át. A harmadik fél anyagai (betűtípusok stb.) a `NOTICE`/`ATTRIBUTION` szerint külön licencűek |
| Gaffer-donor, C++/Python | `14c1a3f22599423fa48d5328f54d1393eb0ac25f` | 2026-11-20 | BSD-3-Clause (`LICENSE`: © 2011-2019 John Haddon, Image Engine Design Inc.) | Átvételnél a licencszöveg és a szerzői jogi fejléc megtartandó |
| Lumit-donor, Rust+wgpu+Flutter | `5567ef97dbbf19cb58ccdb5f119c799eae70592c` | 2026-11-21 | `GPL-3.0-only` (`Cargo.toml`, `LICENSE`) | A tulajdonos elfogadta (2026-11-22): az átvett fájl `GPL-3.0-only` marad (SPDX-fejléc + regisztersor). A Flutter UI nem használható |

## Nem rögzített (nyitott)
- **Elágazási pont SHA** (a szülővel közös utolsó commit) az EC-donornál és a Gaffer-donornál: a szülő története nem lett beolvasva. Átvétel előtt (VFX-0b) pótolni kell, ha az eredet-regiszterhez szükséges.
- A szülők `LICENSE`/`NOTICE` fájljának külön másolata: a donor-repók saját fájljai megfelelnek rá, de a szülő állapotát nem ellenőriztük.

## Következő lépés
VFX-0b: a donor-licencregiszter és a fájlszintű eredetfejléc szabálya, `cargo deny` és `docs/licensing.md` §5 illesztése (külön PR, a gazdával egyeztetve).
