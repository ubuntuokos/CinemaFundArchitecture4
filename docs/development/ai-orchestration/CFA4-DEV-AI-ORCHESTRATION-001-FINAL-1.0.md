# CFA4-DEV-AI-ORCHESTRATION-001
# FINAL 1.0 – RÉSZLETES INTEGRÁCIÓS ÉS FEJLESZTÉSI TERV

**Projekt:** CinemaFundArchitecture4 (CFA4)  
**Dokumentumazonosító:** CFA4-DEV-AI-ORCHESTRATION-001  
**Verzió:** FINAL 1.0  
**Állapot:** APPROVED / FINAL  
**Jóváhagyás dátuma:** 2026. október 29.  
**Célplatform:** Ubuntu 26.04 LTS / amd64  
**GitHub:** https://github.com/ubuntuokos/CinemaFundArchitecture4

---

# 1. A TERV CÉLJA

A JetBrains ThinkRail, a Pi ügynökmotoron keresztül működő Anthropic Claude-modellek, az önálló Claude Code CLI és a Google Elephant Goldfish Model (EGM) egységes alkalmazása a CFA4 fejlesztési infrastruktúrájában.

A cél egy ellenőrizhető, párhuzamos AI-fejlesztési rendszer kialakítása, amelyben:

- Az ügynökök egymástól elkülönítve dolgoznak.
- A követelmények és fejlesztési döntések tartósan megmaradnak.
- A tervezés, implementáció és ellenőrzés egymást kiegészíti.
- Az elkészült módosítások visszakövethetők.
- A tesztek és jóváhagyások nem kerülhetők meg.
- A fejlesztés hatékonysága mérhető.
- A meglévő CFA4-fejlesztési környezet változatlanul használható marad.

A ThinkRail nem váltja le a CFA4 natív Qt6 GUI-ját, hanem a CFA4 fejlesztésének eszköze lesz.

# 2. VÉGLEGES ARCHITEKTÚRA

## 2.1. CFA4 Development Authority

A központi követelmény- és jóváhagyási rendszer határozza meg:

- A jóváhagyott fejlesztési feladatokat.
- Az architekturális szabályokat.
- A végrehajtási jogosultságokat.
- A fejlesztési hatóköröket.
- A tesztelési követelményeket.
- A merge- és release-kapukat.

## 2.2. JetBrains ThinkRail

Feladatai:

- Központi fejlesztési munkafelület.
- Git worktree-k kezelése.
- Elkülönített branch-ek.
- SPEC-gráf megjelenítése.
- Fájlok és módosítások ellenőrzése.
- Fejlesztői terminálok.
- GitHub PR-integráció.
- CI-eredmények megjelenítése.
- Pi-munkamenetek koordinálása.

## 2.3. Pi + Claude

Elsődleges feladatai:

- Követelményelemzés.
- Architektúratervezés.
- Specifikációk értelmezése.
- Fejlesztési koordináció.
- Másodlagos kódellenőrzés.
- EGM-eredmények feldolgozása.
- Dokumentációs feladatok.

## 2.4. Claude Code CLI

Elsődleges feladatai:

- Rust/Cargo fejlesztés.
- C++ és Qt6 implementáció.
- FFmpeg és kodekfejlesztés.
- GPU-integráció.
- Hibajavítás.
- Tesztimplementáció.
- Natív build és tesztfuttatás.

A Claude Code CLI külön folyamat és külön ügynökmunkamenet marad.

## 2.5. Google Elephant Goldfish Model

Az EGM két alapvető szerepkört biztosít.

**Elephant:**

- Mély projektkontextus feldolgozása.
- Követelmények értelmezése.
- Architektúra és műszaki tervek készítése.
- Specifikációk előállítása.

**Goldfish:**

- Friss kontextusú független ellenőrzés.
- Feltételezések és hiányok feltárása.
- Műszaki ellentmondások keresése.
- Implementációs kockázatok vizsgálata.

Az EGM elsődleges, dokumentált végrehajtója a Claude Code CLI.

A ThinkRail Pi + Claude oldali közvetlen EGM-review csak külön kompatibilitási ellenőrzés után engedélyezhető.

# 3. TC-01–TC-10 INTEGRÁCIÓS KOMPONENSEK

## TC-01 – Agent Policy Adapter

**Cél:** egységes CFA4-fejlesztési szabályok biztosítása minden ügynök számára.

Megvalósítandó képességek:

1. Közös szabályforrás kezelése.
2. Pi-kompatibilis instrukciók előállítása.
3. Claude Code CLAUDE.md szabályok biztosítása.
4. CFA4 célplatform-korlátozások alkalmazása.
5. Jóváhagyási követelmények kezelése.
6. [upstream] upstream módosításának tiltása.
7. CI- és tesztkapuk megkerülésének tiltása.
8. Eltérő ügynökszabályok felismerése.

**Elfogadási feltétel:** ugyanarra a feladatra mindkét ügynök azonos érvényes CFA4-korlátozásokat kap, és a tiltások végrehajtási szinten is érvényesülnek.

## TC-02 – Workspace Coordinator

**Cél:** párhuzamos ügynökök biztonságos elkülönítése.

Alapszabály:

**Egy aktív író munkamenethez egy kizárólagosan írható Git worktree tartozik.**

Feladatok:

1. Feladatonként külön worktree létrehozása.
2. Külön fejlesztési branch kezelése.
3. Munkaterület-tulajdonjog nyilvántartása.
4. Párhuzamos írási konfliktusok blokkolása.
5. Olvasási jogosultságú review támogatása.
6. Megszakított munkamenetek helyreállítása.
7. Árva zárolások ellenőrzése.
8. Biztonságos ügynökátadás.

**Elfogadási feltétel:** az ügynökök nem módosítják egymás munkáját koordinálatlanul.

## TC-03 – Context/Spec Bridge

**Cél:** a CFA4 hivatalos követelményeinek egységes átadása.

Feladatok:

1. A CFA4 jóváhagyott dokumentációjának használata elsődleges követelményforrásként.
2. ThinkRail SPEC.md gráf kialakítása.
3. Modulok és követelmények összekapcsolása.
4. Pi Agent Skills támogatása.
5. Claude Code projektszabályok támogatása.
6. Követelményazonosítók megőrzése.
7. Specifikációk és GitHub-feladatok összerendelése.
8. Ellentmondó követelmények blokkolása.

**Elfogadási feltétel:** a Pi és a Claude Code ugyanazon jóváhagyott követelményverzió alapján dolgozik.

## TC-04 – Evidence Collector

**Cél:** minden fejlesztési eredmény bizonyíthatósága.

Kötelező bizonyítékadatok:

- Task ID.
- Agent.
- Model.
- Base SHA.
- Head SHA.
- Worktree.
- Diff.
- Build/test.
- CI run.
- Approval.
- Status.

Feladatok:

1. Git-commitok rögzítése.
2. Diffek összegyűjtése.
3. Buildnaplók kezelése.
4. Teszteredmények tárolása.
5. GitHub CI-futtatások összekapcsolása.
6. Jóváhagyások nyilvántartása.
7. Bizonyítékok módosításvédett kezelése.
8. VERIFIED, PENDING és BLOCKED állapotok elkülönítése.

**Elfogadási feltétel:** bizonyíték nélkül nincs sikeres fejlesztési lezárás.

## TC-05 – Approval & Gate Controller

**Cél:** kötelező emberi jóváhagyások kikényszerítése.

Különösen védett műveletek:

- PR-merge.
- Release.
- Architektúramódosítás.
- CI-kapu módosítása.
- Rendszerszintű konfigurációváltozás.
- Hitelesítő adatok kezelése.

Feladatok:

1. Feladatengedélyek kezelése.
2. Jóváhagyási hatókör ellenőrzése.
3. Jogosulatlan műveletek blokkolása.
4. GitHub branch protection integráció.
5. Required checks kezelése.
6. Állapotátmenetek validálása.
7. Merge-engedély külön ellenőrzése.

**Elfogadási feltétel:** nincs jogosulatlan merge, release vagy védett módosítás.

## TC-06 – EGM Workflow Adapter

**Cél:** a Google EGM integrációja a CFA4 fejlesztési folyamatába.

Támogatandó EGM-skillek:

1. egm-brainstorm.
2. egm-draft.
3. egm-review.
4. egm-resolve.
5. egm-bootstrap.
6. egm-review-loop.
7. egm-drift-report.

Feladatok:

- EGM-tervek összekapcsolása CFA4-feladatokkal.
- Claude Code EGM-futtatás támogatása.
- Pi-kompatibilitási kapu kialakítása.
- Tervezési dokumentumok kezelése.
- Review-jelentések átadása.
- Upstream-kompatibilis adapteres működés.

**Elfogadási feltétel:** a Claude Code-os EGM-folyamat reprodukálhatóan működik.

## TC-07 – Fresh Context Validator

**Cél:** a Goldfish-review tényleges függetlenségének igazolása.

Feladatok:

1. Új alügynök létrehozásának ellenőrzése.
2. Előző tervezői beszélgetés kizárása.
3. Szükséges forrásdokumentumok olvasásának igazolása.
4. Reviewer-perszónák ellenőrzése.
5. Olvasási jogosultságok korlátozása.
6. Review-végrehajtási nyomok megőrzése.
7. Hiányos izoláció esetén blokkolás.

**Elfogadási feltétel:** a Goldfish-review igazoltan friss kontextusban, a szükséges források elérésével fut.

## TC-08 – Design Review Registry

**Cél:** a műszaki ellenőrzések nyilvántartása.

Feladatok:

1. Review-fázisok követése.
2. Megállapítások tárolása.
3. Súlyossági besorolás.
4. Javítások összekapcsolása.
5. Elfogadott kompromisszumok rögzítése.
6. Dokumentumverziók megőrzése.
7. Git-commitok összerendelése.
8. Nyitott blokkolók kezelése.

**Elfogadási feltétel:** minden kritikus megállapítás ellenőrizhető állapotú.

## TC-09 – Specification Drift Monitor

**Cél:** a specifikáció és a tényleges implementáció eltéréseinek felismerése.

Feladatok:

1. egm-drift-report alkalmazása.
2. SPEC és forráskód összevetése.
3. Hiányzó funkciók azonosítása.
4. Eltérő adatmodellek felismerése.
5. Architekturális eltérések jelzése.
6. Drift-megállapítások GitHub-feladatokhoz rendelése.
7. Kiadást blokkoló eltérések kezelése.

**Elfogadási feltétel:** a releváns eltérések kimutathatók és nyomon követhetők.

## TC-10 – Agent Efficiency Evaluator

**Cél:** a fejlesztési rendszer hatékonyságának mérhető igazolása.

Mérőszámok:

- Teljes feladatátfutási idő.
- Tokenfelhasználás.
- API-költség.
- CI-futtatások eredménye.
- Újramunkálási arány.
- Regressziós hibák.
- Review-terhelés.
- Specifikációs eltérések.

Összehasonlítási csoportok:

**A:** jelenlegi CFA4-fejlesztési folyamat.

**B:** ThinkRail + Pi + Claude + Claude Code.

**C:** B rendszer + EGM.

**Elfogadási feltétel:** dokumentált mérési adatok alapján eldönthető, hol növeli a rendszer a hatékonyságot, és hol okozna indokolatlan többletköltséget.

# 4. F0–F9 FEJLESZTÉSI FÁZISOK

## F0 – Kiinduló állapot auditja

Feladatok:

1. Aktuális CFA4 repository ellenőrzése.
2. Branch-ek és nyitott PR-ek vizsgálata.
3. Fejlesztési környezet felmérése.
4. CI-konfiguráció ellenőrzése.
5. Meglévő fejlesztőeszközök azonosítása.
6. Függőségek és verziók ellenőrzése.
7. Meglévő képességek feltérképezése.
8. Hiányzó integrációs képességek meghatározása.

**Kilépési feltétel:** ellenőrzött baseline és dokumentált gap analysis.

## F1 – Kompatibilitási ellenőrzés

Feladatok:

1. ThinkRail Ubuntu 26.04 amd64 működésének ellenőrzése.
2. Pi ügynökmotor validálása.
3. Claude-modellkapcsolat tesztelése.
4. Claude Code CLI működésének ellenőrzése.
5. GitHub-jogosultságok vizsgálata.
6. Függőségi és biztonsági ellenőrzés.
7. Host-rendszer sértetlenségének igazolása.

**Kilépési feltétel:** hitelesítési és működési PASS.

## F2 – Szabály- és SPEC-integráció

Feladatok:

1. TC-01 kialakítása.
2. TC-03 kialakítása.
3. Közös szabályforrás bevezetése.
4. Pi és Claude Code konfigurációk összerendelése.
5. SPEC-gráf létrehozása.
6. Követelményparitási tesztek.

**Kilépési feltétel:** szabály- és követelményparitás PASS.

## F3 – Izoláció és jogosultságkezelés

Feladatok:

1. TC-02 kialakítása.
2. TC-05 kialakítása.
3. Git worktree-zárak tesztelése.
4. Párhuzamos írás blokkolása.
5. Jogosulatlan GitHub-műveletek blokkolása.
6. Helyreállítás ellenőrzése.

**Kilépési feltétel:** izolációs és jogosultsági PASS.

## F4 – Pi + Claude pilot

Feladatok:

1. Izolált Pi-munkamenet.
2. Claude-modellkapcsolat.
3. CFA4-specifikációk betöltése.
4. Engedélyezett tesztfeladat végrehajtása.
5. Eredmények rögzítése.
6. Munkamenet-helyreállítás ellenőrzése.

**Kilépési feltétel:** reprodukálható Pi + Claude eredmény.

## F5 – Claude Code CLI pilot

Feladatok:

1. Izolált Claude Code-munkamenet.
2. CFA4-projektszabályok alkalmazása.
3. Engedélyezett fejlesztési feladat.
4. Build és teszt.
5. Diff és commit ellenőrzése.
6. Eredmények rögzítése.

**Kilépési feltétel:** reprodukálható Claude Code eredmény.

## F6 – Kétirányú ügynökátadás

Feladatok:

1. Strukturált átadási csomag.
2. Feladatazonosító átadása.
3. SPEC-hivatkozások megőrzése.
4. Commit és diff átadása.
5. Teszteredmények átadása.
6. Nyitott blokkolók megőrzése.
7. Írási jogosultság átruházása.
8. Átvevő ügynök állapotellenőrzése.

**Kilépési feltétel:** handoff PASS.

## F7 – CI és bizonyítékintegráció

Feladatok:

1. TC-04 integráció.
2. TC-08 integráció.
3. TC-09 integráció.
4. GitHub CI-kapcsolat.
5. Kötelező tesztállapotok kezelése.
6. PR-review integráció.
7. Megkerülhetetlen elfogadási kapuk.

**Kilépési feltétel:** CI és review-kapuk PASS.

## F8 – Párhuzamos fejlesztési pilot

Feladatok:

1. Több elkülönített worktree.
2. Párhuzamos ügynökmunka.
3. Konfliktuskezelési tesztek.
4. Integrációs ellenőrzés.
5. Regression tesztek.
6. Bizonyítékok kiértékelése.

**Kilépési feltétel:** konfliktusmentes, validált működés.

## F9 – Kontrollált üzemi bevezetés

Feladatok:

1. Elfogadási kapuk ellenőrzése.
2. Pilot-eredmények kiértékelése.
3. Üzemeltetési dokumentáció véglegesítése.
4. Visszaállítási eljárások ellenőrzése.
5. Kijelölt CFA4-fejlesztési területek bekapcsolása.
6. Felhasználói elfogadás.

**Kilépési feltétel:** igazolt és jóváhagyott üzemi működés.

# 5. E0–E6 EGM-INTEGRÁCIÓS FÁZISOK

## E0 – EGM audit

Feladatok:

- Licencellenőrzés.
- Függőségi audit.
- Pluginstruktúra vizsgálata.
- Reviewer-perszónák azonosítása.
- Biztonsági és kompatibilitási ellenőrzés.

**Kilépési feltétel:** audit PASS.

## E1 – Claude Code EGM pilot

Feladatok:

- Teljes EGM-plugin telepítési próbája.
- Skillbetöltés ellenőrzése.
- Reviewer-perszónák ellenőrzése.
- Izolációs teszt.
- Működési bizonyítékok rögzítése.

**Kilépési feltétel:** teljes plugin működik.

## E2 – EGM SPEC-integráció

Feladatok:

- CFA4 követelmények összekapcsolása.
- concepts_brief.md kezelése.
- design_draft.md kezelése.
- Jóváhagyott tervdokumentumok kezelése.
- SPEC és EGM-verziók összerendelése.

**Kilépési feltétel:** követelménylánc igazolt.

## E3 – Goldfish-review validáció

A kötelező review-fázisok:

1. Stage 5 – Initial Zoom Out Review.
2. Stage 6 – Comprehension Review.
3. Stage 7 – Design Critique.
4. Stage 8 – Intermediate Zoom Out Review.
5. Stage 9 – Implementation Readiness.

A fázisok kötött sorrendben követik egymást.

Egy fázison belül megengedett a párhuzamos reviewer-futtatás.

**Kilépési feltétel:** review és friss kontextusú izoláció PASS.

## E4 – Pi + EGM kompatibilitás

Feladatok:

- EGM-skillek Pi alatti elérhetőségének ellenőrzése.
- Reviewer-delegáció vizsgálata.
- Kontextusizoláció tesztelése.
- Perszónaolvasás igazolása.
- Teljes review-folyamat kipróbálása.

**Kilépési feltétel:** PASS vagy dokumentált opcionális kihagyás.

Sikertelen E4 esetén az EGM közvetlen Pi-futtatása kikapcsolva marad, de a Claude Code-os EGM-integráció folytatható.

## E5 – Drift és bizonyítékkezelés

Feladatok:

- EGM Drift Report integráció.
- TC-08 és TC-09 összekapcsolása.
- Specifikáció és forráskód eltéréseinek ellenőrzése.
- Review-bizonyítékok tárolása.
- GitHub PR-ekhez kapcsolás.

**Kilépési feltétel:** eltérések és bizonyítékok követhetők.

## E6 – Hatékonysági pilot

Feladatok:

- A, B és C fejlesztési csoport összehasonlítása.
- Időmérés.
- Költségmérés.
- Regressziós hibák összehasonlítása.
- Újramunkálás értékelése.
- EGM többletköltségének elemzése.
- Elfogadási javaslat.

**Kilépési feltétel:** dokumentált és értékelhető minőségi, valamint gazdasági eredmény.

# 6. MEGVALÓSÍTÁSI FÜGGŐSÉGEK

Az előírt sorrend:

1. F0–F3: alapinfrastruktúra és biztonsági kapuk.
2. F4–F5: Pi + Claude és Claude Code külön pilot.
3. E0–E3: EGM és Goldfish validáció, az alapinfrastruktúra után párhuzamosan végezhető.
4. F6–F7 és E4–E5: átadás, CI, review és drift-integráció.
5. F8 és E6: integrált párhuzamos pilot, hatékonyságmérés.
6. F9: kontrollált üzemi bevezetés.

Az aktív Qt6 GUI- és kodekfejlesztés saját jóváhagyott ütemezése nem állítható le az AI-fejlesztési infrastruktúra bevezetése miatt.

# 7. AC-01–AC-07 ELFOGADÁSI KAPUK

## AC-01 – Platformkompatibilitás

**Követelmény:** a fejlesztési rendszer Ubuntu 26.04 LTS amd64 környezetben működik.

Ellenőrizendő:

- ThinkRail.
- Pi.
- Claude-modellkapcsolat.
- Claude Code CLI.
- Szükséges függőségek.
- Git/GitHub-integráció.
- Host Non-Interference.

**Elfogadás:** valós környezeti teszt PASS.

## AC-02 – Ügynökműködés

**Követelmény:** a Pi + Claude és a Claude Code CLI külön-külön működik.

Ellenőrizendő:

- Modellhozzáférés.
- Feladatvégrehajtás.
- Eszközhasználat.
- Munkamenet-izoláció.
- Eredménykövetés.
- Helyreállítás.

**Elfogadás:** mindkét végrehajtási útvonal PASS.

## AC-03 – EGM-validáció

**Követelmény:** Claude Code alatt az EGM-review működik.

Ellenőrizendő:

- Plugintelepítés.
- Skillbetöltés.
- Reviewer-perszónák.
- Friss kontextusú alügynökök.
- Forrásdokumentumok beolvasása.
- Teljes review-folyamat.

**Elfogadás:** review és kontextusizoláció PASS.

## AC-04 – Koordináció

**Követelmény:** biztonságos párhuzamos munkaterület-kezelés és ügynökátadás.

Ellenőrizendő:

- Külön worktree-k.
- Kizárólagos írási tulajdon.
- Ütköző írás blokkolása.
- Kontextusátadás.
- Commit- és diffkövetés.
- Teszteredmények megőrzése.

**Elfogadás:** worktree és handoff PASS.

## AC-05 – Biztonság

**Követelmény:** jogosulatlan műveletek blokkolása.

Ellenőrizendő:

- Tiltott fájlmódosítás.
- Jogosulatlan GitHub-merge.
- Nem engedélyezett rendszerművelet.
- CI-kapu megkerülésének blokkolása.
- Korlátozott hitelesítőadat-hozzáférés.
- Jóváhagyási szabályok kikényszerítése.

**Elfogadás:** negatív biztonsági tesztek PASS.

## AC-06 – CFA4 CI-validáció

**Követelmény:** a CFA4 valamennyi releváns, kötelező fejlesztési és release-tesztjének teljesítése.

Ellenőrizendő:

- Rust/Cargo build és tesztek.
- Qt6 GUI.
- Wayland működés.
- FFmpeg 9.0.2 baseline.
- FFmpeg-orákulumok.
- HEIF/HEIC round-trip.
- heif-enc/libheif.
- J2K.
- VFR és phone_vfr.
- Import/export.
- Kodek round-trip.
- CUDA/Vulkan.
- NVENC/NVDEC.
- Natív média E2E.
- Integrációs regresszió.

A korábban azonosított #271 és #331 PR-ek hiányzó tesztjeit, illetve a Qt6 GUI befejezéséhez kötött ellenőrzéseket meg kell őrizni.

**Elfogadás:** minden alkalmazandó, kötelező teszt igazolt PASS eredményt kapott.

SKIPPED, BLOCKED és NOT_RUN nem számít sikernek.

## AC-07 – Hatékonyság

**Követelmény:** az integrált rendszer előnye mérhető és értékelhető.

Vizsgálandó:

- Feladatátfutás.
- Első CI-futás sikeressége.
- Újramunkálás.
- Kritikus regressziók.
- Review-terhelés.
- Teljes token- és API-költség.
- Specifikációeltérések.
- Párhuzamos munkaterület-konfliktusok.

Az összehasonlítás alapja:

- A – jelenlegi fejlesztési folyamat.
- B – kettős ügynökrendszer.
- C – EGM-integrált rendszer.

**Elfogadás:** dokumentált, a felhasználó által elfogadott pilot-eredmények.

# 8. KÖTELEZŐ FEJLESZTÉSI SZABÁLYOK

1. Kizárólag Ubuntu 26.04 LTS és variánsai, amd64 architektúrán.
2. A CFA4 natív Qt6 GUI-iránya változatlan.
3. Rust/Cargo és FFmpeg 9.0.2 követelmények megtartása.
4. [upstream] upstream forrás módosítása tilos.
5. A CFA4-fejlesztések kizárólag a CFA4-be kerülnek.
6. Egy worktree-ben egyszerre legfeljebb egy aktív író ügynök.
7. Minden fejlesztési feladat előzetes jóváhagyáshoz kötött.
8. Automatikus merge és release tilos.
9. A kötelező CI-tesztek nem gyengíthetők.
10. EGM-review nem helyettesíti a natív teszteket.
11. Pi-alapú Goldfish-review csak igazolt kompatibilitás után használható.
12. Software Coexistence & Host Non-Interference kötelező.
13. Conda/mamba nem használható.
14. Külső függőségek és MCP-kapcsolatok jogosultságai ellenőrizendők.
15. Megszakítás esetén biztonságos helyreállítás szükséges.
16. A hagyományos CFA4-fejlesztési folyamatnak az új AI-infrastruktúra nélkül is működőképesnek kell maradnia.

# 9. KÖTELEZŐ DOKUMENTÁCIÓ

A megvalósítás során biztosítani kell a következő dokumentációs tartalmakat:

1. Architecture Decision Record.
2. Agent Policy Specification.
3. Workspace Isolation Specification.
4. Context Bridge Specification.
5. Claude Code Execution Policy.
6. EGM Review Protocol.
7. Evidence Schema.
8. Approval Gate Specification.
9. Compatibility Matrix.
10. Integration Test Plan.
11. Rollback Plan.
12. Efficiency Evaluation Report.

A dokumentáció elhelyezését a tényleges CFA4 repositorystruktúrához kell igazítani.

# 10. VÉGREHAJTÁSI ÉS ELFOGADÁSI MODELL

A fejlesztési állapotgép:

PROPOSED  
→ DESIGN REVIEWED  
→ APPROVED  
→ ASSIGNED / ISOLATED  
→ IMPLEMENTED  
→ VALIDATED  
→ REVIEWED  
→ MERGE AUTHORIZED  
→ MERGED / VERIFIED

Bármely állapotból lehetséges BLOCKED, FAILED vagy CANCELLED állapotba kerülni.

A megszakított feladat újraindítása előtt ellenőrizni kell a worktree-t, a commitállapotot és a jóváhagyás érvényességét.

A felhasználói tervjóváhagyás nem jelent automatikus implementációs vagy merge-engedélyt.

# 11. VÉGLEGES JÓVÁHAGYÁSI HATÁROZAT

**Dokumentum:** CFA4-DEV-AI-ORCHESTRATION-001  
**Verzió:** FINAL 1.0  
**Státusz:** APPROVED / FINAL  
**Jóváhagyás:** 2026. október 29.

A CFA4 AI-fejlesztési infrastruktúrájának továbbfejlesztése a következő egységes rendszerrel jóváhagyott:

**JetBrains ThinkRail + Pi + Claude + Claude Code CLI + Google Elephant Goldfish Model**

A jóváhagyott megvalósítási hatókör:

- TC-01–TC-10 integrációs komponensek.
- F0–F9 fejlesztési fázisok.
- E0–E6 EGM-integrációs fázisok.
- AC-01–AC-07 elfogadási kapuk.

A terv célja a CFA4 fejlesztésének gyorsítása, a hibák és újramunkálás csökkentése, valamint az egymást kiegészítő AI-fejlesztési képességek ellenőrizhető összehangolása.

A korábban jóváhagyott CFA4-architektúra és fejlesztési szabályok változatlanul érvényesek.

**Végrehajtási korlátozás:** a terv jóváhagyása önmagában nem jogosít fel telepítésre, forráskód-módosításra, automatikus PR-merge-re vagy release-re.

---

# 12. DOKUMENTUMSTÁTUSZ

**CFA4-DEV-AI-ORCHESTRATION-001 / FINAL 1.0 / APPROVED**

Ez az eredeti FINAL 1.0 terv részletes, a jóváhagyás előtti beszélgetési tartalomból összeállított változata.

A később elfogadott FINAL 1.1 változat AIQT 1.0.6 kiegészítései nem részei ennek a dokumentumnak.

A FINAL 1.0 a CFA4 AI-fejlesztési infrastruktúrájának megőrzendő történeti tervverziója.
