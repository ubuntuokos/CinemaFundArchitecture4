# Feature Specification: Mért paritás bővítése – parancsok, panelek, hűségi szintek, commitolt referencia, CI (G7)

**Feature Branch**: `[###-feature-name]` – a Spec Kit nem nyit ágat; az ágat a munkamenet nyitja a `docs/coordination.md` szerint.

**Created**: 2026-11-14

**Status**: Draft

**Input**: User description: "CFA4-PARITY-G7 – a `cargo xtask parity` mérőeszköz bővítése (docs/gaps.md G7). Ma a `cargo xtask parity` a menüsort (#781), valamint az Effects panel effektjeit és átmeneteit (`--area effects`, `--area transitions`, #831) méri egy referencialistához képest; a referencia alapértelmezésben a helyi, gitignore-olt `plan/premiere/` alatti JSON. A G7 hiányai: 1. További mért területek: parancsok (a `crates/engine` parancsregisztere) és panelek. 2. Hűségi szintek a jelentésben: pontos (exact), közelítő (approximate), csonk (stub). 3. A repóba commitolható, Adobe-asset-mentes referencialista (csak saját szöveges útvonalak, AGENTS.md §1–2 szerint), hogy a mérés `plan/` nélkül is fusson. 4. CI-job, amely a `--min` küszöb alatt hibát ad. 5. Commitolt `docs/parity-checklist.md`. Részletek: docs/gaps.md G7, docs/target-app-parity.md, xtask forrása. Csak tervezés: követelmények, terv és feladatlista; kódot most nem írunk."

## Háttér (a mai állapot, `main` @ `8b7c0ea99`)

- A paritásmérő ma három területet mér: menüsor (alapértelmezett), effektek, átmenetek. A saját oldalt a
  szállított menüfa, illetve a motor effektdefiníciói adják. A referencia alapértelmezésben a helyi,
  gitignore-olt `plan/premiere/<terület>.json`.
- A jelentés a jelenlétet és az elhelyezést külön méri: ugyanott, máshol, hiányzik. A területen kívüli tételek
  (app-, Help- és fiókmenük, presetek, a másik terület tételei) külön számolódnak.
- Hivatkozási lista nélkül a mérő csak a saját leltárt írja ki, és semmit nem mér. A `--min` küszöböt is csak
  hivatkozási listával fogadja el.
- A jelentés a `target/parity/` alá kerül, tehát nem commitolt, és nem fut CI-ban.
- A docs/gaps.md G7 a hiányok között a beállításokat (preferences) és a formátumokat is felsorolja. Ez a
  specifikáció ezekből **csak a parancsokat és a paneleket** veszi fel (lásd Assumptions).

## User Scenarios & Testing *(mandatory)*

### User Story 1 – Mérés `plan/` nélkül, commitolt referenciával (Priority: P1)

Egy fejlesztő vagy ügynök friss klónban, a helyi `plan/premiere/` mappa nélkül futtatja a paritásmérőt
a menük, effektek és átmenetek területén. A mérő a repóban lévő, Adobe-asset-mentes referencialistát
használja, és ugyanolyan jelenlét- és elhelyezési jelentést ad, mint ma a helyi pillanatképpel.

**Why this priority**: E nélkül sem a CI-job (US4), sem a commitolt checklist (US5) nem lehetséges, és ma a
paritásszámot csak a karbantartó tudja előállítani. Ez az alapja minden további lépésnek.

**Independent Test**: Friss klónban, `plan/` mappa nélkül futtatjuk a mérőt mindhárom meglévő területre.
Mindegyik ad egy százalékos jelenlétszámot, és egyik sem a „nincs referencia” ágon áll meg.

**Acceptance Scenarios**:

1. **Given** friss klón `plan/` nélkül, **When** a menüterületet mérjük hivatkozás megadása nélkül,
   **Then** a mérő a commitolt referenciát használja, és jelenlétszámot ad.
2. **Given** a karbantartó gépe, ahol a `plan/premiere/<terület>.json` is megvan, **When** hivatkozás
   megadása nélkül mérünk, **Then** a jelentés első sora megnevezi, melyik referenciát használta. A sorrendet
   lásd FR-006.
3. **Given** a commitolt referencia, **When** a repó asset- és függetlenségi ellenőrzései lefutnak,
   **Then** egyik sem jelez hibát, és a referencia egyetlen bejegyzése sem képfájl, ikon, képernyőkép vagy más
   asset.

---

### User Story 2 – Parancsok és panelek mérése (Priority: P2)

A fejlesztő két új területet mér. A **parancsok** területén a motor parancsregiszterében lévő parancsokat
(azonosító, címke, menüútvonal) veti össze egy referencialistával. A **panelek** területén a Qt6 GUI
paneljeit veti össze ugyanígy. A jelentés formája ugyanaz, mint a meglévő területeké: jelenlét, elhelyezés,
területen kívüli tételek, csak nálunk meglévő tételek.

**Why this priority**: A G7 fő hiánya. A parancsok a „minden parancs” elv miatt (UI, CLI, vezérlőcsatorna és MCP
egyaránt parancsazonosítóval hív) a funkciólefedettség legjobb mérhető közelítése.

**Independent Test**: A parancsok és a panelek területét külön-külön futtatjuk egy kis, kézzel írt
referenciával. Ebben van egy biztosan meglévő, egy máshol lévő és egy hiányzó tétel, és a jelentés
mindhármat helyesen sorolja be.

**Acceptance Scenarios**:

1. **Given** a parancsok területe, **When** referencia nélkül futtatjuk, **Then** a mérő kiírja a regiszter
   leltárát csoportonként (például menü vagy kategória szerint), és nem mér.
2. **Given** egy referenciatétel, amelynek címkéje nálunk más menüútvonalon van, **When** mérünk,
   **Then** a tétel „máshol” besorolást kap, és a jelentés megmutatja a mi útvonalunkat.
3. **Given** csak billentyűparancsként létező (menü nélküli) parancs, **When** mérünk, **Then** a
   mérő a parancsot a saját csoportjában (például „Keyboard”) helyezi el, és nem dobja el.
4. **Given** a panelek területe, **When** mérünk, **Then** minden referenciapanelről eldől, hogy megvan,
   máshol van-e (például egy másik panel lapjaként) vagy hiányzik.

---

### User Story 3 – Hűségi szintek a jelentésben (Priority: P2)

A jelentés a megtalált tételeknél nemcsak azt mutatja, hogy a tétel létezik-e, hanem azt is, milyen
hűséggel: **pontos** (exact), **közelítő** (approximate) vagy **csonk** (stub). Egy olvasó így látja,
hogy a jelenlétszámból mennyi a valódi, teljes értékű funkció.

**Why this priority**: A docs/gaps.md G1 és G18 szerint ma a közelítő effektek ugyanúgy számítanak, mint a
pontosak. A hűségi szint nélküli jelenlétszám túlbecsüli a készültséget.

**Independent Test**: Egy referenciával mérünk, ahol egy-egy megtalált tétel pontos, közelítő, illetve csonk
besorolású, egy negyediknek pedig nincs besorolása. A jelentés mind a négy állapotot külön oszlopban számolja.

**Acceptance Scenarios**:

1. **Given** egy megtalált tétel, amelynek saját hűségi besorolása „közelítő”, **When** mérünk,
   **Then** a tétel a jelentésben a „közelítő” oszlopban számít, és megjelenik mellette a besorolás rövid oka.
2. **Given** egy megtalált tétel besorolás nélkül, **When** mérünk, **Then** „nem besorolt” állapotot kap,
   és a mérő nem tekinti pontosnak.
3. **Given** egy parancs, amely futtatáskor „nem támogatott” hibát ad, **When** mérünk, **Then** a tétel
   „csonk” besorolást kap, akkor is, ha kézi besorolás nincs hozzá.
4. **Given** bármely terület, **When** mérünk, **Then** a jelentés a jelenlét mellett egy külön
   „pontos jelenlét” százalékot is ad (csak a pontos tételek), és a `--min` küszöb mindkettőre megadható.

---

### User Story 4 – CI-job a küszöbbel (Priority: P3)

Egy CI-job minden mért területen lefuttatja a mérőt a commitolt referenciával, és hibát ad, ha egy
terület jelenléte a megadott küszöb alá esik. A paritás így nem romolhat észrevétlenül, például ha egy
parancs vagy menüpont véletlenül kiesik.

**Why this priority**: A visszaesés elleni védelem csak az US1 után lehetséges. Amíg nincs, a mérés kézzel is
futtatható.

**Independent Test**: Egy ágon szándékosan eltávolítunk egy mért tételt, vagy a küszöböt a mért érték fölé
állítjuk. A job pirosra vált, a naplóban látszik a terület, a mért érték és a küszöb. Visszaállítva zöld.

**Acceptance Scenarios**:

1. **Given** minden terület a küszöbe felett, **When** a job fut, **Then** sikeres, és a naplóban
   területenként látszik a jelenlét, a pontos jelenlét és a küszöb.
2. **Given** egy terület a küszöbe alatt, **When** a job fut, **Then** a job hibát ad, és a napló
   megnevezi a területet, a mért értéket és a küszöböt.
3. **Given** egy PR, amelyen nincs teljes validációs címke, és nem kézi indítás, **When** a PR-ellenőrzések
   futnak, **Then** a paritásjob nem fut. Az automatikus PR-ellenőrzés gyors marad (AGENTS.md CFA4-szakasz,
   CI-szabály).

---

### User Story 5 – Commitolt `docs/parity-checklist.md` (Priority: P3)

A repóban van egy olvasható `docs/parity-checklist.md`. Területenként tartalmazza a mért tételeket,
jelenléttel, elhelyezéssel és hűségi szinttel, a commitolt referenciából generálva. Egy közreműködő a
repóból, a mérő futtatása nélkül is látja, mi hiányzik.

**Why this priority**: Dokumentációs érték. A mérő és a commitolt referencia megléte után egyetlen paranccsal
előállítható.

**Independent Test**: A checklistet újragenerálva a commitolt fájllal azonos kimenetet kapunk. A fájl
fejléce megnevezi a generáló parancsot és a referencia revízióját.

**Acceptance Scenarios**:

1. **Given** a commitolt referencia és a mostani kód, **When** a checklistet újrageneráljuk, **Then**
   a kimenet bájtra azonos a commitolt fájllal. A generálás determinisztikus: rendezett, és nem tartalmaz
   időbélyeget.
2. **Given** egy parancs hozzáadása, amely egy addig hiányzó referenciatételt fed le, **When** a CI-job fut,
   **Then** jelzi, hogy a commitolt checklist elavult. Ez figyelmeztetés, nem hiba (lásd Assumptions).

---

### Edge Cases

- **Ellenséges referencia** (AGENTS.md §0.4–0.5): túl mély, túl sok tételt tartalmazó, túl hosszú útvonalú
  vagy hibás JSON mindegyik új területen és a hűségi besorolásokban is hibaüzenetet ad, összeomlást nem.
  Ugyanazok a korlátok érvényesek, mint ma.
- **Üres referencia vagy üres terület**: 0% jelenlét nullával osztás nélkül. Ha a `--min` meg van adva,
  egyértelmű hiba.
- **Hűségi besorolás nem létező tételre** (elírt azonosító, törölt parancs): a jelentés külön listázza
  „árva besorolásként”. A CI-job ezt figyelmeztetésként jelzi.
- **Ugyanaz a címke több helyen** (például egy parancs két menüben): egyszer számít jelenlétként.
  Az elhelyezés „ugyanott”, ha bármelyik helye egyezik.
- **Panelek, amelyek egy másik panel lapjai vagy alnézetei**: „máshol” besorolást kapnak, nem „hiányzik”-ot.
- **Commitolt és helyi referencia eltér**: a jelentés megnevezi a használt referenciát. A CI mindig a
  commitoltat használja.
- **Lokalizált címkék**: a mérés a forrásnyelvi (angol) címkékkel dolgozik. A fordítás (`i18n/`) nem
  befolyásolja.
- **A saját oldal lekérése sikertelen** (például nem fordul a lekérő példa): a mérő a terület nevével és az
  okkal hibát ad, részeredményt nem.

## Requirements *(mandatory)*

### Functional Requirements

**Új területek**

- **FR-001**: A mérőnek MUST legyen **parancsok** területe. Ennek saját oldala a motor parancsregisztere:
  minden regisztrált parancs azonosítóval, címkével, menüútvonallal (ha van), és a menü nélküli parancsok egy
  külön csoportban.
- **FR-002**: A mérőnek MUST legyen **panelek** területe. Ennek saját oldala a **Qt6 GUI**
  (`python/cfa4_qt/`) paneljeinek listája, a panel nevével és helyével (önálló panel, vagy melyik panel
  lapja). A befagyasztott egui szerkesztő paneljeit ez a terület nem méri (tulajdonosi döntés, 2026-11-14, Q1).
- **FR-003**: Az új területek MUST ugyanazt a jelentésformát adják, mint a meglévők: jelenlét, elhelyezés,
  területen kívüli tételek, csak nálunk meglévő tételek, csoportonkénti összesítő. Referencia nélkül csak
  leltárt adnak.
- **FR-004**: A parancsok területe MUST kizárja azokat a tételeket, amelyek a CFA4 szabályai miatt soha nem
  készülnek el (fiók-, felhő-, licenc- és fizetős funkciók, más platform rendszerelemei). A kizárás oka
  külön számolódik, ahogy ma a menüknél.

**Commitolt referencia**

- **FR-005**: A repó MUST tartalmazzon egy commitolt referencialistát minden mért területhez (menük, effektek,
  átmenetek, parancsok, panelek). A lista csak szöveges útvonalakat és neveket tartalmaz. Képet, ikont,
  képernyőképet, betűtípust, presetet vagy más assetet nem (AGENTS.md §1).
- **FR-006**: Ha nincs megadva hivatkozás, a mérő MUST a commitolt referenciát használja. A helyi
  `plan/premiere/` pillanatkép csak kifejezett kérésre (hivatkozásként megadva) használható. A jelentés MUST
  megnevezze a használt referenciát és annak revízióját.
- **FR-007**: A commitolt referencia eredetének MUST meg kell felelnie AGENTS.md §1.5-nek (a helyi
  referenciaanyag nem kerülhet a repóba) és §2-nek (tisztaszobás munka). A `plan/premiere/` fájljaiból nem
  másolható és nem exportálható. A referencia **csak olyan neveket** tartalmazhat, amelyek már a commitolt
  CFA4-dokumentációban szerepelnek (például `docs/target-app-parity.md`, `docs/keyboard.md`, az effektlisták).
  Új külső anyag nem kerül be, a `plan/premiere/` tartalmából semmi (tulajdonosi döntés, 2026-11-14, Q3). Minden
  referenciatételnek MUST legyen forrásmegjelölése: melyik commitolt CFA4-dokumentumban szerepel a név. A
  referencia mellé MUST kerüljön egy rövid leírás arról, hogyan készült.
- **FR-007a**: Egy ellenőrzésnek MUST el kell utasítania azt a referenciatételt, amelynek neve nem szerepel a
  megjelölt commitolt dokumentumban. Így nem kerülhet be új külső név.
- **FR-008**: A commitolt referencia MUST átmenjen a repó meglévő asset- (`cargo xtask assets`) és
  függetlenségi ellenőrzésein.

**Hűségi szintek**

- **FR-009**: A jelentés MUST minden megtalált tételhez hűségi szintet rendeljen: pontos, közelítő, csonk vagy
  nem besorolt.
- **FR-010**: A hűségi szintek forrása MUST a repóban lévő, a saját funkciókról szóló besorolás legyen
  (tételazonosító → szint és rövid indok). Egy tétel csak bizonyítékkal lehet „pontos”: hivatkozott teszttel
  vagy méréssel, a VISION.md 2.5 („bizonyíték, nem állítás”) szerint. Bizonyíték nélküli tétel legfeljebb
  „közelítő”.
- **FR-011**: Az a parancs, amelyet a motor „nem támogatott” állapotúnak jelez, MUST „csonk” szintet kapjon
  kézi besorolás nélkül is.
- **FR-012**: A jelentés MUST adjon területenként egy „pontos jelenlét” százalékot (csak a pontos tételek),
  a meglévő jelenlétszám mellett.
- **FR-013**: A küszöbkapcsoló MUST legyen megadható a jelenlétre és külön a pontos jelenlétre is.

**CI**

- **FR-014**: Egy CI-job MUST futtassa a mérőt minden területen a commitolt referenciával, és MUST hibát adjon,
  ha bármely terület a hozzá rendelt küszöb alatt van. A napló területenként mutatja a mért értéket és a
  küszöböt.
- **FR-015**: A job MUST csak kézi indításra vagy a teljes validációs címkére fusson, ne minden PR-on
  (AGENTS.md CFA4-szakasz: az automatikus PR-ellenőrzés csak a `cargo-check` kapu). Követi a CodeRabbit-
  szabályt, mert `.github/` fájlt érint (`docs/coordination.md` §2.9).
- **FR-016**: A területenkénti küszöbök MUST a repóban, egy helyen legyenek rögzítve, és a jobnak MUST ezeket
  kell használnia. **Racsni** (tulajdonosi döntés, 2026-11-14, Q2): minden terület küszöbe a bevezetéskor mért
  értékről indul, és csak felfelé mozdulhat. Ez a jelenlétre és a pontos jelenlétre egyaránt érvényes.
- **FR-016a**: Egy ellenőrzésnek MUST hibát kell adnia, ha egy változtatás bármely küszöböt a `main`-en
  rögzített érték alá csökkenti. A csökkentés csak külön tulajdonosi döntéssel lehetséges.
- **FR-016b**: Ha a mért érték a küszöb fölé emelkedik, a job MUST jelezze, hogy a küszöb emelhető. Az emelés
  egy commitolt változtatás; a job maga nem írja át a küszöböt.

**Commitolt checklist**

- **FR-017**: A repó MUST tartalmazzon egy `docs/parity-checklist.md` fájlt, amelyet a mérő egyetlen
  paranccsal, a commitolt referenciából generál, minden területről, hűségi szintekkel.
- **FR-018**: A generálás MUST determinisztikus legyen: stabil sorrend, nincs időbélyeg és nincs gépfüggő
  útvonal. Azonos bemenetre bájtra azonos fájlt ad.
- **FR-019**: A CI-job MUST jelezze, ha a commitolt checklist eltér a frissen generálttól (figyelmeztetésként).

**Stabilitás és hatókör**

- **FR-020**: Minden új bemenet (referencia, hűségi besorolás, saját oldali lekérés, kapcsolók) MUST ellenséges
  bemenetnek számítson (AGENTS.md §0.4–0.5): mélység-, tételszám- és hosszkorlát, hibaüzenet összeomlás
  helyett. Az új bemenetekhez MUST legyenek ellenséges-bemeneti tesztek (§0.8).
- **FR-021**: A meglévő területek (menük, effektek, átmenetek) eredménye és kapcsolói MUST változatlanok
  maradjanak, ha ugyanazzal a referenciával futnak. Az új oszlopok hozzáadódnak, a régiek nem változnak.
- **FR-022**: A beállítások (preferences) és formátumok területe MUST NOT legyen ennek a funkciónak a része.
  A docs/gaps.md G7 ezeket nyitott hiányként tartja meg.

### Key Entities

- **Terület (area)**: a mért tartomány (menük, effektek, átmenetek, parancsok, panelek). Megvan a saját
  oldali forrása, az alapértelmezett referenciája, a területen kívüli szabályai és a küszöbe.
- **Referenciatétel**: egy szöveges útvonal vagy név a referencialistában. Csoportja (a legfelső elem) és
  címkéje van.
- **Saját tétel**: a CFA4 által szállított elem (menüpont, effekt, parancs, panel), azonosítóval és útvonallal.
- **Hűségi besorolás**: saját tétel → szint (pontos, közelítő, csonk), rövid indok és bizonyítékhivatkozás
  (pontos szinthez kötelező).
- **Küszöb**: területenként a minimális jelenlét és a minimális pontos jelenlét százalékban.
- **Jelentés / checklist**: a mérés eredménye területenként. Megnevezi a forrásreferenciát és annak
  revízióját.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Friss klónban, `plan/` nélkül mind az öt terület mérhető, és mindegyik jelenlétszámot ad.
  Egyik sem áll meg „nincs referencia” üzenettel.
- **SC-002**: A teljes ötterületes mérés egyetlen paranccsal futtatható, és a referenciagépen előfordított
  állapotból 2 percen belül lefut.
- **SC-003**: A jelentésben a megtalált tételek 100%-a a négy hűségi állapot (pontos, közelítő, csonk, nem
  besorolt) közül pontosan egyben szerepel. „Pontos” tétel bizonyítékhivatkozás nélkül nem fordul elő.
- **SC-004**: Ha egy mért tételt eltávolítunk, és a terület így a küszöbe alá esik, a CI-job egy futáson belül
  hibát ad. A napló megnevezi a területet, a mért értéket és a küszöböt.
- **SC-005**: A `docs/parity-checklist.md` újragenerálása kétszer egymás után, és két különböző gépen is
  bájtra azonos eredményt ad.
- **SC-006**: A commitolt referencia és checklist mellett a repó asset- és függetlenségi ellenőrzései hiba
  nélkül futnak, és egyetlen asset sem kerül a repóba.
- **SC-007**: A meglévő három terület eredménye ugyanazzal a referenciával a bővítés előtt és után
  ugyanaz (jelenlét és elhelyezés tételenként).
- **SC-008**: Ellenséges bemenetek (túl mély, túl sok tétel, túl hosszú, hibás JSON, üres) egyike sem okoz
  összeomlást egyik új bemeneten sem.
- **SC-009**: A menük jelenlétszáma a commitolt referenciával mérve szerepel a checklistben. Mellette egy
  megjegyzés jelzi, hogy ez a szám nem vethető közvetlenül össze a docs/gaps.md G7 „Evidence” pontjának ad hoc
  92%-ával, mert a commitolt referencia csak a CFA4-dokumentációban már szereplő neveket tartalmazza.
- **SC-010**: A commitolt referencia tételeinek 100%-ához van forrásmegjelölés, és mindegyik név megtalálható
  a megjelölt commitolt CFA4-dokumentumban.

## Assumptions

- **Hatókör**: a G7-ből csak a parancsok és a panelek kerülnek be új területként, ahogy a kérés írja. A
  beállítások és formátumok külön feladat maradnak.
- **Hűségi besorolás forrása**: nincs ma a kódban hűségi adat (az effektdefiníciók és a parancsregiszter nem
  tartalmaz ilyet). Ezért új, kézzel karbantartott, commitolt besorolás készül. Az alapértelmezés „nem
  besorolt”, „pontos” csak bizonyítékkal lehet. A „csonk” szint a parancsoknál a „nem támogatott” hibából
  automatikusan is származhat.
- **CI-futás**: a job nem automatikus PR-ellenőrzés. Kézi indításra vagy a `cfa4-full-validation` címkére fut,
  a meglévő teljes validációs workflow-k mintájára.
- **Elavult checklist**: figyelmeztetés, nem hiba. Így egy parancsot hozzáadó PR-nak nem kell kötelezően a
  közös dokumentumhoz nyúlnia, és kevesebb az ütközés (`docs/coordination.md` §2.5).
- **Címkék nyelve**: a mérés a forrásnyelvi címkékkel dolgozik. A fordítások nem számítanak.
- **A panelek forrása**: a Qt6 GUI (Q1). A meglévő menüterület továbbra is az egui menüfát méri. Ezt a G7 nem
  változtatja meg, de a checklist megjelöli, melyik felületről származik a terület adata.
- **A referencia teljessége**: a Q3 döntés miatt a commitolt referencia csak annyit fed le, amennyi név a
  commitolt CFA4-dokumentációban szerepel. Ez parancsokra (`docs/keyboard.md`) várhatóan bőséges, panelekre
  szűkebb. A jelentés a referencia méretét területenként kiírja, hogy a százalék ne tűnjön teljesebbnek, mint
  amilyen. A helyi `plan/premiere/` pillanatkép kifejezett hivatkozásként továbbra is használható, de csak
  helyben, és nem kerül a repóba.
- **Küszöb kezdőértéke**: a racsni (Q2) kezdőértéke az a mért érték, amely a commitolt referenciával
  a CI-job bevezetésekor adódik.
- **Elődök**: #781 (menük) és #831 (effektek, átmenetek) a mérő meglévő alapja. A formátumuk és a korlátaik
  változatlanul maradnak.

## Tulajdonosi döntések (2026-11-14)

A specifikáció három nyitott kérdését a tulajdonos 2026-11-14-én lezárta.

- **Q1 – A panelek területének forrása:** a `--area panels` a **Qt6 GUI** paneljeit méri. Minden új UI-munka
  ott folyik, az egui be van fagyasztva. → FR-002.
- **Q2 – Küszöbök:** **racsni**. Minden terület `--min` küszöbe a bevezetéskor mért értékről indul, és csak
  felfelé mozdulhat. → FR-016, FR-016a, FR-016b.
- **Q3 – A commitolt referencia forrása:** csak olyan nevek, amelyek már a commitolt CFA4-dokumentációban
  szerepelnek (például `docs/target-app-parity.md`, `docs/keyboard.md`, effektlisták). Új külső anyag nem
  kerül be, a `plan/premiere/` tartalmából semmi. → FR-007, FR-007a, SC-010.
- **Jóváhagyás:** a tulajdonos a G7-et 2026-11-14-én a CFA4-DEV-SPEC-KIT-001 SK-F5 pilotfeladataként
  **tervezésre** jóváhagyta. Csak tervezésre: kód nem készül.

## CFA4-követelmény *(kötelező – SK-02)*

| Mező | Érték |
| --- | --- |
| `requirement_id` | docs/gaps.md **G7** („No measured parity tool in CI”); a kérés munkanéven: CFA4-PARITY-G7 |
| `authority_ref` | [docs/gaps.md#g7](../../docs/gaps.md#g7-no-measured-parity-tool-in-ci); [docs/roadmap.md](../../docs/roadmap.md) „Current focus” 4. pont és B4; tulajdonosi döntés 2026-11-14: a CFA4-DEV-SPEC-KIT-001 SK-F5 pilotfeladata (Q1–Q3 és a jóváhagyás: „Tulajdonosi döntések” szakasz) |
| `source_revision` | `main` (= `origin/main`) @ `8b7c0ea99` |
| `task_id` | CFA4-DEV-SPEC-KIT-001 SK-F5 (pilotfeladat, tervezés); a megvalósítás feladatazonosítója: PENDING (elődök: #781, #831) |
| `approval_state` | Tulajdonos, 2026-11-14: jóváhagyva **csak tervezésre** (CFA4-DEV-SPEC-KIT-001 SK-F5 pilot). A megvalósítás jóváhagyása: PENDING |
| `validation_state` | PENDING |
| `evidence_refs` | PENDING (a megvalósítás után: PR, CI-futás, tesztek) |

## Beolvasott CFA4-források

Mind `origin/main` @ `8b7c0ea99` állapotból. A helyi `main` ref elavult (`c294cca01`), ezért nem onnan.

- `.specify/memory/constitution.md` – teljes.
- `AGENTS.md` – a CFA4-szakasz (a fájl elejétől a „CFA4 — rules for agents and contributors” címig), a §0
  („Never crash”), a §1 (Assets) és a §2 (Clean-room) teljes.
- `VISION.md` – 2. pont („Alapelvek”) és 6. pont („Prioritási sorrend”), teljes.
- `docs/coordination.md` – 2. pont (szabályok minden fejlesztő munkamenetnek), teljes.
- `CLAUDE.md` – a munkamenet kontextusában teljes (quality gates, CI, rétegzés).
- `docs/gaps.md` – összefoglaló táblázat és a G7 szakasz; a G1 releváns része.
- `docs/roadmap.md` – a G7-re és a parityre vonatkozó sorok (grep).
- `docs/target-app-parity.md` – címsorstruktúra.
- `xtask/src/parity.rs` – modulleírás, kapcsolók, terület- és hatókörszabályok, korlátok, tesztlista.
- `crates/project/examples/effect_catalog.rs` – teljes. `crates/engine/src/commands.rs` és
  `crates/engine/src/shortcuts.rs` – a `CommandSpec` és a `CommandInfo` helye (grep).
- `.github/workflows/` – lista. A `cfa4-full-validation` címkét használó workflow-k (grep).
- Nem olvastam be: `docs/licensing.md` (a funkció nem hoz be függőséget vagy idegen kódot).
  `plan/` nem létezik ebben a munkafában.
