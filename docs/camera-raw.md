# Camera RAW in CFA4: format survey

Work package 11.0 of the professional codec plan ([`plans/CFA4-PROCODEC-002-2026-11-11.md`](plans/CFA4-PROCODEC-002-2026-11-11.md)
§11). This page states, per camera RAW format, what exists publicly, what route CFA4 can take under its licence
policy ([`licensing.md`](licensing.md) §4), and the status that the capability catalog (`cfa4-cli codecs`) should
show. It is not legal advice. Vendor SDKs, tools and their documents stay in the owner's private storage and are
never committed.

## Status per format

The statuses are the ones the plan's acceptance names: **native** (our own decoder), **SDK adapter** (a
separate process that links a user-installed vendor SDK), **external tool** (a user-installed vendor command-line
program that CFA4 runs as a separate process to transcode or proxy), **metadata / proxy only** and **not
supported**.

| Format (cameras) | Public specification | Linux vendor software | CFA4 route | Status today | Target status |
| --- | --- | --- | --- | --- | --- |
| CinemaDNG (DJI, Blackmagic older models, others) | yes (Adobe DNG / CinemaDNG) | — | own decoder, `crates/dng` | native | native |
| Apple ProRes RAW (Atomos and other recorders, DJI, Kinefinity, Panasonic, Nikon via recorder) | no Apple specification; FFmpeg 8.0 ships an LGPL decoder (`libavcodec/prores_raw.c`) | — | port of the FFmpeg decoder (LGPL-2.1-or-later is allowed by `licensing.md` §3; the port goes into the reused-code register), FFmpeg ≥ 8.0 as the test oracle | not supported | native (11.4) |
| Blackmagic RAW (`.braw`) | no | Blackmagic RAW SDK 6.0 (x86_64 `.so`) | GPL-3.0-or-later adapter with a GPL-3 §7 permission to link the SDK, separate process; the user installs the SDK (owner decision 2026-10-31, recorded in `licensing.md` §4) | not supported | SDK adapter (11.2) |
| RED R3D, R3D NE | no | REDline (free command-line renderer) | REDline as an external process: transcode to EXR / DPX / ProRes, metadata from `--printMeta`; developer tools in [`development/redline.md`](development/redline.md) | not supported | external tool (11.2) |
| ARRIRAW, ARRIRAW HDE, ARRICORE (ALEXA 35, Mini LF, LF, 65) | no | ARRI Reference Tool CMD (`art-cmd`), minimum 1.1.1 | `art-cmd` as an external process: linear EXR, WAV and JSON metadata; real-time playback would need an ARRI Image SDK partner licence | not supported | external tool (11.2) |
| Sony X-OCN (XT / ST / LT), Sony RAW (VENICE, VENICE 2, BURANO, FX5) | no (decoding algorithm not published) | none for Linux; SDK only through Sony's licence programme | read the MXF and the NonRealTimeMeta XML for timecode, camera and lens metadata; show the camera proxy or a transcode | not supported | metadata / proxy only (11.2), SDK adapter only if Sony offers a Linux SDK |
| Canon Cinema RAW Light (`.crm`) | no | Canon SDK not obtainable (developer registration not working, 2026-11-11) | metadata and the camera's proxy | not supported | metadata / proxy only until the SDK can be evaluated |
| Nikon N-RAW (`.nev`, Z8 / Z9) | no public specification found | no Linux SDK found | none yet; the same cameras can record ProRes RAW | not supported | not supported (revisit if Nikon publishes an SDK) |

## Common requirements (11.1–11.3)

Whatever the route, each format must keep:

- **Timecode and reel:** start timecode, drop-frame flag and reel / clip name from the file or its sidecar, so
  multi-camera sync by timecode works (RED broadcast remote recording is the typical many-camera case).
- **Audio:** the camera's audio tracks (X-OCN MXF: 4 channels, 24-bit, 48 kHz; `art-cmd` writes WAV).
- **Colour:** the camera log and gamut chosen per clip from its metadata, not from a project default: Log3G10 /
  REDWideGamutRGB for R3D, S-Log3 with S-Gamut3 or S-Gamut3.Cine for Sony, and the SMPTE ST 2115 / ST 2113 MXF
  labels where the file carries them (plan §11.3).
- **Settings:** debayer quality, white balance, exposure / ISO and full resolution or proxy, as clip settings that
  the Source Settings command (`project_tools.rs`) can show once a RAW decoder exists.
- **Safety:** no automatic SDK download, no vendor library loaded into the CFA4 process (the BRAW adapter is a
  separate process too), and an adapter or tool crash must not bring CFA4 down ([`../AGENTS.md`](../AGENTS.md) §0).
- **Tests:** vendor sample clips are downloaded by a script into `target/fixtures/` with a pinned sha256, never
  committed; the vendor tool (REDline, `art-cmd`) or FFmpeg is the oracle where one exists.

## Order

1. **ProRes RAW native decoder** (11.4): the only format with a licence-compatible open implementation, so it
   needs no vendor software and covers the recorders of several camera makers. Estimate 30–50 h.
2. **External-tool imports** for R3D (REDline) and ARRI (`art-cmd`), sharing one import job with progress,
   cancel and a version check (11.2). Estimate 10–20 h for REDline, 8–15 h for `art-cmd`.
3. **Sony and Canon metadata / proxy** (11.2). Estimate 10–20 h for Sony.
4. **BRAW SDK adapter** after the adapter protocol (11.1). Estimate 25–45 h for the protocol, 20–60 h for the
   adapter.

## Open questions for vendors

- Sony: is there a Linux SDK, and does it decode the FX5's new X-OCN family ("H")? The owner contacts Sony.
- ARRI: does the ARRI Image SDK decode HDE on Linux, or does that need a CODEX SDK?
- Canon: access to the Cinema RAW Light SDK and whether it runs on Linux.
- Nikon: whether an N-RAW SDK exists for Linux.

## Revisions

| Date | Change |
| --- | --- |
| 2026-11-12 | First version from the survey in the codec plan §11 and a check of FFmpeg 8.0 (ProRes RAW decoder). |
