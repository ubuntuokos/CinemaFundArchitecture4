# CFA4-KDENLIVE-EXECUTION-PLAN-001 — K36 újraelemzés és frissített végrehajtási terv

| Dokumentumadat | Érték |
|---|---|
| Verzió | 1.1 (javaslat; a szűrés után) |
| Állapot | **Jóváhagyásra vár** (tulajdonos: ubuntuokos). A jóváhagyás nem indít megvalósítást; minden hullám és K-pont külön tulajdonosi indítási engedélyt igényel (`coordination.md` §2.10) |
| Dátum | 2026-11-22 |
| Alap | CFA4 `main` @ `963b0313` (2026-11-22); `CFA4-AI_modules` @ `e2746cb` (2026-11-16) |
| Módszer | Csak olvasás: a `main` fája és tervei, a nyitott PR-ok címei (a #1559 diffje), a `CFA4-AI_modules` ADR 0008-ja, a 11 kiegészítő tároló és a 7 Kdenlive-automatizálási fork. Kódot nem futtattam, tesztet nem indítottam |
| Alapterv | [`CFA4-KDENLIVE-PLAN-001-FINAL-1.0.md`](CFA4-KDENLIVE-PLAN-001-FINAL-1.0.md) (jóváhagyva 2026-11-03). Ez a dokumentum a **K36** (újraelemzési kapu) és a IV. pont kapujának elvégzése; az alapterv K-számai változatlanok |
| Nyelv | magyar |

## 0. Röviden

1. **A K36 kapu elvégezve** (a tulajdonos elfogadására vár). Az 5B pontjai a 6. pont hullámai szerint adhatók ki.
2. **Ami azóta megváltozott és a Kdenlive-tervet érinti:** (a) az MI-réteg ADR-je **ADR 0008** lett (régen 0006), kód még nincs, az F0 kapu (Qt6 kész, egui kint) még nem nyílt; (b) az MLT-olvasó **közös a Shotcut-importtal** (SC-5, tulajdonosi döntés 2026-11-22); (c) a VFX-terv 1.1 (jóváhagyva 2026-11-22) a K20-táblát és a K43-at hivatkozza; (d) a K17 PostgreSQL-re és JSON-ra bomlott (D-8, 2026-11-15); (e) a Qt6 GUI 66 ✅ / 5 🟡 / 1 ❌ (volt 40/19/9); (f) az Event Fabric terv bekerült a repóba; (g) a jobtár, az erőforrás-kezelés és a render-feladatok közös infrastruktúrát kaptak (`cfa4-tasks`, `cfa4-store`, render C0), amelyre a K14, K16, K34, K39 épül.
3. **Szűrés (István, 2026-11-22): csak az marad, ami még nincs a CFA4-ben és más tervben sem.** 62 K-pontból 28 kikerül (kész, vagy más terv hordozza; a K42 az SC-5 része, a K36 ez a dokumentum: 5A), 34 marad (5B), ebből öt szűkítve (K10, K11, K15, K18, K37). Részletek: 5. pont.
4. **A kiegészítő tárolók közül egy sem változott 2026-10-25 óta**, a `kdenlive` kivételével (261 commit; a `.kdenlive` formátum verziója változatlan, MLT 7.42 az új minimum). Nyolc tároló elavult vagy elhagyott, a működési elvük használható, a kódjuk nem (3. pont).
5. **Az MI-pontok** (K2 kivételével) továbbra is csak az ADR 0008 megfelelő kapuja után indulnak; a legtöbb ilyen pont más terv hordozza, ezért kikerült. Az **első indítható hullám MI nélküli**: a `.kdenlive`-dialektus az SC-5 közös magján, K10-szűkítés, K11-bővítés, K12, K37-szűkítés.
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
- **Import kötelező, export nem feltétel** (István, 2026-11-22; megerősíti a 2026-10-25-i döntést): a `.kdenlive` projektfájlnak importálhatónak kell lennie (K1, W1 első eleme); Kdenlive/MLT-exportra nincs szükség, és nem kerül a tervbe.
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

## 5. Szűrés: csak ami még nincs a CFA4-ben és egyik folyamatban lévő tervében sem

**Szabály (István, 2026-11-22):** a Kdenlive-tervben csak az a képesség marad, amely a `main`-ben nincs meg, és amelyet más jóváhagyott vagy nyitott terv sem hordoz. Ami kész, vagy más terv gazdája, az innen kikerül, itt legfeljebb hivatkozás marad. A szűrés kódkeresésen és a tervek szövegén alapul; ahol a bizonyíték gyenge, a pont „kapunál ellenőrizendő” jelölést kap (a IV. pont kapuja úgyis lefut).

### 5A. Kikerül: kész, vagy más terv hordozza

| K | Miért kerül ki | Hol él |
|---|---|---|
| **K7** | Kész: a Qt6 számmező számolást fogad (`python/cfa4_qt/calc.py`, `hot_number.py`; az egui `calc.rs` portja) | `main` |
| **K19** | Kész: Sine, Expo, Circ, Overshoot, Elastic, Bounce… az `Interpolation`-ben (`crates/project/src/keyframe.rs`) | `main` |
| **K54** | Kész: `history.clear`, `history.deleteRedoable` | `main` |
| **K22** | A Link Media / `media.autoRelink` és a médiaazonosság megvan; a forráscsere és az ujjlenyomat a FILE-SERVICES-001-ben; az eredeti fájl megőrzése az SC-5 D6-ban. A Kdenlive-eredeti megőrzése a K1 része | `main`; `CFA4-FILE-SERVICES-001`; SC-5 |
| **K17** | D-8: JSON + projektfájl-UUID + PG `registry` séma | `CFA4-DATA-PG-001` §4.6 |
| **K2, K31, K32, K33** | Whisper-kiadás (G19), modellkezelő, GPU-futtatás, WER: az ADR 0008 kapui és a Subtitle Studio AC-08 | ADR 0008 (F0/F1/F5), `gaps.md` G19, Subtitle Studio |
| **K3** | Wayland/X11, Qt6-befejezés | `CFA4-QT6-FINISH-001` (Q2 PR nyitva) |
| **K4** | SAM2-maszk, objektumszegmentálás | `OBJECT-MASK-PLAN-001` (OM-A5…, AI-F4 után) |
| **K5** | frei0r és LV2 külön folyamatban | ADR 0003 A6–A7 |
| **K6, K9, K26–K30** | fordítás, ASS/SSA, hangleválasztás, igazítás, felirat-QC, SBV | Subtitle Studio 1.2 (F2, F5), ADR 0008 M10/M15 |
| **K35** | élő felirat | `CFA4-LIVE-STUDIO-001` (ST2110 melléklet említi) |
| **K39** | eseményfeliratkozás | `CFA4-EVENT-FABRIC-001` 1.0 |
| **K40** | Python-kliens és minták | `open-editors-parity.md` E11 |
| **K41** | MI-vágási munkafolyamatok (csendvágás, durva vágás) | V1-A/V1-B, E1, E3, E5 és az ADR 0008 kapuk |
| **K43** | Lottie | VFX X-7 / `CFA4-TITLE-CREDITS-001`; **figyelem:** a TITLE-CREDITS 1.1 szövege nem említi, ezt a terv gazdájának kell pótolnia (D-1) |
| **K44** | FFmpeg-szűrők a routeren át | `CFA4-FFMPEG-BACKEND-R1` (filter-interfész), ADR 0005 |
| **K62** | x264/x265/LAME felismerése, inaktív állapot | `CFA4-FFMPEG-BACKEND-R1` (`cfa4-restricted-extras` modell), ADR 0005 |
| **K36** | Ez a dokumentum | – |

### 5B. Marad: új képesség, amelyet semmi más nem hordoz

| K | Képesség | Állapot / megjegyzés | Hullám |
|---|---|---|---|
| **K1** (Kdenlive-dialektus) + **K20** + **K25** | `.kdenlive` olvasása: `kdenlive:` tulajdonságok, effekt/átmenet-azonosítók → CFA4-effekt tábla (a VFX-katalógus `aliases`-ához igazítva), `melt`-összevetés CI-ben. A **mag és az általános leképezés (K42) az SC-5 része**, nem készül kétszer | A `main`-en nincs `.kdenlive`-olvasó (csak OTIO). Közös PR-sorozat az SC-5.1-gyel | W1 |
| **K10** (szűkítve) | Export külön folyamatban (összeomlás-elszigetelés, JSON-soros előrehaladás, megszakítás félkész fájl nélkül). A `serve` és a képkocka-`render` megvan; az exportmunka külön folyamata ellenőrizendő | részben | W1 |
| **K11** (szűkítve) | A hiányzó platform-presetek: AV1, HEVC, iPhone, VA-API/GPU, archív (YouTube, WebM VP9, Social Deliverables megvan). **D-2 előbb** (licenc) | részben | W1 |
| **K12** | Kamerák saját proxyfájljainak felismerése (GoPro LRV, DJI LRF, Sony, Insta360), újrakódolás nélkül. A `camera-raw.md` csak metaadat/proxy-megjegyzés; az E12 (automatikus proxydöntés) más képesség | nincs | W1 |
| **K37** (szűkítve) | A forkok eszközlistájának parancsonkénti összevetése a regiszterrel; csak a hiányzó parancs készül (az E6 kis MCP-felületével egyeztetve) | nincs összevetés | W1 |
| **K15** (szűkítve) | Borítókép, YouTube-fejezetszöveg, felhasználói render utáni parancs (a fejezet az MP4/MOV/HLS/MKV exportban megvan) | részben | W2 |
| **K21** | CC0 lumák, LUT-ok, átmenet-előnézetek eszközként (`.attribution`) | nincs | W2 |
| **K23** | Projekt-ellenőrző (hiányzó források, proxyk, effektek, integritás); az SC-F0 terv is erre hivatkozik | nincs | W2 |
| **K38** | Jogosultsági szintek (olvasó/szerkesztő/romboló) az MCP-annotációkra építve; a **V3** (Unix socket, mozdulat alatti visszatartás) előfeltétele a `main`-en még nincs | nincs | W2 (V3 után) |
| **K45, K46, K47, K48** | zip/tar projektarchívum; távoli erőforrás figyelmeztetés; renderelt fájl a Bin-be; külső szerkesztés visszatöltése | kapunál ellenőrizendő (a K46-ot az SC-F0 és a VIDEO-EXTERNAL-SOURCES is említi) | W2 |
| **K61** | Render szüneteltetése és folytatása | kapunál ellenőrizendő (`serve.rs` említ szünetet) | W2 |
| **K13** | Giroszkópos stabilizálás `gyroflow-core`-ral; **D-4 előbb** (licenc) | nincs (G18 a 2-D/3-D stabilizátort fedi, a giroszkópot nem) | W3 |
| **K14** | Párhuzamos és hálózati render, render-only kliens; a render C0 worker-szerződésére és a `cfa4-tasks`-ra épül | semmi sem fedi (a render-terv helyi) | W3 |
| **K16** | Kötegelt átkódolás a közös feladatsoron; egyeztetni a `LOSSLESS-MEDIA-TOOLS-001`-gyel (#1124) és a VISION konvertálás-területtel | részben a VISION-ban, terv nincs | W3 |
| **K18** (szűkítve) | Szolgáltatói adatfájl (Hugging Face, Freesound, Pixabay), jóváhagyó felület; a letöltő maga (`cfa4-fetch`) megvan | részben | W3 |
| **K8** | Hangspektrum- és spektrogram-panel | nincs (a `speech/mel.rs` ASR-hez van) | W4 |
| **K49–K53, K55–K60** | sávátrendezés; marker a résekbe; áttűnés-előnézet; több klip sebessége; rögzített lejátszófej; minden effekt ki/be; Delogo; lekerekített téglalap a címszerkesztőben; szóló Shift+kattintással; hanghullám beágyazott szekvencián; többmonitoros munka (a #466 lebegő ablakai részben fedik) | kapunál ellenőrizendő pontonként | W4 |
| **K24** | Diavetítés-automatizálás (Ken Burns, ütemhez igazítás); licencellenőrzés előbb | nincs | W5 |
| **K34** | Figyelt mappa: bedobott médiából átirat és felirat automatikusan (K16 feladatsor, AI-F5) | nincs; MI-kapu | W5 |

## 6. Végrehajtási terv (hullámok, csak az 5B pontjai)

Minden hullám **külön tulajdonosi indítási engedélyt** igényel, és mindegyik K-pont előtt a IV. pont kapuja lefut (friss `main`, nyitott PR-ok, függőség-ellenőrzés). Egy PR egy K-pont (vagy lépése), kész PR „Ready for review”, „Gazda: …” issue. A becslések ügynökórák, nem mértek; a K-pontok alaptervi becsléseiből, a szűkítésekkel.

| Hullám | Tartalom | Előfeltétel | Becslés (ó) | Kilépési bizonyíték |
|---|---|---|---|---|
| **W0** | Terv elfogadása; D-1…D-7; az alapterv átvezetése (az 5A pontjainak törlése, hivatkozásra cserélve) | tulajdonosi jóváhagyás | – | jóváhagyás |
| **W1 — `.kdenlive` és MI nélküli mag** | K1 (Kdenlive-dialektus), K20, K25 az SC-5 közös magjára; K10 (szűkítve); K11 (szűkítve); K12; K37 (szűkítve) | W0; D-2; a #1559 beolvasztása; render C0 a K10-hez | K1+K20+K25 25–45 (a mag az SC-5-é), K10 10–20, K11 10–20, K12 8–15, K37 5–10 | a K1 „Kész, ha”: mintaprojektek megnyílnak, vágópontok képkocka-pontosak, kompatibilitási jelentés, ellenséges bemenet nem pánik; `melt`-összevetés CI-ben |
| **W2 — kicsi, motoros** | K15 (szűkítve), K21, K23, K45–K48, K61, K38 (V3 után) | W1 K1 a K23-hoz; V3 a K38-hoz | K15 6–10, K21 6–10, K23 12–20, K45–K48 12–24, K61 6–12, K38 10–20 | pontonként a „Kész, ha” sor |
| **W3 — közös infrastruktúrára épülők** | K14, K16, K18 (szűkítve), K13 | render F3/C0; ACS-8/`cfa4-tasks`; D-4 | K14 70–120, K16 25–40, K18 8–15, K13 35–55 | két gépen azonos kimenet (K14); hash-ellenőrzött letöltés (K18) |
| **W4 — Qt6-felület és motor** | K8, K49–K53, K55–K60 | a QT6-FINISH-001 kapui | 60–120 | Qt-teszt Wayland/X11 (K60), automatizálási azonosítók |
| **W5 — licenc- vagy MI-kapuhoz kötött** | K24, K34 | licenc-ellenőrzés; AI-F5; K16 | 23–37 | az ADR 0008 kapuk kilépési feltételei |

**Összes becslés:** kb. 330–590 ó (az alaptervi 610–1035 helyett; a különbség a kikerült, kész vagy más terv által hordozott pontok). A W1 a legnagyobb értékű és MI nélküli: a `.kdenlive` megnyitása a kötelező döntés (K1), és az SC-5 ugyanazt a magot használja.

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
| 2026-11-22 | 1.1 (javaslat): István kérésére az 5. pont szűrve: csak a CFA4-ben és más tervben nem szereplő képességek maradnak (5A kikerül, 5B marad); a hullámok ennek megfelelően újraszámolva |
| 2026-11-22 | 1.0 (javaslat): K36 elvégezve; delta, tárolók és forkok állapota, K-pontonkénti állapot, hullámok, nyitott döntések |
