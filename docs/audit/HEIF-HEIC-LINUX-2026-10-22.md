# CFA4 – HEIF/HEIC Linux AMD64 integration audit

**Date:** 2026-10-22
**Branch:** `chore/f1-02-vendor-ui` (draft PR #10)
**Target:** Ubuntu 26.04.1 LTS, Linux x86_64/AMD64 only
**Review decision:** IMPLEMENTED IN SOURCE / NATIVE TEST VALIDATION PENDING / RELEASE HOLD

## Delivered

- `crates/media/src/still/heif.rs`: no-unsafe, local libheif subprocess adapter using `heif-dec`/`heif-enc`. Private 0700 temporary workspace, cleanup on errors, bounded input/output and decode allocation, descriptive errors if runtime codecs are absent.
- `crates/media/src/still.rs`: HEIF/HEIC magic sniffing and still-image decoding, Linux AMD64 platform-gated.
- `crates/media/src/lib.rs`: `.heic`, `.heif`, `.hif` import extensions; HEIF detection before ISO-BMFF movie openers.
- `crates/media/src/reader.rs`: prevent HEIF from being mistaken for MP4 by random-access reader openers, with a bounded image read.
- `crates/export/src/still.rs`: `StillFormat::Heic` and `StillFormat::Heif` single-image HEVC/HEIF output via local libheif, accepting 8- and 16-bit PNG intermediates.
- Ubuntu bootstrap/CI: `libheif-examples`, `libheif-plugin-libde265`, `libheif-plugin-x265`. Debian packaging recommends the runtime tools rather than bundling them.
- `crates/media/tests/heif_oracle.rs` and export module unit tests: locally generated HEIC roundtrip, import opener precedence, import extensions, invalid-file failure, 8-/16-bit still-export roundtrip. Mandatory oracles run in the manual full CI's Rust workspace job.

## Scope

This integration handles **the first decoded HEIF still image** and **single-frame HEIC/HEIF export** using the distribution's libheif tooling; it does not yet expose multi-picture sequences, depth/auxiliary tracks, full ICC/EXIF transfer or 10-/12-bit HDR frame retention in the CFA4 image engine. The current still-image interface uses RGBA8 and sRGB metadata after decoding. 16-bit export intermediates can be encoded at 10-bit by libheif, but do not imply an HDR-accurate metadata pipeline.

HEIF is a container; a working HEVC video decoder on its own did not satisfy HEIC still-image support. No proprietary HEIF/HEVC binaries, codec source files or multimedia models are copied into CFA4.

## Verification gates

| Gate | Evidence / status |
|---|---|
| HEIF image import registered | Source changes implemented; CI compilation pending |
| HEIF checked before MP4 opener | Source and regression test implemented; CI execution pending |
| heif-dec and heif-enc run on Ubuntu 26.04 | Packages configured in manual full CI; pending native run |
| First-image import + no-crash error behavior | Tests added, native CI execution pending |
| HEIC/HEIF 8-/16-bit still export | Tests added, native CI execution pending |
| Full Rust workspace release/oracle | HOLD: existing GPU FX test failures remain independently open |
| Linux native Qt6 GUI integration | Shared Rust media import list enables existing browser entry points; end-to-end Qt GUI is not yet complete |
| Licensing / patents / redistribution | HOLD: libheif LGPL obligations and HEVC codec/plugin GPL/patent considerations need review before shipping |

**Do not merge or release solely from a green static GitHub Actions gate.** A full native validation is required. The runtime CLI dependencies are *not* linked into the CFA4 Rust binary.
