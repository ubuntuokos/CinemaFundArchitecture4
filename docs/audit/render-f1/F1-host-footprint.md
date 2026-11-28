# F1: gazdagép-lábnyom (Ubuntu 26.04 amd64)

**Jelleg:** csak olvasás; a számok a repo és a jelöltek forrásából jönnek, a referenciagépen mért értékek PENDING.

| Jelölt | Rendszercsomag / futtatókörnyezet | Python | Megjegyzés |
|---|---|---|---|
| OpenImageIO (beépítve) | C++17 fordító, CMake ≥ 3.23, OpenEXR/Imath, libtiff, libpng, libjpeg-turbo, zlib; opcionálisan libheif, LibRaw, OpenJPEG, JXL | nem kell (a Python-kötés kikapcsolva) | FFmpeg-bővítmény kikapcsolva (A7); az OIIO + OpenEXR + OCIO beépítési sorrendjét az ADR 0003 adja (meglévő) |
| VapourSynth worker | `libvapoursynth`, a bővítmények `.so` fájljai | rendszer-Python 3.13+ külön virtuális környezetben; `vs-jetpack` (`numpy`, `rich`, `psutil`, `jetpytools`) | R80 óta Vulkan 1.4 GPU-szűrők és szabadon szálazott Python; az GPU-ág nem része a POC-nak |
| Blender | a gazda Blendere vagy rögzített bináris | a Blender saját Pythonja | külön folyamat, EXR csere |
| ReelForge, avio, rust-motion | – | – | REFERENCE_ONLY: nem kerül a gazdagépre |
| EC-jelölt (#11) | Rust 1.95+ fordítás (workspace, 832 függőség); FFmpeg nem kell | nem kell | POC-jelölt; csak az F2 elején, ha az MQ-mérés indul |

A Python-nyilvántartás (A11) első sora: `CFA4 Qt + VapourSynth worker = rendszer-Python 3.13+`; a CFA4 `requires-python >= 3.12`
változatlan marad. Mérendő a referenciagépen: rendszer-Python verzió, `vulkaninfo`, `nvidia-smi`, Landlock ABI
(`render_environment.py` + a sandbox `doctor`).
