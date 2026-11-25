# CFA4 Media Cutter – F0 architektúra- és követelményellenőrzés

**Terv:** CFA4-LOSSLESS-MEDIA-TOOLS-001 v1.0 (tervezet); a végleges 1.1: [`docs/plans/CFA4-LOSSLESS-MEDIA-TOOLS-001-1.1.md`](../plans/CFA4-LOSSLESS-MEDIA-TOOLS-001-1.1.md) · **Fázis:** F0 (csak olvasás) · **Dátum:** 2026-11-12
**Vizsgált állapot:** `ubuntuokos/CinemaFundArchitecture4`, `main` @ `108c2d08` (#1107)
**Módszer:** a repó dokumentumainak és kódjának olvasása. Kód nem változott, nem futott build vagy teszt, függőség nem települt.

## Döntési napló

- **2026-11-13 (István):** a 8. fejezet mind a 9 javasolt módosítását elfogadta. Ezzel a D2
  (Rust-motor az alap, FFmpeg csak útválasztón át), a D4 (saját `.cfa4cut` munkamenetfájl) és a
  D5 (`cfa4-qt --cutter` és saját `.desktop` bejegyzés) eldőlt. A 8/6. pont szerint az F2 a
  SCE-001 P2/P2a/P3 fázisával egyesül.
- **2026-11-13 (István):** a 9. fejezet minden javaslatát elfogadta:
  - **D1:** az F1 felületi része a Qt6-paritás előtt indulhat, és csak a meglévő
    `export.plan/start` parancsokra épül. Az F2 (remux) a SCE-001 P2–P3 fázisával együtt, annak
    ütemezésében készül.
  - **D3:** az F1-ben nincs FFmpeg-háttér a stream copyhoz. Ha az F2-höz kell, előtte külön
    döntés jön a SCE-001 hatókörének kiegészítésével.
  - **D6:** a végleges név külön döntés marad; a munkanév „CFA4 Media Cutter”.
  - **D7:** az átfedést az F1 indítása előtt jelezni kell a gazdának (merge koordinátor,
    `docs/coordination.md` §2/8.). A szöveg az A. mellékletben van.
- **2026-11-13 (István): irányváltás.** „Nem külön alkalmazás kell, hanem a CFA4-ből hiányzó
  képességek átvétele.” Ez felülírja a következőket:
  - **A D5 megszűnik.** Nincs `cfa4-qt --cutter` indítási mód, külön `.desktop` bejegyzés és
    külön ablak.
  - **A D6 tárgytalan.** Nincs külön termék, ezért név sem kell; a „Media Cutter” csak a terv
    munkaneve marad.
  - **A D4 helyett** a szegmensek projektadatok: tartományjelölők és subclipek a `.fcproj`-ban.
    Külön `.cfa4cut` fájl nem készül. (Ez az én olvasatom az irányváltásból; ha mégis kell
    projekt nélküli munkamenetfájl, jelezd.)
  - **Változatlan marad:** D1 (a Qt6-felületbe kerülő részek a paritás előtt is indulhatnak), D2,
    D3, D7 és a 8. fejezet többi pontja.
  - Az új feladatlista a 0. fejezetben van; az 1–9. fejezet ennek fényében olvasandó.
- **2026-11-13 (István):** a 4. fejezet (adatarchitektúra) javaslatait elfogadta:
  - a munkamenet fájlban tárolódik;
  - a PostgreSQL opcionális, későbbi index, saját séma nélkül, és a képességek PostgreSQL nélkül
    is teljes értékűen működnek;
  - a szegmenshatárok `Tick`-ben, a forrás PTS-idővonalán tárolódnak.

  A 4/1. pont fájlformátuma az irányváltás után a `.fcproj` (b változat): tartományjelölők a
  mesterklipen. A korábban javasolt `.cfa4cut` nem készül. István 2026-11-13-án a döntéskártyán ezt megerősítette.
- **2026-11-13 (István):** „A hiányzó követelményekre megoldás kell, a részben meglévőket meg teljessé kell tenni.” A terv 1.1 3.4 pontja a 2. fejezet minden 🟡/❌ sorához megoldást és teljességi feltételt ad (új: K12–K16). Javítás a 2. fejezethez: az MPEG-2 határ-újrakódolás azóta elkészült (WP-09e).
- **Az F0 állapota:** a követelménylista a 0. fejezettel együtt lezárható. A D7 jelzése még
  hátravan. A megvalósítás indításához a terv 14. pontja szerint külön jóváhagyás kell.

---

## 0. Átdolgozott irány: a CFA4-ből hiányzó képességek

A LosslessCut-funkciók közül csak azok kellenek, amelyek a CFA4-ből **hiányoznak**. Mindegyik a
meglévő helyére kerül: forrásmonitor, Jelölők panel, Export ablak, Eszközök menü, illetve a
motor meglévő parancscsaládjai. Az egui be van fagyasztva, ezért minden új felület csak a
Qt6-ba kerül (VISION 2.1).

| # | Hiányzó képesség | Hová kerül a CFA4-ben | Kinek a területe |
|---|---|---|---|
| K1 | Több vágási tartomány egy forráson, és ezek egyenkénti vagy együttes exportja újrakódolás nélkül | Forrásmonitor ▸ Smart Cut Source… ablak: tartománylista a forrás tartományjelölőiből (`markers.addRange`); motor: `export.plan/start` több tartományra, a `jobs.*` / `export.queue.*` sorban | Media Cutter (ez a projekt) |
| K2 | Szegmens mint projektadat: név, címke, megjegyzés, sorrend; mentés és visszatöltés | Meglévő `Marker` (név, szín, megjegyzés) a mesterklipen, a `.fcproj`-ban; hiányzik az exportsorrend és a „szegmens” nézet a Jelölők panelen | Media Cutter |
| K3 | Szegmensek átadása az idővonalra | Új parancs: kijelölt tartományok → subclipek vagy idővonal-klipek (a `clip.sceneEditDetection` subclip-ágának mintájára) | Media Cutter |
| K4 | Jelenetfelismerés forrásklipre, az eredmény tartományjelölő | A `clip.sceneEditDetection` kiterjesztése projektelemre és forrásmonitorra (ma csak idővonal-klipre fut) | Media Cutter |
| K5 | Fekete- és csendszakaszok keresése állítható küszöbbel, az eredmény jelölő vagy tartomány | A `cfa4-qc` elemzői (`picture.rs` `BlackRun`, `audio.rs` csend) új parancsként, amely jelölőket ad; ma csak QC-jelentésben jelennek meg | Media Cutter |
| K6 | Jelölők CSV-importja, a forrás időalapjának ellenőrzésével | `markers.importCsv` a `markers.exportCsv` párjaként; az EDL már van (`cfa4-interchange`) | Media Cutter |
| K7 | Képkocka-export formátum- és minőségválasztással | A `file.exportFrame` bővítése (ma PNG) | Media Cutter |
| K8 | Smart Cut megszakítása és műveleti napló | A `jobs.cancel` a Smart Cutra (WP-09 18. eset); a napló a meglévő export-összegzéshez kerül | SCE-001 / R1 (egyeztetés) |
| K9 | Matroska (MKV/WebM) forrás Smart Cutja | `cfa4-smart-edit` + `cfa4-export` | SCE-001 / R1 |
| K10 | Streamválasztás, -kinyerés, hang- és feliratsáv hozzáadása vagy cseréje, remux | Export ablak Stream Copy módjai | SCE-001 P2/P2a/P3 (8/6. pont) |
| K11 | Forgatás és metaadatok szerkesztése, besorolással (konténer / stream / kép) | Projekt panel ▸ Tulajdonságok, illetve a Compatible Copy metaadat-szabályai | SCE-001 P3 (egyeztetés) |

**Ami már megvan, és nem kell átvenni:** GOP-határ és képkockapontos forrásvágás, a módszer
előzetes kijelzése, újrakódolás csak megerősítéssel, AAC/PCM hangmásolás, kimenet-ellenőrzés,
atomikus írás, streamadatok megjelenítése, jelölők, fejezetek és EDL-export (2. fejezet).

**A megvalósítás sorrendje (javaslat):** K1 → K2 → K3 (a vágás munkafolyamata), utána K4 → K5
(automatikus elemzés), majd K6 → K7. A K8–K11 a SCE-001 ütemezésében készül, a gazdával
egyeztetve (D7).

---

## 1. Összefoglaló

1. **A Media Cutter magja nagyrészt már megvan a CFA4-ben, csak más néven.** A jóváhagyott
   CFA4-SCE-001 v1.5 (Stream Copy / GOP Smart Cut / Smart Render) és a CFA4-SMART-CUT-RENDER-R1
   terv alapján a motorban már működik a **projekt nélküli, fájlútvonal alapú forrásvágás**
   (`export.analyze` / `export.plan` / `export.start`, `"scope": "source"`). Tudja a GOP-határ és a
   képkockapontos módot, a bitazonos és kompatibilis garanciát, a megerősítéshez kötött
   határ-újrakódolást, az AAC/PCM hangmásolást és az atomikus, visszaolvasással ellenőrzött írást.
   A Qt6-ban is van már belépési pont: Eszközök ▸ Videóeszközök ▸ Smart Cut Source…
   **A Media Cutter tehát felület- és munkamenet-réteg legyen a meglévő motor fölött, ne második vágómotor.**
2. **Két ponton a terv szövege ütközik a repó elfogadott döntéseivel**, ezért az F0 lezárása előtt
   módosítani kell:
   - **„Az FFmpeg legyen a médiafeldolgozás alapja” (5.1).** Az ADR 0005 szerint a saját Rust-motor
     az alap, az FFmpeg csak a `cfa4-route` útválasztó mögötti kiegészítő háttér. A tulajdonos
     2026-10-27-i döntése szerint a stream copy, a Smart Cut és a muxolás saját Rust-kódon fut, ahol
     az FFmpeg csak tesztorákulum. A kiadott csomagokban ráadásul nincs `ffmpeg` parancssori
     program, csak megosztott könyvtárak, így „FFmpeg-feladatok indítása és felügyelete” külső
     folyamatként a mai architektúrában nem lehetséges.
   - **„Rust + Qt6” mint két összekötött réteg (5.2–5.3).** A CFA4 Qt6-felülete **PySide6
     (Python)**, és a Rust-motort gyermekfolyamatként (`cfa4-cli serve`, JSON-sorok stdin/stdouton)
     éri el (ADR F8). Rust–Qt kötés nincs, és nem is tervezett. A terv elve („a GUI nem épít
     FFmpeg-parancsot”) ezzel összhangban van; csak a szöveget kell a valós rétegekhez igazítani.
3. **Az önálló indítás illeszkedik a VISION.md 7. alapelvéhez** („külön is indítható részek, egy
   CFA4”). Van rá két minta: a Feliratozó D1 döntése és a Stáblista saját `.desktop` bejegyzése.
   Külön kódbázis vagy külön motor nem készülhet.
4. **Adattárolás:** a PostgreSQL-klaszter (ADR 0006, TASKSTORE-PG-001) a katalógusoké, indexeké és
   feladatoké. A TASKSTORE-PG-001 kimondja, hogy **a projektfájl a CFA4 saját formátuma marad**. A
   Rust-motorban még nincs PostgreSQL-kliens, és a Rust-meghajtó sincs kiválasztva. Ezért a
   szegmenslistát és a munkamenetet **fájlban** javaslom tárolni, a PostgreSQL legyen opcionális
   későbbi index.
5. **Ütemezési ütközés:** a SCE-001 P1–P7 fázisai a Qt6-kapuhoz vannak kötve (`docs/qt-parity.md`
   minden sora ✅; ma 59/63). A Media Cutter F1–F2 ugyanezt a területet érinti. Hogy a Media Cutter
   e kapu előtt indulhat-e, az tulajdonosi döntés. A CLAUDE.md szerint az átfedést a merge
   koordinátornak kell jelezni (`docs/coordination.md` §2.8).
6. **Licenc:** a terv 8. pontja összhangban van az AGENTS.md szabályával: GPL-2.0-only kód nem
   vehető át, csak a funkciók szolgálhatnak referenciaként. Az F1–F2-höz várhatóan **nem kell új
   külső függőség**.

**F0 kilépési feltétel állapota:** a követelménylista a 8. fejezet módosításaival lezárható, az
architektúra jóváhagyásához viszont a 9. fejezet tulajdonosi döntései kellenek.

---

## 2. Követelményenkénti átfedés-mátrix

Jelölés: ✅ megvan és teszt igazolja · 🟡 részben · ❌ hiányzik · ⚠ eltér a terv elvárásától

| Terv | Követelmény | Állapot | Hol van a CFA4-ben / mi hiányzik |
|---|---|---|---|
| 3.1 | Kulcsképkockához igazított, újrakódolás nélküli vágás | ✅ | `crates/smart-edit` (registry, GOP-index, planner), `crates/export/src/smart_cut.rs`, `crates/engine/src/smart_cut.rs`. H.264/HEVC/AV1 MP4/MOV-ban; MPEG-2 PS/TS-ben csak GOP-másolás (`smart_cut_mpeg.rs`) |
| 3.1 | Kért és ténylegesen megvalósítható vágáspont kijelzése | ✅ | `export.plan` GOP-határ módban visszaadja a legközelebbi biztonságos pontokat, és csendben nem igazít (`a_mid_gop_cut_is_refused_…`) |
| 3.1 | Stream copy és pontos (újrakódolt) vágás külön kezelése, a módszer látható | ✅ | `mode: gop-boundary / frame-accurate`, `guarantee: bit-exact / compatible`; újrakódolás csak `"confirm": true` mellett. Qt: `panels/smart_cut.py`, `panels/smart_export.py` |
| 3.1 | Több szegmens egymás utáni exportja | 🟡 | Egy hívás egy tartományt vág. Kötegelés a `jobs.*` / `export.queue.*` mintájára kell |
| 3.1 | Határ-újrakódolás | 🟡 | Csak H.264 és HEVC (SCE-001 7. döntés: R1-ben csak H.264, a HEVC csak NVENC-cel gyártófüggő). AV1/MPEG-2 esetén kulcsképkockára kerekít |
| 3.1 | Matroska (MKV/WebM) forrás vágása | ❌ | A forrásvágás MP4/MOV és MPEG PS/TS forrást nyit. A `cfa4-matroska` demux/mux megvan, de Smart Cut útja nincs |
| 3.2 | Szegmenslista (létrehozás, átnevezés, sorrend, címke, megjegyzés) | 🟡 | A projektmodellben van tartomány-jelölő (`markers.addRange`), címkével, színnel és megjegyzéssel. Önálló, projekt nélküli szegmenslista-objektum nincs |
| 3.2 | Mentés/visszatöltés, átadás a szerkesztőnek | ❌ | Fájlformátum és átadási parancs kell (5. fejezet) |
| 3.3 | Streamek listázása (videó, hang, felirat, adat), tulajdonságok | ✅/🟡 | `cfa4-cli probe`, `mediaBrowser.probe`, `media.colorInfo`, `media.timecode`; Media Services `probe` (ADR 0007, MS-1). Az adatfolyamok teljessége konténerenként ellenőrizendő |
| 3.3 | Stream-kiválasztás exportra, kinyerés külön fájlba | 🟡 | A Smart Cutnál `audio: bool`, és az eldobott sávokat okkal jelzi (`AudioPlan::dropped`). Általános streamválasztás és -kinyerés nincs; `clip.extractAudio` idővonal-művelet |
| 3.3 | Hang/felirat hozzáadása, cseréje | ❌ | Új funkció (SCE-001 Strict/Compatible Copy végrehajtóra épülhet) |
| 3.4 | Remux (MKV↔MP4), kompatibilitás-ellenőrzés | 🟡 | A kompatibilitási döntés megvan (registry, `Mux Mpeg2Video Mp4: Unsupported`). A teljes fájlos Strict/Compatible Copy végrehajtó és a natív TS-muxer bővítése a SCE-001 P2–P3 / P2a fázisa, **még nincs kész** |
| 3.5 | Jelenetfelismerés | 🟡 | `clip.sceneEditDetection` (`crates/render/src/scene.rs`): érzékenység, minimális felvételhossz, jelölők és subclipek. Idővonal-klipre fut, forrásfájlra nem |
| 3.6 | Fekete képkockák | 🟡 | `crates/qc/src/picture.rs` (`BlackRun`, blackdetect-kompatibilis küszöbök), QC-jelentésként. Szegmenssé alakítás nincs |
| 3.7 | Csendfelismerés | 🟡 | `crates/qc/src/audio.rs` (`SILENCE_DBFS = -60`, `SILENCE_MIN_S = 2`), QC-jelentésként. Állítható küszöb és szegmenssé alakítás nincs |
| 3.8 | Jelölők, fejezetek | ✅/🟡 | `markers.*` (sok parancs), `MarkerKind::Chapter`. Fejezetírás MP4/MOV/MKV-be a CHAPTERS-DASH terv szerint |
| 3.8 | CSV/EDL import-export | 🟡 | `markers.exportCsv`, `crates/interchange/src/edl.rs` (CMX EDL), továbbá OTIO, FCPXML, AAF, ALE. CSV-jelölőimport és a forrás-időalap ellenőrzése a szegmenslistára hiányzik |
| 3.9 | Metaadat és orientáció megjelenítése | 🟡 | Forgatási mátrix olvasása/írása: `isobmff`, `matroska` (`rotation`). A szerkesztés és a „konténer / stream / képtartalom” besorolás hiányzik |
| 3.10 | Képkocka-export | ✅ | `file.exportFrame` (PNG), `frame.render`, Media Services `frame` (MS-1). Formátum- és minőségválasztás bővítendő |
| 4.3 | Műveleti visszajelzés (forrás, cél, módszer, figyelmeztetés) | ✅/🟡 | Az `export.plan` összegzése és a Smart Export ablak mutatja; a műveleti napló a Media Cutterhez kell |
| 5.1 | Kimeneti fájl ellenőrzése | ✅ | `verify_mp4_copy`, `verify_audio_copy`, `cfa4-qc` (`qc.run`) |
| 7 | Eredeti nem íródik felül; atomikus írás; nincs részleges fájl | ✅ | Közös atomikus író (SCE-001 6. döntés); `start_refuses_before_writing_anything`. Felhasználói megszakítás a Smart Cutban még nincs (WP-09 18. eset) |

**Következtetés:** az F1 kilépési feltételéhez („megbízhatóan működő alapvető vágás”) a
motoroldali munka kicsi. A fő munka a **Qt6 munkaterület, a szegmenslista-modell, a munkamenetfájl
és a kötegelés**. Az F2 (remux, streamkezelés) viszont **a SCE-001 P2–P3 fázisával azonos munka**,
ezért ott külön tervet nem érdemes indítani, csak a SCE-001-et kell előre hozni vagy bővíteni.

---

## 3. Architektúra: a terv és a repó ütközései

### 3.1. Az FFmpeg szerepe (terv 2., 5.1) ⚠ kritikus

| A terv szövege | A repó érvényes döntése |
|---|---|
| „FFmpeg-alapú médiafeldolgozás”, „Az FFmpeg legyen a médiafeldolgozás alapja” | ADR 0005: **a saját Rust-motor az alap**, a kiegészítő hátterek sorrendje saját utak → FFmpeg 9 → GStreamer (`crates/route/routing.toml`) |
| „Elindítja és felügyeli az FFmpeg-feladatokat” | Hívó kód soha nem hív közvetlenül FFmpeg-et, csak a motor felületeit. FFmpeg csak a `cfa4-ffmpeg` crate-en át (L5, feature `ffmpeg`, megosztott könyvtárak). A csomag **nem szállít `ffmpeg` CLI-t** (`packaging/linux/stage-ffmpeg.sh` csak a `lib/`-et és a licenceket másolja) |
| (nincs szó róla) | Tulajdonosi döntés, 2026-10-27: a Stream Copy, a GOP Smart Cut, a Smart Render és a muxolás **saját Rust-kódon** fut, az FFmpeg csak CI-orákulum (`crates/smart-edit/README.md`: „No FFmpeg entry is registered”) |

**Javasolt új szöveg az 5.1-hez:** „A médiafeldolgozás a CFA4 Rust-motorján fut (ADR 0005). A
motor a `cfa4-route` útválasztón keresztül az FFmpeg 9-háttért (`cfa4-ffmpeg`) is használhatja
azokra a formátumokra, amelyeket a saját út nem kezel. A Media Cutter saját FFmpeg-hívást vagy
-parancsot nem tartalmaz.” Az, hogy a stream copy útjain FFmpeg-háttér is lehessen (például
MKV-remuxra, amíg nincs natív út), külön tulajdonosi döntés. Ma a SCE-001 ezt kizárja.

A jelenet-, fekete- és csendfelismerés ugyanígy legyen a motor saját elemzője (`cfa4-render::scene`,
`cfa4-qc`), ne FFmpeg-szűrő (`scdet`, `blackdetect`, `silencedetect`). A meglévő küszöbök már
blackdetect-kompatibilisek, így az eredmények FFmpeg-gel orákulumként összevethetők.

### 3.2. Rust + Qt6 réteghatár (terv 5.2–5.3)

```text
Media Cutter Qt6 munkaterület / külön ablak  (python/cfa4_qt, PySide6)
  -> EngineClient (cső, olvasószál, Qt-jelek)
     -> cfa4-cli serve (fejetlen Rust Session; JSON-sorok + bináris farok)
        -> engine-parancsok: export.analyze/plan/start, markers.*, qc.run,
           clip.sceneEditDetection, media.service.* (ADR 0007), files.* (ADR 0009)
           -> cfa4-smart-edit, cfa4-export, cfa4-media, cfa4-route (-> cfa4-ffmpeg / GStreamer)
```

- A terv **„Rust szolgáltatási rétege” = új engine-parancsok** a `crates/engine`-ben („minden
  parancs”: id, címke, menü, gyorsbillentyű, paraméterek, `enabled()`, `run()`). Így a CLI, az MCP
  és a vezérlőcsatorna is eléri, ami a CLAUDE.md kötelező elve („minden ügynökkel vezérelhető”).
- A **GUI semmit nem számol**. Ezt a `smart_export.py` már így csinálja („The window computes nothing
  itself”).
- **Időalap:** minden időérték `cfa4_time::Tick`. Kódban f64 másodperc nem lehet (CLAUDE.md). A
  terv 3.8-as „nincs csendes időalap-konverzió” követelménye ezzel és a meglévő
  `media.indeterminateTimebase` beállítással összhangban van.
- **Rétegzés:** az új logika L5 alatt nem függhet UI-tól (`cargo xtask layers`).
- **Never crash** (AGENTS.md §0): sérült fájl, hibás CSV/EDL vagy teli lemez esetén `Result` és
  olvasható hiba jön. Ez a terv 7. fejezetét erősíti.
- **Fordítás:** minden felületi szöveg az `i18n/` alá kerül (tulajdonosi döntés 2026-11-09; az
  áthelyezésig a meglévő katalógusokba).

### 3.3. Önálló modul vs. egy CFA4 (terv 6.1)

A VISION.md 7. alapelve szerint egy terület külön alkalmazásként is indítható, saját
menübejegyzéssel, ikonnal és fájltársítással, de **ugyanazt a motort, adatmodellt, parancsokat és
beállításokat** használja, és a CFA4 csomagjával települ. A meglévő minták:

- Stáblista: `packaging/linux/io.github.ubuntuokos.cfa4.credits.desktop` → `Exec=cfa4-qt %f`, a
  `cfa4-qt --credits` kapcsolóval.
- Feliratozó: SUBTITLE-STUDIO-PLAN-001 1.2, D1 és 4.1.

**Javaslat:** `cfa4-qt --cutter [FÁJL]` indítási mód és egy
`io.github.ubuntuokos.cfa4.cutter.desktop` bejegyzés („CFA4 Media Cutter” / „CFA4 Médiavágó”).
Saját programfájl ne készüljön; a VISION 7. pontja ezt a terület tervére bízza. A munka egy
lépésben átadható a szerkesztőnek és vissza.

### 3.4. Ütemezés és projekthatár

- A `docs/roadmap.md` szerint a SCE-001 P1–P7 akkor indul, amikor a `qt-parity.md` minden sora ✅
  (ma 59/63; nyitott: 1.9, 1.10, 4.2, 5.5). A VISION 2.1 szerint a Qt6-felület az első prioritás.
- A Media Cutter F1–F2 a SCE-001 területén dolgozik (stream copy, remux, Smart Cut UI). A
  CLAUDE.md 3. pontja szerint egy projekt nem vehet át feladatot egy másikból. **Az átfedést a merge
  koordinátornak kell jelezni** (`docs/coordination.md` §2.8), és a tulajdonos a koordinátorral
  együtt dönt.

---

## 4. Adatarchitektúra és PostgreSQL (terv 5.4)

**Mai helyzet:**

- PostgreSQL: egy CFA4-klaszter (`pg_createcluster … cfa4`), csak Unix-socket, `peer`
  hitelesítés, modulonként saját séma és szerep (`backlot`, `cfa4_audit`, `cfa4_files`, `tasks`,
  `ai`, `cache`, később `mi`). Az ADR 0006 és a TASKSTORE-PG-001 rögzíti.
- **A TASKSTORE-PG-001 2. pontja kimondja: „a projektfájl a CFA4 saját formátuma marad”.** Képkocka
  és média nem kerül az adatbázisba.
- A Rust-motorban **nincs PostgreSQL-kliens**. A Rust-meghajtó (`tokio-postgres` vagy `sqlx`) az
  első Rust-író kapuján dől el (TASKSTORE PG-fázis vagy Event Fabric EF-R1). A File Services
  katalógusa Pythonban van (`cfa4_files`, psycopg 3).
- Projektfájl: `.fcproj` (JSON, `schema_version`, `cfa4-format`). A média útvonallal hivatkozott,
  `MediaClip::identity` szerint azonosított; a Link Media / `media.relink` / `media.findMissing`
  kezeli az elérhetetlen forrást.

**Javaslat:**

1. **A munkamenet fájl** legyen: a szegmensek, jelölők, beállítások és a forrás hivatkozása
   (útvonal, ujjlenyomat: méret, mtime, tartalomazonosító) egy sémaverziós JSON-ban, a
   `.fcproj` szabályai szerint (atomikus írás, ismeretlen mezők tűrése, minden szám ellenséges).
   Két lehetőség van:
   - (a) saját kiterjesztés és MIME típus, például `.cfa4cut` / `application/x-cfa4-cut`;
   - (b) maga a `.fcproj`, egy „forrás + tartományjelölők” szerkezettel. Ezzel a szerkesztőbe
     adás triviális, mert a jelölők és subclipek már a projektmodell részei.

   **Az (a)-t javaslom.** Kicsi, önállóan megnyitható, és az átadás a szerkesztőbe egy új
   paranccsal (pl. `cutter.sendToEditor`) képezhető le markerekre, subclipekre vagy idővonalra.
2. **A PostgreSQL legyen opcionális és későbbi.** Ha van File Services katalógus, a forrás és a
   kimenetek `files.*` hivatkozást kapnak. A Media Intelligence (`mi` séma) az F5 után adhat
   szegmensjavaslatot. A modulnak **PostgreSQL nélkül is teljes értékűen** kell működnie; ezt a terv
   5.4 pontja is kéri. Saját Media Cutter-sémára az F0–F4-ben nincs szükség.
3. **Időbélyegek:** a szegmenshatár `Tick`-ben, a forrás PTS-idővonalán tárolódik, ahogy az
   `export.plan` ma is kéri. A képkocka- és időkód-alak csak megjelenítés.

---

## 5. Integrációs pontok (meglévő → Media Cutter)

| Media Cutter funkció | Meglévő belépési pont |
|---|---|
| Fájl megnyitása, streamadatok | `mediaBrowser.probe`, `cfa4-cli probe`, Media Services `probe` (ADR 0007 MS-1) |
| Előnézet, léptetés | `frame.render`, `audio.render`, `serve_shm` (ADR F8 5. lépés; a dmabuf/GL-feltöltés még nyitott, parity 5.5) |
| Vágás tervezése és végrehajtása | `export.analyze`, `export.plan`, `export.start` (`scope: source`) |
| Kimenet ellenőrzése | beépített `verify_*`, `qc.run` / `cfa4-cli qc` |
| Jelölők, fejezetek | `markers.*`, `MarkerKind::Chapter`, CHAPTERS-DASH terv |
| EDL/CSV | `crates/interchange` (EDL, OTIO, FCPXML…), `markers.exportCsv` |
| Jelenet / fekete / csend | `clip.sceneEditDetection` (`cfa4-render::scene`), `cfa4-qc` picture/audio |
| Hosszú feladatok | `jobs.list`, `jobs.cancel`, `export.queue.*` |
| Fájlhivatkozás, katalógus (később) | `files.*` (ADR 0009, FS-1) |
| Media Intelligence (F5 után) | `mi` séma (TASKSTORE-PG-001), Event Fabric (EF-R1) |

**Hiányzó motorparancsok (javaslat, F1–F4):** `cutter.session.*` (megnyitás, mentés, szegmens
CRUD), kötegelt `export.start` szegmenslistára, forrásfájlra futó jelenet-, fekete- és
csendelemzés, amelynek eredménye szegmens lesz és nem idővonal-vágás, streamválasztás és
-kinyerés, Matroska Smart Cut út, metaadat-szerkesztés besorolással, megszakítható Smart Cut.

---

## 6. Csomagolás és együttélés a LosslessCuttal (terv 12.)

**Mai csomagolás** (`packaging/linux`):

- Formátumok: `.deb` (nFPM), tar.gz, AppImage, Flatpak.
- Alkalmazásazonosító: `io.github.ubuntuokos.cfa4`.
- FFmpeg 9.0.2: a `.deb`-ben `/opt/cfa4/ffmpeg-9.0.2`, a hordozható formátumokban
  `lib/cfa4/ffmpeg-9.0.2`. A korlátozott kodekek (x264/x265) a külön `cfa4-restricted-extras`
  `.deb`-ben vannak.
- **Fontos blokkoló:** az ADR F8 7. lépése szerint a `.deb` és a tar.gz hozza a Qt-felületet
  (az Ubuntu PySide6-jával), **az AppImage és a Flatpak még nem**, és az asztali bejegyzés még
  az egui-t indítja (parity 1.10). Qt6-alapú Media Cutter így ma csak `.deb`/tar.gz formában
  szállítható. Ezt a CFA4-QT6-FINISH-001 terv oldja meg, nem a Media Cutter.
- A terjesztést a márka- és licencellenőrzés kapuja tartja vissza (`CFA4_ALLOW_LOCAL_PACKAGING`).

**Ütközésmentesség a LosslessCuttal:**

| Terület | CFA4 | LosslessCut (ismert azonosítók; az F0-ban nem ellenőriztem élőben) | Ütközés? |
|---|---|---|---|
| Program, csomagnév | `cfa4`, `cfa4-qt`, `cfa4-cli` | `losslesscut` (Snap, AppImage, tar), Flathub: `no.mifi.losslesscut` | nincs, ha a Media Cutter nem kap `losslesscut`-szerű nevet |
| Indító, ikon | `io.github.ubuntuokos.cfa4*.desktop` | saját | nincs |
| Konfiguráció, gyorsítótár | `~/.config/cfa4`, CFA4 adatkönyvtár | `~/.config/LosslessCut` (Electron userData) | nincs |
| MIME | `application/x-cfa4*`; a fő bejegyzés a `video/*` típusokat is felsorolja | `video/*` | **társítás közös, de nem ütközés.** A Media Cutter ne tegye magát csendben alapértelmezetté |
| Portok, szolgáltatások | `serve` csak csövön; `--control` opcionális loopback | nincs | nincs |
| Ideiglenes fájlok | atomikus író a célmappában | saját | nincs |

**Javaslat:** a CFA4 a LosslessCut nevét, ikonját és kifejezéseit ne használja a termékben
(márkafüggetlenség, `test_independence.py` mintájára). A dokumentációban referenciaként
megemlíthető.

---

## 7. Licenc és függőségek (terv 8.)

- **CFA4:** GPL-3.0-or-later (`LICENSE`, `docs/licensing.md`).
- **LosslessCut:** a terv szerint GPL-2.0-only. A `docs/licensing.md` §3 és az AGENTS.md szerint
  GPL-2.0-only kód **nem vehető át**: vagy újra kell valósítani, vagy külső programként hívható.
  A terv 8. pontja ezzel összhangban van. Kiegészítő javaslatok:
  - A LosslessCut forrását (kód, UI-szövegek, ikonok, gyorsbillentyű-táblák) a fejlesztők ne
    olvassák implementáció közben. Funkcionális referencia a nyilvános dokumentáció és a program
    viselkedése legyen (AGENTS.md §2 clean-room).
  - Ha később kell a LosslessCut projektfájljának importja, az **adatformátum-interoperabilitás**,
    nem kódátvétel. Ezt a minta-fájlokból és a dokumentációból kell levezetni, és jogilag külön
    ellenőrizni. Nem része az F1-nek.
  - A licenc pontos megjelölését a jogi ellenőrzés a LosslessCut repó aktuális `LICENSE` és
    `package.json` fájljából erősítse meg.
- **FFmpeg:** saját 9.0.2-build, `--enable-gpl --enable-version3`, **soha `--enable-nonfree`**,
  `--disable-autodetect`. Opcionális könyvtárak: VA-API, libdrm, nv-codec-headers (futáskor
  töltődik be), oneVPL, dav1d, libvpx, Opus, libwebp, LAME, Vulkan
  (`scripts/dev/build-ffmpeg-9.0.2.sh`). A csomagolás a betöltött könyvtárak licenceit is
  hozza (`share/doc/ffmpeg/BUILDINFO`). FFmpeg-forráskódot nem másolunk; a Rust-kötés
  (rsmedia → rsmpeg → rusty_ffmpeg) rögzített commitra van vendorozva, és a `deny.toml` csak ezt a
  két git-forrást engedi. **A Media Cutter F1–F4-hez új FFmpeg-opció nem kell.**
- **Új függőség:** az F1–F4 a meglévő crate-ekre épül; új külső Rust- vagy Python-függőség nem
  szükséges. Ha mégis kell, `cargo deny check licenses` és a `docs/licensing.md` §6 ellenőrzése
  kötelező.
- **Szabadalom:** a H.264/HEVC **határ-újrakódolás** kódolót igényel. Ma az x264/x265 csak a
  `cfa4-restricted-extras` csomagban van, a HEVC kódolás pedig csak NVENC-cel (gyártófüggő, ezért
  a SCE-001 7. döntése szerint R1-ben nem alap). A tisztán stream-copy vágás kódolót nem igényel.
  A felületnek jeleznie kell, ha a pontos vágáshoz szükséges kódoló nincs telepítve.
- **Nincs fizetős rész** (2026-11-08): a Media Cutterben sem lehet korlát, vízjel vagy kulcs.

---

## 8. Javasolt módosítások a követelménylistához (az F0 lezárásához)

1. **2. és 5.1:** „FFmpeg-alapú” helyett „a CFA4 Rust-médiamotorja, az ADR 0005 szerinti
   kiegészítő FFmpeg- és GStreamer-háttérrel”. Törölni kell az „FFmpeg-feladatok
   indítása/felügyelete” sort; helyette „motorfeladatok (jobs) indítása és felügyelete”.
2. **5.2–5.3:** „Qt6 (PySide6) felület ↔ `cfa4-cli serve` ↔ Rust-motor”; minden művelet
   engine-parancs (CLI, MCP, vezérlőcsatorna).
3. **3.1:** a támogatott kombinációk listáját a meglévő registry adja (`Registry::report()`), ne a
   terv rögzítse. Az F1 minimuma: H.264/HEVC/AV1 MP4/MOV, MPEG-2 PS/TS GOP-másolással, AAC/PCM
   hanggal. **MKV-forrás az F1-ben ne legyen kötelező**, vagy kerüljön F2-be.
4. **3.2 és 5.4:** a munkamenet fájlba kerül (javaslat: `.cfa4cut`), a PostgreSQL opcionális index.
5. **3.5–3.7:** a meglévő elemzők forrásfájlra futó változata. Az eredmény szegmens/jelölő
   javaslat, amelyet a felhasználó elfogad; a forrást soha nem változtatja.
6. **3.3–3.4 (F2):** a SCE-001 P2/P2a/P3 fázisával egyesítendő; külön remux-motor nem készül.
7. **6.1:** `cfa4-qt --cutter` indítási mód és saját `.desktop` bejegyzés (VISION 7).
8. **10. (tesztek):** a terv tesztmátrixa egészüljön ki a WP-09 jelentés nyitott pontjaival
   (eltérő timescale, VFR, HDR, több hangsáv nem AAC/PCM kodekkel, megszakítás), és az
   FFmpeg-orákulumos teszteknél a `cfa4_testkit::ffmpeg_or_skip` mintát kövesse. A fixture-ök a
   `target/fixtures/` alá generálódnak, médiát nem commitolunk.
9. **12.:** az AppImage/Flatpak Qt-támogatása a CFA4-QT6-FINISH-001 függősége, nem a Media
   Cutteré.

---

## 9. Tulajdonosi döntést igénylő kérdések

| # | Kérdés | Javaslat |
|---|---|---|
| D1 | Indulhat-e a Media Cutter F1 a Qt6-paritás (59/63) lezárása előtt, amikor a SCE-001 P1–P7 erre vár? | Az F1 UI-része igen, mert csak a meglévő `export.plan/start`-ot használja. Az F2 (remux) a SCE-001 P2–P3-mal együtt, annak ütemezésében |
| D2 | Elfogadható-e az 5.1 átfogalmazása (Rust-motor az alap, FFmpeg csak útválasztón át)? | Igen; ez az ADR 0005 és a 2026-10-27-i döntés |
| D3 | Kapjon-e a stream copy/remux FFmpeg-hátteret (például MKV), amíg nincs natív út? | Nem az F1-ben; ha kell, az F2 előtt külön döntés a SCE-001 hatókör-kiegészítésével |
| D4 | Munkamenet: saját `.cfa4cut` fájl vagy `.fcproj`? | Saját `.cfa4cut`, és átadás a szerkesztőbe paranccsal |
| D5 | Indítás: `cfa4-qt --cutter` vagy saját programfájl? | `cfa4-qt --cutter` és saját `.desktop` bejegyzés |
| D6 | Végleges név: „CFA4 Media Cutter” / „CFA4 Médiavágó”? | A terv szerint külön döntés |
| D7 | Az átfedés jelzése a merge koordinátornak (SCE-001 / SMART-CUT-RENDER-R1) | Az F1 indítása előtt kötelező (CLAUDE.md 3., `docs/coordination.md` §2.8) |

---

## 10. Felhasznált források a repóban

`VISION.md` (2.1, 2.7) · `AGENTS.md` (CFA4 szakasz, §0, §2) · `CLAUDE.md` ·
`docs/adr/ADR-F8-QT6-RUST-ENGINE-2026-10-23.md` · `ADR-0005-media-backends.md` ·
`ADR-0006-backlothub-postgresql.md` · `ADR-0007-media-services.md` · `ADR-0009-cfa4-file-services.md` ·
`docs/plans/CFA4-SCE-001-STREAM-COPY-EXPORT-2026-10-27.md` · `GOP-SMART-CUT-2026-10-27.md` ·
`CFA4-SMART-CUT-RENDER-R1.md` · `CFA4-FFMPEG-BACKEND-R1.md` · `CHAPTERS-DASH-2026-10-30.md` ·
`CFA4-TASKSTORE-PG-001.md` · `SUBTITLE-STUDIO-PLAN-001-FINAL-1.2.md` ·
`docs/audit/SMART-CUT-RENDER-WP09-VALIDATION-2026-11-10.md` · `docs/media-qc.md` · `docs/project-files.md` ·
`docs/licensing.md` · `docs/roadmap.md` · `crates/smart-edit/README.md` · `crates/ffmpeg/README.md` ·
`crates/engine/src/smart_cut.rs`, `scene_detect.rs` · `crates/export/src/audio_copy.rs` ·
`crates/qc/src/{picture,audio}.rs` · `python/cfa4_qt/panels/{smart_cut,smart_export}.py`, `tools_menu.py`,
`__main__.py` · `packaging/linux/*` · `scripts/dev/build-ffmpeg-9.0.2.sh`

---

## A. melléklet – átfedésjelzés a gazdának (D7)

> **Átfedés: CFA4-LOSSLESS-MEDIA-TOOLS-001 (Media Cutter) ↔ CFA4-SCE-001 / CFA4-SMART-CUT-RENDER-R1**
>
> A Media Cutter F0-ja (2026-11-13, a tulajdonos jóváhagyásával) a következőt rögzítette:
>
> 1. Külön alkalmazás nem készül: a CFA4-ből hiányzó képességek (jelentés 0. fejezet, K1–K7)
>    kerülnek a meglévő Qt6-felületbe és motorparancsokba. Ilyen a több tartomány vágása a
>    forrásmonitorból, a szegmensek mint tartományjelölők a `.fcproj`-ban, a jelenet-, fekete- és
>    csendfelismerés jelölőkként, a CSV-jelölőimport és a képkocka-export bővítése. Ezek a
>    meglévő `export.analyze/plan/start` (`scope: source`) parancsokra épülnek, új vágómotort nem
>    készítenek, és a Qt6-paritás előtt indulhatnak.
>    A K8–K11 (megszakítás, MKV Smart Cut, streamkezelés, metaadat) a SCE-001 / R1 területe.
> 2. Az F2 (remux, streamkiválasztás és -kinyerés) **nem készül külön**: a SCE-001 P2/P2a/P3
>    fázisával egyesül, annak ütemezésében (a Qt6-kapu után).
> 3. A stream copy útjaira FFmpeg-háttér nem kerül; az ADR 0005 és a 2026-10-27-i döntés
>    változatlan.
>
> Kérés: a gazda rögzítse ezt egy sorban a `docs/coordination-log.md`-ben, és jelezze a SCE-001 /
> SMART-CUT-RENDER-R1 munkameneteinek. A Media Cutter munkamenetek a `crates/smart-edit`, a
> `crates/export/src/smart_cut*` és az `audio_copy` kódjához csak a SCE-001 projekt jóváhagyásával
> nyúlnak.
