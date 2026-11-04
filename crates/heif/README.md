# cfa4-heif

Clean-room reader for HEIF / HEIC still-image files (ISO/IEC 23008-12 on the ISO base media file
format, ISO/IEC 14496-12). It finds the primary image and returns what a decoder needs; it decodes
no pixels itself. `cfa4-media` (`still/heic.rs`) decodes the HEVC bitstreams with `cfa4-hevc` and
assembles the picture.

## What it reads

- `ftyp` brands (`heic`, `heix`, `hevc`, `hevx`, `heim`, `heis`, `mif1`, `msf1`), `meta` with a
  `pict` handler, `pitm`, `iinf` / `infe` (versions 2 and 3), `iloc` (versions 0–2, file and
  `idat` offsets, several extents), `iref`, `iprp` / `ipco` / `ipma`.
- Items:
  - `hvc1` gives the `hvcC` record and the length-prefixed NAL units;
  - `grid` gives its tiles in raster order and the output size;
  - `iden` gives its source with its own transforms added;
  - any other type is reported by name (for example `av01`, `jpeg`, `unci`).
- Properties:
  - `ispe` and `colr` (nclx, or an ICC profile);
  - the transforms `clap`, `irot` and `imir`, in the order the file lists them;
  - `auxC`: an auxiliary image with the alpha URN, which refers to its master with `auxl`, becomes
    the image's alpha.
- `imir` mode 0 swaps top and bottom and mode 1 swaps left and right, as libheif reads it. A clean
  aperture's edges and size are rounded to whole pixels, halves up. Both are checked against
  libheif in `crates/media/tests/heic_native_oracle.rs`.

## Hostile input

Every size is checked against the bytes present. The caps are 65 536 items, properties and grid
tiles, and a derivation depth of 8. Nothing panics: the crate denies the panicking calls, and the
tests feed it truncated and scrambled files.

## Writer

The `writer` feature builds files with chosen items, properties and references. The tests use it,
and so do the libheif oracle tests in `cfa4-media`.
