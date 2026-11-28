# cfa4-media

Media sources, still-image and image-sequence import, generated media, audio streams, and decoded
frame caching for CFA4.

## Image-sequence filenames

`sequence::Numbered` identifies the last run of 1–18 ASCII frame-number digits in a filename's
stem when the extension is a recognized still-image format. `/` and `\` separate directory
components. A prefix may contain other UTF-8 characters, including accented letters, CJK and emoji:
`é0001.png`, `图0001.png` and `🎞️0001.png`. Parsing preserves the prefix, number padding and suffix
when generating sibling-frame paths. Sibling recognition compares the prefix's UTF-8 bytes exactly.
Stems without ASCII digits or with a final frame-number run longer than 18 characters are not numbered
image-sequence filenames.

## HDR gain map JPEGs and MPF (`still::gainmap`)

A JPEG with a gain map (Google Ultra HDR, Adobe Gain Map, ISO 21496-1) opens as its HDR rendition: the
gain map is upsampled bilinearly, applied to the linear base at the full alternate headroom, and the
picture is kept as scene-linear Rec. 709 floats. Sources:

- CIPA DC-007-2009 (Multi-Picture Format): the MPF APP2 segment and the MP Index IFD.
- Adobe Gain Map Specification 1.0 and Google Ultra HDR Image Format v1.1: the XMP `hdrgm`
  properties, the GContainer directory and the application formula.
- ISO 21496-1:2025 metadata block: ported from libultrahdr (commit
  `c6fd15cdb2d05d3aaed84b817e4c4a274a4ce600`, MIT or Apache-2.0, `LICENSE-libultrahdr`; reused code
  register in `docs/licensing.md`), because the standard's text was not available (owner decision
  2026-11-28, F2g E3).

When a file holds both, the ISO block wins. A file whose gain map cannot be read opens as its SDR base
with a warning. An MPF file without a gain map (a stereo `.mpo`) opens its first picture. Tests:
`src/still/gainmap/tests.rs` (synthetic files) and `tests/gainmap_oracle.rs` (FFmpeg's `gainmap`
filter and `jpeg_mpf` muxer as the oracle).
