# cfa4-cinema-pkg

Identifiers of digital cinema and IMF packages (CINECERT WP08 of
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
