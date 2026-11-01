# cfa4-mpeg2v

Clean-room MPEG-2 video and MPEG-1 video decoder, and an MPEG-2 video encoder, in pure Rust. Layer L0: depends only on
`thiserror` and (optional, feature `threads`, default on) `rayon`. Builds for
`wasm32-unknown-unknown`; no `unsafe`. No GPL/LGPL code (FFmpeg, libmpeg2, libavcodec) was
consulted; FFmpeg is used only as an external fixture generator and test oracle.

## Specifications

| Document | Edition | Used for |
|---|---|---|
| ITU-T H.262 \| ISO/IEC 13818-2 | 02/2000 (second edition, with Amd. 1 and Cor. 1/2 as incorporated in the 2000 text) | sequence / GOP / picture headers and extensions, slices, macroblocks, VLC tables (Annex B), inverse quantisation and mismatch control (§7.4), IDCT (Annex A), motion vectors and prediction (§7.6) |
| ISO/IEC 11172-2 | 1993 | MPEG-1 video: slices spanning rows, MPEG-1 escape codes, oddification, full-pel vectors, D pictures |
| MPEG-2 Test Model 5 (ISO/IEC JTC1/SC29/WG11/N0400) | 1993 | encoder: picture bit allocation from I/P/B complexities (K_P 1, K_B 1.4), adaptive quantisation by spatial activity |
| IEEE Std 1180 | 1990 | IDCT accuracy test procedure (run as a unit test) |
| ISO/IEC TR 13818-5 (MPEG Software Simulation Group decoder behaviour) | 1997 | two decoder behaviours the normative text leaves open (see "Interpretations") |

## API

```rust
let mut dec = cfa4_mpeg2v::Decoder::new();
for (i, au) in cfa4_mpeg2v::access_units(&es).into_iter().enumerate() {
    for pic in dec.decode(&es[au], i as i64)? {             // display order
        // pic.y / pic.cb / pic.cr: tight 8-bit planes; pic.chroma: 4:2:0 / 4:2:2
        // pic.field_order(), pic.picture_type, pic.pts, pic.info (SequenceInfo)
    }
}
let rest = dec.flush();
let info = cfa4_mpeg2v::probe(&es);                    // sequence header + extensions
let au = cfa4_mpeg2v::scan_access_unit(&es[..]);       // picture types without decoding
```

`decode` takes any chunk holding whole pictures (an access unit from a TS/PS/MXF/MOV sample).
Pictures come out in display order (anchors are held until the next anchor) and carry the `pts`
of the chunk their first field came in. `reset` forgets pictures but keeps the sequence
parameters, so a random-access point without its own sequence header still decodes.

## Decoder

- Main Profile at Low/Main/High-1440/High Level, 4:2:2 Profile (IMX / D-10, XDCAM HD422),
  Simple Profile, MPEG-1 (including D pictures). 4:4:4 is refused (no profile in use for it).
- Frame pictures with frame and field motion compensation, dual prime, frame and field DCT;
  field pictures with field, 16x8 and dual-prime motion compensation, the second field of a
  P frame predicted from the first; open and closed GOPs; B pictures without a forward
  reference after a broken link are dropped (closed GOP: decoded from the backward reference).
- Both intra VLC tables, zigzag and alternate scan, linear and non-linear quantiser scale,
  quantiser matrices from the sequence header and the quant matrix extension (chroma matrices
  for 4:2:2), intra DC precision 8-11 bits, concealment motion vectors, saturation and
  mismatch control.
- IDCT: the separable 2-D IDCT in `f32` with even/odd symmetry, the DC term added exactly,
  row skipping and a DC-only path; rounded and saturated to [-256, 255]. `ieee_1180_accuracy`
  runs the IEEE 1180 procedure (10 000 blocks × 3 ranges × 2 signs): peak error ≤ 1 and all mean
  / mean-square limits met.
- Motion compensation clamps reference coordinates to the picture (only corrupt streams point
  outside it); a missing reference predicts from mid-gray.
- Slices of different macroblock rows decode in parallel on rayon (each row into its own chunk
  of the frame); MPEG-1 slices, which may span rows, decode in order.
- Corrupt slices are concealed (the rest of the picture still decodes) and counted
  (`Decoder::errors`, `last_error`); nothing panics on truncated or mutated input.

## Encoder

```rust
use cfa4_mpeg2v::encode::{Config, Encoder, Input, Rate};
let mut cfg = Config::new(1920, 1080, 25, 1);           // 4:2:0, quantiser 6, GOP 15, I/P
cfg.rate = Rate::Bitrate { bits_per_second: 25_000_000 };
cfg.b_frames = 2;                                        // IBBP; cfg.closed_gop (default true)
let mut enc = Encoder::new(cfg)?;
let packets = enc.encode(&Input { y, cb, cr, y_stride, c_stride })?;  // decode order, 0 or more
let rest = enc.flush()?;                                 // the pictures still held back
let end = enc.finish();                                  // sequence_end_code
```

- Main Profile (4:2:0) and 4:2:2 Profile at the lowest level that holds the size, picture rate
  and bitrate (Main, High-1440, High; 4:2:2 Main, High); beyond High Level the stream still
  decodes and is signalled as High Level. The eight MPEG-2 frame rates; any size up to
  16383×16383 (odd sizes are edge-padded to whole macroblocks and cropped by the decoder).
- Frame pictures: I and P pictures (`low_delay` set when there are no B
  pictures), and 0-7 B pictures between anchors (`b_frames`). An anchor every `b_frames + 1`
  pictures within each GOP; the B pictures before an I picture belong to its GOP. Packets come
  in decode order with the display number (`index`) and a decode time (`dts`, one frame
  earlier with B pictures, so DTS ≤ PTS); `flush` codes the held-back pictures at the end (the
  last becomes a P picture). Closed GOPs (`closed_gop`, the default) predict those leading B
  pictures from the I picture alone and set `closed_gop`, so every GOP decodes on its own;
  open GOPs predict them from both sides. The sequence header (and a sequence display
  extension with the colour description) comes before every I picture. One slice per
  macroblock row.
- Progressive (the default) or interlaced (`interlaced: Some(FieldOrder)`): interlaced
  sequences set `top_field_first` as asked, use the alternate scan and choose per macroblock
  between frame and field prediction (each field of the macroblock from the better reference
  field, with its `motion_vertical_field_select`) and between frame and field DCT (whichever
  transforms the luma error into the smaller coefficient magnitudes). Skipped and "MC not
  coded" macroblocks stay frame-predicted. On combed moving content this saves about 10 % at
  the same quantiser.
- Motion search on luma at whole samples (left, temporal and zero predictors, then large and
  small diamonds), then half-sample refinement, ±32 samples (`f_code` 3); vectors whose
  luma or chroma prediction would leave the picture are never chosen. B pictures search both
  anchors (temporal predictors: the last P picture's vectors scaled to the distance) and take
  the cheapest of forward, backward and their average (SAD plus vector bits). Intra / inter
  decision by SAD against the deviation from the mean; skipped macroblocks (in B pictures:
  the same directions and vectors as the macroblock before, §7.6.6.4) and "MC not coded"
  when no coefficient survives quantisation.
- Forward DCT as the transpose of the decoder's IDCT basis; default matrices; non-linear
  quantiser scale; intra levels rounded with offset 3/8, inter levels `floor(|F| / qs)` (the
  reconstruction lands mid-step). The intra VLC table (B.14 or B.15) of each picture is the one
  that would have coded the previous picture's intra blocks in fewer bits.
- Rate control: constant quantiser, or an average bitrate: TM5 picture targets from the
  I, P and B complexities (bits × quantiser), the picture quantiser from its type's complexity
  (the first picture of each type is coded once as a trial to measure it), a gentle in-picture
  correction against an expected bit profile (each macroblock's coefficient magnitude), and
  optional TM5 adaptive quantisation (`adaptive_quantization`; looks better on flat areas,
  measures about 2.5 dB lower PSNR).
- Buffer model (Annex C): `Rate::Bitrate` is an average with no buffer model (`vbv_delay`
  0xFFFF). `Rate::ConstrainedVbr` (average, maximum, buffer) and `Rate::Cbr` (bitrate,
  buffer) keep every picture within the VBV: the picture target stays below the buffer's
  occupancy (a tenth kept in reserve), and a picture that still comes out too large is coded
  again with a coarser quantiser. CBR writes real `vbv_delay` values, starts with a full
  buffer and appends zero bytes (stuffing before the next start code) where small pictures
  would let the buffer overflow; the buffer it fills is capped so the delay fits 16 bits.
  The sequence header signals the maximum (VBR) or constant rate and the buffer (0: the
  level's). A level is chosen whose buffer holds the requested one.
- The reconstruction of every picture is our own decoder's output for the coded bytes, so
  encoder and decoder never drift apart; a picture the decoder rejects is an error.
- Macroblock rows are analysed (motion search, prediction, DCT) in parallel on rayon; coding
  is sequential (rate control).

Not yet, each tracked: field pictures, dual-prime and 16x8 prediction (#274), P-STD / T-STD
buffer models in the multiplexer (#277), DVD-Video
(#244) and Blu-ray (#245) authoring, speed and RD quantisation, scene cuts, custom matrices
(#246).

### Interpretations

Two behaviours follow the MPEG Software Simulation Group reference decoder (and FFmpeg), which
the synthetic-stream tests confirmed against FFmpeg:

- Skipped macroblocks in field pictures predict from the field of the same parity, in B
  pictures as well as P pictures.
- Motion vector reconstruction (§7.6.3.1) brings the vector back into range only on the side the
  delta moves it, and keeps the prediction for a zero delta. This differs from the literal
  formula only when the prediction is out of range: a field vector's doubled vertical
  component predicting a following frame vector.

## Accuracy (vs FFmpeg as an external oracle)

MPEG-2's IDCT is not bit-exact by definition (any IEEE 1180-accurate IDCT conforms, and the
encoder's reconstruction drifts from every decoder's). FFmpeg uses an integer IDCT, so frames
are compared within a tolerance: **per frame max |Δ| ≤ 4 and PSNR ≥ 58 dB, at most 3 % of
samples differing**. Frame count, display order, picture types, interlacing and field order are
exact (ffprobe).

`tests/oracle.rs` (FFmpeg-encoded elementary streams):

| Fixture | Content | Max Δ | Differing | PSNR |
|---|---|---|---|---|
| m2v_cif_ibbp | 352×288 progressive IBBP, Main@Main | 2 | 0.60 % | 70.3 dB |
| m2v_576i_tff | 720×576 interlaced TFF, field MC/DCT, alternate scan | 2 | 0.33 % | 73.0 dB |
| m2v_480i_bff_vlc1 | 720×480 BFF, intra VLC table 1, non-linear q, 10-bit DC | 2 | 0.38 % | 72.3 dB |
| m2v_imx_422_intra | 720×608 4:2:2 intra (IMX / D-10 style) | 1 | 0.23 % | 74.6 dB |
| m2v_hd422_longgop | 1920×1080 4:2:2 interlaced long GOP (4:2:2@High) | 2 | 0.20 % | 75.1 dB |
| m2v_1080i | 1920×1080 4:2:0 interlaced, 25 Mb/s (Main@High) | 3 | 0.25 % | 74.2 dB |
| m2v_escapes_qmatrix | q = 1 (escape codes), custom intra/inter matrices | 2 | 1.80 % | 65.6 dB |
| m2v_ipp | no B pictures | 2 | 1.67 % | 65.9 dB |
| m1v_sif | MPEG-1 352×240 IBBP | 2 | 0.59 % | 70.4 dB |
| m1v_q1 | MPEG-1 q = 1 (8/16-bit escapes) | 2 | 1.93 % | 65.2 dB |

FFmpeg's encoder never writes field pictures, 16x8 or dual-prime prediction, so
`src/synth_tests.rs` writes random but valid streams with them (random macroblock types,
vectors, field selects, quantiser scales and coefficients; skipped macroblocks; concealment
vectors; 4:2:0 and 4:2:2; TFF and BFF) and FFmpeg decodes the same streams with `-xerror`:

| Stream | Max Δ | PSNR |
|---|---|---|
| field pictures I/P/B + 16x8, TFF 4:2:0 / BFF 4:2:2 | 2 | 65.7–66.0 dB |
| field pictures + dual prime, TFF 4:2:0 / BFF 4:2:2 | 2 | 66.5–67.5 dB |
| interlaced frame pictures + field MC + dual prime, TFF / BFF | 2 | 64.8–65.7 dB |
| frame pictures with B, 4:2:2, alternate scan, non-linear q | 1 | 66.8 dB |
| seed sweep: 8 seeds × 4 configurations (160 streams checked during development) | ≤ 2 | ≥ 64.7 dB |

Residuals are kept realistic (dequantised coefficients within ±300): with extreme residuals
FFmpeg's integer IDCT saturates differently from the exact one.

## Performance

`cargo test --release -p cfa4-mpeg2v --test perf -- --ignored --nocapture` decodes the
1080i fixture (1920×1080 4:2:0, 25 Mb/s, IBBP, field MC/DCT; FFmpeg-encoded) on an M4 Pro
(14 cores) that was heavily loaded by other jobs (load average ≈ 190), best of five runs:
**271 fps on one thread, 274 fps with slice rows on rayon** (runs under heavier momentary load
fell to 77–150 fps). These are lower bounds.

Encoding (`perf_1080p_encode_fps_and_psnr`, FFmpeg's `testsrc2` at 1920×1080, 50 frames,
GOP 15 I/P, 15 Mb/s; same machine class, unloaded container): **18-21 fps**, 15.10 Mb/s at
**48.5 dB** luma PSNR; FFmpeg's `mpeg2video` with the same GOP, no B frames: 15.16 Mb/s at
48.8 dB.

## Tests

- `src/vlc.rs`: every table builds without prefix clashes; Kraft sums equal 1 minus exactly the
  codes the standard reserves; every coded_block_pattern and motion_code value appears once;
  motion codes are the address-increment codes plus a sign; 111 run/level pairs per DCT table.
- `src/idct.rs`: IEEE 1180; DC-only path equals the full transform for every DC value.
- `src/headers.rs`, `src/mc.rs`, `src/bits.rs`: scans, start codes, frame rates, half-sample
  rounding, field addressing, edge clamping.
- `src/tests.rs`: a hand-built intra picture decodes to its DC; truncation at every byte and
  3000 random mutations never panic.
- `src/synth_tests.rs`, `tests/oracle.rs`: see above (single-threaded decode identical to the
  threaded one).
- `src/encode/`: bit writer and code tables (every value has a code, built from the decoder's
  tables), FDCT against the decoder's IDCT, rate control; round trips through our decoder
  (PSNR per picture, picture types, sequence information), 4:2:2 with 11-bit DC and colour
  description, odd sizes, still and moving content, full-swing noise at the finest quantiser
  (escapes), bitrates within 15 % of the target (within 1 % in practice), level selection,
  rejected configurations, and FFmpeg decoding our 4:2:0 / 4:2:2 / rate-controlled streams
  with `-xerror` to within 2 of our decoder.
