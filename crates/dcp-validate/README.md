# cfa4-dcp-validate

This crate is the DCP validation API, task DCP-03 of
[the DCP validation plan](../../docs/plans/DCP-VALIDATION-2026-10-27.md). It follows the layout and
API proposed in [DCP-00](../../docs/audit/DCP-00-CAPABILITY-AUDIT-2026-10-27.md) §6.3–6.4. It
contains:

- one trait for every backend;
- the seven result states;
- the package verdict;
- versioned profiles;
- the report;
- package discovery;
- the native backend.

```rust
use cfa4_dcp_validate::{DcpValidator, NativeValidator, Profile, ValidationControl, ValidationRequest};

let request = ValidationRequest::new("/media/DCP/Feature_FTR", Profile::builtin("quick-scan").unwrap());
let report = NativeValidator.validate(&request, &ValidationControl::new())?;
println!("{:?}\n{}", report.verdict, report.to_markdown());
```

## Results and verdict

Every rule a profile names gets at least one finding. Each finding has one of these states: `PASS`,
`FAIL`, `WARNING`, `SKIPPED`, `UNCHECKED`, `NOT_APPLICABLE` or `INTERRUPTED`.

If a backend does not implement a rule, it reports that rule as `UNCHECKED` and names the work
package that will add it. The rule is never left out.

The verdict is a separate field. It looks only at the profile's required rules:

| Verdict | When |
|---|---|
| `PASS` | Every required rule is `PASS` or `NOT_APPLICABLE`. Warnings are allowed. |
| `FAIL` | Any required rule is `FAIL`. |
| `INCOMPLETE` | No required rule failed, but one is `SKIPPED` or `UNCHECKED`. |
| `INTERRUPTED` | The validation was cancelled. |

Because of this, an incomplete check never adds up to `PASS` (plan §5–6). In particular, Quick Scan
never claims full integrity, because it does not contain the hash rule.

`Status::qc()` maps each state to a Media QC state as described in DCP-00 §6.4:

| DCP state | Media QC state |
|---|---|
| `PASS` | `Pass` |
| `WARNING` | `Warn` |
| `FAIL` | `Fail` |
| `SKIPPED`, `UNCHECKED`, `INTERRUPTED` | `NotRun` |
| `NOT_APPLICABLE` | left out |

## Rules and profiles

The rule catalogue is in [`src/rules.rs`](src/rules.rs). Rule ids are stable (`DCP.AM.001` …
`DCP.SIG.002`). If a rule's meaning changes, it gets a new id.

`DCP.REF.001` is in no profile. A reference-engine backend (`cfa4-dcp-inspect`, DCP-04) keeps
there every engine message that no catalogue rule covers, and a `FAIL` there fails the package.

The eight profiles of plan §5 are built in, all at version 1:

- `quick-scan`
- `full-integrity`
- `smpte-cinema`
- `interop-cinema`
- `subtitle-accessibility`
- `audio-quality`
- `security`
- `delivery-preflight`

A profile file format arrives with DCP-11. It will deserialise into the same `Profile` type.

The native backend implements these rules:

| Rule | Checks |
|---|---|
| `DCP.AM.001` | The asset map is present and readable. |
| `DCP.AM.002` | The asset map document is valid. |
| `DCP.AM.003` | No asset id appears twice in the asset map, and the map's own id is not an asset id. |
| `DCP.AM.004` | Chunk paths are safe. |
| `DCP.AM.005` | Chunk files exist. A single chunk's optional `Length` must equal the file length. |
| `DCP.AM.006` | The asset map marks at least one packing list. |
| `DCP.PKL.001` | The packing list documents are valid. |
| `DCP.PKL.002` | Every packing list asset is in the asset map. |
| `DCP.PKL.003` | Every file has the `Size` its packing list declares. An asset in two packing lists must have the same size in both. |
| `DCP.PKL.004` | Every file has the SHA-1 `Hash` its packing list declares. Hashing streams the files in 1 MiB steps with progress and cancel, and runs only when the profile has this rule. |
| `DCP.CPL.001` | The composition playlist documents are valid. |
| `DCP.CPL.002` | No composition playlist in the asset map is missing from the packing lists. |
| `DCP.CPL.003` | Every reel track file is in a packing list. One the asset map has but no packing list names is `FAIL`. One not in the package at all is `WARNING`: a version file plays with its original version. |
| `DCP.CPL.004` | `EntryPoint` + `Duration` ≤ `IntrinsicDuration`, and `Duration` is not 0. |
| `DCP.CPL.005` | The assets of a reel have the same `EditRate` and play for the same time (exact rational comparison). |
| `DCP.STD.001` | All documents use one standard (SMPTE or Interop). |
| `DCP.MXF.001` | Track files (see below). |
| `DCP.J2K.001` | Picture codestreams (see below). |
| `DCP.PIC.001` | Picture format (see below). |
| `DCP.MARK.001` | Markers (see below). |

`DCP.MARK.001` checks each composition's markers:

- `FAIL`: a marker offset is outside its reel;
- `FAIL`: a label appears twice;
- `FAIL`: a first-frame marker comes after its last-frame marker;
- `FAIL`: a marker lies outside FFOC … LFOC;
- `WARNING`: a label is not one of ST 429-7's;
- `WARNING`: the composition has no markers;
- `WARNING`: FFOC is later than frame 1;
- `WARNING`: LFOC is not on the last frame.

Tools differ on whether FFOC sits at offset 0 or 1 (CFA4's export writes 1), so both are accepted.

If no packing list or composition playlist was read, the rules over them are `UNCHECKED`, never a
vacuous pass. Sound and immersive audio come with DCP-07, subtitles with DCP-08, and the
signatures with DCP-10.

### Track files and pictures (DCP-06)

The track files are read with `cfa4-mxf` and the codestreams with `cfa4-j2k`; nothing is decoded.
Only a profile with one of the three rules opens the track files, and only one with
`DCP.J2K.001` reads every frame (a few small reads per frame, with progress stage `media` and
cancel).

`DCP.MXF.001` (ST 429-3, 429-6, 429-10), for every picture, sound and SMPTE subtitle reel asset
whose file is in the package, `FAIL` when:

- the file is not MXF, or not readable;
- it is not OP-Atom, or has more or fewer than one essence track;
- it holds another essence than the reel asset declares (JPEG 2000, PCM, timed text);
- its labels are of the other standard (the OP-Atom label is version 2 in SMPTE files, 1 in MXF
  Interop files);
- its edit rate or duration differs from the reel asset's `EditRate` or `IntrinsicDuration`;
- its file package UMID does not carry the reel asset's id;
- it is encrypted without a `KeyId`, not encrypted with one, or with another key;
- a stereoscopic file is in a `MainPicture`, or a mono file in a `MainStereoscopicPicture`.

Problems `cfa4-mxf` recovered from (a truncated partition, an unresolved reference) are `WARNING`.

`DCP.J2K.001` (ISO/IEC 15444-1 and its Amendment 1, as ST 429-4 requires) checks every frame of
every picture file in the clear. `FAIL` when a frame breaks the 2K or 4K digital cinema profile:

- `Rsiz` (3 for 2K, 4 for 4K), size, one tile at origin 0;
- three 12-bit unsigned components without sub-sampling;
- 1-5 decomposition levels (2K) or 1-6 (4K), CPRL, one layer, 9/7 wavelet, no MCT;
- 32 × 32 code-blocks without style flags, precincts 128 × 128 then 256 × 256;
- scalar expounded quantization with 1 guard bit (2K) or 2 (4K);
- 3 tile-parts (2K) or 6 (4K);
- a size other than the MXF picture descriptor's;
- more bytes than the DCI limit at the image rate: 250 Mbit/s a frame, 200 Mbit/s a component
  (above 48 images a second this is a `WARNING`).

A missing TLM marker, a 4K frame without a POC marker and a codestream that does not end with EOC
are `WARNING`. The same problem in many frames is one finding with the first frame and a count.
An encrypted file's codestreams are `UNCHECKED`: they need the content key.

`DCP.PIC.001` (DCI, ST 428-21, ST 429-7, ST 429-10):

- `FAIL`: the picture is larger than 4096 × 2160;
- `FAIL`: `FrameRate` is not the edit rate (twice the edit rate for stereoscopic pictures);
- `FAIL`: the edit rate is not a cinema frame rate (23.976 for example);
- `WARNING`: the size is not one of the DCI flat, scope or full container sizes;
- `WARNING`: `ScreenAspectRatio` is not the image's ratio (within 0.01 for an Interop decimal);
- `WARNING`: the rate is in the standard, but not every server plays it. 24 and 48 (2K) and 24
  (4K, stereoscopic) pass; SMPTE's 25, 30, 50, 60 and the 2K high frame rates, and Interop's 25,
  are warnings;
- `WARNING`: 4K stereoscopic pictures.

### Comparison with dcp_inspect

On CFA4 exports the rules were compared with dcp_inspect: a flipped byte, a grown file and a marker
outside its reel. Both backends report the same failures.

For DCP-06 the export's picture file was replaced by files asdcplib 2.13.3 wrapped from the same
frames: one for another asset, one with MXF Interop labels, one stereoscopic. dcp_inspect and the
native backend report the same problems: the asset id, the edit rate, the labels of the other
standard, the stereoscopic file in a `MainPicture`. On the export itself both report the J2K QCD
guard bits: CFA4's encoder writes 2 where the 2K profile has 1.

## Report

The report schema is `cfa4-dcp-report/1`. `to_json` produces JSON and `from_json` refuses any other
schema. `to_markdown` lists failures first.

The report contains:

- the profile id, version and title;
- the backend and its version;
- the standard;
- the asset map, packing list and playlist ids;
- the number of files checked;
- the counts per state;
- every finding, with its file, element path, line and column, asset id and evidence;
- the start and end times (UTC).

The HTML report is DCP-15.

## Safety

- Validation is read-only on the package.
- Chunk paths are refused before the file system is touched if they are absolute, contain `..`, a
  backslash or a NUL character, or are empty.
- A symbolic link is followed only when its target stays inside the package root.
- Each XML document is limited by the `cfa4-dcp` limits (32 MiB, 2 000 000 nodes, no DTD).
- The number of assets and documents read is capped.
- `ValidationControl` carries the cancel flag and the progress callback, so the engine can run
  `validate` as a background job.

## Tests

- 37 unit tests use synthetic packages:
  - unsafe paths, `..` and symbolic links leading outside the package;
  - each DCP-05 rule, with a passing and a failing package (size, hash, ids, packing list and
    composition consistency, version files, durations, reel sync, markers, chunk length);
  - a cancel while hashing, and while reading frames;
  - track files written with `cfa4-mxf` around frames `cfa4-j2k` encoded: a DCI file that passes,
    and each `DCP.MXF.001`, `DCP.J2K.001` and `DCP.PIC.001` failure and warning;
  - broken documents, a missing asset map, a missing packing list;
  - profiles and the verdict rules;
  - cancellation and progress;
  - the report.
- `tests/export_round_trip.rs` writes a DCP with `cfa4-export` and runs three profiles on it:
  - Quick Scan and Full Integrity, including the hashes, must pass;
  - SMPTE Cinema: the track files pass, the picture format is a warning (25 fps), and the only
    failure allowed is the J2K guard bits above.
