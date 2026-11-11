# CFA4-DEV-AI-ORCHESTRATION-001 – F0-audit v3: felhős fejlesztési környezet

**Dátum:** 2026-11-06. **Issue:** [#336](https://github.com/ubuntuokos/CinemaFundArchitecture4/issues/336).
**Baseline:** `main` @ `75a8584`.
**Állapot:** F0-bizonyíték; a D-C1–D-C5 döntések 2026-11-06-án megszülettek (7a). **Nem** elfogadott F0-kapu; F1–F9 és E0–E6 nincs engedélyezve.

Előzmény: [F0 v2](AI-ORCHESTRATION-F0-2026-10-30.md) (baseline `973a73e`). Ez a v3 a v2-t
kiegészíti, nem írja felül. Kiváltó ok: a tulajdonos 2026-11-06-i közlése (a Worktrunk-terv D6
döntése, [#642](https://github.com/ubuntuokos/CinemaFundArchitecture4/pull/642)), amely szerint
**a CFA4 fejlesztése a GitHubon, felhős Claude Code munkamenetekben folyik**, nem a tulajdonos
helyi gépén. Minden munkamenet saját, ideiglenes konténerben, friss klónnal és saját ággal dolgozik;
az integráció PR-on át megy.

A FINAL 1.0 / 1.1 a fejlesztési infrastruktúra célkörnyezetének a tulajdonos Ubuntu 26.04-es gépét
tekinti (FINAL 1.0 fejléc, 8.1, AC-01), és a párhuzamosságot egy gépen, egy klónban, worktree-kkel
képzeli el (TC-02, F3, F8, AC-04). Ez a v3 felméri, mely részeket érinti a tényleges működés, és
javaslatot tesz az igazításra. **A terv szövegét nem módosítja**; a javaslatok tulajdonosi
döntésre várnak (7. pont).

## 1. Hatókör és módszer

- Csak olvasás: a `main` friss klónja, a #336 hozzászólásai, a #642, a workflow-fájlok, a felhős
  konténer adatai.
- A v1 F0-01 és a WT-F0 eltérése (a `rustup` letöltötte az 1.97.1-et) **nem ismétlődött**: ebben a
  munkamenetben sem `rustup`, sem `cargo` nem futott a tároló könyvtárában; a konténerben csak a
  `stable` toolchain van.
- Nem történt: telepítés, kódmódosítás, workflow-módosítás, beolvasztás, CI-indítás.
- A ruleset aktuális állapotát API-n **nem** olvastam újra; ahol erre hivatkozom, a forrás a
  `docs/coordination.md` és a `.github/workflows/ci-linux.yml` (jelölés: DOCUMENTED).

## 2. A tényleges fejlesztési környezet (mérés, 2026-11-06)

| Tétel | Felhős munkamenet-konténer | Terv szerinti célgép (`docs/development/reference-machine.md`) |
|---|---|---|
| OS | Ubuntu 24.04.5 LTS, x86_64 | Ubuntu Studio 26.04, amd64 |
| CPU / RAM | 4 vCPU, 15 GiB | 88 szál, 512 GiB |
| Lemez | munkamenetenkénti keret (~30 GB szabad) | – |
| Git | 2.43.0 | – |
| FFmpeg | 6.1.1 (Ubuntu-csomag) – **nem** a 9.0.2-es baseline | 9.0.2 (v1) |
| Qt6 fejlesztői eszközök, `heif-enc` | nincs | Qt 6.10.2; `heif-*` hiányzott (v1) |
| GPU | nincs | RTX A1000 + RTX A4000 |
| Claude Code | 2.1.318 | futtatható (v1) |
| Élettartam | a munkamenet végén megszűnik | tartós |
| Hálózat | a környezet hálózati házirendje szerinti proxy | – |

**Ahol a CFA4 ténylegesen Ubuntu 26.04-en ellenőrződik:** a GitHub CI. Minden kötelező job
`ubuntu-26.04` futón fut, a teljes CI FFmpeg 9.0.2-t telepít és ellenőriz, és a HEIF/HEIC
orákulumok is ott futnak (`ci-linux.yml`). A kiadásjelölt-workflow egy önkiszolgáló futót is
megnevez (`[self-hosted, Linux, X64, cfa4-ubuntu-26-04]`); hogy ez regisztrálva és elérhető-e,
nem ellenőriztem.

Következmény: a fejlesztés **három rétegre** bomlik, és a terv minden pontját valamelyikhez kell kötni:

| Réteg | Hol | Mire alkalmas |
|---|---|---|
| **R1 – munkamenet** | felhős konténer | kód írása, gyors ellenőrzések, dokumentáció, PR |
| **R2 – kapu** | GitHub CI (`ubuntu-26.04`) | a kötelező tesztek a célplatformon; `full-validation-gate` |
| **R3 – referenciagép** | a tulajdonos gépe (Remote Control vagy önkiszolgáló futó) | GPU (CUDA/Vulkan, NVENC/NVDEC), Wayland/Qt6 GUI, minimumgép-mérés |

## 3. Változások a v2 óta

| Tétel | v2 (2026-10-30) | v3 (2026-11-06) |
|---|---|---|
| I-01 szerzőség | UNKNOWN | a tulajdonos pontosítása szerint Jamie, a tulajdonos másik asszisztens-ügynöke ([#336](https://github.com/ubuntuokos/CinemaFundArchitecture4/issues/336#issuecomment-6549323368)) |
| I-02 (`test_independence` piros) | #344 nyitott | #344 beolvasztva (`8d2abce`) |
| I-03 (#330 idő előtt) | #338 nyitott | #338 beolvasztva (`d804e3d`) |
| Teljes CI mint beolvasztási feltétel | nem kötelező check | DOCUMENTED: kötelező `full-validation-gate` (tulajdonosi döntés 2026-10-31, `coordination.md` §3.4) |
| Autonóm munkahurok ↔ előzetes jóváhagyás (v2 5.3/1) | nyitott ütközés | részben rendezve: projekthatár-szabály (2026-10-31, `coordination.md` §2.8, `docs/agents.md` §5) – az autonómia a jóváhagyott terven belülre korlátozódik |
| Worktrunk (a TC-02 tervezett eszköze) | – | 1.1 jóváhagyva (2026-11-05), majd **NOT_APPLICABLE**-ként lezárva (D6, 2026-11-06, #642 – még nincs a `main`-en) |
| Célkörnyezet | a tulajdonos gépe | felhős munkamenetek (2. pont) |

## 4. Helyi célgép-feltevések a tervben

Besorolás: **MARAD** = felhőben is változatlanul érvényes; **ÁTÉRTELMEZ** = a cél marad, az eszköz
vagy a hely változik; **R3** = csak a referenciagépen teljesíthető; **DÖNTÉS** = tulajdonosi döntés kell.

| Pont | Feltevés | Besorolás | Javasolt értelmezés |
|---|---|---|---|
| Fejléc „Célplatform: Ubuntu 26.04”, 8.1 | a fejlesztési rendszer is 26.04-en fut | DÖNTÉS (D-C1) | a **termék** és a **kapu (R2)** célplatformja 26.04 marad; a munkamenet-konténer OS-e nem célplatform |
| 2.2 ThinkRail | asztali munkafelület worktree-kkel, terminálokkal | DÖNTÉS (D-C2) | felhős munkamenetben nincs szerepe; csak helyi munkára maradhat |
| 2.3 Pi + Claude | helyi ügynökmotor | DÖNTÉS (D-C2) | mint fent; a Claude Code felhős munkamenet önmagában lefedi a 2.4-et |
| 2.4 Claude Code CLI | helyi folyamat | ÁTÉRTELMEZ | felhős munkamenet; „natív build és tesztfuttatás” = R1 gyors kör + R2 teljes CI |
| TC-02, F3/3–4, F8/1, AC-04 | worktree-izoláció, zárak | ÁTÉRTELMEZ | lásd 5. pont |
| F1/1–4 | ThinkRail, Pi, Claude Code kompatibilitása 26.04-en | ÁTÉRTELMEZ + DÖNTÉS | Claude Code: a felhős környezetben; ThinkRail/Pi: D-C2 szerint |
| F1/7, 8.12, AC-01 „Host Non-Interference” | a tulajdonos gépe ne sérüljön | ÁTÉRTELMEZ | a konténer eldobható; a védendő „host” a **tároló és a GitHub-állapot** (ágak, PR-ok, issue-k, ruleset), R3-hoz a referenciagép |
| AC-06 | teljes CFA4 CI | ÁTÉRTELMEZ | R2 teljesíti (FFmpeg 9.0.2, HEIF, 26.04); **CUDA/Vulkan, NVENC/NVDEC, Wayland, Qt6 GUI = R3** – GitHub-futón nincs GPU |
| AC-07 | A/B/C hatékonyságmérés | MARAD | az „A” bázis a jelenlegi felhős folyamat |
| 8.6 | egy worktree-ben legfeljebb egy író ügynök | MARAD | felhőben triviálisan teljesül (5. pont) |
| 8.13 Conda/mamba tilos | – | MARAD | – |
| 8.15, TC-15 | helyreállítás megszakításkor | ÁTÉRTELMEZ | a konténer elveszhet; a helyreállítható állapot a **pusholt ág + PR + szál** |
| `CLAUDE.md`: „worktree + `CARGO_TARGET_DIR` párhuzamos ügynököknek” | egy gép, több ügynök | MARAD, szűkebb hatókörrel | csak **egy munkameneten belüli** alügynökökre vonatkozik (5.3) |

## 5. TC-02 Workspace Coordinator felhős környezetben

### 5.1 Javasolt alapszabály

A FINAL 1.0 szerint: „Egy aktív író munkamenethez egy kizárólagosan írható Git worktree tartozik.”
Felhős megfelelője (javaslat, D-C3):

> **Egy aktív író munkamenet = egy konténer = egy ág = legfeljebb egy nyitott PR.**
> Munkamenet más munkamenet ágára nem pushol (`coordination.md` §2.4), és a `main`-be nem olvaszt
> (§1). Munkamenetenként egyszerre egy író folyamat van; alügynök csak saját worktree-ben ír (5.3).

Az elkülönítés így erősebb, mint egy worktree-é: külön fájlrendszer, folyamat, port, `target/` és
`.venv`. A worktree-alapú zárolás szükségtelen; **az ütközés a git-/PR-szintre tolódik**.

### 5.2 A TC-02 nyolc feladata

| # | FINAL 1.0 feladat | Felhős megfelelő | Állapot |
|---|---|---|---|
| 1 | feladatonként külön worktree | munkamenetenként külön konténer és klón | VERIFIED (a környezet adja) |
| 2 | külön branch | munkamenetenként kijelölt `claude/…` ág | VERIFIED (a környezet adja) |
| 3 | munkaterület-tulajdon nyilvántartása | ág ↔ munkamenet ↔ PR ↔ projektszál: a PR-leírás attribúciós sora és munkamenet-linkje rögzíti | részben: implicit, **nincs lekérdezhető nyilvántartás** – GAP-C1 |
| 4 | párhuzamos írási konfliktus blokkolása | fájlszinten nem lehetséges; két PR ugyanazt a fájlt módosíthatja | **GAP-C2**: nincs korai jelzés; ma a gazda a beolvasztáskor veszi észre |
| 5 | olvasási jogú review | csak olvasó alügynökök (`.claude/agents/cfa4-ci-runner.md`, `cfa4-branch-lister.md`) | részben (dokumentált, nem kikényszerített) |
| 6 | megszakított munkamenet helyreállítása | ami nincs pusholva, a konténerrel elvész | **GAP-C3**: nincs „korán és gyakran pusholj” szabály |
| 7 | árva zárolások | nincsenek zárak; megfelelője az árva ág / gazdátlan PR (`coordination.md` §2.7, §4) | részben: a gazda kérésre listázza |
| 8 | biztonságos ügynökátadás | ág + PR + szál; egy másik munkamenet a PR-ból folytathat | **GAP-C4**: nincs strukturált átadási csomag (F6) |

### 5.3 Munkameneten belüli párhuzamosság

Egy munkamenet futtathat párhuzamos alügynököket. Itt a FINAL 1.0 eredeti TC-02 szabálya
**szó szerint érvényes**: alügynök csak saját worktree-ben írhat (a Claude Code alügynök-izolációja,
`isolation: "worktree"`), saját `CARGO_TARGET_DIR`-rel (`CLAUDE.md`). A Worktrunk ide sem kell, mert a
Claude Code natívan kezeli (WT-F0, #642). Megjegyzés: a 4 vCPU / 15 GiB-os konténerben több
párhuzamos `cargo` build erőforrás-korlátba ütközhet; mérés nem történt.

### 5.4 Javasolt teendők a hiányokra (F3-ban, külön engedéllyel)

| Gap | Javaslat | Kikényszerítés |
|---|---|---|
| GAP-C1 | a PR-leírás kötelező mezői: feladat-ID, projekt (terv/issue), munkamenet-link | a gazda állapotfelmérése ellenőrzi |
| GAP-C2 | átfedésjelzés: a nyitott PR-ok módosított fájljainak metszete, különösen a §2.5 közös fájlokra | csak olvasó ellenőrzés (gazda vagy fordításmentes workflow); **nem** blokkol, jelez |
| GAP-C3 | szabály: minden zöld lépés után commit és push; a munkamenet végén nem maradhat pusholatlan munka | `docs/agents.md` §5 kiegészítése |
| GAP-C4 | átadási sablon a PR-leírásban (állapot, nyitott blokkolók, utolsó gate-eredmény, következő lépés) | F6 |

## 6. Frissített gap-tábla (csak a v2-höz képest változott sorok)

| TC / AC | v2 | v3 |
|---|---|---|
| TC-01 Agent Policy Adapter | csak szöveges; belső ütközés | ütközés részben rendezve (projekthatár); a Pi-oldali paritás D-C2-től függ |
| TC-02 Workspace Coordinator | nincs zárolás, nyilvántartás | felhőben az izoláció adott (5.2/1–2); maradék: GAP-C1–C4 |
| TC-05 Approval & Gate Controller | BLOCKED (0 jóváhagyás, teljes CI nem kötelező) | a teljes CI kötelező (DOCUMENTED); a kötelező jóváhagyások száma nem ellenőrzött újra – PENDING |
| TC-14 Security & Tool Execution Guard | ügynökszintű allowlist nincs | új felület: a felhős környezet hálózati házirendje, titkai és GitHub-jogosultságai – felmérendő (F1) |
| TC-15 Recovery & Idempotency | nincs szabály | konkrét kockázat: ideiglenes konténer → GAP-C3 |
| AC-01 Platform | NOT RUN | javaslat: AC-01a (R1: Claude Code a felhős környezetben), AC-01b (R2: 26.04 CI), AC-01c (R3: csak ha D-C2 helyi eszközt tart meg) |
| AC-04 Koordináció | NOT RUN | javaslat: ág/PR-tulajdon + átfedésjelzés + munkameneten belüli worktree-izoláció |
| AC-06 CI | BLOCKED a célgépen (HEIF) | R2-ben a HEIF-orákulumok futnak; a GPU-s és Wayland-pontok R3-hoz kötve – a v1 HEIF-blokkolója a célgépre vonatkozik, nem a kapura |

## 7. Tulajdonosi döntések, amelyek a továbblépéshez kellenek

| ID | Kérdés | Javaslat |
|---|---|---|
| D-C1 | A FINAL 1.0 8.1 („Kizárólag Ubuntu 26.04”) a fejlesztési környezetre is vonatkozik, vagy csak a termékre és a CI-kapura? | csak a termékre és a kapura (R2); a munkamenet-konténer OS-e nem célplatform |
| D-C2 | Marad-e a ThinkRail és a Pi + Claude a tervben? | a felhős folyamatból kikerülnek; a terv helyi ágként, „szünetel” állapotban megtartja őket (mint a Worktrunknál: helyi munkára költözéskor újranyitható) |
| D-C3 | Elfogadható-e az 5.1 felhős TC-02 alapszabály? | igen |
| D-C4 | Az AC-01 / AC-04 / AC-06 felosztása R1/R2/R3 szerint (6. pont)? | igen |
| D-C5 | Hol fussanak az R3-pontok (GPU, Wayland/Qt6 GUI, minimumgép)? | a referenciagépen, alkalmanként Remote Controllal; önkiszolgáló futó csak külön döntéssel |

A döntéseket a FINAL 1.2 kiegészítés rögzíti (7a).

## 7a. Tulajdonosi döntések (2026-11-06)

A tulajdonos mind az öt javaslatot elfogadta: D-C1, D-C2, D-C3 és D-C4 a 7. pont szerint, a D-C5-öt
pedig úgy, hogy az R3-pontok a referenciagépen futnak. A döntéseket a
[FINAL 1.2 kiegészítés](../development/ai-orchestration/CFA4-DEV-AI-ORCHESTRATION-001-FINAL-1.2-felho.md)
rögzíti.

## 8. Biztonság, költség, visszaállítás

- Titok nem került a dokumentumba. Fizetős ügynökhívás és CI-futtatás nem történt.
- Visszaállítás: a PR két új fájlt ad hozzá és két tartalomjegyzéket bővít; revert-tel
  nyomtalanul visszavonható.

## 9. Javasolt következő hatókör (jóváhagyásra, nem engedély)

1. ~~D-C1–D-C5 eldöntése~~ – megtörtént (7a); a FINAL 1.2 elkészült.
2. **F1** felhős értelmezéssel (Claude Code
   jogosultságai, hálózati házirend, titokkezelés, GitHub-jogosultságok – TC-14), és a GAP-C3
   szabály mint legolcsóbb azonnali javítás.
