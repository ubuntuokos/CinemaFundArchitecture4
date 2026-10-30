# CFA4-DEV-AI-ORCHESTRATION-001 – F0-audit (v2)

**Dátum:** 2026-10-30 (Europe/Budapest). **Issue:** [#336](https://github.com/ubuntuokos/CinemaFundArchitecture4/issues/336).
**Baseline:** `main` @ `973a73e91e353c24238dd0ba206395900233854f`.
**Állapot:** F0-bizonyíték tulajdonosi felülvizsgálatra. **Nem** elfogadott F0-kapu; F1–F9 és E0–E6 nincs engedélyezve.

Előzmény: F0-audit v1 ([#336 hozzászólás](https://github.com/ubuntuokos/CinemaFundArchitecture4/issues/336#issuecomment-6511060751),
baseline `24ba4f7`). Ez a v2 a v1-et kiegészíti, nem írja felül.

## 1. Hatókör és módszer

- Csak olvasás: GitHub API (PR-ok, check-runok, ruleset-adatok a v1-ből), a `main` friss klónja.
- Tesztfuttatás: csak a 16 Python-egységteszt a Superpowers-előellenőrzőhöz (lásd 4.), valamint a tulajdonos kérésére egy utólagos teljes CI a `main`-re (lásd 6.).
- Nem történt: telepítés, a tulajdonos gépének módosítása, kódmódosítás, beolvasztás, workflow-módosítás.
- A jelölések: **VERIFIED** = megfigyelt, bizonyítékkal; **PENDING** = nem ellenőrzött; **BLOCKED** = a feltétel megfigyelten hiányzik; **NOT RUN** = nem futott. Egyik sem számít a másiknak.

## 2. Forrásdokumentumok

| Dokumentum | Állapot | Hely |
|---|---|---|
| FINAL 1.0 részletes terv | VERIFIED (a tulajdonos csatolta és elfogadta; utólag összeállított változat) | `docs/development/ai-orchestration/` |
| FINAL 1.0 jóváhagyási feljegyzés, FINAL 1.1 kiegészítés és jóváhagyás | VERIFIED | `docs/development/ai-orchestration/` |
| AIQT 1.0.6 | VERIFIED (16 448 bájt, egyezik a v1-ben rögzített mérettel) | `docs/reference/aiqt/1.0.6/` |
| CFA4-AIQT-PROFILE-001 | PENDING – a FINAL 1.1 szerint **elkészítendő** dokumentum, nem hiányzó bemenet (F2) | tervezett hely: `docs/development/ai-orchestration/` |

A v1 BLOCKED pontja („FINAL 1.0 hiányzik, AC-01–AC-07 nem ismert”) ezzel megszűnt.
Hash-ek és a tárolóbeli példányok változtatásai: `docs/development/ai-orchestration/README.md`.

## 3. Változások a v1 óta

| Tétel | v1 | v2 |
|---|---|---|
| Baseline | `24ba4f7` | `973a73e` (+#334, #335, #330) |
| Nyitott PR-ok | 56 | 50 (2026-10-30 16:15 körül) + #338 |
| EGM (#334) | nyitott PR | a `main`-en: `docs/development/egm-design-review.md`, `.agents/skills/cfa4-elephant/` – csak dokumentáció |
| Superpowers-előellenőrző (#335) | nyitott PR | a `main`-en, `enabled=false` |
| Koordináció (#330) | nyitott PR | a `main`-en; a tulajdonos szerint idő előtt → visszavonó PR: #338 |
| Új kapcsolódó munka | – | #337 (draft, SkillSpector) – más munkamenet, érintetlen |

## 4. VERIFIED / PENDING / BLOCKED mátrix

| Tétel | Állapot | Bizonyíték / korlát |
|---|---|---|
| Repository-baseline | VERIFIED | `973a73e`, friss klón |
| Ágak és PR-ok leltára | VERIFIED | GitHub API, 2026-10-30 |
| OS, FFmpeg 9.0.2, Qt 6.10.2, Python 3.14.4 a célgépen | VERIFIED a v1-ben; **nem ellenőriztem újra** | csak a célgépen mérhető |
| `heif-dec` / `heif-enc` a célgépen | BLOCKED (v1) | hiányzik; az AC-06 HEIF-pontját blokkolja |
| Rust 1.97.1 eszköztár a célgépen | VERIFIED (v1); a tulajdonos döntése: **marad** | F0-01, lezárva |
| Claude Code CLI | PENDING | futtatható (v1); bejelentkezés és integráció nem ellenőrzött |
| ThinkRail, Pi | PENDING | a v1 szerint nincsenek a PATH-on |
| Superpowers-előellenőrző 16 egységtesztje | VERIFIED **sandboxban** (16/16 OK), célgépen PENDING | Ubuntu 24.04.5, Python 3.13.18, x86_64; `PYTHONPATH=python python3 -m unittest discover -s python/tests -p test_superpowers_cost.py` |
| Teljes CFA4 CI a `main`-en | futtatva (6. pont) | run 41081470525 |
| `test_independence.py` a `main`-en | **FAIL** (sandbox) | 1 failed / 3 passed – lásd I-02 |
| `scripts/ci/check-linux-amd64.sh` | NOT RUN helyben | a szkript Ubuntu 24.04-et elutasít (exit 2); a GitHub-on a CI futtatja |
| Ruleset a `main`-en | VERIFIED (v1) | PR kötelező, `cargo-check` kötelező, **0 kötelező jóváhagyás**, a teljes CI nem kötelező check |

## 5. Gap-elemzés a FINAL 1.0 / 1.1 szerint

### TC-01–TC-15

| TC | A terv elfogadási feltétele (röviden) | Meglévő | Hiány / állapot |
|---|---|---|---|
| 01 Agent Policy Adapter | minden ügynök ugyanazokat a korlátokat kapja, végrehajtási szinten is | `AGENTS.md`, `CLAUDE.md`, `VISION.md`, `docs/coordination.md` | Csak szöveges; Pi-oldali megfelelő nincs; **belső ütközés** (5.3) – PENDING |
| 02 Workspace Coordinator | egy író munkamenet = egy kizárólagos worktree | `coordination.md` §2, `CLAUDE.md` (worktree + `CARGO_TARGET_DIR`) | Nincs zárolás, tulajdon-nyilvántartás – PENDING |
| 03 Context/Spec Bridge | Pi és Claude Code ugyanarra a jóváhagyott követelményverzióra | tervek a `docs/`-ban; mostantól a FINAL 1.0/1.1 is | SPEC-gráf, követelményazonosítók – PENDING |
| 04 Evidence Collector | bizonyíték nélkül nincs lezárás | `docs/agents.md` „Reporting progress honestly”, `docs/audit/` | Egységes bizonyítékséma (Task ID, base/head SHA, CI run, approval) – PENDING |
| 05 Approval & Gate Controller | nincs jogosulatlan merge/release | ruleset (PR + `cargo-check`), `coordination.md` §3.4 | **0 kötelező jóváhagyás, teljes CI nem kötelező; 2026-10-30-án teljes CI nélküli beolvasztás történt (6. pont)** – BLOCKED |
| 06 EGM Workflow Adapter | Claude Code-os EGM-folyamat reprodukálható | `egm-design-review.md`, `cfa4-elephant` skill | Telepítés és futtatás nincs – PENDING |
| 07 Fresh Context Validator | igazoltan friss kontextusú Goldfish-review | szabály: `GOLDFISH_UNAVAILABLE` | Végrehajtott, igazolt izoláció nincs – PENDING |
| 08 Design Review Registry | kritikus megállapítások ellenőrizhető állapotban | ellenőrzési rekord-sablon az EGM-doksiban | Nyilvántartás nincs – PENDING |
| 09 Specification Drift Monitor | eltérések kimutathatók | eljárásleírás | Futtatott drift-jelentés nincs – PENDING |
| 10 Agent Efficiency Evaluator | A/B/C mérés | offline előellenőrző (`enabled=false`) | Bázismérés (S0 / A-csoport) nincs – BLOCKED |
| 11 AIQT Compliance Validator | AIQT gépi ellenőrzése + negatív tesztek | – | Nincs – PENDING (F2 után) |
| 12 Source & Claim Provenance | állítás–forrás összerendelés | Git-előzmények, ez az audit | Állítás-séma (OBSERVED/INFERRED/…) nincs – PENDING |
| 13 Scope & Change Boundary Guard | engedélyezett hatókör védelme | `coordination.md` §2 | Kikényszerítés nincs – PENDING |
| 14 Security & Tool Execution Guard | eszköz-, hálózat-, titokvédelem | minden workflow `contents: read` | Ügynökszintű allowlist, titokszabály az `AGENTS.md`-ben nincs – PENDING |
| 15 Recovery & Idempotency | ismétlés előtt állapotegyeztetés | – | Nincs szabály – PENDING |

### AIQT 1.0.6 ↔ meglévő CFA4-szabályok

| AIQT | CFA4 | Állapot |
|---|---|---|
| Mért és becsült adat szétválasztása; stub ≠ kész | `docs/agents.md` „Reporting progress honestly” | megvan (dokumentált) |
| Kész = ténylegesen lefutott ellenőrzés | `CLAUDE.md` quality gates, `docs/agents.md` §7 | megvan |
| Hatókörön kívüli dolog jelzése | `AGENTS.md` (VISION-ütközés jelentése), `coordination.md` §2 | részben |
| Csak kifejezett engedély után induljon munka | `docs/agents.md` §5–6 és `CLAUDE.md` 3. lépés: autonóm munkahurok | **ütközés** (5.3) |
| Titkok kizárása naplókból, artefaktumokból | csak az EGM-doksiban és a `cfa4-elephant` skillben | hiány |
| Külső tartalom = adat, nem utasítás | nincs általános szabály | hiány |
| Legkisebb jogosultság | CI-ben megvan (`contents: read`) | részben |
| Fail-closed | Superpowers-előellenőrző DENY | részben |
| Célrendszer megerősítése, biztonságos ismétlés | nincs | hiány |
| Előnézet/audit nem módosít | EGM drift nem szerkeszt; a v1 F0-01 eseménye ezt sértette | részben |

### 5.3 Hatókörön kívüli megállapítások (tulajdonosi döntésre, nem javítva)

1. **Autonóm munkahurok ↔ előzetes jóváhagyás.** A `docs/agents.md` §5 („commit, majd a következő feladat”), §6 („ne blokkolj”) és a `CLAUDE.md` 3. lépése („Don't stop to ask…”) ellentmond a FINAL 1.0 8.7. pontjának („minden fejlesztési feladat előzetes jóváhagyáshoz kötött”), a `coordination.md` 20-as PR-korlátjának és az AIQT-nek.
2. **Fordításmentes CI ↔ `pr-check.yml`.** Az `AGENTS.md` 17. sora szerint az automatikus PR-ellenőrzés fordításmentes; a `.github/workflows/pr-check.yml` minden PR-on `cargo check`-et futtat (a v1 is jelezte).
3. **Ruleset:** 0 kötelező jóváhagyás, a teljes CI nem kötelező check (TC-05, AC-05, AC-10).

## 6. Incidensek

| ID | Esemény | Bizonyíték | Állapot |
|---|---|---|---|
| I-01 | 2026-10-29/30 éjjel a tulajdonos fiókja nevén: #334, #335, #330 beolvasztása, #336 issue és F0 v1 hozzászólás. A tulajdonos szerint egy másik MI-eszköz (ChatGPT) járt el a nevében; **ismeretlen, melyik lépést ki végezte**. | GitHub-időbélyegek (00:32–01:01) | UNKNOWN szerzőség; javasolt intézkedés: a külső MI-eszközök írási jogának szűkítése, külön bot-fiók merge-jog nélkül |
| I-02 | #335 beolvasztva a `b3d1980` fejen **teljes CI nélkül** (a 8 teljes job skipped; csak statikus + `cargo-check` zöld; 3 perccel később merge). Sérti a `coordination.md` §3.4-et. | run 40976153695 | Utólagos teljes CI a `main`-en (`973a73e`, tartalmazza a #335-öt): run [41081470525](https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/41081470525) – **eredmény a futás végén, a PR-ban kerül rögzítésre** |
| I-03 | #330 (koordinációs szabály) idő előtt beolvasztva | merge `973a73e` | Visszavonó PR: #338 – a gazda olvasztja be |
| F0-01 | rustup a v1 alatt letöltötte az 1.97.1 eszköztárat a célgépen | v1 | Lezárva: a tulajdonos döntése szerint marad |

**I-02 következménye (VERIFIED, sandbox):** a `main` (`973a73e`) a `python/tests/test_independence.py::test_no_upstream_brand_in_repository`
tesztet **elbuktatja**: a #335 két fájlja (`docs/plans/CFA4-SUPERPOWERS-COST-001.md` 20. sor,
`docs/development/superpowers-policy.md` 14. sor) visszahozta az upstream márkanevet.
Futtatás: `python3 -m pytest -q python/tests/test_independence.py` → 1 failed, 3 passed
(Ubuntu 24.04.5, Python 3.13.18). Hatókörön kívüli hiba: **nem javítottam**, külön PR kell rá a tulajdonos döntése után.

A #334 a beolvasztott `82c71de` fejen zöld teljes CI-vel ment be (run 40824027165) – megfelel a §3.4-nek.

## 7. AC-01–AC-14 megfeleltetés

| AC | Mit kell igazolni (terv) | Jelenleg |
|---|---|---|
| AC-01 Platform | ThinkRail, Pi, Claude, Claude Code, Git/GitHub Ubuntu 26.04-en, host-érintetlenség | NOT RUN; ThinkRail/Pi PENDING |
| AC-02 Ügynökműködés | Pi + Claude és Claude Code külön-külön | NOT RUN |
| AC-03 EGM | teljes plugin, perszónák, friss kontextus | NOT RUN |
| AC-04 Koordináció | worktree-tulajdon, handoff | NOT RUN |
| AC-05 Biztonság | negatív tesztek, köztük jogosulatlan merge | NOT RUN; **I-01/I-02 valós ellenpélda** |
| AC-06 CFA4 CI | teljes CFA4 CI, HEIF, J2K, VFR, NVENC/NVDEC stb. | BLOCKED a célgépen (`heif-dec`/`heif-enc`); GitHub-on futó teljes CI: 6. pont |
| AC-07 Hatékonyság | A/B/C pilot | BLOCKED – nincs bázismérés |
| AC-08 – AC-14 | AIQT-prioritás, állítás–forrás, hatókör, titok/injekció, ismétlés, paritás, negatív tesztek | NOT RUN |

## 8. Sorrend (a terv szerint, nem engedély)

F0 → F1–F3 (alapinfrastruktúra, biztonsági kapuk) → F4–F5 (két ügynökútvonal külön) →
E0–E3 (EGM, az alapinfrastruktúra után párhuzamosan) → F6–F7 + E4–E5 → F8 + E6 → F9.
Az AIQT-kiegészítések a FINAL 1.1 7. pontja szerint az egyes fázisokba épülnek.
A Qt6- és kodekfejlesztés ütemezése emiatt nem állhat le (FINAL 1.0, 6. pont).

## 9. Biztonság, költség, visszaállítás

- Titok nem került sem a dokumentumokba, sem a bizonyítékokba.
- Fizetős ügynökhívás nem történt. Költség: egy teljes CI-futás a tulajdonos kérésére (I-02).
- Visszaállítás: ez a PR csak új dokumentumokat ad hozzá, egy sort a `docs/audit/README.md`-hez; revert-tel nyomtalanul visszavonható.

## 10. Tulajdonosi döntések (2026-10-30)

- A gazda olvasztja be a #338-at.
- A #335-re utólagos teljes CI fusson.
- Az éjszakai lépések szerzősége ismeretlen.
- Az 1.97.1 eszköztár marad.
- Az AIQT-ből csak az kerül át, ami nem mond ellent a CFA4-szabályoknak.
- A dokumentumimport jóváhagyva; az F0 v2 a #336-ra kerül.

## 11. Javasolt következő hatókör (jóváhagyásra)

1. Az 5.3 pontban felsorolt ütközések eldöntése (különösen az autonóm munkahurok).
2. Ezután **F1** (kompatibilitás) és/vagy **F2** (szabályok + CFA4-AIQT-PROFILE-001) – csak külön, kifejezett engedéllyel.
