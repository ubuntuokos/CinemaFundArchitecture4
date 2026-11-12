# cfa4-mxf

A clean-room MXF demuxer and writer. Layer L0: no dependencies beyond `std`, no `unsafe`, builds for
`wasm32-unknown-unknown`. No GPL/LGPL code (FFmpeg) was consulted; the BSD-3-Clause bmx (BBC) was
read for the D-10 and RDD 9 layouts (register in `docs/licensing.md` §5, nothing copied). FFmpeg is
used only as an external fixture generator and test oracle.

## Specifications

Implemented from the SMPTE documents (editions used):

| Document | Edition | Used for |
|---|---|---|
| SMPTE ST 336 | 2017 | KLV coding: 16-byte UL keys, BER lengths |
| SMPTE ST 377-1 | 2011 (+ Amd 1:2012) | partitions, primer pack, header metadata sets and local tags, index table segments, random index pack, run-in |
| SMPTE ST 378 | 2004 | OP1a |
| SMPTE ST 390 | 2011 | OP-Atom |
| SMPTE ST 391 / 392 / 393 / 407 / 408 | 2004 / 2004 / 2004 / 2006 / 2006 | OP1b (ganged packages), OP2a/b (playlist items), OP3a/b (edit items), OP1c–OP3c (alternate packages) |
| SMPTE ST 379-1 / 379-2 | 2009 / 2010 | generic container: content packages, element keys, frame / clip wrapping |
| SMPTE ST 381-1 | 2005 | MPEG video mapping (MPEG-2 identified, MPEG video descriptor; decoded by `cfa4-mpeg2v` in `cfa4-codecs`); writer: ES container, element keys, MPEG video descriptor, index flags |
| SMPTE ST 381-3 | 2013 | AVC byte-stream mapping and AVC sub-descriptor |
| SMPTE ST 382 | 2007 | AES3 and Broadcast Wave audio mapping (wave / AES3 descriptors) |
| SMPTE ST 331 | 2011 | element data of 8-channel AES3 sound (D-10; reader and writer), ST 12-1 time code metadata of the system item |
| SMPTE ST 385 | 2012 | SDTI-CP compatible system item (writer: system metadata pack, package metadata set) |
| SMPTE ST 2067-201 | 2019 | IAB track files: IAB essence descriptor (set 0x7B), clip-wrapped container, element type 0x0D (labels from the SMPTE registers) |
| SMPTE RDD 9 | 2013 | MPEG long GOP coding labels (Table 6: MP@HL, 422P@HL, MP@H-14) for the writer; the XDCAM HD422 OP1a layout (`WriterConfig::rdd9`) |
| SMPTE ST 384 | 2005 | uncompressed picture mapping: container label (mapping 05h, frame / clip / line wrapping), element types 02h / 03h / 04h |
| SMPTE ST 377-1 | 2019, G.1, G.2.22–G.2.25, G.2.35, G.2.40 | uncompressed layouts: stored / sampled / display rectangles, image start / end fill, the Y'CbCr layout labels of G.2.25, Reversed Byte Order, Padding Bits, RGBA Pixel Layout (with X'Y'Z', split, float and palette codes), Palette, Palette Layout and Component Min / Max Ref |
| SMPTE ST 386 | 2004 | D-10 (IMX) mapping: container label, sound element type; writer: content packages on a 512-byte KAG, constant-size index with delta entries, Annex A descriptor values |
| SMPTE ST 2019-4 | 2009 | VC-3 (DNxHD / DNxHR) mapping |
| SMPTE RDD 44 | 2017 | Apple ProRes mapping |
| SMPTE RP 224 / RP 210 | registers | labels: data definitions, picture coding, essence containers, operational patterns; property labels of the primer pack (writer) |
| SMPTE ST 330 | 2011 | basic UMIDs of the written packages |
| SMPTE ST 429-3 / 429-4 | 2023 | D-Cinema track files (writer): OP-Atom frame wrapping, header metadata only in the header partition, UUID-based package UMIDs, channel assignment; JPEG 2000 picture mapping |
| SMPTE ST 429-18 | 2023 | Immersive audio track files (writer and reader): data element key, essence container, IAB data essence descriptor and sub-descriptor |
| SMPTE ST 429-5 | 2023 | Timed text track files (reader): TimedTextDescriptor (set 0x64) and TimedTextResourceSubDescriptor (set 0x65) with their dynamic properties, the clip-wrapped timed text element (item type 0x17), the timed text essence container label; also the IMF timed text track files of ST 2067-2 |
| SMPTE ST 436-1 | 2013 (revision of 436M-2006) | ANC packets (reader): the ANC packet essence container (mapping 0Eh), frame-wrapped ANC elements (item 17h, element type 02h) and VI elements (element type 01h, VBI line essence container 0Dh, VBI Data Descriptor 5Bh, 1/8/10-bit sample codings), ANC structures with 8-bit and 10-bit payload codings and every Annex B padding scheme, the ANC Data Descriptor (set 5Ch) sample rate; writer: the ANC data track with 8-bit coded packets |
| SMPTE ST 410 | (via ST 429-5 clause 6) | generic stream partitions: the ancillary resources (fonts, PNG subpictures) of timed text track files, found by BodySID |
| SMPTE ST 429-6 | 2023 | Encrypted track files (reader): Cryptographic Context set, Encrypted Triplet pack, AES-128-CBC decryption with the Check Value, HMAC-SHA1 MIC |
| SMPTE ST 422 | 2022 | JPEG 2000 container label and JPEG 2000 picture sub-descriptor (writer) |
| SMPTE ST 2067-2 / 2067-5 | 2020 | IMF track files (writer): OP1a, KAG 1, at least 8 KiB of fill after the header, primary package, frame-wrapped pictures in 60 s essence partitions each followed by an index partition, clip-wrapped 24-bit sound at the sample rate with a constant-rate index, UMIDs, the MCA channel assignment, reference image edit rate and audio alignment level |
| SMPTE ST 2067-21 | 2026 | IMF App 2E pictures (writer): RGBA (QE.1 / QE.2 component ranges) or CDCI (narrow-range levels) descriptor, the JPEG 2000 sub-descriptor with coding style, quantization and J2CLayout |
| SMPTE ST 422 | 2022, Table 3 | the IMF JPEG 2000 picture coding labels (profile and level from Rsiz) |
| SMPTE ST 377-4 | (via asdcplib's dictionary and real DCP-o-matic files) | MCA labels (writer): soundfield group and audio channel label sub-descriptors on the sound descriptor |

## Timed text track files

`is_timed_text(head)` recognises a DCP or IMF subtitle track file from its header partition;
`open_timed_text(src)` returns the XML document's location, its namespace, resource id and the
ancillary resources (fonts, images: id, MIME type, generic stream). `open` reports such a file as
`Unsupported` (it has no picture or sound). `cfa4-engine` imports the XML as a caption track
(ST 428-7 D-Cinema subtitles through `cfa4_captions::dcst`, IMSC1 through `cfa4_captions::ttml`).
In an encrypted track file (ST 429-6) the document and the resources are Encrypted Triplets
(`Region::plain`, `TimedText::encryption`): `read_document` / `read_resource` decrypt them with
the subtitle key (MDSK) from the key ring, or fail with `Error::Encrypted` naming its Key ID.

The `test-util` feature exposes `testing::build_timed_text` and the other synthetic file builders
for tests in other crates.

## ANC data (closed captions)

`has_anc(head)` tells from the header partition whether a file lists the ANC packet essence
container; `read_anc(src)` returns every ANC element of the first ANC data track, one per frame
(ST 436-1 stores them in display order), with the track's edit rate. Each `AncPacket` keeps its
line number, wrapping and coding and the low 8 bits of its words from the DID through the user
data. `AncData::cdps` lists the caption distribution packets (DID 61h, SDID 01h, SMPTE ST 334-2);
`cfa4-engine` decodes them with `cfa4_captions::mcc::decode_cdps` into a caption track when such
a file is imported (#929). `AncData::cea608` lists the byte pairs of ST 334-1 Annex B CEA-608
packets (DID 61h, SDID 02h) with their field. VI elements (element type 01h, the VBI line
essence container, mapping 0Dh, VBI Data Descriptor 5Bh) are read too, also from files that
have only them: `AncData::vi` holds each frame's `ViLine`s (line number, wrapping, coding and the
samples as 10-bit values: 1-bit samples black 64 / white 940, 8-bit ones shifted up, 10-bit ones
three per word), and `ViLine::field2` tells the field. The engine decodes digitised line 21 from
them with `cfa4_captions::cea608::decode_line21` and imports each CEA-608 channel (CC1–CC4) as
a caption track when there are no CDPs (#1005).

Writing (#1004): `WriterConfig::anc` adds an ANC data track to an OP1a file with pictures (also
RDD 9; not D-10, whose content packages have no data item): the ANC Data Descriptor (set 5Ch),
the ST 436 essence container label, and per content package one frame-wrapped ANC element after
the picture and sound, from `MxfWriter::push_anc` (an empty element when none was pushed), in
its own index slice. `anc_element(packets)` builds an element value with the 8-bit luma coding
(04h). `cfa4-export` uses it for `ExportSettings::mxf_anc_captions`: the first caption track as
one CDP per frame (`cfa4_captions::mcc::CdpStream`, CEA-608 field 1 and CEA-708 service 1) on
VANC line 9, at every ST 334-2 rate (23.976, 24, 25, 29.97, 30, 50, 59.94, 60 fps).

## Peeking at the header

`peek(src)` reads only the header partition pack and header metadata: the operational pattern
and the material and file package UMIDs. `cfa4-engine` uses it to find the OP-Atom files of one
clip (ST 390: one essence track per file, a shared material package) in an Avid MediaFiles folder
or on a P2 card (`CONTENTS/VIDEO` + `CONTENTS/AUDIO`), and `cfa4_codecs::MxfSource::open_group`
opens them as one source.

## Reading a file

```rust
let file = std::fs::read("clip.mxf")?;
let mxf = cfa4_mxf::open(&file)?;
println!("{} {:?}", mxf.operational_pattern.name(), mxf.timecode.map(|t| t.format()));
let v = mxf.track_of_kind(cfa4_mxf::TrackKind::Picture).unwrap();
let t = &mxf.tracks[v];
let key = t.sync_before(t.sample_at(42).unwrap());    // stored index of the random-access picture
let bytes = mxf.read_sample(&file, v, key)?;           // one edit unit's essence
let a = mxf.track_of_kind(cfa4_mxf::TrackKind::Sound).unwrap();
let pcm = mxf.read_pcm(&file, a, 48_000, 1920)?;       // planar f32, sample-exact
```

- **Open.** The header partition pack is found in the run-in (≤ 64 KiB). Every KLV packet is then
  walked (keys and lengths only; essence values are skipped, never read): partition packs, primer
  packs, header metadata local sets, index table segments, fill, and generic-container elements
  (system items are ignored). A lost KLV sync resynchronises on the next partition pack; a
  truncated file keeps everything before the cut (a truncated frame-wrapped picture is dropped,
  clip-wrapped essence keeps its whole edit units).
- **Metadata.** The header metadata of the most complete partition is used (closed complete >
  open complete > closed > open; later partitions win ties, so the footer's final durations are
  preferred). Dynamic local tags resolve through the primer pack (e.g. `SubDescriptors`).
- **Tracks.** Material package picture/sound tracks → source clip → file package track (track
  number, edit rate, origin) → descriptor (a multiple descriptor's sub-descriptor by
  `LinkedTrackID`). Without a usable material package the file packages' tracks are used directly.
  Elements are matched by track number, or — when the number is 0 or does not match (some OP-Atom
  writers) — the only unclaimed element stream of the right kind.
- **Pictures.** One `Sample` per edit unit in stored order: frame wrapping (one element each) or
  clip wrapping (one element split by the index: constant `EditUnitByteCount` or VBR entries).
  Random access from index flag bit 7 (intra-only codings: every picture); without an index the
  essence is scanned (AVC IDR, MPEG-2 sequence header). Presentation order from the index temporal
  offsets (display position *n* is stored at *n* + `TemporalOffset[n]`). When the index marks B
  pictures but has no temporal offsets (FFmpeg-written AVC), `needs_reorder` is set: the caller
  orders by the bitstream (`cfa4-codecs` uses the AVC picture order counts).
- **Codecs.** From the picture essence coding label, then the essence container label, then the
  essence bytes: AVC / AVC-Intra, VC-3, ProRes (profile from the label), MPEG-2 (decoded in `cfa4-codecs`),
  MPEG-4 visual, JPEG 2000 (decoded in `cfa4-codecs` with `cfa4-j2k`), DV, uncompressed.
- **Uncompressed pictures** (`uncompressed`, ST 384). `Layout::of` takes the layout from the
  picture essence coding label (the nine Y'CbCr layouts of ST 377-1 G.2.25: 8-bit 4:4:4, UYVY,
  YUY2, planar 4:2:2 and 4:2:0, v210, planar 10-bit, 12- and 16-bit Cb Y Cr Y), else from the
  descriptor: RGBA Pixel Layout (R, G, B, A and fill components up to 16 bits, Component Min /
  Max Ref rescaled to full range; components split into upper and lower fields `R`…`r`; DCDM
  X' Y' Z' codes D8h–DAh, converted to RGB in `cfa4-codecs`; HALF, 32- and 64-bit float
  components, clipped to 0–1 and scaled to 16 bits, read most significant byte first because
  ST 377-1 does not name a byte order; palette indices `P` looked up in the Palette with its
  Palette Layout, an index past the end is black), or CDCI depth, subsampling, Reversed Byte Order and Padding
  Bits (10-bit 4:2:2 without a label is v210). `decode` unpacks one edit unit into planes: image
  start / end fill skipped, separate fields woven (field 1 on the even lines), the display
  rectangle given from the sampled and display offsets. G.2.25 draws every layout as big-endian
  bit-fields; v210 and planar 10-bit are read as files store them: little-endian 32-bit words,
  first component in the low bits (FFmpeg decodes our v210 files identically). A clip-wrapped
  element without an index is split by the edit unit size. Not supported: line wrapping (the
  track is listed as unknown with a warning), palettes with float entries.
  `PictureInfo::rgb` tells R'G'B' components (RGBA descriptor or GBR coding equations).
- **Sound.** PCM chunks (frame- or clip-wrapped) with sample counts; `read_pcm` decodes
  little-endian 8/16/24/32-bit PCM (wave / AES3 descriptors) and ST 331 AES3 elements (D-10).
  Immersive Audio (IAB, `Codec::Iab`): one `Sample` per IABitstream frame, from frame-wrapped
  elements, the index of a clip-wrapped element, or, without an index, the frames' own
  tag-length framing; decoded by `cfa4-iab`.
- **Timecode.** The material package timecode component (start frame count, rounded base,
  drop frame), falling back to the file package's; `Timecode::format` gives `HH:MM:SS:FF` /
  `HH:MM:SS;FF`.

- **Encrypted track files** (ST 429-6, #933). The Cryptographic Context set gives each encrypted
  track its Key ID (`EssenceTrack::encryption`); Encrypted Triplets are filed under the Source Key
  of the triplet they wrap, so tracks, samples and chunks build as usual. `read_sample` /
  `read_pcm` decrypt them (AES-128-CBC, Check Value test; the HMAC-SHA1 MIC is checked when a MIC
  key is loaded) with the content key from the in-memory key ring (`crypt::add_key`, filled from a
  KDM by `cfa4-dcp` and the engine's `dcp.loadKdm`); without it they fail with `Error::Encrypted`
  naming the Key ID. Keys are never written anywhere; `crypt::clear_keys` overwrites them.

- **OP1b–OP3c** (ST 391, 392, 393, 407, 408). A material track whose sequence holds several
  source clips plays them back to back as one `EssenceTrack`. Each clip contributes its own
  range: from its start position, for its duration. The clips may point into different file
  packages, each in its own essence container, and those packages may reuse track numbers,
  because elements are matched by track number and body SID. Before a long-GOP clip's first
  presented picture, the pictures it needs for decoding are kept as hidden samples: a `pts` no
  presented picture uses, and absent from `display_order`. Fillers are left out with a warning.
  Clips whose essence differs from the first clip's are also left out with a warning. That
  covers a different codec, edit rate, picture size or sound format. For OPxc (alternate
  packages), the first material package is read.

Not supported: gaps (fillers) inside edit lists (#1008), external essence (OP-Atom
material packages whose other tracks live in other files are opened track by track), partial
essence, clip-wrapped encrypted essence, the legacy MIC key
derivation of ST 429-6 Annex B (a MIC is checked only with an MDMK key), and descriptive metadata
(parsed sets are ignored).

## Writing a file

```rust
use cfa4_mxf::*;
let ids = PackageIds::from_seed("unique seed (path + time)", "Clip name"); // material + file UMIDs
let mut cfg = WriterConfig::new(Pattern::Op1a, Rational::new(25, 1), ids);
cfg.picture = Some(PictureDesc::new(PictureCoding::Vc3 { cid: 1272 }, 1920, 1080));
cfg.sound = vec![SoundDesc { sample_rate: 48_000, channels: 2, bits: 24 }];
cfg.timecode = Some(StartTimecode { frames: 90_000, rate: Rational::new(25, 1), drop_frame: false });
let mut w = MxfWriter::new(std::io::BufWriter::new(std::fs::File::create("out.mxf")?), cfg)?;
w.push_picture(&frame_bytes, FrameInfo::intra())?;   // stored order; AVC: key / P / B + display position
w.push_sound(0, &interleaved_le_pcm)?;              // any chunking: content packages are cut per edit unit
w.finish()?;
```

- **Streaming.** Any `Write + Seek` sink. `new` writes the header partition (open, incomplete,
  durations unknown) and a body partition; essence follows as it is pushed; `finish` writes the
  footer partition (closed, complete header metadata with the final durations and the index table
  segments), the random index pack, then rewrites the header partition pack and header metadata in
  place (the metadata's size does not depend on the durations; a reserved fill item absorbs any
  difference) and the body partition pack's footer offset. Only the index entries (11 bytes per
  edit unit, plus 4 per sound slice) are kept in memory.
- **OP1a** (ST 378): one content package per edit unit, frame wrapping (ST 379-1): the picture
  element then one element per sound track. Sound is buffered and cut into edit units exactly
  (`floor((n+1)·rate·den/num) − floor(n·rate·den/num)` samples: 1601/1602 at 29.97). Pictures
  without their sound yet wait; `finish` pads the last units' sound with silence. A multiple
  descriptor with one sub-descriptor per track (linked track ids), the "multiple wrappings"
  container label.
- **OP-Atom** (ST 390, Avid style): exactly one essence track per file, clip-wrapped (one element
  with a 9-byte BER length patched by `finish`). Several atom files of one clip can share the
  material package UMID with distinct track ids (`WriterConfig::first_track_id`). Sound files may
  use the sample rate as the edit rate (a constant-bytes index) or the frame rate (VBR entries).
- **D-Cinema track files** (`Pattern::DCinema`, ST 429-3 / 429-4, used by the DCP export): one
  essence track, frame-wrapped, OP-Atom label plus the multiple-mappings container label (as the
  reference implementation writes; FFmpeg then reads the frames), the preface's primary package,
  closed complete header metadata in the header partition only (the footer carries the index),
  the file package UMID from the asset UUID (`Umid::dcinema`). Pictures: JPEG 2000
  (`PictureCoding::J2k`, ST 422 container `02 0C 01`, the DCI 2K / 4K coding labels from Rsiz) in an
  RGBA descriptor (12-bit X'Y'Z', `PictureDesc::dcinema`, ST 428-1 transfer label) with a JPEG 2000
  sub-descriptor (`J2kSubDescriptor`: SIZ and component sizing). Sound: Broadcast Wave with the
  ST 429-2 channel assignment label (`WriterConfig::channel_assignment`). Immersive audio
  (`WriterConfig::iab`, `MxfWriter::push_data`, ST 429-18, #343): one IABitstream frame per data
  element under the class 14 key `0E 09 06 01`, a data track, the IAB data essence descriptor
  (`0E 09 06 03`, data essence coding `0E 09 06 04`, the version 5 primer label of clause 10) and
  its sub-descriptor (version, most channels and objects, first frame, IAB sample rate; laid out as
  the descriptor values of a real Atmos DCP track file). The reader opens such files as an IAB
  sound track. Multichannel audio labels (`WriterConfig::mca`, `McaLabel`, ST 377-4, #348):
  soundfield group (`6Ch`) and audio channel (`6Bh`) label sub-descriptors referenced from the
  Wave descriptor, their items under dynamic tags named in the primer; cinecert `asdcp-info -H`
  reads them back.
- **IMF track files** (`Pattern::Imf`, ST 2067-5 / 2067-2 / 2067-21, #931): one essence track,
  OP1a with KAG 1, the preface's primary package, at least 8 KiB of fill after the header
  metadata, closed complete partitions and a RIP, no header metadata or index in the footer.
  Pictures: JPEG 2000 frame-wrapped in mode P1 (the ST 422 Table 3 IMF coding label from Rsiz,
  `cfa4_j2k::ImfProfile`), a new essence partition every 60 s, each essence partition followed
  by an index partition holding its index segment; an RGBA descriptor (`PictureDesc::rgb`, narrow
  or `full_range` component refs) or a CDCI descriptor with the ST 2067-21 levels, and a JPEG 2000
  sub-descriptor taken from a codestream (`J2kSubDescriptor::from_codestream`: SIZ, COD, QCD,
  J2CLayout). Sound: 24-bit PCM at 48 or 96 kHz, clip-wrapped at the sample rate, the IMF MCA
  channel assignment label with `WriterConfig::mca` labels (the soundfield group with its MCA
  title, title version, audio content and element kind, `McaTitles`), `reference_image_edit_rate` and
  `reference_audio_alignment_level`, a constant-rate index partition after the essence. The IMP
  package (CPL, PKL, ASSETMAP) is written by the IMF export (`crates/export/src/imf_out.rs`, #1011).
- **RegXML essence descriptors** (`regxml::essence_descriptor`, ST 2001-1, #1011): the top-level
  file package's descriptor and its sub-descriptors (JPEG 2000, MCA labels) as the RegXML
  fragment a ST 2067-3 composition playlist carries in its `EssenceDescriptorList`, read from the
  header partition: groups, elements and types in the `r0` / `r1` / `r2` register namespaces,
  integers in decimal, enumerations by name, rationals as `n/d`, labels as `urn:smpte:ul:` and
  UUIDs as `urn:uuid:`, arrays and strong references nested in place. Checked item by item against
  the descriptors of the MERIDIAN test IMP (Netflix Photon, Apache-2.0); element names follow the
  SMPTE metadata registers as regxmllib's test dictionaries (BSD-2-Clause) list them.
- **Index tables** (ST 377-1 §11): VBR entries per edit unit — stream offset (from the first
  essence byte; clip-wrapped: from the element value), key-frame offset, flags (`0x80` random
  access, `0xC0` AVC IDR with its parameter sets, `0x22` P, `0x33` B) and temporal offsets from the
  display positions (`TO[d] = stored(d) − d`); a delta entry and a slice per sound element. Split
  into segments of at most ~60 KB (the entry array is a 2-byte-length local set item).
- **Metadata.** Preface, identification (company / product / version / modification date),
  content storage, essence container data (BodySID 1, IndexSID 2), material and file packages
  (`PackageIds`: UMIDs and names) with a timecode track (start, rounded base, drop frame) and one
  timeline track per essence (source clip → file package track; track numbers = element keys),
  CDCI picture descriptors (size, aspect ratio, depth, subsampling, black / white / range, BT.709 /
  BT.2020 PQ / HLG colour labels, coding label) and wave audio descriptors.
- **Mappings.** VC-3 (ST 2019-4: coding label `…71 cc` from the compression id, container `02 11`,
  element types 0x0C / 0x0D), ProRes (RDD 44: coding label profile byte, container `02 1C`),
  AVC byte stream (ST 381-3: container `02 10 60`, coding label from `profile_idc`; Annex B access
  units with in-band SPS / PPS — the AVC sub-descriptor is not written), MPEG-2 video
  elementary stream (ST 381-1: container `02 04 60`, element types 0x05 / 0x06, the MPEG video
  descriptor with SingleSequence, ConstantBframes, CodedContentType, LowDelay, ClosedGOP,
  IdenticalGOP, MaxGOP, BPictureCount, ProfileAndLevel and BitRate under dynamic tags, the RDD 9
  Table 6 coding labels for MP@HL / MP@H-14 / 422P@HL long GOP; random access entries flagged
  with their sequence header, P and B entries with the naive prediction flags of ST 381-1 A.2;
  without `rdd9` no SDTI-CP system item or AES3 elements; with `WriterConfig::rdd9` the strict
  RDD 9 layout: per content package the ST 385 system item (bitmap 5Ch, the time code as the user
  stamp), the picture element (number 0) and one frame-wrapped AES3 element per channel (ST 382
  container `02 06 03 00`, 4 or 8 elements, the unused ones silent, every element padded to the
  size of the largest edit unit's samples), each item on the 512-byte KAG; a new body partition at
  the first key frame five seconds into the previous one, carrying the previous partition's index
  table segment (one slice for the sound, delta entries system / picture (reordered) / channels),
  the footer indexing the last; an AES3 audio descriptor per mono channel track), type D-10 / IMX
  (`PictureCoding::D10`, ST 386: the defined-template container `01 01 rr 01` and the RP 224
  422P@ML I-frame coding label for 30 / 40 / 50 Mbit/s at 625 / 525 lines; one SDTI-CP
  compatible content package per frame: the ST 385 system item with the time code of the frame,
  the picture element padded to the SMPTE 356 frame size, one 8-channel AES3 element of ST 331
  (1920 samples, or 1601 / 1602 with the 5-sequence count), each item filled to the 512-byte KAG;
  a constant-size index with three delta entries; CDCI descriptor with separate fields, the
  vertical interval and BT.601 labels, generic sound descriptor of 4 or 8 channels), Broadcast
  Wave PCM (ST 382: container `02 06 01/02`, element types 0x01 / 0x02).
- `write_opatom_pcm(&OpAtomPcm, &[i32])` builds a complete in-memory OP-Atom PCM file (AAF media
  consolidation); `encode_pcm` interleaves planar f32 as little-endian PCM.

## Tests

- `src/tests.rs`: a small OP1a writer builds synthetic files (temporal offsets, key flags, PCM,
  timecode); open, presentation order, sample bytes, PCM values; truncation at every 37 bytes and
  600 random mutations never panic and only list fully-present samples. An OP3a writer with two
  file packages that share track numbers checks several things:
  - edit lists across packages: picture bytes, PCM values, and reads that cross a clip boundary;
  - fillers;
  - hidden long-GOP references;
  - hostile start positions and durations.
- `crates/codecs/tests/mxf_oracle.rs` (FFmpeg-written fixtures, FFmpeg as the decode oracle):
  H.264 long-GOP with B pictures (bit-exact every frame + 25 random seeks), H.264 29.97 DF
  timecode, DNxHR LB (±2), ProRes 422 (±1), MPEG-2 long GOP, XDCAM HD422 and D-10 (±4),
  OP-Atom VC-3 (clip-wrapped) and PCM, D-10 AES3 audio; PCM sample-exact in every file;
  truncated and corrupted files. Uncompressed: our writer packs FFmpeg's `testsrc2` frames in
  five layouts (OP1a and OP-Atom); our decode is bit-exact, and FFmpeg's decode of the UYVY, YUY2
  and v210 files is bit-exact too.
- `src/uncompressed/tests.rs`: every labelled layout packed and unpacked at three sizes, the bit
  order of each table entry by hand, descriptor fallbacks (CDCI, RGBA with fill and video-range
  codes, X'Y'Z', split components, HALF / f32 / f64 floats, palettes), field weaving with start / end fill, display rectangles, hostile sizes. `cargo xtask fixtures codecs` pre-generates them.
- `src/write/tests.rs`: writer output read back by `open`: OP1a with two sound tracks pushed in
  uneven chunks (durations, codec label, aspect, PCM exact, index slices, closed partitions),
  29.97 sound per edit unit (1601/1602), a long-GOP AVC index (presentation order, key and B
  flags), OP-Atom picture (clip wrapping) and PCM files at both edit rates, multi-segment index
  of 12 000 units, UMIDs; D-10 625 and 525 content packages (KAG alignment, system item bytes,
  padded pictures, AES3 headers and the 5-sequence count, PCM exact) and their refused
  configurations; RDD 9 at 25 Hz over 13 s (partitions every 126 units with the previous index
  segment, partition links and footer offsets, system item bytes, delta entries, per-channel
  element keys and PCM, presentation order) and 59.94 Hz (800 / 801-sample elements of one size,
  a silent second channel) and its refused configurations; IMF picture track files over 130 s
  (three essence partitions with their index partitions, partition links, 8 KiB fill, IMF coding
  label, sub-descriptor COD and layout), CDCI levels, IMF sound (clip wrapping, PCM exact, CBR
  index partition, MCA channel assignment, reference tags), refused IMF configurations and hostile
  codestream headers; with `CFA4_IMF_SAMPLE` set to a real IMF image or audio track file (the
  MERIDIAN test IMP of Netflix Photon, Apache-2.0) the file opens cleanly and has the same
  partition layout; truncation and random mutation of written files never panic.
- `crates/export/src/mxf_tests.rs` (ffprobe / ffmpeg as external oracles): OP1a exports with
  DNxHR, ProRes and H.264 — ffprobe reports the OP1a label, codec, 24 frames, frame rate, duration
  and `01:00:00:00` timecode; ffmpeg decodes the MXF picture exactly as it decodes the same
  essence exported as MOV, and its PCM equals our demuxer's sample for sample; OP-Atom exports —
  picture file and one mono file per channel, each channel equal to the WAV export's.
