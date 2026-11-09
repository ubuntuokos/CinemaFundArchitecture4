# cfa4-adpcm

Clean-room, pure-Rust decoders for telephony and early PC / QuickTime audio codecs (codec backlog
CB-4.1 and CB-4.2):

| Codec | Source | Containers (wired in `cfa4-media` / `cfa4-codecs`) |
|---|---|---|
| G.711 μ-law, A-law | ITU-T G.711 (11/88) | WAV, AVI, MOV / QuickTime (`ulaw`, `alaw`), Matroska (`A_MS/ACM`) |
| G.722 (64 kbit/s, mode 1) | ITU-T G.722 (11/88) §5–6 | WAV, AVI, Matroska (`A_MS/ACM`) |
| IMA ADPCM | IMA Recommended Practices (1992); Microsoft `WAVE_FORMAT_IMA_ADPCM` block layout | WAV, AVI, Matroska; QuickTime `ima4` |
| Microsoft ADPCM | Microsoft `WAVE_FORMAT_ADPCM` / `ADPCMWAVEFORMAT` | WAV, AVI, Matroska |

`Codec` names a stream's format (from a `WAVEFORMATEX` or a QuickTime fourcc), `Decoder` turns
packets into planar 16-bit samples. The block formats decode each block on its own. G.722 carries
its state from packet to packet (the WAV source keeps decoder checkpoints every 16 KiB of data, so
a seek is exact; packet containers pre-roll eight packets, after which the decoder has converged).
QuickTime `ima4` continues from the previous packet's last sample when the packet header's 9-bit
predictor matches it (one packet of pre-roll restores that).

## Fidelity

Every decoder is bit-exact with ffmpeg's (an external oracle only, never linked) on ffmpeg-encoded
tones, sweeps, noise and clipped signals, and on random codewords and blocks
(`tests/ffmpeg_oracle.rs`; container round trips in `crates/codecs/tests/adpcm_oracle.rs`). Where a
specification leaves a choice to the implementation, the choice was fitted black-box against
ffmpeg's output:

- G.722: the receive QMF's final scaling (Σ >> 11 to 16 bits, saturated); UPPOL2 negates the
  saturated 4·AL1 with 16-bit saturation (0 − (−32768) = 32767), as the Recommendation's arithmetic
  says.
- IMA ADPCM: WAV blocks compute the difference exactly, `((2·|code| + 1) · step) >> 3`; QuickTime
  `ima4` builds it from shifts and adds (`step/8 + …`).
- Microsoft ADPCM: the prediction divides by 256 (rounding toward zero); the adaptive delta is
  capped at `i32::MAX / 768`.

## Robustness

No `unsafe`, no panics on any input: damaged headers are refused when the stream is opened,
a damaged Microsoft ADPCM block decodes to silence of its length, and all arithmetic is bounded.
