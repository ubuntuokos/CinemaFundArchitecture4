# VFX-0b – donor-licenc- és eredetaudit (előzetes)

**Dátum:** 2026-11-22 · **Terv:** `CFA4-VFX-001` §5 (VFX-0b) · **Alap:** `docs/plans/CFA4-VFX-001-VFX-0A-DONOR-SNAPSHOT.md` · **Hatókör:** csak Linux (terv 2. fejezet)

Olvasással készült (a függőségek forrásának letöltésével, fordítás nélkül). Ez nem jogi tanács (`docs/licensing.md`); átvétel előtt a tulajdonos és a gazda jóváhagyása kell. A márkaszűrő miatt a donor- és könyvtárnevek általánosan szerepelnek.

## 1. Licencek donoronként

| Donor | Licenc | Átvétel a CFA4-be (GPL-3.0-or-later) | Feltétel |
|---|---|---|---|
| EC-donor (#11) | `MIT OR Apache-2.0` | megengedett (`docs/licensing.md` §3) | eredeti szerzői jogi fejlécek megtartása; regisztersor (§5); a donor `NOTICE`-a szerinti harmadik fél anyagai külön licencűek |
| Gaffer-donor | BSD-3-Clause | megengedett | licencszöveg és szerzői jogi fejléc megtartása; a szerzők neve nem használható támogatásra |
| Lumit-donor | `GPL-3.0-only` | megengedett, de csak `GPL-3.0-only` marad (tulajdonosi döntés, 2026-11-22) | fájlszintű `SPDX-License-Identifier: GPL-3.0-only` fejléc; regisztersor; a `GPL-3.0-or-later` kikötés ezekre a fájlokra nem terjed ki |

## 2. Az EC-donor Rust-függőségei (git-forrású, Linux-hatókör)

A donor `Cargo.lock`-ja 131 git-forrású csomagot tartalmaz, 5 upstream repóból (a `cargo fetch --locked` sikerült). A letöltött forrás `Cargo.toml` fájljai szerint (a segédkönyvtár-gyűjteménynél egy rögzített revízió mintája; a többi revíziót nem hasonlítottuk össze):

| Forrás (általánosan) | Csomagok | Licenc | Megjegyzés |
|---|---|---|---|
| upstream segédkönyvtár-gyűjtemény | 104 csomag, 13 rögzített revízió | `Apache-2.0 OR MIT` / `MIT OR Apache-2.0` (a többség), 1 csomag `Apache-2.0` | tartalmaz **opcionális DWG-hidat** (`MPL-2.0` harmadik fél, külön függőség); ez **kizárt**, nem vesszük át |
| upstream UI-keretrendszer-fork (egui-fork) | 11 | `MIT OR Apache-2.0`; egy csomag `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0` (betűtípusok) | az egui a CFA4-ben fagyasztott, nem vesszük át |
| upstream szerkesztő-könyvtár | 13 | `MIT OR Apache-2.0` | az átfedő modulokat a gazdaprojektek döntik el |
| upstream vektor-könyvtár | 2 | `MIT OR Apache-2.0` | – |
| ablakkezelő-fork (winit-fork) | 1 | `Apache-2.0` | UI-réteg, nem vesszük át |

Következtetés: a vizsgált git-függőségek mind megengedő licencűek (MIT/Apache-2.0), GPL-3.0-kompatibilisek. Kivételek és figyelendők: az `MPL-2.0` DWG-híd (kizárva), a betűtípusok (OFL-1.1, Ubuntu-font-1.0: külön licencelt eszköz, nem kód), a harmadik fél beágyazott anyagai a segédkönyvtár `NOTICE`/`licenses/` mappájában (pl. PDF- és JPEG-író kódok): átvételkor soronként nézendők. A márkajelek és logók a donor `NOTICE`-a szerint **nem** nyílt licencűek, nem vehetők át.

## 3. Szabály az átvett fájlokra (javaslat a gazdának)
1. Minden átvett fájl fejléce: az eredeti szerzői jogi sor, az eredeti `SPDX-License-Identifier`, és egy sor: `Ported into CFA4 from <donor id> @ <SHA>`.
2. A `docs/licensing.md` §5 regiszterébe sor kerül **fájl- vagy könyvtárszinten**: CFA4-útvonal, eredet (donor, SHA, útvonal), szerzői jogi tulajdonosok, licenc.
3. Lumit-eredetű fájlok: `GPL-3.0-only`; a `packaging/linux/copyright` és a `cargo deny` licenckonfigurációját a gazdával egyeztetve frissíteni kell, mielőtt az első ilyen fájl bekerül.
4. Nem vehető át: macOS-, Windows-, FreeBSD- és web-specifikus rész; márkajel és logó; az `MPL-2.0` DWG-híd; Adobe-eszköz (`AGENTS.md` §1).
5. Elágazási pont SHA (VFX-0a nyitott pontja): a szülő története nem ellenőrzött; az EC- és Gaffer-donor átvétele előtt pótolni kell, vagy a donor `NOTICE`/`LICENSE`-ára kell támaszkodni.

## 4. Nyitott
- A transzitív crates.io-függőségek `cargo deny`-ellenőrzése (a `deny.toml` szabályai szerint) csak a tényleges átvételi PR-nál értelmes, mert ott dől el, mely csomagok kerülnek a CFA4 függőségei közé.
- A harmadik fél beágyazott anyagainak soronkénti átnézése a segédkönyvtár `NOTICE`-a szerint.
