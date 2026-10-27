# Venturi-paritás: mit vehet át a CFA4 a Venturiból

> **Állapot:** a tulajdonos 2026-10-27-én jóváhagyta (V1–V10, a 5. pont javaslataival) · **Készült:** 2026-10-27 ·
> **Forrás:** [VenturiVideo/Venturi](https://github.com/VenturiVideo/Venturi) `main` @ `cdc1c0f`
> (2026-10-26, v0.8.1, alfa) · **CFA4:** `main` @ `16debf9` · **Kapcsolódik:**
> [`kdenlive-parity.md`](kdenlive-parity.md) (K1–K35), [`gaps.md`](gaps.md) (G-számok),
> [ADR 0004](adr/ADR-0004-compute-routing.md), [ADR 0005](adr/ADR-0005-media-backends.md)

Ez a dokumentum a Kdenlive-terv mintájára veti össze a Venturi képességeit és működési elveit a
CFA4-gyel. A Kdenlive-tervben már szereplő tételeket **nem ismétli**, csak hivatkozik rájuk
(K-szám), vagy kiegészíti őket. Az új teendők **V1–V10** számot kapnak.

## 0. Röviden

- A Venturi egy **Rust + egui + wgpu** alapú, Linux-első vágóprogram, ~130 ezer sor Rust,
  10 crate-ben. Szándékosan szűk: vágás, transzformok, kulcskockák, hangkeverő. Színkorrekció
  és szkópok is vannak, csomópont-szerkesztő nincs.
- A licence **GPL-3.0-or-later**, ugyanaz, mint a CFA4-é. Ez a legjobb helyzet: a kódja
  **közvetlenül átvehető**, nem kell átírni más nyelvről (a Kdenlive C++/Qt/MLT volt).
- A CFA4 a Venturi funkcióinak **nagy részét már tudja** (MCP, proxy, R128, hangmagasság-tartó
  gyorstekerés, Morph Cut, hibrid laptopon az integrált GPU, OTIO saját metaadattal, preset
  in/out horgonnyal). Az értéke ezért nem a funkciólistában, hanem **néhány kiforrott
  működési elvben** van: beszédérzékelés modellel, hardveres kódoló önellenőrzése,
  ügynökbiztos MCP-hoszt, inkrementális hangerőmérés.
- Javasolt sorrend: **V2, V3** (P1, nem MI), utána a P2-k mérés alapján. A **V1** (neurális
  VAD) és a **V7** (átirat) MI-funkció, ezért csak az MI-réteg üzembe helyezése után indulhat
  (tulajdonosi szabály, 2026-10-27; lásd 5. pont).
  Két tétel (V6 VST3, V7 felhős átirat) tulajdonosi döntést kér.

## 1. Licenc: mit lehet átvenni

| Rész | Licenc | Átvehető? | Megjegyzés |
|---|---|:-:|---|
| Venturi forráskód (`crates/*`) | GPL-3.0-or-later (`LICENSE`, `Cargo.toml` `license`) | ✅ | A fájlokban nincs SPDX-fejléc, ezért átvételkor **mi írjuk** a fejlécet: eredet (repó + commit), szerző (`morrolinux`, Venturi contributors), GPL-3.0-or-later. Bejegyzés a [`licensing.md` §5](licensing.md) nyilvántartásba ugyanabban a commitban |
| Silero VAD modell (`vv-vad/model`) | MIT (`LICENSE-silero-vad`) | ✅ | Eszköz, ezért `.attribution` oldalfájl és `ATTRIBUTION.md`-sor kell (`cargo xtask assets`) |
| `tract-onnx` (ONNX-futtató) | MIT OR Apache-2.0 | ✅ | Tiszta Rust, `cargo deny` szerint ellenőrizendő |
| `vst3` crate, VST3 SDK | MIT (az SDK 2025 óta MIT) | ✅ | Csak V6-hoz, külön folyamatban; a Steinberg/VST logó nem kerülhet be |
| Venturi név, logó, ikonok, képernyőképek | — | ❌ | Márka, nem vesszük át (ahogy a Kdenlive/KDE nevét sem) |
| `ffmpeg-next` használata | WTFPL (kötés), FFmpeg GPL-build | 🟡 | A CFA4-ben FFmpeg csak a `cfa4-ffmpeg` háttéren és a `cfa4-route` útválasztón át (ADR 0005, #249, #263). A Venturi FFmpeg-hívó kódja **mintának** jó, egy az egyben nem |

**Átvételi szabályok (kötelező):** a Venturi kódja nem felel meg változtatás nélkül a CFA4
szabályainak, ezért minden átvett részt át kell dolgozni:

1. **Soha ne omoljon össze** (AGENTS.md §0): a Venturi nem tesztkódjában ~180 `unwrap`/`expect`
  van. Átvételkor `Result` + `?`, `get()`, telített aritmetika.
2. **`unsafe` csak `crates/platform`-ban:** a Venturi `unsafe`-et használ a dekóderben, a
  kódolóban, a VST3-hosztban és a Wayland drag-and-dropban. Ezek nem vehetők át így.
3. **Idő `Tick`-ben:** a Venturi idővonal-képkockában (`FrameIdx`) és `Rational`-ban számol. A
  CFA4-ben minden idő `cfa4_time::Tick`; az átvett algoritmusokat erre kell átírni.
4. **Rétegek:** a Venturi `vv-session`/`vv-app` szétválasztása hasonló a CFA4
  `engine`/`ui-egui` rendjéhez; az átvett kód L5 alatt nem függhet egui-tól vagy cpal-tól
  (`cargo xtask layers`).

## 2. Ami a CFA4-ben már megvan (nincs teendő)

| Venturi-képesség | CFA4 | Bizonyíték |
|---|:-:|---|
| MCP-szerver ügynököknek (fejetlen és futó ablakhoz kötve) | ✅ | `crates/automation` (rmcp), `cfa4-cli mcp`, [`control-protocol.md`](control-protocol.md#mcp) |
| Proxy háttérben, waveform, idővonal-gyorsítótár | ✅ | `media.createProxies`, `crates/ui-egui/src/frames.rs`, `play_ahead.rs` |
| Hangmagasság-tartó gyorstekerés | ✅ | `crates/ui-egui/src/shuttle.rs` |
| Normalizálás (csúcs, true peak, EBU R128), EQ, kompresszor | ✅ | saját DSP, `mixer.*`, `essentialSound.*` |
| Smooth Cut (optikai áramlásos átmenet) | ✅ | Morph Cut (gaps.md G18 környéke, #805) |
| Hibrid laptopon az integrált GPU dekódol | ✅ | `crates/route/src/router.rs` (ADR 0004) |
| Tulajdonság-presetek a klip elejéhez vagy végéhez igazítva | ✅ | `crates/engine/src/presets.rs` (`AnchorToIn` / `AnchorToOut`) |
| OTIO saját metaadat-névtérrel | ✅ | `crates/interchange/src/otio.rs` (`metadata.cfa4`) |
| Szkópok, színkerekek, 16 bites lebegőpontos köztes képek | ✅ | `crates/scopes`, `crates/gpu` (`Rgba16Float`) |
| Szünetek törlése átirat alapján | ✅ | `transcript.removePauses` (`crates/engine/src/transcript.rs`) |
| Hangfelvétel az idővonalra | ✅ | `audio.voiceover.*` |

## 3. Új teendők

### V1 · Beszédérzékelés modellel (Silero VAD) · P1 · 20–35 óra

**Mit tud a Venturi:** a „Silence removal” a Silero VAD modellel (MIT, 1,3 MB, a binárisba
ágyazva) különíti el a beszédet a kattogástól és a teremzajtól, amit hangerőküszöb nem tud.
A modellt a `tract` futtatja, tiszta Rustban, natív ML-futtató nélkül.

**CFA4 ma:** a `crates/speech/src/vad.rs` **energiaalapú**, és csak a Whisper szóhatárait
szűkíti. Szünettörlés csak átirattal van (`transcript.removePauses`), ehhez előbb Whisper kell.

**Teendő:**
1. `crates/speech`: `vad::silero` modul `tract-onnx`-szel, a Venturi `vv-vad` alapján
   (fejléc + nyilvántartás), 16 kHz-re fagyasztott modell `.attribution` fájllal.
2. Új parancs `audio.detectSpeech` / `clip.removeSilence` (paraméterek: küszöb, minimális
   szünet, megtartott ráhagyás, `Tick`-ben), átirat nélkül, bármilyen nyelven.
3. A Whisper elé VAD-szűrő: csak a beszédes szakaszok mennek a modellbe. Ez gyorsít, és
   csökkenti a csendben „hallucinált” szöveget. **Kiegészíti a K31–K35-öt** (nagyobb
   modellek, élő mikrofonos felirat: ott a VAD dönti el, mikor zárul egy mondat).
4. Teszt: generált beszéd+zaj fixture (`target/fixtures/`), a VAD-határok tűrése ±1 ablak (32 ms).

### V2 · Hardveres kódoló önellenőrzése · P1 · 12–20 óra

**Mit tud a Venturi:** egy hardveres kódolót csak akkor kínál fel, ha egy kis próbakódolás
visszadekódolva **a helyes színt** adja (`check_colors`, egy egyszínű I420-kép). Indok: sok
FFmpeg-be fordított kódoló mögött nincs hardver, vagy rossz színt ír (pl. Intel ANV Gen9-en).

**CFA4 ma:** a `cfa4-route` (ADR 0004) eszközleltár és katalógus alapján választ; a
`route/src/probe.rs` nem végez próbakódolást. A tervezett VA-API-kódolás (#39) és a
FFmpeg/GStreamer kódolók (ADR 0005) pont ott hoznak kockázatot, ahol a Venturi megégett.

**Teendő:**
1. `cfa4-route`: kódolónként egyszeri **önteszt** (64×64, néhány képkocka, ismert szín és egy
   színátmenet), visszadekódolás a saját dekóderrel, eltérés küszöb alatt → elérhető.
2. Az eredmény gyorsítótárban (GPU + meghajtó verziója szerinti kulcs), így induláskor nem lassít.
3. A bukott kódoló a `perf.stats` → `route.chains` „passed over” okai között jelenik meg
   („rossz színt kódol”), és a felület a K-terv szerinti meghajtó-telepítési tippet mutatja.
4. A teszt `catch_unwind` alatt fut, időkorláttal: egy lefagyó meghajtó nem viheti magával a programot.

### V3 · Ügynökbiztos MCP-hoszt · P1 · 15–25 óra

**Mit tud a Venturi:** (a) az ablakhoz kötött MCP **visszatartja** a projektet módosító
hívásokat, amíg a felhasználó egy mozdulat közepén van (húzás, vágás), így az ügynök
módosítása sosem kerül a felhasználó visszavonási csoportjába; (b) nyitott párbeszédablak
alatt elutasítja őket; (c) a futó ablak **0600-as jogú Unix socketen** szolgál, a
`vv-app mcp --attach` köti hozzá a kliens stdio-ját; (d) a lassú hívások külön szálon futnak.

**CFA4 ma:** a vezérlőcsatorna és az MCP-híd TCP-n, `127.0.0.1:9876`-on megy
(`crates/automation/src/bridge.rs`). Ehhez a gép bármely helyi felhasználója vagy
folyamata csatlakozhat. A mozdulat alatti visszatartás ❓ (nem találtam).

**Teendő:**
1. Unix socket (`$XDG_RUNTIME_DIR/cfa4/control.sock`, 0600) mint alapértelmezett;
   a TCP csak kifejezett `--control <port>` kapcsolóval marad (tesztek, visszafelé kompatibilitás).
2. `cfa4-cli mcp --attach`: stdio ↔ socket híd.
3. Az ablakban: a módosító parancsok sorba állnak, amíg mozdulat vagy modális ablak van
   nyitva; a válasz „held” állapotot és okot jelez. Regressziós teszt a `tests/scripted.rs`-ben.
4. Ugyanez a Qt6-hídra (ADR F8, CLI bridge), mert ott is a `cfa4-cli` a kapu.

### V4 · Inkrementális hangerőmérés · P2 · 20–30 óra

**Mit tud a Venturi:** a normalizálás mérését a bemenet vágáspontjainál **szakaszokra**
bontja, minden szakaszt a saját tartalma szerinti kulccsal tárol (memóriában és
`$XDG_CACHE_HOME`-ban). Egy szerkesztés csak az érintett szakaszokat méri újra, egy ripple
egyet sem. A mérés a fájlokból fut, mint az export, nem az előnézet puffereiből; amíg kész nincs,
az előző szint szól. Klipenkénti és sávonkénti („relatív”) mód is van.

**CFA4 ma:** R128 és true peak mérés van; hogy hosszú projektnél szerkesztésenként mennyit mér
újra, ❓. **Teendő:** előbb mérés (`perf.stats`, 60 perces teszt-idővonal), és csak ha lassú,
akkor a szakaszkulcsos gyorsítótár átvétele a `crates/audio-dsp`-be (`Tick`-alapú szakaszhatárokkal).

### V5 · Lejátszás előtti puffer finomhangolása · P2 · 15–30 óra (mérés után)

A Venturi `RenderAhead` elvei, amelyeket a CFA4 `play_ahead.rs` / `frames.rs` megoldásával
össze kell mérni, és csak a jobbat tartani:
- a „keresés vagy előre dekódolás” küszöb **a megfigyelt GOP-hosszhoz igazodik**;
- egylépéses kiürítés: előbb az ablakon kívüli, utána a lejátszófejtől legtávolabbi képkocka;
- a hardveres dekóderek felületei **RAM-ból számolt globális keretből** bérelnek, a
  lejátszófej mögötti és a proxy-dekóderek csak a felét kaphatják; kereten felül szoftver;
- egy médium, amelynek hardveres dekódolása menet közben hibázik, ugyanott szoftverben nyílik
  újra, és ott marad (CFA4-ben: `hardware.fallbacks` számláló már van).

**Teendő:** benchmark a `docs/performance.md` szerint (hosszú GOP-os H.264 1080p, ugrálás),
és csak mérhető javulás esetén változtatás.

### V6 · VST3-hangbővítmények · P2 · 60–100 óra · **tulajdonosi döntés**

A Venturi VST3-hosztja a mixer effektláncában fut, natív szerkesztőablakkal (X11, macOS).
A CFA4-ben ez a **G11** (bővítményhosztolás, tulajdonosi döntés) része. Ha igen:
- **külön folyamatban** (mint a K-terv frei0r/LV2-hosztja, ADR 0003 A6–A7), mert a VST3
  COM-felülete `unsafe`, és egy hibás bővítmény nem döntheti össze a CFA4-et;
- a Venturi `vv-plugin` szkennelő, betöltő és valós idejű feldolgozó logikája mintának,
  a `vv-plugin-fixture` (gain-bővítmény) tesztbővítménynek átvehető;
- a VST3 SDK 2025 óta MIT, így a GPL-3.0-val összefér; a VST-logó nem használható.

### V7 · Whisper-kompatibilis HTTP-átirat · P3 · 20–30 óra · **tulajdonosi döntés**

A Venturi terve (`plans/TRANSCRIPTION.md`) a szabványos `POST /v1/audio/transcriptions`
végpontot hívja: ugyanaz a kliens szolgál ki helyi szervert (whisper.cpp `server`,
faster-whisper-server, LocalAI) és felhős szolgáltatót. Mono 16 kHz, tömörítve, darabolva,
átfedéssel; a szóidők a médium idejére vannak átszámolva.

A CFA4-ben a Whisper helyben fut (K2, K31–K35). Ez a tétel **opcionális kiegészítés**
lenne: helyi szerver alapból, felhős csak kifejezett hozzájárulással és figyelmeztetéssel
(„a hang elhagyja a gépet”), kulcs környezeti változóból. Ez megválaszolja a nyitott „felhős MI
hozzájárulással?” kérdést, ha a tulajdonos igent mond. A fordításra (K6) ugyanez a minta jó.

### V8 · Hálózati (tablet/böngésző) kezelőfelület · P3 · ötlet · **tulajdonosi döntés**

A Venturi `plans/WEB_FRONTEND.md` egy fejetlen asztali háttér + hálózati kliens felépítést
ír le: dekódolás, kompozitálás, kódolás az asztali gépen, a tablet csak megjelenít és
parancsot küld. A CFA4-ben az alap megvan (minden parancs, a felület állapota serde, a
vezérlőcsatorna). Kapcsolódik a K7 hálózati renderhez és a `cfa4-render-node`-hoz. Most
csak rögzítjük, megvalósítás nem javasolt.

### V9 · Felhasználói telepítő sudo nélkül · P3 · 6–10 óra

A Venturi `install.sh` az AppImage-et `~/.local/bin`-be teszi, menüt, ikont és MIME-társítást
regisztrál (`XDG_DATA_HOME`), adott verzióra rögzíthető, és `--uninstall`-lal a projektek
megtartásával eltávolít. A CFA4 kiadása HOLD-on van (F9), de az AppImage-csomaghoz ez a
minta átvehető, amikor a kiadás elindul.

### V10 · Stabil, újra nem osztott azonosítók · P2 · 4–8 óra (audit)

A Venturi `IdMap`-je soha nem oszt ki újra azonosítót, és a visszavonás ugyanazon az
azonosítón állítja vissza az entitást, így az ügynöknek adott azonosítók érvényesek maradnak.
A CFA4 MCP-jénél ugyanez fontos (`command_batch` hosszú ügynöki munkában). **Teendő:**
audit, hogy a CFA4 klip-, sáv- és médiaazonosítói teljesítik-e ezt; ha nem, regressziós teszt + javítás.

## 4. Rangsor és becslés

| # | Teendő | Prioritás | Óra | Függ |
|---|---|:-:|---:|---|
| V1 | Silero VAD, szünettörlés átirat nélkül, Whisper-előszűrő | P1 | 20–35 | **MI-réteg üzembe helyezése** (K31–K35-öt erősíti) |
| V2 | Hardveres kódoló önellenőrzése a `cfa4-route`-ban | P1 | 12–20 | ADR 0004, #39, #263 |
| V3 | Ügynökbiztos MCP: Unix socket, visszatartás mozdulat alatt | P1 | 15–25 | ADR F8 CLI-híd |
| V10 | Stabil azonosítók auditja | P2 | 4–8 | — |
| V4 | Inkrementális hangerőmérés | P2 | 20–30 | előbb mérés |
| V5 | Előre dekódoló puffer elvei | P2 | 15–30 | előbb mérés |
| V6 | VST3 külön folyamatban | P2 | 60–100 | **G11 döntés** |
| V7 | Whisper-kompatibilis HTTP-átirat | P3 | 20–30 | **MI-réteg üzembe helyezése** |
| V9 | Felhasználói telepítő | P3 | 6–10 | F9 kiadás |
| V8 | Hálózati kezelőfelület | P3 | — | ötlet |

Összesen kb. **170–290 óra**, ebből a P1 **47–80 óra**. A Kdenlive-terv (K1–K35) mellett ez
kis többlet, mert a Venturi tudásának nagy része a CFA4-ben már megvan.

## 5. Tulajdonosi döntések (2026-10-27)

A tulajdonos a tervet a javasolt döntésekkel együtt jóváhagyta:

1. **V6:** VST3 kell, **külön folyamatban**, az LV2-hoszt (K5) után. Ezzel a G11 hangbővítmény-része eldőlt.
2. **V7:** átirat Whisper-kompatibilis HTTP-n át: **alapból csak helyi szerver**; felhős szolgáltató
   csak kifejezett, beállításonkénti hozzájárulással (opt-in), figyelmeztetéssel.
3. **V8:** a hálózati kezelőfelület **ötlet marad**; a K7 hálózati render után újranézzük.

A megvalósítás a meglévő szabály szerint csak a tulajdonos kérésére indul, teendőnként.

**Projektszintű szabály (2026-10-27):** minden MI-funkció, minden alkalmazásban, csak az MI-réteg
üzembe helyezése után készül el (az MI-réteg terve folyamatban van). Ez itt a **V1**-et (Silero VAD
neurális modell) és a **V7**-et (átirat) érinti: mindkettő az MI-rétegre épül, és utána indul. A
V2, V3, V4, V5, V6, V9, V10 nem MI, ettől függetlenül indítható.

## 6. Hogyan frissítsd ezt a fájlt

Új Venturi-verziónál nézd át a `README.md` Features listáját, az `ARCHITECTURE.md` „Feature
status” részét és a `plans/` mappát, és frissítsd a fejlécben a commitot. Minden átvett
kódrészhez sor a [`licensing.md` §5](licensing.md) nyilvántartásban.

## Források

- Venturi: `README.md`, `ARCHITECTURE.md`, `TODO.md`, `plans/*.md`, `docs/MCP.md`,
  `crates/vv-vad`, `crates/vv-media/src/encode.rs` (`check_colors`) @ `cdc1c0f`.
- CFA4: [`kdenlive-parity.md`](kdenlive-parity.md), [`gaps.md`](gaps.md) G11,
  [`control-protocol.md`](control-protocol.md), `crates/route`, `crates/speech`, `crates/automation`.

## Változásnapló

- 2026-10-27: első változat (V1–V10).
- 2026-10-27: a tulajdonos jóváhagyta; 5. pont döntésekké alakítva.
- 2026-10-27: MI-réteg szabály: V1 és V7 az MI-réteg után; javasolt kezdés V2, V3.
