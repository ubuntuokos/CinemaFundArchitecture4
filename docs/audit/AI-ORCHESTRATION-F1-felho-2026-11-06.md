# CFA4-DEV-AI-ORCHESTRATION-001 – F1 felhős kompatibilitási felmérés (AC-01a)

**Dátum:** 2026-11-06. **Issue:** [#336](https://github.com/ubuntuokos/CinemaFundArchitecture4/issues/336).
**Alap:** [FINAL 1.2](../development/ai-orchestration/CFA4-DEV-AI-ORCHESTRATION-001-FINAL-1.2-felho.md)
(D-C1–D-C5), [F0 v3](AI-ORCHESTRATION-F0-v3-2026-11-06.md).
**Engedély:** a tulajdonos 2026-11-06-án a „folytasd” utasítással indította; a munkamenet ezt az F1
felhős részének engedélyeként értelmezte. Hatókör: **csak olvasás és mérés** az R1 rétegben.
**Állapot:** F1-bizonyíték tulajdonosi felülvizsgálatra; **nem** F1-kapu PASS. F2–F9 és E0–E6 nincs engedélyezve.

## 1. Hatókör és módszer

A FINAL 1.0 F1 feladatai a FINAL 1.2 szerint (D-C2: ThinkRail és Pi szünetel):

| F1 | Feladat | Ebben a felmérésben |
|---|---|---|
| 1–2 | ThinkRail, Pi | **SZÜNETEL** (D-C2), nem vizsgált |
| 3 | Claude-modellkapcsolat | igen (R1) |
| 4 | Claude Code CLI | igen (R1) |
| 5 | GitHub-jogosultságok | igen, a munkamenet szemszögéből |
| 6 | függőségi és biztonsági ellenőrzés | a munkamenet eszközkészletére és hálózatára (TC-14) |
| 7 | host-sértetlenség | a FINAL 1.2 AC-01 szerint: tároló és GitHub-állapot |

Nem történt: telepítés, `rustup`/`cargo` a tároló könyvtárában, beállítás-módosítás, titokérték
kiolvasása (környezeti változóknál csak a név és az, hogy üres-e). Hálózati próba: négy
nyilvános URL egy-egy GET kérése. Push-próba: csak `git push --dry-run`.

## 2. Eredmények

| # | Tétel | Eredmény | Állapot |
|---|---|---|---|
| 1 | Claude Code | 2.1.318, felhős munkamenetben fut | VERIFIED |
| 2 | Modellkapcsolat | a munkamenet modellje a felhős munkamenet beállítása; a tároló `.claude/settings.json`-ja `opusplan`-t kér. Hogy a felhőben melyik érvényesül, nem ellenőriztem | VERIFIED (kapcsolat), PENDING (modellválasztás) |
| 3 | Git-azonosság | `user.name=Claude`, a commitok SSH-kulccsal aláírva (`commit.gpgsign=true`, a HEAD-commitban van `gpgsig`); helyben nem ellenőrizhető (`%G?` = N) | VERIFIED (aláírás megvan) |
| 4 | Hitelesítő adatok a konténerben | a `GH_TOKEN`, `GITHUB_TOKEN`, `AWS_*`, `CLOUDSDK_AUTH_ACCESS_TOKEN` változók mind 14 karakteresek, azonos hosszúak; valószínűleg helyettesítő értékek, a valódi hitelesítést a kimenő proxy végzi | INFERRED |
| 5 | Hálózat | kimenő HTTPS proxyn át; tetszőleges nyilvános hoszt elérhető (`example.com`, `www.debian.org`: 200) | VERIFIED – **széles hálózati hozzáférés** |
| 6 | GitHub-eszközök a munkamenetben | a GitHub MCP-eszközkészlet tartalmaz merge-, auto-merge-, workflow-indító/-újrafuttató, fájltörlő és címkéző műveletet | VERIFIED (a munkamenet eszközlistája) |
| 7 | Push más ágra | a `git push --dry-run` új ágra és a `main`-re is kliensoldalon átmegy; a tényleges korlátot a szerver (ruleset, proxy) adja, ezt próba nélkül nem lehet igazolni | PENDING (negatív teszt kell, AC-05) |
| 8 | Ruleset | nem olvastam újra (a munkamenetnek nincs rá eszköze); a v1 szerint 0 kötelező jóváhagyás; a `full-validation-gate` kötelező (DOCUMENTED) | PENDING |
| 9 | „Csak olvasó” alügynökök | a `cfa4-ci-runner` és a `cfa4-branch-lister` `Bash` eszközt kap; a csak olvasás **csak a promptban** szerepel | VERIFIED – gap |
| 10 | Betöltött bővítmények | a munkamenetben a CFA4-hez nem kapcsolódó bővítmények is be vannak töltve (GTM/értékesítési, design-, Slack/Notion/Figma-összekötők, issue-triázs ügynök) | VERIFIED – gap (legkisebb jogosultság) |
| 11 | Host-sértetlenség (R1) | munkafa tiszta, nincs `core.hooksPath`, `rustup` nem töltött le toolchaint, a konténerben csak `stable` | VERIFIED |
| 12 | Host-sértetlenség (GitHub) | a munkamenet a #650-en címkét tett fel és vett le (a teljes CI indításához), és egy hibás jobot újraindított; mindkettő a tároló szokásos eljárása | VERIFIED, bejelentve |

**Eltérés (bejelentendő):** ebben a munkamenetben a CI-állapot olvasásához néhány alkalommal
`gh api` olvasó hívás futott, holott a környezet a `gh`-t csak keresésre engedi. Írási művelet
`gh`-val nem történt.

## 3. AC-01a értékelése

| Ellenőrizendő (FINAL 1.0 AC-01 → FINAL 1.2 AC-01a) | Állapot |
|---|---|
| Claude-modellkapcsolat | PASS (működik) |
| Claude Code | PASS (működik) |
| Git/GitHub-integráció | PASS (klón, aláírt commit, push, PR, CI-esemény) |
| Szükséges függőségek | részben: az R1-ben nincs FFmpeg 9.0.2, Qt6, GPU – ezeket a FINAL 1.2 az R2-höz és R3-hoz rendeli |
| Host Non-Interference | PASS az R1-ben; GitHub-oldalon a 12. pont szerinti, eljárásszerű műveletek |
| Jogosultságok a legkisebb szükséges szinten | **FAIL**: lásd 4. pont |

**Összesítés:** az AC-01a működési része PASS, a jogosultsági része FAIL. Az F1 kilépési feltétele
(„hitelesítési és működési PASS”) ezért **nem teljesül** a 4. pont rendezéséig.

## 4. Hiányok (TC-14, AC-05, AC-11)

| ID | Hiány | Kockázat | Javaslat |
|---|---|---|---|
| F1-G1 | a munkamenet tud beolvasztani és auto-merge-et kapcsolni; csak a szabály és a CI-kapu tartja vissza (0 kötelező jóváhagyás, v1) | jogosulatlan merge (I-01 megismétlődése) | a rulesetben **1 kötelező jóváhagyás** a tulajdonostól, vagy a munkamenetek GitHub-hozzáférése merge-jog nélkül |
| F1-G2 | széles kimenő hálózat | prompt-injekcióval adatkiszivárgás, nem jóváhagyott letöltés | a felhős környezet hálózati szintjét „Limited”-re állítani, a szükséges hosztok (crates.io, PyPI, GitHub, Ubuntu-tükrök, FFmpeg-forrás) engedélyezésével – előtte mérni, mi kell |
| F1-G3 | nem kapcsolódó bővítmények és összekötők a munkamenetben | felesleges eszközfelület | a CFA4-projekt munkameneteiben csak a szükséges bővítmények |
| F1-G4 | a „csak olvasó” alügynökök Bash-t kapnak | véletlen írás | `tools`-ból a Bash szűkítése vagy írási parancsok tiltása hookkal |
| F1-G5 | a push-korlát és a ruleset nincs negatív teszttel igazolva | AC-05 nem igazolt | F3-ban negatív teszt: push a `main`-re, más munkamenet ágára, merge kísérlet – csak a tulajdonos engedélyével |

## 5. Tulajdonosi döntést igénylő pontok

| ID | Kérdés | Javaslat |
|---|---|---|
| D-F1-1 | Kötelező jóváhagyás (1 fő) a `main` rulesetjébe (F1-G1)? | igen; a gazda így sem olvaszthat be jóváhagyás nélkül |
| D-F1-2 | Szűkebb hálózati szint a felhős környezetben (F1-G2)? | igen, de előbb egy mérés, mely hosztok kellenek a buildhez |
| D-F1-3 | A nem kapcsolódó bővítmények kikapcsolása a projekt munkameneteiben (F1-G3)? | igen |

A ruleset és a környezeti beállítás a tulajdonos fiókjában módosítható; ez a munkamenet nem
változtat rajtuk.

## 5a. Tulajdonosi döntések (2026-11-06) és végrehajtásuk

A tulajdonos mindhárom javaslatot elfogadta (D-F1-1, D-F1-2, D-F1-3).

| ID | Végrehajtás | Állapot |
|---|---|---|
| D-F1-2 | Új, személyes **„CFA4”** felhős környezet (a tulajdonos jóváhagyásával): korlátozott hálózat, csomagkezelők és MCP-szerverek engedélyezve, további hosztok: `github.com`, `api.github.com`, `codeload.github.com`, `objects.githubusercontent.com`, `static.rust-lang.org` (a `rust-toolchain.toml` toolchainje), `ffmpeg.org` (`scripts/ci/install-ffmpeg-9.0.2.sh`). A Default környezet változatlan, mert a tulajdonos többi tárolóját is kiszolgálja. | létrehozva; a CFA4-munkameneteknek ezt kell választaniuk. Működése egy új munkamenetben igazolandó (ez a munkamenet nem költöztethető át) – PENDING |
| D-F1-3 | A nem kapcsolódó bővítmények kikapcsolása a tulajdonos claude.ai-fiókjában | a tulajdonosra vár |
| D-F1-1 | Kötelező jóváhagyás a `main` rulesetjében | **akadály:** a munkamenetek PR-jai a tulajdonos fiókja nevén jönnek létre, a GitHub pedig nem engedi a saját PR jóváhagyását; a kötelező jóváhagyás így minden beolvasztást blokkolna. Kivétel (bypass) a tulajdonosnak a munkameneteknek is kivételt adna, mert azok ugyanazzal az azonossággal dolgoznak. Feltétele a munkamenetek külön azonossága (bot-fiók vagy GitHub App, merge-jog nélkül) – az I-01 javaslatával egyezik. Tulajdonosi döntésre vár. |

**D-F1-1 döntés (2026-11-06):** a tulajdonos a külön bot-fiókot választotta. Lépések:

| # | Lépés | Ki |
|---|---|---|
| 1 | GitHub bot-fiók létrehozása saját e-mail-címmel és kétlépcsős azonosítással | tulajdonos |
| 2 | A bot-fiók meghívása a tárolóba **Write** szerepkörrel (nem Maintain, nem Admin) | tulajdonos |
| 3 | A claude.ai GitHub-kapcsolatának átállítása a bot-fiókra (`claude.ai/connect-github`). Ez a tulajdonos **minden** felhős munkamenetére érvényes, ezért a bot-fiókot a többi használt tárolóhoz is hozzá kell adni | tulajdonos |
| 4 | Ruleset a `main`-en: 1 kötelező jóváhagyás, elavult jóváhagyások elvetése új pushnál, a legutóbbi push jóváhagyása, üres bypass-lista | tulajdonos, a 3. lépés után |
| 5 | Negatív teszt (F1-G5, F3): a bot nem tud jóváhagyás nélkül beolvasztani, nem tud a `main`-re pusholni | munkamenet, külön engedéllyel |

A 4. lépés a 3. előtt minden beolvasztást blokkolna (saját PR nem hagyható jóvá).

## 6. Biztonság, költség, visszaállítás

- Titokérték nem került kiolvasásra és nem került a dokumentumba.
- Költség: a #650 egy teljes CI-futása és egy job-újrafuttatás.
- Visszaállítás: új dokumentum és egy tartalomjegyzék-sor; revert-tel visszavonható.
