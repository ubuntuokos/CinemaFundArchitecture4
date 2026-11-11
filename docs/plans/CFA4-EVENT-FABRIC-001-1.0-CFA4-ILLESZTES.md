# CFA4-EVENT-FABRIC-001 1.0 – CFA4-illesztési melléklet

**Dátum:** 2026-11-02 · **Állapot:** minden kérdés eldöntve (tulajdonos: Q1–Q4 és Q9–Q12 2026-11-06, Q5–Q8 2026-11-08) · **Terv:**
[`CFA4-EVENT-FABRIC-001-1.0.md`](CFA4-EVENT-FABRIC-001-1.0.md)

Ez a melléklet **nem módosítja** a jóváhagyott tervet. Azt rögzíti, hol tér el a terv a `main`-ben ma
rögzített CFA4-szabályoktól és meglévő tervektől (`VISION.md` fejléce szerint az ellentmondást jelezni
kell, nem csendben feloldani). Ha a tulajdonos egy pontban dönt, a döntés ide kerül, és szükség esetén
a terv új verziót kap (1.1).

## 1. Eltérések

| # | Téma | A terv | A CFA4 mai állapota | Javaslat |
|---|---|---|---|---|
| E1 | HRB (13.1, 17.3, 27.2) | erőforrás-hatóság | **CFA3-elem.** A CFA4 nem függhet a CFA3 HRB-jétől ([`SUBTITLE-STUDIO-PLAN-001-FINAL-1.2.md`](SUBTITLE-STUDIO-PLAN-001-FINAL-1.2.md), 199. sor). Általános hardvererőforrás-hatóság a CFA4-ben **nincs**: az [ADR 0004](../adr/ADR-0004-compute-routing.md) útválasztója (`cfa4-route`) csak feladatonként választ háttérmotort és eszközt, nem foglal; a `CFA4-AI_modules` ADR-0006 erőforrás-kezelője (`cfa4-ai-jobs`) csak az AI-feladatok befogadását és VRAM-foglalását kezeli | Nyitott előfeltétel: ki a CFA4 erőforrás-hatósága. Vagy a `cfa4-ai-jobs` erőforrás-kezelőjét kell általánossá tenni, vagy új terv kell; addig a 13.1 és a 17.3 nem teljesíthető |
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
| E15 | Eseményverzió jelölése (5.1, 6.2) | „a típus és a verzió külön mezőben szerepel”, de a 6.2 példái `.v1` utótagot írnak a típushoz, az 5.2 borítéka pedig `event_type: media.asset.ingested` + `event_version: 1` | – | Egy kanonikus alak: `event_type` verzió nélkül, a verzió csak az `event_version` mezőben; a `.vN` utótagos alak a `schema_id` (pl. `cfa4.media.asset.ingested.v1`). A katalógus, az ACL és a routing ugyanezt használja |
| E16 | Producer oldali kettős írás (8.1–8.2, 9.) | a kritikus esemény JetStreamen megy, a Consumer csak tartós feldolgozás után ACK-ol | A producer saját állapotváltozása és a közzététel nem atomi: ha a kettő között összeomlik, az állapot véglegesül, az esemény elvész (vagy fordítva). A CFA4 never-crash szabálya adatvesztést sem enged | Kritikus eseménynél kötelező a tranzakciós kimenőtár (outbox) vagy azzal egyenértékű helyreállítási út; az F-tesztekbe kerüljön be az összeomlási ablak tesztje (Q9) |
| E17 | Egy feladat életciklus-eseményeinek sorrendje (13.4, 9.) | `job.started` / `progress` / `completed` …; az ismétlést az `event_id` szűri | Párhuzamos Consumer vagy több kliens között megosztott JetStream-Consumer esetén a `completed` korábban tárolódhat, mint egy régebbi `progress`, és azt felülírja. Az `event_id` szerinti szűrés ezt nem veszi észre | Entitásonkénti (pl. `job_id`) sorrend- vagy sorszám-ellenőrzés, elavult frissítés elvetése; teszt átrendezett kézbesítéssel (Q10) |
| E18 | Protobuf-payload kódolása (5.1, 6.1, EF-007) | a `payload` típusa `Object`, a példa `application/json` | Nincs meghatározva, hogy Protobuf-eseménynél a payload bináris, base64 vagy JSON-leképezés; az SDK-k eltérhetnek, a közös JSON-tesztvektorok ezt nem fogják meg | A kanonikus kódolás és a `content_type`-leképezés rögzítése; SDK-k közötti Protobuf-tesztvektorok (Q11) |
| E19 | `trace_id` formátuma (5.1–5.2, 16.2) | String; példa: `trace-example-001` | Az OpenTelemetry/W3C Trace Context 32 kisbetűs hexa karaktert vár, az érvénytelent eldobja, így a nyomkövetés csendben megszakad | Formátum rögzítése és ellenőrzése (W3C trace-id), a példa cseréje érvényes értékre a végrehajtási tervben (Q12) |

## 2. Kérdések és döntések

| # | Kérdés | Javaslat | Döntés |
|---|---|---|---|
| Q1 | Mely CFA4-elemek lépnek a HRB, Execution Fabric, Agent Coordination, Security Governance és Evidence Registry helyére (E1–E6)? | A Model Routernek van megfelelője (`cfa4-ai-registry`); a többinél, köztük az erőforrás-hatóságnál (E1), előbb terv kell, vagy egy meglévő elem (pl. `cfa4-ai-jobs`) általánosítása | Elfogadva (tulajdonos, 2026-11-06): Model Router → `cfa4-ai-registry`, Evidence Registry → a CFA4 naplója; az erőforrás-hatósághoz, a Security Governance-hez és az Execution Fabrichoz előbb terv kell |
| Q2 | Lehet-e részkiadás (E7), amelyre fogyasztó építhet? | Igen: EF-R1 = F1–F5 + Rust és Python SDK | Elfogadva a javaslat szerint (tulajdonos, 2026-11-06) |
| Q3 | Kell-e most a C++ és a TypeScript SDK (E8)? | Csak ha lesz fogyasztója | Elfogadva a javaslat szerint (tulajdonos, 2026-11-06) |
| Q4 | Egygépes módban is a teljes NATS + JetStream + Valkey fusson-e, és hogyan települjön (E9)? | Igen, felhasználói systemd-szolgáltatásként; előbb mérés a minimális kliensgépen | Elfogadva a javaslat szerint (tulajdonos, 2026-11-06) |
| Q5 | Kötelező legyen-e a `producer` mező hitelesített identitáshoz kötése (E13)? | Igen; a Gateway a kapcsolat identitásából tölti ki, eltérésnél elutasít | Elfogadva a javaslat szerint (tulajdonos, 2026-11-08) |
| Q6 | Hogyan fusson a replay az idempotencia mellett (E14)? | Külön replay-Consumer-identitás vagy külön idempotencia-névtér | Elfogadva a javaslat szerint (tulajdonos, 2026-11-08) |
| Q7 | Bekerüljön-e a mutációs fuzz-teszt a 20. pontba és a 25. kapuba (E11)? | Igen (`AGENTS.md` §0.8) | Elfogadva a javaslat szerint (tulajdonos, 2026-11-08) |
| Q8 | Melyik az eseménytípus kanonikus alakja (E15)? | `event_type` verzió nélkül; `.vN` csak a `schema_id`-ban | Elfogadva a javaslat szerint (tulajdonos, 2026-11-08) |
| Q9 | Kötelező-e az outbox (vagy egyenértékű) a kritikus események kibocsátóinál (E16)? | Igen, összeomlási-ablak teszttel | Elfogadva a javaslat szerint (tulajdonos, 2026-11-06) |
| Q10 | Hogyan biztosítjuk egy entitás eseményeinek sorrendjét (E17)? | Entitásonkénti sorszám, elavult frissítés elvetése; teszt átrendezett kézbesítéssel | Elfogadva a javaslat szerint (tulajdonos, 2026-11-06) |
| Q11 | Mi a Protobuf-payload kanonikus kódolása (E18)? | Rögzített kódolás és `content_type`-leképezés, közös Protobuf-tesztvektorok | Elfogadva a javaslat szerint (tulajdonos, 2026-11-06) |
| Q12 | Kötelező-e a W3C-formátumú `trace_id` (E19)? | Igen, a Gateway ellenőrzi | Elfogadva a javaslat szerint (tulajdonos, 2026-11-06) |
