# CFA4-DEV-AI-ORCHESTRATION-001
# FINAL 1.2 – kiegészítés a felhős fejlesztési környezethez

**Verzió:** FINAL 1.2 (additív kiegészítés a FINAL 1.0 + FINAL 1.1-hez)
**Alapja:** a tulajdonos 2026-11-06-i döntései (D-C1–D-C5), az
[F0-audit v3](../../audit/AI-ORCHESTRATION-F0-v3-2026-11-06.md) 7. pontjának javaslatai szerint.
**Állapot:** a döntések elfogadva (2026-11-06); ennek a szövegnek a jóváhagyása a PR
beolvasztásával történik.

A FINAL 1.0 és 1.1 szövege nem változik. Ahol ez a kiegészítés eltér tőlük, ez az irányadó.
Ami itt nem szerepel, változatlanul érvényes. A végrehajtási engedélyek sem változnak:
**F1–F9 és E0–E6 továbbra sincs engedélyezve.**

## 1. Háttér

A CFA4 fejlesztése a GitHubon, felhős Claude Code munkamenetekben folyik (tulajdonosi közlés,
2026-11-06; Worktrunk D6, #642). Minden munkamenet saját, ideiglenes konténerben, friss klónnal és
saját ággal dolgozik, az integráció PR-on át megy. A FINAL 1.0 ezzel szemben a tulajdonos
Ubuntu 26.04-es gépét tekintette a fejlesztési rendszer helyének.

## 2. Végrehajtási rétegek (új fogalom)

| Réteg | Hol | Szerep |
|---|---|---|
| **R1 – munkamenet** | felhős Claude Code konténer | kód és dokumentáció írása, gyors ellenőrzések, PR |
| **R2 – kapu** | GitHub CI, `ubuntu-26.04` futók | a kötelező tesztek a célplatformon; `full-validation-gate` |
| **R3 – referenciagép** | a tulajdonos gépe ([`reference-machine.md`](../reference-machine.md)) | GPU (CUDA/Vulkan, NVENC/NVDEC), Wayland és Qt6 GUI, minimumgép-mérés |

## 3. Döntések és a módosított pontok

### D-C1 – célplatform (FINAL 1.0 fejléc, 8.1)

A „Kizárólag Ubuntu 26.04 LTS és variánsai, amd64” szabály a **terméket**, a **kaput (R2)** és a
**referenciagépet (R3)** köti. Az R1 munkamenet-konténer operációs rendszere nem célplatform; ott
elért eredmény nem helyettesíti az R2 vagy R3 eredményét.

### D-C2 – ThinkRail és Pi + Claude (2.2, 2.3, F4, E4, AC-02, TC-01/2)

A ThinkRail és a Pi + Claude a felhős folyamatból kikerül, állapota: **SZÜNETEL**. A rájuk
vonatkozó feladatok (F1/1–2, F4, E4, a Pi-paritás a TC-01-ben, az AC-02 Pi-ága, AC-13 Pi-része) nem
blokkolják a többi fázist. Ha a fejlesztés helyi gépre költözik, ezek a pontok új revízióval
újranyithatók. A fő ügynökútvonal a Claude Code felhős munkamenet (2.4).

### D-C3 – TC-02 Workspace Coordinator

A FINAL 1.0 alapszabálya helyett:

> **Egy aktív író munkamenet = egy konténer = egy ág = legfeljebb egy nyitott PR.**
> Munkamenet más munkamenet ágára nem pushol, és a `main`-be nem olvaszt (`docs/coordination.md`
> §1, §2.4). Munkameneten belül a párhuzamos alügynökök csak saját Git worktree-ben, saját
> `CARGO_TARGET_DIR`-rel írhatnak (a FINAL 1.0 eredeti szabálya, szűkebb hatókörrel).

A TC-02 feladatai ennek megfelelően:

| # | Feladat (FINAL 1.0) | Új tartalom |
|---|---|---|
| 1–2 | külön worktree, külön branch | munkamenetenként külön konténer és ág (a környezet adja) |
| 3 | tulajdon-nyilvántartás | a PR-leírás kötelező mezői: feladat-ID, projekt, munkamenet-link (GAP-C1) |
| 4 | párhuzamos írás blokkolása | a nyitott PR-ok fájlátfedésének jelzése, különösen a `coordination.md` §2.5 közös fájljaira; jelez, nem blokkol (GAP-C2) |
| 5 | olvasási jogú review | csak olvasó alügynökök (`.claude/agents/`) |
| 6 | helyreállítás | a helyreállítható állapot a pusholt ág + PR + szál; lásd 8.15 (GAP-C3) |
| 7 | árva zárolások | árva ágak és gazdátlan PR-ok (`coordination.md` §2.7, §4) |
| 8 | ügynökátadás | átadási blokk a PR-leírásban (F6, GAP-C4) |

Az F3/3–4 („worktree-zárak”, „párhuzamos írás blokkolása”) és az F8/1 („több elkülönített
worktree”) ennek megfelelően értelmezendő.

### D-C4 – elfogadási kapuk

| AC | Új tartalom |
|---|---|
| AC-01 | **AC-01a** (R1): Claude Code, Git/GitHub-integráció, jogosultságok és hálózati házirend a felhős környezetben. **AC-01b** (R2): a kötelező CI `ubuntu-26.04`-en fut. **AC-01c** (R3): csak a referenciagépen futó pontokra. A „Host Non-Interference” felhőben a tároló és a GitHub-állapot (ágak, PR-ok, issue-k, ruleset) sértetlenségét jelenti, R3-ban a referenciagépét. |
| AC-04 | ág- és PR-tulajdon a D-C3 szerint, fájlátfedés-jelzés, munkameneten belüli worktree-izoláció, átadási blokk. |
| AC-06 | az R2 teljesíti a Rust/Cargo-, FFmpeg 9.0.2-, FFmpeg-orákulum-, HEIF/HEIC-, J2K-, VFR-, import/export- és kodekpontokat; **az R3 teljesíti** a CUDA/Vulkan-, NVENC/NVDEC-, Wayland-, Qt6 GUI- és natív média E2E-pontokat. A SKIPPED, BLOCKED és NOT_RUN továbbra sem siker. |

### D-C5 – az R3-pontok helye

Az R3-pontok **a tulajdonos referenciagépén** futnak, alkalmanként, a tulajdonos engedélyével
(Remote Control-munkamenet). Önkiszolgáló GitHub-futó csak külön döntéssel. Az R3-mérések
jegyzőkönyve a `reference-machine.md` mérési szabályait követi.

## 4. Új kötelező szabály (8.15 kiegészítése)

**8.15a** Felhős munkamenetben a konténer bármikor megszűnhet. Minden zöld lépés után commit és
push következik; munkamenet nem fejeződhet be pusholatlan munkával. (GAP-C3)

## 5. Változatlan pontok

Változatlanul érvényes többek között: 8.2–8.5, 8.7–8.11, 8.13–8.14, 8.16, a TC-03–TC-15 célja,
az AC-03, AC-05, AC-07–AC-14, a fázissorrend (FINAL 1.0, 6.) és az AIQT 1.0.6 alkalmazása a
FINAL 1.1 szerint.

## 6. Következő lépés (nem engedély)

F1 felhős értelmezéssel (AC-01a): Claude Code jogosultságai, hálózati házirend, titokkezelés,
GitHub-jogosultságok (TC-14). Csak külön, kifejezett engedéllyel indul.
