# CFA4-ADAPTIVE-CAPABILITY-001 és az AI-terv összevetése (CFA4-ADAPTIVE-CAPABILITY-001, ACS-0)

> **Állapot:** LEZÁRVA; az összes pont eldőlt (István, 2026-11-07 és 2026-11-13) · **Készült:** 2026-11-07 ·
> **Utolsó átnézés:** 2026-11-13 · **Szerző:** Horváth H. István (tulajdonos), Claude ·
> **Beépítve:** [CFA4-AI_modules ADR 0008](https://github.com/ubuntuokos/CFA4-AI_modules/blob/main/docs/adr/ADR-0008-ai-processing-layer.md),
> [ACS-terv](https://github.com/ubuntuokos/CFA4-AI_modules/blob/main/docs/ai/acs-adaptiv-eroforras.md) (ACS-0…ACS-7),
> CFA4-AI_modules #50, CFA4-sdk #15 · **Kapcsolódik:** [`CFA4-TASKSTORE-PG-001`](CFA4-TASKSTORE-PG-001.md),
> [`CFA4-AI-MODEL-TRAINING-001`](CFA4-AI-MODEL-TRAINING-001.md), CFA4-ADAPTIVE-CAPABILITY-001 (tulajdonosi
> specifikáció, 2026-11-07; a szövege még nincs a tárolóban)
>
> Ez az ACS-0 fázis (architektúra-ellenőrzés) összevetése, a megosztott dokumentum 2026-11-13-i állapotában.
> A szövegben az „ADR 0006” az AI-modulok ADR-jének akkori száma; ma **ADR 0008**. A „Döntések Istvántól”
> lista a 2026-11-07-i kérdéseket mutatja, az eredményük az „Állapot 2026-11-13” részben áll.

## Röviden

A két terv nagyrészt ugyanabba az irányba mutat: helyi futás, nincs automatikus modellletöltés, közös modelltár, determinisztikus erőforrás-kezelő, nincs néma minőségcsökkentés, szakaszolás és folytathatóság. Az adaptív terv (a továbbiakban: **ACS**) viszont **szélesebb**: nem AI-modul, hanem a CFA4 egészének közös feladat- és erőforrás-infrastruktúrája, és **más alapelvre** épül a funkciók elérhetőségében.

**A legnagyobb ütközés:** az ADR 0006 szerint ami a felhasználó VRAM-szintjén nem fér el, az nem elérhető, és a 12 GB-os gép a működési minimum. Az ACS szerint egy funkciót nem szabad letiltani pusztán a lassúság miatt, a CPU-s út is elfogadható. Itt az ACS valójában a VISION.md-vel egyezik (2.4: „mindig legyen CPU-s tartalék”), így az ADR 0006 már most is eltér a VISION-től.

**Hét ütközés, kilenc hiány.** Az ütközések közül négyet egyszerű átnevezéssel vagy pontosítással fel lehet oldani (kapuszámozás, jobállapotok, automatikus visszalépés, a terv elhelyezése). Kettő tulajdonosi döntés volt (elérhetőségi szabály, 12 GB-os minimum). A tartós tárnál az AI-terv redb-je ütközik a CFA4 elfogadott **PostgreSQL-döntésével** (CFA4 ADR 0006, BacklotHub); erre külön terv készült: [CFA4 PostgreSQL-adatréteg a feladatsorhoz és az AI-jobtárhoz](CFA4-TASKSTORE-PG-001.md).

**Amit ellenőriztem:** a CFA4-AI_modules `VISION.md`, `AGENTS.md`, `docs/adr/ADR-0006-ai-processing-layer.md`, `crates/accel`; a cfa4-sdk `docs/protocol.md`, `schemas/host.schema.json`, `schemas/manifest.schema.json` (v4); a CFA4 AI tervdokumentum; a `FareedKhan-dev/kimi-k3-in-c` repó adatai. A kért `docs/coordination.md` **egyik repóban sincs meg** (és a GitHub-keresés a CinemaFundArchitecture4-ben sem találja), ezért azt nem tudtam figyelembe venni.

## Állapot 2026-11-13: a hiányok pótolva

A kilenc hiányból hetet már a 2026-11-07 utáni munka lefedett (ADR 0008, ACS-1…ACS-7, PostgreSQL). A maradék kettőt és az elavult részeket most pótoltam, a javaslatok szerint. Az ütközések 1–7. pontja mind a javaslat szerint dőlt el.

| # | Hiány | Hol van most | Állapot |
| --- | --- | --- | --- |
| 1 | Időbecslés tartománnyal és megbízhatósággal | `cfa4-tasks` `Estimate`; SDK gazdajelentés 4: `estimate`, `elapsed_s`, `remaining` | Kész (SDK: #15) |
| 2 | Tájékoztatási kategóriák | `cfa4-tasks` `Plan::outlook`, `OutlookSettings` (10 perc, 4 óra); SDK `outlook` | Új (SDK #15, AI-modulok commit) |
| 3 | Időpont szerinti ütemezés | `cfa4-tasks` `StartMode`, `MissedStart`; SDK `schedule`, `missed` | Kész (ACS-4); SDK #15 |
| 4 | Felhasználói prioritás | `cfa4-tasks` `Priority`; SDK `class`, `priority` | Kész (ACS-4); SDK #15 |
| 5 | Folyamatos erőforrásfigyelés | `cfa4-tasks` `Monitor` (PSI, swap, hiszterézis), NVML, amdgpu sysfs | Kész (ACS-1, ACS-3) |
| 6 | Részleges betöltés, offload, utanként RAM | SDK manifest 5–6 `paths[]` | Kész |
| 7 | Kétszeri indítás elleni védelem | `cfa4-store` tranzakció, bérlet, `SKIP LOCKED` | Kész (ACS-4, ACS-7) |
| 8 | Eredményhelyesség CPU és GPU között | SDK manifest 7 `validated.golden`; `cfa4-tasks` `Tolerance::admits` | Új szabály; a mérések a workerekkel jönnek |
| 9 | Nem AI-feladatok ugyanabban a sorban | `engine` `task_host`, `background` | Kész (ACS-6) |

**Javított elavult részek (ADR 0008):** az állapotsor („KÓD MÉG NINCS”), a `cfa4-store` rétege (L5, nem L2), a sémaverziók (gazdajelentés 4, manifest 7), a tájékoztatás és az osztálysorrend a 3. pontban, az eredményhelyesség a 9. pontban. Az SDK-mintában a megszűnt `cfa4-paid-worker` is kicserélődött.

**Eldöntve (István, 2026-11-13):** az R (végső render) a P1 előtt áll a sorban (P0, R, P1, P2). Az ADR 0008 3. pontja és az ACS-terv tulajdonosi döntésként rögzíti (AI-modulok #50).

## Egyezések

| ACS követelmény | Az AI-tervben már megvan |
| --- | --- |
| Local-first, nincs kötelező felhő (9.1) | VISION 2.3, ADR 0006 „Felhős AI alapértelmezésként” elvetve |
| Nincs automatikus modellletöltés, a felhasználó dönt (9.1, 15) | ADR 0006 2. pont; a CFA4 modellt nem terjeszt, a felhasználó tölti le, egyszeri licencelfogadással |
| Közös modellkezelő, nincs párhuzamos modellcache (9.1, 15) | `cfa4-ai-models`: közös modelltár, SHA-256 deduplikáció, egyetlen közös letöltés |
| Az AI nem helyettesíti a determinisztikus erőforrás-kezelőt (9) | Minden `ai.*` parancson át fut; az F6-os LLM-méretválasztó is pontozó, nem LLM-prompt |
| Nincs hamis pozitív: csak validált útvonal végrehajtható (2.1, 3.5) | ADR 3. pont: hiányzó mérés vagy ismeretlen feltétel elutasítást jelent; `route_decision` feltételenként indokol |
| Nincs észrevétlen minőségcsökkentés, eltérő eredményhez jóváhagyás (3.4, 4.3, 12) | ADR 3. pont: „Nincs néma visszalépés” |
| Képességlekérdezés futtatás nélkül (3.5) | `route_decision` („mi futna és miért”), F1 kilépési feltétele |
| Szakaszolás, ellenőrzőpont, folytatás (5.3) | `chunk`, `behavior.checkpoint`, `behavior.cancellable`, `caps.features: resume` |
| Tartós állapot újraindítás után (5.3, 10) | Tartós jobtár; a félbemaradt job `failed` vagy folytatható lesz |
| Prioritások (5.2) | Tervdokumentum: P0 interaktív, P1 háttér, P2 spekulatív, R végleges render |
| Stabilitás, nem fogy el minden erőforrás (2.4) | Atomi foglalás, nincs „tartok és várok”, a kijelzőt hajtó GPU alapból kimarad |
| Mérhető tesztek gyengébb gépen is (13) | `run-minspec.sh` cgroup-korlátokkal (32 GB, 8 CPU, egy GPU) |
| Gyártófüggetlen gyorsítás, CPU-s tartalék | VISION 2.4 és a meglévő `accel` crate: sysfs-leltár, a tartaléklánc mindig CPU-val zárul |
| A Kimi K3 nem kötelező, a modell licence külön kezelendő (8) | Rendezett szabály: a súly licence a felhasználó felelőssége, a kódé GPL-politika szerint |

## Ütközések és javaslatok

### 1. Elérhetőségi szabály: „nem elérhető” vagy „lassabb, de elérhető”

- **AI-terv:** ADR 0006 3. pont: „Ami nem fér el, az nem elérhető.” István 2026-10-25-i döntése: nincs kényszerített visszalépés. Az első kör futtatói: ONNX CUDA EP és CPU EP.
- **ACS:** 2.1 és 2.3: nem lehet mesterséges funkciókorlátozás a teljesítménykategória miatt; a CPU-s vagy GPU nélküli végrehajtás elfogadható; a felhasználó dönt a becsült idő ismeretében.
- **VISION.md:** 2.4 és 5: „mindig legyen CPU-s tartalék”, a modellek „CPU-n, vagy bármely gyártó GPU-ján” futnak. Ez az ACS-sel egyezik, az ADR 0006-tal nem.

**Javaslat:** az ACS szabálya legyen az új alapelv, a „nincs néma visszalépés” megtartásával. Egy funkció akkor elérhető, ha van legalább egy **validált végrehajtási útvonala** (GPU, CPU, rétegenkénti offload, csempézés, proxy). „Nem végrehajtható jelenleg” csak akkor, ha egyik validált útvonal sem fut a gépen (például nincs elég RAM a CPU-s úthoz sem). Az útvonalat a manifest deklarálja, mérésekkel; a felhasználó a becsült időt látja és dönt. A nagy generatív videómodellek CPU-n gyakorlatilag órákig, napokig futnak: ezek „nagyon hosszú végrehajtás” jelölést kapnak, nem tiltást, de csak akkor, ha a CPU-s utat valóban lemértük.

### 2. A 12 GB-os működési minimum

- **AI-terv:** ADR 0006 9. pont: NVIDIA Ampere 12 GB, 32 GB RAM, 8 mag a működési minimum; minden ingyenes funkciónak 2K-ban működnie kell rajta; 8 GB-os profil nincs.
- **ACS:** 13.1: ugyanazt a funkciót GPU nélküli és kevés RAM-os gépen is ellenőrizni kell.

**István tapasztalata (2026-11-07):** 12 GB-nál kisebb GPU-val és 16 GB fizikai RAM alatt nem lehet normális munkát végezni helyi AI-val.

**Frissített javaslat:** a 12 GB VRAM **alsó határ marad**, a RAM-ban pedig két szint lesz (István, 2026-11-07): **minimum 16 GB, ajánlott 32 GB**. A minimum alatt a CFA4 AI-funkciói nem támogatottak (a nem AI-s CFA4 természetesen fut). Az ACS funkciómegőrzési elve **e határ felett** érvényes: ha egy funkció nem fér el a 12 GB-os kártyán vagy a 16 GB RAM-ban, de van validált lassabb útja (rétegenkénti offload, csempézés, szakaszos futás), akkor nem tiltjuk le, hanem a becsült idővel felajánljuk. A tisztán CPU-s, GPU nélküli AI-út nem cél. A VISION 2.4 „CPU-s tartaléka” a médiamotorra (dekódolás, render) vonatkozik, az AI-ra a 12 GB-os alsó határ. **Tesztelés:** a `run-minspec.sh` két profilt kap: 32 GB (ajánlott, az ingyenes funkciók 2K-s teljesítményígérete erre szól) és 16 GB (minimum: minden funkció lefut, lassabban is, összeomlás és rendszerlefagyás nélkül).

### 3. Automatikus OOM-visszalépési létra

- **AI-terv belső ellentmondás:** az SDK `docs/protocol.md` 3. pontja szerint `oom` esetén a gazda magától visszalép: kisebb felbontás, csempézés, kisebb pontosság, végül CPU. A tervdokumentum ugyanígy ír. Az ADR 0006 3. pontja viszont tiltja a néma visszalépést.
- **ACS:** 12: automatikus módváltás csak akkor, ha nem veszélyezteti az eredmény helyességét; ha eltérést okozhat, jóváhagyás kell.

**Javaslat:** a létrát két részre kell bontani. **Automatikus** az, ami az eredményt nem változtatja (vagy a manifestben rögzített tűrésen belül marad): kisebb köteg, csempézés átfedéssel, szakaszos futás, várakozás erőforrásra. **Jóváhagyás kell** a pontosság (fp16 → int8), a felbontás vagy a minőségi szint csökkentéséhez, és a CPU-ra váltáshoz, hacsak a manifest nem igazolja, hogy a CPU-s eredmény tűrésen belül azonos. Az SDK `protocol.md` 3. pontját ennek megfelelően javítani kell.

### 4. Jobállapotok

| ACS állapot | AI-terv (`host.schema.json`) | Megjegyzés |
| --- | --- | --- |
| Created | — | Nálunk a job `queued`-ként születik |
| Waiting | `queued` |  |
| Scheduled | — | **Hiányzik:** időpontra ütemezett job |
| Resource Waiting | `waiting_for_resources` | Egyezik |
| Ready | — | Nálunk átmenet, nem állapot |
| Running | `preparing_model`, `loading`, `running` | Nálunk finomabb bontás |
| Paused | — | **Hiányzik** |
| Completed | `succeeded` | Csak név |
| Failed | `failed` | Egyezik |
| Cancelled | `cancelling`, `cancelled` | Nálunk köztes állapot is van |
| — | `blocked` | Az ACS-ben nincs: licencelfogadásra, modellre, hardverre vár |

**Javaslat:** a sémabeli nevek maradjanak (a séma szerződés, az átnevezés törő változás lenne), a felület mutathatja az ACS magyar elnevezéseit. A gazdajelentés 2. változata vegye fel a `scheduled` és a `paused` állapotot, és legyen `created` is, ha a naplózásnak kell. A `blocked`, `preparing_model`, `loading` és `cancelling` maradjon, mert olyan helyzetet írnak le, amelyet az ACS nem kezel (licencelfogadás, modell-előkészítés). A `paused` workertámogatást igényel: legegyszerűbben szakaszhatáron megállás és `resume`, így új protokollüzenet nélkül is működik.

### 5. Tartós állapot: PostgreSQL

- **ACS:** 10: „A CFA4 meglévő PostgreSQL-alapú architektúráját kell használni, nem új adatbázismotort bevezetni”; 7.3: PostgreSQL és pgvector a Media Intelligence-hez.
- **Tény (javítva 2026-11-07):** a CinemaFundArchitecture4-ben van elfogadott PostgreSQL-döntés: `docs/adr/ADR-0006-backlothub-postgresql.md` (2026-11-02), de csak a BacklotHub produkciókezelőre, egygépes módban is, Ubuntu-rendszercsomagból. A feladatsorra, az AI-jobokra és a Media Intelligence-re nincs PostgreSQL-terv, és a kódban még nincs adatbázis-crate. Az AI ADR 0006 redb-t, a tervdokumentum SQLite-ot említ. (Az első változat tévesen állította, hogy nincs PostgreSQL: a CFA4 fő repóját akkor még nem néztem át.)

**Frissített javaslat:** a feladatsor, az ütemezés és az AI-jobtár a BacklotHub által bevezetett **közös CFA4 PostgreSQL-klasztert** használja, saját sémákkal; a redb és az SQLite kikerül, így nincs új adatbázismotor. Részletek: [CFA4 PostgreSQL-adatréteg a feladatsorhoz és az AI-jobtárhoz](CFA4-TASKSTORE-PG-001.md). Mellékesen kiderült: az AI ADR 0006 száma ütközik a CFA4 ADR 0006-tal, és a benne álló 0004/0005-hivatkozások sem stimmelnek.

### 6. Kapuszámozás

Az AI-terv F0–F6 kapukat használ, az ACS F0–F7 fázisokat, a CFA4 auditjai pedig F3–F10 néven futnak (ADR-F8, F9 kiadási döntés, F10). Három, egymást átfedő „F” sorozat van.

**Javaslat:** előtag mindenhol. Az AI-kapuk **AI-F0…AI-F6** (a commitok már így hivatkoznak: „AI-F3”), az ACS fázisai **ACS-0…ACS-7**. Az ADR 0006 és a tervdokumentum szövegében egyszer kell átírni.

### 7. Hol él az erőforrás-kezelő

- **AI-terv:** az erőforrás-kezelő, a VRAM-nyilvántartás és a tartós jobtár a `cfa4-ai-jobs` crate-ben (L2) van, vagyis AI-specifikus.
- **ACS:** 3, 7, 16: közös infrastruktúra minden erőforrásigényes modulnak (render, proxy, kodekkonverzió, Media Intelligence), és az AI nem hozhat létre vele versengő rendszert.

**Javaslat:** az erőforrás-kezelő, a feladatsor, az ütemező és a tartós tár kerüljön egy **új, nem AI-s crate-be** (munkanév: `cfa4-tasks`, L1–L2, felület nélkül). A hardverfelmérés a meglévő `accel` crate bővítése legyen, ne új komponens. A `cfa4-ai-jobs` ennek kliense lesz, és csak az AI-specifikus részt tartja meg: Elemzés → Javaslat → Előnézet → Alkalmazás, modell-előkészítés, licencelfogadás. Így az ACS F0 elfogadási feltétele (nincs párhuzamos architektúra) teljesül.

**Következmény az ADR 0006 F0 kapujára:** ott az áll, hogy a Qt 6 átállás végéig nem indul AI-crate. A `cfa4-tasks` és az `accel` bővítése nem AI és nem felület, ezért az átállás alatt is mehetne (ACS-1…ACS-4), a feladatkezelő nézet (ACS-5) pedig az átállás után. **Ez is tulajdonosi döntés**, és az ACS 15. fejezete szerint a terv maga nem ad kódolási engedélyt.

## Hiányok az AI-tervben

1. **Időbecslés tartománnyal és megbízhatósággal (4.2, 4.3).** A manifestben csak `fps_hint` van felbontásonként, CPU-s útra nincs. A `route_decision`-ben és a `job_status`-ban nincs becslés. Javaslat: `estimate {min_s, max_s, confidence}` a `route_decision`-be, `elapsed_s` és `remaining {min_s, max_s}` a `job_status`-ba; a manifest `fps_hint`-je végrehajtási útvonalanként.
2. **Tájékoztatási kategóriák (4.1).** Normál, lassabb, nagyon hosszú, nem végrehajtható jelenleg: a `route_decision` egy `outlook` mezőjéből származtatható, a küszöböket a felület állítja.
3. **Időpont szerinti ütemezés (6).** Teljesen hiányzik: négy indítási mód (azonnal, erőforrásra, időpontra, időpontra és erőforrásra), időzóna, elmaradt indítás szabálya, gép felébresztése nélkül.
4. **Felhasználói prioritás (5.2).** A P0/P1/P2/R rendszerosztály, az ACS alacsony/normál/magas felhasználói prioritás. Javaslat: a kettő együtt él; a felhasználói prioritás a P1-en (és az R-en) belüli sorrendet adja.
5. **Folyamatos erőforrásfigyelés (3.2, 3.3).** Az ADR csak NVML-es VRAM-nyilvántartást és deklarált igényeket ismer. Hiányzik: RAM-nyomás (Linux PSI, `/proc/pressure`), swap, I/O-terhelés, más alkalmazások VRAM-ja, a kiugrás és a tartós korlát megkülönböztetése (hiszterézis), felhasználói tartalék és korlát. Az NVML csak NVIDIA: AMD-hez `amdgpu` sysfs, Intelhez az `xe`/`i915` sysfs kell, a gyártófüggetlenség miatt.
6. **Részleges modellbetöltés, streaming, offload (2.1, 3.4).** A manifestben nincs mező a rétegenkénti offloadra és a végrehajtási útvonalankénti RAM-igényre. Javaslat: a manifest 5. változatában `paths[]` (útvonal: eszköz, pontosság, offload, `ram_mb`, `vram_mb`, `fps_hint`, eltérés a referencia-úttól és tűrés).
7. **Kétszeri indítás elleni védelem (10).** Az ADR csak annyit mond, hogy a félbemaradt job `failed` vagy folytatható lesz. Kell: tranzakciós állapotváltás, futási bérlet (lease) újraindítás után, idempotens szakaszeredmények a tartalomcímzett gyorsítótárban.
8. **Eredményhelyességi tesztek CPU és GPU között (13.5).** Nincs. Javaslat: implementációnként arany kimenet és tűrés, a `behavior.deterministic` mező alapján bitazonos vagy tűréses összevetés.
9. **Nem AI-feladatok ugyanabban a sorban (5, 7.4).** Render, proxygenerálás, kodekkonverzió. Az `export` crate-nek már van saját jobja; ezt a `cfa4-tasks` kliensévé kell tenni, nem lecserélni.

## Amit az AI-terv tud, és az ACS-ből hiányzik (megtartandó)

- Licencelfogadás egyszer modellenként és súlyhash-enként, `blocked` állapot `license` okkal.
- Projektverzió-ellenőrzés az Alkalmazás előtt, egyetlen visszavonható lépés.
- Vízjel és export utáni lépés a zárt workerekhez.
- Atomi foglalás, holtpont elkerülése, a kijelzőt hajtó GPU kizárása, stabil GPU-azonosítás.
- Modellmegtartás LRU-val és azonos modellű feladatok kötegelése (a modellváltás drága).

Ezeket az ACS-be is érdemes átvezetni, mert a nem AI-feladatokra is érvényesek (például a render is kötegelhető).

## Kimi K3 donor

- **Ellenőrzött adatok:** `FareedKhan-dev/kimi-k3-in-c`, Apache-2.0, hordozható C99, BLAS, keretrendszer és GPU nélkül. A leírása szerint a 2,78 billió paraméteres Kimi K3-at egyetlen CPU-n, 8,24 GB RAM-ban futtatja, a súlyokat rétegenként a lemezről olvasva. Létezik Rust-újraírása is (`mohdhasnain-pixel/kimirust`, Apache-2.0), de az még nagyon kezdetleges.
- **Licenc:** az Apache-2.0 GPL-3.0-kompatibilis, a kód felhasználható, eredeti fejlécekkel és a `docs/licensing.md` 5. pontja szerinti nyilvántartási sorral.
- **Hogyan illeszkedik:** a CFA4 terméke tiszta Rust, ezért a C-kód nem kerülhet a CFA4 folyamatába. Két út van: a fogalmakat (memóriakeret, rétegstreaming, gyűrűpuffer, direkt I/O, telemetria) Rustban újraírjuk az ACS-1…ACS-3 részeként; vagy a motor egy **külön worker-folyamat** lesz, mert a worker-protokoll bármilyen nyelvű workert enged. Az első a közös infrastruktúrához, a második egy esetleges helyi LLM-képességhez (AI-F6) illik.
- **A modell:** a CFA4 a Kimi K3 súlyait nem terjeszti. A felhasználó tölti le első használatkor, a feltételeit megismeri, és egyszer elfogadja a figyelmeztetést (a 2026-11-07-i rendezett szabály szerint). A Kimi K3 nem kötelező modell, és a súlyok saját licencét a letöltés előtt ellenőrizni kell (nem azonos a C-motor Apache-2.0 licencével).

## Mit kellene módosítani

### ADR 0006 (CFA4-AI_modules)

- 3. pont: új elérhetőségi szabály (validált végrehajtási útvonal), a létra kettébontása (automatikus vagy jóváhagyásos).
- 9. pont: a 12 GB referencia- és teljesítményígéret, nem belépési kapu; új tesztprofilok.
- Komponenstérkép: az erőforrás-kezelő, a feladatsor és a tartós tár a `cfa4-tasks` crate-be kerül, a hardverfelmérés az `accel`-be.
- 4. pont: új állapotok (`scheduled`, `paused`), ütemezési módok, felhasználói prioritás.
- Kapuk átnevezése AI-F0…AI-F6-ra, utalás az ACS-0…ACS-7 fázisokra.
- **Elavult részek javítása (a 2026-11-07-i döntés óta):** az F3 bekezdés végén „A #91 videó-felskálázás későbbi kapuban jön, saját tanítású súllyal”, és a nyitott kérdések között „Melyik saját tanítású modell legyen a #91 alapja”. Saját modell nincs, ezeket ki kell venni.

### CFA4-sdk

- `protocol.md` 3. pont: az `oom` létra javítása (csak az eredményt nem változtató lépés automatikus).
- `host.schema.json` 2. változat: `scheduled`, `paused` (esetleg `created`), becslés és hátralévő idő, `outlook` a `route_decision`-ben, ütemezési adatok (időpont, időzóna, mód, elmaradt indítás szabálya, prioritás).
- `manifest.schema.json` 5. változat: végrehajtási útvonalak (`paths[]`) RAM-, VRAM- és sebességadattal, offload, tűrés.

### Tervdokumentum

- Az „Ütemezés, VRAM, cache” rész OOM-létrája és a SQLite-os gyorsítótár-index a fentiek szerint.
- A „Havi bevezetés” rész már elavult (kapuk vannak), egy mondattal az ADR-re kell utalni.

## Döntések Istvántól

1. **Elérhetőségi szabály:** átvesszük az ACS elvét (validált útvonal esetén elérhető, a felhasználó dönt a becsült idő alapján), a „nincs néma visszalépés” megtartásával?
2. **12 GB / RAM:** eldöntve (István, 2026-11-07): 12 GB VRAM alsó határ, RAM minimum 16 GB, ajánlott 32 GB; a funkciómegőrzés e felett érvényes, két tesztprofillal.
3. **Tartós tár:** a közös CFA4 PostgreSQL-klaszter, a külön PostgreSQL-terv szerint (István kérésére 2026-11-07-én elkészült)?
4. **Elhelyezés:** az erőforrás-kezelő és a feladatsor új, nem AI-s `cfa4-tasks` crate-be kerül, az AI ennek kliense?
5. **Ütemezés:** az ACS-1…ACS-4 (nem felület) indulhat-e a Qt 6 átállás alatt, vagy minden az AI-F0 kapu után?
6. **Elnevezés:** AI-F0…AI-F6 és ACS-0…ACS-7 előtagok, a sémabeli állapotnevek változatlanok maradnak?
7. **Kimi K3:** fogalmak Rustban a közös infrastruktúrába, és csak később, külön döntéssel worker egy helyi LLM-hez?
