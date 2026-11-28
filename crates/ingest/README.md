# cfa4-ingest

The shared recorder core of CFA4 (layer L3). Plan: `CFA4-TV-INGEST-RUNDOWN-001-RESZLETES-TERV`
(phase IN-1; owner-approved 2026-11-27) and the recording contract of
`docs/plans/CFA4-LIVE-STUDIO-001.md` annex X (CFA4-DECKLINK-READY-001, R2).

One core, many backends: the libobs recorder (live studio S5) and the DeckLink recorder implement
`Recorder`; this crate holds what every backend shares. It contains **no device or encoder code**
(no DeckLink, no libobs, no SDK) and does no file I/O in this phase.

| Module | What |
|---|---|
| `contract` | `RecordJob`, `Recorder`, `RecordHandle`, `RecordReport` (annex X) |
| `extent` | `Extent` (written by the recorder) and `ExtentView` (read-only, for the editor): how far the media may be read safely; a seek past it is `SeekCheck::NotYetWritten`, never a wrong picture |
| `session` | the session state machine and its journal: `Idle → Starting → Recording → Stopping → Finalized`; `Interrupted → Recoverable → Recovered \| Quarantined`; an invalid transition is an error |
| `events` | typed events for the Event Fabric catalogue (names are provisional until the catalogue owner confirms them) |
| `testsrc` | `TestRecorder`: deterministic synthetic backend with fault injection, so the core is testable without hardware |

Rules: time is `cfa4_time::Tick`; no `unsafe`; no panics outside tests; every number in a job is
hostile input and checked. The File Services types (`StagedFile`, `FileId`) do not exist yet:
`StagedTarget` and `String` stand in for them and are replaced when FS-1 lands.
