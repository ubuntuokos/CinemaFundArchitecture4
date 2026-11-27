# CFA4-BRANCH-OWNERSHIP-001 – BO-1 jelentés: a nyitott PR-ok ág-tulajdona

**Dátum:** 2026-11-15, 07:20–07:30 (UTC), `main` `2c6e67f`. **Terv:** [CFA4-BRANCH-OWNERSHIP-001](CFA4-BRANCH-OWNERSHIP-001.md)
1.4, T5 / BO-1 (a terv a #1284-ben vár beolvasztásra). **Módszer:** csak olvasás a GitHub API-ból; semmit nem módosít.
Szkript és nyers adat: [`CFA4-BRANCH-OWNERSHIP-001-data/`](CFA4-BRANCH-OWNERSHIP-001-data/).

## 1. Eredmény

**Valódi szabálysértést (idegen munkamenet commitját egy másik munkamenet PR-ján) nem találtam.** Az 59 nyitott PR-ból
(58 fejlesztői, 1 köteg) 42-n a terv T2 szabálya szerint minden rendben van. A maradék 17-en a T2 a mostani szövegével
pirosat adna, de ezek közül egy sem szabályszegés: a terv három szokásos munkamódot nem kezel (rakott PR-ok, más
PR-ra épülő ág, Claude Projects-szál), és négy PR a tulajdonos saját webes szerkesztése. **Ezért a BO-2 előtt a terv
4. fejezetét a 3. fejezet E1–E4 pontja szerint pontosítani kell**, különben a check az első napon 17 PR-t blokkolna.

| Kategória | PR-ok | Db | Mit mutat | Szabálysértés? |
|---|---|---|---|---|
| Tiszta | a többi | 42 | egy munkamenet, a leírás `Munkamenet:` hivatkozása egyezik | nem |
| Rakott PR: a bázis- vagy az előző ág beolvasztása | #1193, #1231, #1252, #1256, #1257, #1274, #1281, #1286, #1288, #1304; köteg: #1336 | 11 | merge commit, amelynek második szülője nem a `main`-en, hanem a lánc előző ágán van (például `Merge branch 'cs/midi-monitor' into cs/midi-learn`) | nem; a T2 csak a `main` behúzását engedi (E2) |
| Más nyitott PR-ra épülő ág | #1298 | 1 | az AT9/AT3-ág a #1270 (`cb/atrac9`) fejéből indult; 8 commitja a kodek-munkameneté, a PR saját 4 commitja a leírásban álló munkameneté | nem; a T2 az első commitból azonosít (E1, E3) |
| Claude Projects-szál, munkamenet-váltással | #1124 | 1 | az első commit (terv) egy munkamenetből, a további 21 egy másikból; a leírás a szálra és a második munkamenetre mutat | nem valószínű (ugyanaz a szál); a T2 idegennek látná (E4) |
| Tulajdonosi kézi commit (webes szerkesztés, `ubuntuokos-patch-*`) | #1091, #1092, #1093, #1094 | 4 | `Claude-Session:` sor nélküli commit a tulajdonostól, leírásban nincs munkamenet | nem; a D2 szerint `tulajdonosi-commit` címke kell, mielőtt a check blokkol |
| Claude-commit `Claude-Session:` sor nélkül | `1c69def` („Merge main into cb/tak: rustfmt”, egyszülős commit) a #1256, #1257, #1336 láncában | 1 commit | a kodek-munkamenet egy commitja sor nélkül | nem; formai hiba, a T2 pirosat adna (E1 után a leírás alapján dönthető) |

A számok a 17 jelzett PR-ra: 11 + 1 + 1 + 4 = 17; a sor nélküli `1c69def` a rakott láncok soraiban már benne van.

## 2. Részletek

- **#1298:** a bázisa `main`, de az ág a #1270 fejéből (`b761c70`) indult, ezért a diffje tartalmazza a #1263, #1264,
  #1268, #1269, #1270 még be nem olvasztott commitjait. A PR saját munkája az utolsó 4 commit. Ez függőség egy másik
  munkamenet nyitott PR-jától: a #1298 csak a #1270 lánc után olvasztható be.
- **#1124:** a `claude/project-thread-pzk4nu` ág egy Claude Projects-szál ága. A szál új beszélgetésben új munkamenet-
  azonosítóval folytatódik, ezért az első commit (`6afd299`) más munkamenetből jön, mint a további 21. A PR leírása a
  szálra és a második munkamenetre hivatkozik. A GitHubról nem ellenőrizhető, hogy a két munkamenet ugyanabból a
  szálból jön-e; a leírás és a commitok tartalma (ugyanaz a terv, F0 → F4 folytatás) erre utal.
- **#1336 (köteg):** a köteg első commitjai a kodek-munkamenetéi (a beolvasztandó PR-ok), a leírás a gazda munkamenetére
  mutat. Ez a §3a szerinti rendes köteg; a T2 köteg-kivétele kezeli, de az első commitból azonosító szabály itt is téves.

## 3. Javasolt pontosítások a tervhez (a BO-2 előtt, tulajdonosi döntésre)

| # | Pontosítás | Miért |
|---|---|---|
| E1 | Az ág tulajdonosa a PR leírásának `Munkamenet:` sora (Projects-szálnál a szál hivatkozása is), nem az első commit. Ha a leírásban nincs, akkor az első commit. | #1298, #1336: az első commit nem a PR szerzőjéé. |
| E2 | Megengedett merge commit: a második szülő a `main`-en, a PR báziságán vagy egy másik nyitott PR fején van (rakott PR-ok). | 11 PR rendes láncbeolvasztása. |
| E3 | Egy másik nyitott PR-ból örökölt commitot (amely annak a fejéből elérhető) nem ez a PR ítél meg, hanem a sajátja; a check a függőséget kiírja („csak a #N után olvasztható be”). | #1298. |
| E4 | Claude Projects-szál: a szál minden munkamenete tulajdonos; a folytató munkamenet első push előtt a leírás `Munkamenet:` sorához hozzáfűzi a saját azonosítóját (ez nem átadás, nem kell hozzá `Átadva:` sor). | #1124: a szál új munkamenetben folytatódik. |
| E5 | A T1 új pontjai a `coordination.md`-ben §2.11 (átadás) és §2.12 (gazda-átvétel) lesznek, nem §2.10 és §2.11. | A §2.10-et 2026-11-14-én a „fejlesztés csak jóváhagyott terv alapján” szabály kapta. |

A D2 (tulajdonosi kézi commit csak `tulajdonosi-commit` címkével) változatlan; a #1091–#1094-re a címkét a tulajdonos
teszi fel, mielőtt a check blokkolni kezd (D4, BO-6).

## 4. Következő lépés

A `docs/coordination.md` §2.10 szerint a terv módosítása megállítja a munkát a tulajdonos jóváhagyásáig. A BO-2
(a CI-ellenőrzés) ezért csak az E1–E5 elfogadása után indul; elfogadás után a terv 1.5-ös változata rögzíti őket.
