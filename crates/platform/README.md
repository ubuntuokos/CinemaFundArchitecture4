# cfa4-platform

OS media integration for CFA4 (layer L5): hardware video decoding through the operating
system's drivers (NVDEC, VA-API), behind `cfa4_codecs::VideoDecoder`, and hardware H.264 and
H.265 (HEVC) encoding (NVENC), behind `cfa4_export::VideoEncoder`. It holds OS media FFI and
nothing else. It is the one crate of the workspace allowed to contain `unsafe`, under the rules of
[ADR 0001](../../docs/adr/0001-platform-ffi.md) and [AGENTS.md](../../AGENTS.md) §0.3.

```rust
// at startup (the desktop app, cfa4-cli, the bench)
let availability = cfa4_platform::register(); // Available("NVDEC") or Available("VA-API") with a driver
```

## What it does

- **The compute router chooses** (`routes.rs`, ADR 0004 phase 1). `register()` asks
  `cfa4-route` which hardware decoder to put in front of CFA4's decoders, from the routing table
  (the shipped `routing.toml`, the user's `~/.config/cfa4/routing.toml` on top) and the `/sys`
  device probe. The shipped order is the one before the router: NVDEC (NVIDIA GPUs on the
  proprietary driver), then VA-API, then the software decoders. Each backend's driver probe is
  asked only when the router reaches it, so VA-API is never probed after NVDEC works. The NVENC
  export factory asks the router too (`routes::nvenc_allowed`) before it opens the driver, and the
  HEVC format probe is registered only where the router routes H.264 encoding to NVENC. Hardware
  encoding stays opt-in per export (Off by default). The chosen chains, the device inventory and
  the vendor-neutrality coverage are in `perf.stats` → `route`.
- **64-bit Linux: NVIDIA NVENC H.264 and H.265 (HEVC) encoding** (`nvenc/`), 8-bit SDR 4:2:0 for Export. The driver's
  `libnvidia-encode.so.1`, API 12.1, is loaded at run time,
  so machines without NVIDIA still start. It retains a primary
  CUDA context through the runtime-loaded `libcuda.so.1`, released after the encoder is destroyed.
  No SDK, CUDA toolkit or NVIDIA binaries are bundled or required at build time. 8-bit pictures go
  in as the export's RGBA (ABGR input buffers, a ring of eight) and NVENC converts them to BT.709
  limited 4:2:0 on the GPU, with the same codes as the software encoder's own conversion; a driver
  that refuses RGB input gets that CPU conversion into NV12 instead. The conversion used to run on
  the encode thread, where it competed with the render of the next frames for the thread pool and
  held NVENC back. The encoder runs preset P5 with high-quality tuning, CABAC (CAVLC
  for Baseline), one B-frame when the profile and GPU allow it, and an IDR at every keyframe
  distance; the parameter sets go into `avcC`. Export ▸ Hardware encoding (off by default) selects it.
  It declines two-pass VBR, HDR, MXF, interlaced output, sizes outside NVENC's limits and systems
  without an NVIDIA GPU or driver, and the software encoder runs instead. A failure during an export
  ends it with an error, since a hardware stream cannot be finished in software. The same session,
  ring and input path encode H.265, see [Hardware H.265 (HEVC) encoding (NVENC)](#hardware-h265-hevc-encoding-nvenc).
- **Linux: NVDEC, H.264 (`avcC`) and HEVC (`hvcC`)** on NVIDIA's proprietary driver (`nvdec/`): H.264
  8-bit and HEVC 8- / 10-bit, 4:2:0 progressive. `libnvcuvid.so.1` and `libcuda.so.1` are loaded at
  run time; without them (or without a device) nothing is registered. NVIDIA's own parser takes the
  stream as Annex B access units, so there is no host-side DPB: `cuvid::Session` creates the decoder
  in the parser's sequence callback, decodes in its picture callback, and reads each displayed
  picture back (NV12 / P016) through the CUDA driver into the same planes as the other backends.
  The factory goes in front of VA-API's: what NVDEC declines still reaches VA-API (another GPU),
  then the software decoders. With both registered, `perf.stats` names NVDEC as the hardware
  backend, also for the streams VA-API decodes. The callback state is a `Box::into_raw` allocation
  the parser and `Session` reach through the same pointer, freed in `Drop`. After a start at a CRA picture its RASL pictures are not fed (no
  decoder outputs them, and the parser would otherwise show the CRA picture with the last one's
  timestamp). A seek recreates the parser and keeps the decoder.
- **Linux: VA-API, H.264 (`avcC`) and HEVC (`hvcC`)**: H.264 8-bit 4:2:0 progressive, Constrained
  Baseline / Main / High; HEVC Main, Main 10 and Main Still Picture, 4:2:0 8- and 10-bit (`vaapi/`), on Intel (iHD, i965), AMD and other Mesa drivers. `libva.so.2` and `libva-drm.so.2`
  are loaded at run time (`vaapi/va.rs`; no libva headers needed to build), and each decoder opens
  its own display on the first DRM render node (`/dev/dri/renderD128`…) with a working driver.
  VA-API decoding is *stateless*: the host parses the stream and keeps the decoded picture buffer,
  and the GPU decodes one picture's slice data at a time into a surface. That host side
  (`vaapi/h264.rs`, `vaapi/hevc.rs`, safe code) is the software decoders' own: their parameter-set
  and slice-header parsers, POC computation and DPBs (`cfa4_h264::dpb`, `cfa4_hevc::dpb`,
  generic over what a picture is: H.264 reference marking including MMCOs and reference lists with
  modifications; HEVC reference picture sets and lists, RASL pictures of a CRA the run starts at
  left out; output order). The two decoders therefore make
  the same decisions and output the same pictures in the same order, and the parity tests are
  bit-exact. Each picture is sent as one picture-parameter buffer, the scaling matrices, and a slice
  parameter + slice data buffer per slice (the NAL unit as stored, emulation prevention included).
  HEVC pictures are sent the same way per slice segment (scaling lists only when enabled, converted
  to raster order). Pictures the DPB outputs are read back right away (`vaGetImage` into an NV12 or
  P010 image, then `biplanar.rs`), **the one GPU to CPU copy**. Surfaces: the stream's DPB size plus two.
  - *frame_num gaps* (and every start at a non-IDR picture: an open-GOP seek) get "non-existing"
    frames like the software decoder's: copies of the latest decoded frame, mid-gray when there is
    none (`vaPutImage`). Pictures predicted from them (the leading pictures of an open GOP after a
    seek) are concealment in both decoders and can differ: the hardware also reads motion data the
    stand-ins do not have. Everything from the seek point on is bit-exact.
  - *Declined up front* (our decoder is used): VP9 and AV1 (not through VA-API yet), H.264 profiles
    other than Baseline / Main / High, 10-bit H.264, HEVC range extensions and profiles other than
    Main / Main 10 / Main Still Picture, 4:2:2 / 4:4:4, field / MBAFF coding, no libva, no render
    node or driver, a profile or size the driver does not decode. *Mid-stream errors*
    (`HybridDecoder` continues in software): FMO, SP / SI slices, data partitioning, a size or DPB
    change, a missing reference picture (HEVC: one the RPS names that was never decoded), more than
    15 HEVC reference frames, any driver error.
  - An HEVC `flush` ends the stream as in software (references are dropped and the next CRA
    leaves out its RASL pictures); the GOP cache always seeks after a flush.
- **Without NVDEC and VA-API:** `register()` does nothing and returns `Availability::Unavailable`.
- **`HybridDecoder`** (`hybrid.rs`, safe code): the hardware decoder plus the means to build our
  software decoder for the same `SampleEntry` (`cfa4_codecs::software_video_decoder`). On a
  mid-stream failure (decode error, invalidated session, changed in-band parameter sets) it replays
  the samples since the last restart point (IDR / IRAP; for an HEVC CRA the one before, so its RASL
  pictures decode) through the software decoder, drops pictures already returned, keeps the ones
  the hardware had decoded but not returned, and stays in software for that instance. The replay
  log is bounded (600 samples / 256 MB); beyond it one error is returned and the next seek restarts
  in software. Streams our decoders cannot decode (HEVC 4:2:2) have no fallback: the error stands.

## Hardware H.265 (HEVC) encoding (NVENC)

One codec enum (`nvenc::Codec`, chosen by `Profile::HevcMain`) parameterises the NVENC session: the codec and profile GUIDs, the capability
query, the `hevcConfig` member of the codec-configuration union, how NAL units are told apart
(`(b0 >> 1) & 0x3f`) and the level numbering (`level × 30`, so 4.1 is 123). The level is the lowest
whose **Main tier** limits hold the picture size, sample rate and peak bitrate (H.265 Table A.8,
`nvenc::hevc::main_tier_level`): left to choose, NVENC answered 1080p30 at an 18 Mbit/s peak with
level 4 High tier, which many hardware decoders refuse; it is now level 4.1 Main tier. The H.264 path is unchanged: the same export is the same file, byte for byte.

- **Choosing the format is the opt-in:** H.265 has no built-in encoder. Export ▸ Format ▸ H.265 (HEVC) or
  `"format": "hevc"`. `register()` hands `nvenc::hevc_available` to
  `cfa4_export::register_format_probe`: one small HEVC session (a 640×360 encoder, the answer
  kept; about half a second in a fresh process, nearly all of it opening the CUDA context and
  the NVENC session, which every NVENC export pays too). `register()` also asks it on a thread of its
  own (`nvenc::warm_hevc_probe`), so the first draw of the format list does not wait for it; a
  caller that asks while it runs waits for the same answer. The
  Hardware encoding toggle governs H.264 only: with H.265 selected NVENC is used whether it is Auto
  or Off, because nothing else can encode it.
- **What it writes:** HEVC Main, 8-bit 4:2:0, SDR BT.709 limited range (Main 10 HDR: see below) (the VUI says so, with the
  frame rate as `time_scale / num_units_in_tick`), progressive, preset P5 with high-quality tuning,
  constant or one-pass variable bitrate, an IDR at every keyframe distance, one B-frame when the GPU
  reports support and the GOP is longer than two pictures (shorter ones are written without), in
  MP4 (`hvc1`) or QuickTime. Pictures are converted exactly like the H.264 path's. The coded size is
  a whole number of 32-pixel coding tree blocks and the SPS carries a conformance window (1080 is
  coded as 1088 and cropped back); the `hvcC` is only accepted when the cropped size is the export's.
- **Samples are length-prefixed (4 bytes) with the parameter sets only in the `hvcC`:** VPS, SPS,
  PPS, access unit delimiters and end-of-sequence / bitstream markers are dropped; slices and SEI
  stay. `nvEncGetSequenceParams` returns all three parameter sets.
- **The `hvcC` is built from the SPS the encoder wrote** (`nvenc/hevc.rs`, parsed with
  `cfa4_hevc`): profile space, tier, profile, compatibility flags and level from the SPS's
  profile / tier / level; the 48 general constraint flags and `sps_temporal_id_nesting_flag` read from
  the SPS bits that carry them; chroma format, bit depths and temporal layers from the SPS; the
  arrays complete (`array_completeness` 1). Nothing is a constant of ours except what the format
  leaves unspecified (`avg_frame_rate`, `constant_frame_rate`, `min_spatial_segmentation_idc` and
  `parallelism_type` are 0). It is built, and validated as Main 8-bit 4:2:0 of the right size, when
  the encoder is created, so a stream we cannot describe is a declined export and never a default
  sample entry.
- **Timestamps are the H.264 path's:** dts is the decode index minus the B-frame delay, composition
  offsets are `pts - dts` in frame durations, and the media starts `delay` frames early (edit list).
- **There is no fallback encoder.** A request NVENC cannot take is counted in
  `export.hardware.declined` and ends the export with "H.265 export with NVENC: …" and the reason:
  10-bit on a GPU without it (HDR), interlaced output, two-pass (`ExportSettings::validate` refuses it first), non-square pixels,
  MXF, sizes above 65535, odd sizes, sizes outside the GPU's limits (129×33 to 8192×8192 on the RTX
  5060). On a machine without an HEVC encoder the format is not available and an export fails with
  "encoder not available yet". Frames, sessions and declines are the same `export.hardware` counters
  as for H.264.
- **Main 10 HDR (PQ / HLG).** An HDR sequence (Rec. 2100 PQ or HLG working space) exports as HEVC
  Main 10 when `settings.sdr` is off and this GPU has a 10-bit HEVC encoder. `register()` hands
  `nvenc::hevc_hdr_available` (a Main 10 session, cached, asked on the same background thread as the
  8-bit probe and short-circuiting when HEVC itself is missing) to `cfa4_export::register_hdr_probe`,
  and the export job makes the export HDR for `Format::Hevc` only when
  `cfa4_export::hdr_available(Format::Hevc)` says so: on GPUs
  without 10-bit an HDR sequence is tone-mapped to 8-bit SDR HEVC exactly as before, and `settings.sdr`
  still forces that. The pixels are real: `EncoderFrame::hdr` (encoded BT.2020 R'G'B', 3 floats per
  pixel) goes through `cfa4_export::rgbf_to_yuv420_10` (BT.2020 non-constant-luminance matrix,
  Y = 64 + 876·Y', C = 512 + 896·C, 2x2 chroma average, NaN counts as 0, values clamp to 0..1 and codes
  to 4..1019, wrong-length input is an error) and into a **P010** input buffer (`code << 6`,
  little-endian, chroma interleaved after the luma rows, pitch in bytes; a driver pitch under 2 bytes
  per pixel is refused). One buffer format (`NV_ENC_BUFFER_FORMAT_YUV420_10BIT`) is used for the
  initialisation, the input buffers and every picture; `pixelBitDepthMinus8` is 2, the profile GUID
  is Main 10, `NV_ENC_CAPS_SUPPORT_10BIT_ENCODE` is queried first and a GPU without it declines
  with "no 10-bit (HEVC Main 10) support". The VUI says BT.2020 primaries, transfer 16 (PQ) or 18
  (HLG), matrix 9, limited range. The `hvcC` is checked against what was asked (`hevc_config(…, 10)`:
  profile 2, 10-bit 4:2:0). NVENC chose the High tier for level 4 at 12 Mb/s; the record just says so.
  Pictures of the wrong depth for the encoder (an `hdr` picture to a Main encoder, an RGBA one to
  Main 10, or a float slice of the wrong length) are errors, never panics.
- **HDR10 static metadata is SEI, not an API.** NVENC 12.1 has no mastering-display field, so PQ
  streams get the mastering display colour volume (payload 137) and content light level (144)
  messages through `NV_ENC_PIC_PARAMS_HEVC::seiPayloadArray` on every IDR picture, with the values
  of `ColorSignal::static_metadata` (BT.2020/D65, 1000 / 0.0001 cd/m², MaxCLL/MaxFALL 0), the same
  that the software H.264 encoder's SEI and the `mdcv` / `clli` boxes carry. NVENC writes them as
  two prefix SEI NAL units (type 39) in front of the IDR slice, adding the emulation-prevention
  byte its `00 00 00 01` needs; nothing is attached to other pictures; HLG has no static metadata and
  gets none. The payload bytes and the descriptor array are heap blocks owned by the `Session` for
  its whole life (it is moved by value, and the driver may queue a picture), and IDR pictures are
  known from a submission counter (`n % gop == 0`). The sample entry carries `colr` (nclx BT.2020,
  PQ / HLG, limited), and for PQ `mdcv` and `clli`; 8-bit SDR HEVC is unchanged and has no `colr`
  (the VUI carries its description, as in the software H.264 path).

## Guarantees

- **Never undecodable:** the factory declines (returns `None`, so the software decoder is used)
  when Settings ▸ Playback ▸ Hardware decoding is Off, for formats it does not take (field-coded
  H.264, bit depths other than 8 / 10, 4:4:4 or monochrome, luma / chroma depth mismatch, larger
  than 8192×8192; everything but 8-bit 4:2:0 progressive H.264 and 4:2:0 HEVC Main / Main 10) and
  when the driver cannot create an NVDEC session or a VA-API configuration and context.
- **Interchangeable:** colour, pixel aspect, pts, presentation order, `is_random_access` and
  `is_disposable` come from the software decoders' own helpers (`cfa4_codecs::hw`,
  `video::vui_color`, `sar_par`).
- **Never crash:** no `unwrap` / `expect` / `panic!` outside tests; the output callback (and
  libva's error-message callback) runs under `catch_unwind`; every `unsafe` block has a `// SAFETY:` comment; the public API is safe.
- **Counted:** `perf.stats` `decode.hardware` (frames, software frames, sessions, declined,
  fallbacks; `cfa4_codecs::hw::hw_stats`) and `backend` (the registered backend's name,
  `cfa4_codecs::hw::hw_backend`). `export.hardware` counts the NVENC encoder's frames, sessions and declines.
## Tests

| test | what |
|---|---|
| `tests/fallback.rs` (every OS) | `HybridDecoder` with a stand-in hardware decoder failing after N samples (every sync sample ± a few, first / last sample, after a seek): output identical to the software decoder; in-band parameter sets identical to the sample entry's stay in hardware, different ones switch to software |
| `tests/setting.rs` | Hardware decoding Off gives the software decoder through `make_video_decoder` and the media stack (no hardware frames); Auto gives the hardware decoder where available |
| `tests/nvenc.rs` (Linux, NVIDIA) | H.264 from NVENC (1280×720, 6 Mbps, 72 frames) decodes with our decoder at worst 46.9 dB luma PSNR; IDR at 0, 24 and 48; dts / pts right |
| `tests/nvenc_export.rs` (Linux; fallback also tested without NVIDIA) | Export with hardware encoding against the software encoder through the export pipeline: the two decoded files at worst 54.8 dB luma PSNR; ffmpeg decodes the file without errors; declined cases go to the software encoder; the counters |
| `src/nvenc/abi_tests.rs` (Linux) | FFI structs' sizes, alignments, field offsets, constants, GUIDs and the bit-field masks of the `flags` words (H.264, HEVC, and the HEVC picture parameters with the SEI payload array) against a C compiler's view of NVIDIA's `nvEncodeAPI.h` (12.1) |
| `src/nvenc/{mod,hevc,export}.rs` unit tests | HEVC NAL types, stripping and parameter-set splitting (including `IDR_N_LP`, whose header byte reads as a PPS in H.264, and one-byte or truncated NAL units); the `hvcC` built from real VPS / SPS / PPS (Main and Main 10: accepted as what was asked for, refused as the other, other depths refused), every truncation and bit flip of the SPS, wrong profile, size and NAL types; HEVC levels; P010 layout (shift, little-endian, interleaving, pitch in bytes, narrow pitches and short buffers); the 10-bit capability decline; the SEI messages |
| `crates/export/src/hdr_tests.rs` | `rgbf_to_yuv420_10`: exact codes for black, white, grey and the BT.2020 primaries and secondaries, NaN / infinity / out-of-range, odd sizes, wrong lengths and overflowing sizes; the HDR probe registry; which exports are HDR (HEVC SDR without a probe, HDR with one, `settings.sdr` forcing SDR); the SEI of the software H.264 encoder equals `ColorSignal::static_metadata` |
| `tests/nvdec.rs` (Linux, NVIDIA driver) | H.264 High, HEVC Main (open GOP: CRA + RASL) and Main 10 at 640x360, H.264 and HEVC Main 10 at 1080p: bit-exact with the software decoders over the whole stream, after `reset` + reseek to every sync sample and again after the resets; a forced mid-stream failure continues in software with identical output; interlaced and 10-bit H.264 and 4:2:2 HEVC are declined, as is everything while Hardware decoding is Off; damaged samples (bit flips, truncation, corrupt lengths) never crash or hang |
| `src/nvdec/abi_tests.rs` | FFI structs' sizes, alignments, field offsets and constants against a C compiler's view of NVIDIA's `cuviddec.h` / `nvcuvid.h` (nv-codec-headers) |
| `tests/vaapi.rs` (Linux, VA-API) | HEVC Main (open GOP: CRA + RASL), Main 10, scaling lists, 3 slices with WPP (10-bit), weighted prediction with 4 references, transform skip + AMP + B-pyramid (10-bit, open GOP), Main / Main 10 at 1080p and 2160p (compared picture by picture, little memory); H.264 High (B-pyramid), Constrained Baseline with 3 slices per picture, Main with explicit weighted prediction (P and B) and 4 references, temporal direct, custom scaling matrices (JVT) with the 8x8 transform and 2 slices, an open-GOP stream, and 1080p / 2160p, 640×360 (coded 368: cropping): every picture **bit-exact** with our software decoder, same pts order, count, colour and aspect, also after `reset` + reseek to every later sync sample (after an open-GOP I picture, from the seek point on), a mid-stream `flush` with decoding carrying on, and a full pass after resets; forced mid-stream failures (`VaDecoder::fail_after`) at five points continue with the software decoder's exact output; interlaced and 10-bit H.264, 4:2:2 HEVC and the Off setting are declined; an HEVC mid-GOP flush is not checked (it ends the stream, as in software); seeded mutation of samples (through the hybrid and straight into the hardware decoder) and of `avcC` records never panics or hangs; 40 decoders created and dropped; concurrent decoders; a decoder moving between threads |
| `src/vaapi/tests.rs` (every OS) | The H.264 and HEVC front ends with a recording stand-in for the hardware, on x264 streams (B-pyramid, weighted prediction, 4 slices, open GOP with temporal direct) and x265 streams (open GOP with RASL, B-pyramid, Main 10 with 2 slices and weighted prediction): output order and pts are the software decoder's, also after seeks and a mid-stream flush; every buffer is consistent (current surface free, references decoded and listed once, reference lists of the active length, slice data offsets inside the slice); damaged samples are errors |
| `src/vaapi/abi_tests.rs` | FFI structs' sizes, alignments, field offsets, constants and bit-field positions against a C compiler's view of libva's `va.h` and `va_dec_hevc.h` (VA-API 1.23) |

Fixtures are made with ffmpeg into `target/fixtures/platform/` (generator only, never linked);
tests skip without ffmpeg or without a hardware decoder.

## Performance

Hardware decoding, Linux (Intel Iris Xe, iHD driver; `cargo xtask bench --hw off|auto`): CPU per
decoded frame H.264 1080p 104 → 2.3 ms, 2160p 417 → 11.4 ms; HEVC 1080p 46 → 2.3 ms, 2160p
203 → 10.1 ms, Main 10 2160p 212 → 18.4 ms.
Details in [docs/performance.md](../../docs/performance.md).

## Not yet

Zero-copy upload of decoded pictures into wgpu textures; VP9 and AV1 through VA-API, and VA-API
encoding; field-coded H.264; 4:4:4 and 4:2:2 encoding with NVENC, HDR H.264 with NVENC (HDR H.264
stays with the software encoder), HDR10 MaxCLL / MaxFALL measured from the pictures (they are
written as 0, unknown), AV1 with NVENC, and hardware encoders from other vendors.

### Linux NVENC sources and limits

The device binding uses NVIDIA's [CUDA Driver API 12.1 primary-context reference](https://docs.nvidia.com/cuda/archive/12.1.0/cuda-driver-api/group__CUDA__PRIMARY__CTX.html)
and the existing [Video Codec SDK 12.1 encode API](https://docs.nvidia.com/video-technologies/video-codec-sdk/12.1/nvenc-video-encoder-api-prog-guide/index.html).
It uses the first CUDA device; formats and settings NVENC does not take use the existing encoder
selection.
Linux registers the export factory even without a driver (it asks the compute router before it opens one); `register()`'s return value and
`registered()` still describe hardware **decoding**, which this backend does not provide.
Absent / old drivers decline once per export, with an informational log and
`perf.stats` → `export.hardware.declined`; hardware encoding Off never opens the driver.
