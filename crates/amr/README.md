# cfa4-amr

Pure-Rust decoders for the 3GPP Adaptive Multi-Rate speech codecs (codec backlog CB-6.2):

| Codec | Source | Containers (wired in `cfa4-media` / `cfa4-codecs`) |
|---|---|---|
| AMR-NB (8 kHz, 4.75–12.2 kbit/s, DTX) | opencore-amr 0.1.6 `amr_nb` decoder (3GPP TS 26.073 / 26.090 / 26.091 / 26.092 / 26.093), ported | `.amr` (RFC 4867 §5 storage format), MP4 / 3GP / QuickTime (`samr`), Matroska (`A_MS/ACM` 0x38) |
| AMR-WB (16 kHz, 6.6–23.85 kbit/s, DTX) | opencore-amr 0.1.6 `amr_wb` decoder (3GPP TS 26.173 / 26.190 / 26.191 / 26.192 / 26.193), ported | `.awb`, MP4 / 3GP / QuickTime (`sawb`), Matroska (`A_MS/ACM` 0x58) |

`Decoder` takes storage-format frames (a table-of-contents byte and the payload, as in the
`.amr` / `.awb` files and the 3GP sample entries) and gives 16-bit samples, 160 (NB) or 320 (WB)
per frame; `decode_file` reads a whole storage file. Both decoders carry their whole state from
frame to frame and never converge after a jump (the long-term predictor carries every difference
on), so the media sources keep decoder checkpoints every 250 packets
(`cfa4_codecs::audio::Checkpoints`) and a seek is exact.

## Fidelity

The decoders are ports of opencore-amr (Apache-2.0, see `LICENSE.opencore-amr` and
`NOTICE.opencore-amr`; each file names its source files) and are bit-exact with it in every mode,
in DTX (SID and comfort noise), with lost, damaged and NO_DATA frames, and with decoder homing
frames (`tests/opencore_oracle.rs`; container round trips and seeks in
`crates/codecs/tests/amr_oracle.rs`). The oracle is `cfa4-amr-oracle`
(`scripts/ci/install-amr-oracle.sh`): opencore-amr and vo-amrwbenc built from pinned tags without
link-time optimisation, an external test tool only, never linked or shipped. opencore's quirks are
kept, because they are what every AMR stream in the wild was checked against:

- the reference arithmetic wraps (`-fwrapv`), and shift counts are masked as on x86;
- the AMR-WB wrapper reuses its bit buffer, so a NO_DATA frame sees the previous frame's bits;
- the AMR-WB gain concealment shifts only the first element of its gain histories, and a bad
  frame keeps the last predicted energy.

## Robustness

No `unsafe`, no panics on any input (random frames in `tests/opencore_oracle.rs`): an unknown or
truncated frame decodes as a lost frame, all indexing is bounded.
