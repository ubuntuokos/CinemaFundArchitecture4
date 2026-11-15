# CFA4 helyi AI-modellképzés, finomhangolás és tanulás (CFA4-AI-MODEL-TRAINING-001)

> **Állapot:** TERVEZET 1.1, a tulajdonosi döntések beépítve (István, 2026-11-07); IMPLEMENTÁCIÓ
> NINCS ENGEDÉLYEZVE · **Utolsó átnézés:** 2026-11-07 · **Változás:** az 1.0 vázlat (István,
> 2026-11-07) igazítása a CFA4 AI-tervéhez · **Platform:** Ubuntu 26.04, amd64 · **Kapcsolódik:**
> [CFA4-AI_modules ADR 0008](https://github.com/ubuntuokos/CFA4-AI_modules/blob/main/docs/adr/ADR-0008-ai-processing-layer.md)
> (AI-feldolgozó réteg, F0–F6 kapuk), [`CFA4-TASKSTORE-PG-001`](CFA4-TASKSTORE-PG-001.md)
> (PostgreSQL-adatréteg, `mi` séma), CFA4-ADAPTIVE-CAPABILITY-001 (ACS, tulajdonosi specifikáció,
> 2026-11-07; [összevetése](CFA4-ADAPTIVE-CAPABILITY-001-OSSZEVETES.md)), [`CFA4-EVENT-FABRIC-001`](CFA4-EVENT-FABRIC-001-1.0.md),
> [`ADR 0006`](../adr/ADR-0006-backlothub-postgresql.md) (BacklotHub PostgreSQL),
> [CFA4-DEV-AI-ORCHESTRATION-001](../development/ai-orchestration/README.md) · **Donor:**
> [FareedKhan-dev/train-llm-from-scratch](https://github.com/FareedKhan-dev/train-llm-from-scratch) (MIT)

## 0. A tulajdonosi döntések (2026-11-07)

| # | Kérdés | Döntés |
| --- | --- | --- |
| D1 | A 12 GB VRAM / 16 GB RAM alsó határ a tanításra is érvényes? | Igen. A 8 GB-os profil kikerül. |
| D2 | CPU-s tanítás csak adat-előkészítésre és CI-tesztmodellre? | Igen. Nincs GPU nélküli felhasználói tanítási út. |
| D3 | Adat- és modelllicenc: rögzítés és megjelenítés tiltás helyett? | Igen, ugyanúgy, mint a letöltött modelleknél. |
| D4 | A tanító worker az AI-terv F6 közös uv-környezetét bővíti? | Ellenőrizve (13. fejezet): igen, külön környezet nem kell. |
| D5 | Media Intelligence hivatkozás | A CFA4-MEDIA-INTELLIGENCE-001 FINAL 2.0 nincs a tárolókban; a hivatkozás a [`CFA4-TASKSTORE-PG-001`](CFA4-TASKSTORE-PG-001.md) 4. pontjának `mi` sémája. |
| D6 | Hová kerül? | Ide, a CFA4 `docs/plans` mappájába; az ADR 0008 egy hivatkozó sort kap. |

## 1. Cél és szerep

A CFA4 alkalmazás (nem a fejlesztési AI-koordináció) helyi képzési és adaptációs képességet kap:
kis modellek nulláról tanítása, meglévő modellek finomhangolása (SFT, LoRA), preferenciaalapú
tanulás és ellenőrizhető visszacsatolásos tanulás (RLVR). Ez a saját tanítású modellek egyetlen
helye: az AI-réteg funkcióihoz (például a #91 felskálázáshoz) nem tanítunk saját modellt (ADR 0008).

| Képesség | Feladat | Hol |
| --- | --- | --- |
| AI Inference | Meglévő modellek futtatása | ADR 0008 (F2–F6) |
| AI Adaptation | Meglévő modell célfeladatra alakítása (SFT, LoRA, preferencia) | Ez a terv |
| AI Training | Új modell tanítása, továbbtanítás | Ez a terv |

A rendszer **nem hoz létre párhuzamos AI-infrastruktúrát**. A modellkezelés, a feladatsor, az
adatbázis, a hardverfelismerés és az eseménykommunikáció a már elfogadott elemeké; ez a terv csak
a tanítási folyamatot, a tanítóadat-kezelést és a kiértékelést teszi hozzá.

## 2. Kötelező szabályok

1. Local-first: hálózati tanítás és kötelező felhőszolgáltatás nincs. Kísérletnaplózás csak helyben
   (a donor `wandb` függősége nem jön át).
2. **Modellt, tokenizert, adatkészletet a CFA4 nem tölt le és nem terjeszt.** A felhasználó választja
   ki és tölti le; a gazda letöltője csak az ő kérésére, közvetlenül a forrásból tölt (licencszabály,
   2026-11-07). A tanított modell és adapter a felhasználó gépén marad, a CFA4 nem terjeszti.
3. Egyetlen modellnyilvántartás: a `cfa4-ai-models` (ADR 0008 2. pont). A tanított modell és adapter
   ide kerül be, saját nyilvántartás nincs. A modellcache közös az AI Video, AI Audio és a Media
   Intelligence között.
4. Egyetlen adatbázis: a közös CFA4 PostgreSQL-klaszter (`CFA4-TASKSTORE-PG-001`), új `train`
   sémával. Súly, checkpoint, adatkészlet fájlrendszeren; a DB-ben csak hivatkozás, SHA-256,
   metaadat és állapot.
5. Feladatkezelés a közös feladatsorban (`tasks` séma, `cfa4-store`, `cfa4-ai-jobs`); nincs második
   sor vagy ütemező.
6. Hardver és erőforrás az ACS-től; a tanítás az ACS kliense (4. fejezet).
7. Alsó határ (D1): NVIDIA Ampere vagy újabb, legalább 12 GB VRAM (referencia: RTX 3060 12 GB),
   legalább 16 GB RAM, ajánlott 32 GB. Ez alatt a tanítási funkciók nem érhetők el.
8. CPU (D2): adat-előkészítés, tokenizálás, kiértékelés egy része és a CI apró tesztmodelljei
   futnak CPU-n. Felhasználói tanítás csak GPU-val.
9. Nincs néma visszalépés: az automatikus konfigurációmódosítás naplózott, az eredeti konfiguráció
   megmarad (ADR 0008, ACS).
10. Az interaktív szerkesztés elsőbbséget élvez; a tanítás felfüggeszthető és checkpointból folytatható.
11. Modell vagy adapter csak független kiértékelés és a felhasználó jóváhagyása után használható;
    a regisztráció nem aktivál.
12. Natív Qt6 felület (nincs Streamlit); csak Ubuntu 26.04, amd64; Conda és Miniforge nem.
13. Soha ne omoljon össze ([`AGENTS.md`](../../AGENTS.md) §0): a Rust Training Core ugyanazokat a
    szabályokat követi, a Python worker hibája a worker-folyamatban marad.

## 3. Architektúra

```
Qt6 AI Model Studio (a meglévő Qt6 AI-felület része)
        │ cfa4-cli serve / vezérlőcsatorna (parancsok, automation id-k)
Rust Training Core: cfa4-ai-train (L2)
        │ állapot, checkpoint-nyilvántartás, train séma, jóváhagyás
        ├── cfa4-ai-jobs / cfa4-store ── tasks séma (feladat, lease, ütemezés)
        ├── cfa4-ai-models ───────────── modellnyilvántartás, közös cache
        ├── ACS (accel crate alapon) ─── hardverfelismerés, foglalás, monitor
        └── Event Fabric (outbox) ────── cfa4.execution.job.* események
        │ SDK worker-protokoll (CFA4-sdk, verziózott)
Python workerek (GPL, a közös F6 uv-környezetben)
        training · adaptation · preference · rl · evaluation · dataset
```

| Komponens | Felelősség | Megvalósítás |
| --- | --- | --- |
| AI Training Manager | Képzési feladatok életciklusa | `cfa4-ai-train` |
| Training Orchestrator | Ütemezés, vezérlés | a közös feladatsor (`cfa4-ai-jobs`), nem új ütemező |
| Model Compatibility Layer | Architektúra, tokenizer, súlyformátum, képességek | `cfa4-ai-train` + `cfa4-ai-models` metaadat |
| Training Engine | Pretraining, SFT | Python worker |
| Adaptation Engine | LoRA, PEFT | Python worker (peft) |
| Preference Engine | DPO és társai | Python worker |
| RL Engine | PPO, GRPO, RLVR | Python worker |
| Verifier Engine | Ellenőrizhető jutalom | a meglévő CFA4-ellenőrzők hívása (JSON Schema, feliratformátum, Tick-időkód, parancsszintaxis) |
| Dataset Governance | Adatellenőrzés, verzió, proveniencia | dataset worker + `train` séma |
| Adaptive Training Engine | Hardverhez igazított tanítási profil | ACS-kliens (4. fejezet) |
| Evaluation Engine | Benchmark, regresszió | evaluation worker |
| Experiment Tracker | Kísérletek dokumentálása | `train` séma, helyi |
| Checkpoint Manager | Atomi mentés, visszaállítás | `cfa4-ai-train` |

**Rust és Python munkamegosztása.** Rust: állapot, erőforrás-foglalás (az ACS-en át), felügyelet,
leállítás, jogosultság, checkpoint-nyilvántartás, Event Fabric, PostgreSQL, a Qt6 felület
kiszolgálása. Python: PyTorch-modellek, tokenizálás, a tanító- és kiértékelő algoritmusok. A Python
nem lehet a vezérlőréteg. A kommunikáció az SDK verziózott worker-protokollja; a worker önállóan
újraindítható és leállítható.

## 4. Adaptive Training Engine (ACS-kliens)

A mérés (CPU, utasításkészlet, NUMA, RAM, GPU, VRAM, architektúra, backend, terhelés, tárhely) az
ACS-é, amely az `accel` crate meglévő hardverleltárára épül. A tanítás nem ír saját
hardverfelismerést; a donor eszközfelismerése nem jön át.

A tanítási profil a tanításé. Az ACS által jelentett szabad erőforrásra igazítja:

| Paraméter | Alkalmazkodás |
| --- | --- |
| Batch size | Csökkentés |
| Gradient accumulation | Növelés (azonos effektív batch) |
| Context length | Csökkentés, csak a felhasználó jóváhagyásával |
| Precision | Támogatott pontosság (bf16 Ampere-en) |
| Gradient checkpointing | Bekapcsolás |
| Worker concurrency, CPU threads, prefetch | Korlátozás |
| Checkpoint frequency | Módosítás |
| Model size | Kisebb konfiguráció ajánlása, nem automatikus csere |

Minden módosítás naplózott, az eredeti konfiguráció megmarad. Ami a felhasználó célját változtatja
(kontextushossz, modellméret), az csak jóváhagyással.

**Referenciagépek** (a mérés a [`reference-machine.md`](../development/reference-machine.md)
szabályai szerint, a `run-minspec.sh` mindkét profiljával):

- Fejlesztői gép: 2 × Xeon E5-2696 v4, 512 GiB RAM; számítás RTX A4000 16 GB-on (az RTX A1000 csak
  a kijelzőt hajtja).
- Minimum-referencia: RTX 3060 12 GB, `CFA4_PROFILE=minimum` (16 GB RAM) és `ajanlott` (32 GB).

**Erőforrás-prioritás.** A tanítás nem foglalhatja korlátlanul a GPU-t; a VRAM-ot folyamatosan
figyeljük; a háttérfeladat felfüggeszthető és checkpointból folytatható; 12 GB-os kártyán egyszerre
egy nagy tanítás fut; a felszabadítás ellenőrzött. A 12 GB sem jelent automatikusan lehetőséget
minden 1 milliárd paraméter alatti modell tanítására: az igényt az architektúra, a szekvenciahossz,
az aktivációk, az optimizer és a batch együtt határozza meg, ezt indítás előtt becsüljük és mérjük.

## 5. Képességek

**5.1 Nulláról tanítás.** Klasszikus és modern decoder-only Transformer, RoPE, RMSNorm, SwiGLU, GQA,
MLA, MoE, KV-cache, BPE, pretraining és checkpointolás. Cél: kis és közepes, saját feladatra
optimalizált modellek, nem nagy LLM-ek.

**5.2 SFT.** Chat template, csak az asszisztensválaszra számolt veszteség, adatkészlet-ellenőrzés,
folytatás checkpointból, külön tanító és validációs eredmény. Célok: CFA4-parancsok megértése,
magyar nyelvű médiakeresés, metaadat-strukturálás, feliratozás, szerkesztési munkafolyamatok,
AI-eszközök természetes nyelvű vezérlése.

**5.3 LoRA (elsődleges adaptáció).** Rank, alpha, dropout, célrétegek, adapterenkénti tanítás és
kiértékelés, verzió, aktiválás és kikapcsolás, összevonás ahol megengedett; az alapmodell
változatlan. Nem kötődik a donor saját Transformeréhez: külső modelleket is adapterez (peft).

**5.4 Preferenciaalapú tanulás.** DPO, IPO, SimPO, ORPO, KTO (TR-F6); PPO, GRPO, Dr. GRPO, DAPO,
GSPO (TR-F7). Nem mind az első kiadásban; a választás a feladaton, az adaton, az erőforráson és a
mérésen múlik.

**5.5 RLVR.** Ellenőrizhető eredményre épülő tanulás: érvényes JSON, metaadat-szerkezet, időkód,
felirat, médiaparaméter, parancsszintaxis, munkafolyamat. A Verifier Engine elkülönített, tesztelt
komponens, a meglévő CFA4-ellenőrzőket hívja; a tanított modell nem írhatja felül.

## 6. PostgreSQL: a `train` séma

A közös klaszterben (`CFA4-TASKSTORE-PG-001` 2. pont) új `train` séma, ugyanazokkal a szabályokkal
(UUIDv7, migrációk a `cfa4-store`-ban, outbox az Event Fabric felé).

| Tábla | Tartalom | Megjegyzés |
| --- | --- | --- |
| `train.job` | Képzési feladat | 1:1 egy `tasks.task` sorral; az állapot ott él |
| `train.run` | Egy konkrét futtatás | job-onként több |
| `train.config` | Paraméterek, az eredeti és az igazított változat | a módosítások naplója |
| `train.dataset`, `train.dataset_version` | Adatkészlet és verziója | fájlhivatkozás, SHA-256 |
| `train.checkpoint` | Checkpoint helye, SHA-256, érvényesség | csak atomi mentés után érvényes |
| `train.adapter` | Adapter, alapmodellje, verziója | a `cfa4-ai-models` hivatkozása |
| `train.eval_run`, `train.eval_result` | Kiértékelés és eredménye | alapmodell és adapter külön |
| `train.experiment` | Kísérleti csoport | |
| `train.approval` | Jóváhagyási állapot | a modell nem módosíthatja |
| `train.provenance` | Eredet és licencinformáció | rögzítés, nem tiltás (D3) |

A modell- és alapmodell-hivatkozás a `cfa4-ai-models` nyilvántartására mutat; a hardverprofil és az
erőforrás-foglalás az ACS-é, itt csak hivatkozás van rájuk.

## 7. Dataset Governance

**Források:** helyi szöveg, JSONL, strukturált beszélgetés, CSV, HDF5, feliratfájl, média-metaadat,
jóváhagyott saját gyűjtemény. A multimodális adat külön útvonalat kap.

**Ellenőrzés:** formátum, kódolás, hiányzó adat, duplikáció, hibás rekord, forrás, licencinformáció,
személyes adat, tanító/validációs/teszt felosztás, verzió, SHA-256.

**Adatvédelem:** külső adatküldés nincs; a helyi adat csak a felhasználó kifejezett engedélyével
hagyhatja el a gépet; a személyes adatot külön jelezzük; az előkészítés bármikor megszakítható; a
forrás utólag visszakereshető.

**Licenc (D3).** A kód és a függőségek licence a GPL-szabály szerint kapu (`cargo deny`, Python-
függőségek a [`docs/licensing.md`](../licensing.md) szerint). A tanítóadat és az alapmodell licence a
felhasználó felelőssége: a rendszer rögzíti (`train.provenance`), megmutatja, és egyszeri
elfogadást kér, de funkciót nem tilt. A tanított modell a felhasználó gépén marad.

## 8. Modellkompatibilitás és életciklus

| Formátum | Szerep |
| --- | --- |
| safetensors | Tárolt súly, checkpoint és csereformátum; a modellnyilvántartásba csak ez kerül |
| PyTorch `state_dict` (pickle) | Csak a worker memóriájában; fájlból `torch.load(weights_only=True)`, az ADR 0008 PyTorch-kapus szabálya szerint |
| Hugging Face modellkönyvtár | Külső modellek betöltése (config, tokenizer, safetensors) |
| LoRA adapter | safetensors + adapterkonfiguráció |
| GGUF | Csak inferenciaoldali kapcsolat |

Az optimizer-állapot külön fájl. A rendszer modellenként jelzi: betölthető, futtatható,
finomhangolható, adapterezhető, teljesen továbbtanítható, exportálható.

**Életciklus:** kiválasztás a `cfa4-ai-models`-ből → kompatibilitás és licencinformáció →
erőforrásbecslés (ACS) → konfiguráció → adatkészlet-ellenőrzés → tanítás és checkpoint → független
kiértékelés → jóváhagyás és verzió → regisztráció a `cfa4-ai-models`-be → a felhasználó dönt a
használatba vételről.

## 9. Qt6 AI Model Studio

A meglévő Qt6 AI-felület része, a Qt6 migráció befejezése után (az egui kivezetése előbb jön). Minden
vezérlő automation id-t kap, minden művelet parancs (`crates/engine`), így a vezérlőcsatorna és az MCP
is eléri. Részek: Model Training, Model Adaptation, Preference Learning, Reinforcement Learning,
Dataset Manager, Hardware Monitor (az ACS adatai), Training Monitor, Experiment Manager, Evaluation
Center, Checkpoint Manager, Model Registry (a `cfa4-ai-models` nézete).

**Élő felügyelet:** training és validation loss, learning rate, gradient norm, tokens/s, batch-idő,
CPU, RAM, GPU, VRAM, becsült hátralévő idő (nem garancia), checkpointállapot, fázis.

**Vezérlés:** Pause, Resume, Stop, Cancel, Save checkpoint, Resume from checkpoint, Export
configuration. A leállítás nem hagyhat érvényesnek jelölt félkész checkpointot. A `paused` állapot
felvétele az ADR 0008 állapotlistájába az ACS-javaslattal együtt dől el; addig a Pause = checkpoint
+ leállítás, a Resume = új futtatás ugyanabból a checkpointból.

## 10. Kapcsolat a CFA4 AI-funkcióival

- **Media Intelligence** (`mi` séma, pgvector): magyar lekérdezések értelmezése, metaadat-
  normalizálás, jelenetleírás, keresési szándék, strukturált keresési paraméterek. Az embeddingmodell
  és az LLM két külön család; az architektúra (PostgreSQL + pgvector) változatlan.
- **AI Video Tools:** metaadat-értelmezés, szerkesztési parancsok, jelenetleírás. Vizuális
  felismeréshez képi vagy videós modell kell, nem szöveges LLM.
- **AI Audio Tools:** parancsok, metaadat, átiratfeldolgozás, hangesemény-leírás. Az ASR és a TTS
  külön architektúra, nem a szöveges LLM korlátai közé szorítva.
- **Fejlesztési AI-koordináció** (CFA4-DEV-AI-ORCHESTRATION-001): tréningtervet, konfigurációt
  javasolhat, eredményt elemezhet, de tanítást csak az engedélyezett CFA4-folyamat indít; modellt nem
  tölt le, felhasználói adatot nem küld fel, jóváhagyás nélkül nem aktivál.

## 11. Kapuk (TR-F0…TR-F10)

Az előtag megkülönbözteti az AI-terv F0–F6 és az ACS kapuitól. A tanítás az AI-réteg késői ága.

| Kapu | Tartalom | Előfeltétel | Kilépés |
| --- | --- | --- | --- |
| TR-F0 | Audit: donorkód és licenc, Python 3.14 / torch kompatibilitás, igazítás az AI-tervhez | — | Ez a dokumentum, elfogadva |
| TR-F1 | `cfa4-ai-train`, worker-protokoll, `train` séma, checkpoint-nyilvántartás | AI-F1 (modellkezelő), AI-F2 (PG-2, PG-3) | Apró tesztmodell CPU-n a CI-ben, megszakítás és folytatás |
| TR-F2 | Adaptive Training Engine mint ACS-kliens | ACS-1…ACS-3 | Tanítás 12 GB-os GPU-n mindkét RAM-profilban összeomlás nélkül, reprodukálhatóan |
| TR-F3 | Dataset Governance | AI-F2 | Visszakövethető adatkészlet-verzió |
| TR-F4 | Kompatibilitási réteg, kis modell előtanítása | AI-F6 (közös PyTorch) | Igazolt kis modell RTX 3060 12 GB-on |
| TR-F5 | SFT, LoRA, adapterverziók | TR-F4 | Alapmodellhez kötött, külön értékelt adapter |
| TR-F6 | DPO, IPO, SimPO, ORPO, KTO | TR-F5 | Ellenőrzött preferenciaillesztés |
| TR-F7 | PPO, GRPO, Dr. GRPO, DAPO, GSPO, RLVR | TR-F5 | Ellenőrizhető jutalom, regressziómérés |
| TR-F8 | Evaluation, regisztráció | TR-F5 | Reprodukálható, jóváhagyható modellkiadás |
| TR-F9 | Qt6 AI Model Studio | Qt6 migráció kész, egui ki | GUI–Core–Worker együttműködés |
| TR-F10 | Integráció: Media Intelligence, AI Video, AI Audio, Event Fabric | AI-F4/F5 (PG-4) | Végponttól végpontig teszt |

TR-F0…TR-F3 lezárása előtt nagyobb tanítóalgoritmus nem integrálható.

## 12. Donor: átvétel

| Donor képessége | CFA4-kezelés |
| --- | --- |
| Classic és Modern Transformer, BPE, pretraining, SFT, LoRA | Adaptálás |
| Reward model, DPO, IPO, SimPO, ORPO, KTO, PPO, GRPO, Dr. GRPO | Adaptálás |
| DAPO, GSPO | Kísérleti adaptáció |
| Muon optimizer | Vizsgálat és teszt |
| Speculative decoding, INT8 | Inferenciaoldali vizsgálat (ADR 0008) |
| Benchmark | Kibővítés |
| Checkpoint-kezelés | Átírás safetensorsra, atomi mentéssel |
| Eszközfelismerés | Nem jön át: ACS |
| Adatfeldolgozás | A Dataset Governance alá |
| Streamlit UI, `wandb` | Nem jön át |

Saját fejlesztés: `cfa4-ai-train`, ACS-kliens tanítási profil, Dataset Governance, kompatibilitási
réteg, `cfa4-ai-models`-integráció, Verifier Engine, Experiment Tracker, Qt6 AI Model Studio,
worker-kommunikáció, CFA4-specifikus kiértékelés. A donor MIT-licencű: a GPL-3.0-val összeegyeztethető,
az átvett fájlok fejlécét és a nyilvántartási sort a [`docs/licensing.md`](../licensing.md) §3, §5
szerint megtartjuk.

## 13. Függőségek: a közös F6 PyTorch-környezet (D4 ellenőrzése, 2026-11-07)

Az ADR 0008 F6 egyetlen, uv-vel rögzített PyTorch-környezetet ír elő egy CUDA 12.x-szel; külön
környezet csak írásos indoklással. Ellenőrzés a donor `pyproject.toml`-ja és a PyPI alapján
(2026-11-07):

| Csomag | Legújabb | Licenc | Python 3.14, Linux x86_64 | Döntés |
| --- | --- | --- | --- | --- |
| torch | 2.15.0 | BSD-3-Clause (a PyPI-metaadat szerint további engedékeny licencekkel) | cp314 kerék van | F6 környezet, CUDA 12.x build |
| numpy | 2.5.4 | BSD-3-Clause | cp314 | Átvesszük |
| h5py | 3.16.0 | BSD-3-Clause | cp314 | Átvesszük (HDF5 adatkészlet) |
| tiktoken | 0.14.0 | MIT | cp314 | Csak helyi kódolásfájllal (lásd lejjebb) |
| zstandard | 0.25.0 | BSD-3-Clause | cp314 | Átvesszük |
| regex | 2026.11.7 | Apache-2.0 | cp314 | Átvesszük |
| jaxtyping | 0.3.11 | MIT | tiszta Python | Átvesszük |
| peft | 0.21.2 | Apache-2.0 | tiszta Python | Új (LoRA külső modellekhez) |
| safetensors | 0.8.0 | Apache-2.0 | abi3 kerék (3.14 alatt mérendő) | Új (tárolási formátum) |
| tokenizers | 0.23.3 | Apache-2.0 | cp314 | Új (Hugging Face tokenizerek) |
| datasets | 5.1.0 | Apache-2.0 | tiszta Python | Csak helyi fájlokkal, offline módban |
| requests | — | — | — | Nem jön át (a tanításnak nincs hálózati útja) |
| wandb, streamlit, pandas, altair | — | — | — | Nem jön át |

**Eredmény:** külön környezet nem kell; a tanítás az F6 környezetet bővíti a fenti csomagokkal.
Feltételek:

- A donor `requirements.txt` CUDA 11.8-as indexe (`cu118`) nem jön át; a torch az F6 CUDA 12.x
  buildje. A pontos CUDA-változatot és a kerék működését az F6 kapu az RTX A4000-en és a 3060-on méri
  (a PyTorch saját indexe ebből a környezetből nem volt elérhető).
- A `tiktoken` első használatkor letöltené a BPE-kódolásfájlt. Offline működéshez a kódolásfájlt a
  felhasználó adja meg (vagy a donor saját BPE-tokenizere fut); automatikus letöltés nincs.
- A `datasets` és a `tokenizers` hálózati elérése ki van kapcsolva (`HF_HUB_OFFLINE=1`,
  `HF_DATASETS_OFFLINE=1`); csak helyi fájl.
- A Python-változat: a fejlesztői gépen 3.14.4, a CFA4 CI 3.13-at futtat. Az F6 környezet egy
  interpretert rögzít; mivel minden fenti csomag ad 3.14-es kereket, a javaslat a 3.14, a döntés az F6
  kapué.
- Conda és Miniforge nem; a rendszer-Pythonba nem telepítünk.

## 14. Biztonság és minőség

**Tanítás:** a tanítóadat nem futtat kódot; a modellkimenet nem megbízható parancs; checkpoint
betöltésekor SHA-256 ellenőrzés; a worker korlátozott jogosultsággal fut; külső adatforrás csak
engedéllyel és naplózva; megszakítás nem okoz adatvesztést; a modell nem módosíthatja a saját
jóváhagyását és nem aktiválhatja önmagát.

**Tesztek komponensenként:** unit, integration, CPU (CI-tesztmodell), GPU (referenciagépen),
memóriaterhelés a `run-minspec.sh` mindkét profiljával, checkpoint-helyreállítás, reprodukálhatóság,
adatvalidáció, hibás bemenet, hosszú futású stabilitás. Minden összeomlás-javítás regressziós tesztet
kap.

**Kiértékelési kapu** (modell nem kerül automatikusan használatba): javulás az alapmodellhez képest,
magyar nyelvi minőség, feladatspecifikus pontosság, hallucinációs kockázat, memóriaigény
hardverprofilonként, inferenciasebesség, regresszió, proveniencia, licencinformáció, integritás. Az
adaptert és az alapmodellt külön mérjük.

## 15. Kockázatok

| Kockázat | Következmény | Megelőzés |
| --- | --- | --- |
| Túl nagy modell indítása | VRAM/RAM kimerülés | Becslés és ACS-foglalás indítás előtt |
| Hibás checkpoint | Modellvesztés | Atomi mentés, SHA-256 |
| Rossz tanítóadat | Modellromlás | Dataset Governance |
| Catastrophic forgetting | Korábbi képességek elvesztése | Regressziós benchmark |
| Hibás jutalom | Nem kívánt viselkedés | Független Verifier Engine |
| Nem kompatibilis adapter | Betöltési hiba | Kompatibilitási réteg |
| Túlzott háttérterhelés | Lassú szerkesztés | Interaktív elsőbbség, felfüggesztés |
| Python-függőségütközés | Instabil worker | Egyetlen rögzített F6 környezet, külön folyamat |
| Hálózati hozzáférés egy függőségen át | Adatkiszivárgás, letöltés | Offline kapcsolók, hálózat nélküli worker-teszt |

## 16. Első használható kiadás és elfogadási feltételek

**Első kiadás:** `cfa4-ai-train`, izolált Python worker, CI-s CPU-tesztmodell, NVIDIA GPU-s tanítás
12 GB-tól, kis Transformer nulláról, SFT, LoRA, checkpoint és visszaállítás, Dataset Governance
alapjai, hardverhez igazított profil, alap Evaluation Engine, `cfa4-ai-models`-integráció, Qt6
felügyeleti felület, teljesen helyi működés (TR-F0…TR-F5, TR-F8 alapjai, TR-F9). Nem első kiadás:
teljes méretű LLM nulláról, elosztott tanítás, multimodális pretraining, minden RL-algoritmus,
automatikus modellgenerálás és -telepítés, felhős tanítás.

| Azonosító | Feltétel |
| --- | --- |
| AC-01 | A CPU-s teszttanítás (apró modell) a CI-ben fut |
| AC-02 | NVIDIA GPU-s tanítás működik 12 GB-os kártyán |
| AC-03 | A hardverkorlátok betartása igazolt, mindkét RAM-profilban |
| AC-04 | Kis modell nulláról tanítható |
| AC-05 | SFT működik |
| AC-06 | LoRA adapter létrehozható |
| AC-07 | Adapter visszaállítható és külön értékelhető |
| AC-08 | Checkpointból folytatható a tanítás |
| AC-09 | Adatkészlet-verziózás működik |
| AC-10 | Licenc- és eredetinformáció visszakövethető (rögzítve, nem tiltva) |
| AC-11 | A tanítás megszakítható |
| AC-12 | A szerkesztő interaktív teljesítménye védett |
| AC-13 | A kiértékelés reprodukálható |
| AC-14 | PostgreSQL-integráció (`train` séma, `tasks` sor) működik |
| AC-15 | `cfa4-ai-models`-integráció működik |
| AC-16 | A Qt6 felügyeleti felület működik |
| AC-17 | Külső adatküldés nincs felhasználói engedély nélkül |
| AC-18 | Nincs automatikus modell-, tokenizer- vagy adatkészlet-letöltés |
| AC-19 | Az AI Video és AI Audio funkciókkal kompatibilis |
| AC-20 | Ubuntu 26.04 amd64 alatt validált, a min-spec mérés mindkét profillal lefutott |
