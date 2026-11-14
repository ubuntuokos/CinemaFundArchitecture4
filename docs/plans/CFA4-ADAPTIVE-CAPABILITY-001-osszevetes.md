# CFA4-ADAPTIVE-CAPABILITY-001 és az AI-terv összevetése

Az első változat 2026-11-07-én készült Claude Doc-ként; ez a fájl a 2026-11-14-i állapotra frissített,
a repóban tartott változat. Az ACS-terv a CFA4-AI_modules repóban él:
`docs/ai/acs-adaptiv-eroforras.md` (fázisok, ami elkészült), a döntések az ADR 0008-ban.

## 1. Röviden

A két terv nagyrészt ugyanabba az irányba mutat:

- helyi futás, nincs automatikus modellletöltés;
- közös modelltár és determinisztikus erőforrás-kezelő;
- nincs néma minőségcsökkentés;
- szakaszolás és folytathatóság.

Az adaptív terv (a továbbiakban: **ACS**) viszont **szélesebb**: nem AI-modul, hanem a CFA4 egészének
közös feladat- és erőforrás-infrastruktúrája.

Az összevetés **hét ütközést és kilenc hiányt** talált. Mára:

- mind a hét ütközés a javaslat szerint dőlt el (István, 2026-11-07, 2026-11-13);
- a kilenc hiányból nyolc kész, a kilencedik szabálya kész, a mérése a workerekkel jön.

## 2. Állapot 2026-11-14

### 2.1 A kilenc hiány

| # | Hiány | Hol van most | Állapot |
|---|---|---|---|
| 1 | Időbecslés tartománnyal és megbízhatósággal | `cfa4-tasks` `Estimate`; SDK gazdajelentés 4: `estimate`, `elapsed_s`, `remaining` | Kész (SDK #15) |
| 2 | Tájékoztatási kategóriák | `cfa4-tasks` `Plan::outlook`, `OutlookSettings` (10 perc, 4 óra); SDK `outlook` | Kész a tervezőben (AI #50, SDK #15); a feladatkezelőben még nem látszik |
| 3 | Időpont szerinti ütemezés | `cfa4-tasks` `StartMode`, `MissedStart`, IANA-időzóna; időzíthető exportsor; `tasks.decide` | Kész (ACS-4, ACS-5) |
| 4 | Felhasználói prioritás | `cfa4-tasks` `Priority`; SDK `class`, `priority` | Kész (ACS-4) |
| 5 | Folyamatos erőforrásfigyelés | `cfa4-tasks` `Monitor` (PSI, swap, hiszterézis), NVML, amdgpu sysfs | Kész (ACS-1, ACS-3) |
| 6 | Részleges betöltés, offload, utankénti RAM | SDK manifest 5–6 `paths[]` | Kész |
| 7 | Kétszeri indítás elleni védelem | `cfa4-store`: tranzakció, bérlet, `SKIP LOCKED` | Kész (ACS-4, ACS-7) |
| 8 | Eredményhelyesség CPU és GPU között | SDK manifest 7 `validated.golden`; `cfa4-tasks` `Tolerance::admits` | A szabály kész; a mérések a workerekkel jönnek |
| 9 | Nem AI-feladatok ugyanabban a sorban | motor: `task_host`, `background`, az exportsor kliensként | Kész (ACS-6) |

### 2.2 ACS-fázisok

| Fázis | Állapot |
|---|---|
| ACS-0 | Kész: összevetés (ez a dokumentum), ADR 0008 |
| ACS-1 | Kész (AI #40) |
| ACS-2 | Kész (AI #42) |
| ACS-3 | Kész (AI #44); tájékoztatási kategória és arany kimenet: AI #50 |
| ACS-4 | Kész: sor (AI #45), lezárás: ébresztés, kimenőtár, időzónák (AI #52) |
| ACS-5 | A felület és a motor kész (CFA4 #1172, AI #52): előzetes tájékoztatás (`tasks.check`), várakozás oka, hátralévő idő, megszakítás, döntés a lekésett indításról (`tasks.decide`) |
| ACS-6 | Folyamatban: az exportsor és a motor háttérfeladatai a közös soron (AI #49); az AI-modulok a workerekkel jönnek |
| ACS-7 | Folyamatban: a sor stabilitási próbája kész (AI #49), a referenciagépes futtató is (AI #52) |

### 2.3 Ami még nyitva van

1. **Az ACS-crate-ek a CFA4 főágában.** A `cfa4-tasks`, a motor `TaskHost`-ja és a `tasks.*`
   parancsok a CFA4-AI_modules repóban vannak. Amíg nincsenek a főágban, a főág motorja nem ad:
   - `tasks.list`-et és `tasks.check`-et;
   - várakozási okot;
   - döntési lehetőséget.

   A feladatkezelő panel ezt jelzi, és a háttérfeladatokat meg az exportsort mutatja. Az átvétel
   másik projekt területe: a tulajdonos és a merge koordinátor dönt róla (`docs/coordination.md` §2.8).
2. **Tájékoztatási kategória a feladatkezelőben** (normál / lassabb / nagyon hosszú / nem
   végrehajtható). Az AI #50 után; a kategória a manifest útjaiból számolt becslésből jön, ezért
   az AI-feladatoknál lesz értelme.
3. **A `cfa4-store` bekötése az alkalmazásba.** A motor feladatai most memóriában vannak, mert a
   vágás, a lejátszás és a mentés nem függhet az adatbázistól. A tartós sor az AI-workerekkel jön.
4. **Arany kimenetes CPU/GPU-mérések** (8. hiány): a workerekkel.
5. **A tulajdonos teendője:** a `scripts/dev/acs7-stability.sh` lefuttatása a referenciagépen
   (16 és 32 GB-os profil).

## 3. Egyezések

| ACS követelmény | Az AI-tervben már megvan |
|---|---|
| Local-first, nincs kötelező felhő (9.1) | VISION 2.3; a „felhős AI alapértelmezésként” elvetve |
| Nincs automatikus modellletöltés, a felhasználó dönt (9.1, 15) | A CFA4 modellt nem terjeszt; a felhasználó tölti le, egyszeri licencelfogadással |
| Közös modellkezelő, nincs párhuzamos modellcache (9.1, 15) | `cfa4-ai-models`: közös modelltár, SHA-256 deduplikáció |
| Az AI nem helyettesíti a determinisztikus erőforrás-kezelőt (9) | Minden `ai.*` parancson át fut; az LLM-méretválasztó is pontozó |
| Csak validált útvonal hajtható végre (2.1, 3.5) | Hiányzó mérés vagy ismeretlen feltétel elutasítást jelent; a `route_decision` feltételenként indokol |
| Nincs észrevétlen minőségcsökkentés (3.4, 4.3, 12) | „Nincs néma visszalépés” |
| Képességlekérdezés futtatás nélkül (3.5) | `route_decision`; a motorban `tasks.check` |
| Szakaszolás, ellenőrzőpont, folytatás (5.3) | `chunk`, `behavior.checkpoint`, `resume` |
| Tartós állapot újraindítás után (5.3, 10) | Tartós jobtár (PostgreSQL, `cfa4-store`) |
| Prioritások (5.2) | P0 interaktív, R végső render, P1 háttér, P2 spekulatív |
| Stabilitás (2.4) | Atomi foglalás, nincs „tartok és várok”, a kijelzőt hajtó GPU alapból kimarad |
| Mérhető tesztek gyengébb gépen is (13) | `run-minspec.sh` cgroup-korlátokkal (16 és 32 GB, 8 CPU, egy GPU) |
| Gyártófüggetlen gyorsítás | `accel`: sysfs-leltár, a médiamotor tartaléklánca mindig CPU-val zárul |

## 4. Az ütközések és a döntések

1. **Elérhetőségi szabály.** Eldöntve (2026-11-07): egy funkció elérhető, ha van validált
   végrehajtási útja a 12 GB VRAM / 16 GB RAM alsó határ felett. A felhasználó a becsült idő
   ismeretében dönt, és a „nincs néma visszalépés” megmarad.
2. **12 GB-os minimum.** Eldöntve (2026-11-07):
   - 12 GB VRAM alsó határ;
   - RAM minimum 16 GB, ajánlott 32 GB;
   - tisztán CPU-s AI-út nem cél (a VISION 2.4 CPU-s tartaléka a médiamotorra vonatkozik);
   - két tesztprofil.
3. **OOM-visszalépési létra.** Eldöntve: két részre bomlik. Automatikus az, ami az eredményt nem
   változtatja: kisebb köteg, csempézés átfedéssel, szakaszos futás, várakozás. Jóváhagyás kell a
   pontosság, a felbontás vagy a minőség csökkentéséhez, és a CPU-ra váltáshoz.
4. **Jobállapotok.** Eldöntve:
   - a sémabeli nevek maradnak;
   - új állapot a `scheduled` és a `paused` (SDK gazdajelentés);
   - a `blocked`, a `preparing_model`, a `loading` és a `cancelling` megmarad.
5. **Tartós állapot.** Eldöntve: a közös CFA4 PostgreSQL-klaszter, saját sémákkal; a redb és az
   SQLite kikerült (`docs/plans/CFA4-TASKSTORE-PG-001.md`). Driver: tokio-postgres +
   deadpool-postgres (2026-11-11).
6. **Kapuszámozás.** Eldöntve: előtag mindenhol, AI-F0…AI-F6, ACS-0…ACS-7, PG-0…PG-4.
7. **Hol él az erőforrás-kezelő.** Eldöntve: a nem AI-s `cfa4-tasks` crate-ben (L2). A
   hardverfelmérés az `accel` bővítése, a tartós tár a `cfa4-store` (L5). Az AI ennek kliense. Az
   ACS-1…ACS-4 a Qt 6 átállás alatt is mehetett.

Eldöntve 2026-11-13: az R (végső render) a P1 előtt áll a sorban (P0, R, P1, P2).

## 5. Amit az AI-terv tud, és az ACS-be is érdemes átvezetni

- Licencelfogadás egyszer modellenként és súlyhash-enként (`blocked` állapot, licenc okkal).
- Projektverzió-ellenőrzés az alkalmazás előtt, egyetlen visszavonható lépés.
- Vízjel és export utáni lépés a zárt workerekhez.
- Modellmegtartás LRU-val és azonos modellű feladatok kötegelése (a render is kötegelhető).

## 6. Kimi K3 donor

A `FareedKhan-dev/kimi-k3-in-c` Apache-2.0 licencű, hordozható C99 kód, GPU nélkül. A súlyokat
rétegenként olvassa a lemezről. Eldöntve (2026-11-07): a fogalmai Rustban kerülnek a közös
infrastruktúrába:

- memóriakeret;
- rétegstreaming;
- gyűrűpuffer;
- direkt I/O;
- telemetria.

Egy helyi LLM-workerről később, külön döntés lesz. A súlyokat a CFA4 nem terjeszti: a felhasználó
tölti le, és egyszer elfogadja a feltételeket.
