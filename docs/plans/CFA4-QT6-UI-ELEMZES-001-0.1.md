# CFA4-QT6-UI-ELEMZES-001 – A Qt6-felületi feladatok újraelemzése (0.1)

> **Állapot:** a D-Q1…D-Q3 döntéseket István 2026-11-23 06:01-kor „javaslat szerint” elfogadta (§6); a dokumentum többi része jóváhagyásra vár (`docs/coordination.md` §2.10). Csak dokumentáció, kód nincs benne.
> **Dátum:** 2026-11-23 · **Kérte:** István (ubuntuokos) · **Alap:** CFA4 `main` @ `e493e956`.
> **Olvasott források:** `VISION.md`, `docs/coordination.md`, `AGENTS.md`, `docs/qt-parity.md`, `docs/development/qt-desktop-checks.md`,
> `docs/plans/CFA4-QT6-FINISH-001.md` (1.3), a DNEG/VFX/OpenUSD/N3/EffectChain/Catalog/PointCloud tervek, a #1479 gazda-issue és a nyitott PR-ok listája.
> **Nem hatóköre:** a tervek tartalmát nem módosítja; ahol módosítás kell, azt javaslatként, döntési pontként nevezi meg (§6).

## 1. Összefoglaló

1. **A Qt6 befejezése (QT6-FINISH-001, 1.3) kódban gyakorlatilag kész**; ami hátra van, az **mérés és kézi ellenőrzés a tulajdonos gépén** (Q2, Q3-mérés, Q4.1), nem fejlesztés a felhőben. A `qt-parity.md` 67 sorából 62 ✅, **5 🟡** (1.9, 1.10, 4.14, 5.5 és a jelmagyarázat sora).
2. **A Q5 (egui eltávolítása) négy sorra vár: 1.9, 1.10, 5.5 és a tervben nem szereplő 4.14.** A 4.14 (QC: „a jelölt időpontnál az export lejátszása hiányzik”) a terv 2026-11-08-i táblázata után keletkezett, ezért a FINISH-terv nem tud róla; ez egy **tervmódosítás**, tulajdonosi jóváhagyást kér (D-Q1).
3. **A DNEG/VFX/OpenUSD tervek hét új Qt6-felületi elemet hoznak.** Egyik sem indítható ma: vagy a mögöttes parancsok nincsenek a `main`-ben (#1608 lánc), vagy a fázisengedély/döntés hiányzik (11b/5), vagy a függőség (VFX-2, N1 `main`-be kerülése) nincs kész. Ütközés nincs közöttük, de **két névtani/hatókör-pontatlanság** igen (§4).
4. **Következő logikus, már jóváhagyott Qt6-kódfeladat nincs, amely a felhőből, döntés nélkül indítható** (indoklás: §5). A javaslat: a D-Q1 és a D-Q2 döntés után a 4.14 lezárása, majd az U1-UI szelet a #1608 beolvadása után.

## 2. A QT6-FINISH-001 (1.3) állapota

| Fázis | Terv | Állapot a `main`-ben | Mi hiányzik | Ki teheti |
|---|---|---|---|---|
| Q1 | Export mód (4.2) | **kész** (4.2 ✅) | – | – |
| Q2 | Kézi asztali ellenőrzés (1.9), KDE Plasma, Wayland + X11 | előkészítve: `docs/development/qt-desktop-checks.md`, `scripts/dev/qt-desktop-smoke.sh`; CI-ben Xvfb és weston zöld | a kézi sorok kitöltése a tulajdonos gépén; utána a hibajavító PR-ok | **tulajdonos** (valódi asztal) |
| Q3 | GPU képút (5.5) | 1. lépés kész, opt-in: `CFA4_QT_MONITOR_GL=1` (QOpenGLWidget, tartalék: raszter; `perf.stats playback.present`) | **mérés a referenciagépen** (1080p/4K, kiesett képkockák), majd D2 szerint döntés a dmabufról; az alapértelmezés váltása | **tulajdonos** (mérés); `deferred-physical-tests.md` sor, ha kihagyja |
| Q4 | Csomagolás (1.10) | Q4.3 kész (egyik csomag sem indít egui-t); a PySide6-beágyazás őrzött | az AppImage/Flatpak építése és valódi 26.04-es asztalon ellenőrzése | **tulajdonos** / csomagoló környezet |
| Q5 | egui eltávolítása | nem indulhat | 1.9, 1.10, 5.5, **4.14** ✅ + tulajdonosi jóváhagyás az ADR F8-ban | – |

**Következtetés:** a felhőből ma nem hozható előre Q2–Q4 érdemben; a szűk keresztmetszet a tulajdonos ideje és a beolvasztás (a 42 nyitott PR miatt, `coordination.md` §2.6).

## 3. Qt6-felületi elemek a DNEG / VFX / OpenUSD tervekből

| Azonosító | Elem | Forrás | Függ | Jóváhagyás | Indítható ma? |
|---|---|---|---|---|---|
| **U1-UI (U1-A6)** | minimális `scene3d` asset-panel: `scene3d.list`, A/B út-választó, unsupported-lista, relink, hiányzó-fájl jelzés; csak 3D-projektben | OPENUSD-3D-001 §11b/5 | #1608 (`scene3d.*` parancsok, séma v18) a `main`-ben | **nem**: „javaslat, döntésre vár” (István még nem fogadta el); az U1 fázisengedély megvan (18:03) | **nem** |
| **U2-A4** | teljes importpanel (USD, rétegek, referencialista, út-választó) | OPENUSD-3D-001 §8 | U2 kapuk: render C0, G-USD-1; TOST-1.0 | fázisengedély megvan, kapuk nyitva | **nem** |
| **U3a-A8** | `scene3d.export` párbeszéd (formátum, mappa, veszteség-lista) | OPENUSD-3D-EXPORT-001 | U1 | jóváhagyva (E-1…E-5), fázisengedély megvan | **nem** (U1 előbb) |
| **N1-10** | „Voxel-attribútumok” panel a médiaböngészőben | DNEG-ADAPT-001 1.0 | #1503 (N1, `cfa4-vdb` + worker) a `main`-ben | az N1 jóváhagyott, a kód PR-ban | **nem** |
| **N2-5 / PC-A10** | volume-/pontfelhő-előnézet Qt6-ban | DNEG N2, POINTCLOUD-001 | N1, #1549; PC fázisengedély | részben | **nem** |
| **N3-6′ (X4)** | AX-szerkesztő: szintaxiskiemelés, függvénylista, compile-állapot, előnézet | AX-INTERPRETER-001 (jóváhagyva 11-22) | X1 (#1597) → X2/X3 | terv jóváhagyva, X4 fázisengedély külön | **nem** |
| **EC-4 / VC-4** | effektlánc-mentés/macro az Effect Controlsban, effektböngésző a manifesztből | EFFECTCHAIN-001, VFX-CATALOG-001 | EC-A8, VC-3 | 0.1, kódolási engedély nincs | **nem** |
| **VFX-3** | csomópont-szerkesztő (`VfxGraph`) | VFX-001 | VFX-2 ← render C0/F3 | külön fázisengedély | **nem** |
| **OSA-PROFILES (Qt)** | sorozatprofil-választó, exportbeállítás profilból (#886, #1585, #1586) | nyitott PR-ok | gazda beolvasztása | kész, Ready for review | – (a gazdánál) |

## 4. Ütközések, duplikációk, pontatlanságok

| # | Megállapítás | Súly | Javaslat |
|---|---|---|---|
| Ü-1 | **U1-UI és U2-A4 nem duplikátum, hanem szándékos lépcső** (a 11b szerint a minimális panel U1-ben, a teljes U2-ben). Kockázat: ha az U1-UI nincs jóváhagyva, az U1 funkciója UI nélkül nem érhető el. | K | Az U1-UI-t külön döntéssel jóváhagyni (D-Q2); az U2-A4 szövege hivatkozzon az U1-UI-ra („bővíti, nem cseréli”). |
| Ü-2 | **„A meglévő effekt-szerkesztő panel”** (DNEG audit T3, a N3-GUI sora) félrevezető: a `panels/fx_editor.py` az **audio-effektek** grafikus szerkesztője (EQ, dinamika). Az AX-INTERPRETER-001 jól mondja: **új** panel a `python/cfa4_qt/panels/`-ben. | K | Az audit T3 sorát javítani („új AX-szerkesztő panel”), hogy senki ne az `fx_editor.py`-t bővítse. Javítás: külön docs-PR, ha István engedi (D-Q3). |
| Ü-3 | **Három szerkesztő-felület egy témán:** EC-4 (macro a *meglévő* Effect Controlsban), VC-4 (effektböngésző a manifesztből), VFX-3 (csomópont-szerkesztő), N3-6′ (AX-szerkesztő). A VFX-001 szabálya „egy képesség – egy elsődleges megvalósítás”; az EC-4 és a VFX-3 határa (preset = a `VfxGraph` részgráfja) a K-10-ben már rögzített. | K | Nincs teendő most; a VFX-3 indításakor az EC-4 macro-felületét a `VfxGraph`-ra kell leképezni, nem újraírni. |
| Ü-4 | **Séma-sorrend:** a #1608 a `scene3d`-et **v18**-ra tette (v17 az `objects`-é). A #1479 4. megjegyzése még „16 → 17”-et mond; a tervfájl 1.1-es fejléce már a v18-at. | K | A #1479-ben a gazda-teendőt frissíteni (megtörtént ebben a körben, lásd §7). |
| Ü-5 | **Tervfájl-verzió:** a projekt-könyvtárban `CFA4-OPENUSD-3D-001-1.1.md` van, a repóban a fájlnév `…-1.0.md` marad (a fejléc „1.1”). Zavaró, de dokumentált. | A | Nincs teendő. |
| Ü-6 | **4.14 a FINISH-tervben nem szerepel**, de a Q5-öt blokkolja (minden sor ✅ kell). | M | D-Q1. |
| Ü-7 | **Közös fájl:** `docs/qt-parity.md` (§2.5). Új felületi elemek (U1-UI, N1-10…) sorai csak akkor kerüljenek bele, amikor a kód létezik; ez a terv nem nyúl hozzá. | A | – |

Súly: M magas, K közepes, A alacsony. Nincs olyan ütközés, amely a VISION.md-nek mond ellent; a 3D-mélység (VISION §8) projektfüggő, ez István 2026-11-22-i döntése.

## 5. Mit lehet most a felhőből előrevinni, és mit nem

- **Nem indítható:** minden §3-beli elem (lásd az „Indítható ma?” oszlop). Az U1-UI-hoz **két feltétel** hiányzik: a #1608 láncának beolvadása (a `scene3d.*` parancsok a `main`-ben nincsenek, a panel nélkülük nem tesztelhető), és István döntése a 11b/5-ről.
- **Elvben indítható, de döntés kell:** a **4.14 lezárása**. Az egyetlen hiány: a QC-ablak a jelölt időpontnál megnyitja az exportált fájlt. Ehhez az exportot be kell hozni a Source monitorba (`source.open` projekt-elemet vár), vagyis a fájlt importálni kellene a felhasználó projektjébe, ami **mellékhatás**. Két tisztább út: (a) csak olvasható „QC-előnézet” a QC-ablakban (külön képkocka-lekérés a fájlból, projekt érintetlen), (b) ideiglenes, a projektbe nem mentett Source-elem. Ez tervezési döntés, nem kódolási részlet (D-Q1).
- **Dokumentációs javítás:** Ü-2 (audit T3 szöveg), kis docs-PR, ha engedélyezett.

## 6. Döntési pontok Istvánnak (javaslattal) – D-Q1…D-Q3 ELDÖNTVE 2026-11-23 06:01 (javaslat szerint, mind igen)

**Átvezetés:** D-Q1 → `CFA4-QT6-FINISH-001` 1.4 (Q1b, ebben a PR-ban). D-Q2 → az OpenUSD-terv 11b/5 elfogadva (U1-UI, U1-A6); a fájl a #1552 ágán van, ezért a szövegátvezetés ott, a #1552 tulajdonosáé/gazdáé (jelezve a #1479-ben). D-Q3 → külön docs-PR (audit T3). D-Q4: nem most.

| # | Kérdés | Javaslat | Indok |
|---|---|---|---|
| **D-Q1** | A 4.14 hiányát (jelölt időpontnál lejátszás) felvegyük-e a QT6-FINISH-001 tervbe új Q1b lépésként, és melyik úton (a) vagy (b) legyen? | **Igen, Q1b; az (a) út** (csak olvasható előnézet a QC-ablakban, projekt érintetlen) | A Q5 nem indulhat nélküle; az (a) út nem módosítja a projektet, a „Never crash / ne vesszen el munka” szabályhoz illik. |
| **D-Q2** | Elfogadod-e az OPENUSD-3D-001 §11b/5 javaslatot (minimális `scene3d` asset-panel az U1 része, új U1-A6 kapuval)? | **Igen** | A parancsok UI nélkül a felhasználónak elérhetetlenek; az U2-A4 túl késő. A panel kicsi, csak a parancsokra épül, és csak 3D-projektben jelenik meg (nincs globális felület). |
| **D-Q3** | Javíthatom-e az audit T3 „meglévő effekt-szerkesztő panel” szövegét külön docs-PR-ban (Ü-2)? | **Igen** | A rossz hivatkozás az `fx_editor.py` (audio) bővítéséhez vezethet. |
| **D-Q4** | Kérsz-e a Q3 (GPU-mérés) vagy Q2 (KDE-lista) elvégzéséhez segítséget (kész parancs-csomag a mérésre)? | **Nem most** | A mérés a te gépeden fut; a parancsok és a lista már készen vannak. Kihagyás esetén `deferred-physical-tests.md` sor. |

A jóváhagyás nem implementációs engedély (§2.10): az U1-UI/U2/U3 fázisengedélyek megvannak; a kapuk (a terv `main`-be kerülése, #1608 beolvadása) maradnak.

## 7. Javasolt sorrend

1. **Gazda:** #1550 → #1552 → #1578 → #1608 beolvasztása (terv → olvasók → `scene3d`, séma v18); a #886 → #1585 → #1586 (OSA-PROFILES Qt) külön, független sorban.
2. D-Q1…D-Q3 döntés (István).
3. **Q1b / 4.14** (Qt6, kis PR) – D-Q1 után.
4. **U1-UI** (U1-A6, Qt6 asset-panel) – #1608 beolvadása és D-Q2 után; Qt6-teszt Waylanden és X11-en, automation-id minden elemen.
5. **Q2/Q3/Q4.1** – a tulajdonos gépén, amikor ideje van; eredmény a `qt-desktop-checks.md`-be és a `qt-parity.md` sorokba; ha kihagyja: `deferred-physical-tests.md`.
6. Utána a DNEG-vonal Qt6-elemei a saját kapuik szerint: N1-10 (N1 után), U3a-A8 (U1 után), N3-6′ (X1–X3 után), EC-4/VC-4/VFX-3 (a VFX-2 után).
7. **Q5 (egui törlése)** csak ezek és a tulajdonosi jóváhagyás után.

## 8. Mit nem állít ez a dokumentum

Nem futtattam Qt6-tesztet és nem építettem a GUI-t; az állapotok a `qt-parity.md` sorain és a tervek szövegén alapulnak. A PR-ok állapotát a lekérdezés napján (2026-11-23) olvastam. A `deferred-physical-tests.md` tartalmát nem néztem sort-soronként a Q3 miatt; ha a GPU-mérés ott már szerepel, az a tulajdonos döntése.

## 9. Változásnapló

| Verzió | Dátum | Változás |
|---|---|---|
| 0.1 | 2026-11-23 | Első javaslat: Qt6-feladatok újraelemzése, ütközések, döntési pontok. |
