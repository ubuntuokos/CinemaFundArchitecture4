# CFA4-RENDER-INTEGRATION-PLAN-001 – F0a: környezet (F0-environment)

**Dátum:** 2026-11-14 · **Terv:** [`CFA4-RENDER-INTEGRATION-PLAN-001`](../plans/CFA4-RENDER-INTEGRATION-PLAN-001.md), F0a 3. feladat, A10 szabály ·
**Összefoglaló:** [`RENDER-F0a-BASELINE-2026-11-14.md`](RENDER-F0a-BASELINE-2026-11-14.md)

A terv A10 szabálya (reprodukálhatóság) szerint minden benchmark és release-teszt mellé a
forrásrevíziót, a verziókat, a GPU-drivert, a kernelt és a display-szervert rögzíteni kell. Ezt
a gyűjtő egy paranccsal, root és telepítés nélkül adja JSON-ban:

```sh
python3 python/cfa4_devtools/render_environment.py -o render-environment.json
```

Csak fájlokat olvas és `--version`-jellegű lekérdezéseket futtat (`rustc`, `cargo`, `ffmpeg`,
`qtpaths6`, `nvidia-smi`, `vulkaninfo --summary`, `vainfo`, `lscpu`, `/usr/bin/python3`); ami
hiányzik, az `null`, nem hiba. GPU-munkát, hálózatot és rendszerbeállítást nem érint (A6).

## 1. Forrásból igazolt értékek (gépfüggetlen)

| Mező | Érték | Forrás |
|---|---|---|
| HEAD | `b76870906438ffc3503c36f2210528dd9ab7fab0` (`main`) | git |
| `Cargo.lock` sha256 | `e12b8e69b96936230bd18613381ee37d20563beec99c2a591038f8a25a650334` | gyűjtő |
| `requirements/linux-amd64.txt` sha256 | `2aa0254db441df9ea2e123239f08e773e6edfc749221ee35776b2ffd338a8c90` | gyűjtő |
| `requirements/otio-oracle.txt` sha256 | `6608e9d53041bf8923f04270cc6516a9b628d855b54dfae05ccf88c13277bb94` | gyűjtő |
| Rust | `rust-toolchain.toml` 1.97.1 (rustc 1.97.1 8bab26f4f 2026-07-14), MSRV 1.97, edition 2024 | gyűjtő |
| FFmpeg (termék) | 9.0.2, forrás sha256 `8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e`, GPL-3 build, `/opt/cfa4/ffmpeg-9.0.2` | `scripts/dev/build-ffmpeg-9.0.2.sh` |
| FFmpeg (CI-orákulum) | 9.0.2 statikus, ugyanaz a forrás-hash | `scripts/ci/install-ffmpeg-9.0.2.sh` |
| wgpu | 30.0.1 | `Cargo.lock` |
| PySide6 (venv, AppImage, Flatpak) | 6.12.0 | `requirements/linux-amd64.txt` |
| Python a CI-ben | 3.13 (blokkoló), 3.14 (nem blokkoló) | `docs/development/linux.md` |

## 2. Referenciagép ([`reference-machine.md`](../development/reference-machine.md))

| Mező | Érték | Állapot |
|---|---|---|
| Gép, CPU, RAM | Dell Precision Tower 7910, 2 × Xeon E5-2696 v4 (88 szál, 2 NUMA), 512 GiB | ismert |
| GPU | RTX A1000 (kijelző) + RTX A4000 (számítás) | ismert |
| OS, asztal | Ubuntu Studio 26.04 AMD64, KDE Plasma 6.6.6, Frameworks 6.24.0 | ismert |
| Display-szerver | Wayland | ismert |
| Qt (rendszer) | 6.10.2 | ismert |
| Kernel | 7.0.0-43-generic | ismert |
| NVIDIA-driver, CUDA-driver API | – | PENDING: a gyűjtő `gpu.nvidia_gpus`, `gpu.cuda_driver_api` mezője |
| Vulkan API és eszközök | – | PENDING: `gpu.vulkan_api_versions`, `gpu.vulkan_devices` |
| VA-API | – | PENDING: `gpu.vaapi` |
| Rendszer-Python (Ubuntu 26.04) | – | PENDING: `host.system_python`; az A11 Python-nyilvántartás (F1) alapértéke |
| A CFA4 FFmpeg-build BUILDINFO-ja | – | PENDING: `ffmpeg.cfa4_buildinfo` |

A PENDING sorok a gyűjtő referenciagépes futtatásával töltődnek ki (a tulajdonost nem kell
kérdezni, `CLAUDE.md` 0. lépés); az F0b R0-újrafuttatása (NVENC/NVDEC) ugyanott történik, ezért
a kettő egy alkalommal elvégezhető.

## 3. Felhős fejlesztői munkamenet (ahol ez a dokumentum készült)

A gyűjtő itt is lefutott; az értékek **nem** a referenciagépéi, csak a gyűjtő működését
igazolják: Ubuntu 24.04.5, kernel 6.18.44, 4 vCPU, rendszer-FFmpeg 6.1.1, rendszer-Python 3.13.18,
GPU, Vulkan, Qt és a CFA4 FFmpeg-build nincs (`null`).
