<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# OpenUSD minimal build trial (S1, G-USD-1 path (b))

Measured 2026-11-27 in the cloud container, not on the owner's reference machine (SB-G9 stays open).
Sources fetched with `scripts/dev/fetch-usd-source.sh` (pins in `packaging/usd-version.env`).

| Item | Result |
|---|---|
| Toolchain | g++ 13.3.0 (Ubuntu 24.04), cmake 3.28.3, ninja 1.11.1. Ubuntu 26.04 / newer GCC not tried. |
| oneTBB 2021.12.0 | built and installed in 13 s, 600 KB of libraries |
| OpenUSD 26.11.1 | configure 1 s, build and install 683 s (1827 ninja steps) |
| Options | Python, imaging, usdImaging, usdview, tests, examples, tutorials, docs, tools, exec, MaterialX, OpenVDB, Ptex, OSL, GL, Vulkan all OFF |
| Install size | 63 MB total: 40 shared libraries, 53 MB `lib`, 9.8 MB `include` |
| `ldd` (SB-G4) | the libraries link only libtbb, libstdc++, libm, libgcc_s, libc and the loader. No boost, python, zlib or GL. |

Result: the minimal core builds with oneTBB as the only external library. SB-G4 holds on this toolchain.
Not covered: tag signature (D-SB-4), licence decision (SB-G7), physical measurement (SB-G9), a second compiler.
