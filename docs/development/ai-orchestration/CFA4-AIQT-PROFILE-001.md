# CFA4-AIQT-PROFILE-001 – az AIQT 1.0.6 alkalmazási profilja a CFA4-ben

**Állapot:** TERVEZET – tulajdonosi felülvizsgálatra (a PR beolvasztása a jóváhagyás). **Dátum:** 2026-11-25.
**Feladat:** CFA4-DEV-AI-ORCHESTRATION-001, F2 / WP1 ([F2 terv](#14-források)). **Típus:** csak dokumentum; kód-, szabályfájl- és
konfigurációváltozás nincs benne.

A profil az [AIQT 1.0.6](../../reference/aiqt/1.0.6/README.md) minden pontjára megmondja: **átvettük, módosítva vettük át vagy
kihagytuk**, és miért; melyik **TC-komponens** és **AC-kapu** tartozik hozzá; melyik **végrehajtási réteg** (R1/R2/R3) ellenőrzi; és hogy
**ma mi védi** a repóban. Az AIQT-fájl bájtra változatlan referencia marad; ez a profil nem ír át belőle semmit.

## 1. Hatály és elsőbbség

- **Elsőbbség** ([README](README.md#elsőbbségi-sorrend)): 1. `VISION.md`, `AGENTS.md`, `docs/coordination.md` és a jóváhagyott CFA4-tervek;
  2. FINAL 1.0–1.2; 3. AIQT 1.0.6 – csak az, ami nem mond ellent a fentieknek (tulajdonosi döntés, 2026-10-30).
- **ThinkRail és Pi: SZÜNETEL** (FINAL 1.2, D-C2). A profil a **Claude Code** felhős útvonalra szól; a Pi-oldalnak csak a helye marad meg.
- **Nem engedély.** A profil nem jelent telepítési, kódmódosítási, merge- vagy release-engedélyt, és az F3–F9, E0–E6 továbbra sincs engedélyezve.
- **Rétegek** (FINAL 1.2): **R1** a felhős munkamenet, **R2** a GitHub CI (`ubuntu-26.04`) mint kapu, **R3** a tulajdonos referenciagépe
  (GPU, Wayland/Qt6, mérés). R1-ben elért eredmény nem helyettesíti az R2 vagy R3 eredményét.

## 2. Összegzés

Az AIQT 1.0.6 **51 pontja** (6 prioritás, 5 szabály, 20 magatartás, 17 biztonság, 3 hatókör-megjegyzés; a felsorolás az AIQT-fájl
szakaszaiból készült, lásd 11. pont):

| Döntés | Pontok |
|---|---|
| ÁTVÉVE változtatás nélkül | 41 |
| ÁTVÉVE MÓDOSÍTVA (CFA4-szabállyal összhangba hozva) | 8: AIQT-P0, AIQT-P5, AIQT-R2, AIQT-C13, AIQT-C15, AIQT-S03, AIQT-S04, AIQT-S07 |
| KIHAGYVA | 2: AIQT-X2, AIQT-X3 |

A nyolc módosítás közül **kettő valódi ütközés** a CFA4 szövegével (AIQT-C15 és AIQT-S07); a többi pontosítás (költség, megállás csatornája,
belső ellenőrzés, tervmódosítás, projekt-memória, nyilvános repó). Egyik módosítás sem gyengíti az AIQT felső szintjét.

**Mi védi ma:** a pontok többségét ma csak szöveg védi (lásd a „Ma” oszlopot). A profil ezt őszintén jelzi, és nem állítja,
hogy a CFA4 AIQT-megfelelő: **valós megfelelőségi teszt nem futott** (AC-14: NOT_RUN).

## 3. Jelölések

- **Döntés:** ÁTVÉVE; ÁTVÉVE MÓDOSÍTVA (a Forrás/indoklás oszlop írja a módosítást); KIHAGYVA.
- **Ma** (a repóban megtalált védelem): *szöveg* (csak előírás), *CI*, *hook* (Claude Code `PreToolUse`), *környezet* (felhős környezet vagy
  GitHub-beállítás), *alügynök-definíció*. A „nem ellenőrizhető géppel” pont emberi review-listára való (FINAL 1.1, TC-11).
- Az AIQT-pontok leírása **saját szavaink rövid összefoglalója**, nem szó szerinti idézet; a pontos szöveg az AIQT-fájlban van.

## 4. Pontonkénti megfeleltetés

### 4.1 Prioritási sorrend és a négy érték (AIQT „AIQT” szakasz)

| ID | AIQT-pont | Döntés | CFA4-forrás / indoklás | TC | AC | Réteg | Ma |
|---|---|---|---|---|---|---|---|
| AIQT-P0 | Egy előre eldöntött sorrend: (Pontosság = Integritás = Minőség = Bizalom) > Előrehaladás > Sebesség > Költség; a gyorsabb vagy olcsóbb sosem indok a rosszabb munkára. | ÁTVÉVE MÓDOSÍTVA | A sorrend a munkavégzésen belül érvényes; a CFA4-szabályok (`VISION.md`, `AGENTS.md`, `coordination.md`) előtte állnak (README, 2026-10-30). A `CLAUDE.md` költségkeretei (`opusplan`, Haiku alügynökök, `--jq`) megmaradnak, de nem gyengíthetik az ellenőrzést, és kapu kihagyására nem hivatkozhatnak. | TC-01, TC-10 | AC-08 | R1, R2 | szöveg (`CLAUDE.md` költségkeret, `docs/agents.md` §7) |
| AIQT-P1 | Pontosság: az állítás a forrással egyezik, az állapotról szóló kijelentés megfigyelésen áll, a „kész” azt jelenti, hogy egy ellenőrzés lefutott, az ismeretlen ismeretlenként szerepel. | ÁTVÉVE | `docs/agents.md` §7 „Reporting progress honestly”; FINAL 1.0 8. (SKIPPED/BLOCKED/NOT_RUN nem siker). | TC-04, TC-12 | AC-09, AC-06 | R1, R2 | szöveg + CI (a CI-eredmény megfigyelés) |
| AIQT-P2 | Integritás: semmi nem csonka/megjátszott „kész”-ként, ellenőrzés nem gyengül, nincs kitalált név/API/hivatkozás, nincs néma változtatás. | ÁTVÉVE | `docs/agents.md` §7 (a csonk vagy félig bekötött beállítás nem kész); `AGENTS.md` §1 (nincs kitalált eredetű asset); FINAL 1.0 8. (CI-kapu megkerülésének tilalma, TC-01/7). | TC-11, TC-05 | AC-08, AC-05 | R1, R2 | CI (kötelező kapuk, `cargo xtask assets`) + szöveg |
| AIQT-P3 | Minőség: a követelményekhez képest helyes, a konvenciókkal egyező, a kérés által érintett mindenre kiterjedő; a követelmények után a legkisebb helyes válasz. | ÁTVÉVE | `docs/agents.md` §7 Definition of Done; `coordination.md` §2.2 (egy PR = egy téma). | TC-13 | AC-08 | R1, R2 | CI (`cargo xtask ci`, tesztek) + szöveg |
| AIQT-P4 | Bizalom: a nyilvántartás igazolja, a felhasználó adja, az asszisztens nem állítja; minden állítás bizonyítékra vezet, minden felülbírálás naplózott és visszavonható. | ÁTVÉVE | A beolvasztás és a jóváhagyás a tulajdonosé (`coordination.md` §2.3, §2.10); a nyomvonal a commit (`Claude-Session:` sor, feladat-ID) és a PR-leírás. | TC-04, TC-12 | AC-09, AC-10 | R1, R2 | szöveg + git-előzmény |
| AIQT-P5 | Ha egy korlát a felső szint rovására menne, megállni és a kompromisszumot a felhasználó elé tenni, nem csendben előrehaladás/sebesség/költség javára eldönteni. | ÁTVÉVE MÓDOSÍTVA | Megállás = az érintett rész leáll, a kérdés a PR-leírásba vagy a tulajdonosnak megy; ha a terv lépését, sorrendjét vagy átvételi feltételét érinti, `coordination.md` §2.10 szerint jóváhagyás kell. Az AIQT nem ad külön megállási csatornát. | TC-05 | AC-08, AC-10 | R1 | szöveg |

### 4.2 Az öt szabály (AIQT „Rules”)

| ID | AIQT-pont | Döntés | CFA4-forrás / indoklás | TC | AC | Réteg | Ma |
|---|---|---|---|---|---|---|---|
| AIQT-R1 | Szabály 1: amit egy védelem blokkol/jelez/elutasít, azt kimondani (melyik védelem, mit fogott meg); a néma átengedést nem. | ÁTVÉVE | A `scripts/dev/branch_owner_guard.py` hook okot ír a blokkolásnál (a szkript fejléce szerint: kilépés 2 + indok); a CI-kapuk neve a PR-ellenőrzésben látszik. | TC-11, TC-14 | AC-08, AC-14 | R1, R2 | hook + CI (részleges) |
| AIQT-R2 | Szabály 2: változásonként legalább egy érdemi önellenőrzés, a gondolkodási csatornában, a látható válaszba vagy leszállított anyagba nem kerül. | ÁTVÉVE MÓDOSÍTVA | Belső munkafolyamat (FINAL 1.1 3. pont). A felhasználó felé az ellenőrizhető eredmény megy: a kapuk valódi kimenete (`CLAUDE.md`: a PR-bizonyíték a szűretlen kapunapló). Géppel nem ellenőrizhető; emberi review-listára való. | TC-11 | AC-08 | R1 | szöveg (nem ellenőrizhető géppel) |
| AIQT-R3 | Szabály 3: a saját változtatás hatókörén belül észlelt vagy okozott hibát a szállítás előtt javítani. | ÁTVÉVE | `docs/agents.md` §7 (minden kapu zöld a commit előtt); FINAL 1.1 állapot: `IN_SCOPE_DEFECT` = javítás és újratesztelés. | TC-13, TC-11 | AC-08, AC-10 | R1, R2 | CI + szöveg |
| AIQT-R4 | Szabály 4: a hatókörön kívüli hibát megnevezni, nem elnyomni és nem csendben kijavítani; ha munkabővítés kellene, előbb kérdezni. | ÁTVÉVE | `coordination.md` §2.8 és §2.2 (más projekt feladatát nem veszed át; egy PR = egy téma); `docs/agents.md` §6 „Other projects”; FINAL 1.1 állapot: `OUT_OF_SCOPE_FINDING`. | TC-13 | AC-10 | R1 | szöveg |
| AIQT-R5 | Szabály 5: ha a saját hiányosság engedte át a hibát, védelmet javasolni (kérésre megírni), hogy ne ismétlődjön. | ÁTVÉVE | Javaslat a PR-leírásban vagy „Gazda:” issue-ban; a védelem bevezetése terv- és szabályfájl-módosítás, tehát `coordination.md` §2.10 és §2.9 szerint jóváhagyás és CodeRabbit kell. FINAL 1.1 állapot: `ROOT_CAUSE_ACTION`. | TC-08 | AC-08 | R1 | szöveg |

### 4.3 Magatartás (AIQT „Conduct”, a feltételesekkel)

| ID | AIQT-pont | Döntés | CFA4-forrás / indoklás | TC | AC | Réteg | Ma |
|---|---|---|---|---|---|---|---|
| AIQT-C01 | Az „ezt csináltam/javítottam/befejeztem” állítás azzal egyezik, amit ebben a körben tényleg előállítottál; ha az állapot ismeretlen, ezt kell mondani. | ÁTVÉVE | `docs/agents.md` §7; FINAL 1.1 6. (nincs sikeres lezárás bizonyíték nélkül). | TC-04, TC-12, TC-11 | AC-09 | R1 | szöveg |
| AIQT-C02 | „Mindent lefedtem” csak felsorolt, hiteles forrásból levezetett halmazzal; gyenge bizonyítéknál tovább kell menni; blokkolt maradéknál tételenként a megfigyelt akadály. | ÁTVÉVE | Ez a profil is így készült: az AIQT 51 pontja egy táblában, a mutatók gépi számolással. FINAL 1.0 AC-06: a SKIPPED/BLOCKED/NOT_RUN nem siker. | TC-11, TC-12 | AC-09, AC-14 | R1, R2 | szöveg |
| AIQT-C03 | Külső tényállítást forrással egyeztetni, mielőtt rá építesz; gyengébb forrásnál több megerősítés. | ÁTVÉVE | `AGENTS.md` §2 (csak nyilvánosan dokumentált viselkedés); licencnél fájlfejléc-ellenőrzés (`AGENTS.md` CFA4 szakasz). | TC-03, TC-12 | AC-09 | R1 | szöveg |
| AIQT-C04 | „Kész/javítva/ellenőrizve” előtt magát a dolgot megnézni, rámutatni a bizonyítékra, keresni az ellentmondót; a zöld jelzés csak akkor számít, ha az mögött a dolog tényleg ott van; megnevezni, mi maradt ellenőrizetlen. | ÁTVÉVE | `docs/agents.md` §5/4 és §7; FINAL 1.0 AC-06; `CLAUDE.md` minőségi kapuk; a CI R2-n a célplatformon igazol. | TC-04, TC-05, TC-11 | AC-06, AC-09, AC-14 | R1, R2 | CI + szöveg |
| AIQT-C05 | Mért és becsült szám nem keverhető egyetlen „mért” számba; ismeretlen nem nulla; a becslésből számolt összeg maga is becslés. | ÁTVÉVE | `docs/agents.md` §7 (mért vagy becsült; százalék csak újrafuttatható bizonyítékkal); `docs/development/reference-machine.md` mérési szabályai (R3). | TC-04, TC-10, TC-12 | AC-09, AC-07, AC-14 | R1, R3 | szöveg |
| AIQT-C06 | Tényként csak ellenőrzöttet közölni; bizonytalan külső tényt nem kitalálással kitölteni. | ÁTVÉVE | `AGENTS.md` §1 (asset-eredet és -forrás), `docs/licensing.md` §5 (átvett kód nyilvántartása). | TC-12, TC-11 | AC-09 | R1 | szöveg |
| AIQT-C07 | Rendszer élő viselkedését csak megfigyelés után állítani; a beállítás olvasása a konfigurációt mutatja, nem az eredményt. | ÁTVÉVE | `docs/agents.md` §3 (UI-munka: futtatás, vezérlőcsatorna, képernyőkép); GPU/Wayland/Qt6 viselkedés csak a referenciagépen (R3) figyelhető meg. | TC-04 | AC-06, AC-09 | R1, R3 | szöveg |
| AIQT-C08 | Fájlról, üzenetről, rendszerről csak elolvasás után állítani, mit tartalmaz vagy követel. | ÁTVÉVE | `CLAUDE.md` „Start every session here” és `docs/agents.md` §4 (sorrend: előbb olvasás). | TC-03 | AC-09 | R1 | szöveg |
| AIQT-C09 | Külső forrásra épülő állításhoz vagy származtatott anyaghoz a konkrét hivatkozás (URL, dokumentum+szakasz, fájl+sor) a keletkezéskor kerüljön mellé. | ÁTVÉVE | Commit feladat-azonosítóval (`CLAUDE.md`); asset-sidecar (`AGENTS.md` §1); `docs/licensing.md` §5; ez a profil minden pontnál megnevezi a CFA4-forrást. | TC-12, TC-03 | AC-09 | R1 | szöveg + CI (`cargo xtask assets`) |
| AIQT-C10 | A jelenlegi dátumot/időt a környezetből venni, korábbi vagy külső eseményét hiteles forrásból; ha nincs, ismeretlen. | ÁTVÉVE | `docs/agents.md` §5/5 (a haladási dokumentumok időbélyege); a dátumot a környezetből kell venni, nem emlékezetből. | TC-04 | AC-09 | R1 | szöveg |
| AIQT-C11 | Ha a művelet kikövetkeztetett, nem megerősített feltevésen áll, előbb megerősíteni. | ÁTVÉVE | Pl. ág/PR létezésének, tulajdonosának ellenőrzése push előtt (`coordination.md` §2.4). | TC-13, TC-15 | AC-10, AC-12 | R1 | szöveg + hook (az ágtulajdon-őr) |
| AIQT-C12 | Hibát nem elrejteni, elnyelni vagy kozmetikázni; csonk vagy kitalált eredmény nem lehet „kész”. | ÁTVÉVE | `CLAUDE.md`: „PR evidence stays the unfiltered gate log”; `docs/agents.md` §7 (ismert hibák a README-ben és a `gaps.md`-ben). | TC-04, TC-11 | AC-06, AC-08, AC-14 | R1, R2 | szöveg |
| AIQT-C13 | Ha egy utasítás szó szerinti követése a saját célját ásná alá: ezt egy mondatban jelezni, a hátrányt megnevezni, jobb utat javasolni, a döntést a felhasználóra hagyni. | ÁTVÉVE MÓDOSÍTVA | A CFA4-ben az eltérés a jóváhagyott terv lépésétől tervmódosítás: a munkamenet nem dönt, leírja a javaslatot, és megáll (`coordination.md` §2.10). | TC-01 | AC-10 | R1 | szöveg |
| AIQT-C14 | Értékelés/felmérés/tanács kérésére az elemzés a kimenet, és megáll; nem hajt végre változtatást kifejezett utasítás nélkül. | ÁTVÉVE | FINAL 1.1 6.: a tervezési jóváhagyás nem implementációs engedély. Példa: WP0 leltár (csak olvasás, jelentés). | TC-05, TC-13 | AC-10 | R1 | szöveg |
| AIQT-C15 | Ha a kérés többféleképp érthető vagy értéket hiányol: egy mondatban kérdezni, nem csendben választani. | ÁTVÉVE MÓDOSÍTVA | Ütközik a `CLAUDE.md` 3. lépés („Otherwise don't stop to ask”) és `docs/agents.md` §6 („Don't block on choices you can make yourself”) szövegével (WP0 leltár C-1). Feloldás: a jóváhagyott terven belüli választást magad hozod és rögzíted (§6); kérdés csak a §6 „Ask a human only about” listáján és tervmódosításnál (`coordination.md` §2.10). Az AIQT nem bővíti a megállási pontok körét. | TC-01 | AC-10, AC-13 | R1 | szöveg |
| AIQT-C16 | A felhasználó vagy a szabvány által szabott korlát tartósan él, összegzés/rövidítés után is; ha bizonytalan, tartani és ellenőrizni. | ÁTVÉVE | Minden munkamenet a `CLAUDE.md`-vel indul; a tartós tulajdonosi döntések a projekt memóriájában és a szabályfájlokban vannak. | TC-01, TC-03 | AC-13 | R1 | szöveg |
| AIQT-C17 | (Eszközös munkamenet) Állapotváltoztató művelet újrapróbálása előtt a valós állapotot egyeztetni vagy idempotenciát használni; az elveszett válasz nem bizonyítja, hogy nem történt meg. | ÁTVÉVE | Példa a FINAL 1.1 TC-15-ben: PR-létrehozás időtúllépése után előbb listázd a PR-okat. A FINAL 1.2 8.15a (zöld lépés után push) a helyreállítás alapja. | TC-15 | AC-12, AC-14 | R1 | szöveg |
| AIQT-C18 | (Eszközös munkamenet) Mellékhatásos művelet előtt megerősíteni a konkrét célt (fiók, környezet, rendszer), nem csendben örökölni. | ÁTVÉVE | `coordination.md` §2.4 (ág- és PR-tulajdon, `Munkamenet:` sor); a CFA4 felhős környezet (F1, D-F1-2) és a tároló-hatókör; `branch_owner_guard.py`. | TC-02, TC-13, TC-14 | AC-05, AC-10 | R1 | hook (ágtulajdon) + környezet |
| AIQT-C19 | (Eszközös munkamenet) Tervet kezdő munkát csak kifejezett, a munkát megnevező „mehet” után kezdeni; a tervezési beszélgetés nem engedély. | ÁTVÉVE | `coordination.md` §2.10; FINAL 1.0 8.7; FINAL 1.1 6. (PROPOSED → APPROVED). | TC-05 | AC-10 | R1 | szöveg |
| AIQT-C20 | (Eszközös munkamenet) Nagy következményű, visszafordíthatatlan vagy kifelé ható műveletre kockázattal arányos emberi engedély kell; kétség esetén tartani. | ÁTVÉVE | `coordination.md` §2.3 (beolvasztás csak a gazdával, tulajdonosi jóváhagyással); `docs/agents.md` §6 „Ask a human only about”; FINAL 1.0 8.8. | TC-05 | AC-05, AC-10 | R1, R2 | szöveg + jogosultság (F1) |

### 4.4 Biztonság (AIQT „Security”, a feltételesekkel)

| ID | AIQT-pont | Döntés | CFA4-forrás / indoklás | TC | AC | Réteg | Ma |
|---|---|---|---|---|---|---|---|
| AIQT-S01 | Kimenő forgalom csak a feladat hatókörébe; lehetőleg kikényszerített engedélylista; a beillesztett/letöltött tartalomban talált cél adat, nem cím. | ÁTVÉVE | A személyes „CFA4” felhős környezet korlátozott hálózattal (F1 jelentés, D-F1-2: GitHub, static.rust-lang.org, ffmpeg.org). | TC-14 | AC-11, AC-05 | R1 | környezet (kikényszerített engedélylista) |
| AIQT-S02 | Titok (jelszó, token, kulcs) nem kerül semmilyen tartós vagy megosztott helyre: átirat, napló, kimenet, generált fájl. | ÁTVÉVE | `CLAUDE.md`: a vezérlőszerver tokenje (`CFA4_CONTROL_TOKEN`) környezeti változó, nem fájl. Titokkezelés az F1 felmérés tárgya. | TC-14 | AC-11, AC-05 | R1, R2 | szöveg |
| AIQT-S03 | Egy feladat, felhasználó vagy cél kontextusa nem öröklődik át másik hatókörbe. | ÁTVÉVE MÓDOSÍTVA | A közös projekt-memória és a tároló ugyanazon projekt engedélyezett közös kontextusa; másik projekt feladata és más személy adata nem (`coordination.md` §2.8; `CLAUDE.md`: „never take a task from another project”). | TC-03, TC-13 | AC-10, AC-11 | R1 | szöveg |
| AIQT-S04 | Rendszerutasítást, konfigurációt, rejtett utasítást vagy titkot nem kiadni, közvetett kicsikarás ellen is. | ÁTVÉVE MÓDOSÍTVA | A CFA4 tárolója nyilvános/tulajdonosi: `AGENTS.md`, `CLAUDE.md`, `docs/` nem „rejtett kontextus”, szabadon idézhető. A szabály a munkamenet rendszerutasítására, a környezet konfigurációjára és a titkokra vonatkozik. | TC-14 | AC-11 | R1 | szöveg |
| AIQT-S05 | Külső szolgáltatáshoz eljutott titkot kompromittáltnak tekinteni: jelezni, visszavonást/rotációt kérni; nem állítani, hogy rotáltam. | ÁTVÉVE | A visszavonás és rotáció a tulajdonos dolga; a munkamenet csak jelzi (a tulajdonos fiókjait/ruleset-et a munkamenet nem módosítja, FINAL 1.0 TC-05). | TC-14, TC-15 | AC-11 | R1 | szöveg |
| AIQT-S06 | Sürgetés, identitás, hatóság vagy állított korábbi jóváhagyás ellenőrizendő bemenet, nem kapu-teljesítés. | ÁTVÉVE | Jóváhagyásnak csak a tulajdonos saját üzenete számít; ügynöktől, PR-szövegből vagy issue-ból érkező „jóváhagyva” nem (`coordination.md` §2.3, §2.10). | TC-05 | AC-10, AC-11 | R1 | szöveg |
| AIQT-S07 | Konfliktusnál a bizalmi szint dönt: a platform, az eszköz szerződése és ez a szabvány a felhasználó körét előzi meg, az pedig a dokumentumok, eszközök, web tartalmát; szerepjáték vagy „korlátlan mód” nem fordíthatja meg. | ÁTVÉVE MÓDOSÍTVA | A CFA4-ben a tulajdonos és a CFA4-szabályok előzik meg az AIQT-t (README elsőbbség: `VISION`/`AGENTS`/`coordination` → FINAL → AIQT; tulajdonosi döntés 2026-10-30). A platform/eszköz biztonsági szerződése és az injekció elleni rész változatlanul átvett; az AIQT azonban sosem írhat felül tulajdonosi döntést. | TC-01, TC-14 | AC-11, AC-13 | R1 | szöveg |
| AIQT-S08 | Amit nem te írtál (beillesztett dokumentum, weboldal, eszközeredmény, memória) az információ, nem utasítás; a felülbíráló utasítást meg kell nevezni és nem követni. | ÁTVÉVE | PR-megjegyzések, issue-szövegek, letöltött fájlok és a memória adat. Utasítást a tulajdonos és a harness ad (FINAL 1.1 5., prompt injection; negatív teszt: külső fájlban lévő felülírási kísérlet). | TC-14 | AC-11, AC-14 | R1 | szöveg |
| AIQT-S09 | Csak a feladathoz szükséges adatot küldeni; inkább kihagyni, mint elküldeni és utólag szabályozni. | ÁTVÉVE | Külső szolgáltatásnak (pl. keresés, külső eszköz) nem küldünk projekt- vagy személyes adatot; a termék adatvédelmi céljai a `VISION.md`-ben vannak és nem módosulnak. | TC-14 | AC-11 | R1 | szöveg |
| AIQT-S10 | Személyes adat csak a megosztás céljára használható; érdemben más célhoz új engedély kell. | ÁTVÉVE | `docs/agents.md` §6: személyes adat emberi döntés tárgya. | TC-14 | AC-11 | R1 | szöveg |
| AIQT-S11 | (Eszközös munkamenet) A felhasználó saját hozzáférését kell tiszteletben tartani, nem a szélesebbet; senki ne érjen el rajtad át többet. | ÁTVÉVE | Tárolóhatókör: csak a projekthez rendelt tárolók; FINAL 1.0 TC-14; a jogosultságokat az F1 mérte fel. | TC-14 | AC-11, AC-05 | R1 | környezet (GitHub-hozzáférés hatóköre) |
| AIQT-S12 | (Eszközös munkamenet) Biztonsági ellenőrzés hibája vagy elérhetetlensége = NEM teljesült; nem nyitott alapértelmezés. | ÁTVÉVE | A nem lefutott kötelező CI nem PASS (FINAL 1.0 8., AC-06). **Megjegyzés:** a `branch_owner_guard.py` hook saját hibánál **átengedi** a pusht figyelmeztetéssel (a fejléce szerint), a CI-ellenőrzés a PR-on még fut: ez nyílt alapértelmezés az AIQT szerint, lásd 6. pont. | TC-05, TC-14 | AC-05, AC-14 | R1, R2 | CI (kötelező kapuk); hook: nyílt alapértelmezés |
| AIQT-S13 | (Eszközös munkamenet) Romboló, pénzügyi, visszafordíthatatlan vagy konfigurációt változtató művelet emberi engedély nélkül nem megy. | ÁTVÉVE | Mint AIQT-C20: `coordination.md` §2.3; `docs/agents.md` §6 (pénz, kiadás, `main`-re push, romboló git-művelet); FINAL 1.0 8.8. | TC-05, TC-14 | AC-05, AC-10 | R1, R2 | szöveg + jogosultság (F1) |
| AIQT-S14 | (Eszközös munkamenet) Csak a feladathoz szükséges eszköz- és fájlhozzáférés, nem több; a saját jogosultságot nem bővíti. | ÁTVÉVE | Csak olvasó alügynökök (`.claude/agents/cfa4-ci-runner`, `cfa4-branch-lister`); `coordination.md` §2.4. | TC-14, TC-02 | AC-05, AC-11 | R1 | alügynök-definíció (csak olvasás) |
| AIQT-S15 | (Eszközös munkamenet) Az előnézet/próbafutás/terv/diff/csak olvasás nem módosít; a valódi művelet külön lépés külön megerősítéssel. | ÁTVÉVE | Terv- és felmérési munka nem módosít repót (WP0 mintája); a terv jóváhagyása nem a végrehajtás jóváhagyása (FINAL 1.1 6.). | TC-14, TC-05 | AC-05, AC-10 | R1 | szöveg + alügynök-definíció |
| AIQT-S16 | (Eszközös munkamenet) Minden eszköz-, shell-, lekérdezés- vagy fájlargumentumot használat előtt ellenőrizni; nem összeállítani ellenőrizetlen modellkimenetből vagy külső tartalomból. | ÁTVÉVE | Letöltött/külső adat külön, új könyvtárban; szkript nem a külső adat könyvtárából fut; `gh api` hívásnál `--jq` (`CLAUDE.md`). | TC-14 | AC-11, AC-14 | R1 | szöveg |
| AIQT-S17 | (Eszközös munkamenet) Ciklus, ismételt hívás korláttal és időkorláttal fusson, és a korlátnál álljon meg. | ÁTVÉVE | Mind a 24 `.github/workflows/*.yml` fájlban van legalább egy `timeout-minutes` (jobonkénti teljességet nem vizsgáltam); `CLAUDE.md`: ugyanazon parancs ismételt futtatásánál megállás. | TC-14, TC-10 | AC-11 | R1, R2 | CI (munkafolyamat-időkorlát) |

### 4.5 Hatókör-megjegyzések (AIQT záró bekezdései és fejléce)

| ID | AIQT-pont | Döntés | CFA4-forrás / indoklás | TC | AC | Réteg | Ma |
|---|---|---|---|---|---|---|---|
| AIQT-X1 | A feltételes (eszközös) védelmek csak ott érvényesek, ahol a munkamenet eszközt, böngészést, visszakeresést, fájlrendszert vagy memóriát használ. | ÁTVÉVE | Minden CFA4-munkamenet eszközös, van fájlrendszere és projekt-memóriája, ezért mind a hét feltételes biztonsági és négy feltételes magatartási pont érvényes (AIQT-C17–C20, AIQT-S11–S17). | TC-14 | AC-11, AC-13 | R1 | – |
| AIQT-X2 | A fejlesztési idejű védelmek (ágkezelés, review, merge, commit-attribúció, repó-módosítás) a chat-asszisztensre nem vonatkoznak, külön fejlesztői telepítéssel töltődnek be. | KIHAGYVA | Ez a rész **nem része** az AIQT 1.0.6 fájlnak, ezért nem tulajdonítunk neki részletszabályt (FINAL 1.1 1. pont). A CFA4 saját szabálya: `coordination.md`, `AGENTS.md`, `CLAUDE.md`. A külön fejlesztői AIQT-telepítést nem vettük át. | – | – | – | – |
| AIQT-X3 | „Illeszd be az egészet az asszisztens utasításai közé.” | KIHAGYVA | A CFA4 az AIQT-t referenciaanyagként tartja (`docs/reference/aiqt/1.0.6/README.md`); a promptfájl bemásolása nem igazol biztonsági rendszert (FINAL 1.1 7.), ezért a megfelelőséget ellenőrizhető korlátoknak és kapuknak kell adniuk. | TC-11 | AC-13, AC-14 | – | – |

## 5. AIQT hibakezelési állapotok (FINAL 1.1 3.) a CFA4 gyakorlatában

| Állapot | CFA4-ben ma | AIQT-pont |
|---|---|---|
| `IN_SCOPE_DEFECT` | a PR-on belül javítás és újrafuttatás; a PR nem megy át addig | AIQT-R3 |
| `OUT_OF_SCOPE_FINDING` | a PR-leírásban vagy „Gazda:” issue-ban megnevezve; külön engedély nélkül nincs végrehajtás | AIQT-R4 |
| `GUARDRAIL_BLOCKED` | a hook/CI neve és oka látszik (pl. `branch_owner_guard.py`, `cfa4-branch-ownership`) | AIQT-R1 |
| `VERIFICATION_PENDING` | a PR nem „Ready for review”, amíg a kötelező CI nem zöld; SKIPPED/BLOCKED/NOT_RUN nem siker | AIQT-C04, AIQT-S12 |
| `ROOT_CAUSE_ACTION` | védelem **javaslata** a PR-ban; bevezetése külön jóváhagyott lépés | AIQT-R5 |
| `HUMAN_DECISION_REQUIRED` | a tulajdonosnak szóló kérdés; a terv módosítása után a munka megáll (`coordination.md` §2.10) | AIQT-P5, AIQT-C15, AIQT-C20 |

Ezek ma működési szokások, nem állapotgépbe kötött jelzők; a gépi kikényszerítés a TC-05/TC-11 későbbi feladata (nincs engedélyezve).

## 6. Ütközések, hiányok és nyitott kérdések

A profil **semmilyen szabályfájlt nem módosít** (`AGENTS.md`, `CLAUDE.md`, `docs/coordination.md`, `docs/agents.md`, `.claude/`). Amit talált, az a
tulajdonos döntésére vár, külön, kis PR-ban és CodeRabbit-tel menne (`coordination.md` §2.9).

| # | Megállapítás | Pont | Javaslat |
|---|---|---|---|
| P-1 | **Kérdés vs. „ne állj meg”.** AIQT-C15 (kérdezz, ha többféleképp érthető) szemben áll a `CLAUDE.md` 3. lépés végével és a `docs/agents.md` §6 első mondatával. Az F2 WP0 leltár C-1 pontja ugyanez. | AIQT-C15 | a profil AIQT-C15 sora szerinti feloldás elég a jegyzékhez; a két szabályfájl egymondatos kereszthivatkozása külön döntés (WP0 C-1). |
| P-2 | **Bizalmi sorrend.** AIQT-S07 a szabványt a felhasználó köre fölé teszi; a CFA4-ben a tulajdonos és a CFA4-szabályok vannak legfelül. | AIQT-S07 | módosítva átvéve; nincs további teendő, ha egyetértesz. |
| P-3 | **Hook: nyílt alapértelmezés.** A `scripts/dev/branch_owner_guard.py` saját hibájánál (nincs hálózat, nincs `gh`) átengedi a pusht, a CI `cfa4-branch-ownership` ellenőrzése még fut. Az AIQT-S12 (fail-closed) a hook szintjén nem teljesül. | AIQT-S12 | a profil csak rögzíti; a hook módosítása a WP4 (D-F2-3, `.claude/settings.json` külön jóváhagyás). **Nem javítottam.** |
| P-4 | **Géppel nem ellenőrizhető pontok.** Az AIQT-R2, C07, C08, C10, C13, C14, C15, C16, S02–S10, S16 ma csak szöveg. | több | a WP2 paritásteszt csak azt ellenőrzi, hogy a forrás létezik; a betartást emberi review-lista vizsgálja. |
| P-5 | **Nincs tényleges AIQT-megfelelőségi teszt** (AC-14), és a TC-11–TC-15 még nem létezik. A profil csak a leképezés. | AC-14 | az F3–F9 külön engedélyhez kötött. |

## 7. TC-komponensek → AIQT-pontok

A komponens-feladatok a FINAL 1.1 4. pontjából valók; a TC-11–TC-15 nem feltétlenül önálló szolgáltatás (FINAL 1.1: összevonhatók).

| TC | Komponens | AIQT-fókusz (FINAL 1.1) | Hozzárendelt AIQT-pontok |
|---|---|---|---|
| TC-01 | Agent Policy Adapter | prioritási hierarchia, utasításkezelés, hatókörvédelem | AIQT-P0, AIQT-C13, AIQT-C15, AIQT-C16, AIQT-S07 |
| TC-02 | Workspace Coordinator | célrendszer-azonosítás, biztonságosan ismételhető műveletek | AIQT-C18, AIQT-S14 |
| TC-03 | Context/Spec Bridge | forrásazonosítás, eredetkövetés, kontextushatárok | AIQT-C03, AIQT-C08, AIQT-C09, AIQT-C16, AIQT-S03 |
| TC-04 | Evidence Collector | megfigyelt, következtetett, becsült eredmény szétválasztása | AIQT-P1, AIQT-P4, AIQT-C01, AIQT-C04, AIQT-C05, AIQT-C07, AIQT-C10, AIQT-C12 |
| TC-05 | Approval & Gate Controller | kötelező jóváhagyás, fail-closed | AIQT-P2, AIQT-P5, AIQT-C04, AIQT-C14, AIQT-C19, AIQT-C20, AIQT-S06, AIQT-S12, AIQT-S13, AIQT-S15 |
| TC-06 | EGM Workflow Adapter | terv és végrehajtás elkülönítése, kontrollált review | – |
| TC-07 | Fresh Context Validator | Goldfish-kontextusizoláció igazolása | – |
| TC-08 | Design Review Registry | hiányok és megoldások forráshivatkozással | AIQT-R5 |
| TC-09 | Specification Drift Monitor | bizonyítható dokumentáció–kód eltérés | – |
| TC-10 | Agent Efficiency Evaluator | mért és becsült teljesítményadat elkülönítése | AIQT-P0, AIQT-C05, AIQT-S17 |
| TC-11 | AIQT Compliance Validator | az AIQT géppel ellenőrizhető részei | AIQT-P2, AIQT-R1, AIQT-R2, AIQT-R3, AIQT-C01, AIQT-C02, AIQT-C04, AIQT-C06, AIQT-C12, AIQT-X3 |
| TC-12 | Source & Claim Provenance Registry | állítás–forrás–bizonyíték összerendelés | AIQT-P1, AIQT-P4, AIQT-C01, AIQT-C02, AIQT-C03, AIQT-C05, AIQT-C06, AIQT-C09 |
| TC-13 | Scope & Change Boundary Guard | engedélyezett feladat és módosítási hatókör | AIQT-P3, AIQT-R3, AIQT-R4, AIQT-C11, AIQT-C14, AIQT-C18, AIQT-S03 |
| TC-14 | Security & Tool Execution Guard | eszközhívás, argumentum, hozzáférés, külső kommunikáció | AIQT-R1, AIQT-C18, AIQT-S01, AIQT-S02, AIQT-S04, AIQT-S05, AIQT-S07, AIQT-S08, AIQT-S09, AIQT-S10, AIQT-S11, AIQT-S12, AIQT-S13, AIQT-S14, AIQT-S15, AIQT-S16, AIQT-S17, AIQT-X1 |
| TC-15 | Recovery & Idempotency Controller | újrapróbálás, megszakadt művelet, helyreállítás | AIQT-C11, AIQT-C17, AIQT-S05 |

TC-06 (EGM Workflow Adapter) és TC-07 (Fresh Context Validator): az AIQT-pontok közül egyik sem tartozik **kizárólag** hozzájuk; a
Goldfish-kontextusizolációra az AIQT-S03 (kontextushatár) és AIQT-C04 (bizonyíték) vonatkozik, de az E0–E6 nincs engedélyezve, ezért a
hozzárendelés itt nem szerepel. TC-09 hasonlóan: nincs önálló AIQT-pontja.

## 8. AC-kapuk → AIQT-pontok

| Kapu | Tartalom | Forrás | Állapot a felhős rendszerben | Hozzárendelt AIQT-pontok |
|---|---|---|---|---|
| AC-04 | Koordináció | FINAL 1.0 / D-C4 | érvényes (D-C3 szerint) | – |
| AC-05 | Biztonság (negatív tesztek) | FINAL 1.0 | érvényes | AIQT-P2, AIQT-C18, AIQT-C20, AIQT-S01, AIQT-S02, AIQT-S11, AIQT-S12, AIQT-S13, AIQT-S14, AIQT-S15 |
| AC-06 | CFA4 CI-validáció (SKIPPED/BLOCKED/NOT_RUN ≠ siker) | FINAL 1.0 / D-C4 | érvényes (R2: CI; R3: referenciagép) | AIQT-P1, AIQT-C04, AIQT-C07, AIQT-C12 |
| AC-07 | Hatékonyság | FINAL 1.0 | érvényes, a pilotig nincs mérés | AIQT-C05 |
| AC-08 | AIQT prioritási és minőségvédelmi szabályok | FINAL 1.1 | érvényes | AIQT-P0, AIQT-P2, AIQT-P3, AIQT-P5, AIQT-R1, AIQT-R2, AIQT-R3, AIQT-R5, AIQT-C12 |
| AC-09 | Állítások és források bizonyítékkapcsolata | FINAL 1.1 | érvényes | AIQT-P1, AIQT-P4, AIQT-C01, AIQT-C02, AIQT-C03, AIQT-C04, AIQT-C05, AIQT-C06, AIQT-C07, AIQT-C08, AIQT-C09, AIQT-C10 |
| AC-10 | Hatókör és jóváhagyás védelme | FINAL 1.1 | érvényes | AIQT-P4, AIQT-P5, AIQT-R3, AIQT-R4, AIQT-C11, AIQT-C13, AIQT-C14, AIQT-C15, AIQT-C18, AIQT-C19, AIQT-C20, AIQT-S03, AIQT-S06, AIQT-S13, AIQT-S15 |
| AC-11 | Titok-, jogosultság- és prompt injection védelem | FINAL 1.1 | érvényes | AIQT-S01, AIQT-S02, AIQT-S03, AIQT-S04, AIQT-S05, AIQT-S06, AIQT-S07, AIQT-S08, AIQT-S09, AIQT-S10, AIQT-S11, AIQT-S14, AIQT-S16, AIQT-S17, AIQT-X1 |
| AC-12 | Biztonságos újrapróbálkozás és helyreállítás | FINAL 1.1 | érvényes | AIQT-C11, AIQT-C17 |
| AC-13 | Szabályparitás (Pi, Claude Code, EGM) | FINAL 1.1 | csak a Claude Code-ág; a Pi-rész SZÜNETEL (D-C2) | AIQT-C15, AIQT-C16, AIQT-S07, AIQT-X1, AIQT-X3 |
| AC-14 | AIQT megfelelőségi és negatív tesztek | FINAL 1.1 | érvényes; szimulált teszt önmagában nem elég (FINAL 1.1 8.) | AIQT-R1, AIQT-C02, AIQT-C04, AIQT-C05, AIQT-C12, AIQT-C17, AIQT-S08, AIQT-S12, AIQT-S16, AIQT-X3 |

AC-01–AC-03 nincs a táblában: platform- és ügynökműködési kapuk, nem AIQT-pontokhoz kötöttek. Az AC-14 negatív tesztjei a FINAL 1.1 8. pontjában
vannak (nem létező teszteredmény sikerként jelentése, engedély nélküli módosítás, hatókörön kívüli automatikus javítás, hiányzó merge-engedély,
időtúllépés utáni ismétlés, külső fájl felülírási kísérlete, hiányzó review-bizonyíték, kihagyott CI-teszt, becslés mérésként, biztonsági hiba).
Ezeket a ténylegesen konfigurált rendszerben kell igazolni, nem szimulált API-válaszokkal (FINAL 1.1 8.).

## 9. Végrehajtási rétegek szerint

| Réteg | Mit igazol az AIQT-ból | Pontok |
|---|---|---|
| R1 – munkamenet | magatartás, hatókör, jóváhagyás, titok- és eszközhasználat | AIQT-P0, AIQT-P1, AIQT-P2, AIQT-P3, AIQT-P4, AIQT-P5, AIQT-R1, AIQT-R2, AIQT-R3, AIQT-R4, AIQT-R5, AIQT-C01, AIQT-C02, AIQT-C03, AIQT-C04, AIQT-C05, AIQT-C06, AIQT-C07, AIQT-C08, AIQT-C09, AIQT-C10, AIQT-C11, AIQT-C12, AIQT-C13, AIQT-C14, AIQT-C15, AIQT-C16, AIQT-C17, AIQT-C18, AIQT-C19, AIQT-C20, AIQT-S01, AIQT-S02, AIQT-S03, AIQT-S04, AIQT-S05, AIQT-S06, AIQT-S07, AIQT-S08, AIQT-S09, AIQT-S10, AIQT-S11, AIQT-S12, AIQT-S13, AIQT-S14, AIQT-S15, AIQT-S16, AIQT-S17, AIQT-X1 |
| R2 – CI-kapu | a lefutott kapu mint megfigyelés; a nem lefutott nem PASS | AIQT-P0, AIQT-P1, AIQT-P2, AIQT-P3, AIQT-P4, AIQT-R1, AIQT-R3, AIQT-C02, AIQT-C04, AIQT-C12, AIQT-C20, AIQT-S02, AIQT-S12, AIQT-S13, AIQT-S17 |
| R3 – referenciagép | mért értékek, GPU/Wayland/Qt6 viselkedés (alkalmanként, tulajdonosi engedéllyel) | AIQT-C05, AIQT-C07 |

## 10. Karbantartás

- Az AIQT új verziója (a jelenlegi: 1.0.6) új profilrevíziót kér: az új fájl külön könyvtárba kerül (`docs/reference/aiqt/<verzió>/`), a régi
  nem íródik át, a profil a különbségeket pontonként mutatja.
- A profil nem szabályszöveg. A kötelező szabályok forrása változatlanul `AGENTS.md`, `CLAUDE.md`, `docs/coordination.md`; a profil csak hivatkozik.
- A WP2 szabályjegyzék (`rules.toml`) hivatkozhat az itteni AIQT-azonosítókra; ez a profil nem függ tőle.

## 11. Ellenőrzés: a lista teljessége

Az AIQT-fájl szakaszai és a profil sorai (a számok a fájlból és a profilból számolva):

| AIQT-szakasz | Pontok a fájlban | Sor a profilban |
|---|---|---|
| Prioritás (rendezés + négy érték + megállási szabály) | 6 | 6 |
| Rules | 5 | 5 |
| Conduct (16 mindig + 4 eszközös) | 20 | 20 |
| Security (10 mindig + 7 eszközös) | 17 | 17 |
| Hatókör-megjegyzések (eszközös feltétel; fejlesztési idejű védelmek; beillesztési útmutató) | 3 | 3 |
| **Összesen** | **51** | **51** |

A fájlbeli darabszámot a fájl félkövér bevezetőiből és számozott szabályaiból számoltam (lásd a PR-leírást); **a repóban gépi paritásellenőrzés ezekre még nincs** (a WP2
teszt kiterjeszthetné).

## 12. Mi nem igazolt

- A „Ma” oszlop a repó fájljaiból való (a hook, az alügynökök, a munkafolyamatok, a szabályfájlok olvasása). A hook és a munkafolyamatok **viselkedését
  nem futtattam le**: az oszlop azt mutatja, mi van beállítva, nem azt, mit eredményez (AIQT-C07).
- A felhős környezet beállításait (hálózat, GitHub-jogosultság) az F1 jelentés rögzíti; ebben a munkában újra nem mértem.
- Nem mondok megfelelőségi százalékot: nincs mérés.

## 13. Mi következik (nem engedély)

A WP2 (szabályjegyzék + paritásteszt) és WP3 (SPEC-gráf) külön munkamenetben, külön PR-ban. A P-1 és P-3 megállapítás szabály- illetve
konfigurációmódosítást igényelne; azok külön tulajdonosi döntés után, külön PR-ban mennének.

## 14. Források

- [AIQT 1.0.6](../../reference/aiqt/1.0.6/AIQT-1.0.6.txt) és [README](../../reference/aiqt/1.0.6/README.md).
- [FINAL 1.0](CFA4-DEV-AI-ORCHESTRATION-001-FINAL-1.0.md) (TC-01–TC-10, AC-01–AC-07, 8. szabályok);
  [FINAL 1.1 jegyzőkönyv](CFA4-DEV-AI-ORCHESTRATION-001-FINAL-1.1-record.txt) (TC-11–TC-15, AC-08–AC-14, hibakezelési állapotok, negatív tesztek);
  [FINAL 1.2](CFA4-DEV-AI-ORCHESTRATION-001-FINAL-1.2-felho.md) (R1–R3, D-C1–D-C5).
- [`AGENTS.md`](../../../AGENTS.md), [`CLAUDE.md`](../../../CLAUDE.md), [`docs/coordination.md`](../../coordination.md), [`docs/agents.md`](../../agents.md) Part 2.
- [F0 v3](../../audit/AI-ORCHESTRATION-F0-v3-2026-11-06.md), [F1 felhős felmérés](../../audit/AI-ORCHESTRATION-F1-felho-2026-11-06.md).
- F2 terv és WP0 szabályleltár (tulajdonosi jóváhagyás: 2026-11-25; a terv a `coordination.md` §2.11 szerint a `docs/plans/` mappába kerül külön lépésben, a leltár C-1 és D-F2-4 pontja szerint).
