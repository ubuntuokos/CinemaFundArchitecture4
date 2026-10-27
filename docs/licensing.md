# CFA4 licensing policy

## Összefoglaló (magyarul)

- A CFA4 licence **GPL-3.0-or-later** (GNU GPL 3. vagy későbbi változat). A teljes szöveg a
  [`LICENSE`](../LICENSE) fájlban van.
- Az upstream projektből (lásd `NOTICE`) örökölt kód és eszközök az eredeti **MIT OR Apache-2.0** feltételek alatt is
  elérhetők maradnak; a `LICENSE-MIT`, `LICENSE-APACHE` és `NOTICE` fájlokat meg kell tartani.
- **Átvehető kód:** bármely GPL-3.0-kompatibilis licencű projektből (GPL-2.0-or-later, GPL-3.0,
  LGPL-2.1-or-later, LGPL-3.0, MIT, BSD, Apache-2.0, MPL-2.0 …), forrásmegjelöléssel és az eredeti
  szerzői jogi fejlécek megtartásával. Minden átvételt be kell írni a lenti nyilvántartásba.
- **Nem vehető át:** GPL-2.0-only kód (pl. HandBrake), AGPL, nem szabad licencű vagy ismeretlen
  licencű kód, és bármely zárt program (Adobe Premiere, DaVinci Resolve) kódja vagy eszközei.
  Ezeknek a képességét saját kóddal vagy külső programként hívva kell megvalósítani.
- **Fizetős rész:** a CFA4 alapprogram ingyenes és szabadon terjeszthető. Fizetős lehet a
  hosszabb támogatás (LTS-ágak, frissítések), a kész csomagok és a szolgáltatások, a nem kód
  jellegű tartalmak, valamint a **GPL-3.0-or-later licencű fizetős kiegészítők** (a tulajdonos
  döntése, 2026-10-24): eladhatók, de a vevő a forrással együtt továbbadhatja őket. Zárt (nem GPL)
  kódmodul – sem a CFA4 folyamatába betöltődő, sem külön folyamatként futó – nem készül.
- **Szabadalmilag korlátozott komponensek – Ubuntu-modell (tulajdonosi döntés, 2026-10-25):**
  az alapcsomag nélkülük épül és működik; a felhasználó kifejezett kérésére a
  `cfa4-restricted-extras` kiegészítő csomag telepíti őket (pl. x264, x265). Ez minden jövőbeli
  hasonló esetben a követendő út. A CFA4 saját kodekjei az alapcsomagban maradnak. Részletek: 7. pont.
- A **CFA4 logó** (név, embléma, szóvédjegy, ikonok) István saját, védett jele, **nem** GPL alatti; a
  feltételek: [`assets/cfa4/LICENSE-logo.txt`](../assets/cfa4/LICENSE-logo.txt). A kód GPL marad.

*Ez a dokumentum projektszabályzat, nem jogi tanácsadás. Kereskedelmi kiadás előtt érdemes
jogásszal átnézetni.*

---

## 1. Project licence

CFA4 is distributed under the **GNU General Public License, version 3 or (at your option) any
later version** (SPDX: `GPL-3.0-or-later`). The full text is in [`LICENSE`](../LICENSE).

- New source files carry the header (or at least the SPDX line):

  ```rust
  // SPDX-License-Identifier: GPL-3.0-or-later
  // Copyright (C) <year> <author> and the CFA4 contributors
  ```

- Contributions are accepted under `GPL-3.0-or-later` (inbound = outbound). No contributor licence
  agreement (CLA) is required at the moment; if one is introduced later it applies only to new
  contributions.

## 2. Inherited upstream code

CFA4 derives from an upstream open-source video editor (its copyright notice is kept verbatim in
[`NOTICE`](../NOTICE)), which is licensed `MIT OR Apache-2.0`. Both licences allow
the code to be combined into a GPL-3.0-or-later work. The obligations that remain:

- keep `LICENSE-MIT`, `LICENSE-APACHE` and the upstream notice in [`NOTICE`](../NOTICE), in the
  repository and in every package (`packaging/env.sh` `copy_docs` ships them);
- keep existing copyright lines in inherited files; do not replace them, add to them;
- existing `*.attribution` sidecars keep the licence the asset was published under.

The upstream name, wordmark and logos are trademarks, not covered by any of these licences; they
have been removed from CFA4 and must not be reintroduced (see `AGENTS.md`).

### The CFA4 logo

The CFA4 name, emblem and wordmark are the project owner's own mark (owner decision 2026-10-25) and
are **not** under GPL-3.0-or-later; the code stays GPL. Every file depicting them is licensed under
[`assets/cfa4/LICENSE-logo.txt`](../assets/cfa4/LICENSE-logo.txt) (`LicenseRef-CFA4-Logo`): it may be
kept in copies of the repository, used in official builds and documentation and shipped in
unmodified CFA4 packages, but only unmodified; forks and modified versions must remove or replace
it and must not call themselves CFA4.

## 3. What code may be reused

Allowed, with provenance (section 5):

| Source licence | Reuse in CFA4 | Notes |
|---|---|---|
| GPL-3.0-only, GPL-3.0-or-later | yes | e.g. OpenShot, Subtitle Edit, Krita, Audacity 3.x |
| GPL-2.0-or-later | yes (used under GPL-3.0) | e.g. most of Kdenlive/KDE; check every file header |
| LGPL-2.1-or-later, LGPL-3.0 | yes | linking or copying both fine inside a GPL work |
| MIT, BSD-2/3-Clause, ISC, Zlib, 0BSD, BSL-1.0, Unicode, Unlicense, CC0 | yes | keep the notice |
| Apache-2.0 | yes | GPL-3 compatible; keep NOTICE text if the source has one |
| MPL-2.0 | yes | MPL files stay MPL; modifications to those files are published |

Not allowed:

| Source | Why | Alternative |
|---|---|---|
| GPL-2.0-only (e.g. HandBrake, parts of the Linux kernel) | incompatible with GPL-3.0 | call the program as an external process, or reimplement |
| AGPL-3.0 | network-use obligations we do not want on the whole product | reimplement |
| GPL code linked with GPL-incompatible libraries | cannot be distributed | — |
| Proprietary software (Adobe Premiere, DaVinci Resolve, …) | no licence | reimplement the behaviour from public documentation and observation; never copy code, icons, presets, LUTs or fonts |
| Unknown / no licence | all rights reserved | ask the author or leave it out |

A project's overall licence is not enough: **check the header of every file you take**, because
large projects mix licences (some Kdenlive or Krita files are GPL-2.0-only, some FFmpeg parts are
GPL while the default build is LGPL).

External programs (ffmpeg, HandBrakeCLI, …) started as separate processes and communicating
through files, pipes or command-line arguments are not part of CFA4's code and do not change its
licence. If such a program is bundled in a package, its own licence text and source offer must
ship with it.

## 4. Free core, paid extras

The CFA4 core is free software: anyone may download, use, change and redistribute it under the
GPL. Revenue comes from things that are compatible with that:

1. **Extended support:** long-term-support branches, backported fixes and updates for older
   Ubuntu releases, direct help. The code of any released LTS build is still GPL and its source is
   provided to its recipients.
2. **Convenience:** signed, ready-made packages (deb, Snap, Flatpak) with automatic updates.
3. **Paid extras are GPL too** (owner decision, 2026-10-24): extra features sold on top of the
   free core are licensed `GPL-3.0-or-later`, like the core. They may be loaded into the CFA4
   process or run separately; buyers receive the source and may redistribute it. Rules:
   - every distributed binary comes with its corresponding source (GPL-3.0 §6): a source archive or
     a link to the exact source version, built into the release chain before the release hold is
     lifted;
   - CFA4 works fully without them (no artificially crippled core).
4. **Content and services:** templates, effect presets, AI models, sample media, cloud or hosted
   processing. Non-code content can carry its own (also paid) licence.

**Not part of the model:** proprietary (non-GPL) code extras of any kind, whether loaded into the
CFA4 process (dynamic libraries, Rust crates, Python modules) or shipped as separate programs.
Third-party proprietary plug-ins are not loaded either (ADR 0003 §4).

## 5. Reused code register

Every piece of code taken from another project is listed here (one row per source, file or
directory). The copied files keep their original copyright and licence headers, and the commit
that adds them names the source and revision.

| CFA4 path | Origin (project, URL, revision, path) | Copyright holders | Licence |
|---|---|---|---|
| `crates/dts/src/` (except `cfa4.rs`) | `dca` crate 0.9.0 of harletty-bridge, <https://github.com/harletty/harletty-bridge>, commit `74cf6edaef6742777d3eb4da35387fd370f2e9f3`, `dca/src/`; itself a port of FFmpeg `libavcodec/dca*.c`, `dcadsp.c`, `dcadct.c`, `synth_filter.c` | foo86 and the FFmpeg developers; the harletty-bridge contributors | LGPL-2.1-or-later (`crates/dts/COPYING.LGPL-2.1`; published by harletty-bridge as Apache-2.0, `crates/dts/LICENSE.harletty-bridge`). FFmpeg-derived code, an owner-approved exception to the FFmpeg-only-in-tests rule (#83) |
| `crates/jxl/src/` (105 files, each with a provenance header) | jxl-oxide, <https://github.com/tirr-c/jxl-oxide>, crates jxl-oxide 0.12, jxl-render 0.12.4, jxl-bitstream 1.1.0, jxl-modular, jxl-vardct, jxl-frame, jxl-image, jxl-grid, jxl-color, jxl-coding, jxl-oxide-common, jxl-threadpool (crates.io releases of 2025; repository revisions 74c45ba5, 1d4e4289, f37ac036, b2042ccf); rewritten without `unsafe` and panics | Wonwoo Choi and the jxl-oxide contributors | MIT OR Apache-2.0 (`crates/jxl/LICENSE-MIT`, `crates/jxl/LICENSE-APACHE`); CFA4 changes GPL-3.0-or-later |
| `crates/opus/src/celt/encoder.rs`, encoder parts of `crates/opus/src/celt/{bands,energy,mdct}.rs` | RFC 6716 reference implementation (libopus `celt/`): algorithm structure and tuning constants of the encoder analysis (transient detector, stereo / intensity decisions, PVQ search, coarse / fine energy quantisation, forward MDCT folding), reimplemented in Rust; no files copied | IETF Trust and the RFC 6716 authors (Xiph.Org Foundation, Skype Limited, Octasic, J.-M. Valin, T. B. Terriberry, and others) | BSD-3-Clause |
| `crates/cineform/src/tables.rs` (codebooks), and the tag syntax, companding and inverse 2/6 wavelet re-expressed in `crates/cineform/src/{decode,vlc,wavelet}.rs` | GoPro CineForm SDK, https://github.com/gopro/cineform-sdk, rev. `11574d0295771edccadd17af14af74a539f924c7`, `Common/table9.inc`, `Common/table17.inc`, `Codec/codec.h`, `Codec/spatial.c`, `Codec/InvertHorizontalStrip16s.c`, `Codec/vlc.c` | (C) 2017 GoPro, Inc. | Apache-2.0 OR MIT (MIT used) |
| `crates/j2k/src/ht.rs`, `crates/j2k/src/ht_tables.rs` (HTJ2K block decoder, ported) | OpenJPH, https://github.com/aous72/OpenJPH, 0.28.1 (Ubuntu `openjph_0.28.1.orig.tar.gz`), `src/core/coding/ojph_block_decoder32.cpp`, `ojph_block_common.cpp`, `table0.h`, `table1.h` | Aous Naman; Kakadu Software Pty Ltd; The University of New South Wales | BSD-2-Clause (`crates/j2k/src/LICENSE-OpenJPH`) |

## 6. Dependency checks

- Rust dependencies: `deny.toml` lists the licences that are GPL-3.0 compatible. Run
  `cargo deny check licenses` (installed with `cargo install cargo-deny --locked`); the manual full
  validation in CI runs it too. Dual-licensed crates (`MIT OR Apache-2.0 OR LGPL`, …) pass as long
  as one option is allowed.
- Security advisories: the same CI step runs `cargo deny check advisories` against the RustSec
  database. A new advisory fails the build; a temporary exception goes into `deny.toml`
  `[advisories].ignore` with a reason, and is removed when the dependency moves.
- Python: `PySide6`/Qt 6 (LGPL-3.0) is compatible. GPL-only Qt modules are compatible with CFA4's
  licence as well, but must be listed in `NOTICE` when shipped.
- Packages must ship `LICENSE`, `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE` and the licences of
  bundled fonts and programs.

## 7. Patent-restricted components (Ubuntu model)

Owner decision, 2026-10-25 (recorded in [ADR 0003](adr/ADR-0003-aswf-and-conversion-sources.md)
§12). Patents are separate from copyright licences: a component can be free software (x264 and
x265 are GPL) and still implement a patent-encumbered format. Ubuntu does not install such codecs
by default; the user opts in (`ubuntu-restricted-extras`). CFA4 follows the same model, and it is
the path for **every** such component, now and in future:

1. **The base package builds and runs without them.** No mandatory link dependency: the FFmpeg
   backend and every other part of CFA4 start when they are absent.
2. **They ship in a separate package, `cfa4-restricted-extras`,** installed only on the user's
   explicit request (from the package manager, or from within CFA4 after a confirmation).
3. **Run-time detection:** CFA4 checks whether a component is installed. If it is not, the feature
   stays visible but disabled, with the option to install it; a missing component never causes an
   error or a crash.
4. **Information before installation:** the user is told that the component may be restricted in
   some countries and that checking whether its use is permitted is up to them.

Current classification:

| Component | Package |
|---|---|
| x264, x265 (software H.264 / HEVC encoders, through the FFmpeg backend) | `cfa4-restricted-extras` |
| CFA4's own codecs (H.264, HEVC, AAC, AC-3 and the other decoders, the H.264 encoder) | base package (owner decision, 2026-10-25) |

A new patent-encumbered component goes to `cfa4-restricted-extras` unless the owner decides
otherwise; record each decision in the table above.
