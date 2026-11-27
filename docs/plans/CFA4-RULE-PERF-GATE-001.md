# CFA4-RULE-PERF-GATE-001 – Feltételesen kizárt technológiák: teljesítménykapu

**Állapot:** FINAL 1.0 – a tulajdonos jóváhagyta 2026-11-28-án („jóváhagyom és véglegesítsd!”), D1–D4 a javaslat szerint
**Dátum:** 2026-11-28
**Alap:** `main` @ `fde0fa08` (`AGENTS.md`, `docs/licensing.md`, ellenőrizve az eredeti, aktuális változaton)
**Végrehajtás:** egy PR: a szabály az `AGENTS.md` CFA4-szakaszába, ez a terv a `docs/plans/` mappába kerül. A gazda olvasztja be.

---

## 1. A szabály (a tulajdonos szövege, változatlanul)

### Feltételesen kizárt technológiák – teljesítménykapu

A CFA4-ben korábban kizárt vagy nem preferált technológiák nem tekintendők abszolút tiltásnak.
Alkalmazásuk megengedhető, amennyiben bizonyíthatóan nem rontják a CFA4 működését,
teljesítményét, stabilitását vagy felhasználói élményét.

**Alkalmazási feltételek** – a kizárt technológia CFA4-be történő bevonása előtt vizsgálni kell:

- **Teljesítmény:** nem okoz mérhető lassulást a CFA4 alapműködésében.
- **Erőforrás-használat:** nem okoz indokolatlan CPU-, RAM-, VRAM-, tárhely- vagy I/O-terhelést.
- **Responsivitás:** nem lassítja a GUI-t, a timeline-t, a preview-t, a médiaimportot, a keresést
  vagy az interaktív munkafolyamatokat.
- **Párhuzamos működés:** nem akadályozza a CFA4 egyidejű moduljainak és feldolgozási feladatainak
  működését.
- **Stabilitás:** nem növeli elfogadhatatlan mértékben a hibák, fagyások, összeomlások vagy
  adatvesztések kockázatát.
- **Integráció:** nem hoz létre szükségtelenül új folyamatokat, szolgáltatásokat, függőségeket vagy
  különálló infrastruktúrát.
- **Skálázhatóság:** nagy projektek, hosszú videók, több egyidejű feladat és nagy médiaállomány
  esetén sem jelent elfogadhatatlan szűk keresztmetszetet.
- **Fallback:** ahol indokolt, a CFA4 rendelkezzen alternatív működési móddal, ha a technológia
  túlterhelést vagy teljesítményproblémát okoz.

**Döntési szabály.** A technológia alkalmazható, ha a CFA4 működésére gyakorolt hatása
elfogadható, és az előnyei indokolják a bevezetését. Ha a technológia lassítja a CFA4-et, akkor:

1. optimalizálni kell az integrációt;
2. szükség esetén izolált, háttérben futó vagy ütemezett feldolgozásra kell helyezni;
3. lehetőség szerint adaptív erőforrás-kezelést kell alkalmazni;
4. ha a teljesítményromlás nem szüntethető meg elfogadható mértékben, a technológia nem építhető
   be az adott működési útvonalba.

**Különleges szabály.** A kizárt technológia nem automatikusan elutasítandó donor. Az agent
köteles megvizsgálni, hogy a technológia megfelelő integrációval, optimalizálással, izolálással
vagy opcionális háttérfeldolgozással felhasználható-e a CFA4 teljesítményének romlása nélkül.
A teljesítményre vonatkozó állításokat mérés, benchmark, profilozás vagy ellenőrizhető technikai
bizonyíték alapján kell megtenni. Mérési bizonyíték hiányában az alkalmazhatóság csak előzetes
javaslatként jelölhető meg, nem igazolt megfelelőségként. A CFA4 alapvető működésének
teljesítménye elsőbbséget élvez a kiegészítő technológiák bevezetésével szemben.

---

## 2. Hová kerül, és mihez kapcsolódik

`AGENTS.md` CFA4-szakasz, új pontként közvetlenül a **„Non-Rust components”** pont után.
A szabályfájlok nyelve angol, és minden szabály csak egy helyen él (2026-11-01-i döntés), ezért
az angol szöveg kerül az `AGENTS.md`-be; a fenti magyar szöveg ennek a tervnek a része marad.

A meglévő szabályokkal való viszony:

| Meglévő szabály | Viszony |
|---|---|
| „Port or keep the original language” (2026-11-03) | Ugyanaz a „nem lassíthatja a CFA4-et” mérce; az új szabály általánosítja minden kizárt technológiára. |
| „Non-Rust components” (2026-11-07) | Ugyanígy: annak a mérési módja (referenciagép) itt is érvényes. |
| Media backends / router (`cfa4-route`) | A kizárt technológia is csak a routeren át, saját fallbackkel kerülhet be. |
| §0 Never crash | Változatlanul érvényes, a „Stabilitás” feltétel erre épül. |
| Licencszabályok, zárt kód/plug-in tilalma, gyártófüggetlenség, Ubuntu-modell, CFA4-SECURITY-001 | **Nem oldja fel őket** – lásd D1. |

---

## 3. Javasolt `AGENTS.md`-szöveg (angol)

```markdown
- **Conditionally excluded technologies: the performance gate** (owner decision, 2026-11-28;
  CFA4-RULE-PERF-GATE-001). A technology CFA4 earlier excluded or did not prefer is not banned
  outright: it may be used if it provably does not degrade CFA4's operation, performance,
  stability or user experience. An excluded technology is not an automatically rejected donor:
  the agent must examine whether integration, optimisation, isolation or optional background
  processing makes it usable without slowing CFA4 down.
  - **Check before bringing it in:** performance (no measurable slow-down of core operation);
    resources (no unjustified CPU, RAM, VRAM, storage or I/O load); responsiveness (GUI,
    timeline, preview, media import, search and interactive work stay as fast); concurrency (it
    does not block CFA4's concurrent modules and jobs); stability (no unacceptable rise in errors,
    hangs, crashes or data loss, §0); integration (no needless new processes, services,
    dependencies or separate infrastructure); scalability (no unacceptable bottleneck with large
    projects, long videos, several concurrent jobs or large media libraries); fallback (where
    reasonable, an alternative mode for when it overloads or underperforms).
  - **Decision:** it may be used if its effect on CFA4 is acceptable and its benefits justify it.
    If it slows CFA4 down: (1) optimise the integration; (2) if needed, move it to isolated,
    background or scheduled processing; (3) where possible, use adaptive resource management;
    (4) if the slow-down cannot be brought to an acceptable level, it does not go into that
    operating path.
  - **Evidence:** performance claims rest on measurement, benchmarks, profiling or other
    verifiable technical evidence, taken on the reference machine
    ([`docs/development/reference-machine.md`](docs/development/reference-machine.md)) and
    recorded in the plan or ADR that brings the technology in. Without measured evidence,
    suitability may only be marked as a preliminary proposal, never as verified compliance.
  - **Priority:** the performance of CFA4's core operation comes before adding supplementary
    technologies.
  - **Scope:** this gate decides performance only. It does not lift the licence rules above
    (GPL-2.0-only, AGPL in-process, proprietary code and plug-ins, closed protocols), vendor
    neutrality, the `cfa4-restricted-extras` model, the security layer (CFA4-SECURITY-001) or
    §0–§2; a technology excluded for one of those reasons stays excluded unless the owner
    decides otherwise.
  - **Excluded outright, the gate does not apply** (owner decision, 2026-11-28): Conda and its
    family (Anaconda, Miniconda, Miniforge, Mamba, micromamba, conda-forge environments) are never
    used in CFA4: not in the product, the packages, CI, development tooling or AI workers. Python
    dependencies come from Ubuntu's packages or a `venv`/`pip` environment.
```

---

## 4. Tulajdonosi döntések (elfogadva 2026-11-28)

- **D1 – Hatókör (elfogadva: a fenti „Scope” bekezdés).** A teljesítménykapu csak a
  teljesítmény miatt kizárt vagy nem preferált technológiákra vonatkozik; a licenc-, biztonsági,
  gyártófüggetlenségi vagy szabadalmi okból kizártakat nem oldja fel (azokról továbbra is külön
  tulajdonosi döntés kell, ahogy az AGPL-nél 2026-11-22-én). E nélkül a szabály szövege alapján
  egy agent pl. zárt plug-int vagy NDI-t is „mérés után beépíthetőnek” tekinthetne.
- **D2 – A mérési bizonyíték helye (elfogadva: a technológiát behozó terv vagy ADR).** Ugyanúgy,
  mint a „Port or keep” szabálynál; külön nyilvántartás nem készül. Ha a mérés kimarad, a
  `deferred-physical-tests.md`-be kerül egy sor (§2.14), és az alkalmazhatóság „előzetes
  javaslat” marad.
- **D4 – A Conda végleges kizárása (a tulajdonos döntése, 2026-11-28: „A Conda legyen
  kizárva”).** A Conda nem esik a teljesítménykapu alá, mérés után sem építhető be. Javasolt
  hatókör: az egész Conda-család (Anaconda, Miniconda, Miniforge, Mamba, micromamba,
  conda-forge-környezetek), a termékben, a csomagokban, a CI-ben, a fejlesztői eszközökben és az
  AI-workerekben egyaránt. Eddig csak 9 tervben szerepelt külön-külön (pl.
  CFA4-SMART-CUT-RENDER-R1, CFA4-AI-MODEL-TRAINING-001, CFA4-DEV-AI-ORCHESTRATION-001 13. pont),
  az `AGENTS.md`-ben nem; ezek a tervek változatlanok maradnak, mert egyeznek az új szabállyal.
  Elfogadva: a teljes családra vonatkozik.
- **D3 – Magyar összefoglaló a `docs/licensing.md`-ben (elfogadva: nem).** A szabály nem
  licencszabály, és egy helyen kell élnie; a magyar szöveg ebben a tervben megmarad.

---

## 5. Elfogadási feltételek

- Az `AGENTS.md` CFA4-szakaszában a „Non-Rust components” után ott a fenti pont, a Conda-kizárással
  együtt; a terv a `docs/plans/CFA4-RULE-PERF-GATE-001.md` fájlban.
- Más fájl nem módosul; a szabály az `AGENTS.md`-n kívül sehol nincs megismételve.
- A PR leírása hivatkozik erre a tervre; a beolvasztás a gazdán keresztül megy. A PR a
  szabályfájlt érinti, ezért a CodeRabbit-szabály (D5) szerint kockázatosnak számít, hacsak nem
  adsz rá kivételt.
