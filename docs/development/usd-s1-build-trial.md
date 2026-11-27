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

## S3: reproducible build (SB-G3)

`scripts/dev/build-usd.sh --check-repro` (same toolchain, 4 cores, 2026-11-27): two builds one after the other, with the
same prefix and work directory, produced **identical hashes for all 1212 installed files** (`SHA256SUMS`); 22 min 40 s for both.
Determinism comes from `SOURCE_DATE_EPOCH` (the pinned commit time), `-ffile-prefix-map`, `LC_ALL=C`/`TZ=UTC`, `RUNPATH $ORIGIN`,
a fixed `strip` and a sorted hash list. `BUILDINFO.json` (pins, toolchain, flags) is written next to it and is not part of the hashed set.
Not covered: a different build directory or host path (the install prefix is embedded in CMake config files, so the prefix must stay the same),
a different compiler or distribution (the manual workflow `cfa4-usd-build.yml` runs Ubuntu 26.04 and 24.04; not yet run).

## S4: packaging trial (SB-G5)

`packaging/linux/stage-usd.sh` stages the runtime part of a `build-usd.sh` install and writes a tar.gz and a .deb
(`cfa4-usd-worker`, USD runtime only; the worker itself comes in U2). Same machine and toolchain as above.

| Item | Measured |
|---|---|
| Installed size, runtime only (no headers, no CMake files) | 41 272 KiB (the full install was 63 MB) |
| tar.gz | 11 833 731 bytes |
| .deb (xz) | 7 999 024 bytes |
| Determinism | two runs give byte-identical tar.gz and .deb |
| Licences in the package | OpenUSD LICENSE and NOTICE, oneTBB LICENSE and third-party-programs, generated THIRD-PARTY-NOTICES.txt (header texts of the 7 bundled libraries), `usr/share/doc/cfa4-usd-worker/copyright`, BUILDINFO.json, SHA256SUMS |
| Test | `packaging/linux/test_stage_usd.py` (synthetic prefix, offline; part of `scripts/ci/check-linux-amd64.sh`) |

The main CFA4 package does not change: the USD runtime is a separate optional package, so non-USD users pay no size.

**Not done:** AppImage (no `appimagetool` here; the tar.gz layout is what the AppImage would hold) and Flatpak (a Flatpak source needs a download URL with stable bytes; our pin is the SHA-256 of a `git archive`, so this needs the CFA4 source mirror, D-SB-3). The .deb was built but not installed or linted (`lintian` not run). PEGTL has no licence text in the source tree (its header points to the upstream LICENSE); the notices file says so by quoting only the header. SB-G5 is therefore met for tar.gz and .deb, **not** for AppImage and Flatpak.
