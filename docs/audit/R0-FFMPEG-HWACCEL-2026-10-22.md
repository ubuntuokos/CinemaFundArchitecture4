# CFA4 R0 / FFmpeg hardveres médiaútvonalak – Ubuntu 26.04.1 / AMD64

**Munkacsomag:** [R0 #14](https://github.com/ubuntuokos/CinemaFundArchitecture4/issues/14)  
**Bázis:** [GPU compute hardveraudit #12](https://github.com/ubuntuokos/CinemaFundArchitecture4/issues/12), önálló draft PR #13; ennek a hardveres FFmpeg-harnessnek az eredménye **NOT_RUN**, amíg a helyi mérést nem kapjuk meg.  
**Kiadási döntés:** HOLD (a sikeres gyorsítási teszt sem jogi/SBOM/Qt6 GUI tanúsítvány).

## Miért szükséges önálló teszt?

A WGPU WGSL compute-effektek 12/12 sikeres RTX A1000 tesztje **nem** bizonyít NVENC/NVDEC hardveres videókódolást vagy FFmpeg Vulkan/OpenCL szűrést. Az FFmpeg `-encoders` lista **csak a fordított bináris képességeit** mutatja, a fizikai GPU támogatását nem. Az FFmpeg 9.0.2 CI-orákulum a `scripts/ci/install-ffmpeg-9.0.2.sh` külön, statikus, oracle-célú buildje; nem azonos a helyi, NVIDIA-val bővített, dinamikusan telepített FFmpeg-környezet valamennyi opciójával.

Az NVIDIA RTX A1000 Ampere generáció **AV1 dekódolásra képes, AV1 NVENC kódolásra nem**. Az `av1_nvenc` FFmpeg encoder jelenléte ettől még lehetséges; a harness ezt kifejezetten `UNSUPPORTED_HARDWARE` állapotként kezeli, és AV1 NVDEC-hez külön CPU-val generált mintát használ.

## Helyi futtatás – nincs telepítés vagy Rust-fordítás

A **helyi Ubuntu 26.04 AMD64** gépen, az audit-PR checkoutja után:

```bash
cd ~/GIT/CinemaFundArchitecture4
git fetch origin audit/r0-ffmpeg-hwaccel
git switch --track origin/audit/r0-ffmpeg-hwaccel

# FFmpeg 9.0.2 és ffprobe 9.0.2 szükséges. Alapértelmezetten PATH:
bash scripts/dev/audit-ffmpeg-hwaccel.sh
```

Ha másik, külön telepített FFmpeg/ffprobe binárist kell vizsgálni:

```bash
CFA4_FFMPEG_BIN=/opt/ffmpeg-9.0.2/bin/ffmpeg \
CFA4_FFPROBE_BIN=/opt/ffmpeg-9.0.2/bin/ffprobe \
bash scripts/dev/audit-ffmpeg-hwaccel.sh
```

A parancs **nem** telepít csomagot, nem indít GitHub Actions feladatot, nem futtatja a teljes Rust workspace-et, és nem tölti fel automatikusan a hardveradatokat. A szintetikus videó mindössze **8 képkocka 640×360 méretben, 8 fps**. Az első 128×72-es minta kisebb volt az RTX A1000 NVENC encoderének, illetve AV1 NVDEC dekóderének támogatott minimumánál; a felbontás javítása tesztadat-korrekció, nem GPU-driver-/codec-fix.

## Második fizikai NVIDIA-mérés és izolált libplacebo-utánellenőrzés

A felhasználó a `0f81b74f203ba3329059009c2efc507a70531a61`
commiton, `20261022T153037Z` azonosítóval **24 PASS**, **1 FAIL**
(`libplacebo`), **1 BLOCKED** (`libplacebo_framecheck`) és az RTX A1000
miatt elvárt `AV1_NVENC=UNSUPPORTED_HARDWARE` eredményt mért. Minden
NVENC/NVDEC (H.264, HEVC, AV1), CPU-fallback, `scale_vulkan` és
`avgblur_opencl` ellenőrzés sikeres. A `libplacebo` naplóban az NVIDIA
Vulkan eszköz inicializálása sikeres volt, de közvetlenül az
`Initialized libplacebo v7.360.0 (API v360)` üzenet után a filtergraph
`Invalid argument` hibával megállt, mielőtt akár egy képkocka kimenetre
jutott volna.

Az FFmpeg 9.0 forráskódjának `vf_libplacebo.c` Vulkan-importútvonala,
konkrétan a `copy_pl_queue()` megvizsgálása olyan **lehetséges**
kompatibilitási okot mutatott, amelynél `PL_API_VER < 365` és nem nulla
`AVVulkanDeviceContext.queue_flags` esetén a függvény **külön logolás
nélkül `EINVAL`-t ad vissza**.
Forrás: https://www.ffmpeg.org/doxygen/9.0/vf__libplacebo_8c_source.html
(`copy_pl_queue` függvény, 653–666. sor). A felhasználó libplacebo
API-ja v360; az adott rendszer tényleges `queue_flags` értéke viszont
nem volt kimérve, tehát ezt egyelőre **hipotézisnek** kell kezelni.

A [PR #15](https://github.com/ubuntuokos/CinemaFundArchitecture4/pull/15)
a `libplacebo` tesztből **elhagyta az FFmpeg külső Vulkan-eszközének
`-filter_hw_device vk` importját**, hogy a libplacebo saját Vulkan
eszközt hozhasson létre. A GPU bizonyítási követelmény **szigorúbb**:
a libplacebo naplónak `Device Name: NVIDIA RTX A1000` tartalommal kell
igazolnia, hogy nem `llvmpipe` szoftveres Vulkan-eszközt használt;
és az eredménynek pontosan **8 valódi kimeneti képkockát** kell
tartalmaznia.

A korábbi sikeres NVENC/NVDEC- és egyéb teszteket **nem szükséges
ismételten lefuttatni csak a libplacebo diagnosztikájához**. A PR #15
mellé hozzáadott önálló, helyi `scripts/dev/recheck-libplacebo.sh`
a már meglévő `source.mkv` videót használja, és új bizonyítékot hoz
létre az eredeti auditmappa módosítása nélkül.

```bash
cd ~/GIT/CinemaFundArchitecture4
git pull --ff-only
bash scripts/dev/recheck-libplacebo.sh \
  target/audit/ffmpeg/20261022T153037Z
```

A külön ellenőrzés `libplacebo-recheck-<id>` könyvtárba menti
a `metadata.txt`, `libplacebo.log` és `libplacebo.framehash`
fájlokat. **A fizikailag lefutott teszt eredménye az aktuális
javításra még nincs meg**, így a #14 RELEASE/HARDWARE kapu nyitott.
A jó eredmény sem helyettesíti a CFA4 alkalmazás végponttól végpontig
tesztjét és a jogi/SBOM/Qt6 kiadási ellenőrzéseket.

## Tesztmátrix és valódi elfogadási bizonyíték

| Munkafázis | Kötelező eredmény | Megjegyzés |
|---|---|---|
| `synthetic_source` + `source_probe` | PASS | MPEG-4 szintetikus forrás, ffprobe kodek/felbontás/képkockaszám |
| `nvenc_h264`, `probe_h264` | PASS | Valódi NVENC H.264 kódolás és dekódolható kimenet |
| `nvenc_hevc`, `probe_hevc` | PASS | Valódi NVENC HEVC kódolás és ffprobe |
| `reference_h264`, `reference_hevc`, `probe_reference_*` | PASS | H.264/HEVC **szoftveresen kódolt független referencia-minták**; NVDEC-t NVENC-hibától függetlenül tesztelik |
| `nvdec_h264`, `nvdec_hevc` | PASS | CUDA hardveres dekódolás **a szoftveresen előállított referencia-videókon**, GPU-felületként tartott output |
| `cpu_decode_h264`, `cpu_decode_hevc` | PASS | Kényszerített szoftveres fallback |
| `av1_software_sample`, `av1_software_probe` | PASS | Kis AV1 bemenet libsvtav1/libaom-av1 software encoderrel |
| `av1_nvdec`, `av1_cpu_decode` | PASS | AV1 NVIDIA hardveres + szoftveres dekódolás |
| `scale_vulkan` | PASS | GPU Vulkan upload → méretezés → download |
| `libplacebo` | PASS | GPU Vulkan upload → libplacebo → download |
| `avgblur_opencl` | PASS | OpenCL NVIDIA GPU, nem CPU OpenCL-provider |
| `av1_nvenc` | UNSUPPORTED_HARDWARE | RTX A1000 / Ampere: nincs AV1 encode |
 
A CLI-képesség hiánya `NOT_BUILT`, egy ténylegesen futtatott hibás parancs `FAIL`; ezek nem jelenhetnek meg automatikusan `PASS`-ként. A `BLOCKED` azt jelenti, hogy a prerequisite teszt nem állította elő a bemenetet; a forráskihagyás okát külön kell kezelni.

A javított audit a **H.264/HEVC/AV1 NVDEC** CUDA-felületeiről explicit `hwdownload,format=nv12` művelettel kis, 8 képkockás `framehash` mintát ír; így az esetleges sikeres, de 0 képkockás FFmpeg kilépés **nem** tekinthető dekódolási PASS-nak. A `libplacebo` a hivatalos FFmpeg 9 szűrőpéldáknak megfelelően közvetlenül kezelheti a szoftveres bemeneti képkockát a Vulkan rendererén keresztül; nem kényszerítjük a `scale_vulkan` szűrőhöz szükséges `hwupload/hwdownload` láncot. A `libplacebo` részletes `-loglevel debug` naplót és 8 képkockás `framehash` fájlt kap. **Ennek a javított változatnak a fizikai GPU-s eredménye még nem áll rendelkezésre.** **Bármely kötelező BLOCKED állapot INCOMPLETE auditot okoz**, nem sikeres kaput.

**2026-10-22 – első RTX A1000 mérés és következő diagnosztika:**
A `72cde2dc7af4941a2312f9c820a1c6c4d25bae30` commiton
`20261022T151225Z` azonosítóval **4 FAIL** keletkezett: `nvenc_h264`,
`nvenc_hevc`, `av1_nvdec`, `libplacebo`; `scale_vulkan` és
`avgblur_opencl` PASS. A `nvdec_h264`/`nvdec_hevc` előzőleg csak
az NVENC hibák miatt **BLOCKED** volt; ez NEM hardveres dekóderhiba.
Az elkülönített szoftveres H264/HEVC referencia-minták ezt a vakfoltot
szüntetik meg. A négy hiba részletes naplója megérkezett. A `nvenc_h264` és `nvenc_hevc` inicializálta az NVENC 13.1 API-t és az RTX A1000 hardvert, majd mindkettő `Frame Dimension(s) less than the minimum supported value` hibával megállt. Az AV1 NVDEC `min_height: 128` képességkorlátot jelzett, miközben a minta magassága 72 volt. **E három hiba közös gyökérokát igazolta a napló:** az alulméretezett tesztvideót 640×360-ra cseréltük.

A `libplacebo` inicializálta a Vulkan NVIDIA eszközt és a libplacebo v7.360.0-t, de a filtergraph inicializációja `Invalid argument` hibával megállt. A pontos rész-ok még nem igazolt. A tesztprogramban ezért egyszerűsítettük a szűrőgráfot, elhagytuk a kényszerített Vulkan `hwupload/hwdownload` láncot, közvetlen `libplacebo=w=320:h=180:format=yuv420p` szűréssel és részletes `debug` naplóval. Fizikai újrafuttatás szükséges ennek igazolására.

```bash
cd ~/GIT/CinemaFundArchitecture4
d=target/audit/ffmpeg/20261022T151225Z
for test in nvenc_h264 nvenc_hevc av1_nvdec libplacebo; do
    printf '\n===== %s =====\n' "$test"
    tail -n 65 "$d/logs/$test.log"
done
# A teljes FFmpeg build és OS bizonyítékait a kapcsolódó
# metadata.txt és ffmpeg-buildconf.txt fájlok tartalmazzák.
```


### Az eredményfájlok

A harness létrehozza a `target/audit/ffmpeg/<UTC-időbélyeg>/` könyvtárat, benne:

- `metadata.txt`: Git SHA, OS/kernel/session, FFmpeg és ffprobe pontos binárisútvonal, FFmpeg SHA-256, GPU/driver és compute capability.
- `ffmpeg-version.txt`, `ffprobe-version.txt`, `ffmpeg-buildconf.txt`, `hwaccels.txt`, `encoders.txt`, `decoders.txt`, `filters.txt`: a build- és capability-listák visszakereshető bizonyítékai.
- `results.tsv`, `SUMMARY.txt`, `logs/*.log`: tételes eredmények és a pontos CLI-parancsok, beleértve a hibák stderr kimenetét.
- `libplacebo.framehash`, `nvdec_h264.framehash`, `nvdec_hevc.framehash`, `av1-nvdec.framehash`: a szűrő/dekóder valóban 8 képkockát állított elő; a teszt méretei és kapui fixálva.
- `source.mkv`, `h264.mp4`, `hevc.mp4`, `av1-software.mkv`: kisméretű ellenőrző médiák.

A `summary` önmagában nem igazolja, hogy a **CFA4 alkalmazás** a GPU-végpontot helyesen használja. A `--enable-gpl` / FFmpeg / x264 / x265 / HEVC / Qt6/HEIF és a csomagolás jogi review-ja változatlanul külön R0 kapu. Eltérő CLI-binárisok között a puszta 9.0.2 verzióegyezés nem jelent képesség-/ABI-egyezést.

## Ellenőrzés és lezárás

**A #14 lezárásához** valódi RTX A1000-on meg kell vizsgálni a `results.tsv` és `SUMMARY.txt` tartalmát, továbbá a GPU-futásokat igazoló naplórészleteket. A `libplacebo`, Vulkan vagy OpenCL hibáját **nem** szabad CPU fallbackkel sikeres GPU-tesztnek minősíteni. Az AV1 NVENC A1000-on ismert hardveres korlát, nem implementációs hiba.

Hiba esetén kizárólag a ténylegesen hibás ágat javítsuk; **nem** indokolt a teljes 8 jobos Rust release CI futtatása, amíg csak a helyi FFmpeg CLI-audit módosul.


## 2026-10-22 – a libplacebo célzott tesztjének SHA-256 előellenőrző hibája

A felhasználó a `f57ce4d958a7c1babb61c725ed08b48ee894ca6b`
committal futtatta a `scripts/dev/recheck-libplacebo.sh` szkriptet az
eredeti `20261022T153037Z` adatmappára, és a teszt **még az FFmpeg
elindítása előtt** ezzel állt meg:

```text
CFA4 LIBPLACEBO RECHECK FAILED: FFmpeg executable SHA256 differs from the original audit ( vs 502cff20212a8b2d03f12980637a9070998572efc05bb8999280d47d55f10a8e)
```

**Bizonyított gyökérok:** az eredeti hash olvasására használt
`sed` BRE-kifejezésben a csoportosító zárójelek és az ismétlésszám
kétszer voltak escape-elve. A feldolgozás ezért üres értéket adott,
miközben az új `sha256sum` helyes értéket mutatott. A kimenet nem
igazol FFmpeg-bináris eltérést és nem libplacebo-/GPU-renderelési hiba.

A [`82e453bf`](https://github.com/ubuntuokos/CinemaFundArchitecture4/commit/82e453bf786727323001eddd174e83f6899fea66)
javítócommit óta a `scripts/ci/read-ffmpeg-audit-sha256.sh` külön,
egyszerű mezőolvasóként kezeli az eredeti `FFmpeg SHA256: <hash> <path>`
sort. Pontosan egy bejegyzést és **64 hexadecimális karaktert** követel;
a `recheck-libplacebo.sh` a valódi bináris hash-t továbbra is
összehasonlítja vele. A `scripts/ci/test-ffmpeg-audit-sha256.sh`
regresszióteszt valós formájú metaadaton PASS, hiányzó/rövid/duplikált
hash-bejegyzéseket és nem létező bemenetet elutasít.

**Statikus CI bizonyíték:**
[GitHub Actions #39808033548](https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/39808033548)
PASS, benne a regresszióteszt külön
`PASS: valid FFmpeg SHA256 metadata parsed; malformed, missing and duplicate hashes rejected.`
kimenettel.

A `libplacebo` **fizikai GPU-s** ellenőrzése még függőben van. A
helyi meglévő 8 képkockás mintából, fordítás nélkül:

```bash
cd ~/GIT/CinemaFundArchitecture4
git pull --ff-only
bash scripts/dev/recheck-libplacebo.sh target/audit/ffmpeg/20261022T153037Z
```


## Végleges helyi FFmpeg CLI hardveraudit – 2026-10-22, PASS

**Értelmezési határ:** ez a fejezet a korábbi `libplacebo NOT_RUN/FAIL`
megjegyzéseket *időrendben felülírja*, az eredeti sikertelen mérési
naplók törlése nélkül. A CLI-oracle teljesült; a **CFA4 alkalmazásszintű
Qt6/FFmpeg import/render/export végpontok külön, nyitott
[R0 #16](https://github.com/ubuntuokos/CinemaFundArchitecture4/issues/16)
munkacsomagban** kerülnek ellenőrzésre. A teljes kiadás továbbra is
`HOLD`.

### Két fizikai RTX A1000 mérés – egy összetett elfogadási bizonyíték

| Bizonyíték | Eredmény |
|---|---|
| Alapfuttatás – `20261022T153037Z` | Git `0f81b74f203ba3329059009c2efc507a70531a61`, 640×360, 8 frame @ 8 fps, FFmpeg 9.0.2, NVIDIA RTX A1000 |
| Alapfuttatás orákulumai | **24 PASS** – H.264+HEVC NVENC encode/probe, H.264+HEVC NVDEC CUDA/8-frame check, AV1 NVDEC/8-frame check, SW/CPU decode fallback, `scale_vulkan`, NVIDIA `avgblur_opencl` |
| Alapfuttatás kezdeti hiánya | `libplacebo` **FAIL**, `libplacebo_framecheck` **BLOCKED** – Vulkan-context import `EINVAL` |
| Külön `libplacebo` újraellenőrzés – `libplacebo-recheck-20261022T155032Z-YlOybkR5` | A `3c7a797bfe11edde6bade0d2a2a410d63a89f905` script-committal, **PASS**; `PASS: libplacebo processed 8 frames using the NVIDIA GPU (NVIDIA RTX A1000)` |
| FFmpeg bináris azonossága | Az önálló szkript **kötelező SHA-256 egyezés** után lépett tovább; az eredeti mintán `502cff20212a8b2d03f12980637a9070998572efc05bb8999280d47d55f10a8e` |
| Fizikai GPU / sikerfeltétel | NVIDIA RTX A1000 név bizonyítva a `libplacebo` naplóból, **8 kimeneti képkocka** a `framehash`-ből; nincs CPU/llvmpipe elfogadás |
| RTX A1000 AV1 NVENC | `UNSUPPORTED_HARDWARE` – dokumentált, elvárt Ampere-korlát; AV1 NVDEC ettől függetlenül PASS |

Az eredeti `libplacebo` hiba **reprodukálhatóan megkerülhető** azzal,
hogy a szűrő saját Vulkan-eszközt hoz létre. Az FFmpeg 9.0
`vf_libplacebo.c` `PL_API_VER < 365` importálási ága továbbra is
**lehetséges** kompatibilitási gyökérok; a bemásolt napló nem méri
közvetlenül a `queue_flags` mezőt, ezért konkrét upstream library-bugnak
nem minősítjük.

**Fontos:** a „24 PASS + külön `libplacebo` PASS” **nem 25 siker
ugyanabban a futtatásban**. Két, egymással FFmpeg SHA256, NVIDIA GPU
és eredeti `source.mkv` alapján összekapcsolt fizikai mérésről van szó.
Az audit fájljai a helyi `target/audit/ffmpeg/` könyvtárban vannak;
a GitHubba **csak a parancskimenet és az auditrekord került**,
a bináris videó-/framehash-fájlok nem.

**CLI hardverkapu döntése: `PASS`**. A [#14](https://github.com/ubuntuokos/CinemaFundArchitecture4/issues/14)
munkacsomag kizárólag a FFmpeg CLI-gépességorákulumok lezárását
tanúsítja. A valódi CFA4 Rust/Qt6 futtatási útvonalak és a CPU/GPU
eredményparitás elfogadása [#16](https://github.com/ubuntuokos/CinemaFundArchitecture4/issues/16)
feladat. A GPL/libplacebo/NVENC/HEVC terjesztési licenc/SBOM audit
[F1-01 #6](https://github.com/ubuntuokos/CinemaFundArchitecture4/issues/6)
és a [központi R0 #11](https://github.com/ubuntuokos/CinemaFundArchitecture4/issues/11)
alatt **nyitott**. A [PR #15](https://github.com/ubuntuokos/CinemaFundArchitecture4/pull/15)
továbbra is **draft**, merge és kiadási engedély nélkül.
