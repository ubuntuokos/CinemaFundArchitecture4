# CFA4-DEV-WORKTRUNK-001 – Worktrunk-alapú párhuzamos fejlesztés

**Állapot:** LEZÁRVA – NOT_APPLICABLE (a tulajdonos döntése, 2026-11-06). Az 1.1-et a tulajdonos
2026-11-05-én jóváhagyta. Az F0 a felhős környezetben futott le
([`WT-F0-2026-11-06.md`](../../audit/worktrunk/WT-F0-2026-11-06.md)), az F1–F5 nem indul.

**Indoklás:** a terv abból indult ki, hogy több agent dolgozik egy gépen, egy közös klónban. A CFA4
fejlesztése viszont GitHubon, felhős Claude Code munkamenetekben folyik. Minden munkamenet saját,
ideiglenes konténerben, saját klónnal és saját ággal dolgozik, és PR-on át integrál. Ez erősebb
elkülönítés, mint egy worktree-é, a `wt` telepítése és beállítása pedig minden munkamenetben
elveszne. Ha a fejlesztés később helyi gépre költözik, a terv új revízióval újranyitható.

A terv a CFA4 **fejlesztésének** eszközéről szól, nem a termékről. A
[CFA4-DEV-AI-ORCHESTRATION-001](../ai-orchestration/README.md) TC-02 „Workspace Coordinator”
komponensének eszköze (tulajdonosi döntés, 2026-11-05: az AI-ORCHESTRATION a CFA4 része, az
AI-val automatizált fejlesztői eszköztárhoz tartozik).

## Dokumentumok

| Fájl | Tartalom | SHA-256 |
|---|---|---|
| [`CFA4-DEV-WORKTRUNK-001-1.1.md`](CFA4-DEV-WORKTRUNK-001-1.1.md) | 1.1 jóváhagyott terv: F0–F5, WT-AC-01–15, biztonsági szabályok, visszaállítás | `ce9ad723…3c609ecd` |

## Forrás (provenance)

- **1.0:** a tulajdonos 2026-11-05-én küldte be vázlatként („csak vázlat volt a tényleges
  kidolgozáshoz”). Nem kerül a tárolóba; az 1.1 11. fejezete sorolja fel az eltéréseket.
- **1.1:** az 1.0 kidolgozása a `main` @ `10fb82c` állapota és a Worktrunk dokumentációja
  (0.81.0) alapján, csak olvasási vizsgálattal. A tulajdonos 2026-11-05-én jóváhagyta.

## Rögzített tények és döntések

- A tervben a CFA4 **tényleges** Rust-verziója szerepel: **1.97.1** (`rust-toolchain.toml`,
  TOOL-1, 2026-10-25). A korábbi 1.95.0 elavult. Minden fázis a `rust-toolchain.toml`-ból olvassa
  ki az aktuális értéket. A CFA4 toolchain a Worktrunk miatt nem változik.
- A Worktrunk 0.81.0 `rust-version` értéke 1.98, a 0.80.0-é 1.97 (crates.io, 2026-11-05).
- Elfogadott szabályok (a terv 4. fejezete): `wt merge`, `wt remove`, `wt step prune/squash/rebase`,
  `--yes` agentnek tilos; felhasználói hook tilos; a Claude Code plugin csak az F5 után, ellenőrzéssel.
- **D2b (2026-11-05):** a Worktrunk F1–F5 csak e terv fázisengedélyével **és** az AI-ORCHESTRATION
  TC-02-höz tartozó fázisának engedélyével indulhat.

## Tulajdonosi döntések (2026-11-05)

| Kód | Kérdés | Döntés |
|---|---|---|
| D1 | Telepítési út | A – előre fordított bináris, rögzített verzióval és ellenőrzőösszeggel |
| D2 | Önálló projekt vagy TC-02? | az AI-ORCHESTRATION TC-02 eszköze |
| D2b | Kell-e az F1-hez a szülőterv fázisengedélye? | igen |
| D3 | `wt merge`, `wt remove`, `wt step prune/squash/rebase` agentnek | tilos |
| D4 | Claude Code plugin | csak az F5 után, ellenőrzéssel |
| D5 | Külön XFS/btrfs kötet | az F2 mérése alapján; addig ext4, `copy-ignored` nélkül |
| D6 (2026-11-06) | Kell-e a Worktrunk a felhős fejlesztéshez? | nem; a terv lezárva (NOT_APPLICABLE) |
