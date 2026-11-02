# CFA4-EVENT-FABRIC-001 1.0 – CFA4-illesztési melléklet

**Dátum:** 2026-11-02 · **Állapot:** nyitott kérdések, tulajdonosi döntésre várnak · **Terv:**
[`CFA4-EVENT-FABRIC-001-1.0.md`](CFA4-EVENT-FABRIC-001-1.0.md)

Ez a melléklet **nem módosítja** a jóváhagyott tervet. Azt rögzíti, hol tér el a terv a `main`-ben ma
rögzített CFA4-szabályoktól és meglévő tervektől (`VISION.md` fejléce szerint az ellentmondást jelezni
kell, nem csendben feloldani). Ha a tulajdonos egy pontban dönt, a döntés ide kerül, és szükség esetén
a terv új verziót kap (1.1).

## 1. Eltérések

| # | Téma | A terv | A CFA4 mai állapota | Javaslat |
|---|---|---|---|---|
| E1 | HRB (13.1, 17.3, 27.2) | erőforrás-hatóság | **CFA3-elem.** A CFA4 nem függhet a CFA3 HRB-jétől ([`SUBTITLE-STUDIO-PLAN-001-FINAL-1.2.md`](SUBTITLE-STUDIO-PLAN-001-FINAL-1.2.md), 199. sor). CFA4-megfelelő: [ADR 0004](../adr/ADR-0004-compute-routing.md) útválasztó (`cfa4-route`), és a `CFA4-AI_modules` ADR-0006 erőforrás-kezelője (`cfa4-ai-jobs`) | A 13.1 és a 17.3 a CFA4-megfelelőkre hivatkozzon |
| E2 | Model Router (13.2) | AI-útválasztás | **Tervként megvan:** `CFA4-AI_modules` ADR-0006 `cfa4-ai-registry` („Előd: Model Router”) | A 13.2 a `cfa4-ai-registry`-re hivatkozzon |
| E3 | Execution Fabric (13.4) | feladatvégrehajtás | Nincs a CFA4-ben. Részben: ADR-0006 `cfa4-ai-jobs` (jobállapotok, redb-jobtár) és a motor jobsora | Megfeleltetés kell; ahol nincs CFA4-elem, előbb terv |
| E4 | Agent Coordination (13.3) | agentkoordináció | Nincs külön. CFA4-ben: MCP, vezérlőcsatorna ([`docs/agents.md`](../agents.md)), E1/E2 ügynöki terv ([`docs/open-editors-parity.md`](../open-editors-parity.md)) | Megfeleltetés kell |
| E5 | Security Governance (12.4, 13.5) | biztonsági szabályhatóság | **Nincs** a CFA4-ben | Terv kell, vagy a terv 12. pontja legyen a szabályforrás |
| E6 | Evidence Registry (12.4, 13.6) | bizonyítékok | **Nincs**; ADR-0006: „Evidence (egyszerűsítve)” = a CFA4 meglévő naplója (`events.list`) | A CFA4-naplóra és auditra kötni |
| E7 | Kiadási kapu (25.) | F1–F8 és mind a négy SDK együtt; részleges megfelelés nem fogadható el | – | Részkiadás, amelyre fogyasztó építhet (pl. EF-R1 = F1–F5 + Rust és Python SDK); a 25. pont marad a végső PASS feltétele |
| E8 | C++ és TypeScript SDK (3.5, 19.) | kötelező | A CFA4 Rust + Python + Qt6, Linux-asztali alkalmazás; webes felület nem cél, C++-komponense ma nincs | Igény szerint; amíg nincs fogyasztója, ne legyen kiadási feltétel |
| E9 | Telepítés (17.) | nats-server, valkey-server, Core, Schema Registry külön szolgáltatásként | Csomagok: `.deb`, AppImage, Flatpak, `tar.gz` ([`VISION.md`](../../VISION.md) 3.); egygépes indulás is cél | Ellenőrizni: Ubuntu 26.04 csomagok, Flatpak/AppImage-beágyazás, felhasználói systemd-szolgáltatás, erőforrás a minimális kliensgépen |
| E10 | Folyamaton belüli események (1.1) | „a komponensek nem hozhatnak létre saját eseménybuszt” | A Rust-motorban vannak folyamaton belüli események (`ProjectChanged`, `Toast`…, [`docs/architecture.md`](../architecture.md) 4.) | Pontosítás: az Event Fabric az alkalmazások és folyamatok közötti kommunikációra vonatkozik; a motor belső eseményei maradnak |
| E11 | Never crash ([`AGENTS.md`](../../AGENTS.md) §0) | fail-closed; a 20. pont csak szokásos validációs teszteket és egy hibás sémát ír | hiba = kezelhető hibaüzenet, nincs összeomlás, nincs adatvesztés; olvasókra mutációs fuzz-teszt `catch_unwind` alatt kötelező (§0.8) | A boríték-, séma- és Protobuf-olvasók mutációs fuzz-tesztje kerüljön a 20. pontba és a 25. kiadási kapuba (EF-005–EF-007 mellé) |
| E12 | Licenc ([`docs/licensing.md`](../licensing.md)) | NATS, Valkey | GPL-3.0-or-later; függőség csak GPL-3.0-kompatibilis | NATS-szerver: Apache-2.0, Valkey: BSD-3-Clause – kompatibilis; a kliens-crate-ek `cargo deny` ellenőrzése az F1 része |
| E13 | Producer-azonosság (5.1, 12.1–12.2, 4.2) | a boríték `producer` mezőjét a kibocsátó tölti ki; hitelesített NATS-kapcsolat külön követelmény | – | A `producer` és a `producer_instance` a hitelesített kapcsolat identitásából származzon, vagy eltérésnél a Gateway utasítsa el (EF-1002/EF-2002) a routing előtt; különben a jogosultság és az audit hamisítható kibocsátóban bízhat |
| E14 | Replay és idempotencia (9.1, 10.1–10.2) | idempotenciakulcs: `consumer_id + event_id`; replay projekció újraépítésére és új logika tesztjére | – | A visszajátszás külön Consumer-identitással vagy külön idempotencia-névtérrel (pl. `replay_id`) fusson, különben a korábban feldolgozott események duplikátumként kimaradnak; a 10.3 korlátozásai erre is vonatkoznak |

## 2. Nyitott kérdések

| # | Kérdés | Javaslat |
|---|---|---|
| Q1 | Mely CFA4-elemek lépnek a HRB, Execution Fabric, Agent Coordination, Security Governance és Evidence Registry helyére (E1–E6)? | ADR 0004 / ADR-0006 szerinti megfeleltetés; ahol nincs CFA4-elem, előbb terv |
| Q2 | Lehet-e részkiadás (E7), amelyre fogyasztó építhet? | Igen: EF-R1 = F1–F5 + Rust és Python SDK |
| Q3 | Kell-e most a C++ és a TypeScript SDK (E8)? | Csak ha lesz fogyasztója |
| Q4 | Egygépes módban is a teljes NATS + JetStream + Valkey fusson-e, és hogyan települjön (E9)? | Igen, felhasználói systemd-szolgáltatásként; előbb mérés a minimális kliensgépen |
| Q5 | Kötelező legyen-e a `producer` mező hitelesített identitáshoz kötése (E13)? | Igen; a Gateway a kapcsolat identitásából tölti ki, eltérésnél elutasít |
| Q6 | Hogyan fusson a replay az idempotencia mellett (E14)? | Külön replay-Consumer-identitás vagy külön idempotencia-névtér |
| Q7 | Bekerüljön-e a mutációs fuzz-teszt a 20. pontba és a 25. kapuba (E11)? | Igen (`AGENTS.md` §0.8) |
