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

The native backend currently implements seven rules:

| Rule | Checks |
|---|---|
| `DCP.AM.001` | The asset map is present and readable. |
| `DCP.AM.002` | The asset map document is valid. |
| `DCP.AM.004` | Chunk paths are safe. |
| `DCP.AM.005` | Chunk files exist. |
| `DCP.PKL.001` | The packing list documents are valid. |
| `DCP.CPL.001` | The composition playlist documents are valid. |
| `DCP.STD.001` | All documents use one standard (SMPTE or Interop). |

These are exactly the Quick Scan rules plus the standard check. The integrity and consistency rules
come with DCP-05, the media rules with DCP-06 to DCP-08, and signatures with DCP-10.

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

- 18 unit tests use synthetic packages:
  - unsafe paths, `..` and symbolic links leading outside the package;
  - broken documents, a missing asset map, a missing packing list;
  - profiles and the verdict rules;
  - cancellation and progress;
  - the report.
- `tests/export_round_trip.rs` writes a DCP with `cfa4-export` and runs Quick Scan on it, which must
  pass. It also runs the SMPTE Cinema profile, which must have no `FAIL` and be `INCOMPLETE`.
