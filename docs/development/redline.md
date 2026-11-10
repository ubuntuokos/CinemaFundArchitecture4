# RED REDline: R3D as an external process

**Status:** developer tooling (2026-11-10). It prepares the camera-RAW work package of the
professional codec plan: under the licence rules (`AGENTS.md`, [docs/licensing.md](../licensing.md) §4)
CFA4 handles R3D by calling RED's own command-line tool, REDline, as an external process.
CFA4 does not link the R3D SDK and ships no closed R3D adapter.

## 1. What REDline is and what it gives CFA4

REDline is RED's free command-line renderer (it is part of REDCINE-X PRO). On Linux x86_64 it
reads R3D (including R3D NE) and writes DPX, TIFF, OpenEXR, JPEG, ProRes, DNxHR and R3D trims. It
also prints clip and per-frame metadata (`--printMeta`). Build 65.0.22 (checked 2026-11-10) contains
**R3D SDK 9.1.1**, decodes on the CPU (`--gpuPlatform 0`) or through OpenCL/CUDA, and needs only
glibc, X11 and zlib from the system. It brings its own Qt 5.3, libmpg123 and RED libraries.

For CFA4 it does two jobs:

1. **The R3D import path.** A user-installed REDline converts R3D into a format CFA4 already
   decodes (OpenEXR / DPX sequences, ProRes, DNxHR), with metadata read from `--printMeta`. Since
   it runs in its own process, a REDline crash is an error and never a CFA4 crash (`AGENTS.md` §0).
   Real-time R3D playback is out of scope on this path.
2. **The reference (oracle) for R3D tests**, in the role ffmpeg plays for other codecs
   ([docs/testing.md](../testing.md) §2): fixed-setting EXR frames and metadata to check CFA4's
   R3D path against, and to catch regressions.

REDline and the separate R3D SDK 9.2.1 are **not** bit-identical versions (9.1.1 vs 9.2.1; 9.2.x
added NIKKOR lens correction and changed Broadcast-clip processing). Comparisons between them need
a tolerance. CFA4 itself never uses the SDK, so the REDline build in use is the reference, and every
reference set records it (`manifest.txt`).

## 2. Licence and distribution rules

- REDline is proprietary RED software. **Never commit** the installer, the binary, its libraries,
  R3D clips or frames rendered from them, in this or any public repository. CFA4 does not bundle or
  redistribute REDline: the user downloads it from RED and installs it.
- CFA4 only starts it as a separate program and reads its output files, as the licence rules allow
  for vendor programs (`AGENTS.md`, CFA4 licence section: "call the program as an external process").
- Vendor neutrality: the reference renders use the CPU. REDline's CUDA path is NVIDIA-only and may
  be offered only next to OpenCL or the CPU (`AGENTS.md`, GPU vendor neutrality).

## 3. Setting it up without root

RED's installer (`REDline_Build_<version>_Installer.sh`) is a Makeself archive whose script copies
the files into `/usr/local` as root and runs `ldconfig`. For development, unpack it into a private
directory instead, without running the vendor script:

```sh
scripts/dev/redline-unpack.sh ~/Downloads/REDline_Build_65.0.22_Installer.sh ~/.local/opt/redline
export CFA4_REDLINE=~/.local/opt/redline/redline
"$CFA4_REDLINE" --help | head
```

The script checks the archive's embedded MD5, refuses absolute paths, `..` and links that point
outside the archive, extracts it into an empty temporary directory and puts `REDline` and its
libraries into `<dest>/libexec/` (REDline looks for `REDR3D-x64.so` next to its own binary). It
creates the Qt 5 soname links the installer leaves to `ldconfig`, and writes a `redline` wrapper
that sets `LD_LIBRARY_PATH`. `<dest>/VERSION` records the REDline build and R3D SDK version.

A system-wide install by RED's installer also works: then `REDline` is on `PATH` and
`CFA4_REDLINE` is not needed.

## 4. Reference frames and metadata

```sh
scripts/dev/r3d-reference.sh --frames 3 --check-determinism clip.R3D target/fixtures/r3d/<name>
```

Fixed settings: CPU (`--gpuPlatform 0`), full resolution (`--res 1`), IPP2 (`--colorSciVersion 3`),
primary development only (`--primaryDev`: ungraded REDWideGamutRGB / Log3G10, the clip's look
metadata is not applied), uncompressed OpenEXR (`--format 2 --exrCompression 0`), no audio.

The output directory gets `frames/*.exr`, `metadata.txt` (`--printMeta 3`: header + CSV),
`SHA256SUMS`, `redline.log` and `manifest.txt` (clip sha256 and size, REDline build, R3D SDK
version, arguments, host, date, determinism result). With `--check-determinism` it renders twice
and fails if the files differ, which shows whether a sha256 comparison is enough for regression
tests or a numeric tolerance is needed.

Keep reference sets under `target/fixtures/` (gitignored) or in the external corpus; never commit
them (`AGENTS.md`: no large binary fixtures). Sample clips: RED publishes them at
<https://www.red.com/sample-r3d-files> (R3D NE samples: Nikon's download pages).

## 5. Checked so far, and what is still open

Checked on 2026-11-10 in a cloud container (Ubuntu, x86_64, no GPU) with build 65.0.22:

- the installer's archive is intact (MD5 `8e6c4a00f61e469506fb1ea988c32823`), and its embedded
  script only copies files into `/usr/local` and runs `ldconfig`;
- the unpacked REDline starts without root and reports R3D SDK 9.1.1;
- a missing or corrupt input exits with code 1 and "Unable to load clip", with no crash.

Still open, to run on the reference machine
([reference-machine.md](reference-machine.md)) with a real RED sample clip, because the cloud
container cannot download one:

- a full `r3d-reference.sh --check-determinism` run (CPU determinism, render time per frame);
- the exact EXR layout REDline writes (channels, half or float) and whether CFA4's own EXR reader
  (`crates/media/src/still/exr.rs`) opens it unchanged;
- `--printMeta` field list → mapping to CFA4 clip metadata (timecode, frame rate, ISO, white balance, lens).

The CFA4 side (finding REDline like `cfa4-testkit` finds ffmpeg, the import route in `cfa4-route`,
an import job with progress and cancel) belongs to the camera-RAW work package of the codec plan and
starts when that plan does.
