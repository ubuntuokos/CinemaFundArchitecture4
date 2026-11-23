# cfa4-yamaha-oki

Pure-Rust decoders for **Yamaha ADPCM** (WAV format tag 0x0020, the AICA / YMZ family) and **Dialogic OKI
ADPCM** (WAV tag 0x0010, the MSM6295 / VOX family), codec backlog CB-31.1.

- Mono and stereo; two codes per byte (Yamaha: low nibble first; OKI: high nibble first). In mono both
  nibbles run through one decoder state, in stereo they alternate between the channels.
- `State` is `Clone`, so a reader can checkpoint the stream and seek (`cfa4-media` keeps one every 16 KiB).
- Ported from FFmpeg n7.1.1 (`libavcodec/adpcm.c`, `adpcm_data.c`); LGPL-2.1-or-later, see
  `COPYING.LGPL-2.1`. CFA4 changes are GPL-3.0-or-later. No `unsafe`.
- Test oracle: ffmpeg (`adpcm_yamaha` decode and encode, `adpcm_ima_oki` decode) on random nibble streams.
