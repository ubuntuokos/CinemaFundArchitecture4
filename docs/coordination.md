# CFA4 – beolvasztási gazda és PR-szabályok

**Tulajdonosi döntés, 2026-10-27; módosítva 2026-11-01 (a gazda csak a tulajdonos hívására
dolgozik).** Ez a dokumentum minden munkamenetre (Claude, ChatGPT/Codex, más MI-ügynök vagy
ember) kötelező. A cél: ne keletkezzenek egymásra épülő, egymással ütköző, soha be nem
olvasztott PR-ok és ágak.

## 1. Kinek mi a dolga

| Szerep | Ki | Mit tehet |
|---|---|---|
| **Tulajdonos** | ubuntuokos | Dönt a sorrendről, jóváhagyja a beolvasztásokat, ágat töröl a GitHubon. |
| **Beolvasztási gazda** | egyetlen gazda munkamenet, a tulajdonos `/gazda`-val hívja | **Csak a tulajdonos utasítására cselekszik.** Egyedül ő olvaszt be a `main`-be, kizárólag tulajdonosi jóváhagyással. Sorrendet javasol, ütközést old fel, lezár, ha valamit egy másik PR kivált. |
| **Dependabot-előkészítő** | egyetlen, erre kijelölt Claude-munkamenet (tulajdonosi döntés, 2026-11-09) | A Dependabot-PR-okat **előkészíti**: friss ágra hozza (`update-branch`), lefuttatja rajtuk a teljes CI-t (`cfa4-full-validation`), a csoportosítást a `.github/dependabot.yml`-ben saját PR-ban javítja, és „Gazda:” issue-ban átadja a beolvasztásra kész listát. **Nem olvaszt be** és nem zár le PR-t; a Dependabot-PR-okhoz kódjavítást nem tölt fel (ha egy frissítés kódmódosítást igényel, azt az érintett crate projektjének adja át). |
| **Fejlesztő munkamenet** | minden más | Egy témán dolgozik, egy PR-ban. **Nem olvaszt be**, más PR-jához és ágához nem nyúl. A Dependabot-PR-okhoz sem. |

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
   - **CI:** `.github/`;
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
   megérkezett a CodeRabbit review a PR mostani fejére, és nem maradt megválaszolatlan
   CodeRabbit-szál (§3.4). A CodeRabbit javító funkcióit (Autofix, „Fix comments”,
   CI-javítás, ütközésfeloldás) nem indítod: a PR-ba csak te commitolsz. Dependabot-PR-on
   és köteg-PR-on nem kérsz CodeRabbit-ellenőrzést.

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
   **CodeRabbit** (tulajdonosi döntés, 2026-11-12; D5, 2026-11-13): a gazda a fájllistából maga
   is megnézi, kockázatos-e a PR (§2.9), a leírásban álló jelzéstől függetlenül. Kockázatos
   PR-t csak akkor olvaszt be, ha mindhárom feltétel teljesül; ha a keret elfogyott, a PR vár,
   CodeRabbit nélkül nem engedi tovább:
   (a) a fejlesztő kért ellenőrzést, vagyis a PR megjegyzései között
   (`GET /repos/{owner}/{repo}/issues/{n}/comments`, minden oldal) van `@coderabbitai review`;
   (b) a kért review be is fejeződött: van `coderabbitai[bot]` review
   (`GET /repos/{owner}/{repo}/pulls/{n}/reviews`), amelynek `commit_id`-je a PR mostani feje, és
   amely az utolsó `@coderabbitai review` kérésnél később érkezett. Ha a fej a review után
   változott, a fejlesztőnek újra kell kérnie;
   (c) nincs megválaszolatlan CodeRabbit-megállapítás: sem szál
   (`GET /repos/{owner}/{repo}/pulls/{n}/ccr/review_threads` a Claude Code munkamenetekben;
   máshol a GitHub GraphQL `pullRequest.reviewThreads { isResolved }`), sem a legutóbbi
   review törzsében szál nélkül közölt megállapítás, amelyre a fejlesztő nem válaszolt
   PR-megjegyzésben. Egy megállapítás akkor lezárt, ha meg van oldva, vagy a fejlesztő
   indokolva válaszolt rá.
   Ha valamelyik hiányzik, a PR a „(c) frissíteni kell” kategóriába kerül, és a gazda a PR-ban
   jelzi a fejlesztőnek. Ez nem technikai CI-kapu. Dependabot-PR-ra és magára a köteg-PR-ra
   nem vonatkozik; a kötegbe kerülő kockázatos eredeti PR-okra igen (§3a.1).
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
