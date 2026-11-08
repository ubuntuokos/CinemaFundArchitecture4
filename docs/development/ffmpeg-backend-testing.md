# Az FFmpeg-háttér tesztelése a saját gépen (ADR 0005)

> **Létrehozva:** 2026-10-27 · **Cél:** Ubuntu 26.04 LTS, AMD64, a referenciagép (RTX A1000) és
> legalább egy nem NVIDIA GPU (Intel vagy AMD) · **Ág:** `cfa4/ffmpeg-backend`

A felhőben futó próba GPU nélkül, Ubuntu 24.04-en zajlott. Ami ott nem mérhető, azt ez az
útmutató a valódi gépen ellenőrzi: a VA-API- és NVENC/NVDEC-utat, a Vulkan Videót, és a Qt
Beállítások ablakot képernyővel. Minden lépés végén ott áll, mit kell látni.

## 1. Előkészítés

```sh
sudo apt install build-essential nasm pkg-config clang libclang-dev git curl \
  libva-dev libdrm-dev libdav1d-dev libvpx-dev libopus-dev libwebp-dev libmp3lame-dev \
  libvulkan-dev libffmpeg-nvenc-dev libvpl-dev vainfo
git clone https://github.com/ubuntuokos/CinemaFundArchitecture4 && cd CinemaFundArchitecture4
git checkout cfa4/ffmpeg-backend
```

VA-API-hoz a GPU meghajtója: Intel `intel-media-va-driver`, AMD `mesa-va-drivers`. A `vainfo`
kimenetében a `VAProfileH264…` és `VAProfileHEVC…` sorok jelzik, mit tud a hardver.

## 2. Az FFmpeg 9.0.2 lefordítása

```sh
scripts/dev/build-ffmpeg-9.0.2.sh            # alap: /opt/cfa4/ffmpeg-9.0.2 (sudo-t kérhet)
```

**Elvárt:** a végén `FFmpeg 9.0.2 installed in /opt/cfa4/ffmpeg-9.0.2 (GPL-3 or later, shared
libraries)`, a listában `VA-API`, `NVENC/NVDEC`, `Vulkan Video` (26.04-en a Vulkan-fejlécek elég
újak), `Intel Quick Sync` ha a `libvpl-dev` telepítve van. A szkript hibával leáll, ha a build
nem GPL-3-as, ha nonfree, vagy ha az alapbuildben x264 vagy x265 van.

## 3. A CFA4 lefordítása FFmpeg-gel

```sh
export PKG_CONFIG_PATH=/opt/cfa4/ffmpeg-9.0.2/lib/pkgconfig
cargo build --release -p cfa4 -p cfa4-cli --features cfa4/ffmpeg,cfa4-cli/ffmpeg
readelf -d target/release/cfa4-cli | grep RUNPATH
```

**Elvárt:** `RUNPATH … [/opt/cfa4/ffmpeg-9.0.2-restricted/lib:/opt/cfa4/ffmpeg-9.0.2/lib]`.

## 4. A hátterek állapota

```sh
target/release/cfa4-cli media-backends
```

**Elvárt:** három háttér ebben a sorrendben: `builtin` (CFA4-motor), `ffmpeg`
(`"available": true`, `"version": "n9.0.2"`, `"license": "GPL version 3 or later"`), `gstreamer`.
A `restricted` listában `libx264` és `libx265` `"available": false`. Az útválasztó döntése a
`route` mezőben: a `media.fallback` lánc `software` → `ffmpeg` → `gstreamer` (a saját motor mindig
elöl), az `ffmpeg` `"tried": 1`, a `gstreamer` `"tried": 2` (vagy `null`, ha nincs a buildben).

### 4a. A sorrend felcserélése a felhasználói táblával

```sh
mkdir -p ~/.config/cfa4
printf '[compute.task."media.fallback"]\norder = ["software", "gstreamer", "ffmpeg"]\n' >> ~/.config/cfa4/routing.toml
target/release/cfa4-cli media-backends
```

**Elvárt:** a `gstreamer` kerül a második helyre (`"tried": 1`), az `ffmpeg` a harmadikra. Ha a
sorból kihagyod az `ffmpeg`-et, az állapota `"tried": null`, a Beállítások oldalon *Left out by the
routing table*. A próba után töröld a sort a `routing.toml`-ból.

## 5. Dekódolás – a tesztkészlet

```sh
PATH=/opt/cfa4/ffmpeg-9.0.2/bin:$PATH cargo test --release -p cfa4-ffmpeg --features ffmpeg
```

**Elvárt:** 16 + 7 teszt zöld, köztük a 10 bites 4:2:2 H.264 és HEVC bitpontossága, a DV, a
DVCPRO HD, az AVI-ba csomagolt MPEG-4 Part 2 és a WMV, a sérült fájlok és a hangfolytonosság. (A
korlátozott build az ellenőrzőhöz nem kell: a tesztfájlok x264 nélkül is elkészülnek, ha nem,
a tesztek kihagyják magukat és ezt kiírják.)

## 6. Dekódolás – valódi kamerafájlok

Ha van kéznél: Sony XAVC S-I vagy XAVC HS 4:2:2 10 bit, Panasonic 4:2:2 10 bites MOV (#626),
Canon XF-AVC, régi DV-kazetta digitalizálása, AVI vagy WMV.

```sh
target/release/cfa4-cli probe /útvonal/klip.mov
```

**Elvárt:** a JSON-ban `"openedBy"`: `"CFA4 engine"` vagy `"supplementary backend"`; az
utóbbinál a `video.codec` `(FFmpeg)`-re végződik, és a `video.pixel_format` a forrás saját
bitmélységét mutatja (pl. `YUV 4:2:2 10-bit (yuv422p10le, FFmpeg)`), nem 8 bites 4:2:0-t. Ezután a programban importáld, tedd idővonalra,
lépkedj és ugorj benne: a kép ne csússzon el egy képkockával, a hang legyen szinkronban.

## 7. Hardveres dekódolás és kódolás

Az FFmpeg-build saját eszközeivel, a CFA4-től függetlenül:

```sh
F=/opt/cfa4/ffmpeg-9.0.2/bin/ffmpeg
$F -hide_banner -hwaccels
# nem NVIDIA GPU (Intel / AMD), VA-API kódolás:
$F -hide_banner -vaapi_device /dev/dri/renderD128 -f lavfi -i testsrc2=size=1920x1080:rate=25 -t 5 \
   -vf format=nv12,hwupload -c:v h264_vaapi /tmp/vaapi_h264.mp4
$F -hide_banner -vaapi_device /dev/dri/renderD128 -f lavfi -i testsrc2=size=1920x1080:rate=25 -t 5 \
   -vf format=nv12,hwupload -c:v hevc_vaapi /tmp/vaapi_hevc.mp4
# RTX A1000, NVENC:
$F -hide_banner -f lavfi -i testsrc2=size=1920x1080:rate=25 -t 5 -c:v h264_nvenc /tmp/nvenc_h264.mp4
$F -hide_banner -f lavfi -i testsrc2=size=1920x1080:rate=25 -t 5 -c:v hevc_nvenc /tmp/nvenc_hevc.mp4
```

**Elvárt:** mindegyik parancs hiba nélkül lefut, és a fájlt a `cfa4-cli probe` megnyitja.
Két GPU esetén a `renderD128` / `renderD129` közül az Intel/AMD kártyáét kell megadni
(`ls -l /dev/dri/by-path`). Ha egy kódoló hiányzik, a `vainfo` kimenete mutatja, mit tud a hardver.

## 8. Beállítások és menük (Qt)

```sh
python3 -m pip install --user -e '.[qt]'
PYTHONPATH=python python3 -m cfa4_devtools.qt_app --engine target/release/cfa4-cli
```

- **Szerkesztés ▸ Médiahátterek…** – a Beállítások ablak a *Média és lejátszás ▸ Media Backends*
  oldalon nyílik. Felül két kapcsoló (FFmpeg, GStreamer), alatta a táblázat: sorrend, háttér,
  állapot, verzió, licenc; alatta a *Restricted codecs* rész.
- **Súgó ▸ Korlátozott kodekek…** – ugyanez az oldal.
- Az FFmpeg kapcsolót kapcsold ki, **OK**, nyisd meg újra: az állapot `Available, switched off`;
  `target/release/cfa4-cli media-backends` szerint `"enabled": false`. Kapcsold vissza.

A szerkesztő Qt-felületen (`PYTHONPATH=python python3 -m cfa4_qt --engine target/release/cfa4-cli
--lang hu`) ugyanez a **Szerkesztés ▸ Beállítások ▸ Médiahátterek…** és a **Súgó ▸ Korlátozott
kodekek…** menüpontból érhető el, magyarul.

## 9. A korlátozott kodekek (Ubuntu-modell)

```sh
sudo apt install libx264-dev libx265-dev
scripts/dev/build-ffmpeg-9.0.2.sh --restricted   # → /opt/cfa4/ffmpeg-9.0.2-restricted
ldd target/release/cfa4-cli | grep avcodec
target/release/cfa4-cli media-backends
```

**Elvárt:** az `ldd` most a `…-restricted/lib`-et mutatja, a `restricted` listában `libx264` és
`libx265` `"available": true`, a Beállítások oldalon *Installed: Yes*. Ezután nevezd át a
`/opt/cfa4/ffmpeg-9.0.2-restricted` könyvtárat: a program újraindítás után az alapbuildet tölti
be, és a két kodek újra *No* – a program nem omlik össze, csak a funkció inaktív.

## Mit küldj vissza

A 2., 4., 5. és 7. lépés kimenetét (szövegként), a 8. lépésről egy képernyőképet, és a 6.
lépésből azokat a fájlokat (kamera, kodek, profil), amelyek nem nyíltak meg vagy rosszul
jelentek meg.
