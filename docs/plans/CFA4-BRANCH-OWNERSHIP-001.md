# CFA4-BRANCH-OWNERSHIP-001 – egy ág, egy munkamenet: az ág-tulajdon technikai védelme

**Állapot:** 1.0, javaslat. A tulajdonos jóváhagyására vár. Jóváhagyás előtt semmi nem épül meg belőle.

**Összefoglaló:** a „más PR-jához, ágához ne nyúlj” szabály (`docs/coordination.md` §1 és §2.4) ma csak
írott szabály, és semmi nem állítja meg az ügynököt, amelyik mégis megszegi. A terv után minden PR-ágnak
egy tulajdonos munkamenete van, csak az pushol rá, a kivételeket (gazda, Dependabot, tulajdonosi átadás) pedig
a PR maga sorolja fel. Egy kötelező CI-ellenőrzés (`branch-ownership`) pirosra vált, ha idegen munkamenet
commitja kerül az ágra, így a gazda azt nem olvaszthatja be. Egy helyi Claude Code-hook a push előtt meg is
állítja az idegen munkamenetet. A védelem a véletlen ellen véd, nem a szándékos visszaélés ellen (6. fejezet).

## 1. Kiindulási tények (2026-11-14, `main` b768709)

- A szabály megvan: a fejlesztő munkamenet „más PR-jához és ágához nem nyúl” (`docs/coordination.md` §1),
  „push, rebase, lezárás, gyűjtő-PR” sem, „akkor sem, ha ütközik a tieddel” (§2.4). A köteg-ágra csak a gazda
  pushol, az eredeti PR-ok ágához nem nyúl (§3a).
- Technikai ellenőrzés nincs. A `main` kötelező ellenőrzése ma a `cargo-check` és a `full-validation-gate`;
  egyik sem nézi, ki commitolt.
- Esetek, amikor a szabály sérült vagy két író volt egy ágon:
  - 2026-10-27: egy munkamenet más munkamenetek PR-jait csomagolta össze, a megbeszélt kis körön túl.
  - 2026-10-30: a ChatGPT/Codex a tulajdonos nevében olvasztott be PR-okat (#334, #335, #330).
  - 2026-11-14: a VVC-00 jelentés jóváhagyását a tulajdonos egy új munkamenetben adta meg. Az új munkamenet
    (a tulajdonos kifejezett jóváhagyásával) a régi munkamenet két ágára pusholt (#1215, #1216), a régi
    munkamenet erről nem tudott, és közben ő is pusholt a #1216 ágára. Ütközés nem lett, de két író dolgozott
    ugyanazon az ágon. Az átadásnak nincs leírt módja.
- Amiből az ellenőrzés építkezhet: minden Claude-commit üzenete `Claude-Session: https://claude.ai/code/session_…`
  sorral végződik (a környezet előírja). A 2026-11-14-i 12 legutóbbi nyitott PR-ban minden nem-merge commiton
  ott van. Hiányzik viszont:
  - a merge commitokon (`Merge main into …`, `Merge branch '…' into …`, a GitHub „Update branch” commitja);
  - a gazda köteg-commitjain (`Köteg koteg/…: #…`);
  - a Dependabot commitjain;
  - a tulajdonos kézi commitjain.
- Minden ügynök és a tulajdonos is ugyanazzal a GitHub-fiókkal (`ubuntuokos`) dolgozik, a Claude-commitok
  szerzője „Claude”. A GitHub oldalán tehát nem választható szét, ki tett fel egy commitot, címkét vagy
  megjegyzést. Az egyetlen munkamenet-azonosító a `Claude-Session:` sor.

## 2. Cél és hatókör

**Cél:** minden nyitott PR-ágról el lehessen dönteni, melyik munkamenet az övé. Idegen munkamenet ne tudjon
véletlenül rápusholni, ha mégis megtörténik, az látsszon, és blokkolja a beolvasztást.

**Hatókör:** a `main`-re nyitott minden PR (fejlesztői PR, köteg-PR, Dependabot-PR), a Claude Code-munkamenetek
helyi pusholása, a `docs/coordination.md` és a `/gazda` skill. **Nem** hatókör: a GitHub-fiókok szétválasztása
(6. fejezet, K1), a `main` ág védelmének egyéb szabályai, a CodeRabbit (az nem commitol).

## 3. Fogalmak

- **Az ág tulajdonosa:** az a munkamenet, amelynek `Claude-Session:` sora a PR első, nem-merge commitján áll.
  A PR leírása ezt egy sorban ki is mondja: `Munkamenet: https://claude.ai/code/session_…` (a PR-leírások
  végén ma is ott a munkamenet-hivatkozás, ez lesz kötelező és géppel olvasható).
- **Átadás:** a tulajdonos döntése, hogy egy PR-t egy másik munkamenet visz tovább. A PR leírásába egy
  `Átadva: session_<régi> → session_<új>, tulajdonosi jóváhagyással, ÉÉÉÉ-HH-NN` sor kerül, és egy
  ugyanilyen PR-megjegyzés. Az átadás után a régi munkamenet nem pushol az ágra.
- **Megengedett commit egy PR-ágon:**
  1. nem-merge commit a tulajdonos vagy egy átvevő munkamenet `Claude-Session:` sorával;
  2. merge commit, amelynek második szülője a `main`-en van (a `main` behúzása, kézzel vagy a GitHub
     „Update branch” gombjával), bárkitől;
  3. a 4. fejezet T2 pontjában felsorolt kivételek (Dependabot, köteg-ág, tulajdonosi kézi commit).
  Minden más idegen commit.

## 4. Elemek

**T1 – Szabály és átadási eljárás (`docs/coordination.md`).** A §2.4 kiegészül azzal, hogy a PR-ágra csak a
tulajdonos munkamenete pushol. Új pont (§2.10) írja le az átadást:
1. A tulajdonos az új munkamenetben kimondja, hogy az átveszi a PR-t (például „jóváhagyom, vidd tovább”).
2. Az új munkamenet első lépése, még a push előtt: az `Átadva:` sor a PR leírásába és a PR-megjegyzés.
3. A régi munkamenet minden push előtt megnézi a PR leírását (T3 ezt el is végzi), és ha átadták, nem pushol,
   hanem jelzi a tulajdonosnak.

Ha a tulajdonos nem mondja ki az átadást, az új munkamenet nem nyúl a régi ágához. Új ágat nyit a `main`-ből,
vagy megkérdezi a tulajdonost.

**T2 – `branch-ownership` CI-ellenőrzés.** Új munkafolyamat (`.github/workflows/cfa4-branch-ownership.yml`)
`pull_request` eseményre (`opened`, `synchronize`, `edited`, `reopened`), `contents: read` és
`pull-requests: read` jogokkal. Egy Python-szkript (`scripts/ci/branch_ownership.py`, tesztekkel a
`python/tests/` alatt) a GitHub API-ból lekéri a PR commitjait és leírását, és a 3. fejezet szabályai szerint
minősít. Kivételek:
- `dependabot/*` ág: csak `dependabot[bot]` szerzőjű commit és a `main` behúzása megengedett;
- `koteg/*` ág: csak a köteg-szkript (`scripts/dev/gazda-koteg.sh`) merge commitjai, amelyek második szülője
  a köteg-PR leírásában felsorolt PR-ok egyikének feje, valamint a `main` behúzása (D3);
- tulajdonosi kézi commit (`Claude-Session:` sor nélkül, szerző `ubuntuokos`): D2 szerint.

Ha idegen commitot talál, a check piros, és a naplóban commitonként kiírja, melyik commit melyik munkamenetből
jött, és mi lett volna a megengedett. A check a `main` kötelező ellenőrzései közé kerül (D4).

**T3 – Helyi push-őr (Claude Code-hook).** A `.claude/settings.json` egy `PreToolUse` hookja a `git push`
parancsok előtt lefut (`scripts/dev/branch-owner-guard.sh`), és megállítja a pusht, ha:
- a cél-ágnak van nyitott PR-ja, és annak tulajdonosa (vagy utolsó átvevője) nem ez a munkamenet;
- a PR leírásában `Átadva:` sor szerint ezt a munkamenetet már leváltották.

Az ellenőrzés a saját munkamenet azonosítóját a környezet adja (a T3 első lépése kideríti, honnan olvasható ki
megbízhatóan: környezeti változó vagy a munkamenet által írt commit sora; ha nem olvasható ki, a hook csak
figyelmeztet). A hook a hiba okát és a teendőt is kiírja („kérd a tulajdonost átadásra, vagy nyiss új ágat”).

**T4 – A gazda.** A `/gazda` skill beolvasztás előtt a `branch-ownership` checket is nézi (kötelező checkként
amúgy is), idegen commitnál a PR a (c) kategóriába kerül, és a tulajdonosnak jelzi. A köteg-szkript a köteg-PR
leírásába beírja a felhasznált PR-fejeket, hogy a T2 ellenőrizni tudja.

**T5 – A meglévő nyitott PR-ok.** Bevezetéskor (2026-11-14-én 77 nyitott PR van) egyszeri, csak olvasó jelentés
készül: melyik PR-on van idegen commit, és melyiken hiányzik a `Munkamenet:` sor. Visszamenőleg nem javítunk
commitot. Ahol a jelentés idegen commitot talál, a tulajdonos dönt (átadásnak tekinti vagy nem).

**T6 – Egy helyen a szabály.** A szabály csak a `docs/coordination.md`-ben áll (a 2026-11-01-i döntés: minden
szabály egy helyen). Az `AGENTS.md` és a `CLAUDE.md` legfeljebb egy hivatkozást kap rá.

## 5. Döntési pontok (a javasolt alapértékkel)

| # | Kérdés | Javaslat |
|---|---|---|
| D1 | Az átadást mi bizonyítja? | Az `Átadva:` sor a PR leírásában, amelyet az átvevő munkamenet ír be a tulajdonos szóbeli jóváhagyása után. Erősebb bizonyíték (például a tulajdonos saját megjegyzése) nem lehetséges, mert mindenki ugyanazzal a fiókkal dolgozik. |
| D2 | `Claude-Session:` nélküli, nem-merge commit (a tulajdonos kézi commitja vagy egy sor nélküli ügynök) | Piros, kivéve, ha a PR-on `tulajdonosi-commit` címke van. A címkét a tulajdonos teszi fel; ügynök nem. |
| D3 | Köteg-ágak ellenőrzése | Igen, a 4. fejezet szerint (csak a felsorolt PR-fejek és a `main` beolvasztása). |
| D4 | Mikortól blokkol a check? | Egy hétig csak jelez (nem kötelező check), utána a `main` kötelező ellenőrzése. |
| D5 | A helyi hook (T3) blokkol vagy csak figyelmeztet? | Blokkol, ha a munkamenet azonosítója megbízhatóan kiolvasható; különben figyelmeztet. |
| D6 | A §2.4 tiltásának kivétele a gazda „Update branch”-e és ütközésfeloldása | A `main` behúzása mindenkinek megengedett (a check nem jelez rá). Ütközésfeloldó commitot a gazda csak a köteg-ágon tesz, más PR-ágon nem, ahogy ma is. |

## 6. Korlátok és kockázatok

- **K1 – Nem biztonsági határ.** Mivel mindenki ugyanazzal a GitHub-fiókkal és ugyanazzal a commit-szerzővel
  dolgozik, a `Claude-Session:` sor hamisítható, és az `Átadva:` sort bárki beírhatja. A terv a véletlen és a
  szabálykövetés hiánya ellen véd, nem rosszindulatú ügynök ellen. Valódi határhoz külön GitHub-fiók vagy
  GitHub App kellene munkamenetenként, ez nem része a tervnek.
- **K2 – Hamis riasztás.** A merge commitok és a régi, sor nélküli commitok téves pirosat adhatnak. Ezért van a
  D4 egyhetes jelzős időszak, és a T5 jelentés a bevezetés előtt.
- **K3 – Plusz CI-futás.** Az ellenőrzés csak API-hívás, másodpercek alatt lefut, nem fordít.
- **K4 – A hook a felhőben.** A munkamenetek a felhőben futnak; a hook csak ott véd, ahol a repo
  `.claude/settings.json`-ja betöltődik. Ezt a T3 első lépése ellenőrzi.

## 7. Lépések

A sorrendről a tulajdonos dönt; a lista mindent tartalmaz.

| Lépés | Tartalom | Átadandó |
|---|---|---|
| BO-0 | Ez a terv jóváhagyásra | ez a PR |
| BO-1 | T5 jelentés a nyitott PR-okról (csak olvasás) | jelentés a `docs/plans` alá |
| BO-2 | T2 szkript és tesztjei, munkafolyamat jelző módban | PR (CI, CodeRabbit: kell) |
| BO-3 | T1 szabály és átadási eljárás, T6 hivatkozások | PR (szabályfájl, CodeRabbit: kell) |
| BO-4 | T3 helyi hook | PR (`.claude/`, CodeRabbit: kell) |
| BO-5 | T4 gazda-skill és köteg-szkript módosítása | PR (`.claude/`, CodeRabbit: kell) |
| BO-6 | D4 után: a check kötelezővé tétele a `main` védelmében | tulajdonosi beállítás a GitHubon |

## 8. Elfogadási feltételek

- A T2 szkript tesztjei lefedik: egyetlen munkamenet (zöld); idegen munkamenet commitja (piros); a `main`
  behúzása merge committal más munkamenettől (zöld); más ág beolvasztása merge committal (piros); érvényes
  `Átadva:` sor után az átvevő commitja (zöld), és a leváltott munkamenet későbbi commitja (piros);
  Dependabot-ág (zöld); köteg-ág felsorolt és nem felsorolt PR-fejjel (zöld / piros); sor nélküli kézi commit
  címkével és címke nélkül (zöld / piros).
- A jelzős héten a check minden nyitott PR-on lefut, és a hamis riasztások száma nulla, vagy mindegyiknek
  ismert az oka, és javítva van.
- A T3 hook egy próbaágon megállítja egy második munkamenet pushát, és átadás után engedi.
