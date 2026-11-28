# CFA4-KDENLIVE-EXECUTION-PLAN-001 — KDN-36 újraelemzés és frissített végrehajtási terv

| Dokumentumadat | Érték |
|---|---|
| Verzió | 1.2 (a 2026-11-27-i duplikáció-átvezetés után; a fájlnév változatlan) |
| Állapot | **Jóváhagyva** (István, 2026-11-22 22:44, „a tervet jóváhagyom, véglegesítsd és kezd el a fejlesztést”). A fejlesztés a 6. pont sorrendjében indul; a D-2 (KDN-11) és a D-4 (KDN-13) pontig az érintett KDN-pont áll |
| Átvezetés | **2026-11-27 (István jóváhagyta, 22:04–22:06):** a 2026-11-23-i duplikáció-ellenőrzés eredménye és a `main` @ `9f38bfea` szerinti frissítés (W0). A W1-hez a kiegészítő tárolók és a forkok 2026-11-22-i befagyasztott állapota érvényes; W2–W5 előtt újra kell elemezni őket |
| Dátum | 2026-11-22 |
| Alap | CFA4 `main` @ `963b0313` (2026-11-22); `CFA4-AI_modules` @ `e2746cb` (2026-11-16) |
| Módszer | Csak olvasás: a `main` fája és tervei, a nyitott PR-ok címei (a #1559 diffje), a `CFA4-AI_modules` ADR 0008-ja, a 11 kiegészítő tároló és a 7 Kdenlive-automatizálási fork. Kódot nem futtattam, tesztet nem indítottam |
| Alapterv | [`CFA4-KDENLIVE-PLAN-001-FINAL-1.0.md`](CFA4-KDENLIVE-PLAN-001-FINAL-1.0.md) (jóváhagyva 2026-11-03). Ez a dokumentum a **KDN-36** (újraelemzési kapu) és a IV. pont kapujának elvégzése; az alapterv pontszámai változatlanok, jelölésük 2026-11-23-tól KDN-<n> (régen K<n>; lásd az Azonosítók sort) |
| Azonosítók | **2026-11-23 (István kérésére):** a Kdenlive-pontok jele `K<n>` helyett `KDN-<n>` (n = 1…62, a szám változatlan), mert a korábban készült `LOSSLESS-MEDIA-TOOLS-001` (PR #1124) a K1–K16 jelölést használja, és azt nem módosítjuk. A `K<n>` jelölés ettől kezdve nem Kdenlive-pont |
| Nyelv | magyar |

## 0. Röviden

1. **A KDN-36 kapu elvégezve** (a tulajdonos elfogadására vár). Az 5B pontjai a 6. pont hullámai szerint adhatók ki.
2. **Ami azóta megváltozott és a Kdenlive-tervet érinti:** (a) az MI-réteg ADR-je **ADR 0008** lett (régen 0006), kód még nincs, az F0 kapu (Qt6 kész, egui kint) még nem nyílt; (b) az MLT-olvasó **közös a Shotcut-importtal** (SC-5, tulajdonosi döntés 2026-11-22); (c) a VFX-terv 1.1 (jóváhagyva 2026-11-22) a KDN-20-táblát és a KDN-43-at hivatkozza; (d) a KDN-17 PostgreSQL-re és JSON-ra bomlott (D-8, 2026-11-15); (e) a Qt6 GUI 66 ✅ / 5 🟡 / 1 ❌ (volt 40/19/9); (f) az Event Fabric terv bekerült a repóba; (g) a jobtár, az erőforrás-kezelés és a render-feladatok közös infrastruktúrát kaptak (`cfa4-tasks`, `cfa4-store`, render C0), amelyre a KDN-14, KDN-16, KDN-34, KDN-39 épül.
3. **Szűrés (István, 2026-11-22; frissítve 2026-11-27):** csak az marad, ami még nincs a CFA4-ben és más tervben sem. 62 KDN-pontból **37 kikerül** (kész, vagy más terv hordozza; a KDN-42 az SC-5 része, a KDN-36 ez a dokumentum: 5A), **25 marad** (5B), ebből tizenegy szűkítve (KDN-11, KDN-15, KDN-20, KDN-23, KDN-25, KDN-38, KDN-57, KDN-59, KDN-8, KDN-60 és a KDN-56, amely a VFX-katalógusba kerül, ezért az 5B táblájában nincs külön sora: a táblában 24 pont áll). Részletek: 5. pont.
4. **A kiegészítő tárolók közül egy sem változott 2026-10-25 óta**, a `kdenlive` kivételével (261 commit; a `.kdenlive` formátum verziója változatlan, MLT 7.42 az új minimum). Nyolc tároló elavult vagy elhagyott, a működési elvük használható, a kódjuk nem (3. pont).
5. **Az MI-pontok** (KDN-2 kivételével) továbbra is csak az ADR 0008 megfelelő kapuja után indulnak; a legtöbb ilyen pont más terv hordozza, ezért kikerült. Az **első indítható hullám MI nélküli**: a `.kdenlive`-dialektus az SC-5 közös magján (a mag, a KDN-20-tábla, a jelölők és csoportok már a `main`-ben vannak; hátra: az effekt- és átmenet-leképezés bekötése, KDN-25), a KDN-11-bővítés.
6. **Tulajdonosi döntést igénylő pontok:** 7. pont (D-1…D-10), a legfontosabb: a KDN-43 (Lottie) gazdája jelenleg nincs, a KDN-13 (gyroflow-core) licencének tisztázása, és az ADR-számozás ütközése.

## 1. Mi változott a 2026-11-03-i FINAL 1.0 óta (delta a CFA4-ben)

| # | Változás | Forrás (a `main`-ben, ha nincs más) | Hatás a Kdenlive-tervre |
|---|---|---|---|
| Δ1 | **MI-réteg:** az ADR **0008** (`CFA4-AI_modules/docs/adr/ADR-0008-ai-processing-layer.md`; régen 0006), kapuk **AI-F0…AI-F6**, állapot: elfogadva, **kód még nincs**. F0 = a teljes Qt6-átállás kész és az egui kint; az F0-ban megengedett: dokumentáció, minimumteszt-burkoló, worker-protokoll, KDN-2, ACS-1…4. Az F3 az első képesség (angol felirat-időzítés, wav2vec2/ONNX), az F4 a többi ONNX (köztük SAM2 = KDN-4), az F5 candle (Whisper = KDN-2/KDN-31/KDN-32), az F6 PyTorch/LLM | `CFA4-AI_modules` ADR 0008; `docs/plans/CFA4-AI-MODEL-TRAINING-001.md` | Az alapterv minden „ADR 0006” hivatkozása ADR 0008-at jelent; a KDN-pontok kapui átszámozva az 5. pontban. A fő repó ADR-0006-a a BacklotHub PostgreSQL, az ADR-0008-a a `cfa4-audit`: **számütközés** az ai_modules ADR 0008-cal (D-3) |
| Δ2 | **Közös infrastruktúra:** `cfa4-tasks` (ACS-1…ACS-3 kész; ACS-8 PR nyitva), `cfa4-store` (PG-1…, PG-3 PR nyitva), render C0 (job API, worker-szerződés), `CFA4-EVENT-FABRIC-001` 1.0 a `docs/plans/`-ben | `crates/tasks`, `crates/store`, `docs/plans/CFA4-EVENT-FABRIC-001-1.0.md`, `CFA4-TASKSTORE-PG-001.md` | A KDN-14 (hálózati render), KDN-16 (kötegelt átkódolás), KDN-34 (figyelt mappa) **nem építhet saját sort**: a közös feladatsorra és a render C0 szerződésre épül. A KDN-39 eseményfeliratkozás az Event Fabric illesztési mellékletéhez igazodik (D4/T7 megszűnik) |
| Δ3 | **Közös MLT-mag** (tulajdonosi döntés, 2026-11-22): a KDN-1/KDN-42/KDN-20/KDN-25 és a Shotcut SC-5 **egy** olvasó a `crates/interchange`-ben, dialektusonként külön leképezési tábla; export nincs. A SC-5 terv a #1559-ben van, ugyanott a KDN-1 sora is kiegészül | PR #1559 (`…SC5-MLT-IMPORT.md`), PR #1575 (SC-5.0 jelentés) | A KDN-1 az SC-5.1 magjával **közös PR-sorozat**; a KDN-1 + SC-5 együtt kb. 40–75 h az önálló 66–116 h helyett. A mag a `main`-en még nincs |
| Δ4 | **VFX-terv 1.1** (jóváhagyva 2026-11-22): a KDN-20-táblát (Kdenlive-azonosító → CFA4-effekt) a `G-xx` katalógus `aliases` mezője hivatkozza; a Lottie egyetlen gazdája „a főcímgenerátor” (X-7); a Kdenlive-fork csak olvasott donor, 2026-11-22-i állapoton befagyasztva; a Kdenlive/MLT „külön projekt (KDN-1, KDN-20, KDN-43)” | `docs/plans/CFA4-VFX-001.md` §3, §3.1 | A KDN-20 kimenete katalógus-kompatibilis kell legyen (alias-oszlop). A KDN-43-nak **nincs gazdája**: a `CFA4-TITLE-CREDITS-001` 1.1 nem tartalmaz Lottie-t (D-1). Új effekt nem a KDN-20 feladata: a hiány a VFX-katalógusba kerül |
| Δ5 | **Qt6 GUI:** `qt-parity.md` 66 ✅, 5 🟡, 1 ❌ (2026-11-22; volt 40/19/9). A `CFA4-QT6-FINISH-001` 1.3 a befejezés terve; a Q2 (KDE ellenőrzőlista) PR nyitva | `docs/qt-parity.md`, `docs/plans/CFA4-QT6-FINISH-001.md` | A felületi KDN-pontok (KDN-7, KDN-8, KDN-49, KDN-51, KDN-53, KDN-58, KDN-60) indíthatók, ha a kapu megengedi; az egui-ba nem készül semmi |
| Δ6 | **KDN-17 szétbontva** (D-8, 2026-11-15): feliratsablonok és presetek atomi JSON-ban a `<data dir>`-ben; az epizód sorozat-hivatkozása a projektfájlban (UUIDv7); a nyilvántartás a PostgreSQL `registry` sémában (P7). SQLite **nincs** (új adatbázismotor tilalma) | `docs/plans/CFA4-DATA-PG-001-1.0.md` §4.6; `CFA4-TASKSTORE-PG-001.md` | A KDN-17 tartalma és előfeltétele megváltozik (5. pont) |
| Δ7 | **Letöltés:** a `cfa4-fetch` (külön, szandboxolt folyamat, 443-as port, SHA-256, folytatható `.part`) és a `CFA4-VIDEO-EXTERNAL-SOURCES-001` 1.0 (jóváhagyva 2026-11-15) a KDN-18 közös letöltőjére épül | `apps/cfa4-fetch`, `crates/fetch-client`, `docs/plans/CFA4-VIDEO-EXTERNAL-SOURCES-001-1.0.md` | A KDN-18 infrastruktúrája nagyrészt megvan; a KDN-18 a modell- és erőforrás-szolgáltatók adatfájlját és a jóváhagyó felületet adja |
| Δ8 | **Csend-eltávolítás és Silero:** a V1-A (energiaalapú, nem MI) az MI-réteg előtt is indulhat (jóváhagyva 2026-11-21); a V1-B a Silero-modellt a felhasználó tölti le (PR #1526) | `docs/plans/CFA4-V1-SILENCE-REMOVAL-001.md` | A KDN-41 „csendvágás” része a V1-A-ra támaszkodik, nem külön fejlesztés |
| Δ9 | **Objektumkövetés/maszk:** az `OBJECT-MASK-PLAN-001` végrehajtása megkezdve (2026-11-20; OM-A5, OM-A5b nyitva), a követés tiszta Rust, az MI-részek (detektálás, szegmentálás) az AI-F4 után | `docs/plans/OBJECT-MASK-PLAN-001-2026-11-03.md` | A **KDN-4 (SAM2)** az OBJECT-MASK szegmentálási lépéseivel közös, ott él; a KDN-4 itt hivatkozás marad |
| Δ10 | **Feliratok:** a Subtitle Studio 1.2 F0–F7; az ASS/SSA (KDN-9) még nincs a `crates/captions`-ben (SRT, VTT, TTML, SCC, MCC, STL, CEA-608 van) | `docs/plans/SUBTITLE-STUDIO-PLAN-001-FINAL-1.2.md` | KDN-9, KDN-27, KDN-28, KDN-29, KDN-30 ott maradnak (alapterv 11.2) |
| Δ11 | **Szabályok:** nincs fizetős rész (2026-11-08); GPL-2.0-only/AGPL tilos kód- és függőségi szinten, az AGPL szabályozása a #1561-ben nyitott; a „port vagy marad az eredeti nyelv” szabály (2026-11-03) | `AGENTS.md`; PR #1561 | A forkok licenc-besorolása (4. pont) ehhez igazodik |
| Δ12 | **Névütközés (rendezve, 2026-11-23):** a `LOSSLESS-MEDIA-TOOLS-001` (PR #1124, nyitva) a K1–K16 jelölést használja | PR #1124 | A Kdenlive-pontok jele `KDN-<n>` lett; a #1124 terve változatlan (D-6) |

## 2. Hatókör és szabályok, amelyek ennek a tervnek a végrehajtását kötik

- `VISION.md` §2.1–2.4, §5, §6 (Qt6 az első; MI helyi; gyártófüggetlen gyorsítás; MI-kapuk); `AGENTS.md` (nincs összeomlás, clean-room kodekekre, GPL-3.0-or-later, nincs fizetős rész).
- `coordination.md` §2.8 (projekthatár), §2.10 (fejlesztés csak jóváhagyott terv alapján), §2.11 (terv csak a repóban), §2.6 (30-nál több nyitott PR esetén új munka csak a tulajdonos kérésére; **a lekérdezéskor (2026-11-22) 60 körüli nyitott PR volt**, ezért minden hullám indításához tulajdonosi kérés kell).
- **Import kötelező, export nem feltétel** (István, 2026-11-22; megerősíti a 2026-10-25-i döntést): a `.kdenlive` projektfájlnak importálhatónak kell lennie (KDN-1, W1 első eleme); Kdenlive/MLT-exportra nincs szükség, és nem kerül a tervbe.
- Alapterv 12. pont döntései változatlanok: KDN-1 kötelező és **nincs Kdenlive/MLT-export**; az MLT csak tesztorákulum (E11); a backend-sorrend saját motor → FFmpeg ≥ 9.0.2 → GStreamer a `cfa4-route` alatt (ADR 0005); az x264/x265/LAME a felhasználó csomagja (KDN-62); Qt Widgets idővonal.
- **MI-szabály:** minden MI-funkció csak az ADR 0008 megfelelő kapuja után. A KDN-2 az egyetlen kivétel (ADR 0008 F0). A V1-A nem MI-funkció (jóváhagyva 2026-11-21).
- Az ADR 0003, ADR 0005 és a render-terv már jóváhagyott függőségei nem nyílnak újra itt.

## 3. A kiegészítő tárolók állapota (2026-11-22)

Mind a 11 tárolónak egy ága van, nyitott PR-ja és issue-ja nincs; a kdenlive-n kívül egyiken sem volt commit 2026-10-25 óta.

| Tároló | Utolsó commit | Licenc (ellenőrizve) | Állapot, hatás a tervre |
|---|---|---|---|
| `kdenlive` | 2026-11-22 (27.03.70 dev; stabil: 26.08.3, 2026-11-05) | GPL-3.0-only OR KDE-Accepted-GPL; a `REUSE.toml` CC0 körét lásd lent; új `LICENSES/LGPL-2.1-or-later.txt` (egy titler-fájl) | **Aktív.** 261 commit 10-25 óta (jórészt l10n, timeline-csoport/undo-javítások, titler rich text, relatív médiaútvonalak, projektmappa-refaktor). **`.kdenlive` document version változatlan: 1.1 / patch 1** (`DOCUMENTVERSION`, 2026-11-22). Új property-k: `kdenlive:docproperties.storagetype/storagefolder/lastsavefolder`, relatív `resource`. **MLT minimum 7.42.0** (volt 7.38), Qt ≥ 6.10, OpenTimelineIO kötelező. 10 új frei0r effekt-XML (`3dperspective`, `camerashake`, `flarefx`, `harris0r`, `motiondiff0r`, `shadert0y`, `shake0scillate`, `slitscan0r`, `squigglevision`, `whirlpinch0r`). MI: új funkció nincs (SAM2, Whisper, Seamless, Vosk megvolt; csak CUDA 13.x és crash-javítások). **MCP: nincs.** Hasznos: `src/render/renderserver.*`, `renderer/kdenlive_render.cpp`, headless mód |
| `kdenlive-export-profiles` | 2025-12-26 | **CC0 csak a README-ben, LICENSE fájl nincs** | 72 preset, 9 csomag (web H.264, WebM VP9, AV1, HEVC, Instagram, közösségi, iPhone, VA-API GPU, archív). **KDN-11 előtt a licenc tisztázandó** (D-2) |
| `kdenlive_ease_generator` | 2024-12-24 | GPL-3.0; SPDX nincs; az `easingslist.py` a `semitable/easing-functions`-ból származik, attribúció nélkül | Elavult (a Kdenlive-ben beépített easing van). A KDN-19 már kész a CFA4-ben; kódot nem veszünk át |
| `gyroflow-frei0r` | 2023-10-24 | kód: MIT OR Apache-2.0; a `gyroflow-core` git-függőség (rev `c551b2f`) licencét a bináris örökli | 2023 óta áll; a KDN-13-hoz csak a működési elv és az illesztés hivatkozás. A `gyroflow-core` licence nem ellenőrzött (D-4) |
| `kdenlive-multirender` | 2019-09-24 | MIT | Elhagyott; a darabolás-párhuzamosítás-összefűzés elve a KDN-14-hez |
| `kdenlive-profiles` | 2022-01-10 | GPL-2.0 (szövegméret alapján; „only”/„or-later” nincs kimondva) | Két szöveges YouTube-recept; csak tartalmi minta, **kód/szöveg nem vehető át** (a GPL-2.0-only tilos) |
| `kdenlive-clipper` | 2021-11-05 | MIT | A 21.08-as formátumhoz készült; a klipkivágás-ffmpeg `-c copy` elve a KDN-15/KDN-16-hoz |
| `kdenlive-network-render` | 2021-07-04 | GPL-3.0 | A szerver/kliens felosztás elve a KDN-14-hez; a nyers socket/SSHFS/`input()` megoldásait nem vesszük át (alapterv 11.1) |
| `kdenlive-project-analyzer` | 2016-10-25 | GPL-3.0 (+ Saxon-CE MPL-2.0, Font Awesome SIL OFL/MIT) | Csak a ~15.08 formátumot ismeri; a KDN-23 információs modellje referencia |
| `kdenlive-post-render-script` | 2016-07-21 | GPL-3.0-or-later | Egyszerű, AtomicParsley-alapú metaadat/borító; a KDN-15 felhasználói „render utáni parancs” mintája |
| `kdenlive-tools` | 2014-08-04 | BSD-2-Clause | Elavult (0.9.x); a KDN-22 forráscsere-működés mintája |

**A Kdenlive CC0 köre** (a `REUSE.toml` szerint; 26.08 óta egyetlen sor változott: `kdenlive.entitlements`): effekt-, átmenet- és generátor-XML-ek és átmenet-előnézetek, lumák, layoutok, LUT-ok, resourceprovider-ek, címsablonok, `data/profiles/**`, a `.rc` fájlok (köztük `sammodelsinfo.rc`, `externalproxies.rc`, `camcorderfilters.rc`) és effektlista-txt-k. **Nem CC0:** `data/profiles.xml` (GPL-3.0-only OR KDE-Accepted-GPL), az ikonok (LGPL-3.0-or-later), a `dev-docs` (CC-BY-SA-4.0). A Python-szkriptek és a C++ fejlécek GPL-3.0-only OR KDE-Accepted-GPL: a GPL-3.0 opcióval átvehetők, fájlonként.

**Következtetések:**
- A KDN-1 tesztmintái frissítendők: Kdenlive 26.08.3 és a 26.11-es dev által írt projektek (relatív útvonalak, `storagetype`); a formátumverzió-ugrás nincs, így az olvasó nem veszít a fejlesztéssel.
- A KDN-20 táblának a 10 új frei0r effektet „nem támogatott (frei0r-host hiányzik)” sorként fel kell vennie, amíg a KDN-5 nincs kész.
- Az MLT-minimum emelése (7.42) a `melt` tesztorákulum verzióját érinti: az SC-5 és a KDN-25 CI-ben rögzítendő a `melt` verziója.

## 4. A Kdenlive-forkok (KDN-36 fő vizsgálata)

A vizsgálat a README-ken, a fa- és commitlistákon, a tesztszerkezeten és a tool-listákon alapul; **semmit nem futtattunk**. A két nagy fork (D-Ogi, DanEscher98) csak egy-egy feature-ágon, blobok nélküli klónnal készült; a diffjük a hivatalos Kdenlive-hoz nem készült el.

| Fork | Utolsó commit | Licenc | Mit ad | Átvételi kategória |
|---|---|---|---|---|
| `D-Ogi/kdenlive` | 2026-02-02 (`feature/dbus-api-expansion`) | GPL (+ QuickJS, MIT) | D-Bus API a forkban; `src/expressions/` QuickJS kifejezésmotor; shader-szerkesztő; roto-ecset | **C** referencia (a kifejezésmag gazdája a VFX-4, nem a KDN-terv) |
| `D-Ogi/kdenlive-api` | 2026-02-04 | MIT csak a README-ben, LICENSE fájl nincs | Python D-Bus kliens, Resolve-kompatibilis hierarchia, ~195 metódus, 81 mock-teszt | **B** minta |
| `D-Ogi/mcp-kdenlive` | 2026-02-04 | **nincs licenc** | 177 MCP-eszköz, 4 prompt, `kdenlive://cookbook`; jogosultság és esemény nincs | **D** a kód, **C** az eszközlista |
| `DanEscher98/kdenlive` (`feature/websocket`) | 2025-12-15 | GPL | `src/rpc/`: JSON-RPC 2.0 WebSocketen, `rpc-schema.json5`, dispatcher, notifier (`rpc.subscribe`), 9 handler, loopback + opcionális token, `PermissionDenied` (−32008) | **C** referencia (séma, események) |
| `IO-AtelierTech/kdenlive-automation` | 2025-12-15 | **MIT** (LICENSE fájllal) | WebSocket-kliens + 91 eszközű MCP, `docs/interface-contract.md` (1048 sor), 15 példaszkript, séma- és e2e-tesztek | **B** (a szerződés és a példák átírhatók; a CFA4 Rust-első, így nem kódátvétel) |
| `0xmeowmeow/vibecut` | 2026-09-03 | GPL-3.0-only OR KDE-Accepted-GPL | C++ MI-panel a Kdenliveben (Anthropic SSE + Ollama), 10 belső eszköz, `DESIGN_SPECS.md` (terv → megerősítés → végrehajtás checkpointokkal) | **C** referencia (munkafolyamat-minta) |
| `hive-agi/hive-kdenlive` | 2026-10-21 (v0.3.3) | **MIT** | Clojure; 5 MCP-eszköz, ~38 route; **fork nélkül** `.hkd.edn` + `melt`; byte-stabil `.kdenlive`/MLT XML olvasás-írás, ~142 teszt | **B** (a `.kdenlive`-olvasás és a fejetlen idővonal-logika Rustra átírható; a KDN-1 egyik forrása) |

**Mit adnak a KDN-37–KDN-41-hez** (2026-11-27: a KDN-37 hiányzó eszközei elkészültek, lásd 5A):
- **KDN-37 (hiányzó eszközök):** vágóműveletek (`ripple_delete`, `ripple_trim`, `roll_edit`, `slide_edit`), `extract_zone`, `render_contact_sheet`, `render_frame`, `detect_scenes`, `import_srt_as_titles`, `set_effect_expression`, `timeline_close_gaps`, `flow_delete_short_clips`, `flow_export_and_wait`, `render_start_with_guides`. A CFA4 parancsregiszterében sok megvan; a KDN-37 a lista és a regiszter **parancsonkénti összevetése** (alább, 6. pont W1).
- **KDN-38 (szintek):** egyik fork sem ad valódi szinteket; minták: loopback + token, `PermissionDenied`, a vibecut plan→confirm kapuja, checkpoint mentés/visszaállítás. A szintek (olvasó/szerkesztő/romboló) tervezése a CFA4-re marad, az MCP-annotációkra építve.
- **KDN-39 (események):** `render.progress`, `render.finished`, `project.modified`, `timeline.changed`, projekt megnyitva/zárva/mentve; az Event Fabric illesztési mellékletéhez igazítva.
- **KDN-40 (Python-minták):** `06_trim_silence`, `09_scene_splitter`, `13_audio_ducking_setup`, `14_marker_based_export`, `build_timeline.py`.
- **KDN-41 (MI-vágás):** a vibecut `DESIGN_SPECS.md` és a mcp-kdenlive promptjai csak munkafolyamat-referencia; az MI az E1 terven át szerkeszt.

**Licenc-szabály a forkokra:** kód csak GPL-3.0-kompatibilis, ellenőrzött fejlécű forrásból; a licenc nélküli `mcp-kdenlive` és a LICENSE nélküli `kdenlive-api` kódja **nem használható**, csak a nyilvános eszköznevek mint ötlet. A forkok a VFX-terv szerint is csak olvasott donorok.

## 5. Szűrés: csak ami még nincs a CFA4-ben és egyik folyamatban lévő tervében sem

**Szabály (István, 2026-11-22):** a Kdenlive-tervben csak az a képesség marad, amely a `main`-ben nincs meg, és amelyet más jóváhagyott vagy nyitott terv sem hordoz. Ami kész, vagy más terv gazdája, az innen kikerül, itt legfeljebb hivatkozás marad. A szűrés kódkeresésen és a tervek szövegén alapul; ahol a bizonyíték gyenge, a pont „kapunál ellenőrizendő” jelölést kap (a IV. pont kapuja úgyis lefut).

### 5A. Kikerül: kész, vagy más terv hordozza

| K | Miért kerül ki | Hol él |
|---|---|---|
| **KDN-7** | Kész: a Qt6 számmező számolást fogad (`python/cfa4_qt/calc.py`, `hot_number.py`; az egui `calc.rs` portja) | `main` |
| **KDN-19** | Kész: Sine, Expo, Circ, Overshoot, Elastic, Bounce… az `Interpolation`-ben (`crates/project/src/keyframe.rs`) | `main` |
| **KDN-54** | Kész: `history.clear`, `history.deleteRedoable` | `main` |
| **KDN-22** | A Link Media / `media.autoRelink` és a médiaazonosság megvan; a forráscsere és az ujjlenyomat a FILE-SERVICES-001-ben; az eredeti fájl megőrzése az SC-5 D6-ban. A Kdenlive-eredeti megőrzése a KDN-1 része | `main`; `CFA4-FILE-SERVICES-001`; SC-5 |
| **KDN-17** | D-8: JSON + projektfájl-UUID + PG `registry` séma | `CFA4-DATA-PG-001` §4.6 |
| **KDN-2, KDN-31, KDN-32, KDN-33** | Whisper-kiadás (G19), modellkezelő, GPU-futtatás, WER: az ADR 0008 kapui és a Subtitle Studio AC-08 | ADR 0008 (F0/F1/F5), `gaps.md` G19, Subtitle Studio |
| **KDN-3** | Wayland/X11, Qt6-befejezés | `CFA4-QT6-FINISH-001` (Q2 PR nyitva) |
| **KDN-4** | SAM2-maszk, objektumszegmentálás | `OBJECT-MASK-PLAN-001` (OM-A5…, AI-F4 után) |
| **KDN-5** | frei0r és LV2 külön folyamatban | ADR 0003 A6–A7 |
| **KDN-6, KDN-9, KDN-26–KDN-30** | fordítás, ASS/SSA, hangleválasztás, igazítás, felirat-QC, SBV | Subtitle Studio 1.2 (F2, F5), ADR 0008 M10/M15 |
| **KDN-35** | élő felirat | `CFA4-LIVE-STUDIO-001` (ST2110 melléklet említi) |
| **KDN-39** | eseményfeliratkozás | `CFA4-EVENT-FABRIC-001` 1.0 |
| **KDN-40** | Python-kliens és minták | `open-editors-parity.md` E11 |
| **KDN-41** | MI-vágási munkafolyamatok (csendvágás, durva vágás) | V1-A/V1-B, E1, E3, E5 és az ADR 0008 kapuk |
| **KDN-43** | Lottie | VFX X-7 / `CFA4-TITLE-CREDITS-001`; **figyelem:** a TITLE-CREDITS 1.1 szövege nem említi, ezt a terv gazdájának kell pótolnia (D-1) |
| **KDN-44** | FFmpeg-szűrők a routeren át | `CFA4-FFMPEG-BACKEND-R1` (filter-interfész), ADR 0005 |
| **KDN-62** | x264/x265/LAME felismerése, inaktív állapot | `CFA4-FFMPEG-BACKEND-R1` (`cfa4-restricted-extras` modell), ADR 0005 |
| **KDN-36** | Ez a dokumentum | – |
| **KDN-10** | Export külön folyamatban: atomi kimenet, megszakítás, SIGKILL-teszt, job API; a render C0 hordozza (beolvadt: `render.status/list/cancel`, 20 SIGKILL-ciklus) | render C0/F0V |
| **KDN-12** | Kész: `crates/engine/src/camera_proxies.rs`, `media.findCameraProxies` | `main` |
| **KDN-18** | A letöltő (`cfa4-fetch`) és a szolgáltatóleíró adatfájl az F3-é; a modellnyilvántartás az `ai.model` | `CFA4-VIDEO-EXTERNAL-SOURCES-001` F3, `CFA4-DATA-PG-001` D-7 |
| **KDN-37** | Kész: `edit.deleteShortClips` (#1761), `export.queue.addFromMarkers` (#1765), `captions.importAsTitles` (#1773), `qc.contactSheet`; az `effects.setExpression` a VFX-4/X-sor | `main` (2026-11-24); VFX-4 |
| **KDN-47** | Kész: `export.queue.add import:true` a Project panelbe teszi a kész fájlt | `main` (2026-11-24) |
| **KDN-48** | Kész: `media.refreshChanged`, `media_watch.rs`, `edit.editOriginal` | `main` |
| **KDN-52** | Kész: `clip.speedDuration` `clips:[id]` paraméter, alapérték a kijelölés | `main` |
| **KDN-55** | Kész (SC-4): `effects.toggleEnabled` `clips`/`all`/`enabled`; legfeljebb egy `scope:"sequence"` paraméter, nem külön pont | `main` |
| **KDN-61** | `Paused` állapot a `cfa4-tasks` sorállapotgépében; az `ExportQueue` szüneteltetése a VVC FR-6 | `CFA4-VVC-PLAN-001` FR-6, render C0 job API |

### 5B. Marad: új képesség, amelyet semmi más nem hordoz

| K | Képesség | Állapot / megjegyzés | Hullám |
|---|---|---|---|
| **KDN-1** (Kdenlive-dialektus) + **KDN-20** + **KDN-25** | `.kdenlive` olvasása. **Állapot (2026-11-27):** a közös MLT-mag és a Kdenlive-import a `main`-ben (profil, sávok, klipek, rések, sebesség, színklipek, jelölők, A/V-csoportok, Volume-effekt; #1747 beolvadt). A KDN-20-tábla kész (`crates/interchange/data/kdenlive-effect-map.csv`, `…KDN20-EFFECT-MAP-001-1.0.md`). **Hátra:** az effekt- és átmenet-leképezés bekötése az importerbe (ma „not imported yet” figyelmeztetés), az SC-5.3 szűrőtáblájával egyeztetve; KDN-20 = csak a Kdenlive-azonosító-sorok (alias a VFX-katalógushoz); KDN-25 = csak a Kdenlive-mintaprojektek hozzáadása az SC-5 CI-hez, a `melt` verziójának rögzítésével (MLT ≥ 7.42) | részben kész | W1 |
| **KDN-11** (szűkítve) | A hiányzó platform-presetek szoftveresen: HEVC, AV1, iPhone, archív (adat, kb. 3–6 ó). A VA-API/GPU-presetek a G5-ben és a `cfa4-route`-ban. **D-2 előbb** (licenc) | részben | W1 |
| **KDN-15** (szűkítve) | Borítókép a kimeneti fájlba, YouTube-fejezetszöveg, felhasználói render utáni parancs (a fejezet és a `markers.exportCsv` megvan; a fejezetszöveg-rész kapunál ellenőrizendő) | részben | W2 |
| **KDN-21** | CC0 lumák, LUT-ok, átmenet-előnézetek eszközként (`.attribution`) | nincs | W2 |
| **KDN-23** (szűkítve) | Egy jelentés-parancs a meglévőkre (`project.inspect`, `sequence.checkDelivery`, `qc.run`, relink-jelentés, `tasks.check`): hiányzó forrás/proxy/effekt, integritás; új elemzőmotor nem készül; az SC-F0 terv is erre hivatkozik | nincs | W2 |
| **KDN-38** (szűkítve) | A jogosultsági szintek (olvasó/szerkesztő/romboló) **kikényszerítése**; a vezérlőcsatorna már tokenes, az MCP `readOnlyHint`/`destructiveHint` megvan. **Közös `destructive` mező** a GESTURE-001 V-19/MG0 döntéssel: egyet kell kérni, nem kettőt; a **V3** előfeltétele a `main`-en még nincs | nincs | W2 (V3 után) |
| **KDN-45, KDN-46** | zip/tar projektarchívum (a `project_manager` csak collect/consolidate); távoli erőforrás figyelmeztetés (KDN-46: #1881 nyitva; az SC-F0 és a VIDEO-EXTERNAL-SOURCES is említi) | KDN-45 nincs; KDN-46 folyamatban | W2 |
| **KDN-13** | Giroszkópos stabilizálás `gyroflow-core`-ral; **D-4 előbb** (licenc) | nincs (G18 a 2-D/3-D stabilizátort fedi, a giroszkópot nem) | W3 |
| **KDN-14** | Párhuzamos és hálózati render, render-only kliens; a render C0 worker-szerződésére és a `cfa4-tasks`-ra épül | semmi sem fedi (a render-terv helyi) | W3 |
| **KDN-16** | Kötegelt átkódolás a közös feladatsoron; egyeztetni a `LOSSLESS-MEDIA-TOOLS-001`-gyel (#1124) és a VISION konvertálás-területtel | részben a VISION-ban, terv nincs | W3 |
| **KDN-8** (szűkítve) | Hangspektrum- és spektrogram-panel: **egy közös komponens** a Subtitle Studio 1.2 §4-gyel, kétszer nem készül (a `speech/mel.rs` ASR-hez van) | nincs | W4 |
| **KDN-49, KDN-50, KDN-51, KDN-53, KDN-57, KDN-58, KDN-59, KDN-60** | sávátrendezés; marker a résekbe; áttűnés-előnézet; rögzített lejátszófej és középső gombos görgetés; igazítóvonalak a címszerkesztőben (KDN-57 szűkítve: a `cornerRadius` és a snap megvan); szóló Shift+kattintással; hanghullám beágyazott szekvencián (KDN-59: Qt6-port, QT6-FINISH hatókör); többmonitoros munka (KDN-60 szűkítve: a Program/Source monitor teljes képernyőn másik kijelzőn; a lebegő ablak a #466/QT6-FINISH, az SDI a DECKLINK). Az **KDN-56** (Delogo) a VFX-katalógusba kerül (`G-xx` bejegyzés), nem külön pont | kapunál ellenőrizendő pontonként (a Qt6-oldali „nincs” állítás keresésen alapul) | W4 |
| **KDN-24** | Diavetítés-automatizálás (Ken Burns, ütemhez igazítás); licencellenőrzés előbb | nincs | W5 |
| **KDN-34** | Figyelt mappa: bedobott médiából átirat és felirat automatikusan (KDN-16 feladatsor, AI-F5) | nincs; MI-kapu | W5 |

## 6. Végrehajtási terv (hullámok, csak az 5B pontjai)

Minden hullám **külön tulajdonosi indítási engedélyt** igényel, és mindegyik KDN-pont előtt a IV. pont kapuja lefut (friss `main`, nyitott PR-ok, függőség-ellenőrzés). Egy PR egy KDN-pont (vagy lépése), kész PR „Ready for review”, „Gazda: …” issue. A becslések ügynökórák, nem mértek; a KDN-pontok alaptervi becsléseiből, a szűkítésekkel.

| Hullám | Tartalom | Előfeltétel | Becslés (ó) | Kilépési bizonyíték |
|---|---|---|---|---|
| **W0** | Terv elfogadása; D-1…D-7 (D-8…D-10 a 2026-11-27-i átvezetéssel jött); az alapterv átvezetése (az 5A pontjainak törlése, hivatkozásra cserélve) | tulajdonosi jóváhagyás | – | **Elvégezve 2026-11-27** (István jóváhagyta; a végrehajtási terv 1.2; az alapterv átvezetése ugyanebben a PR-ban) |
| **W1 — `.kdenlive` és MI nélküli mag** | KDN-1 (hátralévő rész: effekt- és átmenet-leképezés), KDN-20 (bekötés), KDN-25 az SC-5 közös magjára; KDN-11 (szűkítve) | D-2 a KDN-11-hez; a mag és a KDN-20-tábla kész | KDN-1+KDN-20+KDN-25 25–45 felső korlát (a mag és a tábla kész, újrabecslendő a kapunál), KDN-11 3–6 | a KDN-1 „Kész, ha”: mintaprojektek megnyílnak, vágópontok képkocka-pontosak, kompatibilitási jelentés, ellenséges bemenet nem pánik; `melt`-összevetés CI-ben |
| **W2 — kicsi, motoros** | KDN-15 (szűkítve), KDN-21, KDN-23 (szűkítve), KDN-45, KDN-46 (#1881), KDN-38 (szűkítve, V3 után) | W1 KDN-1 a KDN-23-hoz; V3 a KDN-38-hoz; a W2 előtt a forkok és kiegészítő tárolók újraelemzése | KDN-15 6–10, KDN-21 6–10, KDN-23 12–20, KDN-45 és KDN-46 6–12, KDN-38 10–20 | pontonként a „Kész, ha” sor |
| **W3 — közös infrastruktúrára épülők** | KDN-14, KDN-16, KDN-13 | render F3/C0; ACS-8/`cfa4-tasks`; D-4; a KDN-16 előtt egyeztetés a `LOSSLESS-MEDIA-TOOLS-001`-gyel (#1124) | KDN-14 70–120, KDN-16 25–40, KDN-13 35–55 | két gépen azonos kimenet (KDN-14) |
| **W4 — Qt6-felület és motor** | KDN-8, KDN-49, KDN-50, KDN-51, KDN-53, KDN-57–KDN-60 | a QT6-FINISH-001 kapui | 50–100 | Qt-teszt Wayland/X11 (KDN-60), automatizálási azonosítók |
| **W5 — licenc- vagy MI-kapuhoz kötött** | KDN-24, KDN-34 | licenc-ellenőrzés; AI-F5; KDN-16 | 23–37 | az ADR 0008 kapuk kilépési feltételei |

**Összes becslés:** a 2026-11-22-i 330–590 óra a 2026-11-27-i átvezetéssel kb. 40–75 órával kisebb (KDN-10, KDN-12, KDN-18, KDN-48, KDN-52, KDN-55, KDN-61 és a KDN-11-szűkítés), ehhez jön a KDN-47 és a KDN-37 maradéka (becslés: 5–10 ó). A KDN-1 és KDN-20 haladását nem vontam le, az a W1 újrabecslésénél derül ki. A W1 a legnagyobb értékű és MI nélküli: a `.kdenlive` megnyitása a kötelező döntés (KDN-1), és az SC-5 ugyanazt a magot használja.

## 7. Tulajdonosi döntést igénylő pontok

| # | Kérdés | Javaslatom |
|---|---|---|
| **D-1** | **KDN-43 (Lottie/Glaxnimate) gazdája.** A VFX-terv X-7 a főcímgenerátort (TITLE-CREDITS-001) nevezi, de annak 1.1 változata nem tartalmaz Lottie-t | A TITLE-CREDITS-001 következő változata vegye fel a Lottie-t (VFX-7 hiánylista), vagy a KDN-43 maradjon külön tételként a VFX-projekt után; addig ne induljon |
| **D-2** | **A `kdenlive-export-profiles` licence:** CC0 csak a README-ben áll, LICENSE fájl nincs; a tároló nem a Kdenlive hivatalos adata | A KDN-11 a Kdenlive hivatalos CC0 `data/profiles/**` fájljaiból és saját presetekből dolgozzon; a külső tárolót csak a szerző írásos CC0-megerősítése (vagy LICENSE-fájl) után használjuk |
| **D-3** | **ADR-számütközés:** az AI-réteg ADR-je az ai_modules-ban 0008, a fő repóban a 0008 a `cfa4-audit`, a 0006 a BacklotHub PG | Az AI-réteg ADR-je kapjon fő-repós számot a (T3/T4) átemeléskor; az alaptervben „ADR 0008 (ai_modules)” jelölést használunk, amíg ez el nem dől |
| **D-4** | **`gyroflow-core` licence** (KDN-13): a licencet a `gyroflow-core` tárolójából nem ellenőriztük; ha GPL-3.0-**only**, a CFA4 „or-later” kikötésével szemben a VFX-terv Lumit-döntésének mintájára kell kezelni | A KDN-13 indítása előtt külön licenc-ellenőrzés; a Lumit-minta (fájlszintű „only” megjegyzés) csak a tulajdonos külön döntésével |
| **D-5** | **A forkok és a kiegészítő tárolók a „mit integrálunk” listában** (tulajdonosi szabály, 2026-10-27) | Ez a terv a forrásokat felveszi (II. pont a frissített alaptervben); a `VISION.md` §4 Kdenlive-sorának bővítése a tulajdonos dolga |
| **D-6** | **Pontjelölés-ütközés: ELDÖNTVE (István, 2026-11-23).** A `LOSSLESS-MEDIA-TOOLS-001` (PR #1124) K1–K16 jelölést használ, és előbb készült | A Kdenlive-pontok jele `KDN-<n>` (régen `K<n>`), a #1124 terve nem változik |
| **D-7** | **Indítás:** 2026-11-27-én 56 nyitott PR van (`coordination.md` §2.6: 30 fölött csak a tulajdonos kérésére) | A W1 további indítását a tulajdonos kérje; a következő logikus lépés a KDN-20-tábla bekötése az importerbe |
| **D-8** | **A kiegészítő tárolók és a forkok újraelemzése** (2026-11-27, **eldöntve a W1-re:** a 2026-11-22-i befagyasztott állapot érvényes) | W2–W5 előtt új elemzés; a `CFA4-AI_modules` ADR 0008 és `docs/ai/` szövege is ellenőrizendő |
| **D-9** | **KDN-38 és GESTURE V-19/MG0:** a `CommandSpec`-nek nincs destruktív jelölője | Egy közös mező, egyeztetve a GESTURE-001 gazdájával |
| **D-10** | **KDN-16 és #1124:** a LOSSLESS-MEDIA-TOOLS-001 nyitott, a kötegelt átkódolás átfedhet vele | Egyeztetés az #1124 gazdájával a KDN-16 indítása előtt |

## 8. Mit nem ellenőriztem

1. A KDN-pontok „kész/részben” besorolása kódkeresésen alapul; a 11.7 B lista (KDN-45–KDN-62) parancsazonosítóit nem soroltam végig a regiszterben, ezek a kapunál ellenőrizendők.
2. A nyitott PR-ok tartalmát (a #1559 kivételével) nem néztem át; csak a címüket.
3. A `CFA4-AI_modules` ADR 0008 szövegét és a docs/ai fájlokat olvastam; a CFA4-sdk repót és a CFA4-EVENT-FABRIC-001 teljes szövegét nem.
4. A forkoknál nem futtattam kódot; a két nagy fork diffje a hivatalos Kdenlive-hoz nem készült el; a D-Ogi D-Bus rétegének pontos helyét nem azonosítottuk.
5. A `gyroflow-core`, a `semitable/easing-functions` és a kiegészítő tárolók GPL-szövegeinek „only/or-later” kikötését nem ellenőriztem.
6. A `plan/` mappát (gép-lokális) nem láttam; a becslések nem mértek.
7. (2026-11-27) A kiegészítő tárolók és a forkok mostani állapotát, valamint a `CFA4-AI_modules` tartalmát nem elemeztem újra (D-8). A „kész/nincs” besorolás a commitnaplón és kulcsszavas kódkeresésen alapul; a Qt6-oldali pontoknál (KDN-49, KDN-51, KDN-53, KDN-58, KDN-59) gyengébb a bizonyíték, kapunál ellenőrizendő. A nyitott PR-ok közül csak a címeket néztem.

## 9. Változásnapló

| Dátum | Változás |
|---|---|
| 2026-11-27 | 1.2 (jóváhagyva, István 22:04–22:06): a 2026-11-23-i duplikáció-ellenőrzés átvezetése (W0) a `main` @ `9f38bfea` állapotával. 5A: +9 pont (KDN-10, 12, 18, 37, 47, 48, 52, 55, 61); 5B: tizenegy pont szűkítve/áthelyezve, a KDN-1 és KDN-20 állapota frissítve; hullámok és becslés újraszámolva; D-7 frissítve, D-8…D-10 új. A pontszámok és a fájlnév változatlanok |
| 2026-11-23 | Jelölés: a Kdenlive-pontok `K<n>` jele `KDN-<n>` lett (ütközés a `LOSSLESS-MEDIA-TOOLS-001` K1–K16 jelölésével, D-6 eldöntve). Tartalmi változás nincs |
| 2026-11-22 | 1.1 (jóváhagyva 22:44):  István kérésére az 5. pont szűrve: csak a CFA4-ben és más tervben nem szereplő képességek maradnak (5A kikerül, 5B marad); a hullámok ennek megfelelően újraszámolva |
| 2026-11-22 | 1.0 (javaslat): KDN-36 elvégezve; delta, tárolók és forkok állapota, KDN-pontonkénti állapot, hullámok, nyitott döntések |
