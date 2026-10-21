# CFA4 – R0 újraalapozás: Ubuntu 26.04 LTS

Nyomon követés: https://github.com/ubuntuokos/CinemaFundArchitecture4/issues/11
Referencia main SHA: `cdf43fd366e378da266cb6d3c94549a45244be1f`. A main változatlan.

## Mért kiindulási állapot (2026-10-21)
- Kötelező célplatform: **Ubuntu 26.04 LTS**. A régi Ubuntu 24.04 CI PASS eredmények nem igazolják a 26.04 célrendszert.
- A main öt történeti GitHub workflow-fájlt tartalmaz: `freebsd.yml`, `nix.yml`, `packaging-lint.yml`, `release.yml`, `windows-arm64.yml`; Ubuntu 26.04 CI nem található ezen a baseline commiton.
- A release és packaging workflow-k mozgó `storytold/craft-common-actions@main` hivatkozást használnak.
- A projekt `docs/gaps.md` G1–G20 ismert hiányosságot jelöl, ezeket újra kell mérni.

## Már mért Ubuntu 26.04 bizonyíték (részleges)
- PR #8 Actions: https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/39553206897
- PR #10 Actions: https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/39553292230
- A GitHub jobnaplók `Ubuntu 26.04.1 LTS`, `ubuntu-26.04` runner image-et igazolnak.
- A Python/Qt6, architecture/assets, vendored docpath jobból mért PASS; a PR #10 UI teszt 79/79 PASS. Ezek csak az adott nem egyesített PR headekhez köthetők.
- A Python job explicit `actions/setup-python` **3.13**; a 26.04 natív Python 3.14-kompatibilitása még nincs igazolva.
- A teljes Rust workspace, CLI, FFmpeg oracle és valamennyi további auditkapu **külön végleges ellenőrzést igényel**.

## R0-folyamat és kapuk

| Szakasz | Kötelező vizsgálat | R0 állapot |
| --- | --- | --- |
| F0-01 | OS/kernel/toolchain, Python 3.14, Qt6 és FFmpeg/ffprobe APV | RÉSZLEGES |
| F0-02 | `cargo metadata`, `cargo build --workspace --locked`, `cargo xtask ci` | NYITOTT |
| F0-03 | Kötelező release workspace FFmpeg oracle tesztek, skip nélkül | NYITOTT |
| F0-04 | Natív GUI, import, edit, save/open, export, audio/hardver | NEM FUTOTT |
| F0-05 | G1–G20 hiányosságok újramérése | NEM FUTOTT |
| F1-01 | Linux app ID, brand, desktop/AppStream/MIME és csomagolás | NYITOTT |
| F1-02 | Release pipeline Linux-only és mozgó upstream workflow refs | NYITOTT |
| SC-01 | Cargo/Python/Qt/FFmpeg SBOM, transzitív license/hash-pin | NYITOTT |
| SC-02 | Upstream Git-blob SHA, NOTICE és LICENSE forrásazonosság | ÚJRAVIZSGÁLANDÓ |
| Vendor-01 | PR #8 teljes Ubuntu 26.04 CI és audit | NYITOTT |
| Vendor-02 | PR #10 teljes Ubuntu 26.04 CI és audit | NYITOTT |

## Bizonyítás és korlát
Minden új kapuhoz OS, runner image, commit SHA, parancs, workflow/job ID és egyértelmű PASS/FAIL/BLOCKED/NOT_RUN státusz szükséges. Korábbi teszteredmény csak történeti hivatkozás. A két PR draft; nincs automatikus merge, main-módosítás vagy kiadási engedély. Az új audit az F0-01 környezeti leltárral kezdődik.

## R0-F0-01 — Kubuntu tesztgép, felhasználói környezetleltár (2026-10-21)

**Forrás:** a felhasználó által ebben a beszélgetésben közölt helyi shell-kimenet. **Bizonyítéktípus:** felhasználó által megadott mérési adat, nem közvetlenül a GitHub Actions által lefuttatott teszt. A tényleges multimédia és Qt/Rust integrációra vonatkozó kapuk ettől még nyitottak.

| Paraméter | Rögzített eredmény | Mérési státusz |
| --- | --- | --- |
| OS | Ubuntu 26.04.1 LTS, Resolute | OBSERVED |
| Kernel | 7.0.0-39-generic | OBSERVED |
| Asztali munkamenet | Wayland, KDE Plasma 6.6.6 | OBSERVED |
| Qt6 fejlesztői pkg-config | Qt6Core 6.10.2 és Qt6Widgets 6.10.2 | OBSERVED |
| Rust | rustc 1.95.0, Cargo 1.95.0 | OBSERVED |
| FFmpeg | 9.0.2 | OBSERVED; APV dekóder még külön vizsgálandó |
| Vulkan instance | 1.4.341 | OBSERVED |
| NVIDIA fizikai GPU | RTX A1000, Vulkan API 1.4.351, driver 615.71.09 | ENUMERATED; valódi GPU-render még nem tesztelt |
| Szoftveres Vulkan GPU | llvmpipe (Mesa 26.2.5, kisak-mesa PPA, LLVM 21.1.8) | ENUMERATED; CPU fallback még nincs tesztelve |
| Kubuntu/Qt indítási figyelmeztetés | QThreadStorage entry destroyed... a `plasmashell --version` végén | WARNING, önmagában nem bizonyít GUI hibát |
| Vulkan ICD figyelmeztetés | `libvulkan_dzn.so` `vkCreateInstance` -9: kihagyott driver; továbbá két DisplayProperties figyelmeztetés | WARNING, az NVIDIA GPU ennek ellenére látható; futó Vulkan render még nem bizonyított |

**R0 döntés:** a tesztgép OS/Wayland/Plasma/QtCore/QtWidgets/Rust/Cargo/FFmpeg/GPU **részleges referencia-leltára rendelkezésre áll**. A gép nem tekinthető teljesen validáltnak.

**Nyitott célzott tesztek:** `python3 --version`; `ffprobe -version`; `ffmpeg -hide_banner -decoders | grep -i apv`; Qt6Qml/Qt6Quick/Qt6Multimedia `pkg-config`; `nvidia-smi`; tényleges GPU-s Vulkan swapchain/present, Wayland + Qt6 GUI smoke, Rust–Qt6 híd, képkocka-előnézet, médiaimport/edit/export és CPU fallback. A szoftveres Vulkan ICD jelenléte nem igazolja az alkalmazás fallback-útját. A Qt `QThreadStorage` figyelmeztetését először nem blokkoló megfigyelésként kezeljük.

**Platformeltérés:** a Kubuntu tesztgép FFmpeg 9.0.2-t használ, míg az Ubuntu 26.04-es GitHub CI rendszerén külön fel kell jegyezni a pontos FFmpeg-verziót. A CI 8+ előellenőrzés sikere nem igazolja az FFmpeg 9-es lokális codec-viselkedést. Az eltérő Mesa/PPA és NVIDIA környezetet reprodukálhatósági kockázatként kell kezelni.

## R0-F0-02 – Helyi Rust/WGPU tesztazonosság (2026-10-21)

A felhasználó által mért `git rev-parse HEAD` SHA: **`cdf43fd366e378da266cb6d3c94549a45244be1f`**; megegyezik a GitHub `main` HEAD-jével. A `git status --short` eredménye nem ismert. A helyi Rust/WGPU/FFmpeg tesztek tehát a *main commitról* származnak, nem a külön vendorizált PR #8/#10 headekről.

| Helyi Ubuntu/Kubuntu 26.04.1 teszt | Mért eredmény |
| --- | --- |
| `cargo test --release --locked -p filmcraft-gpu --lib` | **41/41 PASS** |
| `cargo test --release --locked -p filmcraft-gpu --lib fx_tests` | **12/12 PASS**, 29 szűrve, 3,96 s |
| `FILMCRAFT_REQUIRE_ORACLES=1 cargo test --release --locked -p filmcraft-apv --test oracle` | **3/3 PASS**, FFmpeg 9.0.2 |
| `cargo tree -p filmcraft-gpu` | `wgpu 30.0.1`, `naga 30.0.1`, `ash 0.38.0+1.3.281` |
| Vulkan diagnosztikai réteg | `VK_LAYER_KHRONOS_validation` nem látszik a keresésben; opcionális |

**GitHub Ubuntu 26.04 CI:** PR #8 és PR #10 tesztjei a különböző grafikus adapterkörnyezetben továbbra is **9 GPU-effekthibával** zártak, bár az APV oracle 3/3 PASS FFmpeg 8.0.1 alatt. A konkrét adapter-/Rgba32Float-képességeltérés még nem bizonyított. Következő lépés: CI adapter- és textúraformátum-leltár, valódi GPU-paritás kontra CPU-fallback tesztkapuk külön validálása; PR-ek nem merge-ölhetők audit PASS jelzés nélkül.
