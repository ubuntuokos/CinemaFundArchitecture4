# cfa4-qdm2

QDesign Music Codec 2 (QDM2) audio decoder in pure Rust, codec backlog CB-36.2. A port of FFmpeg
n7.1.1 (`libavcodec/qdm2.c` with its tables, plus the MPEG audio synthesis filter and the 32-point
DCT it shares with the MP3 decoder), re-expressed without `unsafe`. LGPL-2.1-or-later, see
`COPYING.LGPL-2.1`; CFA4's changes are GPL-3.0-or-later (owner's general permission for FFmpeg
ports, 2026-11-14).

- `Decoder::new(extradata)` reads the QuickTime sound description (the `wave` atom with `frma`
  `QDM2` and `QDCA`): channels, sample rate, bit rate, group size, FFT size and block size.
- `Decoder::decode(packet)` turns one block (`checksum_size` bytes) into 16 sub-frames of
  interleaved 16-bit samples (mono or stereo; FFT sizes 128, 256 and 512).

QDM2 lives in QuickTime `.mov` / MP4 files (`QDM2` sample entry); `cfa4-codecs` reads it there.

## Checks

`tests/oracle.rs` decodes the FFmpeg FATE sample `qdm2/subpacket12.mov` (mono, 44.1 kHz, 8
blocks) and compares with the FATE reference PCM: every sample is identical. The sample is
fetched into `target/fixtures/qdm2/fate/` and the test skips without it. FFmpeg has no QDM2
encoder, so no other stream is available: stereo, the other FFT sizes and the superblock types
the FATE sample does not use are ported from the C code but unchecked against an oracle (like
FFmpeg itself, whose sources mark the `!superblocktype_2_3` branch as untested).
`tests/damaged.rs` feeds random and bit-flipped blocks: errors, never a panic.
