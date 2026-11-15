# cfa4-cinema-pkg

Identifiers and names of digital cinema and IMF packages (CINECERT WP08, WP09 and WP13 of
[`docs/plans/CFA4-CINECERT-001-v2.3.md`](../../docs/plans/CFA4-CINECERT-001-v2.3.md)). Layer L1, no
`unsafe`, no panics on malformed input.

## API

```rust
use cfa4_cinema_pkg::{mxf_digest, MxfDigest, vtfp};

let (digest, packets) = mxf_digest(std::io::BufReader::new(file))?; // reads once, 64 KiB at a time
let urn = digest.to_urn();                                            // urn:smpte:mxf-digest:…
let same = MxfDigest::from_urn(&urn)? == digest;

let tracks = vtfp::virtual_tracks(&cpl_xml)?;                         // TrackId, sequence kind, resources
let fp = vtfp::fingerprint(&cpl_xml, &tracks[0].track_id)?;           // accepts "urn:uuid:…" too
let urn = fp.to_urn();                                                // urn:smpte:imf-vtfp:<40 hex>
let ok = vtfp::urns_match(&urn, "urn:smpte:imf-vtfp:190fb72cdd")?;    // abbreviated form
```

```rust
use cfa4_cinema_pkg::isdcf::{dcnc, Registry};

let reg = Registry::builtin()?;                                       // offline ISDCF snapshot
for f in dcnc::check("Film_FTR-1_F_EN-XX_US-13_51_2K_DI_20261115_DLB_SMPTE_OV", reg) {
    eprintln!("{f}");                                                 // DCNC.<RULE> (field n): …
}
let mut name = dcnc::Name::parse(title)?;                             // fields, then to_string()
```

A writer that produces packets can digest them itself: `packet_digest(packet)` per KLV packet (in
parallel if it wants), then `SequenceDigest::push` in file order and `finish`.

## Specifications

| Identifier | Source (implemented from its text) | Test oracle |
|---|---|---|
| MXF-DIGEST | CineCert / HPA draft memo, <https://github.com/cinecert/mxf-digest>, commit `512e8a9723b8ffd4181a4e2e443fb7e371cc17ce`, `readme.md`; C4 Base58 of SMPTE ST 2114 5.1 | the memo's `test/oneframe.pcm.mxf`, fetched by `scripts/dev/setup-cinema-oracles.sh --corpus` (test `tests/mxf_digest_corpus.rs`, skipped without `CFA4_CINEMA_CORPUS`) |
| IMF-VTFP | CineCert / HPA draft memo, <https://github.com/cinecert/imf-vtfp>, commit `3528343079a5ec9565a4bf4aa17674c6cdbb1955`, `README.md`; SMPTE ST 2067-3 composition playlist | the memo's five test compositions, committed in `tests/data/imf-vtfp/` (BSD-2-Clause, `LICENSE.md`) |

### Where the memos leave room

- **Run-in:** MXF-DIGEST starts at the header partition pack key; up to 65 535 bytes of run-in
  (ST 377-1 6.5) are searched and skipped. A file without one in that range is `Error::NotMxf`.
- **BER lengths:** the length field is digested as written. The indefinite form (`0x80`) and lengths
  over 8 bytes are `Error::BadLength`; a length past the end of the file is `Error::Truncated`.
- **Stereoscopic resources (VTFP):** the memo's text does not say where the duration of a
  `StereoImageTrackFileResource` comes from when the outer `SourceDuration` is missing. Test
  composition 5 only matches when it is the eyes' `SourceDuration`, as the memo's reference script
  does; the two eyes must agree, and an outer `SourceDuration`, if present, must equal theirs.
  Anything else is `Error::Cpl`.
- **Defaults:** `EntryPoint` 0, `SourceDuration` = `IntrinsicDuration` − `EntryPoint`,
  `RepeatCount` 1; negative, non-numeric or out-of-range values are `Error::Cpl`.

### Hostile input

CPLs with a DTD are refused (no entity expansion), and the parsed node count is capped at 2 million.
The MXF reader never allocates according to a declared length. The unit tests run mutated and
truncated inputs under `catch_unwind`.

## ISDCF registries and naming (WP09)

`data/isdcf-registry.json` is an offline snapshot of the ISDCF metadata registries
(<https://github.com/ISDCF/registries>, CC BY 4.0, © ISDCF; attribution in the `.attribution`
sidecar and `ATTRIBUTION.md`). It holds the codes the naming convention uses (content types and
modifiers, aspect ratios, audio, languages, territories, studios, facilities) with their obsolete
codes, plus the upstream commit and the SHA-256 of every upstream file. Contact details are left
out. The ratings registry is not included: it is derived from MovieLabs data whose licence is not
stated, so ratings are checked for form only.

Nothing is fetched at run time. `bash scripts/dev/update-isdcf-snapshot.sh --check` reduces the
pinned upstream commit again and compares (offline / online parity, test `tests/isdcf_snapshot.rs`);
`--update COMMIT` rewrites the snapshot from a newer commit for review.

`isdcf::dcnc::check` reports where a content title deviates from the ISDCF Digital Cinema Naming
Convention (rules from the ISDCF registry pages, read 2026-11-15). The convention is not a
standard, so every finding is a warning (`DCNC.FIELDS`, `DCNC.TITLE`, `DCNC.CONTENT_TYPE`,
`DCNC.MODIFIER_ORDER`, `DCNC.ASPECT`, `DCNC.LANGUAGE`, `DCNC.TERRITORY`, `DCNC.AUDIO`,
`DCNC.RESOLUTION`, `DCNC.STUDIO`, `DCNC.DATE`, `DCNC.FACILITY`, `DCNC.STANDARD`, `DCNC.3D`,
`DCNC.PACKAGE`, `DCNC.OBSOLETE`). Where the pages disagree, the checker accepts both: `NULL` and
`NUL` for an unknown facility; ratings are not checked against a list. Free modifiers such as a theatre
chain are not judged; known ones must follow the registry order.

The export (`cfa4-export`, folder name) and the validator (`cfa4-dcp-validate`, `DCP.NAME.001`)
keep their own naming code for now; moving them onto this checker is for their owners
(PROCODEC-002, DCP-VALIDATION).

## Delivery and DCP drives (WP13)

`delivery::deliver(package_dir, target)` copies a finished package so that a damaged or partial
copy can never pass for a complete one:

- file names are checked first (`check_names`: ASCII `A-Z a-z 0-9 . _ -`, no spaces, at most 255
  bytes); symbolic links are refused. A package that fails is not copied at all;
- files go, in sorted order, into `NAME.INCOMPLETE/`, each written under a temporary name,
  synced, renamed and read back; its SHA-256 must equal the source's;
- the manifest `NAME.sha256` (`sha256sum` format, sorted) is written, and `NAME.INCOMPLETE` is
  renamed to `NAME` as the last step. An existing `NAME` is never overwritten;
- after a failure or a cancel (`deliver_with` asks a callback before every 1 MiB step), running it
  again resumes: files recorded as verified are checked again and kept, the rest is copied.

`delivery::verify(package, manifest)` lists missing, changed and unexpected files. The tests
interrupt a delivery, damage a verified file before resuming, damage a delivered package, and
compare every delivered byte with the source (the WP13 gate).

`drive::inspect(reader)` reads a disk image or a read-only opened block device and compares it
with the ISDCF drive recommendation: MBR, one Linux (0x83) partition, ext2 or ext3, 128-byte
inodes. It never writes; `drive::format_hint` gives the `mkfs.ext3 -I 128` command for the
operator. Tested on synthetic images and, where `mkfs.ext2/3/4` are installed, real ones.

Still open in WP13: the provenance report and optional BagIt archive packaging, and multi-volume
DCPs (`VOLINDEX`, `ASSETMAP` `VolumeCount`; the latter changes the export writers, so it is agreed
with PROCODEC-002 first).
