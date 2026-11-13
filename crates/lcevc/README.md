# cfa4-lcevc

MPEG-5 Part 2 LCEVC (ISO/IEC 23094-2) in CFA4: **detection only** (step LCEVC-1 of the
owner-approved DV/DVCPRO + LCEVC plan). Layer L0, no dependencies, no `unsafe`, no panics on
damaged input.

LCEVC adds an enhancement layer on top of a base video. CFA4 does not decode it yet: decoding
(LCEVC-2 through FFmpeg's `liblcevc_dec`, LCEVC-3 an own decoder) waits on the answer to the
patent question. Until then a file that carries LCEVC plays its base layer, and `cfa4-codecs`
appends ` + LCEVC (enhancement not decoded)` to the video stream's codec label and logs a
warning.

## What is detected

From V-Nova's public carriage documentation
(<https://docs.v-nova.com/technologies/mpeg-5.lcevc/carriage/overview-of-lcevc-carriages>):

- **T.35 SEI in the base stream**: SEI payload type 4 (`user_data_registered_itu_t_t35`) in an
  H.264 SEI NAL unit (type 6) or an HEVC prefix SEI NAL unit (type 39), starting with the UK
  country code `0xB4` and the register bytes `00 50 00`. `sample_has_lcevc` scans one access
  unit, Annex B or length-prefixed; emulation prevention bytes are removed first.
- **`lvc1` tracks** in MP4 / MOV (ISO/IEC 14496-15): the MP4 reader skips them when it chooses
  the picture track and reports the enhancement.
- **TS `stream_type` 0x36** (ISO/IEC 13818-1).

The importers scan the first 8 samples of an H.264 or HEVC track (MP4 / MOV, Matroska, MPEG-TS /
PS / elementary streams).

## Sources

No LCEVC specification text or implementation code was read: only the public V-Nova carriage
pages above and the codec standards' own SEI syntax. FFmpeg's source was not read.

## Tests

- Unit tests: SEI in H.264 and HEVC, both framings, other T.35 payloads (HDR10+), several
  messages in one SEI, emulation prevention, damaged-input fuzzing.
- `cfa4-codecs` `tests/lcevc_detect.rs`: ffmpeg encodes H.264 (libx264) and HEVC (libx265), the
  test inserts an LCEVC-shaped T.35 SEI before every picture, ffmpeg remuxes to MP4, Matroska
  and MPEG-TS; the label must say LCEVC, the base must decode, and the unmodified file must not
  be flagged.
