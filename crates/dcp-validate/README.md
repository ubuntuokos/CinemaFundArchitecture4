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
vacuous pass. The media rules come with DCP-06 to DCP-08, and the signatures with DCP-10.

On CFA4 exports the rules were compared with dcp_inspect: a flipped byte, a grown file and a marker
outside its reel. Both backends report the same failures.

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

- 31 unit tests use synthetic packages:
  - unsafe paths, `..` and symbolic links leading outside the package;
  - each DCP-05 rule, with a passing and a failing package (size, hash, ids, packing list and
    composition consistency, version files, durations, reel sync, markers, chunk length);
  - a cancel while hashing;
  - broken documents, a missing asset map, a missing packing list;
  - profiles and the verdict rules;
  - cancellation and progress;
  - the report.
- `tests/export_round_trip.rs` writes a DCP with `cfa4-export` and runs three profiles on it:
  - Quick Scan and Full Integrity, including the hashes, must pass;
  - SMPTE Cinema must have no `FAIL` and be `INCOMPLETE` until the media rules (DCP-06 to DCP-08)
    arrive.
