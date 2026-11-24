# ADR-JAVASLAT – Matte-sík a C0 szerződésben (CFA4-GREENSCREEN-MATTING-INTEGRATION-001, Z1b)

**Dátum:** 2026-11-24 · **Állapot:** JAVASLAT, nem elfogadott; sorszámot a C0 gazdája ad az elfogadáskor ·
**Terv:** [`CFA4-GREENSCREEN-MATTING-INTEGRATION-001-0.1`](../plans/CFA4-GREENSCREEN-MATTING-INTEGRATION-001-0.1.md) 7. pont (Z1b), S3–S6 ·
**Címzett:** a `cfa4-render-contracts` (C0) gazdája, [`CFA4-RENDER-INTEGRATION-PLAN-001`](../plans/CFA4-RENDER-INTEGRATION-PLAN-001.md) A2 és C0 ·
**Érintett rész:** a C0 Frame ABI v0 mezőkészlete; ez a javaslat kódot nem változtat, crate-et nem hoz létre.

## 1. Miért kell

A kulcsolók (Ultra Key, Color Key, Luma Key) és az AI-matte (BMV2, RVM, `ai.matting.*`) ugyanazt állítják elő: **folytonos, egycsatornás alfát**.
Ma a matte nem önálló érték: csak a kulcsoló effekt kimenete (terv H6), az Object Mask `MaskRef` tárolója pedig bináris (0/1 futáshossz), finom hajszálakra nem alkalmas.
A Z0 mérés ([`GREENSCREEN-Z0-KEYER-MEASUREMENT-2026-11-24.md`](../audit/GREENSCREEN-Z0-KEYER-MEASUREMENT-2026-11-24.md)) szerint a két kulcsoló ellentétesen téved, ezért a módszerek közti váltás (S17) csak közös matte-szerződéssel lehet nem romboló.

## 2. Javaslat

A Frame ABI v0 (A2) kapjon **matte-sík** értéktípust. Ez nem párhuzamos típusrendszer: a meglévő `Image` alfa-csatornájának és a `Track Matte Key` bemenetének formalizálása.

| Mező | Javasolt tartalom | Indok |
|---|---|---|
| Elemtípus | egycsatornás sík: `u8`, `u16` vagy `f32` | a meglévő renderer f32; a 8/16 bit tároláshoz és exporthoz |
| Értéktartomány | [0, 1], **adat, nem szín**: nincs transfer-függvény, nincs színtér-címke | a mai „Alpha Channel” kimenet sRGB→lineáris átalakítással jelenít meg szürkét; a matte értéke nem színérték (lásd 3. pont) |
| Alfa-mód | a matte **mindig straight** (nem premultiplikált); a premultiplikálás a kompozitor lépése | a C0 A2 alfa-mód mezőjével egyező szemantika, kétértelműség nélkül |
| Méret | szélesség × magasság, stride; azonos a forráskép méretével, vagy explicit skálázási szabály | a konverziós mátrix nem támogatott párja explicit hiba |
| Időbélyeg | `Tick` (racionális időalap), a forrás képkockájához kötve; VFR esetén nincs csendes kerekítés | A2 időmodell |
| Forrás-azonosító | a forrás (klip, képkocka, timecode) azonosítója, hogy a proxy–eredeti megfeleltetés megmaradjon | A2 kiegészítés (2026-11-23) |
| Eredet (provenance) | módszer (`key.ultra`, `key.color`, `key.luma`, `ai.matting.*`), paraméter-hash, AI esetén modell-azonosító és SHA-256 | a `ObjectTrack` provenance mintájára; S6 |
| Hivatkozás | tartalomcímzett: `MatteRef { sha256, width, height, depth }`, a tár a meglévő lemezes gyorsítótár; **nem** új adatbázis | S4; PostgreSQL a job-állapotra, nem a matte-re |

A `MaskRef` (bináris RLE) **változatlan marad**; a matte-sík és a maszk külön típus. Konverzió: maszk → matte (0/1) veszteségmentes; matte → maszk küszöböléssel, a küszöb explicit paraméter.

## 3. Amit a szerződésnek tisztáznia kell

1. **„Alpha Channel” kimenet.** Az Ultra Key `output` = 1 ágában a megjelenített szürke `srgb_to_linear(a)`, az alfa-csatorna a forrás alfája marad. Ez megjelenítési segéd, nem matte-érték; a matte-sík kimenet ettől külön, transfer nélküli adat legyen, különben a matte értéke az export és a fogyasztók felé kétféleképp értelmezhető.
2. **Premultiplikálás sorrendje.** A kulcsoló ma a kimeneti színt `a · src_a`-val premultiplikálja. A szerződés rögzítse, hogy a matte straight, és a premultiplikálás a Compositor dolga.
3. **Mélység.** 8 bit az előnézet és a gyorsítótár, 16 bit az alfa-megőrző exportokhoz, f32 a belső munkához; a konverzió kerekítése deterministikus (MQ-6 bit-azonosság).
4. **Fogyasztók.** Track Matte Key, klip-alfa, blend, alfa-export (`supports_alpha`) ugyanazt a matte-síkot kapják; további fogyasztó csak a szerződésen át.

## 4. Elfogadási bizonyíték (Z1b átvétele)

- Elfogadott ADR (sorszámmal), a C0-contracts-spec része.
- Hostile-input teszt: NaN, ±inf, 0 méretű és túlméretes sík, nem egyező méret, hibás hash: nincs pánik, explicit hiba.
- Régi-projekt teszt: a meglévő projektek (matte nélkül) változatlanul megnyílnak, a golden változatlan.
- `cargo xtask layers` zöld (a matte-típus a megfelelő rétegben van, nincs fordított függőség).

## 5. Döntésre vár (a C0 gazdájánál, nem a tulajdonosnál)

1. A matte-sík a `cfa4-render-contracts` típusa legyen-e, vagy a C0 elfogadásáig a `cfa4-project` marad a gazdája.
2. Egy vagy három elemtípus (`u8`/`u16`/`f32`) szerepeljen-e a v0-ban.
3. A matte-tár kulcsa: csak tartalom-hash, vagy hash + módszer + paraméter-hash (verziózott matte-ek, Z3).

**Nem része:** AI-modellek, licenc, a Model Manager szerződése (külső tároló), a kulcsolók kódja.
