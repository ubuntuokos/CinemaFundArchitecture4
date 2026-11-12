# CFA4-LINEART-001 – Animált vonalrajz-réteg Rustban (a stáblista-terv C12-je)

**Verzió:** 0.1 (tervezet, egyeztetés alatt)
**Dátum:** 2026-11-12
**Projekt:** CinemaFundArchitecture4 (CFA4)
**Állapot:** a tulajdonos 2026-11-11-én döntött: a vonalrajzos animáció két lépcsőben kerül a
CFA4-be. Az első lépcső (C11: ismétlődő videóháttér, külső eszközzel; #1015) ebben a kiadásban,
ez a második lépcső **a jövő évi kiadásban**. A döntéseket (VII.) lépésenként egyeztetjük.
**Kapcsolódik:** `docs/credits-plan.md` C11–C12 (#1015), [`AGENTS.md`](../../AGENTS.md) §1–2 és a
CFA4-szakasz, [`docs/licensing.md`](../licensing.md) §3 és §5,
[`CFA4-SECURITY-001`](CFA4-SECURITY-001.md) (#981), [`CFA4-LIVE-STUDIO-001`](CFA4-LIVE-STUDIO-001.md)

---

## I. Cél

A CFA4 maga rajzolja a vonalrajzos animációt: tollal húzott, papírral kitöltött figurák járnak,
futnak, bicikliznek, a jelenet nappalból éjszakába vált. **Chrome és böngésző nélkül**, a saját
renderelőben, élesen bármekkora felbontáson, a stáblistában, a címekben és az élő grafikában is.
A felhasználó nem programoz: kész figurákból és jelenetekből választ, és paraméterekkel állítja őket.

## II. Kiindulás: mit ad az eredeti projekt

[`jdhruv14/line-art-animation`](https://github.com/jdhruv14/line-art-animation), commit
`c43d698913cf0ad44f984c7ae14c5348010594d3` (2026-11-10), **MIT**, Copyright (c) 2026 JDhruv14.
TypeScript + SVG, egy Claude-skill és egy kis könyvtár:

| Rész | Tartalom | Méret |
|---|---|---|
| `lib/gait.ts` | járás-, csoszogás- és futásmegoldó: a talajon lévő láb pontosan a haladási sebességgel csúszik hátra (nem csúszik a talajon), a lendülő láb minimális rántású pályán jár, a sarok leválik, a csípő magassága abból adódik, amit a lábak elérnek (nincs szinuszos bólogatás) | 128 sor |
| `lib/motion.ts` | rugó, inga, szél, simítás, minimális rántás, magból számolt véletlen, ütemező | 80 sor |
| `lib/daynight.ts` | napszak szerinti paletta (ég, papír, tinta; éjjel világos tinta indigó papíron, meleg ablakfény) | 67 sor |
| `lib/ink.ts` | rajzelemek: nyitott tollvonás, papírral kitöltött körvonal, tömör tinta, a végtag két nyitott vonallal (a stílus legjellemzőbb jegye), ízület, lógó elem, cipő, elhelyezés; a „tinta” remegése SVG-szűrővel; csíkos minta | 189 sor |
| `examples/`, `references/`, `SKILL.md` | két mintajelenet (egy sétáló alak, egy japán utca), a stílus szabályai, ellenőrzési és videós munkamenet | — |
| `scripts/` | képkocka-mentés fej nélküli Chrome-mal, hurokhossz-mérés ffmpeggel | — |

## III. Mit veszünk át és mit nem (D1)

- **Átírjuk Rustra:** a `lib/` négy modulját (a mozgásmatematika, a paletta és a rajzelemek
  geometriája). Ez a projekt értéke, és függőség nélküli.
- **Nem vesszük át:** a mintajelenetek figuráit és épületeit (saját figurákat rajzolunk, a
  `SKILL.md` „új azt jelenti, hogy új” szabálya szerint is), a Chrome-os és Node-os szkripteket, a
  Claude-skillt.
- **Licenc:** MIT, a `docs/licensing.md` §3 szerint átvehető; a szerzői megjegyzés megmarad
  (`crates/lineart/LICENSE-line-art-animation`), a fájlfejlécek megnevezik a forrást, és a
  §5-ös nyilvántartásba új sor kerül (forrás, commit, fájlok).

## IV. Felépítés (javaslat)

1. **`cfa4-lineart` crate** (L2, csak a `cfa4-geom`-tól függ): tiszta számítás, idő → rajz.
   - `motion`: rugó, inga, szél, simítás; **determinisztikus**: rögzített 1/240 s lépésben
     integrál a klip elejétől, ellenőrzőpontokkal, így bármely időpont azonos eredményt ad
     előre- és hátrajátszásnál, ugrásnál és exportnál (a never-crash és a pontos idő szabálya).
   - `gait`: a három járásmód, változatlan matematikával; az idő `cfa4_time::Tick`.
   - `daynight`: a paletta; a napszak az idő függvénye vagy kulcskockázható paraméter.
   - `ink`: a rajzelemek, kimenetük vonások listája (útvonal, vastagság, kitöltés: tinta,
     papír vagy szín), DOM nélkül.
   - `figures`, `scenes`: a saját figurák és jelenetek (V. L3–L4).
2. **Rajzolás a renderelőben** (`cfa4-render`, `cfa4-text` raszterező):
   - új **nyitott vonás kerek véggel és illesztéssel** (ma a grafikus réteg csak zárt
     útvonalat tölt ki, a körvonal maszkból készül);
   - a tinta remegése **geometriában** (magból számolt zaj a vonás pontjain, időben stabil), nem
     raszterszűrővel, így felbontásfüggetlen;
   - csíkos minta kitöltésként.
3. **Grafikus réteg:** új rétegtípus a grafikus klipekben (`LayerContent::LineArt`), saját
   paraméterekkel (figura vagy jelenet, méret, irány, sebesség, ruhaszínek, napszak, mag). Így a
   címek, az alsó harmadok és a stáblista is ugyanazzal a renderelővel használja, az élő grafika
   (C10) pedig változtatás nélkül.
4. **Stáblista:** az oldalstílus hátterének új fajtája (`"kind": "lineart"`, a jelenet és a
   paraméterei), és egy cellában figura (`{{LineArt walker …}}`), ami a nevekkel együtt halad.

## V. Fázisok (javaslat)

| Fázis | Tartalom | Kész, ha |
|---|---|---|
| **L0** – Referencia | az eredeti `lib/` számszerű kimenete (csípő, boka, térd, lábfej-dőlés, paletta) rögzített fázisokban, egyszer generálva JSON-ba (Node-dal, csak fejlesztéskor; nem kerül a termékbe) | a referenciaadatok és a mérőkészlet megvannak |
| **L1** – `cfa4-lineart` mag | `motion`, `gait`, `daynight`, `ink` Rustban, `unsafe` és pánik nélkül | az L0 adataival 1e-6 pontossággal egyezik; a talajon lévő láb sebessége = haladási sebesség |
| **L2** – Rajzolás | nyitott vonás, kerek vég és illesztés, papírkitöltés, tintaremegés, csíkos minta a renderelőben | aranymintás képek (`cfa4-golden`) 1080p-n és 4K-n |
| **L3** – Figurák | saját figurák: sétáló felnőtt (több alkat és ruha), futó gyerek, csoszogó idős, kerékpáros, kutya, macska, madarak; tárgyak (táska, esernyő, kalap) | mindegyik bemutatója jóváhagyva (a tulajdonos nézi meg) |
| **L4** – Jelenetek | talajvonal, égbolt napszakkal, házak, fák, lámpák, éjjel világító ablakok; hurokbarát elrendezés (a figurák körbeérnek) | a hurok varratmentes: az 1. és az n+1. képkocka azonos |
| **L5** – Motor és felület | `graphics.lineart.*` parancsok, réteg a grafikus klipekben, stáblista-háttér és -cella, Qt6 panel előnézettel, minden nyelven | Qt-tesztek, gépi vezérlés, fordítások |
| **L6** – Teljesítmény | valós idejű lejátszás és élő adás a referenciagépen | 4K 60 fps egy jelenettel, mérve |
| **L7** – Dokumentáció | `docs/credits.md`, grafikai dokumentáció, nyilvántartás, szerzői megjegyzés | — |

## VI. Ütemezés (javaslat)

A jövő évi kiadás, a tulajdonos döntése szerint (2026-11-11). Javaslat: L0–L2 az év elején
(nem érinti a felületet), L3–L4 utána a tulajdonos jóváhagyásaival figuránként, L5–L7 a végén.
A munkát külön munkamenet végzi, ebből a jóváhagyott tervből.

## VII. Döntések (lépésenként egyeztetve)

| # | Kérdés | Javaslat |
|---|---|---|
| D1 | Mit veszünk át | **Eldöntve (2026-11-12): csak a `lib/` matematikáját és rajzelemeit** (`gait`, `motion`, `daynight`, `ink`) írjuk át Rustra; a figurák és a jelenetek saját tervezésűek, a mintapéldákból (`examples/`) semmi nem kerül át |
| D2 | Hogyan állítja a felhasználó | **Eldöntve (2026-11-12): kész figurák és jelenetek paraméterekkel**, programozás nélkül (méret, irány, sebesség, ruhaszínek, napszak, mag) a jövő évi kiadásban; **egy későbbi kiadásban jelenetszerkesztő** a Qt6-ban, ahol a figurák és az épületek egérrel elhelyezhetők |
| D3 | Hol rajzolódik | **Eldöntve (2026-11-12): a saját renderelőben, CPU-n és GPU-n is**, a grafikus rétegek útján, mint a címek és az alakzatok; az előnézet, az export és az élő grafika ugyanazt adja, külső függőség nélkül. A renderelő két új képességet kap (nyitott vonás kerek véggel és illesztéssel, tintaremegés a geometriában), mindkettő CPU- és GPU-úton is, egyező eredménnyel |
| D4 | Időkezelés | **Eldöntve (2026-11-12): determinisztikus.** A rugók, ingák és a szél rögzített 1/240 s lépésben integrálnak a klip elejétől, mentett köztes állapotokkal (ellenőrzőpontokkal); bármely időpont ugyanazt adja előre- és hátrajátszásnál, ugrásnál, exportnál és élő adásban. Ugráskor a legközelebbi ellenőrzőponttól számol tovább (néhány ms) |
| D5 | Hol használható | **Eldöntve (2026-11-12): mindenhol, ahol grafika van:** a stáblistában háttérként és cellában (a nevekkel haladó figura), a címekben és az alsó harmadokban (Grafika panel), és az élő grafikában (C10) változtatás nélkül, mert a réteg közös |
| D6 | Mozdulatok a figurákon | **Eldöntve (2026-11-12): kulcskockázható mozdulatok az idővonalon** (köszön, kalapot emel, megáll, leül, továbbmegy), adott időpontra téve; a figura a járásból simán áll meg és indul tovább (a járásmegoldó `go` keverése). A stáblistában is időzíthető (pl. a figura az utolsó oldalnál megáll és köszön). Kattintásra reagálás nincs |
| D7 | Ütemezés | a jövő évi kiadás, külön munkamenettel (VI.) |

## VIII. Kockázatok

- **A stílus megtartása:** a figurák jósága szemmel dől el; ezért minden figura bemutatója a
  tulajdonos jóváhagyásával kerül be (L3), és az eredeti projekt ellenőrzési szabályait
  (`references/verify.md`) átvesszük tesztekbe, ahol mérhetők (talajon álló láb, nincs
  szinuszos bólogatás, varratmentes hurok).
- **Teljesítmény:** sok figura és vonás 4K-n; a vonások gyorsítótárazva, a remegés olcsó
  zajjal, mérve a referenciagépen (L6).
- **Olvashatóság a stáblistában:** mozgó háttér előtt a szöveg kontrasztja; a napszak-paletta
  ismert, ezért a C2-es kontrasztellenőrzés a vonalrajz-háttérre is mérhetővé válik.
- **Az eredeti projekt változik:** egyetlen commitnál tartunk; a nyilvántartás a rögzített
  commitot nevezi meg, a későbbi változásokat csak tudatosan vesszük át.

## IX. Források

- Az eredeti projekt: <https://github.com/jdhruv14/line-art-animation>, commit `c43d6989`
  (`lib/gait.ts`, `lib/motion.ts`, `lib/daynight.ts`, `lib/ink.ts`, `LICENSE`).
- Licencszabályok: [`docs/licensing.md`](../licensing.md) §3 (MIT átvehető) és §5 (nyilvántartás).
