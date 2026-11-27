# cfa4-twinvq

Pure-Rust decoder for **NTT TwinVQ** audio in VQF files (`.vqf`, `.vql`, `.vqe`), codec backlog CB-31.2,
and **Voxware MetaSound** (WAV format 0x0075), CB-37.1.

- `parse` reads the header like ffmpeg's `vqf` demuxer; `decode` cuts the bit-packed frames and
  returns each channel's samples as floats (the first two frames only prime the overlap, as in ffmpeg).
- All nine VQF modes: 8 kHz/8, 11 kHz/8, 11 kHz/10, 16 kHz/16, 22 kHz/20, 22 kHz/24, 22 kHz/32,
  44 kHz/40 and 44 kHz/48 kbit/s per channel, mono and stereo.
- Ported from FFmpeg n7.1.1 (`libavformat/vqf.c`, `libavcodec/twinvq*.c`, the codebooks of
  `twinvq_data.h`); LGPL-2.1-or-later, see `COPYING.LGPL-2.1`. CFA4 changes are GPL-3.0-or-later.
  No `unsafe`. The inverse MDCT is a DCT-IV through an FFT in double precision.
- Test oracle: ffmpeg on FFmpeg's FATE sample (`vqf/achterba.vqf`) and on synthetic random frames
  in every mode.
- MetaSound is the same transform coder with its own tables (`ms_tables.rs`, generated from
  `metasound_twinvq_data.h`), little-endian bit order, one or two frames per packet and its own
  bark / peak-component parameter coding. `parse_metasound(extradata, block_align)` reads the
  WAV format's extra bytes (the mode tag, e.g. `VX03`, `VOXi`..`VOXz`); the mode oracle is an AVI
  of one block per chunk, since ffmpeg's WAV demuxer hands the decoder several blocks per packet
  and it reads only the first.
