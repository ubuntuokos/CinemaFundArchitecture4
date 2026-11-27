# VFX-5 P0-1: Spill Suppress (saját kód)

**Dátum:** 2026-11-27 · **Terv:** `CFA4-VFX-001` VFX-5, a jóváhagyott besorolás és sorrend szerint (a G-33 mátrixsor `REFERENCE_ONLY`: az EC-jelölt MQ-3 szigorú olvasata miatt nincs átvétel) · **Jelleg:** új effekt, szintetikus mérés.

## Mi ez
`spill_suppress` („Spill Suppress”, Keying): a kulcsszín uralkodó csatornáját a másik két csatorna átlagára (Average) vagy maximumára (Maximum) korlátozza, az alfa érintetlen; az „Amount” a levonás mértéke, a „Preserve Luminance” a levont fényt adja vissza. A kulcsolókkal azonos, kijelzőkódolt jelen dolgozik. **Saját megvalósítás**, szabványos színkülönbség-alapú despill; külső kód nem került át, a CFA4-azonosító új.
Helye: `crates/project/src/effect.rs` (definíció, `CFA4_OWN`), `crates/render/src/effects.rs` (`spill_suppress`), tesztek: `crates/render/src/spill_tests.rs`.

## Mérés (szintetikus, a Z0 `key_synth` jelenetein; Color Key tol. 40, feather 10 után)
Reprodukálás: `cargo test -p cfa4-render --lib spill_ -- --nocapture`. A „spill” a Z0 mérés definíciója (G − max(R,B), átlag az átlátszatlan előtérben).

| Jelenet | spill a Color Key után | Spill Suppress, Maximum, 100% | Average, 100% |
|---|---|---|---|
| disc | 0,1136 | 0,0000 | 0,0000 |
| hair | 0,3511 | 0,0000 | 0,0000 |
| noisy | 0,1137 | 0,0000 | 0,0000 |

(A Z0 dokumentum Color Key spill-értékei 0,09–0,32; a mostani futás a tesztfordítású, jelenet-azonos számokat adja.)

## Tesztek
amount 0 = bitazonos; az alfa bitazonos és az átlátszó képpont érintetlen; semleges képpont nem mozdul; zöld beütés a max(R,B) alá korlátozva, R és B változatlan; Average ≤ Maximum erősségben, a „Preserve Luminance” a levont fényt visszaadja; a Z0-jeleneteken a spill ≤ 5% az eredetinek.

## Korlátok, amit nem mértem
- A mérés **szintetikus**; a spill-metrika csak a zöld túlsúlyt méri, a színhűséget (az előtér színének megőrzését) **nem**: a Maximum 100%-on a spill-t nullára viszi, de az előtér valódi zöld árnyalatait is érintheti. A színhűség-mérő a következő lépés (P0-2 előtt).
- Csak CPU, nincs GPU-útvonal és nincs sebességmérés (referenciagép PENDING); a YUV/float32 jelzők a definícióban az alapértékek, a jelentésüket nem ellenőriztem.
- UI-t nem ellenőriztem: az effekt az effektregiszterbe került, az effektböngészőben/Effect Controlsban való megjelenését nem néztem meg, UI-teszt nem készült.
- Golden-képek az F0V-harnessen nem készültek ehhez az effekthez.
