# CFA4-DEV-WORKTRUNK-001 – Worktrunk-alapú párhuzamos fejlesztés

**Állapot:** 1.1 JÓVÁHAGYVA (a tulajdonos, `ubuntuokos`, 2026-11-05). **Végrehajtás:** F0 nincs
elindítva; F1–F5 **nincs engedélyezve**. A jóváhagyás nem jelent telepítési, kódmódosítási,
merge- vagy fázisindítási engedélyt.

A terv a CFA4 **fejlesztésének** eszközéről szól, nem a termékről. A
[CFA4-DEV-AI-ORCHESTRATION-001](../ai-orchestration/README.md) TC-02 „Workspace Coordinator”
komponensének eszköze (tulajdonosi döntés, 2026-11-05: az AI-ORCHESTRATION a CFA4 része, az
AI-val automatizált fejlesztői eszköztárhoz tartozik).

## Dokumentumok

| Fájl | Tartalom | SHA-256 |
|---|---|---|
| [`CFA4-DEV-WORKTRUNK-001-1.1.md`](CFA4-DEV-WORKTRUNK-001-1.1.md) | 1.1 jóváhagyott terv: F0–F5, WT-AC-01–15, biztonsági szabályok, visszaállítás | `6eb90d85…58ff94e4` |

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

## Nyitott tulajdonosi döntések

| Kód | Kérdés | Javaslat |
|---|---|---|
| D1 | Telepítési út: A) előre fordított bináris, B) 0.80.0 forrásból a CFA4 toolchainnel, C) külön 1.98-as toolchain | A |
| D2b | Az F1-hez elég e terv fázisengedélye, vagy a szülőterv (AI-ORCHESTRATION) fázisengedélye is kell? | – |
| D5 | Külön XFS/btrfs kötet a worktree-knek, ha az ext4-es költség nem elfogadható | F2 után |
