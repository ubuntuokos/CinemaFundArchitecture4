# cfa4-twinvq

Pure-Rust decoder for **NTT TwinVQ** audio in VQF files (`.vqf`, `.vql`, `.vqe`), codec backlog CB-31.2.

- `parse` reads the header like ffmpeg's `vqf` demuxer; `decode` cuts the bit-packed frames and
  returns each channel's samples as floats (the first two frames only prime the overlap, as in ffmpeg).
- All nine VQF modes: 8 kHz/8, 11 kHz/8, 11 kHz/10, 16 kHz/16, 22 kHz/20, 22 kHz/24, 22 kHz/32,
  44 kHz/40 and 44 kHz/48 kbit/s per channel, mono and stereo.
- Ported from FFmpeg n7.1.1 (`libavformat/vqf.c`, `libavcodec/twinvq*.c`, the codebooks of
  `twinvq_data.h`); LGPL-2.1-or-later, see `COPYING.LGPL-2.1`. CFA4 changes are GPL-3.0-or-later.
  No `unsafe`. The inverse MDCT is a DCT-IV through an FFT in double precision.
- Test oracle: ffmpeg on FFmpeg's FATE sample (`vqf/achterba.vqf`) and on synthetic random frames
  in every mode; MetaSound variants (not VQF) are not covered.
