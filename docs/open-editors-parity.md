# Hét nyílt vágóprogram: mit vehet át belőlük a CFA4

> **Állapot:** a tulajdonos 2026-10-27-én döntött (6. pont) · **Készült:** 2026-10-27 ·
> **CFA4:** `main` @ `16debf9` · **Kapcsolódik:** [`kdenlive-parity.md`](kdenlive-parity.md)
> (K1–K35, PR #325), [`venturi-parity.md`](venturi-parity.md) (V1–V10, PR #326),
> [`gaps.md`](gaps.md) (G-számok), [`licensing.md`](licensing.md), [ADR 0004](adr/ADR-0004-compute-routing.md)

Ez a dokumentum a Kdenlive- és a Venturi-terv mintájára hét nyílt forrású vágóprogramot vet össze
a CFA4-gyel. A két korábbi tervben vagy a CFA4-ben már meglévő tételeket **nem ismétli**, csak
hivatkozik rájuk. Az új teendők **E1–E15** számot kapnak.

## 0. Röviden

- A hét programból **egy igazán erős** forrás van: a **Kinewright** (Rust, GPL-3.0-only, ~530 ezer
  sor). Az erőssége nem a vágás, hanem az, ahogyan **egy MI-ügynök biztonságosan szerkeszt**:
  atomikus, revízióhoz kötött szerkesztési terv, ügynökönként külön ág, mérhető ügynöki
  teljesítmény, és hibák helyett típusos „incidensek” biztonságos javítással.
- A **Cutlass** (MIT/Apache-2.0) ugyanezt az elvet egyszerűbben mutatja (próba homokozóban,
  utána egy visszavonási lépésben élesben), plusz Lottie-animációk tiszta Rustban és Python-API.
- A **Concat** AGPL-3.0, ezért **kódja nem vehető át**. Az ötletei igen: modelljegyzék rögzített
  SHA-256-tal és saját tükörrel, valamint a modelljelöltek listája (háttérkivágás, felskálázás).
- A **Gausian** (Apache-2.0) automatikus proxydöntése és SQLite-sémája átvehető minta.
- Az **opencut** (GPL-3.0-only) a projektformátum géppel olvasható sémáját és CLI-ellenőrzését adja.
- A **Lumina** még csak váz, a **RueyMediaEditor** pedig a CFA4-ben már meglévő funkciókat tud.
  Ezekből nincs teendő.
- **Alapszabály (tulajdonosi döntés, 2026-10-27):** minden MI-funkció, minden CFA4-alkalmazásban,
  csak az **MI-réteg üzembe helyezése után** készül (a rétegről külön terv készül). Ebben a tervben
  MI-funkció: E3, E7, E8, E9, az E5 élő része és a külső generatív oda-vissza.
- Javasolt sorrend: most **E1 és E5** (szkriptelt rész, modell nélkül), utána a nem MI-s P2-k;
  az **E7, E3, E8, E9** az MI-réteg után. A négy döntés a 6. pontban.

## 1. A hét forrás

| Program | Repó (commit) | Nyelv, felület | Licenc | Állapot |
|---|---|---|---|---|
| Cutlass | `1mrnewton/cutlass` (`22437e2`) | Rust, Slint, wgpu; iOS/Android-alkalmazás | MIT OR Apache-2.0 | alfa; Linuxon nincs médiaháttér |
| Kinewright | `CanadaApollo6/Kinewright` (`b2b8630`) | Rust, egui, wgpu, FFmpeg | **GPL-3.0-only** | korai, de nagyon kiterjedt |
| Concat | `jub0t/Concat` (`bfb39ca`) | Rust, Slint, wgpu; Android | **AGPL-3.0-or-later** | v0.2.8, kiadott |
| Gausian Native Editor | `gausian-AI/Gausian_native_editor` (`2e173a0`) | Rust, egui, wgpu, GStreamer | Apache-2.0 | prototípus |
| Lumina | `AnimatedGTVR/Lumina` (`8ec48f6`) | Rust mag + Qt/QML C ABI-n át | MIT OR Apache-2.0 (csak a `Cargo.toml`-ban, `LICENSE` fájl nincs) | váz |
| RueyMediaEditor | `git.mitgai.net/rueyday/RueyMediaEditor` | Rust (Tauri) + HTML/JS, FFmpeg külön folyamat | MIT | 87 commit |
| OpenCut | `waterblower/opencut` (`3254727`) | Rust, GPUI | **GPL-3.0-only** | prototípus |

**A Gitea-tárhely (`git.mitgai.net`) git klónozással nem érhető el** ebből a környezetből (a proxy
403-at ad). A nyilvános weboldala viszont olvasható volt, ezért a RueyMediaEditort a README alapján
elemeztem, a forráskódot nem láttam, és commitot sem tudok megadni.

## 2. Licenc: mit lehet átvenni

A [`licensing.md` §3](licensing.md) szabályai szerint:

| Forrás | Kód átvehető? | Megjegyzés |
|---|:-:|---|
| Kinewright, OpenCut (GPL-3.0-only) | ✅ | A §3 megengedi. Fájlonkénti fejléc (eredet, szerző, GPL-3.0-only) és §5-bejegyzés kell. A GPL-3.0-only kód a CFA4 terjesztett egészét gyakorlatilag GPL-3.0-only-vá teszi, ezért **elsősorban az elvet írjuk újra** (lásd 6. pont, 4. kérdés) |
| Cutlass, Gausian, Lumina, Ruey (MIT / Apache-2.0) | ✅ | A szerzői megjegyzést és az Apache NOTICE-t meg kell tartani |
| Concat (AGPL-3.0) | ❌ | Csak ötlet és nyilvános tény (pl. hogy melyik modell milyen licencű). A kódját **nem olvassuk át mintának**, a megoldást magunk írjuk |
| Lumina | 🟡 | A licenc csak a `Cargo.toml`-ban van, `LICENSE` fájl nincs. Átvétel előtt kérdezzük meg a szerzőt (de nincs is mit átvenni) |
| Nevek, logók, ikonok, matricák (Cutlass `assets/stickers`) | ❌ | Márka vagy külön licencű eszköz, nem vesszük át |

**Átvételi szabályok (ugyanazok, mint a Venturinál):** a forrásokban rengeteg `unwrap`/`expect`
van (Kinewright ~18 600, Cutlass + Gausian ~4 100), és képkockában vagy f64 másodpercben számolnak.
Átvételkor: `Result` + `?`, `get()`, telített aritmetika, minden idő `cfa4_time::Tick`,
`unsafe` csak `crates/platform`-ban, és L5 alatt nincs egui/cpal-függés (`cargo xtask layers`).

## 3. Ami a CFA4-ben vagy a korábbi tervekben már megvan (nincs teendő)

| Képesség (forrás) | Hol van már |
|---|---|
| MCP-szerver ügynököknek (Concat, Kinewright) | `crates/automation`, `cfa4-cli mcp`, V3 (ügynökbiztos hoszt) |
| Átirat alapú vágás, töltelékszavak, szünetek (Kinewright, Concat) | `transcript.*`, K31–K35, V1 (VAD) |
| Helyi Whisper, modellméret választható (Concat, Ruey) | `crates/speech`, K2/G19, K31 |
| Felolvasás Kokoro-val (Concat) | `crates/tts` (#278) |
| Háttéreltávolítás (Concat) | K4 (SAM2-szerű); a modelljelöltek az E7-ben |
| Zajcsökkentés, zümmögés- és kattanásszűrő, teremzaj (Cutlass RNNoise, Kinewright AU5) | `audio-dsp` (de-click, de-hum), Essential Sound, room tone |
| R128, true peak, busz-EQ, kompresszor (Kinewright AU1–AU3) | saját DSP, `mixer.*` |
| Színkezelés, szkópok, CDL, LUT, másodlagos korrekció (Kinewright CC1–CC6) | `crates/color`, `crates/scopes`, Lumetri |
| FCPXML, FCP7 XML, EDL export (Gausian) | `crates/interchange` (1.11-ig) |
| Helyreállítási napló összeomlás után (Kinewright) | `autoSave.recoveryJournal` ([`project-files.md`](project-files.md#crash-recovery)) |
| Újralinkelés tartalom szerint (Kinewright SHA-256) | `crates/engine/src/relink.rs` (`MediaIdentity`) |
| Hang alapú szinkron, osztott képernyő (Ruey) | `engine/src/sync.rs`, multicam rács |
| SQLite-nyilvántartás (Gausian) | K17; a Gausian sémája (Apache-2.0) mintaként jó |
| Watch folder (Gausian ComfyUI-mappa) | K34 |
| Hálózati / fejetlen kliens (Kinewright AW1) | K14, `cfa4-cli serve`, V8 ötlet |
| 26 nyelvű felület (Concat) | i18n, G14 |
| Mobil (Cutlass, Concat Android), Windows/macOS | **nem cél**: a CFA4 csak Linux |
| Qt/QML a Rust magon C ABI-n át (Lumina) | **elvetve**: a CFA4 Qt6 PySide6 + CLI-híd (ADR F8), Qt Widgets idővonal |
| Tauri + HTML felület, előnézet FFmpeg-gel (Ruey) | nem illik a CFA4-be |

## 4. Új teendők

### E1 · Atomikus, revízióhoz kötött szerkesztési terv az ügynököknek · P1 · 25–40 óra

**Mit tudnak:** a Kinewright (M36) ügynöke nem parancsonként módosít, hanem **tervet** ad be:
`prepare_edit_plan` → a teljes terv érvényesítése élesítés előtt → `commit_edit_plan` egy
visszavonási lépésként, vagy `discard_edit_plan`. A terv **ahhoz a revízióhoz kötött**, amelyet az
ügynök látott; ha közben a felhasználó módosított, a commit egészében elutasul (M30). A Cutlass
ugyanezt homokozóval csinálja: a promptot a projekt másolatán próbálja ki, és csak az érvényes
tervet játssza vissza élesben, **egy Ctrl+Z-vel visszavonhatóan**, előnézeti kártyával
(Alkalmaz / Elvet).

**CFA4 ma:** a `command_batch` (`crates/automation/src/server.rs`) lépésenként futtat, hiba esetén
megáll, de **a már lefutott lépések bent maradnak**, és nincs egyetlen visszavonási csoport. A
dokumentumnak van `revision` száma (`engine/src/lib.rs`), de a parancsok nem hivatkozhatnak rá.

**Teendő:**
1. `engine`: `plan.prepare {steps, baseRevision}` → a lépések egy másolaton (snapshot) futnak, az
   eredmény: érvényes-e, a változások összefoglalója, a „romboló” lépések listája.
2. `plan.commit {planId}` → ha a `revision` még egyezik, a lépések egy visszavonási csoportként
   futnak élesben; ha nem, `staleRevision` hiba, semmi sem változik. `plan.discard`.
3. A `command_batch` kap egy `atomic: true` módot, amely ugyanezt használja (visszafelé kompatibilis).
4. Romboló tervhez (törlés, ripple) a felület jóváhagyást kér (Qt6 kártya, ADR F8).
5. Regressziós tesztek: hibás 3. lépés → a dokumentum bájtra azonos; elavult revízió → elutasítás;
   egy Ctrl+Z az egész tervet visszavonja.

Ez a V3 (visszatartás mozdulat alatt) és a V10 (stabil azonosítók) természetes folytatása.

### E2 · Ügynökönként külön idővonal-ág, összefésülés egy lépésben · P2 · 40–70 óra

**Mit tud a Kinewright (M31):** minden ügynöki szál a projekt egy **ágán** dolgozik (pontos
revízióból indul). Az élő projekt csak akkor változik, ha a felhasználó az ágat összefésüli (egy
visszavonási lépés) vagy egyes műveleteit átveszi (cherry-pick). Elavult alapból nem ír felül.
Az ág megőrzi, mit látott és mit javasolt az ügynök.

**Teendő:** az E1 után. `branch.create/merge/cherryPick/discard` parancsok, az ág a projektfájlon
kívül (mint a helyreállítási napló), a Qt6-ban egy összehasonlító nézet. Hosszú, több lépéses
MI-munkához (K31–K35 automatikus vágások) ez adja a biztonságot.

### E3 · Beépített MI-asszisztens panel a felhasználó saját ügynökével · P2 · 40–70 óra · **jóváhagyva, MI-réteg után**

**Mit tudnak:** a Kinewright a gépen **már telepített ügynök-CLI-t** indítja el (Claude Code, Codex,
OpenCode, Qwen Code stb.) a saját helyi MCP-végpontjára kötve; kulcsot nem kér és nem tárol. A Cutlass
**OpenAI-kompatibilis végpontot** hív (Ollama, LM Studio, llama.cpp-server), modellt nem szállít,
a kulcs a felhasználói beállításban vagy környezeti változóban van, a projektfájlba sosem kerül.

**CFA4 ma:** MCP van, de a felhasználónak magának kell egy ügynököt kívülről rákötnie. Panel nincs.

**Javaslat:** Qt6 „Asszisztens” panel (dokkolható, mint a többi), amely
1. **alapból helyi** OpenAI-kompatibilis végpontot használ (Ollama / llama.cpp-server), vagy
2. kifejezett bekapcsolás után egy telepített ügynök-CLI-t indít, amely a CFA4 MCP-jére csatlakozik.

Minden módosítás az E1 tervén megy át (előnézet, egy visszavonási lépés). A felhős ügynök a projekt
adatait a gépen kívülre küldheti, ezért ugyanaz a szabály, mint a V7-nél: felhős csak kifejezett
bekapcsolással. **Döntés:** a panel kell; az MI-réteg üzembe helyezése után készül (6. pont).

### E4 · Típusos incidensek: hibaüzenet helyett biztonságos javítás · P2 · 25–40 óra

**Mit tud a Kinewright (IN1–IN2):** egy technikai zsákutca (ismeretlen színleírás, médiahiba,
exporthiba) nem szöveges hibaüzenet, hanem **incidenskártya**: kód, szabályosztály (`AutoApply` =
magától alkalmazható, `AskFirst` = kérdez), és a biztonságos javítás **látható, visszavonható
parancsként** fut. Az ügynök is látja (`get_incidents`), és javaslatot tehet, amely csak
jóváhagyással él. A nyitott incidensek a projekttel együtt mentődnek.

**CFA4-ben:** az AGENTS.md §0 („soha ne omoljon össze”) után ez a következő lépés: a hiba ne csak
ne vigye el a munkát, hanem legyen belőle kiút. **Teendő:** `IncidentCode` enum az első
családdal (forrásszín-leírás hiányzik vagy ellentmondásos; offline média; hardveres dekóder menet
közbeni bukása, amelyet a `hardware.fallbacks` már számol), `incidents.list` / `incidents.resolve`
parancs, Qt6-kártya. Később a V2 (kódoló-önteszt bukása) és a QC-sorozat (#255…) is ide fut be.

### E5 · Mérhető ügynöki teljesítmény: MI-tesztsor a CI-ban · P1 · 15–25 óra

**Mit tudnak:** a Cutlass szkriptelt „szolgáltatóval” (hálózat és modell nélkül) futtat promptokat
mintaprojekteken, és a végső idővonalat és a műveletnaplót ellenőrzi. A Kinewright `kinewright-eval`
pontozott sora (pontos művelet-alapvonal, kész vágás renderelve és mérve) csak kifejezett
`KINEWRIGHT_EVAL=1` mellett fut, élő modellel; a CI csak a keretrendszert teszteli.

**Teendő:**
1. `crates/testkit`: `ScriptedAgent` (rögzített MCP-hívássorozat) és mintaprojektek a
   `target/fixtures/`-ben generálva.
2. Esetek: „vágd le az első 3 másodpercet”, „töröld a szüneteket”, „tegyél feliratot” stb.; az
   ellenőrzés a végső dokumentumon és az E1-tervek naplóján történik.
3. Opcionális élő futás (`CFA4_EVAL=1`, helyi modellel), JSONL-eredmény a `target/evals/`-be,
   a CI-ban sosem. **Ez a rész MI-funkció: csak az MI-réteg után.** Az 1–2. pont modell nélküli, most mehet.
4. A K33 magyar WER-mérése ugyanebbe a keretbe kerül.

### E6 · Kicsi, állandó MCP-felület, sémák igény szerint · P2 · 8–15 óra

**Mit tud a Kinewright (M36):** az ügynök **hét** állandó eszközt lát; a több száz képességből
névvel keres, és csak a kiválasztott sémáját tölti be. Ez minden körben sok tokent spórol, és
mérik (bájtra rögzített méret, regressziós teszt).

**CFA4 ma:** a `command_list` szűrhető, de minden találatot teljes adattal ad vissza. **Teendő:**
`command_describe {id}` (egy parancs teljes sémája), a `command_list` alapból csak id + címke,
és teszt, amely rögzíti az MCP-eszközlista szerializált méretét (ne nőjön észrevétlenül).

### E7 · Modelljegyzék rögzített SHA-256-tal és saját tükörrel · P1 az MI-rétegen belül · 10–20 óra

**Ötlet (Concat, csak az elv):** minden letölthető modell egy jegyzékben szerepel: fájlnév, méret,
**SHA-256**, licenc, forrás. A Hugging Face-forrás **commitra rögzítve**, nem `main`-re; a program
először a projekt saját kiadási tükréből tölt, és csak utána a forrásból; **ellenőrzőösszeg nélküli
modellt nem tölt le**. A tükröt egy CI-munka frissíti.

**CFA4-ben:** a K18 (letöltéskezelő) és a K31 erre a jegyzékre épüljön: `models/manifest.toml`
(saját formátum), `cargo xtask models --check`, a licenc a felületen látszik (K8/K6 döntés:
nem kereskedelmi licencre figyelmeztet). **A Concat kódját nem nézzük**, a formátumot magunk írjuk.
Az E7 az MI-réteg része: annak tervébe kerül bemenetként, és vele együtt készül.

**Modelljelöltek** (a Concat jegyzékéből vett tények, mind a forrásnál ellenőrizendő):

| Feladat | Modell | Licenc | Hova |
|---|---|---|---|
| Ember kivágása videón | RobustVideoMatting MobileNetV3 | GPL-3.0 | K4 |
| Tárgy kivágása | ISNet general-use | Apache-2.0 | K4 |
| Ecsetes kijelölés | SlimSAM-77 (encoder + decoder) | Apache-2.0 | K4 |
| Felskálázás | Real-ESRGAN general x4v3 | BSD-3-Clause | E8 |
| Felolvasás | Kokoro multi-lang v1.0 (int8 is) | Apache-2.0 | megvan (#278) |
| Felolvasás | Pocket-TTS | CC-BY-4.0 | csak forrásmegjelöléssel |
| Hangklónozás | Chatterbox Turbo | MIT | E9, **döntés** |

### E8 · MI-felskálázás (Real-ESRGAN) · P3 · 20–35 óra · MI-réteg után

A G12 MI-listáján nincs felskálázás. Régi vagy kis felbontású anyaghoz hasznos. ONNX-modell
`tract`-tal (mint a V1), a `cfa4-route` szerint CPU-n vagy GPU-n (ADR 0004: kötelező CPU-tartalék),
háttérmunkaként, eredmény új médiafájlként (nem valós idejű effekt).

### E9 · Hangklónozás · P3 · **jóváhagyva, MI-réteg után; most csak előkészítés**

A Concat néhány másodperc felvételből bármilyen hangot utánoz (Chatterbox, MIT). Technikailag a
Kokoro mellé illene, de visszaélésre alkalmas (más hangjának utánzása).

**Döntés:** a hangklónozás az MI-réteg elkészülte után jön, de **legyen előkészítve**. Előkészítés
most (modell nélkül, 4–8 óra):
1. a `crates/tts` felolvasó-felületén a „hang” ne csak beépített hangazonosító lehessen, hanem
   hivatkozás egy **hangprofilra** (mintafájl(ok) + metaadat), a projektfájlban opcionális mezőként;
2. a hangprofil kötelező mezői: a beszélő neve, a **hozzájárulás** rögzítése (ki, mikor, mire),
   és a kimenet jelölése (metaadat a generált hangsávon);
3. a modelljegyzékben (E7) a `voice-clone` család helye, licencmezővel; a modell maga az MI-réteggel jön.

### E10 · Lottie-animációk (matricák, overlay-ek) tiszta Rustban · P2 · 20–35 óra

**Mit tud a Cutlass:** Lottie-animációt (`.json`, `.lottie`) a **velato + vello_cpu** (linebender,
MIT/Apache-2.0, tiszta Rust) rajzol, C++ nélkül. Lottie-szöveget szándékosan nem támogat.

**CFA4 ma:** Lottie nincs (`crates/graphics` saját grafikák, `.mogrt`-szerű sablonok). **Teendő:**
Lottie-import grafikus klipként, képkockánként renderelve `Tick`-időre, gyorsítótárral; `cargo deny`
ellenőrzés; tesztanimáció saját készítésű vagy CC0 forrásból `.attribution` fájllal.

### E11 · Python-szkriptelés a vezérlőcsatornán át · P2 · 15–25 óra

**Mit tud a Cutlass:** `cutlass-py` (PyO3, PyPI-kerék): `Project`, `Track`, `Clip`, `get_frame()` →
numpy, `export()`.

**CFA4-ben** a PyO3 FFI-t és `unsafe`-et hozna, ezért **nem így**: egy tiszta Python csomag
(`cfa4` a PyPI-n vagy a csomagban), amely a `cfa4-cli serve` JSON-sorain beszél (ugyanaz a híd,
mint a Qt6-felületé, ADR F8). `get_frame()` a `render_frame` PNG-jét adja numpy tömbként. Így
kötegelt munkákhoz és tudományos/oktatási használathoz kényelmes, és nincs második kódút.

### E12 · Automatikus proxydöntés · P2 · 8–15 óra

**Mit tud a Gausian** (`proxy_policy.rs`, Apache-2.0): a proxy szükségességét a médiából és a
hardverből dönti el: felbontás (≥ 2160p), képkockasebesség (≥ 60 fps 1080p felett), interframe
kodek nagy bitrátával (HEVC/AV1/VP9 ≥ 25 Mb/s), 10 bites HDR HEVC, VFR, több videósáv, hosszú
interframe anyag; intra kodekhez (ProRes, DNx) nem kér proxyt.

**Teendő:** `media.proxyAdvice` a `cfa4-route` adataival (van-e hardveres dekóder az adott kodekre),
importkor javaslat (nem automatikus létrehozás, csak beállítással), a K12 kameraproxyk előnyben.

### E13 · A projektformátum JSON-sémája és CLI-ellenőrzés · P3 · 6–10 óra

**Mit tud az OpenCut:** `cli schema --json` kiírja az idővonalformátum sémáját, `cli validate`
ellenőriz egy fájlt; az írás `.tmp` + fsync + átnevezés.

**CFA4-ben** az atomikus írás megvan ([`project-files.md`](project-files.md)). **Teendő:**
`cfa4-cli project schema` (a `.fcproj` sémája, `schemars`) és `cfa4-cli project validate`, hogy
ügynökök és külső szkriptek a megnyitás előtt ellenőrizhessenek. Kiegészíti a K1 `.kdenlive`
olvasó jelentését.

### E14 · Export utáni hang- és képellenőrzés · P2 · 8–15 óra

**Mit tud a Kinewright (AU3, CC6):** export után **visszadekódolja a megírt fájlt**, és megméri a
hangosságot és a true peaket a célprofilhoz képest, plusz a tartományt, színteret, bőrtónust.

**CFA4-ben** a nyitott QC-sorozatba (#255, #302, #305, #306, #317, #319) új `cfa4-qc`-szabályként:
„a kész fájl mért hangossága ±1 LU a célon belül”. A K15 export utáni lépései közé illeszkedik.

### E15 · Storyboard és ütemelemzés az ügynöknek · P2 · 15–25 óra

**Mit tud a Kinewright (M30):** az ügynök egy korlátozott méretű **storyboardot** kérhet (az
idővonal kontaktlapja ugyanabból a kompozitorból, mint az előnézet), és a zenéhez **ütem- és
hangindítás-elemzést** (gyorsítótárban, tartalom szerinti kulccsal, forrás- és projektidőre
leképezve).

**CFA4 ma:** `render_frame` egy képkockát ad. **Teendő:** `sequence.storyboard {range, count, size}`
(egy PNG-rács, korlátozott méret), és `audio.beats` (ütemhelyek `Tick`-ben, gyorsítótárban). A
zenére vágás (K-terv, Automate to Sequence) és az E5-tesztsor is használja.

### Ötletek, megvalósítás nélkül

- **Külső alkalmazás oda-vissza (Kinewright Blender-programja, Gausian ComfyUI):** az ügynök vagy
  a felhasználó egy külső programban (Blender, ComfyUI) készít elemet, a CFA4 a renderelt
  eredményt hivatkozott klipként importálja, és jelzi, ha a forrás megváltozott. Ez **generatív
  tartalom. **Döntés:** csak az MI-réteg után.
- **Effektkifejezés-nyelv shaderre fordítva (Lumina `luminafx`):** a Lumina csak a váz kész. A K7
  (matematikai kifejezés a számmezőkben) bővítéseként később érdekes lehet.
- **Kutatói elrendezések címkével (Ruey):** a multicam rács + szöveg már tudja; legfeljebb egy preset.

## 5. Rangsor és becslés

| # | Teendő | Prioritás | Óra | Függ |
|---|---|:-:|---:|---|
| E1 | Atomikus, revízióhoz kötött szerkesztési terv | P1 | 25–40 | V3, V10 |
| E5 | MI-tesztsor (szkriptelt ügynök a CI-ban) | P1 | 15–25 | E1 |
| E7 | Modelljegyzék SHA-256-tal, saját tükörrel | P1 | 10–20 | **MI-réteg** (K18 előtt) |
| E6 | Kicsi MCP-felület, sémák igény szerint | P2 | 8–15 | — |
| E12 | Automatikus proxydöntés | P2 | 8–15 | `cfa4-route` |
| E14 | Export utáni hang- és képellenőrzés | P2 | 8–15 | QC-sorozat |
| E15 | Storyboard és ütemelemzés | P2 | 15–25 | — |
| E4 | Típusos incidensek | P2 | 25–40 | Qt6 (ADR F8) |
| E11 | Python-szkriptelés a CLI-hídon | P2 | 15–25 | ADR F8 CLI-híd |
| E10 | Lottie-animációk | P2 | 20–35 | — |
| E2 | Ügynökönkénti ág | P2 | 40–70 | E1 |
| E3 | MI-asszisztens panel | P2 | 40–70 | E1, **MI-réteg** |
| E13 | Projektséma és `validate` | P3 | 6–10 | — |
| E8 | MI-felskálázás | P3 | 20–35 | E7, **MI-réteg** |
| E9 | Hangklónozás (most csak előkészítés) | P3 | 4–8 | modell: **MI-réteg** |

Összesen kb. **260–450 óra**. Ebből most indítható (MI-réteg nélkül) az E1, az E5 szkriptelt része
és a nem MI-s P2-k; az E3, E7, E8, E9 és az E5 élő része az MI-réteg után. A súlypont az **ügynöki szerkesztés
biztonsága** (E1–E6), mert itt a Kinewright és a Cutlass messze előrébb jár, mint a CFA4; a
vágás, hang és szín terén a CFA4 többet tud mindegyiknél.

## 6. Tulajdonosi döntések (2026-10-27)

1. **E3 MI-asszisztens panel:** kell. Alapból helyi végpont; felhős ügynök csak kifejezett bekapcsolással.
2. **E9 hangklónozás:** az MI-réteg elkészülte után, de most előkészítjük (hangprofil, hozzájárulás, jelölés).
3. **ComfyUI és Blender oda-vissza:** csak az MI-réteg után.
4. **GPL-3.0-only kód (Kinewright, OpenCut):** az elveket írjuk újra a CFA4 szabályai szerint, szó szerinti átvétel nincs.
5. **Általános szabály:** minden MI-funkció, minden CFA4-alkalmazásban, csak az MI-réteg üzembe
   helyezése után készül. Az MI-rétegről külön terv készül; ez vonatkozik a Kdenlive (K) és a Venturi (V)
   terv MI-tételeire is.

A megvalósítás a meglévő szabály szerint csak a tulajdonos kérésére indul, teendőnként.

## 7. Hogyan frissítsd ezt a fájlt

Új verziónál nézd át a README-k funkciólistáját, a Kinewright `docs/` programdokumentumait (M30,
M31, M36, IN1, AW, BL) és a Cutlass `ROADMAP.md`, `docs/ai-agent-roadmap.md` fájlját; frissítsd a
commitokat az 1. pontban. Minden átvett kódrészhez sor a [`licensing.md` §5](licensing.md)
nyilvántartásban. A Concat kódját továbbra se nézd (AGPL).

## Források

- Kinewright: `README.md`, `docs/agent-harnesses.md`, `M30`, `M31`, `M36`, `IN1`, `EVALS.md`,
  `AU5`, `BL0`, `MEDIA-POLICY.md` @ `b2b8630`.
- Cutlass: `README.md`, `ROADMAP.md`, `docs/ai-agent-roadmap.md`, `docs/lottie-design.md` @ `22437e2`.
- Concat: `README.md`, `models/manifest.toml`, `docs/AUDIO_LIBRARY.md` @ `bfb39ca` (csak tények).
- Gausian: `README.md`, `apps/desktop/src/proxy_policy.rs`, SQL-migrációk @ `2e173a0`.
- Lumina: `README.md`, `docs/ARCHITECTURE.md` @ `8ec48f6`.
- OpenCut: `README.md`, `design/spec.md`, `rust/apps/cli/README.md` @ `3254727`.
- RueyMediaEditor: a `git.mitgai.net` weboldal README-je (git-klón nem volt elérhető).
- CFA4: `crates/automation/src/server.rs` (`command_batch`), `crates/engine/src/relink.rs`,
  [`project-files.md`](project-files.md), [`gaps.md`](gaps.md) G11, G12, [`control-protocol.md`](control-protocol.md).

## Változásnapló

- 2026-10-27: első változat (E1–E15).
- 2026-10-27: a tulajdonos döntött; minden MI-tétel az MI-réteg mögé került, E9 előkészítéssel.
