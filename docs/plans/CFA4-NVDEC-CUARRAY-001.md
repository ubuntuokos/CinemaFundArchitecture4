# CFA4-NVDEC-CUARRAY-001 – NVDEC CUDA Array (a CFA4-FFMPEG-9.1-001 F2f része)

**Verzió:** FINAL 1.0 (jóváhagyva 2026-11-28: „jóváhagyom véglegesítsd a véglegesítés után mehet a fejlesztés”)
**Projekt:** CinemaFundArchitecture4 (CFA4)
**Előzmény:** [CFA4-FFMPEG-9.1-001](CFA4-FFMPEG-9.1-001.md) F2f és D10. A tulajdonos 2026-11-28-án
döntött a D10-ről: **„d10 kell”**, vagyis a CUDA Array-támogatás most kerüljön be, ne csak akkor,
amikor a CFA4 támogatott NVIDIA-meghajtóinak minimuma eléri a 610-et. Az F2 részei külön
jóváhagyáshoz kötöttek, ezért készült ez a terv.

---

## I. Cél

Az NVDEC a dekódolt képet CUDA Array-be (átlátszatlan, „opaque” kimenet) is adhassa a mostani
lineáris (pitch-elt) GPU-memória helyett. Ott, ahol a meghajtó tudja, ez a gyorsabb út; ahol nem,
minden marad a régiben, **a régebbi meghajtókon semmi sem romlik el**.

## II. Tények (2026-11-28)

| Téma | Megállapítás |
|------|--------------|
| SDK | A CUDA Array-kimenetet a Video Codec SDK 13.1 hozta (nv-codec-headers `n13.1.15.0`, MIT). Az FFmpeg 9.1 ennél régebbi fejlécekkel ezt a kimenetet nem fordítja be: „CUARRAY output requires Video Codec SDK 13.1 or later”. |
| Meghajtó | Az SDK 13.1 rendszerkövetelménye a 610-es vagy újabb NVIDIA-meghajtó. |
| Ubuntu | Az Ubuntu 26.04-ben az nv-codec-headers 12.1.14 van; a 13.1-et a CFA4-nek magának kell hoznia (rögzített git-tag és ellenőrzőösszeg, mint az AMF-fejléceknél). |
| FFmpeg 9.1 | `AV_PIX_FMT_CUARRAY`, a `cuviddec` `zero_copy` kapcsolója és az `nvdec` hwaccel CUDA Array-felülete (`ff_nvdec_fill_cuarray_desc`). |
| CFA4 mai NVDEC-je | Saját Rust-kötés a `crates/platform/src/nvdec`-ben (`libnvcuvid` és CUDA futáskor betöltve). H.264 és HEVC, a képet a CPU-memóriába másolja vissza (`cuvid.rs` `readback`). Ez a „meglévő NVDEC-útvonal”. |
| FFmpeg-háttér | A `cfa4-route` katalógusa szerint az FFmpeg-háttér ma CPU-n dekódol, az FFmpeg NVDEC hwaccelét a CFA4 nem használja. Az FFmpeg NVENC-jét sem (a CFA4 saját NVENC-et használ). |
| GPU-interop | A CFA4-ben ma nincs CUDA ↔ Vulkan/wgpu memóriamegosztás: minden dekódolt kép a CPU-memórián át megy tovább. |

**Következmény.** A CUDA Array önmagában, amíg a kép úgyis visszamásolódik a CPU-ra, kevés
gyorsulást ígér; a nagy nyereség a GPU-n maradó képpel (interop) jönne. Ezért a terv **mérést** tesz
az elejére, és a fejlesztést ahhoz köti.

## III. Döntendő: melyik úton (E1)

| Változat | Mit jelent | Meghajtó-hatás |
|----------|------------|----------------|
| **A (javaslat)** | A CFA4 **saját** NVDEC-je kapja meg a CUDA Array-kimenetet (saját kötés az SDK 13.1 MIT-fejlécei és a dokumentáció szerint). Futáskor dönt: a meghajtó verzióját (`cuDriverGetVersion`) és a dekóder képességét (`cuvidGetDecoderCaps`) nézi; 610 alatt vagy ha a kártya nem tudja, a mai pitch-elt út fut. Az FFmpeg a 12.1.14-es fejlécekkel épül tovább. | **Senkinél nem romlik semmi**: 610 alatt minden a mai módon megy. |
| B | Az FFmpeg épül a 13.1-es fejlécekkel, és az FFmpeg-háttér NVDEC hwaccelt kap CUDA Array-kimenettel. | Az FFmpeg NVENC/NVDEC-je **610 alatt leáll** (a router a VA-API, a Vulkan vagy a CPU útra vált); az FFmpeg-hátteret használó felhasználóknál ez visszalépés lehet. |
| C | A és B együtt. | mint B |

A javaslat az **A**: a meglévő NVDEC-útvonalon (F2f szövege) gyorsít, és a régebbi meghajtókat nem
zárja ki. A B később külön döntéssel jöhet, ha a mérés szerint az FFmpeg-háttérnek is kell.

## IV. Követelmények (az A változathoz)

| # | Követelmény | Ellenőrzés |
|---|-------------|------------|
| N1 | A CUDA Array-kimenet csak akkor fut, ha a meghajtó ≥ 610 és a dekóder képessége megvan; különben a mai út, hibaüzenet nélkül. A „never crash” szabály (AGENTS.md §0) szerint semmilyen meghajtóhiba nem állítja le az alkalmazást. | Egységtesztek a döntésre (álmeghajtó-verziókkal); a meglévő NVDEC-tesztek változatlanul zöldek. |
| N2 | Az FFI-szerkezetek bájtra egyeznek az SDK 13.1 fejléceivel. | `abi_tests.rs` bővítése (méret, mezőeltolás), mint a mostani NVDEC/NVENC-kötéseknél. |
| N3 | A kép bitre ugyanaz, mint a mai úton. | Teszt: ugyanaz a H.264/HEVC minta a két úton, bitazonos kimenet (NVIDIA-gépen; CI-ben kihagyva, a halasztott fizikai tesztek közé). |
| N4 | Mérés a referenciagépen (RTX A4000, a számítási kártya): dekódolási idő és CPU-terhelés a két úton, 1080p és 4K, H.264 és HEVC 10 bit. A meghajtó verziója a jelentésbe kerül, és a [`reference-machine.md`](../development/reference-machine.md)-be is. | Mérési jelentés a `docs/audit/` alatt; ha a fizikai mérés kimarad, sor a [`deferred-physical-tests.md`](../development/deferred-physical-tests.md) listában. |
| N5 | Látható és vezérelhető: a hardverbeállítások és a `media.backends`/hwaccel-lekérdezés mutatja, melyik NVDEC-kimenet fut; egy kapcsolóval a CUDA Array kikapcsolható (hibakereséshez). Automation id és parancs a szokásos módon. | Parancs- és GUI-teszt. |
| N6 | Clean-room: csak az NVIDIA nyilvános dokumentációja és a MIT-licencű fejlécek a forrás; FFmpeg-kódot nem másolunk. | Átnézés a PR-ban. |

## V. Fázisok

| Fázis | Tartalom |
|-------|----------|
| M0 | Mérőszkript (a meglévő `scripts/dev/audit-ffmpeg-hwaccel.sh` mellé vagy bővítve): a mai NVDEC-út mérése a referenciagépen, a meghajtó verziójának rögzítése. |
| M1 | FFI és döntés (N1, N2): SDK 13.1-es szerkezetek, verzió- és képességvizsgálat, a CUDA Array-kimenet és a visszamásolás. |
| M2 | Mérés a két úton (N3, N4); ha a CUDA Array nem gyorsabb, a kód kapcsolóval kikapcsolt állapotban marad, és ezt a jelentés rögzíti. |
| M3 | Beállítás, lekérdezés, Qt6 GUI (N5), dokumentáció (`crates/platform` README, ADR 0003 sor, `codec-parity.md`). |

## VI. Döntések

| # | Kérdés | Javaslat | Döntés |
|---|--------|----------|--------|
| D10 | Mikor kerüljön be a CUDA Array? | – | **most** (tulajdonos, 2026-11-28: „d10 kell”) |
| E1 | Melyik úton? (III.) | **A**: a saját NVDEC kapja meg, futásidejű meghajtóvizsgálattal; az FFmpeg marad a 12.1.14-es fejléceken | **A** (tulajdonos, 2026-11-28) |
| E2 | Ha a mérés szerint a CUDA Array visszamásolással nem gyorsabb: maradjon kikapcsolva, és a GPU-interop (CUDA ↔ Vulkan) kerüljön külön tervbe? | igen | **igen** (tulajdonos, 2026-11-28) |

## VII. PR-bontás

| PR | Tartalom | CodeRabbit (§2.9) |
|----|----------|-------------------|
| P0 | Ez a terv, és a CFA4-FFMPEG-9.1-001 D10 sorának frissítése | nem |
| P1 | M0 + M1 | igen (platform/FFI) |
| P2 | M2 jelentés + M3 | igen |

**Megvalósítás (2026-11-28).** P1: M0 + M1 (#1956). P2 előre az M3-mal (a mérési jelentés nélkül, mert
az a referenciagépen fut): beállítás `playback.nvdecCudaArray` (Settings ▸ Media & Playback ▸ Decoding &
Encoding a Qt6 GUI-ban, automation id `settings.playback.nvdecCudaArray`), a `perf.stats`
`decode.hardware.nvdecCudaArray` (`enabled`, `frames`), fordítások. Az alapérték a mérésig ki; a mérés
(N3, N4) után külön lépés dönt az alapértékről (E2).
