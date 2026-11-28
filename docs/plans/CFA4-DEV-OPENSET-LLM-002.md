# CFA4-DEV-OPENSET-LLM-002 – Helyi LLM-worker (OpenSet-LLM mint gyorsított CPU-motor) – v2.0 VÉGLEGES

**Állapot:** JÓVÁHAGYVA ÉS VÉGLEGESÍTVE, v2.0 FINAL (István, 2026-11-28 07:49 UTC; előtte 07:45 UTC: a 12. pont döntési listájának javaslatait D1–D15 mind elfogadta; a D15 és a 2 perces várakozási küszöb a 4.8 pontban). Az LLM-F0 (audit és mérés, kód nélkül) indulhat (D14); a többi kapu a 9. pont előfeltételeire vár. Kód- és PR-munka külön „mehet”-re (D13). `.md` fájl, **nem PR** (István szabálya). Kód nem készült, PR nem nyílt, semmi nem lett beolvasztva.
**Dátum:** 2026-11-28. **Alap:** a CFA4-DEV-OPENSET-LLM-001 v1.0 (István, 2026-11-28) elemzése és javítása.
**Kapcsolódó dokumentumok:** `VISION.md` (§1–4), `AGENTS.md` (a CFA4-szakasz eleje: licenc, „port vagy eredeti nyelv”), `docs/coordination.md` (§2.10–2.11) átolvasva, nem soronként; `CFA4-MODEL-MANAGER-001.md` v1.0 FINAL, `CFA4-AI-200-TERV-002.md` v1.0, ADR-0008 (AI-réteg), ADR-0004 (útválasztó), ADR-0006 + CFA4-TASKSTORE-PG-001, `acs-adaptiv-eroforras.md`, CFA4-MEDIA-INTELLIGENCE-001 v2.0, CFA4-SECURITY-001, az SDK protokollja és manifest-sémája, a `cfa4-route`, `cfa4-tasks`, `cfa4-store` crate-ek, `docs/development/reference-machine.md`, `CFA4-AI-UTEMTERV-2026-11-27.md`.

---

## 1. A lényeg

Az 1.0-s terv iránya jó, de **négy ponton ellentmond a már elfogadott CFA4-döntéseknek, három ponton pedig újra megírja azt, ami már megvan.** A javított terv ezeket rendezi, és a donor tényleges állapotához igazítja.

1. **„GPU nélküli működés” nincs, de a CPU-s út a foglalt GPU-k esetére kell (István, 2026-11-28 kiegészítése).** ADR-0008 és a Modellkezelő-terv: a 12 GB VRAM / 16 GB RAM alsó határ alatt nincs CPU-n futó AI-út (István, 2026-11-07, 2026-11-22). A padló fölött viszont a CPU-út **kiegészítő futtatási hely**: akkor is használható, ha a GPU vagy GPU-k foglaltak (másik modell tölti, nincs elég szabad VRAM, vagy a kijelzőt hajtják), és a modell képes CPU-n futni, **egy másik folyamatként a GPU-s munka mellett** (4.8). Ez külön jóváhagyást kér: D1, D15.
2. **Az OpenSet-LLM ma nem általános motor, hanem egy foltcsomag egyetlen modellre.** A Prism ML llama.cpp-elágazására tett kernel- és futásidő-foltok (C/C++), egyetlen támogatott modellel (Ternary-Bonsai-2-27B). A „modellválasztó”, a „több modell” és az „általános backend” az 1.0-ban még nem igaz. A javasolt építmény: **egy közös llama.cpp-családú LLM-worker** (CPU és CUDA ugyanabban a programban), amelyben az OpenSet gyorsított építési változat, rögzített commitról (D2).
3. **Nem készül külön adapter, ütemező, hardverfelmérő, modellkezelő.** Mindez megvan vagy már tervezett: worker-protokoll és SDK (ADR-0008 §7), `cfa4-tasks` (ACS-4, kész), ACS-1 hardverfelmérés, `cfa4-ai-models` (MM-001). A terv 4., 6., 8. fejezete ezért **csatlakozási pontokra** csökken (4. fejezet).
4. **A licencvizsgálat nem lehet blokkoló kapu a modellre.** A CFA4 sosem terjeszt modellt; a licencet a felhasználó fogadja el egyszer, modellenként; a modell/súly licence soha nem tilt funkciót. A motor (MIT) és a függőségek GPL-szabályai maradnak (6. fejezet).

**Mérési tény, ami átrajzolja a várakozást:** István referenciagépe két Xeon E5-2696 v4 (Broadwell-EP): **AVX2 van, AVX-512 és AVX-VNNI nincs.** Az OpenSet gyors (17–36 tok/s) útjához AVX-512 VNNI+VBMI kell; az AVX2-útra a donor **5,8–7,2 tok/s-ot mért, felhős VM-en, kikapcsolt AVX-512-vel, valódi AVX2-s gépen még nem**. A CFA4-ben a „minimumgépen” tehát hosszú, háttérben futó feladatokról van szó, nem interaktív csevegésről (5. fejezet).

---

## 2. Ellenőrzött donor-állapot (élő forrásból, 2026-11-28; emlékezetből semmi)

Módszer: sekély `git clone` (50 commit), `git ls-remote`, a GitHub keresési API-ja (a közvetlen repó-API nincs a munkamenet hatókörében), a Hugging Face nyilvános API-ja és fájljai. Csak olvasás.

| Tétel | Tény | Forrás |
|---|---|---|
| Repó | `opensetlabs/openset-llm` (az 1.0-ban „OpenSet Labs” szerepelt; a tényleges szervezet: **opensetlabs**) | keresési API |
| Licenc | **MIT** (`The ggml authors` 2023–2026 + `Open Set Labs` 2026). A fájlonkénti fejléceket nem néztem végig | `LICENSE`, keresési API |
| Kor / méret | létrehozva 2026-11-13; utolsó push 2026-11-27; címkék: v0.2.0, v0.3.0–v0.3.4; kb. 2239 csillag; 18 nyitott hibajegy | `ls-remote --tags`, keresési API |
| Rögzítendő commit | `ba9197ccdcb43fcf8119346820e5e172320e8764` (main, 2026-11-28 01:53 +05:30, „README: lead with the one-command install…”); v0.3.4 = `4c9922b3650e116c070fb258664bd851aec17c73` | `git log`, `ls-remote` |
| Mi ez | **foltcsomag + kernelek**, nem önálló motor: `patches/0001…0010`, `kernels/*.inc/.h`, `install.sh`, `serve`, `get.sh`. A motor forrása a telepítéskor jön. A README is így fogalmaz: „a set of new kernels and runtime patches on PrismML's llama.cpp fork” | `NOTICE`, `README.md` |
| Motor-elágazás | `PrismML-Eng/llama.cpp`, rögzítve `e3de715222b2667c54a267441159a5729f11155e`; licenc: **MIT** (a ggml szerzők sora). Az elágazás saját HEAD-je ma `43f31f400c…`, a főág (`ggml-org/llama.cpp`) HEAD-je `8e23278c7c…` → az elágazás **gyorsan mozog, a folt a régi commitra épül** | `ls-remote`, `LICENSE` |
| Támogatott modell | **egyetlen**: `prism-ml/Ternary-Bonsai-2-27B-gguf` (PTQ1_0, 5,95 GB) + opcionális MTP-oldalfájl. „More models … will follow” | `README.md` |
| Modell licence | **Apache-2.0** (Hugging Face kártya és `NOTICE.txt`; nem zárolt; 1,26 M letöltés; sha `839bd570…`, módosítva 2026-11-04). Alapmodell: `Qwen/Qwen3.8-27B`; a Qwen Apache-2.0 voltát a Prism ML NOTICE állítja, a Qwen-kártyán közvetlenül nem néztem. A Prism kéri (nem írja elő) a „Created using Bonsai by Prism ML” feltüntetést | HF API, `NOTICE.txt` |
| MTP-oldalfájl | `unsloth/Qwen3.8-27B-GGUF` (Apache-2.0 a donor szerint, **közvetlenül nem ellenőriztem**); a `graft_mtp.py` ebből és az alapfájlból **új, kb. 6,2 GB-os fájlt** ír (nem osztozik az alapfájllal) | `scripts/graft_mtp.py` |
| Modell tulajdonságai | 27,36 B paraméter, **gondolkodó (reasoning) modell**, 262 K kontextus (az OpenSet 4 K-val indul 8 GB-on), képbemenet külön mmproj-fájllal | HF kártya, `README.md` |
| Stock llama.cpp | a modell `Q2_0` csomagolása (7,63 GB) **a főági llama.cpp-n fut** a b11556 kiadástól, elágazás nélkül; lassabb és nagyobb | HF kártya |
| Ismert hiba | üres/megcsonkított válasz kis `max_tokens` mellett, mert a gondolkodás elfogyasztja a keretet; a Prism szerint ez a leggyakoribb oka a „rossz nem angol kimenetnek” is. `reasoning_effort: "high"` HTTP 500 | `KNOWN_ISSUES.md` |
| Futtatás | `./serve`: OpenAI-kompatibilis API **127.0.0.1:8080-on, hitelesítés nélkül**; alapból bekapcsolt WebUI (`--no-webui` kapcsoló); `--multi N`, `--spec`, `--ctx`, `--threads`, `--kv8/6/5` | `serve`, `README.md` |
| Telepítő | `install.sh`/`get.sh`: **apt-csomagokat telepít, a modellt és előre épített binárist tölt le** (SHA256SUMS ugyanabból a kiadásból); `curl … | bash` ajánlott. A CFA4 szabályai szerint ezt **nem használhatjuk** (12. fejezet, 4.5) | `install.sh`, `README.md` |
| Lemezre írt állapot | `~/.openset-llm/state`: legfeljebb 8 előtag-állapot, egyenként kb. 0,3 GB (a felhasználó promptjaiból épül) | `README.md` Limits |
| Memória | 8 GB a padló; 4 K kontextus alapból; 4 K felett <12 GB-on sima dekódolás; spekulatív dekódolás 6,5–7,2 GiB RSS; indításkor a súlyokat **újracsomagolja, 20–45 s** | `README.md` |
| Teljesítmény (donor, nem CFA4-adat) | AVX-512: 17–36 tok/s (Ice Lake, Zen 4, Zen 5); AVX2 (kikapcsolt AVX-512, Ice Lake VM): 5,8–7,2 tok/s, spekuláció alapból ki; Core 12+ AVX-VNNI: csak emulátorban ellenőrizve; ARM64: „untested”; AVX2 nélkül: „generic fallback: slow” | `README.md` |
| Futásidejű kernelválasztás | fordítási időben (`-march=native`) vagy indításkor (`--portable`, egy bináris minden x86-64-re) | `README.md` |

**LLM-F0 részeredmények (git-ből, 2026-11-28; István kérésére utólag elvégezve):**
- **Qwen3.8-27B és unsloth/Qwen3.8-27B-GGUF: Apache-2.0** (HF API, nem zárolt; Qwen sha `37561816…`, 2026-11-07; unsloth sha `4af51b35…`, 2026-11-12). Az MTP-fájl a telepítőben `MTP/mtp-Qwen3.8-27B-Q4_0.gguf`.
- **A `llama-server` támogat Unix-socketet** (a rögzített `e3de715` elágazás-commitban): a `--host` érték `.sock` végződéssel `AF_UNIX`-on kötődik (`tools/server/server-http.cpp` 437–444. sor); van `--api-key` (`server-http.cpp` ~206. sor). **A 4.2 pont ezért Unix-socketet használ**, nem loopback-TCP-t.
- **Az OpenSet-fájloknak nincs fájlonkénti licencfejlécük és SPDX-sorik** (kernelek, szkriptek: egyik sem); a licenc csak a repó `LICENSE`/`NOTICE` fájljában van (MIT). A foltok a következő llama.cpp-fájlokat érintik: `ggml-cpu.c`, `ops.cpp`, `ggml.c`, `ggml.h`, `llama-context.cpp`, `qwen35.cpp`, `server-context.cpp`, `arg.cpp`, `common.h`, `chat.h` (10 patch). A nyilvántartásba a repó-szintű MIT-et vesszük fel, a fejléc hiányát megjegyezve (`docs/licensing.md` §5).
- **Az elágazás (`e3de715`, 2026-10-13) MIT; a repóban nincs GPL/AGPL-szöveg a forrásfában.** A beágyazott Web UI (`LLAMA_BUILD_UI` **alapból BE**, `LLAMA_USE_PREBUILT_UI` **alapból BE**: építéskor előre épített UI-t tölthet le a HF-ről) npm-függőségei között **LGPL-3.0-or-later** csomagok vannak (`tools/ui/package-lock.json`, 14 előfordulás). **A CFA4-build ezért `-DLLAMA_BUILD_UI=OFF -DLLAMA_USE_PREBUILT_UI=OFF -DLLAMA_CURL=OFF`-fel épít**: nincs UI, nincs építéskori letöltés. (Az OpenSet `install.sh` csak a `LLAMA_CURL=OFF`-ot állítja.)
- **A `llama-server` alap-portja 8080 és a főág a 9931-re készül váltani**: nálunk nem számít (socket).
- A gyorsítótár- és `resume`-könyvtár környezeti változóval állítható (`LLAMA_RESUME_DIR`, `LLAMA_RESUME=0` kikapcsol): a CFA4-worker ezt a választott tárra irányítja vagy kikapcsolja (AC-20).
- A foltok **környezeti változókkal** kapcsolódnak (`GGML_X1`, `GGML_X2`, `LLAMA_SPEC_ADAPT`, `GGML_ATTN` …): ha nincsenek beállítva, a korábbi viselkedés marad. A worker ezeket kifejezetten állítja, nem örökli a környezetből.

**Utólag elvégezve (2026-11-28, részletek: [`CFA4-DEV-OPENSET-LLM-002-F0-JEGYZOKONYV.md`](CFA4-DEV-OPENSET-LLM-002-F0-JEGYZOKONYV.md) 3. pont):** patch-átnézés (nincs hálózati/folyamatindító hívás; lemezre csak a kikapcsolható `resume` ír) és a függőségi fa licencei a kikapcsolt UI-s buildre (nincs GPL-2.0-only/AGPL/zárt elem). **Még nyitott:** a `vendor/hash` négy licencszövegének egyenkénti elolvasása; a kernelek numerikus helyességének CFA4-oldali igazolása; a donor számai CFA4-hardveren; az AVX2-mérés a referenciagépen (ehhez a gép kell, nem a git).

---

## 3. Ütközések és átfedések (1.0 → CFA4 jelenlegi állapota)

| # | Az 1.0 állítása | Ütközik / átfed | Javítás a 2.0-ban |
|---|---|---|---|
| C1 | 1.1, AC-02: „GPU nélkül képes inference-re”, „GPU nélküli rendszer alapértelmezett nyelvi backendje” (7.1) | ADR-0008 §3: „GPU nélküli, csak CPU-s AI-út nincs”; MM-001 §6: alatta nincs CPU-n futó AI-út; `cfa4-tasks` `AiFloor::NotMet{NoGpu}` | Cél: **VRAM felszabadítása a küszöb fölött.** A képesség csak akkor elérhető, ha a 12 GB / 16 GB padló teljesül. A VISION 4. alapelvének CPU-tartaléka a **médiamotorra** vonatkozik, nem az AI-ra (ADR-0008 és az ACS-terv is így mondja) |
| C2 | 3.2, 7.1: a CPU-backend „a normál feladatválasztás része”, „GPU túlterhelésekor javasolt”, automatikus választás | István 2026-11-22: **nincs csendes tartalék**; ADR-0008 OOM-létra: másik futtató csak jóváhagyással | A **képességenkénti, felhasználói beállítás** dönt (`ai.llm`: „CPU” / „GPU” / „automatikus javaslat”). Futás közben nem váltunk. Ha a GPU foglalt, a Registry **javaslatkártyát** ad („CPU-n futhat, kb. X perc”), a felhasználó jóváhagyja |
| C3 | 6: saját „Inference API Adapter”, a motor OpenAI-kompatibilis HTTP-API-ja | ADR-0008 §7: minden worker külön folyamat, **JSON-lines Unix-socket**, SDK-protokoll; a hitelesítés nélküli 127.0.0.1-csatornát nem használjuk (SECURITY-001) | A CFA4-nek **egyetlen** csatlakozási felülete a worker-protokoll. Az OpenAI-API legfeljebb a worker és a gyermekfolyamat között van, **szigetelt hálózati névtérben**, API-kulccsal; kifelé nem látszik (4.2) |
| C4 | 8, P4: saját Scheduler és Queue, feladatprioritás, CPU-limit, becsült idő, megszakítás | `cfa4-tasks`: P0/P1/P2/R osztályok, négy indítási mód, `scheduled`/`paused` állapot, atomi RAM/CPU/VRAM-foglalás, PG-s bérlet (`SKIP LOCKED`), ACS-7 stabilitási próba kész | **P4 törölve.** A négy üzemmód (Kímélő/Normál/Teljesítmény/Ütemezett) a meglévő foglalási kérésre és `StartMode`-ra képződik le (4.3) |
| C5 | 4, P3: saját CPU-felismerés | ACS-1 hardverfelmérés (cgroup `cpu.max`, `cpuset`, affinitás), `accel` leltár, `cfa4-route::Inventory` | Csak az **ISA-jelzők** (AVX2, AVX-512 VNNI/VBMI, AVX-VNNI) hiányoznak: ezek a `/proc/cpuinfo`-ból jönnek az ACS-1 leltárába (4.4). A névből találgatás tilos (az 1.0 is ezt kéri) |
| C6 | 5, P2: saját modellregisztráció, kompatibilitás, betöltés/felszabadítás | MM-001 v1.0 FINAL (D1–D11, AC-01..28) | **P2 törölve**, a modell az `ai.model*` PG-táblákba kerül a Modellkezelőn át; **llmfit kötelező** (D10, AC-27) az LLM-ekre |
| C7 | 5.3: nincs duplikált cache | a donor `resume` állapotot ír `~/.openset-llm/state`-be; az MTP-graft új, ~6,2 GB-os másolatot készít | A `resume` alapból **kikapcsolva**; ha bekapcsolják, a felhasználó által választott tárba megy, a `cache` séma indexeli. Az MTP-graft az első körben **kimarad** (D6) |
| C8 | 13: „licencaudit sikeres” kapu, „redisztribúció csak ellenőrzés után” | MM-001 D1 + István 2026-11-07: a modell licence **nem blokkol**; nincs modellterjesztés; nincs fizetős változat (2026-11-22) | A T0 **rögzít és figyelmeztet**, nem tilt. Blokkoló csak a **kód- és függőségi** GPL-szabály (6.) és az integritás (hash/ismeretlen forrás: MM D1) |
| C9 | 2, 14 T2: az OpenAI-kompatibilitás validálása mint kaputerület | a CFA4-nek csak a worker-protokollra kell megfelelnie | A T2 a **worker-protokoll** és a gyermek-API egyezését méri, nem az „OpenAI-teljességet” |
| C10 | 9, 14 T5: a nyelvi modell „query-strukturálást” végez, pgvector-magyarázatot ad | MI-001: **a keresés nem függhet kötelező LLM-től**; az LLM nem állíthat elő közvetlen SQL-t; a worker nem írhat autoritatív adatbázist; DEV-20 „Opcionális LLM Q&A” csak **P2** | Az LLM az MI-ben **opcionális réteg**: típusos JSON-lekérdezést ad, amelyet a `cfa4-mi-query` tervező validál (4.7) |
| C11 | 20.3: az upstream módosítása nem cél | VISION: az upstream csak olvasható ős; a CFA4 önállóan fejlődik | Szó szerint átvéve a VISION megfogalmazásába; a szabály tartalma változatlan |
| C12 | Plan nem említi: ADR-0008 §F6 szerint „a helyi LLM mérete kérésenként egy kis ONNX-es beágyazó/pontozó választja” | a modellválasztás nem LLM-prompt | Az OpenSet egyetlen modellje „kis/közepes” osztály; a router-ötlet (weave-os) későbbi kapu, itt nem kell |
| C13 | Plan nem említi: a manifest `runtime.kind` ma csak `candle`, `onnx`, `python`, `remote` | SDK manifest 6 | **Új SDK-változás kell** (S16): `runtime.kind = "llamacpp"` és tokenfolyam az SDK protokollban (4.1) |
| C14 | Plan nem említi: ADR-0008 §ACS: „C-kód a CFA4 folyamatába nem kerül; külön workerként helyi LLM-hez csak későbbi, külön döntéssel jöhet” | ez a **külön döntés** még nincs meg; az `AGENTS.md` „port vagy eredeti nyelv” szabálya (István, 2026-11-03) viszont megengedi az eredeti nyelvet, ha a CFA4-et nem lassítja | D3: jóváhagyás kell, hogy a C/C++ motor külön workerben, szigetelve fusson. **A „megtartjuk, nem portoljuk” indoklás** (a szabály előírja, hogy a tervben szerepeljen): a motor több tízezer soros, gyorsan változó C++ (llama.cpp + AVX-512/AVX2 kernelek), Rust-portja aránytalanul hosszú és a donor-fejlődést nem követné; a CFA4 folyamatán kívül, felügyelt workerként fut, összeomlása hiba, nem CFA4-összeomlás |
| C15 | 3.1: „CPU/GPU hibrid feladatmegosztás” | nincs validált út, és nincs mérés | Az első körből kimarad; a CUDA-út ugyanazzal a worker-szal **önálló, külön igazolt út** a `paths[]`-ben |
| C16 | 1.1: „Magyar nyelvű feladatok tesztelése” | a 200-as terv és az MI-001 DEV-17 (Magyar benchmark, P0) | A magyar tesztkészlet közös az MI-001 DEV-17-tel, nem külön (7.) |

**Átfedés a 200-as tervvel:** a 28 „M8 LLM-vezérelt gyártás” képesség (200-as terv, 5. fejezet) és a forgatókönyv-fordítás segédje helyi LLM-et kér (AI-F6). **Ez a terv ennek az LLM-nek a futtatója**, nem külön képesség. A fordítás alapmodellje (MADLAD-400) nem változik; az LLM csak segít (a 200-as terv szerint).
**Átfedés az ADR-0004-gyel:** az `ai` család CPU-vége ma `candle-cpu`. Az LLM-worker új `Backend` (`llama-cpu`, később `llama-cuda`), nem helyettesíti a candle-t a Whisper/Kokoro útján.

---

## 4. Javított felépítés: csatlakozási pontok

```text
Qt6 (python/cfa4_qt/)  ── ai.* motorparancsok (L4, crates/engine) ──┐
                                                                     v
 cfa4-ai-registry  (mit futtatnánk és miért; javaslatkártya)   cfa4-route::Backend  (llama-cpu / llama-cuda)
        |                                                            |
 cfa4-ai-jobs  ── feladat ──>  cfa4-tasks (sor, osztály, foglalás, PG-bérlet)  ──>  cfa4-ai-workers (felügyelő)
                                                                                           |  JSON-lines, Unix-socket
                                                                                  cfa4-llm-worker (SDK-val)
                                                                                           |  szigetelt névtér, API-kulcs
                                                                              llama-server (llama.cpp-család, C++)
                                                                                 └ gyorsított építés: OpenSet-foltok
                                       Modellkezelő (cfa4-ai-models, llmfit)  ── GGUF a felhasználó által választott tárból
```

### 4.1 Worker és protokoll
- **`cfa4-llm-worker`**: SDK-t használó, külön folyamatként futó program. Felügyeli a `llama-server` gyermekfolyamatot, lefordítja a protokollüzeneteket (`hello/caps`, `load`, `run`, `cancel`, `unload`, `progress`, `result`, `stats`, `error`) a gyermek API-hívásaira. Összeomlás = `error: crash`, nem a CFA4 összeomlása (ADR-0008 §7).
- **SDK-bővítés (S16, külön SDK-PR, a #15/#17 után):** (a) `runtime.kind = "llamacpp"`; (b) opcionális **tokenfolyam** (`partial`/`stream` képesség a `hello/caps`-ban, a `progress`/`preview` mintájára; a gazda nélküle csak a végeredményt kapja); (c) a bemeneti/kimeneti típus már van (`text`, `json`, `utf8-json`). A protokollverzió emelése: SDK-döntés, nem itt.
- **Eszköz:** a `load`/`run` megnevezi a `"cpu"`-t vagy a GPU-t (ADR-0008 §7); a szálak számát a `cfa4-tasks` foglalása adja, nem külön worker-beállítás.
- **Hibakódok:** a meglévő `oom`, `invalid_input`, `model_error`, `crash` elég. A „kiürült a gondolkodási keret” egy `model_error` alkód lehet (`reasoning_exhausted`), hogy a gazda értse, miért üres a válasz (lásd 2. fejezet, ismert hiba).

### 4.2 A gyermek-API biztonsága
- A `llama-server` **csak a worker számára** érhető el: saját hálózati névtér (nincs külső interfész), véletlen API-kulcs, `--no-webui`. A munkafolyamat a CFA4-SECURITY-001 szerinti elszigetelésben fut (Landlock + seccomp, hálózat nélkül). A GGUF-fájlt natív C++-kód értelmezi, ezért a szigetelés itt **kötelező**, nem javaslat.
- **Unix-socket** (`--host` `.sock` végződéssel, igazolva, lásd 2. fejezet): ezt használjuk; loopback-TCP csak végső tartalék, a privát névtérben.
- A donor WebUI-ja, `get.sh`, `install.sh` és a beépített modell-letöltő **nem kerül be**.

### 4.3 Üzemmódok → `cfa4-tasks`
| 1.0 üzemmód | A CFA4-ben |
|---|---|
| Kímélő | alacsony CPU-szám a foglalási kérésben (pl. a hatásos CPU-k fele); `Class::Background` |
| Normál | a hatásos CPU-k mínusz a tartalék (az ACS alapja: egy CPU az asztalnak); `Class::Background` |
| Teljesítmény | a megengedett maximum; tartós CPU-nyomásnál a keret a meglévő szabályok szerint változik |
| Ütemezett | `StartMode`: időpontra / erőforrásra / időpontra és erőforrásra; `scheduled` állapot |

- Az LLM-feladat osztálya **`Background` (P1)**, soha `Speculative` (P2: a spekulatív előmunka eldobható, itt túl drága), és csak ritkán `Interactive` (P0) (rövid, sima dekódolás, előre betöltött modellel, a felhasználó vár rá).
- A foglalás: `ram_bytes` = súly + KV-gyorsítótár + a spekuláció pótlása (+ biztonsági tartalék), `cpu` = szálszám, **GPU-kérés nincs**. A 16 GB-os minimumprofilon ez azt jelenti, hogy egy 6–7 GiB-os LLM-worker mellett nem fér el egy RAM-nehéz generatív feladat: a `try_reserve` ezt sorba állítással kezeli, nem összeomlással.
- **Hosszú CPU-feladatok és PG:** a feladat állapota és naplója a `tasks` sémában (CFA4-TASKSTORE-PG-001), a végrehajtás **bérlettel** (`SKIP LOCKED`) és ismételt jelentkezéssel; a worker összeomlása után a feladat visszakerül a sorba, kétszer nem indul (PG-2). Mivel a motor indításkor **20–45 s-ig újracsomagolja a súlyokat**, a betöltés külön, mért szakasz a becsült időben (nem a modell „kiszolgálási” ideje). A hosszabb munkák **szakaszolhatók** (chunking, ADR-0008 §3: a szakaszolás eredményt nem változtat), hogy a megszakítás és az újraindítás ne dobja el az egész munkát.
- **A PG-függés időrendje:** a PG-alapú ütemezés (D-5 UUIDv7, DB-alapú ütemezés a #1584 előtt) a PG-állapot terve szerint előbb jön; az LLM-F3 (feladatsor-bekötés) erre épül.

### 4.4 Képességfelmérés (ACS-1 kiegészítése)
- Új leltármezők: `cpu.isa = {avx2, avx512f, avx512_vnni, avx512_vbmi, avx_vnni, fma}` a `/proc/cpuinfo` jelzőiből; a gyártó/modell/magok/szálak/RAM már megvan.
- Út-besorolás (az OpenSet README táblája alapján, **CFA4-méréssel felülírandó**): *gyors* (AVX-512 VNNI+VBMI), *részleges* (AVX-512 VNNI, VBMI nélkül), *256-bites* (AVX2, AVX-VNNI-vel vagy anélkül), *nem elérhető* (AVX2 nélkül; a donor „generic fallback: slow” útját nem tekintjük igazolt útnak).
- Tilos olyan ISA-kernelt választani, amit a CPU nem támogat; a kernelválasztást a csomagolt, **`--portable`** (indításkori kiválasztású) építés végzi, mert a `.deb`/Flatpak/AppImage nem épülhet a felhasználó CPU-jára. A `-march=native` építés csak a fejlesztői útra marad.
- A `Backend::accepts(task, device)` visszaadhat `Err(indok)`-ot, pl. „AVX2 hiányzik”, „kevés szabad RAM”, így a „mi futna és miért nem” lekérdezés (Registry) futtatás nélkül működik.

### 4.5 Modellkezelő és letöltés
- A Bonsai-2-27B az MM-001 szerinti **modellrekord**: forrás (HF-link), SHA-256 (rögzítve a felhasználó első letöltésekor, az MM-001 szerint), méret, RAM-igény, licenc (Apache-2.0, szövegre mutató link), kompatibilitás (`llama-cpu`, `llama-cuda`), állapot a 13 tárolt állapot közül.
- **Letöltés:** csak a felhasználó kérésére, `cfa4-fetch` feladattípusként, a felhasználó által választott tárba (AC-25: nincs néma alapértelmezett hely). Hiányzó modell = javaslatkártya, **soha nem automatikus letöltés** (AC-26). A donor `install.sh`-ja és `get.sh`-ja helyett a CFA4 saját, pinelt építési receptet használ (7. fejezet), hálózat nélkül.
- **Licenc:** a modellkártya megmutatja a feltételeket, egyszeri, modellenkénti elfogadás kell, tiltás nincs (D1). A Prism-féle „Created using Bonsai by Prism ML” szöveget a modellkártya és a Névjegy mutatja (kérés, nem kötelezettség).
- **llmfit** (AC-27): az ítélet a Bonsai-ra „ismeretlen” lehet (az adatbázis LLM-központú, de nem biztos, hogy ismeri ezt a ternáris modellt); ilyenkor a megjelenítés „ismeretlen”, nem kitalált. A 12 GB/16 GB padló érvényes akkor is, ha az llmfit „elfér”-t mondana.
- **Formátum:** GGUF a CFA4-SECURITY-001 D7 szerint megengedett (betöltéskor kódot nem futtató formátum); a modelscan nem vizsgálja (MM-001), ezért a hash- és forrásellenőrzés + a worker szigetelése a védelem.
- **Dedup:** az alapfájl egy példányban él; a levezetett (MTP-grafttal módosított) fájl **külön artefaktum**, nem osztozik az alappal, ezért az első körből kimarad.

### 4.6 Felület (Qt6)
- Minden kezelőfelület a `python/cfa4_qt/` alá kerül, a motorparancsokon (`ai.*`) át; **egui-hoz nem nyúlunk** (István, 2026-11-27: az egui nem akadály). Nincs külön LLM-felület, nincs donor-WebUI.
- Megjelenítendő (1.0 11.1 pont) változatlanul, kiegészítve: a becsült idő **tartományként** (nem egy szám), a betöltési idő külön, az ISA-út neve („gyors”, „256-bites”), a gondolkodás-mód állapota, a „kiürült a gondolkodási keret” hibaüzenet érthetően.
- Minden vezérlő regisztrál automatizálási azonosítót (CFA4-szabály: ügynökkel vezérelhető).

### 4.7 Media Intelligence
- A keresés **LLM nélkül** is teljes (MI-001). Az LLM csak (a) a magyar kérés **strukturált JSON-ra** fordítását, (b) a találatok összefoglalását, (c) a Q&A-t (DEV-20, P2) végzi.
- A `cfa4-mi-query` tervező a JSON-t **sémával és szabályokkal ellenőrzi**, az LLM sosem ad SQL-t; a worker nem ír adatbázist. A relevancia nem bizonyosság: az összefoglalóhoz a forrás-szegmens hivatkozás kötelező.

### 4.8 Kiegészítés (István, 2026-11-28): CPU-s futtatás foglalt GPU-k mellett, külön folyamatban

**Helyzet:** egy újabb modellre is szükség van, de a GPU vagy az összes GPU foglalt, a modell viszont képes CPU-n futni. Ilyenkor a CPU-s út **párhuzamos, másik folyamat**, nem várakozás a GPU-ra és nem a GPU-s munka megszakítása.

**Mikor áll fenn (a `cfa4-tasks` foglalása már most megmondja az okot):**
| Eset | Jel |
|---|---|
| A. A GPU-k VRAM-ja egy másik modellel/feladattal betelt | nincs elég szabad VRAM a kért foglaláshoz (`WaitReason`) |
| B. Minden felhasználható GPU-t már foglaltak (P1/R feladat) | nincs szabad GPU-keret |
| C. A kért modell nem fér a megmaradt VRAM-ba, de a CPU-útja igazolt | `Fit` a GPU-n „nem”, a CPU-úton „igen” |
| D. A GPU a kijelzőt hajtja, vagy a kijelzőhasználat nem olvasható | `GpuExcluded::DrivesDisplay/DisplayUnknown` |

**Feltételek (mind kell):**
1. A gép megfelel a padlónak (12 GB-os, kijelzőt nem hajtó GPU és 16 GB RAM; `AiFloor::Met`). A padló alatt nincs CPU-s AI-út.
2. A képesség manifestjének `paths[]` listájában van **igazolt CPU-út** (`validated{device,date,profile}`), a referencia-úttól való eltérés rögzítve. Aminek nincs ilyen útja (pl. a legtöbb PyTorch-os generatív modell), azt ez a szabály **nem teszi** CPU-n futtathatóvá. „A modell képes CPU-n futni” = a manifest ezt kimondja, nem találgatás.
3. **Várakozási küszöb: 2 perc** (István, 2026-11-28). Ha a GPU becsült felszabadulási ideje legfeljebb 2 perc, a feladat egyszerűen **vár a GPU-ra**, kártya és CPU-s út nélkül. Csak 2 percnél hosszabb becsült várakozásnál lép életbe az alábbi javaslatkártya vagy a felhasználó állandó beállítása. A küszöb állítható beállítás, alapértéke 2 perc. A becslés a `cfa4-tasks` foglalásából jön; ha nem becsülhető (ismeretlen, mikor szabadul fel), a 2 percnél hosszabb esetként kezeljük.
3b. **A váltás nem csendes** (István 2026-11-22): a CPU-út másik eszköz, ezért vagy (a) **javaslatkártya** a döntéshez („GPU-ra várna kb. X percet (X > 2), CPU-n kb. Y–Z perc, RAM-igény, hatás a többi feladatra”), vagy (b) a felhasználó **képességenkénti állandó beállítása**: *kérdezzen* (alapérték) / *CPU-n fusson, ha a GPU foglalt* / *várjon a GPU-ra* / *soha CPU-n*. A (b) beállítás a felhasználó nyílt, visszavonható döntése, a felületen mindig látszik, melyik úton futott a feladat. Futás **közben** nem váltunk át.
4. A CPU-s futás a RAM- és CPU-keretből foglal (`try_reserve`), a GPU-s feladat mellett is: a másik modell hosztoldali RAM-igényét (offload, puffer) a foglaló már számon tartja. A 16 GB-os minimumprofilon gyakran **nem fér el** a kettő együtt; ilyenkor a feladat várakozik és a kártya ezt mondja ki, nem összeomlás.
5. A CPU-s feladat **külön folyamat** (worker), így a GPU-s munka és az editor nem torzul: a CPU-szám a foglalásból jön (az asztalnak marad egy CPU), a `Class::Background` osztály miatt az export (R) megelőzi.
6. Eredménytűrés: a CPU-út eltérhet a GPU-s referenciától. Szöveges (LLM) kimenetnél tűrési mérték nem definiálható, ezért az út `automatic=false`: **mindig jóváhagyás kell** (a beállítás (b) pontja ezt a jóváhagyást előre megadja, de modellenként és nyíltan).

**Hatókör:** a szabály **általános** (Registry + manifest + `cfa4-tasks`), nem LLM-specifikus; ebben a tervben az LLM-worker az első alkalmazása. Az általános rész (Registry-kártya, a felhasználói beállítás, a `cfa4-tasks` indokjelentése) az ADR-0008-ba kerülő módosítás, nem ennek a tervnek a döntése: D15.

**Mérés:** külön tesztelendő a párhuzamosság (T3): egy GPU-s feladat (pl. kép/videó modell az A4000-en) fut, mellette az LLM-worker CPU-n; mérni kell a GPU-s feladat áteresztését (nem romlik a CPU-oldali előkészítés miatt), a GUI és a lejátszás képkockaidejét, a RAM-csúcsot a 16 és 32 GB-os profilon. A CPU-mérés szabálya (a két NVIDIA-kártya elrejtése) a CPU-only mérésre vonatkozik; a **párhuzamos** méréshez a számító kártya látható, az A1000 a kijelzőn marad.

---

## 5. Hardver, teljesítmény és mérés

- **A referenciagép AVX2-s Broadwell** (`docs/development/reference-machine.md`): ezen az AVX2-úton fogunk mérni, és ez lesz az **első valódi AVX2-s adat** (a donor csak AVX-512-mentesített felhős VM-en mért). A CPU-méréseket a gép szabálya szerint kell végezni: mindkét NVIDIA-kártya elrejtve (`CUDA_VISIBLE_DEVICES=""` és `--hw off`), rögzített magok, az `lscpu` kimenet a jegyzőkönyvben; a HT-párok miatt a kevés magra korlátozott mérés a „biztonságos, lassabb” irányba téved.
- **Minimumprofil-teszt:** a 8-magos, korlátozott cgroupban (`systemd-run --scope -p MemoryMax=32G -p CPUQuota=800%`, valamint a 16 GB-os minimumprofil `MemoryMax=16G`), a meglévő `tests/run-minspec.sh` burkolóval; `CUDA_VISIBLE_DEVICES` a 3060-ra (amíg nincs, az RTX A4000-re). A fejlesztőgép 512 GB-os RAM-ja elfedné a memórianyomás-hibákat, ezért korlátozás nélküli mérés csak „informatív” címkével készülhet.
- **AVX-512-es mérés:** a referenciagépen **nem lehetséges**. Ha István akar AVX-512-es számot a tervbe, kell egy külső gép (Zen 4/5 vagy Ice Lake+), egyszeri mérésre, kód nélkül. Addig az AVX-512-út csak „donor szerint”, **nem igazolt CFA4-út** (D7).
- **Elfogadási küszöbök:** a donor tok/s számai nem lehetnek elfogadási küszöbök. A CFA4 saját mérése után rögzítjük: üresjárati tok/s, első token ideje, indítási idő, RSS, a GUI válaszkészsége a futás alatt (a CFA4 saját képkocka-ütemezésére mérve).
- **Reális várakozás:** egy gondolkodó 27 B-modellnél a hosszú válasz AVX2-n perceket vesz igénybe. Emiatt a felület **háttérfeladatként** kezeli, becsült idő-tartománnyal; a kép- és vágási interaktív utak nem függhetnek LLM-választól.
- **RAM:** 8 GB a donor padlója, **nem a CFA4-é**. A CFA4-ben a 16 GB-os minimumon a worker (~6–7 GiB) mellett marad hely az editornak, de nem marad a többi nagy modellnek: ezt a foglaló kezeli; az ajánlott 32 GB a teljesítményígéret.

---

## 6. Licenc- és függőségi állapot (a 13. fejezet helyett)

| Terület | Szabály | Állapot |
|---|---|---|
| Motor (llama.cpp / PrismML-elágazás / OpenSet-foltok) | MIT; GPL-3.0-or-later-kompatibilis. Átvehető **sértetlen fejlécekkel és nyilvántartási bejegyzéssel** (`docs/licensing.md` §3, §5) | Az 1. és 2. fejezet szerint MIT; a fájlonkénti fejléceket az LLM-F0 nézi át |
| Rust-oldal (`cfa4-llm-worker`) | `cargo deny check licenses` zöld | feladat az LLM-F2-ben |
| C/C++ függőségek | a nyilvántartásba kerülnek (a `cargo deny` nem látja őket), a rögzített commit + a forrás SHA-256 | LLM-F0 |
| Modell (Bonsai-2-27B, Apache-2.0) | **nem blokkol**; megjelenítés + egyszeri elfogadás; a CFA4 nem terjeszti | D1 szerint |
| Alapmodell (Qwen3.8-27B), MTP-oldalfájl | ugyanez; mindkettő **Apache-2.0** (HF API, 2026-11-28) | ellenőrizve |
| AGPL / GPL-2.0-only / zárt kód | tilos (változatlan), külön folyamatú AGPL-worker csak a #58 szabályai szerint | nincs ilyen függőség a donorban (ellenőrizendő a teljes fán) |
| Bináris terjesztés | a **CFA4 saját CI-ja építi** a motort, rögzített forráscommitból; a donor előre épített binárisát nem használjuk (az integritást a kiadás saját SHA256SUMS-a adja, ugyanarról a tárhelyről: sértetlenséget ad, hitelességet nem) | LLM-F4 |
| Csomagolás (deb, Flatpak, AppImage, tarball) | `--portable` építés; a C++ fordítókészlet a CI-ban, nem a felhasználónál; az `apt` hívó donor-telepítő nem fut | LLM-F4 |
| Vízjel, licenckulcs, használati korlát | nincs (nincs fizetős rész, 2026-11-22). Megmarad: a modell licencfigyelmeztetése és a MI-által-módosított jelölés (nem alkalmazandó itt) | — |

Az 1.0 „megengedett-e jogilag” kapuja így két külön kérdésre bomlik: **a kódra** (GPL-kompatibilis: igen, MIT) és **a modellre** (nem kapu, csak figyelmeztetés).

---

## 7. Magyar nyelv

- A donor és a Prism ML semmilyen magyar mérést nem közöl; a Qwen3.8-27B magyar tudása **nincs igazolva** a CFA4-ben. A Prism ML ismert-hibalistája szerint a nem angol kimenet gyakori problémáját a megcsonkult gondolkodás okozza; ezért a tesztet **két beállítással** kell futtatni (gondolkodás ki és be, nagy `max_tokens`).
- A tesztkészlet **közös az MI-001 DEV-17-tel** (magyar benchmark), nem külön: magyar promptértés, ragozás, ékezetmentes alakok, szinonimák, filmes szakszavak, idő- és időtartam-kifejezések, kevert nyelv, strukturált JSON magyar szöveggel, hallucinációs vizsgálat.
- Számszerű elfogadási küszöböt **nem találok ki**: az LLM-F5 első fele a mérés (alapvonal); a küszöböt István a mért alapvonal és a saját minták alapján dönti el (D10). A tesztpromptokat tervezet formában én írom meg, István véleményezi.
- Nyelvi hatókör: a worker nyelvi képességeit a 200-as terv szerint a manifest `languages` mezője rögzíti; a fordítás alapmodellje a MADLAD-400 marad, az LLM csak segít.

---

## 8. Kapuk (fázisok), tesztek, elfogadás

Előtag: **LLM-F** (az AI-F6 „helyi LLM” alatt). A fejlesztés **jóváhagyott terv és külön „mehet” nélkül nem indul**; az LLM-F0 kivételével semmi nem indul a #63 beolvadása és a felsorolt előfeltételek előtt.

| Kapu | Tartalom | Előfeltétel | Kapukritérium |
|---|---|---|---|
| **LLM-F0** Forrás- és mérési audit (nem kód) | commit/hash rögzítése; fájlonkénti licenc; Qwen/unsloth licenc közvetlen ellenőrzése; a `llama-server` Unix-socket/API-kulcs/névtér-viselkedés; AVX2-mérés a referenciagépen a korlátozott cgroupban; **jegyzőkönyv** | ez a terv jóváhagyva | a jegyzőkönyv kész; nincs blokkoló licenc/ISA-lelet; D7 eldőlt |
| **LLM-F1** SDK (S16) | `runtime.kind = llamacpp`, tokenfolyam, manifest-példa, séma-tesztek | SDK #15/#17 beolvadt; D4 | az SDK CI zöld; régi manifestek érvényesek |
| **LLM-F2** Worker | `cfa4-llm-worker` + a gyermek felügyelete, szigetelés, hibaképzés, megszakítás | AI-F2 (`cfa4-ai-workers`) létezik; D3 | a protokollteszt zöld; összeomlás/OOM/időtúllépés kezelt |
| **LLM-F3** Sor és modell | `Backend` (`llama-cpu`), foglalás a `cfa4-tasks`-ban, modellrekord, llmfit, letöltés javaslatkártyával | AI #63 + MM-F2b, F3c, F4, F5; a PG-alapú ütemezés | a sor/bérlet újraindítás után is helyes; nincs automatikus letöltés |
| **LLM-F4** Csomagolás | pinelt CI-építés, `--portable`, nyilvántartási bejegyzés, `cargo deny`/`xtask assets` | LLM-F2 | a build reprodukálható; csomagformánként igazolt |
| **LLM-F5** Magyar mérés | alapvonal (két beállítás), a küszöbök meghatározása | LLM-F2 | dokumentált eredmény; István elfogadja a küszöböt |
| **LLM-F6** MI-bekötés | strukturált lekérdezés, összefoglalás, (P2) Q&A, a tervező validálja | MI-PG-4 / AI-F4 (`mi` séma, pgvector); LLM-F3, F5 | az MI LLM nélkül is teljes; LLM-mel helyes, hivatkozott kimenet |
| **LLM-F7** Qt6 | beállítások, állapot, sor, figyelmeztetés | motorparancsok (LLM-F3); nem blokkolja az egui eltávolítása | automatizált UI-teszt + képernyőkép |
| **LLM-F8** Lezárás | tesztjelentés, támogatott CPU-k, ismert hibák | mind | elfogadás vagy elutasítás dokumentálva |

**Tesztek (az 1.0 T0–T7 javítva):**
- **T0** forrás- és licencrögzítés (nem tilt a modellre); **T1** ISA-útvonalak és a nem támogatott CPU szabályos elutasítása (ne omoljon össze: AGENTS §0); **T2** worker-protokoll (hello/caps, load/run/cancel/unload, streaming, időtúllépés, hibás kérés, UTF-8/magyar karakterek, JSON); **T3** erőforrás: RAM/CPU, hosszú és egymás utáni feladatok, **GUI-válaszkészség és médiafeldolgozás-hatás mérése**, párhuzamosság a GPU-s feladatokkal, a 16 GB-os minimumprofil, és a **„nem lassítja a CFA4-et” bizonyíték a referenciagépen**: lejátszás- és renderidő (képkockaidők) változatlan, amíg az LLM fut (`AGENTS.md`, port-vagy-eredeti-nyelv szabály); **T4** magyar (7.); **T5** MI (4.7); **T6** offline (hálózat nélkül indul; nincs kimenő kapcsolat: hálózati névtér + naplóellenőrzés); **T7** rendszerintegráció (Qt6, `cfa4-tasks`, Modellkezelő, PG-újraindítás).

**Elfogadási kritériumok (az 1.0 AC-01…18 javítva):**

| ID | Kritérium | Változás |
|---|---|---|
| AC-01 | Linux amd64-en fut (Ubuntu 26.04) | változatlan |
| AC-02 | A 12 GB / 16 GB padló fölött a nyelvi feladat **CPU-n, külön folyamatban fut, miközben a GPU-k foglaltak vagy szabadon maradnak** egy másik modell számára (4.8); a padló alatt nincs CPU-s AI-út | **átírva** (nem „GPU nélküli”) |
| AC-03 | ISA-felmérés a tényleges `/proc/cpuinfo`-jelzőkből; AVX2 nélkül a képesség nem elérhető, szabályos indokkal | pontosítva |
| AC-04 | Csak a központi Modellkezelőt használja (egy fizikai másolat) | változatlan |
| AC-05 | A worker-protokoll egyetlen csatlakozási felület; nincs külön publikált API | **átírva** |
| AC-06 | Offline működik; nincs kimenő hálózat | változatlan |
| AC-07 | Nincs automatikus modell- vagy binárisletöltés, nincs `apt`/`curl|bash` | bővítve |
| AC-08 | Qt6 (`python/cfa4_qt/`) felület, egui nélkül, automatizálható | pontosítva |
| AC-09 | Sor, ütemezés, erőforráskezelés a **meglévő** `cfa4-tasks`-on | **átírva** |
| AC-10 | CPU-szám a foglalásban állítható, a GUI nem válik használhatatlanná | pontosítva |
| AC-11 | Magyar minőség dokumentált mérés alapján, küszöböt István hagyja jóvá | pontosítva |
| AC-12 | MI: LLM nélkül is teljes; LLM-mel a kimenet a tervezőn át validált, forráshivatkozással | **átírva** |
| AC-13 | A GUI nem fagy le futás közben | változatlan |
| AC-14 | Backendhiba kezelt; nincs csendes váltás, csak javaslatkártya | pontosítva |
| AC-15 | Motor és függőségek licenc- és nyilvántartási rendben; a modell licence **nem blokkol**, egyszeri elfogadással | **átírva** |
| AC-16 | Nincs automatikus felhőváltás | változatlan |
| AC-17 | Csomagolás: a CFA4 építi a pinelt forrásból, `--portable`, csomagformánként igazolva | pontosítva |
| AC-18 | Feladat- és backendnapló követhető, a CFA4 meglévő naplójában | változatlan |
| **AC-19** | **Új:** nincs titkosítatlan, hitelesítés nélküli helyi API a gazdagép többi folyamata felé (szigetelt névtér, API-kulcs) | új |
| **AC-20** | **Új:** a `resume`-állapot alapból ki; bekapcsolva a felhasználó által választott tárba megy, és törölhető | új |

---

## 9. Ütemezés és sorrend

1. **Most (jóváhagyás után, kód nélkül): LLM-F0** audit és AVX2-mérés. Nem érinti a repókat; az eredmény `.md` jegyzőkönyv, nem PR.
2. **A #63 beolvadása után** a Modellkezelő-sorozat (MM-F2b … F5) és a PG-munka a jóváhagyott sorrendben (a PG-állapot dokumentum szerint: #1916 → db-setup → PG-2 → D-5 UUIDv7 → DB-alapú ütemezés a #1584 előtt → ai kapu → registry → mi+pgvector). Ezek **előzik** az LLM-F3-at.
3. **LLM-F1 (SDK)** az SDK #15/#17 után; **LLM-F2** az AI-F2 (worker-felügyelő) után; **LLM-F4** párhuzamosan az F2-vel; **LLM-F5** az F2 után; **LLM-F3** a Modellkezelő és a PG-ütemezés után; **LLM-F6** az MI `mi` sémája után; **LLM-F7** az F3 parancsai után.
4. A helyi LLM a rendszerben AI-F6 („PyTorch és LLM”), de a llama.cpp-családú worker **nem igényel PyTorch-környezetet**, ezért az F6 PyTorch-részétől **függetlenül** is mehet, ha az F2 worker-csővezeték már igazolt (D12).
5. **A mérés időzítése** (István kérdése, 2026-11-28): a fejlesztés megkezdéséhez a referenciagépes mérés **nem kell**. Mérés nélkül indulhat az SDK-bővítés, a worker váza, a sor- és modellbekötés, a csomagolás, a Qt6-felület; ezek a becsült időt és a szálszámot „nem mért” jelöléssel kezelik. A mérés legkésőbb az LLM-F2 végére készüljön el (a „nem lassítja a CFA4-et” bizonyíték a worker első működő változatánál kell), az LLM-F3 előtt biztosan; az LLM-F5, az LLM-F8 és az AVX2-igazolás nélküle nem zárható. Az AVX-512-es mérés (D7) külön és opcionális.
6. **Beolvasztási sorrend:** minden új PR bekerül a Gazda #1705 összesítőbe (állandó szabály: a host csak onnan látja a sorrendet). Egyetlen PR sem épül egy másik, még be nem olvadt PR ágára.
7. Ez a terv jóváhagyás után a CFA4 `docs/plans/` alá kerülne (VISION 6. alapelv: a terv a `main`-en él); mivel az István-szabály tiltja a tervet PR-ban jóváhagyatni, a kommit **csak jóváhagyás után**, István külön „mehet”-jére készül (D13).

---

## 10. Kockázatok (javítva)

| Kockázat | Hatás | Kezelés |
|---|---|---|
| A donor két hete létezik, gyorsan változik, a foltok egy régi elágazás-commitra épülnek | törékeny karbantartás | commit + forrás-hash rögzítve; **stock llama.cpp mint tartalék út** (a Q2_0 csomagolás főági motoron fut, lassabban); a CFA4 nem függ az OpenSet-től a helyes működéshez |
| Egyetlen támogatott modell | a „modellválasztó” üres; a Prism ML modell megszűnése/változása | a worker llama.cpp-családú, tetszőleges GGUF-ot tud; az OpenSet gyorsítás csak ott él, ahol igazolt; a Modellkezelő megszűnt-modell kezelése (AC-23) |
| AVX2-sebesség alacsony, az AVX-512 nem mérhető a referenciagépen | hosszú várakozás, bizonytalan állítások | háttérosztály, idő-tartomány, „donor szerint” címke, külső mérés (D7) |
| Gondolkodó modell üres/megcsonkult válasza | rossz magyar minőség látszata | `reasoning_exhausted` hiba, nagy `max_tokens`, mód- és mérés-párhuzam (7.) |
| Natív GGUF-értelmező C++-kódban | támadási felület | szigetelés, hash, forrásellenőrzés (4.2, 4.5) |
| RAM a 16 GB-os minimumon | nem fér el több nagy feladat | foglaló sorba állít; nem összeomlik; 32 GB ajánlott |
| `resume`-állapot a lemezen (felhasználói promptokból) | adatvédelmi kockázat | alapból ki, törölhető, választott tárban (AC-20) |
| Az LLM kimenete hibás/hallucinált | rossz keresési szándék | séma + tervező-validálás, forráshivatkozás, relevancia ≠ bizonyosság |
| Az SDK-bővítés késik | az LLM-F2 elakad | a tokenfolyam opcionális: nélküle is működik a végeredménnyel |
| Gazda-szűk keresztmetszet | késő beolvadás | a #1705 összesítő; nincs egymásra épülő PR-lánc |

---

## 11. Hatókör

**Benne:** helyi LLM-worker (llama.cpp-család, OpenSet gyorsított építéssel, igazolt úton); SDK-bővítés (S16); `cfa4-route` `Backend`; foglalás és sor a meglévő `cfa4-tasks`-ban; Modellkezelő-rekord, llmfit; ISA-leltár az ACS-1-ben; Qt6-felület; magyar mérés; MI-bekötés (opcionális réteg); offline és szigetelés; tesztek és dokumentáció.

**Nincs benne:** önálló OpenSet-alkalmazás; donor WebUI; saját modell, tanítás, finomhangolás, **LoRA-adapterek** (az OpenSet kísérleti funkciója); automatikus letöltés/telepítés; felhő; második adatbázis; CPU/GPU hibrid megosztás; MTP-graft (az első körben); GPU nélküli AI-út; nem amd64; Windows/macOS; fizetős rész; a modell terjesztése.

---

## 12. Döntési lista (javasolt válaszokkal)

| # | Kérdés | Javaslat | Miért |
|---|---|---|---|
| **D1** | A cél „GPU nélküli működés” helyett „CPU-s kiegészítő futtatási hely a 12 GB / 16 GB padló fölött” (foglalt GPU-k, további modell párhuzamosan)? (AC-02 átírva, 4.8) | **Igen**, a padló marad | István 2026-11-07/22 döntése; az ADR-0008 és a Modellkezelő-terv ezt mondja |
| **D2** | A worker legyen llama.cpp-családú és modellfüggetlen (GGUF), az OpenSet pedig opcionális, pinelt, gyorsított építés az egyetlen igazolt modellre? | **Igen** | az OpenSet ma egy modellre való foltcsomag; a stock llama.cpp mentőtartalék |
| **D3** | Jóváhagyod, hogy C/C++ motor külön, szigetelt workerben fusson? (ADR-0008 külön döntést kért) | **Igen** | a CFA4 folyamatába nem kerül C-kód; Rust-first csak a saját kódra; nincs `unsafe` |
| **D4** | SDK-bővítés (S16): `runtime.kind = llamacpp` és opcionális tokenfolyam? | **Igen** | a manifest ma nem tud ilyen futtatót; nélküle nincs csatlakozás |
| **D5** | Az első körben egyetlen validált modell (Bonsai-2-27B), a többi GGUF később? | **Igen** | amit nem mértünk, nem hirdetjük |
| **D6** | Az MTP-oldalfájl és a graftolt másolat kimarad az első körből? | **Igen** | új ~6,2 GB-os másolat, külön letöltés; az AVX2-úton a spekuláció alapból ki van |
| **D7** | Kell-e egyszeri AVX-512-es mérés külső gépen (Zen 4/5, Ice Lake+)? A referenciagép Broadwell. | **Igen, de csak ha az AVX-512-es számot hirdetni akarod**; különben az AVX-512-út „donor szerint”, nem igazolt | a referenciagépen nem mérhető; kód nem kell hozzá |
| **D8** | Alsó CPU-határ az LLM-hez: AVX2 kell, nélküle nem elérhető? | **Igen** | a donor „generic fallback: slow” útját nem tekinthetjük igazolt útnak |
| **D9** | Gondolkodás-mód alapból ki a strukturált (MI) feladatoknál, és külön mérés mindkét beállításra? | **Igen** | a Prism ismert hibája: a megcsonkult gondolkodás rossz nem angol kimenetet okoz |
| **D10** | Ki határozza meg a magyar elfogadási küszöböt? | **István**, a mért alapvonal és a saját minták alapján; a promptokat tervezetként én írom | nem találok ki küszöböt mérés nélkül |
| **D11** | A `resume`-állapot alapból kikapcsolva, bekapcsolva a választott tárban? | **Igen** | a felhasználó promptjaiból épülő lemezállapot adatvédelmi kockázat |
| **D12** | A llama.cpp-worker az F6 PyTorch-részétől függetlenül indulhat, ha az F2 worker-csővezeték igazolt? | **Igen** | nem igényel PyTorch-ot; korábban hoz hasznot (28 M8-típus) |
| **D13** | Jóváhagyás után a terv a CFA4 `docs/plans/` alá kerüljön (VISION 6. alapelv; `coordination.md` §2.11: minden terv a repóba, saját ágon, PR-rel), a Gazda #1705-be írva? | **Igen, csak jóváhagyás után és külön „mehet”-re** | az István-szabály a *jóváhagyás előtti* PR-t tiltja; a `coordination.md` a jóváhagyott terv helyét a repóban írja elő |
| **D14** | Az LLM-F0 (audit és AVX2-mérés, kód nélkül) indulhat most, párhuzamosan a #63-mal? | **Igen** | nem érinti a repókat; az eredmény `.md` jegyzőkönyv |
| **D15** | A foglalt-GPU esetű CPU-s futtatás általános szabály legyen (Registry-kártya, képességenkénti beállítás: kérdezzen / CPU ha foglalt / várjon / soha; csak igazolt CPU-úttal rendelkező képességekre), és az ADR-0008 ennek megfelelő módosítása külön PR-ban készüljön el? | **Igen**; alapérték: *kérdezzen* | István 2026-11-28 kérése; a „nincs csendes tartalék” elvet megőrzi, mert a váltás nyílt és előre adott |

---

## 13. Mit nem csináltam

Nem írtam kódot, nem nyitottam PR-t vagy hibajegyet, nem olvasztottam be semmit, nem töltöttem le modellt, nem futtattam a donor telepítőjét. A donor-repót csak sekélyen klónoztam (olvasásra), a Hugging Face-ről csak a nyilvános metaadatot és a szöveges kártyákat olvastam. A CFA4 és az AI-repók helyi másolatát olvastam; a `plan/` mappa gitignorált és nem volt elérhető, ezért a nyilvános megfelelőket használtam (`ROADMAP.md`, `docs/gaps.md`, `docs/roadmap.md`). A donor teljesítményadatait nem mértem újra.
