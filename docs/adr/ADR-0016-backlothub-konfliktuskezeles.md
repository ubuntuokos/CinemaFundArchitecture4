# ADR 0016 – BacklotHub: egyidejű módosítás és konfliktuskezelés (M1, M2/M3)

**Dátum:** 2026-11-21 · **Állapot:** elfogadva (a tulajdonos, `ubuntuokos`, 2026-11-21; BacklotHub nyitott kérdések #3 és #5,
az „SQL/offline elemzés” 5. és 6. pontja) · **Érintett rész:** a BacklotHub (CFA4-BACKLOTHUB-001) adatrétege és
műveletei; ez az ADR kódot nem változtat. Az M1 szabályai már érvényes G2-kapukban vannak, az M2/M3 részét az M2
bevezetésekor kell megvalósítani.

## Háttér

[ADR 0006](ADR-0006-backlothub-postgresql.md): a BacklotHub adattárolója minden működési módban PostgreSQL,
UUIDv7-azonosítókkal, offline-first elvvel. Az M1 egygépes, az M2 helyi hálózat, az M3 nagy telepítés; az indulás M1.
Eddig nem volt leírva, mi történik, ha két módosítás ugyanazt a rekordot éri, vagy ha egy offline gép változtatásai
később érkeznek meg. Az elemzés három irányt vetett össze (csak online írás; PostgreSQL + helyi másolat +
műveletnapló; CRDT). A tulajdonos a műveletnapló-alapú irány szabályait fogadta el, az ADR 0006 szerinti
PostgreSQL-lel mindenhol.

## Döntés

### 1. Alap (M1-ben is érvényes): optimista zárolás

1. Minden módosítható rekordnak van `version bigint` mezője. Az írás a látott verzióhoz kötött
   (`UPDATE … WHERE id = $1 AND version = $2`); találat hiányában a művelet `CONFLICT_STALE_VERSION` hibával
   elutasul, és semmi nem íródik felül. Ez a Production Management G2-ben (G2-02) már így van, és minden modul
   ugyanezt használja.
2. A hibakód angol, a felület fordítja (hu/en). Csendes felülírás nincs.
3. Hierarchia- és függőségi szabályok (pl. feladatfüggőségi kör tiltása) szerveroldali, tranzakción belüli
   ellenőrzéssel és tanácsadó zárral érvényesülnek (G2-05), nem kliensoldali ellenőrzéssel.
4. Törlés logikai (`deleted_at`); törlés és módosítás ütközése mindig ütközés, nem csendes eredmény.

### 2. Műveletnapló (M2-től kötelező, M1-ben már így kell tervezni)

1. Minden módosítás egy **művelet**: `op_id` (UUIDv7, kliensoldalon), `entity_id`, `base_version`, mezőszintű
   változás, szereplő, idő. A szerver az `op_id` alapján idempotensen fogadja: az ismétlés ugyanazt a nyugtát adja.
2. A művelet ugyanabban a tranzakcióban kerül az üzleti rekordhoz és a közös auditnaplóhoz
   ([ADR 0008](ADR-0008-cfa4-audit.md)), így az audit és a visszakövethetőség ugyanabból a forrásból jön.
3. A szerver a változásokat monoton `server_seq` sorszámmal adja vissza a klienseknek.

### 3. Konfliktusszabály rekordtípusonként

| Eset | Szabály |
|---|---|
| Ugyanazon rekord **különböző mezői** | Mindkét művelet érvényes, a szerver mezőszinten összefésüli őket. |
| Ugyanaz a **puha** mező (leírás, megjegyzés, címke) | A szerver-sorrend szerinti utolsó nyer; a másik érték az előzményben marad, a felhasználó értesítést kap. Halmazjellegű adatnál (címkék, hozzárendelt személyek) halmaz-összefésülés. |
| **Kemény** adat: státusz, jóváhagyás, publikálás, átadás, költség, foglalás | **Fail-closed:** elavult `base_version` esetén a művelet „Ütközik” állapotba kerül, semmi nem íródik felül, a felhasználó összehasonlító nézetben dönt. |
| Erőforrás-foglalás (személy, eszköz, helyszín) | Szerveroldali egyediségi szabály; offline csak ideiglenes foglalás. |
| Törlés és módosítás | Mindig kézi döntés. |

### 4. Offline működés (M2/M3; tulajdonosi döntés 2026-11-21)

1. **Minden művelet mehet offline**, a jóváhagyás, a publikálás és az átadás is. Ezek az offline ideig
   **ideiglenesek**: a felület „függő” jelzéssel mutatja őket, és csak akkor lépnek életbe, ha a szerver
   visszakapcsoláskor érvényesíti őket.
2. A szerver elutasítja az ideiglenes jóváhagyást vagy publikálást, ha közben a hivatkozott verzió megváltozott
   (a jóváhagyás mindig konkrét verzióhoz kötött), a művelet állapota már nem engedi, vagy a szereplő jogosultsága
   megszűnt. Az elutasított művelet nem vész el: „Ütközik” állapotban, a döntési nézetben marad, az audit rögzíti.
3. **Jogosultság offline:** a kliens a legutóbb szinkronizált jogosultságokkal dolgozik; a szerver visszakapcsoláskor
   minden művelet jogosultságát újraellenőrzi. Az offline időszak maximális hossza és a jogosultság lejárata a
   CFA4 Identity tervének része (az M2 előfeltétele, [ADR 0008](ADR-0008-cfa4-audit.md) 5. pont).
4. **Helyi másolat:** az offline kliens helyi másolatának tárolása (a szerver ugyanazon PostgreSQL-sémájának
   helyi példánya vagy más) az M2 G2 kapuján dől el; az ADR 0006 szerinti PostgreSQL-döntést ez nem érinti, és
   második adatbázismotor nem kerül be (ADR 0006, 1. pont, TASKSTORE kiegészítés).

### 5. Nem választott irányok

- **Csak online írás, offline csak olvasás:** elvetve; a tulajdonos minden művelet offline elérhetőségét kéri.
- **CRDT az egész adatmodellre:** elvetve. A státusz, a jóváhagyás, a költség és a foglalás nem
  összefésülhető adat, a CRDT itt csendben rossz eredményt adhat. Később legfeljebb szűk területen (közös
  szövegszerkesztés, pl. Script Breakdown vagy review-megjegyzések) mérlegelhető, külön ADR-rel és licencvizsgálattal.

## Következmények

- M1-ben nincs új kód: a `version` mező és a `CONFLICT_STALE_VERSION` már a G2-kapukban van. A műveletnapló
  sémáját az M1 modulok G2-jén úgy kell megtervezni, hogy az M2 ne igényeljen sémaátírást (`op_id`, `base_version`).
- Az M2 bevezetése előtt kell: CFA4 Identity terv, a helyi másolat módja, az offline időszak korlátja,
  a hálózati mentési cél (RPO/RTO, nyitott kérdés #13), és a konfliktusnézet felületterve.
- A kemény adatok ideiglenes offline állapota miatt a Review, a Publisher és a Delivery G2-jében ezt az állapotot
  fel kell venni az állapotgépbe az M2 lépcsőn.
- A szabályok tesztelése: két párhuzamos módosítás (különböző és azonos mező), offline-ból késve érkező
  jóváhagyás elavult verzióra, ismételt `op_id`, megszűnt jogosultság; valós méretű adaton.

## Kapcsolódó dokumentumok

- [ADR 0006](ADR-0006-backlothub-postgresql.md) – PostgreSQL minden módban; [ADR 0008](ADR-0008-cfa4-audit.md) – közös auditnapló
- [`docs/open-editors-parity.md`](../open-editors-parity.md) E1/E2 – revízióhoz kötött commit, ág és összefésülés (a minta)
- A BacklotHub-elemzések (SQL/offline elemzés, nyitott kérdések) a projekt munkaanyagai.
