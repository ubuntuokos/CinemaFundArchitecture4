# CFA4-DEV-WORKTRUNK-001

## Worktrunk-alapú párhuzamos fejlesztési infrastruktúra

**Dokumentumverzió:** 1.1 (az 1.0-s vázlat kidolgozása)
**Státusz:** JÓVÁHAGYVA – a tulajdonos (`ubuntuokos`) 2026-11-05-én jóváhagyta. A jóváhagyás nem jelent telepítési, fázisindítási vagy merge-engedélyt; minden fázis külön engedéllyel indul.
**Projekt:** CinemaFundArchitecture4 (CFA4)
**Szülőterv:** CFA4-DEV-AI-ORCHESTRATION-001, TC-02 „Workspace Coordinator”
**Végrehajtó:** Claude Code CLI
**Koordináció:** ThinkRail Pi + Claude Code CLI + Elephant-Goldfish
**Célgép:** a tulajdonos referenciagépe, Ubuntu Studio 26.04 / amd64 (`docs/development/reference-machine.md`)
**Fájlrendszer:** ext4
**Eszköz:** [Worktrunk](https://github.com/max-sixty/worktrunk)

Változások az 1.0-hoz képest a dokumentum végén (11. fejezet).

---

# 1. A feladat célja

A Worktrunk bevezethetőségének vizsgálata, majd sikeres validáció esetén beillesztése a CFA4 fejlesztési munkafolyamatába, a CFA4-DEV-AI-ORCHESTRATION-001 TC-02 komponensének eszközeként.

A Worktrunk a párhuzamos munkákhoz szükséges Git worktree-ket kezeli. Több Claude Code munkamenet dolgozhat így egymástól elkülönített munkaterületen, a feladatkiosztás és a koordináció a ThinkRail Pi dolga marad.

A Worktrunk nem része a CFA4 terméknek, és nem lesz futásidejű függősége. A hagyományos CFA4-fejlesztésnek nélküle is működnie kell (AI-ORCHESTRATION FINAL 1.0, 8.16).

## 1.1 Elsőbbségi sorrend

1. `VISION.md`, `AGENTS.md`, `docs/coordination.md`, `CLAUDE.md`.
2. CFA4-DEV-AI-ORCHESTRATION-001 (FINAL 1.0 + 1.1).
3. Ez a dokumentum.

Ellentmondás esetén a magasabb szint érvényes.

## 1.2 Összevetési alap

A bevezetés akkor indokolt, ha a Worktrunk mérhetően jobb a meglévő megoldásnál: sima `git worktree` és a Claude Code natív worktree-kezelése (a `.claude/worktrees/` már szerepel a `.gitignore`-ban). Az F1-ben ezt az alapvonalat is értékelni kell (WT-AC-14).

---

# 2. CFA4 környezet – rögzített tények

Forrás: `main` @ `10fb82c`, 2026-11-05. A gépfüggő értékeket az F0 a célgépen rögzíti.

| Terület | Tényleges állapot | Forrás |
|---|---|---|
| **Rust toolchain** | **1.97.1** (profile minimal, rustfmt, clippy) | `rust-toolchain.toml` (TOOL-1, `196ad83`, 2026-10-25) |
| Workspace MSRV | `rust-version = "1.97"` | `Cargo.toml` |
| Toolchain a célgépen | 1.97.1, a tulajdonos döntése: marad | `docs/audit/AI-ORCHESTRATION-F0-2026-10-30.md`, F0-01 |
| Cargo workspace | 1 db: `crates/*` (63), `apps/*` (cfa4, cfa4-cli), `xtask`; resolver 3 | `Cargo.toml` |
| Python | ≥3.12, `.venv` a repo gyökerében, `requirements/linux-amd64.txt` (`--require-hashes`), `pip install -e .` | `pyproject.toml`, `docs/development/linux.md` |
| Qt6 | PySide6 ≥6.10, Pythonból | `pyproject.toml` |
| C/C++ | nincs saját C/C++ build | repo |
| npm / yarn | nincs projekt | repo |
| PostgreSQL | elfogadott döntés (BacklotHub, saját klaszter), kód még nincs | `docs/adr/ADR-0006-backlothub-postgresql.md` |
| Integráció | GitHub PR; beolvasztás csak a gazda, a tulajdonos hívására | `docs/coordination.md` §2 |
| Worktree-kezelés ma | `CARGO_TARGET_DIR=target/agent-<név>`, Claude Code worktree | `CLAUDE.md`, `.gitignore` |

A korábbi 1.95.0-s érték a 2026-10-25 előtti állapot (lásd a régebbi auditokat). Ma a `rust-toolchain.toml` a mérvadó. A terv minden fázisa onnan olvassa ki az aktuális értéket, és nem írja be keményen.

**A CFA4 toolchain a Worktrunk kedvéért nem változhat.** Új toolchain telepítése vagy frissítése csak külön tulajdonosi jóváhagyással történhet.

---

# 3. Alaparchitektúra

1. A ThinkRail Pi meghatározza és kiosztja a feladatot.
2. A Worktrunk a friss `main`-ből létrehozza a feladat worktree-jét és ágát (`wt switch -c <ág>`).
3. A Claude Code a kijelölt worktree-ben dolgozik.
4. Lefutnak a CFA4 minőségi kapui (`cargo xtask ci`, `cargo test --workspace`, Python-ellenőrzések).
5. Diff-ellenőrzés és külön review (EGM).
6. Push és PR a GitHubon. **A beolvasztás a gazda dolga**, nem a Worktrunké és nem az agenté.
7. A worktree eltávolítása csak külön jóváhagyással, a PR beolvadása után.

A Worktrunk kizárólag munkaterületeket kezel, a ThinkRail Pi koordinációját nem helyettesíti.

---

# 4. Kötelező biztonsági szabályok

## 4.1 Git

Tilos:

- automatikus merge, bármilyen helyi beolvasztás a `main`-be;
- `git reset --hard`, `git clean -fd`, `git clean -fdx`;
- `git checkout -- .`, `git restore .`, `git stash drop`, `git stash clear`;
- `git branch -D`, `git worktree remove --force`, `git worktree prune`;
- force push más ágára, illetve bármely megosztott ágra;
- felhasználói vagy más worktree módosításainak felülírása;
- CFA4-en kívüli repository módosítása.

## 4.2 Worktrunk

Agentnek tilos:

- `wt merge` bármilyen kapcsolóval (helyben squash-ol, rebase-el, a `main`-be fast-forwardol, majd törli a worktree-t és az ágat);
- `wt remove` és `wt step prune` (worktree- és ágtörlés);
- `wt step squash`, `wt step rebase` (történetátírás);
- `--yes` bármely parancson, `wt config approvals add` (jóváhagyás megkerülése);
- `wt step copy-ignored` `--require-include` nélkül.

Kötelező felhasználói beállítás: `[remove] delete-branch = false`.

## 4.3 Hookok

- Felhasználói hook (`~/.config/worktrunk/config.toml`) nem lehet beállítva: jóváhagyás nélkül futna.
- Projekthook (`.config/wt.toml`) csak a repóban, PR-ral és review-val kerülhet be. A jóváhagyásokat (`~/.config/worktrunk/approvals.toml`) csak a tulajdonos adja meg.
- Agent-futtatásnál a `--no-hooks` az alapértelmezés, amíg a WT-AC-11 nem PASS.
- LLM-alapú commitüzenet (`[commit.generation]`) nincs beállítva.

## 4.4 Eszközök és telepítés

- Rust, Cargo, Git vagy más fejlesztői komponens frissítése külön jóváhagyás nélkül tilos.
- Modell letöltése és automatikus telepítése tilos.
- A Worktrunk Claude Code pluginja az F5 előtt nem telepíthető. Telepítés előtt a `docs/development/skillspector.md` szerinti ellenőrzés kötelező.
- A feladatban nem szereplő CFA4-módosítás tilos.

## 4.5 Izoláció

A Git worktree nem biztonsági sandbox. A közös fájlrendszer, a portok, a GPU, az adatbázisok és a külső szolgáltatások külön vizsgálatot igényelnek (5. fejezet, F4).

---

# 5. Végrehajtási fázisok

Minden fázis végén jelentés készül, a következő fázis csak külön tulajdonosi engedéllyel indul. Az F0 a célgépen fut; a felhős klónban végzett vizsgálat csak előkészítés, nem F0-bizonyíték.

**Engedélyezési feltétel:** a szülőterv (AI-ORCHESTRATION) F1–F9 fázisai jelenleg nincsenek engedélyezve. Az F0 (csak olvasás) ettől függetlenül futhat. Az F1-től **a szülőterv megfelelő fázisának engedélye is kell** (tulajdonosi döntés, D2b, 2026-11-05): a Worktrunk F1–F5 csak akkor indulhat, ha ennek a tervnek a fázisengedélye és az AI-ORCHESTRATION TC-02-höz tartozó fázisának engedélye is megvan.

## F0 – CFA4 repository és célgép felmérése

**Csak olvasás.** Fájlmódosítás, telepítés, konfigurációváltoztatás nem engedélyezett.

| # | Pont | Parancs a célgépen |
|---|---|---|
| 1 | Repo gyökér | `git rev-parse --show-toplevel` |
| 2 | Git verzió | `git --version` |
| 3 | Ág, commit | `git status -sb`, `git log -1 --format='%H %ad'` |
| 4 | Working tree | `git status --porcelain=v2` |
| 5 | Meglévő worktree-k | `git worktree list --porcelain`, `ls .claude/worktrees` |
| 6 | Rust | `rustup show`, `rustc -V`, `cargo -V`; összevetés a `rust-toolchain.toml` értékével |
| 7 | Cargo workspace | `cargo metadata --no-deps --format-version 1` (tagok száma) |
| 8 | Python | `python3 -V`, `.venv/bin/python -V`, `.venv/bin/pip list` |
| 9 | Qt6 | `.venv/bin/python -c "import PySide6; print(PySide6.__version__)"` |
| 10 | Hookok | `git config --show-origin --get-all core.hooksPath`, `ls .git/hooks` (nem `.sample`), `~/.claude/settings.json` |
| 11 | Worktrunk nyomai | `command -v wt`, `ls ~/.config/worktrunk`, `grep -n "wt " ~/.bashrc ~/.zshrc` |
| 12 | Lemez | `df -h` a repo és a tervezett worktree-hely partícióján, `du -sh target .venv` |
| 13 | Portok | `ss -ltn` (különösen 9876) |
| 14 | Fájlrendszer | `findmnt -T .` (ext4 megerősítése) |

**Elfogadás:** teljes, ellenőrizhető jelentés a `docs/audit/` alatt, a parancsok kimenetével.

## F1 – Worktrunk kompatibilitási vizsgálat

Ellenőrizendő:

- kiválasztott verzió és telepítési út (D1);
- a verzió `rust-version` értéke a CFA4 1.97.1-es toolchainjéhez képest;
- Linux amd64 bináris, ellenőrzőösszeg, kiadási hitelesítés;
- Git-minimumverzió a célgép Git-verziójához képest;
- shellintegráció hatása (`wt config shell install` a shell rc fájlba ír);
- összevetés a sima `git worktree` + Claude Code natív worktree alapvonallal (WT-AC-14).

Ismert adatok (crates.io, 2026-11-05):

| Worktrunk | Kiadva | `rust-version` | Viszony a CFA4 1.97.1-hez |
|---|---|---|---|
| 0.81.0 | 2026-10-20 | 1.98 | forrásból nem fordítható a CFA4 toolchainnel |
| 0.80.0 | 2026-09-27 | 1.97 | forrásból fordítható |

Telepítési utak (D1):

- **A – javasolt:** előre fordított 0.81.0 Linux amd64 bináris, rögzített verzióval, ellenőrzőösszeggel. A Rust-verziótól független.
- **B:** `cargo install worktrunk --version 0.80.0 --locked`, a CFA4 könyvtárán kívül, a meglévő toolchainnel.
- **C:** külön 1.98-as toolchain. Új toolchain telepítése, csak külön jóváhagyással.

Megjegyzés: a `rust-toolchain.toml` könyvtárhoz kötött. Ha a `cargo install` a CFA4 könyvtárában fut, az 1.97.1-gyel fordít.

Ha egyik út sem kompatibilis a meglévő környezettel: **BLOCKED**.

## F2 – Worktree-kezelés ext4-en

Vizsgálandó:

- létrehozás, listázás, váltás; ágak és fájlmódosítások elkülönülése;
- nem követett fájlok kezelése;
- hideg build idő- és helyköltsége egy új worktree-ben (`cargo build`, `.venv` felépítése);
- a `copy-ignored` viselkedése `--require-include`-dal, `.worktreeinclude` nélkül (elvárt: no-op).

Rögzített tény, nem kell mérni: ext4-en nincs reflink, a copy-on-write cache csak APFS-en, btrfs-en és XFS-en működik. A `target/` és a `.venv/` nem másolható (a `.venv` abszolút útvonalai miatt el is romlana). Ha a hely vagy az idő nem elfogadható, tulajdonosi döntés: külön XFS/btrfs kötet (D5).

## F3 – Claude Code integráció

Plugin nélkül, `--no-hooks`-szal, külön kijelölt próbamunkával. Ellenőrizendő, hogy a Claude Code:

- a kijelölt worktree-ben indul, és indulás előtt ellenőrzi a könyvtárat és az ágat;
- nem módosítja a fő munkakönyvtárat és más worktree-t;
- a saját worktree ágán lévő `.claude/` skilleket és szabályokat használja;
- a végén pontos módosítási jelentést készít.

## F4 – Párhuzamos agentteszt

Két worktree:

- **A:** kis kockázatú Rust-ellenőrzés (pl. `cargo clippy -p <crate>`).
- **B:** Python- vagy dokumentációs ellenőrzés (`ruff`, `pytest`).

Vizsgálandó:

| Erőforrás | Elvárt szabály |
|---|---|
| Git | külön ág, külön index |
| Cargo | közös `CARGO_HOME`, worktree-nként külön `target/` |
| Python | worktree-nként külön `.venv` |
| Vezérlőport | worktree-nként külön `CFA4_CONTROL_PORT` |
| GPU, teljesítmény | párhuzamos futás alatt nincs benchmark (`reference-machine.md`) |
| Fixture-ök | közös `CFA4_FIXTURES`, csak olvasásra |
| PostgreSQL | NOT_APPLICABLE, amíg a BacklotHub adatrétege nincs kész; utána worktree-nként külön adatbázis |
| npm / yarn / C++ | NOT_APPLICABLE |
| Megszakadt munkamenet | a worktree, az ág és a nem commitolt munka megmarad; a `wt list` beragadt jelölője kézzel törölhető |

Sikeres kétagentteszt után vizsgálható a három vagy több párhuzamos worktree.

## F5 – Beillesztés a CFA4 folyamatba

Részletes bevezetési javaslat, és PR-ral, review után:

- a 4.2 tiltások rögzítése Claude Code engedélyszabályként (deny) a `.claude/settings.json`-ban;
- esetleges projekthook (`.config/wt.toml`) csak ellenőrzött parancsokkal;
- a `docs/development/` alá rövid üzemeltetési útmutató.

A végleges munkafolyamat a 3. fejezet szerinti. Automatikus merge és törlés minden esetben tilos.

---

# 6. Technológiai együttműködés

| Technológia | Követelmény | Állapot |
|---|---|---|
| Rust/Cargo (1.97.1) | worktree-nként külön `target/`, közös registry | releváns |
| Python | worktree-nként külön `.venv` | releváns |
| Qt6 (PySide6) | a Python-környezettel együtt | releváns |
| C/C++ | – | NOT_APPLICABLE |
| npm / yarn | – | NOT_APPLICABLE |
| PostgreSQL | worktree-nként külön adatbázis, migráció csak a sajátra | később |
| Claude Code | worktree-hez kötött munkamenet | releváns |

Közös cache csak tesztelt és biztonságos esetben használható.

---

# 7. Elfogadási kritériumok

| Kód | Kritérium |
|---|---|
| WT-AC-01 | Ubuntu 26.04 / amd64 kompatibilitás |
| WT-AC-02 | Kompatibilitás a CFA4 tényleges Rust-toolchainjével (1.97.1), a toolchain változtatása nélkül |
| WT-AC-03 | Git-kompatibilitás |
| WT-AC-04 | ext4 worktree-kezelés |
| WT-AC-05 | Claude Code helyes munkaterület-használata |
| WT-AC-06 | Párhuzamos agentek elkülönülése |
| WT-AC-07 | Cargo- és Python-környezet kezelése |
| WT-AC-08 | Közös erőforrások ütközéskezelése (port, GPU, fixture, később adatbázis) |
| WT-AC-09 | Megszakadt munkafolyamat helyreállítása |
| WT-AC-10 | Automatikus merge és törlés hiánya, beállítással és engedélyszabállyal kikényszerítve |
| WT-AC-11 | Hookbiztonság |
| WT-AC-12 | Visszaállítási eljárás |
| WT-AC-13 | Worktree-nkénti lemezhely- és időköltség elfogadható |
| WT-AC-14 | Mérhető előny a sima `git worktree` + Claude Code alapvonalhoz képest |
| WT-AC-15 | Harmadik féltől származó plugin vagy hook ellenőrzése |

Státuszok: PASS, FAIL, BLOCKED, NOT_TESTED, NOT_APPLICABLE. Nem tesztelt kritérium nem minősíthető teljesítettnek.

---

# 8. Visszaállítási terv

1. Worktrunk használatának felfüggesztése.
2. A fő munkakönyvtár ellenőrzése (`git status`).
3. Worktree-k listázása (`git worktree list`).
4. Módosítások és nem követett fájlok megőrzése, nem integrált munka push-olása saját ágra.
5. Claude Code munkamenetek állapotának rögzítése.
6. Visszatérés a korábbi munkafolyamatra: a worktree-k sima Git-worktree-k, Worktrunk nélkül is kezelhetők.
7. A Worktrunk nyomai, csak külön jóváhagyással eltávolítva: shell-rc sor, `~/.config/worktrunk/` (config, approvals), Claude-plugin (`wt config plugins claude uninstall`), állapotjelölők (`wt config state marker clear`), maga a `wt` bináris.

Destruktív Git-parancs nem használható.

---

# 9. Claude Code végrehajtási utasítás

Kötelező sorrend: F0 → F1 → F2 → F3 → F4 → F5. Egyik fázis sem ugorható át.

Minden fázis végén:

1. részletes jelentés a `docs/audit/` alatt, PR-ban;
2. a végrehajtott ellenőrzések, parancsok és kimenetek;
3. blokkoló tényezők;
4. az elfogadási kritériumok értékelése;
5. megállás. A következő fázis csak külön tulajdonosi engedéllyel kezdhető.

---

# 10. Első indítási prompt (a célgépen)

„Kezdd meg a CFA4-DEV-WORKTRUNK-001 1.1 terv F0 fázisát a célgépen. Kizárólag olvasási műveleteket végezz, az F0 táblázatának parancsaival. Ne telepíts, ne módosíts, ne törölj, ne hozz létre worktree-t. A Rust-verziót a `rust-toolchain.toml`-ból és a `rustup show` kimenetéből rögzítsd. Készíts teljes, ellenőrizhető jelentést, és a végén állj meg. Ne kezdd el az F1 fázist.”

---

# 11. Változások az 1.0-s vázlathoz képest

| Pont | 1.0 | 1.1 |
|---|---|---|
| Rust-verzió | 1.95.0 | 1.97.1, a `rust-toolchain.toml`-ból |
| Hovatartozás | önálló | az AI-ORCHESTRATION TC-02 eszköze (tulajdonosi döntés, 2026-11-05) |
| Integráció | „integrálható a fő ágba” | PR, a beolvasztás a gazda dolga |
| Worktrunk-tiltások | csak automatikus merge és törlés | `wt merge`, `wt remove`, `wt step prune/squash/rebase`, `--yes` |
| Hookok | általános szabály | a felhasználói hook tilos, `--no-hooks` alapértelmezés |
| F2 reflink | mérendő | ismert: ext4-en nincs; helyette hidegbuild-költség |
| npm, yarn, C++ | vizsgálandó | NOT_APPLICABLE |
| PostgreSQL | vizsgálandó | később, a BacklotHub adatrétegével |
| Új AC | – | WT-AC-13, -14, -15 |
| F0 | általános lista | parancsokkal, a célgépen |

## Nyitott tulajdonosi döntések

- **D1:** telepítési út (A javasolt).
- **D2b:** ~~elég-e e terv fázisengedélye?~~ Eldöntve 2026-11-05: az F1-től a szülőterv (AI-ORCHESTRATION) megfelelő fázisának engedélye is kell.
- **D3:** a 4.2 tiltások elfogadása (javasolt: igen).
- **D4:** Claude Code plugin csak az F5 után, ellenőrzéssel (javasolt: igen).
- **D5:** külön XFS/btrfs kötet a worktree-knek, ha az F2 költsége nem elfogadható.
