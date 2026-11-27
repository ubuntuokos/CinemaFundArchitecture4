# CFA4 – beolvasztási gazda és PR-szabályok

**Tulajdonosi döntés, 2026-10-27; módosítva 2026-11-01 (a gazda csak a tulajdonos hívására
dolgozik).** Ez a dokumentum minden munkamenetre (Claude, ChatGPT/Codex, más MI-ügynök vagy
ember) kötelező. A cél: ne keletkezzenek egymásra épülő, egymással ütköző, soha be nem
olvasztott PR-ok és ágak.

## 1. Kinek mi a dolga

| Szerep | Ki | Mit tehet |
|---|---|---|
| **Tulajdonos** | ubuntuokos | Dönt a sorrendről, jóváhagyja a beolvasztásokat, ágat töröl a GitHubon. |
| **Beolvasztási gazda** | egyetlen gazda munkamenet, a tulajdonos `/gazda`-val hívja | **Csak a tulajdonos utasítására cselekszik.** Egyedül ő olvaszt be a `main`-be, kizárólag tulajdonosi jóváhagyással. Sorrendet javasol, ütközést old fel, lezár, ha valamit egy másik PR kivált. Fejlesztői PR ágára csak átvétel után pushol (§2.13). |
| **Dependabot-előkészítő** | egyetlen, erre kijelölt Claude-munkamenet (tulajdonosi döntés, 2026-11-09) | A Dependabot-PR-okat **előkészíti**: friss ágra hozza (`update-branch`), lefuttatja rajtuk a teljes CI-t (`cfa4-full-validation`), a csoportosítást a `.github/dependabot.yml`-ben saját PR-ban javítja, és „Gazda:” issue-ban átadja a beolvasztásra kész listát. **Nem olvaszt be** és nem zár le PR-t; a Dependabot-PR-okhoz kódjavítást nem tölt fel (ha egy frissítés kódmódosítást igényel, azt az érintett crate projektjének adja át). |
| **Fejlesztő munkamenet** | minden más | Egy témán dolgozik, egy PR-ban. **Nem olvaszt be**, más PR-jához és ágához nem nyúl. A Dependabot-PR-okhoz sem. A saját PR-ja ágára is csak addig pushol, amíg az övé (§2.4, §2.12, §2.13). |

Egy gazda munkamenet van. A tulajdonos a `/gazda` paranccsal hívja
([`.claude/skills/gazda/SKILL.md`](../.claude/skills/gazda/SKILL.md)), és minden sikeres
feladat után `/clear`-rel üríti. A gazda magától nem ellenőriz, nem ütemez és nem dolgozik.
Gazdát csak a tulajdonos jelölhet ki vagy válthat le. Fejlesztő munkamenet nem lép gazda szerepbe.

## 2. Szabályok minden fejlesztő munkamenetnek

1. **Friss `main`-ből** indíts ágat. Ne építs másik, még nyitott PR ágára. Ha a munkád egy nyitott
   PR-tól függ, várd meg, amíg az bekerül, vagy jelezd a PR-od leírásában.
2. **Egy PR = egy téma.** Ne csomagolj össze független változtatásokat.
3. **Kész PR-t ne olvassz be magad.** Jelöld „Ready for review”-nak, és írd a leírásába, hogy kész.
   A beolvasztás a gazda dolga, a tulajdonos hívására.
4. **Más PR-jához, ágához ne nyúlj** (push, rebase, lezárás, gyűjtő-PR), akkor sem, ha ütközik
   a tieddel. Az ütközést a gazda oldja fel.
   **Egy ág, egy tulajdonos** (tulajdonosi döntés, 2026-11-14/15; terv:
   [`plans/CFA4-BRANCH-OWNERSHIP-001.md`](plans/CFA4-BRANCH-OWNERSHIP-001.md)). A PR ágára csak a
   tulajdonosa pushol: az a munkamenet, amely a PR leírásának `Munkamenet:` sorában áll. Ezt a sort
   minden PR leírásába írd be: `Munkamenet: https://claude.ai/code/session_…`. Claude Projects-szál
   új beszélgetésben új munkamenettel folytatódhat: a folytató munkamenet az első push előtt a saját
   azonosítóját hozzáfűzi a `Munkamenet:` sorhoz (ez nem átadás); a helyi push-őr addig megállítja a pushát. A tulajdonost csak átadás (§2.12)
   vagy gazda-átvétel (§2.13) váltja. Minden commitod végén ott a `Claude-Session:` sor. A
   `main` behúzása a saját ágadba megengedett, és rakott PR-nál a lánc előző ágáé is. A
   `cfa4-branch-ownership` check ezt minden PR-on ellenőrzi; a tulajdonos saját, munkamenet-sor
   nélküli commitját a `tulajdonosi-commit` címke engedi.
5. **Közös fájlok** (kodek-regiszter, `crates/codecs`, `crates/export/src/lib.rs`, effektlisták,
   `i18n/*.tsv`, `docs/*-parity.md`, `Cargo.toml`, `Cargo.lock`): csak a saját sorodat add hozzá,
   ne rendezd át és ne formázd újra a fájlt.
6. **Új téma indítása előtt** nézd meg a nyitott PR-okat. Ha 30-nál több van, új munkát csak
   a tulajdonos kérésére kezdj. (Tulajdonosi döntés, 2026-11-07: a korábbi 20-as határ 30-ra nőtt.)
7. Munka végén **hagyd tisztán**: ne maradjon PR nélküli ág, ideiglenes (`tmp/`, `ci-debug/`) ág.
8. **Projekthatár** (tulajdonosi döntés, 2026-10-31). A saját projekteden (a tulajdonos által
   jóváhagyott terv, ADR vagy issue, amellyel a munkamenet indult) belül logikus sorrendben magad
   választhatod a következő feladatot. **Másik projektből nem vehetsz át feladatot.** Ha átfedést
   vagy ütközést találsz egy másik projekttel, jelezd a PR-od leírásában vagy megjegyzésében, és
   az érintett részen ne dolgozz tovább. A tulajdonos és a gazda együtt dönt; **a döntés
   kötelező**, a munkamenetek nem nyitják újra.
9. **CodeRabbit-ellenőrzés** (tulajdonosi döntés, 2026-11-12; terv:
   [`plans/CFA4-CODERABBIT-001.md`](plans/CFA4-CODERABBIT-001.md); beállítás:
   [`.coderabbit.yaml`](../.coderabbit.yaml)). **Csak kockázatos PR-on kell** (tulajdonosi
   döntés D5, 2026-11-13: az ingyenes keret nagyjából óránként egy review). Kockázatos a PR,
   ha a `git diff --name-only --no-renames origin/main...HEAD` bármelyik fájlja ide tartozik
   (átnevezésnél így a régi és az új útvonal is látszik):
   - **biztonság:** a CFA4-SECURITY-001 projekt PR-jai, `crates/platform/`, `deny.toml`,
     `.semgrep/`;
   - **kodek és konténer:** `crates/codecs/`, `crates/ffmpeg/`, `crates/gstreamer/`, valamint
     minden olyan crate, amelyet a [`architecture.md`](architecture.md) §1 kodek- vagy
     konténer-crate-ként sorol fel;
   - **CI** (szűkítve, tulajdonosi döntés D6, 2026-11-20): a PR akkor kockázatos, ha
     - a beolvasztás kötelező vagy biztonsági kapuit adó workflowt módosítja: `pr-check.yml`
       (`cargo-check`), `ci-linux.yml` (`full-validation-gate`), `cfa4-gate.yml`,
       `cfa4-security.yml`, `codeql.yml`;
     - kiadást vagy közzétételt végző workflowt módosít: `release` eseményre induló vagy bármilyen
       `write` jogosultságú workflow (2026-11-20-án csak a `cfa4-ffmpeg-prebuilt.yml`, amely
       `contents: write` jogosultsággal GitHub-kiadást tölt fel);
     - bármelyik workflowban jogosultságot (`permissions:`), titkot (`secrets.`),
       `pull_request_target` vagy `workflow_run` indítót, illetve külső actiont vagy annak rögzített
       verzióját (`uses:`) vesz fel vagy módosít;
     - a `.github/dependabot.yml`-t vagy a `.github/actionlint.yaml`-t módosítja.

     A többi `.github/` változás (például egy kézzel indítható vagy csak tesztet futtató workflow
     lépései) nem kockázatos. Ellenőrzés: a fenti fájllista, és a workflow-diffben
     (`git diff origin/main...HEAD -- .github/workflows/`) keresés a `permissions:`, `secrets.`,
     `pull_request_target`, `workflow_run` és `uses:` szövegekre; a keresés a kontextus miatt nem
     mindig mutatja a `permissions:` szót egy többsoros blokk módosított kulcsa mellett, ezért minden
     módosított workflow-fájl teljes `permissions:` blokkját (a felső szintűt és a jobokét is)
     vesd össze a közös ős (`git merge-base origin/main HEAD`) változatával, így csak a PR saját
     változása számít, a `main` közbeni változása nem;
   - **szabályfájlok:** `AGENTS.md`, `CLAUDE.md`, `docs/coordination.md`, `.coderabbit.yaml`,
     `.claude/`.

   Kockázatos PR-on, amikor „Ready for review”-nak jelölöd, írd a PR-ba megjegyzésként:
   `@coderabbitai review`, és a PR leírásába: „CodeRabbit: kell”. Más PR-on ne kérj
   ellenőrzést (a keretet a kockázatos PR-ok elől venné el), a leírásba pedig ezt írd:
   „CodeRabbit: nem kell”. Ha a CodeRabbit „Review limit reached” választ ad, várd meg
   a jelzett időt, és kérd újra; addig a PR nem adható át. Minden megállapítását vagy javítsd ki
   a saját ágadon, vagy válaszolj rá a szálban, és indokold, miért nem javítod. A review
   törzsében, szál nélkül közölt megállapításokra (például a diffen kívüliekre) egy
   PR-megjegyzésben válaszolj, pontonként (például mert
   az `AGENTS.md` mást ír elő: ütközésnél mindig a CFA4 szabályai nyernek). Javítás után kérd
   újra (`@coderabbitai review`). A PR-t csak akkor add át a gazdának, ha a legutóbbi kérésedre
   a PR mostani fején a `CodeRabbit` check-run `success` lett (§3.4 b), és nem maradt megválaszolatlan
   CodeRabbit-szál (§3.4). A CodeRabbit javító funkcióit (Autofix, „Fix comments”,
   CI-javítás, ütközésfeloldás) nem indítod: a PR-ba csak te commitolsz (gazda-átvétel után a
   gazda, §2.13). Dependabot-PR-on és köteg-PR-on nem kérsz CodeRabbit-ellenőrzést. A gazdához
   csak hibátlan PR mehet: ha a CodeRabbit hibát talál, javítsd ki, és kérd újra.

10. **Fejlesztés csak jóváhagyott terv alapján** (tulajdonosi döntés, 2026-11-14).
    - Fejlesztés (kód, teszt, CI, csomagolás) csak akkor indulhat, ha a terve egyeztetve van
      a tulajdonossal, és a tulajdonos jóváhagyta.
    - A terv menet közben bármikor módosulhat. Módosítás után a fejlesztés **megáll**, és csak
      a módosított terv tulajdonosi jóváhagyása után folytatható.
    - Ha fejlesztés közben a fejlesztő munkamenet kér tervmódosítást, ahhoz is tulajdonosi
      jóváhagyás kell. A kérés után a módosítással érintett munka megáll, és csak a jóváhagyás
      után folytatható.
    - Tervmódosítás az is, ha a munka eltérne a terv lépéseitől, sorrendjétől vagy átvételi
      feltételeitől. Ilyenkor a munkamenet nem dönt maga: leírja a javasolt módosítást, és
      jóváhagyást kér.
    - A §2.8 szerinti választás a saját projekten belül csak a jóváhagyott terv keretein belül
      érvényes.

11. **Terv csak a repóba kerülhet** (tulajdonosi döntés, 2026-11-15).
    - Minden CFA4-terv (terv, specifikáció, jelentés, felmérés, összevetés) Markdown-fájlként
      a `docs/plans/` mappába kerül, saját ágon, PR-rel. Nem kerülhet online dokumentumba
      (Claude-dokumentum, artifact, Google Docs, Notion vagy más külső felület), és nem maradhat
      csak a munkamenet beszélgetésében.
    - Ez akkor is így van, ha a tulajdonos nem mondja külön: ha a munkamenet a saját munkájához
      vagy bármely CFA4-projekthez ír tervet, annak a helye a repó.
    - Ha egy terv mégis repón kívül készült, a munkamenet változatlan tartalommal átviszi
      a `docs/plans/` mappába, és ettől kezdve a repóbeli fájl az irányadó.
    - A tervek egymásra és a repó fájljaira repóbeli relatív hivatkozással mutatnak, nem külső
      dokumentumra.

12. **Átadás másik munkamenetnek** (tulajdonosi döntés, 2026-11-14). Ha a tulajdonos egy nyitott
    PR-t egy másik munkamenetre bíz (például egy új beszélgetésben: „jóváhagyom, vidd tovább”):
    - az új munkamenet az első push előtt a PR leírásába és egy PR-megjegyzésbe beírja:
      `Átadva: session_<régi> → session_<új>, tulajdonosi jóváhagyással, ÉÉÉÉ-HH-NN`;
    - a `Munkamenet:` sort nem kell átírni: az átadás utáni írót az `Átadva:` sor adja meg, ezt olvassa a
      `branch-ownership` check és a helyi push-őr is;
    - a régi munkamenet minden push előtt megnézi a PR leírását; ha átadták, nem pushol, hanem
      jelzi a tulajdonosnak;
    - ha a tulajdonos nem mondja ki az átadást, az új munkamenet nem nyúl a régi ágához: új ágat
      nyit a `main`-ből, vagy megkérdezi a tulajdonost.

13. **Gazda-átvétel** (tulajdonosi döntés, 2026-11-14).
    - Amikor a PR-t „Kész”-nek jelölöd, átadod a gazdának. A gazda az első írása (ághoz igazítás,
      ütközésfeloldás) előtt a PR leírásába és egy PR-megjegyzésbe beírja:
      `Gazda átvette: session_<gazda>, ÉÉÉÉ-HH-NN`. Ehhez nem kell külön tulajdonosi jóváhagyás.
    - Átvétel után a PR ágára a gazda pushol, te nem. Ha javítást látsz szükségesnek,
      PR-megjegyzésben jelzed.
    - Hibát a gazda nem javít: visszaadja a PR-t (`Gazda visszaadta: session_<fejlesztő>,
      ÉÉÉÉ-HH-NN`), és onnantól megint te pusholsz, javítasz, kockázatos PR-on újra kéred a
      CodeRabbitet, és a hibátlan PR-t újra átadod.
    - **Kivétel:** a tulajdonos kérésére a gazda maga javíthat, ha a fejlesztő túlterhelt, és a
      PR-ra máshol szükség van. Ilyenkor a javítás előtt beírja: `Gazda javítja tulajdonosi
      kérésre: <ok>, ÉÉÉÉ-HH-NN`; kockázatos PR-on a javítás után ő kéri a CodeRabbitet, és ő
      kezeli a megállapításait. A PR így is csak hibátlanul olvasztható be.
14. **Kihagyott fizikai teszt feljegyzése** (tulajdonosi döntés, 2026-11-22). Ha egy fizikai tesztet
    (referenciagépes mérés, valódi hardveres, kamerás, kijelzős vagy hálózati próba) a tulajdonos
    döntése alapján kihagysz, ugyanabban a munkamenetben vegyél fel róla egy sort a
    [`docs/development/deferred-physical-tests.md`](development/deferred-physical-tests.md)
    listába: mit kell mérni, a parancsot, a helyettesítő (pl. felhős) eredményt és az okot. A
    pótlólagos futtatás után az állapotot írd át, a sort ne töröld.

## 3. A gazda munkamódja (minden `/gazda` hívásnál)

0. **Minden hívás önálló.** A gazda a korábbi hívások eredményeire nem épít; minden tényt
   a `main` és a GitHub aktuális állapotából olvas újra.
1. **Állapotfelmérés** minden hívás elején (csak olvasás): nyitott PR-ok, az alapáguk, utolsó
   commit ideje, beolvaszthatóság a mostani `main`-re, teljes CI állapota, a PR-leírásokban
   jelzett kérések.
2. **Aktív PR-hoz nem nyúl:** ha az utolsó commit 3 óránál frissebb, vagy a leírás szerint még
   folyik a munka, kihagyja.
3. **Sorrendet javasol** a tulajdonosnak (`VISION.md` prioritása szerint: Qt6 először), és csak
   a tulajdonos által ugyanabban a hívásban jóváhagyott PR-okat olvasztja be, **egyenként**,
   vagy a tulajdonos kérésére **egy köteg-PR-ban** (§3a). Más gyűjtő-PR-t nem nyit.
4. Beolvasztás előtt: a PR ága (köteg esetén a köteg-ág) friss a `main`-nel (`update-branch`),
   a teljes CI (`cfa4-full-validation`) zöld **azon a fejen**.
   Ezt a kötelező `full-validation-gate` ellenőrzés technikailag is kikényszeríti (tulajdonosi
   döntés, 2026-10-31): csak akkor zöld, ha minden blokkoló teljes job azon a fejen ténylegesen
   lefutott és zöld. A Qt6 GUI-tesztek a GUI elkészültéig futnak, de nem blokkolnak.
   **CodeRabbit** (tulajdonosi döntés, 2026-11-12; D5, 2026-11-13): a gazda a fájllistából (és
   a workflow-diffből) maga is megnézi, kockázatos-e a PR (§2.9), a leírásban álló jelzéstől függetlenül. Kockázatos
   PR-t csak akkor olvaszt be, ha mindhárom feltétel teljesül; ha a keret elfogyott, a PR vár,
   CodeRabbit nélkül nem engedi tovább:
   (a) a fejlesztő kért ellenőrzést, vagyis a PR megjegyzései között
   (`GET /repos/{owner}/{repo}/issues/{n}/comments`, minden oldal) van `@coderabbitai review`;
   (b) a kért review be is fejeződött: a PR mostani fején a `CodeRabbit` nevű check-run
   (`GET /repos/{owner}/{repo}/commits/{sha}/check-runs?check_name=CodeRabbit`, minden oldal;
   több találatnál a legkésőbb indult számít) `success` eredménnyel („Finished”)
   zárult, és a `completed_at`-je később van, mint az utolsó `@coderabbitai review` kérés.
   (Megállapítás nélküli review-nál a CodeRabbit nem készít review-t, csak ezt a check-runt;
   kimerült keretnél a check-run `neutral`, „Review rate limited”, ez nem elég.) Ha a fej a
   review után változott, újra kell kérni: átvétel előtt a fejlesztőnek, átvétel után a gazdának
   (§2.13). Nem kell új review, ha a változás csak a `main` ütközés nélküli behúzása, és a PR saját
   változtatása (a diff a `main`-hez képest) nem változott; ilyenkor az előző fejen kapott sikeres
   review elég. Ütközésfeloldás után kell (tulajdonosi döntés D8, 2026-11-14);
   (c) nincs megválaszolatlan CodeRabbit-megállapítás: sem szál
   (`GET /repos/{owner}/{repo}/pulls/{n}/ccr/review_threads` a Claude Code munkamenetekben;
   máshol a GitHub GraphQL `pullRequest.reviewThreads { isResolved }`), sem a PR bármelyik
   (korábbi vagy legutóbbi) CodeRabbit-review törzsében szál nélkül közölt megállapítás, amelyre a fejlesztő nem válaszolt
   PR-megjegyzésben. Egy megállapítás akkor lezárt, ha meg van oldva, vagy a fejlesztő
   indokolva válaszolt rá.
   Ha valamelyik hiányzik, a PR a „(c) frissíteni kell” kategóriába kerül, és a gazda a PR-ban
   jelzi a fejlesztőnek; átvett PR-t visszaad (§2.13). Ez nem technikai CI-kapu. Dependabot-PR-ra és magára a köteg-PR-ra
   nem vonatkozik; a kötegbe kerülő kockázatos eredeti PR-okra igen (§3a.1).
4a. **Ág-tulajdon:** a gazda a `cfa4-branch-ownership` checket is nézi. Idegen commitnál a PR a
   (c) kategóriába kerül, és a tulajdonosnak jelzi (§2.4). Fejlesztői PR ágára csak átvétel után
   ír (§2.13).
5. Ha egy PR-t egy másik kivált, a gazda lezárja, és a megjegyzésben megnevezi, melyik váltotta ki.
6. Minden hívás eredményét egy sorban beírja a [`docs/coordination-log.md`](coordination-log.md)-be.
7. Ágat nem töröl (a munkamenetek nem kapnak rá engedélyt): a törölhető ágak listáját átadja
   a tulajdonosnak.
8. **Átfedés vagy ütközés két projekt között** (§2.8): a tulajdonossal együtt dönt, a döntést
   egy sorban a [`docs/coordination-log.md`](coordination-log.md)-be írja, és az érintett
   munkameneteknek a PR-jukban jelzi. A döntés ezután kötelező.
9. A feladat végén rövid jelentést ad, és **megáll**. Ütemezett vagy ismétlődő ellenőrzést nem hoz létre.

## 3a. Köteg-PR (tulajdonosi döntés, 2026-11-08)

A `main` szabálysora szerint egy PR csak a `main`-nel naprakészen olvasztható be, így minden
beolvasztás elavultatja a többi PR-t, és mindegyikre újra lefut a kb. 2 órás teljes validáció.
A köteg-PR-ral több jóváhagyott PR-ra **egy** teljes validáció jut. A GitHub merge queue a
személyes fiók alatti tárolóban nem érhető el, ezért a kötegelést a gazda végzi.

1. **Indítás:** `/gazda köteg #12 #15 #18` – a tulajdonos által ebben a hívásban jóváhagyott
   PR-ok, alapból 3–8 darab (a tulajdonos kisebbet is kérhet). Aktív PR (§3.2) nem kerül bele.
   Olyan kockázatos eredeti PR sem, amely nem teljesíti a §3.4 CodeRabbit-feltételeit (a Dependabot-PR-ok
   kivételek); a gazda ezt a köteg összerakása előtt PR-onként ellenőrzi.
2. **Összerakás:** `scripts/dev/gazda-koteg.sh #12 #15 #18`. A mostani `main`-ből nyit egy
   `koteg/ÉÉÉÉHHNN-N` ágat, a PR-okat a megadott sorrendben, egyenként, merge committal rárakja,
   az ütközőket kihagyja és kiírja, majd köteg-PR-t nyit a `main`-re a `cfa4-full-validation`
   címkével. A köteg-ágra csak a gazda pushol; az eredeti PR-ok ágához nem nyúl.
3. **Zöld `full-validation-gate` esetén** a gazda a köteg-PR-t **merge committal** olvasztja be
   (squash és rebase tilos). Így az eredeti PR-ok fejei a `main`-be kerülnek, és a GitHub azokat
   magától beolvasztottnak jelöli. Utána ellenőrzi, hogy minden bekerült PR „Merged”; ha egy PR-ra
   közben új commit jött, az nyitva marad, ezt jelenti.
4. **Ha a `main` közben elmozdult:** a köteg-ágat frissíti (`update-branch`), és a címke
   levételével és újra feltételével újraindítja a teljes validációt.
5. **Piros kapu esetén:** a hibás jobból és tesztből megkeresi az okozó PR-t, kiveszi, és új
   köteget rak össze nélküle. Ha nem egyértelmű, a köteget kettébontja, és a két felét külön
   köteg-PR-ban validálja. A kivett PR szerzőjének a PR-jában jelzi az okot.
6. **Saját beolvasztási kivétellel dolgozó munkamenet** (pl. az upstream-sync): amíg nyitott
   köteg-PR van, nem olvaszt be a `main`-be (az a köteg validációját érvénytelenítené); a PR-ját
   a következő kötegbe kéri a leírásában, vagy a köteg beolvasztása után olvaszt be.

## 4. Ágak

A GitHubon be van kapcsolva az „Automatically delete head branches”. 2026-10-27-én mégis
54 olyan ág volt, ami már teljesen benne van a `main`-ben (42-nek a PR-ja ugyanazzal a fejjel
lett beolvasztva). Ennek oka még nincs kiderítve; a gazda a tulajdonos kérésére megnézi,
a következő beolvasztás után törlődik-e az ág.

## 5. A gazda hívása

- `/gazda` – állapotfelmérés és beolvasztási javaslat, utána megáll.
- `/gazda <utasítás>` – csak az adott utasítást hajtja végre (pl. `/gazda mergeld a #123-at`).
- `/gazda köteg #12 #15 #18` – a felsorolt PR-ok egy köteg-PR-ban, egy teljes validációval (§3a).

A szerep teljes leírása: [`.claude/skills/gazda/SKILL.md`](../.claude/skills/gazda/SKILL.md).
A tulajdonos minden sikeres feladat után `/clear`-rel üríti a gazda munkamenetet.
Időzített gazda-ellenőrzés nincs.
