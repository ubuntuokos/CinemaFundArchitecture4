# CFA4-KDENLIVE-EXECUTION-PLAN-001 — K36 újraelemzés és frissített végrehajtási terv

| Dokumentumadat | Érték |
|---|---|
| Verzió | 1.0 (javaslat) |
| Állapot | **Jóváhagyásra vár** (tulajdonos: ubuntuokos). A jóváhagyás nem indít megvalósítást; minden hullám és K-pont külön tulajdonosi indítási engedélyt igényel (`coordination.md` §2.10) |
| Dátum | 2026-11-22 |
| Alap | CFA4 `main` @ `963b0313` (2026-11-22); `CFA4-AI_modules` @ `e2746cb` (2026-11-16) |
| Módszer | Csak olvasás: a `main` fája és tervei, a nyitott PR-ok címei (a #1559 diffje), a `CFA4-AI_modules` ADR 0008-ja, a 11 kiegészítő tároló és a 7 Kdenlive-automatizálási fork. Kódot nem futtattam, tesztet nem indítottam |
| Alapterv | [`CFA4-KDENLIVE-PLAN-001-FINAL-1.0.md`](CFA4-KDENLIVE-PLAN-001-FINAL-1.0.md) (jóváhagyva 2026-11-03). Ez a dokumentum a **K36** (újraelemzési kapu) és a IV. pont kapujának elvégzése; az alapterv K-számai változatlanok |
| Nyelv | magyar |

## 0. Röviden

1. **A K36 kapu elvégezve** (a tulajdonos elfogadására vár). A K37–K41 és a többi K-pont kiadhatóvá válik, a 6. pont hullámai szerint.
2. **Ami azóta megváltozott és a Kdenlive-tervet érinti:** (a) az MI-réteg ADR-je **ADR 0008** lett (régen 0006), kód még nincs, az F0 kapu (Qt6 kész, egui kint) még nem nyílt; (b) az MLT-olvasó **közös a Shotcut-importtal** (SC-5, tulajdonosi döntés 2026-11-22); (c) a VFX-terv 1.1 (jóváhagyva 2026-11-22) a K20-táblát és a K43-at hivatkozza; (d) a K17 PostgreSQL-re és JSON-ra bomlott (D-8, 2026-11-15); (e) a Qt6 GUI 66 ✅ / 5 🟡 / 1 ❌ (volt 40/19/9); (f) az Event Fabric terv bekerült a repóba; (g) a jobtár, az erőforrás-kezelés és a render-feladatok közös infrastruktúrát kaptak (`cfa4-tasks`, `cfa4-store`, render C0), amelyre a K14, K16, K34, K39 épül.
3. **Kész vagy szűkült K-pontok:** K19 (easing), K54 (history.clear) kész; K10, K11, K15, K18 részben fedett; K17 szűkült. Részletek: 5. pont.
4. **A kiegészítő tárolók közül egy sem változott 2026-10-25 óta**, a `kdenlive` kivételével (261 commit; a `.kdenlive` formátum verziója változatlan, MLT 7.42 az új minimum). Nyolc tároló elavult vagy elhagyott, a működési elvük használható, a kódjuk nem (3. pont).
5. **Az MI-pontok** (K2 kivételével) továbbra is csak az ADR 0008 megfelelő kapuja után indulnak. Az **első indítható hullám MI nélküli**: K1/K42/K20/K25 közös MLT-mag, K22, K10-szűkítés, K11-bővítés, K12, K15-maradék, K37, K38.
6. **Tulajdonosi döntést igénylő pontok:** 7. pont (D-1…D-7), a legfontosabb: a K43 (Lottie) gazdája jelenleg nincs, a K13 (gyroflow-core) licencének tisztázása, és az ADR-számozás ütközése.

## 1. Mi változott a 2026-11-03-i FINAL 1.0 óta (delta a CFA4-ben)

| # | Változás | Forrás (a `main`-ben, ha nincs más) | Hatás a Kdenlive-tervre |
|---|---|---|---|
| Δ1 | **MI-réteg:** az ADR **0008** (`CFA4-AI_modules/docs/adr/ADR-0008-ai-processing-layer.md`; régen 0006), kapuk **AI-F0…AI-F6**, állapot: elfogadva, **kód még nincs**. F0 = a teljes Qt6-átállás kész és az egui kint; az F0-ban megengedett: dokumentáció, minimumteszt-burkoló, worker-protokoll, K2, ACS-1…4. Az F3 az első képesség (angol felirat-időzítés, wav2vec2/ONNX), az F4 a többi ONNX (köztük SAM2 = K4), az F5 candle (Whisper = K2/K31/K32), az F6 PyTorch/LLM | `CFA4-AI_modules` ADR 0008; `docs/plans/CFA4-AI-MODEL-TRAINING-001.md` | Az alapterv minden „ADR 0006” hivatkozása ADR 0008-at jelent; a K-pontok kapui átszámozva az 5. pontban. A fő repó ADR-0006-a a BacklotHub PostgreSQL, az ADR-0008-a a `cfa4-audit`: **számütközés** az ai_modules ADR 0008-cal (D-3) |
| Δ2 | **Közös infrastruktúra:** `cfa4-tasks` (ACS-1…ACS-3 kész; ACS-8 PR nyitva), `cfa4-store` (PG-1…, PG-3 PR nyitva), render C0 (job API, worker-szerződés), `CFA4-EVENT-FABRIC-001` 1.0 a `docs/plans/`-ben | `crates/tasks`, `crates/store`, `docs/plans/CFA4-EVENT-FABRIC-001-1.0.md`, `CFA4-TASKSTORE-PG-001.md` | A K14 (hálózati render), K16 (kötegelt átkódolás), K34 (figyelt mappa) **nem építhet saját sort**: a közös feladatsorra és a render C0 szerződésre épül. A K39 eseményfeliratkozás az Event Fabric illesztési mellékletéhez igazodik (D4/T7 megszűnik) |
| Δ3 | **Közös MLT-mag** (tulajdonosi döntés, 2026-11-22): a K1/K42/K20/K25 és a Shotcut SC-5 **egy** olvasó a `crates/interchange`-ben, dialektusonként külön leképezési tábla; export nincs. A SC-5 terv a #1559-ben van, ugyanott a K1 sora is kiegészül | PR #1559 (`…SC5-MLT-IMPORT.md`), PR #1575 (SC-5.0 jelentés) | A K1 az SC-5.1 magjával **közös PR-sorozat**; a K1 + SC-5 együtt kb. 40–75 h az önálló 66–116 h helyett. A mag a `main`-en még nincs |
| Δ4 | **VFX-terv 1.1** (jóváhagyva 2026-11-22): a K20-táblát (Kdenlive-azonosító → CFA4-effekt) a `G-xx` katalógus `aliases` mezője hivatkozza; a Lottie egyetlen gazdája „a főcímgenerátor” (X-7); a Kdenlive-fork csak olvasott donor, 2026-11-22-i állapoton befagyasztva; a Kdenlive/MLT „külön projekt (K1, K20, K43)” | `docs/plans/CFA4-VFX-001.md` §3, §3.1 | A K20 kimenete katalógus-kompatibilis kell legyen (alias-oszlop). A K43-nak **nincs gazdája**: a `CFA4-TITLE-CREDITS-001` 1.1 nem tartalmaz Lottie-t (D-1). Új effekt nem a K20 feladata: a hiány a VFX-katalógusba kerül |
| Δ5 | **Qt6 GUI:** `qt-parity.md` 66 ✅, 5 🟡, 1 ❌ (2026-11-22; volt 40/19/9). A `CFA4-QT6-FINISH-001` 1.3 a befejezés terve; a Q2 (KDE ellenőrzőlista) PR nyitva | `docs/qt-parity.md`, `docs/plans/CFA4-QT6-FINISH-001.md` | A felületi K-pontok (K7, K8, K49, K51, K53, K58, K60) indíthatók, ha a kapu megengedi; az egui-ba nem készül semmi |
| Δ6 | **K17 szétbontva** (D-8, 2026-11-15): feliratsablonok és presetek atomi JSON-ban a `<data dir>`-ben; az epizód sorozat-hivatkozása a projektfájlban (UUIDv7); a nyilvántartás a PostgreSQL `registry` sémában (P7). SQLite **nincs** (új adatbázismotor tilalma) | `docs/plans/CFA4-DATA-PG-001-1.0.md` §4.6; `CFA4-TASKSTORE-PG-001.md` | A K17 tartalma és előfeltétele megváltozik (5. pont) |
| Δ7 | **Letöltés:** a `cfa4-fetch` (külön, szandboxolt folyamat, 443-as port, SHA-256, folytatható `.part`) és a `CFA4-VIDEO-EXTERNAL-SOURCES-001` 1.0 (jóváhagyva 2026-11-15) a K18 közös letöltőjére épül | `apps/cfa4-fetch`, `crates/fetch-client`, `docs/plans/CFA4-VIDEO-EXTERNAL-SOURCES-001-1.0.md` | A K18 infrastruktúrája nagyrészt megvan; a K18 a modell- és erőforrás-szolgáltatók adatfájlját és a jóváhagyó felületet adja |
| Δ8 | **Csend-eltávolítás és Silero:** a V1-A (energiaalapú, nem MI) az MI-réteg előtt is indulhat (jóváhagyva 2026-11-21); a V1-B a Silero-modellt a felhasználó tölti le (PR #1526) | `docs/plans/CFA4-V1-SILENCE-REMOVAL-001.md` | A K41 „csendvágás” része a V1-A-ra támaszkodik, nem külön fejlesztés |
| Δ9 | **Objektumkövetés/maszk:** az `OBJECT-MASK-PLAN-001` végrehajtása megkezdve (2026-11-20; OM-A5, OM-A5b nyitva), a követés tiszta Rust, az MI-részek (detektálás, szegmentálás) az AI-F4 után | `docs/plans/OBJECT-MASK-PLAN-001-2026-11-03.md` | A **K4 (SAM2)** az OBJECT-MASK szegmentálási lépéseivel közös, ott él; a K4 itt hivatkozás marad |
| Δ10 | **Feliratok:** a Subtitle Studio 1.2 F0–F7; az ASS/SSA (K9) még nincs a `crates/captions`-ben (SRT, VTT, TTML, SCC, MCC, STL, CEA-608 van) | `docs/plans/SUBTITLE-STUDIO-PLAN-001-FINAL-1.2.md` | K9, K27, K28, K29, K30 ott maradnak (alapterv 11.2) |
| Δ11 | **Szabályok:** nincs fizetős rész (2026-11-08); GPL-2.0-only/AGPL tilos kód- és függőségi szinten, az AGPL szabályozása a #1561-ben nyitott; a „port vagy marad az eredeti nyelv” szabály (2026-11-03) | `AGENTS.md`; PR #1561 | A forkok licenc-besorolása (4. pont) ehhez igazodik |
| Δ12 | **Névütközés:** a `LOSSLESS-MEDIA-TOOLS-001` (PR #1124, nyitva) saját K1–K7 jelölést használ | PR #1124 | Ütközik a Kdenlive K-számaival: jelezni kell a PR gazdájának (D-6) |

## 2. Hatókör és szabályok, amelyek ennek a tervnek a végrehajtását kötik

- `VISION.md` §2.1–2.4, §5, §6 (Qt6 az első; MI helyi; gyártófüggetlen gyorsítás; MI-kapuk); `AGENTS.md` (nincs összeomlás, clean-room kodekekre, GPL-3.0-or-later, nincs fizetős rész).
- `coordination.md` §2.8 (projekthatár), §2.10 (fejlesztés csak jóváhagyott terv alapján), §2.11 (terv csak a repóban), §2.6 (30-nál több nyitott PR esetén új munka csak a tulajdonos kérésére; **a lekérdezéskor (2026-11-22) 60 körüli nyitott PR volt**, ezért minden hullám indításához tulajdonosi kérés kell).
- Alapterv 12. pont döntései változatlanok: K1 kötelező és **nincs Kdenlive/MLT-export**; az MLT csak tesztorákulum (E11); a backend-sorrend saját motor → FFmpeg ≥ 9.0.2 → GStreamer a `cfa4-route` alatt (ADR 0005); az x264/x265/LAME a felhasználó csomagja (K62); Qt Widgets idővonal.
- **MI-szabály:** minden MI-funkció csak az ADR 0008 megfelelő kapuja után. A K2 az egyetlen kivétel (ADR 0008 F0). A V1-A nem MI-funkció (jóváhagyva 2026-11-21).
- Az ADR 0003, ADR 0005 és a render-terv már jóváhagyott függőségei nem nyílnak újra itt.

## 3. A kiegészítő tárolók állapota (2026-11-22)

Mind a 11 tárolónak egy ága van, nyitott PR-ja és issue-ja nincs; a kdenlive-n kívül egyiken sem volt commit 2026-10-25 óta.

| Tároló | Utolsó commit | Licenc (ellenőrizve) | Állapot, hatás a tervre |
|---|---|---|---|
| `kdenlive` | 2026-11-22 (27.03.70 dev; stabil: 26.08.3, 2026-11-05) | GPL-3.0-only OR KDE-Accepted-GPL; a `REUSE.toml` CC0 körét lásd lent; új `LICENSES/LGPL-2.1-or-later.txt` (egy titler-fájl) | **Aktív.** 261 commit 10-25 óta (jórészt l10n, timeline-csoport/undo-javítások, titler rich text, relatív médiaútvonalak, projektmappa-refaktor). **`.kdenlive` document version változatlan: 1.1 / patch 1** (`DOCUMENTVERSION`, 2026-11-22). Új property-k: `kdenlive:docproperties.storagetype/storagefolder/lastsavefolder`, relatív `resource`. **MLT minimum 7.42.0** (volt 7.38), Qt ≥ 6.10, OpenTimelineIO kötelező. 10 új frei0r effekt-XML (`3dperspective`, `camerashake`, `flarefx`, `harris0r`, `motiondiff0r`, `shadert0y`, `shake0scillate`, `slitscan0r`, `squigglevision`, `whirlpinch0r`). MI: új funkció nincs (SAM2, Whisper, Seamless, Vosk megvolt; csak CUDA 13.x és crash-javítások). **MCP: nincs.** Hasznos: `src/render/renderserver.*`, `renderer/kdenlive_render.cpp`, headless mód |
| `kdenlive-export-profiles` | 2025-12-26 | **CC0 csak a README-ben, LICENSE fájl nincs** | 72 preset, 9 csomag (web H.264, WebM VP9, AV1, HEVC, Instagram, közösségi, iPhone, VA-API GPU, archív). **K11 előtt a licenc tisztázandó** (D-2) |
| `kdenlive_ease_generator` | 2024-12-24 | GPL-3.0; SPDX nincs; az `easingslist.py` a `semitable/easing-functions`-ból származik, attribúció nélkül | Elavult (a Kdenlive-ben beépített easing van). A K19 már kész a CFA4-ben; kódot nem veszünk át |
| `gyroflow-frei0r` | 2023-10-24 | kód: MIT OR Apache-2.0; a `gyroflow-core` git-függőség (rev `c551b2f`) licencét a bináris örökli | 2023 óta áll; a K13-hoz csak a működési elv és az illesztés hivatkozás. A `gyroflow-core` licence nem ellenőrzött (D-4) |
| `kdenlive-multirender` | 2019-09-24 | MIT | Elhagyott; a darabolás-párhuzamosítás-összefűzés elve a K14-hez |
| `kdenlive-profiles` | 2022-01-10 | GPL-2.0 (szövegméret alapján; „only”/„or-later” nincs kimondva) | Két szöveges YouTube-recept; csak tartalmi minta, **kód/szöveg nem vehető át** (a GPL-2.0-only tilos) |
| `kdenlive-clipper` | 2021-11-05 | MIT | A 21.08-as formátumhoz készült; a klipkivágás-ffmpeg `-c copy` elve a K15/K16-hoz |
| `kdenlive-network-render` | 2021-07-04 | GPL-3.0 | A szerver/kliens felosztás elve a K14-hez; a nyers socket/SSHFS/`input()` megoldásait nem vesszük át (alapterv 11.1) |
| `kdenlive-project-analyzer` | 2016-10-25 | GPL-3.0 (+ Saxon-CE MPL-2.0, Font Awesome SIL OFL/MIT) | Csak a ~15.08 formátumot ismeri; a K23 információs modellje referencia |
| `kdenlive-post-render-script` | 2016-07-21 | GPL-3.0-or-later | Egyszerű, AtomicParsley-alapú metaadat/borító; a K15 felhasználói „render utáni parancs” mintája |
| `kdenlive-tools` | 2014-08-04 | BSD-2-Clause | Elavult (0.9.x); a K22 forráscsere-működés mintája |

**A Kdenlive CC0 köre** (a `REUSE.toml` szerint; 26.08 óta egyetlen sor változott: `kdenlive.entitlements`): effekt-, átmenet- és generátor-XML-ek és átmenet-előnézetek, lumák, layoutok, LUT-ok, resourceprovider-ek, címsablonok, `data/profiles/**`, a `.rc` fájlok (köztük `sammodelsinfo.rc`, `externalproxies.rc`, `camcorderfilters.rc`) és effektlista-txt-k. **Nem CC0:** `data/profiles.xml` (GPL-3.0-only OR KDE-Accepted-GPL), az ikonok (LGPL-3.0-or-later), a `dev-docs` (CC-BY-SA-4.0). A Python-szkriptek és a C++ fejlécek GPL-3.0-only OR KDE-Accepted-GPL: a GPL-3.0 opcióval átvehetők, fájlonként.

**Következtetések:**
- A K1 tesztmintái frissítendők: Kdenlive 26.08.3 és a 26.11-es dev által írt projektek (relatív útvonalak, `storagetype`); a formátumverzió-ugrás nincs, így az olvasó nem veszít a fejlesztéssel.
- A K20 táblának a 10 új frei0r effektet „nem támogatott (frei0r-host hiányzik)” sorként fel kell vennie, amíg a K5 nincs kész.
- Az MLT-minimum emelése (7.42) a `melt` tesztorákulum verzióját érinti: az SC-5 és a K25 CI-ben rögzítendő a `melt` verziója.

## 4. A Kdenlive-forkok (K36 fő vizsgálata)

A vizsgálat a README-ken, a fa- és commitlistákon, a tesztszerkezeten és a tool-listákon alapul; **semmit nem futtattunk**. A két nagy fork (D-Ogi, DanEscher98) csak egy-egy feature-ágon, blobok nélküli klónnal készült; a diffjük a hivatalos Kdenlive-hoz nem készült el.

| Fork | Utolsó commit | Licenc | Mit ad | Átvételi kategória |
|---|---|---|---|---|
| `D-Ogi/kdenlive` | 2026-02-02 (`feature/dbus-api-expansion`) | GPL (+ QuickJS, MIT) | D-Bus API a forkban; `src/expressions/` QuickJS kifejezésmotor; shader-szerkesztő; roto-ecset | **C** referencia (a kifejezésmag gazdája a VFX-4, nem a K-terv) |
| `D-Ogi/kdenlive-api` | 2026-02-04 | MIT csak a README-ben, LICENSE fájl nincs | Python D-Bus kliens, Resolve-kompatibilis hierarchia, ~195 metódus, 81 mock-teszt | **B** minta |
| `D-Ogi/mcp-kdenlive` | 2026-02-04 | **nincs licenc** | 177 MCP-eszköz, 4 prompt, `kdenlive://cookbook`; jogosultság és esemény nincs | **D** a kód, **C** az eszközlista |
| `DanEscher98/kdenlive` (`feature/websocket`) | 2025-12-15 | GPL | `src/rpc/`: JSON-RPC 2.0 WebSocketen, `rpc-schema.json5`, dispatcher, notifier (`rpc.subscribe`), 9 handler, loopback + opcionális token, `PermissionDenied` (−32008) | **C** referencia (séma, események) |
| `IO-AtelierTech/kdenlive-automation` | 2025-12-15 | **MIT** (LICENSE fájllal) | WebSocket-kliens + 91 eszközű MCP, `docs/interface-contract.md` (1048 sor), 15 példaszkript, séma- és e2e-tesztek | **B** (a szerződés és a példák átírhatók; a CFA4 Rust-első, így nem kódátvétel) |
| `0xmeowmeow/vibecut` | 2026-09-03 | GPL-3.0-only OR KDE-Accepted-GPL | C++ MI-panel a Kdenliveben (Anthropic SSE + Ollama), 10 belső eszköz, `DESIGN_SPECS.md` (terv → megerősítés → végrehajtás checkpointokkal) | **C** referencia (munkafolyamat-minta) |
| `hive-agi/hive-kdenlive` | 2026-10-21 (v0.3.3) | **MIT** | Clojure; 5 MCP-eszköz, ~38 route; **fork nélkül** `.hkd.edn` + `melt`; byte-stabil `.kdenlive`/MLT XML olvasás-írás, ~142 teszt | **B** (a `.kdenlive`-olvasás és a fejetlen idővonal-logika Rustra átírható; a K1 egyik forrása) |

**Mit adnak a K37–K41-hez:**
- **K37 (hiányzó eszközök):** vágóműveletek (`ripple_delete`, `ripple_trim`, `roll_edit`, `slide_edit`), `extract_zone`, `render_contact_sheet`, `render_frame`, `detect_scenes`, `import_srt_as_titles`, `set_effect_expression`, `timeline_close_gaps`, `flow_delete_short_clips`, `flow_export_and_wait`, `render_start_with_guides`. A CFA4 parancsregiszterében sok megvan; a K37 a lista és a regiszter **parancsonkénti összevetése** (alább, 6. pont W1).
- **K38 (szintek):** egyik fork sem ad valódi szinteket; minták: loopback + token, `PermissionDenied`, a vibecut plan→confirm kapuja, checkpoint mentés/visszaállítás. A szintek (olvasó/szerkesztő/romboló) tervezése a CFA4-re marad, az MCP-annotációkra építve.
- **K39 (események):** `render.progress`, `render.finished`, `project.modified`, `timeline.changed`, projekt megnyitva/zárva/mentve; az Event Fabric illesztési mellékletéhez igazítva.
- **K40 (Python-minták):** `06_trim_silence`, `09_scene_splitter`, `13_audio_ducking_setup`, `14_marker_based_export`, `build_timeline.py`.
- **K41 (MI-vágás):** a vibecut `DESIGN_SPECS.md` és a mcp-kdenlive promptjai csak munkafolyamat-referencia; az MI az E1 terven át szerkeszt.

**Licenc-szabály a forkokra:** kód csak GPL-3.0-kompatibilis, ellenőrzött fejlécű forrásból; a licenc nélküli `mcp-kdenlive` és a LICENSE nélküli `kdenlive-api` kódja **nem használható**, csak a nyilvános eszköznevek mint ötlet. A forkok a VFX-terv szerint is csak olvasott donorok.

## 5. K-pontonkénti delta (K1–K62)

Jelmagyarázat: **KÉSZ** (a `main`-ben, bizonyítékkal), **RÉSZBEN** (átfed vagy részben kész, a kapunál ellenőrizendő), **SZŰKÜL / ÁTKERÜL** (más terv gazdája), **VÁR** (kapura), **INDÍTHATÓ** (a kapu megengedi, külön engedéllyel). Az „ellenőrizendő” azt jelenti: ebben a körben csak kódkeresés történt, a pont végrehajtása előtt a IV. pont kapuja újra lefut.

| K | Állapot 2026-11-22 | Változás az alaptervhez képest | Hullám |
|---|---|---|---|
| **K1** | INDÍTHATÓ | Közös MLT-mag az SC-5-tel (Δ3); a `main`-en nincs `.kdenlive`-olvasó, csak az OTIO-út (`tests/kdenlive_otio.rs`). Mintaprojektek: 26.08.3 és 26.11 dev. Doc version 1.1 változatlan | W1 |
| **K2** | INDÍTHATÓ (AI-F0 kivétel) | A `whisper` feature még ki van kapcsolva a kiadásokban (G19). Az ADR 0008 F5-höz kötött a gazdagabb modellkezelés | W2 |
| **K3** | RÉSZBEN | Q2 (KDE ellenőrzőlista, PR nyitva); Qt6 66/5/1 | a QT6-FINISH-001 szerint |
| **K4** | ÁTKERÜL | Az OBJECT-MASK-PLAN szegmentálási lépései; AI-F4 után | OBJECT-MASK |
| **K5** | VÁR | frei0r + LV2 külön folyamatban (ADR 0003 A6–A7), nincs kezdett munka; 10 új Kdenlive-frei0r effekt a K20-ban „nem támogatott” sor | ADR 0003 |
| **K6, K28** | VÁR | Subtitle Studio F5 + AI-F4/F5; a CFA4 nem szállít modellt | Subtitle Studio |
| **K7** | INDÍTHATÓ | Közös Qt6-spinbox; a számmező-kifejezés a VFX-4 kifejezésmaggal **nem** közös (más cél: számolás, nem animáció) | W4 |
| **K8** | INDÍTHATÓ | Spektrogram-panel nincs (a `speech/mel.rs` ASR-hez van); Qt6 | W4 |
| **K9** | VÁR | Subtitle Studio F2 (ASS/SSA még nincs a `crates/captions`-ben) | Subtitle Studio |
| **K10** | RÉSZBEN | Van `cfa4-cli serve` (a Qt-motor külön folyamat) és `cfa4-cli render` (**csak egy képkocka PNG-ben**, nem export). A K10 szűkül: export-munka külön folyamatban, JSON-soros előrehaladással, megszakítás félkész fájl nélkül; a render C0 worker-szerződésével egyeztetve | W1 |
| **K11** | RÉSZBEN | Vannak YouTube/WebM VP9/H.264-presetek (`crates/export/src/presets.rs`) és Social Deliverables (`social.rs`). Hiányzik a `kdenlive-export-profiles` 72 presetjének összevetése (AV1, HEVC, iPhone, VA-API, archív). **Licenc előbb** (D-2) | W1 |
| **K12** | INDÍTHATÓ | Kameraproxy-felismerés (GoPro LRV, DJI LRF…) nincs; a `MediaPool` proxy-csatolása megvan. Forrás: `externalproxies.rc`, `camcorderfilters.rc` (CC0) | W1 |
| **K13** | VÁR | Nincs gyroflow-integráció; a Warp Stabilizer javult (G18). **Licenc-tisztázás előbb** (D-4) | W3 |
| **K14** | VÁR | Épül: render C0, `cfa4-tasks`, Smart Render; Event Fabric. Saját sor nincs | render-terv |
| **K15** | RÉSZBEN | Fejezetek az MP4/MOV/HLS/MKV exportban megvannak (CHAPTERS-DASH). Maradék: borítókép, YouTube-fejezetszöveg, felhasználói render utáni parancs | W2 |
| **K16** | VÁR | A közös feladatsorra (`cfa4-tasks`) épül | W3 |
| **K17** | SZŰKÜL | D-8: JSON (sablon, preset), projektfájl-UUID, PG `registry` séma; SQLite nincs; előfeltétel a PG-3 | W3 |
| **K18** | RÉSZBEN | `cfa4-fetch` megvan; a szolgáltatói adatfájl, a jóváhagyó és modellfelület hiányzik; közös a VIDEO-EXTERNAL-SOURCES F-fázisaival | W3 |
| **K19** | **KÉSZ** | A `Interpolation` már tartalmazza a Sine, Expo, Circ, Overshoot, Elastic, Bounce… presetet (`crates/project/src/keyframe.rs`). A K19 lezárható; a hiányzó Penner-változatok listázása a kapunál | — |
| **K20** | INDÍTHATÓ | A K1 része; kimenet: azonosító → CFA4-effekt tábla, `aliases`-kompatibilis a VFX-katalógussal (Δ4) | W1 |
| **K21** | INDÍTHATÓ | CC0 lumák, LUT-ok, előnézetek; `.attribution` + `cargo xtask assets` | W2 |
| **K22** | INDÍTHATÓ | A K1-gyel; az eredeti `.kdenlive` megőrzése (SC-5 D6 mintájára) | W1 |
| **K23** | INDÍTHATÓ | `cfa4-qc` szabálya; a modell a projektből, nem XML-ből | W2 |
| **K24** | VÁR | Licenc-ellenőrzés, P3 | W5 |
| **K25** | INDÍTHATÓ | A K1 része; a `melt` verzió rögzítése (MLT ≥ 7.42-es a Kdenlive 26.11-hez) | W1 |
| **K26, K27, K29, K30** | VÁR | Subtitle Studio / ADR 0008 (alapterv 11.2). A K29 (szabályalapú QC) nem MI | Subtitle Studio |
| **K31–K35** | VÁR | AI-F1/F5 és a K18; a K33 (WER-mérés) a K2 után. A V1-B Silero-modellt a felhasználó tölti le | W2–W5 |
| **K36** | **KÉSZ** (ez a dokumentum) | A tulajdonos elfogadására vár | — |
| **K37** | INDÍTHATÓ | Parancsonkénti összevetés a 4. pont listájával; új parancs csak a regiszterbe | W1 |
| **K38** | VÁR | V3 (Unix socket 0600, mozdulat alatti visszatartás) a `main`-en **nincs**; a vezérlőcsatorna TCP + token (`control-protocol.md`). A K38 szintjei az MCP-annotációkra épülnek | W2 (V3 után) |
| **K39** | VÁR | V3 + Event Fabric; K37 után | W3 |
| **K40** | INDÍTHATÓ | Python-minták összevetése a `python/` klienseivel | W2 |
| **K41** | VÁR | E1 (`plan.prepare`/`plan.commit`) a `main`-en **nincs**; AI-F3; a csendvágás a V1-A-ra épül | W5 |
| **K42** | INDÍTHATÓ | A K1 része (leképezési tábla a közös magban) | W1 |
| **K43** | **GAZDA NÉLKÜL** | A VFX X-7 szerint a főcímgenerátor, de a TITLE-CREDITS 1.1-ben nincs Lottie | D-1 |
| **K44** | VÁR | ADR 0005 2. lépés után; a saját effekt az első | W5 |
| **K45–K48** | INDÍTHATÓ | Motoros, kicsi; K48 az `edit.editOriginal`-ra épül | W2 |
| **K49–K53, K55–K59** | INDÍTHATÓ (felület: Qt6) | Egyenként kicsi; a Qt6-állapot miatt a felületi részük a QT6-FINISH-001 szerint | W4 |
| **K54** | **KÉSZ** | `history.clear` és `history.deleteRedoable` megvan (`crates/engine/src/commands.rs`) | — |
| **K56** | VÁR | Saját effekt; a VFX-katalógusba regisztrálandó | W5 |
| **K60** | VÁR | Többmonitoros munka; K3-mal együtt tesztelve | W4 |
| **K61** | INDÍTHATÓ | Az export-sorban megvan a szünet egyes elemei (`serve.rs`); ellenőrizendő | W2 |
| **K62** | RÉSZBEN | A CI-orákulum már x264/x265 nélkül fut (PR #1125); a futásidejű felismerés és az inaktív állapot útmutatója a K62 | W2 |

## 6. Végrehajtási terv (hullámok)

Minden hullám **külön tulajdonosi indítási engedélyt** igényel, és mindegyik K-pont előtt a IV. pont kapuja lefut (friss `main`, nyitott PR-ok, függőség-ellenőrzés). Egy PR egy K-pont (vagy lépése), kész PR „Ready for review”, „Gazda: …” issue. A becslések ügynökórák, nem mértek.

| Hullám | Tartalom | Előfeltétel | Becslés (ó) | Kilépési bizonyíték |
|---|---|---|---|---|
| **W0** | Ez a terv elfogadása; D-1…D-7 eldöntése; az alapterv frissítésének átvezetése | tulajdonosi jóváhagyás | – | jóváhagyás, a döntések az alaptervben |
| **W1 — MI nélküli mag** | **K1 + K42 + K20 + K25 + K22 közös MLT-maggal (SC-5.1-gyel közös PR-sorozat)**; **K10 szűkített** (export külön folyamatban); **K11** (a CC0-tisztázás után); **K12**; **K37** (parancs-összevetés) | W0; D-2; a #1559 beolvasztása; render C0 szerződés a K10-hez | K1-család 40–75 (SC-5-tel együtt), K10 10–20, K11 10–20, K12 8–15, K37 10–20 | a K1 „Kész, ha”: mintaprojektek megnyílnak, vágópontok képkocka-pontosak, kompatibilitási jelentés, ellenséges bemenet nem pánik; `melt`-összevetés CI-ben |
| **W2 — kis motoros és AI-F0 kivétel** | **K2** (Whisper a kiadásokban, G19); **K15** maradéka; **K21, K23**; **K40**; **K45–K48**, **K61**, **K62**; **K38** (V3 után) | W1 K1 a K23-hoz; V3 a K38-hoz | K2 5–10, K15 6–10, K21 6–10, K23 12–20, K40 6–12, K45–K48 12–24, K61 6–12, K62 10–20, K38 10–20 | pontonként a K-pont „Kész, ha” sora |
| **W3 — közös infrastruktúrára épülők** | **K14** (render C0 + `cfa4-tasks`), **K16**, **K17** (D-8), **K18**, **K39**, **K13** | render F3/C0; PG-3; ACS-8; Event Fabric; D-4 | K14 70–120, K16 25–40, K17 20–35, K18 15–30, K39 15–30, K13 35–55 | két gépen azonos kimenet (K14); PG nélkül JSON-fallback (K17); hash-ellenőrzött letöltés (K18) |
| **W4 — Qt6-felület és motor** | **K7, K8, K49, K51, K53, K58, K60, K50, K52, K55, K57, K59** | a QT6-FINISH-001 kapui | 70–130 | Qt-teszt Wayland/X11 (K60), automatizálási azonosítók |
| **W5 — MI-kapuhoz kötött** | **K31–K35, K26–K28, K41, K44, K56, K24** (és a K4, K6, K9 a saját tervükben) | AI-F1…F5; E1; K18 | a K-pontok saját becslése | az ADR 0008 kapuk kilépési feltételei |

**Sorrend és indoklás:** W1 a legnagyobb értékű és MI nélküli (a `.kdenlive` megnyitása a kötelező döntés, K1), és a Shotcut-import ugyanazt a magot használja. A W2 a kis, kockázatmentes pontokat és a K2-t hozza. A W3 nem indulhat a közös infrastruktúra (render C0, PG, ACS, Event Fabric) nélkül, hogy ne készüljön párhuzamos sor. A W5 az MI-rétegé.

**Összes becslés:** W1–W4 kb. 400–730 ó (az alaptervben szereplő számokból, a K1 közös magos megtakarításával és a K17/K19/K54 szűkítésével); a W5 az ADR 0008 kapuit követi. Az alapterv 610–1035 órás becslése így nem nő.

## 7. Tulajdonosi döntést igénylő pontok

| # | Kérdés | Javaslatom |
|---|---|---|
| **D-1** | **K43 (Lottie/Glaxnimate) gazdája.** A VFX-terv X-7 a főcímgenerátort (TITLE-CREDITS-001) nevezi, de annak 1.1 változata nem tartalmaz Lottie-t | A TITLE-CREDITS-001 következő változata vegye fel a Lottie-t (VFX-7 hiánylista), vagy a K43 maradjon külön tételként a VFX-projekt után; addig ne induljon |
| **D-2** | **A `kdenlive-export-profiles` licence:** CC0 csak a README-ben áll, LICENSE fájl nincs; a tároló nem a Kdenlive hivatalos adata | A K11 a Kdenlive hivatalos CC0 `data/profiles/**` fájljaiból és saját presetekből dolgozzon; a külső tárolót csak a szerző írásos CC0-megerősítése (vagy LICENSE-fájl) után használjuk |
| **D-3** | **ADR-számütközés:** az AI-réteg ADR-je az ai_modules-ban 0008, a fő repóban a 0008 a `cfa4-audit`, a 0006 a BacklotHub PG | Az AI-réteg ADR-je kapjon fő-repós számot a (T3/T4) átemeléskor; az alaptervben „ADR 0008 (ai_modules)” jelölést használunk, amíg ez el nem dől |
| **D-4** | **`gyroflow-core` licence** (K13): a licencet a `gyroflow-core` tárolójából nem ellenőriztük; ha GPL-3.0-**only**, a CFA4 „or-later” kikötésével szemben a VFX-terv Lumit-döntésének mintájára kell kezelni | A K13 indítása előtt külön licenc-ellenőrzés; a Lumit-minta (fájlszintű „only” megjegyzés) csak a tulajdonos külön döntésével |
| **D-5** | **A forkok és a kiegészítő tárolók a „mit integrálunk” listában** (tulajdonosi szabály, 2026-10-27) | Ez a terv a forrásokat felveszi (II. pont a frissített alaptervben); a `VISION.md` §4 Kdenlive-sorának bővítése a tulajdonos dolga |
| **D-6** | **K-számütközés:** a `LOSSLESS-MEDIA-TOOLS-001` (PR #1124) is K1–K7-et használ | A #1124 gazdája nevezze át (pl. `LM-K…`); a Kdenlive K-számok a régebbiek |
| **D-7** | **Indítás:** a lekérdezéskor 60 körüli nyitott PR volt (`coordination.md` §2.6: 30 fölött csak a tulajdonos kérésére) | A W1 indítását a tulajdonos kérje; javasolt első PR: a K1/SC-5.1 közös mag |

## 8. Mit nem ellenőriztem

1. A K-pontok „kész/részben” besorolása kódkeresésen alapul; a 11.7 B lista (K45–K62) parancsazonosítóit nem soroltam végig a regiszterben, ezek a kapunál ellenőrizendők.
2. A nyitott PR-ok tartalmát (a #1559 kivételével) nem néztem át; csak a címüket.
3. A `CFA4-AI_modules` ADR 0008 szövegét és a docs/ai fájlokat olvastam; a CFA4-sdk repót és a CFA4-EVENT-FABRIC-001 teljes szövegét nem.
4. A forkoknál nem futtattam kódot; a két nagy fork diffje a hivatalos Kdenlive-hoz nem készült el; a D-Ogi D-Bus rétegének pontos helyét nem azonosítottuk.
5. A `gyroflow-core`, a `semitable/easing-functions` és a kiegészítő tárolók GPL-szövegeinek „only/or-later” kikötését nem ellenőriztem.
6. A `plan/` mappát (gép-lokális) nem láttam; a becslések nem mértek.

## 9. Változásnapló

| Dátum | Változás |
|---|---|
| 2026-11-22 | 1.0 (javaslat): K36 elvégezve; delta, tárolók és forkok állapota, K-pontonkénti állapot, hullámok, nyitott döntések |
