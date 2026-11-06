# CFA4-EVENT-FABRIC-001 – Részletes, átadható megvalósítási terv – 1.0

> **Eredet és állapot.** A terv a tulajdonos (ubuntuokos) ChatGPT-munkamenetében készült, és a
> tulajdonos **2026-11-02-án jóváhagyta**. A szöveg itt változatlan; csak a formázás (táblázatok,
> kódblokkok) igazodik a Markdownhoz, és a beszélgetésből átkerült két illusztráció-felirat maradt ki.
> A CFA4 `main`-ben rögzített szabályokhoz való illesztés kérdései a
> [CFA4-illesztési mellékletben](CFA4-EVENT-FABRIC-001-1.0-CFA4-ILLESZTES.md) vannak; ezek tulajdonosi
> döntésig nyitottak, és nem módosítják a jóváhagyott szöveget.

## Dokumentumadat

| Adat | Érték |
|---|---|
| Azonosító | CFA4-EVENT-FABRIC-001 |
| Verzió | 1.0 – FINAL terv |
| Dátum | 2026. november 2. |
| Rendszer | Cinema Fund Architectura 4 |
| Komponens | Event Fabric |
| Állapot | Részletes megvalósítási terv |
| Megvalósítási mód | Offline-first, local-first |
| Üzenetközvetítő | NATS Core + NATS JetStream |
| Gyors állapottár | Valkey |
| Elsődleges implementációs nyelv | Rust |
| Integrációs nyelvek | Rust, Python, C++, TypeScript |
| GUI-integráció | Qt6 |
| Biztonsági modell | Fail-closed |

## 1. A dokumentum célja

A CFA4 Event Fabric a CFA4 komponensei, szolgáltatásai, AI-motorjai, végrehajtási környezetei és
felhasználói felületei közötti eseményalapú kommunikáció egységes infrastruktúrája.

Feladata, hogy szabályozott, visszakövethető és megbízható módon biztosítsa:

- a rendszerszintű eseménykommunikációt;
- az aszinkron komponensintegrációt;
- az események tartós megőrzését;
- a feliratkozások és eseményfeldolgozások kezelését;
- a munkafolyamatok közötti koordinációt;
- az események visszajátszását;
- a hibás események elkülönítését;
- az események eredetének és feldolgozási útjának bizonyíthatóságát;
- a helyi, internetkapcsolat nélküli működést.

Az Event Fabric nem egyszerű üzenetküldő szolgáltatás. A CFA4 közös eseménykommunikációs infrastruktúrája.

### 1.1. Alapvető tervezési elv

Minden CFA4-komponensnek ugyanazon szabályozott eseménykommunikációs felületen keresztül kell
kommunikálnia, amikor a kommunikáció eseményalapú vagy több komponens közötti koordinációt igényel.

A komponensek nem hozhatnak létre egymástól független, saját eseménybuszokat, amelyek megkerülik a
központi szabályozást.

### 1.2. A megvalósítás határa

Ez a dokumentum az Event Fabric megvalósítási terve.

Nem jelenti a teljes CFA4 alkalmazásrendszer megvalósítását, és nem módosítja más CFA4-komponensek
saját felelősségi körét.

A terv elkészítése nem jár repository-módosítással, commit létrehozásával vagy fejlesztési ág létrehozásával.

## 2. Rendszerszintű architekturális pozíció

Az Event Fabric a CFA4 infrastruktúrájának közös kommunikációs rétege.

A logikai architektúra:

```text
                    CFA4 APPLICATIONS
                           |
          +----------------+----------------+
          |                |                |
      Qt6 GUI          AI Engines       Media Engines
          |                |                |
          +----------------+----------------+
                           |
                    CFA4 SDK LAYER
                           |
                 EVENT FABRIC API
                           |
          +----------------+----------------+
          |                |                |
     Event Gateway    Schema Registry   Policy Layer
          |                |                |
          +----------------+----------------+
                           |
                  EVENT FABRIC CORE
                       (Rust)
                           |
             +-------------+-------------+
             |                           |
         NATS Core                  NATS JetStream
       Live Messaging             Persistent Events
             |                           |
             +-------------+-------------+
                           |
                      VALKEY
              Transient State / Cache
                           |
                  OBSERVABILITY
```

### 2.1. Fő architekturális rétegek

| Réteg | Felelősség |
|---|---|
| Application Layer | CFA4 alkalmazások és szolgáltatások |
| SDK Layer | Nyelvspecifikus kliensfelületek |
| API Layer | Egységes eseménykezelési API |
| Gateway Layer | Bemeneti validáció, jogosultságvizsgálat |
| Schema Layer | Eseményszerződések és verziókezelés |
| Core Layer | Rust-alapú központi koordináció |
| Messaging Layer | NATS Core és JetStream |
| State Layer | Valkey-alapú átmeneti állapot |
| Observability Layer | Naplózás, metrikák, tracing és audit |

### 2.2. Központi architekturális szabály

A NATS a tényleges üzenetközvetítő infrastruktúra.

A Valkey nem üzenetközvetítő, nem eseménytörténeti adatbázis és nem lehet a JetStream helyettesítője.

A Rust Core felelős az Event Fabric szabályainak érvényesítéséért, az események életciklusának
koordinációjáért és az integrációs felület biztosításáért.

## 3. Technológiai alapok

### 3.1. NATS Core

NATS Core felelős a kis késleltetésű, élő eseménykommunikációért.

Felhasználási területek:

- valós idejű komponensértesítések;
- szolgáltatásállapot-változások;
- GUI-frissítési események;
- AI-motorok közötti koordináció;
- vezérlési események;
- rövid életű, nem perzisztens kommunikáció.

Elvárások:

1. Alacsony kommunikációs késleltetés.
2. Subject-alapú útválasztás.
3. Több feliratkozó támogatása.
4. Dinamikus feliratkozás és leiratkozás.
5. Hitelesített kapcsolat.
6. Jogosultságkorlátozás.
7. Kapcsolatvesztés felismerése.

A NATS Core használata önmagában nem biztosít tartós eseménytárolást.

### 3.2. NATS JetStream

NATS JetStream az Event Fabric tartós eseménykezelési rétege.

Feladata:

- események perzisztens tárolása;
- Consumer állapotok kezelése;
- ACK/NACK feldolgozás;
- újrapróbálkozás;
- későbbi feldolgozás;
- esemény-visszajátszás;
- tartós munkafolyamat-kommunikáció;
- hibás feldolgozások elkülönítése.

A JetStreamet kell használni minden olyan eseménynél, amelynek elvesztése üzleti, rendszerbiztonsági
vagy végrehajtási problémát okozhat.

Kötelező garanciák:

- explicit ACK;
- konfigurált maximális kézbesítési kísérletszám;
- tartós Consumer-állapot;
- deduplikációs támogatás;
- üzenetazonosító;
- esemény-visszajátszási lehetőség;
- ellenőrizhető feldolgozási eredmény.

Az Event Fabric nem állíthat általános, végpontok közötti exactly-once feldolgozást. Az alapmodell
legalább egyszeri kézbesítés (at-least-once), idempotens feldolgozással és üzleti szintű deduplikációval.

### 3.3. Valkey

Valkey az átmeneti állapotok és gyors hozzáférést igénylő adatok tárolója.

Feladatok:

- aktív Consumer állapotok gyors elérése;
- rövid élettartamú koordinációs adatok;
- átmeneti lockok;
- rate limiting;
- rövid idejű idempotencia-gyorsítótár;
- gyors állapotlekérdezés;
- ideiglenes routing metadata.

Nem használható:

- a JetStream tartós eseményeinek kiváltására;
- kizárólagos auditnaplóként;
- egyetlen tartós eseményforrásként;
- helyreállíthatatlan üzleti állapot tárolására.

Valkey-kiesés esetén a tartós események megőrzését a JetStreamnek kell biztosítania. Azok a műveletek,
amelyek biztonságos végrehajtásához a Valkey-állapot nélkülözhetetlen, nem folytathatók ellenőrzés nélkül.

### 3.4. Rust Core

A központi Event Fabric implementáció Rust nyelven készül.

Indokok:

- memória- és szálbiztonság;
- kiszámítható erőforrás-felhasználás;
- aszinkron feldolgozás;
- megfelelő teljesítmény;
- szigorú típuskezelés;
- hosszú távon fenntartható szolgáltatásimplementáció.

A Core fő feladatai:

1. Kapcsolatkezelés.
2. Eseményvalidáció.
3. Sémaellenőrzés.
4. Jogosultságellenőrzés.
5. Routing.
6. Consumer-koordináció.
7. Retry és DLQ.
8. Idempotencia.
9. Audit-integráció.
10. Observability.
11. Állapot-helyreállítás.

### 3.5. Támogatott programozási nyelvek

| SDK | Feladat |
|---|---|
| Rust SDK | Natív CFA4-szolgáltatások |
| Python SDK | AI- és automatizálási komponensek |
| C++ SDK | Natív multimédiás és teljesítménykritikus komponensek |
| TypeScript SDK | Webes és egyéb TypeScript-alapú interfészek |

Minden SDK ugyanazt az eseményprotokollt, jogosultsági modellt és eseményszerződést használja.

Nem megengedett, hogy egy nyelvi SDK saját, a többi SDK-val inkompatibilis eseményformátumot vezessen be.

## 4. Event Fabric Core komponensei

A központi szolgáltatás az alábbi modulokból áll.

| Modulazonosító | Modul | Felelősség |
|---|---|---|
| EF-CORE-001 | Connection Manager | NATS-kapcsolatok |
| EF-CORE-002 | Event Gateway | Eseményfogadás és továbbítás |
| EF-CORE-003 | Envelope Validator | Borítékvalidáció |
| EF-CORE-004 | Schema Registry Client | Sémaellenőrzés |
| EF-CORE-005 | Policy Enforcement | Jogosultságok |
| EF-CORE-006 | Subject Router | Eseményútválasztás |
| EF-CORE-007 | Consumer Manager | Feliratkozók |
| EF-CORE-008 | Delivery Manager | Kézbesítés |
| EF-CORE-009 | Retry Manager | Újrapróbálkozás |
| EF-CORE-010 | Dead Letter Manager | Hibás események |
| EF-CORE-011 | Idempotency Manager | Duplikációkezelés |
| EF-CORE-012 | Replay Manager | Visszajátszás |
| EF-CORE-013 | Audit Adapter | Auditkapcsolat |
| EF-CORE-014 | Metrics Adapter | Metrikák |
| EF-CORE-015 | Trace Adapter | Elosztott követés |
| EF-CORE-016 | Health Manager | Egészségi állapot |
| EF-CORE-017 | Recovery Manager | Helyreállítás |
| EF-CORE-018 | Configuration Manager | Konfiguráció |

### 4.1. Modulok közötti elválasztás

A modulok egymással meghatározott belső interfészeken keresztül kommunikálnak.

Kötelező:

- explicit interfészek;
- típusos adatszerkezetek;
- hibakódok egységes kezelése;
- strukturált naplózás;
- determinisztikus konfigurációbetöltés;
- tesztelhető komponenshatárok.

A modulok nem kerülhetik meg a Policy Enforcement réteget.

### 4.2. Hibakezelési alapelv

A kritikus komponens hibája nem vezethet jogosultságellenőrzés nélküli eseményfeldolgozáshoz.

Ha a rendszer nem tudja megállapítani:

- az esemény eredetét;
- a kibocsátó jogosultságát;
- az esemény típusát;
- az esemény sémáját;
- a cél Consumer jogosultságát;

akkor az eseményt el kell utasítani vagy biztonságos elkülönített állapotba kell helyezni.

## 5. Canonical Event Envelope

Minden CFA4-esemény egységes borítékformátumot használ.

A boríték független az esemény tartalmától, így ugyanaz az alapstruktúra használható Python, Rust,
C++ és TypeScript komponensek között.

### 5.1. Kötelező mezők

| Mező | Típus | Kötelező | Jelentés |
|---|---|---|---|
| `event_id` | UUID | Igen | Egyedi eseményazonosító |
| `event_type` | String | Igen | Eseménytípus |
| `event_version` | Integer | Igen | Eseményverzió |
| `occurred_at` | Timestamp | Igen | Esemény létrejötte |
| `published_at` | Timestamp | Igen | Közzététel |
| `producer` | String | Igen | Kibocsátó |
| `producer_instance` | String | Igen | Példányazonosító |
| `correlation_id` | UUID | Igen | Műveletkorreláció |
| `causation_id` | UUID/null | Igen | Kiváltó esemény |
| `trace_id` | String | Igen | Trace azonosító |
| `schema_id` | String | Igen | Sémaazonosító |
| `content_type` | String | Igen | Tartalomtípus |
| `classification` | Enum | Igen | Adatbesorolás |
| `payload` | Object | Igen | Eseményadat |
| `metadata` | Object | Igen | Kiegészítő metaadat |

### 5.2. Példa JSON-borítékra

```json
{
  "event_id": "0192c1a0-8a21-7b42-a20a-112233445566",
  "event_type": "media.asset.ingested",
  "event_version": 1,
  "occurred_at": "2026-11-02T18:30:00Z",
  "published_at": "2026-11-02T18:30:01Z",
  "producer": "cfa4.media.ingest",
  "producer_instance": "ingest-01",
  "correlation_id": "0192c1a0-8a21-7b42-a20a-112233445567",
  "causation_id": null,
  "trace_id": "trace-example-001",
  "schema_id": "cfa4.media.asset.ingested.v1",
  "content_type": "application/json",
  "classification": "internal",
  "payload": {
    "asset_id": "asset-001",
    "media_type": "video",
    "status": "ingested"
  },
  "metadata": {
    "source": "local",
    "environment": "production"
  }
}
```

A példa szemléltető adatokat tartalmaz; az éles rendszerben az azonosítóknak és az időbélyegeknek a
tényleges eseményhez kell tartozniuk.

### 5.3. További szabályok

1. Az eseményazonosító nem változhat újrapróbálkozáskor.
2. Az eredeti esemény időpontját nem szabad újrakézbesítéskor felülírni.
3. A feldolgozási időt külön kell rögzíteni.
4. A `correlation_id` segítségével a teljes munkafolyamat rekonstruálható.
5. A `causation_id` megmutatja, mely esemény váltotta ki az aktuális eseményt.
6. A payload nem tartalmazhat hitelesítő adatokat vagy titkokat.
7. A besorolás nem hagyható el.

## 6. Eseményszerződések és Schema Registry

A CFA4 Event Fabric központi sémanyilvántartást alkalmaz.

### 6.1. Kötelező támogatás

- JSON Schema;
- Protobuf;
- verziózott eseményszerződések;
- kompatibilitás-ellenőrzés;
- sémaazonosítók;
- közzététel előtti validáció;
- fogyasztói kompatibilitás vizsgálata;
- régebbi eseményverziók kezelése.

### 6.2. Verziózási modell

Az eseménytípus és a verzió külön mezőben szerepel.

Példa:

```text
media.asset.ingested.v1
media.asset.ingested.v2
ai.model.loaded.v1
execution.job.completed.v1
security.policy.denied.v1
```

A séma módosításakor:

1. A módosítás kompatibilitását ellenőrizni kell.
2. Visszafelé kompatibilis módosítás esetén a meglévő verzió kezelhető a meghatározott kompatibilitási szabályok szerint.
3. Nem kompatibilis módosítás új verziót igényel.
4. A régi eseményeket nem szabad új verziójú payloadként értelmezni.
5. A régi Consumer-eknek nem kötelező automatikusan új sémára átállniuk.

### 6.3. Tiltott gyakorlatok

- Séma nélküli éles esemény.
- Nem verziózott payload.
- Ismeretlen mezők csendes, jelentésváltoztató átalakítása.
- Eseménytípus jelentésének újrahasznosítása más célra.
- Nem kompatibilis sémafrissítés észrevétlen bevezetése.

## 7. Subject- és routingrendszer

A NATS Subject-rendszer hierarchikus névteret használ.

### 7.1. Névkonvenció

```text
cfa4.<domain>.<component>.<event>
```

Példák:

```text
cfa4.system.health.changed
cfa4.media.asset.ingested
cfa4.media.render.started
cfa4.media.render.completed
cfa4.ai.model.loaded
cfa4.ai.model.unloaded
cfa4.ai.inference.completed
cfa4.execution.job.created
cfa4.execution.job.completed
cfa4.security.policy.denied
cfa4.agent.task.assigned
cfa4.agent.task.completed
```

### 7.2. Domain-szétválasztás

| Domain | Tartalom |
|---|---|
| system | Rendszerállapot |
| media | Multimédiás események |
| ai | AI-modellek és következtetés |
| execution | Feladatvégrehajtás |
| agent | Agent-koordináció |
| security | Biztonsági események |
| resource | Erőforrás-állapot |
| storage | Tárolási események |
| ui | Felhasználói felületi események |
| workflow | Munkafolyamatok |
| audit | Audit-események |

### 7.3. Routingkövetelmények

1. A producer nem adhat tetszőleges, ellenőrizetlen subjectet.
2. A routing a regisztrált eseménytípus és a jogosultsági szabályok alapján történik.
3. A Consumer csak az engedélyezett subjectekre iratkozhat fel.
4. A wildcard feliratkozás korlátozott jogosultságot igényel.
5. A belső rendszeresemények nem tehetők automatikusan elérhetővé alkalmazások számára.

## 8. Delivery, ACK és retry rendszer

### 8.1. Kézbesítési modell

Az Event Fabric két fő kommunikációs modellt biztosít:

| Modell | Felhasználás |
|---|---|
| NATS Core | Élő, átmeneti események |
| JetStream | Tartós, visszaállítható események |

A kritikus eseményeket JetStreamen kell továbbítani.

### 8.2. ACK-szabályok

A Consumer csak akkor küldhet ACK-ot, ha az eseményt a saját feldolgozási szerződése szerint
sikeresen feldolgozta.

Az ACK nem jelentheti pusztán az esemény fogadását.

Példa:

```text
RECEIVE
   |
VALIDATE
   |
AUTHORIZE
   |
PROCESS
   |
PERSIST RESULT
   |
ACK
```

Ha a feldolgozás hibás:

```text
RECEIVE
   |
VALIDATE
   |
PROCESS
   |
FAILURE
   |
RETRY / DLQ
```

### 8.3. Retry-szabályok

A retry konfigurálható, de kötelezően korlátozott.

Alapértelmezett javasolt stratégia:

| Kísérlet | Várakozás |
|---|---|
| 1. | 1 másodperc |
| 2. | 2 másodperc |
| 3. | 5 másodperc |
| 4. | 15 másodperc |
| 5. | 30 másodperc |

Az értékek eseményosztályonként módosíthatók.

A rendszernek támogatnia kell:

- exponenciális backoffot;
- maximális próbálkozásszámot;
- maximális késleltetést;
- jittert;
- Consumer-specifikus retry-politikát;
- hibakategóriák szerinti eltérő kezelést.

Biztonsági vagy jogosultsági hibánál nem szabad végtelen retry-t alkalmazni.

### 8.4. Dead Letter Queue

A DLQ elkülöníti a véglegesen hibás feldolgozású eseményeket.

Kötelező metaadatok:

- eredeti event ID;
- eredeti subject;
- Consumer ID;
- hibakód;
- hibaleírás;
- próbálkozások száma;
- első sikertelen feldolgozás időpontja;
- utolsó feldolgozási időpont;
- korrelációs azonosító.

A DLQ-ból történő visszajátszás kizárólag jogosultság-ellenőrzött művelet lehet.

## 9. Idempotencia és duplikációkezelés

A kézbesítési modellből következően egy esemény többször is eljuthat ugyanahhoz a Consumerhez.

Ezért minden kritikus feldolgozásnak idempotensnek kell lennie.

### 9.1. Idempotenciakulcs

Alapértelmezett kulcs:

```text
consumer_id + event_id
```

A Consumernek ellenőriznie kell, hogy az esemény feldolgozása korábban megtörtént-e.

### 9.2. Követelmények

1. Azonos esemény többszöri feldolgozása nem okozhat ismételt üzleti műveletet.
2. Az idempotencia-állapotnak a feldolgozási eredménnyel összhangban kell állnia.
3. A Valkey-gyorsítótár önmagában nem lehet a végleges deduplikációs bizonyíték.
4. A tartós feldolgozási állapotot az adott üzleti komponens tartós adatkezelésének kell biztosítania.
5. A deduplikációs ablak és az állapotmegőrzés időtartama dokumentált legyen.

### 9.3. Hibakezelési példa

Ha a Consumer elvégzi az üzleti műveletet, de az ACK elveszik, az esemény ismét kézbesítésre kerülhet.

A második feldolgozásnak fel kell ismernie a már végrehajtott műveletet, és ismételt végrehajtás nélkül
kell lezárnia az eseményt.

## 10. Event Replay és esemény-visszajátszás

Az Event Fabric támogatja a JetStreamben tárolt események ellenőrzött visszajátszását.

### 10.1. Felhasználási esetek

- hibás Consumer helyreállítása;
- új feldolgozási logika ellenőrzése;
- munkafolyamat rekonstrukciója;
- hibakeresés;
- állapotprojekció újraépítése;
- auditvizsgálat.

### 10.2. Replay-módok

1. Teljes stream-visszajátszás.
2. Időpont szerinti visszajátszás.
3. Eseményazonosító szerinti visszajátszás.
4. Consumer-specifikus visszajátszás.
5. Ellenőrzött, elkülönített teszt-visszajátszás.

### 10.3. Biztonsági korlátozások

A replay nem hajthat végre automatikusan visszafordíthatatlan külső műveleteket.

Különösen:

- fájlok törlése;
- külső rendszerek módosítása;
- felhasználói értesítések ismételt kiküldése;
- fizikai vagy hardveres műveletek;
- erőforrásfoglalások ismételt létrehozása.

Ezeknél az eredeti műveletet és az újrajátszott esemény feldolgozását külön kell kezelni.

## 11. Schema Registry és eseménykatalógus kezelése

Az eseménykatalógus az Event Fabric által elfogadott eseménytípusok nyilvántartása.

Minden eseményhez tartozik:

- egyedi eseményazonosító;
- domain;
- producer;
- engedélyezett Consumer-kör;
- schema ID;
- verzió;
- adatbesorolás;
- perzisztencia-előírás;
- retention-előírás;
- retry-politika;
- DLQ-politika;
- jogosultsági szabály.

A katalógus a fejlesztési és telepítési folyamat része.

Az ismeretlen eseménytípus éles környezetben nem fogadható el automatikusan.

## 12. Biztonsági architektúra

A biztonsági modell központi alapelve a fail-closed működés.

### 12.1. Hitelesítés

A komponensek egyedi identitást kapnak.

Követelmények:

- egyedi szolgáltatásazonosító;
- hitelesített NATS-kapcsolat;
- TLS;
- titkos kulcsok biztonságos tárolása;
- tanúsítványok ellenőrzése;
- lejárati és visszavonási folyamat.

### 12.2. Jogosultságkezelés

A jogosultságok producer-, Consumer-, domain- és subject-szinten szabályozandók.

Például:

```yaml
media.ingest:
  publish:
    - cfa4.media.asset.ingested

media.render:
  subscribe:
    - cfa4.media.asset.ingested
  publish:
    - cfa4.media.render.started
    - cfa4.media.render.completed
```

Ez szemléltető ACL-formátum, nem önmagában a NATS natív konfigurációja.

### 12.3. Titkosítás

- TLS a szolgáltatáskapcsolatoknál.
- Titkok elkülönített tárolása.
- Kulcsok naplózásának tiltása.
- Titkosított helyi adattárolás, ahol a rendszer biztonsági besorolása ezt előírja.
- Hitelesítési adatok nem szerepelhetnek az esemény payloadjában.

### 12.4. Audit

Minden kritikus műveletnek visszakövethetőnek kell lennie:

- esemény közzététele;
- esemény elutasítása;
- feliratkozás;
- jogosultságmódosítás;
- DLQ-ba helyezés;
- replay;
- Consumer-konfiguráció módosítása;
- szolgáltatásleállítás;
- helyreállítás.

Az audit integrációja a CFA4 központi Security Governance és Evidence Registry felületeivel történik.

### 12.5. Biztonsági hibák

Ha a jogosultságvizsgálat nem érhető el vagy nem ad egyértelmű eredményt, az érintett műveletet el kell utasítani.

Nincs automatikus jogosultságkiterjesztés, névtelen hozzáférés vagy csendes biztonsági visszaesés.

## 13. CFA4-integrációk

Az Event Fabric a CFA4 különböző rendszereinek közös eseménykommunikációs alapja.

### 13.1. HRB – Hardware Resource Broker

A HRB marad a hardver- és erőforrás-hozzáférés kizárólagos szabályozó hatósága.

Az Event Fabric csak az eseményeket továbbítja.

Példák:

```text
cfa4.resource.device.available
cfa4.resource.device.unavailable
cfa4.resource.allocation.changed
cfa4.resource.pressure.detected
```

Az Event Fabric nem foglalhat önállóan GPU-t, CPU-t, NPU-t vagy memóriát.

### 13.2. Model Router

A Model Router marad az AI-modell-útválasztás kizárólagos hatósága.

Példák:

```text
cfa4.ai.model.registered
cfa4.ai.model.loaded
cfa4.ai.model.unloaded
cfa4.ai.routing.changed
cfa4.ai.inference.completed
```

Az Event Fabric nem választhat modellt, és nem végezhet saját AI-routingot.

### 13.3. Agent Coordination

Az Agent Coordination az agentek közötti feladatkoordinációért felel.

Az Event Fabric biztosítja:

- feladatkiosztási események továbbítását;
- státuszváltozások kommunikációját;
- eredmények közvetítését;
- hibajelzéseket;
- korrelációs kapcsolatokat.

### 13.4. Execution Fabric

Az Execution Fabric felel a feladatok végrehajtásáért.

Az Event Fabric nem hajt végre munkafolyamatokat.

A végrehajtási események továbbítását biztosítja:

```text
cfa4.execution.job.created
cfa4.execution.job.started
cfa4.execution.job.progress
cfa4.execution.job.completed
cfa4.execution.job.failed
cfa4.execution.job.cancelled
```

### 13.5. Security Governance

A Security Governance biztosítja a biztonsági szabályokat.

Az Event Fabric ezeket érvényesíti, de nem hozhat létre velük ellentétes saját biztonsági szabályrendszert.

### 13.6. Evidence Registry

Az Evidence Registry felé az Event Fabric biztosítja a szükséges események és feldolgozási bizonyítékok továbbítását.

Az Event Fabric nem helyettesíti az Evidence Registry tartós bizonyítékkezelési funkcióját.

### 13.7. Integrációs felelősségi mátrix

| Funkció | Felelős rendszer |
|---|---|
| Eseményközvetítés | Event Fabric |
| Hardver-erőforrás döntés | HRB |
| AI-modell-útválasztás | Model Router |
| Agent-feladatkoordináció | Agent Coordination |
| Munkavégrehajtás | Execution Fabric |
| Biztonsági szabályok | Security Governance |
| Bizonyítékok nyilvántartása | Evidence Registry |
| Gyors átmeneti állapot | Valkey |
| Tartós eseménytárolás | JetStream |

Ez a felelősségi szétválasztás kötelező architekturális követelmény.

## 14. Offline-first és local-first működés

Az Event Fabric elsődleges működési környezete a helyi CFA4-rendszer.

Internetkapcsolat nem lehet az eseménykommunikáció előfeltétele.

### 14.1. Kötelező offline képességek

- helyi NATS-szerver;
- helyi JetStream-tárolás;
- helyi Valkey;
- helyi Schema Registry-adatok;
- helyi hitelesítési és jogosultsági konfiguráció;
- helyi eseményfeldolgozás;
- helyi auditintegráció;
- helyi helyreállítás;
- hálózati kapcsolat nélküli teljes alapműködés.

### 14.2. Hálózati kapcsolat elvesztése

Az internetkapcsolat megszűnése nem szakíthatja meg a helyi CFA4-komponensek közötti kommunikációt.

A helyi kommunikáció nem támaszkodhat külső felhőszolgáltatásra.

### 14.3. Helyi szolgáltatáskimaradás

| Hiba | Elvárt viselkedés |
|---|---|
| NATS Core kiesik | Új élő kommunikáció nem működik |
| JetStream kiesik | Kritikus tartós események közzététele leáll |
| Valkey kiesik | Átmeneti állapotfüggő műveletek korlátozódnak |
| Schema Registry elérhetetlen | Nem ellenőrizhető új események elutasítása |
| Auditkapcsolat kiesik | Auditköteles műveletek fail-closed kezelése |
| Consumer kiesik | JetStream megőrzi a függő eseményeket |
| Disk space kritikus | Új tartós események szabályozott elutasítása |

### 14.4. Helyreállítás

A szolgáltatás helyreállításakor:

1. A NATS állapotának ellenőrzése.
2. JetStream stream-ek és Consumer-ek ellenőrzése.
3. Tárolási állapot ellenőrzése.
4. Jogosultsági konfiguráció ellenőrzése.
5. Consumer-ek újracsatlakoztatása.
6. Függő események feldolgozása.
7. Retry és DLQ állapot ellenőrzése.
8. Audit és trace folytonosságának ellenőrzése.

Nem történhet automatikus eseményvesztés vagy ellenőrizetlen feldolgozási folytatás.

## 15. GUI és Control Panel integráció

Az Event Fabric kezelése a CFA4 Qt6-alapú Control Paneljéből történik.

A GUI nem helyettesíti az Event Fabric API-t, és nem lehet a szolgáltatás működésének előfeltétele.

### 15.1. Event Fabric kezelőfelület

A Control Panel külön Event Fabric kezelőfelületet biztosít.

Kötelező funkciók:

| Funkció | Leírás |
|---|---|
| Overview | Teljes rendszerállapot |
| Streams | JetStream stream-ek |
| Consumers | Consumer-ek és állapotuk |
| Event Catalog | Eseménytípusok |
| Schema Registry | Sémák és verziók |
| Delivery | Kézbesítési állapot |
| Retry | Újrapróbálkozások |
| DLQ | Hibás események |
| Replay | Visszajátszási műveletek |
| Security | Jogosultságok és audit |
| Metrics | Teljesítménymetrikák |
| Diagnostics | Hibakeresés |
| Recovery | Helyreállítás |

### 15.2. Élő eseménynézet

A felület képes:

- események megjelenítésére;
- domain szerinti szűrésre;
- producer szerinti szűrésre;
- időintervallum szerinti szűrésre;
- correlation ID keresésre;
- eseményállapot megjelenítésére;
- feldolgozási hibák megjelenítésére.

Az érzékeny payload-adatok alapértelmezésben maszkoltak.

### 15.3. Műveleti jogosultságok

A következő műveletekhez külön jogosultság szükséges:

- Consumer leállítása;
- Consumer újraindítása;
- konfigurációmódosítás;
- DLQ-kezelés;
- Replay indítása;
- stream-konfiguráció módosítása;
- szolgáltatás leállítása.

Minden adminisztratív művelet auditálandó.

## 16. Observability és diagnosztika

Az Event Fabric megvalósítása során a megfigyelhetőség nem utólag hozzáadott funkció, hanem alapkövetelmény.

### 16.1. Metrikák

A rendszernek legalább az alábbi metrikákat kell szolgáltatnia:

**Üzenetforgalom**

- publikált események száma;
- sikeresen kézbesített események;
- sikertelen kézbesítések;
- elutasított események;
- duplikált események;
- másodpercenkénti eseményszám.

**JetStream**

- streamenkénti eseményszám;
- tárolási méret;
- Consumer lag;
- függő üzenetek;
- ACK-késleltetés;
- redelivery szám.

**Retry és DLQ**

- retry-k száma;
- véglegesen sikertelen események;
- DLQ mérete;
- feldolgozási hibakategóriák.

**Rendszer**

- CPU;
- memória;
- lemezhasználat;
- hálózati forgalom;
- kapcsolatállapot;
- szolgáltatás-újraindítások.

### 16.2. Elosztott tracing

A `trace_id` és `correlation_id` az esemény teljes életciklusa során megmarad.

A tracing segítségével megállapítható:

- mely komponens indította az eseményt;
- milyen komponenseken haladt át;
- melyik Consumer dolgozta fel;
- hol jelentkezett késleltetés;
- hol következett be hiba;
- milyen eredmény született.

OpenTelemetry-kompatibilis trace-kimenet alkalmazandó.

### 16.3. Strukturált naplózás

A naplózás strukturált formátumot használjon.

Kötelező mezők:

```text
timestamp
severity
component
event_id
correlation_id
trace_id
operation
result
error_code
```

A titkok és érzékeny payload-adatok nem jelenhetnek meg a naplókban.

## 17. Telepítési architektúra

### 17.1. Helyi szolgáltatások

Az Event Fabric külön szolgáltatásokból épül fel:

```text
CFA4 EVENT FABRIC
|
+-- cfa4-event-fabric-core
|
+-- nats-server
|   +-- Core
|   +-- JetStream
|
+-- valkey-server
|
+-- schema-registry
|
+-- observability-adapter
|
+-- control-panel-integration
```

A Core és a Schema Registry CFA4-specifikus fejlesztés.

A NATS és Valkey külső, integrált infrastruktúra-komponensek.

### 17.2. Szolgáltatásfelügyelet

A szolgáltatások:

- önállóan indíthatók;
- önállóan leállíthatók;
- állapotuk ellenőrizhető;
- naplóik hozzáférhetők;
- verziójuk ellenőrizhető;
- frissítésük szabályozott.

### 17.3. Erőforrás-kezelés

Az Event Fabric nem foglalhat saját hatáskörben hardveres erőforrásokat.

A HRB által biztosított erőforrás-kezelési szabályok érvényesek.

A szolgáltatásnak támogatnia kell a konfigurálható:

- CPU-korlátokat;
- memória-korlátokat;
- lemezhasználati korlátokat;
- párhuzamos feldolgozási korlátokat;
- Consumer-terhelési korlátokat.

### 17.4. Tárolás

A JetStream adattárolása:

- helyi;
- tartós;
- konfigurálható;
- méretkorlátozott;
- monitorozott.

A retention és storage policy eseményosztályonként szabályozandó.

A rendszernek védenie kell magát a korlátlan eseményfelhalmozódástól.

## 18. API-specifikáció

Az Event Fabric API minden támogatott SDK számára egységes műveleti modellt biztosít.

### 18.1. Producer API

Alapműveletek:

```text
publish(event)
publish_persistent(event)
publish_ephemeral(event)
validate(event)
```

A közzététel eredménye tartalmazza:

- elfogadott vagy elutasított állapot;
- event ID;
- subject;
- stream ID, ha releváns;
- hibakód;
- visszajelzési időpont.

### 18.2. Consumer API

```text
subscribe(subject)
subscribe_persistent(subject)
unsubscribe(subscription_id)
ack(event_id)
nack(event_id)
terminate(event_id)
```

A Consumer nem módosíthatja az eredeti eseményazonosítót.

### 18.3. Admin API

```text
get_status()
list_streams()
list_consumers()
get_consumer_lag()
list_dlq()
replay_events()
update_consumer()
get_metrics()
get_health()
```

Az Admin API kizárólag hitelesített és jogosult adminisztratív komponensek számára érhető el.

### 18.4. Hibakódok

Egységes hibakód-struktúra szükséges.

| Kód | Jelentés |
|---|---|
| EF-1001 | Ismeretlen eseménytípus |
| EF-1002 | Érvénytelen eseményboríték |
| EF-1003 | Sémahiba |
| EF-1004 | Verzióütközés |
| EF-2001 | Hitelesítési hiba |
| EF-2002 | Jogosultságmegtagadás |
| EF-3001 | NATS-kapcsolati hiba |
| EF-3002 | JetStream-hiba |
| EF-3003 | Valkey-hiba |
| EF-4001 | Consumer-feldolgozási hiba |
| EF-4002 | Retry-limit túllépése |
| EF-4003 | DLQ-ba helyezés |
| EF-5001 | Tárolási korlát |
| EF-5002 | Recovery-hiba |

A hibakódok dokumentáltak, stabilak és géppel feldolgozhatók legyenek.

## 19. SDK-megvalósítási követelmények

Minden SDK-nak azonos funkcionális minimumot kell teljesítenie.

### 19.1. Rust SDK

Kötelező:

- aszinkron API;
- típusos eseményboríték;
- JSON és Protobuf;
- validáció;
- publish;
- subscribe;
- ACK/NACK;
- tracing;
- idempotencia-integráció;
- strukturált hibakezelés.

### 19.2. Python SDK

Felhasználási terület:

- AI-motorok;
- Python-alapú feldolgozók;
- automatizálási komponensek;
- adatfeldolgozási szolgáltatások.

Kötelező:

- asyncio-támogatás;
- típusos eseményobjektumok;
- sémaellenőrzés;
- retry-kezelés;
- tracing;
- hibakezelés.

### 19.3. C++ SDK

Felhasználási terület:

- multimédiás motorok;
- natív teljesítménykritikus komponensek;
- alacsony késleltetésű szolgáltatások.

Kötelező:

- RAII-alapú erőforráskezelés;
- biztonságos kapcsolat-életciklus;
- típusos eseménykezelés;
- aszinkron feldolgozás;
- explicit hibaeredmények.

### 19.4. TypeScript SDK

Felhasználási terület:

- TypeScript-alapú kezelőfelületek;
- kapcsolódó szolgáltatások;
- fejlesztői integrációk.

Kötelező:

- típusos interfészek;
- Promise-alapú API;
- eseményvalidáció;
- feliratkozáskezelés;
- biztonságos leiratkozás;
- tracing.

### 19.5. SDK-kompatibilitási teszt

A négy SDK ugyanazokat a tesztvektorokat kapja.

Ellenőrizni kell, hogy:

- ugyanazt a JSON-borítékot állítják elő;
- ugyanazokat a kötelező mezőket kezelik;
- ugyanazt a sémahibát jelzik;
- ugyanazt az eseménytípust ismerik fel;
- kompatibilis módon dolgozzák fel a verziózott eseményeket.

## 20. Tesztelési stratégia

Az Event Fabric csak mérhető és reprodukálható teszteredmények alapján fogadható el.

### 20.1. Unit tesztek

- borítékvalidáció;
- sémaellenőrzés;
- routing;
- ACL;
- retry;
- deduplikáció;
- hibakódok;
- konfiguráció;
- állapotátmenetek.

### 20.2. Integration tesztek

- Rust Core + NATS;
- Rust Core + JetStream;
- Rust Core + Valkey;
- Schema Registry-integráció;
- SDK-k közötti kommunikáció;
- auditintegráció;
- tracingintegráció.

### 20.3. Failure injection

Kötelezően tesztelendő:

1. NATS leállítása.
2. JetStream leállítása.
3. Valkey elérhetetlensége.
4. Consumer váratlan leállása.
5. Producer váratlan leállása.
6. Hálózati megszakítás.
7. Tárolási korlát elérése.
8. Hibás eseményséma.
9. Jogosulatlan producer.
10. Jogosulatlan Consumer.
11. Duplikált esemény.
12. Elveszett ACK.
13. DLQ-ba kerülő esemény.
14. Hibás replay.
15. Érvénytelen konfiguráció.

### 20.4. Terheléses tesztek

Mérendő:

- esemény/másodperc;
- átlagos késleltetés;
- p95 késleltetés;
- p99 késleltetés;
- Consumer lag;
- memóriahasználat;
- CPU-terhelés;
- tárolási növekedés;
- helyreállítási idő.

### 20.5. Hosszú futású teszt

A rendszernek tartós terhelés mellett is stabilan kell működnie.

Vizsgálni kell:

- memória növekedését;
- kapcsolatvesztéseket;
- Consumer-állapotokat;
- JetStream retention működését;
- Valkey memóriahasználatát;
- retry és DLQ növekedését;
- helyreállítási képességet.

## 21. Megvalósítási ütemterv

A fejlesztés önállóan ellenőrizhető fázisokra oszlik.

A fázisok egymásra épülő fejlesztési és elfogadási kapuk. A következő fázis csak az előző elfogadott
eredményeire építhet.

| Fázis | Tartalom |
|---|---|
| **F1 – Foundation és infrastruktúra** (alapozó fázis) | NATS Core és JetStream; Valkey; alapkonfiguráció; helyi szolgáltatásindítás; health check; offline működés alapjai |
| **F2 – Rust Event Fabric Core** | Connection Manager; Event Gateway; Canonical Envelope; Subject Router; alapvető publish/subscribe |
| **F3 – Schema és Policy** | Schema Registry; JSON Schema; Protobuf; ACL; TLS; fail-closed működés |
| **F4 – Delivery és Reliability** | ACK/NACK; Retry; DLQ; Idempotencia; Consumer Manager |
| **F5 – Replay és Recovery** | Replay; helyreállítás; hibás események kezelése; tartós állapot ellenőrzése |
| **F6 – SDK-k** | Rust SDK; Python SDK; C++ SDK; TypeScript SDK; közös kompatibilitási tesztek |
| **F7 – CFA4-integrációk és Control Panel** | HRB; Model Router; Agent Coordination; Execution Fabric; Security Governance; Evidence Registry; Qt6 Control Panel |
| **F8 – Rendszerszintű validáció és kiadás** (végső kapu) | Teljes integrációs teszt; failure injection; terheléses teszt; hosszú futású teszt; biztonsági vizsgálat; kiadási elfogadás |

### 21.1. Fázisonkénti elfogadási szabály

Minden fázisnak rendelkeznie kell:

- végrehajtható tesztekkel;
- teszteredményekkel;
- dokumentált hibákkal;
- megfelelőségi bizonyítékokkal;
- reprodukálható telepítési eljárással;
- egyértelmű PASS/FAIL állapottal.

A következő fázis nem tekinthető elfogadottnak pusztán azért, mert az előző fázis kódja elkészült.

## 22. Mérhető elfogadási kritériumok

Az Event Fabric elfogadását az alábbi 25 követelmény határozza meg.

**EF-001–EF-025 – Kötelező megfelelőségi jegyzék**

| ID | Követelmény | Elfogadási feltétel |
|---|---|---|
| EF-001 | Helyi infrastruktúra | NATS és Valkey internet nélkül elindul |
| EF-002 | NATS Core | Élő eseménykommunikáció működik |
| EF-003 | JetStream | Tartós eseményközzététel és -fogadás működik |
| EF-004 | Valkey | Átmeneti állapotok kezelhetők |
| EF-005 | Canonical Envelope | Minden kötelező mező validálható |
| EF-006 | JSON Schema | Hibás payload elutasítva |
| EF-007 | Protobuf | Szerializációs tesztek PASS |
| EF-008 | Verziókezelés | Nem kompatibilis sémafrissítés felismerhető |
| EF-009 | Routing | Csak regisztrált subject használható |
| EF-010 | ACL | Jogosulatlan publikálás elutasítva |
| EF-011 | Consumer ACL | Jogosulatlan feliratkozás elutasítva |
| EF-012 | TLS | Hitelesített titkosított kapcsolat működik |
| EF-013 | ACK/NACK | Feldolgozási állapot helyesen kezelhető |
| EF-014 | Retry | Korlátozott újrapróbálkozás működik |
| EF-015 | DLQ | Végleges hibák elkülönítése működik |
| EF-016 | Idempotencia | Duplikált esemény nem okoz ismételt üzleti műveletet |
| EF-017 | Replay | Engedélyezett visszajátszás működik |
| EF-018 | Recovery | Consumer-kiesés után helyreállítható a feldolgozás |
| EF-019 | Rust SDK | A közös SDK-tesztek PASS |
| EF-020 | Python SDK | A közös SDK-tesztek PASS |
| EF-021 | C++ SDK | A közös SDK-tesztek PASS |
| EF-022 | TypeScript SDK | A közös SDK-tesztek PASS |
| EF-023 | Observability | Metrikák, naplók és tracing ellenőrizhető |
| EF-024 | CFA4-integráció | A kijelölt CFA4-komponensek integrációs tesztjei PASS |
| EF-025 | Rendszerszintű validáció | Minden kötelező funkcionális, biztonsági és helyreállítási teszt PASS |

### 22.1. Teljesítménykövetelmények

Az alábbi értékek a megvalósítás során mérendő célértékek, nem előzetesen igazolt teljesítményadatok.

| Mutató | Célérték |
|---|---|
| NATS Core belső kommunikáció p95 | ≤ 10 ms |
| NATS Core belső kommunikáció p99 | ≤ 25 ms |
| JetStream publikálás p95 | ≤ 50 ms |
| JetStream publikálás p99 | ≤ 100 ms |
| Envelope validáció p95 | ≤ 2 ms |
| Helyi Core egészségellenőrzés | ≤ 1 másodperc |
| Hibás Consumer felismerése | ≤ 10 másodperc |
| Helyi szolgáltatások internet nélküli működése | 100% |
| Jogosultságellenőrzés megkerülhetősége | 0 |
| Nem engedélyezett események elfogadása | 0 |

A késleltetési célokat rögzített tesztkörnyezetben, dokumentált terhelési profillal kell ellenőrizni.
A JetStream értékei a tartós publikálás visszaigazolására vonatkoznak, nem a teljes Consumer-feldolgozási időre.

### 22.2. Megbízhatósági követelmények

- Nem veszhet el ACK-olt, tartós esemény a vállalt tárolási garanciák mellett.
- A Consumer újraindítása után a függő események feldolgozhatók.
- A duplikált kézbesítés nem okozhat ismételt üzleti műveletet.
- A Valkey kiesése nem törölheti a JetStream eseményeit.
- A jogosultsági hiba nem vezethet engedélyezettként kezelt eseményhez.
- A tárolási korlát elérése nem okozhat csendes adatvesztést.

## 23. Kockázatok és kezelési stratégiák

| Kockázat | Következmény | Kezelés |
|---|---|---|
| JetStream tároló megtelik | Eseményközzététel leáll | Kvóta, retention, riasztás |
| Consumer lemarad | Növekvő késleltetés | Consumer lag monitorozás |
| Hibás séma | Feldolgozási hiba | Schema Registry és validáció |
| Duplikált esemény | Ismételt művelet | Idempotencia |
| NATS-kiesés | Kommunikáció megszakad | Helyreállítás és állapotellenőrzés |
| Valkey-kiesés | Átmeneti állapot hiánya | Fail-closed, JetStream megőrzés |
| Jogosulatlan hozzáférés | Adat- vagy műveleti incidens | ACL, TLS, audit |
| Replay hibás használata | Külső műveletek ismétlése | Replay-szabályok és elkülönítés |
| SDK-eltérés | Komponensek inkompatibilitása | Közös tesztvektorok |
| Túlzott eseményforgalom | Erőforrás-kimerülés | Backpressure és kvóták |
| Auditkapcsolat megszakad | Bizonyíthatóság sérül | Auditköteles műveletek leállítása |
| Hibás konfiguráció | Nem biztonságos működés | Konfigurációvalidáció |

### 23.1. Backpressure

A rendszernek szabályoznia kell az eseményterhelést.

Kötelező:

- Producer oldali limit;
- Consumer oldali párhuzamossági korlát;
- JetStream pending limit;
- maximális payloadméret;
- tárolási kvóta;
- túlterhelési állapot jelzése;
- ellenőrzött elutasítás.

A túlterhelés nem vezethet korlátlan memóriafelhasználáshoz.

## 24. Átadandó fejlesztési eredmények

A CFA4-EVENT-FABRIC-001 1.0 alapján az alábbi eredményeket kell létrehozni.

### 24.1. Forráskód és komponensek

| Azonosító | Átadandó eredmény |
|---|---|
| DEL-001 | Rust Event Fabric Core |
| DEL-002 | NATS Core + JetStream integráció |
| DEL-003 | Valkey integráció |
| DEL-004 | Canonical Event Envelope |
| DEL-005 | JSON Schema-katalógus |
| DEL-006 | Protobuf-definíciók |
| DEL-007 | Schema Registry |
| DEL-008 | Policy Enforcement |
| DEL-009 | Subject Router |
| DEL-010 | Consumer Manager |
| DEL-011 | Retry és DLQ |
| DEL-012 | Idempotency Manager |
| DEL-013 | Replay Manager |
| DEL-014 | Recovery Manager |
| DEL-015 | Rust SDK |
| DEL-016 | Python SDK |
| DEL-017 | C++ SDK |
| DEL-018 | TypeScript SDK |
| DEL-019 | Observability integráció |
| DEL-020 | Security és audit integráció |
| DEL-021 | Qt6 Control Panel integráció |
| DEL-022 | Telepítési és konfigurációs csomag |
| DEL-023 | Automatizált tesztcsomag |
| DEL-024 | Failure injection tesztcsomag |
| DEL-025 | Fejlesztői dokumentáció |

### 24.2. Kötelező dokumentáció

A fejlesztési eredmény része:

1. Architecture Specification.
2. Event Envelope Specification.
3. Event Naming Convention.
4. Subject Routing Specification.
5. Schema Registry Specification.
6. SDK API Documentation.
7. Security and ACL Specification.
8. Delivery and Retry Specification.
9. DLQ and Replay Specification.
10. Deployment Guide.
11. Operations Manual.
12. Recovery Manual.
13. Testing Specification.
14. Performance Benchmark Report.
15. CFA4 Integration Guide.

### 24.3. Fejlesztői átadás

A fejlesztőcsapatnak a terv alapján önállóan el kell tudnia kezdeni:

- a szolgáltatás infrastruktúrájának felépítését;
- a Rust Core implementációját;
- az eseményformátumok létrehozását;
- az SDK-k elkészítését;
- a biztonsági szabályok bevezetését;
- az integrációs teszteket;
- a Qt6 Control Panel bekötését.

A megvalósítás nem igényelheti más CFA4-komponensek felelősségi körének átvételét.

## 25. Végső elfogadási és kiadási kapu

Az Event Fabric nem tekinthető befejezettnek pusztán attól, hogy a NATS és Valkey működik, vagy az SDK-k elkészültek.

A teljes rendszernek együtt kell megfelelnie az alábbi feltételeknek.

### 25.1. Kötelező kiadási feltételek

**CFA4 Event Fabric – Release Gate, elfogadási ellenőrzőlista (12 tétel)**

1. F1–F8 valamennyi fejlesztési és elfogadási kapuja teljesült.
2. EF-001–EF-025 valamennyi követelménye PASS.
3. A négy SDK közös kompatibilitási tesztje PASS.
4. A biztonsági és ACL-tesztek PASS.
5. A failure injection tesztek PASS.
6. A JetStream tartós eseménykezelése igazolt.
7. Az idempotencia és a DLQ működése igazolt.
8. Az offline működés és a recovery igazolt.
9. A HRB és Model Router hatásköre változatlan maradt.
10. Az audit és Evidence Registry integráció igazolt.
11. A teljesítménycélok mérési jegyzőkönyve elkészült.
12. A dokumentáció és az üzemeltetési átadás teljes.

A lista a terv ellenőrzésére szolgál. A jelölések nem jelentenek tényleges fejlesztési vagy tesztelési bizonyítékot.

### 25.2. Végleges kiadási feltétel

A kiadás csak akkor fogadható el, ha:

- minden kötelező követelmény PASS;
- nincs nyitott kritikus vagy magas súlyosságú hiba;
- nincs ismert jogosultságmegkerülés;
- nincs igazolatlan eseményvesztési kockázat;
- a helyreállítási eljárás reprodukálható;
- az összes szükséges bizonyíték rendelkezésre áll;
- a teljes rendszer integrációja igazolt.

A kiadás eredménye egy egyértelmű PASS vagy FAIL állapot.

A részleges megfelelőség nem jelenthet végleges elfogadást.

## 26. A végleges architekturális döntések összefoglalása

| Terület | Rögzített döntés |
|---|---|
| Komponens | CFA4 Event Fabric |
| Azonosító | CFA4-EVENT-FABRIC-001 |
| Verzió | 1.0 |
| Alaparchitektúra | Centralized Event Fabric |
| Core | Rust |
| Élő kommunikáció | NATS Core |
| Tartós eseménykezelés | NATS JetStream |
| Átmeneti állapot | Valkey |
| Eseményformátum | Canonical Event Envelope |
| Sémák | JSON Schema + Protobuf |
| Kézbesítés | At-least-once |
| Duplikációkezelés | Idempotens feldolgozás |
| Hibakezelés | Retry + DLQ |
| Visszajátszás | JetStream Replay |
| SDK-k | Rust, Python, C++, TypeScript |
| Biztonság | TLS + ACL + fail-closed |
| Audit | CFA4 Security Governance |
| Bizonyítékkezelés | Evidence Registry |
| Erőforrás-kezelés | HRB |
| AI-routing | Model Router |
| GUI | Qt6 Control Panel |
| Működési modell | Offline-first, local-first |
| Fejlesztési fázisok | F1–F8 |
| Elfogadási követelmények | EF-001–EF-025 |

## 27. Architekturális lezárás

A CFA4 Event Fabric a CFA4 rendszer közös eseménykommunikációs gerince.

A megvalósítás során az alábbi határokat kötelező megőrizni:

1. Az Event Fabric közvetít, nem veszi át az üzleti komponensek döntési felelősségét.
2. A HRB marad az erőforrás-hozzáférés hatósága.
3. A Model Router marad az AI-modell-útválasztás hatósága.
4. A Security Governance marad a biztonsági szabályozás hatósága.
5. Az Evidence Registry marad a bizonyítékok nyilvántartásának hatósága.
6. A JetStream marad a tartós eseménykezelés alapja.
7. A Valkey kizárólag átmeneti állapot- és gyorsítótárolási szerepet kap.
8. A rendszer alapvető működése internetkapcsolat nélkül biztosított.
9. A kritikus műveletek minden esetben fail-closed módon működnek.
10. A végleges elfogadás fizikai, integrációs és reprodukálható tesztbizonyítékokon alapul.

A dokumentum a fejlesztés megkezdéséhez szükséges architekturális feladatokat, komponenseket,
interfészeket, biztonsági követelményeket, tesztelési feladatokat és elfogadási kritériumokat rögzíti.

## Jóváhagyás

**JÓVÁHAGYVA** (tulajdonos: ubuntuokos, 2026-11-02)

A CFA4-EVENT-FABRIC-001 1.0 megvalósítási terv jóváhagyott alapdokumentum.

A további munkában:

1. Az 1.0 architektúra és követelményrendszer az irányadó.
2. A fejlesztés az F1–F8 fázisok szerint történik.
3. Az EF-001–EF-025 követelmények kötelezők.
4. A HRB, Model Router, Security Governance és Evidence Registry felelősségi határai változatlanok.
5. A végleges PASS kizárólag igazolt teszteredmények alapján adható ki.

A terv jóváhagyása rögzítve. Az implementáció még nem kezdődött meg.
