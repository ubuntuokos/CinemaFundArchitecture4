# CFA4-DNEG-TECHNOLOGY-ADAPTATION-001 – G1 csomagfelmérés

Állapot: **KÉSZ (dokumentum), 2026-11-21** · Kapu: G1 (lásd `…-1.0.md` §3) · Csak dokumentáció, kód nem változott.
Q4 lezárva: AX = saját korlátozott interpreter, ezért **LLVM nem függőség** (N3-S4 tárgytalan).

## 1. Módszer és forrás
Ubuntu 26.04 (`resolute`) „Published" forráscsomagjai a Launchpad API-n (`getPublishedSources`, `distro_series=resolute`), a licenc a Debian `copyright` fájlból (unstable; a 26.04-es csomag licence ettől eltérhet). Az ellenőrzés a hálózatról történt, **telepítés vagy letöltés a gépre nem**.

## 2. Eredmény
| Forráscsomag | 26.04 verzió | Komponens | Minimum | Licenc (Debian copyright) | Megfelel? |
|---|---|---|---|---|---|
| openvdb (`libopenvdb-dev`) | 10.0.1-3 | universe | – | Apache-2.0 (fő); BSD-3-clause, Expat; CC-BY-4.0 (dokumentáció); GPL-3+ Bison-kivétellel (generált AX-parser) | igen* |
| onetbb (`libtbb-dev`) | 2022.3.0-2 | universe | TBB ≥ 2021 | Apache-2.0 (+ BSD-3, MIT, BSD-like-bzip2 a mellékelt részekre; GPL-1+/GPL-2 sor a tesztek/doksi fájljaira) | igen* |
| zlib (`zlib1g-dev`) | 1:1.3.dfsg+really1.3.1-1ubuntu3.2 | main | – | zlib | igen |
| libzstd (`libzstd-dev`) | 1.5.7+dfsg-3 | main | – | BSD-3-clause **vagy** GPL-2 (kettős; BSD-3-ot választjuk) | igen |

\* A GPL-sorok a Debian-fájlban a kiszolgált **forráscsomag** részeire vonatkoznak (AX generált parser, teszt/doksi/segédfájlok), nem a `libopenvdb`/`libtbb` futásidejű könyvtárra. Mivel a CFA4 maga GPL-3.0-or-later, Apache-2.0 függőség megengedett; a GPL-3+ rész nem akadály. A pontos fájl-szintű hozzárendelés **ellenőrizetlen** (U-G1-1).

## 3. Megállapítások
1. **OpenVDB 10.0.1** a 26.04-ben: a mag (VDB, Points) rendelkezésre áll; **a minimumok (TBB ≥ 2021) teljesülnek, a G1 megállási feltétele nem lép életbe.** Az N1-nek 10.0.x API-ra kell épülnie; újabb OpenVDB (11/12) csak külön döntéssel, a rendszercsomag helyett.
2. **NanoVDB / AX:** a csomagban elérhetőségük **nem igazolt** (U-G1-2); az N2/N3 feltétele csak akkor, ha kellenek – az N3 interpreterrel nem igényli az AX-et.
3. **Nincs automatikus letöltés:** a build rendszercsomagot használ (`libopenvdb-dev`, `libtbb-dev`), a telepítést a felhasználó végzi (docs/development/linux.md bővítendő az N1-ben).
4. **ADR-0003 táblasor:** „BSD-3 / Apache-2.0" → „Apache-2.0" (Q2) – külön kis PR, még nincs megcsinálva.
5. **SBOM/`cargo deny` terv:** a C++ worker nem Cargo-függőség, ezért az SBOM-ba kézzel vezetett „rendszerfüggőség" sor kerül (név, 26.04-es verzió, licenc, fent); `cargo deny` a Rust-oldali kötéseket (ha lesz) fedi.

## 4. Ellenőrizetlen (nem „kész")
- U-G1-1: fájl-szintű licenc-hozzárendelés (openvdb, onetbb).
- U-G1-2: NanoVDB/AX/Points modulok a 26.04-es bináris csomagban (`apt-file`-szintű lista).
- U-G1-3: a 26.04-es csomag tényleges `copyright` fájlja (a fenti a Debian unstable-ből való).
- U-G1-4: libzstd Debian-copyright URL-je 404 volt, a licenc a `sources.debian.org`/csomag alapján tekintendő; a kettős licenc ismert, de itt nem idéztem szó szerint.

## 5. Következő kapu
G2 (N1 – VDB adatréteg) csak külön engedéllyel, ACS-6 (PR #1471) beolvasása után.
