# F2-01: mért futásidejű függőségek a CI Ubuntu 26.04-es gépén, 2026-11-08

Issue: #22 · F9 kapu: `runtime_abi_dependencies` · Eszköz: `scripts/dev/audit-runtime-deps.sh`
· Workflow: `.github/workflows/cfa4-runtime-deps.yml` · Előző mérés:
[F2-01-RUNTIME-DEPS-2026-10-25](F2-01-RUNTIME-DEPS-2026-10-25.md)

A fejlesztés a felhőben fut (tulajdonosi közlés, 2026-11-08), ezért ez a mérés egy GitHub-hosztolt
Ubuntu 26.04 AMD64 gépen készült, nem a tulajdonos gépén.

## Mérési környezet

| | |
|---|---|
| Gép | GitHub-hosztolt `ubuntu-26.04` futtató: Ubuntu 26.04.1 LTS, x86_64, kernel 7.0.0-1019-azure |
| Forrás | a #756 feje (`f6028b95`, a `main` `ccb36679`-ra épül) |
| Build | `cargo build --release --locked -p cfa4 -p cfa4-cli --features cfa4/gstreamer,cfa4-cli/gstreamer` (a `package.sh` feature-jei a #743 előtt) |
| Időpont | 2026-11-08T16:41:11Z |
| Futás | [job 128790524061](https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/42500013873/job/128790524061) (a teljes jelentés a job összegzőjében) |
| Érvényes F9-bizonyíték | igen (`host.ubuntu_2604 = true`) |

## Közvetlen ELF-függőségek (`NEEDED`)

| Bináris | NEEDED |
|---|---|
| `cfa4` | `ld-linux-x86-64.so.2`, `libasound.so.2`, `libc.so.6`, `libgcc_s.so.1`, `libgio-2.0.so.0`, `libglib-2.0.so.0`, `libgobject-2.0.so.0`, `libgstapp-1.0.so.0`, `libgstbase-1.0.so.0`, `libgstpbutils-1.0.so.0`, `libgstreamer-1.0.so.0`, `libgstvideo-1.0.so.0`, `libm.so.6` |
| `cfa4-cli` | ugyanez, a `libasound.so.2` nélkül |

Új a 2026-10-25-i méréshez képest: a GStreamer- és GLib-könyvtárak. Ezeket a `gstreamer` feature
hozza, amellyel a csomagok épülnek. Az előző mérés feature nélküli buildet mért.

## Szimbólumverzió-minimumok és csomagok

| Névtér | Minimum |
|---|---|
| GLIBC | **2.43** (változatlan, a `.deb` `libc6 (>= 2.43)`-t deklarál) |
| GCC | 4.2.0 |
| ALSA | 0.9.0rc4 |

| Csomag | Verzió a mérőgépen |
|---|---|
| `libasound2t64` | 1.2.15.3-1ubuntu1.1 |
| `libc6` | 2.43-2ubuntu2.4 |
| `libgcc-s1` | 16-20260322-1ubuntu1 |
| `libglib2.0-0t64` | 2.88.0-1ubuntu0.1 |
| `libgstreamer-plugins-base1.0-0` | 1.28.2-1ubuntu0.1 |
| `libgstreamer1.0-0` | 1.28.2-1 |

## Futás közben betöltött könyvtárak (`dlopen`)

`libEGL.so.1`, `libdbus-1.so(.3)`, `libva.so.2`, `libva-drm.so.2`, `libwayland-client.so(.0)`,
`libwayland-egl.so(.1)`, `libxkbcommon.so`, `libxkbcommon-x11.so`.

**Eltérés, kivizsgálandó:** a 2026-10-25-i mérés még az X11-könyvtárakat (`libX11.so.6`,
`libX11-xcb.so.1`, `libXcursor.so.1`, `libXi.so`) és az NVIDIA-könyvtárakat (`libcuda.so.1`,
`libnvcuvid.so.1`, `libnvidia-encode.so.1`) is listázta. Ebben a buildben a mérő nem találta
ezeket a neveket a binárisokban. Ennek oka lehet a mérőszkript mintakeresése, a két build
eltérése, vagy hogy azóta ezek a kódutak máshol töltődnek be. A forrásban a
`crates/platform/src/nvdec/cuvid.rs` továbbra is `libnvcuvid.so.1`-et tölt be. Amíg ez nincs
tisztázva, a `.deb` X11- és NVIDIA-ajánlásai maradnak.

## Mi hiányzik még a kapuhoz

- Az FFmpeg 9.0.2 feature-rel (a #743 óta a csomagok része) épített binárisok mérése.
- A négy telepített csomag (.deb, AppImage, Flatpak a homokozóval, tar.gz) mérése a tiszta
  telepítési lépésben ([F9-RELEASE-STATUS](F9-RELEASE-STATUS-2026-11-08.md) 4. szakasz, a #746-ban).
- Az X11/NVIDIA eltérés tisztázása.

A kapu ezért továbbra is OPEN.
