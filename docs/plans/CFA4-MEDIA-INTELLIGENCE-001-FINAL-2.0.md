# CFA4-MEDIA-INTELLIGENCE-001 — FINAL 2.0

**Cím:** Media Intelligence – egységes intelligens multimédiás kereső- és tartalomértelmező rendszer  
**Projekt:** CinemaFundArchitecture4 (CFA4)  
**Dátum:** 2026-10-31  
**Státusz:** **APPROVED / FINAL 2.0**  
**Jóváhagyás:** A felhasználó a `FINAL DRAFT 2.0` teljes tervét kifejezetten jóváhagyta.  
**Végrehajtási korlát:** **A fejlesztés és a telepítés NINCS engedélyezve, NEM indulhat el.** Ez kizárólag a terv dokumentációs véglegesítése.  
**Módosítási szabály:** FilmCraft upstream nem módosítható; későbbi implementáció kizárólag CFA4-ben, külön végrehajtási jóváhagyással.

## 1. Cél és hatókör

A CFA4 önálló, újrahasznosítható Media Intelligence szolgáltatást kap. Magyar nyelven és többnyelvű környezetben kereshet videókban, jelenetekben, beszédben, hangeffektekben, objektumokban, metaadatokban és kapcsolt produkciós dokumentumokban. A keresési infrastruktúra a Media Bin, Timeline, Source Monitor, feliratozó, Story/Shot és kapcsolódó modulok közös szolgáltatása lesz.

**Alapelvek:** offline-first; CPU-only kötelező; GPU opcionális; nem destruktív; magyar nyelvi elsőbbség; különálló, cserélhető AI-modellek; igazolt források és eredetkövetés; jogosultságszűrés; nincs kötelező felhő vagy második autoritatív adatbázis.

**Kiadási elv:** a teljes Media Intelligence nem előfeltétele a CFA4 kezdeti terjesztésének. Magyar metaadat-, beszéd- és szövegkeresés + Qt6 integráció az első önállóan kiadható szint, megfelelő tesztek után.

## 2. CFA3-ból adaptált képességek

A CFA3 Document Manager / Document Fabric / Source Manager / Knowledge Fabric tervei *tervezési források*, nem bizonyítottan kész szoftverek. Az átvétel szerződés- és képességadaptáció, nem upstream kódmásolás.

| ID | Képesség | CFA4 szerep | Fázis |
|---|---|---|---|
| DM-01 | Universal Search | Közös multimédiás kereső API | F1 |
| DM-02 | Canonical Document/Source ID | Tartós média- és forrásazonosítók | F0 |
| DM-03 | Full-text Search | Átirat, felirat, OCR, dokumentumszöveg | F1 |
| DM-04 | Metadata & Index | Formátum, kamera, projekt, dátum, tagek | F1 |
| DM-05 | Semantic Search / RAG | Természetes nyelvű keresés | F2–F4 |
| DM-06 | Structural Anchors | Frame-, idő-, szó-, dokumentumhivatkozás | F0 |
| DM-07 | Provenance | Források és modellkimenetek eredete | F0 |
| DM-08 | Permission-aware Retrieval | Projektjogosultságok | F1 |
| DM-09 | Revision Management | Index- és dokumentumrevíziók | F2 |
| DM-10 | Auto Tagging | Jelenet-, objektum- és hangcímkék | F3 |
| DM-11 | Deduplication | Azonos/közel azonos médiák | F3 |
| DM-12 | Change Intelligence | Inkrementális újraindexelés | F1 |
| DM-13 | Knowledge Graph | Média/dokumentum/gyártás kapcsolatok | F4 |
| DM-14 | Structured Extraction | Script, shotlist és gyártási adatok | F4 |
| DM-15 | Compare / Diff | Átirat, felirat, metaadat változásai | F4 |
| DM-16 | Collections / Attachments | Médiagyűjtemények és kapcsolatok | F2 |
| DM-17 | Cross-application Handoff | Más CFA4-moduloknak találatátadás | F2 |
| DM-18 | Content Provenance Classification | Emberi/AI/gépi adat eredete | F1 |

## 3. Funkcionális követelmények

| ID | Funkció | Példalekérdezés |
|---|---|---|
| MI-01 | Metaadatkeresés | 2024-es 4K felvételek |
| MI-02 | Beszédkeresés | „holnap találkozunk” |
| MI-03 | Magyar szemantikus keresés | beszélgetés a költségvetésről |
| MI-04 | Objektumkeresés | piros autó az utcán |
| MI-05 | Jelenetkeresés | esős, éjszakai város |
| MI-06 | Mozgás/esemény | valaki kiszáll az autóból |
| MI-07 | Hangeffekt | eső és mennydörgés |
| MI-08 | OCR | Budapest felirat a képen |
| MI-09 | Beszélőszűrés | második beszélő mondatai |
| MI-10 | Hasonlósági keresés | referencia-képkockához hasonló |
| MI-11 | Multimodális keresés | autó + eső + beszéd |
| MI-12 | Dokumentum–média kapcsolat | forgatókönyv 12. jelenete |
| MI-13 | Projektközi keresés | összes drónfelvétel |
| MI-14 | Eseménysorrend | ajtócsapódás, utána lépések |
| MI-15 | Negatív feltételek | ember nélküli tájak |
| MI-16 | Referenciaalapú keresés | hasonló hangminta |
| MI-17 | Forrásalapú keresés | ember által ellenőrzött jelenetek |
| MI-18 | Revíziós keresés | utolsó index óta változott |

**Magyar nyelv:** beszédfelismerés, ragozás, ékezet nélküli alakok, szinonimák, filmes szakkifejezések, kevert nyelv, többnyelvű tartalmak magyar lekérdezése, időbeli/logikai kifejezések. A lexikai, szövegszemantikus és keresztmodális minőség külön benchmarkot kap. A hiányra vonatkozó `NOT` feltétel csak elemzési lefedettséggel igazolható.

## 4. Modularchitektúra

**Felső szint:** CFA4 Qt6 alkalmazások → Universal Search API → Rust Media Intelligence Core → AI-adapterek és Media Knowledge Layer → PostgreSQL/pgvector + CFA4 FFmpeg Engine.

| Modul | Felelősség |
|---|---|
| `cfa4-mi-core` | Modul- és feldolgozási életciklus |
| `cfa4-mi-api` | Közös kereső API, jogosultság, bizonyíték |
| `cfa4-mi-indexer` | Indexelés, job scheduler, checkpoint |
| `cfa4-mi-query` | Magyar nyelvű lekérdezés strukturálása |
| `cfa4-mi-search` | Hibrid lexikai/szemantikus keresés |
| `cfa4-mi-vision` | Jelenet, objektum, OCR, vizuális embedding |
| `cfa4-mi-speech` | ASR, időzítés, beszélők |
| `cfa4-mi-audio` | Hangesemények és audioembedding |
| `cfa4-mi-storage` | SQL, migráció, indexgeneráció |
| `cfa4-mi-knowledge` | Tudásgráf, dokumentumkapcsolatok |
| `cfa4-mi-provenance` | Bizonyítékok és eredetkövetés |
| `cfa4-mi-revisions` | Revízió és változáskövetés |
| `cfa4-mi-docbridge` | Kapcsolt dokumentumok adapterei |
| `cfa4-mi-models` | Modellek és erőforrás-felügyelet |
| `cfa4-mi-qt` | Qt6 dokkolható felület |

**Határ:** az AI-workerek csak típusos feldolgozási kimenetet adhatnak vissza; közvetlen, autoritatív projektadatbázis-írásuk tilos. A keresés nem függhet kötelező LLM-től.

## 5. Technológiai jelöltek és elfogadási feltételek

| Feladat | Jelölt | Kapu |
|---|---|---|
| Média-dekódolás | CFA4 FFmpeg 9.0.2 célbaseline | Host-validáció |
| Jelenethatárok | PySceneDetect módszerek / FFmpeg | Licenc + snittteszt |
| Magyar ASR | faster-whisper, alternatíva whisper.cpp | Magyar WER + CPU |
| Szóidőzítés | WhisperX | Magyar igazítás teszt |
| Diarizáció | pyannote.audio | Modelllicenc + minőség |
| Kép/text | OpenCLIP / SigLIP-típusok | Magyar cross-modal benchmark |
| Objektumok | YOLO-család / RT-DETR | Modell- és súlylicenc |
| Audio/text | LAION CLAP | Magyar hangteszt |
| OCR | PaddleOCR / kisebb CPU adapter | Magyar OCR teszt |
| Szövegembedding | Többnyelvű embedding modell | Recall@K + licencek |
| Adatbázis | PostgreSQL + pgvector | Tranzakciók, méretezés |
| ANN opció | FAISS | Csak mért szükség esetén |

Kerülendő a nem kereskedelmi súlyok alapcsomagba foglalása és minden automatikus licencfeltételezés. A konkrét modellverziók F0 során választandók ki.

## 6. Adatséma és azonosítók

**Logikai ID-k:** `ProjectID`, `MediaID`, `ContentHash`, `StreamID`, `SegmentID`, `FrameAnchor`, `TimeRangeID`, `DocumentID`, `RevisionID`, `SpeakerID`, `ModelVersionID`, `EvidenceID`, `GraphEdgeID`.

**Sémacsoportok:** `assets`, `streams`, `sources`, `locations`; `segments`, `time_anchors`, `scene_boundaries`; `transcripts`, `words`, `speakers`; `objects`, `object_tracks`, `ocr_text`; `audio_events`, `audio_segments`; `embeddings`, `embedding_models`; `graph_nodes`, `graph_edges`; `documents`, `document_anchors`, `asset_links`; `evidence`, `provenance`, `review_status`; `jobs`, `job_checkpoints`, `index_generations`; `saved_queries`, `collections`, `feedback`; `revisions`, `change_events`, `tombstones`.

Eredeti médiák a CFA4 fájlkezelésében maradnak, nem kötelező BLOB-ként SQL-be menteni. Időalap: eredeti PTS/racionális timebase; VFR-helyesség és timeline-leképezés kötelező. Indexgenerációk tranzakcióbiztos cseréje, változáskor célzott újraindexelés; fájlátnevezés ne váltson ki felesleges médiaelemzést.

## 7. Keresési terv és bizonyíték

Összetett lekérdezés: lexikai + szemantikus + vizuális + audio + strukturált + időbeli részfeltételek. Támogatandó `AND`, `OR`, `NOT`, `BEFORE`, `AFTER`, `OVERLAPS`, `WITHIN`. A query planner strukturált, ellenőrizhető reprezentációt adjon; az LLM ne állíthasson elő közvetlen, kontrollálatlan SQL-t. Találat: forrás, szegmens/időpont, bizonyítéktípus, modellverzió, eredet, ellenőrzési állapot, indexlefedettség és relevancia. Relevancia ≠ bizonyosság.

## 8. Media Knowledge Graph és Document Bridge

**Csomópontok:** `Project`, `Document`, `ScriptScene`, `Shot`, `Take`, `MediaAsset`, `MediaSegment`, `Object`, `Speaker`, `Transcript`, `AudioEvent`, `Location`, `Collection`.

**Élek:** `BELONGS_TO`, `REPRESENTS_SCENE`, `RECORDED_BY`, `APPEARS_IN`, `SPOKEN_IN`, `HAS_TRANSCRIPT`, `DERIVED_FROM`, `SIMILAR_TO`, `REFERENCES`, `HAS_REVISION`, `CONTAINS`.

**Document Bridge források:** forgatókönyvek, shotlisták, storyboardok, SRT/VTT/ASS, EDL/XML/OTIO, PDF/DOCX/ODT, CSV/JSON, hangjegyzetek, képek/OCR. Dokumentumot nem módosít: kereshető nézeteket és hiteles anchorokat képez. Első körben PostgreSQL kapcsolattáblák; külön graph DB csak szükség esetén.

## 9. Qt6 UI és integráció

Dokkolható Media Intelligence panel, Media Bin/Timeline kontextuális keresés, közös API és parancssoros hozzáférés. Találati bélyegkép, időtartomány, relevancia, forrás, bizonyíték, modell és indexállapot megjelenítése. Műveletek: Source Monitor megnyitás, timeline-ugrás, gyűjteménybe helyezés, feliratozó/dokumentum megnyitás, mentett lekérdezések. Más CFA4 AI-funkciók (Object Mask, Enhance Speech, Generative Extend) csak külön interfészen kapjanak szegmenshivatkozást; ezen terv nem valósítja meg őket.

## 10. Célplatform, erőforrások és indexelés

**Platform:** Ubuntu 26.04 LTS és variánsok, amd64, Qt6, KDE Plasma/Wayland elsődleges, Rust, Python pip+venv (conda/mamba nélkül), PostgreSQL/pgvector. Csomagformák: DEB, Flatpak, AppImage. FFmpeg 9.0.2 célbaseline.

**Erőforrás:** kötelező CPU-only; opcionális NVIDIA. Interaktív videószerkesztés prioritása háttérindex előtt; VRAM- és RAM-keretek, szüneteltetés, checkpoint, folytatás, degradált üzemmód. A 8 GB VRAM-os kártyán nagy modellek korlátlan párhuzamos futtatása tilos.

**Indexszintek:** L0 metaadat/importált felirat; L1 jelenet és beszéd; L2 vizuális, hang és tudásgráf. Részlegesen elkészült indexeken is kereshető, jól látható lefedettségjelzéssel.

## 11. Biztonság, provenance és adatvédelem

Alapértelmezett offline üzem; engedély nélküli cloud-adatküldés tilos. Jogosultság vizsgálata lekérdezéskor és megnyitáskor; tenant-/projektizoláció. Modellek hash-, forrás- és licencellenőrzése. Prompt injectionnek ellenálló dokumentumfeldolgozás. Törlések és jogosultság-visszavonások index- és cache-invalidationje.

Provenance osztályok: `HUMAN`, `AI_GENERATED`, `AI_TRANSFORMED`, `AI_DERIVED_HUMAN_EDITED`, `SOURCE_DERIVED`, `MACHINE_EXTRACTED`, `MIXED`, `UNKNOWN`.

## 12. Fázisok és kapuk

| Fázis | Tartalom | Kimenet |
|---|---|---|
| F0 | CFA4 audit; CFA3 szerződések; licencek; magyar benchmark | G0: megvalósítási szerződések jóváhagyhatók |
| F1 | Canonical ID, adatbázis, metadata-index, ACL, Universal Search | G1: stabil indexalap |
| F2 | Magyar ASR, jelenetek, Qt6 panel, Source Monitor/Timeline | G2/G3: első kiadható képesség |
| F3 | Vizuális és hangembeddingek, OCR, objektumok | G4: AI-minőség és licenc |
| F4 | Document Bridge, Knowledge Graph, összetett lekérdezés | G5: dokumentum/média integráció |
| F5 | CPU/GPU, VFR, regresszió, csomagok, biztonság | G6: kiadási engedély |

A fázisok részben párhuzamosíthatók, de a UI készültsége, kodekvalidáció és a CFA4 fokozatos piacra vitele elsőbbséget élvez; ez a terv nem blokkolhatja a korai kiadást.

## 13. Elfogadási tesztek

| AC | Ellenőrzés |
|---|---|
| AC-01 | Magyar pontos/ragozott/ékezet nélküli keresés |
| AC-02 | Magyar ASR WER az előre rögzített korpuszon |
| AC-03 | Magyar query → idegen nyelvű releváns találat |
| AC-04 | Vizuális Recall@K |
| AC-05 | Hangesemény-visszakeresés |
| AC-06 | Strukturált és időbeli logikai feltételek |
| AC-07 | Dokumentum–média hivatkozás helyes |
| AC-08 | Provenance minden gépi találathoz |
| AC-09 | Revízió és generációbiztos újraindexelés |
| AC-10 | CPU-only végponttól végpontig |
| AC-11 | GPU-fallback adatvesztés nélkül |
| AC-12 | Qt6 Source Monitor/Timeline időpozíció |
| AC-13 | VFR/PTS és időalap pontosság |
| AC-14 | Jogosultsági izoláció/negatív tesztek |
| AC-15 | Alapkeresés offline |
| AC-16 | Indexelés megszakítás, folytatás |
| AC-17 | Duplikáció kezelése |
| AC-18 | Komponens- és súlylicencek igazolása |
| AC-19 | Host software coexistence |
| AC-20 | Média- és dokumentumfájlok sértetlensége |

**Benchmark induló célok – nem mért tények:** lexikai Recall@10 ≥95%; szemantikus szöveg Recall@10 ≥85%; vizuális Recall@10 ≥80%; magyar tiszta ASR WER ≤15%; indexelt keresés p95 ≤2 s definiált adathalmazon; nulla engedély nélküli forrásmódosítás; jogosultsági tiltások 100%. A mérési módszer és hardver/adatkészlet F0-ban véglegesítendő.

## 14. Fejlesztői munkacsomagok (még NEM indíthatók)

| ID | Cím | Prioritás |
|---|---|---|
| DEV-01 | CFA4 architektúra/komponensleltár | P0 |
| DEV-02 | CFA3 Document Fabric illesztés | P0 |
| DEV-03 | Canonical ID / anchor / provenance | P0 |
| DEV-04 | PostgreSQL + pgvector migrációk | P0 |
| DEV-05 | Indexer/checkpoint | P0 |
| DEV-06 | FFmpeg adapter | P0 |
| DEV-07 | Magyar ASR/szövegindex | P0 |
| DEV-08 | Universal Search/query parser | P0 |
| DEV-09 | Qt6 panel | P0 |
| DEV-10 | Monitor/Timeline integráció | P0 |
| DEV-11 | Vizuális index | P1 |
| DEV-12 | Audio/OCR index | P1 |
| DEV-13 | Document Bridge | P1 |
| DEV-14 | Knowledge Graph | P1 |
| DEV-15 | Revízió/change management | P1 |
| DEV-16 | Model/CPU/GPU erőforrás-kezelés | P0 |
| DEV-17 | Magyar benchmark | P0 |
| DEV-18 | Biztonság/licenc/release | P0 |
| DEV-19 | Gyűjtemények/hasonlóság | P1 |
| DEV-20 | Opcionális LLM Q&A | P2 |

## 15. Kizárások és tilalmak

Nem része: teljes CFA3 Document Manager / Office/DTP; kötelező felhő; új autoritatív DB; Object Mask, Enhance Speech vagy Generative Extend funkció implementálása; Windows/macOS/Android célplatform; FilmCraft upstream módosítása; szerződés nélküli licencelt modell beépítése; jogosulatlan fejlesztés, deploy vagy telepítés.

## 16. Döntés és dokumentumlezárás

**Véglegesített döntés:** CFA4 natív, Rust vezérlésű, Qt6-ból elérhető, PostgreSQL + pgvector alapú, offline-first multimédiás keresőrendszer; a CFA3 Document Fabric 18 releváns képessége adaptálva, elsődleges magyar támogatással és fokozatos kiadással.

**Dokumentációs állapot:** FINAL 2.0, jóváhagyott.  
**Fejlesztési állapot:** NOT STARTED BY THIS APPROVAL.  
**Telepítési állapot:** NOT STARTED BY THIS APPROVAL.  
**Következő végrehajtáshoz:** külön, explicit felhasználói engedély szükséges.

## 17. Hivatkozott tervforrások

- `CFA3_UJ_MEGVALOSITASI_TERV_v0_10_2026-10-09.docx`
- `CFA3_TELJES_EGYEZTETETT_MUSZAKI_TERV_MUNKAVALTOZAT_2026-10-09.md`
- `FA_FA2_FA3_CFA3_TELJES_ELERHETO_TARTALMI_GYUJTEMENY_2026-10-08.odt`
- `FA_FA2_FA3_CFA3_IDORENDI_TARTALOMJEGYZEKKEL_2026-10-08.odt`
- E beszélgetés jóváhagyott `CFA4-MEDIA-INTELLIGENCE-001 FINAL DRAFT 2.0` tervének részei.

**Megjegyzés:** E forrásokra való hivatkozás nem állítja, hogy a CFA3 moduljai elkészültek vagy a CFA4 adott komponensei jelenleg igazoltan működnek.
