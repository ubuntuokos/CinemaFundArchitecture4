# CFA4-FILE-SERVICES-001 – Közös CFA4 fájlszolgáltatás (1.0)

**Dátum:** 2026-11-08 · **Állapot:** a tulajdonos (`ubuntuokos`) jóváhagyta 2026-11-08-án (BacklotHub #43: a terv
a 8., 14. és 26. lépcső előfeltétele; #45: Rust adatút, Python katalógus azonos tranzakcióban, BLAKE3; #14: a médiatár
mentése ebben a tervben). Döntés: [ADR 0009](../adr/ADR-0009-cfa4-file-services.md).
**Előzmény:** CFA4-BACKLOTHUB-001 4.4, 4.20, 11.; a BacklotHub 5. és 8. lépcső G1-forrásvizsgálata (projekt-munkaanyag)

## 1. Cél és hatókör

A CFA4 közös szolgáltatása a produkciós fájlok **fizikai tárolására**. A BacklotHub terve (11. pont) szerint
„a fájlok fizikai tárolása a CFA4 File Services feladata”. A BacklotHub csak hivatkozik a fájlra, maga nem
másol, nem nevez át és nem töröl a lemezen.

Feladatai:
- tárolási gyökerek (roots) kezelése: hol vannak a munka-, publikált, review-, csatolmány- és archívumfájlok;
- fájlkatalógus: minden kezelt fájl stabil azonosítóval (UUIDv7), gyökérfüggetlen relatív úttal, mérettel,
  kivonattal;
- biztonságos írás (ellenőrzött másolás, atomikus véglegesítés);
- lezárás: a publikált fájl ezután nem módosulhat;
- épségellenőrzés és újrakapcsolás kivonat alapján;
- törlés és végleges törlés (jóváhagyással);
- a médiatár mentése és visszaállítása (#14).

**Nem része:**
- médiafeldolgozás (proxy, bélyegkép, dekódolás): a Media Services feladata (ADR 0007);
- a verziók jelentése (melyik fájl melyik asset melyik verziója): a BacklotHub adata;
- felhős szinkron és külső tárhely: nem cél (SEC-02: adat a felhasználó engedélye nélkül nem hagyja el a
  gépet);
- elosztott fájlrendszer.

## 2. Mi van már meg a CFA4-ben

Commit: `cinemafundarchitecture4` `7e7b31c9`.

| Forrás | Mit ad | Felhasználás |
|---|---|---|
| Ingest (`project.ingestSettings`, `docs/project-files.md`) | Másolás a célmappába, a másolat ujjlenyomatának ellenőrzése, ütközéskor számozott név, sosem ír felül | **Átvehető minta és kód** az ellenőrzött másoláshoz |
| Médiaazonosság (`MediaClip::identity`) | Méret + 64 bites ujjlenyomat az első és utolsó MiB-ból | Gyors azonosság-előszűrés újrakapcsoláskor; a teljes kivonatot nem váltja ki |
| Link Media / `media.autoRelink` | Mappaelőtag-átírás, keresés mappafában, egyezési feltételek | Minta az újrakapcsoláshoz |
| `cfa4-docpath` (`vendor/cfa4-libs/docpath`) | Tárolt utak biztonságos feloldása (csak helyi gép vagy a dokumentum megosztása) | **Átvehető** az út-feloldás biztonsági szabályához |
| `cfa4-cli serve` + ADR 0007 | A Rust-motor szolgáltatásként való kiajánlása | A fájlműveletek ugyanígy, `files.*` parancsként |
| `sha2` crate (`crates/speech`, `crates/tts`) | SHA-256 | Már a függőségek között van |

## 3. CFA3/FA3

FA3 Unified Journal (`a9c724da`): archívumcsomag SHA-256 kivonattal és ellenőrzött visszaállítással. A
minta az archív gyökérhez és a mentéshez átvehető (mint a CFA4-AUDIT-001-ben). Fájlszolgáltatás-terv nincs.

## 4. Külső donorok

| Donor | Pontos forrás | Licenc | Mit ad | Értékelés |
|---|---|---|---|---|
| **AYON** (`ynput/ayon-core`) | `bf62ec28`, `client/ayon_core/pipeline/anatomy/` (`roots.py`, `templates.py`, `anatomy.py`) | Apache-2.0 | **Gyökérfüggetlen út:** `{root[work]}/projekt/…`; gyökerek gépenként és platformonként; úttemplate-ek | **Elsődleges minta, adaptálható.** Ez oldja meg, hogy M2-ben ugyanaz a katalógus több gépen is feloldható |
| **Stalker** | `eoyilmaz/stalker` @ `7de585b8`, `models/repository.py`, `structure.py` | LGPL-3.0 | `Repository`: gyökér platformonként, `to_os_independent_path`, `find_repo`; `Structure`: mappasablon | Minta; a Workfiles (9.) lépcsőhöz is |
| **Kitsu / Zou** | `cgwire/zou` @ `6204a805`, `models/stored_file.py`, `attachment_file.py` | AGPL-3.0 | Tárolt fájl soft delete-tel, végleges törlési napló (próbálkozások, utolsó hiba) | Csak referencia. A kétlépcsős törlés mintája |
| **BLAKE3** (`BLAKE3-team/BLAKE3`, Rust `blake3` crate) | (a G3-on rögzítendő commit) | CC0-1.0 / Apache-2.0 | Párhuzamosítható, gyors kivonat nagy fájlokra | **Elfogadva** (#45). Ok: a referenciagép Xeon E5 v4 processzorán nincs SHA-hardvergyorsítás, így a SHA-256 egy szálon nagyjából 0,4–0,5 GB/s, ami több száz GB-os produkciónál lassú |

## 5. Modell

- **`StorageRoot`**: UUIDv7, név, szerep (`work`, `publish`, `review`, `attachment`, `archive`), gépenkénti
  feloldás (M1-ben egy gép, egy út; M2-ben gépnév → út), állapot (elérhető, nem elérhető, csak olvasható).
- **`FileObject`**: UUIDv7, gyökér, gyökérfüggetlen relatív út, méret, tartalomkivonat (algoritmus +
  érték), gyors ujjlenyomat (a meglévő `identity`), MIME-típus, módosíthatóság (`mutable` munkafájlhoz,
  `sealed` publikált fájlhoz), állapot (`staging`, `committed`, `deleted`, `purged`), létrehozás ideje és
  szereplője.
- **Nincs verziófogalom a File Servicesben.** Új tartalom új `FileObject`. Azt, hogy ez egy asset 3.
  verziója, a BacklotHub tárolja.
- **Út-sablonok** (a Workfiles és a Publisher igényére): a sablont a BacklotHub adja, a File Services csak
  ellenőrzi, hogy a kész út a gyökéren belül marad (`..`, szimbolikus link kiszökése tiltva).

## 6. Írás, lezárás, törlés

- **Írás:**
  1. másolás a cél gyökér `staging` alkönyvtárába (ugyanazon a fájlrendszeren), a kivonat számítása
     másolás közben;
  2. `fsync`;
  3. atomikus átnevezés a végleges helyre (ütközéskor hiba, felülírás soha);
  4. katalógusrekord `committed` állapotban.

  Összeomláskor a `staging` maradék a következő induláskor kiderül és takarítható. A katalógus soha nem mutat
  félkész fájlra.
- **Lezárás (`seal`):** a publikált fájl csak olvashatóra vált, a kivonat rögzül. Lezárt fájl módosítása
  vagy felülírása nem lehetséges, csak új `FileObject`.
- **Törlés:** két lépcsőben, a Kitsu-minta szerint:
  - `deleted`: a fájl a helyén marad, a katalógusban rejtett, visszaállítható;
  - `purged`: fizikai törlés, csak a felhasználó jóváhagyásával (#18 elve), auditálva.

  Lezárt fájlra hivatkozó verziónál a végleges törlés tiltott, amíg a hivatkozás él.
- **Hivatkozás nélküli fájl:** a katalógus jelzi, a felület listázza, automatikusan nem törlődik.

## 7. Épség és újrakapcsolás

- **`files.verify`**: gyökérre, mappára vagy fájlra számolja újra a kivonatot. Eredmény: ép, módosult,
  hiányzik. Lezárt fájl eltérése súlyos hiba (`FILES_SEALED_MODIFIED`).
- **Újrakapcsolás:** ha egy gyökér máshová kerül (új lemez, átnevezett csatolási pont), elég a gyökér útját
  átírni, mert a katalógus gyökérfüggetlen. Egyedi fájlnál a gyors ujjlenyomat, majd a teljes kivonat dönt.
- **Never crash:** nem elérhető gyökér, teli lemez, jogosultsági hiba vagy sérült fájl kezelhető hibát ad
  angol hibakóddal (`FILES_ROOT_UNAVAILABLE`, `FILES_DISK_FULL`, `FILES_PERMISSION_DENIED`,
  `FILES_HASH_MISMATCH`). Összeomlás és adatvesztés nincs.

## 8. Mentés és visszaállítás (#14)

Elfogadva (#14, #45; 2026-11-08).

| Fájlfajta | Mentés | RPO | Visszaállítás sorrendje |
|---|---|---|---|
| Publikált, lezárt | Növekményes, kivonat alapján (egy fájl egyszer kerül a mentésbe, és ott ellenőrizhető) | a lezárás napja (≤ 24 h) | 1. |
| Munkafájl | Napi növekményes | 24 h (mint az adatbázis, #12) | 2. |
| Csatolmány, dokumentum | Napi növekményes | 24 h | 2. |
| Proxy, bélyegkép | **Nincs mentés:** a Media Services újra előállítja | – | utolsó (újragenerálás) |

- A katalógus a PostgreSQL-mentés része (#12: RPO 24 h, RTO 30 perc).
- A médiatár RTO-ja a mérettől és a lemez sebességétől függ, ezért nem fix érték. Célként a mérés adja meg
  az 1 TB-ra vetített időt a CFA4-MIN gépen. A felület visszaállításkor becsült időt mutat.
- Visszaállítás után a `files.verify` automatikusan lefut a visszaállított részre.
- Mentési cél: másik helyi lemez vagy helyi hálózati tár. Felhő csak a felhasználó kifejezett beállításával.

## 9. Titkosítás (SEC-05: kapcsolható)

Fájlrendszer-szinten (LUKS a teljes lemezre vagy fscrypt a gyökér könyvtárára). Alkalmazásszintű
fájltitkosítás nincs: a DCC-alkalmazások (Blender, Nuke stb.) közvetlenül a lemezről olvasnak, és
nem tudnának vele dolgozni. A felület jelzi, hogy egy gyökér titkosított köteten van-e.

## 10. Architektúra és interfész (#45)

**Döntés (#45, 2026-11-08):**
- **Adatút Rustban:** másolás, kivonatolás, `fsync`, átnevezés, lezárás, `verify`. Ez a meglévő
  ingest-kódra épül, és `files.*` parancsként érhető el a `cfa4-cli serve`-en (ADR 0007 mintájára). Nagy
  fájloknál a Rust megbízhatóbb és gyorsabb.
- **Katalógus Pythonban** (`cfa4_files`), a BacklotHub adatbázisában, saját sémában (`cfa4_files`). Így a
  katalógusírás, a BacklotHub hivatkozása és az auditesemény **egy tranzakcióban** történik, ahogy a
  CFA4-AUDIT-001 is javasolja. A Rust-oldalnak M1-ben nem kell PostgreSQL-kliens.
- **Alternatíva:** a katalógus is Rustban. Akkor a Rust-motor PostgreSQL-függőséget kap, és a BacklotHub
  hivatkozása külön tranzakcióba kerül. Elvetve.
- **Parancsok:**
  - a felület, a CLI, az MCP és az ügynökök számára: `files.import`, `files.seal`, `files.verify`,
    `files.resolve`, `files.delete`, `files.purge`, `files.roots.*`;
  - minden módosító parancs auditált (CFA4-AUDIT-001).
- **Kivonat:** BLAKE3 új Rust-függőségként (G3) a meglévő SHA-256 helyett, mert a
  referenciagépen többszörösen gyorsabb. A katalógus az algoritmust is tárolja, így később váltható.

## 11. Fázisok

| Fázis | Tartalom | Kell ehhez |
|---|---|---|
| FS-0 | Ez a terv jóváhagyva, ADR a `main`-be (#45 döntésével) | – |
| FS-1 | Gyökerek, katalógus, ellenőrzött írás, lezárás, `resolve`; tesztek: összeomlás írás közben, teli lemez, út-kiszökés | BacklotHub 5. lépcső csatolmány (#43), 8. lépcső |
| FS-2 | `verify`, újrakapcsolás, kétlépcsős törlés, hivatkozás nélküli fájlok | 8. lépcső G5 |
| FS-3 | Mentés és visszaállítás (#14), 1 TB-os mérés a CFA4-MIN gépen | 14. és 26. lépcső |
| FS-4 | M2: gépenkénti gyökérfeloldás, hálózati megosztás | M2 |

## 12. Nyitott kérdések

- A BLAKE3 crate pontos verziója és commitja a G3 kapun rögzül (licenc: CC0-1.0 / Apache-2.0).
- A médiatár 1 TB-ra vetített RTO-ja az FS-3 mérésből jön (`CFA4-MIN` profil).
