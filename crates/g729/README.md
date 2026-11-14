# cfa4-g729

Pure-Rust ITU-T G.729 speech decoder (8 kbit/s CS-ACELP, 8 kHz) with G.729 Annex B comfort noise
(SID frames, untransmitted and lost frames), ported from the fixed-point decoder of
[bcg729](https://github.com/BelledonneCommunications/bcg729) 1.1.2 (GPL-3.0-or-later,
Copyright (c) 2011-2019 Belledonne Communications SARL). Codec backlog item CB-11.2.

- `Decoder::decode` takes one frame (`Frame::Speech` with 10 bytes, `Frame::Sid` with 2 bytes,
  `Frame::Erased`) and returns 80 samples, exactly as `bcg729Decoder`.
- `decode_file` reads a raw stream of 10-byte frames (`.g729`, ffmpeg's `g729` format) or an
  ITU-T serial bitstream (`.bit`: sync word `0x6b21`/`0x6b20`, bit count, one 16-bit word per bit).
- The RFC 3389 comfort-noise payload of bcg729 and G.729 Annex A / D / E are not ported.

## Tests

- `tests/patterns.rs`: bit-exact against bcg729's decoder test patterns (the ITU-T G.729 and
  Annex B test vectors with bcg729's output), when the pattern archive is present locally
  (`target/fixtures/g729/p/patterns`, see the file).
- `tests/bcg729_oracle.rs`: bit-exact against bcg729 itself through the helper
  `cfa4-g729-oracle` (`scripts/ci/install-g729-oracle.sh`, built from the pinned 1.1.2 tag, in CI):
  a synthetic speech signal encoded by bcg729 with and without Annex B VAD / DTX, and random
  streams with SID, untransmitted and erased frames.
- `tests/oracle.rs`: against ffmpeg's G.729 decoder on synthetic streams. ffmpeg's decoder is a
  separate implementation that does not match the ITU-T reference sample for sample, so this
  checks the signal-to-noise ratio only.
- `crates/codecs/tests/g729_file.rs`: a `.g729` file through the media source.
