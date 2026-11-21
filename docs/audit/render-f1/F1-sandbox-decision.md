# F1: sandbox-döntés (rootless) – a meglévő `cfa4-sandbox` átvétele

**Szabály:** a CFA4-SECURITY-001 B1 fázisa a `crates/sandbox` crate-et már megvalósította és a D3 döntés rögzítette
(„Landlock és seccomp a saját Rust kódban”, 2026-11-11). Ezt **nem döntjük újra**; a render-terv ezt veszi át, és csak azt
mondja meg, milyen profil kell a render-workereknek.

## Döntés

1. **Technológia: `cfa4-sandbox` (Landlock + seccomp-bpf)**, a worker induláskor maga zárja le magát. Gazdagép-módosítás
   nem kell: nincs `sudo`, nincs felhasználói névtér, az AppArmor `userns` korlátozás nem érinti (azt csak a Chromium/Qt WebEngine
   használja, SECURITY-001 B6).
2. **Erőforrás-korlát:** `systemd-run --user --scope -p MemoryMax= -p CPUQuota=` (cgroup v2, rootless) a workerek indításakor; a
   `systemd-run` jelen van a vizsgált rendszeren, a referenciagépen PENDING.
3. **Profilok:** a meglévő `mediaHost` (csak a megnyitott fájl olvasása, hálózat és `execve` nélkül) a médiafeldolgozó workerekre
   illik. A VapourSynth worker a `pluginHost`-hoz hasonlít (olvassa a bővítményeket, nem ír, nincs hálózat), de kell neki az
   interpreter és a szkriptek olvasása és a csöveken át adott kimenet. A Blenderhez írási jog kell a kimeneti mappára és a saját
   indítása. **Javaslat a SECURITY-001 felelősének:** két új folyamatfajta (`renderWorker`, `externalRender`); a crate-et ez a PR nem módosítja.
4. **Állapot:** a sandbox még egyik folyamatban sincs alkalmazva (B3/B4 függőben); amíg nincs, a render-worker POC csak
   kísérleti, nem szállított útvonal.

## Átfedés és határ

A döntés a SECURITY-001-et érinti, nem a render-tervet: új profilt csak annak felelőse vesz fel (`docs/coordination.md` §2.8).
Szükséges mérés a referenciagépen: Landlock ABI-szint (a D4 szerint ≥ 6 az Ubuntu 26.04-en), és hogy a `systemd-run --user`
cgroup-korlát a munkamenetben működik.

## Kilépési feltétel

„A sandbox-döntés gazdagép-módosítás nélkül megvalósítható”: **teljesül** a fenti terv szerint (a referenciagépes ellenőrzés PENDING).
