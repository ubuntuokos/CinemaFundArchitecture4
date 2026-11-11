# Media QC

Local quality control of media files before and after a render (`crates/qc`, `cfa4-qc`, layer
L3). Plan and owner decisions: [plans/MEDIA-QC-2026-10-27.md](plans/MEDIA-QC-2026-10-27.md).

QC reads media and writes its own report. It never changes a project, a timeline or render
settings, needs no cloud service, and runs on the CPU (a GPU path is optional). Measurements
use CFA4's own demuxers and decoders; an FFmpeg cross-check stays `NOT_RUN` until the
`cfa4-ffmpeg` backend exists ([ADR 0005](adr/ADR-0005-media-backends.md)).

Status: stages B–C (report schema, profiles, input and integrity checks, `cfa4-cli qc`). The
audio and picture checks (stage D) report NOT_RUN for now; the engine commands and the Qt 6
review window follow in stage E.

## Command line

```sh
cfa4-cli qc export.mp4 --expect plan.json --out export.mp4.cfa4qc.json
cfa4-cli qc export.mp4 --profile my-profile.toml      # report JSON on stdout
cfa4-cli qc export.mp4 --no-decode                    # only what the file says about itself
```

The report is written atomically. Exit code: 0 PASS, 3 WARN, 4 FAIL, 5 INCOMPLETE (1 is an
error, 2 a usage error).

`--expect` is the render plan, JSON; every field is optional and a check whose expectation is
missing reports NOT_RUN ("no render plan given"):

```json
{"video": true, "frames": 240, "frameRate": {"num": 24000, "den": 1001}, "width": 1920,
 "height": 1080, "videoCodec": "H.264", "audioTracks": 1, "audioChannels": 2,
 "sampleRate": 48000, "audioCodec": "AAC", "subtitleStreams": 0}
```

## What the input and integrity checks look at

The file is opened with CFA4's own demuxers (`cfa4-media`, `cfa4-codecs`) and read in place.
For **MP4 / MOV** (the containers CFA4 exports H.264, HEVC and AV1 to) QC also reads the sample
index and decodes every video and audio sample with CFA4's software decoders, sample by sample,
so a damaged picture is reported at its frame and time code instead of being concealed as
playback would. A decoder that crashes on a sample is reported as a decode error. Other
containers get the stream checks; the sample-level checks report NOT_RUN ("sample-level analysis
covers MP4 / MOV only"). A stream with no decoder in this build (for example an AV1 profile the
decoder does not support) makes `integrity.decode` NOT_RUN, never PASS.

| Check | Looks at |
|---|---|
| `probe.open` | the file opens; container, stream counts |
| `probe.video` | codec, size, frame rate, time base, pixel format, pixel aspect; against the plan |
| `probe.color` | primaries, transfer, matrix, range, HDR metadata (PQ without mastering metadata warns) |
| `probe.audio` | sample rate, channels, codec per stream; against the plan |
| `probe.consistency` | duration present, empty tracks, codec setup changes, variable frame rate (warn), broken timestamps |
| `integrity.decode` | every sample decodes (errors with frame / time code) |
| `integrity.frameCount` | decoded pictures minus the planned frame count |
| `integrity.duration` | presented video duration (edit list applied) minus the planned one, ms |
| `integrity.frames` | decode timestamps going backwards, two pictures at one time (FAIL), pictures the decoder did not return, size changes; steps longer than 1.5 frame periods (WARN, the frames a constant-rate plan misses). Repeated identical pictures are counted in the detail, not judged: stills repeat by design |
| `integrity.avOffset` | end of the first audio track minus end of the video, ms (a different start is noted) |
| `integrity.streams` | video present or not, audio track count and channels, subtitle streams; against the plan |

## Checks

Every check has a stable id (`crates/qc/src/catalog.rs`). A check either measures a **value**
in a fixed unit, judged by the profile's bounds, or reports **findings** (events with a frame
or time code). Ids are part of the report format and are never renamed.

| Group | Ids |
|---|---|
| input | `probe.open`, `probe.video`, `probe.color`, `probe.audio`, `probe.consistency` |
| integrity | `integrity.decode`, `integrity.frameCount` (frames), `integrity.duration` (ms), `integrity.frames`, `integrity.avOffset` (ms), `integrity.streams` |
| audio | `audio.silence`, `audio.clipping` (samples), `audio.truePeak` (dBTP), `audio.loudness` (LUFS), `audio.loudnessRange` (LU), `audio.layout` |
| picture | `video.contactSheet`, `video.black`, `video.clipBlack` (%), `video.clipWhite` (%), `video.referenceSsim` (SSIM) |

`integrity.frameCount` and `integrity.duration` are the difference from the render plan
(0 = as planned).

## Profiles (`cfa4-qc-profile/1`)

A profile is a TOML file that lists the checks to run, whether each is `mandatory` (default) or
`optional`, and for value checks the bounds:

- inside `pass` → `PASS`;
- inside `accept` but outside `pass` → `WARN`;
- outside `accept` (or outside `pass` when there is no `accept`) → `FAIL`;
- no finite value → `NOT_RUN`.

`human_review = "mandatory"` makes the guided human review part of the verdict. There is no
global threshold: loudness and picture bounds always come from the profile chosen for the
export's purpose. Unknown keys and unknown check ids are errors. Raise `version` whenever a
bound changes; a report records the profile's id, version and the SHA-256 of its file.

Built-in: [`delivery-web`](../crates/qc/profiles/delivery-web.toml) (v1): online platforms,
loudness −15…−13 LUFS (warn to −16…−12), true peak ≤ −1 dBTP (warn to 0), frame count exact,
duration and A/V end within one frame at 23.976 fps.

## Report (`cfa4-qc-report/1`)

JSON with: the profile reference; the checked file (`output`: path, SHA-256, size) and
optionally the source (`input`); links to project, sequence, export job and preset; the render's
own outcome (`render`, kept apart from the QC result); tool versions (CFA4, and FFmpeg/ffprobe
or `null` when not used); run info (start, duration, executable, CPU/GPU, cancelled); one result
per profile check (status, value, unit, `pass`/`accept` bounds, events with frame / tick /
time code, reason for `NOT_RUN`, measured detail); the human review (answers to the guided
questions, findings with severity, time code and note); and the summary.

Same input and profile give the same report bytes, apart from the `run` section.

### Verdict

1. A mandatory check `FAIL`, or a critical human finding → `FAIL`.
2. Otherwise a mandatory check `NOT_RUN` or missing, a required human review not done, a render
   that did not finish, or a cancelled run → `INCOMPLETE`.
3. Otherwise any `WARN`, an optional check `FAIL`, or a major/minor human finding → `WARN`.
4. Otherwise `PASS`.

Only `PASS` and `WARN` allow the "quality checked" state. Skipped checks are listed with their
reasons in `summary.skipped`. Reading a report checks that every status matches its value and
bounds and that the summary matches the results, so a hand-edited or truncated report is
refused instead of trusted.

## Test material

All QC test media is synthetic, generated by CFA4 code into `target/fixtures/` (never
committed). Third-party test patterns (for example Cinematesttools) are not bundled: their
licences are inconsistent or non-commercial; see the plan, §2.
