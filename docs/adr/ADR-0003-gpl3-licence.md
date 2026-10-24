# ADR 0003 – GPL-3.0-or-later licenc, kódátvétel és üzleti modell

**Dátum:** 2026-10-24 · **Állapot:** elfogadva (a tulajdonos, `ubuntuokos` döntése)

## Háttér

A CFA4 a FilmCraft (MIT OR Apache-2.0) továbbfejlesztése. A cél, hogy a lehető legkevesebbet kelljen
újraírni: a Kdenlive, MLT, Shotcut, Audacity, Subtitle Edit és Krita képességei közül sok GPL-kódban
már megvan. Az örökölt szabályok viszont csak engedékeny licencet és teljes „tisztaszobás”
fejlesztést engedtek.

## Döntések

1. **A CFA4 licence GPL-3.0-or-later** (`LICENSE`, `NOTICE`). Az örökölt FilmCraft-kód MIT/Apache
   megjegyzései megmaradnak (`LICENSE-MIT`, `LICENSE-APACHE`).
2. **A kodekek és konténerek tisztaszobásak maradnak.** Az L0 kodek- és konténer-crate-ek, valamint
   minden kód, ami tömörített bitfolyamot vagy konténerformátumot olvas vagy ír, továbbra is csak
   szabványokból készül, harmadik fél kodekkódja nélkül (FFmpeg, x264, x265, LAME, libvpx, dav1d…).
3. **Máshol GPL-3-kompatibilis kód átvehető:** szerkesztés, idővonal, hangszerkesztés és -effektek,
   feliratozás, rajz és animáció, felület, eszközök. Megengedett: GPL-3.0(-or-later),
   GPL-2.0-or-later, LGPL-2.1-or-later, LGPL-3.0, MPL-2.0 és az engedékeny licencek. Tilos:
   GPL-2.0-only, AGPL, korlátozó vagy tisztázatlan licenc. Az átvett kód megtartja az eredeti
   megjegyzéseit, és bekerül a [`THIRD_PARTY_CODE.md`](../../THIRD_PARTY_CODE.md) nyilvántartásba.
   A never-crash, rétegzési és tesztelési szabályok rá is vonatkoznak.
4. **Függőségek:** ugyanez a licenclista; a `deny.toml` (`cargo deny check licenses`) ellenőrzi.
5. **A beépített grafikai sablonok és a generált teszt- (golden) képek MIT OR Apache-2.0 licencűek
   maradnak.** Ez összefér a GPL-3-mal. A hozzájárulók saját új eszközei (ikon, kép) a projekt
   licencét kapják (GPL-3.0-or-later).
6. **Üzleti modell:**
   - A kész CFA4 ingyenes és szabadon terjeszthető.
   - A fizetős kiegészítők, mivel a programmal összekapcsolva futnak, **GPL-3.0-or-later licencűek**.
     Eladhatók, de a vevő a forráskóddal együtt továbbadhatja őket.
   - A **hosszabb támogatási idő szolgáltatásként** fizetős; ez nem ütközik a GPL-lel.
   - **Bináris terjesztésnél** (`.deb`, AppImage, Flatpak, `tar.gz`) a megfelelő forráskódot is
     elérhetővé kell tenni (GPL-3 6. pont): minden kiadáshoz forrásarchívum vagy a pontos
     forrásverzióra mutató hivatkozás. Ezt a kiadási folyamatba (F3) be kell építeni, mielőtt a
     kiadási zár (HOLD) feloldódik.

Ez a döntés nem jogi tanácsadáson alapul; a fizetős modellt kiadás előtt érdemes szakemberrel
átnézetni.

## Következmények

- A kodekek továbbra is a saját, szabványból írt implementációk; a szabadalmi és tisztaszobás
  helyzetük nem változik.
- Más területeken a meglévő GPL-projektekből átvett kód gyorsíthatja a munkát, de minden átvételt
  dokumentálni kell, és a Rust-szabályok szerint kell átírni.
- Az Apple- és Microsoft-áruházas terjesztés a GPL miatt nehezebb; ez a CFA4-nek (csak Linux) nem cél.
