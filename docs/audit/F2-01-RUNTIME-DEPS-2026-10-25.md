# F2-01: mért futásidejű függőségek (Ubuntu 26.04 AMD64), 2026-10-25

Issue: #22 · F9 kapu: `runtime_abi_dependencies` · Eszköz: `scripts/dev/audit-runtime-deps.sh`
(`python/cfa4_devtools/runtime_deps.py`, #61).

## Mérési környezet

| | |
|---|---|
| Gép | Kubuntu, Ubuntu 26.04.1 LTS, x86_64, kernel 7.0.0-39-generic (a tulajdonos referenciagépe) |
| Forrás | `origin/main` @ `d0cf06e`, külön worktree-ben, `cargo build --release --locked -p cfa4 -p cfa4-cli` |
| Időpont | 2026-10-25T17:43:19Z |
| Érvényes F9-bizonyíték | igen (`host.ubuntu_2604 = true`) |

## Közvetlen ELF-függőségek (`NEEDED`)

| Bináris | NEEDED |
|---|---|
| `cfa4` | `ld-linux-x86-64.so.2`, `libasound.so.2`, `libc.so.6`, `libgcc_s.so.1`, `libm.so.6` |
| `cfa4-cli` | `ld-linux-x86-64.so.2`, `libc.so.6`, `libgcc_s.so.1`, `libm.so.6` |

Minden közvetlen függőség feloldódott (`ldd`), nincs „not found”.

## Szimbólumverzió-minimumok (`objdump -T`)

| Névtér | Minimum | Csomag a mérőgépen |
|---|---|---|
| GLIBC | **2.43** | `libc6:amd64` 2.43-2ubuntu2.4 |
| GCC | 4.2.0 | `libgcc-s1:amd64` 16-20260322-1ubuntu1 |
| ALSA | 0.9.0rc8 | `libasound2t64:amd64` 1.2.15.3-1ubuntu1.1 |

**Következmény:** a 26.04-en épített binárisok glibc 2.43-at igényelnek, tehát Ubuntu 24.04-en
(glibc 2.39) nem indulnak. A `.deb` eddig `libc6 (>= 2.35)`-öt deklarált (az upstream
ubuntu-22.04-es buildjéből örökölve), így egy 24.04-es gépre telepíthető lett volna, de ott nem
futott volna. Javítva: `packaging/linux/nfpm.yaml` → `libc6 (>= 2.43)`, `docs/releasing.md` frissítve.

## Futás közben betöltött könyvtárak (`dlopen`, az `ldd` nem látja)

A binárisban szereplő könyvtárnevek, a közvetlen függőségek nélkül. A csomagnevek a 26.04-es
megfeleltetés, nem a mérés részei.

| Terület | Könyvtár | Ubuntu-csomag | A `.deb`-ben |
|---|---|---|---|
| X11 | `libX11.so.6`, `libX11-xcb.so.1`, `libXcursor.so.1`, `libXi.so` | `libx11-6`, `libx11-xcb1`, `libxcursor1`, `libxi6` | recommends |
| Billentyűzet | `libxkbcommon.so.0`, `libxkbcommon-x11.so.0` | `libxkbcommon0`, `libxkbcommon-x11-0` | depends / recommends (új) |
| Wayland | `libwayland-client.so.0`, `libwayland-egl.so.1` | `libwayland-client0`, `libwayland-egl1` | recommends (az egl új) |
| GPU | `libEGL.so.1` | `libegl1` | depends (`libvulkan1 \| libegl1`) |
| Rendszer | `libdbus-1.so.3` | `libdbus-1-3` | recommends (új) |
| VA-API | `libva.so.2`, `libva-drm.so.2` | `libva2`, `libva-drm2` | recommends (új) |
| NVIDIA | `libcuda.so.1`, `libnvcuvid.so.1`, `libnvidia-encode.so.1` | az NVIDIA-illesztőprogram csomagjai | nem (illesztőprogram-függő, opcionális) |

Megfigyelés: `libvulkan.so.1` nem szerepel a release binárisban, a grafika EGL-en át töltődik.
A `libvulkan1 | libegl1` függés ezzel összhangban van; ha a Vulkan-backend később bekerül, a
mérést meg kell ismételni.

## A mérőscript javításai (e mérés alapján)

A nyers jelentés két hibát mutatott, a fenti táblázatok már a javított értelmezést tartalmazzák:

1. **Lokalizált `dpkg-query`:** magyar nyelvi beállítás mellett a diverziós sor
   („eltérítés libc6 által innen: …”) csomagként jelent meg. Javítás: az eszközök `LC_ALL=C`
   alatt futnak, és szóközt tartalmazó „csomagnevet” a parser eldob.
2. **Összefűzött X11-szimbólumnevek:** egy szóköz nélküli string-pool („…XSetXlibControls…libXi.so.6”)
   egyetlen hatalmas „könyvtárnévként” illeszkedett. Javítás: a név csak nem azonosító-karakter
   után kezdődhet, és legfeljebb 64 karakteres.

Mindkettőre regressziós teszt került a `python/tests/test_runtime_deps.py`-ba.

## Mit fed le és mit nem

Lefedve: a két release bináris közvetlen ELF-függőségei, ABI-minimumai és dlopen-jelöltjei a
célplatformon.

Nem fedi le (ezért a `runtime_abi_dependencies` kapu **OPEN** marad):
- a négy csomagformátum (deb, AppImage, Flatpak, tar.gz) telepített futása, beleértve a Flatpak
  sandbox runtime-ját és az AppImage-be csomagolt könyvtárakat;
- a kodekek és külső eszközök (FFmpeg, libheif) futásidejű elérése;
- a Qt 6 / PySide6 felület függőségei.
