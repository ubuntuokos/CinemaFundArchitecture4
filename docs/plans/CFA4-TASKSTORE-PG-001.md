# CFA4 PostgreSQL-adatréteg a feladatsorhoz és az AI-jobtárhoz (CFA4-TASKSTORE-PG-001)

> **Állapot:** ELFOGADVA (István, 2026-11-07); KÓD MÉG NINCS · **Utolsó átnézés:** 2026-11-07 ·
> **Változás:** major (első változat; a tartós jobtár redb helyett PostgreSQL; a Media Intelligence és a pgvector a tervben, István, 2026-11-07) · **Beépítve:**
> [CFA4-AI_modules ADR 0006](https://github.com/ubuntuokos/CFA4-AI_modules/blob/main/docs/adr/ADR-0006-ai-processing-layer.md) F1, F2, F4 és F5 kapu · **Kapcsolódik:** CFA4
> [`ADR 0006`](../adr/ADR-0006-backlothub-postgresql.md) (BacklotHub: PostgreSQL, 2026-11-02),
> [`CFA4-EVENT-FABRIC-001`](CFA4-EVENT-FABRIC-001-1.0.md) és [illesztési melléklete](CFA4-EVENT-FABRIC-001-1.0-CFA4-ILLESZTES.md),
> CFA4-ADAPTIVE-CAPABILITY-001 (adaptív erőforrás-kezelés, tulajdonosi specifikáció, 2026-11-07)

## 1. Kiindulás

- **Van PostgreSQL-döntés, de csak a BacklotHubra.** A CFA4 fő repójának
  `docs/adr/ADR-0006-backlothub-postgresql.md` döntése (elfogadva 2026-11-02): a BacklotHub
  adattárolója minden működési módban, az egygépes M1-ben is a PostgreSQL, az Ubuntu `postgresql`
  rendszercsomagjából, saját klaszterrel, UUIDv7 azonosítókkal, offline-first. A K17
  nyilvántartást kifejezetten nem érinti, a kliensmeghajtóról a BacklotHub G2 kapuja dönt.
- **A feladatsorra, az ütemezésre, az AI-jobokra és a Media Intelligence-re nem volt
  PostgreSQL-terv.** A kódban adatbázis-crate sincs. Ez a dokumentum ezt a hiányt pótolja.
- **Az Event Fabric** a tartós eseményeket JetStreamben, az átmeneti állapotot Valkeyben tartja; a
  kritikus eseményekhez kimenőtár (outbox) kötelező (Q9), a feladatéletciklus eseményei
  `cfa4.execution.job.*` néven futnak, entitásonkénti sorszámmal (Q10). Az illesztési melléklet
  E1 pontja szerint a CFA4-nek nincs erőforrás-hatósága; a tulajdonos 2026-11-06-án úgy döntött,
  hogy ehhez és az Execution Fabrichoz előbb terv kell (Q1).
- **Az AI-terv** eddig redb-t írt a jobtárnak, a tervdokumentum SQLite-ot a gyorsítótár-indexnek.
  Az adaptív erőforrás-kezelési specifikáció 10. fejezete a meglévő PostgreSQL használatát és az
  új adatbázismotor tilalmát írja elő, ezért a redb és az SQLite kikerül.

## 2. Döntés

1. **Egyetlen CFA4 PostgreSQL-klaszter** a gépen: ugyanaz, amelyet a BacklotHub ADR bevezet.
   Minden modul saját sémát és saját adatbázis-szerepet kap benne (`tasks`, `ai`, `cache`, később
   `mi`, `backlot`). Nincs második adatbázismotor.
2. **Ami PostgreSQL-be kerül:** a közös feladatsor és ütemezés, a feladatok állapotnaplója,
   ellenőrzőpontjai, becslései és futási előzményei; az AI-jobok, a licencelfogadások és a
   modellkezelő rekordjai; a tartalomcímzett gyorsítótár indexe; az Event Fabric kimenőtára; a
   Media Intelligence indexe (átiratok, címkék, vektorok).
3. **Ami nem kerül bele:** képkockák, maszkok, média és modellsúlyok (ezek a lemezen maradnak,
   tartalomcímzett fájlként); az eseménytörténet (JetStream); az átmeneti állapot (Valkey); a
   projektfájl (a CFA4 saját formátuma marad).
4. **A Media Intelligence** (médiaindex, átirat- és vektoros keresés) ugyanebbe a klaszterbe kerül,
   saját `mi` sémával és a pgvector bővítménnyel (4. pont, PG-4). A tulajdonos 2026-11-07-én úgy
   döntött, hogy ez most kerüljön a tervbe, ne később. A K17 nyilvántartás külön döntéssel
   csatlakozhat.
5. **Közös klaszter a BacklotHubbal**, külön sémákkal és szerepekkel (tulajdonosi döntés,
   2026-11-07).
6. **Futó PostgreSQL kell az AI-funkciókhoz és a feladatsorhoz** akkor is, ha a felhasználó nem
   használja a BacklotHubot. A `.deb` függőségként hozza; az AppImage, a Flatpak és a `tar.gz` a
   rendszerre telepítettet használja. Ha hiányzik, a vágás, a lejátszás és a projektmentés
   változatlanul működik, a feladatsor és az AI pedig kezelhető hibaüzenetet ad a telepítés
   módjával (`AGENTS.md` §0). (Tulajdonosi döntés, 2026-11-07.)

A tiszta Rust követelmény nem sérül: a PostgreSQL külön folyamat (licence PostgreSQL License,
GPL-3.0-kompatibilis), a CFA4 csak kliensként kapcsolódik hozzá, tiszta Rust meghajtóval.

## 3. Sémavázlat

Minden azonosító UUIDv7, kliensoldalon generálva. Minden idő `timestamptz`, UTC-ben tárolva.

| Tábla | Tartalom |
|---|---|
| `tasks.task` | azonosító, típus (AI-videó, AI-hang, indexelés, modellbetöltés, proxy, render, kodekkonverzió), modul, projektkapcsolat, állapot, felhasználói prioritás (alacsony, normál, magas), rendszerosztály (P0/P1/P2/R), létrehozás ideje, paraméterek (`jsonb`), erőforrásigény, végrehajtási mód, megszakíthatóság, folytathatóság, kísérlet száma, futási bérlet (`lease_owner`, `lease_until`), verziószám |
| `tasks.schedule` | indítási mód (azonnal, erőforrásra, időpontra, időpontra és erőforrásra), tervezett indítás, IANA-időzóna neve, erőforrásfeltétel, elmaradt indítás szabálya (késleltetve, következő alkalmas időpont, várjon a felhasználóra) |
| `tasks.transition` | csak hozzáfűzhető állapotnapló: feladat, sorszám, előző és új állapot, idő, ok, ki (felhasználó, ütemező, worker) |
| `tasks.checkpoint` | elkészült szakaszok: feladat, szakasz sorszáma, gyorsítótár-kulcs, idő |
| `tasks.estimate` | becslések a futás előtt és közben: alsó és felső határ, megbízhatóság, alap (mérés, előzmény, manifest) |
| `tasks.run_history` | tényleges futások: implementáció, végrehajtási útvonal, hardverujjlenyomat, bemenetméret, időtartam, csúcs-RAM és -VRAM; ebből tanul a becslő |
| `tasks.outbox` | Event Fabric kimenőtár: `cfa4.execution.job.*` események feladatonkénti sorszámmal |
| `ai.job` | az AI-rész egy feladathoz: képesség, implementáció, a `route_decision`, az Előnézet és az Alkalmazás projektverziója |
| `ai.license_acceptance` | implementáció, súlyhash, projekt, elfogadás ideje, kereskedelmi figyelmeztetés |
| `ai.model`, `ai.model_use` | modellrekord: SHA-256, család, revízió, változat, formátum, forrás, a négy licencrész, elérési út, állapot; és hogy mely képességek használják |
| `cache.entry` | a tartalomcímzett gyorsítótár indexe: kulcs, fájl, méret, utolsó használat (LRU), szint (előnézet vagy végleges) |

**Állapotok:** a CFA4-SDK `host.schema.json` jobállapotai (`queued`, `preparing_model`,
`waiting_for_resources`, `loading`, `running`, `cancelling`, `succeeded`, `failed`, `cancelled`,
`blocked`) és az adaptív specifikáció további állapotai (`created`, `scheduled`, `ready`,
`paused`), amint az SDK gazdajelentése új változatban felveszi őket. A felület a magyar
elnevezéseket mutatja.

## 4. Media Intelligence: `mi` séma és pgvector

**Mire való:** a projekt médiájának helyi, kereshető indexe. A felhasználó szöveggel vagy képpel
kereshet jelenetet, felvételt, elhangzott mondatot, címkét („kutya a parton”, „ahol X azt mondja,
hogy…”), és a találat egy klip adott szakaszára mutat. Az index a CFA4 AI-képességeinek kimenetéből
épül (átirat, jelenet- és csendfelismerés, címkézés, kép- és szövegbeágyazás), a feladatsoron át,
háttérfeladatként.

**Bővítmény:** pgvector, az Ubuntu `postgresql-<verzió>-pgvector` rendszercsomagjából (licence
PostgreSQL License, GPL-3.0-kompatibilis), ugyanúgy, ahogy maga a PostgreSQL. Ha hiányzik, a
vektoros keresés kezelhető hibát ad, az átiratkeresés és a többi funkció működik.

| Tábla | Tartalom |
|---|---|
| `mi.asset` | indexelt médiafájl: tartalomhash (nem az elérési út azonosít), utolsó ismert elérési út, időtartam, a projektkapcsolatok, az indexelés állapota |
| `mi.segment` | szakasz egy assetben: kezdet és vég `cfa4_time::Tick`-ben (egész szám, nem lebegőpontos másodperc), fajta (felvétel, jelenet, beszéd, csend, zene) |
| `mi.transcript` | átiratszakasz: szöveg, BCP 47 nyelvcímke, beszélő, a keresőindexhez `tsvector` |
| `mi.tag` | címke egy szakaszon: címke, megbízhatóság, a címkéző implementáció és modellje |
| `mi.embedding_model` | beágyazó modell: implementáció, a súly SHA-256-ja (`ai.model`), dimenzió, távolságmérték, modalitás (kép, szöveg, hang) |
| `mi.embedding` | vektor egy szakaszhoz: szakasz, beágyazó modell, a vektor (`vector`, félpontosságnál `halfvec`) |

**Indexek:** modellenként külön részleges HNSW-index (`WHERE model = …`, a dimenzióra kasztolva),
mert a modellek dimenziója eltér, és egy index csak egy dimenziót kezel. Teljes szöveges keresés
`tsvector` + GIN indexszel, nyelvfüggetlen (`simple`) konfigurációval, mert egy film több nyelvet is
keverhet.

**Szabályok:**

- **Modellcsere:** a vektor mindig a beágyazó modell súlyhash-éhez tartozik. Új modellnél az
  újraindexelés új sorokat ír, a régi modell vektorai a régi indexszel kereshetők maradnak, amíg a
  felhasználó nem törli őket. Különböző modellek vektorai sosem keverednek egy keresésben.
- **Indexelés feladatként:** az indexelés a `tasks` sorban fut, P2 (spekulatív, üresjáratban) vagy
  a felhasználó kérésére P1 osztályban; szakaszokra bontva, ellenőrzőponttal, így megszakítás és
  újraindítás után ott folytatódik, ahol abbamaradt, és kétszer nem indexel.
- **Erőforrás:** a HNSW-index építése és a nagy beszúrási kötegek memóriáját
  (`maintenance_work_mem`, kötegméret) az erőforrás-kezelő kerete szabja meg, a 16 GB-os
  minimumgépen is; az indexépítés a többi feladattal együtt ütemeződik, nem mellettük.
- **Helyi adat:** az index soha nem hagyja el a gépet (VISION 2.3). Ha egy asset kikerül minden
  projektből, a felhasználó törölheti a hozzá tartozó indexet; a törlés minden szakaszra, átiratra,
  címkére és vektorra kiterjed.
- **Arcok és személyek:** arcbeágyazás biometrikus adat. Csak kifejezett, projektenkénti
  bekapcsolással indexelhető, alapból ki van kapcsolva, és egy lépésben törölhető; a részletes
  hozzájárulási szabály az arccserére (38. típus) vonatkozó nyitott kérdéssel együtt dől el.
- **Beágyazó modellek** ugyanúgy a felhasználó letöltésével és egyszeri licencelfogadással
  kerülnek a gépre, mint minden más modell; a CFA4 nem terjeszt modellt.

## 5. Kétszeri indítás elleni védelem és idempotencia

- **Indítás egyetlen tranzakcióban:** az ütemező `FOR UPDATE SKIP LOCKED`-del veszi ki a következő
  indítható feladatot, és ugyanabban a tranzakcióban állítja futóra, írja a bérletet, a naplósort
  és a kimenő eseményt. Ha a tranzakció nem véglegesül, a feladat nem indult el.
- **Bérlet:** a futó feladat bérletét a felügyelő rövid időközönként megújítja. CFA4- vagy
  rendszer-újraindítás után a lejárt bérletű feladat a folytathatóságától függően az utolsó
  ellenőrzőponttól folytatódik vagy `failed` lesz; magától sosem indul el másodszor elejéről.
- **Verziószám:** minden állapotváltás a várt verziószámra feltételes; elavult frissítés nem
  írhatja felül az újabbat (Event Fabric Q10).
- **Idempotens szakaszok:** a szakaszeredmény tartalomcímzett fájl. Ha egy szakasz kétszer fut,
  ugyanaz a fájl lesz belőle, és a második futás a gyorsítótárból válaszol.

## 6. Ütemezés az adatbázisban

- A tervezett indítás UTC-ben és az IANA-időzóna nevével együtt tárolódik. A falióra szerint
  megadott időpontot időzóna- vagy nyári idő váltásakor az ütemező újraszámolja.
- Az ütemező a legközelebbi indításig alszik, és `LISTEN/NOTIFY` ébreszti, ha új vagy módosított
  feladat jön.
- Induláskor az ütemező megkeresi az elmaradt indításokat, és a feladat saját szabálya szerint jár
  el. A gépet nem ébreszti fel.

## 7. Ha az adatbázis nem érhető el

- **Új feladat nem indul**, mert az indítás tranzakcióhoz kötött; ez véd a kétszeri indítás ellen.
- **A futó feladat továbbfut** a következő szakaszhatárig, az eredmény a lemezre, a gyorsítótárba
  kerül. Ha a kapcsolat visszajön, a felügyelő a gyorsítótár alapján pótolja a naplót; ha nem jön
  vissza a bérlet lejárta előtt, a feladat a szakaszhatáron megáll.
- **A vágás, a lejátszás és a projektmentés** nem függ az adatbázistól.
- A felület kezelhető hibát mutat; a CFA4 nem omlik össze, és nem próbálkozik végtelenül
  ugyanazzal.

## 8. Erőforrások a minimumgépen

- A PostgreSQL saját RAM-ja a CFA4 erőforrás-keretébe számít, az erőforrás-kezelő a tartalék
  számításánál levonja.
- Kiinduló beállítás a 16 GB-os minimumgépre: kevés kapcsolat (a CFA4 kapcsolatkészlettel
  dolgozik), kis `shared_buffers`. A pontos értékeket mérés dönti el a `run-minspec.sh` 16 GB-os
  (minimum) és 32 GB-os (ajánlott) profilján.
- A telepítés és az indítás módja (felhasználói vagy rendszerszolgáltatás) a BacklotHub G2 kapuján
  dől el; ez a terv ugyanazt használja.

## 9. Rust-oldal

- Új, felületfüggetlen crate: `cfa4-store`. Ez adja a kapcsolatkészletet, a migrációkat és a
  típusos lekérdezéseket. A feladatsor és a `cfa4-ai-jobs` ezen át éri el az adatbázist, más crate
  közvetlenül nem.
- Meghajtó: tiszta Rust (`tokio-postgres` vagy `sqlx`), MIT/Apache-licenccel, `cargo deny`
  ellenőrzéssel; a végleges választás a BacklotHub G2 kapujával közös, hogy egy meghajtó legyen.
- A `cfa4-store` L2-es crate, felület nélkül; a rétegtáblába (`cargo xtask layers`) a létrehozásakor
  kerül be. A `cargo xtask wasm` a CFA4 fő repójában már nem kapu (a WASM nem CFA4-cél), ezért a
  meghajtó natív függősége nem akadály.
- Migráció: egy út minden módban, egy verzió visszafelé, valós méretű adaton tesztelve.

## 10. Tesztek

- Kétszeri indítás: a folyamat leölése az indítási tranzakció minden pontján, utána újraindítás;
  a feladat legfeljebb egyszer fut.
- Bérlet lejárta, adatbázis-újraindítás futó feladat közben, a kapcsolat megszakadása.
- Ütemezés: pontos indítás, elmaradt indítás kikapcsolt gép után, időzóna- és nyári idő váltás.
- Kimenőtár: összeomlási ablak az állapotváltás és a közzététel között (Event Fabric Q9),
  átrendezett kézbesítés (Q10).
- Migráció oda-vissza; RAM-mérés a 16 és a 32 GB-os profilon.
- Media Intelligence: keresés helyessége ismert mintakészleten (az elvárt szakasz a legjobb találatok között), modellcsere utáni elkülönülés, megszakított és folytatott indexelés, asset törlése mindenhonnan, hiányzó pgvectorra kezelhető hiba.
- Ellenséges bemenet: hibás `jsonb`-paraméter, túlméretes mező, érvénytelen időzóna; hiba, pánik
  nélkül (`AGENTS.md` §0).
- A tesztek helyi, eldobható PostgreSQL-példánnyal futnak (`pg_ctl` ideiglenes könyvtárban); ha a
  gépen nincs PostgreSQL, a tesztek ezt jelzik és kimaradnak, a gyors CI-kapu ettől zöld marad.

## 11. Fázisok és beépítés az AI-rétegbe

| Fázis | Tartalom | Kilépés | AI-kapu |
|---|---|---|---|
| PG-0 | Ez a terv | Tulajdonosi jóváhagyás (megvan, 2026-11-07) | F0 (dokumentáció) |
| PG-1 | `cfa4-store`: kapcsolat, migráció, hibakezelés; közös a BacklotHub G2 kapujával | Migráció oda-vissza; hiányzó vagy leállt PostgreSQL-re kezelhető hiba, pánik nélkül | F1 |
| PG-2 | `tasks` séma: indítás, bérlet, ütemezés, kimenőtár | A 10. pont kétszeri indítási és ütemezési tesztjei zöldek | F2 |
| PG-3 | `ai` és `cache` séma: AI-jobok, licencelfogadás, modellrekordok, gyorsítótár-index | A job összes állapota és a licencelfogadás az adatbázisban; CFA4-újraindítás után nincs elveszett vagy kétszer indult job | F2 |
| PG-4 | `mi` séma és pgvector (4. pont): médiaindex, átirat- és vektoros keresés | Az első beágyazó és átiratképesség kimenete indexelődik és kereshető; a 10. pont Media Intelligence-tesztjei zöldek | F4–F5 (az első kép- és szövegbeágyazó, illetve a candle-es átirat mellett) |

## 12. Hatás más tervekre

- **AI ADR 0006:** a tartós jobtár redb helyett PostgreSQL, a gyorsítótár-index SQLite helyett
  PostgreSQL; az F1 és az F2 kapu a PG-1…PG-3, az F4 és az F5 kapu a PG-4 fázist tartalmazza
  ([CFA4-AI_modules #31](https://github.com/ubuntuokos/CFA4-AI_modules/pull/31) átvezetve).
- **CFA4 Event Fabric illesztés:** az E1 (erőforrás-hatóság) és az E3 (Execution Fabric) erre a
  tervre és az adaptív erőforrás-kezelési specifikációra hivatkozhat.
- **CFA4 BacklotHub ADR 0006:** kiegészül azzal, hogy a klasztert a feladatsor, az AI és a Media
  Intelligence is használja, saját sémával; ez a terv a CFA4 fő repójába is bekerül
  (ez a fájl). Azonos tartalmú példánya a CFA4-AI_modules repóban van
  ([`docs/ai/postgresql-adatreteg.md`](https://github.com/ubuntuokos/CFA4-AI_modules/blob/main/docs/ai/postgresql-adatreteg.md)); eltérés esetén ez a
  példány az irányadó.
- **ADR-számozás:** az AI-repó ADR 0006-ja és a CFA4 fő repójának ADR 0006-ja (BacklotHub) azonos
  számot visel, és az AI-ADR „0004 FFI-kivétel, 0005 `cfa4-route`” hivatkozása sem egyezik a fő repó
  mai állapotával (0004 a `cfa4-route`, 0005 a médiahátterek, 0007 a Media Services). Az
  átszámozás külön változtatás.
