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
