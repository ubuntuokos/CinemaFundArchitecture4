# ADR 0008 – Közös CFA4-auditnapló (CFA4-AUDIT-001)

**Dátum:** 2026-11-08 · **Állapot:** elfogadva (a tulajdonos, `ubuntuokos`, 2026-11-08; BacklotHub #40 és #41) ·
**Terv:** [`docs/plans/CFA4-AUDIT-001-1.0.md`](../plans/CFA4-AUDIT-001-1.0.md) · **Érintett rész:** új Python-csomag
(`cfa4_audit`) és új Rust crate (`cfa4-audit`) az AU-1 fázisban; ez az ADR kódot nem változtat

## Háttér

A BacklotHub (CFA4-BACKLOTHUB-001) 1. lépcsője az Audit Trail: minden későbbi modul ide írja, ki mit
változtatott. A terv 11. pontja szerint „a közös naplózás és audit a CFA4 megfelelő szolgáltatásához
kapcsolódjon”. Ilyen szolgáltatásnak a CFA4-ben nem volt terve; a motor `EventLog`-ja és az alkalmazásnaplók
diagnosztikai naplók, nem auditnaplók. Az Event Fabric terve (CFA4-EVENT-FABRIC-001 12.4) is auditnaplót vár
(közzététel, elutasítás, jogosultságmódosítás, DLQ, replay).

A szabály szerint nem létező közös szolgáltatásra modul nem épülhet, stubbal sem. Ezért előbb a közös
auditnaplót kell megtervezni és megvalósítani.

## Döntés

1. **Egy közös CFA4-auditnapló készül**, nem BacklotHub-saját napló. Első fogyasztói a BacklotHub és az
   Event Fabric (EF Audit Adapter).
2. **Tárolás PostgreSQL-ben** (ADR 0006), saját `cfa4_audit` sémában, javaslat szerint az alkalmazás
   adatbázisán belül. Az auditesemény **ugyanabban a tranzakcióban** íródik, mint az üzleti változás: vagy
   mindkettő megtörténik, vagy egyik sem (fail-closed).
3. **Csak hozzáfűzés, kikényszerítve:** az alkalmazás szerepe csak `INSERT` és `SELECT` jogot kap; trigger
   utasít el minden `UPDATE`, `DELETE` és `TRUNCATE` műveletet; a sémát külön tulajdonosi szerep birtokolja.
4. **Manipuláció kimutatása:** eseményenkénti SHA-256 hash-lánc (`seq`, `prev_hash`, `hash`) rögzített
   kanonikus szerializálással, időszakos ellenőrzőponttal, és `audit.verify` paranccsal.
5. **Szereplő:** M1-ben `actor_kind = os_user`, a helyi Linux-felhasználó uid-ja és neve a folyamat saját
   hitelesítő adataiból (#41). M2-től `identity`, a CFA4 Identity szolgáltatással, amelyhez külön terv kell.
   Ügynök vagy automatizmus `agent` / `system`, szükség esetén `on_behalf_of` mezővel.
6. **Személyes adat nem kerül a láncba** (SEC-06, GDPR): a részletek külön, nem láncolt táblában vannak;
   törlési kérésnél csak ez törlődik, a lánc ép marad, és `TOMBSTONE` esemény rögzíti a törlést.
7. **Két nyelvű könyvtár, egy séma:** Python (`cfa4_audit`, a BacklotHub üzleti logikájához) és Rust
   (`cfa4-audit`, az Event Fabric Rust Core-hoz és a motorhoz), közös tesztvektorokkal, hogy mindkettő
   ugyanazt a kivonatot adja. A BacklotHub Python-üzleti logikája a tulajdonos 2026-11-02-i döntésén alapul
   (komponensenként az a nyelv, amelyben a megbízható működés ésszerűbben garantálható); az `AGENTS.md`
   Python-szabályának ehhez igazítása külön PR.
8. **Minden egy parancs:** `audit.list`, `audit.verify`, `audit.export` CFA4-parancsként (CLI, MCP, felület).
9. **Hatásköri határ:** az auditnapló nem jogosultsági hatóság, nem munkafolyamat-motor, nem eseménybusz és
   nem biztonsági mentés. Az Event Fabric nem auditnapló, és az auditnapló sem eseménybusz.
10. **Never crash** (`AGENTS.md` §0): sérült lánc, hiányzó partíció vagy teli lemez kezelhető hibát ad angol
    hibakóddal (`AUDIT_CHAIN_BROKEN`, `AUDIT_WRITE_DENIED`, …); a felület fordít.

## Fázisok

| Fázis | Tartalom | Kapu |
|---|---|---|
| AU-0 | Ez az ADR és a terv a `main`-ben | G2 |
| AU-1 | Séma, migráció, csak-hozzáfűzés kényszerítése, kanonikus szerializálás, közös tesztvektorok | G4 |
| AU-2 | `record` / `list` / `verify` / `export`, CFA4-parancsok | G4 |
| AU-3 | Manipuláció-, összeomlás-, mutációs fuzz- és visszaállítási tesztek; 50 M eseményes mérés a `CFA4-MIN` profilon | G5 |
| AU-4 | Archívum és GDPR-tombstone | G5 |
| AU-5 | BacklotHub Audit Trail felület (Qt6/PySide6) | BacklotHub 1. lépcső G4–G6 |

## Következmények

- A BacklotHub minden lépcsője és az Event Fabric ugyanarra a naplóra épül; nincs két igazságforrás.
- Az auditköteles művelet nem hajtható végre, ha a napló nem írható.
- A Python- és a Rust-könyvtár szerződését a közös tesztvektorok kötik; a formátum változásához verzióemelés kell.
- A CFA4 Identity terve az M2 előfeltétele.

## Kapcsolódó dokumentumok

- [`docs/plans/CFA4-AUDIT-001-1.0.md`](../plans/CFA4-AUDIT-001-1.0.md) – a részletes terv
- ADR 0006 (BacklotHub: PostgreSQL), ADR 0007 (Media Services)
- [`docs/plans/CFA4-EVENT-FABRIC-001-1.0.md`](../plans/CFA4-EVENT-FABRIC-001-1.0.md) 12.4 és a CFA4-melléklet
