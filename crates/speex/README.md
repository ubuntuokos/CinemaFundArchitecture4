# cfa4-speex

Pure-Rust Speex decoder (codec backlog CB-6.1, `docs/plans/CODEC-BACKLOG-PLAN-2026-10-27.md`).

| | |
|---|---|
| Streams | narrowband (8 kHz, submodes 1–8), wideband (16 kHz) and ultra-wideband (32 kHz), CBR, VBR, ABR, VAD / DTX, several frames per packet, in-band intensity stereo |
| Containers (wired in `cfa4-codecs`) | Ogg Speex (`.spx`, `.ogg`; whole stream decoded on open) |
| Not supported | packet-loss concealment (a file decode never loses a packet) |

- `Header::parse`: the 80-byte Speex header packet.
- `Decoder::decode_packet`: one packet → interleaved 16-bit samples, as `speexdec` produces them
  (`speex_decode_int`, then `speex_decode_stereo_int` for stereo), with the perceptual enhancer on.
- `Decoder::lookahead`: the decoder delay `speexdec` trims together with the encoder's (from the
  first page's granule position); `cfa4-codecs` trims the same way.

The decoder reproduces the float build of libspeex to the bit: `f32` where C computes in
`float`, `f64` where a `double` literal or a libm call promotes the expression, and the
four-lane SSE inner product of the Linux builds (`ltp_sse.h`). A corrupt frame ends the packet,
as in `speexdec`; every count from the stream is bounded.

## Provenance and licence

Ported from Speex 1.2.1 (`libspeex/nb_celp.c`, `sb_celp.c`, `filters.c`, `ltp.c`, `cb_search.c`,
`lsp.c`, `quant_lsp.c`, `bits.c`, `stereo.c`, `speex_callbacks.c`, `speex_header.c`, `modes.c`,
`modes_wb.c`, the codebook tables; BSD-3-Clause, Copyright 2002-2008 Xiph.org Foundation,
Jean-Marc Valin and others; `LICENSE.Speex`). CFA4's changes are GPL-3.0-or-later. Register entry:
`docs/licensing.md` §5.

## Fidelity

Bit-exact against the reference `speexdec` 1.2.1 on files from the reference `speexenc`
(`tests/oracle.rs`, `crates/codecs/tests/speex_oracle.rs`; the tests skip without the tools):
every narrowband quality 0–10, wideband and ultra-wideband qualities, VBR, ABR, VAD + DTX, 2–4
frames per packet, narrowband and wideband stereo; and through the media source, front to back
and after a seek.

The reference is the portable float build (SSE, no fused multiply-add), which CI builds from the
upstream tag (`scripts/ci/install-speex.sh`). Distribution packages built for x86-64-v3 (Ubuntu
26.04) let the compiler fuse multiplies and adds, and their output differs by a few LSB.
