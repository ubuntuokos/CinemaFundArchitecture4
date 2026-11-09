# cfa4-interchange

Timeline interchange for CFA4: CMX 3600 EDL, Final Cut Pro 7 XML (xmeml), FCPXML,
OpenTimelineIO, AAF and OMF import and export, plus Avid ALE. Layer L2: no file I/O; media
references are strings and audio essence is supplied by the caller (the engine).

## Specifications

Clean-room: written from public specifications only. The AAF SDK, pyaaf2, the OMF Toolkit and
other implementations were not consulted. Where a specification leaves a layout open (how a
nested sequence is written, how several video tracks share a slot), what Premiere Pro does was
read off files it exports, never from the application itself.

| Format | Document | Edition | Used for |
|---|---|---|---|
| AAF | AMWA *AAF Object Specification* | v1.1 (2005) | classes, properties (ids shared with SMPTE ST 377-1 local tags), mob / slot / segment model, sequences and transitions, operation groups, parameters, definitions, descriptors, locators, essence data |
| AAF | AMWA *AAF Low-Level Container Specification* | v1.0.1 | objects as structured storages, `properties` streams, stored forms, strong / weak reference collections and their index streams, `referenced properties` |
| AAF | AMWA *AAF Edit Protocol* | v1.0 (AMWA AS-01) | top-level composition, mob chains (composition → master → file → physical source), operation definitions (dissolve, fade to black, audio gain / dissolve), usage codes |
| AAF | SMPTE RP 224 / RP 210 registers | — | data definition, class and type labels |
| AAF | Microsoft [MS-CFB] | rev. 10.0 | the container, see [`cfa4-cfb`](../cfb/README.md) |
| OMF | Avid *OMF Interchange Specification* | Version 2.0 (1997) | classes (`HEAD`, `CMOB`, `MMOB`, `SMOB`, `MSLT`, `TRKD`, `SEQU`, `SCLP`, `FILL`, `TRAN`, `EFFE`, `ESLT`, `CVAL`, `VVAL`, `TCCP`, `WAVD`, `AIFD`, `WAVE`, `AIFC`, locators), property and type names |
| OMF | Apple *Bento Specification* | revision 1.0d5 | container label, TOC encoding, objects / properties / types, references |
| WAVE / AIFF | Microsoft RIFF WAVE; Apple AIFF 1.3 | — | embedded and separate audio files |

The other formats' editions are listed in the module docs (`edl`, `fcp7`, `fcpxml`, `otio`, `ale`).

**Invariant — Unicode media URLs:** file URL prefix probes must not panic when their byte boundary
falls inside a UTF-8 character. Timeline importers use the shared converter; `tests/file_urls.rs`
checks expected Unicode paths through the public converter, resolver and EDL importer.

## AAF and OMF

`comp` holds a format-neutral mob-style model (compositions of slots whose sequences of fillers,
source clips and overlapping transitions reference media through master and file source mobs).
`aaf` and `omf` serialise it; both import back into the same model and from there into CFA4
sequences and media. Mapping details (what each CFA4 feature becomes) are in the `aaf` and
`omf` module docs. Times are kept in ticks in the model; compositions use the sequence frame rate
for picture and the sample rate for sound slots, so audio edits are sample-exact.

Embedded / consolidated audio (`essence`): `audio_needs` lists the media ranges an export
references (per media item or per clip, with handles); the engine decodes or renders them and
passes `AudioEssence` (embedded PCM or a written file) back in `MediaOptions`.

Not represented: speed changes and frame holds (exported at 100 % with a report entry),
graphics and synthetic media (gaps), video effects other than transitions, clip
markers (written to the master mob, so they come back as media markers), OMF video and markers.

## Nested sequences

For the formats Premiere Pro exports (FCP7 XML, OTIO, EDL, AAF, OMF) a nested sequence is written
the way Premiere Pro 26.5.2 writes it, as seen in the files it exports for a sequence with a nest.
Premiere Pro does not write FCPXML.

| Format | Written as | On import into CFA4 |
|---|---|---|
| FCP7 XML | a clip item holding the nested `<sequence>` (by id after the first use) | a sequence the clips use |
| FCPXML | a `ref-clip` to a `media` resource holding the sequence | a sequence the clips use |
| OTIO | a `Stack` inside the track | a sequence the clips use |
| EDL | one event: reel `AX` in every reel mode, the nested sequence's name as the clip name, its own time as source timecode (the report says the nest's edit is not in the EDL) | an ordinary clip of that name |
| AAF | a composition mob of its own (not tagged top-level); the nest's clips are source clips that point at it, at its first track of their kind. With embedded or consolidated audio the media inside the nest is prepared too (`NestNeeds::Inside`) | a sequence the clips use, read once however many clips use it |
| OMF | the nested sequence's sound, mixed by the engine with what is on the clip (`NestNeeds::Render`), as one clip named after the sequence. Without that essence: a gap, and the report names the sequence | media in the document |

A sequence that claims to be inside itself (a damaged project or file) is a gap with a report
entry in both directions; nothing recurses without a bound.

The AAF reader also accepts Premiere Pro's layout of a composition: all video tracks in ONE slot
(a nested scope with a segment per track, lowest first), and "top-level" tagged on nested
compositions too (the top-level ones are those no other composition uses).

## Graphics and adjustment layers

No format here has an equivalent of a CFA4 graphic (title) clip or an adjustment layer, and
none of the exports brings one back. Each such clip is named in the export `Report` as a warning
(`graphic clip "Title" at frame 48 …`, one line per clip), with what was written instead:

| Format | Written as | On import into CFA4 |
|---|---|---|
| FCP7 XML | a clip item without media (its layers as CFA4 filter ids) | not read back; the import report names the skipped clip item |
| FCPXML | a gap on V1; left out on other tracks (a gap exists only in the primary storyline) | not read back |
| OTIO | a clip with a `MissingReference` (effects in the `cfa4` metadata) | offline media |
| EDL | an ordinary event with the reel the reel mode gives | an ordinary clip |
| AAF | a gap | nothing |

OMF carries no video at all.

## Tests

- `src/otio.rs` (unit tests): OTIO repeats a nested sequence for every use, so compound clips that each hold
  several uses of the next are exponential (a 3 KB FCPXML nested 12 deep made the export run for minutes and grow to
  gigabytes). After 25 000 clips written inside nested sequences the rest are gaps, with one warning; an ordinary
  project is far below that and unchanged.
- `tests/aaf_omf.rs`: AAF round trips at 25, 29.97 DF, 23.976 and 59.94 DF (clips, gaps,
  two-sided and one-sided transitions, links, markers, start timecodes, constant and keyframed
  gain, media markers and timecode); 512-byte sectors; breakout to mono; embedded trimmed audio
  (sample data and source offsets); separate consolidated files with a video mixdown; OMF round
  trips with embedded audio, separate AIFF files and breakout; empty sequences; truncation and
  random corruption never panic. Nested sequences: AAF round trips as compositions (nests in
  nests, a nest used twice, another frame and sample rate, breakout to mono), OMF with the
  rendered sound, the audio needs of nests, a sequence inside itself.
- `tests/uncarried_clips.rs`: every format's export report names each graphic clip and adjustment
  layer once, as a warning; a sequence without them reports nothing; what a re-import gives back
  matches the report; the FCP7 XML import names a clip item whose file is not defined.
- `src/aaf/tests.rs`: written files checked against the required properties of every class
  written (Header, Identification, Mobs, slots, components, descriptors), weak references
  resolving into the dictionary, source clips resolving to mobs and slots, the Edit Protocol
  transition rules and sequence length arithmetic; every stored form round-trips through the
  container; a file rewritten into Premiere Pro's layout (video tracks as layers of one slot,
  nested compositions tagged top-level) reads back as the same edit.
- `src/omf/tests.rs`: the Bento label and TOC, the required properties of every OMF class
  written, reference resolution, embedded WAVE data, sequence lengths in samples.
- `tests/otio.rs`: a hand-written OTIO document (gaps, transitions, markers, legacy `Clip.1`),
  lossless round trips through the CFA4 metadata, collections, and a property test over random
  edits at five frame rates. `oracle_fixtures` exports these sequences plus a fixed one per frame
  rate (all transition alignments, speed, reverse, frame hold, a disabled clip, sub-frame audio)
  for the external OpenTimelineIO oracle (OTIO-1.1, `docs/plans/OTIO-INTEGRATION-2026-10-24.md`).

### OpenTimelineIO oracle

The official `opentimelineio` Python package (Apache-2.0) checks the files CFA4 writes. It is a
development tool only, like ffmpeg: never linked, imported by the product or shipped.

```sh
pip install -e '.[otio]'
CFA4_OTIO_ORACLE_DIR=target/fixtures/otio-oracle cargo test -p cfa4-interchange --test otio oracle_fixtures
python -m cfa4_devtools.otio_oracle target/fixtures/otio-oracle    # exit 0 PASS, 1 FAIL, 2 cannot run
```

A relative `CFA4_OTIO_ORACLE_DIR` is taken from the workspace root (Cargo runs the test from
`crates/interchange`), so both commands above use the same directory.

In CI (OTIO-1.3) the blocking `otio-oracle` job of the full validation (`workflow_dispatch` or the
`cfa4-full-validation` PR label; the fast PR gate is unchanged) runs both commands and the
fixture-backed oracle tests. It installs the oracle from its own hash-pinned lock,
`requirements/otio-oracle.txt` (`scripts/dev/lock-python.sh` regenerates it with the main lock), so
`opentimelineio` stays out of the development and Qt environment:

```sh
python -m pip install --require-hashes -r requirements/otio-oracle.txt
```

For each file it reads the document with OTIO, re-serialises it, and compares tracks, clip names,
marker counts and every clip's position and length, as OTIO's own `range_in_parent` computes
them, with `manifest.json` (exact ticks, 1 µs tolerance for OTIO's floating-point times).

The other direction (OTIO-1.2): the library writes samples for every schema the reader handles
(`Timeline.1`, `Stack.1`, `Track.1`, `Clip.1`/`Clip.2`, `Gap.1`, `Transition.1`,
`Marker.1`/`Marker.2`, the external, missing, generator and image-sequence references,
`LinearTimeWarp.1`, `FreezeFrame.1`, `Effect.1`, `SerializableCollection.1`, a bare track as the
whole document, a clip with several media references and an active key), and `oracle_samples`
imports them:

```sh
python -m cfa4_devtools.otio_samples target/fixtures/otio-samples
CFA4_OTIO_SAMPLES_DIR=target/fixtures/otio-samples cargo test -p cfa4-interchange --test otio oracle_samples
```

`samples.json` holds what OTIO sees in each file: per timeline the frame rate and start timecode;
tracks (video first, as CFA4 orders them); every clip's and nested stack's position and length;
transition counts and positions; marker positions (clip markers in media time, from the start of
the available range); per clip whether it is enabled, its speed, reverse and hold, and the media it
points at (file path, generator, offline, or a nested sequence); and the import warnings that are
expected (an unknown generator, an effect CFA4 does not know, a file reference without a path, a
clip directly in a stack). Any other warning, or a missing expected one, fails the test. The rates
are 23.976, 24, 25, 29.97, 30, 50, 59.94 and 60 fps. How an image sequence is kept is OTIO-2.3; its
path is not compared yet. Relative media paths resolve under `/media`, the base the test passes.
