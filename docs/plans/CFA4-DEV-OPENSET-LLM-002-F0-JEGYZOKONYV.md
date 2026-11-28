# LLM-F0 jegyzőkönyv – forrás- és licencaudit (részleges)

**Terv:** CFA4-DEV-OPENSET-LLM-002 v2.0 FINAL. **Dátum:** 2026-11-28. **Állapot:** a git-ből elvégezhető rész KÉSZ (licenc, patch-átnézés, függőségi fa); a referenciagépes mérés NYITOTT. `.md`, nem PR.

## 1. Rögzített források
| Tétel | Érték |
|---|---|
| OpenSet-LLM | `opensetlabs/openset-llm`, main `ba9197ccdcb43fcf8119346820e5e172320e8764` (2026-11-28 01:53 +05:30); v0.3.4 = `4c9922b3650e116c070fb258664bd851aec17c73`; MIT |
| Motor-fork | `PrismML-Eng/llama.cpp` `e3de715222b2667c54a267441159a5729f11155e` (2026-10-13); MIT |
| Modell | `prism-ml/Ternary-Bonsai-2-27B-gguf`, HF sha `839bd57023c1558945afea70a74fa83c054f35f9` (2026-11-04); Apache-2.0 |
| Alapmodell | `Qwen/Qwen3.8-27B`, sha `37561816e66b5b9153b1b64c10968c18c1e6f4a0` (2026-11-07); Apache-2.0 |
| MTP-oldalfájl | `unsloth/Qwen3.8-27B-GGUF`, sha `4af51b3542f775b0a8e4b310bfb3f17e01c306c9` (2026-11-12); Apache-2.0 (első körben nem használjuk, D6) |

A forrás-SHA-256 (tarball) a CI-építés (LLM-F4) bevezetésekor kerül rögzítésre; ma csak commit-azonosító van.

## 2. Eredmények
- **Licenc:** nincs blokkoló lelet. A forrásfában nincs GPL/AGPL-szöveg (fork és OpenSet). Az OpenSet-fájlokon nincs fájlonkénti licencfejléc/SPDX; a licenc repó-szintű (MIT). Nyilvántartási sor: repó-szintű MIT, a fejléc hiánya megjegyezve.
- **Web UI:** a fork `LLAMA_BUILD_UI` és `LLAMA_USE_PREBUILT_UI` opciója alapból BE (építéskori letöltés; npm-függőségek közt LGPL-3.0-or-later). CFA4-build: `-DLLAMA_BUILD_UI=OFF -DLLAMA_USE_PREBUILT_UI=OFF -DLLAMA_CURL=OFF`.
- **Unix-socket és hitelesítés:** `--host *.sock` → `AF_UNIX`; `--api-key` támogatott. A worker socketet és API-kulcsot használ.
- **Donor-telepítő:** `install.sh`/`get.sh` apt-ot hív, modellt és binárist tölt le: a CFA4 nem használja (AC-07).
- **Állapot a lemezen:** `LLAMA_RESUME_DIR`/`LLAMA_RESUME=0`; alapból kikapcsolva (AC-20).
- **A patchek** 10 db, a célfájlok: `ggml-cpu.c`, `ops.cpp`, `ggml.c`, `ggml.h`, `llama-context.cpp`, `qwen35.cpp`, `server-context.cpp`, `arg.cpp`, `common.h`, `chat.h`; környezeti változókkal kapcsolódnak.

## 3. Patch-átnézés és függőségi fa (git-ből, 2026-11-28 08:00 körül)
**Patchek (10 db, ~2750 sor) és kernelek (9 fájl, ~3400 sor):** kulcsszavas és célzott átnézés, nem soronkénti kódellenőrzés.
- **Nincs hálózati vagy folyamatindító hívás** a foltokban és a kernelekben (keresés: `curl`, `socket`, `connect(`, `http`, `system(`, `popen`, `exec*`, `dlopen`; csak két megjegyzés-találat a `0007`-ben).
- **Fájlrendszer:** csak a `resume` (0007, 0010) ír lemezre: `LLAMA_RESUME_DIR` alá, ideiglenes fájl + `rename`, lezáró trailer a csonka fájl ellen; olvassa a modellfájl méretét/idejét és a `/proc/self/exe` adatait az állapot azonosításához. Kikapcsolható (`LLAMA_RESUME=0`); a CFA4-worker alapból kikapcsolja (AC-20).
- **Környezeti változók:** 29 `getenv`-hívás a foltokban (`GGML_X1`, `GGML_X2`, `LLAMA_SPEC_ADAPT`, `GGML_ATTN`, `LLAMA_RESUME*` …); a worker mindet kifejezetten beállítja, a környezetből nem örököl.
- **Hatókör:** 10 llama.cpp-fájl; a számítási magot (`ggml-cpu.c`, `ops.cpp`, kernelek) és a szerver-kontextust érintik, az API-felületet és a hitelesítést nem.
- **Nyitott marad:** a kernelek numerikus helyességét a donor tesztjei (`tests/kernel_avx2.c`, `test_kernel_*.py`) állítják; mi nem ellenőriztük, a CFA4-mérés (LLM-F2/F5) igazolja.

**Függőségi fa a CFA4-buildre (`LLAMA_BUILD_UI=OFF`, `LLAMA_USE_PREBUILT_UI=OFF`, `LLAMA_CURL=OFF`, x86-64 CPU):**
| Komponens | Licenc | Megjegyzés |
|---|---|---|
| llama.cpp / ggml (fork) | MIT | |
| OpenSet-foltok és -kernelek | MIT (repó-szint) | nincs fájlonkénti fejléc |
| cpp-httplib | MIT | |
| nlohmann/json | MIT (SPDX a fájlban) | |
| sheredom/subprocess.h | Unlicense / közkincs | |
| stb_image | közkincs (vagy MIT) | |
| miniaudio | közkincs vagy MIT-0 | csak a hangbemenethez kell; a szerverhez nem |
| `vendor/hash` (sha1, sha256, xxhash, rotate-bits) | saját LICENSE-fájlok vannak; **egyenkénti elolvasás hátra van** | |
| llamafile sgemm | Mozilla Foundation, MIT-szöveg a fejlécben | |
| KleidiAI | ARM-oldali, x86-on nem épül; építéskor letöltene | `GGML_CPU_KLEIDIAI` ki |
| LLGuidance | `LLAMA_LLGUIDANCE` alapból KI; BE esetén építéskor `git clone` | ki marad |
| Web UI npm-fa | LGPL-3.0-or-later csomagokat tartalmaz | **ki van kapcsolva** a CFA4-buildben |
GPL-2.0-only, AGPL vagy zárt komponens: **nem találtam**. A fenti nyilvántartási sorokat az LLM-F4-ben visszük a `docs/licensing.md` §5-be.

## 4. Nyitott (nem a git-ből oldható meg)
1. **AVX2-mérés a referenciagépen** (Xeon E5-2696 v4): CPU-only (az NVIDIA-kártyák elrejtve), rögzített magok, korlátozott cgroup; a ~6 GB-os modellt a felhasználónak kell letöltenie (CFA4 nem tölt le), a motort a gépen kell építeni. A fejlesztés indításához nem kell (a terv 9/5), legkésőbb az LLM-F2 végéig.
2. A `vendor/hash` négy alkönyvtárának licencszövegét egyenként el kell olvasni (az LLM-F4 előtt).
3. A kernelek numerikus helyességének CFA4-oldali igazolása (mérés).
4. AVX-512-es mérés külső gépen: csak ha az AVX-512 számot hirdetni akarod (D7).
